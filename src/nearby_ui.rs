//! What the Nearby UI shows and what its buttons do. Pure: built from a
//! `nearby::View` snapshot (and the device list), drawn by the notch's Nearby
//! space and the Settings window's Nearby page. Every decision names the
//! transfer it was shown for (`Card::id`); the runtime refuses it for any
//! other (`Refused::NotCurrent`), so a stale button can never decide a newer
//! transfer. Nothing here talks to the network or the disk but `perform`'s
//! calls into the runtime and the device store.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::nearby::{
    Capability, Connection, Device, DeviceState, Direction, Failure, FileRow, Identity, Link,
    Nearby, Peer, Refused, Retry, Stage, Status, Transfer, TransferId, TrustError, View,
};

/// What a button does. Decisions go to the transfer the card was built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Approval, from a trusted device: `respond(id, true)`.
    Accept,
    /// Approval, from a stock LocalSend peer: `allow_unverified_once`.
    AcceptOnce,
    /// Refuses what is being asked (approval, code, name): `respond(id, false)`.
    Decline,
    /// The codes match: `verify(id, name)`.
    CodesMatch,
    /// It is the trusted device under a new name: `confirm_name`.
    ConfirmName,
    /// Send once to a stock LocalSend peer: `allow_unverified_once`.
    SendOnce,
    Cancel,
    /// A finished transfer's card goes away (UI only).
    Dismiss,
    /// Opens `Downloads\Oeca Hop` (never a file).
    ShowFolder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    pub action: Action,
    pub label: &'static str,
    /// The one filled (call-to-action) button; at most one per card.
    pub primary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Good,
    Warn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    Pending,
    /// Received and saved, or sent and confirmed by the receiver.
    Done,
    /// Failed or skipped.
    NotDone,
    /// The transfer ended before this file was confirmed either way.
    NotConfirmed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLine {
    /// The name without its extension (may be cut when drawn), and the
    /// extension with its dot (never cut).
    pub stem: String,
    pub ext: String,
    pub size: String,
    pub state: FileState,
    /// Runs programs or opens other places (`FileRow::dangerous`).
    pub dangerous: bool,
}

/// One transfer, as shown. Built only from the runtime's snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub id: TransferId,
    pub direction: Direction,
    /// The stage asks a question (its buttons are decisions).
    pub asks: bool,
    /// When the runtime starts to count an answer (`Transfer::decide_after`).
    pub decide_after: Option<Instant>,
    pub title: String,
    pub identity: Option<(&'static str, Tone)>,
    pub note: String,
    pub warn: bool,
    /// The six digits to compare (Verification only).
    pub code: Option<String>,
    pub files: Vec<FileLine>,
    /// "3 files, 12 MB" (and how many are dangerous), when there are files.
    pub summary: String,
    /// Confirmed progress, 0..1 (Transferring only).
    pub progress: Option<f32>,
    pub buttons: Vec<Button>,
    pub finished: bool,
}

impl Card {
    /// A question's buttons work only once both the runtime's arming delay
    /// and the time it has actually been on screen (`shown`) have passed.
    pub fn armed(&self, now: Instant, shown: Option<Instant>) -> bool {
        !self.asks
            || (self.decide_after.is_none_or(|t| now >= t)
                && shown.is_some_and(|s| now.saturating_duration_since(s) >= crate::nearby::ARMING))
    }

    /// Whether `action` is a decision (waits for `armed`); Cancel, Dismiss
    /// and Show in folder never wait.
    pub fn is_decision(action: Action) -> bool {
        !matches!(
            action,
            Action::Cancel | Action::Dismiss | Action::ShowFolder
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PeerLine {
    /// Exactly as the snapshot had it (`send_files` refuses any other).
    pub peer: Peer,
    /// The device's own name: a claim, never an identity.
    pub name: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    /// The setting is off.
    Off,
    /// On, but there is no runtime (test builds, or it failed to start).
    Unavailable(&'static str),
    /// Connecting, offline, or nobody around.
    Message {
        title: String,
        note: String,
    },
    Peers(Vec<PeerLine>),
    Transfer(Box<Card>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// The header's status line.
    pub status: String,
    pub page: Page,
}

pub const OFF_TITLE: &str = "Hop is off";
pub const OFF_NOTE: &str = "Send files between this PC and your other devices.";
pub const OPEN_SETTINGS: &str = "Turn on in Settings";
pub const EMPTY_TITLE: &str = "No devices nearby";
pub const EMPTY_NOTE: &str = "Devices on your network with Oeca Hop or LocalSend open appear here.";
const TEST_BUILD: &str = "Test builds never connect to the signaling servers.";
const NOT_STARTED: &str = "Hop couldn't start. Turn it off and on again in Settings.";

/// The notch's (and Settings') view of Nearby. `running`: the runtime exists.
/// `dismissed`: a finished card the user closed (a new transfer has a new id).
pub fn model(
    enabled: bool,
    running: bool,
    view: Option<&View>,
    dismissed: Option<&TransferId>,
) -> Model {
    let Some(view) = view.filter(|_| enabled && running) else {
        let (status, page) = match (enabled, running) {
            (false, _) => ("Off", Page::Off),
            (true, _) if crate::data_dir::ISOLATED => {
                ("Unavailable", Page::Unavailable(TEST_BUILD))
            }
            _ => ("Unavailable", Page::Unavailable(NOT_STARTED)),
        };
        return Model {
            status: status.into(),
            page,
        };
    };
    let status = status_line(&view.link, view.peers.len());
    if let Some(t) = &view.transfer {
        let final_ = is_final(&t.stage);
        if !(final_ && dismissed == Some(&t.id)) {
            return Model {
                status,
                page: Page::Transfer(Box::new(card(t))),
            };
        }
    }
    let page = match (&view.link, view.peers.is_empty()) {
        (Link::Online, false) => Page::Peers(view.peers.iter().map(peer_line).collect()),
        (Link::Online, true) => Page::Message {
            title: EMPTY_TITLE.into(),
            note: EMPTY_NOTE.into(),
        },
        (Link::Connecting, _) => Page::Message {
            title: "Looking for devices\u{2026}".into(),
            note: "Connecting to the signaling server.".into(),
        },
        (Link::Retrying { .. }, _) => Page::Message {
            title: "Offline".into(),
            note: retry_note(&view.link),
        },
    };
    Model { status, page }
}

fn status_line(link: &Link, peers: usize) -> String {
    match link {
        Link::Connecting => "Connecting\u{2026}".into(),
        Link::Online => match peers {
            0 => "On".into(),
            1 => "On \u{00B7} 1 device".into(),
            n => format!("On \u{00B7} {n} devices"),
        },
        Link::Retrying { .. } => "Offline".into(),
    }
}

/// Why it is offline and what happens next. The wait is rounded up (the
/// snapshot's value is not counted down).
fn retry_note(link: &Link) -> String {
    let Link::Retrying { in_secs, why } = link else {
        return String::new();
    };
    let wait = if *in_secs >= 90 {
        format!("about {} min", in_secs.div_ceil(60))
    } else {
        format!("about {} s", (*in_secs).max(1))
    };
    match why {
        Retry::Lost => format!("The connection was lost. Retrying in {wait}."),
        Retry::Requested => "Reconnecting\u{2026}".into(),
        Retry::Rejected | Retry::RateLimited => {
            format!("The signaling server is limiting connections. Retrying in {wait}.")
        }
    }
}

fn peer_line(p: &Peer) -> PeerLine {
    let kind = kind_label(p.kind);
    let detail = match p.model.as_deref().map(crate::nearby::display_name) {
        Some(m) if !m.is_empty() => format!("{kind} \u{00B7} {m}"),
        _ => kind.to_string(),
    };
    PeerLine {
        peer: p.clone(),
        name: shown_alias(&p.alias),
        detail,
    }
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "mobile" => "Phone",
        "desktop" => "Computer",
        "web" => "Browser",
        "headless" | "server" => "Device",
        _ => "Unknown device",
    }
}

fn shown_alias(alias: &str) -> String {
    let a = crate::nearby::display_name(alias);
    if a.is_empty() {
        "Unnamed device".into()
    } else {
        a
    }
}

pub fn is_final(stage: &Stage) -> bool {
    matches!(
        stage,
        Stage::Done | Stage::Declined | Stage::Cancelled | Stage::Failed(_)
    )
}

/// What the identity is, never derived from a name.
fn identity(t: &Transfer) -> Option<(&'static str, Tone)> {
    Some(match (&t.identity, t.capability) {
        (Some(Identity::Trusted { .. }), _) => ("Trusted", Tone::Good),
        (Some(Identity::Changed), _) => ("Identity changed", Tone::Warn),
        (Some(Identity::Revoked), _) => ("Forgotten device", Tone::Warn),
        (Some(Identity::New), Capability::Stock) => ("Unverifiable", Tone::Warn),
        (Some(Identity::New), _) => ("New device", Tone::Plain),
        (None, _) if t.connection == Connection::Closed => return None,
        (None, _) => ("Not verified yet", Tone::Plain),
    })
}

/// The name to show: the one the user confirmed for a trusted device,
/// otherwise the device's own claim, in quotes.
fn who(t: &Transfer) -> String {
    match &t.identity {
        Some(Identity::Trusted { name, .. }) => name.clone(),
        _ => format!("\u{201C}{}\u{201D}", shown_alias(&t.claimed.peer.alias)),
    }
}

/// The warning an identity carries, if any (shown before any decision).
fn identity_warning(t: &Transfer) -> Option<String> {
    let alias = shown_alias(&t.claimed.peer.alias);
    match (&t.identity, t.capability) {
        (Some(Identity::Changed), _) => Some(format!(
            "This isn't the device you verified as \u{201C}{alias}\u{201D}. Don't continue unless you expected it."
        )),
        (Some(Identity::Revoked), _) => {
            Some("You forgot this device. It can't be trusted again from here.".into())
        }
        (Some(Identity::New), Capability::Stock) => Some(
            "The LocalSend app can't prove who it is. Continue only if you know this device."
                .into(),
        ),
        (
            Some(Identity::Trusted {
                name,
                name_mismatch: true,
            }),
            _,
        ) => Some(format!(
            "Your device {name} now calls itself \u{201C}{alias}\u{201D}."
        )),
        _ => None,
    }
}

fn btn(action: Action, label: &'static str, primary: bool) -> Button {
    Button {
        action,
        label,
        primary,
    }
}

fn card(t: &Transfer) -> Card {
    let send = t.direction == Direction::Send;
    let name = who(t);
    let files: Vec<FileLine> = t.files.iter().map(|f| file_line(f, &t.stage)).collect();
    let cancel = || vec![btn(Action::Cancel, "Cancel", false)];
    let done = || vec![btn(Action::Dismiss, "Done", true)];
    let warning = identity_warning(t);
    let mut title = if send {
        format!("To {name}")
    } else {
        format!("From {name}")
    };
    // Every stage sets both
    let note: String;
    let mut buttons: Vec<Button>;
    let mut warn = false;
    let mut progress = None;
    match &t.stage {
        Stage::Signaling => {
            note = "Waiting for the device to answer\u{2026}".into();
            buttons = cancel();
        }
        Stage::Connecting => {
            note = "Connecting\u{2026}".into();
            buttons = cancel();
        }
        Stage::Verification => {
            note = "Check that the other device shows the same code.".into();
            buttons = vec![
                btn(Action::Decline, "Codes differ", false),
                btn(Action::CodesMatch, "Codes match", true),
            ];
        }
        Stage::NameCheck => {
            note = warning.clone().unwrap_or_default() + " Is it the same device?";
            warn = true;
            buttons = vec![
                btn(Action::Decline, "Not mine", false),
                btn(Action::ConfirmName, "Same device", true),
            ];
        }
        Stage::Unverified => {
            note = warning.clone().unwrap_or_default();
            warn = true;
            buttons = vec![
                btn(Action::Cancel, "Cancel", false),
                btn(Action::SendOnce, "Send once", true),
            ];
        }
        Stage::AwaitingOffer => {
            note = "Waiting for the list of files\u{2026}".into();
            buttons = cancel();
        }
        Stage::AwaitingPeer => {
            note = "Waiting for the other device to accept\u{2026}".into();
            buttons = cancel();
        }
        Stage::Approval => {
            title = format!("{name} wants to send");
            let trusted = matches!(t.identity, Some(Identity::Trusted { .. }));
            let stock = t.capability == Capability::Stock && t.identity == Some(Identity::New);
            buttons = vec![btn(Action::Decline, "Decline", !(trusted || stock))];
            if trusted {
                buttons.push(btn(Action::Accept, "Accept", true));
            } else if stock {
                buttons.push(btn(Action::AcceptOnce, "Accept once", true));
            }
            // The identity's warning first, then the files'
            let dangerous = t.files.iter().filter(|f| f.dangerous()).count();
            match (&warning, dangerous) {
                (Some(w), _) => {
                    note = w.clone();
                    warn = true;
                }
                (None, 0) => {
                    note = format!(
                        "Saved to Downloads\\{} and never opened.",
                        crate::nearby::RECEIVE_FOLDER
                    )
                }
                (None, n) => {
                    note = format!(
                        "{n} {} can run programs or open other places. Open only what you trust.",
                        if n == 1 { "file" } else { "files" }
                    );
                    warn = true;
                }
            }
        }
        Stage::Transferring => {
            let (confirmed, n) = (
                t.files.iter().filter(|f| f.ok == Some(true)).count(),
                t.files.len(),
            );
            note = format!(
                "{} \u{00B7} {confirmed} of {n} {}",
                if send { "Sending" } else { "Receiving" },
                if n == 1 { "file" } else { "files" }
            );
            progress = Some(confirmed_progress(&t.files, t.direction));
            buttons = cancel();
        }
        Stage::Done => {
            let all = !files.is_empty() && files.iter().all(|f| f.state == FileState::Done);
            title = match (send, all) {
                (true, true) => format!("Sent to {name}"),
                (false, true) => format!("Received from {name}"),
                _ => "Finished".into(),
            };
            note = if !all {
                warn = true;
                "Some files weren't confirmed.".into()
            } else if send {
                "The other device confirmed every file.".into()
            } else {
                format!(
                    "Saved to Downloads\\{}. Nothing was opened.",
                    crate::nearby::RECEIVE_FOLDER
                )
            };
            buttons = done();
        }
        Stage::Declined => {
            title = if send {
                format!("{name} declined")
            } else {
                "Declined".into()
            };
            note = if send {
                "Nothing was sent.".into()
            } else {
                "Nothing was received.".into()
            };
            buttons = done();
        }
        Stage::Cancelled => {
            title = "Cancelled".into();
            note = partial_note(&files, send);
            buttons = done();
        }
        Stage::Failed(why) => {
            title = if send {
                "Couldn't send".into()
            } else {
                "Couldn't receive".into()
            };
            note = failure_text(why, send);
            warn = true;
            buttons = done();
        }
    }
    let saved_any = !send && !t.saved.is_empty();
    if is_final(&t.stage) && saved_any {
        buttons.insert(0, btn(Action::ShowFolder, "Show in folder", false));
    }
    let dangerous = files.iter().filter(|f| f.dangerous).count();
    let total: u64 = t.files.iter().map(|f| f.size).sum();
    let summary = match files.len() {
        0 => String::new(),
        n => {
            let mut s = format!(
                "{n} {}, {}",
                if n == 1 { "file" } else { "files" },
                crate::convert_batch::human_size(total)
            );
            // Said in the note unless an identity warning took its place
            if dangerous > 0 && (warning.is_some() || t.stage != Stage::Approval) {
                s += &format!(" \u{00B7} {dangerous} can run programs");
            }
            s
        }
    };
    Card {
        id: t.id.clone(),
        direction: t.direction,
        asks: t.stage.asks(),
        decide_after: t.decide_after,
        title,
        identity: identity(t),
        note: note.trim().to_string(),
        warn,
        code: (t.stage == Stage::Verification)
            .then(|| t.code().map(str::to_string))
            .flatten(),
        files,
        summary,
        progress,
        buttons,
        finished: is_final(&t.stage),
    }
}

/// Confirmed bytes over all bytes. Finished files count whole. A file in
/// flight counts only when receiving (what arrived here); when sending, its
/// bytes are only queued until the receiver confirms the file, so it counts
/// nothing yet. Files with no bytes count by number.
fn confirmed_progress(files: &[FileRow], direction: Direction) -> f32 {
    let total: u64 = files.iter().map(|f| f.size).sum();
    if total == 0 {
        let n = files.len().max(1);
        return files.iter().filter(|f| f.ok == Some(true)).count() as f32 / n as f32;
    }
    let done: u64 = files
        .iter()
        .map(|f| match f.ok {
            Some(true) => f.size,
            Some(false) => 0,
            None if direction == Direction::Receive => f.done.min(f.size),
            None => 0,
        })
        .sum();
    (done as f64 / total as f64).clamp(0.0, 1.0) as f32
}

fn partial_note(files: &[FileLine], send: bool) -> String {
    let done = files.iter().filter(|f| f.state == FileState::Done).count();
    match (done, send) {
        (0, true) => "Nothing was sent.".into(),
        (0, false) => "Nothing was received.".into(),
        (n, true) => format!(
            "{n} of {} files were confirmed before it stopped.",
            files.len()
        ),
        (n, false) => format!("{n} of {} files were saved before it stopped.", files.len()),
    }
}

fn failure_text(why: &Failure, send: bool) -> String {
    let nothing = if send {
        "Nothing was sent."
    } else {
        "Nothing was received."
    };
    match why {
        Failure::Busy => "The other device is busy with another transfer.".into(),
        Failure::TooManyAttempts => "Too many attempts. Try again in a few minutes.".into(),
        Failure::Authentication(w) => format!("The device couldn't be verified: {w}"),
        Failure::Downgraded => {
            format!("This device was verified before but can't prove who it is now. {nothing}")
        }
        Failure::Revoked => "The device was forgotten during the transfer.".into(),
        Failure::TrustUnavailable => "Your trusted devices can't be read right now.".into(),
        Failure::Other(w) => w.clone(),
    }
}

fn file_line(f: &FileRow, stage: &Stage) -> FileLine {
    let (stem, ext) = split_name(&f.name);
    FileLine {
        stem,
        ext,
        size: crate::convert_batch::human_size(f.size),
        state: match (f.ok, is_final(stage)) {
            (Some(true), _) => FileState::Done,
            (Some(false), _) => FileState::NotDone,
            (None, true) => FileState::NotConfirmed,
            (None, false) => FileState::Pending,
        },
        dangerous: f.dangerous(),
    }
}

/// "report.final.pdf" -> ("report.final", ".pdf"); a leading dot or no dot:
/// no extension.
pub fn split_name(name: &str) -> (String, String) {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => (name[..i].into(), name[i..].into()),
        _ => (name.into(), String::new()),
    }
}

/// `text` shortened from its start ("\u{2026}tail") until `width` of it fits
/// `room`: an extension too long for its line keeps its end, never spills.
/// `width` measures a string (the renderer's text width).
pub fn fit_tail(text: &str, room: f32, width: impl Fn(&str) -> f32) -> String {
    if width(text) <= room {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let tail = |k: usize| format!("\u{2026}{}", chars[k..].iter().collect::<String>());
    // The longest tail that fits (fewest chars dropped); at least one char
    let (mut lo, mut hi) = (1, chars.len().saturating_sub(1).max(1));
    while lo < hi {
        let mid = (lo + hi) / 2;
        if width(&tail(mid)) <= room {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    tail(lo.min(chars.len().saturating_sub(1)))
}

/// The code as two groups of three ("123 456"), and for screen readers digit
/// by digit ("1 2 3 4 5 6").
pub fn code_text(code: &str) -> (String, String) {
    let digits: Vec<char> = code.chars().collect();
    let shown = if digits.len() == 6 {
        format!(
            "{} {}",
            digits[..3].iter().collect::<String>(),
            digits[3..].iter().collect::<String>()
        )
    } else {
        code.into()
    };
    let spoken = digits
        .iter()
        .map(char::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    (shown, spoken)
}

/// A local name for a device being verified: its own name (or its kind),
/// made unique among the trusted names ("Pixel 2"). The code comparison is
/// what establishes trust; the name only tells devices apart.
pub fn local_name(alias: &str, kind: &str, taken: &[String]) -> String {
    let base = match crate::nearby::display_name(alias) {
        a if a.is_empty() => kind_label(kind).to_string(),
        a => a
            .chars()
            .take(56)
            .collect::<String>()
            .trim_end()
            .to_string(),
    };
    let used = |n: &str| taken.iter().any(|t| t.to_lowercase() == n.to_lowercase());
    if !used(&base) {
        return base;
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|n| !used(n))
        .unwrap_or(base)
}

/// The status line while the file dialog is open for `peer`.
pub fn picking_status(peer: &Peer) -> String {
    format!("Choose files to send to {}", shown_alias(&peer.alias))
}

/// Why an action did not happen, for the status note. None: nothing to say
/// (the snapshot shows what happened).
pub fn refusal_text(r: &Refused) -> Option<&'static str> {
    Some(match r {
        Refused::NotCurrent => "That transfer is no longer current.",
        Refused::WrongStage => return None,
        Refused::NotYet => return None,
        Refused::NotEligible => "That device can't be trusted from here.",
        Refused::Busy => "Another transfer is in progress.",
        Refused::Offline => "Hop is offline.",
        Refused::PeerChanged => "That device left or changed.",
        Refused::Trust(e) => trust_error_text(e),
    })
}

pub fn trust_error_text(e: &TrustError) -> &'static str {
    match e {
        TrustError::Unavailable => "Trusted devices aren't available.",
        TrustError::Unreadable => "Your trusted devices can't be read.",
        TrustError::Paused => "Trust is paused until a forgotten device is saved.",
        TrustError::NotFound => "That device isn't in your list.",
        TrustError::InvalidName => "That name can't be used.",
        TrustError::NameInUse => "Another trusted device has that name.",
        TrustError::Full => "Your list of trusted devices is full.",
        TrustError::Write(_) => "Couldn't save the trusted device.",
        TrustError::NotSaved { .. } => {
            "Forgotten, but not saved yet. The device stays untrusted; try again to save it."
        }
    }
}

/// Runs `action` against the transfer `id` (the card that showed it), whose
/// claimed peer the displayed snapshot had (`peer`: for the local name when
/// the codes match). Only the runtime decides whether it still applies.
pub fn perform(
    nearby: &Nearby,
    id: &TransferId,
    action: Action,
    peer: Option<&Peer>,
) -> Result<(), Refused> {
    match action {
        Action::Accept => nearby.respond(id, true),
        Action::Decline => nearby.respond(id, false),
        Action::AcceptOnce | Action::SendOnce => nearby.allow_unverified_once(id),
        Action::ConfirmName => nearby.confirm_name(id),
        Action::Cancel => nearby.cancel(id),
        Action::CodesMatch => {
            let peer = peer.ok_or(Refused::NotCurrent)?;
            let taken: Vec<String> = nearby
                .devices()
                .list()
                .unwrap_or_default()
                .into_iter()
                .filter(|d| d.state == DeviceState::Trusted)
                .map(|d| d.name)
                .collect();
            nearby.verify(id, &local_name(&peer.alias, peer.kind, &taken))
        }
        Action::Dismiss | Action::ShowFolder => Ok(()),
    }
}

/// The trusted-device list as Settings shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLine {
    pub name: String,
    pub detail: String,
    pub state: &'static str,
    pub fingerprint: crate::nearby::Fingerprint,
    /// Forget (trusted) or save again (forgetting not saved); none for a
    /// forgotten device.
    pub action: Option<&'static str>,
}

pub fn device_lines(list: &[Device]) -> Vec<DeviceLine> {
    list.iter()
        .map(|d| {
            let (state, action) = match d.state {
                DeviceState::Trusted => ("Trusted", Some("Forget")),
                DeviceState::Revoked { .. } => ("Forgotten", None),
                DeviceState::RevocationPending => ("Forgetting not saved", Some("Retry")),
            };
            DeviceLine {
                name: d.name.clone(),
                detail: format!("Verified {} \u{00B7} {}", date(d.verified_at), d.short),
                state,
                fingerprint: d.fingerprint.clone(),
                action,
            }
        })
        .collect()
}

/// The store's state, when it is not simply ready.
pub fn store_note(status: Status) -> Option<&'static str> {
    match status {
        Status::Ready => None,
        Status::Unavailable => Some("Trusted devices aren't available in this build."),
        Status::Unreadable => {
            Some("Your trusted devices can't be read. Nobody is trusted until they can be.")
        }
        Status::Paused => Some(
            "A forgotten device isn't saved yet: all trust is paused. Use Retry or Forget all.",
        ),
    }
}

/// "12 Nov 2026" (UTC) from seconds since 1970.
pub fn date(secs: u64) -> String {
    // Days to civil date (H. Hinnant's algorithm)
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!("{d} {} {y}", MONTHS[(m - 1) as usize])
}

/// Identifies the privacy notice's text: accepting it is recorded as this
/// (`OecaSettings::nearby_notice`), so a changed notice is shown again.
/// FNV-1a, stable across builds; never 0 (0 means "none accepted").
pub fn notice_id() -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in crate::nearby::privacy_notice().bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    h.max(1)
}

/// Whether Nearby is on: its setting, with the current privacy notice
/// accepted. A saved `nearby=true` without it (an older or edited file, or
/// a notice that changed since) is off until the notice is accepted.
pub fn enabled(s: &crate::space::OecaSettings) -> bool {
    s.nearby && s.nearby_notice == notice_id()
}

/// The setting after the user flips it: turning on needs the current notice
/// accepted (`consented`, or accepted before); turning off never asks.
/// Start on launch and every other setting are untouched.
pub fn toggled(mut s: crate::space::OecaSettings, consented: bool) -> crate::space::OecaSettings {
    if enabled(&s) {
        s.nearby = false;
    } else if consented || s.nearby_notice == notice_id() {
        s.nearby_notice = notice_id();
        s.nearby = true;
    }
    s
}

/// The accessible description of the Nearby space.
pub fn accessible(m: &Model, now: Instant, shown: Option<Instant>) -> String {
    match &m.page {
        Page::Off => format!("Hop, off. {OFF_NOTE} {OPEN_SETTINGS} button"),
        Page::Unavailable(why) => format!("Hop, unavailable. {why}"),
        Page::Message { title, note } => format!("Hop, {}. {title}. {note}", m.status),
        Page::Peers(peers) => {
            let names: Vec<String> = peers
                .iter()
                .map(|p| format!("{}, {}, Send files button", p.name, p.detail))
                .collect();
            format!("Hop, {}. {}", m.status, names.join("; "))
        }
        Page::Transfer(c) => {
            let mut s = format!("Hop. {}", c.title);
            if let Some((label, _)) = c.identity {
                s += &format!(", {label}");
            }
            if !c.note.is_empty() {
                s += &format!(". {}", c.note);
            }
            if let Some(code) = &c.code {
                s += &format!(". Code {}", code_text(code).1);
            }
            if !c.summary.is_empty() {
                s += &format!(". {}", c.summary);
            }
            for f in c.files.iter().take(20) {
                s += &format!(". {}{}, {}", f.stem, f.ext, f.size);
                if f.dangerous {
                    s += ", can run programs";
                }
            }
            if let Some(p) = c.progress {
                s += &format!(". {}% confirmed", (p * 100.0).floor());
            }
            let armed = c.armed(now, shown);
            let labels: Vec<String> = c
                .buttons
                .iter()
                .map(|b| {
                    if Card::is_decision(b.action) && !armed {
                        format!("{} button, available shortly", b.label)
                    } else {
                        format!("{} button", b.label)
                    }
                })
                .collect();
            format!("{s}. {}", labels.join(", "))
        }
    }
}

/// How long until a shown question's buttons arm (for one redraw then).
pub fn arm_delay(card: &Card, now: Instant, shown: Instant) -> Option<Duration> {
    if !card.asks {
        return None;
    }
    let at = (shown + crate::nearby::ARMING).max(card.decide_after.unwrap_or(shown));
    (at > now).then(|| at - now)
}

/// Asks for files to send in the system dialog, on a short-lived thread of
/// its own (the dialog runs its own message loop); posts the choice to
/// `hwnd` as `msg` (lParam: a `Box<Option<Vec<PathBuf>>>`, taken back with
/// `Box::from_raw`). False if the thread could not start.
pub fn pick_files_async(hwnd: windows::Win32::Foundation::HWND, msg: u32, peer: &Peer) -> bool {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::UI::Shell::{
        FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FileOpenDialog,
        IFileOpenDialog, SIGDN_FILESYSPATH,
    };
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    let target = hwnd.0 as isize;
    // The dialog names the destination (the device's own name: a claim)
    let title: Vec<u16> = format!("Send to {}", shown_alias(&peer.alias))
        .encode_utf16()
        .chain([0])
        .collect();
    std::thread::Builder::new()
        .name("oeca-nearby-pick".into())
        .spawn(move || {
            let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
            let choice = (|| -> windows::core::Result<Vec<PathBuf>> {
                unsafe {
                    let dialog: IFileOpenDialog =
                        CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
                    dialog.SetOptions(
                        dialog.GetOptions()?
                            | FOS_ALLOWMULTISELECT
                            | FOS_FILEMUSTEXIST
                            | FOS_FORCEFILESYSTEM,
                    )?;
                    dialog.SetTitle(windows::core::PCWSTR(title.as_ptr()))?;
                    dialog.Show(None)?;
                    let items = dialog.GetResults()?;
                    let mut paths = Vec::new();
                    for i in 0..items.GetCount()? {
                        let name = items.GetItemAt(i)?.GetDisplayName(SIGDN_FILESYSPATH)?;
                        let path = PathBuf::from(name.to_string()?);
                        CoTaskMemFree(Some(name.0 as *const _));
                        paths.push(path);
                    }
                    Ok(paths)
                }
            })()
            .ok()
            .filter(|p| !p.is_empty());
            if com {
                unsafe { CoUninitialize() };
            }
            let raw = Box::into_raw(Box::new(choice));
            let hwnd = HWND(target as *mut _);
            if unsafe { PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(raw as isize)) }.is_err() {
                // SAFETY: never posted, so still owned here
                drop(unsafe { Box::from_raw(raw) });
            }
        })
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nearby::{Claimed, Transfer};

    fn peer(alias: &str) -> Peer {
        Peer {
            id: "p1".into(),
            alias: alias.into(),
            model: Some("Pixel\u{202E} 8".into()),
            kind: "mobile",
        }
    }

    fn transfer(direction: Direction, stage: Stage, identity: Option<Identity>) -> Transfer {
        Transfer {
            id: crate::nearby::test_transfer_id(),
            direction,
            claimed: Claimed {
                peer: peer("Pixel"),
                fingerprint: None,
            },
            capability: Capability::Oeca,
            connection: Connection::Claimed,
            identity,
            stage,
            decide_after: None,
            files: Vec::new(),
            saved: Vec::new(),
        }
    }

    fn row(name: &str, size: u64, done: u64, ok: Option<bool>) -> FileRow {
        FileRow {
            name: name.into(),
            size,
            done,
            ok,
        }
    }

    fn online(t: Option<Transfer>, peers: Vec<Peer>) -> View {
        View {
            link: Link::Online,
            peers,
            transfer: t,
            ..View::default()
        }
    }

    fn card_of(m: &Model) -> &Card {
        match &m.page {
            Page::Transfer(c) => c,
            other => panic!("no card: {other:?}"),
        }
    }

    fn actions(c: &Card) -> Vec<Action> {
        c.buttons.iter().map(|b| b.action).collect()
    }

    #[test]
    fn off_and_unavailable_show_no_runtime_state() {
        let v = online(None, vec![peer("A")]);
        assert_eq!(model(false, true, Some(&v), None).page, Page::Off);
        assert_eq!(model(false, false, None, None).page, Page::Off);
        // On without a runtime (a test build): honest, never a device list
        assert!(matches!(
            model(true, false, None, None).page,
            Page::Unavailable(TEST_BUILD)
        ));
        // A stale view while off is ignored
        assert_eq!(model(false, false, Some(&v), None).page, Page::Off);
    }

    #[test]
    fn link_states_peers_and_empty_state() {
        let mut v = online(None, Vec::new());
        assert!(
            matches!(model(true, true, Some(&v), None).page, Page::Message { ref title, .. } if title == EMPTY_TITLE)
        );
        v.peers = vec![peer("Pixel")];
        let m = model(true, true, Some(&v), None);
        assert_eq!(m.status, "On \u{00B7} 1 device");
        let Page::Peers(p) = &m.page else { panic!() };
        assert_eq!(p[0].name, "Pixel");
        // The model string's bidi override is not shown
        assert_eq!(p[0].detail, "Phone \u{00B7} Pixel 8");
        v.link = Link::Connecting;
        assert!(matches!(
            model(true, true, Some(&v), None).page,
            Page::Message { .. }
        ));
        v.link = Link::Retrying {
            in_secs: 900,
            why: Retry::RateLimited,
        };
        let Page::Message { title, note } = model(true, true, Some(&v), None).page else {
            panic!()
        };
        assert_eq!(title, "Offline");
        assert!(note.contains("about 15 min"), "{note}");
    }

    #[test]
    fn every_identity_state_is_distinct_and_never_from_a_name() {
        let cases = [
            (
                Some(Identity::Trusted {
                    name: "My phone".into(),
                    name_mismatch: false,
                }),
                Capability::Oeca,
                "Trusted",
            ),
            (Some(Identity::New), Capability::Oeca, "New device"),
            (Some(Identity::New), Capability::Stock, "Unverifiable"),
            (
                Some(Identity::Changed),
                Capability::Oeca,
                "Identity changed",
            ),
            (
                Some(Identity::Revoked),
                Capability::Oeca,
                "Forgotten device",
            ),
            (None, Capability::Unknown, "Not verified yet"),
        ];
        for (identity, capability, label) in cases {
            let mut t = transfer(Direction::Receive, Stage::Approval, identity.clone());
            t.capability = capability;
            // A claimed name equal to a trusted one changes nothing
            t.claimed.peer.alias = "My phone".into();
            let m = model(true, true, Some(&online(Some(t), vec![])), None);
            assert_eq!(
                card_of(&m).identity.map(|i| i.0),
                Some(label),
                "{identity:?}"
            );
        }
        // Trusted shows the confirmed name, not the live alias
        let t = transfer(
            Direction::Receive,
            Stage::Approval,
            Some(Identity::Trusted {
                name: "My phone".into(),
                name_mismatch: true,
            }),
        );
        let c = model(true, true, Some(&online(Some(t), vec![])), None);
        let c = card_of(&c);
        assert!(c.title.starts_with("My phone"), "{}", c.title);
        assert!(c.note.contains("\u{201C}Pixel\u{201D}") && c.warn);
    }

    #[test]
    fn approval_never_offers_a_weaker_accept() {
        let at = |identity, capability| {
            let mut t = transfer(Direction::Receive, Stage::Approval, Some(identity));
            t.capability = capability;
            t.files = vec![row("a.txt", 3, 0, None)];
            actions(card_of(&model(
                true,
                true,
                Some(&online(Some(t), vec![])),
                None,
            )))
        };
        let trusted = Identity::Trusted {
            name: "P".into(),
            name_mismatch: false,
        };
        assert_eq!(
            at(trusted, Capability::Oeca),
            [Action::Decline, Action::Accept]
        );
        assert_eq!(
            at(Identity::New, Capability::Stock),
            [Action::Decline, Action::AcceptOnce]
        );
        // Changed, revoked, or a new Oeca device (must verify first): decline only
        for i in [Identity::Changed, Identity::Revoked, Identity::New] {
            assert_eq!(at(i.clone(), Capability::Oeca), [Action::Decline], "{i:?}");
        }
        assert_eq!(at(Identity::Revoked, Capability::Stock), [Action::Decline]);
        assert_eq!(at(Identity::Changed, Capability::Stock), [Action::Decline]);
    }

    #[test]
    fn approval_shows_files_extensions_sizes_and_danger() {
        let mut t = transfer(
            Direction::Receive,
            Stage::Approval,
            Some(Identity::Trusted {
                name: "P".into(),
                name_mismatch: false,
            }),
        );
        t.files = vec![
            row("holiday.photo.jpg", 2048, 0, None),
            row("setup.exe", 10, 0, None),
            row(".hidden", 0, 0, None),
        ];
        let m = model(true, true, Some(&online(Some(t), vec![])), None);
        let c = card_of(&m);
        assert_eq!(c.files[0].stem, "holiday.photo");
        assert_eq!(c.files[0].ext, ".jpg");
        assert_eq!(c.files[0].size, "2.0 KB");
        assert!(c.files[1].dangerous && !c.files[0].dangerous);
        assert_eq!(
            (c.files[2].stem.as_str(), c.files[2].ext.as_str()),
            (".hidden", "")
        );
        assert!(c.summary.starts_with("3 files"), "{}", c.summary);
        assert_eq!(c.summary, "3 files, 2.0 KB");
        assert!(c.note.starts_with("1 file can run programs"), "{}", c.note);
        assert!(c.warn);
        let a = accessible(&m, Instant::now(), None);
        assert!(a.contains("setup.exe, 10 B, can run programs"), "{a}");
        assert!(a.contains("Accept button, available shortly"), "{a}");
    }

    #[test]
    fn verification_shows_the_code_and_reads_it_digit_by_digit() {
        assert_eq!(
            code_text("123456"),
            ("123 456".into(), "1 2 3 4 5 6".into())
        );
        let t = transfer(Direction::Send, Stage::Verification, Some(Identity::New));
        let c = model(true, true, Some(&online(Some(t), vec![])), None);
        let c = card_of(&c);
        // Claimed connection: no code to compare, so nothing that looks like one
        assert_eq!(c.code, None);
        assert_eq!(actions(c), [Action::Decline, Action::CodesMatch]);
    }

    #[test]
    fn final_states_are_distinct_and_never_claim_unconfirmed_files() {
        let text = |dir, stage: Stage, files: Vec<FileRow>| {
            let mut t = transfer(dir, stage, Some(Identity::New));
            t.files = files;
            let m = model(true, true, Some(&online(Some(t), vec![])), None);
            let c = card_of(&m).clone();
            (
                c.title,
                c.note,
                c.files.iter().map(|f| f.state).collect::<Vec<_>>(),
            )
        };
        let (t, n, _) = text(Direction::Send, Stage::Declined, vec![]);
        assert!(t.ends_with("declined") && n == "Nothing was sent.");
        let (t, _, _) = text(Direction::Receive, Stage::Declined, vec![]);
        assert_eq!(t, "Declined");
        let (t, _, _) = text(Direction::Send, Stage::Cancelled, vec![]);
        assert_eq!(t, "Cancelled");
        // A disconnect or timeout is a failure with its reason, not a decline
        for why in ["The connection closed", "Timed out waiting for a decision"] {
            let (t, n, _) = text(
                Direction::Send,
                Stage::Failed(Failure::Other(why.into())),
                vec![],
            );
            assert_eq!((t.as_str(), n.as_str()), ("Couldn't send", why));
        }
        // Done with a file not confirmed: never "Sent"
        let (t, n, states) = text(
            Direction::Send,
            Stage::Done,
            vec![row("a", 1, 1, Some(true)), row("b", 1, 0, None)],
        );
        assert_eq!(t, "Finished");
        assert!(n.contains("weren't confirmed"));
        assert_eq!(states, [FileState::Done, FileState::NotConfirmed]);
        let (t, _, states) = text(
            Direction::Receive,
            Stage::Done,
            vec![row("a", 1, 1, Some(true)), row("b", 1, 1, Some(true))],
        );
        assert!(t.starts_with("Received from"));
        assert_eq!(states, [FileState::Done, FileState::Done]);
        // Cancelled mid-way: per-file truth
        let (_, n, states) = text(
            Direction::Receive,
            Stage::Cancelled,
            vec![
                row("a", 1, 1, Some(true)),
                row("b", 5, 2, Some(false)),
                row("c", 1, 0, None),
            ],
        );
        assert_eq!(n, "1 of 3 files were saved before it stopped.");
        assert_eq!(
            states,
            [FileState::Done, FileState::NotDone, FileState::NotConfirmed]
        );
    }

    #[test]
    fn progress_counts_only_confirmed_bytes() {
        let files = [
            row("a", 100, 100, Some(true)),
            row("b", 100, 100, None), // in flight: all bytes passed through here
            row("c", 200, 50, Some(false)),
        ];
        // Sending: b's bytes are only queued, not confirmed by the receiver
        assert!((confirmed_progress(&files, Direction::Send) - 0.25).abs() < 1e-6);
        // Receiving: b's bytes arrived here
        assert!((confirmed_progress(&files, Direction::Receive) - 0.5).abs() < 1e-6);
        let empty = [row("a", 0, 0, Some(true)), row("b", 0, 0, None)];
        for d in [Direction::Send, Direction::Receive] {
            assert!((confirmed_progress(&empty, d) - 0.5).abs() < 1e-6);
        }
        let mut t = transfer(Direction::Send, Stage::Transferring, Some(Identity::New));
        t.files = files.to_vec();
        let m = model(true, true, Some(&online(Some(t), vec![])), None);
        assert!(
            card_of(&m).note.ends_with("1 of 3 files"),
            "{}",
            card_of(&m).note
        );
    }

    #[test]
    fn a_dismissed_final_card_stays_gone_but_a_new_transfer_shows() {
        let t = transfer(Direction::Send, Stage::Cancelled, None);
        let id = t.id.clone();
        let v = online(Some(t), vec![peer("A")]);
        assert!(matches!(
            model(true, true, Some(&v), None).page,
            Page::Transfer(_)
        ));
        // A repeated (stale) snapshot of the same finished transfer: not resurrected
        assert!(matches!(
            model(true, true, Some(&v), Some(&id)).page,
            Page::Peers(_)
        ));
        // A live one under a dismissed id is still shown (dismiss is for finished cards)
        let mut live = transfer(Direction::Send, Stage::Connecting, None);
        live.id = id.clone();
        let v = online(Some(live), vec![]);
        assert!(matches!(
            model(true, true, Some(&v), Some(&id)).page,
            Page::Transfer(_)
        ));
        // A new transfer has a new id
        let v = online(
            Some(transfer(Direction::Receive, Stage::Done, None)),
            vec![],
        );
        assert!(matches!(
            model(true, true, Some(&v), Some(&id)).page,
            Page::Transfer(_)
        ));
    }

    #[test]
    fn show_in_folder_only_after_receiving_saved_files() {
        let mut t = transfer(Direction::Receive, Stage::Failed(Failure::Revoked), None);
        assert!(
            !actions(card_of(&model(
                true,
                true,
                Some(&online(Some(t.clone()), vec![])),
                None
            )))
            .contains(&Action::ShowFolder)
        );
        t.saved = vec![PathBuf::from("x")];
        let a = actions(card_of(&model(
            true,
            true,
            Some(&online(Some(t), vec![])),
            None,
        )));
        assert_eq!(a, [Action::ShowFolder, Action::Dismiss]);
    }

    #[test]
    fn arming_needs_both_the_runtime_delay_and_time_on_screen() {
        let t = transfer(Direction::Receive, Stage::Approval, Some(Identity::New));
        let m = model(true, true, Some(&online(Some(t), vec![])), None);
        let mut c = card_of(&m).clone();
        let now = Instant::now();
        assert!(!c.armed(now, None), "never shown");
        assert!(!c.armed(now, Some(now)), "just shown");
        let later = now + crate::nearby::ARMING;
        assert!(c.armed(later, Some(now)));
        c.decide_after = Some(later + Duration::from_millis(50));
        assert!(!c.armed(later, Some(now)), "runtime not armed yet");
        assert_eq!(arm_delay(&c, later, now), Some(Duration::from_millis(50)));
        // Cancel never waits
        assert!(!Card::is_decision(Action::Cancel) && Card::is_decision(Action::Accept));
    }

    #[test]
    fn local_names_are_unique_and_never_empty() {
        assert_eq!(local_name("Pixel", "mobile", &[]), "Pixel");
        let taken = ["pixel".to_string(), "Pixel 2".to_string()];
        assert_eq!(local_name("Pixel", "mobile", &taken), "Pixel 3");
        assert_eq!(local_name("\u{200B} ", "mobile", &[]), "Phone");
        assert!(local_name(&"x".repeat(200), "web", &[]).chars().count() <= 56);
    }

    #[test]
    fn the_setting_needs_the_current_notice_and_never_touches_start_on_launch() {
        use crate::space::OecaSettings;
        let fresh = OecaSettings::default();
        assert!(!fresh.nearby && fresh.nearby_notice == 0);
        // Turning on without consent: stays off
        assert!(!toggled(fresh, false).nearby);
        // With consent: on, and the notice recorded
        let on = toggled(fresh, true);
        assert!(on.nearby && on.nearby_notice == notice_id());
        // Off needs no consent; on again does not ask again (same notice)
        let off = toggled(on, false);
        assert!(!off.nearby && off.nearby_notice == notice_id());
        assert!(toggled(off, false).nearby);
        // A changed notice asks again
        let old = OecaSettings {
            nearby_notice: notice_id() ^ 1,
            ..fresh
        };
        assert!(!toggled(old, false).nearby);
        for launch in [false, true] {
            let s = OecaSettings {
                start_on_launch: launch,
                ..fresh
            };
            assert_eq!(toggled(s, true).start_on_launch, launch);
            assert_eq!(toggled(toggled(s, true), false).start_on_launch, launch);
        }
        assert_ne!(notice_id(), 0);
        // A saved "on" without the current notice is off, and asks again
        let stale = OecaSettings {
            nearby: true,
            nearby_notice: notice_id() ^ 1,
            ..fresh
        };
        assert!(!enabled(&stale));
        assert!(!enabled(&OecaSettings {
            nearby: true,
            ..fresh
        }));
        assert!(!enabled(&toggled(stale, false)), "no notice, still off");
        assert!(enabled(&toggled(stale, true)));
        assert!(!enabled(&toggled(toggled(stale, true), false)));
    }

    #[test]
    fn a_long_extension_keeps_its_end() {
        let w = |s: &str| s.chars().count() as f32 * 7.0;
        assert_eq!(fit_tail(".pdf", 100.0, w), ".pdf");
        let long = format!(".{}exe", "x".repeat(200));
        let fitted = fit_tail(&long, 70.0, w);
        assert!(
            fitted.starts_with('\u{2026}') && fitted.ends_with("exe"),
            "{fitted}"
        );
        assert!(w(&fitted) <= 70.0, "{fitted}");
        assert_eq!(w(&fitted), 70.0, "as long as fits");
        // No room at all: the last char, never a spill past the line
        assert_eq!(fit_tail(&long, 0.0, w), "\u{2026}e");
    }

    #[test]
    fn the_file_dialog_names_the_destination() {
        let mut p = peer("Pixel\u{202E}gpj.exe");
        assert_eq!(picking_status(&p), "Choose files to send to Pixelgpj.exe");
        p.alias = String::new();
        assert_eq!(picking_status(&p), "Choose files to send to Unnamed device");
    }

    #[test]
    fn device_lines_and_dates() {
        assert_eq!(date(0), "1 Jan 1970");
        assert_eq!(date(1_795_000_000), "18 Nov 2026");
        let fp = crate::nearby::test_fingerprint(1);
        let d = |state| Device {
            name: "Phone".into(),
            fingerprint: fp.clone(),
            short: "AB12".into(),
            verified_at: 0,
            state,
        };
        let lines = device_lines(&[
            d(DeviceState::Trusted),
            d(DeviceState::Revoked { at: 5 }),
            d(DeviceState::RevocationPending),
        ]);
        assert_eq!(
            lines
                .iter()
                .map(|l| (l.state, l.action))
                .collect::<Vec<_>>(),
            [
                ("Trusted", Some("Forget")),
                ("Forgotten", None),
                ("Forgetting not saved", Some("Retry"))
            ]
        );
        assert_eq!(lines[0].detail, "Verified 1 Jan 1970 \u{00B7} AB12");
    }
}
