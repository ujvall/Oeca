<p align="center">
  <img src="assets/readme/hero.png" alt="Oeca" width="100%">
</p>

<p align="center">
  A native notch for the top of your Windows desktop: the time, what is playing, a timer and your clipboard history, one click away.
</p>

<p align="center">
  <a href="https://github.com/ujvall/Oeca/releases/latest/download/Oeca-Setup.exe"><img src="assets/readme/button-download.png" alt="Download for Windows" height="44"></a>
  <a href="https://github.com/ujvall/Oeca/releases"><img src="assets/readme/button-releases.png" alt="All releases" height="44"></a>
  <img src="assets/readme/chip-windows.png" alt="Windows 10 or later, 64-bit" height="44">
  <img src="assets/readme/chip-version.png" alt="Version 0.1.0" height="44">
</p>

<br>

<p align="center">
  <img src="assets/readme/clipboard.png" alt="The expanded notch showing the Clipboard space with four items" width="680">
  <br>
  <sub>The Clipboard space, captured from the running app (example clipboard items).</sub>
</p>

## Features

<p align="center">
  <img src="assets/readme/card-notch.png" alt="Notch: the time, track artwork, a visualizer and a running timer at a glance. Click to open; it closes again 5, 10 or 15 seconds after you click elsewhere." width="300">
  <img src="assets/readme/card-media.png" alt="Media: play, pause, previous, next and seek in any app that uses the Windows media controls." width="300">
  <img src="assets/readme/card-timer.png" alt="Timer: a countdown from five seconds to twelve hours, with pause and resume. Stays visible in the collapsed notch." width="300">
  <br>
  <img src="assets/readme/card-clipboard.png" alt="Clipboard: copied text and images, 5, 10 or 20 items. Copy an item again, delete it or clear the history; drag image files onto the notch to add them. Kept across restarts until you sign out." width="300">
  <img src="assets/readme/card-settings.png" alt="Settings: accent colour, always on top, reduced motion and start on launch, in the notch or the full Settings window. Saved and restored." width="300">
  <img src="assets/readme/card-native.png" alt="Native: one small Rust executable on Win32 and Direct2D, with no runtime and no web view." width="300">
</p>

Click an item in the Clipboard space to copy it again, or drag image files onto the notch to add them. A notification-area icon offers **Open**, **Settings** and **Quit**.

## How it is built

<p>
  <img src="assets/readme/chip-rust.png" alt="Rust 2024" height="36">
  <img src="assets/readme/chip-win32.png" alt="Win32" height="36">
  <img src="assets/readme/chip-d2d.png" alt="Direct2D" height="36">
  <img src="assets/readme/chip-dwrite.png" alt="DirectWrite" height="36">
  <img src="assets/readme/chip-smtc.png" alt="Windows media controls" height="36">
  <img src="assets/readme/chip-noruntime.png" alt="No runtime" height="36">
</p>

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

You may build and run an unmodified copy to evaluate, test or review Oeca; redistributing builds needs permission (see [LICENSE](LICENSE)).

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

```text
Oeca/
├── src/
│   ├── main.rs                Entry point
│   ├── window.rs              Window, message loop, gestures, single instance, lifecycle
│   ├── renderer.rs            Direct2D and DirectWrite rendering
│   ├── layout.rs              DPI-scaled layout
│   ├── config.rs              Geometry, springs and colours
│   ├── space.rs               Spaces and the settings model
│   ├── media.rs               Media sessions, artwork and visualizer
│   ├── clock.rs               Clock
│   ├── timer.rs               Countdown timer
│   ├── clipboard.rs           Clipboard history
│   ├── clipboard_store.rs     Session-scoped history storage
│   ├── dragdrop.rs            Image drag and drop
│   ├── settings_window.rs     Settings window
│   ├── settings_store.rs      Settings file
│   ├── startup.rs             Start on launch
│   └── tray.rs                Notification area icon and menu
├── assets/                    Icon, version resource and README images
├── installer/                 Inno Setup script and wizard images
└── build.rs                   Embeds the icon and version information
```

## Contributing

Contributions (pull requests or other code) are not being accepted at the moment, and there is no contributor agreement. Bug reports and feature requests in [Issues](https://github.com/ujvall/Oeca/issues) are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License and credits

Oeca is source-available and proprietary. It is not open source. The repository is public so that you can read and inspect how Oeca works, but public visibility does not by itself give you permission to copy, modify, redistribute or reuse the code. You may view the code, use and fork it through GitHub as GitHub's Terms of Service allow (forks stay on GitHub; that is not permission to publish or distribute the code elsewhere), and install and use the official releases yourself, including for work or study. Anything else, such as redistributing the code, publishing modified versions, commercial reuse, sublicensing or deploying Oeca for other people, needs the author's prior written permission, except where applicable law allows it. Referring to Oeca by name, for example in reviews or discussion, is fine. The full terms are in [LICENSE](LICENSE).

Earlier versions were published under the MIT License, including the v0.1.0 release and the commits that contained the MIT text. The new licence applies to versions published with it, and within those, to what is new or changed. It does not take back the rights anyone already received under the MIT License: those earlier versions, and the code as it appeared in them, stay available under the MIT License.

The Oeca name, icon and branding assets remain the property of the author: `assets/branding/` and the images rendered from the Oeca icon, including the installer's `installer/images/` and this README's `assets/readme/`. Third-party materials are not the author's and stay under their own licences: the Rust dependencies (each under the licence in its own package metadata, such as MIT OR Apache-2.0 for Microsoft's `windows` crates) and the items credited below. The notices for the third-party components that ship with Oeca (the `windows` crates compiled into `Oeca.exe`, the icons it draws, and Inno Setup) are in [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt), which the installer places beside `Oeca.exe`.

- Clipboard, House blank, Music-alt, Palette, Info, Settings and Trash icons: [designed by UIcons from Flaticon](https://www.flaticon.com/uicons), used under Flaticon's free licence (free for commercial use with attribution).
- Drop page icon: "Inbox", created by Jivan from the [Noun Project](https://thenounproject.com/icon/1052548/), used under the Noun Project free licence (with attribution).
- The Oeca wordmark is set in General Sans by the Indian Type Foundry ([Fontshare](https://www.fontshare.com)); the font itself is not included in this repository.
- The screenshot's background is a desktop wallpaper from [wallhaven](https://wallhaven.cc); it remains its creator's and is not covered by Oeca's licence.
