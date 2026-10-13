//! Start on launch: a per-user `Run` entry
//! (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Oeca`) that
//! starts the running executable when this user signs in. No elevation, no
//! service or scheduled task, nothing machine-wide.
//!
//! The setting always mirrors the real state: the entry exists, points at an
//! executable that exists, and is not disabled in Task Manager's Startup
//! apps (`...\Explorer\StartupApproved\Run`, odd first byte = disabled).
//! Enabling writes the current executable's quoted path and clears a Task
//! Manager "disabled" mark; disabling removes both.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{PCWSTR, w};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const APPROVED_KEY: PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run");
const VALUE: PCWSTR = w!("Oeca");

/// The `Run` command for `exe`: its path in quotes (spaces are safe).
pub fn command_for(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// The executable a `Run` command starts: the quoted path, or the text up
/// to the first space when unquoted. None for an empty command.
pub fn exe_of(command: &str) -> Option<PathBuf> {
    let c = command.trim();
    let path = match c.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => c.split(' ').next()?,
    };
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// Whether a `Run` command and its Task Manager mark (the
/// `StartupApproved` bytes, if any) mean Oeca starts at sign-in: the
/// command's executable exists and the mark, if present, is not "disabled".
pub fn is_active(
    command: Option<&str>,
    approved: Option<&[u8]>,
    exists: impl Fn(&Path) -> bool,
) -> bool {
    let runs = command.and_then(exe_of).is_some_and(|exe| exists(&exe));
    let disabled = approved.and_then(|b| b.first()).is_some_and(|b| b & 1 == 1);
    runs && !disabled
}

/// Whether Oeca starts when this user signs in (the real registration).
pub fn is_enabled() -> bool {
    let command = read_string(RUN_KEY, VALUE);
    let approved = read_bytes(APPROVED_KEY, VALUE);
    is_active(command.as_deref(), approved.as_deref(), Path::is_file)
}

/// Registers (`on`) or removes the entry for the running executable.
/// Returns the registration as it now is, which is what the toggle shows
/// (a failure leaves it unchanged rather than claiming success).
pub fn set_enabled(on: bool) -> bool {
    let _ = if on { register() } else { unregister() };
    is_enabled()
}

fn register() -> Result<(), ()> {
    let exe = std::env::current_exe().map_err(|_| ())?;
    let data: Vec<u8> = command_for(&exe)
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    let key = create(RUN_KEY)?;
    let r = unsafe { RegSetValueExW(key, VALUE, None, REG_SZ, Some(&data)) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    if r != ERROR_SUCCESS {
        return Err(());
    }
    // A "disabled" mark from Task Manager would keep it from starting
    delete_value(APPROVED_KEY, VALUE);
    Ok(())
}

fn unregister() -> Result<(), ()> {
    delete_value(APPROVED_KEY, VALUE);
    if delete_value(RUN_KEY, VALUE) {
        Ok(())
    } else {
        Err(())
    }
}

fn create(path: PCWSTR) -> Result<HKEY, ()> {
    let mut key = HKEY::default();
    let r = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path,
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    if r == ERROR_SUCCESS { Ok(key) } else { Err(()) }
}

/// True if the value is gone afterwards (deleted, or never there).
fn delete_value(path: PCWSTR, name: PCWSTR) -> bool {
    let mut key = HKEY::default();
    unsafe {
        let r = RegOpenKeyExW(HKEY_CURRENT_USER, path, None, KEY_SET_VALUE, &mut key);
        if r == ERROR_FILE_NOT_FOUND {
            return true;
        }
        if r != ERROR_SUCCESS {
            return false;
        }
        let r = RegDeleteValueW(key, name);
        let _ = RegCloseKey(key);
        r == ERROR_SUCCESS || r == ERROR_FILE_NOT_FOUND
    }
}

fn read_bytes(path: PCWSTR, name: PCWSTR) -> Option<Vec<u8>> {
    let mut key = HKEY::default();
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, path, None, KEY_QUERY_VALUE, &mut key) != ERROR_SUCCESS
        {
            return None;
        }
        let mut len = 0u32;
        let mut data = None;
        if RegQueryValueExW(key, name, None, None, None, Some(&mut len)) == ERROR_SUCCESS
            && len <= 64 * 1024
        {
            let mut buf = vec![0u8; len as usize];
            if RegQueryValueExW(
                key,
                name,
                None,
                None,
                Some(buf.as_mut_ptr()),
                Some(&mut len),
            ) == ERROR_SUCCESS
            {
                buf.truncate(len as usize);
                data = Some(buf);
            }
        }
        let _ = RegCloseKey(key);
        data
    }
}

fn read_string(path: PCWSTR, name: PCWSTR) -> Option<String> {
    let bytes = read_bytes(path, name)?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16(&units).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_quotes_paths_with_spaces() {
        let exe = Path::new(r"C:\Users\A User\AppData\Local\Programs\Oeca\Oeca.exe");
        let cmd = command_for(exe);
        assert_eq!(
            cmd,
            r#""C:\Users\A User\AppData\Local\Programs\Oeca\Oeca.exe""#
        );
        assert_eq!(exe_of(&cmd).as_deref(), Some(exe));
        // Unquoted commands (written by something else): up to the first space
        assert_eq!(
            exe_of(r"C:\Tools\Oeca.exe --flag").as_deref(),
            Some(Path::new(r"C:\Tools\Oeca.exe"))
        );
        assert_eq!(
            exe_of(r#"  "C:\x y\Oeca.exe" --arg "#).as_deref(),
            Some(Path::new(r"C:\x y\Oeca.exe"))
        );
        assert_eq!(exe_of(""), None);
        assert_eq!(exe_of("\"\""), None);
    }

    #[test]
    fn test_registration_state() {
        let present = |_: &Path| true;
        let missing = |_: &Path| false;
        let cmd = r#""C:\Program Files\Oeca\Oeca.exe""#;
        // No entry: off
        assert!(!is_active(None, None, present));
        // Entry for an existing exe: on
        assert!(is_active(Some(cmd), None, present));
        // Entry pointing at a missing exe (moved, deleted build): off
        assert!(!is_active(Some(cmd), None, missing));
        // Task Manager: enabled marks (even first byte) keep it on, disabled
        // marks (odd) turn it off
        for b in [0x02u8, 0x06] {
            assert!(is_active(Some(cmd), Some(&[b, 0, 0, 0]), present), "{b:#x}");
        }
        for b in [0x03u8, 0x07] {
            assert!(
                !is_active(Some(cmd), Some(&[b, 0, 0, 0]), present),
                "{b:#x}"
            );
        }
        // An empty mark is no mark
        assert!(is_active(Some(cmd), Some(&[]), present));
    }

    /// The real per-user Run entry (run explicitly: `cargo test startup_live
    /// -- --ignored`). Registers the test executable, checks it, removes it,
    /// and restores whatever was there before.
    #[test]
    #[ignore]
    fn startup_live_register_and_remove() {
        let before = read_string(RUN_KEY, VALUE);
        let approved_before = read_bytes(APPROVED_KEY, VALUE);
        assert!(set_enabled(true), "registered");
        let cmd = read_string(RUN_KEY, VALUE).unwrap();
        assert_eq!(exe_of(&cmd), std::env::current_exe().ok());
        assert!(!set_enabled(false), "removed");
        assert_eq!(read_string(RUN_KEY, VALUE), None);
        assert!(!set_enabled(false), "removing again is fine");
        // Leave the user's registration as it was
        if let Some(c) = before {
            let data: Vec<u8> = c
                .encode_utf16()
                .chain(std::iter::once(0))
                .flat_map(u16::to_le_bytes)
                .collect();
            let key = create(RUN_KEY).unwrap();
            unsafe {
                let _ = RegSetValueExW(key, VALUE, None, REG_SZ, Some(&data));
                let _ = RegCloseKey(key);
            }
        }
        if let Some(b) = approved_before {
            let key = create(APPROVED_KEY).unwrap();
            unsafe {
                let _ = RegSetValueExW(
                    key,
                    VALUE,
                    None,
                    windows::Win32::System::Registry::REG_BINARY,
                    Some(&b),
                );
                let _ = RegCloseKey(key);
            }
        }
    }
}
