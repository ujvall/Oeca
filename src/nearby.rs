//! Nearby: the native peer, built on the official LocalSend Rust
//! core (`localsend` crate, pinned in Cargo.toml). The core does signaling,
//! WebRTC, the token exchange and the transfer protocol; this file only hosts
//! it and adds what the core leaves to the app:
//!
//! - one dedicated OS thread with a current-thread Tokio runtime, started by
//!   `Nearby::start` and stopped (joined) on drop; the UI thread never waits
//!   on the network: state comes back as a `View` snapshot plus one coalesced
//!   `notify` (Oeca: a PostMessage);
//! - reconnect with jittered exponential backoff, and longer waits when the
//!   server refuses the connection (its per-IP limits) or rate-limits (429:
//!   nothing shortens that one). A connected transfer does not need signaling
//!   and outlives its loss;
//! - file I/O on blocking threads: a slow disk or share stalls neither
//!   signaling nor quitting, which waits `SHUTDOWN_DEADLINE` at most;
//! - a deadline on every stage: signaling connect, SDP answer, ICE/DTLS/data
//!   channel + token exchange, each decision, transfer inactivity;
//! - a persistent device key (DPAPI-protected at rest);
//! - an explicit decision for every incoming transfer (never accepted
//!   automatically; no decision within `APPROVAL_TIMEOUT` declines), file
//!   count / size limits, sanitized names, streaming to a temporary file that
//!   only becomes the real file once complete (never over an existing file);
//! - decisions (`transfer`): each names its transfer's id and is checked and
//!   applied on the caller's thread, so none can reach another transfer;
//! - identity (`proof`): claimed, connected and authenticated are different
//!   types. A peer is who its DTLS certificate says, once it also proved, on
//!   that connection, that it is the peer selected here. LocalSend's token
//!   exchange is not bound to the connection and is not relied on;
//! - trust (`devices`): a saved record of an authenticated certificate under a
//!   name the user confirmed. A peer without the extension is never trusted:
//!   an unknown one gets explicit one-off transfers, a known one is refused.
//!
//! Received files are never opened or run. Everything a peer sends (alias,
//! names, sizes, SDP) is untrusted.
// There is no Nearby UI yet: most of the view and the decisions are used by the
// tests only until the notch UI (a later stage) uses them.
#![cfg_attr(not(test), allow(dead_code, unused_imports))]
use localsend::crypto::token;
use localsend::model::discovery::DeviceType;
use localsend::model::transfer::FileDto;
use localsend::util::{base64, filename};
use localsend::webrtc::signaling::{
    ClientInfo, ClientInfoWithoutId, ManagedSignalingConnection, SignalingConnection,
    WsClientMessage, WsServerMessage, WsServerSdpMessage,
};
use localsend::webrtc::webrtc::{self as rtc, RTCFile, RTCSendFileResponse, RTCStatus};
use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::{Semaphore, mpsc, oneshot, watch};
use tokio::time::Instant;
use webrtc::peer_connection::certificate::RTCCertificate;

mod devices;
mod proof;
mod transfer;

// The API's types, for the UI
#[allow(unused_imports)]
pub use devices::{Device, DeviceState, Devices, Status, TrustError};
#[allow(unused_imports)]
pub use proof::{Authenticated, ClaimedFingerprint, Connected, Fingerprint};
pub use transfer::{
    ARMING, Capability, Claimed, Connection, Direction, Failure, FileRow, Identity, Refused, Stage,
    Transfer, TransferId,
};
use transfer::{Board, Gate, Proof};

pub const SIGNALING_URL: &str = "wss://public.localsend.org/v1/ws";
/// LocalSend's own STUN server (the official app's default). No TURN: when a
/// direct connection is impossible the transfer fails; nothing is relayed.
pub const STUN: &str = "stun:stun.localsend.org:3478";
const PROTOCOL_VERSION: &str = "2.3";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);
const RTC_TIMEOUT: Duration = Duration::from_secs(30);
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// After the core reports the end, how long its connection close may take.
const CLOSE_GRACE: Duration = Duration::from_secs(3);
const BUSY_DECLINE_TIMEOUT: Duration = Duration::from_secs(30);
const TOKEN_REFRESH: Duration = Duration::from_secs(30 * 60);
/// After a network change, how long to let interfaces settle before reconnecting.
const NETWORK_SETTLE: Duration = Duration::from_secs(2);

const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(300);
/// Refused before HELLO: the server's per-IP connection or hourly limit.
const REJECTED_MIN: Duration = Duration::from_secs(60);
/// 429 from the server: its request counter is per IP and per hour.
const RATE_LIMITED_WAIT: Duration = Duration::from_secs(15 * 60);
/// A session that lasted this long resets the backoff.
const STABLE: Duration = Duration::from_secs(60);
/// Between two connection attempts, whatever asks for the next one.
const MIN_RECONNECT_GAP: Duration = Duration::from_secs(10);
/// The other user compares the code, then decides (the browser waits as long).
const PEER_DECIDES: Duration = Duration::from_secs(250);
/// A big file being flushed, or the receiver's confirmations of what was sent.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(120);
/// Files from this size may take `CONFIRM_TIMEOUT` to flush.
const SLOW_FLUSH_BYTES: u64 = 1 << 20;
/// Looking at the files to send (a share or cloud folder can stall).
const PREPARE_TIMEOUT: Duration = Duration::from_secs(30);
/// At shutdown: time for the close messages to leave.
const SHUTDOWN_FLUSH: Duration = Duration::from_millis(250);
/// At shutdown: for blocking I/O to notice it and remove its partial file.
const SHUTDOWN_IO: Duration = Duration::from_secs(1);
/// How long dropping `Nearby` may take, on the caller's (UI) thread.
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(2);

pub const MAX_FILES: usize = 1000;
// ponytail: fixed cap, no free-space check; add GetDiskFreeSpaceExW when the UI shows it.
pub const MAX_TOTAL_BYTES: u64 = 256 << 30;
const READ_CHUNK: usize = 64 * 1024;
const MAX_SDP_BYTES: u64 = 64 * 1024;
/// An SDP as signaled (compressed, base64): more cannot inflate to a valid one.
const MAX_SIGNALED_BYTES: usize = 128 * 1024;
/// Echoed into our answer, so a peer does not choose how much we send.
const MAX_SESSION_ID: usize = 128;
/// Of a file list, per file (Windows' longest name is 765 bytes of UTF-8).
const MAX_NAME_BYTES: usize = 1024;
const MAX_ID_BYTES: usize = 128;
/// `IDLE_TIMEOUT` restarts only after this much arrived: a drip is not progress.
const MIN_PROGRESS: u64 = 64 * 1024;
/// A finished file counts as this much, so many tiny files are not a drip.
/// Finished: received and saved, or sent and confirmed by the receiver.
/// Only for the idle deadline: completion is each file's `ok`.
const FILE_PROGRESS: u64 = 4096;
/// Busy: connections made only to decline, at once.
const MAX_DECLINES: usize = 2;
/// Busy: answers per window. Each one costs the room's shared signaling quota.
const MAX_REFUSALS: usize = 30;
const MAX_REFUSALS_PER_PEER: usize = 5;
const REFUSAL_WINDOW: Duration = Duration::from_secs(10 * 60);

pub struct Config {
    pub signaling_url: String,
    pub stun: Vec<String>,
    pub alias: String,
    /// Where the identity (signing key, DTLS certificate) lives (None: a new
    /// identity every start).
    pub data_dir: Option<PathBuf>,
    /// The folder's one device store: the device list's, whether Nearby runs or not.
    pub devices: Devices,
    pub receive_dir: PathBuf,
}

impl Config {
    /// `%LOCALAPPDATA%\Oeca\nearby-*.bin`, received files in
    /// `Downloads\Oeca Hop`. None in test builds: they never use the public
    /// signaling server.
    pub fn for_oeca() -> Option<Self> {
        if crate::data_dir::ISOLATED {
            return None;
        }
        Some(Self {
            signaling_url: SIGNALING_URL.into(),
            stun: vec![STUN.into()],
            alias: DEFAULT_ALIAS.into(),
            data_dir: crate::data_dir::dir(),
            devices: Devices::for_oeca(),
            receive_dir: downloads_dir()?.join(RECEIVE_FOLDER),
        })
    }
}

/// What every room peer sees before any consent: nothing about this computer
/// (not its Windows name). Devices tell Oeca PCs apart by their verified
/// identity and the name the user gave them.
pub const DEFAULT_ALIAS: &str = "Oeca";
/// Received files go into this folder in Downloads, never Downloads itself.
pub const RECEIVE_FOLDER: &str = "Oeca Hop";

/// What to show before Nearby is first turned on (the Nearby UI shows it).
/// Built from the same constants the code uses, so it cannot drift from them.
pub fn privacy_notice() -> String {
    let host = |url: &str| {
        url.split("://")
            .nth(1)
            .unwrap_or(url)
            .split(['/', ':'])
            .next()
            .unwrap_or("")
            .to_string()
    };
    let stun = STUN
        .trim_start_matches("stun:")
        .split(':')
        .next()
        .unwrap_or("");
    format!(
        "Oeca Hop sends files between this PC and your phone or other devices.\n\
         \n\
         - While Hop is on and Oeca is open, this PC connects to LocalSend's public \
         signaling server ({signaling}) and STUN server ({stun}), run by the LocalSend \
         project, not by Oeca. Both see this PC's public IP address. The signaling server \
         also sees the name \"{alias}\" and the device type, and it passes on every \
         connection request, which carries this PC's Hop identity (the same until you \
         reset it) and its network addresses.\n\
         - The signaling server lists this PC to every device that shares its public IPv4 \
         address or IPv6 /64 network: usually your own network, but on public Wi-Fi, a \
         shared VPN or a mobile network that can include strangers. Any of them can contact this PC, and learns, \
         before you approve anything, that it is online, its Hop identity and its network \
         addresses: the public one, and its local ones on Ethernet or Wi-Fi (not those of \
         virtual adapters or most VPNs).\n\
         - A VPN, and whether your devices connect over IPv4 or IPv6, decide which devices \
         share your public address, and so which devices find each other. Hop doesn't \
         promise that only devices on your Wi-Fi can see this PC.\n\
         - Files go directly between the two devices, encrypted, never through those servers; \
         nothing is stored online. \
         Nothing is received without your approval; received files go to Downloads\\{folder} \
         and are marked as downloaded from the internet. Nothing is ever opened by Hop.\n\
         - Hop is off until you turn it on, and stops when Oeca closes. Resetting this PC's \
         Hop identity makes your devices ask you to verify it again.",
        signaling = host(SIGNALING_URL),
        alias = DEFAULT_ALIAS,
        folder = RECEIVE_FOLDER,
    )
}

/// A peer's string as it may be shown (no control, invisible or bidi
/// characters, whitespace collapsed, at most 64 characters): the same rule as
/// peer aliases and device names.
pub fn display_name(untrusted: &str) -> String {
    devices::clean_name(untrusted)
}

#[cfg(test)]
pub fn test_transfer_id() -> TransferId {
    TransferId::new()
}

#[cfg(test)]
pub fn test_fingerprint(seed: usize) -> Fingerprint {
    proof::tests::fingerprint(seed)
}

/// The identity files, under the data folder.
const IDENTITY_FILES: [&str; 2] = ["nearby-identity.bin", "nearby-dtls.bin"];

/// A new identity from the next start: this PC's signing key and DTLS
/// certificate are deleted (only those two files), so devices that trusted
/// the old one see a new device and must verify it again; they cannot be told
/// (their trust is theirs). The device list here is kept: it is about the
/// other devices, whose identities did not change. Only while Nearby is
/// stopped (its worker holds the identity).
/// Both files are attempted; any failure is returned (the reset is then not
/// done, and trying again finishes it), never reported as done.
pub fn reset_identity(data_dir: &Path) -> std::io::Result<()> {
    let results = IDENTITY_FILES.map(|name| crate::data_dir::remove(&data_dir.join(name)));
    results.into_iter().collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retry {
    /// The connection dropped or could not be made.
    Lost,
    /// The server closed it before HELLO (its per-IP connection / request limits).
    Rejected,
    /// The server answered 429.
    RateLimited,
    /// `Nearby::reconnect`, or the network changed.
    Requested,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Link {
    #[default]
    Connecting,
    Online,
    Retrying {
        in_secs: u64,
        why: Retry,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
    pub id: String,
    /// Untrusted: whatever the peer calls itself. Never an identity.
    pub alias: String,
    pub model: Option<String>,
    pub kind: &'static str,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub link: Link,
    /// This device's id in the current signaling room.
    pub me: Option<String>,
    /// This device's DTLS fingerprint, shortened (stable when persistent).
    pub identity: String,
    pub persistent_identity: bool,
    pub peers: Vec<Peer>,
    pub transfer: Option<Transfer>,
}

/// What still goes through the worker. Decisions do not: nothing queued can
/// decide anything.
enum Command {
    /// Starts the transfer `send_files` made, if it is still that one.
    Send {
        id: TransferId,
        paths: Vec<PathBuf>,
    },
    Reconnect,
    NetworkChanged,
}

struct Shared {
    /// Lock order: board, then devices.
    board: Mutex<Board>,
    devices: Devices,
    /// Here, not in the session: a reconnect must not reset either.
    refusals: Mutex<Refusals>,
    declines: Arc<Semaphore>,
    /// A notification is outstanding (cleared by `snapshot`): bursts of
    /// updates (progress) cost one message until the UI looks.
    posted: AtomicBool,
    notify: Box<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn new(devices: Devices, notify: Box<dyn Fn() + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            board: Mutex::new(Board::default()),
            devices,
            refusals: Mutex::new(Refusals::default()),
            declines: Arc::new(Semaphore::new(MAX_DECLINES)),
            posted: AtomicBool::new(false),
            notify,
        })
    }

    fn with<T>(&self, f: impl FnOnce(&mut Board) -> T) -> T {
        let out = f(&mut lock(&self.board));
        if !self.posted.swap(true, Ordering::AcqRel) {
            (self.notify)();
        }
        out
    }

    fn update(&self, f: impl FnOnce(&mut View)) {
        self.with(|b| f(&mut b.view));
    }

    fn files(&self, id: &TransferId, f: impl FnOnce(&mut [FileRow], &mut Vec<PathBuf>)) {
        self.with(|b| b.edit_files(id, f));
    }

    /// Refusing a connection and dropping what it claimed are one step.
    fn admit(&self, id: &TransferId, proof: Proof) -> Result<Gate, Failure> {
        self.with(|b| {
            let admitted = b.admit(id, proof, &self.devices);
            if let Err(why) = &admitted {
                b.end(id, Stage::Failed(why.clone()));
            }
            admitted
        })
    }

    fn may_refuse(&self, peer: &str) -> bool {
        lock(&self.refusals).allow(peer, Instant::now())
    }

    fn still_allowed(&self, id: &TransferId) -> Result<(), Failure> {
        lock(&self.board).still_allowed(id, &self.devices)
    }
}

/// The answers given to peers that cannot be served now. Past the limit: silence.
#[derive(Default)]
struct Refusals(VecDeque<(Instant, String)>);

impl Refusals {
    fn allow(&mut self, peer: &str, now: Instant) -> bool {
        while self
            .0
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) >= REFUSAL_WINDOW)
        {
            self.0.pop_front();
        }
        let by_peer = self.0.iter().filter(|(_, p)| p == peer).count();
        if self.0.len() >= MAX_REFUSALS || by_peer >= MAX_REFUSALS_PER_PEER {
            return false;
        }
        self.0.push_back((now, peer.to_string()));
        true
    }
}

/// When progress restarts the idle deadline.
#[derive(Default)]
struct Pace(Option<u64>);

impl Pace {
    fn counts(&mut self, total: u64) -> bool {
        let counts = self
            .0
            .is_none_or(|mark| total.saturating_sub(mark) >= MIN_PROGRESS);
        if counts {
            self.0 = Some(total);
        }
        counts
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Nearby {
    cmd: Option<mpsc::Sender<Command>>,
    shared: Arc<Shared>,
    worker: Option<Worker>,
    network: Option<net::Watch>,
}

struct Worker {
    thread: std::thread::JoinHandle<()>,
    /// Ends whatever the worker is awaiting, a stalled connect included.
    stop: oneshot::Sender<()>,
    /// Disconnects when the worker is through.
    ended: std::sync::mpsc::Receiver<()>,
}

impl Nearby {
    /// Starts the worker thread. `notify` runs whenever the view changed
    /// since the last `snapshot`, on the worker or on the thread that called
    /// a decision; it must not block.
    pub fn start(
        config: Config,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        // Peers see host candidates only from real networks (net::offered)
        rtc::set_ip_filter(Some(Arc::new(net::may_offer)));
        let shared = Shared::new(config.devices.clone(), Box::new(notify));
        let (tx, rx) = mpsc::channel(16);
        let worker_shared = shared.clone();
        let (stop, stopped) = oneshot::channel();
        let (ended_tx, ended) = std::sync::mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("oeca-nearby".into())
            .spawn(move || {
                run_worker(config, worker_shared, rx, stopped);
                drop(ended_tx);
            })?;
        let network = net::Watch::start(tx.clone());
        Ok(Self {
            cmd: Some(tx),
            shared,
            worker: Some(Worker {
                thread,
                stop,
                ended,
            }),
            network,
        })
    }

    /// For showing: a question in it counts as displayed from now, and its
    /// answers from `Transfer::decide_after`.
    pub fn snapshot(&self) -> View {
        self.shared.posted.store(false, Ordering::Release);
        lock(&self.shared.board).snapshot(std::time::Instant::now())
    }

    /// The trusted-device list (`Devices::for_oeca` without Nearby).
    pub fn devices(&self) -> &Devices {
        &self.shared.devices
    }

    /// Offers `paths` to `peer`, exactly as the last snapshot showed it. It
    /// replaces an incoming connection that has not proven or asked anything.
    pub fn send_files(&self, peer: &Peer, paths: Vec<PathBuf>) -> Result<TransferId, Refused> {
        let cmd = self.cmd.as_ref().ok_or(Refused::Offline)?;
        self.shared.with(|b| {
            if b.view.link != Link::Online || b.view.me.is_none() {
                return Err(Refused::Offline);
            }
            if b.busy() && !b.unproven_incoming() {
                return Err(Refused::Busy);
            }
            if !b.view.peers.contains(peer) {
                return Err(Refused::PeerChanged);
            }
            let slot = cmd.try_reserve().map_err(|_| Refused::Offline)?;
            let id = b.begin(Direction::Send, peer.clone());
            slot.send(Command::Send {
                id: id.clone(),
                paths,
            });
            Ok(id)
        })
    }

    /// Stage::Verification: the codes match, and this is what the user calls
    /// the device. Saves the trust. (Codes differ: `respond(id, false)`.)
    pub fn verify(&self, id: &TransferId, name: &str) -> Result<(), Refused> {
        let s = &self.shared;
        s.with(|b| b.verify(id, name, &s.devices, std::time::Instant::now()))
    }

    /// Stage::NameCheck: it is the trusted device, for this transfer.
    pub fn confirm_name(&self, id: &TransferId) -> Result<(), Refused> {
        let s = &self.shared;
        s.with(|b| b.confirm_name(id, &s.devices, std::time::Instant::now()))
    }

    /// `true`: accept, at Stage::Approval and from a trusted device only.
    /// `false`: refuse whatever is being asked.
    pub fn respond(&self, id: &TransferId, accept: bool) -> Result<(), Refused> {
        let s = &self.shared;
        s.with(|b| b.respond(id, accept, &s.devices, std::time::Instant::now()))
    }

    /// An unknown stock LocalSend peer only: this one transfer goes ahead
    /// without verification (accept it, or send to it). Nothing is remembered.
    pub fn allow_unverified_once(&self, id: &TransferId) -> Result<(), Refused> {
        let s = &self.shared;
        s.with(|b| b.allow_once(id, &s.devices, std::time::Instant::now()))
    }

    pub fn cancel(&self, id: &TransferId) -> Result<(), Refused> {
        self.shared.with(|b| b.cancel(id))
    }

    /// Drops the signaling connection and connects again now (e.g. after resume).
    pub fn reconnect(&self) -> bool {
        self.cmd
            .as_ref()
            .is_some_and(|tx| tx.try_send(Command::Reconnect).is_ok())
    }
}

impl Drop for Nearby {
    fn drop(&mut self) {
        // The network watch holds a sender too: stop it first (it waits for a
        // running callback).
        self.network = None;
        self.cmd = None;
        if let Some(Worker {
            thread,
            stop,
            ended,
        }) = self.worker.take()
        {
            let _ = stop.send(());
            // Past the deadline the thread is left to finish on its own: a stuck
            // disk must not hold the UI thread
            let waited = ended.recv_timeout(SHUTDOWN_DEADLINE);
            if waited != Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                let _ = thread.join();
            }
        }
    }
}

fn run_worker(
    config: Config,
    shared: Arc<Shared>,
    cmds: mpsc::Receiver<Command>,
    stop: oneshot::Receiver<()>,
) {
    // Before any transfer can start: no part file here is in use by this Oeca
    sweep_parts(&config.receive_dir, std::time::SystemTime::now());
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        shared.update(|v| {
            v.link = Link::Retrying {
                in_secs: 0,
                why: Retry::Lost,
            }
        });
        return;
    };
    rt.block_on(async {
        tokio::select! {
            _ = stop => {}
            _ = supervise(config, shared.clone(), cmds) => {}
        }
        shared.with(|b| b.end_session(Stage::Cancelled));
        tokio::time::sleep(SHUTDOWN_FLUSH).await;
    });
    rt.shutdown_timeout(SHUTDOWN_IO);
}

#[derive(Debug)]
struct Backoff {
    next: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self { next: BACKOFF_MIN }
    }
}

impl Backoff {
    fn next(&mut self, why: Retry) -> Duration {
        let wait = match why {
            Retry::Requested => return Duration::ZERO,
            Retry::RateLimited => RATE_LIMITED_WAIT,
            Retry::Rejected => self.next.max(REJECTED_MIN),
            Retry::Lost => self.next,
        };
        self.next = (wait * 2).clamp(BACKOFF_MIN, BACKOFF_MAX);
        wait
    }
}

/// Up to a quarter more, so many Oecas behind one IP do not retry in lockstep.
fn jittered(wait: Duration) -> Duration {
    let random = localsend::crypto::nonce::generate_nonce();
    wait + wait / 4 * u32::from(random.first().copied().unwrap_or(0)) / 256
}

/// May `Reconnect` (`asked`) or a network change shorten a wait made for `why`?
fn may_hurry(why: Retry, asked: bool) -> bool {
    match why {
        // The server said how long
        Retry::RateLimited => false,
        // Its per-IP limits: a flapping adapter is no reason to try again
        Retry::Rejected => asked,
        Retry::Lost | Retry::Requested => true,
    }
}

enum End {
    Shutdown,
    Retry(Retry),
}

/// A change in the device store ends what it no longer allows, whoever made it.
async fn follow_devices(shared: Arc<Shared>, mut changes: watch::Receiver<u64>) {
    while changes.changed().await.is_ok() {
        shared.with(|b| b.recheck(&shared.devices));
    }
}

async fn supervise(cfg: Config, shared: Arc<Shared>, mut cmds: mpsc::Receiver<Command>) {
    let dir = cfg.data_dir.as_deref();
    let (key, key_kept) =
        identity::load_or_create(dir.map(|d| d.join("nearby-identity.bin")).as_deref());
    let dtls = identity::dtls(dir.map(|d| d.join("nearby-dtls.bin")).as_deref());
    let cert_kept = dtls.as_ref().is_some_and(|(_, kept)| *kept);
    let cert = dtls.map(|(c, _)| c);
    let me = Me {
        fingerprint: cert.as_ref().and_then(cert_fingerprint),
        cert,
        key,
    };
    tokio::spawn(follow_devices(shared.clone(), shared.devices.changes()));
    shared.update(|v| {
        v.identity = me
            .fingerprint
            .as_ref()
            .map(Fingerprint::short)
            .unwrap_or_default();
        v.persistent_identity = key_kept && cert_kept;
    });
    let key = &me.key;
    let mut backoff = Backoff::default();
    loop {
        shared.update(|v| v.link = Link::Connecting);
        let started = Instant::now();
        let end = match connect(&cfg, key).await {
            Ok((sig, rx)) => session(&cfg, &shared, &me, sig, rx, &mut cmds).await,
            Err(why) => End::Retry(why),
        };
        let End::Retry(why) = end else { return };
        if started.elapsed() >= STABLE {
            backoff = Backoff::default();
        }
        let earliest = started + MIN_RECONNECT_GAP;
        let wait = match backoff.next(why) {
            Duration::ZERO => earliest.saturating_duration_since(Instant::now()),
            wait => jittered(wait),
        };
        let mut until = Instant::now() + wait;
        let retrying = |until: Instant| {
            shared.update(|v| {
                v.link = Link::Retrying {
                    in_secs: until.saturating_duration_since(Instant::now()).as_secs(),
                    why,
                }
            });
        };
        retrying(until);
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(until) => break,
                c = cmds.recv() => {
                    let asked = match c {
                        None => return,
                        Some(Command::Reconnect) => true,
                        Some(Command::NetworkChanged) => false,
                        // Of the room that is gone: that transfer already ended with it
                        Some(Command::Send { .. }) => continue,
                    };
                    if may_hurry(why, asked) {
                        until = until.min(earliest.max(Instant::now()));
                        retrying(until);
                    }
                }
            }
        }
    }
}

fn client_info(cfg: &Config, key: &token::SigningTokenKey) -> Option<ClientInfoWithoutId> {
    Some(ClientInfoWithoutId {
        alias: cfg.alias.clone(),
        version: PROTOCOL_VERSION.into(),
        device_model: Some("Windows".into()),
        device_type: Some(DeviceType::Desktop),
        token: token::generate_token_timestamp(key).ok()?,
    })
}

type Signaling = Arc<ManagedSignalingConnection>;

async fn connect(
    cfg: &Config,
    key: &token::SigningTokenKey,
) -> Result<(Signaling, mpsc::Receiver<WsServerMessage>), Retry> {
    let info = client_info(cfg, key).ok_or(Retry::Lost)?;
    let url = cfg.signaling_url.clone();
    // Its own task, so nothing the core does there (it used to panic on a
    // missing HELLO) can take the worker down.
    let mut task = tokio::spawn(async move {
        SignalingConnection::connect(url, &info)
            .await
            .map(|c| c.start_listener())
    });
    match tokio::time::timeout(CONNECT_TIMEOUT, &mut task).await {
        Ok(Ok(Ok((sig, rx)))) => Ok((Arc::new(sig), rx)),
        // Refused over the server's limits: closed before HELLO (vendor patch 3
        // makes that an error; it used to panic in the core)
        Ok(Ok(Err(e))) if e.to_string().contains("before HELLO") => Err(Retry::Rejected),
        Ok(Err(e)) if e.is_panic() => Err(Retry::Rejected),
        Ok(_) => Err(Retry::Lost),
        Err(_) => {
            task.abort();
            Err(Retry::Lost)
        }
    }
}

/// This device: its LocalSend signing key and its DTLS identity.
struct Me {
    key: token::SigningTokenKey,
    /// None only if generating one failed: then nothing can be authenticated,
    /// and nothing is sent.
    cert: Option<RTCCertificate>,
    fingerprint: Option<Fingerprint>,
}

/// One signaling session's surroundings.
struct Room<'a> {
    cfg: &'a Config,
    shared: &'a Arc<Shared>,
    me: &'a Me,
    sig: Signaling,
}

async fn session(
    cfg: &Config,
    shared: &Arc<Shared>,
    me: &Me,
    sig: Signaling,
    mut rx: mpsc::Receiver<WsServerMessage>,
    cmds: &mut mpsc::Receiver<Command>,
) -> End {
    let room = Room {
        cfg,
        shared,
        me,
        sig,
    };
    shared.update(|v| {
        v.link = Link::Online;
        v.me = Some(room.sig.client.id.to_string());
    });
    // A network change ends the session after a short settle, but not under a
    // running transfer (its own deadlines catch a dead one)
    let mut network_at: Option<Instant> = None;
    let mut peers: Vec<ClientInfo> = Vec::new();
    let publish = |peers: &[ClientInfo]| {
        let list = peers.iter().map(peer_view).collect();
        shared.update(|v| v.peers = list);
    };
    let mut refresh = tokio::time::interval_at(Instant::now() + TOKEN_REFRESH, TOKEN_REFRESH);
    let end = loop {
        tokio::select! {
            m = rx.recv() => match m {
                None => break End::Retry(Retry::Lost),
                Some(WsServerMessage::Hello { peers: list, .. }) => {
                    peers = list;
                    publish(&peers);
                }
                Some(WsServerMessage::Join { peer } | WsServerMessage::Update { peer }) => {
                    peers.retain(|p| p.id != peer.id);
                    peers.push(peer);
                    publish(&peers);
                }
                Some(WsServerMessage::Left { peer_id }) => {
                    peers.retain(|p| p.id != peer_id);
                    publish(&peers);
                }
                Some(WsServerMessage::Offer(offer)) => room.incoming(offer),
                // Any peer can send an ANSWER with any session id: only the
                // selected peer's reach the sending task, whose core sees no others
                Some(WsServerMessage::Answer(answer)) => {
                    let route = lock(&shared.board).answer_route(&answer.peer.id.to_string());
                    if let Some(to) = route {
                        let _ = to.try_send(answer);
                    }
                }
                Some(WsServerMessage::Error { code: 429 }) => {
                    break End::Retry(Retry::RateLimited)
                }
                Some(WsServerMessage::Error { .. }) => {}
            },
            c = cmds.recv() => match c {
                None => break End::Shutdown,
                Some(Command::Reconnect) => break End::Retry(Retry::Requested),
                Some(Command::NetworkChanged) => {
                    network_at = Some(Instant::now() + NETWORK_SETTLE);
                }
                Some(Command::Send { id, paths }) => room.outgoing(&id, paths, &peers),
            },
            _ = tokio::time::sleep_until(network_at.unwrap_or_else(Instant::now)), if network_at.is_some() => {
                if lock(&shared.board).busy() {
                    network_at = Some(Instant::now() + NETWORK_SETTLE);
                } else {
                    break End::Retry(Retry::Requested);
                }
            }
            _ = refresh.tick() => {
                // Tokens older than an hour are rejected by peers; LocalSend Web refreshes every 30 min too
                if let Some(info) = client_info(cfg, &me.key) {
                    let _ = room.sig.send_update(info).await;
                }
            }
        }
    };
    // The old room's peers and ids go. A connection already made does not
    // need the room: it, its id and its decisions stay
    shared.with(|b| match end {
        End::Shutdown => b.end_session(Stage::Cancelled),
        End::Retry(_) => b.leave_room(Stage::Failed(Failure::Other(
            "Connection to the signaling server lost".into(),
        ))),
    });
    end
}

fn peer_view(p: &ClientInfo) -> Peer {
    Peer {
        id: p.id.to_string(),
        alias: devices::clean_name(&p.alias),
        model: p
            .device_model
            .as_ref()
            .map(|m| m.chars().take(64).collect()),
        kind: match p.device_type {
            Some(DeviceType::Mobile) => "mobile",
            Some(DeviceType::Desktop) => "desktop",
            Some(DeviceType::Web) => "web",
            Some(DeviceType::Headless) => "headless",
            Some(DeviceType::Server) => "server",
            None => "unknown",
        },
    }
}

/// The core takes the key by value per session; one persistent identity.
fn copy_key(key: &token::SigningTokenKey) -> token::SigningTokenKey {
    token::export_private_key(key)
        .ok()
        .and_then(|pem| token::parse_private_key(&pem).ok())
        .unwrap_or_else(token::generate_key)
}

/// What an offer or answer says. Claims only.
struct Said {
    fingerprint: Option<ClaimedFingerprint>,
    capability: Capability,
    /// The commitment (offer) or nonce (answer), if it is well formed.
    value: Option<String>,
    busy: bool,
}

impl Said {
    fn read(signaled: &str, name: &str) -> Self {
        let text = sdp_text(signaled).unwrap_or_default();
        Self {
            fingerprint: fingerprint_from_sdp(&text).map(ClaimedFingerprint::from_signaling),
            // Malformed is still Oeca (failed), never a reason to fall back
            capability: if osv::marked(&text) {
                Capability::Oeca
            } else {
                Capability::Stock
            },
            value: osv::attr(&text, name),
            busy: osv::busy(&text),
        }
    }

    /// None: a stock peer.
    fn usable(&self) -> Result<(ClaimedFingerprint, Option<String>), Failure> {
        let oeca = self.capability == Capability::Oeca;
        let malformed = |why: &str| {
            if oeca {
                Failure::Authentication(why.into())
            } else {
                Failure::Other(why.into())
            }
        };
        let claimed = self
            .fingerprint
            .clone()
            .ok_or_else(|| malformed("No single certificate fingerprint was announced"))?;
        match (oeca, &self.value) {
            (true, None) => Err(malformed("The verification data is malformed")),
            (true, value) => Ok((claimed, value.clone())),
            (false, _) => Ok((claimed, None)),
        }
    }
}

impl Room<'_> {
    fn exchange(&self, peer_id: String, claimed: ClaimedFingerprint) -> Option<proof::Exchange> {
        Some(proof::Exchange {
            mine: self.me.fingerprint.clone()?,
            my_id: self.sig.client.id.to_string(),
            peer_id,
            claimed,
        })
    }

    fn incoming(&self, offer: WsServerSdpMessage) {
        if offer.sdp.len() > MAX_SIGNALED_BYTES || offer.session_id.len() > MAX_SESSION_ID {
            return;
        }
        let said = Said::read(&offer.sdp, osv::COMMIT);
        let oeca = said.capability == Capability::Oeca;
        let shared = self.shared;
        // One step with the check: a send cannot start in between
        let begun = shared.with(|b| {
            if b.busy() {
                return None;
            }
            let id = b.begin(Direction::Receive, peer_view(&offer.peer));
            b.claim(&id, said.fingerprint.clone(), said.capability);
            Some(id)
        });
        let peer = offer.peer.id.to_string();
        let Some(id) = begun else {
            if said.usable().is_err() {
                return;
            }
            if oeca {
                if shared.may_refuse(&peer) {
                    tokio::spawn(refuse_busy(self.sig.clone(), offer));
                }
            } else if let Ok(slot) = shared.declines.clone().try_acquire_owned()
                && shared.may_refuse(&peer)
            {
                let declining = decline(
                    self.sig.clone(),
                    copy_key(&self.me.key),
                    self.me.cert.clone(),
                    self.cfg.stun.clone(),
                    offer,
                );
                tokio::spawn(async move {
                    declining.await;
                    drop(slot);
                });
            }
            return;
        };
        let fail = |why| shared.with(|b| b.kill(&id, Stage::Failed(why)));
        let (claimed, commit) = match said.usable() {
            Ok(usable) => usable,
            Err(why) => return fail(why),
        };
        let verification = match commit {
            Some(commit) => {
                let Some(exchange) = self.exchange(peer.clone(), claimed.clone()) else {
                    return fail(Failure::Authentication("No identity on this device".into()));
                };
                // Our answer lets the sender compute the code before it reveals,
                // so it is counted first, whoever the offer says it is from
                if !shared.devices.begin_attempt(&peer, id.as_str()) {
                    fail(if shared.devices.status() == Status::Ready {
                        Failure::TooManyAttempts
                    } else {
                        Failure::TrustUnavailable
                    });
                    if shared.may_refuse(&peer) {
                        tokio::spawn(refuse_busy(self.sig.clone(), offer));
                    }
                    return;
                }
                Some((exchange, commit))
            }
            None => None,
        };
        let task = self.receive(id.clone(), offer, claimed, verification);
        shared.with(|b| b.attach(&id, task, None));
    }

    fn outgoing(&self, id: &TransferId, paths: Vec<PathBuf>, peers: &[ClientInfo]) {
        let shared = self.shared;
        let target = {
            let board = lock(&shared.board);
            let Some(t) = board.live_transfer(id) else {
                return; // cancelled or replaced while queued
            };
            let selected = &t.claimed.peer.id;
            peers.iter().find(|p| p.id.to_string() == *selected)
        };
        let fail = |why| shared.with(|b| b.kill(id, Stage::Failed(why)));
        let Some(target) = target else {
            return fail(Failure::Other("The selected device left".into()));
        };
        // Without one the offer would look like a stock peer's
        let Some(mine) = self.me.fingerprint.clone() else {
            return fail(Failure::Authentication("No identity on this device".into()));
        };
        let (answers_tx, answers_rx) = mpsc::channel(16);
        let task = self.send(id.clone(), target, mine, paths, answers_rx);
        shared.with(|b| b.attach(id, task, Some((target.id.to_string(), answers_tx))));
    }

    fn send(
        &self,
        id: TransferId,
        target: &ClientInfo,
        mine: Fingerprint,
        paths: Vec<PathBuf>,
        mut answers: mpsc::Receiver<WsServerSdpMessage>,
    ) -> tokio::task::AbortHandle {
        let (sig, key, cert, stun, shared) = (
            self.sig.clone(),
            copy_key(&self.me.key),
            self.me.cert.clone(),
            self.cfg.stun.clone(),
            self.shared.clone(),
        );
        let target = target.id;
        let secret = osv::random();
        let task = tokio::spawn(async move {
            let looked = tokio::task::spawn_blocking(move || (prepare(&paths), paths));
            let Ok(Ok(((dtos, rows, bad), paths))) =
                tokio::time::timeout(PREPARE_TIMEOUT, looked).await
            else {
                let why = Failure::Other("The files to send could not be read".into());
                return finish(&shared, &id, Err(why));
            };
            shared.with(|b| b.set_files(&id, rows));
            if let Some(why) = bad {
                return finish(&shared, &id, Err(Failure::Other(why)));
            }
            // The core applies the first answer with its session id, whoever
            // sent it: it gets a signaling connection of its own, which carries
            // only its offer out and the selected peer's answer in
            let (core_out, mut core_offers) = mpsc::channel(4);
            let (core_in, core_answers) = mpsc::channel(4);
            let (core_sig, _core_events) = SignalingConnection {
                client: sig.client.clone(),
                tx: core_out,
                rx: core_answers,
            }
            .start_listener();
            let (gate_tx, gate_rx) = oneshot::channel::<bool>();
            let (pc_tx, pc_rx) = oneshot::channel();
            let (results_tx, results) = mpsc::channel(16);
            let extension = rtc::Extension {
                sdp_lines: vec![format!("a={}:{}", osv::COMMIT, osv::commit(&secret))],
                peer_connection: Some(pc_tx),
                channels: None,
                applied_answer: None,
                file_results: Some(results_tx),
            };
            let (status_tx, status) = mpsc::channel(8);
            let (sel_tx, sel_rx) = oneshot::channel::<HashSet<String>>();
            let (err_tx, errors) = mpsc::channel(8);
            let (connected_tx, connected_rx) = oneshot::channel();
            // PIN and pairing are not offered: dropping their receivers makes the
            // core fail fast instead of waiting for an answer that never comes
            let (pin_tx, _) = mpsc::channel(1);
            let (pair_tx, _) = oneshot::channel();
            let (sending_tx, sending_rx) = mpsc::channel::<RTCFile>(1);
            let (progress_tx, progress) = watch::channel(0u64);
            let (asked_tx, asked) = watch::channel(DECIDING);
            let files = dtos.clone();
            let confirmed = confirmations(results, &shared, &id, progress_tx.clone());
            let side = {
                let (s, id) = (shared.clone(), id.clone());
                let my_id = sig.client.id.to_string();
                async move {
                    let Some(WsClientMessage::Offer(offer)) = core_offers.recv().await else {
                        return Err(closed());
                    };
                    let session = offer.session_id.clone();
                    sig.send_offer(offer.session_id, target, offer.sdp)
                        .await
                        .map_err(|_| closed())?;
                    // All it needs of the room: holding on would keep the old
                    // socket (a ghost peer) open after a reconnect
                    drop(sig);
                    let answer = selected_answer(&mut answers, target.to_string(), session)
                        .await
                        .ok_or_else(closed)?;
                    let said = Said::read(&answer.sdp, osv::NONCE);
                    s.with(|b| b.claim(&id, said.fingerprint.clone(), said.capability));
                    if said.busy {
                        return Err(Failure::Busy);
                    }
                    let (claimed, nonce) = said.usable()?;
                    core_in
                        .send(WsServerMessage::Answer(answer))
                        .await
                        .map_err(|_| closed())?;
                    let pc = pc_rx.await.map_err(|_| closed())?;
                    // Kept open as long as this session
                    let mut _channel = None;
                    let authenticated = match nonce {
                        Some(nonce) => {
                            let exchange = proof::Exchange {
                                mine,
                                my_id,
                                peer_id: target.to_string(),
                                claimed: claimed.clone(),
                            };
                            let (peer, dc) =
                                authentication(exchange.initiate(&pc, &secret, &nonce)).await?;
                            _channel = Some(dc);
                            Some(peer)
                        }
                        None => None,
                    };
                    connected_rx.await.map_err(|_| closed())?;
                    let who = match authenticated {
                        Some(peer) => Proof::Oeca(peer),
                        None => Proof::Stock(
                            proof::connected(&pc, &claimed)
                                .await
                                .map_err(|e| Failure::Other(e.into()))?,
                        ),
                    };
                    let gate = s.admit(&id, who)?;
                    asked_tx.send_replace(DECIDING);
                    decided(gate).await?;
                    // Names, sizes and contents leave only past this line
                    s.still_allowed(&id)?;
                    gate_tx.send(true).map_err(|_| closed())?;
                    asked_tx.send_replace(THEY_DECIDE);
                    let Ok(selected) = sel_rx.await else {
                        return Ok(());
                    };
                    s.with(|b| b.sending(&id));
                    for (i, (dto, path)) in dtos.iter().zip(&paths).enumerate() {
                        if !selected.contains(&dto.id) {
                            s.files(&id, |files, _| files[i].ok = Some(false));
                            continue;
                        }
                        s.still_allowed(&id)?;
                        let (btx, brx) = mpsc::channel(4);
                        let file = RTCFile {
                            file_id: dto.id.clone(),
                            binary_rx: brx,
                        };
                        sending_tx.send(file).await.map_err(|_| closed())?;
                        let (mut chunks, read) = read_file(path.clone(), dto.size);
                        let mut sent = 0u64;
                        while let Some(chunk) = chunks.recv().await {
                            let n = chunk.len() as u64;
                            btx.send(chunk.into()).await.map_err(|_| closed())?;
                            sent += n;
                            progress_tx.send_modify(|b| *b += n);
                            s.files(&id, |files, _| files[i].done = sent);
                        }
                        read.await.map_err(|_| closed())?.map_err(Failure::Other)?;
                        // Its bytes counted as they were queued; the file itself
                        // counts once, when the receiver confirms it (`confirmations`)
                    }
                    asked_tx.send_replace(CONFIRMING);
                    Ok(())
                }
            };
            let sending = rtc::send_offer(
                &core_sig,
                stun,
                cert,
                target,
                key,
                None,
                None,
                files,
                status_tx,
                sel_tx,
                err_tx,
                pin_tx,
                pair_tx,
                sending_rx,
                Some(gate_rx),
                Some(extension),
            );
            // Over only once every confirmation the core read is recorded
            let core = async { tokio::join!(sending, confirmed).0 };
            let signals = Signals {
                status,
                errors,
                progress,
                asked,
                connected: Some(connected_tx),
            };
            let r = drive(core, side, signals, &shared, &id, ANSWER_TIMEOUT).await;
            finish(&shared, &id, r);
        });
        task.abort_handle()
    }

    fn receive(
        &self,
        id: TransferId,
        offer: WsServerSdpMessage,
        claimed: ClaimedFingerprint,
        verification: Option<(proof::Exchange, String)>,
    ) -> tokio::task::AbortHandle {
        let (sig, key, cert, stun, shared, dir) = (
            self.sig.clone(),
            copy_key(&self.me.key),
            self.me.cert.clone(),
            self.cfg.stun.clone(),
            self.shared.clone(),
            self.cfg.receive_dir.clone(),
        );
        // With a commitment in the offer: our random goes into the answer, and
        // the code exists once the sender's reveal matches its commitment
        let verification = verification.map(|(exchange, commit)| (exchange, commit, osv::random()));
        let task = tokio::spawn(async move {
            // A signaling connection of the core's own, carrying only its
            // answer out: the core would hold the room's open until the end
            let (core_out, mut core_answers) = mpsc::channel(1);
            let (_nothing_in, core_in) = mpsc::channel(1);
            let (core_sig, _core_events) = SignalingConnection {
                client: sig.client.clone(),
                tx: core_out,
                rx: core_in,
            }
            .start_listener();
            let (pc_tx, pc_rx) = oneshot::channel();
            let (channels_tx, mut channels) = mpsc::channel(2);
            let extension = rtc::Extension {
                sdp_lines: verification
                    .iter()
                    .map(|(_, _, nonce)| format!("a={}:{nonce}", osv::NONCE))
                    .collect(),
                peer_connection: Some(pc_tx),
                channels: verification.is_some().then_some(channels_tx),
                applied_answer: None,
                file_results: None,
            };
            let (status_tx, status) = mpsc::channel(8);
            let (files_tx, files_rx) = oneshot::channel::<Vec<FileDto>>();
            let (sel_tx, sel_rx) = oneshot::channel();
            let (err_tx, errors) = mpsc::channel(8);
            let (pin_tx, _) = mpsc::channel(1);
            let (recv_tx, mut recv_rx) = mpsc::channel::<RTCFile>(1);
            let (result_tx, result_rx) = mpsc::channel(1);
            let (progress_tx, progress) = watch::channel(0u64);
            let (asked_tx, asked) = watch::channel(DECIDING);
            let side = {
                let (s, id) = (shared.clone(), id.clone());
                async move {
                    let Some(WsClientMessage::Answer(answer)) = core_answers.recv().await else {
                        return Err(closed());
                    };
                    sig.send_answer(answer.session_id, answer.target, answer.sdp)
                        .await
                        .map_err(|_| closed())?;
                    drop(sig);
                    let pc = pc_rx.await.map_err(|_| closed())?;
                    let mut _channel = None;
                    let files = if let Some((exchange, commit, nonce)) = &verification {
                        let (peer, dc) =
                            authentication(exchange.answer(&pc, &mut channels, commit, nonce))
                                .await?;
                        _channel = Some(dc);
                        let gate = s.admit(&id, Proof::Oeca(peer))?;
                        asked_tx.send_replace(DECIDING);
                        decided(gate).await?;
                        asked_tx.send_replace(THEY_DECIDE); // the sender may still be comparing
                        let Ok(files) = files_rx.await else {
                            return Ok(());
                        };
                        files
                    } else {
                        // Stock: the list is the first thing that shows it is connected
                        let Ok(files) = files_rx.await else {
                            return Ok(());
                        };
                        let peer = proof::connected(&pc, &claimed)
                            .await
                            .map_err(|e| Failure::Other(e.into()))?;
                        decided(s.admit(&id, Proof::Stock(peer))?).await?;
                        files
                    };
                    s.still_allowed(&id)?;
                    if let Err(e) = check_offer(&files) {
                        let _ = sel_tx.send(None);
                        s.with(|b| b.end(&id, Stage::Failed(Failure::Other(e))));
                        return Ok(());
                    }
                    let rows = files
                        .iter()
                        .map(|f| FileRow {
                            name: safe_name(&f.file_name),
                            size: f.size,
                            done: 0,
                            ok: None,
                        })
                        .collect();
                    let Some(gate) = s.with(|b| b.offer(&id, rows)) else {
                        return Err(closed());
                    };
                    asked_tx.send_replace(DECIDING);
                    // Never automatic: an explicit yes, or the timeout declines
                    let accept = matches!(
                        tokio::time::timeout(APPROVAL_TIMEOUT, gate).await,
                        Ok(Ok(true))
                    );
                    if !accept {
                        let _ = sel_tx.send(None);
                        s.with(|b| b.end(&id, Stage::Declined));
                        return Ok(());
                    }
                    s.still_allowed(&id)?;
                    let _ = sel_tx.send(Some(files.iter().map(|f| f.id.clone()).collect()));
                    // Starts the inactivity deadline: a sender that vanished now
                    // (cancelled without closing) ends us in IDLE_TIMEOUT
                    progress_tx.send_modify(|_| ());
                    let mut once = Once::new(&files);
                    while let Some(mut file) = recv_rx.recv().await {
                        let i = once.take(&file.file_id)?;
                        s.still_allowed(&id)?;
                        let big = files[i].size >= SLOW_FLUSH_BYTES;
                        let flushing = || {
                            if big {
                                asked_tx.send_replace(FLUSHING);
                            }
                        };
                        let shown = |done| s.files(&id, |rows, _| rows[i].done = done);
                        let r = save_one(&dir, &files[i], &mut file, &progress_tx, flushing, shown)
                            .await;
                        s.files(&id, |rows, saved| {
                            rows[i].ok = Some(r.is_ok());
                            if let Ok(p) = &r {
                                saved.push(p.clone());
                            }
                        });
                        progress_tx.send_modify(|b| *b += FILE_PROGRESS);
                        let response = RTCSendFileResponse {
                            id: file.file_id,
                            success: r.is_ok(),
                            error: r.err(),
                        };
                        if result_tx.send(response).await.is_err() {
                            break;
                        }
                    }
                    Ok(())
                }
            };
            let core = rtc::accept_offer(
                &core_sig,
                stun,
                cert,
                &offer,
                key,
                None,
                None,
                status_tx,
                files_tx,
                sel_rx,
                err_tx,
                pin_tx,
                recv_tx,
                result_rx,
                Some(extension),
            );
            let signals = Signals {
                status,
                errors,
                progress,
                asked,
                connected: None,
            };
            let r = drive(core, side, signals, &shared, &id, RTC_TIMEOUT).await;
            finish(&shared, &id, r);
        });
        task.abort_handle()
    }
}

fn closed() -> Failure {
    Failure::Other("The connection closed".into())
}

async fn authentication<T>(
    exchange: impl Future<Output = Result<T, &'static str>>,
) -> Result<T, Failure> {
    let failed = |why: &str| Failure::Authentication(why.into());
    tokio::time::timeout(RTC_TIMEOUT, exchange)
        .await
        .map_err(|_| failed("Verification did not complete in time"))?
        .map_err(failed)
}

/// Waits for the question the board is asking, if it asks one.
async fn decided(gate: Gate) -> Result<(), Failure> {
    let Some(answer) = gate else { return Ok(()) };
    match tokio::time::timeout(APPROVAL_TIMEOUT, answer).await {
        Ok(Ok(true)) => Ok(()),
        Ok(_) => Err(Failure::Other("Not confirmed".into())),
        Err(_) => Err(Failure::Other("Timed out waiting for a decision".into())),
    }
}

/// The answer to this session from the selected peer. Others never reach the core.
async fn selected_answer(
    answers: &mut mpsc::Receiver<WsServerSdpMessage>,
    target: String,
    session: String,
) -> Option<WsServerSdpMessage> {
    loop {
        let answer = answers.recv().await?;
        if answer.session_id == session && answer.peer.id.to_string() == target {
            return Some(answer);
        }
    }
}

/// An Oeca peer that cannot be answered now is told so in the answer itself:
/// no nonce, no connection, and still visibly Oeca.
async fn refuse_busy(sig: Signaling, offer: WsServerSdpMessage) {
    let text = format!(
        "v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\na={}:1\r\n",
        osv::BUSY
    );
    let Some(sdp) = signaled_sdp(&text) else {
        return;
    };
    let sent = sig.send_answer(offer.session_id, offer.peer.id, sdp);
    let _ = tokio::time::timeout(BUSY_DECLINE_TIMEOUT, sent).await;
}

/// Another transfer is running: say no over the protocol, so the (stock)
/// sender learns it now instead of timing out.
async fn decline(
    sig: Signaling,
    key: token::SigningTokenKey,
    cert: Option<RTCCertificate>,
    stun: Vec<String>,
    offer: WsServerSdpMessage,
) {
    let (status_tx, mut status_rx) = mpsc::channel(8);
    let (files_tx, files_rx) = oneshot::channel::<Vec<FileDto>>();
    let (sel_tx, sel_rx) = oneshot::channel();
    let (err_tx, _) = mpsc::channel(8);
    let (pin_tx, _) = mpsc::channel(1);
    let (recv_tx, _) = mpsc::channel(1);
    let (_result_tx, result_rx) = mpsc::channel(1);
    tokio::spawn(async move { while status_rx.recv().await.is_some() {} });
    tokio::spawn(async move {
        if files_rx.await.is_ok() {
            let _ = sel_tx.send(None);
        }
    });
    let core = rtc::accept_offer(
        &sig, stun, cert, &offer, key, None, None, status_tx, files_tx, sel_rx, err_tx, pin_tx,
        recv_tx, result_rx, None,
    );
    let _ = tokio::time::timeout(BUSY_DECLINE_TIMEOUT, core).await;
}

/// How long something may take, and what is then said to be late.
type Wait = (Duration, &'static str);
const DECIDING: Wait = (APPROVAL_TIMEOUT.saturating_add(IDLE_TIMEOUT), "a decision");
const THEY_DECIDE: Wait = (PEER_DECIDES, "the other device");
const FLUSHING: Wait = (CONFIRM_TIMEOUT, "the disk");
const CONFIRMING: Wait = (CONFIRM_TIMEOUT, "the receiver's confirmation");

/// A sent file is saved when the receiver says so, not when it was queued.
async fn confirmations(
    mut results: mpsc::Receiver<RTCSendFileResponse>,
    shared: &Shared,
    id: &TransferId,
    progress: watch::Sender<u64>,
) {
    while let Some(result) = results.recv().await {
        // Our own ids (the row's number): the core passes on no others
        let Ok(row) = result.id.parse::<usize>() else {
            continue;
        };
        let mut known = false;
        shared.files(id, |files, _| {
            if let Some(file) = files.get_mut(row) {
                file.ok = Some(result.success);
                known = true;
            }
        });
        // Counts only for a file of this transfer
        if known {
            progress.send_modify(|b| *b += FILE_PROGRESS);
        }
    }
}

/// What `drive` listens to.
struct Signals<E> {
    status: mpsc::Receiver<RTCStatus>,
    errors: mpsc::Receiver<E>,
    progress: watch::Receiver<u64>,
    /// Someone was just given something slow to do: their time starts again.
    asked: watch::Receiver<Wait>,
    /// Send: told when the core's handshake is through.
    connected: Option<oneshot::Sender<()>>,
}

/// Runs a core session next to its app-side half (`side`), with a deadline
/// for whatever stage it is in. Returns when the core does, or on the first error.
async fn drive<C, X, S, E>(
    core: C,
    side: S,
    signals: Signals<E>,
    shared: &Shared,
    id: &TransferId,
    first: Duration,
) -> Result<(), Failure>
where
    C: Future<Output = Result<(), X>>,
    X: std::fmt::Display,
    S: Future<Output = Result<(), Failure>>,
    E: std::fmt::Debug,
{
    let Signals {
        mut status,
        mut errors,
        mut progress,
        mut asked,
        mut connected,
    } = signals;
    tokio::pin!(core, side);
    let mut side_done = false;
    let mut deadline = Instant::now() + first;
    let mut what = "the answer";
    let mut closing = false;
    let mut pace = Pace::default();
    let failed = |why: String| Err(Failure::Other(why));
    let declined = || shared.with(|b| b.end(id, Stage::Declined));
    loop {
        tokio::select! {
            r = &mut core => {
                // The core reports Declined / Error and returns at once: those
                // statuses may still be queued when its future wins the select
                while let Ok(st) = status.try_recv() {
                    match st {
                        RTCStatus::Declined => declined(),
                        RTCStatus::Error(e) => return failed(e),
                        _ => {}
                    }
                }
                return r.map_err(|e| Failure::Other(format!("{e:#}")));
            }
            r = &mut side, if !side_done => {
                side_done = true;
                r?;
            }
            Some(st) = status.recv() => match st {
                RTCStatus::SdpExchanged => {
                    deadline = Instant::now() + RTC_TIMEOUT;
                    what = "the connection";
                }
                RTCStatus::Connected => {
                    if let Some(tx) = connected.take() {
                        let _ = tx.send(());
                    }
                    deadline = Instant::now() + APPROVAL_TIMEOUT + IDLE_TIMEOUT;
                    what = "a decision";
                }
                RTCStatus::Declined | RTCStatus::Finished => {
                    if st == RTCStatus::Declined {
                        declined();
                    }
                    // Over: the core only closes its connection now, and that
                    // close can hang once the other side is gone
                    deadline = Instant::now() + CLOSE_GRACE;
                    closing = true;
                }
                RTCStatus::PinRequired => return failed("The other device requires a PIN".into()),
                RTCStatus::TooManyAttempts => return failed("Too many PIN attempts".into()),
                RTCStatus::Error(e) => return failed(e),
                RTCStatus::Sending => {}
            },
            Some(e) = errors.recv() => {
                // ponytail: file-level errors fail the session; the core keeps going, we do not
                return failed(format!("{e:?}"));
            }
            Ok(()) = asked.changed() => {
                let (wait, whose) = *asked.borrow_and_update();
                if !closing {
                    deadline = Instant::now() + wait;
                    what = whose;
                }
            }
            Ok(()) = progress.changed() => {
                let total = *progress.borrow_and_update();
                // The last chunk and the flush after it can be seen together:
                // the longer wait is the one that is asked for
                let waiting = matches!(asked.has_changed(), Ok(true));
                if pace.counts(total) && !closing && !waiting {
                    deadline = Instant::now() + IDLE_TIMEOUT;
                    what = "data";
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                // ponytail: dropping the core here abandons its close(); the runtime reclaims it at shutdown
                return if closing { Ok(()) } else { failed(format!("Timed out waiting for {what}")) };
            }
        }
    }
}

fn finish(shared: &Shared, id: &TransferId, r: Result<(), Failure>) {
    shared.with(|b| {
        // Not live: it already ended (cancelled, declined, replaced) and stays so
        let Some(stage) = b.live_transfer(id).map(|t| match &r {
            Err(why) => Stage::Failed(why.clone()),
            Ok(()) => outcome(t),
        }) else {
            return;
        };
        b.end(id, stage);
    });
}

/// What a session that ended without an error achieved: Done only if every
/// file was saved (sending: confirmed as saved), and never Declined, which only
/// the protocol's own DECLINED is.
fn outcome(t: &Transfer) -> Stage {
    if !t.files.is_empty() && t.files.iter().all(|f| f.ok == Some(true)) {
        return Stage::Done;
    }
    let why = match (&t.stage, t.direction) {
        // Nothing was proven
        (Stage::Signaling | Stage::Connecting, _) => {
            return Stage::Failed(if t.capability == Capability::Oeca {
                Failure::Authentication("The connection ended before it was authenticated".into())
            } else {
                closed()
            });
        }
        (Stage::Transferring, direction) => {
            let unsaved = t.files.iter().filter(|f| f.ok != Some(true)).count();
            let what = match direction {
                Direction::Send => "confirmed as saved",
                Direction::Receive => "saved",
            };
            format!("{unsaved} of {} files were not {what}", t.files.len())
        }
        (_, Direction::Send) => "The other device ended the connection before accepting".into(),
        (_, Direction::Receive) => "The other device ended the connection before sending".into(),
    };
    Stage::Failed(Failure::Other(why))
}

/// One approval is each approved file once. With the per-file size check, that
/// is also the bound on all bytes written: the approved total.
struct Once {
    left: std::collections::HashMap<String, usize>,
}

impl Once {
    fn new(files: &[FileDto]) -> Self {
        Self {
            left: files
                .iter()
                .enumerate()
                .map(|(i, f)| (f.id.clone(), i))
                .collect(),
        }
    }

    fn take(&mut self, id: &str) -> Result<usize, Failure> {
        self.left.remove(id).ok_or_else(|| {
            Failure::Other("The sender sent a file that was not approved, or sent one twice".into())
        })
    }
}

fn check_offer(files: &[FileDto]) -> Result<(), String> {
    if files.is_empty() {
        return Err("No files".into());
    }
    if files.len() > MAX_FILES {
        return Err(format!("More than {MAX_FILES} files"));
    }
    if files
        .iter()
        .any(|f| f.id.is_empty() || f.id.len() > MAX_ID_BYTES)
    {
        return Err("A file id is missing or too long".into());
    }
    if files.iter().any(|f| f.file_name.len() > MAX_NAME_BYTES) {
        return Err("A file name is too long".into());
    }
    if files.iter().map(|f| &f.id).collect::<HashSet<_>>().len() != files.len() {
        return Err("Two files have the same id".into());
    }
    let total = files
        .iter()
        .try_fold(0u64, |acc, f| acc.checked_add(f.size));
    if total.is_none_or(|t| t > MAX_TOTAL_BYTES) {
        return Err("Too large".into());
    }
    Ok(())
}

/// The longest name kept, in UTF-16 units (NTFS counts those, 255 per name):
/// room is left for " (999)" and ".999.oeca-part".
const MAX_SAVED_NAME: usize = 255 - " (999)".len() - ".999.oeca-part".len();
/// Longer than this after its last dot, it is not kept as an extension.
const MAX_EXTENSION: usize = 16;

/// A name that is safe as one file in the receive folder, and shows what it
/// is: no separators, streams (`:`), traversal or device names (the core's
/// sanitizer, plus the names it misses); no control, bidi or invisible
/// characters, so the name shown is the name saved and its real extension is
/// its last; too long, it is shortened before its extension.
pub fn safe_name(untrusted: &str) -> String {
    let visible: String = untrusted
        .chars()
        .filter(|&c| !c.is_control() && !devices::invisible(c))
        .collect();
    let mut n = filename::sanitize(&fit(visible.trim()), filename::Rules::Windows);
    if is_device_name(&n) {
        n.insert(0, '_');
    }
    let n = fit(&n);
    if n.is_empty() || n.chars().all(|c| c == '.') {
        "file".into()
    } else {
        n
    }
}

/// Windows' device names, read as Windows reads them: the part before the
/// first dot, without trailing spaces or dots (`con .txt`, `COM¹`, `CONIN$`).
fn is_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches([' ', '.'])
        .to_lowercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix).is_some_and(|d| {
            matches!(
                d,
                "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    };
    matches!(
        stem.as_str(),
        "con" | "prn" | "aux" | "nul" | "conin$" | "conout$" | "clock$"
    ) || numbered("com")
        || numbered("lpt")
}

/// At most `MAX_SAVED_NAME` UTF-16 units and 255 bytes (the core's limit),
/// cut before the extension so the extension is never lost.
fn fit(name: &str) -> String {
    let fits = |s: &str| s.encode_utf16().count() <= MAX_SAVED_NAME && s.len() <= 255;
    if fits(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && name[i..].chars().count() <= MAX_EXTENSION + 1 => name.split_at(i),
        _ => (name, ""),
    };
    let mut kept = String::new();
    for c in stem.chars() {
        let next = format!("{kept}{c}{ext}");
        if !fits(&next) {
            break;
        }
        kept.push(c);
    }
    format!("{}{ext}", kept.trim_end_matches([' ', '.']))
}

/// File types that run code, or open something elsewhere, when opened: the
/// approval and the received list say so. Read from the saved name's last
/// extension (the one Windows uses).
pub fn dangerous(name: &str) -> bool {
    const TYPES: &[&str] = &[
        "exe",
        "com",
        "scr",
        "pif",
        "cpl",
        "dll",
        "ocx",
        "sys",
        "drv",
        "msi",
        "msp",
        "mst",
        "msix",
        "msixbundle",
        "appx",
        "appxbundle",
        "appinstaller",
        "application",
        "appref-ms",
        "bat",
        "cmd",
        "ps1",
        "psm1",
        "psd1",
        "ps1xml",
        "psc1",
        "vbs",
        "vbe",
        "js",
        "jse",
        "wsf",
        "wsh",
        "wsc",
        "hta",
        "msc",
        "jar",
        "reg",
        "inf",
        "chm",
        "hlp",
        "lnk",
        "url",
        "website",
        "scf",
        "library-ms",
        "searchconnector-ms",
        "settingcontent-ms",
        "diagcab",
        "gadget",
        "xll",
        "xlam",
        "xlsm",
        "docm",
        "dotm",
        "pptm",
        "potm",
        "ppsm",
        "iso",
        "img",
        "vhd",
        "vhdx",
        "theme",
        "themepack",
        "desktopthemepackfile",
        "shs",
        "ws",
    ];
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e).to_lowercase();
    TYPES.contains(&ext.as_str())
}

/// The receive folder, made if missing (its parent, Downloads, must exist);
/// never a link or junction (one would send files elsewhere). Checked at each file: a same-user program can still
/// swap it between the check and the write, which Windows offers no way to
/// rule out from here.
fn receive_folder(dir: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    const REPARSE_POINT: u32 = 0x400;
    match std::fs::create_dir(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(e),
        _ => {}
    }
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() || meta.file_attributes() & REPARSE_POINT != 0 {
        return Err(std::io::Error::other(
            "The receive folder is a link: nothing is saved there",
        ));
    }
    Ok(())
}

/// Windows' Mark of the Web: received from another device, like an Internet
/// download (ZoneId 3), so SmartScreen and Protected View apply to it. No
/// address is written: the file says nothing about where it came from.
fn mark_internet_zone(path: &Path) -> std::io::Result<()> {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    std::fs::write(PathBuf::from(stream), "[ZoneTransfer]\r\nZoneId=3\r\n")
}

/// Renames, never replacing what is there: the file system refuses it if the
/// name exists, so nothing put there meanwhile is overwritten.
fn rename_new(from: &Path, to: &Path) -> std::io::Result<()> {
    use windows::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
    use windows::core::HSTRING;
    // As \\?\ paths, which Windows takes past 260 characters (std does this
    // for its own calls; MoveFileExW does not)
    let missing = || std::io::Error::from(std::io::ErrorKind::InvalidInput);
    let from = from.canonicalize()?;
    let to = to
        .parent()
        .ok_or_else(missing)?
        .canonicalize()?
        .join(to.file_name().ok_or_else(missing)?);
    unsafe {
        MoveFileExW(
            &HSTRING::from(from.as_os_str()),
            &HSTRING::from(to.as_os_str()),
            MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF))
}

/// How old a part file must be before a sweep may take it as abandoned.
const ABANDONED_AFTER: Duration = Duration::from_secs(24 * 3600);

/// Whether `name` is exactly what `Part::create` makes: `<safe name>.<n>.oeca-part`,
/// n from 0 to 999, written as `Part::create` writes it.
fn is_own_part(name: &str) -> bool {
    let Some(rest) = name.strip_suffix(".oeca-part") else {
        return false;
    };
    let Some((stem, n)) = rest.rsplit_once('.') else {
        return false;
    };
    let number = !n.is_empty()
        && n.len() <= 3
        && n.bytes().all(|b| b.is_ascii_digit())
        && (n == "0" || !n.starts_with('0'));
    number && !stem.is_empty() && safe_name(stem) == stem
}

/// Part files a crash left in the receive folder, deleted when Nearby starts
/// (before any transfer). Only Oeca's own names, only files (never a folder,
/// link or junction, never followed), only a day old by both their creation
/// and their last write, only if no one has them open. Anything uncertain
/// (unreadable times, a time in the future, an error) is kept. The folder is
/// read, not searched: nothing below it. Returns how many went.
fn sweep_parts(dir: &Path, now: std::time::SystemTime) -> usize {
    use std::os::windows::fs::MetadataExt;
    const REPARSE_POINT: u32 = 0x400;
    let plain_folder = std::fs::symlink_metadata(dir)
        .is_ok_and(|m| m.is_dir() && m.file_attributes() & REPARSE_POINT == 0);
    let Ok(entries) = std::fs::read_dir(dir)
        .map_err(|_| ())
        .and_then(|e| if plain_folder { Ok(e) } else { Err(()) })
    else {
        return 0;
    };
    let old = |m: &std::fs::Metadata| {
        let aged = |t: std::io::Result<std::time::SystemTime>| {
            t.ok()
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|age| age >= ABANDONED_AFTER)
        };
        m.is_file() && aged(m.modified()) && aged(m.created())
    };
    entries
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(is_own_part))
        .filter(|e| crate::data_dir::remove_if(&dir.join(e.file_name()), old).unwrap_or(false))
        .count()
}

/// The file being received: deleted on drop unless `commit` made it the real one.
struct Part {
    path: PathBuf,
    file: Option<std::fs::File>,
}

impl Part {
    fn create(dir: &Path, name: &str) -> std::io::Result<Self> {
        receive_folder(dir)?;
        for n in 0..1000 {
            let path = dir.join(format!("{name}.{n}.oeca-part"));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file: Some(file),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(std::io::ErrorKind::AlreadyExists.into())
    }

    /// Marked as from another device (`mark`), then becomes `dir/name`, or
    /// `name (2)`...: the first name free, never replacing a file. Unmarked,
    /// it is not saved at all.
    fn commit(
        mut self,
        dir: &Path,
        name: &str,
        mark: impl Fn(&Path) -> std::io::Result<()>,
    ) -> std::io::Result<PathBuf> {
        if let Some(f) = self.file.take() {
            f.sync_all()?;
        }
        mark(&self.path).map_err(|e| {
            std::io::Error::other(format!(
                "Not saved: it could not be marked as received from another device ({e})"
            ))
        })?;
        let stem = Path::new(name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ext = Path::new(name)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        for n in 1..1000 {
            let candidate = if n == 1 {
                dir.join(name)
            } else {
                dir.join(format!("{stem} ({n}){ext}"))
            };
            match rename_new(&self.path, &candidate) {
                Ok(()) => {
                    self.path = PathBuf::new();
                    return Ok(candidate);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(std::io::ErrorKind::AlreadyExists.into())
    }
}

impl Drop for Part {
    fn drop(&mut self) {
        self.file = None;
        if !self.path.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

enum Disk<B> {
    Chunk(B),
    /// All of it came. Without this (cancel, failure) the partial file is removed.
    Commit,
}

/// Writes one received file on a blocking thread. Dropping either end abandons
/// it, up to the rename.
fn write_file<B: AsRef<[u8]> + Send + 'static>(
    dir: PathBuf,
    name: String,
) -> (
    mpsc::Sender<Disk<B>>,
    oneshot::Receiver<Result<PathBuf, String>>,
) {
    let (tx, mut rx) = mpsc::channel::<Disk<B>>(4);
    let (result_tx, result) = oneshot::channel();
    tokio::task::spawn_blocking(move || {
        let failed = |e: std::io::Error| e.to_string();
        let written = (|| {
            let mut part = Part::create(&dir, &name).map_err(failed)?;
            loop {
                let file = part.file.as_mut().ok_or("closed")?;
                match rx.blocking_recv() {
                    None => return Err("Cancelled".to_string()),
                    Some(Disk::Chunk(bytes)) => file.write_all(bytes.as_ref()).map_err(failed)?,
                    Some(Disk::Commit) => {
                        file.sync_all().map_err(failed)?;
                        break;
                    }
                }
            }
            // The flush can be slow: cancelled meanwhile, it stays unsaved
            if result_tx.is_closed() {
                return Err("Cancelled".to_string());
            }
            part.commit(&dir, &name, mark_internet_zone).map_err(failed)
        })();
        drop(rx);
        let _ = result_tx.send(written);
    });
    (tx, result)
}

/// A failed write still reads the file to its end: the sender is told about
/// this file, and the next one goes on.
async fn save_one(
    dir: &Path,
    dto: &FileDto,
    file: &mut RTCFile,
    progress: &watch::Sender<u64>,
    flushing: impl Fn(),
    done: impl Fn(u64),
) -> Result<PathBuf, String> {
    let (disk, written) = write_file(dir.to_path_buf(), safe_name(&dto.file_name));
    let mut got = 0u64;
    let mut writing = true;
    while let Some(chunk) = file.binary_rx.recv().await {
        let len = chunk.len() as u64;
        got += len;
        // The core already stops a sender at its declared size; checked again here
        if got > dto.size {
            return Err("More data than declared".into());
        }
        writing = writing && disk.send(Disk::Chunk(chunk)).await.is_ok();
        progress.send_modify(|b| *b += len);
        if writing {
            done(got);
        }
    }
    if got != dto.size {
        return Err("Incomplete".into());
    }
    if writing {
        flushing();
        let _ = disk.send(Disk::Commit).await;
    }
    drop(disk);
    written
        .await
        .unwrap_or_else(|_| Err("Saving stopped".into()))
}

/// Reads one file to send on a blocking thread, exactly `size` bytes of it.
/// Dropping the receiver ends it.
fn read_file(
    path: PathBuf,
    size: u64,
) -> (
    mpsc::Receiver<Vec<u8>>,
    oneshot::Receiver<Result<(), String>>,
) {
    let (tx, chunks) = mpsc::channel(2);
    let (result_tx, result) = oneshot::channel();
    tokio::task::spawn_blocking(move || {
        let read = (|| {
            let changed = || "A file changed while sending".to_string();
            let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
            let mut total = 0u64;
            loop {
                let mut buf = vec![0u8; READ_CHUNK];
                let n = file.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    return if total == size {
                        Ok(())
                    } else {
                        Err(changed())
                    };
                }
                total += n as u64;
                if total > size {
                    return Err(changed());
                }
                buf.truncate(n);
                if tx.blocking_send(buf).is_err() {
                    return Err("Cancelled".to_string());
                }
            }
        })();
        drop(tx);
        let _ = result_tx.send(read);
    });
    (chunks, result)
}

/// What to offer for `paths`, and why not (blocking: it looks at each).
fn prepare(paths: &[PathBuf]) -> (Vec<FileDto>, Vec<FileRow>, Option<String>) {
    let mut dtos = Vec::new();
    let mut rows = Vec::new();
    let mut bad = None;
    for (i, p) in paths.iter().enumerate() {
        let size = std::fs::metadata(p)
            .ok()
            .filter(|m| m.is_file())
            .map(|m| m.len());
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if size.is_none() {
            bad = Some(format!("Not a readable file: {name}"));
        }
        let size = size.unwrap_or(0);
        rows.push(FileRow {
            name: name.clone(),
            size,
            done: 0,
            ok: None,
        });
        dtos.push(FileDto {
            id: i.to_string(),
            file_name: name,
            size,
            file_type: "application/octet-stream".into(),
            sha256: None,
            preview: None,
            metadata: None,
        });
    }
    if let Err(e) = check_offer(&dtos) {
        bad = Some(e);
    }
    (dtos, rows, bad)
}

/// An SDP as signaled (zlib + base64) back to text, bounded.
pub fn sdp_text(signaled_sdp: &str) -> Option<String> {
    if signaled_sdp.len() > MAX_SIGNALED_BYTES {
        return None;
    }
    let raw = base64::decode(signaled_sdp).ok()?;
    let mut text = String::new();
    flate2::read::ZlibDecoder::new(&raw[..])
        .take(MAX_SDP_BYTES + 1)
        .read_to_string(&mut text)
        .ok()?;
    // Over the limit is refused, as the core does: never a truncated view
    (text.len() as u64 <= MAX_SDP_BYTES).then_some(text)
}

/// An SDP as it is signaled.
fn signaled_sdp(text: &str) -> Option<String> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(text.as_bytes()).ok()?;
    Some(base64::encode(z.finish().ok()?))
}

/// The SDP's one fingerprint: a claim. Parsers differ on which line wins
/// (Chromium: the last of the media section; webrtc-rs: the first of each
/// section), so anything ambiguous is refused: every line that mentions a
/// fingerprint must be exactly `a=fingerprint:` with the same value.
fn fingerprint_from_sdp(sdp: &str) -> Option<Fingerprint> {
    let mut found: Option<Fingerprint> = None;
    for line in sdp.split(['\r', '\n']) {
        if !line.to_ascii_lowercase().contains("fingerprint") {
            continue;
        }
        let fp = Fingerprint::parse(line.strip_prefix("a=fingerprint:")?)?;
        if found.as_ref().is_some_and(|f| *f != fp) {
            return None;
        }
        found = Some(fp);
    }
    found
}

fn cert_fingerprint(cert: &RTCCertificate) -> Option<Fingerprint> {
    let fingerprints = cert.get_fingerprints();
    let f = fingerprints.first()?;
    Fingerprint::parse(&format!("{} {}", f.algorithm, f.value))
}

/// Oeca Nearby Verification v1: an Oeca-only extension that makes the
/// six-digit code safe at first contact. LocalSend's own messages are
/// untouched; peers that do not know it see two ignored SDP lines.
///
/// 1. Offer: `a=x-oeca-nearby-commit:C`, C = b64(SHA-256("oeca-nearby-commit-v1|" + rO)).
/// 2. Answer (only if the offer had C): `a=x-oeca-nearby-nonce:rA`. A peer
///    that cannot take the transfer answers `a=x-oeca-nearby-busy:1` instead,
///    with no nonce.
/// 3. After DTLS, only if the answer had rA: the offerer opens the data channel
///    `oeca-nearby-v1`, where rO is revealed and checked against C (`proof`
///    has the messages).
/// 4. Code = six digits of SHA-256("oeca-nearby-code-v2|" + lower fp + "|" +
///    higher fp + "|" + rO + "|" + rA): the DTLS fingerprints of this exact
///    connection, and randoms each side fixed before seeing the other's, so a
///    party in the middle gets one blind guess (1 in 10^6) per attempt.
///
/// rO, rA: 32 random bytes, URL-safe base64 without padding (LocalSend's).
/// The browser does the same (nearby-web/app/services/trust.ts); both test a
/// shared vector.
mod osv {
    use localsend::crypto::{hash, nonce};
    use localsend::util::base64;

    pub const COMMIT: &str = "x-oeca-nearby-commit";
    pub const NONCE: &str = "x-oeca-nearby-nonce";
    pub const BUSY: &str = "x-oeca-nearby-busy";
    pub const CHANNEL: &str = "oeca-nearby-v1";

    pub fn random() -> String {
        base64::encode(nonce::generate_nonce())
    }

    pub fn commit(secret: &str) -> String {
        base64::encode(hash::sha256(
            format!("oeca-nearby-commit-v1|{secret}").as_bytes(),
        ))
    }

    /// Claims to be Oeca Nearby, well formed or not.
    pub fn marked(sdp: &str) -> bool {
        sdp.to_ascii_lowercase().contains("x-oeca-nearby")
    }

    pub fn busy(sdp: &str) -> bool {
        let line = format!("a={BUSY}:1");
        sdp.lines().any(|l| l.trim() == line)
    }

    /// The one session-level `a=name:value`, its value 32 bytes of canonical
    /// unpadded URL-safe base64 (the commitment and the randoms), or None.
    /// One grammar with the browser's (`sdpAttr`, shared vectors in both
    /// tests): lines end at LF (a CR before it is dropped), nothing is
    /// trimmed, and the line appears exactly once, before the first `m=`.
    pub fn attr(sdp: &str, name: &str) -> Option<String> {
        let prefix = format!("a={name}:");
        let lines: Vec<&str> = sdp
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .collect();
        let session = lines
            .iter()
            .position(|l| l.starts_with("m="))
            .unwrap_or(lines.len());
        let mut found = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.starts_with(prefix.as_str()));
        let (at, line) = found.next()?;
        if found.next().is_some() || at >= session {
            return None;
        }
        let value = &line[prefix.len()..];
        (base64::decode(value).ok()?.len() == 32).then(|| value.to_string())
    }

    pub fn code(a: &str, b: &str, offer_secret: &str, answer_nonce: &str) -> String {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let h = hash::sha256(
            format!("oeca-nearby-code-v2|{lo}|{hi}|{offer_secret}|{answer_nonce}").as_bytes(),
        );
        let n = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) % 1_000_000;
        format!("{:03} {:03}", n / 1000, n % 1000)
    }
}

fn downloads_dir() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Downloads, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None).ok()?;
        let path = p.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(p.0 as *const _));
        path
    }
}

/// Windows' IP address notifications: an address added or removed (Wi-Fi
/// switch, cable, VPN up / down) asks the worker to reconnect, instead of
/// waiting for the socket to die. Callbacks run on a system thread and only
/// `try_send`; `Drop` cancels the registration, which waits for a callback in
/// progress, before the sender it points to is freed.
mod net {
    use super::Command;
    use tokio::sync::mpsc;
    use windows::Win32::Foundation::{HANDLE, NO_ERROR};
    use windows::Win32::NetworkManagement::IpHelper::{
        CancelMibChangeNotify2, MIB_NOTIFICATION_TYPE, MIB_UNICASTIPADDRESS_ROW, MibAddInstance,
        MibDeleteInstance, NotifyUnicastIpAddressChange,
    };
    use windows::Win32::Networking::WinSock::AF_UNSPEC;

    pub struct Watch {
        handle: HANDLE,
        context: *mut mpsc::Sender<Command>,
    }

    // The handle and the boxed sender are only touched by Drop (and the callback, by Windows)
    unsafe impl Send for Watch {}

    unsafe extern "system" fn changed(
        context: *const core::ffi::c_void,
        _row: *const MIB_UNICASTIPADDRESS_ROW,
        kind: MIB_NOTIFICATION_TYPE,
    ) {
        if (kind == MibAddInstance || kind == MibDeleteInstance) && !context.is_null() {
            let tx = unsafe { &*(context as *const mpsc::Sender<Command>) };
            let _ = tx.try_send(Command::NetworkChanged);
        }
    }

    impl Watch {
        pub fn start(tx: mpsc::Sender<Command>) -> Option<Self> {
            let context = Box::into_raw(Box::new(tx));
            let mut handle = HANDLE::default();
            let r = unsafe {
                NotifyUnicastIpAddressChange(
                    AF_UNSPEC,
                    Some(changed),
                    Some(context as *const core::ffi::c_void),
                    false,
                    &mut handle,
                )
            };
            if r != NO_ERROR {
                drop(unsafe { Box::from_raw(context) });
                return None;
            }
            Some(Self { handle, context })
        }
    }

    impl Drop for Watch {
        fn drop(&mut self) {
            unsafe {
                let _ = CancelMibChangeNotify2(self.handle);
                drop(Box::from_raw(self.context));
            }
        }
    }

    /// One network adapter, as the ICE host-candidate filter judges it.
    #[derive(Debug, Clone)]
    pub struct Adapter {
        pub name: String,
        pub if_type: u32,
        pub up: bool,
        pub gateway: bool,
        pub addrs: Vec<std::net::IpAddr>,
    }

    /// Whose addresses peers may see as host candidates: an adapter on a real
    /// network (Ethernet or Wi-Fi, up, with a default gateway). Virtual
    /// switches (Hyper-V, WSL, Docker, VirtualBox host-only) have no gateway;
    /// tunnels and most VPNs are other types. ponytail: a heuristic: a VPN
    /// that presents itself as Ethernet with a gateway (some TAP adapters)
    /// still passes; a cable between two PCs without a router does not.
    pub fn offered(a: &Adapter) -> bool {
        const ETHERNET: u32 = 6;
        const WIFI: u32 = 71;
        a.up && a.gateway && matches!(a.if_type, ETHERNET | WIFI)
    }

    /// This computer's adapters (empty if Windows cannot say).
    pub fn adapters() -> Vec<Adapter> {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
        use windows::Win32::NetworkManagement::IpHelper::{
            GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
            GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
        };
        use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
        use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6, SOCKADDR_IN, SOCKADDR_IN6};
        let flags = GAA_FLAG_INCLUDE_GATEWAYS
            | GAA_FLAG_SKIP_ANYCAST
            | GAA_FLAG_SKIP_MULTICAST
            | GAA_FLAG_SKIP_DNS_SERVER;
        let mut size = 16 * 1024u32;
        let mut buf: Vec<u64>;
        loop {
            buf = vec![0; size as usize / 8 + 1];
            let r = unsafe {
                GetAdaptersAddresses(
                    AF_UNSPEC.0 as u32,
                    flags,
                    None,
                    Some(buf.as_mut_ptr().cast()),
                    &mut size,
                )
            };
            match r {
                0 => break,
                // ERROR_BUFFER_OVERFLOW: `size` now says how much
                111 => continue,
                _ => return Vec::new(),
            }
        }
        let mut out = Vec::new();
        let mut next = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while let Some(a) = unsafe { next.as_ref() } {
            let mut addrs = Vec::new();
            let mut u = a.FirstUnicastAddress;
            while let Some(row) = unsafe { u.as_ref() } {
                let sa = row.Address.lpSockaddr;
                if let Some(family) = unsafe { sa.as_ref() }.map(|s| s.sa_family) {
                    if family == AF_INET {
                        let v4 = unsafe { &*(sa as *const SOCKADDR_IN) };
                        addrs.push(IpAddr::V4(Ipv4Addr::from(u32::from_be(unsafe {
                            v4.sin_addr.S_un.S_addr
                        }))));
                    } else if family == AF_INET6 {
                        let v6 = unsafe { &*(sa as *const SOCKADDR_IN6) };
                        addrs.push(IpAddr::V6(Ipv6Addr::from(unsafe { v6.sin6_addr.u.Byte })));
                    }
                }
                u = row.Next;
            }
            out.push(Adapter {
                name: unsafe { a.FriendlyName.to_string() }.unwrap_or_default(),
                if_type: a.IfType,
                up: a.OperStatus == IfOperStatusUp,
                gateway: !a.FirstGatewayAddress.is_null(),
                addrs,
            });
            next = a.Next;
        }
        out
    }

    /// For the core's ICE filter (vendor patch 17): may this local address be
    /// offered to a peer?
    pub fn may_offer(ip: std::net::IpAddr) -> bool {
        adapters()
            .iter()
            .any(|a| offered(a) && a.addrs.contains(&ip))
    }
}

/// The device key: Ed25519 (LocalSend's), PKCS#8 PEM sealed with DPAPI for
/// this Windows user, never written in the clear.
mod identity {
    use localsend::crypto::{hash, token};
    use std::path::Path;
    use std::time::{Duration, SystemTime};
    use webrtc::peer_connection::certificate::RTCCertificate;

    const ENTROPY: &[u8] = b"Oeca Nearby identity v1";

    pub fn load_or_create(path: Option<&Path>) -> (token::SigningTokenKey, bool) {
        let Some(path) = path else {
            return (token::generate_key(), false);
        };
        if let Some(key) = std::fs::read(path).ok().and_then(|b| load(&b)) {
            return (key, true);
        }
        // Missing: a new identity, kept. There but unusable (sealed for another
        // user / machine, corrupt, locked): one for this run, the file untouched
        let key = token::generate_key();
        if !absent(path) {
            return (key, false);
        }
        let saved = token::export_private_key(&key)
            .ok()
            .and_then(|pem| dpapi(pem.as_bytes(), true))
            .is_some_and(|blob| write_atomic(path, &blob).is_ok());
        (key, saved)
    }

    fn load(blob: &[u8]) -> Option<token::SigningTokenKey> {
        let mut pem = dpapi(blob, false)?;
        let key = std::str::from_utf8(&pem)
            .ok()
            .and_then(|s| token::parse_private_key(s).ok());
        pem.fill(0);
        key
    }

    fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::data_dir::replace(path, &path.with_extension("tmp"), data)
    }

    /// Whether `path` is certainly not there (only then may it be created).
    pub fn absent(path: &Path) -> bool {
        matches!(path.try_exists(), Ok(false))
    }

    pub fn fingerprint(key: &token::SigningTokenKey) -> String {
        let pem = token::export_public_key(key).unwrap_or_default();
        hash::sha256_hex(pem.as_bytes())[..16].to_string()
    }

    /// The DTLS certificate (ECDSA P-256, self-signed): its fingerprint is
    /// this device's identity to peers. Kept sealed like the signing key.
    pub fn dtls(path: Option<&Path>) -> Option<(RTCCertificate, bool)> {
        if let Some(cert) = path
            .and_then(unseal_file)
            .and_then(|pem| RTCCertificate::from_pem(&pem).ok())
        {
            return Some((cert, true));
        }
        let c = webrtc::dtls::crypto::Certificate::generate_self_signed(vec![
            "oeca-nearby".to_string(),
        ])
        .ok()?;
        // Peers pin the fingerprint, not a chain: a long validity, no rotation by date
        let cert = RTCCertificate::from_existing(
            c,
            SystemTime::now() + Duration::from_secs(20 * 365 * 86400),
        );
        let saved = path
            .filter(|p| absent(p))
            .is_some_and(|p| seal_file(p, cert.serialize_pem().as_bytes()).is_ok());
        Some((cert, saved))
    }

    pub fn seal_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
        let blob = dpapi(data, true).ok_or(std::io::ErrorKind::Other)?;
        write_atomic(path, &blob)
    }

    pub fn unseal_file(path: &Path) -> Option<String> {
        let mut bytes = dpapi(&std::fs::read(path).ok()?, false)?;
        let text = String::from_utf8(bytes.clone()).ok();
        bytes.fill(0);
        text
    }

    fn dpapi(data: &[u8], protect: bool) -> Option<Vec<u8>> {
        use windows::Win32::Foundation::{HLOCAL, LocalFree};
        use windows::Win32::Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        };
        let input = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(data.len()).ok()?,
            pbData: data.as_ptr() as *mut u8,
        };
        let entropy = CRYPT_INTEGER_BLOB {
            cbData: ENTROPY.len() as u32,
            pbData: ENTROPY.as_ptr() as *mut u8,
        };
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe {
            let r = if protect {
                CryptProtectData(
                    &input,
                    windows::core::PCWSTR::null(),
                    Some(&entropy),
                    None,
                    None,
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            } else {
                CryptUnprotectData(
                    &input,
                    None,
                    Some(&entropy),
                    None,
                    None,
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            };
            r.ok()?;
            let bytes = std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize);
            let v = bytes.to_vec();
            bytes.fill(0);
            let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
            Some(v)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn identity_persists_sealed_and_survives_restart() {
            let dir =
                crate::data_dir::temp_dir().join(format!("oeca-nearby-id-{}", std::process::id()));
            let path = dir.join("nearby-identity.bin");
            let (a, saved) = load_or_create(Some(&path));
            assert!(saved);
            let blob = std::fs::read(&path).unwrap();
            assert!(
                !String::from_utf8_lossy(&blob).contains("PRIVATE KEY"),
                "never in the clear"
            );
            let (b, loaded) = load_or_create(Some(&path));
            assert!(loaded);
            assert_eq!(fingerprint(&a), fingerprint(&b));
            // A corrupt file gives an identity for this run only, and is kept
            std::fs::write(&path, b"garbage").unwrap();
            let (c, saved) = load_or_create(Some(&path));
            assert_ne!(fingerprint(&a), fingerprint(&c));
            assert!(!saved);
            assert_eq!(std::fs::read(&path).unwrap(), b"garbage");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use localsend::crypto::hash;

    fn dto(name: &str, size: u64) -> FileDto {
        FileDto {
            id: name.into(),
            file_name: name.into(),
            size,
            file_type: "text/plain".into(),
            sha256: None,
            preview: None,
            metadata: None,
        }
    }

    #[test]
    fn backoff_grows_caps_and_respects_server_limits() {
        let mut b = Backoff::default();
        let waits: Vec<u64> = (0..12).map(|_| b.next(Retry::Lost).as_secs()).collect();
        assert_eq!(waits[..5], [1, 2, 4, 8, 16]);
        assert_eq!(*waits.last().unwrap(), BACKOFF_MAX.as_secs());
        let mut b = Backoff::default();
        assert_eq!(b.next(Retry::Rejected), REJECTED_MIN);
        assert_eq!(b.next(Retry::RateLimited), RATE_LIMITED_WAIT);
        assert_eq!(b.next(Retry::Requested), Duration::ZERO);
    }

    #[test]
    fn outcome_comes_from_the_files_not_the_last_status() {
        let row = |ok| FileRow {
            name: "a".into(),
            size: 1,
            done: 0,
            ok,
        };
        let t = |direction, rows| Transfer {
            files: rows,
            direction,
            id: TransferId::new(),
            claimed: Claimed {
                peer: peer("p"),
                fingerprint: None,
            },
            capability: Capability::Oeca,
            connection: Connection::Claimed,
            identity: None,
            stage: Stage::AwaitingPeer,
            decide_after: None,
            saved: Vec::new(),
        };
        assert_eq!(
            outcome(&t(Direction::Send, vec![row(Some(true))])),
            Stage::Done
        );
        assert!(
            matches!(
                outcome(&t(Direction::Send, vec![row(None)])),
                Stage::Failed(_)
            ),
            "only DECLINED is a decline"
        );
        assert!(matches!(
            outcome(&t(Direction::Send, vec![row(Some(true)), row(None)])),
            Stage::Failed(_)
        ));
        assert!(matches!(
            outcome(&t(Direction::Receive, vec![row(None)])),
            Stage::Failed(_)
        ));
        // Stopped mid-transfer by the receiver: not a decline
        let mut cut = t(Direction::Send, vec![row(None)]);
        cut.stage = Stage::Transferring;
        assert!(matches!(outcome(&cut), Stage::Failed(_)));
        assert!(matches!(
            outcome(&t(Direction::Receive, vec![])),
            Stage::Failed(_)
        ));
    }

    #[test]
    fn offers_over_the_limits_are_refused() {
        assert!(check_offer(&[]).is_err());
        assert!(check_offer(&[dto("a", 1)]).is_ok());
        let many: Vec<_> = (0..=MAX_FILES).map(|i| dto(&i.to_string(), 1)).collect();
        assert!(check_offer(&many).is_err());
        assert!(check_offer(&[dto("a", MAX_TOTAL_BYTES), dto("b", 1)]).is_err());
        assert!(check_offer(&[dto("a", u64::MAX), dto("b", u64::MAX)]).is_err());
    }

    #[test]
    fn a_list_with_ambiguous_or_oversized_entries_is_refused() {
        assert!(check_offer(&[dto("a", 1), dto("b", 1)]).is_ok());
        assert!(
            check_offer(&[dto("a", 1), dto("a", 4096)]).is_err(),
            "same id"
        );
        let with = |id: String, name: String| FileDto {
            id,
            file_name: name,
            ..dto("x", 1)
        };
        assert!(check_offer(&[with(String::new(), "a".into())]).is_err());
        assert!(check_offer(&[with("i".repeat(MAX_ID_BYTES + 1), "a".into())]).is_err());
        assert!(check_offer(&[with("0".into(), "n".repeat(MAX_NAME_BYTES + 1))]).is_err());
        // The longest name Windows has still goes
        assert!(check_offer(&[with("0".into(), "\u{20AC}".repeat(255))]).is_ok());
    }

    #[test]
    fn one_approval_is_each_file_once() {
        let files = [dto("a", 1), dto("b", 1)];
        let mut once = Once::new(&files);
        assert_eq!(once.take("b").unwrap(), 1);
        assert!(once.take("b").is_err(), "replayed");
        assert!(once.take("c").is_err(), "never approved");
        assert_eq!(once.take("a").unwrap(), 0);
        assert!(once.take("a").is_err(), "nothing is left");
    }

    #[test]
    fn a_drip_is_not_progress() {
        let mut pace = Pace::default();
        assert!(pace.counts(0), "accepting starts the clock");
        assert!(!pace.counts(1));
        assert!(!pace.counts(MIN_PROGRESS - 1));
        assert!(pace.counts(MIN_PROGRESS));
        assert!(!pace.counts(MIN_PROGRESS + 1), "from the last mark");
        let mut tiny_files = Pace::default();
        tiny_files.counts(0);
        assert!(tiny_files.counts(MIN_PROGRESS.div_ceil(FILE_PROGRESS) * FILE_PROGRESS));
    }

    #[test]
    fn refusals_are_limited_per_peer_and_in_all() {
        let (mut r, t0) = (Refusals::default(), Instant::now());
        for _ in 0..MAX_REFUSALS_PER_PEER {
            assert!(r.allow("greedy", t0));
        }
        assert!(!r.allow("greedy", t0));
        for i in 0..MAX_REFUSALS - MAX_REFUSALS_PER_PEER {
            assert!(r.allow(&format!("other {i}"), t0));
        }
        assert!(!r.allow("one more", t0));
        assert!(r.allow("greedy", t0 + REFUSAL_WINDOW), "window over");
    }

    #[test]
    fn a_server_s_wait_is_not_shortened() {
        for asked in [false, true] {
            assert!(!may_hurry(Retry::RateLimited, asked));
            assert!(may_hurry(Retry::Lost, asked));
        }
        assert!(!may_hurry(Retry::Rejected, false), "a flapping adapter");
        assert!(may_hurry(Retry::Rejected, true));
    }

    #[test]
    fn jitter_only_adds_up_to_a_quarter() {
        let wait = Duration::from_secs(8);
        for _ in 0..200 {
            let j = jittered(wait);
            assert!(j >= wait && j <= wait + wait / 4, "{j:?}");
        }
    }

    #[test]
    fn test_builds_never_use_the_public_signaling_server() {
        assert!(Config::for_oeca().is_none());
    }

    #[test]
    fn quitting_does_not_wait_for_a_stalled_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dir = crate::data_dir::temp_dir().join(format!("oeca-nearby-quit-{}", osv::random()));
        let cfg = Config {
            signaling_url: format!("ws://{}/v1/ws", listener.local_addr().unwrap()),
            stun: Vec::new(),
            alias: "Oeca".into(),
            data_dir: None,
            devices: Devices::open(Some(dir.clone())),
            receive_dir: dir,
        };
        let nearby = Nearby::start(cfg, || {}).unwrap();
        let _stalled = listener.accept().unwrap();
        let began = std::time::Instant::now();
        drop(nearby);
        assert!(began.elapsed() < SHUTDOWN_DEADLINE + Duration::from_millis(500));
    }

    fn io_test<T>(name: &str, f: impl AsyncFnOnce(PathBuf) -> T) -> T {
        let dir = crate::data_dir::temp_dir().join(format!("oeca-nearby-{name}-{}", osv::random()));
        std::fs::create_dir_all(&dir).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let out = rt.block_on(f(dir.clone()));
        rt.shutdown_timeout(Duration::from_secs(2));
        let _ = std::fs::remove_dir_all(&dir);
        out
    }

    #[test]
    fn a_received_file_exists_only_once_committed() {
        io_test("write", async |dir| {
            let (disk, written) = write_file::<Vec<u8>>(dir.clone(), "a.bin".into());
            disk.send(Disk::Chunk(b"abc".to_vec())).await.ok().unwrap();
            drop(disk); // cancelled
            assert!(written.await.unwrap().is_err());
            assert_eq!(
                std::fs::read_dir(&dir).unwrap().count(),
                0,
                "nothing partial"
            );

            let (disk, written) = write_file::<Vec<u8>>(dir.clone(), "a.bin".into());
            disk.send(Disk::Chunk(b"abc".to_vec())).await.ok().unwrap();
            disk.send(Disk::Commit).await.ok().unwrap();
            assert_eq!(
                std::fs::read(written.await.unwrap().unwrap()).unwrap(),
                b"abc"
            );
        });
    }

    #[test]
    fn a_failed_save_still_reads_the_file_to_its_end() {
        io_test("drain", async |dir| {
            let (tx, rx) = mpsc::channel(1);
            let mut file = RTCFile {
                file_id: "0".into(),
                binary_rx: rx,
            };
            let (progress, _seen) = watch::channel(0u64);
            let sender = tokio::spawn(async move {
                for _ in 0..40 {
                    tx.send(vec![7u8; 1000].into()).await.ok()?;
                }
                Some(())
            });
            let gone = dir.join("no such Downloads").join(RECEIVE_FOLDER);
            let dto = dto("a.bin", 40_000);
            let saved = save_one(&gone, &dto, &mut file, &progress, || {}, |_| {}).await;
            assert!(saved.is_err());
            assert_eq!(sender.await.unwrap(), Some(()), "every chunk was taken");
        });
    }

    #[test]
    fn a_file_to_send_must_keep_its_size() {
        io_test("read", async |dir| {
            let path = dir.join("f");
            std::fs::write(&path, b"12345").unwrap();
            for (size, good) in [(5, true), (4, false), (6, false)] {
                let (mut chunks, read) = read_file(path.clone(), size);
                while chunks.recv().await.is_some() {}
                assert_eq!(read.await.unwrap().is_ok(), good, "{size}");
            }
        });
    }

    #[test]
    fn untrusted_names_stay_in_the_folder() {
        for evil in [
            "../../x.exe",
            "..\\..\\x",
            "C:\\Windows\\x",
            "con",
            "a/b",
            "..",
            "",
            "   ",
        ] {
            let n = safe_name(evil);
            assert!(!n.contains(['/', '\\', ':']), "{evil:?} -> {n:?}");
            assert!(!n.is_empty() && n != "." && n != "..", "{evil:?} -> {n:?}");
        }
    }

    #[test]
    fn names_show_what_they_are() {
        for (untrusted, saved) in [
            // Bidi, invisible and control characters: what is shown is what is saved
            ("invoice\u{202E}fdp.exe", "invoicefdp.exe"),
            ("pay\u{200B}ment\u{2066}.pdf", "payment.pdf"),
            ("a\nb\r\tc.txt", "abc.txt"),
            ("\u{FEFF}x\u{3164}.txt", "x.txt"),
            ("  spaced.txt", "spaced.txt"),
            // Streams and separators
            ("file.txt:evil.exe", "file.txt_evil.exe"),
            ("data::$DATA", "data__$DATA"),
            ("../../x.exe", ".._.._x.exe"),
            ("C:\\Windows\\x", "C__Windows_x"),
            ("x.exe. . ", "x.exe_"),
            ("", "untitled"),
            ("   ", "untitled"),
            ("..", "_"),
            // Device names the core misses, as Windows reads them
            ("CONIN$", "_CONIN$"),
            ("conout$.log", "_conout$.log"),
            ("COM\u{00B9}.txt", "_COM\u{00B9}.txt"),
            ("LPT0", "_LPT0"),
            ("con .txt", "_con .txt"),
            ("nul.tar.gz", "_.tar.gz"),
            ("CON", "_"),
            // Not device names
            ("console.txt", "console.txt"),
            ("com10.txt", "com10.txt"),
            // Unicode kept (NFC)
            ("Re\u{0301}sume\u{0301}.pdf", "R\u{00E9}sum\u{00E9}.pdf"),
            ("日本語の書類.txt", "日本語の書類.txt"),
            ("\u{2764}\u{FE0F}.png", "\u{2764}\u{FE0F}.png"),
        ] {
            assert_eq!(safe_name(untrusted), saved, "{untrusted:?}");
        }
        assert!(dangerous(&safe_name("invoice\u{202E}fdp.exe")));
        assert!(dangerous(&safe_name("report.pdf.exe")));
        assert!(dangerous(&safe_name("file.txt:evil.exe")));
        for name in [
            "a.EXE",
            "b.lnk",
            "c.url",
            "d.library-ms",
            "e.ps1",
            "f.msi",
            "g.dll",
            "h.bat",
            "i.cmd",
            "j.scr",
            "k.com",
            "l.js",
            "m.hta",
            ".exe",
        ] {
            assert!(dangerous(name), "{name}");
        }
        for name in ["a.pdf", "b.exe.txt", "c", "d.jpeg", "exe", "e.exe_"] {
            assert!(!dangerous(name), "{name}");
        }
        let row = |name: &str| FileRow {
            name: name.into(),
            size: 0,
            done: 0,
            ok: None,
        };
        assert!(row("setup.msi").dangerous() && !row("photo.jpg").dangerous());
    }

    #[test]
    fn long_names_keep_their_extension_and_fit() {
        let units = |s: &str| s.encode_utf16().count();
        for (untrusted, ext) in [
            (format!("{}.pdf", "a".repeat(300)), ".pdf"),
            (format!("{}.docx", "日".repeat(300)), ".docx"),
            (format!("{}.exe", "\u{1F600}".repeat(200)), ".exe"),
            (format!("{}.tar.gz", "b".repeat(400)), ".gz"),
        ] {
            let n = safe_name(&untrusted);
            assert!(n.ends_with(ext), "{n}");
            assert!(
                units(&n) <= MAX_SAVED_NAME && n.len() <= 255,
                "{} units, {} bytes",
                units(&n),
                n.len()
            );
        }
        // The same cut as the browser's (nearby-web limits.test.ts)
        assert_eq!(
            safe_name(&format!("{}.exe", "\u{1F600}".repeat(200))),
            format!("{}.exe", "\u{1F600}".repeat(62))
        );
        // A 40-character "extension" is not one: the name is cut at the end
        let n = safe_name(&format!("{}.{}", "c".repeat(250), "x".repeat(40)));
        assert!(units(&n) <= MAX_SAVED_NAME);
        // The longest name kept still saves, with its collision suffix, in a
        // folder deep enough that the whole path passes 260 characters
        let dir = crate::data_dir::temp_dir()
            .join(format!("oeca-nearby-long-{}", std::process::id()))
            .join("d".repeat(120));
        std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
        assert!(dir.as_os_str().len() + MAX_SAVED_NAME > 260);
        let name = safe_name(&format!("{}.pdf", "\u{00E9}".repeat(300)));
        for expected in [
            name.clone(),
            format!("{} (2).pdf", name.trim_end_matches(".pdf")),
        ] {
            let mut p = Part::create(&dir, &name).unwrap();
            p.file.as_mut().unwrap().write_all(b"x").unwrap();
            assert_eq!(
                p.commit(&dir, &name, mark_internet_zone).unwrap(),
                dir.join(&expected)
            );
            assert!(units(&expected) <= 255);
        }
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn received_file_is_marked_and_never_replaces_an_existing_one() {
        let dir =
            crate::data_dir::temp_dir().join(format!("oeca-nearby-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"mine").unwrap();
        // Two files of the same name in one transfer, and one already there
        let mut saved = vec![];
        for body in [b"first", b"other"] {
            let mut p = Part::create(&dir, "a.txt").unwrap();
            p.file.as_mut().unwrap().write_all(body).unwrap();
            saved.push(p.commit(&dir, "a.txt", mark_internet_zone).unwrap());
        }
        assert_eq!(saved, [dir.join("a (2).txt"), dir.join("a (3).txt")]);
        assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), b"mine");
        assert_eq!(std::fs::read(&saved[1]).unwrap(), b"other");
        // Marked: Windows treats it as an Internet download
        for path in &saved {
            let mut stream = path.as_os_str().to_owned();
            stream.push(":Zone.Identifier");
            let mark = std::fs::read_to_string(PathBuf::from(stream)).unwrap();
            assert!(
                mark.contains("[ZoneTransfer]") && mark.contains("ZoneId=3"),
                "{mark}"
            );
            assert!(!mark.contains("Url"), "no address: {mark}");
        }
        // Nothing incomplete shows: parts carry their own suffix until the rename
        let p = Part::create(&dir, "b.bin").unwrap();
        assert!(p.path.to_string_lossy().ends_with(".oeca-part"));
        let part = p.path.clone();
        drop(p);
        assert!(!part.exists(), "an abandoned part leaves nothing behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unmarked_file_is_not_saved() {
        let dir = crate::data_dir::temp_dir()
            .join(format!("oeca-nearby-unmarked-{}", std::process::id()));
        let mut p = Part::create(&dir, "setup.exe").unwrap();
        p.file.as_mut().unwrap().write_all(b"MZ").unwrap();
        let part = p.path.clone();
        let failing = |_: &Path| Err(std::io::Error::other("no streams on this volume"));
        let e = p.commit(&dir, "setup.exe", failing).unwrap_err();
        assert!(e.to_string().contains("could not be marked"), "{e}");
        assert!(!dir.join("setup.exe").exists() && !part.exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "nothing left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file `secs` old (negative: from the future) by creation and last write.
    fn aged(path: &Path, created_secs: i64, written_secs: i64) {
        use std::os::windows::fs::FileTimesExt;
        let at = |s: i64| {
            let now = std::time::SystemTime::now();
            if s >= 0 {
                now - Duration::from_secs(s as u64)
            } else {
                now + Duration::from_secs((-s) as u64)
            }
        };
        let f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .unwrap();
        f.set_times(
            std::fs::FileTimes::new()
                .set_created(at(created_secs))
                .set_modified(at(written_secs)),
        )
        .unwrap();
    }

    fn junction(link: &Path, target: &Path) {
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
    }

    #[test]
    fn only_oeca_s_own_part_names_are_recognised() {
        for yes in [
            "a.pdf.0.oeca-part",
            "report.pdf.999.oeca-part",
            "x.7.oeca-part",
            "日本.txt.12.oeca-part",
        ] {
            assert!(is_own_part(yes), "{yes}");
        }
        for no in [
            "a.pdf.oeca-part",    // no number
            "a.pdf.01.oeca-part", // not as Part::create writes it
            "a.pdf.1000.oeca-part",
            "a.pdf.x.oeca-part",
            ".0.oeca-part",    // no name
            "con.0.oeca-part", // a name Part::create never gets (safe_name gives "_")
            "a\u{202E}b.0.oeca-part",
            "a:b.0.oeca-part",
            "a.pdf.0.oeca-part.txt",
            "a.pdf.0.OECA-PART",
            "a.pdf",
        ] {
            assert!(!is_own_part(no), "{no}");
        }
    }

    #[test]
    fn crash_leftovers_go_and_nothing_else() {
        let day = 24 * 3600 + 60;
        let root =
            crate::data_dir::temp_dir().join(format!("oeca-nearby-sweep-{}", std::process::id()));
        let dir = root.join(RECEIVE_FOLDER);
        let outside = root.join("outside");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        // The one that goes: Oeca's name, a plain file, a day old by both times, closed
        aged(&dir.join("old.pdf.0.oeca-part"), day, day);
        // Kept
        aged(&dir.join("recent.pdf.0.oeca-part"), 3600, 3600);
        aged(&dir.join("rewritten.pdf.1.oeca-part"), day, 60);
        aged(&dir.join("recreated.pdf.2.oeca-part"), 60, day);
        aged(&dir.join("future.pdf.3.oeca-part"), -3600, -3600);
        aged(&dir.join("done.pdf"), day, day);
        aged(&dir.join("unrelated.txt"), day, day);
        aged(&dir.join("a.pdf.01.oeca-part"), day, day);
        std::fs::create_dir(dir.join("folder.0.oeca-part")).unwrap();
        aged(
            &dir.join("folder.0.oeca-part").join("inside.0.oeca-part"),
            day,
            day,
        );
        // A junction with Oeca's name, to a folder with a part file: not followed
        aged(&outside.join("victim.0.oeca-part"), day, day);
        junction(&dir.join("junction.0.oeca-part"), &outside);
        // A second name of a file elsewhere
        aged(&outside.join("linked.bin"), day, day);
        std::fs::hard_link(
            outside.join("linked.bin"),
            dir.join("linked.bin.0.oeca-part"),
        )
        .unwrap();
        // In use by a transfer (open), though a day old
        let active = Part::create(&dir, "active.bin").unwrap();
        aged(&active.path, day, day);
        // Not deletable (read-only): kept, and not made deletable
        let ro = dir.join("readonly.pdf.0.oeca-part");
        aged(&ro, day, day);
        let mut perms = std::fs::metadata(&ro).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&ro, perms.clone()).unwrap();

        let before: HashSet<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(sweep_parts(&dir, std::time::SystemTime::now()), 1);
        let after: HashSet<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        let gone: Vec<_> = before.difference(&after).collect();
        assert_eq!(gone, [&std::ffi::OsString::from("old.pdf.0.oeca-part")]);
        assert!(outside.join("victim.0.oeca-part").exists() && outside.join("linked.bin").exists());
        assert!(
            dir.join("folder.0.oeca-part")
                .join("inside.0.oeca-part")
                .exists(),
            "nothing below the folder"
        );
        assert!(
            std::fs::metadata(&ro).unwrap().permissions().readonly(),
            "left as it was"
        );
        drop(active);

        // A receive folder that is itself a junction is not swept at all
        let other = root.join("other");
        std::fs::create_dir_all(&other).unwrap();
        aged(&other.join("old.pdf.0.oeca-part"), day, day);
        let linked = root.join("linked-folder");
        junction(&linked, &other);
        assert_eq!(sweep_parts(&linked, std::time::SystemTime::now()), 0);
        assert!(other.join("old.pdf.0.oeca-part").exists());
        // Missing: nothing to do
        assert_eq!(
            sweep_parts(&root.join("missing"), std::time::SystemTime::now()),
            0
        );

        // Windows: clears the read-only attribute this test set (the lint is about Unix modes)
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&ro, perms).unwrap();
        std::fs::remove_dir(dir.join("junction.0.oeca-part")).unwrap();
        std::fs::remove_dir(&linked).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn nearby_sweeps_when_it_starts() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dir =
            crate::data_dir::temp_dir().join(format!("oeca-nearby-startsweep-{}", osv::random()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("left.bin.0.oeca-part");
        aged(&old, 24 * 3600 + 60, 24 * 3600 + 60);
        let cfg = Config {
            signaling_url: format!("ws://{}/v1/ws", listener.local_addr().unwrap()),
            stun: Vec::new(),
            alias: "Oeca".into(),
            data_dir: None,
            devices: Devices::open(Some(dir.clone())),
            receive_dir: dir.clone(),
        };
        let nearby = Nearby::start(cfg, || {}).unwrap();
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while old.exists() && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(nearby);
        assert!(!old.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_receive_folder_is_made_and_never_a_link() {
        let root =
            crate::data_dir::temp_dir().join(format!("oeca-nearby-folder-{}", std::process::id()));
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        // Made when missing, in an existing Downloads; never a missing Downloads
        let fresh = root.join("Downloads").join(RECEIVE_FOLDER);
        assert!(Part::create(&fresh, "a").is_err());
        std::fs::create_dir(root.join("Downloads")).unwrap();
        drop(Part::create(&fresh, "a").unwrap());
        assert!(fresh.is_dir());
        // A junction (no privilege needed) pointing elsewhere: refused, nothing written there
        let junction = root.join("junction");
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&elsewhere)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
        assert!(Part::create(&junction, "a.txt").is_err());
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
        // A file where the folder should be (or a folder that cannot be made)
        let file = root.join("not-a-folder");
        std::fs::write(&file, b"x").unwrap();
        assert!(Part::create(&file, "a.txt").is_err());
        assert!(Part::create(&file.join("deeper"), "a.txt").is_err());
        std::fs::remove_dir(&junction).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_real_networks_are_offered_to_peers() {
        let a = |name: &str, if_type: u32, up: bool, gateway: bool| net::Adapter {
            name: name.into(),
            if_type,
            up,
            gateway,
            addrs: vec![],
        };
        for (adapter, offered) in [
            (a("Wi-Fi", 71, true, true), true),
            (a("Ethernet", 6, true, true), true),
            (a("Ethernet, cable out", 6, false, true), false),
            (a("vEthernet (WSL)", 6, true, false), false),
            (a("vEthernet (Default Switch)", 6, true, false), false),
            (a("VirtualBox Host-Only Network", 6, true, false), false),
            (a("WireGuard tunnel", 53, true, true), false),
            (a("Teredo / IP-HTTPS tunnel", 131, true, true), false),
            (a("PPP VPN", 23, true, true), false),
            (a("Loopback", 24, true, false), false),
        ] {
            assert_eq!(net::offered(&adapter), offered, "{}", adapter.name);
        }
        // This computer, for the record: loopback is never offered
        for x in net::adapters() {
            println!(
                "{:<40} type {:>3} up={} gateway={} offered={}",
                x.name,
                x.if_type,
                x.up,
                x.gateway,
                net::offered(&x)
            );
            assert!(
                !(net::offered(&x) && x.addrs.iter().any(|ip| ip.is_loopback())),
                "{}",
                x.name
            );
        }
        assert!(!net::may_offer("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn the_privacy_notice_says_what_the_code_does() {
        let n = privacy_notice();
        // Line continuations drop the next line's indent: no sentences run together
        let joined = n
            .as_bytes()
            .windows(3)
            .any(|w| w[0].is_ascii_lowercase() && w[1] == b'.' && w[2].is_ascii_uppercase());
        assert!(!joined, "sentences run together in: {n}");
        for fact in [
            "public.localsend.org",
            "stun.localsend.org",
            &format!("\"{DEFAULT_ALIAS}\""),
            &format!("Downloads\\{RECEIVE_FOLDER}"),
            "public IP address",
            "the device type",
            "passes on every connection request",
            "the same until you reset it",
            "can include strangers",
            "not those of virtual adapters or most VPNs",
            "A VPN, and whether your devices connect over IPv4 or IPv6",
            "public IPv4 address or IPv6 /64 network",
            "doesn't promise that only devices on your Wi-Fi",
            "before you approve anything",
            "Nothing is received without your approval",
            "nothing is stored online. Nothing is received",
            "marked as downloaded from the internet",
            "off until you turn it on",
            "stops when Oeca closes",
        ] {
            assert!(n.contains(fact), "missing {fact:?}");
        }
        // Each claim, checked against the code it describes
        assert!(
            SIGNALING_URL.contains("public.localsend.org") && STUN.contains("stun.localsend.org")
        );
        assert!(
            !crate::space::OecaSettings::default().nearby,
            "off by default"
        );
        let saved = include_str!("nearby.rs");
        assert!(
            saved.contains("part.commit(&dir, &name, mark_internet_zone)"),
            "every save is marked"
        );
        assert!(
            saved.contains("rtc::set_ip_filter(Some(Arc::new(net::may_offer)))"),
            "only Ethernet or Wi-Fi addresses are offered (vendor patch 17)"
        );
        assert!(
            saved.contains("device_model: Some(\"Windows\".into())"),
            "the device type, and no more"
        );
    }

    /// The web app's upstream container recipe was unsafe (unpinned images,
    /// unlocked install, on-demand TLS) and was removed: none may come back
    /// unnoticed. Nearby web is deployed as static files (nearby-web/OECA.md).
    #[test]
    fn the_web_app_has_no_container_recipe() {
        let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("nearby-web");
        for name in [
            "Containerfile",
            "Dockerfile",
            "docker-compose.yml",
            "compose.yaml",
        ] {
            assert!(!web.join(name).exists(), "nearby-web/{name}");
        }
        let readme = std::fs::read_to_string(web.join("README.md")).unwrap();
        assert!(
            !readme.contains("docker build"),
            "README still builds a container"
        );
    }

    #[test]
    fn the_default_alias_tells_nothing_about_this_computer() {
        assert_eq!(DEFAULT_ALIAS, "Oeca");
        // Nowhere is the Windows computer name read for Nearby
        let source = include_str!("nearby.rs");
        let reads = ["COMPUTER", "NAME"].concat();
        assert_eq!(source.matches(&reads).count(), 0);
        assert!(!source.contains(&["GetComputer", "Name"].concat()));
    }

    #[test]
    fn resetting_the_identity_deletes_only_the_identity() {
        let dir =
            crate::data_dir::temp_dir().join(format!("oeca-nearby-reset-{}", std::process::id()));
        let (key, saved) = identity::load_or_create(Some(&dir.join(IDENTITY_FILES[0])));
        let (cert, _) = identity::dtls(Some(&dir.join(IDENTITY_FILES[1]))).unwrap();
        assert!(saved);
        let devices = Devices::open(Some(dir.clone()));
        devices.pin(&authenticated(7), "Phone", "Phone").unwrap();
        std::fs::write(dir.join("settings.ini"), b"unrelated").unwrap();
        reset_identity(&dir).unwrap();
        for name in IDENTITY_FILES {
            assert!(!dir.join(name).exists(), "{name}");
        }
        assert_eq!(
            std::fs::read(dir.join("settings.ini")).unwrap(),
            b"unrelated"
        );
        // The other devices are kept: they did not change
        assert_eq!(Devices::open(Some(dir.clone())).list().unwrap().len(), 1);
        // The next start is a new device to everyone
        let (key2, _) = identity::load_or_create(Some(&dir.join(IDENTITY_FILES[0])));
        let (cert2, _) = identity::dtls(Some(&dir.join(IDENTITY_FILES[1]))).unwrap();
        assert_ne!(identity::fingerprint(&key), identity::fingerprint(&key2));
        let value = |c: &RTCCertificate| {
            c.get_fingerprints()
                .into_iter()
                .map(|f| f.value)
                .collect::<Vec<_>>()
        };
        assert_ne!(value(&cert), value(&cert2));
        // Nothing there: nothing to do
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        reset_identity(&empty).unwrap();
        // One file cannot be deleted (a folder by that name): reported, the other still goes
        let stuck = dir.join("stuck");
        std::fs::create_dir_all(stuck.join(IDENTITY_FILES[0])).unwrap();
        std::fs::write(stuck.join(IDENTITY_FILES[1]), b"x").unwrap();
        assert!(reset_identity(&stuck).is_err(), "never reported as done");
        assert!(
            !stuck.join(IDENTITY_FILES[1]).exists(),
            "the other was still deleted"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Shared with nearby-web/app/services/trust.test.ts (and checked by hand in
    /// Python): the commitment of R_O and the code of (FP_A, FP_B, R_O, R_A).
    const FP_A: &str = "sha-256 e0:79:e1:b9:10:34:7e:e7:3b:75:0c:ab:75:bc:b7:b0:0f:27:fe:fc:91:79:a8:c5:7d:14:1b:df:78:ae:26:e6";
    const FP_B: &str = "sha-256 58:41:72:c7:fe:91:8f:d4:b1:90:80:2e:16:13:7f:da:99:56:f8:5e:58:b4:d6:f3:7d:fa:ea:30:6c:5e:a4:9b";
    const R_O: &str = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA";
    const R_A: &str = "ISIjJCUmJygpKissLS4vMDEyMzQ1Njc4OTo7PD0-P0A";

    fn fp(text: &str) -> Option<Fingerprint> {
        Fingerprint::parse(text)
    }

    #[test]
    fn verification_vector_matches_the_browser() {
        assert_eq!(
            osv::commit(R_O),
            "VPyCJA6QvLU-7r7ZRl4u0bm4tVpOoHOx9FYAtdCFaaU"
        );
        let code = osv::code(FP_A, FP_B, R_O, R_A);
        assert_eq!(code, "346 672");
        // Each side passes its own fingerprint first: the same code
        assert_eq!(code, osv::code(FP_B, FP_A, R_O, R_A));
        // Any other random, or the roles swapped, gives another code
        assert_ne!(code, osv::code(FP_A, FP_B, R_A, R_O));
        assert_ne!(code, osv::code(FP_A, FP_B, R_O, R_O));
        assert_eq!(osv::random().len(), 43);
        assert_ne!(osv::random(), osv::random());
    }

    #[test]
    fn extension_lines_are_read_strictly() {
        let sdp = format!(
            "v=0\r\na=x-oeca-nearby-commit:{R_O}\r\na=x-oeca-nearby-nonce: {R_A} \r\nm=application 9\r\n"
        );
        assert_eq!(osv::attr(&sdp, osv::COMMIT).as_deref(), Some(R_O));
        // Padded with spaces: not the one grammar (see the shared vectors below)
        assert_eq!(osv::attr(&sdp, osv::NONCE), None);
        // Missing, short or not base64: none
        assert_eq!(osv::attr("v=0\r\n", osv::COMMIT), None);
        assert_eq!(osv::attr("a=x-oeca-nearby-commit:abc", osv::COMMIT), None);
        assert_eq!(osv::attr("a=x-oeca-nearby-commit:!!!!", osv::COMMIT), None);
    }

    /// Shared with nearby-web/app/services/trust.test.ts ("extension lines"): one grammar.
    #[test]
    fn extension_lines_share_one_grammar_with_the_browser() {
        let n = osv::COMMIT;
        let short = &R_O[..42];
        let plus = format!("+{}", &R_O[1..]);
        let trail = format!("{}B", &R_O[..42]);
        for (sdp, ok, why) in [
            (
                format!("v=0\r\na={n}:{R_O}\r\nm=application 9\r\n"),
                true,
                "CRLF, before m=",
            ),
            (
                format!("v=0\na={n}:{R_O}\nm=application 9\n"),
                true,
                "LF only",
            ),
            (format!("v=0\r\na={n}:{R_O}\r\n"), true, "no media section"),
            (
                format!("v=0\r\na={n}:{R_O} \r\nm=x\r\n"),
                false,
                "trailing space",
            ),
            (format!("v=0\r\n a={n}:{R_O}\r\n"), false, "leading space"),
            (format!("v=0\r\n\u{FEFF}a={n}:{R_O}\r\n"), false, "BOM"),
            (format!("v=0\r\na={n}:{R_O}\u{85}\r\n"), false, "NEL"),
            (
                format!("v=0\r\na={n}:{R_O}\r\na={n}:{R_O}\r\nm=x\r\n"),
                false,
                "twice",
            ),
            (
                format!("v=0\r\nm=x\r\na={n}:{R_O}\r\n"),
                false,
                "media level only",
            ),
            (
                format!("v=0\r\na={n}:{R_O}\r\nm=x\r\na={n}:{R_O}\r\n"),
                false,
                "session and media",
            ),
            (format!("v=0\r\na={n}:{R_O}=\r\n"), false, "padded"),
            (
                format!("v=0\r\na={n}:{plus}\r\n"),
                false,
                "standard alphabet",
            ),
            (
                format!("v=0\r\na={n}:{trail}\r\n"),
                false,
                "non-canonical last character",
            ),
            (format!("v=0\r\na={n}:{R_O}\rx\r\n"), false, "lone CR"),
            (format!("v=0\r\na={n}:{short}\r\n"), false, "42 characters"),
            (format!("v=0\r\na={n}:{R_O}A\r\n"), false, "44 characters"),
            (format!("v=0\r\nA={n}:{R_O}\r\n"), false, "upper-case a="),
            (format!("v=0\r\na={n}:{R_O}\r\r\n"), false, "two CRs"),
        ] {
            let want = ok.then_some(R_O);
            assert_eq!(osv::attr(&sdp, n).as_deref(), want, "{why}");
        }
    }

    /// Shared with nearby-web/app/services/trust.test.ts ("fingerprints").
    #[test]
    fn fingerprints_share_one_grammar_with_the_browser() {
        let (fp, hex) = (FP_A, &FP_A[8..]);
        let upper = FP_A.to_ascii_uppercase();
        let short = &FP_A[8..FP_A.len() - 3];
        for (text, ok, why) in [
            (fp.to_string(), true, "canonical"),
            (upper.clone(), true, "upper case"),
            (format!("sha-256\t{hex}"), true, "tab"),
            (format!("sha-256  {hex}"), true, "two spaces"),
            (format!(" {fp} "), true, "spaces around"),
            (format!("sha-256\u{85}{hex}"), false, "NEL"),
            (format!("sha-256\u{A0}{hex}"), false, "no-break space"),
            (format!("\u{FEFF}{fp}"), false, "BOM"),
            (format!("sha-1 {hex}"), false, "another digest"),
            (format!("sha-256 {short}"), false, "31 bytes"),
        ] {
            assert_eq!(fp_text(&text), ok.then(|| FP_A.to_string()), "{why}");
        }
    }

    fn fp_text(text: &str) -> Option<String> {
        Fingerprint::parse(text).map(|f| f.as_str().to_string())
    }

    #[test]
    fn a_malformed_or_busy_oeca_peer_is_never_a_stock_peer() {
        let read = |line: &str| {
            let text = format!("v=0\r\na=fingerprint:{FP_A}\r\n{line}\r\n");
            Said::read(&signaled_sdp(&text).unwrap(), osv::NONCE)
        };
        let stock = read("a=mid:0");
        assert_eq!(stock.capability, Capability::Stock);
        assert!(matches!(stock.usable(), Ok((_, None))));
        for line in [
            "a=x-oeca-nearby-nonce:short",
            "a=X-OECA-NEARBY-NONCE:abc",
            "a=x-oeca-nearby-v2:hello",
        ] {
            let said = read(line);
            assert_eq!(said.capability, Capability::Oeca, "{line}");
            assert!(
                matches!(said.usable(), Err(Failure::Authentication(_))),
                "{line}"
            );
        }
        let busy = read("a=x-oeca-nearby-busy:1");
        assert!(busy.busy && busy.capability == Capability::Oeca);
        assert!(busy.usable().is_err(), "busy proves nothing");
    }

    #[test]
    fn fingerprints_are_read_from_signaled_sdps_and_normalized() {
        let upper = FP_A.replace("sha-256", "SHA-256").to_ascii_uppercase();
        let sdp =
            format!("v=0\r\na=group:BUNDLE 0\r\na=fingerprint:{upper}\r\nm=application 9\r\n");
        assert_eq!(fingerprint_from_sdp(&sdp), fp(FP_A));
        let text = sdp_text(&signaled_sdp(&sdp).unwrap()).unwrap();
        assert_eq!(fingerprint_from_sdp(&text), fp(FP_A));
        // Weaker digests, truncated values and garbage are refused
        assert_eq!(fingerprint_from_sdp("a=fingerprint:sha-1 ab:cd"), None);
        assert_eq!(fingerprint_from_sdp("a=fingerprint:sha-256 ab:cd"), None);
        assert_eq!(fingerprint_from_sdp("v=0\r\n"), None);
        assert_eq!(sdp_text("not base64 !"), None);
    }

    /// Evidence for the extension's design: an unknown session-level SDP
    /// attribute (how an Oeca peer announces the commitment) is kept, not
    /// rejected, by webrtc-rs's parser (Chromium accepts it too).
    #[test]
    fn unknown_sdp_attributes_are_tolerated() {
        let sdp = "v=0
o=- 1 2 IN IP4 127.0.0.1
s=-
t=0 0
a=x-oeca-commit:v1 abc
a=fingerprint:sha-256 {}
m=application 9 UDP/DTLS/SCTP webrtc-datachannel
c=IN IP4 0.0.0.0
a=mid:0
a=sctp-port:5000
";
        let sdp = sdp.replace("{}", &FP_A[8..]);
        let parsed = webrtc::sdp::description::session::SessionDescription::unmarshal(
            &mut std::io::Cursor::new(sdp.as_bytes()),
        )
        .unwrap();
        assert_eq!(
            parsed.attribute("x-oeca-commit").map(String::as_str),
            Some("v1 abc")
        );
        assert_eq!(fingerprint_from_sdp(&sdp), fp(FP_A));
    }

    /// The fingerprint we check must be the only one a WebRTC stack could use.
    #[test]
    fn ambiguous_fingerprints_are_refused() {
        for sdp in [
            format!("v=0\r\na=fingerprint:{FP_A}\r\nm=application 9\r\na=fingerprint:{FP_B}\r\n"),
            format!("m=application 9\r\na=fingerprint:{FP_A}\r\na=fingerprint:{FP_B}\r\n"),
            format!("v=0\r\n a=fingerprint:{FP_A}\r\n"),
            format!("a=fingerprint:{FP_A}\r\na=Fingerprint:{FP_B}\r\n"),
            format!("a=fingerprint:{FP_A}\ra=fingerprint:{FP_B}\r\n"),
        ] {
            assert_eq!(fingerprint_from_sdp(&sdp), None, "{sdp:?}");
        }
        let twice = format!("a=fingerprint:{FP_A}\r\nm=application 9\r\na=fingerprint:{FP_A}\r\n");
        assert_eq!(fingerprint_from_sdp(&twice), fp(FP_A));
        // Over the limit: refused, not cut (the core would read past the cut)
        let long = format!(
            "a=fingerprint:{FP_A}\r\n{}",
            "x".repeat(MAX_SDP_BYTES as usize)
        );
        assert_eq!(sdp_text(&signaled_sdp(&long).unwrap()), None);
    }

    #[test]
    fn dtls_identity_is_persistent() {
        let dir =
            crate::data_dir::temp_dir().join(format!("oeca-nearby-dtls-{}", std::process::id()));
        let path = dir.join("nearby-dtls.bin");
        let (a, saved) = identity::dtls(Some(&path)).unwrap();
        assert!(saved);
        let (b, loaded) = identity::dtls(Some(&path)).unwrap();
        assert!(loaded);
        assert!(cert_fingerprint(&a).is_some());
        assert_eq!(cert_fingerprint(&a), cert_fingerprint(&b));
        assert!(!String::from_utf8_lossy(&std::fs::read(&path).unwrap()).contains("PRIVATE KEY"));
        std::fs::write(&path, b"garbage").unwrap();
        assert!(!identity::dtls(Some(&path)).unwrap().1, "not saved over");
        assert_eq!(std::fs::read(&path).unwrap(), b"garbage");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- The decision API, on a board with scripted peers: no network

    use proof::tests::{authenticated, connected, fingerprint};
    use transfer::ARMING as ARMED;

    struct Rig {
        dir: PathBuf,
        nearby: Nearby,
        commands: mpsc::Receiver<Command>,
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    impl Rig {
        fn new() -> Self {
            let dir =
                crate::data_dir::temp_dir().join(format!("oeca-nearby-rig-{}", osv::random()));
            std::fs::create_dir_all(&dir).unwrap();
            let shared = Shared::new(Devices::open(Some(dir.clone())), Box::new(|| {}));
            let (cmd, commands) = mpsc::channel(16);
            let nearby = Nearby {
                cmd: Some(cmd),
                shared,
                worker: None,
                network: None,
            };
            Self {
                dir,
                nearby,
                commands,
            }
        }

        fn trust(&self, seed: usize, name: &str, alias: &str) {
            self.devices()
                .pin(&authenticated(seed), name, alias)
                .unwrap();
        }

        fn devices(&self) -> &Devices {
            self.nearby.devices()
        }

        fn board<T>(&self, f: impl FnOnce(&mut Board) -> T) -> T {
            f(&mut lock(&self.nearby.shared.board))
        }

        /// An attempt up to where signaling has spoken.
        fn connecting(
            &self,
            direction: Direction,
            alias: &str,
            cap: Capability,
            seed: usize,
        ) -> TransferId {
            self.board(|b| {
                let id = b.begin(direction, peer(alias));
                b.claim(
                    &id,
                    Some(ClaimedFingerprint::from_signaling(fingerprint(seed))),
                    cap,
                );
                id
            })
        }

        fn oeca(&self, direction: Direction, alias: &str, seed: usize) -> (TransferId, Gate) {
            let id = self.connecting(direction, alias, Capability::Oeca, seed);
            let gate = self
                .nearby
                .shared
                .admit(&id, Proof::Oeca(authenticated(seed)))
                .unwrap();
            (id, gate)
        }

        fn stock(
            &self,
            direction: Direction,
            alias: &str,
            seed: usize,
        ) -> (TransferId, Result<Gate, Failure>) {
            let id = self.connecting(direction, alias, Capability::Stock, seed);
            let admitted = self.nearby.shared.admit(&id, Proof::Stock(connected(seed)));
            (id, admitted)
        }

        /// The UI showed it, and the arming delay has passed.
        fn show(&self) -> std::time::Instant {
            let now = std::time::Instant::now();
            self.board(|b| b.snapshot(now - ARMED));
            now
        }

        fn transfer(&self) -> Transfer {
            self.board(|b| b.view.transfer.clone().unwrap())
        }
    }

    fn peer(alias: &str) -> Peer {
        Peer {
            id: format!("id of {alias}"),
            alias: alias.into(),
            model: None,
            kind: "desktop",
        }
    }

    fn rows() -> Vec<FileRow> {
        vec![FileRow {
            name: "a.txt".into(),
            size: 1,
            done: 0,
            ok: None,
        }]
    }

    fn no_decision_works(r: &Rig, id: &TransferId, why: Refused) {
        let (now, d) = (r.show(), r.devices());
        r.board(|b| {
            assert_eq!(b.verify(id, "Name", d, now), Err(why.clone()));
            assert_eq!(b.confirm_name(id, d, now), Err(why.clone()));
            assert_eq!(b.respond(id, true, d, now), Err(why.clone()));
            assert_eq!(b.respond(id, false, d, now), Err(why.clone()));
            assert_eq!(b.allow_once(id, d, now), Err(why));
        });
    }

    #[test]
    fn a_click_meant_for_a_replaced_transfer_does_nothing() {
        let r = Rig::new();
        let (shown, gate) = r.oeca(Direction::Receive, "Alice", 1);
        r.show();
        drop(gate);
        // The first one ends and another takes its place under the cursor
        r.board(|b| b.end(&shown, Stage::Cancelled));
        let (attacker, mut gate) = r.oeca(Direction::Receive, "Mallory", 2);
        assert_ne!(shown, attacker);
        no_decision_works(&r, &shown, Refused::NotCurrent);
        assert_eq!(r.board(|b| b.cancel(&shown)), Err(Refused::NotCurrent));
        assert_eq!(gate.as_mut().unwrap().try_recv().ok(), None);
        assert!(r.devices().list().unwrap().is_empty(), "nobody was pinned");
        assert_eq!(r.transfer().stage, Stage::Verification);
    }

    #[test]
    fn a_decision_waits_for_the_question_to_be_seen() {
        let r = Rig::new();
        let (id, _gate) = r.oeca(Direction::Receive, "Alice", 1);
        let (d, t0) = (r.devices(), std::time::Instant::now());
        r.board(|b| {
            assert_eq!(
                b.verify(&id, "Alice", d, t0 + ARMED),
                Err(Refused::NotYet),
                "never shown"
            );
            let shown = b.snapshot(t0).transfer.unwrap();
            assert_eq!(shown.decide_after, Some(t0 + ARMED));
            assert_eq!(
                b.verify(&id, "Alice", d, t0 + ARMED / 2),
                Err(Refused::NotYet)
            );
            b.verify(&id, "Alice", d, t0 + ARMED).unwrap();
        });
        // The next question starts its own delay
        let _approval = r.board(|b| b.offer(&id, rows()));
        r.board(|b| {
            assert_eq!(
                b.respond(&id, true, d, t0 + ARMED * 9),
                Err(Refused::NotYet)
            );
        });
    }

    #[test]
    fn nothing_can_be_decided_while_connecting() {
        let r = Rig::new();
        r.trust(1, "Phone", "Phone");
        for cap in [Capability::Oeca, Capability::Stock] {
            let id = r.connecting(Direction::Receive, "Phone", cap, 1);
            no_decision_works(&r, &id, Refused::WrongStage);
            assert_eq!(r.board(|b| b.offer(&id, rows())).map(|_| ()), None);
            assert!(r.transfer().files.is_empty(), "no list before admission");
        }
    }

    #[test]
    fn acceptance_exists_only_with_the_list_on_screen() {
        let r = Rig::new();
        r.trust(1, "Phone", "Phone");
        let (id, gate) = r.oeca(Direction::Receive, "Phone", 1);
        assert!(gate.is_none());
        assert_eq!(r.transfer().stage, Stage::AwaitingOffer);
        no_decision_works(&r, &id, Refused::WrongStage);
        let mut gate = r.board(|b| b.offer(&id, rows())).unwrap();
        let (now, d) = (r.show(), r.devices());
        assert_eq!(r.transfer().files, rows());
        r.board(|b| {
            assert_eq!(b.verify(&id, "x", d, now), Err(Refused::WrongStage));
            assert_eq!(b.allow_once(&id, d, now), Err(Refused::NotEligible));
            b.respond(&id, true, d, now).unwrap();
            assert_eq!(
                b.respond(&id, true, d, now),
                Err(Refused::WrongStage),
                "once"
            );
        });
        assert_eq!(gate.try_recv(), Ok(true));
    }

    #[test]
    fn verifying_is_not_accepting() {
        let r = Rig::new();
        let (id, gate) = r.oeca(Direction::Receive, "Alice", 1);
        let (now, d) = (r.show(), r.devices());
        r.board(|b| {
            assert_eq!(b.respond(&id, true, d, now), Err(Refused::WrongStage));
            b.verify(&id, "Alice", d, now).unwrap();
        });
        assert_eq!(gate.unwrap().try_recv(), Ok(true));
        assert_eq!(r.transfer().stage, Stage::AwaitingOffer);
    }

    #[test]
    fn a_late_decision_after_the_session_ended_does_nothing() {
        let r = Rig::new();
        let (id, mut gate) = r.oeca(Direction::Send, "Alice", 1);
        r.show();
        r.board(|b| b.end_session(Stage::Failed(Failure::Other("lost".into()))));
        no_decision_works(&r, &id, Refused::NotCurrent);
        assert!(
            gate.as_mut().unwrap().try_recv().is_err(),
            "the gate died with it"
        );
        assert!(r.devices().list().unwrap().is_empty());
    }

    #[test]
    fn losing_signaling_ends_only_what_needs_it() {
        let r = Rig::new();
        r.trust(1, "Phone", "Phone");
        let (id, _gate) = r.oeca(Direction::Send, "Phone", 1);
        let lost = || Stage::Failed(Failure::Other("lost".into()));
        r.board(|b| b.leave_room(lost()));
        assert!(r.board(|b| b.is_live(&id)), "connected: it needs no room");
        assert!(r.board(|b| b.view.me.is_none() && b.view.peers.is_empty()));
        r.board(|b| b.end_session(Stage::Cancelled));
        let offered = r.board(|b| b.begin(Direction::Send, peer("Phone")));
        r.board(|b| b.leave_room(lost()));
        assert!(!r.board(|b| b.is_live(&offered)), "no answer can come");
    }

    /// Runs `drive` on a core that reports `statuses` and then ends with `ends`.
    fn driven(statuses: &[RTCStatus], ends: Result<(), String>) -> Stage {
        let r = Rig::new();
        r.trust(1, "Phone", "Phone");
        let (id, _) = r.oeca(Direction::Send, "Phone", 1);
        assert_eq!(r.transfer().stage, Stage::AwaitingPeer);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (status_tx, status) = mpsc::channel(8);
            for st in statuses {
                let copy = match st {
                    RTCStatus::Declined => RTCStatus::Declined,
                    RTCStatus::Finished => RTCStatus::Finished,
                    _ => unreachable!(),
                };
                status_tx.send(copy).await.unwrap();
            }
            // The statuses are still queued when the core's future is ready
            let core = async move {
                drop(status_tx);
                ends
            };
            let (_errors_tx, errors) = mpsc::channel::<String>(1);
            let (_progress_tx, progress) = watch::channel(0u64);
            let (_asked_tx, asked) = watch::channel(DECIDING);
            let signals = Signals {
                status,
                errors,
                progress,
                asked,
                connected: None,
            };
            let side = std::future::pending::<Result<(), Failure>>();
            let shared = &r.nearby.shared;
            let result = drive(core, side, signals, shared, &id, Duration::from_secs(5)).await;
            finish(shared, &id, result);
        });
        r.transfer().stage
    }

    #[test]
    fn only_the_protocol_s_declined_is_a_decline() {
        for _ in 0..20 {
            assert_eq!(driven(&[RTCStatus::Declined], Ok(())), Stage::Declined);
            assert_eq!(
                driven(&[RTCStatus::Declined, RTCStatus::Finished], Ok(())),
                Stage::Declined
            );
            // DECLINED arrived; the connection failing afterwards does not undo it
            assert_eq!(
                driven(&[RTCStatus::Declined], Err("closed".into())),
                Stage::Declined
            );
            // Ended without a decision: a failure, not a decline
            for statuses in [&[][..], &[RTCStatus::Finished][..]] {
                let stage = driven(statuses, Ok(()));
                assert!(
                    matches!(&stage, Stage::Failed(Failure::Other(why)) if why.contains("before accepting")),
                    "{stage:?}"
                );
            }
        }
    }

    #[test]
    fn a_sent_file_is_saved_only_as_the_receiver_confirms() {
        let r = Rig::new();
        r.trust(1, "Phone", "Phone");
        let (id, _gate) = r.oeca(Direction::Send, "Phone", 1);
        let row = |name: &str| FileRow {
            name: name.into(),
            size: 1,
            done: 1,
            ok: None,
        };
        r.board(|b| {
            b.set_files(&id, vec![row("a"), row("b")]);
            b.sending(&id);
        });
        let queued = r.board(|b| outcome(b.live_transfer(&id).unwrap()));
        assert!(matches!(queued, Stage::Failed(_)), "queued is not saved");
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (progress, seen) = watch::channel(0u64);
        let report = |answers: &[(&str, bool)]| {
            rt.block_on(async {
                let (tx, rx) = mpsc::channel(8);
                for (file, success) in answers {
                    let response = RTCSendFileResponse {
                        id: file.to_string(),
                        success: *success,
                        error: None,
                    };
                    tx.send(response).await.ok().unwrap();
                }
                drop(tx);
                confirmations(rx, &r.nearby.shared, &id, progress.clone()).await;
            });
            r.board(|b| {
                let t = b.live_transfer(&id).unwrap();
                (outcome(t), t.files.iter().map(|f| f.ok).collect::<Vec<_>>())
            })
        };
        let (stage, ok) = report(&[("0", true)]);
        assert!(matches!(stage, Stage::Failed(_)), "one of two is not done");
        assert_eq!(ok, [Some(true), None]);
        assert_eq!(*seen.borrow(), FILE_PROGRESS, "a confirmation counts once");
        // Answers the core would never pass on change and count nothing
        let (_, ok) = report(&[("9", true), ("x", true), ("-1", true)]);
        assert_eq!(ok, [Some(true), None]);
        assert_eq!(*seen.borrow(), FILE_PROGRESS);
        let (stage, ok) = report(&[("1", true)]);
        assert_eq!((stage, ok), (Stage::Done, vec![Some(true), Some(true)]));
        assert_eq!(
            *seen.borrow(),
            2 * FILE_PROGRESS,
            "one per file, at its confirmation"
        );
    }

    #[test]
    fn a_failed_file_is_not_saved() {
        let r = Rig::new();
        r.trust(1, "Phone", "Phone");
        let (id, _gate) = r.oeca(Direction::Send, "Phone", 1);
        let row = |name: &str| FileRow {
            name: name.into(),
            size: 1,
            done: 1,
            ok: None,
        };
        r.board(|b| {
            b.set_files(&id, vec![row("a"), row("b")]);
            b.sending(&id);
        });
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (progress, _seen) = watch::channel(0u64);
        rt.block_on(async {
            let (tx, rx) = mpsc::channel(8);
            for (file, success) in [("0", true), ("1", false)] {
                let response = RTCSendFileResponse {
                    id: file.into(),
                    success,
                    error: Some("disk full".into()),
                };
                tx.send(response).await.ok().unwrap();
            }
            drop(tx);
            confirmations(rx, &r.nearby.shared, &id, progress).await;
        });
        let (stage, ok) = r.board(|b| {
            let t = b.live_transfer(&id).unwrap();
            (outcome(t), t.files.iter().map(|f| f.ok).collect::<Vec<_>>())
        });
        assert_eq!(ok, [Some(true), Some(false)]);
        assert!(
            matches!(&stage, Stage::Failed(Failure::Other(why)) if why.starts_with("1 of 2")),
            "{stage:?}"
        );
    }

    #[test]
    fn sending_counts_a_file_once_towards_the_idle_deadline() {
        // CI checks out with CRLF line endings
        let source = include_str!("nearby.rs").replace("\r\n", "\n");
        let send =
            &source[source.find("    fn send(").unwrap()..source.find("    fn receive(").unwrap()];
        assert_eq!(
            send.matches("send_modify(|b| *b += FILE_PROGRESS)").count(),
            0,
            "not when queued"
        );
        let confirming = &source[source.find("async fn confirmations(").unwrap()..];
        let confirming = &confirming[..confirming.find("\n}\n").unwrap()];
        assert_eq!(
            confirming
                .matches("send_modify(|b| *b += FILE_PROGRESS)")
                .count(),
            1
        );
    }

    #[test]
    fn a_decision_whose_task_is_gone_pins_nobody() {
        let r = Rig::new();
        let (id, gate) = r.oeca(Direction::Send, "Alice", 1);
        drop(gate);
        no_decision_works(&r, &id, Refused::NotCurrent);
        assert!(r.devices().list().unwrap().is_empty());
    }

    #[test]
    fn a_send_is_for_the_peer_as_displayed() {
        let mut r = Rig::new();
        let shown = peer("Alice");
        assert_eq!(r.nearby.send_files(&shown, vec![]), Err(Refused::Offline));
        r.board(|b| {
            b.view.link = Link::Online;
            b.view.me = Some("me".into());
            b.view.peers = vec![Peer {
                alias: "Mum".into(),
                ..shown.clone()
            }];
        });
        assert_eq!(
            r.nearby.send_files(&shown, vec![]),
            Err(Refused::PeerChanged)
        );
        r.board(|b| b.view.peers = vec![shown.clone()]);
        let id = r.nearby.send_files(&shown, vec![]).unwrap();
        assert_eq!(r.nearby.send_files(&shown, vec![]), Err(Refused::Busy));
        assert!(
            matches!(r.commands.try_recv(), Ok(Command::Send { id: queued, .. }) if queued == id)
        );
    }

    #[test]
    fn a_stranger_who_proved_nothing_does_not_block_sending() {
        let mut r = Rig::new();
        let mine = peer("My phone");
        r.board(|b| {
            b.view.link = Link::Online;
            b.view.me = Some("me".into());
            b.view.peers = vec![mine.clone()];
        });
        r.connecting(Direction::Receive, "Squatter", Capability::Stock, 9);
        let id = r.nearby.send_files(&mine, vec![]).unwrap();
        assert!(
            matches!(r.commands.try_recv(), Ok(Command::Send { id: queued, .. }) if queued == id)
        );
        r.board(|b| b.kill(&id, Stage::Cancelled));
        // One that is asking something keeps its place
        let (asking, _gate) = r.oeca(Direction::Receive, "Someone", 1);
        assert_eq!(r.nearby.send_files(&mine, vec![]), Err(Refused::Busy));
        assert!(r.board(|b| b.is_live(&asking)));
    }

    #[test]
    fn a_failure_does_not_carry_a_peers_text_without_bound() {
        let r = Rig::new();
        let id = r.connecting(Direction::Receive, "Phone", Capability::Stock, 1);
        let why = Failure::Other("x".repeat(100_000));
        r.board(|b| b.end(&id, Stage::Failed(why)));
        let Stage::Failed(Failure::Other(shown)) = r.transfer().stage else {
            panic!("not failed");
        };
        assert!(shown.len() <= 300);
    }

    #[test]
    fn a_trusted_key_keeps_its_confirmed_name_under_another_alias() {
        let r = Rig::new();
        r.trust(1, "Bob's laptop", "Bob");
        let (id, gate) = r.oeca(Direction::Send, "Mallory", 1);
        assert!(gate.is_some(), "held until the user confirms");
        let t = r.transfer();
        assert_eq!(t.stage, Stage::NameCheck);
        assert_eq!(
            t.identity,
            Some(Identity::Trusted {
                name: "Bob's laptop".into(),
                name_mismatch: true
            })
        );
        let (now, d) = (r.show(), r.devices());
        r.board(|b| {
            assert_eq!(b.allow_once(&id, d, now), Err(Refused::WrongStage));
            assert_eq!(b.verify(&id, "Mallory", d, now), Err(Refused::WrongStage));
            b.confirm_name(&id, d, now).unwrap();
        });
        assert_eq!(d.list().unwrap()[0].name, "Bob's laptop", "never renamed");
    }

    #[test]
    fn a_spoofed_alias_warns_and_replaces_nobody() {
        let r = Rig::new();
        r.trust(1, "Phone", "Phone");
        let (id, _gate) = r.oeca(Direction::Receive, "Phone", 2);
        assert_eq!(r.transfer().identity, Some(Identity::Changed));
        let (now, d) = (r.show(), r.devices());
        r.board(|b| {
            assert_eq!(
                b.verify(&id, "Phone", d, now),
                Err(Refused::Trust(TrustError::NameInUse))
            );
            b.verify(&id, "New phone", d, now).unwrap();
        });
        let kept = d.list().unwrap();
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().all(|k| k.state == DeviceState::Trusted));
    }

    #[test]
    fn a_claimed_fingerprint_is_no_identity() {
        let r = Rig::new();
        r.trust(1, "Phone", "Phone");
        for cap in [Capability::Oeca, Capability::Stock] {
            r.connecting(Direction::Send, "Phone", cap, 1);
            let t = r.transfer();
            assert_eq!(t.connection, Connection::Claimed);
            assert_eq!((t.identity.as_ref(), t.code()), (None, None));
        }
    }

    #[test]
    fn offering_oeca_without_the_proof_is_a_failure() {
        let r = Rig::new();
        r.trust(1, "Phone", "Phone");
        let id = r.connecting(Direction::Send, "Phone", Capability::Oeca, 1);
        let admitted = r.nearby.shared.admit(&id, Proof::Stock(connected(1)));
        assert!(matches!(admitted, Err(Failure::Authentication(_))));
        let t = r.transfer();
        assert_eq!((t.connection, t.identity), (Connection::Closed, None));
        assert_eq!(t.capability, Capability::Oeca);
    }

    #[test]
    fn failure_leaves_no_trust_behind() {
        for end in [
            Stage::Failed(Failure::Other("cut".into())),
            Stage::Cancelled,
            Stage::Declined,
        ] {
            let r = Rig::new();
            r.trust(1, "Phone", "Phone");
            let (id, _) = r.oeca(Direction::Send, "Phone", 1);
            assert!(r.transfer().code().is_some());
            r.board(|b| b.end(&id, end.clone()));
            let t = r.transfer();
            assert_eq!((t.code(), t.identity.as_ref()), (None, None), "{end:?}");
            assert_eq!(t.connection, Connection::Closed);
            r.board(|b| b.end(&id, Stage::Done));
            assert_eq!(r.transfer().stage, end, "final is final");
        }
    }

    #[test]
    fn revocation_ends_a_live_transfer_and_shuts_its_gate() {
        for at_gate in [true, false] {
            let r = Rig::new();
            r.trust(1, "Phone", "Phone");
            let alias = if at_gate { "Other name" } else { "Phone" };
            let (id, mut gate) = r.oeca(Direction::Send, alias, 1);
            if !at_gate {
                r.board(|b| b.sending(&id));
            }
            r.devices().forget(&fingerprint(1)).unwrap();
            // Every check sees it before the worker even wakes
            assert!(r.nearby.shared.still_allowed(&id).is_err());
            if at_gate {
                let now = r.show();
                let confirmed = r.board(|b| b.confirm_name(&id, r.devices(), now));
                assert_eq!(confirmed, Err(Refused::NotEligible));
            }
            r.board(|b| b.recheck(r.devices()));
            assert_eq!(r.transfer().stage, Stage::Failed(Failure::Revoked));
            assert_eq!(r.transfer().identity, None);
            if let Some(gate) = gate.as_mut() {
                assert!(gate.try_recv().is_err());
            }
            assert!(!r.board(|b| b.is_live(&id)));
        }
    }

    #[test]
    fn revocation_beats_a_pending_approval() {
        let r = Rig::new();
        r.trust(1, "Phone", "Phone");
        let (id, _) = r.oeca(Direction::Receive, "Phone", 1);
        let _gate = r.board(|b| b.offer(&id, rows()));
        let now = r.show();
        r.devices().forget(&fingerprint(1)).unwrap();
        let accepted = r.board(|b| b.respond(&id, true, r.devices(), now));
        assert_eq!(accepted, Err(Refused::NotEligible));
    }

    #[test]
    fn an_unsaved_revocation_is_still_effective() {
        let r = Rig::new();
        r.trust(1, "Phone", "Phone");
        let (id, _) = r.oeca(Direction::Send, "Phone", 1);
        r.devices().fail_writes(Some(devices::Fault::DiskFull));
        assert!(matches!(
            r.devices().forget(&fingerprint(1)),
            Err(TrustError::NotSaved { .. })
        ));
        assert_eq!(r.nearby.shared.still_allowed(&id), Err(Failure::Revoked));
    }

    #[test]
    fn a_pin_that_cannot_be_saved_opens_nothing() {
        let r = Rig::new();
        let (id, gate) = r.oeca(Direction::Send, "Alice", 1);
        let now = r.show();
        r.devices().fail_writes(Some(devices::Fault::AllWrites));
        let verified = r.board(|b| b.verify(&id, "Alice", r.devices(), now));
        assert!(matches!(
            verified,
            Err(Refused::Trust(TrustError::Write(_)))
        ));
        assert_eq!(r.transfer().stage, Stage::Verification);
        assert_eq!(r.transfer().identity, Some(Identity::New));
        assert!(gate.unwrap().try_recv().is_err());
    }

    #[test]
    fn a_revoked_device_returns_as_revoked() {
        let r = Rig::new();
        r.trust(1, "Phone", "Phone");
        r.devices().forget(&fingerprint(1)).unwrap();
        for alias in ["Phone", "Brand new name"] {
            let (id, _gate) = r.oeca(Direction::Receive, alias, 1);
            let t = r.transfer();
            assert_eq!(
                (t.stage, t.identity),
                (Stage::Verification, Some(Identity::Revoked))
            );
            let now = r.show();
            let once = r.board(|b| b.allow_once(&id, r.devices(), now));
            assert_eq!(once, Err(Refused::WrongStage));
        }
    }

    #[test]
    fn a_missing_extension_erases_no_warning_and_allows_nothing() {
        let r = Rig::new();
        r.trust(1, "Phone", "Phone");
        r.trust(2, "Tablet", "Tablet");
        r.devices().forget(&fingerprint(2)).unwrap();
        for (seed, alias, left) in [
            (1, "Phone", None),                     // trusted: never shown as such
            (2, "Tablet", Some(Identity::Revoked)), // the revoked key
            (3, "Phone", Some(Identity::Changed)),  // another key, a known name
            (4, "Tablet", Some(Identity::Changed)), // another key, a revoked name
        ] {
            for direction in [Direction::Send, Direction::Receive] {
                let (id, admitted) = r.stock(direction, alias, seed);
                assert_eq!(admitted.err(), Some(Failure::Downgraded), "{seed}");
                let t = r.transfer();
                assert_eq!(t.identity, left, "{seed}: the warning stays");
                assert_eq!(t.stage, Stage::Failed(Failure::Downgraded));
                no_decision_works(&r, &id, Refused::NotCurrent);
            }
        }
    }

    #[test]
    fn once_is_for_an_unknown_stock_peer_and_is_not_remembered() {
        for direction in [Direction::Send, Direction::Receive] {
            let r = Rig::new();
            let (id, admitted) = r.stock(direction, "Someone", 7);
            let mut gate = admitted.unwrap();
            if direction == Direction::Receive {
                assert!(gate.is_none());
                gate = r.board(|b| b.offer(&id, rows()));
            }
            let t = r.transfer();
            assert_eq!(
                (t.identity, t.capability),
                (Some(Identity::New), Capability::Stock)
            );
            let (now, d) = (r.show(), r.devices());
            r.board(|b| {
                assert!(
                    b.respond(&id, true, d, now).is_err(),
                    "never a plain accept"
                );
                assert!(
                    b.verify(&id, "Someone", d, now).is_err(),
                    "never verifiable"
                );
                b.allow_once(&id, d, now).unwrap();
            });
            assert_eq!(gate.unwrap().try_recv(), Ok(true));
            assert_eq!(r.transfer().code(), None);
            assert!(d.list().unwrap().is_empty());
        }
    }

    #[test]
    fn the_public_api_takes_the_id_it_showed() {
        let r = Rig::new();
        r.trust(1, "Phone", "Phone");
        let (id, _gate) = r.oeca(Direction::Send, "Other name", 1);
        assert_eq!(
            r.nearby.confirm_name(&id),
            Err(Refused::NotYet),
            "not shown yet"
        );
        assert_eq!(r.nearby.snapshot().transfer.unwrap().id, id);
        assert_eq!(
            r.nearby.confirm_name(&TransferId::new()),
            Err(Refused::NotCurrent)
        );
        std::thread::sleep(ARMING + Duration::from_millis(20));
        r.nearby.confirm_name(&id).unwrap();
        assert_eq!(r.transfer().stage, Stage::AwaitingPeer);
        r.nearby.cancel(&id).unwrap();
    }

    // ---- The real session(), with the socket scripted: no PeerConnection

    struct Hostile {
        server: mpsc::Sender<WsServerMessage>,
        sent: mpsc::Receiver<WsClientMessage>,
        commands: mpsc::Sender<Command>,
        shared: Arc<Shared>,
        peer: ClientInfo,
    }

    impl Hostile {
        async fn offer(&self, session: &str, fingerprint: &str) {
            let text =
                format!("v=0\r\na=fingerprint:{fingerprint}\r\na=x-oeca-nearby-commit:{R_O}\r\n");
            let offer = WsServerSdpMessage {
                peer: self.peer.clone(),
                session_id: session.into(),
                sdp: signaled_sdp(&text).unwrap(),
            };
            self.server
                .send(WsServerMessage::Offer(offer))
                .await
                .unwrap();
        }

        async fn refused_as_busy(&mut self, session: &str) {
            let sent = tokio::time::timeout(Duration::from_secs(5), self.sent.recv()).await;
            let Ok(Some(WsClientMessage::Answer(answer))) = sent else {
                panic!("no answer");
            };
            assert_eq!(answer.session_id, session);
            let said = Said::read(&answer.sdp, osv::NONCE);
            assert!(said.busy && said.capability == Capability::Oeca);
            assert_eq!(said.value, None, "no nonce: no guess");
            assert_eq!(said.fingerprint, None, "and no identity claimed");
        }

        async fn settle(&self) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn client(id: &str, alias: &str) -> ClientInfo {
        ClientInfo {
            id: id.parse().unwrap(),
            alias: alias.into(),
            version: PROTOCOL_VERSION.into(),
            device_model: None,
            device_type: None,
            token: String::new(),
        }
    }

    const ME: &str = "00000000-0000-4000-8000-000000000001";
    const PHONE: &str = "00000000-0000-4000-8000-000000000002";
    const OTHER: &str = "00000000-0000-4000-8000-000000000003";

    fn hostile<F, T>(r: &Rig, script: impl FnOnce(Hostile) -> F) -> T
    where
        F: Future<Output = T>,
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (out_tx, sent) = mpsc::channel(32);
            let (server, server_rx) = mpsc::channel(32);
            let (sig, rx) = SignalingConnection {
                client: client(ME, "Oeca"),
                tx: out_tx,
                rx: server_rx,
            }
            .start_listener();
            let (commands, mut cmds) = mpsc::channel(8);
            let cfg = Config {
                signaling_url: String::new(),
                stun: Vec::new(),
                alias: "Oeca".into(),
                data_dir: None,
                devices: r.devices().clone(),
                receive_dir: r.dir.clone(),
            };
            let shared = r.nearby.shared.clone();
            let cert = identity::dtls(None).map(|(c, _)| c);
            let me = Me {
                key: token::generate_key(),
                fingerprint: cert.as_ref().and_then(cert_fingerprint),
                cert,
            };
            let peer = client(PHONE, "Phone");
            let hello = WsServerMessage::Hello {
                client: client(ME, "Oeca"),
                peers: vec![peer.clone()],
            };
            server.send(hello).await.unwrap();
            let h = Hostile { server, sent, commands, shared: shared.clone(), peer };
            let follow = tokio::spawn(follow_devices(shared.clone(), shared.devices.changes()));
            let out = tokio::select! {
                _ = session(&cfg, &shared, &me, Arc::new(sig), rx, &mut cmds) => panic!("session ended"),
                out = script(h) => out,
            };
            follow.abort();
            out
        })
    }

    #[test]
    fn a_busy_oeca_says_so_and_stays_oeca() {
        let r = Rig::new();
        hostile(&r, |mut h| async move {
            let running = h.shared.with(|b| b.begin(Direction::Send, peer("Someone")));
            h.offer("while busy", FP_B).await;
            h.refused_as_busy("while busy").await;
            assert!(lock(&h.shared.board).is_live(&running), "undisturbed");
        });
    }

    #[test]
    fn oversized_offers_are_dropped_unanswered() {
        let r = Rig::new();
        hostile(&r, |mut h| async move {
            let _running = h.shared.with(|b| b.begin(Direction::Send, peer("Someone")));
            h.offer(&"s".repeat(MAX_SESSION_ID + 1), FP_B).await;
            let huge = WsServerSdpMessage {
                peer: h.peer.clone(),
                session_id: "big".into(),
                sdp: "A".repeat(MAX_SIGNALED_BYTES + 1),
            };
            h.server.send(WsServerMessage::Offer(huge)).await.unwrap();
            h.settle().await;
            assert!(h.sent.try_recv().is_err(), "no answer, no echo");
        });
    }

    #[test]
    fn refusing_while_busy_is_bounded() {
        let r = Rig::new();
        hostile(&r, |mut h| async move {
            let _running = h.shared.with(|b| b.begin(Direction::Send, peer("Someone")));
            for i in 0..MAX_REFUSALS_PER_PEER + 5 {
                h.offer(&i.to_string(), FP_B).await;
            }
            h.settle().await;
            let mut answered = 0;
            while h.sent.try_recv().is_ok() {
                answered += 1;
            }
            assert_eq!(answered, MAX_REFUSALS_PER_PEER);
        });
    }

    #[test]
    fn attempts_are_counted_before_a_claimed_trusted_key() {
        let r = Rig::new();
        r.trust(1, "Phone", "Phone");
        for i in 0..devices::MAX_ATTEMPTS_PER_PEER {
            assert!(r.devices().begin_attempt(PHONE, &i.to_string()));
        }
        let claimed = fingerprint(1);
        hostile(&r, |mut h| async move {
            h.offer("one more", claimed.as_str()).await;
            h.refused_as_busy("one more").await;
            h.settle().await;
            let t = lock(&h.shared.board).view.transfer.clone().unwrap();
            assert_eq!(t.stage, Stage::Failed(Failure::TooManyAttempts));
            assert_eq!(t.identity, None, "a claim");
        });
    }

    #[test]
    fn a_queued_command_never_reaches_a_replacement() {
        let r = Rig::new();
        hostile(&r, |mut h| async move {
            let old = h.shared.with(|b| b.begin(Direction::Send, peer("Old")));
            let new = h.shared.with(|b| b.begin(Direction::Send, peer("New")));
            let paths = vec![std::env::current_exe().unwrap()];
            let queued = Command::Send { id: old, paths };
            h.commands.send(queued).await.ok().unwrap();
            h.settle().await;
            assert!(h.sent.try_recv().is_err(), "no offer left");
            let t = lock(&h.shared.board).view.transfer.clone().unwrap();
            assert_eq!((t.id, t.stage, t.files.len()), (new, Stage::Signaling, 0));
        });
    }

    #[test]
    fn only_the_selected_peers_answer_reaches_the_core() {
        let r = Rig::new();
        hostile(&r, |h| async move {
            let id = h.shared.with(|b| b.begin(Direction::Send, peer("Phone")));
            let task = tokio::spawn(std::future::pending::<()>());
            let (to_core, mut answers) = mpsc::channel(8);
            h.shared
                .with(|b| b.attach(&id, task.abort_handle(), Some((PHONE.into(), to_core))));
            for (from, session) in [(OTHER, "s"), (PHONE, "not ours"), (PHONE, "s")] {
                let answer = WsServerSdpMessage {
                    peer: client(from, "Phone"),
                    session_id: session.into(),
                    sdp: signaled_sdp(&format!("a=fingerprint:{FP_A}\r\n")).unwrap(),
                };
                h.server
                    .send(WsServerMessage::Answer(answer))
                    .await
                    .unwrap();
            }
            h.settle().await;
            assert_eq!(answers.len(), 2, "the stranger's was dropped at the door");
            let got = selected_answer(&mut answers, PHONE.into(), "s".into())
                .await
                .unwrap();
            assert_eq!(
                (got.peer.id.to_string(), got.session_id),
                (PHONE.into(), "s".into())
            );
            assert_eq!(
                lock(&h.shared.board)
                    .view
                    .transfer
                    .as_ref()
                    .unwrap()
                    .identity,
                None
            );
        });
    }

    #[test]
    fn revoking_aborts_the_running_task() {
        let r = Rig::new();
        r.trust(1, "Phone", "Phone");
        let (id, _) = r.oeca(Direction::Send, "Phone", 1);
        let devices = r.devices().clone();
        hostile(&r, |h| async move {
            let task = tokio::spawn(std::future::pending::<()>());
            h.shared.with(|b| b.attach(&id, task.abort_handle(), None));
            h.settle().await;
            devices.forget(&fingerprint(1)).unwrap();
            let ended = tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap();
            assert!(ended.unwrap_err().is_cancelled());
        });
        assert_eq!(r.transfer().stage, Stage::Failed(Failure::Revoked));
    }

    // ---- The vendored core's bounds (OECA-VENDOR.txt, 12-15), no network but 127.0.0.1

    /// The inner task holds a `status` sender: that closing is that task ending.
    #[test]
    fn a_failed_session_takes_its_inner_task_with_it() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (out_tx, _out) = mpsc::channel(1);
            let (_server, server_rx) = mpsc::channel(1);
            let (sig, _events) = SignalingConnection {
                client: client(ME, "Oeca"),
                tx: out_tx,
                rx: server_rx,
            }
            .start_listener();
            let offer = WsServerSdpMessage {
                peer: client(PHONE, "Phone"),
                session_id: "s".into(),
                // Refused right after the task is spawned, before any socket
                sdp: "not an SDP".into(),
            };
            let (status_tx, mut status) = mpsc::channel(4);
            let (files_tx, _files) = oneshot::channel();
            let (_selected, selected) = oneshot::channel();
            let (errors_tx, _errors) = mpsc::channel(1);
            let (pin_tx, _pin) = mpsc::channel(1);
            let (file_tx, _file) = mpsc::channel(1);
            let (_result, result) = mpsc::channel(1);
            // Held, as a peer's open connection would hold the task's input
            let (pc_tx, mut pc_rx) = oneshot::channel();
            let extension = rtc::Extension {
                peer_connection: Some(pc_tx),
                ..Default::default()
            };
            let accepted = rtc::accept_offer(
                &sig,
                Vec::new(),
                identity::dtls(None).map(|(cert, _)| cert),
                &offer,
                token::generate_key(),
                None,
                None,
                status_tx,
                files_tx,
                selected,
                errors_tx,
                pin_tx,
                file_tx,
                result,
                Some(extension),
            )
            .await;
            assert!(accepted.is_err());
            let _connection = pc_rx.try_recv().unwrap();
            let ended = tokio::time::timeout(Duration::from_secs(2), status.recv()).await;
            assert!(
                matches!(ended, Ok(None)),
                "the inner task outlived its session"
            );
        });
    }

    /// A WebSocket server that says HELLO, then sends `messages`.
    fn websocket_server(messages: Vec<String>) -> String {
        use std::io::{BufRead, BufReader};
        use windows::Win32::Security::Cryptography::{BCRYPT_SHA1_ALG_HANDLE, BCryptHash};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}/v1/ws", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let limit = Some(Duration::from_secs(10));
            socket.set_read_timeout(limit).unwrap();
            socket.set_write_timeout(limit).unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut key = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line.trim().is_empty() {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("sec-websocket-key")
                {
                    key = value.trim().to_string();
                }
            }
            // RFC 6455's handshake: SHA-1 of the key and a fixed GUID
            let seed = format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
            let mut hash = [0u8; 20];
            unsafe { BCryptHash(BCRYPT_SHA1_ALG_HANDLE, None, seed.as_bytes(), &mut hash) }
                .ok()
                .unwrap();
            write!(
                socket,
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                standard_base64(&hash)
            )
            .unwrap();
            let hello = WsServerMessage::Hello {
                client: client(ME, "Oeca"),
                peers: Vec::new(),
            };
            let hello = localsend::serde_json::to_string(&hello).unwrap();
            for text in std::iter::once(hello).chain(messages) {
                let mut frame = vec![0x81, 127];
                frame.extend_from_slice(&(text.len() as u64).to_be_bytes());
                frame.extend_from_slice(text.as_bytes());
                if socket.write_all(&frame).is_err() {
                    return; // the client refused it
                }
            }
            let _ = reader.read(&mut [0u8; 64]); // until the client is done
        });
        url
    }

    /// With padding and + /, unlike LocalSend's.
    fn standard_base64(bytes: &[u8]) -> String {
        let mut out = base64::encode(bytes).replace('-', "+").replace('_', "/");
        while !out.len().is_multiple_of(4) {
            out.push('=');
        }
        out
    }

    #[test]
    fn a_signaling_message_over_1_mib_is_refused() {
        let offer = |len: usize| {
            let offer = WsServerSdpMessage {
                peer: client(PHONE, "Phone"),
                session_id: "s".into(),
                sdp: "A".repeat(len),
            };
            localsend::serde_json::to_string(&WsServerMessage::Offer(offer)).unwrap()
        };
        let url = websocket_server(vec![offer(900 << 10), offer(2 << 20)]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let info = ClientInfoWithoutId::from(client(ME, "Oeca"));
            let connected = SignalingConnection::connect(url, &info);
            let mut sig = tokio::time::timeout(Duration::from_secs(10), connected)
                .await
                .unwrap()
                .unwrap();
            let mut next = async || {
                tokio::time::timeout(Duration::from_secs(10), sig.rx.recv())
                    .await
                    .unwrap()
            };
            assert!(matches!(next().await, Some(WsServerMessage::Hello { .. })));
            let within = next().await;
            assert!(matches!(within, Some(WsServerMessage::Offer(o)) if o.sdp.len() == 900 << 10));
            assert!(next().await.is_none(), "an oversized message was delivered");
        });
    }

    /// Those that need a peer connection to exercise (a send's task, the
    /// message and channel limits) at least cannot be lost in a re-vendoring.
    #[test]
    fn the_vendored_core_keeps_its_bounds() {
        let core = include_str!("../vendor/localsend-core/src/webrtc/webrtc.rs");
        let count = |what: &str| core.matches(what).count();
        assert_eq!(
            count("AbortOnDrop(tokio::spawn("),
            2,
            "both sessions' tasks"
        );
        assert_eq!(count("= &mut send_task.0 =>"), 1, "awaited, not detached");
        assert_eq!(
            count("= &mut receive_task.0 =>"),
            1,
            "awaited, not detached"
        );
        assert_eq!(count("self.0.abort();"), 1);

        assert_eq!(count("const MAX_CONTROL_BYTES: usize = 1024 * 1024;"), 1);
        assert_eq!(
            count("buffer.len() + msg.data.len() > MAX_CONTROL_BYTES"),
            1
        );
        assert_eq!(
            count("receive_string_from_chunks(&mut receive_rx).await?;"),
            3
        );
        assert_eq!(count("receive_string_from_chunks(receive_rx).await?,"), 1);

        assert_eq!(count("const MAX_CHANNELS: usize = 4;"), 1);
        assert_eq!(count("opened > MAX_CHANNELS"), 1);

        assert_eq!(
            count("while !unconfirmed.is_empty()"),
            1,
            "every confirmation"
        );
        assert_eq!(count("take_remaining_results(&mut receive_rx"), 1);
        assert_eq!(count("while let Ok(answer) = receive_rx.try_recv()"), 1);
        assert_eq!(count("unconfirmed.remove(&response.id)"), 1);
        assert_eq!(count("timeout(LAST_ANSWERS_TIMEOUT, &mut send_task.0)"), 1);
        assert_eq!(count("= channel_limit(done_tx.clone());"), 2, "both ends");
    }

    // ---- Live tests against wss://public.localsend.org (network; run by hand):
    // cargo test nearby_live -- --ignored --nocapture --test-threads 1

    fn wait_for(n: &Nearby, what: &str, secs: u64, f: impl Fn(&View) -> bool) -> View {
        let end = std::time::Instant::now() + Duration::from_secs(secs);
        loop {
            let v = n.snapshot();
            if f(&v) {
                return v;
            }
            assert!(
                std::time::Instant::now() < end,
                "timed out waiting for {what}: {v:#?}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn nothing_received(dir: &Path) -> bool {
        std::fs::read_dir(dir).unwrap().all(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("nearby-")
        })
    }

    fn stage_is(n: &Nearby, what: &str, secs: u64, f: impl Fn(&Stage) -> bool) -> Transfer {
        wait_for(n, what, secs, |v| {
            v.transfer.as_ref().is_some_and(|t| f(&t.stage))
        })
        .transfer
        .unwrap()
    }

    /// A person at `n`'s screen: sees the question, waits, then answers that one.
    fn click<T>(n: &Nearby, answer: impl FnOnce(&Transfer) -> Result<T, Refused>) -> bool {
        let seen = n.snapshot().transfer.unwrap();
        std::thread::sleep(ARMING + Duration::from_millis(50));
        answer(&seen).is_ok()
    }

    fn send(n: &Nearby, peer: &str, paths: Vec<PathBuf>) -> bool {
        let shown = n.snapshot().peers.into_iter().find(|p| p.id == peer);
        shown.is_some_and(|p| n.send_files(&p, paths).is_ok())
    }

    fn codes_match(n: &Nearby) -> bool {
        click(n, |t| {
            let key = t.claimed.fingerprint.as_ref().map(|f| format!("{f:?}"));
            // Unique, as two devices may not share a confirmed name
            let name = hash::sha256_hex(key.unwrap_or_default().as_bytes());
            n.verify(&t.id, &format!("Test device {}", &name[..8]))
        })
    }

    fn answer(n: &Nearby, yes: bool) -> bool {
        click(n, |t| n.respond(&t.id, yes))
    }

    fn cancel(n: &Nearby) -> bool {
        let shown = n.snapshot().transfer.unwrap();
        n.cancel(&shown.id).is_ok()
    }

    /// The receiver's screen up to Approval: it verifies, then sees the list.
    fn asked(n: &Nearby, what: &str, secs: u64) -> Transfer {
        let t = stage_is(n, what, secs, |s| {
            matches!(s, Stage::Verification | Stage::Approval)
        });
        if t.stage == Stage::Approval {
            return t;
        }
        assert!(codes_match(n), "{what}: {t:?}");
        stage_is(n, what, secs, |s| *s == Stage::Approval)
    }

    fn trusted(t: &Transfer) -> bool {
        matches!(t.identity, Some(Identity::Trusted { .. }))
    }

    fn live(alias: &str, dir: &Path) -> Nearby {
        live_at(SIGNALING_URL, alias, dir)
    }

    fn live_at(url: &str, alias: &str, dir: &Path) -> Nearby {
        std::fs::create_dir_all(dir).unwrap();
        let cfg = Config {
            signaling_url: url.into(),
            stun: vec![STUN.into()],
            alias: alias.into(),
            data_dir: Some(dir.into()),
            devices: Devices::open(Some(dir.into())),
            receive_dir: dir.into(),
        };
        Nearby::start(cfg, || {}).unwrap()
    }

    #[test]
    #[ignore]
    fn nearby_live_native_to_native() {
        let base =
            crate::data_dir::temp_dir().join(format!("oeca-nearby-live-{}", std::process::id()));
        let (da, db) = (base.join("a"), base.join("b"));
        let tag = format!("{}", std::process::id());
        let a = live(&format!("Oeca test A {tag}"), &da);
        let b = live(&format!("Oeca test B {tag}"), &db);
        let va = wait_for(&a, "A online", 20, |v| v.link == Link::Online);
        assert!(va.persistent_identity);
        let vb = wait_for(&b, "B sees A", 20, |v| {
            v.peers.iter().any(|p| Some(&p.id) == va.me.as_ref())
        });
        let a_id = va.me.clone().unwrap();
        let b_id = vb.me.clone().unwrap();
        wait_for(&a, "A sees B", 20, |v| v.peers.iter().any(|p| p.id == b_id));

        // 1. A offers, B declines: nothing written
        let src = base.join("payload.bin");
        let payload: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7 % 251) as u8).collect();
        std::fs::write(&src, &payload).unwrap();
        assert!(send(&a, &b_id, vec![src.clone()]));
        // Unverified receiver: A holds the file list until it verifies
        let ta = stage_is(&a, "A asks to verify", 40, |s| *s == Stage::Verification);
        assert_eq!(ta.identity, Some(Identity::New));
        assert!(b.snapshot().transfer.is_none_or(|t| t.files.is_empty()));
        assert!(codes_match(&a));
        let tb = stage_is(&b, "B compares", 40, |s| *s == Stage::Verification);
        assert!(tb.files.is_empty() && tb.code().is_some());
        assert_eq!(tb.identity, Some(Identity::New));
        // Accepting is refused until B has verified
        assert!(!answer(&b, true));
        assert!(answer(&b, false));
        stage_is(&a, "A ends", 40, |s| matches!(s, Stage::Failed(_)));
        assert!(nothing_received(&db));

        // 1b. A offers and cancels before B decides: B's side ends, nothing written
        assert!(send(&a, &b_id, vec![src.clone()]));
        asked(&b, "B asked (cancel)", 40);
        assert!(cancel(&a));
        stage_is(&a, "A cancelled", 5, |s| *s == Stage::Cancelled);
        let t0 = std::time::Instant::now();
        // The cancel closes A's peer connection: B ends without waiting for a deadline
        stage_is(&b, "B ends after cancel", 15, |s| {
            matches!(s, Stage::Failed(_))
        });
        println!("B noticed the cancel in {:?}", t0.elapsed());
        assert!(nothing_received(&db));

        // 2. A offers again, B accepts: same bytes arrive
        assert!(send(&a, &b_id, vec![src.clone()]));
        asked(&b, "B asked again", 40);
        assert!(answer(&b, true));
        let vb = wait_for(&b, "B done", 60, |v| {
            v.transfer
                .as_ref()
                .is_some_and(|t| matches!(t.stage, Stage::Done | Stage::Failed(_)))
        });
        let tb = vb.transfer.unwrap();
        assert_eq!(tb.stage, Stage::Done, "{tb:?}");
        assert_eq!(std::fs::read(&tb.saved[0]).unwrap(), payload);
        let ta = wait_for(&a, "A done", 30, |v| {
            v.transfer.as_ref().is_some_and(|t| t.stage == Stage::Done)
        })
        .transfer
        .unwrap();
        // Both ends compute the same code from the same two certificates
        assert!(
            ta.code().is_some() && ta.code() == tb.code(),
            "{:?} {:?}",
            ta.code(),
            tb.code()
        );
        assert!(trusted(&ta) && trusted(&tb));

        // 2b. Verified now: no verification step, accept without re-verifying
        let small = base.join("small.txt");
        std::fs::write(&small, b"again").unwrap();
        assert!(send(&a, &b_id, vec![small.clone()]));
        let tb = asked(&b, "B asked (verified)", 40);
        assert!(trusted(&tb));
        assert!(answer(&b, true));
        let ta = stage_is(&a, "A done (verified)", 40, |s| {
            matches!(s, Stage::Done | Stage::Failed(_))
        });
        assert_eq!(ta.stage, Stage::Done, "{ta:?}");
        assert!(std::fs::read_dir(&db).unwrap().all(|e| {
            !e.unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".oeca-part")
        }));

        // 3. Restart B: same identity, discovered again, A's stale entry for the old B is gone
        let b_identity = b.snapshot().identity;
        drop(b);
        wait_for(&a, "A forgets old B", 20, |v| {
            !v.peers.iter().any(|p| p.id == b_id)
        });
        let b = live(&format!("Oeca test B {tag}"), &db);
        let vb = wait_for(&b, "B back", 20, |v| v.link == Link::Online);
        assert_eq!(vb.identity, b_identity);
        assert!(vb.persistent_identity);
        wait_for(&a, "A sees new B", 20, |v| {
            v.peers.iter().any(|p| Some(&p.id) == vb.me.as_ref())
        });
        let b_id = vb.me.unwrap();
        assert!(send(&a, &b_id, vec![small.clone()]));
        let tb = asked(&b, "B asked after restart", 40);
        assert!(trusted(&tb), "B remembers A");
        assert!(answer(&b, false));
        let ta = stage_is(&a, "A declined after restart", 40, |s| {
            *s == Stage::Declined
        });
        assert_eq!(
            (ta.identity, a.devices().list().unwrap().len()),
            (None, 1),
            "A remembers B"
        );

        // 4. B loses its DTLS identity (reset / new browser): same alias, new
        // certificate: A says Changed and sends nothing unless re-verified
        drop(b);
        std::fs::remove_file(db.join("nearby-dtls.bin")).unwrap();
        let b = live(&format!("Oeca test B {tag}"), &db);
        let vb = wait_for(&b, "B back again", 20, |v| v.link == Link::Online);
        let b_id = vb.me.unwrap();
        wait_for(&a, "A sees B again", 20, |v| {
            v.peers.iter().any(|p| p.id == b_id)
        });
        assert!(send(&a, &b_id, vec![small.clone()]));
        let ta = stage_is(&a, "A asks to verify the changed B", 40, |s| {
            *s == Stage::Verification
        });
        assert_eq!(ta.identity, Some(Identity::Changed));
        let tb = b.snapshot().transfer.unwrap();
        assert!(trusted(&tb), "A did not change");
        assert!(tb.files.is_empty(), "the changed B got no file list");
        assert!(answer(&a, false));
        let ta = stage_is(&a, "A refuses", 20, |s| *s == Stage::Declined);
        println!("A after refusing: {:?}", ta.stage);
        stage_is(&b, "B ends", 20, |s| {
            matches!(s, Stage::Failed(_) | Stage::Declined)
        });
        // Only what was accepted earlier (2, 2b): no new copy, no partial file
        let names: Vec<String> = std::fs::read_dir(&db)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with("nearby-"))
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");

        // 5. Reconnect: the old socket closes, so A drops the old B at once (no ghost)
        let t0 = std::time::Instant::now();
        assert!(b.reconnect());
        wait_for(&a, "A drops the old B", 10, |v| {
            !v.peers.iter().any(|p| p.id == b_id)
        });
        println!("old B left A's list in {:?}", t0.elapsed());
        let _ = a_id;
        drop((a, b));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// With Oeca Nearby's web peer (nearby-web) open in a browser on this
    /// network (`OECA_NEARBY_WEB` = its alias), driven by hand in steps the
    /// test prints ("STEP ..."):
    /// 1. Oeca sends a file: Oeca shows the code and waits for verification
    ///    (the test verifies after printing it; compare with the browser's);
    ///    the browser shows the same code and asks to accept.
    /// 2. The browser sends a file: Oeca declines; the browser must say so.
    /// 3. The browser sends a file: the browser is verified now; Oeca accepts.
    /// 4. Oeca sends `OECA_NEARBY_WEB_MB` (default 256) MB: accept in the browser.
    /// 5. The browser sends a large file: Oeca accepts; cancel it in the browser.
    #[test]
    #[ignore]
    fn nearby_live_browser() {
        let alias = std::env::var("OECA_NEARBY_WEB").expect("OECA_NEARBY_WEB=<web alias>");
        let mb: usize = std::env::var("OECA_NEARBY_WEB_MB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(256);
        let dir =
            crate::data_dir::temp_dir().join(format!("oeca-nearby-web-{}", std::process::id()));
        let n = live("Oeca live", &dir);
        let v = wait_for(&n, "web peer", 30, |v| {
            v.peers.iter().any(|p| p.alias == alias)
        });
        let web = v.peers.iter().find(|p| p.alias == alias).unwrap().clone();
        assert_eq!(web.kind, "web");
        let is_receive = |t: &Transfer| t.direction == Direction::Receive;

        println!("STEP 1: accept in the browser after comparing codes");
        let src = dir.join("oeca-stage2.txt");
        let text = b"Hello from Oeca Stage 2 (verified)";
        std::fs::write(&src, text).unwrap();
        assert!(send(&n, &web.id, vec![src]));
        let t = stage_is(&n, "verification", 180, |s| {
            matches!(s, Stage::Verification | Stage::Failed(_))
        });
        assert_eq!(t.stage, Stage::Verification, "{t:?}");
        println!("  native code {:?} trust {:?}", t.code(), t.identity);
        assert!(codes_match(&n));
        let t = stage_is(&n, "sent", 120, |s| {
            matches!(s, Stage::Done | Stage::Failed(_) | Stage::Declined)
        });
        println!(
            "  result {:?} trust {:?} sha256={}",
            t.stage,
            t.identity,
            hash::sha256_hex(text)
        );

        println!("STEP 2: send any file from the browser (Oeca declines)");
        let t = wait_for(&n, "incoming 2", 180, |v| {
            v.transfer
                .as_ref()
                .is_some_and(|t| is_receive(t) && t.stage == Stage::Approval)
        })
        .transfer
        .unwrap();
        println!(
            "  offered {:?} code {:?} trust {:?}",
            t.files,
            t.code(),
            t.identity
        );
        assert!(answer(&n, false));
        stage_is(&n, "declined", 30, |s| *s == Stage::Declined);

        println!("STEP 3: send a file from the browser again (Oeca accepts)");
        std::thread::sleep(Duration::from_secs(2));
        let t = wait_for(&n, "incoming 3", 180, |v| {
            v.transfer.as_ref().is_some_and(|t| {
                is_receive(t)
                    && t.stage == Stage::Approval
                    && !t.files.is_empty()
                    && t.saved.is_empty()
                    && t.files[0].ok.is_none()
            })
        })
        .transfer
        .unwrap();
        println!(
            "  trust {:?} (Verified expected: step 1 pinned it)",
            t.identity
        );
        assert!(answer(&n, true));
        let t = stage_is(&n, "received 3", 120, |s| {
            matches!(s, Stage::Done | Stage::Failed(_))
        });
        assert_eq!(t.stage, Stage::Done, "{t:?}");
        let data = std::fs::read(&t.saved[0]).unwrap();
        println!(
            "  received {} bytes sha256={}",
            data.len(),
            hash::sha256_hex(&data)
        );

        println!("STEP 4: accept the {mb} MB file in the browser");
        let big = dir.join("oeca-big.bin");
        {
            let mut f = std::io::BufWriter::new(std::fs::File::create(&big).unwrap());
            let block: Vec<u8> = (0..1u32 << 20)
                .map(|i| (i.wrapping_mul(2654435761) >> 11) as u8)
                .collect();
            for _ in 0..mb {
                f.write_all(&block).unwrap();
            }
        }
        assert!(send(&n, &web.id, vec![big]));
        wait_for(&n, "big offered", 20, |v| {
            v.transfer
                .as_ref()
                .is_some_and(|t| t.files.first().is_some_and(|f| f.name == "oeca-big.bin"))
        });
        let t0 = std::time::Instant::now();
        let t = stage_is(&n, "big sent", 900, |s| {
            matches!(
                s,
                Stage::Done | Stage::Failed(_) | Stage::Declined | Stage::Cancelled
            )
        });
        println!(
            "  result {:?} in {:.1} s",
            t.stage,
            t0.elapsed().as_secs_f64()
        );

        println!("STEP 5: send a large file from the browser, then cancel it there");
        std::thread::sleep(Duration::from_secs(2));
        wait_for(&n, "incoming 5", 180, |v| {
            v.transfer
                .as_ref()
                .is_some_and(|t| is_receive(t) && t.stage == Stage::Approval)
        })
        .transfer
        .unwrap();
        assert!(answer(&n, true));
        let t0 = std::time::Instant::now();
        let t = stage_is(&n, "cancelled by the browser", 300, |s| {
            matches!(s, Stage::Done | Stage::Failed(_))
        });
        println!(
            "  native saw {:?} {:.1} s after accepting",
            t.stage,
            t0.elapsed().as_secs_f64()
        );
        let parts = std::fs::read_dir(&dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".oeca-part")
            })
            .count();
        println!("  partial files left: {parts}");
        drop(n);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The official LocalSend signaling server, built from the pinned
    /// repository's `server` crate (`OECA_NEARBY_SERVER` = its exe), on
    /// localhost with tiny limits: refusal over the connection limit, the
    /// request limit (429), loss and reconnect. The public server's limits
    /// are shared by everyone behind this IP, so they are not provoked there.
    #[test]
    #[ignore]
    fn nearby_live_server_limits() {
        let exe = std::env::var("OECA_NEARBY_SERVER").expect("OECA_NEARBY_SERVER=<server.exe>");
        let port = "39317";
        // Killed on drop, so a failing assertion leaves no server behind
        struct Server(std::process::Child);
        impl Drop for Server {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let start = || {
            let child = std::process::Command::new(&exe)
                .env("SERVER_IP", "127.0.0.1")
                .env("SERVER_PORT", port)
                .env("MAX_CONNECTIONS_PER_IP", "2")
                .env("MAX_REQUESTS_PER_IP_PER_HOUR", "2")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            std::thread::sleep(Duration::from_millis(800));
            Server(child)
        };
        let url = format!("ws://127.0.0.1:{port}/v1/ws");
        let base =
            crate::data_dir::temp_dir().join(format!("oeca-nearby-limits-{}", std::process::id()));
        let server = start();
        let a = live_at(&url, "A", &base.join("a"));
        let va = wait_for(&a, "A online", 10, |v| v.link == Link::Online);
        let b = live_at(&url, "B", &base.join("b"));
        let vb = wait_for(&b, "B online", 10, |v| v.link == Link::Online);
        wait_for(&a, "A sees B", 10, |v| {
            v.peers.iter().any(|p| Some(&p.id) == vb.me.as_ref())
        });

        // Over the connection limit: refused before HELLO, long wait, no panic escapes
        let c = live_at(&url, "C", &base.join("c"));
        let vc = wait_for(&c, "C refused", 10, |v| {
            matches!(
                v.link,
                Link::Retrying {
                    why: Retry::Rejected,
                    ..
                }
            )
        });
        println!("C: {:?}", vc.link);

        // Over the request limit: A's offer is answered 429 and A is dropped
        let src = base.join("x.txt");
        std::fs::write(&src, b"x").unwrap();
        assert!(send(&a, vb.me.as_ref().unwrap(), vec![src]));
        let va2 = wait_for(&a, "A limited", 10, |v| {
            matches!(v.link, Link::Retrying { .. })
        });
        println!("A after its offer: {:?}", va2.link);
        assert!(
            va2.peers.is_empty() && va2.me.is_none(),
            "no stale room state"
        );
        let va3 = wait_for(&a, "A's transfer ended", 40, |v| {
            v.transfer
                .as_ref()
                .is_some_and(|t| matches!(t.stage, Stage::Failed(_)))
        });
        println!("A transfer: {:?}", va3.transfer.map(|t| t.stage));
        wait_for(&b, "B sees A leave", 10, |v| {
            !v.peers.iter().any(|p| Some(&p.id) == va.me.as_ref())
        });

        // Server gone: B loses the room, then reconnects by itself once it is back
        // (A stops first: it would take one of the two connections)
        drop(a);
        drop(server);
        let vb2 = wait_for(&b, "B lost", 10, |v| {
            matches!(
                v.link,
                Link::Retrying {
                    why: Retry::Lost,
                    ..
                }
            )
        });
        assert!(vb2.peers.is_empty() && vb2.me.is_none());
        let _server = start();
        wait_for(&b, "B back", 30, |v| v.link == Link::Online);
        // C was told to wait a minute; an explicit reconnect goes now
        assert!(c.reconnect());
        wait_for(&c, "C online", 10, |v| v.link == Link::Online);
        let t0 = std::time::Instant::now();
        drop((b, c));
        println!("2 workers stopped in {:?}", t0.elapsed());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A large native-to-native transfer over the public server, for memory
    /// and throughput (`OECA_NEARBY_BIG_MB`, default 256). Both peers live in
    /// this process; sample its working set from outside while it runs.
    #[test]
    #[ignore]
    fn nearby_live_large() {
        let mb: u64 = std::env::var("OECA_NEARBY_BIG_MB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(256);
        let base =
            crate::data_dir::temp_dir().join(format!("oeca-nearby-big-{}", std::process::id()));
        let (da, db) = (base.join("a"), base.join("b"));
        let tag = std::process::id();
        let a = live(&format!("Oeca big A {tag}"), &da);
        let b = live(&format!("Oeca big B {tag}"), &db);
        let vb = wait_for(&b, "B online", 20, |v| v.link == Link::Online);
        let b_id = vb.me.unwrap();
        wait_for(&a, "A sees B", 20, |v| v.peers.iter().any(|p| p.id == b_id));
        let src = base.join("big.bin");
        {
            let mut f = std::io::BufWriter::new(std::fs::File::create(&src).unwrap());
            let block: Vec<u8> = (0..1 << 20)
                .map(|i: u32| (i.wrapping_mul(2654435761) >> 13) as u8)
                .collect();
            for i in 0..mb {
                let mut blk = block.clone();
                blk[0] = i as u8;
                f.write_all(&blk).unwrap();
            }
        }
        println!("MEASURE pid={} size={} MB", std::process::id(), mb);
        let t0 = std::time::Instant::now();
        assert!(send(&a, &b_id, vec![src.clone()]));
        stage_is(&a, "A verifies", 40, |s| *s == Stage::Verification);
        assert!(codes_match(&a));
        asked(&b, "B asked", 40);
        assert!(answer(&b, true));
        let t = wait_for(&b, "B done", 1800, |v| {
            v.transfer
                .as_ref()
                .is_some_and(|t| matches!(t.stage, Stage::Done | Stage::Failed(_)))
        })
        .transfer
        .unwrap();
        let secs = t0.elapsed().as_secs_f64();
        assert_eq!(t.stage, Stage::Done, "{t:?}");
        println!("DONE {mb} MB in {secs:.1} s = {:.1} MB/s", mb as f64 / secs);
        // Byte-for-byte, streamed (no whole-file buffer here either)
        let (mut x, mut y) = (
            std::fs::File::open(&src).unwrap(),
            std::fs::File::open(&t.saved[0]).unwrap(),
        );
        let (mut bx, mut by) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
        loop {
            let n = x.read(&mut bx).unwrap();
            y.read_exact(&mut by[..n]).unwrap();
            assert_eq!(bx[..n], by[..n]);
            if n == 0 {
                break;
            }
        }
        drop((a, b));
        let _ = std::fs::remove_dir_all(&base);
    }

    fn udp_sockets() -> usize {
        let script = format!(
            "@(Get-NetUDPEndpoint -OwningProcess {} -ErrorAction SilentlyContinue).Count",
            std::process::id()
        );
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    }

    /// Cancelled and declined sessions release their sockets (vendor patch
    /// CloseOnDrop), instead of keeping them until Oeca exits.
    #[test]
    #[ignore]
    fn nearby_live_cleanup() {
        let base =
            crate::data_dir::temp_dir().join(format!("oeca-nearby-clean-{}", std::process::id()));
        let tag = std::process::id();
        let a = live(&format!("Oeca clean A {tag}"), &base.join("a"));
        let b = live(&format!("Oeca clean B {tag}"), &base.join("b"));
        let b_id = wait_for(&b, "B online", 20, |v| v.link == Link::Online)
            .me
            .unwrap();
        wait_for(&a, "A sees B", 20, |v| v.peers.iter().any(|p| p.id == b_id));
        let src = base.join("x.bin");
        std::fs::write(&src, vec![7u8; 100_000]).unwrap();
        let before = udp_sockets();
        for i in 0..4 {
            assert!(send(&a, &b_id, vec![src.clone()]));
            if i == 0 {
                stage_is(&a, "A verifies", 40, |s| *s == Stage::Verification);
                assert!(codes_match(&a));
            }
            asked(&b, "B asked", 40);
            let during = udp_sockets();
            assert!(during > before, "the count must see the session's sockets");
            print!("cycle {i}: during {during}, ");
            if i % 2 == 0 {
                assert!(cancel(&a));
                stage_is(&b, "B ends", 20, |s| matches!(s, Stage::Failed(_)));
            } else {
                assert!(answer(&b, false));
                stage_is(&a, "A declined", 20, |s| *s == Stage::Declined);
            }
            std::thread::sleep(Duration::from_millis(500));
            println!("after {}", udp_sockets());
        }
        // close() runs in the background (bounded by 5 s)
        std::thread::sleep(Duration::from_secs(7));
        let after = udp_sockets();
        println!("udp sockets before {before}, after 4 cancel/decline cycles {after}");
        assert!(after <= before, "leaked {} UDP sockets", after - before);
        drop((a, b));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// An unverified receiver gets nothing about the files: no name, size or
    /// type reaches it until the sender verifies (core gate before the file
    /// list). Then: verified sends go through; `forget` revokes.
    #[test]
    #[ignore]
    fn nearby_live_no_metadata_before_verification() {
        let base =
            crate::data_dir::temp_dir().join(format!("oeca-nearby-meta-{}", std::process::id()));
        let tag = std::process::id();
        let a = live(&format!("Oeca meta A {tag}"), &base.join("a"));
        let b = live(&format!("Oeca meta B {tag}"), &base.join("b"));
        let b_id = wait_for(&b, "B online", 20, |v| v.link == Link::Online)
            .me
            .unwrap();
        wait_for(&a, "A sees B", 20, |v| v.peers.iter().any(|p| p.id == b_id));
        let src = base.join("secret-name-q3-report.pdf");
        std::fs::write(&src, vec![1u8; 4096]).unwrap();

        // 1. Unverified: A holds everything; B, connected, sees no file at all
        assert!(send(&a, &b_id, vec![src.clone()]));
        let ta = stage_is(&a, "A waits for verification", 40, |s| {
            *s == Stage::Verification
        });
        assert_eq!(ta.identity, Some(Identity::New));
        let until = std::time::Instant::now() + Duration::from_secs(4);
        while std::time::Instant::now() < until {
            let tb = b.snapshot().transfer;
            assert!(
                tb.as_ref()
                    .is_none_or(|t| t.files.is_empty() && t.stage != Stage::Approval),
                "B saw metadata: {tb:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        let tb = b.snapshot().transfer.unwrap();
        println!(
            "B while A is unverified: stage {:?}, files {:?}, code {:?}",
            tb.stage,
            tb.files,
            tb.code()
        );
        assert_eq!(tb.code(), ta.code(), "B can already compare the code");
        assert!(answer(&a, false));
        stage_is(&a, "A refused", 20, |s| *s == Stage::Declined);
        let tb = stage_is(&b, "B ends", 20, |s| {
            matches!(s, Stage::Failed(_) | Stage::Declined)
        });
        assert!(tb.files.is_empty(), "B never got the list: {tb:?}");

        // 2. Verified: the list arrives only now
        assert!(send(&a, &b_id, vec![src.clone()]));
        stage_is(&a, "A waits again", 40, |s| *s == Stage::Verification);
        assert!(b.snapshot().transfer.unwrap().files.is_empty());
        assert!(codes_match(&a));
        let tb = asked(&b, "B asked", 20);
        assert_eq!(tb.files[0].name, "secret-name-q3-report.pdf");
        assert!(answer(&b, false));
        let ta = stage_is(&a, "A declined", 20, |s| *s == Stage::Declined);
        assert_eq!(ta.identity, None, "declined");

        // 3. Revoked: back to verification, never trusted by alias
        a.devices()
            .forget(&a.devices().list().unwrap()[0].fingerprint)
            .unwrap();
        assert!(send(&a, &b_id, vec![src.clone()]));
        let ta = stage_is(&a, "A asks again after forget", 40, |s| {
            *s == Stage::Verification
        });
        assert_eq!(ta.identity, Some(Identity::Revoked));
        assert!(b.snapshot().transfer.unwrap().files.is_empty());
        assert!(cancel(&a));
        drop((a, b));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Cancel mid-transfer, from either side: the other side ends at once
    /// (data channel close), nothing partial stays, nothing reports Done.
    #[test]
    #[ignore]
    fn nearby_live_cancel_midway() {
        let base =
            crate::data_dir::temp_dir().join(format!("oeca-nearby-cancel-{}", std::process::id()));
        let tag = std::process::id();
        let (da, db) = (base.join("a"), base.join("b"));
        let a = live(&format!("Oeca cancel A {tag}"), &da);
        let b = live(&format!("Oeca cancel B {tag}"), &db);
        let b_id = wait_for(&b, "B online", 20, |v| v.link == Link::Online)
            .me
            .unwrap();
        wait_for(&a, "A sees B", 20, |v| v.peers.iter().any(|p| p.id == b_id));
        let src = base.join("big.bin");
        std::fs::write(&src, vec![5u8; 200 << 20]).unwrap();
        for (round, receiver_cancels) in [(0, false), (1, true)] {
            assert!(send(&a, &b_id, vec![src.clone()]));
            if round == 0 {
                stage_is(&a, "verify", 40, |s| *s == Stage::Verification);
                assert!(codes_match(&a));
                asked(&b, "asked", 40);
                assert!(answer(&b, true));
            } else {
                asked(&b, "asked", 40);
                assert!(answer(&b, true));
            }
            wait_for(&b, "bytes flowing", 40, |v| {
                v.transfer
                    .as_ref()
                    .is_some_and(|t| t.files[0].done > (8 << 20))
            });
            let (canceller, other) = if receiver_cancels { (&b, &a) } else { (&a, &b) };
            let t0 = std::time::Instant::now();
            assert!(cancel(canceller));
            let t = stage_is(other, "other side ends", 30, |s| {
                !matches!(
                    s,
                    Stage::Transferring
                        | Stage::Approval
                        | Stage::Connecting
                        | Stage::Signaling
                        | Stage::Verification
                        | Stage::AwaitingOffer
                        | Stage::AwaitingPeer
                )
            });
            let took = t0.elapsed();
            println!(
                "{} cancelled: the other side saw {:?} after {took:?}",
                if receiver_cancels {
                    "receiver"
                } else {
                    "sender"
                },
                t.stage
            );
            assert_ne!(t.stage, Stage::Done);
            assert!(took < Duration::from_secs(3), "slow: {took:?}");
            std::thread::sleep(Duration::from_millis(300));
            let left: Vec<_> = std::fs::read_dir(&db)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .filter(|n| !n.to_string_lossy().starts_with("nearby-"))
                .collect();
            assert!(left.is_empty(), "partial files: {left:?}");
        }
        drop((a, b));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Stage 2.1 hardening against nearby-web (`OECA_NEARBY_WEB` = its alias),
    /// driven by hand in the printed steps.
    #[test]
    #[ignore]
    fn nearby_live_browser_hardening() {
        let alias = std::env::var("OECA_NEARBY_WEB").expect("OECA_NEARBY_WEB=<web alias>");
        let dir =
            crate::data_dir::temp_dir().join(format!("oeca-nearby-hard-{}", std::process::id()));
        let n = live("Oeca live", &dir);
        let v = wait_for(&n, "web peer", 30, |v| {
            v.peers.iter().any(|p| p.alias == alias)
        });
        let web = v.peers.iter().find(|p| p.alias == alias).unwrap().clone();
        let receive = |t: &Transfer| t.direction == Direction::Receive;
        let ended = |s: &Stage| {
            matches!(
                s,
                Stage::Done | Stage::Failed(_) | Stage::Declined | Stage::Cancelled
            )
        };

        println!(
            "STEP 1: send a file from the browser; do NOT confirm the code for 8 s, then tap Codes differ"
        );
        wait_for(&n, "incoming 1", 180, |v| {
            v.transfer.as_ref().is_some_and(receive)
        });
        let t0 = std::time::Instant::now();
        let mut seen = Vec::new();
        loop {
            let t = n.snapshot().transfer.unwrap();
            assert!(
                t.files.is_empty(),
                "metadata before verification: {:?}",
                t.files
            );
            if !seen.contains(&t.stage) {
                println!(
                    "  native: {:?} code {:?} files {:?} at {:?}",
                    t.stage,
                    t.code(),
                    t.files,
                    t0.elapsed()
                );
                seen.push(t.stage.clone());
            }
            if ended(&t.stage) {
                break;
            }
            assert!(t0.elapsed() < Duration::from_secs(180));
            std::thread::sleep(Duration::from_millis(50));
        }

        println!(
            "STEP 2: Oeca sends; it stays unverified for 8 s (the browser must show no file name), then refuses"
        );
        std::thread::sleep(Duration::from_secs(2));
        let src = dir.join("private-name.txt");
        std::fs::write(&src, b"x").unwrap();
        assert!(send(&n, &web.id, vec![src]));
        let t = stage_is(&n, "verification", 60, |s| *s == Stage::Verification);
        println!("  native code {:?}; holding the list", t.code());
        std::thread::sleep(Duration::from_secs(8));
        assert!(answer(&n, false));
        let t = stage_is(&n, "refused", 20, |s| ended(s));
        println!("  native: {:?}", t.stage);

        println!(
            "STEP 3: Oeca sends 200 MB, verified: accept it in the browser, then cancel it there mid-way"
        );
        std::thread::sleep(Duration::from_secs(2));
        let big = dir.join("big.bin");
        std::fs::write(&big, vec![9u8; 200 << 20]).unwrap();
        assert!(send(&n, &web.id, vec![big]));
        stage_is(&n, "verification 3", 60, |s| *s == Stage::Verification);
        assert!(codes_match(&n));
        wait_for(&n, "bytes flowing", 180, |v| {
            v.transfer
                .as_ref()
                .is_some_and(|t| t.files.first().is_some_and(|f| f.done > (4 << 20)))
        });
        let t0 = std::time::Instant::now();
        let t = stage_is(&n, "browser cancelled", 300, |s| ended(s));
        println!(
            "  native saw {:?} {:?} after bytes started flowing (includes your reaction time)",
            t.stage,
            t0.elapsed()
        );

        println!(
            "STEP 4: send a large file (e.g. 300 MB) from the browser; Oeca accepts and cancels it after 8 MB"
        );
        std::thread::sleep(Duration::from_secs(2));
        let t = wait_for(&n, "incoming 4", 180, |v| {
            v.transfer
                .as_ref()
                .is_some_and(|t| receive(t) && t.stage == Stage::Approval)
        })
        .transfer
        .unwrap();
        println!("  trust {:?}", t.identity);
        assert!(answer(&n, true));
        wait_for(&n, "bytes 4", 120, |v| {
            v.transfer
                .as_ref()
                .is_some_and(|t| t.files.first().is_some_and(|f| f.done > (8 << 20)))
        });
        let t0 = std::time::Instant::now();
        println!(
            "  CANCEL NOW at {:?}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );
        assert!(cancel(&n));
        stage_is(&n, "cancelled 4", 5, |s| *s == Stage::Cancelled);
        println!("  native cancelled in {:?}", t0.elapsed());
        std::thread::sleep(Duration::from_millis(500));
        let parts = std::fs::read_dir(&dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".oeca-part")
            })
            .count();
        println!("  partial files left: {parts}");
        assert_eq!(parts, 0);

        println!(
            "STEP 5: tap Forget this device in the browser, then Oeca sends: the browser must say revoked; decline there"
        );
        std::thread::sleep(Duration::from_secs(15));
        let small = dir.join("after-forget.txt");
        std::fs::write(&small, b"y").unwrap();
        assert!(send(&n, &web.id, vec![small]));
        let t = stage_is(&n, "done 5", 180, |s| ended(s));
        println!("  native: {:?}", t.stage);
        drop(n);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One send to nearby-web, with a fixed identity folder (`OECA_NEARBY_TEST_DIR`)
    /// so the browser can recognise this Oeca across runs. `OECA_NEARBY_MB`: size
    /// (0: a small text file); `OECA_NEARBY_CANCEL_AFTER_MB`: cancel mid-way.
    #[test]
    #[ignore]
    fn nearby_live_browser_send() {
        let alias = std::env::var("OECA_NEARBY_WEB").expect("OECA_NEARBY_WEB=<web alias>");
        let dir =
            PathBuf::from(std::env::var("OECA_NEARBY_TEST_DIR").expect("OECA_NEARBY_TEST_DIR"));
        let mb: usize = std::env::var("OECA_NEARBY_MB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let cancel_after: Option<u64> = std::env::var("OECA_NEARBY_CANCEL_AFTER_MB")
            .ok()
            .and_then(|v| v.parse().ok());
        let n = live("Oeca live", &dir);
        let v = wait_for(&n, "web peer", 30, |v| {
            v.peers.iter().any(|p| p.alias == alias)
        });
        let web = v.peers.iter().find(|p| p.alias == alias).unwrap().clone();
        if std::env::var("OECA_NEARBY_RECEIVE").is_ok() {
            println!("READY: send a file from the browser");
            let t = wait_for(&n, "incoming", 180, |v| {
                v.transfer.as_ref().is_some_and(|t| {
                    t.direction == Direction::Receive
                        && (t.code().is_some() || t.capability == Capability::Stock)
                })
            })
            .transfer
            .unwrap();
            println!(
                "native receiver: trust {:?} code {:?} files {:?}",
                t.identity,
                t.code(),
                t.files
            );
            asked(&n, "approval", 180);
            assert!(answer(&n, true));
            let t = stage_is(&n, "received", 120, |s| {
                matches!(
                    s,
                    Stage::Done | Stage::Failed(_) | Stage::Declined | Stage::Cancelled
                )
            });
            println!("native end: {:?} trust {:?}", t.stage, t.identity);
            return;
        }
        let src = dir.join(if mb == 0 { "hello.txt" } else { "big.bin" });
        std::fs::write(
            &src,
            if mb == 0 {
                b"hello".to_vec()
            } else {
                vec![3u8; mb << 20]
            },
        )
        .unwrap();
        assert!(send(&n, &web.id, vec![src.clone()]));
        let t = stage_is(&n, "connected", 120, |s| {
            !matches!(s, Stage::Signaling | Stage::Connecting)
        });
        println!(
            "native: {:?} trust {:?} code {:?}",
            t.stage,
            t.identity,
            t.code()
        );
        if t.stage == Stage::Verification {
            assert!(codes_match(&n));
        }
        if let Some(after) = cancel_after {
            wait_for(&n, "bytes", 300, |v| {
                v.transfer
                    .as_ref()
                    .is_some_and(|t| t.files[0].done > (after << 20))
            });
            println!(
                "CANCEL at {}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis()
            );
            assert!(cancel(&n));
        }
        let t = stage_is(&n, "end", 300, |s| {
            matches!(
                s,
                Stage::Done | Stage::Failed(_) | Stage::Declined | Stage::Cancelled
            )
        });
        println!("native end: {:?}", t.stage);
        let _ = std::fs::remove_file(&src);
    }

    /// Strict policy against the unmodified LocalSend Web (`OECA_NEARBY_WEB` =
    /// its alias; it accepts incoming files by itself). Oeca: Unverifiable, no
    /// code, `verify` refused, `allow_unverified_once` for one transfer, never
    /// remembered. Then send one file from LocalSend Web, twice (printed steps).
    #[test]
    #[ignore]
    fn nearby_live_stock_localsend_web() {
        let alias = std::env::var("OECA_NEARBY_WEB").expect("OECA_NEARBY_WEB=<web alias>");
        let dir =
            crate::data_dir::temp_dir().join(format!("oeca-nearby-stock-{}", std::process::id()));
        let n = live("Oeca live", &dir);
        let v = wait_for(&n, "web peer", 30, |v| {
            v.peers.iter().any(|p| p.alias == alias)
        });
        let web = v.peers.iter().find(|p| p.alias == alias).unwrap().clone();
        let src = dir.join("to-stock.txt");
        std::fs::write(&src, b"to stock LocalSend Web").unwrap();
        let ended = |s: &Stage| {
            matches!(
                s,
                Stage::Done | Stage::Failed(_) | Stage::Declined | Stage::Cancelled
            )
        };

        for round in 0..2 {
            assert!(send(&n, &web.id, vec![src.clone()]));
            let t = stage_is(&n, "held", 60, |s| *s == Stage::Unverified);
            println!("send {round}: trust {:?} code {:?}", t.identity, t.code());
            assert_eq!((t.capability, t.code()), (Capability::Stock, None));
            // "Codes match" means nothing without a code: still held
            assert!(!codes_match(&n), "nothing to verify");
            assert!(answer(&n, false));
            assert!(send(&n, &web.id, vec![src.clone()]));
            stage_is(&n, "held again", 60, |s| *s == Stage::Unverified);
            assert!(click(&n, |t| n.allow_unverified_once(&t.id)));
            let t = stage_is(&n, "sent once", 60, |s| ended(s));
            println!("  once: {:?}, trust still {:?}", t.stage, t.identity);
            assert_eq!(t.stage, Stage::Done);
            assert_eq!(t.capability, Capability::Stock);
        }

        println!("STEP: send a file from LocalSend Web to Oeca live (twice)");
        let receive =
            |t: &Transfer| t.direction == Direction::Receive && t.stage == Stage::Approval;
        let t = wait_for(&n, "incoming", 180, |v| {
            v.transfer.as_ref().is_some_and(receive)
        })
        .transfer
        .unwrap();
        println!(
            "incoming: trust {:?} code {:?} files {:?}",
            t.identity,
            t.code(),
            t.files
        );
        assert_eq!((t.capability, t.code()), (Capability::Stock, None));
        assert!(!answer(&n, true));
        assert!(answer(&n, false));
        let t = stage_is(&n, "refused", 30, |s| ended(s));
        println!("  accept+verify on unverifiable: {:?}", t.stage);
        assert_eq!(t.stage, Stage::Declined);
        let t = wait_for(&n, "incoming again", 180, |v| {
            v.transfer.as_ref().is_some_and(receive)
        })
        .transfer
        .unwrap();
        assert_eq!(t.capability, Capability::Stock);
        assert!(click(&n, |t| n.allow_unverified_once(&t.id)));
        let t = stage_is(&n, "received once", 60, |s| ended(s));
        println!("  once: {:?} saved {:?}", t.stage, t.saved);
        assert_eq!(t.stage, Stage::Done);
        drop(n);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
