<#
.SYNOPSIS
    Pre-publish privacy check: fails when personal data shows up in tracked files, image
    metadata, commit metadata or anywhere in the git history.

.DESCRIPTION
    The terms it looks for are never stored in the repository. They come from:
      * this machine: user name, computer name, home folder path, volume labels;
      * an optional private list, one term per line (e.g. a work email address), in
        .git/info/public-denylist (inside .git, so it is never committed). Lines starting
        with # are comments.
    Commit author / committer emails must be in -AllowedEmails (default: the current
    `git config user.email`).

    Checks:
      1. tracked files (content and paths) at HEAD,
      2. text chunks / metadata of tracked PNG / JPEG / WebP images,
      3. author / committer identities and messages of every commit on every ref,
      4. every blob ever committed (full history, all refs),
      5. gitleaks over the full history, when gitleaks is on PATH (.gitleaks.toml).

    Exit code 0 = clean, 1 = something found (each finding is printed with where it is).

.EXAMPLE
    ./scripts/public-check.ps1
    ./scripts/public-check.ps1 -AllowedEmails me@example.com -Extra 'internal.example.net'
#>
[CmdletBinding()]
param(
    [string[]]$AllowedEmails = @(),
    [string[]]$Extra = @(),
    [switch]$SkipHistory
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
$findings = 0
function Find([string]$where, [string]$what) {
    $script:findings++
    Write-Host "FOUND  $where  ->  $what" -ForegroundColor Red
}

try {
    # ---- terms -----------------------------------------------------------------------
    $terms = [System.Collections.Generic.List[string]]::new()
    foreach ($t in @($env:USERNAME, $env:USER, $env:COMPUTERNAME, $env:HOSTNAME)) {
        if ($t -and $t.Length -ge 3) { $terms.Add($t) }
    }
    $userHome = [Environment]::GetFolderPath("UserProfile")
    if ($userHome) { $terms.Add($userHome); $terms.Add($userHome.Replace('\', '/')); $terms.Add($userHome.Replace('\', '\\')) }
    if ($IsWindows -or $env:OS -eq "Windows_NT") {
        try {
            Get-Volume -ErrorAction Stop | Where-Object { $_.FileSystemLabel -and $_.FileSystemLabel.Length -ge 3 } |
                ForEach-Object { $terms.Add($_.FileSystemLabel) }
        } catch { Write-Host "note: volume labels not checked ($($_.Exception.Message))" }
    }
    $deny = Join-Path (git rev-parse --git-common-dir) "info/public-denylist"
    if (Test-Path $deny) {
        Get-Content $deny | ForEach-Object { $_.Trim() } | Where-Object { $_ -and -not $_.StartsWith("#") } |
            ForEach-Object { $terms.Add($_) }
    } else {
        Write-Host "note: no private deny-list at $deny (add one term per line, e.g. a work email)"
    }
    $Extra | Where-Object { $_ } | ForEach-Object { $terms.Add($_) }
    # Generic words that would drown the report (e.g. a volume called "Data").
    $generic = @("data", "system", "windows", "local disk", "backup", "recovery", "new volume", "user", "admin",
        "source", "games", "game", "media", "work", "storage", "files", "music", "photos", "pictures", "video", "videos",
        "documents", "downloads", "projects", "temp", "apps", "programs", "dev", "home", "share", "shared", "archive")
    $terms = $terms | Where-Object { $generic -notcontains $_.ToLowerInvariant() } | Sort-Object -Unique
    Write-Host ("checking {0} terms (not printed)" -f @($terms).Count)

    if (-not $AllowedEmails) { $AllowedEmails = @((git config user.email)) }
    $AllowedEmails = $AllowedEmails | Where-Object { $_ } | ForEach-Object { $_.ToLowerInvariant() }

    # A term's position in the list, so reports never echo the term itself.
    function Label([string]$t) { "term #" + ([array]::IndexOf(@($terms), $t) + 1) }
    # Whole-word, case-insensitive match (a user name "ann" must not hit "annotation").
    $rx = @{}
    foreach ($t in $terms) {
        $rx[$t] = [regex]::new('(?<![A-Za-z0-9])' + [regex]::Escape($t) + '(?![A-Za-z0-9])', 'IgnoreCase')
    }
    function Has([string]$text, [string]$t) { $text -and $rx[$t].IsMatch($text) }

    # ---- 1. tracked files at HEAD ----------------------------------------------------
    $files = git ls-files
    foreach ($t in $terms) {
        $hits = git grep -I -l -i -F -e $t -- . 2>$null
        foreach ($h in $hits) {
            if (Has (Get-Content -Raw -LiteralPath (Join-Path $root $h)) $t) { Find "HEAD:$h" (Label $t) }
        }
        foreach ($f in $files) { if (Has $f $t) { Find "path:$f" (Label $t) } }
    }

    # ---- 2. image metadata -----------------------------------------------------------
    $images = $files | Where-Object { $_ -match '\.(png|jpe?g|webp)$' }
    foreach ($img in $images) {
        $bytes = [System.IO.File]::ReadAllBytes((Join-Path $root $img))
        # Metadata lives in text chunks / EXIF / XMP: scan the printable runs of the file.
        $text = [System.Text.Encoding]::Latin1.GetString($bytes)
        foreach ($t in $terms) {
            if (Has $text $t) { Find "image:$img" (Label $t) }
        }
        foreach ($chunk in @("tEXt", "iTXt", "zTXt", "eXIf", "Exif", "<x:xmpmeta")) {
            if ($text.Contains($chunk)) { Write-Host "note: $img has a $chunk metadata block (review or strip it)" -ForegroundColor Yellow }
        }
    }

    if (-not $SkipHistory) {
        # ---- 3. commit metadata ------------------------------------------------------
        $log = git log --all --format="%H%x1f%an%x1f%ae%x1f%cn%x1f%ce%x1f%B%x1e"
        foreach ($rec in (($log -join "`n") -split [char]0x1e)) {
            $p = $rec.Trim() -split [char]0x1f
            if ($p.Count -lt 6) { continue }
            foreach ($e in @($p[2], $p[4])) {
                if ($AllowedEmails -notcontains $e.ToLowerInvariant()) { Find "commit $($p[0].Substring(0, 10))" "email not in -AllowedEmails" }
            }
            foreach ($t in $terms) {
                if (Has ($p[1] + " " + $p[3] + " " + $p[5]) $t) {
                    Find "commit $($p[0].Substring(0, 10)) message/name" (Label $t)
                }
            }
        }

        # ---- 4. every blob in history --------------------------------------------------
        $blobs = git rev-list --all --objects | ForEach-Object { ($_ -split ' ', 2) } |
            Where-Object { $_ -match '^[0-9a-f]{40}$' }
        $types = $blobs | git cat-file --batch-check="%(objectname) %(objecttype)"
        $blobIds = $types | Where-Object { $_.EndsWith(" blob") } | ForEach-Object { $_.Split(' ')[0] }
        $paths = @{}
        git rev-list --all --objects | ForEach-Object {
            $kv = $_ -split ' ', 2
            if ($kv.Count -eq 2) { $paths[$kv[0]] = $kv[1] }
        }
        Write-Host ("scanning {0} blobs in history" -f @($blobIds).Count)
        foreach ($id in $blobIds) {
            $raw = git cat-file blob $id 2>$null | Out-String
            foreach ($t in $terms) {
                if (Has $raw $t) {
                    Find "history:$($paths[$id]) ($($id.Substring(0, 10)))" (Label $t)
                }
            }
        }
        $names = git log --all --format= --name-only | Sort-Object -Unique
        foreach ($n in $names) {
            foreach ($t in $terms) { if (Has $n $t) { Find "history path:$n" (Label $t) } }
        }

        # ---- 5. gitleaks -----------------------------------------------------------------
        $gl = Get-Command gitleaks -ErrorAction SilentlyContinue
        if ($gl) {
            & $gl.Source git --no-banner --redact -c .gitleaks.toml .
            if ($LASTEXITCODE -ne 0) { Find "gitleaks" "secrets found (see above)" }
        } else {
            Write-Host "note: gitleaks not on PATH; secret scan skipped (CI runs it)" -ForegroundColor Yellow
        }
    }

    if ($findings -gt 0) {
        Write-Host "$findings finding(s). Do not publish until they are resolved." -ForegroundColor Red
        exit 1
    }
    Write-Host "clean: nothing personal found" -ForegroundColor Green
}
finally {
    Pop-Location
}
