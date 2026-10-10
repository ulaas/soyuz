<#
.SYNOPSIS
    Build a release of Soyuz and package it for distribution.

.DESCRIPTION
    Runs `cargo build --release -p soyuz-app`, then creates
    dist/soyuz-<version>-<os>-<arch>.zip (Windows) or .tar.gz (Linux / macOS, keeps the
    executable bit) containing the executable, README, both licenses,
    the example config and the user guide. Pass -Portable to include an empty `portable`
    marker so the unzipped folder keeps its settings next to the exe (portable-data/).

    -Msi also builds the Windows installer (installer/windows/build-msi.ps1, needs WiX v5).

    Every artifact gets a `<file>.sha256` next to it, and dist/SHA256SUMS-<os>.txt lists
    them all in `sha256sum` format (what the in-app update check verifies against).

    -DryRun prints the plan (version, file names, staged files) without building or
    writing anything.

.EXAMPLE
    ./scripts/package.ps1
    ./scripts/package.ps1 -Portable -Features glow
    ./scripts/package.ps1 -Msi -SkipBuild
    ./scripts/package.ps1 -DryRun -Portable -Msi
#>
[CmdletBinding()]
param(
    [switch]$Portable,
    [string]$Features = "",
    [switch]$NoDefaultFeatures,
    [string]$OutDir = "dist",
    [switch]$Msi,
    [string]$Wix = "",
    [switch]$SkipBuild,
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $version = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    $isWin = $IsWindows -or ($env:OS -eq "Windows_NT")
    $os = if ($isWin) { "windows" } elseif ($IsMacOS) { "macos" } else { "linux" }
    $arch = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
        "Arm64" { "arm64" }
        default { "x64" }
    }
    $exe = if ($isWin) { "soyuz.exe" } else { "soyuz" }
    $bin = Join-Path "target/release" $exe
    $suffix = if ($Portable) { "-portable" } else { "" }
    $name = "soyuz-$version-$os-$arch$suffix"
    $ext = if ($isWin) { "zip" } else { "tar.gz" }
    $zip = Join-Path $OutDir "$name.$ext"
    $sums = Join-Path $OutDir "SHA256SUMS-$os.txt"

    # What goes into the zip: (source, path inside the zip).
    $files = @(
        @($bin, $exe),
        @("README.md", "README.md"),
        @("LICENSE-MIT", "LICENSE-MIT"),
        @("LICENSE-APACHE", "LICENSE-APACHE"),
        @("docs/USER_GUIDE.md", "docs/USER_GUIDE.md"),
        @("docs/config.example.toml", "docs/config.example.toml")
    )

    $cargoArgs = @("build", "--release", "-p", "soyuz-app", "--locked")
    if ($Features) { $cargoArgs += @("--features", $Features) }
    if ($NoDefaultFeatures) { $cargoArgs += "--no-default-features" }

    if ($DryRun) {
        Write-Host "version:  $version"
        Write-Host "build:    $(if ($SkipBuild) { '(skipped)' } else { "cargo $($cargoArgs -join ' ')" })"
        Write-Host "zip:      $zip"
        foreach ($f in $files) { Write-Host ("  {0,-28} <- {1}" -f $f[1], $f[0]) }
        if ($Portable) { Write-Host ("  {0,-28} (empty marker: settings in portable-data/)" -f "portable") }
        if ($Msi) {
            Write-Host "msi:"
            & (Join-Path $PSScriptRoot "../installer/windows/build-msi.ps1") -OutDir $OutDir -Wix $Wix -DryRun
        }
        Write-Host "checksums: <artifact>.sha256 and $sums"
        Write-Host "dry run: nothing built or written"
        return
    }

    if (-not $SkipBuild) {
        Write-Host "cargo $($cargoArgs -join ' ')"
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
    }
    if (-not (Test-Path $bin)) { throw "missing $bin" }

    $stage = Join-Path $OutDir $name
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force $stage | Out-Null
    foreach ($f in $files) {
        $dest = Join-Path $stage $f[1]
        New-Item -ItemType Directory -Force (Split-Path -Parent $dest) | Out-Null
        Copy-Item $f[0] $dest
    }
    if ($Portable) { New-Item -ItemType File -Force (Join-Path $stage "portable") | Out-Null }

    if (Test-Path $zip) { Remove-Item -Force $zip }
    if ($isWin) {
        Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip
    } else {
        & chmod +x (Join-Path $stage $exe)
        & tar -czf $zip -C $OutDir $name
        if ($LASTEXITCODE -ne 0) { throw "tar failed ($LASTEXITCODE)" }
    }
    Remove-Item -Recurse -Force $stage
    $artifacts = @($zip)

    if ($Msi) {
        if (-not $isWin) { throw "-Msi is Windows-only" }
        & (Join-Path $PSScriptRoot "../installer/windows/build-msi.ps1") -OutDir $OutDir -Wix $Wix
        $artifacts += (Join-Path $OutDir "soyuz-$version-windows-$arch.msi")
    }

    # Checksums: one file per artifact plus a combined sha256sum-style list (merged with
    # entries from earlier runs for other artifacts, e.g. portable + installer zips).
    $lines = @{}
    if (Test-Path $sums) {
        foreach ($l in Get-Content $sums) {
            if ($l -match '^([0-9a-f]{64})\s+\*?(.+)$' -and (Test-Path (Join-Path $OutDir $Matches[2]))) { $lines[$Matches[2]] = $Matches[1] }
        }
    }
    foreach ($a in $artifacts) {
        $hash = (Get-FileHash $a -Algorithm SHA256).Hash.ToLower()
        $leaf = Split-Path -Leaf $a
        Set-Content -Encoding ascii -NoNewline -LiteralPath "$a.sha256" -Value "$hash  $leaf`n"
        $lines[$leaf] = $hash
    }
    ($lines.GetEnumerator() | Sort-Object Name | ForEach-Object { "{0}  {1}" -f $_.Value, $_.Name }) |
        Set-Content -Encoding ascii $sums

    $size = (Get-Item $bin).Length / 1MB
    foreach ($a in $artifacts) {
        Write-Host ("{0}: {1:N1} MiB" -f $a, ((Get-Item $a).Length / 1MB))
    }
    Write-Host ("executable {0:N1} MiB; checksums in {1}" -f $size, $sums)
}
finally {
    Pop-Location
}
