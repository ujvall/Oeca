//! The decision API's state machine. It has no I/O but the device store, and
//! it is the only place a transfer's identity, stage or decision can change.
//!
//! ```text
//! send     Signaling -> Connecting -+-> Verification ---verify--+
//!                                   +-> NameCheck -confirm_name-+-> AwaitingPeer -> Transferring
//!                                   +-> (trusted) --------------+
//!                                   +-> Unverified -----once----+
//! receive  Connecting --------------+-> Verification ---verify--+
//!                                   +-> NameCheck -confirm_name-+-> AwaitingOffer -> Approval
//!                                   +-> (trusted) --------------+
//!                                   +-> (stock: needs the list) ------------------> Approval
//!          Approval --respond (trusted) / once (stock)--> Transferring
//! any      -> Done | Declined | Cancelled | Failed   (final)
//! ```
//!
//! - A decision names its transfer's id (random, made here, never sent) and
//!   is checked and applied at once, under one lock: there is no queue that
//!   could deliver it to another transfer.
//! - It exists only in the stage that asks for it: the gate it opens is
//!   created when that stage begins and dropped when it ends.
//! - It counts only `ARMING` after that stage was first handed to the UI.
//! - Identity, names and trust come only from `proof`'s types, and are
//!   cleared when a transfer ends without succeeding.

use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;

use localsend::webrtc::signaling::WsServerSdpMessage;

use super::devices::{Devices, Standing, TrustError};
use super::proof::{Authenticated, ClaimedFingerprint, Connected, Fingerprint};
use super::{Peer, View};

/// How long a question is on screen before an answer to it counts.
pub const ARMING: Duration = Duration::from_millis(700);
const MAX_WHY: usize = 300;

/// Random (128 bits or more), local, never sent.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TransferId(String);

impl TransferId {
    pub(super) fn new() -> Self {
        Self(super::osv::random())
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Send,
    Receive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileRow {
    pub name: String,
    pub size: u64,
    pub done: u64,
    /// None while pending; Some(false) for a failed or skipped file.
    pub ok: Option<bool>,
}

impl FileRow {
    /// A type that runs code or opens something elsewhere: the approval and
    /// the received list must say so (`name` is the name it is saved as).
    pub fn dangerous(&self) -> bool {
        super::dangerous(&self.name)
    }
}

/// What signaling said. None of it is proven, and none of it is a name to trust.
#[derive(Clone, Debug, PartialEq)]
pub struct Claimed {
    pub peer: Peer,
    pub fingerprint: Option<ClaimedFingerprint>,
}

/// What the peer can do, which says nothing about who it is. Claimed until
/// the connection is `Authenticated`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    Unknown,
    /// No Oeca extension (stock LocalSend, or stripped on the way).
    Stock,
    /// Says it is Oeca Nearby. Without `Connection::Authenticated` its
    /// authentication is incomplete or failed.
    Oeca,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Connection {
    /// Nothing is proven yet.
    Claimed,
    /// The key is proven, the device is not (a peer without the extension).
    Connected(Connected),
    Authenticated(Authenticated),
    /// Ended or failed: proves nothing any more.
    Closed,
}

/// Who it is, decided only for a connected key. Separate from `Capability`.
#[derive(Clone, Debug, PartialEq)]
pub enum Identity {
    New,
    /// Only ever with `Connection::Authenticated`. `name` is the confirmed
    /// one: show it, never the live alias.
    Trusted {
        name: String,
        name_mismatch: bool,
    },
    /// An unknown key using a known device's name.
    Changed,
    Revoked,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// The other Oeca is in another transfer, or limiting attempts. Not a downgrade.
    Busy,
    TooManyAttempts,
    /// An Oeca peer whose authentication did not complete.
    Authentication(String),
    /// A known device without the extension: refused, never offered "once".
    Downgraded,
    /// Trust was withdrawn during the transfer.
    Revoked,
    TrustUnavailable,
    Other(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    /// Offer sent / answer pending.
    Signaling,
    /// ICE, DTLS and authentication. Nothing can be decided.
    Connecting,
    /// Authenticated, not trusted: compare the code (`verify`).
    Verification,
    /// Trusted, under another name: `confirm_name`.
    NameCheck,
    /// Send to a stock peer: `allow_unverified_once`.
    Unverified,
    /// Receive: the file list has not arrived.
    AwaitingOffer,
    /// Send: the receiver decides.
    AwaitingPeer,
    /// Receive: the whole list is in the View: `respond` (stock: `allow_unverified_once`).
    Approval,
    Transferring,
    Done,
    Declined,
    Cancelled,
    Failed(Failure),
}

impl Failure {
    /// Some of these quote the peer.
    fn bounded(self) -> Self {
        let short = |why: String| why.chars().take(MAX_WHY).collect();
        match self {
            Self::Other(why) => Self::Other(short(why)),
            Self::Authentication(why) => Self::Authentication(short(why)),
            other => other,
        }
    }
}

impl Stage {
    pub fn asks(&self) -> bool {
        matches!(
            self,
            Self::Verification | Self::NameCheck | Self::Unverified | Self::Approval
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Transfer {
    /// Every decision takes this.
    pub id: TransferId,
    pub direction: Direction,
    pub claimed: Claimed,
    pub capability: Capability,
    pub connection: Connection,
    pub identity: Option<Identity>,
    pub stage: Stage,
    /// In a stage that asks: when answers start to count.
    pub decide_after: Option<Instant>,
    pub files: Vec<FileRow>,
    pub saved: Vec<PathBuf>,
}

impl Transfer {
    /// The code to compare: only of an authenticated connection.
    pub fn code(&self) -> Option<&str> {
        match &self.connection {
            Connection::Authenticated(peer) => Some(peer.code()),
            _ => None,
        }
    }

    fn key(&self) -> Option<&Fingerprint> {
        match &self.connection {
            Connection::Authenticated(peer) => Some(peer.fingerprint()),
            Connection::Connected(peer) => Some(peer.fingerprint()),
            Connection::Claimed | Connection::Closed => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// That transfer is over, replaced, or never was.
    NotCurrent,
    WrongStage,
    /// The question has not been on screen for `ARMING`.
    NotYet,
    /// Revoked, changed, not stock, or no longer what was displayed.
    NotEligible,
    Busy,
    Offline,
    /// The selected peer left, or is not what was displayed.
    PeerChanged,
    /// The device store refused. The question stays open.
    Trust(TrustError),
}

/// How a connection introduced itself.
pub(super) enum Proof {
    Stock(Connected),
    Oeca(Authenticated),
}

/// The task waits on it; None: no question, go on.
pub(super) type Gate = Option<oneshot::Receiver<bool>>;

struct Live {
    id: TransferId,
    /// None until its task runs.
    task: Option<AbortHandle>,
    /// Exists exactly while the stage asks.
    gate: Option<oneshot::Sender<bool>>,
    /// When the stage was first handed out by `snapshot`.
    seen: Option<Instant>,
    /// Passed authentication and the decisions before the file list.
    admitted: bool,
    /// Send: the peer the offer went to, and where its answers go.
    target: Option<(String, mpsc::Sender<WsServerSdpMessage>)>,
}

#[derive(Default)]
pub(super) struct Board {
    pub view: View,
    live: Option<Live>,
    /// A finished transfer's task, still closing its connection.
    closing: Option<AbortHandle>,
}

fn identity(standing: Standing) -> Option<Identity> {
    Some(match standing {
        Standing::New => Identity::New,
        Standing::Trusted { name, conflict } => Identity::Trusted {
            name,
            name_mismatch: conflict,
        },
        Standing::Changed => Identity::Changed,
        Standing::Revoked => Identity::Revoked,
        Standing::Unavailable => return None,
    })
}

fn same_kind(a: &Option<Identity>, b: &Option<Identity>) -> bool {
    a.as_ref().map(std::mem::discriminant) == b.as_ref().map(std::mem::discriminant)
}

impl Board {
    pub fn busy(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| l.task.as_ref().is_none_or(|t| !t.is_finished()))
    }

    /// An incoming connection that has proven and asked nothing yet: the
    /// user's own send may take its place.
    pub fn unproven_incoming(&self) -> bool {
        self.view.transfer.as_ref().is_some_and(|t| {
            self.is_live(&t.id) && t.direction == Direction::Receive && t.stage == Stage::Connecting
        })
    }

    pub fn is_live(&self, id: &TransferId) -> bool {
        self.live.as_ref().is_some_and(|l| l.id == *id)
    }

    /// A new attempt, with a new id. Whatever was there is over.
    pub fn begin(&mut self, direction: Direction, peer: Peer) -> TransferId {
        if let Some(old) = self.live.take().and_then(|l| l.task) {
            old.abort();
        }
        if let Some(old) = self.closing.take() {
            old.abort();
        }
        let id = TransferId::new();
        self.view.transfer = Some(Transfer {
            id: id.clone(),
            direction,
            claimed: Claimed {
                peer,
                fingerprint: None,
            },
            capability: Capability::Unknown,
            connection: Connection::Claimed,
            identity: None,
            stage: match direction {
                Direction::Send => Stage::Signaling,
                Direction::Receive => Stage::Connecting,
            },
            decide_after: None,
            files: Vec::new(),
            saved: Vec::new(),
        });
        self.live = Some(Live {
            id: id.clone(),
            task: None,
            gate: None,
            seen: None,
            admitted: false,
            target: None,
        });
        id
    }

    pub fn attach(
        &mut self,
        id: &TransferId,
        task: AbortHandle,
        target: Option<(String, mpsc::Sender<WsServerSdpMessage>)>,
    ) {
        match self.live.as_mut().filter(|l| l.id == *id) {
            Some(live) => {
                live.task = Some(task);
                live.target = target;
            }
            None => task.abort(),
        }
    }

    /// An ANSWER reaches the sending task only from the peer its offer went to.
    pub fn answer_route(&self, from: &str) -> Option<mpsc::Sender<WsServerSdpMessage>> {
        let (target, tx) = self.live.as_ref()?.target.as_ref()?;
        (target == from).then(|| tx.clone())
    }

    /// The transfer, while it runs.
    pub fn live_transfer(&self, id: &TransferId) -> Option<&Transfer> {
        self.view
            .transfer
            .as_ref()
            .filter(|t| t.id == *id && self.is_live(id))
    }

    pub fn set_files(&mut self, id: &TransferId, files: Vec<FileRow>) {
        if let Some(t) = self.current(id) {
            t.files = files;
        }
    }

    pub fn edit_files(
        &mut self,
        id: &TransferId,
        f: impl FnOnce(&mut [FileRow], &mut Vec<PathBuf>),
    ) {
        if let Some(t) = self.current(id) {
            f(&mut t.files, &mut t.saved);
        }
    }

    /// The receiver took the offer.
    pub fn sending(&mut self, id: &TransferId) {
        if self
            .live_transfer(id)
            .is_some_and(|t| t.stage == Stage::AwaitingPeer)
        {
            self.enter(id, Stage::Transferring);
        }
    }

    // Private: nothing outside this module writes a stage, identity or gate.
    fn current(&mut self, id: &TransferId) -> Option<&mut Transfer> {
        if !self.is_live(id) {
            return None;
        }
        self.view.transfer.as_mut().filter(|t| t.id == *id)
    }

    fn enter(&mut self, id: &TransferId, stage: Stage) {
        if let Some(t) = self.current(id) {
            t.stage = stage;
            t.decide_after = None;
            if let Some(live) = self.live.as_mut() {
                live.seen = None;
                live.gate = None;
            }
        }
    }

    fn ask(&mut self, id: &TransferId, stage: Stage) -> Gate {
        self.enter(id, stage);
        let (tx, rx) = oneshot::channel();
        self.live.as_mut()?.gate = Some(tx);
        Some(rx)
    }

    /// Signaling spoke. It sets only what is claimed.
    pub fn claim(
        &mut self,
        id: &TransferId,
        fingerprint: Option<ClaimedFingerprint>,
        capability: Capability,
    ) {
        let Some(t) = self
            .current(id)
            .filter(|t| t.connection == Connection::Claimed)
        else {
            return;
        };
        t.claimed.fingerprint = fingerprint;
        t.capability = capability;
        self.enter(id, Stage::Connecting);
    }

    /// The only way a transfer gets an identity.
    pub fn admit(
        &mut self,
        id: &TransferId,
        proof: Proof,
        devices: &Devices,
    ) -> Result<Gate, Failure> {
        let gone = || Failure::Other("The transfer is over".into());
        let t = self.current(id).ok_or_else(gone)?;
        if t.connection != Connection::Claimed {
            return Err(gone());
        }
        let alias = t.claimed.peer.alias.clone();
        let direction = t.direction;
        let (oeca, standing) = match &proof {
            Proof::Oeca(peer) => (true, devices.standing(peer.fingerprint(), &alias)),
            Proof::Stock(peer) => (false, devices.standing(peer.fingerprint(), &alias)),
        };
        // Offering Oeca and not completing it is not a stock peer
        if (t.capability == Capability::Oeca) != oeca {
            return Err(Failure::Authentication("Incomplete".into()));
        }
        if standing == Standing::Unavailable {
            return Err(Failure::TrustUnavailable);
        }
        t.connection = match proof {
            Proof::Oeca(peer) => Connection::Authenticated(peer),
            Proof::Stock(peer) => Connection::Connected(peer),
        };
        // Unauthenticated, a known key keeps its warnings and never its name
        t.identity = match (&standing, oeca) {
            (Standing::Trusted { .. }, false) => None,
            _ => identity(standing.clone()),
        };
        let question = if oeca {
            match standing {
                Standing::Trusted {
                    conflict: false, ..
                } => None,
                Standing::Trusted { .. } => Some(Stage::NameCheck),
                _ => Some(Stage::Verification),
            }
        } else if standing != Standing::New {
            return Err(Failure::Downgraded);
        } else if direction == Direction::Send {
            Some(Stage::Unverified)
        } else {
            None
        };
        Ok(match question {
            Some(stage) => self.ask(id, stage),
            None => {
                self.pass(id, direction);
                None
            }
        })
    }

    fn pass(&mut self, id: &TransferId, direction: Direction) {
        self.enter(
            id,
            match direction {
                Direction::Send => Stage::AwaitingPeer,
                Direction::Receive => Stage::AwaitingOffer,
            },
        );
        if let Some(live) = self.live.as_mut() {
            live.admitted = true;
        }
    }

    /// The complete list: it enters the View together with the Approval stage.
    pub fn offer(&mut self, id: &TransferId, files: Vec<FileRow>) -> Gate {
        let admitted = self.live.as_ref().is_some_and(|l| l.admitted);
        let t = self.current(id)?;
        if !admitted || t.stage != Stage::AwaitingOffer {
            return None;
        }
        t.files = files;
        self.ask(id, Stage::Approval)
    }

    /// Before each thing that gives the peer something: still allowed?
    pub fn still_allowed(&self, id: &TransferId, devices: &Devices) -> Result<(), Failure> {
        let over = || Failure::Other("The transfer is over".into());
        let live = self.live.as_ref().filter(|l| l.id == *id && l.admitted);
        let t = self
            .view
            .transfer
            .as_ref()
            .filter(|_| live.is_some())
            .ok_or_else(over)?;
        let alias = &t.claimed.peer.alias;
        let standing = |key| devices.standing(key, alias);
        match &t.connection {
            Connection::Authenticated(p) => match standing(p.fingerprint()) {
                Standing::Trusted { .. } => Ok(()),
                Standing::Unavailable => Err(Failure::TrustUnavailable),
                _ => Err(Failure::Revoked),
            },
            Connection::Connected(p) => match standing(p.fingerprint()) {
                Standing::New => Ok(()),
                Standing::Unavailable => Err(Failure::TrustUnavailable),
                _ => Err(Failure::Revoked),
            },
            Connection::Claimed | Connection::Closed => Err(over()),
        }
    }

    /// The device store changed: end what it no longer allows.
    pub fn recheck(&mut self, devices: &Devices) {
        let Some(t) = &self.view.transfer else { return };
        let Some(key) = t.key() else { return };
        let standing = devices.standing(key, &t.claimed.peer.alias);
        let unavailable = standing == Standing::Unavailable;
        if same_kind(&t.identity, &identity(standing)) {
            return;
        }
        let id = t.id.clone();
        if self.is_live(&id) {
            let why = if unavailable {
                Failure::TrustUnavailable
            } else {
                Failure::Revoked
            };
            self.kill(&id, Stage::Failed(why));
        } else if let Some(t) = self.view.transfer.as_mut() {
            t.connection = Connection::Closed;
            t.identity = None;
        }
    }

    /// Final, once. Its decisions die with it.
    pub fn end(&mut self, id: &TransferId, stage: Stage) {
        let Some(t) = self.current(id) else { return };
        if stage != Stage::Done {
            t.connection = Connection::Closed;
            // A warning may stay; trust may not
            if matches!(t.identity, Some(Identity::Trusted { .. })) {
                t.identity = None;
            }
        }
        t.stage = match stage {
            Stage::Failed(why) => Stage::Failed(why.bounded()),
            stage => stage,
        };
        t.decide_after = None;
        if let Some(live) = self.live.take() {
            self.closing = live.task;
        }
    }

    /// Final, and its task is stopped now (dropping the session closes it).
    pub fn kill(&mut self, id: &TransferId, stage: Stage) {
        if self.is_live(id) {
            self.end(id, stage);
            if let Some(task) = self.closing.take() {
                task.abort();
            }
        }
    }

    /// The room is gone, and the decisions and peers that belonged to it.
    pub fn end_session(&mut self, stage: Stage) {
        if let Some(id) = self.live.as_ref().map(|l| l.id.clone()) {
            self.kill(&id, stage);
        }
        self.view.me = None;
        self.view.peers.clear();
    }

    /// Signaling is gone. Only what still needs it ends: an offer with no answer yet.
    pub fn leave_room(&mut self, stage: Stage) {
        let unanswered = self
            .view
            .transfer
            .as_ref()
            .filter(|t| t.stage == Stage::Signaling)
            .map(|t| t.id.clone());
        if let Some(id) = unanswered {
            self.kill(&id, stage);
        }
        self.view.me = None;
        self.view.peers.clear();
    }

    /// What the UI gets. A question counts as shown from here.
    pub fn snapshot(&mut self, now: Instant) -> View {
        let asks = self.view.transfer.as_ref().is_some_and(|t| t.stage.asks());
        if let Some(live) = self.live.as_mut().filter(|_| asks) {
            let seen = *live.seen.get_or_insert(now);
            if let Some(t) = self.view.transfer.as_mut() {
                t.decide_after = Some(seen + ARMING);
            }
        }
        self.view.clone()
    }

    /// The checks every decision shares.
    fn question(&self, id: &TransferId, now: Instant) -> Result<&Transfer, Refused> {
        let live = self
            .live
            .as_ref()
            .filter(|l| l.id == *id)
            .ok_or(Refused::NotCurrent)?;
        let t = self
            .view
            .transfer
            .as_ref()
            .filter(|t| t.id == *id)
            .ok_or(Refused::NotCurrent)?;
        let gate = live
            .gate
            .as_ref()
            .filter(|_| t.stage.asks())
            .ok_or(Refused::WrongStage)?;
        if gate.is_closed() || live.task.as_ref().is_some_and(|task| task.is_finished()) {
            return Err(Refused::NotCurrent);
        }
        match live.seen {
            Some(seen) if now.saturating_duration_since(seen) >= ARMING => Ok(t),
            _ => Err(Refused::NotYet),
        }
    }

    fn open_gate(&mut self, yes: bool) -> Result<(), Refused> {
        let gate = self.live.as_mut().and_then(|l| l.gate.take());
        gate.and_then(|g| g.send(yes).ok())
            .ok_or(Refused::NotCurrent)
    }

    /// The user compared the code and names the device.
    pub fn verify(
        &mut self,
        id: &TransferId,
        name: &str,
        devices: &Devices,
        now: Instant,
    ) -> Result<(), Refused> {
        let t = self.question(id, now)?;
        if t.stage != Stage::Verification {
            return Err(Refused::WrongStage);
        }
        let Connection::Authenticated(peer) = t.connection.clone() else {
            return Err(Refused::NotEligible);
        };
        let (alias, direction) = (t.claimed.peer.alias.clone(), t.direction);
        // Exactly what was displayed: a revocation meanwhile is not undone
        if !same_kind(
            &t.identity,
            &identity(devices.standing(peer.fingerprint(), &alias)),
        ) {
            return Err(Refused::NotEligible);
        }
        devices.pin(&peer, name, &alias).map_err(Refused::Trust)?;
        let trusted = identity(devices.standing(peer.fingerprint(), &alias));
        if !matches!(trusted, Some(Identity::Trusted { .. })) {
            return Err(Refused::NotEligible);
        }
        if let Some(t) = self.current(id) {
            t.identity = trusted;
        }
        // Saved, so it is done, even if the connection ended this instant
        let _ = self.open_gate(true);
        self.pass(id, direction);
        devices.refund_attempt(id.as_str());
        Ok(())
    }

    /// The user means this trusted device, whatever it calls itself today.
    /// This transfer only: the record keeps its name.
    pub fn confirm_name(
        &mut self,
        id: &TransferId,
        devices: &Devices,
        now: Instant,
    ) -> Result<(), Refused> {
        let t = self.question(id, now)?;
        if t.stage != Stage::NameCheck {
            return Err(Refused::WrongStage);
        }
        let (Connection::Authenticated(peer), Some(Identity::Trusted { name: shown, .. })) =
            (&t.connection, &t.identity)
        else {
            return Err(Refused::NotEligible);
        };
        match devices.standing(peer.fingerprint(), &t.claimed.peer.alias) {
            Standing::Trusted { name, .. } if name == *shown => {}
            _ => return Err(Refused::NotEligible),
        }
        let direction = t.direction;
        self.open_gate(true)?;
        self.pass(id, direction);
        devices.refund_attempt(id.as_str());
        Ok(())
    }

    /// `true` only at Approval, for a trusted device. `false` refuses
    /// whatever question is on screen.
    pub fn respond(
        &mut self,
        id: &TransferId,
        accept: bool,
        devices: &Devices,
        now: Instant,
    ) -> Result<(), Refused> {
        let t = self.question(id, now)?;
        let trusted = match &t.connection {
            Connection::Authenticated(p) => matches!(
                devices.standing(p.fingerprint(), &t.claimed.peer.alias),
                Standing::Trusted { .. }
            ),
            _ => false,
        };
        if accept {
            if t.stage != Stage::Approval {
                return Err(Refused::WrongStage);
            }
            if !trusted {
                return Err(Refused::NotEligible);
            }
        }
        self.open_gate(accept)?;
        if accept {
            self.enter(id, Stage::Transferring);
        } else {
            self.end(id, Stage::Declined);
        }
        if trusted {
            devices.refund_attempt(id.as_str());
        }
        Ok(())
    }

    /// Only an unknown stock peer, and never remembered.
    pub fn allow_once(
        &mut self,
        id: &TransferId,
        devices: &Devices,
        now: Instant,
    ) -> Result<(), Refused> {
        let t = self.question(id, now)?;
        if !matches!(t.stage, Stage::Unverified | Stage::Approval) {
            return Err(Refused::WrongStage);
        }
        let eligible = t.capability == Capability::Stock
            && t.identity == Some(Identity::New)
            && matches!(&t.connection, Connection::Connected(p)
                if devices.standing(p.fingerprint(), &t.claimed.peer.alias) == Standing::New);
        if !eligible {
            return Err(Refused::NotEligible);
        }
        let (stage, direction) = (t.stage.clone(), t.direction);
        self.open_gate(true)?;
        if stage == Stage::Approval {
            self.enter(id, Stage::Transferring);
        } else {
            self.pass(id, direction);
        }
        Ok(())
    }

    pub fn cancel(&mut self, id: &TransferId) -> Result<(), Refused> {
        if !self.is_live(id) {
            return Err(Refused::NotCurrent);
        }
        self.kill(id, Stage::Cancelled);
        Ok(())
    }
}
