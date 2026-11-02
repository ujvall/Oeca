//! The Inbox's conversion batch: the dropped files, which are selected, what
//! they will become, and how each is doing. Pure state (no Win32, no drawing):
//! the window feeds it facts (an inspection, a progress report, a click) and
//! performs the actions it hands back. It is independent of the clipboard
//! history: a conversion drop never touches either.
//!
//! The engine is `convert`; this module orders its work. Files are inspected
//! and converted on worker threads (`spawn_inspect`, `spawn_convert`), one
//! conversion at a time (bounded memory), and report back by posting one
//! window message each, so the UI thread never waits on a file.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::convert::{self, ConvertError, Done, Family, Format, Info, Settings};

/// At most this many files in a batch (a drop is capped at 32 as well).
pub const MAX_ITEMS: usize = 64;
/// File rows visible at once; more scroll.
pub const VISIBLE_ROWS: usize = 3;

pub type ItemId = u32;

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// Being looked at by the inspection worker.
    Checking,
    Ready,
    /// Can't be converted (unsupported, damaged, unreadable, too large).
    Invalid(String),
    Queued,
    Converting(f32),
    /// Converted into the staging folder; not yet saved.
    Done {
        staged: PathBuf,
        bytes: u64,
        to: Format,
        note: Option<&'static str>,
    },
    Failed(String),
    Saved(PathBuf),
}

#[derive(Debug)]
pub struct Item {
    pub id: ItemId,
    pub path: PathBuf,
    pub name: String,
    pub info: Option<Info>,
    pub status: Status,
    pub selected: bool,
    /// Fading out: no longer part of the batch's logic, gone once faded.
    pub removing: bool,
    cancel: Arc<AtomicBool>,
}

impl Item {
    /// Can be (re)converted: ready, or failed before (a retry).
    pub fn convertible(&self) -> bool {
        matches!(self.status, Status::Ready | Status::Failed(_))
            && self.info.is_some()
            && !self.removing
    }

    fn busy(&self) -> bool {
        matches!(
            self.status,
            Status::Checking | Status::Queued | Status::Converting(_)
        )
    }
}

/// The quality / background choices the option chips cycle through.
pub const JPEG_QUALITIES: [u8; 4] = [95, 90, 80, 70];
pub const BACKGROUNDS: [(&str, [u8; 3]); 3] = [
    ("White", [255, 255, 255]),
    ("Black", [0, 0, 0]),
    ("Gray", [128, 128, 128]),
];
pub const VORBIS_QUALITIES: [(&str, f32); 3] = [("Small", 0.3), ("Standard", 0.5), ("High", 0.8)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub jpeg: usize,
    pub background: usize,
    pub vorbis: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            jpeg: 1,
            background: 0,
            vorbis: 1,
        }
    }
}

impl Options {
    pub fn settings(self) -> Settings {
        Settings {
            jpeg_quality: JPEG_QUALITIES[self.jpeg % JPEG_QUALITIES.len()],
            background: BACKGROUNDS[self.background % BACKGROUNDS.len()].1,
            vorbis_quality: VORBIS_QUALITIES[self.vorbis % VORBIS_QUALITIES.len()].1,
        }
    }
}

/// What the primary button does right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Primary {
    /// Convert the selected files.
    Convert,
    /// Stop the running conversion.
    Cancel,
    /// Save every converted file that is not saved yet.
    SaveAll,
    /// Everything is saved: close the batch.
    Done,
}

#[derive(Debug, Default)]
pub struct Batch {
    pub items: Vec<Item>,
    next_id: ItemId,
    /// The chosen output format (always one of `formats()`, or None).
    target: Option<Format>,
    pub options: Options,
    /// Where saved files go; None: beside each original.
    pub out_dir: Option<PathBuf>,
    /// First visible row.
    pub scroll: usize,
    /// The inline "discard unsaved files?" question is showing.
    pub confirming: bool,
    running: bool,
}

/// One conversion handed to the worker.
#[derive(Debug)]
pub struct Job {
    pub id: ItemId,
    pub input: PathBuf,
    pub to: Format,
    pub settings: Settings,
    /// Part of a bulk run: a file already in `to` is copied unchanged.
    pub same_format_copy: bool,
    cancel: Arc<AtomicBool>,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, id: ItemId) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    fn get_mut(&mut self, id: ItemId) -> Option<&mut Item> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    #[cfg(test)]
    pub fn running(&self) -> bool {
        self.running
    }

    pub fn selected_count(&self) -> usize {
        self.items.iter().filter(|i| i.selected).count()
    }

    fn selectable_count(&self) -> usize {
        self.items.iter().filter(|i| i.convertible()).count()
    }

    pub fn all_selected(&self) -> bool {
        let n = self.selectable_count();
        n > 0 && self.selected_count() == n
    }

    pub fn unsaved(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.status, Status::Done { .. }))
            .count()
    }

    pub fn target(&self) -> Option<Format> {
        self.target
    }

    /// Adds dropped files (those already in the batch are skipped, the batch
    /// is capped); returns the new items to inspect. They start unselected and
    /// are selected once inspection says they can convert.
    pub fn add(&mut self, paths: Vec<PathBuf>) -> Vec<(ItemId, PathBuf)> {
        let mut added = Vec::new();
        for path in paths {
            if self.items.len() >= MAX_ITEMS || self.items.iter().any(|i| i.path == path) {
                continue;
            }
            let id = self.next_id;
            self.next_id += 1;
            let name = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            self.items.push(Item {
                id,
                path: path.clone(),
                name,
                info: None,
                status: Status::Checking,
                selected: false,
                removing: false,
                cancel: Arc::new(AtomicBool::new(false)),
            });
            added.push((id, path));
        }
        added
    }

    /// An inspection came back: ready (and selected) or invalid with why.
    pub fn inspected(&mut self, id: ItemId, result: Result<Info, ConvertError>) {
        if let Some(item) = self.get_mut(id)
            && item.status == Status::Checking
        {
            match result {
                Ok(info) => {
                    item.info = Some(info);
                    item.status = Status::Ready;
                    item.selected = true;
                }
                Err(e) => item.status = Status::Invalid(e.to_string()),
            }
        }
        self.settle_target();
    }

    pub fn toggle(&mut self, id: ItemId) {
        if self.running {
            return;
        }
        if let Some(item) = self.get_mut(id)
            && item.convertible()
        {
            item.selected = !item.selected;
        }
        self.settle_target();
    }

    /// Select all, or deselect all when everything is already selected.
    pub fn toggle_all(&mut self) {
        if self.running {
            return;
        }
        let select = !self.all_selected();
        for item in self.items.iter_mut() {
            item.selected = select && item.convertible();
        }
        self.settle_target();
    }

    /// The formats of the selected files, deduplicated.
    fn selected_formats(&self) -> Vec<Format> {
        self.items
            .iter()
            .filter(|i| i.selected)
            .filter_map(|i| i.info.as_ref().map(|n| n.format))
            .collect()
    }

    /// Output formats every selected file can become (empty: nothing selected
    /// or mixed families).
    pub fn formats(&self) -> Vec<Format> {
        let selected = self.selected_formats();
        // A bulk selection is offered every format of its family: a file that
        // already has one is copied unchanged rather than hiding the option
        if self.is_bulk() {
            convert::bulk_outputs(&selected)
        } else {
            convert::common_outputs(&selected)
        }
    }

    /// Two or more files are selected (or a run of several is going).
    fn is_bulk(&self) -> bool {
        self.selected_count() >= 2
    }

    /// The selection mixes images and audio.
    pub fn mixed(&self) -> bool {
        let f = self.selected_formats();
        f.first()
            .is_some_and(|a| f.iter().any(|b| b.family() != a.family()))
    }

    /// Keeps `target` one of the offered formats; picks a typical default
    /// when it no longer is.
    fn settle_target(&mut self) {
        let offered = self.formats();
        if self.target.is_some_and(|t| offered.contains(&t)) {
            return;
        }
        const PREFERRED: [Format; 3] = [Format::Png, Format::Jpeg, Format::Wav];
        self.target = PREFERRED
            .into_iter()
            .find(|f| offered.contains(f))
            .or_else(|| offered.first().copied());
        // Lossless audio is converted to the lossless pair by default
        if offered.contains(&Format::Flac) && self.selected_formats().contains(&Format::Wav) {
            self.target = Some(Format::Flac);
        }
    }

    pub fn choose(&mut self, format: Format) {
        if !self.running && self.formats().contains(&format) {
            self.target = Some(format);
        }
    }

    pub fn cycle_jpeg_quality(&mut self) {
        self.options.jpeg = (self.options.jpeg + 1) % JPEG_QUALITIES.len();
    }

    pub fn cycle_background(&mut self) {
        self.options.background = (self.options.background + 1) % BACKGROUNDS.len();
    }

    pub fn cycle_vorbis_quality(&mut self) {
        self.options.vorbis = (self.options.vorbis + 1) % VORBIS_QUALITIES.len();
    }

    pub fn primary(&self) -> Primary {
        if self.running {
            Primary::Cancel
        } else if self.selected_count() > 0 && self.target.is_some() {
            Primary::Convert
        } else if self.unsaved() > 0 {
            Primary::SaveAll
        } else {
            Primary::Done
        }
    }

    /// Queues the selected files; returns the jobs for the worker (None when
    /// nothing can start). Deselects them: their rows now show their progress.
    pub fn start(&mut self) -> Option<Vec<Job>> {
        let to = self.target?;
        if self.running || !self.formats().contains(&to) {
            return None;
        }
        let settings = self.options.settings();
        let same_format_copy = self.is_bulk();
        let mut jobs = Vec::new();
        for item in self
            .items
            .iter_mut()
            .filter(|i| i.selected && i.convertible())
        {
            item.cancel = Arc::new(AtomicBool::new(false));
            item.status = Status::Queued;
            item.selected = false;
            jobs.push(Job {
                id: item.id,
                input: item.path.clone(),
                to,
                settings,
                same_format_copy,
                cancel: item.cancel.clone(),
            });
        }
        if jobs.is_empty() {
            return None;
        }
        self.running = true;
        Some(jobs)
    }

    pub fn progress(&mut self, id: ItemId, p: f32) {
        if let Some(item) = self.get_mut(id)
            && matches!(item.status, Status::Queued | Status::Converting(_))
        {
            item.status = Status::Converting(p.clamp(0.0, 1.0));
        }
    }

    /// A conversion ended. Returns a staged file to delete when the item is
    /// gone (removed or cleared while it ran).
    pub fn finished(&mut self, id: ItemId, result: Result<Done, ConvertError>) -> Option<PathBuf> {
        let to_delete = match self.get_mut(id) {
            Some(item) => {
                match result {
                    Ok(done) => {
                        item.status = Status::Done {
                            staged: done.output,
                            bytes: done.bytes,
                            to: done.format,
                            note: done.note,
                        };
                        None
                    }
                    Err(ConvertError::Cancelled) => {
                        // Back to the selection, so a restart picks it up
                        item.status = Status::Ready;
                        item.selected = true;
                        None
                    }
                    Err(e) => {
                        item.status = Status::Failed(e.to_string());
                        item.selected = false;
                        None
                    }
                }
            }
            None => result.ok().map(|d| d.output),
        };
        self.running = self
            .items
            .iter()
            .any(|i| matches!(i.status, Status::Queued | Status::Converting(_)));
        self.settle_target();
        to_delete
    }

    /// The whole run ended (the worker is done): anything still queued or
    /// converting (a worker that died) fails visibly instead of hanging.
    pub fn run_ended(&mut self) {
        for item in self.items.iter_mut() {
            if matches!(item.status, Status::Queued | Status::Converting(_)) {
                item.status = Status::Failed("Stopped".into());
            }
        }
        self.running = false;
    }

    /// Cancels everything queued or converting (they return to Ready).
    pub fn cancel_run(&self) {
        for item in &self.items {
            item.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Starts a file's removal: it stops counting at once (deselected, its
    /// conversion cancelled) and is dropped by `reap` once its row has faded.
    pub fn begin_remove(&mut self, id: ItemId) {
        if let Some(item) = self.get_mut(id) {
            item.removing = true;
            item.selected = false;
            item.cancel.store(true, Ordering::Relaxed);
        }
        self.settle_target();
    }

    /// Drops the fading items whose row `faded` says is gone; returns their
    /// unsaved staged outputs to delete.
    pub fn reap(&mut self, faded: impl Fn(ItemId) -> bool) -> Vec<PathBuf> {
        let ids: Vec<ItemId> = self
            .items
            .iter()
            .filter(|i| i.removing && faded(i.id))
            .map(|i| i.id)
            .collect();
        ids.into_iter().filter_map(|id| self.remove(id)).collect()
    }

    /// Removes a file from the batch (never deletes the original); returns
    /// its unsaved staged output for the caller to delete.
    pub fn remove(&mut self, id: ItemId) -> Option<PathBuf> {
        let at = self.items.iter().position(|i| i.id == id)?;
        let item = self.items.remove(at);
        item.cancel.store(true, Ordering::Relaxed);
        self.running = self
            .items
            .iter()
            .any(|i| matches!(i.status, Status::Queued | Status::Converting(_)));
        self.scroll = self
            .scroll
            .min(self.items.len().saturating_sub(VISIBLE_ROWS));
        self.settle_target();
        match item.status {
            Status::Done { staged, .. } => Some(staged),
            _ => None,
        }
    }

    /// Empties the batch; returns the staged outputs to delete.
    pub fn clear(&mut self) -> Vec<PathBuf> {
        self.cancel_run();
        let staged = self
            .items
            .drain(..)
            .filter_map(|i| match i.status {
                Status::Done { staged, .. } => Some(staged),
                _ => None,
            })
            .collect();
        self.target = None;
        self.scroll = 0;
        self.confirming = false;
        self.running = false;
        staged
    }

    /// Where `id` would be saved: the chosen folder, else beside the original.
    pub fn save_dir(&self, id: ItemId) -> Option<PathBuf> {
        match &self.out_dir {
            Some(d) => Some(d.clone()),
            None => self.get(id)?.path.parent().map(Path::to_path_buf),
        }
    }

    /// Marks a file failed with a reason (e.g. it could not be saved).
    pub fn fail(&mut self, id: ItemId, why: String) {
        if let Some(item) = self.get_mut(id) {
            item.status = Status::Failed(why);
        }
    }

    pub fn mark_saved(&mut self, id: ItemId, path: PathBuf) {
        if let Some(item) = self.get_mut(id) {
            item.status = Status::Saved(path);
        }
    }

    /// Items whose output waits to be saved.
    pub fn unsaved_ids(&self) -> Vec<ItemId> {
        self.items
            .iter()
            .filter(|i| matches!(i.status, Status::Done { .. }))
            .map(|i| i.id)
            .collect()
    }

    pub fn scroll_by(&mut self, rows: i32) -> bool {
        let max = self.items.len().saturating_sub(VISIBLE_ROWS) as i32;
        let next = (self.scroll as i32 + rows).clamp(0, max) as usize;
        std::mem::replace(&mut self.scroll, next) != next
    }

    /// Everything done and nothing waiting to be saved: closing loses nothing.
    pub fn safe_to_close(&self) -> bool {
        self.unsaved() == 0 && !self.running
    }

    /// "Q90" etc.: the chips for the chosen format.
    pub fn chips(&self) -> Vec<String> {
        match self.target {
            Some(Format::Jpeg) => vec![
                format!("Q {}", JPEG_QUALITIES[self.options.jpeg]),
                BACKGROUNDS[self.options.background].0.to_string(),
            ],
            Some(Format::Ogg) => vec![VORBIS_QUALITIES[self.options.vorbis].0.to_string()],
            _ => Vec::new(),
        }
    }

    /// Files sharing a name with another row (their folder is shown too).
    fn duplicate_name(&self, item: &Item) -> bool {
        self.items
            .iter()
            .any(|o| o.id != item.id && o.name.eq_ignore_ascii_case(&item.name))
    }

    /// The second line of a row.
    pub fn detail(&self, item: &Item) -> String {
        let size = |b: u64| human_size(b);
        let folder = || {
            item.path
                .parent()
                .and_then(Path::file_name)
                .map(|f| format!(" · in {}", f.to_string_lossy()))
                .unwrap_or_default()
        };
        match &item.status {
            Status::Checking => "Checking…".into(),
            Status::Ready => {
                let info = item.info.as_ref();
                let mut s = info.map_or_else(String::new, |i| {
                    let mut parts = vec![i.format.label().to_string()];
                    if !i.detail.is_empty() {
                        parts.push(i.detail.clone());
                    }
                    parts.push(size(i.bytes));
                    parts.join(" · ")
                });
                if self.duplicate_name(item) {
                    s.push_str(&folder());
                }
                s
            }
            Status::Invalid(why) | Status::Failed(why) => why.clone(),
            Status::Queued => "Waiting…".into(),
            Status::Converting(p) => format!("Converting {}%", (p * 100.0).round() as u32),
            Status::Done {
                bytes, to, note, ..
            } => {
                let mut s = format!("{} · {}", to.label(), size(*bytes));
                if let Some(n) = note {
                    s.push_str(" · ");
                    s.push_str(n);
                }
                s
            }
            Status::Saved(p) => format!(
                "Saved as {}",
                p.file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
            ),
        }
    }
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[unit])
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

// ---- What the renderer shows ----------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Normal,
    Busy,
    Done,
    Failed,
    Saved,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowView {
    pub id: ItemId,
    pub name: String,
    pub detail: String,
    pub family: Option<Family>,
    pub kind: RowKind,
    pub selected: bool,
    pub selectable: bool,
    /// Conversion progress (0..=1) while queued / converting.
    pub progress: Option<f32>,
    pub save: bool,
    pub show: bool,
    pub removing: bool,
}

/// A snapshot of the batch for drawing and hit-testing: the rows on show and
/// every label, so the renderer holds no batch logic.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub rows: Vec<RowView>,
    pub total: usize,
    pub scroll: usize,
    pub count_label: String,
    pub selected: usize,
    pub all_selected: bool,
    pub any_selectable: bool,
    pub formats: Vec<Format>,
    pub target: Option<Format>,
    /// Said in place of the format pills when there are none to offer.
    pub hint: Option<&'static str>,
    pub chips: Vec<String>,
    pub primary: Primary,
    pub primary_label: String,
    pub primary_enabled: bool,
    pub confirming: bool,
    pub confirm_message: String,
}

impl Batch {
    pub fn view(&self) -> View {
        let selected = self.selected_count();
        let primary = self.primary();
        let formats = self.formats();
        let hint = if !formats.is_empty() {
            None
        } else if self.mixed() {
            Some("Convert images and audio separately")
        } else if self.running {
            Some("Converting…")
        } else if primary == Primary::SaveAll {
            Some("Converted files are ready to save")
        } else if primary == Primary::Done
            && self
                .items
                .iter()
                .all(|i| matches!(i.status, Status::Saved(_)))
        {
            Some("All saved")
        } else {
            Some("Select files to convert")
        };
        let rows = self
            .items
            .iter()
            .skip(self.scroll)
            .take(VISIBLE_ROWS)
            .map(|i| RowView {
                id: i.id,
                name: i.name.clone(),
                detail: self.detail(i),
                family: i.info.as_ref().map(|n| n.format.family()).or_else(|| {
                    crate::dragdrop::is_audio(&i.path)
                        .then_some(Family::Audio)
                        .or(Some(Family::Image))
                }),
                kind: match i.status {
                    Status::Checking | Status::Ready => RowKind::Normal,
                    Status::Queued | Status::Converting(_) => RowKind::Busy,
                    Status::Done { .. } => RowKind::Done,
                    Status::Invalid(_) | Status::Failed(_) => RowKind::Failed,
                    Status::Saved(_) => RowKind::Saved,
                },
                selected: i.selected,
                selectable: i.convertible() && !self.running,
                progress: match i.status {
                    Status::Queued => Some(0.0),
                    Status::Converting(p) => Some(p),
                    _ => None,
                },
                save: matches!(i.status, Status::Done { .. }) && !i.removing,
                show: matches!(i.status, Status::Saved(_)) && !i.removing,
                removing: i.removing,
            })
            .collect();
        let n = self.selectable_count();
        let primary_label = match primary {
            Primary::Convert if n > 1 && selected == n => "Convert all".to_string(),
            Primary::Convert if selected > 1 => format!("Convert {selected}"),
            Primary::Convert => "Convert".to_string(),
            Primary::Cancel => "Cancel".to_string(),
            Primary::SaveAll if self.unsaved() > 1 => format!("Save all {}", self.unsaved()),
            Primary::SaveAll => "Save".to_string(),
            Primary::Done => "Done".to_string(),
        };
        let count_label = if self.running {
            let busy = self.items.iter().filter(|i| i.busy()).count();
            format!("Converting {busy}")
        } else if selected == 0 {
            "None selected".to_string()
        } else {
            format!("{selected} of {} selected", self.items.len())
        };
        let unsaved = self.unsaved();
        View {
            rows,
            total: self.items.len(),
            scroll: self.scroll,
            count_label,
            selected,
            all_selected: self.all_selected(),
            any_selectable: n > 0 && !self.running,
            formats,
            target: self.target,
            hint,
            chips: self.chips(),
            primary,
            primary_label,
            primary_enabled: true,
            confirming: self.confirming,
            confirm_message: if self.running {
                "Stop converting and close?".to_string()
            } else {
                format!(
                    "Discard {unsaved} unsaved file{}?",
                    if unsaved == 1 { "" } else { "s" }
                )
            },
        }
    }
}

// ---- Workers --------------------------------------------------------------------

/// What a worker posts back (lParam: a `Box<Update>` taken with `Box::from_raw`).
#[derive(Debug)]
pub enum Update {
    Inspected(ItemId, Result<Info, ConvertError>),
    Progress(ItemId, f32),
    Finished(ItemId, Result<Done, ConvertError>),
    /// The worker has nothing left to do.
    Ended,
}

fn post(hwnd: isize, msg: u32, update: Update) {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    let raw = Box::into_raw(Box::new(update));
    let hwnd = HWND(hwnd as *mut _);
    if unsafe { PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(raw as isize)) }.is_err() {
        // SAFETY: never posted, so still owned here
        drop(unsafe { Box::from_raw(raw) });
    }
}

/// Inspects dropped files off the UI thread (`Update::Inspected` each).
pub fn spawn_inspect(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    files: Vec<(ItemId, PathBuf)>,
) -> bool {
    let target = hwnd.0 as isize;
    std::thread::Builder::new()
        .name("oeca-inspect".into())
        .spawn(move || {
            for (id, path) in files {
                post(target, msg, Update::Inspected(id, convert::inspect(&path)));
            }
        })
        .is_ok()
}

/// Converts `jobs` one after another off the UI thread into `staging`,
/// posting progress at most ~20 times a second per file, then `Update::Ended`.
pub fn spawn_convert(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    jobs: Vec<Job>,
    staging: PathBuf,
) -> bool {
    let target = hwnd.0 as isize;
    std::thread::Builder::new()
        .name("oeca-convert".into())
        .spawn(move || {
            let _ = std::fs::create_dir_all(&staging);
            for job in jobs {
                if job.cancel.load(Ordering::Relaxed) {
                    post(
                        target,
                        msg,
                        Update::Finished(job.id, Err(ConvertError::Cancelled)),
                    );
                    continue;
                }
                post(target, msg, Update::Progress(job.id, 0.0));
                let mut last = Instant::now();
                let id = job.id;
                let request = convert::Request {
                    input: &job.input,
                    to: job.to,
                    out_dir: &staging,
                    settings: job.settings,
                    same_format_copy: job.same_format_copy,
                };
                let result = convert::convert(&request, &job.cancel, &mut |p| {
                    if last.elapsed() >= Duration::from_millis(50) {
                        last = Instant::now();
                        post(target, msg, Update::Progress(id, p));
                    }
                });
                post(target, msg, Update::Finished(id, result));
            }
            post(target, msg, Update::Ended);
        })
        .is_ok()
}

/// Asks for a folder in the system dialog, on a short-lived thread of its own
/// (the dialog runs its own message loop); posts the choice to `hwnd` as `msg`
/// (lParam: a `Box<Option<PathBuf>>`, taken back with `Box::from_raw`). False if
/// the thread could not start.
pub fn pick_folder_async(hwnd: windows::Win32::Foundation::HWND, msg: u32) -> bool {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::UI::Shell::{
        FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH,
    };
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    let target = hwnd.0 as isize;
    std::thread::Builder::new()
        .name("oeca-folder".into())
        .spawn(move || {
            let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
            let choice = (|| -> windows::core::Result<PathBuf> {
                unsafe {
                    let dialog: IFileOpenDialog =
                        CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
                    dialog
                        .SetOptions(dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)?;
                    dialog.SetTitle(windows::core::w!("Save converted files in"))?;
                    dialog.Show(None)?;
                    let item = dialog.GetResult()?;
                    let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
                    let path = PathBuf::from(name.to_string()?);
                    CoTaskMemFree(Some(name.0 as *const _));
                    Ok(path)
                }
            })()
            .ok();
            if com {
                unsafe { CoUninitialize() };
            }
            let raw = Box::into_raw(Box::new(choice));
            let hwnd = HWND(target as *mut _);
            if unsafe { PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(raw as isize)) }.is_err() {
                // SAFETY: never posted, so still owned here
                drop(unsafe { Box::from_raw(raw) });
            }
        })
        .is_ok()
}

/// Opens Explorer on the folder of `path` with the file selected.
pub fn show_in_folder(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{SHOpenFolderAndSelectItems, SHParseDisplayName};
    use windows::core::PCWSTR;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    unsafe {
        let mut pidl = std::ptr::null_mut();
        if SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None).is_ok() {
            let _ = SHOpenFolderAndSelectItems(pidl, None, 0);
            CoTaskMemFree(Some(pidl as *const _));
        }
    }
}

/// Removes the staging folders (`Oeca-convert-<pid>` in the temp folder) that
/// crashed or killed Oeca instances left behind. Only folders whose process is
/// gone are touched, only their plain files are deleted (never a symlink or
/// subfolder), and the folder itself goes only if that emptied it.
pub fn cleanup_stale_staging() {
    cleanup_stale_in(&std::env::temp_dir(), std::process::id());
}

pub fn cleanup_stale_in(root: &Path, own_pid: u32) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|n| n.strip_prefix("Oeca-convert-"))
            .and_then(|p| p.parse::<u32>().ok())
        else {
            continue;
        };
        let dir = entry.path();
        let is_dir = entry
            .file_type()
            .is_ok_and(|t| t.is_dir() && !t.is_symlink());
        if pid == own_pid || !is_dir || process_alive(pid) {
            continue;
        }
        if let Ok(files) = std::fs::read_dir(&dir) {
            for f in files.flatten() {
                if f.file_type().is_ok_and(|t| t.is_file()) {
                    let _ = std::fs::remove_file(f.path());
                }
            }
        }
        if std::fs::remove_dir(&dir).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Whether process `pid` still runs (an unreadable one counts as running).
fn process_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(h) => {
                let mut code = 0u32;
                let alive = GetExitCodeProcess(h, &mut code).is_err() || code == 259;
                let _ = CloseHandle(h);
                alive
            }
            Err(e) => e.code() != ERROR_INVALID_PARAMETER.to_hresult(),
        }
    }
}

/// The folder staged outputs live in until saved.
pub fn staging_dir() -> PathBuf {
    std::env::temp_dir().join(format!("Oeca-convert-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(format: Format) -> Info {
        Info {
            format,
            bytes: 2048,
            detail: "10 × 10".into(),
        }
    }

    fn batch_of(files: &[(&str, Format)]) -> Batch {
        let mut b = Batch::default();
        let added = b.add(
            files
                .iter()
                .map(|(n, _)| PathBuf::from(format!("C:/in/{n}")))
                .collect(),
        );
        for ((id, _), (_, f)) in added.into_iter().zip(files) {
            b.inspected(id, Ok(info(*f)));
        }
        b
    }

    #[test]
    fn test_dropped_files_become_selected_rows_once_inspected() {
        let mut b = Batch::default();
        let added = b.add(vec![
            "C:/in/a.png".into(),
            "C:/in/b.mp3".into(),
            "C:/in/c.txt".into(),
        ]);
        assert_eq!(added.len(), 3);
        assert!(
            b.items
                .iter()
                .all(|i| i.status == Status::Checking && !i.selected)
        );
        b.inspected(0, Ok(info(Format::Png)));
        b.inspected(1, Ok(info(Format::Mp3)));
        b.inspected(2, Err(ConvertError::Unsupported));
        assert_eq!(b.selected_count(), 2);
        assert!(matches!(b.items[2].status, Status::Invalid(_)));
        assert_eq!(b.detail(&b.items[2]), "Unsupported format");
        // The invalid file can't be selected, by click or by Select all
        b.toggle(2);
        assert!(!b.items[2].selected);
        b.toggle_all();
        assert_eq!(
            b.selected_count(),
            0,
            "all selectable were selected: now none"
        );
        b.toggle_all();
        assert_eq!(b.selected_count(), 2);
        assert!(b.all_selected());
    }

    #[test]
    fn test_duplicates_are_skipped_and_same_names_show_their_folder() {
        let mut b = Batch::default();
        assert_eq!(
            b.add(vec!["C:/a/x.png".into(), "C:/b/x.png".into()]).len(),
            2
        );
        assert_eq!(b.add(vec!["C:/a/x.png".into()]).len(), 0, "same path twice");
        b.inspected(0, Ok(info(Format::Png)));
        b.inspected(1, Ok(info(Format::Png)));
        assert!(b.detail(&b.items[0]).ends_with("in a"));
        assert!(b.detail(&b.items[1]).ends_with("in b"));
        // The batch is bounded
        let many = (0..MAX_ITEMS + 10)
            .map(|i| PathBuf::from(format!("C:/m/{i}.png")))
            .collect();
        b.add(many);
        assert_eq!(b.items.len(), MAX_ITEMS);
    }

    #[test]
    fn test_offered_formats_follow_the_selection_and_mixed_families_are_explained() {
        let mut b = batch_of(&[("a.png", Format::Png), ("b.jpg", Format::Jpeg)]);
        let f = b.formats();
        assert!(
            f.contains(&Format::WebP) && f.contains(&Format::Png) && f.contains(&Format::Jpeg),
            "a bulk batch keeps the formats its files already are"
        );
        assert!(b.target().is_some_and(|t| f.contains(&t)));
        b.toggle(1);
        assert!(
            b.formats().contains(&Format::Jpeg) && !b.formats().contains(&Format::Png),
            "now only the PNG is selected: the single-file rule"
        );
        let mut m = batch_of(&[("a.png", Format::Png), ("b.wav", Format::Wav)]);
        assert!(m.mixed() && m.formats().is_empty() && m.target().is_none());
        assert_eq!(m.primary(), Primary::Done, "nothing convertible to start");
        m.toggle(0);
        assert!(!m.mixed());
        assert!(m.formats().contains(&Format::Flac));
        assert_eq!(
            m.target(),
            Some(Format::Flac),
            "WAV defaults to its lossless pair"
        );
        m.choose(Format::Png);
        assert_eq!(
            m.target(),
            Some(Format::Flac),
            "a format the selection can't make is refused"
        );
    }

    #[test]
    fn test_a_run_walks_through_queued_converting_done_and_saved() {
        let mut b = batch_of(&[("a.png", Format::Png), ("b.png", Format::Png)]);
        b.choose(Format::Jpeg);
        assert_eq!(b.primary(), Primary::Convert);
        let jobs = b.start().unwrap();
        assert_eq!(jobs.len(), 2);
        assert!(b.running() && b.primary() == Primary::Cancel);
        assert!(b.start().is_none(), "no second run while one is going");
        b.progress(0, 0.5);
        assert_eq!(b.items[0].status, Status::Converting(0.5));
        assert_eq!(b.detail(&b.items[0]), "Converting 50%");
        b.toggle(0);
        assert!(!b.items[0].selected, "selection is frozen during a run");
        let done = |p: &str| Done {
            output: PathBuf::from(p),
            bytes: 900,
            format: Format::Jpeg,
            note: None,
        };
        assert_eq!(b.finished(0, Ok(done("C:/s/a.jpg"))), None);
        b.finished(1, Err(ConvertError::Malformed("bad".into())));
        b.run_ended();
        assert!(!b.running());
        assert!(matches!(b.items[0].status, Status::Done { .. }));
        assert!(
            matches!(&b.items[1].status, Status::Failed(m) if m.to_lowercase().contains("bad"))
        );
        assert_eq!(b.unsaved(), 1);
        assert!(!b.safe_to_close());
        // The failed file can be retried; the rest waits to be saved
        b.toggle(1);
        assert_eq!(b.primary(), Primary::Convert);
        b.toggle(1);
        assert_eq!(b.primary(), Primary::SaveAll);
        b.mark_saved(0, PathBuf::from("C:/in/a.jpg"));
        assert_eq!(b.detail(&b.items[0]), "Saved as a.jpg");
        assert!(b.safe_to_close() && b.primary() == Primary::Done);
    }

    #[test]
    fn test_removing_never_touches_originals_and_returns_unsaved_output() {
        let mut b = batch_of(&[("a.png", Format::Png), ("b.png", Format::Png)]);
        b.choose(Format::Jpeg);
        let _ = b.start().unwrap();
        b.finished(
            0,
            Ok(Done {
                output: "C:/s/a.jpg".into(),
                bytes: 1,
                format: Format::Jpeg,
                note: None,
            }),
        );
        assert_eq!(b.remove(0), Some(PathBuf::from("C:/s/a.jpg")));
        assert_eq!(b.items.len(), 1);
        assert!(b.running(), "the other file is still queued");
        // A result arriving for a removed file hands its output back to delete
        let stray = b.finished(
            0,
            Ok(Done {
                output: "C:/s/x.jpg".into(),
                bytes: 1,
                format: Format::Jpeg,
                note: None,
            }),
        );
        assert_eq!(stray, Some(PathBuf::from("C:/s/x.jpg")));
        assert!(b.remove(99).is_none());
    }

    #[test]
    fn test_cancel_returns_files_to_ready_and_clear_forgets_everything() {
        let mut b = batch_of(&[("a.png", Format::Png)]);
        b.choose(Format::Jpeg);
        let jobs = b.start().unwrap();
        b.cancel_run();
        assert!(jobs[0].cancel.load(Ordering::Relaxed));
        b.finished(0, Err(ConvertError::Cancelled));
        assert_eq!(b.items[0].status, Status::Ready);
        assert!(!b.running());
        let staged = {
            let _ = b.start().unwrap();
            b.finished(
                0,
                Ok(Done {
                    output: "C:/s/a.jpg".into(),
                    bytes: 1,
                    format: Format::Jpeg,
                    note: None,
                }),
            );
            b.clear()
        };
        assert_eq!(staged, vec![PathBuf::from("C:/s/a.jpg")]);
        assert!(b.is_empty() && b.target().is_none() && !b.confirming);
    }

    #[test]
    fn test_save_folder_defaults_beside_the_original_and_options_map_to_settings() {
        let mut b = batch_of(&[("a.png", Format::Png)]);
        assert_eq!(b.save_dir(0), Some(PathBuf::from("C:/in")));
        b.out_dir = Some(PathBuf::from("D:/out"));
        assert_eq!(b.save_dir(0), Some(PathBuf::from("D:/out")));
        let s = b.options.settings();
        assert_eq!((s.jpeg_quality, s.background), (90, [255, 255, 255]));
        assert!(s.validate().is_ok());
        for _ in 0..8 {
            b.cycle_jpeg_quality();
            b.cycle_background();
            b.cycle_vorbis_quality();
            assert!(b.options.settings().validate().is_ok());
        }
        b.choose(Format::Jpeg);
        assert_eq!(b.chips().len(), 2);
        b.choose(Format::WebP);
        assert!(b.chips().is_empty());
    }

    #[test]
    fn test_scrolling_is_clamped_and_follows_removal() {
        let mut b = batch_of(&[
            ("1.png", Format::Png),
            ("2.png", Format::Png),
            ("3.png", Format::Png),
            ("4.png", Format::Png),
            ("5.png", Format::Png),
        ]);
        assert!(b.scroll_by(1) && b.scroll_by(5));
        assert_eq!(b.scroll, 2);
        assert!(!b.scroll_by(1), "already at the end");
        b.remove(0);
        b.remove(1);
        assert_eq!(b.scroll, 0, "the window shrinks back with the list");
        assert!(!b.scroll_by(-1));
    }

    #[test]
    fn test_the_view_labels_what_the_buttons_will_do() {
        let mut b = batch_of(&[
            ("a.png", Format::Png),
            ("b.png", Format::Png),
            ("c.png", Format::Png),
        ]);
        let v = b.view();
        assert_eq!(v.rows.len(), VISIBLE_ROWS);
        assert_eq!(
            (v.count_label.as_str(), v.primary_label.as_str()),
            ("3 of 3 selected", "Convert all")
        );
        assert!(v.all_selected && v.hint.is_none() && !v.formats.is_empty());
        b.toggle(0);
        assert_eq!(b.view().primary_label, "Convert 2");
        b.toggle(1);
        assert_eq!(b.view().primary_label, "Convert");
        b.toggle(2);
        let v = b.view();
        assert_eq!(
            (v.count_label.as_str(), v.hint),
            ("None selected", Some("Select files to convert"))
        );
        assert_eq!(v.primary, Primary::Done);
        b.toggle_all();
        b.choose(Format::Jpeg);
        let _ = b.start().unwrap();
        let v = b.view();
        assert_eq!(
            (v.primary, v.primary_label.as_str(), v.hint),
            (Primary::Cancel, "Cancel", Some("Converting…"))
        );
        assert!(
            v.rows
                .iter()
                .all(|r| r.kind == RowKind::Busy && r.progress == Some(0.0) && !r.selectable)
        );
        b.finished(
            0,
            Ok(Done {
                output: "C:/s/a.jpg".into(),
                bytes: 5,
                format: Format::Jpeg,
                note: None,
            }),
        );
        b.finished(
            1,
            Ok(Done {
                output: "C:/s/b.jpg".into(),
                bytes: 5,
                format: Format::Jpeg,
                note: None,
            }),
        );
        b.finished(
            2,
            Ok(Done {
                output: "C:/s/c.jpg".into(),
                bytes: 5,
                format: Format::Jpeg,
                note: None,
            }),
        );
        let v = b.view();
        assert_eq!(
            (v.primary_label.as_str(), v.confirm_message.as_str()),
            ("Save all 3", "Discard 3 unsaved files?")
        );
        assert!(v.rows.iter().all(|r| r.save && r.kind == RowKind::Done));
    }

    #[test]
    fn test_removal_fades_first_and_reaps_only_what_has_faded() {
        let mut b = batch_of(&[("a.png", Format::Png), ("b.png", Format::Png)]);
        b.choose(Format::Jpeg);
        let _ = b.start().unwrap();
        b.finished(
            0,
            Ok(Done {
                output: "C:/s/a.jpg".into(),
                bytes: 1,
                format: Format::Jpeg,
                note: None,
            }),
        );
        b.begin_remove(0);
        let v = b.view();
        assert!(
            v.rows[0].removing && !v.rows[0].save,
            "a fading row offers nothing"
        );
        assert_eq!(b.items.len(), 2, "still there while it fades");
        assert!(b.reap(|_| false).is_empty() && b.items.len() == 2);
        assert_eq!(b.reap(|id| id == 0), vec![PathBuf::from("C:/s/a.jpg")]);
        assert_eq!(b.items.len(), 1);
        b.toggle(0); // a removing row can't be ticked
        let mut c = batch_of(&[("x.png", Format::Png)]);
        c.begin_remove(0);
        c.toggle(0);
        assert!(!c.items[0].selected && c.selected_count() == 0);
    }

    #[test]
    fn test_stale_staging_cleanup_only_touches_dead_oeca_staging_folders() {
        let root = std::env::temp_dir().join(format!("oeca-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let own = std::process::id();
        let dead = u32::MAX - 1;
        let mk = |name: String, files: &[&str]| {
            let d = root.join(name);
            std::fs::create_dir_all(&d).unwrap();
            for f in files {
                std::fs::write(d.join(f), b"x").unwrap();
            }
            d
        };
        let stale = mk(format!("Oeca-convert-{dead}"), &["a.jpg", ".oeca-1-a.part"]);
        let live_own = mk(format!("Oeca-convert-{own}"), &["keep.jpg"]);
        let other_live = mk("Oeca-convert-4".into(), &["keep.jpg"]);
        let unrelated = mk("Documents".into(), &["mine.txt"]);
        let lookalike = mk(format!("Oeca-convert-{dead}x"), &["mine.txt"]);
        let with_sub = mk(format!("Oeca-convert-{}", dead - 1), &["a.jpg"]);
        std::fs::create_dir(with_sub.join("user-folder")).unwrap();
        let stray_file = root.join(format!("Oeca-convert-{}", dead - 2));
        std::fs::write(&stray_file, b"file, not a folder").unwrap();
        assert_eq!(
            cleanup_stale_in(&root, own),
            1,
            "only the emptied stale folder goes"
        );
        assert!(!stale.exists());
        assert!(
            live_own.join("keep.jpg").exists(),
            "this run's own batch is kept"
        );
        assert!(
            other_live.join("keep.jpg").exists(),
            "a live process's folder is kept"
        );
        assert!(unrelated.join("mine.txt").exists() && lookalike.join("mine.txt").exists());
        assert!(
            with_sub.join("user-folder").exists(),
            "a subfolder is never deleted"
        );
        assert!(!with_sub.join("a.jpg").exists() || with_sub.exists());
        assert!(
            stray_file.exists(),
            "a plain file with that name is left alone"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_human_sizes() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human_size(300 * 1024 * 1024), "300 MB");
    }
    #[test]
    fn test_bulk_batch_offers_every_image_format_and_copies_matching_files() {
        let mut b = batch_of(&[
            ("a.png", Format::Png),
            ("b.jpg", Format::Jpeg),
            ("c.webp", Format::WebP),
            ("d.gif", Format::Gif),
        ]);
        let offered = b.formats();
        for f in [
            Format::Png,
            Format::Jpeg,
            Format::WebP,
            Format::Bmp,
            Format::Tiff,
        ] {
            assert!(offered.contains(&f), "{f:?} stays offered in a bulk batch");
        }
        b.choose(Format::Png);
        assert_eq!(b.target(), Some(Format::Png));
        let jobs = b.start().unwrap();
        assert_eq!(jobs.len(), 4, "no file is dropped from the batch");
        assert!(
            jobs.iter()
                .all(|j| j.same_format_copy && j.to == Format::Png)
        );
        // With one file selected, the single-file rule applies again
        let mut one = batch_of(&[("a.png", Format::Png), ("b.jpg", Format::Jpeg)]);
        one.toggle(1);
        assert!(!one.formats().contains(&Format::Png));
        assert!(one.formats().contains(&Format::Jpeg));
        let jobs = one.start();
        assert!(jobs.is_none() || !jobs.unwrap()[0].same_format_copy);
    }
    /// A whole bulk run without a window: real files, the batch's own jobs run
    /// one after another as the worker would, then Save all beside the originals.
    #[test]
    fn test_bulk_run_with_matching_unsupported_and_failing_files_then_save_all() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("oeca-bulk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb([200, 30, 30]));
        let png = dir.join("a.png");
        let jpg = dir.join("b.jpg");
        let webp = dir.join("c.webp");
        img.save(&png).unwrap();
        img.save(&jpg).unwrap();
        image::DynamicImage::ImageRgb8(img.clone())
            .save_with_format(&webp, image::ImageFormat::WebP)
            .unwrap();
        let text = dir.join("notes.txt");
        fs::write(&text, b"not an image").unwrap();
        // Valid PNG header, damaged body: inspection passes, conversion fails
        let damaged = dir.join("d.png");
        let mut bytes = fs::read(&png).unwrap();
        bytes.truncate(bytes.len() - 12);
        fs::write(&damaged, bytes).unwrap();
        let before: Vec<_> = [&png, &jpg, &webp, &text, &damaged]
            .iter()
            .map(|p| fs::read(p).unwrap())
            .collect();

        let mut b = Batch::default();
        let added = b.add(
            [&png, &jpg, &webp, &text, &damaged]
                .map(|p| p.to_path_buf())
                .to_vec(),
        );
        assert_eq!(added.len(), 5);
        for (id, path) in &added {
            b.inspected(*id, convert::inspect(path));
        }
        assert!(
            matches!(b.items[3].status, Status::Invalid(_)),
            "the text file is explained, not dropped"
        );
        assert_eq!(b.items.len(), 5, "nothing leaves the batch");
        // JPEG is offered although b.jpg already is one
        assert!(b.formats().contains(&Format::Jpeg));
        b.choose(Format::Jpeg);
        let jobs = b.start().unwrap();
        let staging = dir.join("staging");
        fs::create_dir_all(&staging).unwrap();
        for job in jobs {
            let req = convert::Request {
                input: &job.input,
                to: job.to,
                out_dir: &staging,
                settings: job.settings,
                same_format_copy: job.same_format_copy,
            };
            let r = convert::convert(&req, &AtomicBool::new(false), &mut |_| {});
            b.finished(job.id, r);
        }
        b.run_ended();
        let status = |n: usize| b.items[n].status.clone();
        assert!(matches!(
            status(0),
            Status::Done {
                to: Format::Jpeg,
                note: None,
                ..
            }
        ));
        assert!(
            matches!(
                status(1),
                Status::Done {
                    note: Some("Copied unchanged"),
                    ..
                }
            ),
            "the JPEG is copied, not recompressed"
        );
        assert!(matches!(status(2), Status::Done { .. }));
        assert!(matches!(status(3), Status::Invalid(_)));
        assert!(
            matches!(status(4), Status::Failed(_)),
            "a per-file failure is a failure, not a success"
        );
        assert_eq!(b.unsaved(), 3);

        // Save all: beside the originals, never over them
        for id in b.unsaved_ids() {
            let Some(Status::Done { staged, to, .. }) = b.get(id).map(|i| i.status.clone()) else {
                unreachable!()
            };
            let stem = b
                .get(id)
                .unwrap()
                .path
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let saved =
                convert::publish(&staged, &b.save_dir(id).unwrap(), &stem, to.extension()).unwrap();
            b.mark_saved(id, saved);
        }
        assert!(b.safe_to_close());
        assert_eq!(
            fs::read(&jpg).unwrap(),
            before[1],
            "b.jpg itself is untouched"
        );
        let saved_b = match b.items[1].status.clone() {
            Status::Saved(p) => p,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            saved_b.file_name().unwrap(),
            "b (2).jpg",
            "an existing file is never overwritten"
        );
        assert_eq!(fs::read(&saved_b).unwrap(), before[1]);
        for (path, old) in [&png, &jpg, &webp, &text, &damaged].iter().zip(&before) {
            assert_eq!(&fs::read(path).unwrap(), old, "{path:?} unchanged");
        }
        assert!(fs::read_dir(&staging).unwrap().all(|e| {
            let n = e.unwrap().file_name().to_string_lossy().into_owned();
            !n.ends_with(".part")
        }));
        let _ = fs::remove_dir_all(&dir);
    }
}
