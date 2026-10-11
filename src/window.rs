use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::NotifyWinEvent;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, GetDpiForWindow,
    SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_STATECHANGE, GWLP_USERDATA, GetClientRect, GetCursorPos,
    GetMessageW, GetSystemMetrics, GetWindowLongPtrW, HTCLIENT, HTTRANSPARENT, HWND_TOPMOST,
    IDC_ARROW, KillTimer, LoadCursorW, MA_NOACTIVATE, MSG, OBJID_CLIENT, PBT_APMRESUMEAUTOMATIC,
    PBT_APMRESUMESUSPEND, PostMessageW, PostQuitMessage, RegisterClassExW, SM_CXSCREEN,
    SPI_GETCLIENTAREAANIMATION, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW,
    ShowWindow, SystemParametersInfoW, TranslateMessage, UnregisterClassW, WM_APP,
    WM_CAPTURECHANGED, WM_CLIPBOARDUPDATE, WM_CLOSE, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCDESTROY, WM_NCHITTEST,
    WM_POWERBROADCAST, WM_TIMECHANGE, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
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
use crate::config::{
    ANIMATION_FRAME_INTERVAL_MS, ANIMATION_TIMER_ID, AnimationState, CLIPBOARD_SETTLE_MS,
    CLIPBOARD_TIMER_ID, CLOCK_TIMER_ID, MEDIA_FEEDBACK_FRAME_MS, MEDIA_FEEDBACK_TIMER_ID,
    MEDIA_LIVE_FRAME_MS, MEDIA_LIVE_TIMER_ID, NotchDimensions, NotchState, WINDOW_CLASS_NAME,
    WINDOW_TITLE, calculate_notch_x,
};
use crate::dragdrop::{self, DragEvent, DragSession, DropTarget};
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
use crate::space::{NottSettings, NottSpace, PANEL_SPACE, Scene};
use windows::Win32::System::Ole::{
    OleInitialize, OleUninitialize, RegisterDragDrop, RevokeDragDrop,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, WM_INPUT, WM_MBUTTONDOWN, WM_RBUTTONDOWN, WindowFromPoint,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_NOTOPMOST, SET_WINDOW_POS_FLAGS, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
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

struct WindowState {
    renderer: Renderer,
    /// 1 ms timer resolution, held only during the notch animation.
    high_res_timer: HighResTimer,
    state: NotchState,
    hovered: bool,
    dimensions: NotchDimensions,
    pos_x: i32,
    pos_y: i32,
    animation: Option<AnimationState>,
    last_frame_time: Option<std::time::Instant>,
    clock: ClockEngine,
    media: MediaEngine,
    /// Display model derived from `media` (None = no session).
    media_content: Option<MediaContent>,
    /// Active space (every space renders the existing UI for now). Starts in
    /// Home, never persisted; media updates never touch it. Changed only through
    /// `select_space`, which keeps the renderer's selector in sync.
    space: NottSpace,
    /// Last tick of the control feedback timer (Some only while it runs).
    feedback_tick: Option<std::time::Instant>,
    /// Last tick of the playback live timer (Some only while it runs), and
    /// its current period.
    live_tick: Option<std::time::Instant>,
    live_ms: u32,
    /// Clipboard history (in memory only; filled on `WM_CLIPBOARDUPDATE`).
    clipboard: ClipboardHistory,
    /// Active OLE drag over the notch (image-file drops into the history).
    drag: DragSession,
    /// User settings (memory only; every launch starts from the defaults).
    settings: NottSettings,
    /// The Settings page is open over the active space (which stays selected).
    settings_open: bool,
    /// Outside-click auto-collapse (see `GracePeriod`), and whether the Raw
    /// Input sink that reports outside clicks is registered (expanded only).
    grace: GracePeriod,
    outside_watch: bool,
    /// The Clipboard space asks "Clear clipboard history?" (Cancel / Clear).
    /// Any view change (space, Settings page, collapse) cancels it.
    clear_pending: bool,
}

/// `SetWindowPos` z-order arguments for routine moves and resizes: topmost
/// re-asserts the topmost band; otherwise the z-order is left untouched
/// (`SWP_NOZORDER`), so windows that covered Nott stay above it.
fn z_order(always_on_top: bool) -> (Option<HWND>, SET_WINDOW_POS_FLAGS) {
    if always_on_top {
        (Some(HWND_TOPMOST), SET_WINDOW_POS_FLAGS(0))
    } else {
        (None, SWP_NOZORDER)
    }
}

/// Where the setting moves the window when it flips: into the topmost band,
/// or out of it (`HWND_NOTOPMOST`: top of the normal band, coverable).
fn z_band(always_on_top: bool) -> HWND {
    if always_on_top {
        HWND_TOPMOST
    } else {
        HWND_NOTOPMOST
    }
}

impl WindowState {
    /// Media composition of the settled expanded notch, if it is showing.
    fn active_media_layout(&self) -> Option<MediaLayout> {
        if self.state != NotchState::Expanded
            || self.animation.is_some()
            || self.media_content.is_none()
            || self.space == NottSpace::Clipboard
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

    /// Redraws the settled notch (no-op while the notch animation owns frames).
    fn redraw(&self, hwnd: HWND) {
        if self.animation.is_none() {
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
        let animating =
            self.renderer.feedback().is_animating() || self.renderer.switches_animating();
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
    }

    /// Space capsule under a client-area point: on the expanded notch, settled
    /// or mid width transition (the selector is drawn from the same interpolated
    /// dimensions, so what is clicked is what is seen). Not during expand/collapse.
    fn space_at(&self, lparam: LPARAM) -> Option<NottSpace> {
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
        if self.state != NotchState::Expanded || !width_only || self.renderer.drop_page() {
            return None;
        }
        let x = (lparam.0 & 0xFFFF) as i16 as f32;
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

    /// Selects a space (the selector's only route to `NottSpace::switch_to`); a
    /// no-op when already active. On the settled expanded notch the width
    /// transitions to the space's width on the existing animation timer; while
    /// collapsed (or mid expand/collapse) the space is only stored, and the next
    /// expansion opens straight into its width.
    fn select_space(&mut self, hwnd: HWND, space: NottSpace) {
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
            );
        }
    }

    /// The space whose expanded size the notch takes: the active space's, or
    /// for Settings and the drop page the panel (Clipboard) size, whichever
    /// space is active.
    fn size_space(&self) -> NottSpace {
        self.scene().size_space()
    }

    /// Animate, unless the user (Reduced motion) or the system asked not to.
    fn motion_enabled(&self) -> bool {
        motion_allowed(self.settings.reduced_motion, is_client_animation_enabled())
    }

    /// What the expanded notch shows: the drop page during an image drag,
    /// otherwise the active space.
    fn scene(&self) -> Scene {
        Scene::current(self.renderer.drop_page(), self.settings_open, self.space)
    }

    /// The Settings page's accessible state (Some(settings) while open).
    fn settings_title(&self) -> Option<NottSettings> {
        self.settings_open.then_some(self.settings)
    }

    /// Flips Reduced motion: from now on view changes snap (the existing
    /// reduced-motion path) instead of animating.
    fn toggle_reduced_motion(&mut self, hwnd: HWND) {
        self.settings.reduced_motion = !self.settings.reduced_motion;
        self.renderer
            .set_settings(self.settings_open, self.settings);
        settings_window::sync(self.settings);
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
        );
    }

    /// "Open Full Settings": opens (or focuses) the native Settings window.
    /// The notch and its Settings page stay as they are.
    fn open_full_settings(&mut self, hwnd: HWND) {
        // A failure leaves the notch untouched (nothing to show)
        let _ = settings_window::open(hwnd, self.settings);
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
            }
            // Presentation only: redraw; the live timer follows the
            // visualizer (fast with bars, slow for a lone scrubber)
            settings_window::Control::ShowSourceApp | settings_window::Control::ShowVisualizer => {
                control.toggle(&mut self.settings);
                self.renderer
                    .set_settings(self.settings_open, self.settings);
                settings_window::sync(self.settings);
                self.redraw(hwnd);
                self.kick_live(hwnd);
            }
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
        if trimmed {
            self.clipboard_changed(hwnd);
        }
    }

    /// Feeds the outside-click grace period and carries out its decision:
    /// (re)start or stop the one-shot timer, or collapse through the normal
    /// (Phase 6.1) path.
    fn grace(&mut self, hwnd: HWND, event: GraceEvent) {
        match self.grace.handle(event, self.expanded_or_expanding()) {
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
        }
        self.renderer
            .set_settings(self.settings_open, self.settings);
        settings_window::sync(self.settings);
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
        self.last_frame_time = Some(std::time::Instant::now());
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

    /// Places history entry `index` back on the system clipboard and makes it
    /// the newest entry. The update Windows then sends is Nott's own (owner
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

    /// One OLE drag event (UI thread). Supported image files over the notch are
    /// accepted; the first such moment expands a collapsed notch through the
    /// existing animation, and leave/cancel/drop collapse it again only if the
    /// drag opened it. Files are decoded only on drop, into the existing
    /// clipboard history; the newest is then placed on the system clipboard
    /// through the restore path (its own update is skipped by the owner check).
    fn on_drag(&mut self, hwnd: HWND, event: DragEvent) -> bool {
        let over = |state: &mut Self, at: POINT| {
            let in_notch = state.screen_point_in_drop_zone(at);
            let expanded = state.expanded_or_expanding();
            let before = state.scene();
            let action = state.drag.over(in_notch, expanded);
            // Drop page while an accepted image is over the notch, at its own
            // universal size (an expansion it starts opens straight at it)
            if state.renderer.set_drop_page(action.accept) && !action.expand {
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
            let changed = state.renderer.set_drop_page(false);
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
                self.grace(hwnd, GraceEvent::NottInteraction);
                self.drag.enter(images);
                over(self, at)
            }
            DragEvent::Over { at } => over(self, at),
            DragEvent::Leave => {
                end(self);
                false
            }
            DragEvent::Drop { images, at } => {
                let accept = over(self, at) && !images.is_empty();
                let mut stored = false;
                if accept {
                    // Unsupported / unreadable / malformed / too large: skipped
                    let decoded: Vec<_> = images
                        .iter()
                        .filter_map(|p| dragdrop::decode_image_file(p).ok())
                        .collect();
                    if dragdrop::ingest(&mut self.clipboard, decoded) {
                        stored = true;
                        // Clipboard busy: the history keeps it; no retry
                        let _ = self.restore_clipboard(hwnd, 0);
                        self.clipboard_changed(hwnd);
                    }
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "[dragdrop] {} image file(s) dropped, stored: {}, {} entries",
                        images.len(),
                        stored,
                        self.clipboard.len()
                    );
                }
                end(self);
                stored
            }
        }
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
        let x = (lparam.0 & 0xFFFF) as i16 as f32;
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
        if self.space != NottSpace::Clipboard {
            return None;
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
        self.renderer.set_clipboard(&self.clipboard);
        if self.state == NotchState::Expanded && self.space == NottSpace::Clipboard {
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
            );
        }
    }

    /// Transport control under a client-area point (only when media UI is visible).
    fn media_control_at(&self, lparam: LPARAM) -> Option<MediaControl> {
        let x = (lparam.0 & 0xFFFF) as i16 as f32;
        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
        self.active_media_layout()?.control_at(x, y)
    }
}

/// The Clipboard space's clear flow. Clear: asks first when `confirm` is on
/// (sets `pending`), otherwise forgets the history at once. Cancel: dismisses,
/// history untouched. Confirmed Clear: forgets it. Only Nott's in-memory
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
/// lands outside Nott; that starts a grace period (`AUTO_COLLAPSE_MS`), and a
/// later outside click restarts it. Using Nott (a click in the notch, a drag
/// onto it, the Settings window) cancels it; so does any expand / collapse.
/// When it runs out the notch collapses through the normal path.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GracePeriod {
    pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraceEvent {
    OutsideClick,
    NottInteraction,
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
    /// `expanded`: the notch is expanded or expanding.
    pub fn handle(&mut self, event: GraceEvent, expanded: bool) -> GraceAction {
        match event {
            GraceEvent::OutsideClick if expanded => {
                self.pending = true;
                GraceAction::Arm
            }
            GraceEvent::OutsideClick => GraceAction::None,
            GraceEvent::NottInteraction | GraceEvent::StateChanged => {
                if std::mem::take(&mut self.pending) {
                    GraceAction::Disarm
                } else {
                    GraceAction::None
                }
            }
            // A new duration restarts a pending countdown with it
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
        Self::NottInteraction,
        Self::StateChanged,
        Self::TimerFired,
        Self::DelayChanged,
    ];
}

/// Posted to the notch with a `GraceEvent` index (from Raw Input, the
/// Settings window, or the timer) so it is applied through the state guard.
pub const WM_APP_GRACE: u32 = WM_APP + 40;

/// Whose a click is, by the top-level window under it: the notch itself
/// (handled by its own mouse messages), Nott's Settings window (Nott in use),
/// or anything else (outside). The notch's transparent margins pass clicks
/// through, so they count as outside.
fn classify_click(root: HWND, notch: HWND, settings: Option<HWND>) -> Option<GraceEvent> {
    if root == notch {
        None
    } else if Some(root) == settings {
        Some(GraceEvent::NottInteraction)
    } else {
        Some(GraceEvent::OutsideClick)
    }
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

/// Whether a `WM_INPUT` is a mouse button going down (moves are ignored).
fn raw_button_down(lparam: LPARAM) -> bool {
    use windows::Win32::UI::Input::{
        GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTHEADER, RID_INPUT, RIM_TYPEMOUSE,
    };
    // Left, right, middle, X1, X2 down
    const DOWNS: u16 = 0x0001 | 0x0004 | 0x0010 | 0x0040 | 0x0100;
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
        && unsafe { raw.data.mouse.Anonymous.Anonymous.usButtonFlags } & DOWNS != 0
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
/// asked for reduced motion.
pub(crate) fn motion_allowed(reduced_motion: bool, system_animations: bool) -> bool {
    system_animations && !reduced_motion
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
fn accessible_space(space: NottSpace) -> String {
    let all: Vec<&str> = NottSpace::ALL.iter().map(|s| s.label()).collect();
    format!(
        "{} space selected (spaces: {})",
        space.label(),
        all.join(", ")
    )
}

/// The clear confirmation's accessible text: its message, then its two
/// buttons by name (Cancel first).
const CLEAR_CONFIRM_ACCESSIBLE: &str = "Clear clipboard history? Only Nott's history is removed, the Windows clipboard is not changed - Cancel button - Clear button";

/// Accessible title including the Clipboard space: there it describes the
/// history (count, newest entry's kind, the Clear control), never contents.
fn accessible_title(
    state: NotchState,
    clock: &crate::clock::ClockDateState,
    media: Option<&MediaContent>,
    space: NottSpace,
    clipboard: &ClipboardHistory,
    clear_pending: bool,
    settings: Option<NottSettings>,
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
    if state != NotchState::Expanded || space != NottSpace::Clipboard {
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
    space: NottSpace,
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

/// Notifies Windows accessibility clients (screen readers, UI Automation bridge)
/// of a settled state change and updates the top-level window title.
#[allow(clippy::too_many_arguments)]
fn notify_accessibility_state_changed(
    hwnd: HWND,
    new_state: NotchState,
    clock: &crate::clock::ClockDateState,
    media: Option<&MediaContent>,
    space: NottSpace,
    clipboard: &ClipboardHistory,
    clear_pending: bool,
    settings: Option<NottSettings>,
) {
    let title = accessible_title(
        new_state,
        clock,
        media,
        space,
        clipboard,
        clear_pending,
        settings,
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
    space: NottSpace,
    clipboard: &ClipboardHistory,
    clear_pending: bool,
    settings: Option<NottSettings>,
) {
    let title = accessible_title(
        state,
        clock,
        media,
        space,
        clipboard,
        clear_pending,
        settings,
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
        unsafe {
            let _ = KillTimer(Some(hwnd), ANIMATION_TIMER_ID);
        }
        state.high_res_timer.set(false);
        state.animation = None;
        state.renderer.set_transition(None);
        state.last_frame_time = None;
        state.state = target_state;
        let new_dims = space_dimensions(target_state, state.dimensions.dpi, state.size_space());
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
        );
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
        anim
    };

    state.animation = Some(new_anim);
    state.last_frame_time = Some(std::time::Instant::now());

    // Arm the temporary high-precision timer for ~60 FPS animation steps. A
    // reversal re-arms the same timer ID; the resolution request is not stacked.
    state.high_res_timer.set(true);
    unsafe {
        SetTimer(
            Some(hwnd),
            ANIMATION_TIMER_ID,
            ANIMATION_FRAME_INTERVAL_MS,
            None,
        );
    }
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
    );
    Ok(())
}

/// Private: a `WM_DPICHANGED` that arrived re-entrantly, replayed later (the real
/// message cannot be posted). wParam carries the original DPI pair.
const WM_APP_DEFERRED_DPICHANGED: u32 = WM_APP + 16;

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

            let dims = match with_state(hwnd, |state| state.dimensions) {
                Access::Ran(d) => Some(d),
                // In use by an outer handler: last completed snapshot
                Access::Busy => HIT_DIMS.get(),
                Access::Missing => None,
            };
            let is_inside = if let Some(dims) = dims {
                dims.contains_point(pt.x as f32, pt.y as f32)
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
            // handled. Only button presses outside the notch matter.
            if raw_button_down(lparam) {
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

        WM_APP_GRACE => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                if let Some(event) = GraceEvent::ALL.get(wparam.0) {
                    state.grace(hwnd, *event);
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
            if wparam.0 == ANIMATION_TIMER_ID {
                with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                    let now = std::time::Instant::now();
                    let delta_ms = if let Some(last) = state.last_frame_time {
                        now.duration_since(last).as_secs_f32() * 1000.0
                    } else {
                        ANIMATION_FRAME_INTERVAL_MS as f32
                    };
                    state.last_frame_time = Some(now);

                    let still_active = if let Some(anim) = &mut state.animation {
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

                    let _ = state.renderer.render(
                        hwnd,
                        new_x,
                        new_y,
                        &state.dimensions,
                        state.hovered,
                        state.clock.state(),
                    );

                    if !still_active {
                        // Animation complete: snap cleanly to target state and deactivate timer
                        if let Some(anim) = state.animation.take() {
                            state.state = anim.target_state;
                            state.dimensions = space_dimensions(
                                anim.target_state,
                                state.dimensions.dpi,
                                state.size_space(),
                            );
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
                            );
                        }
                        state.last_frame_time = None;
                        state.high_res_timer.set(false);
                        unsafe {
                            let _ = KillTimer(Some(hwnd), ANIMATION_TIMER_ID);
                            let _ = SetWindowPos(
                                hwnd,
                                z_order(state.settings.always_on_top).0,
                                new_x,
                                new_y,
                                state.dimensions.width,
                                state.dimensions.height,
                                SWP_NOACTIVATE
                                    | SWP_SHOWWINDOW
                                    | z_order(state.settings.always_on_top).1,
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
                    }
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
                    let animating = feedback || switches;
                    state.redraw(hwnd);
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
                    | PanelHit::FullSettings => None,
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
                state.grace(hwnd, GraceEvent::NottInteraction);
            });
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }

        WM_LBUTTONDOWN => {
            with_state_or_defer(hwnd, msg, wparam, lparam, |state| {
                // Cancel a pending auto-collapse first, then the click as
                // always (a collapsing click still collapses)
                state.grace(hwnd, GraceEvent::NottInteraction);
                if let Some(control) = state.media_control_at(lparam) {
                    // Tactile compression; capture so the release is always seen
                    state.renderer.feedback().press(control);
                    unsafe {
                        SetCapture(hwnd);
                    }
                    // Press compresses smoothly on the shared feedback timer
                    state.kick_feedback(hwnd);
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
                let pressed = state.renderer.feedback().release();
                if let Some(pressed) = pressed {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    // Press started on a control: never toggles the notch. The command
                    // fires only if released over the same control.
                    if state.media_control_at(lparam) == Some(pressed) {
                        state.media.send_command(pressed);
                    }
                    state.kick_feedback(hwnd);
                } else if let Some(control) = state.media_control_at(lparam) {
                    // Transport control: send to the current session; never toggles the notch.
                    // Resulting playback/metadata state arrives through media events.
                    state.media.send_command(control);
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
                    let new_dims = space_dimensions(state.state, new_dpi, state.size_space());

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
                    let new_dims = space_dimensions(state.state, dpi, state.size_space());

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
                        state.media_content = content;
                        let changed = state.renderer.set_media(state.media_content.as_ref());
                        // Play/pause starts the visualizer fade in either notch state
                        state.kick_live(hwnd);
                        // Media is only visible in the settled expanded state; an in-flight
                        // animation picks the new content up on its next frame.
                        let visible =
                            state.state == NotchState::Expanded && state.animation.is_none();
                        if changed && visible {
                            state.redraw(hwnd);
                            // A new track fades in on the shared feedback timer
                            state.kick_feedback(hwnd);
                        } else if !visible {
                            // Off-screen track changes never run a fade
                            state.renderer.feedback().finish_track_fade();
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
            // Nott's own restore: the history already holds (and fronts) it.
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
            // The Settings window shuts down with Nott
            settings_window::close();
            unsafe {
                let _ = KillTimer(Some(hwnd), ANIMATION_TIMER_ID);
                let _ = KillTimer(Some(hwnd), CLOCK_TIMER_ID);
                let _ = KillTimer(Some(hwnd), MEDIA_FEEDBACK_TIMER_ID);
                let _ = KillTimer(Some(hwnd), MEDIA_LIVE_TIMER_ID);
                let _ = KillTimer(Some(hwnd), CLIPBOARD_TIMER_ID);
                let _ = KillTimer(Some(hwnd), AUTO_COLLAPSE_TIMER_ID);
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
            let _ = with_state(hwnd, |state| state.high_res_timer.set(false));
            LRESULT(0)
        }

        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
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
    // OLE drag-and-drop needs this (UI) thread in a single-threaded apartment.
    // Failure is non-fatal: Nott simply runs without being a drop target.
    let ole = unsafe { OleInitialize(None) }.is_ok();
    let result = run_window(ole);
    if ole {
        unsafe { OleUninitialize() };
    }
    result
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
        let ex_style = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
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
                Some(HWND_TOPMOST),
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
            accessible_title_for_clock(dimensions.state, clock.state(), None, NottSpace::default());
        let title_wide: Vec<u16> = initial_title
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let _ = SetWindowTextW(hwnd, windows::core::PCWSTR(title_wide.as_ptr()));

        let state = Box::new(WindowState {
            renderer,
            high_res_timer: HighResTimer::default(),
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
            space: NottSpace::default(),
            feedback_tick: None,
            live_tick: None,
            live_ms: MEDIA_LIVE_FRAME_MS,
            clipboard: ClipboardHistory::default(),
            drag: DragSession::default(),
            settings: NottSettings::default(),
            settings_open: false,
            clear_pending: false,
            grace: GracePeriod::default(),
            outside_watch: false,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);

        // Start async media session discovery (non-fatal; completes via WM_APP
        // message) and draw the first frame, through the same guarded access as
        // the window procedure (rendering can re-enter it synchronously).
        let initial = with_state(hwnd, |state| {
            let _ = state.media.initialize(hwnd);
            // Event-driven clipboard history (no polling)
            let _ = clipboard::start_listening(hwnd);
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

        // 7. Display window without stealing focus
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

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

        // Clean up registered window class
        let _ = UnregisterClassW(WINDOW_CLASS_NAME, Some(hinstance.into()));

        loop_result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let home = NottSpace::Home;
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
        for space in NottSpace::ALL {
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
                NottSpace::Clipboard,
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
            NottSpace::Clipboard,
            &history,
            true,
            None,
        );
        assert!(confirming.starts_with(CLEAR_CONFIRM_ACCESSIBLE));
        assert!(confirming.contains("Cancel button") && confirming.contains("Clear button"));
        assert!(!confirming.contains("secret"));
        // Collapsed and the other spaces are unchanged
        assert_eq!(title(&history, NotchState::Collapsed), time);
        for s in [NottSpace::Home, NottSpace::Music] {
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
        let mut settings = NottSettings::default();
        assert!(
            settings.always_on_top,
            "Nott starts topmost, as it always has"
        );
        // Routine moves keep the band: topmost re-asserted, or z-order untouched
        assert_eq!(z_order(true), (Some(HWND_TOPMOST), SET_WINDOW_POS_FLAGS(0)));
        assert_eq!(z_order(false), (None, SWP_NOZORDER));
        // Flipping moves into / out of the topmost band
        assert_eq!(z_band(true), HWND_TOPMOST);
        assert_eq!(z_band(false), HWND_NOTOPMOST);
        settings.always_on_top = !settings.always_on_top;
        assert!(!settings.always_on_top);
        settings.always_on_top = !settings.always_on_top;
        assert!(settings.always_on_top);
    }

    #[test]
    fn test_reduced_motion_setting_uses_the_snap_path() {
        let mut settings = NottSettings::default();
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
            let clipboard = expanded_size_dip(NottSpace::Clipboard, dpi);
            for space in NottSpace::ALL {
                let open = Scene::current(false, true, space);
                assert_eq!(
                    expanded_size_dip(open.size_space(), dpi),
                    clipboard,
                    "{space:?}"
                );
                // Back from Settings: the originating space's own size
                let closed = Scene::current(false, false, space);
                assert_eq!(closed, Scene::Space(space));
                assert_eq!(
                    expanded_size_dip(closed.size_space(), dpi),
                    expanded_size_dip(space, dpi)
                );
            }
        }
        // Still exactly three product spaces
        assert_eq!(NottSpace::ALL.len(), 3);
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
        for space in NottSpace::ALL {
            let t = |always_on_top, reduced_motion| {
                accessible_title(
                    NotchState::Expanded,
                    &clock,
                    Some(&media),
                    space,
                    &history,
                    false,
                    Some(NottSettings {
                        always_on_top,
                        reduced_motion,
                        ..NottSettings::default()
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
                NottSpace::Home,
                &history,
                false,
                Some(NottSettings::default())
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
        let mut space = NottSpace::default();
        let title = |s| accessible_title_for_clock(NotchState::Expanded, &clock, Some(&media), s);
        assert!(title(space).ends_with("Home space selected (spaces: Home, Music, Clipboard)"));
        space.switch_to(NottSpace::Music);
        assert!(title(space).ends_with("Music space selected (spaces: Home, Music, Clipboard)"));
        // The media part is identical in both spaces
        let media_part = |t: String| t.split(" - Home space").next().unwrap().to_string();
        assert_eq!(
            media_part(title(NottSpace::Home)),
            title(NottSpace::Music)
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
        // Ask again, then confirm: Nott's history is gone
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
            g.handle(TimerFired, true),
            Disarm,
            "no fire without a click"
        );
        // An outside click starts the countdown; another restarts it
        assert_eq!(g.handle(OutsideClick, true), Arm);
        assert_eq!(
            g.handle(OutsideClick, true),
            Arm,
            "restart from the latest click"
        );
        assert!(g.pending());
        // Running out collapses, once
        assert_eq!(g.handle(TimerFired, true), Collapse);
        assert_eq!(g.handle(TimerFired, true), Disarm, "a second fire is stale");
        // A click in the notch (or the Settings window) cancels it first
        g.handle(OutsideClick, true);
        assert_eq!(g.handle(NottInteraction, true), Disarm);
        assert_eq!(g.handle(TimerFired, true), Disarm, "cancelled: no collapse");
        // An explicit collapse cancels; a stale fire after re-expanding is
        // ignored
        g.handle(OutsideClick, true);
        assert_eq!(g.handle(StateChanged, false), Disarm, "explicit collapse");
        assert_eq!(g.handle(StateChanged, true), None, "re-expanded");
        assert_eq!(g.handle(TimerFired, true), Disarm, "stale fire ignored");
        // Collapsed: outside clicks start nothing
        assert_eq!(g.handle(OutsideClick, false), None);
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
            Some(GraceEvent::NottInteraction),
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
    fn test_collapse_delay_durations_and_restart_on_change() {
        use crate::config::CollapseDelay;
        use GraceAction::*;
        use GraceEvent::*;
        assert_eq!(NottSettings::default().collapse_delay, CollapseDelay::Five);
        assert_eq!(
            CollapseDelay::ALL.map(CollapseDelay::ms),
            [5_000, 10_000, 15_000]
        );
        for (i, d) in CollapseDelay::ALL.into_iter().enumerate() {
            assert_eq!(CollapseDelay::from_index(i), Some(d));
        }
        assert_eq!(CollapseDelay::from_index(3), Option::None);
        let mut g = GracePeriod::default();
        // Changing the delay with nothing pending starts nothing
        assert_eq!(g.handle(DelayChanged, true), None);
        // Pending: the countdown restarts (the window arms it with the new ms)
        g.handle(OutsideClick, true);
        assert_eq!(g.handle(DelayChanged, true), Arm);
        assert!(g.pending());
        // Later outside clicks restart it; inside clicks / Settings cancel it
        assert_eq!(g.handle(OutsideClick, true), Arm);
        assert_eq!(g.handle(NottInteraction, true), Disarm);
        assert_eq!(
            g.handle(DelayChanged, true),
            None,
            "cancelled stays cancelled"
        );
        // Explicit collapse cancels at once
        g.handle(OutsideClick, true);
        assert_eq!(g.handle(StateChanged, false), Disarm);
        assert_eq!(g.handle(TimerFired, false), Disarm, "no late collapse");
    }
}
