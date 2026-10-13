//! Session-scoped clipboard history persistence: the history survives Oeca
//! quitting (tray Quit, window close, a crash) and relaunching within the
//! same Windows logon session, and never reaches a later one.
//!
//! Session identity: a volatile registry key (`HKCU\Software\Oeca\Session`,
//! `REG_OPTION_VOLATILE`) holds this session's id: the logon session's LUID
//! plus the moment the key was made. Volatile keys live only in memory and
//! disappear when the user's hive unloads (sign-out) or Windows shuts down /
//! restarts, so a new session finds no key and starts a new id. The LUID check
//! covers a hive that stays loaded across a sign-out (LUIDs are unique within
//! a boot).
//!
//! Storage: one file, `%LOCALAPPDATA%\Oeca\clipboard-history.bin` (per-user,
//! outside the install folder; not encrypted), tagged with the session id and
//! a checksum. It is loaded only when its id matches the current session, so
//! a stale file (power loss, crash) can never be shown in a new session; a new
//! session also deletes it. Writes go to a temporary file renamed into place,
//! so a partial write leaves the previous file (or nothing) behind.
//!
//! Privacy: contents are never logged; failures only disable persistence.

use std::path::{Path, PathBuf};

use crate::clipboard::{ClipboardHistory, ClipboardItem};
use crate::config::{CLIPBOARD_MAX_ENTRIES, CLIPBOARD_MAX_IMAGE_BYTES, CLIPBOARD_MAX_TEXT_UNITS};
use crate::media::Artwork;

const FILE_NAME: &str = "clipboard-history.bin";
const TEMP_NAME: &str = "clipboard-history.tmp";
const MAGIC: &[u8; 8] = b"OECACLP1";
const TAG_TEXT: u8 = 1;
const TAG_IMAGE: u8 = 2;
/// Upper bound on a valid file: every entry at the text limit (UTF-8, at most
/// 3 bytes per UTF-16 unit) plus the whole image budget, plus framing. Larger
/// files are rejected before reading.
const MAX_FILE_BYTES: u64 = (8 + 16 + 4 + 8) as u64
    + (CLIPBOARD_MAX_ENTRIES * (1 + 8 + CLIPBOARD_MAX_TEXT_UNITS * 3)) as u64
    + CLIPBOARD_MAX_IMAGE_BYTES as u64;

/// One Windows logon session: its LUID and when Oeca first saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionId {
    pub logon: u64,
    pub stamp: u64,
}

impl SessionId {
    fn to_bytes(self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&self.logon.to_le_bytes());
        b[8..].copy_from_slice(&self.stamp.to_le_bytes());
        b
    }

    fn from_bytes(b: &[u8]) -> Option<Self> {
        let b: &[u8; 16] = b.try_into().ok()?;
        Some(Self {
            logon: u64::from_le_bytes(b[..8].try_into().ok()?),
            stamp: u64::from_le_bytes(b[8..].try_into().ok()?),
        })
    }
}

/// Whether the session is the one Oeca last ran in (pure).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    /// Same logon session: saved history may be restored.
    Resumed(SessionId),
    /// A new session: nothing earlier may be shown.
    Fresh(SessionId),
}

/// The session from the volatile key's id (`stored`, None if the key is new
/// or empty) and the current logon LUID; `now` stamps a fresh one.
pub fn decide(stored: Option<SessionId>, logon: u64, now: u64) -> Session {
    match stored {
        Some(id) if id.logon == logon => Session::Resumed(id),
        _ => Session::Fresh(SessionId { logon, stamp: now }),
    }
}

/// Why saved data was not used (never contains clipboard contents).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadError {
    Missing,
    TooLarge,
    Corrupt,
    /// Saved in another Windows session.
    OtherSession,
}

/// The history as file bytes, newest first, tagged with `session`.
pub fn encode(session: SessionId, history: &ClipboardHistory) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&session.to_bytes());
    out.extend_from_slice(&(history.len() as u32).to_le_bytes());
    for item in history.items() {
        match item {
            ClipboardItem::Text(t) => {
                out.push(TAG_TEXT);
                out.extend_from_slice(&(t.len() as u32).to_le_bytes());
                out.extend_from_slice(t.as_bytes());
            }
            ClipboardItem::Image(a) => {
                out.push(TAG_IMAGE);
                out.extend_from_slice(&a.width.to_le_bytes());
                out.extend_from_slice(&a.height.to_le_bytes());
                out.extend_from_slice(&a.pixels);
            }
        }
    }
    let sum = fnv1a(&out);
    out.extend_from_slice(&sum.to_le_bytes());
    out
}

/// Saved items (newest first) if `bytes` is a complete, intact file from
/// `session`. Every length is checked against the remaining bytes and the
/// clipboard limits before anything is allocated.
pub fn decode(bytes: &[u8], session: SessionId) -> Result<Vec<ClipboardItem>, LoadError> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(LoadError::TooLarge);
    }
    let (body, sum) = bytes.split_last_chunk::<8>().ok_or(LoadError::Corrupt)?;
    if !body.starts_with(MAGIC) || fnv1a(body) != u64::from_le_bytes(*sum) {
        return Err(LoadError::Corrupt);
    }
    let mut r = Reader(&body[MAGIC.len()..]);
    let saved = SessionId::from_bytes(r.take(16)?).ok_or(LoadError::Corrupt)?;
    if saved != session {
        return Err(LoadError::OtherSession);
    }
    let count = r.u32()? as usize;
    if count > CLIPBOARD_MAX_ENTRIES {
        return Err(LoadError::Corrupt);
    }
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        let item = match r.take(1)?[0] {
            TAG_TEXT => {
                let len = r.u32()? as usize;
                if len > CLIPBOARD_MAX_TEXT_UNITS * 3 {
                    return Err(LoadError::Corrupt);
                }
                let text = std::str::from_utf8(r.take(len)?).map_err(|_| LoadError::Corrupt)?;
                ClipboardItem::Text(text.to_owned())
            }
            TAG_IMAGE => {
                let (w, h) = (r.u32()?, r.u32()?);
                let len = (w as usize)
                    .checked_mul(h as usize)
                    .and_then(|n| n.checked_mul(4))
                    .filter(|&n| n <= CLIPBOARD_MAX_IMAGE_BYTES)
                    .ok_or(LoadError::Corrupt)?;
                let pixels = r.take(len)?.to_vec();
                ClipboardItem::Image(Artwork::new(w, h, pixels).ok_or(LoadError::Corrupt)?)
            }
            _ => return Err(LoadError::Corrupt),
        };
        items.push(item);
    }
    if !r.0.is_empty() {
        return Err(LoadError::Corrupt);
    }
    Ok(items)
}

/// Puts saved items (newest first) into `history` through its own `add`, so
/// the text and image limits, the capacity and the image budget all apply
/// exactly as for copied items; their order is kept.
pub fn restore(history: &mut ClipboardHistory, items: Vec<ClipboardItem>) {
    for item in items.into_iter().rev() {
        history.add(item);
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], LoadError> {
        if n > self.0.len() {
            return Err(LoadError::Corrupt);
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn u32(&mut self) -> Result<u32, LoadError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().unwrap_or([0; 4]),
        ))
    }
}

/// FNV-1a 64 (detects truncation and corruption; not a security measure).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The history file for one session in `dir`. `None` session: persistence
/// is off (the session could not be identified), so nothing is read or kept.
pub struct ClipboardStore {
    dir: PathBuf,
    session: Option<SessionId>,
}

impl ClipboardStore {
    /// This Windows session's store (registry + `%LOCALAPPDATA%\Oeca`). A
    /// fresh session removes any earlier session's file first.
    pub fn open() -> Self {
        let dir = local_app_data().map(|d| d.join("Oeca"));
        let session = session::current();
        match (dir, session) {
            (Some(dir), Some(s)) => Self::at(dir, s),
            (dir, _) => Self {
                dir: dir.unwrap_or_default(),
                session: None,
            },
        }
    }

    /// The store in `dir` for `session` (tests use a temporary `dir`).
    pub fn at(dir: PathBuf, session: Session) -> Self {
        let store = Self {
            dir,
            session: Some(match session {
                Session::Resumed(id) | Session::Fresh(id) => id,
            }),
        };
        if matches!(session, Session::Fresh(_)) {
            store.wipe();
        }
        store
    }

    /// The saved history of this session, if any and intact.
    pub fn load(&self) -> Result<Vec<ClipboardItem>, LoadError> {
        let session = self.session.ok_or(LoadError::Missing)?;
        let path = self.dir.join(FILE_NAME);
        let len = std::fs::metadata(&path)
            .map_err(|_| LoadError::Missing)?
            .len();
        if len > MAX_FILE_BYTES {
            return Err(LoadError::TooLarge);
        }
        let bytes = std::fs::read(&path).map_err(|_| LoadError::Missing)?;
        decode(&bytes, session)
    }

    /// Saves the history (an empty one removes the file). Written to a
    /// temporary file, flushed, then renamed over the old one.
    pub fn save(&self, history: &ClipboardHistory) -> std::io::Result<()> {
        let Some(session) = self.session else {
            return Ok(());
        };
        if history.is_empty() {
            self.wipe();
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir)?;
        let temp = self.dir.join(TEMP_NAME);
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&temp)?;
            f.write_all(&encode(session, history))?;
            f.sync_all()?;
        }
        std::fs::rename(&temp, self.dir.join(FILE_NAME))
    }

    /// Removes this store's own files (nothing else in the folder).
    pub fn wipe(&self) {
        wipe_dir(&self.dir);
    }
}

fn wipe_dir(dir: &Path) {
    if dir.as_os_str().is_empty() {
        return;
    }
    for name in [FILE_NAME, TEMP_NAME] {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

/// The Windows session is ending (shutdown, restart, sign-out): best effort,
/// remove the saved history now. Session isolation does not depend on it.
pub fn wipe_for_session_end() {
    if let Some(dir) = local_app_data() {
        wipe_dir(&dir.join("Oeca"));
    }
}

fn local_app_data() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None).ok()?;
        let path = p.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(p.0 as *const _));
        path
    }
}

/// The volatile registry key and the logon LUID.
mod session {
    use super::{Session, SessionId, decide};
    use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TOKEN_STATISTICS, TokenStatistics};
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_BINARY, REG_OPTION_VOLATILE,
        RegCloseKey, RegCreateKeyExW, RegQueryValueExW, RegSetValueExW,
    };
    use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows::core::w;

    /// This logon session, or None if Windows would not say (persistence
    /// then stays off rather than risk mixing sessions).
    pub fn current() -> Option<Session> {
        let logon = logon_luid()?;
        unsafe {
            let mut key = HKEY::default();
            if RegCreateKeyExW(
                HKEY_CURRENT_USER,
                w!("Software\\Oeca\\Session"),
                None,
                None,
                REG_OPTION_VOLATILE,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                None,
                &mut key,
                None,
            ) != ERROR_SUCCESS
            {
                return None;
            }
            let mut buf = [0u8; 16];
            let mut len = buf.len() as u32;
            let stored = (RegQueryValueExW(
                key,
                w!("Id"),
                None,
                None,
                Some(buf.as_mut_ptr()),
                Some(&mut len),
            ) == ERROR_SUCCESS
                && len == 16)
                .then(|| SessionId::from_bytes(&buf))
                .flatten();
            let ft = GetSystemTimeAsFileTime();
            let now = (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
            let session = decide(stored, logon, now);
            let ok = match session {
                Session::Resumed(_) => true,
                Session::Fresh(id) => {
                    RegSetValueExW(key, w!("Id"), None, REG_BINARY, Some(&id.to_bytes()))
                        == ERROR_SUCCESS
                }
            };
            let _ = RegCloseKey(key);
            ok.then_some(session)
        }
    }

    /// The logon session's LUID (`TOKEN_STATISTICS.AuthenticationId`), from
    /// this process's token (the `GetCurrentProcessToken` pseudo-handle).
    fn logon_luid() -> Option<u64> {
        let mut stats = TOKEN_STATISTICS::default();
        let mut len = 0u32;
        unsafe {
            GetTokenInformation(
                HANDLE(-4isize as *mut _),
                TokenStatistics,
                Some(&mut stats as *mut _ as *mut _),
                std::mem::size_of::<TOKEN_STATISTICS>() as u32,
                &mut len,
            )
            .ok()?;
        }
        let id = stats.AuthenticationId;
        Some((u64::from(id.HighPart as u32) << 32) | u64::from(id.LowPart))
    }

    #[cfg(test)]
    pub fn luid_for_tests() -> Option<u64> {
        logon_luid()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ClipboardCapacity;

    const S1: SessionId = SessionId {
        logon: 0x0000_0001_0003_e7a1,
        stamp: 133_000_000_000_000_000,
    };

    fn text(s: &str) -> ClipboardItem {
        ClipboardItem::Text(s.to_owned())
    }

    fn image(w: u32, h: u32, shade: u8) -> ClipboardItem {
        let px = (0..w * h)
            .flat_map(|i| [shade, (i % 251) as u8, 255 - shade, 255])
            .collect();
        ClipboardItem::Image(Artwork::new(w, h, px).unwrap())
    }

    fn history(items: &[ClipboardItem]) -> ClipboardHistory {
        let mut h = ClipboardHistory::default();
        for i in items.iter().rev() {
            h.add(i.clone());
        }
        h
    }

    fn items(h: &ClipboardHistory) -> Vec<ClipboardItem> {
        h.items().cloned().collect()
    }

    /// A private temporary folder, removed on drop.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let p =
                std::env::temp_dir().join(format!("oeca-store-test-{}-{name}", std::process::id()));
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
    fn test_text_and_images_round_trip_in_order() {
        let saved = history(&[
            text("newest"),
            image(3, 2, 10),
            text("multi\nline \u{1F600} \u{00E9}"),
            image(1, 1, 200),
            text("oldest"),
        ]);
        let back = decode(&encode(S1, &saved), S1).unwrap();
        assert_eq!(back, items(&saved), "same items, newest first");
        // Image bytes themselves come back, not just their size
        let ClipboardItem::Image(a) = &back[1] else {
            panic!()
        };
        let ClipboardItem::Image(b) = &items(&saved)[1] else {
            panic!()
        };
        assert_eq!(&*a.pixels, &*b.pixels);
        let mut restored = ClipboardHistory::default();
        restore(&mut restored, back);
        assert_eq!(items(&restored), items(&saved));
    }

    #[test]
    fn test_restore_keeps_capacity_and_limits() {
        let saved = history(&(0..20).map(|i| text(&format!("t{i}"))).collect::<Vec<_>>());
        let back = decode(&encode(S1, &saved), S1).unwrap();
        // Relaunched with capacity 5: the newest five, in order
        let mut h = ClipboardHistory::default();
        h.set_capacity(ClipboardCapacity::Five);
        restore(&mut h, back);
        assert_eq!(
            items(&h),
            (0..5).map(|i| text(&format!("t{i}"))).collect::<Vec<_>>()
        );
        // Over-limit entries in a (valid) file are refused by the history's
        // own limits: the image budget keeps the newest image only
        let big = (CLIPBOARD_MAX_IMAGE_BYTES / 4 / 1000) as u32;
        let saved = [image(1000, big, 1), image(1000, big, 2)];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&S1.to_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        for i in &saved {
            let ClipboardItem::Image(a) = i else {
                unreachable!()
            };
            bytes.push(TAG_IMAGE);
            bytes.extend_from_slice(&a.width.to_le_bytes());
            bytes.extend_from_slice(&a.height.to_le_bytes());
            bytes.extend_from_slice(&a.pixels);
        }
        let sum = fnv1a(&bytes);
        bytes.extend_from_slice(&sum.to_le_bytes());
        let mut h = ClipboardHistory::default();
        restore(&mut h, decode(&bytes, S1).unwrap());
        assert_eq!(items(&h), [saved[0].clone()]);
        assert!(h.image_bytes() <= CLIPBOARD_MAX_IMAGE_BYTES);
    }

    #[test]
    fn test_corrupt_incomplete_and_foreign_data_is_refused() {
        let good = encode(S1, &history(&[text("hello"), image(2, 2, 9)]));
        assert!(decode(&good, S1).is_ok());
        // Every truncation (a partial write) is refused, never a panic
        for len in 0..good.len() {
            assert!(decode(&good[..len], S1).is_err(), "truncated to {len}");
        }
        // Any flipped byte is caught
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 0x41;
            assert!(decode(&bad, S1).is_err(), "byte {i}");
        }
        // Trailing junk, wrong magic, random bytes
        let mut long = good.clone();
        long.insert(good.len() - 8, 0);
        assert!(decode(&long, S1).is_err());
        assert_eq!(
            decode(b"not an oeca file at all", S1),
            Err(LoadError::Corrupt)
        );
        // Valid checksum but lying lengths: refused before allocating
        let mut lie = Vec::new();
        lie.extend_from_slice(MAGIC);
        lie.extend_from_slice(&S1.to_bytes());
        lie.extend_from_slice(&1u32.to_le_bytes());
        lie.push(TAG_IMAGE);
        lie.extend_from_slice(&u32::MAX.to_le_bytes());
        lie.extend_from_slice(&u32::MAX.to_le_bytes());
        let sum = fnv1a(&lie);
        lie.extend_from_slice(&sum.to_le_bytes());
        assert_eq!(decode(&lie, S1), Err(LoadError::Corrupt));
        let mut many = lie[..MAGIC.len() + 16].to_vec();
        many.extend_from_slice(&(CLIPBOARD_MAX_ENTRIES as u32 + 1).to_le_bytes());
        let sum = fnv1a(&many);
        many.extend_from_slice(&sum.to_le_bytes());
        assert_eq!(
            decode(&many, S1),
            Err(LoadError::Corrupt),
            "too many entries"
        );
        // Another session's file, intact, is never decoded
        let other = SessionId {
            stamp: S1.stamp + 1,
            ..S1
        };
        assert_eq!(decode(&good, other), Err(LoadError::OtherSession));
    }

    #[test]
    fn test_session_decision() {
        // No key yet (first run, or after sign-out / restart): fresh
        let s = decide(None, 7, 100);
        assert_eq!(
            s,
            Session::Fresh(SessionId {
                logon: 7,
                stamp: 100
            })
        );
        // Same logon session (relaunch, crash): resumed with the same id
        let id = SessionId {
            logon: 7,
            stamp: 100,
        };
        assert_eq!(decide(Some(id), 7, 999), Session::Resumed(id));
        // A key left by another logon (hive stayed loaded): fresh, new stamp
        assert_eq!(
            decide(Some(id), 8, 999),
            Session::Fresh(SessionId {
                logon: 8,
                stamp: 999
            })
        );
    }

    #[test]
    fn test_close_and_reopen_in_the_same_session() {
        let dir = TempDir::new("reopen");
        let session = Session::Fresh(S1);
        let store = ClipboardStore::at(dir.0.clone(), session);
        assert_eq!(store.load(), Err(LoadError::Missing));
        let mut h = history(&[text("a"), image(4, 3, 50), text("b")]);
        store.save(&h).unwrap();
        // Quit and relaunch: same session, everything back in order
        let reopened = ClipboardStore::at(dir.0.clone(), decide(Some(S1), S1.logon, 1));
        let mut back = ClipboardHistory::default();
        restore(&mut back, reopened.load().unwrap());
        assert_eq!(items(&back), items(&h));
        // Deleting one entry is saved
        h.remove(1);
        reopened.save(&h).unwrap();
        assert_eq!(reopened.load().unwrap(), items(&h));
        // Clearing removes the file entirely
        h.clear();
        reopened.save(&h).unwrap();
        assert_eq!(reopened.load(), Err(LoadError::Missing));
        assert!(!dir.0.join(FILE_NAME).exists());
        // A leftover temporary file (write interrupted) is never loaded
        std::fs::write(dir.0.join(TEMP_NAME), encode(S1, &history(&[text("x")]))).unwrap();
        assert_eq!(reopened.load(), Err(LoadError::Missing));
    }

    #[test]
    fn test_new_windows_session_never_sees_old_history() {
        let dir = TempDir::new("isolation");
        let old = ClipboardStore::at(dir.0.clone(), Session::Fresh(S1));
        old.save(&history(&[text("secret from yesterday"), image(2, 2, 1)]))
            .unwrap();
        // Restart: the volatile key is gone, so the next launch is fresh. Even
        // with the same LUID value (LUIDs repeat across boots) it gets a new
        // stamp, and the old file is removed before anything is read
        let unrelated = dir.0.join("someone-elses-file.txt");
        std::fs::write(&unrelated, "keep me").unwrap();
        let next = ClipboardStore::at(dir.0.clone(), decide(None, S1.logon, S1.stamp + 5));
        assert_eq!(next.load(), Err(LoadError::Missing));
        assert!(!dir.0.join(FILE_NAME).exists(), "stale history deleted");
        assert!(unrelated.exists(), "unrelated files untouched");
        // Even if deleting failed, a stale file is refused by its session id
        std::fs::write(
            dir.0.join(FILE_NAME),
            encode(S1, &history(&[text("stale")])),
        )
        .unwrap();
        assert_eq!(next.load(), Err(LoadError::OtherSession));
        // Persistence off (session unknown): nothing read, nothing written
        let off = ClipboardStore {
            dir: dir.0.clone(),
            session: None,
        };
        assert_eq!(off.load(), Err(LoadError::Missing));
        off.save(&history(&[text("x")])).unwrap();
    }

    #[test]
    fn test_storage_is_bounded() {
        // The fullest possible history fits the file bound; anything larger
        // is refused unread
        let max_text = "\u{FFFF}".repeat(CLIPBOARD_MAX_TEXT_UNITS);
        assert!(max_text.len() <= CLIPBOARD_MAX_TEXT_UNITS * 3);
        let mut h = ClipboardHistory::default();
        h.add(image(1024, 1024, 3));
        for i in 0..CLIPBOARD_MAX_ENTRIES {
            let mut t = max_text.clone();
            t.pop();
            t.push(char::from(b'a' + i as u8));
            h.add(ClipboardItem::Text(t));
        }
        let bytes = encode(S1, &h);
        assert!(bytes.len() as u64 <= MAX_FILE_BYTES);
        assert_eq!(decode(&bytes, S1).unwrap().len(), h.len());
        let too_big = vec![0u8; MAX_FILE_BYTES as usize + 1];
        assert_eq!(decode(&too_big, S1), Err(LoadError::TooLarge));
    }

    /// The real volatile key and logon LUID (run explicitly:
    /// `cargo test clipboard_store_live -- --ignored`). Uses the real
    /// `HKCU\Software\Oeca\Session` key.
    #[test]
    #[ignore]
    fn clipboard_store_live_session_is_stable_within_a_logon() {
        let luid = session::luid_for_tests().expect("logon LUID");
        assert_ne!(luid, 0);
        let first = session::current().expect("volatile key");
        let second = session::current().expect("volatile key");
        let id = |s: Session| match s {
            Session::Resumed(id) | Session::Fresh(id) => id,
        };
        assert_eq!(id(first).logon, luid);
        assert_eq!(
            second,
            Session::Resumed(id(first)),
            "same session on relaunch"
        );
        assert!(local_app_data().is_some());
    }
}
