//! Notification-area (system tray) icon with a right-click menu: Open Oeca,
//! Open Settings, Quit Oeca.
//!
//! The icon belongs to the notch window and reports through it on the UI
//! thread (`WM_APP_TRAY`, `NOTIFYICON_VERSION_4`); the menu's choice arrives
//! as an ordinary `WM_COMMAND`. Explorer restarting drops every tray icon and
//! broadcasts `TaskbarCreated`; the notch then adds it again. A failure to
//! add (e.g. no taskbar yet) only means no icon: the notch is unaffected, and
//! the next `TaskbarCreated` retries.

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION, NIN_SELECT,
    NINF_KEY, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyIcon, DestroyMenu, GetSystemMetrics, HICON, IMAGE_ICON,
    LR_DEFAULTCOLOR, LoadImageW, MF_SEPARATOR, MF_STRING, PostMessageW, RegisterWindowMessageW,
    SM_CXSMICON, SM_CYSMICON, SM_MENUDROPALIGNMENT, SetForegroundWindow, SetMenuDefaultItem,
    TPM_LEFTALIGN, TPM_RIGHTALIGN, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_APP, WM_CONTEXTMENU,
    WM_NULL,
};
use windows::core::{PCWSTR, w};

/// Enter / Space on the focused icon (shellapi.h: NIN_SELECT | NINF_KEY).
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

/// Tray notifications to the notch window.
pub const WM_APP_TRAY: u32 = WM_APP + 50;
/// The notch window's one tray icon.
const TRAY_ID: u32 = 1;
/// Icon resource embedded by `build.rs` (`assets/windows/oeca.rc`).
const ICON_RESOURCE: u16 = 1;

/// A menu choice (`WM_COMMAND` identifiers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Open = 0x100,
    Settings = 0x101,
    Quit = 0x102,
}

impl Command {
    pub fn from_id(id: usize) -> Option<Self> {
        [Self::Open, Self::Settings, Self::Quit]
            .into_iter()
            .find(|c| *c as usize == id)
    }
}

/// The menu, top to bottom (None = separator). "Open Oeca" is the default
/// (bold) item, also what a left click or Enter on the icon does.
const MENU: [Option<(Command, PCWSTR)>; 4] = [
    Some((Command::Open, w!("&Open Oeca"))),
    Some((Command::Settings, w!("Open &Settings"))),
    None,
    Some((Command::Quit, w!("&Quit Oeca"))),
];

/// What a tray notification asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Right click, or the menu key / Shift+F10 on the focused icon: the
    /// menu at this screen point.
    Menu { x: i32, y: i32 },
    /// Left click, or Enter / Space on the focused icon.
    Activate,
}

/// Decodes a `NOTIFYICON_VERSION_4` notification: the event in LOWORD(lParam),
/// the icon in HIWORD(lParam), the anchor point in wParam (signed x, y).
pub fn decode(wparam: WPARAM, lparam: LPARAM) -> Option<Event> {
    if (lparam.0 as u32 >> 16) & 0xFFFF != TRAY_ID {
        return None;
    }
    match lparam.0 as u32 & 0xFFFF {
        WM_CONTEXTMENU => Some(Event::Menu {
            x: (wparam.0 & 0xFFFF) as u16 as i16 as i32,
            y: ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32,
        }),
        NIN_SELECT | NIN_KEYSELECT => Some(Event::Activate),
        _ => None,
    }
}

/// `TaskbarCreated`, registered once per process.
pub fn taskbar_created() -> u32 {
    static ID: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *ID.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}

/// The tray icon: the window it is shown for (None: not shown), and its
/// icon handle (owned here). Dropping it removes the icon, so every way the
/// notch's state is freed also cleans up the tray.
#[derive(Debug, Default)]
pub struct TrayIcon {
    icon: Option<HICON>,
    shown: Option<HWND>,
}

impl TrayIcon {
    /// Adds the icon (no-op if already shown); false if the shell refused.
    pub fn add(&mut self, hwnd: HWND) -> bool {
        if self.shown.is_some() {
            return true;
        }
        if self.icon.is_none() {
            self.icon = load_icon(hwnd);
        }
        let mut data = base_data(hwnd);
        data.uFlags = NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = WM_APP_TRAY;
        if let Some(icon) = self.icon {
            data.uFlags |= NIF_ICON;
            data.hIcon = icon;
        }
        for (dst, src) in data.szTip.iter_mut().zip("Oeca".encode_utf16()) {
            *dst = src;
        }
        unsafe {
            if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
                return false;
            }
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
        }
        self.shown = Some(hwnd);
        true
    }

    /// Explorer restarted: its icons are gone, so add ours again.
    pub fn restore(&mut self, hwnd: HWND) -> bool {
        self.shown = None;
        self.add(hwnd)
    }

    /// Removes the icon (no-op if not shown). Safe to call more than once.
    pub fn remove(&mut self) {
        if let Some(hwnd) = self.shown.take() {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &base_data(hwnd));
            }
        }
    }

    #[cfg(test)]
    pub fn is_shown(&self) -> bool {
        self.shown.is_some()
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        self.remove();
        if let Some(icon) = self.icon.take() {
            unsafe {
                let _ = DestroyIcon(icon);
            }
        }
    }
}

fn base_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    }
}

/// Oeca's embedded icon at the small-icon size for the window's DPI (the
/// shell picks the best of the resource's images). None: no icon resource.
fn load_icon(hwnd: HWND) -> Option<HICON> {
    unsafe {
        let module = GetModuleHandleW(None).ok()?;
        let dpi = GetDpiForWindow(hwnd);
        let (cx, cy) = if dpi > 0 {
            (
                GetSystemMetricsForDpi(SM_CXSMICON, dpi),
                GetSystemMetricsForDpi(SM_CYSMICON, dpi),
            )
        } else {
            (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON))
        };
        let handle = LoadImageW(
            Some(module.into()),
            PCWSTR(ICON_RESOURCE as usize as *const u16),
            IMAGE_ICON,
            cx,
            cy,
            LR_DEFAULTCOLOR,
        )
        .ok()?;
        Some(HICON(handle.0))
    }
}

/// Shows the menu at (x, y). The choice is posted to `hwnd` as `WM_COMMAND`
/// once the menu closes; dismissing it posts nothing. Must not run while the
/// window state is borrowed: the menu's modal loop keeps dispatching the
/// notch's messages (animation, timers) while it is open.
pub fn show_menu(hwnd: HWND, x: i32, y: i32) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        for item in MENU {
            let _ = match item {
                Some((command, text)) => AppendMenuW(menu, MF_STRING, command as usize, text),
                None => AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()),
            };
        }
        let _ = SetMenuDefaultItem(menu, Command::Open as u32, 0);
        // The menu closes on a click elsewhere only if its owner is the
        // foreground window, and the shell lets the clicked icon's owner take
        // the foreground; WM_NULL afterwards lets a second right click work
        let _ = SetForegroundWindow(hwnd);
        let align = if GetSystemMetrics(SM_MENUDROPALIGNMENT) != 0 {
            TPM_RIGHTALIGN
        } else {
            TPM_LEFTALIGN
        };
        let _ = TrackPopupMenuEx(menu, (align | TPM_RIGHTBUTTON).0, x, y, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lparam(event: u32, id: u32) -> LPARAM {
        LPARAM(((id << 16) | event) as isize)
    }

    #[test]
    fn test_menu_order_and_commands() {
        let labels: Vec<Option<String>> = MENU
            .iter()
            .map(|i| i.map(|(_, t)| unsafe { t.to_string().unwrap() }))
            .collect();
        assert_eq!(
            labels,
            [
                Some("&Open Oeca".to_string()),
                Some("Open &Settings".to_string()),
                None,
                Some("&Quit Oeca".to_string()),
            ]
        );
        let commands: Vec<Command> = MENU.iter().flatten().map(|(c, _)| *c).collect();
        assert_eq!(commands, [Command::Open, Command::Settings, Command::Quit]);
        // Each id maps back to its command; unrelated WM_COMMANDs are ignored
        for c in commands {
            assert_eq!(Command::from_id(c as usize), Some(c));
        }
        for id in [0, 1, 2, 0xFF, 0x103, 0xFFFF] {
            assert_eq!(Command::from_id(id), None, "{id:#x}");
        }
        // Access keys are distinct (keyboard navigation)
        let keys: Vec<char> = labels
            .iter()
            .flatten()
            .map(|l| l.chars().skip_while(|&c| c != '&').nth(1).unwrap())
            .collect();
        let mut unique = keys.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(keys.len(), unique.len(), "{keys:?}");
    }

    #[test]
    fn test_decode_version4_notifications() {
        // Right click / menu key: the menu at the anchor (negative on a
        // monitor left of or above the primary one)
        let at = |x: i16, y: i16| WPARAM(((y as u16 as usize) << 16) | x as u16 as usize);
        assert_eq!(
            decode(at(1800, 1050), lparam(WM_CONTEXTMENU, TRAY_ID)),
            Some(Event::Menu { x: 1800, y: 1050 })
        );
        assert_eq!(
            decode(at(-1200, -40), lparam(WM_CONTEXTMENU, TRAY_ID)),
            Some(Event::Menu { x: -1200, y: -40 })
        );
        // Left click and keyboard selection open Oeca
        assert_eq!(
            decode(at(0, 0), lparam(NIN_SELECT, TRAY_ID)),
            Some(Event::Activate)
        );
        assert_eq!(
            decode(at(0, 0), lparam(NIN_KEYSELECT, TRAY_ID)),
            Some(Event::Activate)
        );
        // Hover, popups and other icons' ids are ignored
        use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_MOUSEMOVE};
        for e in [WM_MOUSEMOVE, WM_LBUTTONDOWN, 0x406 /* NIN_POPUPOPEN */] {
            assert_eq!(decode(at(5, 5), lparam(e, TRAY_ID)), None, "{e:#x}");
        }
        assert_eq!(decode(at(5, 5), lparam(WM_CONTEXTMENU, TRAY_ID + 1)), None);
    }

    #[test]
    fn test_taskbar_created_is_registered_once() {
        let id = taskbar_created();
        assert!(
            (0xC000..=0xFFFF).contains(&id),
            "registered message range: {id:#x}"
        );
        assert_eq!(taskbar_created(), id);
        assert!(id != WM_APP_TRAY);
    }

    /// Real notification area (run explicitly: `cargo test tray_live -- --ignored`).
    /// Adds, restores (as after an Explorer restart) and removes a tray icon
    /// for a hidden test window, asking the shell where the icon is each time.
    #[test]
    #[ignore]
    fn tray_live_add_restore_remove() {
        use windows::Win32::UI::Shell::{NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect};
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
        };
        // A hidden top-level window (the shell needs a real owner window)
        let owner = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Oeca tray test"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        let in_tray = || {
            let id = NOTIFYICONIDENTIFIER {
                cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
                hWnd: owner,
                uID: TRAY_ID,
                ..Default::default()
            };
            unsafe { Shell_NotifyIconGetRect(&id) }.is_ok()
        };
        let mut tray = TrayIcon::default();
        assert!(tray.add(owner), "shell accepted the icon");
        assert!(tray.icon.is_some(), "embedded icon resource loaded");
        assert!(in_tray(), "icon present");
        assert!(tray.add(owner), "adding again is a no-op");
        // Explorer restart: the shell forgot the icon; restore re-adds it
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &base_data(owner));
        }
        assert!(!in_tray());
        assert!(tray.restore(owner));
        assert!(in_tray(), "icon restored");
        tray.remove();
        assert!(!in_tray(), "icon removed");
        tray.remove();
        // Dropping a shown icon removes it too
        let mut again = TrayIcon::default();
        assert!(again.add(owner));
        drop(again);
        assert!(!in_tray(), "removed on drop");
        unsafe {
            let _ = DestroyWindow(owner);
        }
    }

    #[test]
    fn test_remove_without_add_is_a_no_op() {
        // Never shown: removing (twice) touches nothing and stays hidden
        let mut tray = TrayIcon::default();
        tray.remove();
        tray.remove();
        assert!(!tray.is_shown());
    }
}
