<#
.SYNOPSIS
  Regenerates the terminal UI screenshots (SVG) from the release `soyuz-tui` build.

.DESCRIPTION
  1. Builds `soyuz-tui` in release mode (skip with -SkipBuild).
  2. Creates the privacy-safe demo tree with scripts/demo-fixture.ps1 (scripts/demo-fixture.sh
     off Windows; default <repo drive>:\SoyuzDemo or /tmp/SoyuzDemo, short paths without a
     user name, because the breadcrumbs show them).
  3. Copies the binary into a temporary sandbox next to an empty `portable` marker, with a
     fresh portable-data/config.toml (Unicode icons; AI, updates, the CLI server and the
     index off) and session-tui.toml (tabs in the demo tree). Your own settings are never
     read or written.
  4. Runs it headless (SOYUZ_TUI_HEADLESS=<Size>): no window, no console, no terminal and no
     input; SOYUZ_SCRIPT navigates, opens the palette, a dialog, the jobs panel, the preview,
     the settings and screen-reader mode, and saves each frame with `shot:<file>.svg` (plus a
     `.txt` of the same frame for the checks).
  5. Writes tui-*.svg to site/assets/shots/ (or the folders given with -OutDir).
  6. Removes the sandbox (binary copy, portable files, log) and the demo tree.

.PARAMETER OutDir
  Folders that receive the SVGs (default site/assets/shots; previous tui-*.svg there are
  replaced).

.PARAMETER Size
  Terminal size in cells, <columns>x<rows> (default 140x36).

.PARAMETER Only
  Save only the shots whose names match (browse, palette, conflict, preview, jobs, settings,
  screen-reader); the session always runs in full.

.EXAMPLE
  ./scripts/tui-screenshots.ps1
  ./scripts/tui-screenshots.ps1 -SkipBuild -KeepDemo -Only browse,palette
  $env:SOYUZ_SHOTS_KEEP_RAW = '1'   # keep the .txt frames and the log in the sandbox
#>
[CmdletBinding()]
param(
    [string]$Exe,
    [string]$Demo,
    [string[]]$OutDir,
    [string]$Size = '140x36',
    [string[]]$Only = @(),
    [switch]$SkipBuild,
    [switch]$KeepDemo,
    [int]$TimeoutSec = 120
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$onWindows = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)
$exeName = if ($onWindows) { 'soyuz-tui.exe' } else { 'soyuz-tui' }
if (-not $Exe) { $Exe = Join-Path $repo "target/release/$exeName" }
if (-not $Demo) {
    $Demo = if ($onWindows) { Join-Path ((Split-Path -Qualifier $repo) + '\') 'SoyuzDemo' } else { '/tmp/SoyuzDemo' }
}
if (-not $OutDir) { $OutDir = @(Join-Path $repo 'site/assets/shots') }

if (-not $SkipBuild) {
    Push-Location $repo
    try { cargo build --release -p soyuz-tui; if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' } }
    finally { Pop-Location }
}
if (-not (Test-Path -LiteralPath $Exe)) { throw "no binary at $Exe (build it or pass -Exe)" }

# A small big file: the jobs shot only needs a copy in flight (throttled below).
if ($onWindows) { & (Join-Path $PSScriptRoot 'demo-fixture.ps1') -Root $Demo -BigFileMB 48 }
else { bash (Join-Path $PSScriptRoot 'demo-fixture.sh') $Demo 48; if ($LASTEXITCODE -ne 0) { throw 'demo-fixture.sh failed' } }

$sandbox = Join-Path ([System.IO.Path]::GetTempPath()) ("soyuz-tui-shots-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
$data = Join-Path $sandbox 'portable-data'
$raw = Join-Path $sandbox 'shots'
New-Item -ItemType Directory -Path $data, $raw | Out-Null

function Q([string]$s) { "'" + $s + "'" }   # TOML literal string (no escapes needed for paths)
function D([string]$rel) { if ($rel) { Join-Path $Demo $rel } else { $Demo } }
function Write-Utf8([string]$path, [string]$text) {
    [System.IO.File]::WriteAllText($path, $text, [System.Text.UTF8Encoding]::new($false))
}

try {
    $sandboxExe = Join-Path $sandbox $exeName
    Copy-Item -LiteralPath $Exe -Destination $sandboxExe
    New-Item -ItemType File -Path (Join-Path $sandbox 'portable') | Out-Null

    Write-Utf8 (Join-Path $data 'config.toml') @"
schema = 2

[general]
start_location = $(Q $Demo)
language = "en-US"

[appearance]
theme = "dark"

[behaviour]
restore_session = true

[sidebar]
# No devices / cloud sections: they would list this machine's drives.
mode = "places"
sections = ["favorites", "portal", "smart", "trash"]

[[sidebar.portal]]
path = $(Q (D 'Projects\orbit-tracker'))

[[sidebar.portal]]
path = $(Q (D 'Photos\2026 Summer trip'))
name = "Summer trip"

[jobs]
auto_open_activity = true

[tui]
# Nerd Font glyphs show as boxes on GitHub; Unicode icons render everywhere.
icons = "unicode"

[updates]
weekly_check = false

[cli]
server_enabled = false

[search.index]
enabled = false

[ai]
enabled = false
"@

    $favs = @('Documents', 'Photos', 'Projects', 'Music', 'Downloads') | ForEach-Object { "{ name = `"$_`", path = $(Q (D $_)) }" }
    $tabs = @(
        , @('Documents\Reports', 'Archive\Reports')
        , @('Projects\orbit-tracker')
        , @('Downloads', 'Archive')
    )
    $sb = [System.Text.StringBuilder]::new()
    [void]$sb.AppendLine('version = 1').AppendLine('active_tab = 0').AppendLine('welcome_seen = true')
    [void]$sb.AppendLine('show_hidden = false').AppendLine('theme = "dark"')
    [void]$sb.AppendLine('favorites = [' + ($favs -join ', ') + ']')
    foreach ($t in $tabs) {
        $panes = @($t)
        [void]$sb.AppendLine().AppendLine('[[tabs]]')
        [void]$sb.AppendLine("dual = $(if ($panes.Count -gt 1) { 'true' } else { 'false' })").AppendLine('focused = 0').AppendLine('split = 0.5')
        foreach ($p in $panes) { [void]$sb.AppendLine('[[tabs.panes]]').AppendLine("location = $(Q (D $p))") }
    }
    Write-Utf8 (Join-Path $data 'session-tui.toml') $sb.ToString()

    # Each shot saves the next frame as .svg and .txt; `expect:screen~` checks it shows what
    # the picture is about.
    function Shot([string]$name, [string]$shows) {
        "wait:400"; "expect:screen~$shows"; "shot:$raw/tui-$name.svg"; "shot:$raw/tui-$name.txt"
    }
    $steps = @(
        'wait_for:has=Budget 2026.csv', 'select:Q3 mission report.md') + (Shot 'browse' 'Q3 mission report.md') + @(
        'command_palette', 'wait:300', 'keys:copy') + (Shot 'palette' 'Copy') + @(
        # The first Esc clears the query, the second closes the palette.
        'keys:{Escape}', 'wait:200', 'keys:{Escape}', 'expect:palette=closed'
        # Archive\Reports holds a copy of Budget 2026.csv: the copy asks what to do (S skips).
        'select:Budget 2026.csv', 'copy_to_other_pane', 'wait_for:dialog=open') + (Shot 'conflict' 'Keep both') + @(
        'keys:s', 'wait_for:dialog=closed', 'wait:500'
        'select_tab:2', 'wait_for:has=README.md', 'select:README.md', 'toggle_preview_pane', 'wait:1500') + (Shot 'preview' 'orbit') + @(
        'toggle_preview_pane'
        'select_tab:3', 'wait_for:has=Telemetry dump.dat', 'select:Telemetry dump.dat', 'copy_to_other_pane', 'wait:3000') + (Shot 'jobs' 'Telemetry dump.dat') + @(
        'jobs_cancel_all', 'wait:1000', 'jobs_clear_finished', 'toggle_activity_panel', 'wait:500'
        'select_tab:1', 'wait:300', 'keys:{Ctrl+,}', 'wait_for:settings=open'
        # Ctrl+PageDown: the next section, seven times to *Terminal UI*.
        ('keys:' + ('{Ctrl+PageDown}' * 7))) + (Shot 'settings' 'screen_reader') + @(
        'keys:{Escape}', 'wait_for:settings=closed'
        # Through the setting rather than the palette action: no "mode on" toast over the
        # status sentence.
        'select:Q3 mission report.md', 'config:tui.screen_reader=true', 'wait:500') + (Shot 'screen-reader' 'Other pane') + @(
        'quit')

    Write-Host "== soyuz-tui headless $Size"
    $psi = [System.Diagnostics.ProcessStartInfo]::new($sandboxExe)
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardInput = $true
    $psi.WorkingDirectory = $sandbox
    $psi.Environment['SOYUZ_TUI_HEADLESS'] = $Size
    $psi.Environment['SOYUZ_SCRIPT'] = $steps -join ';'
    $psi.Environment['SOYUZ_DEMO_MASK'] = '1'
    $psi.Environment['SOYUZ_THROTTLE_MBPS'] = '4'
    $psi.Environment['RUST_LOG'] = 'info'
    [void]$psi.Environment.Remove('WT_SESSION')
    $p = [System.Diagnostics.Process]::Start($psi)
    $p.StandardInput.Close()
    $errTask = $p.StandardError.ReadToEndAsync()
    $outTask = $p.StandardOutput.ReadToEndAsync()
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        Write-Warning "soyuz-tui timed out; killing it"
        $p.Kill($true)
        $p.WaitForExit()
    }
    $logFile = Join-Path $data 'soyuz-tui.log'
    $log = $errTask.Result + $outTask.Result + $(if (Test-Path -LiteralPath $logFile) { Get-Content -Raw -LiteralPath $logFile } else { '' })
    $failed = [regex]::Matches($log, '(SOYUZ_SCRIPT[^\r\n]*(cannot|unknown|FAILED|not a terminal UI step)[^\r\n]*)') | ForEach-Object { $_.Value }
    foreach ($f in $failed) { Write-Warning "   $f" }
    if ($env:SOYUZ_SHOTS_KEEP_RAW -eq '1') { Write-Utf8 (Join-Path $raw 'soyuz-tui.log') $log }

    $shots = @(Get-ChildItem -LiteralPath $raw -Filter 'tui-*.svg' | Sort-Object Name |
        Where-Object { $Only.Count -eq 0 -or $Only -contains ($_.BaseName -replace '^tui-', '') })
    if ($shots.Count -eq 0) { throw "no screenshots were saved (exit code $($p.ExitCode)); set SOYUZ_SHOTS_KEEP_RAW=1 to keep the log" }
    foreach ($dir in $OutDir) {
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        if ($Only.Count -eq 0) { Get-ChildItem -LiteralPath $dir -Filter 'tui-*.svg' | Remove-Item -Force }
        foreach ($s in $shots) { Copy-Item -LiteralPath $s.FullName -Destination (Join-Path $dir $s.Name) -Force }
    }
    foreach ($s in $shots) { Write-Host ("   {0,-26} {1,7:N1} KB" -f $s.Name, ($s.Length / 1KB)) }
    Write-Host ("TUI screenshots: {0} files in {1}" -f $shots.Count, ($OutDir -join ', '))
    if ($failed.Count -gt 0 -or $p.ExitCode -ne 0) { Write-Warning "the script reported problems (exit code $($p.ExitCode)); check the shots" }
}
finally {
    if ($env:SOYUZ_SHOTS_KEEP_RAW -eq '1') {
        Remove-Item -LiteralPath (Join-Path $sandbox $exeName), (Join-Path $sandbox 'portable'), $data -Recurse -Force -ErrorAction SilentlyContinue
        Write-Host "raw shots and the log kept in $raw"
    }
    else { Remove-Item -LiteralPath $sandbox -Recurse -Force -ErrorAction SilentlyContinue }
    if (-not $KeepDemo) { Remove-Item -LiteralPath $Demo -Recurse -Force -ErrorAction SilentlyContinue }
}
