<#
.SYNOPSIS
  Automated accessibility audit: reads the UI Automation tree that Soyuz exposes through
  AccessKit and checks names, roles, selection state and progress values.

.DESCRIPTION
  This is the automated counterpart of a screen-reader pass (it reads exactly what NVDA /
  Narrator get from UI Automation), not a replacement for one: a human run with NVDA and
  Narrator remains on the checklist in docs/ACCESSIBILITY.md.

  1. Creates the demo tree (scripts/demo-fixture.ps1, default <drive>:\SoyuzDemo) and runs
     the release build in portable mode (an empty `portable` file next to the exe,
     settings in `portable-data/`, both removed afterwards).
  2. Drives the app with SOYUZ_SCRIPT only (no mouse or keyboard input is sent to the
     desktop). The script walks through several states and holds each one with an
     `until:<file>` step while this script inspects the window:
       main      details view of Documents\Reports, one file selected, folder tree, tabs
       palette   the command palette with a query
       settings  the settings window (General)
       appearance  the settings window (Appearance: dropdowns, sliders)
       confirm   the "delete permanently" confirmation (cancelled with Esc afterwards)
       video     the video player in Quick Look (Videos\Launch replay.mp4, played for a
                 moment, then paused; muted)
       mediatools  the media tools dialog (Convert) on the same video, cancelled with Esc
       progress  a throttled copy job with the activity panel
     The video and mediatools states need the FFmpeg libraries (SOYUZ_FFMPEG_DIR or next
     to the exe) and the demo video (demo-fixture.ps1 renders it with an ffmpeg program
     from SOYUZ_FFMPEG_DIR or the PATH); without them they are skipped with a warning.
     With SOYUZ_WINDOW_POS set (an off-screen run), dialogs are drawn inside the main window
     ([appearance] native_dialogs = false): a native dialog window that is never activated
     does not deliver its AccessKit tree. The content and roles are the same.
  3. In each state it reads the whole UIA subtree of the Soyuz window in one cached call
     (System.Windows.Automation, UIAutomationClient) and checks:
       G1 the window has a name
       G2 no keyboard-focusable element is unnamed
       G3 interactive controls (buttons, menu items, tabs, list / tree items, check boxes,
          edits, combo boxes, sliders, progress bars) have a name
       G4 list / tree / tab items expose the SelectionItem pattern (selection state)
     plus per-state checks (selected file row, tree items, tab selection, dialog roles and
     names, progress bar values).
  4. Prints the findings and, with -Report, writes them as Markdown.

  Exit code: the number of failed checks (0 = clean).

.PARAMETER Exe
  The soyuz.exe to test (default target\release\soyuz.exe).

.PARAMETER Report
  Write the findings as Markdown to this file (e.g. docs\ACCESSIBILITY_AUDIT.generated.md).

.EXAMPLE
  cargo build --release -p soyuz-app; ./scripts/uia-audit.ps1
  ./scripts/uia-audit.ps1 -Report $env:TEMP\uia.md -Only main,confirm
  $env:SOYUZ_FFMPEG_DIR = '<folder with the FFmpeg DLLs>'; ./scripts/uia-audit.ps1 -Only video,mediatools
#>
[CmdletBinding()]
param(
    [string]$Exe,
    [string]$Demo,
    [string]$Report,
    [string[]]$Only = @(),
    [int]$TimeoutSec = 180,
    [switch]$KeepDemo,
    # Write every element of every state as JSON lines to this file (debugging).
    [string]$Dump
)

$ErrorActionPreference = 'Stop'
$Only = @($Only | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$repo = Split-Path -Parent $PSScriptRoot
if (-not $Exe) { $Exe = Join-Path $repo 'target\release\soyuz.exe' }
if (-not $Demo) { $Demo = Join-Path ((Split-Path -Qualifier $repo) + '\') 'SoyuzDemo' }
if (-not (Test-Path -LiteralPath $Exe)) { throw "no binary at $Exe (cargo build --release -p soyuz-app)" }

$exeDir = Split-Path -Parent $Exe
$marker = Join-Path $exeDir 'portable'
$data = Join-Path $exeDir 'portable-data'
if ((Test-Path -LiteralPath $marker) -or (Test-Path -LiteralPath $data)) {
    throw "$exeDir already has a portable marker or portable-data/; move them away first (they may hold real settings)."
}

& (Join-Path $PSScriptRoot 'demo-fixture.ps1') -Root $Demo -BigFileMB 64
$work = Join-Path ([System.IO.Path]::GetTempPath()) ('soyuz-uia-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $work | Out-Null

function Q([string]$s) { "'" + $s + "'" }
function D([string]$rel) { if ($rel) { Join-Path $Demo $rel } else { $Demo } }

$AE = [System.Windows.Automation.AutomationElement]
$CT = [System.Windows.Automation.ControlType]
$NotSupported = [System.Windows.Automation.AutomationElement]::NotSupported
# Properties the managed client has no constants for, looked up by their UIA ids.
$AriaRoleProp = [System.Windows.Automation.AutomationProperty]::LookupById(30101)   # UIA_AriaRolePropertyId
$IsDialogProp = [System.Windows.Automation.AutomationProperty]::LookupById(30174)   # UIA_IsDialogPropertyId

# ---------------------------------------------------------------------------
# UIA snapshot
# ---------------------------------------------------------------------------

function Get-SoyuzWindow([int]$procId) {
    $cond = [System.Windows.Automation.PropertyCondition]::new($AE::ProcessIdProperty, $procId)
    $wins = [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)
    $best = $null; $area = 0
    foreach ($w in $wins) {
        $r = $w.Current.BoundingRectangle
        if ($r.Width * $r.Height -gt $area) { $best = $w; $area = $r.Width * $r.Height }
    }
    $best
}

function Get-Snapshot($win) {
    $cr = [System.Windows.Automation.CacheRequest]::new()
    $cr.TreeScope = [System.Windows.Automation.TreeScope]::Element
    foreach ($p in @($AE::NameProperty, $AE::ControlTypeProperty, $AE::IsKeyboardFocusableProperty, $AE::AutomationIdProperty,
            $AE::BoundingRectangleProperty, $AE::IsEnabledProperty, $AE::IsOffscreenProperty, $AE::LocalizedControlTypeProperty,
            $AE::IsSelectionItemPatternAvailableProperty, $AE::IsRangeValuePatternAvailableProperty, $AE::IsTogglePatternAvailableProperty,
            $AE::IsExpandCollapsePatternAvailableProperty, $AE::HelpTextProperty,
            [System.Windows.Automation.SelectionItemPattern]::IsSelectedProperty,
            [System.Windows.Automation.RangeValuePattern]::ValueProperty,
            [System.Windows.Automation.RangeValuePattern]::MinimumProperty,
            [System.Windows.Automation.RangeValuePattern]::MaximumProperty,
            [System.Windows.Automation.ValuePattern]::ValueProperty)) { $cr.Add($p) }
    foreach ($p in @($AriaRoleProp, $IsDialogProp)) { if ($p) { try { $cr.Add($p) } catch {} } }
    $cr.Push()
    try { $all = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) }
    finally { $cr.Pop() }
    $out = [System.Collections.Generic.List[object]]::new()
    foreach ($e in $all) {
        $get = { param($p) $v = $e.GetCachedPropertyValue($p, $true); if ($v -eq $NotSupported) { $null } else { $v } }
        $ct = & $get $AE::ControlTypeProperty
        $aria = $null; if ($AriaRoleProp) { try { $aria = & $get $AriaRoleProp } catch {} }
        $isDialog = $null; if ($IsDialogProp) { try { $isDialog = & $get $IsDialogProp } catch {} }
        $typeName = '?'
        if ($ct) { $typeName = $ct.ProgrammaticName -replace '^ControlType\.', '' }
        $out.Add([pscustomobject]@{
                Name      = [string](& $get $AE::NameProperty)
                Type      = $typeName
                Aria      = [string]$aria
                IsDialog  = [bool]$isDialog
                Loc       = [string](& $get $AE::LocalizedControlTypeProperty)
                Focusable = [bool](& $get $AE::IsKeyboardFocusableProperty)
                Enabled   = [bool](& $get $AE::IsEnabledProperty)
                Offscreen = [bool](& $get $AE::IsOffscreenProperty)
                Rect      = & $get $AE::BoundingRectangleProperty
                HasSel    = [bool](& $get $AE::IsSelectionItemPatternAvailableProperty)
                Selected  = & $get ([System.Windows.Automation.SelectionItemPattern]::IsSelectedProperty)
                HasRange  = [bool](& $get $AE::IsRangeValuePatternAvailableProperty)
                RangeVal  = & $get ([System.Windows.Automation.RangeValuePattern]::ValueProperty)
                RangeMax  = & $get ([System.Windows.Automation.RangeValuePattern]::MaximumProperty)
                Value     = [string](& $get ([System.Windows.Automation.ValuePattern]::ValueProperty))
                HasToggle = [bool](& $get $AE::IsTogglePatternAvailableProperty)
            })
        # Cached pattern-availability flags are not reliable through every provider: ask
        # the live element for the patterns the checks depend on.
        $o = $out[$out.Count - 1]
        if (@('ListItem', 'TreeItem', 'TabItem') -contains $o.Type) {
            $pat = $null
            if ($e.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$pat)) {
                $o.HasSel = $true
                $o.Selected = $pat.Current.IsSelected
            }
        }
        if ($o.Type -eq 'ProgressBar') {
            $pat = $null
            if ($e.TryGetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern, [ref]$pat)) {
                $o.HasRange = $true
                $o.RangeVal = $pat.Current.Value
                $o.RangeMax = $pat.Current.Maximum
            }
        }
    }
    $out
}

# ---------------------------------------------------------------------------
# Checks
# ---------------------------------------------------------------------------

$results = [System.Collections.Generic.List[object]]::new()
function Check([string]$phase, [string]$id, [string]$what, [bool]$ok, [string[]]$details = @()) {
    $results.Add([pscustomobject]@{ Phase = $phase; Id = $id; What = $what; Ok = $ok; Details = $details })
    $mark = if ($ok) { 'ok  ' } else { 'FAIL' }
    Write-Host ("  [{0}] {1,-4} {2}" -f $mark, $id, $what)
    foreach ($d in ($details | Select-Object -First 12)) { Write-Host "         - $d" }
}

function Describe($e) {
    $r = $e.Rect
    $pos = if ($r) { '@{0:N0},{1:N0} {2:N0}x{3:N0}' -f $r.X, $r.Y, $r.Width, $r.Height } else { '' }
    $aria = if ($e.Aria) { " aria=$($e.Aria)" } else { '' }
    "$($e.Type)$aria '$($e.Name)' $pos"
}

$Interactive = @('Button', 'MenuItem', 'TabItem', 'ListItem', 'TreeItem', 'CheckBox', 'RadioButton', 'Edit', 'ComboBox', 'Slider', 'ProgressBar', 'Hyperlink', 'SplitButton', 'Spinner')

function Test-Generic([string]$phase, $winName, $els) {
    Check $phase 'G1' "window has a name ('$winName')" ([bool]$winName)
    $unnamedFocus = @($els | Where-Object { $_.Focusable -and -not $_.Name.Trim() })
    Check $phase 'G2' "no unnamed keyboard-focusable elements ($($unnamedFocus.Count) found)" ($unnamedFocus.Count -eq 0) @($unnamedFocus | ForEach-Object { Describe $_ })
    $unnamed = @($els | Where-Object { $Interactive -contains $_.Type -and -not $_.Name.Trim() })
    $n = @($els | Where-Object { $Interactive -contains $_.Type }).Count
    Check $phase 'G3' "interactive controls are named ($($n - $unnamed.Count)/$n)" ($unnamed.Count -eq 0) @($unnamed | ForEach-Object { Describe $_ })
    $noSel = @($els | Where-Object { @('ListItem', 'TreeItem', 'TabItem') -contains $_.Type -and -not $_.HasSel })
    Check $phase 'G4' "list / tree / tab items expose selection state ($($noSel.Count) without)" ($noSel.Count -eq 0) @($noSel | ForEach-Object { Describe $_ })
    # Icon-font glyphs (Private Use Area) or replacement characters as the whole name: a
    # screen reader announces them as "private use character".
    $glyphOnly = @($els | Where-Object { $_.Name -and ($_.Name.Trim() -match '^[-�\s]+$') })
    Check $phase 'G5' "no names made only of icon glyphs ($($glyphOnly.Count) found)" ($glyphOnly.Count -eq 0) @($glyphOnly | ForEach-Object { (Describe $_) + (' U+{0:X4}' -f [int]$_.Name.Trim()[0]) })
}

function Count-Types($els) {
    ($els | Group-Object Type | Sort-Object Count -Descending | ForEach-Object { "$($_.Name) $($_.Count)" }) -join ', '
}

$phaseChecks = @{
    main     = {
        param($els)
        $items = @($els | Where-Object Type -eq 'ListItem')
        Check 'main' 'M1' "file list rows are list items with names ($($items.Count))" ($items.Count -ge 3 -and -not ($items | Where-Object { -not $_.Name }))
        $sel = @($items | Where-Object { $_.Selected -eq $true })
        Check 'main' 'M2' "the selected file is reported selected ($(($sel | ForEach-Object Name) -join ', '))" (($sel | Where-Object { $_.Name -like 'Q3 mission report*' }).Count -ge 1)
        $tree = @($els | Where-Object Type -eq 'TreeItem')
        Check 'main' 'M3' "folder tree rows are tree items ($($tree.Count))" ($tree.Count -ge 1)
        $tabs = @($els | Where-Object Type -eq 'TabItem')
        Check 'main' 'M4' "tabs are tab items, one selected ($($tabs.Count))" ($tabs.Count -ge 2 -and @($tabs | Where-Object { $_.Selected -eq $true }).Count -eq 1)
        $buttons = @($els | Where-Object { $_.Type -eq 'Button' -and $_.Name })
        foreach ($want in @('Back', 'Forward', 'Up', 'Settings')) {
            Check 'main' 'M5' "toolbar button '$want' is present" (($buttons | Where-Object { $_.Name -like "$want*" }).Count -ge 1)
        }
    }
    palette  = {
        param($els)
        $edit = @($els | Where-Object { $_.Type -eq 'Edit' -and $_.Name })
        Check 'palette' 'P1' "the palette search field is a named edit ($($edit.Count))" ($edit.Count -ge 1)
        $items = @($els | Where-Object { $_.Type -eq 'ListItem' -and $_.Name -match 'copy' })
        Check 'palette' 'P2' "palette results are named list items ($($items.Count) match 'copy')" ($items.Count -ge 1)
    }
    settings = {
        param($els)
        $dlg = @($els | Where-Object { $_.Type -eq 'Window' -and $_.Name })
        Check 'settings' 'S1' "settings is a named window / dialog ($(($dlg | ForEach-Object Name) -join ', '))" ($dlg.Count -ge 1)
        $toggles = @($els | Where-Object { ($_.HasToggle -or $_.Type -eq 'CheckBox') })
        $unnamed = @($toggles | Where-Object { -not $_.Name })
        Check 'settings' 'S2' "switches have names ($($toggles.Count - $unnamed.Count)/$($toggles.Count))" ($toggles.Count -ge 1 -and $unnamed.Count -eq 0) @($unnamed | ForEach-Object { Describe $_ })
    }
    appearance = {
        param($els)
        $named = @($els | Where-Object { @('ComboBox', 'Slider', 'Edit') -contains $_.Type })
        $unnamed = @($named | Where-Object { -not $_.Name })
        Check 'appearance' 'A1' "combo boxes, sliders and fields have names ($($named.Count - $unnamed.Count)/$($named.Count))" ($named.Count -ge 1 -and $unnamed.Count -eq 0) @($unnamed | ForEach-Object { Describe $_ })
    }
    confirm  = {
        param($els)
        $dlg = @($els | Where-Object { $_.Type -eq 'Window' -and $_.Name -and ($_.Aria -match 'dialog' -or $_.IsDialog -or $_.Loc -match 'dialog') })
        Check 'confirm' 'C1' "the confirmation is a named (alert)dialog ($(($dlg | ForEach-Object { "$($_.Loc): $($_.Name)" }) -join '; '))" ($dlg.Count -ge 1)
        $btn = @($els | Where-Object { $_.Type -eq 'Button' -and $_.Name -match 'Delete|Cancel' })
        Check 'confirm' 'C2' "its buttons are named ($(($btn | ForEach-Object Name) -join ', '))" ($btn.Count -ge 2)
    }
    video    = {
        param($els)
        $sliders = @($els | Where-Object Type -eq 'Slider')
        foreach ($want in @('Position', 'Volume')) {
            $s = @($sliders | Where-Object Name -eq $want)
            Check 'video' 'V1' "the player's '$want' slider is named and has a value ($(($s | ForEach-Object { "$($_.Value)|$($_.RangeVal)" }) -join '; '))" ($s.Count -ge 1 -and ($s | Where-Object { $_.Value -or $null -ne $_.RangeVal }).Count -ge 1)
        }
        $buttons = @($els | Where-Object { $_.Type -eq 'Button' -and $_.Name })
        foreach ($want in @('Video: Launch replay.mp4', 'Play|Pause', 'Mute|Unmute', 'Loop')) {
            Check 'video' 'V2' "player button '$want' is present" (($buttons | Where-Object { $_.Name -match "^($want)$" }).Count -ge 1)
        }
    }
    mediatools = {
        param($els)
        $dlg = @($els | Where-Object { $_.Type -eq 'Window' -and $_.Name -match 'Convert media' })
        Check 'mediatools' 'T1' "the media tools dialog is a named window ($(($dlg | ForEach-Object { "$($_.Loc): $($_.Name)" }) -join '; '))" ($dlg.Count -ge 1)
        $tools = @($els | Where-Object { $_.Name -in @('Convert', 'Extract audio', 'Trim', 'Save frame') })
        Check 'mediatools' 'T2' "the tool switcher and the Run button are named ($(($tools | ForEach-Object Name) -join ', '))" ($tools.Count -ge 4)
        $fields = @($els | Where-Object { @('Edit', 'ComboBox') -contains $_.Type })
        $unnamed = @($fields | Where-Object { -not $_.Name.Trim() })
        Check 'mediatools' 'T3' "the preset and the output name fields are named ($($fields.Count - $unnamed.Count)/$($fields.Count))" ($fields.Count -ge 1 -and $unnamed.Count -eq 0) @($unnamed | ForEach-Object { Describe $_ })
    }
    progress = {
        param($els)
        $bars = @($els | Where-Object Type -eq 'ProgressBar')
        Check 'progress' 'R1' "a job progress bar is present ($($bars.Count))" ($bars.Count -ge 1)
        $bad = @($bars | Where-Object { -not $_.Name -or -not $_.HasRange -or $null -eq $_.RangeVal -or $_.RangeVal -lt 0 -or $_.RangeVal -gt 100 })
        Check 'progress' 'R2' "progress bars have a name and a 0-100 value ($(($bars | ForEach-Object { "$($_.Name)=$($_.RangeVal)" }) -join '; '))" ($bars.Count -ge 1 -and $bad.Count -eq 0) @($bad | ForEach-Object { Describe $_ })
    }
}

# ---------------------------------------------------------------------------
# Run
# ---------------------------------------------------------------------------

$phases = @('main', 'palette', 'settings', 'appearance', 'confirm', 'video', 'mediatools', 'progress') | Where-Object { $Only.Count -eq 0 -or $Only -contains $_ }
$clip = D 'Videos\Launch replay.mp4'
$libs = [bool]$env:SOYUZ_FFMPEG_DIR -or [bool](Get-ChildItem -LiteralPath $exeDir -Filter 'avcodec-*.dll' -ErrorAction SilentlyContinue)
if (-not ($libs -and (Test-Path -LiteralPath $clip)) -and ($phases -contains 'video' -or $phases -contains 'mediatools')) {
    Write-Warning 'video / mediatools skipped: set SOYUZ_FFMPEG_DIR to a folder with the FFmpeg libraries (and ffmpeg for the demo video)'
    $phases = @($phases | Where-Object { $_ -notin @('video', 'mediatools') })
}
function Hold([string]$name) { "mark:phase-$name"; "until:$(Join-Path $work "go-$name")" }

$steps = @('size:1400x900', 'wait:2500', 'select:Q3 mission report.md', 'wait:800')
if ($phases -contains 'main') { $steps += Hold 'main' }
if ($phases -contains 'palette') { $steps += @('command_palette', 'wait:300', 'keys:copy', 'wait:900') + (Hold 'palette') + @('keys:{Escape}', 'wait:200', 'keys:{Escape}', 'wait:300') }
if ($phases -contains 'settings') { $steps += @('settings:general', 'wait:1200') + (Hold 'settings') + @('settings:close', 'wait:400') }
if ($phases -contains 'appearance') { $steps += @('settings:appearance', 'wait:1200') + (Hold 'appearance') + @('settings:close', 'wait:400') }
if ($phases -contains 'confirm') {
    $steps += @('focus_list', 'select:Launch checklist.txt', 'delete_permanent', 'wait:1000') + (Hold 'confirm') + @('keys:{Escape}', 'wait:600', "expect:exists=$(D 'Documents\Reports\Launch checklist.txt')")
}
if ($phases -contains 'video' -or $phases -contains 'mediatools') {
    $steps += @("cd:$(D 'Videos')", 'wait:1000', 'select:Launch replay.mp4', 'wait:500')
    # Quick Look: its player bar is wide enough for the volume slider (the pane's hides it
    # when narrow).
    if ($phases -contains 'video') {
        $steps += @('quick_look', 'wait:2000', 'player_play_pause', 'wait:2000', 'player_play_pause', 'wait:800') + (Hold 'video') + @('keys:{Escape}', 'wait:500')
    }
    if ($phases -contains 'mediatools') { $steps += @('focus_list', 'media_convert', 'wait:2500') + (Hold 'mediatools') + @('keys:{Escape}', 'wait:500') }
}
if ($phases -contains 'progress') { $steps += @('select_tab:2', 'wait:1200', 'select:Telemetry dump.dat', 'copy_to_other_pane', 'wait:2500') + (Hold 'progress') + @('jobs_cancel_all', 'wait:1500') }
$steps += 'quit'

New-Item -ItemType Directory -Path $data | Out-Null
New-Item -ItemType File -Path $marker | Out-Null
$exit = 0
try {
    $cfg = @"
schema = 1

[general]
start_location = $(Q (D 'Documents\Reports'))
language = "en-US"

[appearance]
theme = "dark"
$(if ($env:SOYUZ_WINDOW_POS) { 'native_dialogs = false' })

[behaviour]
restore_session = true

[jobs]
auto_open_activity = true

[media]
muted = true

[sidebar]
mode = "both"
sections = ["favorites", "tree", "trash"]
"@
    [System.IO.File]::WriteAllText((Join-Path $data 'config.toml'), $cfg, [System.Text.UTF8Encoding]::new($false))
    $session = @"
version = 1
active_tab = 0
welcome_seen = true

[[tabs]]
dual = false
focused = 0
split = 0.5
[[tabs.panes]]
location = $(Q (D 'Documents\Reports'))

[[tabs]]
dual = true
focused = 0
split = 0.5
[[tabs.panes]]
location = $(Q (D 'Downloads'))
[[tabs.panes]]
location = $(Q (D 'Archive'))
"@
    [System.IO.File]::WriteAllText((Join-Path $data 'session.toml'), $session, [System.Text.UTF8Encoding]::new($false))

    $log = Join-Path $work 'soyuz.log'
    $psi = [System.Diagnostics.ProcessStartInfo]::new($Exe)
    $psi.UseShellExecute = $false
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardOutput = $true
    $psi.WorkingDirectory = $exeDir
    $psi.Environment['SOYUZ_SCRIPT'] = $steps -join ';'
    $psi.Environment['SOYUZ_THROTTLE_MBPS'] = '2'
    $psi.Environment['RUST_LOG'] = 'info,wgpu_core=warn,wgpu_hal=warn,naga=warn'
    $proc = [System.Diagnostics.Process]::Start($psi)
    $sb = [System.Text.StringBuilder]::new()
    $errEvt = Register-ObjectEvent -InputObject $proc -EventName ErrorDataReceived -Action { if ($EventArgs.Data) { [void]$Event.MessageData.AppendLine($EventArgs.Data) } } -MessageData $sb
    $outEvt = Register-ObjectEvent -InputObject $proc -EventName OutputDataReceived -Action { if ($EventArgs.Data) { [void]$Event.MessageData.AppendLine($EventArgs.Data) } } -MessageData $sb
    $proc.BeginErrorReadLine(); $proc.BeginOutputReadLine()
    $deadline = (Get-Date).AddSeconds($TimeoutSec)

    foreach ($ph in $phases) {
        Write-Host "== $ph"
        while (-not $sb.ToString().Contains("SOYUZ_SCRIPT: mark phase-$ph")) {
            if ($proc.HasExited -or (Get-Date) -gt $deadline) { throw "the app did not reach phase $ph" }
            Start-Sleep -Milliseconds 200
        }
        Start-Sleep -Milliseconds 400
        # The first UIA request turns AccessKit on; the full tree follows a frame later.
        $els = @(); $win = $null
        for ($i = 0; $i -lt 30; $i++) {
            $win = Get-SoyuzWindow $proc.Id
            if ($win) { $els = @(Get-Snapshot $win) }
            if ($els.Count -ge 20) { break }
            Start-Sleep -Milliseconds 300
        }
        # A dialog is its own native window: its AccessKit tree is built only after the first
        # request reached it, so read the tree once more.
        if (@('confirm', 'mediatools') -contains $ph) {
            Start-Sleep -Milliseconds 1200
            $els = @(Get-Snapshot $win)
        }
        if (-not $win) { throw 'no Soyuz window found through UI Automation' }
        Write-Host "   $($els.Count) elements: $(Count-Types $els)"
        if ($Dump) { $els | ForEach-Object { $_ | Add-Member -NotePropertyName Phase -NotePropertyValue $ph -PassThru | ConvertTo-Json -Compress -Depth 3 } | Add-Content -LiteralPath $Dump -Encoding utf8 }
        Test-Generic $ph $win.Current.Name $els
        & $phaseChecks[$ph] $els
        New-Item -ItemType File -Path (Join-Path $work "go-$ph") | Out-Null
    }
    if (-not $proc.WaitForExit(60000)) { $proc.Kill($true) }
    $proc.WaitForExit()
    Unregister-Event -SourceIdentifier $errEvt.Name; Unregister-Event -SourceIdentifier $outEvt.Name
    $text = $sb.ToString()
    [System.IO.File]::WriteAllText($log, $text)
    foreach ($m in [regex]::Matches($text, 'SOYUZ_SCRIPT: expect (ok|FAILED)[^\r\n]*')) {
        Check 'script' 'X1' $m.Value.Substring(14) ($m.Groups[1].Value -eq 'ok')
    }
    if ($text -match 'panicked') { Check 'script' 'X2' 'the app did not panic' $false }
}
finally {
    if ($proc -and -not $proc.HasExited) { $proc.Kill($true) }
    Remove-Item -LiteralPath $marker -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $data -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    if (-not $KeepDemo) { Remove-Item -LiteralPath $Demo -Recurse -Force -ErrorAction SilentlyContinue }
}

$failed = @($results | Where-Object { -not $_.Ok })
Write-Host ''
Write-Host ("{0} checks, {1} failed" -f $results.Count, $failed.Count)
if ($Report) {
    $md = [System.Text.StringBuilder]::new()
    [void]$md.AppendLine("# UIA audit ($(Get-Date -Format 'yyyy-MM-dd HH:mm'))").AppendLine()
    [void]$md.AppendLine("$($results.Count) checks, $($failed.Count) failed.").AppendLine()
    [void]$md.AppendLine('| State | Check | Result | What |').AppendLine('|---|---|---|---|')
    foreach ($r in $results) {
        [void]$md.AppendLine("| $($r.Phase) | $($r.Id) | $(if ($r.Ok) { 'pass' } else { '**fail**' }) | $($r.What -replace '\|', '/') |")
    }
    foreach ($r in $failed) {
        if ($r.Details.Count -gt 0) {
            [void]$md.AppendLine().AppendLine("## $($r.Phase) $($r.Id)").AppendLine()
            foreach ($d in $r.Details) { [void]$md.AppendLine("- ``$d``") }
        }
    }
    [System.IO.File]::WriteAllText($Report, $md.ToString(), [System.Text.UTF8Encoding]::new($false))
    Write-Host "report: $Report"
}
exit $failed.Count
