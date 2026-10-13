//! Windows media session discovery, metadata and playback state
//! (Windows.Media.Control / SMTC).
//!
//! Threading model:
//! - Async WinRT operations (`RequestAsync`, `TryGetMediaPropertiesAsync`) are never
//!   awaited on the UI thread. Their completion callbacks (`IAsyncOperation::when`,
//!   provided by `windows-future`, already a dependency of `windows`) run on a WinRT
//!   thread-pool thread, hand the result over through an mpsc channel and post a
//!   private `WM_APP_MEDIA_*` message to the Oeca window.
//! - Event handlers (`CurrentSessionChanged`, `MediaPropertiesChanged`,
//!   `PlaybackInfoChanged`) only post a message. They capture nothing but the HWND
//!   value, so they can never touch `WindowState`.
//! - All `MediaEngine` state is read and written on the UI thread only.
//! - Metadata results are tagged with a session generation; results that arrive
//!   after the session was replaced are discarded, so old metadata cannot leak.
//! - COM is not initialized explicitly: windows-core joins the implicit MTA
//!   (`CoIncrementMTAUsage`) on first activation, which lives until process exit.

use std::sync::Arc;

use crate::config::{
    MEDIA_HOVER_FADE_MS, MEDIA_PRESS_IN_MS, MEDIA_PRESS_RELEASE_MS, MEDIA_TRACK_FADE_FLOOR,
    MEDIA_TRACK_FADE_MS, VISUALIZER_FADE_MS, VISUALIZER_MIN_HEIGHT,
};
use std::sync::mpsc::{Receiver, Sender, channel};

use windows::Foundation::TypedEventHandler;
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform,
    ColorManagementMode, ExifOrientationMode,
};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Storage::Streams::IRandomAccessStreamReference;
use windows::System::Threading::{ThreadPool, WorkItemHandler};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, KF_FLAG_DEFAULT,
    SHCreateItemInKnownFolder, SHGetKnownFolderItem, SIGDN, SIGDN_NORMALDISPLAY,
    SIGDN_PARENTRELATIVEPARSING,
};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use windows::core::{HSTRING, Result};

/// Posted when `RequestAsync()` completes (successfully or not).
pub const WM_APP_MEDIA_MANAGER_READY: u32 = WM_APP + 1;
/// Posted when Windows reports `CurrentSessionChanged`.
pub const WM_APP_MEDIA_SESSION_CHANGED: u32 = WM_APP + 2;
/// Posted when the current session reports `MediaPropertiesChanged`.
pub const WM_APP_MEDIA_PROPERTIES_CHANGED: u32 = WM_APP + 3;
/// Posted when a `TryGetMediaPropertiesAsync()` request completes.
pub const WM_APP_MEDIA_PROPERTIES_READY: u32 = WM_APP + 4;
/// Posted when the current session reports `PlaybackInfoChanged`.
pub const WM_APP_MEDIA_PLAYBACK_CHANGED: u32 = WM_APP + 5;
/// Posted when an artwork decode for a metadata request finishes (Some or None).
pub const WM_APP_MEDIA_ARTWORK_READY: u32 = WM_APP + 6;
/// Posted when a source-application name lookup finishes.
pub const WM_APP_MEDIA_SOURCE_READY: u32 = WM_APP + 7;
/// Posted when the current session reports `TimelinePropertiesChanged`.
pub const WM_APP_MEDIA_TIMELINE_CHANGED: u32 = WM_APP + 8;

/// True for the private media messages handled by `MediaEngine::handle_message`.
pub fn is_media_message(msg: u32) -> bool {
    (WM_APP_MEDIA_MANAGER_READY..=WM_APP_MEDIA_TIMELINE_CHANGED).contains(&msg)
}

/// Longest edge of decoded artwork. Thumbnails are scaled down (never up) to this,
/// which bounds memory to 256 KiB per image.
pub const ARTWORK_MAX_EDGE: u32 = 256;

/// Decoded album artwork owned by Oeca (no WinRT objects): BGRA8, premultiplied
/// alpha, tightly packed (stride = width * 4). Ready for a later
/// `ID2D1RenderTarget::CreateBitmap`. Cloning shares the pixel buffer.
#[derive(Clone)]
pub struct Artwork {
    pub width: u32,
    pub height: u32,
    /// Consumed by the later artwork-rendering phase.
    #[allow(dead_code)]
    pub pixels: Arc<[u8]>,
    /// Content fingerprint used for cheap change detection.
    fingerprint: u64,
}

impl Artwork {
    /// Validates dimensions/buffer length; returns None for empty or malformed data.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        if width == 0 || height == 0 || pixels.len() != expected {
            return None;
        }
        // FNV-1a over dimensions and pixels
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in width
            .to_le_bytes()
            .iter()
            .chain(height.to_le_bytes().iter())
            .chain(pixels.iter())
        {
            h = (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
        }
        Some(Self {
            width,
            height,
            pixels: pixels.into(),
            fingerprint: h,
        })
    }
}

impl PartialEq for Artwork {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.fingerprint == other.fingerprint
    }
}
impl Eq for Artwork {}

impl std::fmt::Debug for Artwork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Artwork({}x{}, {:016x})",
            self.width, self.height, self.fingerprint
        )
    }
}

/// Scales (w, h) down to fit within `max` on the longest edge, keeping aspect.
fn fit_within(w: u32, h: u32, max: u32) -> (u32, u32) {
    if w == 0 || h == 0 || (w <= max && h <= max) {
        return (w, h);
    }
    let scale = max as f64 / w.max(h) as f64;
    (
        ((w as f64 * scale).round() as u32).max(1),
        ((h as f64 * scale).round() as u32).max(1),
    )
}

/// Human-readable name of the session's source application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppName {
    /// Lookup in flight.
    Pending,
    /// Name Windows provided for the AUMID.
    Available(String),
    /// Windows could not resolve the AUMID; only `app_id` is known.
    Unavailable,
}

/// Source application of the current session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceApp {
    /// Raw AUMID reported by Windows (kept even when the name is unavailable).
    pub app_id: String,
    pub name: AppName,
    /// Badge icon (squircle-masked BGRA) for apps in `BADGE_APPS`; None otherwise.
    pub icon: Option<Artwork>,
}

/// Apps that get a squircle source badge on the artwork, matched against the
/// display name Windows resolves (case-insensitive substring).
pub const BADGE_APPS: [&str; 3] = ["spotify", "apple music", "youtube music"];

/// Pixel size the badge icon is extracted and masked at (scaled down when drawn).
pub const BADGE_ICON_PX: u32 = 64;

fn wants_badge(name: &str) -> bool {
    let name = name.to_lowercase();
    BADGE_APPS.iter().any(|app| name.contains(app))
}

/// Turns an extracted app icon (BGRA, straight or premultiplied alpha) into a
/// badge: composited over a plate in the icon's own dominant colour (so a round
/// logo becomes a filled squircle tile; dark plate for neutral icons) and masked
/// to a squircle with anti-aliased edges. Returns premultiplied BGRA. Runs once per app, off the UI thread.
pub fn badge_pixels(mut px: Vec<u8>, w: u32, h: u32) -> Option<Artwork> {
    let n = (w as usize).checked_mul(h as usize)?;
    if w == 0 || h == 0 || px.len() != n * 4 {
        return None;
    }
    // Shell bitmaps may carry straight alpha: premultiply if any channel exceeds alpha
    let straight = px
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| p[0] > p[3] || p[1] > p[3] || p[2] > p[3]);
    // Opaque pixels look the same straight or premultiplied, so this is safe
    // before premultiplying.
    let plate = dominant_rgb(&px, w as usize, h as usize).map_or(
        [0x22u8, 0x20, 0x1e], // BGR of a near-black warm plate
        |[r, g, b]| [b, g, r].map(|v| (v * 255.0).round() as u8),
    );
    let (fw, fh) = (w as f32, h as f32);
    let radius = fw.min(fh) * crate::config::BADGE_CORNER_FRACTION;
    for (i, p) in px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let a = p[3] as u32;
        for c in 0..3 {
            let v = if straight {
                p[c] as u32 * a / 255
            } else {
                p[c] as u32
            };
            // icon over opaque plate
            p[c] = (v + plate[c] as u32 * (255 - a) / 255).min(255) as u8;
        }
        // Rounded-square coverage (1 px anti-aliasing at the curved edge)
        let (x, y) = ((i as u32 % w) as f32 + 0.5, (i as u32 / w) as f32 + 0.5);
        let dx = (radius - x).max(x - (fw - radius)).max(0.0);
        let dy = (radius - y).max(y - (fh - radius)).max(0.0);
        let coverage = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
        for c in p.iter_mut() {
            *c = (*c as f32 * coverage).round() as u8;
        }
        if coverage > 0.0 {
            p[3] = (255.0 * coverage).round() as u8;
        }
    }
    Artwork::new(w, h, px)
}

// ---- Artwork accent ----------------------------------------------------------

/// Accent colour as sRGB bytes `[r, g, b]`.
pub type Accent = [u8; 3];

/// Pixels with less chroma (max - min channel) than this are treated as grey.
const ACCENT_MIN_CHROMA: f32 = 0.15;
/// Pixels darker than this (max channel) carry no usable hue.
const ACCENT_MIN_VALUE: f32 = 0.12;
/// Mean chroma per sample below which the artwork counts as neutral.
const ACCENT_MIN_COVERAGE: f32 = 0.02;
/// Normalized accent range (HSL): premium on pure black, never dark or neon.
const ACCENT_SATURATION: (f32, f32) = (0.35, 0.75);
const ACCENT_LIGHTNESS: (f32, f32) = (0.52, 0.68);
/// Upper bound on the accent's luma (bright hues such as yellow/green are dimmed).
const ACCENT_MAX_LUMA: f32 = 0.42;
const ACCENT_MIN_LIGHTNESS: f32 = 0.36;

fn luma([r, g, b]: [f32; 3]) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// RGB (0..=1) -> HSL (hue 0..1).
fn to_hsl([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let c = max - min;
    if c <= f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let s = c / (1.0 - (2.0 * l - 1.0).abs()).max(f32::EPSILON);
    let h = if max == r {
        ((g - b) / c).rem_euclid(6.0)
    } else if max == g {
        (b - r) / c + 2.0
    } else {
        (r - g) / c + 4.0
    };
    (h / 6.0, s.min(1.0), l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(1.0) * 6.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m]
}

/// Clamps saturation and lightness into the premium range, then dims hues that
/// would still read as neon on black.
fn normalize_accent(rgb: [f32; 3]) -> Accent {
    let (h, s, l) = to_hsl(rgb);
    let s = s.clamp(ACCENT_SATURATION.0, ACCENT_SATURATION.1);
    let mut l = l.clamp(ACCENT_LIGHTNESS.0, ACCENT_LIGHTNESS.1);
    while luma(from_hsl(h, s, l)) > ACCENT_MAX_LUMA && l > ACCENT_MIN_LIGHTNESS {
        l -= 0.02;
    }
    from_hsl(h, s, l).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Derives one accent colour from already-decoded artwork (no decoding, no
/// allocation): a ~64x64 sample grid votes into 24 hue bins weighted by chroma;
/// the strongest hue (with its neighbours) is averaged and normalized. Returns
/// `None` for neutral (grey/black/white or transparent) artwork. Deterministic.
pub fn artwork_accent(art: &Artwork) -> Option<Accent> {
    dominant_rgb(&art.pixels, art.width as usize, art.height as usize).map(normalize_accent)
}

/// Chroma-weighted dominant hue of premultiplied BGRA pixels, as averaged RGB
/// (0..=1), or None when the image is essentially neutral.
fn dominant_rgb(pixels: &[u8], w: usize, h: usize) -> Option<[f32; 3]> {
    const BINS: usize = 24;
    let mut weight = [0f32; BINS];
    let mut sum = [[0f32; 3]; BINS];
    let step = (w.max(h) / 64).max(1);
    let mut samples = 0f32;
    for y in (0..h).step_by(step) {
        for x in (0..w).step_by(step) {
            let i = (y * w + x) * 4;
            let [b, g, r, a] = [0, 1, 2, 3].map(|k| f32::from(pixels[i + k]));
            samples += 1.0;
            if a < 128.0 {
                continue;
            }
            // Unpremultiply to 0..=1
            let rgb = [r / a, g / a, b / a].map(|v| v.min(1.0));
            let max = rgb[0].max(rgb[1]).max(rgb[2]);
            let chroma = max - rgb[0].min(rgb[1]).min(rgb[2]);
            if chroma < ACCENT_MIN_CHROMA || max < ACCENT_MIN_VALUE {
                continue;
            }
            let bin = ((to_hsl(rgb).0 * BINS as f32) as usize).min(BINS - 1);
            weight[bin] += chroma;
            for k in 0..3 {
                sum[bin][k] += rgb[k] * chroma;
            }
        }
    }
    let total: f32 = weight.iter().sum();
    if samples == 0.0 || total / samples < ACCENT_MIN_COVERAGE {
        return None;
    }
    let around = |i: usize| [(i + BINS - 1) % BINS, i, (i + 1) % BINS];
    let score = |i: usize| around(i).iter().map(|&j| weight[j]).sum::<f32>();
    let best = (0..BINS).fold(0, |b, i| if score(i) > score(b) { i } else { b });
    let mut rgb = [0f32; 3];
    for j in around(best) {
        for k in 0..3 {
            rgb[k] += sum[j][k];
        }
    }
    Some(rgb.map(|v| v / score(best)))
}

// ---- Visualizer palette -------------------------------------------------------

/// The visualizer's one gradient, taken from the artwork: the colour at the
/// top end of every bar and the colour at its bottom end (sRGB bytes). Every
/// bar shows this same gradient. Separate from `Accent` (the scrubber and the
/// rest keep that one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VizGradient {
    pub top: Accent,
    pub bottom: Accent,
}

/// A second colour joins the gradient only with at least this share of the
/// dominant one, and only this many hue bins (10 degrees each) apart: far
/// enough to read as two colours, not so far (near-complementary) that the
/// blend between them would pass through a muddy middle.
const GRADIENT_SECOND_SHARE: f32 = 0.25;
const GRADIENT_HUE_BINS: (usize, usize) = (3, 15);

/// Samples darker than this (HSV value) are shadows, without usable colour.
const PALETTE_MIN_VALUE: f32 = 0.2;
/// Samples with less chroma than this are greys / whites / near-blacks.
const PALETTE_MIN_CHROMA: f32 = 0.12;
/// Colourful weight per sample below which the artwork counts as neutral.
const PALETTE_MIN_COVERAGE: f32 = 0.03;
/// Every bar colour is at least this bright on black (relative luminance,
/// linear light): visible, yet the colour itself is kept.
const PALETTE_MIN_LUMINANCE: f32 = 0.16;

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Relative luminance (WCAG) of sRGB 0..=1.
fn relative_luminance(rgb: [f32; 3]) -> f32 {
    let [r, g, b] = rgb.map(srgb_to_linear);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// HSV (hue 0..1) of sRGB 0..=1.
fn to_hsv([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let c = max - r.min(g).min(b);
    let h = if c <= f32::EPSILON {
        0.0
    } else if max == r {
        ((g - b) / c).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / c + 2.0) / 6.0
    } else {
        ((r - g) / c + 4.0) / 6.0
    };
    (h, if max > 0.0 { c / max } else { 0.0 }, max)
}

fn from_hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let c = v * s;
    let hp = h.rem_euclid(1.0) * 6.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

/// A cluster's colour as shown on black: its own hue and saturation, at its
/// vivid samples' brightness; only if that would be too dim to read on
/// black is it lifted (brighter first, then a touch toward white), never
/// greyed and never pushed past its own saturation.
fn palette_colour(h: f32, s: f32, v: f32) -> Accent {
    let (mut s, mut v) = (s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    while relative_luminance(from_hsv(h, s, v)) < PALETTE_MIN_LUMINANCE && v < 1.0 {
        v = (v + 0.02).min(1.0);
    }
    while relative_luminance(from_hsv(h, s, v)) < PALETTE_MIN_LUMINANCE && s > 0.45 {
        s -= 0.02;
    }
    from_hsv(h, s, v).map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The colour `t` (0..=1) of the way from `a` to `b` (sRGB 0..=1): hue
/// along the shorter arc, saturation and brightness straight across, so a
/// blend of two vivid colours stays vivid (no grey or muddy middle). A
/// colourless end takes the other end's hue.
pub fn gradient_mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let ((ha, sa, va), (hb, sb, vb)) = (to_hsv(a), to_hsv(b));
    let (ha, hb) = match (sa < 0.02, sb < 0.02) {
        (true, false) => (hb, hb),
        (false, true) => (ha, ha),
        _ => (ha, hb),
    };
    let dh = (hb - ha + 0.5).rem_euclid(1.0) - 0.5;
    from_hsv(ha + dh * t, sa + (sb - sa) * t, va + (vb - va) * t)
}

/// The visualizer gradient of already-decoded artwork (no decoding; runs
/// once per artwork, on a ~48 x 48 sample grid). Each colourful sample (not
/// a shadow, grey, white or transparent pixel) votes for its hue, weighted by
/// how vivid and bright it is. The dominant hue cluster gives the main
/// colour, rebuilt from its own hue (circular mean) and its samples'
/// saturation and brightness, so tones are never averaged into grey.
/// - A second prominent colour (a real share of the dominant one, a clearly
///   different but not opposite hue) makes it a two-colour gradient: the
///   dominant colour at the top, the second at the bottom.
/// - Otherwise it is a quiet ramp of the dominant colour itself: a little
///   lighter at the top, a little deeper at the bottom.
///
/// None for neutral artwork (grey, black and white, transparent): the
/// caller then uses its usual accent. Deterministic.
pub fn visualizer_gradient(art: &Artwork) -> Option<VizGradient> {
    const BINS: usize = 36;
    let (w, h) = (art.width as usize, art.height as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let mut weight = [0f32; BINS];
    let mut hue = [[0f32; 2]; BINS];
    let mut sat = [0f32; BINS];
    let mut val = [0f32; BINS];
    let step = (w.max(h) / 48).max(1);
    let mut samples = 0f32;
    for y in (0..h).step_by(step) {
        for x in (0..w).step_by(step) {
            let i = (y * w + x) * 4;
            let [b, g, r, a] = [0, 1, 2, 3].map(|k| f32::from(art.pixels[i + k]));
            samples += 1.0;
            if a < 128.0 {
                continue;
            }
            // Premultiplied BGRA -> straight sRGB 0..=1
            let rgb = [r / a, g / a, b / a].map(|v| v.min(1.0));
            let (hh, ss, vv) = to_hsv(rgb);
            let chroma = ss * vv;
            if vv < PALETTE_MIN_VALUE || chroma < PALETTE_MIN_CHROMA {
                continue;
            }
            let wt = chroma * vv;
            let bin = ((hh * BINS as f32) as usize).min(BINS - 1);
            let ang = hh * std::f32::consts::TAU;
            weight[bin] += wt;
            hue[bin][0] += ang.cos() * wt;
            hue[bin][1] += ang.sin() * wt;
            sat[bin] += ss * wt;
            val[bin] += vv * wt;
        }
    }
    let total: f32 = weight.iter().sum();
    if samples == 0.0 || total / samples < PALETTE_MIN_COVERAGE {
        return None;
    }
    let around = |i: usize| [(i + BINS - 1) % BINS, i, (i + 1) % BINS];
    let score = |i: usize| around(i).iter().map(|&j| weight[j]).sum::<f32>();
    let distance = |a: usize, b: usize| {
        let d = a.abs_diff(b);
        d.min(BINS - d)
    };
    // The dominant cluster (ties broken by bin: deterministic)
    let first = (0..BINS).fold(0, |b, i| if score(i) > score(b) { i } else { b });
    // A second colour: the strongest cluster at a pleasing distance
    let second = (0..BINS)
        .filter(|&i| {
            let d = distance(first, i);
            d >= GRADIENT_HUE_BINS.0 && d <= GRADIENT_HUE_BINS.1
        })
        .fold(None, |best: Option<usize>, i| match best {
            Some(b) if score(b) >= score(i) => Some(b),
            _ => Some(i),
        })
        .filter(|&i| score(i) >= GRADIENT_SECOND_SHARE * score(first));
    let colour = |p: usize| {
        let (mut c, mut s_, mut v_, mut wt) = ([0f32; 2], 0f32, 0f32, 0f32);
        for j in around(p) {
            c[0] += hue[j][0];
            c[1] += hue[j][1];
            s_ += sat[j];
            v_ += val[j];
            wt += weight[j];
        }
        let hh = c[1].atan2(c[0]).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
        (hh, s_ / wt, v_ / wt)
    };
    let (h0, s0, v0) = colour(first);
    Some(match second {
        Some(p) => {
            let (h1, s1, v1) = colour(p);
            VizGradient {
                top: palette_colour(h0, s0, v0),
                bottom: palette_colour(h1, s1, v1),
            }
        }
        // One colour: lighter (a little less saturated) at the top, a little
        // deeper at the bottom: the same hue throughout
        None => VizGradient {
            top: palette_colour(h0, s0 * 0.85, (v0 + 0.1).min(1.0)),
            bottom: palette_colour(h0, s0, v0 * 0.86),
        },
    })
}

// ---- Timeline ----------------------------------------------------------------

/// FILETIME / WinRT `DateTime` ticks (100 ns since 1601-01-01 UTC) for now.
pub fn now_filetime() -> i64 {
    const UNIX_EPOCH_AS_FILETIME: i64 = 116_444_736_000_000_000;
    let since_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    UNIX_EPOCH_AS_FILETIME + (since_unix.as_nanos() / 100) as i64
}

/// Track timeline from SMTC `GetTimelineProperties`, normalized to milliseconds
/// from the track start. `stamp` is the FILETIME the position was valid at, so
/// the current position is derived without polling the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeline {
    pub duration_ms: i64,
    pub position_ms: i64,
    pub stamp: i64,
}

impl Timeline {
    /// From raw SMTC values (all in 100 ns ticks). None when the session reports
    /// no meaningful duration (e.g. live streams). An invalid `LastUpdatedTime`
    /// (zero or in the future) falls back to `now`.
    pub fn from_raw(
        start: i64,
        end: i64,
        position: i64,
        last_updated: i64,
        now: i64,
    ) -> Option<Self> {
        let duration_ms = end.saturating_sub(start) / 10_000;
        if duration_ms < 1_000 {
            return None;
        }
        Some(Self {
            duration_ms,
            position_ms: (position.saturating_sub(start) / 10_000).clamp(0, duration_ms),
            stamp: if last_updated > 0 && last_updated <= now {
                last_updated
            } else {
                now
            },
        })
    }

    /// Position at `now`: advances in real time only while playing; clamped.
    pub fn position_at(&self, now: i64, playing: bool) -> i64 {
        let elapsed = if playing {
            now.saturating_sub(self.stamp).max(0) / 10_000
        } else {
            0
        };
        self.position_ms
            .saturating_add(elapsed)
            .clamp(0, self.duration_ms)
    }
}

/// Formats milliseconds as `m:ss` / `h:mm:ss` (optionally `-` prefixed) into a
/// fixed UTF-16 buffer (no allocation per frame); returns the length used.
pub fn format_clock(ms: i64, negative: bool, out: &mut [u16; 12]) -> usize {
    let total = ms.max(0) / 1000;
    let (h, m, sec) = ((total / 3600).min(99), (total / 60) % 60, total % 60);
    let mut n = 0;
    let mut push = |c: u8| {
        if n < out.len() {
            out[n] = u16::from(c);
            n += 1;
        }
    };
    if negative {
        push(b'-');
    }
    if h > 0 {
        if h >= 10 {
            push(b'0' + (h / 10) as u8);
        }
        push(b'0' + (h % 10) as u8);
        push(b':');
        push(b'0' + (m / 10) as u8);
    } else if m >= 10 {
        push(b'0' + (m / 10) as u8);
    }
    push(b'0' + (m % 10) as u8);
    push(b':');
    push(b'0' + (sec / 10) as u8);
    push(b'0' + (sec % 10) as u8);
    n
}

// ---- Visualizer --------------------------------------------------------------

pub const VISUALIZER_BARS: usize = 4;

/// Bars are shown only while the session is actually playing.
pub fn shows_visualizer(playback: PlaybackState) -> bool {
    playback == PlaybackState::Playing
}

/// Playback indicator: not an audio waveform but a deterministic periodic motion
/// (two incommensurate sines per bar) scaled by a fade `level` that eases in on
/// play and out on pause. Driven by `step` only while `is_active`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Visualizer {
    playing: bool,
    level: f32,
}

impl Visualizer {
    /// Returns true if the playing state changed.
    pub fn set_playing(&mut self, playing: bool) -> bool {
        std::mem::replace(&mut self.playing, playing) != playing
    }

    /// Needs frames: playing, or still fading out.
    pub fn is_active(&self) -> bool {
        self.playing || self.level > 0.0
    }

    /// Fade level 0..=1 (0 = hidden).
    pub fn level(&self) -> f32 {
        self.level
    }

    /// Jumps the fade to its target (used when no visualizer is on screen, so a
    /// later reveal never shows a stale fade).
    pub fn settle(&mut self) {
        self.level = if self.playing { 1.0 } else { 0.0 };
    }

    /// Advances the fade; returns `is_active()`.
    pub fn step(&mut self, dt_ms: f32) -> bool {
        let d = dt_ms / VISUALIZER_FADE_MS;
        self.level = if self.playing {
            (self.level + d).min(1.0)
        } else {
            (self.level - d).max(0.0)
        };
        self.is_active()
    }

    /// Bar heights (fractions of the full bar height) at `t` seconds. Fading
    /// bars settle to the minimum height as they disappear.
    pub fn bar_heights(&self, t: f64) -> [f32; VISUALIZER_BARS] {
        // (frequency Hz, phase) pairs per bar: smooth, never in lockstep
        const WAVES: [[(f64, f64); 2]; VISUALIZER_BARS] = [
            [(1.13, 0.0), (2.71, 1.9)],
            [(1.57, 2.3), (3.19, 0.4)],
            [(0.97, 4.1), (2.39, 2.8)],
            [(1.41, 1.2), (2.93, 5.0)],
        ];
        let eased = self.level * self.level * (3.0 - 2.0 * self.level);
        WAVES.map(|[(f1, p1), (f2, p2)]| {
            let tau = std::f64::consts::TAU;
            let wave = 0.58 + 0.26 * (tau * f1 * t + p1).sin() + 0.16 * (tau * f2 * t + p2).sin();
            let wave = (wave as f32).clamp(VISUALIZER_MIN_HEIGHT, 1.0);
            VISUALIZER_MIN_HEIGHT + (wave - VISUALIZER_MIN_HEIGHT) * eased
        })
    }
}

/// Playback status of the current session (mirrors the SMTC enum, plus `Unknown`
/// when the status could not be read or has an unrecognized value).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackState {
    #[default]
    Unknown,
    Closed,
    Opened,
    Changing,
    Stopped,
    Playing,
    Paused,
}

impl PlaybackState {
    fn from_status(status: Result<PlaybackStatus>) -> Self {
        match status {
            Ok(PlaybackStatus::Closed) => Self::Closed,
            Ok(PlaybackStatus::Opened) => Self::Opened,
            Ok(PlaybackStatus::Changing) => Self::Changing,
            Ok(PlaybackStatus::Stopped) => Self::Stopped,
            Ok(PlaybackStatus::Playing) => Self::Playing,
            Ok(PlaybackStatus::Paused) => Self::Paused,
            _ => Self::Unknown,
        }
    }
}

/// Owned track metadata. Missing fields are empty strings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
}

/// Raw metadata read on the completion thread; converted to `String` on the UI
/// thread only if it differs from what is already stored.
#[derive(Debug, Default)]
struct RawMetadata {
    title: HSTRING,
    artist: HSTRING,
    album: HSTRING,
}

/// Lightweight internal representation of the current Windows media session.
// One instance lives in the engine; boxing the session variant buys nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MediaSessionState {
    /// No current Windows media session available.
    #[default]
    NoSession,
    /// A current Windows media session exists and is accessible.
    SessionAvailable {
        /// Source application (AUMID + resolved name).
        source: SourceApp,
        metadata: MediaMetadata,
        playback: PlaybackState,
        /// Current track artwork; None when Windows provides none.
        artwork: Option<Artwork>,
        /// Accent derived from `artwork` (set together with it; None = neutral).
        accent: Option<Accent>,
        /// Track timeline; None when the session reports no duration.
        timeline: Option<Timeline>,
    },
}

impl MediaSessionState {
    /// Fresh state for a newly current session: metadata and artwork start empty
    /// and the source name starts `Pending`, so nothing from a previous session
    /// can carry over.
    fn for_session(source: Option<&HSTRING>, playback: PlaybackState) -> Self {
        match source {
            None => Self::NoSession,
            Some(id) => Self::SessionAvailable {
                source: SourceApp {
                    app_id: id.to_string_lossy(),
                    name: AppName::Pending,
                    icon: None,
                },
                metadata: MediaMetadata::default(),
                playback,
                artwork: None,
                accent: None,
                timeline: None,
            },
        }
    }

    /// Replaces artwork (dropping the old buffer) and its accent in the same
    /// step, so a new cover never shows with the previous accent; true if changed.
    fn apply_artwork(&mut self, new: Option<Artwork>) -> bool {
        match self {
            Self::SessionAvailable {
                artwork, accent, ..
            } if *artwork != new => {
                *accent = new.as_ref().and_then(artwork_accent);
                *artwork = new;
                true
            }
            _ => false,
        }
    }

    /// Replaces the timeline; true if it changed. No-op without a session.
    fn apply_timeline(&mut self, new: Option<Timeline>) -> bool {
        match self {
            Self::SessionAvailable { timeline, .. } if *timeline != new => {
                *timeline = new;
                true
            }
            _ => false,
        }
    }

    /// Applies a resolved source (name + badge icon) if it belongs to the current AUMID.
    fn apply_source(&mut self, app_id: &str, name: AppName, icon: Option<Artwork>) -> bool {
        match self {
            Self::SessionAvailable { source, .. }
                if source.app_id == app_id && (source.name != name || source.icon != icon) =>
            {
                source.name = name;
                source.icon = icon;
                true
            }
            _ => false,
        }
    }

    #[cfg(test)]
    fn apply_source_name(&mut self, app_id: &str, name: AppName) -> bool {
        self.apply_source(app_id, name, None)
    }

    fn source(&self) -> Option<&SourceApp> {
        match self {
            Self::SessionAvailable { source, .. } => Some(source),
            Self::NoSession => None,
        }
    }

    /// Applies metadata; returns true if anything changed. Allocates only for
    /// fields that differ. No-op without a session.
    fn apply_metadata(&mut self, raw: &RawMetadata) -> bool {
        let Self::SessionAvailable { metadata, .. } = self else {
            return false;
        };
        let mut changed = false;
        for (field, value) in [
            (&mut metadata.title, &raw.title),
            (&mut metadata.artist, &raw.artist),
            (&mut metadata.album, &raw.album),
        ] {
            if *value != *field {
                *field = value.to_string_lossy();
                changed = true;
            }
        }
        changed
    }

    /// Applies playback state; returns true if it changed. No-op without a session.
    fn apply_playback(&mut self, new: PlaybackState) -> bool {
        match self {
            Self::SessionAvailable { playback, .. } if *playback != new => {
                *playback = new;
                true
            }
            _ => false,
        }
    }
}

/// Transport controls shown in the expanded media composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaControl {
    Previous,
    PlayPause,
    Next,
}

impl MediaControl {
    pub const ALL: [MediaControl; 3] = [Self::Previous, Self::PlayPause, Self::Next];

    /// Accessible name describing the action the control performs now.
    pub fn accessible_name(self, icon: PlayPauseIcon) -> &'static str {
        match (self, icon) {
            (Self::Previous, _) => "Previous track",
            (Self::Next, _) => "Next track",
            (Self::PlayPause, PlayPauseIcon::Play) => "Play",
            (Self::PlayPause, PlayPauseIcon::Pause) => "Pause",
        }
    }
}

/// Control-local hover/press micro-interaction. Levels are 0..=1 per control:
/// hover fades in/out over `MEDIA_HOVER_FADE_MS`; press applies instantly on
/// mouse-down and eases back over `MEDIA_PRESS_RELEASE_MS` on release. Driven by
/// `step(dt)` only while `is_animating()`; idle costs nothing.
use crate::layout::PanelHit;

#[derive(Debug, Clone, PartialEq)]
pub struct ControlFeedback {
    hovered: Option<MediaControl>,
    pressed: Option<MediaControl>,
    hover: [f32; 3],
    press: [f32; 3],
    /// Released before the press finished compressing: keep going to full depth
    /// first, then spring back, so even a quick click shows the whole motion.
    tap: [bool; 3],
    /// New-track fade progress, 0..=1 (1 = settled). One timeline for the
    /// whole track content, not per field.
    track_fade: f32,
    /// Clipboard space icon buttons (row copy/trash, header X): the same
    /// hover/press easing, kept only for buttons that are targeted or still
    /// easing back (hover, press levels).
    clip_hovered: Option<PanelHit>,
    clip_pressed: Option<PanelHit>,
    /// Row under the pointer: its `Row(i)` hover level fades its buttons in.
    clip_row: Option<usize>,
    clip: Vec<(PanelHit, f32, f32)>,
}

impl Default for ControlFeedback {
    fn default() -> Self {
        Self {
            hovered: None,
            pressed: None,
            hover: [0.0; 3],
            press: [0.0; 3],
            tap: [false; 3],
            track_fade: 1.0,
            clip_hovered: None,
            clip_pressed: None,
            clip_row: None,
            clip: Vec::new(),
        }
    }
}

impl ControlFeedback {
    fn index(control: MediaControl) -> usize {
        match control {
            MediaControl::Previous => 0,
            MediaControl::PlayPause => 1,
            MediaControl::Next => 2,
        }
    }

    /// Sets the hovered control; true if the hover target changed.
    pub fn set_hovered(&mut self, control: Option<MediaControl>) -> bool {
        std::mem::replace(&mut self.hovered, control) != control
    }

    /// Mouse-down on a control: instant compression.
    pub fn press(&mut self, control: MediaControl) {
        self.pressed = Some(control);
    }

    /// Mouse-up / capture loss: returns the control that was pressed; its press
    /// level finishes compressing if needed, then eases back to 0 via `step`.
    pub fn release(&mut self) -> Option<MediaControl> {
        let released = self.pressed.take();
        if let Some(control) = released {
            let i = Self::index(control);
            if self.press[i] < 1.0 {
                self.tap[i] = true;
            }
        }
        released
    }

    /// (hover, press) levels for rendering.
    pub fn levels(&self, control: MediaControl) -> (f32, f32) {
        let i = Self::index(control);
        (self.hover[i], self.press[i])
    }

    fn targets(&self, i: usize) -> (f32, f32) {
        let is = |c: Option<MediaControl>| c.is_some_and(|c| Self::index(c) == i);
        (
            if is(self.hovered) { 1.0 } else { 0.0 },
            if is(self.pressed) || self.tap[i] {
                1.0
            } else {
                0.0
            },
        )
    }

    pub fn is_animating(&self) -> bool {
        self.track_fade < 1.0
            || (0..3).any(|i| self.targets(i) != (self.hover[i], self.press[i]))
            || self
                .clip
                .iter()
                .any(|&(b, h, p)| self.clip_targets(b) != (h, p))
            || self
                .clip_targeted()
                .into_iter()
                .flatten()
                .any(|b| !self.clip.iter().any(|e| e.0 == b))
    }

    /// Hovered clipboard icon button; true if it changed.
    pub fn set_clip_hovered(&mut self, button: Option<PanelHit>) -> bool {
        std::mem::replace(&mut self.clip_hovered, button) != button
    }

    /// Row under the pointer (its copy / trash buttons fade in); true if it
    /// changed.
    pub fn set_clip_row(&mut self, row: Option<usize>) -> bool {
        std::mem::replace(&mut self.clip_row, row) != row
    }

    /// Everything currently targeted: hovered button, pressed button, hovered row.
    fn clip_targeted(&self) -> [Option<PanelHit>; 3] {
        [
            self.clip_hovered,
            self.clip_pressed,
            self.clip_row.map(PanelHit::Row),
        ]
    }

    /// Mouse-down on a clipboard icon button.
    pub fn press_clip(&mut self, button: PanelHit) {
        self.clip_pressed = Some(button);
    }

    /// Mouse-up: the button eases back (returns whether one was pressed).
    pub fn release_clip(&mut self) -> bool {
        self.clip_pressed.take().is_some()
    }

    /// (hover, press) levels of a clipboard icon button.
    pub fn clip_levels(&self, button: PanelHit) -> (f32, f32) {
        self.clip
            .iter()
            .find(|e| e.0 == button)
            .map_or((0.0, 0.0), |e| (e.1, e.2))
    }

    fn clip_targets(&self, button: PanelHit) -> (f32, f32) {
        (
            f32::from(u8::from(
                self.clip_hovered == Some(button)
                    || self.clip_row.map(PanelHit::Row) == Some(button),
            )),
            f32::from(u8::from(self.clip_pressed == Some(button))),
        )
    }

    /// Restarts the new-track fade (content was already swapped atomically).
    pub fn start_track_fade(&mut self) {
        self.track_fade = 0.0;
    }

    /// Settles the track fade immediately (content not visible).
    pub fn finish_track_fade(&mut self) {
        self.track_fade = 1.0;
    }

    /// Opacity multiplier for track content: floor -> 1 with an ease-out curve.
    pub fn track_alpha(&self) -> f32 {
        let t = self.track_fade.clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        MEDIA_TRACK_FADE_FLOOR + (1.0 - MEDIA_TRACK_FADE_FLOOR) * eased
    }

    /// Advances levels toward their targets; returns true while still animating.
    pub fn step(&mut self, dt_ms: f32) -> bool {
        let approach = |v: f32, target: f32, duration: f32| {
            let d = dt_ms / duration;
            if v < target {
                (v + d).min(target)
            } else {
                (v - d).max(target)
            }
        };
        for i in 0..3 {
            let (hover_t, press_t) = self.targets(i);
            self.hover[i] = approach(self.hover[i], hover_t, MEDIA_HOVER_FADE_MS);
            let press_ms = if press_t > self.press[i] {
                MEDIA_PRESS_IN_MS
            } else {
                MEDIA_PRESS_RELEASE_MS
            };
            self.press[i] = approach(self.press[i], press_t, press_ms);
            if self.tap[i] && self.press[i] >= 1.0 {
                self.tap[i] = false;
            }
        }
        self.track_fade = approach(self.track_fade, 1.0, MEDIA_TRACK_FADE_MS);
        let targeted = self.clip_targeted();
        for b in targeted.into_iter().flatten() {
            if !self.clip.iter().any(|e| e.0 == b) {
                self.clip.push((b, 0.0, 0.0));
            }
        }
        let targets: Vec<_> = self.clip.iter().map(|e| self.clip_targets(e.0)).collect();
        for (e, (hover_t, press_t)) in self.clip.iter_mut().zip(targets) {
            e.1 = approach(e.1, hover_t, MEDIA_HOVER_FADE_MS);
            let press_ms = if press_t > e.2 {
                MEDIA_PRESS_IN_MS
            } else {
                MEDIA_PRESS_RELEASE_MS
            };
            e.2 = approach(e.2, press_t, press_ms);
        }
        self.clip
            .retain(|e| e.1 > 0.0 || e.2 > 0.0 || targeted.contains(&Some(e.0)));
        self.is_animating()
    }

    /// Clears everything immediately (media hidden, notch toggling).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Reduced motion: hover levels (control brightening, a clipboard tile's
    /// buttons) jump to their targets instead of fading. Presses still ease.
    pub fn snap_hover(&mut self) {
        for i in 0..3 {
            self.hover[i] = self.targets(i).0;
        }
        let targeted = self.clip_targeted();
        for b in targeted.into_iter().flatten() {
            if !self.clip.iter().any(|e| e.0 == b) {
                self.clip.push((b, 0.0, 0.0));
            }
        }
        let targets: Vec<_> = self.clip.iter().map(|e| self.clip_targets(e.0)).collect();
        for (e, (hover, _)) in self.clip.iter_mut().zip(targets) {
            e.1 = hover;
        }
        self.clip
            .retain(|e| e.1 > 0.0 || e.2 > 0.0 || targeted.contains(&Some(e.0)));
    }
}

/// How long a play/pause click's expected icon waits for the session to
/// confirm it before the glyph returns to the reported state (a command the
/// app ignored never leaves a wrong glyph behind).
pub const PLAY_PAUSE_CONFIRM_MS: u64 = 3000;

/// Transport glyph motion, independent of the commands themselves:
/// - Play/pause morphs continuously between the two glyphs (0 = play,
///   1 = pause). A click heads for the other glyph at once (the command is
///   sent at the same moment); the session's reported state then confirms
///   it, or corrects it after `PLAY_PAUSE_CONFIRM_MS`. Every change retargets
///   from where the glyph is, with its velocity.
/// - Previous / Next slide their glyph one step per click in their direction
///   (`skip` counts steps); repeated clicks extend the same motion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconMotion {
    morph: crate::config::Glide,
    /// What the session reports.
    actual: PlayPauseIcon,
    /// A click's expected glyph, until confirmed or the wait ends.
    expected: Option<(PlayPauseIcon, std::time::Instant)>,
    skip: [crate::config::Glide; 2],
}

impl Default for IconMotion {
    fn default() -> Self {
        Self {
            morph: crate::config::Glide::at(0.0),
            actual: PlayPauseIcon::Play,
            expected: None,
            skip: [crate::config::Glide::at(0.0); 2],
        }
    }
}

fn morph_of(icon: PlayPauseIcon) -> f32 {
    match icon {
        PlayPauseIcon::Play => 0.0,
        PlayPauseIcon::Pause => 1.0,
    }
}

impl IconMotion {
    /// The session's reported glyph. A pending click that it confirms is
    /// settled; one it does not yet confirm keeps its target until the wait
    /// ends. `instant`: jump (Reduced motion, nothing on screen).
    pub fn sync(&mut self, actual: PlayPauseIcon, now: std::time::Instant, instant: bool) {
        self.actual = actual;
        match self.expected {
            Some((icon, _)) if icon == actual => self.expected = None,
            Some((_, until)) if now < until => {}
            _ => self.expected = None,
        }
        let target = morph_of(self.expected.map_or(actual, |(icon, _)| icon));
        if instant {
            self.morph.snap(target);
        } else {
            self.morph.target = target;
        }
    }

    /// Play/pause clicked (its command already sent): head for the other
    /// glyph from wherever the morph is.
    pub fn clicked_play_pause(&mut self, now: std::time::Instant, instant: bool) {
        let shown = self.expected.map_or(self.actual, |(icon, _)| icon);
        let next = match shown {
            PlayPauseIcon::Play => PlayPauseIcon::Pause,
            PlayPauseIcon::Pause => PlayPauseIcon::Play,
        };
        self.expected = Some((
            next,
            now + std::time::Duration::from_millis(PLAY_PAUSE_CONFIRM_MS),
        ));
        if instant {
            self.morph.snap(morph_of(next));
        } else {
            self.morph.target = morph_of(next);
        }
    }

    /// Previous / Next clicked (command sent): one more step of slide. At
    /// most a few steps queue up, so a burst never spins on.
    pub fn clicked_skip(&mut self, control: MediaControl, instant: bool) {
        let Some(g) = self.skip_mut(control) else {
            return;
        };
        if instant {
            g.snap(0.0);
            return;
        }
        g.target = (g.target + 1.0).min(g.value.floor() + 3.0);
    }

    fn skip_mut(&mut self, control: MediaControl) -> Option<&mut crate::config::Glide> {
        match control {
            MediaControl::Previous => Some(&mut self.skip[0]),
            MediaControl::Next => Some(&mut self.skip[1]),
            MediaControl::PlayPause => None,
        }
    }

    /// 0 = play glyph, 1 = pause glyph.
    pub fn morph(&self) -> f32 {
        self.morph.value
    }

    /// Where a skip glyph is in its current step, 0..1 (0 at rest).
    pub fn skip_phase(&self, control: MediaControl) -> f32 {
        let g = match control {
            MediaControl::Previous => self.skip[0],
            MediaControl::Next => self.skip[1],
            MediaControl::PlayPause => return 0.0,
        };
        g.value - g.value.floor()
    }

    /// Needs frames: a glyph moving, or a click awaiting confirmation.
    pub fn is_animating(&self) -> bool {
        self.morph.is_animating()
            || self.expected.is_some()
            || self.skip.iter().any(|g| g.is_animating())
    }

    /// Advances by `dt_ms`; an unconfirmed click's wait ends here. True
    /// while anything still needs frames.
    pub fn step(&mut self, dt_ms: f32, now: std::time::Instant) -> bool {
        if self.expected.is_some_and(|(_, until)| now >= until) {
            self.expected = None;
            self.morph.target = morph_of(self.actual);
        }
        self.morph.step(crate::config::ICON_SPRING, dt_ms);
        // Skips roll on the calmer space spring (smoother than the
        // play/pause snap)
        for g in &mut self.skip {
            if g.step(crate::config::SPACE_SPRING, dt_ms) {
                continue;
            }
            // Settled on a whole step: back to 0 (same picture)
            g.snap(0.0);
        }
        self.is_animating()
    }

    /// Everything at rest on the reported glyph (nothing on screen).
    pub fn settle(&mut self) {
        let actual = self.actual;
        *self = Self::default();
        self.actual = actual;
        self.morph.snap(morph_of(actual));
    }
}

/// Icon on the central control: the action a click performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayPauseIcon {
    Play,
    Pause,
}

impl PlaybackState {
    /// Playing shows Pause; everything else (including Unknown) safely shows Play.
    pub fn play_pause_icon(self) -> PlayPauseIcon {
        match self {
            Self::Playing => PlayPauseIcon::Pause,
            _ => PlayPauseIcon::Play,
        }
    }
}

/// Fallback primary line when a session reports no title.
pub const MEDIA_TITLE_FALLBACK: &str = "Now playing";

/// Owned display model for the expanded media composition. Contains no WinRT
/// objects; renderer/UI code never needs to know about media sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaContent {
    /// Display title (falls back to `MEDIA_TITLE_FALLBACK`).
    pub title: String,
    /// Artist, or empty (line omitted). Album is not shown for now.
    pub subtitle: String,
    pub icon: PlayPauseIcon,
    /// Raw fields for later UI phases; None when Windows provides nothing.
    #[allow(dead_code)]
    pub artist: Option<String>,
    #[allow(dead_code)]
    pub album: Option<String>,
    #[allow(dead_code)]
    pub playback: PlaybackState,
    #[allow(dead_code)]
    pub artwork: Option<Artwork>,
    #[allow(dead_code)]
    pub source: SourceApp,
    /// Artwork-derived accent (None = neutral UI accent).
    pub accent: Option<Accent>,
    pub timeline: Option<Timeline>,
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

impl MediaContent {
    /// Returns the content to display, or `None` when there is no session.
    pub fn from_state(state: &MediaSessionState) -> Option<Self> {
        let MediaSessionState::SessionAvailable {
            source,
            metadata,
            playback,
            artwork,
            accent,
            timeline,
        } = state
        else {
            return None;
        };
        let title = metadata.title.trim();
        let artist = metadata.artist.trim();
        let album = metadata.album.trim();
        // Album is not displayed for now (kept in `album` for a later layout)
        let subtitle = artist.to_string();
        Some(Self {
            title: if title.is_empty() {
                MEDIA_TITLE_FALLBACK.to_string()
            } else {
                title.to_string()
            },
            subtitle,
            icon: playback.play_pause_icon(),
            artist: non_empty(artist),
            album: non_empty(album),
            playback: *playback,
            artwork: artwork.clone(),
            source: source.clone(),
            accent: *accent,
            timeline: *timeline,
        })
    }

    #[cfg(test)]
    pub fn test_default() -> Self {
        Self {
            title: String::new(),
            subtitle: String::new(),
            icon: PlayPauseIcon::Play,
            artist: None,
            album: None,
            playback: PlaybackState::Unknown,
            artwork: None,
            source: SourceApp {
                app_id: String::new(),
                name: AppName::Unavailable,
                icon: None,
            },
            accent: None,
            timeline: None,
        }
    }

    /// What the layout has to make room for (artwork slot, secondary lines).
    pub fn shape(&self) -> crate::layout::MediaShape {
        crate::layout::MediaShape {
            artwork: self.artwork.is_some(),
            secondary_lines: u32::from(!self.subtitle.is_empty())
                + u32::from(self.shows_source_text()),
        }
    }

    /// The badge (drawn on the artwork) replaces the source name; the text line
    /// shows only when there is no badge on screen.
    pub fn shows_badge(&self) -> bool {
        self.source.icon.is_some() && self.artwork.is_some()
    }

    /// Source-name text line: other apps, or a badge app without artwork.
    pub fn shows_source_text(&self) -> bool {
        self.source_name().is_some() && !self.shows_badge()
    }

    /// Resolved source application name, if Windows provided one.
    pub fn source_name(&self) -> Option<&str> {
        match &self.source.name {
            AppName::Available(name) => Some(name),
            AppName::Pending | AppName::Unavailable => None,
        }
    }

    /// Accessible description of the displayed media, e.g.
    /// "Song - Artist - Spotify" (empty parts omitted).
    pub fn accessible_text(&self) -> String {
        [
            Some(self.title.as_str()),
            Some(self.subtitle.as_str()),
            self.source_name(),
        ]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" - ")
    }
}

/// Media session engine providing discovery, metadata and playback tracking.
pub struct MediaEngine {
    state: MediaSessionState,
    /// Current session; its event tokens are only valid against this object.
    session: Option<Session>,
    properties_token: Option<i64>,
    playback_token: Option<i64>,
    timeline_token: Option<i64>,
    /// Incremented whenever `session` is replaced; tags source-name lookups.
    generation: u64,
    /// Incremented on every metadata request and session replacement; tags
    /// metadata and artwork results so only the latest request is applied.
    metadata_seq: u64,
    metadata_tx: Sender<(u64, RawMetadata)>,
    metadata_rx: Receiver<(u64, RawMetadata)>,
    artwork_tx: Sender<(u64, Option<Artwork>)>,
    artwork_rx: Receiver<(u64, Option<Artwork>)>,
    source_tx: Sender<(u64, String, AppName, Option<Artwork>)>,
    source_rx: Receiver<(u64, String, AppName, Option<Artwork>)>,
    /// Last resolved source (single entry) so re-selecting the same app is free.
    source_cache: Option<SourceApp>,
    /// Text of the latest request awaiting its artwork result.
    pending_metadata: Option<(u64, RawMetadata)>,
    /// Sequence whose artwork result has been applied.
    artwork_ready_seq: Option<u64>,
    manager: Option<SessionManager>,
    session_changed_token: Option<i64>,
    pending_manager: Option<Receiver<Result<SessionManager>>>,
}

impl Default for MediaEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn post(hwnd_raw: usize, msg: u32) {
    unsafe {
        let _ = PostMessageW(Some(HWND(hwnd_raw as _)), msg, WPARAM(0), LPARAM(0));
    }
}

/// Delivers exactly one artwork result for a metadata request. If any stage of
/// the async decode chain fails or is dropped, `Drop` delivers `None`, so a
/// request can never leave the previous track's artwork in place by omission.
struct ArtworkDelivery {
    tx: Sender<(u64, Option<Artwork>)>,
    seq: u64,
    hwnd_raw: usize,
    sent: bool,
}

impl ArtworkDelivery {
    fn new(tx: Sender<(u64, Option<Artwork>)>, seq: u64, hwnd_raw: usize) -> Self {
        Self {
            tx,
            seq,
            hwnd_raw,
            sent: false,
        }
    }

    fn send(mut self, artwork: Option<Artwork>) {
        self.deliver(artwork);
    }

    fn deliver(&mut self, artwork: Option<Artwork>) {
        if !self.sent {
            self.sent = true;
            if self.tx.send((self.seq, artwork)).is_ok() {
                post(self.hwnd_raw, WM_APP_MEDIA_ARTWORK_READY);
            }
        }
    }
}

impl Drop for ArtworkDelivery {
    fn drop(&mut self) {
        self.deliver(None);
    }
}

/// Decodes a session thumbnail entirely through WinRT async operations
/// (OpenReadAsync -> BitmapDecoder::CreateAsync -> GetPixelDataTransformedAsync);
/// every continuation runs on a WinRT completion thread, never the UI thread.
/// Produces owned BGRA8 premultiplied pixels scaled to `ARTWORK_MAX_EDGE`.
/// All WinRT objects (stream, decoder, transform, provider) are dropped here.
fn decode_artwork(thumb: IRandomAccessStreamReference, delivery: ArtworkDelivery) {
    let _ = thumb.OpenReadAsync().and_then(move |op| {
        op.when(move |stream| {
            let Ok(stream) = stream else { return };
            let _ = BitmapDecoder::CreateAsync(&stream).and_then(move |op| {
                op.when(move |decoder| {
                    let Ok(decoder) = decoder else { return };
                    let request = (|| {
                        let (w, h) = fit_within(
                            decoder.PixelWidth()?,
                            decoder.PixelHeight()?,
                            ARTWORK_MAX_EDGE,
                        );
                        let transform = BitmapTransform::new()?;
                        transform.SetScaledWidth(w)?;
                        transform.SetScaledHeight(h)?;
                        transform.SetInterpolationMode(BitmapInterpolationMode::Fant)?;
                        let op = decoder.GetPixelDataTransformedAsync(
                            BitmapPixelFormat::Bgra8,
                            BitmapAlphaMode::Premultiplied,
                            &transform,
                            ExifOrientationMode::IgnoreExifOrientation,
                            ColorManagementMode::ColorManageToSRgb,
                        )?;
                        Ok::<_, windows::core::Error>((op, w, h))
                    })();
                    if let Ok((op, w, h)) = request {
                        let _ = op.when(move |provider| {
                            let artwork = provider
                                .and_then(|p| p.DetachPixelData())
                                .ok()
                                .and_then(|px| Artwork::new(w, h, px.to_vec()));
                            delivery.send(artwork);
                        });
                    }
                })
            });
        })
    });
}

/// True for AUMIDs that are a bare executable name (e.g. "SomePlayer.exe"), which
/// Windows uses for desktop apps without an explicit AppUserModelID.
fn is_executable_name(app_id: &str) -> bool {
    app_id.len() > 4
        && app_id[app_id.len() - 4..].eq_ignore_ascii_case(".exe")
        && !app_id.contains(['\\', '/', '!'])
}

/// Resolves an AUMID to the display name Windows shows for it, using only the
/// Shell "Applications" folder (FOLDERID_AppsFolder):
/// 1. direct parse of the AUMID (packaged apps and AUMID-registered shortcuts);
/// 2. for bare executable names, the Apps-folder entry whose parsing path ends
///    in that executable (desktop apps without an AUMID).
///
/// Generic: no application is special-cased. Runs on the Windows thread pool.
#[cfg(test)]
fn resolve_app_name(app_id: &str) -> AppName {
    resolve_app(app_id).0
}

/// Name plus, for `BADGE_APPS` only, the app's own icon from the same shell item
/// (`IShellItemImageFactory`, read-only, local). Other apps skip icon work.
fn resolve_app(app_id: &str) -> (AppName, Option<Artwork>) {
    if app_id.is_empty() {
        return (AppName::Unavailable, None);
    }
    let direct = unsafe {
        SHCreateItemInKnownFolder::<_, IShellItem>(
            &FOLDERID_AppsFolder,
            KF_FLAG_DEFAULT,
            &HSTRING::from(app_id),
        )
    }
    .ok()
    .and_then(|item| Some((shell_display_name(&item, SIGDN_NORMALDISPLAY)?, item)));
    let found = direct.or_else(|| {
        is_executable_name(app_id)
            .then(|| find_app_by_executable(app_id))
            .flatten()
    });
    match found {
        Some((name, item)) => {
            let icon = wants_badge(&name)
                .then(|| shell_icon(&item, BADGE_ICON_PX))
                .flatten();
            (AppName::Available(name), icon)
        }
        None => (AppName::Unavailable, None),
    }
}

/// The app's icon as BGRA pixels via the shell image factory (icon only, no
/// thumbnail extraction). The HBITMAP is always deleted.
fn shell_icon(item: &IShellItem, size: u32) -> Option<Artwork> {
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC,
        GetDIBits, GetObjectW, HGDIOBJ, ReleaseDC,
    };
    use windows::Win32::UI::Shell::{IShellItemImageFactory, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY};
    unsafe {
        let factory: IShellItemImageFactory = windows::core::Interface::cast(item).ok()?;
        let side = size as i32;
        let hbm = factory
            .GetImage(
                SIZE { cx: side, cy: side },
                SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
            )
            .ok()?;
        let mut bm = BITMAP::default();
        let ok = GetObjectW(
            HGDIOBJ(hbm.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        ) != 0;
        let (w, h) = (bm.bmWidth.max(0) as u32, bm.bmHeight.unsigned_abs());
        let mut px = vec![0u8; (w as usize) * (h as usize) * 4];
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let dc = GetDC(None);
        let lines = if ok && w > 0 && h > 0 {
            GetDIBits(
                dc,
                hbm,
                0,
                h,
                Some(px.as_mut_ptr() as *mut _),
                &mut bmi,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };
        ReleaseDC(None, dc);
        let _ = DeleteObject(HGDIOBJ(hbm.0));
        if lines != h as i32 {
            return None;
        }
        badge_pixels(px, w, h)
    }
}

fn shell_display_name(item: &IShellItem, sigdn: SIGDN) -> Option<String> {
    unsafe {
        let p = item.GetDisplayName(sigdn).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.filter(|s| !s.is_empty())
    }
}

// ponytail: linear scan of the Apps folder (~0.4 s measured, off the UI thread),
// once per new source app thanks to `source_cache`; index it if it ever shows up.
fn find_app_by_executable(exe: &str) -> Option<(String, IShellItem)> {
    let suffix = format!("\\{}", exe.to_lowercase());
    unsafe {
        let folder: IShellItem =
            SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None).ok()?;
        let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems).ok()?;
        loop {
            let mut batch = [None];
            let mut fetched = 0u32;
            if items.Next(&mut batch, Some(&mut fetched)).is_err() || fetched == 0 {
                return None;
            }
            let item = batch[0].take()?;
            if shell_display_name(&item, SIGDN_PARENTRELATIVEPARSING)
                .is_some_and(|p| p.to_lowercase().ends_with(&suffix))
            {
                return Some((shell_display_name(&item, SIGDN_NORMALDISPLAY)?, item));
            }
        }
    }
}

impl MediaEngine {
    /// Creates an engine with no manager; call `initialize` to start discovery.
    pub fn new() -> Self {
        let (metadata_tx, metadata_rx) = channel();
        let (artwork_tx, artwork_rx) = channel();
        let (source_tx, source_rx) = channel();
        Self {
            state: MediaSessionState::NoSession,
            session: None,
            properties_token: None,
            playback_token: None,
            timeline_token: None,
            generation: 0,
            metadata_seq: 0,
            metadata_tx,
            metadata_rx,
            artwork_tx,
            artwork_rx,
            source_tx,
            source_rx,
            source_cache: None,
            pending_metadata: None,
            artwork_ready_seq: None,
            manager: None,
            session_changed_token: None,
            pending_manager: None,
        }
    }

    /// Returns the current media session state.
    #[inline]
    #[allow(dead_code)]
    pub fn state(&self) -> &MediaSessionState {
        &self.state
    }

    /// Starts `RequestAsync()` without blocking. Completion is delivered to `hwnd`
    /// as `WM_APP_MEDIA_MANAGER_READY`.
    /// Returns false if the request could not be started (media stays `NoSession`).
    pub fn initialize(&mut self, hwnd: HWND) -> bool {
        let (tx, rx) = channel();
        let hwnd_raw = hwnd.0 as usize;
        let started = SessionManager::RequestAsync().and_then(|op| {
            op.when(move |result| {
                // Receiver gone means Oeca is shutting down; nothing to do.
                if tx.send(result).is_ok() {
                    post(hwnd_raw, WM_APP_MEDIA_MANAGER_READY);
                }
            })
        });
        match started {
            Ok(()) => {
                self.pending_manager = Some(rx);
                true
            }
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("[media] RequestAsync failed to start: {_e}");
                false
            }
        }
    }

    /// UI thread: dispatches a private media message. Returns true if the media
    /// state changed.
    pub fn handle_message(&mut self, hwnd: HWND, msg: u32) -> bool {
        let changed = match msg {
            WM_APP_MEDIA_MANAGER_READY => self.on_manager_ready(hwnd),
            WM_APP_MEDIA_SESSION_CHANGED => self.refresh_current_session(hwnd),
            WM_APP_MEDIA_PROPERTIES_CHANGED => {
                self.request_metadata(hwnd);
                false
            }
            WM_APP_MEDIA_PROPERTIES_READY => self.on_metadata_ready(),
            WM_APP_MEDIA_PLAYBACK_CHANGED => self.refresh_playback(),
            WM_APP_MEDIA_ARTWORK_READY => self.on_artwork_ready(),
            WM_APP_MEDIA_SOURCE_READY => self.on_source_ready(),
            WM_APP_MEDIA_TIMELINE_CHANGED => self.refresh_timeline(),
            _ => false,
        };
        #[cfg(debug_assertions)]
        if changed {
            eprintln!("[media] {:?}", self.state);
        }
        changed
    }

    /// Takes the manager delivered by `RequestAsync`, subscribes to
    /// `CurrentSessionChanged` and queries the current session.
    fn on_manager_ready(&mut self, hwnd: HWND) -> bool {
        let Some(result) = self
            .pending_manager
            .take()
            .and_then(|rx| rx.try_recv().ok())
        else {
            return false;
        };
        let manager = match result {
            Ok(m) => m,
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("[media] RequestAsync failed: {_e}");
                return false;
            }
        };

        let hwnd_raw = hwnd.0 as usize;
        let handler = TypedEventHandler::new(move |_, _| {
            post(hwnd_raw, WM_APP_MEDIA_SESSION_CHANGED);
            Ok(())
        });
        match manager.CurrentSessionChanged(&handler) {
            Ok(token) => self.session_changed_token = Some(token),
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("[media] CurrentSessionChanged subscribe failed: {_e}");
            }
        }
        #[cfg(debug_assertions)]
        eprintln!(
            "[media] manager ready, CurrentSessionChanged token: {:?}",
            self.session_changed_token
        );
        self.manager = Some(manager);
        self.refresh_current_session(hwnd)
    }

    /// Queries `GetCurrentSession()`. If the session object changed, moves the
    /// per-session subscriptions to it and resets its metadata/playback.
    fn refresh_current_session(&mut self, hwnd: HWND) -> bool {
        let Some(manager) = &self.manager else {
            return false;
        };
        // A null current session surfaces as Err; both mean "no usable session".
        let session = manager.GetCurrentSession().ok();
        if session == self.session {
            return false;
        }

        self.unsubscribe_session();
        self.session = session;
        self.generation = self.generation.wrapping_add(1);
        // Invalidate in-flight metadata/artwork of the previous session
        self.metadata_seq = self.metadata_seq.wrapping_add(1);

        let mut new_state = match self.session.clone() {
            None => MediaSessionState::NoSession,
            Some(s) => {
                self.subscribe_session(&s, hwnd);
                let source = s.SourceAppUserModelId().unwrap_or_default();
                let playback = PlaybackState::from_status(
                    s.GetPlaybackInfo().and_then(|info| info.PlaybackStatus()),
                );
                let mut fresh = MediaSessionState::for_session(Some(&source), playback);
                fresh.apply_timeline(read_timeline(&s));
                fresh
            }
        };
        if let Some(app_id) = new_state.source().map(|s| s.app_id.clone()) {
            match self.source_cache.as_ref().filter(|c| c.app_id == app_id) {
                Some(cached) => {
                    new_state.apply_source(&app_id, cached.name.clone(), cached.icon.clone());
                }
                None => self.request_source_name(hwnd, app_id),
            }
        }
        if self.session.is_some() {
            self.request_metadata(hwnd);
        }
        let changed = new_state != self.state;
        self.state = new_state;
        changed
    }

    /// Resolves the AUMID's display name on the Windows thread pool (shell lookups
    /// can block). The result arrives as `WM_APP_MEDIA_SOURCE_READY`, tagged with
    /// the session generation.
    fn request_source_name(&self, hwnd: HWND, app_id: String) {
        let tx = self.source_tx.clone();
        let generation = self.generation;
        let hwnd_raw = hwnd.0 as usize;
        let lookup_id = app_id.clone();
        let started = ThreadPool::RunAsync(&WorkItemHandler::new(move |_| {
            let (name, icon) = resolve_app(&lookup_id);
            if tx.send((generation, lookup_id.clone(), name, icon)).is_ok() {
                post(hwnd_raw, WM_APP_MEDIA_SOURCE_READY);
            }
            Ok(())
        }));
        if started.is_err() {
            // Could not schedule: report unavailable through the normal path
            if self
                .source_tx
                .send((generation, app_id, AppName::Unavailable, None))
                .is_ok()
            {
                post(hwnd_raw, WM_APP_MEDIA_SOURCE_READY);
            }
        }
    }

    /// Applies source-name results for the current session generation.
    fn on_source_ready(&mut self) -> bool {
        let mut changed = false;
        while let Ok((generation, app_id, name, icon)) = self.source_rx.try_recv() {
            if generation != self.generation {
                continue;
            }
            self.source_cache = Some(SourceApp {
                app_id: app_id.clone(),
                name: name.clone(),
                icon: icon.clone(),
            });
            changed |= self.state.apply_source(&app_id, name, icon);
        }
        changed
    }

    /// Applies artwork results for the latest metadata request only, together
    /// with that request's buffered text so title and artwork change in one step.
    fn on_artwork_ready(&mut self) -> bool {
        let mut changed = false;
        while let Ok((seq, artwork)) = self.artwork_rx.try_recv() {
            if seq != self.metadata_seq {
                continue;
            }
            self.artwork_ready_seq = Some(seq);
            if let Some((text_seq, raw)) = self.pending_metadata.take()
                && text_seq == seq
            {
                changed |= self.state.apply_metadata(&raw);
            }
            changed |= self.state.apply_artwork(artwork);
        }
        // A new track: take its timeline in the same step as its text and artwork
        if changed {
            self.refresh_timeline();
        }
        changed
    }

    fn subscribe_session(&mut self, session: &Session, hwnd: HWND) {
        let hwnd_raw = hwnd.0 as usize;
        self.properties_token = session
            .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                post(hwnd_raw, WM_APP_MEDIA_PROPERTIES_CHANGED);
                Ok(())
            }))
            .ok();
        self.playback_token = session
            .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                post(hwnd_raw, WM_APP_MEDIA_PLAYBACK_CHANGED);
                Ok(())
            }))
            .ok();
        self.timeline_token = session
            .TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                post(hwnd_raw, WM_APP_MEDIA_TIMELINE_CHANGED);
                Ok(())
            }))
            .ok();
    }

    fn unsubscribe_session(&mut self) {
        if let Some(session) = &self.session {
            if let Some(token) = self.properties_token.take() {
                let _ = session.RemoveMediaPropertiesChanged(token);
            }
            if let Some(token) = self.playback_token.take() {
                let _ = session.RemovePlaybackInfoChanged(token);
            }
            if let Some(token) = self.timeline_token.take() {
                let _ = session.RemoveTimelinePropertiesChanged(token);
            }
        }
    }

    /// Starts `TryGetMediaPropertiesAsync()` for the current session. Text arrives
    /// as `WM_APP_MEDIA_PROPERTIES_READY`; the thumbnail is then decoded off the UI
    /// thread and arrives as `WM_APP_MEDIA_ARTWORK_READY` (None if unavailable).
    /// Both are tagged with this request's sequence number; a newer request or a
    /// session switch makes them stale. A failed request yields empty metadata.
    fn request_metadata(&mut self, hwnd: HWND) {
        let Some(session) = &self.session else {
            return;
        };
        self.metadata_seq = self.metadata_seq.wrapping_add(1);
        let seq = self.metadata_seq;
        let tx = self.metadata_tx.clone();
        let hwnd_raw = hwnd.0 as usize;
        let artwork = ArtworkDelivery::new(self.artwork_tx.clone(), seq, hwnd_raw);
        let started = session.TryGetMediaPropertiesAsync().and_then(move |op| {
            op.when(move |result| {
                let props = result.ok();
                let raw = props
                    .as_ref()
                    .map(|p| RawMetadata {
                        title: p.Title().unwrap_or_default(),
                        artist: p.Artist().unwrap_or_default(),
                        album: p.AlbumTitle().unwrap_or_default(),
                    })
                    .unwrap_or_default();
                if tx.send((seq, raw)).is_ok() {
                    post(hwnd_raw, WM_APP_MEDIA_PROPERTIES_READY);
                }
                // A null/missing thumbnail drops `artwork`, which delivers None.
                if let Some(thumb) = props.and_then(|p| p.Thumbnail().ok()) {
                    decode_artwork(thumb, artwork);
                }
            })
        });
        if started.is_err() && self.metadata_tx.send((seq, RawMetadata::default())).is_ok() {
            post(hwnd_raw, WM_APP_MEDIA_PROPERTIES_READY);
        }
    }

    /// Text for the latest request is held until that request's artwork result
    /// (always delivered, `None` included) arrives, so a new title never appears
    /// next to the previous track's artwork. If the artwork result already
    /// arrived, the text applies immediately. Stale results are dropped.
    fn on_metadata_ready(&mut self) -> bool {
        let mut changed = false;
        while let Ok((seq, raw)) = self.metadata_rx.try_recv() {
            if seq != self.metadata_seq {
                continue;
            }
            if self.artwork_ready_seq == Some(seq) {
                changed |= self.state.apply_metadata(&raw);
            } else {
                self.pending_metadata = Some((seq, raw));
            }
        }
        changed
    }

    /// UI thread: sends a transport command to the session that is current *now*
    /// (queried from the manager, not the cached session). Fire-and-forget: the
    /// async result is never awaited; the resulting state arrives through the
    /// normal playback/metadata events. Returns false if nothing was sent.
    pub fn send_command(&self, control: MediaControl) -> bool {
        let Some(session) = self
            .manager
            .as_ref()
            .and_then(|m| m.GetCurrentSession().ok())
        else {
            return false;
        };
        let op = match control {
            MediaControl::Previous => session.TrySkipPreviousAsync(),
            MediaControl::PlayPause => session.TryTogglePlayPauseAsync(),
            MediaControl::Next => session.TrySkipNextAsync(),
        };
        match op {
            Ok(_op) => {
                #[cfg(debug_assertions)]
                let _ = _op.when(move |r| eprintln!("[media] command {control:?} -> {r:?}"));
                true
            }
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("[media] command {control:?} failed to start: {_e}");
                false
            }
        }
    }

    /// Whether the session shown lets its position be changed (the app
    /// says so through its transport controls).
    pub fn can_seek(&self) -> bool {
        self.session.as_ref().is_some_and(|s| {
            s.GetPlaybackInfo()
                .and_then(|i| i.Controls())
                .and_then(|c| c.IsPlaybackPositionEnabled())
                .unwrap_or(false)
        })
    }

    /// UI thread: asks the session shown to play from `position_ms` into its
    /// timeline. Fire-and-forget like `send_command` (the new position
    /// arrives as a timeline event). Nothing is sent if the current session
    /// is no longer the one shown or it does not allow seeking. Returns
    /// whether it was sent.
    pub fn send_seek(&self, position_ms: i64) -> bool {
        let Some(session) = self
            .manager
            .as_ref()
            .and_then(|m| m.GetCurrentSession().ok())
        else {
            return false;
        };
        if Some(&session) != self.session.as_ref() || !self.can_seek() {
            return false;
        }
        let start = session
            .GetTimelineProperties()
            .and_then(|t| t.StartTime())
            .map_or(0, |d| d.Duration);
        let ticks = start.saturating_add(position_ms.max(0).saturating_mul(10_000));
        session.TryChangePlaybackPositionAsync(ticks).is_ok()
    }

    /// Re-reads playback info of the current session (synchronous WinRT call).
    fn refresh_playback(&mut self) -> bool {
        let Some(session) = &self.session else {
            return false;
        };
        let playback =
            PlaybackState::from_status(session.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()));
        // Play/pause re-anchors the position: re-read the timeline too
        self.state.apply_playback(playback) | self.refresh_timeline()
    }

    /// Re-reads the timeline of the current session (synchronous WinRT call).
    fn refresh_timeline(&mut self) -> bool {
        let Some(session) = &self.session else {
            return false;
        };
        let timeline = read_timeline(session);
        self.state.apply_timeline(timeline)
    }
}

/// Current session timeline via `GetTimelineProperties` (None if unavailable).
fn read_timeline(session: &Session) -> Option<Timeline> {
    let t = session.GetTimelineProperties().ok()?;
    Timeline::from_raw(
        t.StartTime().ok()?.Duration,
        t.EndTime().ok()?.Duration,
        t.Position().ok()?.Duration,
        t.LastUpdatedTime().ok()?.UniversalTime,
        now_filetime(),
    )
}

impl Drop for MediaEngine {
    fn drop(&mut self) {
        self.unsubscribe_session();
        if let (Some(manager), Some(token)) = (&self.manager, self.session_changed_token.take()) {
            let _ = manager.RemoveCurrentSessionChanged(token);
        }
    }
}

// ============================================================================
// UNIT TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn available(id: &str) -> MediaSessionState {
        MediaSessionState::for_session(Some(&HSTRING::from(id)), PlaybackState::Unknown)
    }

    fn raw(title: &str, artist: &str, album: &str) -> RawMetadata {
        RawMetadata {
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
        }
    }

    fn metadata(state: &MediaSessionState) -> Option<&MediaMetadata> {
        match state {
            MediaSessionState::SessionAvailable { metadata, .. } => Some(metadata),
            MediaSessionState::NoSession => None,
        }
    }

    fn playback(state: &MediaSessionState) -> Option<PlaybackState> {
        match state {
            MediaSessionState::SessionAvailable { playback, .. } => Some(*playback),
            MediaSessionState::NoSession => None,
        }
    }

    #[test]
    fn test_media_session_state_session_available() {
        match available("Spotify.exe") {
            MediaSessionState::SessionAvailable { source, .. } => {
                assert_eq!(source.app_id, "Spotify.exe");
                assert_eq!(source.name, AppName::Pending);
            }
            _ => panic!("Expected SessionAvailable state"),
        }
    }

    #[test]
    fn test_media_engine_default_state() {
        let engine = MediaEngine::new();
        assert_eq!(engine.state(), &MediaSessionState::NoSession);
    }

    #[test]
    fn test_no_session_source_gives_no_session() {
        assert_eq!(
            MediaSessionState::for_session(None, PlaybackState::Playing),
            MediaSessionState::NoSession
        );
    }

    #[test]
    fn test_empty_source_id_is_still_a_session() {
        assert!(matches!(
            available(""),
            MediaSessionState::SessionAvailable { .. }
        ));
    }

    #[test]
    fn test_title_artist_album_applied() {
        let mut s = available("App");
        assert!(s.apply_metadata(&raw("Song", "Artist", "Album")));
        assert_eq!(
            metadata(&s),
            Some(&MediaMetadata {
                title: "Song".into(),
                artist: "Artist".into(),
                album: "Album".into(),
            })
        );
    }

    #[test]
    fn test_empty_metadata_is_safe_and_unchanged_on_fresh_session() {
        let mut s = available("App");
        assert!(!s.apply_metadata(&RawMetadata::default()));
        assert_eq!(metadata(&s), Some(&MediaMetadata::default()));
    }

    #[test]
    fn test_metadata_change_detected() {
        let mut s = available("App");
        s.apply_metadata(&raw("Song A", "Artist", ""));
        assert!(s.apply_metadata(&raw("Song B", "Artist", "")));
        assert_eq!(metadata(&s).unwrap().title, "Song B");
    }

    #[test]
    fn test_identical_metadata_is_not_a_change() {
        let mut s = available("App");
        s.apply_metadata(&raw("Song", "Artist", "Album"));
        assert!(!s.apply_metadata(&raw("Song", "Artist", "Album")));
    }

    #[test]
    fn test_metadata_cleared_to_empty_is_a_change() {
        let mut s = available("App");
        s.apply_metadata(&raw("Song", "Artist", "Album"));
        assert!(s.apply_metadata(&RawMetadata::default()));
        assert_eq!(metadata(&s), Some(&MediaMetadata::default()));
    }

    #[test]
    fn test_metadata_ignored_without_session() {
        let mut s = MediaSessionState::NoSession;
        assert!(!s.apply_metadata(&raw("Song", "Artist", "Album")));
        assert_eq!(s, MediaSessionState::NoSession);
    }

    #[test]
    fn test_playback_status_mapping() {
        use PlaybackState as P;
        let cases = [
            (PlaybackStatus::Closed, P::Closed),
            (PlaybackStatus::Opened, P::Opened),
            (PlaybackStatus::Changing, P::Changing),
            (PlaybackStatus::Stopped, P::Stopped),
            (PlaybackStatus::Playing, P::Playing),
            (PlaybackStatus::Paused, P::Paused),
            (PlaybackStatus(99), P::Unknown),
        ];
        for (status, expected) in cases {
            assert_eq!(PlaybackState::from_status(Ok(status)), expected);
        }
        assert_eq!(
            PlaybackState::from_status(Err(windows::core::Error::empty())),
            P::Unknown
        );
    }

    #[test]
    fn test_playback_transitions() {
        let mut s = available("App");
        assert!(s.apply_playback(PlaybackState::Playing));
        assert!(s.apply_playback(PlaybackState::Paused));
        assert!(!s.apply_playback(PlaybackState::Paused));
        assert!(s.apply_playback(PlaybackState::Playing));
        assert_eq!(playback(&s), Some(PlaybackState::Playing));
    }

    #[test]
    fn test_playback_ignored_without_session() {
        let mut s = MediaSessionState::NoSession;
        assert!(!s.apply_playback(PlaybackState::Playing));
    }

    #[test]
    fn test_session_replacement_clears_then_loads_new_metadata() {
        let mut s = available("Old");
        s.apply_metadata(&raw("Old Song", "Old Artist", "Old Album"));
        s.apply_playback(PlaybackState::Playing);

        let mut s2 =
            MediaSessionState::for_session(Some(&HSTRING::from("New")), PlaybackState::Paused);
        assert_eq!(metadata(&s2), Some(&MediaMetadata::default()));
        assert_eq!(playback(&s2), Some(PlaybackState::Paused));
        assert!(s2.apply_metadata(&raw("New Song", "New Artist", "")));
        assert_eq!(metadata(&s2).unwrap().title, "New Song");
        assert_ne!(s, s2);
    }

    #[test]
    fn test_no_session_clears_media_data() {
        let mut s = available("App");
        s.apply_metadata(&raw("Song", "Artist", "Album"));
        s = MediaSessionState::for_session(None, PlaybackState::Unknown);
        assert_eq!(metadata(&s), None);
        assert_eq!(playback(&s), None);
    }

    #[test]
    fn test_stale_generation_metadata_is_discarded() {
        let mut engine = MediaEngine::new();
        engine.state = available("New");
        engine.metadata_seq = 2;
        engine
            .metadata_tx
            .send((1, raw("Old Song", "Old Artist", "")))
            .unwrap();
        assert!(!engine.on_metadata_ready());
        assert_eq!(metadata(&engine.state), Some(&MediaMetadata::default()));

        engine
            .metadata_tx
            .send((2, raw("New Song", "", "")))
            .unwrap();
        assert!(!engine.on_metadata_ready(), "text waits for its artwork");
        engine.artwork_tx.send((2, None)).unwrap();
        assert!(engine.on_artwork_ready());
        assert_eq!(metadata(&engine.state).unwrap().title, "New Song");
    }

    #[test]
    fn test_messages_without_manager_are_noops() {
        let mut engine = MediaEngine::new();
        for msg in WM_APP_MEDIA_MANAGER_READY..=WM_APP_MEDIA_PLAYBACK_CHANGED {
            assert!(is_media_message(msg));
            assert!(!engine.handle_message(HWND::default(), msg));
        }
        assert!(!is_media_message(WM_APP));
        assert_eq!(engine.state(), &MediaSessionState::NoSession);
    }

    #[test]
    fn test_play_pause_icon_mapping() {
        use PlaybackState as P;
        assert_eq!(P::Playing.play_pause_icon(), PlayPauseIcon::Pause);
        assert_eq!(P::Paused.play_pause_icon(), PlayPauseIcon::Play);
        assert_eq!(P::Stopped.play_pause_icon(), PlayPauseIcon::Play);
        // Neutral/unknown states fall back safely to Play
        for p in [P::Unknown, P::Closed, P::Opened, P::Changing] {
            assert_eq!(p.play_pause_icon(), PlayPauseIcon::Play);
        }
    }

    #[test]
    fn test_control_accessible_names() {
        use MediaControl as C;
        assert_eq!(
            C::Previous.accessible_name(PlayPauseIcon::Play),
            "Previous track"
        );
        assert_eq!(C::Next.accessible_name(PlayPauseIcon::Pause), "Next track");
        assert_eq!(C::PlayPause.accessible_name(PlayPauseIcon::Play), "Play");
        assert_eq!(C::PlayPause.accessible_name(PlayPauseIcon::Pause), "Pause");
    }

    #[test]
    fn test_send_command_without_manager_is_noop() {
        let engine = MediaEngine::new();
        for c in MediaControl::ALL {
            assert!(!engine.send_command(c));
        }
    }

    fn content(state: &MediaSessionState) -> Option<MediaContent> {
        MediaContent::from_state(state)
    }

    #[test]
    fn test_content_session_appears_and_disappears() {
        assert_eq!(content(&MediaSessionState::NoSession), None);
        let mut s = available("App");
        s.apply_metadata(&raw("Song", "Artist", ""));
        s.apply_playback(PlaybackState::Playing);
        let c = content(&s).unwrap();
        assert_eq!((c.title.as_str(), c.subtitle.as_str()), ("Song", "Artist"));
        assert_eq!(c.icon, PlayPauseIcon::Pause);
        assert_eq!(c.artist.as_deref(), Some("Artist"));
        assert_eq!(c.album, None);
        assert_eq!(c.playback, PlaybackState::Playing);
        assert_eq!(content(&MediaSessionState::NoSession), None);
    }

    #[test]
    fn test_content_metadata_and_playback_changes() {
        let mut s = available("App");
        s.apply_metadata(&raw("Song A", "Artist", "Album"));
        let a = content(&s).unwrap();
        assert_eq!(a.subtitle, "Artist", "album not displayed");
        assert_eq!(a.icon, PlayPauseIcon::Play);

        s.apply_metadata(&raw("Song B", "Artist", "Album"));
        s.apply_playback(PlaybackState::Playing);
        let b = content(&s).unwrap();
        assert_eq!(b.title, "Song B");
        assert_eq!(b.icon, PlayPauseIcon::Pause);
        assert_ne!(a, b);
    }

    #[test]
    fn test_content_session_change_drops_old_metadata() {
        let mut old = available("Old");
        old.apply_metadata(&raw("Old Song", "Old Artist", ""));
        let new =
            MediaSessionState::for_session(Some(&HSTRING::from("New")), PlaybackState::Paused);
        let c = content(&new).unwrap();
        assert_eq!(c.title, MEDIA_TITLE_FALLBACK);
        assert_eq!(c.subtitle, "");
        assert_ne!(content(&old), Some(c));
    }

    #[test]
    fn test_content_fallbacks() {
        let mut s = available("App");
        // Whitespace-only title uses fallback; artist omitted; album shown alone
        s.apply_metadata(&raw("   ", "", "Album"));
        let c = content(&s).unwrap();
        assert_eq!(c.title, MEDIA_TITLE_FALLBACK);
        assert_eq!(c.subtitle, "", "album alone is not displayed");
        assert_eq!(c.album.as_deref(), Some("Album"), "but kept in the model");
        assert_eq!(c.accessible_text(), "Now playing");

        s.apply_metadata(&raw("Song", "", ""));
        let c = content(&s).unwrap();
        assert_eq!(c.subtitle, "");
        assert_eq!(c.accessible_text(), "Song");
    }

    // ---- Artwork -------------------------------------------------------------

    fn art(w: u32, h: u32, fill: u8) -> Artwork {
        Artwork::new(w, h, vec![fill; (w * h * 4) as usize]).unwrap()
    }

    fn artwork_of(state: &MediaSessionState) -> Option<&Artwork> {
        match state {
            MediaSessionState::SessionAvailable { artwork, .. } => artwork.as_ref(),
            MediaSessionState::NoSession => None,
        }
    }

    #[test]
    fn test_artwork_validation() {
        assert!(Artwork::new(2, 2, vec![0; 16]).is_some());
        assert!(Artwork::new(0, 2, vec![]).is_none(), "empty");
        assert!(Artwork::new(2, 2, vec![0; 15]).is_none(), "short buffer");
        assert!(Artwork::new(2, 2, vec![0; 17]).is_none(), "long buffer");
        assert!(
            Artwork::new(u32::MAX, u32::MAX, vec![]).is_none(),
            "overflow"
        );
    }

    #[test]
    fn test_artwork_change_detection() {
        assert_eq!(art(4, 4, 7), art(4, 4, 7), "identical pixels are equal");
        assert_ne!(art(4, 4, 7), art(4, 4, 8), "different pixels");
        assert_ne!(art(4, 4, 7), art(2, 8, 7), "same bytes, different shape");
    }

    #[test]
    fn test_artwork_available_replaced_and_cleared() {
        let mut s = available("App");
        assert_eq!(artwork_of(&s), None, "new session starts without artwork");
        assert!(s.apply_artwork(Some(art(4, 4, 1))));
        assert_eq!(artwork_of(&s), Some(&art(4, 4, 1)));
        assert!(
            !s.apply_artwork(Some(art(4, 4, 1))),
            "same artwork is not a change"
        );
        assert!(
            s.apply_artwork(Some(art(4, 4, 2))),
            "new track artwork replaces"
        );
        assert_eq!(artwork_of(&s), Some(&art(4, 4, 2)));
        assert!(s.apply_artwork(None), "unavailable artwork clears");
        assert_eq!(artwork_of(&s), None);
        assert!(!MediaSessionState::NoSession.apply_artwork(Some(art(1, 1, 0))));
    }

    #[test]
    fn test_artwork_replacement_releases_old_buffer() {
        let mut s = available("App");
        let first = art(8, 8, 1);
        let weak = Arc::downgrade(&first.pixels);
        s.apply_artwork(Some(first));
        assert!(weak.upgrade().is_some());
        s.apply_artwork(Some(art(8, 8, 2)));
        assert!(weak.upgrade().is_none(), "old pixels freed on replacement");

        let second = Arc::downgrade(&artwork_of(&s).unwrap().pixels);
        s = MediaSessionState::for_session(None, PlaybackState::Unknown);
        assert!(
            second.upgrade().is_none(),
            "pixels freed when session disappears"
        );
        assert_eq!(artwork_of(&s), None);
    }

    #[test]
    fn test_stale_artwork_discarded_track_and_session() {
        let mut engine = MediaEngine::new();
        engine.state = available("App");
        // Track A requested (seq 1), then track B requested (seq 2)
        engine.metadata_seq = 2;
        engine.artwork_tx.send((1, Some(art(4, 4, 0xA)))).unwrap();
        assert!(!engine.on_artwork_ready(), "track A artwork discarded");
        assert_eq!(artwork_of(&engine.state), None);
        engine.artwork_tx.send((2, Some(art(4, 4, 0xB)))).unwrap();
        assert!(engine.on_artwork_ready());
        assert_eq!(artwork_of(&engine.state), Some(&art(4, 4, 0xB)));

        // Session switch bumps the sequence: B's late duplicate is ignored too
        engine.state = available("Other");
        engine.metadata_seq = 3;
        engine.artwork_tx.send((2, Some(art(4, 4, 0xB)))).unwrap();
        assert!(
            !engine.on_artwork_ready(),
            "session A artwork discarded for B"
        );
        assert_eq!(artwork_of(&engine.state), None);
        assert!(
            engine.artwork_rx.try_recv().is_err(),
            "no results accumulate"
        );
    }

    #[test]
    fn test_artwork_delivery_sends_none_when_dropped() {
        let (tx, rx) = channel();
        drop(ArtworkDelivery::new(tx.clone(), 5, 0));
        assert!(matches!(rx.try_recv(), Ok((5, None))));
        ArtworkDelivery::new(tx, 6, 0).send(Some(art(1, 1, 0)));
        assert!(matches!(rx.try_recv(), Ok((6, Some(_)))));
        assert!(rx.try_recv().is_err(), "exactly one result per delivery");
    }

    #[test]
    fn test_fit_within() {
        assert_eq!(fit_within(300, 300, 256), (256, 256));
        assert_eq!(fit_within(640, 360, 256), (256, 144));
        assert_eq!(fit_within(100, 400, 256), (64, 256));
        assert_eq!(fit_within(120, 80, 256), (120, 80), "never upscales");
        assert_eq!(fit_within(0, 10, 256), (0, 10));
        assert_eq!(fit_within(10_000, 1, 256), (256, 1));
    }

    // ---- Source application --------------------------------------------------

    fn source_of(state: &MediaSessionState) -> Option<&SourceApp> {
        state.source()
    }

    #[test]
    fn test_source_aumid_stored_and_pending() {
        let s = available("Vendor.App_abc!App");
        assert_eq!(
            source_of(&s),
            Some(&SourceApp {
                app_id: "Vendor.App_abc!App".into(),
                name: AppName::Pending,
                icon: None,
            })
        );
    }

    #[test]
    fn test_source_name_available_and_unavailable() {
        let mut s = available("App");
        assert!(s.apply_source_name("App", AppName::Available("App Name".into())));
        assert_eq!(
            source_of(&s).unwrap().name,
            AppName::Available("App Name".into())
        );
        assert!(!s.apply_source_name("App", AppName::Available("App Name".into())));

        let mut u = available("unknown.exe");
        assert!(u.apply_source_name("unknown.exe", AppName::Unavailable));
        let c = MediaContent::from_state(&u).unwrap();
        assert_eq!(c.source.app_id, "unknown.exe", "AUMID kept when unresolved");
        assert_eq!(c.source.name, AppName::Unavailable);
    }

    #[test]
    fn test_source_name_for_other_app_is_ignored() {
        let mut s = available("B");
        assert!(!s.apply_source_name("A", AppName::Available("App A".into())));
        assert_eq!(source_of(&s).unwrap().name, AppName::Pending);
    }

    #[test]
    fn test_stale_source_lookup_discarded_and_cached() {
        let mut engine = MediaEngine::new();
        engine.state = available("B");
        engine.generation = 2;
        // Lookup for session A (generation 1) completes late
        engine
            .source_tx
            .send((1, "A".into(), AppName::Available("App A".into()), None))
            .unwrap();
        assert!(!engine.on_source_ready());
        assert_eq!(source_of(&engine.state).unwrap().name, AppName::Pending);
        assert_eq!(engine.source_cache, None, "stale result not cached");

        engine
            .source_tx
            .send((2, "B".into(), AppName::Available("App B".into()), None))
            .unwrap();
        assert!(engine.on_source_ready());
        assert_eq!(
            source_of(&engine.state).unwrap().name,
            AppName::Available("App B".into())
        );
        assert_eq!(
            engine.source_cache,
            Some(SourceApp {
                app_id: "B".into(),
                name: AppName::Available("App B".into()),
                icon: None,
            })
        );
    }

    #[test]
    fn test_session_switch_resets_source_and_artwork() {
        let mut a = available("A");
        a.apply_source_name("A", AppName::Available("App A".into()));
        a.apply_artwork(Some(art(4, 4, 1)));
        let b = MediaSessionState::for_session(Some(&HSTRING::from("B")), PlaybackState::Playing);
        assert_eq!(source_of(&b).unwrap().app_id, "B");
        assert_eq!(source_of(&b).unwrap().name, AppName::Pending);
        assert_eq!(artwork_of(&b), None);
    }

    #[test]
    fn test_is_executable_name() {
        assert!(is_executable_name("Player.exe"));
        assert!(is_executable_name("PLAYER.EXE"));
        assert!(!is_executable_name(".exe"));
        assert!(!is_executable_name("Vendor.App_abc!App"));
        assert!(!is_executable_name("C:\\Apps\\Player.exe"));
        assert!(!is_executable_name("MSEdge"));
    }

    /// Machine-dependent: prints what Windows resolves for real AUMIDs.
    /// Run with `OECA_AUMIDS="A;B" cargo test resolve_real -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn resolve_real_aumids() {
        use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
        // Same apartment as the Windows thread pool work item in production
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        for id in std::env::var("OECA_AUMIDS").unwrap_or_default().split(';') {
            let t = std::time::Instant::now();
            println!("{id:?} -> {:?} ({:?})", resolve_app_name(id), t.elapsed());
        }
    }

    /// Stress (manual): concurrent shell lookups from MTA threads, mirroring the
    /// thread-pool work items. `cargo test stress_resolve -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn stress_resolve_app_name_concurrent() {
        use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
        let ids = [
            "MSEdge",
            "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic",
            "Spotify.exe",
            "Oeca.Missing!App",
        ];
        let handles: Vec<_> = (0..8)
            .map(|t| {
                std::thread::spawn(move || {
                    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                    for i in 0..150 {
                        let _ = resolve_app_name(ids[(i + t) % ids.len()]);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        println!("concurrent lookups completed");
    }

    #[test]
    fn test_resolve_empty_or_unknown_app_is_unavailable() {
        assert_eq!(resolve_app_name(""), AppName::Unavailable);
        assert_eq!(
            resolve_app_name("Oeca.Test.Definitely.Not.Installed!App"),
            AppName::Unavailable
        );
    }

    // ---- Media model -----------------------------------------------------------

    #[test]
    fn test_content_complete_metadata() {
        let mut s = available("App");
        s.apply_metadata(&raw("Song", "Artist", "Album"));
        s.apply_playback(PlaybackState::Paused);
        s.apply_artwork(Some(art(2, 2, 9)));
        s.apply_source_name("App", AppName::Available("App Name".into()));
        let c = MediaContent::from_state(&s).unwrap();
        assert_eq!(c.title, "Song");
        assert_eq!(c.artist.as_deref(), Some("Artist"));
        assert_eq!(c.album.as_deref(), Some("Album"));
        assert_eq!(c.playback, PlaybackState::Paused);
        assert_eq!(c.artwork, Some(art(2, 2, 9)));
        assert_eq!(c.source.name, AppName::Available("App Name".into()));
    }

    #[test]
    fn test_content_missing_fields_are_none() {
        let mut s = available("");
        s.apply_metadata(&raw("Song", "", ""));
        let c = MediaContent::from_state(&s).unwrap();
        assert_eq!(c.artist, None);
        assert_eq!(c.album, None);
        assert_eq!(c.artwork, None);
        assert_eq!(c.source.app_id, "");
        assert_eq!(c.source.name, AppName::Pending);
        assert_eq!(
            MediaContent::from_state(&MediaSessionState::NoSession),
            None
        );
    }

    // ---- Phase 3.8: control feedback, text/artwork sync, source in a11y --------

    #[test]
    fn test_feedback_hover_fades_in_and_out() {
        use MediaControl as C;
        let mut f = ControlFeedback::default();
        assert!(!f.is_animating(), "idle costs nothing");
        assert!(f.set_hovered(Some(C::Next)));
        assert!(!f.set_hovered(Some(C::Next)), "same target is not a change");
        assert!(f.is_animating());
        f.step(MEDIA_HOVER_FADE_MS / 2.0);
        let (h, _) = f.levels(C::Next);
        assert!(h > 0.4 && h < 0.6, "half-way after half the fade: {h}");
        assert!(!f.step(MEDIA_HOVER_FADE_MS), "settles at full hover");
        assert_eq!(f.levels(C::Next), (1.0, 0.0));
        assert_eq!(
            f.levels(C::Previous),
            (0.0, 0.0),
            "other controls untouched"
        );
        f.set_hovered(None);
        assert!(!f.step(MEDIA_HOVER_FADE_MS * 2.0));
        assert_eq!(f.levels(C::Next), (0.0, 0.0));
    }

    #[test]
    fn test_feedback_press_animates_in_and_tap_completes() {
        use MediaControl as C;
        let mut f = ControlFeedback::default();
        f.press(C::PlayPause);
        assert_eq!(f.levels(C::PlayPause).1, 0.0, "press eases in");
        assert!(f.is_animating());
        assert!(!f.step(MEDIA_PRESS_IN_MS * 2.0), "held press settles");
        assert_eq!(f.levels(C::PlayPause).1, 1.0);
        assert_eq!(f.release(), Some(C::PlayPause));
        assert_eq!(f.release(), None, "release reports once");
        f.step(MEDIA_PRESS_RELEASE_MS / 2.0);
        let p = f.levels(C::PlayPause).1;
        assert!(p > 0.4 && p < 0.6, "eases back: {p}");
        assert!(!f.step(MEDIA_PRESS_RELEASE_MS));
        assert_eq!(f.levels(C::PlayPause), (0.0, 0.0));

        // A quick tap still shows the full shrink before springing back
        f.press(C::Next);
        f.step(MEDIA_PRESS_IN_MS / 4.0);
        f.release();
        f.step(MEDIA_PRESS_IN_MS);
        assert_eq!(f.levels(C::Next).1, 1.0, "tap reaches full press");
        assert!(!f.step(MEDIA_PRESS_RELEASE_MS * 2.0));
        assert_eq!(f.levels(C::Next).1, 0.0);
    }

    #[test]
    fn test_clipboard_buttons_grow_and_settle_back() {
        use crate::layout::PanelHit;
        let mut f = ControlFeedback::default();
        let trash = PanelHit::Remove(1);
        assert!(!f.is_animating(), "idle costs nothing");
        assert!(f.set_clip_hovered(Some(trash)) && f.is_animating());
        f.step(30.0);
        let (h1, _) = f.clip_levels(trash);
        assert!(h1 > 0.0 && h1 < 1.0, "eases in, not a jump: {h1}");
        while f.step(16.0) {}
        assert_eq!(f.clip_levels(trash), (1.0, 0.0), "fully hovered");
        f.press_clip(trash);
        while f.step(16.0) {}
        assert_eq!(f.clip_levels(trash), (1.0, 1.0), "pressed on top of hover");
        assert!(f.release_clip() && !f.release_clip());
        f.set_clip_hovered(None);
        f.step(30.0);
        let (h2, p2) = f.clip_levels(trash);
        assert!(h2 < 1.0 && p2 < 1.0 && (h2 > 0.0 || p2 > 0.0), "eases back");
        while f.step(16.0) {}
        assert_eq!(f.clip_levels(trash), (0.0, 0.0));
        assert!(f.clip.is_empty(), "settled buttons are dropped");
        // Other buttons are independent
        f.set_clip_hovered(Some(PanelHit::Copy(0)));
        while f.step(16.0) {}
        assert_eq!(f.clip_levels(trash), (0.0, 0.0));
        assert_eq!(f.clip_levels(PanelHit::Copy(0)).0, 1.0);
        // The hovered row fades in; moving to the next row fades the first out
        f.set_clip_hovered(None);
        f.set_clip_row(Some(0));
        while f.step(16.0) {}
        assert_eq!(f.clip_levels(PanelHit::Row(0)).0, 1.0);
        assert!(f.set_clip_row(Some(1)));
        while f.step(16.0) {}
        assert_eq!(
            f.clip_levels(PanelHit::Row(0)).0,
            0.0,
            "previous row hidden"
        );
        assert_eq!(f.clip_levels(PanelHit::Row(1)).0, 1.0);
        f.reset();
        assert!(f.clip.is_empty() && !f.is_animating());
    }

    #[test]
    fn test_feedback_reset_clears_everything() {
        let mut f = ControlFeedback::default();
        f.set_hovered(Some(MediaControl::Previous));
        f.press(MediaControl::Next);
        f.step(5.0);
        f.reset();
        assert_eq!(f, ControlFeedback::default());
        assert!(!f.is_animating());
    }

    #[test]
    fn test_track_change_text_waits_for_its_artwork() {
        let mut engine = MediaEngine::new();
        engine.state = available("App");
        engine.metadata_seq = 1;
        engine
            .metadata_tx
            .send((1, raw("Track A", "Artist", "")))
            .unwrap();
        engine.on_metadata_ready();
        engine.artwork_tx.send((1, Some(art(4, 4, 0xA)))).unwrap();
        assert!(engine.on_artwork_ready());
        assert_eq!(metadata(&engine.state).unwrap().title, "Track A");

        // Track B: its title must not appear next to Track A's artwork
        engine.metadata_seq = 2;
        engine
            .metadata_tx
            .send((2, raw("Track B", "Artist", "")))
            .unwrap();
        assert!(!engine.on_metadata_ready());
        assert_eq!(metadata(&engine.state).unwrap().title, "Track A");
        assert_eq!(artwork_of(&engine.state), Some(&art(4, 4, 0xA)));
        engine.artwork_tx.send((2, Some(art(4, 4, 0xB)))).unwrap();
        assert!(
            engine.on_artwork_ready(),
            "title and artwork switch together"
        );
        assert_eq!(metadata(&engine.state).unwrap().title, "Track B");
        assert_eq!(artwork_of(&engine.state), Some(&art(4, 4, 0xB)));
    }

    #[test]
    fn test_track_without_artwork_and_artwork_first_ordering() {
        let mut engine = MediaEngine::new();
        engine.state = available("App");
        engine.state.apply_artwork(Some(art(4, 4, 1)));
        // Artwork result (None) can arrive before the text (failed request path)
        engine.metadata_seq = 7;
        engine.artwork_tx.send((7, None)).unwrap();
        assert!(engine.on_artwork_ready(), "artwork present -> absent");
        assert_eq!(artwork_of(&engine.state), None);
        engine
            .metadata_tx
            .send((7, raw("No Art Track", "", "")))
            .unwrap();
        assert!(
            engine.on_metadata_ready(),
            "text applies immediately once art is settled"
        );
        assert_eq!(metadata(&engine.state).unwrap().title, "No Art Track");

        // Absent -> present on the next request; text from a superseded request is dropped
        engine.metadata_seq = 9;
        engine.metadata_tx.send((8, raw("Stale", "", ""))).unwrap();
        engine
            .metadata_tx
            .send((9, raw("With Art", "", "")))
            .unwrap();
        engine.on_metadata_ready();
        engine.artwork_tx.send((9, Some(art(2, 2, 3)))).unwrap();
        assert!(engine.on_artwork_ready());
        assert_eq!(metadata(&engine.state).unwrap().title, "With Art");
        assert_eq!(artwork_of(&engine.state), Some(&art(2, 2, 3)));
    }

    #[test]
    fn test_accessible_text_includes_source_name() {
        let mut s = available("App");
        s.apply_metadata(&raw("Song", "Artist", ""));
        let c = MediaContent::from_state(&s).unwrap();
        assert_eq!(c.accessible_text(), "Song - Artist", "pending name omitted");
        assert_eq!(c.source_name(), None);
        s.apply_source_name("App", AppName::Available("Player".into()));
        let c = MediaContent::from_state(&s).unwrap();
        assert_eq!(c.source_name(), Some("Player"));
        assert_eq!(c.accessible_text(), "Song - Artist - Player");
        s.apply_source_name("App", AppName::Unavailable);
        let c = MediaContent::from_state(&s).unwrap();
        assert_eq!(
            c.accessible_text(),
            "Song - Artist",
            "unavailable name omitted"
        );
    }

    #[test]
    fn test_track_fade_curve_and_timer_lifetime() {
        let mut f = ControlFeedback::default();
        assert_eq!(f.track_alpha(), 1.0, "settled by default");
        assert!(!f.is_animating());
        f.start_track_fade();
        assert!((f.track_alpha() - MEDIA_TRACK_FADE_FLOOR).abs() < 1e-6);
        assert!(f.is_animating());
        f.step(MEDIA_TRACK_FADE_MS / 2.0);
        let mid = f.track_alpha();
        assert!(mid > MEDIA_TRACK_FADE_FLOOR && mid < 1.0);
        // Ease-out: more than half of the way after half the time
        assert!(mid > MEDIA_TRACK_FADE_FLOOR + (1.0 - MEDIA_TRACK_FADE_FLOOR) / 2.0);
        assert!(!f.step(MEDIA_TRACK_FADE_MS), "stops when settled");
        assert_eq!(f.track_alpha(), 1.0);
        f.start_track_fade();
        f.finish_track_fade();
        assert!(!f.is_animating(), "off-screen changes settle instantly");
    }

    #[test]
    fn test_callbacks_after_engine_drop_are_harmless() {
        // Completion callbacks hold only a Sender and the HWND value. Once the
        // engine (and its receivers) are gone, delivery fails quietly and posts nothing.
        let engine = MediaEngine::new();
        let art_tx = engine.artwork_tx.clone();
        let meta_tx = engine.metadata_tx.clone();
        let src_tx = engine.source_tx.clone();
        drop(engine);
        ArtworkDelivery::new(art_tx.clone(), 1, 0).send(Some(art(2, 2, 1)));
        drop(ArtworkDelivery::new(art_tx, 2, 0));
        assert!(meta_tx.send((1, RawMetadata::default())).is_err());
        assert!(
            src_tx
                .send((1, "X".into(), AppName::Unavailable, None))
                .is_err()
        );
    }

    #[test]
    fn test_session_loss_clears_everything_visible() {
        let mut s = available("App");
        s.apply_metadata(&raw("Song", "Artist", "Album"));
        s.apply_artwork(Some(art(4, 4, 1)));
        s.apply_source_name("App", AppName::Available("Player".into()));
        s.apply_playback(PlaybackState::Playing);
        assert!(MediaContent::from_state(&s).is_some());
        let gone = MediaSessionState::for_session(None, PlaybackState::Playing);
        assert_eq!(gone, MediaSessionState::NoSession);
        assert_eq!(
            MediaContent::from_state(&gone),
            None,
            "no stale title/art/source/state"
        );
        // Late results for the vanished session cannot resurrect content
        let mut gone = gone;
        assert!(!gone.apply_metadata(&raw("Late", "", "")));
        assert!(!gone.apply_artwork(Some(art(1, 1, 0))));
        assert!(!gone.apply_source_name("App", AppName::Available("Player".into())));
        assert!(!gone.apply_playback(PlaybackState::Paused));
        assert_eq!(gone, MediaSessionState::NoSession);
    }

    #[test]
    fn test_badge_apps_gating() {
        for name in ["Spotify", "Apple Music", "YouTube Music", "spotify premium"] {
            assert!(wants_badge(name), "{name}");
        }
        for name in ["Microsoft Edge", "Media Player", "VLC media player", ""] {
            assert!(!wants_badge(name), "{name}");
        }
    }

    #[test]
    fn test_badge_plate_uses_logo_colour() {
        // Opaque green disc (Spotify-like) on a transparent square
        let (w, h) = (64u32, 64u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as f32 + 0.5 - 32.0, y as f32 + 0.5 - 32.0);
                if dx * dx + dy * dy < 30.0 * 30.0 {
                    let i = ((y * w + x) * 4) as usize;
                    px[i..i + 4].copy_from_slice(&[0x60, 0xD7, 0x1E, 0xFF]);
                }
            }
        }
        let b = badge_pixels(px, w, h).expect("badge");
        // Inside the badge shape but outside the logo's disc: filled with the
        // logo's own green (no dark rim around the logo)
        let i = ((w + 32) * 4) as usize; // (32, 1)
        let p = &b.pixels[i..i + 4];
        assert_eq!(p[3], 255, "inside the badge shape");
        assert!(
            p[1] > 150 && p[1] > p[0] + 60 && p[1] > p[2] + 60,
            "green plate: {p:?}"
        );
        assert_eq!(b.pixels[3], 0, "outer corner still transparent");
    }

    #[test]
    fn test_badge_pixels_squircle_and_premultiplied() {
        // Straight-alpha white icon with a transparent background half
        let (w, h) = (32u32, 32u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w / 2 {
                let i = ((y * w + x) * 4) as usize;
                px[i..i + 4].copy_from_slice(&[255, 255, 255, 128]); // straight alpha
            }
        }
        let b = badge_pixels(px, w, h).expect("badge");
        let at = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [
                b.pixels[i],
                b.pixels[i + 1],
                b.pixels[i + 2],
                b.pixels[i + 3],
            ]
        };
        assert_eq!(at(0, 0), [0, 0, 0, 0], "squircle corner is transparent");
        assert_eq!(at(w - 1, h - 1)[3], 0);
        assert_eq!(at(w / 2 + 4, h / 2)[3], 255, "plate is opaque inside");
        let p = at(4, h / 2);
        assert_eq!(p[3], 255);
        assert!(p.iter().take(3).all(|c| *c <= p[3]), "premultiplied output");
        assert!(
            p[0] > 120 && p[0] < 160,
            "icon blended over plate, not raw white"
        );
        assert!(
            badge_pixels(vec![0; 3], 1, 1).is_none(),
            "malformed input rejected"
        );
    }

    #[test]
    fn test_source_icon_applied_cached_and_cleared() {
        let mut s = available("Spotify.exe");
        let icon = Some(art(4, 4, 7));
        assert!(s.apply_source(
            "Spotify.exe",
            AppName::Available("Spotify".into()),
            icon.clone()
        ));
        assert_eq!(source_of(&s).unwrap().icon, icon);
        assert!(!s.apply_source(
            "Spotify.exe",
            AppName::Available("Spotify".into()),
            icon.clone()
        ));
        let c = MediaContent::from_state(&s).unwrap();
        assert_eq!(c.source.icon, icon, "badge reaches the display model");
        // Another app's result never applies; a new session starts without a badge
        assert!(!s.apply_source("Other", AppName::Available("Other".into()), None));
        let next =
            MediaSessionState::for_session(Some(&HSTRING::from("MSEdge")), PlaybackState::Playing);
        assert_eq!(source_of(&next).unwrap().icon, None);
    }

    #[test]
    fn test_badge_replaces_source_text() {
        let mut s = available("Spotify.exe");
        s.apply_metadata(&raw("Song", "Artist", ""));
        s.apply_source(
            "Spotify.exe",
            AppName::Available("Spotify".into()),
            Some(art(4, 4, 1)),
        );
        // Badge app with artwork: badge shown, text line hidden, layout drops the row
        s.apply_artwork(Some(art(8, 8, 2)));
        let c = MediaContent::from_state(&s).unwrap();
        assert!(c.shows_badge() && !c.shows_source_text());
        assert_eq!(c.shape().secondary_lines, 1);
        assert!(
            c.accessible_text().contains("Spotify"),
            "name still announced"
        );
        // Badge app without artwork: no badge to carry it, so the text returns
        s.apply_artwork(None);
        let c = MediaContent::from_state(&s).unwrap();
        assert!(!c.shows_badge() && c.shows_source_text());
        assert_eq!(c.shape().secondary_lines, 2);
        // Any other app keeps its source text
        let mut e = available("MSEdge");
        e.apply_metadata(&raw("Song", "Artist", ""));
        e.apply_artwork(Some(art(8, 8, 2)));
        e.apply_source("MSEdge", AppName::Available("Microsoft Edge".into()), None);
        let c = MediaContent::from_state(&e).unwrap();
        assert!(!c.shows_badge() && c.shows_source_text());
    }

    // ---- Accent ----------------------------------------------------------------

    /// Premultiplied BGRA artwork from (rgb, alpha, share) regions laid out by rows.
    fn painted(regions: &[([u8; 3], u8, f32)]) -> Artwork {
        let (w, h) = (64u32, 64u32);
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        let mut bounds = Vec::new();
        let mut acc = 0.0;
        for (rgb, a, share) in regions {
            acc += share;
            bounds.push(((acc * h as f32).round() as u32, *rgb, *a));
        }
        for y in 0..h {
            let (_, [r, g, b], a) = *bounds.iter().find(|(end, _, _)| y < *end).unwrap();
            let pm = |c: u8| ((u32::from(c) * u32::from(a) + 127) / 255) as u8;
            for _ in 0..w {
                px.extend_from_slice(&[pm(b), pm(g), pm(r), a]);
            }
        }
        Artwork::new(w, h, px).unwrap()
    }

    fn solid(rgb: [u8; 3]) -> Artwork {
        painted(&[(rgb, 255, 1.0)])
    }

    fn hsl_of(a: Accent) -> (f32, f32, f32) {
        to_hsl(a.map(|v| f32::from(v) / 255.0))
    }

    /// Every accent is usable on #000: within the normalized range, never dark,
    /// never neon.
    fn assert_premium(a: Accent) {
        let (_, s, l) = hsl_of(a);
        let rgb = a.map(|v| f32::from(v) / 255.0);
        assert!((0.33..=0.77).contains(&s), "saturation {s} for {a:?}");
        assert!((0.34..=0.70).contains(&l), "lightness {l} for {a:?}");
        assert!(luma(rgb) <= 0.6, "too bright: {a:?}");
        assert!(luma(rgb) >= 0.15, "too dark: {a:?}");
    }

    /// Premultiplied BGRA artwork from a function of (x, y) -> (rgb, alpha).
    fn cover_of(w: u32, h: u32, f: impl Fn(u32, u32) -> ([u8; 3], u8)) -> Artwork {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let ([r, g, b], a) = f(x, y);
                let pm = |c: u8| (u32::from(c) * u32::from(a) / 255) as u8;
                px.extend([pm(b), pm(g), pm(r), a]);
            }
        }
        Artwork::new(w, h, px).unwrap()
    }

    fn hsv_of(c: Accent) -> (f32, f32, f32) {
        to_hsv(c.map(|v| f32::from(v) / 255.0))
    }

    fn hue_near(a: f32, b: f32, tol: f32) -> bool {
        let d = (a - b).abs();
        d.min(1.0 - d) <= tol
    }

    fn visible(c: Accent) -> bool {
        relative_luminance(c.map(|v| f32::from(v) / 255.0)) >= PALETTE_MIN_LUMINANCE - 0.005
    }

    #[test]
    fn test_gradient_of_vivid_single_colour_artwork_is_one_hue() {
        // A vivid red-orange cover with deep shadows and a white title: one
        // rich red-orange, lighter at the top, deeper at the bottom
        let art = cover_of(96, 96, |x, y| {
            if y < 12 {
                ([250, 250, 250], 255)
            } else if x < 20 {
                ([20, 6, 4], 255)
            } else {
                ([235, 60, 30], 255)
            }
        });
        let g = visualizer_gradient(&art).unwrap();
        let (top, bottom) = (hsv_of(g.top), hsv_of(g.bottom));
        assert!(hue_near(top.0, 0.025, 0.03) && hue_near(bottom.0, 0.025, 0.03));
        assert!(
            bottom.1 > 0.75 && bottom.2 > 0.7,
            "rich, not muted: {bottom:?}"
        );
        assert!(top.2 >= bottom.2, "lighter at the top");
        assert_ne!(g.top, g.bottom, "a gentle ramp, not flat");
        // The old accent of the same cover is the duller one
        assert!(hsv_of(artwork_accent(&art).unwrap()).1 < bottom.1);
        assert!(visible(g.top) && visible(g.bottom));
    }

    #[test]
    fn test_gradient_of_two_colour_artwork_blends_them() {
        // Mostly blue with a strong purple area and a small orange detail:
        // blue to purple; the small (and opposite) orange is left out
        let art = cover_of(120, 120, |x, y| {
            if x < 70 {
                ([30, 80, 230], 255)
            } else if y < 90 {
                ([140, 50, 220], 255)
            } else {
                ([245, 140, 30], 255)
            }
        });
        let g = visualizer_gradient(&art).unwrap();
        assert!(
            hue_near(hsv_of(g.top).0, 0.62, 0.04),
            "dominant blue on top: {g:?}"
        );
        assert!(
            hue_near(hsv_of(g.bottom).0, 0.75, 0.04),
            "purple below: {g:?}"
        );
        assert!(visible(g.top) && visible(g.bottom));
        // Red and yellow make a warm two-colour gradient too
        let warm = cover_of(64, 64, |x, _| {
            if x < 36 {
                ([225, 40, 50], 255)
            } else {
                ([245, 200, 40], 255)
            }
        });
        let g = visualizer_gradient(&warm).unwrap();
        assert!(
            hue_near(hsv_of(g.top).0, 0.99, 0.04) && hue_near(hsv_of(g.bottom).0, 0.13, 0.04),
            "{g:?}"
        );
        // Near-opposite colours (blue / orange) would blend through mud: the
        // dominant one alone
        let opposite = cover_of(64, 64, |x, _| {
            if x < 36 {
                ([30, 80, 230], 255)
            } else {
                ([245, 140, 30], 255)
            }
        });
        let g = visualizer_gradient(&opposite).unwrap();
        assert!(
            hue_near(hsv_of(g.top).0, hsv_of(g.bottom).0, 0.03),
            "one hue: {g:?}"
        );
        // A minor second colour (under a quarter of the main one): one hue
        let minor = cover_of(64, 64, |x, _| {
            if x < 58 {
                ([30, 80, 230], 255)
            } else {
                ([140, 50, 220], 255)
            }
        });
        let g = visualizer_gradient(&minor).unwrap();
        assert!(hue_near(hsv_of(g.top).0, hsv_of(g.bottom).0, 0.03), "{g:?}");
    }

    #[test]
    fn test_gradient_lifts_dark_hues_to_visible_without_greying() {
        let art = cover_of(64, 64, |_, _| ([12, 24, 110], 255));
        let g = visualizer_gradient(&art).unwrap();
        for c in [g.top, g.bottom] {
            assert!(visible(c), "visible on black: {c:?}");
            let (h, s, _) = hsv_of(c);
            assert!(hue_near(h, 0.64, 0.05) && s >= 0.45, "still blue: {c:?}");
        }
    }

    #[test]
    fn test_gradient_neutral_dark_and_transparent_artwork_fall_back() {
        let grey = cover_of(64, 64, |x, _| ([(x * 4) as u8; 3], 255));
        let mono = cover_of(64, 64, |x, y| {
            (
                if (x / 8 + y / 8) % 2 == 0 {
                    [0; 3]
                } else {
                    [255; 3]
                },
                255,
            )
        });
        let black = cover_of(64, 64, |_, _| ([8, 4, 10], 255));
        let clear = cover_of(64, 64, |_, _| ([255, 0, 0], 0));
        let washed = cover_of(64, 64, |_, _| ([235, 228, 230], 255));
        for (name, art) in [
            ("grey", grey),
            ("mono", mono),
            ("black", black),
            ("clear", clear),
            ("washed", washed),
        ] {
            assert_eq!(
                visualizer_gradient(&art),
                None,
                "{name}: the usual accent instead"
            );
        }
        // A coloured core in transparent artwork counts
        let core = cover_of(64, 64, |x, y| {
            if (20..44).contains(&x) && (20..44).contains(&y) {
                ([40, 200, 120], 255)
            } else {
                ([0, 0, 0], 0)
            }
        });
        assert!(hue_near(
            hsv_of(visualizer_gradient(&core).unwrap().bottom).0,
            0.42,
            0.04
        ));
        // Low saturation that is still coloured: kept subtle, not boosted
        let dusty = cover_of(64, 64, |_, _| ([150, 110, 170], 255));
        let g = visualizer_gradient(&dusty).unwrap();
        assert!(hsv_of(g.bottom).1 < 0.5, "dusty stays dusty");
        assert!(visible(g.bottom) && visible(g.top));
    }

    #[test]
    fn test_gradient_is_stable_for_the_same_artwork() {
        let art = cover_of(200, 200, |x, y| {
            ([(x % 256) as u8, (y % 256) as u8, 160], 255)
        });
        let g = visualizer_gradient(&art);
        for _ in 0..5 {
            assert_eq!(visualizer_gradient(&art), g, "deterministic");
        }
        let copy = Artwork::new(200, 200, art.pixels.to_vec()).unwrap();
        assert_eq!(visualizer_gradient(&copy), g, "same pixels, same gradient");
    }

    #[test]
    fn test_accent_dark_artwork_is_lifted() {
        // Near-black cover with a dim red motif
        let art = painted(&[([8, 8, 10], 255, 0.6), ([70, 12, 14], 255, 0.4)]);
        let a = artwork_accent(&art).expect("dim red is still a hue");
        assert_premium(a);
        assert!(a[0] > a[1] && a[0] > a[2], "keeps the red hue: {a:?}");
    }

    #[test]
    fn test_accent_bright_artwork_is_toned_down() {
        let a = artwork_accent(&solid([255, 190, 205])).expect("pastel pink");
        assert_premium(a);
        assert!(a[0] > a[1], "still pink: {a:?}");
    }

    #[test]
    fn test_accent_saturated_artwork_is_not_neon() {
        for rgb in [
            [0, 255, 0],
            [255, 255, 0],
            [0, 255, 255],
            [255, 0, 255],
            [0, 0, 255],
        ] {
            let a = artwork_accent(&solid(rgb)).expect("saturated");
            assert_premium(a);
            let (_, s, _) = hsl_of(a);
            assert!(s <= 0.77, "saturation capped for {rgb:?}: {a:?}");
        }
    }

    #[test]
    fn test_accent_neutral_artwork_uses_neutral_fallback() {
        for rgb in [[128, 128, 128], [0, 0, 0], [255, 255, 255], [40, 42, 44]] {
            assert_eq!(artwork_accent(&solid(rgb)), None, "grey {rgb:?}");
        }
        // A tiny coloured speck on a grey cover does not tint the UI
        let speck = painted(&[([120, 120, 120], 255, 0.99), ([255, 0, 0], 255, 0.01)]);
        assert_eq!(artwork_accent(&speck), None);
        // Fully transparent artwork
        assert_eq!(artwork_accent(&painted(&[([255, 0, 0], 0, 1.0)])), None);
    }

    #[test]
    fn test_accent_follows_dominant_hue_and_is_stable() {
        let art = painted(&[([20, 60, 220], 255, 0.7), ([230, 30, 30], 255, 0.3)]);
        let a = artwork_accent(&art).unwrap();
        assert!(a[2] > a[0] && a[2] > a[1], "blue dominates: {a:?}");
        // Identical artwork (separately decoded buffer) -> identical accent
        let again = painted(&[([20, 60, 220], 255, 0.7), ([230, 30, 30], 255, 0.3)]);
        assert_eq!(artwork_accent(&again), Some(a));
        assert_eq!(artwork_accent(&art), Some(a), "repeatable");
    }

    #[test]
    fn test_accent_set_with_artwork_and_cleared_with_it() {
        let accent_of = |s: &MediaSessionState| MediaContent::from_state(s).unwrap().accent;
        let mut s = available("App");
        assert_eq!(accent_of(&s), None, "no artwork -> neutral");
        let blue = painted(&[([20, 60, 220], 255, 1.0)]);
        assert!(s.apply_artwork(Some(blue.clone())));
        assert_eq!(
            accent_of(&s),
            artwork_accent(&blue),
            "accent swaps with artwork"
        );
        let red = solid([220, 30, 30]);
        assert!(s.apply_artwork(Some(red.clone())));
        assert_eq!(
            accent_of(&s),
            artwork_accent(&red),
            "never the previous accent"
        );
        assert!(s.apply_artwork(None));
        assert_eq!(accent_of(&s), None, "artwork gone -> neutral");
    }

    // ---- Timeline --------------------------------------------------------------

    const SEC: i64 = 10_000_000; // 100 ns ticks
    const NOW: i64 = 133_000_000_000_000_000;

    #[test]
    fn test_timeline_normalization() {
        let t = Timeline::from_raw(5 * SEC, 185 * SEC, 65 * SEC, NOW - SEC, NOW).unwrap();
        assert_eq!(t.duration_ms, 180_000, "relative to the start time");
        assert_eq!(t.position_ms, 60_000);
        assert_eq!(t.stamp, NOW - SEC);
        // Position outside the track is clamped
        let t = Timeline::from_raw(0, 100 * SEC, 500 * SEC, NOW, NOW).unwrap();
        assert_eq!(t.position_ms, 100_000);
        let t = Timeline::from_raw(10 * SEC, 100 * SEC, 0, NOW, NOW).unwrap();
        assert_eq!(t.position_ms, 0);
        // No meaningful duration (live stream / unknown)
        assert_eq!(Timeline::from_raw(0, 0, 0, NOW, NOW), None);
        assert_eq!(Timeline::from_raw(10 * SEC, 5 * SEC, 0, NOW, NOW), None);
        // Invalid LastUpdatedTime falls back to now
        assert_eq!(
            Timeline::from_raw(0, 60 * SEC, 0, 0, NOW).unwrap().stamp,
            NOW
        );
        assert_eq!(
            Timeline::from_raw(0, 60 * SEC, 0, NOW + SEC, NOW)
                .unwrap()
                .stamp,
            NOW
        );
    }

    #[test]
    fn test_timeline_position_pause_and_resume() {
        let t = Timeline::from_raw(0, 120 * SEC, 30 * SEC, NOW, NOW).unwrap();
        assert_eq!(
            t.position_at(NOW + 5 * SEC, true),
            35_000,
            "advances while playing"
        );
        assert_eq!(
            t.position_at(NOW + 5 * SEC, false),
            30_000,
            "frozen while paused"
        );
        assert_eq!(
            t.position_at(NOW + 500 * SEC, true),
            120_000,
            "clamped at the end"
        );
        assert_eq!(
            t.position_at(NOW - 5 * SEC, true),
            30_000,
            "clock skew never rewinds"
        );
    }

    fn clock(ms: i64, negative: bool) -> String {
        let mut buf = [0u16; 12];
        let n = format_clock(ms, negative, &mut buf);
        String::from_utf16(&buf[..n]).unwrap()
    }

    #[test]
    fn test_timeline_formatting() {
        assert_eq!(clock(0, false), "0:00");
        assert_eq!(clock(49_999, false), "0:49");
        assert_eq!(clock(128_000, true), "-2:08");
        assert_eq!(clock(600_000, false), "10:00");
        assert_eq!(clock(3_723_000, false), "1:02:03");
        assert_eq!(clock(36_000_000, true), "-10:00:00");
        assert_eq!(clock(-5, false), "0:00", "negative input clamps");
    }

    #[test]
    fn test_session_switch_clears_timeline_and_accent() {
        let mut s = available("A");
        s.apply_artwork(Some(solid([220, 30, 30])));
        assert!(s.apply_timeline(Timeline::from_raw(0, 60 * SEC, 0, NOW, NOW)));
        let c = MediaContent::from_state(&s).unwrap();
        assert!(c.accent.is_some() && c.timeline.is_some());
        // A new session starts clean: nothing from the previous track carries over
        let fresh = available("B");
        let c = MediaContent::from_state(&fresh).unwrap();
        assert_eq!((c.accent, c.timeline), (None, None));
        assert!(!MediaSessionState::NoSession.clone().apply_timeline(None));
    }

    // ---- Visualizer ------------------------------------------------------------

    #[test]
    fn test_visualizer_visible_only_while_playing() {
        use PlaybackState as P;
        for (p, shown) in [
            (P::Playing, true),
            (P::Paused, false),
            (P::Stopped, false),
            (P::Closed, false),
            (P::Unknown, false),
            (P::Opened, false),
            (P::Changing, false),
        ] {
            assert_eq!(shows_visualizer(p), shown, "{p:?}");
        }
    }

    #[test]
    fn test_visualizer_fades_in_and_out_then_stops() {
        let mut v = Visualizer::default();
        assert!(!v.is_active(), "idle costs nothing");
        assert!(v.set_playing(true));
        assert!(!v.set_playing(true), "no change");
        assert!(v.step(VISUALIZER_FADE_MS / 2.0));
        assert!((v.level() - 0.5).abs() < 1e-4, "eases in");
        v.step(VISUALIZER_FADE_MS);
        assert_eq!(v.level(), 1.0);
        assert!(v.is_active(), "keeps animating while playing");
        // Pause: fades out, then needs no more frames
        v.set_playing(false);
        assert!(v.step(VISUALIZER_FADE_MS / 2.0), "still fading");
        assert!(!v.step(VISUALIZER_FADE_MS), "stops once hidden");
        assert_eq!(v.level(), 0.0);
        // Resume from mid-fade continues smoothly from the current level
        v.set_playing(true);
        v.step(VISUALIZER_FADE_MS / 4.0);
        v.set_playing(false);
        v.step(VISUALIZER_FADE_MS / 8.0);
        v.set_playing(true);
        let before = v.level();
        v.step(16.0);
        assert!(
            v.level() > before && v.level() < 0.3,
            "no jump: {}",
            v.level()
        );
    }

    #[test]
    fn test_visualizer_motion_is_deterministic_smooth_and_bounded() {
        let mut v = Visualizer::default();
        v.set_playing(true);
        v.step(VISUALIZER_FADE_MS);
        let mut prev = v.bar_heights(0.0);
        let mut differs = false;
        for i in 1..2000 {
            let t = f64::from(i) * 0.016;
            let h = v.bar_heights(t);
            assert_eq!(h, v.bar_heights(t), "same time -> same heights");
            for (a, b) in h.iter().zip(prev.iter()) {
                assert!((VISUALIZER_MIN_HEIGHT..=1.0).contains(a));
                assert!((a - b).abs() < 0.12, "smooth frame to frame");
            }
            differs |= h.windows(2).any(|w| (w[0] - w[1]).abs() > 0.1);
            prev = h;
        }
        assert!(differs, "bars do not move in lockstep");
        // Hidden: bars rest at the minimum
        let hidden = Visualizer::default();
        assert_eq!(
            hidden.bar_heights(3.7),
            [VISUALIZER_MIN_HEIGHT; VISUALIZER_BARS]
        );
    }

    #[test]
    fn test_visualizer_settle_skips_hidden_fades() {
        let mut v = Visualizer::default();
        v.set_playing(true);
        v.settle();
        assert_eq!(v.level(), 1.0, "playing while off screen: shown at full");
        v.set_playing(false);
        v.settle();
        assert_eq!(
            v.level(),
            0.0,
            "paused while off screen: no stale fade later"
        );
        assert!(!v.is_active());
    }

    // ---- Transport glyph motion ----------------------------------------------

    fn run(m: &mut IconMotion, now: std::time::Instant, ms: u32) -> Vec<f32> {
        (0..ms / 16)
            .map(|_| {
                m.step(16.0, now);
                m.morph()
            })
            .collect()
    }

    #[test]
    fn test_play_pause_answers_a_click_at_once_in_both_directions() {
        let now = std::time::Instant::now();
        let mut deltas = Vec::new();
        for (actual, toward) in [(PlayPauseIcon::Play, 1.0f32), (PlayPauseIcon::Pause, 0.0)] {
            let mut m = IconMotion::default();
            m.sync(actual, now, true);
            let start = m.morph();
            m.clicked_play_pause(now, false);
            assert!(m.is_animating(), "frames start with the click");
            m.step(16.0, now);
            let moved = m.morph() - start;
            assert!(moved.abs() > 0.02, "the first frame already moves: {moved}");
            assert_eq!(moved.signum(), (toward - start).signum());
            deltas.push(moved.abs());
            // ...and it gets there without waiting for the session
            run(&mut m, now, 600);
            assert_eq!(m.morph(), toward);
        }
        assert!(
            (deltas[0] - deltas[1]).abs() < 1e-4,
            "same response both ways: {deltas:?}"
        );
    }

    #[test]
    fn test_play_pause_reverses_mid_way_without_a_jump() {
        let now = std::time::Instant::now();
        let mut m = IconMotion::default();
        m.sync(PlayPauseIcon::Play, now, true);
        m.clicked_play_pause(now, false);
        let path = run(&mut m, now, 80);
        let mid = *path.last().unwrap();
        assert!(mid > 0.1 && mid < 0.9, "mid-way: {mid}");
        m.clicked_play_pause(now, false); // back to play
        m.step(1.0, now);
        assert!((m.morph() - mid).abs() < 0.03, "continues from where it is");
        let back = run(&mut m, now, 600);
        assert_eq!(*back.last().unwrap(), 0.0);
        // Never outside the two glyphs (critically damped: no overshoot)
        assert!(path.iter().chain(&back).all(|v| (0.0..=1.0).contains(v)));
        // Smooth: no frame jumps more than a modest step
        let all: Vec<f32> = path.iter().chain(&back).copied().collect();
        assert!(all.windows(2).all(|w| (w[1] - w[0]).abs() < 0.2));
    }

    #[test]
    fn test_play_pause_confirmed_or_corrected_by_the_session() {
        let t0 = std::time::Instant::now();
        let ms = std::time::Duration::from_millis;
        // Confirmed: the session reports the new state
        let mut m = IconMotion::default();
        m.sync(PlayPauseIcon::Play, t0, true);
        m.clicked_play_pause(t0, false);
        // A stale report of the old state (e.g. a timeline event) changes nothing
        m.sync(PlayPauseIcon::Play, t0 + ms(100), false);
        run(&mut m, t0 + ms(100), 400);
        assert_eq!(m.morph(), 1.0, "still heading for pause");
        m.sync(PlayPauseIcon::Pause, t0 + ms(400), false);
        run(&mut m, t0 + ms(400), 200);
        assert!(!m.is_animating(), "confirmed: settles, no frames");
        // Ignored by the app: back to the reported glyph once the wait ends
        let mut m = IconMotion::default();
        m.sync(PlayPauseIcon::Play, t0, true);
        m.clicked_play_pause(t0, false);
        run(&mut m, t0, 300);
        assert!(m.is_animating(), "waits for the session");
        let late = t0 + ms(PLAY_PAUSE_CONFIRM_MS);
        run(&mut m, late, 600);
        assert_eq!(m.morph(), 0.0, "never left showing a wrong glyph");
        assert!(!m.is_animating());
    }

    #[test]
    fn test_skip_glyphs_slide_their_own_way_and_settle_after_rapid_clicks() {
        let now = std::time::Instant::now();
        let mut m = IconMotion::default();
        assert_eq!(m.skip_phase(MediaControl::Next), 0.0);
        m.clicked_skip(MediaControl::Next, false);
        m.step(16.0, now);
        let p = m.skip_phase(MediaControl::Next);
        assert!(p > 0.0 && p < 1.0, "moves at once: {p}");
        assert_eq!(
            m.skip_phase(MediaControl::Previous),
            0.0,
            "Previous untouched"
        );
        assert_eq!(m.skip_phase(MediaControl::PlayPause), 0.0);
        // Rapid clicks extend the same motion (bounded), never jumping
        let mut total = vec![m.skip[1].value];
        for _ in 0..5 {
            m.clicked_skip(MediaControl::Next, false);
            m.step(16.0, now);
            total.push(m.skip[1].value);
        }
        assert!(
            m.skip[1].target <= m.skip[1].value.floor() + 3.0,
            "a burst never spins on"
        );
        while m.step(16.0, now) {
            total.push(m.skip[1].value);
            assert!(total.len() < 400, "settles");
        }
        assert!(
            total.windows(2).all(|w| w[1] >= w[0] - 1e-4 || w[1] == 0.0),
            "one way"
        );
        assert!(
            total
                .windows(2)
                .all(|w| w[1] == 0.0 || (w[1] - w[0]).abs() < 0.5),
            "continuous"
        );
        assert_eq!(m.skip_phase(MediaControl::Next), 0.0, "at rest again");
        assert!(!m.is_animating(), "no frames once settled");
    }

    #[test]
    fn test_reduced_motion_glyphs_change_at_once() {
        let now = std::time::Instant::now();
        let mut m = IconMotion::default();
        m.sync(PlayPauseIcon::Play, now, true);
        m.clicked_play_pause(now, true);
        assert_eq!(m.morph(), 1.0);
        m.clicked_skip(MediaControl::Previous, true);
        assert_eq!(m.skip_phase(MediaControl::Previous), 0.0);
        m.sync(PlayPauseIcon::Pause, now, true);
        assert!(!m.is_animating());
        // Settling (nothing on screen) lands on the reported glyph
        let mut m = IconMotion::default();
        m.sync(PlayPauseIcon::Pause, now, false);
        m.clicked_skip(MediaControl::Next, false);
        m.settle();
        assert_eq!(m.morph(), 1.0);
        assert!(!m.is_animating());
    }
}
