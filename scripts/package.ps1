<#
.SYNOPSIS
    Build a release of Soyuz and zip it for distribution.

.DESCRIPTION
    Runs `cargo build --release -p soyuz-app`, then creates
    dist/soyuz-<version>-<os>-<arch>.zip containing the executable, README,
    licenses, the example config and the user guide. Pass -Portable to include an
    empty `portable` marker so the unzipped folder keeps its settings next to the exe.

.EXAMPLE
    ./scripts/package.ps1
    ./scripts/package.ps1 -Portable -Features glow
#>
[CmdletBinding()]
param(
    [switch]$Portable,
    [string]$Features = "",
    [switch]$NoDefaultFeatures,
    [string]$OutDir = "dist"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $cargoArgs = @("build", "--release", "-p", "soyuz-app", "--locked")
    if ($Features) { $cargoArgs += @("--features", $Features) }
    if ($NoDefaultFeatures) { $cargoArgs += "--no-default-features" }
    Write-Host "cargo $($cargoArgs -join ' ')"
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }

    $version = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    $isWin = $IsWindows -or ($env:OS -eq "Windows_NT")
    $os = if ($isWin) { "windows" } elseif ($IsMacOS) { "macos" } else { "linux" }
    $arch = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
        "Arm64" { "arm64" }
        default { "x64" }
    }
    $exe = if ($isWin) { "soyuz.exe" } else { "soyuz" }
    $bin = Join-Path "target/release" $exe
    if (-not (Test-Path $bin)) { throw "missing $bin" }

    $name = "soyuz-$version-$os-$arch"
    $stage = Join-Path $OutDir $name
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force $stage | Out-Null
    Copy-Item $bin $stage
    Copy-Item README.md, LICENSE-MIT, LICENSE-APACHE $stage
    New-Item -ItemType Directory -Force (Join-Path $stage "docs") | Out-Null
    Copy-Item docs/USER_GUIDE.md, docs/config.example.toml (Join-Path $stage "docs")
    if ($Portable) { New-Item -ItemType File -Force (Join-Path $stage "portable") | Out-Null }

    $zip = Join-Path $OutDir "$name.zip"
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip
    Remove-Item -Recurse -Force $stage

    $size = (Get-Item $bin).Length / 1MB
    $zsize = (Get-Item $zip).Length / 1MB
    Write-Host ("{0}: executable {1:N1} MiB, zip {2:N1} MiB" -f $zip, $size, $zsize)
}
finally {
    Pop-Location
}
