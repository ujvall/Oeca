# Renders the installer's wizard images from the existing Oeca icon
# (assets/branding/Oeca.png) into installer/images/. Run from the repository
# root after changing the icon:
#   powershell -NoProfile -File installer\make-wizard-images.ps1
#
# - WizardImage-<w>x<h>.png: the Welcome / Finish side panel: the page's
#   own charcoal (one seamless surface), the icon centred a little above
#   the middle.
# - WizardSmallImage-<n>.png: the icon alone, inset, for the page header.
# Several sizes each, so Inno Setup picks a sharp one at every DPI.
# Uses only .NET's System.Drawing (no downloads, no extra tools).

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$root = Split-Path -Parent $PSScriptRoot
$icon = [System.Drawing.Image]::FromFile((Join-Path $root 'assets\branding\Oeca.png'))
$out = Join-Path $PSScriptRoot 'images'
New-Item -ItemType Directory -Force $out | Out-Null

# Must match WizardBackColor in Oeca.iss
$panel = [System.Drawing.Color]::FromArgb(255, 18, 18, 20)    # #121214

function New-Canvas($w, $h) {
    $bmp = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
    return @($bmp, $g)
}

# Side panel: (width, height) per DPI step, all the same aspect
foreach ($size in @(@(200, 384), @(250, 480), @(300, 576), @(400, 768))) {
    $w, $h = $size
    $bmp, $g = New-Canvas $w $h
    $g.Clear($panel)
    $side = [int]($w * 0.46)
    $cx = $w / 2; $cy = $h * 0.42
    # (No glow: on near-black, 8-bit gradients band into visible rings.)
    $g.DrawImage($icon, [int]($cx - $side / 2), [int]($cy - $side / 2), $side, $side)
    $g.Dispose()
    $bmp.Save((Join-Path $out "WizardImage-${w}x${h}.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}

# Header icon: transparent background, the icon inset (Inno fills the
# header box edge to edge, which would crop an unpadded icon)
foreach ($n in @(64, 80, 96, 128)) {
    $bmp, $g = New-Canvas $n $n
    $g.Clear([System.Drawing.Color]::Transparent)
    $side = [int]($n * 0.72); $o = [int](($n - $side) / 2)
    $g.DrawImage($icon, $o, $o, $side, $side)
    $g.Dispose()
    $bmp.Save((Join-Path $out "WizardSmallImage-$n.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}
$icon.Dispose()
Get-ChildItem $out | ForEach-Object { "$($_.Name) ($($_.Length) bytes)" }
