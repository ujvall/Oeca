<p align="center">
  <img src="assets/readme/hero.png" alt="Oeca" width="100%">
</p>

<p align="center">
  A native notch for the top of your Windows desktop: the time, what is playing, a timer and your clipboard history, one click away.
</p>

<p align="center">
  <a href="https://github.com/ujvall/Oeca/releases/download/v0.5.0/Oeca-Setup-0.5.0.exe"><img src="assets/readme/button-download.png" alt="Download for Windows" height="44"></a>
  <a href="https://github.com/ujvall/Oeca/releases"><img src="assets/readme/button-releases.png" alt="All releases" height="44"></a>
  <img src="assets/readme/chip-windows.png" alt="Windows 10 or later, 64-bit" height="44">
  <img src="assets/readme/chip-version.png" alt="Version 0.5.0" height="44">
</p>

<br>

<p align="center">
  <img src="assets/readme/clipboard.png" alt="The expanded notch showing the Clipboard space with four items" width="570">
  <br>
  <sub>The Clipboard space, captured from Oeca 0.1.0 (example clipboard items).</sub>
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

Click an item in the Clipboard space to copy it again, or drag image files onto the left half of the drop page to add them. A notification-area icon offers **Open**, **Settings** and **Quit**.

### Convert images and audio

Drag files onto the notch and drop them on **Convert** (audio always goes there). A compact workspace lists each file with its format and size; tick the ones you want, pick an output format, and press **Convert**. Then **Save** each result, or **Save all**, beside the originals or in a folder you choose.

- **Images:** open PNG, JPEG, WebP, BMP, TIFF, GIF and AVIF; write PNG, JPEG, WebP (lossless), BMP and TIFF. Transparency flattens onto a background you choose for JPEG, EXIF rotation is applied, and colour profiles are kept where the format allows.
- **Audio:** open WAV, FLAC, MP3, Ogg Vorbis and M4A (AAC, ALAC); write WAV, FLAC and Ogg Vorbis. Tags carry over, and lossless sources keep their bit depth. MP3, AAC and Opus can be opened but not created.
- **Bulk:** with several files selected, every output format of their kind stays on offer; a file already in that format is copied unchanged.
- **Safe and offline:** everything runs on your PC, in the background. Originals are never modified, existing files are never overwritten (you get `name (2).ext`), and nothing is uploaded.
- With Always on top off, Oeca comes forward while you drag files in and stays in front until you press Convert, collapse the notch or close the workspace.

### Oeca Hop

Send files between your PC and your phone, or any device running LocalSend, from the notch's **Hop** space. On a phone, open the Oeca Hop web page ([oeca-hop.vercel.app](https://oeca-hop.vercel.app)) in the browser: nothing to install.

- **Off until you turn it on**, in **Settings → Oeca Hop**, after a privacy notice that says exactly what is shared. It runs only while Oeca is open, and starts with Oeca only if Start on launch is on.
- **Verified devices:** the first time two devices connect, both show the same code; confirm it once and the device is remembered. You can reset this PC's Hop identity at any time.
- **Nothing arrives unasked:** every incoming transfer waits for your approval. Received files go to `Downloads\Oeca Hop`, are marked as downloaded from the internet, and are never opened by Hop.
- **Direct and encrypted:** files travel over WebRTC straight between the two devices. LocalSend's public signaling server only introduces them; see [Privacy](#privacy).

## How it is built

<p>
  <img src="assets/readme/chip-rust.png" alt="Rust 2024" height="36">
  <img src="assets/readme/chip-win32.png" alt="Win32" height="36">
  <img src="assets/readme/chip-d2d.png" alt="Direct2D" height="36">
  <img src="assets/readme/chip-dwrite.png" alt="DirectWrite" height="36">
  <img src="assets/readme/chip-smtc.png" alt="Windows media controls" height="36">
  <img src="assets/readme/chip-noruntime.png" alt="No runtime" height="36">
</p>

Oeca is a free, open-source (MIT) single Rust executable: no runtime, UI framework or web view. The app itself uses only Microsoft's [`windows`](https://crates.io/crates/windows) and `windows-core` crates. The converter adds pure-Rust codecs ([`image`](https://crates.io/crates/image), [Symphonia](https://github.com/pdeljanov/Symphonia), `flacenc`, and a vendored copy of `avif-decode` with `rav1d`) plus `vorbis_rs`, which bundles the C libogg and libvorbis. Oeca Hop adds a vendored copy of [LocalSend](https://github.com/localsend/localsend)'s Rust core with `webrtc` and `tokio` (and a vendored `dtls`). All of them are listed, with their licences, in [`THIRD-PARTY-NOTICES.txt`](THIRD-PARTY-NOTICES.txt).

- **Window.** One layered, per-monitor-DPI-aware Win32 window. Pixels outside the notch shape pass clicks through to the windows below.
- **Rendering.** Direct2D and DirectWrite draw into a premultiplied 32-bit bitmap that is composited with `UpdateLayeredWindow`. Brushes, text formats and bitmaps are cached per artwork and DPI.
- **Event-driven.** A blocking `GetMessageW` loop; clipboard changes, media sessions (`Windows.Media.Control`) and the tray all arrive as messages, with no polling. Animation timers run only while something moves; when nothing is playing or animating, Oeca only wakes for the clock's once-a-minute update.
- **Motion.** Opening, closing and switching spaces run on critically damped springs that keep their velocity when interrupted, and snap when Reduced motion is on.
- **Storage.** Settings live in `%LOCALAPPDATA%\Oeca\settings.ini`. Clipboard history is saved beside it, tied to the current Windows sign-in through a volatile registry key, so a new session never sees the previous one's history.

## Install

1. Download `Oeca-Setup-0.5.0.exe` from [Releases](https://github.com/ujvall/Oeca/releases).
2. Run it. Oeca installs for your Windows account only, to `%LOCALAPPDATA%\Programs\Oeca`; no administrator rights are needed.
3. Look for the notch at the top of your screen and the Oeca icon in the notification area.

The installer and app are not code-signed yet, so Windows SmartScreen may ask you to confirm; see the [code signing policy](#code-signing-policy). Oeca adds nothing that runs at startup unless you turn on **Start on launch** in Settings.

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

`installer/Oeca.iss` builds the setup with [Inno Setup](https://jrsoftware.org/isinfo.php) **6.7.0 or later** (its dark wizard style and colours need 6.7). Build the release executable first, with `tools/release.ps1`: it is `cargo build --release --locked` without this computer's folders in the executable (Rust records where each crate's source was, which on a local build names your user folder), and it checks that none are left. The installer script refuses to package an executable that is older than the sources.

```powershell
powershell -ExecutionPolicy Bypass -File tools\release.ps1
& "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe" installer\Oeca.iss
```

The output is `target\installer\Oeca-Setup.exe`, and its version comes from `Cargo.toml`. The same steps run on GitHub Actions in [`.github/workflows/release.yml`](.github/workflows/release.yml), which builds, tests and packages Oeca on GitHub-hosted Windows runners. Upgrading or reinstalling asks a running Oeca to quit, replaces it in place and keeps settings, clipboard history and Start on launch. Adjust the compiler path if Inno Setup is installed elsewhere.

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
│   ├── dragdrop.rs            File drag and drop, temporary window priority
│   ├── convert.rs             Image and audio conversion engine
│   ├── convert_batch.rs       Converter batch state and workers
│   ├── settings_window.rs     Settings window
│   ├── settings_store.rs      Settings file
│   ├── startup.rs             Start on launch
│   ├── data_dir.rs            Data folder checks, writer lock, test isolation
│   ├── nearby.rs, nearby/     Oeca Hop: connection, trusted devices, transfers
│   ├── nearby_ui.rs           Oeca Hop space in the notch
│   └── tray.rs                Notification area icon and menu
├── nearby-web/                Oeca Hop web page for phones (Nuxt, static)
├── assets/                    Icon, version resource and README images
├── installer/                 Inno Setup script and wizard images
├── tools/                     Release build script
├── vendor/avif-decode/        Vendored AVIF decoder (BSD-3-Clause)
├── vendor/localsend-core/     Vendored LocalSend core (Apache-2.0)
├── vendor/dtls/               Vendored DTLS for WebRTC (MIT or Apache-2.0)
└── build.rs                   Embeds the icon and version information
```

## Privacy

Oeca makes no network requests unless you turn on a feature that needs them. Check for updates (Settings, About) and the optional Check for updates automatically setting, off by default, send one HTTPS request to GitHub's public releases API, with no account or personal data. Oeca Hop, off by default, connects while it is on and Oeca is open to LocalSend's public signaling server (`public.localsend.org`) and STUN server (`stun.localsend.org`), run by the LocalSend project: they see this PC's public IP address, and the signaling server also sees its Hop name, device type and identity, and lists it to devices that share its public address (on public Wi-Fi or a shared VPN, that can include strangers). Files go directly between devices, encrypted, never through those servers. The notice in Settings says this in full before Hop starts. Oeca never downloads or installs updates itself, and it has no telemetry, analytics or crash reporting. Its settings and its clipboard history are stored only on your PC, in `%LOCALAPPDATA%\Oeca`; the clipboard history is not encrypted and is discarded when you sign out or restart Windows. The full [privacy policy](website/app/privacy/page.tsx) covers the app, the installer and the website. It will also be published on the Oeca website once that is online.

## Code signing policy

**Status: pending.** Oeca is applying for free code signing from [SignPath Foundation](https://signpath.org). Current releases, including v0.5.0, are not signed. Once the application is accepted, releases will be built by the GitHub Actions workflow in this repository, signed through SignPath and published here, with this notice: Free code signing provided by SignPath.io, certificate by SignPath Foundation.

- Authors, Reviewers and Approvers: [Ujval Sai (@ujvall)](https://github.com/ujvall), currently the only maintainer.
- Every signing request is approved manually by the maintainer, for releases built from this repository only.
- Privacy: this program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it.

The full [code signing policy](website/app/code-signing/page.tsx) will also be published on the Oeca website.

## Contributing

Bug reports, feature requests and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for how to report a problem, propose a change and build and test Oeca locally. Contributions are accepted under the MIT License.

## License and credits

Oeca is free and open-source software, released under the [MIT License](LICENSE). The MIT License covers Oeca's source code and its own assets, including the Oeca icon and the images in `assets/` and `installer/images/`. It does not grant rights to the Oeca name or logo as trademarks: please don't use them to brand other software or imply endorsement.

Oeca was first published under the MIT License, including the v0.1.0 release. For a short time one commit (`c627fd2`) replaced it with a proprietary source-available licence; that change has been reversed, and it never revoked any rights granted under the MIT License.

Third-party components keep their own licences. Those shipped with Oeca, Microsoft's `windows` crates (MIT), the General Sans typeface embedded in official builds (ITF Free Font License, not open source), the Sunghyun Sans typeface used by the timer (SIL Open Font License 1.1) and Inno Setup, are listed with their notices in [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt), which the installer places beside `Oeca.exe`.

- Icons in the app: Oeca's own, drawn from simple shapes by [assets/icons/generate.py](assets/icons/generate.py) and covered by the MIT License like the rest of the code.
- Screenshots of Oeca 0.1.0 in this README and on the website show its earlier icons, [designed by UIcons from Flaticon](https://www.flaticon.com/uicons), used under Flaticon's free licence with attribution.
- Oeca's text and wordmark are set in General Sans, © Indian Type Foundry ([Fontshare](https://www.fontshare.com/fonts/general-sans)). General Sans is not open source: it is used under the [ITF Free Font License](https://www.fontshare.com/licenses/itf-ffl), which lets official builds embed it in `Oeca.exe` but does not allow publishing the font files here, so they are not in this repository. To build with it, see [assets/fonts/README.md](assets/fonts/README.md); without it, Oeca uses the fonts installed on the PC.
