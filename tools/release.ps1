# Builds target/release/Oeca.exe for distribution: `cargo build --release --locked`,
# with this computer's folders taken out of the executable. Rust records the
# source path of every crate it compiles (for its error messages), and on a
# local build those paths name the user folder of whoever built it. Then
# compile installer/Oeca.iss as usual. GitHub Actions builds need none of this.
#
#   powershell -ExecutionPolicy Bypass -File tools/release.ps1
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

# The user folder, and Cargo's and rustup's homes where they are elsewhere
$folders = @([Environment]::GetFolderPath("UserProfile"), $env:CARGO_HOME, $env:RUSTUP_HOME) |
    Where-Object { $_ } | ForEach-Object { $_.TrimEnd("/", [char]92) } | Select-Object -Unique
$env:RUSTFLAGS = ($folders | ForEach-Object { "--remap-path-prefix=$_=~" }) -join " "

# One job: more has run out of memory compiling the larger crates
cargo build --release --locked -j 1
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# Checked, not assumed: none of those folders may be left in the executable
$exe = [IO.File]::ReadAllBytes("target/release/Oeca.exe")
$text = [Text.Encoding]::GetEncoding(28591).GetString($exe)
$left = $folders | Where-Object { $text.IndexOf($_, [StringComparison]::OrdinalIgnoreCase) -ge 0 }
if ($left) {
    Write-Error "target/release/Oeca.exe still contains: $($left -join ', '). Do not distribute it."
    exit 1
}
Write-Host "target/release/Oeca.exe is built, without these folders in it: $($folders -join ', ')"
