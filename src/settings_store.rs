//! Persistent user settings: `%LOCALAPPDATA%\Oeca\settings.ini`, kept across
//! Oeca restarts, sign-outs and Windows restarts (unlike the session-scoped
//! clipboard history, which lives beside it and is never touched from here).
//!
//! Format: small UTF-8 text, one `key=value` per line, with a `version` line.
//! Each field is read on its own: a missing, unknown or invalid line only
//! leaves that one setting at its default, so a damaged or partial file never
//! resets the others. A file from a newer Oeca (higher version), one that is
//! not text or cannot be read, or one over `MAX_FILE_BYTES` is ignored
//! entirely (defaults) and not overwritten until the user changes a setting:
//! startup goes through `SettingsStore::initialize`, which leaves it alone.
//! Writes go to a temporary file renamed into place. No clipboard contents
//! are ever stored here.

use std::path::PathBuf;

use crate::config::{AccentChoice, ClipboardCapacity, CollapseDelay};
use crate::space::OecaSettings;

const FILE_NAME: &str = "settings.ini";
const TEMP_NAME: &str = "settings.ini.tmp";
const VERSION: u32 = 1;
/// Far above any valid file (a dozen short lines); larger files are not read.
const MAX_FILE_BYTES: u64 = 16 * 1024;

/// The settings as file text.
pub fn encode(s: &OecaSettings) -> String {
    let b = |v: bool| if v { "true" } else { "false" };
    format!(
        "# Oeca settings (edited by Oeca; one key=value per line)\n\
         version={VERSION}\n\
         always_on_top={}\n\
         reduced_motion={}\n\
         close_settings_on_escape={}\n\
         confirm_clear_clipboard={}\n\
         collapse_delay_seconds={}\n\
         accent={}\n\
         clipboard_capacity={}\n\
         show_source_app={}\n\
         show_visualizer={}\n\
         start_on_launch={}\n\
         focus_seconds={}\n\
         break_seconds={}\n\
         check_for_updates={}\n",
        b(s.always_on_top),
        b(s.reduced_motion),
        b(s.close_settings_on_escape),
        b(s.confirm_clear_clipboard),
        s.collapse_delay.ms() / 1000,
        accent_key(s.accent),
        s.clipboard_capacity.entries(),
        b(s.show_source_app),
        b(s.show_visualizer),
        b(s.start_on_launch),
        s.focus_secs,
        s.break_secs,
        b(s.check_for_updates),
    )
}

/// Why a file was ignored (defaults used).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadError {
    Missing,
    TooLarge,
    /// Not UTF-8 text, or no readable `version` line.
    Unreadable,
    /// Written by a newer Oeca.
    FutureVersion(u32),
}

/// Settings from file text: each recognised, valid line over the defaults.
pub fn decode(text: &str) -> Result<OecaSettings, LoadError> {
    let mut version = None;
    let mut s = OecaSettings::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        let flag = |v: &str| match v {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        };
        match key {
            "version" => version = value.parse::<u32>().ok(),
            "always_on_top" => set(&mut s.always_on_top, flag(value)),
            "reduced_motion" => set(&mut s.reduced_motion, flag(value)),
            "close_settings_on_escape" => set(&mut s.close_settings_on_escape, flag(value)),
            "confirm_clear_clipboard" => set(&mut s.confirm_clear_clipboard, flag(value)),
            "collapse_delay_seconds" => set(
                &mut s.collapse_delay,
                CollapseDelay::ALL
                    .into_iter()
                    .find(|d| value.parse() == Ok(d.ms() / 1000)),
            ),
            "accent" => set(
                &mut s.accent,
                AccentChoice::ALL
                    .into_iter()
                    .find(|a| accent_key(*a) == value),
            ),
            "clipboard_capacity" => set(
                &mut s.clipboard_capacity,
                ClipboardCapacity::ALL
                    .into_iter()
                    .find(|c| value.parse() == Ok(c.entries())),
            ),
            "show_source_app" => set(&mut s.show_source_app, flag(value)),
            "show_visualizer" => set(&mut s.show_visualizer, flag(value)),
            "start_on_launch" => set(&mut s.start_on_launch, flag(value)),
            "focus_seconds" => set(&mut s.focus_secs, timer_secs(value)),
            "break_seconds" => set(&mut s.break_secs, timer_secs(value)),
            "check_for_updates" => set(&mut s.check_for_updates, flag(value)),
            _ => {}
        }
    }
    match version {
        Some(VERSION) => Ok(s),
        Some(v) if v > VERSION => Err(LoadError::FutureVersion(v)),
        _ => Err(LoadError::Unreadable),
    }
}

/// A timer duration (seconds): one of the timer scale's stops.
fn timer_secs(v: &str) -> Option<u32> {
    v.parse::<u32>()
        .ok()
        .filter(|s| crate::timer::stop_of(*s).is_some())
}

/// A valid value replaces the default; anything else keeps it.
fn set<T>(field: &mut T, value: Option<T>) {
    if let Some(v) = value {
        *field = v;
    }
}

/// Stable file names for the accents (independent of the displayed names).
fn accent_key(a: AccentChoice) -> &'static str {
    match a {
        AccentChoice::White => "white",
        AccentChoice::Blue => "blue",
        AccentChoice::Teal => "teal",
        AccentChoice::Green => "green",
        AccentChoice::Amber => "amber",
        AccentChoice::Rose => "rose",
        AccentChoice::Violet => "violet",
    }
}

/// The settings file in a folder; remembers what was last written so an
/// unchanged value is not rewritten, and whether an existing file could not
/// be used (then startup leaves it alone; see `initialize`).
pub struct SettingsStore {
    dir: Option<PathBuf>,
    saved: Option<OecaSettings>,
    keep_existing: bool,
}

impl SettingsStore {
    /// `%LOCALAPPDATA%\Oeca` (None if Windows will not say: nothing is read
    /// or written, defaults apply).
    pub fn open() -> Self {
        Self::at(crate::clipboard_store::local_app_data().map(|d| d.join("Oeca")))
    }

    pub fn at(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            saved: None,
            keep_existing: false,
        }
    }

    /// The saved settings, or why the defaults apply. A file that exists but
    /// cannot be used (newer version, corrupt, oversized, unreadable) is
    /// marked to be kept: only an explicit change (`save`) replaces it.
    pub fn load(&mut self) -> Result<OecaSettings, LoadError> {
        let result = self.read();
        self.keep_existing = matches!(result, Err(e) if e != LoadError::Missing);
        if let Ok(s) = result {
            self.saved = Some(s);
        }
        result
    }

    fn read(&self) -> Result<OecaSettings, LoadError> {
        let path = self.dir.as_ref().ok_or(LoadError::Missing)?.join(FILE_NAME);
        let len = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(LoadError::Missing),
            // There, but not accessible: not "missing" (never write over it)
            Err(_) => return Err(LoadError::Unreadable),
        };
        if len > MAX_FILE_BYTES {
            return Err(LoadError::TooLarge);
        }
        let bytes = std::fs::read(&path).map_err(|_| LoadError::Unreadable)?;
        let text = String::from_utf8(bytes).map_err(|_| LoadError::Unreadable)?;
        decode(&text)
    }

    /// Startup: records the settings as applied (the first launch writes
    /// the defaults; later launches write only if something changed, e.g.
    /// Start on launch outside Oeca). Never replaces an existing file that
    /// `load` could not use, such as one from a newer Oeca: that waits for
    /// the user to change a setting.
    pub fn initialize(&mut self, s: &OecaSettings) -> std::io::Result<()> {
        if self.keep_existing {
            return Ok(());
        }
        self.save(s)
    }

    /// Writes `s` if it differs from what was last read or written: to a
    /// temporary file, flushed, then renamed over the old one. An error
    /// leaves the previous file in place. This is for explicit changes, so
    /// it also replaces a file `load` could not use.
    pub fn save(&mut self, s: &OecaSettings) -> std::io::Result<()> {
        if self.saved.as_ref() == Some(s) {
            return Ok(());
        }
        let Some(dir) = &self.dir else {
            return Err(std::io::ErrorKind::NotFound.into());
        };
        std::fs::create_dir_all(dir)?;
        let temp = dir.join(TEMP_NAME);
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&temp)?;
            f.write_all(encode(s).as_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&temp, dir.join(FILE_NAME))?;
        self.saved = Some(*s);
        self.keep_existing = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom() -> OecaSettings {
        OecaSettings {
            always_on_top: false,
            reduced_motion: true,
            close_settings_on_escape: false,
            confirm_clear_clipboard: false,
            accent: AccentChoice::Violet,
            clipboard_capacity: ClipboardCapacity::Five,
            show_source_app: false,
            show_visualizer: false,
            collapse_delay: CollapseDelay::Fifteen,
            start_on_launch: true,
            focus_secs: 50 * 60,
            break_secs: 30,
            check_for_updates: true,
        }
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir()
                .join(format!("oeca-settings-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_closing_delay_zero_default_and_saved_values_kept() {
        let file = |delay: &str| format!("version = {VERSION}\ncollapse_delay_seconds = {delay}\n");
        // New: 0 s, and it round-trips
        assert_eq!(OecaSettings::default().collapse_delay, CollapseDelay::Zero);
        assert_eq!(
            decode(&file("0")).unwrap().collapse_delay,
            CollapseDelay::Zero
        );
        let zero = OecaSettings {
            collapse_delay: CollapseDelay::Zero,
            ..OecaSettings::default()
        };
        assert_eq!(decode(&encode(&zero)).unwrap(), zero);
        // A user's saved choice is kept as it is (no migration resets it)
        for (text, d) in [
            ("5", CollapseDelay::Five),
            ("10", CollapseDelay::Ten),
            ("15", CollapseDelay::Fifteen),
        ] {
            assert_eq!(decode(&file(text)).unwrap().collapse_delay, d);
        }
        // Missing or invalid: the default, with the rest of the file intact
        let other = format!("version = {VERSION}\nalways_on_top = false\n");
        let s = decode(&other).unwrap();
        assert_eq!(
            (s.collapse_delay, s.always_on_top),
            (CollapseDelay::Zero, false)
        );
        for bad in ["7", "-1", "0.5", "", "zero"] {
            assert_eq!(
                decode(&file(bad)).unwrap().collapse_delay,
                CollapseDelay::Zero,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn test_new_user_defaults() {
        let d = OecaSettings::default();
        assert!(d.always_on_top && !d.reduced_motion);
        assert!(d.close_settings_on_escape && d.confirm_clear_clipboard);
        assert_eq!(d.collapse_delay, CollapseDelay::Zero);
        assert_eq!(d.accent, AccentChoice::White);
        assert_eq!(d.clipboard_capacity, ClipboardCapacity::Twenty);
        assert!(d.show_source_app && d.show_visualizer);
        assert!(!d.start_on_launch, "Start on launch is off for a new user");
        // No file: defaults
        let dir = TempDir::new("defaults");
        let mut store = SettingsStore::at(Some(dir.0.clone()));
        assert_eq!(store.load(), Err(LoadError::Missing));
    }

    #[test]
    fn test_every_setting_round_trips() {
        for s in [OecaSettings::default(), custom()] {
            assert_eq!(decode(&encode(&s)), Ok(s));
        }
        // Every value of every enum survives
        for a in AccentChoice::ALL {
            let s = OecaSettings {
                accent: a,
                ..custom()
            };
            assert_eq!(decode(&encode(&s)), Ok(s), "{a:?}");
        }
        for c in ClipboardCapacity::ALL {
            let s = OecaSettings {
                clipboard_capacity: c,
                ..custom()
            };
            assert_eq!(decode(&encode(&s)), Ok(s), "{c:?}");
        }
        for d in CollapseDelay::ALL {
            let s = OecaSettings {
                collapse_delay: d,
                ..custom()
            };
            assert_eq!(decode(&encode(&s)), Ok(s), "{d:?}");
        }
        // Every boolean field is independent
        let text = encode(&OecaSettings::default());
        let flags = text
            .lines()
            .filter(|l| l.ends_with("=true") || l.ends_with("=false"));
        assert_eq!(flags.count(), 8);
        assert!(!text.contains("clipboard-history"), "no clipboard data");
    }

    #[test]
    fn test_save_load_and_restart() {
        let dir = TempDir::new("restart");
        let mut store = SettingsStore::at(Some(dir.0.clone()));
        store.save(&custom()).unwrap();
        // A new process (relaunch, sign-in, restart) reads the same values
        let mut next = SettingsStore::at(Some(dir.0.clone()));
        assert_eq!(next.load(), Ok(custom()));
        // Unchanged settings are not rewritten; a change is
        let path = dir.0.join(FILE_NAME);
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        next.save(&custom()).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before
        );
        let changed = OecaSettings {
            accent: AccentChoice::Teal,
            ..custom()
        };
        next.save(&changed).unwrap();
        assert_eq!(SettingsStore::at(Some(dir.0.clone())).load(), Ok(changed));
        assert!(
            !dir.0.join(TEMP_NAME).exists(),
            "temporary file renamed away"
        );
        // The clipboard history file beside it is unrelated: removing it
        // leaves the settings
        std::fs::write(dir.0.join("clipboard-history.bin"), b"x").unwrap();
        let _ = std::fs::remove_file(dir.0.join("clipboard-history.bin"));
        assert_eq!(SettingsStore::at(Some(dir.0.clone())).load(), Ok(changed));
    }

    #[test]
    fn test_partial_and_invalid_values_keep_the_rest() {
        // Only some lines: the others stay at their defaults
        let s = decode("version=1\naccent=rose\nreduced_motion=true\n").unwrap();
        assert_eq!(
            s,
            OecaSettings {
                accent: AccentChoice::Rose,
                reduced_motion: true,
                ..OecaSettings::default()
            }
        );
        // Invalid values (out-of-range delay / capacity, unknown accent,
        // non-boolean) fall back per field; valid neighbours are kept
        let text = "version=1\n\
                    always_on_top=yes\n\
                    collapse_delay_seconds=7\n\
                    clipboard_capacity=100\n\
                    accent=chartreuse\n\
                    show_visualizer=false\n\
                    show_source_app=TRUE\n\
                    start_on_launch=1\n\
                    focus_seconds=0\n\
                    break_seconds=20\n\
                    unknown_future_key=whatever\n\
                    no equals sign here\n\
                    =\n";
        let s = decode(text).unwrap();
        assert_eq!(
            s,
            OecaSettings {
                show_visualizer: false,
                ..OecaSettings::default()
            }
        );
        // Whitespace, CRLF line endings and comments are fine
        let s = decode("# note\r\n version = 1 \r\n accent = blue \r\n").unwrap();
        assert_eq!(s.accent, AccentChoice::Blue);
        // Timer durations at the ends of their range are kept
        let s = decode("version=1\nfocus_seconds=7200\nbreak_seconds=30\n").unwrap();
        assert_eq!((s.focus_secs, s.break_secs), (7200, 30));
    }

    #[test]
    fn test_unreadable_and_future_files_are_ignored() {
        assert_eq!(decode(""), Err(LoadError::Unreadable));
        assert_eq!(
            decode("accent=rose\n"),
            Err(LoadError::Unreadable),
            "no version"
        );
        assert_eq!(decode("version=banana\n"), Err(LoadError::Unreadable));
        assert_eq!(decode("version=0\n"), Err(LoadError::Unreadable));
        assert_eq!(
            decode("version=2\naccent=rose\n"),
            Err(LoadError::FutureVersion(2))
        );
        // Truncated mid-file (partial write of an older format): the lines
        // that made it are used
        let full = encode(&custom());
        let cut = &full[..full.find("clipboard_capacity").unwrap()];
        let s = decode(cut).unwrap();
        assert_eq!(s.accent, AccentChoice::Violet);
        assert_eq!(s.clipboard_capacity, ClipboardCapacity::Twenty, "default");
        let dir = TempDir::new("corrupt");
        let path = dir.0.join(FILE_NAME);
        // Binary garbage, and an oversized file: defaults, no panic
        std::fs::write(&path, [0xFF, 0xFE, 0x00, 0x9C, 0x01]).unwrap();
        assert_eq!(
            SettingsStore::at(Some(dir.0.clone())).load(),
            Err(LoadError::Unreadable)
        );
        std::fs::write(&path, vec![b'#'; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert_eq!(
            SettingsStore::at(Some(dir.0.clone())).load(),
            Err(LoadError::TooLarge)
        );
    }

    #[test]
    fn test_failed_writes_report_an_error_and_keep_the_old_file() {
        let dir = TempDir::new("failure");
        let mut store = SettingsStore::at(Some(dir.0.clone()));
        store.save(&custom()).unwrap();
        // The target path is a folder: the rename fails, the error is
        // returned, and the earlier settings remain readable
        let blocked = TempDir::new("failure-blocked");
        std::fs::create_dir_all(blocked.0.join(FILE_NAME)).unwrap();
        let mut bad = SettingsStore::at(Some(blocked.0.clone()));
        assert!(bad.save(&custom()).is_err());
        assert_eq!(SettingsStore::at(Some(dir.0.clone())).load(), Ok(custom()));
        // A failed save is retried on the next change (not remembered as saved)
        assert!(bad.saved.is_none());
        // No folder known: an error, never a silent "saved"
        assert!(SettingsStore::at(None).save(&custom()).is_err());
    }

    /// Oeca's startup, exactly as `window::run_window` does it: load (or the
    /// defaults), reconcile, then record the settings as applied.
    fn startup(dir: &TempDir) -> (SettingsStore, OecaSettings) {
        let mut store = SettingsStore::at(Some(dir.0.clone()));
        let settings = store.load().unwrap_or_default();
        store.initialize(&settings).unwrap();
        (store, settings)
    }

    #[test]
    fn test_startup_initializes_a_missing_file_with_defaults() {
        let dir = TempDir::new("startup-missing");
        let path = dir.0.join(FILE_NAME);
        let (_, s) = startup(&dir);
        assert_eq!(s, OecaSettings::default());
        assert!(path.exists(), "first launch writes the defaults");
        assert_eq!(SettingsStore::at(Some(dir.0.clone())).load(), Ok(s));
        // The next launch reads them back and does not rewrite the file
        let before = std::fs::read(&path).unwrap();
        let (_, again) = startup(&dir);
        assert_eq!(again, s);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn test_startup_keeps_a_valid_file_and_records_reconciled_values() {
        let dir = TempDir::new("startup-valid");
        let path = dir.0.join(FILE_NAME);
        std::fs::write(&path, encode(&custom())).unwrap();
        let (_, s) = startup(&dir);
        assert_eq!(s, custom(), "every saved value applied");
        // Start on launch found removed outside Oeca: the corrected value is
        // recorded at startup (normal persistence)
        let mut store = SettingsStore::at(Some(dir.0.clone()));
        let mut s = store.load().unwrap();
        s.start_on_launch = false;
        store.initialize(&s).unwrap();
        assert_eq!(SettingsStore::at(Some(dir.0.clone())).load(), Ok(s));
    }

    #[test]
    fn test_startup_never_overwrites_an_unusable_file() {
        let newer = b"version=2\naccent=rose\nsome_new_setting=on\n".to_vec();
        let corrupt = vec![0xFF, 0xFE, 0x00, 0x9C, 0x01, 0x80];
        let unversioned = b"accent=rose\nreduced_motion=true\n".to_vec();
        let oversized = vec![b'#'; MAX_FILE_BYTES as usize + 1];
        for (name, bytes) in [
            ("newer", newer),
            ("corrupt", corrupt),
            ("unversioned", unversioned),
            ("oversized", oversized),
        ] {
            let dir = TempDir::new(&format!("startup-{name}"));
            let path = dir.0.join(FILE_NAME);
            std::fs::write(&path, &bytes).unwrap();
            // Oeca runs on the defaults, and the file is left exactly as it was
            let (mut store, s) = startup(&dir);
            assert_eq!(s, OecaSettings::default(), "{name}");
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "{name}: untouched");
            // Starting again changes nothing either
            startup(&dir);
            assert_eq!(
                std::fs::read(&path).unwrap(),
                bytes,
                "{name}: still untouched"
            );
            // An explicit change by the user replaces it
            let changed = OecaSettings {
                accent: AccentChoice::Teal,
                ..s
            };
            store.save(&changed).unwrap();
            assert_eq!(
                SettingsStore::at(Some(dir.0.clone())).load(),
                Ok(changed),
                "{name}: the user's change is saved"
            );
            // ...after which startup records values normally again
            assert!(!store.keep_existing);
        }
    }
}
