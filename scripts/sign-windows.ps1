<#
.SYNOPSIS
    Authenticode-sign one file (exe or msi) with signtool.

.DESCRIPTION
    Used by .github/workflows/release.yml when the signing secrets exist, and locally to
    validate the pipeline (see docs/RELEASING.md). The certificate comes from, in order:
      -Thumbprint        a certificate with a private key in Cert:\CurrentUser\My
      CERT_PFX_BASE64    base64 of a .pfx (CI secret), with CERT_PASSWORD
    Env:
      TIMESTAMP_URL      RFC 3161 TSA (default http://timestamp.digicert.com); "none" skips
                         timestamping (local tests only: an untimestamped signature stops
                         validating when the certificate expires)
    -AllowUntrustedRoot accepts a signature whose chain ends in a root this machine does
    not trust (a self-signed test certificate); every other verification error still
    fails. Never use it in CI.
#>
param(
    [Parameter(Mandatory = $true)][string]$Path,
    [string]$Thumbprint,
    [switch]$AllowUntrustedRoot
)

$ErrorActionPreference = "Stop"
$tsa = if ($env:TIMESTAMP_URL) { $env:TIMESTAMP_URL } else { "http://timestamp.digicert.com" }
$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue |
    Sort-Object { [version]($_.Directory.Parent.Name) } -Descending | Select-Object -First 1 -ExpandProperty FullName
if (-not $signtool) { throw "signtool.exe not found (Windows SDK missing)" }

$pfx = $null
$certArgs = @()
if ($Thumbprint) {
    $certArgs = @('/sha1', $Thumbprint, '/s', 'My')
} else {
    if (-not $env:CERT_PFX_BASE64) { throw "CERT_PFX_BASE64 not set (or pass -Thumbprint)" }
    $tmp = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
    $pfx = Join-Path $tmp "codesign-$PID.pfx"
    [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:CERT_PFX_BASE64))
    $certArgs = @('/f', $pfx, '/p', $env:CERT_PASSWORD)
}
try {
    $tsArgs = if ($tsa -eq 'none') { @() } else { @('/tr', $tsa, '/td', 'SHA256') }
    & $signtool sign @certArgs /fd SHA256 @tsArgs /d "Soyuz" $Path
    if ($LASTEXITCODE -ne 0) { throw "signtool sign failed ($LASTEXITCODE)" }
    $out = & $signtool verify /pa /v $Path 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) {
        $untrustedRoot = $out -match 'terminated in a root\s+certificate which is not trusted'
        if (-not ($AllowUntrustedRoot -and $untrustedRoot)) {
            Write-Host $out
            throw "signtool verify failed ($LASTEXITCODE)"
        }
        Write-Warning "signature is valid but its root is not trusted on this machine (test certificate)"
    }
    $sig = Get-AuthenticodeSignature -LiteralPath $Path
    Write-Host ("signed {0}: {1} ({2}), signer {3}" -f $Path, $sig.Status, $sig.StatusMessage, $sig.SignerCertificate.Subject)
}
finally {
    if ($pfx) { Remove-Item $pfx -Force -ErrorAction SilentlyContinue }
}
