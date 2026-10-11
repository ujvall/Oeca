use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap, ID2D1DCRenderTarget,
    ID2D1Factory, ID2D1PathGeometry, ID2D1RenderTarget,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT, DWRITE_GRID_FIT_MODE_DISABLED, DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
    DWRITE_TEXT_RANGE, IDWriteFactory2, IDWriteRenderingParams,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP,
    HDC, HGDIOBJ, RGBQUAD, ReleaseDC, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{ULW_ALPHA, UpdateLayeredWindow};
use windows::core::{Error, Result};

use crate::clipboard::{ClipboardHistory, ClipboardItem};
use crate::clock::ClockDateState;
use crate::config::Transition;
use crate::config::{
    AccentChoice, BASE_SETTINGS_CHEVRON_SIZE, BASE_SETTINGS_TOGGLE_KNOB_INSET, rgb_color,
};
use crate::config::{
    BASE_CLIPBOARD_ROW_PAD, BASE_CLIPBOARD_ROW_RADIUS, BASE_CLIPBOARD_THUMB_INSET,
    BASE_CLIPBOARD_THUMB_RADIUS, CLIPBOARD_PREVIEW_CHARS, CLIPBOARD_VISIBLE_ROWS,
    COLOR_CLIPBOARD_ROW,
};
use crate::config::{
    BASE_DROP_ICON_SIZE, BASE_DROP_OUTLINE_INSET, BASE_DROP_OUTLINE_RADIUS,
    BASE_DROP_OUTLINE_WIDTH, COLOR_DROP_ICON, COLOR_DROP_OUTLINE, DROP_OUTLINE_DASHES,
};
use crate::config::{
    BASE_MEDIA_ARTWORK_RADIUS_EXTRA, BASE_MEDIA_BADGE_OVERHANG, BASE_MEDIA_BADGE_RING,
    BASE_MEDIA_BADGE_SIZE, BASE_MEDIA_ICON_SIZE, BASE_MEDIA_PLAY_ICON_SIZE,
    BASE_MEDIA_TIMELINE_LABEL_GAP, BASE_MEDIA_TIMELINE_LABEL_WIDTH, BASE_MEDIA_TIMELINE_MIN_TRACK,
    BASE_MEDIA_TIMELINE_TRACK, BASE_MUSIC_ARTWORK_RADIUS, BASE_MUSIC_ICON_SIZE,
    BASE_MUSIC_PLAY_ICON_SIZE, BASE_SPACE_ICON_SIZE, COLOR_ARTWORK_HAIRLINE, COLOR_BORDER_HOVER,
    COLOR_MEDIA_DIVIDER, COLOR_MEDIA_TRACK_UNPLAYED, COLOR_SPACE_PILL_SELECTED, COLOR_TEXT_PRIMARY,
    COLOR_TEXT_SECONDARY, COLOR_TEXT_TERTIARY, COLOR_TRANSPARENT, CornerProfile,
    FONT_FAMILY_FALLBACK, MEDIA_ICON_REST_OPACITY, MEDIA_PRESS_ICON_SCALE, NOTCH_BG_COLOR,
    NOTCH_BORDER_COLOR, NotchDimensions, SKIP_GLYPH_HALF_WIDTH,
};
use crate::config::{TextRole, UI_FONT_CANDIDATES};
#[cfg(test)]
use crate::layout::resolve_media_layout;
use crate::layout::{
    ClipboardLayout, PanelHit, blended_selector, resolve_clipboard_layout, resolve_settings_layout,
    space_dimensions,
};
use crate::layout::{
    CollapsedLayout, ExpandedLayout, MediaLayout, RectF, ResolvedLayout, resolve_collapsed_layout,
    resolve_layout, resolve_media_layout_in, visualizer_metrics,
};
use crate::media::{
    Artwork, ControlFeedback, MediaContent, MediaControl, PlayPauseIcon, Visualizer, format_clock,
    now_filetime, shows_visualizer,
};
use crate::space::NottSpace;
use crate::space::{NottSettings, PANEL_SPACE, Scene};
use windows::Win32::Graphics::Direct2D::{
    D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_CUSTOM, D2D1_LAYER_PARAMETERS, D2D1_LINE_JOIN_ROUND,
    D2D1_STROKE_STYLE_PROPERTIES, ID2D1StrokeStyle,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct D2D_POINT_2F {
    pub x: f32,
    pub y: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct D2D1_BEZIER_SEGMENT {
    pub point1: D2D_POINT_2F,
    pub point2: D2D_POINT_2F,
    pub point3: D2D_POINT_2F,
}

pub struct GeometrySinkHelper {
    raw: *mut std::ffi::c_void,
    vtable: *const usize,
}

impl GeometrySinkHelper {
    pub unsafe fn from_raw(raw: *mut std::ffi::c_void) -> Self {
        unsafe {
            let vtable = *(raw as *const *const usize);
            Self { raw, vtable }
        }
    }

    pub unsafe fn begin_figure(&self, start: D2D_POINT_2F) {
        type FnBeginFigure = unsafe extern "system" fn(*mut std::ffi::c_void, D2D_POINT_2F, u32);
        unsafe {
            let func: FnBeginFigure = std::mem::transmute(*self.vtable.add(5));
            func(self.raw, start, 0); // 0 = D2D1_FIGURE_BEGIN_FILLED
        }
    }

    pub unsafe fn add_line(&self, point: D2D_POINT_2F) {
        type FnAddLine = unsafe extern "system" fn(*mut std::ffi::c_void, D2D_POINT_2F);
        unsafe {
            let func: FnAddLine = std::mem::transmute(*self.vtable.add(10));
            func(self.raw, point);
        }
    }

    pub unsafe fn add_bezier(&self, bezier: &D2D1_BEZIER_SEGMENT) {
        type FnAddBezier =
            unsafe extern "system" fn(*mut std::ffi::c_void, *const D2D1_BEZIER_SEGMENT);
        unsafe {
            let func: FnAddBezier = std::mem::transmute(*self.vtable.add(11));
            func(self.raw, bezier);
        }
    }

    pub unsafe fn end_figure(&self) {
        type FnEndFigure = unsafe extern "system" fn(*mut std::ffi::c_void, u32);
        unsafe {
            let func: FnEndFigure = std::mem::transmute(*self.vtable.add(8));
            func(self.raw, 1); // 1 = D2D1_FIGURE_END_CLOSED
        }
    }

    pub unsafe fn end_figure_open(&self) {
        type FnEndFigure = unsafe extern "system" fn(*mut std::ffi::c_void, u32);
        unsafe {
            let func: FnEndFigure = std::mem::transmute(*self.vtable.add(8));
            func(self.raw, 0); // 0 = D2D1_FIGURE_END_OPEN
        }
    }

    pub unsafe fn close(&self) -> Result<()> {
        type FnClose = unsafe extern "system" fn(*mut std::ffi::c_void) -> windows::core::HRESULT;
        unsafe {
            let func: FnClose = std::mem::transmute(*self.vtable.add(9));
            func(self.raw).ok()
        }
    }
}

pub struct Renderer {
    d2d_factory: ID2D1Factory,
    dwrite_factory: IDWriteFactory,
    cached_screen_dc: Cell<HDC>,
    cached_mem_dc: Cell<HDC>,
    cached_dib: Cell<HBITMAP>,
    cached_bits: Cell<*mut std::ffi::c_void>,
    cached_old_bmp: Cell<HGDIOBJ>,
    cached_rt: RefCell<Option<ID2D1DCRenderTarget>>,
    cached_capacity_w: Cell<i32>,
    cached_capacity_h: Cell<i32>,
    scratch_a: RefCell<Vec<u8>>,
    scratch_b: RefCell<Vec<u8>>,
    /// Silhouette (dimensions, hovered border) the shadow mask in `scratch_a`
    /// was built for.
    shadow_key: Cell<Option<(NotchDimensions, bool)>>,
    /// Expanded media composition content (None = clock/date expanded state).
    /// UTF-16 is encoded once per content change, not per frame.
    media: RefCell<Option<MediaText>>,
    /// Hover/press micro-interaction state of the transport controls.
    feedback: RefCell<ControlFeedback>,
    /// Device bitmap of the current artwork (tied to `cached_rt`; at most one).
    art_bitmap: RefCell<Option<(Artwork, ID2D1Bitmap)>>,
    /// Device bitmap of the source-app badge (tied to `cached_rt`; at most one).
    /// Keyed by (icon, on-screen pixel size): resampled once per icon and DPI.
    badge_bitmap: RefCell<Option<(Artwork, u32, ID2D1Bitmap)>>,
    /// Media text formats, cached per DPI.
    media_formats: RefCell<Option<(u32, MediaFormats)>>,
    /// Collapsed clock format, cached per font size (the collapsed notch redraws
    /// every frame while the visualizer runs).
    clock_format: RefCell<Option<(u32, IDWriteTextFormat)>>,
    /// Playback visualizer fade/motion (window drives `step` on the live timer).
    visualizer: RefCell<Visualizer>,
    /// Time origin of the visualizer motion.
    epoch: std::time::Instant,
    /// Active space shown by the selector (mirrors `WindowState.space`, which
    /// sets it on every switch).
    space: Cell<NottSpace>,
    /// An image drag is over the notch: the expanded notch shows the drop page.
    drop_page: Cell<bool>,
    /// The running animation's content blend and opacity (None when settled).
    transition: Cell<Option<Transition>>,
    /// Settings page open / settings values (mirror `WindowState`, which sets
    /// them on every change).
    settings_open: Cell<bool>,
    settings: Cell<NottSettings>,
    /// The Clipboard space shows its clear confirmation (mirrors `WindowState`).
    clear_pending: Cell<bool>,
    /// The Settings page's switches (Always on top, Reduced motion) as they
    /// slide; stepped on the control-feedback timer.
    switches: RefCell<[SwitchAnim; 2]>,
    /// Round-capped dashes of the drop page outline (device independent).
    drop_stroke: ID2D1StrokeStyle,
    /// Text rendering for the layered surface (see `notch_text_params`).
    text_params: IDWriteRenderingParams,
    /// Solid round caps and joins (the outline copy icon).
    round_stroke: ID2D1StrokeStyle,
    /// Clipboard space rows, rebuilt only when the history changes.
    clipboard: RefCell<ClipboardView>,
    /// Device bitmaps of the shown clipboard thumbnails, keyed by (image, pixel
    /// size); tied to `cached_rt`, pruned to the visible rows on every change.
    thumbs: RefCell<Vec<(Artwork, u32, ID2D1Bitmap)>>,
}

/// What the Clipboard space shows, UTF-16 encoded once per history change.
#[derive(Default)]
struct ClipboardView {
    /// The newest entries (at most `CLIPBOARD_VISIBLE_ROWS`).
    rows: Vec<ClipRow>,
}

enum ClipRow {
    /// One-line, bounded preview.
    Text(Vec<u16>),
    /// The entry's image (shared pixels) and its "Image · W × H" caption.
    Image(Artwork, Vec<u16>),
}

/// Settings page text.
const SETTINGS_LABEL: &str = "Settings";
const ALWAYS_ON_TOP_TITLE: &str = "Always on top";
const ALWAYS_ON_TOP_DESCRIPTION: &str = "Keep Nott above other windows";
const REDUCED_MOTION_TITLE: &str = "Reduced motion";
const REDUCED_MOTION_DESCRIPTION: &str = "Minimize Nott's interface transitions";
const FULL_SETTINGS_TITLE: &str = "Open Full Settings";

/// Clipboard space empty state.
const CLIPBOARD_EMPTY_LABEL: &str = "Copied text and images appear here";
/// Clipboard clear confirmation.
const CLEAR_CONFIRM_MESSAGE: &str = "Clear clipboard history?";
const CLEAR_CONFIRM_NOTE: &str = "Only Nott's history is removed. Your clipboard stays.";
const CLEAR_CONFIRM_CANCEL: &str = "Cancel";
const CLEAR_CONFIRM_CLEAR: &str = "Clear";

struct MediaText {
    content: MediaContent,
    title: Vec<u16>,
    subtitle: Vec<u16>,
    source: Vec<u16>,
}

struct MediaFormats {
    title: IDWriteTextFormat,
    artist: IDWriteTextFormat,
    source: IDWriteTextFormat,
    time: IDWriteTextFormat,
    date: IDWriteTextFormat,
    /// Scrubber labels hugging the strip's edges: elapsed (leading) and
    /// remaining (trailing), aligned with the cover and visualizer edges
    elapsed: IDWriteTextFormat,
    remaining: IDWriteTextFormat,
    /// Music space without a session (centered)
    empty: IDWriteTextFormat,
    /// Expanded Home clock without media: numerals and date line (centred).
    clock_display: IDWriteTextFormat,
    date_line: IDWriteTextFormat,
}

/// Music space empty state (no media session): a quiet label, never metadata.
const MUSIC_EMPTY_LABEL: &str = "Nothing playing";

/// Accent of the shown media (artwork-derived), or else the user's accent.
fn accent_color(content: Option<&MediaContent>, user: AccentChoice) -> D2D1_COLOR_F {
    match content.and_then(|c| c.accent) {
        Some(rgb) => rgb_color(rgb),
        None => user.color(),
    }
}

/// Area-averaging resample of premultiplied BGRA to `px` x `px` (each output
/// pixel is the coverage-weighted mean of the source pixels under it), so small
/// icons stay smooth instead of aliasing. Returns the input when already that size.
fn resample_area(src: &Artwork, px: u32) -> Artwork {
    let (sw, sh) = (src.width as usize, src.height as usize);
    let d = px.max(1) as usize;
    if sw == d && sh == d {
        return src.clone();
    }
    let (fx, fy) = (sw as f32 / d as f32, sh as f32 / d as f32);
    let mut out = vec![0u8; d * d * 4];
    for oy in 0..d {
        let (y0, y1) = (oy as f32 * fy, (oy + 1) as f32 * fy);
        for ox in 0..d {
            let (x0, x1) = (ox as f32 * fx, (ox + 1) as f32 * fx);
            let mut acc = [0f32; 4];
            let mut total = 0f32;
            for sy in (y0.floor() as usize)..(y1.ceil() as usize).min(sh) {
                let wy = (y1.min(sy as f32 + 1.0) - y0.max(sy as f32)).max(0.0);
                for sx in (x0.floor() as usize)..(x1.ceil() as usize).min(sw) {
                    let w = wy * (x1.min(sx as f32 + 1.0) - x0.max(sx as f32)).max(0.0);
                    let i = (sy * sw + sx) * 4;
                    for (c, a) in acc.iter_mut().enumerate() {
                        *a += src.pixels[i + c] as f32 * w;
                    }
                    total += w;
                }
            }
            let o = (oy * d + ox) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / total.max(f32::EPSILON)).round() as u8;
            }
        }
    }
    Artwork::new(d as u32, d as u32, out).unwrap_or_else(|| src.clone())
}

/// Nott's typeface: the first `UI_FONT_CANDIDATES` family installed (system
/// and per-user fonts), else the documented `FONT_FAMILY_FALLBACK`. Resolved
/// once per process; shared by the notch and the Settings window.
pub(crate) fn ui_font(dwrite: &IDWriteFactory) -> (&'static str, windows::core::PCWSTR) {
    static CHOICE: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    let choice = *CHOICE.get_or_init(|| {
        let families = UI_FONT_CANDIDATES.map(|(_, f)| f);
        first_installed(dwrite, &families)
    });
    match choice {
        Some(i) => UI_FONT_CANDIDATES[i],
        None => (
            crate::config::FONT_FAMILY_FALLBACK_NAME,
            FONT_FAMILY_FALLBACK,
        ),
    }
}

/// Index of the first of `families` installed, if any.
pub(crate) fn first_installed(
    dwrite: &IDWriteFactory,
    families: &[windows::core::PCWSTR],
) -> Option<usize> {
    let mut collection = None;
    unsafe { dwrite.GetSystemFontCollection(&mut collection, false) }.ok()?;
    let collection = collection?;
    families.iter().position(|family| {
        let (mut index, mut exists) = (0u32, windows::core::BOOL(0));
        unsafe { collection.FindFamilyName(*family, &mut index, &mut exists) }.is_ok()
            && exists.as_bool()
    })
}

/// Text rendering for the notch's layered surface: the system's own gamma
/// and contrast (so the user's ClearType tuning still applies), greyscale
/// (no ClearType: the surface is transparent, premultiplied), symmetric
/// natural rendering and no vertical grid-fitting, so glyphs keep their true
/// outlines at fractional DPIs instead of snapping into uneven, stepped
/// stems.
pub(crate) fn notch_text_params(dwrite: &IDWriteFactory) -> Result<IDWriteRenderingParams> {
    unsafe {
        let system = dwrite.CreateRenderingParams()?;
        let factory: IDWriteFactory2 = windows::core::Interface::cast(dwrite)?;
        let grayscale_contrast = windows::core::Interface::cast::<
            windows::Win32::Graphics::DirectWrite::IDWriteRenderingParams1,
        >(&system)
        .map_or(0.0, |p| p.GetGrayscaleEnhancedContrast());
        let params = factory.CreateCustomRenderingParams(
            system.GetGamma(),
            system.GetEnhancedContrast(),
            grayscale_contrast,
            0.0,
            windows::Win32::Graphics::DirectWrite::DWRITE_PIXEL_GEOMETRY_FLAT,
            DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
            DWRITE_GRID_FIT_MODE_DISABLED,
        )?;
        windows::core::Interface::cast(&params)
    }
}

/// UTF-16 (start, length) of a time's day-period marker ("AM", "p.m.",
/// "下午"...) at its start or end, including the space that separates it
/// from the numerals; None for 24-hour times.
pub fn meridiem_range(time: &str) -> Option<(u32, u32)> {
    let units: Vec<char> = time.chars().collect();
    let is_mark = |c: char| c.is_alphabetic() || c == '.';
    let len16 = |cs: &[char]| cs.iter().map(|c| c.len_utf16() as u32).sum::<u32>();
    // Trailing marker: letters at the end, plus the whitespace before them
    let mut end = units.len();
    while end > 0 && is_mark(units[end - 1]) {
        end -= 1;
    }
    if end < units.len() && units[..end].iter().any(char::is_ascii_digit) {
        let mut start = end;
        while start > 0 && units[start - 1].is_whitespace() {
            start -= 1;
        }
        return Some((len16(&units[..start]), len16(&units[start..])));
    }
    // Leading marker: letters at the start, plus the whitespace after them
    let mut start = 0;
    while start < units.len() && is_mark(units[start]) {
        start += 1;
    }
    if start > 0 && units[start..].iter().any(char::is_ascii_digit) {
        let mut end = start;
        while end < units.len() && units[end].is_whitespace() {
            end += 1;
        }
        return Some((0, len16(&units[..end])));
    }
    None
}

/// Fills a UIcons glyph (`IconSeg` path) in a `u`-sized box centred on (cx, cy);
/// shared by the notch and the Settings window.
pub(crate) fn fill_glyph(
    factory: &ID2D1Factory,
    rt: &ID2D1RenderTarget,
    segments: &[IconSeg],
    cx: f32,
    cy: f32,
    u: f32,
    brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
) -> Result<()> {
    let p = |x: f32, y: f32| D2D_POINT_2F {
        x: cx + x * u,
        y: cy + y * u,
    };
    let path = unsafe { factory.CreatePathGeometry()? };
    let sink = unsafe { path.Open()? };
    let helper = unsafe { GeometrySinkHelper::from_raw(windows::core::Interface::as_raw(&sink)) };
    unsafe {
        for seg in segments {
            match *seg {
                IconSeg::M(x, y) => helper.begin_figure(p(x, y)),
                IconSeg::L(x, y) => helper.add_line(p(x, y)),
                IconSeg::C(x1, y1, x2, y2, x, y) => helper.add_bezier(&D2D1_BEZIER_SEGMENT {
                    point1: p(x1, y1),
                    point2: p(x2, y2),
                    point3: p(x, y),
                }),
                IconSeg::Z => helper.end_figure(),
            }
        }
        helper.close()?;
        rt.FillGeometry(&path, brush, None);
    }
    Ok(())
}

/// A switch's animated state: its knob travel `level()` from 0 (OFF) to 1
/// (ON), eased in and out over `SWITCH_TRAVEL_MS` for a full travel. A new
/// target mid-way starts from the current level (no jump) and takes only the
/// remaining distance's share of the time. Stepped by its owner's timer only
/// while `is_animating()`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SwitchAnim {
    from: f32,
    to: f32,
    /// Progress through the current travel, 0..=1.
    t: f32,
}

impl SwitchAnim {
    pub fn settled(on: bool) -> Self {
        let v = f32::from(u8::from(on));
        Self {
            from: v,
            to: v,
            t: 1.0,
        }
    }

    pub fn level(&self) -> f32 {
        let t = self.t.clamp(0.0, 1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        self.from + (self.to - self.from) * eased
    }

    pub fn is_animating(&self) -> bool {
        self.t < 1.0
    }

    /// Heads for `on`: animated, or at once when `instant` (Reduced motion).
    pub fn set(&mut self, on: bool, instant: bool) {
        let target = f32::from(u8::from(on));
        if instant {
            *self = Self::settled(on);
        } else if target != self.to {
            *self = Self {
                from: self.level(),
                to: target,
                t: 0.0,
            };
        }
    }

    /// Advances by `dt_ms`; true while still moving.
    pub fn step(&mut self, dt_ms: f32) -> bool {
        if self.t < 1.0 {
            let span = (self.to - self.from).abs().max(0.05) * crate::config::SWITCH_TRAVEL_MS;
            self.t = (self.t + dt_ms / span).min(1.0);
        }
        self.is_animating()
    }
}

/// A switch, shared by the notch Settings page and the Settings window: white
/// track with a black knob at the right when on; a faint track with a white
/// knob at the left when off. `knob_scale` < 1 tucks the knob in (pressed).
pub(crate) fn draw_switch(
    rt: &ID2D1RenderTarget,
    t: RectF,
    level: f32,
    inset: f32,
    knob_scale: f32,
    accent: AccentChoice,
) -> Result<()> {
    // `level`: 0 = OFF, 1 = ON (in between while it animates); the track
    // colour and knob colour blend with the knob's travel
    let level = level.clamp(0.0, 1.0);
    let mix = |a: &D2D1_COLOR_F, b: &D2D1_COLOR_F| D2D1_COLOR_F {
        r: a.r + (b.r - a.r) * level,
        g: a.g + (b.g - a.g) * level,
        b: a.b + (b.b - a.b) * level,
        a: a.a + (b.a - a.a) * level,
    };
    unsafe {
        // On: the accent track with the knob at the right (position, not
        // color alone, shows the state)
        let track =
            rt.CreateSolidColorBrush(&mix(&COLOR_SPACE_PILL_SELECTED, &accent.color()), None)?;
        rt.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: t.to_d2d_rect(),
                radiusX: t.height() / 2.0,
                radiusY: t.height() / 2.0,
            },
            &track,
        );
        let full = t.height() - 2.0 * inset;
        let (off_left, on_left) = (t.left + inset, t.right - inset - full);
        let left = off_left + (on_left - off_left) * level;
        let (cx, cy, d) = (
            left + full / 2.0,
            t.top + inset + full / 2.0,
            full * knob_scale,
        );
        let knob = RectF::new(cx - d / 2.0, cy - d / 2.0, cx + d / 2.0, cy + d / 2.0);
        let knob_brush =
            rt.CreateSolidColorBrush(&mix(&COLOR_TEXT_PRIMARY, &NOTCH_BG_COLOR), None)?;
        rt.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: knob.to_d2d_rect(),
                radiusX: d / 2.0,
                radiusY: d / 2.0,
            },
            &knob_brush,
        );
    }
    Ok(())
}

/// Opacity of a blended scene part at weight `w` (0..1): smoothstep over
/// 0.4..0.9, so an outgoing part is gone before the incoming one is fully in,
/// and swapping from/to with `1 - w` gives the same value (reversals are seamless).
fn scene_fade(w: f32) -> f32 {
    smoothstep(0.4, 0.9, w)
}

/// 0 below `a`, 1 above `b`, eased in between.
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Settled drop page dimensions at a DPI (the panel size).
fn drop_page_dimensions(dpi: u32) -> NotchDimensions {
    crate::layout::space_dimensions(crate::config::NotchState::Expanded, dpi, PANEL_SPACE)
}

/// Clipboard thumbnail: the centre square of `src` (cover crop, aspect kept),
/// area-resampled to `px` x `px`, with corners rounded to `radius` pixels by
/// scaling the premultiplied pixels with their coverage. Built from the
/// entry's existing pixels: nothing is decoded.
fn thumbnail(src: &Artwork, px: u32, radius: f32) -> Artwork {
    let side = src.width.min(src.height);
    let (x0, y0) = (
        (src.width - side) as usize / 2,
        (src.height - side) as usize / 2,
    );
    let (w, n) = (src.width as usize, side as usize);
    let mut square = Vec::with_capacity(n * n * 4);
    for y in y0..y0 + n {
        let start = (y * w + x0) * 4;
        square.extend_from_slice(&src.pixels[start..start + n * 4]);
    }
    let Some(square) = Artwork::new(side, side, square) else {
        return src.clone();
    };
    let thumb = resample_area(&square, px);
    let d = thumb.width as f32;
    let r = radius.clamp(0.0, d / 2.0);
    let mut pixels = thumb.pixels.to_vec();
    for (i, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (
            (i as u32 % thumb.width) as f32 + 0.5,
            (i as u32 / thumb.width) as f32 + 0.5,
        );
        let (cx, cy) = (x.clamp(r, d - r), y.clamp(r, d - r));
        let dist = (x - cx).hypot(y - cy);
        let cover = if dist == 0.0 {
            1.0
        } else {
            (r - dist + 0.5).clamp(0.0, 1.0)
        };
        if cover < 1.0 {
            for c in pixel.iter_mut() {
                *c = (f32::from(*c) * cover).round() as u8;
            }
        }
    }
    Artwork::new(thumb.width, thumb.height, pixels).unwrap_or(thumb)
}

/// Skip glyph height (fraction of the icon size).
const SKIP_GLYPH_HEIGHT: f32 = 0.74;
/// Triangle vertex rounding (fraction of the triangle's height).
const TRIANGLE_ROUNDING: f32 = 0.16;

/// One filled primitive of a transport icon.
#[derive(Debug, Clone, Copy, PartialEq)]
enum IconShape {
    /// Capsule bar (fully rounded ends)
    Bar(RectF),
    Triangle([(f32, f32); 3]),
}

/// Geometry of a transport icon of size `u` (pixels) centered at (cx, cy).
/// With `snap`, every edge lands on a whole pixel so bars stay crisp at small
/// sizes (bars are at least 2 px wide). Proportions are shared so Previous and
/// Next are exact mirrors and Play is optically centered: its left edge sits so
/// the midpoint of the box center and the centroid falls on the axis.
fn icon_shapes(
    control: MediaControl,
    play_pause: PlayPauseIcon,
    cx: f32,
    cy: f32,
    u: f32,
    snap: bool,
) -> Vec<IconShape> {
    let q = |v: f32| if snap { v.round() } else { v };
    let len = |v: f32, min: f32| if snap { v.round().max(min) } else { v.max(min) };
    match (control, play_pause) {
        (MediaControl::PlayPause, PlayPauseIcon::Pause) => {
            let w = len(u * 0.26, 2.0);
            let gap = len(u * 0.24, 2.0);
            let h = len(u * 0.92, 2.0);
            let left = q(cx - (2.0 * w + gap) / 2.0);
            let top = q(cy - h / 2.0);
            vec![
                IconShape::Bar(RectF::new(left, top, left + w, top + h)),
                IconShape::Bar(RectF::new(
                    left + w + gap,
                    top,
                    left + 2.0 * w + gap,
                    top + h,
                )),
            ]
        }
        (MediaControl::PlayPause, PlayPauseIcon::Play) => {
            let h = len(u, 2.0);
            let w = len(u * 0.88, 2.0);
            let left = q(cx - w * 5.0 / 12.0);
            let top = q(cy - h / 2.0);
            vec![IconShape::Triangle([
                (left, top),
                (left + w, top + h / 2.0),
                (left, top + h),
            ])]
        }
        (MediaControl::Next | MediaControl::Previous, _) => {
            // Two touching triangles (fast-forward / rewind style)
            let h = len(u * SKIP_GLYPH_HEIGHT, 2.0);
            let tw = len(u * SKIP_GLYPH_HALF_WIDTH, 2.0);
            let left = q(cx - tw);
            let top = q(cy - h / 2.0);
            let mid = top + h / 2.0;
            let tri = |base: f32, tip: f32| {
                IconShape::Triangle([(base, top), (tip, mid), (base, top + h)])
            };
            if control == MediaControl::Next {
                vec![tri(left, left + tw), tri(left + tw, left + 2.0 * tw)]
            } else {
                vec![tri(left + tw, left), tri(left + 2.0 * tw, left + tw)]
            }
        }
    }
}

/// For each triangle vertex: the points where its rounded corner starts (`a`, on
/// the incoming edge) and ends (`b`, on the outgoing edge). The rounding length is
/// a fraction of the triangle's height, capped so corners never overlap.
type Corners = [(f32, f32); 3];
fn rounded_triangle_corners(pts: Corners) -> (Corners, Corners) {
    let dist = |p: (f32, f32), q: (f32, f32)| ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt();
    let ys = pts.map(|p| p.1);
    let height =
        ys.iter().cloned().fold(f32::MIN, f32::max) - ys.iter().cloned().fold(f32::MAX, f32::min);
    let min_edge = dist(pts[0], pts[1])
        .min(dist(pts[1], pts[2]))
        .min(dist(pts[2], pts[0]));
    let r = (height * TRIANGLE_ROUNDING).min(min_edge * 0.45).max(0.0);
    let toward = |from: (f32, f32), to: (f32, f32)| {
        let d = dist(from, to).max(f32::EPSILON);
        (
            from.0 + (to.0 - from.0) * r / d,
            from.1 + (to.1 - from.1) * r / d,
        )
    };
    let mut a = [(0.0, 0.0); 3];
    let mut b = [(0.0, 0.0); 3];
    for k in 0..3 {
        let (prev, next) = (pts[(k + 2) % 3], pts[(k + 1) % 3]);
        a[k] = toward(pts[k], prev);
        b[k] = toward(pts[k], next);
    }
    (a, b)
}

/// Source rectangle that center-crops a `w`x`h` image to the destination's aspect
/// ratio ("cover"): fills the slot without stretching.
fn cover_source_rect(w: u32, h: u32, dest_w: f32, dest_h: f32) -> RectF {
    let (w, h) = (w as f32, h as f32);
    if w <= 0.0 || h <= 0.0 || dest_w <= 0.0 || dest_h <= 0.0 {
        return RectF::new(0.0, 0.0, w.max(0.0), h.max(0.0));
    }
    let target = dest_w / dest_h;
    if w / h > target {
        let cw = h * target;
        let x = (w - cw) / 2.0;
        RectF::new(x, 0.0, x + cw, h)
    } else {
        let ch = w / target;
        let y = (h - ch) / 2.0;
        RectF::new(0.0, y, w, y + ch)
    }
}

impl Renderer {
    /// Sets the media content shown in the expanded state. Returns true if the
    /// visible content changed (caller decides whether to redraw). Fields not
    /// drawn yet (artwork, source app) are stored but never force a redraw, and
    /// UTF-16 is only re-encoded when the drawn text changes.
    pub fn set_media(&self, content: Option<&MediaContent>) -> bool {
        let mut media = self.media.borrow_mut();
        let changed = match (media.as_ref(), content) {
            (Some(m), Some(c)) => {
                m.content.title != c.title
                    || m.content.subtitle != c.subtitle
                    || m.content.icon != c.icon
                    || m.content.source_name() != c.source_name()
                    || m.content.artwork != c.artwork
                    || m.content.source.icon != c.source.icon
                    || m.content.shows_source_text() != c.shows_source_text()
                    || m.content.accent != c.accent
                    || m.content.timeline != c.timeline
                    || m.content.playback != c.playback
            }
            (None, None) => false,
            _ => true,
        };
        // A different track (text or artwork) on screen fades in; playback icon or
        // source-name changes stay crisp, and nothing fades from/to "no media".
        let new_track = matches!((media.as_ref(), content), (Some(m), Some(c))
            if m.content.title != c.title
                || m.content.subtitle != c.subtitle
                || m.content.artwork != c.artwork);
        if new_track {
            self.feedback.borrow_mut().start_track_fade();
        }
        match (media.as_mut(), content) {
            (Some(m), Some(c)) if !changed => m.content = c.clone(),
            _ => {
                *media = content.map(|c| MediaText {
                    content: c.clone(),
                    title: c.title.encode_utf16().collect(),
                    subtitle: c.subtitle.encode_utf16().collect(),
                    // Empty when the badge stands in for the source name
                    source: if c.shows_source_text() {
                        c.source_name().unwrap_or_default().encode_utf16().collect()
                    } else {
                        Vec::new()
                    },
                });
            }
        }
        // Release the device bitmap as soon as its artwork is no longer shown
        let shown_art = media.as_ref().and_then(|m| m.content.artwork.as_ref());
        let mut art = self.art_bitmap.borrow_mut();
        if art.as_ref().is_some_and(|(a, _)| Some(a) != shown_art) {
            *art = None;
        }
        let shown_badge = media.as_ref().and_then(|m| m.content.source.icon.as_ref());
        let mut badge = self.badge_bitmap.borrow_mut();
        if badge
            .as_ref()
            .is_some_and(|(b, _, _)| Some(b) != shown_badge)
        {
            *badge = None;
        }
        if media.is_none() {
            self.feedback.borrow_mut().reset();
        }
        self.visualizer
            .borrow_mut()
            .set_playing(content.is_some_and(|c| shows_visualizer(c.playback)));
        changed
    }

    /// Sets the space the selector shows; true if it changed (caller redraws).
    pub fn set_space(&self, space: NottSpace) -> bool {
        self.space.replace(space) != space
    }

    /// The animation frame's content blend / opacity (None: settled).
    pub fn set_transition(&self, transition: Option<Transition>) {
        self.transition.set(transition);
    }

    /// Mirrors the Settings page state and the settings values.
    pub fn set_settings(&self, open: bool, settings: NottSettings) {
        let was_open = self.settings_open.replace(open);
        let old = self.settings.replace(settings);
        // A value flipped while the page shows slides its switch (at once
        // under Reduced motion); a page that is hidden or just opening simply
        // shows the current values
        let instant = !crate::window::motion_allowed(
            settings.reduced_motion,
            crate::window::is_client_animation_enabled(),
        );
        let pairs = [
            (old.always_on_top, settings.always_on_top),
            (old.reduced_motion, settings.reduced_motion),
        ];
        for (anim, (was, now)) in self.switches.borrow_mut().iter_mut().zip(pairs) {
            if !open || !was_open {
                *anim = SwitchAnim::settled(now);
            } else if was != now {
                anim.set(now, instant);
            }
        }
    }

    /// Advances the Settings page's switches; true while any still moves.
    pub fn step_switches(&self, dt_ms: f32) -> bool {
        let mut any = false;
        for anim in self.switches.borrow_mut().iter_mut() {
            any |= anim.step(dt_ms);
        }
        any
    }

    /// Whether a Settings page switch is still sliding.
    pub fn switches_animating(&self) -> bool {
        self.switches.borrow().iter().any(SwitchAnim::is_animating)
    }

    /// Shows / hides the Clipboard space's clear confirmation (caller redraws).
    pub fn set_clear_pending(&self, pending: bool) {
        self.clear_pending.set(pending);
    }

    /// What this frame shows: the transition, or the settled scene at full
    /// opacity.
    fn scene_view(&self) -> Transition {
        self.transition.get().unwrap_or_else(|| {
            let scene = Scene::current(
                self.drop_page.get(),
                self.settings_open.get(),
                self.space.get(),
            );
            Transition {
                from: scene,
                to: scene,
                mix: 1.0,
                alpha: 1.0,
            }
        })
    }

    /// Whether the drop page is showing (the window sizes the notch for it).
    pub fn drop_page(&self) -> bool {
        self.drop_page.get()
    }

    /// Shows/hides the drop page; true if it changed (caller redraws).
    pub fn set_drop_page(&self, on: bool) -> bool {
        self.drop_page.replace(on) != on
    }

    /// Rebuilds the Clipboard space rows from the history (call on every
    /// history change; the caller redraws if the space is visible). Thumbnails
    /// of entries no longer shown are released.
    pub fn set_clipboard(&self, history: &ClipboardHistory) {
        let rows: Vec<ClipRow> = history
            .items()
            .take(CLIPBOARD_VISIBLE_ROWS)
            .map(|item| match item {
                ClipboardItem::Text(t) => ClipRow::Text(
                    crate::clipboard::preview(t, CLIPBOARD_PREVIEW_CHARS)
                        .encode_utf16()
                        .collect(),
                ),
                ClipboardItem::Image(a) => ClipRow::Image(
                    a.clone(),
                    format!("Image \u{00B7} {} \u{00D7} {}", a.width, a.height)
                        .encode_utf16()
                        .collect(),
                ),
            })
            .collect();
        self.thumbs.borrow_mut().retain(|(a, _, _)| {
            rows.iter()
                .any(|r| matches!(r, ClipRow::Image(b, _) if b == a))
        });
        *self.clipboard.borrow_mut() = ClipboardView { rows };
    }

    /// Playback visualizer state (window drives `step` while it is active).
    pub fn visualizer(&self) -> std::cell::RefMut<'_, Visualizer> {
        self.visualizer.borrow_mut()
    }

    /// Sets the hovered transport control. Returns true if it changed.
    pub fn set_hovered_control(&self, control: Option<MediaControl>) -> bool {
        self.feedback.borrow_mut().set_hovered(control)
    }

    /// Hover/press state of the transport controls (window drives it).
    pub fn feedback(&self) -> std::cell::RefMut<'_, ControlFeedback> {
        self.feedback.borrow_mut()
    }
}

impl Renderer {
    /// Creates a new Renderer instance initializing Direct2D and DirectWrite factories.
    pub fn new() -> Result<Self> {
        let d2d_factory: ID2D1Factory =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let dwrite_factory: IDWriteFactory =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let text_params = notch_text_params(&dwrite_factory)?;
        let round_stroke = unsafe {
            d2d_factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 1.0,
                    ..Default::default()
                },
                None,
            )?
        };
        let drop_stroke = unsafe {
            d2d_factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 1.0,
                    dashStyle: D2D1_DASH_STYLE_CUSTOM,
                    dashOffset: 0.0,
                },
                Some(&DROP_OUTLINE_DASHES),
            )?
        };
        Ok(Self {
            d2d_factory,
            dwrite_factory,
            cached_screen_dc: Cell::new(HDC::default()),
            cached_mem_dc: Cell::new(HDC::default()),
            cached_dib: Cell::new(HBITMAP::default()),
            cached_bits: Cell::new(std::ptr::null_mut()),
            cached_old_bmp: Cell::new(HGDIOBJ::default()),
            cached_rt: RefCell::new(None),
            cached_capacity_w: Cell::new(0),
            cached_capacity_h: Cell::new(0),
            scratch_a: RefCell::new(Vec::new()),
            scratch_b: RefCell::new(Vec::new()),
            shadow_key: Cell::new(None),
            media: RefCell::new(None),
            feedback: RefCell::new(ControlFeedback::default()),
            art_bitmap: RefCell::new(None),
            badge_bitmap: RefCell::new(None),
            media_formats: RefCell::new(None),
            clock_format: RefCell::new(None),
            visualizer: RefCell::new(Visualizer::default()),
            epoch: std::time::Instant::now(),
            space: Cell::new(NottSpace::default()),
            clipboard: RefCell::new(ClipboardView::default()),
            round_stroke,
            thumbs: RefCell::new(Vec::new()),
            drop_page: Cell::new(false),
            transition: Cell::new(None),
            settings_open: Cell::new(false),
            settings: Cell::new(NottSettings::default()),
            clear_pending: Cell::new(false),
            switches: RefCell::new({
                let s = NottSettings::default();
                [
                    SwitchAnim::settled(s.always_on_top),
                    SwitchAnim::settled(s.reduced_motion),
                ]
            }),
            drop_stroke,
            text_params,
        })
    }

    /// Releases cached GDI surface and Direct2D render target resources.
    fn cleanup_cached_resources(&self) {
        unsafe {
            // Device bitmaps belong to the render target being released
            *self.art_bitmap.borrow_mut() = None;
            *self.badge_bitmap.borrow_mut() = None;
            self.thumbs.borrow_mut().clear();
            *self.cached_rt.borrow_mut() = None;
            let mem_dc = self.cached_mem_dc.get();
            if !mem_dc.is_invalid() {
                let old_bmp = self.cached_old_bmp.get();
                if !old_bmp.is_invalid() {
                    let _ = SelectObject(mem_dc, old_bmp);
                    self.cached_old_bmp.set(HGDIOBJ::default());
                }
                let dib = self.cached_dib.get();
                if !dib.is_invalid() {
                    let _ = DeleteObject(dib.into());
                    self.cached_dib.set(HBITMAP::default());
                }
                let _ = DeleteDC(mem_dc);
                self.cached_mem_dc.set(HDC::default());
            }
            let screen_dc = self.cached_screen_dc.get();
            if !screen_dc.is_invalid() {
                let _ = ReleaseDC(None, screen_dc);
                self.cached_screen_dc.set(HDC::default());
            }
            self.cached_bits.set(std::ptr::null_mut());
            self.cached_capacity_w.set(0);
            self.cached_capacity_h.set(0);
        }
    }

    /// Pre-allocates and caches high-performance GDI and Direct2D surfaces to guarantee
    /// zero-allocation, 60+ FPS frame rendering without stutter.
    fn ensure_buffer(&self, req_w: i32, req_h: i32) -> Result<()> {
        if self.cached_capacity_w.get() >= req_w
            && self.cached_capacity_h.get() >= req_h
            && self.cached_rt.borrow().is_some()
        {
            return Ok(());
        }

        let alloc_w = req_w.max(1200);
        let alloc_h = req_h.max(600);

        unsafe {
            self.cleanup_cached_resources();

            let screen_dc = GetDC(None);
            if screen_dc.is_invalid() {
                return Err(Error::from_thread());
            }

            let mem_dc = CreateCompatibleDC(Some(screen_dc));
            if mem_dc.is_invalid() {
                let _ = ReleaseDC(None, screen_dc);
                return Err(Error::from_thread());
            }

            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: alloc_w,
                    biHeight: -alloc_h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                },
                bmiColors: [RGBQUAD::default()],
            };

            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib_result =
                CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
            let dib = match dib_result {
                Ok(hbitmap) => {
                    if hbitmap.is_invalid() || bits.is_null() {
                        let _ = DeleteDC(mem_dc);
                        let _ = ReleaseDC(None, screen_dc);
                        return Err(Error::from_thread());
                    }
                    hbitmap
                }
                Err(e) => {
                    let _ = DeleteDC(mem_dc);
                    let _ = ReleaseDC(None, screen_dc);
                    return Err(e);
                }
            };

            let old_bitmap = SelectObject(mem_dc, dib.into());

            let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 0.0,
                dpiY: 0.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };

            let rt = self.d2d_factory.CreateDCRenderTarget(&rt_props)?;
            // Greyscale is what a transparent, premultiplied layered surface
            // can blend correctly (ClearType would fall back to it anyway);
            // the params keep outlines true instead of grid-snapped
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            rt.SetTextRenderingParams(&self.text_params);

            self.cached_screen_dc.set(screen_dc);
            self.cached_mem_dc.set(mem_dc);
            self.cached_dib.set(dib);
            self.cached_bits.set(bits);
            self.cached_old_bmp.set(old_bitmap);
            *self.cached_rt.borrow_mut() = Some(rt);
            self.cached_capacity_w.set(alloc_w);
            self.cached_capacity_h.set(alloc_h);
        }

        Ok(())
    }

    /// A DirectWrite text format in Nott's typeface (`ui_font`).
    fn create_text_format(
        &self,
        font_size: f32,
        weight: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT,
        style: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_STYLE,
    ) -> Result<IDWriteTextFormat> {
        self.create_text_format_in(ui_font(&self.dwrite_factory).1, font_size, weight, style)
    }

    /// A role's single-line format at `scale`, aligned `align`.
    fn role_format(
        &self,
        role: TextRole,
        scale: f32,
        align: DWRITE_TEXT_ALIGNMENT,
    ) -> Result<IDWriteTextFormat> {
        let style = role.style();
        self.create_line_format(
            ui_font(&self.dwrite_factory).1,
            style.size * scale,
            DWRITE_FONT_WEIGHT(style.weight),
            align,
        )
    }

    /// Text format in `family`, falling back to Segoe UI if it is not installed.
    fn create_text_format_in(
        &self,
        family: windows::core::PCWSTR,
        font_size: f32,
        weight: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT,
        style: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_STYLE,
    ) -> Result<IDWriteTextFormat> {
        let locale: Vec<u16> = "en-us\0".encode_utf16().collect();
        let format_result = unsafe {
            self.dwrite_factory.CreateTextFormat(
                family,
                None,
                weight,
                style,
                DWRITE_FONT_STRETCH_NORMAL,
                font_size,
                windows::core::PCWSTR(locale.as_ptr()),
            )
        };

        // A missing family does not fail here (DirectWrite substitutes at
        // draw time), so the family is resolved beforehand (`ui_font`)
        format_result
    }

    /// Mathematically generates a hardware-notch path geometry with smooth
    /// upper shoulder concave-corner curvature flowing seamlessly into the top display border
    /// and rounded lower corners.
    pub fn create_notch_geometry(&self, dimensions: &NotchDimensions) -> Result<ID2D1PathGeometry> {
        let pad_x = dimensions.shadow_margin_x;
        let notch_w = dimensions.notch_width();
        let notch_h = dimensions.notch_height();
        let r_top_x = dimensions
            .curvature
            .top_transition_radius
            .max(0.0)
            .min(notch_h / 2.0)
            .min(notch_w / 4.0);
        let r_top_y = dimensions
            .curvature
            .top_transition_height
            .max(0.0)
            .min(notch_h / 2.0)
            .min(notch_w / 4.0);
        // Bottom corners: circular when collapsed, continuous ("squircle") when
        // expanded; the same profile drives hit testing.
        let corner = dimensions.bottom_corner_profile();
        let r_bottom = corner.span;
        let k_bottom = corner.handle;
        let border_width = dimensions.border_width;
        let half_border = border_width / 2.0;

        let path = unsafe { self.d2d_factory.CreatePathGeometry()? };
        let sink = unsafe { path.Open()? };
        let raw_sink = windows::core::Interface::as_raw(&sink);
        let helper = unsafe { GeometrySinkHelper::from_raw(raw_sink) };

        // Standard cubic Bezier quarter-circle constant (0.5522848)
        let kb = 0.5522848f32;
        let (c1, c2) = (kb, 1.0f32 - kb);

        unsafe {
            let w = pad_x + notch_w - half_border;
            let h = notch_h - half_border;
            let start_x = pad_x + half_border;
            let start_y = 0.0f32; // Anchored at physical screen edge y=0

            // Start at top-left ear attached to screen edge (y = 0)
            helper.begin_figure(D2D_POINT_2F {
                x: start_x,
                y: start_y,
            });

            // 1. Top-left concave shoulder: curves from (start_x, start_y) down to (start_x + r_top_x, start_y + r_top_y)
            if r_top_x > 0.0 && r_top_y > 0.0 {
                helper.add_bezier(&D2D1_BEZIER_SEGMENT {
                    point1: D2D_POINT_2F {
                        x: start_x + c1 * r_top_x,
                        y: start_y,
                    },
                    point2: D2D_POINT_2F {
                        x: start_x + r_top_x,
                        y: start_y + c2 * r_top_y,
                    },
                    point3: D2D_POINT_2F {
                        x: start_x + r_top_x,
                        y: start_y + r_top_y,
                    },
                });
            } else {
                helper.add_line(D2D_POINT_2F {
                    x: start_x + r_top_x,
                    y: start_y + r_top_y,
                });
            }

            // 2. Left vertical wall — from shoulder down to bottom corner start
            helper.add_line(D2D_POINT_2F {
                x: start_x + r_top_x,
                y: h - r_bottom,
            });

            // 3. Bottom-left convex rounded corner
            if r_bottom > 0.0 {
                helper.add_bezier(&D2D1_BEZIER_SEGMENT {
                    point1: D2D_POINT_2F {
                        x: start_x + r_top_x,
                        y: h - r_bottom * (1.0 - k_bottom),
                    },
                    point2: D2D_POINT_2F {
                        x: start_x + r_top_x + r_bottom * (1.0 - k_bottom),
                        y: h,
                    },
                    point3: D2D_POINT_2F {
                        x: start_x + r_top_x + r_bottom,
                        y: h,
                    },
                });
            }

            // 4. Bottom horizontal edge
            helper.add_line(D2D_POINT_2F {
                x: w - r_top_x - r_bottom,
                y: h,
            });

            // 5. Bottom-right convex rounded corner
            if r_bottom > 0.0 {
                helper.add_bezier(&D2D1_BEZIER_SEGMENT {
                    point1: D2D_POINT_2F {
                        x: w - r_top_x - r_bottom + r_bottom * k_bottom,
                        y: h,
                    },
                    point2: D2D_POINT_2F {
                        x: w - r_top_x,
                        y: h - r_bottom * (1.0 - k_bottom),
                    },
                    point3: D2D_POINT_2F {
                        x: w - r_top_x,
                        y: h - r_bottom,
                    },
                });
            }

            // 6. Right vertical wall — up to shoulder endpoint
            helper.add_line(D2D_POINT_2F {
                x: w - r_top_x,
                y: start_y + r_top_y,
            });

            // 7. Top-right concave shoulder: curves from (w - r_top_x, start_y + r_top_y) up to (w, start_y)
            if r_top_x > 0.0 && r_top_y > 0.0 {
                helper.add_bezier(&D2D1_BEZIER_SEGMENT {
                    point1: D2D_POINT_2F {
                        x: w - r_top_x,
                        y: start_y + c2 * r_top_y,
                    },
                    point2: D2D_POINT_2F {
                        x: w - c1 * r_top_x,
                        y: start_y,
                    },
                    point3: D2D_POINT_2F { x: w, y: start_y },
                });
            } else {
                helper.add_line(D2D_POINT_2F { x: w, y: start_y });
            }

            // 8. Top edge: runs all the way left back to left screen edge (y=0 attachment)
            helper.add_line(D2D_POINT_2F {
                x: start_x,
                y: start_y,
            });

            helper.end_figure();
            helper.close()?;
        }

        Ok(path)
    }

    /// Mathematically generates an open notch shadow path along the shoulders, side walls,
    /// and bottom corners/edge, with an optional vertical drop offset `dy`.
    #[allow(dead_code)]
    pub fn create_notch_shadow_path(
        &self,
        dimensions: &NotchDimensions,
        dy: f32,
    ) -> Result<ID2D1PathGeometry> {
        let pad_x = dimensions.shadow_margin_x;
        let notch_w = dimensions.notch_width();
        let notch_h = dimensions.notch_height();
        let r_top_x = dimensions
            .curvature
            .top_transition_radius
            .max(0.0)
            .min(notch_h / 2.0)
            .min(notch_w / 4.0);
        let r_top_y = dimensions
            .curvature
            .top_transition_height
            .max(0.0)
            .min(notch_h / 2.0)
            .min(notch_w / 4.0);
        // Bottom corners: circular when collapsed, continuous ("squircle") when
        // expanded; the same profile drives hit testing.
        let corner = dimensions.bottom_corner_profile();
        let r_bottom = corner.span;
        let k_bottom = corner.handle;
        let border_width = dimensions.border_width;
        let half_border = border_width / 2.0;

        let path = unsafe { self.d2d_factory.CreatePathGeometry()? };
        let sink = unsafe { path.Open()? };
        let raw_sink = windows::core::Interface::as_raw(&sink);
        let helper = unsafe { GeometrySinkHelper::from_raw(raw_sink) };

        let kb = 0.5522848f32;
        let (c1, c2) = (kb, 1.0f32 - kb);

        unsafe {
            let start_x = pad_x + half_border;
            let start_y = dy;
            let w = pad_x + notch_w - half_border;
            let h = notch_h - half_border + dy;

            // Start at top-left ear attached to screen edge
            helper.begin_figure(D2D_POINT_2F {
                x: start_x,
                y: start_y,
            });

            // 1. Top-left concave shoulder
            if r_top_x > 0.0 && r_top_y > 0.0 {
                helper.add_bezier(&D2D1_BEZIER_SEGMENT {
                    point1: D2D_POINT_2F {
                        x: start_x + c1 * r_top_x,
                        y: start_y,
                    },
                    point2: D2D_POINT_2F {
                        x: start_x + r_top_x,
                        y: start_y + c2 * r_top_y,
                    },
                    point3: D2D_POINT_2F {
                        x: start_x + r_top_x,
                        y: start_y + r_top_y,
                    },
                });
            } else {
                helper.add_line(D2D_POINT_2F {
                    x: start_x + r_top_x,
                    y: start_y + r_top_y,
                });
            }

            // 2. Left vertical wall
            helper.add_line(D2D_POINT_2F {
                x: start_x + r_top_x,
                y: h - r_bottom,
            });

            // 3. Bottom-left convex rounded corner
            if r_bottom > 0.0 {
                helper.add_bezier(&D2D1_BEZIER_SEGMENT {
                    point1: D2D_POINT_2F {
                        x: start_x + r_top_x,
                        y: h - r_bottom * (1.0 - k_bottom),
                    },
                    point2: D2D_POINT_2F {
                        x: start_x + r_top_x + r_bottom * (1.0 - k_bottom),
                        y: h,
                    },
                    point3: D2D_POINT_2F {
                        x: start_x + r_top_x + r_bottom,
                        y: h,
                    },
                });
            }

            // 4. Bottom horizontal edge
            helper.add_line(D2D_POINT_2F {
                x: w - r_top_x - r_bottom,
                y: h,
            });

            // 5. Bottom-right convex rounded corner
            if r_bottom > 0.0 {
                helper.add_bezier(&D2D1_BEZIER_SEGMENT {
                    point1: D2D_POINT_2F {
                        x: w - r_top_x - r_bottom + r_bottom * k_bottom,
                        y: h,
                    },
                    point2: D2D_POINT_2F {
                        x: w - r_top_x,
                        y: h - r_bottom * (1.0 - k_bottom),
                    },
                    point3: D2D_POINT_2F {
                        x: w - r_top_x,
                        y: h - r_bottom,
                    },
                });
            }

            // 6. Right vertical wall
            helper.add_line(D2D_POINT_2F {
                x: w - r_top_x,
                y: start_y + r_top_y,
            });

            // 7. Top-right concave shoulder: curves up to screen edge
            if r_top_x > 0.0 && r_top_y > 0.0 {
                helper.add_bezier(&D2D1_BEZIER_SEGMENT {
                    point1: D2D_POINT_2F {
                        x: w - r_top_x,
                        y: start_y + c2 * r_top_y,
                    },
                    point2: D2D_POINT_2F {
                        x: w - c1 * r_top_x,
                        y: start_y,
                    },
                    point3: D2D_POINT_2F { x: w, y: start_y },
                });
            } else {
                helper.add_line(D2D_POINT_2F { x: w, y: start_y });
            }

            // Open figure: ends at right screen attachment without crossing the top
            helper.end_figure_open();
            helper.close()?;
        }

        Ok(path)
    }
}

/// Applies a soft drop shadow beneath the notch silhouette into `bits`.
///
/// The notch alpha is shifted down slightly and Gaussian-blurred (3-pass
/// separable box blur), then shaped so it reads as a premium drop shadow rather
/// than an outline:
/// - cast by the straight-walled body only: nothing on the concave top shoulders
///   (the notch meets the screen edge there), full strength right below them;
/// - edge fade: reaches zero before the left/right/bottom window edges, so the
///   shadow is never clipped by the (unchanged) shadow margins;
/// - composited only outside the opaque notch body.
///
/// (The renderer uses `build_shadow_mask` + `composite_shadow` directly so the
/// mask can be cached; this one-shot form is kept for tests.)
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn apply_ambient_shadow(
    bits: *mut u8,
    width: usize,
    height: usize,
    stride: usize,
    scale: f32,
    shadow_opacity: f32,
    scratch_a: &mut Vec<u8>,
    scratch_b: &mut Vec<u8>,
) {
    if build_shadow_mask(
        bits,
        width,
        height,
        stride,
        scale,
        shadow_opacity,
        scratch_a,
        scratch_b,
    ) {
        composite_shadow(bits, width, height, stride, scratch_a);
    }
}

/// Steps 1-3 of `apply_ambient_shadow`: leaves the final shadow alpha of every
/// pixel in `scratch_a` (width x height). It depends only on the notch
/// silhouette, so a settled notch reuses it across frames. False if none.
#[allow(clippy::too_many_arguments)]
fn build_shadow_mask(
    bits: *mut u8,
    width: usize,
    height: usize,
    stride: usize,
    scale: f32,
    shadow_opacity: f32,
    scratch_a: &mut Vec<u8>,
    scratch_b: &mut Vec<u8>,
) -> bool {
    if width == 0 || height == 0 || shadow_opacity <= 0.0 || bits.is_null() {
        return false;
    }

    let total_pixels = width * height;
    if scratch_a.len() < total_pixels {
        scratch_a.resize(total_pixels, 0);
    }
    if scratch_b.len() < total_pixels {
        scratch_b.resize(total_pixels, 0);
    }

    let alpha_at = |x: usize, y: usize| unsafe { *bits.add((y * stride + x) * 4 + 3) };
    let first_lit = |y: usize| (0..width).find(|&x| alpha_at(x, y) > 0);
    // Bottom of the opaque body (center column)
    let body_bottom = (0..height)
        .rev()
        .find(|&y| alpha_at(width / 2, y) == 255)
        .map_or(0, |y| y + 1);
    // Straight side walls (at mid-height) and the row where the concave top
    // shoulders end: only the straight-walled body casts the shadow, so none of
    // it lands on the shoulder curves.
    let mid = body_bottom / 2;
    let wall_left = first_lit(mid).unwrap_or(0);
    let wall_right = (0..width)
        .rev()
        .find(|&x| alpha_at(x, mid) > 0)
        .unwrap_or(width - 1);
    let shoulder_end = (0..mid)
        .find(|&y| first_lit(y).is_some_and(|x| x >= wall_left))
        .unwrap_or(0);

    // 1. Extract the body alpha shifted down by `offset` rows (light from above)
    let offset = ((SHADOW_OFFSET_Y * scale).round() as usize).min(height);
    unsafe {
        for y in 0..height {
            let target_offset = y * width;
            if y < offset {
                scratch_a[target_offset..target_offset + width].fill(0);
            } else {
                let src_row = (y - offset) * stride * 4;
                for x in 0..width {
                    scratch_a[target_offset + x] = if (wall_left..=wall_right).contains(&x) {
                        *bits.add(src_row + x * 4 + 3)
                    } else {
                        0
                    };
                }
            }
        }
    }

    // 2. 3-pass separable box blur (Gaussian approximation), radius scales with DPI
    let radius = ((SHADOW_BLUR_RADIUS * scale).round() as usize).max(1);
    for _ in 0..3 {
        box_blur_horizontal(
            &scratch_a[..total_pixels],
            &mut scratch_b[..total_pixels],
            width,
            height,
            radius,
        );
        box_blur_vertical(
            &scratch_b[..total_pixels],
            &mut scratch_a[..total_pixels],
            width,
            height,
            radius,
        );
    }

    // 3. Shape: shoulder, bottom and side weighting
    let peak = (shadow_opacity * SHADOW_STRENGTH).min(1.0);
    let edge_fade = (SHADOW_EDGE_FADE * scale).max(1.0);
    let smoothstep = |e0: f32, e1: f32, v: f32| {
        let t = ((v - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };

    for y in 0..height {
        let row = &mut scratch_a[y * width..(y + 1) * width];
        // None on the concave shoulders; full strength right below them
        let vertical = smoothstep(
            shoulder_end as f32,
            shoulder_end as f32 + (SHADOW_SHOULDER_FADE * scale).max(1.0),
            y as f32,
        );
        let bottom_fade = smoothstep(0.0, edge_fade, (height - 1 - y) as f32);
        for (x, v) in row.iter_mut().enumerate() {
            let side = x.min(width - 1 - x) as f32;
            let weight = peak * vertical * bottom_fade * smoothstep(0.0, edge_fade, side);
            *v = (f32::from(*v) * weight).round().min(255.0) as u8;
        }
    }
    true
}

/// Composites a shadow mask underneath the notch: only outside the opaque body
/// (premultiplied black, so alpha only).
fn composite_shadow(bits: *mut u8, width: usize, height: usize, stride: usize, mask: &[u8]) {
    for y in 0..height {
        for x in 0..width {
            let shadow_a = u32::from(mask[y * width + x]);
            if shadow_a == 0 {
                continue;
            }
            unsafe {
                let alpha = bits.add((y * stride + x) * 4 + 3);
                let orig_a = u32::from(*alpha);
                if orig_a < 255 {
                    // Final_A = Orig_A + Shadow_A * (255 - Orig_A) / 255
                    *alpha = (orig_a + (shadow_a * (255 - orig_a) + 127) / 255).min(255) as u8;
                }
            }
        }
    }
}

/// Drop-shadow shaping (DIP at 96 DPI / unitless). Lives entirely inside the
/// existing expanded shadow margins; the notch geometry and window size are unchanged.
const SHADOW_OFFSET_Y: f32 = 3.0;
const SHADOW_BLUR_RADIUS: f32 = 5.0;
const SHADOW_EDGE_FADE: f32 = 4.0;
/// The side shadow fades in gently over this distance below the concave top
/// shoulders, so it never starts with a visible edge.
const SHADOW_SHOULDER_FADE: f32 = 28.0;
/// Multiplier on the per-state shadow opacity (0.28 expanded -> ~0.78 peak).
const SHADOW_STRENGTH: f32 = 2.8;
#[inline]
fn box_blur_horizontal(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize) {
    let window_size = (radius * 2 + 1) as u32;

    for y in 0..height {
        let row_offset = y * width;
        let mut sum: u32 = 0;

        for x_rel in -(radius as isize)..=(radius as isize) {
            let clamped_x = x_rel.clamp(0, (width - 1) as isize) as usize;
            sum += src[row_offset + clamped_x] as u32;
        }
        dst[row_offset] = ((sum + window_size / 2) / window_size) as u8;

        for x in 1..width {
            let add_x = (x + radius).min(width - 1);
            let sub_x = (x as isize - radius as isize - 1).max(0) as usize;
            sum += src[row_offset + add_x] as u32;
            sum -= src[row_offset + sub_x] as u32;
            dst[row_offset + x] = ((sum + window_size / 2) / window_size) as u8;
        }
    }
}

#[inline]
fn box_blur_vertical(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize) {
    let window_size = (radius * 2 + 1) as u32;

    for x in 0..width {
        let mut sum: u32 = 0;

        for y_rel in -(radius as isize)..=(radius as isize) {
            let clamped_y = y_rel.clamp(0, (height - 1) as isize) as usize;
            sum += src[clamped_y * width + x] as u32;
        }
        dst[x] = ((sum + window_size / 2) / window_size) as u8;

        for y in 1..height {
            let add_y = (y + radius).min(height - 1);
            let sub_y = (y as isize - radius as isize - 1).max(0) as usize;
            sum += src[add_y * width + x] as u32;
            sum -= src[sub_y * width + x] as u32;
            dst[y * width + x] = ((sum + window_size / 2) / window_size) as u8;
        }
    }
}

impl Renderer {
    /// Renders the components of the collapsed notch: centered live Windows time
    fn render_collapsed_content(
        &self,
        rt: &ID2D1RenderTarget,
        layout: &CollapsedLayout,
        dimensions: &NotchDimensions,
        formatted_time: &str,
    ) -> Result<()> {
        let clip_rect = layout.content_bounds.to_d2d_rect();
        unsafe {
            rt.PushAxisAlignedClip(&clip_rect, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }

        let key = dimensions.font_size_clock.to_bits();
        if self
            .clock_format
            .borrow()
            .as_ref()
            .is_none_or(|(k, _)| *k != key)
        {
            let format = self.create_text_format(
                dimensions.font_size_clock,
                DWRITE_FONT_WEIGHT(TextRole::ClockCompact.style().weight),
                DWRITE_FONT_STYLE_NORMAL,
            )?;
            unsafe {
                format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
                format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            }
            *self.clock_format.borrow_mut() = Some((key, format));
        }
        let clock_format = std::cell::Ref::map(self.clock_format.borrow(), |f| {
            &f.as_ref().expect("clock format just ensured").1
        });
        let time_utf16: Vec<u16> = formatted_time.encode_utf16().collect();
        let time_rect = layout.clock_bounds.to_d2d_rect();
        let brush = unsafe { rt.CreateSolidColorBrush(&COLOR_TEXT_PRIMARY, None)? };
        unsafe {
            rt.DrawText(
                &time_utf16,
                &*clock_format,
                &time_rect,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
        let accent = accent_color(
            self.media.borrow().as_ref().map(|m| &m.content),
            self.settings.get().accent,
        );
        self.draw_visualizer(rt, layout.visualizer_bounds, accent, dimensions.scale, 1.0)?;
        unsafe { rt.PopAxisAlignedClip() };

        Ok(())
    }

    /// Space selector: two capsules above the expanded content. The active space
    /// gets a quiet filled capsule and primary text; the other is text only.
    /// The active space's selector laid out on `dimensions` as they are.
    #[cfg(test)]
    fn draw_space_selector(
        &self,
        rt: &ID2D1RenderTarget,
        dimensions: &NotchDimensions,
    ) -> Result<()> {
        let active = self.space.get();
        let Some(selector) = crate::layout::resolve_space_selector_in(dimensions, active) else {
            return Ok(());
        };
        self.paint_selector(rt, dimensions, &selector, selector.bounds(active), 1.0)
    }

    /// The selector between two spaces (see `blended_selector`): pills glide,
    /// the highlight slides from the outgoing to the incoming space's pill.
    fn draw_selector_blended(
        &self,
        rt: &ID2D1RenderTarget,
        dimensions: &NotchDimensions,
        from: Scene,
        to: Scene,
        mix: f32,
        alpha: f32,
    ) -> Result<()> {
        let blended = blended_selector(dimensions, from, to, self.space.get(), mix);
        let Some((selector, highlight)) = blended else {
            return Ok(());
        };
        self.paint_selector(rt, dimensions, &selector, highlight, alpha)
    }

    /// Pills (white solid icons) with the highlight capsule behind one place.
    fn paint_selector(
        &self,
        rt: &ID2D1RenderTarget,
        dimensions: &NotchDimensions,
        selector: &crate::layout::SpaceSelectorLayout,
        highlight: RectF,
        alpha: f32,
    ) -> Result<()> {
        let s = dimensions.scale;
        let u = (BASE_SPACE_ICON_SIZE * s).round();
        unsafe {
            let fill = rt.CreateSolidColorBrush(&COLOR_SPACE_PILL_SELECTED, None)?;
            fill.SetOpacity(alpha);
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: highlight.to_d2d_rect(),
                    radiusX: highlight.height() / 2.0,
                    radiusY: highlight.height() / 2.0,
                },
                &fill,
            );
            let white = rt.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                None,
            )?;
            white.SetOpacity(alpha);
            for space in NottSpace::ALL {
                let r = selector.bounds(space);
                // White glyphs; the active space is marked by its pill
                let brush = &white;
                let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
                self.draw_space_icon(rt, space, cx, cy, u, brush)?;
            }
            // Settings icon at the right end: same glyph size, grows a little
            // on hover / press like the other icon buttons
            let (hover, press) = self.feedback.borrow().clip_levels(PanelHit::Settings);
            let grow = 1.0
                + crate::config::CLIPBOARD_BUTTON_HOVER_GROW * hover
                + crate::config::CLIPBOARD_BUTTON_PRESS_GROW * press;
            let r = selector.settings;
            let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
            self.draw_glyph(rt, GEAR, cx, cy, u * grow, &white)?;
        }
        Ok(())
    }

    /// Space icons (Flaticon UIcons solid-rounded, see `HOUSE_BLANK` /
    /// `MUSIC_ALT`), filled in a `u`-sized box centred on (cx, cy).
    fn draw_space_icon(
        &self,
        rt: &ID2D1RenderTarget,
        space: NottSpace,
        cx: f32,
        cy: f32,
        u: f32,
        brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
    ) -> Result<()> {
        let segments = match space {
            NottSpace::Home => HOUSE_BLANK,
            NottSpace::Music => MUSIC_ALT,
            NottSpace::Clipboard => CLIPBOARD,
        };
        self.draw_glyph(rt, segments, cx, cy, u, brush)
    }

    /// Fills a UIcons glyph (`IconSeg` path) in a `u`-sized box centred on (cx, cy).
    fn draw_glyph(
        &self,
        rt: &ID2D1RenderTarget,
        segments: &[IconSeg],
        cx: f32,
        cy: f32,
        u: f32,
        brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
    ) -> Result<()> {
        fill_glyph(&self.d2d_factory, rt, segments, cx, cy, u, brush)
    }

    /// Music space without a media session: a single quiet centered label (no
    /// stale artwork/text/accent can show: there is no media content at all).
    fn draw_music_empty(
        &self,
        rt: &ID2D1RenderTarget,
        layout: &ExpandedLayout,
        dimensions: &NotchDimensions,
    ) -> Result<()> {
        let formats = self.media_formats(dimensions)?;
        let text: Vec<u16> = MUSIC_EMPTY_LABEL.encode_utf16().collect();
        unsafe {
            let brush = rt.CreateSolidColorBrush(&COLOR_TEXT_TERTIARY, None)?;
            rt.DrawText(
                &text,
                &formats.empty,
                &layout.content_bounds.to_d2d_rect(),
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
        Ok(())
    }

    /// Drop page, a fresh surface while an image is dragged over the notch: a
    /// rounded dashed outline inset from the notch body (same corner profile as
    /// the notch) with the inbox glyph centred in it. Nothing else is drawn.
    /// The page is always laid out for its one universal size (the Clipboard
    /// notch) and clipped to the current silhouette, so while the notch opens
    /// or resizes it is revealed in place: the dashes never re-flow or crawl.
    fn draw_drop_page(&self, rt: &ID2D1RenderTarget, dimensions: &NotchDimensions) -> Result<()> {
        let target = drop_page_dimensions(dimensions.dpi);
        let px = |v: f32| (v * target.scale).round();
        let wall = target.shadow_margin_x + target.curvature.top_transition_radius;
        let inset = px(BASE_DROP_OUTLINE_INSET);
        let width = px(BASE_DROP_OUTLINE_WIDTH).max(1.0);
        // Both windows are centred on the screen: align the page's centre
        let dx = ((dimensions.width - target.width) as f32 / 2.0).round();
        // Stroke centred half a width inside the outline box
        let half = width / 2.0;
        let outline = RectF::new(
            dx + wall + inset + half,
            inset + half,
            dx + target.width as f32 - wall - inset - half,
            target.height as f32 - target.shadow_margin_bottom - inset - half,
        );
        if outline.width() <= 0.0 || outline.height() <= 0.0 {
            return Ok(());
        }
        let profile = CornerProfile::new(
            px(BASE_DROP_OUTLINE_RADIUS),
            1.0,
            outline.width().min(outline.height()) / 2.0,
        );
        let path = self.smooth_rect_path(outline, profile.span, profile.handle)?;
        let silhouette = self.create_notch_geometry(dimensions)?;
        self.with_layer(rt, Some(&silhouette), 1.0, || unsafe {
            let dashes = rt.CreateSolidColorBrush(&COLOR_DROP_OUTLINE, None)?;
            rt.DrawGeometry(&path, &dashes, width, &self.drop_stroke);
            let icon = rt.CreateSolidColorBrush(&COLOR_DROP_ICON, None)?;
            let (cx, cy) = (
                (outline.left + outline.right) / 2.0,
                (outline.top + outline.bottom) / 2.0,
            );
            self.draw_glyph(rt, INBOX, cx, cy, px(BASE_DROP_ICON_SIZE), &icon)
        })
    }

    /// Runs `draw` inside a layer clipped to `mask` (if any) at `opacity`.
    /// Without a mask at full opacity it draws directly (settled frames).
    fn with_layer(
        &self,
        rt: &ID2D1RenderTarget,
        mask: Option<&ID2D1PathGeometry>,
        opacity: f32,
        draw: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        if mask.is_none() && opacity >= 1.0 {
            return draw();
        }
        let mut layer = D2D1_LAYER_PARAMETERS {
            contentBounds: windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
                left: -f32::MAX,
                top: -f32::MAX,
                right: f32::MAX,
                bottom: f32::MAX,
            },
            geometricMask: std::mem::ManuallyDrop::new(match mask {
                Some(m) => Some(windows::core::Interface::cast(m)?),
                None => None,
            }),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            opacity: opacity.clamp(0.0, 1.0),
            ..Default::default()
        };
        // Identity mask transform
        layer.maskTransform.M11 = 1.0;
        layer.maskTransform.M22 = 1.0;
        unsafe {
            rt.PushLayer(
                &layer,
                None::<&windows::Win32::Graphics::Direct2D::ID2D1Layer>,
            );
        }
        let drawn = draw();
        unsafe { rt.PopLayer() };
        // Release the mask reference the layer parameters hold
        drop(std::mem::ManuallyDrop::into_inner(layer.geometricMask));
        drawn
    }

    /// Draws everything after this shifted right by `dx` (0 resets).
    fn set_offset(rt: &ID2D1RenderTarget, dx: f32) {
        let mut m = D2D1_LAYER_PARAMETERS::default().maskTransform;
        m.M11 = 1.0;
        m.M22 = 1.0;
        m.M31 = dx;
        unsafe { rt.SetTransform(&m) };
    }

    /// The expanded notch's content this frame. Settled: the active scene,
    /// directly. Animating: clipped to the notch silhouette at the content
    /// opacity; during a space change the outgoing and incoming scenes blend
    /// (Home <-> Music with media morph as one composition, the rest
    /// cross-fade). Every scene keeps its settled layout, centred in the
    /// window, so the shell morphs around content instead of pushing it.
    fn draw_expanded(
        &self,
        rt: &ID2D1RenderTarget,
        dims: &NotchDimensions,
        clock: &ClockDateState,
    ) -> Result<()> {
        let t = self.scene_view();
        if t.alpha <= 0.0 {
            return Ok(());
        }
        let blending = t.from != t.to;
        let mask = if blending || t.alpha < 1.0 {
            Some(self.create_notch_geometry(dims)?)
        } else {
            None
        };
        self.with_layer(rt, mask.as_ref(), t.alpha, || {
            let weight = |s: Scene| {
                if !blending {
                    f32::from(u8::from(s == t.to))
                } else if s == t.to {
                    t.mix
                } else if s == t.from {
                    1.0 - t.mix
                } else {
                    0.0
                }
            };
            let selector_alpha = 1.0 - weight(Scene::Drop);
            if selector_alpha > 0.0 {
                let mix = if blending { t.mix } else { 1.0 };
                self.draw_selector_blended(rt, dims, t.from, t.to, mix, selector_alpha)?;
            }
            if !blending {
                return self.draw_scene(rt, dims, clock, t.to);
            }
            // Home <-> Music with a session: one composition morphing
            if let (Scene::Space(a), Scene::Space(b)) = (t.from, t.to)
                && a != NottSpace::Clipboard
                && b != NottSpace::Clipboard
                && self.media.borrow().is_some()
            {
                let music = if b == NottSpace::Music {
                    t.mix
                } else {
                    1.0 - t.mix
                };
                return self.draw_media_morph(rt, dims, clock, music);
            }
            for (scene, w) in [(t.from, 1.0 - t.mix), (t.to, t.mix)] {
                let a = scene_fade(w);
                if a > 0.0 {
                    self.with_layer(rt, None, a, || self.draw_scene(rt, dims, clock, scene))?;
                }
            }
            Ok(())
        })
    }

    /// One scene at its settled layout, centred in the current window.
    fn draw_scene(
        &self,
        rt: &ID2D1RenderTarget,
        dims: &NotchDimensions,
        clock: &ClockDateState,
        scene: Scene,
    ) -> Result<()> {
        let (space, settings) = match scene {
            Scene::Drop => return self.draw_drop_page(rt, dims),
            Scene::Settings => (PANEL_SPACE, true),
            Scene::Space(space) => (space, false),
        };
        let settled = space_dimensions(crate::config::NotchState::Expanded, dims.dpi, space);
        let ResolvedLayout::Expanded { components, .. } = resolve_layout(&settled) else {
            return Ok(());
        };
        let dx = ((dims.width - settled.width) as f32 / 2.0).round();
        if dx != 0.0 {
            Self::set_offset(rt, dx);
        }
        let drawn = if settings {
            self.draw_settings(rt, &settled)
        } else if space == NottSpace::Clipboard {
            self.draw_clipboard(rt, &settled)
        } else {
            let media = self.media.borrow();
            let composition = media.as_ref().and_then(|m| {
                resolve_media_layout_in(&settled, m.content.shape(), space).map(|l| (m, l))
            });
            match composition {
                Some((m, l)) => {
                    let music = f32::from(u8::from(space == NottSpace::Music));
                    self.render_media_blend(rt, &components, &l, &settled, clock, m, music)
                }
                None if space == NottSpace::Music => {
                    self.draw_music_empty(rt, &components, &settled)
                }
                None => self.render_expanded_content(
                    rt,
                    &components,
                    &settled,
                    &clock.formatted_time,
                    &clock.formatted_date,
                ),
            }
        };
        if dx != 0.0 {
            Self::set_offset(rt, 0.0);
        }
        drawn
    }

    /// Settings page: a quiet "Settings" label, the quick settings (title,
    /// description, switch at the right content edge), then the "Open Full
    /// Settings" action row (text and a chevron, no switch).
    fn draw_settings(&self, rt: &ID2D1RenderTarget, dimensions: &NotchDimensions) -> Result<()> {
        let Some(l) = resolve_settings_layout(dimensions) else {
            return Ok(());
        };
        let formats = self.media_formats(dimensions)?;
        let px = |v: f32| (v * dimensions.scale).round();
        let text = |s: &str, f: &IDWriteTextFormat, r: RectF, b| unsafe {
            let s: Vec<u16> = s.encode_utf16().collect();
            rt.DrawText(
                &s,
                f,
                &r.to_d2d_rect(),
                b,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        };
        unsafe {
            let primary = rt.CreateSolidColorBrush(&COLOR_TEXT_PRIMARY, None)?;
            let tertiary = rt.CreateSolidColorBrush(&COLOR_TEXT_TERTIARY, None)?;
            text(SETTINGS_LABEL, &formats.source, l.label, &tertiary);
            let levels = self.switches.borrow().map(|a| a.level());
            for (row, title, description, level) in [
                (
                    l.always_on_top,
                    ALWAYS_ON_TOP_TITLE,
                    ALWAYS_ON_TOP_DESCRIPTION,
                    levels[0],
                ),
                (
                    l.reduced_motion,
                    REDUCED_MOTION_TITLE,
                    REDUCED_MOTION_DESCRIPTION,
                    levels[1],
                ),
            ] {
                text(title, &formats.artist, row.title, &primary);
                text(description, &formats.source, row.description, &tertiary);
                self.draw_switch(rt, row.toggle, level, px(BASE_SETTINGS_TOGGLE_KNOB_INSET))?;
            }
            // Action row: text, and a chevron at the right content edge
            let secondary = rt.CreateSolidColorBrush(&COLOR_TEXT_SECONDARY, None)?;
            let a = l.full_settings;
            text(FULL_SETTINGS_TITLE, &formats.artist, a, &primary);
            let u = px(BASE_SETTINGS_CHEVRON_SIZE);
            let (cx, cy) = (a.right - u / 2.0, (a.top + a.bottom) / 2.0);
            let path = self.d2d_factory.CreatePathGeometry()?;
            let sink = path.Open()?;
            let h = GeometrySinkHelper::from_raw(windows::core::Interface::as_raw(&sink));
            h.begin_figure(D2D_POINT_2F {
                x: cx - u * 0.2,
                y: cy - u * 0.45,
            });
            h.add_line(D2D_POINT_2F {
                x: cx + u * 0.25,
                y: cy,
            });
            h.add_line(D2D_POINT_2F {
                x: cx - u * 0.2,
                y: cy + u * 0.45,
            });
            h.end_figure_open();
            h.close()?;
            rt.DrawGeometry(&path, &secondary, px(1.5).max(1.0), &self.round_stroke);
        }
        Ok(())
    }

    /// A switch (see `draw_switch`).
    fn draw_switch(&self, rt: &ID2D1RenderTarget, t: RectF, level: f32, inset: f32) -> Result<()> {
        draw_switch(rt, t, level, inset, 1.0, self.settings.get().accent)
    }

    /// Home <-> Music with a session: cover, text and controls move between
    /// their Home and Music places (each centred in the window); the clock
    /// column and divider fade out as the scrubber and visualizer fade in.
    fn draw_media_morph(
        &self,
        rt: &ID2D1RenderTarget,
        dims: &NotchDimensions,
        clock: &ClockDateState,
        music: f32,
    ) -> Result<()> {
        let media = self.media.borrow();
        let Some(m) = media.as_ref() else {
            return Ok(());
        };
        let place = |space: NottSpace| {
            let settled = space_dimensions(crate::config::NotchState::Expanded, dims.dpi, space);
            let dx = ((dims.width - settled.width) as f32 / 2.0).round();
            resolve_media_layout_in(&settled, m.content.shape(), space).map(|l| l.offset_x(dx))
        };
        let (Some(home), Some(player)) = (place(NottSpace::Home), place(NottSpace::Music)) else {
            return Ok(());
        };
        let ResolvedLayout::Expanded { components, .. } = resolve_layout(dims) else {
            return Ok(());
        };
        let blend = MediaLayout::morph(&home, &player, music);
        self.render_media_blend(rt, &components, &blend, dims, clock, m, music)
    }

    /// Clipboard space: count label and Clear in the selector band, then the
    /// newest entries as rows (text preview, or thumbnail + size), or the
    /// empty state. Clipped to the notch body (rows ride the height transition).
    fn draw_clipboard(&self, rt: &ID2D1RenderTarget, dimensions: &NotchDimensions) -> Result<()> {
        let Some(l) = resolve_clipboard_layout(dimensions) else {
            return Ok(());
        };
        let formats = self.media_formats(dimensions)?;
        let view = self.clipboard.borrow();
        let px = |v: f32| (v * dimensions.scale).round();
        let body = RectF::new(
            0.0,
            0.0,
            dimensions.width as f32,
            dimensions.height as f32 - dimensions.shadow_margin_bottom,
        );
        // Icon buttons grow smoothly with their hover / press levels
        let grow = |b: PanelHit| {
            let (hover, press) = self.feedback.borrow().clip_levels(b);
            1.0 + crate::config::CLIPBOARD_BUTTON_HOVER_GROW * hover
                + crate::config::CLIPBOARD_BUTTON_PRESS_GROW * press
        };
        let center = |r: RectF| ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        unsafe {
            let outline = rt.CreateSolidColorBrush(&COLOR_CLIPBOARD_ROW, None)?;
            let outline_w = px(crate::config::BASE_CLIPBOARD_ROW_OUTLINE).max(1.0);
            let primary = rt.CreateSolidColorBrush(&COLOR_TEXT_PRIMARY, None)?;
            let secondary = rt.CreateSolidColorBrush(&COLOR_TEXT_SECONDARY, None)?;
            let tertiary = rt.CreateSolidColorBrush(&COLOR_TEXT_TERTIARY, None)?;
            let text = |s: &[u16], f: &IDWriteTextFormat, r: RectF, b| {
                rt.DrawText(
                    s,
                    f,
                    &r.to_d2d_rect(),
                    b,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    DWRITE_MEASURING_MODE_NATURAL,
                )
            };
            rt.PushAxisAlignedClip(&body.to_d2d_rect(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            let icon = px(crate::config::BASE_CLIPBOARD_ACTION_ICON_SIZE);
            let mut drawn = Ok(());
            let confirming = self.clear_pending.get() && !view.rows.is_empty();
            if view.rows.is_empty() {
                let empty: Vec<u16> = CLIPBOARD_EMPTY_LABEL.encode_utf16().collect();
                text(&empty, &formats.empty, l.list, &tertiary);
            } else if confirming {
                // In place of the rows: question, note, then Cancel (filled,
                // the safe choice) and Clear (outlined); hover lifts each
                let utf16 = |s: &str| -> Vec<u16> { s.encode_utf16().collect() };
                text(
                    &utf16(CLEAR_CONFIRM_MESSAGE),
                    &formats.empty,
                    l.confirm_message,
                    &primary,
                );
                text(
                    &utf16(CLEAR_CONFIRM_NOTE),
                    &formats.empty,
                    l.confirm_note,
                    &tertiary,
                );
                let fill = rt.CreateSolidColorBrush(&COLOR_SPACE_PILL_SELECTED, None)?;
                for (r, label, hit, filled) in [
                    (l.cancel, CLEAR_CONFIRM_CANCEL, PanelHit::CancelClear, true),
                    (
                        l.confirm_clear,
                        CLEAR_CONFIRM_CLEAR,
                        PanelHit::ConfirmClear,
                        false,
                    ),
                ] {
                    let (hover, press) = self.feedback.borrow().clip_levels(hit);
                    let pill = D2D1_ROUNDED_RECT {
                        rect: r.to_d2d_rect(),
                        radiusX: r.height() / 2.0,
                        radiusY: r.height() / 2.0,
                    };
                    let lift = (hover + press).min(1.0);
                    if filled || lift > 0.0 {
                        fill.SetOpacity(if filled { 1.0 + 0.6 * lift } else { 0.6 * lift });
                        rt.FillRoundedRectangle(&pill, &fill);
                    }
                    if !filled {
                        let half = outline_w / 2.0;
                        let inner = RectF::new(
                            r.left + half,
                            r.top + half,
                            r.right - half,
                            r.bottom - half,
                        );
                        rt.DrawRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: inner.to_d2d_rect(),
                                radiusX: inner.height() / 2.0,
                                radiusY: inner.height() / 2.0,
                            },
                            &outline,
                            outline_w,
                            None,
                        );
                    }
                    text(&utf16(label), &formats.empty, r, &primary);
                }
            } else {
                let (cx, cy) = center(l.clear);
                let u = px(crate::config::BASE_CLIPBOARD_CLEAR_ICON_SIZE) * grow(PanelHit::Clear);
                if let Err(e) = self.draw_cross(rt, cx, cy, u, &secondary) {
                    drawn = Err(e);
                }
            }
            let (pad, inset) = (px(BASE_CLIPBOARD_ROW_PAD), px(BASE_CLIPBOARD_THUMB_INSET));
            let shown_rows = if confirming { 0 } else { view.rows.len() };
            for (i, (row, r)) in view.rows.iter().take(shown_rows).zip(l.rows).enumerate() {
                // Full-width outlined box; its copy / trash buttons fade in only
                // while the row is hovered (one row at a time)
                let (entry, copy, trash) = ClipboardLayout::row_parts(r);
                let shown = self.feedback.borrow().clip_levels(PanelHit::Row(i)).0;
                let half = outline_w / 2.0;
                rt.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: RectF::new(
                            entry.left + half,
                            entry.top + half,
                            entry.right - half,
                            entry.bottom - half,
                        )
                        .to_d2d_rect(),
                        radiusX: px(BASE_CLIPBOARD_ROW_RADIUS),
                        radiusY: px(BASE_CLIPBOARD_ROW_RADIUS),
                    },
                    &outline,
                    outline_w,
                    None,
                );
                if shown > 0.0 {
                    secondary.SetOpacity(shown);
                    let (cx, cy) = center(copy);
                    let u = icon * grow(PanelHit::Copy(i));
                    if let Err(e) = self.draw_glyph(rt, COPY, cx, cy, u, &secondary) {
                        drawn = Err(e);
                    }
                    let (cx, cy) = center(trash);
                    let u = icon * grow(PanelHit::Remove(i));
                    if let Err(e) = self.draw_glyph(rt, TRASH, cx, cy, u, &secondary) {
                        drawn = Err(e);
                    }
                    secondary.SetOpacity(1.0);
                }
                // Text uses the full box, stopping before the buttons while shown
                let right = if shown > 0.0 {
                    copy.left
                } else {
                    entry.right - pad / 2.0
                };
                let r = RectF::new(entry.left, entry.top, right, entry.bottom);
                match row {
                    ClipRow::Text(t) => {
                        let line = RectF::new(r.left + pad, r.top, r.right, r.bottom);
                        text(t, &formats.artist, line, &primary);
                    }
                    ClipRow::Image(art, caption) => {
                        let side = (r.height() - 2.0 * inset).max(1.0);
                        let dest = RectF::new(
                            r.left + inset,
                            r.top + inset,
                            r.left + inset + side,
                            r.top + inset + side,
                        );
                        match self.thumb_bitmap(
                            rt,
                            art,
                            side as u32,
                            px(BASE_CLIPBOARD_THUMB_RADIUS),
                        ) {
                            Ok(bitmap) => rt.DrawBitmap(
                                &bitmap,
                                Some(&dest.to_d2d_rect()),
                                1.0,
                                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                                None,
                            ),
                            Err(e) => drawn = Err(e),
                        }
                        let line = RectF::new(dest.right + pad, r.top, r.right, r.bottom);
                        text(caption, &formats.source, line, &secondary);
                    }
                }
            }
            rt.PopAxisAlignedClip();
            drawn
        }
    }

    /// Solid rounded X in a `u`-sized box centred on (cx, cy): two thick
    /// round-capped bars.
    fn draw_cross(
        &self,
        rt: &ID2D1RenderTarget,
        cx: f32,
        cy: f32,
        u: f32,
        brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
    ) -> Result<()> {
        let (w, a) = (0.24 * u, 0.5 * u - 0.12 * u);
        let p = |x: f32, y: f32| D2D_POINT_2F {
            x: cx + x,
            y: cy + y,
        };
        let path = unsafe { self.d2d_factory.CreatePathGeometry()? };
        let sink = unsafe { path.Open()? };
        let h = unsafe { GeometrySinkHelper::from_raw(windows::core::Interface::as_raw(&sink)) };
        unsafe {
            for (from, to) in [((-a, -a), (a, a)), ((-a, a), (a, -a))] {
                h.begin_figure(p(from.0, from.1));
                h.add_line(p(to.0, to.1));
                h.end_figure_open();
            }
            h.close()?;
            rt.DrawGeometry(&path, brush, w, &self.round_stroke);
        }
        Ok(())
    }

    /// Thumbnail device bitmap of `art` at exactly `px` x `px` (built once per
    /// image and size, then reused every frame).
    fn thumb_bitmap(
        &self,
        rt: &ID2D1RenderTarget,
        art: &Artwork,
        px: u32,
        radius: f32,
    ) -> Result<ID2D1Bitmap> {
        if let Some((_, _, bitmap)) = self
            .thumbs
            .borrow()
            .iter()
            .find(|(a, size, _)| a == art && *size == px)
        {
            return Ok(bitmap.clone());
        }
        let bitmap = Self::cached_bitmap(rt, &RefCell::new(None), &thumbnail(art, px, radius))?;
        let mut thumbs = self.thumbs.borrow_mut();
        thumbs.retain(|(a, _, _)| a != art);
        thumbs.push((art.clone(), px, bitmap.clone()));
        Ok(bitmap)
    }

    /// Thin rounded bars, centered on the slot, in the playback accent. Draws
    /// nothing unless the visualizer is (fading) visible.
    fn draw_visualizer(
        &self,
        rt: &ID2D1RenderTarget,
        bounds: RectF,
        accent: D2D1_COLOR_F,
        scale: f32,
        alpha: f32,
    ) -> Result<()> {
        let viz = *self.visualizer.borrow();
        if viz.level() <= 0.0 {
            return Ok(());
        }
        let (bar, gap, full) = visualizer_metrics(scale);
        let cy = (bounds.top + bounds.bottom) / 2.0;
        let brush = unsafe { rt.CreateSolidColorBrush(&accent, None)? };
        unsafe { brush.SetOpacity(viz.level() * alpha) };
        let t = self.epoch.elapsed().as_secs_f64();
        for (i, h) in viz.bar_heights(t).into_iter().enumerate() {
            let left = bounds.left + i as f32 * (bar + gap);
            let half = (full * h).max(bar) / 2.0;
            unsafe {
                rt.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: RectF::new(left, cy - half, left + bar, cy + half).to_d2d_rect(),
                        radiusX: bar / 2.0,
                        radiusY: bar / 2.0,
                    },
                    &brush,
                );
            }
        }
        Ok(())
    }

    /// Inline scrubber: elapsed label, rounded track (dark neutral unplayed,
    /// accent played), remaining label. Omitted when the
    /// session has no timeline or the strip is too narrow.
    #[allow(clippy::too_many_arguments)]
    fn draw_timeline(
        &self,
        rt: &ID2D1RenderTarget,
        strip: RectF,
        content: &MediaContent,
        formats: &MediaFormats,
        accent: D2D1_COLOR_F,
        scale: f32,
        opacity: f32,
    ) -> Result<()> {
        let Some(timeline) = content.timeline else {
            return Ok(());
        };
        let px = |v: f32| (v * scale).round();
        let (label_w, label_gap) = (
            px(BASE_MEDIA_TIMELINE_LABEL_WIDTH),
            px(BASE_MEDIA_TIMELINE_LABEL_GAP),
        );
        let track_left = strip.left + label_w + label_gap;
        let track_right = strip.right - label_w - label_gap;
        if track_right - track_left < px(BASE_MEDIA_TIMELINE_MIN_TRACK) {
            return Ok(());
        }
        let playing = shows_visualizer(content.playback);
        let elapsed = timeline.position_at(now_filetime(), playing);
        let frac = elapsed as f32 / timeline.duration_ms as f32;

        // Apple-style track: one rounded bar; the played part is the same bar in
        // the accent, clipped at the playhead (rounded start, straight cut), no thumb
        let thick = px(BASE_MEDIA_TIMELINE_TRACK).max(2.0);
        let top = ((strip.top + strip.bottom) / 2.0 - thick / 2.0).round();
        let bar = D2D1_ROUNDED_RECT {
            rect: RectF::new(track_left, top, track_right, top + thick).to_d2d_rect(),
            radiusX: thick / 2.0,
            radiusY: thick / 2.0,
        };
        unsafe {
            let unplayed = rt.CreateSolidColorBrush(&COLOR_MEDIA_TRACK_UNPLAYED, None)?;
            unplayed.SetOpacity(opacity);
            rt.FillRoundedRectangle(&bar, &unplayed);
            let x = track_left + (track_right - track_left) * frac.clamp(0.0, 1.0);
            if x > track_left {
                let played = rt.CreateSolidColorBrush(&accent, None)?;
                played.SetOpacity(opacity);
                rt.PushAxisAlignedClip(
                    &RectF::new(track_left, top, x, top + thick).to_d2d_rect(),
                    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                );
                rt.FillRoundedRectangle(&bar, &played);
                rt.PopAxisAlignedClip();
            }

            let label = rt.CreateSolidColorBrush(&COLOR_TEXT_TERTIARY, None)?;
            label.SetOpacity(opacity);
            let mut buf = [0u16; 12];
            let n = format_clock(elapsed, false, &mut buf);
            rt.DrawText(
                &buf[..n],
                &formats.elapsed,
                &RectF::new(strip.left, strip.top, strip.left + label_w, strip.bottom)
                    .to_d2d_rect(),
                &label,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            let n = format_clock(timeline.duration_ms - elapsed, true, &mut buf);
            rt.DrawText(
                &buf[..n],
                &formats.remaining,
                &RectF::new(strip.right - label_w, strip.top, strip.right, strip.bottom)
                    .to_d2d_rect(),
                &label,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
        Ok(())
    }

    /// Renders the components of the expanded notch: centered live Windows time and date
    fn render_expanded_content(
        &self,
        rt: &ID2D1RenderTarget,
        layout: &ExpandedLayout,
        dimensions: &NotchDimensions,
        formatted_time: &str,
        formatted_date: &str,
    ) -> Result<()> {
        let clip_rect = layout.content_bounds.to_d2d_rect();
        unsafe {
            rt.PushAxisAlignedClip(&clip_rect, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }

        // 1. Live time: display numerals with a smaller, lighter AM/PM
        let formats = self.media_formats(dimensions)?;
        let primary_brush = unsafe { rt.CreateSolidColorBrush(&COLOR_TEXT_PRIMARY, None)? };
        self.draw_clock_display(
            rt,
            formatted_time,
            &formats.clock_display,
            TextRole::ClockDisplay.style().size * dimensions.scale,
            layout.time_bounds,
            &primary_brush,
        )?;

        // 2. Date line
        let date_format = &formats.date_line;
        let date_utf16: Vec<u16> = formatted_date.encode_utf16().collect();
        let date_rect = layout.date_bounds.to_d2d_rect();
        let secondary_brush = unsafe { rt.CreateSolidColorBrush(&COLOR_TEXT_SECONDARY, None)? };
        unsafe {
            rt.DrawText(
                &date_utf16,
                date_format,
                &date_rect,
                &secondary_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            rt.PopAxisAlignedClip();
        }

        Ok(())
    }

    /// The expanded Home time in `rect` (centred): its numerals in `format`
    /// (`size` px), and an AM/PM (or locale equivalent, with the space before
    /// it) at `CLOCK_MERIDIEM_SCALE` of that size in `CLOCK_MERIDIEM_WEIGHT`,
    /// sharing the numerals' baseline.
    fn draw_clock_display(
        &self,
        rt: &ID2D1RenderTarget,
        text: &str,
        format: &IDWriteTextFormat,
        size: f32,
        rect: RectF,
        brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
    ) -> Result<()> {
        let utf16: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            // DrawText builds the same layout internally; building it here
            // only adds the meridiem's range
            let layout = self.dwrite_factory.CreateTextLayout(
                &utf16,
                format,
                rect.width().max(1.0),
                rect.height().max(1.0),
            )?;
            if let Some((start, length)) = meridiem_range(text) {
                let range = DWRITE_TEXT_RANGE {
                    startPosition: start,
                    length,
                };
                layout.SetFontSize(size * crate::config::CLOCK_MERIDIEM_SCALE, range)?;
                layout.SetFontWeight(
                    DWRITE_FONT_WEIGHT(crate::config::CLOCK_MERIDIEM_WEIGHT),
                    range,
                )?;
            }
            // The origin's vector type, reached like `set_offset`'s matrix
            let mut origin =
                windows::Win32::Graphics::Direct2D::D2D1_POINT_DESCRIPTION::default().point;
            origin.X = rect.left;
            origin.Y = rect.top;
            rt.DrawTextLayout(origin, &layout, brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
        Ok(())
    }

    /// Single-line text format with ellipsis trimming (never wraps, never overflows).
    fn create_line_format(
        &self,
        family: windows::core::PCWSTR,
        font_size: f32,
        weight: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT,
        alignment: DWRITE_TEXT_ALIGNMENT,
    ) -> Result<IDWriteTextFormat> {
        let format =
            self.create_text_format_in(family, font_size, weight, DWRITE_FONT_STYLE_NORMAL)?;
        unsafe {
            format.SetTextAlignment(alignment)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let sign = self.dwrite_factory.CreateEllipsisTrimmingSign(&format)?;
            let trimming = DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                delimiter: 0,
                delimiterCount: 0,
            };
            format.SetTrimming(&trimming, &sign)?;
        }
        Ok(format)
    }

    /// Media text formats for the current DPI, created once per DPI (not per frame).
    fn media_formats(
        &self,
        dimensions: &NotchDimensions,
    ) -> Result<std::cell::Ref<'_, MediaFormats>> {
        let stale = self
            .media_formats
            .borrow()
            .as_ref()
            .is_none_or(|(dpi, _)| *dpi != dimensions.dpi);
        if stale {
            let s = dimensions.scale;
            let role = |role, align| self.role_format(role, s, align);
            let formats = MediaFormats {
                title: role(TextRole::MediaTitle, DWRITE_TEXT_ALIGNMENT_LEADING)?,
                artist: role(TextRole::MediaArtist, DWRITE_TEXT_ALIGNMENT_LEADING)?,
                source: role(TextRole::MediaMeta, DWRITE_TEXT_ALIGNMENT_LEADING)?,
                time: role(TextRole::ClockColumn, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
                date: role(TextRole::DateCompact, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
                elapsed: role(TextRole::Timestamp, DWRITE_TEXT_ALIGNMENT_LEADING)?,
                remaining: role(TextRole::Timestamp, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
                empty: role(TextRole::EmptyState, DWRITE_TEXT_ALIGNMENT_CENTER)?,
                clock_display: role(TextRole::ClockDisplay, DWRITE_TEXT_ALIGNMENT_CENTER)?,
                date_line: role(TextRole::DateLine, DWRITE_TEXT_ALIGNMENT_CENTER)?,
            };
            *self.media_formats.borrow_mut() = Some((dimensions.dpi, formats));
        }
        Ok(std::cell::Ref::map(self.media_formats.borrow(), |f| {
            &f.as_ref().expect("formats just ensured").1
        }))
    }

    /// Device bitmap for the artwork, created once per artwork (and per render
    /// target). Pixels are already decoded BGRA8 premultiplied (Phase 3.7).
    fn artwork_bitmap(&self, rt: &ID2D1RenderTarget, artwork: &Artwork) -> Result<ID2D1Bitmap> {
        Self::cached_bitmap(rt, &self.art_bitmap, artwork)
    }

    /// One-entry device-bitmap cache: reuses the bitmap while the image is unchanged.
    fn cached_bitmap(
        rt: &ID2D1RenderTarget,
        cache: &RefCell<Option<(Artwork, ID2D1Bitmap)>>,
        artwork: &Artwork,
    ) -> Result<ID2D1Bitmap> {
        if let Some((cached, bitmap)) = cache.borrow().as_ref()
            && cached == artwork
        {
            return Ok(bitmap.clone());
        }
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        let bitmap = unsafe {
            rt.CreateBitmap(
                D2D_SIZE_U {
                    width: artwork.width,
                    height: artwork.height,
                },
                Some(artwork.pixels.as_ptr() as *const _),
                artwork.width * 4,
                &props,
            )?
        };
        *cache.borrow_mut() = Some((artwork.clone(), bitmap.clone()));
        Ok(bitmap)
    }

    /// Where the source-app badge goes: straddling the artwork's bottom-right
    /// corner, overhanging the right and bottom edges.
    fn badge_rect(art: RectF, scale: f32) -> RectF {
        let size = (BASE_MEDIA_BADGE_SIZE * scale).round();
        let overhang = (BASE_MEDIA_BADGE_OVERHANG * scale).round();
        RectF::new(
            art.right + overhang - size,
            art.bottom + overhang - size,
            art.right + overhang,
            art.bottom + overhang,
        )
    }

    /// Badge device bitmap at exactly `px` x `px`: the icon is area-resampled
    /// once (Direct2D's bilinear filter aliases when shrinking more than ~2x),
    /// then drawn 1:1 every frame.
    fn badge_bitmap_for(
        &self,
        rt: &ID2D1RenderTarget,
        icon: &Artwork,
        px: u32,
    ) -> Result<ID2D1Bitmap> {
        if let Some((cached, size, bitmap)) = self.badge_bitmap.borrow().as_ref()
            && cached == icon
            && *size == px
        {
            return Ok(bitmap.clone());
        }
        let resized = resample_area(icon, px);
        let scratch = RefCell::new(None);
        let bitmap = Self::cached_bitmap(rt, &scratch, &resized)?;
        *self.badge_bitmap.borrow_mut() = Some((icon.clone(), px, bitmap.clone()));
        Ok(bitmap)
    }

    /// Source-app badge: the prepared squircle icon straddling the artwork's
    /// bottom-right corner, cut out of the cover by a ring of the notch's black.
    /// Drawn with its own clip (content area grown by the overhang + ring), which
    /// stays inside the notch body, also during the expand/collapse animation.
    fn draw_badge(
        &self,
        rt: &ID2D1RenderTarget,
        icon: &Artwork,
        art: RectF,
        content: RectF,
        scale: f32,
        opacity: f32,
    ) -> Result<()> {
        let dest = Self::badge_rect(art, scale);
        let ring = (BASE_MEDIA_BADGE_RING * scale).round().max(1.0);
        let reach = (BASE_MEDIA_BADGE_OVERHANG * scale).round() + ring;
        let clip = RectF::new(
            content.left,
            content.top,
            content.right + reach,
            content.bottom + reach,
        );
        let bitmap = self.badge_bitmap_for(rt, icon, dest.width() as u32)?;
        let radius = dest.width() * crate::config::BADGE_CORNER_FRACTION;
        unsafe {
            rt.PushAxisAlignedClip(&clip.to_d2d_rect(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            let cutout = rt.CreateSolidColorBrush(&NOTCH_BG_COLOR, None)?;
            cutout.SetOpacity(opacity);
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: RectF::new(
                        dest.left - ring,
                        dest.top - ring,
                        dest.right + ring,
                        dest.bottom + ring,
                    )
                    .to_d2d_rect(),
                    radiusX: radius + ring,
                    radiusY: radius + ring,
                },
                &cutout,
            );
            rt.DrawBitmap(
                &bitmap,
                Some(&dest.to_d2d_rect()),
                opacity,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
            rt.PopAxisAlignedClip();
        }
        Ok(())
    }
    /// Renders the expanded media composition (see `MediaLayout`): artwork,
    /// title / artist — album / source, secondary time + short date, and the
    /// transport controls with their hover/press feedback. Track content (artwork
    /// and text) carries the new-track fade; the clock and controls never fade.
    #[cfg(test)]
    fn render_media_content(
        &self,
        rt: &ID2D1RenderTarget,
        layout: &ExpandedLayout,
        media_layout: &MediaLayout,
        dimensions: &NotchDimensions,
        clock: &ClockDateState,
        media: &MediaText,
    ) -> Result<()> {
        // The Music composition is the one without Home's divider
        let music = f32::from(u8::from(media_layout.divider_bounds.width() <= 0.0));
        self.render_media_blend(rt, layout, media_layout, dimensions, clock, media, music)
    }

    /// The media composition with a Music weight (0 = Home, 1 = Music; in
    /// between while morphing): Home-only parts (clock column, divider, badge)
    /// fade out and Music-only parts (scrubber, visualizer) fade in.
    #[allow(clippy::too_many_arguments)]
    fn render_media_blend(
        &self,
        rt: &ID2D1RenderTarget,
        layout: &ExpandedLayout,
        media_layout: &MediaLayout,
        dimensions: &NotchDimensions,
        clock: &ClockDateState,
        media: &MediaText,
        music: f32,
    ) -> Result<()> {
        // Home-only parts leave early (gone by ~35%, before the narrowing
        // shell would clip them); Music-only parts arrive late
        let home_alpha = 1.0 - smoothstep(0.0, 0.35, music);
        let music_alpha = scene_fade(music);
        let s = dimensions.scale;
        let formats = self.media_formats(dimensions)?;
        let feedback = self.feedback.borrow();
        let track_alpha = feedback.track_alpha();
        let brush = |color: &D2D1_COLOR_F, opacity: f32| unsafe {
            let b = rt.CreateSolidColorBrush(color, None)?;
            b.SetOpacity(opacity);
            Ok::<_, Error>(b)
        };
        let title_brush = brush(&COLOR_TEXT_PRIMARY, track_alpha)?;
        let artist_brush = brush(&COLOR_TEXT_SECONDARY, track_alpha)?;
        // The source row is Home-only, unless it stands in for a missing artist
        let source_alpha = if media.subtitle.is_empty() {
            1.0
        } else {
            home_alpha
        };
        let source_brush = brush(&COLOR_TEXT_TERTIARY, track_alpha * source_alpha)?;
        let time_brush = brush(&COLOR_TEXT_SECONDARY, home_alpha)?;
        let date_brush = brush(&COLOR_TEXT_TERTIARY, home_alpha)?;
        let icon_brush = brush(&COLOR_TEXT_PRIMARY, 1.0)?;
        let draw = |text: &[u16], format: &IDWriteTextFormat, rect: RectF, brush| unsafe {
            rt.DrawText(
                text,
                format,
                &rect.to_d2d_rect(),
                brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        };

        // The artwork sits out at the notch edge, left of the content area
        // Content area, grown to everything the composition places outside it:
        // the artwork at the notch edge (Home), the Music player's wider column
        // (cover, scrubber, visualizer) and its larger controls' backdrops
        let clip_bounds = {
            let c = layout.content_bounds;
            let mut parts = vec![
                media_layout.timeline_bounds,
                media_layout.visualizer_bounds,
                media_layout.control_visual_bounds(MediaControl::PlayPause),
            ];
            parts.extend(media_layout.artwork_bounds);
            parts.into_iter().fold(c, |u, r| {
                RectF::new(
                    u.left.min(r.left),
                    u.top.min(r.top),
                    u.right.max(r.right),
                    u.bottom.max(r.bottom),
                )
            })
        };
        unsafe {
            rt.PushAxisAlignedClip(
                &clip_bounds.to_d2d_rect(),
                D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            );
        }

        if let (Some(artwork), Some(dest)) = (&media.content.artwork, media_layout.artwork_bounds) {
            // Home: follows the notch's bottom corner (its radius minus the gap
            // between them), softened slightly so the cover reads a touch rounder.
            // Music: the small player cover has its own modest radius.
            // Home's radius comes from its settled notch (the cover nests in
            // its bottom corner); Music's small player cover has its own
            let home = space_dimensions(
                crate::config::NotchState::Expanded,
                dimensions.dpi,
                NottSpace::Home,
            );
            let home_radius =
                resolve_media_layout_in(&home, media.content.shape(), NottSpace::Home)
                    .and_then(|l| l.artwork_bounds)
                    .map_or(0.0, |a| {
                        let gap = home.notch_height() - a.bottom;
                        (home.curvature.bottom_radius - gap + BASE_MEDIA_ARTWORK_RADIUS_EXTRA * s)
                            .max(0.0)
                    });
            let radius = home_radius + (BASE_MUSIC_ARTWORK_RADIUS * s - home_radius) * music;
            self.draw_artwork(rt, artwork, dest, radius, s, track_alpha)?;
            // The source badge belongs to Home; the Music player cover is clean
            // (only while Settings > Media > Show source app is on)
            if let Some(icon) = media
                .content
                .source
                .icon
                .as_ref()
                .filter(|_| home_alpha > 0.0 && self.settings.get().show_source_app)
            {
                // The badge overhangs the content area: draw it outside the content clip
                unsafe { rt.PopAxisAlignedClip() };
                self.draw_badge(rt, icon, dest, clip_bounds, s, track_alpha * home_alpha)?;
                unsafe {
                    rt.PushAxisAlignedClip(
                        &clip_bounds.to_d2d_rect(),
                        D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    );
                }
            }
        }

        draw(
            &media.title,
            &formats.title,
            media_layout.title_bounds,
            &title_brush,
        );
        if !media.subtitle.is_empty() {
            draw(
                &media.subtitle,
                &formats.artist,
                media_layout.artist_bounds,
                &artist_brush,
            );
        }
        if !media.source.is_empty() {
            // Without an artist line the source moves up so no gap is left
            let rect = if media.subtitle.is_empty() {
                media_layout.artist_bounds
            } else {
                media_layout.source_bounds
            };
            if rect.height() > 0.0 {
                draw(&media.source, &formats.source, rect, &source_brush);
            }
        }

        // Home: thin divider between the track text and the time/date
        let divider = media_layout.divider_bounds;
        if divider.width() > 0.0 && divider.height() > 0.0 && home_alpha > 0.0 {
            unsafe {
                let line = rt.CreateSolidColorBrush(&COLOR_MEDIA_DIVIDER, None)?;
                line.SetOpacity(home_alpha);
                rt.FillRectangle(&divider.to_d2d_rect(), &line);
            }
        }

        // Secondary live time / short date in the right column (Home only;
        // the Music layout has no clock column)
        if media_layout.time_bounds.width() > 0.0 && home_alpha > 0.0 {
            let time_utf16: Vec<u16> = clock.formatted_time.encode_utf16().collect();
            draw(
                &time_utf16,
                &formats.time,
                media_layout.time_bounds,
                &time_brush,
            );
            let date_utf16: Vec<u16> = clock.formatted_date_short.encode_utf16().collect();
            draw(
                &date_utf16,
                &formats.date,
                media_layout.date_bounds,
                &date_brush,
            );
        }

        // Playback accent (shared by visualizer and scrubber): swaps with the
        // artwork, so it rides the same track fade and never lags a track
        let accent = accent_color(Some(&media.content), self.settings.get().accent);
        // (only while Settings > Media > Show visualizer is on; its slot stays
        // reserved so nothing else moves)
        if media_layout.visualizer_bounds.width() > 0.0
            && music_alpha > 0.0
            && self.settings.get().show_visualizer
        {
            self.draw_visualizer(rt, media_layout.visualizer_bounds, accent, s, music_alpha)?;
        }
        if music_alpha > 0.0 {
            self.draw_timeline(
                rt,
                media_layout.timeline_bounds,
                &media.content,
                &formats,
                accent,
                s,
                track_alpha * music_alpha,
            )?;
        }

        // Transport controls: no backdrop; icons quiet at rest and full on
        // hover/press, compressed while pressed.
        for control in MediaControl::ALL {
            let v = media_layout.control_visual_bounds(control);
            let (cx, cy) = ((v.left + v.right) / 2.0, (v.top + v.bottom) / 2.0);
            let (hover, press) = feedback.levels(control);
            // Same glyphs; the Music player draws them larger
            let (home_size, music_size) = match control {
                MediaControl::PlayPause => (BASE_MEDIA_PLAY_ICON_SIZE, BASE_MUSIC_PLAY_ICON_SIZE),
                _ => (BASE_MEDIA_ICON_SIZE, BASE_MUSIC_ICON_SIZE),
            };
            let base = home_size + (music_size - home_size) * music;
            // Smooth press curve: eased compression and spring back
            let eased = press * press * (3.0 - 2.0 * press);
            let u = (base * s).round() * (1.0 - (1.0 - MEDIA_PRESS_ICON_SCALE) * eased);
            unsafe {
                icon_brush.SetOpacity(
                    MEDIA_ICON_REST_OPACITY + (1.0 - MEDIA_ICON_REST_OPACITY) * hover.max(press),
                );
            }
            // Snap to whole pixels at rest for crisp edges; exact while compressing
            for shape in icon_shapes(control, media.content.icon, cx, cy, u, press == 0.0) {
                match shape {
                    IconShape::Bar(r) => unsafe {
                        let radius = r.width() / 2.0;
                        rt.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: r.to_d2d_rect(),
                                radiusX: radius,
                                radiusY: radius,
                            },
                            &icon_brush,
                        );
                    },
                    IconShape::Triangle(pts) => self.fill_triangle(rt, pts, &icon_brush)?,
                }
            }
        }

        unsafe { rt.PopAxisAlignedClip() };
        Ok(())
    }

    /// Draws artwork center-cropped to the square slot (aspect preserved, never
    /// stretched), rounds it by painting the four outside-corner regions in the
    /// notch's own pure black, and adds a near-invisible hairline so dark covers
    /// keep their edge against the black notch.
    fn draw_artwork(
        &self,
        rt: &ID2D1RenderTarget,
        artwork: &Artwork,
        dest: RectF,
        radius: f32,
        scale: f32,
        opacity: f32,
    ) -> Result<()> {
        let bitmap = self.artwork_bitmap(rt, artwork)?;
        let src = cover_source_rect(artwork.width, artwork.height, dest.width(), dest.height());
        unsafe {
            rt.DrawBitmap(
                &bitmap,
                Some(&dest.to_d2d_rect()),
                opacity,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                Some(&src.to_d2d_rect()),
            );
        }

        // Same continuous-curvature corner as the expanded notch's bottom corners,
        // so the cover's corners read as the notch's corners in miniature.
        let profile = CornerProfile::new(radius, 1.0, dest.width().min(dest.height()) / 2.0);
        let r = profile.span;
        if r > 0.0 {
            let kb = profile.handle * r;
            let path = unsafe { self.d2d_factory.CreatePathGeometry()? };
            let sink = unsafe { path.Open()? };
            let helper =
                unsafe { GeometrySinkHelper::from_raw(windows::core::Interface::as_raw(&sink)) };
            let p = |x: f32, y: f32| D2D_POINT_2F { x, y };
            // (corner, unit direction along x, unit direction along y) into the rect
            for (cx, cy, dx, dy) in [
                (dest.left, dest.top, 1.0, 1.0),
                (dest.right, dest.top, -1.0, 1.0),
                (dest.left, dest.bottom, 1.0, -1.0),
                (dest.right, dest.bottom, -1.0, -1.0),
            ] {
                unsafe {
                    helper.begin_figure(p(cx, cy));
                    helper.add_line(p(cx + dx * r, cy));
                    helper.add_bezier(&D2D1_BEZIER_SEGMENT {
                        point1: p(cx + dx * (r - kb), cy),
                        point2: p(cx, cy + dy * (r - kb)),
                        point3: p(cx, cy + dy * r),
                    });
                    helper.end_figure();
                }
            }
            unsafe {
                helper.close()?;
                let black = rt.CreateSolidColorBrush(&NOTCH_BG_COLOR, None)?;
                rt.FillGeometry(&path, &black, None);
                let hairline = rt.CreateSolidColorBrush(&COLOR_ARTWORK_HAIRLINE, None)?;
                hairline.SetOpacity(opacity);
                let inset = 0.5 * scale.max(1.0);
                let edge = self.smooth_rect_path(
                    RectF::new(
                        dest.left + inset,
                        dest.top + inset,
                        dest.right - inset,
                        dest.bottom - inset,
                    ),
                    r - inset,
                    profile.handle,
                )?;
                rt.DrawGeometry(&edge, &hairline, scale.max(1.0), None);
            }
        }
        Ok(())
    }

    /// Closed rectangle outline whose corners use the notch's corner curve
    /// (`span` along each edge, `handle` as in `CornerProfile`).
    fn smooth_rect_path(&self, rect: RectF, span: f32, handle: f32) -> Result<ID2D1PathGeometry> {
        let path = unsafe { self.d2d_factory.CreatePathGeometry()? };
        let sink = unsafe { path.Open()? };
        let helper =
            unsafe { GeometrySinkHelper::from_raw(windows::core::Interface::as_raw(&sink)) };
        let p = |x: f32, y: f32| D2D_POINT_2F { x, y };
        let k = span * (1.0 - handle);
        let (l, t, r, b) = (rect.left, rect.top, rect.right, rect.bottom);
        unsafe {
            helper.begin_figure(p(l + span, t));
            // (corner, end of the straight run, control points, corner end), clockwise
            for (line_to, c1, c2, end) in [
                (p(r - span, t), p(r - k, t), p(r, t + k), p(r, t + span)),
                (p(r, b - span), p(r, b - k), p(r - k, b), p(r - span, b)),
                (p(l + span, b), p(l + k, b), p(l, b - k), p(l, b - span)),
                (p(l, t + span), p(l, t + k), p(l + k, t), p(l + span, t)),
            ] {
                helper.add_line(line_to);
                helper.add_bezier(&D2D1_BEZIER_SEGMENT {
                    point1: c1,
                    point2: c2,
                    point3: end,
                });
            }
            helper.end_figure();
            helper.close()?;
        }
        Ok(path)
    }

    /// Fills a triangle with softly rounded vertices (each corner replaced by a
    /// curve tangent to both edges), for Apple-like transport glyphs.
    fn fill_triangle(
        &self,
        rt: &ID2D1RenderTarget,
        pts: [(f32, f32); 3],
        brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
    ) -> Result<()> {
        let path = unsafe { self.d2d_factory.CreatePathGeometry()? };
        let sink = unsafe { path.Open()? };
        let helper =
            unsafe { GeometrySinkHelper::from_raw(windows::core::Interface::as_raw(&sink)) };
        let (a, b) = rounded_triangle_corners(pts);
        let p = |(x, y): (f32, f32)| D2D_POINT_2F { x, y };
        unsafe {
            helper.begin_figure(p(b[0]));
            for k in [1usize, 2, 0] {
                let v = pts[k];
                helper.add_line(p(a[k]));
                // Quadratic corner (control at the vertex) as an exact cubic
                helper.add_bezier(&D2D1_BEZIER_SEGMENT {
                    point1: p((
                        a[k].0 + (v.0 - a[k].0) * 2.0 / 3.0,
                        a[k].1 + (v.1 - a[k].1) * 2.0 / 3.0,
                    )),
                    point2: p((
                        b[k].0 + (v.0 - b[k].0) * 2.0 / 3.0,
                        b[k].1 + (v.1 - b[k].1) * 2.0 / 3.0,
                    )),
                    point3: p(b[k]),
                });
            }
            helper.end_figure();
            helper.close()?;
            rt.FillGeometry(&path, brush, None);
        }
        Ok(())
    }

    /// Renders the notch shape into a 32-bit premultiplied ARGB DIB surface
    /// and uploads it atomically to the Desktop Window Manager (DWM) via UpdateLayeredWindow.
    ///
    /// Reuses cached high-performance GDI/D2D surfaces to guarantee 60+ FPS zero-allocation animation.
    pub fn render(
        &self,
        hwnd: HWND,
        x: i32,
        y: i32,
        dimensions: &NotchDimensions,
        hovered: bool,
        clock: &ClockDateState,
    ) -> Result<()> {
        let width = dimensions.width;
        let height = dimensions.height;
        let border_width = dimensions.border_width;

        if width <= 0 || height <= 0 {
            return Ok(());
        }

        self.ensure_buffer(width, height)?;

        let mem_dc = self.cached_mem_dc.get();
        let screen_dc = self.cached_screen_dc.get();
        let rt_borrow = self.cached_rt.borrow();
        let rt = match rt_borrow.as_ref() {
            Some(target) => target,
            None => return Err(Error::from_thread()),
        };

        unsafe {
            let bind_rect = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            rt.BindDC(mem_dc, &bind_rect)?;

            rt.BeginDraw();

            // Transparent background (clears all alpha to 0)
            rt.Clear(Some(&COLOR_TRANSPARENT));

            // Solid AMOLED pure black notch body (#000000)
            let fill_brush = rt.CreateSolidColorBrush(&NOTCH_BG_COLOR, None)?;

            // Build mathematical hardware-notch path geometry
            let path = self.create_notch_geometry(dimensions)?;

            // Fill interior with pure black
            rt.FillGeometry(&path, &fill_brush, None);

            // Specular highlight border only if not transparent (preserves deep black on hover)
            let border_color = if hovered {
                COLOR_BORDER_HOVER
            } else {
                NOTCH_BORDER_COLOR
            };
            if border_color.a > 0.0 {
                let border_brush = rt.CreateSolidColorBrush(&border_color, None)?;
                rt.DrawGeometry(&path, &border_brush, border_width, None);
            }

            // Render layout components based on state and height
            let layout = resolve_layout(dimensions);
            match &layout {
                ResolvedLayout::Collapsed { components, .. } => {
                    // Fades back in as a collapse nears its end
                    let alpha = self.scene_view().alpha;
                    if alpha > 0.0 {
                        self.with_layer(rt, None, alpha, || {
                            self.render_collapsed_content(
                                rt,
                                components,
                                dimensions,
                                &clock.formatted_time,
                            )
                        })?;
                    }
                }
                ResolvedLayout::Expanded { .. } => {
                    let min_content_height = (50.0 * dimensions.scale).round() as i32;
                    if dimensions.height >= min_content_height {
                        self.draw_expanded(rt, dimensions, clock)?;
                    } else {
                        let collapsed_layout = resolve_collapsed_layout(dimensions);
                        self.render_collapsed_content(
                            rt,
                            &collapsed_layout,
                            dimensions,
                            &clock.formatted_time,
                        )?;
                    }
                }
            }

            rt.EndDraw(None, None)?;

            // Soft dark ambient shadow for expanded notch. The mask (kept in
            // scratch_a) depends only on the silhouette, so frames of a settled
            // notch (visualizer, scrubber) reuse it and only composite.
            if dimensions.shadow_opacity > 0.0 {
                let bits = self.cached_bits.get();
                if !bits.is_null() {
                    let (w, h) = (width as usize, height as usize);
                    let stride = self.cached_capacity_w.get() as usize;
                    let mut scratch_a = self.scratch_a.borrow_mut();
                    let key = Some((*dimensions, hovered));
                    if self.shadow_key.get() != key {
                        let mut scratch_b = self.scratch_b.borrow_mut();
                        let built = build_shadow_mask(
                            bits as *mut u8,
                            w,
                            h,
                            stride,
                            dimensions.scale,
                            dimensions.shadow_opacity,
                            &mut scratch_a,
                            &mut scratch_b,
                        );
                        self.shadow_key.set(if built { key } else { None });
                    }
                    if self.shadow_key.get().is_some() {
                        composite_shadow(bits as *mut u8, w, h, stride, &scratch_a);
                    }
                }
            }

            // Upload to DWM atomically via UpdateLayeredWindow
            let pt_dst = POINT { x, y };
            let size = SIZE {
                cx: width,
                cy: height,
            };
            let pt_src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };

            UpdateLayeredWindow(
                hwnd,
                Some(screen_dc),
                Some(&pt_dst),
                Some(&size),
                Some(mem_dc),
                Some(&pt_src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.cleanup_cached_resources();
    }
}

/// One step of an icon outline in a unit box (-0.5..0.5, y down): move, line,
/// cubic Bezier (two controls, end), close.
#[derive(Clone, Copy)]
pub(crate) enum IconSeg {
    M(f32, f32),
    L(f32, f32),
    C(f32, f32, f32, f32, f32, f32),
    Z,
}

// Space icons: outlines of Flaticon UIcons (solid rounded, used under the
// project's Flaticon license), converted from the font glyphs to cubic paths.
/// Flaticon UIcons solid-rounded `house-blank` (U+F7C2).
const HOUSE_BLANK: &[IconSeg] = &[
    IconSeg::M(0.2933, 0.5),
    IconSeg::L(-0.29, 0.5),
    IconSeg::C(-0.3278, 0.5, -0.3628, 0.4906, -0.395, 0.4717),
    IconSeg::C(-0.4272, 0.4528, -0.4528, 0.4272, -0.4717, 0.395),
    IconSeg::C(-0.4906, 0.3628, -0.5, 0.3278, -0.5, 0.29),
    IconSeg::L(-0.5, -0.0933),
    IconSeg::C(-0.5, -0.1289, -0.4917, -0.1622, -0.475, -0.1933),
    IconSeg::C(-0.4583, -0.2244, -0.4356, -0.2489, -0.4067, -0.2667),
    IconSeg::L(-0.1167, -0.4633),
    IconSeg::C(-0.0811, -0.4878, -0.0422, -0.5, 0.0, -0.5),
    IconSeg::C(0.0422, -0.5, 0.0811, -0.4878, 0.1167, -0.4633),
    IconSeg::L(0.4067, -0.2667),
    IconSeg::C(0.4356, -0.2489, 0.4583, -0.2244, 0.475, -0.1933),
    IconSeg::C(0.4917, -0.1622, 0.5, -0.1289, 0.5, -0.0933),
    IconSeg::L(0.5, 0.29),
    IconSeg::C(0.5, 0.3278, 0.4906, 0.3628, 0.4717, 0.395),
    IconSeg::C(0.4528, 0.4272, 0.4272, 0.4528, 0.395, 0.4717),
    IconSeg::C(0.3628, 0.4906, 0.3289, 0.5, 0.2933, 0.5),
    IconSeg::Z,
];
/// Flaticon UIcons solid-rounded `music-alt` (U+F985).
pub(crate) const MUSIC_ALT: &[IconSeg] = &[
    IconSeg::M(0.44, -0.46),
    IconSeg::C(0.42, -0.4778, 0.3983, -0.4894, 0.375, -0.495),
    IconSeg::C(0.3517, -0.5006, 0.3278, -0.5011, 0.3033, -0.4967),
    IconSeg::L(-0.08, -0.4267),
    IconSeg::C(-0.1289, -0.4156, -0.1694, -0.3911, -0.2017, -0.3533),
    IconSeg::C(-0.2339, -0.3156, -0.25, -0.2711, -0.25, -0.22),
    IconSeg::L(-0.25, 0.19),
    IconSeg::C(-0.2767, 0.1744, -0.3044, 0.1667, -0.3333, 0.1667),
    IconSeg::C(-0.38, 0.1667, -0.4194, 0.1828, -0.4517, 0.215),
    IconSeg::C(-0.4839, 0.2472, -0.5, 0.2867, -0.5, 0.3333),
    IconSeg::C(-0.5, 0.38, -0.4839, 0.4194, -0.4517, 0.4517),
    IconSeg::C(-0.4194, 0.4839, -0.38, 0.5, -0.3333, 0.5),
    IconSeg::C(-0.2867, 0.5, -0.2472, 0.4839, -0.215, 0.4517),
    IconSeg::C(-0.1828, 0.4194, -0.1667, 0.38, -0.1667, 0.3333),
    IconSeg::L(-0.1667, -0.0467),
    IconSeg::C(-0.1667, -0.0667, -0.16, -0.0844, -0.1467, -0.1),
    IconSeg::C(-0.1333, -0.1156, -0.1178, -0.1256, -0.1, -0.13),
    IconSeg::L(0.3667, -0.2167),
    IconSeg::C(0.38, -0.2189, 0.3917, -0.2156, 0.4017, -0.2067),
    IconSeg::C(0.4117, -0.1978, 0.4167, -0.1867, 0.4167, -0.1733),
    IconSeg::L(0.4167, 0.0633),
    IconSeg::C(0.39, 0.05, 0.3622, 0.0422, 0.3333, 0.04),
    IconSeg::C(0.2867, 0.04, 0.2472, 0.0567, 0.215, 0.09),
    IconSeg::C(0.1828, 0.1233, 0.1667, 0.1628, 0.1667, 0.2083),
    IconSeg::C(0.1667, 0.2539, 0.1828, 0.2928, 0.215, 0.325),
    IconSeg::C(0.2472, 0.3572, 0.2867, 0.3733, 0.3333, 0.3733),
    IconSeg::C(0.38, 0.3733, 0.4194, 0.3572, 0.4517, 0.325),
    IconSeg::C(0.4839, 0.2928, 0.5, 0.2533, 0.5, 0.2067),
    IconSeg::L(0.5, -0.3333),
    IconSeg::C(0.5, -0.3578, 0.495, -0.3811, 0.485, -0.4033),
    IconSeg::C(0.475, -0.4256, 0.46, -0.4444, 0.44, -0.46),
    IconSeg::Z,
];
/// Flaticon UIcons solid-rounded `clipboard` (U+F437).
pub(crate) const CLIPBOARD: &[IconSeg] = &[
    IconSeg::M(0.0417, -0.3333),
    IconSeg::L(-0.0417, -0.3333),
    IconSeg::C(-0.0661, -0.3333, -0.0867, -0.3417, -0.1033, -0.3583),
    IconSeg::C(-0.12, -0.375, -0.1283, -0.3944, -0.1283, -0.4167),
    IconSeg::C(-0.1283, -0.4389, -0.12, -0.4583, -0.1033, -0.475),
    IconSeg::C(-0.0867, -0.4917, -0.0661, -0.5, -0.0417, -0.5),
    IconSeg::L(0.0417, -0.5),
    IconSeg::C(0.0639, -0.5, 0.0833, -0.4917, 0.1, -0.475),
    IconSeg::C(0.1167, -0.4583, 0.125, -0.4389, 0.125, -0.4167),
    IconSeg::C(0.125, -0.3944, 0.1167, -0.375, 0.1, -0.3583),
    IconSeg::C(0.0833, -0.3417, 0.0639, -0.3333, 0.0417, -0.3333),
    IconSeg::Z,
    IconSeg::M(0.205, -0.4133),
    IconSeg::C(0.205, -0.3667, 0.1889, -0.3278, 0.1567, -0.2967),
    IconSeg::C(0.1244, -0.2656, 0.0861, -0.25, 0.0417, -0.25),
    IconSeg::L(-0.0417, -0.25),
    IconSeg::C(-0.0883, -0.25, -0.1278, -0.2656, -0.16, -0.2967),
    IconSeg::C(-0.1922, -0.3278, -0.2083, -0.3667, -0.2083, -0.4133),
    IconSeg::C(-0.2572, -0.4022, -0.2972, -0.3778, -0.3283, -0.34),
    IconSeg::C(-0.3594, -0.3022, -0.375, -0.2578, -0.375, -0.2067),
    IconSeg::L(-0.375, 0.29),
    IconSeg::C(-0.375, 0.3278, -0.3661, 0.3628, -0.3483, 0.395),
    IconSeg::C(-0.3306, 0.4272, -0.3056, 0.4528, -0.2733, 0.4717),
    IconSeg::C(-0.2411, 0.4906, -0.2061, 0.5, -0.1683, 0.5),
    IconSeg::L(0.165, 0.5),
    IconSeg::C(0.2028, 0.5, 0.2378, 0.4906, 0.27, 0.4717),
    IconSeg::C(0.3022, 0.4528, 0.3278, 0.4272, 0.3467, 0.395),
    IconSeg::C(0.3656, 0.3628, 0.375, 0.3278, 0.375, 0.29),
    IconSeg::L(0.375, -0.21),
    IconSeg::C(0.375, -0.2589, 0.3589, -0.3022, 0.3267, -0.34),
    IconSeg::C(0.2944, -0.3778, 0.2539, -0.4022, 0.205, -0.4133),
    IconSeg::Z,
];
/// Flaticon UIcons solid-rounded `palette` (U+F9DC): Settings window, Appearance.
pub(crate) const PALETTE: &[IconSeg] = &[
    IconSeg::M(0.3331, 0.2001),
    IconSeg::L(0.3364, 0.2034),
    IconSeg::C(0.3453, 0.2145, 0.3564, 0.2217, 0.3698, 0.2251),
    IconSeg::C(0.3831, 0.2284, 0.3964, 0.229, 0.4097, 0.2267),
    IconSeg::C(0.4231, 0.2245, 0.4347, 0.219, 0.4447, 0.2101),
    IconSeg::C(0.4547, 0.2012, 0.462, 0.1901, 0.4664, 0.1768),
    IconSeg::C(0.4908, 0.1168, 0.5019, 0.0535, 0.4997, -0.0132),
    IconSeg::C(0.4975, -0.0998, 0.4742, -0.1793, 0.4297, -0.2515),
    IconSeg::C(0.3853, -0.3237, 0.327, -0.382, 0.2548, -0.4264),
    IconSeg::C(0.1826, -0.4708, 0.1032, -0.4953, 0.0165, -0.4997),
    IconSeg::C(-0.0501, -0.5019, -0.1151, -0.4908, -0.1784, -0.4664),
    IconSeg::C(-0.2417, -0.442, -0.2978, -0.4064, -0.3467, -0.3598),
    IconSeg::C(-0.3956, -0.3131, -0.4334, -0.2587, -0.46, -0.1965),
    IconSeg::C(-0.4867, -0.1343, -0.5, -0.0687, -0.5, 0.0001),
    IconSeg::C(-0.5, 0.0912, -0.4778, 0.1751, -0.4334, 0.2517),
    IconSeg::C(-0.3889, 0.3284, -0.3284, 0.3889, -0.2517, 0.4334),
    IconSeg::C(-0.1751, 0.4778, -0.0912, 0.5, -0.0001, 0.5),
    IconSeg::L(0.0432, 0.4967),
    IconSeg::C(0.0521, 0.4967, 0.0604, 0.4928, 0.0682, 0.485),
    IconSeg::C(0.076, 0.4772, 0.0798, 0.4678, 0.0798, 0.4567),
    IconSeg::L(0.0798, 0.3067),
    IconSeg::C(0.0776, 0.2756, 0.0848, 0.2478, 0.1015, 0.2234),
    IconSeg::C(0.1182, 0.199, 0.1404, 0.1806, 0.1681, 0.1684),
    IconSeg::C(0.1959, 0.1562, 0.2248, 0.1529, 0.2548, 0.1584),
    IconSeg::C(0.2848, 0.164, 0.3109, 0.1779, 0.3331, 0.2001),
    IconSeg::Z,
    IconSeg::M(0.2098, -0.1631),
    IconSeg::C(0.2254, -0.1676, 0.2409, -0.1659, 0.2565, -0.1582),
    IconSeg::C(0.272, -0.1504, 0.282, -0.1382, 0.2864, -0.1215),
    IconSeg::C(0.2909, -0.1048, 0.2887, -0.0887, 0.2798, -0.0732),
    IconSeg::C(0.2709, -0.0576, 0.2581, -0.0476, 0.2415, -0.0432),
    IconSeg::C(0.2248, -0.0387, 0.2092, -0.041, 0.1948, -0.0498),
    IconSeg::C(0.1804, -0.0587, 0.1709, -0.0715, 0.1665, -0.0882),
    IconSeg::C(0.162, -0.1048, 0.1637, -0.1204, 0.1715, -0.1348),
    IconSeg::C(0.1793, -0.1493, 0.192, -0.1587, 0.2098, -0.1631),
    IconSeg::Z,
    IconSeg::M(-0.1734, 0.2067),
    IconSeg::C(-0.1912, 0.2112, -0.2073, 0.209, -0.2217, 0.2001),
    IconSeg::C(-0.2362, 0.1912, -0.2456, 0.1784, -0.2501, 0.1618),
    IconSeg::C(-0.2545, 0.1451, -0.2528, 0.1295, -0.2451, 0.1151),
    IconSeg::C(-0.2373, 0.1007, -0.2251, 0.0912, -0.2084, 0.0868),
    IconSeg::C(-0.1918, 0.0823, -0.1756, 0.084, -0.1601, 0.0918),
    IconSeg::C(-0.1445, 0.0996, -0.1345, 0.1118, -0.1301, 0.1284),
    IconSeg::C(-0.1257, 0.1451, -0.1279, 0.1612, -0.1368, 0.1768),
    IconSeg::C(-0.1457, 0.1923, -0.1579, 0.2023, -0.1734, 0.2067),
    IconSeg::Z,
    IconSeg::M(-0.1734, -0.0432),
    IconSeg::C(-0.1912, -0.0387, -0.2073, -0.041, -0.2217, -0.0498),
    IconSeg::C(-0.2362, -0.0587, -0.2456, -0.0715, -0.2501, -0.0882),
    IconSeg::C(-0.2545, -0.1048, -0.2528, -0.1204, -0.2451, -0.1348),
    IconSeg::C(-0.2373, -0.1493, -0.2251, -0.1587, -0.2084, -0.1631),
    IconSeg::C(-0.1918, -0.1676, -0.1756, -0.1659, -0.1601, -0.1582),
    IconSeg::C(-0.1445, -0.1504, -0.1345, -0.1382, -0.1301, -0.1215),
    IconSeg::C(-0.1257, -0.1048, -0.1279, -0.0887, -0.1368, -0.0732),
    IconSeg::C(-0.1457, -0.0576, -0.1579, -0.0476, -0.1734, -0.0432),
    IconSeg::Z,
    IconSeg::M(0.0332, -0.1698),
    IconSeg::C(0.0176, -0.1654, 0.0021, -0.167, -0.0135, -0.1748),
    IconSeg::C(-0.029, -0.1826, -0.039, -0.1948, -0.0435, -0.2115),
    IconSeg::C(-0.0479, -0.2281, -0.0457, -0.2442, -0.0368, -0.2598),
    IconSeg::C(-0.0279, -0.2753, -0.0151, -0.2853, 0.0015, -0.2898),
    IconSeg::C(0.0182, -0.2942, 0.0337, -0.292, 0.0482, -0.2831),
    IconSeg::C(0.0626, -0.2742, 0.0721, -0.2615, 0.0765, -0.2448),
    IconSeg::C(0.0809, -0.2281, 0.0793, -0.2126, 0.0715, -0.1981),
    IconSeg::C(0.0637, -0.1837, 0.051, -0.1743, 0.0332, -0.1698),
    IconSeg::Z,
];
/// Flaticon UIcons solid-rounded `info` (U+F7FF): Settings window, About.
pub(crate) const INFO: &[IconSeg] = &[
    IconSeg::M(0.0, 0.5),
    IconSeg::C(0.0911, 0.5, 0.175, 0.4778, 0.2517, 0.4333),
    IconSeg::C(0.3283, 0.3889, 0.3889, 0.3283, 0.4333, 0.2517),
    IconSeg::C(0.4778, 0.175, 0.5, 0.0911, 0.5, -0.0),
    IconSeg::C(0.5, -0.0911, 0.4778, -0.175, 0.4333, -0.2517),
    IconSeg::C(0.3889, -0.3283, 0.3283, -0.3889, 0.2517, -0.4333),
    IconSeg::C(0.175, -0.4778, 0.0911, -0.5, 0.0, -0.5),
    IconSeg::C(-0.0911, -0.5, -0.175, -0.4778, -0.2517, -0.4333),
    IconSeg::C(-0.3283, -0.3889, -0.3889, -0.3283, -0.4333, -0.2517),
    IconSeg::C(-0.4778, -0.175, -0.5, -0.0911, -0.5, -0.0),
    IconSeg::C(-0.5, 0.0911, -0.4778, 0.175, -0.4333, 0.2517),
    IconSeg::C(-0.3889, 0.3283, -0.3283, 0.3889, -0.2517, 0.4333),
    IconSeg::C(-0.175, 0.4778, -0.0911, 0.5, 0.0, 0.5),
    IconSeg::Z,
    IconSeg::M(0.0, -0.2933),
    IconSeg::C(0.0178, -0.2911, 0.0328, -0.2844, 0.045, -0.2733),
    IconSeg::C(0.0572, -0.2622, 0.0633, -0.2478, 0.0633, -0.23),
    IconSeg::C(0.0633, -0.2122, 0.0572, -0.1972, 0.045, -0.185),
    IconSeg::C(0.0328, -0.1728, 0.0178, -0.1667, 0.0, -0.1667),
    IconSeg::C(-0.0178, -0.1667, -0.0328, -0.1728, -0.045, -0.185),
    IconSeg::C(-0.0572, -0.1972, -0.0633, -0.2122, -0.0633, -0.23),
    IconSeg::C(-0.0633, -0.2478, -0.0572, -0.2628, -0.045, -0.275),
    IconSeg::C(-0.0328, -0.2872, -0.0178, -0.2933, 0.0, -0.2933),
    IconSeg::Z,
    IconSeg::M(-0.04, -0.0833),
    IconSeg::L(0.0, -0.0833),
    IconSeg::C(0.0222, -0.0833, 0.0417, -0.075, 0.0583, -0.0583),
    IconSeg::C(0.075, -0.0417, 0.0833, -0.0222, 0.0833, -0.0),
    IconSeg::L(0.0833, 0.25),
    IconSeg::C(0.0833, 0.2611, 0.0794, 0.2706, 0.0717, 0.2783),
    IconSeg::C(0.0639, 0.2861, 0.0539, 0.29, 0.0417, 0.29),
    IconSeg::C(0.0294, 0.29, 0.0194, 0.2861, 0.0117, 0.2783),
    IconSeg::C(0.0039, 0.2706, 0.0, 0.2611, 0.0, 0.25),
    IconSeg::L(0.0, -0.0),
    IconSeg::L(-0.04, -0.0),
    IconSeg::C(-0.0533, -0.0, -0.0639, -0.0039, -0.0717, -0.0117),
    IconSeg::C(-0.0794, -0.0194, -0.0833, -0.0294, -0.0833, -0.0417),
    IconSeg::C(-0.0833, -0.0539, -0.0794, -0.0639, -0.0717, -0.0717),
    IconSeg::C(-0.0639, -0.0794, -0.0533, -0.0833, -0.04, -0.0833),
    IconSeg::Z,
];
/// Flaticon UIcons solid-rounded `settings` (U+FBAC): the header's Settings icon.
/// (Outline coordinates; a value near 1/pi is coincidence.)
#[allow(clippy::approx_constant)]
pub(crate) const GEAR: &[IconSeg] = &[
    IconSeg::M(-0.4333, 0.25),
    IconSeg::C(-0.4156, 0.2789, -0.39, 0.2983, -0.3567, 0.3083),
    IconSeg::C(-0.3233, 0.3183, -0.2922, 0.3144, -0.2633, 0.2967),
    IconSeg::L(-0.2433, 0.2867),
    IconSeg::C(-0.2078, 0.3156, -0.1678, 0.3378, -0.1233, 0.3533),
    IconSeg::L(-0.1233, 0.3733),
    IconSeg::C(-0.1256, 0.4089, -0.1144, 0.4389, -0.09, 0.4633),
    IconSeg::C(-0.0656, 0.4878, -0.0356, 0.5, 0.0, 0.5),
    IconSeg::C(0.0356, 0.5, 0.0656, 0.4878, 0.09, 0.4633),
    IconSeg::C(0.1144, 0.4389, 0.1256, 0.4089, 0.1233, 0.3733),
    IconSeg::L(0.1233, 0.3533),
    IconSeg::C(0.1678, 0.3378, 0.2078, 0.3156, 0.2433, 0.2867),
    IconSeg::L(0.2633, 0.2967),
    IconSeg::C(0.2922, 0.3144, 0.3233, 0.3183, 0.3567, 0.3083),
    IconSeg::C(0.39, 0.2983, 0.4156, 0.2789, 0.4333, 0.25),
    IconSeg::C(0.4511, 0.2211, 0.4556, 0.19, 0.4467, 0.1567),
    IconSeg::C(0.4378, 0.1233, 0.4178, 0.0978, 0.3867, 0.08),
    IconSeg::L(0.37, 0.07),
    IconSeg::C(0.3767, 0.0233, 0.3767, -0.0233, 0.37, -0.07),
    IconSeg::L(0.3867, -0.08),
    IconSeg::C(0.4178, -0.0978, 0.4378, -0.1233, 0.4467, -0.1567),
    IconSeg::C(0.4556, -0.19, 0.4511, -0.2211, 0.4333, -0.25),
    IconSeg::C(0.4156, -0.2789, 0.39, -0.2983, 0.3567, -0.3083),
    IconSeg::C(0.3233, -0.3183, 0.2922, -0.3144, 0.2633, -0.2967),
    IconSeg::L(0.2433, -0.2867),
    IconSeg::C(0.2078, -0.3156, 0.1678, -0.3378, 0.1233, -0.3533),
    IconSeg::L(0.1233, -0.3767),
    IconSeg::C(0.1233, -0.41, 0.1117, -0.4389, 0.0883, -0.4633),
    IconSeg::C(0.065, -0.4878, 0.0356, -0.5, 0.0, -0.5),
    IconSeg::C(-0.0356, -0.5, -0.065, -0.4878, -0.0883, -0.4633),
    IconSeg::C(-0.1117, -0.4389, -0.1233, -0.4089, -0.1233, -0.3733),
    IconSeg::L(-0.1233, -0.3533),
    IconSeg::C(-0.1678, -0.3378, -0.2078, -0.3156, -0.2433, -0.2867),
    IconSeg::L(-0.2633, -0.2967),
    IconSeg::C(-0.2922, -0.3144, -0.3233, -0.3183, -0.3567, -0.3083),
    IconSeg::C(-0.39, -0.2983, -0.4156, -0.2789, -0.4333, -0.25),
    IconSeg::C(-0.4511, -0.2211, -0.4556, -0.19, -0.4467, -0.1567),
    IconSeg::C(-0.4378, -0.1233, -0.4178, -0.0978, -0.3867, -0.08),
    IconSeg::L(-0.3867, -0.08),
    IconSeg::L(-0.37, -0.07),
    IconSeg::C(-0.3767, -0.0233, -0.3767, 0.0233, -0.37, 0.07),
    IconSeg::L(-0.3867, 0.08),
    IconSeg::C(-0.4178, 0.0978, -0.4378, 0.1233, -0.4467, 0.1567),
    IconSeg::C(-0.4556, 0.19, -0.4511, 0.2211, -0.4333, 0.25),
    IconSeg::Z,
    IconSeg::M(0.0, -0.1667),
    IconSeg::C(0.0467, -0.1667, 0.0861, -0.1506, 0.1183, -0.1183),
    IconSeg::C(0.1506, -0.0861, 0.1667, -0.0467, 0.1667, -0.0),
    IconSeg::C(0.1667, 0.0467, 0.1506, 0.0861, 0.1183, 0.1183),
    IconSeg::C(0.0861, 0.1506, 0.0467, 0.1667, 0.0, 0.1667),
    IconSeg::C(-0.0467, 0.1667, -0.0861, 0.1506, -0.1183, 0.1183),
    IconSeg::C(-0.1506, 0.0861, -0.1667, 0.0467, -0.1667, -0.0),
    IconSeg::C(-0.1667, -0.0467, -0.1506, -0.0861, -0.1183, -0.1183),
    IconSeg::C(-0.0861, -0.1506, -0.0467, -0.1667, 0.0, -0.1667),
    IconSeg::Z,
];
/// Clipboard row copy button: two overlapping rounded squares, the back one
/// cut by the front (filled, even-odd), from a 20 x 20 SVG path (13 x 13 icon
/// box) with its arcs converted to cubics.
const COPY: &[IconSeg] = &[
    IconSeg::M(0.3274, -0.4990),
    IconSeg::C(0.4255, -0.4889, 0.5000, -0.4063, 0.5000, -0.3077),
    IconSeg::L(0.5000, 0.0000),
    IconSeg::L(0.4990, 0.0197),
    IconSeg::C(0.4897, 0.1103, 0.4180, 0.1820, 0.3274, 0.1913),
    IconSeg::L(0.3077, 0.1923),
    IconSeg::L(0.1923, 0.1923),
    IconSeg::L(0.1923, 0.3077),
    IconSeg::C(0.1923, 0.4139, 0.1062, 0.5000, 0.0000, 0.5000),
    IconSeg::L(-0.3077, 0.5000),
    IconSeg::C(-0.4139, 0.5000, -0.5000, 0.4139, -0.5000, 0.3077),
    IconSeg::L(-0.5000, 0.0000),
    IconSeg::C(-0.5000, -0.1062, -0.4139, -0.1923, -0.3077, -0.1923),
    IconSeg::L(-0.1923, -0.1923),
    IconSeg::L(-0.1923, -0.3077),
    IconSeg::C(-0.1923, -0.4139, -0.1062, -0.5000, 0.0000, -0.5000),
    IconSeg::L(0.3077, -0.5000),
    IconSeg::Z,
    IconSeg::M(0.0000, -0.3846),
    IconSeg::C(-0.0425, -0.3846, -0.0769, -0.3502, -0.0769, -0.3077),
    IconSeg::L(-0.0769, -0.1923),
    IconSeg::L(0.0000, -0.1923),
    IconSeg::C(0.1062, -0.1923, 0.1923, -0.1062, 0.1923, 0.0000),
    IconSeg::L(0.1923, 0.0769),
    IconSeg::L(0.3077, 0.0769),
    IconSeg::C(0.3502, 0.0769, 0.3846, 0.0425, 0.3846, 0.0000),
    IconSeg::L(0.3846, -0.3077),
    IconSeg::C(0.3846, -0.3502, 0.3502, -0.3846, 0.3077, -0.3846),
    IconSeg::Z,
];
/// Flaticon UIcons solid-rounded `trash` (U+FDDF): clipboard row delete button.
const TRASH: &[IconSeg] = &[
    IconSeg::M(0.3767, -0.3333),
    IconSeg::L(0.2467, -0.3333),
    IconSeg::C(0.2356, -0.3822, 0.2111, -0.4222, 0.1733, -0.4533),
    IconSeg::C(0.1356, -0.4844, 0.0911, -0.5, 0.04, -0.5),
    IconSeg::L(-0.0433, -0.5),
    IconSeg::C(-0.0922, -0.5, -0.1356, -0.4844, -0.1733, -0.4533),
    IconSeg::C(-0.2111, -0.4222, -0.2356, -0.3822, -0.2467, -0.3333),
    IconSeg::L(-0.3733, -0.3333),
    IconSeg::C(-0.3867, -0.3333, -0.3972, -0.3294, -0.405, -0.3217),
    IconSeg::C(-0.4128, -0.3139, -0.4167, -0.3039, -0.4167, -0.2917),
    IconSeg::C(-0.4167, -0.2794, -0.4128, -0.2694, -0.405, -0.2617),
    IconSeg::C(-0.3972, -0.2539, -0.3878, -0.25, -0.3767, -0.25),
    IconSeg::L(-0.3333, -0.25),
    IconSeg::L(-0.3333, 0.29),
    IconSeg::C(-0.3333, 0.3278, -0.3239, 0.3628, -0.305, 0.395),
    IconSeg::C(-0.2861, 0.4272, -0.2606, 0.4528, -0.2283, 0.4717),
    IconSeg::C(-0.1961, 0.4906, -0.1622, 0.5, -0.1267, 0.5),
    IconSeg::L(0.1267, 0.5),
    IconSeg::C(0.1622, 0.5, 0.1961, 0.4906, 0.2283, 0.4717),
    IconSeg::C(0.2606, 0.4528, 0.2861, 0.4272, 0.305, 0.395),
    IconSeg::C(0.3239, 0.3628, 0.3333, 0.3278, 0.3333, 0.29),
    IconSeg::L(0.3333, -0.25),
    IconSeg::L(0.3767, -0.25),
    IconSeg::C(0.3878, -0.25, 0.3972, -0.2539, 0.405, -0.2617),
    IconSeg::C(0.4128, -0.2694, 0.4167, -0.2794, 0.4167, -0.2917),
    IconSeg::C(0.4167, -0.3039, 0.4128, -0.3139, 0.405, -0.3217),
    IconSeg::C(0.3972, -0.3294, 0.3878, -0.3333, 0.3767, -0.3333),
    IconSeg::Z,
    IconSeg::M(-0.04, 0.21),
    IconSeg::C(-0.04, 0.2211, -0.0444, 0.2306, -0.0533, 0.2383),
    IconSeg::C(-0.0622, 0.2461, -0.0722, 0.25, -0.0833, 0.25),
    IconSeg::C(-0.0944, 0.25, -0.1044, 0.2461, -0.1133, 0.2383),
    IconSeg::C(-0.1222, 0.2306, -0.1267, 0.2211, -0.1267, 0.21),
    IconSeg::L(-0.1267, -0.04),
    IconSeg::C(-0.1244, -0.0533, -0.1194, -0.0639, -0.1117, -0.0717),
    IconSeg::C(-0.1039, -0.0794, -0.0944, -0.0833, -0.0833, -0.0833),
    IconSeg::C(-0.0722, -0.0833, -0.0628, -0.0794, -0.055, -0.0717),
    IconSeg::C(-0.0472, -0.0639, -0.0433, -0.0533, -0.0433, -0.04),
    IconSeg::L(-0.0433, 0.21),
    IconSeg::Z,
    IconSeg::M(0.1267, 0.21),
    IconSeg::C(0.1267, 0.2211, 0.1222, 0.2306, 0.1133, 0.2383),
    IconSeg::C(0.1044, 0.2461, 0.0944, 0.25, 0.0833, 0.25),
    IconSeg::C(0.0722, 0.25, 0.0628, 0.2461, 0.055, 0.2383),
    IconSeg::C(0.0472, 0.2306, 0.0433, 0.2211, 0.0433, 0.21),
    IconSeg::L(0.0433, -0.04),
    IconSeg::C(0.0433, -0.0533, 0.0472, -0.0639, 0.055, -0.0717),
    IconSeg::C(0.0628, -0.0794, 0.0722, -0.0833, 0.0833, -0.0833),
    IconSeg::C(0.0944, -0.0833, 0.1044, -0.0794, 0.1133, -0.0717),
    IconSeg::C(0.1222, -0.0639, 0.1267, -0.0533, 0.1267, -0.04),
    IconSeg::Z,
    IconSeg::M(-0.16, -0.3333),
    IconSeg::C(-0.1511, -0.3578, -0.1356, -0.3778, -0.1133, -0.3933),
    IconSeg::C(-0.0911, -0.4089, -0.0667, -0.4167, -0.04, -0.4167),
    IconSeg::L(0.0433, -0.4167),
    IconSeg::C(0.0678, -0.4167, 0.0911, -0.4089, 0.1133, -0.3933),
    IconSeg::C(0.1356, -0.3778, 0.1511, -0.3578, 0.16, -0.3333),
    IconSeg::Z,
];
/// Drop page glyph: "Inbox" by Jivan from the Noun Project (noun 1052548),
/// a tray outline with the drop slot cut out (even-odd fill).
const INBOX: &[IconSeg] = &[
    IconSeg::M(0.3836, -0.2868),
    IconSeg::C(0.3646, -0.326, 0.3254, -0.3498, 0.2815, -0.3498),
    IconSeg::L(-0.2827, -0.3498),
    IconSeg::C(-0.3266, -0.3498, -0.3646, -0.326, -0.3848, -0.2868),
    IconSeg::L(-0.4941, -0.0659),
    IconSeg::C(-0.4976, -0.0588, -0.5, -0.0517, -0.5, -0.0445),
    IconSeg::C(-0.5, -0.0445, -0.5, -0.0445, -0.5, -0.0445),
    IconSeg::C(-0.5, -0.0445, -0.5, -0.0445, -0.5, -0.0445),
    IconSeg::C(-0.5, -0.0445, -0.5, -0.0445, -0.5, -0.0445),
    IconSeg::C(-0.5, -0.0445, -0.5, -0.0445, -0.5, -0.0445),
    IconSeg::L(-0.5, 0.2548),
    IconSeg::C(-0.5, 0.307, -0.4572, 0.3498, -0.405, 0.3498),
    IconSeg::L(0.405, 0.3498),
    IconSeg::C(0.4572, 0.3498, 0.5, 0.307, 0.5, 0.2548),
    IconSeg::L(0.5, -0.0433),
    IconSeg::C(0.5, -0.0433, 0.5, -0.0433, 0.5, -0.0433),
    IconSeg::C(0.5, -0.0433, 0.5, -0.0433, 0.5, -0.0433),
    IconSeg::C(0.5, -0.0433, 0.5, -0.0433, 0.5, -0.0433),
    IconSeg::C(0.5, -0.0433, 0.5, -0.0433, 0.5, -0.0433),
    IconSeg::C(0.5, -0.0517, 0.4976, -0.0588, 0.4941, -0.0647),
    IconSeg::L(0.3836, -0.2868),
    IconSeg::Z,
    IconSeg::M(-0.2993, -0.2441),
    IconSeg::C(-0.2957, -0.25, -0.2898, -0.2548, -0.2827, -0.2548),
    IconSeg::L(0.2815, -0.2548),
    IconSeg::C(0.2886, -0.2548, 0.2945, -0.2512, 0.2981, -0.2441),
    IconSeg::L(0.3741, -0.0909),
    IconSeg::L(0.1936, -0.0909),
    IconSeg::C(0.171, -0.0909, 0.152, -0.0754, 0.1473, -0.0529),
    IconSeg::C(0.1461, -0.0481, 0.1211, 0.0707, -0.0, 0.0707),
    IconSeg::C(-0.1176, 0.0707, -0.1449, -0.0398, -0.1473, -0.0529),
    IconSeg::C(-0.152, -0.0754, -0.171, -0.0909, -0.1936, -0.0909),
    IconSeg::L(-0.3753, -0.0909),
    IconSeg::L(-0.2993, -0.2441),
    IconSeg::Z,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NotchState;
    use crate::layout::{resolve_settings_layout, resolve_space_selector_in};

    #[test]
    fn test_renderer_initialization_and_drawing() {
        let renderer = Renderer::new().expect("Failed to initialize Direct2D renderer");
        let dims = NotchDimensions::from_dpi(96);
        let width = dims.width;
        let height = dims.height;
        let border_width = dims.border_width;

        unsafe {
            let screen_dc = GetDC(None);
            assert!(!screen_dc.is_invalid());
            let mem_dc = CreateCompatibleDC(Some(screen_dc));
            assert!(!mem_dc.is_invalid());

            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                },
                bmiColors: [RGBQUAD::default()],
            };

            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib = CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
                .expect("Failed to create DIB section");
            let old_bitmap = SelectObject(mem_dc, dib.into());

            let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 0.0,
                dpiY: 0.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };

            let rt = renderer
                .d2d_factory
                .CreateDCRenderTarget(&rt_props)
                .expect("Failed to create DC render target");
            let bind_rect = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            rt.BindDC(mem_dc, &bind_rect).expect("Failed to bind DC");

            rt.BeginDraw();
            rt.Clear(Some(&COLOR_TRANSPARENT));

            let fill_brush = rt
                .CreateSolidColorBrush(&NOTCH_BG_COLOR, None)
                .expect("Failed to create fill brush");
            let border_brush = rt
                .CreateSolidColorBrush(&NOTCH_BORDER_COLOR, None)
                .expect("Failed to create border brush");

            let path = renderer
                .create_notch_geometry(&dims)
                .expect("Failed to create notch path geometry");

            rt.FillGeometry(&path, &fill_brush, None);
            rt.DrawGeometry(&path, &border_brush, border_width, None);

            let layout = resolve_layout(&dims);
            let clock_state =
                ClockDateState::from_pure_components(2026, 10, 5, 1, 17, 42, 0, 0, false);
            if let ResolvedLayout::Collapsed { components, .. } = &layout {
                renderer
                    .render_collapsed_content(&rt, components, &dims, &clock_state.formatted_time)
                    .expect("render collapsed content failed");
            } else {
                panic!("Expected ResolvedLayout::Collapsed");
            }

            rt.EndDraw(None, None).expect("EndDraw failed");

            // Verify rendered pixels
            assert!(!bits.is_null());
            let pixel_slice =
                std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize);

            // Left area (x=50, y=16) inside the notch surface to the left of the centered clock must be solid black (alpha=255, r=0, g=0, b=0)
            let left_safe_idx = ((16 * width + 50) * 4) as usize;
            assert_eq!(
                pixel_slice[left_safe_idx + 3],
                255,
                "Inside notch pixel should have alpha = 255"
            );
            assert_eq!(pixel_slice[left_safe_idx], 0, "Blue should be 0");
            assert_eq!(pixel_slice[left_safe_idx + 1], 0, "Green should be 0");
            assert_eq!(pixel_slice[left_safe_idx + 2], 0, "Red should be 0");

            // Right area (x=170, y=16) inside the notch surface to the right of the centered clock must be solid black (alpha=255, r=0, g=0, b=0)
            let right_safe_idx = ((16 * width + 170) * 4) as usize;
            assert_eq!(
                pixel_slice[right_safe_idx + 3],
                255,
                "Right inside notch pixel should have alpha = 255"
            );
            assert_eq!(pixel_slice[right_safe_idx], 0, "Blue should be 0");
            assert_eq!(pixel_slice[right_safe_idx + 1], 0, "Green should be 0");
            assert_eq!(pixel_slice[right_safe_idx + 2], 0, "Red should be 0");

            // Check that old emblem area (x in 20..40, y in 8..24) contains ZERO non-black pixels (emblem completely removed)
            let mut old_emblem_non_black_count = 0;
            for y in 8..24 {
                for x in 20..40 {
                    let idx = ((y * width + x) * 4) as usize;
                    let r = pixel_slice[idx + 2];
                    let g = pixel_slice[idx + 1];
                    let b = pixel_slice[idx];
                    if r > 0 || g > 0 || b > 0 {
                        old_emblem_non_black_count += 1;
                    }
                }
            }
            assert_eq!(
                old_emblem_non_black_count, 0,
                "Old emblem area (x in 20..40) must be pure black with emblem removed (found {old_emblem_non_black_count} non-black pixels)"
            );

            // Check that centered live time is actually rendered in the center region (x in 70..150, y in 8..24)
            let mut center_time_pixel_count = 0;
            for y in 8..24 {
                for x in 70..150 {
                    let idx = ((y * width + x) * 4) as usize;
                    let r = pixel_slice[idx + 2];
                    let g = pixel_slice[idx + 1];
                    let b = pixel_slice[idx];
                    if r > 20 || g > 20 || b > 20 {
                        center_time_pixel_count += 1;
                    }
                }
            }
            assert!(
                center_time_pixel_count > 25,
                "Collapsed surface should contain rendered centered time text (found {center_time_pixel_count} pixels)"
            );

            // Top screen attachment pixel (x=110, y=0) must be attached and opaque (alpha=255)
            let top_center_idx = (110 * 4) as usize;
            assert_eq!(
                pixel_slice[top_center_idx + 3],
                255,
                "Top attachment center should be opaque"
            );

            // Corner pixel (x=1, y=1) outside shoulder curve must be transparent (alpha=0)
            let corner_idx = ((width + 1) * 4) as usize;
            assert_eq!(
                pixel_slice[corner_idx + 3],
                0,
                "Top-left corner pixel outside shoulder radius should be transparent"
            );

            // Outside left wall (x=2, y=16) must be transparent (alpha=0)
            let left_outside_idx = ((16 * width + 2) * 4) as usize;
            assert_eq!(
                pixel_slice[left_outside_idx + 3],
                0,
                "Pixel to the left of the notch body should be transparent"
            );

            // Save rendered notch bitmap to artifact directory for visual verification
            let bmp_path = r"C:\Users\ujval\.gemini\antigravity-ide\brain\402578f9-d9e5-44d8-9b61-4772077cf14b\notch_hardware_notch.bmp";
            let mut file_bytes = Vec::with_capacity(54 + pixel_slice.len());
            file_bytes.extend_from_slice(b"BM");
            let file_size = (54 + pixel_slice.len()) as u32;
            file_bytes.extend_from_slice(&file_size.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&54u32.to_le_bytes());
            file_bytes.extend_from_slice(&40u32.to_le_bytes());
            file_bytes.extend_from_slice(&width.to_le_bytes());
            file_bytes.extend_from_slice(&(-height).to_le_bytes());
            file_bytes.extend_from_slice(&1u16.to_le_bytes());
            file_bytes.extend_from_slice(&32u16.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&(pixel_slice.len() as u32).to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(pixel_slice);
            let _ = std::fs::write(bmp_path, file_bytes);

            SelectObject(mem_dc, old_bitmap);
            let _ = DeleteObject(dib.into());
            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen_dc);
        }
    }

    #[test]
    fn test_renderer_expanded_state_drawing() {
        let renderer = Renderer::new().expect("Failed to initialize Direct2D renderer");
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 96);
        let width = dims.width;
        let height = dims.height;
        let border_width = dims.border_width;

        assert_eq!(width, 600);
        assert_eq!(height, 138);

        unsafe {
            let screen_dc = GetDC(None);
            assert!(!screen_dc.is_invalid());
            let mem_dc = CreateCompatibleDC(Some(screen_dc));
            assert!(!mem_dc.is_invalid());

            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                },
                bmiColors: [RGBQUAD::default()],
            };

            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib = CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
                .expect("Failed to create DIB section");
            let old_bitmap = SelectObject(mem_dc, dib.into());

            let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 0.0,
                dpiY: 0.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };

            let rt = renderer
                .d2d_factory
                .CreateDCRenderTarget(&rt_props)
                .expect("Failed to create DC render target");
            let bind_rect = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            rt.BindDC(mem_dc, &bind_rect).expect("Failed to bind DC");

            rt.BeginDraw();
            rt.Clear(Some(&COLOR_TRANSPARENT));

            let fill_brush = rt
                .CreateSolidColorBrush(&NOTCH_BG_COLOR, None)
                .expect("Failed to create fill brush");
            let border_brush = rt
                .CreateSolidColorBrush(&NOTCH_BORDER_COLOR, None)
                .expect("Failed to create border brush");

            let path = renderer
                .create_notch_geometry(&dims)
                .expect("Failed to create notch path geometry");

            rt.FillGeometry(&path, &fill_brush, None);
            rt.DrawGeometry(&path, &border_brush, border_width, None);

            let clock_state =
                ClockDateState::from_pure_components(2026, 10, 6, 2, 0, 47, 0, 0, false);
            let layout = resolve_layout(&dims);
            if let ResolvedLayout::Expanded { components, .. } = &layout {
                renderer
                    .render_expanded_content(
                        &rt,
                        components,
                        &dims,
                        &clock_state.formatted_time,
                        &clock_state.formatted_date,
                    )
                    .expect("render expanded content failed");
            } else {
                panic!("Expected ResolvedLayout::Expanded");
            }

            rt.EndDraw(None, None).expect("EndDraw failed");

            // Apply soft dark ambient shadow
            if dims.shadow_opacity > 0.0 {
                let mut scratch_a = renderer.scratch_a.borrow_mut();
                let mut scratch_b = renderer.scratch_b.borrow_mut();
                apply_ambient_shadow(
                    bits as *mut u8,
                    width as usize,
                    height as usize,
                    width as usize,
                    dims.scale,
                    dims.shadow_opacity,
                    &mut scratch_a,
                    &mut scratch_b,
                );
            }

            // Verify rendered pixels
            assert!(!bits.is_null());
            let pixel_slice =
                std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize);

            // Center pixel (x=300, y=55) inside the expanded notch must be opaque (alpha=255) and pure AMOLED black (RGB=0)
            let center_idx = ((55 * width + 300) * 4) as usize;
            assert_eq!(
                pixel_slice[center_idx + 3],
                255,
                "Center pixel should have alpha = 255"
            );

            // Top screen attachment pixel (x=300, y=0) must be attached and opaque (alpha=255)
            let top_center_idx = (300 * 4) as usize;
            assert_eq!(
                pixel_slice[top_center_idx + 3],
                255,
                "Top attachment center should be opaque"
            );

            // Top-left shoulder ear (x=1, y=1) outside shoulder radius must be transparent (alpha=0)
            let corner_idx = ((width + 1) * 4) as usize;
            assert_eq!(
                pixel_slice[corner_idx + 3],
                0,
                "Top-left corner pixel outside shoulder radius should be transparent"
            );

            // Outside shadow perimeter (x=1, y=55) has near-zero or zero alpha
            let far_left_idx = ((55 * width + 1) * 4) as usize;
            assert!(
                pixel_slice[far_left_idx + 3] < 15,
                "Pixel at window boundary should be transparent/near-zero shadow"
            );

            // Ambient shadow in margin outside notch wall (x=20, y=55) has soft dark alpha
            let shadow_sample_idx = ((55 * width + 20) * 4) as usize;
            let shadow_a = pixel_slice[shadow_sample_idx + 3];
            assert!(
                shadow_a > 0 && shadow_a < 120,
                "Ambient shadow should exist outside wall with soft alpha (got {shadow_a})"
            );

            // Verify that live time and date are rendered in the center region
            let mut text_pixel_count = 0;
            for y in 30..90 {
                for x in 150..(width - 150) {
                    let idx = ((y * width + x) * 4) as usize;
                    let b = pixel_slice[idx];
                    let g = pixel_slice[idx + 1];
                    let r = pixel_slice[idx + 2];
                    let a = pixel_slice[idx + 3];
                    if a == 255 && (r > 0 || g > 0 || b > 0) {
                        text_pixel_count += 1;
                    }
                }
            }
            assert!(
                text_pixel_count > 50,
                "Expanded surface should display rendered time and date text (found {text_pixel_count} text pixels)"
            );

            // Verify left safe-area inside notch body remains pure AMOLED black background
            let mut left_text_pixel_count = 0;
            for y in 35..85 {
                for x in 50..100 {
                    let idx = ((y * width + x) * 4) as usize;
                    let b = pixel_slice[idx];
                    let g = pixel_slice[idx + 1];
                    let r = pixel_slice[idx + 2];
                    if r > 0 || g > 0 || b > 0 {
                        left_text_pixel_count += 1;
                    }
                }
            }
            assert_eq!(
                left_text_pixel_count, 0,
                "Left padding area should be pure black (no stray text pixels)"
            );

            // Save rendered expanded notch bitmap to artifact directory for visual verification
            let bmp_path = r"C:\Users\ujval\.gemini\antigravity-ide\brain\402578f9-d9e5-44d8-9b61-4772077cf14b\notch_expanded.bmp";
            let mut file_bytes = Vec::with_capacity(54 + pixel_slice.len());
            file_bytes.extend_from_slice(b"BM");
            let file_size = (54 + pixel_slice.len()) as u32;
            file_bytes.extend_from_slice(&file_size.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&54u32.to_le_bytes());
            file_bytes.extend_from_slice(&40u32.to_le_bytes());
            file_bytes.extend_from_slice(&width.to_le_bytes());
            file_bytes.extend_from_slice(&(-height).to_le_bytes());
            file_bytes.extend_from_slice(&1u16.to_le_bytes());
            file_bytes.extend_from_slice(&32u16.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&(pixel_slice.len() as u32).to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(pixel_slice);
            let _ = std::fs::write(bmp_path, file_bytes);

            SelectObject(mem_dc, old_bitmap);
            let _ = DeleteObject(dib.into());
            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen_dc);
        }
    }

    #[test]
    fn test_renderer_zero_and_negative_dimensions_safety() {
        let renderer = Renderer::new().expect("Failed to initialize Direct2D renderer");
        let hwnd = HWND::default();
        let clock = ClockDateState::default();
        let mut dims_zero = NotchDimensions::from_dpi(96);
        dims_zero.width = 0;
        dims_zero.height = 0;
        // Zero dimensions should safely return Ok(()) without attempting allocation or panic
        assert!(
            renderer
                .render(hwnd, 0, 0, &dims_zero, false, &clock)
                .is_ok()
        );

        let mut dims_negative = NotchDimensions::from_dpi(96);
        dims_negative.width = -50;
        dims_negative.height = -20;
        // Negative dimensions should also safely return Ok(())
        assert!(
            renderer
                .render(hwnd, 0, 0, &dims_negative, false, &clock)
                .is_ok()
        );
    }

    #[test]
    fn test_hovered_rendering_visual_and_subtle_highlight() {
        let renderer = Renderer::new().expect("Failed to initialize Direct2D renderer");
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, 96);
        let width = dims.width;
        let height = dims.height;

        unsafe {
            let screen_dc = GetDC(None);
            assert!(!screen_dc.is_invalid());
            let mem_dc = CreateCompatibleDC(Some(screen_dc));
            assert!(!mem_dc.is_invalid());

            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                },
                bmiColors: [RGBQUAD::default()],
            };

            // 1. Render Hovered Collapsed Notch
            let mut bits_hover: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib_hover =
                CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits_hover, None, 0)
                    .expect("Failed to create DIB section for hover test");
            let old_bmp = SelectObject(mem_dc, dib_hover.into());

            let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };

            let rt = renderer
                .d2d_factory
                .CreateDCRenderTarget(&rt_props)
                .expect("Failed to create DC render target");

            let rc = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            rt.BindDC(mem_dc, &rc).expect("BindDC failed");

            rt.BeginDraw();
            rt.Clear(Some(&COLOR_TRANSPARENT));

            let fill_brush = rt
                .CreateSolidColorBrush(&NOTCH_BG_COLOR, None)
                .expect("Failed to create fill brush");
            let hover_border_brush = rt
                .CreateSolidColorBrush(&COLOR_BORDER_HOVER, None)
                .expect("Failed to create hover border brush");

            let path = renderer
                .create_notch_geometry(&dims)
                .expect("Failed to create notch path geometry");

            rt.FillGeometry(&path, &fill_brush, None);
            rt.DrawGeometry(&path, &hover_border_brush, dims.border_width, None);

            let layout = resolve_layout(&dims);
            if let ResolvedLayout::Collapsed { components, .. } = &layout {
                renderer
                    .render_collapsed_content(&rt, components, &dims, "12:00 PM")
                    .expect("render collapsed content failed");
            }

            rt.EndDraw(None, None).expect("EndDraw failed");

            let pixel_slice =
                std::slice::from_raw_parts(bits_hover as *const u8, (width * height * 4) as usize);

            // Center pixel must remain pure AMOLED #000000
            let center_idx = ((16 * width + 110) * 4) as usize;
            assert_eq!(pixel_slice[center_idx + 3], 255, "Alpha must be 255");
            assert_eq!(pixel_slice[center_idx], 0, "Blue must be 0");
            assert_eq!(pixel_slice[center_idx + 1], 0, "Green must be 0");
            assert_eq!(pixel_slice[center_idx + 2], 0, "Red must be 0");

            // Corner pixel outside shoulder curve must remain transparent
            let corner_idx = ((width + 1) * 4) as usize;
            assert_eq!(
                pixel_slice[corner_idx + 3],
                0,
                "Outside must be transparent"
            );

            // Save rendered hover bitmap
            let bmp_path = r"C:\Users\ujval\.gemini\antigravity-ide\brain\402578f9-d9e5-44d8-9b61-4772077cf14b\notch_hovered_collapsed.bmp";
            let mut file_bytes = Vec::with_capacity(54 + pixel_slice.len());
            file_bytes.extend_from_slice(b"BM");
            let file_size = (54 + pixel_slice.len()) as u32;
            file_bytes.extend_from_slice(&file_size.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&54u32.to_le_bytes());
            file_bytes.extend_from_slice(&40u32.to_le_bytes());
            file_bytes.extend_from_slice(&width.to_le_bytes());
            file_bytes.extend_from_slice(&(-height).to_le_bytes());
            file_bytes.extend_from_slice(&1u16.to_le_bytes());
            file_bytes.extend_from_slice(&32u16.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&(pixel_slice.len() as u32).to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(pixel_slice);
            let _ = std::fs::write(bmp_path, file_bytes);

            SelectObject(mem_dc, old_bmp);
            let _ = DeleteObject(dib_hover.into());

            // 2. Render Hovered Expanded Notch
            let dims_exp = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 96);
            let w_exp = dims_exp.width;
            let h_exp = dims_exp.height;
            let bmi_exp = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w_exp,
                    biHeight: -h_exp,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                },
                bmiColors: [RGBQUAD::default()],
            };

            let mut bits_exp: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib_exp = CreateDIBSection(
                Some(mem_dc),
                &bmi_exp,
                DIB_RGB_COLORS,
                &mut bits_exp,
                None,
                0,
            )
            .expect("Failed to create DIB section for expanded hover test");
            let old_bmp_exp = SelectObject(mem_dc, dib_exp.into());

            let rt_exp = renderer
                .d2d_factory
                .CreateDCRenderTarget(&rt_props)
                .expect("Failed to create DC render target for expanded hover");
            let rc_exp = RECT {
                left: 0,
                top: 0,
                right: w_exp,
                bottom: h_exp,
            };
            rt_exp.BindDC(mem_dc, &rc_exp).expect("BindDC failed");
            rt_exp.BeginDraw();
            rt_exp.Clear(Some(&COLOR_TRANSPARENT));

            let fill_brush_exp = rt_exp
                .CreateSolidColorBrush(&NOTCH_BG_COLOR, None)
                .expect("Failed to create fill brush");
            let hover_border_brush_exp = rt_exp
                .CreateSolidColorBrush(&COLOR_BORDER_HOVER, None)
                .expect("Failed to create hover border brush");

            let path_exp = renderer
                .create_notch_geometry(&dims_exp)
                .expect("Failed to create expanded notch path");
            rt_exp.FillGeometry(&path_exp, &fill_brush_exp, None);
            rt_exp.DrawGeometry(
                &path_exp,
                &hover_border_brush_exp,
                dims_exp.border_width,
                None,
            );

            let layout_exp = resolve_layout(&dims_exp);
            if let ResolvedLayout::Expanded { components, .. } = &layout_exp {
                renderer
                    .render_expanded_content(
                        &rt_exp,
                        components,
                        &dims_exp,
                        "12:00 PM",
                        "Monday, October 6",
                    )
                    .expect("render expanded content failed");
            }

            rt_exp.EndDraw(None, None).expect("EndDraw failed");

            let pixel_slice_exp =
                std::slice::from_raw_parts(bits_exp as *const u8, (w_exp * h_exp * 4) as usize);

            // Save rendered expanded hover bitmap
            let bmp_path_exp = r"C:\Users\ujval\.gemini\antigravity-ide\brain\402578f9-d9e5-44d8-9b61-4772077cf14b\notch_hovered_expanded.bmp";
            let mut file_bytes_exp = Vec::with_capacity(54 + pixel_slice_exp.len());
            file_bytes_exp.extend_from_slice(b"BM");
            let file_size_exp = (54 + pixel_slice_exp.len()) as u32;
            file_bytes_exp.extend_from_slice(&file_size_exp.to_le_bytes());
            file_bytes_exp.extend_from_slice(&0u32.to_le_bytes());
            file_bytes_exp.extend_from_slice(&54u32.to_le_bytes());
            file_bytes_exp.extend_from_slice(&40u32.to_le_bytes());
            file_bytes_exp.extend_from_slice(&w_exp.to_le_bytes());
            file_bytes_exp.extend_from_slice(&(-h_exp).to_le_bytes());
            file_bytes_exp.extend_from_slice(&1u16.to_le_bytes());
            file_bytes_exp.extend_from_slice(&32u16.to_le_bytes());
            file_bytes_exp.extend_from_slice(&0u32.to_le_bytes());
            file_bytes_exp.extend_from_slice(&(pixel_slice_exp.len() as u32).to_le_bytes());
            file_bytes_exp.extend_from_slice(&0u32.to_le_bytes());
            file_bytes_exp.extend_from_slice(&0u32.to_le_bytes());
            file_bytes_exp.extend_from_slice(&0u32.to_le_bytes());
            file_bytes_exp.extend_from_slice(&0u32.to_le_bytes());
            file_bytes_exp.extend_from_slice(pixel_slice_exp);
            let _ = std::fs::write(bmp_path_exp, file_bytes_exp);

            SelectObject(mem_dc, old_bmp_exp);
            let _ = DeleteObject(dib_exp.into());
            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen_dc);
        }
    }

    #[test]
    fn test_animation_intermediate_content_clipping() {
        let renderer = Renderer::new().expect("Failed to initialize Direct2D renderer");

        // 1. Expanding animation frame (progress = 0.65)
        let mut anim_exp =
            crate::config::AnimationState::new(NotchState::Collapsed, NotchState::Expanded);
        anim_exp.set_progress(0.65);
        let dims_exp = anim_exp.current_dimensions(96);
        let w_exp = dims_exp.width;
        let h_exp = dims_exp.height;

        unsafe {
            let screen_dc = GetDC(None);
            assert!(!screen_dc.is_invalid());
            let mem_dc = CreateCompatibleDC(Some(screen_dc));
            assert!(!mem_dc.is_invalid());

            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w_exp,
                    biHeight: -h_exp,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                },
                bmiColors: [RGBQUAD::default()],
            };

            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib = CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
                .expect("Failed to create DIB section");
            let old_bmp = SelectObject(mem_dc, dib.into());

            let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };

            let rt = renderer
                .d2d_factory
                .CreateDCRenderTarget(&rt_props)
                .expect("Failed to create DC render target");

            let rc = RECT {
                left: 0,
                top: 0,
                right: w_exp,
                bottom: h_exp,
            };
            rt.BindDC(mem_dc, &rc).expect("BindDC failed");
            rt.BeginDraw();
            rt.Clear(Some(&COLOR_TRANSPARENT));

            let fill_brush = rt
                .CreateSolidColorBrush(&NOTCH_BG_COLOR, None)
                .expect("fill brush");
            let border_brush = rt
                .CreateSolidColorBrush(&NOTCH_BORDER_COLOR, None)
                .expect("border brush");
            let path = renderer.create_notch_geometry(&dims_exp).expect("geometry");
            rt.FillGeometry(&path, &fill_brush, None);
            rt.DrawGeometry(&path, &border_brush, dims_exp.border_width, None);

            let layout = resolve_layout(&dims_exp);
            match &layout {
                ResolvedLayout::Expanded { components, .. } => {
                    let min_h = (50.0 * dims_exp.scale).round() as i32;
                    if dims_exp.height >= min_h {
                        renderer
                            .render_expanded_content(
                                &rt,
                                components,
                                &dims_exp,
                                "12:00 PM",
                                "Monday, October 6",
                            )
                            .expect("render expanded content");
                    }
                }
                ResolvedLayout::Collapsed { components, .. } => {
                    renderer
                        .render_collapsed_content(&rt, components, &dims_exp, "12:00 PM")
                        .expect("render collapsed content");
                }
            }

            rt.EndDraw(None, None).expect("EndDraw failed");

            let pixel_slice =
                std::slice::from_raw_parts(bits as *const u8, (w_exp * h_exp * 4) as usize);

            // Save expanding animation frame bitmap
            let bmp_path = r"C:\Users\ujval\.gemini\antigravity-ide\brain\402578f9-d9e5-44d8-9b61-4772077cf14b\notch_animating_expanding.bmp";
            let mut file_bytes = Vec::with_capacity(54 + pixel_slice.len());
            file_bytes.extend_from_slice(b"BM");
            let file_size = (54 + pixel_slice.len()) as u32;
            file_bytes.extend_from_slice(&file_size.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&54u32.to_le_bytes());
            file_bytes.extend_from_slice(&40u32.to_le_bytes());
            file_bytes.extend_from_slice(&w_exp.to_le_bytes());
            file_bytes.extend_from_slice(&(-h_exp).to_le_bytes());
            file_bytes.extend_from_slice(&1u16.to_le_bytes());
            file_bytes.extend_from_slice(&32u16.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&(pixel_slice.len() as u32).to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(&0u32.to_le_bytes());
            file_bytes.extend_from_slice(pixel_slice);
            let _ = std::fs::write(bmp_path, file_bytes);

            SelectObject(mem_dc, old_bmp);
            let _ = DeleteObject(dib.into());

            // 2. Collapsing animation frame (progress = 0.60)
            let mut anim_col =
                crate::config::AnimationState::new(NotchState::Expanded, NotchState::Collapsed);
            anim_col.set_progress(0.60);
            let dims_col = anim_col.current_dimensions(96);
            let w_col = dims_col.width;
            let h_col = dims_col.height;

            let bmi_col = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w_col,
                    biHeight: -h_col,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                },
                bmiColors: [RGBQUAD::default()],
            };

            let mut bits_col: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib_col = CreateDIBSection(
                Some(mem_dc),
                &bmi_col,
                DIB_RGB_COLORS,
                &mut bits_col,
                None,
                0,
            )
            .expect("create DIB");
            let old_bmp_col = SelectObject(mem_dc, dib_col.into());

            let rt_col = renderer
                .d2d_factory
                .CreateDCRenderTarget(&rt_props)
                .expect("rt_col");
            let rc_col = RECT {
                left: 0,
                top: 0,
                right: w_col,
                bottom: h_col,
            };
            rt_col.BindDC(mem_dc, &rc_col).expect("BindDC");
            rt_col.BeginDraw();
            rt_col.Clear(Some(&COLOR_TRANSPARENT));

            let fill_col = rt_col
                .CreateSolidColorBrush(&NOTCH_BG_COLOR, None)
                .expect("fill");
            let border_col = rt_col
                .CreateSolidColorBrush(&NOTCH_BORDER_COLOR, None)
                .expect("border");
            let path_col = renderer.create_notch_geometry(&dims_col).expect("path");
            rt_col.FillGeometry(&path_col, &fill_col, None);
            rt_col.DrawGeometry(&path_col, &border_col, dims_col.border_width, None);

            let layout_col = resolve_layout(&dims_col);
            match &layout_col {
                ResolvedLayout::Expanded { components, .. } => {
                    let min_h = (50.0 * dims_col.scale).round() as i32;
                    if dims_col.height >= min_h {
                        renderer
                            .render_expanded_content(
                                &rt_col,
                                components,
                                &dims_col,
                                "12:00 PM",
                                "Monday, October 6",
                            )
                            .expect("render expanded");
                    } else {
                        let collapsed_layout = resolve_collapsed_layout(&dims_col);
                        renderer
                            .render_collapsed_content(
                                &rt_col,
                                &collapsed_layout,
                                &dims_col,
                                "12:00 PM",
                            )
                            .expect("render collapsed");
                    }
                }
                ResolvedLayout::Collapsed { components, .. } => {
                    renderer
                        .render_collapsed_content(&rt_col, components, &dims_col, "12:00 PM")
                        .expect("render collapsed");
                }
            }

            rt_col.EndDraw(None, None).expect("EndDraw");

            let pixel_slice_col =
                std::slice::from_raw_parts(bits_col as *const u8, (w_col * h_col * 4) as usize);

            // Save collapsing animation frame bitmap
            let bmp_path_col = r"C:\Users\ujval\.gemini\antigravity-ide\brain\402578f9-d9e5-44d8-9b61-4772077cf14b\notch_animating_collapsing.bmp";
            let mut file_bytes_col = Vec::with_capacity(54 + pixel_slice_col.len());
            file_bytes_col.extend_from_slice(b"BM");
            let file_size_col = (54 + pixel_slice_col.len()) as u32;
            file_bytes_col.extend_from_slice(&file_size_col.to_le_bytes());
            file_bytes_col.extend_from_slice(&0u32.to_le_bytes());
            file_bytes_col.extend_from_slice(&54u32.to_le_bytes());
            file_bytes_col.extend_from_slice(&40u32.to_le_bytes());
            file_bytes_col.extend_from_slice(&w_col.to_le_bytes());
            file_bytes_col.extend_from_slice(&(-h_col).to_le_bytes());
            file_bytes_col.extend_from_slice(&1u16.to_le_bytes());
            file_bytes_col.extend_from_slice(&32u16.to_le_bytes());
            file_bytes_col.extend_from_slice(&0u32.to_le_bytes());
            file_bytes_col.extend_from_slice(&(pixel_slice_col.len() as u32).to_le_bytes());
            file_bytes_col.extend_from_slice(&0u32.to_le_bytes());
            file_bytes_col.extend_from_slice(&0u32.to_le_bytes());
            file_bytes_col.extend_from_slice(&0u32.to_le_bytes());
            file_bytes_col.extend_from_slice(&0u32.to_le_bytes());
            file_bytes_col.extend_from_slice(pixel_slice_col);
            let _ = std::fs::write(bmp_path_col, file_bytes_col);

            SelectObject(mem_dc, old_bmp_col);
            let _ = DeleteObject(dib_col.into());
            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen_dc);
        }
    }

    #[test]
    fn test_renderer_clock_12h_and_24h_display() {
        let renderer = Renderer::new().expect("Failed to initialize Direct2D renderer");
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, 96);
        let width = dims.width;
        let height = dims.height;

        unsafe {
            let screen_dc = GetDC(None);
            let mem_dc = CreateCompatibleDC(Some(screen_dc));

            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                },
                bmiColors: [RGBQUAD::default()],
            };

            let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };

            let rt = renderer
                .d2d_factory
                .CreateDCRenderTarget(&rt_props)
                .expect("Failed to create DC render target");

            let rc = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };

            // Test 1: 12-hour format "5:42 PM"
            {
                let mut bits_12h: *mut std::ffi::c_void = std::ptr::null_mut();
                let dib_12h =
                    CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits_12h, None, 0)
                        .expect("DIB 12h");
                let old_bmp = SelectObject(mem_dc, dib_12h.into());
                rt.BindDC(mem_dc, &rc).expect("BindDC 12h");

                rt.BeginDraw();
                rt.Clear(Some(&COLOR_TRANSPARENT));
                let fill_brush = rt
                    .CreateSolidColorBrush(&NOTCH_BG_COLOR, None)
                    .expect("fill");
                let path = renderer.create_notch_geometry(&dims).expect("geometry");
                rt.FillGeometry(&path, &fill_brush, None);

                let layout = resolve_layout(&dims);
                if let ResolvedLayout::Collapsed { components, .. } = &layout {
                    renderer
                        .render_collapsed_content(&rt, components, &dims, "5:42 PM")
                        .expect("render 12h");
                }
                rt.EndDraw(None, None).expect("EndDraw 12h");

                let pixel_slice = std::slice::from_raw_parts(
                    bits_12h as *const u8,
                    (width * height * 4) as usize,
                );

                // Center time region has rendered text pixels
                let mut text_pixel_count = 0;
                for y in 8..24 {
                    for x in 70..150 {
                        let idx = ((y * width + x) * 4) as usize;
                        let r = pixel_slice[idx + 2];
                        if r > 20 {
                            text_pixel_count += 1;
                        }
                    }
                }
                assert!(text_pixel_count > 20, "12h text must be rendered");

                SelectObject(mem_dc, old_bmp);
                let _ = DeleteObject(dib_12h.into());
            }

            // Test 2: 24-hour format "17:42"
            {
                let mut bits_24h: *mut std::ffi::c_void = std::ptr::null_mut();
                let dib_24h =
                    CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits_24h, None, 0)
                        .expect("DIB 24h");
                let old_bmp = SelectObject(mem_dc, dib_24h.into());
                rt.BindDC(mem_dc, &rc).expect("BindDC 24h");

                rt.BeginDraw();
                rt.Clear(Some(&COLOR_TRANSPARENT));
                let fill_brush = rt
                    .CreateSolidColorBrush(&NOTCH_BG_COLOR, None)
                    .expect("fill");
                let path = renderer.create_notch_geometry(&dims).expect("geometry");
                rt.FillGeometry(&path, &fill_brush, None);

                let layout = resolve_layout(&dims);
                if let ResolvedLayout::Collapsed { components, .. } = &layout {
                    renderer
                        .render_collapsed_content(&rt, components, &dims, "17:42")
                        .expect("render 24h");
                }
                rt.EndDraw(None, None).expect("EndDraw 24h");

                let pixel_slice = std::slice::from_raw_parts(
                    bits_24h as *const u8,
                    (width * height * 4) as usize,
                );

                // Center time region has rendered text pixels
                let mut text_pixel_count = 0;
                for y in 8..24 {
                    for x in 70..150 {
                        let idx = ((y * width + x) * 4) as usize;
                        let r = pixel_slice[idx + 2];
                        if r > 20 {
                            text_pixel_count += 1;
                        }
                    }
                }
                assert!(text_pixel_count > 20, "24h text must be rendered");

                SelectObject(mem_dc, old_bmp);
                let _ = DeleteObject(dib_24h.into());
            }

            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen_dc);
        }
    }

    #[test]
    fn test_expanded_renderer_12h_24h_and_date_display() {
        let renderer = Renderer::new().expect("Failed to initialize Direct2D renderer");
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 96);
        let width = dims.width;
        let height = dims.height;

        unsafe {
            let screen_dc = GetDC(None);
            let mem_dc = CreateCompatibleDC(Some(screen_dc));
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                },
                bmiColors: [RGBQUAD::default()],
            };

            let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 0.0,
                dpiY: 0.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };

            let rt = renderer
                .d2d_factory
                .CreateDCRenderTarget(&rt_props)
                .expect("Failed to create DC render target");
            let rc = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };

            // Test 1: 12-hour format "12:47 AM" with date "Monday, October 6"
            {
                let mut bits_12h: *mut std::ffi::c_void = std::ptr::null_mut();
                let dib_12h =
                    CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits_12h, None, 0)
                        .expect("DIB 12h");
                let old_bmp = SelectObject(mem_dc, dib_12h.into());
                rt.BindDC(mem_dc, &rc).expect("BindDC 12h");

                rt.BeginDraw();
                rt.Clear(Some(&COLOR_TRANSPARENT));

                let fill_brush = rt
                    .CreateSolidColorBrush(&NOTCH_BG_COLOR, None)
                    .expect("fill");
                let path = renderer.create_notch_geometry(&dims).expect("geometry");
                rt.FillGeometry(&path, &fill_brush, None);

                let layout = resolve_layout(&dims);
                if let ResolvedLayout::Expanded { components, .. } = &layout {
                    renderer
                        .render_expanded_content(
                            &rt,
                            components,
                            &dims,
                            "12:47 AM",
                            "Monday, October 6",
                        )
                        .expect("render 12h + date");
                }
                rt.EndDraw(None, None).expect("EndDraw 12h");

                let pixel_slice = std::slice::from_raw_parts(
                    bits_12h as *const u8,
                    (width * height * 4) as usize,
                );

                // Time region (y: 35..65, x: 200..400) has rendered primary text pixels
                let mut time_pixel_count = 0;
                for y in 35..65 {
                    for x in 200..400 {
                        let idx = ((y * width + x) * 4) as usize;
                        let r = pixel_slice[idx + 2];
                        if r > 20 {
                            time_pixel_count += 1;
                        }
                    }
                }
                assert!(
                    time_pixel_count > 30,
                    "Expanded 12h time text must be rendered"
                );

                // Date region (y: 70..95, x: 180..420) has rendered secondary text pixels
                let mut date_pixel_count = 0;
                for y in 70..95 {
                    for x in 180..420 {
                        let idx = ((y * width + x) * 4) as usize;
                        let r = pixel_slice[idx + 2];
                        if r > 20 {
                            date_pixel_count += 1;
                        }
                    }
                }
                assert!(date_pixel_count > 30, "Expanded date text must be rendered");

                SelectObject(mem_dc, old_bmp);
                let _ = DeleteObject(dib_12h.into());
            }

            // Test 2: 24-hour format "17:42" with date "Monday, October 6"
            {
                let mut bits_24h: *mut std::ffi::c_void = std::ptr::null_mut();
                let dib_24h =
                    CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits_24h, None, 0)
                        .expect("DIB 24h");
                let old_bmp = SelectObject(mem_dc, dib_24h.into());
                rt.BindDC(mem_dc, &rc).expect("BindDC 24h");

                rt.BeginDraw();
                rt.Clear(Some(&COLOR_TRANSPARENT));

                let fill_brush = rt
                    .CreateSolidColorBrush(&NOTCH_BG_COLOR, None)
                    .expect("fill");
                let path = renderer.create_notch_geometry(&dims).expect("geometry");
                rt.FillGeometry(&path, &fill_brush, None);

                let layout = resolve_layout(&dims);
                if let ResolvedLayout::Expanded { components, .. } = &layout {
                    renderer
                        .render_expanded_content(
                            &rt,
                            components,
                            &dims,
                            "17:42",
                            "Monday, October 6",
                        )
                        .expect("render 24h + date");
                }
                rt.EndDraw(None, None).expect("EndDraw 24h");

                let pixel_slice = std::slice::from_raw_parts(
                    bits_24h as *const u8,
                    (width * height * 4) as usize,
                );

                // Time region has rendered 24h text pixels
                let mut time_pixel_count = 0;
                for y in 35..65 {
                    for x in 200..400 {
                        let idx = ((y * width + x) * 4) as usize;
                        let r = pixel_slice[idx + 2];
                        if r > 20 {
                            time_pixel_count += 1;
                        }
                    }
                }
                assert!(
                    time_pixel_count > 25,
                    "Expanded 24h time text must be rendered"
                );

                SelectObject(mem_dc, old_bmp);
                let _ = DeleteObject(dib_24h.into());
            }

            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen_dc);
        }
    }

    #[test]
    fn test_collapsed_state_regression_unmodified() {
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, 96);
        assert_eq!(dims.width, 220);
        assert_eq!(dims.height, 32);
        assert_eq!(dims.curvature.bottom_radius, 14.0);
        assert_eq!(dims.curvature.top_transition_radius, 6.0);
        assert_eq!(dims.curvature.top_transition_height, 6.0);

        let layout = resolve_layout(&dims);
        match layout {
            ResolvedLayout::Collapsed { components, bounds } => {
                assert_eq!(bounds.width(), 220.0);
                assert_eq!(bounds.height(), 32.0);
                assert!(components.content_bounds.width() > 0.0);
                assert!(components.clock_bounds.width() > 0.0);
            }
            _ => panic!("Expected ResolvedLayout::Collapsed"),
        }
    }

    use crate::layout::MediaShape;

    const LONG_TITLE: &str = "An Extremely Long Track Title That Keeps Going Well Beyond Any \
         Reasonable Width (Extended Deluxe Remastered Live Version) feat. Many Artists";
    const ALL_DPIS: [u32; 7] = [96, 120, 137, 144, 168, 192, 288];

    /// Measures `text` laid out with a media line format in `rect`.
    fn measure(
        renderer: &Renderer,
        format: &IDWriteTextFormat,
        text: &str,
        rect: RectF,
    ) -> (u32, f32) {
        use windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_METRICS;
        let text: Vec<u16> = text.encode_utf16().collect();
        let layout = unsafe {
            renderer
                .dwrite_factory
                .CreateTextLayout(&text, format, rect.width(), rect.height())
                .unwrap()
        };
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut metrics).unwrap() };
        (metrics.lineCount, metrics.width)
    }

    #[test]
    fn test_media_lines_single_line_and_trimmed_all_dpis() {
        let renderer = Renderer::new().expect("renderer");
        for dpi in ALL_DPIS {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            let m = resolve_media_layout(&dims, MediaShape::FULL).unwrap();
            let f = renderer.media_formats(&dims).unwrap();
            for (name, format, rect) in [
                ("title", &f.title, m.title_bounds),
                ("artist", &f.artist, m.artist_bounds),
                ("source", &f.source, m.source_bounds),
            ] {
                let (lines, width) = measure(&renderer, format, LONG_TITLE, rect);
                assert_eq!(lines, 1, "{name} wrapped at {dpi} DPI");
                assert!(
                    width <= rect.width() + 0.5,
                    "{name} {width} > {} at {dpi}",
                    rect.width()
                );
            }
        }
    }

    #[test]
    fn test_media_formats_cached_per_dpi() {
        let renderer = Renderer::new().expect("renderer");
        let d96 = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 96);
        let a = renderer.media_formats(&d96).unwrap().title.clone();
        let b = renderer.media_formats(&d96).unwrap().title.clone();
        assert_eq!(a, b, "same DPI reuses the cached format");
        let d144 = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 144);
        let c = renderer.media_formats(&d144).unwrap().title.clone();
        assert_ne!(a, c, "DPI change rebuilds formats");
    }

    #[test]
    fn test_cover_source_rect_preserves_aspect() {
        let r = cover_source_rect(300, 300, 72.0, 72.0);
        assert_eq!(r, RectF::new(0.0, 0.0, 300.0, 300.0));
        let r = cover_source_rect(400, 300, 72.0, 72.0);
        assert_eq!(
            r,
            RectF::new(50.0, 0.0, 350.0, 300.0),
            "wide: center-crop width"
        );
        let r = cover_source_rect(150, 300, 72.0, 72.0);
        assert_eq!(
            r,
            RectF::new(0.0, 75.0, 150.0, 225.0),
            "tall: center-crop height"
        );
        for (w, h) in [(256, 144), (100, 256), (1, 1), (256, 256)] {
            let r = cover_source_rect(w, h, 90.0, 90.0);
            assert!(
                (r.width() - r.height()).abs() < 0.01,
                "square crop for {w}x{h}"
            );
            assert!(r.left >= 0.0 && r.top >= 0.0 && r.right <= w as f32 && r.bottom <= h as f32);
        }
        assert_eq!(
            cover_source_rect(0, 10, 72.0, 72.0),
            RectF::new(0.0, 0.0, 0.0, 10.0)
        );
    }

    fn sample_artwork(w: u32, h: u32) -> Artwork {
        // Opaque orange BGRA (premultiplied, alpha 255)
        let px: Vec<u8> = (0..w * h).flat_map(|_| [0x20, 0x80, 0xF0, 0xFF]).collect();
        Artwork::new(w, h, px).unwrap()
    }

    fn content(title: &str, art: Option<Artwork>) -> MediaContent {
        MediaContent {
            title: title.to_string(),
            subtitle: format!("{LONG_TITLE} \u{2014} {LONG_TITLE}"),
            icon: PlayPauseIcon::Pause,
            artwork: art,
            source: crate::media::SourceApp {
                app_id: "X".into(),
                name: crate::media::AppName::Available(LONG_TITLE.into()),
                icon: None,
            },
            ..MediaContent::test_default()
        }
    }

    /// Renders the notch with media content through the real buffers and
    /// returns (pixels, stride).
    fn render_media_frame(
        renderer: &Renderer,
        dims: &NotchDimensions,
        content: &MediaContent,
    ) -> (Vec<u32>, usize) {
        renderer.set_media(Some(content));
        // Static frame: settle any new-track fade (fading has its own test)
        renderer.feedback().finish_track_fade();
        render_frame_as_is(renderer, dims)
    }

    /// Renders the renderer's current media content with its current feedback state.
    fn render_frame_as_is(renderer: &Renderer, dims: &NotchDimensions) -> (Vec<u32>, usize) {
        renderer.ensure_buffer(dims.width, dims.height).unwrap();
        let rt_ref = renderer.cached_rt.borrow();
        let rt = rt_ref.as_ref().unwrap();
        let clock = ClockDateState::from_pure_components(2026, 10, 6, 2, 0, 47, 0, 0, false);
        unsafe {
            let bind = RECT {
                left: 0,
                top: 0,
                right: dims.width,
                bottom: dims.height,
            };
            rt.BindDC(renderer.cached_mem_dc.get(), &bind).unwrap();
            rt.BeginDraw();
            rt.Clear(Some(&COLOR_TRANSPARENT));
            let fill = rt.CreateSolidColorBrush(&NOTCH_BG_COLOR, None).unwrap();
            let path = renderer.create_notch_geometry(dims).unwrap();
            rt.FillGeometry(&path, &fill, None);
            let ResolvedLayout::Expanded { components, .. } = resolve_layout(dims) else {
                panic!("expanded");
            };
            let media = renderer.media.borrow();
            let media = media.as_ref().unwrap();
            renderer
                .render_media_content(
                    rt,
                    &components,
                    &resolve_media_layout(dims, media.content.shape()).unwrap(),
                    dims,
                    &clock,
                    media,
                )
                .unwrap();
            rt.EndDraw(None, None).unwrap();
        }
        let stride = renderer.cached_capacity_w.get() as usize;
        let bits = renderer.cached_bits.get() as *const u32;
        let pixels = unsafe { std::slice::from_raw_parts(bits, stride * dims.height as usize) };
        (pixels.to_vec(), stride)
    }

    fn lit_in(px: &[u32], stride: usize, r: RectF, threshold: u32) -> bool {
        (r.top.ceil() as usize..r.bottom.floor() as usize).any(|y| {
            (r.left.ceil() as usize..r.right.floor() as usize).any(|x| {
                (px[y * stride + x] & 0xFF) > threshold
                    || ((px[y * stride + x] >> 16) & 0xFF) > threshold
            })
        })
    }

    #[test]
    fn test_media_rendering_contained_with_and_without_artwork_all_dpis() {
        let renderer = Renderer::new().expect("renderer");
        for dpi in ALL_DPIS {
            for art in [Some(sample_artwork(256, 144)), None] {
                let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
                let c = content(LONG_TITLE, art.clone());
                let m = resolve_media_layout(&dims, c.shape()).unwrap();
                let ResolvedLayout::Expanded { components, .. } = resolve_layout(&dims) else {
                    panic!("expanded");
                };
                // Content area, grown left to the artwork at the notch edge
                let cb = components.content_bounds;
                let bounds = RectF::new(
                    m.artwork_bounds.map_or(cb.left, |a| a.left),
                    cb.top,
                    cb.right,
                    m.artwork_bounds
                        .map_or(cb.bottom, |a| a.bottom.max(cb.bottom)),
                );
                let (px, stride) = render_media_frame(&renderer, &dims, &c);
                // Nothing drawn outside those bounds
                for y in 0..dims.height as usize {
                    for x in 0..dims.width as usize {
                        let p = px[y * stride + x];
                        if (p & 0xFF) > 40 || ((p >> 16) & 0xFF) > 40 {
                            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                            assert!(
                                fx >= bounds.left - 1.0
                                    && fx <= bounds.right + 1.0
                                    && fy >= bounds.top - 1.0
                                    && fy <= bounds.bottom + 1.0,
                                "spill at ({x},{y}) at {dpi} DPI"
                            );
                        }
                    }
                }
                // Long text never reaches the clock column (the title row ends
                // earlier, at the visualizer slot)
                let gap = RectF::new(
                    m.artist_bounds.right + 1.0,
                    m.title_bounds.top,
                    m.time_bounds.left - 1.0,
                    m.source_bounds.bottom,
                );
                assert!(!lit_in(&px, stride, gap, 40), "text overflow at {dpi} DPI");
                for (name, r) in [
                    ("title", m.title_bounds),
                    ("artist", m.artist_bounds),
                    ("source", m.source_bounds),
                    ("time", m.time_bounds),
                    ("date", m.date_bounds),
                ] {
                    assert!(lit_in(&px, stride, r, 40), "{name} not drawn at {dpi} DPI");
                }
                for k in MediaControl::ALL {
                    assert!(
                        lit_in(&px, stride, m.control_visual_bounds(k), 40),
                        "{k:?} icon missing at {dpi}"
                    );
                }
                if let Some(a) = m.artwork_bounds {
                    // Artwork fills its slot (center) and its corners are rounded off to black
                    let cx = ((a.left + a.right) / 2.0) as usize;
                    let cy = ((a.top + a.bottom) / 2.0) as usize;
                    assert_eq!(
                        px[cy * stride + cx] & 0x00FF_FFFF,
                        0x00F0_8020,
                        "artwork center at {dpi}"
                    );
                    let corner = px[(a.top as usize + 1) * stride + a.left as usize + 1];
                    assert!(
                        corner & 0xFF < 0x20 && (corner >> 16) & 0xFF < 0x20,
                        "corner not rounded at {dpi}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_hover_and_press_brighten_icon_without_backdrop() {
        let renderer = Renderer::new().expect("renderer");
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 96);
        let c = content("Song", None);
        let m = resolve_media_layout(&dims, c.shape()).unwrap();
        // A point inside the old round backdrop but away from the icon
        let corner = |px: &[u32], stride: usize, k: MediaControl| {
            let v = m.control_visual_bounds(k);
            px[(v.top + 2.0) as usize * stride + ((v.left + v.right) / 2.0) as usize] & 0xFF
        };
        // Brightest icon pixel of a control
        let icon = |px: &[u32], stride: usize, k: MediaControl| {
            let v = m.control_visual_bounds(k);
            (v.top as usize..v.bottom as usize)
                .flat_map(|y| (v.left as usize..v.right as usize).map(move |x| (x, y)))
                .map(|(x, y)| px[y * stride + x] & 0xFF)
                .max()
                .unwrap()
        };
        let (idle, stride) = render_media_frame(&renderer, &dims, &c);
        renderer.feedback().set_hovered(Some(MediaControl::Next));
        renderer.feedback().step(1000.0);
        let (hover, _) = render_media_frame(&renderer, &dims, &c);
        renderer.feedback().press(MediaControl::Next);
        renderer.feedback().step(1000.0);
        let (pressed, _) = render_media_frame(&renderer, &dims, &c);
        for (state, px) in [("idle", &idle), ("hover", &hover), ("pressed", &pressed)] {
            assert_eq!(
                corner(px, stride, MediaControl::Next),
                0,
                "no backdrop when {state}"
            );
        }
        assert!(
            icon(&hover, stride, MediaControl::Next) > icon(&idle, stride, MediaControl::Next),
            "hover brightens the icon"
        );
        assert_eq!(
            icon(&pressed, stride, MediaControl::Previous),
            icon(&idle, stride, MediaControl::Previous),
            "other controls unaffected"
        );
    }

    #[test]
    fn test_set_media_change_detection_and_bitmap_release() {
        let renderer = Renderer::new().expect("renderer");
        let base = MediaContent {
            title: "Song".into(),
            subtitle: "Artist".into(),
            icon: PlayPauseIcon::Play,
            ..MediaContent::test_default()
        };
        assert!(renderer.set_media(Some(&base)));
        assert!(
            !renderer.set_media(Some(&base)),
            "identical content is not a change"
        );
        let paused = MediaContent {
            icon: PlayPauseIcon::Pause,
            ..base.clone()
        };
        assert!(renderer.set_media(Some(&paused)), "icon change is visible");
        let with_art = MediaContent {
            artwork: Some(sample_artwork(4, 4)),
            ..paused.clone()
        };
        assert!(
            renderer.set_media(Some(&with_art)),
            "artwork change is visible"
        );
        let hidden_change = MediaContent {
            album: Some("Album".into()),
            ..with_art.clone()
        };
        assert!(
            !renderer.set_media(Some(&hidden_change)),
            "undrawn field is not a change"
        );

        // Artwork bitmap is created once and released when the artwork goes away
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 96);
        render_media_frame(&renderer, &dims, &with_art);
        assert!(renderer.art_bitmap.borrow().is_some());
        let first = renderer.art_bitmap.borrow().as_ref().unwrap().1.clone();
        render_media_frame(&renderer, &dims, &with_art);
        assert_eq!(
            renderer.art_bitmap.borrow().as_ref().unwrap().1,
            first,
            "bitmap reused across frames"
        );
        assert!(renderer.set_media(Some(&paused)));
        assert!(
            renderer.art_bitmap.borrow().is_none(),
            "bitmap released with artwork"
        );

        renderer
            .feedback()
            .set_hovered(Some(MediaControl::PlayPause));
        assert!(renderer.set_media(None));
        assert!(
            !renderer.feedback().is_animating(),
            "feedback cleared with media"
        );
    }

    // ---- Phase 3.9 polish ------------------------------------------------------

    fn shapes_bbox(shapes: &[IconShape]) -> RectF {
        let mut b = RectF::new(f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for s in shapes {
            let pts: Vec<(f32, f32)> = match s {
                IconShape::Bar(r) => vec![(r.left, r.top), (r.right, r.bottom)],
                IconShape::Triangle(p) => p.to_vec(),
            };
            for (x, y) in pts {
                b = RectF::new(b.left.min(x), b.top.min(y), b.right.max(x), b.bottom.max(y));
            }
        }
        b
    }

    #[test]
    fn test_icon_shapes_crisp_balanced_and_mirrored() {
        for dpi in ALL_DPIS {
            let s = dpi as f32 / 96.0;
            let (cx, cy) = (200.5, 87.0);
            for (control, icon, base) in [
                (
                    MediaControl::Previous,
                    PlayPauseIcon::Play,
                    BASE_MEDIA_ICON_SIZE,
                ),
                (
                    MediaControl::Next,
                    PlayPauseIcon::Play,
                    BASE_MEDIA_ICON_SIZE,
                ),
                (
                    MediaControl::PlayPause,
                    PlayPauseIcon::Play,
                    BASE_MEDIA_PLAY_ICON_SIZE,
                ),
                (
                    MediaControl::PlayPause,
                    PlayPauseIcon::Pause,
                    BASE_MEDIA_PLAY_ICON_SIZE,
                ),
            ] {
                let u = (base * s).round();
                let shapes = icon_shapes(control, icon, cx, cy, u, true);
                for shape in &shapes {
                    if let IconShape::Bar(r) = shape {
                        // Whole-pixel edges and at least 2 px wide: crisp at every DPI
                        for e in [r.left, r.right, r.top, r.bottom] {
                            assert_eq!(e, e.round(), "{control:?} bar edge off-grid at {dpi}");
                        }
                        assert!(r.width() >= 2.0, "{control:?} bar too thin at {dpi}");
                    }
                }
                let b = shapes_bbox(&shapes);
                // Skip glyphs are two triangles wide (Apple-style), the rest fit u
                let max_w = if control == MediaControl::PlayPause {
                    u
                } else {
                    (u * SKIP_GLYPH_HALF_WIDTH).round() * 2.0
                };
                assert!(
                    b.width() <= max_w + 1.0 && b.height() <= u + 1.0,
                    "{control:?} too big"
                );
                assert!(
                    (b.top + b.bottom) / 2.0 - cy <= 1.0,
                    "{control:?} not vertically centered"
                );
                if icon == PlayPauseIcon::Pause || control != MediaControl::PlayPause {
                    assert!(
                        ((b.left + b.right) / 2.0 - cx).abs() <= 1.0,
                        "{control:?} off-center"
                    );
                }
            }
            // Previous and Next are exact mirrors around the axis
            let u = (BASE_MEDIA_ICON_SIZE * s).round();
            let p = shapes_bbox(&icon_shapes(
                MediaControl::Previous,
                PlayPauseIcon::Play,
                cx,
                cy,
                u,
                true,
            ));
            let n = shapes_bbox(&icon_shapes(
                MediaControl::Next,
                PlayPauseIcon::Play,
                cx,
                cy,
                u,
                true,
            ));
            assert_eq!(
                (p.width(), p.height()),
                (n.width(), n.height()),
                "skip glyphs differ at {dpi}"
            );
            // Play: centroid right of the box center's left edge, visual mass on the axis
            let u = (BASE_MEDIA_PLAY_ICON_SIZE * s).round();
            if let [IconShape::Triangle(t)] = icon_shapes(
                MediaControl::PlayPause,
                PlayPauseIcon::Play,
                cx,
                cy,
                u,
                false,
            )[..]
            {
                let centroid = (t[0].0 + t[1].0 + t[2].0) / 3.0;
                let box_center = (t[0].0 + t[1].0) / 2.0;
                assert!(
                    ((centroid + box_center) / 2.0 - cx).abs() < 0.01,
                    "play not optical at {dpi}"
                );
            } else {
                panic!("play is one triangle");
            }
        }
    }

    #[test]
    fn test_icons_quiet_at_rest_bright_on_hover() {
        let renderer = Renderer::new().expect("renderer");
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 96);
        let c = content("Song", None);
        let m = resolve_media_layout(&dims, c.shape()).unwrap();
        let v = m.control_visual_bounds(MediaControl::PlayPause);
        let peak = |px: &[u32], stride: usize| {
            let mut best = 0;
            for y in v.top as usize..v.bottom as usize {
                for x in v.left as usize..v.right as usize {
                    best = best.max(px[y * stride + x] & 0xFF);
                }
            }
            best
        };
        let (rest, stride) = render_media_frame(&renderer, &dims, &c);
        renderer
            .feedback()
            .set_hovered(Some(MediaControl::PlayPause));
        renderer.feedback().step(1000.0);
        let (hover, _) = render_media_frame(&renderer, &dims, &c);
        assert!(peak(&rest, stride) < 0xF0, "icon dimmed at rest");
        assert!(
            peak(&hover, stride) > peak(&rest, stride),
            "icon brightens on hover"
        );
    }

    #[test]
    fn test_track_change_fades_track_content_only() {
        let renderer = Renderer::new().expect("renderer");
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 96);
        let a = content("Track A", Some(sample_artwork(8, 8)));
        let mut b = content("Track B", Some(sample_artwork(4, 4)));
        b.icon = PlayPauseIcon::Pause;
        let m = resolve_media_layout(&dims, MediaShape::FULL).unwrap();
        let art = m.artwork_bounds.unwrap();
        let art_px = |px: &[u32], stride: usize| {
            px[((art.top + art.bottom) / 2.0) as usize * stride
                + ((art.left + art.right) / 2.0) as usize]
        };
        let (settled, stride) = render_media_frame(&renderer, &dims, &a);

        // New track: fade starts; render without settling it
        assert!(renderer.set_media(Some(&b)));
        assert!(
            renderer.feedback().is_animating(),
            "track change starts the fade"
        );
        let frame = |r: &Renderer| render_frame_as_is(r, &dims);
        let (fading, _) = frame(&renderer);
        assert!(
            (art_px(&fading, stride) & 0xFF) < (art_px(&settled, stride) & 0xFF),
            "artwork starts dimmed"
        );
        // Clock is environmental: identical while the track fades
        let time = m.time_bounds;
        let same_clock = (time.top as usize..time.bottom as usize).all(|y| {
            (time.left as usize..time.right as usize)
                .all(|x| fading[y * stride + x] == settled[y * stride + x])
        });
        assert!(same_clock, "clock must not fade");
        renderer.feedback().step(1000.0);
        assert!(
            !renderer.feedback().is_animating(),
            "fade settles and stops the timer"
        );
        let (done, _) = frame(&renderer);
        assert_eq!(
            art_px(&done, stride),
            art_px(&settled, stride),
            "full opacity after fade"
        );

        // A playback-only change stays crisp (no fade)
        let mut paused = b.clone();
        paused.icon = PlayPauseIcon::Play;
        assert!(renderer.set_media(Some(&paused)));
        assert!(
            !renderer.feedback().is_animating(),
            "icon change does not fade"
        );
        // Appearing from / disappearing to no media never fades
        assert!(renderer.set_media(None));
        assert!(renderer.set_media(Some(&a)));
        assert!(!renderer.feedback().is_animating());
    }

    #[test]
    fn test_render_target_recreation_drops_and_rebuilds_device_bitmap() {
        let renderer = Renderer::new().expect("renderer");
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 96);
        let c = content("Song", Some(sample_artwork(8, 8)));
        render_media_frame(&renderer, &dims, &c);
        assert!(renderer.art_bitmap.borrow().is_some());
        // Render target / surfaces released (resize past capacity, device loss path)
        renderer.cleanup_cached_resources();
        assert!(
            renderer.art_bitmap.borrow().is_none(),
            "device bitmap tied to old target"
        );
        assert!(renderer.cached_rt.borrow().is_none());
        // A larger DPI forces a new target; artwork is recreated from cached pixels
        let big = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 288);
        render_media_frame(&renderer, &big, &c);
        assert!(
            renderer.art_bitmap.borrow().is_some(),
            "recreated lazily on next frame"
        );
        assert!(renderer.cached_capacity_w.get() >= big.width);
        // DPI change rebuilds text formats for the new DPI only
        assert_eq!(renderer.media_formats.borrow().as_ref().unwrap().0, 288);
    }

    /// Stress (manual): thousands of media frames through every unsafe drawing
    /// path (geometry sinks, artwork bitmaps, fades, hover/press, DPI changes).
    /// `cargo test stress_media_render -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn stress_media_render_frames() {
        let renderer = Renderer::new().expect("renderer");
        let arts = [
            Some(sample_artwork(256, 256)),
            Some(sample_artwork(150, 112)),
            None,
        ];
        for i in 0..3000usize {
            let dpi = [96, 120, 144, 192][i / 750];
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            let mut c = content(
                if i % 2 == 0 { "A" } else { LONG_TITLE },
                arts[i % 3].clone(),
            );
            c.icon = if i % 5 == 0 {
                PlayPauseIcon::Play
            } else {
                PlayPauseIcon::Pause
            };
            renderer.set_media(Some(&c));
            renderer
                .feedback()
                .set_hovered(Some(MediaControl::ALL[i % 3]));
            if i % 7 == 0 {
                renderer.feedback().press(MediaControl::ALL[(i + 1) % 3]);
            } else {
                renderer.feedback().release();
            }
            renderer.feedback().step(8.0);
            render_frame_as_is(&renderer, &dims);
            if i % 300 == 0 {
                renderer.set_media(None);
            }
        }
        println!("3000 frames rendered");
    }

    #[test]
    fn test_drop_shadow_shape() {
        // Synthetic frame: opaque notch body x 10..90, y 0..30 in a 100x40 window,
        // with flared "shoulders" (x 4..96) along the top 5 rows
        let (w, h) = (100usize, 40usize);
        let mut px = vec![0u8; w * h * 4];
        for y in 0..30 {
            let (l, r) = if y < 5 { (4, 96) } else { (10, 90) };
            for x in l..r {
                px[(y * w + x) * 4 + 3] = 255;
            }
        }
        let (mut a, mut b) = (Vec::new(), Vec::new());
        apply_ambient_shadow(px.as_mut_ptr(), w, h, w, 1.0, 0.28, &mut a, &mut b);
        let alpha = |x: usize, y: usize| px[(y * w + x) * 4 + 3];
        assert!(alpha(50, 31) > 60, "dark directly beneath the notch");
        assert!(alpha(50, 31) > alpha(50, 36), "fades smoothly downward");
        for y in 0..5 {
            for x in 0..4 {
                assert_eq!(alpha(x, y), 0, "no shadow beside the shoulders");
            }
        }
        assert_eq!(alpha(7, 5), 0, "starts right where the shoulder ends");
        // Fades in gradually below the shoulders: no visible starting edge
        assert!(
            alpha(8, 7) < alpha(8, 12) && alpha(8, 12) < alpha(8, 20),
            "side shadow eases in"
        );
        assert!(alpha(8, 7) < 12, "only a whisper right below the shoulder");
        assert!(
            alpha(8, 20) > 20,
            "visible along the sides, not just beneath"
        );
        for x in 0..w {
            assert_eq!(alpha(x, h - 1), 0, "reaches zero before the bottom edge");
        }
        for y in 0..h {
            assert_eq!(alpha(0, y), 0, "reaches zero before the left edge");
            assert_eq!(alpha(w - 1, y), 0, "reaches zero before the right edge");
        }
        assert_eq!(alpha(50, 10), 255, "opaque body untouched");
    }

    #[test]
    fn test_media_settings_hide_only_the_home_badge_and_music_bars() {
        use crate::media::PlaybackState as P;
        let renderer = Renderer::new().expect("renderer");
        let with = |f: fn(&mut NottSettings)| {
            let mut s = NottSettings::default();
            f(&mut s);
            renderer.set_settings(false, s);
        };
        for dpi in [96u32, 120] {
            // Home: the badge goes, the artwork and the title stay
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            let green: Vec<u8> = (0..16 * 16)
                .flat_map(|_| [0x20, 0xD0, 0x20, 0xFF])
                .collect();
            let mut c = content("Song", Some(sample_artwork(64, 64)));
            c.source.icon = Some(Artwork::new(16, 16, green).unwrap());
            let m = resolve_media_layout(&dims, c.shape()).unwrap();
            let art = m.artwork_bounds.unwrap();
            let badge = Renderer::badge_rect(art, dims.scale);
            let green_in = |px: &[u32], stride: usize| {
                let (x, y) = (
                    (badge.left + badge.right) / 2.0,
                    (badge.top + badge.bottom) / 2.0,
                );
                let p = px[y as usize * stride + x as usize];
                ((p >> 8) & 0xFF) > 0xA0 && (p & 0xFF) < 0x60
            };
            with(|_| {});
            let (shown, stride) = render_media_frame(&renderer, &dims, &c);
            assert!(green_in(&shown, stride), "badge on by default at {dpi}");
            with(|s| s.show_source_app = false);
            let (hidden, stride) = render_media_frame(&renderer, &dims, &c);
            assert!(!green_in(&hidden, stride), "badge hidden at {dpi}");
            let middle =
                |r: RectF| RectF::new(r.left + 4.0, r.top + 4.0, r.right - 4.0, r.bottom - 4.0);
            assert!(lit_in(&hidden, stride, middle(art), 120), "artwork kept");
            assert!(lit_in(&hidden, stride, m.title_bounds, 120), "title kept");
            // ...and nothing else moves: only the badge (and its cutout ring)
            // differs
            let ring = (4.0 * dims.scale).ceil();
            let around = RectF::new(
                badge.left - ring,
                badge.top - ring,
                badge.right + ring,
                badge.bottom + ring,
            );
            for y in 0..dims.height as usize {
                for x in 0..stride {
                    if !around.contains(x as f32, y as f32) {
                        assert_eq!(shown[y * stride + x], hidden[y * stride + x], "({x},{y})");
                    }
                }
            }

            // Music: bars hidden, the scrubber still drawn; the collapsed
            // notch keeps its bars
            let music =
                crate::layout::space_dimensions(NotchState::Expanded, dpi, NottSpace::Music);
            let c = playing_content(P::Playing);
            let m = resolve_media_layout_in(&music, c.shape(), NottSpace::Music).unwrap();
            let collapsed = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
            let viz_c = resolve_collapsed_layout(&collapsed).visualizer_bounds;
            with(|s| s.show_visualizer = false);
            renderer.set_media(Some(&c));
            renderer.visualizer().step(1000.0);
            let (px, stride) = music_frame(&renderer, &music, &c);
            assert!(
                !accent_in(&px, stride, m.visualizer_bounds),
                "no bars at {dpi}"
            );
            assert!(
                accent_in(&px, stride, m.timeline_bounds),
                "scrubber at {dpi}"
            );
            let (px, stride) = render_collapsed_frame(&renderer, &collapsed);
            assert!(
                accent_in(&px, stride, viz_c),
                "collapsed bars kept at {dpi}"
            );
            // Shown again: the bars return
            with(|_| {});
            let (px, stride) = music_frame(&renderer, &music, &c);
            assert!(
                accent_in(&px, stride, m.visualizer_bounds),
                "bars back at {dpi}"
            );
        }
    }

    #[test]
    fn test_badge_straddles_artwork_corner() {
        let renderer = Renderer::new().expect("renderer");
        for dpi in [96u32, 120, 144, 192, 288] {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            // Bright-green opaque badge so it is easy to find over the orange artwork
            let green: Vec<u8> = (0..16 * 16)
                .flat_map(|_| [0x20, 0xD0, 0x20, 0xFF])
                .collect();
            let mut c = content("Song", Some(sample_artwork(64, 64)));
            c.source.icon = Some(Artwork::new(16, 16, green).unwrap());
            let m = resolve_media_layout(&dims, c.shape()).unwrap();
            let art = m.artwork_bounds.unwrap();
            let badge = Renderer::badge_rect(art, dims.scale);
            let (px, stride) = render_media_frame(&renderer, &dims, &c);
            let green_at = |x: usize, y: usize| {
                let p = px[y * stride + x];
                ((p >> 8) & 0xFF) > 0xA0 && (p & 0xFF) < 0x60
            };
            // Straddles the corner: overhangs the art's right and bottom edges
            assert!(
                badge.right > art.right && badge.bottom > art.bottom,
                "overhang at {dpi}"
            );
            assert!(
                badge.left < art.right && badge.top < art.bottom,
                "overlaps art at {dpi}"
            );
            let (bx, by) = (
                (badge.left + badge.right) / 2.0,
                (badge.top + badge.bottom) / 2.0,
            );
            assert!(green_at(bx as usize, by as usize), "badge drawn at {dpi}");
            assert!(
                green_at((badge.right - 3.0) as usize, by as usize),
                "overhanging part is not clipped at {dpi}"
            );
            // Fully inside the notch body and clear of the text column
            for (x, y) in [
                (badge.right, badge.bottom),
                (badge.left, badge.bottom),
                (badge.right, badge.top),
            ] {
                assert!(
                    dims.contains_point(x, y),
                    "badge corner outside notch at {dpi}"
                );
            }
            assert!(
                badge.right < m.title_bounds.left,
                "badge reaches the text at {dpi}"
            );
            // Black cutout ring between badge and artwork
            let ring = (BASE_MEDIA_BADGE_RING * dims.scale).round().max(1.0);
            let p = px[by as usize * stride + (badge.left - ring / 2.0 - 0.5).floor() as usize];
            assert!(
                (p & 0x00FF_FFFF) < 0x0020_2020,
                "cutout ring is notch black at {dpi}"
            );
            // Nothing green anywhere except the badge
            for y in 0..dims.height as usize {
                for x in 0..dims.width as usize {
                    if green_at(x, y) {
                        assert!(
                            badge.contains(x as f32, y as f32),
                            "stray badge pixel ({x},{y}) {badge:?} at {dpi}"
                        );
                    }
                }
            }
            assert!(
                renderer.badge_bitmap.borrow().is_some(),
                "cached device bitmap"
            );
        }
        // Without an icon: no badge, bitmap released
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 120);
        let c = content("Song", Some(sample_artwork(64, 64)));
        render_media_frame(&renderer, &dims, &c);
        assert!(renderer.badge_bitmap.borrow().is_none());
    }

    #[test]
    fn test_resample_area_smooth_and_exact_size() {
        // 64x64 badge-like icon: left half opaque white, right half transparent
        let px: Vec<u8> = (0..64 * 64)
            .flat_map(|i| {
                if i % 64 < 32 {
                    [255u8, 255, 255, 255]
                } else {
                    [0, 0, 0, 0]
                }
            })
            .collect();
        let src = Artwork::new(64, 64, px).unwrap();
        for d in [15u32, 19, 23, 45] {
            let out = resample_area(&src, d);
            assert_eq!((out.width, out.height), (d, d));
            let at = |x: u32, y: u32| out.pixels[((y * d + x) * 4) as usize..][..4].to_vec();
            assert_eq!(
                at(0, d / 2),
                vec![255, 255, 255, 255],
                "solid area stays solid"
            );
            assert_eq!(at(d - 1, d / 2), vec![0, 0, 0, 0], "empty area stays empty");
            // Total coverage is preserved (area average, no skipped source pixels)
            let alpha_sum: u64 = out.pixels.chunks(4).map(|p| p[3] as u64).sum();
            let expected = 255.0 * (d * d) as f64 / 2.0;
            assert!(
                (alpha_sum as f64 - expected).abs() / expected < 0.03,
                "coverage at {d}"
            );
            // Premultiplied invariant holds
            assert!(
                out.pixels
                    .chunks(4)
                    .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3])
            );
        }
        // Same size is returned unchanged
        assert_eq!(resample_area(&src, 64), src);
    }

    /// Renders the collapsed notch with the renderer's current media/visualizer.
    fn render_collapsed_frame(renderer: &Renderer, dims: &NotchDimensions) -> (Vec<u32>, usize) {
        renderer.ensure_buffer(dims.width, dims.height).unwrap();
        let rt_ref = renderer.cached_rt.borrow();
        let rt = rt_ref.as_ref().unwrap();
        unsafe {
            let bind = RECT {
                left: 0,
                top: 0,
                right: dims.width,
                bottom: dims.height,
            };
            rt.BindDC(renderer.cached_mem_dc.get(), &bind).unwrap();
            rt.BeginDraw();
            rt.Clear(Some(&COLOR_TRANSPARENT));
            let fill = rt.CreateSolidColorBrush(&NOTCH_BG_COLOR, None).unwrap();
            rt.FillGeometry(&renderer.create_notch_geometry(dims).unwrap(), &fill, None);
            renderer
                .render_collapsed_content(rt, &resolve_collapsed_layout(dims), dims, "2:47 PM")
                .unwrap();
            rt.EndDraw(None, None).unwrap();
        }
        let stride = renderer.cached_capacity_w.get() as usize;
        let bits = renderer.cached_bits.get() as *const u32;
        let pixels = unsafe { std::slice::from_raw_parts(bits, stride * dims.height as usize) };
        (pixels.to_vec(), stride)
    }

    /// Blue-dominant pixel (the test accent) somewhere inside `r`.
    fn accent_in(px: &[u32], stride: usize, r: RectF) -> bool {
        (r.top.floor() as usize..r.bottom.ceil() as usize).any(|y| {
            (r.left.floor() as usize..r.right.ceil() as usize).any(|x| {
                let p = px[y * stride + x];
                let (b, g, rr) = (p & 0xFF, (p >> 8) & 0xFF, (p >> 16) & 0xFF);
                b > 120 && b > rr + 60 && b > g + 30
            })
        })
    }

    const TEST_ACCENT: crate::media::Accent = [40, 110, 230];

    fn playing_content(playback: crate::media::PlaybackState) -> MediaContent {
        let now = crate::media::now_filetime();
        MediaContent {
            playback,
            accent: Some(TEST_ACCENT),
            timeline: crate::media::Timeline::from_raw(
                0,
                200 * 10_000_000,
                100 * 10_000_000,
                now,
                now,
            ),
            ..content("Song", Some(sample_artwork(64, 64)))
        }
    }

    #[test]
    fn test_visualizer_follows_playback_in_both_notch_states() {
        use crate::media::PlaybackState as P;
        let renderer = Renderer::new().expect("renderer");
        for dpi in [96u32, 144] {
            let collapsed = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
            let viz_c = resolve_collapsed_layout(&collapsed).visualizer_bounds;

            renderer.set_media(Some(&playing_content(P::Playing)));
            assert!(
                renderer.visualizer().is_active(),
                "playing starts the visualizer"
            );
            renderer.visualizer().step(1000.0);
            let (px, stride) = render_collapsed_frame(&renderer, &collapsed);
            assert!(
                accent_in(&px, stride, viz_c),
                "collapsed bars in the accent at {dpi}"
            );
            // Expanded: the Music space shows the bars (Home's expanded notch
            // has none)
            let expanded =
                crate::layout::space_dimensions(NotchState::Expanded, dpi, NottSpace::Music);
            let c = playing_content(P::Playing);
            let m = resolve_media_layout_in(&expanded, c.shape(), NottSpace::Music).unwrap();
            let (px, stride) = music_frame(&renderer, &expanded, &c);
            assert!(
                accent_in(&px, stride, m.visualizer_bounds),
                "expanded bars at {dpi}"
            );

            // Paused: fades out, then nothing is drawn and no frames are needed
            renderer.set_media(Some(&playing_content(P::Paused)));
            assert!(renderer.visualizer().step(50.0), "fading out");
            assert!(!renderer.visualizer().step(1000.0), "stopped once hidden");
            let (px, stride) = render_collapsed_frame(&renderer, &collapsed);
            assert!(
                !accent_in(&px, stride, viz_c),
                "no bars when paused at {dpi}"
            );
            let (px, stride) = music_frame(&renderer, &expanded, &playing_content(P::Paused));
            assert!(!accent_in(&px, stride, m.visualizer_bounds));

            // No media session: hidden and idle
            renderer.set_media(None);
            assert!(!renderer.visualizer().step(1000.0));
            let (px, stride) = render_collapsed_frame(&renderer, &collapsed);
            assert!(
                !accent_in(&px, stride, viz_c),
                "no bars without media at {dpi}"
            );
        }
    }

    #[test]
    fn test_scrubber_played_part_uses_accent_and_hides_without_timeline() {
        use crate::media::PlaybackState as P;
        let renderer = Renderer::new().expect("renderer");
        // The scrubber lives in the Music space
        let dims = crate::layout::space_dimensions(NotchState::Expanded, 96, NottSpace::Music);
        let c = playing_content(P::Paused);
        let m = resolve_media_layout_in(&dims, c.shape(), NottSpace::Music).unwrap();
        let strip = m.timeline_bounds;
        let (px, stride) = music_frame(&renderer, &dims, &c);
        // Track spans label + gap .. strip end - label - gap; half played
        let track_l = strip.left + BASE_MEDIA_TIMELINE_LABEL_WIDTH + BASE_MEDIA_TIMELINE_LABEL_GAP;
        let track_r = strip.right - BASE_MEDIA_TIMELINE_LABEL_WIDTH - BASE_MEDIA_TIMELINE_LABEL_GAP;
        let cy = ((strip.top + strip.bottom) / 2.0).round();
        let row = |x0: f32, x1: f32| RectF::new(x0, cy - 1.0, x1, cy + 1.0);
        let q = (track_r - track_l) / 4.0;
        assert!(
            accent_in(&px, stride, row(track_l + 2.0, track_l + q)),
            "played = accent"
        );
        assert!(
            !accent_in(&px, stride, row(track_r - q, track_r - 2.0)),
            "unplayed is neutral"
        );
        assert!(
            lit_in(&px, stride, row(track_r - q, track_r - 2.0), 20),
            "unplayed track drawn"
        );
        // Labels: "1:40" and "-1:40"
        assert!(
            lit_in(
                &px,
                stride,
                RectF {
                    right: track_l - 2.0,
                    ..strip
                },
                60
            ),
            "elapsed label"
        );
        assert!(
            lit_in(
                &px,
                stride,
                RectF {
                    left: track_r + 2.0,
                    ..strip
                },
                60
            ),
            "remaining label"
        );

        // Neutral fallback: no artwork accent -> the neutral UI colour, never a hue
        let neutral = MediaContent {
            accent: None,
            ..c.clone()
        };
        let (px, stride) = music_frame(&renderer, &dims, &neutral);
        assert!(!accent_in(&px, stride, strip));
        assert!(
            lit_in(&px, stride, row(track_l + 2.0, track_l + q), 150),
            "neutral played part"
        );

        // No timeline (live stream / unsupported): nothing in the strip
        let none = MediaContent {
            timeline: None,
            ..c
        };
        let (px, stride) = music_frame(&renderer, &dims, &none);
        assert!(!lit_in(&px, stride, strip, 20), "scrubber omitted");
    }

    /// Expanded notch body plus the space selector only.
    fn render_selector_frame(renderer: &Renderer, dims: &NotchDimensions) -> (Vec<u32>, usize) {
        renderer.ensure_buffer(dims.width, dims.height).unwrap();
        let rt_ref = renderer.cached_rt.borrow();
        let rt = rt_ref.as_ref().unwrap();
        unsafe {
            let bind = RECT {
                left: 0,
                top: 0,
                right: dims.width,
                bottom: dims.height,
            };
            rt.BindDC(renderer.cached_mem_dc.get(), &bind).unwrap();
            rt.BeginDraw();
            rt.Clear(Some(&COLOR_TRANSPARENT));
            let fill = rt.CreateSolidColorBrush(&NOTCH_BG_COLOR, None).unwrap();
            rt.FillGeometry(&renderer.create_notch_geometry(dims).unwrap(), &fill, None);
            renderer.draw_space_selector(rt, dims).unwrap();
            rt.EndDraw(None, None).unwrap();
        }
        let stride = renderer.cached_capacity_w.get() as usize;
        let bits = renderer.cached_bits.get() as *const u32;
        let pixels = unsafe { std::slice::from_raw_parts(bits, stride * dims.height as usize) };
        (pixels.to_vec(), stride)
    }

    #[test]
    fn test_space_selector_reflects_active_space() {
        let renderer = Renderer::new().expect("renderer");
        for dpi in [96u32, 144] {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            // The pills follow the active space's content edge
            let sel = resolve_space_selector_in(&dims, NottSpace::Home).unwrap();
            // Capsule fill sampled away from the label text (left end, centered)
            let fill_at = |px: &[u32], stride: usize, s: NottSpace, active: NottSpace| {
                let r = resolve_space_selector_in(&dims, active).unwrap().bounds(s);
                let (x, y) = (
                    (r.left + r.height() * 0.35) as usize,
                    ((r.top + r.bottom) / 2.0) as usize,
                );
                px[y * stride + x] & 0xFF
            };
            // Default: Home selected
            renderer.set_space(NottSpace::default());
            let (px, stride) = render_selector_frame(&renderer, &dims);
            assert!(
                fill_at(&px, stride, NottSpace::Home, NottSpace::Home) > 15,
                "Home capsule filled at {dpi}"
            );
            assert_eq!(
                fill_at(&px, stride, NottSpace::Music, NottSpace::Home),
                0,
                "Music unfilled at {dpi}"
            );
            assert!(
                lit_in(&px, stride, sel.music, 60),
                "Music label still visible"
            );
            // Music, then back to Home, repeatedly: the drawing follows the state
            for target in [NottSpace::Music, NottSpace::Home, NottSpace::Music] {
                renderer.set_space(target);
                let (px, stride) = render_selector_frame(&renderer, &dims);
                for s in NottSpace::ALL {
                    assert_eq!(
                        fill_at(&px, stride, s, target) > 15,
                        s == target,
                        "{s:?} at {dpi}"
                    );
                }
            }
            renderer.set_space(NottSpace::Home);
        }
        assert!(
            !renderer.set_space(NottSpace::Home),
            "no change, no redraw needed"
        );
        assert!(renderer.set_space(NottSpace::Music));
    }

    #[test]
    fn test_space_selector_hidden_when_collapsed_and_media_untouched() {
        let renderer = Renderer::new().expect("renderer");
        let c = content("Song", Some(sample_artwork(64, 64)));
        renderer.set_media(Some(&c));
        renderer.set_space(NottSpace::Music);
        // Collapsed: no selector at all (notch body stays pure black)
        let collapsed = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, 96);
        let (px, stride) = render_selector_frame(&renderer, &collapsed);
        let all = RectF::new(0.0, 0.0, collapsed.width as f32, collapsed.height as f32);
        assert!(!lit_in(&px, stride, all, 5), "nothing drawn when collapsed");
        // Expanding again shows the still-active Music space
        let expanded = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 96);
        let sel = resolve_space_selector_in(&expanded, NottSpace::Music).unwrap();
        let (px, stride) = render_selector_frame(&renderer, &expanded);
        let r = sel.music;
        let (x, y) = (
            (r.left + r.height() * 0.35) as usize,
            ((r.top + r.bottom) / 2.0) as usize,
        );
        assert!(
            px[y * stride + x] & 0xFF > 15,
            "Music still selected after re-expanding"
        );
        // Switching spaces never touches the shared media content
        assert!(
            !renderer.set_media(Some(&c)),
            "media unchanged by space switches"
        );
        renderer.set_space(NottSpace::Home);
        assert!(!renderer.set_media(Some(&c)));
    }

    /// Renders an arbitrary frame: notch body, then `draw`.
    fn render_custom_frame(
        renderer: &Renderer,
        dims: &NotchDimensions,
        draw: impl FnOnce(&ID2D1RenderTarget),
    ) -> (Vec<u32>, usize) {
        renderer.ensure_buffer(dims.width, dims.height).unwrap();
        let rt_ref = renderer.cached_rt.borrow();
        let rt = rt_ref.as_ref().unwrap();
        unsafe {
            let bind = RECT {
                left: 0,
                top: 0,
                right: dims.width,
                bottom: dims.height,
            };
            rt.BindDC(renderer.cached_mem_dc.get(), &bind).unwrap();
            rt.BeginDraw();
            rt.Clear(Some(&COLOR_TRANSPARENT));
            let fill = rt.CreateSolidColorBrush(&NOTCH_BG_COLOR, None).unwrap();
            rt.FillGeometry(&renderer.create_notch_geometry(dims).unwrap(), &fill, None);
            draw(rt);
            rt.EndDraw(None, None).unwrap();
        }
        let stride = renderer.cached_capacity_w.get() as usize;
        let bits = renderer.cached_bits.get() as *const u32;
        let pixels = unsafe { std::slice::from_raw_parts(bits, stride * dims.height as usize) };
        (pixels.to_vec(), stride)
    }

    fn music_frame(
        renderer: &Renderer,
        dims: &NotchDimensions,
        c: &MediaContent,
    ) -> (Vec<u32>, usize) {
        renderer.set_media(Some(c));
        renderer.feedback().finish_track_fade();
        let clock = ClockDateState::from_pure_components(2026, 10, 6, 2, 0, 47, 0, 0, false);
        render_custom_frame(renderer, dims, |rt| {
            let ResolvedLayout::Expanded { components, .. } = resolve_layout(dims) else {
                panic!("expanded");
            };
            let media = renderer.media.borrow();
            let media = media.as_ref().unwrap();
            let m = resolve_media_layout_in(dims, media.content.shape(), NottSpace::Music).unwrap();
            renderer
                .render_media_content(rt, &components, &m, dims, &clock, media)
                .unwrap();
        })
    }

    #[test]
    fn test_music_space_renders_focused_player_without_clock() {
        let renderer = Renderer::new().expect("renderer");
        for dpi in [96u32, 144] {
            let dims = crate::layout::space_dimensions(NotchState::Expanded, dpi, NottSpace::Music);
            let c = playing_content(crate::media::PlaybackState::Playing);
            renderer.set_media(Some(&c));
            renderer.visualizer().step(1000.0);
            let m = resolve_media_layout_in(&dims, c.shape(), NottSpace::Music).unwrap();
            let (px, stride) = music_frame(&renderer, &dims, &c);
            assert!(lit_in(&px, stride, m.title_bounds, 80), "title at {dpi}");
            assert!(
                lit_in(&px, stride, m.artwork_bounds.unwrap(), 80),
                "artwork at {dpi}"
            );
            assert!(
                accent_in(&px, stride, m.timeline_bounds),
                "accent scrubber at {dpi}"
            );
            assert!(
                accent_in(&px, stride, m.visualizer_bounds),
                "visualizer at {dpi}"
            );
            // No time/date column in Music
            assert_eq!(m.time_bounds.width(), 0.0);
            assert_eq!(m.date_bounds.width(), 0.0);
            // Long title stays ellipsis-trimmed inside its own bounds
            let long = MediaContent {
                title: LONG_TITLE.into(),
                ..c.clone()
            };
            let (px, stride) = music_frame(&renderer, &dims, &long);
            let past_title = RectF::new(
                m.title_bounds.right + 1.0,
                m.title_bounds.top,
                m.visualizer_bounds.left - 1.0,
                m.title_bounds.bottom,
            );
            assert!(
                !lit_in(&px, stride, past_title, 40),
                "long title trimmed at {dpi}"
            );
        }
    }

    #[test]
    fn test_music_empty_state_shows_no_stale_media() {
        let renderer = Renderer::new().expect("renderer");
        let dims = crate::layout::space_dimensions(NotchState::Expanded, 96, NottSpace::Music);
        // A previous track was showing, then the session went away
        renderer.set_media(Some(&playing_content(crate::media::PlaybackState::Playing)));
        renderer.set_media(None);
        assert!(renderer.media.borrow().is_none(), "media cleared");
        assert!(
            renderer.art_bitmap.borrow().is_none(),
            "artwork bitmap released"
        );
        renderer.visualizer().step(1000.0);
        assert_eq!(renderer.visualizer().level(), 0.0, "visualizer hidden");
        let ResolvedLayout::Expanded { components, .. } = resolve_layout(&dims) else {
            unreachable!()
        };
        let (px, stride) = render_custom_frame(&renderer, &dims, |rt| {
            renderer.draw_music_empty(rt, &components, &dims).unwrap();
        });
        let c = components.content_bounds;
        assert!(lit_in(&px, stride, c, 60), "quiet empty label");
        assert!(!accent_in(&px, stride, c), "no stale accent");
        // Only a centered label: the cover slot (left) stays black
        let cover_slot = RectF::new(c.left, c.top, c.left + 60.0, c.bottom);
        assert!(!lit_in(&px, stride, cover_slot, 20), "no stale artwork");
    }

    fn clipboard_frame(renderer: &Renderer, dims: &NotchDimensions) -> (Vec<u32>, usize) {
        render_custom_frame(renderer, dims, |rt| {
            renderer.draw_clipboard(rt, dims).unwrap()
        })
    }

    fn row_kinds(renderer: &Renderer) -> Vec<&'static str> {
        renderer
            .clipboard
            .borrow()
            .rows
            .iter()
            .map(|r| match r {
                ClipRow::Text(_) => "text",
                ClipRow::Image(..) => "image",
            })
            .collect()
    }

    #[test]
    fn test_clipboard_empty_state_renders() {
        let renderer = Renderer::new().expect("renderer");
        renderer.set_clipboard(&ClipboardHistory::default());
        for dpi in ALL_DPIS {
            let dims =
                crate::layout::space_dimensions(NotchState::Expanded, dpi, NottSpace::Clipboard);
            let l = resolve_clipboard_layout(&dims).unwrap();
            let (px, stride) = clipboard_frame(&renderer, &dims);
            assert!(lit_in(&px, stride, l.list, 60), "empty label at {dpi}");
            assert!(
                !lit_in(&px, stride, l.clear, 12),
                "no Clear when empty at {dpi}"
            );
            // No row tiles: the first row's left edge strip stays black
            let r = l.rows[0];
            let edge = RectF::new(r.left, r.top, r.left + 6.0, r.bottom);
            assert!(!lit_in(&px, stride, edge, 8), "no rows at {dpi}");
        }
        assert!(renderer.clipboard.borrow().rows.is_empty());
    }

    #[test]
    fn test_clear_confirmation_replaces_rows_and_x() {
        let renderer = Renderer::new().expect("renderer");
        let mut history = ClipboardHistory::default();
        history.add(ClipboardItem::Image(sample_artwork(300, 100)));
        renderer.set_clipboard(&history);
        let dims = crate::layout::space_dimensions(NotchState::Expanded, 120, NottSpace::Clipboard);
        let l = resolve_clipboard_layout(&dims).unwrap();
        let (rows_px, stride) = clipboard_frame(&renderer, &dims);
        assert!(lit_in(&rows_px, stride, l.clear, 60), "X before asking");
        renderer.set_clear_pending(true);
        let (px, stride) = clipboard_frame(&renderer, &dims);
        assert!(!lit_in(&px, stride, l.clear, 60), "X hidden while asking");
        for r in [l.confirm_message, l.confirm_note, l.cancel, l.confirm_clear] {
            assert!(lit_in(&px, stride, r, 60), "{r:?} drawn");
        }
        assert_ne!(px, rows_px, "rows replaced");
        // Dismissed: exactly the original rows again
        renderer.set_clear_pending(false);
        assert_eq!(clipboard_frame(&renderer, &dims).0, rows_px);
        // Pending with an empty history shows the empty state, not the question
        renderer.set_clear_pending(true);
        renderer.set_clipboard(&ClipboardHistory::default());
        let (px, stride) = clipboard_frame(&renderer, &dims);
        assert!(!lit_in(&px, stride, l.cancel, 60));
    }

    #[test]
    fn test_clipboard_rows_newest_first_with_thumbnail() {
        let renderer = Renderer::new().expect("renderer");
        let mut history = ClipboardHistory::default();
        history.add(ClipboardItem::Text("first".into()));
        history.add(ClipboardItem::Image(sample_artwork(300, 100)));
        history.add(ClipboardItem::Text("third\nline\n".into()));
        renderer.set_clipboard(&history);
        assert_eq!(row_kinds(&renderer), ["text", "image", "text"]);
        let first_row = || match &renderer.clipboard.borrow().rows[0] {
            ClipRow::Text(t) => String::from_utf16(t).unwrap(),
            ClipRow::Image(..) => unreachable!(),
        };
        assert_eq!(first_row(), "third line", "newline shown as a space");
        assert_eq!(
            history.get(0),
            Some(&ClipboardItem::Text("third\nline\n".into())),
            "entry keeps its text"
        );

        let dims = crate::layout::space_dimensions(NotchState::Expanded, 144, NottSpace::Clipboard);
        let l = resolve_clipboard_layout(&dims).unwrap();
        let (px, stride) = clipboard_frame(&renderer, &dims);
        assert!(lit_in(&px, stride, l.clear, 60), "Clear shown");
        // Image row: orange thumbnail at the row's left
        let r = l.rows[1];
        let (x, y) = (
            (r.left + r.height() / 2.0) as usize,
            ((r.top + r.bottom) / 2.0) as usize,
        );
        let p = px[y * stride + x];
        assert!(
            (p >> 16) & 0xFF > 200 && p & 0xFF < 100,
            "thumbnail drawn: {p:08x}"
        );
        assert!(lit_in(&px, stride, l.rows[0], 60) && lit_in(&px, stride, l.rows[2], 60));
        assert!(!lit_in(&px, stride, l.rows[3], 12), "only three rows");
        // Buttons are hidden until a row is hovered
        for r in &l.rows[..3] {
            let (_, copy, trash) = ClipboardLayout::row_parts(*r);
            let icons = RectF::new(
                copy.left + 4.0,
                copy.top + 5.0,
                trash.right - 4.0,
                copy.bottom - 5.0,
            );
            assert!(!lit_in(&px, stride, icons, 60), "no buttons without hover");
        }
        // Hovering row 0 shows only row 0's buttons
        renderer.feedback().set_clip_row(Some(0));
        while renderer.feedback().step(16.0) {}
        let (px, stride) = clipboard_frame(&renderer, &dims);
        for (i, r) in l.rows[..3].iter().enumerate() {
            let (_, copy, trash) = ClipboardLayout::row_parts(*r);
            let lit = lit_in(&px, stride, copy, 120) && lit_in(&px, stride, trash, 120);
            assert_eq!(lit, i == 0, "row {i} buttons");
        }
        renderer.feedback().reset();
        let (px, stride) = clipboard_frame(&renderer, &dims);
        for r in &l.rows[..3] {
            let entry = *r;
            // Outline only, spanning the whole row: both side edges are lit
            let right_edge = RectF::new(
                entry.right - 1.5,
                entry.top + 8.0,
                entry.right,
                entry.bottom - 8.0,
            );
            assert!(
                lit_in(&px, stride, right_edge, 25),
                "outline reaches the row end"
            );
            // Outline only: the box's edge is lit, its empty middle stays black,
            // and it closes off before the buttons
            let edge = RectF::new(
                entry.left,
                entry.top + 6.0,
                entry.left + 1.5,
                entry.bottom - 6.0,
            );
            assert!(lit_in(&px, stride, edge, 25), "outline edge");
            let inside = RectF::new(
                entry.right - 40.0,
                entry.top + 6.0,
                entry.right - 14.0,
                entry.bottom - 6.0,
            );
            assert!(!lit_in(&px, stride, inside, 12), "no filled tile");
        }
        // The thumbnail bitmap is cached and reused, not rebuilt per frame
        let first = renderer.thumbs.borrow()[0].2.clone();
        clipboard_frame(&renderer, &dims);
        assert_eq!(renderer.thumbs.borrow().len(), 1);
        assert_eq!(renderer.thumbs.borrow()[0].2, first);

        // Clear: rows, label and thumbnail memory all go; the empty state is valid
        history.clear();
        renderer.set_clipboard(&history);
        assert!(renderer.clipboard.borrow().rows.is_empty());
        assert!(renderer.thumbs.borrow().is_empty(), "thumbnails released");
        let (px, stride) = clipboard_frame(&renderer, &dims);
        assert!(lit_in(&px, stride, l.list, 60), "empty state after clear");
        assert!(!lit_in(&px, stride, l.clear, 12));
    }

    #[test]
    fn test_clipboard_updates_refresh_view_without_touching_media() {
        let renderer = Renderer::new().expect("renderer");
        let media = playing_content(crate::media::PlaybackState::Playing);
        renderer.set_media(Some(&media));
        let mut history = ClipboardHistory::default();
        history.add(ClipboardItem::Text("one".into()));
        renderer.set_clipboard(&history);
        assert_eq!(row_kinds(&renderer).len(), 1);
        history.add(ClipboardItem::Text("two".into()));
        renderer.set_clipboard(&history);
        assert_eq!(row_kinds(&renderer).len(), 2);
        // Restore: promoted entry is first, no duplicate row
        history.promote(1);
        renderer.set_clipboard(&history);
        let first_row = || match &renderer.clipboard.borrow().rows[0] {
            ClipRow::Text(t) => String::from_utf16(t).unwrap(),
            ClipRow::Image(..) => unreachable!(),
        };
        assert_eq!(first_row(), "one");
        assert_eq!(row_kinds(&renderer).len(), 2);
        // Home -> Clipboard -> Music -> Home keeps both media and clipboard state
        for s in [NottSpace::Clipboard, NottSpace::Music, NottSpace::Home] {
            renderer.set_space(s);
        }
        assert_eq!(renderer.media.borrow().as_ref().unwrap().content, media);
        assert_eq!(row_kinds(&renderer).len(), 2);
        // At most the visible rows are kept, however long the history
        for i in 0..20 {
            history.add(ClipboardItem::Text(format!("t{i}")));
        }
        renderer.set_clipboard(&history);
        assert_eq!(row_kinds(&renderer).len(), CLIPBOARD_VISIBLE_ROWS);
    }

    /// The expanded content of one transition frame.
    fn transition_frame(
        renderer: &Renderer,
        dims: &NotchDimensions,
        t: Transition,
    ) -> (Vec<u32>, usize) {
        renderer.set_transition(Some(t));
        let clock = ClockDateState::from_pure_components(2026, 10, 6, 2, 0, 47, 0, 0, false);
        let frame = render_custom_frame(renderer, dims, |rt| {
            renderer.draw_expanded(rt, dims, &clock).unwrap()
        });
        renderer.set_transition(None);
        frame
    }

    #[test]
    fn test_transition_frames_blend_inside_the_notch() {
        let renderer = Renderer::new().expect("renderer");
        renderer.set_media(Some(&playing_content(crate::media::PlaybackState::Playing)));
        renderer.feedback().finish_track_fade();
        let mut h = ClipboardHistory::default();
        h.add(ClipboardItem::Text("row".into()));
        renderer.set_clipboard(&h);
        let home = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 120);
        // A shell mid-way between sizes: every scene is laid out at its own
        // settled size, so parts overhang and must be clipped to the notch
        let mid = home.with_width_dip(470.0).with_height_dip(170.0);
        let (bare, _) = render_custom_frame(&renderer, &mid, |_| {});
        let s = |x| Scene::Space(x);
        let pairs = [
            (s(NottSpace::Home), s(NottSpace::Music)),
            (s(NottSpace::Music), s(NottSpace::Clipboard)),
            (s(NottSpace::Clipboard), s(NottSpace::Home)),
            (s(NottSpace::Home), Scene::Drop),
        ];
        for (from, to) in pairs {
            for mix in [0.0, 0.3, 0.5, 0.7, 1.0] {
                let t = Transition {
                    from,
                    to,
                    mix,
                    alpha: 1.0,
                };
                let (px, _) = transition_frame(&renderer, &mid, t);
                for (i, (&p, &b)) in px.iter().zip(&bare).enumerate() {
                    if b >> 24 == 0 {
                        assert_eq!(p, 0, "outside the notch at {i} ({from:?}->{to:?} {mix})");
                    }
                }
                assert!(px != bare, "content drawn ({from:?}->{to:?} {mix})");
            }
        }
        // Content opacity: 0 leaves only the black shell; half is dimmer
        let t = |alpha| Transition {
            from: s(NottSpace::Music),
            to: s(NottSpace::Music),
            mix: 1.0,
            alpha,
        };
        assert!(transition_frame(&renderer, &mid, t(0.0)).0 == bare);
        let peak = |px: &[u32]| px.iter().map(|p| p & 0xFF).max().unwrap();
        let (full, _) = transition_frame(&renderer, &mid, t(1.0));
        let (half, _) = transition_frame(&renderer, &mid, t(0.5));
        assert!(peak(&half) < peak(&full));
    }

    #[test]
    fn test_user_accent_colors_switches_and_media_fallback_only() {
        // The playback accent: the artwork's own when it has one, otherwise
        // the user's (White by default: the original neutral)
        let with_art = playing_content(crate::media::PlaybackState::Playing);
        let no_art = MediaContent {
            accent: None,
            ..with_art.clone()
        };
        for a in AccentChoice::ALL {
            assert_eq!(accent_color(Some(&with_art), a), rgb_color(TEST_ACCENT));
            assert_eq!(accent_color(Some(&no_art), a), a.color());
            assert_eq!(accent_color(None, a), a.color());
        }
        // An "on" switch takes the accent; the surface stays pure black
        let renderer = Renderer::new().expect("renderer");
        let clock = ClockDateState::from_pure_components(2026, 10, 6, 2, 0, 47, 0, 0, false);
        let dims = crate::layout::space_dimensions(NotchState::Expanded, 120, PANEL_SPACE);
        let l = resolve_settings_layout(&dims).unwrap();
        for a in AccentChoice::ALL {
            renderer.set_settings(
                true,
                NottSettings {
                    accent: a,
                    ..NottSettings::default()
                },
            );
            let (px, stride) = render_custom_frame(&renderer, &dims, |rt| {
                renderer.draw_expanded(rt, &dims, &clock).unwrap()
            });
            let at = |x: f32, y: f32| px[y as usize * stride + x as usize];
            let t = l.always_on_top.toggle;
            let p = at(t.left + 4.0, (t.top + t.bottom) / 2.0);
            let got = [(p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF];
            for (g, w) in got.iter().zip(a.rgb()) {
                assert!(g.abs_diff(u32::from(w)) <= 3, "{a:?}: {got:?}");
            }
            // Off switch (Reduced motion) stays neutral; black surface below
            let r = l.reduced_motion.toggle;
            let off = at(r.right - 4.0, (r.top + r.bottom) / 2.0);
            assert!((off & 0xFF) < 80, "{a:?} off track");
            let body = at(
                dims.width as f32 / 2.0,
                (dims.height as f32 - dims.shadow_margin_bottom) - 8.0,
            );
            assert_eq!(body, 0xFF00_0000, "{a:?} opaque AMOLED-black surface");
        }
    }

    #[test]
    fn test_settings_page_shows_switches_and_action_and_touches_nothing_else() {
        let renderer = Renderer::new().expect("renderer");
        let media = playing_content(crate::media::PlaybackState::Playing);
        renderer.set_media(Some(&media));
        let mut h = ClipboardHistory::default();
        h.add(ClipboardItem::Text("kept".into()));
        renderer.set_clipboard(&h);
        let clock = ClockDateState::from_pure_components(2026, 10, 6, 2, 0, 47, 0, 0, false);
        for dpi in [96, 120] {
            // Opened from any space, Settings is drawn at the panel size
            let dims = crate::layout::space_dimensions(NotchState::Expanded, dpi, PANEL_SPACE);
            let l = resolve_settings_layout(&dims).unwrap();
            for space in NottSpace::ALL {
                renderer.set_space(space);
                for (aot, rm) in [(true, false), (false, true)] {
                    let s = NottSettings {
                        always_on_top: aot,
                        reduced_motion: rm,
                        ..NottSettings::default()
                    };
                    renderer.set_settings(true, s);
                    let (px, stride) = render_custom_frame(&renderer, &dims, |rt| {
                        renderer.draw_expanded(rt, &dims, &clock).unwrap()
                    });
                    let mid_y = |t: RectF| ((t.top + t.bottom) / 2.0) as usize;
                    let at = |t: RectF, x: f32| px[mid_y(t) * stride + x as usize] & 0xFF;
                    for (row, on) in [(l.always_on_top, aot), (l.reduced_motion, rm)] {
                        let t = row.toggle;
                        assert!(
                            lit_in(&px, stride, row.title, 120),
                            "{space:?} title at {dpi}"
                        );
                        if on {
                            assert!(at(t, t.left + 4.0) > 200, "on: lit track at {dpi}");
                            assert!(at(t, t.right - t.height() / 2.0) < 60, "on: dark knob");
                        } else {
                            assert!(at(t, t.left + t.height() / 2.0) > 200, "off: white knob");
                            assert!(at(t, t.right - 4.0) < 80, "off: dim track");
                        }
                    }
                    // The action row: text and a chevron, no switch track
                    let a = l.full_settings;
                    assert!(lit_in(
                        &px,
                        stride,
                        RectF::new(a.left, a.top, a.left + 60.0, a.bottom),
                        120
                    ));
                    assert!(lit_in(
                        &px,
                        stride,
                        RectF::new(a.right - 12.0, a.top, a.right, a.bottom),
                        60
                    ));
                    let mid =
                        RectF::new(a.right - 60.0, a.top + 2.0, a.right - 16.0, a.bottom - 2.0);
                    assert!(!lit_in(&px, stride, mid, 20), "not a switch");
                }
            }
        }
        renderer.set_settings(false, NottSettings::default());
        // Nothing shared changed
        assert_eq!(renderer.media.borrow().as_ref().unwrap().content, media);
        assert_eq!(row_kinds(&renderer), ["text"]);
    }

    #[test]
    fn test_settled_frame_unchanged_by_transition_path() {
        // A settled transition (from == to, mix 1, alpha 1) draws exactly what
        // the settled renderer draws
        let renderer = Renderer::new().expect("renderer");
        renderer.set_media(Some(&playing_content(crate::media::PlaybackState::Paused)));
        renderer.feedback().finish_track_fade();
        for space in NottSpace::ALL {
            renderer.set_space(space);
            let dims = crate::layout::space_dimensions(NotchState::Expanded, 144, space);
            let clock = ClockDateState::from_pure_components(2026, 10, 6, 2, 0, 47, 0, 0, false);
            let (settled, _) = render_custom_frame(&renderer, &dims, |rt| {
                renderer.draw_expanded(rt, &dims, &clock).unwrap()
            });
            let t = Transition {
                from: Scene::Space(space),
                to: Scene::Space(space),
                mix: 1.0,
                alpha: 1.0,
            };
            assert!(
                transition_frame(&renderer, &dims, t).0 == settled,
                "{space:?}"
            );
        }
    }

    #[test]
    fn test_drop_page_revealed_in_place_and_clipped_to_the_notch() {
        let renderer = Renderer::new().expect("renderer");
        let full = drop_page_dimensions(144);
        let (big, stride_big) = render_custom_frame(&renderer, &full, |rt| {
            renderer.draw_drop_page(rt, &full).unwrap()
        });
        // Mid-opening frames (narrower/shorter silhouettes, e.g. from Music)
        for (w, h) in [(384.0, 158.0), (300.0, 70.0), (420.0, 110.0)] {
            let dims = full.with_width_dip(w).with_height_dip(h);
            let (bare, stride) = render_custom_frame(&renderer, &dims, |_| {});
            let (px, _) = render_custom_frame(&renderer, &dims, |rt| {
                renderer.draw_drop_page(rt, &dims).unwrap()
            });
            let dx = (full.width - dims.width) as usize / 2;
            for y in 0..dims.height as usize {
                for x in 0..dims.width as usize {
                    let (p, b) = (px[y * stride + x], bare[y * stride + x]);
                    if b >> 24 == 0 {
                        assert_eq!(p, 0, "nothing outside the silhouette at {x},{y} ({w}x{h})");
                    } else if y < full.height as usize - 30 && p & 0xFF > 200 {
                        // Lit pixels are exactly where the settled page has them
                        let q = big[y * stride_big + x + dx];
                        assert!(
                            q & 0xFF > 120,
                            "page revealed in place at {x},{y} ({w}x{h})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_drop_page_icon_and_dashed_outline_replace_content() {
        let renderer = Renderer::new().expect("renderer");
        assert!(renderer.set_drop_page(true) && !renderer.set_drop_page(true));
        for dpi in ALL_DPIS {
            let dims = drop_page_dimensions(dpi);
            let (px, stride) = render_custom_frame(&renderer, &dims, |rt| {
                renderer.draw_drop_page(rt, &dims).unwrap()
            });
            let (w, body) = (
                dims.width as usize,
                dims.height - dims.shadow_margin_bottom as i32,
            );
            // Inbox glyph at the centre of the outline
            let (cx, cy) = (w / 2, (body as f32 / 2.0) as usize);
            let s = dims.scale;
            let icon = RectF::new(
                cx as f32 - 12.0 * s,
                cy as f32 - 12.0 * s,
                cx as f32 + 12.0 * s,
                cy as f32 + 12.0 * s,
            );
            assert!(lit_in(&px, stride, icon, 150), "inbox glyph at {dpi}");
            // Bottom edge of the outline: dashes, i.e. lit and dark runs alternate
            let inset = (8.0 * s).round() + (1.5 * s).round().max(1.0) / 2.0;
            let y = (body as f32 - inset).round() as usize;
            let row: Vec<bool> = (w / 4..3 * w / 4)
                .map(|x| (px[y * stride + x] & 0xFF) > 40 || (px[(y - 1) * stride + x] & 0xFF) > 40)
                .collect();
            let runs = row.windows(2).filter(|p| p[0] != p[1]).count();
            assert!(
                row.iter().any(|b| *b) && runs > 10,
                "dashed outline at {dpi}: {runs}"
            );
        }
        assert!(renderer.set_drop_page(false));
    }

    #[test]
    fn test_clipboard_thumbnail_center_crop_and_rounded() {
        // 300x100: green | orange | blue thirds; the centre square is orange
        let (w, h) = (300u32, 100u32);
        let px: Vec<u8> = (0..w * h)
            .flat_map(|i| match (i % w) / 100 {
                0 => [0x00, 0xFF, 0x00, 0xFF],
                1 => [0x20, 0x80, 0xF0, 0xFF],
                _ => [0xFF, 0x00, 0x00, 0xFF],
            })
            .collect();
        let t = thumbnail(&Artwork::new(w, h, px).unwrap(), 20, 5.0);
        assert_eq!(
            (t.width, t.height),
            (20, 20),
            "square, aspect kept by cropping"
        );
        let at = |x: usize, y: usize| &t.pixels[(y * 20 + x) * 4..(y * 20 + x) * 4 + 4];
        assert_eq!(
            at(10, 10),
            [0x20, 0x80, 0xF0, 0xFF],
            "centre is the middle third"
        );
        assert_eq!(
            at(1, 10),
            [0x20, 0x80, 0xF0, 0xFF],
            "edges too: no side bands"
        );
        assert_eq!(at(0, 0)[3], 0, "rounded corner is transparent");
        // Premultiplied: no channel exceeds alpha anywhere
        assert!(
            t.pixels
                .chunks(4)
                .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3])
        );
    }

    #[test]
    fn test_shadow_follows_the_music_width() {
        let renderer = Renderer::new().expect("renderer");
        let dims = crate::layout::space_dimensions(NotchState::Expanded, 96, NottSpace::Music);
        let (px, stride) = render_custom_frame(&renderer, &dims, |_| {});
        let (w, h) = (dims.width as usize, dims.height as usize);
        let mut bytes: Vec<u8> = (0..h)
            .flat_map(|y| {
                px[y * stride..y * stride + w]
                    .iter()
                    .flat_map(|p| p.to_le_bytes())
            })
            .collect();
        let (mut a, mut b) = (Vec::new(), Vec::new());
        apply_ambient_shadow(
            bytes.as_mut_ptr(),
            w,
            h,
            w,
            dims.scale,
            dims.shadow_opacity,
            &mut a,
            &mut b,
        );
        let alpha = |x: usize, y: usize| bytes[(y * w + x) * 4 + 3];
        let wall = (dims.shadow_margin_x + dims.curvature.top_transition_radius) as usize;
        let mid = (dims.notch_height() * 0.7) as usize;
        assert!(
            alpha(wall - 3, mid) > 20,
            "shadow along the Music notch's left wall"
        );
        assert!(alpha(w - wall + 2, mid) > 20, "and its right wall");
        assert!(
            alpha(w / 2, dims.notch_height() as usize + 2) > 40,
            "and beneath it"
        );
        assert_eq!(alpha(0, mid), 0, "fades out inside the narrower window");
    }

    #[test]
    fn test_home_has_no_scrubber_or_visualizer_but_keeps_its_overview() {
        use crate::media::PlaybackState as P;
        let renderer = Renderer::new().expect("renderer");
        for dpi in [96u32, 144] {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            let c = playing_content(P::Playing);
            renderer.set_media(Some(&c));
            renderer.visualizer().step(1000.0);
            let m = resolve_media_layout(&dims, c.shape()).unwrap();
            let (px, stride) = render_media_frame(&renderer, &dims, &c);
            // Where Music's scrubber (labels + track) would be: nothing at all
            let next = m.control_visual_bounds(MediaControl::Next);
            let row = RectF::new(
                next.right + 2.0,
                next.top,
                m.title_bounds.right,
                next.bottom,
            );
            assert!(!lit_in(&px, stride, row, 20), "no scrubber/labels at {dpi}");
            // No visualizer either: the accent appears nowhere in Home
            let ResolvedLayout::Expanded { components, .. } = resolve_layout(&dims) else {
                unreachable!()
            };
            let text = RectF::new(
                m.title_bounds.left,
                components.content_bounds.top,
                m.title_bounds.right,
                components.content_bounds.bottom,
            );
            assert!(!accent_in(&px, stride, text), "no Home visualizer at {dpi}");
            // Overview content remains
            for (what, r) in [
                ("time", m.time_bounds),
                ("date", m.date_bounds),
                ("title", m.title_bounds),
                ("artist", m.artist_bounds),
                ("source", m.source_bounds),
                ("artwork", m.artwork_bounds.unwrap()),
            ] {
                assert!(lit_in(&px, stride, r, 60), "{what} at {dpi}");
            }
            for k in MediaControl::ALL {
                assert!(
                    lit_in(&px, stride, m.control_visual_bounds(k), 60),
                    "{k:?} at {dpi}"
                );
            }
        }
    }

    #[test]
    fn test_music_cover_has_no_badge() {
        let renderer = Renderer::new().expect("renderer");
        for dpi in [96u32, 144] {
            let dims = crate::layout::space_dimensions(NotchState::Expanded, dpi, NottSpace::Music);
            let green: Vec<u8> = (0..16 * 16)
                .flat_map(|_| [0x20, 0xD0, 0x20, 0xFF])
                .collect();
            let mut c = content("Song", Some(sample_artwork(64, 64)));
            c.source.icon = Some(Artwork::new(16, 16, green).unwrap());
            assert!(c.shows_badge(), "Home would show the badge");
            renderer.set_space(NottSpace::Music);
            let (px, stride) = music_frame(&renderer, &dims, &c);
            renderer.set_space(NottSpace::Home);
            let green_at = |p: u32| ((p >> 8) & 0xFF) > 0xA0 && (p & 0xFF) < 0x60;
            for y in 0..dims.height as usize {
                for x in 0..dims.width as usize {
                    assert!(
                        !green_at(px[y * stride + x]),
                        "badge pixel at ({x},{y}) {dpi}"
                    );
                }
            }
            let m = resolve_media_layout_in(&dims, c.shape(), NottSpace::Music).unwrap();
            assert!(
                lit_in(&px, stride, m.artwork_bounds.unwrap(), 80),
                "cover drawn at {dpi}"
            );
        }
    }

    #[test]
    fn test_home_divider_drawn_and_long_title_stops_before_it() {
        let renderer = Renderer::new().expect("renderer");
        for dpi in [96u32, 144] {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            let c = MediaContent {
                title: LONG_TITLE.into(),
                ..content("Song", Some(sample_artwork(64, 64)))
            };
            let m = resolve_media_layout(&dims, c.shape()).unwrap();
            let (px, stride) = render_media_frame(&renderer, &dims, &c);
            let d = m.divider_bounds;
            let (x, y) = (d.left as usize, ((d.top + d.bottom) / 2.0) as usize);
            let p = px[y * stride + x];
            let (b, g, r) = (p & 0xFF, (p >> 8) & 0xFF, (p >> 16) & 0xFF);
            assert!(
                b > 15 && b < 80 && b == g && g == r,
                "subtle grey divider at {dpi}: {b}"
            );
            // The long title ends (ellipsis) before the divider
            let past = RectF::new(
                m.title_bounds.right + 1.0,
                m.title_bounds.top,
                d.left - 1.0,
                m.title_bounds.bottom,
            );
            assert!(
                !lit_in(&px, stride, past, 40),
                "title clear of the divider at {dpi}"
            );
        }
    }

    #[test]
    fn test_space_icons_solid_white_and_distinct() {
        let renderer = Renderer::new().expect("renderer");
        for dpi in [96u32, 144] {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            let sel = resolve_space_selector_in(&dims, NottSpace::Home).unwrap();
            renderer.set_space(NottSpace::Home);
            let (px, stride) = render_selector_frame(&renderer, &dims);
            let brightest = |r: RectF| {
                (r.top as usize..r.bottom as usize)
                    .flat_map(|y| (r.left as usize..r.right as usize).map(move |x| (x, y)))
                    .map(|(x, y)| px[y * stride + x] & 0xFF)
                    .max()
                    .unwrap()
            };
            let (home, music) = (brightest(sel.home), brightest(sel.music));
            assert!(
                home > 240 && music > 240,
                "both icons white at {dpi}: {home}/{music}"
            );
            // Solid glyph: the house is filled, not just an outline
            let r = sel.home;
            let (x, y) = (
                ((r.left + r.right) / 2.0) as usize,
                ((r.top + r.bottom) / 2.0 + 2.0 * dims.scale) as usize,
            );
            assert!(px[y * stride + x] & 0xFF > 240, "house is filled at {dpi}");
            // Two different glyphs: compare their lit pixel masks
            let mask = |r: RectF| -> Vec<bool> {
                (r.top as usize..r.bottom as usize)
                    .flat_map(|y| (r.left as usize..r.right as usize).map(move |x| (x, y)))
                    .map(|(x, y)| (px[y * stride + x] & 0xFF) > 60)
                    .collect()
            };
            assert_ne!(mask(sel.home), mask(sel.music), "distinct icons at {dpi}");
        }
    }

    #[test]
    fn test_ui_font_resolution_and_fallback() {
        use windows::core::w;
        let renderer = Renderer::new().expect("renderer");
        let dw = &renderer.dwrite_factory;
        // First installed wins; nothing installed is reported, not guessed
        assert_eq!(
            first_installed(dw, &[w!("No Such Font 7f3a"), w!("Bahnschrift")]),
            Some(1)
        );
        assert_eq!(first_installed(dw, &[w!("No Such Font 7f3a")]), None);
        // The chosen face is a candidate that exists here, or the named
        // fallback; Bahnschrift ships with Windows, so it is never needed
        let (name, family) = ui_font(dw);
        let candidates: Vec<&str> = UI_FONT_CANDIDATES.iter().map(|(n, _)| *n).collect();
        assert!(candidates.contains(&name), "{name}");
        assert_eq!(
            first_installed(dw, &[family]),
            Some(0),
            "{name} is installed"
        );
        assert_eq!(ui_font(dw), (name, family), "resolved once, stable");
    }

    #[test]
    fn test_meridiem_range_across_locales() {
        let r = |s: &str| meridiem_range(s);
        assert_eq!(r("12:47 AM"), Some((5, 3)), "space + AM");
        assert_eq!(r("9:05 p.m."), Some((4, 5)));
        assert_eq!(r("AM 12:47"), Some((0, 3)), "leading, with its space");
        assert_eq!(r("\u{4e0b}\u{5348} 5:49"), Some((0, 3)), "CJK day period");
        assert_eq!(r("오후 5:49"), Some((0, 3)));
        assert_eq!(r("17:42"), None, "24-hour");
        assert_eq!(r("17.42"), None);
        assert_eq!(r(""), None);
        assert_eq!(r("AM"), None, "no numerals");
    }

    #[test]
    fn test_text_resources_are_shared_and_rendering_is_greyscale() {
        use windows::Win32::Graphics::Direct2D::D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE;
        use windows::core::Interface;
        let renderer = Renderer::new().expect("renderer");
        for dpi in [96u32, 120, 137, 144, 168, 192, 288] {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            // One format set per DPI, reused frame after frame
            let a = renderer.media_formats(&dims).unwrap().title.as_raw();
            let b = renderer.media_formats(&dims).unwrap().title.as_raw();
            assert_eq!(a, b, "formats reused at {dpi}");
            let f = renderer.media_formats(&dims).unwrap();
            let size = unsafe { f.title.GetFontSize() };
            assert!((size - TextRole::MediaTitle.style().size * dims.scale).abs() < 1e-3);
            assert!(
                unsafe { f.source.GetFontSize() } >= 10.0,
                "small text >= 10 px at {dpi}"
            );
        }
        // The layered surface draws greyscale text with the custom params
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 120);
        renderer.ensure_buffer(dims.width, dims.height).unwrap();
        let rt = renderer.cached_rt.borrow();
        let rt = rt.as_ref().unwrap();
        unsafe {
            assert_eq!(
                rt.GetTextAntialiasMode(),
                D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE
            );
            let params = rt.GetTextRenderingParams().unwrap();
            assert_eq!(params.as_raw(), renderer.text_params.as_raw());
            let p2: windows::Win32::Graphics::DirectWrite::IDWriteRenderingParams2 =
                renderer.text_params.cast().unwrap();
            assert_eq!(
                p2.GetGridFitMode(),
                windows::Win32::Graphics::DirectWrite::DWRITE_GRID_FIT_MODE_DISABLED
            );
            assert_eq!(p2.GetClearTypeLevel(), 0.0);
        }
    }

    #[test]
    fn test_clock_meridiem_is_smaller_than_the_numerals() {
        let renderer = Renderer::new().expect("renderer");
        for dpi in [96u32, 120, 144, 192] {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            let ResolvedLayout::Expanded { components, .. } = resolve_layout(&dims) else {
                unreachable!()
            };
            let clock = ClockDateState::from_pure_components(2026, 10, 6, 1, 0, 47, 0, 0, false);
            renderer.set_media(None);
            let (px, stride) = render_custom_frame(&renderer, &dims, |rt| {
                renderer
                    .render_expanded_content(
                        rt,
                        &components,
                        &dims,
                        &clock.formatted_time,
                        &clock.formatted_date,
                    )
                    .unwrap()
            });
            // Lit columns inside the time box, grouped into glyph runs
            let t = components.time_bounds;
            let column = |x: usize| -> Option<(usize, usize)> {
                let ys: Vec<usize> = (t.top as usize..t.bottom as usize)
                    .filter(|&y| (px[y * stride + x] & 0xFF) > 90)
                    .collect();
                Some((*ys.first()?, *ys.last()?))
            };
            let cols: Vec<(usize, Option<(usize, usize)>)> = (t.left as usize..t.right as usize)
                .map(|x| (x, column(x)))
                .collect();
            let lit: Vec<&(usize, Option<(usize, usize)>)> =
                cols.iter().filter(|(_, c)| c.is_some()).collect();
            let (first_x, last_x) = (lit.first().unwrap().0, lit.last().unwrap().0);
            let height = |x0: usize, x1: usize| {
                let spans: Vec<(usize, usize)> = cols
                    .iter()
                    .filter(|(x, c)| *x >= x0 && *x <= x1 && c.is_some())
                    .map(|(_, c)| c.unwrap())
                    .collect();
                spans.iter().map(|s| s.1).max().unwrap() - spans.iter().map(|s| s.0).min().unwrap()
            };
            let w = last_x - first_x;
            // Numerals at the left, the meridiem in the last fifth
            let digits = height(first_x, first_x + w / 5);
            let meridiem = height(last_x - w / 5, last_x);
            assert!(
                (meridiem as f32) < digits as f32 * 0.7,
                "AM {meridiem} px vs digits {digits} px at {dpi}"
            );
            // Same baseline: the marker's bottom sits on the numerals' bottom
            let bottom = |x0: usize, x1: usize| {
                cols.iter()
                    .filter(|(x, c)| *x >= x0 && *x <= x1 && c.is_some())
                    .map(|(_, c)| c.unwrap().1)
                    .max()
                    .unwrap()
            };
            let (db, mb) = (
                bottom(first_x, first_x + w / 5),
                bottom(last_x - w / 5, last_x),
            );
            assert!(
                db.abs_diff(mb) <= 1 + dpi as usize / 96,
                "baseline at {dpi}"
            );
        }
    }

    #[test]
    fn test_switch_anim_endpoints_reversal_and_settling() {
        let mut a = SwitchAnim::settled(false);
        assert_eq!(a.level(), 0.0);
        assert!(!a.is_animating(), "settled: no work");
        a.set(true, false);
        assert!(a.is_animating());
        // Monotonic ease to exactly ON within the travel time
        let mut prev = a.level();
        let mut t = 0.0;
        while a.step(8.0) {
            assert!(a.level() >= prev - 1e-6);
            prev = a.level();
            t += 8.0;
            assert!(t <= crate::config::SWITCH_TRAVEL_MS + 8.0);
        }
        assert_eq!(a.level(), 1.0);
        assert!((140.0..=200.0).contains(&crate::config::SWITCH_TRAVEL_MS));
        // Reverse mid-way: continues from where it is (no jump), back to OFF
        // in less than a full travel
        a.set(false, false);
        a.step(60.0);
        let mid = a.level();
        a.set(true, false);
        assert!((a.level() - mid).abs() < 1e-6, "no jump on retarget");
        let mut steps = 0;
        while a.step(8.0) {
            steps += 1;
        }
        assert_eq!(a.level(), 1.0);
        assert!(
            (steps as f32) * 8.0 < crate::config::SWITCH_TRAVEL_MS,
            "shorter: partial travel"
        );
        // Same target again: nothing restarts; instant snaps
        a.set(true, false);
        assert!(!a.is_animating());
        a.set(false, true);
        assert_eq!((a.level(), a.is_animating()), (0.0, false));
    }

    #[test]
    fn test_notch_page_switches_slide_on_toggle_and_snap_otherwise() {
        let renderer = Renderer::new().expect("renderer");
        let on = NottSettings::default();
        let off = NottSettings {
            always_on_top: false,
            ..on
        };
        // Page hidden: values just show
        renderer.set_settings(false, off);
        assert!(!renderer.switches_animating());
        assert_eq!(renderer.switches.borrow()[0].level(), 0.0);
        // Opening shows current values, no slide
        renderer.set_settings(true, on);
        assert!(!renderer.switches_animating());
        assert_eq!(renderer.switches.borrow()[0].level(), 1.0);
        // Flipped while showing: slides (unless the system disables motion)
        renderer.set_settings(true, off);
        if crate::window::is_client_animation_enabled() {
            assert!(renderer.switches_animating());
            while renderer.step_switches(16.0) {}
        }
        assert_eq!(renderer.switches.borrow()[0].level(), 0.0);
        assert!(
            !renderer.switches_animating(),
            "settled: the timer can stop"
        );
        // Under Reduced motion the flip is immediate
        let reduced = NottSettings {
            reduced_motion: true,
            ..off
        };
        renderer.set_settings(true, reduced);
        assert!(!renderer.switches_animating());
        assert_eq!(renderer.switches.borrow()[1].level(), 1.0);
    }
}
