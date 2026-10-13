//! Drag-and-drop ingestion (Phase 5.1): image files dragged from Explorer onto
//! the notch go into the existing clipboard history (and onto the system
//! clipboard). Native OLE: the Oeca window is registered as an `IDropTarget` on
//! the UI thread (an STA, see `window::run`); OLE calls these methods there,
//! from the message loop.
//!
//! Nothing is read while a drag passes over Oeca: during the drag only the file
//! *names* are inspected (extension check). Files are opened and decoded only on
//! Drop, through WIC, into the same owned `Artwork` the clipboard history uses.
//! Dropped files are never modified, moved or deleted.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{GENERIC_READ, HWND, POINT, POINTL};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
    WICConvertBitmapSource, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, DVASPECT_CONTENT, FORMATETC, IDataObject, TYMED_HGLOBAL,
};
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl, ReleaseStgMedium,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::core::{PCWSTR, Ref, Result, implement};

use crate::clipboard::{AddOutcome, ClipboardHistory, ClipboardItem};
use crate::config::CLIPBOARD_MAX_IMAGE_BYTES;
use crate::media::Artwork;

/// Image file types Oeca accepts (decoded by the built-in WIC codecs).
const SUPPORTED_EXTENSIONS: [&str; 9] = [
    "png", "jpg", "jpeg", "jfif", "bmp", "gif", "tif", "tiff", "webp",
];

/// At most this many dropped paths are considered (bounded work per drop).
const MAX_DROPPED_FILES: u32 = 32;

/// Standard clipboard format for a dropped file list (shellapi.h).
const CF_HDROP: u16 = 15;

/// Supported by file extension (case-insensitive). The content is only checked
/// when the file is decoded on drop.
pub fn is_supported_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        SUPPORTED_EXTENSIONS
            .iter()
            .any(|s| e.eq_ignore_ascii_case(s))
    })
}

/// The supported image paths among the dragged ones, in order.
pub fn supported_images(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|p| is_supported_image(p))
        .cloned()
        .collect()
}

/// Why a dropped file was not stored. Every case is non-fatal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropError {
    Unsupported,
    /// Missing, a folder, or not readable.
    Unreadable,
    /// Not a decodable image (e.g. a renamed non-image file).
    Malformed,
    /// Over the clipboard image budget (checked before decoding pixels).
    TooLarge,
}

/// Decodes an image file into owned premultiplied BGRA (`Artwork`) via WIC.
/// Requires COM on the calling thread. The pixel budget is checked from the
/// header size before any pixel is decoded.
pub fn decode_image_file(path: &Path) -> std::result::Result<Artwork, DropError> {
    if !is_supported_image(path) {
        return Err(DropError::Unsupported);
    }
    if !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
        return Err(DropError::Unreadable);
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let bad = |_| DropError::Malformed;
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(bad)?;
        let decoder = factory
            .CreateDecoderFromFilename(
                PCWSTR(wide.as_ptr()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .map_err(bad)?;
        let frame = decoder.GetFrame(0).map_err(bad)?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h).map_err(bad)?;
        if w == 0 || h == 0 {
            return Err(DropError::Malformed);
        }
        let bytes = (w as usize)
            .checked_mul(h as usize)
            .and_then(|n| n.checked_mul(4))
            .filter(|&n| n <= CLIPBOARD_MAX_IMAGE_BYTES)
            .ok_or(DropError::TooLarge)?;
        let source = WICConvertBitmapSource(&GUID_WICPixelFormat32bppPBGRA, &frame).map_err(bad)?;
        let mut pixels = vec![0u8; bytes];
        source
            .CopyPixels(std::ptr::null(), w * 4, &mut pixels)
            .map_err(bad)?;
        Artwork::new(w, h, pixels).ok_or(DropError::Malformed)
    }
}

/// Adds dropped images to the history (all its limits and the consecutive
/// duplicate rule apply). True when the newest dropped image is now the
/// history's front entry and should also go on the system clipboard.
pub fn ingest(history: &mut ClipboardHistory, images: Vec<Artwork>) -> bool {
    let mut front = false;
    for image in images {
        front = match history.add(ClipboardItem::Image(image)) {
            AddOutcome::Added | AddOutcome::Duplicate => true,
            AddOutcome::Rejected(_) => front,
        };
    }
    front
}

// ---- Drag session ---------------------------------------------------------

/// Per-drag state (one enter ... leave/drop sequence). Pure: the window feeds
/// it facts and performs the actions it returns.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DragSession {
    active: bool,
    /// The dragged data contains at least one supported image file.
    images: bool,
    /// This drag expanded the notch, so ending it collapses it again.
    expanded_by_drag: bool,
}

/// What a drag-over position means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragOver {
    /// Report the copy effect (a drop here would be accepted).
    pub accept: bool,
    /// Expand the notch (it is neither expanded nor expanding yet).
    pub expand: bool,
}

impl DragSession {
    /// A drag entered the window; any previous session is replaced.
    pub fn enter(&mut self, images: bool) {
        *self = Self {
            active: true,
            images,
            expanded_by_drag: false,
        };
    }

    /// The pointer moved. Supported images over the notch body are accepted;
    /// the first such moment expands a collapsed notch (once per drag).
    pub fn over(&mut self, over_notch: bool, notch_expanded: bool) -> DragOver {
        let accept = self.active && self.images && over_notch;
        let expand = accept && !notch_expanded;
        if expand {
            self.expanded_by_drag = true;
        }
        DragOver { accept, expand }
    }

    /// Leave, cancel or drop: ends the session. True if the notch should
    /// collapse again (only when this drag expanded it).
    pub fn end(&mut self) -> bool {
        let collapse = self.active && self.expanded_by_drag;
        *self = Self::default();
        collapse
    }

    #[cfg(test)]
    pub fn is_active(&self) -> bool {
        self.active
    }
}

// ---- OLE drop target ------------------------------------------------------

/// Drag events delivered to the window (UI thread).
#[derive(Debug)]
pub enum DragEvent {
    Enter { images: bool, at: POINT },
    Over { at: POINT },
    Leave,
    Drop { images: Vec<PathBuf>, at: POINT },
}

/// The window's handler: returns true to accept (copy effect).
pub type DragHandler = fn(HWND, DragEvent) -> bool;

/// `IDropTarget` for the Oeca window. Holds no COM references between calls;
/// OLE owns it after `RegisterDragDrop` until `RevokeDragDrop`.
#[implement(IDropTarget)]
pub struct DropTarget {
    hwnd: HWND,
    handler: DragHandler,
}

impl DropTarget {
    /// The COM drop target for `hwnd` (handed to `RegisterDragDrop`).
    pub fn create(hwnd: HWND, handler: DragHandler) -> IDropTarget {
        Self { hwnd, handler }.into()
    }
}

fn point(pt: &POINTL) -> POINT {
    POINT { x: pt.x, y: pt.y }
}

/// Writes the effect only when the source allows copying.
fn set_effect(effect: *mut DROPEFFECT, accept: bool) {
    if effect.is_null() {
        return;
    }
    // SAFETY: OLE passes a valid in/out DROPEFFECT for the duration of the call
    unsafe {
        let allowed = *effect;
        *effect = if accept && (allowed.0 & DROPEFFECT_COPY.0) != 0 {
            DROPEFFECT_COPY
        } else {
            DROPEFFECT_NONE
        };
    }
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        // File names only: nothing is opened while dragging
        let images = data
            .ok()
            .is_ok_and(|d| !supported_images(&dropped_paths(d)).is_empty());
        let accept = (self.handler)(
            self.hwnd,
            DragEvent::Enter {
                images,
                at: point(pt),
            },
        );
        set_effect(effect, accept);
        Ok(())
    }

    fn DragOver(
        &self,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        let accept = (self.handler)(self.hwnd, DragEvent::Over { at: point(pt) });
        set_effect(effect, accept);
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        (self.handler)(self.hwnd, DragEvent::Leave);
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        let images = data
            .ok()
            .map(|d| supported_images(&dropped_paths(d)))
            .unwrap_or_default();
        let accept = (self.handler)(
            self.hwnd,
            DragEvent::Drop {
                images,
                at: point(pt),
            },
        );
        set_effect(effect, accept);
        Ok(())
    }
}

/// File paths in a dragged data object (CF_HDROP), at most `MAX_DROPPED_FILES`.
/// The storage medium is always released; nothing outlives this call.
fn dropped_paths(data: &IDataObject) -> Vec<PathBuf> {
    let format = FORMATETC {
        cfFormat: CF_HDROP,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    // SAFETY: GetData returns an owned STGMEDIUM that is released below; the
    // HDROP is only used while the medium is alive.
    unsafe {
        let Ok(mut medium) = data.GetData(&format) else {
            return Vec::new();
        };
        let mut paths = Vec::new();
        if medium.tymed == TYMED_HGLOBAL.0 as u32 {
            let hdrop = HDROP(medium.u.hGlobal.0);
            let count = DragQueryFileW(hdrop, u32::MAX, None).min(MAX_DROPPED_FILES);
            for i in 0..count {
                let len = DragQueryFileW(hdrop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                let n = DragQueryFileW(hdrop, i, Some(&mut buf)) as usize;
                if n > 0 {
                    paths.push(PathBuf::from(OsString::from_wide(&buf[..n.min(len)])));
                }
            }
        }
        ReleaseStgMedium(&mut medium);
        paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CLIPBOARD_MAX_ENTRIES;
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn test_supported_image_detection() {
        for ok in [
            "a.png", "B.JPG", "c.jpeg", "d.Bmp", "e.gif", "f.webp", "g.tiff", "h.jfif",
        ] {
            assert!(is_supported_image(Path::new(ok)), "{ok}");
        }
        for no in [
            "notes.txt",
            "song.mp3",
            "archive.zip",
            "noext",
            "png",
            ".png.exe",
            "x.svg",
        ] {
            assert!(!is_supported_image(Path::new(no)), "{no}");
        }
    }

    #[test]
    fn test_multiple_paths_keep_only_images_in_order() {
        let dragged = paths(&[
            r"C:\a\one.png",
            r"C:\a\doc.pdf",
            r"C:\a\two.JPG",
            r"C:\a\folder",
        ]);
        assert_eq!(
            supported_images(&dragged),
            paths(&[r"C:\a\one.png", r"C:\a\two.JPG"])
        );
        assert!(supported_images(&paths(&["a.txt", "b.exe"])).is_empty());
        assert!(supported_images(&[]).is_empty());
    }

    // ---- WIC decoding on real files ----------------------------------------

    /// A temp file removed when dropped.
    struct TempFile(PathBuf);
    impl TempFile {
        fn new(name: &str, bytes: &[u8]) -> Self {
            let path =
                std::env::temp_dir().join(format!("oeca-test-{}-{name}", std::process::id()));
            std::fs::write(&path, bytes).unwrap();
            Self(path)
        }
    }
    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn com() {
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    }

    /// A .bmp file (file header + Oeca's own 32 bpp DIB) of `w` x `h`.
    fn bmp(w: u32, h: u32, bgra: [u8; 4]) -> Vec<u8> {
        let px: Vec<u8> = (0..w * h).flat_map(|_| bgra).collect();
        let dib = crate::clipboard::artwork_to_dib(&Artwork::new(w, h, px).unwrap());
        let mut file = b"BM".to_vec();
        file.extend(((14 + dib.len()) as u32).to_le_bytes());
        file.extend([0u8; 4]);
        file.extend(54u32.to_le_bytes());
        file.extend(dib);
        file
    }

    #[test]
    fn test_decode_valid_image_into_artwork() {
        com();
        let f = TempFile::new("ok.bmp", &bmp(5, 3, [40, 80, 160, 255]));
        let art = decode_image_file(&f.0).expect("decodes");
        assert_eq!((art.width, art.height), (5, 3));
        assert_eq!(art.pixels.len(), 5 * 3 * 4);
        assert!(
            art.pixels.chunks(4).all(|p| p == [40, 80, 160, 255]),
            "BGRA premultiplied"
        );
        // Deterministic: decoding the same file twice gives an equal item
        assert_eq!(decode_image_file(&f.0), Ok(art));
    }

    #[test]
    fn test_decode_rejects_bad_inputs_cleanly() {
        com();
        let renamed = TempFile::new("fake.png", b"this is a text file, not a PNG");
        assert_eq!(decode_image_file(&renamed.0), Err(DropError::Malformed));
        let missing = std::env::temp_dir().join("oeca-test-does-not-exist.png");
        assert_eq!(decode_image_file(&missing), Err(DropError::Unreadable));
        let dir = std::env::temp_dir().join(format!("oeca-test-{}-folder.png", std::process::id()));
        let _ = std::fs::create_dir(&dir);
        assert_eq!(
            decode_image_file(&dir),
            Err(DropError::Unreadable),
            "a folder"
        );
        let _ = std::fs::remove_dir(&dir);
        let text = TempFile::new("notes.txt", b"hello");
        assert_eq!(decode_image_file(&text.0), Err(DropError::Unsupported));
        // Over the budget (1700 x 1700 x 4 = 11.56 MB > 10 MiB): rejected from the
        // frame size, before any pixel is decoded
        let big = TempFile::new("huge.bmp", &bmp(1700, 1700, [0, 0, 0, 255]));
        assert_eq!(decode_image_file(&big.0), Err(DropError::TooLarge));
    }

    // ---- Ingestion into the existing history -------------------------------

    fn art(w: u32, h: u32, shade: u8) -> Artwork {
        Artwork::new(w, h, vec![shade; (w * h * 4) as usize]).unwrap()
    }

    #[test]
    fn test_ingest_respects_history_limits_and_dedup() {
        let mut h = ClipboardHistory::default();
        assert!(!ingest(&mut h, vec![]), "nothing dropped");
        assert!(ingest(&mut h, vec![art(4, 4, 1)]));
        assert_eq!(h.len(), 1);
        // Same image dropped again: no second entry, still the front item
        assert!(ingest(&mut h, vec![art(4, 4, 1)]));
        assert_eq!(h.len(), 1);
        // Many dropped images: the entry limit holds, oldest evicted
        let many: Vec<Artwork> = (0..30).map(|i| art(2, 2, i as u8 + 10)).collect();
        assert!(ingest(&mut h, many));
        assert_eq!(h.len(), CLIPBOARD_MAX_ENTRIES);
        assert_eq!(h.get(0), Some(&ClipboardItem::Image(art(2, 2, 39))));
        // Image memory limit: an oversized image is rejected, nothing to place
        let mut fresh = ClipboardHistory::default();
        assert!(!ingest(&mut fresh, vec![art(1700, 1700, 3)]));
        assert!(fresh.is_empty());
        // ...but a valid image in the same drop still lands at the front
        assert!(ingest(&mut fresh, vec![art(3, 3, 9), art(1700, 1700, 3)]));
        assert_eq!(fresh.get(0), Some(&ClipboardItem::Image(art(3, 3, 9))));
    }

    #[test]
    fn test_own_clipboard_write_after_drop_adds_nothing() {
        // After a drop, Oeca writes the front image to the clipboard; the update
        // that write triggers is skipped by the owner check, and even if it were
        // read it would be a consecutive duplicate.
        let mut h = ClipboardHistory::default();
        assert!(ingest(&mut h, vec![art(6, 6, 42)]));
        let written = h.get(0).cloned().unwrap();
        assert_eq!(h.add(written), AddOutcome::Duplicate);
        assert_eq!(h.len(), 1);
    }

    // ---- Drag session -----------------------------------------------------

    #[test]
    fn test_drag_expands_once_and_collapses_only_what_it_opened() {
        let mut s = DragSession::default();
        s.enter(true);
        assert_eq!(
            s.over(false, false),
            DragOver {
                accept: false,
                expand: false
            },
            "outside the notch"
        );
        assert_eq!(
            s.over(true, false),
            DragOver {
                accept: true,
                expand: true
            }
        );
        // Now expanding/expanded: keep accepting, never restart the expansion
        assert_eq!(
            s.over(true, true),
            DragOver {
                accept: true,
                expand: false
            }
        );
        assert!(s.end(), "leaving collapses what the drag opened");
        assert!(!s.is_active());

        // Already expanded before the drag: dropping leaves it expanded
        s.enter(true);
        assert_eq!(
            s.over(true, true),
            DragOver {
                accept: true,
                expand: false
            }
        );
        assert!(!s.end());
    }

    #[test]
    fn test_unsupported_drag_is_never_accepted() {
        let mut s = DragSession::default();
        s.enter(false);
        assert_eq!(
            s.over(true, false),
            DragOver {
                accept: false,
                expand: false
            }
        );
        assert!(!s.end(), "nothing to collapse");
        // Over/end without an enter (e.g. after destruction) are inert
        assert_eq!(
            s.over(true, false),
            DragOver {
                accept: false,
                expand: false
            }
        );
        assert!(!s.end());
    }

    #[test]
    fn test_repeated_enter_leave_cycles_stay_balanced() {
        let mut s = DragSession::default();
        for i in 0..50 {
            s.enter(i % 3 != 0);
            s.over(i % 2 == 0, false);
            s.end();
            assert_eq!(s, DragSession::default(), "clean after cycle {i}");
        }
        // A new enter after an expansion that was never ended starts fresh
        s.enter(true);
        s.over(true, false);
        s.enter(true);
        assert!(
            !s.end(),
            "stale expansion flag does not leak into the next drag"
        );
    }
}
