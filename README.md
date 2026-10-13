<p align="center">
  <img src="assets/readme/hero.png" alt="Oeca" width="100%">
</p>

<p align="center">
  A native notch for the top of your Windows desktop: the time, what is playing, a timer and your clipboard history, one click away.
</p>

<p align="center">
  <a href="https://github.com/ujvall/Oeca/releases/latest/download/Oeca-Setup.exe"><img src="assets/readme/button-download.png" alt="Download for Windows" height="44"></a>
  <img src="assets/readme/chip-windows.png" alt="Windows 10 or later, 64-bit" height="44">
  <img src="assets/readme/chip-version.png" alt="Version 0.1.0" height="44">
</p>

<p align="center">
  <sub>Downloads <code>Oeca-Setup.exe</code> from the latest release · <a href="https://github.com/ujvall/Oeca/releases">All releases</a></sub>
</p>

---

## Features

**Notch.** A small black notch at the top centre of the screen. Collapsed, it shows the time, the current track's artwork, a playback visualizer and a running countdown. Click it to open; it closes again a few seconds after you click elsewhere (5, 10 or 15 s).

**Media.** Works with any app that reports to Windows' media controls. Shows title, artist and artwork with play/pause, previous and next; the Music space adds a scrubber for seeking and the visualizer.

**Timer.** A countdown from 5 seconds to 12 hours, with pause and resume. While it runs, it stays visible in the collapsed notch.

**Clipboard.** Remembers copied text and images (5, 10 or 20 items). Click an item to copy it again, delete items one by one or clear the history. Image files dragged onto the notch are added too. History survives restarting Oeca and is cleared when you sign out or restart Windows.

<p align="center">
  <img src="assets/readme/clipboard.png" alt="The expanded notch showing the Clipboard space with four items" width="680">
  <br>
  <sub>The Clipboard space, captured from the running app (example clipboard items).</sub>
</p>

**Settings.** Accent colour, always on top, reduced motion, start on sign-in and more, in the notch itself or the full Settings window. Settings are saved and restored. A tray icon offers Open, Settings and Quit.

## How it is built

Oeca is a single Rust executable whose only dependencies are Microsoft's [`windows`](https://crates.io/crates/windows) and `windows-core` crates: no runtime, UI framework or web view.

- **Window.** One layered, per-monitor-DPI-aware Win32 window. Pixels outside the notch shape pass clicks through to the windows below.
- **Rendering.** Direct2D and DirectWrite draw into a premultiplied 32-bit bitmap that is composited with `UpdateLayeredWindow`. Brushes, text formats and bitmaps are cached per artwork and DPI.
- **Event-driven.** A blocking `GetMessageW` loop; clipboard changes, media sessions (`Windows.Media.Control`) and the tray all arrive as messages, with no polling. Animation timers run only while something moves; when nothing is playing or animating, Oeca only wakes for the clock's once-a-minute update.
- **Motion.** Opening, closing and switching spaces run on critically damped springs that keep their velocity when interrupted, and snap when Reduced motion is on.
- **Storage.** Settings live in `%LOCALAPPDATA%\Oeca\settings.ini`. Clipboard history is saved beside it, tied to the current Windows sign-in through a volatile registry key, so a new session never sees the previous one's history.

## Install

1. Download `Oeca-Setup.exe` from [Releases](https://github.com/ujvall/Oeca/releases).
2. Run it. Oeca installs for your Windows account only, to `%LOCALAPPDATA%\Programs\Oeca`; no administrator rights are needed.
3. Look for the notch at the top of your screen and the Oeca icon in the notification area.

The installer and app are not code-signed, so Windows SmartScreen may ask you to confirm. Oeca adds nothing that runs at startup unless you turn on **Start on launch** in Settings.

Uninstall from **Settings → Apps → Installed apps**. This removes the app, its shortcuts and its Start on launch entry, and keeps your data in `%LOCALAPPDATA%\Oeca`; delete that folder yourself to remove it. After a reinstall, Start on launch is off until you turn it on again.

## Build from source

Requires Windows 10 or later (x64), Rust with the MSVC toolchain (edition 2024), and the Windows 10/11 SDK, which comes with the Visual Studio C++ build tools. `build.rs` embeds the icon and version information with the SDK's `rc.exe` (set `RC` to use a specific one).

```powershell
git clone https://github.com/ujvall/Oeca.git
cd Oeca
cargo test --locked
cargo build --release --locked
.\target\release\Oeca.exe
```

### Installer

`installer/Oeca.iss` builds the setup with [Inno Setup](https://jrsoftware.org/isinfo.php) **6.7.0 or later** (its dark wizard style and colours need 6.7). Build the release executable first: the script refuses to package one that is older than the sources.

```powershell
cargo build --release --locked
& "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe" installer\Oeca.iss
```

The output is `target\installer\Oeca-Setup.exe`, and its version comes from `Cargo.toml`. Upgrading or reinstalling asks a running Oeca to quit, replaces it in place and keeps settings, clipboard history and Start on launch. Adjust the compiler path if Inno Setup is installed elsewhere.

## Project structure

| Path | Contents |
| --- | --- |
| `src/window.rs` | Window, message loop, gestures, single instance, lifecycle |
| `src/renderer.rs` | Direct2D / DirectWrite rendering |
| `src/layout.rs`, `src/config.rs` | DPI-scaled geometry, springs, colours |
| `src/media.rs` | Media sessions, artwork, visualizer |
| `src/clipboard.rs`, `src/clipboard_store.rs`, `src/dragdrop.rs` | Clipboard history, session storage, drag and drop |
| `src/timer.rs`, `src/clock.rs` | Countdown and clock |
| `src/settings_window.rs`, `src/settings_store.rs`, `src/startup.rs` | Settings window, settings file, Start on launch |
| `src/tray.rs` | Notification area icon and menu |
| `installer/` | Inno Setup script and wizard images |

## License and credits

Oeca's source code is released under the [MIT License](LICENSE). It does not cover the third-party icons credited below, which remain under their own licences, or the Oeca icon and branding assets, which remain the property of the author: `assets/branding/` and the images rendered from the Oeca icon, including the installer's `installer/images/` and this README's `assets/readme/`.

- Clipboard, House blank, Music-alt, Palette, Info, Settings and Trash icons: [Uicons by Flaticon](https://www.flaticon.com/uicons), used under the Flaticon free licence (with attribution).
- Drop page icon: "Inbox" by Jivan from [Noun Project](https://thenounproject.com/icon/1052548/), used under the Noun Project free licence (with attribution).
- The Oeca wordmark is set in General Sans by the Indian Type Foundry ([Fontshare](https://www.fontshare.com)); the font itself is not included in this repository.
- The screenshot's background is a desktop wallpaper from [wallhaven](https://wallhaven.cc); it remains its creator's and is not covered by the MIT License.
