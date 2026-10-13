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

/// The durations the scale offers (seconds): seconds through hours, finer
/// where timers are most often set. 5 s is the shortest, 12 h the longest.
pub const STOPS: [u32; 28] = [
    5, 10, 15, 20, 30, 45, // seconds
    60, 120, 180, 240, 300, 360, 480, 600, 720, 900, 1200, 1500, 1800, 2700, // minutes
    3600, 5400, 7200, 10800, 14400, 21600, 28800, 43200, // hours
];
/// 5 minutes.
pub const DEFAULT_STOP: usize = 10;

/// Where the countdown is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Setup: the duration can be changed.
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

/// The countdown: the chosen duration (a stop on the scale) and its phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Countdown {
    stop: usize,
    phase: Phase,
}

impl Default for Countdown {
    fn default() -> Self {
        Self {
            stop: DEFAULT_STOP,
            phase: Phase::Idle,
        }
    }
}

impl Countdown {
    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn stop(&self) -> usize {
        self.stop
    }

    /// The chosen duration.
    pub fn duration(&self) -> Duration {
        Duration::from_secs(u64::from(STOPS[self.stop]))
    }

    /// Chooses a stop (clamped to the scale); only while idle, so a running
    /// countdown is never changed under the user. True if it changed.
    pub fn set_stop(&mut self, stop: usize) -> bool {
        let stop = stop.min(STOPS.len() - 1);
        if self.phase != Phase::Idle || stop == self.stop {
            return false;
        }
        self.stop = stop;
        true
    }

    /// Moves `delta` stops along the scale (clamped); idle only.
    pub fn step_stop(&mut self, delta: i32) -> bool {
        let stop = (self.stop as i64 + i64::from(delta)).clamp(0, STOPS.len() as i64 - 1);
        self.set_stop(stop as usize)
    }

    /// The round button: start (idle, or again once finished), pause,
    /// resume. True if the phase changed.
    pub fn primary(&mut self, now: Instant) -> bool {
        self.phase = match self.phase {
            Phase::Idle | Phase::Done { .. } => Phase::Running {
                deadline: now + self.duration(),
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

    /// Cancel / reset: back to the idle setup, keeping the duration. True if
    /// anything was running, paused or on show.
    pub fn cancel(&mut self) -> bool {
        std::mem::replace(&mut self.phase, Phase::Idle) != Phase::Idle
    }

    /// Time left at `now` (the whole duration while idle, none once done).
    pub fn remaining(&self, now: Instant) -> Duration {
        match self.phase {
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
        match self.phase {
            Phase::Running { deadline } if now >= deadline => {
                self.phase = Phase::Done { at: now };
                Some(TimerEvent::Completed)
            }
            Phase::Done { at } if now >= at + Duration::from_millis(TIMER_DONE_HOLD_MS) => {
                self.phase = Phase::Idle;
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
        match self.phase {
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
        matches!(self.phase, Phase::Running { .. } | Phase::Done { .. })
    }

    /// Milliseconds since it finished (None unless done).
    pub fn done_for(&self, now: Instant) -> Option<f32> {
        match self.phase {
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

    #[test]
    fn test_stops_cover_seconds_to_hours_in_order() {
        assert!(STOPS.windows(2).all(|w| w[0] < w[1]), "strictly increasing");
        assert_eq!(STOPS[0], 5, "seconds");
        assert_eq!(*STOPS.last().unwrap(), 12 * 3600, "hours, 12 h at most");
        assert_eq!(STOPS[DEFAULT_STOP], 300, "5 minutes by default");
        let t = Countdown::default();
        assert_eq!(t.duration(), Duration::from_secs(300));
        assert_eq!(t.phase(), Phase::Idle);
    }

    #[test]
    fn test_duration_is_validated_and_only_changes_while_idle() {
        let now = Instant::now();
        let mut t = Countdown::default();
        assert!(t.set_stop(0));
        assert!(!t.step_stop(-5), "clamped at the shortest");
        assert_eq!(t.duration(), Duration::from_secs(5));
        assert!(t.set_stop(usize::MAX), "out of range clamps to the longest");
        assert_eq!(t.duration(), Duration::from_secs(43_200));
        assert!(!t.step_stop(3));
        assert!(t.step_stop(-1));
        assert_eq!(t.stop(), STOPS.len() - 2);
        t.primary(now);
        assert!(!t.set_stop(0) && !t.step_stop(-1), "never while running");
        t.primary(now);
        assert!(!t.set_stop(0), "nor while paused");
        t.cancel();
        assert!(t.set_stop(3), "idle again");
    }

    #[test]
    fn test_remaining_comes_from_the_deadline() {
        let t0 = Instant::now();
        let mut t = Countdown::default();
        t.set_stop(6); // 1 minute
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
        t.set_stop(0); // 5 s
        t.primary(t0);
        assert_eq!(t.next_wake(t0), Some(ms(1000)));
        assert_eq!(t.next_wake(t0 + ms(250)), Some(ms(750)));
        assert_eq!(t.next_wake(t0 + ms(4_999)), Some(ms(1)), "the deadline");
        assert_eq!(t.next_wake(t0 + ms(5_000)), Some(ms(0)), "due now");
        // Each wake lands where the display changes
        let mut at = t0;
        let mut shown = vec![t.display_secs(at)];
        while let Some(w) = t.next_wake(at).filter(|w| !w.is_zero()) {
            at += w;
            shown.push(t.display_secs(at));
        }
        assert_eq!(shown, [5, 4, 3, 2, 1, 0]);
        t.primary(t0 + ms(1500));
        assert_eq!(t.next_wake(t0 + ms(1500)), None, "paused: none");
    }

    #[test]
    fn test_pause_keeps_what_is_left_and_resume_continues_accurately() {
        let t0 = Instant::now();
        let mut t = Countdown::default();
        t.set_stop(6); // 60 s
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
            t.set_stop(8); // 3 min
            t.primary(t0);
            if advance >= 2_000 {
                t.primary(t0 + ms(advance)); // pause
            }
            assert!(t.cancel());
            assert_eq!(t.phase(), Phase::Idle);
            assert_eq!(t.stop(), 8, "keeps the chosen duration");
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
        t.set_stop(0); // 5 s
        t.primary(t0);
        assert_eq!(t.poll(t0 + ms(4_999)), None);
        // A late wake (e.g. after sleep) still completes once, then never again
        let late = t0 + ms(7_000);
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
        t.poll(t0 + Duration::from_secs(300));
        assert!(t.primary(t0 + Duration::from_secs(301)));
        assert!(matches!(t.phase(), Phase::Running { .. }));
    }

    #[test]
    fn test_stale_wakes_never_affect_a_newer_countdown() {
        let t0 = Instant::now();
        // A 1-minute countdown is cancelled and a 10-minute one started: a
        // wake-up armed for the old deadline changes nothing
        let mut t = Countdown::default();
        t.set_stop(6);
        t.primary(t0);
        let old_deadline = t0 + Duration::from_secs(60);
        t.cancel();
        t.set_stop(13); // 10 min
        t.primary(t0 + ms(5_000));
        assert_eq!(t.poll(old_deadline), None);
        assert!(matches!(t.phase(), Phase::Running { .. }));
        assert_eq!(t.display_secs(old_deadline), 545);
        // Paused and resumed: the original deadline is stale too
        let mut t = Countdown::default();
        t.set_stop(6);
        t.primary(t0);
        t.primary(t0 + ms(30_000));
        t.primary(t0 + ms(40_000)); // now ends at 70 s
        assert_eq!(t.poll(t0 + ms(60_000)), None);
        assert_eq!(t.poll(t0 + ms(70_000)), Some(TimerEvent::Completed));
        // A hold wake from an earlier finish can't dismiss a restarted one
        let mut t = Countdown::default();
        t.set_stop(0);
        t.primary(t0);
        t.poll(t0 + ms(5_000));
        t.primary(t0 + ms(5_500)); // restarted
        assert_eq!(
            t.poll(t0 + ms(5_000 + TIMER_DONE_HOLD_MS)),
            Some(TimerEvent::Completed)
        );
        assert_eq!(t.poll(t0 + ms(5_000 + TIMER_DONE_HOLD_MS + 1)), None);
        // ...and a cancelled finish ignores its hold wake
        t.cancel();
        assert_eq!(t.poll(t0 + ms(100_000)), None);
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
        assert_eq!(t.remaining(t0), Duration::from_secs(300));
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
}
