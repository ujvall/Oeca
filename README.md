# Oeca

Oeca is a lightweight, ultra-efficient native Windows notch application built in Rust with native Win32 and Direct2D.

## Status

- **Phase 1 — Foundation**: COMPLETE (Stable borderless layered Win32 window, top-center positioning, DPI awareness, sub-pixel hit testing, clean lifecycle and shutdown).

---

## Architectural Pipeline

```
Win32 OS Messages (WM_MOUSEACTIVATE, WM_NCHITTEST, WM_DPICHANGED, WM_DISPLAYCHANGE)
   ↓
Native Event Handling (window.rs)
   ↓
DPI & Geometry Calculations (config.rs)
   ↓
Direct2D Renderer (renderer.rs)
   ↓
Desktop Window Manager Blit (UpdateLayeredWindow into 32-bit premultiplied ARGB DIB)
```

---

## Technical Specifications

- **Window Styles**: `WS_POPUP`
- **Extended Styles**: `WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`
- **Reference Geometry (@ 96 DPI)**:
  - Width: 220 px
  - Height: 32 px
  - Corner Radius: 14 px
  - Border Width: 1 px
- **Positioning**: Anchored top-center on the primary monitor (`((screen_width - width) / 2).max(0)`).
- **Hit Testing**: Sub-pixel anti-aliased rounded rectangle hit test returning `HTCLIENT` inside and `HTTRANSPARENT` outside (allowing clicks to pass through to underlying applications).
- **Idle CPU**: **0.0%** (blocks in `GetMessageW` event loop; zero polling, zero timers).

---

## Building and Testing

Prerequisites: Rust (MSVC toolchain, `x86_64-pc-windows-msvc`) and the Windows 10/11 SDK, which ships with the Visual Studio C++ build tools. `build.rs` compiles `assets/windows/oeca.rc` (the executable's icon and version information) with the SDK's `rc.exe`; set `RC` to point at a specific `rc.exe` if needed.

```powershell
# Format check
cargo fmt --check

# Check compilation
cargo check

# Run unit tests
cargo test

# Build debug & release executables
cargo build
cargo build --release
```

---

## Windows Installer

`installer/Oeca.iss` builds `Oeca-Setup.exe` with [Inno Setup 6](https://jrsoftware.org/isinfo.php) from the release executable. Build the executable first, then compile the script (the path below is Inno Setup's per-user install location; adjust it if Inno Setup is installed elsewhere):

```powershell
cargo build --release --locked
& "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe" installer\Oeca.iss
```

Output: `target\installer\Oeca-Setup.exe`. The product version comes from the executable's version resource (`Cargo.toml`).

- Installs for the current user only, without administrator rights, to `%LOCALAPPDATA%\Programs\Oeca`.
- Optional Start Menu (on by default) and Desktop (off by default) shortcuts.
- Registers Oeca in Settings → Apps → Installed apps; uninstalling there removes the installed files, shortcuts and that entry, and nothing else.
- Adds no startup entry, service or scheduled task. The installer is not code-signed.

---

## Branding & Packaging Assets

Located under `assets/branding/`, designed by the author:
- `Oeca.png`: the official Oeca icon, featuring the minimalist hardware-notch cutout.
- `Oeca.ico`: Multi-resolution Windows icon (16x16, 32x32, 48x48, 64x64, 128x128, 256x256), embedded in the executable at build time.

---

## Credits

- Clipboard, House blank, Music-alt, Palette, Info, Settings and Trash icons: [Uicons by Flaticon](https://www.flaticon.com/uicons), used under the Flaticon free licence (with attribution).
- Drop page icon: "Inbox" by Jivan from [Noun Project](https://thenounproject.com/icon/1052548/), used under the Noun Project free licence (with attribution).

## Licence scope

Oeca's source code is released under the [MIT License](LICENSE). It does not cover the third-party icons credited above, which remain under their own licences, or the Oeca icon and branding assets (`assets/branding/`), which remain the property of the author.
