//! Embeds Oeca's Windows resources (icon + version information) into the
//! executable: compiles `assets/windows/oeca.rc` with the Windows SDK resource
//! compiler (`rc.exe`) and links the result. The MSVC toolchain already
//! requires the Windows SDK, so this adds no prerequisite. Set `RC` to a
//! specific `rc.exe` to override the lookup.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/windows/oeca.rc");
    println!("cargo:rerun-if-changed=assets/branding/Oeca.ico");
    println!("cargo:rerun-if-env-changed=RC");

    // General Sans is embedded only when its files are present: its licence
    // forbids publishing them in the repository (see assets/fonts/README.md)
    println!("cargo:rustc-check-cfg=cfg(oeca_bundled_font)");
    println!("cargo:rerun-if-changed=assets/fonts");
    let fonts =
        ["Regular", "Medium", "Semibold"].map(|w| format!("assets/fonts/GeneralSans-{w}.otf"));
    if fonts.iter().all(|f| Path::new(f).is_file()) {
        println!("cargo:rustc-cfg=oeca_bundled_font");
    } else if env::var("PROFILE").as_deref() == Ok("release") {
        println!(
            "cargo:warning=General Sans is not bundled (assets/fonts/README.md): \
             this build uses the installed fonts"
        );
    }

    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("windows-msvc") {
        return;
    }

    // Version from Cargo.toml: "0.1.0" and 0,1,0,0
    let ver = |k: &str| env::var(k).unwrap_or_else(|_| "0".into());
    let (major, minor, patch) = (
        ver("CARGO_PKG_VERSION_MAJOR"),
        ver("CARGO_PKG_VERSION_MINOR"),
        ver("CARGO_PKG_VERSION_PATCH"),
    );
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let header = format!(
        "#define OECA_VERSION_BINARY {major},{minor},{patch},0\n\
         #define OECA_VERSION_STRING \"{}\"\n",
        env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION")
    );
    std::fs::write(out.join("version.h"), header).expect("write version.h");

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let res = out.join("oeca.res");
    let rc = find_rc();
    // Run from the crate root so the script's asset paths resolve
    let status = Command::new(&rc)
        .current_dir(&manifest)
        .arg("/nologo")
        .arg("/I")
        .arg(&out)
        .arg("/fo")
        .arg(&res)
        .arg("assets/windows/oeca.rc")
        .status()
        .unwrap_or_else(|e| {
            panic!(
                "could not run {} ({e}): the Windows SDK resource compiler is required. \
                 Install the Windows 10/11 SDK (part of the Visual Studio C++ build tools) \
                 or set RC to the path of rc.exe.",
                rc.display()
            )
        });
    assert!(status.success(), "rc.exe failed: {status}");

    // link.exe takes compiled resources as an input file
    println!("cargo:rustc-link-arg-bins={}", res.display());
}

/// `RC` if set; else the newest Windows 10/11 SDK's rc.exe for the host
/// architecture (from a Developer prompt's `WindowsSdkVerBinPath`, or the
/// SDK's default install location); else `rc.exe` on PATH.
fn find_rc() -> PathBuf {
    if let Some(rc) = env::var_os("RC") {
        return rc.into();
    }
    let arch = match env::var("HOST").unwrap_or_default().split('-').next() {
        Some("aarch64") => "arm64",
        Some("i686") | Some("i586") => "x86",
        _ => "x64",
    };
    if let Some(dir) = env::var_os("WindowsSdkVerBinPath") {
        let rc = Path::new(&dir).join(arch).join("rc.exe");
        if rc.is_file() {
            return rc;
        }
    }
    let bin = env::var_os("ProgramFiles(x86)")
        .or_else(|| env::var_os("ProgramFiles"))
        .map(|p| PathBuf::from(p).join("Windows Kits").join("10").join("bin"));
    let mut versions: Vec<(Vec<u32>, PathBuf)> = bin
        .and_then(|b| std::fs::read_dir(b).ok())
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let parts: Vec<u32> = name
                .split('.')
                .map(|p| p.parse().ok())
                .collect::<Option<_>>()?;
            let rc = e.path().join(arch).join("rc.exe");
            rc.is_file().then_some((parts, rc))
        })
        .collect();
    versions.sort();
    versions
        .pop()
        .map_or_else(|| PathBuf::from("rc.exe"), |(_, rc)| rc)
}
