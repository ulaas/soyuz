<#
.SYNOPSIS
    Regenerates THIRD-PARTY-NOTICES.md (fonts, icons and every Rust crate with its licence).

.DESCRIPTION
    Needs cargo-about (`cargo install cargo-about --locked --features cli`). The static part
    (embedded fonts, egui fonts, icons, font licence texts) lives in scripts/notices/about.hbs;
    the crate list and licence texts come from Cargo.lock for the Windows, Linux and macOS
    targets. Run it after changing dependencies, and commit the result.
#>
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    & cargo about generate --locked --offline --fail -c scripts/notices/about.toml -o THIRD-PARTY-NOTICES.md scripts/notices/about.hbs
    if ($LASTEXITCODE -ne 0) { throw "cargo about failed ($LASTEXITCODE)" }
    Write-Host ("THIRD-PARTY-NOTICES.md: {0:N0} KiB" -f ((Get-Item THIRD-PARTY-NOTICES.md).Length / 1KB))
}
finally {
    Pop-Location
}
