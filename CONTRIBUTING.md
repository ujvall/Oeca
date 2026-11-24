# Contributing to Oeca

Oeca is free and open-source software under the [MIT License](LICENSE), maintained by one person, [@ujvall](https://github.com/ujvall). Bug reports, feature requests and pull requests are welcome.

## Reporting a problem or suggesting a feature

Open an issue in [Issues](https://github.com/ujvall/Oeca/issues). For a bug, include your Windows version, the Oeca version (Settings → About) and the steps that show the problem. Please don't paste private clipboard contents or other personal data.

## Proposing a change

1. For anything larger than a small fix, open an issue first so we can agree on the approach.
2. Fork the repository and make your change on a branch based on `master`.
3. Keep the change focused: one fix or feature per pull request, with no unrelated refactoring.
4. Run the checks below, then open a pull request that explains what changed and why.

Every pull request is reviewed by the maintainer before it is merged. Changes to build scripts, the installer script or the GitHub Actions workflows get the same careful review as code, because they decide what goes into a release.

By submitting a pull request, you agree that your contribution is licensed under the MIT License, the same as the rest of the project. Only submit work you have the right to contribute; third-party code, icons or other assets must come with a licence compatible with MIT and be added to [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt).

## Safety rules for development

These apply to everyone working on Oeca, people and coding agents alike, and take precedence over older notes and recipes.

### User data

- Never launch the real Oeca installation for testing without its user's explicit permission. Every release build is the real Oeca: `target/release`, the installer's output and any copy of them use the user's data.
- Use isolated test instances and synthetic data. A debug build is a test instance, with its data in `Oeca-Test-Data` beside the executable. For an optimized one: `cargo build --release --config profile.release.debug-assertions=true --target-dir target/test-instance`.
- Never use the real user's AppData for tests. `%TEMP%` is inside AppData, so code and tests use `data_dir::temp_dir()`, not `std::env::temp_dir()`.
- Never delete or overwrite real clipboard history during testing. Don't run `clipboard_live_*`, or `--ignored` without a test name: ignored tests can replace the system clipboard or take desktop-wide resources.
- Preserve user data when identity or ownership is ambiguous: don't load, replace, migrate or delete it. Missing or damaged identity is never a reason to clean up.
- User data is written and deleted only through `data_dir::replace` and `data_dir::remove`, in the folder `data_dir::dir()` returns. Don't bypass or weaken the folder checks or the writer lock to make something run.
- A test build takes nothing on the desktop (hotkeys, hooks, raw input, audio or media sessions, tray icon, topmost band). A new desktop-wide registration gets the same `data_dir::ISOLATED` guard and a line in `test_test_builds_take_nothing_on_the_desktop`.
- Never stop or disrupt a user's running Oeca, and never touch recovery copies or backups without permission.

### Working tree

- Check `git status` and the existing diffs before editing.
- Preserve unrelated uncommitted changes: no reset, checkout, clean or stash over someone else's work.
- A coding agent never commits or pushes without the maintainer's explicit permission, never rewrites history or changes repository permissions, and never adds AI attribution or co-author trailers.

## Requirements for planned work

### Oeca Hop: lifecycle

These bind the Oeca Hop UI and integration stages (Hop is called Nearby in the code: `src/nearby*`, `nearby-web/`).

- Hop discovery and LocalSend signaling start only when Oeca is actually opened.
- If the user disables `Start on open`, Hop must not start automatically at Windows startup or independently in the background.
- Hop must remain inactive while Oeca is closed.
- No independent always-on discovery service or background process.
- When Oeca is manually opened, Hop initializes according to the user's setting.

`Start on open` is Oeca's existing **Start on launch** setting (Settings; the per-user Run entry that opens Oeca at sign-in), not a separate Hop setting. Hop gets no startup entry or autostart path of its own: at Windows startup it can run only because Start on launch opened Oeca.

"The user's setting" is a separate Hop on/off setting, **off by default** (also for existing users, and when the saved value is missing or unreadable). Off: no discovery and no signaling connection, however Oeca was opened. On: Hop runs while Oeca is open, whether opened by hand or by Start on launch. The two settings are independent: changing one never changes the other.

## Building and testing

Oeca builds on Windows 10 or later (x64) with Rust (MSVC toolchain, edition 2024) and the Windows 10/11 SDK. See [Build from source](README.md#build-from-source).

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release --locked  # the real Oeca: build it, don't launch it to test
```

For changes to the website (`website/`, Node.js 20.9 or later):

```powershell
cd website
npm ci
npm run lint
npm run typecheck
npm run build
```
