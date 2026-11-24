//! Where Oeca keeps user data, and the right to change it.
//!
//! Nothing reads or writes user data until this process has (1) validated
//! the folder and (2) taken its writer lock. `dir()` is None before that, and
//! `window::run` does not start if either step fails.
//!
//! Redirection: a process started by a packaged app can see a merged view of
//! AppData where saves fork into the package and deletes fall through to the
//! user's real files. Such a process has no package identity and its folder
//! paths look normal; only what the paths resolve to gives it away. So every
//! entry of the root must live in it, and a file created in the root and in
//! the folder must land there (which is all that shows an empty overlay).
//!
//! Checked again at each change: `replace` and `remove` are the only ways
//! user data is written or deleted. They re-check the folder (still the one
//! locked), refuse links of every kind, and delete the very file they
//! checked, by its handle.
//!
//! Not detectable, so not promised: an overlay that passes creations through
//! too (it looks exactly like the real folder), links resolved out of sight
//! by a driver or bind link, and another program of the same user changing a
//! path between a check and the rename after it. Paths that cannot be
//! checked locally (network folders) are refused.
//!
//! One writer per physical folder: an OS lock on a file inside it, shared by
//! every install, sign-in session and elevation level. Windows drops it when
//! the process dies.
//!
//! Isolation: debug builds and tests cannot name the user's data, and take
//! nothing on the desktop: no hotkeys, hooks, raw input, audio or media
//! sessions, notification icon or topmost band (guarded at each of those,
//! by `ISOLATED` / `TEST_INSTANCE`). Close one with WM_CLOSE. A debug
//! build is a test instance with its data in `Oeca-Test-Data` beside its
//! executable; it refuses to run from inside AppData. An optimized one:
//! `cargo build --release --config profile.release.debug-assertions=true
//! --target-dir target/test-instance`.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{ErrorKind, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::{E_FAIL, GENERIC_WRITE, HANDLE};
use windows::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO,
    FILE_DISPOSITION_INFO_EX, FILE_DISPOSITION_INFO_EX_FLAGS, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_DELETE_ON_CLOSE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileDispositionInfo,
    FileDispositionInfoEx, GETFINALPATHNAMEBYHANDLE_FLAGS, GetFileInformationByHandle,
    GetFinalPathNameByHandleW, SetFileInformationByHandle,
};
use windows::core::{Error, Result};

/// A running test instance (a debug build of the app, not `cargo test`).
pub const TEST_INSTANCE: bool = cfg!(all(debug_assertions, not(test)));
/// Tests and test instances: never the user's data, names or registrations.
pub const ISOLATED: bool = cfg!(any(test, debug_assertions));

const FOLDER: &str = "Oeca";
const TEST_FOLDER: &str = "Oeca-Test-Data";
const LOCK_NAME: &str = "writer.lock";

const UNVERIFIED: &str = "Its data folder could not be verified as your own (redirected or inaccessible), so nothing was changed. If another app started Oeca, start it from the Start menu instead.";
const BUSY: &str = "Another Oeca is using your data, perhaps as administrator or in another sign-in. Nothing was changed.";
const IN_APPDATA: &str = "A test build cannot run from inside AppData. Nothing was changed.";

static CLAIM: OnceLock<Claim> = OnceLock::new();

/// The validated folder and its lock, held for the process's life.
#[derive(Debug)]
struct Claim {
    dir: PathBuf,
    lock: File,
}

/// A root that passed validation (nothing is locked or changed yet).
#[derive(Debug)]
pub struct Place {
    root: PathBuf,
}

/// The data folder, only once this process holds it.
pub fn dir() -> Option<PathBuf> {
    CLAIM.get().map(|c| c.dir.clone())
}

/// Checks the place this build keeps its data in.
pub fn validate() -> Result<Place> {
    let root = if ISOLATED {
        test_root()?
    } else {
        local_app_data().ok_or_else(|| fail(UNVERIFIED))?
    };
    validate_root(&root)
}

impl Place {
    /// Takes the writer lock. Until this succeeds, `dir()` stays None.
    pub fn claim(self) -> Result<()> {
        let claim = self.lock()?;
        CLAIM.set(claim).map_err(|_| fail(BUSY))
    }

    fn lock(&self) -> Result<Claim> {
        let dir = self.root.join(FOLDER);
        create_own_dir(&dir).map_err(|_| fail(UNVERIFIED))?;
        // The root can be real while this folder forks into a package
        if !creates_in_place(&dir) {
            return Err(fail(UNVERIFIED));
        }
        let path = dir.join(LOCK_NAME);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            // Not deletable or renamable while held: a second lock file would be a second writer
            .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
            // A planted link is judged as itself, never created through
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(&path)
            .map_err(|_| fail(UNVERIFIED))?;
        verify_plain(&file, &path).map_err(|_| fail(UNVERIFIED))?;
        // A predecessor that just quit may still be exiting
        for attempt in 0.. {
            match file.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if attempt < 20 => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(_) => return Err(fail(BUSY)),
            }
        }
        // Every file that will be read, replaced or deleted must really be in here
        let entries = std::fs::read_dir(&dir).map_err(|_| fail(UNVERIFIED))?;
        for entry in entries {
            let path = entry.map_err(|_| fail(UNVERIFIED))?.path();
            // Vanished while listing (another launch's probe) is nothing to check
            if !resolves_to_itself(&path) && !gone(&path) {
                return Err(fail(UNVERIFIED));
            }
        }
        Ok(Claim { dir, lock: file })
    }
}

impl Claim {
    /// Whether the lock found by path is still the very file held.
    fn still_holds(&self, dir: &Path) -> bool {
        let found = open_itself(&self.dir.join(LOCK_NAME), FILE_READ_ATTRIBUTES.0);
        same_path(dir, &self.dir)
            && found.is_ok_and(|f| identity(&f).is_some_and(|id| Some(id) == identity(&self.lock)))
    }
}

/// Writes `bytes` to a new `temp` and renames it over `path`, after checking
/// both again. A leftover `temp` is removed first, never written through.
pub fn replace(path: &Path, temp: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(refused)?;
    check_dir(dir, CLAIM.get())?;
    if temp.parent() != Some(dir) {
        return Err(refused());
    }
    check_file(path)?;
    remove(temp)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .access_mode(GENERIC_WRITE.0 | FILE_READ_ATTRIBUTES.0 | DELETE.0)
        .open(temp)?;
    // Creating it is the probe: in an overlay it lands somewhere else
    let written = verify_plain(&file, temp)
        .and_then(|()| file.write_all(bytes))
        .and_then(|()| file.sync_all());
    if let Err(e) = written {
        let _ = delete_open(&file); // only the file just created
        return Err(e);
    }
    drop(file);
    let renamed = check_dir(dir, CLAIM.get())
        .and_then(|()| check_file(path))
        .and_then(|()| std::fs::rename(temp, path));
    if renamed.is_err() {
        let _ = remove(temp);
    }
    renamed
}

/// Deletes `path` (absent is fine), after checking it again.
pub fn remove(path: &Path) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(refused)?;
    check_dir(dir, CLAIM.get())?;
    if gone(path) {
        return Ok(());
    }
    // A delete is where an overlay falls through to the real file
    if !creates_in_place(dir) {
        return Err(refused());
    }
    let file = open_itself(path, DELETE.0 | FILE_READ_ATTRIBUTES.0)?;
    verify_plain(&file, path)?;
    delete_open(&file)
}

/// Deletes the file at `path` only if it is one plain file exactly there (no
/// link, junction or second name; never a folder), no one has it open, and
/// `delete` agrees with what the open handle shows (so a swap after the
/// caller looked cannot change what is judged). Ok(false): kept. Not for
/// the data folder (that is `remove`): for Nearby's own part files in its
/// receive folder.
pub fn remove_if(
    path: &Path,
    delete: impl FnOnce(&std::fs::Metadata) -> bool,
) -> std::io::Result<bool> {
    // No share mode: open anywhere else (a transfer writing it), it is not opened here.
    // No backup semantics: a folder or junction does not open at all.
    let file = OpenOptions::new()
        .access_mode(DELETE.0 | FILE_READ_ATTRIBUTES.0)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)?;
    verify_plain(&file, path)?;
    if !delete(&file.metadata()?) {
        return Ok(false);
    }
    delete_open(&file)?;
    Ok(true)
}

fn check_dir(dir: &Path, claim: Option<&Claim>) -> std::io::Result<()> {
    // No claim is a test's own folder: without one, stores get no folder
    if claim.is_some_and(|c| !c.still_holds(dir)) || !resolves_to_itself(dir) {
        return Err(refused());
    }
    Ok(())
}

/// Absent, or a plain file that really is at `path`.
fn check_file(path: &Path) -> std::io::Result<()> {
    match open_itself(path, FILE_READ_ATTRIBUTES.0) {
        Ok(file) => verify_plain(&file, path),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// One name, no reparse point, at exactly `path`.
fn verify_plain(file: &File, path: &Path) -> std::io::Result<()> {
    let special = FILE_ATTRIBUTE_DIRECTORY.0 | FILE_ATTRIBUTE_REPARSE_POINT.0;
    let plain = info_of(file)
        .is_some_and(|i| i.dwFileAttributes & special == 0 && i.nNumberOfLinks == 1)
        && final_of(file).is_some_and(|p| same_path(&p, path));
    if plain { Ok(()) } else { Err(refused()) }
}

/// Deletes the open file itself, whatever its path points at by now.
fn delete_open(file: &File) -> std::io::Result<()> {
    let handle = HANDLE(file.as_raw_handle());
    let at_once = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_INFO_EX_FLAGS(
            FILE_DISPOSITION_FLAG_DELETE.0 | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS.0,
        ),
    };
    let on_close = FILE_DISPOSITION_INFO { DeleteFile: true };
    unsafe {
        // Frees the name at once; file systems without that delete on close
        SetFileInformationByHandle(
            handle,
            FileDispositionInfoEx,
            (&raw const at_once).cast(),
            std::mem::size_of_val(&at_once) as u32,
        )
        .or_else(|_| {
            SetFileInformationByHandle(
                handle,
                FileDispositionInfo,
                (&raw const on_close).cast(),
                std::mem::size_of_val(&on_close) as u32,
            )
        })
        .map_err(|_| std::io::Error::last_os_error())
    }
}

fn info_of(file: &File) -> Option<BY_HANDLE_FILE_INFORMATION> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.ok()?;
    Some(info)
}

/// Volume and file number: the same for every path to one file.
fn identity(file: &File) -> Option<(u32, u32, u32)> {
    let i = info_of(file)?;
    Some((i.dwVolumeSerialNumber, i.nFileIndexHigh, i.nFileIndexLow))
}

fn gone(path: &Path) -> bool {
    matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == ErrorKind::NotFound)
}

fn refused() -> std::io::Error {
    ErrorKind::PermissionDenied.into()
}

/// Creates `dir` (its parent exists) if need be; it must then be a real
/// folder right there, so nothing is ever created through a link.
fn create_own_dir(dir: &Path) -> std::io::Result<()> {
    match std::fs::create_dir(dir) {
        Err(e) if e.kind() != ErrorKind::AlreadyExists => Err(e),
        _ if resolves_to_itself(dir) => Ok(()),
        _ => Err(refused()),
    }
}

fn validate_root(root: &Path) -> Result<Place> {
    if !is_local(root) || !resolves_to_itself(root) {
        return Err(fail(UNVERIFIED));
    }
    if has_foreign_entry(root).unwrap_or(true) {
        return Err(fail(UNVERIFIED));
    }
    if !creates_in_place(root) {
        return Err(fail(UNVERIFIED));
    }
    Ok(Place {
        root: root.to_owned(),
    })
}

/// Whether anything listed in `root` really lives elsewhere (a merged view).
/// Links are judged as themselves: the user's own junctions are fine.
fn has_foreign_entry(root: &Path) -> std::io::Result<bool> {
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        // Gone or inaccessible entries are no evidence either way
        let Some(real) = final_path(&path, true) else {
            continue;
        };
        if !real.parent().is_some_and(|p| same_path(p, root)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether a file created in `dir` lands in `dir`. It removes itself on close.
fn creates_in_place(dir: &Path) -> bool {
    static PROBES: AtomicU32 = AtomicU32::new(0);
    let unique = std::time::UNIX_EPOCH.elapsed().map_or(0, |t| t.as_nanos());
    let n = PROBES.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!(".oeca-probe-{}-{unique}-{n}", std::process::id()));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .access_mode(GENERIC_WRITE.0 | DELETE.0)
        .custom_flags(FILE_FLAG_DELETE_ON_CLOSE.0)
        .open(&path)
        .ok()
        .and_then(|f| final_of(&f))
        .is_some_and(|p| same_path(&p, &path))
}

/// A drive-letter path: only those can be resolved to where they really are
/// (a share's server follows links out of sight).
fn is_local(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_))
    ) && path.is_absolute()
}

/// Whether `path` is no link and, links above it followed, exactly where it
/// says it is. (A link to a sibling differing only in case would pass the
/// path comparison alone.)
fn resolves_to_itself(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| !m.file_type().is_symlink())
        && final_path(path, false).is_some_and(|p| same_path(&p, path))
}

fn final_path(path: &Path, link_itself: bool) -> Option<PathBuf> {
    let file = if link_itself {
        open_itself(path, FILE_READ_ATTRIBUTES.0).ok()?
    } else {
        open_with(path, FILE_READ_ATTRIBUTES.0, FILE_FLAG_BACKUP_SEMANTICS.0).ok()?
    };
    final_of(&file)
}

/// Opens what is at `path` itself, not what it links to.
fn open_itself(path: &Path, access: u32) -> std::io::Result<File> {
    let flags = FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0;
    open_with(path, access, flags)
}

fn open_with(path: &Path, access: u32, flags: u32) -> std::io::Result<File> {
    OpenOptions::new()
        .access_mode(access)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
        .custom_flags(flags)
        .open(path)
}

/// The physical path of an open file, without the `\\?\` prefix.
fn final_of(file: &File) -> Option<PathBuf> {
    let mut buf = vec![0u16; 32 * 1024];
    let len = unsafe {
        GetFinalPathNameByHandleW(
            HANDLE(file.as_raw_handle()),
            &mut buf,
            GETFINALPATHNAMEBYHANDLE_FLAGS(0),
        )
    } as usize;
    if len == 0 || len >= buf.len() {
        return None;
    }
    let path = String::from_utf16(&buf[..len]).ok()?;
    Some(PathBuf::from(match path.strip_prefix(r"\\?\UNC\") {
        Some(share) => format!(r"\\{share}"),
        None => path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned(),
    }))
}

/// Windows' own file-name comparison: lower-casing would merge names that
/// NTFS keeps apart (the Kelvin sign and k).
fn same_path(a: &Path, b: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    let key = |p: &Path| {
        let mut text: Vec<u16> = p.as_os_str().encode_wide().collect();
        while text.last() == Some(&u16::from(b'\\')) {
            text.pop();
        }
        text
    };
    let (a, b) = (key(a), key(b));
    !a.is_empty() && !b.is_empty() && unsafe { CompareStringOrdinal(&a, &b, true) } == CSTR_EQUAL
}

fn fail(why: &str) -> Error {
    Error::new(E_FAIL, why)
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

fn in_app_data(path: &Path) -> bool {
    let named = path
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case("AppData"));
    // Or the user's, moved to a folder of another name
    named || local_app_data().is_some_and(|local| path.ancestors().any(|p| same_path(p, &local)))
}

/// The physical folder of this executable, which must not be in AppData.
fn isolated_base() -> Result<PathBuf> {
    let exe = std::env::current_exe().map_err(|_| fail(UNVERIFIED))?;
    let base = exe
        .parent()
        .and_then(|d| final_path(d, false))
        .ok_or_else(|| fail(UNVERIFIED))?;
    if !is_local(&base) {
        return Err(fail(UNVERIFIED));
    }
    if in_app_data(&base) {
        return Err(fail(IN_APPDATA));
    }
    Ok(base)
}

fn test_root() -> Result<PathBuf> {
    let root = isolated_base()?.join(TEST_FOLDER);
    // Not a link: it could lead back into AppData
    create_own_dir(&root).map_err(|_| fail(UNVERIFIED))?;
    Ok(root)
}

/// The scratch folder: Windows' for Oeca, one beside the executable for
/// tests and test instances (Windows' is inside AppData, and holds the real
/// Oeca's staging folders).
pub fn temp_dir() -> PathBuf {
    if !ISOLATED {
        return std::env::temp_dir();
    }
    // No fallback: the only other choice is the user's AppData
    let root = test_root().expect("test builds run outside AppData");
    let dir = root.join("Temp");
    create_own_dir(&dir).expect("the test scratch folder is a real folder");
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Root(PathBuf);
    impl Root {
        fn new(name: &str) -> Self {
            let p = temp_dir().join(format!("oeca-data-dir-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn junction(link: &Path, target: &Path) {
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(made.status.success(), "junction");
    }

    #[test]
    fn test_builds_cannot_reach_user_data() {
        const { assert!(ISOLATED && !TEST_INSTANCE) };
        assert!(!in_app_data(&temp_dir()));
        assert!(!in_app_data(&test_root().unwrap()));
        assert!(in_app_data(Path::new(r"C:\Users\A\appdata\Local\Temp")));
        assert_eq!(dir(), None, "unclaimed: stores have no folder");
        // The whole start-up check, on the place a test build would use
        assert!(!in_app_data(&validate().unwrap().root));
        assert!(crate::settings_store::SettingsStore::open().load().is_err());
    }

    #[test]
    fn test_a_plain_folder_is_accepted_and_the_probe_leaves_nothing() {
        let root = Root::new("plain");
        let place = validate_root(&root.0).unwrap();
        assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0, "probe gone");
        let claim = place.lock().unwrap();
        assert_eq!(claim.dir, root.0.join(FOLDER));
        assert!(claim.dir.join(LOCK_NAME).exists());
        // Another spelling of the same folder is the same folder
        let upper = PathBuf::from(root.0.to_str().unwrap().to_uppercase());
        assert!(validate_root(&upper).is_ok());
    }

    #[test]
    fn test_redirected_unknown_and_relative_roots_are_refused_untouched() {
        let (real, holder) = (Root::new("real"), Root::new("holder"));
        std::fs::write(real.0.join("users-file"), "keep").unwrap();
        let view = holder.0.join("Local");
        junction(&view, &real.0);
        // Each signal on its own sees the redirection
        assert!(!resolves_to_itself(&view));
        assert!(has_foreign_entry(&view).unwrap());
        assert!(!creates_in_place(&view));
        assert!(validate_root(&view).is_err());
        let left: Vec<_> = std::fs::read_dir(&real.0)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, ["users-file"], "target untouched");
        std::fs::remove_dir(&view).unwrap(); // the link only
        assert!(validate_root(&holder.0.join("missing")).is_err());
        assert!(validate_root(Path::new("relative")).is_err());
    }

    #[test]
    fn test_the_users_own_junctions_in_the_root_are_not_redirection() {
        let (root, target) = (Root::new("links"), Root::new("links-target"));
        junction(&root.0.join("Application Data"), &target.0);
        assert!(validate_root(&root.0).is_ok());
        std::fs::remove_dir(root.0.join("Application Data")).unwrap();
    }

    #[test]
    fn test_a_redirected_data_folder_or_entry_is_refused() {
        let (root, target) = (Root::new("folder"), Root::new("folder-target"));
        std::fs::write(target.0.join("clipboard-history.bin"), "keep").unwrap();
        let dir = root.0.join(FOLDER);
        junction(&dir, &target.0);
        assert!(validate_root(&root.0).unwrap().lock().is_err());
        assert!(
            !target.0.join(LOCK_NAME).exists(),
            "nothing written through"
        );
        std::fs::remove_dir(&dir).unwrap();
        std::fs::create_dir(&dir).unwrap();
        junction(&dir.join("settings.ini"), &target.0);
        assert!(validate_root(&root.0).unwrap().lock().is_err());
        std::fs::remove_dir(dir.join("settings.ini")).unwrap();
        assert!(target.0.join("clipboard-history.bin").exists());
    }

    fn refused_untouched(path: &Path, victim: &Path) {
        let temp = path.with_extension("new");
        assert!(replace(path, &temp, b"new").is_err(), "replace: {path:?}");
        assert!(remove(path).is_err(), "remove: {path:?}");
        assert_eq!(std::fs::read(victim).unwrap(), b"keep", "{path:?}");
        assert!(!temp.exists(), "nothing left: {path:?}");
    }

    #[test]
    fn test_an_empty_overlay_is_caught_by_the_create_probe_alone() {
        let (real, holder) = (Root::new("empty"), Root::new("empty-holder"));
        let view = holder.0.join("Local");
        junction(&view, &real.0);
        assert!(!has_foreign_entry(&view).unwrap(), "nothing to list");
        assert!(!creates_in_place(&view), "but a created file gives it away");
        assert!(validate_root(&view).is_err());
        assert_eq!(std::fs::read_dir(&real.0).unwrap().count(), 0, "probe gone");
        std::fs::remove_dir(&view).unwrap();
    }

    #[test]
    fn test_a_folder_redirected_after_start_is_refused_at_the_operation() {
        let (real, holder) = (Root::new("later"), Root::new("later-holder"));
        let victim = real.0.join("clipboard-history.bin");
        std::fs::write(&victim, "keep").unwrap();
        let view = holder.0.join("Oeca");
        junction(&view, &real.0);
        refused_untouched(&view.join("clipboard-history.bin"), &victim);
        // Absent there: nothing is created through the link either
        let absent = view.join("settings.ini");
        assert!(replace(&absent, &view.join("settings.tmp"), b"new").is_err());
        assert_eq!(std::fs::read_dir(&real.0).unwrap().count(), 1);
        std::fs::remove_dir(&view).unwrap();
        // A link further up is the same
        let up = holder.0.join("Local");
        junction(&up, &real.0);
        std::fs::create_dir(real.0.join("Oeca")).unwrap();
        let victim = real.0.join("Oeca/settings.ini");
        std::fs::write(&victim, "keep").unwrap();
        refused_untouched(&up.join("Oeca").join("settings.ini"), &victim);
        std::fs::remove_dir(&up).unwrap();
    }

    #[test]
    fn test_links_planted_as_the_file_or_its_temp_are_never_written_through() {
        let (dir, away) = (Root::new("planted"), Root::new("planted-target"));
        let victim = away.0.join("victim");
        std::fs::write(&victim, "keep").unwrap();
        let (path, temp) = (dir.0.join("data.bin"), dir.0.join("data.tmp"));

        // A hard link has no reparse point and resolves to its own name
        std::fs::hard_link(&victim, &path).unwrap();
        assert!(resolves_to_itself(&path), "invisible to path checks");
        refused_untouched(&path, &victim);
        std::fs::remove_file(&path).unwrap();
        std::fs::hard_link(&victim, &temp).unwrap();
        assert!(replace(&path, &temp, b"new").is_err(), "linked temp");
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        assert!(!path.exists());
        std::fs::remove_file(&temp).unwrap();

        // A junction where the file should be
        junction(&path, &away.0);
        refused_untouched(&path, &victim);
        std::fs::remove_dir(&path).unwrap();

        // Symbolic links (these need Developer Mode or elevation to make)
        match std::os::windows::fs::symlink_file(&victim, &path) {
            Ok(()) => {
                refused_untouched(&path, &victim);
                std::fs::remove_file(&path).unwrap();
                std::os::windows::fs::symlink_file(&victim, &temp).unwrap();
                assert!(replace(&path, &temp, b"new").is_err(), "linked temp");
                assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
                std::fs::remove_file(&temp).unwrap();
                println!("file symlinks: tested");
            }
            Err(e) => println!("file symlinks: NOT TESTED, cannot create one ({e})"),
        }
        match std::os::windows::fs::symlink_dir(&away.0, &path) {
            Ok(()) => {
                assert!(validate_root(&path).is_err(), "linked root");
                refused_untouched(&path.join("victim"), &victim);
                std::fs::remove_dir(&path).unwrap();
                println!("directory symlinks: tested");
            }
            Err(e) => println!("directory symlinks: NOT TESTED, cannot create one ({e})"),
        }
    }

    /// What a file symlink is to these checks (and needs no privilege): a
    /// file with a reparse point, here under a tag no driver knows.
    fn make_reparse_point(path: &Path) {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn DeviceIoControl(
                device: HANDLE,
                code: u32,
                input: *const u8,
                input_len: u32,
                output: *mut u8,
                output_len: u32,
                returned: *mut u32,
                overlapped: *mut core::ffi::c_void,
            ) -> i32;
        }
        const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
        // REPARSE_GUID_DATA_BUFFER: tag, data length, reserved, GUID, data
        let mut buffer = [0u8; 28];
        buffer[..4].copy_from_slice(&0x0000_0ECAu32.to_le_bytes());
        buffer[4..6].copy_from_slice(&4u16.to_le_bytes());
        buffer[8..24].copy_from_slice(b"Oeca-test-reparse".get(..16).unwrap());
        let file = open_itself(path, GENERIC_WRITE.0).unwrap();
        let mut got = 0;
        let done = unsafe {
            DeviceIoControl(
                HANDLE(file.as_raw_handle()),
                FSCTL_SET_REPARSE_POINT,
                buffer.as_ptr(),
                buffer.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut got,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(done, 0, "{}", std::io::Error::last_os_error());
    }

    #[test]
    fn test_file_reparse_points_are_refused() {
        let dir = Root::new("reparse");
        let (path, temp) = (dir.0.join("data.bin"), dir.0.join("data.tmp"));
        std::fs::write(&path, "").unwrap();
        make_reparse_point(&path);
        assert!(replace(&path, &temp, b"new").is_err(), "as the file");
        assert!(remove(&path).is_err());
        assert!(!gone(&path) && !temp.exists());

        let other = dir.0.join("other.bin");
        std::fs::write(&temp, "").unwrap();
        make_reparse_point(&temp);
        assert!(replace(&other, &temp, b"new").is_err(), "as the temp");
        assert!(gone(&other) && !gone(&temp));

        let root = Root::new("reparse-lock");
        let lock = root.0.join(FOLDER).join(LOCK_NAME);
        std::fs::create_dir(lock.parent().unwrap()).unwrap();
        std::fs::write(&lock, "").unwrap();
        make_reparse_point(&lock);
        assert!(
            validate_root(&root.0).unwrap().lock().is_err(),
            "as the lock"
        );

        // Test cleanup: the link itself, by handle (it cannot be followed)
        for link in [&path, &temp, &lock] {
            delete_open(&open_itself(link, DELETE.0).unwrap()).unwrap();
        }
    }

    #[test]
    fn test_plain_files_are_replaced_and_removed() {
        let dir = Root::new("plain-ops");
        let (path, temp) = (dir.0.join("data.bin"), dir.0.join("data.tmp"));
        std::fs::write(&temp, "left by an interrupted write").unwrap();
        for bytes in [&b"first"[..], b"second"] {
            replace(&path, &temp, bytes).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            assert!(!temp.exists());
        }
        // Someone still reading it does not keep the name busy
        let reader = std::fs::File::open(&path).unwrap();
        remove(&path).unwrap();
        replace(&path, &temp, b"third").unwrap();
        drop(reader);
        remove(&path).unwrap();
        remove(&path).unwrap(); // absent is fine
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0, "no residue");
    }

    #[test]
    fn test_operations_only_work_in_the_folder_still_locked() {
        let (a, b) = (Root::new("held-a"), Root::new("held-b"));
        let first = validate_root(&a.0).unwrap().lock().unwrap();
        let second = validate_root(&b.0).unwrap().lock().unwrap();
        assert!(check_dir(&first.dir, Some(&first)).is_ok());
        assert!(
            check_dir(&second.dir, Some(&first)).is_err(),
            "another folder"
        );
        // The path now leading to a different lock file (a remapped drive, a swap)
        let swapped = Claim {
            dir: second.dir.clone(),
            lock: first.lock,
        };
        assert!(check_dir(&second.dir, Some(&swapped)).is_err());
    }

    #[test]
    fn test_a_linked_lock_file_is_refused() {
        let (root, away) = (Root::new("lock-link"), Root::new("lock-link-target"));
        std::fs::write(away.0.join("victim"), "keep").unwrap();
        let dir = root.0.join(FOLDER);
        std::fs::create_dir(&dir).unwrap();
        std::fs::hard_link(away.0.join("victim"), dir.join(LOCK_NAME)).unwrap();
        assert!(validate_root(&root.0).unwrap().lock().is_err());
    }

    #[test]
    fn test_only_local_paths_compared_as_windows_does() {
        for p in [
            r"\\server\share\Local",
            r"\\?\C:\Local",
            r"\\.\C:\Local",
            r"C:Local",
            r"\Local",
        ] {
            assert!(!is_local(Path::new(p)), "{p}");
            assert!(validate_root(Path::new(p)).is_err(), "{p}");
        }
        assert!(is_local(Path::new(r"C:\Users")));
        let same = |a: &str, b: &str| same_path(Path::new(a), Path::new(b));
        assert!(same(r"C:\Users\Kim\", r"c:\users\KIM"));
        assert!(
            !same("C:\\Users\\\u{212A}im", r"C:\Users\kim"),
            "Kelvin sign"
        );
        assert!(!same(r"C:\Users\Kim", r"C:\Users\Kim2"));
    }

    #[test]
    fn test_a_linked_scratch_folder_is_refused() {
        let (base, away) = (Root::new("scratch"), Root::new("scratch-target"));
        let link = base.0.join(TEST_FOLDER);
        junction(&link, &away.0);
        assert!(create_own_dir(&link).is_err());
        assert_eq!(std::fs::read_dir(&away.0).unwrap().count(), 0);
        std::fs::remove_dir(&link).unwrap();
    }

    /// The other process of `test_one_writer_per_folder_across_processes`.
    #[test]
    fn writer_lock_child() {
        let Some(root) = std::env::var_os("OECA_LOCK_TEST_ROOT").map(PathBuf::from) else {
            return;
        };
        assert!(!in_app_data(&root));
        let got = validate_root(&root).unwrap().lock().is_ok();
        std::fs::write(root.join("child-result"), got.to_string()).unwrap();
    }

    #[test]
    fn test_one_writer_per_folder_across_processes() {
        let root = Root::new("writer");
        let child_gets_it = || {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "data_dir::tests::writer_lock_child"])
                .env("OECA_LOCK_TEST_ROOT", &root.0)
                .output()
                .unwrap()
                .status;
            assert!(status.success());
            std::fs::read_to_string(root.0.join("child-result")).unwrap()
        };
        let place = validate_root(&root.0).unwrap();
        let held = place.lock().unwrap();
        assert!(place.lock().is_err(), "a second writer in this process");
        assert_eq!(child_gets_it(), "false", "a second writer process");
        let lock = held.dir.join(LOCK_NAME);
        assert!(std::fs::remove_file(&lock).is_err(), "cannot be deleted");
        assert!(std::fs::rename(&held.dir, root.0.join("moved")).is_err());
        assert!(held.still_holds(&held.dir));
        drop(held);
        assert_eq!(child_gets_it(), "true", "free once released");
        // The child exited without unlocking: Windows released it
        assert!(place.lock().is_ok());
    }

    /// One of the racers of `test_simultaneous_starts_give_one_writer_and_a_crash_frees_it`.
    #[test]
    fn writer_race_child() {
        let Some(root) = std::env::var_os("OECA_RACE_TEST_ROOT").map(PathBuf::from) else {
            return;
        };
        assert!(!in_app_data(&root));
        let claim = validate_root(&root).unwrap().lock();
        let result = root.join(format!("racer-{}-{}", std::process::id(), claim.is_ok()));
        std::fs::write(result, "").unwrap();
        // Held until killed, as by a crash
        std::thread::sleep(std::time::Duration::from_secs(30));
    }

    #[test]
    fn test_simultaneous_starts_give_one_writer_and_a_crash_frees_it() {
        let root = Root::new("race");
        let mut racers: Vec<_> = (0..6)
            .map(|_| {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "data_dir::tests::writer_race_child"])
                    .env("OECA_RACE_TEST_ROOT", &root.0)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect();
        let began = std::time::Instant::now();
        let results = || -> Vec<String> {
            std::fs::read_dir(&root.0)
                .unwrap()
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .filter(|n| n.starts_with("racer-"))
                .collect()
        };
        while results().len() < racers.len() && began.elapsed().as_secs() < 20 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let results = results();
        // Only these children: killed by their own handles
        for racer in &mut racers {
            let _ = racer.kill();
            let _ = racer.wait();
        }
        assert_eq!(results.len(), racers.len(), "all answered: {results:?}");
        let writers = results.iter().filter(|n| n.ends_with("-true")).count();
        assert_eq!(writers, 1, "{results:?}");
        assert!(
            validate_root(&root.0).unwrap().lock().is_ok(),
            "freed by the crash"
        );
    }
}
