<#
.SYNOPSIS
  Trims stale build outputs from target/: cargo never deletes old artifacts, so every
  edit, feature or toolchain change leaves another copy of each binary and its PDB.

.DESCRIPTION
  Works per build unit, so cargo never sees half a unit: a unit is stale when nothing in
  its fingerprint folder (<profile>/.fingerprint/<name>-<hash>) was written for -Days days
  (default 3) or since -Before. Then its fingerprint, its build-script folder
  (build/<name>-<hash>) and its deps files (deps/<name>-<hash>.*, lib<name>-<hash>.*) go
  together, and cargo rebuilds the unit if it is still wanted. Incremental sessions
  older than the cutoff go too. Covers target/debug, target/release and
  target/<triple>/<profile>. -All runs `cargo clean`.

.EXAMPLE
  ./scripts/trim-target.ps1                 # units untouched for 3 days
  ./scripts/trim-target.ps1 -Days 1
  ./scripts/trim-target.ps1 -Before (Get-Date).AddHours(-2)
  ./scripts/trim-target.ps1 -All
#>
param([int]$Days = 3, [datetime]$Before, [switch]$All)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$target = Join-Path $root 'target'
if (-not (Test-Path $target)) { Write-Host 'No target/ folder.'; return }

function SizeGB($path) {
    $sum = 0L
    foreach ($f in [IO.Directory]::EnumerateFiles($path, '*', [IO.SearchOption]::AllDirectories)) {
        $sum += ([IO.FileInfo]::new($f)).Length
    }
    [math]::Round($sum / 1GB, 1)
}

$sizeBefore = SizeGB $target
if ($All) {
    Push-Location $root
    try { cargo clean } finally { Pop-Location }
} else {
    $cutoff = if ($PSBoundParameters.ContainsKey('Before')) { $Before } else { (Get-Date).AddDays(-$Days) }
    $units = 0
    # Every profile folder: one holding a .fingerprint folder (depth <= 3 under target/).
    $profiles = [IO.Directory]::EnumerateDirectories($target, '.fingerprint', [IO.SearchOption]::AllDirectories) |
        ForEach-Object { [IO.Path]::GetDirectoryName($_) }
    foreach ($p in $profiles) {
        # deps files by unit (`<crate>-<hash>`, `lib` prefix and extensions dropped), listed once.
        $byUnit = @{}
        $deps = Join-Path $p 'deps'
        if ([IO.Directory]::Exists($deps)) {
            foreach ($f in [IO.Directory]::EnumerateFiles($deps)) {
                $m = [regex]::Match([IO.Path]::GetFileName($f), '^(?:lib)?(.+?-[0-9a-f]{16})(?:\.|$)')
                if (-not $m.Success) { continue }
                $k = $m.Groups[1].Value
                if (-not $byUnit.ContainsKey($k)) { $byUnit[$k] = [Collections.Generic.List[string]]::new() }
                $byUnit[$k].Add($f)
            }
        }
        foreach ($fp in [IO.Directory]::GetDirectories((Join-Path $p '.fingerprint'))) {
            $fresh = $false
            foreach ($f in [IO.Directory]::EnumerateFiles($fp)) {
                if ([IO.File]::GetLastWriteTime($f) -ge $cutoff) { $fresh = $true; break }
            }
            if ($fresh) { continue }
            $unit = [IO.Path]::GetFileName($fp)  # <name>-<hash>
            [IO.Directory]::Delete($fp, $true)
            $build = Join-Path (Join-Path $p 'build') $unit
            if ([IO.Directory]::Exists($build)) { [IO.Directory]::Delete($build, $true) }
            $crate = $unit -replace '-', '_' -replace '_([0-9a-f]{16})$', '-$1'
            foreach ($k in @($crate, "lib$crate")) {
                if ($byUnit.ContainsKey($k)) {
                    foreach ($f in $byUnit[$k]) { try { [IO.File]::Delete($f) } catch {} }
                }
            }
            $units++
        }
        $inc = Join-Path $p 'incremental'
        if ([IO.Directory]::Exists($inc)) {
            foreach ($crateDir in [IO.Directory]::GetDirectories($inc)) {
                foreach ($session in [IO.Directory]::GetDirectories($crateDir)) {
                    if ([IO.Directory]::GetLastWriteTime($session) -lt $cutoff) {
                        try { [IO.Directory]::Delete($session, $true) } catch {}
                    }
                }
            }
        }
    }
    Write-Host "Removed $units stale build units (older than $cutoff)."
}
$sizeAfter = SizeGB $target
Write-Host ("target/: {0} GB -> {1} GB (freed {2} GB)" -f $sizeBefore, $sizeAfter, [math]::Round($sizeBefore - $sizeAfter, 1))
