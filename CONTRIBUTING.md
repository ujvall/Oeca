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

## Building and testing

Oeca builds on Windows 10 or later (x64) with Rust (MSVC toolchain, edition 2024) and the Windows 10/11 SDK. See [Build from source](README.md#build-from-source).

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release --locked
```

For changes to the website (`website/`, Node.js 20.9 or later):

```powershell
cd website
npm ci
npm run lint
npm run typecheck
npm run build
```
