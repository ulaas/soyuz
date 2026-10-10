<#
.SYNOPSIS
  Renders target/font-comparison/inter-variable.png: the shipped static Inter against the
  variable Inter with optical sizes ("Inter Variable (optical sizes)").

.DESCRIPTION
  Runs Soyuz once in portable mode, off-screen (SOYUZ_WINDOW_POS) and driven only by
  SOYUZ_SCRIPT, on a small demo folder (no personal paths). For the static Inter and for
  Inter Variable with crisp and with light hinting (-Only picks variants by id) it
  captures the file list (body text, small status text), Settings > Fonts (17 / 19 pt
  headings, captions) and an empty folder (22 pt display title), then stitches 1:1 crops
  (judge them at 100 %) plus 3x zooms of the headings into one sheet. Cleans up the
  portable marker, portable-data/, the demo folder and the raw captures.

.EXAMPLE
  cargo build -p soyuz-app --release; ./scripts/font-inter-variable.ps1 -Exe target\release\soyuz.exe
#>
[CmdletBinding()]
param(
    [string]$Exe,
    [string]$Out,
    [int]$TimeoutSec = 180,
    [switch]$KeepRaw,
    [string[]]$Only
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$repo = Split-Path -Parent $PSScriptRoot
if (-not $Exe) { $Exe = Join-Path $repo 'target\debug\soyuz.exe' }
if (-not $Out) { $Out = Join-Path $repo 'target\font-comparison\inter-variable.png' }
$exeDir = Split-Path -Parent (Resolve-Path $Exe)
$marker = Join-Path $exeDir 'portable'
$data = Join-Path $exeDir 'portable-data'
if ((Test-Path -LiteralPath $marker) -or (Test-Path -LiteralPath $data)) {
    throw "$exeDir already has a portable marker or portable-data/; move them away first."
}
$tag = [guid]::NewGuid().ToString('N').Substring(0, 8)
$demo = Join-Path ([System.IO.Path]::GetTempPath()) "SoyuzInterDemo-$tag"
$raw = Join-Path ([System.IO.Path]::GetTempPath()) "soyuz-inter-raw-$tag"
New-Item -ItemType Directory -Path $raw, $demo | Out-Null
$variants = @(
    @{ id = 'static'; family = 'Inter'; hinting = 'crisp'; label = 'Inter (static Regular / SemiBold) - default, crisp hinting' }
    @{ id = 'variable'; family = 'Inter Variable'; hinting = 'crisp'; label = 'Inter Variable (optical sizes), crisp hinting: body opsz 14 / 400, captions 14 / 440, titles 15 / 600, large headings 20 / 600' }
    @{ id = 'variable-light'; family = 'Inter Variable'; hinting = 'light'; label = 'Inter Variable (optical sizes), light hinting' }
)
if ($Only) { $variants = @($variants | Where-Object { $Only -contains $_.id }) }
try {
    foreach ($d in 'Belgeler', 'Fotoğraflar', 'Projeler', 'Şirket sözleşmeleri', 'Boş klasör') { New-Item -ItemType Directory -Force (Join-Path $demo $d) | Out-Null }
    $files = @{
        'Ödeme planı.csv'                = "ay,tutar`nOcak,1250`n"
        'Üçüncü çeyrek özeti.md'         = "# Üçüncü çeyrek özeti`n`nGelir %12 arttı.`n"
        'İstanbul - İzmir yolculuğu.txt' = "Kalkış 08:40, varış 14:15.`n"
        'Quarterly report 2026.pdf'      = 'pdf'
        'orbit-tracker.zip'              = 'zip'
        'Invoice 0917 - Kışlık lastik.txt' = "Fatura no: 2026-0917`n"
    }
    foreach ($k in $files.Keys) { [System.IO.File]::WriteAllText((Join-Path $demo $k), $files[$k], [System.Text.UTF8Encoding]::new($false)) }
    New-Item -ItemType Directory -Path $data | Out-Null
    New-Item -ItemType File -Path $marker | Out-Null
    $q = { param($s) "'" + $s + "'" }
    $cfg = @"
schema = 1

[general]
start_location = $(& $q $demo)
language = "en-US"

[appearance]
theme = "dark"
"@
    [System.IO.File]::WriteAllText((Join-Path $data 'config.toml'), $cfg, [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText((Join-Path $data 'session.toml'), "version = 1`nwelcome_seen = true`n", [System.Text.UTF8Encoding]::new($false))
    $steps = [System.Collections.Generic.List[string]]::new()
    $steps.AddRange([string[]]@('size:1400x900', 'wait:2000', 'select:Ödeme planı.csv'))
    foreach ($v in $variants) {
        $steps.Add("config:fonts.ui.family=`"$($v.family)`"")
        $steps.Add("config:fonts.hinting_strength=`"$($v.hinting)`"")
        $steps.Add("cd:$demo"); $steps.Add('wait:800'); $steps.Add("shot:$raw\list-$($v.id).png")
        $steps.Add('settings:fonts'); $steps.Add('wait:800'); $steps.Add("shot:$raw\settings-$($v.id).png"); $steps.Add('settings:close')
        $steps.Add("cd:$(Join-Path $demo 'Boş klasör')"); $steps.Add('wait:800'); $steps.Add("shot:$raw\empty-$($v.id).png")
    }
    $steps.Add('quit')
    $psi = [System.Diagnostics.ProcessStartInfo]::new((Resolve-Path $Exe).Path)
    $psi.UseShellExecute = $false
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardOutput = $true
    $psi.WorkingDirectory = $exeDir
    $psi.Environment['SOYUZ_SCRIPT'] = ($steps -join ';')
    $psi.Environment['SOYUZ_WINDOW_POS'] = '-4000,-4000'
    $psi.Environment['RUST_LOG'] = 'info,wgpu_core=warn,wgpu_hal=warn,naga=warn'
    $p = [System.Diagnostics.Process]::Start($psi)
    $errTask = $p.StandardError.ReadToEndAsync()
    $outTask = $p.StandardOutput.ReadToEndAsync()
    if (-not $p.WaitForExit($TimeoutSec * 1000)) { $p.Kill($true); $p.WaitForExit(); Write-Warning 'capture timed out' }
    $log = $errTask.Result + $outTask.Result
    [regex]::Matches($log, '(SOYUZ_SCRIPT[^\r\n]*(cannot|unknown|FAILED)[^\r\n]*)') | ForEach-Object { Write-Warning $_.Value }

    # ---- Sheet: per region, static above variable, 1:1; then 3x zooms of headings.
    $crops = @(
        @{ shot = 'list'; title = 'File list, sidebar, status bar (body 13 pt, small 11 pt)'; rect = [System.Drawing.Rectangle]::new(0, 40, 1000, 300); zoom = 1 }
        @{ shot = 'list'; title = 'Status bar (small text, 11 pt) at 3x'; rect = [System.Drawing.Rectangle]::new(0, 872, 300, 28); zoom = 3 }
        @{ shot = 'settings'; title = 'Settings > Fonts (17 / 19 pt headings, captions)'; rect = [System.Drawing.Rectangle]::new(180, 60, 1040, 330); zoom = 1 }
        @{ shot = 'settings'; title = 'Settings headings at 3x'; rect = [System.Drawing.Rectangle]::new(180, 60, 400, 70); zoom = 3 }
        @{ shot = 'empty'; title = 'Empty folder (22 pt display title)'; rect = [System.Drawing.Rectangle]::new(380, 330, 760, 200); zoom = 1 }
    )
    $pad = 12; $labelH = 20; $headH = 28
    $width = ($crops | ForEach-Object { $_.rect.Width * $_.zoom } | Measure-Object -Maximum).Maximum + 2 * $pad
    $height = 40
    foreach ($c in $crops) { $height += $headH + 2 * ($labelH + $c.rect.Height * $c.zoom + $pad) }
    $bmp = [System.Drawing.Bitmap]::new($width, $height, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.Clear([System.Drawing.Color]::FromArgb(32, 32, 36))
    $g.TextRenderingHint = 'ClearTypeGridFit'
    $g.InterpolationMode = 'NearestNeighbor'
    $g.PixelOffsetMode = 'Half'
    $big = [System.Drawing.Font]::new('Segoe UI', 15, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
    $mid = [System.Drawing.Font]::new('Segoe UI', 13, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
    $small = [System.Drawing.Font]::new('Segoe UI', 11, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
    $g.DrawString('Inter static vs Inter Variable with optical sizes - real app, 100 % scale, dark theme, crisp hinting', $big, [System.Drawing.Brushes]::White, $pad, 10)
    $y = 40
    foreach ($c in $crops) {
        $g.DrawString($c.title, $mid, [System.Drawing.Brushes]::LightSkyBlue, $pad, $y + 6)
        $y += $headH
        foreach ($v in $variants) {
            $g.DrawString($v.label, $small, [System.Drawing.Brushes]::Gold, $pad, $y + 3)
            $y += $labelH
            $src = Join-Path $raw "$($c.shot)-$($v.id).png"
            $w = $c.rect.Width * $c.zoom; $h = $c.rect.Height * $c.zoom
            if (Test-Path -LiteralPath $src) {
                $img = [System.Drawing.Image]::FromFile($src)
                $g.DrawImage($img, [System.Drawing.Rectangle]::new($pad, $y, $w, $h), $c.rect, [System.Drawing.GraphicsUnit]::Pixel)
                $img.Dispose()
            } else {
                $g.DrawString("missing: $src", $small, [System.Drawing.Brushes]::Red, $pad, $y + 4)
            }
            $y += $h + $pad
        }
    }
    $g.Dispose()
    New-Item -ItemType Directory -Force (Split-Path $Out) | Out-Null
    $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host "wrote $Out"
}
finally {
    Remove-Item -LiteralPath $marker -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $data -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $demo -Recurse -Force -ErrorAction SilentlyContinue
    if ($KeepRaw) { Write-Host "raw captures kept in $raw" } else { Remove-Item -LiteralPath $raw -Recurse -Force -ErrorAction SilentlyContinue }
}
