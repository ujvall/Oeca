//! Session-scoped clipboard history persistence: the history survives Oeca
//! quitting (tray Quit, window close, a crash) and relaunching within the
//! same Windows sign-in, and never reaches a later one.
//!
//! Identity: the terminal session and its sign-in time, asked of Windows on
//! each launch. Normal and elevated Oeca share it (their logon LUIDs differ),
//! and there is nothing stored that could be lost or damaged.
//!
//! Deleting needs proof. A saved file is
//! - ours: tagged with this sign-in (or, from older versions, with one of its
//!   two logon LUIDs after it began): loaded, replaced, removed when cleared;
//! - an earlier sign-in's: intact, tagged before this one began and before
//!   this process's own logon began (two separate Windows sources), and no
//!   other sign-in of this user that could own it still exists: deleted at
//!   launch;
//! - anything else (damaged, unreadable, unknown): never loaded, replaced or
//!   deleted. History then stays in memory while it is there.
//!
//! Storage: one file, `clipboard-history.bin` in the claimed data folder
//! (`data_dir`: validated, one writer; not encrypted), tagged with the
//! sign-in and a checksum. Writes go to a temporary file renamed into place,
//! so a partial write leaves the previous file (or nothing) behind.
//!
//! Two sources: Windows promises nothing about the session's sign-in time
//! across lock, resume or reconnect. The process's logon session has its own
//! time, set when it was created. Were the first moved forward, the second
//! still predates the file, which then stays, unloaded, until a later
//! sign-in. Limits: a process from a logon made inside the session (runas)
//! has only the first, in effect; and both are wall-clock times, so a file
//! of an older version tagged after the clock was set back reads as earlier.
//!
//! Privacy: contents are never logged; failures only disable persistence.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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

/// One Windows sign-in: its terminal session and when it began. (Files from
/// older versions hold a logon LUID and when Oeca first saw it.)
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

/// No real sign-in time is older (2020-01-01, as a FILETIME).
const OLDEST_STAMP: u64 = 132_223_104_000_000_000;

/// The sign-in this process runs in, and what is needed to judge a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignIn {
    pub id: SessionId,
    /// Both halves' logon LUIDs (normal and elevated), for older files.
    pub logons: [u64; 2],
    /// When this user's other live sign-ins began (None: unknown).
    pub others: Option<Vec<u64>>,
    /// When this process's logon session began (None: unknown, so no file
    /// can be shown to be an earlier sign-in's).
    pub logon_began: Option<u64>,
}

/// Whose a saved file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Absent,
    /// This sign-in's.
    Mine,
    /// Provably an ended, earlier sign-in's, or holding nothing at all.
    Disposable,
    /// Not provable: never loaded, replaced or deleted.
    Unknown,
}

impl SignIn {
    /// Whose a file tagged `saved` is (pure).
    pub fn owner(&self, saved: SessionId) -> Owner {
        let start = self.id.stamp;
        let older_version = self.logons.contains(&saved.logon) && saved.stamp >= start;
        if saved == self.id || older_version {
            return Owner::Mine;
        }
        let plausible = saved.stamp >= OLDEST_STAMP && saved.stamp < start;
        // The second source: the session's time alone could have moved
        let before_logon = self.logon_began.is_some_and(|t| saved.stamp < t);
        // A sign-in that was already running when the file was made may own it
        let ended = self
            .others
            .as_ref()
            .is_some_and(|o| o.iter().all(|&began| began > saved.stamp));
        if plausible && before_logon && ended {
            Owner::Disposable
        } else {
            Owner::Unknown
        }
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

static SIGN_IN: OnceLock<Option<SignIn>> = OnceLock::new();

/// The saved id of a file, before anything else in it is trusted.
fn tag(bytes: &[u8]) -> Option<SessionId> {
    SessionId::from_bytes(bytes.strip_prefix(MAGIC)?.get(..16)?)
}

/// The history file in `dir`, as seen from one sign-in. No sign-in or no
/// folder: persistence is off, so nothing is read, kept or deleted.
pub struct ClipboardStore {
    dir: Option<PathBuf>,
    me: Option<SignIn>,
    /// Size and time of the file last verified as ours (no re-read per copy).
    verified: Cell<Option<(u64, std::time::SystemTime)>>,
}

impl ClipboardStore {
    /// This sign-in's store in the claimed data folder, after removing what
    /// an earlier sign-in provably left there.
    pub fn open() -> Self {
        let me = SIGN_IN.get_or_init(sign_in::current).clone();
        Self::at(crate::data_dir::dir(), me)
    }

    /// The store in `dir` for `me` (tests use a temporary `dir`).
    pub fn at(dir: Option<PathBuf>, me: Option<SignIn>) -> Self {
        let store = Self::new(dir, me);
        if let Some((dir, me)) = store.active() {
            let history = (FILE_NAME, store.history_owner(dir, me));
            for (name, owner) in [history, (TEMP_NAME, temp_owner(dir, me))] {
                if owner == Owner::Disposable {
                    let _ = crate::data_dir::remove(&dir.join(name));
                }
            }
        }
        store
    }

    fn new(dir: Option<PathBuf>, me: Option<SignIn>) -> Self {
        Self {
            dir,
            me,
            verified: Cell::new(None),
        }
    }

    fn active(&self) -> Option<(&PathBuf, &SignIn)> {
        self.dir.as_ref().zip(self.me.as_ref())
    }

    /// The saved history of this sign-in, if any and intact.
    pub fn load(&self) -> Result<Vec<ClipboardItem>, LoadError> {
        let (dir, me) = self.active().ok_or(LoadError::Missing)?;
        let path = dir.join(FILE_NAME);
        let len = std::fs::metadata(&path)
            .map_err(|_| LoadError::Missing)?
            .len();
        if len > MAX_FILE_BYTES {
            return Err(LoadError::TooLarge);
        }
        let bytes = std::fs::read(&path).map_err(|_| LoadError::Missing)?;
        let saved = tag(&bytes).ok_or(LoadError::Corrupt)?;
        let items = decode(&bytes, saved)?;
        if me.owner(saved) != Owner::Mine {
            return Err(LoadError::OtherSession);
        }
        Ok(items)
    }

    /// Whose the history file is: intact as a whole, then by its tag.
    fn history_owner(&self, dir: &Path, me: &SignIn) -> Owner {
        let path = dir.join(FILE_NAME);
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Owner::Absent,
            Err(_) => return Owner::Unknown,
        };
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
            return Owner::Unknown;
        }
        let seen = meta.modified().ok().map(|t| (meta.len(), t));
        if seen.is_some() && seen == self.verified.get() {
            return Owner::Mine;
        }
        let owner = std::fs::read(&path)
            .ok()
            .and_then(|b| tag(&b).filter(|&id| decode(&b, id).is_ok()))
            .map_or(Owner::Unknown, |id| me.owner(id));
        if owner == Owner::Mine {
            self.verified.set(seen);
        }
        owner
    }

    /// Saves the history (an empty one removes the file). Written to a
    /// temporary file, flushed, then renamed over the old one. Never over a
    /// file whose owner is unknown: that is an error, and it stays as it is.
    pub fn save(&self, history: &ClipboardHistory) -> std::io::Result<()> {
        let Some((dir, me)) = self.active() else {
            return Ok(());
        };
        if history.is_empty() {
            return self.wipe();
        }
        if self.history_owner(dir, me) == Owner::Unknown || temp_owner(dir, me) == Owner::Unknown {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
        let path = dir.join(FILE_NAME);
        crate::data_dir::replace(&path, &dir.join(TEMP_NAME), &encode(me.id, history))?;
        let seen = std::fs::symlink_metadata(&path)
            .and_then(|m| Ok((m.len(), m.modified()?)))
            .ok();
        self.verified.set(seen);
        Ok(())
    }

    /// Removes this sign-in's files and provably stale ones, nothing else.
    pub fn wipe(&self) -> std::io::Result<()> {
        let Some((dir, me)) = self.active() else {
            return Ok(());
        };
        let mut result = Ok(());
        let history = (FILE_NAME, self.history_owner(dir, me));
        for (name, owner) in [history, (TEMP_NAME, temp_owner(dir, me))] {
            match owner {
                Owner::Absent => {}
                Owner::Mine | Owner::Disposable => {
                    if let Err(e) = crate::data_dir::remove(&dir.join(name)) {
                        result = Err(e);
                    }
                }
                Owner::Unknown => result = Err(std::io::ErrorKind::PermissionDenied.into()),
            }
        }
        result
    }
}

/// Whose an interrupted write is. It has no checksum yet, but the tag is the
/// first thing written; one with only zeros (power loss) holds nothing.
fn temp_owner(dir: &Path, me: &SignIn) -> Owner {
    let path = dir.join(TEMP_NAME);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Owner::Absent,
        Err(_) => return Owner::Unknown,
    };
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return Owner::Unknown;
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return Owner::Unknown;
    };
    if bytes.iter().all(|&b| b == 0) {
        return Owner::Disposable;
    }
    tag(&bytes).map_or(Owner::Unknown, |id| me.owner(id))
}

/// The Windows session is ending (shutdown, restart, sign-out): best effort,
/// remove this sign-in's saved history now. The sign-in found at launch is
/// used, as Windows may no longer report it. Isolation does not depend on it.
pub fn wipe_for_session_end() {
    if let Some(me) = SIGN_IN.get() {
        let _ = ClipboardStore::new(crate::data_dir::dir(), me.clone()).wipe();
    }
}

/// This process's Windows sign-in, read from Windows (nothing stored).
mod sign_in {
    use super::{SessionId, SignIn};
    use windows::Win32::Foundation::{CloseHandle, E_ACCESSDENIED, HANDLE, LUID};
    use windows::Win32::Security::Authentication::Identity::{
        LsaFreeReturnBuffer, LsaGetLogonSessionData, SECURITY_LOGON_SESSION_DATA,
    };
    use windows::Win32::Security::{
        GetTokenInformation, LookupAccountSidW, SID_NAME_USE, TOKEN_LINKED_TOKEN, TOKEN_STATISTICS,
        TOKEN_USER, TokenLinkedToken, TokenStatistics, TokenUser,
    };
    use windows::Win32::System::RemoteDesktop::{
        WTS_CURRENT_SESSION, WTS_SESSION_INFOW, WTSEnumerateSessionsW, WTSFreeMemory, WTSINFOW,
        WTSQuerySessionInformationW, WTSSessionInfo,
    };
    use windows::core::{PWSTR, Result};

    /// `GetCurrentProcessToken`'s pseudo-handle.
    fn own_token() -> HANDLE {
        HANDLE(-4isize as *mut _)
    }

    /// The sign-in, or None if Windows would not say or the session is
    /// someone else's (elevated as another account): then nothing is saved,
    /// loaded or cleaned up.
    ///
    /// It is the terminal session's number plus the time it was signed in,
    /// which normal and elevated processes share, unlike their logon LUIDs.
    pub fn current() -> Option<SignIn> {
        let mine = info(WTS_CURRENT_SESSION).ok()?;
        let start = u64::try_from(mine.LogonTime).ok().filter(|&t| t != 0)?;
        let (domain, name) = account()?;
        // Copies: the fields are compared as NUL-terminated, truncated text
        if mine.UserName[0] == 0
            || !same_name(&name, &mine.UserName)
            || !same_name(&domain, &mine.Domain)
        {
            return None;
        }
        let own = luid(own_token())?;
        Some(SignIn {
            id: SessionId {
                logon: u64::from(mine.SessionId),
                stamp: start,
            },
            logons: [own, linked_luid().unwrap_or(own)],
            others: other_sessions(&mine),
            logon_began: logon_began(own),
        })
    }

    /// When this user's other signed-in sessions (disconnected ones too)
    /// began; None if that cannot be known, so nothing counts as earlier.
    fn other_sessions(mine: &WTSINFOW) -> Option<Vec<u64>> {
        unsafe {
            let mut list: *mut WTS_SESSION_INFOW = std::ptr::null_mut();
            let mut count = 0u32;
            WTSEnumerateSessionsW(None, 0, 1, &mut list, &mut count).ok()?;
            if list.is_null() {
                return None;
            }
            let ids: Vec<u32> = std::slice::from_raw_parts(list, count as usize)
                .iter()
                .map(|s| s.SessionId)
                .collect();
            WTSFreeMemory(list as *mut _);
            let mut starts = Vec::new();
            for id in ids.into_iter().filter(|&id| id != mine.SessionId) {
                match info(id) {
                    Ok(s) if s.UserName == mine.UserName && s.Domain == mine.Domain => {
                        starts.push(u64::try_from(s.LogonTime).ok()?);
                    }
                    Ok(_) => {}
                    // Documented: only another user's session needs a permission for this
                    Err(e) if e.code() == E_ACCESSDENIED => {}
                    Err(_) => return None,
                }
            }
            Some(starts)
        }
    }

    fn info(session: u32) -> Result<WTSINFOW> {
        unsafe {
            let mut buf = PWSTR::null();
            let mut bytes = 0u32;
            WTSQuerySessionInformationW(None, session, WTSSessionInfo, &mut buf, &mut bytes)?;
            let whole = !buf.is_null() && bytes as usize >= std::mem::size_of::<WTSINFOW>();
            let info = whole.then(|| (buf.0 as *const WTSINFOW).read_unaligned());
            WTSFreeMemory(buf.0 as *mut _);
            info.ok_or_else(windows::core::Error::empty)
        }
    }

    /// This process's user: (domain, name).
    fn account() -> Option<(Vec<u16>, Vec<u16>)> {
        // A TOKEN_USER and its SID fit in 256 bytes; u64 for its alignment
        let mut token = [0u64; 32];
        let (mut name, mut domain) = ([0u16; 256], [0u16; 256]);
        let (mut name_len, mut domain_len) = (name.len() as u32, domain.len() as u32);
        let (mut got, mut kind) = (0u32, SID_NAME_USE::default());
        unsafe {
            GetTokenInformation(
                own_token(),
                TokenUser,
                Some(token.as_mut_ptr().cast()),
                std::mem::size_of_val(&token) as u32,
                &mut got,
            )
            .ok()?;
            LookupAccountSidW(
                None,
                (*token.as_ptr().cast::<TOKEN_USER>()).User.Sid,
                Some(PWSTR(name.as_mut_ptr())),
                &mut name_len,
                Some(PWSTR(domain.as_mut_ptr())),
                &mut domain_len,
                &mut kind,
            )
            .ok()?;
        }
        Some((
            domain.get(..domain_len as usize)?.to_vec(),
            name.get(..name_len as usize)?.to_vec(),
        ))
    }

    /// Whether the fixed-size, NUL-terminated `field` holds `full` (cut to fit).
    pub(super) fn same_name(full: &[u16], field: &[u16]) -> bool {
        let cut = &full[..full.len().min(field.len() - 1)];
        let end = field.iter().position(|&c| c == 0).unwrap_or(field.len());
        let text = |s: &[u16]| String::from_utf16_lossy(s).to_lowercase();
        text(cut) == text(&field[..end])
    }

    /// When logon session `id` was created. Its owner may ask.
    fn logon_began(id: u64) -> Option<u64> {
        let luid = LUID {
            LowPart: id as u32,
            HighPart: (id >> 32) as i32,
        };
        let mut data: *mut SECURITY_LOGON_SESSION_DATA = std::ptr::null_mut();
        unsafe {
            LsaGetLogonSessionData(&luid, &mut data).ok().ok()?;
            if data.is_null() {
                return None;
            }
            let began = (*data).LogonTime;
            let _ = LsaFreeReturnBuffer(data.cast());
            u64::try_from(began).ok().filter(|&t| t != 0)
        }
    }

    /// The other half of a UAC-split sign-in, if there is one.
    fn linked_luid() -> Option<u64> {
        let mut linked = TOKEN_LINKED_TOKEN::default();
        let mut got = 0u32;
        unsafe {
            GetTokenInformation(
                own_token(),
                TokenLinkedToken,
                Some((&raw mut linked).cast()),
                std::mem::size_of::<TOKEN_LINKED_TOKEN>() as u32,
                &mut got,
            )
            .ok()?;
            let id = luid(linked.LinkedToken);
            let _ = CloseHandle(linked.LinkedToken);
            id
        }
    }

    /// A token's logon LUID (what older versions tagged files with).
    fn luid(token: HANDLE) -> Option<u64> {
        let mut stats = TOKEN_STATISTICS::default();
        let mut got = 0u32;
        unsafe {
            GetTokenInformation(
                token,
                TokenStatistics,
                Some((&raw mut stats).cast()),
                std::mem::size_of::<TOKEN_STATISTICS>() as u32,
                &mut got,
            )
            .ok()?;
        }
        let id = stats.AuthenticationId;
        Some((u64::from(id.HighPart as u32) << 32) | u64::from(id.LowPart))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ClipboardCapacity;

    const S1: SessionId = SessionId {
        logon: 7,
        stamp: 133_000_000_000_000_000,
    };
    const DAY: u64 = 864_000_000_000;
    /// A logon session begins this long before its sign-in time (0.3 s).
    const LOGON_LEAD: u64 = 3_000_000;
    /// The two logon LUIDs UAC gives one sign-in (normal, elevated).
    const NORMAL: u64 = 1_209_401_913;
    const ELEVATED: u64 = 1_209_401_815;

    fn me() -> SignIn {
        SignIn {
            id: S1,
            logons: [NORMAL, ELEVATED],
            others: Some(Vec::new()),
            logon_began: Some(S1.stamp - LOGON_LEAD),
        }
    }

    fn next_sign_in() -> SignIn {
        SignIn {
            id: SessionId {
                stamp: S1.stamp + DAY,
                ..S1
            },
            logons: [NORMAL + 500, ELEVATED + 500],
            others: Some(Vec::new()),
            logon_began: Some(S1.stamp + DAY - LOGON_LEAD),
        }
    }

    fn store(dir: &TempDir, me: &SignIn) -> ClipboardStore {
        ClipboardStore::at(Some(dir.0.clone()), Some(me.clone()))
    }

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
            let p = crate::data_dir::temp_dir()
                .join(format!("oeca-store-test-{}-{name}", std::process::id()));
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
    fn test_ownership_needs_proof() {
        let id = |logon, stamp| SessionId { logon, stamp };
        let (start, me) = (S1.stamp, me());
        // This sign-in
        assert_eq!(me.owner(S1), Owner::Mine);
        // An older version's file from this sign-in, saved by either half
        assert_eq!(me.owner(id(NORMAL, start + 9)), Owner::Mine);
        assert_eq!(me.owner(id(ELEVATED, start + 9)), Owner::Mine);
        // An earlier sign-in: the same session number after a restart, an
        // older version's file, a LUID that repeats across boots
        for logon in [S1.logon, 978_130_866, NORMAL] {
            assert_eq!(me.owner(id(logon, start - DAY)), Owner::Disposable);
        }
        // No proof: from "later", another session at the same moment, no time
        for saved in [
            id(99, start + 1),
            id(99, start),
            id(99, 0),
            id(0, 0),
            id(99, 5),
        ] {
            assert_eq!(me.owner(saved), Owner::Unknown, "{saved:?}");
        }
        // An earlier sign-in of this user that still exists may own it
        let with = |others| SignIn {
            others,
            ..me.clone()
        };
        let saved = id(3, start - DAY);
        assert_eq!(with(Some(vec![start - DAY])).owner(saved), Owner::Unknown);
        assert_eq!(
            with(Some(vec![start - 2 * DAY])).owner(saved),
            Owner::Unknown
        );
        assert_eq!(with(Some(vec![start - 5])).owner(saved), Owner::Disposable);
        assert_eq!(with(None).owner(saved), Owner::Unknown, "sessions unknown");
    }

    #[test]
    fn test_close_and_reopen_in_the_same_session() {
        let dir = TempDir::new("reopen");
        let first = store(&dir, &me());
        assert_eq!(first.load(), Err(LoadError::Missing));
        let mut h = history(&[text("a"), image(4, 3, 50), text("b")]);
        first.save(&h).unwrap();
        // Quit and relaunch: same session, everything back in order
        let reopened = store(&dir, &me());
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
    fn test_normal_and_elevated_launches_share_one_history() {
        let dir = TempDir::new("elevated");
        let elevated = SignIn {
            logons: [ELEVATED, NORMAL],
            ..me()
        };
        store(&dir, &me())
            .save(&history(&[text("normal")]))
            .unwrap();
        let up = store(&dir, &elevated);
        assert_eq!(up.load().unwrap(), [text("normal")]);
        up.save(&history(&[text("elevated")])).unwrap();
        assert_eq!(store(&dir, &me()).load().unwrap(), [text("elevated")]);
        // An older version's file, tagged with either LUID, carries over
        for logon in [NORMAL, ELEVATED] {
            let old = SessionId {
                logon,
                stamp: S1.stamp + 60,
            };
            std::fs::write(
                dir.0.join(FILE_NAME),
                encode(old, &history(&[text("old version")])),
            )
            .unwrap();
            let s = store(&dir, &me());
            assert_eq!(s.load().unwrap(), [text("old version")]);
            s.save(&history(&[text("saved again")])).unwrap();
            assert_eq!(
                tag(&std::fs::read(dir.0.join(FILE_NAME)).unwrap()),
                Some(S1)
            );
        }
    }

    #[test]
    fn test_new_windows_session_never_sees_old_history() {
        let dir = TempDir::new("isolation");
        let path = dir.0.join(FILE_NAME);
        let secret = history(&[text("synthetic, from yesterday"), image(2, 2, 1)]);
        store(&dir, &me()).save(&secret).unwrap();
        // An interrupted write from then, too
        std::fs::write(dir.0.join(TEMP_NAME), &encode(S1, &secret)[..40]).unwrap();
        let unrelated = dir.0.join("someone-elses-file.txt");
        std::fs::write(&unrelated, "keep me").unwrap();
        std::fs::create_dir(dir.0.join("Recovery")).unwrap();
        std::fs::write(dir.0.join("Recovery/backup.dpapi"), "keep me").unwrap();
        let next = store(&dir, &next_sign_in());
        assert_eq!(next.load(), Err(LoadError::Missing));
        assert!(!path.exists(), "stale history deleted");
        assert!(
            !dir.0.join(TEMP_NAME).exists(),
            "stale partial write deleted"
        );
        assert!(unrelated.exists(), "unrelated files untouched");
        assert!(
            dir.0.join("Recovery/backup.dpapi").exists(),
            "backups untouched"
        );
        // Even if deleting failed, a stale file is refused by its session id
        std::fs::write(&path, encode(S1, &secret)).unwrap();
        assert_eq!(next.load(), Err(LoadError::OtherSession));
    }

    #[test]
    fn test_unknown_ownership_is_never_loaded_replaced_or_deleted() {
        let good = encode(S1, &history(&[text("synthetic")]));
        let mut flipped = good.clone();
        flipped[30] ^= 1;
        let mut no_identity = good[..good.len() - 8].to_vec();
        no_identity[8..24].fill(0);
        let sum = fnv1a(&no_identity);
        no_identity.extend_from_slice(&sum.to_le_bytes());
        let future = SessionId {
            logon: 99,
            stamp: S1.stamp + 2 * DAY,
        };
        let cases = [
            ("flipped", flipped),
            ("truncated", good[..good.len() - 3].to_vec()),
            ("header-only", good[..24].to_vec()),
            ("no-identity", no_identity),
            ("foreign", encode(future, &history(&[text("synthetic")]))),
            ("empty", Vec::new()),
            ("zeroed", vec![0; 64]),
            ("other-format", b"synthetic, some other format".to_vec()),
            ("too-large", vec![7; MAX_FILE_BYTES as usize + 1]),
        ];
        for (name, bytes) in cases {
            let dir = TempDir::new(name);
            let path = dir.0.join(FILE_NAME);
            std::fs::write(&path, &bytes).unwrap();
            for who in [me(), next_sign_in()] {
                let s = store(&dir, &who); // launch cleanup
                assert!(s.load().is_err(), "{name}: not loaded");
                assert!(
                    s.save(&history(&[text("new copy")])).is_err(),
                    "{name}: not replaced"
                );
                assert!(
                    s.save(&ClipboardHistory::default()).is_err(),
                    "{name}: not cleared"
                );
                assert!(s.wipe().is_err(), "{name}: not wiped at sign-out");
                assert!(std::fs::read(&path).unwrap() == bytes, "{name}: changed");
            }
        }
        // The same for an interrupted write whose tag is unreadable
        let dir = TempDir::new("unknown-temp");
        std::fs::write(dir.0.join(TEMP_NAME), "synthetic").unwrap();
        let s = store(&dir, &me());
        assert!(s.save(&history(&[text("new copy")])).is_err());
        assert!(s.wipe().is_err());
        assert_eq!(std::fs::read(dir.0.join(TEMP_NAME)).unwrap(), b"synthetic");
    }

    #[test]
    fn test_another_live_sign_in_keeps_its_history() {
        let dir = TempDir::new("concurrent");
        let h = history(&[text("synthetic")]);
        store(&dir, &me()).save(&h).unwrap();
        let second = SignIn {
            others: Some(vec![S1.stamp]),
            ..next_sign_in()
        };
        let unknown_sessions = SignIn {
            others: None,
            ..next_sign_in()
        };
        for who in [second, unknown_sessions] {
            let s = store(&dir, &who);
            assert_eq!(s.load(), Err(LoadError::OtherSession));
            assert!(s.save(&history(&[text("second's copy")])).is_err());
            assert!(s.wipe().is_err());
        }
        assert_eq!(store(&dir, &me()).load().unwrap(), items(&h));
    }

    #[test]
    fn test_a_file_changed_behind_our_back_is_not_overwritten() {
        let dir = TempDir::new("changed");
        let s = store(&dir, &me());
        s.save(&history(&[text("mine")])).unwrap();
        let path = dir.0.join(FILE_NAME);
        std::fs::write(&path, "synthetic, written by something else").unwrap();
        assert!(s.save(&history(&[text("mine again")])).is_err());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"synthetic, written by something else"
        );
    }

    #[test]
    fn test_our_interrupted_writes_do_not_block_saving() {
        let dir = TempDir::new("interrupted");
        let h = history(&[text("synthetic")]);
        for partial in [encode(S1, &h)[..24].to_vec(), Vec::new(), vec![0; 100]] {
            std::fs::write(dir.0.join(TEMP_NAME), &partial).unwrap();
            let s = store(&dir, &me());
            s.save(&h).unwrap();
            assert_eq!(s.load().unwrap(), items(&h));
        }
    }

    #[test]
    fn test_no_identity_or_no_lock_changes_nothing() {
        let dir = TempDir::new("off");
        let path = dir.0.join(FILE_NAME);
        let bytes = encode(S1, &history(&[text("synthetic")]));
        std::fs::write(&path, &bytes).unwrap();
        // Identity unknown; and no folder, which is what a failed lock leaves
        let no_identity = ClipboardStore::at(Some(dir.0.clone()), None);
        let no_lock = ClipboardStore::at(crate::data_dir::dir(), Some(next_sign_in()));
        for off in [no_identity, no_lock] {
            assert_eq!(off.load(), Err(LoadError::Missing));
            off.save(&history(&[text("x")])).unwrap();
            off.save(&ClipboardHistory::default()).unwrap();
            off.wipe().unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn test_session_end_removes_only_this_sign_ins_history() {
        let dir = TempDir::new("session-end");
        let path = dir.0.join(FILE_NAME);
        store(&dir, &me()).save(&history(&[text("mine")])).unwrap();
        ClipboardStore::new(Some(dir.0.clone()), Some(me()))
            .wipe()
            .unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_a_sign_in_time_moved_forward_deletes_nothing() {
        let dir = TempDir::new("moved-forward");
        store(&dir, &me())
            .save(&history(&[text("synthetic")]))
            .unwrap();
        let path = dir.0.join(FILE_NAME);
        let partial = &encode(S1, &history(&[text("interrupted")]))[..40];
        std::fs::write(dir.0.join(TEMP_NAME), partial).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        // As if an unlock, resume or reconnect moved it, by a tick or by hours;
        // for the normal half and the elevated one
        let elevated = SignIn {
            logons: [ELEVATED, NORMAL],
            ..me()
        };
        for half in [me(), elevated] {
            for later in [1, LOGON_LEAD, DAY / 8, 3 * DAY] {
                let mut moved = half.clone();
                moved.id.stamp += later;
                assert_eq!(moved.owner(S1), Owner::Unknown, "+{later}");
                let s = store(&dir, &moved); // launch cleanup
                assert_eq!(s.load(), Err(LoadError::OtherSession));
                assert!(s.save(&history(&[text("new copy")])).is_err());
                assert!(s.save(&ClipboardHistory::default()).is_err());
                assert!(s.wipe().is_err(), "nor at sign-out");
                assert_eq!(std::fs::read(&path).unwrap(), bytes, "+{later}");
                assert!(dir.0.join(TEMP_NAME).exists(), "nor the partial write");
            }
        }
        // The next real sign-in clears both, as always
        let _ = store(&dir, &next_sign_in());
        assert!(!path.exists() && !dir.0.join(TEMP_NAME).exists());
    }

    #[test]
    fn test_deleting_needs_both_times_to_agree() {
        let next = next_sign_in();
        let (start, began) = (next.id.stamp, next.logon_began.unwrap());
        let owner = |logon_began, stamp| {
            SignIn {
                logon_began,
                ..next.clone()
            }
            .owner(SessionId { logon: 3, stamp })
        };
        assert_eq!(owner(Some(began), began - 1), Owner::Disposable);
        assert_eq!(owner(Some(began), began), Owner::Unknown, "not before it");
        assert_eq!(owner(Some(began), start - 1), Owner::Unknown, "in between");
        // A logon made later inside the session proves nothing more
        assert_eq!(owner(Some(start + DAY), start + 1), Owner::Unknown);
        assert_eq!(owner(None, start - DAY), Owner::Unknown, "time unknown");
    }

    #[test]
    fn test_an_unknown_logon_time_never_stops_this_sign_ins_own_history() {
        let dir = TempDir::new("no-logon-time");
        let blind = |who: SignIn| SignIn {
            logon_began: None,
            ..who
        };
        let h = history(&[text("synthetic")]);
        let s = store(&dir, &blind(me()));
        s.save(&h).unwrap();
        assert_eq!(store(&dir, &blind(me())).load().unwrap(), items(&h));
        // A later sign-in that cannot learn it neither shows nor deletes it
        let later = store(&dir, &blind(next_sign_in()));
        assert_eq!(later.load(), Err(LoadError::OtherSession));
        assert!(dir.0.join(FILE_NAME).exists());
    }

    #[test]
    fn test_a_sign_in_time_moved_back_deletes_nothing() {
        // A clock set back, or the time reported earlier than it was
        let dir = TempDir::new("moved-back");
        store(&dir, &me())
            .save(&history(&[text("synthetic")]))
            .unwrap();
        let mut moved = me();
        moved.id.stamp -= 1;
        let s = store(&dir, &moved);
        assert_eq!(s.load(), Err(LoadError::OtherSession));
        assert!(s.wipe().is_err());
        assert!(dir.0.join(FILE_NAME).exists());
    }

    #[test]
    fn test_nightly_fast_startup_sign_ins_each_drop_only_the_one_before() {
        // Fast Startup signs out without rebooting: the session number grows,
        // the kernel (and every old logon session) lives on
        let dir = TempDir::new("nights");
        let mut last: Option<SignIn> = None;
        for night in 1..=7u64 {
            let who = SignIn {
                id: SessionId {
                    logon: night,
                    stamp: S1.stamp + night * DAY,
                },
                logons: [night * 1000, night * 1000 + 1],
                others: Some(Vec::new()),
                logon_began: Some(S1.stamp + night * DAY - LOGON_LEAD),
            };
            let s = store(&dir, &who);
            assert_eq!(s.load(), Err(LoadError::Missing), "night {night}");
            s.save(&history(&[text("synthetic")])).unwrap();
            // Relaunches, sleep and wake inside the sign-in: the same answer
            for _ in 0..3 {
                assert_eq!(store(&dir, &who).load().unwrap().len(), 1);
            }
            if let Some(old) = last.replace(who) {
                assert_eq!(store(&dir, &old).load(), Err(LoadError::OtherSession));
            }
        }
    }

    #[test]
    fn test_links_planted_in_the_store_never_reach_their_targets() {
        let (dir, away) = (TempDir::new("linked"), TempDir::new("linked-target"));
        let bytes = encode(S1, &history(&[text("synthetic backup")]));
        let backup = away.0.join("backup.bin");
        std::fs::write(&backup, &bytes).unwrap();
        for name in [FILE_NAME, TEMP_NAME] {
            let link = dir.0.join(name);
            std::fs::hard_link(&backup, &link).unwrap();
            // Same sign-in (replace, clear) and a later one (launch cleanup)
            let s = store(&dir, &me());
            assert!(s.save(&history(&[text("new")])).is_err(), "{name}");
            assert!(s.save(&ClipboardHistory::default()).is_err(), "{name}");
            let _ = store(&dir, &next_sign_in());
            assert!(link.exists(), "{name}: not removed");
            assert_eq!(std::fs::read(&backup).unwrap(), bytes, "{name}");
            std::fs::remove_file(&link).unwrap();
        }
    }

    #[test]
    fn test_account_names_match_windows_fixed_fields() {
        let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        let field = |s: &str| {
            let mut f = [0u16; 21];
            for (to, from) in f.iter_mut().take(20).zip(s.encode_utf16()) {
                *to = from;
            }
            f
        };
        assert!(sign_in::same_name(&w("Alex"), &field("alex")));
        assert!(!sign_in::same_name(&w("Alex"), &field("Alexa")));
        assert!(!sign_in::same_name(&w("Alexa"), &field("Alex")));
        let long = "a_very_long_account_name_indeed";
        assert!(sign_in::same_name(&w(long), &field(long)));
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

    /// Windows' own answer on this machine (read-only; run explicitly:
    /// `cargo test clipboard_store_live -- --ignored`).
    #[test]
    #[ignore]
    fn clipboard_store_live_sign_in_is_stable() {
        let first = sign_in::current().expect("an interactive sign-in");
        assert_eq!(sign_in::current(), Some(first.clone()), "same on relaunch");
        assert!(first.id.stamp >= OLDEST_STAMP);
        assert!(first.others.is_some(), "sessions could be listed");
        let began = first.logon_began.expect("the logon session's time");
        assert!(
            began >= OLDEST_STAMP && began <= first.id.stamp,
            "{first:?}"
        );
        println!("{first:?}");
    }
}
