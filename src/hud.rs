//! Volume HUD: the level shown in the notch in place of the Windows flyout.
//!
//! Volume: the three volume keys are registered as hotkeys (`RegisterHotKey`,
//! no hook), so Windows neither changes the volume nor shows its flyout for
//! them; Oeca steps the default output's master volume itself through
//! `IAudioEndpointVolume` (the same 2% steps as Windows), on a thread-pool
//! work item: a step blocks ~10 ms, too long for the UI thread while a held
//! key repeats. The HUD shows at once at the actual level. While it shows,
//! an endpoint callback keeps it in sync with changes made elsewhere. Keys
//! another app already holds stay with that app; without an audio endpoint
//! the keys are handed back to Windows.
//!
//! Brightness is not Oeca's: on laptops like the one this was built on the
//! brightness keys never reach Windows as input (the firmware handles them),
//! so they cannot be claimed and Windows keeps its own brightness flyout.
//!
//! Motion: everything moves on the notch's frame clock: presence and the
//! notch's widening on the notch's own springs, the level bar on
//! `HUD_LEVEL_SPRING` (a small overshoot), the mute morph on `SPACE_SPRING`.
//! Every retarget continues from the current position and velocity.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    AUDIO_VOLUME_NOTIFICATION_DATA, IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Threading::{PTP_CALLBACK_INSTANCE, TrySubmitThreadpoolCallback};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, RegisterHotKey, SendInput, UnregisterHotKey, VIRTUAL_KEY, VK_VOLUME_DOWN,
    VK_VOLUME_MUTE, VK_VOLUME_UP,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{GUID, Result, implement};

use crate::config::{
    BASE_HUD_EXTRA_WIDTH, BASE_HUD_STRETCH_WIDTH, COLLAPSE_SPRING, HUD_DIGIT_SPRING,
    HUD_LEVEL_SPRING, HUD_STRETCH_SPRING, NotchDimensions, SPACE_SPRING, Spring,
};
use crate::timer::Presence;

/// What the renderer draws: the level bar (gliding), the level as a number
/// (the actual value) and its rolling digit wheels, the speaker's sound
/// waves and mute cross, and how present the HUD is (0..1).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HudView {
    pub level: f32,
    pub percent: u8,
    /// The number's wheels (hundreds, tens, ones), each at its own place:
    /// wheel `k` settles on `percent / 10^(2-k)` (a whole number; its digit
    /// is that mod 10), rolling through the values between. And how fast
    /// each turns (wheel values / s): the motion blur.
    pub digits: [f32; 3],
    pub digit_speed: [f32; 3],
    /// Sound waves shown, 0..3 (wave `n` is drawn at `waves - (n - 1)`).
    pub waves: f32,
    /// What the HUD shows: 0 the volume, 1 the power-saving notice (in
    /// between, one crossfading into the other), and the notice's state
    /// (0 Energy Saver: Off, green; 1 On, yellow; gliding on a toggle).
    pub notice: f32,
    pub power_saving: f32,
    pub muted: f32,
    pub presence: f32,
    /// The notch's stretch: 0 at rest, toward 1 stretched to the right (its
    /// left edge anchored), toward -1 to the left (its right edge anchored).
    pub stretch: f32,
}

/// A value following its target on a spring, from where it is and with its
/// velocity; settles exactly on the target.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Glide {
    pub(crate) value: f32,
    pub(crate) vel: f32,
}

impl Glide {
    pub(crate) fn snap(&mut self, to: f32) {
        self.value = to;
        self.vel = 0.0;
    }

    pub(crate) fn moving(&self, target: f32) -> bool {
        self.value != target || self.vel != 0.0
    }

    pub(crate) fn step(&mut self, target: f32, dt_ms: f32, spring: Spring) {
        if !self.moving(target) {
            return;
        }
        (self.value, self.vel) = spring.advance(self.value, self.vel, target, dt_ms);
        if (self.value - target).abs() < 1e-3 && self.vel.abs() < 0.02 {
            self.snap(target);
        }
    }
}

/// The HUD's state and motion (stepped by the notch's frames).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Hud {
    presence: Presence,
    /// The collapsed notch's widening (0..1 of `BASE_HUD_EXTRA_WIDTH`).
    widen: Presence,
    shown: bool,
    muted: bool,
    /// The actual level (0..1); the bar glides to it.
    target: f32,
    level: Glide,
    mute: Glide,
    /// The number's wheels (see `HudView::digits`), each on its own spring.
    digits: [Glide; 3],
    /// Sound-wave bars for the level (0..3, with hysteresis), and as shown.
    bars: u8,
    waves: Glide,
    /// Showing the power-saving notice (else the volume), and its state;
    /// both crossfade.
    notice_on: bool,
    notice: Glide,
    power_saving: bool,
    power: Glide,
    /// The stretch toward an edge (-1 left .. 1 right, 0 at rest) and its
    /// target, and the side last stretched toward.
    stretch: Glide,
    stretch_target: f32,
    stretch_side: f32,
}

impl Hud {
    fn mute_target(&self) -> f32 {
        f32::from(u8::from(self.muted))
    }

    /// Muted: the waves give way to the cross (a muted device is not the
    /// same as 0%, which shows the speaker alone).
    fn waves_target(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            f32::from(self.bars)
        }
    }

    fn percent(&self) -> u32 {
        (self.target * 100.0).round() as u32
    }

    /// Shows the actual `level` (0..1). A HUD appearing from nothing starts
    /// there; one on screen (even fading out) keeps its bar and width where
    /// they are and glides on.
    pub fn show(&mut self, level: f32, muted: bool, instant: bool) {
        let fresh = !self.shown && self.presence.level() <= 0.0;
        self.notice_on = false;
        if fresh || instant {
            self.notice.snap(0.0);
        }
        self.shown = true;
        self.presence.set(true, instant);
        self.widen.set(true, instant);
        self.set_level(level, muted, instant || fresh);
    }

    /// Shows the power-saving notice: Windows' power saving turned `on` (or
    /// off). Like a volume change: from nothing it appears; over the volume
    /// it crossfades in; a toggle while it shows glides between On and Off
    /// (never stacked, never restarted). `instant` (no motion: this very
    /// power saving, or Reduced motion) shows it at once.
    pub fn show_power(&mut self, on: bool, instant: bool) {
        let fresh = !self.shown && self.presence.level() <= 0.0;
        self.notice_on = true;
        self.power_saving = on;
        self.shown = true;
        self.presence.set(true, instant);
        self.widen.set(true, instant);
        if fresh || instant {
            self.notice.snap(1.0);
            self.power.snap(self.power_target());
        }
    }

    fn power_target(&self) -> f32 {
        f32::from(u8::from(self.power_saving))
    }

    fn notice_target(&self) -> f32 {
        f32::from(u8::from(self.notice_on))
    }

    /// A level changed elsewhere while showing (no effect once hidden).
    pub fn update(&mut self, level: f32, muted: bool, instant: bool) {
        if self.shown {
            self.set_level(level, muted, instant);
        }
    }

    fn set_level(&mut self, level: f32, muted: bool, snap: bool) {
        self.target = level.clamp(0.0, 1.0);
        self.muted = muted;
        let percent = self.percent() as u8;
        self.bars = if snap {
            wave_bars_for(percent)
        } else {
            wave_bars(percent, self.bars)
        };
        if snap {
            self.snap_level();
        }
    }

    /// The bar, number, waves and mute cross at the actual level at once.
    fn snap_level(&mut self) {
        self.level.snap(self.target);
        self.mute.snap(self.mute_target());
        self.waves.snap(self.waves_target());
        let targets = digit_targets(self.percent());
        for (wheel, target) in self.digits.iter_mut().zip(targets) {
            wheel.snap(target);
        }
    }

    /// Every motion at its end, at once: Battery Saver (or Reduced motion)
    /// switching on mid-animation. No jump later, no frames needed.
    pub fn settle(&mut self) {
        self.presence.set(self.shown, true);
        self.widen.set(self.shown, true);
        self.snap_level();
        self.notice.snap(self.notice_target());
        self.power.snap(self.power_target());
        self.stretch.snap(self.stretch_target);
    }

    pub fn hide(&mut self, instant: bool) {
        self.shown = false;
        self.presence.set(false, instant);
        self.widen.set(false, instant);
    }

    /// Showing (or about to), as opposed to gone or fading out.
    pub fn is_shown(&self) -> bool {
        self.shown
    }

    pub fn is_animating(&self) -> bool {
        let targets = digit_targets(self.percent());
        self.presence.is_animating()
            || self.widen.is_animating()
            || self.level.moving(self.target)
            || self.mute.moving(self.mute_target())
            || self.waves.moving(self.waves_target())
            || (0..3).any(|k| self.digits[k].moving(targets[k]))
            || self.notice.moving(self.notice_target())
            || self.power.moving(self.power_target())
            || self.stretch.moving(self.stretch_target)
    }

    /// Advances by `dt_ms` (any frame length: the springs are exact); true
    /// while anything still moves.
    pub fn step(&mut self, dt_ms: f32) -> bool {
        self.presence.step_on(dt_ms, COLLAPSE_SPRING);
        self.widen.step_on(dt_ms, COLLAPSE_SPRING);
        self.level.step(self.target, dt_ms, HUD_LEVEL_SPRING);
        self.mute.step(self.mute_target(), dt_ms, SPACE_SPRING);
        self.waves.step(self.waves_target(), dt_ms, SPACE_SPRING);
        self.notice.step(self.notice_target(), dt_ms, SPACE_SPRING);
        self.power.step(self.power_target(), dt_ms, SPACE_SPRING);
        let targets = digit_targets(self.percent());
        for (wheel, target) in self.digits.iter_mut().zip(targets) {
            wheel.step(target, dt_ms, HUD_DIGIT_SPRING);
        }
        self.stretch
            .step(self.stretch_target, dt_ms, HUD_STRETCH_SPRING);
        self.is_animating()
    }

    /// The notch stretches toward `edge` (held there for as long as it is
    /// asked to), or relaxes back to its exact shape (None). Retargets keep
    /// the spring's position and velocity; `instant` (Reduced motion) snaps.
    pub fn stretch(&mut self, edge: Option<Edge>, instant: bool) {
        self.stretch_target = match edge {
            Some(Edge::Left) => -1.0,
            Some(Edge::Right) => 1.0,
            None => 0.0,
        };
        if self.stretch_target != 0.0 {
            self.stretch_side = self.stretch_target;
        }
        if instant {
            self.stretch.snap(self.stretch_target);
        }
    }

    /// Stretched (or stretching) toward an edge.
    pub fn is_stretched(&self) -> bool {
        self.stretch_target != 0.0
    }

    /// The stretch as drawn: signed like `HudView::stretch`. Relaxing, the
    /// spring's sub-pixel rebound past rest is not drawn (it would pull the
    /// anchored edge the other way).
    fn stretch_shown(&self) -> f32 {
        let v = self.stretch.value;
        if self.stretch_target == 0.0 {
            (v * self.stretch_side).max(0.0) * self.stretch_side
        } else {
            v
        }
    }

    /// How much wider (DIP) the collapsed notch is now.
    pub fn extra_width(&self) -> f32 {
        self.widen.level().max(0.0) * BASE_HUD_EXTRA_WIDTH
    }

    pub fn view(&self) -> HudView {
        HudView {
            // The spring may pass the level; what is shown stays in range
            level: self.level.value.clamp(0.0, 1.0),
            percent: self.percent() as u8,
            digits: self.digits.map(|w| w.value),
            digit_speed: self.digits.map(|w| w.vel.abs()),
            waves: self.waves.value.clamp(0.0, 3.0),
            notice: self.notice.value.clamp(0.0, 1.0),
            power_saving: self.power.value.clamp(0.0, 1.0),
            muted: self.mute.value.clamp(0.0, 1.0),
            presence: self.presence.level().clamp(0.0, 1.0),
            stretch: self.stretch_shown(),
        }
    }
}

/// The power-saving notice's text for Windows' power saving `on` / off.
pub fn power_notice_text(on: bool) -> &'static str {
    if on {
        "Energy Saver: On"
    } else {
        "Energy Saver: Off"
    }
}

/// The battery's fill on the power-saving notice: yellow while power
/// saving is on, green once it is off (Apple's system yellow / green).
pub fn power_notice_fill(on: bool) -> [f32; 3] {
    if on {
        [1.0, 214.0 / 255.0, 10.0 / 255.0]
    } else {
        [48.0 / 255.0, 209.0 / 255.0, 88.0 / 255.0]
    }
}

/// Whether a power-saving report is a real change to show: Windows' first
/// report for each saver (sent on registering, i.e. at startup) only states
/// the current mode; Battery and Energy Saver reporting the same effective
/// state is no change.
pub fn power_notice_due(source_seen_before: bool, was_on: bool, now_on: bool) -> bool {
    source_seen_before && was_on != now_on
}

/// The number's wheel targets for `percent` (0..=100): hundreds, tens and
/// ones as running counts, so 19 -> 20 turns the ones wheel 19 -> 20 (its
/// digit rolls 9 -> 0) while the tens wheel turns 1 -> 2 on its own spring.
pub fn digit_targets(percent: u32) -> [f32; 3] {
    let p = percent.min(100);
    [(p / 100) as f32, (p / 10) as f32, p as f32]
}

/// Sound-wave bars for a level shown fresh: none at 0%, then one, two or
/// three across thirds of the range.
pub fn wave_bars_for(percent: u8) -> u8 {
    match percent {
        0 => 0,
        1..=33 => 1,
        34..=66 => 2,
        _ => 3,
    }
}

/// Sound-wave bars for `percent` given the bars shown now: a change needs
/// the level clearly past a boundary (3 points of hysteresis either side of
/// 33% / 66%), so a level hovering at one never flickers. 0% is exact.
pub fn wave_bars(percent: u8, current: u8) -> u8 {
    if percent == 0 {
        return 0;
    }
    // To gain bar n (from below), and to keep it (from above)
    const UP: [u8; 4] = [0, 1, 37, 70];
    const KEEP: [u8; 4] = [0, 1, 31, 64];
    let mut n = current.clamp(1, 3);
    while n < 3 && percent >= UP[usize::from(n) + 1] {
        n += 1;
    }
    while n > 1 && percent < KEEP[usize::from(n)] {
        n -= 1;
    }
    n
}

/// Which edge of the notch a held key's momentum presses against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
}

/// Presses of one held volume key before the level reaches 0% / 100% for
/// that arrival to stretch the notch: ~5 repeats (~165 ms at Windows'
/// default 30 repeats/s, 10% of travel) is clearly a held key, not a tap.
pub const MIN_RAPID_RUN: u32 = 5;
/// The arrival must come this soon after the last press. Measured: the
/// audio service can take ~200 ms to apply the first step after a quiet
/// spell, so arrivals are judged on key timing with this much slack.
pub const ARRIVAL_WINDOW_MS: u32 = 400;

/// The longest gap between presses that still counts as one held key:
/// twice the keyboard's repeat interval (`SPI_GETKEYBOARDSPEED` 0..31 maps
/// to ~2.5..30 repeats/s), so repeat jitter never breaks a run, within
/// 70..200 ms. At the default rate (33 ms) that is 70 ms: deliberate taps,
/// rarely faster than ~8 a second (125 ms), never qualify.
pub fn rapid_gap_ms(keyboard_speed: u32) -> u32 {
    let per_second = 2.5 + 27.5 * keyboard_speed.min(31) as f32 / 31.0;
    ((2000.0 / per_second) as u32).clamp(70, 200)
}

fn at_end(dir: i8, level: f32) -> bool {
    if dir > 0 {
        level >= 0.995
    } else {
        level <= 0.005
    }
}

/// Volume-key momentum, from the keys' own input times (never from audio
/// callbacks): the current same-direction run of rapid presses that still
/// moved the level, and whether it is pressing against its end (latched
/// when it arrives there, held while the key keeps repeating).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Momentum {
    /// +1 up, -1 down, 0 none.
    dir: i8,
    run: u32,
    last_ms: Option<u32>,
    pressing: bool,
}

impl Momentum {
    /// A volume key pressed at `at_ms` (the input's message time) with the
    /// level at `level` before its step. A gap longer than `rapid_gap_ms`,
    /// a reversal or mute starts over; a press at the end already moves
    /// nothing and does not count.
    pub fn press(&mut self, key: VolumeKey, at_ms: u32, level: f32, rapid_gap_ms: u32) {
        let dir = match key {
            VolumeKey::Up => 1,
            VolumeKey::Down => -1,
            VolumeKey::Mute => {
                *self = Self::default();
                return;
            }
        };
        let rapid = self
            .last_ms
            .is_some_and(|t| at_ms.wrapping_sub(t) <= rapid_gap_ms);
        if !rapid || dir != self.dir {
            *self = Self {
                dir,
                ..Self::default()
            };
        }
        self.last_ms = Some(at_ms);
        if !at_end(dir, level) {
            self.run += 1;
        }
    }

    /// The level is now `level` at `now_ms`: the edge the held run presses
    /// against, if any. It starts pressing when a sustained rapid run's
    /// level arrives at its end (soon after its last press: never on a
    /// change from elsewhere), keeps pressing while held there (later
    /// presses at the end neither count nor re-trigger), and stops once
    /// the level leaves the end.
    pub fn pressing(&mut self, level: f32, now_ms: u32) -> Option<Edge> {
        if self.dir == 0 {
            return None;
        }
        if !at_end(self.dir, level) {
            self.pressing = false;
            return None;
        }
        if !self.pressing {
            let recent = self
                .last_ms
                .is_some_and(|t| now_ms.wrapping_sub(t) <= ARRIVAL_WINDOW_MS);
            if self.run < MIN_RAPID_RUN || !recent {
                return None;
            }
            self.pressing = true;
        }
        self.edge()
    }

    /// The edge pressed against right now (None unless `pressing`).
    pub fn edge(&self) -> Option<Edge> {
        match (self.pressing, self.dir) {
            (true, 1) => Some(Edge::Right),
            (true, -1) => Some(Edge::Left),
            _ => None,
        }
    }

    /// The key was released (no repeat within the rapid gap): the run and
    /// any press against the end are over.
    pub fn release(&mut self) {
        *self = Self::default();
    }
}

/// One frame of the endpoint stretch, for the renderer and hit testing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stretch {
    /// The deformed silhouette, as notch dimensions: the surface wider by
    /// whole pixels, the body wider (fractional, via its margins).
    pub shape: NotchDimensions,
    /// Where `shape` is drawn (px, window coordinates).
    pub shape_dx: f32,
    /// Where the content is drawn from (px, window coordinates): exactly
    /// where it is unstretched on screen (`left` in when the window starts
    /// further left).
    pub content: (f32, f32),
    /// How far (px) the window starts left of the notch's own left edge.
    pub left: i32,
}

/// The notch `dims` stretched by `s` (`HudView::stretch`): the body
/// `|s| * BASE_HUD_STRETCH_WIDTH` wider on the stretched side, the other
/// edge, the height and the content exactly where they were, with its own
/// corners and shoulders: one continuous outline, not a moved or added
/// piece. The surface grows by whole pixels on that side (the window starts
/// further left for a left stretch); the body's exact (fractional) extent is
/// set through the shape's margins. `s == 0` is exactly `dims`.
pub fn stretch_geometry(dims: &NotchDimensions, s: f32) -> Stretch {
    if s == 0.0 {
        return Stretch {
            shape: *dims,
            shape_dx: 0.0,
            content: (0.0, 0.0),
            left: 0,
        };
    }
    let a = s.abs();
    let grow = a * BASE_HUD_STRETCH_WIDTH * dims.scale;
    let ext = grow.ceil() as i32;
    let slack = (ext as f32 - grow) / 2.0;
    let mut shape = *dims;
    shape.width += ext;
    shape.shadow_margin_x += slack;
    let (shape_dx, left) = if s > 0.0 { (-slack, 0) } else { (slack, ext) };
    Stretch {
        shape,
        shape_dx,
        content: (left as f32, 0.0),
        left,
    }
}

/// The collapsed notch's width (px) with the HUD's `extra` DIP: grown in
/// 2 px steps (1 px per side keeps it centred), exactly `base` at 0.
pub fn widened_width(base: i32, extra: f32, scale: f32) -> i32 {
    base + 2 * (extra.max(0.0) * scale / 2.0).round() as i32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeKey {
    Up,
    Down,
    Mute,
}

/// The volume keys, registered under their own key codes as hotkey ids.
const VOLUME_KEYS: [VIRTUAL_KEY; 3] = [VK_VOLUME_UP, VK_VOLUME_DOWN, VK_VOLUME_MUTE];

impl VolumeKey {
    /// From a `WM_HOTKEY` id (the key code it was registered under).
    pub fn from_hotkey(id: usize) -> Option<Self> {
        match id {
            x if x == usize::from(VK_VOLUME_UP.0) => Some(Self::Up),
            x if x == usize::from(VK_VOLUME_DOWN.0) => Some(Self::Down),
            x if x == usize::from(VK_VOLUME_MUTE.0) => Some(Self::Mute),
            _ => None,
        }
    }

    fn vk(self) -> VIRTUAL_KEY {
        match self {
            Self::Up => VK_VOLUME_UP,
            Self::Down => VK_VOLUME_DOWN,
            Self::Mute => VK_VOLUME_MUTE,
        }
    }
}

/// Takes the volume keys (unmodified only: with Shift, Ctrl... they stay
/// Windows'). A key another app holds is left to it.
pub fn take_volume_keys(hwnd: HWND) {
    for vk in VOLUME_KEYS {
        unsafe {
            let _ = RegisterHotKey(
                Some(hwnd),
                i32::from(vk.0),
                HOT_KEY_MODIFIERS(0),
                u32::from(vk.0),
            );
        }
    }
}

/// Hands the volume keys back to Windows (no-op for keys not taken).
pub fn release_volume_keys(hwnd: HWND) {
    for vk in VOLUME_KEYS {
        unsafe {
            let _ = UnregisterHotKey(Some(hwnd), i32::from(vk.0));
        }
    }
}

/// Gives Windows a press Oeca could not handle (after `release_volume_keys`,
/// so it reaches Windows' own handling).
pub fn replay(key: VolumeKey) {
    let input = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key.vk(),
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let inputs = [input(KEYBD_EVENT_FLAGS(0)), input(KEYEVENTF_KEYUP)];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Marks Oeca's own volume changes: their notifications are skipped (the
/// level is read right after each press anyway).
const OECA_CHANGE: GUID = GUID::from_u128(0x5e2c_7a41_93d8_4f0b_8a6e_1c4b_d27f_0e93);

/// Presses waiting to be applied; a backlog past this is dropped (a held
/// key repeats far slower than presses are applied, so it never is).
const MAX_QUEUED_KEYS: usize = 32;

fn default_endpoint() -> Result<IAudioEndpointVolume> {
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        devices
            .GetDefaultAudioEndpoint(eRender, eConsole)?
            .Activate(CLSCTX_ALL, None)
    }
}

/// Applies a key like Windows does: up / down by one step (up also
/// unmutes), mute toggles. A step takes the audio service ~10 ms (it
/// notifies every client before returning): never on the UI thread.
fn press(endpoint: &IAudioEndpointVolume, key: VolumeKey) -> Result<()> {
    let ours: *const GUID = &OECA_CHANGE;
    unsafe {
        match key {
            VolumeKey::Up => {
                if endpoint.GetMute()?.as_bool() {
                    endpoint.SetMute(false, ours)?;
                }
                endpoint.VolumeStepUp(ours)
            }
            VolumeKey::Down => endpoint.VolumeStepDown(ours),
            VolumeKey::Mute => {
                let muted = endpoint.GetMute()?.as_bool();
                endpoint.SetMute(!muted, ours)
            }
        }
    }
}

/// What the notch, the change callback and the volume worker share.
struct Shared {
    hwnd: isize,
    msg: u32,
    /// A change message is queued (at most one at a time).
    posted: AtomicBool,
    /// Presses not applied yet, in order.
    keys: std::sync::Mutex<std::collections::VecDeque<VolumeKey>>,
    /// A worker is applying presses.
    busy: AtomicBool,
    /// The worker could not drive the audio endpoint: this press failed.
    failed: std::sync::Mutex<Option<VolumeKey>>,
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Shared {
    fn new(hwnd: HWND, msg: u32) -> Self {
        Self {
            hwnd: hwnd.0 as isize,
            msg,
            posted: AtomicBool::new(false),
            keys: Default::default(),
            busy: AtomicBool::new(false),
            failed: Default::default(),
        }
    }

    /// Tells the notch the level changed, unless it has not read the last
    /// such message yet.
    fn post(&self) {
        if !self.posted.swap(true, Ordering::AcqRel) {
            let hwnd = HWND(self.hwnd as *mut _);
            if unsafe { PostMessageW(Some(hwnd), self.msg, WPARAM(0), LPARAM(0)) }.is_err() {
                self.posted.store(false, Ordering::Release);
            }
        }
    }

    /// Queues a press; true if a worker must be started for it.
    fn enqueue(&self, key: VolumeKey) -> bool {
        {
            let mut keys = lock(&self.keys);
            if keys.len() < MAX_QUEUED_KEYS {
                keys.push_back(key);
            }
        }
        !self.busy.swap(true, Ordering::AcqRel)
    }

    /// Applies the queued presses in order, posting after each, until none
    /// is left (also one queued while it was finishing).
    fn run(&self, endpoint: Option<&IAudioEndpointVolume>) {
        loop {
            let applied = match endpoint {
                Some(e) => self.drain(e).map_err(Some),
                None => default_endpoint()
                    .map_err(|_| lock(&self.keys).pop_front())
                    .and_then(|e| self.drain(&e).map_err(Some)),
            };
            // The press that failed (if any was waiting) goes back to Windows
            if let Err(key) = applied {
                lock(&self.keys).clear();
                if key.is_some() {
                    *lock(&self.failed) = key;
                    self.post();
                }
            }
            self.busy.store(false, Ordering::Release);
            if lock(&self.keys).is_empty() || self.busy.swap(true, Ordering::AcqRel) {
                return;
            }
        }
    }

    fn drain(&self, endpoint: &IAudioEndpointVolume) -> std::result::Result<(), VolumeKey> {
        loop {
            let Some(key) = lock(&self.keys).pop_front() else {
                return Ok(());
            };
            press(endpoint, key).map_err(|_| key)?;
            self.post();
        }
    }
}

/// The volume worker: a thread-pool work item (no thread of Oeca's own),
/// alive only while presses are queued.
unsafe extern "system" fn volume_work(_: PTP_CALLBACK_INSTANCE, context: *mut core::ffi::c_void) {
    let shared = unsafe { Arc::from_raw(context as *const Shared) };
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    shared.run(None);
    if com {
        unsafe { CoUninitialize() };
    }
}

/// The default output's master volume, held only while the HUD shows (so a
/// new default device is picked up on the next press). Presses are applied
/// by the volume worker; the notch reads the level (~0.1 ms) when told it
/// changed, by the worker or, for changes elsewhere, the endpoint callback.
pub struct SystemVolume {
    endpoint: IAudioEndpointVolume,
    callback: IAudioEndpointVolumeCallback,
    shared: Arc<Shared>,
}

impl SystemVolume {
    pub fn open(hwnd: HWND, msg: u32) -> Result<Self> {
        let endpoint = default_endpoint()?;
        let shared = Arc::new(Shared::new(hwnd, msg));
        let callback = volume_callback(shared.clone());
        unsafe { endpoint.RegisterControlChangeNotify(&callback)? };
        Ok(Self {
            endpoint,
            callback,
            shared,
        })
    }

    /// Queues a key press; returns at once (the step is applied off the UI
    /// thread). Without a thread pool it is applied here.
    pub fn queue(&self, key: VolumeKey) {
        if !self.shared.enqueue(key) {
            return;
        }
        let context = Arc::into_raw(self.shared.clone()) as *mut core::ffi::c_void;
        let submitted =
            unsafe { TrySubmitThreadpoolCallback(Some(volume_work), Some(context), None) };
        if submitted.is_err() {
            let shared = unsafe { Arc::from_raw(context as *const Shared) };
            shared.run(Some(&self.endpoint));
        }
    }

    /// A press the worker could not apply (the keys should go back to
    /// Windows), once.
    pub fn take_failure(&self) -> Option<VolumeKey> {
        lock(&self.shared.failed).take()
    }

    /// Master volume (0..1) and mute. Also acknowledges a queued change
    /// message: a change after this read posts again.
    pub fn level(&self) -> Result<(f32, bool)> {
        self.shared.posted.store(false, Ordering::Release);
        unsafe {
            Ok((
                self.endpoint.GetMasterVolumeLevelScalar()?,
                self.endpoint.GetMute()?.as_bool(),
            ))
        }
    }
}

impl Drop for SystemVolume {
    fn drop(&mut self) {
        unsafe {
            let _ = self.endpoint.UnregisterControlChangeNotify(&self.callback);
        }
    }
}

fn volume_callback(shared: Arc<Shared>) -> IAudioEndpointVolumeCallback {
    VolumeChanged { shared }.into()
}

/// Called on an audio thread: only posts to the notch, which re-reads. A
/// burst of changes (another app's slider) queues one message, not one each.
#[implement(IAudioEndpointVolumeCallback)]
struct VolumeChanged {
    shared: Arc<Shared>,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeChanged_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        // Oeca's own steps: the worker posts for those
        if data.is_null() || unsafe { (*data).guidEventContext } != OECA_CHANGE {
            self.shared.post();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steps at a display's refresh period until settled (or `ms`).
    fn run_at(hud: &mut Hud, ms: f32, frame: f32) {
        let mut t = 0.0;
        while t < ms && hud.step(frame) {
            t += frame;
        }
    }

    fn run(hud: &mut Hud, ms: f32) {
        run_at(hud, ms, 1000.0 / 144.0);
    }

    fn shown(level: f32) -> Hud {
        let mut hud = Hud::default();
        hud.show(level, false, false);
        run(&mut hud, 3000.0);
        hud
    }

    #[test]
    fn test_hud_appears_at_its_level_then_glides() {
        let mut hud = Hud::default();
        hud.show(0.4, false, false);
        assert_eq!(hud.view().level, 0.4, "a new HUD starts at its level");
        run(&mut hud, 3000.0);
        assert_eq!(hud.view().presence, 1.0);
        hud.show(0.6, false, false);
        hud.step(7.0);
        let mid = hud.view().level;
        assert!(mid > 0.4 && mid < 0.6, "glides, no jump: {mid}");
        assert_eq!(
            hud.view().percent,
            60,
            "the number is the real level at once"
        );
    }

    #[test]
    fn test_level_overshoots_slightly_then_settles_exactly() {
        let mut hud = shown(0.3);
        hud.show(0.5, false, false);
        let mut peak: f32 = 0.0;
        while hud.step(1000.0 / 144.0) {
            peak = peak.max(hud.view().level);
        }
        let over = (peak - 0.5) / 0.2;
        assert!(
            over > 0.01 && over < 0.06,
            "small, visible overshoot: {over}"
        );
        assert_eq!(hud.view().level, 0.5, "settles exactly");
    }

    #[test]
    fn test_level_never_shows_out_of_range() {
        let mut hud = shown(0.9);
        hud.show(1.0, false, false);
        let mut t = 0.0;
        while t < 2000.0 && hud.step(4.0) {
            let l = hud.view().level;
            assert!((0.0..=1.0).contains(&l), "{l}");
            t += 4.0;
        }
        assert_eq!(hud.view().level, 1.0);
        hud.update(0.0, false, false);
        run(&mut hud, 2000.0);
        assert_eq!(hud.view().level, 0.0);
        hud.update(1.7, false, false);
        assert_eq!(hud.view().percent, 100, "an invalid level is clamped");
    }

    #[test]
    fn test_reversal_keeps_velocity() {
        let mut hud = shown(0.2);
        hud.show(0.8, false, false);
        run_at(&mut hud, 60.0, 7.0);
        let before = hud.view().level;
        let mut probe = hud;
        probe.step(7.0);
        let v = probe.view().level - before;
        assert!(v > 0.0, "moving up");
        // Reverse: it keeps moving up for a moment (decelerating), no restart
        hud.show(0.2, false, false);
        hud.step(7.0);
        let after = hud.view().level - before;
        assert!(after > 0.0 && after < v, "continuous: {after} vs {v}");
        run(&mut hud, 3000.0);
        assert_eq!(hud.view().level, 0.2);
    }

    #[test]
    fn test_held_key_stream_glides_continuously() {
        // A key held at ~30 repeats a second: 2% per repeat, frames at 144 Hz
        for frame in [1000.0 / 60.0, 1000.0 / 90.0, 1000.0 / 120.0, 1000.0 / 144.0] {
            let mut hud = shown(0.3);
            let mut t = 0.0;
            let mut next = 0.0;
            let mut level = 0.3;
            let mut last = hud.view().level;
            let mut max_step: f32 = 0.0;
            let mut peak: f32 = 0.0;
            while t < 1000.0 {
                let held = level < 0.59;
                if t >= next && held {
                    level += 0.02;
                    hud.show(level, false, false);
                    next += 33.0;
                }
                hud.step(frame);
                let l = hud.view().level;
                if held {
                    assert!(l >= last - 1e-4, "{frame}: never steps back while held");
                } else {
                    peak = peak.max(l);
                }
                max_step = max_step.max((l - last).abs());
                last = l;
                t += frame;
            }
            // Each frame moves a fraction of a key step: no visible jumps
            assert!(max_step < 0.02 * frame / 7.0, "{frame}: {max_step}");
            // Released: a small rebound past the last level, then it settles
            assert!(peak > level && peak - level < 0.01, "{frame}: {peak}");
            run(&mut hud, 3000.0);
            assert!((hud.view().level - level).abs() < 1e-6);
            assert!(!hud.is_animating(), "frames stop once released and settled");
        }
    }

    #[test]
    fn test_rapid_alternating_inputs_stay_continuous() {
        let mut hud = shown(0.5);
        let mut last = hud.view().level;
        for i in 0..40 {
            let to = if i % 2 == 0 { 0.52 } else { 0.5 };
            hud.show(to, false, false);
            for _ in 0..3 {
                hud.step(7.0);
                let l = hud.view().level;
                assert!((l - last).abs() < 0.01, "no jump");
                last = l;
            }
            assert_eq!(hud.view().presence, 1.0, "presence never restarts");
        }
        run(&mut hud, 3000.0);
        assert_eq!(hud.view().level, 0.5);
    }

    #[test]
    fn test_mute_morphs_and_unmute_returns() {
        let mut hud = shown(0.5);
        hud.show(0.5, true, false);
        hud.step(7.0);
        assert!(hud.view().muted > 0.0 && hud.view().muted < 1.0);
        run(&mut hud, 3000.0);
        assert_eq!(hud.view().muted, 1.0);
        hud.show(0.52, false, false);
        run(&mut hud, 3000.0);
        assert_eq!(hud.view().muted, 0.0);
    }

    #[test]
    fn test_hide_fades_out_restores_width_and_stops() {
        let mut hud = shown(0.5);
        assert_eq!(hud.extra_width(), BASE_HUD_EXTRA_WIDTH);
        hud.hide(false);
        assert!(!hud.is_shown());
        hud.step(7.0);
        assert!(hud.view().presence < 1.0 && hud.view().presence > 0.0);
        run(&mut hud, 3000.0);
        assert_eq!(hud.view().presence, 0.0);
        assert_eq!(hud.extra_width(), 0.0, "exactly the prior width");
        assert!(!hud.is_animating());
        hud.update(0.9, false, false);
        assert_eq!(hud.view().percent, 50, "updates after hiding are ignored");
    }

    #[test]
    fn test_level_changes_never_restart_the_widening() {
        let mut hud = Hud::default();
        hud.show(0.3, false, false);
        let mut last = 0.0;
        for i in 0..30 {
            hud.show(0.3 + 0.01 * i as f32, false, false);
            hud.step(7.0);
            let w = hud.extra_width();
            assert!(w >= last, "widening only grows while showing");
            last = w;
        }
    }

    #[test]
    fn test_widened_width_is_centred_and_restores_exactly() {
        for scale in [1.0f32, 1.25, 1.5, 2.0, 2.5, 3.0] {
            let base = (220.0 * scale).round() as i32;
            assert_eq!(widened_width(base, 0.0, scale), base);
            assert_eq!(widened_width(base, -1.0, scale), base);
            let full = widened_width(base, BASE_HUD_EXTRA_WIDTH, scale);
            assert_eq!((full - base) % 2, 0, "grows evenly on both sides");
            assert!((full - base) as f32 >= BASE_HUD_EXTRA_WIDTH * scale - 2.0);
        }
    }

    #[test]
    fn test_reduced_motion_snaps() {
        let mut hud = Hud::default();
        hud.show(0.3, false, true);
        assert_eq!(hud.view().presence, 1.0);
        assert_eq!(hud.extra_width(), BASE_HUD_EXTRA_WIDTH);
        hud.show(0.7, false, true);
        assert_eq!(hud.view().level, 0.7);
        assert!(!hud.is_animating());
        hud.hide(true);
        assert_eq!(hud.view().presence, 0.0);
        assert_eq!(hud.extra_width(), 0.0);
        assert!(!hud.is_animating());
    }

    #[test]
    fn test_volume_hotkey_ids() {
        assert_eq!(VolumeKey::from_hotkey(0xAF), Some(VolumeKey::Up));
        assert_eq!(VolumeKey::from_hotkey(0xAE), Some(VolumeKey::Down));
        assert_eq!(VolumeKey::from_hotkey(0xAD), Some(VolumeKey::Mute));
        assert_eq!(VolumeKey::from_hotkey(1), None);
    }

    #[test]
    fn test_volume_changes_queue_one_message_and_skip_our_own() {
        use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW};
        const MSG_ID: u32 = 0x8000 + 0x3F0;
        // A thread message (no window): counted straight off this queue
        let count = || {
            let mut n = 0;
            let mut m = MSG::default();
            while unsafe { PeekMessageW(&mut m, None, MSG_ID, MSG_ID, PM_REMOVE) }.as_bool() {
                n += 1;
            }
            n
        };
        let shared = Arc::new(Shared::new(HWND::default(), MSG_ID));
        let callback = volume_callback(shared.clone());
        let mut data = AUDIO_VOLUME_NOTIFICATION_DATA::default();
        for _ in 0..20 {
            unsafe { callback.OnNotify(&mut data).unwrap() };
        }
        assert_eq!(count(), 1, "a burst queues one message");
        shared.posted.store(false, Ordering::Release); // the notch read the level
        unsafe { callback.OnNotify(&mut data).unwrap() };
        // The worker's own posts share the one slot
        shared.post();
        assert_eq!(count(), 1, "a later change posts again, once");
        shared.posted.store(false, Ordering::Release);
        data.guidEventContext = OECA_CHANGE;
        unsafe { callback.OnNotify(&mut data).unwrap() };
        assert_eq!(count(), 0, "Oeca's own steps are not echoed");
    }

    #[test]
    fn test_held_key_presses_queue_in_order_and_bounded() {
        let shared = Shared::new(HWND::default(), 0);
        assert!(
            shared.enqueue(VolumeKey::Up),
            "the first press starts a worker"
        );
        for _ in 0..100 {
            assert!(!shared.enqueue(VolumeKey::Down), "one worker at a time");
        }
        let keys = lock(&shared.keys);
        assert_eq!(
            keys.len(),
            MAX_QUEUED_KEYS,
            "a backlog never grows unbounded"
        );
        assert_eq!(keys.front(), Some(&VolumeKey::Up), "in order");
    }

    /// Presses `n` keys `gap` ms apart from `level`, stepping 2% each like
    /// Windows; after each applied step (the audio service applies it a
    /// little later), the edge the run presses against, if any.
    fn presses(
        m: &mut Momentum,
        key: VolumeKey,
        n: u32,
        gap: u32,
        level: &mut f32,
        t: &mut u32,
    ) -> Vec<Option<Edge>> {
        let gap_limit = rapid_gap_ms(31);
        let mut states = Vec::new();
        for _ in 0..n {
            *t += gap;
            m.press(key, *t, *level, gap_limit);
            let step = if key == VolumeKey::Up { 0.02 } else { -0.02 };
            *level = (*level + step).clamp(0.0, 1.0);
            states.push(m.pressing(*level, *t + 15));
        }
        states
    }

    fn never(states: &[Option<Edge>]) -> bool {
        states.iter().all(Option::is_none)
    }

    #[test]
    fn test_rapid_gap_follows_the_keyboard_repeat_rate() {
        assert_eq!(
            rapid_gap_ms(31),
            70,
            "default 30/s: 33 ms repeats, taps never"
        );
        assert!(rapid_gap_ms(20) > 70 && rapid_gap_ms(20) < 120);
        assert_eq!(rapid_gap_ms(0), 200, "slowest repeat: capped");
        assert_eq!(rapid_gap_ms(99), 70);
    }

    #[test]
    fn test_held_into_the_end_presses_it_for_as_long_as_held() {
        for (key, start, edge) in [
            (VolumeKey::Up, 0.8, Edge::Right),
            (VolumeKey::Down, 0.2, Edge::Left),
        ] {
            let (mut m, mut level, mut t) = (Momentum::default(), start, 1000);
            // Held: 33 ms repeats, through the end and on (20 more at it)
            let states = presses(&mut m, key, 30, 33, &mut level, &mut t);
            let first = states.iter().position(Option::is_some).expect("presses");
            assert_eq!(first, 9, "{key:?}: from the press that reaches the end");
            assert!(
                states[first..].iter().all(|s| *s == Some(edge)),
                "{key:?}: held at the end, it keeps pressing (no re-trigger)"
            );
            // Released: over; held again at the end: never (no arrival)
            m.release();
            assert_eq!(m.edge(), None);
            t += 600;
            assert!(never(&presses(&mut m, key, 20, 33, &mut level, &mut t)));
        }
    }

    #[test]
    fn test_slow_taps_and_short_runs_never_press() {
        // Slow taps all the way to 100% (and on)
        let (mut m, mut level, mut t) = (Momentum::default(), 0.8, 0);
        assert!(never(&presses(
            &mut m,
            VolumeKey::Up,
            15,
            180,
            &mut level,
            &mut t
        )));
        // Fast but intermittent: bursts of 3 with gaps
        let (mut m, mut level, mut t) = (Momentum::default(), 0.8, 0);
        for _ in 0..5 {
            assert!(never(&presses(
                &mut m,
                VolumeKey::Up,
                3,
                33,
                &mut level,
                &mut t
            )));
            t += 300;
        }
        // A single press (or a short run) that happens to reach the end
        let (mut m, mut level, mut t) = (Momentum::default(), 0.98, 0);
        assert!(never(&presses(
            &mut m,
            VolumeKey::Up,
            1,
            33,
            &mut level,
            &mut t
        )));
        let (mut m, mut level, mut t) = (Momentum::default(), 0.94, 0);
        assert!(never(&presses(
            &mut m,
            VolumeKey::Up,
            4,
            33,
            &mut level,
            &mut t
        )));
    }

    #[test]
    fn test_reversal_mute_leaving_the_end_and_stale_arrivals_clear_it() {
        let held = |key| {
            let (mut m, mut level, mut t) = (Momentum::default(), 0.7, 0);
            presses(&mut m, key, 20, 33, &mut level, &mut t);
            (m, level, t)
        };
        // Reversed at the end: the press itself ends it
        let (mut m, level, t) = held(VolumeKey::Up);
        assert_eq!(m.edge(), Some(Edge::Right));
        m.press(VolumeKey::Down, t + 33, level, 70);
        assert_eq!(m.edge(), None);
        assert_eq!(m.pressing(0.98, t + 40), None);
        // Mute ends it
        let (mut m, level, t) = held(VolumeKey::Up);
        m.press(VolumeKey::Mute, t + 33, level, 70);
        assert_eq!(m.edge(), None);
        // The level leaves the end (changed elsewhere): it lets go
        let (mut m, _, t) = held(VolumeKey::Up);
        assert_eq!(m.pressing(0.9, t + 20), None);
        assert_eq!(m.edge(), None);
        // A run reported at the end long after its last key (a change made
        // elsewhere) never starts pressing
        let (mut m, mut level, mut t) = (Momentum::default(), 0.7, 0);
        presses(&mut m, VolumeKey::Up, 8, 33, &mut level, &mut t);
        assert_eq!(m.pressing(1.0, t + ARRIVAL_WINDOW_MS + 1), None);
        // No keys at all: audio callbacks alone never press
        assert_eq!(Momentum::default().pressing(1.0, 0), None);
        assert_eq!(Momentum::default().pressing(0.0, 0), None);
    }

    #[test]
    fn test_stretch_holds_while_held_and_relaxes_exactly_on_release() {
        for (edge, side) in [(Edge::Right, 1.0f32), (Edge::Left, -1.0)] {
            for frame in [1000.0 / 60.0, 1000.0 / 90.0, 1000.0 / 120.0, 1000.0 / 144.0] {
                let mut hud = shown(1.0);
                hud.stretch(Some(edge), false);
                let mut peak = 0.0f32;
                let mut t = 0.0;
                while hud.step(frame) {
                    peak = peak.max(hud.view().stretch * side);
                    t += frame;
                }
                // Held: it stays fully stretched, and the frames stop
                assert_eq!(hud.view().stretch, side, "{frame}: held stretched");
                assert!(!hud.is_animating());
                assert!(peak < 1.03, "{frame}: restrained: {peak}");
                assert!(t < 900.0, "{frame}: settles into the stretch: {t}");
                // Released: back to exactly the original shape, never past
                // it toward the other side
                hud.stretch(None, false);
                let mut t = 0.0;
                while hud.step(frame) {
                    let s = hud.view().stretch * side;
                    assert!((0.0..=1.03).contains(&s), "{frame}: {s}");
                    t += frame;
                }
                assert_eq!(hud.view().stretch, 0.0, "exactly at rest");
                assert!(t < 900.0, "{frame}: relaxes promptly: {t}");
                assert_eq!(
                    hud.extra_width(),
                    BASE_HUD_EXTRA_WIDTH,
                    "widening untouched"
                );
            }
        }
    }

    #[test]
    fn test_stretch_retargets_continuously_and_survives_hide() {
        let mut hud = shown(1.0);
        hud.stretch(Some(Edge::Right), false);
        run_at(&mut hud, 80.0, 7.0);
        let mut last = hud.view().stretch;
        // Released mid-stretch, held again, released: never a jump
        for target in [None, Some(Edge::Right), None] {
            hud.stretch(target, false);
            for _ in 0..12 {
                hud.step(7.0);
                let s = hud.view().stretch;
                assert!((s - last).abs() < 0.12, "continuous: {last} -> {s}");
                last = s;
            }
        }
        // The HUD leaving does not cut the relaxation short
        hud.hide(false);
        hud.step(7.0);
        assert!((hud.view().stretch - last).abs() < 0.12);
        run(&mut hud, 3000.0);
        assert_eq!(hud.view().stretch, 0.0);
        assert_eq!(hud.extra_width(), 0.0);
        assert!(!hud.is_animating());
    }

    #[test]
    fn test_stretch_geometry_anchors_one_edge_and_compresses_at_all_dpis() {
        use crate::config::NotchState;
        for dpi in [96u32, 120, 144, 168, 192, 240, 288] {
            let mut dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
            // Also on the HUD-widened notch
            for extra in [0.0, BASE_HUD_EXTRA_WIDTH] {
                dims.width = widened_width(
                    NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi).width,
                    extra,
                    dims.scale,
                );
                assert_eq!(stretch_geometry(&dims, 0.0).shape, dims, "at rest: exact");
                // Body extent in screen px (window origin at 0 unstretched)
                let body = |st: &Stretch| {
                    let l = -st.left as f32 + st.shape_dx + st.shape.shadow_margin_x;
                    (l, l + st.shape.notch_width(), st.shape.notch_height())
                };
                let (l0, r0, h0) = body(&stretch_geometry(&dims, 0.0));
                // Content in screen px: where it is drawn from, less how far
                // the window starts left
                let content = |st: &Stretch| (st.content.0 - st.left as f32, st.content.1);
                for s in [0.25f32, 1.0, 1.02] {
                    let grow = s * BASE_HUD_STRETCH_WIDTH * dims.scale;
                    // Right: left edge fixed, right edge out, same height,
                    // content where it was
                    let st = stretch_geometry(&dims, s);
                    let (l, r, h) = body(&st);
                    assert!((l - l0).abs() < 1e-3, "{dpi} {s}: left anchored");
                    assert!((r - (r0 + grow)).abs() < 1e-3, "{dpi} {s}: right out");
                    assert_eq!(h, h0, "{dpi} {s}: never thinner");
                    assert_eq!(st.shape.height, dims.height);
                    assert_eq!(st.left, 0);
                    assert!(
                        st.shape.width >= dims.width + grow as i32,
                        "surface holds it"
                    );
                    assert_eq!(content(&st), (0.0, 0.0), "{dpi} {s}: content still");
                    // Left: mirrored, the right edge fixed
                    let st = stretch_geometry(&dims, -s);
                    let (l, r, h) = body(&st);
                    assert!((r - r0).abs() < 1e-3, "{dpi} {s}: right anchored");
                    assert!((l - (l0 - grow)).abs() < 1e-3, "{dpi} {s}: left out");
                    assert_eq!(h, h0, "{dpi} {s}: never thinner");
                    assert_eq!(st.left, grow.ceil() as i32);
                    assert_eq!(content(&st), (0.0, 0.0), "{dpi} {s}: content still");
                    // The corners keep their size (one outline, not squashed)
                    assert_eq!(st.shape.curvature, dims.curvature);
                    // Hit testing follows the deformed body
                    let mid = h / 2.0;
                    let x = |screen: f32| screen + st.left as f32 - st.shape_dx;
                    // (the side wall at mid height: inside the top shoulder)
                    let wall = l0 + dims.curvature.top_transition_radius;
                    let plain = stretch_geometry(&dims, 0.0).shape;
                    assert!(!plain.contains_point(wall - grow / 2.0, mid), "was outside");
                    assert!(
                        st.shape.contains_point(x(wall - grow / 2.0), mid),
                        "stretched part"
                    );
                    assert!(
                        !st.shape.contains_point(x(wall - grow - 1.0), mid),
                        "beyond it"
                    );
                    assert!(
                        st.shape.contains_point(x((l + r) / 2.0), h0 - 0.5),
                        "full height"
                    );
                }
            }
        }
    }

    #[test]
    fn test_reduced_motion_stretch_snaps() {
        let mut hud = shown(0.0);
        hud.stretch(Some(Edge::Left), true);
        assert_eq!(hud.view().stretch, -1.0);
        hud.stretch(None, true);
        assert_eq!(hud.view().stretch, 0.0);
        assert!(!hud.is_animating());
    }

    #[test]
    fn test_wave_bars_for_levels_zero_and_mute() {
        assert_eq!(wave_bars_for(0), 0, "0%: the speaker alone");
        assert_eq!(wave_bars_for(1), 1);
        assert_eq!(wave_bars_for(33), 1);
        assert_eq!(wave_bars_for(50), 2);
        assert_eq!(wave_bars_for(66), 2);
        assert_eq!(wave_bars_for(67), 3);
        assert_eq!(wave_bars_for(100), 3);
        // Shown: 0% unmuted is no waves and no cross; muted at 60% is the
        // cross and no waves (muted is not the same as silent)
        let silent = shown(0.0).view();
        assert_eq!((silent.waves, silent.muted), (0.0, 0.0));
        let mut hud = shown(0.6);
        assert_eq!(hud.view().waves, 2.0);
        hud.show(0.6, true, false);
        run(&mut hud, 3000.0);
        assert_eq!((hud.view().waves, hud.view().muted), (0.0, 1.0));
        hud.show(0.6, false, false);
        run(&mut hud, 3000.0);
        assert_eq!((hud.view().waves, hud.view().muted), (2.0, 0.0));
    }

    #[test]
    fn test_wave_bars_hold_steady_near_a_boundary() {
        // Hovering around 33% / 66% never flickers
        for (around, from) in [(33u8, 1u8), (34, 2), (66, 2), (67, 3)] {
            let mut n = from;
            for p in [around - 2, around + 2, around - 2, around + 2, around] {
                n = wave_bars(p, n);
                assert_eq!(n, from, "{p} from {from}");
            }
        }
        // ...but a clear move changes it, both ways
        assert_eq!(wave_bars(40, 1), 2);
        assert_eq!(wave_bars(28, 2), 1);
        assert_eq!(wave_bars(72, 2), 3);
        assert_eq!(wave_bars(60, 3), 2);
        assert_eq!(wave_bars(0, 3), 0, "0% is exact");
        assert_eq!(wave_bars(5, 0), 1);
        // A held key's stream across a boundary changes once
        let mut hud = shown(0.30);
        let mut changes = 0;
        let mut last = hud.view().waves;
        for p in (30..=40).chain((30..40).rev()).chain(30..=40) {
            hud.show(p as f32 / 100.0, false, false);
            run(&mut hud, 3000.0);
            if hud.view().waves != last {
                changes += 1;
                last = hud.view().waves;
            }
        }
        assert!(changes <= 3, "{changes}");
    }

    #[test]
    fn test_digit_wheels_turn_on_their_own_and_land_exactly() {
        // 19 -> 20: ones 19 -> 20, tens 1 -> 2, each its own spring
        let mut hud = shown(0.19);
        assert_eq!(hud.view().digits, [0.0, 1.0, 19.0]);
        hud.show(0.20, false, false);
        hud.step(7.0);
        let v = hud.view();
        assert!(v.digits[2] > 19.0 && v.digits[2] < 20.0, "ones rolling");
        assert!(v.digits[1] > 1.0 && v.digits[1] < 2.0, "tens rolling");
        assert_eq!(v.digits[0], 0.0, "hundreds still");
        run(&mut hud, 3000.0);
        assert_eq!(hud.view().digits, [0.0, 2.0, 20.0], "exactly 20");
        // A bigger jump: the ones wheel turns much faster than the tens one
        // (not in lockstep), and 99 -> 100 brings in the hundreds
        for (from, to) in [
            (9u32, 10u32),
            (99, 100),
            (100, 99),
            (10, 9),
            (0, 100),
            (73, 5),
        ] {
            let mut hud = shown(from as f32 / 100.0);
            hud.show(to as f32 / 100.0, false, false);
            hud.step(7.0);
            let v = hud.view();
            if from.abs_diff(to) > 9 {
                assert!(
                    v.digit_speed[2] > v.digit_speed[1],
                    "{from}->{to}: own speeds"
                );
            }
            run(&mut hud, 3000.0);
            let end = hud.view();
            assert_eq!(end.digits, digit_targets(to), "{from}->{to}");
            assert_eq!(end.percent as u32, to);
            assert_eq!(end.digit_speed, [0.0; 3], "at rest: no blur");
            assert!(!hud.is_animating());
        }
    }

    #[test]
    fn test_digit_retargets_keep_velocity_and_never_snap() {
        let mut hud = shown(0.40);
        hud.show(0.60, false, false);
        run_at(&mut hud, 60.0, 7.0);
        let before = hud.view();
        assert!(before.digit_speed[2] > 0.0);
        // Reversed mid-roll: continues from where it is, turning around
        hud.show(0.40, false, false);
        let mut last = before.digits[2];
        for _ in 0..200 {
            hud.step(7.0);
            let now = hud.view().digits[2];
            assert!((now - last).abs() < 1.5, "continuous: {last} -> {now}");
            last = now;
        }
        assert_eq!(hud.view().digits, digit_targets(40));
        // Rapid held-key steps: the wheels never jump, and land on the last
        let mut hud = shown(0.30);
        let mut last = hud.view().digits;
        for p in (32..=60).step_by(2) {
            hud.show(p as f32 / 100.0, false, false);
            for _ in 0..5 {
                hud.step(1000.0 / 144.0);
                let now = hud.view().digits;
                for k in 0..3 {
                    assert!(now[k] >= last[k] - 1e-4, "never back while rising");
                    assert!(now[k] - last[k] < 1.0, "never a jump");
                }
                last = now;
            }
        }
        run(&mut hud, 3000.0);
        assert_eq!(hud.view().digits, digit_targets(60));
    }

    #[test]
    fn test_reduced_motion_number_and_icon_at_once() {
        let mut hud = shown(0.30);
        hud.show(0.75, false, true);
        let v = hud.view();
        assert_eq!(v.digits, digit_targets(75));
        assert_eq!(v.digit_speed, [0.0; 3], "no roll, no blur");
        assert_eq!(v.waves, 3.0);
        assert!(!hud.is_animating());
    }

    #[test]
    fn test_settle_lands_everything_mid_animation() {
        // Battery Saver switching on mid-animation: everything at its end,
        // exactly, at once; nothing left to animate
        let mut hud = Hud::default();
        hud.show(0.42, false, false);
        hud.step(7.0);
        hud.show(0.88, true, false);
        hud.stretch(Some(Edge::Right), false);
        hud.step(7.0);
        assert!(hud.is_animating());
        hud.settle();
        let v = hud.view();
        assert!(!hud.is_animating());
        assert_eq!(v.presence, 1.0);
        assert_eq!(v.level, 0.88);
        assert_eq!(v.digits, digit_targets(88));
        assert_eq!((v.waves, v.muted), (0.0, 1.0));
        assert_eq!(v.stretch, 1.0);
        assert_eq!(hud.extra_width(), BASE_HUD_EXTRA_WIDTH);
        // Hidden mid-fade: gone at once
        hud.hide(false);
        hud.step(7.0);
        hud.settle();
        assert_eq!(hud.view().presence, 0.0);
        assert_eq!(hud.extra_width(), 0.0);
        assert!(!hud.is_animating() || hud.is_stretched());
    }

    #[test]
    fn test_power_notice_text_colours_and_due() {
        assert_eq!(power_notice_text(true), "Energy Saver: On");
        assert_eq!(power_notice_text(false), "Energy Saver: Off");
        let [r, g, b] = power_notice_fill(true);
        assert!(r > 0.9 && g > 0.75 && b < 0.1, "on: yellow");
        let [r, g, b] = power_notice_fill(false);
        assert!(g > 0.75 && r < 0.3 && b < 0.4, "off: green");
        // Startup report (source not seen): never a notice, whatever it says
        assert!(!power_notice_due(false, false, true));
        assert!(!power_notice_due(false, true, false));
        // A real change: shown; the other saver agreeing (no change): not
        assert!(power_notice_due(true, false, true));
        assert!(power_notice_due(true, true, false));
        assert!(!power_notice_due(true, true, true));
        assert!(!power_notice_due(true, false, false));
    }

    #[test]
    fn test_power_notice_appears_crossfades_and_restores() {
        // From nothing: the notice appears (motion), widening like the HUD
        let mut hud = Hud::default();
        hud.show_power(false, false);
        let v = hud.view();
        assert_eq!(
            (v.notice, v.power_saving),
            (1.0, 0.0),
            "content set at once"
        );
        assert!(v.presence < 1.0, "fading in");
        run(&mut hud, 3000.0);
        assert_eq!(hud.view().presence, 1.0);
        assert_eq!(hud.extra_width(), BASE_HUD_EXTRA_WIDTH);
        // Rapid toggles: one notice, gliding between Off and On, never
        // restarting or leaving
        let mut last = hud.view().power_saving;
        for on in [true, false, true, true, false, true] {
            hud.show_power(on, false);
            for _ in 0..6 {
                hud.step(7.0);
                let p = hud.view().power_saving;
                assert!((p - last).abs() < 0.2, "continuous: {last} -> {p}");
                assert_eq!(hud.view().presence, 1.0, "never restarts");
                last = p;
            }
        }
        run(&mut hud, 3000.0);
        assert_eq!(hud.view().power_saving, 1.0, "lands on the last state");
        // A volume key takes over: the notice crossfades to the volume
        hud.show(0.5, false, false);
        hud.step(7.0);
        let n = hud.view().notice;
        assert!(n > 0.0 && n < 1.0, "crossfading");
        run(&mut hud, 3000.0);
        assert_eq!(hud.view().notice, 0.0);
        // ...and the notice can come back over it
        hud.show_power(false, false);
        run(&mut hud, 3000.0);
        assert_eq!((hud.view().notice, hud.view().power_saving), (1.0, 0.0));
        // Timeout: fades out, exact previous width, frames stop
        hud.hide(false);
        run(&mut hud, 3000.0);
        assert_eq!(hud.view().presence, 0.0);
        assert_eq!(hud.extra_width(), 0.0);
        assert!(!hud.is_animating());
    }

    #[test]
    fn test_power_notice_without_motion_is_there_at_once() {
        // Power saving on means no motion: the notice still appears, at its
        // final state and width, with nothing to animate
        let mut hud = Hud::default();
        hud.show(0.3, false, false);
        hud.step(7.0);
        hud.show_power(true, true);
        let v = hud.view();
        assert_eq!((v.presence, v.notice, v.power_saving), (1.0, 1.0, 1.0));
        assert_eq!(hud.extra_width(), BASE_HUD_EXTRA_WIDTH);
        hud.settle();
        assert!(!hud.is_animating());
        hud.hide(true);
        assert_eq!((hud.view().presence, hud.extra_width()), (0.0, 0.0));
    }
}
