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
    DWriteCreateFactory, IDWriteFactory, IDWriteFactory5, IDWriteFontCollection, IDWriteTextFormat,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_FEATURE, DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES, DWRITE_TEXT_METRICS,
    IDWriteTypography,
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
    BASE_DROP_ICON_SIZE, BASE_DROP_MESSAGE_GAP, BASE_DROP_MESSAGE_LINE, BASE_DROP_OUTLINE_INSET,
    BASE_DROP_OUTLINE_RADIUS, BASE_DROP_OUTLINE_WIDTH, BASE_DROP_RESULT_ICON_SIZE, COLOR_DROP_ICON,
    COLOR_DROP_OUTLINE, DROP_OUTLINE_DASHES,
};
use crate::config::{
    BASE_MEDIA_ARTWORK_RADIUS_EXTRA, BASE_MEDIA_BADGE_OVERHANG, BASE_MEDIA_BADGE_RING,
    BASE_MEDIA_BADGE_SIZE, BASE_MEDIA_ICON_SIZE, BASE_MEDIA_PLAY_ICON_SIZE,
    BASE_MEDIA_TIMELINE_LABEL_GAP, BASE_MEDIA_TIMELINE_LABEL_WIDTH, BASE_MEDIA_TIMELINE_MIN_TRACK,
    BASE_MEDIA_TIMELINE_TRACK, BASE_MUSIC_ARTWORK_RADIUS, BASE_MUSIC_ICON_SIZE,
    BASE_MUSIC_PLAY_ICON_SIZE, BASE_SPACE_ICON_SIZE, COLOR_ARTWORK_HAIRLINE, COLOR_BORDER_HOVER,
    COLOR_MEDIA_DIVIDER, COLOR_MEDIA_TRACK_UNPLAYED, COLOR_SPACE_PILL_SELECTED, COLOR_TEXT_PRIMARY,
    COLOR_TEXT_SECONDARY, COLOR_TEXT_TERTIARY, COLOR_TIMER, COLOR_TRANSPARENT, CornerProfile,
    FONT_FAMILY_FALLBACK, MEDIA_ICON_REST_OPACITY, MEDIA_PRESS_ICON_SCALE, NOTCH_BG_COLOR,
    NOTCH_BORDER_COLOR, NotchDimensions, SKIP_GLYPH_HALF_WIDTH,
};
use crate::config::{CLIPBOARD_PREVIEW_CHARS, COLOR_CLIPBOARD_ROW};
use crate::config::{TextRole, UI_FONT_CANDIDATES};
#[cfg(test)]
use crate::layout::resolve_media_layout;
use crate::layout::{
    CollapsedLayout, ExpandedLayout, MediaLayout, RectF, ResolvedLayout, resolve_collapsed_layout,
    resolve_layout, resolve_media_layout_in, visualizer_metrics,
};
use crate::layout::{
    PanelHit, blended_selector, resolve_clipboard_layout, resolve_settings_layout, space_dimensions,
};
use crate::media::{
    Artwork, ControlFeedback, IconMotion, MediaContent, MediaControl, PlayPauseIcon, Visualizer,
    format_clock, now_filetime, shows_visualizer,
};
use crate::space::OecaSpace;
use crate::space::{OecaSettings, PANEL_SPACE, Scene};
use crate::timer::{Countdown, Phase};
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
    space: Cell<OecaSpace>,
    /// An image drag is over the notch: the expanded notch shows the drop page.
    /// What the drop page shows (`DropPage::Off`: no drop page), and the
    /// last drop's outcome (kept while its result fades out).
    drop: Cell<crate::dragdrop::DropPage>,
    drop_outcome: Cell<crate::dragdrop::DropOutcome>,
    /// The running animation's content blend and opacity (None when settled).
    transition: Cell<Option<Transition>>,
    /// Settings page open / settings values (mirror `WindowState`, which sets
    /// them on every change).
    settings_open: Cell<bool>,
    settings: Cell<OecaSettings>,
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
    thumbs: RefCell<Vec<Thumb>>,
    /// Transport glyph motion (play/pause morph, skip slides).
    icons: RefCell<IconMotion>,
    /// The visualizer's artwork palette, worked out once per artwork (keyed
    /// by it), the blend of its bar colours toward a new palette, and its
    /// two brushes (a recoloured solid fill and a fixed shading gradient),
    /// made once per render target.
    viz_palette: RefCell<Option<(Artwork, Option<crate::media::VizGradient>)>>,
    viz_blend: Cell<VizBlend>,
    /// The visualizer's gradient ramp and round-end masks, and the bar
    /// width (device pixels) the masks were made for.
    viz_brushes: RefCell<Option<(ID2D1Bitmap, ID2D1Bitmap, u32)>>,
    /// The gradient ends and opacity last baked into the ramp bitmap.
    viz_baked: Cell<Option<([D2D1_COLOR_F; 2], f32)>>,
    /// The artwork's turn to a new track's (None when not turning).
    flip: RefCell<Option<ArtFlip>>,
    /// The collapsed notch's album art (covers fading in / out), and its
    /// rounded thumbnails' device bitmaps (tied to `cached_rt`, pruned to
    /// the covers on screen).
    collapsed_art: RefCell<CollapsedArt>,
    collapsed_thumbs: RefCell<Vec<Thumb>>,
    /// The scrubber while dragged (0..1 of the track), and a committed seek
    /// shown until the session reports its new position (or briefly).
    scrub: Cell<Option<f32>>,
    seek_hold: Cell<Option<(i64, std::time::Instant)>>,
    /// The countdown timer as shown (mirrors `WindowState`, which sets it on
    /// every change; the numerals are read from it at draw time).
    timer: Cell<TimerView>,
    /// Tabular figures: the timer's digits keep their width as they count.
    tabular: IDWriteTypography,
}

/// The artwork flipping to a new track's: one flat photograph turning 180
/// degrees about the vertical line through its centre, the old cover on its
/// front and the new one on its back. One progress value (the angle, on a
/// spring) gives everything: the projected width |cos|, which side faces the
/// viewer, and that side's cover (always upright).
struct ArtFlip {
    /// The front and back covers.
    faces: [Artwork; 2],
    /// The turn (radians): 0 = front facing, pi = back facing.
    angle: crate::config::Glide,
}

impl ArtFlip {
    /// Old cover facing the viewer, half a turn to go.
    fn new(from: Artwork, to: Artwork) -> Self {
        let mut angle = crate::config::Glide::at(0.0);
        angle.target = std::f32::consts::PI;
        Self {
            faces: [from, to],
            angle,
        }
    }

    /// The side facing the viewer at `angle` (0 front, 1 back): it changes
    /// only when the photo is edge-on (90 degrees, 270, ...).
    fn face_at(angle: f32) -> usize {
        ((angle / std::f32::consts::PI).round() as i64).rem_euclid(2) as usize
    }

    /// The cover facing the viewer now.
    fn shown(&self) -> &Artwork {
        &self.faces[Self::face_at(self.angle.value)]
    }

    /// The photo's apparent width as a fraction of its own: |cos| of the
    /// turn, never quite 0 (edge-on it is a hairline, never gone).
    fn width_fraction(&self, full_px: f32) -> f32 {
        self.angle.value.cos().abs().max(1.0 / full_px.max(1.0))
    }

    /// A newer cover mid-flip, without restarting or snapping: before the
    /// photo is edge-on it simply goes on the back (the side still to come);
    /// once the back is already showing, the photo carries on turning
    /// another half-turn, with its velocity, and the newest goes on the
    /// side that comes round next.
    fn retarget(&mut self, cover: Artwork) {
        let mut end = Self::face_at(self.angle.target);
        if Self::face_at(self.angle.value) == end {
            self.angle.target += std::f32::consts::PI;
            end ^= 1;
        }
        self.faces[end] = cover;
    }
}

/// The collapsed notch's album art: the covers on screen, oldest first, each
/// with its opacity on `SPACE_SPRING` (the content blend's spring). The
/// current track's cover heads to 1, every other to 0, so appearing,
/// disappearing and a track change (a cross-fade) are one motion that
/// retargets from where it is, with its velocity; a cover is dropped once
/// it has faded out. The slot itself never moves.
#[derive(Default)]
struct CollapsedArt {
    layers: Vec<(Artwork, crate::config::Glide)>,
}

/// At most this many covers mid-fade (rapid skips); beyond it the faintest
/// fading-out one goes.
const COLLAPSED_ART_LAYERS: usize = 3;

impl CollapsedArt {
    /// Heads for `art` (None: no cover). Snaps when `!motion`.
    fn retarget(&mut self, art: Option<&Artwork>, motion: bool) {
        if let Some(a) = art
            && !self.layers.iter().any(|(b, _)| b == a)
        {
            self.layers.push((a.clone(), crate::config::Glide::at(0.0)));
        }
        for (b, g) in &mut self.layers {
            g.target = if Some(&*b) == art { 1.0 } else { 0.0 };
        }
        while self.layers.len() > COLLAPSED_ART_LAYERS {
            let faintest = (0..self.layers.len())
                .filter(|&i| self.layers[i].1.target == 0.0)
                .min_by(|&i, &j| self.layers[i].1.value.total_cmp(&self.layers[j].1.value));
            match faintest {
                Some(i) => drop(self.layers.remove(i)),
                None => break,
            }
        }
        if !motion {
            self.settle();
        }
    }

    /// Jumps every cover to its end state (Reduced motion, not on screen).
    fn settle(&mut self) {
        for (_, g) in &mut self.layers {
            g.snap(g.target);
        }
        self.layers.retain(|(_, g)| g.value > 0.0);
    }

    fn is_animating(&self) -> bool {
        self.layers.iter().any(|(_, g)| g.is_animating())
    }

    /// Advances the fades; true while any cover still moves.
    fn step(&mut self, dt_ms: f32) -> bool {
        for (_, g) in &mut self.layers {
            g.step(crate::config::SPACE_SPRING, dt_ms);
        }
        self.layers
            .retain(|(_, g)| g.is_animating() || g.value > 0.0);
        self.is_animating()
    }
}

/// The flip: critically damped and quick (visually done in about 300 ms):
/// a smooth start and a crisp landing with no overshoot.
const ART_FLIP_SPRING: crate::config::Spring = crate::config::Spring {
    zeta: 1.0,
    response: 0.28,
};
/// A committed seek is shown at most this long while the session catches up.
const SEEK_HOLD_MS: u128 = 1500;

type Quad = [(f32, f32); 4];

/// `pts` inset by `r` (a convex outline, either winding; repeated vertices
/// allowed). Stroking the result `2 r` wide with round joins gives back the
/// outline with every corner rounded by `r`. Edges too short to survive the
/// inset (they would turn inside out) are dropped, so their corners merge
/// into one, exactly like the rounded outline: the result changes
/// continuously as the outline does. Each output vertex is its input
/// vertex's inset (merged corners share a point), so indices keep meaning.
fn inset_quad(pts: Quad, r: f32) -> Quad {
    const N: usize = 4;
    let area: f32 = (0..N)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % N]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum();
    let sign = if area >= 0.0 { 1.0 } else { -1.0 };
    let dir = |k: usize| {
        let (a, b) = (pts[k], pts[(k + 1) % N]);
        let l = (b.0 - a.0).hypot(b.1 - a.1);
        (l, ((b.0 - a.0) / l.max(1e-6), (b.1 - a.1) / l.max(1e-6)))
    };
    // Edge k runs from vertex k to k + 1; offset inward by r
    let offset = |k: usize| {
        let (_, d) = dir(k);
        let nrm = (-d.1 * sign, d.0 * sign);
        let a = pts[k];
        ((a.0 + nrm.0 * r, a.1 + nrm.1 * r), d, nrm)
    };
    let mut dropped: [bool; N] = std::array::from_fn(|k| dir(k).0 < 1e-4);
    let mut out = pts;
    for _ in 0..N {
        if dropped.iter().filter(|d| !**d).count() < 2 {
            break;
        }
        // Each vertex: where the nearest kept edges into and out of it meet
        out = std::array::from_fn(|i| {
            let mut into = (i + N - 1) % N;
            while dropped[into] {
                into = (into + N - 1) % N;
            }
            let mut out_k = i;
            while dropped[out_k] {
                out_k = (out_k + 1) % N;
            }
            let (p1, d1, n1) = offset(into);
            let (p2, d2, _) = offset(out_k);
            let cross = d1.0 * d2.1 - d1.1 * d2.0;
            if cross.abs() < 1e-5 {
                return (pts[i].0 + n1.0 * r, pts[i].1 + n1.1 * r);
            }
            let t = ((p2.0 - p1.0) * d2.1 - (p2.1 - p1.1) * d2.0) / cross;
            (p1.0 + d1.0 * t, p1.1 + d1.1 * t)
        });
        // A kept edge whose inset turned inside out is eaten by the rounding
        let flipped = (0..N).find(|&k| {
            let (a, b) = (out[k], out[(k + 1) % N]);
            let d = dir(k).1;
            !dropped[k] && (b.0 - a.0) * d.0 + (b.1 - a.1) * d.1 < -1e-5
        });
        match flipped {
            Some(k) => dropped[k] = true,
            None => break,
        }
    }
    out
}

/// The play/pause glyph at morph `m` (0 = play, 1 = pause), as convex
/// outlines already inset by the returned radius: fill them and stroke them
/// that radius x 2 wide with round joins. The play triangle is split down
/// its middle into two halves that become the two pause bars (each vertex
/// travels straight to its partner), the corner rounding blends from the
/// triangle's to the bars' fully rounded ends, and while the halves still
/// touch a bridge keeps the seam closed. At 0 and 1 this is exactly the rest
/// glyph (`icon_shapes`' geometry, rounded).
fn play_pause_outline(cx: f32, cy: f32, u: f32, m: f32, snap: bool) -> (Vec<Quad>, f32) {
    let m = m.clamp(0.0, 1.0);
    let play = icon_shapes(
        MediaControl::PlayPause,
        PlayPauseIcon::Play,
        cx,
        cy,
        u,
        snap,
    );
    let pause = icon_shapes(
        MediaControl::PlayPause,
        PlayPauseIcon::Pause,
        cx,
        cy,
        u,
        snap,
    );
    let (IconShape::Triangle(tri), [IconShape::Bar(b1), IconShape::Bar(b2)]) =
        (play[0], [pause[0], pause[1]])
    else {
        return (Vec::new(), 0.0);
    };
    let (l, t) = tri[0];
    let (w, h) = (tri[1].0 - l, tri[2].1 - t);
    let mid = l + w / 2.0;
    let left_half: Quad = [
        (l, t),
        (mid, t + h / 4.0),
        (mid, t + 3.0 * h / 4.0),
        (l, t + h),
    ];
    let right_half: Quad = [
        (mid, t + h / 4.0),
        (l + w, t + h / 2.0),
        (l + w, t + h / 2.0),
        (mid, t + 3.0 * h / 4.0),
    ];
    let bar = |b: RectF| -> Quad {
        [
            (b.left, b.top),
            (b.right, b.top),
            (b.right, b.bottom),
            (b.left, b.bottom),
        ]
    };
    // Corner rounding: the triangle's (as `rounded_triangle_corners`), and
    // the bars' fully round ends
    let (a, _) = rounded_triangle_corners(tri);
    let cut = (a[0].0 - tri[0].0).hypot(a[0].1 - tri[0].1);
    let r_play = cut * (std::f32::consts::PI / 6.0).tan();
    let r = r_play + (b1.width() / 2.0 - r_play) * m;
    let lerp =
        |p: (f32, f32), q: (f32, f32), k: f32| (p.0 + (q.0 - p.0) * k, p.1 + (q.1 - p.1) * k);
    let mix = |x: Quad, y: Quad| -> Quad { std::array::from_fn(|i| lerp(x[i], y[i], m)) };
    let q1 = inset_quad(mix(left_half, bar(b1)), r);
    let q2 = inset_quad(mix(right_half, bar(b2)), r);
    let mut shapes = vec![q1, q2];
    let k = 1.0 - smoothstep(0.0, 0.35, m);
    if k > 0.0 {
        shapes.push([q1[1], lerp(q1[1], q2[0], k), lerp(q1[2], q2[3], k), q1[2]]);
    }
    (shapes, r)
}

/// A skip glyph (two touching triangles) part-way through one step of its
/// roll, `phase` 0..1 (0 and 1 are both exactly the rest glyph): the lead
/// triangle shrinks into its own tip, the rear one slides
/// forward into the lead's place, and a new rear triangle grows out of
/// nothing behind it. One continuous morph: nothing fades or lingers.
/// Previous is the mirror image of Next.
fn skip_morph(control: MediaControl, cx: f32, cy: f32, u: f32, phase: f32) -> Vec<[(f32, f32); 3]> {
    // Linear in the phase: the spring driving it already eases, and a burst
    // of clicks then rolls on without slowing at every step
    let e = phase.clamp(0.0, 1.0);
    // The rest glyph's own whole-pixel geometry (`icon_shapes`, snapped), so
    // the roll starts and ends exactly on it: no pop at either end
    let h = (u * SKIP_GLYPH_HEIGHT).round().max(2.0);
    let tw = (u * SKIP_GLYPH_HALF_WIDTH).round().max(2.0);
    let left = (cx - tw).round();
    let cy = (cy - h / 2.0).round() + h / 2.0;
    // (base x, width, scale) of each triangle, pointing right
    let tri = |base: f32, w: f32, k: f32| {
        [
            (base, cy - h * k / 2.0),
            (base + w, cy),
            (base, cy + h * k / 2.0),
        ]
    };
    let shapes = [
        // The new rear triangle, growing from its base
        tri(left, tw * e, e),
        // The rear one moving up into the lead's place
        tri(left + tw * e, tw, 1.0),
        // The lead one, pushed by it, shrinking into its own (fixed) tip:
        // the three always touch, one continuous shape
        tri(left + tw + tw * e, tw * (1.0 - e), 1.0 - e),
    ];
    let mirror = control == MediaControl::Previous;
    shapes
        .into_iter()
        .filter(|t| (t[1].0 - t[0].0).abs() > 0.25)
        .map(|t| {
            if mirror {
                t.map(|(x, y)| (2.0 * cx - x, y))
            } else {
                t
            }
        })
        .collect()
}

/// `p` scaled by `k` about (cx, cy) (a pressed glyph's compression).
fn squeeze(p: (f32, f32), cx: f32, cy: f32, k: f32) -> (f32, f32) {
    (cx + (p.0 - cx) * k, cy + (p.1 - cy) * k)
}

/// Glyph primitives scaled by `k` about (cx, cy).
fn squeeze_shapes(shapes: Vec<IconShape>, cx: f32, cy: f32, k: f32) -> Vec<IconShape> {
    shapes
        .into_iter()
        .map(|s| match s {
            IconShape::Bar(r) => {
                let (l, t) = squeeze((r.left, r.top), cx, cy, k);
                let (rr, b) = squeeze((r.right, r.bottom), cx, cy, k);
                IconShape::Bar(RectF::new(l, t, rr, b))
            }
            IconShape::Triangle(pts) => IconShape::Triangle(pts.map(|p| squeeze(p, cx, cy, k))),
        })
        .collect()
}

/// The visualizer gradient's two ends (top, bottom), blending from one
/// artwork's gradient to the next over `VIZ_BLEND_MS` (worked out from
/// elapsed time when drawn, so it needs no frames of its own).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct VizBlend {
    from: [D2D1_COLOR_F; 2],
    to: [D2D1_COLOR_F; 2],
    since: Option<std::time::Instant>,
    /// False until the first colours are known (they appear at once).
    ready: bool,
}

/// How long the bars take to blend to a new artwork's gradient.
const VIZ_BLEND_MS: f32 = 450.0;
/// The bar group's silhouette: each bar's share of the full height, so the
/// group reads as a balanced little waveform (tallest just left of centre).
const VIZ_ENVELOPE: [f32; 4] = [0.72, 1.0, 0.88, 0.64];
/// How much darker each bar is at its very ends than at its middle (easing
/// in from the middle, so the middle reads brightest and richest).
const VIZ_END_DARKEN: f32 = 0.26;
/// The one gradient baked into a tiny bitmap (1 x this), stretched along
/// each bar: the software target's gradient brushes cost ~90 us per bar,
/// this a texture lookup.
const VIZ_RAMP_STEPS: u32 = 64;

/// Transparent rows between the two end masks (so smooth sampling at one
/// mask's edge never picks up the other).
const VIZ_CAP_GAP: u32 = 2;

/// The round-end masks for `bar`-wide bars (premultiplied black, its alpha
/// where a bar's rectangle lies outside its round ends): the top end in
/// rows 0..ceil(r), then `VIZ_CAP_GAP` clear rows, then the bottom end, r =
/// bar / 2. Drawn over a bar's top / bottom r it leaves the end a smooth
/// half disc on the notch's black. Returns the pixels and the row count.
fn viz_cap_mask(bar: u32) -> (Vec<u8>, u32) {
    const SUB: u32 = 8;
    let r = bar as f32 / 2.0;
    let rows = r.ceil() as u32;
    let below = rows + VIZ_CAP_GAP;
    let height = below + rows;
    let mut px = vec![0u8; (bar * height * 4) as usize];
    for y in (0..rows).chain(below..height) {
        // The disc's centre row in this mask, and which side is the cap
        let (centre, top) = if y < rows {
            (r, true)
        } else {
            (below as f32, false)
        };
        for x in 0..bar {
            let mut outside = 0;
            for (sx, sy) in (0..SUB * SUB).map(|k| (k % SUB, k / SUB)) {
                let px_ = x as f32 + (sx as f32 + 0.5) / SUB as f32;
                let py = y as f32 + (sy as f32 + 0.5) / SUB as f32;
                let in_cap = if top { py < centre } else { py > centre };
                let (dx, dy) = (px_ - r, py - centre);
                if in_cap && dx * dx + dy * dy > r * r {
                    outside += 1;
                }
            }
            let i = ((y * bar + x) * 4) as usize;
            px[i + 3] = (outside * 255 / (SUB * SUB)) as u8;
        }
    }
    (px, height)
}

/// The gradient's colour at `y` (0 top end .. 1 bottom end of any bar):
/// `ends` mixed along the hue arc, a touch deeper toward both ends, faded by
/// `opacity` into the notch's black (opaque: it is drawn on that black).
fn viz_ramp_colour(ends: [D2D1_COLOR_F; 2], y: f32, opacity: f32) -> D2D1_COLOR_F {
    let y = y.clamp(0.0, 1.0);
    let [a, b] = ends.map(|c| [c.r, c.g, c.b]);
    let d = 2.0 * y - 1.0;
    let k = (1.0 - VIZ_END_DARKEN * d * d) * opacity.clamp(0.0, 1.0);
    let [r, g, b] = crate::media::gradient_mix(a, b, y).map(|v| (v * k).clamp(0.0, 1.0));
    D2D1_COLOR_F { r, g, b, a: 1.0 }
}

impl VizBlend {
    fn current(&self, now: std::time::Instant) -> [D2D1_COLOR_F; 2] {
        let Some(since) = self.since else {
            return self.to;
        };
        let t = (now.saturating_duration_since(since).as_secs_f32() * 1000.0 / VIZ_BLEND_MS)
            .clamp(0.0, 1.0);
        if t >= 1.0 {
            return self.to; // exactly the new gradient once there
        }
        let e = t * t * (3.0 - 2.0 * t);
        std::array::from_fn(|i| {
            let (a, b) = (self.from[i], self.to[i]);
            D2D1_COLOR_F {
                r: a.r + (b.r - a.r) * e,
                g: a.g + (b.g - a.g) * e,
                b: a.b + (b.b - a.b) * e,
                a: a.a + (b.a - a.a) * e,
            }
        })
    }

    /// Heads for `target` from wherever the colours are now (at once the
    /// first time, or under Reduced motion).
    fn retarget(&mut self, target: [D2D1_COLOR_F; 2], now: std::time::Instant, instant: bool) {
        if self.ready && self.to == target {
            return;
        }
        self.from = if self.ready && !instant {
            self.current(now)
        } else {
            target
        };
        self.to = target;
        self.since = (self.ready && !instant).then_some(now);
        self.ready = true;
    }
}

/// What the renderer shows of the countdown timer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimerView {
    pub countdown: Countdown,
    /// What the collapsed presentations (slot, bubble) show: the countdown
    /// as last shown there, so one fading out after it ended keeps its own
    /// numerals (0:00) rather than the idle setup's.
    pub collapsed: Countdown,
    /// Presence (0..1) of the remaining time in the collapsed notch's
    /// visualizer slot.
    pub slot: f32,
    /// Opacity of finished numerals (the gentle done pulse; 1 otherwise).
    pub pulse: f32,
}

impl Default for TimerView {
    fn default() -> Self {
        Self {
            countdown: Countdown::default(),
            collapsed: Countdown::default(),
            slot: 0.0,
            pulse: 1.0,
        }
    }
}

/// A clipboard thumbnail's device bitmap, keyed by (image, pixel size).
type Thumb = (Artwork, (u32, u32), ID2D1Bitmap);

/// What the Clipboard space shows, UTF-16 encoded once per history change.
#[derive(Default)]
struct ClipboardView {
    /// The newest entries (at most `CLIPBOARD_TILES`).
    rows: Vec<ClipRow>,
}

enum ClipRow {
    /// One-line, bounded preview.
    Text(Vec<u16>),
    /// The entry's image (shared pixels) and its "W × H" caption.
    Image(Artwork, Vec<u16>),
}

/// Settings page text.
const SETTINGS_LABEL: &str = "Settings";
const ALWAYS_ON_TOP_TITLE: &str = "Always on top";
const ALWAYS_ON_TOP_DESCRIPTION: &str = "Keep Oeca above other windows";
const REDUCED_MOTION_TITLE: &str = "Reduced motion";
const REDUCED_MOTION_DESCRIPTION: &str = "Minimize Oeca's interface transitions";
const FULL_SETTINGS_TITLE: &str = "Open Full Settings";

/// Clipboard space empty state.
const CLIPBOARD_EMPTY_LABEL: &str = "Copied text and images appear here";
/// Clipboard clear confirmation.
const CLEAR_CONFIRM_MESSAGE: &str = "Clear clipboard history?";
const CLEAR_CONFIRM_NOTE: &str = "Only Oeca's history is removed. Your clipboard stays.";
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
    /// Countdown: Home numerals and status line (leading), and the collapsed
    /// slot (trailing, to the visualizer's edge).
    timer_display: IDWriteTextFormat,
    timer_status: IDWriteTextFormat,
    timer_slot: IDWriteTextFormat,
    /// A clipboard tile's text preview: wrapped, top-aligned, the last line
    /// that fits ends in an ellipsis.
    clip_text: IDWriteTextFormat,
}

/// The drop page's hint for files that are not images Oeca can add.
const DROP_UNSUPPORTED_LABEL: &str = "Only images can be added";

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
    resample_area_rect(src, px, px)
}

/// `resample_area` to `w` x `h`.
fn resample_area_rect(src: &Artwork, w: u32, h: u32) -> Artwork {
    let (sw, sh) = (src.width as usize, src.height as usize);
    let (dw, dh) = (w.max(1) as usize, h.max(1) as usize);
    if sw == dw && sh == dh {
        return src.clone();
    }
    let (fx, fy) = (sw as f32 / dw as f32, sh as f32 / dh as f32);
    let mut out = vec![0u8; dw * dh * 4];
    for oy in 0..dh {
        let (y0, y1) = (oy as f32 * fy, (oy + 1) as f32 * fy);
        for ox in 0..dw {
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
            let o = (oy * dw + ox) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / total.max(f32::EPSILON)).round() as u8;
            }
        }
    }
    Artwork::new(dw as u32, dh as u32, out).unwrap_or_else(|| src.clone())
}

/// General Sans (Indian Type Foundry, ITF Free Font License), embedded only
/// when its files are present at build time (`build.rs`): the licence
/// forbids publishing them in the repository. Regular, Medium and Semibold:
/// the weights `TextRole` uses.
#[cfg(oeca_bundled_font)]
pub(crate) const BUNDLED_FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/GeneralSans-Regular.otf"),
    include_bytes!("../assets/fonts/GeneralSans-Medium.otf"),
    include_bytes!("../assets/fonts/GeneralSans-Semibold.otf"),
];
#[cfg(not(oeca_bundled_font))]
pub(crate) const BUNDLED_FONTS: &[&[u8]] = &[];

/// Oeca's typeface: a family name and the collection it is found in (`None`:
/// the installed fonts). Every text format is made from it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UiFont {
    pub name: &'static str,
    pub family: windows::core::PCWSTR,
    pub collection: Option<IDWriteFontCollection>,
}

/// Oeca's typeface: the bundled General Sans when this build embeds it, else
/// the first `UI_FONT_CANDIDATES` family installed (system and per-user
/// fonts), else the documented `FONT_FAMILY_FALLBACK`. Resolved once per
/// thread (the UI thread in the app); shared by the notch and the Settings
/// window.
pub(crate) fn ui_font(dwrite: &IDWriteFactory) -> UiFont {
    thread_local! {
        static FONT: std::cell::OnceCell<UiFont> = const { std::cell::OnceCell::new() };
    }
    FONT.with(|f| {
        f.get_or_init(|| resolve_ui_font(dwrite, BUNDLED_FONTS))
            .clone()
    })
}

fn resolve_ui_font(dwrite: &IDWriteFactory, bundled: &[&'static [u8]]) -> UiFont {
    let (name, family) = UI_FONT_CANDIDATES[0];
    if let Some(collection) = bundled_collection(dwrite, bundled).filter(|c| has_family(c, family))
    {
        return UiFont {
            name,
            family,
            collection: Some(collection),
        };
    }
    let families = UI_FONT_CANDIDATES.map(|(_, f)| f);
    let (name, family) = match first_installed(dwrite, &families) {
        Some(i) => UI_FONT_CANDIDATES[i],
        None => (
            crate::config::FONT_FAMILY_FALLBACK_NAME,
            FONT_FAMILY_FALLBACK,
        ),
    };
    UiFont {
        name,
        family,
        collection: None,
    }
}

/// A private, in-memory font collection of `fonts` (OpenType file data),
/// visible only to this process: nothing is installed or registered with
/// Windows. `None` when there are no fonts, the data is unusable, or the
/// system predates `IDWriteFactory5` (Windows 10 1703); Oeca then uses the
/// installed fonts.
fn bundled_collection(
    dwrite: &IDWriteFactory,
    fonts: &[&'static [u8]],
) -> Option<IDWriteFontCollection> {
    if fonts.is_empty() {
        return None;
    }
    let factory: IDWriteFactory5 = windows::core::Interface::cast(dwrite).ok()?;
    unsafe {
        // Stays registered for the process's lifetime, as the collection's
        // font files refer to it
        let loader = factory.CreateInMemoryFontFileLoader().ok()?;
        factory.RegisterFontFileLoader(&loader).ok()?;
        let builder = factory.CreateFontSetBuilder().ok()?;
        for data in fonts {
            let file = loader
                .CreateInMemoryFontFileReference(
                    &factory,
                    data.as_ptr().cast(),
                    u32::try_from(data.len()).ok()?,
                    None::<&windows::core::IUnknown>,
                )
                .ok()?;
            builder.AddFontFile(&file).ok()?;
        }
        let set = builder.CreateFontSet().ok()?;
        let collection = factory.CreateFontCollectionFromFontSet(&set).ok()?;
        windows::core::Interface::cast(&collection).ok()
    }
}

fn has_family(collection: &IDWriteFontCollection, family: windows::core::PCWSTR) -> bool {
    let (mut index, mut exists) = (0u32, windows::core::BOOL(0));
    unsafe { collection.FindFamilyName(family, &mut index, &mut exists) }.is_ok()
        && exists.as_bool()
}

/// Index of the first of `families` installed, if any.
pub(crate) fn first_installed(
    dwrite: &IDWriteFactory,
    families: &[windows::core::PCWSTR],
) -> Option<usize> {
    let mut collection = None;
    unsafe { dwrite.GetSystemFontCollection(&mut collection, false) }.ok()?;
    let collection = collection?;
    families
        .iter()
        .position(|family| has_family(&collection, *family))
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

/// Fills an icon glyph (`IconSeg` path) in a `u`-sized box centred on (cx, cy);
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
    // Solid parts overlap and merge; holes run the other way (see `IconSeg`)
    unsafe { sink.SetFillMode(windows::Win32::Graphics::Direct2D::Common::D2D1_FILL_MODE_WINDING) };
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

/// The countdown's numerals (m:ss / h:mm:ss, the remaining time rounded up)
/// into `out`; returns the length used.
fn timer_numerals(countdown: &Countdown, out: &mut [u16; 12]) -> usize {
    let secs = countdown.display_secs(std::time::Instant::now());
    format_clock((secs * 1000) as i64, false, out)
}

/// Settled drop page dimensions at a DPI (the panel size).
fn drop_page_dimensions(dpi: u32) -> NotchDimensions {
    crate::layout::space_dimensions(crate::config::NotchState::Expanded, dpi, PANEL_SPACE)
}

/// Clipboard thumbnail: the centre of `src` cropped to the `w` x `h` aspect
/// (cover crop, never stretched), area-resampled to `w` x `h`, with corners
/// rounded to `radius` pixels by scaling the premultiplied pixels with their
/// coverage. Built from the entry's existing pixels: nothing is decoded.
fn thumbnail(src: &Artwork, w: u32, h: u32, radius: f32) -> Artwork {
    let (w, h) = (w.max(1), h.max(1));
    let crop = cover_source_rect(src.width, src.height, w as f32, h as f32);
    let (x0, y0) = (crop.left.round() as usize, crop.top.round() as usize);
    let cw = (crop.width().round() as usize).clamp(1, src.width as usize - x0);
    let ch = (crop.height().round() as usize).clamp(1, src.height as usize - y0);
    let stride = src.width as usize;
    let mut cropped = Vec::with_capacity(cw * ch * 4);
    for y in y0..y0 + ch {
        let start = (y * stride + x0) * 4;
        cropped.extend_from_slice(&src.pixels[start..start + cw * 4]);
    }
    let Some(cropped) = Artwork::new(cw as u32, ch as u32, cropped) else {
        return src.clone();
    };
    let thumb = resample_area_rect(&cropped, w, h);
    let (tw, th) = (thumb.width as f32, thumb.height as f32);
    let r = radius.clamp(0.0, tw.min(th) / 2.0);
    let mut pixels = thumb.pixels.to_vec();
    for (i, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (
            (i as u32 % thumb.width) as f32 + 0.5,
            (i as u32 / thumb.width) as f32 + 0.5,
        );
        let (cx, cy) = (x.clamp(r, tw - r), y.clamp(r, th - r));
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
        let motion = crate::window::motion_allowed(
            self.settings.get().reduced_motion,
            crate::window::is_client_animation_enabled(),
        );
        // A new cover replacing an old one turns into view (only a real
        // change: a refresh of the same artwork never animates). A cover
        // arriving mid-turn takes the incoming face's place.
        let old_art = media.as_ref().and_then(|m| m.content.artwork.clone());
        let new_art = content.and_then(|c| c.artwork.as_ref());
        match (&old_art, new_art) {
            (Some(a), Some(b)) if a != b && new_track && motion => {
                let mut flip = self.flip.borrow_mut();
                match flip.as_mut() {
                    Some(f) => f.retarget(b.clone()),
                    None => *flip = Some(ArtFlip::new(a.clone(), b.clone())),
                }
            }
            (_, None) => *self.flip.borrow_mut() = None,
            _ => {}
        }
        // The collapsed notch's cover follows the session's artwork
        self.collapsed_art.borrow_mut().retarget(new_art, motion);
        // No media: the next session's bars start from their own colours
        if content.is_none() {
            self.viz_blend.set(VizBlend::default());
        }
        // The visualizer's palette: worked out once per artwork
        {
            let art = content.and_then(|c| c.artwork.as_ref());
            let mut cached = self.viz_palette.borrow_mut();
            if cached.as_ref().map(|(a, _)| a) != art {
                *cached = art.map(|a| (a.clone(), crate::media::visualizer_gradient(a)));
            }
        }
        // The session's position replaces a held seek
        if !matches!((media.as_ref(), content), (Some(m), Some(c)) if m.content.timeline == c.timeline)
        {
            self.seek_hold.set(None);
        }
        // The glyph follows the reported playback state (at once when media
        // just appeared, or under Reduced motion)
        match content {
            Some(c) => self.icons.borrow_mut().sync(
                c.icon,
                std::time::Instant::now(),
                media.is_none() || !motion,
            ),
            None => {
                let mut icons = self.icons.borrow_mut();
                icons.sync(PlayPauseIcon::Play, std::time::Instant::now(), true);
                icons.settle();
            }
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
    pub fn set_space(&self, space: OecaSpace) -> bool {
        self.space.replace(space) != space
    }

    /// The animation frame's content blend / opacity (None: settled).
    pub fn set_transition(&self, transition: Option<Transition>) {
        self.transition.set(transition);
    }

    /// Mirrors the Settings page state and the settings values.
    pub fn set_settings(&self, open: bool, settings: OecaSettings) {
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

    /// Transport glyph motion (the window reports clicks into it).
    pub fn icons(&self) -> std::cell::RefMut<'_, IconMotion> {
        self.icons.borrow_mut()
    }

    /// Glyph motion or an artwork turn still needs frames.
    pub fn media_motion_animating(&self) -> bool {
        self.icons.borrow().is_animating()
            || self.flip.borrow().is_some()
            || self.collapsed_art.borrow().is_animating()
    }

    /// Advances glyph motion, the artwork turn and the collapsed album art's
    /// fades; true while any moves.
    pub fn step_media_motion(&self, dt_ms: f32) -> bool {
        let icons = self
            .icons
            .borrow_mut()
            .step(dt_ms, std::time::Instant::now());
        let collapsed = self.collapsed_art.borrow_mut().step(dt_ms);
        let mut flip = self.flip.borrow_mut();
        if let Some(f) = flip.as_mut()
            && !f.angle.step(ART_FLIP_SPRING, dt_ms)
        {
            *flip = None;
        }
        icons || collapsed || flip.is_some()
    }

    /// Ends the collapsed album art's fades at once (the collapsed notch is
    /// not on screen). Separate from `settle_media_motion`: the expanded
    /// media being off screen says nothing about the collapsed notch.
    pub fn settle_collapsed_art(&self) {
        self.collapsed_art.borrow_mut().settle();
    }

    /// Ends all media motion at once (the media is not on screen).
    pub fn settle_media_motion(&self) {
        self.icons.borrow_mut().settle();
        *self.flip.borrow_mut() = None;
    }

    /// The scrubber while dragged (fraction of the track), or None.
    pub fn set_scrub(&self, fraction: Option<f32>) {
        self.scrub.set(fraction);
    }

    /// Shows a committed seek at `position_ms` until the session reports
    /// its new position (or `SEEK_HOLD_MS` passes), so the bar never jumps
    /// back first.
    pub fn hold_seek(&self, position_ms: i64) {
        self.seek_hold
            .set(Some((position_ms, std::time::Instant::now())));
    }

    /// Mirrors the countdown timer and its collapsed slot (caller redraws).
    pub fn set_timer(&self, view: TimerView) {
        self.timer.set(view);
    }

    /// Shows / hides the Clipboard space's clear confirmation (caller redraws).
    pub fn set_clear_pending(&self, pending: bool) {
        self.clear_pending.set(pending);
    }

    /// What this frame shows: the transition, or the settled scene at full
    /// opacity.
    fn scene_view(&self) -> Transition {
        self.transition.get().unwrap_or_else(|| {
            let scene = Scene::current(self.drop.get(), self.settings_open.get(), self.space.get());
            Transition {
                from: scene,
                to: scene,
                mix: 1.0,
                alpha: 1.0,
            }
        })
    }

    /// Whether the drop page (or a drop's result) is showing (the window
    /// sizes the notch for it).
    pub fn drop_page(&self) -> bool {
        self.drop.get() != crate::dragdrop::DropPage::Off
    }

    pub fn drop_state(&self) -> crate::dragdrop::DropPage {
        self.drop.get()
    }

    /// Sets what the drop page shows; true if it changed (caller redraws).
    pub fn set_drop(&self, page: crate::dragdrop::DropPage) -> bool {
        self.drop.replace(page) != page
    }

    /// Shows a drop's outcome (`DropPage::Result`); true if anything changed.
    pub fn set_drop_result(&self, outcome: crate::dragdrop::DropOutcome) -> bool {
        let changed = self.drop_outcome.replace(outcome) != outcome;
        self.set_drop(crate::dragdrop::DropPage::Result) || changed
    }

    /// Rebuilds the Clipboard space rows from the history (call on every
    /// history change; the caller redraws if the space is visible). Thumbnails
    /// of entries no longer shown are released.
    pub fn set_clipboard(&self, history: &ClipboardHistory) {
        let rows: Vec<ClipRow> = history
            .items()
            .take(crate::config::CLIPBOARD_TILES)
            .map(|item| match item {
                ClipboardItem::Text(t) => ClipRow::Text(
                    crate::clipboard::preview(t, CLIPBOARD_PREVIEW_CHARS)
                        .encode_utf16()
                        .collect(),
                ),
                ClipboardItem::Image(a) => ClipRow::Image(
                    a.clone(),
                    format!("{} \u{00D7} {}", a.width, a.height)
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
        let tabular = unsafe {
            let t = dwrite_factory.CreateTypography()?;
            t.AddFontFeature(DWRITE_FONT_FEATURE {
                nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES,
                parameter: 1,
            })?;
            t
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
            space: Cell::new(OecaSpace::default()),
            clipboard: RefCell::new(ClipboardView::default()),
            round_stroke,
            thumbs: RefCell::new(Vec::new()),
            icons: RefCell::new(IconMotion::default()),
            viz_palette: RefCell::new(None),
            viz_blend: Cell::new(VizBlend::default()),
            viz_brushes: RefCell::new(None),
            viz_baked: Cell::new(None),
            flip: RefCell::new(None),
            collapsed_art: RefCell::new(CollapsedArt::default()),
            collapsed_thumbs: RefCell::new(Vec::new()),
            scrub: Cell::new(None),
            seek_hold: Cell::new(None),
            drop: Cell::new(crate::dragdrop::DropPage::Off),
            drop_outcome: Cell::new(crate::dragdrop::DropOutcome::Copied),
            transition: Cell::new(None),
            settings_open: Cell::new(false),
            settings: Cell::new(OecaSettings::default()),
            clear_pending: Cell::new(false),
            switches: RefCell::new({
                let s = OecaSettings::default();
                [
                    SwitchAnim::settled(s.always_on_top),
                    SwitchAnim::settled(s.reduced_motion),
                ]
            }),
            drop_stroke,
            text_params,
            timer: Cell::new(TimerView::default()),
            tabular,
        })
    }

    /// Releases cached GDI surface and Direct2D render target resources.
    fn cleanup_cached_resources(&self) {
        unsafe {
            // Device bitmaps belong to the render target being released
            *self.art_bitmap.borrow_mut() = None;
            *self.badge_bitmap.borrow_mut() = None;
            self.thumbs.borrow_mut().clear();
            self.collapsed_thumbs.borrow_mut().clear();
            *self.viz_brushes.borrow_mut() = None;
            self.viz_baked.set(None);

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

    /// A DirectWrite text format in Oeca's typeface (`ui_font`).
    fn create_text_format(
        &self,
        font_size: f32,
        weight: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT,
        style: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_STYLE,
    ) -> Result<IDWriteTextFormat> {
        self.create_text_format_in(&ui_font(&self.dwrite_factory), font_size, weight, style)
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
            &ui_font(&self.dwrite_factory),
            style.size * scale,
            DWRITE_FONT_WEIGHT(style.weight),
            align,
        )
    }

    /// Text format in `font` (its family, in its collection).
    fn create_text_format_in(
        &self,
        font: &UiFont,
        font_size: f32,
        weight: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT,
        style: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_STYLE,
    ) -> Result<IDWriteTextFormat> {
        let locale: Vec<u16> = "en-us\0".encode_utf16().collect();
        let format_result = unsafe {
            self.dwrite_factory.CreateTextFormat(
                font.family,
                font.collection.as_ref(),
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
        // The row's share of the weight, once per row; zero rows stay zero
        let row_weight = peak * vertical * bottom_fade;
        if row_weight == 0.0 {
            row.fill(0);
            continue;
        }
        for (x, v) in row.iter_mut().enumerate() {
            // Never drawn under the opaque body (`composite_shadow`)
            if alpha_at(x, y) == 255 {
                continue;
            }
            let side = x.min(width - 1 - x) as f32;
            // Away from the side edges the edge factor is exactly 1.0
            let weight = if side >= edge_fade {
                row_weight
            } else {
                row_weight * smoothstep(0.0, edge_fade, side)
            };
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
/// `(sum + w / 2) / w`, the box blurs' rounded average, as a multiply and a
/// shift: exact for every sum a window of `w` u8 values can reach (tested
/// for every odd `w` up to 121; blur windows are ~11-61 wide).
#[derive(Clone, Copy)]
struct RoundDiv {
    half: u32,
    inv: u64,
}

impl RoundDiv {
    fn new(w: u32) -> Self {
        Self {
            half: w / 2,
            inv: (1u64 << 32).div_ceil(u64::from(w)),
        }
    }

    #[inline]
    fn apply(self, sum: u32) -> u8 {
        ((u64::from(sum + self.half) * self.inv) >> 32) as u8
    }
}

#[inline]
fn box_blur_horizontal(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize) {
    let div = RoundDiv::new((radius * 2 + 1) as u32);

    for y in 0..height {
        let row_offset = y * width;
        // A row like the one above blurs like it (most of the notch body)
        if y > 0 && src[row_offset..row_offset + width] == src[row_offset - width..row_offset] {
            dst.copy_within(row_offset - width..row_offset, row_offset);
            continue;
        }
        let mut sum: u32 = 0;

        for x_rel in -(radius as isize)..=(radius as isize) {
            let clamped_x = x_rel.clamp(0, (width - 1) as isize) as usize;
            sum += src[row_offset + clamped_x] as u32;
        }
        dst[row_offset] = div.apply(sum);

        for x in 1..width {
            let add_x = (x + radius).min(width - 1);
            let sub_x = (x as isize - radius as isize - 1).max(0) as usize;
            sum += src[row_offset + add_x] as u32;
            sum -= src[row_offset + sub_x] as u32;
            dst[row_offset + x] = div.apply(sum);
        }
    }
}

/// The vertical pass, walked row by row (cache-friendly) with one running sum
/// per column: the same sums, in the same order, as a column-by-column walk.
#[inline]
fn box_blur_vertical(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize) {
    thread_local! {
        // Kept between frames: no allocation once it has grown to the width
        static SUMS: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    let div = RoundDiv::new((radius * 2 + 1) as u32);
    let row = |y: usize| &src[y * width..(y + 1) * width];

    SUMS.with_borrow_mut(|sums| {
        sums.clear();
        sums.resize(width, 0);
        for y_rel in -(radius as isize)..=(radius as isize) {
            let clamped_y = y_rel.clamp(0, (height - 1) as isize) as usize;
            for (s, &v) in sums.iter_mut().zip(row(clamped_y)) {
                *s += u32::from(v);
            }
        }
        for (d, &s) in dst[..width].iter_mut().zip(sums.iter()) {
            *d = div.apply(s);
        }

        for y in 1..height {
            let add = row((y + radius).min(height - 1));
            let sub = row((y as isize - radius as isize - 1).max(0) as usize);
            // The row entering the window equals the one leaving: the sums,
            // and so the output row, are unchanged
            if add == sub {
                dst.copy_within((y - 1) * width..y * width, y * width);
                continue;
            }
            let out = &mut dst[y * width..(y + 1) * width];
            for x in 0..width {
                sums[x] = sums[x] + u32::from(add[x]) - u32::from(sub[x]);
                out[x] = div.apply(sums[x]);
            }
        }
    });
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
        // A running countdown without playback takes the visualizer's place
        // (the bars give way while it is there)
        let timer = self.timer.get();
        let slot = timer.slot.clamp(0.0, 1.0);
        if slot > 0.0 {
            let rect = crate::layout::timer_slot_rect(layout, dimensions.scale);
            let formats = self.media_formats(dimensions)?;
            let mut buf = [0u16; 12];
            let n = timer_numerals(&timer.collapsed, &mut buf);
            let brush = unsafe { rt.CreateSolidColorBrush(&COLOR_TIMER, None)? };
            unsafe { brush.SetOpacity(slot * timer.pulse) };
            self.draw_timer_text(rt, &buf[..n], &formats.timer_slot, rect, &brush)?;
        }
        self.draw_visualizer(
            rt,
            layout.visualizer_bounds,
            accent,
            dimensions.scale,
            1.0 - slot,
        )?;
        self.draw_collapsed_art(rt, layout.artwork_bounds, dimensions.scale)?;
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
            for space in OecaSpace::ALL {
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

    /// Space icons (Oeca's own; see `HOUSE_BLANK` /
    /// `MUSIC_ALT`), filled in a `u`-sized box centred on (cx, cy).
    fn draw_space_icon(
        &self,
        rt: &ID2D1RenderTarget,
        space: OecaSpace,
        cx: f32,
        cy: f32,
        u: f32,
        brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
    ) -> Result<()> {
        let segments = match space {
            OecaSpace::Home => HOUSE_BLANK,
            OecaSpace::Music => MUSIC_ALT,
            OecaSpace::Clipboard => CLIPBOARD,
        };
        self.draw_glyph(rt, segments, cx, cy, u, brush)
    }

    /// Fills an icon glyph (`IconSeg` path) in a `u`-sized box centred on (cx, cy).
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

    /// Drop page, a fresh surface while files are dragged over the notch: a
    /// rounded dashed outline inset from the notch body (same corner profile as
    /// the notch) with the inbox glyph centred in it. Files that are not
    /// images dim the glyph and add a one-line hint under it.
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
        let unsupported = self.drop.get() == crate::dragdrop::DropPage::Unsupported;
        self.with_layer(rt, Some(&silhouette), 1.0, || unsafe {
            let dashes = rt.CreateSolidColorBrush(&COLOR_DROP_OUTLINE, None)?;
            rt.DrawGeometry(&path, &dashes, width, &self.drop_stroke);
            let (cx, cy) = (
                (outline.left + outline.right) / 2.0,
                (outline.top + outline.bottom) / 2.0,
            );
            if !unsupported {
                let icon = rt.CreateSolidColorBrush(&COLOR_DROP_ICON, None)?;
                return self.draw_glyph(rt, INBOX, cx, cy, px(BASE_DROP_ICON_SIZE), &icon);
            }
            let icon = rt.CreateSolidColorBrush(&COLOR_TEXT_TERTIARY, None)?;
            self.draw_drop_message(
                rt,
                &target,
                outline,
                INBOX,
                &icon,
                DROP_UNSUPPORTED_LABEL,
                &COLOR_TEXT_TERTIARY,
            )
        })
    }

    /// A drop's result, on the drop page's surface: a check and "Item copied
    /// to clipboard", or an alert and why not. Laid out like the drop page
    /// (one universal size, clipped to the current silhouette); it fades in
    /// over the page through the scene blend.
    fn draw_drop_result(&self, rt: &ID2D1RenderTarget, dimensions: &NotchDimensions) -> Result<()> {
        let target = drop_page_dimensions(dimensions.dpi);
        let px = |v: f32| (v * target.scale).round();
        let wall = target.shadow_margin_x + target.curvature.top_transition_radius;
        let inset = px(BASE_DROP_OUTLINE_INSET);
        let dx = ((dimensions.width - target.width) as f32 / 2.0).round();
        let area = RectF::new(
            dx + wall + inset,
            inset,
            dx + target.width as f32 - wall - inset,
            target.height as f32 - target.shadow_margin_bottom - inset,
        );
        if area.width() <= 0.0 || area.height() <= 0.0 {
            return Ok(());
        }
        let outcome = self.drop_outcome.get();
        let (glyph, glyph_color, text_color) = if outcome.succeeded() {
            (DONE, COLOR_DROP_ICON, COLOR_TEXT_PRIMARY)
        } else {
            (ALERT, COLOR_TEXT_TERTIARY, COLOR_TEXT_SECONDARY)
        };
        let silhouette = self.create_notch_geometry(dimensions)?;
        self.with_layer(rt, Some(&silhouette), 1.0, || unsafe {
            let icon = rt.CreateSolidColorBrush(&glyph_color, None)?;
            self.draw_drop_message(
                rt,
                &target,
                area,
                glyph,
                &icon,
                outcome.message(),
                &text_color,
            )
        })
    }

    /// A glyph with a one-line message under it, centred as a pair in `area`
    /// (laid out at the drop page's size, `target`).
    #[allow(clippy::too_many_arguments)]
    fn draw_drop_message(
        &self,
        rt: &ID2D1RenderTarget,
        target: &NotchDimensions,
        area: RectF,
        glyph: &[IconSeg],
        icon: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
        message: &str,
        color: &D2D1_COLOR_F,
    ) -> Result<()> {
        let px = |v: f32| (v * target.scale).round();
        let size = px(BASE_DROP_RESULT_ICON_SIZE);
        let line = px(BASE_DROP_MESSAGE_LINE);
        let gap = px(BASE_DROP_MESSAGE_GAP);
        let top = ((area.top + area.bottom) - (size + gap + line)) / 2.0;
        let cx = (area.left + area.right) / 2.0;
        self.draw_glyph(rt, glyph, cx, top + size / 2.0, size, icon)?;
        let formats = self.media_formats(target)?;
        let text: Vec<u16> = message.encode_utf16().collect();
        let row = RectF::new(
            area.left,
            top + size + gap,
            area.right,
            top + size + gap + line,
        );
        unsafe {
            let brush = rt.CreateSolidColorBrush(color, None)?;
            rt.DrawText(
                &text,
                &formats.empty,
                &row.to_d2d_rect(),
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
        Ok(())
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
            let selector_alpha = 1.0 - weight(Scene::Drop) - weight(Scene::Dropped);
            if selector_alpha > 0.0 {
                let mix = if blending { t.mix } else { 1.0 };
                self.draw_selector_blended(rt, dims, t.from, t.to, mix, selector_alpha)?;
            }
            if !blending {
                return self.draw_scene(rt, dims, clock, t.to);
            }
            // Home <-> Music with a session: one composition morphing
            if let (Scene::Space(a), Scene::Space(b)) = (t.from, t.to)
                && a != OecaSpace::Clipboard
                && b != OecaSpace::Clipboard
                && self.media.borrow().is_some()
            {
                let music = if b == OecaSpace::Music {
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
            Scene::Dropped => return self.draw_drop_result(rt, dims),
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
        } else if space == OecaSpace::Clipboard {
            self.draw_clipboard(rt, &settled)
        } else {
            let media = self.media.borrow();
            let composition = media.as_ref().and_then(|m| {
                resolve_media_layout_in(&settled, m.content.shape(), space).map(|l| (m, l))
            });
            match composition {
                Some((m, l)) => {
                    let music = f32::from(u8::from(space == OecaSpace::Music));
                    self.render_media_blend(rt, &components, &l, &settled, clock, m, music)
                }
                None if space == OecaSpace::Music => {
                    self.draw_music_empty(rt, &components, &settled)
                }
                None => match crate::layout::resolve_home_timer_layout(&settled, None) {
                    // Home without media: the clock in the left column, the
                    // timer in the right one
                    Some(home) => {
                        self.draw_home_with_timer(rt, &components, &home, &settled, clock)
                    }
                    None => self.render_expanded_content(
                        rt,
                        &components,
                        &settled,
                        &clock.formatted_time,
                        &clock.formatted_date,
                    ),
                },
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
        let place = |space: OecaSpace| {
            let settled = space_dimensions(crate::config::NotchState::Expanded, dims.dpi, space);
            let dx = ((dims.width - settled.width) as f32 / 2.0).round();
            resolve_media_layout_in(&settled, m.content.shape(), space).map(|l| l.offset_x(dx))
        };
        let (Some(home), Some(player)) = (place(OecaSpace::Home), place(OecaSpace::Music)) else {
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
            let shown_tiles = if confirming { 0 } else { view.rows.len() };
            let lift = rt.CreateSolidColorBrush(&COLOR_SPACE_PILL_SELECTED, None)?;
            for (i, row) in view.rows.iter().take(shown_tiles).enumerate() {
                // A squircle tile; its copy / trash buttons fade in only while
                // this tile is hovered (one tile at a time), over space kept
                // for them, so nothing moves
                let p = l.tile_parts(i);
                let shown = self.feedback.borrow().clip_levels(PanelHit::Row(i)).0;
                let half = outline_w / 2.0;
                let edge = RectF::new(
                    p.tile.left + half,
                    p.tile.top + half,
                    p.tile.right - half,
                    p.tile.bottom - half,
                );
                let profile = CornerProfile::new(
                    px(crate::config::BASE_CLIPBOARD_TILE_RADIUS),
                    1.0,
                    edge.width().min(edge.height()) / 2.0,
                );
                let path = self.smooth_rect_path(edge, profile.span, profile.handle)?;
                if shown > 0.0 {
                    lift.SetOpacity(0.45 * shown);
                    rt.FillGeometry(&path, &lift, None);
                }
                rt.DrawGeometry(&path, &outline, outline_w, None);
                match row {
                    ClipRow::Text(t) => text(t, &formats.clip_text, p.text, &primary),
                    ClipRow::Image(art, caption) => {
                        let img = p.image;
                        let size = (img.width().max(1.0) as u32, img.height().max(1.0) as u32);
                        let radius = (profile.span - outline_w).max(0.0) * 0.6;
                        match self.thumb_bitmap(rt, art, size, radius) {
                            Ok(bitmap) => rt.DrawBitmap(
                                &bitmap,
                                Some(&img.to_d2d_rect()),
                                1.0,
                                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                                None,
                            ),
                            Err(e) => drawn = Err(e),
                        }
                        text(caption, &formats.source, p.caption, &tertiary);
                    }
                }
                if shown > 0.0 {
                    secondary.SetOpacity(shown);
                    for (hit, glyph, r) in [
                        (PanelHit::Copy(i), COPY, p.copy),
                        (PanelHit::Remove(i), TRASH, p.trash),
                    ] {
                        let (cx, cy) = center(r);
                        if let Err(e) =
                            self.draw_glyph(rt, glyph, cx, cy, icon * grow(hit), &secondary)
                        {
                            drawn = Err(e);
                        }
                    }
                    secondary.SetOpacity(1.0);
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
        size: (u32, u32),
        radius: f32,
    ) -> Result<ID2D1Bitmap> {
        Self::thumb_bitmap_in(&self.thumbs, rt, art, size, radius)
    }

    /// The collapsed notch's album art in `dest` (whole pixels): each cover
    /// on screen as a premade rounded thumbnail (area-resampled once per
    /// cover and size, then a plain blit) at its fade's opacity. Thumbnails
    /// of covers that have faded out are released.
    fn draw_collapsed_art(&self, rt: &ID2D1RenderTarget, dest: RectF, scale: f32) -> Result<()> {
        let art = self.collapsed_art.borrow();
        let size = (dest.width().max(1.0) as u32, dest.height().max(1.0) as u32);
        let radius = (crate::config::BASE_COLLAPSED_ARTWORK_RADIUS * scale).round();
        for (cover, fade) in &art.layers {
            let opacity = fade.value.clamp(0.0, 1.0);
            if opacity <= 0.0 {
                continue;
            }
            let bitmap = Self::thumb_bitmap_in(&self.collapsed_thumbs, rt, cover, size, radius)?;
            unsafe {
                rt.DrawBitmap(
                    &bitmap,
                    Some(&dest.to_d2d_rect()),
                    opacity,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
            }
        }
        self.collapsed_thumbs
            .borrow_mut()
            .retain(|(a, _, _)| art.layers.iter().any(|(b, _)| a == b));
        Ok(())
    }

    /// `thumb_bitmap` kept in `cache` (one size per image).
    fn thumb_bitmap_in(
        cache: &RefCell<Vec<Thumb>>,
        rt: &ID2D1RenderTarget,
        art: &Artwork,
        size: (u32, u32),
        radius: f32,
    ) -> Result<ID2D1Bitmap> {
        if let Some((_, _, bitmap)) = cache
            .borrow()
            .iter()
            .find(|(a, s, _)| a == art && *s == size)
        {
            return Ok(bitmap.clone());
        }
        let thumb = thumbnail(art, size.0, size.1, radius);
        let bitmap = Self::cached_bitmap(rt, &RefCell::new(None), &thumb)?;
        let mut thumbs = cache.borrow_mut();
        thumbs.retain(|(a, _, _)| a != art);
        thumbs.push((art.clone(), size, bitmap.clone()));
        Ok(bitmap)
    }

    /// The gradient's two ends now (top, bottom): the artwork's gradient, or
    /// for neutral or missing artwork the usual accent (`fallback`), a touch
    /// lighter at the top; blending smoothly when the artwork changes.
    fn viz_colours(&self, fallback: D2D1_COLOR_F) -> [D2D1_COLOR_F; 2] {
        let target = match self.viz_palette.borrow().as_ref().and_then(|(_, g)| *g) {
            Some(g) => [rgb_color(g.top), rgb_color(g.bottom)],
            None => {
                let lift = |v: f32| v + (1.0 - v) * 0.12;
                let top = D2D1_COLOR_F {
                    r: lift(fallback.r),
                    g: lift(fallback.g),
                    b: lift(fallback.b),
                    a: 1.0,
                };
                [top, D2D1_COLOR_F { a: 1.0, ..fallback }]
            }
        };
        let now = std::time::Instant::now();
        let mut blend = self.viz_blend.get();
        let instant = !crate::window::motion_allowed(
            self.settings.get().reduced_motion,
            crate::window::is_client_animation_enabled(),
        );
        blend.retarget(target, now, instant);
        self.viz_blend.set(blend);
        blend.current(now)
    }

    /// The visualizer's two bitmaps for the current render target (made once
    /// per target and bar width, i.e. DPI; the ramp is then only rewritten in
    /// place): the gradient ramp (1 x `VIZ_RAMP_STEPS`), and the round-end
    /// masks for `bar`-wide bars (see `viz_cap_mask`).
    fn viz_brushes(&self, rt: &ID2D1RenderTarget, bar: f32) -> Result<(ID2D1Bitmap, ID2D1Bitmap)> {
        let width = bar as u32;
        if let Some((ramp, caps, w)) = self.viz_brushes.borrow().as_ref()
            && *w == width
        {
            return Ok((ramp.clone(), caps.clone()));
        }
        let (mask, rows) = viz_cap_mask(width);
        let bitmap = |w: u32, h: u32, data: Option<&[u8]>| unsafe {
            rt.CreateBitmap(
                D2D_SIZE_U {
                    width: w,
                    height: h,
                },
                data.map(|d| d.as_ptr() as *const _),
                w * 4,
                &D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                },
            )
        };
        let ramp = bitmap(1, VIZ_RAMP_STEPS, None)?;
        let caps = bitmap(width, rows, Some(&mask))?;
        self.viz_baked.set(None);
        *self.viz_brushes.borrow_mut() = Some((ramp.clone(), caps.clone(), width));
        Ok((ramp, caps))
    }

    /// Writes the gradient into the ramp bitmap, only when its ends or the
    /// opacity changed (a blend or fade; never while steady). From a stack
    /// buffer: no allocation.
    fn bake_viz_ramp(
        &self,
        ramp: &ID2D1Bitmap,
        ends: [D2D1_COLOR_F; 2],
        opacity: f32,
    ) -> Result<()> {
        if self.viz_baked.get() == Some((ends, opacity)) {
            return Ok(());
        }
        let mut px = [[0u8; 4]; VIZ_RAMP_STEPS as usize];
        for (i, p) in px.iter_mut().enumerate() {
            let c = viz_ramp_colour(ends, (i as f32 + 0.5) / VIZ_RAMP_STEPS as f32, opacity);
            let byte = |v: f32| (v * 255.0).round() as u8;
            *p = [byte(c.b), byte(c.g), byte(c.r), 255];
        }
        unsafe { ramp.CopyFromMemory(None, px.as_ptr() as *const _, 4)? };
        self.viz_baked.set(Some((ends, opacity)));
        Ok(())
    }

    /// The visualizer: four thin bars with fully round ends, a compact group
    /// on whole pixels at its slot, heights following the playback motion
    /// within a balanced silhouette. Every bar shows the same gradient (the
    /// artwork's, top end to bottom end of each bar, brightest in the
    /// middle): the ramp bitmap stretched over the bar, its corners then
    /// rounded off by the end masks (the notch's black). Three blits a bar
    /// (anti-aliased shape fills cost ~15 us each on this software target, a
    /// blit ~5). Draws nothing unless the visualizer is (fading) visible. No
    /// resources are made per frame.
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
        // Whole-pixel bars at every DPI (a half-pixel slot would smear them)
        let x0 = bounds.left.round();
        let ends = self.viz_colours(accent);
        let opacity = viz.level() * alpha;
        let (ramp, caps) = self.viz_brushes(rt, bar)?;
        self.bake_viz_ramp(&ramp, ends, opacity)?;
        let t = self.epoch.elapsed().as_secs_f64();
        let r = bar / 2.0;
        // The bottom mask's first row (below the top mask and a spacer)
        let below = r.ceil() + VIZ_CAP_GAP as f32;
        let blit = |bitmap: &ID2D1Bitmap, dest: RectF, source: Option<RectF>| unsafe {
            rt.DrawBitmap(
                bitmap,
                Some(&dest.to_d2d_rect()),
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                source
                    .map(|s| s.to_d2d_rect())
                    .as_ref()
                    .map(|s| s as *const _),
            )
        };
        for (i, h) in viz.bar_heights(t).into_iter().enumerate() {
            let left = x0 + i as f32 * (bar + gap);
            let half = (full * h * VIZ_ENVELOPE[i]).max(bar) / 2.0;
            let (top, bottom) = (cy - half, cy + half);
            blit(&ramp, RectF::new(left, top, left + bar, bottom), None);
            blit(
                &caps,
                RectF::new(left, top, left + bar, top + r),
                Some(RectF::new(0.0, 0.0, bar, r)),
            );
            blit(
                &caps,
                RectF::new(left, bottom - r, left + bar, bottom),
                Some(RectF::new(0.0, below, bar, below + r)),
            );
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
        // Dragged: where the pointer is; just committed: the seek, moving on
        // from there while it plays; otherwise the session's position
        let elapsed = match (self.scrub.get(), self.seek_hold.get()) {
            (Some(f), _) => (f.clamp(0.0, 1.0) * timeline.duration_ms as f32) as i64,
            (None, Some((pos, at))) if at.elapsed().as_millis() < SEEK_HOLD_MS => {
                let moved = if playing {
                    at.elapsed().as_millis() as i64
                } else {
                    0
                };
                (pos + moved).clamp(0, timeline.duration_ms)
            }
            _ => timeline.position_at(now_filetime(), playing),
        };
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

    /// Home without media, with its timer: the clock (same type, centred in
    /// the left column), the centre divider, and the timer panel.
    fn draw_home_with_timer(
        &self,
        rt: &ID2D1RenderTarget,
        components: &ExpandedLayout,
        home: &crate::layout::HomeTimerLayout,
        dimensions: &NotchDimensions,
        clock: &ClockDateState,
    ) -> Result<()> {
        let mut layout = components.clone();
        if let Some((time, date)) = home.clock {
            layout.time_bounds = time;
            layout.date_bounds = date;
        }
        self.render_expanded_content(
            rt,
            &layout,
            dimensions,
            &clock.formatted_time,
            &clock.formatted_date,
        )?;
        if let Some(d) = home.divider {
            unsafe {
                let line = rt.CreateSolidColorBrush(&COLOR_MEDIA_DIVIDER, None)?;
                rt.FillRectangle(&d.to_d2d_rect(), &line);
            }
        }
        self.draw_timer_panel(rt, &home.timer, dimensions, 1.0)
    }

    /// Home's timer panel at `alpha`: the numerals (setup: the chosen
    /// duration in white; running: what is left in the timer's orange;
    /// paused: dimmed; done: 0:00 pulsing gently), the round buttons (the
    /// primary one in an orange wash, Cancel in a quiet grey one), and the
    /// duration scale (setup) or a status line.
    fn draw_timer_panel(
        &self,
        rt: &ID2D1RenderTarget,
        l: &crate::layout::TimerLayout,
        dimensions: &NotchDimensions,
        alpha: f32,
    ) -> Result<()> {
        use crate::config::{
            BASE_TIMER_GLYPH_SIZE, BASE_TIMER_INDICATOR_HEIGHT, BASE_TIMER_INDICATOR_WIDTH,
            BASE_TIMER_MAJOR_TICK_HEIGHT, BASE_TIMER_TICK_HEIGHT, BASE_TIMER_TICK_WIDTH,
            CLIPBOARD_BUTTON_HOVER_GROW, CLIPBOARD_BUTTON_PRESS_GROW, COLOR_TIMER_TICK,
            TIMER_BUTTON_WASH,
        };
        use crate::timer::STOPS;
        let view = self.timer.get();
        let t = view.countdown;
        let idle = t.phase() == Phase::Idle;
        let formats = self.media_formats(dimensions)?;
        let px = |v: f32| (v * dimensions.scale).round();
        let brush = |color: &D2D1_COLOR_F, opacity: f32| unsafe {
            let b = rt.CreateSolidColorBrush(color, None)?;
            b.SetOpacity(opacity * alpha);
            Ok::<_, Error>(b)
        };

        // Numerals
        let (color, opacity) = match t.phase() {
            Phase::Idle => (COLOR_TEXT_PRIMARY, 1.0),
            Phase::Running { .. } => (COLOR_TIMER, 1.0),
            Phase::Paused { .. } => (COLOR_TEXT_SECONDARY, 1.0),
            Phase::Done { .. } => (COLOR_TIMER, view.pulse),
        };
        let mut buf = [0u16; 12];
        let n = timer_numerals(&t, &mut buf);
        let numerals = brush(&color, opacity)?;
        self.draw_timer_text(
            rt,
            &buf[..n],
            &formats.timer_display,
            l.display(idle),
            &numerals,
        )?;

        // Round buttons: a wash disc and a glyph, growing a little on hover
        // and press like the other icon buttons
        let feedback = self.feedback.borrow();
        let u = px(BASE_TIMER_GLYPH_SIZE);
        let mut buttons = vec![(PanelHit::TimerPrimary, l.primary)];
        if !idle {
            buttons.push((PanelHit::TimerCancel, l.cancel));
        }
        for (hit, r) in buttons {
            let (hover, press) = feedback.clip_levels(hit);
            let grow = 1.0 + CLIPBOARD_BUTTON_HOVER_GROW * hover * 0.5
                - CLIPBOARD_BUTTON_PRESS_GROW * press * 0.5;
            let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
            let radius = r.width() / 2.0 * grow;
            let (wash, ink) = if hit == PanelHit::TimerPrimary {
                (
                    brush(&COLOR_TIMER, TIMER_BUTTON_WASH)?,
                    brush(&COLOR_TIMER, 1.0)?,
                )
            } else {
                (
                    brush(&COLOR_SPACE_PILL_SELECTED, 1.0)?,
                    brush(&COLOR_TEXT_PRIMARY, 1.0)?,
                )
            };
            unsafe {
                rt.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: RectF::new(cx - radius, cy - radius, cx + radius, cy + radius)
                            .to_d2d_rect(),
                        radiusX: radius,
                        radiusY: radius,
                    },
                    &wash,
                );
            }
            if hit == PanelHit::TimerCancel {
                self.draw_cross(rt, cx, cy, u * grow, &ink)?;
                continue;
            }
            // Pause while it runs; otherwise play (start / resume / restart)
            let icon = if matches!(t.phase(), Phase::Running { .. }) {
                PlayPauseIcon::Pause
            } else {
                PlayPauseIcon::Play
            };
            for shape in icon_shapes(
                MediaControl::PlayPause,
                icon,
                cx,
                cy,
                u * grow,
                press == 0.0,
            ) {
                match shape {
                    IconShape::Bar(b) => unsafe {
                        rt.FillRoundedRectangle(
                            &D2D1_ROUNDED_RECT {
                                rect: b.to_d2d_rect(),
                                radiusX: b.width() / 2.0,
                                radiusY: b.width() / 2.0,
                            },
                            &ink,
                        );
                    },
                    IconShape::Triangle(pts) => self.fill_triangle(rt, pts, &ink)?,
                }
            }
        }

        let row = l.row;
        let cy = ((row.top + row.bottom) / 2.0).round();
        if idle {
            // Duration scale: a tick per stop (taller where minutes and
            // hours begin), the indicator on the chosen one
            let ticks = brush(&COLOR_TIMER_TICK, 1.0)?;
            let tw = px(BASE_TIMER_TICK_WIDTH).max(1.0);
            for (i, secs) in STOPS.iter().enumerate() {
                let major = matches!(secs, 60 | 3600) || i == 0 || i == STOPS.len() - 1;
                let h = px(if major {
                    BASE_TIMER_MAJOR_TICK_HEIGHT
                } else {
                    BASE_TIMER_TICK_HEIGHT
                });
                let x = l.stop_x(i, STOPS.len());
                let top = (cy - h / 2.0).round();
                unsafe {
                    rt.FillRectangle(
                        &RectF::new(
                            x - (tw / 2.0).floor(),
                            top,
                            x - (tw / 2.0).floor() + tw,
                            top + h,
                        )
                        .to_d2d_rect(),
                        &ticks,
                    );
                }
            }
            let (iw, ih) = (
                px(BASE_TIMER_INDICATOR_WIDTH),
                px(BASE_TIMER_INDICATOR_HEIGHT),
            );
            let x = l.stop_x(t.stop(), STOPS.len());
            let top = (cy - ih / 2.0).round();
            let left = (x - iw / 2.0).round();
            let indicator = brush(&COLOR_TIMER, 1.0)?;
            unsafe {
                rt.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: RectF::new(left, top, left + iw, top + ih).to_d2d_rect(),
                        radiusX: iw / 2.0,
                        radiusY: iw / 2.0,
                    },
                    &indicator,
                );
            }
        } else {
            let status = match t.phase() {
                Phase::Paused { .. } => "Paused".to_string(),
                Phase::Done { .. } => "Timer done".to_string(),
                _ => format!(
                    "{} timer",
                    crate::timer::describe(t.duration().as_secs(), false)
                ),
            };
            let text: Vec<u16> = status.encode_utf16().collect();
            let label = brush(&COLOR_TEXT_TERTIARY, 1.0)?;
            unsafe {
                rt.DrawText(
                    &text,
                    &formats.timer_status,
                    &row.to_d2d_rect(),
                    &label,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
        }
        Ok(())
    }

    /// A text layout of `text` in `format` with tabular figures (digits keep
    /// their width as a countdown runs), and its natural width.
    fn tabular_layout(
        &self,
        text: &[u16],
        format: &IDWriteTextFormat,
    ) -> Result<(
        windows::Win32::Graphics::DirectWrite::IDWriteTextLayout,
        f32,
    )> {
        unsafe {
            let layout = self
                .dwrite_factory
                .CreateTextLayout(text, format, 10_000.0, 1.0)?;
            layout.SetTypography(
                &self.tabular,
                DWRITE_TEXT_RANGE {
                    startPosition: 0,
                    length: text.len() as u32,
                },
            )?;
            let mut m = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut m)?;
            Ok((layout, m.widthIncludingTrailingWhitespace))
        }
    }

    /// Timer numerals in `rect`, aligned as `format` says, with tabular
    /// figures; scaled down (never clipped or trimmed) if they would not fit
    /// (e.g. a long duration at a large DPI).
    fn draw_timer_text(
        &self,
        rt: &ID2D1RenderTarget,
        text: &[u16],
        format: &IDWriteTextFormat,
        rect: RectF,
        brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
    ) -> Result<()> {
        let (layout, natural) = self.tabular_layout(text, format)?;
        unsafe {
            layout.SetMaxWidth(rect.width().max(1.0))?;
            layout.SetMaxHeight(rect.height().max(1.0))?;
            if natural > rect.width() && natural > 0.0 {
                let size = format.GetFontSize() * rect.width() / natural * 0.98;
                layout.SetFontSize(
                    size,
                    DWRITE_TEXT_RANGE {
                        startPosition: 0,
                        length: text.len() as u32,
                    },
                )?;
            }
            let mut origin =
                windows::Win32::Graphics::Direct2D::D2D1_POINT_DESCRIPTION::default().point;
            origin.X = rect.left;
            origin.Y = rect.top;
            rt.DrawTextLayout(origin, &layout, brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
        Ok(())
    }

    /// Renders the countdown bubble beside the collapsed notch into its own
    /// layered window: `wall_x` / `top` (screen px) is where the notch's body
    /// ends; `presence` 0..1 is how far it has budded out (see
    /// `timer_bubble_geometry`). A pure-black circle with the timer's ring,
    /// depleting as the countdown runs, and while it is close to the notch a
    /// liquid neck joining it to the wall. Shares this renderer's surface
    /// (the notch's frame is already with DWM when this runs).
    pub fn render_bubble(
        &self,
        hwnd: HWND,
        wall_x: i32,
        top: i32,
        dpi: u32,
        presence: f32,
    ) -> Result<crate::layout::BubbleGeometry> {
        let g = crate::layout::timer_bubble_geometry(dpi, presence);
        let view = self.timer.get();
        let (w, h) = g.window;
        self.ensure_buffer(w, h)?;
        let rt_borrow = self.cached_rt.borrow();
        let Some(rt) = rt_borrow.as_ref() else {
            return Err(Error::from_thread());
        };
        unsafe {
            rt.BindDC(
                self.cached_mem_dc.get(),
                &RECT {
                    left: 0,
                    top: 0,
                    right: w,
                    bottom: h,
                },
            )?;
            rt.BeginDraw();
            rt.Clear(Some(&COLOR_TRANSPARENT));
            let black = rt.CreateSolidColorBrush(&NOTCH_BG_COLOR, None)?;
            let (cx, cy) = g.center;
            let r = g.radius;
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: RectF::new(cx - r, cy - r, cx + r, cy + r).to_d2d_rect(),
                    radiusX: r,
                    radiusY: r,
                },
                &black,
            );
            if g.neck > 0.0 {
                let neck = self.liquid_neck(&g)?;
                rt.FillGeometry(&neck, &black, None);
            }
            // The ring: a faint track, and the time left from 12 o'clock
            // clockwise (it shows once the bubble is out)
            let ring_alpha = smoothstep(0.6, 1.0, presence) * view.pulse;
            let (ring_r, ring_w) = g.ring;
            if ring_alpha > 0.0 && ring_r > 0.0 {
                let c = view.collapsed;
                let total = c.duration().as_secs_f32().max(0.001);
                let left = c.remaining(std::time::Instant::now()).as_secs_f32();
                let frac = (left / total).clamp(0.0, 1.0);
                let track = rt.CreateSolidColorBrush(&COLOR_TIMER, None)?;
                track.SetOpacity(0.22 * ring_alpha);
                let full = self.arc_path(cx, cy, ring_r, 1.0)?;
                rt.DrawGeometry(&full, &track, ring_w, &self.round_stroke);
                if frac > 0.0 {
                    let ink = rt.CreateSolidColorBrush(&COLOR_TIMER, None)?;
                    ink.SetOpacity(ring_alpha);
                    let arc = self.arc_path(cx, cy, ring_r, frac)?;
                    rt.DrawGeometry(&arc, &ink, ring_w, &self.round_stroke);
                }
            }
            rt.EndDraw(None, None)?;
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                hwnd,
                Some(self.cached_screen_dc.get()),
                Some(&POINT {
                    x: wall_x - g.wall as i32,
                    y: top,
                }),
                Some(&SIZE { cx: w, cy: h }),
                Some(self.cached_mem_dc.get()),
                Some(&POINT { x: 0, y: 0 }),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )?;
        }
        Ok(g)
    }

    /// An open arc of `frac` of a full turn around (cx, cy), from 12 o'clock
    /// clockwise (a close polyline: smooth at these sizes).
    fn arc_path(&self, cx: f32, cy: f32, r: f32, frac: f32) -> Result<ID2D1PathGeometry> {
        let path = unsafe { self.d2d_factory.CreatePathGeometry()? };
        let steps = ((64.0 * frac).ceil() as usize).max(2);
        let at = |k: usize| {
            let a = std::f32::consts::TAU * frac * k as f32 / steps as f32;
            D2D_POINT_2F {
                x: cx + r * a.sin(),
                y: cy - r * a.cos(),
            }
        };
        unsafe {
            let sink = path.Open()?;
            let g = GeometrySinkHelper::from_raw(windows::core::Interface::as_raw(&sink));
            g.begin_figure(at(0));
            for k in 1..=steps {
                g.add_line(at(k));
            }
            if frac >= 1.0 {
                g.end_figure();
            } else {
                g.end_figure_open();
            }
            g.close()?;
        }
        Ok(path)
    }

    /// The liquid neck from the notch's wall to the circle, shaped like
    /// surface tension: a meniscus foot flaring out of the wall, a waist
    /// (`g.neck`) that thins as the drop moves away until it pinches off,
    /// and a smooth join into the circle along its own tangent.
    fn liquid_neck(&self, g: &crate::layout::BubbleGeometry) -> Result<ID2D1PathGeometry> {
        let (cx, cy) = g.center;
        let r = g.radius;
        let full = (g.neck / (0.85 * r)).clamp(0.0, 1.0);
        // Where it meets the circle: nearer its leftmost point while thin
        let a = 0.35 + 0.52 * full;
        let (tx, ty) = (cx - r * a.cos(), r * a.sin());
        // The foot on the wall is wider than the waist (a meniscus)
        let foot = g.neck + 0.3 * r * full.sqrt();
        let waist = g.neck;
        let path = unsafe { self.d2d_factory.CreatePathGeometry()? };
        unsafe {
            let sink = path.Open()?;
            let s = GeometrySinkHelper::from_raw(windows::core::Interface::as_raw(&sink));
            let p = |x: f32, y: f32| D2D_POINT_2F { x, y };
            let span = (tx - g.wall).max(1.0);
            let k = span * 0.45;
            // From inside the notch (the window's left edge) to the wall
            s.begin_figure(p(0.0, cy - foot));
            s.add_line(p(g.wall, cy - foot));
            // Upper edge: down into the waist, then up the circle's tangent
            s.add_bezier(&D2D1_BEZIER_SEGMENT {
                point1: p(g.wall + span * 0.35, cy - waist),
                point2: p(tx - k * a.sin(), cy - ty + k * a.cos()),
                point3: p(tx, cy - ty),
            });
            s.add_line(p(cx, cy));
            s.add_line(p(tx, cy + ty));
            s.add_bezier(&D2D1_BEZIER_SEGMENT {
                point1: p(tx - k * a.sin(), cy + ty - k * a.cos()),
                point2: p(g.wall + span * 0.35, cy + waist),
                point3: p(g.wall, cy + foot),
            });
            s.add_line(p(0.0, cy + foot));
            s.end_figure();
            s.close()?;
        }
        Ok(path)
    }

    /// Fills transport glyph primitives (capsule bars, rounded triangles).
    fn fill_shapes(
        &self,
        rt: &ID2D1RenderTarget,
        shapes: &[IconShape],
        brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
    ) -> Result<()> {
        for shape in shapes {
            match *shape {
                IconShape::Bar(r) => unsafe {
                    let radius = r.width() / 2.0;
                    rt.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: r.to_d2d_rect(),
                            radiusX: radius,
                            radiusY: radius,
                        },
                        brush,
                    );
                },
                IconShape::Triangle(pts) => self.fill_triangle(rt, pts, brush)?,
            }
        }
        Ok(())
    }

    /// The play/pause glyph at morph `m` (see `play_pause_outline`), pressed
    /// down to `squeeze_by` of its size.
    #[allow(clippy::too_many_arguments)]
    fn draw_play_pause(
        &self,
        rt: &ID2D1RenderTarget,
        cx: f32,
        cy: f32,
        u: f32,
        m: f32,
        squeeze_by: f32,
        brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
    ) -> Result<()> {
        // Always the whole-pixel glyph geometry, compressed by a press as a
        // scale about its centre
        let (shapes, r) = play_pause_outline(cx, cy, u, m, true);
        let k = squeeze_by;
        let shapes: Vec<Quad> = shapes
            .into_iter()
            .map(|q| q.map(|p| squeeze(p, cx, cy, k)))
            .collect();
        let r = r * k;
        // Its parts overlap (fill + stroke, halves + bridge): drawn opaque in
        // one layer that takes the brush's opacity, so they never compound
        let opacity = unsafe { brush.GetOpacity() };
        unsafe { brush.SetOpacity(1.0) };
        let drawn = self.with_layer(rt, None, opacity.min(0.999), || {
            self.draw_outlines(rt, &shapes, r, brush)
        });
        unsafe { brush.SetOpacity(opacity) };
        drawn
    }

    /// Fills convex outlines and strokes them `2 r` wide with round joins.
    /// Repeated points are dropped first, and an outline collapsed to a line
    /// (a pause bar inset to its centre) is stroked as that line with round
    /// caps: Direct2D does not stroke fully degenerate closed figures the
    /// same way as nearly degenerate ones, which made the rest glyph differ
    /// from the frame before it.
    fn draw_outlines(
        &self,
        rt: &ID2D1RenderTarget,
        shapes: &[Quad],
        r: f32,
        brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
    ) -> Result<()> {
        for q in shapes {
            let mut pts: Vec<(f32, f32)> = Vec::with_capacity(4);
            for p in q {
                if pts
                    .last()
                    .is_none_or(|l: &(f32, f32)| (l.0 - p.0).hypot(l.1 - p.1) > 1e-3)
                {
                    pts.push(*p);
                }
            }
            while pts.len() > 1 && {
                let (f, l) = (pts[0], pts[pts.len() - 1]);
                (f.0 - l.0).hypot(f.1 - l.1) <= 1e-3
            } {
                pts.pop();
            }
            let area: f32 = (0..pts.len())
                .map(|i| {
                    let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                    a.0 * b.1 - b.0 * a.1
                })
                .sum::<f32>()
                .abs()
                / 2.0;
            let perimeter: f32 = (0..pts.len())
                .map(|i| {
                    let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                    (b.0 - a.0).hypot(b.1 - a.1)
                })
                .sum();
            // How thick it is (2 area / perimeter: a sliver's width)
            let thickness = 2.0 * area / perimeter.max(1e-6);
            let path = unsafe { self.d2d_factory.CreatePathGeometry()? };
            unsafe {
                let sink = path.Open()?;
                let h = GeometrySinkHelper::from_raw(windows::core::Interface::as_raw(&sink));
                let p = |(x, y): (f32, f32)| D2D_POINT_2F { x, y };
                if pts.len() < 3 || thickness < 0.3 {
                    // A line, or a sliver about to become one (drawn the same
                    // way, so the morph lands on it seamlessly): its two
                    // farthest-apart points, the stroke widened by its
                    // thickness
                    let (mut a, mut b) = (pts[0], pts[0]);
                    for &u in &pts {
                        for &v in &pts {
                            if (u.0 - v.0).hypot(u.1 - v.1) > (a.0 - b.0).hypot(a.1 - b.1) {
                                (a, b) = (u, v);
                            }
                        }
                    }
                    h.begin_figure(p(a));
                    h.add_line(p(b));
                    h.end_figure_open();
                    h.close()?;
                    let width = 2.0 * r + thickness;
                    if width > 0.01 {
                        rt.DrawGeometry(&path, brush, width, &self.round_stroke);
                    }
                    continue;
                }
                h.begin_figure(p(pts[0]));
                for v in &pts[1..] {
                    h.add_line(p(*v));
                }
                h.end_figure();
                h.close()?;
                rt.FillGeometry(&path, brush, None);
                if r > 0.01 {
                    rt.DrawGeometry(&path, brush, 2.0 * r, &self.round_stroke);
                }
            }
        }
        Ok(())
    }

    /// The artwork mid-flip: exactly the resting artwork (same bitmap,
    /// corners and hairline) of the side facing the viewer, foreshortened
    /// horizontally by the flip about the cover's vertical centre line. One
    /// affine scale: the plane stays one flat rectangle, its height and
    /// position never change, and nothing fades, slides or is sliced.
    ///
    /// Direct2D 1.0's render target (the one this layered surface uses)
    /// offers affine transforms only, so this is the orthographic view of
    /// the turning plane: true perspective (a nearer edge drawn taller) is
    /// not available here.
    fn draw_artwork_turn(
        &self,
        rt: &ID2D1RenderTarget,
        dest: RectF,
        radius: f32,
        scale: f32,
    ) -> Result<()> {
        let (cover, k) = {
            let flip = self.flip.borrow();
            let Some(f) = flip.as_ref() else {
                return Ok(());
            };
            (f.shown().clone(), f.width_fraction(dest.width()))
        };
        // Scale x by k about the centre line, on top of whatever transform
        // is current (a scene may be offset in the window)
        let cx = (dest.left + dest.right) / 2.0;
        let mut m = D2D1_LAYER_PARAMETERS::default().maskTransform;
        unsafe { rt.GetTransform(&mut m) };
        let mut flipped = m;
        flipped.M11 = k * m.M11;
        flipped.M12 = k * m.M12;
        flipped.M31 = cx * (1.0 - k) * m.M11 + m.M31;
        flipped.M32 = cx * (1.0 - k) * m.M12 + m.M32;
        unsafe { rt.SetTransform(&flipped) };
        let drawn = self.draw_artwork(rt, &cover, dest, radius, scale, 1.0);
        unsafe { rt.SetTransform(&m) };
        drawn
    }

    /// Single-line text format with ellipsis trimming (never wraps, never overflows).
    fn create_line_format(
        &self,
        font: &UiFont,
        font_size: f32,
        weight: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT,
        alignment: DWRITE_TEXT_ALIGNMENT,
    ) -> Result<IDWriteTextFormat> {
        let format =
            self.create_text_format_in(font, font_size, weight, DWRITE_FONT_STYLE_NORMAL)?;
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
                timer_display: role(TextRole::ClockDisplay, DWRITE_TEXT_ALIGNMENT_LEADING)?,
                timer_status: role(TextRole::MediaMeta, DWRITE_TEXT_ALIGNMENT_LEADING)?,
                timer_slot: role(TextRole::ClockCompact, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
                clip_text: {
                    let f = role(TextRole::MediaArtist, DWRITE_TEXT_ALIGNMENT_LEADING)?;
                    unsafe {
                        f.SetWordWrapping(
                            windows::Win32::Graphics::DirectWrite::DWRITE_WORD_WRAPPING_WRAP,
                        )?;
                        f.SetParagraphAlignment(
                            windows::Win32::Graphics::DirectWrite::DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
                        )?;
                    }
                    f
                },
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
                OecaSpace::Home,
            );
            let home_radius =
                resolve_media_layout_in(&home, media.content.shape(), OecaSpace::Home)
                    .and_then(|l| l.artwork_bounds)
                    .map_or(0.0, |a| {
                        let gap = home.notch_height() - a.bottom;
                        (home.curvature.bottom_radius - gap + BASE_MEDIA_ARTWORK_RADIUS_EXTRA * s)
                            .max(0.0)
                    });
            let radius = home_radius + (BASE_MUSIC_ARTWORK_RADIUS * s - home_radius) * music;
            if self.flip.borrow().is_some() {
                // Flipping over to the new cover (instead of its fade)
                self.draw_artwork_turn(rt, dest, radius, s)?;
            } else {
                self.draw_artwork(rt, artwork, dest, radius, s, track_alpha)?;
            }
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

        // Home's timer in the band between the divider and the clock column
        // (leaves with the other Home-only parts)
        if home_alpha > 0.0 {
            let home = space_dimensions(
                crate::config::NotchState::Expanded,
                dimensions.dpi,
                OecaSpace::Home,
            );
            if let Some(h) =
                crate::layout::resolve_home_timer_layout(&home, Some(media.content.shape()))
            {
                let dx = ((dimensions.width - home.width) as f32 / 2.0).round();
                self.draw_timer_panel(rt, &h.timer.offset_x(dx), &home, home_alpha)?;
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
            // Smooth press curve: eased compression and spring back, applied
            // as a scale about the glyph's centre, so its whole-pixel
            // geometry is the same pressed or not, moving or at rest (no
            // snapping pop when a press or a morph ends)
            let eased = press * press * (3.0 - 2.0 * press);
            let u = (base * s).round();
            let k = 1.0 - (1.0 - MEDIA_PRESS_ICON_SCALE) * eased;
            unsafe {
                icon_brush.SetOpacity(
                    MEDIA_ICON_REST_OPACITY + (1.0 - MEDIA_ICON_REST_OPACITY) * hover.max(press),
                );
            }
            let icons = self.icons.borrow();
            if control == MediaControl::PlayPause {
                self.draw_play_pause(rt, cx, cy, u, icons.morph(), k, &icon_brush)?;
                continue;
            }
            let phase = icons.skip_phase(control);
            if phase <= 0.0 {
                let shapes = icon_shapes(control, media.content.icon, cx, cy, u, true);
                self.fill_shapes(rt, &squeeze_shapes(shapes, cx, cy, k), &icon_brush)?;
                continue;
            }
            // A skip: the glyph rolls one step in its direction (see
            // `skip_morph`), clipped to the button, in one opacity layer so
            // its overlapping triangles never brighten
            let base = unsafe { icon_brush.GetOpacity() };
            unsafe {
                icon_brush.SetOpacity(1.0);
                rt.PushAxisAlignedClip(&v.to_d2d_rect(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            }
            let drawn = self.with_layer(rt, None, base.min(0.999), || {
                for tri in skip_morph(control, cx, cy, u, phase) {
                    self.fill_triangle(rt, tri.map(|p| squeeze(p, cx, cy, k)), &icon_brush)?;
                }
                Ok(())
            });
            unsafe {
                rt.PopAxisAlignedClip();
                icon_brush.SetOpacity(base);
            }
            drawn?;
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

#[cfg(test)]
impl Renderer {
    /// The shared surface as last drawn (notch or bubble): rows of `h`, and
    /// the stride.
    pub(crate) fn surface(&self, h: i32) -> (Vec<u32>, usize) {
        let stride = self.cached_capacity_w.get() as usize;
        let bits = self.cached_bits.get() as *const u32;
        let pixels = unsafe { std::slice::from_raw_parts(bits, stride * h as usize) };
        (pixels.to_vec(), stride)
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.cleanup_cached_resources();
    }
}

/// One step of an icon outline in a unit box (-0.5..0.5, y down): move, line,
/// cubic Bezier (two controls, end), close. Outlines fill with the nonzero
/// (winding) rule (`fill_glyph`).
#[derive(Clone, Copy)]
pub(crate) enum IconSeg {
    M(f32, f32),
    L(f32, f32),
    C(f32, f32, f32, f32, f32, f32),
    Z,
}

// Icons: Oeca's own, made by `assets/icons/generate.py` (edit and re-run it
// rather than these numbers): rounded polygons, circles and arcs on a 24 x 24
// grid, and Ujval's own Copy icon from its SVG path; each scaled to the box it
// occupies. Solid shapes and holes run in opposite directions: filled with the
// nonzero rule, overlaps merge and holes cut out.
/// Home: a soft-roofed house with an arched door; the Home space.
const HOUSE_BLANK: &[IconSeg] = &[
    IconSeg::M(0.0900, -0.4521),
    IconSeg::L(0.4625, -0.1444),
    IconSeg::C(0.4863, -0.1248, 0.5000, -0.0956, 0.5000, -0.0648),
    IconSeg::L(0.5000, 0.3540),
    IconSeg::C(0.5000, 0.4320, 0.4367, 0.4953, 0.3587, 0.4953),
    IconSeg::L(0.2011, 0.4953),
    IconSeg::C(0.1681, 0.4953, 0.1413, 0.4685, 0.1413, 0.4355),
    IconSeg::L(0.1413, 0.2942),
    IconSeg::C(0.1413, 0.2312, 0.0902, 0.1801, 0.0272, 0.1801),
    IconSeg::L(-0.0272, 0.1801),
    IconSeg::C(-0.0902, 0.1801, -0.1413, 0.2312, -0.1413, 0.2942),
    IconSeg::L(-0.1413, 0.4355),
    IconSeg::C(-0.1413, 0.4685, -0.1681, 0.4953, -0.2011, 0.4953),
    IconSeg::L(-0.3587, 0.4953),
    IconSeg::C(-0.4367, 0.4953, -0.5000, 0.4320, -0.5000, 0.3540),
    IconSeg::L(-0.5000, -0.0648),
    IconSeg::C(-0.5000, -0.0956, -0.4863, -0.1248, -0.4625, -0.1444),
    IconSeg::L(-0.0900, -0.4521),
    IconSeg::C(-0.0378, -0.4953, 0.0378, -0.4953, 0.0900, -0.4521),
    IconSeg::Z,
];
/// Music: two beamed notes; the Music space.
pub(crate) const MUSIC_ALT: &[IconSeg] = &[
    IconSeg::M(-0.1000, 0.2895),
    IconSeg::C(-0.1000, 0.3999, -0.1895, 0.4895, -0.3000, 0.4895),
    IconSeg::C(-0.4105, 0.4895, -0.5000, 0.3999, -0.5000, 0.2895),
    IconSeg::C(-0.5000, 0.1790, -0.4105, 0.0895, -0.3000, 0.0895),
    IconSeg::C(-0.1895, 0.0895, -0.1000, 0.1790, -0.1000, 0.2895),
    IconSeg::Z,
    IconSeg::M(0.5000, 0.1450),
    IconSeg::C(0.5000, 0.2555, 0.4105, 0.3450, 0.3000, 0.3450),
    IconSeg::C(0.1895, 0.3450, 0.1000, 0.2555, 0.1000, 0.1450),
    IconSeg::C(0.1000, 0.0346, 0.1895, -0.0550, 0.3000, -0.0550),
    IconSeg::C(0.4105, -0.0550, 0.5000, 0.0346, 0.5000, 0.1450),
    IconSeg::Z,
    IconSeg::M(-0.1556, -0.2994),
    IconSeg::L(-0.1500, -0.2994),
    IconSeg::C(-0.1224, -0.2994, -0.1000, -0.2770, -0.1000, -0.2494),
    IconSeg::L(-0.1000, 0.2395),
    IconSeg::C(-0.1000, 0.2671, -0.1224, 0.2895, -0.1500, 0.2895),
    IconSeg::L(-0.1556, 0.2895),
    IconSeg::C(-0.1832, 0.2895, -0.2056, 0.2671, -0.2056, 0.2395),
    IconSeg::L(-0.2056, -0.2494),
    IconSeg::C(-0.2056, -0.2770, -0.1832, -0.2994, -0.1556, -0.2994),
    IconSeg::Z,
    IconSeg::M(0.4444, -0.4550),
    IconSeg::L(0.4500, -0.4550),
    IconSeg::C(0.4776, -0.4550, 0.5000, -0.4326, 0.5000, -0.4050),
    IconSeg::L(0.5000, 0.0950),
    IconSeg::C(0.5000, 0.1227, 0.4776, 0.1450, 0.4500, 0.1450),
    IconSeg::L(0.4444, 0.1450),
    IconSeg::C(0.4168, 0.1450, 0.3944, 0.1227, 0.3944, 0.0950),
    IconSeg::L(0.3944, -0.4050),
    IconSeg::C(0.3944, -0.4326, 0.4168, -0.4550, 0.4444, -0.4550),
    IconSeg::Z,
    IconSeg::M(-0.1544, -0.3280),
    IconSeg::L(0.4234, -0.4793),
    IconSeg::C(0.4621, -0.4895, 0.5000, -0.4603, 0.5000, -0.4202),
    IconSeg::L(0.5000, -0.3299),
    IconSeg::C(0.5000, -0.3021, 0.4813, -0.2778, 0.4544, -0.2708),
    IconSeg::L(-0.1234, -0.1195),
    IconSeg::C(-0.1621, -0.1093, -0.2000, -0.1385, -0.2000, -0.1786),
    IconSeg::L(-0.2000, -0.2689),
    IconSeg::C(-0.2000, -0.2967, -0.1813, -0.3210, -0.1544, -0.3280),
    IconSeg::Z,
];
/// Clipboard: a board and its clip; the Clipboard space.
pub(crate) const CLIPBOARD: &[IconSeg] = &[
    IconSeg::M(-0.2647, -0.3627),
    IconSeg::L(-0.3088, -0.3627),
    IconSeg::C(-0.2845, -0.3627, -0.2647, -0.3430, -0.2647, -0.3186),
    IconSeg::L(-0.2647, -0.2451),
    IconSeg::C(-0.2647, -0.2018, -0.2296, -0.1667, -0.1863, -0.1667),
    IconSeg::L(0.1863, -0.1667),
    IconSeg::C(0.2296, -0.1667, 0.2647, -0.2018, 0.2647, -0.2451),
    IconSeg::L(0.2647, -0.3186),
    IconSeg::C(0.2647, -0.3430, 0.2845, -0.3627, 0.3088, -0.3627),
    IconSeg::L(0.2647, -0.3627),
    IconSeg::C(0.3459, -0.3627, 0.4118, -0.2969, 0.4118, -0.2157),
    IconSeg::L(0.4118, 0.3529),
    IconSeg::C(0.4118, 0.4344, 0.3459, 0.5000, 0.2647, 0.5000),
    IconSeg::L(-0.2647, 0.5000),
    IconSeg::C(-0.3459, 0.5000, -0.4118, 0.4344, -0.4118, 0.3529),
    IconSeg::L(-0.4118, -0.2157),
    IconSeg::C(-0.4118, -0.2969, -0.3459, -0.3627, -0.2647, -0.3627),
    IconSeg::Z,
    IconSeg::M(-0.0686, -0.5000),
    IconSeg::L(0.0686, -0.5000),
    IconSeg::C(0.1174, -0.5000, 0.1569, -0.4605, 0.1569, -0.4118),
    IconSeg::L(0.1569, -0.3529),
    IconSeg::C(0.1569, -0.3042, 0.1174, -0.2647, 0.0686, -0.2647),
    IconSeg::L(-0.0686, -0.2647),
    IconSeg::C(-0.1174, -0.2647, -0.1569, -0.3042, -0.1569, -0.3529),
    IconSeg::L(-0.1569, -0.4118),
    IconSeg::C(-0.1569, -0.4605, -0.1174, -0.5000, -0.0686, -0.5000),
    IconSeg::Z,
];
/// Palette: three paint wells and a thumb notch; Settings window, Appearance.
pub(crate) const PALETTE: &[IconSeg] = &[
    IconSeg::M(0.1376, 0.4790),
    IconSeg::L(0.1219, 0.4850),
    IconSeg::L(0.0935, 0.4912),
    IconSeg::L(0.0649, 0.4958),
    IconSeg::L(0.0360, 0.4988),
    IconSeg::L(0.0070, 0.5000),
    IconSeg::L(-0.0221, 0.4996),
    IconSeg::L(-0.0510, 0.4974),
    IconSeg::L(-0.0798, 0.4936),
    IconSeg::L(-0.1083, 0.4882),
    IconSeg::L(-0.1365, 0.4811),
    IconSeg::L(-0.1641, 0.4723),
    IconSeg::L(-0.1913, 0.4620),
    IconSeg::L(-0.2178, 0.4501),
    IconSeg::L(-0.2435, 0.4367),
    IconSeg::L(-0.2685, 0.4219),
    IconSeg::L(-0.2925, 0.4056),
    IconSeg::L(-0.3155, 0.3879),
    IconSeg::L(-0.3375, 0.3689),
    IconSeg::L(-0.3583, 0.3487),
    IconSeg::L(-0.3780, 0.3274),
    IconSeg::L(-0.3963, 0.3049),
    IconSeg::L(-0.4134, 0.2814),
    IconSeg::L(-0.4290, 0.2569),
    IconSeg::L(-0.4432, 0.2316),
    IconSeg::L(-0.4559, 0.2054),
    IconSeg::L(-0.4670, 0.1786),
    IconSeg::L(-0.4766, 0.1512),
    IconSeg::L(-0.4846, 0.1233),
    IconSeg::L(-0.4909, 0.0950),
    IconSeg::L(-0.4956, 0.0663),
    IconSeg::L(-0.4986, 0.0375),
    IconSeg::L(-0.5000, 0.0085),
    IconSeg::L(-0.4996, -0.0206),
    IconSeg::L(-0.4976, -0.0495),
    IconSeg::L(-0.4939, -0.0783),
    IconSeg::L(-0.4885, -0.1068),
    IconSeg::L(-0.4815, -0.1350),
    IconSeg::L(-0.4728, -0.1627),
    IconSeg::L(-0.4626, -0.1899),
    IconSeg::L(-0.4508, -0.2164),
    IconSeg::L(-0.4375, -0.2422),
    IconSeg::L(-0.4227, -0.2672),
    IconSeg::L(-0.4065, -0.2913),
    IconSeg::L(-0.3889, -0.3144),
    IconSeg::L(-0.3700, -0.3364),
    IconSeg::L(-0.3498, -0.3573),
    IconSeg::L(-0.3285, -0.3770),
    IconSeg::L(-0.3061, -0.3955),
    IconSeg::L(-0.2826, -0.4125),
    IconSeg::L(-0.2582, -0.4283),
    IconSeg::L(-0.2329, -0.4425),
    IconSeg::L(-0.2069, -0.4553),
    IconSeg::L(-0.1801, -0.4665),
    IconSeg::L(-0.1527, -0.4762),
    IconSeg::L(-0.1248, -0.4843),
    IconSeg::L(-0.0965, -0.4907),
    IconSeg::L(-0.0679, -0.4955),
    IconSeg::L(-0.0390, -0.4986),
    IconSeg::L(-0.0100, -0.5000),
    IconSeg::L(0.0190, -0.4997),
    IconSeg::L(0.0480, -0.4978),
    IconSeg::L(0.0768, -0.4942),
    IconSeg::L(0.1053, -0.4889),
    IconSeg::L(0.1335, -0.4820),
    IconSeg::L(0.1613, -0.4734),
    IconSeg::L(0.1885, -0.4632),
    IconSeg::L(0.2150, -0.4515),
    IconSeg::L(0.2409, -0.4383),
    IconSeg::L(0.2659, -0.4236),
    IconSeg::L(0.2900, -0.4074),
    IconSeg::L(0.3132, -0.3899),
    IconSeg::L(0.3353, -0.3711),
    IconSeg::L(0.3562, -0.3510),
    IconSeg::L(0.3760, -0.3298),
    IconSeg::L(0.3945, -0.3074),
    IconSeg::L(0.4117, -0.2840),
    IconSeg::L(0.4275, -0.2596),
    IconSeg::L(0.4418, -0.2344),
    IconSeg::L(0.4547, -0.2083),
    IconSeg::L(0.4660, -0.1816),
    IconSeg::L(0.4757, -0.1543),
    IconSeg::L(0.4839, -0.1264),
    IconSeg::L(0.4904, -0.0981),
    IconSeg::L(0.4953, -0.0695),
    IconSeg::L(0.4985, -0.0406),
    IconSeg::L(0.5000, -0.0116),
    IconSeg::L(0.4998, 0.0174),
    IconSeg::L(0.4979, 0.0464),
    IconSeg::L(0.4944, 0.0752),
    IconSeg::L(0.4900, 0.0914),
    IconSeg::C(0.4773, 0.1376, 0.4224, 0.1570, 0.3836, 0.1289),
    IconSeg::L(0.3815, 0.1274),
    IconSeg::L(0.3567, 0.1208),
    IconSeg::L(0.3311, 0.1178),
    IconSeg::L(0.3054, 0.1186),
    IconSeg::L(0.2801, 0.1231),
    IconSeg::L(0.2557, 0.1312),
    IconSeg::L(0.2327, 0.1429),
    IconSeg::L(0.2117, 0.1577),
    IconSeg::L(0.1930, 0.1754),
    IconSeg::L(0.1771, 0.1957),
    IconSeg::L(0.1644, 0.2180),
    IconSeg::L(0.1550, 0.2420),
    IconSeg::L(0.1492, 0.2670),
    IconSeg::L(0.1471, 0.2927),
    IconSeg::L(0.1488, 0.3184),
    IconSeg::L(0.1542, 0.3435),
    IconSeg::L(0.1632, 0.3677),
    IconSeg::L(0.1648, 0.3695),
    IconSeg::C(0.1964, 0.4055, 0.1824, 0.4620, 0.1376, 0.4790),
    IconSeg::Z,
    IconSeg::M(-0.1127, -0.0785),
    IconSeg::C(-0.1127, -0.1353, -0.1588, -0.1814, -0.2157, -0.1814),
    IconSeg::C(-0.2725, -0.1814, -0.3186, -0.1353, -0.3186, -0.0785),
    IconSeg::C(-0.3186, -0.0216, -0.2725, 0.0245, -0.2157, 0.0245),
    IconSeg::C(-0.1588, 0.0245, -0.1127, -0.0216, -0.1127, -0.0785),
    IconSeg::Z,
    IconSeg::M(0.1226, -0.2501),
    IconSeg::C(0.1226, -0.3069, 0.0765, -0.3530, 0.0196, -0.3530),
    IconSeg::C(-0.0372, -0.3530, -0.0833, -0.3069, -0.0833, -0.2501),
    IconSeg::C(-0.0833, -0.1932, -0.0372, -0.1471, 0.0196, -0.1471),
    IconSeg::C(0.0765, -0.1471, 0.1226, -0.1932, 0.1226, -0.2501),
    IconSeg::Z,
    IconSeg::M(0.3481, -0.1177),
    IconSeg::C(0.3481, -0.1746, 0.3020, -0.2206, 0.2452, -0.2206),
    IconSeg::C(0.1883, -0.2206, 0.1422, -0.1746, 0.1422, -0.1177),
    IconSeg::C(0.1422, -0.0608, 0.1883, -0.0147, 0.2452, -0.0147),
    IconSeg::C(0.3020, -0.0147, 0.3481, -0.0608, 0.3481, -0.1177),
    IconSeg::Z,
    IconSeg::M(-0.0980, 0.1814),
    IconSeg::C(-0.0980, 0.1245, -0.1441, 0.0784, -0.2010, 0.0784),
    IconSeg::C(-0.2578, 0.0784, -0.3039, 0.1245, -0.3039, 0.1814),
    IconSeg::C(-0.3039, 0.2382, -0.2578, 0.2843, -0.2010, 0.2843),
    IconSeg::C(-0.1441, 0.2843, -0.0980, 0.2382, -0.0980, 0.1814),
    IconSeg::Z,
];
/// Info: an "i" cut from a disc; Settings window, About.
pub(crate) const INFO: &[IconSeg] = &[
    IconSeg::M(0.5000, 0.0000),
    IconSeg::C(0.5000, 0.2761, 0.2761, 0.5000, 0.0000, 0.5000),
    IconSeg::C(-0.2761, 0.5000, -0.5000, 0.2761, -0.5000, 0.0000),
    IconSeg::C(-0.5000, -0.2761, -0.2761, -0.5000, 0.0000, -0.5000),
    IconSeg::C(0.2761, -0.5000, 0.5000, -0.2761, 0.5000, 0.0000),
    IconSeg::Z,
    IconSeg::M(0.0784, -0.2255),
    IconSeg::C(0.0784, -0.2688, 0.0433, -0.3039, 0.0000, -0.3039),
    IconSeg::C(-0.0433, -0.3039, -0.0784, -0.2688, -0.0784, -0.2255),
    IconSeg::C(-0.0784, -0.1822, -0.0433, -0.1471, 0.0000, -0.1471),
    IconSeg::C(0.0433, -0.1471, 0.0784, -0.1822, 0.0784, -0.2255),
    IconSeg::Z,
    IconSeg::M(0.0000, 0.3137),
    IconSeg::L(0.0000, 0.3137),
    IconSeg::C(0.0379, 0.3137, 0.0686, 0.2830, 0.0686, 0.2451),
    IconSeg::L(0.0686, -0.0098),
    IconSeg::C(0.0686, -0.0477, 0.0379, -0.0784, 0.0000, -0.0784),
    IconSeg::L(0.0000, -0.0784),
    IconSeg::C(-0.0379, -0.0784, -0.0686, -0.0477, -0.0686, -0.0098),
    IconSeg::L(-0.0686, 0.2451),
    IconSeg::C(-0.0686, 0.2830, -0.0379, 0.3137, 0.0000, 0.3137),
    IconSeg::Z,
];
/// Settings: an eight-tooth gear; the header's Settings icon.
pub(crate) const GEAR: &[IconSeg] = &[
    IconSeg::M(-0.1110, -0.3862),
    IconSeg::L(-0.0988, -0.4618),
    IconSeg::C(-0.0953, -0.4838, -0.0763, -0.5000, -0.0540, -0.5000),
    IconSeg::L(0.0540, -0.5000),
    IconSeg::C(0.0763, -0.5000, 0.0953, -0.4838, 0.0988, -0.4618),
    IconSeg::L(0.1110, -0.3862),
    IconSeg::C(0.1133, -0.3723, 0.1227, -0.3604, 0.1358, -0.3550),
    IconSeg::L(0.1550, -0.3470),
    IconSeg::C(0.1681, -0.3416, 0.1831, -0.3433, 0.1946, -0.3516),
    IconSeg::L(0.2567, -0.3965),
    IconSeg::C(0.2747, -0.4095, 0.2996, -0.4075, 0.3154, -0.3917),
    IconSeg::L(0.3917, -0.3154),
    IconSeg::C(0.4075, -0.2996, 0.4095, -0.2747, 0.3965, -0.2567),
    IconSeg::L(0.3516, -0.1946),
    IconSeg::C(0.3433, -0.1831, 0.3416, -0.1681, 0.3470, -0.1550),
    IconSeg::L(0.3550, -0.1358),
    IconSeg::C(0.3604, -0.1227, 0.3723, -0.1133, 0.3862, -0.1110),
    IconSeg::L(0.4618, -0.0988),
    IconSeg::C(0.4838, -0.0953, 0.5000, -0.0763, 0.5000, -0.0540),
    IconSeg::L(0.5000, 0.0540),
    IconSeg::C(0.5000, 0.0763, 0.4838, 0.0953, 0.4618, 0.0988),
    IconSeg::L(0.3862, 0.1110),
    IconSeg::C(0.3723, 0.1133, 0.3604, 0.1227, 0.3550, 0.1358),
    IconSeg::L(0.3470, 0.1550),
    IconSeg::C(0.3416, 0.1681, 0.3433, 0.1831, 0.3516, 0.1946),
    IconSeg::L(0.3965, 0.2567),
    IconSeg::C(0.4095, 0.2747, 0.4075, 0.2996, 0.3917, 0.3154),
    IconSeg::L(0.3154, 0.3917),
    IconSeg::C(0.2996, 0.4075, 0.2747, 0.4095, 0.2567, 0.3965),
    IconSeg::L(0.1946, 0.3516),
    IconSeg::C(0.1831, 0.3433, 0.1681, 0.3416, 0.1550, 0.3470),
    IconSeg::L(0.1358, 0.3550),
    IconSeg::C(0.1227, 0.3604, 0.1133, 0.3723, 0.1110, 0.3862),
    IconSeg::L(0.0988, 0.4618),
    IconSeg::C(0.0953, 0.4838, 0.0763, 0.5000, 0.0540, 0.5000),
    IconSeg::L(-0.0540, 0.5000),
    IconSeg::C(-0.0763, 0.5000, -0.0953, 0.4838, -0.0988, 0.4618),
    IconSeg::L(-0.1110, 0.3862),
    IconSeg::C(-0.1133, 0.3723, -0.1227, 0.3604, -0.1358, 0.3550),
    IconSeg::L(-0.1550, 0.3470),
    IconSeg::C(-0.1681, 0.3416, -0.1831, 0.3433, -0.1946, 0.3516),
    IconSeg::L(-0.2567, 0.3965),
    IconSeg::C(-0.2747, 0.4095, -0.2996, 0.4075, -0.3154, 0.3917),
    IconSeg::L(-0.3917, 0.3154),
    IconSeg::C(-0.4075, 0.2996, -0.4095, 0.2747, -0.3965, 0.2567),
    IconSeg::L(-0.3516, 0.1946),
    IconSeg::C(-0.3433, 0.1831, -0.3416, 0.1681, -0.3470, 0.1550),
    IconSeg::L(-0.3550, 0.1358),
    IconSeg::C(-0.3604, 0.1227, -0.3723, 0.1133, -0.3862, 0.1110),
    IconSeg::L(-0.4618, 0.0988),
    IconSeg::C(-0.4838, 0.0953, -0.5000, 0.0763, -0.5000, 0.0540),
    IconSeg::L(-0.5000, -0.0540),
    IconSeg::C(-0.5000, -0.0763, -0.4838, -0.0953, -0.4618, -0.0988),
    IconSeg::L(-0.3862, -0.1110),
    IconSeg::C(-0.3723, -0.1133, -0.3604, -0.1227, -0.3550, -0.1358),
    IconSeg::L(-0.3470, -0.1550),
    IconSeg::C(-0.3416, -0.1681, -0.3433, -0.1831, -0.3516, -0.1946),
    IconSeg::L(-0.3965, -0.2567),
    IconSeg::C(-0.4095, -0.2747, -0.4075, -0.2996, -0.3917, -0.3154),
    IconSeg::L(-0.3154, -0.3917),
    IconSeg::C(-0.2996, -0.4075, -0.2747, -0.4095, -0.2567, -0.3965),
    IconSeg::L(-0.1946, -0.3516),
    IconSeg::C(-0.1831, -0.3433, -0.1681, -0.3416, -0.1550, -0.3470),
    IconSeg::L(-0.1358, -0.3550),
    IconSeg::C(-0.1227, -0.3604, -0.1133, -0.3723, -0.1110, -0.3862),
    IconSeg::Z,
    IconSeg::M(0.1544, 0.0000),
    IconSeg::C(0.1544, -0.0853, 0.0853, -0.1544, 0.0000, -0.1544),
    IconSeg::C(-0.0853, -0.1544, -0.1544, -0.0853, -0.1544, 0.0000),
    IconSeg::C(-0.1544, 0.0853, -0.0853, 0.1544, 0.0000, 0.1544),
    IconSeg::C(0.0853, 0.1544, 0.1544, 0.0853, 0.1544, 0.0000),
    IconSeg::Z,
];
/// Copy: a sheet in front of an outlined one; clipboard row copy button.
const COPY: &[IconSeg] = &[
    IconSeg::M(0.3274, -0.4990),
    IconSeg::C(0.4254, -0.4889, 0.5000, -0.4063, 0.5000, -0.3077),
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
/// Trash: a lid over a solid bin; clipboard row delete button.
const TRASH: &[IconSeg] = &[
    IconSeg::M(-0.3235, -0.3725),
    IconSeg::L(0.3235, -0.3725),
    IconSeg::C(0.3614, -0.3725, 0.3922, -0.3418, 0.3922, -0.3039),
    IconSeg::L(0.3922, -0.3039),
    IconSeg::C(0.3922, -0.2660, 0.3614, -0.2353, 0.3235, -0.2353),
    IconSeg::L(-0.3235, -0.2353),
    IconSeg::C(-0.3614, -0.2353, -0.3922, -0.2660, -0.3922, -0.3039),
    IconSeg::L(-0.3922, -0.3039),
    IconSeg::C(-0.3922, -0.3418, -0.3614, -0.3725, -0.3235, -0.3725),
    IconSeg::Z,
    IconSeg::M(-0.0882, -0.5000),
    IconSeg::L(0.0882, -0.5000),
    IconSeg::C(0.1261, -0.5000, 0.1569, -0.4693, 0.1569, -0.4314),
    IconSeg::L(0.1569, -0.3922),
    IconSeg::C(0.1569, -0.3543, 0.1261, -0.3235, 0.0882, -0.3235),
    IconSeg::L(-0.0882, -0.3235),
    IconSeg::C(-0.1261, -0.3235, -0.1569, -0.3543, -0.1569, -0.3922),
    IconSeg::L(-0.1569, -0.4314),
    IconSeg::C(-0.1569, -0.4693, -0.1261, -0.5000, -0.0882, -0.5000),
    IconSeg::Z,
    IconSeg::M(-0.3039, -0.1373),
    IconSeg::L(0.3039, -0.1373),
    IconSeg::C(0.3310, -0.1373, 0.3529, -0.1153, 0.3529, -0.0882),
    IconSeg::L(0.3529, 0.3824),
    IconSeg::C(0.3529, 0.4473, 0.3003, 0.5000, 0.2353, 0.5000),
    IconSeg::L(-0.2353, 0.5000),
    IconSeg::C(-0.3003, 0.5000, -0.3529, 0.4473, -0.3529, 0.3824),
    IconSeg::L(-0.3529, -0.0882),
    IconSeg::C(-0.3529, -0.1153, -0.3310, -0.1373, -0.3039, -0.1373),
    IconSeg::Z,
];
/// Tray: an open tray with a dipped front; the drop page glyph.
const INBOX: &[IconSeg] = &[
    IconSeg::M(-0.2786, -0.3476),
    IconSeg::L(0.2786, -0.3476),
    IconSeg::C(0.3125, -0.3476, 0.3438, -0.3296, 0.3609, -0.3004),
    IconSeg::L(0.4935, -0.0730),
    IconSeg::C(0.4978, -0.0657, 0.5000, -0.0575, 0.5000, -0.0490),
    IconSeg::L(0.5000, 0.2333),
    IconSeg::C(0.5000, 0.2965, 0.4488, 0.3476, 0.3857, 0.3476),
    IconSeg::L(-0.3857, 0.3476),
    IconSeg::C(-0.4488, 0.3476, -0.5000, 0.2965, -0.5000, 0.2333),
    IconSeg::L(-0.5000, -0.0490),
    IconSeg::C(-0.5000, -0.0575, -0.4978, -0.0657, -0.4935, -0.0730),
    IconSeg::L(-0.3609, -0.3004),
    IconSeg::C(-0.3438, -0.3296, -0.3125, -0.3476, -0.2786, -0.3476),
    IconSeg::Z,
    IconSeg::M(-0.3364, -0.0905),
    IconSeg::L(-0.1763, -0.0905),
    IconSeg::C(-0.1566, -0.0905, -0.1395, -0.0771, -0.1347, -0.0580),
    IconSeg::L(-0.1260, -0.0231),
    IconSeg::L(-0.1188, -0.0111),
    IconSeg::L(-0.1104, 0.0002),
    IconSeg::L(-0.1010, 0.0105),
    IconSeg::L(-0.0906, 0.0200),
    IconSeg::L(-0.0794, 0.0283),
    IconSeg::L(-0.0673, 0.0355),
    IconSeg::L(-0.0547, 0.0415),
    IconSeg::L(-0.0415, 0.0462),
    IconSeg::L(-0.0279, 0.0496),
    IconSeg::L(-0.0140, 0.0517),
    IconSeg::L(0.0000, 0.0524),
    IconSeg::L(0.0140, 0.0517),
    IconSeg::L(0.0279, 0.0496),
    IconSeg::L(0.0415, 0.0462),
    IconSeg::L(0.0547, 0.0415),
    IconSeg::L(0.0673, 0.0355),
    IconSeg::L(0.0794, 0.0283),
    IconSeg::L(0.0906, 0.0200),
    IconSeg::L(0.1010, 0.0105),
    IconSeg::L(0.1104, 0.0002),
    IconSeg::L(0.1188, -0.0111),
    IconSeg::L(0.1260, -0.0231),
    IconSeg::L(0.1347, -0.0580),
    IconSeg::C(0.1395, -0.0771, 0.1566, -0.0905, 0.1763, -0.0905),
    IconSeg::L(0.3364, -0.0905),
    IconSeg::C(0.3563, -0.0905, 0.3674, -0.1135, 0.3550, -0.1291),
    IconSeg::L(0.2672, -0.2398),
    IconSeg::C(0.2608, -0.2477, 0.2512, -0.2524, 0.2410, -0.2524),
    IconSeg::L(-0.2410, -0.2524),
    IconSeg::C(-0.2512, -0.2524, -0.2608, -0.2477, -0.2672, -0.2398),
    IconSeg::L(-0.3550, -0.1291),
    IconSeg::C(-0.3674, -0.1135, -0.3563, -0.0905, -0.3364, -0.0905),
    IconSeg::Z,
];
/// Done: a check cut from a disc; a drop's success.
const DONE: &[IconSeg] = &[
    IconSeg::M(0.5000, 0.0000),
    IconSeg::C(0.5000, 0.2761, 0.2761, 0.5000, 0.0000, 0.5000),
    IconSeg::C(-0.2761, 0.5000, -0.5000, 0.2761, -0.5000, 0.0000),
    IconSeg::C(-0.5000, -0.2761, -0.2761, -0.5000, 0.0000, -0.5000),
    IconSeg::C(0.2761, -0.5000, 0.5000, -0.2761, 0.5000, 0.0000),
    IconSeg::Z,
    IconSeg::M(-0.2362, 0.1186),
    IconSeg::L(-0.0901, 0.2647),
    IconSeg::C(-0.0773, 0.2775, -0.0563, 0.2771, -0.0441, 0.2637),
    IconSeg::L(0.2500, -0.0580),
    IconSeg::C(0.2733, -0.0834, 0.2715, -0.1229, 0.2460, -0.1462),
    IconSeg::L(0.2442, -0.1479),
    IconSeg::C(0.2187, -0.1712, 0.1792, -0.1694, 0.1559, -0.1440),
    IconSeg::L(-0.0482, 0.0793),
    IconSeg::C(-0.0605, 0.0927, -0.0814, 0.0932, -0.0943, 0.0803),
    IconSeg::L(-0.1461, 0.0285),
    IconSeg::C(-0.1705, 0.0041, -0.2100, 0.0041, -0.2344, 0.0285),
    IconSeg::L(-0.2362, 0.0303),
    IconSeg::C(-0.2606, 0.0547, -0.2606, 0.0942, -0.2362, 0.1186),
    IconSeg::Z,
];
/// Alert: an exclamation mark cut from a disc; a drop that failed.
const ALERT: &[IconSeg] = &[
    IconSeg::M(0.5000, 0.0000),
    IconSeg::C(0.5000, 0.2761, 0.2761, 0.5000, 0.0000, 0.5000),
    IconSeg::C(-0.2761, 0.5000, -0.5000, 0.2761, -0.5000, 0.0000),
    IconSeg::C(-0.5000, -0.2761, -0.2761, -0.5000, 0.0000, -0.5000),
    IconSeg::C(0.2761, -0.5000, 0.5000, -0.2761, 0.5000, 0.0000),
    IconSeg::Z,
    IconSeg::M(0.0000, 0.0784),
    IconSeg::L(0.0000, 0.0784),
    IconSeg::C(0.0379, 0.0784, 0.0686, 0.0477, 0.0686, 0.0098),
    IconSeg::L(0.0686, -0.2451),
    IconSeg::C(0.0686, -0.2830, 0.0379, -0.3137, 0.0000, -0.3137),
    IconSeg::L(0.0000, -0.3137),
    IconSeg::C(-0.0379, -0.3137, -0.0686, -0.2830, -0.0686, -0.2451),
    IconSeg::L(-0.0686, 0.0098),
    IconSeg::C(-0.0686, 0.0477, -0.0379, 0.0784, 0.0000, 0.0784),
    IconSeg::Z,
    IconSeg::M(0.0784, 0.2451),
    IconSeg::C(0.0784, 0.2018, 0.0433, 0.1667, 0.0000, 0.1667),
    IconSeg::C(-0.0433, 0.1667, -0.0784, 0.2018, -0.0784, 0.2451),
    IconSeg::C(-0.0784, 0.2884, -0.0433, 0.3235, 0.0000, 0.3235),
    IconSeg::C(0.0433, 0.3235, 0.0784, 0.2884, 0.0784, 0.2451),
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

    /// A rendered frame (top-down premultiplied BGRA, `w` x `h`) shows the
    /// notch: its opaque black body at the top centre and nothing outside its
    /// rounded bottom-left corner.
    fn assert_notch_frame(px: &[u8], w: i32, h: i32, what: &str) {
        let at = |x: i32, y: i32| &px[((y * w + x) * 4) as usize..][..4];
        assert_eq!(at(w / 2, 1), [0, 0, 0, 255], "{what}: notch body");
        assert_eq!(at(0, h - 1)[3], 0, "{what}: transparent outside the corner");
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

            assert_notch_frame(pixel_slice_exp, w_exp, h_exp, "hovered expanded");

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

            assert_notch_frame(pixel_slice, w_exp, h_exp, "expanding frame");

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

            assert_notch_frame(pixel_slice_col, w_col, h_col, "collapsing frame");

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
                // Long text never reaches the timer in the band before the
                // clock column (the title row ends earlier, at the divider)
                let timer = crate::layout::resolve_home_timer_layout(&dims, Some(c.shape()))
                    .unwrap()
                    .timer;
                let gap = RectF::new(
                    m.artist_bounds.right + 1.0,
                    m.title_bounds.top,
                    timer.panel.left - 1.0,
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
        // The track's text starts dimmed (the cover turns instead: see the
        // artwork turn tests)
        let title = m.title_bounds;
        let brightest = |px: &[u32]| {
            (title.top as usize..title.bottom as usize)
                .flat_map(|y| (title.left as usize..title.right as usize).map(move |x| (x, y)))
                .map(|(x, y)| px[y * stride + x] & 0xFF)
                .max()
                .unwrap()
        };
        assert!(
            brightest(&fading) < brightest(&settled),
            "text starts dimmed"
        );
        // The cover turns, unless the system disables motion (then it swaps)
        assert_eq!(
            renderer.flip.borrow().is_some(),
            crate::window::is_client_animation_enabled(),
            "the cover turns"
        );
        renderer.settle_media_motion();
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

    /// The shadow mask exactly as it was built before the blur and shaping
    /// were optimized (division per pixel, column-by-column vertical pass,
    /// every weight per pixel): the reference the fast version must match.
    mod reference_shadow {
        use super::super::{
            SHADOW_BLUR_RADIUS, SHADOW_EDGE_FADE, SHADOW_OFFSET_Y, SHADOW_SHOULDER_FADE,
            SHADOW_STRENGTH,
        };

        #[allow(clippy::too_many_arguments)]
        pub(super) fn build_shadow_mask(
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
                            scratch_a[target_offset + x] = if (wall_left..=wall_right).contains(&x)
                            {
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
                h(
                    &scratch_a[..total_pixels],
                    &mut scratch_b[..total_pixels],
                    width,
                    height,
                    radius,
                );
                v(
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

        pub(super) fn h(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize) {
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

        pub(super) fn v(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize) {
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
    }

    #[test]
    fn test_fast_shadow_matches_the_reference_exactly() {
        // The rounded average: every reachable sum, every window width
        for w in (1..=121u32).step_by(2) {
            let div = RoundDiv::new(w);
            for sum in 0..=255 * w {
                assert_eq!(u32::from(div.apply(sum)), (sum + w / 2) / w, "{sum}/{w}");
            }
        }
        // Both blur passes on noisy data, odd sizes and every radius used
        let mut seed = 0x2545_f491_u32;
        let mut noise = |n: usize| -> Vec<u8> {
            (0..n)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed >> 24) as u8
                })
                .collect()
        };
        for (w, h) in [(1, 1), (7, 3), (61, 40), (300, 90)] {
            for radius in [1, 5, 8, 15, 25] {
                let src = noise(w * h);
                let (mut fast, mut slow) = (vec![0; w * h], vec![0; w * h]);
                box_blur_horizontal(&src, &mut fast, w, h, radius);
                reference_shadow::h(&src, &mut slow, w, h, radius);
                assert_eq!(fast, slow, "horizontal {w}x{h} r{radius}");
                box_blur_vertical(&src, &mut fast, w, h, radius);
                reference_shadow::v(&src, &mut slow, w, h, radius);
                assert_eq!(fast, slow, "vertical {w}x{h} r{radius}");
            }
        }
        // Whole masks of real notches, settled and mid-animation, at each DPI
        let renderer = Renderer::new().expect("renderer");
        for dpi in ALL_DPIS {
            let mut frames = vec![
                NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi),
                NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi),
            ];
            let mut anim =
                crate::config::AnimationState::new(NotchState::Collapsed, NotchState::Expanded);
            while anim.step(96.0) {
                frames.push(anim.current_dimensions(dpi));
            }
            for dims in frames {
                // The shadow depends only on the silhouette: the filled notch
                renderer.ensure_buffer(dims.width, dims.height).unwrap();
                {
                    let rt = renderer.cached_rt.borrow();
                    let rt = rt.as_ref().unwrap();
                    let bind = RECT {
                        left: 0,
                        top: 0,
                        right: dims.width,
                        bottom: dims.height,
                    };
                    unsafe {
                        rt.BindDC(renderer.cached_mem_dc.get(), &bind).unwrap();
                        rt.BeginDraw();
                        rt.Clear(Some(&COLOR_TRANSPARENT));
                        let fill = rt.CreateSolidColorBrush(&NOTCH_BG_COLOR, None).unwrap();
                        let path = renderer.create_notch_geometry(&dims).unwrap();
                        rt.FillGeometry(&path, &fill, None);
                        rt.EndDraw(None, None).unwrap();
                    }
                }
                // What reaches the screen: the notch with its shadow composited
                let (px, stride) = renderer.surface(dims.height);
                let (w, h) = (dims.width as usize, dims.height as usize);
                let opacity = dims.shadow_opacity.max(0.28);
                let (mut fast, mut slow) = (px.clone(), px);
                let (mut a, mut b, mut ra, mut rb) = (vec![], vec![], vec![], vec![]);
                let bits = fast.as_mut_ptr() as *mut u8;
                assert!(build_shadow_mask(
                    bits, w, h, stride, dims.scale, opacity, &mut a, &mut b
                ));
                composite_shadow(bits, w, h, stride, &a);
                let bits = slow.as_mut_ptr() as *mut u8;
                assert!(reference_shadow::build_shadow_mask(
                    bits, w, h, stride, dims.scale, opacity, &mut ra, &mut rb,
                ));
                composite_shadow(bits, w, h, stride, &ra);
                assert!(fast == slow, "{w}x{h} at {dpi}");
            }
        }
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
        let with = |f: fn(&mut OecaSettings)| {
            let mut s = OecaSettings::default();
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
                crate::layout::space_dimensions(NotchState::Expanded, dpi, OecaSpace::Music);
            let c = playing_content(P::Playing);
            let m = resolve_media_layout_in(&music, c.shape(), OecaSpace::Music).unwrap();
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
            // A blue cover, like the accent: the bars (from the artwork's
            // palette) and the scrubber (from the accent) both read as blue
            ..content("Song", Some(blue_artwork()))
        }
    }

    fn blue_artwork() -> Artwork {
        let [r, g, b] = TEST_ACCENT;
        Artwork::new(64, 64, (0..64 * 64).flat_map(|_| [b, g, r, 0xFF]).collect()).unwrap()
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
                crate::layout::space_dimensions(NotchState::Expanded, dpi, OecaSpace::Music);
            let c = playing_content(P::Playing);
            let m = resolve_media_layout_in(&expanded, c.shape(), OecaSpace::Music).unwrap();
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
        let dims = crate::layout::space_dimensions(NotchState::Expanded, 96, OecaSpace::Music);
        let c = playing_content(P::Paused);
        let m = resolve_media_layout_in(&dims, c.shape(), OecaSpace::Music).unwrap();
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
            let sel = resolve_space_selector_in(&dims, OecaSpace::Home).unwrap();
            // Capsule fill sampled away from the label text (left end, centered)
            let fill_at = |px: &[u32], stride: usize, s: OecaSpace, active: OecaSpace| {
                let r = resolve_space_selector_in(&dims, active).unwrap().bounds(s);
                let (x, y) = (
                    (r.left + r.height() * 0.35) as usize,
                    ((r.top + r.bottom) / 2.0) as usize,
                );
                px[y * stride + x] & 0xFF
            };
            // Default: Home selected
            renderer.set_space(OecaSpace::default());
            let (px, stride) = render_selector_frame(&renderer, &dims);
            assert!(
                fill_at(&px, stride, OecaSpace::Home, OecaSpace::Home) > 15,
                "Home capsule filled at {dpi}"
            );
            assert_eq!(
                fill_at(&px, stride, OecaSpace::Music, OecaSpace::Home),
                0,
                "Music unfilled at {dpi}"
            );
            assert!(
                lit_in(&px, stride, sel.music, 60),
                "Music label still visible"
            );
            // Music, then back to Home, repeatedly: the drawing follows the state
            for target in [OecaSpace::Music, OecaSpace::Home, OecaSpace::Music] {
                renderer.set_space(target);
                let (px, stride) = render_selector_frame(&renderer, &dims);
                for s in OecaSpace::ALL {
                    assert_eq!(
                        fill_at(&px, stride, s, target) > 15,
                        s == target,
                        "{s:?} at {dpi}"
                    );
                }
            }
            renderer.set_space(OecaSpace::Home);
        }
        assert!(
            !renderer.set_space(OecaSpace::Home),
            "no change, no redraw needed"
        );
        assert!(renderer.set_space(OecaSpace::Music));
    }

    #[test]
    fn test_space_selector_hidden_when_collapsed_and_media_untouched() {
        let renderer = Renderer::new().expect("renderer");
        let c = content("Song", Some(sample_artwork(64, 64)));
        renderer.set_media(Some(&c));
        renderer.set_space(OecaSpace::Music);
        // Collapsed: no selector at all (notch body stays pure black)
        let collapsed = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, 96);
        let (px, stride) = render_selector_frame(&renderer, &collapsed);
        let all = RectF::new(0.0, 0.0, collapsed.width as f32, collapsed.height as f32);
        assert!(!lit_in(&px, stride, all, 5), "nothing drawn when collapsed");
        // Expanding again shows the still-active Music space
        let expanded = NotchDimensions::from_state_and_dpi(NotchState::Expanded, 96);
        let sel = resolve_space_selector_in(&expanded, OecaSpace::Music).unwrap();
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
        renderer.set_space(OecaSpace::Home);
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
            let m = resolve_media_layout_in(dims, media.content.shape(), OecaSpace::Music).unwrap();
            renderer
                .render_media_content(rt, &components, &m, dims, &clock, media)
                .unwrap();
        })
    }

    #[test]
    fn test_music_space_renders_focused_player_without_clock() {
        let renderer = Renderer::new().expect("renderer");
        for dpi in [96u32, 144] {
            let dims = crate::layout::space_dimensions(NotchState::Expanded, dpi, OecaSpace::Music);
            let c = playing_content(crate::media::PlaybackState::Playing);
            renderer.set_media(Some(&c));
            renderer.visualizer().step(1000.0);
            let m = resolve_media_layout_in(&dims, c.shape(), OecaSpace::Music).unwrap();
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
        let dims = crate::layout::space_dimensions(NotchState::Expanded, 96, OecaSpace::Music);
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
                crate::layout::space_dimensions(NotchState::Expanded, dpi, OecaSpace::Clipboard);
            let l = resolve_clipboard_layout(&dims).unwrap();
            let (px, stride) = clipboard_frame(&renderer, &dims);
            assert!(lit_in(&px, stride, l.list, 60), "empty label at {dpi}");
            assert!(
                !lit_in(&px, stride, l.clear, 12),
                "no Clear when empty at {dpi}"
            );
            // No tiles are invented: the first tile's left edge stays black
            let r = l.tiles[0];
            let edge = RectF::new(r.left, r.top, r.left + 3.0, r.bottom);
            assert!(!lit_in(&px, stride, edge, 8), "no tiles at {dpi}");
        }
        assert!(renderer.clipboard.borrow().rows.is_empty());
    }

    #[test]
    fn test_clear_confirmation_replaces_rows_and_x() {
        let renderer = Renderer::new().expect("renderer");
        let mut history = ClipboardHistory::default();
        history.add(ClipboardItem::Image(sample_artwork(300, 100)));
        renderer.set_clipboard(&history);
        let dims = crate::layout::space_dimensions(NotchState::Expanded, 120, OecaSpace::Clipboard);
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
    fn test_clipboard_mixed_history_fills_each_tile_with_its_own_image() {
        // Screenshots of different shapes interleaved with text: every image
        // tile is covered edge to edge by its own image (no empty or stale
        // regions), text tiles stay text, at every DPI
        let solid = |w: u32, h: u32, bgr: [u8; 3]| {
            let px = (0..w * h)
                .flat_map(|_| [bgr[0], bgr[1], bgr[2], 0xFF])
                .collect();
            ClipboardItem::Image(Artwork::new(w, h, px).unwrap())
        };
        let renderer = Renderer::new().expect("renderer");
        let mut history = ClipboardHistory::default();
        history.add(solid(1920, 1080, [0xF0, 0x40, 0x20]));
        history.add(ClipboardItem::Text("copied text".into()));
        history.add(solid(90, 700, [0x20, 0xF0, 0x40]));
        history.add(ClipboardItem::Text("more".into()));
        history.add(solid(3000, 40, [0x40, 0x20, 0xF0]));
        history.add(solid(64, 64, [0xC0, 0xC0, 0xC0]));
        renderer.set_clipboard(&history);
        assert_eq!(
            row_kinds(&renderer),
            ["image", "image", "text", "image", "text", "image"]
        );
        let expect: [Option<[u8; 3]>; 6] = [
            Some([0xC0, 0xC0, 0xC0]),
            Some([0x40, 0x20, 0xF0]),
            None,
            Some([0x20, 0xF0, 0x40]),
            None,
            Some([0xF0, 0x40, 0x20]),
        ];
        for dpi in ALL_DPIS {
            let dims =
                crate::layout::space_dimensions(NotchState::Expanded, dpi, OecaSpace::Clipboard);
            let l = resolve_clipboard_layout(&dims).unwrap();
            renderer.feedback().reset();
            let (px, stride) = clipboard_frame(&renderer, &dims);
            for (i, bgr) in expect.iter().enumerate() {
                let img = l.tile_parts(i).image;
                // Inside the rounded corners: the centre and both edges' midpoints
                let (cx, cy) = (img.left + img.width() / 2.0, img.top + img.height() / 2.0);
                let samples = [
                    (cx, cy),
                    (img.left + 1.5, cy),
                    (img.right - 1.5, cy),
                    (cx, img.top + 1.5),
                    (cx, img.bottom - 1.5),
                ];
                for (x, y) in samples {
                    let v = px[y as usize * stride + x as usize];
                    let got = [v as u8, (v >> 8) as u8, (v >> 16) as u8];
                    match bgr {
                        Some(want) => {
                            let close = got.iter().zip(want).all(|(g, w)| g.abs_diff(*w) <= 2);
                            assert!(close, "tile {i} at ({x},{y}) {dpi}: {got:?} != {want:?}");
                            assert_eq!(v >> 24, 0xFF, "tile {i} opaque at {dpi}");
                        }
                        None => assert!(
                            got.iter().all(|c| *c < 0x30) || got[0] == got[1],
                            "text tile {i} shows no image colour at {dpi}: {got:?}"
                        ),
                    }
                }
            }
        }
        // One thumbnail per shown image, each tied to its own entry
        let thumbs = renderer.thumbs.borrow();
        assert_eq!(thumbs.len(), 4);
        for (a, _, _) in thumbs.iter() {
            assert!(
                history
                    .items()
                    .any(|it| it == &ClipboardItem::Image(a.clone()))
            );
        }
    }

    #[test]
    fn test_clipboard_tiles_newest_first_with_previews_at_all_dpis() {
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
        for dpi in ALL_DPIS {
            let dims =
                crate::layout::space_dimensions(NotchState::Expanded, dpi, OecaSpace::Clipboard);
            let l = resolve_clipboard_layout(&dims).unwrap();
            renderer.feedback().reset();
            let (px, stride) = clipboard_frame(&renderer, &dims);
            assert!(lit_in(&px, stride, l.clear, 60), "Clear shown at {dpi}");
            // Image tile: the orange image fills its preview area
            let img = l.tile_parts(1).image;
            let (x, y) = (
                ((img.left + img.right) / 2.0) as usize,
                ((img.top + img.bottom) / 2.0) as usize,
            );
            let p = px[y * stride + x];
            assert!(
                (p >> 16) & 0xFF > 200 && p & 0xFF < 100,
                "thumbnail drawn at {dpi}: {p:08x}"
            );
            assert!(
                lit_in(&px, stride, l.tile_parts(1).caption, 40),
                "caption at {dpi}"
            );
            for i in [0, 2] {
                assert!(
                    lit_in(&px, stride, l.tile_parts(i).text, 60),
                    "text {i} at {dpi}"
                );
            }
            // Outlined squircles: lit edges, a dark middle, square-ish
            // corners rounded off (the very corner pixel stays dark)
            for t in &l.tiles[..3] {
                let mid_y = (t.top + t.bottom) / 2.0;
                let edge = RectF::new(t.left, mid_y - 2.0, t.left + 1.5, mid_y + 2.0);
                assert!(lit_in(&px, stride, edge, 25), "outline at {dpi}");
                let corner = RectF::new(t.left, t.top, t.left + 1.0, t.top + 1.0);
                assert!(!lit_in(&px, stride, corner, 25), "rounded corner at {dpi}");
            }
            // Fewer than six: the other tiles are not drawn at all
            for t in &l.tiles[3..] {
                assert!(!lit_in(&px, stride, *t, 12), "empty slot at {dpi}");
            }
            // Buttons are hidden until a tile is hovered...
            for i in 0..3 {
                let p = l.tile_parts(i);
                for r in [p.copy, p.trash] {
                    assert!(!lit_in(&px, stride, r, 60), "no buttons without hover");
                }
            }
            // ...and hovering tile 0 fades in only tile 0's (no layout shift:
            // its text is unchanged)
            renderer.feedback().set_clip_row(Some(0));
            assert!(renderer.feedback().is_animating(), "fades in");
            renderer.feedback().step(40.0);
            let mid_level = renderer.feedback().clip_levels(PanelHit::Row(0)).0;
            assert!(mid_level > 0.0 && mid_level < 1.0, "fading: {mid_level}");
            while renderer.feedback().step(16.0) {}
            assert_eq!(renderer.feedback().clip_levels(PanelHit::Row(0)).0, 1.0);
            let (hovered, stride) = clipboard_frame(&renderer, &dims);
            for i in 0..3 {
                let p = l.tile_parts(i);
                let lit =
                    lit_in(&hovered, stride, p.copy, 120) && lit_in(&hovered, stride, p.trash, 120);
                assert_eq!(lit, i == 0, "tile {i} buttons at {dpi}");
            }
            let text = l.tile_parts(0).text;
            let crop = |v: &[u32]| {
                (text.top as usize..text.bottom as usize)
                    .flat_map(|y| {
                        (text.left as usize + 2..text.right as usize - 2).map(move |x| (x, y))
                    })
                    .map(|(x, y)| v[y * stride + x] & 0xFF)
                    .max()
            };
            assert_eq!(crop(&px).is_some(), crop(&hovered).is_some());
            // Leaving fades them out to nothing
            renderer.feedback().set_clip_row(None);
            while renderer.feedback().step(16.0) {}
            assert_eq!(renderer.feedback().clip_levels(PanelHit::Row(0)).0, 0.0);
            assert!(!renderer.feedback().is_animating(), "settled: no frames");
        }
        // The thumbnail bitmap is cached and reused, not rebuilt per frame
        let dims = crate::layout::space_dimensions(NotchState::Expanded, 144, OecaSpace::Clipboard);
        clipboard_frame(&renderer, &dims);
        let first = renderer.thumbs.borrow()[0].2.clone();
        clipboard_frame(&renderer, &dims);
        assert_eq!(renderer.thumbs.borrow().len(), 1);
        assert_eq!(renderer.thumbs.borrow()[0].2, first);

        // Clear: tiles and thumbnail memory all go; the empty state is valid
        history.clear();
        renderer.set_clipboard(&history);
        assert!(renderer.clipboard.borrow().rows.is_empty());
        assert!(renderer.thumbs.borrow().is_empty(), "thumbnails released");
        let l = resolve_clipboard_layout(&dims).unwrap();
        let (px, stride) = clipboard_frame(&renderer, &dims);
        assert!(lit_in(&px, stride, l.list, 60), "empty state after clear");
        assert!(!lit_in(&px, stride, l.clear, 12));
    }

    #[test]
    fn test_clipboard_shows_six_tiles_when_full() {
        let renderer = Renderer::new().expect("renderer");
        let mut history = ClipboardHistory::default();
        for i in 0..9 {
            history.add(ClipboardItem::Text(format!("entry {i}")));
        }
        renderer.set_clipboard(&history);
        assert_eq!(row_kinds(&renderer).len(), 6, "the six newest");
        for dpi in ALL_DPIS {
            let dims =
                crate::layout::space_dimensions(NotchState::Expanded, dpi, OecaSpace::Clipboard);
            let l = resolve_clipboard_layout(&dims).unwrap();
            let (px, stride) = clipboard_frame(&renderer, &dims);
            for i in 0..6 {
                assert!(
                    lit_in(&px, stride, l.tile_parts(i).text, 60),
                    "tile {i} at {dpi}"
                );
            }
        }
    }

    // ---- Media motion ---------------------------------------------------------

    fn bbox(shapes: &[Quad], r: f32) -> RectF {
        let pts = shapes.iter().flatten();
        let (mut l, mut t, mut rr, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in pts {
            assert!(x.is_finite() && y.is_finite());
            (l, t, rr, b) = (l.min(x), t.min(y), rr.max(x), b.max(y));
        }
        RectF::new(l - r, t - r, rr + r, b + r)
    }

    #[test]
    fn test_play_pause_morph_ends_on_the_rest_glyphs_and_stays_in_bounds() {
        for (u, snap) in [(13.0f32, true), (19.0, true), (28.5, false)] {
            let (cx, cy) = (100.0, 50.0);
            // Ends: exactly the rest glyphs' extents
            let shape_box = |icon| {
                let shapes = icon_shapes(MediaControl::PlayPause, icon, cx, cy, u, snap);
                shapes_bbox(&shapes)
            };
            let (play, r0) = play_pause_outline(cx, cy, u, 0.0, snap);
            let (pause, r1) = play_pause_outline(cx, cy, u, 1.0, snap);
            let close = |a: RectF, b: RectF| {
                (a.left - b.left).abs() < 0.05
                    && (a.right - b.right).abs() < 0.05
                    && (a.top - b.top).abs() < 0.05
                    && (a.bottom - b.bottom).abs() < 0.05
            };
            let pb = shape_box(PlayPauseIcon::Play);
            let qb = shape_box(PlayPauseIcon::Pause);
            // The triangle's rounded tip pulls in a little from the sharp tip
            assert!((bbox(&play, r0).left - pb.left).abs() < 0.05, "{u}");
            // ...by less than the rest glyph's own corner cut
            let cut = u * TRIANGLE_ROUNDING;
            let right = bbox(&play, r0).right;
            assert!(
                right <= pb.right + 0.05 && right > pb.right - cut,
                "{u}: {right}"
            );
            assert!(
                close(bbox(&pause, r1), qb),
                "pause {u}: {:?} vs {qb:?}",
                bbox(&pause, r1)
            );
            // Bars: fully round ends (inset to centre lines)
            assert!(
                (pause[0][1].0 - pause[0][0].0).abs() < 1e-3,
                "bar inset to its centre line"
            );
            assert!((2.0 * r1 - (pause[0][0].0 - qb.left) * 2.0).abs() < 1e-3);
            // In between: finite, inside the union of the two, continuous
            let union = RectF::new(
                pb.left.min(qb.left) - 0.5,
                pb.top.min(qb.top) - 0.5,
                pb.right.max(qb.right) + 0.5,
                pb.bottom.max(qb.bottom) + 0.5,
            );
            let mut last = bbox(&play, r0);
            for k in 1..=40 {
                let (shapes, r) = play_pause_outline(cx, cy, u, k as f32 / 40.0, snap);
                let b = bbox(&shapes, r);
                assert!(inside_rect(&b, &union), "m {k}/40 at {u}: {b:?}");
                assert!((b.left - last.left).abs() < 1.0 && (b.right - last.right).abs() < 1.0);
                last = b;
            }
        }
    }

    fn inside_rect(a: &RectF, b: &RectF) -> bool {
        a.left >= b.left && a.right <= b.right && a.top >= b.top && a.bottom <= b.bottom
    }

    /// The Music player frame with its controls' current motion.
    fn player_frame(
        renderer: &Renderer,
        dims: &NotchDimensions,
        c: &MediaContent,
    ) -> (Vec<u32>, usize) {
        renderer.set_space(OecaSpace::Music);
        let frame = music_frame(renderer, dims, c);
        renderer.set_space(OecaSpace::Home);
        frame
    }

    #[test]
    fn test_play_pause_glyph_renders_crisply_at_both_ends_and_mid_morph() {
        let renderer = Renderer::new().expect("renderer");
        for dpi in [96u32, 144] {
            let dims = space_dimensions(NotchState::Expanded, dpi, OecaSpace::Music);
            let mut c = playing_content(crate::media::PlaybackState::Paused);
            c.icon = PlayPauseIcon::Play;
            renderer.set_media(None);
            renderer.set_media(Some(&c));
            let m = resolve_media_layout_in(&dims, c.shape(), OecaSpace::Music).unwrap();
            let v = m.control_visual_bounds(MediaControl::PlayPause);
            let (rest, stride) = player_frame(&renderer, &dims, &c);
            assert!(lit_in(&rest, stride, v, 100), "play at {dpi}");
            let level = |px: &[u32]| {
                (v.top as usize..v.bottom as usize)
                    .flat_map(|y| (v.left as usize..v.right as usize).map(move |x| (x, y)))
                    .map(|(x, y)| px[y * stride + x] & 0xFF)
                    .max()
                    .unwrap()
            };
            let rest_level = level(&rest);
            // Mid-morph: drawn, inside the control, one even opacity (the
            // overlapping parts never stack up into brighter patches)
            renderer
                .icons()
                .clicked_play_pause(std::time::Instant::now(), false);
            for _ in 0..6 {
                renderer.icons().step(16.0, std::time::Instant::now());
            }
            let (mid, _) = player_frame(&renderer, &dims, &c);
            assert!(lit_in(&mid, stride, v, 100), "mid-morph at {dpi}");
            let peak = level(&mid);
            assert!(
                peak <= rest_level + 3,
                "no brighter overlaps at {dpi}: {peak} vs {rest_level}"
            );
            renderer.settle_media_motion();
        }
        renderer.set_media(None);
    }

    #[test]
    fn test_skip_slide_stays_inside_its_button() {
        let renderer = Renderer::new().expect("renderer");
        for dpi in [96u32, 144] {
            let dims = space_dimensions(NotchState::Expanded, dpi, OecaSpace::Music);
            let c = playing_content(crate::media::PlaybackState::Playing);
            renderer.set_media(Some(&c));
            let m = resolve_media_layout_in(&dims, c.shape(), OecaSpace::Music).unwrap();
            for control in [MediaControl::Previous, MediaControl::Next] {
                renderer.settle_media_motion();
                let hit = m.control_bounds(control);
                let v = m.control_visual_bounds(control);
                renderer.icons().clicked_skip(control, false);
                for k in 0..20 {
                    renderer.icons().step(16.0, std::time::Instant::now());
                    let (px, stride) = player_frame(&renderer, &dims, &c);
                    // Only inside the button's own bounds (clipped); its
                    // neighbours' gaps stay dark
                    for gap in [
                        RectF::new(hit.left, v.top, v.left - 1.0, v.bottom),
                        RectF::new(v.right + 1.0, v.top, hit.right, v.bottom),
                    ] {
                        assert!(
                            !lit_in(&px, stride, gap, 30),
                            "{control:?} frame {k} at {dpi}"
                        );
                    }
                    // The control itself never moves
                    assert_eq!(m.control_bounds(control), hit);
                }
            }
            renderer.settle_media_motion();
        }
        renderer.set_media(None);
    }

    fn cover(rgb: [u8; 3]) -> Artwork {
        Artwork::new(
            32,
            32,
            (0..32 * 32)
                .flat_map(|_| [rgb[2], rgb[1], rgb[0], 0xFF])
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn test_artwork_flips_on_a_real_artwork_change_only() {
        // The flip needs system animations, which CI runners turn off
        if !crate::window::is_client_animation_enabled() {
            return;
        }
        let renderer = Renderer::new().expect("renderer");
        let a = content("A", Some(cover([200, 40, 40])));
        let b = content("B", Some(cover([40, 40, 200])));
        renderer.set_media(Some(&a));
        assert!(renderer.flip.borrow().is_none(), "appearing never flips");
        // Same track refreshed (duplicate metadata): nothing
        renderer.set_media(Some(&a.clone()));
        assert!(renderer.flip.borrow().is_none());
        let mut a2 = a.clone();
        a2.icon = PlayPauseIcon::Pause;
        renderer.set_media(Some(&a2));
        assert!(renderer.flip.borrow().is_none(), "playback change: no flip");
        // New track, same artwork (e.g. the next song of an album): no flip
        renderer.set_media(Some(&content("A2", a.artwork.clone())));
        assert!(renderer.flip.borrow().is_none());
        // A new cover: half a turn, nothing more
        renderer.set_media(Some(&b));
        {
            let flip = renderer.flip.borrow();
            let f = flip.as_ref().unwrap();
            assert_eq!(f.angle.value, 0.0, "starts facing the viewer");
            assert!(
                (f.angle.target - std::f32::consts::PI).abs() < 1e-6,
                "180 degrees"
            );
            assert_eq!(f.shown(), a.artwork.as_ref().unwrap(), "old cover in front");
        }
        // A duplicate update mid-flip never restarts it
        renderer.step_media_motion(48.0);
        let before = renderer.flip.borrow().as_ref().unwrap().angle;
        renderer.set_media(Some(&b.clone()));
        assert_eq!(
            renderer.flip.borrow().as_ref().unwrap().angle,
            before,
            "no restart"
        );
        // About 300 ms to land (within 1 degree), then settled: no frames
        let mut ms = 48.0;
        while (renderer.flip.borrow().as_ref().unwrap().angle.value - std::f32::consts::PI).abs()
            > 0.0175
        {
            renderer.step_media_motion(16.0);
            ms += 16.0;
            assert!(ms < 1000.0);
        }
        assert!((250.0..=350.0).contains(&ms), "lands in about 300 ms: {ms}");
        let mut extra = 0.0;
        while renderer.step_media_motion(16.0) {
            extra += 16.0;
            assert!(extra < 1000.0, "settles");
        }
        assert!(renderer.flip.borrow().is_none());
        // Missing artwork: no flip (and a running one stops at once)
        renderer.set_media(Some(&a));
        assert!(renderer.flip.borrow().is_some());
        renderer.set_media(Some(&content("D", None)));
        assert!(renderer.flip.borrow().is_none(), "no artwork: no flip");
        // Reduced motion: never flips
        let reduced = OecaSettings {
            reduced_motion: true,
            ..OecaSettings::default()
        };
        renderer.set_settings(false, reduced);
        renderer.set_media(Some(&a));
        renderer.set_media(Some(&b));
        assert!(renderer.flip.borrow().is_none(), "reduced motion");
        renderer.set_settings(false, OecaSettings::default());
        // Off screen: settled at once
        renderer.set_media(Some(&a));
        assert!(renderer.flip.borrow().is_some());
        renderer.settle_media_motion();
        // (the collapsed cover settles separately: settled-expanded notch)
        renderer.settle_collapsed_art();
        assert!(!renderer.media_motion_animating());
        renderer.set_media(None);
    }

    #[test]
    fn test_rapid_track_changes_retarget_without_restart_or_snap() {
        use std::f32::consts::{FRAC_PI_2, PI};
        // The flip needs system animations, which CI runners turn off
        if !crate::window::is_client_animation_enabled() {
            return;
        }
        let renderer = Renderer::new().expect("renderer");
        let (a, b, c, d) = (
            content("A", Some(cover([200, 40, 40]))),
            content("B", Some(cover([40, 40, 200]))),
            content("C", Some(cover([40, 200, 40]))),
            content("D", Some(cover([200, 200, 40]))),
        );
        renderer.set_media(Some(&a));
        renderer.set_media(Some(&b));
        // Before edge-on: the newest simply goes on the back (still to come)
        renderer.step_media_motion(32.0);
        let before = renderer.flip.borrow().as_ref().unwrap().angle;
        assert!(before.value > 0.0 && before.value < FRAC_PI_2);
        renderer.set_media(Some(&c));
        {
            let flip = renderer.flip.borrow();
            let f = flip.as_ref().unwrap();
            assert_eq!(f.angle, before, "continues, velocity and all");
            assert_eq!(&f.faces[1], c.artwork.as_ref().unwrap());
            assert_eq!(
                f.shown(),
                a.artwork.as_ref().unwrap(),
                "no snap: A still facing"
            );
            assert!((f.angle.target - PI).abs() < 1e-6, "still half a turn");
        }
        // After edge-on (C showing): the photo carries on another half-turn
        // and D comes round on the front; C is never swapped out in view
        while renderer.flip.borrow().as_ref().unwrap().angle.value < 2.0 {
            renderer.step_media_motion(16.0);
        }
        let mid = renderer.flip.borrow().as_ref().unwrap().angle;
        renderer.set_media(Some(&d));
        {
            let flip = renderer.flip.borrow();
            let f = flip.as_ref().unwrap();
            assert_eq!(
                (f.angle.value, f.angle.target > mid.target),
                (mid.value, true),
                "no jump (same place and speed), on another half-turn"
            );
            assert_eq!(f.shown(), c.artwork.as_ref().unwrap(), "C stays in view");
            assert_eq!(&f.faces[0], d.artwork.as_ref().unwrap(), "D next");
            assert!((f.angle.target - 2.0 * PI).abs() < 1e-4);
        }
        while renderer.step_media_motion(16.0) {}
        assert!(renderer.flip.borrow().is_none(), "ends on D, settled");
        renderer.set_media(None);
    }

    /// A cover in two halves (left / right colours: shows its orientation).
    fn halves(left: [u8; 3], right: [u8; 3]) -> Artwork {
        let px = (0..32 * 32)
            .flat_map(|i| {
                let c = if i % 32 < 16 { left } else { right };
                [c[2], c[1], c[0], 0xFF]
            })
            .collect();
        Artwork::new(32, 32, px).unwrap()
    }

    #[test]
    fn test_artwork_flip_frames_one_flat_photo_about_its_centre() {
        use std::f32::consts::PI;
        // The flip needs system animations, which CI runners turn off
        if !crate::window::is_client_animation_enabled() {
            return;
        }
        let renderer = Renderer::new().expect("renderer");
        let (red, green, blue, yellow) =
            ([230, 30, 30], [30, 220, 30], [30, 30, 230], [230, 230, 30]);
        let kind = |p: u32| {
            let (b, g, r) = (p & 0xFF, (p >> 8) & 0xFF, (p >> 16) & 0xFF);
            match (r > 150, g > 150, b > 150) {
                (true, false, false) => Some("red"),
                (false, true, false) => Some("green"),
                (false, false, true) => Some("blue"),
                (true, true, false) => Some("yellow"),
                _ => None,
            }
        };
        for dpi in [96u32, 144] {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            let a = content("A", Some(halves(red, green)));
            let b = content("B", Some(halves(blue, yellow)));
            let art = resolve_media_layout(&dims, a.shape())
                .unwrap()
                .artwork_bounds
                .unwrap();
            let still = |c: &MediaContent| {
                renderer.set_media(None);
                renderer.set_media(Some(c));
                render_media_frame(&renderer, &dims, c).0
            };
            let (rest_a, rest_b) = (still(&a), still(&b));
            renderer.set_media(None);
            renderer.set_media(Some(&a));
            renderer.set_media(Some(&b));
            renderer.feedback().finish_track_fade();
            let stride = render_frame_as_is(&renderer, &dims).1;
            let frame = |deg: f32| {
                renderer.flip.borrow_mut().as_mut().unwrap().angle.value = deg * PI / 180.0;
                render_frame_as_is(&renderer, &dims).0
            };
            let region = |px: &[u32]| -> Vec<u32> {
                (art.top as usize..art.bottom as usize)
                    .flat_map(|y| {
                        (art.left as usize - 3..art.right as usize + 3).map(move |x| (x, y))
                    })
                    .map(|(x, y)| px[y * stride + x])
                    .collect()
            };
            // 0 and 180 degrees: exactly the resting old / new artwork
            assert_eq!(region(&frame(0.0)), region(&rest_a), "0 degrees at {dpi}");
            assert_eq!(
                region(&frame(180.0)),
                region(&rest_b),
                "180 degrees at {dpi}"
            );
            let (cx, cy) = (
                (art.left + art.right) / 2.0,
                ((art.top + art.bottom) / 2.0) as usize,
            );
            for deg in [
                10.0f32, 30.0, 60.0, 80.0, 89.0, 90.0, 91.0, 100.0, 120.0, 150.0, 170.0,
            ] {
                let px = frame(deg);
                let row: Vec<(usize, Option<&str>)> = (art.left as usize - 3
                    ..art.right as usize + 3)
                    .map(|x| (x, kind(px[cy * stride + x])))
                    .filter(|(_, k)| k.is_some())
                    .collect();
                let width = art.width() * (deg * PI / 180.0).cos().abs();
                let at = format!("{deg} degrees at {dpi}");
                if width < 2.0 {
                    // Edge-on: a hairline at the centre, never gone
                    let lit = (art.left as usize..art.right as usize)
                        .filter(|&x| (px[cy * stride + x] & 0xFF_FFFF) > 0x10_1010)
                        .count();
                    assert!((1..=3).contains(&lit), "hairline, {at}: {lit}");
                    continue;
                }
                let (l, r) = (row[0].0 as f32, row[row.len() - 1].0 as f32 + 1.0);
                // Narrowed by |cos|, about the centre: no sideways drift
                assert!(
                    ((r - l) - width).abs() <= 2.5,
                    "width {} vs {width}, {at}",
                    r - l
                );
                assert!((((l + r) / 2.0) - cx).abs() <= 1.0, "centred, {at}");
                // One face at a time, upright (left half on the left), no mix
                let (left, right) = if deg < 90.0 {
                    ("red", "green")
                } else {
                    ("blue", "yellow")
                };
                let mid = (l + r) / 2.0;
                for (x, k) in &row {
                    let k = k.unwrap();
                    assert!(k == left || k == right, "only its own cover, {at}: {k}");
                    let side = if (*x as f32) < mid - 1.0 {
                        Some(left)
                    } else if (*x as f32) > mid + 1.0 {
                        Some(right)
                    } else {
                        None
                    };
                    if let Some(side) = side {
                        assert_eq!(k, side, "upright, not mirrored, {at}");
                    }
                }
                // Full height (no vertical distortion), never outside its slot
                let col = cx as usize;
                assert!(
                    kind(px[(art.top as usize + 1) * stride + col]).is_some(),
                    "top edge, {at}"
                );
                assert!(
                    kind(px[(art.bottom as usize - 2) * stride + col]).is_some(),
                    "bottom edge, {at}"
                );
                for y in art.top as usize..art.bottom as usize {
                    for x in [art.left as usize - 2, art.right as usize + 1] {
                        assert!(kind(px[y * stride + x]).is_none(), "outside its slot, {at}");
                    }
                }
            }
            renderer.settle_media_motion();
        }
        renderer.set_media(None);
    }

    /// The glyph lands on exactly its rest picture: the last moving frame of a
    /// morph (or of a press easing back) differs from the settled frame by
    /// no more than a sliver of anti-aliasing (no snapping pop at the end).
    #[test]
    fn test_play_pause_morph_and_press_end_without_a_pop() {
        let renderer = Renderer::new().expect("renderer");
        for dpi in [96u32, 120, 144] {
            let dims = space_dimensions(NotchState::Expanded, dpi, OecaSpace::Music);
            let mut c = playing_content(crate::media::PlaybackState::Paused);
            c.icon = PlayPauseIcon::Play;
            renderer.set_media(None);
            renderer.set_media(Some(&c));
            let m = resolve_media_layout_in(&dims, c.shape(), OecaSpace::Music).unwrap();
            let v = m.control_visual_bounds(MediaControl::PlayPause);
            let diff = |a: &[u32], b: &[u32], stride: usize| {
                (v.top as usize..v.bottom as usize)
                    .flat_map(|y| (v.left as usize..v.right as usize).map(move |x| y * stride + x))
                    .map(|i| ((a[i] & 0xFF) as i32 - (b[i] & 0xFF) as i32).unsigned_abs())
                    .max()
                    .unwrap()
            };
            let now = std::time::Instant::now();
            renderer.icons().clicked_play_pause(now, false);
            // The session confirms (otherwise it would wait for it)
            c.icon = PlayPauseIcon::Pause;
            renderer.set_media(Some(&c));
            // Run to the last frame before it settles
            loop {
                let mut probe = *renderer.icons();
                if !probe.step(16.0, now) {
                    break;
                }
                renderer.icons().step(16.0, now);
            }
            let (moving, stride) = player_frame(&renderer, &dims, &c);
            renderer.icons().step(16.0, now);
            let (settled, _) = player_frame(&renderer, &dims, &c);
            assert!(
                diff(&moving, &settled, stride) <= 24,
                "pop at the end at {dpi}"
            );
            // ...and back: pause -> play lands just as cleanly
            renderer.icons().clicked_play_pause(now, false);
            c.icon = PlayPauseIcon::Play;
            renderer.set_media(Some(&c));
            loop {
                let mut probe = *renderer.icons();
                if !probe.step(16.0, now) {
                    break;
                }
                renderer.icons().step(16.0, now);
            }
            let (moving, _) = player_frame(&renderer, &dims, &c);
            renderer.icons().step(16.0, now);
            let (settled, _) = player_frame(&renderer, &dims, &c);
            assert!(
                diff(&moving, &settled, stride) <= 24,
                "pop at the play end at {dpi}"
            );
            // A press easing back ends the same way
            renderer.feedback().press(MediaControl::PlayPause);
            while renderer.feedback().step(16.0) {}
            renderer.feedback().release();
            let mut last = None;
            while renderer.feedback().step(16.0) {
                last = Some(player_frame(&renderer, &dims, &c).0);
            }
            let (rest, _) = player_frame(&renderer, &dims, &c);
            if let Some(last) = last {
                assert!(
                    diff(&last, &rest, stride) <= 40,
                    "press release pop at {dpi}"
                );
            }
            renderer.settle_media_motion();
        }
        renderer.set_media(None);
    }

    /// Skip morph geometry: rest at both ends, one way, never fading.
    #[test]
    fn test_skip_morph_rolls_one_step_and_rests_on_the_glyph() {
        let (cx, cy, u) = (50.0, 20.0, 16.0);
        for control in [MediaControl::Next, MediaControl::Previous] {
            let rest: Vec<[(f32, f32); 3]> =
                icon_shapes(control, PlayPauseIcon::Play, cx, cy, u, true)
                    .into_iter()
                    .map(|s| match s {
                        IconShape::Triangle(t) => t,
                        IconShape::Bar(_) => unreachable!(),
                    })
                    .collect();
            let close = |a: &[[(f32, f32); 3]], b: &[[(f32, f32); 3]]| {
                a.len() == b.len()
                    && a.iter().all(|t| {
                        b.iter().any(|o| {
                            t.iter()
                                .zip(o)
                                .all(|(p, q)| (p.0 - q.0).abs() < 1e-3 && (p.1 - q.1).abs() < 1e-3)
                        })
                    })
            };
            assert!(
                close(&skip_morph(control, cx, cy, u, 0.0), &rest),
                "{control:?} at 0"
            );
            assert!(
                close(&skip_morph(control, cx, cy, u, 1.0), &rest),
                "{control:?} at 1"
            );
            // In between: three triangles at most, all moving its way
            let dir = if control == MediaControl::Next {
                1.0
            } else {
                -1.0
            };
            let mut last = skip_morph(control, cx, cy, u, 0.0);
            for k in 1..20 {
                let now = skip_morph(control, cx, cy, u, k as f32 / 20.0);
                assert!(now.len() <= 3);
                let tip = |v: &Vec<[(f32, f32); 3]>| {
                    v.iter().map(|t| t[1].0 * dir).fold(f32::MIN, f32::max)
                };
                assert!(tip(&now) >= tip(&last) - 1e-3, "{control:?} moves its way");
                assert!(
                    now.iter().flatten().all(|p| (p.0 - cx).abs() <= u * 1.2),
                    "stays near"
                );
                last = now;
            }
        }
    }

    #[test]
    fn test_scrub_and_committed_seek_drive_the_scrubber() {
        use crate::media::PlaybackState as P;
        let renderer = Renderer::new().expect("renderer");
        let dims = space_dimensions(NotchState::Expanded, 144, OecaSpace::Music);
        let c = playing_content(P::Paused); // 100 s of 200 s
        renderer.set_media(Some(&c));
        let m = resolve_media_layout_in(&dims, c.shape(), OecaSpace::Music).unwrap();
        let track = m.scrub_track(dims.scale).unwrap();
        let played_to = |renderer: &Renderer| {
            let (px, stride) = player_frame(renderer, &dims, &c);
            let y = ((m.timeline_bounds.top + m.timeline_bounds.bottom) / 2.0) as usize;
            (track.left as usize..track.right as usize)
                .rev()
                .find(|&x| {
                    accent_in(
                        &px,
                        stride,
                        RectF::new(x as f32, y as f32, x as f32 + 1.0, y as f32 + 1.0),
                    )
                })
                .map_or(track.left, |x| x as f32)
        };
        let frac = |x: f32| (x - track.left) / track.width();
        assert!(
            (frac(played_to(&renderer)) - 0.5).abs() < 0.04,
            "session position"
        );
        renderer.set_scrub(Some(0.8));
        assert!(
            (frac(played_to(&renderer)) - 0.8).abs() < 0.04,
            "follows the drag"
        );
        renderer.set_scrub(Some(0.2));
        assert!(
            (frac(played_to(&renderer)) - 0.2).abs() < 0.04,
            "continuously"
        );
        // Released: held at the seek until the session reports back
        renderer.set_scrub(None);
        renderer.hold_seek(150_000);
        assert!(
            (frac(played_to(&renderer)) - 0.75).abs() < 0.04,
            "held at the seek"
        );
        let mut moved = c.clone();
        moved.timeline =
            crate::media::Timeline::from_raw(0, 200 * 10_000_000, 30 * 10_000_000, 0, 0);
        renderer.set_media(Some(&moved));
        let (px, stride) = player_frame(&renderer, &dims, &moved);
        let _ = (px, stride);
        assert!(
            renderer.seek_hold.get().is_none(),
            "the session's own position wins"
        );
        renderer.set_media(None);
    }

    // ---- Visualizer look ------------------------------------------------------

    fn solid_cover(rgb: [u8; 3]) -> Artwork {
        Artwork::new(
            48,
            48,
            (0..48 * 48)
                .flat_map(|_| [rgb[2], rgb[1], rgb[0], 0xFF])
                .collect(),
        )
        .unwrap()
    }

    /// The collapsed notch with steady, fully shown bars for `art`.
    fn bars_frame(
        renderer: &Renderer,
        dims: &NotchDimensions,
        art: Artwork,
    ) -> (Vec<u32>, usize, RectF) {
        let c = MediaContent {
            playback: crate::media::PlaybackState::Playing,
            accent: None,
            ..content("Song", Some(art))
        };
        renderer.set_media(None);
        renderer.set_media(Some(&c));
        renderer.visualizer().step(1000.0);
        let slot = resolve_collapsed_layout(dims).visualizer_bounds;
        let (px, stride) = render_collapsed_frame(renderer, dims);
        (px, stride, slot)
    }

    /// Each bar's centre column, top end to bottom end: (y, pixel) of its
    /// lit pixels.
    fn bar_columns(
        px: &[u32],
        stride: usize,
        dims: &NotchDimensions,
        slot: RectF,
    ) -> Vec<Vec<(usize, u32)>> {
        let (bar, gap, _) = visualizer_metrics(dims.scale);
        let x0 = slot.left.round() as usize;
        let level = |p: u32| ((p >> 16) & 0xFF).max((p >> 8) & 0xFF).max(p & 0xFF);
        (0..4)
            .map(|i| {
                let x = x0 + i * (bar + gap) as usize + (bar / 2.0) as usize;
                let column: Vec<_> = (0..dims.height as usize)
                    .map(|y| (y, px[y * stride + x]))
                    .collect();
                // The bar's own pixels, not its anti-aliased end rows
                let peak = column.iter().map(|&(_, p)| level(p)).max().unwrap_or(0);
                column
                    .into_iter()
                    .filter(|&(_, p)| peak > 40 && level(p) * 5 > peak * 3)
                    .collect()
            })
            .collect()
    }

    /// The colour at `f` (0 top .. 1 bottom) along a bar's column.
    fn at_share(column: &[(usize, u32)], f: f32) -> [i32; 3] {
        let p = column[((column.len() - 1) as f32 * f).round() as usize].1;
        [(p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF].map(|v| v as i32)
    }

    fn close(a: [i32; 3], b: [i32; 3], tol: i32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
    }

    /// Every bar shows the same gradient: each of their pixels is a colour
    /// of the one ramp now shown, and each bar's middle is the ramp's middle.
    fn assert_one_gradient(renderer: &Renderer, columns: &[Vec<(usize, u32)>], what: &str) {
        let ends = renderer.viz_blend.get().current(std::time::Instant::now());
        let bytes = |c: D2D1_COLOR_F| [c.r, c.g, c.b].map(|v| (v * 255.0).round() as i32);
        let curve: Vec<_> = (0..=400)
            .map(|k| bytes(viz_ramp_colour(ends, k as f32 / 400.0, 1.0)))
            .collect();
        for (i, c) in columns.iter().enumerate() {
            assert!(c.len() > 3, "{what}: bar {i} drawn");
            // Inner pixels (the end rows are anti-aliased against black)
            for &(y, _) in &c[1..c.len() - 1] {
                let p = at_share(c, (y - c[0].0) as f32 / (c.len() - 1) as f32);
                assert!(
                    curve.iter().any(|&q| close(p, q, 6)),
                    "{what}: bar {i} pixel {p:?} off the gradient"
                );
            }
            let mid = at_share(c, 0.5);
            assert!(
                close(mid, bytes(viz_ramp_colour(ends, 0.5, 1.0)), 12),
                "{what}: bar {i} middle {mid:?}"
            );
        }
    }

    #[test]
    fn test_all_bars_share_one_gradient_brightest_in_the_middle() {
        let renderer = Renderer::new().expect("renderer");
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, 288);
        // One colour: red-orange throughout every bar, brightest in the middle
        let (px, stride, slot) = bars_frame(&renderer, &dims, solid_cover([235, 60, 30]));
        let columns = bar_columns(&px, stride, &dims, slot);
        assert_one_gradient(&renderer, &columns, "single colour");
        for (i, column) in columns.iter().enumerate() {
            assert!(column.len() > 4, "bar {i} drawn");
            for (_, p) in column {
                let (a, r, g, b) = (p >> 24, (p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
                assert!(r <= a && g <= a && b <= a, "premultiplied");
                assert_eq!(a, 255, "on the notch's black");
                assert!(r > g * 2 && r > b * 3, "one hue, never greyer: {p:08x}");
            }
            let (mid, end) = (at_share(column, 0.5), at_share(column, 0.97));
            assert!(mid[0] > 200, "rich at its heart (bar {i}): {mid:?}");
            assert!(
                mid[0] > end[0] + 10,
                "a touch deeper toward the end (bar {i})"
            );
            assert!(end[0] > mid[0] * 3 / 5, "restrained, not black (bar {i})");
        }
        // Two colours (blue and purple): every bar blue at the top to purple
        // at the bottom, the same in all four
        let two = Artwork::new(
            48,
            48,
            (0..48 * 48)
                .flat_map(|i| {
                    if i % 48 < 28 {
                        [230, 80, 30, 0xFF]
                    } else {
                        [220, 50, 140, 0xFF]
                    }
                })
                .collect(),
        )
        .unwrap();
        let (px, stride, slot) = bars_frame(&renderer, &dims, two);
        let columns = bar_columns(&px, stride, &dims, slot);
        assert_one_gradient(&renderer, &columns, "two colours");
        for column in &columns {
            let (top, bottom) = (at_share(column, 0.15), at_share(column, 0.85));
            assert!(top[2] > top[0] * 2, "blue at the top: {top:?}");
            assert!(
                bottom[0] > top[0] + 30,
                "purple at the bottom: {bottom:?} vs {top:?}"
            );
        }
        renderer.set_media(None);
    }

    #[test]
    fn test_neutral_or_missing_artwork_bars_use_the_accent_gradient() {
        let renderer = Renderer::new().expect("renderer");
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, 144);
        let grey = solid_cover([128, 128, 128]);
        let mono = Artwork::new(
            48,
            48,
            (0..48 * 48)
                .flat_map(|i| if i % 2 == 0 { [0, 0, 0, 255] } else { [255; 4] })
                .collect(),
        )
        .unwrap();
        let clear = Artwork::new(48, 48, vec![0; 48 * 48 * 4]).unwrap();
        let shown = |art: Option<Artwork>| {
            let c = MediaContent {
                playback: crate::media::PlaybackState::Playing,
                accent: None,
                ..content("Song", art)
            };
            renderer.set_media(None);
            renderer.set_media(Some(&c));
            renderer.visualizer().step(1000.0);
            let (px, stride) = render_collapsed_frame(&renderer, &dims);
            let slot = resolve_collapsed_layout(&dims).visualizer_bounds;
            bar_columns(&px, stride, &dims, slot)
        };
        let missing = shown(None);
        assert_one_gradient(&renderer, &missing, "missing artwork");
        for (name, art) in [("grey", grey), ("mono", mono), ("clear", clear)] {
            let columns = shown(Some(art));
            assert_one_gradient(&renderer, &columns, name);
            for (a, b) in columns.iter().zip(&missing) {
                assert!(
                    close(at_share(a, 0.5), at_share(b, 0.5), 3),
                    "{name}: the accent's gradient"
                );
            }
        }
        renderer.set_media(None);
    }

    #[test]
    fn test_bars_are_sharp_round_and_in_their_slot_at_all_dpis() {
        let renderer = Renderer::new().expect("renderer");
        for dpi in ALL_DPIS {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
            let (px, stride, slot) = bars_frame(&renderer, &dims, solid_cover([60, 140, 240]));
            let (bar, gap, full) = visualizer_metrics(dims.scale);
            let lit = |x: usize, y: usize| px[y * stride + x] & 0xFF > 40;
            let cy = ((slot.top + slot.bottom) / 2.0) as usize;
            let x0 = slot.left.round() as usize;
            for i in 0..4 {
                let left = x0 + i * (bar + gap) as usize;
                // Whole-pixel columns: the bar's own columns fully lit at its
                // middle, the gap columns dark (no half-pixel smear)
                for x in left..left + bar as usize {
                    assert!(lit(x, cy), "bar {i} column at {dpi}");
                }
                if i < 3 {
                    for x in left + bar as usize..left + (bar + gap) as usize {
                        assert!(!lit(x, cy), "gap after bar {i} at {dpi}");
                    }
                }
                // Within the slot's height
                for y in 0..dims.height as usize {
                    if lit(left, y) {
                        assert!(
                            (y as f32) >= cy as f32 - full / 2.0 - 1.0
                                && (y as f32) <= cy as f32 + full / 2.0 + 1.0,
                            "bar {i} height at {dpi}"
                        );
                    }
                }
            }
            // Nothing outside the slot
            for y in 0..dims.height as usize {
                assert!(
                    !lit(x0 - 2, y) && !lit((slot.right + 2.0) as usize, y),
                    "outside at {dpi}"
                );
            }
        }
        renderer.set_media(None);
    }

    #[test]
    fn test_bar_gradient_blends_to_a_new_one_and_snaps_under_reduced_motion() {
        let t0 = std::time::Instant::now();
        let ms = std::time::Duration::from_millis;
        let colour = |r, g, b| D2D1_COLOR_F { r, g, b, a: 1.0 };
        let red = [colour(1.0, 0.3, 0.2), colour(1.0, 0.2, 0.1)];
        let blue = [colour(0.2, 0.5, 1.0), colour(0.5, 0.2, 1.0)];
        let mut b = VizBlend::default();
        b.retarget(red, t0, false);
        assert_eq!(b.current(t0), red, "the first gradient appears at once");
        b.retarget(blue, t0, false);
        assert_eq!(b.current(t0), red, "starts from where it is: no jump");
        // Midway: both ends in between, and still one coherent gradient
        // (the same for every bar: the ramp is the only colour source)
        let mid = b.current(t0 + ms(225));
        for (m, (f, t)) in mid.iter().zip(red.iter().zip(&blue)) {
            for (x, (lo, hi)) in [(m.r, (f.r, t.r)), (m.b, (f.b, t.b))] {
                assert!(x > lo.min(hi) && x < lo.max(hi), "in between: {mid:?}");
            }
        }
        let steps: Vec<_> = (0..=16)
            .map(|k| viz_ramp_colour(mid, k as f32 / 16.0, 1.0))
            .collect();
        for w in steps.windows(2) {
            let d = (w[0].r - w[1].r).abs() + (w[0].g - w[1].g).abs() + (w[0].b - w[1].b).abs();
            assert!(d < 0.12, "smooth along the bar, no seam: {steps:?}");
        }
        assert_eq!(b.current(t0 + ms(450)), blue, "there in 450 ms");
        // The same gradient again (a repaint, a duplicate update): unchanged
        let settled = b;
        b.retarget(blue, t0 + ms(500), false);
        assert_eq!(b, settled);
        // A third gradient mid-blend: carries on from the current colours
        let mut c = VizBlend::default();
        c.retarget(red, t0, false);
        c.retarget(blue, t0, false);
        let now = c.current(t0 + ms(200));
        c.retarget(red, t0 + ms(200), false);
        assert_eq!(c.current(t0 + ms(200)), now, "no jump on a rapid change");
        // Reduced motion: straight to the new colours
        let mut r = VizBlend::default();
        r.retarget(red, t0, true);
        r.retarget(blue, t0, true);
        assert_eq!(r.current(t0), blue);
    }

    #[test]
    fn test_bar_gradient_is_brightest_in_the_middle_and_fades_into_black() {
        let ends = [
            D2D1_COLOR_F {
                r: 0.2,
                g: 0.5,
                b: 1.0,
                a: 1.0,
            },
            D2D1_COLOR_F {
                r: 0.6,
                g: 0.2,
                b: 1.0,
                a: 1.0,
            },
        ];
        let at = |y| viz_ramp_colour(ends, y, 1.0);
        assert!(
            at(0.5).b > at(0.0).b && at(0.5).b > at(1.0).b,
            "brightest in the middle"
        );
        assert!(at(0.0).b > 0.7 && at(1.0).b > 0.7, "restrained at the ends");
        // Hue mixed along the arc: the middle is a violet, not a grey
        let m = at(0.5);
        assert!(m.b - m.g > 0.6, "vivid middle: {m:?}");
        // Faded: the same colours toward the notch's black, still opaque
        let half = viz_ramp_colour(ends, 0.5, 0.5);
        assert!((half.b - m.b / 2.0).abs() < 1e-4 && half.a == 1.0);
    }

    #[test]
    fn test_visualizer_reuses_its_brushes_and_palette() {
        let renderer = Renderer::new().expect("renderer");
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, 96);
        // Hidden visualizer: nothing drawn, nothing made
        renderer.set_media(None);
        render_collapsed_frame(&renderer, &dims);
        assert!(
            renderer.viz_brushes.borrow().is_none(),
            "no work while hidden"
        );
        let art = solid_cover([40, 200, 120]);
        bars_frame(&renderer, &dims, art.clone());
        let raw = |r: &Renderer| {
            let b = r.viz_brushes.borrow();
            let (s, g, _) = b.as_ref().unwrap();
            (
                windows::core::Interface::as_raw(s),
                windows::core::Interface::as_raw(g),
            )
        };
        let first = raw(&renderer);
        for _ in 0..10 {
            render_collapsed_frame(&renderer, &dims);
        }
        assert_eq!(raw(&renderer), first, "the same two brushes every frame");
        // The palette is worked out once per artwork: updates of the same
        // cover keep it
        let palette = renderer.viz_palette.borrow().clone();
        assert_eq!(palette.as_ref().unwrap().0, art);
        let c = MediaContent {
            playback: crate::media::PlaybackState::Paused,
            ..content("Song", Some(art.clone()))
        };
        renderer.set_media(Some(&c));
        assert_eq!(*renderer.viz_palette.borrow(), palette);
        // A new render target drops them; the next frame makes new ones
        renderer.cleanup_cached_resources();
        assert!(renderer.viz_brushes.borrow().is_none());
        bars_frame(&renderer, &dims, art);
        assert!(renderer.viz_brushes.borrow().is_some());
        // Media gone: the next session's bars start from their own colours
        renderer.set_media(None);
        assert_eq!(renderer.viz_blend.get(), VizBlend::default());
        bars_frame(&renderer, &dims, solid_cover([40, 200, 120]));
        // No artwork: no palette, the usual accent instead
        renderer.set_media(Some(&content("Song", None)));
        assert!(renderer.viz_palette.borrow().is_none());
        renderer.set_media(None);
    }

    #[test]
    fn test_repeated_artwork_changes_keep_one_palette_and_the_same_brushes() {
        let renderer = Renderer::new().expect("renderer");
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, 120);
        bars_frame(&renderer, &dims, solid_cover([200, 40, 40]));
        let first = {
            let b = renderer.viz_brushes.borrow();
            let (s, g, _) = b.as_ref().unwrap();
            (
                windows::core::Interface::as_raw(s),
                windows::core::Interface::as_raw(g),
            )
        };
        for k in 0..200u32 {
            let c = MediaContent {
                playback: crate::media::PlaybackState::Playing,
                ..content(
                    &format!("T{k}"),
                    Some(solid_cover([(k % 200) as u8 + 30, 60, 200])),
                )
            };
            renderer.set_media(Some(&c));
            render_collapsed_frame(&renderer, &dims);
        }
        let now = {
            let b = renderer.viz_brushes.borrow();
            let (s, g, _) = b.as_ref().unwrap();
            (
                windows::core::Interface::as_raw(s),
                windows::core::Interface::as_raw(g),
            )
        };
        assert_eq!(now, first, "no new brushes per track");
        // One palette entry (the current artwork's), whatever came before
        let palette = renderer.viz_palette.borrow();
        let (art, _) = palette.as_ref().unwrap();
        assert_eq!(*art, solid_cover([229, 60, 200]));
        // The ramp was baked for the gradient now shown, no more often
        assert!(renderer.viz_baked.get().is_some());
        drop(palette);
        renderer.set_media(None);
    }

    /// Frame cost of the visualizer (offscreen, release-like work per frame:
    /// bind, clear, notch body, content with the bars, end draw). Prints
    /// Cost of every frame of a real expand animation at 100-300% scaling:
    /// the whole `render` (draw, shadow, upload to an off-screen layered
    /// window), and the shadow mask and the upload alone. Prints timings; run
    /// with `cargo test --release bench_expand -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_expand_animation_frame_cost() {
        use crate::config::AnimationState;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, SW_SHOWNOACTIVATE, ShowWindow, WS_EX_LAYERED,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
        };
        // Off screen, so the test never shows anything
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
        let clock = ClockDateState::from_pure_components(2026, 10, 6, 2, 0, 47, 0, 0, false);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for dpi in [96u32, 120, 144, 192, 240, 288] {
            let renderer = Renderer::new().expect("renderer");
            let mut anim = AnimationState::new(NotchState::Collapsed, NotchState::Expanded);
            let mut frames = Vec::new();
            while anim.step(12.0) {
                frames.push((anim.transition(), anim.current_dimensions(dpi)));
            }
            let settled = space_dimensions(NotchState::Expanded, dpi, OecaSpace::Home);
            let (mut render_us, mut worst_us, mut shadow_us, mut upload_us) =
                (0.0, 0.0f64, 0.0, 0.0);
            let reps = 20;
            for rep in 0..=reps {
                for (transition, dims) in &frames {
                    renderer.set_transition(Some(*transition));
                    let t = std::time::Instant::now();
                    renderer
                        .render(hwnd, -30000, -30000, dims, false, &clock)
                        .unwrap();
                    let full = t.elapsed().as_secs_f64() * 1e6;
                    // The shadow mask alone, as the frame rebuilt it
                    let t = std::time::Instant::now();
                    build_shadow_mask(
                        renderer.cached_bits.get() as *mut u8,
                        dims.width as usize,
                        dims.height as usize,
                        renderer.cached_capacity_w.get() as usize,
                        dims.scale,
                        dims.shadow_opacity.max(0.01),
                        &mut a,
                        &mut b,
                    );
                    let shadow = t.elapsed().as_secs_f64() * 1e6;
                    // The upload alone
                    let t = std::time::Instant::now();
                    unsafe {
                        UpdateLayeredWindow(
                            hwnd,
                            Some(renderer.cached_screen_dc.get()),
                            Some(&POINT {
                                x: -30000,
                                y: -30000,
                            }),
                            Some(&SIZE {
                                cx: dims.width,
                                cy: dims.height,
                            }),
                            Some(renderer.cached_mem_dc.get()),
                            Some(&POINT { x: 0, y: 0 }),
                            COLORREF(0),
                            Some(&BLENDFUNCTION {
                                BlendOp: AC_SRC_OVER as u8,
                                BlendFlags: 0,
                                SourceConstantAlpha: 255,
                                AlphaFormat: AC_SRC_ALPHA as u8,
                            }),
                            ULW_ALPHA,
                        )
                        .unwrap();
                    }
                    let upload = t.elapsed().as_secs_f64() * 1e6;
                    if rep > 0 {
                        render_us += full;
                        worst_us = worst_us.max(full);
                        shadow_us += shadow;
                        upload_us += upload;
                    }
                }
            }
            let n = (reps * frames.len()) as f64;
            // A settled notch for comparison (its shadow mask is reused)
            renderer.set_transition(None);
            renderer
                .render(hwnd, -30000, -30000, &settled, false, &clock)
                .unwrap();
            let t = std::time::Instant::now();
            for _ in 0..200 {
                renderer
                    .render(hwnd, -30000, -30000, &settled, false, &clock)
                    .unwrap();
            }
            let settled_us = t.elapsed().as_secs_f64() * 1e6 / 200.0;
            eprintln!(
                "BENCH expand {:>3}% {}x{} px, {} frames: render {:.0} us avg / {:.0} worst \
                 (shadow mask {:.0}, upload {:.0}); settled frame {:.0} us",
                dpi * 100 / 96,
                settled.width,
                settled.height,
                frames.len(),
                render_us / n,
                worst_us,
                shadow_us / n,
                upload_us / n,
                settled_us
            );
        }
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    /// timings; run with `cargo test --release bench_visualizer -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore]
    fn bench_visualizer_frame_cost() {
        let renderer = Renderer::new().expect("renderer");
        let c = playing_content(crate::media::PlaybackState::Playing);
        renderer.set_media(Some(&c));
        renderer.visualizer().step(1000.0);
        renderer.feedback().finish_track_fade();
        // The collapsed frame as playing media shows it: with its album art
        renderer.settle_collapsed_art();
        let clock = ClockDateState::from_pure_components(2026, 10, 6, 2, 0, 47, 0, 0, false);
        for (name, dims) in [
            (
                "collapsed",
                NotchDimensions::from_state_and_dpi(NotchState::Collapsed, 144),
            ),
            (
                "music",
                space_dimensions(NotchState::Expanded, 144, OecaSpace::Music),
            ),
        ] {
            renderer.set_space(if name == "music" {
                OecaSpace::Music
            } else {
                OecaSpace::Home
            });
            renderer.ensure_buffer(dims.width, dims.height).unwrap();
            let frame = || {
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
                    rt.FillGeometry(&renderer.create_notch_geometry(&dims).unwrap(), &fill, None);
                    match resolve_layout(&dims) {
                        ResolvedLayout::Collapsed { components, .. } => renderer
                            .render_collapsed_content(rt, &components, &dims, "2:47 PM")
                            .unwrap(),
                        ResolvedLayout::Expanded { .. } => {
                            renderer.draw_expanded(rt, &dims, &clock).unwrap()
                        }
                    }
                    rt.EndDraw(None, None).unwrap();
                }
            };
            for _ in 0..200 {
                frame();
            }
            let n = 3000;
            let t0 = std::time::Instant::now();
            for _ in 0..n {
                frame();
            }
            let per = t0.elapsed().as_secs_f64() * 1e6 / n as f64;
            eprintln!("BENCH {name}: {per:.1} us/frame over {n} frames");
        }
        // The bars alone (what this visualizer adds to a frame)
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, 144);
        let slot = resolve_collapsed_layout(&dims).visualizer_bounds;
        let accent = accent_color(Some(&c), AccentChoice::White);
        let bars = |n: usize| {
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
                for _ in 0..n {
                    renderer
                        .draw_visualizer(rt, slot, accent, dims.scale, 1.0)
                        .unwrap();
                }
                rt.EndDraw(None, None).unwrap();
            }
        };
        bars(200);
        let n = 5000;
        let t0 = std::time::Instant::now();
        bars(n);
        let per = t0.elapsed().as_secs_f64() * 1e6 / n as f64;
        eprintln!("BENCH bars only: {per:.2} us per bar group over {n} draws");
        let t0 = std::time::Instant::now();
        for _ in 0..n {
            std::hint::black_box(crate::window::is_client_animation_enabled());
        }
        eprintln!(
            "PROBE spi: {:.2} us",
            t0.elapsed().as_secs_f64() * 1e6 / n as f64
        );
        let t0 = std::time::Instant::now();
        for _ in 0..n {
            std::hint::black_box(renderer.viz_colours(accent));
        }
        eprintln!(
            "PROBE viz_colours: {:.2} us",
            t0.elapsed().as_secs_f64() * 1e6 / n as f64
        );
        // A real collapsed frame with the bars, then the same frame without
        // them: what the bars add to each frame (interleaved runs)
        {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, 144);
            renderer.set_space(OecaSpace::Home);
            let frame = || {
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
                    let ResolvedLayout::Collapsed { components, .. } = resolve_layout(&dims) else {
                        unreachable!()
                    };
                    renderer
                        .render_collapsed_content(rt, &components, &dims, "2:47 PM")
                        .unwrap();
                    rt.EndDraw(None, None).unwrap();
                }
            };
            let (mut with, mut without) = (0f64, 0f64);
            for _ in 0..6 {
                renderer.set_media(Some(&c));
                renderer.visualizer().step(1000.0);
                for _ in 0..100 {
                    frame();
                }
                let t0 = std::time::Instant::now();
                for _ in 0..1000 {
                    frame();
                }
                with += t0.elapsed().as_secs_f64();
                renderer.set_media(None);
                renderer.visualizer().step(1000.0);
                for _ in 0..100 {
                    frame();
                }
                let t0 = std::time::Instant::now();
                for _ in 0..1000 {
                    frame();
                }
                without += t0.elapsed().as_secs_f64();
            }
            eprintln!(
                "BENCH frame with bars {:.1} us, without {:.1} us, bars add {:.1} us",
                with * 1e6 / 6000.0,
                without * 1e6 / 6000.0,
                (with - without) * 1e6 / 6000.0
            );
        }
        renderer.set_space(OecaSpace::Home);
        renderer.set_media(None);
    }

    #[test]
    fn test_reduced_motion_hover_shows_at_once() {
        let mut f = ControlFeedback::default();
        f.set_clip_row(Some(2));
        f.snap_hover();
        assert_eq!(f.clip_levels(PanelHit::Row(2)).0, 1.0, "no fade in");
        assert!(!f.is_animating(), "nothing left to animate");
        f.set_clip_row(Some(4));
        f.snap_hover();
        assert_eq!(f.clip_levels(PanelHit::Row(2)).0, 0.0, "no fade out");
        assert_eq!(f.clip_levels(PanelHit::Row(4)).0, 1.0);
        f.set_clip_row(None);
        f.snap_hover();
        assert!(!f.is_animating());
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
        for s in [OecaSpace::Clipboard, OecaSpace::Music, OecaSpace::Home] {
            renderer.set_space(s);
        }
        assert_eq!(renderer.media.borrow().as_ref().unwrap().content, media);
        assert_eq!(row_kinds(&renderer).len(), 2);
        // At most the visible rows are kept, however long the history
        for i in 0..20 {
            history.add(ClipboardItem::Text(format!("t{i}")));
        }
        renderer.set_clipboard(&history);
        assert_eq!(row_kinds(&renderer).len(), crate::config::CLIPBOARD_TILES);
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
            (s(OecaSpace::Home), s(OecaSpace::Music)),
            (s(OecaSpace::Music), s(OecaSpace::Clipboard)),
            (s(OecaSpace::Clipboard), s(OecaSpace::Home)),
            (s(OecaSpace::Home), Scene::Drop),
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
            from: s(OecaSpace::Music),
            to: s(OecaSpace::Music),
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
                OecaSettings {
                    accent: a,
                    ..OecaSettings::default()
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
            for space in OecaSpace::ALL {
                renderer.set_space(space);
                for (aot, rm) in [(true, false), (false, true)] {
                    let s = OecaSettings {
                        always_on_top: aot,
                        reduced_motion: rm,
                        ..OecaSettings::default()
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
        renderer.set_settings(false, OecaSettings::default());
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
        for space in OecaSpace::ALL {
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
        use crate::dragdrop::DropPage;
        assert!(renderer.set_drop(DropPage::Ready) && !renderer.set_drop(DropPage::Ready));
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
        assert!(renderer.set_drop(DropPage::Off));
    }

    #[test]
    fn test_drop_hint_and_results_draw_a_glyph_and_a_line_of_text() {
        use crate::dragdrop::{DropOutcome, DropPage};
        let renderer = Renderer::new().expect("renderer");
        // Lit pixels in a band of rows (above / below the page's middle)
        let lit_rows = |px: &[u32], stride: usize, w: usize, rows: std::ops::Range<usize>| {
            rows.flat_map(|y| (w / 4..3 * w / 4).map(move |x| (x, y)))
                .filter(|&(x, y)| (px[y * stride + x] & 0xFF) > 120)
                .count()
        };
        for dpi in ALL_DPIS {
            let dims = drop_page_dimensions(dpi);
            let (w, body) = (
                dims.width as usize,
                (dims.height as f32 - dims.shadow_margin_bottom) as usize,
            );
            let mid = body / 2;
            let mut frames = Vec::new();
            renderer.set_drop(DropPage::Unsupported);
            frames.push(render_custom_frame(&renderer, &dims, |rt| {
                renderer.draw_drop_page(rt, &dims).unwrap()
            }));
            for outcome in [DropOutcome::Copied, DropOutcome::TooLarge] {
                assert!(renderer.set_drop_result(outcome));
                assert_eq!(renderer.drop_state(), DropPage::Result);
                frames.push(render_custom_frame(&renderer, &dims, |rt| {
                    renderer.draw_drop_result(rt, &dims).unwrap()
                }));
            }
            for (i, (px, stride)) in frames.iter().enumerate() {
                // A glyph above the middle, a line of text below it
                let glyph = lit_rows(
                    px,
                    *stride,
                    w,
                    (mid as f32 - 20.0 * dims.scale) as usize..mid,
                );
                let text = lit_rows(
                    px,
                    *stride,
                    w,
                    mid + (6.0 * dims.scale) as usize..mid + (24.0 * dims.scale) as usize,
                );
                assert!(
                    glyph > 20 && text > 20,
                    "frame {i} at {dpi}: {glyph} {text}"
                );
            }
            // The success and failure pages differ (different glyph and text)
            assert_ne!(frames[1].0, frames[2].0, "{dpi}");
        }
        assert!(renderer.set_drop(DropPage::Off));
        assert!(!renderer.drop_page());
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
        let src = Artwork::new(w, h, px).unwrap();
        // A wide tile: the centre 3:1 band is the whole image, never stretched
        let wide = thumbnail(&src, 60, 20, 0.0);
        assert_eq!((wide.width, wide.height), (60, 20));
        let wide_at = |x: usize| &wide.pixels[(10 * 60 + x) * 4..(10 * 60 + x) * 4 + 4];
        assert_eq!(wide_at(5)[1], 0xFF, "green third on the left");
        assert_eq!(wide_at(55)[0], 0xFF, "blue third on the right");
        let t = thumbnail(&src, 20, 20, 5.0);
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
        let dims = crate::layout::space_dimensions(NotchState::Expanded, 96, OecaSpace::Music);
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
            let dims = crate::layout::space_dimensions(NotchState::Expanded, dpi, OecaSpace::Music);
            let green: Vec<u8> = (0..16 * 16)
                .flat_map(|_| [0x20, 0xD0, 0x20, 0xFF])
                .collect();
            let mut c = content("Song", Some(sample_artwork(64, 64)));
            c.source.icon = Some(Artwork::new(16, 16, green).unwrap());
            assert!(c.shows_badge(), "Home would show the badge");
            renderer.set_space(OecaSpace::Music);
            let (px, stride) = music_frame(&renderer, &dims, &c);
            renderer.set_space(OecaSpace::Home);
            let green_at = |p: u32| ((p >> 8) & 0xFF) > 0xA0 && (p & 0xFF) < 0x60;
            for y in 0..dims.height as usize {
                for x in 0..dims.width as usize {
                    assert!(
                        !green_at(px[y * stride + x]),
                        "badge pixel at ({x},{y}) {dpi}"
                    );
                }
            }
            let m = resolve_media_layout_in(&dims, c.shape(), OecaSpace::Music).unwrap();
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
            let sel = resolve_space_selector_in(&dims, OecaSpace::Home).unwrap();
            renderer.set_space(OecaSpace::Home);
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
            // Solid glyph: the house is filled, not just an outline (probed
            // above the door cut-out)
            let r = sel.home;
            let (x, y) = (
                ((r.left + r.right) / 2.0) as usize,
                ((r.top + r.bottom) / 2.0 - 2.0 * dims.scale) as usize,
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
        // Without bundled fonts: a candidate that exists here, or the named
        // fallback; Bahnschrift ships with Windows, so it is never needed
        let installed = resolve_ui_font(dw, &[]);
        let candidates: Vec<&str> = UI_FONT_CANDIDATES.iter().map(|(n, _)| *n).collect();
        assert!(candidates.contains(&installed.name), "{}", installed.name);
        assert_eq!(installed.collection, None, "the installed fonts");
        assert_eq!(
            first_installed(dw, &[installed.family]),
            Some(0),
            "{} is installed",
            installed.name
        );
        // Unusable font data never fails: the installed choice is used
        let broken: &[&[u8]] = &[b"not a font", &[0; 64]];
        assert_eq!(resolve_ui_font(dw, broken), installed);
        assert_eq!(ui_font(dw), ui_font(dw), "resolved once, stable");
    }

    /// Family name of a text format, as DirectWrite will look it up.
    fn format_family(format: &IDWriteTextFormat) -> String {
        let len = unsafe { format.GetFontFamilyNameLength() } as usize;
        let mut buf = vec![0u16; len + 1];
        unsafe { format.GetFontFamilyName(&mut buf) }.expect("family name");
        String::from_utf16_lossy(&buf[..len])
    }

    #[test]
    fn test_bundled_general_sans_loads_privately_with_every_weight() {
        use windows::Win32::Graphics::DirectWrite::DWRITE_FONT_SIMULATIONS_NONE;
        let renderer = Renderer::new().expect("renderer");
        let dw = &renderer.dwrite_factory;
        let font = ui_font(dw);
        if BUNDLED_FONTS.is_empty() {
            // A build without the font files: the installed fonts are used
            assert_eq!(font.collection, None);
            return;
        }
        // Official, unmodified OpenType (CFF) files: Regular, Medium, Semibold
        assert_eq!(BUNDLED_FONTS.len(), 3);
        assert!(BUNDLED_FONTS.iter().all(|f| f.starts_with(b"OTTO")));
        assert_eq!(font.name, "General Sans");
        let collection = font.collection.clone().expect("bundled collection");
        let (mut index, mut exists) = (0u32, windows::core::BOOL(0));
        unsafe { collection.FindFamilyName(font.family, &mut index, &mut exists) }.unwrap();
        assert!(exists.as_bool());
        let family = unsafe { collection.GetFontFamily(index) }.unwrap();
        // Every weight a role uses is a real face, not a synthesized one
        for role in TextRole::ALL {
            let weight = DWRITE_FONT_WEIGHT(role.style().weight);
            let face = unsafe {
                family.GetFirstMatchingFont(
                    weight,
                    DWRITE_FONT_STRETCH_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                )
            }
            .unwrap();
            assert_eq!(unsafe { face.GetWeight() }, weight, "{role:?}");
            assert_eq!(
                unsafe { face.GetSimulations() },
                DWRITE_FONT_SIMULATIONS_NONE,
                "{role:?}"
            );
        }
    }

    #[test]
    fn test_every_notch_text_role_uses_the_ui_font() {
        let renderer = Renderer::new().expect("renderer");
        let font = ui_font(&renderer.dwrite_factory);
        for role in TextRole::ALL {
            for scale in [1.0, 1.5, 2.0] {
                let format = renderer
                    .role_format(role, scale, DWRITE_TEXT_ALIGNMENT_LEADING)
                    .unwrap();
                assert_eq!(format_family(&format), font.name, "{role:?}");
                if let Some(c) = &font.collection {
                    assert_eq!(&unsafe { format.GetFontCollection() }.unwrap(), c);
                }
            }
        }
        // The clock's own format (sized by the notch) too
        let format = renderer
            .create_text_format(12.0, DWRITE_FONT_WEIGHT(600), DWRITE_FONT_STYLE_NORMAL)
            .unwrap();
        assert_eq!(format_family(&format), font.name);
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
        let on = OecaSettings::default();
        let off = OecaSettings {
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
        let reduced = OecaSettings {
            reduced_motion: true,
            ..off
        };
        renderer.set_settings(true, reduced);
        assert!(!renderer.switches_animating());
        assert_eq!(renderer.switches.borrow()[1].level(), 1.0);
    }

    // ---- Countdown timer ----------------------------------------------------

    /// The timer's orange (anti-aliased edges included).
    fn timer_in(px: &[u32], stride: usize, r: RectF) -> bool {
        (r.top.floor().max(0.0) as usize..r.bottom.ceil() as usize).any(|y| {
            (r.left.floor().max(0.0) as usize..r.right.ceil() as usize).any(|x| {
                let p = px[y * stride + x];
                let (b, g, rr) = (p & 0xFF, (p >> 8) & 0xFF, (p >> 16) & 0xFF);
                rr > 120 && rr > g + 30 && g > b + 30
            })
        })
    }

    /// Countdowns in each phase (a 5-second one finished just now).
    fn countdowns() -> [(&'static str, crate::timer::Countdown); 4] {
        use crate::timer::Countdown;
        let now = std::time::Instant::now();
        let idle = Countdown::default();
        let mut running = Countdown::default();
        running.primary(now);
        let mut paused = running;
        paused.primary(now);
        let mut done = Countdown::default();
        done.set_stop(0);
        done.primary(now - std::time::Duration::from_secs(10));
        assert!(done.poll(now).is_some());
        [
            ("idle", idle),
            ("running", running),
            ("paused", paused),
            ("done", done),
        ]
    }

    fn timer_media() -> MediaContent {
        // No artwork: nothing orange but the timer
        MediaContent {
            playback: crate::media::PlaybackState::Playing,
            ..content("Song", None)
        }
    }

    fn expanded_frame(renderer: &Renderer, dims: &NotchDimensions) -> (Vec<u32>, usize) {
        let clock = ClockDateState::from_pure_components(2026, 10, 6, 2, 0, 47, 0, 0, false);
        renderer.feedback().finish_track_fade();
        render_custom_frame(renderer, dims, |rt| {
            renderer.draw_expanded(rt, dims, &clock).unwrap()
        })
    }

    #[test]
    fn test_home_timer_renders_every_state_with_and_without_media_at_all_dpis() {
        let renderer = Renderer::new().expect("renderer");
        renderer.set_space(OecaSpace::Home);
        for dpi in ALL_DPIS {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            for media in [None, Some(timer_media())] {
                renderer.set_media(media.as_ref());
                let shape = media.as_ref().map(MediaContent::shape);
                let home = crate::layout::resolve_home_timer_layout(&dims, shape).unwrap();
                let l = home.timer;
                for (name, countdown) in countdowns() {
                    renderer.set_timer(TimerView {
                        countdown,
                        ..Default::default()
                    });
                    let (px, stride) = expanded_frame(&renderer, &dims);
                    let idle = name == "idle";
                    let at = format!("{name} at {dpi} (media: {})", media.is_some());
                    assert!(lit_in(&px, stride, l.display(idle), 60), "numerals, {at}");
                    // Orange numerals while it runs and once done; white
                    // during setup, dimmed while paused
                    let orange = timer_in(&px, stride, l.display(idle));
                    assert_eq!(orange, matches!(name, "running" | "done"), "{at}");
                    assert!(timer_in(&px, stride, l.primary), "primary button, {at}");
                    assert_eq!(lit_in(&px, stride, l.cancel, 60), !idle, "Cancel, {at}");
                    // Setup: the scale with its orange indicator; otherwise
                    // a quiet status line
                    assert!(lit_in(&px, stride, l.row, 40), "row, {at}");
                    assert_eq!(timer_in(&px, stride, l.row), idle, "indicator, {at}");
                    // The rest of Home is still there
                    match &media {
                        Some(c) => {
                            let m = resolve_media_layout(&dims, c.shape()).unwrap();
                            assert!(lit_in(&px, stride, m.title_bounds, 60), "title, {at}");
                            assert!(lit_in(&px, stride, m.time_bounds, 60), "clock, {at}");
                            let play = m.control_visual_bounds(MediaControl::PlayPause);
                            assert!(lit_in(&px, stride, play, 60), "controls, {at}");
                        }
                        None => {
                            let (time, date) = home.clock.unwrap();
                            assert!(lit_in(&px, stride, time, 60), "clock, {at}");
                            assert!(lit_in(&px, stride, date, 60), "date, {at}");
                        }
                    }
                }
            }
        }
        renderer.set_media(None);
        renderer.set_timer(TimerView::default());
    }

    #[test]
    fn test_long_timer_numerals_fit_before_the_buttons_at_all_dpis() {
        let renderer = Renderer::new().expect("renderer");
        renderer.set_space(OecaSpace::Home);
        let now = std::time::Instant::now();
        let mut longest = crate::timer::Countdown::default();
        longest.set_stop(crate::timer::STOPS.len() - 1); // 12:00:00
        let mut running = longest;
        running.primary(now);
        for dpi in ALL_DPIS {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            for media in [None, Some(timer_media())] {
                renderer.set_media(media.as_ref());
                let shape = media.as_ref().map(MediaContent::shape);
                let l = crate::layout::resolve_home_timer_layout(&dims, shape)
                    .unwrap()
                    .timer;
                for (countdown, idle) in [(longest, true), (running, false)] {
                    renderer.set_timer(TimerView {
                        countdown,
                        ..Default::default()
                    });
                    let (px, stride) = expanded_frame(&renderer, &dims);
                    let d = l.display(idle);
                    let first = if idle { l.primary } else { l.cancel };
                    // Scaled to fit: nothing between the numerals and the buttons
                    let gap = RectF::new(d.right + 1.0, d.top, first.left - 1.0, d.bottom);
                    assert!(!lit_in(&px, stride, gap, 30), "overflow at {dpi} {idle}");
                    let before =
                        RectF::new(l.panel.left - 6.0, d.top, l.panel.left - 1.0, d.bottom);
                    assert!(!lit_in(&px, stride, before, 30), "left spill at {dpi}");
                    assert!(lit_in(&px, stride, d, 60), "drawn at {dpi}");
                }
            }
        }
        renderer.set_media(None);
        renderer.set_timer(TimerView::default());
    }

    #[test]
    fn test_timer_shows_only_in_home_not_music_clipboard_or_settings() {
        let renderer = Renderer::new().expect("renderer");
        let [_, (_, running), _, _] = countdowns();
        renderer.set_timer(TimerView {
            countdown: running,
            ..Default::default()
        });
        renderer.set_media(Some(&timer_media()));
        for dpi in [96u32, 144] {
            let home = NotchDimensions::from_state_and_dpi(NotchState::Expanded, dpi);
            let whole = |d: &NotchDimensions| RectF::new(0.0, 0.0, d.width as f32, d.height as f32);
            renderer.set_space(OecaSpace::Home);
            let (px, stride) = expanded_frame(&renderer, &home);
            assert!(
                timer_in(&px, stride, whole(&home)),
                "Home shows it at {dpi}"
            );
            for space in [OecaSpace::Music, OecaSpace::Clipboard] {
                renderer.set_space(space);
                let dims = space_dimensions(NotchState::Expanded, dpi, space);
                let (px, stride) = expanded_frame(&renderer, &dims);
                assert!(!timer_in(&px, stride, whole(&dims)), "{space:?} at {dpi}");
            }
            // The Settings page over Home
            renderer.set_space(OecaSpace::Home);
            renderer.set_settings(true, OecaSettings::default());
            let dims = space_dimensions(NotchState::Expanded, dpi, PANEL_SPACE);
            let (px, stride) = expanded_frame(&renderer, &dims);
            assert!(!timer_in(&px, stride, whole(&dims)), "Settings at {dpi}");
            renderer.set_settings(false, OecaSettings::default());
            // Collapsed without its slot: the notch as it always was
            let collapsed = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
            let (px, stride) = render_collapsed_frame(&renderer, &collapsed);
            assert!(
                !timer_in(&px, stride, whole(&collapsed)),
                "collapsed at {dpi}"
            );
        }
        renderer.set_media(None);
        renderer.set_timer(TimerView::default());
    }

    fn solid_art(bgr: [u8; 3]) -> Artwork {
        Artwork::new(
            60,
            40,
            (0..60 * 40)
                .flat_map(|_| [bgr[0], bgr[1], bgr[2], 0xFF])
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn test_collapsed_art_fades_retarget_and_settle() {
        let (a, b) = (solid_art([0, 0, 200]), solid_art([200, 0, 0]));
        let value = |art: &CollapsedArt, x: &Artwork| {
            art.layers
                .iter()
                .find(|(c, _)| c == x)
                .map(|(_, g)| g.value)
        };
        // Appears: rises monotonically to exactly 1, never past it, then stops
        let mut art = CollapsedArt::default();
        art.retarget(Some(&a), true);
        assert_eq!(value(&art, &a), Some(0.0), "starts invisible: no pop");
        let (mut last, mut ms) = (0.0, 0.0);
        while art.step(16.0) {
            let v = value(&art, &a).unwrap();
            assert!(v >= last && v <= 1.0, "monotonic, no overshoot: {v}");
            last = v;
            ms += 16.0;
            assert!(ms < 1500.0, "settles");
        }
        assert_eq!(value(&art, &a), Some(1.0));
        assert!(!art.is_animating(), "settled: no frames");
        // A refresh of the same cover changes nothing
        art.retarget(Some(&a.clone()), true);
        assert!(!art.is_animating());
        // Disappears, reversed mid-way: continues from where it is (no jump)
        art.retarget(None, true);
        for _ in 0..6 {
            art.step(16.0);
        }
        let mid = value(&art, &a).unwrap();
        assert!(mid > 0.0 && mid < 1.0, "fading: {mid}");
        art.retarget(Some(&a), true);
        assert_eq!(value(&art, &a), Some(mid), "reversal keeps the position");
        art.step(16.0);
        let next = value(&art, &a).unwrap();
        assert!((next - mid).abs() < 0.08, "continuous: {mid} -> {next}");
        while art.step(16.0) {}
        assert_eq!(value(&art, &a), Some(1.0));
        // Track change: a cross-fade, the old cover dropped once gone
        art.retarget(Some(&b), true);
        art.step(16.0);
        let (va, vb) = (value(&art, &a).unwrap(), value(&art, &b).unwrap());
        assert!(va < 1.0 && vb > 0.0, "both mid-fade: {va} {vb}");
        while art.step(16.0) {}
        assert_eq!(art.layers.len(), 1);
        assert_eq!(value(&art, &b), Some(1.0));
        // Rapid skips: bounded layers, the newest heading in
        for i in 0..8u8 {
            art.retarget(Some(&solid_art([i, 10, 10])), true);
            art.step(30.0);
            assert!(art.layers.len() <= COLLAPSED_ART_LAYERS);
            let (_, g) = art.layers.last().unwrap();
            assert_eq!(g.target, 1.0);
        }
        while art.step(16.0) {}
        assert_eq!(art.layers.len(), 1);
        // No artwork: gone; Reduced motion: snaps both ways
        art.retarget(None, false);
        assert!(art.layers.is_empty() && !art.is_animating());
        art.retarget(Some(&a), false);
        assert_eq!(value(&art, &a), Some(1.0));
        assert!(!art.is_animating());
        // Off screen: settle ends a fade at once
        art.retarget(Some(&b), true);
        art.settle();
        assert_eq!(art.layers.len(), 1);
        assert_eq!(value(&art, &b), Some(1.0));
    }

    #[test]
    fn test_collapsed_art_left_of_clock_inside_notch_all_dpis() {
        let renderer = Renderer::new().expect("renderer");
        let red = solid_art([0x20, 0x30, 0xE0]);
        let blue = solid_art([0xE0, 0x30, 0x20]);
        let frame = |dims: &NotchDimensions| {
            render_custom_frame(&renderer, dims, |rt| {
                renderer
                    .render_collapsed_content(rt, &resolve_collapsed_layout(dims), dims, "12:59 PM")
                    .unwrap()
            })
        };
        let at = |px: &[u32], stride: usize, x: f32, y: f32| {
            let v = px[y as usize * stride + x as usize];
            [v as u8, (v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8]
        };
        for dpi in ALL_DPIS {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
            let (w, h) = (dims.width, dims.height);
            let l = resolve_collapsed_layout(&dims);
            let a = l.artwork_bounds;
            // Geometry: a whole-pixel square, centred, at the content's left
            // end, clear of the bottom corner curve and of the bars / slot
            assert_eq!(a.width(), a.height(), "square at {dpi}");
            assert_eq!(a.width(), (18.0 * dims.scale).round(), "size at {dpi}");
            assert!(
                [a.left, a.top, a.right, a.bottom]
                    .iter()
                    .all(|v| v.fract() == 0.0)
            );
            let c = l.content_bounds;
            assert!(a.left >= c.left.floor() && a.right <= c.right);
            assert!(a.top >= c.top && a.bottom <= c.bottom, "inside at {dpi}");
            let mid = (c.top + c.bottom) / 2.0;
            assert!(
                ((a.top + a.bottom) / 2.0 - mid).abs() <= 0.5,
                "centred at {dpi}"
            );
            let cv = &dims.curvature;
            assert!(a.left >= (cv.top_transition_radius + cv.bottom_radius).floor());
            let slot = crate::layout::timer_slot_rect(&l, dims.scale);
            assert!(a.right < l.visualizer_bounds.left && a.right < slot.left);
            assert_eq!((dims.width, dims.height), (w, h), "notch size unchanged");

            // No media: nothing in the slot; the widest clock stays clear of it
            renderer.set_media(None);
            renderer.settle_collapsed_art();
            let (bare, stride) = frame(&dims);
            assert!(!lit_in(&bare, stride, a, 12), "empty slot at {dpi}");
            let clock_left = (0..w as usize)
                .find(|&x| (0..h as usize).any(|y| bare[y * stride + x] & 0xFF > 40))
                .unwrap() as f32;
            let gap = (4.0 * dims.scale).round();
            assert!(
                clock_left >= a.right + gap,
                "clock {clock_left} vs art {} at {dpi}",
                a.right
            );

            // Artwork: fills the slot (rounded corners), nothing else changes
            renderer.set_media(Some(&content("Song", Some(red.clone()))));
            renderer.settle_collapsed_art();
            let (px, _) = frame(&dims);
            let (cx, cy) = ((a.left + a.right) / 2.0, (a.top + a.bottom) / 2.0);
            for (x, y) in [
                (cx, cy),
                (a.left + 0.5, cy),
                (a.right - 1.0, cy),
                (cx, a.top + 0.5),
            ] {
                let p = at(&px, stride, x, y);
                assert!(
                    p[2] > 0xC0 && p[0] < 0x50 && p[3] == 0xFF,
                    "red at {dpi}: {p:?}"
                );
            }
            let corner = at(&px, stride, a.left, a.top);
            assert!(corner[2] < 0x90, "rounded corner at {dpi}: {corner:?}");
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let inside = a.contains(x as f32 + 0.5, y as f32 + 0.5);
                    if !inside {
                        assert_eq!(px[y * stride + x], bare[y * stride + x], "({x},{y}) {dpi}");
                    }
                }
            }
            // A new track's cover replaces it (one cached thumbnail)
            renderer.set_media(Some(&content("Next", Some(blue.clone()))));
            renderer.settle_collapsed_art();
            let (px, _) = frame(&dims);
            let p = at(&px, stride, cx, cy);
            assert!(p[0] > 0xC0 && p[2] < 0x50, "new cover at {dpi}: {p:?}");
            assert_eq!(renderer.collapsed_thumbs.borrow().len(), 1);
            // Missing artwork: the slot empties (thumbnail released)
            renderer.set_media(Some(&content("Next", None)));
            renderer.settle_collapsed_art();
            let (px, _) = frame(&dims);
            assert!(!lit_in(&px, stride, a, 12), "no artwork at {dpi}");
            assert!(renderer.collapsed_thumbs.borrow().is_empty());
        }
        renderer.set_media(None);
    }

    #[test]
    fn test_collapsed_slot_takes_the_visualizer_place_without_stale_text() {
        use crate::media::PlaybackState as P;
        let renderer = Renderer::new().expect("renderer");
        let [_, (_, running), _, (_, done)] = countdowns();
        for dpi in ALL_DPIS {
            let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, dpi);
            let l = resolve_collapsed_layout(&dims);
            let slot = crate::layout::timer_slot_rect(&l, dims.scale);
            // The clock's text ends before the slot (nothing to overlap)
            renderer.set_media(None);
            renderer.visualizer().settle();
            renderer.set_timer(TimerView::default());
            let (px, stride) = render_collapsed_frame(&renderer, &dims);
            let clock_right = (0..dims.width as usize)
                .rev()
                .find(|&x| (0..dims.height as usize).any(|y| px[y * stride + x] & 0xFF > 40))
                .unwrap() as f32;
            assert!(
                clock_right < slot.left,
                "clock {clock_right} vs slot {} at {dpi}",
                slot.left
            );
            // The remaining time in orange, inside the slot only
            for countdown in [running, done] {
                renderer.set_timer(TimerView {
                    countdown,
                    collapsed: countdown,
                    slot: 1.0,
                    pulse: 1.0,
                });
                let (px, stride) = render_collapsed_frame(&renderer, &dims);
                assert!(timer_in(&px, stride, slot), "slot at {dpi}");
                let left = RectF::new(0.0, 0.0, slot.left - 1.0, dims.height as f32);
                assert!(!timer_in(&px, stride, left), "only in the slot at {dpi}");
            }
            // Playback with the slot: the bars give way while it is there...
            renderer.set_media(Some(&playing_content(P::Playing)));
            renderer.visualizer().step(1000.0);
            renderer.set_timer(TimerView {
                countdown: running,
                collapsed: running,
                slot: 1.0,
                pulse: 1.0,
            });
            let (px, stride) = render_collapsed_frame(&renderer, &dims);
            assert!(
                !accent_in(&px, stride, l.visualizer_bounds),
                "bars suppressed at {dpi}"
            );
            // ...and come back as it leaves (the countdown went to the bubble)
            renderer.set_timer(TimerView {
                countdown: running,
                collapsed: running,
                slot: 0.0,
                pulse: 1.0,
            });
            let (px, stride) = render_collapsed_frame(&renderer, &dims);
            assert!(
                accent_in(&px, stride, l.visualizer_bounds),
                "bars back at {dpi}"
            );
            assert!(!timer_in(&px, stride, slot), "no stale numerals at {dpi}");
            // Cancelled / dismissed: nothing left behind
            renderer.set_media(None);
            renderer.set_timer(TimerView::default());
            let (px, stride) = render_collapsed_frame(&renderer, &dims);
            assert!(!timer_in(&px, stride, slot), "cancelled at {dpi}");
        }
        renderer.set_timer(TimerView::default());
    }

    #[test]
    fn test_done_pulse_dims_the_numerals() {
        let renderer = Renderer::new().expect("renderer");
        let [_, _, _, (_, done)] = countdowns();
        let dims = NotchDimensions::from_state_and_dpi(NotchState::Collapsed, 96);
        let slot = crate::layout::timer_slot_rect(&resolve_collapsed_layout(&dims), dims.scale);
        let brightest = |pulse: f32| {
            renderer.set_timer(TimerView {
                countdown: done,
                collapsed: done,
                slot: 1.0,
                pulse,
            });
            let (px, stride) = render_collapsed_frame(&renderer, &dims);
            (slot.top as usize..slot.bottom as usize)
                .flat_map(|y| (slot.left as usize..slot.right as usize).map(move |x| (x, y)))
                .map(|(x, y)| (px[y * stride + x] >> 16) & 0xFF)
                .max()
                .unwrap()
        };
        let (full, dim) = (brightest(1.0), brightest(0.5));
        assert!(
            full > 200 && dim < full * 3 / 4,
            "pulse dims: {full} -> {dim}"
        );
        renderer.set_timer(TimerView::default());
    }
}
