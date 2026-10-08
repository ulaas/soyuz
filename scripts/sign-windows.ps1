<#
.SYNOPSIS
    Authenticode-sign one file (exe or msi) with the PFX in $env:CERT_PFX_BASE64.

.DESCRIPTION
    Used by .github/workflows/release.yml when the signing secrets exist (skeleton: never
    run against a real certificate yet, see SECURITY.md). Env:
      CERT_PFX_BASE64  base64 of the .pfx
      CERT_PASSWORD    its password
      TIMESTAMP_URL    optional RFC 3161 TSA (default DigiCert)
#>
param([Parameter(Mandatory = $true)][string]$Path)

$ErrorActionPreference = "Stop"
if (-not $env:CERT_PFX_BASE64) { throw "CERT_PFX_BASE64 not set" }
$tsa = if ($env:TIMESTAMP_URL) { $env:TIMESTAMP_URL } else { "http://timestamp.digicert.com" }
$tmp = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$pfx = Join-Path $tmp "codesign-$PID.pfx"
[IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:CERT_PFX_BASE64))
try {
    $signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
        Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
    if (-not $signtool) { throw "signtool.exe not found (Windows SDK missing)" }
    & $signtool sign /f $pfx /p $env:CERT_PASSWORD /fd SHA256 /tr $tsa /td SHA256 /d "Soyuz" $Path
    if ($LASTEXITCODE -ne 0) { throw "signtool sign failed ($LASTEXITCODE)" }
    & $signtool verify /pa /v $Path
    if ($LASTEXITCODE -ne 0) { throw "signtool verify failed ($LASTEXITCODE)" }
}
finally {
    Remove-Item $pfx -Force -ErrorAction SilentlyContinue
}
