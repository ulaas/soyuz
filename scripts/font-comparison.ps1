<#
.SYNOPSIS
  Renders the font comparison sheets (default: target/font-comparison/) from the real app.

.DESCRIPTION
  1. Creates the demo tree (scripts/demo-fixture.ps1) at <repo drive>:\SoyuzFontDemo plus a
     folder with Turkish file names, so the pictures contain no personal paths.
  2. Runs Soyuz once in portable mode, off-screen (SOYUZ_WINDOW_POS) and driven only by
     SOYUZ_SCRIPT: a dual pane with a throttled copy job (activity card), the terminal and
     the toolbar / sidebar, then Settings -> Fonts and the text editor. For every candidate
     it switches `[fonts]` live and saves a window screenshot; a second pass varies the
     renderer settings (hinting strength, text gamma, light theme).
  3. Crops the captures 1:1 (no scaling: judge them at 100 %) into labelled tiles and
     stitches the sheets with System.Drawing.
  4. Removes the portable marker, portable-data/, the raw captures and the demo tree.

.PARAMETER Exe
  The soyuz binary (default target\debug\soyuz.exe; build it first).

.PARAMETER Raw
  Reuse the captures in this folder (only re-compose the sheets).

.EXAMPLE
  cargo build -p soyuz-app; ./scripts/font-comparison.ps1
#>
[CmdletBinding()]
param(
    [string]$Exe,
    [string]$Demo,
    [string]$OutDir,
    [string]$Raw,
    [switch]$KeepRaw,
    [int]$TimeoutSec = 600
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$repo = Split-Path -Parent $PSScriptRoot
if (-not $Exe) { $Exe = Join-Path $repo 'target\debug\soyuz.exe' }
if (-not $Demo) { $Demo = Join-Path ((Split-Path -Qualifier $repo) + '\') 'SoyuzFontDemo' }
if (-not $OutDir) { $OutDir = Join-Path $repo 'target\font-comparison' }

# Rendering: `before` = what Soyuz shipped (egui's default hinting and coverage curves),
# `after` = the new defaults (crisp hinting, text gamma 0.45 dark / 0.80 light).
$before = @('config:fonts.hinting=true', 'config:fonts.hinting_strength="normal"', 'config:fonts.text_gamma_dark=0.0', 'config:fonts.text_gamma_light=1.0')
$after = @('config:fonts.hinting=true', 'config:fonts.hinting_strength="crisp"', 'config:fonts.text_gamma_dark=0.45', 'config:fonts.text_gamma_light=0.8')
# Interface candidates (the first is the old default, drawn the old way).
$uiFonts = @(
    @{ id = 'segoe'; family = 'Segoe UI'; label = 'Segoe UI (system) - old default, old rendering'; render = $before }
    @{ id = 'inter'; family = 'Inter'; label = 'Inter (embedded) - new default'; render = $after }
    @{ id = 'plex'; family = 'IBM Plex Sans'; label = 'IBM Plex Sans (embedded)'; render = $after }
    @{ id = 'noto'; family = 'Noto Sans'; label = 'Noto Sans (embedded)'; render = $after }
)
# Monospace candidates (terminal, editor, hex).
$monoFonts = @(
    @{ id = 'consolas'; family = 'Consolas'; label = 'Consolas (system) - old default, old rendering'; render = $before }
    @{ id = 'cascadia'; family = 'Cascadia Mono'; label = 'Cascadia Mono (embedded) - new default'; render = $after }
    @{ id = 'jetbrains'; family = 'JetBrains Mono'; label = 'JetBrains Mono (embedded)'; render = $after }
    @{ id = 'plexmono'; family = 'IBM Plex Mono'; label = 'IBM Plex Mono (embedded)'; render = $after }
    @{ id = 'scp'; family = 'Source Code Pro'; label = 'Source Code Pro (embedded)'; render = $after }
)
# Heading faces (Settings page titles and section names), new rendering.
$headings = @(
    @{ id = 'segoe'; ui = 'Segoe UI'; label = 'Segoe UI Semibold (system)' }
    @{ id = 'inter'; ui = 'Inter'; label = 'Inter SemiBold (embedded) - new default' }
    @{ id = 'plex'; ui = 'IBM Plex Sans'; label = 'IBM Plex Sans SemiBold (embedded)' }
    @{ id = 'noto'; ui = 'Noto Sans'; label = 'Noto Sans SemiBold (embedded)' }
)
# Renderer variants: id, label, config steps (applied in order, cumulative).
$variants = @(
    @{ id = 'off'; label = 'hinting off'; steps = @('config:fonts.hinting=false') }
    @{ id = 'normal'; label = 'hinting normal = egui default (before)'; steps = @('config:fonts.hinting=true', 'config:fonts.hinting_strength="normal"') }
    @{ id = 'light'; label = 'hinting light'; steps = @('config:fonts.hinting_strength="light"') }
    @{ id = 'crisp'; label = 'hinting crisp'; steps = @('config:fonts.hinting_strength="crisp"') }
    @{ id = 'crisp-g45'; label = 'hinting crisp + dark text gamma 0.45 (after)'; steps = @('config:fonts.text_gamma_dark=0.45') }
    @{ id = 'crisp-g60'; label = 'hinting crisp + dark text gamma 0.60'; steps = @('config:fonts.text_gamma_dark=0.6') }
    @{ id = 'l-normal'; label = 'light theme: hinting normal, gamma 1.0 (before)'; steps = @('config:fonts.text_gamma_dark=0.0', 'config:fonts.hinting_strength="normal"', 'theme:light') }
    @{ id = 'l-crisp'; label = 'light theme: hinting crisp, gamma 1.0'; steps = @('config:fonts.hinting_strength="crisp"') }
    @{ id = 'l-crisp-g80'; label = 'light theme: hinting crisp, gamma 0.80 (after)'; steps = @('config:fonts.text_gamma_light=0.8') }
)
$renderFonts = @('segoe', 'inter', 'plex', 'noto')
$turkish = 'Belgeler - Türkçe'

function Q([string]$s) { "'" + $s + "'" }
function D([string]$rel) { if ($rel) { Join-Path $Demo $rel } else { $Demo } }

function New-TurkishFolder {
    $dir = D $turkish
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $dir 'Doğum günü fotoğrafları') | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $dir 'Şirket sözleşmeleri') | Out-Null
    $files = @{
        'Ödeme planı.csv'                 = "ay,tutar`nOcak,1250`nŞubat,980`n"
        'Üçüncü çeyrek özeti.md'          = "# Üçüncü çeyrek özeti`n`nGelir %12 arttı; gider ığdır şubesinde düştü.`n"
        'İstanbul - İzmir yolculuğu.txt'  = "Kalkış 08:40, varış 14:15. Çiğ köfte, ayran, şalgam.`n"
        'Kışlık lastik faturası.txt'      = "Fatura no: 2026-0917 · Tutar: 4.860,00 TL`n"
        'Iğdır şubesi ağ çizelgesi.json'  = "{ `"şube`": `"Iğdır`", `"çalışan`": 14 }`n"
        'Gökyüzü gözlem günlüğü.md'       = "# Gökyüzü gözlem günlüğü`n`nJüpiter ve Satürn; ışık kirliliği az.`n"
    }
    foreach ($k in $files.Keys) { [System.IO.File]::WriteAllText((Join-Path $dir $k), $files[$k], [System.Text.UTF8Encoding]::new($false)) }
    $sample = @'
fn main() { let x = 0x1F; /* 0O 1lI |{}[] */ }
PS> git status --short    M src/orbit.rs   ?? notlar.md
İstanbul ışık şğüöç ĞÜŞİÖÇ   == != <= >= -> => :: //
┌──────────┬─────────┬──────────┐
│ dosya    │  boyut  │ değişti  │
├──────────┼─────────┼──────────┤
│ orbit.rs │ 12,4 KB │ 14:32    │
└──────────┴─────────┴──────────┘
'@
    # UTF-16 with a BOM: `type` in cmd.exe prints it as Unicode whatever the code page.
    [System.IO.File]::WriteAllText((Join-Path $dir 'kod.txt'), $sample, [System.Text.UnicodeEncoding]::new($false, $true))
    Move-Item -LiteralPath (D 'Downloads\Telemetry dump.dat') -Destination (Join-Path $dir 'Yedekleme arşivi.dat')
}

function Invoke-Capture([string]$rawDir) {
    if (-not (Test-Path -LiteralPath $Exe)) { throw "no binary at $Exe (cargo build -p soyuz-app first)" }
    $exeDir = Split-Path -Parent $Exe
    $marker = Join-Path $exeDir 'portable'
    $data = Join-Path $exeDir 'portable-data'
    if ((Test-Path -LiteralPath $marker) -or (Test-Path -LiteralPath $data)) {
        throw "$exeDir already has a portable marker or portable-data/; move them away first."
    }
    & (Join-Path $PSScriptRoot 'demo-fixture.ps1') -Root $Demo -BigFileMB 400
    New-TurkishFolder
    New-Item -ItemType Directory -Path $data | Out-Null
    New-Item -ItemType File -Path $marker | Out-Null
    try {
        $cfg = @"
schema = 1

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
mode = "places"
sections = ["favorites", "smart", "tags", "trash"]

[terminal]
shell = "cmd.exe"
args = ["/k", "type kod.txt"]
font_size = 13.0

[fonts]
hinting = true
hinting_strength = "normal"
text_gamma_dark = 0.0
text_gamma_light = 1.0
"@
        [System.IO.File]::WriteAllText((Join-Path $data 'config.toml'), $cfg, [System.Text.UTF8Encoding]::new($false))
        $favs = @('Documents', 'Photos', 'Projects', $turkish, 'Downloads') | ForEach-Object { "{ name = `"$_`", path = $(Q (D $_)) }" }
        $session = @"
version = 1
active_tab = 0
welcome_seen = true
show_hidden = false
theme = "dark"
favorites = [$($favs -join ', ')]

[[tabs]]
dual = true
focused = 0
split = 0.55
[[tabs.panes]]
location = $(Q (D $turkish))
[[tabs.panes]]
location = $(Q (D 'Archive'))
"@
        [System.IO.File]::WriteAllText((Join-Path $data 'session.toml'), $session, [System.Text.UTF8Encoding]::new($false))

        $steps = [System.Collections.Generic.List[string]]::new()
        $shot = { param($name) $steps.Add('wait:700'); $steps.Add("shot:$rawDir\$name.png") }
        $set = { param($role, $family) $steps.Add("config:fonts.$role.family=`"$family`"") }
        $steps.AddRange([string[]]@('size:1600x1000', 'wait:3000', 'select:Yedekleme arşivi.dat', 'copy_to_other_pane', 'wait:1500',
                'toggle_terminal', 'wait:3000', 'focus_list', 'select:Ödeme planı.csv', 'wait:500'))
        # Interface fonts at 13 and 14 px (the monospace roles stay at the old Consolas).
        foreach ($k in 'terminal', 'code', 'hex') { & $set $k 'Consolas' }
        foreach ($u in $uiFonts) {
            $steps.AddRange([string[]]$u.render)
            & $set 'ui' $u.family
            foreach ($size in 13, 14) {
                $steps.Add("config:appearance.font_size=$size.0")
                & $shot "main-$($u.id)-$size"
                $steps.Add('settings:fonts')
                & $shot "settings-$($u.id)-$size"
                $steps.Add('settings:close')
            }
        }
        $steps.Add('config:appearance.font_size=13.0')
        # Monospace fonts: the terminal, then the text editor.
        foreach ($m in $monoFonts) {
            $steps.AddRange([string[]]$m.render)
            foreach ($k in 'terminal', 'code', 'hex') { & $set $k $m.family }
            & $shot "term-$($m.id)"
        }
        $steps.Add("edit:$(D 'Projects\orbit-tracker\src\orbit.rs')")
        $steps.Add('wait:1500')
        foreach ($m in $monoFonts) {
            $steps.AddRange([string[]]$m.render)
            & $set 'code' $m.family
            & $shot "editor-$($m.id)"
        }
        $steps.Add('toggle_editor_panel')
        # Heading faces (derived from the interface font).
        $steps.AddRange([string[]]$after)
        $steps.Add('settings:fonts')
        foreach ($h in $headings) {
            & $set 'ui' $h.ui
            & $shot "heading-$($h.id)"
        }
        $steps.Add('settings:close')
        # Renderer variants per interface font.
        foreach ($id in $renderFonts) {
            $u = $uiFonts | Where-Object { $_.id -eq $id }
            & $set 'ui' $u.family
            foreach ($v in $variants) {
                $steps.AddRange([string[]]$v.steps)
                & $shot "render-$id-$($v.id)"
            }
            $steps.Add('theme:dark')
        }
        # The shipped defaults: no font overrides at all.
        foreach ($k in 'ui', 'terminal', 'code', 'hex') { & $set $k '' }
        $steps.AddRange([string[]]$after)
        & $shot 'after-main'
        $steps.Add('theme:light')
        & $shot 'after-main-light'
        $steps.AddRange([string[]]@('theme:dark', 'jobs_cancel_all', 'wait:1000', 'quit'))

        $psi = [System.Diagnostics.ProcessStartInfo]::new($Exe)
        $psi.UseShellExecute = $false
        $psi.RedirectStandardError = $true
        $psi.RedirectStandardOutput = $true
        $psi.WorkingDirectory = $exeDir
        $psi.Environment['SOYUZ_SCRIPT'] = ($steps -join ';')
        $psi.Environment['SOYUZ_THROTTLE_MBPS'] = '1'
        # Off-screen and not activated: the capture never shows up on the desktop.
        $psi.Environment['SOYUZ_WINDOW_POS'] = '-4000,-4000'
        $psi.Environment['RUST_LOG'] = 'info,wgpu_core=warn,wgpu_hal=warn,naga=warn'
        $p = [System.Diagnostics.Process]::Start($psi)
        $errTask = $p.StandardError.ReadToEndAsync()
        $outTask = $p.StandardOutput.ReadToEndAsync()
        if (-not $p.WaitForExit($TimeoutSec * 1000)) {
            Write-Warning 'capture timed out; killing it'
            $p.Kill($true)
            $p.WaitForExit()
        }
        $log = $errTask.Result + $outTask.Result
        [System.IO.File]::WriteAllText((Join-Path $rawDir 'session.log'), $log)
        $saved = ([regex]::Matches($log, 'SOYUZ_SCRIPT: saved')).Count
        Write-Host "saved $saved screenshot(s)"
        [regex]::Matches($log, '(SOYUZ_SCRIPT[^\r\n]*(cannot|unknown|FAILED|no table)[^\r\n]*)') | ForEach-Object { Write-Warning $_.Value }
    }
    finally {
        Remove-Item -LiteralPath $marker -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $data -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $Demo -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# ---- Composition: crop regions 1:1 into labelled tiles stacked in a grid.
$labelFont = [System.Drawing.Font]::new('Segoe UI', 11, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
function New-Sheet([string]$file, [string]$title, [object[]]$tiles, [System.Drawing.Rectangle]$crop, [int]$cols) {
    $pad = 12; $labelH = 22; $titleH = 30
    $rows = [Math]::Ceiling($tiles.Count / $cols)
    $w = $pad + $cols * ($crop.Width + $pad)
    $h = $titleH + $rows * ($labelH + $crop.Height + $pad) + $pad
    $bmp = [System.Drawing.Bitmap]::new($w, $h, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.Clear([System.Drawing.Color]::FromArgb(32, 32, 36))
    $g.TextRenderingHint = 'ClearTypeGridFit'
    $g.InterpolationMode = 'NearestNeighbor'
    $g.DrawString($title, [System.Drawing.Font]::new('Segoe UI', 14, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel), [System.Drawing.Brushes]::White, $pad, 7)
    for ($i = 0; $i -lt $tiles.Count; $i++) {
        $t = $tiles[$i]
        $x = $pad + ($i % $cols) * ($crop.Width + $pad)
        $y = $titleH + [Math]::Floor($i / $cols) * ($labelH + $crop.Height + $pad)
        $g.DrawString($t.label, $labelFont, [System.Drawing.Brushes]::Gold, $x, $y + 3)
        $src = Join-Path $Raw "$($t.shot).png"
        if (Test-Path -LiteralPath $src) {
            $img = [System.Drawing.Image]::FromFile($src)
            $dest = [System.Drawing.Rectangle]::new($x, $y + $labelH, $crop.Width, $crop.Height)
            $g.DrawImage($img, $dest, $crop, [System.Drawing.GraphicsUnit]::Pixel)
            $img.Dispose()
        } else {
            $g.DrawString("missing: $($t.shot)", $labelFont, [System.Drawing.Brushes]::Red, $x, $y + $labelH + 4)
        }
    }
    $g.Dispose()
    $bmp.Save((Join-Path $OutDir $file), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host "  $file"
}

function Compose {
    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
    # Regions of the 1600x1000 window (logical = physical pixels at 100 % scaling).
    $listCrop = [System.Drawing.Rectangle]::new(0, 0, 1600, 445)
    $termCrop = [System.Drawing.Rectangle]::new(0, 785, 900, 150)
    $settingsCrop = [System.Drawing.Rectangle]::new(300, 150, 1000, 700)
    $editorCrop = [System.Drawing.Rectangle]::new(507, 155, 723, 400)
    $headingCrop = [System.Drawing.Rectangle]::new(540, 150, 760, 400)
    $fullCrop = [System.Drawing.Rectangle]::new(0, 0, 1600, 1000)
    $renderCrop = [System.Drawing.Rectangle]::new(0, 140, 780, 300)
    foreach ($size in 13, 14) {
        New-Sheet "ui-$size-px.png" "Interface fonts at $size px - sidebar, toolbar, file list, activity card (100 % scale)" `
            ($uiFonts | ForEach-Object { @{ label = $_.label; shot = "main-$($_.id)-$size" } }) $listCrop 1
        New-Sheet "settings-$size-px.png" "Settings > Fonts at $size px" `
            ($uiFonts | ForEach-Object { @{ label = $_.label; shot = "settings-$($_.id)-$size" } }) $settingsCrop 2
    }
    New-Sheet 'mono-terminal.png' 'Monospace fonts - terminal (13 px)' `
        ($monoFonts | ForEach-Object { @{ label = $_.label; shot = "term-$($_.id)" } }) $termCrop 1
    New-Sheet 'mono-editor.png' 'Monospace fonts - text editor (13 px)' `
        ($monoFonts | ForEach-Object { @{ label = $_.label; shot = "editor-$($_.id)" } }) $editorCrop 2
    New-Sheet 'headings.png' 'Heading faces (Settings > Fonts, 13 px, new rendering)' `
        ($headings | ForEach-Object { @{ label = $_.label; shot = "heading-$($_.id)" } }) $headingCrop 2
    New-Sheet 'before-after.png' 'Whole window at 100 % - before and after' @(
        @{ label = 'before: Segoe UI + Consolas, egui default hinting and coverage'; shot = 'main-segoe-13' }
        @{ label = 'after: the new defaults (Inter + Cascadia Mono, crisp hinting, text gamma 0.45)'; shot = 'after-main' }
        @{ label = 'after, light theme (text gamma 0.80)'; shot = 'after-main-light' }
    ) $fullCrop 1
    foreach ($id in $renderFonts) {
        $u = $uiFonts | Where-Object { $_.id -eq $id }
        New-Sheet "rendering-$id.png" "Renderer settings with $($u.family) at 13 px" `
            ($variants | ForEach-Object { @{ label = $_.label; shot = "render-$id-$($_.id)" } }) $renderCrop 2
    }
}

$ownRaw = -not $Raw
if ($ownRaw) {
    $Raw = Join-Path ([System.IO.Path]::GetTempPath()) ('soyuz-fonts-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
    New-Item -ItemType Directory -Path $Raw | Out-Null
    Invoke-Capture $Raw
}
try { Compose }
finally {
    if ($ownRaw -and -not $KeepRaw) { Remove-Item -LiteralPath $Raw -Recurse -Force -ErrorAction SilentlyContinue }
    elseif ($ownRaw) { Write-Host "raw captures kept in $Raw" }
}
