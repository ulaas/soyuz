<#
.SYNOPSIS
    Build an (unsigned) MSIX package of Soyuz with makeappx.exe from the Windows SDK.

.DESCRIPTION
    Stages soyuz.exe, README, licenses and logo PNGs (scaled from
    crates/soyuz-app/assets/icon.png) with AppxManifest.xml, then runs
        makeappx pack /d <stage> /p dist/soyuz-<version>-windows-x64.msix /o
    Sign the result with a certificate whose Subject equals -Publisher:
        signtool sign /fd SHA256 /f cert.pfx /p <password> dist/soyuz-<ver>-windows-x64.msix

.EXAMPLE
    ./installer/msix/build-msix.ps1 -Publisher "CN=Soyuz Dev"
#>
[CmdletBinding()]
param(
    [string]$BinDir = "target/release",
    [string]$OutDir = "dist",
    [string]$Publisher = "CN=Soyuz Contributors",
    [string]$MakeAppx = "",
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
Push-Location $root
try {
    $version = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    $pkgVersion = (($version -split '[-+]')[0]) + ".0"
    if (-not $MakeAppx) {
        $MakeAppx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\makeappx.exe" -ErrorAction SilentlyContinue |
            Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
    }
    $msix = Join-Path $OutDir "soyuz-$version-windows-x64.msix"
    Write-Host "MSIX $pkgVersion ($Publisher) -> $msix using $MakeAppx"
    if ($DryRun) { Write-Host "dry run: nothing built"; return }
    if (-not $MakeAppx -or -not (Test-Path $MakeAppx)) { throw "makeappx.exe not found (install the Windows 10/11 SDK)" }
    $exe = Join-Path $BinDir "soyuz.exe"
    if (-not (Test-Path $exe)) { throw "missing $exe (run cargo build --release -p soyuz-app first)" }

    $stage = Join-Path $OutDir "msix-stage"
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force (Join-Path $stage "Assets") | Out-Null
    Copy-Item $exe, README.md, LICENSE, THIRD-PARTY-NOTICES.md $stage
    (Get-Content installer/msix/AppxManifest.xml -Raw) -replace '@VERSION@', $pkgVersion -replace '@PUBLISHER@', [Security.SecurityElement]::Escape($Publisher) |
        Set-Content -Encoding utf8 (Join-Path $stage "AppxManifest.xml")

    Add-Type -AssemblyName System.Drawing
    $src = [System.Drawing.Image]::FromFile((Resolve-Path "crates/soyuz-app/assets/icon.png").Path)
    try {
        foreach ($l in @(@("StoreLogo", 50, 50), @("Square44x44Logo", 44, 44), @("Square150x150Logo", 150, 150), @("Wide310x150Logo", 310, 150))) {
            $bmp = New-Object System.Drawing.Bitmap $l[1], $l[2]
            $g = [System.Drawing.Graphics]::FromImage($bmp)
            $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $side = [Math]::Min($l[1], $l[2])
            $g.DrawImage($src, [int](($l[1] - $side) / 2), [int](($l[2] - $side) / 2), $side, $side)
            $g.Dispose()
            $bmp.Save((Join-Path $stage "Assets/$($l[0]).png"), [System.Drawing.Imaging.ImageFormat]::Png)
            $bmp.Dispose()
        }
    } finally { $src.Dispose() }

    & $MakeAppx pack /d $stage /p $msix /o
    if ($LASTEXITCODE -ne 0) { throw "makeappx failed ($LASTEXITCODE)" }
    Remove-Item -Recurse -Force $stage
    Write-Host ("{0}: {1:N1} MiB (unsigned)" -f $msix, ((Get-Item $msix).Length / 1MB))
}
finally {
    Pop-Location
}
