<#
.SYNOPSIS
  Writes CHANGELOG.md from `git log`, grouped the same way as cliff.toml.

.DESCRIPTION
  Uses git-cliff when it is installed (`cargo install git-cliff`); otherwise groups the
  commits itself. Only reads the repository (git log / git tag); it never commits.
  Commits between two tags form one release; commits after the newest tag are "Unreleased".

.PARAMETER Output
  Target file (default: CHANGELOG.md in the repository root).

.PARAMETER NoCliff
  Use the built-in grouping even when git-cliff is available.
#>
[CmdletBinding()]
param(
    [string]$Output = (Join-Path (Split-Path -Parent $PSScriptRoot) 'CHANGELOG.md'),
    [switch]$NoCliff
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
# git prints UTF-8 subjects; decode them as such (not the OEM code page).
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)

if (-not $NoCliff -and (Get-Command git-cliff -ErrorAction SilentlyContinue)) {
    git -C $repo cliff --config (Join-Path $repo 'cliff.toml') -o $Output
    if ($LASTEXITCODE -ne 0) { throw "git-cliff failed" }
    Write-Host "CHANGELOG written by git-cliff: $Output"
    return
}

# Same order and names as cliff.toml's commit_parsers.
$groups = @(
    @{ Name = 'Milestones (M0–M13)'; Pattern = '^M\d' },
    @{ Name = 'Rigidity (R)'; Pattern = '^R\d' },
    @{ Name = 'Looks (B)'; Pattern = '^B\d' },
    @{ Name = 'Features (F)'; Pattern = '^(F\d|fonts)' },
    @{ Name = 'AI (A)'; Pattern = '^(A\d|AI )' },
    @{ Name = 'Distribution (D)'; Pattern = '^D\d' },
    @{ Name = 'Other features'; Pattern = '^(feat|feature)' },
    @{ Name = 'Fixes and security'; Pattern = '^(fix|security)' },
    @{ Name = 'Documentation'; Pattern = '^docs' },
    @{ Name = 'Other'; Pattern = '.*' }
)

function Get-Commits([string]$range) {
    $sep = [char]0x1f
    $args = @('-C', $repo, 'log', '--no-merges', '--reverse', "--format=%h$sep%ad$sep%s", '--date=short')
    if ($range) { $args += $range }
    foreach ($line in (& git @args)) {
        $p = $line -split $sep
        [pscustomobject]@{ Id = $p[0]; Date = $p[1]; Subject = $p[2] }
    }
}

function Format-Release([string]$title, $commits) {
    $out = [System.Text.StringBuilder]::new()
    [void]$out.AppendLine("## $title").AppendLine()
    foreach ($g in $groups) {
        $mine = @($commits | Where-Object { -not $_.Taken -and $_.Subject -match $g.Pattern })
        if ($mine.Count -eq 0) { continue }
        [void]$out.AppendLine("### $($g.Name)").AppendLine()
        foreach ($c in $mine) {
            $c | Add-Member -NotePropertyName Taken -NotePropertyValue $true -Force
            [void]$out.AppendLine("- $($c.Subject) ($($c.Id))")
        }
        [void]$out.AppendLine()
    }
    $out.ToString()
}

$tags = @(git -C $repo tag --sort=-creatordate)
$version = (Select-String -Path (Join-Path $repo 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value

$sb = [System.Text.StringBuilder]::new()
[void]$sb.AppendLine('# Changelog').AppendLine()
[void]$sb.AppendLine('All notable changes to Soyuz. Grouped by the commit prefix: M = the')
[void]$sb.AppendLine('original milestones, R = rigidity, B = looks, F = features, A = AI,')
[void]$sb.AppendLine('D = distribution. Generated from `git log` by `scripts/changelog.ps1` (or `git cliff`')
[void]$sb.AppendLine('with `cliff.toml`); commit hashes refer to the main branch.').AppendLine()

if ($tags.Count -eq 0) {
    $all = @(Get-Commits '')
    $last = ($all | Select-Object -Last 1).Date
    [void]$sb.Append((Format-Release "Unreleased ($version, last change $last)" $all))
} else {
    $head = @(Get-Commits "$($tags[0])..HEAD")
    if ($head.Count -gt 0) { [void]$sb.Append((Format-Release 'Unreleased' $head)) }
    for ($i = 0; $i -lt $tags.Count; $i++) {
        $range = if ($i + 1 -lt $tags.Count) { "$($tags[$i + 1])..$($tags[$i])" } else { $tags[$i] }
        $date = git -C $repo log -1 --format=%ad --date=short $tags[$i]
        [void]$sb.Append((Format-Release "$($tags[$i].TrimStart('v')) ($date)" @(Get-Commits $range)))
    }
}

$text = $sb.ToString().TrimEnd() + "`n"
[System.IO.File]::WriteAllText($Output, ($text -replace "`r`n", "`n"), [System.Text.UTF8Encoding]::new($false))
Write-Host "CHANGELOG written: $Output"
