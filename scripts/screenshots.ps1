<#
.SYNOPSIS
  Regenerates the screenshots in site/assets/shots/ from the release build.

.DESCRIPTION
  1. Builds `soyuz` in release mode (skip with -SkipBuild).
  2. Creates the privacy-safe demo tree with scripts/demo-fixture.ps1 (default E:\SoyuzDemo
     or <repo drive>:\SoyuzDemo), so no personal paths appear in the pictures.
  3. Runs the app in portable mode (an empty `portable` file next to the exe, settings in
     `portable-data/`) several times. Each run gets a fresh config.toml and session.toml
     (tabs, favorites pointing into the demo tree) and a SOYUZ_SCRIPT that navigates,
     switches views / panels / themes / language and saves window screenshots with `shot:`.
     The app captures its own window; no mouse or keyboard input is sent to the desktop.
  4. Downscales every PNG to at most -Width pixels and writes it to site/assets/shots/.
  5. Removes the portable marker, portable-data/, the raw captures and the demo tree.

.PARAMETER Width
  Maximum width of the committed PNGs (default 1600).

.PARAMETER Only
  Run only the sessions whose names match (main, activity, treemap).

.EXAMPLE
  ./scripts/screenshots.ps1
  ./scripts/screenshots.ps1 -SkipBuild -Only main -KeepDemo
#>
[CmdletBinding()]
param(
    [string]$Exe,
    [string]$Demo,
    [string]$OutDir,
    [int]$Width = 1600,
    [string[]]$Only = @(),
    [switch]$SkipBuild,
    [switch]$KeepDemo,
    [int]$TimeoutSec = 150
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$repo = Split-Path -Parent $PSScriptRoot
if (-not $Exe) { $Exe = Join-Path $repo 'target\release\soyuz.exe' }
if (-not $Demo) { $Demo = Join-Path ((Split-Path -Qualifier $repo) + '\') 'SoyuzDemo' }
if (-not $OutDir) { $OutDir = Join-Path $repo 'site\assets\shots' }

if (-not $SkipBuild) {
    Push-Location $repo
    try { cargo build --release -p soyuz-app; if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' } }
    finally { Pop-Location }
}
if (-not (Test-Path -LiteralPath $Exe)) { throw "no binary at $Exe (build it or pass -Exe)" }

$exeDir = Split-Path -Parent $Exe
$marker = Join-Path $exeDir 'portable'
$data = Join-Path $exeDir 'portable-data'
if ((Test-Path -LiteralPath $marker) -or (Test-Path -LiteralPath $data)) {
    throw "$exeDir already has a portable marker or portable-data/; move them away first (they may hold real settings)."
}

& (Join-Path $PSScriptRoot 'demo-fixture.ps1') -Root $Demo
$raw = Join-Path ([System.IO.Path]::GetTempPath()) ("soyuz-shots-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $raw | Out-Null
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

function Q([string]$s) { "'" + $s + "'" }   # TOML literal string (no escapes needed for paths)
function D([string]$rel) { if ($rel) { Join-Path $Demo $rel } else { $Demo } }

function Write-Config([string]$extra = '') {
    $cfg = @"
schema = 2

[general]
start_location = $(Q $Demo)
language = "en-US"

[appearance]
theme = "dark"

[behaviour]
restore_session = true

[jobs]
auto_open_activity = true

[sidebar]
# No devices / cloud sections: they would list this machine's drives (SOYUZ_DEMO_MASK also
# genericizes volume labels and sizes wherever a volume still shows, e.g. the breadcrumb).
mode = "places"
sections = ["favorites", "portal", "tags", "smart", "trash"]

[[sidebar.portal]]
path = $(Q (D 'Projects\orbit-tracker'))

[[sidebar.portal]]
path = $(Q (D 'Photos\2026 Summer trip'))
name = "Summer trip"

[[sidebar.portal]]
path = $(Q (D 'Documents\Reports'))

[terminal]
shell = "cmd.exe"
args = ["/k", "dir /b"]

[ai]
enabled = true
$extra
"@
    [System.IO.File]::WriteAllText((Join-Path $data 'config.toml'), $cfg, [System.Text.UTF8Encoding]::new($false))
}

# Tabs: each entry is one tab, an array of 1 (single) or 2 (dual pane) demo-relative folders.
function Write-Session([object[]]$tabs, [int]$active = 0) {
    $sb = [System.Text.StringBuilder]::new()
    [void]$sb.AppendLine("version = 1").AppendLine("active_tab = $active").AppendLine("welcome_seen = true")
    [void]$sb.AppendLine("show_hidden = false").AppendLine("theme = `"dark`"")
    $favs = @('Documents', 'Photos', 'Projects', 'Music', 'Downloads') | ForEach-Object { "{ name = `"$_`", path = $(Q (D $_)) }" }
    [void]$sb.AppendLine("favorites = [" + ($favs -join ', ') + "]")
    foreach ($t in $tabs) {
        $panes = @($t)
        [void]$sb.AppendLine().AppendLine('[[tabs]]')
        [void]$sb.AppendLine("dual = $(if ($panes.Count -gt 1) { 'true' } else { 'false' })").AppendLine('focused = 0').AppendLine('split = 0.5')
        foreach ($p in $panes) {
            [void]$sb.AppendLine('[[tabs.panes]]').AppendLine("location = $(Q (D $p))")
        }
    }
    [System.IO.File]::WriteAllText((Join-Path $data 'session.toml'), $sb.ToString(), [System.Text.UTF8Encoding]::new($false))
}

function Invoke-Session([string]$name, [string[]]$steps, [hashtable]$envs = @{}) {
    if ($Only.Count -gt 0 -and -not ($Only -contains $name)) { return }
    Write-Host "== session $name"
    $script = ($steps | ForEach-Object { $_ -replace '\{raw\}', $raw }) -join ';'
    $psi = [System.Diagnostics.ProcessStartInfo]::new($Exe)
    $psi.UseShellExecute = $false
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardOutput = $true
    $psi.WorkingDirectory = $exeDir
    $psi.Environment['SOYUZ_SCRIPT'] = $script
    $psi.Environment["RUST_LOG"] = "info,wgpu_core=warn,wgpu_hal=warn,naga=warn"
    $psi.Environment["SOYUZ_DEMO_MASK"] = "1"
    foreach ($k in $envs.Keys) { $psi.Environment[$k] = $envs[$k] }
    $p = [System.Diagnostics.Process]::Start($psi)
    $errTask = $p.StandardError.ReadToEndAsync()
    $outTask = $p.StandardOutput.ReadToEndAsync()
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        Write-Warning "session $name timed out; killing it"
        $p.Kill($true)
        $p.WaitForExit()
    }
    $log = $errTask.Result + $outTask.Result
    [System.IO.File]::WriteAllText((Join-Path $raw "$name.log"), $log)
    $saved = ([regex]::Matches($log, 'SOYUZ_SCRIPT: saved')).Count
    $failed = [regex]::Matches($log, '(SOYUZ_SCRIPT[^\r\n]*(cannot|unknown|FAILED)[^\r\n]*)') | ForEach-Object { $_.Value }
    Write-Host "   saved $saved screenshot(s)"
    foreach ($f in $failed) { Write-Warning "   $f" }
}

function Shot([string]$file) { "wait:300"; "shot:{raw}\$file" }

New-Item -ItemType Directory -Path $data | Out-Null
New-Item -ItemType File -Path $marker | Out-Null
try {
    # ---- Main tour: the screenshots used by the website and the README.
    Write-Config
    Write-Session @(
        , @('Photos')
        , @('Documents\Reports', 'Archive\Reports')
        , @('Projects\orbit-tracker')
        , @('Music')
        , @('Documents')
    )
    Invoke-Session 'main' (@(
            'size:1600x1000', 'wait:2500'
            'select:Sunset over the bay.jpg', 'toggle_preview_pane', 'wait:2000') + (Shot 'hero.png') + @(
            'toggle_preview_pane'
            'select:Northern lights.jpg', 'quick_look', 'wait:1500') + (Shot 'quick-look.png') + @(
            'keys:{Escape}', 'wait:400'
            'select_tab:2', 'wait:1500') + (Shot 'dual-pane.png') + @(
            'select_tab:3', 'wait:1200', 'select:README.md', 'toggle_terminal', 'wait:3000') + (Shot 'terminal.png') + @(
            'toggle_terminal', 'focus_list', 'wait:300'
            'select_tab:1', 'wait:500', 'command_palette', 'wait:300', 'keys:copy', 'wait:800') + (Shot 'palette.png') + @(
            # The first Esc clears the query, the second closes the palette.
            'keys:{Escape}', 'wait:200', 'keys:{Escape}', 'wait:300', 'expect:palette=closed'
            'view_grid', 'theme:light', 'wait:2500') + (Shot 'light-grid.png') + @(
            'view_details', 'theme:dark', 'quit')) @{}

    # ---- A copy job in flight (throttled) with the activity panel.
    Write-Config
    Write-Session @(, @('Downloads', 'Archive'))
    Invoke-Session 'activity' (@(
            'size:1600x1000', 'wait:2500', 'select:Telemetry dump.dat', 'copy_to_other_pane', 'wait:4500') + (Shot 'jobs.png') + @(
            'jobs_cancel_all', 'wait:1500', 'quit')) @{ SOYUZ_THROTTLE_MBPS = '6' }

    # ---- Tools opened at startup (without the big job file, which would fill the treemap).
    Remove-Item -LiteralPath (D 'Downloads\Telemetry dump.dat') -Force -ErrorAction SilentlyContinue
    Write-Config
    Write-Session @(, @(''))
    Invoke-Session 'treemap' (@('size:1600x1000', 'wait:4000') + (Shot 'treemap.png') + @('quit')) @{ SOYUZ_OPEN_TOOL = "treemap=$Demo" }

    # ---- Downscale into the output folder (replacing the previous screenshots).
    if ((Get-ChildItem -LiteralPath $raw -Filter *.png).Count -gt 0 -and $Only.Count -eq 0) {
        Get-ChildItem -LiteralPath $OutDir -Filter *.png | Remove-Item -Force
    }
    $total = 0
    Get-ChildItem -LiteralPath $raw -Filter *.png | Sort-Object Name | ForEach-Object {
        $src = [System.Drawing.Image]::FromFile($_.FullName)
        try {
            $w = [Math]::Min($Width, $src.Width)
            $h = [int][Math]::Round($src.Height * $w / $src.Width)
            $bmp = [System.Drawing.Bitmap]::new($w, $h, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
            $g = [System.Drawing.Graphics]::FromImage($bmp)
            $g.InterpolationMode = 'HighQualityBicubic'
            $g.PixelOffsetMode = 'HighQuality'
            $g.SmoothingMode = 'HighQuality'
            $g.DrawImage($src, 0, 0, $w, $h)
            $g.Dispose()
            $dst = Join-Path $OutDir $_.Name
            $bmp.Save($dst, [System.Drawing.Imaging.ImageFormat]::Png)
            $bmp.Dispose()
            $len = (Get-Item -LiteralPath $dst).Length
            $total += $len
            Write-Host ("   {0,-28} {1,5}x{2,-5} {3,7:N0} KB" -f $_.Name, $w, $h, ($len / 1KB))
        } finally { $src.Dispose() }
    }
    Write-Host ("Screenshots: {0} files, {1:N1} MB in {2}" -f (Get-ChildItem -LiteralPath $OutDir -Filter *.png).Count, ($total / 1MB), $OutDir)
}
finally {
    Remove-Item -LiteralPath $marker -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $data -Recurse -Force -ErrorAction SilentlyContinue
    if ($env:SOYUZ_SHOTS_KEEP_RAW -ne '1') { Remove-Item -LiteralPath $raw -Recurse -Force -ErrorAction SilentlyContinue }
    else { Write-Host "raw captures and logs kept in $raw" }
    if (-not $KeepDemo) { Remove-Item -LiteralPath $Demo -Recurse -Force -ErrorAction SilentlyContinue }
}
