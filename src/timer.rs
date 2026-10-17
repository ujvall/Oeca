//! Countdown timer (Home): the countdown itself and how it shows on the
//! collapsed notch. Pure state: every call takes `now` (a monotonic
//! `Instant`), so the window drives it from its own events and tests drive it
//! with chosen instants.
//!
//! Timing: a running countdown is a deadline, not a counter. What is left is
//! always `deadline - now`, so late, dropped or coalesced wake-ups never make
//! it drift. Pausing stores what was left; resuming sets a new deadline from
//! it. Wake-ups carry no state of their own: `poll` only ever acts on the
//! current countdown's own deadline, so a wake-up armed for an earlier
//! (cancelled, restarted, paused) countdown can never complete or change a
//! newer one.

use std::time::{Duration, Instant};

use crate::config::TIMER_PULSES;
use crate::config::{COLLAPSE_SPRING, EXPAND_SPRING, TIMER_DONE_HOLD_MS, TIMER_PULSE_MS};

/// Focus or Break: two modes, each with its own duration (a stop on the
/// scale) and its own countdown, chosen with the selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Focus,
    Break,
}

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::Focus, Mode::Break];

    pub fn index(self) -> usize {
        match self {
            Self::Focus => 0,
            Self::Break => 1,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Focus => "Focus",
            Self::Break => "Break",
        }
    }

    /// A focus session and a short break (seconds).
    pub fn default_secs(self) -> u32 {
        match self {
            Self::Focus => 25 * 60,
            Self::Break => 5 * 60,
        }
    }
}

/// The scale's stops, a tick each: 30 and 45 seconds, then every whole
/// minute from 1 to 120.
pub const STOP_COUNT: usize = 122;

/// Seconds at stop `i` (clamped to the scale).
pub fn stop_secs(i: usize) -> u32 {
    let i = i.min(STOP_COUNT - 1) as u32;
    if i < 2 { 15 * (i + 2) } else { 60 * (i - 1) }
}

/// The stop of exactly `secs`, if it is one.
pub fn stop_of(secs: u32) -> Option<usize> {
    (0..STOP_COUNT).find(|&i| stop_secs(i) == secs)
}

/// Where `secs` sits on the scale, in stops (fractional between them; 0 s
/// is one stop before the first).
pub fn scale_position(secs: f32) -> f32 {
    if secs <= 30.0 {
        secs / 30.0 - 1.0
    } else if secs <= 60.0 {
        secs / 15.0 - 2.0
    } else {
        secs / 60.0 + 1.0
    }
}

/// What the scale shows at its centre (in stops): in setup the chosen
/// duration, running or paused what is left (fractional, so it slides as
/// the time runs), done nothing left.
pub fn scale_value(t: &Countdown, now: Instant) -> f32 {
    match t.phase() {
        Phase::Idle => t.stop() as f32,
        Phase::Running { .. } | Phase::Paused { .. } => {
            scale_position(t.remaining(now).as_secs_f32())
        }
        Phase::Done { .. } => scale_position(0.0),
    }
}

/// The timer panel's motion, on the notch's frames: the selector's
/// highlight (0 Focus .. 1 Break), in setup the scale's value (stops), and
/// running or paused the scale's lag behind what is left (stops, gliding to
/// 0, so a new time slides in rather than jumping), each gliding on
/// `TIMER_SPRING` from where it is with its velocity (a new target
/// mid-glide just retargets). Stepped only while something moves.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TimerMotion {
    mode: crate::hud::Glide,
    mode_target: f32,
    scale: crate::hud::Glide,
    scale_target: f32,
    lag: crate::hud::Glide,
}

impl TimerMotion {
    /// At rest on `t`'s mode and setup value.
    pub fn new(t: &Countdown) -> Self {
        let mut m = Self::default();
        m.aim(t, None, Instant::now(), true);
        m
    }

    /// Aims at `t`: its mode, and its duration on the scale. `from`: the
    /// scale's value shown just before (so arriving in setup, e.g. on
    /// Cancel, glides from where the countdown had got to; running or
    /// paused, the scale glides from it to what is left at `now`).
    /// `instant` (Reduced motion): at once.
    pub fn aim(&mut self, t: &Countdown, from: Option<f32>, now: Instant, instant: bool) {
        self.mode_target = f32::from(u8::from(t.mode() == Mode::Break));
        self.scale_target = t.stop() as f32;
        if let Some(from) = from
            && t.phase() == Phase::Idle
            && from != self.scale.value
        {
            // Continue from what was on show, with no leftover velocity
            self.scale.snap(from);
        }
        if t.phase() == Phase::Idle {
            self.lag.snap(0.0);
        } else if let Some(from) = from {
            // From what was on show, keeping the lag's velocity
            self.lag.value = from - scale_value(t, now);
        }
        if instant {
            self.lag.snap(0.0);
            self.mode.snap(self.mode_target);
            self.scale.snap(self.scale_target);
        }
    }

    pub fn is_animating(&self) -> bool {
        self.mode.moving(self.mode_target)
            || self.scale.moving(self.scale_target)
            || self.lag.moving(0.0)
    }

    /// Advances by `dt_ms`; true while anything still moves.
    pub fn step(&mut self, dt_ms: f32) -> bool {
        use crate::config::TIMER_SPRING;
        self.mode.step(self.mode_target, dt_ms, TIMER_SPRING);
        self.scale.step(self.scale_target, dt_ms, TIMER_SPRING);
        self.lag.step(0.0, dt_ms, TIMER_SPRING);
        self.is_animating()
    }

    /// The highlight, 0 (Focus) .. 1 (Break).
    pub fn mode(&self) -> f32 {
        self.mode.value.clamp(0.0, 1.0)
    }

    /// The setup scale's value now (stops, within the scale).
    pub fn scale(&self) -> f32 {
        self.scale.value.clamp(-1.0, (STOP_COUNT - 1) as f32)
    }

    /// Running or paused: how far the scale on show lags what is left
    /// (stops; 0 at rest).
    pub fn lag(&self) -> f32 {
        self.lag.value
    }
}

/// Where a countdown is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Setup: not started.
    Idle,
    Running {
        deadline: Instant,
    },
    Paused {
        remaining: Duration,
    },
    /// Finished at `at` (when it was noticed); on show until the hold ends.
    Done {
        at: Instant,
    },
}

/// What a `poll` changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerEvent {
    /// The countdown reached zero (reported exactly once per countdown).
    Completed,
    /// The finished state's hold ran out: back to the idle setup.
    Dismissed,
}

/// The timer: the selected mode, and for each mode its duration (a stop)
/// and its own countdown. Only the selected mode's countdown runs: leaving
/// a running one pauses it, so switching never loses time and coming back
/// finds it where it was. Session only: the phases are never saved (the
/// durations are settings, saved by the window).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Countdown {
    mode: Mode,
    stops: [usize; 2],
    phases: [Phase; 2],
}

impl Default for Countdown {
    fn default() -> Self {
        Self::with_secs(Mode::Focus.default_secs(), Mode::Break.default_secs())
    }
}

impl Countdown {
    /// Idle in Focus, with these durations (seconds; one that is not a stop
    /// is the mode's default).
    pub fn with_secs(focus: u32, rest: u32) -> Self {
        let stop = |secs: u32, mode: Mode| {
            stop_of(secs)
                .or_else(|| stop_of(mode.default_secs()))
                .unwrap_or(0)
        };
        Self {
            mode: Mode::Focus,
            stops: [stop(focus, Mode::Focus), stop(rest, Mode::Break)],
            phases: [Phase::Idle; 2],
        }
    }

    /// The selected mode's countdown.
    pub fn phase(&self) -> Phase {
        self.phases[self.mode.index()]
    }

    fn phase_mut(&mut self) -> &mut Phase {
        &mut self.phases[self.mode.index()]
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The selected mode's duration, as a stop on the scale.
    pub fn stop(&self) -> usize {
        self.stops[self.mode.index()]
    }

    /// `mode`'s duration (seconds).
    pub fn secs_for(&self, mode: Mode) -> u32 {
        stop_secs(self.stops[mode.index()])
    }

    /// The selected mode's duration.
    pub fn duration(&self) -> Duration {
        Duration::from_secs(u64::from(stop_secs(self.stop())))
    }

    /// Sets the selected mode's duration to stop `stop` (clamped to the
    /// scale), in any phase: the scale shows the time, so setting it sets
    /// the time. Running, it runs on from the new time; paused, it holds it;
    /// a finished one goes back to setup with it. True if anything changed.
    pub fn set_stop(&mut self, stop: usize, now: Instant) -> bool {
        let stop = stop.min(STOP_COUNT - 1);
        let changed = stop != self.stop();
        self.stops[self.mode.index()] = stop;
        let duration = self.duration();
        let phase = self.phase_mut();
        let before = *phase;
        *phase = match *phase {
            Phase::Idle | Phase::Done { .. } => Phase::Idle,
            Phase::Running { .. } => Phase::Running {
                deadline: now + duration,
            },
            Phase::Paused { .. } => Phase::Paused {
                remaining: duration,
            },
        };
        changed || *phase != before
    }

    /// Moves `delta` stops along the scale (clamped), from the duration in
    /// setup or from what is left otherwise.
    pub fn step(&mut self, delta: i32, now: Instant) -> bool {
        let from = match self.phase() {
            Phase::Idle => self.stop() as f32,
            _ => scale_value(self, now).round(),
        };
        let to = (from as i64 + i64::from(delta)).clamp(0, STOP_COUNT as i64 - 1);
        self.set_stop(to as usize, now)
    }

    /// Switches Focus / Break, at any time. A running countdown is paused
    /// (kept, to resume on coming back); a finished one is dismissed. True
    /// if the mode changed.
    pub fn set_mode(&mut self, mode: Mode, now: Instant) -> bool {
        if mode == self.mode {
            return false;
        }
        let phase = self.phase_mut();
        *phase = match *phase {
            Phase::Running { deadline } => Phase::Paused {
                remaining: deadline.saturating_duration_since(now),
            },
            Phase::Done { .. } => Phase::Idle,
            p => p,
        };
        self.mode = mode;
        true
    }

    /// The round button: start (idle, or again once finished), pause,
    /// resume. True if the phase changed.
    pub fn primary(&mut self, now: Instant) -> bool {
        let duration = self.duration();
        let phase = self.phase_mut();
        *phase = match *phase {
            Phase::Idle | Phase::Done { .. } => Phase::Running {
                deadline: now + duration,
            },
            Phase::Running { deadline } => Phase::Paused {
                remaining: deadline.saturating_duration_since(now),
            },
            Phase::Paused { remaining } => Phase::Running {
                deadline: now + remaining,
            },
        };
        true
    }

    /// Cancel / reset: the selected mode back to its setup, keeping the
    /// duration. True if anything was running, paused or on show.
    pub fn cancel(&mut self) -> bool {
        std::mem::replace(self.phase_mut(), Phase::Idle) != Phase::Idle
    }

    /// Time left at `now` (the whole duration while idle, none once done).
    pub fn remaining(&self, now: Instant) -> Duration {
        match self.phase() {
            Phase::Idle => self.duration(),
            Phase::Running { deadline } => deadline.saturating_duration_since(now),
            Phase::Paused { remaining } => remaining,
            Phase::Done { .. } => Duration::ZERO,
        }
    }

    /// Whole seconds shown: what is left, rounded up (5:00 at the start, 0:01
    /// in the last second, 0:00 only when done).
    pub fn display_secs(&self, now: Instant) -> u64 {
        self.remaining(now).as_millis().div_ceil(1000) as u64
    }

    /// Applies what `now` means for the countdown: a running one that reached
    /// its deadline completes (once: it is then done, not running); a done
    /// one whose hold ran out goes back to idle. Anything else is untouched.
    pub fn poll(&mut self, now: Instant) -> Option<TimerEvent> {
        let phase = self.phase_mut();
        match *phase {
            Phase::Running { deadline } if now >= deadline => {
                *phase = Phase::Done { at: now };
                Some(TimerEvent::Completed)
            }
            Phase::Done { at } if now >= at + Duration::from_millis(TIMER_DONE_HOLD_MS) => {
                *phase = Phase::Idle;
                Some(TimerEvent::Dismissed)
            }
            _ => None,
        }
    }

    /// When the next `poll` can change something on screen: running, when
    /// the displayed second next changes (the last one ends at the
    /// deadline); done, when its hold ends. None when nothing will change by
    /// itself (idle, paused): no wake-ups at all.
    pub fn next_wake(&self, now: Instant) -> Option<Duration> {
        match self.phase() {
            Phase::Running { deadline } => {
                let left = deadline.saturating_duration_since(now).as_millis() as u64;
                Some(Duration::from_millis(if left == 0 {
                    0
                } else {
                    (left - 1) % 1000 + 1
                }))
            }
            Phase::Done { at } => Some(
                (at + Duration::from_millis(TIMER_DONE_HOLD_MS)).saturating_duration_since(now),
            ),
            Phase::Idle | Phase::Paused { .. } => None,
        }
    }

    /// Shown on the collapsed notch: while it runs, and while a finished one
    /// is on show. Idle and paused countdowns leave the notch as it was.
    pub fn shows_collapsed(&self) -> bool {
        matches!(self.phase(), Phase::Running { .. } | Phase::Done { .. })
    }

    /// Milliseconds since it finished (None unless done).
    pub fn done_for(&self, now: Instant) -> Option<f32> {
        match self.phase() {
            Phase::Done { at } => Some(now.saturating_duration_since(at).as_secs_f32() * 1000.0),
            _ => None,
        }
    }
}

/// Opacity of the finished countdown's numerals `ms` after it finished: a
/// few gentle dips, then steady (1).
pub fn done_pulse(ms: f32) -> f32 {
    if !(0.0..TIMER_PULSE_MS * TIMER_PULSES).contains(&ms) {
        return 1.0;
    }
    let s = (std::f32::consts::PI * ms / TIMER_PULSE_MS).sin();
    1.0 - 0.5 * s * s
}

/// Whether the pulse still needs frames `ms` after finishing.
pub fn pulsing(ms: f32) -> bool {
    (0.0..TIMER_PULSE_MS * TIMER_PULSES).contains(&ms)
}

/// Where the collapsed notch shows a countdown: (bubble beside the notch,
/// the visualizer's slot). Only while collapsed: during playback the slot
/// belongs to the visualizer, so a running countdown takes the bubble (which
/// is absorbed back into the notch the moment it finishes); otherwise the
/// countdown takes the slot, which also shows it finishing.
pub fn collapsed_targets(
    running: bool,
    shows: bool,
    collapsed: bool,
    playing: bool,
) -> (bool, bool) {
    (
        running && collapsed && playing,
        shows && collapsed && !playing,
    )
}

/// How often the bubble's ring needs redrawing (ms) for a countdown
/// `total_ms` long on a ring `radius_px` round: about every half pixel of
/// its circumference, but no faster than ~30 fps and no slower than the
/// once a second the numerals need.
pub fn ring_step_ms(total_ms: u64, radius_px: f32) -> u64 {
    let circumference = (std::f32::consts::TAU * radius_px).max(1.0);
    ((total_ms as f32 / (2.0 * circumference)) as u64).clamp(33, 1000)
}

/// A presentation's presence, 0 (gone) to 1 (fully shown), moved by the
/// notch's own springs: in on `EXPAND_SPRING`, out on `COLLAPSE_SPRING`, the
/// velocity carried over when the target flips mid-way (so a reversal is
/// smooth). Stepped only while it moves.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Presence {
    value: f32,
    vel: f32,
    on: bool,
}

impl Presence {
    pub fn level(&self) -> f32 {
        self.value
    }

    pub fn is_animating(&self) -> bool {
        self.value != f32::from(u8::from(self.on)) || self.vel != 0.0
    }

    /// Heads for shown / gone: animated, or at once when `instant` (Reduced
    /// motion). True if the target changed.
    pub fn set(&mut self, on: bool, instant: bool) -> bool {
        let changed = std::mem::replace(&mut self.on, on) != on;
        if instant {
            self.value = f32::from(u8::from(on));
            self.vel = 0.0;
        }
        changed
    }

    /// Advances by `dt_ms`; true while still moving. Settles exactly on the
    /// target once it no longer visibly moves.
    pub fn step(&mut self, dt_ms: f32) -> bool {
        self.step_on(dt_ms, COLLAPSE_SPRING)
    }

    /// `step`, going out on `out` (in is always `EXPAND_SPRING`).
    pub fn step_on(&mut self, dt_ms: f32, out: crate::config::Spring) -> bool {
        if !self.is_animating() {
            return false;
        }
        let target = f32::from(u8::from(self.on));
        let spring = if self.on { EXPAND_SPRING } else { out };
        (self.value, self.vel) = spring.advance(self.value, self.vel, target, dt_ms);
        if !self.on {
            // Never below gone (the collapse spring is critically damped)
            self.value = self.value.max(0.0);
        }
        if (self.value - target).abs() < 1e-3 && self.vel.abs() < 0.02 {
            self.value = target;
            self.vel = 0.0;
        }
        self.is_animating()
    }
}

/// The bubble's way back into the notch: unhurried and critically damped,
/// like a drop drawn back into its body (the shell's springs are untouched).
pub const ABSORB_SPRING: crate::config::Spring = crate::config::Spring {
    zeta: 1.0,
    response: 0.55,
};

/// "5 min", "1 hr 30 min", "45 sec" (or spelled out for screen readers:
/// "5 minutes", "1 hour 30 minutes", "45 seconds").
pub fn describe(secs: u64, long: bool) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let unit = |n: u64, short: &str, word: &str| {
        if long {
            format!("{n} {word}{}", if n == 1 { "" } else { "s" })
        } else {
            format!("{n} {short}")
        }
    };
    let parts: Vec<String> = [
        (h, "hr", "hour"),
        (m, "min", "minute"),
        (s, "sec", "second"),
    ]
    .into_iter()
    .filter(|(n, _, _)| *n > 0)
    .map(|(n, short, word)| unit(n, short, word))
    .collect();
    if parts.is_empty() {
        unit(0, "sec", "second")
    } else {
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// The stop of `m` whole minutes.
    fn minute(m: u32) -> usize {
        stop_of(m * 60).unwrap()
    }

    #[test]
    fn test_stops_run_from_30_seconds_to_2_hours() {
        let secs: Vec<u32> = (0..STOP_COUNT).map(stop_secs).collect();
        assert_eq!(&secs[..4], [30, 45, 60, 120]);
        assert_eq!(secs[STOP_COUNT - 1], 120 * 60);
        assert!(secs.windows(2).all(|w| w[0] < w[1]), "in order");
        assert_eq!(stop_secs(usize::MAX), 120 * 60, "clamped");
        assert_eq!(stop_of(15), None);
        // Positions: a stop at its own index, continuous between them
        for i in 0..STOP_COUNT {
            assert_eq!(scale_position(stop_secs(i) as f32), i as f32);
        }
        assert_eq!(scale_position(0.0), -1.0);
        assert_eq!(scale_position(90.0), 2.5);
        assert_eq!(scale_position(37.5), 0.5);
        assert_eq!(scale_position(15.0), -0.5);
    }

    #[test]
    fn test_focus_and_break_have_their_own_durations() {
        let t = Countdown::default();
        assert_eq!((t.mode(), t.secs_for(Mode::Focus)), (Mode::Focus, 1500));
        assert_eq!(t.secs_for(Mode::Break), 300, "a 5 min break");
        assert_eq!(t.duration(), Duration::from_secs(25 * 60));
        assert_eq!(t.phase(), Phase::Idle);
        let t = Countdown::with_secs(30, 45);
        assert_eq!((t.secs_for(Mode::Focus), t.secs_for(Mode::Break)), (30, 45));
        let t = Countdown::with_secs(0, 7);
        assert_eq!(
            (t.secs_for(Mode::Focus), t.secs_for(Mode::Break)),
            (1500, 300),
            "not a stop: the defaults"
        );
    }

    #[test]
    fn test_setting_the_time_works_in_every_phase() {
        let t0 = Instant::now();
        let mut t = Countdown::default();
        // Setup: clamped to the scale
        assert!(t.set_stop(0, t0));
        assert_eq!(t.duration(), ms(30_000), "30 seconds");
        assert!(!t.step(-1, t0), "already the shortest");
        assert!(t.set_stop(usize::MAX, t0));
        assert_eq!(t.duration(), Duration::from_secs(120 * 60));
        assert!(!t.step(3, t0));
        assert!(t.step(-1, t0));
        assert_eq!(t.duration(), Duration::from_secs(119 * 60));
        assert_eq!(t.secs_for(Mode::Break), 300, "the other mode keeps its own");
        // Running: runs on from the new time
        assert!(t.set_stop(minute(10), t0));
        t.primary(t0);
        assert!(t.set_stop(minute(3), t0 + ms(5_000)));
        assert!(matches!(t.phase(), Phase::Running { .. }));
        assert_eq!(t.remaining(t0 + ms(6_000)), ms(179_000));
        // From what is left: 2:59 left, one stop more is 3 minutes...
        assert!(t.step(1, t0 + ms(6_000)));
        assert_eq!(t.remaining(t0 + ms(6_000)), ms(4 * 60_000));
        // ...and under a minute, 30 and 45 second stops
        assert!(t.set_stop(1, t0 + ms(6_000)));
        assert_eq!(t.remaining(t0 + ms(6_000)), ms(45_000));
        // Paused: holds the new time
        t.primary(t0 + ms(7_000));
        assert!(t.set_stop(minute(2), t0 + ms(8_000)));
        assert_eq!(
            t.phase(),
            Phase::Paused {
                remaining: ms(120_000)
            }
        );
        // Finished: back to setup with it
        t.primary(t0 + ms(8_000));
        assert_eq!(t.poll(t0 + ms(128_000)), Some(TimerEvent::Completed));
        assert!(t.set_stop(minute(5), t0 + ms(129_000)));
        assert_eq!((t.phase(), t.duration()), (Phase::Idle, ms(300_000)));
    }

    #[test]
    fn test_switching_modes_pauses_and_keeps_each_countdown() {
        let t0 = Instant::now();
        let mut t = Countdown::default();
        assert!(t.set_mode(Mode::Break, t0));
        assert_eq!((t.mode(), t.duration()), (Mode::Break, ms(300_000)));
        assert!(!t.set_mode(Mode::Break, t0), "already");
        assert!(t.set_stop(minute(7), t0));
        assert!(t.set_mode(Mode::Focus, t0));
        assert_eq!(t.duration(), ms(25 * 60_000), "each mode keeps its own");
        assert_eq!(t.secs_for(Mode::Break), 7 * 60);
        // Running Focus, then Break: Focus is paused with what it had left
        t.primary(t0);
        assert!(t.set_mode(Mode::Break, t0 + ms(30_000)));
        assert_eq!(t.phase(), Phase::Idle, "Break's own countdown");
        assert_eq!(
            t.poll(t0 + ms(99 * 60_000)),
            None,
            "Focus never completes away"
        );
        // Break runs on its own
        t.primary(t0 + ms(40_000));
        assert!(matches!(t.phase(), Phase::Running { .. }));
        // Back to Focus: found where it was (Break is paused in turn)
        assert!(t.set_mode(Mode::Focus, t0 + ms(100_000)));
        assert_eq!(
            t.phase(),
            Phase::Paused {
                remaining: ms(25 * 60_000 - 30_000)
            },
            "no time lost"
        );
        assert!(t.set_mode(Mode::Break, t0 + ms(110_000)));
        assert_eq!(
            t.phase(),
            Phase::Paused {
                remaining: ms(7 * 60_000 - 60_000)
            }
        );
        // A finished one is dismissed by switching away
        t.primary(t0 + ms(110_000));
        let end = t0 + ms(110_000 + 6 * 60_000);
        assert_eq!(t.poll(end), Some(TimerEvent::Completed));
        assert!(t.set_mode(Mode::Focus, end));
        assert!(t.set_mode(Mode::Break, end));
        assert_eq!(t.phase(), Phase::Idle);
        assert_eq!(
            t.poll(end + ms(TIMER_DONE_HOLD_MS)),
            None,
            "no stale dismiss"
        );
    }

    #[test]
    fn test_remaining_comes_from_the_deadline() {
        let t0 = Instant::now();
        let mut t = Countdown::default();
        t.set_stop(minute(1), t0);
        assert_eq!(t.display_secs(t0), 60, "idle shows the duration");
        t.primary(t0);
        assert_eq!(t.remaining(t0), Duration::from_secs(60));
        assert_eq!(t.display_secs(t0), 60, "1:00 at the start");
        assert_eq!(t.display_secs(t0 + ms(1)), 60, "rounded up");
        assert_eq!(t.display_secs(t0 + ms(1000)), 59);
        assert_eq!(
            t.display_secs(t0 + ms(59_001)),
            1,
            "0:01 in the last second"
        );
        // Accurate however late or irregular the wake-ups are: no counter
        assert_eq!(t.remaining(t0 + ms(12_345)), ms(47_655));
        assert_eq!(
            t.remaining(t0 + ms(90_000)),
            Duration::ZERO,
            "never negative"
        );
    }

    #[test]
    fn test_wakes_line_up_with_the_displayed_second() {
        let t0 = Instant::now();
        let mut t = Countdown::default();
        assert_eq!(t.next_wake(t0), None, "idle: no wake-ups");
        t.set_stop(minute(1), t0);
        t.primary(t0);
        assert_eq!(t.next_wake(t0), Some(ms(1000)));
        assert_eq!(t.next_wake(t0 + ms(250)), Some(ms(750)));
        assert_eq!(t.next_wake(t0 + ms(59_999)), Some(ms(1)), "the deadline");
        assert_eq!(t.next_wake(t0 + ms(60_000)), Some(ms(0)), "due now");
        // Each wake lands where the display changes: every second, 60 to 0
        let mut at = t0;
        let mut shown = vec![t.display_secs(at)];
        while let Some(w) = t.next_wake(at).filter(|w| !w.is_zero()) {
            at += w;
            shown.push(t.display_secs(at));
        }
        assert_eq!(shown, (0..=60).rev().collect::<Vec<u64>>());
        t.primary(t0 + ms(1500));
        assert_eq!(t.next_wake(t0 + ms(1500)), None, "paused: none");
    }

    #[test]
    fn test_pause_keeps_what_is_left_and_resume_continues_accurately() {
        let t0 = Instant::now();
        let mut t = Countdown::default();
        t.set_stop(minute(1), t0);
        t.primary(t0);
        t.primary(t0 + ms(10_400)); // pause
        assert_eq!(
            t.phase(),
            Phase::Paused {
                remaining: ms(49_600)
            }
        );
        // Paused time does not count, however long
        assert_eq!(t.remaining(t0 + ms(500_000)), ms(49_600));
        assert_eq!(
            t.poll(t0 + ms(500_000)),
            None,
            "a paused one never completes"
        );
        t.primary(t0 + ms(600_000)); // resume
        assert_eq!(t.remaining(t0 + ms(600_000)), ms(49_600));
        assert_eq!(t.poll(t0 + ms(649_599)), None);
        assert_eq!(t.poll(t0 + ms(649_600)), Some(TimerEvent::Completed));
    }

    #[test]
    fn test_cancel_returns_to_a_defined_idle_state() {
        let t0 = Instant::now();
        for advance in [0u64, 2_000, 400_000] {
            let mut t = Countdown::default();
            t.set_stop(minute(3), t0);
            t.primary(t0);
            if advance >= 2_000 {
                t.primary(t0 + ms(advance)); // pause
            }
            assert!(t.cancel());
            assert_eq!(t.phase(), Phase::Idle);
            assert_eq!(t.duration(), ms(180_000), "keeps the chosen duration");
            assert_eq!(t.display_secs(t0 + ms(advance)), 180);
            assert_eq!(t.next_wake(t0), None);
            assert!(!t.shows_collapsed());
            assert!(!t.cancel(), "already idle");
        }
    }

    #[test]
    fn test_completion_happens_exactly_once_then_holds_then_dismisses() {
        let t0 = Instant::now();
        let mut t = Countdown::default();
        t.set_stop(minute(1), t0);
        t.primary(t0);
        assert_eq!(t.poll(t0 + ms(59_999)), None);
        // A late wake (e.g. after sleep) still completes once, then never again
        let late = t0 + ms(62_000);
        assert_eq!(t.poll(late), Some(TimerEvent::Completed));
        for k in 0..5 {
            assert_eq!(t.poll(late + ms(k * 100)), None, "completed only once");
        }
        assert_eq!(t.phase(), Phase::Done { at: late });
        assert!(t.shows_collapsed());
        assert_eq!(t.display_secs(late), 0);
        assert_eq!(
            t.next_wake(late),
            Some(ms(TIMER_DONE_HOLD_MS)),
            "one wake for the hold"
        );
        // The hold ends by itself: never stuck showing "done"
        let end = late + ms(TIMER_DONE_HOLD_MS);
        assert_eq!(t.poll(end - ms(1)), None);
        assert_eq!(t.poll(end), Some(TimerEvent::Dismissed));
        assert_eq!(t.phase(), Phase::Idle);
        assert_eq!(t.poll(end + ms(10_000)), None);
        // Start again from done (restart) works too
        let mut t = Countdown::default();
        t.primary(t0);
        t.poll(t0 + Duration::from_secs(25 * 60));
        assert!(t.primary(t0 + Duration::from_secs(25 * 60 + 1)));
        assert!(matches!(t.phase(), Phase::Running { .. }));
    }

    #[test]
    fn test_stale_wakes_never_affect_a_newer_countdown() {
        let t0 = Instant::now();
        // A 1-minute countdown is cancelled and a 10-minute one started: a
        // wake-up armed for the old deadline changes nothing
        let mut t = Countdown::default();
        t.set_stop(minute(1), t0);
        t.primary(t0);
        let old_deadline = t0 + Duration::from_secs(60);
        t.cancel();
        t.set_stop(minute(10), t0);
        t.primary(t0 + ms(5_000));
        assert_eq!(t.poll(old_deadline), None);
        assert!(matches!(t.phase(), Phase::Running { .. }));
        assert_eq!(t.display_secs(old_deadline), 545);
        // Paused and resumed: the original deadline is stale too
        let mut t = Countdown::default();
        t.set_stop(minute(1), t0);
        t.primary(t0);
        t.primary(t0 + ms(30_000));
        t.primary(t0 + ms(40_000)); // now ends at 70 s
        assert_eq!(t.poll(t0 + ms(60_000)), None);
        assert_eq!(t.poll(t0 + ms(70_000)), Some(TimerEvent::Completed));
        // A hold wake from an earlier finish can't dismiss a restarted one
        let mut t = Countdown::default();
        t.set_stop(minute(1), t0);
        t.primary(t0);
        t.poll(t0 + ms(60_000));
        t.primary(t0 + ms(60_500)); // restarted
        assert_eq!(t.poll(t0 + ms(60_000 + TIMER_DONE_HOLD_MS)), None);
        assert!(matches!(t.phase(), Phase::Running { .. }), "still running");
        // ...and a cancelled finish ignores its hold wake
        t.cancel();
        assert_eq!(t.poll(t0 + ms(1_000_000)), None);
        assert_eq!(t.phase(), Phase::Idle);
    }

    #[test]
    fn test_collapsed_presentation_follows_playback() {
        // (running, shows, collapsed, playing) -> (bubble, slot)
        let targets = collapsed_targets;
        assert_eq!(targets(true, true, true, true), (true, false));
        assert_eq!(targets(true, true, true, false), (false, true));
        assert_eq!(targets(true, true, false, true), (false, false), "expanded");
        assert_eq!(targets(true, true, false, false), (false, false));
        assert_eq!(
            targets(false, false, true, true),
            (false, false),
            "idle/paused"
        );
        assert_eq!(targets(false, false, true, false), (false, false));
        // Finished: the bubble is absorbed; without playback the slot shows it
        assert_eq!(targets(false, true, true, true), (false, false), "absorbed");
        assert_eq!(targets(false, true, true, false), (false, true));
        // Media starting / stopping mid-countdown swaps the two, never both
        let t0 = Instant::now();
        let mut t = Countdown::default();
        t.primary(t0);
        let mut last = None;
        for playing in [false, true, false, true] {
            let running = matches!(t.phase(), Phase::Running { .. });
            let now = collapsed_targets(running, t.shows_collapsed(), true, playing);
            assert!(now.0 != now.1, "exactly one presentation");
            assert_ne!(Some(now), last);
            last = Some(now);
        }
        assert!(
            matches!(t.phase(), Phase::Running { .. }),
            "media never resets it"
        );
        t.primary(t0 + ms(1_000));
        assert!(
            !t.shows_collapsed(),
            "paused leaves the collapsed notch as it was"
        );
    }

    #[test]
    fn test_presence_springs_in_and_out_and_reverses_smoothly() {
        let mut p = Presence::default();
        assert!(!p.is_animating() && p.level() == 0.0);
        assert!(p.set(true, false));
        let mut frames = 0;
        let mut max: f32 = 0.0;
        let mut prev = 0.0;
        while p.step(16.0) {
            frames += 1;
            max = max.max(p.level());
            assert!(
                p.level() >= prev - 0.006,
                "no visible undershoot on the way in"
            );
            prev = p.level();
            assert!(frames < 200, "settles");
        }
        assert_eq!(p.level(), 1.0, "settles exactly");
        assert!(max < 1.02, "restrained overshoot: {max}");
        // Out, reversed half-way: continuous (no jump) and back in
        p.set(false, false);
        for _ in 0..6 {
            p.step(16.0);
        }
        let mid = p.level();
        assert!(mid > 0.05 && mid < 0.95, "mid-way: {mid}");
        p.set(true, false);
        p.step(1.0);
        assert!((p.level() - mid).abs() < 0.05, "no jump on reversal");
        while p.step(16.0) {}
        assert_eq!(p.level(), 1.0);
        p.set(false, false);
        let mut prev = 1.0;
        while p.step(16.0) {
            assert!(
                p.level() <= prev + 1e-6 && p.level() >= 0.0,
                "monotonic out"
            );
            prev = p.level();
        }
        assert_eq!(p.level(), 0.0);
        assert!(!p.step(16.0), "no frames once settled");
    }

    #[test]
    fn test_bubble_and_slot_follow_collapse_expand_and_playback() {
        let t0 = Instant::now();
        let mut t = Countdown::default();
        t.primary(t0);
        let (mut bubble, mut slot) = (Presence::default(), Presence::default());
        let aim = |collapsed: bool, playing: bool, b: &mut Presence, s: &mut Presence| {
            let running = matches!(t.phase(), Phase::Running { .. });
            let (on_b, on_s) = collapsed_targets(running, t.shows_collapsed(), collapsed, playing);
            b.set(on_b, false);
            s.set(on_s, false);
        };
        let run = |b: &mut Presence, s: &mut Presence, frames: usize| {
            for _ in 0..frames {
                b.step(16.0);
                s.step(16.0);
            }
        };
        // Collapsed during playback: the bubble buds off
        aim(true, true, &mut bubble, &mut slot);
        run(&mut bubble, &mut slot, 6);
        let mid = bubble.level();
        assert!(mid > 0.1 && mid < 1.0, "appearing: {mid}");
        // Expanded while it appears: it reverses from where it is
        aim(false, true, &mut bubble, &mut slot);
        bubble.step(1.0);
        assert!((bubble.level() - mid).abs() < 0.05, "no jump");
        run(&mut bubble, &mut slot, 120);
        assert_eq!(
            (bubble.level(), slot.level()),
            (0.0, 0.0),
            "gone while expanded"
        );
        // Collapsed again, settled, then playback stops: bubble out, slot in
        aim(true, true, &mut bubble, &mut slot);
        run(&mut bubble, &mut slot, 120);
        assert_eq!(bubble.level(), 1.0);
        aim(true, false, &mut bubble, &mut slot);
        run(&mut bubble, &mut slot, 4);
        assert!(bubble.level() < 1.0 && slot.level() > 0.0, "crossing over");
        run(&mut bubble, &mut slot, 120);
        assert_eq!((bubble.level(), slot.level()), (0.0, 1.0));
        assert!(
            !bubble.is_animating() && !slot.is_animating(),
            "no frames once settled"
        );
        // The countdown itself never changed
        assert!(matches!(t.phase(), Phase::Running { .. }));
        assert_eq!(t.remaining(t0), t.duration());
    }

    #[test]
    fn test_ring_redraws_often_enough_to_deplete_smoothly() {
        // A 5 s countdown on a ~10 px ring: about every half pixel of travel
        let short = ring_step_ms(5_000, 10.0);
        assert!((33..=45).contains(&short), "{short}");
        // Long countdowns: once a second is already smoother than a pixel
        assert_eq!(ring_step_ms(300_000, 10.0), 1000);
        assert_eq!(ring_step_ms(43_200_000, 15.0), 1000);
        // Never faster than ~30 fps
        assert_eq!(ring_step_ms(1, 10.0), 33);
    }

    #[test]
    fn test_absorption_is_smooth_and_unhurried() {
        let mut p = Presence::default();
        p.set(true, true);
        p.set(false, false);
        let (mut ms, mut prev) = (0.0, 1.0);
        while p.step_on(16.0, ABSORB_SPRING) {
            ms += 16.0;
            assert!(
                p.level() <= prev + 1e-6 && p.level() >= 0.0,
                "one way, no bounce"
            );
            assert!(prev - p.level() < 0.12, "no lurch");
            prev = p.level();
        }
        assert!((400.0..=1200.0).contains(&ms), "about half a second: {ms}");
        assert_eq!(p.level(), 0.0);
    }

    #[test]
    fn test_reduced_motion_snaps_presence_and_skips_frames() {
        let mut p = Presence::default();
        p.set(true, true);
        assert_eq!(p.level(), 1.0);
        assert!(!p.is_animating(), "no frames needed");
        // Mid-animation, an instant change snaps too
        p.set(false, false);
        p.step(16.0);
        p.set(true, true);
        assert_eq!(p.level(), 1.0);
        assert!(!p.is_animating());
    }

    #[test]
    fn test_done_pulse_is_gentle_and_bounded() {
        assert_eq!(done_pulse(0.0), 1.0);
        let lowest = (0..2100)
            .map(|ms| done_pulse(ms as f32))
            .fold(1.0, f32::min);
        assert!(
            (0.49..0.6).contains(&lowest),
            "dips to about half: {lowest}"
        );
        let total = TIMER_PULSE_MS * TIMER_PULSES;
        assert_eq!(done_pulse(total), 1.0, "then steady");
        assert!(pulsing(total - 1.0) && !pulsing(total) && !pulsing(-1.0));
    }

    #[test]
    fn test_describe_durations() {
        assert_eq!(describe(45, false), "45 sec");
        assert_eq!(describe(300, false), "5 min");
        assert_eq!(describe(5400, false), "1 hr 30 min");
        assert_eq!(describe(3600, true), "1 hour");
        assert_eq!(describe(5400, true), "1 hour 30 minutes");
        assert_eq!(describe(61, true), "1 minute 1 second");
        assert_eq!(describe(0, true), "0 seconds");
    }

    #[test]
    fn test_scale_value_follows_the_phase() {
        let now = Instant::now();
        let mut t = Countdown::default();
        assert_eq!(scale_value(&t, now), minute(25) as f32);
        t.primary(now);
        let later = now + Duration::from_secs(90);
        // 23:30 left: half way between the 23 and 24 minute stops
        assert!((scale_value(&t, later) - (minute(23) as f32 + 0.5)).abs() < 1e-3);
        t.primary(later); // paused: frozen
        let frozen = scale_value(&t, later + Duration::from_secs(600));
        assert!((frozen - (minute(23) as f32 + 0.5)).abs() < 1e-3);
        let mut done = Countdown::with_secs(60, 300);
        done.primary(now - Duration::from_secs(70));
        assert!(done.poll(now).is_some());
        assert_eq!(scale_value(&done, now), -1.0, "nothing left");
    }

    #[test]
    fn test_timer_motion_glides_retargets_and_settles() {
        let t0 = Instant::now();
        let mut t = Countdown::default();
        let mut m = TimerMotion::new(&t);
        assert!(!m.is_animating(), "at rest on start");
        assert_eq!((m.mode(), m.scale()), (0.0, minute(25) as f32));
        // Break: both glide, monotonically, never jumping
        assert!(t.set_mode(Mode::Break, t0));
        m.aim(&t, None, t0, false);
        assert!(m.is_animating());
        let mut last = m.scale();
        for _ in 0..6 {
            m.step(16.0);
            assert!(m.scale() <= last && m.scale() > minute(5) as f32 && m.mode() > 0.0);
            last = m.scale();
        }
        // Reversal mid-glide: continues from where it is (no jump)
        let (mode, scale) = (m.mode(), m.scale());
        assert!(t.set_mode(Mode::Focus, t0));
        m.aim(&t, None, t0, false);
        assert_eq!((m.mode(), m.scale()), (mode, scale));
        m.step(16.0);
        assert!((m.scale() - scale).abs() < 2.0, "velocity carried, no jump");
        // Settles, then frames stop
        let mut frames = 0;
        while m.step(16.0) {
            frames += 1;
            assert!(frames < 200, "settles");
        }
        assert!(!m.is_animating());
        assert_eq!((m.mode(), m.scale()), (0.0, minute(25) as f32));
        // Reduced motion: at once, nothing to animate
        assert!(t.set_stop(minute(40), t0));
        m.aim(&t, None, t0, true);
        assert!(!m.is_animating());
        assert_eq!(m.scale(), minute(40) as f32);
        // Cancel mid-run: glides back from the time that was left
        t.primary(Instant::now());
        m.aim(&t, Some(12.5), Instant::now(), false);
        assert_eq!(
            m.scale(),
            minute(40) as f32,
            "the setup scale holds while it runs"
        );
        t.cancel();
        m.aim(&t, Some(12.5), Instant::now(), false);
        assert_eq!(m.scale(), 12.5);
        assert!(m.is_animating());
        // Running: a new time glides in from what was on show, no jump
        let now = Instant::now();
        t.primary(now);
        let from = scale_value(&t, now);
        assert!(t.set_stop(minute(30), now));
        m.aim(&t, Some(from), now, false);
        assert!((scale_value(&t, now) + m.lag() - from).abs() < 1e-3);
        let mut last = m.lag().abs();
        while m.step(16.0) {
            assert!(m.lag().abs() <= last + 1e-3, "eases toward what is left");
            last = m.lag().abs();
        }
        assert_eq!(m.lag(), 0.0, "settles on what is left");
        // Reduced motion: at once
        assert!(t.set_stop(minute(10), now));
        m.aim(&t, Some(from), now, true);
        assert_eq!(m.lag(), 0.0);
    }
}
