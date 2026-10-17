use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::NotifyWinEvent;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, GetDpiForWindow,
    SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    ENDSESSION_CLOSEAPP, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_STATECHANGE, GWLP_USERDATA,
    GetClientRect, GetCursorPos, GetMessageW, GetSystemMetrics, GetWindowLongPtrW, HTCLIENT,
    HTTRANSPARENT, HWND_TOPMOST, IDC_ARROW, KillTimer, LoadCursorW, MA_NOACTIVATE, MSG,
    OBJID_CLIENT, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PostMessageW, PostQuitMessage,
    RegisterClassExW, SM_CXSCREEN, SPI_GETCLIENTAREAANIMATION, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetTimer, SetWindowLongPtrW, SetWindowPos,
    SetWindowTextW, ShowWindow, SystemParametersInfoW, TranslateMessage, UnregisterClassW, WM_APP,
    WM_CAPTURECHANGED, WM_CLIPBOARDUPDATE, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_DISPLAYCHANGE,
    WM_DPICHANGED, WM_ENDSESSION, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
    WM_NCDESTROY, WM_NCHITTEST, WM_POWERBROADCAST, WM_TIMECHANGE, WM_TIMER, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Error, Result};

pub(crate) const WM_MOUSELEAVE: u32 = 0x02A3;

#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(uPeriod: u32) -> u32;
    fn timeEndPeriod(uPeriod: u32) -> u32;
}

use crate::clipboard::{self, ClipboardHistory, ClipboardItem};
use crate::clock::ClockEngine;
use crate::config::AUTO_COLLAPSE_TIMER_ID;
use crate::config::TIMER_TICK_TIMER_ID;
use crate::config::{
    ANIMATION_FRAME_INTERVAL_MS, ANIMATION_TIMER_ID, AnimationState, CLIPBOARD_SETTLE_MS,
    CLIPBOARD_TIMER_ID, CLOCK_TIMER_ID, MEDIA_FEEDBACK_FRAME_MS, MEDIA_FEEDBACK_TIMER_ID,
    MEDIA_LIVE_FRAME_MS, MEDIA_LIVE_TIMER_ID, NotchDimensions, NotchState, WINDOW_CLASS_NAME,
    WINDOW_TITLE, calculate_notch_x,
};
use crate::config::{DROP_RESULT_MS, DROP_RESULT_TIMER_ID};
use crate::config::{HUD_HIDE_TIMER_ID, HUD_HOLD_MS, HUD_RELEASE_TIMER_ID};
use crate::dragdrop::{self, DragEvent, DragSession, DropTarget};
use crate::hud::VolumeKey;
use crate::layout::{MediaLayout, PanelHit, resolve_clipboard_layout};
use crate::layout::{SpaceSelectorLayout, resolve_settings_layout};
use crate::layout::{
    blended_selector, expanded_size_dip, resolve_media_layout_in, space_dimensions,
};
use crate::media::{
    MediaContent, MediaControl, MediaEngine, WM_APP_MEDIA_PLAYBACK_CHANGED,
    WM_APP_MEDIA_PROPERTIES_CHANGED, WM_APP_MEDIA_SESSION_CHANGED, is_media_message,
};
use crate::renderer::Renderer;
use crate::settings_window;
use crate::space::{OecaSettings, OecaSpace, PANEL_SPACE, Scene};
use crate::timer::{Countdown, Phase, Presence};
use windows::Win32::System::Ole::{
    OleInitialize, OleUninitialize, RegisterDragDrop, RevokeDragDrop,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, WM_INPUT, WM_MBUTTONDOWN, WM_RBUTTONDOWN, WindowFromPoint,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GW_OWNER, GetWindow, IDC_HAND, SW_HIDE, SetCursor, WM_MOUSEWHEEL, WM_RBUTTONUP, WM_SETCURSOR,
    WS_EX_TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageTime, PBT_POWERSETTINGCHANGE, SPI_GETKEYBOARDSPEED, WM_HOTKEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_NOTOPMOST, SET_WINDOW_POS_FLAGS, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    Graphics::Dwm::{DWM_TIMING_INFO, DwmGetCompositionTimingInfo},
    System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
    System::Threading::{
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CancelWaitableTimer, CloseThreadpoolWait,
        CreateThreadpoolWait, CreateWaitableTimerExW, PTP_CALLBACK_INSTANCE, PTP_WAIT,
        SetThreadpoolWait, SetWaitableTimer, TIMER_ALL_ACCESS, WaitForThreadpoolWaitCallbacks,
    },
};

/// Owns the process's 1 ms timer-resolution request (`timeBeginPeriod`), held
/// only while the notch animation runs. Requests never stack (reversing an
/// in-flight animation does not raise it again) and are never released twice;
/// `Drop` releases a request still held at shutdown.
#[derive(Debug, Default)]
struct HighResTimer {
    active: bool,
}

impl HighResTimer {
    /// Pure transition: Some(true) = call timeBeginPeriod, Some(false) = call
    /// timeEndPeriod, None = nothing to do.
    fn transition(&mut self, want: bool) -> Option<bool> {
        if self.active == want {
            return None;
        }
        self.active = want;
        Some(want)
    }

    fn set(&mut self, want: bool) {
        match self.transition(want) {
            Some(true) => unsafe {
                let _ = timeBeginPeriod(1);
            },
            Some(false) => unsafe {
                let _ = timeEndPeriod(1);
            },
            None => {}
        }
    }
}

impl Drop for HighResTimer {
    fn drop(&mut self) {
        self.set(false);
    }
}

/// Posted by `FrameClock` for each animation frame.
const WM_APP_ANIM_FRAME: u32 = WM_APP + 42;
/// Ticks after which an unanswered frame message is posted again.
const FRAME_RESEND_TICKS: u32 = 8;

/// What the thread-pool callback needs: where to post, how many ticks ago a
/// frame message was posted and not yet received (0 = none queued), and the
/// timer to wait on again.
struct FrameTarget {
    hwnd: std::sync::atomic::AtomicIsize,
    pending: std::sync::atomic::AtomicU32,
    running: std::sync::atomic::AtomicBool,
    timer: HANDLE,
}

/// Animation frames at the display's refresh rate, only while the notch
/// animates. A high-resolution waitable timer, waited on by the thread pool
/// (no thread of Oeca's own), posts `WM_APP_ANIM_FRAME`: `SetTimer` and
/// thread-pool timers follow the system timer resolution, which Windows 11
/// may keep at 15.6 ms however a process asks (`timeBeginPeriod`), and even
/// at best `SetTimer` cannot tick faster than 10 ms; a 144 Hz refresh is
/// 6.9 ms. At most one frame message is queued at a time, so a frame that
/// takes longer than a refresh never builds a backlog: frames then simply
/// come less often, each still timed for its own refresh.
struct FrameClock {
    wait: PTP_WAIT,
    /// Stable address handed to the callback; outlives the wait
    target: Box<FrameTarget>,
    /// Current period (0 = stopped)
    period_ms: u32,
}

impl Default for FrameClock {
    fn default() -> Self {
        Self {
            wait: PTP_WAIT::default(),
            target: Box::new(FrameTarget {
                hwnd: std::sync::atomic::AtomicIsize::new(0),
                pending: std::sync::atomic::AtomicU32::new(0),
                running: std::sync::atomic::AtomicBool::new(false),
                timer: HANDLE::default(),
            }),
            period_ms: 0,
        }
    }
}

unsafe extern "system" fn post_animation_frame(
    _: PTP_CALLBACK_INSTANCE,
    context: *mut core::ffi::c_void,
    wait: PTP_WAIT,
    _: u32,
) {
    use std::sync::atomic::Ordering;
    let target = unsafe { &*(context as *const FrameTarget) };
    if !target.running.load(Ordering::Acquire) {
        return;
    }
    // One frame message at a time; one that has gone unanswered for a while
    // (lost somehow) is replaced, so the animation can never stall
    let ticks = target.pending.fetch_add(1, Ordering::AcqRel);
    if ticks == 0 || ticks >= FRAME_RESEND_TICKS {
        target.pending.store(1, Ordering::Release);
        let hwnd = HWND(target.hwnd.load(Ordering::Acquire) as *mut _);
        if unsafe { PostMessageW(Some(hwnd), WM_APP_ANIM_FRAME, WPARAM(0), LPARAM(0)) }.is_err() {
            target.pending.store(0, Ordering::Release);
        }
    }
    // A wait fires once: wait for the timer's next period
    unsafe { SetThreadpoolWait(wait, Some(target.timer), None) };
}

impl FrameClock {
    /// Frames to `hwnd` every `period_ms`; keeps an already running clock at
    /// the same period untouched (a reversal keeps its rhythm). False if the
    /// timer could not be created (the caller falls back to `SetTimer`).
    fn start(&mut self, hwnd: HWND, period_ms: u32) -> bool {
        use std::sync::atomic::Ordering;
        if self.wait.is_invalid() {
            let Ok(timer) = (unsafe {
                CreateWaitableTimerExW(
                    None,
                    windows::core::PCWSTR::null(),
                    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                    TIMER_ALL_ACCESS.0,
                )
            }) else {
                return false;
            };
            self.target.timer = timer;
            let context = &*self.target as *const FrameTarget as *mut core::ffi::c_void;
            match unsafe { CreateThreadpoolWait(Some(post_animation_frame), Some(context), None) } {
                Ok(wait) => self.wait = wait,
                Err(_) => {
                    unsafe {
                        let _ = CloseHandle(timer);
                    }
                    self.target.timer = HANDLE::default();
                    return false;
                }
            }
        }
        if self.period_ms == period_ms {
            return true;
        }
        self.stop();
        self.target.hwnd.store(hwnd.0 as isize, Ordering::Release);
        // First frame one period from now (relative due time, 100 ns units)
        let due = -(i64::from(period_ms) * 10_000);
        if unsafe { SetWaitableTimer(self.target.timer, &due, period_ms as i32, None, None, false) }
            .is_err()
        {
            return false;
        }
        self.target.running.store(true, Ordering::Release);
        unsafe { SetThreadpoolWait(self.wait, Some(self.target.timer), None) };
        self.period_ms = period_ms;
        true
    }

    /// No more frames: stops the timer and waits for a callback in progress.
    /// A frame message already queued is ignored by its handler.
    fn stop(&mut self) {
        if self.wait.is_invalid() || self.period_ms == 0 {
            return;
        }
        self.target
            .running
            .store(false, std::sync::atomic::Ordering::Release);
        unsafe {
            let _ = CancelWaitableTimer(self.target.timer);
            SetThreadpoolWait(self.wait, None, None);
            WaitForThreadpoolWaitCallbacks(self.wait, true);
            // A callback that was running may have waited again
            SetThreadpoolWait(self.wait, None, None);
        }
        self.period_ms = 0;
    }

    /// The queued frame message arrived: the next one may be posted.
    fn frame_received(&self) {
        self.target
            .pending
            .store(0, std::sync::atomic::Ordering::Release);
    }
}

impl Drop for FrameClock {
    fn drop(&mut self) {
        self.stop();
        unsafe {
            if !self.wait.is_invalid() {
                CloseThreadpoolWait(self.wait);
            }
            if !self.target.timer.is_invalid() {
                let _ = CloseHandle(self.target.timer);
            }
        }
    }
}

/// The next display refresh at or after `now`, from DWM's last vblank and
/// refresh period (all in performance-counter ticks).
fn next_vblank(now: i64, vblank: i64, period: i64) -> i64 {
    vblank + ((now - vblank).div_euclid(period) + 1) * period
}

/// Frame-clock period for a display refresh period: one frame per refresh,
/// rounded down so frames never fall behind the display, within 4..=16 ms.
fn frame_period_ms(refresh_ms: f64) -> u32 {
    (refresh_ms.floor() as u32).clamp(4, 16)
}

/// The animation clock, in ms of the performance counter: the time of the
/// first display refresh after `lead_ms` from now (when a frame that takes
/// that long to draw will be seen), or now if DWM's timing is unavailable;
/// and the refresh period, if known.
fn frame_time(lead_ms: f64) -> (f64, Option<f64>) {
    let (mut now, mut freq) = (0i64, 0i64);
    unsafe {
        let _ = QueryPerformanceCounter(&mut now);
        let _ = QueryPerformanceFrequency(&mut freq);
    }
    let to_ms = |ticks: i64| ticks as f64 * 1000.0 / freq.max(1) as f64;
    let mut info = DWM_TIMING_INFO {
        cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32,
        ..Default::default()
    };
    let timing = unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut info) }
        .ok()
        .map(|()| (info.qpcVBlank as i64, info.qpcRefreshPeriod as i64))
        .filter(|&(vblank, period)| vblank > 0 && period > 0);
    match timing {
        Some((vblank, period)) => {
            let lead = (lead_ms * freq as f64 / 1000.0) as i64;
            (
                to_ms(next_vblank(now + lead, vblank, period)),
                Some(to_ms(period)),
            )
        }
        None => (to_ms(now), None),
    }
}

struct WindowState {
    renderer: Renderer,
    /// 1 ms timer resolution, held only during a notch animation that runs on
    /// the `SetTimer` fallback (no `FrameClock`).
    high_res_timer: HighResTimer,
    state: NotchState,
    hovered: bool,
    dimensions: NotchDimensions,
    pos_x: i32,
    pos_y: i32,
    animation: Option<AnimationState>,
    /// Display time (`frame_time`, ms) of the last animation frame drawn.
    last_frame_time: Option<f64>,
    /// Posts the animation frames (`FrameClock`).
    frame_clock: FrameClock,
    /// How long recent animation frames took to draw (ms, a decaying
    /// maximum): frames are timed for the refresh they can make.
    frame_lead_ms: f64,
    clock: ClockEngine,
    media: MediaEngine,
    /// Display model derived from `media` (None = no session).
    media_content: Option<MediaContent>,
    /// Active space (every space renders the existing UI for now). Starts in
    /// Home, never persisted; media updates never touch it. Changed only through
    /// `select_space`, which keeps the renderer's selector in sync.
    space: OecaSpace,
    /// Last tick of the control feedback timer (Some only while it runs).
    feedback_tick: Option<std::time::Instant>,
    /// Last tick of the playback live timer (Some only while it runs), and
    /// its current period.
    live_tick: Option<std::time::Instant>,
    live_ms: u32,
    /// Clipboard history (filled on `WM_CLIPBOARDUPDATE`), saved for this
    /// Windows session only through `clip_store`.
    clipboard: ClipboardHistory,
    clip_store: crate::clipboard_store::ClipboardStore,
    /// The persistent settings file (`settings` is saved to it on change).
    settings_store: crate::settings_store::SettingsStore,
    /// Active OLE drag over the notch (image-file drops into the history).
    drag: DragSession,
    /// The drop being shown (or still decoding) was opened by its drag, so
    /// the notch closes once its result has been seen.
    drop_closes: bool,
    /// User settings: restored from `settings.ini` at launch (defaults for a
    /// new user) and saved through `settings_store` on every change.
    settings: OecaSettings,
    /// The Settings page is open over the active space (which stays selected).
    settings_open: bool,
    /// Outside-click auto-collapse (see `GracePeriod`), and whether the Raw
    /// Input sink that reports outside clicks is registered (expanded only).
    grace: GracePeriod,
    outside_watch: bool,
    /// The Clipboard space asks "Clear clipboard history?" (Cancel / Clear).
    /// Any view change (space, Settings page, collapse) cancels it.
    clear_pending: bool,
    /// Home's countdown timer (memory only), its collapsed presentations
    /// (bubble beside the notch during playback, else the visualizer's
    /// slot), the bubble's window (created on first use, owned by the
    /// notch) and whether it is showing, and a drag along the scale.
    timer: Countdown,
    bubble_presence: Presence,
    slot_presence: Presence,
    bubble: Option<HWND>,
    bubble_shown: bool,
    /// A press / drag on the timer's scale.
    timer_drag: Option<TimerDrag>,
    /// The timer panel's motion (selector, setup scale), on the frames.
    timer_motion: crate::timer::TimerMotion,
    /// The countdown as last shown collapsed (kept while that fades out).
    timer_shown: Countdown,
    /// A drag along the Music scrubber: where it is (0..1 of the track). The
    /// notch holds the mouse capture for the whole gesture.
    scrub: Option<f32>,
    /// The notification-area icon (removed when this state is freed).
    tray: crate::tray::TrayIcon,
    /// The volume HUD, and the master volume (held only while the HUD
    /// shows).
    hud: crate::hud::Hud,
    /// Volume-key momentum (input times only): a rapid held run into
    /// 0% / 100% presses that edge of the notch.
    momentum: crate::hud::Momentum,
    volume: Option<crate::hud::SystemVolume>,
    /// Battery / Energy Saver notifications' registrations.
    saver_watch: [Option<windows::Win32::System::Power::HPOWERNOTIFY>; 2],
    /// The one update state (release, checks, the notice's dismissal), the
    /// notice's presence, the collapsed notch's update icon's presence, the
    /// notice's window (created on first use, owned by the notch), whether
    /// that shows, and where it was last drawn (x, y, dpi, level).
    updates: crate::update::UpdateState,
    update_presence: Presence,
    update_icon: Presence,
    update_notice: Option<HWND>,
    update_shown: bool,
    update_drawn: Option<(i32, i32, u32, f32)>,
}

/// `SetWindowPos` z-order arguments for routine moves and resizes: topmost
/// re-asserts the topmost band; otherwise the z-order is left untouched
/// (`SWP_NOZORDER`), so windows that covered Oeca stay above it.
fn z_order(always_on_top: bool) -> (Option<HWND>, SET_WINDOW_POS_FLAGS) {
    if always_on_top {
        (Some(HWND_TOPMOST), SET_WINDOW_POS_FLAGS(0))
    } else {
        (None, SWP_NOZORDER)
    }
}

/// Where the setting moves the window when it flips (or the notch first
/// shows): into the topmost band; or, off, into the normal band just below
/// the window in use (see `normal_slot`), never above it.
fn z_band(always_on_top: bool) -> HWND {
    if always_on_top {
        HWND_TOPMOST
    } else {
        normal_slot(unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() })
    }
}

/// The foreground window, as far as placing the notch below it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Foreground {
    /// An ordinary (non-topmost) application window, e.g. a game.
    App(HWND),
    /// Nothing to go below: none, the desktop or the taskbar, a topmost
    /// window (inserting after it would make the notch topmost), or Oeca.
    None,
}

/// Where a non-topmost notch goes: right below the foreground app window,
/// so leaving the topmost band (or appearing) never covers what the user is
/// in (`HWND_NOTOPMOST` alone puts it at the top of the normal band, above
/// a fullscreen game until that is activated again); else the top of the
/// normal band.
fn normal_slot(foreground: HWND) -> HWND {
    match classify_foreground(foreground) {
        Foreground::App(app) => app,
        Foreground::None => HWND_NOTOPMOST,
    }
}

fn classify_foreground(hwnd: HWND) -> Foreground {
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetClassNameW, GetShellWindow, GetWindowThreadProcessId,
    };
    if hwnd.is_invalid() || hwnd == unsafe { GetShellWindow() } {
        return Foreground::None;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    let mut class = [0u16; 32];
    let n = unsafe { GetClassNameW(hwnd, &mut class) }.max(0) as usize;
    let shell = matches!(
        String::from_utf16_lossy(&class[..n]).as_str(),
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    );
    foreground_kind(
        pid == std::process::id(),
        ex & WS_EX_TOPMOST.0 != 0,
        shell,
        hwnd,
    )
}

/// `classify_foreground`'s decision, from what it found.
fn foreground_kind(oeca: bool, topmost: bool, shell: bool, hwnd: HWND) -> Foreground {
    if oeca || topmost || shell {
        Foreground::None
    } else {
        Foreground::App(hwnd)
    }
}

impl WindowState {
    /// Media composition of the settled expanded notch, if it is showing.
    fn active_media_layout(&self) -> Option<MediaLayout> {
        if self.state != NotchState::Expanded
            || self.animation.is_some()
            || self.media_content.is_none()
            || self.space == OecaSpace::Clipboard
            || self.settings_open
        {
            return None;
        }
        resolve_media_layout_in(
            &self.dimensions,
            self.media_content.as_ref()?.shape(),
            self.space,
        )
    }

    /// Redraws the settled notch (no-op while animation frames run: the next
    /// one, within a refresh, draws everything).
    fn redraw(&self, hwnd: HWND) {
        if self.animation.is_none() && self.last_frame_time.is_none() {
            let _ = self.renderer.render(
                hwnd,
                self.pos_x,
                self.pos_y,
                &self.dimensions,
                self.hovered,
                self.clock.state(),
            );
        }
    }

    /// Starts the short-lived control feedback timer if a hover/press level needs
    /// to animate. It stops itself once every level reaches its target.
    fn kick_feedback(&mut self, hwnd: HWND) {
        // Reduced motion: hover changes show at once (presses still ease)
        if !self.motion_enabled() {
            self.renderer.feedback().snap_hover();
        }
        let animating = self.renderer.feedback().is_animating()
            || self.renderer.switches_animating()
            || self.renderer.media_motion_animating()
            || self.timer_animating()
            || self.update_presence.is_animating()
            || self.update_icon.is_animating();
        if self.feedback_tick.is_none() && animating {
            self.feedback_tick = Some(std::time::Instant::now());
            unsafe {
                let _ = SetTimer(
                    Some(hwnd),
                    MEDIA_FEEDBACK_TIMER_ID,
                    MEDIA_FEEDBACK_FRAME_MS,
                    None,
                );
            }
        }
    }

    /// Starts the playback live timer (visualizer motion + scrubber position) if
    /// the visualizer needs frames. It stops itself once playback is no longer
    /// playing and the bars have faded out, so paused/no media costs nothing.
    fn kick_live(&mut self, hwnd: HWND) {
        if !self.live_visible() {
            // Nothing live on screen (Home expanded): no frames; settle the fade
            self.renderer.visualizer().settle();
            return;
        }
        let period = live_period_ms(
            self.animation.is_some(),
            self.state,
            self.space.is_music() && !self.settings_open,
            self.settings.show_visualizer,
        );
        let start = self.live_tick.is_none() && self.renderer.visualizer().is_active();
        if start {
            self.live_tick = Some(std::time::Instant::now());
        }
        // Started, or running at the wrong rate (e.g. the visualizer was just
        // hidden / shown, or the notch left / entered Music)
        if start || (self.live_tick.is_some() && self.live_ms != period) {
            self.live_ms = period;
            unsafe {
                let _ = SetTimer(Some(hwnd), MEDIA_LIVE_TIMER_ID, period, None);
            }
        }
    }

    /// Whether a visualizer or scrubber is on screen: the collapsed notch's
    /// visualizer, the Music space, or a notch animation (its frames show both
    /// ends). The settled Home expanded notch shows neither.
    fn live_visible(&self) -> bool {
        self.animation.is_some()
            || self.state == NotchState::Collapsed
            || (self.space.is_music() && !self.settings_open)
    }

    /// Drops all control feedback immediately (e.g. when the notch toggles).
    fn reset_feedback(&mut self, hwnd: HWND) {
        self.renderer.feedback().reset();
        if self.feedback_tick.take().is_some() {
            unsafe {
                let _ = KillTimer(Some(hwnd), MEDIA_FEEDBACK_TIMER_ID);
            }
        }
        // The timer's presentations and the Settings switches share that
        // timer: whatever of them still moves keeps its frames
        self.kick_feedback(hwnd);
    }

    /// Space capsule under a client-area point: on the expanded notch, settled
    /// or mid width transition (the selector is drawn from the same interpolated
    /// dimensions, so what is clicked is what is seen). Not during expand/collapse.
    fn space_at(&self, lparam: LPARAM) -> Option<OecaSpace> {
        let (selector, x, y) = self.header_at(lparam)?;
        selector.space_at(x, y)
    }

    /// Whether a client-area point is on the header's Settings icon, settled
    /// or mid transition (like the pills: where it is drawn this frame).
    fn settings_icon_at(&self, lparam: LPARAM) -> bool {
        self.header_at(lparam)
            .is_some_and(|(selector, x, y)| selector.settings.contains(x, y))
    }

    /// The header (pills and Settings icon) as drawn this frame, and the
    /// client point: on the expanded notch, settled or mid space transition
    /// (gliding), not during expand/collapse or on the drop page.
    fn header_at(&self, lparam: LPARAM) -> Option<(SpaceSelectorLayout, f32, f32)> {
        let width_only = self
            .animation
            .as_ref()
            .is_none_or(|a| a.is_space_transition());
        // Nor while the volume HUD covers the header
        if self.state != NotchState::Expanded
            || !width_only
            || self.renderer.drop_page()
            || self.hud.is_shown()
        {
            return None;
        }
        let x = self.client_x(lparam);
        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
        let (from, to, mix) = match &self.animation {
            Some(anim) => {
                let t = anim.transition();
                (t.from, t.to, t.mix)
            }
            None => (self.scene(), self.scene(), 1.0),
        };
        let (selector, _) = blended_selector(&self.dimensions, from, to, self.space, mix)?;
        Some((selector, x, y))
    }

    /// Selects a space (the selector's only route to `OecaSpace::switch_to`); a
    /// no-op when already active. On the settled expanded notch the width
    /// transitions to the space's width on the existing animation timer; while
    /// collapsed (or mid expand/collapse) the space is only stored, and the next
    /// expansion opens straight into its width.
    fn select_space(&mut self, hwnd: HWND, space: OecaSpace) {
        self.end_scrub(hwnd, false);
        let before = self.scene();
        let changed = self.space.switch_to(space);
        // A pill on the Settings page also closes it (back to that space)
        let closed = std::mem::take(&mut self.settings_open);
        if changed || closed {
            self.set_clear_pending(hwnd, false);
            self.renderer.set_space(space);
            self.renderer.set_settings(false, self.settings);
            self.kick_live(hwnd);
            // Controls move with the layout: drop stale hover/press
            self.reset_feedback(hwnd);
            self.resize_to_target(hwnd, before);
            notify_accessibility_state_changed(
                hwnd,
                self.state,
                self.clock.state(),
                self.media_content.as_ref(),
                self.space,
                &self.clipboard,
                self.clear_pending,
                self.settings_title(),
                self.timer_title().as_deref(),
            );
        }
    }

    /// The space whose expanded size the notch takes: the active space's, or
    /// for Settings and the drop page the panel (Clipboard) size, whichever
    /// space is active.
    fn size_space(&self) -> OecaSpace {
        self.scene().size_space()
    }

    /// Animate, unless the user (Reduced motion) or the system asked not to.
    fn motion_enabled(&self) -> bool {
        motion_allowed(self.settings.reduced_motion, is_client_animation_enabled())
    }

    /// What the expanded notch shows: the drop page during an image drag,
    /// otherwise the active space.
    fn scene(&self) -> Scene {
        Scene::current(self.renderer.drop_state(), self.settings_open, self.space)
    }

    /// The Settings page's accessible state (Some(settings) while open).
    fn settings_title(&self) -> Option<OecaSettings> {
        self.settings_open.then_some(self.settings)
    }

    /// Flips Reduced motion: from now on view changes snap (the existing
    /// reduced-motion path) instead of animating.
    fn toggle_reduced_motion(&mut self, hwnd: HWND) {
        self.settings.reduced_motion = !self.settings.reduced_motion;
        self.renderer
            .set_settings(self.settings_open, self.settings);
        settings_window::sync(self.settings);
        self.persist_settings();
        // Timer presentations snap (and the done pulse stops) from now on
        self.refresh_timer_presence(hwnd);
        // The page's switch slides (control-feedback timer)
        self.kick_feedback(hwnd);
        self.redraw(hwnd);
        notify_accessibility_state_changed(
            hwnd,
            self.state,
            self.clock.state(),
            self.media_content.as_ref(),
            self.space,
            &self.clipboard,
            self.clear_pending,
            self.settings_title(),
            self.timer_title().as_deref(),
        );
    }

    /// Saves the settings after a change (from the notch page or the Settings
    /// window). A failure keeps the change for this run only; the next change
    /// tries again. Nothing is logged but the error kind.
    fn persist_settings(&mut self) {
        let _saved = self.settings_store.save(&self.settings);
        #[cfg(debug_assertions)]
        if let Err(e) = &_saved {
            eprintln!("[settings] not saved: {:?}", e.kind());
        }
    }

    /// Start on launch flipped in the Settings window: registers or removes
    /// the per-user Run entry, then shows what actually happened (a failed
    /// registration leaves the switch where it was).
    fn toggle_start_on_launch(&mut self) {
        let want = !self.settings.start_on_launch;
        self.settings.start_on_launch = crate::startup::set_enabled(want);
        settings_window::sync(self.settings);
        self.persist_settings();
    }

    /// "Open Full Settings": opens (or focuses) the native Settings window.
    /// The notch and its Settings page stay as they are.
    fn open_full_settings(&mut self, hwnd: HWND) {
        // A failure leaves the notch untouched (nothing to show)
        let _ = settings_window::open(hwnd, self.settings);
        // An update found before Settings opened shows there too
        settings_window::sync_update(self.updates.status());
    }

    /// Tray "Open Oeca" (or a left click on the icon): the notch shown and
    /// expanded through the usual spring, keeping its space, media and timer.
    /// Already open or opening: nothing changes.
    fn open_from_tray(&mut self, hwnd: HWND) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        if !self.expanded_or_expanding() {
            self.reset_feedback(hwnd);
            let _ = start_or_reverse_animation(hwnd, self);
        }
    }

    /// A switch in the Settings window: the same change as on the notch page.
    fn apply_settings_window_toggle(&mut self, hwnd: HWND, control: settings_window::Control) {
        match control {
            settings_window::Control::AlwaysOnTop => self.toggle_always_on_top(hwnd),
            settings_window::Control::ReducedMotion => self.toggle_reduced_motion(hwnd),
            // Read where they apply (the Settings window's Escape, the
            // Clipboard space's Clear): nothing on the notch changes now
            settings_window::Control::CloseOnEscape
            | settings_window::Control::ConfirmClearClipboard => {
                control.toggle(&mut self.settings);
                self.renderer
                    .set_settings(self.settings_open, self.settings);
                settings_window::sync(self.settings);
                self.persist_settings();
            }
            settings_window::Control::StartOnLaunch => self.toggle_start_on_launch(),
            settings_window::Control::CheckForUpdates => self.toggle_update_checks(hwnd),
            // Presentation only: redraw; the live timer follows the
            // visualizer (fast with bars, slow for a lone scrubber)
            settings_window::Control::ShowSourceApp | settings_window::Control::ShowVisualizer => {
                control.toggle(&mut self.settings);
                self.renderer
                    .set_settings(self.settings_open, self.settings);
                settings_window::sync(self.settings);
                self.persist_settings();
                self.redraw(hwnd);
                self.kick_live(hwnd);
            }
        }
    }

    /// The opt-in daily update check: turned on, a check runs now and then
    /// once a day; turned off, no further automatic check (checking by hand
    /// in Settings always works, and a release already found stays).
    fn toggle_update_checks(&mut self, hwnd: HWND) {
        settings_window::Control::CheckForUpdates.toggle(&mut self.settings);
        self.renderer
            .set_settings(self.settings_open, self.settings);
        settings_window::sync(self.settings);
        self.persist_settings();
        unsafe {
            if self.settings.check_for_updates {
                let _ = SetTimer(
                    Some(hwnd),
                    crate::update::CHECK_TIMER_ID,
                    crate::update::RECHECK_MS,
                    None,
                );
            } else {
                let _ = KillTimer(Some(hwnd), crate::update::CHECK_TIMER_ID);
            }
        }
        if self.settings.check_for_updates {
            self.check_for_updates(hwnd);
        }
    }

    /// Uses an accent: the notch redraws with it (switches, and the playback
    /// accent when the artwork has none) and the Settings window follows.
    /// Geometry, motion and the black surface are untouched.
    fn set_accent(&mut self, hwnd: HWND, accent: crate::config::AccentChoice) {
        if std::mem::replace(&mut self.settings.accent, accent) != accent {
            self.renderer
                .set_settings(self.settings_open, self.settings);
            settings_window::sync(self.settings);
            self.persist_settings();
            self.redraw(hwnd);
        }
    }

    /// Uses a history capacity: a smaller one drops the oldest entries at
    /// once (the Clipboard space and its accessible title follow). The system
    /// clipboard is untouched.
    fn set_clipboard_capacity(&mut self, hwnd: HWND, capacity: crate::config::ClipboardCapacity) {
        self.settings.clipboard_capacity = capacity;
        let trimmed = self.clipboard.set_capacity(capacity);
        self.renderer
            .set_settings(self.settings_open, self.settings);
        settings_window::sync(self.settings);
        self.persist_settings();
        if trimmed {
            self.clipboard_changed(hwnd);
        }
    }

    /// Feeds the outside-click grace period and carries out its decision:
    /// (re)start or stop the one-shot timer, or collapse through the normal
    /// (Phase 6.1) path.
    fn grace(&mut self, hwnd: HWND, event: GraceEvent) {
        let at_once = self.settings.collapse_delay.ms() == 0;
        match self
            .grace
            .handle(event, self.expanded_or_expanding(), at_once)
        {
            GraceAction::None => {}
            GraceAction::Arm => unsafe {
                // Same id: re-arming restarts the countdown, for the
                // currently selected duration
                let ms = self.settings.collapse_delay.ms();
                let _ = SetTimer(Some(hwnd), AUTO_COLLAPSE_TIMER_ID, ms, None);
            },
            GraceAction::Disarm => unsafe {
                let _ = KillTimer(Some(hwnd), AUTO_COLLAPSE_TIMER_ID);
            },
            GraceAction::Collapse => {
                unsafe {
                    let _ = KillTimer(Some(hwnd), AUTO_COLLAPSE_TIMER_ID);
                }
                self.reset_feedback(hwnd);
                let _ = start_or_reverse_animation(hwnd, self);
            }
        }
    }

    /// Registers / removes the Raw Input sink that reports mouse buttons
    /// anywhere (the notch never activates, so it is not told otherwise):
    /// only while the notch is (becoming) expanded.
    fn set_outside_watch(&mut self, hwnd: HWND, on: bool) {
        if self.outside_watch != on && watch_outside_clicks(hwnd, on) {
            self.outside_watch = on;
        }
    }

    /// Uses a notch closing delay: a countdown already running restarts with
    /// it; later outside clicks use it too.
    fn set_collapse_delay(&mut self, hwnd: HWND, delay: crate::config::CollapseDelay) {
        if std::mem::replace(&mut self.settings.collapse_delay, delay) != delay {
            self.renderer
                .set_settings(self.settings_open, self.settings);
            settings_window::sync(self.settings);
            self.persist_settings();
            self.grace(hwnd, GraceEvent::DelayChanged);
        }
    }

    /// Shows / dismisses the Clipboard space's clear confirmation (the caller
    /// redraws). Its buttons replace the rows: drop stale hover / press.
    fn set_clear_pending(&mut self, hwnd: HWND, pending: bool) {
        if std::mem::replace(&mut self.clear_pending, pending) != pending {
            self.renderer.set_clear_pending(pending);
            self.reset_feedback(hwnd);
        }
    }

    /// Opens / closes the Settings page over the active space: the content
    /// blends on the existing space transition (same size), the selected
    /// space, media and clipboard history are untouched.
    fn set_settings_open(&mut self, hwnd: HWND, open: bool) {
        if self.settings_open == open {
            return;
        }
        self.end_scrub(hwnd, false);
        let before = self.scene();
        self.set_clear_pending(hwnd, false);
        self.settings_open = open;
        self.renderer.set_settings(open, self.settings);
        self.resize_to_target(hwnd, before);
        self.kick_live(hwnd);
        notify_accessibility_state_changed(
            hwnd,
            self.state,
            self.clock.state(),
            self.media_content.as_ref(),
            self.space,
            &self.clipboard,
            self.clear_pending,
            self.settings_title(),
            self.timer_title().as_deref(),
        );
    }

    /// Flips Always on top and applies it to this window at once: into or
    /// out of the topmost band, keeping position, size, visibility and
    /// activation (no window is recreated).
    fn toggle_always_on_top(&mut self, hwnd: HWND) {
        let on = !self.settings.always_on_top;
        self.settings.always_on_top = on;
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                Some(z_band(on)),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            // The countdown bubble shares the notch's band
            if let Some(bubble) = self.bubble {
                let _ = SetWindowPos(
                    bubble,
                    Some(z_band(on)),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
        self.renderer
            .set_settings(self.settings_open, self.settings);
        settings_window::sync(self.settings);
        self.persist_settings();
        // The page's switch slides (control-feedback timer)
        self.kick_feedback(hwnd);
        self.redraw(hwnd);
        notify_accessibility_state_changed(
            hwnd,
            self.state,
            self.clock.state(),
            self.media_content.as_ref(),
            self.space,
            &self.clipboard,
            self.clear_pending,
            self.settings_title(),
            self.timer_title().as_deref(),
        );
    }

    /// After the space or drop page changed (from showing `before`): the notch
    /// moves to its new size and content. A running animation (any kind,
    /// including an expand still opening) is retargeted in place, keeping its
    /// position and velocity; the settled expanded notch starts a space
    /// transition; collapsed, the change is only stored.
    fn resize_to_target(&mut self, hwnd: HWND, before: Scene) {
        let to = expanded_size_dip(self.size_space(), self.dimensions.dpi);
        let scene = self.scene();
        if let Some(anim) = &mut self.animation {
            if anim.target_state == NotchState::Expanded {
                anim.retarget(NotchState::Expanded, to);
            }
            anim.retarget_scene(scene);
            return;
        }
        if self.state != NotchState::Expanded {
            self.redraw(hwnd);
            return;
        }
        let from = (
            self.dimensions.width as f32 / self.dimensions.scale,
            self.dimensions.height as f32 / self.dimensions.scale,
        );
        if from == to && before == scene {
            self.redraw(hwnd);
            return;
        }
        if !self.motion_enabled() {
            let dims =
                space_dimensions(NotchState::Expanded, self.dimensions.dpi, self.size_space());
            let screen_width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
            self.dimensions = dims;
            self.pos_x = calculate_notch_x(screen_width, dims.width);
            self.redraw(hwnd);
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    z_order(self.settings.always_on_top).0,
                    self.pos_x,
                    self.pos_y,
                    dims.width,
                    dims.height,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW | z_order(self.settings.always_on_top).1,
                );
            }
            return;
        }
        let mut anim = AnimationState::space(from, to, before, scene);
        anim.scale = self.dimensions.scale;
        self.animation = Some(anim);
        self.start_frames(hwnd);
    }

    /// Starts (or keeps running) the animation frames, timed from now: frames
    /// at the display's refresh rate (`FrameClock`), or, if no frame clock is
    /// available, the 12 ms timer with 1 ms timer resolution.
    fn start_frames(&mut self, hwnd: HWND) {
        let (now, refresh) = frame_time(self.frame_lead_ms);
        self.last_frame_time = Some(now);
        let period = refresh.map_or(ANIMATION_FRAME_INTERVAL_MS, frame_period_ms);
        if !self.frame_clock.start(hwnd, period) {
            self.high_res_timer.set(true);
            unsafe {
                SetTimer(
                    Some(hwnd),
                    ANIMATION_TIMER_ID,
                    ANIMATION_FRAME_INTERVAL_MS,
                    None,
                );
            }
        }
    }

    /// Stops the animation frames and releases the timer resolution.
    fn stop_frames(&mut self, hwnd: HWND) {
        self.frame_clock.stop();
        unsafe {
            let _ = KillTimer(Some(hwnd), ANIMATION_TIMER_ID);
        }
        self.high_res_timer.set(false);
        self.last_frame_time = None;
    }

    /// Places history entry `index` back on the system clipboard and makes it
    /// the newest entry. The update Windows then sends is Oeca's own (owner
    /// check), so no duplicate is stored. Failures leave everything unchanged.
    fn restore_clipboard(
        &mut self,
        hwnd: HWND,
        index: usize,
    ) -> std::result::Result<(), clipboard::ClipboardError> {
        let item = self
            .clipboard
            .get(index)
            .cloned()
            .ok_or(clipboard::ClipboardError::Invalid)?;
        clipboard::write(hwnd, &item)?;
        self.clipboard.promote(index);
        Ok(())
    }

    /// Expanded, or animating toward expanded.
    fn expanded_or_expanding(&self) -> bool {
        self.animation
            .as_ref()
            .map_or(self.state == NotchState::Expanded, |a| {
                a.target_state == NotchState::Expanded
            })
    }

    /// Whether a screen point is in the drop zone: the drop page's own settled
    /// silhouette (centred at the top of the screen). Fixed whatever size the
    /// notch has right now, so the page opening/resizing under the pointer can
    /// never flip the answer (no flicker at the edges).
    fn screen_point_in_drop_zone(&self, at: POINT) -> bool {
        let zone = space_dimensions(NotchState::Expanded, self.dimensions.dpi, PANEL_SPACE);
        let left = calculate_notch_x(unsafe { GetSystemMetrics(SM_CXSCREEN) }, zone.width);
        zone.contains_point((at.x - left) as f32, (at.y - self.pos_y) as f32)
    }

    /// One OLE drag event (UI thread). Anything dragged over the notch shows
    /// the drop page (files that are not images, with its hint); supported
    /// images are accepted. The first such moment expands a collapsed notch
    /// through the existing animation; leave/cancel close it again only if
    /// the drag opened it. A drop keeps the page up while its files decode
    /// off the UI thread (`finish_drop` then shows the result).
    fn on_drag(&mut self, hwnd: HWND, event: DragEvent) -> bool {
        use dragdrop::DropPage;
        let over = |state: &mut Self, at: POINT| {
            let in_notch = state.screen_point_in_drop_zone(at);
            let expanded = state.expanded_or_expanding();
            let before = state.scene();
            let action = state.drag.over(in_notch, expanded);
            let page = match (action.show, action.accept) {
                (true, true) => DropPage::Ready,
                (true, false) => DropPage::Unsupported,
                _ => DropPage::Off,
            };
            // The drop page at its own universal size (an expansion it
            // starts opens straight at it)
            if state.renderer.set_drop(page) && !action.expand {
                state.resize_to_target(hwnd, before);
            }
            if action.expand {
                state.reset_feedback(hwnd);
                let _ = start_or_reverse_animation(hwnd, state);
            }
            action.accept
        };
        let end = |state: &mut Self| {
            let collapse = state.drag.end() && state.expanded_or_expanding();
            let before = state.scene();
            let changed = state.renderer.set_drop(DropPage::Off);
            if collapse {
                state.reset_feedback(hwnd);
                let _ = start_or_reverse_animation(hwnd, state);
            } else if changed {
                // Back to the active space's own size and content
                state.resize_to_target(hwnd, before);
            }
        };
        match event {
            DragEvent::Enter { images, at } => {
                self.grace(hwnd, GraceEvent::OecaInteraction);
                // A drop result still on show gives way to the new drag,
                // which then closes the notch the earlier drag opened
                let opened = self.stop_drop_result(hwnd);
                self.drag.enter(images, opened);
                over(self, at)
            }
            DragEvent::Over { at } => over(self, at),
            DragEvent::Leave => {
                end(self);
                false
            }
            DragEvent::Drop { images, at } => {
                if !(over(self, at) && !images.is_empty()) {
                    end(self);
                    return false;
                }
                // The page stays up until the result replaces it; the
                // expansion this drag made is closed after the result
                self.drop_closes = self.drag.end() && self.expanded_or_expanding();
                if !dragdrop::decode_async(hwnd, WM_APP_DROP_DECODED, images.clone()) {
                    // No worker thread: decode here (this thread has COM)
                    self.finish_drop(hwnd, dragdrop::decode_all(&images));
                }
                true
            }
        }
    }

    /// A drop's files are decoded: adds the images to the history (all its
    /// limits apply), places the newest on the system clipboard (its own
    /// update is then skipped by the owner check), and shows the outcome on
    /// the drop page: "Item copied to clipboard" only if both worked.
    fn finish_drop(&mut self, hwnd: HWND, decoded: dragdrop::Decoded) {
        use dragdrop::DropOutcome;
        let mut outcome = DropOutcome::of(decoded.images.len(), decoded.error);
        if dragdrop::ingest(&mut self.clipboard, decoded.images) {
            if self.restore_clipboard(hwnd, 0).is_err() {
                outcome = DropOutcome::ClipboardBusy;
            }
            self.clipboard_changed(hwnd);
        } else if outcome.succeeded() {
            // Decoded, but over the history's image budget
            outcome = DropOutcome::TooLarge;
        }
        #[cfg(debug_assertions)]
        eprintln!(
            "[dragdrop] drop finished: {outcome:?}, {} entries",
            self.clipboard.len()
        );
        // A new drag took over, or the notch closed meanwhile: the history
        // has the images, there is nothing left to show them on
        if self.drag.is_active() || !self.expanded_or_expanding() {
            self.drop_closes = false;
            return;
        }
        let before = self.scene();
        if self.renderer.set_drop_result(outcome) {
            self.resize_to_target(hwnd, before);
        }
        unsafe {
            let _ = SetTimer(Some(hwnd), DROP_RESULT_TIMER_ID, DROP_RESULT_MS, None);
        }
    }

    /// The drop result has been on show long enough: back to the active
    /// space, or closed if its drag opened the notch.
    fn end_drop_result(&mut self, hwnd: HWND) {
        let close = self.stop_drop_result(hwnd);
        if self.renderer.drop_state() != dragdrop::DropPage::Result {
            return;
        }
        let before = self.scene();
        self.renderer.set_drop(dragdrop::DropPage::Off);
        if close && self.expanded_or_expanding() {
            self.reset_feedback(hwnd);
            let _ = start_or_reverse_animation(hwnd, self);
        } else {
            self.resize_to_target(hwnd, before);
        }
    }

    /// Stops a drop result's countdown; true if the notch was to close after
    /// it (the caller takes that over). The result itself stays on show.
    fn stop_drop_result(&mut self, hwnd: HWND) -> bool {
        unsafe {
            let _ = KillTimer(Some(hwnd), DROP_RESULT_TIMER_ID);
        }
        std::mem::take(&mut self.drop_closes)
    }

    /// Panel control under a client-area point on the settled expanded notch:
    /// the header's Settings icon; on the Settings page its switch; in the
    /// Clipboard space a row, its buttons or Clear. Everything else is notch
    /// background (the selector pills are `space_at`).
    fn panel_hit(&self, lparam: LPARAM) -> Option<PanelHit> {
        // The Settings icon works mid transition too (reverses it)
        if self.settings_icon_at(lparam) {
            return Some(PanelHit::Settings);
        }
        if self.state != NotchState::Expanded
            || self.animation.is_some()
            || self.renderer.drop_page()
        {
            return None;
        }
        let x = self.client_x(lparam);
        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
        if self.settings_open {
            // Only the switches and the action row are live on the Settings page
            let l = resolve_settings_layout(&self.dimensions)?;
            return [
                (l.always_on_top.toggle, PanelHit::AlwaysOnTop),
                (l.reduced_motion.toggle, PanelHit::ReducedMotion),
                (l.full_settings, PanelHit::FullSettings),
            ]
            .into_iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, hit)| hit);
        }
        if self.space != OecaSpace::Clipboard {
            // Home: the timer's mode pills, its buttons and its scale
            let idle = self.timer.phase() == Phase::Idle;
            return self.timer_layout()?.hit(x, y, idle);
        }
        let l = resolve_clipboard_layout(&self.dimensions)?;
        if self.clear_pending {
            // Only Cancel and Clear while the confirmation shows
            return l.confirm_hit(x, y);
        }
        l.hit(x, y, self.clipboard.len())
    }

    /// After any history change: rebuild the Clipboard rows, and if that space
    /// is on screen, redraw it and refresh its accessible title.
    fn clipboard_changed(&mut self, hwnd: HWND) {
        // Saved at once (a crash loses nothing); a failure only means this
        // change is not kept across a relaunch. Contents are never logged.
        let _saved = self.clip_store.save(&self.clipboard);
        #[cfg(debug_assertions)]
        if let Err(e) = &_saved {
            eprintln!("[clipboard] history not saved: {:?}", e.kind());
        }
        self.renderer.set_clipboard(&self.clipboard);
        if self.state == NotchState::Expanded && self.space == OecaSpace::Clipboard {
            self.redraw(hwnd);
            notify_accessibility_state_changed(
                hwnd,
                self.state,
                self.clock.state(),
                self.media_content.as_ref(),
                self.space,
                &self.clipboard,
                self.clear_pending,
                self.settings_title(),
                self.timer_title().as_deref(),
            );
        }
    }

    /// A transport command was sent: its glyph answers at once (play/pause
    /// heads for the other glyph; a skip rolls its way).
    fn control_clicked(&mut self, hwnd: HWND, control: MediaControl) {
        let instant = !self.motion_enabled();
        match control {
            MediaControl::PlayPause => self
                .renderer
                .icons()
                .clicked_play_pause(std::time::Instant::now(), instant),
            MediaControl::Previous | MediaControl::Next => {
                self.renderer.icons().clicked_skip(control, instant);
            }
        }
        self.kick_feedback(hwnd);
    }

    /// The Music scrubber's track on the settled expanded notch, while the
    /// session has a timeline (None elsewhere).
    fn scrub_track(&self) -> Option<crate::layout::RectF> {
        if self.space != OecaSpace::Music {
            return None;
        }
        self.media_content.as_ref()?.timeline?;
        self.active_media_layout()?
            .scrub_track(self.dimensions.scale)
    }

    /// The point on the scrubber's track (fraction), if a press there would
    /// scrub.
    fn scrub_at(&self, lparam: LPARAM) -> Option<f32> {
        let x = self.client_x(lparam);
        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
        let track = self.scrub_track()?;
        track
            .contains(x, y)
            .then(|| MediaLayout::scrub_fraction(track, x))
    }

    /// Starts a scrub (only if the app allows seeking): the notch takes the
    /// mouse capture so the whole drag, wherever it goes, stays this gesture.
    fn begin_scrub(&mut self, hwnd: HWND, fraction: f32) {
        if !self.media.can_seek() {
            return;
        }
        self.scrub = Some(fraction);
        self.renderer.set_scrub(self.scrub);
        unsafe {
            SetCapture(hwnd);
        }
        self.redraw(hwnd);
    }

    /// The scrub follows the pointer (clamped to the track's ends).
    fn move_scrub(&mut self, hwnd: HWND, lparam: LPARAM) {
        let Some(track) = self.scrub_track() else {
            self.end_scrub(hwnd, false);
            return;
        };
        let x = self.client_x(lparam);
        let f = MediaLayout::scrub_fraction(track, x);
        if self.scrub != Some(f) {
            self.scrub = Some(f);
            self.renderer.set_scrub(self.scrub);
            self.redraw(hwnd);
        }
    }

    /// Ends a scrub: on release (`commit`) the session is asked to play from
    /// there and the bar holds that spot until it reports back; cancelled
    /// (capture lost, the notch or media changed) nothing is sought.
    fn end_scrub(&mut self, hwnd: HWND, commit: bool) {
        let Some(f) = self.scrub.take() else {
            return;
        };
        self.renderer.set_scrub(None);
        let duration = self
            .media_content
            .as_ref()
            .and_then(|c| c.timeline)
            .map(|t| t.duration_ms);
        if let (true, Some(duration)) = (commit, duration) {
            let position = scrub_position(f, duration);
            if self.media.send_seek(position) {
                self.renderer.hold_seek(position);
            }
        }
        unsafe {
            if GetCapture() == hwnd {
                let _ = ReleaseCapture();
            }
        }
        self.redraw(hwnd);
    }

    /// Home's timer panel on the settled expanded notch (None elsewhere: the
    /// timer is only operable there).
    fn timer_layout(&self) -> Option<crate::layout::TimerLayout> {
        if self.state != NotchState::Expanded
            || self.animation.is_some()
            || self.space != OecaSpace::Home
            || self.settings_open
            || self.renderer.drop_page()
        {
            return None;
        }
        crate::layout::resolve_home_timer_layout(
            &self.dimensions,
            self.media_content.as_ref().map(MediaContent::shape),
        )
        .map(|h| h.timer)
    }

    /// The finished countdown's gentle pulse still needs frames: only while
    /// it is on screen (Home, the bubble or the slot), never under Reduced
    /// motion.
    fn timer_pulsing(&self) -> bool {
        let home = self.state == NotchState::Expanded
            && self.space == OecaSpace::Home
            && !self.settings_open;
        let on_screen =
            home || self.bubble_presence.level() > 0.0 || self.slot_presence.level() > 0.0;
        on_screen
            && self.motion_enabled()
            && self
                .timer
                .done_for(std::time::Instant::now())
                .is_some_and(crate::timer::pulsing)
    }

    /// Something of the timer still moves (bubble, slot, done pulse).
    fn timer_animating(&self) -> bool {
        self.bubble_presence.is_animating()
            || self.slot_presence.is_animating()
            || self.timer_pulsing()
    }

    /// Mirrors the countdown (and its slot / pulse) into the renderer; the
    /// collapsed presentations keep the countdown they last showed.
    fn push_timer_view(&mut self) {
        if self.timer.shows_collapsed() {
            self.timer_shown = self.timer;
        }
        let pulse = if self.motion_enabled() {
            self.timer
                .done_for(std::time::Instant::now())
                .map_or(1.0, crate::timer::done_pulse)
        } else {
            1.0
        };
        self.renderer.set_timer(crate::renderer::TimerView {
            countdown: self.timer,
            collapsed: self.timer_shown,
            slot: self.slot_presence.level(),
            pulse,
            mode: self.timer_motion.mode(),
            scale: self.timer_motion.scale(),
            lag: self.timer_motion.lag(),
        });
    }

    /// The scale's value on show now (minutes): the setup's gliding value,
    /// else what is left.
    fn shown_scale(&self) -> f32 {
        if self.timer.phase() == Phase::Idle {
            self.timer_motion.scale()
        } else {
            crate::timer::scale_value(&self.timer, std::time::Instant::now())
                + self.timer_motion.lag()
        }
    }

    /// The timer panel's motion follows the countdown (selector, setup
    /// scale, gliding from `from`) on the notch's frames, started if idle;
    /// under no motion it is simply drawn.
    fn aim_timer_motion(&mut self, hwnd: HWND, from: f32) {
        let instant = !self.motion_enabled();
        self.timer_motion
            .aim(&self.timer, Some(from), std::time::Instant::now(), instant);
        self.push_timer_view();
        if self.timer_motion.is_animating() {
            if self.last_frame_time.is_none() {
                self.start_frames(hwnd);
            }
        } else {
            self.redraw(hwnd);
        }
    }

    /// Saves the Focus / Break durations (settings) after they changed.
    fn persist_timer_durations(&mut self) {
        use crate::timer::Mode;
        let (focus, rest) = (
            self.timer.secs_for(Mode::Focus),
            self.timer.secs_for(Mode::Break),
        );
        if (self.settings.focus_secs, self.settings.break_secs) != (focus, rest) {
            self.settings.focus_secs = focus;
            self.settings.break_secs = rest;
            self.persist_settings();
        }
    }

    /// Whether a client point is on Home's timer panel (a click there is
    /// the timer's, never the notch background's).
    fn on_timer_panel(&self, lparam: LPARAM) -> bool {
        let x = self.client_x(lparam);
        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
        self.timer_layout().is_some_and(|l| l.panel.contains(x, y))
    }

    /// Aims the collapsed presentations at what the countdown, the notch and
    /// playback now call for (bubble during playback, else the slot; only
    /// while collapsed, i.e. once a collapse's own content fades in), and
    /// starts frames if they move. Cheap: called on every notch frame.
    fn refresh_timer_presence(&mut self, hwnd: HWND) {
        let collapsed = match &self.animation {
            None => self.state == NotchState::Collapsed,
            Some(a) => a.target_state == NotchState::Collapsed && a.collapsed_alpha > 0.0,
        };
        let playing = self
            .media_content
            .as_ref()
            .is_some_and(|c| crate::media::shows_visualizer(c.playback));
        let running = matches!(self.timer.phase(), Phase::Running { .. });
        let (bubble, slot) = crate::timer::collapsed_targets(
            running,
            self.timer.shows_collapsed(),
            collapsed,
            playing,
        );
        let instant = !self.motion_enabled();
        self.bubble_presence.set(bubble, instant);
        self.slot_presence.set(slot, instant);
        // Update: the notice under the collapsed notch first; once it is
        // dismissed, the icon in the artwork's place (never both)
        let (notice, icon) = update_targets(&self.updates, collapsed);
        self.update_presence.set(notice, instant);
        self.update_icon.set(icon, instant);
        self.renderer.set_update_icon(self.update_icon.level());
        self.push_timer_view();
        self.kick_feedback(hwnd);
        self.update_bubble(hwnd);
        self.draw_update_notice(hwnd);
    }

    /// Starts an update check (scheduled or from Settings) unless one is
    /// running already (its result answers both), off the UI thread.
    fn check_for_updates(&mut self, hwnd: HWND) {
        if let Some(id) = self.updates.begin()
            && !crate::update::check_async(hwnd, WM_APP_UPDATE_READY, id)
        {
            self.updates.finish(id, crate::update::Outcome::Failed);
        }
        settings_window::sync_update(self.updates.status());
    }

    /// An update check's outcome, into the shared state (a stale one changes
    /// nothing); the notice, the icon and Settings follow it.
    fn update_ready(&mut self, hwnd: HWND, id: usize, outcome: crate::update::Outcome) {
        if !self.updates.finish(id, outcome) {
            return;
        }
        // A different release: drawn and named afresh
        self.update_drawn = None;
        if let (Some(notice), Some(r)) = (self.update_notice, self.updates.release()) {
            let title: Vec<u16> = r.title().encode_utf16().chain([0]).collect();
            unsafe {
                let _ = SetWindowTextW(notice, windows::core::PCWSTR(title.as_ptr()));
            }
        }
        settings_window::sync_update(self.updates.status());
        self.refresh_timer_presence(hwnd);
    }

    /// Opens the newest validated release page (the icon, the notice and
    /// Settings all open this one).
    fn open_update(&self) {
        if let Some(release) = self.updates.release() {
            crate::update::open_release(release);
        }
    }

    /// The update notice was clicked (`open`: the release page in the
    /// browser), right-clicked, or has been on show long enough: it leaves
    /// and is not offered again for this version. The release stays
    /// available: the icon takes the notice's place.
    fn dismiss_update(&mut self, hwnd: HWND, open: bool) {
        if open {
            self.open_update();
        }
        self.updates.dismiss_notice();
        unsafe {
            let _ = KillTimer(Some(hwnd), crate::update::HIDE_TIMER_ID);
        }
        self.refresh_timer_presence(hwnd);
    }

    /// The collapsed notch's update icon is showing and under the point.
    fn update_icon_at(&self, lparam: LPARAM) -> bool {
        if self.state != NotchState::Collapsed
            || self.animation.is_some()
            || self.update_icon.level() < 0.5
        {
            return false;
        }
        let x = self.client_x(lparam);
        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
        crate::layout::resolve_collapsed_layout(&self.dimensions)
            .artwork_bounds
            .contains(x, y)
    }

    /// Draws the update notice centred under the notch at its presence (shown
    /// on first need, hidden once fully gone; redrawn only when it moves).
    /// Its time on show starts when it appears.
    fn draw_update_notice(&mut self, hwnd: HWND) {
        let p = self.update_presence.level();
        let Some(release) = self.updates.release() else {
            return;
        };
        if p <= 0.0 && !self.update_presence.is_animating() {
            self.update_drawn = None;
            if std::mem::take(&mut self.update_shown)
                && let Some(notice) = self.update_notice
            {
                unsafe {
                    let _ = ShowWindow(notice, SW_HIDE);
                    let _ = KillTimer(Some(hwnd), crate::update::HIDE_TIMER_ID);
                }
            }
            return;
        }
        let label = release.label();
        if self.update_notice.is_none() {
            self.update_notice = create_update_notice(hwnd, &release.title());
        }
        let Some(notice) = self.update_notice else {
            return;
        };
        let center_x = self.pos_x + self.dimensions.width / 2;
        let bottom = self.pos_y + self.dimensions.height
            - self.dimensions.shadow_margin_bottom.round() as i32;
        let key = (center_x, bottom, self.dimensions.dpi, p);
        if self.update_drawn == Some(key) {
            return;
        }
        self.update_drawn = Some(key);
        let _ = self.renderer.render_update_notice(
            notice,
            center_x,
            bottom,
            &self.dimensions,
            &label,
            p,
        );
        if !self.update_shown {
            self.update_shown = true;
            let (after, flags) = z_order(self.settings.always_on_top);
            unsafe {
                let _ = SetWindowPos(
                    notice,
                    after,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW | flags,
                );
                let _ = SetTimer(
                    Some(hwnd),
                    crate::update::HIDE_TIMER_ID,
                    crate::update::SHOW_MS,
                    None,
                );
            }
        }
    }

    /// Draws the bubble where the notch's right edge is now (shown on first
    /// need, hidden once fully gone). Never activated, never hit.
    fn update_bubble(&mut self, hwnd: HWND) {
        let p = self.bubble_presence.level();
        if p <= 0.0 && !self.bubble_presence.is_animating() {
            if std::mem::take(&mut self.bubble_shown)
                && let Some(bubble) = self.bubble
            {
                unsafe {
                    let _ = ShowWindow(bubble, SW_HIDE);
                }
            }
            return;
        }
        if self.bubble.is_none() {
            self.bubble = create_bubble(hwnd);
        }
        let Some(bubble) = self.bubble else {
            return;
        };
        // The notch body's right edge (past its top shoulder)
        let right = self.pos_x + self.dimensions.width
            - (self.dimensions.shadow_margin_x + self.dimensions.curvature.top_transition_radius)
                .round() as i32;
        let _ = self
            .renderer
            .render_bubble(bubble, right, self.pos_y, self.dimensions.dpi, p);
        if !self.bubble_shown {
            self.bubble_shown = true;
            let (after, flags) = z_order(self.settings.always_on_top);
            unsafe {
                let _ = SetWindowPos(
                    bubble,
                    after,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW | flags,
                );
            }
            // Its ring now sets the pace of the countdown's wake-ups
            self.arm_timer_tick(hwnd);
        }
    }

    /// (Re)arms the countdown's one wake-up: when its displayed second next
    /// changes, or when a finished one's hold ends; none otherwise.
    fn arm_timer_tick(&mut self, hwnd: HWND) {
        let mut wake = self.timer.next_wake(std::time::Instant::now());
        // The bubble's ring depletes smoothly: often enough while it shows
        if self.bubble_shown && matches!(self.timer.phase(), Phase::Running { .. }) {
            let ring = crate::layout::timer_bubble_geometry(self.dimensions.dpi, 1.0)
                .ring
                .0;
            let step = crate::timer::ring_step_ms(self.timer.duration().as_millis() as u64, ring);
            wake = wake.map(|w| w.min(std::time::Duration::from_millis(step)));
        }
        match wake {
            Some(wait) => unsafe {
                let ms = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX).max(1);
                let _ = SetTimer(Some(hwnd), TIMER_TICK_TIMER_ID, ms, None);
            },
            None => unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_TICK_TIMER_ID);
            },
        }
    }

    /// After the countdown's phase changed (its scale showing `from` just
    /// before): re-arm its wake-up, re-aim the collapsed presentations and
    /// the panel's motion, redraw, and announce the new state.
    fn timer_changed(&mut self, hwnd: HWND, from: f32) {
        self.arm_timer_tick(hwnd);
        self.refresh_timer_presence(hwnd);
        self.aim_timer_motion(hwnd, from);
        self.redraw(hwnd);
        notify_accessibility_state_changed(
            hwnd,
            self.state,
            self.clock.state(),
            self.media_content.as_ref(),
            self.space,
            &self.clipboard,
            self.clear_pending,
            self.settings_title(),
            self.timer_title().as_deref(),
        );
    }

    /// The timer's wake-up: completes / dismisses through the countdown's
    /// own deadline (a stale wake-up changes nothing), else shows the new
    /// second.
    fn on_timer_tick(&mut self, hwnd: HWND) {
        let from = self.shown_scale();
        if self.timer.poll(std::time::Instant::now()).is_some() {
            self.timer_changed(hwnd, from);
            return;
        }
        self.arm_timer_tick(hwnd);
        self.push_timer_view();
        self.redraw(hwnd);
        if self.bubble_shown {
            self.update_bubble(hwnd);
        }
    }

    /// A volume key (registered hotkey, repeating while held): queued for
    /// the volume worker (the UI thread never waits on a step), and the HUD
    /// shown at once at the actual level; each applied step then arrives as
    /// a change. Without an audio endpoint the keys go back to Windows, this
    /// press included.
    fn volume_key(&mut self, hwnd: HWND, key: VolumeKey) {
        if self.volume.is_none() {
            self.volume = crate::hud::SystemVolume::open(hwnd, WM_APP_VOLUME_CHANGED).ok();
        }
        let current = self.volume.as_ref().map(|v| {
            v.queue(key);
            v.level()
        });
        match current {
            Some(Ok((level, muted))) => {
                // Timed by the key's own input (its message time), never by
                // when the audio service applies it
                let at = unsafe { GetMessageTime() } as u32;
                let gap = crate::hud::rapid_gap_ms(keyboard_speed());
                self.momentum.press(key, at, level, gap);
                // Reversed or muted: a stretch lets go at once
                if self.momentum.edge().is_none() && self.hud.is_stretched() {
                    self.hud.stretch(None, !self.motion_enabled());
                }
                // No repeat within the gap: the key was released
                unsafe {
                    let _ = SetTimer(Some(hwnd), HUD_RELEASE_TIMER_ID, gap, None);
                }
                self.show_hud(hwnd, level, muted);
            }
            _ => self.give_back_volume_keys(hwnd, key),
        }
    }

    /// Battery / Energy Saver turned on or off. On: decorative motion in
    /// flight lands at its end now (the HUD's bar, number, waves, stretch;
    /// the countdown's presentations; hover), so nothing jumps later and
    /// no frames are left running; a notch expand / collapse in progress
    /// still completes. Off: motion returns, unless the user's own Reduced
    /// motion is on (the saved setting is never touched).
    /// With `notice` (a real change, not the startup report) the HUD says
    /// so: Power Saving Mode On / Off, for `HUD_NOTICE_HOLD_MS`. Turning on,
    /// it appears at once (no motion any more); off, it animates in.
    fn power_saver_changed(&mut self, hwnd: HWND, notice: bool) {
        if !self.motion_enabled() {
            self.hud.settle();
            self.renderer.set_hud(self.hud.view());
            if self.animation.is_none() {
                let _ = self.apply_hud_width();
            }
            self.refresh_timer_presence(hwnd);
            self.kick_feedback(hwnd);
        }
        if notice {
            self.hud
                .show_power(power_saver_active(), !self.motion_enabled());
            unsafe {
                let _ = SetTimer(
                    Some(hwnd),
                    HUD_HIDE_TIMER_ID,
                    crate::config::HUD_NOTICE_HOLD_MS,
                    None,
                );
            }
            self.hud_changed(hwnd);
            return;
        }
        // Frames stop at the next one if nothing moves any more
        self.redraw(hwnd);
    }

    /// The volume key was released (it stopped repeating): a stretch
    /// relaxes back to the notch's exact shape.
    fn volume_released(&mut self, hwnd: HWND) {
        self.momentum.release();
        if self.hud.is_stretched() {
            self.hud.stretch(None, !self.motion_enabled());
            self.hud_changed(hwnd);
        }
    }

    fn give_back_volume_keys(&mut self, hwnd: HWND, key: VolumeKey) {
        self.volume = None;
        crate::hud::release_volume_keys(hwnd);
        crate::hud::replay(key);
    }

    /// The master volume changed (a step applied, or a change elsewhere):
    /// the volume HUD follows the actual level.
    fn volume_changed(&mut self, hwnd: HWND) {
        let Some(volume) = &self.volume else {
            return;
        };
        if let Some(key) = volume.take_failure() {
            return self.give_back_volume_keys(hwnd, key);
        }
        if !self.hud.is_shown() {
            return;
        }
        if let Ok((level, muted)) = volume.level() {
            // A held key's run pressing against its end stretches the notch
            // toward that side for as long as it does (only Oeca's own keys;
            // never under Reduced motion); leaving the end lets go
            let now = unsafe { GetTickCount() };
            let edge = self.momentum.pressing(level, now);
            if edge.is_some() && self.motion_enabled() {
                self.hud.stretch(edge, false);
            } else if self.hud.is_stretched() {
                self.hud.stretch(None, !self.motion_enabled());
            }
            self.hud.update(level, muted, !self.motion_enabled());
            self.hud_changed(hwnd);
        }
    }

    /// Shows (or keeps showing) the HUD at a new level; it leaves
    /// `HUD_HOLD_MS` after the last one.
    fn show_hud(&mut self, hwnd: HWND, level: f32, muted: bool) {
        self.hud.show(level, muted, !self.motion_enabled());
        unsafe {
            let _ = SetTimer(Some(hwnd), HUD_HIDE_TIMER_ID, HUD_HOLD_MS, None);
        }
        self.hud_changed(hwnd);
    }

    fn hide_hud(&mut self, hwnd: HWND) {
        self.hud.hide(!self.motion_enabled());
        // Changes elsewhere no longer matter; the next press re-reads the
        // (possibly new) default output
        self.volume = None;
        self.hud_changed(hwnd);
    }

    /// The HUD has a new target: it moves on the notch's frames (started if
    /// idle; running ones are simply retargeted, never restarted). Reduced
    /// motion: the end state is drawn now.
    fn hud_changed(&mut self, hwnd: HWND) {
        self.renderer.set_hud(self.hud.view());
        if self.hud.is_animating() {
            if self.last_frame_time.is_none() {
                self.start_frames(hwnd);
            }
            return;
        }
        if self.apply_hud_width() {
            self.update_bubble(hwnd);
        }
        self.redraw(hwnd);
    }

    /// A mouse message's client x in content coordinates (the content is
    /// drawn from elsewhere while the HUD stretches the notch).
    fn client_x(&self, lparam: LPARAM) -> f32 {
        f32::from((lparam.0 & 0xFFFF) as i16) - self.renderer.content_offset().0
    }

    /// Where a collapse ends (DIP width): the collapsed notch, as wide as
    /// the HUD makes it (exactly the reference width without it).
    fn collapsed_target_dip(&self) -> f32 {
        if self.hud.extra_width() <= 0.0 {
            return crate::config::geometry_for(NotchState::Collapsed, (0.0, 0.0))[0];
        }
        let dims = self.settled_dims(NotchState::Collapsed, self.dimensions.dpi);
        dims.width as f32 / dims.scale
    }

    /// The settled notch of `state`: the collapsed one widened by the HUD.
    fn settled_dims(&self, state: NotchState, dpi: u32) -> NotchDimensions {
        let mut dims = space_dimensions(state, dpi, self.size_space());
        if state == NotchState::Collapsed {
            dims.width = crate::hud::widened_width(dims.width, self.hud.extra_width(), dims.scale);
        }
        dims
    }

    /// The settled collapsed notch follows the HUD's widening (centred).
    /// True if its size changed. No-op while expanded or animating (the notch
    /// animation carries the width, see `animation_frame`).
    fn apply_hud_width(&mut self) -> bool {
        if self.animation.is_some() || self.state != NotchState::Collapsed {
            return false;
        }
        let dims = self.settled_dims(NotchState::Collapsed, self.dimensions.dpi);
        if dims == self.dimensions {
            return false;
        }
        self.dimensions = dims;
        let screen_width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        self.pos_x = calculate_notch_x(screen_width, dims.width);
        true
    }

    /// The round primary button: start / pause / resume / restart. Media
    /// playback is never touched.
    fn timer_primary(&mut self, hwnd: HWND) {
        let from = self.shown_scale();
        if self.timer.primary(std::time::Instant::now()) {
            self.timer_changed(hwnd, from);
        }
    }

    /// Cancel: back to the idle setup (the duration is kept).
    fn timer_cancel(&mut self, hwnd: HWND) {
        let from = self.shown_scale();
        if self.timer.cancel() {
            self.timer_changed(hwnd, from);
        }
    }

    /// A mode pill: Focus / Break, at any time (a running countdown is
    /// paused and kept for when its mode comes back).
    fn timer_mode(&mut self, hwnd: HWND, mode: crate::timer::Mode) {
        let from = self.shown_scale();
        if self.timer.set_mode(mode, std::time::Instant::now()) {
            self.timer_changed(hwnd, from);
        }
    }

    /// The time was set on the scale (drag, tap or wheel), `from` the value
    /// shown before: in setup the scale glides to it; otherwise the
    /// countdown runs on from it (its wake-up re-armed).
    fn timer_time_set(&mut self, hwnd: HWND, from: f32) {
        if self.timer.phase() == Phase::Idle {
            self.aim_timer_motion(hwnd, from);
            self.announce_timer(hwnd);
        } else {
            self.timer_changed(hwnd, from);
        }
    }

    /// A press on the scale (any phase): a drag may start here, from the
    /// value shown now.
    fn begin_timer_drag(&mut self, lparam: LPARAM) {
        self.timer_drag = Some(TimerDrag {
            x0: self.client_x(lparam),
            value0: self.shown_scale(),
            stop: None,
        });
    }

    /// The scale follows the drag: moving it left by a stop's width adds a
    /// stop (the ruler is dragged under its fixed pointer). Nothing until it
    /// moves half a stop (a tap is not a drag); each stop is set once.
    fn drag_timer_to(&mut self, hwnd: HWND, lparam: LPARAM) {
        let (Some(drag), Some(l)) = (self.timer_drag, self.timer_layout()) else {
            return;
        };
        let dx = drag.x0 - self.client_x(lparam);
        if drag.stop.is_none() && dx.abs() < l.step / 2.0 {
            return;
        }
        let stop = timer_stop(drag.value0 + dx / l.step);
        if drag.stop == Some(stop) {
            return;
        }
        self.timer_drag = Some(TimerDrag {
            stop: Some(stop),
            ..drag
        });
        let from = self.shown_scale();
        if self.timer.set_stop(stop, std::time::Instant::now()) {
            self.timer_time_set(hwnd, from);
        }
    }

    /// A press on the scale ended (`lparam`: where, if released there): a
    /// tap sets the stop tapped; then the durations are saved and announced
    /// once.
    fn end_timer_drag(&mut self, hwnd: HWND, lparam: Option<LPARAM>) {
        let Some(drag) = self.timer_drag.take() else {
            return;
        };
        if drag.stop.is_none()
            && let (Some(lparam), Some(l)) = (lparam, self.timer_layout())
        {
            let from = self.shown_scale();
            let stop = timer_stop(l.stop_at(self.client_x(lparam), from));
            if self.timer.set_stop(stop, std::time::Instant::now()) {
                self.timer_time_set(hwnd, from);
            }
        }
        self.persist_timer_durations();
        self.announce_timer(hwnd);
    }

    /// Refreshes the accessible title for a new timer duration.
    fn announce_timer(&self, hwnd: HWND) {
        notify_accessibility_clock_changed(
            hwnd,
            self.state,
            self.clock.state(),
            self.media_content.as_ref(),
            self.space,
            &self.clipboard,
            self.clear_pending,
            self.settings_title(),
            self.timer_title().as_deref(),
        );
    }

    /// The timer's part of the accessible title: on the expanded Home, its
    /// state and controls; on the collapsed notch, a running or finished
    /// countdown. Elsewhere nothing.
    fn timer_title(&self) -> Option<String> {
        use crate::timer::describe;
        let now = std::time::Instant::now();
        let t = &self.timer;
        let home = self.state == NotchState::Expanded
            && self.space == OecaSpace::Home
            && !self.settings_open;
        if home {
            return Some(match t.phase() {
                Phase::Idle => format!(
                    "Timer, {} - Start button, duration scale",
                    describe(t.duration().as_secs(), true)
                ),
                Phase::Running { .. } => format!(
                    "Timer running, set for {} - Pause button, Cancel button",
                    describe(t.duration().as_secs(), true)
                ),
                Phase::Paused { .. } => format!(
                    "Timer paused, {} left - Resume button, Cancel button",
                    describe(t.display_secs(now), true)
                ),
                Phase::Done { .. } => "Timer done - Restart button, Cancel button".to_string(),
            });
        }
        if self.state == NotchState::Collapsed && t.shows_collapsed() {
            return Some(
                match t.phase() {
                    Phase::Done { .. } => "Timer done",
                    _ => "Timer running",
                }
                .to_string(),
            );
        }
        None
    }

    /// Transport control under a client-area point (only when media UI is visible).
    fn media_control_at(&self, lparam: LPARAM) -> Option<MediaControl> {
        let x = self.client_x(lparam);
        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
        self.active_media_layout()?.control_at(x, y)
    }
}

/// A press / drag on the timer's scale: where it started, the value shown
/// then, and the stop it has set (None until it moves: a tap so far).
#[derive(Debug, Clone, Copy, PartialEq)]
struct TimerDrag {
    x0: f32,
    value0: f32,
    stop: Option<usize>,
}

/// The stop nearest a (fractional) scale value, on the scale.
fn timer_stop(value: f32) -> usize {
    value
        .round()
        .clamp(0.0, (crate::timer::STOP_COUNT - 1) as f32) as usize
}

/// The Clipboard space's clear flow. Clear: asks first when `confirm` is on
/// (sets `pending`), otherwise forgets the history at once. Cancel: dismisses,
/// history untouched. Confirmed Clear: forgets it. Only Oeca's in-memory
/// history changes; the system clipboard is never touched.
fn clear_flow(hit: PanelHit, confirm: bool, pending: &mut bool, history: &mut ClipboardHistory) {
    match hit {
        PanelHit::Clear if confirm => *pending = true,
        PanelHit::Clear | PanelHit::ConfirmClear => {
            history.clear();
            *pending = false;
        }
        PanelHit::CancelClear => *pending = false,
        _ => {}
    }
}

/// Outside-click auto-collapse. The expanded notch stays open until a click
/// lands outside Oeca; that starts a grace period (`AUTO_COLLAPSE_MS`), and a
/// later outside click restarts it. Using Oeca (a click in the notch, a drag
/// onto it, the Settings window) cancels it; so does any expand / collapse.
/// When it runs out the notch collapses through the normal path.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GracePeriod {
    pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraceEvent {
    OutsideClick,
    OecaInteraction,
    StateChanged,
    TimerFired,
    /// The closing delay setting changed.
    DelayChanged,
}

/// What the window does for an event: (re)start the one-shot timer, stop
/// it, collapse, or nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraceAction {
    None,
    Arm,
    Disarm,
    Collapse,
}

impl GracePeriod {
    /// `expanded`: the notch is expanded or expanding. `at_once`: the closing
    /// delay is 0 s, so an outside click collapses without a countdown.
    pub fn handle(&mut self, event: GraceEvent, expanded: bool, at_once: bool) -> GraceAction {
        match event {
            GraceEvent::OutsideClick if expanded && at_once => {
                self.pending = false;
                GraceAction::Collapse
            }
            GraceEvent::OutsideClick if expanded => {
                self.pending = true;
                GraceAction::Arm
            }
            GraceEvent::OutsideClick => GraceAction::None,
            GraceEvent::OecaInteraction | GraceEvent::StateChanged => {
                if std::mem::take(&mut self.pending) {
                    GraceAction::Disarm
                } else {
                    GraceAction::None
                }
            }
            // A new duration restarts a pending countdown with it (0 s:
            // its time is up)
            GraceEvent::DelayChanged if self.pending && at_once => {
                self.pending = false;
                if expanded {
                    GraceAction::Collapse
                } else {
                    GraceAction::Disarm
                }
            }
            GraceEvent::DelayChanged if self.pending => GraceAction::Arm,
            GraceEvent::DelayChanged => GraceAction::None,
            // A fire with nothing pending is stale (cancelled meanwhile)
            GraceEvent::TimerFired => {
                if std::mem::take(&mut self.pending) && expanded {
                    GraceAction::Collapse
                } else {
                    GraceAction::Disarm
                }
            }
        }
    }

    #[cfg(test)]
    pub fn pending(&self) -> bool {
        self.pending
    }
}

impl GraceEvent {
    const ALL: [Self; 5] = [
        Self::OutsideClick,
        Self::OecaInteraction,
        Self::StateChanged,
        Self::TimerFired,
        Self::DelayChanged,
    ];
}

/// Posted to the notch with a `GraceEvent` index (from Raw Input, the
/// Settings window, or the timer) so it is applied through the state guard.
pub const WM_APP_GRACE: u32 = WM_APP + 40;

/// Posted when the countdown's wake-up fires (see `TIMER_TICK_TIMER_ID`).
const WM_APP_TIMER_TICK: u32 = WM_APP + 41;

/// Posted by a drop's decoding worker (lParam: its `Box<dragdrop::Decoded>`).
const WM_APP_DROP_DECODED: u32 = WM_APP + 43;

/// Posted when a drop result has been on show long enough.
const WM_APP_DROP_RESULT_DONE: u32 = WM_APP + 44;

/// Posted by the master volume's change callback (an audio thread).
const WM_APP_VOLUME_CHANGED: u32 = WM_APP + 45;
/// Posted when the HUD has been on show long enough.
const WM_APP_HUD_HIDE: u32 = WM_APP + 47;
/// Posted when a volume key stopped repeating (released).
const WM_APP_VOLUME_RELEASED: u32 = WM_APP + 48;
/// Posted when Battery / Energy Saver turned on or off (effective motion).
const WM_APP_POWER_SAVER: u32 = WM_APP + 49;
/// Posted by an update check (lParam: a `Box<update::Release>` when a newer
/// stable release exists, else 0).
const WM_APP_UPDATE_READY: u32 = WM_APP + 51;
/// Posted by the update notice: wParam 1 opens the release, 0 dismisses.
const WM_APP_UPDATE_CLICK: u32 = WM_APP + 52;
/// Posted when the daily update check is due (applied through the state,
/// so a busy state replays it).
const WM_APP_UPDATE_CHECK: u32 = WM_APP + 53;

/// The update notice and the collapsed notch's icon, from the shared update
/// state: the notice while it is due (under the collapsed notch only), the
/// icon once it was dismissed, for as long as the release is newer.
fn update_targets(updates: &crate::update::UpdateState, collapsed: bool) -> (bool, bool) {
    let due = updates.notice_due();
    (due && collapsed, updates.release().is_some() && !due)
}

/// Whose a click is, by the top-level window under it: the notch itself
/// (handled by its own mouse messages), Oeca's Settings window (Oeca in use),
/// or anything else (outside). The notch's transparent margins pass clicks
/// through, so they count as outside.
fn classify_click(root: HWND, notch: HWND, settings: Option<HWND>) -> Option<GraceEvent> {
    if root == notch {
        None
    } else if Some(root) == settings {
        Some(GraceEvent::OecaInteraction)
    } else {
        Some(GraceEvent::OutsideClick)
    }
}

/// Whether the notch's own gesture (a scrub, a drag on the timer's scale)
/// is in progress: it holds the mouse capture, so every button event belongs
/// to that gesture and none can count as an outside click.
fn gesture_owns_buttons(capture: HWND, notch: HWND) -> bool {
    !capture.is_invalid() && capture == notch
}

/// Where a scrub released at `fraction` of the track seeks to (ms), for a
/// track `duration_ms` long: always inside the track, however far past its
/// ends the pointer was let go.
fn scrub_position(fraction: f32, duration_ms: i64) -> i64 {
    ((f64::from(fraction.clamp(0.0, 1.0)) * duration_ms as f64).round() as i64)
        .clamp(0, duration_ms.max(0))
}

/// Registers (`on`) or removes this process's Raw Input mouse sink: button
/// presses are reported to `hwnd` even while another app is in front.
/// Read-only (input still reaches its target unchanged); true on success.
fn watch_outside_clicks(hwnd: HWND, on: bool) -> bool {
    use windows::Win32::UI::Input::{
        RAWINPUTDEVICE, RIDEV_INPUTSINK, RIDEV_REMOVE, RegisterRawInputDevices,
    };
    let device = RAWINPUTDEVICE {
        usUsagePage: 0x01, // generic desktop
        usUsage: 0x02,     // mouse
        dwFlags: if on { RIDEV_INPUTSINK } else { RIDEV_REMOVE },
        hwndTarget: if on { hwnd } else { HWND::default() },
    };
    unsafe {
        RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32).is_ok()
    }
}

/// Whether a `WM_INPUT` is a mouse button being released (moves and
/// presses are ignored). Outside clicks count on release: a drag that starts
/// elsewhere (a file from Explorer) and ends on the notch is no outside click.
fn raw_button_released(lparam: LPARAM) -> bool {
    use windows::Win32::UI::Input::{
        GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTHEADER, RID_INPUT, RIM_TYPEMOUSE,
    };
    // Left, right, middle, X1, X2 up
    const UPS: u16 = 0x0002 | 0x0008 | 0x0020 | 0x0080 | 0x0200;
    let mut raw = RAWINPUT::default();
    let mut size = std::mem::size_of::<RAWINPUT>() as u32;
    let read = unsafe {
        GetRawInputData(
            HRAWINPUT(lparam.0 as *mut _),
            RID_INPUT,
            Some(&mut raw as *mut _ as *mut core::ffi::c_void),
            &mut size,
            std::mem::size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    read != u32::MAX
        && raw.header.dwType == RIM_TYPEMOUSE.0
        && unsafe { raw.data.mouse.Anonymous.Anonymous.usButtonFlags } & UPS != 0
}

/// The playback live timer's period: visualizer frames whenever bars can be on
/// screen (collapsed, animating, or Music with its visualizer shown); the slow
/// scrubber rate in the settled expanded Music space with the visualizer
/// hidden.
fn live_period_ms(
    animating: bool,
    state: NotchState,
    music_showing: bool,
    show_visualizer: bool,
) -> u32 {
    if !animating && state == NotchState::Expanded && music_showing && !show_visualizer {
        crate::config::MEDIA_SCRUBBER_FRAME_MS
    } else {
        MEDIA_LIVE_FRAME_MS
    }
}

/// Animations run only when neither the user (Reduced motion) nor the system
/// asked for reduced motion, and Windows is not saving power.
pub(crate) fn motion_allowed(reduced_motion: bool, system_animations: bool) -> bool {
    effective_motion(reduced_motion, system_animations, power_saver_active())
}

/// The motion actually used: the user's Reduced motion OR the system's
/// animation setting OR an active Battery / Energy Saver. The saved setting
/// is never changed by the last two.
pub(crate) fn effective_motion(reduced_motion: bool, system_animations: bool, saver: bool) -> bool {
    system_animations && !reduced_motion && !saver
}

/// Windows' power saving, as last reported: Battery Saver, and Energy Saver
/// (Windows 11 24H2+; either one counts). Set from their power-setting
/// notifications (the UI thread), read wherever motion is decided.
static BATTERY_SAVER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static ENERGY_SAVER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Whether each saver has reported yet (its first report, on registering,
/// only states the current mode: no notice).
static BATTERY_SAVER_SEEN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static ENERGY_SAVER_SEEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn power_saver_active() -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    BATTERY_SAVER.load(Relaxed) || ENERGY_SAVER.load(Relaxed)
}

/// Energy Saver's status, 0 off / 1 standard / 2 high savings (winnt.h,
/// Windows SDK 10.0.26100; not in the `windows` crate's metadata yet).
const GUID_ENERGY_SAVER_STATUS: windows::core::GUID =
    windows::core::GUID::from_u128(0x550e8400_e29b_41d4_a716_446655440000);

/// Which power-saving status a notification reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Saver {
    Battery,
    Energy,
}

impl Saver {
    const ALL: [Saver; 2] = [Saver::Battery, Saver::Energy];

    fn guid(self) -> windows::core::GUID {
        match self {
            Self::Battery => windows::Win32::System::SystemServices::GUID_POWER_SAVING_STATUS,
            Self::Energy => GUID_ENERGY_SAVER_STATUS,
        }
    }

    fn flag(self) -> &'static std::sync::atomic::AtomicBool {
        match self {
            Self::Battery => &BATTERY_SAVER,
            Self::Energy => &ENERGY_SAVER,
        }
    }

    fn seen(self) -> &'static std::sync::atomic::AtomicBool {
        match self {
            Self::Battery => &BATTERY_SAVER_SEEN,
            Self::Energy => &ENERGY_SAVER_SEEN,
        }
    }
}

/// A `PBT_POWERSETTINGCHANGE` for one of the savers: which, and whether on
/// (Battery Saver 1; Energy Saver 1 or 2).
///
/// # Safety
/// `lparam` is the `WM_POWERBROADCAST` / `PBT_POWERSETTINGCHANGE` one.
unsafe fn saver_report(lparam: LPARAM) -> Option<(Saver, bool)> {
    let setting = lparam.0 as *const windows::Win32::System::Power::POWERBROADCAST_SETTING;
    if setting.is_null() {
        return None;
    }
    let s = unsafe { &*setting };
    if s.DataLength < 4 {
        return None;
    }
    let value = unsafe { std::ptr::read_unaligned(s.Data.as_ptr().cast::<u32>()) };
    Saver::ALL
        .into_iter()
        .find(|saver| saver.guid() == s.PowerSetting)
        .map(|saver| (saver, value != 0))
}

/// Asks for both savers' notifications (each is sent once at once with its
/// current state, then on every change). Before any arrives, and where they
/// are not offered, the power status's Battery Saver flag stands in.
fn watch_power_saving(hwnd: HWND) -> [Option<windows::Win32::System::Power::HPOWERNOTIFY>; 2] {
    use windows::Win32::System::Power::{
        GetSystemPowerStatus, RegisterPowerSettingNotification, SYSTEM_POWER_STATUS,
    };
    let mut status = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut status) }.is_ok() {
        BATTERY_SAVER.store(
            status.SystemStatusFlag == 1,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
    Saver::ALL.map(|saver| {
        unsafe {
            RegisterPowerSettingNotification(
                windows::Win32::Foundation::HANDLE(hwnd.0),
                &saver.guid(),
                windows::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_WINDOW_HANDLE,
            )
        }
        .ok()
    })
}

/// Checks if system-level client area animations are enabled.
/// If disabled (user requested reduced motion), animations should complete instantly.
pub(crate) fn is_client_animation_enabled() -> bool {
    let mut anim_enabled = windows::core::BOOL(1);
    let res = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut anim_enabled as *mut _ as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if res.is_ok() {
        anim_enabled.as_bool()
    } else {
        true
    }
}

/// Accessible description of the space selector, e.g. "Home space selected
/// (spaces: Home, Music, Clipboard)".
fn accessible_space(space: OecaSpace) -> String {
    let all: Vec<&str> = OecaSpace::ALL.iter().map(|s| s.label()).collect();
    format!(
        "{} space selected (spaces: {})",
        space.label(),
        all.join(", ")
    )
}

/// The clear confirmation's accessible text: its message, then its two
/// buttons by name (Cancel first).
const CLEAR_CONFIRM_ACCESSIBLE: &str = "Clear clipboard history? Only Oeca's history is removed, the Windows clipboard is not changed - Cancel button - Clear button";

/// Accessible title including the Clipboard space: there it describes the
/// history (count, newest entry's kind, the Clear control), never contents.
fn accessible_title(
    state: NotchState,
    clock: &crate::clock::ClockDateState,
    media: Option<&MediaContent>,
    space: OecaSpace,
    clipboard: &ClipboardHistory,
    clear_pending: bool,
    settings: Option<OecaSettings>,
) -> String {
    if let (NotchState::Expanded, Some(s)) = (state, settings) {
        let enabled = |on| if on { "enabled" } else { "disabled" };
        let switch = |on| if on { "on" } else { "off" };
        return format!(
            "Settings, Always on top {}, Reduced motion {} - Always on top switch, {} - Reduced motion switch, {} - Open Full Settings button - {} - {} - {}",
            enabled(s.always_on_top),
            enabled(s.reduced_motion),
            switch(s.always_on_top),
            switch(s.reduced_motion),
            clock.formatted_time,
            clock.formatted_date,
            accessible_space(space)
        );
    }
    if state != NotchState::Expanded || space != OecaSpace::Clipboard {
        return accessible_title_for_clock(state, clock, media, space);
    }
    let history = match (clipboard.len(), clipboard.get(0)) {
        _ if clear_pending => CLEAR_CONFIRM_ACCESSIBLE.to_string(),
        (n, Some(newest)) => format!(
            "Clipboard history, {n} {}, newest is {}, Copy and Delete buttons on each item, Clear history button",
            if n == 1 { "item" } else { "items" },
            match newest {
                ClipboardItem::Text(_) => "text",
                ClipboardItem::Image(_) => "an image",
            }
        ),
        _ => "Clipboard history, empty".to_string(),
    };
    format!(
        "{history} - {} - {} - {}",
        clock.formatted_time,
        clock.formatted_date,
        accessible_space(space)
    )
}

/// Returns the user-facing accessible title for a given notch state and clock state.
/// When expanded with media, the displayed track (and the control actions) lead the
/// title; the expanded notch ends with the active space and the selectable spaces.
fn accessible_title_for_clock(
    state: NotchState,
    clock: &crate::clock::ClockDateState,
    media: Option<&MediaContent>,
    space: OecaSpace,
) -> String {
    match (state, media) {
        (NotchState::Collapsed, _) => clock.formatted_time.clone(),
        (NotchState::Expanded, None) => format!(
            "{} - {} - {}",
            clock.formatted_time,
            clock.formatted_date,
            accessible_space(space)
        ),
        (NotchState::Expanded, Some(m)) => format!(
            "{} - {}, {}, {} - {} - {} - {}",
            m.accessible_text(),
            MediaControl::Previous.accessible_name(m.icon),
            MediaControl::PlayPause.accessible_name(m.icon),
            MediaControl::Next.accessible_name(m.icon),
            clock.formatted_time,
            clock.formatted_date,
            accessible_space(space)
        ),
    }
}

/// The title with the timer's part (if any) after it.
fn with_timer(title: String, timer: Option<&str>) -> String {
    match timer {
        Some(t) => format!("{title} - {t}"),
        None => title,
    }
}

/// Notifies Windows accessibility clients (screen readers, UI Automation bridge)
/// of a settled state change and updates the top-level window title.
#[allow(clippy::too_many_arguments)]
fn notify_accessibility_state_changed(
    hwnd: HWND,
    new_state: NotchState,
    clock: &crate::clock::ClockDateState,
    media: Option<&MediaContent>,
    space: OecaSpace,
    clipboard: &ClipboardHistory,
    clear_pending: bool,
    settings: Option<OecaSettings>,
    timer: Option<&str>,
) {
    let title = with_timer(
        accessible_title(
            new_state,
            clock,
            media,
            space,
            clipboard,
            clear_pending,
            settings,
        ),
        timer,
    );
    let wide_title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = SetWindowTextW(hwnd, windows::core::PCWSTR(wide_title.as_ptr()));
        NotifyWinEvent(EVENT_OBJECT_NAMECHANGE, hwnd, OBJID_CLIENT.0, 0);
        NotifyWinEvent(EVENT_OBJECT_STATECHANGE, hwnd, OBJID_CLIENT.0, 0);
    }
}

/// Notifies accessibility clients when the displayed clock minute actually changes.
#[allow(clippy::too_many_arguments)]
fn notify_accessibility_clock_changed(
    hwnd: HWND,
    state: NotchState,
    clock: &crate::clock::ClockDateState,
    media: Option<&MediaContent>,
    space: OecaSpace,
    clipboard: &ClipboardHistory,
    clear_pending: bool,
    settings: Option<OecaSettings>,
    timer: Option<&str>,
) {
    let title = with_timer(
        accessible_title(
            state,
            clock,
            media,
            space,
            clipboard,
            clear_pending,
            settings,
        ),
        timer,
    );
    let wide_title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = SetWindowTextW(hwnd, windows::core::PCWSTR(wide_title.as_ptr()));
        NotifyWinEvent(EVENT_OBJECT_NAMECHANGE, hwnd, OBJID_CLIENT.0, 0);
    }
}

/// Initiates a smooth state transition or gracefully reverses an in-flight transition.
/// If reduced motion is requested by the system preference, transitions immediately without delay.
fn start_or_reverse_animation(hwnd: HWND, state: &mut WindowState) -> Result<()> {
    let target_state = if let Some(current_anim) = &state.animation {
        match current_anim.target_state {
            NotchState::Collapsed => NotchState::Expanded,
            NotchState::Expanded => NotchState::Collapsed,
        }
    } else {
        match state.state {
            NotchState::Collapsed => NotchState::Expanded,
            NotchState::Expanded => NotchState::Collapsed,
        }
    };
    // Closing takes a drop page that outlived its drag with it (a result on
    // show, or a drop still decoding: its images still reach the history)
    if target_state == NotchState::Collapsed && !state.drag.is_active() {
        state.stop_drop_result(hwnd);
        state.renderer.set_drop(crate::dragdrop::DropPage::Off);
    }
    // A scrub in progress ends without seeking
    state.end_scrub(hwnd, false);
    // Opening or closing ends a held key's stretch (it relaxes on the way)
    state.momentum.release();
    if state.hud.is_stretched() {
        state.hud.stretch(None, !state.motion_enabled());
        state.renderer.set_hud(state.hud.view());
    }
    // Collapsing closes the Settings page; the next expansion shows the space
    if target_state == NotchState::Collapsed && state.settings_open {
        state.settings_open = false;
        state.renderer.set_settings(false, state.settings);
    }
    // Collapsing also dismisses a pending clear (nothing is cleared)
    if target_state == NotchState::Collapsed {
        state.set_clear_pending(hwnd, false);
    }
    // Any expand / collapse ends a pending auto-collapse (a stale timer can
    // never close a notch that has since reopened); outside clicks are only
    // watched while expanded
    state.grace(hwnd, GraceEvent::StateChanged);
    state.set_outside_watch(hwnd, target_state == NotchState::Expanded);

    if !state.motion_enabled() {
        // Reduced motion requested: snap immediately to target state without timer
        state.stop_frames(hwnd);
        state.animation = None;
        state.renderer.set_transition(None);
        state.state = target_state;
        let new_dims = state.settled_dims(target_state, state.dimensions.dpi);
        state.dimensions = new_dims;

        let screen_width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let new_x = calculate_notch_x(screen_width, new_dims.width);
        state.pos_x = new_x;
        state.pos_y = 0;

        // Check if cursor remains inside the new notch geometry after resize
        let mut cursor_pt = POINT::default();
        let cursor_in_notch = unsafe {
            if GetCursorPos(&mut cursor_pt).is_ok() {
                let mut client_pt = cursor_pt;
                let _ = ScreenToClient(hwnd, &mut client_pt);
                new_dims.contains_point(client_pt.x as f32, client_pt.y as f32)
            } else {
                false
            }
        };

        if state.hovered != cursor_in_notch {
            state.hovered = cursor_in_notch;
            if cursor_in_notch {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                unsafe {
                    let _ = TrackMouseEvent(&mut tme);
                }
            }
        }

        state.renderer.render(
            hwnd,
            new_x,
            0,
            &state.dimensions,
            state.hovered,
            state.clock.state(),
        )?;
        unsafe {
            SetWindowPos(
                hwnd,
                z_order(state.settings.always_on_top).0,
                new_x,
                0,
                new_dims.width,
                new_dims.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW | z_order(state.settings.always_on_top).1,
            )?;
        }

        notify_accessibility_state_changed(
            hwnd,
            target_state,
            state.clock.state(),
            state.media_content.as_ref(),
            state.space,
            &state.clipboard,
            state.clear_pending,
            state.settings_title(),
            state.timer_title().as_deref(),
        );
        // The countdown's collapsed presentations follow at once
        state.refresh_timer_presence(hwnd);
        return Ok(());
    }

    // The expanded end is the active space's (or drop page's) size. A running
    // animation is retargeted: it continues from its position and velocity.
    let size = expanded_size_dip(state.size_space(), state.dimensions.dpi);
    let scene = state.scene();
    let new_anim = if let Some(mut anim) = state.animation {
        anim.retarget(target_state, size);
        anim.retarget_scene(scene);
        anim
    } else {
        let from = (
            state.dimensions.width as f32 / state.dimensions.scale,
            state.dimensions.height as f32 / state.dimensions.scale,
        );
        let mut anim = AnimationState::start(state.state, from, target_state, size, scene);
        anim.scale = state.dimensions.scale;
        // Opening from a notch the HUD has widened: from its actual width
        if state.state == NotchState::Collapsed && state.hud.extra_width() > 0.0 {
            anim.pos[0] = from.0;
        }
        anim
    };

    state.animation = Some(new_anim);
    // A reversal keeps the running frames; the resolution request is not stacked
    state.start_frames(hwnd);
    // Expanding: the bubble / slot leave at once; collapsing: they follow
    // once the collapsed content fades in (re-aimed on every frame)
    state.refresh_timer_presence(hwnd);
    Ok(())
}

/// Switches the notch visual and geometry state, recomputing centered coordinates,
/// resizing the native window, and re-rendering Direct2D content atomically.
#[allow(dead_code)]
fn set_notch_state(hwnd: HWND, state: &mut WindowState, new_state: NotchState) -> Result<()> {
    let dpi = state.dimensions.dpi;
    let new_dims = space_dimensions(new_state, dpi, state.size_space());
    let screen_width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let new_x = calculate_notch_x(screen_width, new_dims.width);
    let new_y = 0;

    state.state = new_state;
    state.dimensions = new_dims;
    state.pos_x = new_x;
    state.pos_y = new_y;

    // Check if cursor remains inside the new notch geometry after resize
    let mut cursor_pt = POINT::default();
    let cursor_in_notch = unsafe {
        if GetCursorPos(&mut cursor_pt).is_ok() {
            let mut client_pt = cursor_pt;
            let _ = ScreenToClient(hwnd, &mut client_pt);
            new_dims.contains_point(client_pt.x as f32, client_pt.y as f32)
        } else {
            false
        }
    };

    state.hovered = cursor_in_notch;
    if cursor_in_notch {
        let mut tme = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        unsafe {
            let _ = TrackMouseEvent(&mut tme);
        }
    }

    state.renderer.render(
        hwnd,
        new_x,
        new_y,
        &state.dimensions,
        state.hovered,
        state.clock.state(),
    )?;
    unsafe {
        SetWindowPos(
            hwnd,
            z_order(state.settings.always_on_top).0,
            new_x,
            new_y,
            new_dims.width,
            new_dims.height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW | z_order(state.settings.always_on_top).1,
        )?;
    }
    notify_accessibility_state_changed(
        hwnd,
        new_state,
        state.clock.state(),
        state.media_content.as_ref(),
        state.space,
        &state.clipboard,
        state.clear_pending,
        state.settings_title(),
        state.timer_title().as_deref(),
    );
    Ok(())
}

/// The keyboard repeat rate setting (0..31; Windows' default 31 if unread).
fn keyboard_speed() -> u32 {
    let mut speed = 31u32;
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETKEYBOARDSPEED,
            0,
            Some(&mut speed as *mut u32 as *mut core::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    speed
}

/// A frame with only the HUD moving: the settled notch follows its widening
/// and is drawn once; the frames stop when the HUD settles.
fn hud_frame(hwnd: HWND, state: &mut WindowState, hud_active: bool) {
    let resized = state.apply_hud_width();
    let drawing = std::time::Instant::now();
    let _ = state.renderer.render(
        hwnd,
        state.pos_x,
        state.pos_y,
        &state.dimensions,
        state.hovered,
        state.clock.state(),
    );
    let cost = drawing.elapsed().as_secs_f64() * 1000.0;
    state.frame_lead_ms = cost.max(state.frame_lead_ms * 0.9);
    // The countdown bubble rides the notch's edge
    if resized && state.bubble_shown {
        state.update_bubble(hwnd);
    }
    if !hud_active {
        state.stop_frames(hwnd);
    }
}

/// Private: a `WM_DPICHANGED` that arrived re-entrantly, replayed later (the real
/// message cannot be posted). wParam carries the original DPI pair.
const WM_APP_DEFERRED_DPICHANGED: u32 = WM_APP + 16;

/// One frame of the notch animation (expand, collapse, space switch): steps
/// the springs to this frame's display time, draws, and on settling draws the
/// exact end state and stops the frames.
fn animation_frame(hwnd: HWND, state: &mut WindowState) {
    // Motion is sampled at the refresh this frame will be shown on, so every
    // frame advances by whole refresh periods whenever the tick happens to
    // run; a second tick for an already drawn refresh draws nothing
    let now = frame_time(state.frame_lead_ms).0;
    let delta_ms = match state.last_frame_time {
        Some(last) if now <= last => return,
        Some(last) => (now - last) as f32,
        None => ANIMATION_FRAME_INTERVAL_MS as f32,
    };
    state.last_frame_time = Some(now);

    // The volume HUD moves on the same frames
    let hud_active = state.hud.step(delta_ms);
    state.renderer.set_hud(state.hud.view());
    // ...and the timer panel's selector / scale
    let timer_moving = state.timer_motion.is_animating();
    let timer_active = state.timer_motion.step(delta_ms);
    if timer_moving {
        state.push_timer_view();
    }
    let hud_active = hud_active || timer_active;
    if state.animation.is_none() {
        hud_frame(hwnd, state, hud_active);
        return;
    }

    // A collapse lands on the width the HUD gives the collapsed notch
    let collapsed_width = state.collapsed_target_dip();
    let still_active = if let Some(anim) = &mut state.animation {
        if anim.target_state == NotchState::Collapsed {
            anim.target[0] = collapsed_width;
        }
        anim.step(delta_ms)
    } else {
        false
    };

    let current_dims = if let Some(anim) = &state.animation {
        state.renderer.set_transition(Some(anim.transition()));
        anim.current_dimensions(state.dimensions.dpi)
    } else {
        state.dimensions
    };

    state.dimensions = current_dims;
    let screen_width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let new_x = calculate_notch_x(screen_width, current_dims.width);
    let new_y = 0;
    state.pos_x = new_x;
    state.pos_y = new_y;

    let drawing = std::time::Instant::now();
    let _ = state.renderer.render(
        hwnd,
        new_x,
        new_y,
        &state.dimensions,
        state.hovered,
        state.clock.state(),
    );
    // A decaying maximum: one slow frame aims the next few a refresh later
    // rather than letting them miss it
    let cost = drawing.elapsed().as_secs_f64() * 1000.0;
    state.frame_lead_ms = cost.max(state.frame_lead_ms * 0.9);
    // The bubble rides the notch's edge; collapsed
    // presentations follow the collapsed content's fade
    state.refresh_timer_presence(hwnd);

    if !still_active {
        // Animation complete: snap cleanly to target state and deactivate timer
        if let Some(anim) = state.animation.take() {
            state.state = anim.target_state;
            state.dimensions = state.settled_dims(anim.target_state, state.dimensions.dpi);
            // The last frame was already at the target; draw the
            // settled state once (exact pixels, no transition)
            state.renderer.set_transition(None);
            let _ = state.renderer.render(
                hwnd,
                new_x,
                new_y,
                &state.dimensions,
                state.hovered,
                state.clock.state(),
            );
            notify_accessibility_state_changed(
                hwnd,
                state.state,
                state.clock.state(),
                state.media_content.as_ref(),
                state.space,
                &state.clipboard,
                state.clear_pending,
                state.settings_title(),
                state.timer_title().as_deref(),
            );
        }
        // A HUD still moving keeps the frames (they continue as HUD frames)
        if !hud_active {
            state.stop_frames(hwnd);
        }
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                z_order(state.settings.always_on_top).0,
                new_x,
                new_y,
                state.dimensions.width,
                state.dimensions.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW | z_order(state.settings.always_on_top).1,
            );
        }

        // Re-evaluate cursor in notch after transition completes
        let mut cursor_pt = POINT::default();
        let cursor_in_notch = unsafe {
            if GetCursorPos(&mut cursor_pt).is_ok() {
                let mut client_pt = cursor_pt;
                let _ = ScreenToClient(hwnd, &mut client_pt);
                state
                    .dimensions
                    .contains_point(client_pt.x as f32, client_pt.y as f32)
            } else {
                false
            }
        };
        if state.hovered != cursor_in_notch {
            state.hovered = cursor_in_notch;
            if cursor_in_notch {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                unsafe {
                    let _ = TrackMouseEvent(&mut tme);
                }
            }
        }

        // Final static render at exact target dimensions
        let _ = state.renderer.render(
            hwnd,
            state.pos_x,
            state.pos_y,
            &state.dimensions,
            state.hovered,
            state.clock.state(),
        );
        // Settled somewhere showing a visualizer/scrubber again
        state.kick_live(hwnd);
        state.refresh_timer_presence(hwnd);
    }
}

/// What to do with a message that arrives while `WindowState` is already in use
/// by an outer handler (synchronous re-entry, e.g. `ReleaseCapture` sending
/// `WM_CAPTURECHANGED`, or `SetWindowPos`/`UpdateLayeredWindow` sending
/// `WM_DPICHANGED`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BusyPolicy {
    /// Recurs on its own (periodic timers, pointer moves)
    Drop,
    /// Replay this message after the outer handler returns
    Replay(u32),
}

fn busy_policy(msg: u32) -> BusyPolicy {
    match msg {
        WM_TIMER | WM_MOUSEMOVE => BusyPolicy::Drop,
        WM_DPICHANGED => BusyPolicy::Replay(WM_APP_DEFERRED_DPICHANGED),
        other => BusyPolicy::Replay(other),
    }
}

/// Re-entrancy guard for `WindowState` (UI thread only). Exactly one `&mut
/// WindowState` exists at a time: the window procedure only creates it through
/// `with_state`, which refuses while another handler holds it. Re-entrant
/// messages are dropped or queued and re-posted when the outermost handler
/// finishes; a `WM_NCDESTROY` arriving inside a handler defers the free until
/// that handler has returned, so no frame ever outlives its state.
#[derive(Debug, Default)]
struct StateGuard {
    busy: bool,
    deferred: Vec<(u32, usize, isize)>,
    free_on_exit: bool,
}

impl StateGuard {
    /// Returns false if state is already in use (re-entry).
    fn enter(&mut self) -> bool {
        !std::mem::replace(&mut self.busy, true)
    }

    /// Queues a re-entrant message (duplicates collapse: same msg/params).
    fn defer(&mut self, msg: u32, wparam: usize, lparam: isize) {
        if !self.deferred.contains(&(msg, wparam, lparam)) {
            self.deferred.push((msg, wparam, lparam));
        }
    }

    /// `WM_NCDESTROY`: true = free now; false = in use, free on exit.
    fn request_free(&mut self) -> bool {
        if self.busy {
            self.free_on_exit = true;
            false
        } else {
            true
        }
    }

    /// Leaves the outermost handler: (messages to replay, free state now?).
    fn exit(&mut self) -> (Vec<(u32, usize, isize)>, bool) {
        self.busy = false;
        (
            std::mem::take(&mut self.deferred),
            std::mem::take(&mut self.free_on_exit),
        )
    }
}

thread_local! {
    static GUARD: std::cell::RefCell<StateGuard> = std::cell::RefCell::new(StateGuard::default());
    /// State detached by a deferred WM_NCDESTROY, freed when the handler exits.
    static DETACHED: std::cell::Cell<*mut WindowState> = const { std::cell::Cell::new(std::ptr::null_mut()) };
    /// Notch dimensions as of the last completed handler, for WM_NCHITTEST
    /// arriving while state is in use.
    static HIT_DIMS: std::cell::Cell<Option<NotchDimensions>> = const { std::cell::Cell::new(None) };
}

/// Outcome of a guarded state access.
#[derive(Debug, PartialEq, Eq)]
enum Access<R> {
    Ran(R),
    /// Another handler holds the state (re-entrant call)
    Busy,
    /// No state attached (before creation finished / after destruction)
    Missing,
}

/// Runs `f` with exclusive access to the window's state, or reports why not.
fn with_state<R>(hwnd: HWND, f: impl FnOnce(&mut WindowState) -> R) -> Access<R> {
    if !GUARD.with_borrow_mut(|g| g.enter()) {
        return Access::Busy;
    }
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState };
    let result = if ptr.is_null() {
        Access::Missing
    } else {
        // SAFETY: the guard is held, so this is the only live reference; the
        // allocation cannot be freed until `exit` below (WM_NCDESTROY defers).
        let state = unsafe { &mut *ptr };
        let r = f(state);
        HIT_DIMS.set(Some(state.dimensions));
        Access::Ran(r)
    };
    let (deferred, free) = GUARD.with_borrow_mut(|g| g.exit());
    if free {
        let detached = DETACHED.replace(std::ptr::null_mut());
        if !detached.is_null() {
            // SAFETY: detached by WM_NCDESTROY (Box::into_raw origin), freed once.
            drop(unsafe { Box::from_raw(detached) });
        }
    }
    for (m, w, l) in deferred {
        #[cfg(debug_assertions)]
        eprintln!("[guard] replaying re-entrant message 0x{m:04x}");
        unsafe {
            let _ = PostMessageW(Some(hwnd), m, WPARAM(w), LPARAM(l));
        }
    }
    result
}

/// Guarded handler: on re-entry the message is dropped or replayed per policy.
fn with_state_or_defer(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    f: impl FnOnce(&mut WindowState),
) {
    if with_state(hwnd, f) == Access::Busy
        && let BusyPolicy::Replay(m) = busy_policy(msg)
    {
        GUARD.with_borrow_mut(|g| g.defer(m, wparam.0, lparam.0));
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => {
            // Screen cursor coordinates
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;

            let mut pt = POINT { x, y };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut pt);
            }

            // On the notch as last drawn (its stretched shape while the HUD
            // stretches it, see `Renderer::hits`)
            let (x, y) = (pt.x as f32, pt.y as f32);
            let hit = match with_state(hwnd, |state| {
                state
                    .renderer
                    .hits(x, y)
                    .unwrap_or_else(|| state.dimensions.contains_point(x, y))
            }) {
                Access::Ran(hit) => Some(hit),
                // In use by an outer handler: last completed snapshot
                Access::Busy => HIT_DIMS.get().map(|d| d.contains_point(x, y)),
                Access::Missing => None,
            };
            let is_inside = if let Some(hit) = hit {
                hit
            } else {
                let mut rect = RECT::default();
                unsafe {
                    let _ = GetClientRect(hwnd, &mut rect);
                }
                let w = (rect.right - rect.left) as f32;
                let h = (rect.bottom - rect.top) as f32;
                crate::config::is_point_in_notch_ex(
                    pt.x as f32,
                    pt.y as f32,
                    w,
                    h,
                    crate::config::BASE_COLLAPSED_TOP_TRANSITION_RADIUS,
                    crate::config::BASE_COLLAPSED_TOP_TRANSITION_HEIGHT,
                    crate::config::BASE_COLLAPSED_BOTTOM_CORNER_RADIUS,
                )
            };

            // Transparent hit test passes click directly to the application underneath
            if is_inside {
                LRESULT(HTCLIENT as isize)
            } else {
                LRESULT(HTTRANSPARENT as isize)
            }
        }

        WM_MOUSEACTIVATE => {
            // Prevent the notch from stealing keyboard focus when clicked
            LRESULT(MA_NOACTIVATE as isize)
        }

        WM_INPUT => {
            // Read now: the raw handle is only valid while this message is
            // handled. Only button releases outside the notch matter.
            // While the notch holds the mouse capture (a scrub or a drag on
            // the timer's scale), every button belongs to that gesture: none
            // is an outside click
            if !gesture_owns_buttons(unsafe { GetCapture() }, hwnd) && raw_button_released(lparam) {
                let mut pt = POINT::default();
                let event = unsafe { GetCursorPos(&mut pt) }.ok().and_then(|()| {
                    let under = unsafe { WindowFromPoint(pt) };
                    let root = unsafe { GetAncestor(under, GA_ROOT) };
                    classify_click(root, hwnd, settings_window::handle())
                });
                if let Some(event) = event {
                    unsafe {
                        let _ = PostMessageW(
                            Some(hwnd),
                            WM_APP_GRACE,
                            WPARAM(event as usize),
                            LPARAM(0),
                        );
                    }
                }
            }
            // Required for Raw Input cleanup
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }

        settings_window::WM_APP_SETTINGS_DELAY => {
            // A closing delay chosen in the Settings window: applied by the notch
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                if let Some(d) = crate::config::CollapseDelay::from_index(wparam.0) {
                    state.set_collapse_delay(hwnd, d);
                }
            });
            LRESULT(0)
        }

        WM_APP_UPDATE_READY => {
            // wParam: the check's id; lParam owns a `Box<update::Outcome>`,
            // taken back exactly once, as for a drop's decoded result
            let ptr = lparam.0 as *mut crate::update::Outcome;
            let take = || (!ptr.is_null()).then(|| unsafe { *Box::from_raw(ptr) });
            let ready = |state: &mut WindowState| {
                if let Some(outcome) = take() {
                    state.update_ready(hwnd, wparam.0, outcome);
                }
            };
            match with_state(hwnd, ready) {
                Access::Ran(()) => {}
                Access::Busy => unsafe {
                    if PostMessageW(Some(hwnd), msg, wparam, lparam).is_err() {
                        drop(take());
                    }
                },
                Access::Missing => drop(take()),
            }
            LRESULT(0)
        }

        WM_APP_UPDATE_CLICK => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                state.dismiss_update(hwnd, wparam.0 != 0);
            });
            LRESULT(0)
        }

        WM_APP_UPDATE_CHECK => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                state.check_for_updates(hwnd);
            });
            LRESULT(0)
        }

        settings_window::WM_APP_SETTINGS_UPDATE => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                match settings_window::UpdateRequest::from_index(wparam.0) {
                    Some(settings_window::UpdateRequest::Check) => state.check_for_updates(hwnd),
                    Some(settings_window::UpdateRequest::ViewRelease) => state.open_update(),
                    None => {}
                }
            });
            LRESULT(0)
        }

        WM_APP_DROP_DECODED => {
            // lParam owns a `Box<dragdrop::Decoded>` from `decode_async`: it
            // is taken back exactly once, when applied or when the window
            // is gone; a busy state gets the same message again later
            let ptr = lparam.0 as *mut dragdrop::Decoded;
            let take = || unsafe { *Box::from_raw(ptr) };
            match with_state(hwnd, |state| state.finish_drop(hwnd, take())) {
                Access::Ran(()) => {}
                Access::Busy => unsafe {
                    if PostMessageW(Some(hwnd), msg, wparam, lparam).is_err() {
                        drop(take());
                    }
                },
                Access::Missing => drop(take()),
            }
            LRESULT(0)
        }

        WM_APP_DROP_RESULT_DONE => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                state.end_drop_result(hwnd);
            });
            LRESULT(0)
        }

        WM_APP_TIMER_TICK => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                state.on_timer_tick(hwnd);
            });
            LRESULT(0)
        }

        WM_HOTKEY => {
            if let Some(key) = VolumeKey::from_hotkey(wparam.0) {
                with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                    state.volume_key(hwnd, key);
                });
            }
            LRESULT(0)
        }

        WM_APP_VOLUME_CHANGED => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                state.volume_changed(hwnd);
            });
            LRESULT(0)
        }

        WM_APP_HUD_HIDE => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                state.hide_hud(hwnd);
            });
            LRESULT(0)
        }

        WM_APP_VOLUME_RELEASED => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                state.volume_released(hwnd);
            });
            LRESULT(0)
        }

        WM_APP_POWER_SAVER => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                state.power_saver_changed(hwnd, wparam.0 != 0);
            });
            LRESULT(0)
        }

        WM_MOUSEWHEEL => {
            // Over the timer during setup: one stop per notch of the wheel
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as i16;
                let mut pt = POINT {
                    x: (lparam.0 & 0xFFFF) as i16 as i32,
                    y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                };
                unsafe {
                    let _ = ScreenToClient(hwnd, &mut pt);
                }
                let (cx, cy) = state.renderer.content_offset();
                pt.x -= cx as i32;
                pt.y -= cy as i32;
                let over = state
                    .timer_layout()
                    .is_some_and(|l| l.panel.contains(pt.x as f32, pt.y as f32));
                let from = state.shown_scale();
                let now = std::time::Instant::now();
                if over && delta != 0 && state.timer.step(delta.signum().into(), now) {
                    state.timer_time_set(hwnd, from);
                    state.persist_timer_durations();
                }
            });
            LRESULT(0)
        }

        WM_APP_GRACE => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                if let Some(event) = GraceEvent::ALL.get(wparam.0) {
                    state.grace(hwnd, *event);
                }
            });
            LRESULT(0)
        }

        WM_APP_ANIM_FRAME => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                state.frame_clock.frame_received();
                // A frame queued just before the frames stopped draws nothing
                if state.animation.is_some() || state.last_frame_time.is_some() {
                    animation_frame(hwnd, state);
                }
            });
            LRESULT(0)
        }

        WM_TIMER => {
            if wparam.0 == AUTO_COLLAPSE_TIMER_ID {
                // One-shot; applied as a message so a busy state replays it
                // (timer messages would be dropped)
                unsafe {
                    let _ = KillTimer(Some(hwnd), AUTO_COLLAPSE_TIMER_ID);
                    let _ = PostMessageW(
                        Some(hwnd),
                        WM_APP_GRACE,
                        WPARAM(GraceEvent::TimerFired as usize),
                        LPARAM(0),
                    );
                }
                return LRESULT(0);
            }
            if wparam.0 == DROP_RESULT_TIMER_ID {
                // One-shot, applied as a message so a busy state replays it
                unsafe {
                    let _ = KillTimer(Some(hwnd), DROP_RESULT_TIMER_ID);
                    let _ = PostMessageW(Some(hwnd), WM_APP_DROP_RESULT_DONE, WPARAM(0), LPARAM(0));
                }
                return LRESULT(0);
            }
            if wparam.0 == HUD_HIDE_TIMER_ID || wparam.0 == HUD_RELEASE_TIMER_ID {
                // One-shots, applied as messages (a busy state replays them)
                let posted = if wparam.0 == HUD_HIDE_TIMER_ID {
                    WM_APP_HUD_HIDE
                } else {
                    WM_APP_VOLUME_RELEASED
                };
                unsafe {
                    let _ = KillTimer(Some(hwnd), wparam.0);
                    let _ = PostMessageW(Some(hwnd), posted, WPARAM(0), LPARAM(0));
                }
                return LRESULT(0);
            }
            if wparam.0 == crate::update::CHECK_TIMER_ID {
                // The next check a day later (failures wait for it too);
                // this one through the shared state (joins a running one)
                unsafe {
                    let _ = SetTimer(
                        Some(hwnd),
                        crate::update::CHECK_TIMER_ID,
                        crate::update::RECHECK_MS,
                        None,
                    );
                    let _ = PostMessageW(Some(hwnd), WM_APP_UPDATE_CHECK, WPARAM(0), LPARAM(0));
                }
                return LRESULT(0);
            }
            if wparam.0 == crate::update::HIDE_TIMER_ID {
                unsafe {
                    let _ = KillTimer(Some(hwnd), wparam.0);
                    let _ = PostMessageW(Some(hwnd), WM_APP_UPDATE_CLICK, WPARAM(0), LPARAM(0));
                }
                return LRESULT(0);
            }
            if wparam.0 == TIMER_TICK_TIMER_ID {
                // Applied as a message so a busy state replays it (timer
                // messages would be dropped); re-armed by its handler
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_TICK_TIMER_ID);
                    let _ = PostMessageW(Some(hwnd), WM_APP_TIMER_TICK, WPARAM(0), LPARAM(0));
                }
                return LRESULT(0);
            }
            if wparam.0 == ANIMATION_TIMER_ID {
                // Fallback frames (no thread-pool timer, see `FrameClock`)
                with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                    animation_frame(hwnd, state);
                });
            } else if wparam.0 == MEDIA_FEEDBACK_TIMER_ID {
                with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                    let now = std::time::Instant::now();
                    let dt = state
                        .feedback_tick
                        .replace(now)
                        .map_or(MEDIA_FEEDBACK_FRAME_MS as f32, |t| {
                            now.duration_since(t).as_secs_f32() * 1000.0
                        });
                    // Control feedback and the Settings page's switches share
                    // this timer (both step every tick)
                    let feedback = state.renderer.feedback().step(dt);
                    let switches = state.renderer.step_switches(dt);
                    // ...transport glyphs and the artwork turn...
                    let media = state.renderer.step_media_motion(dt);
                    // ...and the countdown's bubble / slot / done pulse
                    state
                        .bubble_presence
                        .step_on(dt, crate::timer::ABSORB_SPRING);
                    state.slot_presence.step(dt);
                    state.update_presence.step(dt);
                    state.update_icon.step(dt);
                    state.renderer.set_update_icon(state.update_icon.level());
                    state.push_timer_view();
                    let animating = feedback
                        || switches
                        || media
                        || state.timer_animating()
                        || state.update_presence.is_animating()
                        || state.update_icon.is_animating();
                    state.redraw(hwnd);
                    state.update_bubble(hwnd);
                    state.draw_update_notice(hwnd);
                    if !animating {
                        state.feedback_tick = None;
                        unsafe {
                            let _ = KillTimer(Some(hwnd), MEDIA_FEEDBACK_TIMER_ID);
                        }
                    }
                });
            } else if wparam.0 == MEDIA_LIVE_TIMER_ID {
                with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                    let now = std::time::Instant::now();
                    let dt = state
                        .live_tick
                        .replace(now)
                        .map_or(MEDIA_LIVE_FRAME_MS as f32, |t| {
                            now.duration_since(t).as_secs_f32() * 1000.0
                        });
                    let active = state.renderer.visualizer().step(dt);
                    let visible = state.live_visible();
                    // During a notch animation its own frames show the motion
                    if visible {
                        state.redraw(hwnd);
                    } else {
                        state.renderer.visualizer().settle();
                    }
                    if !active || !visible {
                        state.live_tick = None;
                        unsafe {
                            let _ = KillTimer(Some(hwnd), MEDIA_LIVE_TIMER_ID);
                        }
                    } else {
                        // Keep the rate matched to what is on screen
                        state.kick_live(hwnd);
                    }
                });
            } else if wparam.0 == CLIPBOARD_TIMER_ID {
                unsafe {
                    let _ = KillTimer(Some(hwnd), CLIPBOARD_TIMER_ID);
                }
                with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                    if clipboard::owned_by(hwnd) {
                        return;
                    }
                    // Unavailable / unsupported / too large: skipped, never retried
                    if let Ok(item) = clipboard::read(hwnd) {
                        let _outcome = state.clipboard.add(item);
                        if _outcome == clipboard::AddOutcome::Added {
                            state.clipboard_changed(hwnd);
                        }
                        // Kinds and counts only, never contents
                        #[cfg(debug_assertions)]
                        eprintln!(
                            "[clipboard] {:?}, {} entries, {} image bytes",
                            _outcome,
                            state.clipboard.len(),
                            state.clipboard.image_bytes()
                        );
                    }
                });
            } else if wparam.0 == CLOCK_TIMER_ID {
                with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                    let changed = state.clock.refresh();
                    if changed {
                        notify_accessibility_clock_changed(
                            hwnd,
                            state.state,
                            state.clock.state(),
                            state.media_content.as_ref(),
                            state.space,
                            &state.clipboard,
                            state.clear_pending,
                            state.settings_title(),
                            state.timer_title().as_deref(),
                        );
                        if state.animation.is_none() {
                            let _ = state.renderer.render(
                                hwnd,
                                state.pos_x,
                                state.pos_y,
                                &state.dimensions,
                                state.hovered,
                                state.clock.state(),
                            );
                        }
                    }
                    let next_ms = state.clock.ms_until_next_minute();
                    unsafe {
                        let _ = SetTimer(Some(hwnd), CLOCK_TIMER_ID, next_ms, None);
                    }
                });
            }
            LRESULT(0)
        }

        WM_MOUSEMOVE => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                if state.timer_drag.is_some() {
                    state.drag_timer_to(hwnd, lparam);
                }
                if state.scrub.is_some() {
                    state.move_scrub(hwnd, lparam);
                }
                if !state.hovered {
                    state.hovered = true;
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut tme);
                    }
                    if state.animation.is_none() {
                        let _ = state.renderer.render(
                            hwnd,
                            state.pos_x,
                            state.pos_y,
                            &state.dimensions,
                            true,
                            state.clock.state(),
                        );
                    }
                }
                // Control-local hover: fades only when the hovered control changes
                let control = state.media_control_at(lparam);
                if state.renderer.set_hovered_control(control) {
                    state.kick_feedback(hwnd);
                }
                // Clipboard: the hovered row shows its buttons; a hovered button grows
                let hit = state.panel_hit(lparam);
                let row = hit.and_then(|h| match h {
                    PanelHit::Row(i) | PanelHit::Copy(i) | PanelHit::Remove(i) => Some(i),
                    PanelHit::Clear
                    | PanelHit::CancelClear
                    | PanelHit::ConfirmClear
                    | PanelHit::Settings
                    | PanelHit::AlwaysOnTop
                    | PanelHit::ReducedMotion
                    | PanelHit::FullSettings
                    | PanelHit::TimerPrimary
                    | PanelHit::TimerCancel
                    | PanelHit::TimerScale
                    | PanelHit::TimerMode(_) => None,
                });
                let changed = {
                    let mut feedback = state.renderer.feedback();
                    let row_changed = feedback.set_clip_row(row);
                    feedback.set_clip_hovered(hit.filter(|h| h.is_button())) || row_changed
                };
                if changed {
                    state.kick_feedback(hwnd);
                }
            });
            LRESULT(0)
        }

        WM_MOUSELEAVE => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                let left_clip = {
                    let mut feedback = state.renderer.feedback();
                    let released = feedback.release_clip();
                    let row_changed = feedback.set_clip_row(None);
                    feedback.set_clip_hovered(None) || released || row_changed
                };
                if state.renderer.set_hovered_control(None) || left_clip {
                    state.kick_feedback(hwnd);
                }
                if state.hovered {
                    state.hovered = false;
                    if state.animation.is_none() {
                        let _ = state.renderer.render(
                            hwnd,
                            state.pos_x,
                            state.pos_y,
                            &state.dimensions,
                            false,
                            state.clock.state(),
                        );
                    }
                }
            });
            LRESULT(0)
        }

        WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            // Using the notch: a pending auto-collapse is cancelled
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                state.grace(hwnd, GraceEvent::OecaInteraction);
            });
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }

        WM_LBUTTONDOWN => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                // Cancel a pending auto-collapse first, then the click as
                // always (a collapsing click still collapses)
                state.grace(hwnd, GraceEvent::OecaInteraction);
                if let Some(f) = state.scrub_at(lparam) {
                    // The scrubber: jump there, then follow the drag
                    state.begin_scrub(hwnd, f);
                } else if let Some(control) = state.media_control_at(lparam) {
                    // Tactile compression; capture so the release is always seen
                    state.renderer.feedback().press(control);
                    unsafe {
                        SetCapture(hwnd);
                    }
                    // Press compresses smoothly on the shared feedback timer
                    state.kick_feedback(hwnd);
                } else if state.panel_hit(lparam) == Some(PanelHit::TimerScale) {
                    // The timer's scale: drag it (or tap a stop)
                    state.begin_timer_drag(lparam);
                    unsafe {
                        SetCapture(hwnd);
                    }
                } else if let Some(button) = state.panel_hit(lparam).filter(|h| h.is_button()) {
                    // Clipboard icon button: grows a little more while pressed
                    state.renderer.feedback().press_clip(button);
                    state.kick_feedback(hwnd);
                }
            });
            LRESULT(0)
        }

        WM_LBUTTONUP => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                // A pressed clipboard button eases back (its action runs below)
                if state.renderer.feedback().release_clip() {
                    state.kick_feedback(hwnd);
                }
                // A scrub ends: seek where it was released (inside the
                // track or not); never toggles the notch
                if state.scrub.is_some() {
                    state.end_scrub(hwnd, true);
                    return;
                }
                // A drag along the timer's scale ends (never toggles the notch)
                if state.timer_drag.is_some() {
                    state.end_timer_drag(hwnd, Some(lparam));
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    return;
                }
                let pressed = state.renderer.feedback().release();
                if let Some(pressed) = pressed {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    // Press started on a control: never toggles the notch. The command
                    // fires only if released over the same control.
                    if state.media_control_at(lparam) == Some(pressed)
                        && state.media.send_command(pressed)
                    {
                        state.control_clicked(hwnd, pressed);
                    }
                    state.kick_feedback(hwnd);
                } else if let Some(control) = state.media_control_at(lparam) {
                    // Transport control: send to the current session; never toggles the notch.
                    // Resulting playback/metadata state arrives through media events.
                    if state.media.send_command(control) {
                        state.control_clicked(hwnd, control);
                    }
                } else if state.scrub_at(lparam).is_some() {
                    // On the scrubber of an app that does not allow seeking:
                    // still not a click on the notch's background
                } else if let Some(hit) = state.panel_hit(lparam) {
                    // Settings icon: open / close the page. Switch: apply
                    // Always on top. Clipboard row: back onto the system
                    // clipboard and to the front (no duplicate). Clear: forget
                    // the in-memory history only. None toggles the notch.
                    match hit {
                        PanelHit::Settings => {
                            let open = !state.settings_open;
                            state.set_settings_open(hwnd, open);
                            return;
                        }
                        PanelHit::AlwaysOnTop => {
                            state.toggle_always_on_top(hwnd);
                            return;
                        }
                        PanelHit::ReducedMotion => {
                            state.toggle_reduced_motion(hwnd);
                            return;
                        }
                        PanelHit::FullSettings => {
                            state.open_full_settings(hwnd);
                            return;
                        }
                        PanelHit::TimerPrimary => {
                            state.timer_primary(hwnd);
                            return;
                        }
                        PanelHit::TimerCancel => {
                            state.timer_cancel(hwnd);
                            return;
                        }
                        PanelHit::TimerScale => return,
                        PanelHit::TimerMode(mode) => {
                            state.timer_mode(hwnd, mode);
                            return;
                        }
                        PanelHit::Row(index) | PanelHit::Copy(index) => {
                            // Clipboard busy: nothing changes; no retry
                            let _ = state.restore_clipboard(hwnd, index);
                        }
                        PanelHit::Remove(index) => {
                            state.clipboard.remove(index);
                        }
                        PanelHit::Clear | PanelHit::CancelClear | PanelHit::ConfirmClear => {
                            let mut pending = state.clear_pending;
                            clear_flow(
                                hit,
                                state.settings.confirm_clear_clipboard,
                                &mut pending,
                                &mut state.clipboard,
                            );
                            state.set_clear_pending(hwnd, pending);
                        }
                    }
                    state.clipboard_changed(hwnd);
                } else if let Some(space) = state.space_at(lparam) {
                    // Space capsule: switches spaces; never toggles the notch
                    state.select_space(hwnd, space);
                } else if state.on_timer_panel(lparam) {
                    // Between the timer's controls: its panel, not the
                    // notch's background
                } else if state.update_icon_at(lparam) {
                    // The collapsed notch's update icon: the release page;
                    // never toggles the notch
                    state.open_update();
                } else {
                    state.reset_feedback(hwnd);
                    let _ = start_or_reverse_animation(hwnd, state);
                }
            });
            LRESULT(0)
        }

        WM_CAPTURECHANGED => {
            // Capture lost mid-press (e.g. another window took it): cancel the press
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                if state.timer_drag.is_some() {
                    state.end_timer_drag(hwnd, None);
                }
                // A scrub whose capture was taken away seeks nowhere
                state.end_scrub(hwnd, false);
                let cancelled = state.renderer.feedback().release().is_some();
                if cancelled {
                    state.kick_feedback(hwnd);
                }
            });
            LRESULT(0)
        }

        WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }

        WM_DPICHANGED | WM_APP_DEFERRED_DPICHANGED => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                unsafe {
                    let new_dpi = (wparam.0 & 0xFFFF) as u32;
                    let new_dpi = if new_dpi > 0 {
                        new_dpi
                    } else {
                        GetDpiForWindow(hwnd)
                    };
                    let new_dims = state.settled_dims(state.state, new_dpi);

                    let screen_width = GetSystemMetrics(SM_CXSCREEN);
                    let new_x = calculate_notch_x(screen_width, new_dims.width);
                    let new_y = 0;

                    // Reconfigure rendering only when dimensions or position actually change
                    if state.dimensions != new_dims || state.pos_x != new_x || state.pos_y != new_y
                    {
                        state.dimensions = new_dims;
                        state.pos_x = new_x;
                        state.pos_y = new_y;

                        let _ = state.renderer.render(
                            hwnd,
                            new_x,
                            new_y,
                            &new_dims,
                            state.hovered,
                            state.clock.state(),
                        );
                        let _ = SetWindowPos(
                            hwnd,
                            z_order(state.settings.always_on_top).0,
                            new_x,
                            new_y,
                            new_dims.width,
                            new_dims.height,
                            SWP_NOACTIVATE
                                | SWP_SHOWWINDOW
                                | z_order(state.settings.always_on_top).1,
                        );
                        // The countdown bubble follows the notch's new size
                        state.update_bubble(hwnd);
                    }
                }
            });
            LRESULT(0)
        }

        WM_DISPLAYCHANGE => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                unsafe {
                    let dpi = GetDpiForWindow(hwnd);
                    let dpi = if dpi > 0 { dpi } else { GetDpiForSystem() };
                    let new_dims = state.settled_dims(state.state, dpi);

                    let screen_width = GetSystemMetrics(SM_CXSCREEN);
                    let new_x = calculate_notch_x(screen_width, new_dims.width);
                    let new_y = 0;

                    // Reconfigure rendering only when dimensions or position actually change
                    if state.dimensions != new_dims || state.pos_x != new_x || state.pos_y != new_y
                    {
                        state.dimensions = new_dims;
                        state.pos_x = new_x;
                        state.pos_y = new_y;

                        let _ = state.renderer.render(
                            hwnd,
                            new_x,
                            new_y,
                            &new_dims,
                            state.hovered,
                            state.clock.state(),
                        );
                        let _ = SetWindowPos(
                            hwnd,
                            z_order(state.settings.always_on_top).0,
                            new_x,
                            new_y,
                            new_dims.width,
                            new_dims.height,
                            SWP_NOACTIVATE
                                | SWP_SHOWWINDOW
                                | z_order(state.settings.always_on_top).1,
                        );
                        // The countdown bubble follows the notch's new size
                        state.update_bubble(hwnd);
                    }
                }
            });
            LRESULT(0)
        }

        WM_TIMECHANGE => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                let changed = state.clock.refresh();
                if changed {
                    notify_accessibility_clock_changed(
                        hwnd,
                        state.state,
                        state.clock.state(),
                        state.media_content.as_ref(),
                        state.space,
                        &state.clipboard,
                        state.clear_pending,
                        state.settings_title(),
                        state.timer_title().as_deref(),
                    );
                    if state.animation.is_none() {
                        let _ = state.renderer.render(
                            hwnd,
                            state.pos_x,
                            state.pos_y,
                            &state.dimensions,
                            state.hovered,
                            state.clock.state(),
                        );
                    }
                }
                let next_ms = state.clock.ms_until_next_minute();
                unsafe {
                    let _ = SetTimer(Some(hwnd), CLOCK_TIMER_ID, next_ms, None);
                }
            });
            LRESULT(0)
        }

        WM_POWERBROADCAST => {
            let event = wparam.0 as u32;
            if event == PBT_POWERSETTINGCHANGE {
                // lParam is only valid now: read it, then apply as a message
                if let Some((saver, on)) = unsafe { saver_report(lparam) } {
                    use std::sync::atomic::Ordering::Relaxed;
                    let before = power_saver_active();
                    let seen = saver.seen().swap(true, Relaxed);
                    saver.flag().store(on, Relaxed);
                    let after = power_saver_active();
                    // Only a real change of the effective state (not the
                    // first report, not the other saver agreeing) is shown
                    if after != before {
                        let notice = crate::hud::power_notice_due(seen, before, after);
                        unsafe {
                            let _ = PostMessageW(
                                Some(hwnd),
                                WM_APP_POWER_SAVER,
                                WPARAM(usize::from(notice)),
                                LPARAM(0),
                            );
                        }
                    }
                }
                return LRESULT(1);
            }
            if event == PBT_APMRESUMEAUTOMATIC || event == PBT_APMRESUMESUSPEND {
                with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                    let changed = state.clock.refresh();
                    if changed {
                        notify_accessibility_clock_changed(
                            hwnd,
                            state.state,
                            state.clock.state(),
                            state.media_content.as_ref(),
                            state.space,
                            &state.clipboard,
                            state.clear_pending,
                            state.settings_title(),
                            state.timer_title().as_deref(),
                        );
                        if state.animation.is_none() {
                            let _ = state.renderer.render(
                                hwnd,
                                state.pos_x,
                                state.pos_y,
                                &state.dimensions,
                                state.hovered,
                                state.clock.state(),
                            );
                        }
                    }
                    let next_ms = state.clock.ms_until_next_minute();
                    unsafe {
                        let _ = SetTimer(Some(hwnd), CLOCK_TIMER_ID, next_ms, None);
                    }
                    // A countdown that ran out while asleep finishes now
                    state.on_timer_tick(hwnd);
                    // Media sessions may have changed while asleep: re-query once
                    // through the normal event messages (generation-protected).
                    for media_msg in [
                        WM_APP_MEDIA_SESSION_CHANGED,
                        WM_APP_MEDIA_PLAYBACK_CHANGED,
                        WM_APP_MEDIA_PROPERTIES_CHANGED,
                    ] {
                        unsafe {
                            let _ = PostMessageW(Some(hwnd), media_msg, WPARAM(0), LPARAM(0));
                        }
                    }
                });
            }
            LRESULT(1)
        }

        // Private, in-process only: posted by MediaEngine's WinRT callbacks (and
        // once on resume). Redraws only when the visible media content changes.
        _ if is_media_message(msg) => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                if state.media.handle_message(hwnd, msg) {
                    let content = MediaContent::from_state(state.media.state());
                    if content != state.media_content {
                        // Artwork/source-name updates don't change what is announced
                        let spoken = |c: &Option<MediaContent>| {
                            c.as_ref().map(|c| (c.accessible_text(), c.icon))
                        };
                        let a11y_changed = spoken(&content) != spoken(&state.media_content);
                        let app =
                            |c: &Option<MediaContent>| c.as_ref().map(|c| c.source.app_id.clone());
                        if app(&content) != app(&state.media_content) {
                            state.end_scrub(hwnd, false);
                        }
                        state.media_content = content;
                        let changed = state.renderer.set_media(state.media_content.as_ref());
                        // Play/pause starts the visualizer fade in either notch state
                        state.kick_live(hwnd);
                        // ...and moves a collapsed countdown between the
                        // bubble and the visualizer's slot (never resets it)
                        state.refresh_timer_presence(hwnd);
                        // Media is only visible in the settled expanded state; an in-flight
                        // animation picks the new content up on its next frame.
                        let visible =
                            state.state == NotchState::Expanded && state.animation.is_none();
                        if changed && visible {
                            state.redraw(hwnd);
                            // A new track fades in on the shared feedback timer
                            state.kick_feedback(hwnd);
                        } else if !visible {
                            // Off-screen track changes never run a fade (nor
                            // a glyph morph or an artwork turn)
                            state.renderer.feedback().finish_track_fade();
                            state.renderer.settle_media_motion();
                        }
                        // The collapsed notch's album art: fades on the
                        // feedback timer where that notch shows, else
                        // settles so it never reappears mid-fade
                        if state.state == NotchState::Collapsed || state.animation.is_some() {
                            if changed {
                                state.redraw(hwnd);
                                state.kick_feedback(hwnd);
                            }
                        } else {
                            state.renderer.settle_collapsed_art();
                        }
                        // A scrub ends (unsought) if its timeline went away
                        // or another app's session took over
                        if state.scrub.is_some() && state.scrub_track().is_none() {
                            state.end_scrub(hwnd, false);
                        }
                        if a11y_changed
                            && state.state == NotchState::Expanded
                            && state.animation.is_none()
                        {
                            notify_accessibility_clock_changed(
                                hwnd,
                                state.state,
                                state.clock.state(),
                                state.media_content.as_ref(),
                                state.space,
                                &state.clipboard,
                                state.clear_pending,
                                state.settings_title(),
                                state.timer_title().as_deref(),
                            );
                        }
                    }
                }
            });
            LRESULT(0)
        }

        WM_NCDESTROY => {
            // Detach the state atomically (later messages see Missing). If a handler
            // up the stack is still using it (destruction re-entered that handler),
            // the free is deferred until that handler returns.
            let ptr = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut WindowState };
            if !ptr.is_null() {
                if GUARD.with_borrow_mut(|g| g.request_free()) {
                    // SAFETY: Box::into_raw origin, detached above, no live borrow.
                    drop(unsafe { Box::from_raw(ptr) });
                } else {
                    DETACHED.set(ptr);
                }
            }
            HIT_DIMS.set(None);
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }

        WM_CLIPBOARDUPDATE => {
            // Oeca's own restore: the history already holds (and fronts) it.
            // Otherwise read once the copying app has settled (one-shot timer;
            // re-arming coalesces a burst of updates into a single read).
            if !clipboard::owned_by(hwnd) {
                unsafe {
                    let _ = SetTimer(Some(hwnd), CLIPBOARD_TIMER_ID, CLIPBOARD_SETTLE_MS, None);
                }
            }
            LRESULT(0)
        }

        settings_window::WM_APP_SETTINGS_TOGGLE => {
            // A switch in the Settings window: applied by the notch (the owner)
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                if let Some(control) = settings_window::Control::from_index(wparam.0) {
                    state.apply_settings_window_toggle(hwnd, control);
                }
            });
            LRESULT(0)
        }

        settings_window::WM_APP_SETTINGS_ACCENT => {
            // An accent swatch in the Settings window: applied by the notch
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                if let Some(accent) = crate::config::AccentChoice::from_index(wparam.0) {
                    state.set_accent(hwnd, accent);
                }
            });
            LRESULT(0)
        }

        settings_window::WM_APP_SETTINGS_CAPACITY => {
            // A history capacity in the Settings window: applied by the notch
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                if let Some(c) = crate::config::ClipboardCapacity::from_index(wparam.0) {
                    state.set_clipboard_capacity(hwnd, c);
                }
            });
            LRESULT(0)
        }

        WM_DESTROY => {
            // The Settings window shuts down with Oeca
            settings_window::close();
            unsafe {
                let _ = KillTimer(Some(hwnd), ANIMATION_TIMER_ID);
                let _ = KillTimer(Some(hwnd), CLOCK_TIMER_ID);
                let _ = KillTimer(Some(hwnd), MEDIA_FEEDBACK_TIMER_ID);
                let _ = KillTimer(Some(hwnd), MEDIA_LIVE_TIMER_ID);
                let _ = KillTimer(Some(hwnd), CLIPBOARD_TIMER_ID);
                let _ = KillTimer(Some(hwnd), AUTO_COLLAPSE_TIMER_ID);
                let _ = KillTimer(Some(hwnd), TIMER_TICK_TIMER_ID);
                let _ = KillTimer(Some(hwnd), DROP_RESULT_TIMER_ID);
                let _ = KillTimer(Some(hwnd), HUD_HIDE_TIMER_ID);
                let _ = KillTimer(Some(hwnd), HUD_RELEASE_TIMER_ID);
                PostQuitMessage(0);
            }
            // Stop reporting outside clicks (no-op if never registered)
            watch_outside_clicks(hwnd, false);
            clipboard::stop_listening(hwnd);
            // Releases OLE's reference to the drop target (no-op if never registered)
            unsafe {
                let _ = RevokeDragDrop(hwnd);
            }
            // Release a resolution request still held by an in-flight animation
            // (if the state is busy, HighResTimer's Drop releases it on free).
            // The countdown bubble goes too (owned by the notch, it would be
            // destroyed with it anyway)
            crate::hud::release_volume_keys(hwnd);
            let _ = with_state(hwnd, |state| {
                state.stop_frames(hwnd);
                state.tray.remove();
                state.volume = None;
                for watch in state.saver_watch.iter_mut().filter_map(Option::take) {
                    unsafe {
                        let _ = windows::Win32::System::Power::UnregisterPowerSettingNotification(
                            watch,
                        );
                    }
                }
                if let Some(bubble) = state.bubble.take() {
                    unsafe {
                        let _ = DestroyWindow(bubble);
                    }
                }
            });
            LRESULT(0)
        }

        // Windows is shutting down, restarting or signing the user out: drop
        // the saved history now (best effort; a new session never loads it
        // anyway). Not when an installer's Restart Manager only closes Oeca.
        WM_ENDSESSION => {
            if wparam.0 != 0 && lparam.0 as u32 & ENDSESSION_CLOSEAPP == 0 {
                crate::clipboard_store::wipe_for_session_end();
            }
            LRESULT(0)
        }

        crate::tray::WM_APP_TRAY => {
            match crate::tray::decode(wparam, lparam) {
                // Shown without holding the state: the menu's modal loop keeps
                // the notch's own messages flowing while it is open
                Some(crate::tray::Event::Menu { x, y }) => crate::tray::show_menu(hwnd, x, y),
                Some(crate::tray::Event::Activate) => {
                    with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                        state.open_from_tray(hwnd)
                    })
                }
                None => {}
            }
            LRESULT(0)
        }

        WM_COMMAND => match crate::tray::Command::from_id(wparam.0 & 0xFFFF) {
            Some(crate::tray::Command::Open) => {
                with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                    state.open_from_tray(hwnd)
                });
                LRESULT(0)
            }
            Some(crate::tray::Command::Settings) => {
                with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                    state.open_full_settings(hwnd)
                });
                LRESULT(0)
            }
            // The usual shutdown: WM_CLOSE -> DestroyWindow -> WM_DESTROY
            // (tray icon, Settings window, timers) -> WM_QUIT
            Some(crate::tray::Command::Quit) => {
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                LRESULT(0)
            }
            None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        },

        // Explorer (re)started: every tray icon is gone; add ours again
        _ if msg == crate::tray::taskbar_created() => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                state.tray.restore(hwnd);
            });
            LRESULT(0)
        }

        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// The countdown bubble's window class: it only shows what the notch renders
/// into it, so it has no state of its own.
const BUBBLE_CLASS_NAME: windows::core::PCWSTR = windows::core::w!("OecaTimerBubbleClass");
const UPDATE_CLASS_NAME: windows::core::PCWSTR = windows::core::w!("OecaUpdateNoticeClass");

/// The update notice: never activated; a click (on the pill: its
/// transparent surroundings pass clicks through) opens the release, a
/// right-click dismisses it. Both are posted to the notch.
unsafe extern "system" fn update_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let post = |open: usize| unsafe {
        if let Ok(owner) = GetWindow(hwnd, GW_OWNER) {
            let _ = PostMessageW(Some(owner), WM_APP_UPDATE_CLICK, WPARAM(open), LPARAM(0));
        }
    };
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_SETCURSOR => unsafe {
            if let Ok(hand) = LoadCursorW(None, IDC_HAND) {
                SetCursor(Some(hand));
            }
            LRESULT(1)
        },
        WM_LBUTTONUP => {
            post(1);
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            post(0);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Creates the (hidden) update notice: a layered popup that never
/// activates, owned by the notch so it can never outlive it; `title` is
/// what screen readers say for it.
fn create_update_notice(owner: HWND, title: &str) -> Option<HWND> {
    let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
    unsafe {
        let hinstance = GetModuleHandleW(None).ok()?;
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(update_wndproc),
            hInstance: hinstance.into(),
            lpszClassName: UPDATE_CLASS_NAME,
            ..Default::default()
        };
        // Already registered (an earlier notice): fails harmlessly
        let _ = RegisterClassExW(&wc);
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            UPDATE_CLASS_NAME,
            windows::core::PCWSTR(title.as_ptr()),
            WS_POPUP,
            0,
            0,
            1,
            1,
            Some(owner),
            None,
            Some(hinstance.into()),
            None,
        )
        .ok()
    }
}

unsafe extern "system" fn bubble_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // Click-through and never activated: it must never take input
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Creates the (hidden) countdown bubble: a layered, click-through popup
/// that never activates, owned by the notch so it can never outlive it.
fn create_bubble(owner: HWND) -> Option<HWND> {
    unsafe {
        let hinstance = GetModuleHandleW(None).ok()?;
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(bubble_wndproc),
            hInstance: hinstance.into(),
            lpszClassName: BUBBLE_CLASS_NAME,
            ..Default::default()
        };
        // Already registered (an earlier bubble): fails harmlessly
        let _ = RegisterClassExW(&wc);
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            BUBBLE_CLASS_NAME,
            windows::core::w!("Oeca timer"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            Some(owner),
            None,
            Some(hinstance.into()),
            None,
        )
        .ok()
    }
}

/// OLE drop-target entry point (UI thread, from the message loop): routes the
/// event through the same guarded `WindowState` access as the window procedure.
/// Busy or missing state (re-entry, teardown) declines the drop.
fn handle_drag(hwnd: HWND, event: DragEvent) -> bool {
    match with_state(hwnd, |state| state.on_drag(hwnd, event)) {
        Access::Ran(accept) => accept,
        Access::Busy | Access::Missing => false,
    }
}

pub fn run() -> Result<()> {
    // One Oeca per sign-in session: a second launch (Start on launch plus a
    // manual start, a double click) opens the running one and exits, so
    // there is never a second notch or tray icon
    let Some(_instance) = SingleInstance::acquire() else {
        SingleInstance::open_existing();
        return Ok(());
    };
    // OLE drag-and-drop needs this (UI) thread in a single-threaded apartment.
    // Failure is non-fatal: Oeca simply runs without being a drop target.
    let ole = unsafe { OleInitialize(None) }.is_ok();
    let result = run_window(ole);
    if ole {
        unsafe { OleUninitialize() };
    }
    result
}

/// The per-session single-instance mutex, held for the process's life.
struct SingleInstance(windows::Win32::Foundation::HANDLE);

impl SingleInstance {
    const NAME: windows::core::PCWSTR = windows::core::w!("Local\\Oeca.SingleInstance");

    /// The mutex if this is the first Oeca in the session; None if another
    /// one already holds it. If the mutex cannot be made at all, Oeca runs
    /// (as before) rather than refusing to start.
    fn acquire() -> Option<Self> {
        use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
        let handle =
            unsafe { windows::Win32::System::Threading::CreateMutexW(None, false, Self::NAME) };
        match handle {
            Ok(h) if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS => {
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(h);
                }
                None
            }
            Ok(h) => Some(Self(h)),
            Err(_) => Some(Self(windows::Win32::Foundation::HANDLE::default())),
        }
    }

    /// Asks the running Oeca to open (as its tray's Open Oeca does).
    fn open_existing() {
        unsafe {
            if let Ok(hwnd) =
                windows::Win32::UI::WindowsAndMessaging::FindWindowW(WINDOW_CLASS_NAME, None)
            {
                let _ = PostMessageW(
                    Some(hwnd),
                    WM_COMMAND,
                    WPARAM(crate::tray::Command::Open as usize),
                    LPARAM(0),
                );
            }
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

fn run_window(ole: bool) -> Result<()> {
    unsafe {
        // Enable Per-Monitor V2 DPI awareness for sharp native rendering
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let hinstance = GetModuleHandleW(None)?;

        // 1. Register Window Class
        let cursor = LoadCursorW(None, IDC_ARROW)?;
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance.into(),
            hIcon: windows::Win32::UI::WindowsAndMessaging::HICON::default(),
            hCursor: cursor,
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH::default(),
            lpszMenuName: windows::core::PCWSTR::null(),
            lpszClassName: WINDOW_CLASS_NAME,
            hIconSm: windows::Win32::UI::WindowsAndMessaging::HICON::default(),
        };

        let atom = RegisterClassExW(&wc);
        if atom == 0 {
            return Err(Error::from_thread());
        }

        // 2. Compute initial notch position and dimensions from monitor DPI
        let system_dpi = GetDpiForSystem();
        let dimensions = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, system_dpi);
        let screen_width = GetSystemMetrics(SM_CXSCREEN);
        let x = calculate_notch_x(screen_width, dimensions.width);
        let y = 0;

        // 3. Create Borderless, Layered, Topmost, Toolwindow
        // Saved settings first, so the notch never shows with the defaults.
        // Start on launch mirrors the real registration (it may have been
        // removed elsewhere, e.g. in Task Manager)
        let mut settings_store = crate::settings_store::SettingsStore::open();
        let mut settings = settings_store.load().unwrap_or_default();
        settings.start_on_launch = crate::startup::is_enabled();
        let always_on_top = settings.always_on_top;
        let ex_style = if always_on_top {
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE
        } else {
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE
        };
        let hwnd = CreateWindowExW(
            ex_style,
            WINDOW_CLASS_NAME,
            WINDOW_TITLE,
            WS_POPUP,
            x,
            y,
            dimensions.width,
            dimensions.height,
            None,
            None,
            Some(hinstance.into()),
            None,
        )?;

        if hwnd.is_invalid() {
            return Err(Error::from_thread());
        }

        // 4. Verify DPI with specific window handle and re-align if necessary
        let window_dpi = GetDpiForWindow(hwnd);
        let dimensions = if window_dpi > 0 && window_dpi != system_dpi {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, window_dpi);
            let new_x = calculate_notch_x(screen_width, dims.width);
            let _ = SetWindowPos(
                hwnd,
                Some(z_band(settings.always_on_top)),
                new_x,
                y,
                dims.width,
                dims.height,
                SWP_NOACTIVATE,
            );
            dims
        } else {
            dimensions
        };
        let x = calculate_notch_x(screen_width, dimensions.width);

        // 5. Initialize renderer and attach WindowState to HWND
        let renderer = match Renderer::new() {
            Ok(r) => r,
            Err(e) => {
                let _ = DestroyWindow(hwnd);
                return Err(e);
            }
        };

        let clock = ClockEngine::now();
        let rollover_ms = clock.ms_until_next_minute();

        let initial_title =
            accessible_title_for_clock(dimensions.state, clock.state(), None, OecaSpace::default());
        let title_wide: Vec<u16> = initial_title
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let _ = SetWindowTextW(hwnd, windows::core::PCWSTR(title_wide.as_ptr()));

        // This Windows session's saved clipboard history (none in a new
        // session; see clipboard_store.rs)
        let clip_store = crate::clipboard_store::ClipboardStore::open();
        let mut clipboard = ClipboardHistory::default();
        clipboard.set_capacity(settings.clipboard_capacity);
        if let Ok(items) = clip_store.load() {
            crate::clipboard_store::restore(&mut clipboard, items);
        }
        renderer.set_clipboard(&clipboard);
        renderer.set_settings(false, settings);
        // Records the settings as applied (the first launch writes the
        // defaults), but never over a settings file that could not be used,
        // e.g. one from a newer Oeca: only a setting change replaces that
        let _ = settings_store.initialize(&settings);
        // The timer starts in Focus with the saved durations (the countdown
        // itself is never saved)
        let timer = Countdown::with_secs(settings.focus_secs, settings.break_secs);
        let state = Box::new(WindowState {
            renderer,
            high_res_timer: HighResTimer::default(),
            frame_clock: FrameClock::default(),
            frame_lead_ms: 0.0,
            state: NotchState::Collapsed,
            hovered: false,
            dimensions,
            pos_x: x,
            pos_y: y,
            animation: None,
            last_frame_time: None,
            clock,
            media: MediaEngine::new(),
            media_content: None,
            space: OecaSpace::default(),
            feedback_tick: None,
            live_tick: None,
            live_ms: MEDIA_LIVE_FRAME_MS,
            clipboard,
            clip_store,
            drag: DragSession::default(),
            drop_closes: false,
            settings,
            settings_store,
            settings_open: false,
            clear_pending: false,
            grace: GracePeriod::default(),
            outside_watch: false,
            timer,
            timer_motion: crate::timer::TimerMotion::new(&timer),
            bubble_presence: Presence::default(),
            slot_presence: Presence::default(),
            bubble: None,
            bubble_shown: false,
            timer_drag: None,
            timer_shown: Countdown::default(),
            scrub: None,
            tray: crate::tray::TrayIcon::default(),
            hud: crate::hud::Hud::default(),
            momentum: crate::hud::Momentum::default(),
            volume: None,
            saver_watch: [None, None],
            updates: crate::update::UpdateState::default(),
            update_presence: Presence::default(),
            update_icon: Presence::default(),
            update_notice: None,
            update_shown: false,
            update_drawn: None,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);

        // Start async media session discovery (non-fatal; completes via WM_APP
        // message) and draw the first frame, through the same guarded access as
        // the window procedure (rendering can re-enter it synchronously).
        let initial = with_state(hwnd, |state| {
            let _ = state.media.initialize(hwnd);
            // Event-driven clipboard history (no polling)
            let _ = clipboard::start_listening(hwnd);
            // The volume keys show the HUD
            crate::hud::take_volume_keys(hwnd);
            // Battery / Energy Saver: Oeca moves as if Reduced motion were on
            state.saver_watch = watch_power_saving(hwnd);
            // Update availability, only if the user turned it on: once
            // startup has settled, then daily
            if state.settings.check_for_updates {
                let _ = SetTimer(
                    Some(hwnd),
                    crate::update::CHECK_TIMER_ID,
                    crate::update::FIRST_CHECK_MS,
                    None,
                );
            }
            state
                .renderer
                .render(hwnd, x, y, &state.dimensions, false, state.clock.state())
        });

        // Image files dragged from Explorer (OLE owns the target until revoked)
        if ole {
            let _ = RegisterDragDrop(hwnd, &DropTarget::create(hwnd, handle_drag));
        }

        // Schedule first minute rollover timer
        let _ = SetTimer(Some(hwnd), CLOCK_TIMER_ID, rollover_ms, None);

        // 6. Initial render failure is fatal (destroys the window, frees the state)
        match initial {
            Access::Ran(Ok(())) => {}
            Access::Ran(Err(e)) => {
                let _ = DestroyWindow(hwnd);
                return Err(e);
            }
            Access::Busy | Access::Missing => {
                let _ = DestroyWindow(hwnd);
                return Err(Error::from_thread());
            }
        }

        // 7. Display window without stealing focus; not on top, it shows
        // below the window in use (a game already running stays in front)
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        if !always_on_top {
            let _ = SetWindowPos(
                hwnd,
                Some(z_band(false)),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }

        // Notification-area icon: without a taskbar yet it is simply absent
        // until Explorer announces itself (TaskbarCreated)
        let _ = crate::tray::taskbar_created();
        let _ = with_state(hwnd, |state| state.tray.add(hwnd));

        // 8. Event-driven message loop: blocks until messages arrive (0% idle CPU)
        let mut msg = MSG::default();
        let loop_result = loop {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 <= 0 {
                // ret.0 == 0 means WM_QUIT; ret.0 < 0 means error
                break if ret.0 == 0 {
                    Ok(())
                } else {
                    Err(Error::from_thread())
                };
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        };

        // Clean up registered window classes
        let _ = UnregisterClassW(WINDOW_CLASS_NAME, Some(hinstance.into()));
        let _ = UnregisterClassW(BUBBLE_CLASS_NAME, Some(hinstance.into()));

        loop_result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_notice_then_icon_never_both() {
        use crate::update::{Outcome, Release, UpdateState};
        let mut s = UpdateState::default();
        let found = |s: &mut UpdateState, outcome| {
            let id = s.begin().unwrap();
            assert!(s.finish(id, outcome));
        };
        // Nothing known (or a failed check): neither
        assert_eq!(update_targets(&s, true), (false, false));
        found(&mut s, Outcome::Failed);
        assert_eq!(update_targets(&s, true), (false, false));
        // A newer release: the notice under the collapsed notch (waiting
        // while expanded), the artwork stays
        let release = Release {
            version: (0, 3, 0),
            url: "https://github.com/ujvall/Oeca/releases/tag/v0.3.0".into(),
            installer: None,
        };
        found(&mut s, Outcome::Newer(release));
        assert_eq!(update_targets(&s, true), (true, false));
        assert_eq!(update_targets(&s, false), (false, false));
        // Dismissed: the icon takes over, for as long as it is newer
        s.dismiss_notice();
        assert_eq!(update_targets(&s, true), (false, true));
        found(&mut s, Outcome::Failed);
        assert_eq!(
            update_targets(&s, true),
            (false, true),
            "a failure keeps it"
        );
        // Current again: the artwork returns
        found(&mut s, Outcome::Current);
        assert_eq!(update_targets(&s, true), (false, false));
    }

    #[test]
    fn test_state_guard_refuses_reentry_and_replays_after_exit() {
        let mut g = StateGuard::default();
        assert!(g.enter(), "outer handler gets the state");
        // Synchronous re-entry (e.g. ReleaseCapture -> WM_CAPTURECHANGED) is refused
        assert!(!g.enter(), "no second &mut WindowState");
        g.defer(WM_CAPTURECHANGED, 0, 0);
        g.defer(WM_CAPTURECHANGED, 0, 0); // duplicate collapses
        g.defer(WM_APP_DEFERRED_DPICHANGED, (144 << 16) | 144, 0);
        let (replay, free) = g.exit();
        assert_eq!(
            replay,
            vec![
                (WM_CAPTURECHANGED, 0, 0),
                (WM_APP_DEFERRED_DPICHANGED, (144 << 16) | 144, 0)
            ]
        );
        assert!(!free);
        assert!(g.enter(), "available again after the outer handler exits");
        let (replay, _) = g.exit();
        assert!(replay.is_empty(), "queue drained exactly once");
    }

    #[test]
    fn test_state_guard_defers_free_during_handler() {
        let mut g = StateGuard::default();
        // Idle: WM_NCDESTROY frees immediately
        assert!(g.request_free());
        // Destruction re-entered a running handler: free only after it returns
        assert!(g.enter());
        assert!(!g.request_free(), "state still borrowed by the outer frame");
        let (_, free) = g.exit();
        assert!(free, "freed exactly when the outer handler exits");
        assert!(g.enter());
        let (_, free) = g.exit();
        assert!(!free, "never freed twice");
    }

    #[test]
    fn test_busy_policy() {
        // Periodic/self-recurring messages are dropped, never queued without bound
        assert_eq!(busy_policy(WM_TIMER), BusyPolicy::Drop);
        assert_eq!(busy_policy(WM_MOUSEMOVE), BusyPolicy::Drop);
        // WM_DPICHANGED cannot be posted: replayed as the private message
        assert_eq!(
            busy_policy(WM_DPICHANGED),
            BusyPolicy::Replay(WM_APP_DEFERRED_DPICHANGED)
        );
        for m in [
            WM_CAPTURECHANGED,
            WM_LBUTTONDOWN,
            WM_LBUTTONUP,
            WM_MOUSELEAVE,
            WM_DISPLAYCHANGE,
            WM_TIMECHANGE,
            WM_POWERBROADCAST,
            WM_APP_MEDIA_SESSION_CHANGED,
        ] {
            assert_eq!(busy_policy(m), BusyPolicy::Replay(m), "0x{m:04x}");
        }
    }

    #[test]
    fn test_with_state_missing_window_state_is_safe() {
        // No WindowState attached (invalid HWND): never dereferenced
        assert_eq!(with_state(HWND::default(), |_| ()), Access::Missing);
        // The guard is released afterwards
        assert!(GUARD.with_borrow_mut(|g| {
            let ok = g.enter();
            g.exit();
            ok
        }));
    }

    /// Frame pacing of a real expand animation (renderer, off-screen layered
    /// window) with the old 12 ms timer sampled at tick time, and with the
    /// frame clock sampled at the next refresh. Per frame: the time the motion
    /// was sampled for, and the refresh it was actually shown on. Run with
    /// `cargo test --release bench_frame_pacing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_frame_pacing() {
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, PM_REMOVE, PeekMessageW, SW_SHOWNOACTIVATE, ShowWindow,
            WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
        };
        let Some(refresh) = frame_time(0.0).1 else {
            eprintln!("BENCH pacing: DWM reports no refresh timing here");
            return;
        };
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                windows::core::w!("STATIC"),
                None,
                WS_POPUP,
                -30000,
                -30000,
                1,
                1,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        let now_ms = || {
            let (mut t, mut f) = (0i64, 0i64);
            unsafe {
                let _ = QueryPerformanceCounter(&mut t);
                let _ = QueryPerformanceFrequency(&mut f);
            }
            t as f64 * 1000.0 / f as f64
        };
        let clock =
            crate::clock::ClockDateState::from_pure_components(2026, 10, 6, 2, 0, 47, 0, 0, false);
        let mut res = HighResTimer::default();
        res.set(true);
        eprintln!(
            "BENCH pacing: display refresh {:.2} ms ({:.0} Hz)",
            refresh,
            1000.0 / refresh
        );
        for dpi in [96u32, 192] {
            for paced in [false, true] {
                let renderer = Renderer::new().unwrap();
                let mut fc = FrameClock::default();
                if paced {
                    assert!(fc.start(hwnd, frame_period_ms(refresh)));
                } else {
                    unsafe { SetTimer(Some(hwnd), 7, ANIMATION_FRAME_INTERVAL_MS, None) };
                }
                // (sampled for, shown at) per frame, over 10 animations
                let mut frames: Vec<(f64, f64)> = Vec::new();
                let mut holes = 0.0;
                let mut render_ms = 0.0;
                let mut lead = 0.0;
                for _ in 0..10 {
                    let mut anim = AnimationState::new(NotchState::Collapsed, NotchState::Expanded);
                    let start = if paced { frame_time(lead).0 } else { now_ms() };
                    let mut last = start;
                    let mut shown = Vec::new();
                    let mut active = true;
                    while active {
                        let mut msg = MSG::default();
                        let _ = unsafe { GetMessageW(&mut msg, Some(hwnd), 0, 0) };
                        let tick = (paced && msg.message == WM_APP_ANIM_FRAME)
                            || (!paced && msg.message == WM_TIMER);
                        if !tick {
                            continue;
                        }
                        fc.frame_received();
                        let sample = if paced { frame_time(lead).0 } else { now_ms() };
                        if sample <= last {
                            continue;
                        }
                        active = anim.step((sample - last) as f32);
                        last = sample;
                        renderer.set_transition(Some(anim.transition()));
                        let dims = anim.current_dimensions(dpi);
                        let t = now_ms();
                        renderer
                            .render(hwnd, -30000, -30000, &dims, false, &clock)
                            .unwrap();
                        let cost = now_ms() - t;
                        render_ms += cost;
                        lead = cost.max(lead * 0.9);
                        // Shown on the first refresh after the upload
                        let at = frame_time(0.0).0;
                        frames.push((sample, at));
                        shown.push(at);
                    }
                    // Refreshes during the animation that showed no new frame
                    shown.dedup_by(|a, b| (*a - *b).abs() < refresh / 2.0);
                    let span = ((shown.last().unwrap() - shown[0]) / refresh).round() + 1.0;
                    holes += span - shown.len() as f64;
                    while unsafe { PeekMessageW(&mut MSG::default(), Some(hwnd), 0, 0, PM_REMOVE) }
                        .as_bool()
                    {}
                    fc.frame_received();
                }
                fc.stop();
                unsafe {
                    let _ = KillTimer(Some(hwnd), 7);
                }
                // Judder: how far each frame's shown time is from the time its
                // motion was sampled for (spread = uneven motion)
                let errs: Vec<f64> = frames.iter().map(|(s, a)| a - s).collect();
                let mean = errs.iter().sum::<f64>() / errs.len() as f64;
                let sd = (errs.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / errs.len() as f64)
                    .sqrt();
                eprintln!(
                    "BENCH pacing {:>3}% {}: {} frames, {:.0}% of refreshes without a new frame,                      shown-vs-sampled offset {:.2} ms +/- {:.2} ms, render {:.2} ms avg",
                    dpi * 100 / 96,
                    if paced {
                        "new (frame clock, next refresh)"
                    } else {
                        "old (12 ms timer, tick time)  "
                    },
                    frames.len(),
                    100.0 * holes / (holes + frames.len() as f64),
                    mean,
                    sd,
                    render_ms / frames.len() as f64
                );
            }
        }
        res.set(false);
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    #[test]
    fn test_frames_are_timed_to_the_next_refresh() {
        // 144 Hz in ticks of 1 us: a vblank every 6944
        let (vblank, period) = (1_000_000, 6_944);
        assert_eq!(
            next_vblank(1_000_000, vblank, period),
            1_006_944,
            "on a vblank: the next"
        );
        assert_eq!(next_vblank(1_003_000, vblank, period), 1_006_944);
        assert_eq!(next_vblank(1_006_944, vblank, period), 1_013_888);
        // A stale vblank still gives a refresh in the near future
        let t = next_vblank(9_000_000, vblank, period);
        assert!(t > 9_000_000 && t <= 9_000_000 + period && (t - vblank) % period == 0);
        assert!(
            next_vblank(990_000, vblank, period) > 990_000,
            "now before the vblank"
        );
        // One frame per refresh, rounded down; bounded at both ends
        assert_eq!(frame_period_ms(1000.0 / 60.0), 16);
        assert_eq!(frame_period_ms(1000.0 / 120.0), 8);
        assert_eq!(frame_period_ms(1000.0 / 144.0), 6);
        assert_eq!(frame_period_ms(1000.0 / 240.0), 4);
        assert_eq!(frame_period_ms(1000.0 / 360.0), 4);
        assert_eq!(frame_period_ms(1000.0 / 30.0), 16);
        // The live clock moves forward, a refresh at a time when DWM reports one
        let (a, refresh) = frame_time(0.0);
        let (b, _) = frame_time(0.0);
        assert!(b >= a);
        if let Some(r) = refresh {
            assert!((2.0..=50.0).contains(&r), "{r} ms");
        }
    }

    #[test]
    fn test_frame_clock_paces_coalesces_and_stops() {
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, HWND_MESSAGE, PM_REMOVE, PeekMessageW, WINDOW_EX_STYLE,
            WINDOW_STYLE,
        };
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                None,
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
        }
        .unwrap();
        // Frames received over `ms`, acknowledging each one if `ack`
        let pump = |clock: &FrameClock, ms: u64, ack: bool| {
            let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
            let mut frames = 0;
            while std::time::Instant::now() < end {
                let mut msg = MSG::default();
                while unsafe { PeekMessageW(&mut msg, Some(hwnd), 0, 0, PM_REMOVE) }.as_bool() {
                    if msg.message == WM_APP_ANIM_FRAME {
                        frames += 1;
                        if ack {
                            clock.frame_received();
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            frames
        };
        let mut res = HighResTimer::default();
        res.set(true);
        let mut clock = FrameClock::default();
        assert!(clock.start(hwnd, 8));
        // ~25 frames in 200 ms at 8 ms (generous bounds for a loaded machine)
        let paced = pump(&clock, 200, true);
        assert!((8..=40).contains(&paced), "{paced} frames");
        // Unacknowledged: one frame queued, no backlog; one left unanswered
        // for 8 ticks is sent again (a lost message never stalls the frames)
        clock.stop();
        let _ = pump(&clock, 20, true);
        assert!(clock.start(hwnd, 8));
        assert_eq!(pump(&clock, 40, false), 1, "no backlog");
        let resent = pump(&clock, 150, false);
        assert!((1..=3).contains(&resent), "{resent} resent");
        // Restarting at the same period keeps it running; stopping ends it
        assert!(clock.start(hwnd, 8));
        clock.frame_received();
        clock.stop();
        let _ = pump(&clock, 20, true);
        assert_eq!(pump(&clock, 100, true), 0, "stopped");
        res.set(false);
        drop(clock);
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    #[test]
    fn test_high_res_timer_never_stacks_or_double_releases() {
        let mut t = HighResTimer::default();
        assert_eq!(
            t.transition(false),
            None,
            "release without request is a no-op"
        );
        assert_eq!(
            t.transition(true),
            Some(true),
            "animation start raises once"
        );
        assert_eq!(
            t.transition(true),
            None,
            "reversal mid-animation does not stack"
        );
        assert_eq!(t.transition(true), None);
        assert_eq!(t.transition(false), Some(false), "completion releases once");
        assert_eq!(
            t.transition(false),
            None,
            "close/destroy after completion: no-op"
        );
        // Begin/end calls are always balanced over any sequence
        let mut depth = 0i32;
        for want in [
            true, true, false, true, false, false, true, true, true, false,
        ] {
            match t.transition(want) {
                Some(true) => depth += 1,
                Some(false) => depth -= 1,
                None => {}
            }
            assert!((0..=1).contains(&depth), "unbalanced: {depth}");
        }
        assert_eq!(depth, 0);
    }

    #[test]
    fn test_accessible_title_with_media() {
        use crate::media::PlayPauseIcon;
        let clock =
            crate::clock::ClockDateState::from_pure_components(2026, 10, 6, 2, 0, 47, 0, 0, false);
        let time = clock.formatted_time.clone();
        let date = clock.formatted_date.clone();
        let home = OecaSpace::Home;
        let playing = MediaContent {
            title: "Song".into(),
            subtitle: "Artist".into(),
            icon: PlayPauseIcon::Pause,
            ..MediaContent::test_default()
        };
        assert_eq!(
            accessible_title_for_clock(NotchState::Expanded, &clock, Some(&playing), home),
            format!(
                "Song - Artist - Previous track, Pause, Next track - {time} - {date} - Home space selected (spaces: Home, Music, Clipboard)"
            )
        );
        let paused = MediaContent {
            icon: PlayPauseIcon::Play,
            ..playing.clone()
        };
        assert!(
            accessible_title_for_clock(NotchState::Expanded, &clock, Some(&paused), home)
                .contains("Previous track, Play, Next track")
        );
        // Collapsed title is unchanged (no selector there)
        for space in OecaSpace::ALL {
            assert_eq!(
                accessible_title_for_clock(NotchState::Collapsed, &clock, Some(&playing), space),
                time
            );
        }
        assert_eq!(
            accessible_title_for_clock(NotchState::Expanded, &clock, None, home),
            format!("{time} - {date} - Home space selected (spaces: Home, Music, Clipboard)")
        );
    }

    #[test]
    fn test_accessible_title_for_clipboard_space() {
        use crate::clipboard::ClipboardItem;
        let clock =
            crate::clock::ClockDateState::from_pure_components(2026, 10, 6, 2, 0, 47, 0, 0, false);
        let (time, date) = (clock.formatted_time.clone(), clock.formatted_date.clone());
        let media = MediaContent {
            title: "Song".into(),
            ..MediaContent::test_default()
        };
        let mut history = ClipboardHistory::default();
        let title = |h: &ClipboardHistory, state| {
            accessible_title(
                state,
                &clock,
                Some(&media),
                OecaSpace::Clipboard,
                h,
                false,
                None,
            )
        };
        assert_eq!(
            title(&history, NotchState::Expanded),
            format!(
                "Clipboard history, empty - {time} - {date} - Clipboard space selected (spaces: Home, Music, Clipboard)"
            )
        );
        history.add(ClipboardItem::Text("secret password".into()));
        history.add(ClipboardItem::Image(
            crate::media::Artwork::new(1, 1, vec![0, 0, 0, 255]).unwrap(),
        ));
        let t = title(&history, NotchState::Expanded);
        assert!(t.starts_with(
            "Clipboard history, 2 items, newest is an image, Copy and Delete buttons on each item, Clear history button - "
        ));
        assert!(
            !t.contains("secret") && !t.contains("Song"),
            "no contents, no media"
        );
        history.promote(1);
        assert!(title(&history, NotchState::Expanded).contains("newest is text"));
        // The clear confirmation reads its message and both buttons by name
        let confirming = accessible_title(
            NotchState::Expanded,
            &clock,
            Some(&media),
            OecaSpace::Clipboard,
            &history,
            true,
            None,
        );
        assert!(confirming.starts_with(CLEAR_CONFIRM_ACCESSIBLE));
        assert!(confirming.contains("Cancel button") && confirming.contains("Clear button"));
        assert!(!confirming.contains("secret"));
        // Collapsed and the other spaces are unchanged
        assert_eq!(title(&history, NotchState::Collapsed), time);
        for s in [OecaSpace::Home, OecaSpace::Music] {
            assert_eq!(
                accessible_title(
                    NotchState::Expanded,
                    &clock,
                    Some(&media),
                    s,
                    &history,
                    false,
                    None
                ),
                accessible_title_for_clock(NotchState::Expanded, &clock, Some(&media), s)
            );
        }
    }

    #[test]
    fn test_always_on_top_defaults_on_and_maps_to_z_order() {
        let mut settings = OecaSettings::default();
        assert!(
            settings.always_on_top,
            "Oeca starts topmost, as it always has"
        );
        // Routine moves keep the band: topmost re-asserted, or z-order untouched
        assert_eq!(z_order(true), (Some(HWND_TOPMOST), SET_WINDOW_POS_FLAGS(0)));
        assert_eq!(z_order(false), (None, SWP_NOZORDER));
        // Flipping moves into / out of the topmost band
        assert_eq!(z_band(true), HWND_TOPMOST);
        // Off: never the topmost band; below the app in use, else the top of
        // the normal band
        assert_ne!(z_band(false), HWND_TOPMOST);
        assert_eq!(normal_slot(HWND::default()), HWND_NOTOPMOST);
        let app = HWND(0x1234 as *mut _);
        assert_eq!(
            foreground_kind(false, false, false, app),
            Foreground::App(app)
        );
        for (oeca, topmost, shell) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            assert_eq!(
                foreground_kind(oeca, topmost, shell, app),
                Foreground::None,
                "never below Oeca itself, a topmost window or the desktop / taskbar"
            );
        }
        settings.always_on_top = !settings.always_on_top;
        assert!(!settings.always_on_top);
        settings.always_on_top = !settings.always_on_top;
        assert!(settings.always_on_top);
    }

    #[test]
    fn test_power_saver_is_effective_reduced_motion_never_the_saved_one() {
        // Effective motion: the user's preference OR the system OR a saver
        assert!(effective_motion(false, true, false));
        assert!(!effective_motion(false, true, true), "saver on: snap");
        assert!(!effective_motion(true, true, false), "the user's own");
        assert!(!effective_motion(true, true, true));
        assert!(
            !effective_motion(false, false, false),
            "system animations off"
        );
        // The saved setting is a separate value the saver never touches
        let settings = OecaSettings::default();
        let effective = effective_motion(settings.reduced_motion, true, true);
        assert!(!effective && !settings.reduced_motion);
        // Saver off again: back to whatever the user chose
        assert!(effective_motion(settings.reduced_motion, true, false));
        assert!(!effective_motion(true, true, false));
    }

    #[test]
    fn test_saver_reports_are_read_from_the_notification() {
        use windows::Win32::System::SystemServices::GUID_POWER_SAVING_STATUS;
        #[repr(C)]
        struct Setting {
            guid: windows::core::GUID,
            len: u32,
            data: u32,
        }
        let read = |guid, data| {
            let mut s = Setting { guid, len: 4, data };
            unsafe { saver_report(LPARAM(&mut s as *mut Setting as isize)) }
        };
        assert_eq!(
            read(GUID_POWER_SAVING_STATUS, 1),
            Some((Saver::Battery, true))
        );
        assert_eq!(
            read(GUID_POWER_SAVING_STATUS, 0),
            Some((Saver::Battery, false))
        );
        assert_eq!(
            read(GUID_ENERGY_SAVER_STATUS, 2),
            Some((Saver::Energy, true))
        );
        assert_eq!(
            read(GUID_ENERGY_SAVER_STATUS, 1),
            Some((Saver::Energy, true))
        );
        assert_eq!(
            read(GUID_ENERGY_SAVER_STATUS, 0),
            Some((Saver::Energy, false))
        );
        assert_eq!(
            read(windows::core::GUID::zeroed(), 1),
            None,
            "other settings"
        );
        assert_eq!(unsafe { saver_report(LPARAM(0)) }, None);
        let mut short = Setting {
            guid: GUID_POWER_SAVING_STATUS,
            len: 0,
            data: 1,
        };
        let lp = LPARAM(&mut short as *mut Setting as isize);
        assert_eq!(unsafe { saver_report(lp) }, None, "no data");
    }

    #[test]
    fn test_reduced_motion_setting_uses_the_snap_path() {
        let mut settings = OecaSettings::default();
        assert!(!settings.reduced_motion, "animations on by default");
        assert!(motion_allowed(settings.reduced_motion, true));
        settings.reduced_motion = !settings.reduced_motion;
        assert!(
            !motion_allowed(settings.reduced_motion, true),
            "the user's choice snaps"
        );
        // The system preference still snaps on its own
        assert!(!motion_allowed(false, false));
        assert!(!motion_allowed(true, false));
        settings.reduced_motion = !settings.reduced_motion;
        assert!(motion_allowed(settings.reduced_motion, true));
    }

    #[test]
    fn test_settings_target_is_the_clipboard_size_from_every_space() {
        use crate::layout::expanded_size_dip;
        for dpi in [96, 120, 144, 192] {
            let clipboard = expanded_size_dip(OecaSpace::Clipboard, dpi);
            for space in OecaSpace::ALL {
                let open = Scene::current(crate::dragdrop::DropPage::Off, true, space);
                assert_eq!(
                    expanded_size_dip(open.size_space(), dpi),
                    clipboard,
                    "{space:?}"
                );
                // Back from Settings: the originating space's own size
                let closed = Scene::current(crate::dragdrop::DropPage::Off, false, space);
                assert_eq!(closed, Scene::Space(space));
                assert_eq!(
                    expanded_size_dip(closed.size_space(), dpi),
                    expanded_size_dip(space, dpi)
                );
            }
        }
        // Still exactly three product spaces
        assert_eq!(OecaSpace::ALL.len(), 3);
    }

    #[test]
    fn test_accessible_title_for_settings() {
        use crate::clipboard::ClipboardItem;
        let clock =
            crate::clock::ClockDateState::from_pure_components(2026, 10, 6, 2, 0, 47, 0, 0, false);
        let mut history = ClipboardHistory::default();
        history.add(ClipboardItem::Text("secret".into()));
        let media = MediaContent {
            title: "Song".into(),
            ..MediaContent::test_default()
        };
        for space in OecaSpace::ALL {
            let t = |always_on_top, reduced_motion| {
                accessible_title(
                    NotchState::Expanded,
                    &clock,
                    Some(&media),
                    space,
                    &history,
                    false,
                    Some(OecaSettings {
                        always_on_top,
                        reduced_motion,
                        ..OecaSettings::default()
                    }),
                )
            };
            assert!(t(true, false).starts_with(
                "Settings, Always on top enabled, Reduced motion disabled - Always on top switch, on - Reduced motion switch, off - Open Full Settings button - "
            ));
            assert!(t(false, true).starts_with(
                "Settings, Always on top disabled, Reduced motion enabled - Always on top switch, off - Reduced motion switch, on - "
            ));
            assert!(t(true, false).contains(&format!("{} space selected", space.label())));
            assert!(!t(true, false).contains("secret") && !t(true, false).contains("Song"));
        }
        // Collapsed: just the time, as before
        assert_eq!(
            accessible_title(
                NotchState::Collapsed,
                &clock,
                None,
                OecaSpace::Home,
                &history,
                false,
                Some(OecaSettings::default())
            ),
            clock.formatted_time
        );
    }

    #[test]
    fn test_accessible_title_reflects_active_space() {
        let clock =
            crate::clock::ClockDateState::from_pure_components(2026, 10, 6, 2, 0, 47, 0, 0, false);
        let media = MediaContent {
            title: "Song".into(),
            ..MediaContent::test_default()
        };
        let mut space = OecaSpace::default();
        let title = |s| accessible_title_for_clock(NotchState::Expanded, &clock, Some(&media), s);
        assert!(title(space).ends_with("Home space selected (spaces: Home, Music, Clipboard)"));
        space.switch_to(OecaSpace::Music);
        assert!(title(space).ends_with("Music space selected (spaces: Home, Music, Clipboard)"));
        // The media part is identical in both spaces
        let media_part = |t: String| t.split(" - Home space").next().unwrap().to_string();
        assert_eq!(
            media_part(title(OecaSpace::Home)),
            title(OecaSpace::Music)
                .split(" - Music space")
                .next()
                .unwrap()
        );
    }

    #[test]
    fn test_is_client_animation_enabled_runs_safely() {
        // Query system parameter safely without crashing
        let enabled = is_client_animation_enabled();
        // Result is a valid boolean
        assert!(enabled || !enabled);
    }

    #[test]
    fn test_clear_flow_confirmation_on_cancel_and_confirm() {
        use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
        let filled = || {
            let mut h = ClipboardHistory::default();
            h.add(ClipboardItem::Text("one".into()));
            h.add(ClipboardItem::Text("two".into()));
            h
        };
        let system_clipboard = unsafe { GetClipboardSequenceNumber() };
        // Confirmation on: Clear only asks
        let (mut history, mut pending) = (filled(), false);
        clear_flow(PanelHit::Clear, true, &mut pending, &mut history);
        assert!(pending, "confirmation requested");
        assert_eq!(history.len(), 2, "nothing cleared yet");
        // Cancel: dismissed, history unchanged
        clear_flow(PanelHit::CancelClear, true, &mut pending, &mut history);
        assert!(!pending);
        assert_eq!(history.len(), 2);
        assert_eq!(history.get(0), Some(&ClipboardItem::Text("two".into())));
        // Ask again, then confirm: Oeca's history is gone
        clear_flow(PanelHit::Clear, true, &mut pending, &mut history);
        clear_flow(PanelHit::ConfirmClear, true, &mut pending, &mut history);
        assert!(!pending && history.is_empty());
        // Confirmation off: Clear forgets at once, never asks
        let (mut history, mut pending) = (filled(), false);
        clear_flow(PanelHit::Clear, false, &mut pending, &mut history);
        assert!(!pending && history.is_empty());
        // Other hits never touch the history or the confirmation
        let (mut history, mut pending) = (filled(), true);
        clear_flow(PanelHit::Settings, true, &mut pending, &mut history);
        assert!(pending && history.len() == 2);
        // The system clipboard was never written (its sequence number counts
        // every change)
        assert_eq!(unsafe { GetClipboardSequenceNumber() }, system_clipboard);
    }

    #[test]
    fn test_live_timer_slows_when_only_the_scrubber_moves() {
        use crate::config::{MEDIA_LIVE_FRAME_MS, MEDIA_SCRUBBER_FRAME_MS};
        use NotchState::{Collapsed, Expanded};
        // Settled Music with the visualizer hidden: the slow scrubber rate
        assert_eq!(
            live_period_ms(false, Expanded, true, false),
            MEDIA_SCRUBBER_FRAME_MS
        );
        const { assert!(MEDIA_SCRUBBER_FRAME_MS >= 7 * MEDIA_LIVE_FRAME_MS) };
        // Shown again: visualizer frames resume
        assert_eq!(
            live_period_ms(false, Expanded, true, true),
            MEDIA_LIVE_FRAME_MS
        );
        // Bars that stay visible keep their rate whatever the setting: the
        // collapsed notch, and any notch animation
        for show in [true, false] {
            assert_eq!(
                live_period_ms(false, Collapsed, false, show),
                MEDIA_LIVE_FRAME_MS
            );
            assert_eq!(
                live_period_ms(true, Expanded, true, show),
                MEDIA_LIVE_FRAME_MS
            );
            assert_eq!(
                live_period_ms(true, Collapsed, true, show),
                MEDIA_LIVE_FRAME_MS
            );
        }
    }

    #[test]
    fn test_grace_period_outside_click_lifecycle() {
        use GraceAction::*;
        use GraceEvent::*;
        let mut g = GracePeriod::default();
        // Expanded and untouched: nothing ever collapses it
        assert!(!g.pending());
        assert_eq!(
            g.handle(TimerFired, true, false),
            Disarm,
            "no fire without a click"
        );
        // An outside click starts the countdown; another restarts it
        assert_eq!(g.handle(OutsideClick, true, false), Arm);
        assert_eq!(
            g.handle(OutsideClick, true, false),
            Arm,
            "restart from the latest click"
        );
        assert!(g.pending());
        // Running out collapses, once
        assert_eq!(g.handle(TimerFired, true, false), Collapse);
        assert_eq!(
            g.handle(TimerFired, true, false),
            Disarm,
            "a second fire is stale"
        );
        // A click in the notch (or the Settings window) cancels it first
        g.handle(OutsideClick, true, false);
        assert_eq!(g.handle(OecaInteraction, true, false), Disarm);
        assert_eq!(
            g.handle(TimerFired, true, false),
            Disarm,
            "cancelled: no collapse"
        );
        // An explicit collapse cancels; a stale fire after re-expanding is
        // ignored
        g.handle(OutsideClick, true, false);
        assert_eq!(
            g.handle(StateChanged, false, false),
            Disarm,
            "explicit collapse"
        );
        assert_eq!(g.handle(StateChanged, true, false), None, "re-expanded");
        assert_eq!(
            g.handle(TimerFired, true, false),
            Disarm,
            "stale fire ignored"
        );
        // Collapsed: outside clicks start nothing
        assert_eq!(g.handle(OutsideClick, false, false), None);
        assert!(!g.pending());
        // Indices round-trip for the posted message
        for (i, e) in GraceEvent::ALL.into_iter().enumerate() {
            assert_eq!(e as usize, i);
        }
    }

    #[test]
    fn test_click_classification_and_raw_input_registration() {
        let (notch, settings, other) = (
            HWND(0x100 as *mut _),
            HWND(0x200 as *mut _),
            HWND(0x300 as *mut _),
        );
        assert_eq!(
            classify_click(notch, notch, Some(settings)),
            None,
            "the notch handles its own"
        );
        assert_eq!(
            classify_click(settings, notch, Some(settings)),
            Some(GraceEvent::OecaInteraction),
            "using Settings never collapses the notch"
        );
        assert_eq!(
            classify_click(other, notch, Some(settings)),
            Some(GraceEvent::OutsideClick)
        );
        assert_eq!(
            classify_click(other, notch, None),
            Some(GraceEvent::OutsideClick)
        );
        // The sink registers on a real window and is removed again (as on
        // collapse and shutdown), leaving nothing registered
        use windows::Win32::UI::Input::{GetRegisteredRawInputDevices, RAWINPUTDEVICE};
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
        };
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                None,
                WINDOW_STYLE(0),
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        let registered = || {
            let mut n = 0u32;
            unsafe {
                GetRegisteredRawInputDevices(
                    None,
                    &mut n,
                    std::mem::size_of::<RAWINPUTDEVICE>() as u32,
                );
            }
            n
        };
        assert!(watch_outside_clicks(hwnd, true));
        assert_eq!(registered(), 1);
        assert!(watch_outside_clicks(hwnd, false));
        assert_eq!(registered(), 0, "cleaned up");
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    #[test]
    fn test_zero_delay_collapses_on_the_outside_click_itself() {
        use GraceAction::*;
        use GraceEvent::*;
        let mut g = GracePeriod::default();
        // Expanded: an outside click collapses now, through the normal path
        // (the window animates it); nothing is left pending or armed
        assert_eq!(g.handle(OutsideClick, true, true), Collapse);
        assert!(!g.pending());
        assert_eq!(g.handle(TimerFired, true, true), Disarm, "no stale timer");
        // Collapsed: nothing to close
        assert_eq!(g.handle(OutsideClick, false, true), None);
        // Inside clicks and the Settings window never close it
        assert_eq!(g.handle(OecaInteraction, true, true), None);
        // Switching to 0 s while a countdown runs: its time is up
        g.handle(OutsideClick, true, false);
        assert_eq!(g.handle(DelayChanged, true, true), Collapse);
        assert!(!g.pending());
        // ...but only if the notch is still open
        g.handle(OutsideClick, true, false);
        assert_eq!(g.handle(DelayChanged, false, true), Disarm);
        // Positive delays keep their countdown
        assert_eq!(g.handle(OutsideClick, true, false), Arm);
        assert!(g.pending());
    }

    #[test]
    fn test_collapse_delay_durations_and_restart_on_change() {
        use crate::config::CollapseDelay;
        use GraceAction::*;
        use GraceEvent::*;
        assert_eq!(OecaSettings::default().collapse_delay, CollapseDelay::Zero);
        assert_eq!(
            CollapseDelay::ALL.map(CollapseDelay::ms),
            [0, 5_000, 10_000, 15_000]
        );
        for (i, d) in CollapseDelay::ALL.into_iter().enumerate() {
            assert_eq!(CollapseDelay::from_index(i), Some(d));
        }
        assert_eq!(CollapseDelay::from_index(4), Option::None);
        let mut g = GracePeriod::default();
        // Changing the delay with nothing pending starts nothing
        assert_eq!(g.handle(DelayChanged, true, false), None);
        // Pending: the countdown restarts (the window arms it with the new ms)
        g.handle(OutsideClick, true, false);
        assert_eq!(g.handle(DelayChanged, true, false), Arm);
        assert!(g.pending());
        // Later outside clicks restart it; inside clicks / Settings cancel it
        assert_eq!(g.handle(OutsideClick, true, false), Arm);
        assert_eq!(g.handle(OecaInteraction, true, false), Disarm);
        assert_eq!(
            g.handle(DelayChanged, true, false),
            None,
            "cancelled stays cancelled"
        );
        // Explicit collapse cancels at once
        g.handle(OutsideClick, true, false);
        assert_eq!(g.handle(StateChanged, false, false), Disarm);
        assert_eq!(
            g.handle(TimerFired, false, false),
            Disarm,
            "no late collapse"
        );
    }

    #[test]
    fn test_timer_bubble_is_click_through_renders_and_dies_with_the_notch() {
        use windows::Win32::UI::WindowsAndMessaging::{
            GW_OWNER, GWL_EXSTYLE, GetWindow, IsWindow, IsWindowVisible, SendMessageW,
        };
        // A stand-in notch (hidden, stateless) owning the bubble
        let owner = create_bubble(HWND::default()).expect("owner");
        let bubble = create_bubble(owner).expect("bubble");
        let renderer = Renderer::new().expect("renderer");
        let mut countdown = Countdown::default();
        countdown.primary(std::time::Instant::now());
        renderer.set_timer(crate::renderer::TimerView {
            countdown,
            ..Default::default()
        });
        unsafe {
            assert_eq!(
                GetWindow(bubble, GW_OWNER).ok(),
                Some(owner),
                "owned by the notch"
            );
            let ex = GetWindowLongPtrW(bubble, GWL_EXSTYLE) as u32;
            for flag in [
                WS_EX_TRANSPARENT,
                WS_EX_NOACTIVATE,
                WS_EX_LAYERED,
                WS_EX_TOOLWINDOW,
            ] {
                assert!(ex & flag.0 != 0, "style {flag:?}");
            }
            // Never takes the mouse, never activates
            let hit = SendMessageW(bubble, WM_NCHITTEST, None, None);
            assert_eq!(hit.0, HTTRANSPARENT as isize);
            let activate = SendMessageW(bubble, WM_MOUSEACTIVATE, None, None);
            assert_eq!(activate.0, MA_NOACTIVATE as isize);
            assert!(!IsWindowVisible(bubble).as_bool(), "hidden until needed");
            for dpi in [96u32, 144] {
                let collapsed = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
                let h = collapsed.height;
                let alpha = |p: u32| p >> 24;
                let orange = |p: u32| {
                    let (b, g, r) = (p & 0xFF, (p >> 8) & 0xFF, (p >> 16) & 0xFF);
                    r > 120 && r > g + 30 && g > b + 30
                };
                // Absorbed: nothing past the notch's wall (what is left of the
                // drop is under the notch's own black body)
                let g = renderer.render_bubble(bubble, 100, 0, dpi, 0.0).unwrap();
                let (w, wall) = (g.window.0, g.wall.ceil() as usize);
                let (px, stride) = renderer.surface(h);
                let outside = (0..h as usize)
                    .any(|y| (wall..w as usize).any(|x| alpha(px[y * stride + x]) > 0));
                assert!(!outside, "fully absorbed at {dpi}");
                let any = |px: &[u32], f: &dyn Fn(u32) -> bool| {
                    (0..h as usize).any(|y| (0..w as usize).any(|x| f(px[y * stride + x])))
                };
                // Budding: the black drop shows first, the ring not yet
                renderer.render_bubble(bubble, 100, 0, dpi, 0.5).unwrap();
                let (px, _) = renderer.surface(h);
                assert!(any(&px, &|p| alpha(p) > 0), "shape at {dpi}");
                assert!(!any(&px, &orange), "numerals come last at {dpi}");
                // Shown: a floating pure-black circle with the orange ring
                let g = renderer.render_bubble(bubble, 100, 0, dpi, 1.0).unwrap();
                let w = g.window.0;
                let (px, _) = renderer.surface(h);
                let any = |f: &dyn Fn(u32) -> bool| {
                    (0..h as usize).any(|y| (0..w as usize).any(|x| f(px[y * stride + x])))
                };
                assert!(any(&orange), "ring at {dpi}");
                let ((cx, cy), r) = (g.center, g.radius);
                let centre = px[cy as usize * stride + cx as usize];
                assert_eq!(alpha(centre), 255, "solid at {dpi}");
                assert_eq!(centre & 0x00FF_FFFF, 0, "pure black at {dpi}");
                // Not attached to the screen edge: its top rows are clear...
                for y in 0..(cy - r).floor() as usize {
                    assert!(
                        (0..w as usize).all(|x| alpha(px[y * stride + x]) == 0),
                        "row {y} clear at {dpi}"
                    );
                }
                // ...and it is round: its bounding box's corners stay clear
                let (l, t, rr, b) = (cx - r, cy - r, cx + r - 1.0, cy + r - 1.0);
                for (x, y) in [(l, t), (rr, t), (l, b), (rr, b)] {
                    assert!(
                        alpha(px[y as usize * stride + x as usize]) < 40,
                        "round at {dpi}"
                    );
                }
                // The ring sits at 12 o'clock first (a full countdown)
                let top_of_ring = px[(cy - g.ring.0) as usize * stride + cx as usize];
                assert!(orange(top_of_ring), "ring starts at 12 at {dpi}");
                // Close to the notch: joined to the wall by the liquid neck
                let g = renderer.render_bubble(bubble, 100, 0, dpi, 0.45).unwrap();
                let (px, _) = renderer.surface(h);
                assert!(g.neck > 0.0, "neck at {dpi}");
                let wall_px = px[g.center.1 as usize * stride + g.wall as usize + 1];
                assert_eq!(alpha(wall_px), 255, "joined to the wall at {dpi}");
            }
            // Shutting the notch down takes the bubble with it
            let _ = DestroyWindow(owner);
            assert!(
                !IsWindow(Some(bubble)).as_bool(),
                "destroyed with its owner"
            );
            assert!(!IsWindow(Some(owner)).as_bool());
        }
    }

    #[test]
    fn test_timer_title_parts() {
        assert_eq!(with_timer("12:47 AM".into(), None), "12:47 AM");
        assert_eq!(
            with_timer("12:47 AM".into(), Some("Timer running")),
            "12:47 AM - Timer running"
        );
    }

    #[test]
    fn test_scrub_gesture_is_never_an_outside_click() {
        let notch = create_bubble(HWND::default()).expect("stand-in notch");
        let other = create_bubble(HWND::default()).expect("another window");
        // Capture held by the notch: its gesture owns every button
        assert!(gesture_owns_buttons(notch, notch));
        // Nobody / someone else holds it: clicks are classified as usual
        assert!(!gesture_owns_buttons(HWND::default(), notch));
        assert!(!gesture_owns_buttons(other, notch));
        // ...and a genuine outside click still arms the countdown
        let mut g = GracePeriod::default();
        let event = classify_click(other, notch, None).unwrap();
        assert_eq!(event, GraceEvent::OutsideClick);
        assert_eq!(g.handle(event, true, false), GraceAction::Arm);
        // A press on the notch itself (where a scrub starts) is not outside
        assert_eq!(classify_click(notch, notch, None), None);
        unsafe {
            let _ = DestroyWindow(notch);
            let _ = DestroyWindow(other);
        }
    }

    #[test]
    fn test_scrub_release_position_stays_on_the_track() {
        assert_eq!(scrub_position(0.0, 200_000), 0);
        assert_eq!(scrub_position(0.5, 200_000), 100_000);
        assert_eq!(scrub_position(1.0, 200_000), 200_000);
        // Released past either end (the fraction is already clamped by the
        // track, and again here): never outside the track
        assert_eq!(scrub_position(-3.0, 200_000), 0);
        assert_eq!(scrub_position(7.0, 200_000), 200_000);
        assert_eq!(scrub_position(0.3, 0), 0);
        assert_eq!(scrub_position(f32::NAN.max(0.0), 1000), 0);
    }
}
