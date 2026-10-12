<#
.SYNOPSIS
    Build the Soyuz MSI (WiX Toolset v5) from an already-built soyuz.exe and soyuz-tui.exe.

.DESCRIPTION
    Produces dist/soyuz-<version>-windows-<arch>.msi from installer/windows/soyuz.wxs.

    Needs the WiX .NET tool, pinned to v5 (WiX v6+ requires accepting the Open Source
    Maintenance Fee EULA; v5 is plain MS-RL):
        dotnet tool install --global wix --version 5.0.2
    With -UI the feature-tree dialogs are added (WixToolset.UI.wixext, fetched into the
    WiX extension cache on first use). Without it the MSI uses Windows Installer's basic UI,
    which is all `msiexec /qn` and the in-app updater need.

    The MSI version is the numeric part of the workspace version (MSI versions cannot carry
    a pre-release tag): 0.3.0-rc.1 -> 0.3.0.

.EXAMPLE
    ./installer/windows/build-msi.ps1                       # after cargo build --release
    ./installer/windows/build-msi.ps1 -UI -Wix C:\tools\wix.exe
    ./installer/windows/build-msi.ps1 -DryRun
#>
[CmdletBinding()]
param(
    [string]$BinDir = "target/release",
    [string]$OutDir = "dist",
    [string]$Wix = "",
    [string]$Arch = "x64",
    [switch]$UI,
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
Push-Location $root
try {
    $version = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    $msiVersion = ($version -split '[-+]')[0]
    if ($msiVersion -notmatch '^\d+\.\d+\.\d+$') { throw "cannot derive an MSI version from '$version'" }

    if (-not $Wix) {
        $cmd = Get-Command wix -ErrorAction SilentlyContinue
        $Wix = if ($cmd) { $cmd.Source } else { "wix" }
    }
    $exes = @((Join-Path $BinDir "soyuz.exe"), (Join-Path $BinDir "soyuz-tui.exe"))
    $wxs = "installer/windows/soyuz.wxs"
    New-Item -ItemType Directory -Force $OutDir | Out-Null
    $msi = Join-Path $OutDir "soyuz-$version-windows-$Arch.msi"
    $obj = Join-Path $OutDir "msi-obj"

    $wixArgs = @(
        "build", $wxs, "-nologo", "-arch", $Arch,
        "-d", "Version=$msiVersion",
        "-d", "BinDir=$((Resolve-Path $BinDir -ErrorAction SilentlyContinue) ?? $BinDir)",
        "-d", "RootDir=$root",
        "-intermediatefolder", $obj,
        "-o", $msi
    )
    if ($UI) {
        $rtf = Join-Path $obj "license.rtf"
        $wixArgs += @("-ext", "WixToolset.UI.wixext", "-d", "UseUI=1", "-d", "LicenseRtf=$rtf")
    }

    Write-Host "MSI $msiVersion -> $msi"
    Write-Host "files: $($exes -join ', ') + docs from $wxs"
    Write-Host "$Wix $($wixArgs -join ' ')"
    if ($DryRun) {
        Write-Host "dry run: nothing built"
        return
    }
    foreach ($exe in $exes) {
        if (-not (Test-Path $exe)) { throw "missing $exe (run cargo build --release -p soyuz-app -p soyuz-tui first)" }
    }

    if ($UI) {
        New-Item -ItemType Directory -Force $obj | Out-Null
        # The license page shows LICENSE (MIT); non-ASCII characters become RTF \uN? escapes.
        $text = (Get-Content LICENSE -Raw -Encoding utf8) -replace '\\', '\\\\' -replace '\{', '\{' -replace '\}', '\}'
        $text = -join ($text.ToCharArray() | ForEach-Object { if ([int]$_ -gt 127) { "\u$([int]$_)?" } else { "$_" } })
        $body = ($text -split "`r?`n") -join "\par`r`n"
        Set-Content -Encoding ascii $rtf "{\rtf1\ansi\deff0{\fonttbl{\f0 Segoe UI;}}\fs18 $body}"
        & $Wix extension add -g WixToolset.UI.wixext/5.0.2
        if ($LASTEXITCODE -ne 0) { throw "wix extension add failed ($LASTEXITCODE)" }
    }

    & $Wix @wixArgs
    if ($LASTEXITCODE -ne 0) { throw "wix build failed ($LASTEXITCODE)" }
    Remove-Item -Recurse -Force $obj -ErrorAction SilentlyContinue
    Get-ChildItem $OutDir -Filter "*.wixpdb" | Remove-Item -Force -ErrorAction SilentlyContinue
    Write-Host ("{0}: {1:N1} MiB" -f $msi, ((Get-Item $msi).Length / 1MB))
}
finally {
    Pop-Location
}
