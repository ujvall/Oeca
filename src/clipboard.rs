//! Clipboard history foundation (Phase 5.0): Oeca listens for system clipboard
//! changes (`AddClipboardFormatListener` / `WM_CLIPBOARDUPDATE`, no polling) and
//! keeps a small in-memory history of Unicode text and images that later phases
//! can display and restore.
//!
//! Privacy: history lives in memory only. Nothing is written to disk, logs,
//! the network or debug output; clipboard contents are never printed.
//!
//! Ownership: every item is copied into Oeca-owned memory while the clipboard is
//! open; no clipboard handle or pointer outlives `CloseClipboard`. Images reuse
//! the media artwork representation (owned BGRA8 premultiplied + fingerprint),
//! but each item owns its own pixels, independent of the media session.

use std::collections::VecDeque;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData,
    GetClipboardOwner, IsClipboardFormatAvailable, OpenClipboard, RemoveClipboardFormatListener,
    SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};

use crate::config::{CLIPBOARD_MAX_IMAGE_BYTES, CLIPBOARD_MAX_TEXT_UNITS, ClipboardCapacity};
use crate::media::Artwork;

/// Standard clipboard formats (winuser.h).
const CF_UNICODETEXT: u32 = 13;
const CF_DIB: u32 = 8;

/// One clipboard history entry, fully owned by Oeca.
#[derive(Clone, PartialEq, Eq)]
pub enum ClipboardItem {
    Text(String),
    /// BGRA8 premultiplied, tightly packed; equality uses dimensions + pixel
    /// fingerprint (computed once, when the image is captured).
    Image(Artwork),
}

/// Never prints contents: kind and size only.
impl std::fmt::Debug for ClipboardItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(t) => write!(f, "Text({} chars)", t.chars().count()),
            Self::Image(a) => write!(f, "Image({}x{})", a.width, a.height),
        }
    }
}

impl ClipboardItem {
    /// Owned image memory of this item (0 for text).
    fn image_bytes(&self) -> usize {
        match self {
            Self::Image(a) => a.pixels.len(),
            Self::Text(_) => 0,
        }
    }
}

/// One-line display preview of `text` (display only: the entry keeps its exact
/// text). Runs of whitespace and control characters, newlines included, become
/// one space; at most `max` characters, ending in an ellipsis when cut.
pub fn preview(text: &str, max: usize) -> String {
    let mut out = String::new();
    let (mut count, mut gap) = (0, false);
    for ch in text.chars() {
        if ch.is_whitespace() || ch.is_control() {
            gap = count > 0;
            continue;
        }
        if count + usize::from(gap) >= max {
            out.push('\u{2026}');
            break;
        }
        if gap {
            out.push(' ');
            count += 1;
            gap = false;
        }
        out.push(ch);
        count += 1;
    }
    out
}

/// Why an item was not stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    Empty,
    TextTooLong,
    ImageTooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddOutcome {
    Added,
    /// Identical to the newest entry: nothing stored.
    Duplicate,
    Rejected(Rejected),
}

/// Recent clipboard items, newest first, bounded by entry count (the chosen
/// capacity) and total image memory (oldest evicted first). Owned by
/// `WindowState`.
#[derive(Debug, Default)]
pub struct ClipboardHistory {
    items: VecDeque<ClipboardItem>,
    capacity: ClipboardCapacity,
}

impl ClipboardHistory {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Newest first.
    pub fn items(&self) -> impl Iterator<Item = &ClipboardItem> {
        self.items.iter()
    }

    pub fn get(&self, index: usize) -> Option<&ClipboardItem> {
        self.items.get(index)
    }

    pub fn image_bytes(&self) -> usize {
        self.items.iter().map(ClipboardItem::image_bytes).sum()
    }

    /// Stores a newly copied item at the front.
    pub fn add(&mut self, item: ClipboardItem) -> AddOutcome {
        if let Err(why) = check_limits(&item) {
            return AddOutcome::Rejected(why);
        }
        if self.items.front() == Some(&item) {
            return AddOutcome::Duplicate;
        }
        self.items.push_front(item);
        self.trim();
        AddOutcome::Added
    }

    /// Drops the oldest entries until both limits hold. Image memory is
    /// summed from the kept items, so it is always exact.
    fn trim(&mut self) {
        while self.items.len() > self.capacity.entries()
            || self.image_bytes() > CLIPBOARD_MAX_IMAGE_BYTES
        {
            self.items.pop_back();
        }
    }

    /// Uses a new capacity: a smaller one drops the oldest entries (the
    /// newest keep their order); a larger one keeps everything. True if
    /// entries were dropped. The system clipboard is untouched.
    pub fn set_capacity(&mut self, capacity: ClipboardCapacity) -> bool {
        let before = self.items.len();
        self.capacity = capacity;
        self.trim();
        self.items.len() != before
    }

    /// Forgets one entry (its row's trash button).
    pub fn remove(&mut self, index: usize) -> Option<ClipboardItem> {
        self.items.remove(index)
    }

    /// Forgets every entry (and its image memory). The system clipboard is
    /// untouched.
    pub fn clear(&mut self) {
        self.items = VecDeque::new();
    }

    /// Restoring an entry makes it the newest (it is what the clipboard now
    /// holds), without duplicating it. Returns the entry to place on the system
    /// clipboard.
    pub fn promote(&mut self, index: usize) -> Option<&ClipboardItem> {
        let item = self.items.remove(index)?;
        self.items.push_front(item);
        self.items.front()
    }
}

fn check_limits(item: &ClipboardItem) -> Result<(), Rejected> {
    match item {
        ClipboardItem::Text(t) if t.is_empty() => Err(Rejected::Empty),
        ClipboardItem::Text(t) if t.encode_utf16().count() > CLIPBOARD_MAX_TEXT_UNITS => {
            Err(Rejected::TextTooLong)
        }
        ClipboardItem::Image(a) if a.pixels.len() > CLIPBOARD_MAX_IMAGE_BYTES => {
            Err(Rejected::ImageTooLarge)
        }
        _ => Ok(()),
    }
}

// ---- Native clipboard ---------------------------------------------------------

/// Why a clipboard read/write did not happen. Every case is non-fatal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardError {
    /// Another application has the clipboard open (no retry, no spinning).
    Unavailable,
    /// Nothing Oeca stores (no text or bitmap).
    Unsupported,
    /// Content over the configured limits (rejected before copying).
    TooLarge,
    /// Malformed or unsupported data.
    Invalid,
    /// Allocation or `SetClipboardData` failed.
    WriteFailed,
}

/// Starts clipboard change notifications (`WM_CLIPBOARDUPDATE`) for `hwnd`.
pub fn start_listening(hwnd: HWND) -> bool {
    unsafe { AddClipboardFormatListener(hwnd).is_ok() }
}

pub fn stop_listening(hwnd: HWND) {
    unsafe {
        let _ = RemoveClipboardFormatListener(hwnd);
    }
}

/// True when the clipboard's current content was put there by `hwnd` (Oeca
/// restoring a history item), so the resulting update is not a new copy.
pub fn owned_by(hwnd: HWND) -> bool {
    unsafe { GetClipboardOwner() }.is_ok_and(|owner| owner == hwnd)
}

/// Closes the clipboard when dropped (every early return included).
struct OpenGuard;

impl OpenGuard {
    fn open(owner: Option<HWND>) -> Result<Self, ClipboardError> {
        unsafe { OpenClipboard(owner) }
            .map(|()| Self)
            .map_err(|_| ClipboardError::Unavailable)
    }
}

impl Drop for OpenGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

/// Borrowed view of a clipboard memory block while it is locked.
fn with_locked<R>(handle: HANDLE, f: impl FnOnce(&[u8]) -> R) -> Result<R, ClipboardError> {
    let hglobal = HGLOBAL(handle.0);
    unsafe {
        let ptr = GlobalLock(hglobal) as *const u8;
        if ptr.is_null() {
            return Err(ClipboardError::Invalid);
        }
        let bytes = std::slice::from_raw_parts(ptr, GlobalSize(hglobal));
        let result = f(bytes);
        let _ = GlobalUnlock(hglobal);
        Ok(result)
    }
}

/// Reads the current clipboard into an owned item: Unicode text if present,
/// otherwise a bitmap (CF_DIB, which Windows synthesizes from the other bitmap
/// formats). Text that is blank or unreadable never hides an image offered
/// alongside it (`text_wins`). The clipboard is closed before this returns.
pub fn read(owner: HWND) -> Result<ClipboardItem, ClipboardError> {
    let _open = OpenGuard::open(Some(owner))?;
    unsafe {
        let has_image = IsClipboardFormatAvailable(CF_DIB).is_ok();
        if IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok() {
            let handle = GetClipboardData(CF_UNICODETEXT).map_err(|_| ClipboardError::Invalid)?;
            let text = with_locked(handle, |bytes| {
                let units: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c))
                    .take_while(|&u| u != 0)
                    .take(CLIPBOARD_MAX_TEXT_UNITS + 1)
                    .collect();
                text_from_utf16(&units)
            })?;
            if text_wins(&text, has_image) {
                return text.map(ClipboardItem::Text);
            }
        }
        if has_image {
            let handle = GetClipboardData(CF_DIB).map_err(|_| ClipboardError::Invalid)?;
            return with_locked(handle, dib_to_artwork)?.map(ClipboardItem::Image);
        }
    }
    Err(ClipboardError::Unsupported)
}

/// Places an item on the system clipboard (Oeca becomes the owner). On success
/// the memory handle belongs to the system and is not freed here; on failure
/// it is freed. The clipboard is closed before this returns.
#[allow(dead_code)]
pub fn write(owner: HWND, item: &ClipboardItem) -> Result<(), ClipboardError> {
    let (format, bytes) = match item {
        ClipboardItem::Text(t) => {
            let mut bytes: Vec<u8> = t.encode_utf16().flat_map(u16::to_le_bytes).collect();
            bytes.extend([0, 0]);
            (CF_UNICODETEXT, bytes)
        }
        ClipboardItem::Image(a) => (CF_DIB, artwork_to_dib(a)),
    };
    let _open = OpenGuard::open(Some(owner))?;
    unsafe {
        EmptyClipboard().map_err(|_| ClipboardError::WriteFailed)?;
        let hglobal =
            GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|_| ClipboardError::WriteFailed)?;
        let dst = GlobalLock(hglobal) as *mut u8;
        if dst.is_null() {
            let _ = GlobalFree(Some(hglobal));
            return Err(ClipboardError::WriteFailed);
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
        let _ = GlobalUnlock(hglobal);
        if SetClipboardData(format, Some(HANDLE(hglobal.0))).is_err() {
            // Ownership was not transferred: the memory is still ours
            let _ = GlobalFree(Some(hglobal));
            return Err(ClipboardError::WriteFailed);
        }
    }
    Ok(())
}

// ---- Conversions (pure, testable) -------------------------------------------

/// Owned text from UTF-16 units (already cut at the terminator). Rejects empty
/// or over-limit text without keeping it.
fn text_from_utf16(units: &[u16]) -> Result<String, ClipboardError> {
    if units.is_empty() {
        return Err(ClipboardError::Unsupported);
    }
    if units.len() > CLIPBOARD_MAX_TEXT_UNITS {
        return Err(ClipboardError::TooLarge);
    }
    Ok(String::from_utf16_lossy(units))
}

/// Whether the clipboard's text (decoded, or why not) is the entry rather than
/// its bitmap: always without a bitmap; with one, only readable, non-blank
/// text. Apps that copy a picture often add blank or placeholder text (a
/// space, U+FFFC), which must not replace the image with an empty tile.
fn text_wins(text: &Result<String, ClipboardError>, has_image: bool) -> bool {
    !has_image || text.as_deref().is_ok_and(|t| !is_blank(t))
}

/// True if `text` would preview as nothing: only whitespace, control
/// characters or object replacement placeholders (U+FFFC).
fn is_blank(text: &str) -> bool {
    text.chars()
        .all(|c| c.is_whitespace() || c.is_control() || c == '\u{FFFC}')
}

/// Whether a 32 bpp DIB's fourth byte is real alpha. In plain BI_RGB DIBs that
/// byte is formally reserved, and screen captures (Print Screen, the CF_DIB
/// synthesized from a screen bitmap) fill it with whatever the desktop surface
/// held: 255 under some windows, 0 under others. Honouring that made most of a
/// screenshot transparent (a partly black tile). Real alpha never keeps colour
/// under a fully transparent pixel (premultiplied sources cannot; Oeca's own
/// DIB writes 0 there), so any `a == 0` pixel with colour, or no alpha at all,
/// means opaque.
fn alpha_is_meaningful(pixels: impl Iterator<Item = [u8; 4]>) -> bool {
    let mut any_alpha = false;
    for [b, g, r, a] in pixels {
        if a == 0 && (b | g | r) != 0 {
            return false;
        }
        any_alpha |= a != 0;
    }
    any_alpha
}

const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;

/// Packed DIB (BITMAPINFOHEADER or later + optional masks/palette + pixels) to
/// owned premultiplied BGRA. Supports 24 and 32 bpp, bottom-up and top-down;
/// 32 bpp is opaque unless its alpha is meaningful (`alpha_is_meaningful`).
/// The size limit is checked before any pixel is copied.
fn dib_to_artwork(dib: &[u8]) -> Result<Artwork, ClipboardError> {
    // The whole fixed header must be present before any field is read
    if dib.len() < 40 {
        return Err(ClipboardError::Invalid);
    }
    let u32_at = |o: usize| {
        dib.get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or(ClipboardError::Invalid)
    };
    let header_size = u32_at(0)? as usize;
    if header_size < 40 {
        return Err(ClipboardError::Invalid);
    }
    let width = u32_at(4)? as i32;
    let height = u32_at(8)? as i32;
    let bit_count = u16::from_le_bytes([dib[14], dib[15]]);
    let compression = u32_at(16)?;
    let colors_used = u32_at(32)? as usize;
    if width <= 0 || height == 0 || !(bit_count == 24 || bit_count == 32) {
        return Err(ClipboardError::Invalid);
    }
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    let out_bytes = w
        .checked_mul(h)
        .and_then(|n| n.checked_mul(4))
        .ok_or(ClipboardError::TooLarge)?;
    if out_bytes > CLIPBOARD_MAX_IMAGE_BYTES {
        return Err(ClipboardError::TooLarge);
    }
    match (compression, bit_count) {
        (BI_RGB, _) => {}
        (BI_BITFIELDS, 32) => {
            // Masks follow a 40-byte header, or live inside V4/V5 headers
            let masks_at = 40;
            let (r, g, b) = (
                u32_at(masks_at)?,
                u32_at(masks_at + 4)?,
                u32_at(masks_at + 8)?,
            );
            if (r, g, b) != (0x00FF_0000, 0x0000_FF00, 0x0000_00FF) {
                return Err(ClipboardError::Invalid);
            }
        }
        _ => return Err(ClipboardError::Invalid),
    }
    let masks = if compression == BI_BITFIELDS && header_size == 40 {
        12
    } else {
        0
    };
    let offset = header_size + masks + colors_used * 4;
    let bpp = bit_count as usize / 8;
    let stride = (w * bit_count as usize).div_ceil(32) * 4;
    let needed = stride.checked_mul(h).and_then(|n| n.checked_add(offset));
    if needed.is_none_or(|n| n > dib.len()) {
        return Err(ClipboardError::Invalid);
    }
    let rows = &dib[offset..];
    let has_alpha = bit_count == 32
        && alpha_is_meaningful((0..h).flat_map(|y| {
            rows[y * stride..][..w * 4]
                .as_chunks::<4>()
                .0
                .iter()
                .copied()
        }));
    let mut out = vec![0u8; out_bytes];
    for y in 0..h {
        // Bottom-up DIBs store the last row first
        let src_row = if height > 0 { h - 1 - y } else { y };
        let src = &rows[src_row * stride..src_row * stride + w * bpp];
        for x in 0..w {
            let p = &src[x * bpp..x * bpp + bpp];
            let a = if has_alpha { p[3] } else { 255 };
            let pm = |c: u8| ((u32::from(c) * u32::from(a) + 127) / 255) as u8;
            out[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[
                pm(p[0]),
                pm(p[1]),
                pm(p[2]),
                a,
            ]);
        }
    }
    Artwork::new(w as u32, h as u32, out).ok_or(ClipboardError::Invalid)
}

/// Owned image to a packed 32 bpp bottom-up CF_DIB (straight alpha, as DIB
/// consumers expect).
pub(crate) fn artwork_to_dib(a: &Artwork) -> Vec<u8> {
    let (w, h) = (a.width as usize, a.height as usize);
    let mut dib = Vec::with_capacity(40 + w * h * 4);
    for v in [40u32, a.width, a.height] {
        dib.extend(v.to_le_bytes());
    }
    dib.extend(1u16.to_le_bytes()); // planes
    dib.extend(32u16.to_le_bytes()); // bit count
    dib.extend(BI_RGB.to_le_bytes());
    dib.extend(((w * h * 4) as u32).to_le_bytes()); // image size
    dib.extend([0u8; 16]); // resolution, colours used/important
    for y in (0..h).rev() {
        for x in 0..w {
            let p = &a.pixels[(y * w + x) * 4..(y * w + x) * 4 + 4];
            let alpha = p[3];
            let un = |c: u8| {
                if alpha == 0 {
                    0
                } else {
                    ((u32::from(c) * 255 + u32::from(alpha) / 2) / u32::from(alpha)).min(255) as u8
                }
            };
            dib.extend([un(p[0]), un(p[1]), un(p[2]), alpha]);
        }
    }
    dib
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CLIPBOARD_MAX_ENTRIES, ClipboardCapacity};

    fn text(s: &str) -> ClipboardItem {
        ClipboardItem::Text(s.to_string())
    }

    /// Opaque w x h image filled with one colour.
    fn image(w: u32, h: u32, shade: u8) -> ClipboardItem {
        let px: Vec<u8> = (0..w * h)
            .flat_map(|_| [shade, shade / 2, 255 - shade, 255])
            .collect();
        ClipboardItem::Image(Artwork::new(w, h, px).unwrap())
    }

    #[test]
    fn test_preview_normalizes_whitespace_and_newlines() {
        assert_eq!(preview("  hello\r\n\n\tworld  \n", 50), "hello world");
        assert_eq!(preview("a\u{0}b\u{7}\u{1b}c", 50), "a b c");
        assert_eq!(preview("\n\n\t ", 50), "");
        assert_eq!(preview("line one\nline two", 50), "line one line two");
    }

    #[test]
    fn test_preview_truncates_long_text() {
        let long = "word ".repeat(100_000);
        let p = preview(&long, 160);
        assert!(p.ends_with('\u{2026}'));
        assert!(p.chars().count() <= 161, "bounded: max + ellipsis");
        assert!(!p.contains('\n'));
        assert_eq!(preview("short", 160), "short", "no ellipsis when it fits");
        assert_eq!(preview("abcdef", 3), "abc\u{2026}");
    }

    #[test]
    fn test_remove_one_entry_keeps_order() {
        let mut h = ClipboardHistory::default();
        for t in ["a", "b", "c"] {
            h.add(text(t));
        }
        assert_eq!(h.remove(1), Some(text("b")));
        assert_eq!(
            h.items().cloned().collect::<Vec<_>>(),
            [text("c"), text("a")]
        );
        assert_eq!(h.remove(5), None, "out of range is a no-op");
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn test_clear_releases_history_only() {
        let mut h = ClipboardHistory::default();
        h.add(text("a"));
        h.add(image(64, 64, 1));
        assert!(h.image_bytes() > 0);
        h.clear();
        assert!(h.is_empty());
        assert_eq!(h.image_bytes(), 0, "image memory released");
        assert_eq!(h.get(0), None);
        // Still a valid history afterwards
        assert_eq!(h.add(text("b")), AddOutcome::Added);
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn test_empty_history() {
        let h = ClipboardHistory::default();
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
        assert_eq!(h.image_bytes(), 0);
        assert!(h.get(0).is_none());
    }

    #[test]
    fn test_add_text_and_image_newest_first() {
        let mut h = ClipboardHistory::default();
        assert_eq!(h.add(text("first")), AddOutcome::Added);
        assert_eq!(h.add(image(4, 4, 10)), AddOutcome::Added);
        assert_eq!(h.add(text("third")), AddOutcome::Added);
        let kinds: Vec<String> = h.items().map(|i| format!("{i:?}")).collect();
        assert_eq!(kinds, ["Text(5 chars)", "Image(4x4)", "Text(5 chars)"]);
        assert_eq!(h.get(0), Some(&text("third")));
        assert_eq!(h.get(2), Some(&text("first")));
        assert_eq!(h.image_bytes(), 4 * 4 * 4);
    }

    #[test]
    fn test_max_entries_evicts_oldest() {
        let mut h = ClipboardHistory::default();
        for i in 0..CLIPBOARD_MAX_ENTRIES + 5 {
            h.add(text(&format!("item {i}")));
        }
        assert_eq!(h.len(), CLIPBOARD_MAX_ENTRIES);
        assert_eq!(
            h.get(0),
            Some(&text(&format!("item {}", CLIPBOARD_MAX_ENTRIES + 4)))
        );
        assert_eq!(
            h.get(CLIPBOARD_MAX_ENTRIES - 1),
            Some(&text("item 5")),
            "oldest gone"
        );
    }

    #[test]
    fn test_image_memory_budget_evicts_oldest() {
        let mut h = ClipboardHistory::default();
        // 1000 x 1000 x 4 = 4 MB each: the third pushes past 10 MB
        h.add(text("keep me?"));
        for shade in [1, 2, 3] {
            assert_eq!(h.add(image(1000, 1000, shade)), AddOutcome::Added);
        }
        assert!(h.image_bytes() <= CLIPBOARD_MAX_IMAGE_BYTES);
        assert_eq!(h.image_bytes(), 8_000_000, "two images remain");
        assert_eq!(h.get(0), Some(&image(1000, 1000, 3)));
        assert_eq!(h.get(1), Some(&image(1000, 1000, 2)));
        assert_eq!(h.len(), 2, "older entries evicted oldest-first");
    }

    #[test]
    fn test_oversized_items_rejected() {
        let mut h = ClipboardHistory::default();
        // 1700 x 1700 x 4 = 11.56 MB > 10 MiB
        assert_eq!(
            h.add(image(1700, 1700, 7)),
            AddOutcome::Rejected(Rejected::ImageTooLarge)
        );
        let long = "x".repeat(CLIPBOARD_MAX_TEXT_UNITS + 1);
        assert_eq!(
            h.add(text(&long)),
            AddOutcome::Rejected(Rejected::TextTooLong)
        );
        assert_eq!(h.add(text("")), AddOutcome::Rejected(Rejected::Empty));
        assert!(h.is_empty(), "nothing stored");
        let fits = "x".repeat(CLIPBOARD_MAX_TEXT_UNITS);
        assert_eq!(h.add(text(&fits)), AddOutcome::Added);
    }

    #[test]
    fn test_consecutive_duplicates_and_distinct_items() {
        let mut h = ClipboardHistory::default();
        assert_eq!(h.add(text("same")), AddOutcome::Added);
        assert_eq!(h.add(text("same")), AddOutcome::Duplicate);
        assert_eq!(h.add(text("other")), AddOutcome::Added);
        assert_eq!(h.add(image(8, 8, 50)), AddOutcome::Added);
        assert_eq!(h.add(image(8, 8, 50)), AddOutcome::Duplicate, "same pixels");
        assert_eq!(
            h.add(image(8, 8, 51)),
            AddOutcome::Added,
            "different pixels"
        );
        assert_eq!(
            h.add(image(4, 16, 51)),
            AddOutcome::Added,
            "different shape"
        );
        // Only *consecutive* duplicates are suppressed
        assert_eq!(h.add(text("same")), AddOutcome::Added);
        assert_eq!(h.len(), 6);
    }

    #[test]
    fn test_items_are_owned_and_survive_source_changes() {
        let mut h = ClipboardHistory::default();
        let mut source = String::from("copied text");
        h.add(ClipboardItem::Text(source.clone()));
        source.clear(); // the "other application" changes its buffer
        let mut pixels = vec![9u8; 2 * 2 * 4];
        h.add(ClipboardItem::Image(
            Artwork::new(2, 2, pixels.clone()).unwrap(),
        ));
        pixels.fill(0);
        assert_eq!(h.get(1), Some(&text("copied text")));
        let ClipboardItem::Image(a) = h.get(0).unwrap() else {
            panic!("image")
        };
        assert!(a.pixels.iter().all(|&b| b == 9), "owned copy unchanged");
    }

    #[test]
    fn test_restore_promotes_without_duplicating() {
        let mut h = ClipboardHistory::default();
        for t in ["a", "b", "c"] {
            h.add(text(t));
        }
        // Restore "a" (oldest): it becomes the newest, still one copy
        assert_eq!(h.promote(2), Some(&text("a")));
        assert_eq!(h.len(), 3);
        assert_eq!(h.get(0), Some(&text("a")));
        // The clipboard update Oeca's own restore triggers is a no-op
        assert_eq!(h.add(text("a")), AddOutcome::Duplicate);
        assert_eq!(h.len(), 3);
        assert!(h.promote(9).is_none());
    }

    #[test]
    fn test_debug_output_never_contains_contents() {
        let secret = "hunter2-password";
        let shown = format!("{:?}", text(secret));
        assert!(!shown.contains(secret));
        let mut h = ClipboardHistory::default();
        h.add(text(secret));
        assert!(!format!("{h:?}").contains(secret));
    }

    #[test]
    fn test_text_conversion_and_limits() {
        let units: Vec<u16> = "héllo 👋".encode_utf16().collect();
        assert_eq!(text_from_utf16(&units), Ok("héllo 👋".to_string()));
        assert_eq!(text_from_utf16(&[]), Err(ClipboardError::Unsupported));
        let too_long = vec![b'x' as u16; CLIPBOARD_MAX_TEXT_UNITS + 1];
        assert_eq!(text_from_utf16(&too_long), Err(ClipboardError::TooLarge));
    }

    /// Minimal 24 bpp bottom-up DIB: 2x2, rows padded to 4 bytes.
    fn dib24() -> Vec<u8> {
        let mut d = Vec::new();
        for v in [40u32, 2, 2] {
            d.extend(v.to_le_bytes());
        }
        d.extend(1u16.to_le_bytes());
        d.extend(24u16.to_le_bytes());
        d.extend([0u8; 24]);
        // bottom row: blue, green; top row: red, white
        d.extend([255, 0, 0, 0, 255, 0, 0, 0]);
        d.extend([0, 0, 255, 255, 255, 255, 0, 0]);
        d
    }

    #[test]
    fn test_dib_decode_orientation_and_alpha() {
        let a = dib_to_artwork(&dib24()).unwrap();
        assert_eq!((a.width, a.height), (2, 2));
        // Top-left is the red pixel (bottom-up flipped), opaque
        assert_eq!(&a.pixels[0..4], &[0, 0, 255, 255]);
        assert_eq!(&a.pixels[12..16], &[0, 255, 0, 255], "bottom-right green");
        // Round trip through Oeca's own CF_DIB keeps the pixels
        let back = dib_to_artwork(&artwork_to_dib(&a)).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn test_dib_alpha_round_trip_and_malformed_input() {
        // Half-transparent pixel survives premultiply -> DIB -> premultiply
        let a = Artwork::new(1, 1, vec![64, 32, 16, 128]).unwrap();
        let back = dib_to_artwork(&artwork_to_dib(&a)).unwrap();
        for (x, y) in back.pixels.iter().zip(a.pixels.iter()) {
            assert!((*x as i32 - *y as i32).abs() <= 1);
        }
        assert_eq!(dib_to_artwork(&[]), Err(ClipboardError::Invalid));
        assert_eq!(
            dib_to_artwork(&dib24()[..50]),
            Err(ClipboardError::Invalid),
            "truncated"
        );
        let mut bad_bpp = dib24();
        bad_bpp[14] = 8;
        assert_eq!(dib_to_artwork(&bad_bpp), Err(ClipboardError::Invalid));
        // Huge declared size is rejected before allocating
        let mut huge = dib24();
        huge[4..8].copy_from_slice(&5000u32.to_le_bytes());
        huge[8..12].copy_from_slice(&5000u32.to_le_bytes());
        assert_eq!(dib_to_artwork(&huge), Err(ClipboardError::TooLarge));
    }

    /// Packed 32 bpp BI_RGB top-down DIB (40-byte header) from BGRA rows.
    fn dib32(w: u32, h: u32, px: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut d = Vec::new();
        for v in [40u32, w, (-(h as i32)) as u32] {
            d.extend(v.to_le_bytes());
        }
        d.extend(1u16.to_le_bytes());
        d.extend(32u16.to_le_bytes());
        d.extend([0u8; 24]);
        for y in 0..h {
            for x in 0..w {
                d.extend(px(x, y));
            }
        }
        d
    }

    /// A screen capture's CF_DIB: the reserved byte is 255 under one "window"
    /// (left half) and 0 over the rest of the desktop, colour everywhere.
    fn screenshot_dib(w: u32, h: u32, bgr: [u8; 3]) -> Vec<u8> {
        dib32(w, h, |x, _| {
            [bgr[0], bgr[1], bgr[2], if x < w / 2 { 255 } else { 0 }]
        })
    }

    #[test]
    fn test_screen_capture_reserved_byte_decodes_opaque() {
        let a = dib_to_artwork(&screenshot_dib(8, 4, [10, 120, 230])).unwrap();
        assert_eq!((a.width, a.height), (8, 4));
        for p in a.pixels.as_chunks::<4>().0 {
            assert_eq!(p, &[10, 120, 230, 255], "every pixel kept, fully opaque");
        }
        // All-zero reserved bytes (the usual BI_RGB case) are opaque too
        let a = dib_to_artwork(&dib32(2, 2, |_, _| [1, 2, 3, 0])).unwrap();
        assert!(
            a.pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p == &[1, 2, 3, 255])
        );
        // Real alpha (nothing hidden under a == 0) is still honoured, premultiplied
        let a = dib_to_artwork(&dib32(2, 1, |x, _| {
            if x == 0 {
                [0, 0, 0, 0]
            } else {
                [200, 100, 50, 128]
            }
        }))
        .unwrap();
        assert_eq!(&a.pixels[0..4], &[0, 0, 0, 0]);
        assert_eq!(&a.pixels[4..8], &[100, 50, 25, 128]);
    }

    #[test]
    fn test_truncated_header_is_rejected_without_panicking() {
        let full = dib24();
        for len in 0..40 {
            assert_eq!(
                dib_to_artwork(&full[..len]),
                Err(ClipboardError::Invalid),
                "{len}"
            );
        }
    }

    #[test]
    fn test_format_selection_keeps_images_next_to_blank_text() {
        let ok = |t: &str| Ok(t.to_string());
        // No bitmap: text is the entry, even blank (unchanged behaviour)
        assert!(text_wins(&ok("hello"), false));
        assert!(text_wins(&ok(" \r\n"), false));
        assert!(text_wins(&Err(ClipboardError::TooLarge), false));
        // Bitmap + real text: text wins (e.g. spreadsheet cells)
        assert!(text_wins(&ok("A1\tB1"), true));
        assert!(text_wins(&ok("  x  "), true));
        // Bitmap + blank, placeholder or unreadable text: the image is kept
        for blank in [" ", "\r\n", "\t", "\u{FFFC}", " \u{FFFC}\r\n", "\u{0}"] {
            assert!(!text_wins(&ok(blank), true), "{blank:?}");
        }
        assert!(!text_wins(&Err(ClipboardError::Unsupported), true));
        assert!(!text_wins(&Err(ClipboardError::TooLarge), true));
    }

    #[test]
    fn test_mixed_screenshot_and_text_history_keeps_each_image() {
        // Screenshots decoded as they arrive, interleaved with text and a
        // repeated text; each image entry keeps its own full, opaque pixels
        let mut h = ClipboardHistory::default();
        let shades: [[u8; 3]; 4] = [[200, 0, 0], [0, 200, 0], [0, 0, 200], [90, 90, 90]];
        for (i, bgr) in shades.iter().enumerate() {
            let shot = dib_to_artwork(&screenshot_dib(40 + i as u32, 30, *bgr)).unwrap();
            assert_eq!(h.add(ClipboardItem::Image(shot)), AddOutcome::Added);
            assert_eq!(h.add(text(&format!("note {i}"))), AddOutcome::Added);
        }
        assert_eq!(h.len(), 8);
        // Newest first: note 3, shot 3, note 2, shot 2, ...
        for (i, item) in h.items().enumerate() {
            let k = 3 - i / 2;
            match (i % 2, item) {
                (0, ClipboardItem::Text(t)) => assert_eq!(t, &format!("note {k}")),
                (1, ClipboardItem::Image(a)) => {
                    assert_eq!((a.width, a.height), (40 + k as u32, 30));
                    let [b, g, r] = shades[k];
                    assert!(
                        a.pixels
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .all(|p| p == &[b, g, r, 255])
                    );
                }
                other => panic!("entry {i} out of order: {other:?}"),
            }
        }
        // Removing one image shifts nothing else
        assert!(matches!(h.remove(1), Some(ClipboardItem::Image(_))));
        assert_eq!(h.get(1), Some(&text("note 2")));
        assert!(matches!(h.get(2), Some(ClipboardItem::Image(a)) if a.width == 42));
    }

    /// Real clipboard (run explicitly: `cargo test clipboard_live -- --ignored`).
    /// Overwrites the user's clipboard.
    #[test]
    #[ignore]
    fn clipboard_live_write_read_and_busy_failure() {
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
        };
        // A message-only window owns the clipboard (SetClipboardData needs one)
        let owner = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!(""),
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
        .expect("owner window");
        let item = text("oeca clipboard test");
        write(owner, &item).expect("write");
        assert!(owned_by(owner), "Oeca's own write is recognisable");
        assert_eq!(read(owner), Ok(item));
        let img = image(3, 2, 77);
        write(owner, &img).expect("write image");
        assert_eq!(read(owner), Ok(img.clone()));
        // Clearing the history leaves the system clipboard as it was
        let mut history = ClipboardHistory::default();
        history.add(img.clone());
        history.clear();
        assert_eq!(read(owner), Ok(img));
        // Another thread holds the clipboard open: read/write fail cleanly
        let (tx, rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let holder = std::thread::spawn(move || {
            let _g = OpenGuard::open(None).expect("open");
            tx.send(()).unwrap();
            let _ = done_rx.recv();
        });
        rx.recv().unwrap();
        assert_eq!(read(owner), Err(ClipboardError::Unavailable));
        assert_eq!(write(owner, &text("x")), Err(ClipboardError::Unavailable));
        done_tx.send(()).unwrap();
        holder.join().unwrap();
        unsafe {
            let _ = DestroyWindow(owner);
        }
    }

    #[test]
    fn test_capacity_trims_oldest_keeps_order_and_accounts_images() {
        use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
        let system_clipboard = unsafe { GetClipboardSequenceNumber() };
        // 20 entries (the default): every third an image, newest first
        let mut h = ClipboardHistory::default();
        let item = |i: usize| {
            if i.is_multiple_of(3) {
                image(8, 8, i as u8)
            } else {
                text(&format!("item {i}"))
            }
        };
        for i in 0..CLIPBOARD_MAX_ENTRIES {
            assert_eq!(h.add(item(i)), AddOutcome::Added);
        }
        assert_eq!(h.len(), 20);
        let all: Vec<ClipboardItem> = h.items().cloned().collect();
        let bytes = |items: &[ClipboardItem]| items.iter().map(ClipboardItem::image_bytes).sum();
        assert_eq!(h.image_bytes(), bytes(&all));
        // 5: the 5 newest stay, in order; image memory is theirs only
        assert!(h.set_capacity(ClipboardCapacity::Five));
        let kept: Vec<ClipboardItem> = h.items().cloned().collect();
        assert_eq!(kept, all[..5]);
        assert_eq!(h.image_bytes(), bytes(&all[..5]));
        assert!(h.image_bytes() < bytes(&all));
        // New entries respect the smaller limit; duplicates still skipped
        assert_eq!(h.add(text("new")), AddOutcome::Added);
        assert_eq!(h.add(text("new")), AddOutcome::Duplicate);
        assert_eq!(h.len(), 5);
        assert_eq!(h.get(0), Some(&text("new")));
        assert_eq!(h.get(4), Some(&all[3]), "oldest dropped");
        // Larger: nothing removed, later entries fill up to the new limit
        assert!(!h.set_capacity(ClipboardCapacity::Twenty));
        assert_eq!(h.len(), 5);
        for i in 100..120 {
            h.add(text(&format!("more {i}")));
        }
        assert_eq!(h.len(), 20);
        assert!(h.set_capacity(ClipboardCapacity::Ten));
        assert_eq!(h.len(), 10);
        assert_eq!(h.get(0), Some(&text("more 119")));
        assert_eq!(h.image_bytes(), 0, "the images were the oldest");
        // Same capacity again: no change
        assert!(!h.set_capacity(ClipboardCapacity::Ten));
        // The system clipboard was never written
        assert_eq!(unsafe { GetClipboardSequenceNumber() }, system_clipboard);
    }

    #[test]
    fn test_capacity_keeps_the_other_limits() {
        // Options, default and the unchanged item limits
        assert_eq!(ClipboardCapacity::default(), ClipboardCapacity::Twenty);
        assert_eq!(
            ClipboardCapacity::ALL.map(ClipboardCapacity::entries),
            [5, 10, 20]
        );
        assert_eq!(
            ClipboardCapacity::ALL.map(ClipboardCapacity::name),
            ["5 items", "10 items", "20 items"]
        );
        for (i, c) in ClipboardCapacity::ALL.into_iter().enumerate() {
            assert_eq!(c.index(), i);
            assert_eq!(ClipboardCapacity::from_index(i), Some(c));
        }
        assert_eq!(ClipboardCapacity::from_index(3), None);
        let mut h = ClipboardHistory::default();
        h.set_capacity(ClipboardCapacity::Five);
        assert_eq!(
            h.add(text(&"x".repeat(CLIPBOARD_MAX_TEXT_UNITS + 1))),
            AddOutcome::Rejected(Rejected::TextTooLong)
        );
        assert_eq!(CLIPBOARD_MAX_IMAGE_BYTES, 10 * 1024 * 1024);
        assert_eq!(CLIPBOARD_MAX_TEXT_UNITS, 256 * 1024);
    }
}
