//! Oeca Settings (Phases 6.4-6.6): a normal native window, separate from the
//! notch, opened from the notch Settings page's "Open Full Settings". A
//! persistent sidebar selects a category page; General holds the quick
//! Settings page's switches plus the window-behavior ones (Escape closes this
//! window, confirm before clearing the clipboard history); Appearance holds
//! the accent color (Phase 6.7), chosen from swatches; Clipboard the history
//! capacity and Media the source badge / visualizer switches (Phase 6.8).
//! The notch owns them
//! (`WindowState.settings`, the one source of truth): a switch here posts
//! `WM_APP_SETTINGS_TOGGLE` to the notch, which applies the change through its
//! existing code and pushes the new values back with `sync`.
//!
//! Lifecycle: at most one window, created on demand (`open` focuses an existing
//! one), unowned (so it is never pulled into the notch's topmost band) and
//! never topmost. Closing it leaves Oeca running; the notch destroys it on
//! shutdown (`close`). A new window always starts on General. All on the UI
//! thread, drawn with Direct2D in DIPs at the window's own DPI.

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{
    D2DERR_RECREATE_TARGET, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Direct2D::Common::{D2D_SIZE_U, D2D1_COLOR_F};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_ROUNDED_RECT, D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1RenderTarget,
    ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_PARAGRAPH_ALIGNMENT_FAR, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT, ScreenToClient,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_DOWN,
    VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, IDC_ARROW, ISMEX_NOTIFY,
    ISMEX_SEND, InSendMessageEx, IsIconic, LoadCursorW, MINMAXINFO, PostMessageW, RegisterClassExW,
    SET_WINDOW_POS_FLAGS, SW_RESTORE, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SetForegroundWindow, SetWindowPos, ShowWindow, WINDOWPOS, WM_APP,
    WM_CAPTURECHANGED, WM_DESTROY, WM_DPICHANGED, WM_ENTERSIZEMOVE, WM_ERASEBKGND,
    WM_GETMINMAXINFO, WM_GETOBJECT, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_NCLBUTTONDOWN, WM_PAINT, WM_SIZE, WM_SYSKEYDOWN, WM_WINDOWPOSCHANGED, WNDCLASSEXW,
    WS_EX_APPWINDOW, WS_OVERLAPPEDWINDOW,
};
use windows::core::{PCWSTR, Result, w};

use crate::config::{
    AccentChoice, COLOR_SPACE_PILL_SELECTED, COLOR_TEXT_PRIMARY, COLOR_TEXT_SECONDARY,
    COLOR_TEXT_TERTIARY, ClipboardCapacity, CollapseDelay, NOTCH_BG_COLOR, TextRole,
};
use crate::layout::RectF;
use crate::renderer::{CLIPBOARD, GEAR, INFO, IconSeg, MUSIC_ALT, PALETTE, fill_glyph};
use crate::space::OecaSettings;
use crate::window::WM_MOUSELEAVE;

const CLASS_NAME: PCWSTR = w!("OecaSettingsWindowClass");
const TITLE: PCWSTR = w!("Oeca Settings");

/// Posted to the notch window when a switch here is used (wParam: the
/// `Control` index). The notch applies it and calls `sync`.
pub const WM_APP_SETTINGS_TOGGLE: u32 = WM_APP + 30;
/// Posted to the notch window when an accent swatch is chosen (wParam: the
/// `AccentChoice` index). The notch applies it and calls `sync`.
pub const WM_APP_SETTINGS_ACCENT: u32 = WM_APP + 31;
/// Posted to the notch window when a history capacity is chosen (wParam: the
/// `ClipboardCapacity` index). The notch applies it and calls `sync`.
pub const WM_APP_SETTINGS_CAPACITY: u32 = WM_APP + 32;
/// Posted to the notch window when a closing delay is chosen (wParam: the
/// `CollapseDelay` index). The notch applies it and calls `sync`.
pub const WM_APP_SETTINGS_DELAY: u32 = WM_APP + 33;
/// The Settings window's switch-animation frame timer (runs only while a
/// switch slides).
const SWITCH_TIMER_ID: usize = 1;
/// General's "Notch" section: the closing-delay row under the two switches.
pub const CLOSING_TITLE: &str = "Notch closing behaviour";
pub const CLOSING_DESCRIPTION: &str = "Delay after an outside click";

/// Client area (DIP): initial size, and the minimum the window can shrink to
/// (the sidebar plus a page column wide enough for the General rows).
pub const CLIENT_SIZE: (f32, f32) = (820.0, 520.0);
/// The minimum holds General: its "Notch" section (two switches and the
/// closing-delay row) and "Window behavior" (two switches), and the longest
/// setting title unclipped beside its switch.
pub const MIN_CLIENT_SIZE: (f32, f32) = (640.0, 504.0);

/// Sidebar (DIP): fixed width; category entries with an icon and a label.
pub const SIDEBAR_WIDTH: f32 = 200.0;
const SIDEBAR_TOP: f32 = 20.0;
const SIDEBAR_INSET: f32 = 10.0;
const ITEM_HEIGHT: f32 = 36.0;
const ITEM_GAP: f32 = 2.0;
const ITEM_ICON: f32 = 16.0;
/// Page column padding (DIP).
const PAGE_PADDING: f32 = 36.0;

/// Window background: a deep neutral, a step up from the notch's pure black;
/// the sidebar sits a shade darker.
const COLOR_BACKGROUND: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.071,
    g: 0.071,
    b: 0.078,
    a: 1.0,
};
const COLOR_SIDEBAR: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.051,
    g: 0.051,
    b: 0.057,
    a: 1.0,
};
const COLOR_SEPARATOR: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.08,
};
const COLOR_HOVER: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.06,
};

/// A settings page, in sidebar order. Typed identity: routing, drawing and
/// accessibility all match on this, never on strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Category {
    #[default]
    General,
    Appearance,
    Clipboard,
    Media,
    About,
}

impl Category {
    pub const ALL: [Self; 5] = [
        Self::General,
        Self::Appearance,
        Self::Clipboard,
        Self::Media,
        Self::About,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// Sidebar label and page title.
    pub fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Clipboard => "Clipboard",
            Self::Media => "Media",
            Self::About => "About",
        }
    }

    /// The page's description line under its title. Pages without settings
    /// yet only say what they are for; nothing claims unbuilt features.
    pub fn description(self) -> &'static str {
        match self {
            Self::General => "How Oeca behaves on your desktop.",
            Self::Appearance => "How Oeca looks on your desktop.",
            Self::Clipboard => "How much copied history Oeca keeps.",
            Self::Media => "How detected Windows media sessions are shown.",
            Self::About => concat!("Oeca, version ", env!("CARGO_PKG_VERSION"), "."),
        }
    }

    /// Sidebar icon: the project's licensed UIcons glyphs.
    fn icon(self) -> &'static [IconSeg] {
        match self {
            Self::General => GEAR,
            Self::Appearance => PALETTE,
            Self::Clipboard => CLIPBOARD,
            Self::Media => MUSIC_ALT,
            Self::About => INFO,
        }
    }

    /// The switches on this page, in page order.
    pub fn controls(self) -> &'static [Control] {
        match self {
            Self::General => &Control::ALL[..4],
            Self::Media => &Control::ALL[4..],
            _ => &[],
        }
    }

    /// The history capacity choices on this page (Clipboard's).
    pub fn capacities(self) -> &'static [ClipboardCapacity] {
        match self {
            Self::Clipboard => &ClipboardCapacity::ALL,
            _ => &[],
        }
    }

    /// The accent swatches on this page (Appearance's).
    pub fn accents(self) -> &'static [AccentChoice] {
        match self {
            Self::Appearance => &AccentChoice::ALL,
            _ => &[],
        }
    }
}

/// Appearance's section headings: the swatches, then the preview.
pub const APPEARANCE_SECTIONS: [&str; 2] = ["Accent color", "Preview"];
/// The single section heading of Clipboard, then of Media.
pub const CLIPBOARD_SECTION: &str = "History";
pub const MEDIA_SECTION: &str = "Now playing";
/// Clipboard's capacity row, and its informational lines.
pub const CAPACITY_TITLE: &str = "Items to keep";
pub const CAPACITY_DESCRIPTION: &str = "Oldest items are removed first";
pub const CLIPBOARD_NOTES: [&str; 2] = [
    "History stays in memory and is never saved to disk.",
    "Clearing it never changes the Windows clipboard.",
];

/// A setting shown in the window (all switches). `ALL` is the page and
/// focus order: General's four, then Media's two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    AlwaysOnTop,
    ReducedMotion,
    CloseOnEscape,
    ConfirmClearClipboard,
    ShowSourceApp,
    ShowVisualizer,
}

/// General's sections: a heading over its rows (in `Control::ALL` order).
pub const SECTIONS: [(&str, [Control; 2]); 2] = [
    ("Notch", [Control::AlwaysOnTop, Control::ReducedMotion]),
    (
        "Window behavior",
        [Control::CloseOnEscape, Control::ConfirmClearClipboard],
    ),
];

impl Control {
    pub const ALL: [Self; 6] = [
        Self::AlwaysOnTop,
        Self::ReducedMotion,
        Self::CloseOnEscape,
        Self::ConfirmClearClipboard,
        Self::ShowSourceApp,
        Self::ShowVisualizer,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Option<Self> {
        Self::ALL.get(i).copied()
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::AlwaysOnTop => "Always on top",
            Self::ReducedMotion => "Reduced motion",
            Self::CloseOnEscape => "Close Settings window with Escape",
            Self::ConfirmClearClipboard => "Confirm before clearing clipboard history",
            Self::ShowSourceApp => "Show source app",
            Self::ShowVisualizer => "Show visualizer",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::AlwaysOnTop => "Keep Oeca above other windows",
            Self::ReducedMotion => "Minimize Oeca's interface transitions",
            Self::CloseOnEscape => "Press Esc to close this window",
            Self::ConfirmClearClipboard => "Ask before Clear forgets Oeca's copied items",
            Self::ShowSourceApp => "App icon on the Home artwork",
            Self::ShowVisualizer => "Animated bars in the Music space",
        }
    }

    fn automation_id(self) -> &'static str {
        match self {
            Self::AlwaysOnTop => "AlwaysOnTop",
            Self::ReducedMotion => "ReducedMotion",
            Self::CloseOnEscape => "CloseOnEscape",
            Self::ConfirmClearClipboard => "ConfirmClearClipboard",
            Self::ShowSourceApp => "ShowSourceApp",
            Self::ShowVisualizer => "ShowVisualizer",
        }
    }

    /// Flips this control's value (the notch applies it to its settings).
    pub fn toggle(self, settings: &mut OecaSettings) {
        let value = match self {
            Self::AlwaysOnTop => &mut settings.always_on_top,
            Self::ReducedMotion => &mut settings.reduced_motion,
            Self::CloseOnEscape => &mut settings.close_settings_on_escape,
            Self::ConfirmClearClipboard => &mut settings.confirm_clear_clipboard,
            Self::ShowSourceApp => &mut settings.show_source_app,
            Self::ShowVisualizer => &mut settings.show_visualizer,
        };
        *value = !*value;
    }

    /// This control's value in the (notch-owned) settings.
    pub fn is_on(self, settings: OecaSettings) -> bool {
        match self {
            Self::AlwaysOnTop => settings.always_on_top,
            Self::ReducedMotion => settings.reduced_motion,
            Self::CloseOnEscape => settings.close_settings_on_escape,
            Self::ConfirmClearClipboard => settings.confirm_clear_clipboard,
            Self::ShowSourceApp => settings.show_source_app,
            Self::ShowVisualizer => settings.show_visualizer,
        }
    }
}

/// Something focusable / clickable: a sidebar entry or a page control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Category(Category),
    Control(Control),
    /// An Appearance accent swatch.
    Accent(AccentChoice),
    /// A Clipboard history capacity segment.
    Capacity(ClipboardCapacity),
    /// A General closing-delay segment.
    Delay(CollapseDelay),
}

/// Navigation state: the selected page and keyboard focus. Separate from the
/// settings (which only the notch changes) and from drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nav {
    pub category: Category,
    pub focus: Target,
    /// Focus ring shown (after keyboard navigation).
    pub focus_visible: bool,
}

impl Default for Nav {
    fn default() -> Self {
        Self {
            category: Category::General,
            focus: Target::Category(Category::General),
            focus_visible: false,
        }
    }
}

impl Nav {
    /// Tab order: the sidebar entries, then the current page's controls
    /// (General's switches, or Appearance's swatches).
    pub fn focus_order(self) -> Vec<Target> {
        // General, in visual order: its first two switches, the closing-delay
        // segments, then the "Window behavior" switches
        let controls = self.category.controls();
        let split = if self.category == Category::General {
            2
        } else {
            controls.len()
        };
        let delays: &[CollapseDelay] = if self.category == Category::General {
            &CollapseDelay::ALL
        } else {
            &[]
        };
        Category::ALL
            .into_iter()
            .map(Target::Category)
            .chain(controls[..split].iter().map(|c| Target::Control(*c)))
            .chain(delays.iter().map(|d| Target::Delay(*d)))
            .chain(controls[split..].iter().map(|c| Target::Control(*c)))
            .chain(self.category.accents().iter().map(|a| Target::Accent(*a)))
            .chain(
                self.category
                    .capacities()
                    .iter()
                    .map(|c| Target::Capacity(*c)),
            )
            .collect()
    }

    /// Shows a page. Focus stays put unless it was on a control of another page.
    pub fn select(&mut self, category: Category) -> bool {
        let changed = self.category != category;
        self.category = category;
        if !self.focus_order().contains(&self.focus) {
            self.focus = Target::Category(category);
        }
        changed
    }

    /// Tab / Shift+Tab: the next or previous item in the tab order.
    pub fn tab(&mut self, back: bool) -> Target {
        let order = self.focus_order();
        let i = order.iter().position(|t| *t == self.focus).unwrap_or(0);
        let n = order.len();
        self.focus = order[if back { (i + n - 1) % n } else { (i + 1) % n }];
        self.focus_visible = true;
        self.focus
    }

    /// Up / Down (and Left / Right on the swatches): moves within the focused
    /// group (sidebar entries, the page's switches, or the swatches),
    /// stopping at its ends.
    pub fn arrow(&mut self, back: bool) -> Target {
        let step = |i: usize, n: usize| {
            if back {
                i.saturating_sub(1)
            } else {
                (i + 1).min(n - 1)
            }
        };
        self.focus = match self.focus {
            Target::Category(c) => {
                Target::Category(Category::ALL[step(c.index(), Category::ALL.len())])
            }
            Target::Control(c) => {
                let controls = self.category.controls();
                let i = controls.iter().position(|x| *x == c).unwrap_or(0);
                Target::Control(controls[step(i, controls.len())])
            }
            Target::Accent(a) => {
                Target::Accent(AccentChoice::ALL[step(a.index(), AccentChoice::ALL.len())])
            }
            Target::Capacity(c) => Target::Capacity(
                ClipboardCapacity::ALL[step(c.index(), ClipboardCapacity::ALL.len())],
            ),
            Target::Delay(d) => {
                Target::Delay(CollapseDelay::ALL[step(d.index(), CollapseDelay::ALL.len())])
            }
        };
        self.focus_visible = true;
        self.focus
    }
}

/// One setting row (DIP): the whole row is clickable; the switch sits at the
/// page's right edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Row {
    pub row: RectF,
    pub title: RectF,
    pub description: RectF,
    pub switch: RectF,
}

/// The window's layout (DIP, client coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowLayout {
    pub sidebar: RectF,
    /// Sidebar entries, in `Category::ALL` order.
    pub items: [RectF; 5],
    pub page_title: RectF,
    pub page_description: RectF,
    /// General's section headings, in `SECTIONS` order.
    pub sections: [RectF; 2],
    /// The switch rows, in `Control::ALL` order: General's two sections,
    /// then Media's (in General's first section's place).
    pub rows: [Row; 6],
    /// Appearance's section headings, in `APPEARANCE_SECTIONS` order.
    pub appearance_sections: [RectF; 2],
    /// Appearance's swatches (each a square hit target), in
    /// `AccentChoice::ALL` order; the selected one's name under them.
    pub swatches: [RectF; 7],
    pub accent_name: RectF,
    /// The accent preview strip.
    pub preview: RectF,
    /// Clipboard's capacity row (title, description) and its segments, in
    /// `ClipboardCapacity::ALL` order; then the informational lines.
    pub capacity_title: RectF,
    pub capacity_description: RectF,
    pub capacity: [RectF; 3],
    pub clipboard_notes: [RectF; 2],
    /// General's closing-delay row (title, description, whole row) and its
    /// segments, in `CollapseDelay::ALL` order.
    pub closing_title: RectF,
    pub closing_description: RectF,
    pub closing_row: RectF,
    pub closing: [RectF; 3],
}

/// A three-segment selector at the right end of a row starting at `top`,
/// and where the row's text must end.
fn segments(right: f32, top: f32) -> ([RectF; 3], f32) {
    const SEGMENT: (f32, f32) = (52.0, 28.0);
    let segs = std::array::from_fn(|i| {
        let x = right - (3 - i) as f32 * SEGMENT.0;
        RectF::new(x, top + 18.0, x + SEGMENT.0, top + 18.0 + SEGMENT.1)
    });
    (segs, right - 3.0 * SEGMENT.0 - 16.0)
}

/// Lays the window out for a client size (DIP). The sidebar keeps its width;
/// the page column takes the rest.
pub fn layout(width: f32, height: f32) -> WindowLayout {
    const SECTION_TOP: f32 = 100.0;
    const SECTION_HEIGHT: f32 = 32.0;
    const SECTION_GAP: f32 = 8.0;
    const ROW_HEIGHT: f32 = 64.0;
    let items = std::array::from_fn(|i| {
        let top = SIDEBAR_TOP + i as f32 * (ITEM_HEIGHT + ITEM_GAP);
        RectF::new(
            SIDEBAR_INSET,
            top,
            SIDEBAR_WIDTH - SIDEBAR_INSET,
            top + ITEM_HEIGHT,
        )
    });
    let left = SIDEBAR_WIDTH + PAGE_PADDING;
    let right = (width - PAGE_PADDING).max(left + 240.0);
    // Each section: its heading, then its rows ("Notch" has three: two
    // switches and the closing-delay row; "Window behavior" two)
    let section_top =
        |k: usize| SECTION_TOP + k as f32 * (SECTION_HEIGHT + 3.0 * ROW_HEIGHT + SECTION_GAP);
    let sections = std::array::from_fn(|k| {
        let top = section_top(k);
        RectF::new(left, top, right, top + SECTION_HEIGHT)
    });
    let rows = std::array::from_fn(|i| {
        // Media's rows (4, 5) sit where General's first section's do
        let i = if i < 4 { i } else { i - 4 };
        let top = section_top(i / 2) + SECTION_HEIGHT + (i % 2) as f32 * ROW_HEIGHT;
        let switch = RectF::new(right - 40.0, top + 21.0, right, top + 43.0);
        let text_right = switch.left - 16.0;
        Row {
            row: RectF::new(left, top, right, top + ROW_HEIGHT),
            title: RectF::new(left, top + 12.0, text_right, top + 32.0),
            description: RectF::new(left, top + 32.0, text_right, top + 52.0),
            switch,
        }
    });
    // Appearance: "Accent color", a row of swatches and the chosen name,
    // then "Preview" over a short strip
    const SWATCH: f32 = 36.0;
    const SWATCH_GAP: f32 = 8.0;
    let appearance_sections = [
        RectF::new(left, SECTION_TOP, right, SECTION_TOP + SECTION_HEIGHT),
        RectF::new(left, 216.0, right, 216.0 + SECTION_HEIGHT),
    ];
    let swatch_top = SECTION_TOP + SECTION_HEIGHT + 8.0;
    let swatches = std::array::from_fn(|i| {
        let x = left + i as f32 * (SWATCH + SWATCH_GAP);
        RectF::new(x, swatch_top, x + SWATCH, swatch_top + SWATCH)
    });
    let accent_name = RectF::new(
        left,
        swatch_top + SWATCH + 4.0,
        right,
        swatch_top + SWATCH + 24.0,
    );
    let preview_top = appearance_sections[1].bottom + 8.0;
    let preview = RectF::new(
        left,
        preview_top,
        right.min(left + 360.0),
        preview_top + 56.0,
    );
    // Clipboard: "History", one row with a three-segment selector at its
    // right end, then two informational lines
    let row_top = SECTION_TOP + SECTION_HEIGHT;
    let (capacity, text_right) = segments(right, row_top);
    let capacity_title = RectF::new(left, row_top + 12.0, text_right, row_top + 32.0);
    let capacity_description = RectF::new(left, row_top + 32.0, text_right, row_top + 52.0);
    let notes_top = row_top + ROW_HEIGHT + 16.0;
    let clipboard_notes = std::array::from_fn(|i| {
        let top = notes_top + i as f32 * 20.0;
        RectF::new(left, top, right, top + 20.0)
    });
    // General: the closing-delay row, third in "Notch"
    let closing_top = section_top(0) + SECTION_HEIGHT + 2.0 * ROW_HEIGHT;
    let (closing, closing_right) = segments(right, closing_top);
    let closing_title = RectF::new(left, closing_top + 12.0, closing_right, closing_top + 32.0);
    let closing_description =
        RectF::new(left, closing_top + 32.0, closing_right, closing_top + 52.0);
    let closing_row = RectF::new(left, closing_top, right, closing_top + ROW_HEIGHT);
    WindowLayout {
        sidebar: RectF::new(0.0, 0.0, SIDEBAR_WIDTH, height),
        items,
        page_title: RectF::new(left, 28.0, right, 62.0),
        page_description: RectF::new(left, 64.0, right, 84.0),
        sections,
        rows,
        appearance_sections,
        swatches,
        accent_name,
        preview,
        capacity_title,
        capacity_description,
        capacity,
        clipboard_notes,
        closing_title,
        closing_description,
        closing_row,
        closing,
    }
}

/// What a client point (DIP) lands on, with `category` showing: a sidebar
/// entry, or a control on the current page.
pub fn hit(width: f32, height: f32, category: Category, x: f32, y: f32) -> Option<Target> {
    let l = layout(width, height);
    if let Some(c) = Category::ALL
        .into_iter()
        .find(|c| l.items[c.index()].contains(x, y))
    {
        return Some(Target::Category(c));
    }
    if let Some(a) = category
        .accents()
        .iter()
        .find(|a| l.swatches[a.index()].contains(x, y))
    {
        return Some(Target::Accent(*a));
    }
    if let Some(c) = category
        .capacities()
        .iter()
        .find(|c| l.capacity[c.index()].contains(x, y))
    {
        return Some(Target::Capacity(*c));
    }
    if category == Category::General
        && let Some(d) = CollapseDelay::ALL
            .into_iter()
            .find(|d| l.closing[d.index()].contains(x, y))
    {
        return Some(Target::Delay(d));
    }
    category
        .controls()
        .iter()
        .find(|c| l.rows[c.index()].row.contains(x, y))
        .map(|c| Target::Control(*c))
}

/// A DIP size in physical pixels at `dpi`.
pub fn to_px(size: (f32, f32), dpi: u32) -> (i32, i32) {
    let s = dpi as f32 / 96.0;
    ((size.0 * s).round() as i32, (size.1 * s).round() as i32)
}

struct Formats {
    heading: IDWriteTextFormat,
    body: IDWriteTextFormat,
    caption: IDWriteTextFormat,
    section: IDWriteTextFormat,
    item: IDWriteTextFormat,
    item_selected: IDWriteTextFormat,
    /// Capacity segment labels (centred), regular and selected.
    segment: IDWriteTextFormat,
    segment_selected: IDWriteTextFormat,
}

/// The open window's view state (UI thread only).
struct View {
    hwnd: HWND,
    notch: HWND,
    /// Display copy of the notch's settings, replaced by `sync`.
    settings: OecaSettings,
    nav: Nav,
    hover: Option<Target>,
    pressed: Option<Target>,
    d2d: ID2D1Factory,
    formats: Formats,
    target: Option<ID2D1HwndRenderTarget>,
    /// Where Oeca placed the new window (outer top-left, px), until the user
    /// takes over (a click, key, move/size or DPI change). While set, a move
    /// made by another process is undone (`placement_correction`).
    placement: Option<(i32, i32)>,
    corrections: u8,
    /// Each switch's slide (`Control::ALL` order), and the last frame of the
    /// switch timer (Some only while it runs).
    switches: [crate::renderer::SwitchAnim; 6],
    switch_tick: Option<std::time::Instant>,
}

thread_local! {
    static VIEW: RefCell<Option<View>> = const { RefCell::new(None) };
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
}

fn with_view<R>(f: impl FnOnce(&mut View) -> R) -> Option<R> {
    VIEW.with(|v| v.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Whether the Settings window exists.
pub fn is_open() -> bool {
    with_view(|_| ()).is_some()
}

/// The open Settings window, if any (the notch treats clicks on it as Oeca
/// being used).
pub fn handle() -> Option<HWND> {
    window_handle()
}

fn window_handle() -> Option<HWND> {
    with_view(|v| v.hwnd)
}

/// Opens the Settings window (creating it once) and brings it to the front.
/// `notch` receives the switch changes; `settings` is the current state.
pub fn open(notch: HWND, settings: OecaSettings) -> Result<()> {
    let hwnd = match window_handle() {
        Some(hwnd) => hwnd,
        None => create(notch, settings)?,
    };
    sync(settings);
    unsafe {
        let _ = ShowWindow(
            hwnd,
            if IsIconic(hwnd).as_bool() {
                SW_RESTORE
            } else {
                SW_SHOW
            },
        );
        let _ = SetForegroundWindow(hwnd);
    }
    Ok(())
}

/// Shows new settings values (after a change from either surface).
pub fn sync(settings: OecaSettings) {
    // Flipped switches slide (at once under Reduced motion), on a frame timer
    // that stops when they settle
    let instant = !crate::window::motion_allowed(
        settings.reduced_motion,
        crate::window::is_client_animation_enabled(),
    );
    let changed = with_view(|v| {
        let old = std::mem::replace(&mut v.settings, settings);
        for c in Control::ALL {
            if c.is_on(old) != c.is_on(settings) {
                v.switches[c.index()].set(c.is_on(settings), instant);
            }
        }
        if v.switch_tick.is_none() && v.switches.iter().any(|s| s.is_animating()) {
            v.switch_tick = Some(std::time::Instant::now());
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::SetTimer(
                    Some(v.hwnd),
                    SWITCH_TIMER_ID,
                    crate::config::SWITCH_FRAME_MS,
                    None,
                );
            }
        }
        (v.hwnd, old)
    });
    if let Some((hwnd, old)) = changed {
        invalidate(hwnd);
        access::toggled(hwnd, old, settings);
    }
}

/// Destroys the Settings window if it is open (Oeca shutting down).
pub fn close() {
    if let Some(hwnd) = window_handle() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

/// Shows a category page (sidebar click, keyboard or UI Automation).
fn navigate(hwnd: HWND, category: Category) {
    if with_view(|v| v.nav.select(category)) == Some(true) {
        invalidate(hwnd);
        access::page_changed(hwnd, category);
    }
}

fn create(notch: HWND, settings: OecaSettings) -> Result<HWND> {
    let instance = unsafe { GetModuleHandleW(None)? };
    if !REGISTERED.get() {
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW)? },
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        // The class is process-wide: another thread may have registered it
        if unsafe { RegisterClassExW(&class) } == 0 {
            let e = windows::core::Error::from_thread();
            if e.code() != windows::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS.to_hresult() {
                return Err(e);
            }
        }
        REGISTERED.set(true);
    }
    // On the notch's monitor, sized at that monitor's DPI, below the band the
    // expanded notch can cover. Only here, at creation: an open window keeps
    // wherever the user moves it.
    let (dpi, work, monitor_top) = notch_monitor(notch);
    let (w, h) = outer_size(CLIENT_SIZE, dpi);
    let (x, y) = initial_position(work, (w, h), notch_clear_top(monitor_top, dpi));
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_APPWINDOW,
            CLASS_NAME,
            TITLE,
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(instance.into()),
            None,
        )?
    };
    // Dark title bar to match the content (cosmetic: ignored where unsupported)
    let dark = windows::core::BOOL(1);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as *const _,
            std::mem::size_of_val(&dark) as u32,
        );
    }
    let view = (|| -> Result<View> {
        let d2d: ID2D1Factory =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let format = |role| role_format(&dwrite, role);
        let centred = |f: IDWriteTextFormat| -> Result<IDWriteTextFormat> {
            unsafe { f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)? };
            Ok(f)
        };
        let item = |role| -> Result<IDWriteTextFormat> {
            let f = format(role)?;
            unsafe { f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)? };
            Ok(f)
        };
        Ok(View {
            hwnd,
            notch,
            settings,
            nav: Nav::default(),
            hover: None,
            pressed: None,
            d2d,
            formats: Formats {
                heading: format(TextRole::PageTitle)?,
                body: format(TextRole::SettingLabel)?,
                caption: format(TextRole::SettingDescription)?,
                section: {
                    let f = format(TextRole::SectionHeading)?;
                    unsafe { f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_FAR)? };
                    f
                },
                item: item(TextRole::SidebarItem)?,
                item_selected: item(TextRole::SidebarItemSelected)?,
                segment: centred(item(TextRole::SidebarItem)?)?,
                segment_selected: centred(item(TextRole::SidebarItemSelected)?)?,
            },
            target: None,
            placement: Some((x, y)),
            corrections: 0,
            switches: Control::ALL.map(|c| crate::renderer::SwitchAnim::settled(c.is_on(settings))),
            switch_tick: None,
        })
    })();
    match view {
        Ok(view) => {
            VIEW.with(|v| *v.borrow_mut() = Some(view));
            Ok(hwnd)
        }
        Err(e) => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            Err(e)
        }
    }
}

/// The notch's monitor (the primary one without a notch window): its DPI,
/// work area and top edge (physical px).
fn notch_monitor(notch: HWND) -> (u32, RECT, i32) {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    unsafe {
        let monitor = MonitorFromWindow(notch, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let (mut dx, mut dy) = (0, 0);
        let dpi = match GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) {
            Ok(()) if dx > 0 => dx,
            _ => GetDpiForSystem(),
        };
        (dpi, info.rcWork, info.rcMonitor.top)
    }
}

/// Bottom (physical px) of the band the notch covers at the top of its
/// monitor when expanded (its tallest space, shadow included), plus a gap.
pub fn notch_clear_top(monitor_top: i32, dpi: u32) -> i32 {
    const GAP: f32 = 16.0;
    let tallest = crate::space::OecaSpace::ALL
        .into_iter()
        .map(|s| {
            crate::layout::space_dimensions(crate::config::NotchState::Expanded, dpi, s).height
        })
        .max()
        .unwrap_or(0);
    monitor_top + tallest + (GAP * dpi as f32 / 96.0).round() as i32
}

/// Initial outer position (physical px) of a window of `size`: centred
/// horizontally in the work area, its top at `clear_top` (clear of the
/// notch), moved up only as far as needed to stay inside the work area, and
/// never above it.
pub fn initial_position(work: RECT, size: (i32, i32), clear_top: i32) -> (i32, i32) {
    let x = work.left + ((work.right - work.left - size.0) / 2).max(0);
    let y = clear_top
        .max(work.top)
        .min(work.bottom - size.1)
        .max(work.top);
    (x, y)
}

/// Most external corrections made for one window (guards against a
/// repositioning tool that insists).
const MAX_PLACEMENT_CORRECTIONS: u8 = 3;

/// Where to put the window back after it moved to `moved_to`, if anywhere:
/// only while Oeca's placement still stands (`placement`, cleared once the
/// user interacts), only for a move made by another process or thread
/// (`external`; Oeca's own moves and the user's are never undone), and at
/// most `MAX_PLACEMENT_CORRECTIONS` times. Some shells re-centre a new window
/// that opens without the foreground (as Oeca's does: the notch never
/// activates), which would put its title bar under the expanded notch.
pub fn placement_correction(
    placement: Option<(i32, i32)>,
    corrections: u8,
    moved_to: (i32, i32),
    external: bool,
) -> Option<(i32, i32)> {
    let target = placement?;
    (external && moved_to != target && corrections < MAX_PLACEMENT_CORRECTIONS).then_some(target)
}

/// Window size for a client size (DIP) at `dpi`, frame included.
fn outer_size(client: (f32, f32), dpi: u32) -> (i32, i32) {
    let (w, h) = to_px(client, dpi);
    let mut r = RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };
    unsafe {
        let _ = AdjustWindowRectExForDpi(&mut r, WS_OVERLAPPEDWINDOW, false, WS_EX_APPWINDOW, dpi);
    }
    (r.right - r.left, r.bottom - r.top)
}

fn text_format(
    dwrite: &IDWriteFactory,
    family: PCWSTR,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
) -> Result<IDWriteTextFormat> {
    let locale = w!("en-us");
    let make = |family| unsafe {
        dwrite.CreateTextFormat(
            family,
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            locale,
        )
    };
    // A missing family never fails here; `ui_font` resolved it beforehand
    let format = make(family)?;
    unsafe { format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)? };
    Ok(format)
}

/// A role's format in Oeca's typeface (sizes are DIP: the window draws at
/// its own DPI).
fn role_format(dwrite: &IDWriteFactory, role: TextRole) -> Result<IDWriteTextFormat> {
    let style = role.style();
    text_format(
        dwrite,
        crate::renderer::ui_font(dwrite).1,
        style.size,
        DWRITE_FONT_WEIGHT(style.weight),
    )
}

fn scale(hwnd: HWND) -> f32 {
    unsafe { GetDpiForWindow(hwnd) as f32 / 96.0 }
}

/// Client size in DIP.
fn client_size(hwnd: HWND) -> (f32, f32) {
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut r);
    }
    let s = scale(hwnd);
    (r.right as f32 / s, r.bottom as f32 / s)
}

/// What a client point (physical px) lands on, with the current page showing.
fn hit_px(hwnd: HWND, x: f32, y: f32) -> Option<Target> {
    let s = scale(hwnd);
    let (w, h) = client_size(hwnd);
    let category = with_view(|v| v.nav.category).unwrap_or_default();
    hit(w, h, category, x / s, y / s)
}

/// Client point (physical px) of a mouse message.
fn point(lparam: LPARAM) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
    (x, y)
}

/// Asks the notch to flip a setting (it owns the value and syncs back).
fn request_toggle(control: Control) {
    if let Some(notch) = with_view(|v| v.notch) {
        unsafe {
            let _ = PostMessageW(
                Some(notch),
                WM_APP_SETTINGS_TOGGLE,
                WPARAM(control.index()),
                LPARAM(0),
            );
        }
    }
}

/// Asks the notch to use an accent (it owns the value and syncs back).
fn request_accent(accent: AccentChoice) {
    if let Some(notch) = with_view(|v| v.notch) {
        unsafe {
            let _ = PostMessageW(
                Some(notch),
                WM_APP_SETTINGS_ACCENT,
                WPARAM(accent.index()),
                LPARAM(0),
            );
        }
    }
}

/// Asks the notch to keep a history capacity (it owns the value, trims its
/// history and syncs back).
fn request_capacity(capacity: ClipboardCapacity) {
    if let Some(notch) = with_view(|v| v.notch) {
        unsafe {
            let _ = PostMessageW(
                Some(notch),
                WM_APP_SETTINGS_CAPACITY,
                WPARAM(capacity.index()),
                LPARAM(0),
            );
        }
    }
}

/// Asks the notch to use a closing delay (it owns the value and syncs back).
fn request_delay(delay: CollapseDelay) {
    if let Some(notch) = with_view(|v| v.notch) {
        unsafe {
            let _ = PostMessageW(
                Some(notch),
                WM_APP_SETTINGS_DELAY,
                WPARAM(delay.index()),
                LPARAM(0),
            );
        }
    }
}

/// Click / Enter / Space on a target: show its page, flip its switch, or
/// choose its accent / capacity / closing delay.
fn activate(hwnd: HWND, target: Target) {
    match target {
        Target::Category(c) => navigate(hwnd, c),
        Target::Control(c) => request_toggle(c),
        Target::Accent(a) => request_accent(a),
        Target::Capacity(c) => request_capacity(c),
        Target::Delay(d) => request_delay(d),
    }
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // The user (or a DPI change) takes over the position: Oeca's initial
    // placement is no longer defended
    if matches!(
        msg,
        WM_ENTERSIZEMOVE
            | WM_NCLBUTTONDOWN
            | WM_LBUTTONDOWN
            | WM_KEYDOWN
            | WM_SYSKEYDOWN
            | WM_DPICHANGED
    ) {
        with_view(|v| v.placement = None);
    }
    // Oeca is in use: the notch's pending auto-collapse (if any) is cancelled
    let activated =
        msg == windows::Win32::UI::WindowsAndMessaging::WM_ACTIVATE && (wparam.0 & 0xFFFF) != 0;
    if (activated || matches!(msg, WM_LBUTTONDOWN | WM_KEYDOWN | WM_SYSKEYDOWN))
        && let Some(notch) = with_view(|v| v.notch)
    {
        {
            unsafe {
                let _ = PostMessageW(
                    Some(notch),
                    crate::window::WM_APP_GRACE,
                    WPARAM(crate::window::GraceEvent::OecaInteraction as usize),
                    LPARAM(0),
                );
            }
        }
    }
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            unsafe { BeginPaint(hwnd, &mut ps) };
            with_view(|v| {
                if paint(v).is_err() {
                    v.target = None;
                }
            });
            unsafe {
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        windows::Win32::UI::WindowsAndMessaging::WM_TIMER if wparam.0 == SWITCH_TIMER_ID => {
            let moving = with_view(|v| {
                let now = std::time::Instant::now();
                let dt = v
                    .switch_tick
                    .replace(now)
                    .map_or(0.0, |t| now.duration_since(t).as_secs_f32() * 1000.0);
                let mut any = false;
                for s in v.switches.iter_mut() {
                    any |= s.step(dt);
                }
                if !any {
                    v.switch_tick = None;
                }
                any
            });
            invalidate(hwnd);
            // Settled (or the window is going): no more frames
            if moving != Some(true) {
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::KillTimer(
                        Some(hwnd),
                        SWITCH_TIMER_ID,
                    );
                }
            }
            LRESULT(0)
        }
        WM_WINDOWPOSCHANGED => {
            let wp = unsafe { &*(lparam.0 as *const WINDOWPOS) };
            // Sent from another thread / process (Oeca moves its own window
            // on this thread only)
            let external = unsafe { InSendMessageEx(None) } & (ISMEX_SEND | ISMEX_NOTIFY) != 0;
            if wp.flags & SWP_NOMOVE == SET_WINDOW_POS_FLAGS(0) {
                let back = with_view(|v| {
                    let back =
                        placement_correction(v.placement, v.corrections, (wp.x, wp.y), external);
                    v.corrections += u8::from(back.is_some());
                    back
                })
                .flatten();
                if let Some((x, y)) = back {
                    unsafe {
                        let _ = SetWindowPos(
                            hwnd,
                            None,
                            x,
                            y,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                }
            }
            // Default handling still sends WM_MOVE / WM_SIZE
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_SIZE => {
            let (w, h) = (
                (lparam.0 & 0xFFFF) as u32,
                ((lparam.0 >> 16) & 0xFFFF) as u32,
            );
            with_view(|v| {
                if let Some(t) = &v.target {
                    let _ = unsafe {
                        t.Resize(&D2D_SIZE_U {
                            width: w,
                            height: h,
                        })
                    };
                }
            });
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // Move to the suggested rect; the target is rebuilt at the new DPI
            let r = unsafe { *(lparam.0 as *const RECT) };
            with_view(|v| v.target = None);
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            if dpi > 0 {
                let (w, h) = outer_size(MIN_CLIENT_SIZE, dpi);
                let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
                info.ptMinTrackSize = POINT { x: w, y: h };
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = point(lparam);
            let target = hit_px(hwnd, x, y);
            let changed = with_view(|v| std::mem::replace(&mut v.hover, target) != target);
            if changed == Some(true) {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                unsafe {
                    let _ = TrackMouseEvent(&mut tme);
                }
                invalidate(hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_view(|v| v.hover = None);
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point(lparam);
            if let Some(target) = hit_px(hwnd, x, y) {
                with_view(|v| {
                    v.pressed = Some(target);
                    v.nav.focus = target;
                    v.nav.focus_visible = false;
                });
                unsafe { SetCapture(hwnd) };
                invalidate(hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = point(lparam);
            let target = hit_px(hwnd, x, y);
            let pressed = with_view(|v| v.pressed.take()).flatten();
            if pressed.is_some() {
                unsafe {
                    let _ = ReleaseCapture();
                }
                invalidate(hwnd);
            }
            // Fires only if released over the target that was pressed
            if let Some(t) = pressed.filter(|p| Some(*p) == target) {
                activate(hwnd, t);
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            if with_view(|v| v.pressed.take()).flatten().is_some() {
                invalidate(hwnd);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let key = wparam.0 as u16;
            if key == VK_ESCAPE.0 {
                // Only while the setting allows it; the title-bar X always works
                if with_view(|v| v.settings.close_settings_on_escape) == Some(true) {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            } else if key == VK_SPACE.0 || key == VK_RETURN.0 {
                let focus = with_view(|v| {
                    v.nav.focus_visible = true;
                    v.nav.focus
                });
                if let Some(focus) = focus {
                    activate(hwnd, focus);
                }
                invalidate(hwnd);
            } else if key == VK_TAB.0
                || key == VK_UP.0
                || key == VK_DOWN.0
                || key == VK_LEFT.0
                || key == VK_RIGHT.0
            {
                let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
                let focus = with_view(|v| match key {
                    k if k == VK_TAB.0 => Some(v.nav.tab(shift)),
                    // Left / Right only along the swatch / segment rows
                    k if k == VK_LEFT.0 || k == VK_RIGHT.0 => matches!(
                        v.nav.focus,
                        Target::Accent(_) | Target::Capacity(_) | Target::Delay(_)
                    )
                    .then(|| v.nav.arrow(k == VK_LEFT.0)),
                    k => Some(v.nav.arrow(k == VK_UP.0)),
                })
                .flatten();
                if let Some(focus) = focus {
                    access::focused(hwnd, focus);
                }
                invalidate(hwnd);
            }
            LRESULT(0)
        }
        WM_GETOBJECT => access::answer(hwnd, wparam, lparam)
            .unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }),
        WM_DESTROY => {
            // Only this window goes: Oeca (and its message loop) keeps running
            access::disconnect(hwnd);
            VIEW.with(|v| v.borrow_mut().take());
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn rounded(rect: RectF, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: rect.to_d2d_rect(),
        radiusX: radius,
        radiusY: radius,
    }
}

fn paint(v: &mut View) -> Result<()> {
    let dpi = unsafe { GetDpiForWindow(v.hwnd) } as f32;
    if v.target.is_none() {
        let mut r = RECT::default();
        unsafe { GetClientRect(v.hwnd, &mut r)? };
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            dpiX: dpi,
            dpiY: dpi,
            ..Default::default()
        };
        let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd: v.hwnd,
            pixelSize: D2D_SIZE_U {
                width: r.right as u32,
                height: r.bottom as u32,
            },
            presentOptions: D2D1_PRESENT_OPTIONS_NONE,
        };
        v.target = Some(unsafe { v.d2d.CreateHwndRenderTarget(&props, &hwnd_props)? });
    }
    let Some(target) = v.target.clone() else {
        return Ok(());
    };
    let rt: &ID2D1RenderTarget = &target;
    let (width, height) = client_size(v.hwnd);
    let l = layout(width, height);
    let category = v.nav.category;
    unsafe {
        rt.BeginDraw();
        rt.Clear(Some(&COLOR_BACKGROUND));
        let brush = |c: &D2D1_COLOR_F| rt.CreateSolidColorBrush(c, None);
        let primary = brush(&COLOR_TEXT_PRIMARY)?;
        let secondary = brush(&COLOR_TEXT_SECONDARY)?;
        let tertiary = brush(&COLOR_TEXT_TERTIARY)?;
        let separator = brush(&COLOR_SEPARATOR)?;
        let selected_fill = brush(&COLOR_SPACE_PILL_SELECTED)?;
        let hover_fill = brush(&COLOR_HOVER)?;
        let accent = brush(&v.settings.accent.color())?;
        let text = |s: &str, f: &IDWriteTextFormat, r: RectF, b: &ID2D1SolidColorBrush| {
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
        let focus_ring = |r: RectF, radius: f32| {
            rt.DrawRoundedRectangle(&rounded(r, radius), &primary, 1.5, None);
        };

        // Sidebar: darker panel with a hairline edge; the selected entry is
        // filled, bold and marked by a leading bar (not by color alone)
        rt.FillRectangle(&l.sidebar.to_d2d_rect(), &brush(&COLOR_SIDEBAR)?);
        let edge = RectF::new(l.sidebar.right - 1.0, 0.0, l.sidebar.right, height);
        rt.FillRectangle(&edge.to_d2d_rect(), &separator);
        for c in Category::ALL {
            let r = l.items[c.index()];
            let selected = c == category;
            let target = Target::Category(c);
            if selected {
                rt.FillRoundedRectangle(&rounded(r, 8.0), &selected_fill);
                let bar = RectF::new(r.left + 3.0, r.top + 10.0, r.left + 6.0, r.bottom - 10.0);
                rt.FillRoundedRectangle(&rounded(bar, 1.5), &accent);
            } else if v.hover == Some(target) || v.pressed == Some(target) {
                rt.FillRoundedRectangle(&rounded(r, 8.0), &hover_fill);
            }
            let ink = if selected { &primary } else { &secondary };
            let icon_cx = r.left + 14.0 + ITEM_ICON / 2.0;
            fill_glyph(
                &v.d2d,
                rt,
                c.icon(),
                icon_cx,
                (r.top + r.bottom) / 2.0,
                ITEM_ICON,
                ink,
            )?;
            let label = RectF::new(r.left + 44.0, r.top, r.right - 8.0, r.bottom);
            let format = if selected {
                &v.formats.item_selected
            } else {
                &v.formats.item
            };
            text(c.title(), format, label, ink);
            if v.nav.focus_visible && v.nav.focus == target {
                focus_ring(r, 8.0);
            }
        }

        // Page: title, description, then its settings
        text(category.title(), &v.formats.heading, l.page_title, &primary);
        text(
            category.description(),
            &v.formats.body,
            l.page_description,
            &tertiary,
        );
        // Segmented selector: a quiet track; the chosen segment is a filled
        // pill with a bold label (not color alone)
        let segmented = |rects: [RectF; 3],
                         labels: [&str; 3],
                         chosen: usize,
                         targets: [Target; 3]| {
            let (first, last) = (rects[0], rects[2]);
            let track = RectF::new(first.left, first.top, last.right, last.bottom);
            rt.FillRoundedRectangle(&rounded(track, track.height() / 2.0), &hover_fill);
            for i in 0..3 {
                let (r, target) = (rects[i], targets[i]);
                let pill = RectF::new(r.left + 2.0, r.top + 2.0, r.right - 2.0, r.bottom - 2.0);
                if i == chosen {
                    rt.FillRoundedRectangle(&rounded(pill, pill.height() / 2.0), &selected_fill);
                } else if v.hover == Some(target) || v.pressed == Some(target) {
                    rt.FillRoundedRectangle(&rounded(pill, pill.height() / 2.0), &hover_fill);
                }
                let (format, ink) = if i == chosen {
                    (&v.formats.segment_selected, &primary)
                } else {
                    (&v.formats.segment, &secondary)
                };
                text(labels[i], format, r, ink);
                if v.nav.focus_visible && v.nav.focus == target {
                    focus_ring(r, r.height() / 2.0);
                }
            }
        };
        if category == Category::General {
            for ((heading, _), r) in SECTIONS.iter().zip(l.sections) {
                text(heading, &v.formats.section, r, &secondary);
            }
            text(CLOSING_TITLE, &v.formats.body, l.closing_title, &primary);
            text(
                CLOSING_DESCRIPTION,
                &v.formats.caption,
                l.closing_description,
                &tertiary,
            );
            segmented(
                l.closing,
                CollapseDelay::ALL.map(CollapseDelay::label),
                v.settings.collapse_delay.index(),
                CollapseDelay::ALL.map(Target::Delay),
            );
            let r = l.closing_row;
            let line = RectF::new(r.left, r.bottom - 1.0, r.right, r.bottom);
            rt.FillRectangle(&line.to_d2d_rect(), &separator);
        }
        if category == Category::Media {
            text(MEDIA_SECTION, &v.formats.section, l.sections[0], &secondary);
        }
        if category == Category::Clipboard {
            text(
                CLIPBOARD_SECTION,
                &v.formats.section,
                l.sections[0],
                &secondary,
            );
            text(CAPACITY_TITLE, &v.formats.body, l.capacity_title, &primary);
            text(
                CAPACITY_DESCRIPTION,
                &v.formats.caption,
                l.capacity_description,
                &tertiary,
            );
            segmented(
                l.capacity,
                ["5", "10", "20"],
                v.settings.clipboard_capacity.index(),
                ClipboardCapacity::ALL.map(Target::Capacity),
            );
            let separator_line = RectF::new(
                l.rows[0].row.left,
                l.rows[0].row.bottom - 1.0,
                l.rows[0].row.right,
                l.rows[0].row.bottom,
            );
            rt.FillRectangle(&separator_line.to_d2d_rect(), &separator);
            for (note, r) in CLIPBOARD_NOTES.iter().zip(l.clipboard_notes) {
                text(note, &v.formats.caption, r, &tertiary);
            }
        }
        if category == Category::Appearance {
            for (heading, r) in APPEARANCE_SECTIONS.iter().zip(l.appearance_sections) {
                text(heading, &v.formats.section, r, &secondary);
            }
            // Swatches: a filled disc each; the chosen one is ringed and named
            // below (not marked by color alone); hover lifts a soft halo
            let chosen = v.settings.accent;
            for a in AccentChoice::ALL {
                let r = l.swatches[a.index()];
                let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
                let disc =
                    |d: f32| RectF::new(cx - d / 2.0, cy - d / 2.0, cx + d / 2.0, cy + d / 2.0);
                let target = Target::Accent(a);
                if v.hover == Some(target) || v.pressed == Some(target) {
                    rt.FillRoundedRectangle(&rounded(disc(34.0), 17.0), &hover_fill);
                }
                if a == chosen {
                    rt.DrawRoundedRectangle(&rounded(disc(30.0), 15.0), &primary, 2.0, None);
                }
                let d = if v.pressed == Some(target) {
                    20.0
                } else {
                    22.0
                };
                rt.FillRoundedRectangle(&rounded(disc(d), d / 2.0), &brush(&a.color())?);
                if v.nav.focus_visible && v.nav.focus == target {
                    focus_ring(disc(36.0), 18.0);
                }
            }
            text(chosen.name(), &v.formats.caption, l.accent_name, &secondary);
            // Preview: Oeca's black surface with an "on" switch and a
            // progress track in the accent (the real switch drawing)
            let p = l.preview;
            rt.FillRoundedRectangle(&rounded(p, 14.0), &brush(&NOTCH_BG_COLOR)?);
            let sw = RectF::new(p.right - 56.0, p.top + 17.0, p.right - 16.0, p.top + 39.0);
            crate::renderer::draw_switch(rt, sw, 1.0, 3.0, 1.0, chosen)?;
            let track = RectF::new(p.left + 18.0, p.top + 26.0, sw.left - 24.0, p.top + 30.0);
            rt.FillRoundedRectangle(&rounded(track, 2.0), &selected_fill);
            let played = RectF::new(
                track.left,
                track.top,
                track.left + track.width() * 0.6,
                track.bottom,
            );
            rt.FillRoundedRectangle(&rounded(played, 2.0), &accent);
        }
        for control in category.controls() {
            let row = l.rows[control.index()];
            text(control.title(), &v.formats.body, row.title, &primary);
            text(
                control.description(),
                &v.formats.caption,
                row.description,
                &tertiary,
            );
            // Separator under each row
            let line = RectF::new(
                row.row.left,
                row.row.bottom - 1.0,
                row.row.right,
                row.row.bottom,
            );
            rt.FillRectangle(&line.to_d2d_rect(), &separator);
            // Hover: a soft halo; press: the knob tucks in a little
            let s = row.switch;
            let target = Target::Control(*control);
            if v.hover == Some(target) || v.pressed == Some(target) {
                let halo = RectF::new(s.left - 3.0, s.top - 3.0, s.right + 3.0, s.bottom + 3.0);
                rt.FillRoundedRectangle(&rounded(halo, halo.height() / 2.0), &selected_fill);
            }
            let knob = if v.pressed == Some(target) { 0.82 } else { 1.0 };
            crate::renderer::draw_switch(
                rt,
                s,
                v.switches[control.index()].level(),
                3.0,
                knob,
                v.settings.accent,
            )?;
            if v.nav.focus_visible && v.nav.focus == target {
                let f = RectF::new(s.left - 4.0, s.top - 4.0, s.right + 4.0, s.bottom + 4.0);
                focus_ring(f, f.height() / 2.0);
            }
        }
        match rt.EndDraw(None, None) {
            Err(e) if e.code() == D2DERR_RECREATE_TARGET => v.target = None,
            r => r?,
        }
    }
    Ok(())
}

/// Screen rectangle (physical px) of an element, for UI Automation.
fn screen_rect(hwnd: HWND, r: RectF) -> (f64, f64, f64, f64) {
    let s = scale(hwnd);
    let mut origin = POINT { x: 0, y: 0 };
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut origin);
    }
    (
        (origin.x as f32 + r.left * s) as f64,
        (origin.y as f32 + r.top * s) as f64,
        (r.width() * s) as f64,
        (r.height() * s) as f64,
    )
}

/// The accessible elements under the window root, in order: the five sidebar
/// entries, the page title, then (General) each section heading followed by
/// its switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Element {
    Category(Category),
    PageTitle,
    /// A section heading: General's (0, 1: `SECTIONS`), then Appearance's
    /// (2, 3: `APPEARANCE_SECTIONS`).
    Section(usize),
    Control(Control),
    /// An Appearance swatch (a radio button: selected = the current accent).
    Accent(AccentChoice),
    /// A Clipboard capacity segment (a radio button: selected = the current
    /// capacity).
    Capacity(ClipboardCapacity),
    /// A Clipboard informational line (index into `CLIPBOARD_NOTES`).
    Note(usize),
    /// A General closing-delay segment (a radio button: selected = the
    /// current delay).
    Delay(CollapseDelay),
}

impl From<Target> for Element {
    fn from(t: Target) -> Self {
        match t {
            Target::Category(c) => Self::Category(c),
            Target::Control(c) => Self::Control(c),
            Target::Accent(a) => Self::Accent(a),
            Target::Capacity(c) => Self::Capacity(c),
            Target::Delay(d) => Self::Delay(d),
        }
    }
}

impl Element {
    /// Children of the root while `category` is showing.
    pub fn children(category: Category) -> Vec<Self> {
        Category::ALL
            .into_iter()
            .map(Self::Category)
            .chain(std::iter::once(Self::PageTitle))
            .chain(
                SECTIONS
                    .iter()
                    .enumerate()
                    .filter(|_| category == Category::General)
                    .flat_map(|(k, (_, controls))| {
                        // "Notch" ends with the closing-delay segments
                        let delays: &[CollapseDelay] =
                            if k == 0 { &CollapseDelay::ALL } else { &[] };
                        std::iter::once(Self::Section(k))
                            .chain(controls.iter().map(|c| Self::Control(*c)))
                            .chain(delays.iter().map(|d| Self::Delay(*d)))
                    }),
            )
            .chain(
                (category == Category::Appearance)
                    .then(|| {
                        std::iter::once(Self::Section(2))
                            .chain(AccentChoice::ALL.map(Self::Accent))
                            .chain(std::iter::once(Self::Section(3)))
                    })
                    .into_iter()
                    .flatten(),
            )
            .chain(
                (category == Category::Clipboard)
                    .then(|| {
                        std::iter::once(Self::Section(4))
                            .chain(ClipboardCapacity::ALL.map(Self::Capacity))
                            .chain((0..CLIPBOARD_NOTES.len()).map(Self::Note))
                    })
                    .into_iter()
                    .flatten(),
            )
            .chain(
                (category == Category::Media)
                    .then(|| {
                        std::iter::once(Self::Section(5))
                            .chain(Category::Media.controls().iter().map(|c| Self::Control(*c)))
                    })
                    .into_iter()
                    .flatten(),
            )
            .collect()
    }

    /// Accessible name.
    pub fn name(self, category: Category) -> &'static str {
        match self {
            Self::Category(c) => c.title(),
            Self::PageTitle => category.title(),
            Self::Section(k) if k < SECTIONS.len() => SECTIONS[k].0,
            Self::Section(4) => CLIPBOARD_SECTION,
            Self::Section(5) => MEDIA_SECTION,
            Self::Section(k) => APPEARANCE_SECTIONS[k - SECTIONS.len()],
            Self::Control(c) => c.title(),
            Self::Accent(a) => a.name(),
            Self::Capacity(c) => c.name(),
            Self::Note(i) => CLIPBOARD_NOTES[i],
            Self::Delay(d) => d.name(),
        }
    }

    /// Selected state (SelectionItem): the showing page's sidebar entry, and
    /// the swatch of the current accent.
    pub fn is_selected(self, showing: Category, settings: OecaSettings) -> bool {
        match self {
            Self::Category(c) => c == showing,
            Self::Accent(a) => a == settings.accent,
            Self::Capacity(c) => c == settings.clipboard_capacity,
            Self::Delay(d) => d == settings.collapse_delay,
            _ => false,
        }
    }

    /// Stable runtime id suffix (unique per element kind and item).
    fn runtime_id(self) -> i32 {
        match self {
            Self::Category(c) => 1 + c.index() as i32,
            Self::PageTitle => 100,
            Self::Section(k) => 150 + k as i32,
            Self::Accent(a) => 300 + a.index() as i32,
            Self::Capacity(c) => 400 + c.index() as i32,
            Self::Note(i) => 500 + i as i32,
            Self::Delay(d) => 600 + d.index() as i32,
            Self::Control(c) => 200 + c.index() as i32,
        }
    }
}

/// UI Automation: the window is a fragment root whose children are the
/// sidebar entries (tab items with the SelectionItem pattern: name, selected
/// state, select), the page title (a level-1 heading) and General's switches
/// (Button control type with the Toggle pattern, like a toggle switch). Every
/// action goes through the same paths as mouse and keyboard.
#[allow(non_upper_case_globals)]
mod access {
    use super::*;
    use windows::Win32::System::Com::SAFEARRAY;
    use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayPutElement};
    use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_BSTR, VT_I4};
    use windows::Win32::UI::Accessibility::*;
    use windows::core::{BSTR, IUnknown, Interface, implement};

    fn empty<T>() -> Result<T> {
        Err(windows::core::Error::empty())
    }

    fn var_i32(v: i32) -> VARIANT {
        let mut x = VARIANT::default();
        unsafe {
            (*x.Anonymous.Anonymous).vt = VT_I4;
            (*x.Anonymous.Anonymous).Anonymous.lVal = v;
        }
        x
    }

    fn var_bool(v: bool) -> VARIANT {
        let mut x = VARIANT::default();
        unsafe {
            (*x.Anonymous.Anonymous).vt = VT_BOOL;
            (*x.Anonymous.Anonymous).Anonymous.boolVal =
                windows::Win32::Foundation::VARIANT_BOOL(if v { -1 } else { 0 });
        }
        x
    }

    fn var_str(s: &str) -> VARIANT {
        let mut x = VARIANT::default();
        unsafe {
            (*x.Anonymous.Anonymous).vt = VT_BSTR;
            (*x.Anonymous.Anonymous).Anonymous.bstrVal = std::mem::ManuallyDrop::new(BSTR::from(s));
        }
        x
    }

    fn toggle_state(on: bool) -> ToggleState {
        if on { ToggleState_On } else { ToggleState_Off }
    }

    fn category() -> Category {
        with_view(|v| v.nav.category).unwrap_or_default()
    }

    #[implement(
        IRawElementProviderSimple,
        IRawElementProviderFragment,
        IRawElementProviderFragmentRoot
    )]
    struct Root {
        hwnd: HWND,
    }

    #[implement(
        IRawElementProviderSimple,
        IRawElementProviderFragment,
        IToggleProvider,
        ISelectionItemProvider
    )]
    struct Item {
        hwnd: HWND,
        element: Element,
    }

    fn root(hwnd: HWND) -> IRawElementProviderSimple {
        Root { hwnd }.into()
    }

    fn item(hwnd: HWND, element: Element) -> IRawElementProviderSimple {
        Item { hwnd, element }.into()
    }

    fn fragment(p: IRawElementProviderSimple) -> Result<IRawElementProviderFragment> {
        p.cast()
    }

    /// WM_GETOBJECT: hands UI Automation the window's root provider.
    pub(super) fn answer(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        if lparam.0 as i32 != UiaRootObjectId || !super::is_open() {
            return None;
        }
        Some(unsafe { UiaReturnRawElementProvider(hwnd, wparam, lparam, &root(hwnd)) })
    }

    /// The window is going away: release UI Automation's references.
    pub(super) fn disconnect(hwnd: HWND) {
        unsafe {
            let _ = UiaReturnRawElementProvider(hwnd, WPARAM(0), LPARAM(0), None);
        }
    }

    fn listening() -> bool {
        unsafe { UiaClientsAreListening().as_bool() }
    }

    /// Keyboard focus moved.
    pub(super) fn focused(hwnd: HWND, target: Target) {
        if listening() {
            let element = Element::from(target);
            unsafe {
                let _ = UiaRaiseAutomationEvent(
                    &item(hwnd, element),
                    UIA_AutomationFocusChangedEventId,
                );
            }
        }
    }

    /// A new page is showing: its entry is selected and the page's elements
    /// changed (screen readers announce the selection).
    pub(super) fn page_changed(hwnd: HWND, category: Category) {
        if !listening() {
            return;
        }
        unsafe {
            let _ = UiaRaiseAutomationEvent(
                &item(hwnd, Element::Category(category)),
                UIA_SelectionItem_ElementSelectedEventId,
            );
            let _ = UiaRaiseStructureChangedEvent(
                &root(hwnd),
                StructureChangeType_ChildrenInvalidated,
                std::ptr::null_mut(),
                0,
            );
        }
    }

    /// Settings changed: announce the switches whose state flipped, and a
    /// newly chosen accent.
    pub(super) fn toggled(hwnd: HWND, old: OecaSettings, new: OecaSettings) {
        if !listening() {
            return;
        }
        if old.accent != new.accent {
            unsafe {
                let _ = UiaRaiseAutomationEvent(
                    &item(hwnd, Element::Accent(new.accent)),
                    UIA_SelectionItem_ElementSelectedEventId,
                );
            }
        }
        if old.collapse_delay != new.collapse_delay {
            unsafe {
                let _ = UiaRaiseAutomationEvent(
                    &item(hwnd, Element::Delay(new.collapse_delay)),
                    UIA_SelectionItem_ElementSelectedEventId,
                );
            }
        }
        if old.clipboard_capacity != new.clipboard_capacity {
            unsafe {
                let _ = UiaRaiseAutomationEvent(
                    &item(hwnd, Element::Capacity(new.clipboard_capacity)),
                    UIA_SelectionItem_ElementSelectedEventId,
                );
            }
        }
        for c in Control::ALL {
            let (a, b) = (c.is_on(old), c.is_on(new));
            if a != b {
                unsafe {
                    let _ = UiaRaiseAutomationPropertyChangedEvent(
                        &item(hwnd, Element::Control(c)),
                        UIA_ToggleToggleStatePropertyId,
                        &var_i32(toggle_state(a).0),
                        &var_i32(toggle_state(b).0),
                    );
                }
            }
        }
    }

    /// Whether an element a client holds is still on screen: this window is
    /// the open one and the element belongs to the page showing. Elements of
    /// a page navigated away from (whose rects another page's controls may
    /// now occupy) or of a closed window are unavailable: no bounds, no
    /// actions.
    pub(super) fn is_live(hwnd: HWND, element: Element) -> bool {
        with_view(|v| v.hwnd == hwnd && Element::children(v.nav.category).contains(&element))
            == Some(true)
    }

    fn not_available<T>() -> Result<T> {
        Err(windows::core::HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32).into())
    }

    /// A provider for `element` of `hwnd` (tests drive it like a client).
    #[cfg(test)]
    pub(super) fn provider(hwnd: HWND, element: Element) -> IRawElementProviderSimple {
        item(hwnd, element)
    }

    fn options() -> ProviderOptions {
        ProviderOptions(ProviderOptions_ServerSideProvider.0 | ProviderOptions_UseComThreading.0)
    }

    impl IRawElementProviderSimple_Impl for Root_Impl {
        fn ProviderOptions(&self) -> Result<ProviderOptions> {
            Ok(options())
        }
        fn GetPatternProvider(&self, _: UIA_PATTERN_ID) -> Result<IUnknown> {
            empty()
        }
        fn GetPropertyValue(&self, id: UIA_PROPERTY_ID) -> Result<VARIANT> {
            // The host window supplies name ("Oeca Settings") and type
            Ok(if id == UIA_AutomationIdPropertyId {
                var_str("OecaSettings")
            } else {
                VARIANT::default()
            })
        }
        fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
            unsafe { UiaHostProviderFromHwnd(self.hwnd) }
        }
    }

    impl IRawElementProviderFragment_Impl for Root_Impl {
        fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
            let children = Element::children(category());
            let child = match direction {
                NavigateDirection_FirstChild => children.first(),
                NavigateDirection_LastChild => children.last(),
                _ => None,
            };
            match child {
                Some(e) => fragment(item(self.hwnd, *e)),
                None => empty(),
            }
        }
        fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
            Ok(std::ptr::null_mut())
        }
        fn BoundingRectangle(&self) -> Result<UiaRect> {
            Ok(UiaRect::default())
        }
        fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
            Ok(std::ptr::null_mut())
        }
        fn SetFocus(&self) -> Result<()> {
            Ok(())
        }
        fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
            root(self.hwnd).cast()
        }
    }

    impl IRawElementProviderFragmentRoot_Impl for Root_Impl {
        fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
            let mut p = POINT {
                x: x as i32,
                y: y as i32,
            };
            unsafe {
                let _ = ScreenToClient(self.hwnd, &mut p);
            }
            match super::hit_px(self.hwnd, p.x as f32, p.y as f32) {
                Some(t) => fragment(item(self.hwnd, Element::from(t))),
                None => fragment(root(self.hwnd)),
            }
        }
        fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
            match with_view(|v| v.nav.focus) {
                Some(t) => fragment(item(self.hwnd, Element::from(t))),
                None => empty(),
            }
        }
    }

    impl IRawElementProviderSimple_Impl for Item_Impl {
        fn ProviderOptions(&self) -> Result<ProviderOptions> {
            Ok(options())
        }
        fn GetPatternProvider(&self, id: UIA_PATTERN_ID) -> Result<IUnknown> {
            let wanted = matches!(
                (self.element, id),
                (Element::Control(_), UIA_TogglePatternId)
                    | (
                        Element::Category(_)
                            | Element::Accent(_)
                            | Element::Capacity(_)
                            | Element::Delay(_),
                        UIA_SelectionItemPatternId
                    )
            );
            if !wanted {
                return empty();
            }
            let me: IRawElementProviderSimple = item(self.hwnd, self.element);
            me.cast()
        }
        fn GetPropertyValue(&self, id: UIA_PROPERTY_ID) -> Result<VARIANT> {
            let showing = category();
            let e = self.element;
            let focus = with_view(|v| v.nav.focus);
            Ok(match (e, id) {
                (_, UIA_NamePropertyId) => var_str(e.name(showing)),
                (Element::Category(_), UIA_ControlTypePropertyId) => {
                    var_i32(UIA_TabItemControlTypeId.0)
                }
                (Element::Category(c), UIA_SelectionItemIsSelectedPropertyId) => {
                    var_bool(c == showing)
                }
                (Element::Category(c), UIA_HasKeyboardFocusPropertyId) => {
                    var_bool(focus == Some(Target::Category(c)))
                }
                (Element::Category(c), UIA_AutomationIdPropertyId) => var_str(c.title()),
                (Element::PageTitle, UIA_ControlTypePropertyId) => var_i32(UIA_TextControlTypeId.0),
                (Element::PageTitle, UIA_HeadingLevelPropertyId) => var_i32(HeadingLevel1.0),
                (Element::PageTitle, UIA_HelpTextPropertyId) => var_str(showing.description()),
                (Element::PageTitle, UIA_AutomationIdPropertyId) => var_str("PageTitle"),
                (Element::Section(_), UIA_ControlTypePropertyId) => {
                    var_i32(UIA_TextControlTypeId.0)
                }
                (Element::Section(_), UIA_HeadingLevelPropertyId) => var_i32(HeadingLevel2.0),
                (Element::Control(c), UIA_HelpTextPropertyId) => var_str(c.description()),
                (Element::Control(c), UIA_AutomationIdPropertyId) => var_str(c.automation_id()),
                (Element::Control(_), UIA_ControlTypePropertyId) => {
                    var_i32(UIA_ButtonControlTypeId.0)
                }
                (Element::Control(c), UIA_HasKeyboardFocusPropertyId) => {
                    var_bool(focus == Some(Target::Control(c)))
                }
                (Element::Accent(_), UIA_ControlTypePropertyId) => {
                    var_i32(UIA_RadioButtonControlTypeId.0)
                }
                (Element::Accent(_), UIA_SelectionItemIsSelectedPropertyId) => {
                    var_bool(with_view(|v| e.is_selected(showing, v.settings)) == Some(true))
                }
                (Element::Accent(a), UIA_HasKeyboardFocusPropertyId) => {
                    var_bool(focus == Some(Target::Accent(a)))
                }
                (Element::Accent(a), UIA_AutomationIdPropertyId) => {
                    var_str(&format!("Accent{}", a.name()))
                }
                (Element::Accent(_), UIA_HelpTextPropertyId) => var_str("Accent color"),
                (Element::Capacity(_), UIA_ControlTypePropertyId) => {
                    var_i32(UIA_RadioButtonControlTypeId.0)
                }
                (Element::Capacity(_), UIA_SelectionItemIsSelectedPropertyId) => {
                    var_bool(with_view(|v| e.is_selected(showing, v.settings)) == Some(true))
                }
                (Element::Capacity(c), UIA_HasKeyboardFocusPropertyId) => {
                    var_bool(focus == Some(Target::Capacity(c)))
                }
                (Element::Capacity(c), UIA_AutomationIdPropertyId) => {
                    var_str(&format!("Capacity{}", c.entries()))
                }
                (Element::Capacity(_), UIA_HelpTextPropertyId) => {
                    var_str("Clipboard history: items to keep")
                }
                (Element::Delay(_), UIA_ControlTypePropertyId) => {
                    var_i32(UIA_RadioButtonControlTypeId.0)
                }
                (Element::Delay(_), UIA_SelectionItemIsSelectedPropertyId) => {
                    var_bool(with_view(|v| e.is_selected(showing, v.settings)) == Some(true))
                }
                (Element::Delay(d), UIA_HasKeyboardFocusPropertyId) => {
                    var_bool(focus == Some(Target::Delay(d)))
                }
                (Element::Delay(d), UIA_AutomationIdPropertyId) => {
                    var_str(&format!("CloseAfter{}", d.ms() / 1000))
                }
                (Element::Delay(_), UIA_HelpTextPropertyId) => var_str(CLOSING_TITLE),
                (Element::Note(_), UIA_ControlTypePropertyId) => var_i32(UIA_TextControlTypeId.0),
                (
                    Element::Category(_)
                    | Element::Control(_)
                    | Element::Accent(_)
                    | Element::Capacity(_)
                    | Element::Delay(_),
                    UIA_IsKeyboardFocusablePropertyId,
                ) => var_bool(true),
                (_, UIA_IsEnabledPropertyId) => var_bool(true),
                (_, UIA_IsOffscreenPropertyId) => var_bool(!is_live(self.hwnd, e)),
                _ => VARIANT::default(),
            })
        }
        fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
            empty()
        }
    }

    impl IRawElementProviderFragment_Impl for Item_Impl {
        fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
            if direction == NavigateDirection_Parent {
                return fragment(root(self.hwnd));
            }
            // Siblings in the current page's element list (an element that left
            // with the previous page has none)
            let children = Element::children(category());
            let Some(i) = children.iter().position(|e| *e == self.element) else {
                return empty();
            };
            let sibling = match direction {
                NavigateDirection_NextSibling => children.get(i + 1),
                NavigateDirection_PreviousSibling => i.checked_sub(1).and_then(|j| children.get(j)),
                _ => None,
            };
            match sibling {
                Some(e) => fragment(item(self.hwnd, *e)),
                None => empty(),
            }
        }
        fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
            unsafe {
                let ids = SafeArrayCreateVector(VT_I4, 0, 2);
                if ids.is_null() {
                    return empty();
                }
                for (k, v) in [UiaAppendRuntimeId as i32, self.element.runtime_id()]
                    .iter()
                    .enumerate()
                {
                    SafeArrayPutElement(ids, &(k as i32), v as *const i32 as *const _)?;
                }
                Ok(ids)
            }
        }
        fn BoundingRectangle(&self) -> Result<UiaRect> {
            if !is_live(self.hwnd, self.element) {
                return Ok(UiaRect::default());
            }
            let (w, h) = super::client_size(self.hwnd);
            let l = layout(w, h);
            let r = match self.element {
                Element::Category(c) => l.items[c.index()],
                Element::PageTitle => l.page_title,
                Element::Section(k) if k < SECTIONS.len() => l.sections[k],
                Element::Section(4 | 5) => l.sections[0],
                Element::Capacity(c) => l.capacity[c.index()],
                Element::Delay(d) => l.closing[d.index()],
                Element::Note(i) => l.clipboard_notes[i],
                Element::Section(k) => l.appearance_sections[k - SECTIONS.len()],
                Element::Accent(a) => l.swatches[a.index()],
                Element::Control(c) => l.rows[c.index()].row,
            };
            let (left, top, width, height) = super::screen_rect(self.hwnd, r);
            Ok(UiaRect {
                left,
                top,
                width,
                height,
            })
        }
        fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
            Ok(std::ptr::null_mut())
        }
        fn SetFocus(&self) -> Result<()> {
            if !is_live(self.hwnd, self.element) {
                return not_available();
            }
            let target = match self.element {
                Element::Category(c) => Target::Category(c),
                Element::Control(c) => Target::Control(c),
                Element::Accent(a) => Target::Accent(a),
                Element::Capacity(c) => Target::Capacity(c),
                Element::Delay(d) => Target::Delay(d),
                Element::PageTitle | Element::Section(_) | Element::Note(_) => return Ok(()),
            };
            with_view(|v| v.nav.focus = target);
            super::invalidate(self.hwnd);
            Ok(())
        }
        fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
            root(self.hwnd).cast()
        }
    }

    impl IToggleProvider_Impl for Item_Impl {
        fn Toggle(&self) -> Result<()> {
            if !is_live(self.hwnd, self.element) {
                return not_available();
            }
            match self.element {
                Element::Control(c) => {
                    super::request_toggle(c);
                    Ok(())
                }
                _ => empty(),
            }
        }
        fn ToggleState(&self) -> Result<ToggleState> {
            match self.element {
                Element::Control(c) => Ok(toggle_state(
                    with_view(|v| c.is_on(v.settings)).unwrap_or(false),
                )),
                _ => empty(),
            }
        }
    }

    impl ISelectionItemProvider_Impl for Item_Impl {
        fn Select(&self) -> Result<()> {
            if !is_live(self.hwnd, self.element) {
                return not_available();
            }
            match self.element {
                Element::Category(c) => {
                    super::navigate(self.hwnd, c);
                    Ok(())
                }
                Element::Accent(a) => {
                    super::request_accent(a);
                    Ok(())
                }
                Element::Capacity(c) => {
                    super::request_capacity(c);
                    Ok(())
                }
                Element::Delay(d) => {
                    super::request_delay(d);
                    Ok(())
                }
                _ => empty(),
            }
        }
        fn AddToSelection(&self) -> Result<()> {
            self.Select()
        }
        fn RemoveFromSelection(&self) -> Result<()> {
            // Exactly one page (and one accent) is always selected
            empty()
        }
        fn IsSelected(&self) -> Result<windows::core::BOOL> {
            let e = self.element;
            Ok((with_view(|v| e.is_selected(v.nav.category, v.settings)) == Some(true)).into())
        }
        fn SelectionContainer(&self) -> Result<IRawElementProviderSimple> {
            Ok(root(self.hwnd))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW, WM_QUIT};

    #[test]
    fn test_five_categories_in_order_general_default() {
        assert_eq!(
            Category::ALL.map(Category::title),
            ["General", "Appearance", "Clipboard", "Media", "About"]
        );
        assert_eq!(Category::default(), Category::General);
        assert_eq!(Nav::default().category, Category::General);
        for (i, c) in Category::ALL.into_iter().enumerate() {
            assert_eq!(c.index(), i);
            assert!(!c.description().is_empty());
        }
        // Switches: General's four, then Media's two; Clipboard has the
        // capacity choices, Appearance the swatches; About nothing
        assert_eq!(Category::General.controls(), &Control::ALL[..4]);
        assert_eq!(
            Category::Media.controls(),
            [Control::ShowSourceApp, Control::ShowVisualizer]
        );
        for c in [Category::Appearance, Category::Clipboard, Category::About] {
            assert!(c.controls().is_empty(), "{c:?} has no switches");
        }
        assert_eq!(Category::Clipboard.capacities(), &ClipboardCapacity::ALL);
        for c in Category::ALL {
            assert_eq!(c.capacities().is_empty(), c != Category::Clipboard);
            assert_eq!(c.accents().is_empty(), c != Category::Appearance);
        }
        assert_eq!(
            Nav {
                category: Category::About,
                ..Nav::default()
            }
            .focus_order()
            .len(),
            Category::ALL.len()
        );
        assert!(
            Category::About
                .description()
                .contains(env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn test_navigation_selects_pages_and_keeps_focus_sane() {
        let mut nav = Nav::default();
        assert!(!nav.select(Category::General), "already showing");
        nav.focus = Target::Control(Control::ReducedMotion);
        assert!(nav.select(Category::Media));
        assert_eq!(nav.category, Category::Media);
        assert_eq!(
            nav.focus,
            Target::Category(Category::Media),
            "focus leaves the hidden control"
        );
        // Tab order: sidebar entries, then the page's controls
        assert_eq!(nav.focus_order().len(), 7, "Media: its two switches");
        nav.select(Category::About);
        assert_eq!(nav.focus_order().len(), 5, "About: sidebar only");
        nav.select(Category::General);
        assert_eq!(nav.focus_order().len(), 12, "4 switches + 3 closing delays");
        nav.focus = Target::Category(Category::About);
        assert_eq!(nav.tab(false), Target::Control(Control::AlwaysOnTop));
        assert_eq!(nav.tab(false), Target::Control(Control::ReducedMotion));
        // Visual order: the closing-delay segments sit between the sections
        for d in CollapseDelay::ALL {
            assert_eq!(nav.tab(false), Target::Delay(d));
        }
        assert_eq!(nav.tab(false), Target::Control(Control::CloseOnEscape));
        assert_eq!(
            nav.tab(false),
            Target::Control(Control::ConfirmClearClipboard)
        );
        assert_eq!(nav.tab(false), Target::Category(Category::General), "wraps");
        assert_eq!(
            nav.tab(true),
            Target::Control(Control::ConfirmClearClipboard)
        );
        assert!(nav.focus_visible);
        // Arrows move within the focused group and stop at its ends
        nav.focus = Target::Category(Category::General);
        assert_eq!(nav.arrow(true), Target::Category(Category::General));
        assert_eq!(nav.arrow(false), Target::Category(Category::Appearance));
        nav.focus = Target::Category(Category::About);
        assert_eq!(nav.arrow(false), Target::Category(Category::About));
        nav.focus = Target::Control(Control::AlwaysOnTop);
        assert_eq!(nav.arrow(false), Target::Control(Control::ReducedMotion));
        assert_eq!(nav.arrow(false), Target::Control(Control::CloseOnEscape));
        assert_eq!(
            nav.arrow(false),
            Target::Control(Control::ConfirmClearClipboard)
        );
        assert_eq!(
            nav.arrow(false),
            Target::Control(Control::ConfirmClearClipboard),
            "stops at the last switch"
        );
        assert_eq!(nav.arrow(true), Target::Control(Control::CloseOnEscape));
        // Moving focus never changes the page
        assert_eq!(nav.category, Category::General);
    }

    #[test]
    fn test_sizes_and_dpi_scaling() {
        assert!(CLIENT_SIZE.0 >= MIN_CLIENT_SIZE.0 && CLIENT_SIZE.1 >= MIN_CLIENT_SIZE.1);
        assert_eq!(to_px(CLIENT_SIZE, 96), (820, 520));
        assert_eq!(to_px(CLIENT_SIZE, 120), (1025, 650));
        assert_eq!(to_px(MIN_CLIENT_SIZE, 96), (640, 504));
        assert_eq!(to_px(MIN_CLIENT_SIZE, 120), (800, 630));
        let (w, h) = outer_size(CLIENT_SIZE, 120);
        assert!(w > 1025 && h > 650, "frame added on top of the client area");
    }

    #[test]
    fn test_layout_fits_from_minimum_to_large() {
        for (width, height) in [MIN_CLIENT_SIZE, CLIENT_SIZE, (1400.0, 900.0)] {
            let l = layout(width, height);
            // Sidebar keeps its width; entries stack inside it
            assert_eq!(l.sidebar.width(), SIDEBAR_WIDTH);
            for (i, r) in l.items.iter().enumerate() {
                assert!(r.left >= 0.0 && r.right <= SIDEBAR_WIDTH && r.height() >= 32.0);
                assert!(r.bottom <= height, "entry {i} fits");
                if i > 0 {
                    assert!(l.items[i - 1].bottom <= r.top);
                }
            }
            // Page column right of the sidebar, inside the window
            assert!(l.page_title.left > SIDEBAR_WIDTH && l.page_title.right <= width);
            assert!(l.page_description.bottom <= l.rows[0].row.top);
            for r in l.rows {
                assert!(
                    r.row.left > SIDEBAR_WIDTH && r.switch.right <= width - PAGE_PADDING + 0.01
                );
                assert!(r.title.right < r.switch.left && r.title.width() >= 150.0);
                assert!(r.row.bottom <= height, "rows fit the height");
            }
            // Rows stack in `Control::ALL` order; each section heading sits
            // right above its first row, below the previous section
            for i in 1..4 {
                assert!(l.rows[i - 1].row.bottom <= l.rows[i].row.top);
            }
            // Media's two rows take General's first section's places
            assert_eq!(l.rows[4], l.rows[0]);
            assert_eq!(l.rows[5], l.rows[1]);
            for (k, (_, controls)) in SECTIONS.iter().enumerate() {
                let first = l.rows[controls[0].index()].row;
                assert_eq!(l.sections[k].bottom, first.top);
                assert!(l.page_description.bottom <= l.sections[k].top);
                if k > 0 {
                    let prev = l.rows[SECTIONS[k - 1].1[1].index()].row;
                    assert!(prev.bottom < l.sections[k].top);
                }
            }
        }
        // Sections cover every control once, in page order
        let flat: Vec<Control> = SECTIONS.iter().flat_map(|(_, c)| *c).collect();
        assert_eq!(flat, Category::General.controls());
        // The page column grows with the window; the sidebar does not
        let (small, large) = (layout(MIN_CLIENT_SIZE.0, 400.0), layout(1400.0, 400.0));
        assert_eq!(small.items, large.items);
        assert!(large.rows[0].switch.right > small.rows[0].switch.right);
    }

    #[test]
    fn test_hit_testing_at_several_dpis() {
        let (w, h) = CLIENT_SIZE;
        let l = layout(w, h);
        for dpi in [96u32, 120, 144] {
            let s = dpi as f32 / 96.0;
            // Physical points converted back to DIP land on the same targets
            let at = |r: RectF, category| {
                let (px, py) = (
                    ((r.left + r.right) / 2.0) * s,
                    ((r.top + r.bottom) / 2.0) * s,
                );
                hit(w, h, category, px / s, py / s)
            };
            for c in Category::ALL {
                assert_eq!(
                    at(l.items[c.index()], Category::About),
                    Some(Target::Category(c))
                );
            }
            for page in [Category::General, Category::Media] {
                for c in page.controls() {
                    let row = l.rows[c.index()].row;
                    assert_eq!(at(row, page), Some(Target::Control(*c)), "{dpi}");
                    // Controls of a page that is not showing are not hit
                    assert_eq!(at(row, Category::About), None);
                }
            }
            assert_eq!(
                at(l.page_title, Category::General),
                None,
                "title is not interactive"
            );
        }
    }

    #[test]
    fn test_accessibility_elements_names_and_order() {
        for category in Category::ALL {
            let children = Element::children(category);
            let names: Vec<_> = children.iter().map(|e| e.name(category)).collect();
            assert_eq!(
                &names[..5],
                ["General", "Appearance", "Clipboard", "Media", "About"]
            );
            assert_eq!(
                names[5],
                category.title(),
                "page title follows the selection"
            );
            let switches = &children[6..];
            if category == Category::General {
                assert_eq!(
                    switches,
                    [
                        Element::Section(0),
                        Element::Control(Control::AlwaysOnTop),
                        Element::Control(Control::ReducedMotion),
                        Element::Delay(CollapseDelay::Five),
                        Element::Delay(CollapseDelay::Ten),
                        Element::Delay(CollapseDelay::Fifteen),
                        Element::Section(1),
                        Element::Control(Control::CloseOnEscape),
                        Element::Control(Control::ConfirmClearClipboard),
                    ]
                );
                assert_eq!(
                    switches
                        .iter()
                        .map(|e| e.name(category))
                        .collect::<Vec<_>>(),
                    [
                        "Notch",
                        "Always on top",
                        "Reduced motion",
                        "5 seconds",
                        "10 seconds",
                        "15 seconds",
                        "Window behavior",
                        "Close Settings window with Escape",
                        "Confirm before clearing clipboard history",
                    ]
                );
                // Accessible order matches keyboard (Tab) order
                let nav = Nav::default();
                let tab: Vec<Element> = nav.focus_order().into_iter().map(Element::from).collect();
                let focusable: Vec<Element> = children
                    .iter()
                    .copied()
                    .filter(|e| {
                        matches!(
                            e,
                            Element::Category(_) | Element::Control(_) | Element::Delay(_)
                        )
                    })
                    .collect();
                assert_eq!(tab, focusable);
            } else if category == Category::Appearance {
                // Heading, the seven swatches (radio buttons), then Preview
                let names: Vec<_> = switches.iter().map(|e| e.name(category)).collect();
                assert_eq!(
                    names,
                    [
                        "Accent color",
                        "White",
                        "Blue",
                        "Teal",
                        "Green",
                        "Amber",
                        "Rose",
                        "Violet",
                        "Preview"
                    ]
                );
                // Only the current accent reads as selected; page entries too
                let s = OecaSettings {
                    accent: AccentChoice::Rose,
                    ..OecaSettings::default()
                };
                let selected: Vec<_> = children
                    .iter()
                    .filter(|e| e.is_selected(category, s))
                    .collect();
                assert_eq!(
                    selected,
                    [
                        &Element::Category(Category::Appearance),
                        &Element::Accent(AccentChoice::Rose)
                    ]
                );
            } else if category == Category::Clipboard {
                // Heading, the three capacity radio buttons, the two notes;
                // never any clipboard contents
                let names: Vec<_> = switches.iter().map(|e| e.name(category)).collect();
                assert_eq!(
                    names,
                    [
                        "History",
                        "5 items",
                        "10 items",
                        "20 items",
                        CLIPBOARD_NOTES[0],
                        CLIPBOARD_NOTES[1]
                    ]
                );
                let s = OecaSettings {
                    clipboard_capacity: ClipboardCapacity::Ten,
                    ..OecaSettings::default()
                };
                let selected: Vec<_> = switches
                    .iter()
                    .filter(|e| e.is_selected(category, s))
                    .collect();
                assert_eq!(selected, [&Element::Capacity(ClipboardCapacity::Ten)]);
            } else if category == Category::Media {
                assert_eq!(
                    switches,
                    [
                        Element::Section(5),
                        Element::Control(Control::ShowSourceApp),
                        Element::Control(Control::ShowVisualizer)
                    ]
                );
                assert_eq!(switches[0].name(category), "Now playing");
            } else {
                assert!(switches.is_empty(), "nothing on {category:?}");
            }
            // Unique runtime ids
            let mut ids: Vec<_> = children.iter().map(|e| e.runtime_id()).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), children.len());
        }
    }

    #[test]
    fn test_controls_map_to_the_shared_settings() {
        // Defaults of every General setting
        let d = OecaSettings::default();
        assert_eq!(
            Control::ALL.map(|c| c.is_on(d)),
            [true, false, true, true, true, true],
            "Always on top on, Reduced motion off, Escape closes on, Confirm clear on,              source app shown, visualizer shown"
        );
        // Each switch flips exactly its own value, and back
        for c in Control::ALL {
            let mut s = d;
            c.toggle(&mut s);
            for other in Control::ALL {
                assert_eq!(other.is_on(s), other.is_on(d) ^ (other == c), "{c:?}");
            }
            c.toggle(&mut s);
            assert_eq!(s, d);
            assert_eq!(Control::from_index(c.index()), Some(c));
        }
        assert_eq!(Control::from_index(Control::ALL.len()), None);
        const { assert!(WM_APP_SETTINGS_TOGGLE > WM_APP + 16) };
    }

    #[test]
    fn test_window_lifecycle_navigation_and_sync() {
        // Hidden creation: lifecycle only (open() would also show it)
        assert!(!is_open());
        let hwnd = create(HWND::default(), OecaSettings::default()).unwrap();
        assert!(is_open() && window_handle() == Some(hwnd));
        assert_eq!(with_view(|v| v.nav.category), Some(Category::General));
        // A second request reuses the same window (no duplicate)
        let again = window_handle()
            .unwrap_or_else(|| create(HWND::default(), OecaSettings::default()).unwrap());
        assert_eq!(again, hwnd);
        // Navigating changes only the page, never the (notch-owned) settings
        let before = with_view(|v| v.settings);
        for c in Category::ALL {
            navigate(hwnd, c);
            assert_eq!(with_view(|v| v.nav.category), Some(c));
        }
        assert_eq!(with_view(|v| v.settings), before);
        // Changes from the notch show up here on any page; back on General the
        // switches read them
        let changed = OecaSettings {
            always_on_top: false,
            reduced_motion: true,
            close_settings_on_escape: false,
            confirm_clear_clipboard: false,
            accent: AccentChoice::Teal,
            clipboard_capacity: ClipboardCapacity::Five,
            show_source_app: false,
            show_visualizer: false,
            collapse_delay: CollapseDelay::Fifteen,
        };
        sync(changed);
        navigate(hwnd, Category::General);
        assert_eq!(with_view(|v| v.settings), Some(changed));
        // Closing destroys only this window: no quit request for the app
        close();
        assert!(!is_open());
        let mut msg = MSG::default();
        let quit = unsafe { PeekMessageW(&mut msg, None, WM_QUIT, WM_QUIT, PM_REMOVE) };
        assert!(!quit.as_bool(), "closing Settings never quits Oeca");
        // Reopening: a fresh window, back on General
        let reopened = create(HWND::default(), changed).unwrap();
        assert_eq!(with_view(|v| v.nav.category), Some(Category::General));
        assert_eq!(with_view(|v| v.settings), Some(changed));
        unsafe {
            let _ = DestroyWindow(reopened);
        }
        assert!(!is_open());
    }

    #[test]
    fn test_texts_fit_their_rows_at_the_minimum_width() {
        let dwrite: IDWriteFactory =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }.unwrap();
        let width_of = |s: &str, role: TextRole| {
            let f = role_format(&dwrite, role).unwrap();
            let s: Vec<u16> = s.encode_utf16().collect();
            let layout = unsafe { dwrite.CreateTextLayout(&s, &f, 10_000.0, 100.0) }.unwrap();
            let mut m = Default::default();
            unsafe { layout.GetMetrics(&mut m) }.unwrap();
            m.widthIncludingTrailingWhitespace
        };
        let l = layout(MIN_CLIENT_SIZE.0, MIN_CLIENT_SIZE.1);
        assert!(width_of(CAPACITY_TITLE, TextRole::SettingLabel) <= l.capacity_title.width());
        assert!(width_of(CLOSING_TITLE, TextRole::SettingLabel) <= l.closing_title.width());
        assert!(
            width_of(CLOSING_DESCRIPTION, TextRole::SettingDescription)
                <= l.closing_description.width()
        );
        assert!(
            width_of(CAPACITY_DESCRIPTION, TextRole::SettingDescription)
                <= l.capacity_description.width()
        );
        for (note, r) in CLIPBOARD_NOTES.iter().zip(l.clipboard_notes) {
            assert!(
                width_of(note, TextRole::SettingDescription) <= r.width(),
                "{note}"
            );
        }
        for c in Control::ALL {
            let row = l.rows[c.index()];
            assert!(
                width_of(c.title(), TextRole::SettingLabel) <= row.title.width(),
                "{c:?} title"
            );
            assert!(
                width_of(c.description(), TextRole::SettingDescription) <= row.description.width(),
                "{c:?} description"
            );
        }
    }

    /// Sends a key to the window as the keyboard would.
    fn key(hwnd: HWND, vk: u16) {
        use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
        unsafe {
            SendMessageW(hwnd, WM_KEYDOWN, Some(WPARAM(vk as usize)), Some(LPARAM(0)));
        }
    }

    /// The toggle requests posted to the (null, so thread-queued) notch.
    fn toggle_requests() -> Vec<usize> {
        use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW};
        let mut out = Vec::new();
        let mut msg = MSG::default();
        while unsafe {
            PeekMessageW(
                &mut msg,
                None,
                WM_APP_SETTINGS_TOGGLE,
                WM_APP_SETTINGS_TOGGLE,
                PM_REMOVE,
            )
        }
        .as_bool()
        {
            out.push(msg.wParam.0);
        }
        out
    }

    #[test]
    fn test_escape_follows_the_setting_and_keyboard_still_works() {
        // On (default): Escape closes the window
        let _ = create(HWND::default(), OecaSettings::default()).unwrap();
        key(window_handle().unwrap(), VK_ESCAPE.0);
        assert!(!is_open(), "Escape closes Settings");
        // Off: Escape is ignored
        let off = OecaSettings {
            close_settings_on_escape: false,
            ..OecaSettings::default()
        };
        let hwnd = create(HWND::default(), off).unwrap();
        toggle_requests();
        key(hwnd, VK_ESCAPE.0);
        assert!(is_open(), "Escape does nothing when off");
        // ...and Tab, arrows, Enter and Space all still work
        with_view(|v| v.nav.focus = Target::Category(Category::About));
        key(hwnd, VK_TAB.0);
        assert_eq!(
            with_view(|v| v.nav.focus),
            Some(Target::Control(Control::AlwaysOnTop))
        );
        key(hwnd, VK_DOWN.0);
        key(hwnd, VK_DOWN.0);
        assert_eq!(
            with_view(|v| v.nav.focus),
            Some(Target::Control(Control::CloseOnEscape))
        );
        key(hwnd, VK_SPACE.0);
        key(hwnd, VK_RETURN.0);
        assert_eq!(
            toggle_requests(),
            [Control::CloseOnEscape.index(); 2],
            "Space and Enter ask the notch to flip the focused switch"
        );
        key(hwnd, VK_UP.0);
        assert_eq!(
            with_view(|v| v.nav.focus),
            Some(Target::Control(Control::ReducedMotion))
        );
        with_view(|v| v.nav.focus = Target::Category(Category::Media));
        key(hwnd, VK_RETURN.0);
        assert_eq!(with_view(|v| v.nav.category), Some(Category::Media));
        assert!(is_open());
        // The notch flips the value and syncs back: Escape now closes, the
        // switch reads On, on any page
        sync(OecaSettings::default());
        assert_eq!(
            with_view(|v| Control::CloseOnEscape.is_on(v.settings)),
            Some(true)
        );
        key(hwnd, VK_ESCAPE.0);
        assert!(!is_open());
        // The title-bar X always closes, whatever the setting
        let hwnd = create(HWND::default(), off).unwrap();
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                hwnd,
                windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                None,
                None,
            );
        }
        assert!(!is_open(), "WM_CLOSE (the X) closes with Escape off");
    }

    #[test]
    fn test_appearance_layout_fits_and_hits_at_all_dpis() {
        for (w, h) in [MIN_CLIENT_SIZE, CLIENT_SIZE] {
            let l = layout(w, h);
            let [accent_heading, preview_heading] = l.appearance_sections;
            assert!(l.page_description.bottom <= accent_heading.top);
            for (i, r) in l.swatches.iter().enumerate() {
                // Reliable targets: 36 DIP squares in one row, in the page
                // column, between the heading and the selected name
                assert_eq!((r.width(), r.height()), (36.0, 36.0));
                assert!(r.left > SIDEBAR_WIDTH && r.right <= w - PAGE_PADDING, "{i}");
                assert!(accent_heading.bottom <= r.top && r.bottom <= l.accent_name.top);
                if i > 0 {
                    assert!(l.swatches[i - 1].right < r.left);
                }
            }
            assert!(l.accent_name.bottom <= preview_heading.top);
            assert!(preview_heading.bottom <= l.preview.top);
            assert!(l.preview.bottom <= h && l.preview.right <= w - PAGE_PADDING);
        }
        // Physical points at each DPI map back onto the same swatch; only on
        // the Appearance page
        let (w, h) = CLIENT_SIZE;
        let l = layout(w, h);
        for dpi in [96u32, 120, 144, 192] {
            let s = dpi as f32 / 96.0;
            for a in AccentChoice::ALL {
                let r = l.swatches[a.index()];
                for (x, y) in [
                    ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0),
                    (r.left + 1.0, r.top + 1.0),
                    (r.right - 1.0, r.bottom - 1.0),
                ] {
                    let (px, py) = ((x * s).round(), (y * s).round());
                    assert_eq!(
                        hit(w, h, Category::Appearance, px / s, py / s),
                        Some(Target::Accent(a)),
                        "{a:?} at {dpi}"
                    );
                    assert!(!matches!(
                        hit(w, h, Category::General, px / s, py / s),
                        Some(Target::Accent(_))
                    ));
                }
            }
            // Between two swatches: nothing
            let gap = (l.swatches[0].right + l.swatches[1].left) / 2.0;
            assert_eq!(
                hit(w, h, Category::Appearance, gap, l.swatches[0].top + 18.0),
                None
            );
        }
    }

    /// The accent requests posted to the (null, so thread-queued) notch.
    fn accent_requests() -> Vec<usize> {
        use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW};
        let mut out = Vec::new();
        let mut msg = MSG::default();
        while unsafe {
            PeekMessageW(
                &mut msg,
                None,
                WM_APP_SETTINGS_ACCENT,
                WM_APP_SETTINGS_ACCENT,
                PM_REMOVE,
            )
        }
        .as_bool()
        {
            out.push(msg.wParam.0);
        }
        out
    }

    #[test]
    fn test_accent_keyboard_selection_sync_and_retention() {
        let hwnd = create(HWND::default(), OecaSettings::default()).unwrap();
        accent_requests();
        navigate(hwnd, Category::Appearance);
        // Tab from the last sidebar entry reaches the swatches, in order
        with_view(|v| v.nav.focus = Target::Category(Category::About));
        key(hwnd, VK_TAB.0);
        assert_eq!(
            with_view(|v| v.nav.focus),
            Some(Target::Accent(AccentChoice::White))
        );
        key(hwnd, VK_LEFT.0);
        assert_eq!(
            with_view(|v| v.nav.focus),
            Some(Target::Accent(AccentChoice::White)),
            "stops at the first swatch"
        );
        key(hwnd, VK_RIGHT.0);
        key(hwnd, VK_RIGHT.0);
        key(hwnd, VK_LEFT.0);
        assert_eq!(
            with_view(|v| v.nav.focus),
            Some(Target::Accent(AccentChoice::Blue))
        );
        // Moving focus only moves focus; Space / Enter ask the notch
        assert!(accent_requests().is_empty());
        key(hwnd, VK_SPACE.0);
        assert_eq!(accent_requests(), [AccentChoice::Blue.index()]);
        // The notch applies it and syncs back: selected here at once
        let owned = OecaSettings {
            accent: AccentChoice::Blue,
            ..OecaSettings::default()
        };
        sync(owned);
        assert_eq!(with_view(|v| v.settings.accent), Some(AccentChoice::Blue));
        assert!(Element::Accent(AccentChoice::Blue).is_selected(Category::Appearance, owned));
        // Left / Right on the sidebar do nothing
        with_view(|v| v.nav.focus = Target::Category(Category::Appearance));
        key(hwnd, VK_RIGHT.0);
        assert_eq!(
            with_view(|v| v.nav.focus),
            Some(Target::Category(Category::Appearance))
        );
        // Other pages keep it; other settings are untouched
        for c in Category::ALL {
            navigate(hwnd, c);
        }
        navigate(hwnd, Category::Appearance);
        assert_eq!(with_view(|v| v.settings), Some(owned));
        // A swatch hidden with its page drops focus back to the sidebar
        with_view(|v| v.nav.focus = Target::Accent(AccentChoice::Violet));
        navigate(hwnd, Category::General);
        assert_eq!(
            with_view(|v| v.nav.focus),
            Some(Target::Category(Category::General))
        );
        // Closing and reopening (the notch passes its settings) keeps it
        close();
        let reopened = create(HWND::default(), owned).unwrap();
        assert_eq!(with_view(|v| v.settings.accent), Some(AccentChoice::Blue));
        unsafe {
            let _ = DestroyWindow(reopened);
        }
    }

    #[test]
    fn test_clipboard_page_layout_fits_and_hits_at_all_dpis() {
        for (w, h) in [MIN_CLIENT_SIZE, CLIENT_SIZE] {
            let l = layout(w, h);
            let row = l.rows[0].row;
            // Segments: equal, adjacent, inside the row's right end, clear of
            // the title text
            for (i, r) in l.capacity.iter().enumerate() {
                assert_eq!((r.width(), r.height()), (52.0, 28.0));
                assert!(r.top > row.top && r.bottom < row.bottom);
                assert!(r.left > l.capacity_title.right && r.right <= w - PAGE_PADDING);
                if i > 0 {
                    assert_eq!(l.capacity[i - 1].right, r.left);
                }
            }
            assert!(l.sections[0].bottom <= row.top);
            assert!(row.bottom <= l.clipboard_notes[0].top);
            assert!(l.clipboard_notes[0].bottom <= l.clipboard_notes[1].top);
            assert!(l.clipboard_notes[1].bottom <= h);
        }
        let (w, h) = CLIENT_SIZE;
        let l = layout(w, h);
        for dpi in [96u32, 120, 144, 192] {
            let s = dpi as f32 / 96.0;
            for c in ClipboardCapacity::ALL {
                let r = l.capacity[c.index()];
                for (x, y) in [
                    ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0),
                    (r.left + 1.0, r.top + 1.0),
                    (r.right - 1.0, r.bottom - 1.0),
                ] {
                    let (px, py) = ((x * s).round(), (y * s).round());
                    assert_eq!(
                        hit(w, h, Category::Clipboard, px / s, py / s),
                        Some(Target::Capacity(c)),
                        "{c:?} at {dpi}"
                    );
                    // Not on other pages
                    assert!(!matches!(
                        hit(w, h, Category::General, px / s, py / s),
                        Some(Target::Capacity(_))
                    ));
                }
            }
            // The notes and the title are not interactive
            for r in [l.capacity_title, l.clipboard_notes[0]] {
                let (x, y) = ((r.left + 20.0), (r.top + r.bottom) / 2.0);
                assert_eq!(hit(w, h, Category::Clipboard, x, y), None);
            }
        }
    }

    /// The capacity requests posted to the (null, so thread-queued) notch.
    fn capacity_requests() -> Vec<usize> {
        use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW};
        let mut out = Vec::new();
        let mut msg = MSG::default();
        while unsafe {
            PeekMessageW(
                &mut msg,
                None,
                WM_APP_SETTINGS_CAPACITY,
                WM_APP_SETTINGS_CAPACITY,
                PM_REMOVE,
            )
        }
        .as_bool()
        {
            out.push(msg.wParam.0);
        }
        out
    }

    #[test]
    fn test_capacity_and_media_keyboard_sync_and_retention() {
        let hwnd = create(HWND::default(), OecaSettings::default()).unwrap();
        capacity_requests();
        toggle_requests();
        // Clipboard: Tab reaches the segments (20 selected by default)
        navigate(hwnd, Category::Clipboard);
        assert!(
            Element::Capacity(ClipboardCapacity::Twenty)
                .is_selected(Category::Clipboard, OecaSettings::default())
        );
        with_view(|v| v.nav.focus = Target::Category(Category::About));
        key(hwnd, VK_TAB.0);
        assert_eq!(
            with_view(|v| v.nav.focus),
            Some(Target::Capacity(ClipboardCapacity::Five))
        );
        key(hwnd, VK_RIGHT.0);
        key(hwnd, VK_RIGHT.0);
        key(hwnd, VK_RIGHT.0);
        assert_eq!(
            with_view(|v| v.nav.focus),
            Some(Target::Capacity(ClipboardCapacity::Twenty)),
            "stops at the last segment"
        );
        key(hwnd, VK_LEFT.0);
        assert!(
            capacity_requests().is_empty(),
            "moving focus changes nothing"
        );
        key(hwnd, VK_RETURN.0);
        assert_eq!(capacity_requests(), [ClipboardCapacity::Ten.index()]);
        // The notch applies it and syncs back
        let owned = OecaSettings {
            clipboard_capacity: ClipboardCapacity::Ten,
            ..OecaSettings::default()
        };
        sync(owned);
        assert_eq!(
            with_view(|v| v.settings.clipboard_capacity),
            Some(ClipboardCapacity::Ten)
        );
        // Media: Tab from the sidebar reaches its two switches in visual order
        navigate(hwnd, Category::Media);
        with_view(|v| v.nav.focus = Target::Category(Category::About));
        key(hwnd, VK_TAB.0);
        assert_eq!(
            with_view(|v| v.nav.focus),
            Some(Target::Control(Control::ShowSourceApp))
        );
        key(hwnd, VK_DOWN.0);
        assert_eq!(
            with_view(|v| v.nav.focus),
            Some(Target::Control(Control::ShowVisualizer))
        );
        key(hwnd, VK_SPACE.0);
        assert_eq!(toggle_requests(), [Control::ShowVisualizer.index()]);
        let owned = OecaSettings {
            show_visualizer: false,
            ..owned
        };
        sync(owned);
        // Other pages keep everything; reopening shows the notch's values
        for c in Category::ALL {
            navigate(hwnd, c);
        }
        assert_eq!(with_view(|v| v.settings), Some(owned));
        close();
        let reopened = create(HWND::default(), owned).unwrap();
        assert_eq!(with_view(|v| v.settings), Some(owned));
        assert!(!Control::ShowVisualizer.is_on(owned) && Control::ShowSourceApp.is_on(owned));
        unsafe {
            let _ = DestroyWindow(reopened);
        }
    }

    #[test]
    fn test_initial_position_clears_the_notch_inside_the_work_area() {
        let rect = |left, top, right, bottom| RECT {
            left,
            top,
            right,
            bottom,
        };
        // 1920 x 1080 at 125% with a 60 px bottom taskbar
        let work = rect(0, 0, 1920, 1020);
        let size = outer_size(CLIENT_SIZE, 120);
        let clear = notch_clear_top(0, 120);
        let (x, y) = initial_position(work, size, clear);
        assert_eq!(x, (1920 - size.0) / 2, "centred");
        assert_eq!(y, clear, "below the expanded notch");
        assert!(y + size.1 <= work.bottom);
        // The band covers the tallest expanded notch at that DPI
        for space in crate::space::OecaSpace::ALL {
            let d =
                crate::layout::space_dimensions(crate::config::NotchState::Expanded, 120, space);
            assert!(clear > d.height, "{space:?}");
        }
        // Short work area: moved up just enough, never above its top
        let short = rect(0, 0, 1366, 720);
        let (_, y) = initial_position(short, size, clear);
        assert_eq!(y, 720 - size.1);
        let tiny = rect(0, 0, 800, 500);
        let (x, y) = initial_position(tiny, size, clear);
        assert_eq!((x, y), (0, 0), "larger than the work area: its top-left");
        // Another monitor (left of the primary, top taskbar): its own area
        let left_monitor = rect(-2560, 40, 0, 1440);
        let clear = notch_clear_top(0, 96);
        let size = outer_size(CLIENT_SIZE, 96);
        let (x, y) = initial_position(left_monitor, size, clear);
        assert_eq!(x, -2560 + (2560 - size.0) / 2);
        assert!(y >= 40 && y == clear.max(40));
        // Exactly the 16 DIP gap below the tallest expanded notch, at each DPI
        for dpi in [96u32, 120, 144, 192] {
            let tallest = crate::space::OecaSpace::ALL
                .into_iter()
                .map(|s| {
                    crate::layout::space_dimensions(crate::config::NotchState::Expanded, dpi, s)
                        .height
                })
                .max()
                .unwrap();
            let gap = (16.0 * dpi as f32 / 96.0).round() as i32;
            assert_eq!(notch_clear_top(0, dpi), tallest + gap, "{dpi}");
        }
        // Live inputs from this machine: 1920 x 1080 work area at 120 DPI
        let live = rect(0, 0, 1920, 1080);
        assert_eq!(
            initial_position(live, outer_size(CLIENT_SIZE, 120), notch_clear_top(0, 120)),
            ((1920 - outer_size(CLIENT_SIZE, 120).0) / 2, 281)
        );
        // DPI scales the band: never physical-pixel constants
        assert!(notch_clear_top(0, 192) > notch_clear_top(0, 96));
        assert_eq!(notch_clear_top(100, 96), notch_clear_top(0, 96) + 100);
    }

    #[test]
    fn test_elements_of_a_hidden_page_or_closed_window_are_unavailable() {
        use windows::Win32::UI::Accessibility::{
            IRawElementProviderFragment, ISelectionItemProvider, IToggleProvider,
        };
        use windows::core::Interface;
        let hwnd = create(HWND::default(), OecaSettings::default()).unwrap();
        toggle_requests();
        let switch = access::provider(hwnd, Element::Control(Control::AlwaysOnTop));
        let toggle: IToggleProvider = switch.cast().unwrap();
        let frag: IRawElementProviderFragment = switch.cast().unwrap();
        // On its page: bounds and Toggle work
        assert!(unsafe { frag.BoundingRectangle() }.unwrap().width > 0.0);
        unsafe { toggle.Toggle() }.unwrap();
        assert_eq!(toggle_requests(), [Control::AlwaysOnTop.index()]);
        // Media now shows a different switch in the same rect: the stale one
        // reports no bounds and refuses to act
        navigate(hwnd, Category::Media);
        assert!(!access::is_live(
            hwnd,
            Element::Control(Control::AlwaysOnTop)
        ));
        assert_eq!(unsafe { frag.BoundingRectangle() }.unwrap().width, 0.0);
        assert!(unsafe { toggle.Toggle() }.is_err());
        assert!(toggle_requests().is_empty(), "no hidden setting changed");
        // Media's own switch is live; categories always are
        assert!(access::is_live(
            hwnd,
            Element::Control(Control::ShowSourceApp)
        ));
        assert!(access::is_live(hwnd, Element::Category(Category::General)));
        // A swatch held from Appearance can't change the accent elsewhere
        navigate(hwnd, Category::Appearance);
        let swatch: ISelectionItemProvider =
            access::provider(hwnd, Element::Accent(AccentChoice::Teal))
                .cast()
                .unwrap();
        navigate(hwnd, Category::About);
        assert!(unsafe { swatch.Select() }.is_err());
        assert!(accent_requests().is_empty());
        // Back on its page it works again
        navigate(hwnd, Category::General);
        unsafe { toggle.Toggle() }.unwrap();
        assert_eq!(toggle_requests(), [Control::AlwaysOnTop.index()]);
        // After the window closes (and a new one opens) the old element is dead
        close();
        let reopened = create(HWND::default(), OecaSettings::default()).unwrap();
        assert!(!access::is_live(
            hwnd,
            Element::Control(Control::AlwaysOnTop)
        ));
        assert!(unsafe { toggle.Toggle() }.is_err());
        assert!(toggle_requests().is_empty());
        unsafe {
            let _ = DestroyWindow(reopened);
        }
    }

    #[test]
    fn test_placement_correction_rules() {
        let placed = Some((438, 281));
        // An external move away from the placement is undone
        assert_eq!(
            placement_correction(placed, 0, (438, 191), true),
            Some((438, 281))
        );
        // ...but never Oeca's own moves, a move to the same spot, or once the
        // user has taken over (placement cleared), or past the cap
        assert_eq!(placement_correction(placed, 0, (438, 191), false), None);
        assert_eq!(placement_correction(placed, 0, (438, 281), true), None);
        assert_eq!(placement_correction(None, 0, (438, 191), true), None);
        assert_eq!(
            placement_correction(placed, MAX_PLACEMENT_CORRECTIONS, (438, 191), true),
            None
        );
    }

    /// Pumps this thread's messages until `done` (cross-thread SetWindowPos
    /// calls are delivered while pumping).
    fn pump_until(done: &std::sync::atomic::AtomicBool) {
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
        };
        let start = std::time::Instant::now();
        while !done.load(std::sync::atomic::Ordering::SeqCst) {
            let mut m = MSG::default();
            while unsafe { PeekMessageW(&mut m, None, 0, 0, PM_REMOVE) }.as_bool() {
                unsafe {
                    let _ = TranslateMessage(&m);
                    DispatchMessageW(&m);
                }
            }
            assert!(start.elapsed().as_secs() < 5, "cross-thread move timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Moves the window from another thread, as a repositioning shell does.
    fn external_move(hwnd: HWND, x: i32, y: i32) {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let raw = hwnd.0 as isize;
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let mover = std::thread::spawn(move || {
            unsafe {
                let _ = SetWindowPos(
                    HWND(raw as *mut _),
                    None,
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            flag.store(true, Ordering::SeqCst);
        });
        pump_until(&done);
        mover.join().unwrap();
    }

    fn top_left(hwnd: HWND) -> (i32, i32) {
        let mut r = RECT::default();
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut r);
        }
        (r.left, r.top)
    }

    #[test]
    fn test_external_recentre_is_undone_until_the_user_moves_it() {
        use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
        let hwnd = create(HWND::default(), OecaSettings::default()).unwrap();
        let placed = top_left(hwnd);
        assert_eq!(with_view(|v| v.placement), Some(Some(placed)));
        // Another thread re-centres it: Oeca puts it back
        external_move(hwnd, placed.0, placed.1 - 90);
        assert_eq!(top_left(hwnd), placed, "title bar stays clear of the notch");
        // Oeca's own moves are never fought
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                placed.0 + 5,
                placed.1,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        assert_eq!(top_left(hwnd), (placed.0 + 5, placed.1));
        // The user starts moving it: from then on its position is theirs
        unsafe {
            SendMessageW(hwnd, WM_ENTERSIZEMOVE, None, None);
        }
        assert_eq!(with_view(|v| v.placement), Some(None));
        external_move(hwnd, 100, 120);
        assert_eq!(
            top_left(hwnd),
            (100, 120),
            "a user-placed window is left alone"
        );
        // Reopening an open window does not move it either
        open(HWND::default(), OecaSettings::default()).unwrap();
        assert_eq!(top_left(hwnd), (100, 120));
        close();
    }

    #[test]
    fn test_closing_delay_row_layout_hits_and_selection() {
        assert_eq!(OecaSettings::default().collapse_delay, CollapseDelay::Five);
        for (w, h) in [MIN_CLIENT_SIZE, CLIENT_SIZE] {
            let l = layout(w, h);
            // Third row of "Notch", between the switches and "Window behavior"
            assert_eq!(
                l.closing_row.top,
                l.rows[Control::ReducedMotion.index()].row.bottom
            );
            assert!(l.closing_row.bottom <= l.sections[1].top);
            assert_eq!(
                l.sections[1].bottom,
                l.rows[Control::CloseOnEscape.index()].row.top
            );
            assert!(
                l.rows[Control::ConfirmClearClipboard.index()].row.bottom <= h,
                "fits at {w}x{h}"
            );
            for (i, r) in l.closing.iter().enumerate() {
                assert!(r.top > l.closing_row.top && r.bottom < l.closing_row.bottom);
                assert!(r.left > l.closing_title.right && r.right <= w - PAGE_PADDING);
                if i > 0 {
                    assert_eq!(l.closing[i - 1].right, r.left);
                }
            }
            // Media keeps its rows (General's first section's places)
            assert_eq!(l.rows[4], l.rows[0]);
        }
        let (w, h) = CLIENT_SIZE;
        let l = layout(w, h);
        for dpi in [96u32, 120, 144, 192] {
            let s = dpi as f32 / 96.0;
            for d in CollapseDelay::ALL {
                let r = l.closing[d.index()];
                let (x, y) = (
                    ((r.left + r.right) / 2.0 * s).round() / s,
                    ((r.top + r.bottom) / 2.0 * s).round() / s,
                );
                assert_eq!(
                    hit(w, h, Category::General, x, y),
                    Some(Target::Delay(d)),
                    "{dpi}"
                );
                assert_eq!(hit(w, h, Category::Media, x, y), None, "General only");
            }
        }
        // UIA: radio buttons named by duration; only the current one selected
        let s = OecaSettings {
            collapse_delay: CollapseDelay::Ten,
            ..OecaSettings::default()
        };
        let selected: Vec<_> = Element::children(Category::General)
            .into_iter()
            .filter(|e| matches!(e, Element::Delay(_)) && e.is_selected(Category::General, s))
            .collect();
        assert_eq!(selected, [Element::Delay(CollapseDelay::Ten)]);
        assert_eq!(
            Element::Delay(CollapseDelay::Fifteen).name(Category::General),
            "15 seconds"
        );
    }

    /// The delay requests posted to the (null, so thread-queued) notch.
    fn delay_requests() -> Vec<usize> {
        use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW};
        let mut out = Vec::new();
        let mut msg = MSG::default();
        while unsafe {
            PeekMessageW(
                &mut msg,
                None,
                WM_APP_SETTINGS_DELAY,
                WM_APP_SETTINGS_DELAY,
                PM_REMOVE,
            )
        }
        .as_bool()
        {
            out.push(msg.wParam.0);
        }
        out
    }

    #[test]
    fn test_closing_delay_keyboard_request_and_sync() {
        let hwnd = create(HWND::default(), OecaSettings::default()).unwrap();
        delay_requests();
        with_view(|v| v.nav.focus = Target::Control(Control::ReducedMotion));
        key(hwnd, VK_TAB.0);
        assert_eq!(
            with_view(|v| v.nav.focus),
            Some(Target::Delay(CollapseDelay::Five))
        );
        key(hwnd, VK_RIGHT.0);
        key(hwnd, VK_RIGHT.0);
        key(hwnd, VK_RIGHT.0);
        assert_eq!(
            with_view(|v| v.nav.focus),
            Some(Target::Delay(CollapseDelay::Fifteen)),
            "stops at the end"
        );
        assert!(delay_requests().is_empty(), "moving focus changes nothing");
        key(hwnd, VK_SPACE.0);
        assert_eq!(delay_requests(), [CollapseDelay::Fifteen.index()]);
        // The notch applies it and syncs back; it survives page changes
        let owned = OecaSettings {
            collapse_delay: CollapseDelay::Fifteen,
            ..OecaSettings::default()
        };
        sync(owned);
        for c in Category::ALL {
            navigate(hwnd, c);
        }
        assert_eq!(
            with_view(|v| v.settings.collapse_delay),
            Some(CollapseDelay::Fifteen)
        );
        close();
    }

    #[test]
    fn test_window_switches_slide_and_stop_and_respect_reduced_motion() {
        let hwnd = create(HWND::default(), OecaSettings::default()).unwrap();
        let level = |c: Control| with_view(|v| v.switches[c.index()].level()).unwrap();
        let ticking = || with_view(|v| v.switch_tick.is_some()).unwrap();
        assert_eq!(level(Control::AlwaysOnTop), 1.0, "starts settled on");
        assert!(!ticking(), "no timer while settled");
        // Turned off (motion allowed): it slides on a frame timer
        let mut s = OecaSettings {
            always_on_top: false,
            ..OecaSettings::default()
        };
        sync(s);
        let motion = crate::window::is_client_animation_enabled();
        if motion {
            assert!(ticking(), "frames only while sliding");
            assert_eq!(level(Control::AlwaysOnTop), 1.0, "starts where it was");
            with_view(|v| v.switches[0].step(60.0));
            let mid = level(Control::AlwaysOnTop);
            assert!(mid > 0.0 && mid < 1.0);
            with_view(|v| v.switches[0].step(1000.0));
        }
        assert_eq!(level(Control::AlwaysOnTop), 0.0);
        // Under Reduced motion a flip is immediate
        s.reduced_motion = true;
        sync(s);
        assert_eq!(level(Control::ReducedMotion), 1.0, "Reduced motion snaps");
        s.close_settings_on_escape = false;
        sync(s);
        assert_eq!(
            level(Control::CloseOnEscape),
            0.0,
            "snapped while Reduced motion is on"
        );
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}
