//! The trusted-device store. It needs no worker and no network: the device
//! list works while Nearby is off.
//!
//! - A record is one certificate fingerprint. The name is the one this user
//!   confirmed; what the device called itself then is kept apart, only to
//!   notice a change. No record is ever found, replaced or removed by a name.
//! - Forgetting keeps the record, as revoked. Revoked records are never
//!   dropped to make room: at capacity, new pins fail instead.
//! - Every write reports its result. Trust is added only once it is saved.
//!   It is withdrawn at once, saved or not.
//! - A revocation that cannot be saved pauses all trust and leaves a hold
//!   file beside the store, which keeps trust paused (and finishes the
//!   revocation) after a restart. `NotSaved { held: false }` means not even
//!   that could be written: it holds only until Oeca closes.
//! - A store that cannot be read trusts nobody and is never written over.
//!
//! One store per folder in a process (`for_oeca`); Oeca's writer lock keeps
//! other processes out.

use localsend::serde_json::{Value, from_str, json};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use tokio::sync::watch;

use super::identity;
use super::proof::{Authenticated, Fingerprint};

const FILE: &str = "nearby-trusted.bin";
const HOLD: &str = "nearby-trusted-hold.bin";

pub const MAX_TRUSTED: usize = 256;
/// Trusted and revoked together. Forgetting never adds a record, so it always fits.
pub const MAX_RECORDS: usize = 4096;

/// Per 10 minutes, saved, counted before anything the sender claims is looked at.
// ponytail: a peer that reconnects for new room ids can still use up the total
// (nothing unforgeable identifies it before authentication); blocks guessing, not denial
pub const MAX_ATTEMPTS: usize = 10;
pub const MAX_ATTEMPTS_PER_PEER: usize = 4;
const ATTEMPT_WINDOW: u64 = 10 * 60;
const MAX_NAME: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceState {
    Trusted,
    Revoked {
        at: u64,
    },
    /// Distrusted now, but not saved yet: never shown as done (`retry`).
    RevocationPending,
}

/// What the device list shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// The name this user confirmed.
    pub name: String,
    pub fingerprint: Fingerprint,
    pub short: String,
    /// Seconds since 1970.
    pub verified_at: u64,
    pub state: DeviceState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ready,
    /// No data folder: nobody is trusted and nothing can be.
    Unavailable,
    /// There but not readable: nobody is trusted, the file is left as it is.
    Unreadable,
    /// A revocation is not saved: all trust is paused until `retry` saves it
    /// (or `forget_all`).
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustError {
    Unavailable,
    Unreadable,
    Paused,
    NotFound,
    /// A name to show is required.
    InvalidName,
    /// Another trusted device has that name: two would be indistinguishable.
    NameInUse,
    /// `MAX_TRUSTED` or `MAX_RECORDS`.
    Full,
    /// `pin`: not trusted.
    Write(ErrorKind),
    /// `forget`: distrusted now, not saved. `held`: a restart keeps trust paused.
    NotSaved {
        held: bool,
        error: ErrorKind,
    },
}

/// What the store says about a key, with the name it came under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Standing {
    New,
    /// `conflict`: it goes by another name than the confirmed one, or by
    /// another trusted device's.
    Trusted {
        name: String,
        conflict: bool,
    },
    /// An unknown key under a known (trusted or revoked) device's name.
    Changed,
    Revoked,
    /// The store cannot answer: nobody is trusted.
    Unavailable,
}

#[derive(Clone, Debug)]
struct Record {
    fingerprint: Fingerprint,
    name: String,
    /// What it called itself when it was verified. Never an identity.
    alias: String,
    verified_at: u64,
    revoked_at: Option<u64>,
}

#[derive(Clone, Debug)]
struct Attempt {
    at: u64,
    peer: String,
    transfer: String,
}

#[derive(Clone, Debug, Default)]
struct Records {
    devices: Vec<Record>,
    attempts: Vec<Attempt>,
}

#[derive(Clone, Debug, Default)]
struct Pending {
    all: bool,
    keys: Vec<Fingerprint>,
    /// A hold was found that says nothing readable: only `forget_all` resolves it.
    unknown: bool,
}

impl Pending {
    fn is_empty(&self) -> bool {
        !self.all && !self.unknown && self.keys.is_empty()
    }

    fn revokes(&self) -> bool {
        self.all || !self.keys.is_empty()
    }

    fn covers(&self, key: &Fingerprint) -> bool {
        self.all || self.keys.contains(key)
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fault {
    /// A full disk: creating an empty file works, writing bytes does not.
    DiskFull,
    /// Nothing can be written.
    AllWrites,
}

struct Inner {
    dir: Option<PathBuf>,
    /// None: unreadable.
    records: Option<Records>,
    pending: Pending,
    #[cfg(test)]
    fault: Option<Fault>,
}

#[derive(Clone)]
pub struct Devices {
    inner: Arc<Mutex<Inner>>,
    changes: Arc<watch::Sender<u64>>,
}

impl Devices {
    /// Oeca's one store, in the claimed data folder.
    pub fn for_oeca() -> Self {
        static STORE: OnceLock<Devices> = OnceLock::new();
        match (STORE.get(), crate::data_dir::dir()) {
            (Some(store), _) => store.clone(),
            (None, Some(dir)) => STORE.get_or_init(|| Self::open(Some(dir))).clone(),
            // Before the folder is claimed: nobody is trusted, nothing is cached
            (None, None) => Self::open(None),
        }
    }

    /// Reads the store. No network, and nothing is written unless an
    /// interrupted revocation is waiting to be finished.
    pub fn open(dir: Option<PathBuf>) -> Self {
        let mut inner = Inner {
            dir,
            records: Some(Records::default()),
            pending: Pending::default(),
            #[cfg(test)]
            fault: None,
        };
        if let Some(dir) = inner.dir.clone() {
            inner.records = read_records(&dir.join(FILE));
            inner.pending = read_hold(&dir.join(HOLD));
            if inner.pending.revokes() {
                let _still_paused = inner.settle();
            }
        }
        Self {
            inner: Arc::new(Mutex::new(inner)),
            changes: Arc::new(watch::channel(0).0),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn changed(&self) {
        self.changes.send_modify(|n| *n += 1);
    }

    pub(super) fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub fn status(&self) -> Status {
        self.lock().status()
    }

    /// Trusted and revoked devices, trusted first. Works offline.
    pub fn list(&self) -> Result<Vec<Device>, TrustError> {
        let inner = self.lock();
        if inner.dir.is_none() {
            return Err(TrustError::Unavailable);
        }
        let records = inner.records.as_ref().ok_or(TrustError::Unreadable)?;
        let mut list: Vec<Device> = records
            .devices
            .iter()
            .map(|r| Device {
                name: r.name.clone(),
                short: r.fingerprint.short(),
                fingerprint: r.fingerprint.clone(),
                verified_at: r.verified_at,
                state: if inner.pending.covers(&r.fingerprint) {
                    DeviceState::RevocationPending
                } else {
                    r.revoked_at
                        .map_or(DeviceState::Trusted, |at| DeviceState::Revoked { at })
                },
            })
            .collect();
        list.sort_by_key(|d| (d.state != DeviceState::Trusted, d.name.to_lowercase()));
        Ok(list)
    }

    /// Revokes one device. It is distrusted before anything is written:
    /// whatever this returns, it is not trusted afterwards.
    pub fn forget(&self, key: &Fingerprint) -> Result<(), TrustError> {
        let mut inner = self.lock();
        if inner.dir.is_none() {
            return Err(TrustError::Unavailable);
        }
        // An unreadable store may hold it: that revocation is recorded all the same
        if let Some(r) = &inner.records
            && !r.devices.iter().any(|d| d.fingerprint == *key)
        {
            return Err(TrustError::NotFound);
        }
        if !inner.pending.keys.contains(key) {
            inner.pending.keys.push(key.clone());
        }
        let result = inner.settle();
        drop(inner);
        self.changed();
        result
    }

    /// Revokes every device. Also how a `Paused` store with an unknown hold is resolved.
    pub fn forget_all(&self) -> Result<(), TrustError> {
        let mut inner = self.lock();
        if inner.dir.is_none() {
            return Err(TrustError::Unavailable);
        }
        inner.pending.all = true;
        inner.pending.unknown = false;
        let result = inner.settle();
        drop(inner);
        self.changed();
        result
    }

    /// Reads an unreadable store again and saves what is waiting. Never
    /// restores trust that was withdrawn.
    pub fn retry(&self) -> Result<(), TrustError> {
        let mut inner = self.lock();
        let before = inner.status();
        let Some(dir) = inner.dir.clone() else {
            return Err(TrustError::Unavailable);
        };
        if inner.records.is_none() {
            inner.records = read_records(&dir.join(FILE));
        }
        let settled = if inner.pending.revokes() {
            inner.settle()
        } else {
            Ok(())
        };
        let result = settled.and_then(|()| inner.writable());
        let after = inner.status();
        drop(inner);
        if before != after {
            self.changed();
        }
        result
    }

    pub(super) fn standing(&self, key: &Fingerprint, live_alias: &str) -> Standing {
        self.lock().standing(key, live_alias)
    }

    /// Trusts an authenticated peer under the name the user confirmed.
    /// Trusted only if this returns Ok.
    pub(super) fn pin(
        &self,
        peer: &Authenticated,
        name: &str,
        live_alias: &str,
    ) -> Result<(), TrustError> {
        let mut inner = self.lock();
        inner.writable()?;
        let name = clean_name(name);
        if name.is_empty() {
            return Err(TrustError::InvalidName);
        }
        let mut next = inner.records.clone().unwrap_or_default();
        let key = peer.fingerprint();
        let trusted = |r: &&Record| r.revoked_at.is_none() && r.fingerprint != *key;
        if next
            .devices
            .iter()
            .filter(trusted)
            .any(|r| same_name(&r.name, &name))
        {
            return Err(TrustError::NameInUse);
        }
        if next.devices.iter().filter(trusted).count() >= MAX_TRUSTED {
            return Err(TrustError::Full);
        }
        let record = Record {
            fingerprint: key.clone(),
            name,
            alias: clean_name(live_alias),
            verified_at: now(),
            revoked_at: None,
        };
        let count = next.devices.len();
        match next.devices.iter_mut().find(|r| r.fingerprint == *key) {
            // Only ever its own record
            Some(own) => *own = record,
            None if count >= MAX_RECORDS => return Err(TrustError::Full),
            None => next.devices.push(record),
        }
        inner.save(&next).map_err(TrustError::Write)?;
        inner.records = Some(next);
        drop(inner);
        self.changed();
        Ok(())
    }

    /// Counts one handshake before it is answered. False: do not answer.
    /// An attempt that cannot be saved is not answered either.
    pub(super) fn begin_attempt(&self, peer: &str, transfer: &str) -> bool {
        self.begin_attempt_at(peer, transfer, now())
    }

    fn begin_attempt_at(&self, peer: &str, transfer: &str, at: u64) -> bool {
        let mut inner = self.lock();
        if inner.writable().is_err() {
            return false;
        }
        let mut next = inner.records.clone().unwrap_or_default();
        next.attempts.retain_mut(|a| {
            a.at = a.at.min(at); // a clock set back never shortens the window
            at - a.at < ATTEMPT_WINDOW
        });
        let by_peer = next.attempts.iter().filter(|a| a.peer == peer).count();
        if next.attempts.len() >= MAX_ATTEMPTS || by_peer >= MAX_ATTEMPTS_PER_PEER {
            return false;
        }
        next.attempts.push(Attempt {
            at,
            peer: peer.into(),
            transfer: transfer.into(),
        });
        if inner.save(&next).is_err() {
            return false;
        }
        inner.records = Some(next);
        true
    }

    /// Only after the user decided on an authenticated, trusted peer: a
    /// handshake abandoned after learning the code is never given back.
    pub(super) fn refund_attempt(&self, transfer: &str) {
        let mut inner = self.lock();
        let Some(mut next) = inner.records.clone() else {
            return;
        };
        let count = next.attempts.len();
        next.attempts.retain(|a| a.transfer != transfer);
        // If this cannot be saved the attempt stays counted: the safe direction
        if next.attempts.len() != count && inner.save(&next).is_ok() {
            inner.records = Some(next);
        }
    }

    #[cfg(test)]
    pub(super) fn fail_writes(&self, fault: Option<Fault>) {
        self.lock().fault = fault;
    }
}

impl Inner {
    fn status(&self) -> Status {
        if self.dir.is_none() {
            Status::Unavailable
        } else if !self.pending.is_empty() {
            Status::Paused
        } else if self.records.is_none() {
            Status::Unreadable
        } else {
            Status::Ready
        }
    }

    fn writable(&self) -> Result<(), TrustError> {
        match self.status() {
            Status::Ready => Ok(()),
            Status::Unavailable => Err(TrustError::Unavailable),
            Status::Unreadable => Err(TrustError::Unreadable),
            Status::Paused => Err(TrustError::Paused),
        }
    }

    fn standing(&self, key: &Fingerprint, live_alias: &str) -> Standing {
        let Some(records) = &self.records else {
            return Standing::Unavailable;
        };
        let known = records.devices.iter().find(|r| r.fingerprint == *key);
        if self.pending.covers(key) || known.is_some_and(|r| r.revoked_at.is_some()) {
            return Standing::Revoked;
        }
        if self.status() != Status::Ready {
            return Standing::Unavailable; // all trust is paused
        }
        let alias = clean_name(live_alias);
        let goes_by = |r: &Record| same_name(&r.name, &alias) || same_name(&r.alias, &alias);
        let mut others = records.devices.iter().filter(|r| r.fingerprint != *key);
        match known {
            Some(me) => Standing::Trusted {
                name: me.name.clone(),
                conflict: !goes_by(me) || others.any(|r| r.revoked_at.is_none() && goes_by(r)),
            },
            None if others.any(goes_by) => Standing::Changed,
            None => Standing::New,
        }
    }

    /// Saves every pending revocation, or leaves the hold that outlives a restart.
    fn settle(&mut self) -> Result<(), TrustError> {
        let Some(dir) = self.dir.clone() else {
            return Err(TrustError::Unavailable);
        };
        let error = match &mut self.records {
            Some(records) => {
                for r in &mut records.devices {
                    if self.pending.covers(&r.fingerprint) && r.revoked_at.is_none() {
                        r.revoked_at = Some(now());
                    }
                }
                let next = records.clone();
                match self.save(&next) {
                    Ok(()) => {
                        // A hold nobody can read keeps trust paused (`forget_all`)
                        let unknown = self.pending.unknown;
                        if !unknown {
                            // Left behind, it only repeats these revocations at the next start
                            let _ = crate::data_dir::remove(&dir.join(HOLD));
                        }
                        self.pending = Pending {
                            unknown,
                            ..Default::default()
                        };
                        return Ok(());
                    }
                    Err(e) => e,
                }
            }
            None => ErrorKind::InvalidData,
        };
        let held = self.write_hold(&dir.join(HOLD));
        Err(TrustError::NotSaved { held, error })
    }

    fn save(&self, records: &Records) -> Result<(), ErrorKind> {
        let dir = self.dir.as_ref().ok_or(ErrorKind::NotFound)?;
        self.write(&dir.join(FILE), &encode(records))
    }

    /// Says what is pending, or at worst (a full disk) only that something
    /// is. True only if what is on disk then covers all that is pending.
    fn write_hold(&self, path: &Path) -> bool {
        let keys: Vec<&str> = self.pending.keys.iter().map(Fingerprint::as_str).collect();
        let text = json!({ "all": self.pending.all, "keys": keys }).to_string();
        // One that cannot be read is left as it is
        let may_write = !self.pending.unknown || identity::absent(path);
        if may_write && (self.write(path, &text).is_ok() || self.write(path, "").is_ok()) {
            return true;
        }
        if std::fs::metadata(path).is_ok_and(|m| m.len() == 0) {
            return true;
        }
        let saved = read_hold(path);
        saved.all || (!self.pending.all && self.pending.keys.iter().all(|k| saved.keys.contains(k)))
    }

    /// Empty text is an empty file (nothing to seal).
    fn write(&self, path: &Path, text: &str) -> Result<(), ErrorKind> {
        #[cfg(test)]
        match self.fault {
            Some(Fault::AllWrites) => return Err(ErrorKind::PermissionDenied),
            Some(Fault::DiskFull) if !text.is_empty() => return Err(ErrorKind::StorageFull),
            _ => {}
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.kind())?;
        }
        if text.is_empty() {
            crate::data_dir::replace(path, &path.with_extension("tmp"), b"")
        } else {
            identity::seal_file(path, text.as_bytes())
        }
        .map_err(|e| e.kind())
    }
}

/// None: there, but unreadable or not a store this version knows.
fn read_records(path: &Path) -> Option<Records> {
    if identity::absent(path) {
        return Some(Records::default());
    }
    decode(&identity::unseal_file(path)?)
}

fn read_hold(path: &Path) -> Pending {
    if identity::absent(path) {
        return Pending::default();
    }
    let said = || -> Option<Pending> {
        let v: Value = from_str(&identity::unseal_file(path)?).ok()?;
        let pending = Pending {
            all: v.get("all")?.as_bool()?,
            keys: v
                .get("keys")?
                .as_array()?
                .iter()
                .map(|k| Fingerprint::parse(k.as_str()?))
                .collect::<Option<_>>()?,
            unknown: false,
        };
        (!pending.is_empty()).then_some(pending)
    };
    said().unwrap_or(Pending {
        unknown: true,
        ..Default::default()
    })
}

fn encode(records: &Records) -> String {
    let devices: Vec<Value> = records
        .devices
        .iter()
        .map(|r| {
            json!({
                "key": r.fingerprint.as_str(),
                "name": r.name,
                "alias": r.alias,
                "verified": r.verified_at,
                "revoked": r.revoked_at,
            })
        })
        .collect();
    let attempts: Vec<Value> = records
        .attempts
        .iter()
        .map(|a| json!({ "at": a.at, "peer": a.peer, "transfer": a.transfer }))
        .collect();
    json!({ "version": 1, "devices": devices, "attempts": attempts }).to_string()
}

fn decode(text: &str) -> Option<Records> {
    let v: Value = from_str(text).ok()?;
    if v.get("version")?.as_u64()? != 1 {
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    let devices = v
        .get("devices")?
        .as_array()?
        .iter()
        .map(|d| {
            let fingerprint = Fingerprint::parse(d.get("key")?.as_str()?)?;
            // A key twice could be both trusted and revoked: not a store
            seen.insert(fingerprint.clone()).then_some(())?;
            let revoked = d.get("revoked")?;
            Some(Record {
                fingerprint,
                name: d.get("name")?.as_str()?.into(),
                alias: d.get("alias")?.as_str()?.into(),
                verified_at: d.get("verified")?.as_u64()?,
                revoked_at: if revoked.is_null() {
                    None
                } else {
                    Some(revoked.as_u64()?)
                },
            })
        })
        .collect::<Option<_>>()?;
    let attempts = v
        .get("attempts")?
        .as_array()?
        .iter()
        .map(|a| {
            Some(Attempt {
                at: a.get("at")?.as_u64()?,
                peer: a.get("peer")?.as_str()?.into(),
                transfer: a.get("transfer")?.as_str()?.into(),
            })
        })
        .collect::<Option<_>>()?;
    Some(Records { devices, attempts })
}

/// A name fit to show and to compare: one line, no controls or invisible
/// direction marks, spaces collapsed.
/// Characters that show nothing, or reorder what is shown (bidi controls,
/// zero-width and filler characters, tags): never in a name or file name.
pub(super) fn invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}'
        | '\u{180B}'..='\u{180F}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}' | '\u{3164}' | '\u{FEFF}' | '\u{FFA0}' | '\u{FFF9}'..='\u{FFFB}'
        | '\u{1D173}'..='\u{1D17A}' | '\u{E0000}'..='\u{E007F}')
}

pub(super) fn clean_name(name: &str) -> String {
    let shown: String = name
        .chars()
        .take(MAX_NAME * 8) // a peer's alias can be as large as a message
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|&c| !c.is_control() && !invisible(c))
        .collect();
    let joined = shown
        .split(' ')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    joined
        .chars()
        .take(MAX_NAME)
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn same_name(a: &str, b: &str) -> bool {
    !a.is_empty() && a.to_lowercase() == b.to_lowercase()
}

fn now() -> u64 {
    std::time::SystemTime::UNIX_EPOCH
        .elapsed()
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::super::proof::tests::{authenticated as auth, fingerprint as key};
    use super::*;

    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Self {
            let p = crate::data_dir::temp_dir()
                .join(format!("oeca-devices-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn open(&self) -> Devices {
            Devices::open(Some(self.0.clone()))
        }
        /// Holds the store open as a scanner does: replacing it fails.
        fn lock_store(&self) -> std::fs::File {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(self.0.join(FILE))
                .unwrap()
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn trusted(name: &str, conflict: bool) -> Standing {
        Standing::Trusted {
            name: name.into(),
            conflict,
        }
    }

    #[test]
    fn the_confirmed_name_is_kept_apart_from_what_the_peer_calls_itself() {
        let d = Dir::new("names");
        let s = d.open();
        s.pin(&auth(1), "Bob's laptop", "DESKTOP-1234").unwrap();
        assert_eq!(
            s.standing(&key(1), "DESKTOP-1234"),
            trusted("Bob's laptop", false)
        );
        assert_eq!(
            s.standing(&key(1), "bob's  LAPTOP"),
            trusted("Bob's laptop", false)
        );
        assert_eq!(
            s.standing(&key(1), "Mum's phone"),
            trusted("Bob's laptop", true),
            "renamed: flagged, never renamed here"
        );
        assert_eq!(s.list().unwrap()[0].name, "Bob's laptop");
        assert_eq!(
            d.open().standing(&key(1), "x"),
            trusted("Bob's laptop", true)
        );
    }

    #[test]
    fn a_shared_name_never_replaces_or_hides_a_record() {
        let d = Dir::new("same-name");
        let s = d.open();
        s.pin(&auth(1), "Tablet", "Tablet").unwrap();
        assert_eq!(s.standing(&key(2), "Tablet"), Standing::Changed);
        assert_eq!(
            s.standing(&key(2), "tab\u{200B}let\u{202E}"),
            Standing::Changed
        );
        assert_eq!(
            s.pin(&auth(2), " tablet ", "Tablet"),
            Err(TrustError::NameInUse)
        );
        s.pin(&auth(2), "Other tablet", "Tablet").unwrap();
        for (k, name) in [(1, "Tablet"), (2, "Other tablet")] {
            assert_eq!(
                s.standing(&key(k), "Tablet"),
                trusted(name, true),
                "both kept; a shared live name is ambiguous"
            );
        }
        assert_eq!(
            s.pin(&auth(3), " \n\u{202E}", "x"),
            Err(TrustError::InvalidName)
        );
    }

    #[test]
    fn revoked_stays_revoked_and_its_name_stays_known() {
        let d = Dir::new("revoked");
        let s = d.open();
        s.pin(&auth(1), "Phone", "Pixel").unwrap();
        s.forget(&key(1)).unwrap();
        for store in [&s, &d.open()] {
            assert_eq!(store.standing(&key(1), "Pixel"), Standing::Revoked);
            assert_eq!(store.standing(&key(1), "Any new name"), Standing::Revoked);
            assert_eq!(
                store.standing(&key(2), "Pixel"),
                Standing::Changed,
                "same alias"
            );
            assert_eq!(
                store.standing(&key(2), "Phone"),
                Standing::Changed,
                "same name"
            );
        }
        assert_eq!(s.forget(&key(9)), Err(TrustError::NotFound));
        // Only an explicit verification brings the key back
        s.pin(&auth(1), "Phone", "Pixel").unwrap();
        assert_eq!(d.open().standing(&key(1), "Pixel"), trusted("Phone", false));
    }

    #[test]
    fn a_revocation_that_cannot_be_saved_survives_a_restart() {
        let d = Dir::new("locked");
        let s = d.open();
        s.pin(&auth(1), "Phone", "Phone").unwrap();
        s.pin(&auth(2), "Tablet", "Tablet").unwrap();
        let held = d.lock_store();
        let result = s.forget(&key(1));
        assert!(
            matches!(result, Err(TrustError::NotSaved { held: true, .. })),
            "{result:?}"
        );
        assert_eq!(s.standing(&key(1), "Phone"), Standing::Revoked);
        assert_eq!(
            s.standing(&key(2), "Tablet"),
            Standing::Unavailable,
            "all paused"
        );
        let states: Vec<_> = s.list().unwrap().into_iter().map(|x| x.state).collect();
        assert!(
            states.contains(&DeviceState::RevocationPending),
            "never shown as done"
        );
        assert!(
            !states
                .iter()
                .any(|x| matches!(x, DeviceState::Revoked { .. }))
        );
        // Restarted, still unwritable
        let restarted = d.open();
        assert_eq!(restarted.status(), Status::Paused);
        assert_eq!(restarted.standing(&key(1), "Phone"), Standing::Revoked);
        drop(held);
        // Restarted, writable: finished, and nobody else lost
        let restarted = d.open();
        assert_eq!(restarted.status(), Status::Ready);
        assert_eq!(restarted.standing(&key(1), "Phone"), Standing::Revoked);
        assert_eq!(
            restarted.standing(&key(2), "Tablet"),
            trusted("Tablet", false)
        );
        assert!(!d.0.join(HOLD).exists());
    }

    #[test]
    fn retry_finishes_a_revocation_without_restoring_anything() {
        let d = Dir::new("retry");
        let s = d.open();
        s.pin(&auth(1), "Phone", "Phone").unwrap();
        s.fail_writes(Some(Fault::AllWrites));
        assert!(matches!(
            s.forget(&key(1)),
            Err(TrustError::NotSaved { held: false, .. })
        ));
        assert_eq!(
            s.standing(&key(1), "Phone"),
            Standing::Revoked,
            "in this process"
        );
        assert!(s.retry().is_err());
        assert_eq!(s.status(), Status::Paused);
        s.fail_writes(None);
        s.retry().unwrap();
        assert_eq!(s.status(), Status::Ready);
        assert_eq!(d.open().standing(&key(1), "Phone"), Standing::Revoked);
    }

    #[test]
    fn a_full_disk_still_holds_trust_across_a_restart() {
        let d = Dir::new("full");
        let s = d.open();
        s.pin(&auth(1), "Phone", "Phone").unwrap();
        s.fail_writes(Some(Fault::DiskFull));
        assert_eq!(
            s.forget(&key(1)),
            Err(TrustError::NotSaved {
                held: true,
                error: ErrorKind::StorageFull
            })
        );
        // The empty hold cannot say who: nobody is trusted until Forget all
        let restarted = d.open();
        assert_eq!(restarted.status(), Status::Paused);
        assert_eq!(restarted.standing(&key(1), "Phone"), Standing::Unavailable);
        assert_eq!(restarted.retry(), Err(TrustError::Paused));
        assert_eq!(
            restarted.pin(&auth(2), "New", "New"),
            Err(TrustError::Paused)
        );
        restarted.forget_all().unwrap();
        assert_eq!(restarted.status(), Status::Ready);
        assert_eq!(d.open().standing(&key(1), "Phone"), Standing::Revoked);
    }

    #[test]
    fn a_pin_that_is_not_saved_is_not_trust() {
        let d = Dir::new("pin-fails");
        let s = d.open();
        s.fail_writes(Some(Fault::DiskFull));
        assert_eq!(
            s.pin(&auth(1), "Phone", "Phone"),
            Err(TrustError::Write(ErrorKind::StorageFull))
        );
        assert_eq!(s.standing(&key(1), "Phone"), Standing::New);
        assert!(s.list().unwrap().is_empty());
    }

    #[test]
    fn an_unreadable_store_trusts_nobody_and_is_left_as_it_is() {
        let d = Dir::new("corrupt");
        d.open().pin(&auth(1), "Phone", "Phone").unwrap();
        let good = std::fs::read(d.0.join(FILE)).unwrap();
        std::fs::write(d.0.join(FILE), b"not a store").unwrap();
        let s = d.open();
        assert_eq!(s.status(), Status::Unreadable);
        assert_eq!(s.standing(&key(1), "Phone"), Standing::Unavailable);
        assert_eq!(s.list(), Err(TrustError::Unreadable));
        assert_eq!(s.pin(&auth(2), "New", "New"), Err(TrustError::Unreadable));
        assert!(!s.begin_attempt("p", "t"));
        assert!(matches!(
            s.forget(&key(1)),
            Err(TrustError::NotSaved { held: true, .. })
        ));
        assert_eq!(
            std::fs::read(d.0.join(FILE)).unwrap(),
            b"not a store",
            "untouched"
        );
        // Readable again later: the revocation is applied before anyone is trusted
        std::fs::write(d.0.join(FILE), good).unwrap();
        s.retry().unwrap();
        assert_eq!(d.open().standing(&key(1), "Phone"), Standing::Revoked);
    }

    #[test]
    fn a_damaged_hold_keeps_trust_paused() {
        let d = Dir::new("bad-hold");
        d.open().pin(&auth(1), "Phone", "Phone").unwrap();
        std::fs::write(d.0.join(HOLD), b"garbage").unwrap();
        assert_eq!(d.open().status(), Status::Paused);
        assert_eq!(d.open().standing(&key(1), "Phone"), Standing::Unavailable);
        assert_eq!(std::fs::read(d.0.join(HOLD)).unwrap(), b"garbage");
    }

    #[test]
    fn an_older_hold_is_not_claimed_for_a_newer_revocation() {
        let d = Dir::new("stale-hold");
        let s = d.open();
        s.pin(&auth(1), "Phone", "Phone").unwrap();
        s.pin(&auth(2), "Tablet", "Tablet").unwrap();
        let held = d.lock_store();
        assert!(matches!(
            s.forget(&key(1)),
            Err(TrustError::NotSaved { held: true, .. })
        ));
        // Now nothing at all can be written: the hold on disk names only the first
        s.fail_writes(Some(Fault::AllWrites));
        assert!(matches!(
            s.forget(&key(2)),
            Err(TrustError::NotSaved { held: false, .. })
        ));
        assert!(matches!(
            s.forget(&key(1)),
            Err(TrustError::NotSaved { held: false, .. })
        ));
        assert_eq!(s.standing(&key(2), "Tablet"), Standing::Revoked);
        drop(held);
        s.fail_writes(None);
        s.retry().unwrap();
        assert_eq!(d.open().standing(&key(2), "Tablet"), Standing::Revoked);
    }

    #[test]
    fn a_revocation_is_saved_even_under_a_damaged_hold() {
        let d = Dir::new("bad-hold-forget");
        d.open().pin(&auth(1), "Phone", "Phone").unwrap();
        std::fs::write(d.0.join(HOLD), b"garbage").unwrap();
        d.open().forget(&key(1)).unwrap();
        let s = d.open();
        assert_eq!(s.status(), Status::Paused, "still says something unknown");
        assert_eq!(s.standing(&key(1), "Phone"), Standing::Revoked, "saved");
        assert_eq!(s.retry(), Err(TrustError::Paused));
    }

    #[test]
    fn the_device_list_works_offline() {
        let d = Dir::new("list");
        assert!(d.open().list().unwrap().is_empty());
        assert_eq!(
            std::fs::read_dir(&d.0).unwrap().count(),
            0,
            "listing writes nothing"
        );
        {
            let s = d.open();
            for (k, name) in [(1, "Zed"), (2, "Amy"), (3, "Mum")] {
                s.pin(&auth(k), name, "alias").unwrap();
            }
        }
        let s = d.open();
        s.forget(&key(3)).unwrap();
        let list = d.open().list().unwrap();
        let shown: Vec<_> = list.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(shown, ["Amy", "Zed", "Mum"]);
        assert!(matches!(list[2].state, DeviceState::Revoked { .. }));
        assert_eq!(list[0].short, key(2).short());
        assert!(list[0].verified_at > 1_700_000_000);
        s.forget_all().unwrap();
        let list = d.open().list().unwrap();
        assert_eq!(list.len(), 3, "kept as revoked");
        assert!(
            list.iter()
                .all(|x| matches!(x.state, DeviceState::Revoked { .. }))
        );
        assert_eq!(Devices::open(None).list(), Err(TrustError::Unavailable));
    }

    /// Writes the store directly: the API would take thousands of saves.
    fn seed(d: &Dir, trusted: usize, revoked: usize) {
        let devices = (0..trusted + revoked)
            .map(|i| Record {
                fingerprint: key(i + 1),
                name: format!("device {i}"),
                alias: format!("alias {i}"),
                verified_at: 1,
                revoked_at: (i < revoked).then_some(2),
            })
            .collect();
        let records = Records {
            devices,
            attempts: vec![],
        };
        identity::seal_file(&d.0.join(FILE), encode(&records).as_bytes()).unwrap();
    }

    #[test]
    fn the_256th_fits_and_the_257th_is_refused_not_evicting() {
        let d = Dir::new("256");
        seed(&d, MAX_TRUSTED - 1, 1);
        let s = d.open();
        s.pin(&auth(9001), "256th", "a").unwrap();
        assert_eq!(s.pin(&auth(9002), "257th", "b"), Err(TrustError::Full));
        let s = d.open();
        assert_eq!(
            s.standing(&key(1), "x"),
            Standing::Revoked,
            "the oldest record"
        );
        assert_eq!(s.list().unwrap().len(), MAX_TRUSTED + 1);
        assert_eq!(s.standing(&key(9001), "a"), trusted("256th", false));
        // Forgetting makes room, and keeps the record
        s.forget(&key(9001)).unwrap();
        s.pin(&auth(9002), "257th", "b").unwrap();
        assert_eq!(s.standing(&key(9001), "a"), Standing::Revoked);
    }

    #[test]
    fn revoked_records_are_never_evicted() {
        for revoked in [256usize, 257, MAX_RECORDS - 1] {
            let d = Dir::new(&format!("revoked-{revoked}"));
            seed(&d, 1, revoked);
            let s = d.open();
            let pinned = s.pin(&auth(99_999), "one more", "z");
            if revoked + 1 == MAX_RECORDS {
                assert_eq!(pinned, Err(TrustError::Full), "explicit");
            } else {
                pinned.unwrap();
            }
            s.forget(&key(revoked + 1)).unwrap(); // always possible
            let s = d.open();
            for k in 1..=revoked + 1 {
                assert_eq!(
                    s.standing(&key(k), "x"),
                    Standing::Revoked,
                    "{revoked}: {k}"
                );
            }
        }
    }

    #[test]
    fn attempts_are_limited_per_peer_and_in_all_and_survive_a_restart() {
        let d = Dir::new("attempts");
        let t = 2_000_000_000;
        for i in 0..MAX_ATTEMPTS_PER_PEER {
            // A fresh store each time: a restart or reconnect resets nothing
            assert!(d.open().begin_attempt_at("greedy", &i.to_string(), t));
        }
        let s = d.open();
        assert!(!s.begin_attempt_at("greedy", "more", t), "one peer's share");
        for i in 0..MAX_ATTEMPTS - MAX_ATTEMPTS_PER_PEER {
            assert!(s.begin_attempt_at(&format!("other {i}"), &format!("o{i}"), t));
        }
        assert!(!d.open().begin_attempt_at("yet another", "x", t), "total");
        assert!(
            !s.begin_attempt_at("clock", "c", t - 5000),
            "a clock set back"
        );
        assert!(
            s.begin_attempt_at("later", "l", t + ATTEMPT_WINDOW),
            "window over"
        );
    }

    #[test]
    fn only_a_decided_attempt_is_given_back() {
        let d = Dir::new("refund");
        let s = d.open();
        let t = 2_000_000_000;
        for i in 0..MAX_ATTEMPTS_PER_PEER * 3 {
            assert!(
                s.begin_attempt_at("friend", &i.to_string(), t),
                "reconnection {i}"
            );
            s.refund_attempt(&i.to_string());
        }
        s.refund_attempt("unknown");
        s.fail_writes(Some(Fault::AllWrites));
        assert!(
            !s.begin_attempt_at("friend", "unsaved", t),
            "uncountable: not answered"
        );
    }

    #[test]
    fn concurrent_updates_lose_nothing() {
        let d = Dir::new("concurrent");
        let s = d.open();
        std::thread::scope(|scope| {
            for k in 1..=16 {
                let s = s.clone();
                scope.spawn(move || {
                    s.pin(&auth(k), &format!("device {k}"), "a").unwrap();
                    if k % 2 == 0 {
                        s.forget(&key(k)).unwrap();
                    }
                });
            }
        });
        let list = d.open().list().unwrap();
        assert_eq!(list.len(), 16);
        assert_eq!(
            list.iter()
                .filter(|x| x.state == DeviceState::Trusted)
                .count(),
            8
        );
    }
}
