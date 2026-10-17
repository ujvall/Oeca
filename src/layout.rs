#![allow(dead_code)]

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

use crate::config::{
    BASE_CLIPBOARD_BOTTOM_PAD, BASE_CLIPBOARD_ROW_GAP, BASE_CLIPBOARD_ROW_HEIGHT,
    BASE_CLIPBOARD_SIDE_INSET, BASE_CLIPBOARD_WIDTH, CLIPBOARD_TILES, CLIPBOARD_VISIBLE_ROWS,
};
use crate::config::{
    BASE_EXPANDED_HEIGHT, BASE_EXPANDED_PADDING_V, BASE_EXPANDED_SHADOW_MARGIN_BOTTOM,
    BASE_EXPANDED_SHADOW_MARGIN_X, BASE_EXPANDED_TOP_TRANSITION_HEIGHT,
    BASE_EXPANDED_TOP_TRANSITION_RADIUS, BASE_EXPANDED_WIDTH, BASE_MEDIA_ARTIST_HEIGHT,
    BASE_MEDIA_ARTWORK_GAP, BASE_MEDIA_ARTWORK_INSET, BASE_MEDIA_ARTWORK_SIZE,
    BASE_MEDIA_CLOCK_COLUMN_WIDTH, BASE_MEDIA_COLUMN_GAP, BASE_MEDIA_CONTROL_GAP,
    BASE_MEDIA_CONTROL_HEIGHT, BASE_MEDIA_CONTROL_VISUAL_SIZE, BASE_MEDIA_CONTROL_WIDTH,
    BASE_MEDIA_CONTROLS_SPACING, BASE_MEDIA_DIVIDER_AFTER_CONTROLS, BASE_MEDIA_DIVIDER_INSET,
    BASE_MEDIA_DIVIDER_TEXT_GAP, BASE_MEDIA_DIVIDER_WIDTH, BASE_MEDIA_ICON_SIZE,
    BASE_MEDIA_SOURCE_HEIGHT, BASE_MEDIA_TIMELINE_LABEL_GAP, BASE_MEDIA_TIMELINE_LABEL_WIDTH,
    BASE_MEDIA_TITLE_HEIGHT, BASE_MEDIA_VISUALIZER_GAP, BASE_MUSIC_ARTWORK_GAP,
    BASE_MUSIC_ARTWORK_SIZE, BASE_MUSIC_BOTTOM_PAD, BASE_MUSIC_CONTROL_GAP,
    BASE_MUSIC_CONTROL_HEIGHT, BASE_MUSIC_CONTROL_VISUAL_SIZE, BASE_MUSIC_CONTROL_WIDTH,
    BASE_MUSIC_CONTROLS_GAP, BASE_MUSIC_SCRUBBER_GAP, BASE_MUSIC_SCRUBBER_HEIGHT,
    BASE_MUSIC_SIDE_INSET, BASE_MUSIC_TIMELINE_TRACK, BASE_SPACE_PILL_GAP, BASE_SPACE_PILL_HEIGHT,
    BASE_SPACE_PILL_WIDTH, BASE_SPACE_SELECTOR_TOP, BASE_VISUALIZER_BAR_GAP,
    BASE_VISUALIZER_BAR_WIDTH, BASE_VISUALIZER_HEIGHT, NotchDimensions, NotchState,
    SKIP_GLYPH_HALF_WIDTH,
};
use crate::config::{
    BASE_SETTINGS_ACTION_HEIGHT, BASE_SETTINGS_DESCRIPTION_HEIGHT, BASE_SETTINGS_LABEL_GAP,
    BASE_SETTINGS_LABEL_HEIGHT, BASE_SETTINGS_ROW_GAP, BASE_SETTINGS_TITLE_HEIGHT,
    BASE_SETTINGS_TOGGLE_GAP, BASE_SETTINGS_TOGGLE_HEIGHT, BASE_SETTINGS_TOGGLE_WIDTH,
};
use crate::media::MediaControl;
use crate::media::VISUALIZER_BARS;
use crate::space::{OecaSpace, Scene};

/// Floating-point axis-aligned rectangle for layout computation
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectF {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl RectF {
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    #[inline]
    pub fn width(&self) -> f32 {
        (self.right - self.left).max(0.0)
    }

    #[inline]
    pub fn height(&self) -> f32 {
        (self.bottom - self.top).max(0.0)
    }

    #[inline]
    pub fn to_d2d_rect(self) -> D2D_RECT_F {
        D2D_RECT_F {
            left: self.left,
            top: self.top,
            right: self.right,
            bottom: self.bottom,
        }
    }

    #[inline]
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.left && px <= self.right && py >= self.top && py <= self.bottom
    }

    /// Moved right by `dx`.
    pub fn offset_x(self, dx: f32) -> Self {
        Self::new(self.left + dx, self.top, self.right + dx, self.bottom)
    }

    /// Linear interpolation toward `to` (t = 0: self, 1: to).
    pub fn lerp(self, to: Self, t: f32) -> Self {
        let l = |a: f32, b: f32| a + (b - a) * t;
        Self::new(
            l(self.left, to.left),
            l(self.top, to.top),
            l(self.right, to.right),
            l(self.bottom, to.bottom),
        )
    }
}

/// Resolved layout components for the collapsed notch state
#[derive(Debug, Clone, PartialEq)]
pub struct CollapsedLayout {
    /// Safe boundary inside the collapsed notch respecting padding and hardware curvature
    pub content_bounds: RectF,
    /// Centered clock text bounds for live Windows time
    pub clock_bounds: RectF,
    /// Playback visualizer, right end of the content area (inside the notch body)
    pub visualizer_bounds: RectF,
    /// Album art, left end of the content area (whole pixels, square)
    pub artwork_bounds: RectF,
}

/// Space selector capsules (Home, Music, Clipboard) in the expanded notch's
/// top band, and the Settings icon at the band's right end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpaceSelectorLayout {
    pub home: RectF,
    pub music: RectF,
    pub clipboard: RectF,
    /// Settings icon (a band-height square at the right content edge).
    pub settings: RectF,
}

impl SpaceSelectorLayout {
    pub fn bounds(&self, space: OecaSpace) -> RectF {
        match space {
            OecaSpace::Home => self.home,
            OecaSpace::Music => self.music,
            OecaSpace::Clipboard => self.clipboard,
        }
    }

    fn map(&self, f: impl Fn(RectF) -> RectF) -> Self {
        Self {
            home: f(self.home),
            music: f(self.music),
            clipboard: f(self.clipboard),
            settings: f(self.settings),
        }
    }

    /// The space whose capsule contains the client point (only the capsules
    /// themselves are interactive).
    pub fn space_at(&self, px: f32, py: f32) -> Option<OecaSpace> {
        OecaSpace::ALL
            .into_iter()
            .find(|s| self.bounds(*s).contains(px, py))
    }
}

/// Resolves the space selector for expanded dimensions (None when collapsed).
/// It sits above the content area, starting at the cover's left edge (notch
/// wall + cover inset), so it never touches the media composition.
pub fn resolve_space_selector(dimensions: &NotchDimensions) -> Option<SpaceSelectorLayout> {
    resolve_space_selector_in(dimensions, OecaSpace::Home)
}

/// The selector for the active space: its left edge follows that space's
/// content edge (Home: the cover inset; Music: the player column inset).
pub fn resolve_space_selector_in(
    dimensions: &NotchDimensions,
    space: OecaSpace,
) -> Option<SpaceSelectorLayout> {
    if dimensions.state != NotchState::Expanded {
        return None;
    }
    let px = |v: f32| (v * dimensions.scale).round();
    let wall = dimensions.shadow_margin_x + dimensions.curvature.top_transition_radius;
    let inset = match space {
        OecaSpace::Home => BASE_MEDIA_ARTWORK_INSET,
        OecaSpace::Music => BASE_MUSIC_SIDE_INSET,
        OecaSpace::Clipboard => BASE_CLIPBOARD_SIDE_INSET,
    };
    let left = (wall + px(inset)).round();
    let top = px(BASE_SPACE_SELECTOR_TOP);
    let (w, h) = (px(BASE_SPACE_PILL_WIDTH), px(BASE_SPACE_PILL_HEIGHT));
    let home = RectF::new(left, top, left + w, top + h);
    let pill = |i: f32| {
        let l = left + i * (w + px(BASE_SPACE_PILL_GAP));
        RectF::new(l, top, l + w, top + h)
    };
    let right = (dimensions.width as f32 - wall - px(inset)).round();
    Some(SpaceSelectorLayout {
        home,
        music: pill(1.0),
        clipboard: pill(2.0),
        settings: RectF::new(right - h, top, right, top + h),
    })
}

/// The selector mid space transition: the pills glide between their places in
/// the outgoing and incoming space (each laid out at its settled size, centred
/// in the current window) and the highlight glides from the outgoing space's
/// pill to the incoming one's. With `from == to` (or `mix` 1) it is exactly the
/// settled selector. Returns the pills and the highlight.
pub fn blended_selector(
    dimensions: &NotchDimensions,
    from: Scene,
    to: Scene,
    space: OecaSpace,
    mix: f32,
) -> Option<(SpaceSelectorLayout, RectF)> {
    if dimensions.state != NotchState::Expanded {
        return None;
    }
    // Each scene lays the header out at its own settled size (Settings and
    // the drop page: the panel size)
    let place = |scene: Scene| {
        let size = scene.size_space();
        let settled = space_dimensions(NotchState::Expanded, dimensions.dpi, size);
        let dx = ((dimensions.width - settled.width) as f32 / 2.0).round();
        let sel = resolve_space_selector_in(&settled, size)?.map(|r| r.offset_x(dx));
        // Highlight: the space's pill, or the Settings icon on Settings
        let highlight = match scene {
            Scene::Settings => sel.settings,
            Scene::Space(s) => sel.bounds(s),
            Scene::Drop | Scene::Dropped => sel.bounds(space),
        };
        Some((sel, highlight))
    };
    let ((a, ha), (b, hb)) = (place(from)?, place(to)?);
    let pills = SpaceSelectorLayout {
        home: a.home.lerp(b.home, mix),
        music: a.music.lerp(b.music, mix),
        clipboard: a.clipboard.lerp(b.clipboard, mix),
        settings: a.settings.lerp(b.settings, mix),
    };
    Some((pills, ha.lerp(hb, mix)))
}

/// One quick setting: title, description, switch at the right content edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SettingRow {
    pub title: RectF,
    pub description: RectF,
    pub toggle: RectF,
}

/// The Settings page: a section label, the quick settings, then the
/// "Open Full Settings" action row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SettingsLayout {
    pub label: RectF,
    pub always_on_top: SettingRow,
    pub reduced_motion: SettingRow,
    /// Whole action row (text left, chevron at the right content edge).
    pub full_settings: RectF,
}

/// Resolves the Settings page inside expanded dimensions (None when collapsed).
pub fn resolve_settings_layout(dimensions: &NotchDimensions) -> Option<SettingsLayout> {
    let ResolvedLayout::Expanded { components, .. } = resolve_layout(dimensions) else {
        return None;
    };
    let px = |v: f32| (v * dimensions.scale).round();
    let wall = dimensions.shadow_margin_x + dimensions.curvature.top_transition_radius;
    let inset = px(BASE_CLIPBOARD_SIDE_INSET);
    let (left, right) = (
        (wall + inset).round(),
        (dimensions.width as f32 - wall - inset).round(),
    );
    let top = components.content_bounds.top.round();
    let label = RectF::new(left, top, right, top + px(BASE_SETTINGS_LABEL_HEIGHT));
    let (title_h, desc_h) = (
        px(BASE_SETTINGS_TITLE_HEIGHT),
        px(BASE_SETTINGS_DESCRIPTION_HEIGHT),
    );
    let (tw, th) = (
        px(BASE_SETTINGS_TOGGLE_WIDTH),
        px(BASE_SETTINGS_TOGGLE_HEIGHT),
    );
    let row = |row_top: f32| {
        let toggle_top = (row_top + (title_h + desc_h - th) / 2.0).round();
        let toggle = RectF::new(right - tw, toggle_top, right, toggle_top + th);
        let text_right = (toggle.left - px(BASE_SETTINGS_TOGGLE_GAP)).max(left);
        let title = RectF::new(left, row_top, text_right, row_top + title_h);
        SettingRow {
            title,
            description: RectF::new(left, title.bottom, text_right, title.bottom + desc_h),
            toggle,
        }
    };
    let always_on_top = row(label.bottom + px(BASE_SETTINGS_LABEL_GAP));
    let reduced_motion = row(always_on_top.description.bottom + px(BASE_SETTINGS_ROW_GAP));
    let action_top = reduced_motion.description.bottom + px(BASE_SETTINGS_ROW_GAP);
    Some(SettingsLayout {
        label,
        always_on_top,
        reduced_motion,
        full_settings: RectF::new(
            left,
            action_top,
            right,
            action_top + px(BASE_SETTINGS_ACTION_HEIGHT),
        ),
    })
}

/// What a click in the Clipboard space lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelHit {
    /// The header's Settings icon: opens / closes the Settings page.
    Settings,
    /// The Settings page's Always-on-top switch.
    AlwaysOnTop,
    /// The Settings page's Reduced-motion switch.
    ReducedMotion,
    /// The Settings page's "Open Full Settings" action row.
    FullSettings,
    /// A history entry's outlined box (index into `ClipboardHistory`, newest
    /// first): restores it.
    Row(usize),
    /// The entry's copy button: restores it too.
    Copy(usize),
    /// The entry's trash button: forgets that entry.
    Remove(usize),
    /// The header's X: clears the history (asking first when the setting
    /// says so).
    Clear,
    /// The clear confirmation's Cancel: keeps the history.
    CancelClear,
    /// The clear confirmation's Clear: forgets the history.
    ConfirmClear,
    /// Home's timer: the primary button (Start / Pause / Resume / Restart),
    /// the Cancel button, the minute scale, and a mode pill (Focus / Break).
    TimerPrimary,
    TimerCancel,
    TimerScale,
    TimerMode(crate::timer::Mode),
}

impl PanelHit {
    /// Icon buttons (they grow on hover/press); a row box, the timer's
    /// scale or a mode pill is not one.
    pub fn is_button(self) -> bool {
        !matches!(self, Self::Row(_) | Self::TimerScale | Self::TimerMode(_))
    }
}

/// Home's Focus / Break timer, in two columns. Left: the mode pills over
/// the scale (labels over the ticks, a tick per stop, the pointer under its
/// fixed centre: the scale slides so the value sits there). Right: the
/// numerals (right-aligned) over the round buttons (primary at the right
/// edge, Cancel beside it).
///
/// ```text
///       [Focus] [Break]
///  15   20   25   30   35          25:00
///  |||||||||||||||||||||||
///             ^                 (x) (>)
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimerLayout {
    pub panel: RectF,
    /// The Focus and Break pills (`Mode::index` order).
    pub modes: [RectF; 2],
    pub labels: RectF,
    pub ruler: RectF,
    pub pointer: RectF,
    pub primary: RectF,
    pub cancel: RectF,
    /// Where the numerals draw (right-aligned; the same whatever is shown).
    pub numerals: RectF,
    /// The scale: px from one stop to the next.
    pub step: f32,
}

impl TimerLayout {
    pub fn offset_x(self, dx: f32) -> Self {
        Self {
            panel: self.panel.offset_x(dx),
            modes: self.modes.map(|r| r.offset_x(dx)),
            labels: self.labels.offset_x(dx),
            ruler: self.ruler.offset_x(dx),
            pointer: self.pointer.offset_x(dx),
            primary: self.primary.offset_x(dx),
            cancel: self.cancel.offset_x(dx),
            numerals: self.numerals.offset_x(dx),
            step: self.step,
        }
    }

    /// The scale's fixed centre (where the value is).
    pub fn scale_centre(&self) -> f32 {
        ((self.ruler.left + self.ruler.right) / 2.0).round()
    }

    /// The whole scale, a generous target: from its labels down to the
    /// panel's bottom (drag or tap it, in any phase).
    pub fn scale(&self) -> RectF {
        RectF::new(
            self.ruler.left,
            self.labels.top,
            self.ruler.right,
            self.panel.bottom,
        )
    }

    /// x of stop `i` while the scale shows `value` at its centre.
    pub fn stop_x(&self, i: f32, value: f32) -> f32 {
        self.scale_centre() + (i - value) * self.step
    }

    /// The (fractional) stop at `x` while the scale shows `value`.
    pub fn stop_at(&self, x: f32, value: f32) -> f32 {
        value + (x - self.scale_centre()) / self.step
    }

    /// What a point hits: a mode pill, the buttons shown (Cancel outside
    /// setup), the scale.
    pub fn hit(&self, px: f32, py: f32, idle: bool) -> Option<PanelHit> {
        if let Some(mode) = crate::timer::Mode::ALL
            .into_iter()
            .find(|m| self.modes[m.index()].contains(px, py))
        {
            return Some(PanelHit::TimerMode(mode));
        }
        if self.primary.contains(px, py) {
            Some(PanelHit::TimerPrimary)
        } else if !idle && self.cancel.contains(px, py) {
            Some(PanelHit::TimerCancel)
        } else if self.scale().contains(px, py) {
            Some(PanelHit::TimerScale)
        } else {
            None
        }
    }
}

/// Home with its timer. With media the timer takes the empty band between
/// the divider and the clock column (the media composition is untouched).
/// Without media the content area becomes two columns around a centre
/// divider: the clock (unchanged type, centred in the left column) and the
/// timer (centred in the right one).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HomeTimerLayout {
    pub timer: TimerLayout,
    /// Without media: the clock's time and date bounds (the left column).
    pub clock: Option<(RectF, RectF)>,
    /// Without media: the centre divider.
    pub divider: Option<RectF>,
}

/// Resolves Home's timer panel for settled Home dimensions (None when
/// collapsed). `media`: the shape of the media composition shown, if any.
pub fn resolve_home_timer_layout(
    dimensions: &NotchDimensions,
    media: Option<MediaShape>,
) -> Option<HomeTimerLayout> {
    use crate::config::{
        BASE_TIMER_BUTTON_GAP, BASE_TIMER_BUTTON_SIZE, BASE_TIMER_COLUMN_GAP,
        BASE_TIMER_COLUMN_SPACING, BASE_TIMER_DISPLAY_HEIGHT, BASE_TIMER_LABEL_GAP,
        BASE_TIMER_LABEL_HEIGHT, BASE_TIMER_PANEL_MAX_WIDTH, BASE_TIMER_PILL_GAP,
        BASE_TIMER_PILL_HEIGHT, BASE_TIMER_PILL_WIDTH, BASE_TIMER_POINTER_GAP,
        BASE_TIMER_POINTER_HEIGHT, BASE_TIMER_POINTER_WIDTH, BASE_TIMER_RULER_HEIGHT,
        BASE_TIMER_SIDE_WIDTH, BASE_TIMER_STEP, BASE_TIMER_TICK_GAP,
    };
    let ResolvedLayout::Expanded { components, .. } = resolve_layout(dimensions) else {
        return None;
    };
    let px = |v: f32| (v * dimensions.scale).round();
    let c = components.content_bounds;
    // Centred on the settled Home body, like the clock and media block
    let bottom = (home_settled_notch_h(dimensions.scale) - dimensions.padding_v).max(c.top);
    let (region, clock, divider) = match media {
        Some(shape) => {
            let m = resolve_media_layout_in(dimensions, shape, OecaSpace::Home)?;
            let left = m.divider_bounds.right + px(BASE_MEDIA_DIVIDER_TEXT_GAP);
            (
                RectF::new(left, c.top, m.time_bounds.left.max(left), bottom),
                None,
                None,
            )
        }
        None => {
            let mid = ((c.left + c.right) / 2.0).round();
            let gap = px(BASE_TIMER_COLUMN_GAP);
            let inset = px(BASE_MEDIA_DIVIDER_INSET);
            let clock_col = (c.left, mid - gap);
            let time = RectF::new(
                clock_col.0,
                components.time_bounds.top,
                clock_col.1,
                components.time_bounds.bottom,
            );
            let date = RectF::new(
                clock_col.0,
                components.date_bounds.top,
                clock_col.1,
                components.date_bounds.bottom,
            );
            let line = px(BASE_MEDIA_DIVIDER_WIDTH).max(1.0);
            (
                RectF::new(mid + gap, c.top, c.right, bottom),
                Some((time, date)),
                Some(RectF::new(mid, c.top + inset, mid + line, bottom - inset)),
            )
        }
    };
    let w = px(BASE_TIMER_PANEL_MAX_WIDTH).min(region.width().floor());
    let h = px(72.0).min(c.bottom.floor() - c.top.ceil());
    let left = (region.left + (region.width() - w) / 2.0).round();
    let top = (region.top + (region.height() - h) / 2.0)
        .round()
        .max(c.top.ceil());
    let panel = RectF::new(left, top, left + w, top + h);
    // Right: the numerals over the buttons
    let side = px(BASE_TIMER_SIDE_WIDTH).min(w / 2.0);
    let side_left = panel.right - side;
    let numerals = RectF::new(
        side_left,
        top,
        panel.right,
        top + px(BASE_TIMER_DISPLAY_HEIGHT),
    );
    let (b, bgap) = (
        px(BASE_TIMER_BUTTON_SIZE).min(panel.bottom - numerals.bottom),
        px(BASE_TIMER_BUTTON_GAP),
    );
    let primary = RectF::new(panel.right - b, panel.bottom - b, panel.right, panel.bottom);
    let cancel = RectF::new(
        primary.left - bgap - b,
        primary.top,
        primary.left - bgap,
        primary.bottom,
    );
    // Left: the pills (centred over the scale) over the scale
    let scale_right = side_left - px(BASE_TIMER_COLUMN_SPACING);
    let centre = ((left + scale_right) / 2.0).round();
    // (narrowed to the scale's width in the narrowest band)
    let pgap = px(BASE_TIMER_PILL_GAP);
    let pw = px(BASE_TIMER_PILL_WIDTH).min(((scale_right - left - pgap) / 2.0).floor());
    let pill_h = px(BASE_TIMER_PILL_HEIGHT);
    let pills_left = (centre - (2.0 * pw + pgap) / 2.0).round();
    let pill = |i: f32| {
        let l = pills_left + i * (pw + pgap);
        RectF::new(l, top, l + pw, top + pill_h)
    };
    let labels_top = top + pill_h + px(BASE_TIMER_LABEL_GAP);
    let labels = RectF::new(
        left,
        labels_top,
        scale_right,
        labels_top + px(BASE_TIMER_LABEL_HEIGHT),
    );
    let ruler_top = labels.bottom + px(BASE_TIMER_TICK_GAP);
    let ruler = RectF::new(
        left,
        ruler_top,
        scale_right,
        (ruler_top + px(BASE_TIMER_RULER_HEIGHT)).min(panel.bottom),
    );
    let pw_ptr = px(BASE_TIMER_POINTER_WIDTH);
    let pointer_top = ruler.bottom + px(BASE_TIMER_POINTER_GAP);
    let pointer = RectF::new(
        centre - (pw_ptr / 2.0).round(),
        pointer_top.min(panel.bottom),
        centre - (pw_ptr / 2.0).round() + pw_ptr,
        (pointer_top + px(BASE_TIMER_POINTER_HEIGHT)).min(panel.bottom),
    );
    Some(HomeTimerLayout {
        timer: TimerLayout {
            panel,
            modes: [pill(0.0), pill(1.0)],
            labels,
            ruler,
            pointer,
            primary,
            cancel,
            numerals,
            step: px(BASE_TIMER_STEP),
        },
        clock,
        divider,
    })
}

/// The collapsed notch's countdown slot: the visualizer's place, grown left
/// (toward the centred clock) just enough for the remaining time.
pub fn timer_slot_rect(layout: &CollapsedLayout, scale: f32) -> RectF {
    let right = layout.visualizer_bounds.right;
    let w = (crate::config::BASE_TIMER_SLOT_WIDTH * scale).round();
    RectF::new(
        (right - w).max(layout.content_bounds.left),
        layout.clock_bounds.top,
        right,
        layout.clock_bounds.bottom,
    )
}

/// The volume HUD as one row: the icon (centred in `icon`,
/// `glyph` wide), the level bar, the level number right-aligned in `text`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HudLayout {
    pub icon: RectF,
    pub glyph: f32,
    pub bar: RectF,
    pub text: RectF,
}

fn hud_row(icon: RectF, glyph: f32, top: f32, bottom: f32, right: f32, scale: f32) -> HudLayout {
    let px = |v: f32| (v * scale).round();
    let text = RectF::new(
        right - px(crate::config::BASE_HUD_TEXT_WIDTH),
        top,
        right,
        bottom,
    );
    let h = px(crate::config::BASE_HUD_BAR_HEIGHT).max(2.0);
    let bar_top = ((top + bottom - h) / 2.0).round();
    let left = icon.right + px(crate::config::BASE_HUD_GAP);
    let bar = RectF::new(
        left,
        bar_top,
        (text.left - px(crate::config::BASE_HUD_GAP)).max(left),
        bar_top + h,
    );
    HudLayout {
        icon,
        glyph,
        bar,
        text,
    }
}

/// The HUD number's three digit cells (hundreds, tens, ones), each `cell`
/// wide, right-aligned in `text`: fixed whatever the number, so it never
/// shifts as digits come and go.
pub fn hud_digit_cells(text: RectF, cell: f32) -> [RectF; 3] {
    let at = |k: f32| {
        let left = text.right - (3.0 - k) * cell;
        RectF::new(left, text.top, left + cell, text.bottom)
    };
    [at(0.0), at(1.0), at(2.0)]
}

/// The HUD in the collapsed notch: icon in the artwork's place, number in
/// the visualizer's (as the countdown slot), the bar across the clock.
pub fn collapsed_hud_layout(layout: &CollapsedLayout, scale: f32) -> HudLayout {
    let icon = layout.artwork_bounds;
    hud_row(
        icon,
        (icon.height() * 0.9).round(),
        layout.clock_bounds.top,
        layout.clock_bounds.bottom,
        layout.visualizer_bounds.right,
        scale,
    )
}

/// The HUD on the expanded notch: across the header (pills to Settings icon).
pub fn header_hud_layout(selector: &SpaceSelectorLayout, scale: f32) -> HudLayout {
    let s = selector.settings;
    let left = selector.home.left;
    let icon = RectF::new(left, s.top, left + s.height(), s.bottom);
    let glyph = (crate::config::BASE_SPACE_ICON_SIZE * 1.2 * scale).round();
    hud_row(icon, glyph, s.top, s.bottom, s.right, scale)
}

/// The countdown bubble beside the collapsed notch: a floating circle of
/// its own, centred on the notch's height (clear of the screen edge), that
/// behaves like a drop of the notch's own liquid. It buds out of the
/// notch's wall and is absorbed back into it: while it is close a neck joins
/// it to the wall, which thins and pinches off as it moves away. Window
/// coordinates: the window starts `overlap` px back over the notch's body,
/// so the wall is at x = `wall`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BubbleGeometry {
    /// Window size (px).
    pub window: (i32, i32),
    /// The notch body's right wall (x).
    pub wall: f32,
    /// The circle.
    pub center: (f32, f32),
    pub radius: f32,
    /// The ring (centre line radius, stroke width).
    pub ring: (f32, f32),
    /// The neck's half-height where it meets the wall (0: detached).
    pub neck: f32,
}

/// The bubble at `presence` (0: inside the notch .. 1: floating a gap
/// away; slightly above while the spring settles). On the way in it moves
/// into the wall while shrinking a little, so at 0 it is entirely behind
/// the notch's black body.
pub fn timer_bubble_geometry(dpi: u32, presence: f32) -> BubbleGeometry {
    use crate::config::{
        BASE_TIMER_BUBBLE_GAP, BASE_TIMER_BUBBLE_HEIGHT, BASE_TIMER_BUBBLE_OVERLAP,
        BASE_TIMER_NECK_REACH, BASE_TIMER_RING_INSET, BASE_TIMER_RING_WIDTH,
    };
    let d = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
    let px = |v: f32| (v * d.scale).round();
    let big_r = px(BASE_TIMER_BUBBLE_HEIGHT).min(d.height as f32 - 2.0) / 2.0;
    let gap = px(BASE_TIMER_BUBBLE_GAP);
    let wall = px(BASE_TIMER_BUBBLE_OVERLAP);
    // 0 = floating, 1 = absorbed (below 0 while the spring overshoots out)
    let t = 1.0 - presence.clamp(0.0, 1.05);
    let home = gap + big_r;
    let cx = home - (home + 0.6 * big_r) * t;
    let r = big_r * (1.0 - 0.5 * t.max(0.0));
    // The neck: from nothing at its reach to most of the circle's height
    let clear = cx - r;
    let near = (1.0 - clear / px(BASE_TIMER_NECK_REACH)).clamp(0.0, 1.0);
    let neck = r * 0.85 * near.powf(1.5);
    let ring_w = px(BASE_TIMER_RING_WIDTH).max(1.0);
    BubbleGeometry {
        window: (
            (wall + gap + 2.0 * big_r * 1.05).ceil() as i32 + 2,
            d.height,
        ),
        wall,
        center: (wall + cx, d.height as f32 / 2.0),
        radius: r,
        ring: (
            (r - px(BASE_TIMER_RING_INSET) + ring_w / 2.0).max(0.0),
            ring_w,
        ),
        neck,
    }
}

/// Clipboard space: the clear X sits at the right end of the selector band;
/// the newest entries fill the content area as six squircle tiles, three
/// columns by two rows, newest first (left to right, then down).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipboardLayout {
    /// The header's X (a band-height square).
    pub clear: RectF,
    pub tiles: [RectF; CLIPBOARD_TILES],
    /// The whole tile area (the empty state centres in it).
    pub list: RectF,
    /// The clear confirmation, centred in the tile area in place of the
    /// tiles: its question, the note under it, then Cancel and Clear.
    pub confirm_message: RectF,
    pub confirm_note: RectF,
    pub cancel: RectF,
    pub confirm_clear: RectF,
    /// DIP to px of the dimensions (the tiles' parts scale with it).
    pub scale: f32,
}

/// A tile's parts: its preview (text, or the image's own area), the image
/// caption line, and its round copy and trash buttons along its bottom edge
/// (copy in the corner). The buttons fade in only while the tile is
/// hovered, over space reserved for them, so nothing moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileParts {
    pub tile: RectF,
    pub text: RectF,
    pub image: RectF,
    pub caption: RectF,
    pub copy: RectF,
    pub trash: RectF,
}

impl ClipboardLayout {
    pub fn tile_parts(&self, i: usize) -> TileParts {
        use crate::config::{
            BASE_CLIPBOARD_TILE_BUTTON, BASE_CLIPBOARD_TILE_INSET, BASE_CLIPBOARD_TILE_PAD,
        };
        let px = |v: f32| (v * self.scale).round();
        let t = self.tiles[i.min(CLIPBOARD_TILES - 1)];
        let (b, inset, pad) = (
            px(BASE_CLIPBOARD_TILE_BUTTON),
            px(BASE_CLIPBOARD_TILE_INSET),
            px(BASE_CLIPBOARD_TILE_PAD),
        );
        let copy = RectF::new(
            t.right - inset - b,
            t.bottom - inset - b,
            t.right - inset,
            t.bottom - inset,
        );
        let trash = RectF::new(copy.left - b, copy.top, copy.left, copy.bottom);
        TileParts {
            tile: t,
            text: RectF::new(t.left + pad, t.top + pad, t.right - pad, copy.top),
            // Whole pixels, so its thumbnail is built at exactly its size
            // (a fractional edge would be stretched by a sub-pixel)
            image: RectF::new(
                (t.left + inset).round(),
                (t.top + inset).round(),
                (t.right - inset).round(),
                (copy.top - inset / 2.0).round(),
            ),
            caption: RectF::new(t.left + pad, copy.top, trash.left, copy.bottom),
            copy,
            trash,
        }
    }

    /// Only what is drawn is interactive: the tiles holding an entry (of
    /// `len`) and their buttons, and the X while there is history.
    /// Everything else is notch background. The buttons stay hit-testable
    /// while faded, so a click (or a tap without hover) still works.
    pub fn hit(&self, px: f32, py: f32, len: usize) -> Option<PanelHit> {
        if len > 0 && self.clear.contains(px, py) {
            return Some(PanelHit::Clear);
        }
        let i = self
            .tiles
            .iter()
            .take(len)
            .position(|r| r.contains(px, py))?;
        let parts = self.tile_parts(i);
        if parts.trash.contains(px, py) {
            Some(PanelHit::Remove(i))
        } else if parts.copy.contains(px, py) {
            Some(PanelHit::Copy(i))
        } else {
            Some(PanelHit::Row(i))
        }
    }

    /// While the clear confirmation shows, only its two buttons are live.
    pub fn confirm_hit(&self, px: f32, py: f32) -> Option<PanelHit> {
        if self.cancel.contains(px, py) {
            Some(PanelHit::CancelClear)
        } else {
            self.confirm_clear
                .contains(px, py)
                .then_some(PanelHit::ConfirmClear)
        }
    }
}

/// Resolves the Clipboard space for expanded dimensions (None when collapsed).
pub fn resolve_clipboard_layout(dimensions: &NotchDimensions) -> Option<ClipboardLayout> {
    let ResolvedLayout::Expanded { components, .. } = resolve_layout(dimensions) else {
        return None;
    };
    let band = resolve_space_selector_in(dimensions, OecaSpace::Clipboard)?.clipboard;
    let px = |v: f32| (v * dimensions.scale).round();
    let wall = dimensions.shadow_margin_x + dimensions.curvature.top_transition_radius;
    let inset = px(BASE_CLIPBOARD_SIDE_INSET);
    let (left, right) = (
        (wall + inset).round(),
        (dimensions.width as f32 - wall - inset).round(),
    );
    // The X sits just left of the header's Settings icon
    let gear = resolve_space_selector_in(dimensions, OecaSpace::Clipboard)?.settings;
    let clear_right = gear.left - px(BASE_SPACE_PILL_GAP);
    let clear = RectF::new(
        clear_right - band.height(),
        band.top,
        clear_right,
        band.bottom,
    );
    // The content area: as tall as the former five rows at this DPI (the
    // space's height is unchanged)
    let top = components.content_bounds.top.round();
    let n = CLIPBOARD_VISIBLE_ROWS as f32;
    let area_h = n * px(BASE_CLIPBOARD_ROW_HEIGHT) + (n - 1.0) * px(BASE_CLIPBOARD_ROW_GAP);
    let list = RectF::new(left, top, right, top + area_h);
    // 3 x 2 whole-pixel tiles, centred in it
    let gap = px(crate::config::BASE_CLIPBOARD_TILE_GAP);
    let cols = crate::config::CLIPBOARD_TILE_COLUMNS as f32;
    let rows = (CLIPBOARD_TILES / crate::config::CLIPBOARD_TILE_COLUMNS) as f32;
    let tw = ((list.width() - (cols - 1.0) * gap) / cols).floor();
    let th = ((list.height() - (rows - 1.0) * gap) / rows).floor();
    let x0 = (list.left + (list.width() - (cols * tw + (cols - 1.0) * gap)) / 2.0).round();
    let y0 = (list.top + (list.height() - (rows * th + (rows - 1.0) * gap)) / 2.0).round();
    let tiles = std::array::from_fn(|i| {
        let (c, r) = (
            (i % crate::config::CLIPBOARD_TILE_COLUMNS) as f32,
            (i / crate::config::CLIPBOARD_TILE_COLUMNS) as f32,
        );
        let (l, t) = (x0 + c * (tw + gap), y0 + r * (th + gap));
        RectF::new(l, t, l + tw, t + th)
    });
    // Confirmation block (question 20, note 18, gap 12, buttons 28 DIP),
    // vertically centred in the tile area
    let (line, note, gap, button) = (px(20.0), px(18.0), px(12.0), px(28.0));
    let block_top = (list.top + (list.height() - (line + note + gap + button)) / 2.0).round();
    let confirm_message = RectF::new(left, block_top, right, block_top + line);
    let confirm_note = RectF::new(
        left,
        confirm_message.bottom,
        right,
        confirm_message.bottom + note,
    );
    let (bw, bgap) = (px(92.0), px(8.0));
    let cx = ((left + right) / 2.0).round();
    let btop = confirm_note.bottom + gap;
    let cancel = RectF::new(cx - bgap / 2.0 - bw, btop, cx - bgap / 2.0, btop + button);
    let confirm_clear = RectF::new(cx + bgap / 2.0, btop, cx + bgap / 2.0 + bw, btop + button);
    Some(ClipboardLayout {
        clear,
        tiles,
        list,
        confirm_message,
        confirm_note,
        cancel,
        confirm_clear,
        scale: dimensions.scale,
    })
}

/// Whole-pixel (bar width, bar gap, full height) of the visualizer at `scale`.
pub fn visualizer_metrics(scale: f32) -> (f32, f32, f32) {
    let px = |v: f32| (v * scale).round().max(1.0);
    (
        px(BASE_VISUALIZER_BAR_WIDTH),
        px(BASE_VISUALIZER_BAR_GAP),
        px(BASE_VISUALIZER_HEIGHT),
    )
}

/// Visualizer slot whose right edge is `right`, vertically centered on `cy`.
fn visualizer_rect(right: f32, cy: f32, scale: f32) -> RectF {
    let (bar, gap, h) = visualizer_metrics(scale);
    let n = VISUALIZER_BARS as f32;
    let w = n * bar + (n - 1.0) * gap;
    let top = (cy - h / 2.0).round();
    RectF::new(right - w, top, right, top + h)
}

/// Resolved layout components for the expanded notch state: live time and date
#[derive(Debug, Clone, PartialEq)]
pub struct ExpandedLayout {
    /// Safe boundary inside the notch respecting padding and hardware curvature
    pub content_bounds: RectF,
    /// Centered live time bounds (e.g. "12:47 AM")
    pub time_bounds: RectF,
    /// Centered live date bounds (e.g. "Monday, October 6")
    pub date_bounds: RectF,
}

/// Overall layout resolved from NotchDimensions and NotchState
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedLayout {
    Collapsed {
        bounds: RectF,
        components: CollapsedLayout,
    },
    Expanded {
        bounds: RectF,
        components: ExpandedLayout,
    },
}

impl ResolvedLayout {
    /// Returns the outer bounding rectangle of the notch surface
    pub fn bounds(&self) -> RectF {
        match self {
            Self::Collapsed { bounds, .. } => *bounds,
            Self::Expanded { bounds, .. } => *bounds,
        }
    }
}

/// Expanded composition when a media session is available. Visual rectangles lie
/// inside `ExpandedLayout::content_bounds`; control *hit* rectangles may extend into
/// the notch's bottom padding (still inside the notch contour). Nothing here
/// changes the notch itself.
///
/// ```text
/// [artwork] [title                     ]   [time]
///           [artist — album            ]   [date]
///           [source app                ]
///              [prev] [play/pause] [next]
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaLayout {
    /// Square artwork slot (None when the track has no artwork; text then starts
    /// at the content edge)
    pub artwork_bounds: Option<RectF>,
    /// Primary line (single line, ellipsis-trimmed)
    pub title_bounds: RectF,
    /// Artist / album line
    pub artist_bounds: RectF,
    /// Source application line (smallest hierarchy)
    pub source_bounds: RectF,
    /// Secondary live time / short date, right column
    pub time_bounds: RectF,
    pub date_bounds: RectF,
    /// Hit areas of the transport controls, left-aligned with the text column
    pub previous_bounds: RectF,
    pub play_pause_bounds: RectF,
    pub next_bounds: RectF,
    /// Diameter of each control's visual backdrop (top-aligned in its hit area)
    pub control_visual_size: f32,
    /// Playback visualizer at the end of the title row
    pub visualizer_bounds: RectF,
    /// Inline scrubber strip (elapsed label, track, remaining label) in the
    /// controls row, right of the controls; may be too narrow to draw
    pub timeline_bounds: RectF,
    /// Home: vertical divider between the text column and the time/date column
    /// (titles truncate before it). Empty in Music.
    pub divider_bounds: RectF,
}

impl MediaLayout {
    /// Moved right by `dx` (a composition centred in a wider/narrower window).
    pub fn offset_x(&self, dx: f32) -> Self {
        Self {
            artwork_bounds: self.artwork_bounds.map(|r| r.offset_x(dx)),
            title_bounds: self.title_bounds.offset_x(dx),
            artist_bounds: self.artist_bounds.offset_x(dx),
            source_bounds: self.source_bounds.offset_x(dx),
            time_bounds: self.time_bounds.offset_x(dx),
            date_bounds: self.date_bounds.offset_x(dx),
            previous_bounds: self.previous_bounds.offset_x(dx),
            play_pause_bounds: self.play_pause_bounds.offset_x(dx),
            next_bounds: self.next_bounds.offset_x(dx),
            control_visual_size: self.control_visual_size,
            visualizer_bounds: self.visualizer_bounds.offset_x(dx),
            timeline_bounds: self.timeline_bounds.offset_x(dx),
            divider_bounds: self.divider_bounds.offset_x(dx),
        }
    }

    /// One composition morphing into another (t = 0: `a`, 1: `b`): shared
    /// elements (cover, text, controls) move between their places; a slot only
    /// one side has (clock column, divider, scrubber, visualizer) stays where
    /// that side puts it (the renderer fades it).
    pub fn morph(a: &Self, b: &Self, t: f32) -> Self {
        let shared = |x: RectF, y: RectF| x.lerp(y, t);
        let either = |x: RectF, y: RectF| match (x.width() > 0.0, y.width() > 0.0) {
            (true, false) => x,
            (false, true) => y,
            _ => x.lerp(y, t),
        };
        Self {
            // Whole-pixel cover edges (crisp corners mid-morph, as when settled)
            artwork_bounds: match (a.artwork_bounds, b.artwork_bounds) {
                (Some(x), Some(y)) => {
                    let r = x.lerp(y, t);
                    Some(RectF::new(
                        r.left.round(),
                        r.top.round(),
                        r.right.round(),
                        r.bottom.round(),
                    ))
                }
                (x, y) => x.or(y),
            },
            title_bounds: shared(a.title_bounds, b.title_bounds),
            artist_bounds: shared(a.artist_bounds, b.artist_bounds),
            // Music has no source row (zero height): Home's stays and fades
            source_bounds: match (
                a.source_bounds.height() > 0.0,
                b.source_bounds.height() > 0.0,
            ) {
                (true, false) => a.source_bounds,
                (false, true) => b.source_bounds,
                _ => shared(a.source_bounds, b.source_bounds),
            },
            time_bounds: either(a.time_bounds, b.time_bounds),
            date_bounds: either(a.date_bounds, b.date_bounds),
            previous_bounds: shared(a.previous_bounds, b.previous_bounds),
            play_pause_bounds: shared(a.play_pause_bounds, b.play_pause_bounds),
            next_bounds: shared(a.next_bounds, b.next_bounds),
            control_visual_size: a.control_visual_size
                + (b.control_visual_size - a.control_visual_size) * t,
            visualizer_bounds: either(a.visualizer_bounds, b.visualizer_bounds),
            timeline_bounds: either(a.timeline_bounds, b.timeline_bounds),
            divider_bounds: either(a.divider_bounds, b.divider_bounds),
        }
    }

    pub fn control_bounds(&self, control: MediaControl) -> RectF {
        match control {
            MediaControl::Previous => self.previous_bounds,
            MediaControl::PlayPause => self.play_pause_bounds,
            MediaControl::Next => self.next_bounds,
        }
    }

    /// Square visual area of a control (backdrop/icon), top-aligned in its hit area
    /// so it stays inside the content bounds.
    pub fn control_visual_bounds(&self, control: MediaControl) -> RectF {
        let hit = self.control_bounds(control);
        let d = self.control_visual_size;
        let cx = (hit.left + hit.right) / 2.0;
        RectF::new(cx - d / 2.0, hit.top, cx + d / 2.0, hit.top + d)
    }

    /// The scrubber's track (Music): between its two time labels, exactly
    /// where `draw_timeline` draws it, with a hit area a little taller than
    /// the strip (never reaching the controls). None without a scrubber
    /// (Home) or when it is too narrow to draw.
    pub fn scrub_track(&self, scale: f32) -> Option<RectF> {
        use crate::config::{BASE_MEDIA_TIMELINE_MIN_TRACK, BASE_MUSIC_CONTROLS_GAP};
        let s = self.timeline_bounds;
        if s.width() <= 0.0 || s.height() <= 0.0 {
            return None;
        }
        let px = |v: f32| (v * scale).round();
        let edge = px(BASE_MEDIA_TIMELINE_LABEL_WIDTH) + px(BASE_MEDIA_TIMELINE_LABEL_GAP);
        let (left, right) = (s.left + edge, s.right - edge);
        if right - left < px(BASE_MEDIA_TIMELINE_MIN_TRACK) {
            return None;
        }
        let reach = px(BASE_MUSIC_CONTROLS_GAP) / 2.0;
        Some(RectF::new(left, s.top - px(4.0), right, s.bottom + reach))
    }

    /// Where on the track `x` is, 0..=1 (clamped: a drag may leave it).
    pub fn scrub_fraction(track: RectF, x: f32) -> f32 {
        ((x - track.left) / track.width().max(1.0)).clamp(0.0, 1.0)
    }

    /// Returns the transport control under the client point, if any.
    pub fn control_at(&self, px: f32, py: f32) -> Option<MediaControl> {
        MediaControl::ALL
            .into_iter()
            .find(|c| self.control_bounds(*c).contains(px, py))
    }
}

/// Resolves the media composition inside the expanded content area. Returns `None`
/// for collapsed dimensions. Derived from the current layout system (content bounds)
/// and DPI-scaled DIP tokens; during animation the stack anchors at the content top
/// and is clipped by the interpolated content bounds.
/// What the media composition has to show; drives artwork slot and vertical balance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaShape {
    pub artwork: bool,
    /// Lines under the title (artist/album, source): 0..=2.
    pub secondary_lines: u32,
}

impl MediaShape {
    pub const FULL: Self = Self {
        artwork: true,
        secondary_lines: 2,
    };
}

/// Expanded window width (DIP at 96 DPI) of a space. Home is the existing
/// expanded notch. Music is derived from its player column: shadow margin +
/// shoulder on both sides, the column inset on both sides, and its widest row,
/// the full-width scrubber (two edge labels with their gaps around
/// `BASE_MUSIC_TIMELINE_TRACK`).
pub fn expanded_width(space: OecaSpace) -> f32 {
    match space {
        OecaSpace::Home => BASE_EXPANDED_WIDTH,
        OecaSpace::Clipboard => BASE_CLIPBOARD_WIDTH,
        OecaSpace::Music => {
            // Side walls, the player column's inset on both sides, and the
            // widest row: the full-width scrubber (labels + track)
            let edges = 2.0 * (BASE_EXPANDED_SHADOW_MARGIN_X + BASE_EXPANDED_TOP_TRANSITION_RADIUS);
            let insets = 2.0 * BASE_MUSIC_SIDE_INSET;
            let scrubber = 2.0 * (BASE_MEDIA_TIMELINE_LABEL_WIDTH + BASE_MEDIA_TIMELINE_LABEL_GAP)
                + BASE_MUSIC_TIMELINE_TRACK;
            edges + insets + scrubber
        }
    }
}

/// Settled notch dimensions for a state in a space: the collapsed notch is the
/// same for every space; the expanded notch takes the space's width and height.
pub fn space_dimensions(state: NotchState, dpi: u32, space: OecaSpace) -> NotchDimensions {
    let dims = NotchDimensions::from_state_and_dpi(state, dpi);
    match (state, space) {
        (NotchState::Expanded, OecaSpace::Music) => {
            let dims = dims.with_width_dip(expanded_width(space));
            NotchDimensions {
                height: music_height_px(&dims),
                ..dims
            }
        }
        (NotchState::Expanded, OecaSpace::Clipboard) => {
            let dims = dims.with_width_dip(expanded_width(space));
            NotchDimensions {
                height: clipboard_height_px(&dims),
                ..dims
            }
        }
        _ => dims,
    }
}

/// Settled expanded window size of a space at a DPI, in DIP (what transitions
/// target, so they end exactly on the settled pixel size).
pub fn expanded_size_dip(space: OecaSpace, dpi: u32) -> (f32, f32) {
    let d = space_dimensions(NotchState::Expanded, dpi, space);
    (d.width as f32 / d.scale, d.height as f32 / d.scale)
}

/// Music window height in pixels: the player rows as they actually round at
/// this DPI (see `resolve_music_layout`), so every row always fits.
/// `expanded_height(Music)` is the same sum at 96 DPI.
fn music_height_px(d: &NotchDimensions) -> i32 {
    let px = |v: f32| (v * d.scale).round();
    let rows = px(BASE_MUSIC_ARTWORK_SIZE)
        + px(BASE_MUSIC_SCRUBBER_GAP)
        + px(BASE_MUSIC_SCRUBBER_HEIGHT)
        + px(BASE_MUSIC_CONTROLS_GAP)
        + px(BASE_MUSIC_CONTROL_HEIGHT)
        + px(BASE_MUSIC_BOTTOM_PAD);
    let header = px(crate::config::BASE_EXPANDED_HEADER_EXTRA);
    let notch = d.curvature.top_transition_height + d.padding_v + header + rows;
    (notch + d.shadow_margin_bottom).ceil() as i32
}

/// Clipboard window height in pixels: the rows as they round at this DPI (see
/// `resolve_clipboard_layout`); `expanded_height(Clipboard)` at 96 DPI.
fn clipboard_height_px(d: &NotchDimensions) -> i32 {
    let px = |v: f32| (v * d.scale).round();
    let n = CLIPBOARD_VISIBLE_ROWS as f32;
    let rows = n * px(BASE_CLIPBOARD_ROW_HEIGHT)
        + (n - 1.0) * px(BASE_CLIPBOARD_ROW_GAP)
        + px(BASE_CLIPBOARD_BOTTOM_PAD);
    let header = px(crate::config::BASE_EXPANDED_HEADER_EXTRA);
    let notch = d.curvature.top_transition_height + d.padding_v + header + rows;
    (notch + d.shadow_margin_bottom).ceil() as i32
}

/// Media composition of the Home space (see `resolve_media_layout_in`).
pub fn resolve_media_layout(
    dimensions: &NotchDimensions,
    shape: MediaShape,
) -> Option<MediaLayout> {
    resolve_media_layout_in(dimensions, shape, OecaSpace::Home)
}

/// Media composition for a space. Home keeps the right time/date column; Music
/// is the focused player: no clock column, so the text column (title +
/// visualizer, artist, controls + scrubber) runs to the content edge of the
/// narrower notch. Clock bounds are empty in Music.
pub fn resolve_media_layout_in(
    dimensions: &NotchDimensions,
    shape: MediaShape,
    space: OecaSpace,
) -> Option<MediaLayout> {
    let ResolvedLayout::Expanded { components, .. } = resolve_layout(dimensions) else {
        return None;
    };
    let c = components.content_bounds;
    if space.is_music() {
        return Some(resolve_music_layout(dimensions, shape, c));
    }
    // Centred on the settled Home notch (same as `c` once settled), so frames
    // of the opening reveal the block in place
    let c = RectF::new(
        c.left,
        c.top,
        c.right,
        (home_settled_notch_h(dimensions.scale) - dimensions.padding_v).max(c.top),
    );
    let has_artwork = shape.artwork;
    let s = dimensions.scale;
    let px = |v: f32| (v * s).round();

    let title_h = px(BASE_MEDIA_TITLE_HEIGHT);
    let artist_h = px(BASE_MEDIA_ARTIST_HEIGHT);
    let source_h = px(BASE_MEDIA_SOURCE_HEIGHT);
    let text_h =
        px(BASE_MEDIA_TITLE_HEIGHT) + px(BASE_MEDIA_ARTIST_HEIGHT) + px(BASE_MEDIA_SOURCE_HEIGHT);
    // Shrinks (by rounding slack only) so the control row never leaves the content area
    let visual = px(BASE_MEDIA_CONTROL_VISUAL_SIZE).min((c.height() - text_h).max(0.0));
    let control_w = px(BASE_MEDIA_CONTROL_WIDTH);
    // The hit area gives up whatever the visual row lost, so it never reaches
    // further into the rounded bottom corners.
    let control_h =
        (px(BASE_MEDIA_CONTROL_HEIGHT) - (px(BASE_MEDIA_CONTROL_VISUAL_SIZE) - visual)).max(visual);

    // The text/controls gap absorbs per-DPI rounding so the visual stack never
    // exceeds the content height.
    let fixed_h = title_h + artist_h + source_h + visual;
    let full_gap = px(BASE_MEDIA_CONTROLS_SPACING).min((c.height() - fixed_h).max(0.0));
    let full_top = (c.top + (c.height() - fixed_h - full_gap) / 2.0).max(c.top);
    // Fewer secondary lines: the text + controls block stays together and is
    // re-centered vertically (no empty row above the controls). The clock column
    // keeps the full layout's rows so environmental info never moves.
    let missing_h = match shape.secondary_lines {
        0 => artist_h + source_h,
        1 => source_h,
        _ => 0.0,
    };
    // Breathing room above the controls comes from the rows actually shown.
    let shown_h = fixed_h - missing_h;
    let gap = px(BASE_MEDIA_CONTROLS_SPACING).min((c.height() - shown_h).max(0.0));
    let top = (c.top + ((c.height() - shown_h - gap) / 2.0).round()).max(c.top);

    let artwork_bounds = has_artwork.then(|| corner_artwork(dimensions, c));
    let clock_w = px(BASE_MEDIA_CLOCK_COLUMN_WIDTH).min(c.width() / 3.0);
    let clock_left = c.right - clock_w;
    let text_left = artwork_bounds.map_or(c.left, |a| a.right + px(BASE_MEDIA_ARTWORK_GAP));

    // The control group shares the text column's left edge: the Previous glyph's
    // left edge sits on the text edge, so metadata and controls read as one block
    // ...unless that would push the hover backdrop past the content edge (no artwork)
    // and the hit area stays clear of the rounded bottom corner (no-artwork case).
    let step = control_w + px(BASE_MEDIA_CONTROL_GAP);
    let first_cx = (text_left + (px(BASE_MEDIA_ICON_SIZE) * SKIP_GLYPH_HALF_WIDTH).round())
        .max(c.left + visual / 2.0)
        .max(c.left + control_w / 2.0);

    // Divider: a little after the controls (never into the clock column); the
    // track text truncates before it
    let divider_w = px(BASE_MEDIA_DIVIDER_WIDTH).max(1.0);
    let controls_right = first_cx + 2.0 * step + visual / 2.0;
    let divider_x = (controls_right + px(BASE_MEDIA_DIVIDER_AFTER_CONTROLS))
        .min(clock_left - px(BASE_MEDIA_COLUMN_GAP) / 2.0)
        .round();
    let text_right = (divider_x - px(BASE_MEDIA_DIVIDER_TEXT_GAP)).max(text_left);

    let row = |y: f32, h: f32| RectF::new(text_left, y, text_right, y + h);
    let title_bounds = row(top, title_h);
    let artist_bounds = row(title_bounds.bottom, artist_h);
    let source_bounds = row(artist_bounds.bottom, source_h);

    let controls_top = source_bounds.bottom - missing_h + gap;
    let control = |index: f32| {
        let left = first_cx + index * step - control_w / 2.0;
        RectF::new(
            left,
            controls_top,
            left + control_w,
            controls_top + control_h,
        )
    };

    Some(MediaLayout {
        artwork_bounds,
        title_bounds,
        artist_bounds,
        source_bounds,
        time_bounds: RectF::new(clock_left, full_top, c.right, full_top + title_h),
        date_bounds: RectF::new(
            clock_left,
            full_top + title_h,
            c.right,
            full_top + title_h + artist_h,
        ),
        previous_bounds: control(0.0),
        play_pause_bounds: control(1.0),
        next_bounds: control(2.0),
        control_visual_size: visual,
        // Home's expanded notch shows neither a visualizer nor a scrubber
        visualizer_bounds: RectF::new(text_right, controls_top, text_right, controls_top),
        divider_bounds: {
            let inset = px(BASE_MEDIA_DIVIDER_INSET);
            RectF::new(
                divider_x,
                c.top + inset,
                divider_x + divider_w,
                c.bottom - inset,
            )
        },
        timeline_bounds: RectF::new(text_right, controls_top, text_right, controls_top + visual),
    })
}

/// Settled Home notch body height in pixels (what animation frames lay Home
/// content out against, so it is revealed in place rather than sliding).
fn home_settled_notch_h(scale: f32) -> f32 {
    (BASE_EXPANDED_HEIGHT * scale).round() - (BASE_EXPANDED_SHADOW_MARGIN_BOTTOM * scale).round()
}

/// Home cover: nests in the settled notch's bottom-left corner, the same inset
/// from the side wall as from the bottom edge.
fn corner_artwork(dimensions: &NotchDimensions, c: RectF) -> RectF {
    let px = |v: f32| (v * dimensions.scale).round();
    let notch_h = home_settled_notch_h(dimensions.scale);
    let content_h = (notch_h - dimensions.padding_v - c.top).max(0.0);
    // Whole-pixel cover (and so badge) edges at every scale
    let side = px(BASE_MEDIA_ARTWORK_SIZE).min(content_h).max(0.0).floor();
    let art_top = (notch_h - px(BASE_MEDIA_ARTWORK_INSET) - side)
        .round()
        .max(c.top.ceil());
    let inset = (notch_h - (art_top + side)).max(0.0);
    let wall = dimensions.shadow_margin_x + dimensions.curvature.top_transition_radius;
    let art_left = (wall + inset).round().min(c.left);
    RectF::new(art_left, art_top, art_left + side, art_top + side)
}

/// Music player composition: a compact player, all rows in one column inset
/// equally from both side walls:
///
/// ```text
/// [cvr] Title                         |||   <- cover + title/artist, visualizer
/// [cvr] Artist                                 at the end of the title line
/// 0:49 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ -2:08   <- full-width scrubber
///             <<     ||     >>              <- larger controls, centred
/// ```
///
/// No badge on the cover, no clock column. Source text only stands in for a
/// missing artist.
fn resolve_music_layout(dimensions: &NotchDimensions, shape: MediaShape, c: RectF) -> MediaLayout {
    let s = dimensions.scale;
    let px = |v: f32| (v * s).round();
    let (title_h, artist_h) = (px(BASE_MEDIA_TITLE_HEIGHT), px(BASE_MEDIA_ARTIST_HEIGHT));
    let art = px(BASE_MUSIC_ARTWORK_SIZE);
    let visual = px(BASE_MUSIC_CONTROL_VISUAL_SIZE);
    let (control_w, control_h) = (px(BASE_MUSIC_CONTROL_WIDTH), px(BASE_MUSIC_CONTROL_HEIGHT));

    // One column for every row, inset from the side walls by the cover inset:
    // the selector pills, the cover and the elapsed time share its left edge;
    // the visualizer and the remaining time share its right edge.
    let wall = dimensions.shadow_margin_x + dimensions.curvature.top_transition_radius;
    let inset = px(BASE_MUSIC_SIDE_INSET);
    let c = RectF::new(
        (wall + inset).round(),
        c.top,
        (dimensions.width as f32 - wall - inset).round(),
        c.bottom,
    );

    // Row 1: cover + metadata (the metadata block centred on the cover)
    let top = c.top.round();
    // Whole-pixel cover edges at every scale (sharp artwork)
    let left = c.left;
    let artwork_bounds = shape
        .artwork
        .then(|| RectF::new(left, top, left + art, top + art));
    let text_left = artwork_bounds.map_or(c.left, |a| a.right + px(BASE_MUSIC_ARTWORK_GAP));
    let shown_h = if shape.secondary_lines > 0 {
        title_h + artist_h
    } else {
        title_h
    };
    let meta_top = (top + (art - shown_h) / 2.0).round();
    let visualizer_bounds = visualizer_rect(c.right, meta_top + title_h / 2.0, s);
    let title_bounds = RectF::new(
        text_left,
        meta_top,
        (visualizer_bounds.left - px(BASE_MEDIA_VISUALIZER_GAP)).max(text_left),
        meta_top + title_h,
    );
    let artist_bounds = RectF::new(
        text_left,
        title_bounds.bottom,
        c.right.max(text_left),
        title_bounds.bottom + artist_h,
    );
    // Source text only replaces a missing artist (drawn in the artist row)
    let source_bounds = RectF::new(
        text_left,
        artist_bounds.bottom,
        artist_bounds.right,
        artist_bounds.bottom,
    );

    // Row 2: full-width scrubber
    let scrub_top = top + art + px(BASE_MUSIC_SCRUBBER_GAP);
    let timeline_bounds = RectF::new(
        c.left,
        scrub_top,
        c.right,
        scrub_top + px(BASE_MUSIC_SCRUBBER_HEIGHT),
    );

    // Row 3: larger controls, centred on the notch
    let controls_top = timeline_bounds.bottom + px(BASE_MUSIC_CONTROLS_GAP);
    let cx = ((c.left + c.right) / 2.0).round();
    let step = control_w + px(BASE_MUSIC_CONTROL_GAP);
    let control = |index: f32| {
        let left = cx + index * step - control_w / 2.0;
        RectF::new(
            left,
            controls_top,
            left + control_w,
            controls_top + control_h,
        )
    };

    MediaLayout {
        artwork_bounds,
        title_bounds,
        artist_bounds,
        source_bounds,
        time_bounds: RectF::new(c.right, top, c.right, top + title_h),
        date_bounds: RectF::new(c.right, top + title_h, c.right, top + title_h + artist_h),
        previous_bounds: control(-1.0),
        play_pause_bounds: control(0.0),
        next_bounds: control(1.0),
        control_visual_size: visual,
        visualizer_bounds,
        timeline_bounds,
        divider_bounds: RectF::new(c.right, top, c.right, top),
    }
}

/// Expanded window height (DIP) of a space. Home is the existing expanded notch.
/// Music is derived from its player rows: shoulder band + top padding, cover
/// row, gap, scrubber, gap, controls, bottom padding, shadow margin.
pub fn expanded_height(space: OecaSpace) -> f32 {
    match space {
        OecaSpace::Home => BASE_EXPANDED_HEIGHT,
        OecaSpace::Clipboard => {
            let n = CLIPBOARD_VISIBLE_ROWS as f32;
            BASE_EXPANDED_TOP_TRANSITION_HEIGHT
                + crate::config::BASE_EXPANDED_HEADER_EXTRA
                + BASE_EXPANDED_PADDING_V
                + n * BASE_CLIPBOARD_ROW_HEIGHT
                + (n - 1.0) * BASE_CLIPBOARD_ROW_GAP
                + BASE_CLIPBOARD_BOTTOM_PAD
                + BASE_EXPANDED_SHADOW_MARGIN_BOTTOM
        }
        OecaSpace::Music => {
            BASE_EXPANDED_TOP_TRANSITION_HEIGHT
                + crate::config::BASE_EXPANDED_HEADER_EXTRA
                + BASE_EXPANDED_PADDING_V
                + BASE_MUSIC_ARTWORK_SIZE
                + BASE_MUSIC_SCRUBBER_GAP
                + BASE_MUSIC_SCRUBBER_HEIGHT
                + BASE_MUSIC_CONTROLS_GAP
                + BASE_MUSIC_CONTROL_HEIGHT
                + BASE_MUSIC_BOTTOM_PAD
                + BASE_EXPANDED_SHADOW_MARGIN_BOTTOM
        }
    }
}

/// Resolves collapsed layout components given NotchDimensions
pub fn resolve_collapsed_layout(dimensions: &NotchDimensions) -> CollapsedLayout {
    let w = dimensions.width as f32;
    let h = dimensions.height as f32;
    let scale = dimensions.scale;
    let r_top = dimensions.curvature.top_transition_radius;
    let pad_h = dimensions.padding_h;
    let pad_v = dimensions.padding_v;

    // Safe content bounds respecting shoulder transition and padding
    let content_left = r_top + pad_h;
    let content_right = (w - r_top - pad_h).max(content_left);
    let content_top = pad_v;
    let content_bottom = (h - pad_v).max(content_top);
    let content_bounds = RectF::new(content_left, content_top, content_right, content_bottom);

    let optical_y = crate::config::BASE_CLOCK_OPTICAL_Y_OFFSET * scale;
    let clock_bounds = RectF::new(
        content_left,
        content_top + optical_y,
        content_right,
        content_bottom + optical_y,
    );

    let cy = (content_top + content_bottom) / 2.0;
    let art = (crate::config::BASE_COLLAPSED_ARTWORK_SIZE * scale)
        .round()
        .min(content_bottom - content_top);
    let (art_left, art_top) = (content_left.round(), (cy - art / 2.0).round());
    CollapsedLayout {
        content_bounds,
        clock_bounds,
        visualizer_bounds: visualizer_rect(content_right, cy, scale),
        artwork_bounds: RectF::new(art_left, art_top, art_left + art, art_top + art),
    }
}

/// Resolves complete layout and component boundaries from NotchDimensions
pub fn resolve_layout(dimensions: &NotchDimensions) -> ResolvedLayout {
    let w = dimensions.width as f32;
    let h = dimensions.height as f32;
    let bounds = RectF::new(0.0, 0.0, w, h);

    match dimensions.state {
        NotchState::Collapsed => ResolvedLayout::Collapsed {
            bounds,
            components: resolve_collapsed_layout(dimensions),
        },
        NotchState::Expanded => {
            let scale = dimensions.scale;
            let pad_x = dimensions.shadow_margin_x;
            let notch_w = dimensions.notch_width();
            let notch_h = dimensions.notch_height();
            let r_top = dimensions.curvature.top_transition_radius;
            let pad_h = dimensions.padding_h;
            let pad_v = dimensions.padding_v;

            // Safe content bounds: avoids top shoulder curves, shadow margins, and bottom rounded corners
            let content_left = pad_x + r_top + pad_h;
            let content_right = (pad_x + notch_w - r_top - pad_h).max(content_left);
            // Content sits at its settled position on every frame (settled top
            // shoulder + padding + header band): the opening notch reveals it
            // in place instead of sliding it down and snapping at the end
            let header = (crate::config::BASE_EXPANDED_HEADER_EXTRA * scale).round();
            let content_top = BASE_EXPANDED_TOP_TRANSITION_HEIGHT * scale + pad_v + header;
            let content_bottom = (notch_h - pad_v).max(content_top);
            // The clock stack centres on the settled Home notch, not the
            // still-growing one
            let settled_bottom = (home_settled_notch_h(scale) - pad_v).max(content_top);
            let content_bounds =
                RectF::new(content_left, content_top, content_right, content_bottom);

            // Optical layout for stacked live time and date
            let time_height = (crate::config::BASE_EXPANDED_TIME_HEIGHT * scale).round();
            let date_height = (crate::config::BASE_EXPANDED_DATE_HEIGHT * scale).round();
            let spacing = (crate::config::BASE_EXPANDED_CLOCK_SPACING * scale).round();
            let optical_y = (crate::config::BASE_EXPANDED_CLOCK_OPTICAL_Y_OFFSET * scale).round();

            let total_stack_height = time_height + spacing + date_height;
            let available_height = settled_bottom - content_top;
            let stack_top =
                (content_top + (available_height - total_stack_height) / 2.0 + optical_y)
                    .max(content_top);

            let time_top = stack_top;
            let time_bottom = (time_top + time_height).min(content_bottom);
            let time_bounds = RectF::new(content_left, time_top, content_right, time_bottom);

            let date_top = (time_bottom + spacing).min(content_bottom);
            let date_bottom = (date_top + date_height).min(content_bottom);
            let date_bounds = RectF::new(content_left, date_top, content_right, date_bottom);

            ResolvedLayout::Expanded {
                bounds,
                components: ExpandedLayout {
                    content_bounds,
                    time_bounds,
                    date_bounds,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BASE_EXPANDED_HEIGHT, BASE_EXPANDED_WIDTH, is_point_in_notch_ex};

    #[test]
    fn test_expanded_layout_dimensions_and_containment() {
        for dpi in [96, 120, 137, 144, 168, 192, 288] {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            let layout = resolve_layout(&dims);

            match layout {
                ResolvedLayout::Expanded { bounds, components } => {
                    assert_eq!(bounds.width(), dims.width as f32);
                    assert_eq!(bounds.height(), dims.height as f32);

                    // Content bounds inside outer notch bounds
                    assert!(components.content_bounds.left >= 0.0);
                    assert!(components.content_bounds.top >= 0.0);
                    assert!(components.content_bounds.right <= bounds.right);
                    assert!(components.content_bounds.bottom <= bounds.bottom);

                    // Time bounds inside content bounds
                    assert!(components.time_bounds.left >= components.content_bounds.left);
                    assert!(components.time_bounds.right <= components.content_bounds.right);
                    assert!(components.time_bounds.top >= components.content_bounds.top);
                    assert!(components.time_bounds.bottom <= components.content_bounds.bottom);

                    // Date bounds inside content bounds, placed below time bounds
                    assert!(components.date_bounds.left >= components.content_bounds.left);
                    assert!(components.date_bounds.right <= components.content_bounds.right);
                    assert!(components.date_bounds.top >= components.time_bounds.bottom);
                    assert!(components.date_bounds.bottom <= components.content_bounds.bottom);

                    // Horizontal centering check: centers of time and date bounds must equal notch center
                    let center_x = bounds.width() / 2.0;
                    let time_center_x =
                        (components.time_bounds.left + components.time_bounds.right) / 2.0;
                    let date_center_x =
                        (components.date_bounds.left + components.date_bounds.right) / 2.0;
                    assert!(
                        (time_center_x - center_x).abs() <= 1.0,
                        "Time must be horizontally centered at {dpi} DPI"
                    );
                    assert!(
                        (date_center_x - center_x).abs() <= 1.0,
                        "Date must be horizontally centered at {dpi} DPI"
                    );

                    // All four corners of content_bounds must fall inside the AMOLED notch geometry
                    assert!(
                        dims.contains_point(
                            components.content_bounds.left,
                            components.content_bounds.top,
                        ),
                        "Top-left content bound must be inside notch geometry at {dpi} DPI"
                    );
                    assert!(
                        dims.contains_point(
                            components.content_bounds.right,
                            components.content_bounds.top,
                        ),
                        "Top-right content bound must be inside notch geometry at {dpi} DPI"
                    );
                    assert!(
                        dims.contains_point(
                            components.content_bounds.left,
                            components.content_bounds.bottom,
                        ),
                        "Bottom-left content bound must be inside notch geometry at {dpi} DPI"
                    );
                    assert!(
                        dims.contains_point(
                            components.content_bounds.right,
                            components.content_bounds.bottom,
                        ),
                        "Bottom-right content bound must be inside notch geometry at {dpi} DPI"
                    );
                }
                _ => panic!("Expected ResolvedLayout::Expanded"),
            }
        }
    }

    #[test]
    fn test_collapsed_and_expanded_centering_invariance() {
        // Collapsed center must equal expanded center for any monitor width and DPI
        for screen_w in [1024, 1280, 1366, 1440, 1536, 1920, 2560, 3840] {
            for dpi in [96, 120, 144, 168, 192] {
                let dims_c = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
                let dims_e = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);

                let center_c = crate::config::calculate_notch_center_x(screen_w, dims_c.width);
                let center_e = crate::config::calculate_notch_center_x(screen_w, dims_e.width);

                // Both centers must equal screen_w / 2 when notch fits on screen
                let ideal_center = screen_w as f32 / 2.0;
                assert!(
                    (center_c - ideal_center).abs() <= 0.5,
                    "Collapsed center mismatch at screen {screen_w}, dpi {dpi}"
                );
                if screen_w >= dims_e.width {
                    assert!(
                        (center_e - ideal_center).abs() <= 0.5,
                        "Expanded center mismatch at screen {screen_w}, dpi {dpi}"
                    );
                    assert!(
                        (center_c - center_e).abs() <= 1.0,
                        "Collapsed and expanded centers must match within rounding at screen {screen_w}, dpi {dpi}"
                    );
                } else {
                    // When display is narrower than expanded notch at high scale factor, clamped safely to 0
                    assert_eq!(crate::config::calculate_notch_x(screen_w, dims_e.width), 0);
                }
            }
        }
    }

    #[test]
    fn test_collapsed_layout_clock_containment_and_centering() {
        for dpi in [96, 120, 137, 144, 168, 192, 288] {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
            let layout = resolve_layout(&dims);

            match layout {
                ResolvedLayout::Collapsed { bounds, components } => {
                    assert_eq!(bounds.width(), dims.width as f32);
                    assert_eq!(bounds.height(), dims.height as f32);

                    // Content bounds inside outer notch bounds
                    assert!(components.content_bounds.left >= 0.0);
                    assert!(components.content_bounds.top >= 0.0);
                    assert!(components.content_bounds.right <= bounds.right);
                    assert!(components.content_bounds.bottom <= bounds.bottom);

                    // Clock bounds inside content bounds
                    assert!(components.clock_bounds.left >= components.content_bounds.left);
                    assert!(components.clock_bounds.right <= components.content_bounds.right);

                    // Mathematical horizontal centering: clock center must equal notch center w / 2.0 exactly
                    let clock_cx =
                        (components.clock_bounds.left + components.clock_bounds.right) / 2.0;
                    let notch_cx = dims.width as f32 / 2.0;
                    assert!(
                        (clock_cx - notch_cx).abs() <= 0.001,
                        "Clock horizontal center ({clock_cx}) must equal notch center ({notch_cx}) at {dpi} DPI"
                    );

                    // Vertical centering: clock vertical center matches notch vertical center h / 2.0
                    let clock_cy =
                        (components.clock_bounds.top + components.clock_bounds.bottom) / 2.0;
                    let notch_cy = dims.height as f32 / 2.0;
                    assert!(
                        (clock_cy - notch_cy).abs() <= 0.5,
                        "Clock vertical center ({clock_cy}) must match notch vertical center ({notch_cy}) at {dpi} DPI"
                    );

                    // All four corners of content_bounds must fall inside the AMOLED notch geometry
                    let r_top_x = dims.curvature.top_transition_radius;
                    let r_top_y = dims.curvature.top_transition_height;
                    let r_bottom = dims.curvature.bottom_radius;
                    let w = dims.width as f32;
                    let h = dims.height as f32;

                    assert!(
                        is_point_in_notch_ex(
                            components.content_bounds.left,
                            components.content_bounds.top,
                            w,
                            h,
                            r_top_x,
                            r_top_y,
                            r_bottom
                        ),
                        "Collapsed top-left content bound must be inside notch geometry at {dpi} DPI"
                    );
                    assert!(
                        is_point_in_notch_ex(
                            components.content_bounds.right,
                            components.content_bounds.top,
                            w,
                            h,
                            r_top_x,
                            r_top_y,
                            r_bottom
                        ),
                        "Collapsed top-right content bound must be inside notch geometry at {dpi} DPI"
                    );
                    assert!(
                        is_point_in_notch_ex(
                            components.content_bounds.left,
                            components.content_bounds.bottom,
                            w,
                            h,
                            r_top_x,
                            r_top_y,
                            r_bottom
                        ),
                        "Collapsed bottom-left content bound must be inside notch geometry at {dpi} DPI"
                    );
                    assert!(
                        is_point_in_notch_ex(
                            components.content_bounds.right,
                            components.content_bounds.bottom,
                            w,
                            h,
                            r_top_x,
                            r_top_y,
                            r_bottom
                        ),
                        "Collapsed bottom-right content bound must be inside notch geometry at {dpi} DPI"
                    );
                }
                _ => panic!("Expected ResolvedLayout::Collapsed"),
            }
        }
    }

    #[test]
    fn test_collapsed_state_does_not_expose_expanded_content() {
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, 96);
        let layout = resolve_layout(&dims);
        match layout {
            ResolvedLayout::Collapsed { components, .. } => {
                // Collapsed state only has content_bounds and clock_bounds
                assert!(components.content_bounds.width() > 0.0);
                assert!(components.clock_bounds.width() > 0.0);
            }
            _ => panic!("Expected ResolvedLayout::Collapsed"),
        }
    }

    #[test]
    fn test_collapsed_and_expanded_horizontal_center_alignment() {
        for dpi in [96, 120, 144, 192] {
            let dims_c = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
            let dims_e = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);

            let layout_c = resolve_layout(&dims_c);
            let layout_e = resolve_layout(&dims_e);

            if let (
                ResolvedLayout::Collapsed {
                    components: comp_c, ..
                },
                ResolvedLayout::Expanded {
                    bounds: bounds_e, ..
                },
            ) = (layout_c, layout_e)
            {
                // Collapsed clock center is exactly width / 2.0
                let clock_cx = (comp_c.clock_bounds.left + comp_c.clock_bounds.right) / 2.0;
                let notch_c_cx = dims_c.width as f32 / 2.0;
                assert!((clock_cx - notch_c_cx).abs() <= 0.001);

                // Both collapsed notch and expanded notch have their symmetry axes at half their width
                assert_eq!(bounds_e.width(), dims_e.width as f32);
            } else {
                panic!("State resolution mismatch");
            }
        }
    }

    #[test]
    fn test_reference_dimensions_at_96_dpi() {
        let dims_e = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 96);
        assert_eq!(dims_e.width, BASE_EXPANDED_WIDTH as i32);
        assert_eq!(dims_e.height, BASE_EXPANDED_HEIGHT as i32);
        assert_eq!(dims_e.width, 600);
        assert_eq!(dims_e.height, 138);
    }

    #[test]
    fn test_expanded_clock_and_date_bounds() {
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 96);
        let layout = resolve_layout(&dims);
        match layout {
            ResolvedLayout::Expanded { components, .. } => {
                assert!(components.time_bounds.width() > 0.0);
                assert_eq!(components.time_bounds.height(), 34.0);
                assert!(components.date_bounds.width() > 0.0);
                assert_eq!(components.date_bounds.height(), 16.0);
                assert!(components.date_bounds.top >= components.time_bounds.bottom);
            }
            _ => panic!("Expected ResolvedLayout::Expanded"),
        }
    }

    #[test]
    fn test_expanded_time_and_date_containment_across_all_dpis() {
        for dpi in [96, 120, 137, 144, 168, 192, 288] {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            let layout = resolve_layout(&dims);
            match layout {
                ResolvedLayout::Expanded { bounds, components } => {
                    // Content bounds inside outer notch bounds
                    assert!(components.content_bounds.left >= 0.0);
                    assert!(components.content_bounds.top >= 0.0);
                    assert!(components.content_bounds.right <= bounds.right);
                    assert!(components.content_bounds.bottom <= bounds.bottom);

                    // Time bounds strictly contained in content bounds
                    assert!(components.time_bounds.left >= components.content_bounds.left);
                    assert!(components.time_bounds.right <= components.content_bounds.right);
                    assert!(components.time_bounds.top >= components.content_bounds.top);
                    assert!(components.time_bounds.bottom <= components.content_bounds.bottom);

                    // Date bounds strictly contained in content bounds
                    assert!(components.date_bounds.left >= components.content_bounds.left);
                    assert!(components.date_bounds.right <= components.content_bounds.right);
                    assert!(components.date_bounds.top >= components.content_bounds.top);
                    assert!(components.date_bounds.bottom <= components.content_bounds.bottom);

                    // Corners of time and date within notch geometry
                    assert!(
                        dims.contains_point(
                            components.time_bounds.left,
                            components.time_bounds.top
                        )
                    );
                    assert!(
                        dims.contains_point(
                            components.time_bounds.right,
                            components.time_bounds.top
                        )
                    );
                    assert!(dims.contains_point(
                        components.date_bounds.left,
                        components.date_bounds.bottom
                    ));
                    assert!(dims.contains_point(
                        components.date_bounds.right,
                        components.date_bounds.bottom
                    ));
                }
                _ => panic!("Expected ResolvedLayout::Expanded"),
            }
        }
    }

    #[test]
    fn test_expanded_clock_date_centered_composition() {
        for dpi in [96, 120, 137, 144, 168, 192, 288] {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            let layout = resolve_layout(&dims);
            match layout {
                ResolvedLayout::Expanded { bounds, components } => {
                    let notch_cx = bounds.width() / 2.0;
                    let time_cx =
                        (components.time_bounds.left + components.time_bounds.right) / 2.0;
                    let date_cx =
                        (components.date_bounds.left + components.date_bounds.right) / 2.0;
                    let content_cx =
                        (components.content_bounds.left + components.content_bounds.right) / 2.0;

                    // Perfect horizontal optical alignment with notch center
                    assert!(
                        (time_cx - notch_cx).abs() <= 1.0,
                        "Time must center on notch at {dpi} DPI"
                    );
                    assert!(
                        (date_cx - notch_cx).abs() <= 1.0,
                        "Date must center on notch at {dpi} DPI"
                    );
                    assert!(
                        (content_cx - notch_cx).abs() <= 1.0,
                        "Content must center on notch at {dpi} DPI"
                    );

                    // Vertical optical stack balance: gap between content_top and time_top must be >= 0
                    assert!(components.time_bounds.top >= components.content_bounds.top);
                    assert!(components.content_bounds.bottom >= components.date_bounds.bottom);
                }
                _ => panic!("Expected ResolvedLayout::Expanded"),
            }
        }
    }

    const ALL_DPIS: [u32; 7] = [96, 120, 137, 144, 168, 192, 288];

    fn inside(inner: &RectF, outer: &RectF) -> bool {
        inner.left >= outer.left - 0.001
            && inner.right <= outer.right + 0.001
            && inner.top >= outer.top - 0.001
            && inner.bottom <= outer.bottom + 0.001
    }

    fn overlaps(a: &RectF, b: &RectF) -> bool {
        a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
    }

    fn corners(r: &RectF) -> [(f32, f32); 4] {
        [
            (r.left, r.top),
            (r.right, r.top),
            (r.left, r.bottom),
            (r.right, r.bottom),
        ]
    }

    fn media_and_content(dpi: u32, art: bool) -> (NotchDimensions, MediaLayout, RectF) {
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
        let media = resolve_media_layout(
            &dims,
            MediaShape {
                artwork: art,
                secondary_lines: 2,
            },
        )
        .expect("expanded media layout");
        let ResolvedLayout::Expanded { components, .. } = resolve_layout(&dims) else {
            panic!("Expected expanded layout");
        };
        (dims, media, components.content_bounds)
    }

    #[test]
    fn test_media_layout_absent_when_collapsed() {
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, 96);
        assert!(resolve_media_layout(&dims, MediaShape::FULL).is_none());
        assert!(
            resolve_media_layout(
                &dims,
                MediaShape {
                    artwork: false,
                    secondary_lines: 2
                }
            )
            .is_none()
        );
    }

    #[test]
    fn test_media_layout_reference_values_at_96_dpi() {
        let (_, m, c) = media_and_content(96, true);
        assert_eq!(c, RectF::new(42.0, 36.0, 558.0, 108.0));
        assert_eq!(m.artwork_bounds, Some(RectF::new(33.0, 39.0, 105.0, 111.0)));
        // Home: full-width title; no visualizer in the expanded Home notch
        assert_eq!(m.title_bounds, RectF::new(119.0, 36.0, 241.0, 56.0));
        assert_eq!(m.visualizer_bounds.width(), 0.0);
        // Home has no scrubber (empty strip at the text column end)
        assert_eq!(m.timeline_bounds.width(), 0.0);
        assert_eq!(m.artist_bounds, RectF::new(119.0, 56.0, 241.0, 72.0));
        assert_eq!(m.source_bounds, RectF::new(119.0, 72.0, 241.0, 86.0));
        assert_eq!(m.time_bounds, RectF::new(446.0, 36.0, 558.0, 56.0));
        assert_eq!(m.date_bounds, RectF::new(446.0, 56.0, 558.0, 72.0));
        // Controls start at the text edge: Previous glyph (2 x 7 px) left edge = 119
        assert_eq!(m.previous_bounds, RectF::new(110.0, 86.0, 142.0, 114.0));
        assert_eq!(m.play_pause_bounds, RectF::new(148.0, 86.0, 180.0, 114.0));
        assert_eq!(m.next_bounds, RectF::new(186.0, 86.0, 218.0, 114.0));
        assert_eq!(
            m.control_visual_bounds(MediaControl::PlayPause),
            RectF::new(153.0, 86.0, 175.0, 108.0)
        );
    }

    #[test]
    fn test_media_layout_without_artwork_uses_full_text_column() {
        let (_, m, c) = media_and_content(96, false);
        assert_eq!(m.artwork_bounds, None);
        assert_eq!(m.title_bounds.left, c.left);
        let (_, with_art, _) = media_and_content(96, true);
        // The text column ends at the divider, which follows the controls
        assert_eq!(m.title_bounds.right, m.divider_bounds.left - 12.0);
        assert_eq!(
            m.time_bounds, with_art.time_bounds,
            "clock column is stable"
        );
    }

    #[test]
    fn test_media_layout_containment_and_no_overlap_all_dpis() {
        for dpi in ALL_DPIS {
            for art in [true, false] {
                let (dims, m, c) = media_and_content(dpi, art);
                // Everything drawn stays inside the content bounds
                let mut visual = vec![
                    m.title_bounds,
                    m.artist_bounds,
                    m.source_bounds,
                    m.time_bounds,
                    m.date_bounds,
                ];
                visual.extend(m.artwork_bounds);
                visual.extend(MediaControl::ALL.map(|k| m.control_visual_bounds(k)));
                // Content area, grown left to the artwork at the notch edge
                let grown = RectF::new(
                    m.artwork_bounds.map_or(c.left, |a| a.left),
                    c.top,
                    c.right,
                    m.artwork_bounds
                        .map_or(c.bottom, |a| a.bottom.max(c.bottom)),
                );
                for (i, r) in visual.iter().enumerate() {
                    assert!(
                        inside(r, &grown),
                        "visual {i} escapes content at {dpi} DPI: {r:?}"
                    );
                    assert!(
                        r.width() > 0.0 && r.height() > 0.0,
                        "visual {i} empty at {dpi}"
                    );
                    for (j, other) in visual.iter().enumerate().skip(i + 1) {
                        assert!(!overlaps(r, other), "visuals {i}/{j} overlap at {dpi} DPI");
                    }
                }
                // Hit areas stay inside the notch contour and never cover text/artwork
                for k in MediaControl::ALL {
                    let hit = m.control_bounds(k);
                    for (x, y) in corners(&hit) {
                        assert!(
                            dims.contains_point(x, y),
                            "{k:?} hit outside notch at {dpi}"
                        );
                    }
                    for r in visual.iter().take(5).chain(m.artwork_bounds.iter()) {
                        assert!(!overlaps(&hit, r), "{k:?} hit covers content at {dpi}");
                    }
                }
                assert!(m.artist_bounds.top >= m.title_bounds.bottom);
                assert!(m.source_bounds.top >= m.artist_bounds.bottom);
                assert!(m.play_pause_bounds.top >= m.source_bounds.bottom);
            }
        }
    }

    #[test]
    fn test_media_artwork_square_and_inside_notch_all_dpis() {
        for dpi in ALL_DPIS {
            let (dims, m, c) = media_and_content(dpi, true);
            let a = m.artwork_bounds.unwrap();
            assert!(
                (a.width() - a.height()).abs() < 0.001,
                "artwork not square at {dpi}"
            );
            assert!(
                (a.height() - c.height()).abs() <= 1.0,
                "artwork spans content height"
            );
            // The drawn (rounded, concentric) cover nests inside the notch's corner
            let p = crate::config::CornerProfile::new(
                dims.curvature.bottom_radius - (dims.notch_height() - a.bottom),
                1.0,
                a.width() / 2.0,
            );
            for i in 0..=10 {
                let (u, v) = p.point(i as f32 / 10.0);
                let (x, y) = (a.left + u, a.bottom - p.span + v);
                assert!(
                    dims.contains_point(x, y),
                    "artwork corner outside notch at {dpi}"
                );
            }
            // Same inset from the side wall as from the bottom edge
            assert!(
                ((a.left - dims.shadow_margin_x - dims.curvature.top_transition_radius)
                    - (dims.notch_height() - a.bottom))
                    .abs()
                    <= 0.5,
                "artwork not nested in the corner at {dpi}"
            );
            for (x, y) in [(a.left, a.top), (a.right, a.top), (a.right, a.bottom)] {
                assert!(
                    dims.contains_point(x, y),
                    "artwork corner outside notch at {dpi}"
                );
            }
            assert!(
                m.title_bounds.left > a.right,
                "text starts right of artwork"
            );
        }
    }

    #[test]
    fn test_media_controls_aligned_to_text_and_evenly_spaced() {
        for dpi in ALL_DPIS {
            for art in [true, false] {
                let (dims, m, c) = media_and_content(dpi, art);
                let icon = (BASE_MEDIA_ICON_SIZE * dims.scale).round();
                let prev_cx = (m.previous_bounds.left + m.previous_bounds.right) / 2.0;
                let glyph_left = prev_cx - (icon * SKIP_GLYPH_HALF_WIDTH).round();
                if art {
                    // Previous glyph shares the text column's left edge
                    assert!(
                        (glyph_left - m.title_bounds.left).abs() <= 0.5,
                        "align at {dpi}"
                    );
                } else {
                    // At the content edge the backdrop stays inside the content
                    let v = m.control_visual_bounds(MediaControl::Previous);
                    assert!(v.left >= c.left - 0.001, "backdrop escapes at {dpi}");
                    assert!(
                        glyph_left >= m.title_bounds.left,
                        "glyph before text at {dpi}"
                    );
                }
                let gap_l = m.play_pause_bounds.left - m.previous_bounds.right;
                let gap_r = m.next_bounds.left - m.play_pause_bounds.right;
                assert!((gap_l - gap_r).abs() < 0.001 && gap_l > 0.0);
                assert_eq!(m.previous_bounds.width(), m.next_bounds.width());
                assert_eq!(m.previous_bounds.top, m.next_bounds.top);
                // The group is compact: it never reaches the clock column
                assert!(m.next_bounds.right < m.time_bounds.left);
            }
        }
    }

    #[test]
    fn test_media_layout_dpi_scaling() {
        let (_, base, _) = media_and_content(96, true);
        for dpi in ALL_DPIS {
            let (_, m, _) = media_and_content(dpi, true);
            let s = dpi as f32 / 96.0;
            let w = m.play_pause_bounds.width();
            let h = m.play_pause_bounds.height();
            assert!((w - (base.play_pause_bounds.width() * s).round()).abs() <= 0.001);
            // (the hit row gives up what the visual row loses to rounding: <= 1.5 px)
            assert!((h - (base.play_pause_bounds.height() * s).round()).abs() <= 1.5);
            // Hit target stays about 28x28 DIP at every scale
            assert!(
                w / s >= 28.0 - 0.5 && h / s >= 28.0 - 1.5,
                "hit target too small at {dpi}"
            );
            assert!((m.title_bounds.height() - (20.0 * s).round()).abs() <= 0.001);
            let a = m.artwork_bounds.unwrap();
            assert!(
                (a.width() - (72.0 * s).round()).abs() <= 1.0,
                "artwork scales at {dpi}"
            );
        }
    }

    #[test]
    fn test_media_hit_testing() {
        for dpi in ALL_DPIS {
            let (dims, m, _) = media_and_content(dpi, true);
            let center = |r: RectF| ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
            for control in MediaControl::ALL {
                let (x, y) = center(m.control_bounds(control));
                assert_eq!(m.control_at(x, y), Some(control), "{control:?} at {dpi}");
                // The visual backdrop center is also a hit
                let (vx, vy) = center(m.control_visual_bounds(control));
                assert_eq!(m.control_at(vx, vy), Some(control));
            }
            // Gaps, text, artwork and clock are not controls
            let gap_x = (m.previous_bounds.right + m.play_pause_bounds.left) / 2.0;
            let (_, ctrl_y) = center(m.play_pause_bounds);
            assert_eq!(m.control_at(gap_x, ctrl_y), None);
            for r in [
                m.title_bounds,
                m.source_bounds,
                m.time_bounds,
                m.artwork_bounds.unwrap(),
            ] {
                let (x, y) = center(r);
                assert_eq!(m.control_at(x, y), None);
            }
            // Outside the notch body stays transparent
            assert!(!dims.contains_point(1.0, 1.0));
            assert!(!dims.contains_point(dims.width as f32 / 2.0, dims.height as f32 - 1.0));
            assert_eq!(m.control_at(1.0, 1.0), None);
        }
    }

    #[test]
    fn test_opening_reveals_content_in_place_every_space() {
        use crate::config::AnimationState;
        // Vertical positions of each space's content on a frame
        let tops = |d: &NotchDimensions, space: OecaSpace| -> Vec<f32> {
            match space {
                OecaSpace::Clipboard => resolve_clipboard_layout(d)
                    .unwrap()
                    .tiles
                    .iter()
                    .map(|r| r.top)
                    .collect(),
                _ => {
                    let m = resolve_media_layout_in(d, MediaShape::FULL, space).unwrap();
                    let mut v = vec![m.title_bounds.top, m.artist_bounds.top];
                    v.extend(m.artwork_bounds.map(|a| a.top));
                    v
                }
            }
        };
        for dpi in ALL_DPIS {
            for space in OecaSpace::ALL {
                let settled = tops(&space_dimensions(NotchState::Expanded, dpi, space), space);
                let mut anim = AnimationState::start(
                    NotchState::Collapsed,
                    (0.0, 0.0),
                    NotchState::Expanded,
                    expanded_size_dip(space, dpi),
                    crate::space::Scene::Space(space),
                );
                for step in 0..=20 {
                    anim.set_progress(step as f32 / 20.0);
                    let d = anim.current_dimensions(dpi);
                    if d.state == NotchState::Expanded {
                        assert_eq!(
                            tops(&d, space),
                            settled,
                            "{space:?} moved at {dpi}, step {step}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_media_layout_contained_during_animation() {
        use crate::config::AnimationState;
        for dpi in [96, 144, 192] {
            for target in [NotchState::Expanded, NotchState::Collapsed] {
                let start = match target {
                    NotchState::Expanded => NotchState::Collapsed,
                    NotchState::Collapsed => NotchState::Expanded,
                };
                let mut anim = AnimationState::new(start, target);
                for step in 0..=20 {
                    anim.set_progress(step as f32 / 20.0);
                    let dims = anim.current_dimensions(dpi);
                    if let Some(m) = resolve_media_layout(&dims, MediaShape::FULL) {
                        // The stack starts inside the content area; overflow is clipped
                        let ResolvedLayout::Expanded { components, .. } = resolve_layout(&dims)
                        else {
                            unreachable!()
                        };
                        let c = components.content_bounds;
                        assert!(m.title_bounds.top >= c.top);
                        let a = m.artwork_bounds.unwrap();
                        let wall = dims.shadow_margin_x + dims.curvature.top_transition_radius;
                        assert!(a.left >= wall && a.top >= c.top - 0.001);
                        assert!(m.time_bounds.right <= c.right + 0.001);
                    }
                }
            }
        }
    }

    #[test]
    fn test_media_block_rebalances_for_missing_lines_all_dpis() {
        for dpi in ALL_DPIS {
            for artwork in [true, false] {
                let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
                let full = resolve_media_layout(
                    &dims,
                    MediaShape {
                        artwork,
                        secondary_lines: 2,
                    },
                )
                .unwrap();
                let ResolvedLayout::Expanded { components, .. } = resolve_layout(&dims) else {
                    unreachable!()
                };
                let c = components.content_bounds;
                for lines in [0u32, 1] {
                    let m = resolve_media_layout(
                        &dims,
                        MediaShape {
                            artwork,
                            secondary_lines: lines,
                        },
                    )
                    .unwrap();
                    // Controls follow the last shown line (no empty row), with at
                    // least the full layout's breathing room
                    let last_line_bottom = match lines {
                        0 => m.title_bounds.bottom,
                        _ => m.artist_bounds.bottom,
                    };
                    let gap = m.play_pause_bounds.top - last_line_bottom;
                    let full_gap = full.play_pause_bounds.top - full.source_bounds.bottom;
                    assert!(
                        gap >= full_gap - 0.001
                            && gap <= (BASE_MEDIA_CONTROLS_SPACING * dims.scale).round() + 0.001,
                        "empty row left at {dpi}, {lines} lines"
                    );
                    // The shorter block is re-centered: equal slack above and below (+/-1 px)
                    let block_bottom = m.control_visual_bounds(MediaControl::PlayPause).bottom;
                    let above = m.title_bounds.top - c.top;
                    let below = c.bottom - block_bottom;
                    assert!(
                        (above - below).abs() <= 1.5,
                        "not centered at {dpi}: {above} vs {below}"
                    );
                    // Clock column and artwork never move with the metadata
                    assert_eq!(m.time_bounds, full.time_bounds);
                    assert_eq!(m.date_bounds, full.date_bounds);
                    assert_eq!(m.artwork_bounds, full.artwork_bounds);
                    // Everything drawn stays inside the content bounds
                    for k in MediaControl::ALL {
                        assert!(inside(&m.control_visual_bounds(k), &c));
                    }
                }
            }
        }
    }

    #[test]
    fn test_visualizer_and_timeline_slots_all_dpis() {
        for dpi in ALL_DPIS {
            // Collapsed: inside the existing notch body, right end of the content
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
            let l = resolve_collapsed_layout(&dims);
            let v = l.visualizer_bounds;
            assert!(
                inside(&v, &l.content_bounds),
                "collapsed viz in content at {dpi}"
            );
            assert_eq!(v.right, l.content_bounds.right);
            for (x, y) in corners(&v) {
                assert!(
                    dims.contains_point(x, y),
                    "collapsed viz inside notch at {dpi}"
                );
            }
            assert!(v.width() > 0.0 && v.height() > 0.0);

            for art in [true, false] {
                // Home: full-width title, no visualizer, no scrubber
                let (_, m, _) = media_and_content(dpi, art);
                let v = m.visualizer_bounds;
                assert_eq!(
                    m.title_bounds.right, m.artist_bounds.right,
                    "full title row"
                );
                assert_eq!(v.width(), 0.0, "no Home visualizer at {dpi}");
                assert_eq!(m.timeline_bounds.width(), 0.0, "no Home scrubber at {dpi}");

                // Music: covered by the Music player tests below
            }
        }
    }

    #[test]
    fn test_settings_icon_at_the_right_end_of_every_header() {
        for dpi in ALL_DPIS {
            for space in OecaSpace::ALL {
                let dims = space_dimensions(NotchState::Expanded, dpi, space);
                let sel = resolve_space_selector_in(&dims, space).unwrap();
                let gear = sel.settings;
                // Same row and height as the pills, right of all of them
                assert_eq!((gear.top, gear.bottom), (sel.home.top, sel.home.bottom));
                assert!(gear.left > sel.clipboard.right, "{space:?} at {dpi}");
                for (x, y) in corners(&gear) {
                    assert!(dims.contains_point(x, y), "{space:?} gear inside at {dpi}");
                }
                // Not a space pill
                let (cx, cy) = (
                    (gear.left + gear.right) / 2.0,
                    (gear.top + gear.bottom) / 2.0,
                );
                assert_eq!(sel.space_at(cx, cy), None);
                // On the Settings page (panel size) the highlight sits on the icon
                let panel = space_dimensions(NotchState::Expanded, dpi, crate::space::PANEL_SPACE);
                let panel_gear = resolve_space_selector_in(&panel, crate::space::PANEL_SPACE)
                    .unwrap()
                    .settings;
                let (_, hi) =
                    blended_selector(&panel, Scene::Settings, Scene::Settings, space, 1.0).unwrap();
                assert_eq!(hi, panel_gear);
            }
            // Clipboard's clear X moves just left of the icon
            let dims = space_dimensions(NotchState::Expanded, dpi, OecaSpace::Clipboard);
            let gear = resolve_space_selector_in(&dims, OecaSpace::Clipboard)
                .unwrap()
                .settings;
            let clear = resolve_clipboard_layout(&dims).unwrap().clear;
            assert!(clear.right < gear.left && !overlaps(&clear, &gear));
        }
    }

    #[test]
    fn test_settings_page_fits_the_panel_size() {
        for dpi in ALL_DPIS {
            // Settings is always laid out at the panel (Clipboard) size
            let dims = space_dimensions(NotchState::Expanded, dpi, crate::space::PANEL_SPACE);
            let l = resolve_settings_layout(&dims).unwrap();
            let sel = resolve_space_selector_in(&dims, crate::space::PANEL_SPACE).unwrap();
            assert!(l.label.top >= sel.home.bottom, "below the header at {dpi}");
            let (a, m) = (l.always_on_top, l.reduced_motion);
            assert!(l.label.bottom <= a.title.top && a.description.bottom < m.title.top);
            assert!(
                m.description.bottom < l.full_settings.top,
                "rows in order at {dpi}"
            );
            for row in [a, m] {
                assert!(row.title.bottom <= row.description.top);
                assert!(
                    row.title.right < row.toggle.left && row.description.right < row.toggle.left
                );
            }
            assert!(!overlaps(&a.toggle, &m.toggle));
            let body_bottom = dims.height as f32 - dims.shadow_margin_bottom;
            assert!(
                l.full_settings.bottom <= body_bottom - 4.0,
                "room left at {dpi}"
            );
            for r in [
                l.label,
                a.title,
                a.description,
                a.toggle,
                m.title,
                m.description,
                m.toggle,
                l.full_settings,
            ] {
                for (x, y) in corners(&r) {
                    assert!(dims.contains_point(x, y), "{r:?} inside at {dpi}");
                }
            }
        }
        assert_eq!(
            resolve_settings_layout(&NotchDimensions::from_state_and_dpi(
                NotchState::Collapsed,
                96
            )),
            None
        );
    }

    #[test]
    fn test_blended_selector_glides_between_spaces() {
        for dpi in ALL_DPIS {
            for space in OecaSpace::ALL {
                let dims = space_dimensions(NotchState::Expanded, dpi, space);
                let plain = resolve_space_selector_in(&dims, space).unwrap();
                // Settled: exactly the plain selector, highlight on the space
                let sc = Scene::Space(space);
                let (sel, hi) = blended_selector(&dims, sc, sc, space, 1.0).unwrap();
                assert_eq!((sel, hi), (plain, plain.bounds(space)));
            }
            // Mid Home -> Music: the highlight is between the two pills
            let dims =
                space_dimensions(NotchState::Expanded, dpi, OecaSpace::Home).with_width_dip(490.0);
            let (h, m) = (
                Scene::Space(OecaSpace::Home),
                Scene::Space(OecaSpace::Music),
            );
            let (a, _) = blended_selector(&dims, h, h, OecaSpace::Home, 1.0).unwrap();
            let (b, _) = blended_selector(&dims, m, m, OecaSpace::Music, 1.0).unwrap();
            let (_, hi) = blended_selector(&dims, h, m, OecaSpace::Music, 0.5).unwrap();
            assert!(
                hi.left > a.home.left.min(b.music.left) && hi.left < a.home.left.max(b.music.left)
            );
            assert!(
                blended_selector(
                    &NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi),
                    h,
                    m,
                    OecaSpace::Music,
                    0.5
                )
                .is_none()
            );
        }
    }

    #[test]
    fn test_media_morph_moves_shared_parts_and_keeps_one_sided_slots() {
        let dims = |s| space_dimensions(NotchState::Expanded, 120, s);
        let home =
            resolve_media_layout_in(&dims(OecaSpace::Home), MediaShape::FULL, OecaSpace::Home)
                .unwrap();
        let music =
            resolve_media_layout_in(&dims(OecaSpace::Music), MediaShape::FULL, OecaSpace::Music)
                .unwrap();
        assert_eq!(
            MediaLayout::morph(&home, &music, 0.0).title_bounds,
            home.title_bounds
        );
        assert_eq!(
            MediaLayout::morph(&home, &music, 1.0).title_bounds,
            music.title_bounds
        );
        let mid = MediaLayout::morph(&home, &music, 0.5);
        let between = |x: f32, a: f32, b: f32| x >= a.min(b) && x <= a.max(b);
        assert!(between(
            mid.title_bounds.top,
            home.title_bounds.top,
            music.title_bounds.top
        ));
        assert!(between(
            mid.play_pause_bounds.left,
            home.play_pause_bounds.left,
            music.play_pause_bounds.left
        ));
        assert_eq!(
            mid.source_bounds, home.source_bounds,
            "Home-only row stays put"
        );
        // Home's clock column stays put (it fades); Music's scrubber likewise
        assert_eq!(mid.time_bounds, home.time_bounds);
        assert_eq!(mid.timeline_bounds, music.timeline_bounds);
        // Offsetting moves everything horizontally only
        let moved = home.offset_x(7.0);
        assert_eq!(moved.title_bounds.left, home.title_bounds.left + 7.0);
        assert_eq!(moved.title_bounds.top, home.title_bounds.top);
    }

    #[test]
    fn test_selector_shows_three_spaces_in_every_space() {
        for dpi in ALL_DPIS {
            for active in OecaSpace::ALL {
                let dims = space_dimensions(NotchState::Expanded, dpi, active);
                let sel = resolve_space_selector_in(&dims, active).unwrap();
                let pills = OecaSpace::ALL.map(|s| sel.bounds(s));
                assert_eq!(pills.len(), 3);
                for (i, s) in OecaSpace::ALL.into_iter().enumerate() {
                    let r = pills[i];
                    for (x, y) in corners(&r) {
                        assert!(
                            dims.contains_point(x, y),
                            "{s:?} inside {active:?} at {dpi}"
                        );
                    }
                    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
                    assert_eq!(sel.space_at(cx, cy), Some(s), "{s:?} pill in {active:?}");
                    if i > 0 {
                        assert!(pills[i - 1].right < r.left, "ordered, apart at {dpi}");
                    }
                }
            }
        }
    }

    #[test]
    fn test_clipboard_space_size_and_tiles_fit_all_dpis() {
        assert_eq!(expanded_width(OecaSpace::Home), 600.0, "Home unchanged");
        assert_eq!(expanded_height(OecaSpace::Home), BASE_EXPANDED_HEIGHT);
        for dpi in ALL_DPIS {
            let dims = space_dimensions(NotchState::Expanded, dpi, OecaSpace::Clipboard);
            assert_eq!(
                dims.width,
                NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi)
                    .with_width_dip(BASE_CLIPBOARD_WIDTH)
                    .width
            );
            let l = resolve_clipboard_layout(&dims).unwrap();
            let sel = resolve_space_selector_in(&dims, OecaSpace::Clipboard).unwrap();
            let body_bottom = dims.height as f32 - dims.shadow_margin_bottom;
            assert_eq!(l.tiles.len(), 6, "six tiles");
            for (i, r) in l.tiles.iter().enumerate() {
                assert!(r.height() > 0.0 && r.width() > 0.0);
                assert!(r.top >= sel.clipboard.bottom, "below the selector at {dpi}");
                assert!(
                    r.bottom <= body_bottom,
                    "tile {i} inside the notch at {dpi}"
                );
                assert!(inside(r, &l.list), "tile {i} in the area at {dpi}");
                for (x, y) in corners(r) {
                    assert!(dims.contains_point(x, y), "tile {i} corner at {dpi}");
                }
                // Whole pixels, all the same size
                assert_eq!(r.left, r.left.round());
                assert_eq!(
                    (r.width(), r.height()),
                    (l.tiles[0].width(), l.tiles[0].height())
                );
                for (j, o) in l.tiles.iter().enumerate() {
                    assert!(i == j || !overlaps(r, o), "tiles {i}/{j} overlap at {dpi}");
                }
            }
            // Three columns, two rows, newest first left to right
            assert_eq!(l.tiles[0].top, l.tiles[2].top);
            assert!(l.tiles[0].right < l.tiles[1].left && l.tiles[1].right < l.tiles[2].left);
            assert!(l.tiles[0].bottom < l.tiles[3].top);
            assert_eq!(l.tiles[3].left, l.tiles[0].left);
            // Tiles are wider than tall, but no pills
            assert!(l.tiles[0].width() > l.tiles[0].height());
            // Each tile's parts fit it without overlapping
            for i in 0..6 {
                let p = l.tile_parts(i);
                for r in [p.text, p.image, p.caption, p.copy, p.trash] {
                    assert!(inside(&r, &p.tile), "part {r:?} of tile {i} at {dpi}");
                }
                assert!(p.trash.right <= p.copy.left, "{dpi}");
                assert!(p.caption.right <= p.trash.left, "{dpi}");
                assert!(p.image.bottom <= p.copy.top && p.text.bottom <= p.copy.top);
                assert!(
                    (p.tile.bottom - p.copy.bottom) < p.tile.height() / 4.0,
                    "copy near the bottom edge at {dpi}"
                );
                assert!(p.image.width() > 0.0 && p.image.height() > 0.0, "{dpi}");
            }
            // Header: the X shares the selector band, at its right end
            assert!(l.clear.left > sel.clipboard.right);
            assert_eq!(
                (l.clear.top, l.clear.bottom),
                (sel.clipboard.top, sel.clipboard.bottom)
            );
            for (x, y) in corners(&l.clear) {
                assert!(dims.contains_point(x, y), "Clear inside at {dpi}");
            }
            let at96 = space_dimensions(NotchState::Expanded, 96, OecaSpace::Clipboard);
            assert_eq!(at96.height as f32, expanded_height(OecaSpace::Clipboard));
        }
    }

    #[test]
    fn test_clipboard_tile_hit_testing_with_zero_to_six_entries() {
        let mid = |r: RectF| ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        for dpi in ALL_DPIS {
            let dims = space_dimensions(NotchState::Expanded, dpi, OecaSpace::Clipboard);
            let l = resolve_clipboard_layout(&dims).unwrap();
            for len in 0..=6 {
                for i in 0..6 {
                    let p = l.tile_parts(i);
                    let has = i < len;
                    let (x, y) = mid(p.text);
                    assert_eq!(l.hit(x, y, len), has.then_some(PanelHit::Row(i)), "{dpi}");
                    let (x, y) = mid(p.copy);
                    assert_eq!(l.hit(x, y, len), has.then_some(PanelHit::Copy(i)));
                    let (x, y) = mid(p.trash);
                    assert_eq!(l.hit(x, y, len), has.then_some(PanelHit::Remove(i)));
                }
                let (x, y) = mid(l.clear);
                assert_eq!(l.hit(x, y, len), (len > 0).then_some(PanelHit::Clear));
            }
            // Gaps between tiles and the margins are background
            let t = l.tiles;
            let gap_x = (t[0].right + t[1].left) / 2.0;
            assert_eq!(l.hit(gap_x, mid(t[0]).1, 6), None, "{dpi}");
            let gap_y = (t[0].bottom + t[3].top) / 2.0;
            assert_eq!(l.hit(mid(t[0]).0, gap_y, 6), None, "{dpi}");
            assert_eq!(l.hit(t[0].left - 2.0, mid(t[0]).1, 6), None, "{dpi}");
        }
        assert_eq!(
            resolve_clipboard_layout(&NotchDimensions::from_state_and_dpi(
                NotchState::Collapsed,
                96
            )),
            None
        );
    }

    #[test]
    fn test_space_selector_bounds_all_dpis() {
        for dpi in ALL_DPIS {
            let collapsed = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
            assert_eq!(
                resolve_space_selector(&collapsed),
                None,
                "hidden when collapsed"
            );

            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            let sel = resolve_space_selector(&dims).unwrap();
            let ResolvedLayout::Expanded { components, .. } = resolve_layout(&dims) else {
                unreachable!()
            };
            let c = components.content_bounds;
            for space in OecaSpace::ALL {
                let r = sel.bounds(space);
                assert!(r.width() > 0.0 && r.height() > 0.0);
                for (x, y) in corners(&r) {
                    assert!(dims.contains_point(x, y), "{space:?} inside notch at {dpi}");
                }
                assert!(r.bottom <= c.top, "{space:?} above the content at {dpi}");
                let center = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
                assert_eq!(sel.space_at(center.0, center.1), Some(space));
            }
            assert!(
                sel.home.right < sel.music.left,
                "Home first, then Music at {dpi}"
            );
            assert!(
                sel.music.right < sel.clipboard.left,
                "then Clipboard at {dpi}"
            );
            assert!(!overlaps(&sel.home, &sel.music));
            assert!(!overlaps(&sel.music, &sel.clipboard));
            // Only the capsules are interactive
            let gap_x = (sel.home.right + sel.music.left) / 2.0;
            assert_eq!(sel.space_at(gap_x, sel.home.top + 2.0), None);
            assert_eq!(
                sel.space_at(sel.clipboard.right + 5.0, sel.home.top + 2.0),
                None
            );
            assert_eq!(
                sel.space_at(sel.home.left + 2.0, sel.home.bottom + 3.0),
                None
            );
            // Never overlaps media controls, artwork or text (any media shape)
            for art in [true, false] {
                let (_, m, _) = media_and_content(dpi, art);
                let mut others = vec![
                    m.title_bounds,
                    m.artist_bounds,
                    m.time_bounds,
                    m.visualizer_bounds,
                    m.timeline_bounds,
                ];
                others.extend(m.artwork_bounds);
                others.extend(MediaControl::ALL.map(|k| m.control_bounds(k)));
                for space in OecaSpace::ALL {
                    for o in &others {
                        assert!(
                            !overlaps(&sel.bounds(space), o),
                            "{space:?} overlap at {dpi}"
                        );
                    }
                }
            }
        }
    }

    fn music(dpi: u32, shape: MediaShape) -> (NotchDimensions, MediaLayout, RectF) {
        let dims = space_dimensions(NotchState::Expanded, dpi, OecaSpace::Music);
        let m = resolve_media_layout_in(&dims, shape, OecaSpace::Music).unwrap();
        let ResolvedLayout::Expanded { components, .. } = resolve_layout(&dims) else {
            unreachable!()
        };
        (dims, m, components.content_bounds)
    }

    #[test]
    fn test_music_hit_testing_follows_the_narrow_width() {
        for dpi in ALL_DPIS {
            let home = space_dimensions(NotchState::Expanded, dpi, OecaSpace::Home);
            let (dims, m, _) = music(dpi, MediaShape::FULL);
            let mid_y = dims.notch_height() / 2.0;
            assert!(dims.contains_point(dims.width as f32 / 2.0, mid_y));
            // Where Home's right column was, Music's window has nothing
            for x in [
                dims.width as f32 + 1.0,
                home.width as f32 - 40.0 * dims.scale,
            ] {
                assert!(
                    !dims.contains_point(x, mid_y),
                    "x={x} not interactive at {dpi}"
                );
            }
            // ...and only inside the Music notch's own walls
            assert!(!dims.contains_point(dims.shadow_margin_x - 1.0, mid_y));
            // Controls hit-test at their Music positions
            for k in MediaControl::ALL {
                let r = m.control_bounds(k);
                assert_eq!(
                    m.control_at((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0),
                    Some(k)
                );
            }
            // Selector stays inside the Music notch and clear of its layout
            let sel = resolve_space_selector(&dims).unwrap();
            for s in OecaSpace::ALL {
                let r = sel.bounds(s);
                for (x, y) in corners(&r) {
                    assert!(dims.contains_point(x, y), "{s:?} selector inside at {dpi}");
                }
                let mut others = vec![m.title_bounds, m.visualizer_bounds, m.timeline_bounds];
                others.extend(m.artwork_bounds);
                others.extend(MediaControl::ALL.map(|k| m.control_bounds(k)));
                for o in &others {
                    assert!(!overlaps(&r, o), "{s:?} selector overlap at {dpi}");
                }
            }
        }
    }

    #[test]
    fn test_space_sizes_home_unchanged_music_taller() {
        assert_eq!(expanded_width(OecaSpace::Home), BASE_EXPANDED_WIDTH);
        assert_eq!(expanded_height(OecaSpace::Home), BASE_EXPANDED_HEIGHT);
        assert_eq!(
            expanded_width(OecaSpace::Music),
            384.0,
            "derived Music width: 48 walls + 32 insets + 64 labels + 240 track"
        );
        // 14 shoulder + 12 pad + 40 cover row + 6 + 14 scrubber + 4 + 34 controls
        // + 6 pad + 18 shadow margin + 10 header band
        assert_eq!(
            expanded_height(OecaSpace::Music),
            158.0,
            "derived Music height"
        );
        for dpi in ALL_DPIS {
            let home = space_dimensions(NotchState::Expanded, dpi, OecaSpace::Home);
            let music = space_dimensions(NotchState::Expanded, dpi, OecaSpace::Music);
            assert_eq!(
                home,
                NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi)
            );
            assert!(
                music.width < home.width && music.height > home.height,
                "{dpi}"
            );
            // Derived from the rows as they round at this DPI (~158 DIP)
            let h = music.height as f32 / music.scale;
            assert!(
                (157.0..=161.0).contains(&h),
                "Music height {h} DIP at {dpi}"
            );
            assert_eq!(music.shadow_margin_bottom, home.shadow_margin_bottom);
            assert_eq!(
                NotchDimensions {
                    width: home.width,
                    height: home.height,
                    ..music
                },
                home,
                "only width/height differ at {dpi}"
            );
            for space in OecaSpace::ALL {
                assert_eq!(
                    space_dimensions(NotchState::Collapsed, dpi, space),
                    NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi),
                    "collapsed notch identical in every space"
                );
            }
        }
    }

    #[test]
    fn test_music_hit_testing_and_selector_on_taller_notch() {
        for dpi in ALL_DPIS {
            let (dims, m, _) = music(dpi, MediaShape::FULL);
            let home = space_dimensions(NotchState::Expanded, dpi, OecaSpace::Home);
            // The extra Music height is part of the notch (scrubber row)...
            let y = home.notch_height() + 2.0;
            assert!(
                dims.contains_point(dims.width as f32 / 2.0, y),
                "taller body at {dpi}"
            );
            // ...and the notch ends where the Music notch ends
            assert!(!dims.contains_point(dims.width as f32 / 2.0, dims.notch_height() + 1.0));
            for k in MediaControl::ALL {
                let r = m.control_bounds(k);
                assert_eq!(
                    m.control_at((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0),
                    Some(k)
                );
            }
            assert_eq!(
                m.control_at(
                    (m.timeline_bounds.left + m.timeline_bounds.right) / 2.0,
                    m.timeline_bounds.bottom - 1.0
                ),
                None,
                "scrubber is not a control"
            );
            let sel = resolve_space_selector(&dims).unwrap();
            let home_sel = resolve_space_selector(&home).unwrap();
            assert_eq!(
                sel.home.top, home_sel.home.top,
                "selector keeps its band at {dpi}"
            );
            for s in OecaSpace::ALL {
                let r = sel.bounds(s);
                assert!(
                    r.bottom <= m.title_bounds.top,
                    "selector above the player at {dpi}"
                );
                let mut others = vec![m.title_bounds, m.visualizer_bounds, m.timeline_bounds];
                others.extend(m.artwork_bounds);
                others.extend(MediaControl::ALL.map(|k| m.control_bounds(k)));
                for o in &others {
                    assert!(!overlaps(&r, o), "{s:?} selector overlap at {dpi}");
                }
            }
        }
    }

    #[test]
    fn test_music_player_rows_all_dpis() {
        for dpi in ALL_DPIS {
            for artwork in [true, false] {
                for secondary_lines in 0..=2 {
                    let shape = MediaShape {
                        artwork,
                        secondary_lines,
                    };
                    let (dims, m, c) = music(dpi, shape);
                    // The player column: inset from both side walls; its left edge
                    // is the selector pills' left edge
                    let wall = dims.shadow_margin_x + dims.curvature.top_transition_radius;
                    let inset = (BASE_MUSIC_SIDE_INSET * dims.scale).round();
                    let col = RectF::new(
                        (wall + inset).round(),
                        c.top,
                        (dims.width as f32 - wall - inset).round(),
                        c.bottom,
                    );
                    let sel = resolve_space_selector_in(&dims, OecaSpace::Music).unwrap();
                    let at = format!("{dpi} DPI art={artwork} lines={secondary_lines}");
                    assert_eq!(col.left, sel.home.left, "aligned with the pills {at}");
                    let px = |v: f32| (v * dims.scale).round();
                    let t = m.timeline_bounds;
                    let play = m.control_visual_bounds(MediaControl::PlayPause);
                    // Row 1: small cover at the content's left edge, metadata beside
                    // it (left-aligned), centred on the cover
                    let meta_bottom = if secondary_lines == 0 {
                        m.title_bounds.bottom
                    } else {
                        m.artist_bounds.bottom
                    };
                    if let Some(a) = m.artwork_bounds {
                        assert_eq!(a.width(), px(BASE_MUSIC_ARTWORK_SIZE), "{at}");
                        assert_eq!(a.left, a.left.round(), "whole-pixel cover {at}");
                        assert_eq!(a.left, col.left, "cover on the column edge {at}");
                        assert!(a.right < m.title_bounds.left, "{at}");
                        let meta_mid = (m.title_bounds.top + meta_bottom) / 2.0;
                        assert!(((a.top + a.bottom) / 2.0 - meta_mid).abs() <= 1.0, "{at}");
                    } else {
                        assert_eq!(m.title_bounds.left, col.left, "{at}");
                    }
                    // Visualizer at the end of the title line
                    let v = m.visualizer_bounds;
                    assert_eq!(v.right, col.right, "visualizer on the column edge {at}");
                    assert!(m.title_bounds.right < v.left, "title clear of viz {at}");
                    assert!(v.top >= m.title_bounds.top && v.bottom <= m.title_bounds.bottom);
                    // Row 2: full-width scrubber below the cover row
                    assert_eq!((t.left, t.right), (col.left, col.right), "full column {at}");
                    let row1_bottom = m.artwork_bounds.map_or(meta_bottom, |a| a.bottom);
                    assert!(t.top > row1_bottom, "scrubber below the cover row {at}");
                    // Row 3: controls centred below the scrubber
                    for k in MediaControl::ALL {
                        assert!(
                            m.control_bounds(k).top >= t.bottom,
                            "{k:?} below scrubber {at}"
                        );
                    }
                    let mid = |r: RectF| (r.left + r.right) / 2.0;
                    assert!(
                        (mid(play) - (col.left + col.right) / 2.0).abs() <= 1.0,
                        "centred {at}"
                    );
                    let (prev, next) = (
                        m.control_visual_bounds(MediaControl::Previous),
                        m.control_visual_bounds(MediaControl::Next),
                    );
                    assert!(((mid(prev) + mid(next)) / 2.0 - mid(play)).abs() < 0.001);
                    assert!(
                        play.width() > px(BASE_MEDIA_CONTROL_VISUAL_SIZE),
                        "larger {at}"
                    );
                    assert_eq!(m.time_bounds.width(), 0.0, "no clock {at}");
                    // Everything inside the content/notch; no overlaps
                    let mut visual = vec![m.title_bounds, v, t, prev, play, next];
                    if secondary_lines > 0 {
                        visual.push(m.artist_bounds);
                    }
                    visual.extend(m.artwork_bounds);
                    // (the larger controls' backdrops may reach into the bottom
                    // padding; the renderer's clip includes them, the notch too)
                    let area = RectF::new(col.left, c.top, col.right, c.bottom.max(play.bottom));
                    assert!(play.bottom <= dims.notch_height() - px(4.0), "{at}");
                    for (i, r) in visual.iter().enumerate() {
                        assert!(inside(r, &area), "visual {i} escapes {at}");
                        for o in visual.iter().skip(i + 1) {
                            assert!(!overlaps(r, o), "visual {i} overlaps {at}");
                        }
                    }
                    for k in MediaControl::ALL {
                        for (x, y) in corners(&m.control_bounds(k)) {
                            assert!(dims.contains_point(x, y), "{k:?} hit outside {at}");
                        }
                        assert!(m.control_bounds(k).bottom <= dims.notch_height(), "{at}");
                    }
                    assert!(
                        m.title_bounds.width() > 150.0 * dims.scale,
                        "title room {at}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_home_divider_between_text_and_clock_all_dpis() {
        for dpi in ALL_DPIS {
            for art in [true, false] {
                let (dims, m, c) = media_and_content(dpi, art);
                let d = m.divider_bounds;
                // Thin, whole-pixel, inside the content, spanning most of its height
                assert_eq!(d.width(), (dims.scale).round().max(1.0), "{dpi}");
                assert_eq!(d.left, d.left.round());
                assert!(inside(&d, &c), "inside content at {dpi}");
                assert!(d.height() > c.height() * 0.8, "tall at {dpi}");
                // Between the text column and the time/date column
                for r in [m.title_bounds, m.artist_bounds, m.source_bounds] {
                    assert!(r.right < d.left, "text stops before the divider at {dpi}");
                }
                assert!(d.right < m.time_bounds.left && d.right < m.date_bounds.left);
                // A little after the controls; the text stops just before it
                let px = |v: f32| (v * dims.scale).round();
                let controls = m.control_visual_bounds(MediaControl::Next).right;
                assert!(
                    (d.left - controls - px(BASE_MEDIA_DIVIDER_AFTER_CONTROLS)).abs() <= 1.0,
                    "after the controls at {dpi}"
                );
                assert_eq!(
                    m.title_bounds.right,
                    d.left - px(BASE_MEDIA_DIVIDER_TEXT_GAP)
                );
            }
            // Music has no divider
            let (_, m, _) = music(dpi, MediaShape::FULL);
            assert_eq!(m.divider_bounds.width(), 0.0);
        }
    }

    #[test]
    fn test_clear_confirmation_fits_and_hits_at_all_dpis() {
        for dpi in ALL_DPIS {
            let dims = space_dimensions(NotchState::Expanded, dpi, OecaSpace::Clipboard);
            let l = resolve_clipboard_layout(&dims).unwrap();
            // Inside the row area, stacked, and the buttons side by side
            // (Cancel first) without overlapping
            for r in [l.confirm_message, l.confirm_note, l.cancel, l.confirm_clear] {
                assert!(
                    r.left >= l.list.left && r.right <= l.list.right,
                    "{dpi}: {r:?} in {:?}",
                    l.list
                );
                assert!(r.top >= l.list.top && r.bottom <= l.list.bottom, "{dpi}");
            }
            assert!(l.confirm_message.bottom <= l.confirm_note.top);
            assert!(l.confirm_note.bottom < l.cancel.top);
            assert!(l.cancel.right < l.confirm_clear.left);
            assert_eq!(l.cancel.top, l.confirm_clear.top);
            assert_eq!(l.cancel.width(), l.confirm_clear.width());
            // Clear is nowhere near the header X (a double click on X never
            // confirms)
            assert!(l.clear.bottom < l.confirm_clear.top);
            let mid = |r: RectF| ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
            let (x, y) = mid(l.cancel);
            assert_eq!(l.confirm_hit(x, y), Some(PanelHit::CancelClear));
            let (x, y) = mid(l.confirm_clear);
            assert_eq!(l.confirm_hit(x, y), Some(PanelHit::ConfirmClear));
            for r in [l.clear, l.confirm_message, l.confirm_note] {
                let (x, y) = mid(r);
                assert_eq!(
                    l.confirm_hit(x, y),
                    None,
                    "{dpi}: only the buttons are live"
                );
            }
        }
    }

    #[test]
    fn test_home_timer_fits_the_empty_band_beside_media_at_all_dpis() {
        for dpi in ALL_DPIS {
            for artwork in [true, false] {
                for lines in [0, 1, 2] {
                    let shape = MediaShape {
                        artwork,
                        secondary_lines: lines,
                    };
                    let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
                    let m = resolve_media_layout(&dims, shape).unwrap();
                    let h = resolve_home_timer_layout(&dims, Some(shape)).unwrap();
                    let ResolvedLayout::Expanded { components, .. } = resolve_layout(&dims) else {
                        unreachable!()
                    };
                    let t = h.timer;
                    assert!(
                        h.clock.is_none() && h.divider.is_none(),
                        "media keeps its clock"
                    );
                    // Between the divider and the clock column, inside the content
                    assert!(t.panel.left > m.divider_bounds.right, "{dpi}");
                    assert!(t.panel.right <= m.time_bounds.left + 0.001, "{dpi}");
                    assert!(inside(&t.panel, &components.content_bounds), "{dpi}");
                    // Touches none of the media composition
                    let mut parts = vec![
                        m.title_bounds,
                        m.artist_bounds,
                        m.source_bounds,
                        m.time_bounds,
                        m.date_bounds,
                        m.divider_bounds,
                    ];
                    parts.extend(MediaControl::ALL.map(|c| m.control_bounds(c)));
                    parts.extend(m.artwork_bounds);
                    for r in parts {
                        assert!(!overlaps(&t.panel, &r), "{dpi} {artwork} {lines}: {r:?}");
                    }
                    timer_panel_is_well_formed(&t, dpi);
                }
            }
        }
    }

    #[test]
    fn test_home_without_media_is_clock_and_timer_columns_at_all_dpis() {
        for dpi in ALL_DPIS {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            let ResolvedLayout::Expanded { components, .. } = resolve_layout(&dims) else {
                unreachable!()
            };
            let h = resolve_home_timer_layout(&dims, None).unwrap();
            let (time, date) = h.clock.unwrap();
            let d = h.divider.unwrap();
            let c = components.content_bounds;
            // The clock keeps its rows (same heights), moved to the left
            // column; the timer takes the right one; the divider between
            let rows = |r: RectF| (r.top, r.bottom);
            assert_eq!(rows(time), rows(components.time_bounds));
            assert_eq!(rows(date), rows(components.date_bounds));
            assert_eq!(time.left, c.left);
            assert!(time.right < d.left && date.right < d.left, "{dpi}");
            assert!(d.right < h.timer.panel.left, "{dpi}");
            assert!(inside(&h.timer.panel, &c), "{dpi}");
            // Balanced: the divider is the content's centre
            let mid = (c.left + c.right) / 2.0;
            assert!((d.left - mid).abs() <= 1.0, "{dpi}");
            assert!(d.top > c.top && d.width() >= 1.0, "{dpi}");
            // Never the collapsed notch
            let collapsed = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
            assert!(resolve_home_timer_layout(&collapsed, None).is_none());
            timer_panel_is_well_formed(&h.timer, dpi);
        }
    }

    /// Everything inside the panel, in order top to bottom (pills, labels,
    /// ticks, pointer, buttons / numerals) with nothing overlapping; the
    /// scale's minutes evenly spaced about its fixed centre; every control
    /// hit-testable as drawn.
    fn timer_panel_is_well_formed(t: &TimerLayout, dpi: u32) {
        let scale = dpi as f32 / 96.0;
        for r in [
            t.modes[0], t.modes[1], t.labels, t.ruler, t.pointer, t.primary, t.cancel, t.numerals,
        ] {
            assert!(inside(&r, &t.panel), "{dpi}: {r:?} in {:?}", t.panel);
        }
        // Big enough to use: the whole of Home's content height, a real
        // scale, the full-size buttons and type
        assert!(t.panel.height() >= (70.0 * scale).floor(), "{dpi}: height");
        assert!(
            t.ruler.width() >= (88.0 * scale).floor(),
            "{dpi}: a usable scale"
        );
        let px = |v: f32| (v * scale).round();
        assert_eq!(
            t.primary.width(),
            px(crate::config::BASE_TIMER_BUTTON_SIZE),
            "{dpi}"
        );
        assert_eq!(
            t.modes[0].height(),
            px(crate::config::BASE_TIMER_PILL_HEIGHT),
            "{dpi}"
        );
        // Left column, top to bottom: pills, labels, ticks, pointer
        assert!(
            t.modes[0].right <= t.modes[1].left,
            "{dpi}: pills side by side"
        );
        let pills_mid = (t.modes[0].left + t.modes[1].right) / 2.0;
        assert!(
            (pills_mid - t.scale_centre()).abs() <= 1.0,
            "{dpi}: pills centred over the pointer"
        );
        assert!(t.modes[0].bottom <= t.labels.top, "{dpi}");
        assert!(
            t.labels.bottom <= t.ruler.top && t.ruler.bottom <= t.pointer.top,
            "{dpi}"
        );
        // Right column: the numerals over the buttons, clear of the scale;
        // the primary button at the right edge (it never moves), Cancel
        // beside it; equal circles
        assert!(t.ruler.right < t.numerals.left, "{dpi}: two columns");
        assert!(t.numerals.bottom <= t.primary.top, "{dpi}");
        assert_eq!(
            t.numerals.right, t.panel.right,
            "right-aligned to the panel"
        );
        assert_eq!(t.primary.right, t.panel.right, "{dpi}");
        assert_eq!(t.primary.width(), t.primary.height(), "{dpi}: round");
        assert_eq!(t.primary.width(), t.cancel.width());
        assert!(t.cancel.right < t.primary.left, "{dpi}");
        assert!(t.ruler.right < t.cancel.left, "{dpi}");
        // The pointer sits under the scale's centre; stops evenly spaced,
        // the same at every value
        let centre = t.scale_centre();
        assert!(
            (((t.pointer.left + t.pointer.right) / 2.0) - centre).abs() <= 1.0,
            "{dpi}"
        );
        assert_eq!(t.step, (crate::config::BASE_TIMER_STEP * scale).round());
        for value in [-1.0f32, 0.0, 25.0, 59.5, 122.0] {
            assert_eq!(
                t.stop_x(value, value),
                centre,
                "{dpi}: the value at the centre"
            );
            let step = t.stop_x(value + 1.0, value) - t.stop_x(value, value);
            assert!((step - t.step).abs() < 1e-3, "{dpi}: even spacing");
            assert!((t.stop_at(t.stop_x(value + 3.0, value), value) - (value + 3.0)).abs() < 1e-3);
        }
        // Hits: everything works in every phase (Cancel once started)
        let mid = |r: RectF| ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        use crate::timer::Mode;
        for idle in [true, false] {
            let (px, py) = mid(t.primary);
            assert_eq!(t.hit(px, py, idle), Some(PanelHit::TimerPrimary));
            let (cx, cy) = mid(t.cancel);
            let cancel = (!idle).then_some(PanelHit::TimerCancel);
            assert_eq!(t.hit(cx, cy, idle), cancel, "Cancel only once started");
            for (x, y) in [mid(t.ruler), mid(t.labels), mid(t.pointer)] {
                assert_eq!(t.hit(x, y, idle), Some(PanelHit::TimerScale), "{dpi}");
            }
            for mode in Mode::ALL {
                let (mx, my) = mid(t.modes[mode.index()]);
                assert_eq!(t.hit(mx, my, idle), Some(PanelHit::TimerMode(mode)));
            }
        }
        let (nx, ny) = (t.numerals.right - 2.0, mid(t.numerals).1);
        assert_eq!(t.hit(nx, ny, true), None, "numerals are not a control");
        assert!(PanelHit::TimerPrimary.is_button() && PanelHit::TimerCancel.is_button());
        assert!(!PanelHit::TimerScale.is_button() && !PanelHit::TimerMode(Mode::Focus).is_button());
    }

    #[test]
    fn test_clipboard_image_is_whole_pixels_and_evenly_inset() {
        for dpi in ALL_DPIS {
            let dims = space_dimensions(NotchState::Expanded, dpi, OecaSpace::Clipboard);
            let l = resolve_clipboard_layout(&dims).unwrap();
            for i in 0..crate::config::CLIPBOARD_TILES {
                let p = l.tile_parts(i);
                let img = p.image;
                for v in [img.left, img.top, img.right, img.bottom] {
                    assert_eq!(v, v.round(), "{dpi}: whole pixels");
                }
                // The same inset from the tile on the left, top and right (so
                // one nested corner fits all of them)
                let (l_in, t_in, r_in) = (
                    img.left - p.tile.left,
                    img.top - p.tile.top,
                    p.tile.right - img.right,
                );
                assert!(
                    (l_in - t_in).abs() <= 1.0 && (l_in - r_in).abs() <= 1.0,
                    "{dpi}: {l_in} {t_in} {r_in}"
                );
                assert!(img.width() > 0.0 && img.height() > 0.0);
                assert!(img.bottom <= p.copy.top, "{dpi}: clear of the buttons");
            }
        }
    }

    #[test]
    fn test_hud_digit_cells_are_fixed_right_aligned_and_inside() {
        for dpi in ALL_DPIS {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
            let text = collapsed_hud_layout(&resolve_collapsed_layout(&dims), dims.scale).text;
            let cell = (7.5 * dims.scale).round();
            let cells = hud_digit_cells(text, cell);
            assert_eq!(cells[2].right, text.right, "{dpi}: right-aligned");
            for k in 0..2 {
                assert_eq!(cells[k].right, cells[k + 1].left, "{dpi}: contiguous");
            }
            assert!(cells[0].left >= text.left, "{dpi}: three digits fit");
            assert!(cells.iter().all(|c| c.width() == cell && c.top == text.top));
        }
    }

    #[test]
    fn test_hud_rows_fit_collapsed_and_header_at_all_dpis() {
        let check = |h: HudLayout, bounds: RectF, what: &str| {
            for r in [h.icon, h.bar, h.text] {
                assert!(inside(&r, &bounds), "{what}: {r:?} in {bounds:?}");
            }
            assert!(
                h.icon.right < h.bar.left && h.bar.right < h.text.left,
                "{what}"
            );
            assert!(h.bar.width() >= 40.0, "{what}: a usable bar");
            assert!(h.bar.height() >= 2.0 && h.glyph <= h.icon.height().max(h.icon.width()));
            // Centred on the row
            let mid = |r: RectF| (r.top + r.bottom) / 2.0;
            assert!((mid(h.bar) - mid(h.text)).abs() <= 1.0, "{what}");
        };
        for dpi in ALL_DPIS {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
            let l = resolve_collapsed_layout(&dims);
            check(
                collapsed_hud_layout(&l, dims.scale),
                l.content_bounds,
                "collapsed",
            );
            for space in OecaSpace::ALL {
                let dims = space_dimensions(NotchState::Expanded, dpi, space);
                let sel = resolve_space_selector_in(&dims, space).unwrap();
                let header = RectF::new(
                    sel.home.left,
                    sel.home.top,
                    sel.settings.right,
                    sel.home.bottom,
                );
                check(header_hud_layout(&sel, dims.scale), header, "header");
            }
        }
    }

    #[test]
    fn test_timer_slot_and_bubble_geometry_at_all_dpis() {
        for dpi in ALL_DPIS {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
            let l = resolve_collapsed_layout(&dims);
            let slot = timer_slot_rect(&l, dims.scale);
            // The visualizer's place (same right edge), inside the content,
            // right of the centre (the clock)
            assert_eq!(slot.right, l.visualizer_bounds.right);
            assert!(slot.left <= l.visualizer_bounds.left);
            assert!(inside(&slot, &l.content_bounds), "{dpi}");
            assert!(slot.left > dims.width as f32 / 2.0, "{dpi}");
            // Bubble: a floating circle, centred on the notch's height and
            // clear of the screen edge once out; budding from / absorbed into
            // the notch's wall through a liquid neck
            let px = |v: f32| (v * dims.scale).round();
            let full = timer_bubble_geometry(dpi, 1.0);
            let (w, h) = full.window;
            assert_eq!(h, dims.height, "{dpi}");
            let (cx, cy) = full.center;
            let r = full.radius;
            assert_eq!(
                r * 2.0,
                px(crate::config::BASE_TIMER_BUBBLE_HEIGHT),
                "a circle at {dpi}"
            );
            assert!((cy - h as f32 / 2.0).abs() < 0.01, "centred at {dpi}");
            assert!(cy - r >= 1.0, "clear of the screen edge at {dpi}");
            assert_eq!(
                cx - r - full.wall,
                px(crate::config::BASE_TIMER_BUBBLE_GAP),
                "{dpi}"
            );
            assert!(cx + r <= w as f32, "inside its window at {dpi}");
            assert_eq!(full.neck, 0.0, "detached when out at {dpi}");
            assert!(
                full.ring.0 > 0.0 && full.ring.0 + full.ring.1 / 2.0 < r,
                "ring inside at {dpi}"
            );
            // In between: moving into the wall, shrinking a little; the neck
            // forms as it nears and stays inside the circle's height
            let mut last_x = cx;
            let mut neck_seen = false;
            for k in (0..=40).rev() {
                let g = timer_bubble_geometry(dpi, k as f32 / 40.0);
                assert!(g.center.0 <= last_x + 1e-3, "moves one way at {dpi}");
                assert!((g.center.0 - last_x).abs() < r * 0.2, "no jump at {dpi}");
                last_x = g.center.0;
                assert!(g.radius <= r && g.radius >= r * 0.5 - 1e-3);
                assert!(g.neck <= g.radius, "{dpi}");
                neck_seen |= g.neck > 0.0;
                assert!(g.center.0 + g.radius <= g.window.0 as f32);
            }
            assert!(neck_seen, "a liquid neck while close at {dpi}");
            // Absorbed: entirely behind the wall
            let gone = timer_bubble_geometry(dpi, 0.0);
            assert!(
                gone.center.0 + gone.radius <= gone.wall,
                "inside the notch at {dpi}"
            );
            // The overshoot past fully out still fits its window
            let over = timer_bubble_geometry(dpi, 1.05);
            assert!(over.center.0 + over.radius <= over.window.0 as f32, "{dpi}");
        }
    }
}
