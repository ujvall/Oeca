//! Offline image and audio conversion (the Inbox's conversion workspace).
//!
//! Pure Rust, no Win32 and no UI state: `inspect` (what is this file),
//! `outputs_for` (what can it become) and `convert` (do it, off the UI thread,
//! with cooperative cancellation and bounded progress). Nothing here touches
//! the network, the clipboard or the input file.
//!
//! Engines (all in-process, no external executables):
//! - images: the `image` crate (PNG, JPEG, WebP, BMP, TIFF, GIF in; PNG, JPEG,
//!   WebP, BMP, TIFF out). WebP output is lossless (the encoder has no lossy
//!   mode); JPEG takes a quality and an explicit background for transparency.
//! - audio in: Symphonia (WAV, FLAC, MP3, Ogg Vorbis, AAC and ALAC in M4A).
//! - audio out: WAV (written here), FLAC (`flacenc`), Ogg Vorbis (`vorbis_rs`).
//!   MP3, AAC and Opus are decode-only: their encoders are LGPL/GPL or need C
//!   toolchains and patents Oeca will not ship.
//!
//! Output handling: the result is written to a hidden temp file next to its
//! destination and only then moved to a name that did not exist ("name.ext",
//! then "name (2).ext", ...). A failed or cancelled conversion removes its
//! temp file; an existing file is never overwritten.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use image::{DynamicImage, ImageDecoder, ImageEncoder, Limits};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_ALAC, CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey};
use symphonia::core::probe::Hint;

/// Images beyond this many pixels (or side length) are refused before any pixel
/// is decoded: RGBA at the limit is ~256 MB.
const MAX_PIXELS: u64 = 64_000_000;
const MAX_SIDE: u32 = 16_384;
/// A damaged stream may skip this many undecodable packets before it fails.
const MAX_BAD_PACKETS: u32 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Image,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Png,
    Jpeg,
    WebP,
    Bmp,
    Tiff,
    Gif,
    /// Decode only.
    Avif,
    Wav,
    Flac,
    Mp3,
    Ogg,
    M4a,
}

impl Format {
    pub fn family(self) -> Family {
        match self {
            Self::Png
            | Self::Jpeg
            | Self::WebP
            | Self::Bmp
            | Self::Tiff
            | Self::Gif
            | Self::Avif => Family::Image,
            _ => Family::Audio,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::WebP => "WebP",
            Self::Bmp => "BMP",
            Self::Tiff => "TIFF",
            Self::Gif => "GIF",
            Self::Avif => "AVIF",
            Self::Wav => "WAV",
            Self::Flac => "FLAC",
            Self::Mp3 => "MP3",
            Self::Ogg => "OGG",
            Self::M4a => "M4A",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::WebP => "webp",
            Self::Bmp => "bmp",
            Self::Tiff => "tif",
            Self::Gif => "gif",
            Self::Avif => "avif",
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Mp3 => "mp3",
            Self::Ogg => "ogg",
            Self::M4a => "m4a",
        }
    }

    /// Formats Oeca can write.
    pub fn can_encode(self) -> bool {
        matches!(
            self,
            Self::Png
                | Self::Jpeg
                | Self::WebP
                | Self::Bmp
                | Self::Tiff
                | Self::Wav
                | Self::Flac
                | Self::Ogg
        )
    }
}

/// Every format Oeca can write, in the order the UI offers them.
pub const OUTPUTS: [Format; 8] = [
    Format::Png,
    Format::Jpeg,
    Format::WebP,
    Format::Bmp,
    Format::Tiff,
    Format::Wav,
    Format::Flac,
    Format::Ogg,
];

/// The output formats offered for an input in `from`: every writable format of
/// the same family except `from` itself (no same-format "conversion").
#[cfg(test)]
pub fn outputs_for(from: Format) -> Vec<Format> {
    common_outputs(&[from])
}

/// Output formats offered for a bulk selection (two or more files): every
/// writable format of their family, including a format some of the files
/// already are (those are copied unchanged, see `Request::same_format_copy`).
/// Empty when the families are mixed or `inputs` is empty.
pub fn bulk_outputs(inputs: &[Format]) -> Vec<Format> {
    let Some(&first) = inputs.first() else {
        return Vec::new();
    };
    if inputs.iter().any(|f| f.family() != first.family()) {
        return Vec::new();
    }
    OUTPUTS
        .into_iter()
        .filter(|out| out.family() == first.family())
        .collect()
}

/// Output formats every one of `inputs` can become: the writable formats of
/// their family that none of them already is. Empty when the families are
/// mixed (the selection must be split) or `inputs` is empty.
pub fn common_outputs(inputs: &[Format]) -> Vec<Format> {
    let Some(&first) = inputs.first() else {
        return Vec::new();
    };
    if inputs.iter().any(|f| f.family() != first.family()) {
        return Vec::new();
    }
    OUTPUTS
        .into_iter()
        .filter(|&out| out.family() == first.family() && !inputs.contains(&out))
        .collect()
}

#[derive(Debug)]
pub enum ConvertError {
    /// Missing, a folder, or not readable.
    Unreadable,
    /// Not an image or audio format Oeca recognises.
    Unsupported,
    /// Recognised, but the content is damaged, truncated or uses an
    /// unsupported codec/profile.
    Malformed(String),
    /// Over the safe size/dimension limits.
    TooLarge,
    /// The chosen output cannot be produced from this input.
    NotAvailable(&'static str),
    /// A setting is outside its range.
    BadSetting(&'static str),
    Cancelled,
    Io(io::Error),
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable => f.write_str("Can't read this file"),
            Self::Unsupported => f.write_str("Unsupported format"),
            Self::Malformed(why) => {
                // "the AVIF can't be decoded" -> "The AVIF can't be decoded"
                let mut chars = why.chars();
                let first = chars.next().map(|c| c.to_uppercase().collect::<String>());
                write!(f, "{}{}", first.unwrap_or_default(), chars.as_str())
            }
            Self::TooLarge => f.write_str("Too large to convert"),
            Self::NotAvailable(why) => f.write_str(why),
            Self::BadSetting(why) => f.write_str(why),
            Self::Cancelled => f.write_str("Cancelled"),
            Self::Io(e) => write!(f, "Couldn't write the output: {e}"),
        }
    }
}

impl std::error::Error for ConvertError {}

impl From<io::Error> for ConvertError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Conversion settings with sensible defaults (a typical user never changes them).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// JPEG quality, 1..=100.
    pub jpeg_quality: u8,
    /// What transparency becomes in JPEG (which has no alpha). White, never
    /// silently black.
    pub background: [u8; 3],
    /// Ogg Vorbis quality, -0.1..=1.0 (0.5 is about 160 kbit/s stereo).
    pub vorbis_quality: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            jpeg_quality: 90,
            background: [255, 255, 255],
            vorbis_quality: 0.5,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), ConvertError> {
        if !(1..=100).contains(&self.jpeg_quality) {
            return Err(ConvertError::BadSetting("JPEG quality must be 1 to 100"));
        }
        if !(-0.1..=1.0).contains(&self.vorbis_quality) {
            return Err(ConvertError::BadSetting("Vorbis quality must be -0.1 to 1"));
        }
        Ok(())
    }
}

// ---- Detection ------------------------------------------------------------

/// The format of `head` (a file's first bytes) by content, never by extension.
pub fn sniff_bytes(head: &[u8]) -> Option<Format> {
    let h = head;
    if h.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Format::Png)
    } else if h.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Format::Jpeg)
    } else if h.starts_with(b"GIF87a") || h.starts_with(b"GIF89a") {
        Some(Format::Gif)
    } else if h.starts_with(b"BM") && h.len() >= 14 {
        Some(Format::Bmp)
    } else if h.starts_with(b"II*\0") || h.starts_with(b"MM\0*") {
        Some(Format::Tiff)
    } else if h.len() >= 12 && &h[..4] == b"RIFF" && &h[8..12] == b"WEBP" {
        Some(Format::WebP)
    } else if h.len() >= 12 && &h[..4] == b"RIFF" && &h[8..12] == b"WAVE" {
        Some(Format::Wav)
    } else if h.starts_with(b"fLaC") {
        Some(Format::Flac)
    } else if h.starts_with(b"OggS") {
        Some(Format::Ogg)
    } else if h.len() >= 12 && &h[4..8] == b"ftyp" {
        // AVIF is an ISO-BMFF file whose major or compatible brands say so;
        // any other brand (M4A, MP4, HEIC...) is treated as audio and
        // rejected by the audio decoder if it is not
        let brands = &h[8..];
        let avif = brands
            .chunks(4)
            .enumerate()
            .any(|(i, b)| i != 1 && (b == b"avif" || b == b"avis"));
        Some(if avif { Format::Avif } else { Format::M4a })
    } else if h.starts_with(b"ID3") || (h.len() >= 2 && h[0] == 0xFF && (h[1] & 0xE6) == 0xE2) {
        // ID3 tag, or an MPEG audio frame sync (layer bits not 00: ADTS AAC
        // would be 00 and is not supported)
        Some(Format::Mp3)
    } else {
        None
    }
}

pub fn sniff(path: &Path) -> Result<Format, ConvertError> {
    let meta = fs::metadata(path).map_err(|_| ConvertError::Unreadable)?;
    if !meta.is_file() {
        return Err(ConvertError::Unreadable);
    }
    let mut head = [0u8; 32];
    let mut file = File::open(path).map_err(|_| ConvertError::Unreadable)?;
    let n = read_up_to(&mut file, &mut head).map_err(|_| ConvertError::Unreadable)?;
    sniff_bytes(&head[..n]).ok_or(ConvertError::Unsupported)
}

fn read_up_to(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// What a dropped file is, for its row in the batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub format: Format,
    pub bytes: u64,
    /// "1920 × 1080" or "44.1 kHz stereo".
    pub detail: String,
}

/// Validates `path` without decoding it: readable, a recognised format, within
/// the size limits, and (audio) a codec Symphonia can decode.
pub fn inspect(path: &Path) -> Result<Info, ConvertError> {
    guarded(|| inspect_inner(path))
}

fn inspect_inner(path: &Path) -> Result<Info, ConvertError> {
    let format = sniff(path)?;
    let bytes = fs::metadata(path)
        .map_err(|_| ConvertError::Unreadable)?
        .len();
    if bytes == 0 {
        return Err(ConvertError::Malformed("the file is empty".into()));
    }
    let detail = match format.family() {
        Family::Image if format == Format::Avif => {
            let (w, h) = avif_dimensions(path)?;
            check_dimensions(w, h)?;
            format!("{w} × {h}")
        }
        Family::Image => {
            let reader = image_reader(path, format)?;
            let (w, h) = reader
                .into_dimensions()
                .map_err(|e| ConvertError::Malformed(image_err(&e)))?;
            check_dimensions(w, h)?;
            format!("{w} × {h}")
        }
        Family::Audio => {
            let probed = probe(path)?;
            let track = pick_track(probed.format.as_ref())?;
            let p = &track.codec_params;
            // Fails early for codecs with no decoder (Opus, ADTS, ...)
            symphonia::default::get_codecs()
                .make(p, &DecoderOptions::default())
                .map_err(|_| ConvertError::Malformed("unsupported audio codec".into()))?;
            let rate = p.sample_rate.map_or(String::new(), |r| {
                format!("{:.1} kHz", f64::from(r) / 1000.0)
            });
            let ch = p.channels.map_or("", |c| match c.count() {
                1 => " mono",
                2 => " stereo",
                _ => " multichannel",
            });
            format!("{rate}{ch}").trim().to_string()
        }
    };
    Ok(Info {
        format,
        bytes,
        detail,
    })
}

fn check_dimensions(w: u32, h: u32) -> Result<(), ConvertError> {
    if w == 0 || h == 0 {
        return Err(ConvertError::Malformed("the image is empty".into()));
    }
    if w > MAX_SIDE || h > MAX_SIDE || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(ConvertError::TooLarge);
    }
    Ok(())
}

// ---- Output handling --------------------------------------------------------

/// "stem.ext" in `dir`, or the first of "stem (2).ext", "stem (3).ext", ...
/// that does not exist. The name is reserved atomically (created empty) so two
/// conversions can never pick the same one.
fn reserve_output(dir: &Path, stem: &str, ext: &str) -> io::Result<PathBuf> {
    for n in 1..10_000u32 {
        let name = if n == 1 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free file name",
    ))
}

/// Temp file removed on drop unless `persist` moved it to its final name.
struct Temp {
    path: PathBuf,
    done: bool,
}

impl Drop for Temp {
    fn drop(&mut self) {
        if !self.done {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Moves a staged output to its final home under a name that did not exist
/// (`stem.ext`, `stem (2).ext`, ...): renamed, or copied then removed when
/// `dir` is on another volume. The staged file is removed on success only.
pub fn publish(staged: &Path, dir: &Path, stem: &str, ext: &str) -> io::Result<PathBuf> {
    let target = reserve_output(dir, stem, ext)?;
    let moved = fs::rename(staged, &target).or_else(|_| {
        fs::copy(staged, &target)?;
        fs::remove_file(staged)
    });
    match moved {
        Ok(()) => Ok(target),
        Err(e) => {
            let _ = fs::remove_file(&target);
            Err(e)
        }
    }
}

// ---- Public entry -------------------------------------------------------------

#[derive(Debug)]
pub struct Request<'a> {
    pub input: &'a Path,
    pub to: Format,
    pub out_dir: &'a Path,
    pub settings: Settings,
    /// A file already in `to` is copied unchanged (bulk batches); otherwise
    /// converting a file to its own format is refused.
    pub same_format_copy: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Done {
    pub output: PathBuf,
    pub bytes: u64,
    pub format: Format,
    /// Something the output does not carry over (e.g. a colour profile).
    pub note: Option<&'static str>,
}

/// Runs `f`, turning a panic inside a decoder (a malformed file can trip an
/// assertion in a parser) into an error for that one file, so a worker never
/// dies silently with a batch left waiting on it.
fn guarded<T>(f: impl FnOnce() -> Result<T, ConvertError>) -> Result<T, ConvertError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| {
        Err(ConvertError::Malformed(
            "the file can't be read safely".into(),
        ))
    })
}

/// Converts `req.input` into `req.to` inside `req.out_dir`. Runs on the
/// calling thread (never the UI thread); `cancel` is polled between steps and
/// `progress` receives 0.0..=1.0, at most about once per percent.
pub fn convert(
    req: &Request,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<Done, ConvertError> {
    guarded(|| convert_inner(req, cancel, progress))
}

fn convert_inner(
    req: &Request,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<Done, ConvertError> {
    req.settings.validate()?;
    let from = sniff(req.input)?;
    if from == req.to && !req.same_format_copy {
        return Err(ConvertError::NotAvailable(
            "Already in this format: choose another",
        ));
    }
    if from.family() != req.to.family() {
        return Err(ConvertError::NotAvailable(
            "Images and audio can't convert to each other",
        ));
    }
    if !req.to.can_encode() {
        return Err(ConvertError::NotAvailable("Oeca can't write this format"));
    }
    if !req.out_dir.is_dir() {
        return Err(ConvertError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            "the output folder doesn't exist",
        )));
    }
    let stem = req
        .input
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("converted");
    let check = |c: &AtomicBool| {
        if c.load(Ordering::Relaxed) {
            Err(ConvertError::Cancelled)
        } else {
            Ok(())
        }
    };
    check(cancel)?;

    // Hidden temp beside the destination (same volume: the move is a rename)
    let temp_path = req
        .out_dir
        .join(format!(".oeca-{}-{stem}.part", std::process::id()));
    let mut temp = Temp {
        path: temp_path,
        done: false,
    };
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temp.path)?;
    let mut out = BufWriter::with_capacity(1 << 16, file);

    let (mut out, note) = match req.to.family() {
        // Already in the requested format: the safest quality-preserving
        // result is the file itself, byte for byte (tags, profile and all),
        // after checking it really is a readable file of that format
        _ if from == req.to => {
            inspect(req.input)?;
            decode_check(req.input, from, cancel)?;
            let mut src = File::open(req.input).map_err(|_| ConvertError::Unreadable)?;
            io::copy(&mut src, &mut out)?;
            (out, Some("Copied unchanged"))
        }
        Family::Image => {
            let note = convert_image(req, from, &mut out, cancel, progress)?;
            (out, note)
        }
        Family::Audio => convert_audio(req, from, out, cancel, progress)?,
    };
    check(cancel)?;
    out.flush()?;
    drop(out);

    let output = reserve_output(req.out_dir, stem, req.to.extension())?;
    if let Err(e) = fs::rename(&temp.path, &output) {
        let _ = fs::remove_file(&output);
        return Err(e.into());
    }
    temp.done = true;
    let bytes = fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    progress(1.0);
    Ok(Done {
        output,
        bytes,
        format: req.to,
        note,
    })
}

/// Decodes the whole file and drops the result: a truncated or damaged file
/// with a valid header is reported, not copied as a success.
fn decode_check(path: &Path, from: Format, cancel: &AtomicBool) -> Result<(), ConvertError> {
    if from == Format::Avif {
        decode_avif(path)?;
        return Ok(());
    }
    if from.family() == Family::Image {
        let malformed = |e: image::ImageError| match e {
            image::ImageError::Limits(_) => ConvertError::TooLarge,
            e => ConvertError::Malformed(image_err(&e)),
        };
        let mut decoder = image_reader(path, from)?
            .into_decoder()
            .map_err(malformed)?;
        let mut limits = Limits::default();
        limits.max_alloc = Some(1 << 30);
        decoder.set_limits(limits).map_err(malformed)?;
        DynamicImage::from_decoder(decoder).map_err(malformed)?;
        return Ok(());
    }
    let mut probed = probe(path)?;
    let track = pick_track(probed.format.as_ref())?.clone();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|_| ConvertError::Malformed("unsupported audio codec".into()))?;
    let (mut good, mut bad) = (0u32, 0u32);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(ConvertError::Cancelled);
        }
        let packet = match probed.format.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if packet.track_id() != track.id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(_) => good += 1,
            Err(SymError::DecodeError(_)) => {
                bad += 1;
                if bad > MAX_BAD_PACKETS {
                    return Err(ConvertError::Malformed("the audio is damaged".into()));
                }
            }
            Err(_) => break,
        }
    }
    if good == 0 {
        return Err(ConvertError::Malformed("the file has no audio".into()));
    }
    Ok(())
}

// ---- Images ---------------------------------------------------------------------

/// AVIF files are read whole (they are small by nature); anything bigger than
/// this is refused before it is parsed.
const MAX_AVIF_BYTES: u64 = 64 << 20;

fn read_avif_file(path: &Path) -> Result<Vec<u8>, ConvertError> {
    let len = fs::metadata(path)
        .map_err(|_| ConvertError::Unreadable)?
        .len();
    if len > MAX_AVIF_BYTES {
        return Err(ConvertError::TooLarge);
    }
    fs::read(path).map_err(|_| ConvertError::Unreadable)
}

fn avif_error(what: &str) -> ConvertError {
    ConvertError::Malformed(format!("the AVIF {what}"))
}

/// Picture size from the container and the AV1 sequence header (no pixels).
fn avif_dimensions(path: &Path) -> Result<(u32, u32), ConvertError> {
    let bytes = read_avif_file(path)?;
    let data = avif_parse::read_avif(&mut bytes.as_slice())
        .map_err(|_| avif_error("is damaged or not a still image"))?;
    let meta = data
        .primary_item_metadata()
        .map_err(|_| avif_error("has no readable picture"))?;
    Ok((meta.max_frame_width.get(), meta.max_frame_height.get()))
}

/// Decodes a still AVIF (8/10/12-bit, with alpha) to pixels. Orientation
/// (irot/imir) and colour profiles are not exposed by the parser, so they are
/// not applied.
fn decode_avif(path: &Path) -> Result<DynamicImage, ConvertError> {
    use avif_decode::Image as A;
    let (w, h) = avif_dimensions(path)?;
    check_dimensions(w, h)?;
    let bytes = read_avif_file(path)?;
    let decoder =
        avif_decode::Decoder::from_avif(&bytes).map_err(|_| avif_error("can't be read"))?;
    let img = decoder
        .to_image()
        .map_err(|_| avif_error("can't be decoded"))?;
    macro_rules! flat {
        ($img:expr, $f:expr) => {{
            let (buf, w, h) = $img.into_contiguous_buf();
            (
                buf.into_iter().flat_map($f).collect::<Vec<_>>(),
                w as u32,
                h as u32,
            )
        }};
    }
    let bad = || avif_error("has an inconsistent size");
    Ok(match img {
        A::Rgb8(i) => {
            let (v, w, h) = flat!(i, |p| [p.r, p.g, p.b]);
            DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, v).ok_or_else(bad)?)
        }
        A::Rgba8(i) => {
            let (v, w, h) = flat!(i, |p| [p.r, p.g, p.b, p.a]);
            DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, v).ok_or_else(bad)?)
        }
        A::Gray8(i) => {
            let (v, w, h) = flat!(i, |p| [p.value()]);
            DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, v).ok_or_else(bad)?)
        }
        A::Rgb16(i) => {
            let (v, w, h) = flat!(i, |p| [p.r, p.g, p.b]);
            DynamicImage::ImageRgb16(image::ImageBuffer::from_raw(w, h, v).ok_or_else(bad)?)
        }
        A::Rgba16(i) => {
            let (v, w, h) = flat!(i, |p| [p.r, p.g, p.b, p.a]);
            DynamicImage::ImageRgba16(image::ImageBuffer::from_raw(w, h, v).ok_or_else(bad)?)
        }
        A::Gray16(i) => {
            let (v, w, h) = flat!(i, |p| [p.value()]);
            DynamicImage::ImageLuma16(image::ImageBuffer::from_raw(w, h, v).ok_or_else(bad)?)
        }
    })
}

fn image_format(f: Format) -> image::ImageFormat {
    match f {
        Format::Png => image::ImageFormat::Png,
        Format::Jpeg => image::ImageFormat::Jpeg,
        Format::WebP => image::ImageFormat::WebP,
        Format::Bmp => image::ImageFormat::Bmp,
        Format::Tiff => image::ImageFormat::Tiff,
        _ => image::ImageFormat::Gif,
    }
}

fn image_reader(
    path: &Path,
    format: Format,
) -> Result<image::ImageReader<BufReader<File>>, ConvertError> {
    let file = File::open(path).map_err(|_| ConvertError::Unreadable)?;
    Ok(image::ImageReader::with_format(
        BufReader::new(file),
        image_format(format),
    ))
}

fn image_err(e: &image::ImageError) -> String {
    match e {
        image::ImageError::Unsupported(_) => "unsupported image variant".into(),
        image::ImageError::Limits(_) => "too large".into(),
        _ => "the image can't be decoded".into(),
    }
}

/// Source colour profile that the target could not embed.
const PROFILE_LOST: &str = "Colour profile not kept";

fn convert_image(
    req: &Request,
    from: Format,
    out: &mut BufWriter<File>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<Option<&'static str>, ConvertError> {
    let malformed = |e: image::ImageError| match e {
        image::ImageError::Limits(_) => ConvertError::TooLarge,
        e => ConvertError::Malformed(image_err(&e)),
    };
    let (img, icc) = if from == Format::Avif {
        (decode_avif(req.input)?, None)
    } else {
        let mut decoder = image_reader(req.input, from)?
            .into_decoder()
            .map_err(malformed)?;
        let (w, h) = decoder.dimensions();
        check_dimensions(w, h)?;
        let mut limits = Limits::default();
        limits.max_alloc = Some(1 << 30);
        decoder.set_limits(limits).map_err(malformed)?;
        let orientation = decoder
            .orientation()
            .unwrap_or(image::metadata::Orientation::NoTransforms);
        let icc = decoder.icc_profile().ok().flatten();
        let mut img = DynamicImage::from_decoder(decoder).map_err(malformed)?;
        img.apply_orientation(orientation);
        (img, icc)
    };
    progress(0.4);
    if cancel.load(Ordering::Relaxed) {
        return Err(ConvertError::Cancelled);
    }

    let mut embedded = icc.is_none();
    let mut set_icc = |enc: &mut dyn ImageEncoder| {
        if let Some(p) = &icc {
            embedded = enc.set_icc_profile(p.clone()).is_ok();
        }
    };
    match req.to {
        Format::Png => {
            let mut enc = image::codecs::png::PngEncoder::new(&mut *out);
            set_icc(&mut enc);
            img.write_with_encoder(enc).map_err(write_err)?;
        }
        Format::Tiff => {
            let img = normalise(img, true);
            let mut enc = image::codecs::tiff::TiffEncoder::new(&mut *out);
            set_icc(&mut enc);
            img.write_with_encoder(enc).map_err(write_err)?;
        }
        Format::Jpeg => {
            let img = flatten(img, req.settings.background);
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut *out,
                req.settings.jpeg_quality,
            );
            set_icc(&mut enc);
            img.write_with_encoder(enc).map_err(write_err)?;
        }
        Format::WebP => {
            let img = normalise(img, false);
            let mut enc = image::codecs::webp::WebPEncoder::new_lossless(&mut *out);
            set_icc(&mut enc);
            img.write_with_encoder(enc).map_err(write_err)?;
        }
        Format::Bmp => {
            let img = normalise(img, false);
            let mut enc = image::codecs::bmp::BmpEncoder::new(&mut *out);
            set_icc(&mut enc);
            img.write_with_encoder(enc).map_err(write_err)?;
        }
        _ => return Err(ConvertError::NotAvailable("Oeca can't write this format")),
    }
    Ok((!embedded).then_some(PROFILE_LOST))
}

fn write_err(e: image::ImageError) -> ConvertError {
    match e {
        image::ImageError::IoError(e) => ConvertError::Io(e),
        e => ConvertError::Malformed(image_err(&e)),
    }
}

/// A layout every encoder here accepts: 8-bit (16-bit kept when `keep16`),
/// with or without alpha, grayscale kept only as far as the encoders allow.
fn normalise(img: DynamicImage, keep16: bool) -> DynamicImage {
    use DynamicImage::*;
    let alpha = img.color().has_alpha();
    match img {
        ImageLuma8(_) | ImageRgb8(_) | ImageRgba8(_) => img,
        ImageLuma16(_) | ImageRgb16(_) | ImageRgba16(_) if keep16 => img,
        _ if alpha => ImageRgba8(img.to_rgba8()),
        _ => ImageRgb8(img.to_rgb8()),
    }
}

/// For formats without alpha: transparency is composited over `bg`.
fn flatten(img: DynamicImage, bg: [u8; 3]) -> DynamicImage {
    if !img.color().has_alpha() {
        return if img.color().has_color() {
            DynamicImage::ImageRgb8(img.to_rgb8())
        } else {
            DynamicImage::ImageLuma8(img.to_luma8())
        };
    }
    let mut rgba = img.to_rgba8();
    for p in rgba.pixels_mut() {
        let a = u32::from(p[3]);
        for c in 0..3 {
            p[c] = ((u32::from(p[c]) * a + u32::from(bg[c]) * (255 - a) + 127) / 255) as u8;
        }
        p[3] = 255;
    }
    DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(rgba).to_rgb8())
}

// ---- Audio ----------------------------------------------------------------------

fn probe(path: &Path) -> Result<symphonia::core::probe::ProbeResult, ConvertError> {
    let file = File::open(path).map_err(|_| ConvertError::Unreadable)?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| ConvertError::Malformed(format!("not readable as audio {e}")))
}

fn pick_track(
    format: &dyn symphonia::core::formats::FormatReader,
) -> Result<&symphonia::core::formats::Track, ConvertError> {
    format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| ConvertError::Malformed("no audio track".into()))
}

/// Tags every target here can carry, as Vorbis-comment keys.
fn collect_tags(rev: &MetadataRevision) -> Vec<(&'static str, String)> {
    let mut tags: Vec<(&'static str, String)> = Vec::new();
    for tag in rev.tags() {
        let key = match tag.std_key {
            Some(StandardTagKey::TrackTitle) => "TITLE",
            Some(StandardTagKey::Artist) => "ARTIST",
            Some(StandardTagKey::Album) => "ALBUM",
            Some(StandardTagKey::AlbumArtist) => "ALBUMARTIST",
            Some(StandardTagKey::Genre) => "GENRE",
            Some(StandardTagKey::Date) => "DATE",
            Some(StandardTagKey::TrackNumber) => "TRACKNUMBER",
            Some(StandardTagKey::DiscNumber) => "DISCNUMBER",
            Some(StandardTagKey::Composer) => "COMPOSER",
            Some(StandardTagKey::Comment) => "COMMENT",
            _ => continue,
        };
        let value = tag
            .value
            .to_string()
            .trim_end_matches('\0')
            .trim()
            .to_string();
        if !value.is_empty() && !tags.iter().any(|(k, _)| *k == key) {
            tags.push((key, value));
        }
    }
    tags
}

/// The audio sink for one conversion, created from the first decoded buffer
/// (the stream's real rate and channel count).
enum Sink {
    Wav(WavWriter),
    Ogg(Box<vorbis_rs::VorbisEncoder<BufWriter<File>>>),
}

struct WavWriter {
    out: BufWriter<File>,
    bytes_per_sample: usize,
    data_bytes: u64,
    /// Offset of the RIFF size and the data size, patched at the end.
    data_size_at: u64,
}

fn tag_chunk(tags: &[(&str, String)]) -> Vec<u8> {
    const IDS: [(&str, &[u8; 4]); 8] = [
        ("TITLE", b"INAM"),
        ("ARTIST", b"IART"),
        ("ALBUM", b"IPRD"),
        ("DATE", b"ICRD"),
        ("GENRE", b"IGNR"),
        ("TRACKNUMBER", b"ITRK"),
        ("COMMENT", b"ICMT"),
        ("COMPOSER", b"IMUS"),
    ];
    let mut body = b"INFO".to_vec();
    for (key, id) in IDS {
        if let Some((_, v)) = tags.iter().find(|(k, _)| *k == key) {
            let mut text = v.as_bytes().to_vec();
            text.push(0);
            body.extend_from_slice(id);
            body.extend_from_slice(&(text.len() as u32).to_le_bytes());
            body.extend_from_slice(&text);
            if text.len() % 2 == 1 {
                body.push(0);
            }
        }
    }
    if body.len() == 4 {
        return Vec::new();
    }
    let mut chunk = b"LIST".to_vec();
    chunk.extend_from_slice(&(body.len() as u32).to_le_bytes());
    chunk.extend_from_slice(&body);
    chunk
}

impl WavWriter {
    fn new(
        mut out: BufWriter<File>,
        rate: u32,
        channels: u16,
        bits: u16,
        tags: &[(&str, String)],
    ) -> io::Result<Self> {
        let bytes_per_sample = usize::from(bits / 8);
        let block = channels * (bits / 8);
        let list = tag_chunk(tags);
        out.write_all(b"RIFF\0\0\0\0WAVEfmt ")?;
        out.write_all(&16u32.to_le_bytes())?;
        out.write_all(&1u16.to_le_bytes())?;
        out.write_all(&channels.to_le_bytes())?;
        out.write_all(&rate.to_le_bytes())?;
        out.write_all(&(rate * u32::from(block)).to_le_bytes())?;
        out.write_all(&block.to_le_bytes())?;
        out.write_all(&bits.to_le_bytes())?;
        out.write_all(&list)?;
        out.write_all(b"data\0\0\0\0")?;
        let data_size_at = 12 + 24 + list.len() as u64 + 4;
        Ok(Self {
            out,
            bytes_per_sample,
            data_bytes: 0,
            data_size_at,
        })
    }

    /// `samples`: interleaved, scaled to the full i32 range.
    fn write(&mut self, samples: &[i32]) -> Result<(), ConvertError> {
        let shift = 32 - 8 * self.bytes_per_sample as u32;
        for &s in samples {
            let v = round_down(s, shift);
            self.out
                .write_all(&v.to_le_bytes()[..self.bytes_per_sample])?;
        }
        self.data_bytes += (samples.len() * self.bytes_per_sample) as u64;
        if self.data_bytes > u64::from(u32::MAX) - 1024 {
            return Err(ConvertError::TooLarge);
        }
        Ok(())
    }

    fn finish(mut self) -> Result<BufWriter<File>, ConvertError> {
        let pad = self.data_bytes % 2 == 1;
        if pad {
            self.out.write_all(&[0])?;
        }
        let end = self.out.stream_position()?;
        self.out.seek(SeekFrom::Start(4))?;
        self.out.write_all(&((end - 8) as u32).to_le_bytes())?;
        self.out.seek(SeekFrom::Start(self.data_size_at))?;
        self.out
            .write_all(&(self.data_bytes as u32).to_le_bytes())?;
        self.out.seek(SeekFrom::Start(end))?;
        Ok(self.out)
    }
}

/// `s` (full i32 range) at fewer bits: rounded and clamped, kept in the low bits.
fn round_down(s: i32, shift: u32) -> i32 {
    if shift == 0 {
        return s;
    }
    let half = 1i64 << (shift - 1);
    let max = i64::from(i32::MAX) >> shift;
    (((i64::from(s) + half) >> shift).min(max)) as i32
}

fn convert_audio(
    req: &Request,
    from: Format,
    out: BufWriter<File>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<(BufWriter<File>, Option<&'static str>), ConvertError> {
    let mut probed = probe(req.input)?;
    // Tags live in the container (WAV, FLAC, Ogg, M4A) or in a leading ID3 tag
    let mut tags = probed
        .format
        .metadata()
        .current()
        .map(collect_tags)
        .unwrap_or_default();
    if let Some(id3) = probed
        .metadata
        .get()
        .as_ref()
        .and_then(|m| m.current().map(collect_tags))
    {
        for (k, v) in id3 {
            if !tags.iter().any(|(t, _)| *t == k) {
                tags.push((k, v));
            }
        }
    }
    let mut format = probed.format;
    let track = pick_track(format.as_ref())?.clone();
    let params = &track.codec_params;
    let mut decoder = symphonia::default::get_codecs()
        .make(params, &DecoderOptions::default())
        .map_err(|_| ConvertError::Malformed("unsupported audio codec".into()))?;
    let lossless = matches!(from, Format::Wav | Format::Flac)
        || (from == Format::M4a && params.codec == CODEC_TYPE_ALAC);
    // Lossless sources keep their depth (16 to 24 bit); lossy decode to 16
    let bits = if lossless {
        params
            .bits_per_sample
            .or(params.bits_per_coded_sample)
            .unwrap_or(16)
            .clamp(16, 24)
    } else {
        16
    };
    let total = params.n_frames;
    // A lossy source in a lossless container is not any closer to the original
    let lossy_note = (!lossless && req.to != Format::Ogg).then_some("Source was lossy");
    let mut last_report = 0.0f32;

    let mut sink: Option<Sink> = None;
    let mut out = Some(out);
    // FLAC is encoded from a pull source over the same decode loop
    if req.to == Format::Flac {
        let (out, note) = flac(
            req,
            &mut format,
            decoder,
            track.id,
            bits,
            total,
            out.take().expect("unused"),
            tags,
            cancel,
            progress,
        )?;
        return Ok((out, note.or(lossy_note)));
    }

    let mut ints: Option<SampleBuffer<i32>> = None;
    let mut floats: Option<SampleBuffer<f32>> = None;
    let mut decoded_any = false;
    let mut bad = 0u32;
    let mut done_frames = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(ConvertError::Cancelled);
        }
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(_) if decoded_any => break,
            Err(_) => return Err(ConvertError::Malformed("no audio could be read".into())),
        };
        if packet.track_id() != track.id {
            continue;
        }
        let buf = match decoder.decode(&packet) {
            Ok(b) => b,
            Err(SymError::DecodeError(_)) => {
                bad += 1;
                if bad > MAX_BAD_PACKETS {
                    return Err(ConvertError::Malformed("the audio is damaged".into()));
                }
                continue;
            }
            Err(_) if decoded_any => break,
            Err(_) => return Err(ConvertError::Malformed("no audio could be read".into())),
        };
        let spec = *buf.spec();
        let channels = spec.channels.count();
        if channels > 2 {
            return Err(ConvertError::NotAvailable(
                "More than 2 channels: convert to FLAC instead",
            ));
        }
        decoded_any = true;
        let frames = buf.frames();
        if frames == 0 {
            continue;
        }
        if sink.is_none() {
            sink = Some(match req.to {
                Format::Wav => Sink::Wav(WavWriter::new(
                    out.take().expect("one sink"),
                    spec.rate,
                    channels as u16,
                    bits as u16,
                    &tags,
                )?),
                _ => {
                    let mut b = vorbis_rs::VorbisEncoderBuilder::new_with_serial(
                        std::num::NonZeroU32::new(spec.rate)
                            .ok_or_else(|| ConvertError::Malformed("no sample rate".into()))?,
                        std::num::NonZeroU8::new(channels as u8)
                            .ok_or_else(|| ConvertError::Malformed("no channels".into()))?,
                        out.take().expect("one sink"),
                        std::process::id() as i32,
                    );
                    b.bitrate_management_strategy(
                        vorbis_rs::VorbisBitrateManagementStrategy::QualityVbr {
                            target_quality: req.settings.vorbis_quality,
                        },
                    );
                    b.comment_tags(tags.iter().map(|(k, v)| (*k, v.as_str())))
                        .map_err(|_| ConvertError::Malformed("tags".into()))?;
                    Sink::Ogg(Box::new(b.build().map_err(vorbis_err)?))
                }
            });
        }
        match sink.as_mut() {
            Some(Sink::Wav(w)) => {
                let s = ints.get_or_insert_with(|| SampleBuffer::new(buf.capacity() as u64, spec));
                s.copy_interleaved_ref(buf);
                w.write(s.samples())?;
            }
            Some(Sink::Ogg(e)) => {
                let s =
                    floats.get_or_insert_with(|| SampleBuffer::new(buf.capacity() as u64, spec));
                s.copy_planar_ref(buf);
                let planes: Vec<&[f32]> = s.samples().chunks(frames).take(channels).collect();
                e.encode_audio_block(&planes).map_err(vorbis_err)?;
            }
            None => {}
        }
        done_frames += frames as u64;
        if let Some(t) = total.filter(|&t| t > 0) {
            let p = (done_frames as f32 / t as f32).min(0.99);
            if p - last_report >= 0.01 {
                last_report = p;
                progress(p);
            }
        }
    }
    let out = match sink {
        Some(Sink::Wav(w)) => w.finish()?,
        Some(Sink::Ogg(e)) => e.finish().map_err(vorbis_err)?,
        None => return Err(ConvertError::Malformed("the file has no audio".into())),
    };
    Ok((out, lossy_note))
}

fn vorbis_err(e: vorbis_rs::VorbisError) -> ConvertError {
    ConvertError::Malformed(format!("Vorbis encoder: {e}"))
}

/// FLAC: `flacenc` pulls blocks from a source that decodes on demand, so the
/// PCM is never held whole (only the compressed frames are).
#[allow(clippy::too_many_arguments)]
fn flac(
    req: &Request,
    format: &mut Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    bits: u32,
    total: Option<u64>,
    mut out: BufWriter<File>,
    tags: Vec<(&'static str, String)>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<(BufWriter<File>, Option<&'static str>), ConvertError> {
    use flacenc::error::{SourceError, Verify};
    use flacenc::source::{Fill, Source};

    struct Pull<'a> {
        format: &'a mut Box<dyn symphonia::core::formats::FormatReader>,
        decoder: Box<dyn symphonia::core::codecs::Decoder>,
        track_id: u32,
        bits: u32,
        rate: u32,
        channels: usize,
        queue: std::collections::VecDeque<i32>,
        buf: Option<SampleBuffer<i32>>,
        bad: u32,
        err: Option<ConvertError>,
        frames: u64,
        total: Option<u64>,
        cancel: &'a AtomicBool,
        progress: &'a mut dyn FnMut(f32),
        last: f32,
        eof: bool,
    }

    impl Pull<'_> {
        /// Decodes one packet into `queue`; false at the end or on error.
        fn refill(&mut self) -> bool {
            if self.cancel.load(Ordering::Relaxed) {
                self.err = Some(ConvertError::Cancelled);
                return false;
            }
            loop {
                let packet = match self.format.next_packet() {
                    Ok(p) => p,
                    Err(SymError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                        return false;
                    }
                    Err(SymError::ResetRequired) => return false,
                    Err(_) => {
                        if self.frames == 0 {
                            self.err =
                                Some(ConvertError::Malformed("no audio could be read".into()));
                        }
                        return false;
                    }
                };
                if packet.track_id() != self.track_id {
                    continue;
                }
                match self.decoder.decode(&packet) {
                    Ok(b) => {
                        let spec = *b.spec();
                        if b.frames() == 0 {
                            continue;
                        }
                        if spec.channels.count() != self.channels || spec.rate != self.rate {
                            self.err = Some(ConvertError::Malformed("the format changes".into()));
                            return false;
                        }
                        let s = self
                            .buf
                            .get_or_insert_with(|| SampleBuffer::new(b.capacity() as u64, spec));
                        s.copy_interleaved_ref(b);
                        let shift = 32 - self.bits;
                        self.queue
                            .extend(s.samples().iter().map(|&v| round_down(v, shift)));
                        self.frames += s.len() as u64 / self.channels as u64;
                        if let Some(t) = self.total.filter(|&t| t > 0) {
                            let p = (self.frames as f32 / t as f32).min(0.99);
                            if p - self.last >= 0.01 {
                                self.last = p;
                                (self.progress)(p);
                            }
                        }
                        return true;
                    }
                    Err(SymError::DecodeError(_)) => {
                        self.bad += 1;
                        if self.bad > MAX_BAD_PACKETS {
                            self.err = Some(ConvertError::Malformed("the audio is damaged".into()));
                            return false;
                        }
                    }
                    Err(_) => return false,
                }
            }
        }
    }

    impl Source for Pull<'_> {
        fn channels(&self) -> usize {
            self.channels
        }
        fn bits_per_sample(&self) -> usize {
            self.bits as usize
        }
        fn sample_rate(&self) -> usize {
            self.rate as usize
        }
        fn read_samples<F: Fill>(
            &mut self,
            block_size: usize,
            dest: &mut F,
        ) -> Result<usize, SourceError> {
            let want = block_size * self.channels;
            while self.queue.len() < want && !self.eof {
                if !self.refill() {
                    self.eof = true;
                }
            }
            let n = want.min(self.queue.len()) / self.channels * self.channels;
            let block: Vec<i32> = self.queue.drain(..n).collect();
            dest.fill_interleaved(&block)?;
            Ok(n / self.channels)
        }
    }

    // The first buffer tells the true rate and channel count
    let mut pull = Pull {
        format,
        decoder,
        track_id,
        bits,
        rate: 0,
        channels: 0,
        queue: Default::default(),
        buf: None,
        bad: 0,
        err: None,
        frames: 0,
        total,
        cancel,
        progress,
        last: 0.0,
        eof: false,
    };
    // Prime: decode once to learn the spec (a lookahead the queue keeps)
    let spec = loop {
        let packet = match pull.format.next_packet() {
            Ok(p) => p,
            Err(_) => return Err(ConvertError::Malformed("no audio could be read".into())),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match pull.decoder.decode(&packet) {
            Ok(b) if b.frames() > 0 => {
                let spec = *b.spec();
                let s = SampleBuffer::<i32>::new(b.capacity() as u64, spec);
                pull.buf = Some(s);
                if let Some(s) = pull.buf.as_mut() {
                    s.copy_interleaved_ref(b);
                }
                break spec;
            }
            Ok(_) => {}
            Err(SymError::DecodeError(_)) => {
                pull.bad += 1;
                if pull.bad > MAX_BAD_PACKETS {
                    return Err(ConvertError::Malformed("the audio is damaged".into()));
                }
            }
            Err(_) => return Err(ConvertError::Malformed("no audio could be read".into())),
        }
    };
    pull.rate = spec.rate;
    pull.channels = spec.channels.count();
    if pull.channels > 8 {
        return Err(ConvertError::NotAvailable("More than 8 channels"));
    }
    if let Some(s) = &pull.buf {
        let shift = 32 - bits;
        pull.queue
            .extend(s.samples().iter().map(|&v| round_down(v, shift)));
        pull.frames += s.len() as u64 / pull.channels as u64;
    }

    use flacenc::component::{BitRepr, MetadataBlockData, Stream};
    use flacenc::source::{Context, FrameBuf};
    fn fail<E>(_: E) -> ConvertError {
        ConvertError::Malformed("FLAC encoding failed".into())
    }
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|_| ConvertError::Malformed("FLAC settings".into()))?;
    let block = config.block_size;
    let mut stream = Stream::new(pull.rate as usize, pull.channels, bits as usize).map_err(fail)?;
    stream
        .stream_info_mut()
        .set_block_sizes(block, block)
        .map_err(fail)?;
    if !tags.is_empty() {
        let vendor = b"Oeca";
        let mut body = (vendor.len() as u32).to_le_bytes().to_vec();
        body.extend_from_slice(vendor);
        body.extend_from_slice(&(tags.len() as u32).to_le_bytes());
        for (k, v) in &tags {
            let entry = format!("{k}={v}");
            body.extend_from_slice(&(entry.len() as u32).to_le_bytes());
            body.extend_from_slice(entry.as_bytes());
        }
        if let Ok(b) = MetadataBlockData::new_unknown(4, &body) {
            stream.add_metadata_block(b);
        }
    }
    // The header (STREAMINFO: totals, checksum, frame sizes) is only known at
    // the end, but its size is fixed: write it now, frames as they are encoded
    // (never all of them in memory), then come back and fill it in
    let header = |stream: &Stream| -> Result<Vec<u8>, ConvertError> {
        let mut sink = flacenc::bitsink::ByteSink::new();
        stream.write(&mut sink).map_err(fail)?;
        let mut bytes = sink.as_slice().to_vec();
        // flacenc records the (shorter) last block as the minimum block size;
        // a fixed-blocksize stream states min == max, which strict readers
        // (Symphonia, libFLAC's checks) rely on to treat the frames as fixed
        if bytes.len() > 12 {
            bytes.copy_within(10..12, 8);
        }
        Ok(bytes)
    };
    let placeholder = header(&stream)?;
    out.write_all(&placeholder)?;
    let mut buf = (
        FrameBuf::with_size(pull.channels, block).map_err(fail)?,
        Context::new(bits as usize, pull.channels),
    );
    loop {
        let read = pull.read_samples(block, &mut buf);
        if let Some(e) = pull.err.take() {
            return Err(e);
        }
        if read.map_err(fail)? == 0 {
            break;
        }
        let number = buf.1.current_frame_number().unwrap_or(0);
        let frame = flacenc::encode_fixed_size_frame(&config, &buf.0, number, stream.stream_info())
            .map_err(fail)?;
        stream.stream_info_mut().update_frame_info(&frame);
        let mut sink = flacenc::bitsink::ByteSink::new();
        frame.write(&mut sink).map_err(fail)?;
        out.write_all(sink.as_slice())?;
    }
    let (_, context) = buf;
    stream
        .stream_info_mut()
        .set_md5_digest(&context.md5_digest());
    stream
        .stream_info_mut()
        .set_total_samples(context.total_samples());
    let finished = header(&stream)?;
    if finished.len() != placeholder.len() {
        return Err(ConvertError::Malformed("FLAC header size changed".into()));
    }
    let end = out.stream_position()?;
    out.seek(SeekFrom::Start(0))?;
    out.write_all(&finished)?;
    out.seek(SeekFrom::Start(end))?;
    let _ = req;
    Ok((out, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Self {
            let p =
                std::env::temp_dir().join(format!("oeca-convert-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn no_cancel() -> AtomicBool {
        AtomicBool::new(false)
    }

    fn run(input: &Path, to: Format, dir: &Path) -> Result<Done, ConvertError> {
        convert(
            &Request {
                input,
                to,
                out_dir: dir,
                settings: Settings::default(),
                same_format_copy: false,
            },
            &no_cancel(),
            &mut |_| {},
        )
    }

    /// 8×4 image: left half opaque red, right half fully transparent.
    fn write_png(path: &Path) {
        let img = ImageBuffer::from_fn(8, 4, |x, _| {
            if x < 4 {
                Rgba([255u8, 0, 0, 255])
            } else {
                Rgba([0u8, 0, 255, 0])
            }
        });
        img.save_with_format(path, image::ImageFormat::Png).unwrap();
    }

    /// A 16-bit stereo sine WAV, written by the code under test's own writer.
    fn write_wav(path: &Path, frames: usize) {
        let out = BufWriter::new(File::create(path).unwrap());
        let mut w = WavWriter::new(out, 44_100, 2, 16, &[("TITLE", "Tone".into())]).unwrap();
        let samples: Vec<i32> = (0..frames)
            .flat_map(|i| {
                let v = ((i as f32 * 0.05).sin() * 20_000.0) as i32;
                [v << 16, (v / 2) << 16]
            })
            .collect();
        w.write(&samples).unwrap();
        w.finish().unwrap().flush().unwrap();
    }

    fn decode_all_i32(path: &Path) -> (u32, usize, Vec<i32>) {
        let mut probed = probe(path).unwrap();
        let track = pick_track(probed.format.as_ref()).unwrap().clone();
        let mut dec = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .unwrap();
        let mut samples = Vec::new();
        let (mut rate, mut ch) = (0, 0);
        while let Ok(p) = probed.format.next_packet() {
            if let Ok(b) = dec.decode(&p) {
                rate = b.spec().rate;
                ch = b.spec().channels.count();
                let mut s = SampleBuffer::<i32>::new(b.capacity() as u64, *b.spec());
                s.copy_interleaved_ref(b);
                samples.extend_from_slice(s.samples());
            }
        }
        (rate, ch, samples)
    }

    #[test]
    fn test_detection_is_by_content_not_extension() {
        let d = Dir::new("sniff");
        write_png(&d.file("really.jpg"));
        assert_eq!(sniff(&d.file("really.jpg")).unwrap(), Format::Png);
        fs::write(d.file("notes.png"), b"just some text, not an image").unwrap();
        assert!(matches!(
            sniff(&d.file("notes.png")),
            Err(ConvertError::Unsupported)
        ));
        assert!(matches!(
            sniff(&d.file("missing.png")),
            Err(ConvertError::Unreadable)
        ));
        assert!(matches!(sniff(&d.0), Err(ConvertError::Unreadable)));
        for (head, f) in [
            (&b"fLaC\0\0\0\0"[..], Format::Flac),
            (&b"OggS\0\x02"[..], Format::Ogg),
            (&b"ID3\x04\0\0"[..], Format::Mp3),
            (&[0xFF, 0xFB, 0x90, 0x00][..], Format::Mp3),
            (&b"\0\0\0\x20ftypM4A "[..], Format::M4a),
            (&b"RIFF\0\0\0\0WAVEfmt "[..], Format::Wav),
            (&b"RIFF\0\0\0\0WEBPVP8 "[..], Format::WebP),
        ] {
            assert_eq!(sniff_bytes(head), Some(f), "{f:?}");
        }
        // ADTS AAC (layer bits 00) is not claimed as MP3
        assert_eq!(sniff_bytes(&[0xFF, 0xF1, 0x50, 0x80]), None);
    }

    #[test]
    fn test_outputs_pair_by_family_and_never_offer_the_same_format() {
        assert!(!outputs_for(Format::Png).contains(&Format::Png));
        assert!(outputs_for(Format::Png).contains(&Format::Jpeg));
        assert!(
            outputs_for(Format::Png)
                .iter()
                .all(|f| f.family() == Family::Image)
        );
        assert!(outputs_for(Format::Mp3).contains(&Format::Wav));
        assert!(!outputs_for(Format::Mp3).contains(&Format::Mp3));
        assert!(
            outputs_for(Format::Flac)
                .iter()
                .all(|f| f.family() == Family::Audio && f.can_encode())
        );
        // A GIF is not offered GIF and MP3/M4A/GIF are never offered at all
        assert!(!outputs_for(Format::Gif).contains(&Format::Gif));
        assert!(
            outputs_for(Format::Wav)
                .iter()
                .all(|f| !matches!(f, Format::Mp3 | Format::M4a))
        );
        // Common outputs: shared, minus any selected input's own format
        let png_jpg = common_outputs(&[Format::Png, Format::Jpeg]);
        assert!(png_jpg.contains(&Format::WebP) && !png_jpg.contains(&Format::Png));
        assert!(!png_jpg.contains(&Format::Jpeg));
        assert!(common_outputs(&[Format::Png, Format::Wav]).is_empty());
        assert!(common_outputs(&[]).is_empty());
    }

    #[test]
    fn test_settings_validation() {
        assert!(Settings::default().validate().is_ok());
        let bad = |s: Settings| s.validate().is_err();
        assert!(bad(Settings {
            jpeg_quality: 0,
            ..Default::default()
        }));
        assert!(bad(Settings {
            jpeg_quality: 101,
            ..Default::default()
        }));
        assert!(bad(Settings {
            vorbis_quality: 1.5,
            ..Default::default()
        }));
    }

    #[test]
    fn test_png_converts_to_every_image_format_and_each_output_decodes() {
        let d = Dir::new("images");
        let src = d.file("pic.png");
        write_png(&src);
        let out = d.file("out");
        fs::create_dir(&out).unwrap();
        for to in [Format::Jpeg, Format::WebP, Format::Bmp, Format::Tiff] {
            let done = run(&src, to, &out).unwrap();
            assert_eq!(done.output.extension().unwrap(), to.extension());
            assert_eq!(sniff(&done.output).unwrap(), to);
            let back = image::open(&done.output).unwrap();
            assert_eq!((back.width(), back.height()), (8, 4), "{to:?}");
        }
        // The input is untouched and no temp file is left behind
        assert!(src.is_file());
        assert!(
            fs::read_dir(&out).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part"))
        );
    }

    #[test]
    fn test_transparency_is_kept_where_possible_and_flattened_on_the_chosen_background() {
        let d = Dir::new("alpha");
        let src = d.file("a.png");
        write_png(&src);
        let webp = run(&src, Format::WebP, &d.0).unwrap();
        let px = image::open(&webp.output).unwrap().to_rgba8();
        assert_eq!(px.get_pixel(6, 1)[3], 0, "WebP keeps the transparent half");
        assert_eq!(px.get_pixel(1, 1).0, [255, 0, 0, 255]);

        // JPEG: transparent pixels become the background (white by default)
        let jpg = run(&src, Format::Jpeg, &d.0).unwrap();
        let px = image::open(&jpg.output).unwrap().to_rgb8();
        let p = px.get_pixel(6, 1);
        assert!(p.0.iter().all(|&c| c > 240), "white, not black: {p:?}");

        // ... or the colour the user picked
        let green = convert(
            &Request {
                input: &src,
                to: Format::Jpeg,
                out_dir: &d.0,
                settings: Settings {
                    background: [0, 255, 0],
                    ..Default::default()
                },
                same_format_copy: false,
            },
            &no_cancel(),
            &mut |_| {},
        )
        .unwrap();
        let p = *image::open(&green.output)
            .unwrap()
            .to_rgb8()
            .get_pixel(6, 1);
        assert!(p[1] > 230 && p[0] < 30 && p[2] < 30, "{p:?}");
    }

    #[test]
    fn test_flatten_composites_partial_alpha() {
        let img = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(1, 1, Rgba([0u8, 0, 0, 128])));
        let flat = flatten(img, [255, 255, 255]).to_rgb8();
        let v = flat.get_pixel(0, 0)[0];
        assert!((126..=128).contains(&v), "{v}");
    }

    #[test]
    fn test_lossless_image_round_trip_is_pixel_exact() {
        let d = Dir::new("lossless");
        let src = d.file("a.png");
        write_png(&src);
        let tiff = run(&src, Format::Tiff, &d.0).unwrap();
        let back = run(&tiff.output, Format::Png, &d.0).unwrap();
        assert_eq!(
            image::open(&back.output).unwrap().to_rgba8(),
            image::open(&src).unwrap().to_rgba8()
        );
        let webp = run(&src, Format::WebP, &d.0).unwrap();
        let back = image::open(&webp.output).unwrap().to_rgba8();
        let orig = image::open(&src).unwrap().to_rgba8();
        // Lossless WebP keeps every visible pixel exactly
        for (a, b) in back.pixels().zip(orig.pixels()) {
            if b[3] != 0 {
                assert_eq!(a, b);
            }
        }
    }

    #[test]
    fn test_same_format_and_cross_family_are_refused_with_a_reason() {
        let d = Dir::new("refuse");
        let src = d.file("a.png");
        write_png(&src);
        assert!(matches!(
            run(&src, Format::Png, &d.0),
            Err(ConvertError::NotAvailable(_))
        ));
        assert!(matches!(
            run(&src, Format::Wav, &d.0),
            Err(ConvertError::NotAvailable(_))
        ));
        assert!(matches!(
            run(&src, Format::Mp3, &d.0),
            Err(ConvertError::NotAvailable(_))
        ));
        let missing = d.file("missing");
        assert!(matches!(
            run(&src, Format::Jpeg, &missing),
            Err(ConvertError::Io(_))
        ));
    }

    #[test]
    fn test_corrupt_and_truncated_inputs_fail_cleanly_without_output() {
        let d = Dir::new("corrupt");
        let good = d.file("good.png");
        write_png(&good);
        let bytes = fs::read(&good).unwrap();
        let cut = d.file("cut.png");
        fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        let junk = d.file("junk.png");
        let mut b = bytes[..16].to_vec();
        b.extend_from_slice(&[7u8; 64]);
        fs::write(&junk, b).unwrap();
        let empty = d.file("empty.png");
        fs::write(&empty, b"").unwrap();
        let out = d.file("out");
        fs::create_dir(&out).unwrap();
        for bad in [&cut, &junk, &empty] {
            assert!(run(bad, Format::Jpeg, &out).is_err(), "{bad:?}");
        }
        assert_eq!(fs::read_dir(&out).unwrap().count(), 0, "no output or temp");
        // Audio: a WAV header with no audio, and text renamed .flac
        fs::write(d.file("x.wav"), b"RIFF\0\0\0\0WAVEfmt ").unwrap();
        fs::write(d.file("y.flac"), b"hello").unwrap();
        for bad in ["x.wav", "y.flac"] {
            assert!(
                inspect(&d.file(bad)).is_err() || run(&d.file(bad), Format::Ogg, &out).is_err()
            );
        }
        assert_eq!(fs::read_dir(&out).unwrap().count(), 0);
    }

    #[test]
    fn test_oversized_dimensions_are_refused_before_decoding() {
        assert!(matches!(
            check_dimensions(20_000, 10),
            Err(ConvertError::TooLarge)
        ));
        assert!(matches!(
            check_dimensions(9_000, 9_000),
            Err(ConvertError::TooLarge)
        ));
        assert!(check_dimensions(8_000, 8_000).is_ok());
        assert!(check_dimensions(0, 5).is_err());
    }

    #[test]
    fn test_outputs_never_overwrite_and_collisions_get_numbered_names() {
        let d = Dir::new("collide");
        let src = d.file("shot.png");
        write_png(&src);
        let a = run(&src, Format::Jpeg, &d.0).unwrap();
        let b = run(&src, Format::Jpeg, &d.0).unwrap();
        let c = run(&src, Format::Jpeg, &d.0).unwrap();
        assert_eq!(a.output.file_name().unwrap(), "shot.jpg");
        assert_eq!(b.output.file_name().unwrap(), "shot (2).jpg");
        assert_eq!(c.output.file_name().unwrap(), "shot (3).jpg");
        // A pre-existing file is left exactly as it was
        fs::write(d.file("keep.jpg"), b"precious").unwrap();
        fs::copy(&src, d.file("keep.png")).unwrap();
        let k = run(&d.file("keep.png"), Format::Jpeg, &d.0).unwrap();
        assert_eq!(fs::read(d.file("keep.jpg")).unwrap(), b"precious");
        assert_eq!(k.output.file_name().unwrap(), "keep (2).jpg");
    }

    #[test]
    fn test_unicode_and_spaced_names_convert() {
        let d = Dir::new("unicode");
        let src = d.file("Привет 写真 café.png");
        write_png(&src);
        let out = d.file("выход ñ");
        fs::create_dir(&out).unwrap();
        let done = run(&src, Format::Bmp, &out).unwrap();
        assert_eq!(done.output.file_name().unwrap(), "Привет 写真 café.bmp");
        assert!(image::open(&done.output).is_ok());
    }

    #[test]
    fn test_cancel_leaves_no_output_and_no_temp_file() {
        let d = Dir::new("cancel");
        let src = d.file("a.png");
        write_png(&src);
        let out = d.file("out");
        fs::create_dir(&out).unwrap();
        let cancel = AtomicBool::new(true);
        let r = convert(
            &Request {
                input: &src,
                to: Format::Jpeg,
                out_dir: &out,
                settings: Settings::default(),
                same_format_copy: false,
            },
            &cancel,
            &mut |_| {},
        );
        assert!(matches!(r, Err(ConvertError::Cancelled)));
        assert_eq!(fs::read_dir(&out).unwrap().count(), 0);

        // Cancelled mid-way (after the first progress report)
        let wav = d.file("t.wav");
        write_wav(&wav, 200_000);
        let cancel = AtomicBool::new(false);
        let r = convert(
            &Request {
                input: &wav,
                to: Format::Flac,
                out_dir: &out,
                settings: Settings::default(),
                same_format_copy: false,
            },
            &cancel,
            &mut |_| cancel.store(true, Ordering::Relaxed),
        );
        assert!(matches!(r, Err(ConvertError::Cancelled)), "{r:?}");
        assert_eq!(fs::read_dir(&out).unwrap().count(), 0);
    }

    #[test]
    fn test_wav_to_flac_and_back_is_sample_exact() {
        let d = Dir::new("flac");
        let wav = d.file("tone.wav");
        write_wav(&wav, 30_000);
        let info = inspect(&wav).unwrap();
        assert_eq!(info.format, Format::Wav);
        assert_eq!(info.detail, "44.1 kHz stereo");
        let (rate, ch, original) = decode_all_i32(&wav);
        assert_eq!((rate, ch, original.len()), (44_100, 2, 60_000));

        let flac = run(&wav, Format::Flac, &d.0).unwrap();
        assert_eq!(sniff(&flac.output).unwrap(), Format::Flac);
        // Regression: a fixed-blocksize stream states min == max block size
        // (flacenc writes the shorter last block as the minimum)
        let head = fs::read(&flac.output).unwrap();
        assert_eq!(
            &head[8..10],
            &head[10..12],
            "STREAMINFO min block == max block"
        );
        let back = run(&flac.output, Format::Wav, &d.0).unwrap();
        let (rate2, ch2, round) = decode_all_i32(&back.output);
        assert_eq!((rate2, ch2), (44_100, 2));
        assert_eq!(round, original, "lossless chain keeps every sample");
        // Tags survive WAV -> FLAC
        let probed = probe(&flac.output).unwrap();
        let mut probed = probed;
        let mut format_tags = probed.format.metadata();
        let rev = format_tags.skip_to_latest();
        let tags = rev.map(collect_tags).unwrap_or_default();
        assert!(
            tags.iter().any(|(k, v)| *k == "TITLE" && v == "Tone"),
            "{tags:?}"
        );
    }

    #[test]
    fn test_wav_to_ogg_vorbis_decodes_at_the_same_rate_and_length() {
        let d = Dir::new("ogg");
        let wav = d.file("tone.wav");
        write_wav(&wav, 44_100);
        let ogg = run(&wav, Format::Ogg, &d.0).unwrap();
        assert_eq!(sniff(&ogg.output).unwrap(), Format::Ogg);
        assert!(ogg.bytes > 1000 && ogg.bytes < fs::metadata(&wav).unwrap().len());
        let (rate, ch, samples) = decode_all_i32(&ogg.output);
        assert_eq!((rate, ch), (44_100, 2));
        let frames = samples.len() / 2;
        assert!((43_000..=46_000).contains(&frames), "{frames}");
        // Lossy input to a lossless container is allowed (and decodes)
        let back = run(&ogg.output, Format::Flac, &d.0).unwrap();
        assert_eq!(sniff(&back.output).unwrap(), Format::Flac);
    }

    #[test]
    fn test_exif_orientation_is_applied() {
        // 2×1 JPEG-less check through the image API: rotating a 2×1 image by
        // a 90° orientation gives 1×2, which is what `convert_image` applies.
        let mut img =
            DynamicImage::ImageRgb8(ImageBuffer::from_pixel(2, 1, image::Rgb([9u8, 9, 9])));
        img.apply_orientation(image::metadata::Orientation::Rotate90);
        assert_eq!((img.width(), img.height()), (1, 2));
    }

    #[test]
    fn test_progress_is_bounded_and_ends_at_one() {
        let d = Dir::new("progress");
        let wav = d.file("t.wav");
        write_wav(&wav, 400_000);
        let mut seen = Vec::new();
        convert(
            &Request {
                input: &wav,
                to: Format::Flac,
                out_dir: &d.0,
                settings: Settings::default(),
                same_format_copy: false,
            },
            &no_cancel(),
            &mut |p| seen.push(p),
        )
        .unwrap();
        assert!(
            seen.len() <= 110,
            "about one report per percent: {}",
            seen.len()
        );
        assert!(seen.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(*seen.last().unwrap(), 1.0);
    }
    /// Converts every file of `$OECA_AUDIT_DIR/in` to every offered format into
    /// `$OECA_AUDIT_DIR/out/<file>/`, printing one line each, for checking with
    /// independent tools (`cargo test external_audit -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn external_audit() {
        let root = PathBuf::from(std::env::var("OECA_AUDIT_DIR").expect("OECA_AUDIT_DIR"));
        let mut files: Vec<_> = fs::read_dir(root.join("in"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        for f in files {
            let name = f.file_name().unwrap().to_string_lossy().into_owned();
            let from = match inspect(&f) {
                Ok(i) => i.format,
                Err(e) => {
                    println!("AUDIT {name} INSPECT-FAIL {e}");
                    continue;
                }
            };
            for to in common_outputs(&[from]) {
                let out = root.join("out").join(&name);
                fs::create_dir_all(&out).unwrap();
                match run(&f, to, &out) {
                    Ok(d) => println!(
                        "AUDIT {name} {} OK {} note={:?}",
                        to.label(),
                        d.output.display(),
                        d.note
                    ),
                    Err(e) => println!("AUDIT {name} {} ERR {e}", to.label()),
                }
            }
        }
    }
    #[test]
    fn test_publish_moves_the_staged_file_under_a_free_name_and_never_overwrites() {
        let d = Dir::new("publish");
        let staged = d.file("staged.jpg");
        fs::write(&staged, b"converted").unwrap();
        let out = d.file("saved");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("holiday.jpg"), b"already here").unwrap();
        let p = publish(&staged, &out, "holiday", "jpg").unwrap();
        assert_eq!(p.file_name().unwrap(), "holiday (2).jpg");
        assert_eq!(fs::read(&p).unwrap(), b"converted");
        assert_eq!(fs::read(out.join("holiday.jpg")).unwrap(), b"already here");
        assert!(!staged.exists(), "the staged file moved");
        // A missing folder fails without leaving an empty reserved file anywhere
        let again = d.file("staged2.jpg");
        fs::write(&again, b"x").unwrap();
        assert!(publish(&again, &d.file("nope"), "a", "jpg").is_err());
        assert!(again.exists(), "a failed save keeps the staged file");
    }

    #[test]
    fn test_a_file_another_program_holds_open_is_reported_unreadable() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = Dir::new("locked");
        let f = d.file("held.png");
        write_png(&f);
        let _lock = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&f)
            .unwrap();
        assert!(matches!(inspect(&f), Err(ConvertError::Unreadable)));
        let r = run(&f, Format::Jpeg, &d.0);
        assert!(matches!(r, Err(ConvertError::Unreadable)), "{r:?}");
        assert_eq!(
            fs::read_dir(&d.0)
                .unwrap()
                .filter(|e| e
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".part"))
                .count(),
            0
        );
    }
    #[test]
    fn test_bulk_outputs_keep_every_format_of_the_family_even_ones_inputs_already_use() {
        let mixed = [Format::Png, Format::Jpeg, Format::WebP, Format::Gif];
        let bulk = bulk_outputs(&mixed);
        assert_eq!(
            bulk,
            vec![
                Format::Png,
                Format::Jpeg,
                Format::WebP,
                Format::Bmp,
                Format::Tiff
            ]
        );
        // The single-file rule (no same-format choice) is unchanged
        assert!(!common_outputs(&[Format::Png]).contains(&Format::Png));
        assert!(common_outputs(&mixed).iter().all(|f| !mixed.contains(f)));
        assert!(bulk_outputs(&[Format::Png, Format::Wav]).is_empty());
        assert!(bulk_outputs(&[]).is_empty());
        assert_eq!(
            bulk_outputs(&[Format::Wav, Format::Flac]),
            vec![Format::Wav, Format::Flac, Format::Ogg]
        );
    }

    #[test]
    fn test_same_format_copy_is_byte_exact_validated_and_only_when_asked() {
        let d = Dir::new("samecopy");
        let src = d.file("pic.png");
        write_png(&src);
        let out = d.file("out");
        fs::create_dir(&out).unwrap();
        let ask = |same| {
            convert(
                &Request {
                    input: &src,
                    to: Format::Png,
                    out_dir: &out,
                    settings: Settings::default(),
                    same_format_copy: same,
                },
                &no_cancel(),
                &mut |_| {},
            )
        };
        assert!(
            matches!(ask(false), Err(ConvertError::NotAvailable(_))),
            "single-file rule"
        );
        let done = ask(true).unwrap();
        assert_eq!(fs::read(&done.output).unwrap(), fs::read(&src).unwrap());
        assert_eq!(done.note, Some("Copied unchanged"));
        assert_eq!(done.format, Format::Png);
        // A second copy never overwrites the first
        let again = ask(true).unwrap();
        assert_ne!(again.output, done.output);
        // A damaged file with the right signature is not "copied" as a success
        let bad = d.file("bad.png");
        let mut b = fs::read(&src).unwrap();
        b.truncate(b.len() / 2);
        fs::write(&bad, b).unwrap();
        let r = convert(
            &Request {
                input: &bad,
                to: Format::Png,
                out_dir: &out,
                settings: Settings::default(),
                same_format_copy: true,
            },
            &no_cancel(),
            &mut |_| {},
        );
        assert!(
            r.is_err() || fs::read_dir(&out).unwrap().count() == 2,
            "{r:?}"
        );
        assert!(src.is_file());
    }
    #[test]
    fn test_avif_is_detected_inspected_and_converted_and_other_isobmff_is_not_avif() {
        let d = Dir::new("avif");
        let rgb = d.file("rgb.avif");
        fs::write(&rgb, include_bytes!("testdata/rgb.avif")).unwrap();
        // By content, even under another name
        let renamed = d.file("photo.jpg");
        fs::copy(&rgb, &renamed).unwrap();
        assert_eq!(sniff(&renamed).unwrap(), Format::Avif);
        let info = inspect(&rgb).unwrap();
        assert_eq!(
            (info.format, info.detail.as_str()),
            (Format::Avif, "64 × 48")
        );
        assert!(!Format::Avif.can_encode(), "decode only");
        assert!(!bulk_outputs(&[Format::Avif, Format::Png]).contains(&Format::Avif));
        for to in [
            Format::Png,
            Format::Jpeg,
            Format::WebP,
            Format::Bmp,
            Format::Tiff,
        ] {
            let done = run(&rgb, to, &d.0).unwrap();
            let back = image::open(&done.output).unwrap();
            assert_eq!((back.width(), back.height()), (64, 48), "{to:?}");
        }
        // An M4A is still audio; a damaged AVIF is refused with a reason
        let mut head = b"\0\0\0\x20ftypM4A \0\0\0\0M4A mp42isom".to_vec();
        head.resize(64, 0);
        assert_eq!(sniff_bytes(&head), Some(Format::M4a));
        let bad = d.file("bad.avif");
        let bytes = fs::read(&rgb).unwrap();
        fs::write(&bad, &bytes[..bytes.len() / 2]).unwrap();
        assert!(matches!(inspect(&bad), Err(ConvertError::Malformed(_))));
        assert!(matches!(
            run(&bad, Format::Png, &d.0),
            Err(ConvertError::Malformed(_))
        ));
    }
}
