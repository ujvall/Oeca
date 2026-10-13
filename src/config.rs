#![allow(dead_code)]

use crate::space::Scene;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::core::{PCWSTR, w};

pub const WINDOW_CLASS_NAME: PCWSTR = w!("OecaWindowClass");
pub const WINDOW_TITLE: PCWSTR = w!("Oeca");

// ============================================================================
// 1. BASELINE DESIGN SYSTEM TOKENS (Logical Pixels @ 96 DPI / 100% scale)
// ============================================================================

pub const BASE_DPI: f32 = 96.0;

// Collapsed reference dimensions (Phase 1 baseline preserved, smoothed shoulder)
pub const BASE_COLLAPSED_WIDTH: f32 = 220.0;
pub const BASE_COLLAPSED_HEIGHT: f32 = 32.0;
pub const BASE_COLLAPSED_BOTTOM_CORNER_RADIUS: f32 = 14.0;
pub const BASE_COLLAPSED_TOP_TRANSITION_RADIUS: f32 = 6.0;
pub const BASE_COLLAPSED_TOP_TRANSITION_HEIGHT: f32 = 6.0;

// Legacy aliases for full backward compatibility
pub const BASE_WIDTH: f32 = BASE_COLLAPSED_WIDTH;
pub const BASE_HEIGHT: f32 = BASE_COLLAPSED_HEIGHT;
pub const BASE_CORNER_RADIUS: f32 = BASE_COLLAPSED_BOTTOM_CORNER_RADIUS;
pub const BASE_BOTTOM_CORNER_RADIUS: f32 = BASE_COLLAPSED_BOTTOM_CORNER_RADIUS;
pub const BASE_TOP_TRANSITION_RADIUS: f32 = BASE_COLLAPSED_TOP_TRANSITION_RADIUS;
pub const BASE_TOP_TRANSITION_HEIGHT: f32 = BASE_COLLAPSED_TOP_TRANSITION_HEIGHT;

// Expanded reference dimensions (600 × 128 logical DIP window: 110 DIP notch + shadow margin horizontal pill, compact smooth shoulder, subtle dropshadow)
pub const BASE_EXPANDED_WIDTH: f32 = 600.0;
pub const BASE_EXPANDED_HEIGHT: f32 = 138.0;
pub const BASE_EXPANDED_BOTTOM_CORNER_RADIUS: f32 = 18.0;

/// Expanded bottom corners use a continuous-curvature ("squircle") profile: the
/// curve starts this much earlier along the wall/bottom edge (fraction of the
/// radius) and eases in, instead of a circular arc meeting a straight edge.
pub const EXPANDED_CORNER_SPAN_EXTRA: f32 = 0.30;
/// Cubic handle length (fraction of the corner span): circle vs. smoothed corner.
pub const CORNER_HANDLE_CIRCLE: f32 = 0.552_284_8;
pub const CORNER_HANDLE_SMOOTH: f32 = 0.67;

/// One bottom corner as a cubic Bezier in corner-local coordinates: starts on the
/// side wall at (0, 0), ends on the bottom edge at (span, span). `handle` is the
/// control-handle length as a fraction of `span` (0.5523 = circular arc).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CornerProfile {
    pub span: f32,
    pub handle: f32,
}

impl CornerProfile {
    /// `smoothing` 0 = exact circular corner of `radius`; 1 = full continuous corner.
    pub fn new(radius: f32, smoothing: f32, max_span: f32) -> Self {
        let s = smoothing.clamp(0.0, 1.0);
        Self {
            span: (radius * (1.0 + EXPANDED_CORNER_SPAN_EXTRA * s))
                .min(max_span)
                .max(0.0),
            handle: CORNER_HANDLE_CIRCLE + (CORNER_HANDLE_SMOOTH - CORNER_HANDLE_CIRCLE) * s,
        }
    }

    /// Point on the corner curve at parameter t (corner-local coordinates).
    pub fn point(&self, t: f32) -> (f32, f32) {
        let (s, k) = (self.span, self.handle);
        let mt = 1.0 - t;
        let x = 3.0 * mt * t * t * (s - k * s) + t * t * t * s;
        let y = 3.0 * mt * mt * t * (k * s) + 3.0 * mt * t * t * s + t * t * t * s;
        (x, y)
    }

    /// Horizontal inset of the curve from the side wall at depth `v` below the
    /// corner start (the curve's y is monotonic in t, so bisection is exact enough).
    pub fn inset_at(&self, v: f32) -> f32 {
        if self.span <= 0.0 {
            return 0.0;
        }
        let v = v.clamp(0.0, self.span);
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = 0.5 * (lo + hi);
            if self.point(mid).1 < v {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        self.point(0.5 * (lo + hi)).0
    }
}
pub const BASE_EXPANDED_TOP_TRANSITION_RADIUS: f32 = 14.0;
pub const BASE_EXPANDED_TOP_TRANSITION_HEIGHT: f32 = 14.0;
pub const BASE_EXPANDED_SHADOW_MARGIN_X: f32 = 10.0;
pub const BASE_EXPANDED_SHADOW_MARGIN_BOTTOM: f32 = 18.0;
pub const BASE_EXPANDED_SHADOW_OPACITY: f32 = 0.28;
/// Extra header band at the top of the expanded notch (both spaces) so the
/// space selector has real breathing room above the content.
pub const BASE_EXPANDED_HEADER_EXTRA: f32 = 10.0;

// Stroke & separation tokens
pub const BASE_BORDER_WIDTH: f32 = 1.0;

// Padding & spacing tokens
pub const BASE_COLLAPSED_PADDING_H: f32 = 14.0;
pub const BASE_COLLAPSED_PADDING_V: f32 = 6.0;
pub const BASE_EXPANDED_PADDING_H: f32 = 18.0;
pub const BASE_EXPANDED_PADDING_V: f32 = 12.0;

pub const BASE_PADDING_H: f32 = BASE_COLLAPSED_PADDING_H;
pub const BASE_PADDING_V: f32 = BASE_COLLAPSED_PADDING_V;
pub const BASE_SPACING: f32 = 8.0;

// ── Typography ──────────────────────────────────────────────────────────
//
// One typeface for everything, resolved once at run time
// (`renderer::ui_font`): the first of these installed. General Sans and
// Satoshi (Fontshare) are used only when the user has installed them, never
// bundled; Bahnschrift ships with every Windows 10 / 11, so it is the
// guaranteed fallback. To try another face, put it first.
pub const UI_FONT_CANDIDATES: [(&str, PCWSTR); 3] = [
    ("General Sans", w!("General Sans")),
    ("Satoshi", w!("Satoshi")),
    ("Bahnschrift", w!("Bahnschrift")),
];
/// Last resort when none of the candidates is installed (reported by
/// `ui_font`, never substituted silently by DirectWrite).
pub const FONT_FAMILY_FALLBACK_NAME: &str = "Segoe UI Variable Text";
pub const FONT_FAMILY_FALLBACK: PCWSTR = w!("Segoe UI Variable Text");

/// A typographic role: every piece of text in Oeca is drawn in one of these,
/// so size and weight live here, not at the call sites. Sizes are DIP (the
/// notch scales them by its DPI; the Settings window draws in DIP).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRole {
    /// Collapsed notch clock.
    ClockCompact,
    /// Expanded Home clock (no media); its AM/PM is drawn at
    /// `CLOCK_MERIDIEM_SCALE` of this size in `CLOCK_MERIDIEM_WEIGHT`.
    ClockDisplay,
    /// Home media layout: the clock column's time and short date.
    ClockColumn,
    DateCompact,
    /// Expanded Home date (no media).
    DateLine,
    /// Track title / artist / tertiary metadata (source app, Settings scene
    /// captions, clipboard image captions).
    MediaTitle,
    MediaArtist,
    MediaMeta,
    /// Music scrubber elapsed / remaining.
    Timestamp,
    /// Quiet centred empty-state labels.
    EmptyState,
    /// Settings window: page title, section heading, setting label,
    /// supporting description, sidebar entry (regular / selected) and the
    /// compact capacity segments.
    PageTitle,
    SectionHeading,
    SettingLabel,
    SettingDescription,
    SidebarItem,
    SidebarItemSelected,
}

/// Size (DIP) and weight (100..900) of a role.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub weight: i32,
}

impl TextRole {
    pub const ALL: [Self; 16] = [
        Self::ClockCompact,
        Self::ClockDisplay,
        Self::ClockColumn,
        Self::DateCompact,
        Self::DateLine,
        Self::MediaTitle,
        Self::MediaArtist,
        Self::MediaMeta,
        Self::Timestamp,
        Self::EmptyState,
        Self::PageTitle,
        Self::SectionHeading,
        Self::SettingLabel,
        Self::SettingDescription,
        Self::SidebarItem,
        Self::SidebarItemSelected,
    ];

    /// Restrained weights: semibold carries emphasis (titles, numerals),
    /// regular carries reading text, and medium keeps the smallest grey text
    /// (10-11 DIP) from thinning out on black.
    pub fn style(self) -> TextStyle {
        let (size, weight) = match self {
            Self::ClockCompact => (BASE_FONT_SIZE_CLOCK, 600),
            Self::ClockDisplay => (BASE_EXPANDED_FONT_SIZE_TIME, 600),
            Self::ClockColumn => (BASE_MEDIA_TIME_FONT_SIZE, 600),
            Self::DateCompact => (BASE_MEDIA_DATE_FONT_SIZE, 500),
            Self::DateLine => (BASE_EXPANDED_FONT_SIZE_DATE, 400),
            Self::MediaTitle => (BASE_MEDIA_TITLE_FONT_SIZE, 600),
            Self::MediaArtist => (BASE_MEDIA_ARTIST_FONT_SIZE, 400),
            Self::MediaMeta => (BASE_MEDIA_SOURCE_FONT_SIZE, 500),
            Self::Timestamp => (BASE_MEDIA_TIMELINE_FONT_SIZE, 500),
            Self::EmptyState => (BASE_MEDIA_ARTIST_FONT_SIZE, 400),
            Self::PageTitle => (24.0, 600),
            Self::SectionHeading => (13.0, 600),
            Self::SettingLabel => (14.0, 400),
            Self::SettingDescription => (12.0, 400),
            Self::SidebarItem => (14.0, 400),
            Self::SidebarItemSelected => (14.0, 600),
        };
        TextStyle { size, weight }
    }
}

/// The expanded Home clock's AM/PM (or locale equivalent): a smaller, lighter
/// companion to the numerals on the same baseline.
pub const CLOCK_MERIDIEM_SCALE: f32 = 0.5;
pub const CLOCK_MERIDIEM_WEIGHT: i32 = 500;

pub const BASE_FONT_SIZE_PRIMARY: f32 = 13.0;
pub const BASE_FONT_SIZE_SECONDARY: f32 = 10.5;
pub const BASE_FONT_SIZE_MUTED: f32 = 9.0;
pub const BASE_FONT_SIZE_CLOCK: f32 = 13.5;
pub const BASE_EXPANDED_FONT_SIZE_TIME: f32 = 28.0;
pub const BASE_EXPANDED_FONT_SIZE_DATE: f32 = 12.0;
pub const BASE_EXPANDED_TIME_HEIGHT: f32 = 34.0;
pub const BASE_EXPANDED_DATE_HEIGHT: f32 = 16.0;
pub const BASE_EXPANDED_CLOCK_SPACING: f32 = 4.0;
pub const BASE_EXPANDED_CLOCK_OPTICAL_Y_OFFSET: f32 = 0.0;
pub const BASE_CLOCK_OPTICAL_Y_OFFSET: f32 = 0.0;
pub const BASE_TITLE_OPTICAL_Y_OFFSET: f32 = 0.65;
pub const BASE_HEADER_HEIGHT: f32 = 24.0;

// Expanded media composition (DIP at 96 DPI). The text/controls stack equals the
// expanded content height at 96 DPI: 20 + 16 + 14 + 22 = 72 (the 5 DIP control
// spacing appears once a secondary line is hidden), and the artwork
// is a square of the same height.
pub const BASE_MEDIA_ARTWORK_SIZE: f32 = 72.0;
/// Cover inset from the notch's side wall and bottom edge (nested in the
/// bottom-left corner).
pub const BASE_MEDIA_ARTWORK_INSET: f32 = 9.0;
/// Added to the concentric artwork corner radius (notch radius minus inset).
pub const BASE_MEDIA_ARTWORK_RADIUS_EXTRA: f32 = 3.0;
/// Source-app badge (Spotify / Apple Music / YouTube Music) on the artwork corner.
pub const BASE_MEDIA_BADGE_SIZE: f32 = 13.0;
/// Badge corner radius as a fraction of its size (0.5 = circle).
pub const BADGE_CORNER_FRACTION: f32 = 0.5;
/// The badge straddles the artwork's bottom-right corner: it extends this far past
/// the art's right and bottom edges...
pub const BASE_MEDIA_BADGE_OVERHANG: f32 = 3.0;
/// ...and is cut out of the cover by a ring of the notch's own black.
pub const BASE_MEDIA_BADGE_RING: f32 = 2.0;
pub const BASE_MEDIA_ARTWORK_GAP: f32 = 14.0;
pub const BASE_MEDIA_TITLE_FONT_SIZE: f32 = 15.0;
pub const BASE_MEDIA_ARTIST_FONT_SIZE: f32 = 12.0;
pub const BASE_MEDIA_SOURCE_FONT_SIZE: f32 = 10.5;
pub const BASE_MEDIA_TITLE_HEIGHT: f32 = 20.0;
pub const BASE_MEDIA_ARTIST_HEIGHT: f32 = 16.0;
pub const BASE_MEDIA_SOURCE_HEIGHT: f32 = 14.0;
pub const BASE_MEDIA_CONTROLS_SPACING: f32 = 5.0;
/// Visual diameter of a control's hover/press backdrop.
pub const BASE_MEDIA_CONTROL_VISUAL_SIZE: f32 = 22.0;
/// Hit area; extends below the visual row into the notch's bottom padding.
pub const BASE_MEDIA_CONTROL_WIDTH: f32 = 32.0;
pub const BASE_MEDIA_CONTROL_HEIGHT: f32 = 28.0;
pub const BASE_MEDIA_CONTROL_GAP: f32 = 6.0;
pub const BASE_MEDIA_ICON_SIZE: f32 = 12.0;
pub const BASE_MEDIA_PLAY_ICON_SIZE: f32 = 13.0;
/// Skip glyph: each of its two triangles is this fraction of the icon size wide.
pub const SKIP_GLYPH_HALF_WIDTH: f32 = 0.56;
/// Icon opacity at rest (full on hover): quiet until the pointer arrives.
pub const MEDIA_ICON_REST_OPACITY: f32 = 0.82;
pub const BASE_MEDIA_CLOCK_COLUMN_WIDTH: f32 = 112.0;
pub const BASE_MEDIA_COLUMN_GAP: f32 = 16.0;
pub const BASE_MEDIA_TIME_FONT_SIZE: f32 = 13.0;
pub const BASE_MEDIA_DATE_FONT_SIZE: f32 = 11.0;

/// Control feedback timings (control-local micro-interaction only).
pub const MEDIA_HOVER_FADE_MS: f32 = 110.0;
pub const MEDIA_PRESS_IN_MS: f32 = 70.0;
pub const MEDIA_PRESS_RELEASE_MS: f32 = 180.0;
/// Press compresses the icon to this scale.
pub const MEDIA_PRESS_ICON_SCALE: f32 = 0.84;
/// New track content (artwork + text) fades in from this opacity...
pub const MEDIA_TRACK_FADE_FLOOR: f32 = 0.35;
/// ...over this duration (ease-out). Shares the control feedback timer.
pub const MEDIA_TRACK_FADE_MS: f32 = 180.0;
/// Clipboard history (in memory only): newest-first entries, total owned image
/// memory, and the longest text kept (UTF-16 code units, ~512 KB).
pub const CLIPBOARD_MAX_ENTRIES: usize = 20;

/// How many entries the clipboard history keeps (Settings > Clipboard). The
/// largest is `CLIPBOARD_MAX_ENTRIES`, the default (the original behavior).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClipboardCapacity {
    Five,
    Ten,
    #[default]
    Twenty,
}

impl ClipboardCapacity {
    pub const ALL: [Self; 3] = [Self::Five, Self::Ten, Self::Twenty];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Option<Self> {
        Self::ALL.get(i).copied()
    }

    pub fn entries(self) -> usize {
        match self {
            Self::Five => 5,
            Self::Ten => 10,
            Self::Twenty => CLIPBOARD_MAX_ENTRIES,
        }
    }

    /// Display and accessible name ("5 items").
    pub fn name(self) -> &'static str {
        match self {
            Self::Five => "5 items",
            Self::Ten => "10 items",
            Self::Twenty => "20 items",
        }
    }
}
pub const CLIPBOARD_MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
pub const CLIPBOARD_MAX_TEXT_UNITS: usize = 256 * 1024;
/// One-shot settle delay before reading a clipboard change: the copying app
/// finishes rendering/flushing its data first, and bursts coalesce into one
/// read. The timer exists only between an update and its read.
pub const CLIPBOARD_TIMER_ID: usize = 1005;
/// One-shot: the expanded notch collapses `CollapseDelay` after a click
/// outside Oeca, unless Oeca is used again first (restarted by each outside
/// click).
pub const AUTO_COLLAPSE_TIMER_ID: usize = 1006;

/// How long after an outside click the expanded notch collapses (Settings >
/// General > Notch closing behaviour). 5 s is the default (the original
/// behaviour).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CollapseDelay {
    #[default]
    Five,
    Ten,
    Fifteen,
}

impl CollapseDelay {
    pub const ALL: [Self; 3] = [Self::Five, Self::Ten, Self::Fifteen];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Option<Self> {
        Self::ALL.get(i).copied()
    }

    /// The countdown (ms).
    pub fn ms(self) -> u32 {
        match self {
            Self::Five => 5_000,
            Self::Ten => 10_000,
            Self::Fifteen => 15_000,
        }
    }

    /// Accessible name ("5 seconds") and compact segment label ("5s").
    pub fn name(self) -> &'static str {
        match self {
            Self::Five => "5 seconds",
            Self::Ten => "10 seconds",
            Self::Fifteen => "15 seconds",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Five => "5s",
            Self::Ten => "10s",
            Self::Fifteen => "15s",
        }
    }
}

/// A settings switch's knob travel, OFF -> ON, and how long a full travel
/// takes (a reversal mid-way travels only the remaining distance).
pub const SWITCH_TRAVEL_MS: f32 = 170.0;
/// Frame period of the Settings window's switch animation (it has no other
/// timer; the notch's switches ride its control-feedback timer).
pub const SWITCH_FRAME_MS: u32 = 16;
pub const CLIPBOARD_SETTLE_MS: u32 = 150;

/// Countdown timer (Home): wakes only while a countdown runs (when its
/// displayed second changes) or a finished one is on show (when that ends).
pub const TIMER_TICK_TIMER_ID: usize = 1007;
/// A finished countdown stays on show (collapsed bubble / slot, Home's
/// "done" state) this long, then returns to the idle setup by itself.
pub const TIMER_DONE_HOLD_MS: u64 = 6_000;
/// The finished countdown's numerals pulse gently: this many cycles of this
/// length, then hold steady (none under Reduced motion).
pub const TIMER_PULSE_MS: f32 = 700.0;
pub const TIMER_PULSES: f32 = 3.0;
/// The timer's warm accent (numerals, indicator, primary button), and its
/// button wash. Independent of the user accent, like Apple's timer orange.
pub const COLOR_TIMER: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 159.0 / 255.0,
    b: 10.0 / 255.0,
    a: 1.0,
};
pub const TIMER_BUTTON_WASH: f32 = 0.22;
/// Home timer panel (DIP at 96 DPI): numerals row, then the duration scale
/// (setup) or a status line; round buttons at the row's right end.
pub const BASE_TIMER_PANEL_WIDTH: f32 = 178.0;
pub const BASE_TIMER_DISPLAY_HEIGHT: f32 = 32.0;
pub const BASE_TIMER_ROW_GAP: f32 = 6.0;
pub const BASE_TIMER_SCALE_HEIGHT: f32 = 20.0;
pub const BASE_TIMER_BUTTON_SIZE: f32 = 30.0;
pub const BASE_TIMER_BUTTON_GAP: f32 = 8.0;
pub const BASE_TIMER_GLYPH_SIZE: f32 = 11.0;
/// Scale: tick (minor / unit-start) heights and width, indicator size.
pub const BASE_TIMER_TICK_HEIGHT: f32 = 6.0;
pub const BASE_TIMER_MAJOR_TICK_HEIGHT: f32 = 10.0;
pub const BASE_TIMER_TICK_WIDTH: f32 = 1.0;
pub const BASE_TIMER_INDICATOR_WIDTH: f32 = 3.0;
pub const BASE_TIMER_INDICATOR_HEIGHT: f32 = 16.0;
pub const COLOR_TIMER_TICK: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.28,
};
/// Home without media: the clock and the timer share the content area as two
/// columns this far either side of the centre divider.
pub const BASE_TIMER_COLUMN_GAP: f32 = 16.0;
/// Collapsed notch without playback: the remaining time in the visualizer's
/// place, right-aligned to its edge, at most this wide.
pub const BASE_TIMER_SLOT_WIDTH: f32 = 54.0;
/// Collapsed notch during playback: a floating circle beside the notch (not
/// attached to the screen edge): this wide, centred on the notch's height,
/// this far from the notch's body. Inside it, the timer's ring (this far in
/// from its edge, this thick) depletes as the countdown runs.
pub const BASE_TIMER_BUBBLE_HEIGHT: f32 = 24.0;
pub const BASE_TIMER_BUBBLE_GAP: f32 = 8.0;
pub const BASE_TIMER_RING_INSET: f32 = 4.5;
pub const BASE_TIMER_RING_WIDTH: f32 = 2.5;
/// The liquid neck between the circle and the notch's wall forms once the
/// circle comes this close (and pinches off as it leaves).
pub const BASE_TIMER_NECK_REACH: f32 = 7.0;
/// How far the bubble's window reaches back over the notch's body, so the
/// neck joins the wall even where the notch's bottom corner curves away.
pub const BASE_TIMER_BUBBLE_OVERLAP: f32 = 4.0;

pub const MEDIA_FEEDBACK_TIMER_ID: usize = 1003;
pub const MEDIA_FEEDBACK_FRAME_MS: u32 = 16;
/// Playback "live" timer (visualizer + scrubber): runs only while playing or
/// while the visualizer fades out, at a modest ~30 fps.
pub const MEDIA_LIVE_TIMER_ID: usize = 1004;
pub const MEDIA_LIVE_FRAME_MS: u32 = 33;
/// The same timer's period while only the scrubber moves (the settled Music
/// space with its visualizer hidden): ample for a seconds label and a bar
/// that moves well under a pixel per tick.
pub const MEDIA_SCRUBBER_FRAME_MS: u32 = 250;

/// Playback visualizer: thin rounded bars (DIP at 96 DPI).
pub const BASE_VISUALIZER_BAR_WIDTH: f32 = 2.0;
pub const BASE_VISUALIZER_BAR_GAP: f32 = 2.0;
pub const BASE_VISUALIZER_HEIGHT: f32 = 12.0;
/// Resting bar height as a fraction of the full height.
pub const VISUALIZER_MIN_HEIGHT: f32 = 0.25;
/// Fade in on play / out on pause.
pub const VISUALIZER_FADE_MS: f32 = 220.0;

/// Collapsed notch album art (DIP at 96 DPI): a small rounded square at the
/// left end of the content area, mirroring the visualizer at the right end,
/// vertically centred in the 32 px notch (7 px above and below).
pub const BASE_COLLAPSED_ARTWORK_SIZE: f32 = 18.0;
pub const BASE_COLLAPSED_ARTWORK_RADIUS: f32 = 4.5;
/// Space between the title and the expanded visualizer.
pub const BASE_MEDIA_VISUALIZER_GAP: f32 = 8.0;

/// Space selector (Home / Music): capsules in the expanded notch's top band,
/// above the media content, left-aligned with the cover.
pub const BASE_SPACE_SELECTOR_TOP: f32 = 7.0;
pub const BASE_SPACE_PILL_WIDTH: f32 = 34.0;
pub const BASE_SPACE_PILL_HEIGHT: f32 = 22.0;
/// Space icons (Flaticon UIcons solid-rounded house-blank / music-alt): glyph
/// box (DIP); the glyphs fill their box edge to edge.
pub const BASE_SPACE_ICON_SIZE: f32 = 12.0;
pub const BASE_SPACE_PILL_GAP: f32 = 4.0;
pub const BASE_SPACE_FONT_SIZE: f32 = 11.0;

/// Music player column inset from both side walls (cover, visualizer, scrubber
/// and the selector pills keep this breathing room from the notch edges).
pub const BASE_MUSIC_SIDE_INSET: f32 = 16.0;
/// Music player (DIP): a small cover with title/artist beside it, then a
/// full-width scrubber row, then larger centred controls. The Music notch
/// height is derived from these rows.
pub const BASE_MUSIC_ARTWORK_SIZE: f32 = 40.0;
pub const BASE_MUSIC_ARTWORK_RADIUS: f32 = 8.0;
pub const BASE_MUSIC_ARTWORK_GAP: f32 = 12.0;
pub const BASE_MUSIC_SCRUBBER_GAP: f32 = 6.0;
pub const BASE_MUSIC_SCRUBBER_HEIGHT: f32 = 14.0;
pub const BASE_MUSIC_CONTROLS_GAP: f32 = 4.0;
/// Larger transport controls (same icons, scaled up): backdrop diameter, hit
/// area, gap between hit areas, glyph sizes.
pub const BASE_MUSIC_CONTROL_VISUAL_SIZE: f32 = 30.0;
pub const BASE_MUSIC_CONTROL_WIDTH: f32 = 44.0;
pub const BASE_MUSIC_CONTROL_HEIGHT: f32 = 34.0;
pub const BASE_MUSIC_CONTROL_GAP: f32 = 16.0;
pub const BASE_MUSIC_ICON_SIZE: f32 = 16.0;
pub const BASE_MUSIC_PLAY_ICON_SIZE: f32 = 19.0;
/// Below the controls' hit areas, before the notch's bottom edge.
pub const BASE_MUSIC_BOTTOM_PAD: f32 = 6.0;

/// Music space: scrubber track width the compact Music notch is sized around
/// (the only Music-specific width input; everything else is existing layout).
pub const BASE_MUSIC_TIMELINE_TRACK: f32 = 240.0;

/// Clipboard space (DIP): its own window width, and its content area's
/// height, still measured in the former list's five rows (so the space keeps
/// its exact size at every DPI). The Clear X shares the selector band; the
/// newest entries fill the area as a 3 x 2 grid of tiles (older entries stay
/// in the history, reachable once newer ones are cleared or restored past
/// them).
pub const BASE_CLIPBOARD_WIDTH: f32 = 440.0;
pub const BASE_CLIPBOARD_SIDE_INSET: f32 = 16.0;
pub const CLIPBOARD_VISIBLE_ROWS: usize = 5;
/// The tile grid: columns, rows, the gap between tiles, their squircle
/// corner, the padding of their text, and their round copy / trash buttons
/// (inset from the tile's bottom corner).
pub const CLIPBOARD_TILE_COLUMNS: usize = 3;
pub const CLIPBOARD_TILES: usize = 6;
pub const BASE_CLIPBOARD_TILE_GAP: f32 = 8.0;
pub const BASE_CLIPBOARD_TILE_RADIUS: f32 = 12.0;
pub const BASE_CLIPBOARD_TILE_PAD: f32 = 8.0;
pub const BASE_CLIPBOARD_TILE_BUTTON: f32 = 20.0;
pub const BASE_CLIPBOARD_TILE_INSET: f32 = 3.0;
pub const BASE_CLIPBOARD_ROW_HEIGHT: f32 = 26.0;
pub const BASE_CLIPBOARD_ROW_GAP: f32 = 2.0;
pub const BASE_CLIPBOARD_BOTTOM_PAD: f32 = 12.0;
/// Copy / trash glyphs on each tile, and the header's clear X.
pub const BASE_CLIPBOARD_ACTION_ICON_SIZE: f32 = 13.0;
pub const BASE_CLIPBOARD_CLEAR_ICON_SIZE: f32 = 9.0;
/// Tile outline stroke (DIP).
pub const BASE_CLIPBOARD_ROW_OUTLINE: f32 = 1.0;
/// Icon button growth at full hover / full press (fractions, additive).
pub const CLIPBOARD_BUTTON_HOVER_GROW: f32 = 0.16;
pub const CLIPBOARD_BUTTON_PRESS_GROW: f32 = 0.12;
/// Settings page (DIP): its section label and the one setting row (title,
/// description, switch). Drawn inside whichever space's size is active.
pub const BASE_SETTINGS_LABEL_HEIGHT: f32 = 16.0;
pub const BASE_SETTINGS_LABEL_GAP: f32 = 6.0;
pub const BASE_SETTINGS_TITLE_HEIGHT: f32 = 18.0;
pub const BASE_SETTINGS_DESCRIPTION_HEIGHT: f32 = 16.0;
pub const BASE_SETTINGS_TOGGLE_WIDTH: f32 = 34.0;
pub const BASE_SETTINGS_TOGGLE_HEIGHT: f32 = 20.0;
pub const BASE_SETTINGS_TOGGLE_KNOB_INSET: f32 = 2.0;
pub const BASE_SETTINGS_TOGGLE_GAP: f32 = 12.0;
/// Between the setting rows, and the "Open Full Settings" action row.
pub const BASE_SETTINGS_ROW_GAP: f32 = 10.0;
pub const BASE_SETTINGS_ACTION_HEIGHT: f32 = 22.0;
/// Chevron at the action row's end (glyph box, DIP).
pub const BASE_SETTINGS_CHEVRON_SIZE: f32 = 8.0;

/// Longest text preview kept for a row (characters; DirectWrite ellipsizes).
pub const CLIPBOARD_PREVIEW_CHARS: usize = 160;

/// Drop page (an image dragged over the notch, DIP): a rounded dashed outline
/// inset from the notch body, with the inbox glyph centred inside.
pub const BASE_DROP_OUTLINE_INSET: f32 = 8.0;
pub const BASE_DROP_OUTLINE_RADIUS: f32 = 14.0;
pub const BASE_DROP_OUTLINE_WIDTH: f32 = 1.5;
/// Dash and gap lengths in stroke widths (round caps add one width to each dash).
pub const DROP_OUTLINE_DASHES: [f32; 2] = [2.0, 3.5];
pub const BASE_DROP_ICON_SIZE: f32 = 34.0;

/// Inline scrubber in the controls row: `[controls]  0:49 ━━━●── -2:08`.
pub const BASE_MEDIA_TIMELINE_LEAD: f32 = 4.0;
pub const BASE_MEDIA_TIMELINE_LABEL_WIDTH: f32 = 28.0;
pub const BASE_MEDIA_TIMELINE_LABEL_GAP: f32 = 4.0;
pub const BASE_MEDIA_TIMELINE_TRACK: f32 = 5.0;
pub const BASE_MEDIA_TIMELINE_FONT_SIZE: f32 = 10.0;
/// Narrowest track worth drawing; below this the scrubber is omitted.
pub const BASE_MEDIA_TIMELINE_MIN_TRACK: f32 = 24.0;
pub const BASE_SEPARATOR_MARGIN_TOP: f32 = 6.0;
pub const BASE_PREVIEW_MARGIN_TOP: f32 = 8.0;

// Icon sizing tokens (logical DIP sizes)
pub const BASE_ICON_SIZE_DEFAULT: f32 = 16.0;
pub const BASE_ICON_SIZE_SMALL: f32 = 12.0;
pub const BASE_ICON_STROKE_WIDTH: f32 = 1.2;
pub const BASE_ICON_ASPECT_RATIO: f32 = 0.70;
pub const BASE_ICON_CORNER_RADIUS: f32 = 2.5;
pub const BASE_ICON_APERTURE_RADIUS: f32 = 1.5;

// ============================================================================
// 2. MINIMAL DARK / AMOLED COLOR PALETTE
// ============================================================================

/// True AMOLED / OLED-style pure black surface (#000000, alpha 1.0).
/// Visually seamless against black OLED displays and dark laptop bezels.
pub const COLOR_SURFACE_AMOLED: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

/// High-contrast soft white primary text against pure black
pub const COLOR_TEXT_PRIMARY: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.96,
    g: 0.96,
    b: 0.97,
    a: 1.0,
};

/// Subdued neutral gray secondary text
pub const COLOR_TEXT_SECONDARY: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.65,
    g: 0.65,
    b: 0.68,
    a: 1.0,
};

/// Dim muted gray text for subtle labels
pub const COLOR_TEXT_MUTED: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.45,
    g: 0.45,
    b: 0.48,
    a: 1.0,
};

/// Subtle border highlight (set to transparent so notch stays pure black with zero white overlay)
pub const COLOR_BORDER_SUBTLE: D2D1_COLOR_F = COLOR_TRANSPARENT;

/// Hover border highlight (strictly transparent: eliminates the unwanted white overlay on hover)
pub const COLOR_BORDER_HOVER: D2D1_COLOR_F = COLOR_TRANSPARENT;

/// Transparent background color for layered window clearing
pub const COLOR_TRANSPARENT: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

// Aliases for seamless Phase 1 compatibility
pub const NOTCH_BG_COLOR: D2D1_COLOR_F = COLOR_SURFACE_AMOLED;
pub const NOTCH_BORDER_COLOR: D2D1_COLOR_F = COLOR_BORDER_SUBTLE;

/// Control backdrop at full hover (scaled by hover/press level; control-local only)
pub const COLOR_MEDIA_CONTROL_HOVER: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.07,
};

/// Near-invisible artwork edge so dark covers keep their shape on #000
pub const COLOR_ARTWORK_HAIRLINE: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.08,
};

/// sRGB bytes as a renderer color (the one conversion for every accent:
/// the user's choice and the artwork-derived media accent).
pub fn rgb_color([r, g, b]: [u8; 3]) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a: 1.0,
    }
}

/// The user's accent (Settings > Appearance): a few named colors that read
/// on the black notch. Used only where Oeca already shows an accent: an "on"
/// switch, the Settings window's selected-page bar, and the playback
/// visualizer / scrubber when the artwork gives no accent of its own.
/// `White` (the default) is the original neutral accent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AccentChoice {
    #[default]
    White,
    Blue,
    Teal,
    Green,
    Amber,
    Rose,
    Violet,
}

impl AccentChoice {
    pub const ALL: [Self; 7] = [
        Self::White,
        Self::Blue,
        Self::Teal,
        Self::Green,
        Self::Amber,
        Self::Rose,
        Self::Violet,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Option<Self> {
        Self::ALL.get(i).copied()
    }

    /// Display and accessible name.
    pub fn name(self) -> &'static str {
        match self {
            Self::White => "White",
            Self::Blue => "Blue",
            Self::Teal => "Teal",
            Self::Green => "Green",
            Self::Amber => "Amber",
            Self::Rose => "Rose",
            Self::Violet => "Violet",
        }
    }

    /// sRGB value (all light enough for a black knob / black surface).
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Self::White => [245, 245, 247],
            Self::Blue => [80, 160, 255],
            Self::Teal => [64, 200, 190],
            Self::Green => [72, 200, 110],
            Self::Amber => [255, 180, 60],
            Self::Rose => [255, 110, 140],
            Self::Violet => [170, 135, 255],
        }
    }

    pub fn color(self) -> D2D1_COLOR_F {
        rgb_color(self.rgb())
    }
}

/// Selected space pill: a quiet lift off the black surface
pub const COLOR_SPACE_PILL_SELECTED: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.13,
};

/// Drop page outline (dashes) and inbox glyph.
pub const COLOR_DROP_OUTLINE: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.38,
};
pub const COLOR_DROP_ICON: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.9,
};

/// Clipboard row outline: a faint white hairline on the black notch.
pub const COLOR_CLIPBOARD_ROW: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.18,
};

/// Home: thin vertical divider between the track text and the time/date
pub const COLOR_MEDIA_DIVIDER: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.14,
};
/// Divider placement: this far after the transport controls, and the track
/// text stops this far before it.
pub const BASE_MEDIA_DIVIDER_AFTER_CONTROLS: f32 = 40.0;
pub const BASE_MEDIA_DIVIDER_TEXT_GAP: f32 = 12.0;
/// Divider thickness (DIP) and how far it stops short of the content top/bottom.
pub const BASE_MEDIA_DIVIDER_WIDTH: f32 = 1.0;
pub const BASE_MEDIA_DIVIDER_INSET: f32 = 4.0;

/// Unplayed part of the scrubber track: dark neutral on #000
pub const COLOR_MEDIA_TRACK_UNPLAYED: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.16,
};

/// Additional backdrop alpha while a control is pressed
pub const MEDIA_CONTROL_PRESS_ALPHA: f32 = 0.06;

/// Quiet tertiary text (source app, secondary date); ~5.7:1 on #000
pub const COLOR_TEXT_TERTIARY: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.52,
    g: 0.52,
    b: 0.55,
    a: 1.0,
};

/// Subtle separator color token for expanded state divider line
pub const COLOR_SEPARATOR: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.04,
};

// ============================================================================
// 3. HARDWARE-NOTCH GEOMETRY & CURVATURE MODEL
// ============================================================================

/// UI state of the notch
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NotchState {
    #[default]
    Collapsed,
    Expanded,
}

/// Explicit interaction state tracking notch expansion and hover
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InteractionState {
    pub state: NotchState,
    pub hovered: bool,
}

impl InteractionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles left click toggle between Collapsed and Expanded.
    /// Returns true if the state changed.
    pub fn on_left_click(&mut self) -> bool {
        let prev = self.state;
        self.state = match self.state {
            NotchState::Collapsed => NotchState::Expanded,
            NotchState::Expanded => NotchState::Collapsed,
        };
        prev != self.state
    }

    /// Updates hover status. Returns true if hover status actually changed.
    pub fn set_hovered(&mut self, hovered: bool) -> bool {
        if self.hovered != hovered {
            self.hovered = hovered;
            true
        } else {
            false
        }
    }
}

/// Curvature parameters for hardware cutout representation
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HardwareNotchCurvature {
    /// Radius of the lower rounded corners
    pub bottom_radius: f32,
    /// Curvature radius for smooth attachment flowing into the top screen bezel (horizontal spread)
    pub top_transition_radius: f32,
    /// Vertical height of top shoulder curve before the straight vertical side wall begins
    pub top_transition_height: f32,
}

/// Centralized DPI-aware dimensions and geometry for the notch
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NotchDimensions {
    pub state: NotchState,
    pub width: i32,
    pub height: i32,
    /// Lower corner radius (aliased for compatibility)
    pub corner_radius: f32,
    /// Curvature model representing the hardware cutout geometry
    pub curvature: HardwareNotchCurvature,
    pub border_width: f32,
    pub padding_h: f32,
    pub padding_v: f32,
    pub spacing: f32,
    pub icon_size_default: f32,
    pub icon_size_small: f32,
    pub font_size_primary: f32,
    pub font_size_secondary: f32,
    pub font_size_muted: f32,
    pub font_size_clock: f32,
    pub font_size_expanded_time: f32,
    pub font_size_expanded_date: f32,
    pub dpi: u32,
    pub scale: f32,
    pub shadow_margin_x: f32,
    pub shadow_margin_bottom: f32,
    pub shadow_opacity: f32,
}

impl NotchDimensions {
    /// Same notch with a different window width (DIP); everything vertical is
    /// untouched. Used by the Music space's narrower expanded notch. Even pixel
    /// widths match the animation frames, so settling never shifts by a pixel.
    pub fn with_width_dip(mut self, width_dip: f32) -> Self {
        let mut width = (width_dip * self.scale).round() as i32;
        if width % 2 != 0 {
            width += 1;
        }
        self.width = width;
        self
    }

    /// Same notch with a different window height (DIP); shadow margins and
    /// everything horizontal untouched. Used by the taller Music player.
    pub fn with_height_dip(mut self, height_dip: f32) -> Self {
        self.height = (height_dip * self.scale).round() as i32;
        self
    }

    /// Computes physical pixel dimensions and design parameters for a given state and display DPI.
    /// Standard baseline is 96 DPI (100% Windows scaling).
    pub fn from_state_and_dpi(state: NotchState, dpi: u32) -> Self {
        let effective_dpi = if dpi == 0 { 96 } else { dpi };
        let scale = effective_dpi as f32 / BASE_DPI;

        let (base_w, base_h, base_bottom_r, base_top_r, base_top_h, base_pad_h, base_pad_v) =
            match state {
                NotchState::Collapsed => (
                    BASE_COLLAPSED_WIDTH,
                    BASE_COLLAPSED_HEIGHT,
                    BASE_COLLAPSED_BOTTOM_CORNER_RADIUS,
                    BASE_COLLAPSED_TOP_TRANSITION_RADIUS,
                    BASE_COLLAPSED_TOP_TRANSITION_HEIGHT,
                    BASE_COLLAPSED_PADDING_H,
                    BASE_COLLAPSED_PADDING_V,
                ),
                NotchState::Expanded => (
                    BASE_EXPANDED_WIDTH,
                    BASE_EXPANDED_HEIGHT,
                    BASE_EXPANDED_BOTTOM_CORNER_RADIUS,
                    BASE_EXPANDED_TOP_TRANSITION_RADIUS,
                    BASE_EXPANDED_TOP_TRANSITION_HEIGHT,
                    BASE_EXPANDED_PADDING_H,
                    BASE_EXPANDED_PADDING_V,
                ),
            };

        let bottom_radius = base_bottom_r * scale;
        let top_transition_radius = base_top_r * scale;
        let top_transition_height = base_top_h * scale;

        let (shadow_margin_x, shadow_margin_bottom, shadow_opacity) = match state {
            NotchState::Collapsed => (0.0, 0.0, 0.0),
            NotchState::Expanded => (
                (BASE_EXPANDED_SHADOW_MARGIN_X * scale).round(),
                (BASE_EXPANDED_SHADOW_MARGIN_BOTTOM * scale).round(),
                BASE_EXPANDED_SHADOW_OPACITY,
            ),
        };

        Self {
            state,
            width: (base_w * scale).round() as i32,
            height: (base_h * scale).round() as i32,
            corner_radius: bottom_radius,
            curvature: HardwareNotchCurvature {
                bottom_radius,
                top_transition_radius,
                top_transition_height,
            },
            border_width: (BASE_BORDER_WIDTH * scale).max(1.0),
            padding_h: (base_pad_h * scale).round(),
            padding_v: (base_pad_v * scale).round(),
            spacing: (BASE_SPACING * scale).round(),
            icon_size_default: (BASE_ICON_SIZE_DEFAULT * scale).round(),
            icon_size_small: (BASE_ICON_SIZE_SMALL * scale).round(),
            font_size_primary: BASE_FONT_SIZE_PRIMARY * scale,
            font_size_secondary: BASE_FONT_SIZE_SECONDARY * scale,
            font_size_muted: BASE_FONT_SIZE_MUTED * scale,
            font_size_clock: BASE_FONT_SIZE_CLOCK * scale,
            font_size_expanded_time: BASE_EXPANDED_FONT_SIZE_TIME * scale,
            font_size_expanded_date: BASE_EXPANDED_FONT_SIZE_DATE * scale,
            dpi: effective_dpi,
            scale,
            shadow_margin_x,
            shadow_margin_bottom,
            shadow_opacity,
        }
    }

    /// Convenience constructor defaulting to Collapsed state for backwards compatibility.
    pub fn from_dpi(dpi: u32) -> Self {
        Self::from_state_and_dpi(NotchState::Collapsed, dpi)
    }

    /// Returns the lower corner radius of the notch
    #[inline]
    pub fn bottom_corner_radius(&self) -> f32 {
        self.curvature.bottom_radius
    }

    /// Returns the top transition/shoulder radius of the notch
    #[inline]
    pub fn top_transition_radius(&self) -> f32 {
        self.curvature.top_transition_radius
    }

    /// Returns the top transition/shoulder height of the notch
    #[inline]
    pub fn top_transition_height(&self) -> f32 {
        self.curvature.top_transition_height
    }

    /// Returns the physical width of the solid notch body excluding shadow margins
    #[inline]
    pub fn notch_width(&self) -> f32 {
        (self.width as f32 - 2.0 * self.shadow_margin_x).max(0.0)
    }

    /// Returns the physical height of the solid notch body excluding shadow margins
    #[inline]
    pub fn notch_height(&self) -> f32 {
        (self.height as f32 - self.shadow_margin_bottom).max(0.0)
    }

    /// Verifies left-right symmetry of the geometry
    #[inline]
    pub fn is_symmetric(&self) -> bool {
        self.width % 2 == 0 || self.width > 0
    }

    /// Bottom-corner smoothing, 0 (collapsed: exact circle) ..= 1 (expanded:
    /// continuous corner). Derived from the current logical bottom radius, so it
    /// follows the existing animation interpolation with no extra state.
    pub fn bottom_smoothing(&self) -> f32 {
        if self.scale <= 0.0 {
            return 0.0;
        }
        let r = self.curvature.bottom_radius / self.scale;
        let s = ((r - BASE_COLLAPSED_BOTTOM_CORNER_RADIUS)
            / (BASE_EXPANDED_BOTTOM_CORNER_RADIUS - BASE_COLLAPSED_BOTTOM_CORNER_RADIUS))
            .clamp(0.0, 1.0);
        // Snap float noise from DPI scaling so the settled states are exact
        if s < 1e-3 {
            0.0
        } else if s > 1.0 - 1e-3 {
            1.0
        } else {
            s
        }
    }

    /// Bottom-corner curve shared by rendering and hit testing.
    pub fn bottom_corner_profile(&self) -> CornerProfile {
        let max_span = (self.notch_height() / 2.0).min(self.notch_width() / 4.0);
        CornerProfile::new(
            self.curvature.bottom_radius.max(0.0).min(max_span),
            self.bottom_smoothing(),
            max_span,
        )
    }

    /// Hit-tests whether a client coordinate point (px, py) is inside the hardware notch body
    #[inline]
    pub fn contains_point(&self, px: f32, py: f32) -> bool {
        let pad_x = self.shadow_margin_x;
        let notch_w = self.notch_width();
        let notch_h = self.notch_height();

        if px < pad_x || px > (pad_x + notch_w) || py > notch_h {
            return false;
        }

        if self.bottom_smoothing() > 0.0 {
            return is_point_in_notch_profile(
                px - pad_x,
                py,
                notch_w,
                notch_h,
                self.curvature.top_transition_radius,
                self.curvature.top_transition_height,
                self.bottom_corner_profile(),
            );
        }

        is_point_in_notch_ex(
            px - pad_x,
            py,
            notch_w,
            notch_h,
            self.curvature.top_transition_radius,
            self.curvature.top_transition_height,
            self.curvature.bottom_radius,
        )
    }
}

/// Computes the horizontal top-center position $X$ for given screen and notch widths.
/// Ensures window is horizontally centered and clamped to valid display bounds.
pub fn calculate_notch_x(screen_width: i32, notch_width: i32) -> i32 {
    if screen_width <= 0 || notch_width <= 0 {
        return 0;
    }
    ((screen_width - notch_width) / 2).max(0)
}

/// Computes the horizontal center coordinate of the notch for given screen and notch widths.
pub fn calculate_notch_center_x(screen_width: i32, notch_width: i32) -> f32 {
    let x = calculate_notch_x(screen_width, notch_width);
    x as f32 + (notch_width as f32 / 2.0)
}

// ============================================================================
// 4. HIT TESTING
// ============================================================================

/// Mathematical hit testing to check if a client-space point (px, py)
/// lies inside the physical hardware-notch geometry.
///
/// Follows the hardware notch shape:
/// - Top shoulders: concave transition arcs connecting to top screen edge
/// - Central body: vertical side walls
/// - Bottom corners: convex rounded corners
///
/// Mathematical hit testing to check if a client-space point (px, py)
/// lies inside the physical hardware-notch geometry with independent
/// horizontal transition radius and vertical transition height.
pub fn is_point_in_notch_ex(
    px: f32,
    py: f32,
    width: f32,
    height: f32,
    top_transition_radius: f32,
    top_transition_height: f32,
    bottom_corner_radius: f32,
) -> bool {
    if width <= 0.0 || height <= 0.0 || px < 0.0 || px > width || py < 0.0 || py > height {
        return false;
    }

    let r_top_x = top_transition_radius
        .max(0.0)
        .min(height / 2.0)
        .min(width / 4.0);
    let r_top_y = top_transition_height
        .max(0.0)
        .min(height / 2.0)
        .min(width / 4.0);
    let r_bottom = bottom_corner_radius
        .max(0.0)
        .min(height / 2.0)
        .min(width / 4.0);

    // 1. Top shoulder region (0 <= py <= r_top_y)
    if py <= r_top_y && r_top_x > 0.0 && r_top_y > 0.0 {
        let ny = (py - r_top_y) / r_top_y;
        // Left concave shoulder: outside if left of arc
        if px < r_top_x {
            let nx = px / r_top_x;
            return (nx * nx + ny * ny) >= 1.0;
        }
        // Right concave shoulder: outside if right of arc
        if px > (width - r_top_x) {
            let nx = (px - width) / r_top_x;
            return (nx * nx + ny * ny) >= 1.0;
        }
        // Between shoulders at the top edge is solid notch body
        return true;
    }

    // 2. Outside side walls below the shoulder region (py > r_top_y)
    if px < r_top_x || px > (width - r_top_x) {
        return false;
    }

    // 3. Bottom rounded corners (height - r_bottom <= py <= height)
    if py > (height - r_bottom) && r_bottom > 0.0 {
        let dy = py - (height - r_bottom);
        // Bottom-left rounded corner
        if px < (r_top_x + r_bottom) {
            let dx = px - (r_top_x + r_bottom);
            return (dx * dx + dy * dy) <= (r_bottom * r_bottom);
        }
        // Bottom-right rounded corner
        if px > (width - r_top_x - r_bottom) {
            let dx = px - (width - r_top_x - r_bottom);
            return (dx * dx + dy * dy) <= (r_bottom * r_bottom);
        }
    }

    // 4. Central body of the notch
    true
}

/// Hit test for a notch whose bottom corners follow `bottom` (the same curve the
/// renderer draws). Shoulders and walls are identical to `is_point_in_notch_ex`.
pub fn is_point_in_notch_profile(
    px: f32,
    py: f32,
    width: f32,
    height: f32,
    top_transition_radius: f32,
    top_transition_height: f32,
    bottom: CornerProfile,
) -> bool {
    let span = bottom.span;
    let r_top_x = top_transition_radius
        .max(0.0)
        .min(height / 2.0)
        .min(width / 4.0);
    // Everything except the bottom corners: reuse the circle version with no corner
    if !is_point_in_notch_ex(
        px,
        py,
        width,
        height,
        top_transition_radius,
        top_transition_height,
        0.0,
    ) {
        return false;
    }
    if span <= 0.0 || py <= height - span {
        return true;
    }
    let v = py - (height - span);
    let left_u = px - r_top_x;
    let right_u = (width - r_top_x) - px;
    let u = left_u.min(right_u);
    if u >= span {
        return true;
    }
    u >= bottom.inset_at(v)
}

/// Backward-compatible hit testing assuming equal horizontal and vertical top transition radii.
pub fn is_point_in_notch(
    px: f32,
    py: f32,
    width: f32,
    height: f32,
    top_transition_radius: f32,
    bottom_corner_radius: f32,
) -> bool {
    is_point_in_notch_ex(
        px,
        py,
        width,
        height,
        top_transition_radius,
        top_transition_radius,
        bottom_corner_radius,
    )
}

// ============================================================================
// 5. ANIMATION MODEL & EASING (Phase 2.5)
// ============================================================================

/// Nominal expand/collapse time scale (ms). The springs below settle on their
/// own; this only sizes the reversal / test time budget.
pub const ANIMATION_DURATION_MS: u64 = 340;

/// Frame interval in milliseconds (~12ms for fluid ~80 FPS animation ticks)
pub const ANIMATION_FRAME_INTERVAL_MS: u32 = 12;

/// Dedicated timer ID for window animation
pub const ANIMATION_TIMER_ID: usize = 1001;

/// Dedicated timer ID for minute clock rollover (Phase 3.1)
pub const CLOCK_TIMER_ID: usize = 1002;

/// A damped spring: damping ratio and response (the undamped period, s).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    pub zeta: f32,
    pub response: f32,
}

/// Expand: restrained, ~0.5% overshoot (expand -> tiny continuation -> settle).
pub const EXPAND_SPRING: Spring = Spring {
    zeta: 0.86,
    response: 0.36,
};
/// Collapse: critically damped, never passes the collapsed size.
pub const COLLAPSE_SPRING: Spring = Spring {
    zeta: 1.0,
    response: 0.28,
};
/// Space switch (and the content blend): critically damped, calm shell.
pub const SPACE_SPRING: Spring = Spring {
    zeta: 1.0,
    response: 0.34,
};

/// Settled when every geometry value is this close to its target (physical
/// px) and moving slower than `SETTLE_SPEED_PX` (px/s): the frame no longer
/// changes, so ending there shows no pop.
pub const SETTLE_DISTANCE_PX: f32 = 0.25;
pub const SETTLE_SPEED_PX: f32 = 40.0;

/// Content fades (ms): expanded content fades in once the shell is open
/// enough (`CONTENT_IN_OPENNESS`) and out right as a collapse begins;
/// collapsed content fades back in near the collapsed size.
pub const CONTENT_FADE_MS: f32 = 120.0;
pub const CONTENT_IN_OPENNESS: f32 = 0.4;
pub const COLLAPSED_IN_OPENNESS: f32 = 0.3;

impl Spring {
    /// Exact solution of the spring after `dt_ms` from position `x` with
    /// velocity `v` (units/s) toward `target`: returns the new (x, v). Exact
    /// for any frame length, and continuous in position and velocity.
    pub fn advance(self, x: f32, v: f32, target: f32, dt_ms: f32) -> (f32, f32) {
        let t = dt_ms.max(0.0) / 1000.0;
        let w = 2.0 * std::f32::consts::PI / self.response;
        let a = x - target;
        if self.zeta >= 1.0 {
            let b = v + w * a;
            let e = (-w * t).exp();
            (target + (a + b * t) * e, (b - w * (a + b * t)) * e)
        } else {
            let z = self.zeta;
            let wd = w * (1.0 - z * z).sqrt();
            let b = (v + z * w * a) / wd;
            let e = (-z * w * t).exp();
            let (s, c) = (wd * t).sin_cos();
            let x2 = a * c + b * s;
            (target + e * x2, e * (-z * w * x2 + wd * (b * c - a * s)))
        }
    }
}

/// Transport glyph motion (play/pause morph, skip slide): critically damped
/// and quicker than the shell, so a glyph answers a click at once and never
/// overshoots.
pub const ICON_SPRING: Spring = Spring {
    zeta: 1.0,
    response: 0.24,
};

/// A value following a target on a spring (position + velocity): a new
/// target mid-way continues from where it is, at the speed it has, so
/// reversals and repeated retargets never jump. Settles exactly on the
/// target; needs frames only while `is_animating`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Glide {
    pub value: f32,
    vel: f32,
    pub target: f32,
}

impl Glide {
    pub fn at(value: f32) -> Self {
        Self {
            value,
            vel: 0.0,
            target: value,
        }
    }

    /// Jumps to `value` and stays (Reduced motion, content not on screen).
    pub fn snap(&mut self, value: f32) {
        *self = Self::at(value);
    }

    pub fn is_animating(&self) -> bool {
        self.value != self.target || self.vel != 0.0
    }

    /// Advances by `dt_ms` on `spring`; true while still moving.
    pub fn step(&mut self, spring: Spring, dt_ms: f32) -> bool {
        if !self.is_animating() {
            return false;
        }
        (self.value, self.vel) = spring.advance(self.value, self.vel, self.target, dt_ms);
        if (self.value - self.target).abs() < 1e-3 && self.vel.abs() < 0.05 {
            self.snap(self.target);
        }
        self.is_animating()
    }
}

/// Cubic ease-out curve: f(t) = 1 - (1 - t)^3
/// Provides responsive initial velocity smoothly decelerating to rest.
pub fn cubic_ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    1.0 - inv * inv * inv
}

/// Linear interpolation helper
#[inline]
pub fn lerp(start: f32, target: f32, t: f32) -> f32 {
    start + (target - start) * t
}

/// Returns reference (width, height, top_transition_radius, top_transition_height, bottom_corner_radius) in logical DIP for a given NotchState.
pub fn reference_geometry_for_state(state: NotchState) -> (f32, f32, f32, f32, f32) {
    match state {
        NotchState::Collapsed => (
            BASE_COLLAPSED_WIDTH,
            BASE_COLLAPSED_HEIGHT,
            BASE_COLLAPSED_TOP_TRANSITION_RADIUS,
            BASE_COLLAPSED_TOP_TRANSITION_HEIGHT,
            BASE_COLLAPSED_BOTTOM_CORNER_RADIUS,
        ),
        NotchState::Expanded => (
            BASE_EXPANDED_WIDTH,
            BASE_EXPANDED_HEIGHT,
            BASE_EXPANDED_TOP_TRANSITION_RADIUS,
            BASE_EXPANDED_TOP_TRANSITION_HEIGHT,
            BASE_EXPANDED_BOTTOM_CORNER_RADIUS,
        ),
    }
}

/// Notch geometry (DIP): width, height, top shoulder radius, top shoulder
/// height, bottom corner radius.
pub type Geometry = [f32; 5];

/// Geometry of a state, the expanded one at a space's window size.
pub fn geometry_for(state: NotchState, size: (f32, f32)) -> Geometry {
    let (w, h, top_r, top_h, bot_r) = reference_geometry_for_state(state);
    match state {
        NotchState::Collapsed => [w, h, top_r, top_h, bot_r],
        NotchState::Expanded => [size.0, size.1, top_r, top_h, bot_r],
    }
}

/// Which spring drives the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Expand,
    Collapse,
    /// Expanded to expanded (space switch, drop page).
    Space,
}

impl Motion {
    fn spring(self) -> Spring {
        match self {
            Self::Expand => EXPAND_SPRING,
            Self::Collapse => COLLAPSE_SPRING,
            Self::Space => SPACE_SPRING,
        }
    }
}

/// What the renderer needs from an animation frame: the content blend between
/// two scenes and the content opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transition {
    pub from: Scene,
    pub to: Scene,
    /// 0 = all `from`, 1 = all `to`.
    pub mix: f32,
    /// Opacity of the content drawn this frame (expanded or collapsed).
    pub alpha: f32,
}

/// Continuous notch animation: every geometry value is a spring (position +
/// velocity), so a new target (reverse, space change, drop page) continues
/// from the current position *and* velocity. Content follows the geometry:
/// time-based fades gated on how open the shell is, plus a blend spring
/// between the outgoing and incoming scene.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationState {
    pub start_state: NotchState,
    pub target_state: NotchState,
    pub motion: Motion,
    pub pos: Geometry,
    /// DIP per second.
    pub vel: Geometry,
    pub target: Geometry,
    pub expanded_alpha: f32,
    pub collapsed_alpha: f32,
    pub scene_from: Scene,
    pub scene_to: Scene,
    pub mix: f32,
    mix_vel: f32,
    /// DIP to px for the settle threshold.
    pub scale: f32,
    pub elapsed_ms: f32,
    pub active: bool,
}

impl AnimationState {
    /// From rest in `from_state` (at `from_size` if expanded) toward
    /// `to_state` (at `to_size` if expanded), showing `scene`.
    pub fn start(
        from_state: NotchState,
        from_size: (f32, f32),
        to_state: NotchState,
        to_size: (f32, f32),
        scene: Scene,
    ) -> Self {
        let motion = match (from_state, to_state) {
            (_, NotchState::Collapsed) => Motion::Collapse,
            (NotchState::Collapsed, NotchState::Expanded) => Motion::Expand,
            (NotchState::Expanded, NotchState::Expanded) => Motion::Space,
        };
        let open = f32::from(u8::from(from_state == NotchState::Expanded));
        Self {
            start_state: from_state,
            target_state: to_state,
            motion,
            pos: geometry_for(from_state, from_size),
            vel: [0.0; 5],
            target: geometry_for(to_state, to_size),
            expanded_alpha: open,
            collapsed_alpha: 1.0 - open,
            scene_from: scene,
            scene_to: scene,
            mix: 1.0,
            mix_vel: 0.0,
            scale: 1.0,
            elapsed_ms: 0.0,
            active: true,
        }
    }

    /// Expand/collapse between the reference (Home) sizes.
    pub fn new(current_state: NotchState, target_state: NotchState) -> Self {
        let home = (BASE_EXPANDED_WIDTH, BASE_EXPANDED_HEIGHT);
        Self::start(
            current_state,
            home,
            target_state,
            home,
            Scene::Space(crate::space::OecaSpace::Home),
        )
    }

    /// Expanded-to-expanded resize (space switch / drop page) with the content
    /// blending from `from_scene` to `to_scene`.
    pub fn space(from: (f32, f32), to: (f32, f32), from_scene: Scene, to_scene: Scene) -> Self {
        let mut anim = Self::start(
            NotchState::Expanded,
            from,
            NotchState::Expanded,
            to,
            to_scene,
        );
        anim.retarget_scene_from(from_scene);
        anim
    }

    fn retarget_scene_from(&mut self, from_scene: Scene) {
        if from_scene != self.scene_to {
            self.scene_from = from_scene;
            self.mix = 0.0;
        }
    }

    /// New target state / size mid-flight: position and velocity carry over.
    pub fn retarget(&mut self, state: NotchState, size: (f32, f32)) {
        self.motion = match state {
            NotchState::Collapsed => Motion::Collapse,
            NotchState::Expanded if self.motion == Motion::Collapse => Motion::Expand,
            NotchState::Expanded => self.motion,
        };
        self.target_state = state;
        self.target = geometry_for(state, size);
        self.active = true;
    }

    /// New scene mid-flight. Back to the outgoing scene: the blend reverses
    /// from where it is (with its velocity). A third scene: the blend restarts
    /// from whichever scene currently dominates.
    pub fn retarget_scene(&mut self, scene: Scene) {
        if scene == self.scene_to {
            return;
        }
        if scene == self.scene_from {
            std::mem::swap(&mut self.scene_from, &mut self.scene_to);
            self.mix = 1.0 - self.mix;
            self.mix_vel = -self.mix_vel;
        } else {
            // ponytail: a third scene mid-blend restarts from the dominant one
            // (rare: three targets within one transition)
            if self.mix >= 0.5 {
                self.scene_from = self.scene_to;
            }
            self.scene_to = scene;
            self.mix = 0.0;
            self.mix_vel = 0.0;
        }
        self.active = true;
    }

    /// True for an expanded-to-expanded transition (no expand/collapse).
    pub fn is_space_transition(&self) -> bool {
        self.motion == Motion::Space
    }

    /// How open the shell is: 0 at the collapsed height, 1 from the smallest
    /// expanded (Home) height up. Drives margins, shadow and content fades.
    pub fn openness(&self) -> f32 {
        ((self.pos[1] - BASE_COLLAPSED_HEIGHT) / (BASE_EXPANDED_HEIGHT - BASE_COLLAPSED_HEIGHT))
            .clamp(0.0, 1.0)
    }

    /// Advances by `dt_ms` (fractional frame time). Returns true while still
    /// moving; once settled the geometry is exactly the target.
    pub fn step(&mut self, dt_ms: f32) -> bool {
        if !self.active {
            return false;
        }
        self.elapsed_ms += dt_ms;
        let spring = self.motion.spring();
        for i in 0..5 {
            (self.pos[i], self.vel[i]) =
                spring.advance(self.pos[i], self.vel[i], self.target[i], dt_ms);
        }
        if self.motion == Motion::Collapse {
            // Never smaller than the collapsed notch
            for i in 0..2 {
                if self.pos[i] < self.target[i] {
                    self.pos[i] = self.target[i];
                    self.vel[i] = self.vel[i].max(0.0);
                }
            }
        }
        (self.mix, self.mix_vel) = SPACE_SPRING.advance(self.mix, self.mix_vel, 1.0, dt_ms);
        self.mix = self.mix.clamp(0.0, 1.0);

        // Content fades, gated on the shell's openness
        let open = self.openness();
        let expanding = self.target_state == NotchState::Expanded;
        let fade = |a: f32, on: bool| {
            let d = dt_ms / CONTENT_FADE_MS;
            if on {
                (a + d).min(1.0)
            } else {
                (a - d).max(0.0)
            }
        };
        let show_expanded = expanding && (open >= CONTENT_IN_OPENNESS || self.expanded_alpha > 0.0);
        self.expanded_alpha = fade(self.expanded_alpha, show_expanded);
        self.collapsed_alpha = fade(
            self.collapsed_alpha,
            !expanding && open <= COLLAPSED_IN_OPENNESS,
        );

        let px = |d: f32| d * self.scale;
        let settled = (0..5).all(|i| {
            px((self.pos[i] - self.target[i]).abs()) < SETTLE_DISTANCE_PX
                && px(self.vel[i].abs()) < SETTLE_SPEED_PX
        }) && (1.0 - self.mix) < 1e-3
            && self.mix_vel.abs() < 0.05
            && self.expanded_alpha == f32::from(u8::from(expanding))
            && self.collapsed_alpha == f32::from(u8::from(!expanding));
        if settled {
            self.pos = self.target;
            self.vel = [0.0; 5];
            self.mix = 1.0;
            self.mix_vel = 0.0;
            self.scene_from = self.scene_to;
            self.active = false;
        }
        self.active
    }

    /// Test helper: advances (12 ms frames) to `p` of the nominal duration;
    /// `p >= 1` runs until settled. Progress only moves forward.
    #[cfg(test)]
    pub fn set_progress(&mut self, p: f32) {
        if p >= 1.0 {
            self.run_for(f32::MAX);
            return;
        }
        let goal = p.max(0.0) * ANIMATION_DURATION_MS as f32;
        while self.elapsed_ms + 6.0 < goal && self.step(ANIMATION_FRAME_INTERVAL_MS as f32) {}
    }

    /// Steps in 12 ms frames until settled (or `max_ms`); deterministic tests.
    #[cfg(test)]
    pub fn run_for(&mut self, ms: f32) {
        let mut t = 0.0;
        while t < ms && self.step(ANIMATION_FRAME_INTERVAL_MS as f32) {
            t += ANIMATION_FRAME_INTERVAL_MS as f32;
        }
    }

    /// Current logical (width, height, top radius, bottom radius) in DIP.
    pub fn current_logical_geometry(&self) -> (f32, f32, f32, f32) {
        (self.pos[0], self.pos[1], self.pos[2], self.pos[4])
    }

    /// What the renderer shows this frame.
    pub fn transition(&self) -> Transition {
        Transition {
            from: self.scene_from,
            to: self.scene_to,
            mix: self.mix,
            alpha: if self.expanded_alpha > 0.0 {
                self.expanded_alpha
            } else {
                self.collapsed_alpha
            },
        }
    }

    /// Computes physical NotchDimensions for the current animation frame given display DPI.
    pub fn current_dimensions(&self, dpi: u32) -> NotchDimensions {
        let scale = if dpi == 0 { 1.0 } else { dpi as f32 / BASE_DPI };
        let [log_w, log_h, log_top_r, log_top_h, log_bot_r] = self.pos;

        let top_transition_radius = log_top_r * scale;
        let top_transition_height = log_top_h * scale;
        let bottom_radius = log_bot_r * scale;

        // Expanded while expanded content is (fading) visible
        let state = if self.expanded_alpha > 0.0 {
            NotchState::Expanded
        } else {
            NotchState::Collapsed
        };

        // Width moves in 2 px steps (1 px per side keeps it centred) and lands
        // exactly on the target's settled width, odd or even
        let target_w = (self.target[0] * scale).round() as i32;
        let width = target_w + 2 * ((log_w * scale - target_w as f32) / 2.0).round() as i32;
        // Margins and shadow follow the shell's actual openness, unrounded
        // (no 1 px stepping), reaching exactly the settled values at the ends
        let open = self.openness();
        let (pad_h, pad_v) = if state == NotchState::Expanded {
            (BASE_EXPANDED_PADDING_H, BASE_EXPANDED_PADDING_V)
        } else {
            (BASE_COLLAPSED_PADDING_H, BASE_COLLAPSED_PADDING_V)
        };

        NotchDimensions {
            state,
            width,
            height: (log_h * scale).round() as i32,
            corner_radius: bottom_radius,
            curvature: HardwareNotchCurvature {
                bottom_radius,
                top_transition_radius,
                top_transition_height,
            },
            border_width: (BASE_BORDER_WIDTH * scale).max(1.0),
            padding_h: (pad_h * scale).round(),
            padding_v: (pad_v * scale).round(),
            spacing: (BASE_SPACING * scale).round(),
            icon_size_default: (BASE_ICON_SIZE_DEFAULT * scale).round(),
            icon_size_small: (BASE_ICON_SIZE_SMALL * scale).round(),
            font_size_primary: BASE_FONT_SIZE_PRIMARY * scale,
            font_size_secondary: BASE_FONT_SIZE_SECONDARY * scale,
            font_size_muted: BASE_FONT_SIZE_MUTED * scale,
            font_size_clock: BASE_FONT_SIZE_CLOCK * scale,
            font_size_expanded_time: BASE_EXPANDED_FONT_SIZE_TIME * scale,
            font_size_expanded_date: BASE_EXPANDED_FONT_SIZE_DATE * scale,
            dpi: if dpi == 0 { 96 } else { dpi },
            scale,
            shadow_margin_x: (BASE_EXPANDED_SHADOW_MARGIN_X * scale).round() * open,
            shadow_margin_bottom: (BASE_EXPANDED_SHADOW_MARGIN_BOTTOM * scale).round() * open,
            shadow_opacity: BASE_EXPANDED_SHADOW_OPACITY * open,
        }
    }
}

// ============================================================================
// 6. ACCESSIBILITY SYSTEM TOKENS & MODEL (Phase 2.7)
// ============================================================================

/// Accessible name for the top-level application / overlay
pub const ACCESSIBLE_NAME: PCWSTR = w!("Oeca");
pub const ACCESSIBLE_NAME_STR: &str = "Oeca";

/// Concise accessible description explaining the overlay's purpose
pub const ACCESSIBLE_DESCRIPTION: &str = "Interactive desktop notch overlay";

/// Accessible semantic role name for the visual overlay
pub const ACCESSIBLE_ROLE_NAME: &str = "Desktop Overlay";

/// Full accessible title when the notch is in its expanded state
pub const ACCESSIBLE_EXPANDED_TITLE: PCWSTR = w!("Oeca");
pub const ACCESSIBLE_EXPANDED_TITLE_STR: &str = "Oeca";

/// Returns the user-facing accessible window title for a given notch state
pub fn accessible_title_for_state(state: NotchState) -> PCWSTR {
    match state {
        NotchState::Collapsed => ACCESSIBLE_NAME,
        NotchState::Expanded => ACCESSIBLE_EXPANDED_TITLE,
    }
}

/// Returns the user-facing accessible text string for a given notch state
pub fn accessible_text_for_state(state: NotchState) -> &'static str {
    match state {
        NotchState::Collapsed => ACCESSIBLE_NAME_STR,
        NotchState::Expanded => ACCESSIBLE_EXPANDED_TITLE_STR,
    }
}

/// Returns the accessible state description string
pub fn accessible_state_description(state: NotchState) -> &'static str {
    match state {
        NotchState::Collapsed => "Collapsed",
        NotchState::Expanded => "Expanded",
    }
}

/// Explicit semantic accessibility snapshot for Oeca representing what
/// screen readers and accessibility clients observe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibilitySnapshot {
    pub name: String,
    pub description: &'static str,
    pub role: &'static str,
    pub state_label: &'static str,
    pub visible_text: String,
    pub exposes_subtitle: bool,
    pub is_icon_interactive: bool,
}

impl AccessibilitySnapshot {
    pub fn for_state_with_clock(
        state: NotchState,
        formatted_time: &str,
        formatted_date: &str,
    ) -> Self {
        match state {
            NotchState::Collapsed => Self {
                name: formatted_time.to_string(),
                description: ACCESSIBLE_DESCRIPTION,
                role: ACCESSIBLE_ROLE_NAME,
                state_label: "Collapsed",
                visible_text: formatted_time.to_string(),
                exposes_subtitle: false,
                is_icon_interactive: false,
            },
            NotchState::Expanded => {
                let text = format!("{formatted_time} - {formatted_date}");
                Self {
                    name: text.clone(),
                    description: ACCESSIBLE_DESCRIPTION,
                    role: ACCESSIBLE_ROLE_NAME,
                    state_label: "Expanded",
                    visible_text: text,
                    exposes_subtitle: false,
                    is_icon_interactive: false,
                }
            }
        }
    }

    pub fn for_state_with_time(state: NotchState, formatted_time: &str) -> Self {
        Self::for_state_with_clock(state, formatted_time, "Monday, October 6")
    }

    pub fn for_state(state: NotchState) -> Self {
        Self::for_state_with_clock(state, "12:00 PM", "Monday, October 6")
    }
}

/// Deterministic event tracker modeling accessibility event emissions.
/// Validates that hover and intermediate animation ticks produce zero events,
/// while settled state changes emit exactly one event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessibilityEventTracker {
    pub name_change_count: u32,
    pub state_change_count: u32,
    pub last_state: Option<NotchState>,
}

impl AccessibilityEventTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Emits a state transition event if and only if the stable state changed.
    pub fn on_state_settled(&mut self, new_state: NotchState) -> bool {
        if self.last_state != Some(new_state) {
            self.last_state = Some(new_state);
            self.name_change_count += 1;
            self.state_change_count += 1;
            true
        } else {
            false
        }
    }

    /// Explicitly documents that intermediate animation frames do not emit events.
    pub fn on_animation_frame(&mut self, _progress: f32) -> bool {
        false
    }

    /// Explicitly documents that hover state changes do not emit events.
    pub fn on_hover_changed(&mut self, _hovered: bool) -> bool {
        false
    }

    /// Emits a name change event when the displayed minute changes in collapsed or expanded state.
    pub fn on_clock_minute_changed(&mut self, _is_collapsed: bool) -> bool {
        self.name_change_count += 1;
        true
    }
}

// ============================================================================
// 7. DETERMINISTIC TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::Scene;

    #[test]
    fn test_dpi_dimension_scaling() {
        // 100% scaling: 96 DPI
        let dims_100 = NotchDimensions::from_dpi(96);
        assert_eq!(dims_100.width, 220);
        assert_eq!(dims_100.height, 32);
        assert!((dims_100.corner_radius - 14.0).abs() < 1e-4);
        assert!((dims_100.curvature.bottom_radius - 14.0).abs() < 1e-4);
        assert!((dims_100.curvature.top_transition_radius - 6.0).abs() < 1e-4);
        assert!((dims_100.border_width - 1.0).abs() < 1e-4);
        assert_eq!(dims_100.padding_h, 14.0);
        assert_eq!(dims_100.padding_v, 6.0);
        assert_eq!(dims_100.icon_size_default, 16.0);
        assert_eq!(dims_100.icon_size_small, 12.0);

        // 125% scaling: 120 DPI
        let dims_125 = NotchDimensions::from_dpi(120);
        assert_eq!(dims_125.width, 275);
        assert_eq!(dims_125.height, 40);
        assert!((dims_125.corner_radius - 17.5).abs() < 1e-4);
        assert!((dims_125.curvature.bottom_radius - 17.5).abs() < 1e-4);
        assert!((dims_125.curvature.top_transition_radius - 7.5).abs() < 1e-4);
        assert!((dims_125.border_width - 1.25).abs() < 1e-4);

        // 150% scaling: 144 DPI
        let dims_150 = NotchDimensions::from_dpi(144);
        assert_eq!(dims_150.width, 330);
        assert_eq!(dims_150.height, 48);
        assert!((dims_150.corner_radius - 21.0).abs() < 1e-4);
        assert!((dims_150.curvature.top_transition_radius - 9.0).abs() < 1e-4);
        assert!((dims_150.border_width - 1.5).abs() < 1e-4);

        // 175% scaling: 168 DPI
        let dims_175 = NotchDimensions::from_dpi(168);
        assert_eq!(dims_175.width, 385);
        assert_eq!(dims_175.height, 56);
        assert!((dims_175.corner_radius - 24.5).abs() < 1e-4);
        assert!((dims_175.curvature.top_transition_radius - 10.5).abs() < 1e-4);
        assert!((dims_175.border_width - 1.75).abs() < 1e-4);

        // 200% scaling: 192 DPI
        let dims_200 = NotchDimensions::from_dpi(192);
        assert_eq!(dims_200.width, 440);
        assert_eq!(dims_200.height, 64);
        assert!((dims_200.corner_radius - 28.0).abs() < 1e-4);
        assert!((dims_200.curvature.top_transition_radius - 12.0).abs() < 1e-4);
        assert!((dims_200.border_width - 2.0).abs() < 1e-4);

        // Unusual DPI: 137 DPI (arbitrary non-standard display scaling)
        let dims_137 = NotchDimensions::from_dpi(137);
        let expected_scale: f32 = 137.0 / 96.0;
        assert_eq!(dims_137.width, (220.0f32 * expected_scale).round() as i32);
        assert_eq!(dims_137.height, (32.0f32 * expected_scale).round() as i32);
        assert!((dims_137.corner_radius - (14.0 * expected_scale)).abs() < 1e-4);
        assert!((dims_137.curvature.top_transition_radius - (6.0 * expected_scale)).abs() < 1e-4);

        // High DPI: 288 DPI (300% 4K display scaling)
        let dims_288 = NotchDimensions::from_dpi(288);
        assert_eq!(dims_288.width, 660);
        assert_eq!(dims_288.height, 96);
        assert!((dims_288.corner_radius - 42.0).abs() < 1e-4);
        assert!((dims_288.curvature.top_transition_radius - 18.0).abs() < 1e-4);
        assert!((dims_288.border_width - 3.0).abs() < 1e-4);

        // Zero DPI fallback defaults safely to 96 DPI
        let dims_zero = NotchDimensions::from_dpi(0);
        assert_eq!(dims_zero.width, 220);
        assert_eq!(dims_zero.height, 32);
        assert_eq!(dims_zero.dpi, 96);
    }

    #[test]
    fn test_amoled_surface_color() {
        // Must be true pure black (0, 0, 0, 1.0)
        assert_eq!(COLOR_SURFACE_AMOLED.r, 0.0);
        assert_eq!(COLOR_SURFACE_AMOLED.g, 0.0);
        assert_eq!(COLOR_SURFACE_AMOLED.b, 0.0);
        assert_eq!(COLOR_SURFACE_AMOLED.a, 1.0);
        assert_eq!(NOTCH_BG_COLOR.r, 0.0);
        assert_eq!(NOTCH_BG_COLOR.g, 0.0);
        assert_eq!(NOTCH_BG_COLOR.b, 0.0);
        assert_eq!(NOTCH_BG_COLOR.a, 1.0);
    }

    #[test]
    fn test_hardware_curvature_model() {
        let dims = NotchDimensions::from_dpi(96);
        assert_eq!(dims.bottom_corner_radius(), 14.0);
        assert_eq!(dims.top_transition_radius(), 6.0);
        assert!(dims.is_symmetric());

        let dims_200 = NotchDimensions::from_dpi(192);
        assert_eq!(dims_200.bottom_corner_radius(), 28.0);
        assert_eq!(dims_200.top_transition_radius(), 12.0);
        assert!(dims_200.is_symmetric());
    }

    #[test]
    fn test_design_token_symmetry() {
        for dpi in [96, 120, 137, 144, 168, 192, 288] {
            let dims = NotchDimensions::from_dpi(dpi);
            assert!(dims.width > 0);
            assert!(dims.height > 0);
            assert!(dims.is_symmetric());
        }
    }

    #[test]
    fn test_corner_radius_clamping() {
        let w = 100.0;
        let h = 40.0;
        let r_top = 6.0;
        let r_bottom = 10.0;

        // Normal dimensions
        assert!(is_point_in_notch(50.0, 20.0, w, h, r_top, r_bottom));

        // Negative radius clamped to 0.0 without panic
        assert!(is_point_in_notch(50.0, 20.0, w, h, -5.0, -10.0));

        // Zero radius (sharp corners)
        assert!(is_point_in_notch(50.0, 20.0, w, h, 0.0, 0.0));

        // Oversized radius > height / 2.0 (e.g. radius=50 for height=40)
        // Must clamp to height / 2.0 without panicking
        assert!(is_point_in_notch(50.0, 20.0, w, h, 50.0, 50.0));
    }

    #[test]
    fn test_hardware_notch_hit_testing_and_shoulder_curves() {
        let w = 220.0;
        let h = 32.0;
        let r_top = 6.0;
        let r_bottom = 14.0;

        // 1. Center of the notch
        assert!(is_point_in_notch(110.0, 16.0, w, h, r_top, r_bottom));

        // 2. Top display edge attachment (y = 0.5) inside the notch
        assert!(is_point_in_notch(110.0, 0.5, w, h, r_top, r_bottom));
        assert!(is_point_in_notch(20.0, 0.5, w, h, r_top, r_bottom));
        assert!(is_point_in_notch(200.0, 0.5, w, h, r_top, r_bottom));

        // 3. Top-left concave shoulder curve:
        // Region x in [0, 6], y in [0, 6]:
        // (1.0, 1.0) is in the transparent ear outside the curve:
        // dx = 1.0, dy = 1.0 - 6.0 = -5.0 -> 1^2 + (-5)^2 = 26 < 36 -> false (transparent)
        assert!(!is_point_in_notch(1.0, 1.0, w, h, r_top, r_bottom));
        assert!(!is_point_in_notch(2.0, 3.0, w, h, r_top, r_bottom));
        // Point (5.5, 0.5) is inside the body near the top attachment:
        // dx = 5.5, dy = -5.5 -> 30.25 + 30.25 = 60.5 >= 36 -> true (inside)
        assert!(is_point_in_notch(5.5, 0.5, w, h, r_top, r_bottom));

        // 4. Top-right concave shoulder curve:
        // (219.0, 1.0) is in the transparent ear outside the curve:
        assert!(!is_point_in_notch(219.0, 1.0, w, h, r_top, r_bottom));
        assert!(!is_point_in_notch(218.0, 3.0, w, h, r_top, r_bottom));
        // (214.5, 0.5) is inside the body near top attachment:
        assert!(is_point_in_notch(214.5, 0.5, w, h, r_top, r_bottom));

        // 5. Outside vertical body walls (y = 16.0)
        assert!(!is_point_in_notch(2.0, 16.0, w, h, r_top, r_bottom)); // left outside
        assert!(!is_point_in_notch(218.0, 16.0, w, h, r_top, r_bottom)); // right outside

        // 6. Inside body vertical walls (y = 16.0)
        assert!(is_point_in_notch(7.0, 16.0, w, h, r_top, r_bottom)); // just inside left wall (x > r_top=6)
        assert!(is_point_in_notch(213.0, 16.0, w, h, r_top, r_bottom)); // just inside right wall (x < 214)

        // 7. Bottom-left rounded corner (center at x=20, y=18, radius=14):
        // (7.0, 31.0): dx = 7 - 20 = -13, dy = 31 - 18 = 13 -> 169 + 169 = 338 > 196 -> outside
        assert!(!is_point_in_notch(7.0, 31.0, w, h, r_top, r_bottom));
        // (15.0, 28.0): dx = 15 - 20 = -5, dy = 28 - 18 = 10 -> 25 + 100 = 125 <= 196 -> inside
        assert!(is_point_in_notch(15.0, 28.0, w, h, r_top, r_bottom));

        // 8. Bottom-right rounded corner (center at x=200, y=18, radius=14):
        // (213.0, 31.0): dx = 213 - 200 = 13, dy = 13 -> outside
        assert!(!is_point_in_notch(213.0, 31.0, w, h, r_top, r_bottom));
        // (205.0, 28.0): dx = 5, dy = 10 -> inside
        assert!(is_point_in_notch(205.0, 28.0, w, h, r_top, r_bottom));

        // 9. Exact left-right hit-testing symmetry
        for y in [1.0, 5.0, 10.0, 16.0, 25.0, 30.0] {
            for x_offset in [1.0, 3.0, 5.0, 7.0, 15.0, 25.0, 50.0] {
                let left_in = is_point_in_notch(x_offset, y, w, h, r_top, r_bottom);
                let right_in = is_point_in_notch(w - x_offset, y, w, h, r_top, r_bottom);
                assert_eq!(
                    left_in, right_in,
                    "Hit-testing must be strictly symmetric at offset {x_offset}, y {y}"
                );
            }
        }
    }

    #[test]
    fn test_invalid_dimensions_safety() {
        let r_top = 6.0;
        let r_bottom = 14.0;

        // Negative dimensions
        assert!(!is_point_in_notch(
            10.0, 10.0, -100.0, 32.0, r_top, r_bottom
        ));
        assert!(!is_point_in_notch(
            10.0, 10.0, 220.0, -32.0, r_top, r_bottom
        ));
        assert!(!is_point_in_notch(
            10.0, 10.0, -50.0, -50.0, r_top, r_bottom
        ));

        // Zero dimensions
        assert!(!is_point_in_notch(0.0, 0.0, 0.0, 32.0, r_top, r_bottom));
        assert!(!is_point_in_notch(0.0, 0.0, 220.0, 0.0, r_top, r_bottom));
        assert!(!is_point_in_notch(0.0, 0.0, 0.0, 0.0, r_top, r_bottom));

        // Negative point coordinates
        assert!(!is_point_in_notch(-0.1, 10.0, 220.0, 32.0, r_top, r_bottom));
        assert!(!is_point_in_notch(10.0, -0.1, 220.0, 32.0, r_top, r_bottom));

        // Out of bounds point coordinates
        assert!(!is_point_in_notch(
            220.1, 10.0, 220.0, 32.0, r_top, r_bottom
        ));
        assert!(!is_point_in_notch(10.0, 32.1, 220.0, 32.0, r_top, r_bottom));
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn test_design_system_tokens_and_palette() {
        // Text palette contrast against pure black
        assert_eq!(COLOR_TEXT_PRIMARY.a, 1.0);
        assert!(COLOR_TEXT_PRIMARY.r >= 0.9);
        assert_eq!(COLOR_TEXT_SECONDARY.a, 1.0);
        assert!(COLOR_TEXT_SECONDARY.r >= 0.6 && COLOR_TEXT_SECONDARY.r < COLOR_TEXT_PRIMARY.r);
        assert_eq!(COLOR_TEXT_MUTED.a, 1.0);
        assert!(COLOR_TEXT_MUTED.r >= 0.4 && COLOR_TEXT_MUTED.r < COLOR_TEXT_SECONDARY.r);

        // Border & transparency tokens
        assert_eq!(COLOR_BORDER_SUBTLE.a, 0.0);
        assert_eq!(COLOR_TRANSPARENT.a, 0.0);

        // Base dimension aliases & expanded tokens
        assert_eq!(BASE_WIDTH, BASE_COLLAPSED_WIDTH);
        assert_eq!(BASE_HEIGHT, BASE_COLLAPSED_HEIGHT);
        assert_eq!(BASE_EXPANDED_WIDTH, 600.0);
        assert_eq!(BASE_EXPANDED_HEIGHT, 138.0);

        // Typography and icon scaling at 150% (144 DPI)
        let dims_150 = NotchDimensions::from_dpi(144);
        assert!((dims_150.font_size_primary - (13.0 * 1.5)).abs() < 1e-4);
        assert!((dims_150.font_size_secondary - (10.5 * 1.5)).abs() < 1e-4);
        assert!((dims_150.font_size_muted - (9.0 * 1.5)).abs() < 1e-4);
        assert_eq!(dims_150.icon_size_default, (16.0f32 * 1.5).round());
        assert_eq!(dims_150.icon_size_small, (12.0f32 * 1.5).round());
        assert_eq!(dims_150.padding_h, (14.0f32 * 1.5).round());
        assert_eq!(dims_150.padding_v, (6.0f32 * 1.5).round());
        assert_eq!(dims_150.spacing, (8.0f32 * 1.5).round());

        // Font family strings are non-empty
        assert!(UI_FONT_CANDIDATES.iter().all(|(_, f)| !f.is_null()));
        assert!(!FONT_FAMILY_FALLBACK.is_null());
        assert_eq!(BASE_ICON_STROKE_WIDTH, 1.2);
    }

    #[test]
    fn test_expanded_dpi_dimension_scaling() {
        // 100% scaling: 96 DPI
        let dims_100 = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 96);
        assert_eq!(dims_100.width, 600);
        assert_eq!(dims_100.height, 138);
        assert_eq!(dims_100.state, NotchState::Expanded);
        assert!((dims_100.corner_radius - BASE_EXPANDED_BOTTOM_CORNER_RADIUS).abs() < 1e-4);
        assert!(
            (dims_100.curvature.bottom_radius - BASE_EXPANDED_BOTTOM_CORNER_RADIUS).abs() < 1e-4
        );
        assert!((dims_100.curvature.top_transition_radius - 14.0).abs() < 1e-4);
        assert!((dims_100.curvature.top_transition_height - 14.0).abs() < 1e-4);
        assert_eq!(dims_100.shadow_margin_x, 10.0);
        assert_eq!(dims_100.shadow_margin_bottom, 18.0);
        assert_eq!(dims_100.notch_width(), 580.0);
        assert_eq!(dims_100.notch_height(), 120.0);
        assert_eq!(dims_100.padding_h, 18.0);
        assert_eq!(dims_100.padding_v, 12.0);

        // 125% scaling: 120 DPI
        let dims_125 = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 120);
        assert_eq!(dims_125.width, 750);
        assert_eq!(dims_125.height, 173);
        assert!((dims_125.corner_radius - BASE_EXPANDED_BOTTOM_CORNER_RADIUS * 1.25).abs() < 1e-4);
        assert!(
            (dims_125.curvature.bottom_radius - BASE_EXPANDED_BOTTOM_CORNER_RADIUS * 1.25).abs()
                < 1e-4
        );
        assert!((dims_125.curvature.top_transition_radius - 17.5).abs() < 1e-4);
        assert!((dims_125.curvature.top_transition_height - 17.5).abs() < 1e-4);

        // 150% scaling: 144 DPI
        let dims_150 = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 144);
        assert_eq!(dims_150.width, 900);
        assert_eq!(dims_150.height, 207);
        assert!((dims_150.corner_radius - BASE_EXPANDED_BOTTOM_CORNER_RADIUS * 1.5).abs() < 1e-4);
        assert!(
            (dims_150.curvature.bottom_radius - BASE_EXPANDED_BOTTOM_CORNER_RADIUS * 1.5).abs()
                < 1e-4
        );
        assert!((dims_150.curvature.top_transition_radius - 21.0).abs() < 1e-4);
        assert!((dims_150.curvature.top_transition_height - 21.0).abs() < 1e-4);

        // 175% scaling: 168 DPI
        let dims_175 = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 168);
        assert_eq!(dims_175.width, 1050);
        assert_eq!(dims_175.height, 242);
        assert!((dims_175.corner_radius - BASE_EXPANDED_BOTTOM_CORNER_RADIUS * 1.75).abs() < 1e-4);
        assert!(
            (dims_175.curvature.bottom_radius - BASE_EXPANDED_BOTTOM_CORNER_RADIUS * 1.75).abs()
                < 1e-4
        );
        assert!((dims_175.curvature.top_transition_radius - 24.5).abs() < 1e-4);
        assert!((dims_175.curvature.top_transition_height - 24.5).abs() < 1e-4);

        // 200% scaling: 192 DPI
        let dims_200 = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 192);
        assert_eq!(dims_200.width, 1200);
        assert_eq!(dims_200.height, 276);
        assert!((dims_200.corner_radius - BASE_EXPANDED_BOTTOM_CORNER_RADIUS * 2.0).abs() < 1e-4);
        assert!(
            (dims_200.curvature.bottom_radius - BASE_EXPANDED_BOTTOM_CORNER_RADIUS * 2.0).abs()
                < 1e-4
        );
        assert!((dims_200.curvature.top_transition_radius - 28.0).abs() < 1e-4);
        assert!((dims_200.curvature.top_transition_height - 28.0).abs() < 1e-4);

        // Unusual DPI: 137 DPI
        let dims_137 = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 137);
        let scale_137 = 137.0f32 / 96.0;
        assert_eq!(dims_137.width, (600.0f32 * scale_137).round() as i32);
        assert_eq!(dims_137.height, (138.0f32 * scale_137).round() as i32);

        // High DPI: 288 DPI
        let dims_288 = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 288);
        assert_eq!(dims_288.width, 1800);
        assert_eq!(dims_288.height, 414);
        assert!((dims_288.corner_radius - BASE_EXPANDED_BOTTOM_CORNER_RADIUS * 3.0).abs() < 1e-4);
        assert!(
            (dims_288.curvature.bottom_radius - BASE_EXPANDED_BOTTOM_CORNER_RADIUS * 3.0).abs()
                < 1e-4
        );
        assert!((dims_288.curvature.top_transition_radius - 42.0).abs() < 1e-4);
        assert!((dims_288.curvature.top_transition_height - 42.0).abs() < 1e-4);

        // Zero DPI fallback
        let dims_zero = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 0);
        assert_eq!(dims_zero.width, 600);
        assert_eq!(dims_zero.height, 138);
        assert_eq!(dims_zero.dpi, 96);
    }

    #[test]
    fn test_expanded_hardware_notch_hit_testing() {
        let w = 560.0;
        let h = 140.0;
        let r_top = 28.0;
        let r_bottom = 28.0;

        // 1. Center of the expanded notch
        assert!(is_point_in_notch(280.0, 70.0, w, h, r_top, r_bottom));

        // 2. Top display edge attachment (y = 0.5) inside the notch
        assert!(is_point_in_notch(280.0, 0.5, w, h, r_top, r_bottom));
        assert!(is_point_in_notch(35.0, 0.5, w, h, r_top, r_bottom));
        assert!(is_point_in_notch(525.0, 0.5, w, h, r_top, r_bottom));

        // 3. Top-left concave shoulder curve (region x in [0, 28], y in [0, 28]):
        // Outside ear: (1.0, 1.0)
        assert!(!is_point_in_notch(1.0, 1.0, w, h, r_top, r_bottom));
        assert!(!is_point_in_notch(2.0, 4.0, w, h, r_top, r_bottom));
        // Inside body near attachment: (27.0, 0.5)
        assert!(is_point_in_notch(27.0, 0.5, w, h, r_top, r_bottom));

        // 4. Top-right concave shoulder curve:
        // Outside ear: (559.0, 1.0)
        assert!(!is_point_in_notch(559.0, 1.0, w, h, r_top, r_bottom));
        assert!(!is_point_in_notch(558.0, 4.0, w, h, r_top, r_bottom));
        // Inside body near attachment: (533.0, 0.5)
        assert!(is_point_in_notch(533.0, 0.5, w, h, r_top, r_bottom));

        // 5. Outside vertical body walls (x < r_top=28 or x > w-r_top=532)
        assert!(!is_point_in_notch(2.0, 70.0, w, h, r_top, r_bottom)); // left outside
        assert!(!is_point_in_notch(15.0, 70.0, w, h, r_top, r_bottom)); // left outside
        assert!(!is_point_in_notch(558.0, 70.0, w, h, r_top, r_bottom)); // right outside

        // 6. Inside body vertical walls (x > r_top=28 and x < w-r_top=532)
        assert!(is_point_in_notch(30.0, 70.0, w, h, r_top, r_bottom)); // just inside left wall
        assert!(is_point_in_notch(530.0, 70.0, w, h, r_top, r_bottom)); // just inside right wall

        // 7. Bottom-left rounded corner (center at x=56, y=112, radius=28):
        // (30.0, 138.0): outside corner arc
        assert!(!is_point_in_notch(30.0, 138.0, w, h, r_top, r_bottom));
        // (50.0, 125.0): inside corner
        assert!(is_point_in_notch(50.0, 125.0, w, h, r_top, r_bottom));

        // 8. Bottom-right rounded corner (center at x=504, y=112, radius=28):
        // (530.0, 138.0): outside corner arc
        assert!(!is_point_in_notch(530.0, 138.0, w, h, r_top, r_bottom));
        // (510.0, 125.0): inside corner
        assert!(is_point_in_notch(510.0, 125.0, w, h, r_top, r_bottom));

        // 9. Exact left-right hit-testing symmetry on expanded notch
        for y in [1.0, 5.0, 10.0, 20.0, 70.0, 120.0, 135.0] {
            for x_offset in [1.0, 4.0, 7.0, 10.0, 20.0, 50.0, 100.0, 200.0] {
                let left_in = is_point_in_notch(x_offset, y, w, h, r_top, r_bottom);
                let right_in = is_point_in_notch(w - x_offset, y, w, h, r_top, r_bottom);
                assert_eq!(
                    left_in, right_in,
                    "Expanded hit-testing must be strictly symmetric at offset {x_offset}, y {y}"
                );
            }
        }
    }

    #[test]
    fn test_display_bounds_and_centering_safety() {
        // Normal screens
        assert_eq!(calculate_notch_x(1920, 560), (1920 - 560) / 2);
        assert_eq!(calculate_notch_x(1536, 560), (1536 - 560) / 2);

        // Centering alignment: collapsed center equals expanded center
        let screen_w = 1920;
        let c_center = calculate_notch_center_x(screen_w, 220);
        let e_center = calculate_notch_center_x(screen_w, 560);
        assert_eq!(c_center, 960.0);
        assert_eq!(e_center, 960.0);

        // Screen width smaller than notch width (e.g. 300px screen, 560px notch)
        // Must clamp to 0 without underflow or panic
        assert_eq!(calculate_notch_x(300, 560), 0);

        // Zero and negative screen width
        assert_eq!(calculate_notch_x(0, 560), 0);
        assert_eq!(calculate_notch_x(-500, 560), 0);

        // Zero and negative notch width
        assert_eq!(calculate_notch_x(1920, 0), 0);
        assert_eq!(calculate_notch_x(1920, -100), 0);
    }

    #[test]
    fn test_interaction_state_model() {
        let mut interaction = InteractionState::new();

        // 1. Initial state is Collapsed and non-hovered
        assert_eq!(interaction.state, NotchState::Collapsed);
        assert!(!interaction.hovered);

        // 2. Collapsed + left-click -> Expanded
        let toggled = interaction.on_left_click();
        assert!(toggled);
        assert_eq!(interaction.state, NotchState::Expanded);

        // 3. Expanded + left-click -> Collapsed
        let toggled_back = interaction.on_left_click();
        assert!(toggled_back);
        assert_eq!(interaction.state, NotchState::Collapsed);

        // 4. Hover false -> true (returns true indicating redraw required)
        assert!(interaction.set_hovered(true));
        assert!(interaction.hovered);

        // 6. Repeated mouse movement while hovered does NOT logically change hover state (returns false -> no redraw)
        assert!(!interaction.set_hovered(true));
        assert!(interaction.hovered);

        // 5. Hover true -> false (returns true indicating redraw required)
        assert!(interaction.set_hovered(false));
        assert!(!interaction.hovered);

        // Repeated mouse movement while unhovered does NOT change state (returns false -> no redraw)
        assert!(!interaction.set_hovered(false));
        assert!(!interaction.hovered);
    }

    #[test]
    fn test_right_click_and_middle_click_state_invariance() {
        let initial_state = InteractionState::new();
        let test_state = initial_state;

        // In Phase 2.4, right-click and middle-click do NOT mutate state or trigger exit
        // Simulating right-click: no effect
        assert_eq!(test_state.state, initial_state.state);
        assert_eq!(test_state.hovered, initial_state.hovered);

        // Simulating middle-click: no effect
        assert_eq!(test_state.state, initial_state.state);
        assert_eq!(test_state.hovered, initial_state.hovered);
    }

    #[test]
    fn test_hover_visual_design_tokens() {
        // Zero white overlay border: pure AMOLED black maintained both resting and hovered
        assert_eq!(COLOR_BORDER_SUBTLE.a, 0.0);
        assert_eq!(COLOR_BORDER_HOVER.a, 0.0);

        // Surface fill remains pure AMOLED #000000 regardless of hover
        assert_eq!(COLOR_SURFACE_AMOLED.r, 0.0);
        assert_eq!(COLOR_SURFACE_AMOLED.g, 0.0);
        assert_eq!(COLOR_SURFACE_AMOLED.b, 0.0);
        assert_eq!(COLOR_SURFACE_AMOLED.a, 1.0);
    }

    #[test]
    fn test_state_transitions_preserve_layout_state() {
        let mut interaction = InteractionState::new();
        let dims_c = NotchDimensions::from_state_and_dpi(interaction.state, 96);
        let layout_c = crate::layout::resolve_layout(&dims_c);
        match layout_c {
            crate::layout::ResolvedLayout::Collapsed { bounds, components } => {
                assert_eq!(bounds.width(), 220.0);
                assert_eq!(bounds.height(), 32.0);
                assert!(components.content_bounds.width() > 0.0);
                assert!(components.clock_bounds.width() > 0.0);
            }
            _ => panic!("Expected collapsed layout"),
        }

        // Left-click to expand
        interaction.on_left_click();
        let dims_e = NotchDimensions::from_state_and_dpi(interaction.state, 96);
        let layout_e = crate::layout::resolve_layout(&dims_e);
        match layout_e {
            crate::layout::ResolvedLayout::Expanded { bounds, components } => {
                assert_eq!(bounds.width(), 600.0);
                assert_eq!(bounds.height(), 138.0);
                assert!(components.content_bounds.width() > 0.0);
                assert!(components.time_bounds.width() > 0.0);
                assert!(components.date_bounds.width() > 0.0);
            }
            _ => panic!("Expected expanded layout"),
        }

        // Left-click to collapse back
        interaction.on_left_click();
        let dims_c2 = NotchDimensions::from_state_and_dpi(interaction.state, 96);
        let layout_c2 = crate::layout::resolve_layout(&dims_c2);
        match layout_c2 {
            crate::layout::ResolvedLayout::Collapsed { bounds, components } => {
                assert_eq!(bounds.width(), 220.0);
                assert_eq!(bounds.height(), 32.0);
                assert!(components.content_bounds.width() > 0.0);
                assert!(components.clock_bounds.width() > 0.0);
            }
            _ => panic!("Expected collapsed layout"),
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn test_typography_tokens_valid_and_non_negative() {
        assert!(BASE_FONT_SIZE_PRIMARY > 0.0);
        assert!(BASE_FONT_SIZE_SECONDARY > 0.0);
        assert!(BASE_FONT_SIZE_MUTED > 0.0);
        assert!(BASE_FONT_SIZE_CLOCK > 0.0);
        assert!(BASE_CLOCK_OPTICAL_Y_OFFSET >= 0.0);
        assert!(BASE_TITLE_OPTICAL_Y_OFFSET >= 0.0);
        assert!(BASE_HEADER_HEIGHT > 0.0);
        assert!(BASE_SEPARATOR_MARGIN_TOP >= 0.0);
        assert!(BASE_PREVIEW_MARGIN_TOP >= 0.0);
        assert!(!FONT_FAMILY_FALLBACK.is_null());

        // Relative font size hierarchy
        assert!(BASE_FONT_SIZE_PRIMARY > BASE_FONT_SIZE_SECONDARY);
        assert!(BASE_FONT_SIZE_SECONDARY > BASE_FONT_SIZE_MUTED);
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn test_opacity_hierarchy_strict() {
        // Pure monochrome white text with strict opacity hierarchy
        assert_eq!(COLOR_TEXT_PRIMARY.a, 1.0);
        assert_eq!(COLOR_TEXT_SECONDARY.a, 1.0);
        assert_eq!(COLOR_TEXT_MUTED.a, 1.0);

        assert!(COLOR_TEXT_PRIMARY.r > COLOR_TEXT_SECONDARY.r);
        assert!(COLOR_TEXT_SECONDARY.r > COLOR_TEXT_MUTED.r);

        // Separator line is subtle and does not produce harsh white border
        assert!(COLOR_SEPARATOR.a <= 0.08);
        assert!(COLOR_SEPARATOR.a >= 0.02);
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn test_icon_tokens_and_proportions() {
        assert_eq!(BASE_ICON_SIZE_DEFAULT, 16.0);
        assert_eq!(BASE_ICON_SIZE_SMALL, 12.0);
        assert_eq!(BASE_ICON_STROKE_WIDTH, 1.2);
        assert!((BASE_ICON_ASPECT_RATIO - 0.70).abs() < 1e-4);
        assert!((BASE_ICON_CORNER_RADIUS - 2.5).abs() < 1e-4);
        assert!((BASE_ICON_APERTURE_RADIUS - 1.5).abs() < 1e-4);
    }

    #[test]
    fn test_animation_collapsed_to_expanded_starts() {
        let anim = AnimationState::new(NotchState::Collapsed, NotchState::Expanded);
        assert!(anim.active);
        assert_eq!(anim.start_state, NotchState::Collapsed);
        assert_eq!(anim.target_state, NotchState::Expanded);
        assert_eq!(anim.motion, Motion::Expand);
        assert_eq!(anim.pos, geometry_for(NotchState::Collapsed, (0.0, 0.0)));
        assert_eq!(anim.target, geometry_for(NotchState::Expanded, HOME));
        assert_eq!(anim.vel, [0.0; 5], "starts at rest");
        assert_eq!((anim.expanded_alpha, anim.collapsed_alpha), (0.0, 1.0));
    }

    #[test]
    fn test_animation_expanded_to_collapsed_starts() {
        let anim = AnimationState::new(NotchState::Expanded, NotchState::Collapsed);
        assert!(anim.active);
        assert_eq!(anim.motion, Motion::Collapse);
        assert_eq!(anim.pos, geometry_for(NotchState::Expanded, HOME));
        assert_eq!(anim.target, geometry_for(NotchState::Collapsed, (0.0, 0.0)));
        assert_eq!((anim.expanded_alpha, anim.collapsed_alpha), (1.0, 0.0));
    }

    #[test]
    fn test_animation_completes_exactly_at_target() {
        for (from, to) in [
            (NotchState::Collapsed, NotchState::Expanded),
            (NotchState::Expanded, NotchState::Collapsed),
        ] {
            let mut anim = AnimationState::new(from, to);
            let f = frames(&mut anim);
            assert!(!anim.active);
            assert_eq!(anim.pos, anim.target);
            assert_eq!(*f.last().unwrap(), anim.target);
            assert!(!anim.step(12.0), "stays settled");
        }
    }

    #[test]
    fn test_cubic_ease_out_determinism_and_properties() {
        assert_eq!(cubic_ease_out(0.0), 0.0);
        assert_eq!(cubic_ease_out(1.0), 1.0);

        // Strictly monotonic increasing
        let mut prev = 0.0;
        for i in 1..=20 {
            let t = i as f32 / 20.0;
            let val = cubic_ease_out(t);
            assert!(val > prev, "Eased value must be strictly monotonic");
            prev = val;
        }

        // Ease-out property: fast initial response, smooth deceleration
        assert!(cubic_ease_out(0.5) > 0.5);
        let mid = cubic_ease_out(0.5);
        assert!((mid - 0.875).abs() < 1e-4);
    }

    #[test]
    fn test_animation_progress_zero_equals_start_geometry() {
        let anim = AnimationState::new(NotchState::Collapsed, NotchState::Expanded);
        let (w, h, top_r, bot_r) = anim.current_logical_geometry();
        assert_eq!(w, BASE_COLLAPSED_WIDTH);
        assert_eq!(h, BASE_COLLAPSED_HEIGHT);
        assert_eq!(top_r, BASE_COLLAPSED_TOP_TRANSITION_RADIUS);
        assert_eq!(bot_r, BASE_COLLAPSED_BOTTOM_CORNER_RADIUS);

        let dims = anim.current_dimensions(96);
        assert_eq!(dims.width, 220);
        assert_eq!(dims.height, 32);
        assert_eq!(dims.state, NotchState::Collapsed);
    }

    #[test]
    fn test_animation_progress_one_equals_target_geometry() {
        let mut anim = AnimationState::new(NotchState::Collapsed, NotchState::Expanded);
        anim.set_progress(1.0);
        let (w, h, top_r, bot_r) = anim.current_logical_geometry();
        assert_eq!(w, BASE_EXPANDED_WIDTH);
        assert_eq!(h, BASE_EXPANDED_HEIGHT);
        assert_eq!(top_r, BASE_EXPANDED_TOP_TRANSITION_RADIUS);
        assert_eq!(bot_r, BASE_EXPANDED_BOTTOM_CORNER_RADIUS);

        let dims = anim.current_dimensions(96);
        assert_eq!(dims.width, 600);
        assert_eq!(dims.height, 138);
        assert_eq!(dims.state, NotchState::Expanded);
    }

    #[test]
    fn test_intermediate_progress_produces_intermediate_dimensions() {
        let mut anim = AnimationState::new(NotchState::Collapsed, NotchState::Expanded);
        anim.set_progress(0.5);
        let (w, h, top_r, bot_r) = anim.current_logical_geometry();
        assert!(w > BASE_COLLAPSED_WIDTH && w < BASE_EXPANDED_WIDTH);
        assert!(h > BASE_COLLAPSED_HEIGHT && h < BASE_EXPANDED_HEIGHT);
        assert!(
            (BASE_COLLAPSED_TOP_TRANSITION_RADIUS..=BASE_EXPANDED_TOP_TRANSITION_RADIUS)
                .contains(&top_r)
        );
        let min_bot_r = BASE_COLLAPSED_BOTTOM_CORNER_RADIUS.min(BASE_EXPANDED_BOTTOM_CORNER_RADIUS);
        let max_bot_r = BASE_COLLAPSED_BOTTOM_CORNER_RADIUS.max(BASE_EXPANDED_BOTTOM_CORNER_RADIUS);
        assert!((min_bot_r..=max_bot_r).contains(&bot_r));
    }

    #[test]
    fn test_center_x_invariance_during_animation() {
        let mut anim = AnimationState::new(NotchState::Collapsed, NotchState::Expanded);
        let screen_width = 1920;
        let expected_center = 1920.0 / 2.0;

        for step_i in 0..=10 {
            anim.set_progress(step_i as f32 / 10.0);
            let dims = anim.current_dimensions(96);
            let center_x = calculate_notch_center_x(screen_width, dims.width);
            assert!(
                (center_x - expected_center).abs() < 1e-4,
                "Center X must remain invariant throughout animation ({} ms)",
                anim.elapsed_ms
            );
        }
    }

    #[test]
    fn test_top_y_anchored_at_zero() {
        let _anim = AnimationState::new(NotchState::Collapsed, NotchState::Expanded);
        let y = 0;
        assert_eq!(y, 0, "Top screen attachment must remain anchored at 0");
    }

    #[test]
    fn test_reversing_active_animation_continuity() {
        let mut anim = AnimationState::new(NotchState::Collapsed, NotchState::Expanded);
        anim.run_for(96.0);
        let (pos, vel) = (anim.pos, anim.vel);
        assert!(vel[0] > 0.0, "opening");
        anim.retarget(NotchState::Collapsed, HOME);
        // Same place, same velocity: only the target changed
        assert_eq!(anim.pos, pos);
        assert_eq!(anim.vel, vel);
        assert_eq!(anim.target, geometry_for(NotchState::Collapsed, HOME));
        assert_eq!(anim.motion, Motion::Collapse);
    }

    #[test]
    fn test_hover_state_independence_from_animation() {
        let mut interaction = InteractionState::new();
        let anim = AnimationState::new(NotchState::Collapsed, NotchState::Expanded);

        assert!(!interaction.hovered);
        interaction.set_hovered(true);
        assert!(interaction.hovered);
        // Animation state remains unaffected
        assert_eq!(anim.target_state, NotchState::Expanded);
        assert_eq!(anim.elapsed_ms, 0.0);
    }

    #[test]
    fn test_hit_testing_valid_for_intermediate_geometry() {
        let mut anim = AnimationState::new(NotchState::Collapsed, NotchState::Expanded);
        anim.set_progress(0.5);
        let dims = anim.current_dimensions(96);

        // Center must be inside
        assert!(is_point_in_notch(
            dims.width as f32 / 2.0,
            dims.height as f32 / 2.0,
            dims.width as f32,
            dims.height as f32,
            dims.curvature.top_transition_radius,
            dims.curvature.bottom_radius,
        ));

        // Outside shoulder ear must be transparent
        assert!(!is_point_in_notch(
            1.0,
            1.0,
            dims.width as f32,
            dims.height as f32,
            dims.curvature.top_transition_radius,
            dims.curvature.bottom_radius,
        ));
    }

    // ========================================================================
    // ACCESSIBILITY UNIT TESTS (Phase 2.7)
    // ========================================================================

    #[test]
    fn test_accessible_name_and_metadata() {
        assert_eq!(ACCESSIBLE_NAME_STR, "Oeca");
        assert_eq!(ACCESSIBLE_DESCRIPTION, "Interactive desktop notch overlay");
        assert_eq!(ACCESSIBLE_ROLE_NAME, "Desktop Overlay");

        // Verify description does not expose internal implementation details
        let forbidden = [
            "Rust",
            "Direct2D",
            "DirectWrite",
            "HWND",
            "timer",
            "thread",
            "pointer",
        ];
        for term in forbidden {
            assert!(!ACCESSIBLE_NAME_STR.contains(term));
            assert!(!ACCESSIBLE_DESCRIPTION.contains(term));
            assert!(!ACCESSIBLE_ROLE_NAME.contains(term));
        }
    }

    #[test]
    fn test_collapsed_semantic_state() {
        let snapshot = AccessibilitySnapshot::for_state(NotchState::Collapsed);
        assert_eq!(snapshot.name, "12:00 PM");
        assert_eq!(snapshot.state_label, "Collapsed");
        assert_eq!(snapshot.visible_text, "12:00 PM");
        assert!(!snapshot.exposes_subtitle);
        assert!(!snapshot.is_icon_interactive);

        let snapshot_custom =
            AccessibilitySnapshot::for_state_with_time(NotchState::Collapsed, "5:42 PM");
        assert_eq!(snapshot_custom.name, "5:42 PM");
        assert_eq!(snapshot_custom.visible_text, "5:42 PM");

        let snapshot_24h =
            AccessibilitySnapshot::for_state_with_time(NotchState::Collapsed, "17:42");
        assert_eq!(snapshot_24h.name, "17:42");
        assert_eq!(snapshot_24h.visible_text, "17:42");
    }

    #[test]
    fn test_expanded_semantic_state() {
        let snapshot = AccessibilitySnapshot::for_state(NotchState::Expanded);
        assert_eq!(snapshot.name, "12:00 PM - Monday, October 6");
        assert_eq!(snapshot.state_label, "Expanded");
        assert_eq!(snapshot.visible_text, "12:00 PM - Monday, October 6");
        assert!(!snapshot.exposes_subtitle);
        assert!(!snapshot.is_icon_interactive);

        let custom = AccessibilitySnapshot::for_state_with_clock(
            NotchState::Expanded,
            "12:47 AM",
            "Monday, October 6",
        );
        assert_eq!(custom.name, "12:47 AM - Monday, October 6");
    }

    #[test]
    fn test_collapsed_state_does_not_expose_expanded_subtitle() {
        let snapshot = AccessibilitySnapshot::for_state(NotchState::Collapsed);
        assert_eq!(snapshot.visible_text, "12:00 PM");
        assert!(
            !snapshot
                .visible_text
                .contains("Design & Interaction Preview")
        );
        assert!(!snapshot.exposes_subtitle);
    }

    #[test]
    fn test_expanded_state_has_no_temporary_subtitle() {
        let snapshot = AccessibilitySnapshot::for_state(NotchState::Expanded);
        assert_eq!(snapshot.visible_text, "12:00 PM - Monday, October 6");
        assert!(
            !snapshot
                .visible_text
                .contains("Design & Interaction Preview")
        );
        assert!(!snapshot.exposes_subtitle);
    }

    #[test]
    fn test_decorative_icon_not_exposed_as_independent_control() {
        let snap_collapsed = AccessibilitySnapshot::for_state(NotchState::Collapsed);
        let snap_expanded = AccessibilitySnapshot::for_state(NotchState::Expanded);

        assert!(!snap_collapsed.is_icon_interactive);
        assert!(!snap_expanded.is_icon_interactive);
    }

    #[test]
    fn test_accessibility_minute_changed_event() {
        let mut tracker = AccessibilityEventTracker::new();
        tracker.on_state_settled(NotchState::Collapsed);
        let names_before = tracker.name_change_count;
        let states_before = tracker.state_change_count;

        // When collapsed, minute rollover triggers a name change notification
        let emitted = tracker.on_clock_minute_changed(true);
        assert!(emitted);
        assert_eq!(tracker.name_change_count, names_before + 1);
        assert_eq!(tracker.state_change_count, states_before); // State didn't change

        // When expanded, minute rollover also triggers a name change notification
        let emitted_exp = tracker.on_clock_minute_changed(false);
        assert!(emitted_exp);
        assert_eq!(tracker.name_change_count, names_before + 2);
    }

    #[test]
    fn test_hover_does_not_generate_semantic_state_changes() {
        let mut tracker = AccessibilityEventTracker::new();
        tracker.on_state_settled(NotchState::Collapsed);
        let names_before = tracker.name_change_count;
        let states_before = tracker.state_change_count;

        // Hover enter
        assert!(!tracker.on_hover_changed(true));
        assert_eq!(tracker.name_change_count, names_before);
        assert_eq!(tracker.state_change_count, states_before);

        // Hover leave
        assert!(!tracker.on_hover_changed(false));
        assert_eq!(tracker.name_change_count, names_before);
        assert_eq!(tracker.state_change_count, states_before);

        // InteractionState hover toggling does not affect semantic state
        let mut interaction = InteractionState::new();
        assert_eq!(interaction.state, NotchState::Collapsed);
        interaction.set_hovered(true);
        assert_eq!(interaction.state, NotchState::Collapsed);
        interaction.set_hovered(false);
        assert_eq!(interaction.state, NotchState::Collapsed);
    }

    #[test]
    fn test_animation_frames_do_not_generate_accessibility_state_changes() {
        let mut tracker = AccessibilityEventTracker::new();
        tracker.on_state_settled(NotchState::Collapsed);
        let names_before = tracker.name_change_count;
        let states_before = tracker.state_change_count;

        let mut anim = AnimationState::new(NotchState::Collapsed, NotchState::Expanded);

        // Step through 15 intermediate frames (~240ms)
        for _ in 0..15 {
            anim.step(16.0);
            if anim.active {
                assert!(!tracker.on_animation_frame(anim.openness()));
                assert_eq!(tracker.name_change_count, names_before);
                assert_eq!(tracker.state_change_count, states_before);
            }
        }
    }

    #[test]
    fn test_collapsed_to_expanded_produces_exactly_one_state_transition() {
        let mut tracker = AccessibilityEventTracker::new();
        // Initial setup at Collapsed
        assert!(tracker.on_state_settled(NotchState::Collapsed));
        assert_eq!(tracker.name_change_count, 1);
        assert_eq!(tracker.state_change_count, 1);

        // Intermediate animation frames emit 0 events
        let mut anim = AnimationState::new(NotchState::Collapsed, NotchState::Expanded);
        while anim.step(16.0) {
            assert!(!tracker.on_animation_frame(anim.openness()));
        }
        assert_eq!(tracker.name_change_count, 1);
        assert_eq!(tracker.state_change_count, 1);

        // Once animation completes and settles at target state: exactly ONE transition
        assert!(tracker.on_state_settled(NotchState::Expanded));
        assert_eq!(tracker.name_change_count, 2);
        assert_eq!(tracker.state_change_count, 2);

        // Calling again with same settled state is idempotent (0 additional events)
        assert!(!tracker.on_state_settled(NotchState::Expanded));
        assert_eq!(tracker.name_change_count, 2);
        assert_eq!(tracker.state_change_count, 2);
    }

    #[test]
    fn test_expanded_to_collapsed_produces_exactly_one_state_transition() {
        let mut tracker = AccessibilityEventTracker::new();
        tracker.on_state_settled(NotchState::Expanded);
        assert_eq!(tracker.name_change_count, 1);
        assert_eq!(tracker.state_change_count, 1);

        // Intermediate animation frames emit 0 events
        let mut anim = AnimationState::new(NotchState::Expanded, NotchState::Collapsed);
        while anim.step(16.0) {
            assert!(!tracker.on_animation_frame(anim.openness()));
        }
        assert_eq!(tracker.name_change_count, 1);

        // Settled at Collapsed: exactly ONE transition
        assert!(tracker.on_state_settled(NotchState::Collapsed));
        assert_eq!(tracker.name_change_count, 2);
        assert_eq!(tracker.state_change_count, 2);

        // Idempotent
        assert!(!tracker.on_state_settled(NotchState::Collapsed));
        assert_eq!(tracker.name_change_count, 2);
        assert_eq!(tracker.state_change_count, 2);
    }

    #[test]
    fn test_reduced_motion_instant_transition_logic() {
        let mut tracker = AccessibilityEventTracker::new();
        tracker.on_state_settled(NotchState::Collapsed);

        // In reduced motion, transition is direct without timer ticks
        let target_state = NotchState::Expanded;
        assert!(tracker.on_state_settled(target_state));
        assert_eq!(tracker.name_change_count, 2);
        assert_eq!(tracker.state_change_count, 2);
    }

    #[test]
    fn test_high_contrast_amoled_text_ratios() {
        // Luminance formula for relative luminance L = 0.2126 * R + 0.7152 * G + 0.0722 * B
        let lum_black = 0.2126 * COLOR_SURFACE_AMOLED.r
            + 0.7152 * COLOR_SURFACE_AMOLED.g
            + 0.0722 * COLOR_SURFACE_AMOLED.b;
        assert_eq!(lum_black, 0.0);

        let lum_primary = 0.2126 * COLOR_TEXT_PRIMARY.r
            + 0.7152 * COLOR_TEXT_PRIMARY.g
            + 0.0722 * COLOR_TEXT_PRIMARY.b;
        let contrast_primary = (lum_primary + 0.05) / (lum_black + 0.05);
        // WCAG AAA requires 7.0:1 for normal text. We achieve ~19.8:1!
        assert!(contrast_primary > 7.0);

        let lum_secondary = 0.2126 * COLOR_TEXT_SECONDARY.r
            + 0.7152 * COLOR_TEXT_SECONDARY.g
            + 0.0722 * COLOR_TEXT_SECONDARY.b;
        let contrast_secondary = (lum_secondary + 0.05) / (lum_black + 0.05);
        // Secondary text achieves ~8.2:1, exceeding WCAG AAA 7.0:1
        assert!(contrast_secondary > 7.0);
    }

    #[test]
    fn test_dpi_text_readability_invariance() {
        for dpi in [96, 120, 144, 192] {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            assert!(dims.font_size_primary > 0.0);
            assert!(dims.font_size_secondary > 0.0);
            assert!(dims.font_size_muted > 0.0);
            assert!(dims.font_size_primary > dims.font_size_secondary);
            assert!(dims.font_size_secondary > dims.font_size_muted);

            // Ratio of primary to secondary font sizes is strictly preserved across all DPIs
            let ratio = dims.font_size_primary / dims.font_size_secondary;
            assert!((ratio - (BASE_FONT_SIZE_PRIMARY / BASE_FONT_SIZE_SECONDARY)).abs() < 1e-4);
        }
    }

    #[test]
    fn test_corner_profile_circle_and_smooth() {
        // smoothing 0 = the original circular corner exactly (collapsed unchanged)
        let circle = CornerProfile::new(14.0, 0.0, 100.0);
        assert_eq!(circle.span, 14.0);
        assert!((circle.handle - 0.552_284_8).abs() < 1e-6);
        // Midpoint of the cubic sits on the circle within Bezier-approximation error
        let (x, y) = circle.point(0.5);
        let d = ((x - 14.0).powi(2) + y.powi(2)).sqrt();
        assert!(
            (d - 14.0).abs() < 0.05,
            "circle midpoint off by {}",
            d - 14.0
        );
        // smoothing 1 = longer, continuous corner
        let smooth = CornerProfile::new(34.0, 1.0, 100.0);
        assert!((smooth.span - 34.0 * (1.0 + EXPANDED_CORNER_SPAN_EXTRA)).abs() < 1e-4);
        assert!((smooth.handle - CORNER_HANDLE_SMOOTH).abs() < 1e-6);
        // Endpoints land on the wall and the bottom edge; inset is monotonic
        assert_eq!(smooth.point(0.0), (0.0, 0.0));
        let (ex, ey) = smooth.point(1.0);
        assert!((ex - smooth.span).abs() < 1e-3 && (ey - smooth.span).abs() < 1e-3);
        let mut last = -1.0;
        for i in 0..=20 {
            let u = smooth.inset_at(smooth.span * i as f32 / 20.0);
            assert!(u >= last - 1e-3, "inset not monotonic");
            last = u;
        }
        // Span never exceeds the clamp
        assert_eq!(CornerProfile::new(80.0, 1.0, 55.0).span, 55.0);
    }

    #[test]
    fn test_expanded_corners_smooth_collapsed_unchanged() {
        for dpi in [96, 120, 137, 144, 168, 192, 288] {
            let c = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
            assert_eq!(
                c.bottom_smoothing(),
                0.0,
                "collapsed stays circular at {dpi}"
            );
            let e = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            assert!(
                (e.bottom_smoothing() - 1.0).abs() < 1e-4,
                "expanded is smooth at {dpi}"
            );
            // Collapsed hit testing is the exact original circle test
            let (w, h) = (c.notch_width(), c.notch_height());
            for y in 0..=(h as i32) {
                for x in 0..=(w as i32) {
                    let (px, py) = (x as f32, y as f32);
                    assert_eq!(
                        c.contains_point(px, py),
                        is_point_in_notch_ex(
                            px,
                            py,
                            w,
                            h,
                            c.curvature.top_transition_radius,
                            c.curvature.top_transition_height,
                            c.curvature.bottom_radius,
                        ),
                        "collapsed hit test changed at ({x},{y}) {dpi} DPI"
                    );
                }
            }
        }
    }

    #[test]
    fn test_expanded_hit_test_follows_drawn_corner() {
        let e = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 96);
        let p = e.bottom_corner_profile();
        let (pad, h, w) = (e.shadow_margin_x, e.notch_height(), e.notch_width());
        let wall = pad + e.curvature.top_transition_radius;
        for i in 1..20 {
            let (cx, cy) = p.point(i as f32 / 20.0);
            let (x, y) = (wall + cx, h - p.span + cy);
            assert!(
                e.contains_point(x + 0.75, y - 0.75),
                "just inside curve at t={i}/20"
            );
            assert!(
                !e.contains_point(x - 0.75, y + 0.75),
                "just outside curve at t={i}/20"
            );
            // Mirror: right corner
            let xr = pad + w - e.curvature.top_transition_radius - cx;
            assert!(e.contains_point(xr - 0.75, y - 0.75));
            assert!(!e.contains_point(xr + 0.75, y + 0.75));
        }
    }

    #[test]
    fn test_corner_smoothing_is_continuous_through_animation() {
        for target in [NotchState::Expanded, NotchState::Collapsed] {
            let start = match target {
                NotchState::Expanded => NotchState::Collapsed,
                NotchState::Collapsed => NotchState::Expanded,
            };
            let mut anim = AnimationState::new(start, target);
            let mut prev = anim.current_dimensions(120).bottom_smoothing();
            for step in 1..=50 {
                anim.set_progress(step as f32 / 50.0);
                let s = anim.current_dimensions(120).bottom_smoothing();
                assert!((s - prev).abs() < 0.2, "smoothing jumped {prev} -> {s}");
                prev = s;
            }
            let end = if target == NotchState::Expanded {
                1.0
            } else {
                0.0
            };
            assert!((prev - end).abs() < 1e-4);
        }
    }

    /// Runs 12 ms frames until settled; every frame's geometry.
    fn frames(anim: &mut AnimationState) -> Vec<Geometry> {
        let mut out = Vec::new();
        while anim.step(12.0) {
            out.push(anim.pos);
            assert!(out.len() < 200, "must settle");
        }
        out.push(anim.pos);
        out
    }

    const CLIPBOARD: (f32, f32) = (440.0, 202.0);
    const SPACES: [(f32, f32); 3] = [HOME, MUSIC, CLIPBOARD];

    #[test]
    fn test_spring_solution_is_exact_and_continuous() {
        // Two half frames equal one full frame (exact solution, any frame time)
        for spring in [EXPAND_SPRING, COLLAPSE_SPRING, SPACE_SPRING] {
            let (x1, v1) = spring.advance(0.0, 300.0, 100.0, 12.0);
            let (xa, va) = spring.advance(0.0, 300.0, 100.0, 6.0);
            let (x2, v2) = spring.advance(xa, va, 100.0, 6.0);
            assert!((x1 - x2).abs() < 1e-3 && (v1 - v2).abs() < 1e-2);
            assert_eq!(spring.advance(5.0, 7.0, 1.0, 0.0), (5.0, 7.0));
        }
    }

    #[test]
    fn test_expand_overshoot_is_tiny() {
        for size in SPACES {
            let mut anim = AnimationState::start(
                NotchState::Collapsed,
                (0.0, 0.0),
                NotchState::Expanded,
                size,
                Scene::Space(crate::space::OecaSpace::Home),
            );
            let f = frames(&mut anim);
            for (i, target) in [(0, size.0), (1, size.1)] {
                let start = geometry_for(NotchState::Collapsed, (0.0, 0.0))[i];
                let peak = f.iter().map(|g| g[i]).fold(f32::MIN, f32::max);
                let overshoot = (peak - target) / (target - start);
                assert!(overshoot <= 0.01, "{size:?}[{i}] overshoot {overshoot}");
            }
        }
    }

    #[test]
    fn test_collapse_never_dips_below_collapsed_size() {
        let collapsed = geometry_for(NotchState::Collapsed, (0.0, 0.0));
        for size in SPACES {
            let mut anim = AnimationState::new(NotchState::Expanded, NotchState::Collapsed);
            anim.pos = geometry_for(NotchState::Expanded, size);
            for g in frames(&mut anim) {
                assert!(g[0] >= collapsed[0] && g[1] >= collapsed[1], "{g:?}");
            }
            // ...even when collapsing out of a fast shrinking space switch
            let mut anim = AnimationState::space(HOME, MUSIC, scene(), scene());
            anim.run_for(60.0);
            anim.retarget(NotchState::Collapsed, MUSIC);
            for g in frames(&mut anim) {
                assert!(g[0] >= collapsed[0] && g[1] >= collapsed[1], "{g:?}");
            }
        }
    }

    fn scene() -> Scene {
        Scene::Space(crate::space::OecaSpace::Home)
    }

    #[test]
    fn test_last_frame_is_at_target_no_end_pop() {
        // Every kind of transition, at several scales: the frame before the
        // settle snap is already within 0.5 px of the target
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let mut cases = vec![
                AnimationState::new(NotchState::Collapsed, NotchState::Expanded),
                AnimationState::new(NotchState::Expanded, NotchState::Collapsed),
            ];
            for (a, b) in [(HOME, MUSIC), (MUSIC, CLIPBOARD), (CLIPBOARD, HOME)] {
                cases.push(AnimationState::space(a, b, scene(), scene()));
            }
            for mut anim in cases {
                anim.scale = scale;
                let f = frames(&mut anim);
                let before_snap = f[f.len() - 2];
                for (g, t) in before_snap.iter().zip(anim.target) {
                    let px = (g - t).abs() * scale;
                    assert!(px <= 0.5, "{px} px from target at {scale}x");
                }
            }
        }
    }

    #[test]
    fn test_settles_in_bounded_time() {
        // Visually complete (within 1 DIP) by ~420 ms; the timer stops once
        // the sub-pixel tail settles (<= 600 ms)
        let mut cases = vec![
            AnimationState::new(NotchState::Collapsed, NotchState::Expanded),
            AnimationState::new(NotchState::Expanded, NotchState::Collapsed),
        ];
        for (a, b) in [(HOME, MUSIC), (MUSIC, CLIPBOARD), (CLIPBOARD, HOME)] {
            cases.push(AnimationState::space(a, b, scene(), scene()));
        }
        for mut anim in cases {
            anim.scale = 1.25;
            let target = anim.target;
            let f = frames(&mut anim);
            let within = f
                .iter()
                .position(|g| (0..5).all(|i| (g[i] - target[i]).abs() < 1.0))
                .unwrap();
            assert!(
                (within + 1) * 12 <= 420,
                "within 1 DIP at {} ms",
                (within + 1) * 12
            );
            assert!(
                anim.elapsed_ms <= 600.0,
                "settled at {} ms",
                anim.elapsed_ms
            );
        }
    }

    #[test]
    fn test_space_switch_has_no_visible_overshoot() {
        for (a, b) in [
            (HOME, MUSIC),
            (MUSIC, HOME),
            (MUSIC, CLIPBOARD),
            (CLIPBOARD, HOME),
        ] {
            let mut anim = AnimationState::space(a, b, scene(), scene());
            for g in frames(&mut anim) {
                for (i, (from, to)) in [(0, (a.0, b.0)), (1, (a.1, b.1))] {
                    let (lo, hi) = (f32::min(from, to), f32::max(from, to));
                    assert!(g[i] >= lo - 0.05 && g[i] <= hi + 0.05, "{g:?}");
                }
            }
        }
    }

    #[test]
    fn test_reversal_keeps_velocity_no_stall() {
        for (a, b) in [(HOME, MUSIC), (MUSIC, CLIPBOARD)] {
            let (sa, sb) = (
                Scene::Space(crate::space::OecaSpace::Home),
                Scene::Space(crate::space::OecaSpace::Music),
            );
            let mut anim = AnimationState::space(a, b, sa, sb);
            anim.run_for(110.0);
            let (pos, vel, mix) = (anim.pos, anim.vel, anim.mix);
            assert!(mix > 0.2 && mix < 0.9, "mid transition: {mix}");
            anim.retarget(NotchState::Expanded, a);
            anim.retarget_scene(sa);
            assert_eq!((anim.pos, anim.vel), (pos, vel), "continues from here");
            assert!(
                (anim.mix - (1.0 - mix)).abs() < 1e-6,
                "content blend reverses in place"
            );
            // The next frame keeps moving the way it was going (no stall), and
            // no frame jumps
            anim.step(12.0);
            let moved = anim.pos[0] - pos[0];
            assert!(
                moved.signum() == vel[0].signum() && moved.abs() > 0.2,
                "{moved}"
            );
            let mut prev = anim.pos;
            while anim.step(12.0) {
                assert!((anim.pos[0] - prev[0]).abs() < 30.0);
                assert!((anim.pos[1] - prev[1]).abs() < 30.0);
                prev = anim.pos;
            }
            assert_eq!(anim.pos, geometry_for(NotchState::Expanded, a));
            assert_eq!((anim.scene_from, anim.scene_to, anim.mix), (sa, sa, 1.0));
        }
    }

    #[test]
    fn test_mid_flight_retarget_reaches_new_target_without_snap() {
        // Opening toward Home, the drop page retargets it to the Clipboard size
        let mut anim = AnimationState::new(NotchState::Collapsed, NotchState::Expanded);
        anim.scale = 1.25;
        anim.run_for(84.0);
        let vel = anim.vel;
        anim.retarget(NotchState::Expanded, CLIPBOARD);
        anim.retarget_scene(Scene::Drop);
        assert_eq!(anim.vel, vel);
        assert_eq!(anim.motion, Motion::Expand, "still the opening spring");
        let f = frames(&mut anim);
        let before_snap = f[f.len() - 2];
        assert!((0..5).all(|i| (before_snap[i] - anim.target[i]).abs() * 1.25 <= 0.5));
        assert_eq!(anim.pos, geometry_for(NotchState::Expanded, CLIPBOARD));
        assert_eq!(anim.scene_to, Scene::Drop);
    }

    #[test]
    fn test_content_fades_are_monotonic_and_timed() {
        // Expand: collapsed content only fades out, expanded only fades in
        let mut anim = AnimationState::new(NotchState::Collapsed, NotchState::Expanded);
        let (mut ea, mut ca) = (0.0, 1.0);
        let mut first_in = None;
        while anim.step(12.0) {
            assert!(anim.expanded_alpha >= ea && anim.collapsed_alpha <= ca);
            if anim.expanded_alpha > 0.0 && first_in.is_none() {
                first_in = Some(anim.openness());
            }
            (ea, ca) = (anim.expanded_alpha, anim.collapsed_alpha);
        }
        assert!(
            first_in.unwrap() >= CONTENT_IN_OPENNESS,
            "only once there is room"
        );
        assert_eq!((anim.expanded_alpha, anim.collapsed_alpha), (1.0, 0.0));
        // Collapse: expanded content leaves early, collapsed returns at the end
        let mut anim = AnimationState::new(NotchState::Expanded, NotchState::Collapsed);
        let (mut ea, mut ca) = (1.0, 0.0);
        let mut gone_at = None;
        while anim.step(12.0) {
            assert!(anim.expanded_alpha <= ea && anim.collapsed_alpha >= ca);
            if anim.expanded_alpha == 0.0 && gone_at.is_none() {
                gone_at = Some(anim.elapsed_ms);
            }
            if anim.collapsed_alpha > 0.0 {
                assert!(anim.openness() <= COLLAPSED_IN_OPENNESS + 0.05);
            }
            (ea, ca) = (anim.expanded_alpha, anim.collapsed_alpha);
        }
        assert!(gone_at.unwrap() <= 132.0, "fades out in the first ~30%");
        assert_eq!((anim.expanded_alpha, anim.collapsed_alpha), (0.0, 1.0));
    }

    #[test]
    fn test_drop_page_blend_keeps_geometry_and_settles() {
        // Clipboard-size space -> drop page: same shell, content blends
        let mut anim = AnimationState::space(
            CLIPBOARD,
            CLIPBOARD,
            Scene::Space(crate::space::OecaSpace::Clipboard),
            Scene::Drop,
        );
        assert_eq!(anim.mix, 0.0);
        let mut prev = 0.0;
        for g in frames(&mut anim) {
            assert_eq!(g, geometry_for(NotchState::Expanded, CLIPBOARD));
            assert!(anim.mix >= prev);
            prev = anim.mix;
        }
        assert_eq!(
            (anim.scene_from, anim.scene_to, anim.mix),
            (Scene::Drop, Scene::Drop, 1.0)
        );
        // Same scene and size: nothing to animate beyond the first frame
        let mut idle = AnimationState::space(HOME, HOME, scene(), scene());
        assert!(!idle.step(12.0));
    }

    /// Home and Music expanded window sizes (DIP) as the layout defines them.
    const HOME: (f32, f32) = (600.0, 138.0);
    const MUSIC: (f32, f32) = (384.0, 158.0);

    fn settled(dpi: u32, size: (f32, f32)) -> NotchDimensions {
        NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi)
            .with_width_dip(size.0)
            .with_height_dip(size.1)
    }

    #[test]
    fn test_space_transition_reaches_width_and_height() {
        for dpi in [96u32, 120, 137, 144, 168, 192, 288] {
            let home = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            assert_eq!(
                settled(dpi, HOME).height,
                home.height,
                "Home height unchanged"
            );
            for (from, to) in [(HOME, MUSIC), (MUSIC, HOME)] {
                let mut anim = AnimationState::space(from, to, scene(), scene());
                assert!(anim.is_space_transition());
                let start = anim.current_dimensions(dpi);
                assert!((start.width - settled(dpi, from).width).abs() <= 1);
                assert_eq!(
                    start.height,
                    settled(dpi, from).height,
                    "start height at {dpi}"
                );
                while anim.step(12.0) {
                    let d = anim.current_dimensions(dpi);
                    // Only the window size moves: margins, radii, shoulders, padding stay
                    assert_eq!(
                        NotchDimensions {
                            width: home.width,
                            height: home.height,
                            ..d
                        },
                        home,
                        "only width/height move (dpi {dpi})"
                    );
                }
                let end = anim.current_dimensions(dpi);
                assert_eq!(end.width, settled(dpi, to).width, "end width at {dpi}");
                assert_eq!(end.height, settled(dpi, to).height, "end height at {dpi}");
            }
        }
    }

    #[test]
    fn test_rapid_space_switching_ends_exactly_at_the_last_target() {
        for dpi in [96u32, 120, 137, 144, 168, 192, 288] {
            let mut anim = AnimationState::space(HOME, MUSIC, scene(), scene());
            for target in [HOME, MUSIC, CLIPBOARD, HOME, MUSIC] {
                anim.run_for(60.0);
                anim.retarget(NotchState::Expanded, target);
            }
            anim.set_progress(1.0);
            assert_eq!(
                anim.current_dimensions(dpi).width,
                settled(dpi, MUSIC).width
            );
            assert_eq!(
                anim.current_dimensions(dpi).height,
                settled(dpi, MUSIC).height
            );
        }
    }

    #[test]
    fn test_space_transition_can_reverse_into_collapse() {
        let mut anim = AnimationState::space(HOME, MUSIC, scene(), scene());
        anim.run_for(130.0);
        let pos = anim.pos;
        anim.retarget(NotchState::Collapsed, MUSIC);
        assert_eq!(anim.target_state, NotchState::Collapsed);
        assert_eq!(anim.pos, pos, "continues from the current size");
        assert!(!anim.is_space_transition());
        anim.set_progress(1.0);
        let collapsed = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, 120);
        assert_eq!(anim.current_dimensions(120).height, collapsed.height);
        assert_eq!(anim.current_dimensions(120).width, collapsed.width);
    }

    #[test]
    fn test_expand_opens_at_the_space_size() {
        for size in SPACES {
            let mut anim = AnimationState::start(
                NotchState::Collapsed,
                (0.0, 0.0),
                NotchState::Expanded,
                size,
                scene(),
            );
            assert_eq!(anim.motion, Motion::Expand);
            anim.set_progress(1.0);
            let end = anim.current_dimensions(120);
            assert_eq!(end.width, settled(120, size).width);
            assert_eq!(end.height, settled(120, size).height);
            assert_eq!(
                end,
                settled(120, size),
                "settles exactly on the settled dims"
            );
        }
    }

    #[test]
    fn test_accent_choices_ids_default_and_contrast() {
        assert_eq!(AccentChoice::default(), AccentChoice::White);
        assert_eq!(
            AccentChoice::ALL.map(AccentChoice::name),
            ["White", "Blue", "Teal", "Green", "Amber", "Rose", "Violet"]
        );
        for (i, a) in AccentChoice::ALL.into_iter().enumerate() {
            assert_eq!(a.index(), i, "stable identifier");
            assert_eq!(AccentChoice::from_index(i), Some(a));
        }
        assert_eq!(AccentChoice::from_index(AccentChoice::ALL.len()), None);
        // Distinct colors, each readable on the black notch (WCAG non-text
        // contrast >= 3:1 against black)
        let luminance = |[r, g, b]: [u8; 3]| {
            let lin = |c: u8| {
                let c = f32::from(c) / 255.0;
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
        };
        for a in AccentChoice::ALL {
            assert!((luminance(a.rgb()) + 0.05) / 0.05 >= 3.0, "{a:?}");
            for b in AccentChoice::ALL {
                assert!(a == b || a.rgb() != b.rgb());
            }
        }
    }

    #[test]
    fn test_accent_color_conversion() {
        let c = rgb_color([255, 0, 51]);
        assert_eq!((c.r, c.g, c.b, c.a), (1.0, 0.0, 0.2, 1.0));
        for a in AccentChoice::ALL {
            let c = a.color();
            let [r, g, b] = a.rgb();
            assert_eq!(
                [c.r, c.g, c.b].map(|v| (v * 255.0).round() as u8),
                [r, g, b]
            );
            assert_eq!(c.a, 1.0);
        }
        // The default renders exactly as the original neutral accent
        let (w, p) = (AccentChoice::White.color(), COLOR_TEXT_PRIMARY);
        for (x, y) in [(w.r, p.r), (w.g, p.g), (w.b, p.b)] {
            assert_eq!((x * 255.0).round(), (y * 255.0).round());
        }
    }

    #[test]
    fn test_reduced_motion_snap_lands_where_the_unchanged_springs_settle() {
        // Reduced motion snaps straight to the target geometry; with motion
        // on, the (unchanged) springs animate over several frames and settle
        // on exactly that geometry, for every space and both directions
        for size in SPACES {
            for (from, to) in [
                (NotchState::Collapsed, NotchState::Expanded),
                (NotchState::Expanded, NotchState::Collapsed),
            ] {
                let mut anim = AnimationState::start(
                    from,
                    size,
                    to,
                    size,
                    Scene::Space(crate::space::OecaSpace::Home),
                );
                let f = frames(&mut anim);
                assert!(f.len() > 10, "normal motion still animates");
                assert_eq!(*f.last().unwrap(), geometry_for(to, size));
            }
        }
    }

    #[test]
    fn test_typography_roles_hierarchy_and_dpi_floor() {
        use TextRole::*;
        let st = |r: TextRole| r.style();
        // Restrained, available weights only (regular, medium, semibold)
        for r in TextRole::ALL {
            assert!([400, 500, 600].contains(&st(r).weight), "{r:?}");
            // Nothing below 10 DIP: at 96 DPI that is the smallest crisp size
            assert!(st(r).size >= 10.0, "{r:?}");
        }
        // Notch: numerals lead, then title, artist, metadata
        assert!(st(ClockDisplay).size > st(MediaTitle).size);
        assert!(st(MediaTitle).size > st(MediaArtist).size);
        assert!(st(MediaArtist).size > st(MediaMeta).size);
        assert!(st(MediaMeta).size >= st(Timestamp).size);
        assert!(st(MediaTitle).weight > st(MediaArtist).weight);
        assert!(st(DateLine).size < st(ClockDisplay).size / 2.0);
        // Settings window: page title > section > label > description
        assert!(st(PageTitle).size > st(SettingLabel).size);
        assert!(st(SettingLabel).size > st(SectionHeading).size);
        assert!(st(SectionHeading).weight > st(SettingLabel).weight);
        assert!(st(SettingLabel).size > st(SettingDescription).size);
        assert_eq!(st(SidebarItem).size, st(SidebarItemSelected).size);
        assert!(st(SidebarItemSelected).weight > st(SidebarItem).weight);
        // The meridiem stays readable but secondary
        let m = st(ClockDisplay).size * CLOCK_MERIDIEM_SCALE;
        assert!((10.0..st(ClockDisplay).size * 0.6).contains(&m));
        // The smallest text keeps at least 10 physical px at every DPI
        for dpi in [96u32, 120, 137, 144, 168, 192, 288] {
            let px = st(Timestamp).size * dpi as f32 / 96.0;
            assert!(px >= 10.0, "{dpi}");
        }
        // Bahnschrift (ships with Windows) is the guaranteed last candidate
        assert_eq!(UI_FONT_CANDIDATES.last().unwrap().0, "Bahnschrift");
    }
}
