#![windows_subsystem = "windows"]

mod clipboard;
mod clipboard_store;
mod clock;
mod config;
mod dragdrop;
mod layout;
mod media;
mod renderer;
mod settings_window;
mod space;
mod timer;
mod tray;
mod window;

fn main() {
    if let Err(e) = window::run() {
        // Only fatal initialization failures (window/renderer) reach here; media
        // and other recoverable failures never surface. There is no console in
        // the windows subsystem, so show the reason instead of exiting silently.
        let text: Vec<u16> = format!("Oeca could not start.\n\n{e}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
            MessageBoxW(
                None,
                windows::core::PCWSTR(text.as_ptr()),
                windows::core::w!("Oeca"),
                MB_OK | MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}
