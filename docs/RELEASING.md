# Releasing and code signing

## Windows Authenticode signing

`scripts/sign-windows.ps1` signs one file (`soyuz.exe` or an `.msi`) with signtool from the
newest installed Windows 10/11 SDK. It uses SHA-256 file digests and an RFC 3161 timestamp,
then verifies the result with `signtool verify /pa` and `Get-AuthenticodeSignature`.

The certificate comes from one of two places:

| Source | Used by | How |
|---|---|---|
| `CERT_PFX_BASE64` + `CERT_PASSWORD` env | CI (`.github/workflows/release.yml`, secrets `WINDOWS_CERT_PFX_BASE64` / `WINDOWS_CERT_PASSWORD`) | the PFX is written to a temp file and deleted afterwards |
| `-Thumbprint <sha1>` | local signing, hardware tokens / cloud HSM providers that expose the key in `Cert:\CurrentUser\My` | `signtool /sha1 … /s My` |

Options:
- `TIMESTAMP_URL` picks the timestamp authority. The default is `http://timestamp.digicert.com`.
  `none` skips timestamping; use it for local tests only, because an untimestamped signature
  stops validating when the certificate expires.
- `-AllowUntrustedRoot` accepts a chain that ends in a root this machine does not trust, such
  as a self-signed test certificate. Any other verification error still fails. Never use it
  in CI.

When the secrets are missing, `release.yml` publishes unsigned artifacts with a warning.

### Using a real certificate

1. **Buy an OV or EV code-signing certificate.**
   - Since 2023, CAs ship OV/EV keys only on hardware (a USB token or a cloud HSM such as
     Azure Trusted Signing, DigiCert KeyLocker or SSL.com eSigner).
   - EV builds SmartScreen reputation immediately. OV builds it over downloads.
2. **Sign locally.** With a token, the certificate appears in `Cert:\CurrentUser\My` and you
   sign with:

   ```powershell
   ./scripts/sign-windows.ps1 -Path target\release\soyuz.exe -Thumbprint <thumbprint>
   ```

3. **Sign in CI.** It needs a key the runner can use:
   - **Exportable PFX:** store it base64 in `WINDOWS_CERT_PFX_BASE64` with its password in
     `WINDOWS_CERT_PASSWORD`. This only works for certificates issued before the hardware
     rule.
   - **HSM:** replace the `signtool sign` line with the provider's tool (Azure: `signtool
     /dlib Azure.CodeSigning.Dlib.dll /dmdf metadata.json`; others ship similar dlibs). Keep
     the verify step.
4. **Order matters.** Sign `soyuz.exe` *before* it is packed into the MSI / zip, then sign
   the MSI. `release.yml` already does it in that order.
5. **Check the result:**
   - `Get-AuthenticodeSignature soyuz.exe` must report `Valid`.
   - The app's update checker uses the same `WinVerifyTrust` call
     (`soyuz_fs::integration::authenticode`). To see its verdict:

     ```powershell
     $env:SOYUZ_AUTHENTICODE_PROBE = 'path\to\file'
     cargo test -p soyuz-fs --lib authenticode_probe -- --ignored --nocapture
     ```

6. **MSIX:** signing needs the same certificate. Its subject must equal the `Publisher` in
   the MSIX manifest. This is not wired up yet.

### Pipeline validation with a temporary certificate (2026-10-09)

The whole path was exercised locally with a throwaway self-signed certificate. The steps:

1. `New-SelfSignedCertificate -Type CodeSigningCert -Subject "CN=Soyuz Signing Test (temporary)" -CertStoreLocation Cert:\CurrentUser\My -KeyExportPolicy Exportable -KeyAlgorithm RSA -KeyLength 3072 -HashAlgorithm SHA256 -NotAfter (Get-Date).AddDays(2)`
   (thumbprint `75D241FE6B23974D19B149F6270353496880CC86`).
   - The cmdlet also drops a copy into `Cert:\CurrentUser\CA`.
   - Nothing was added to any Root store.
2. **Exe through the cert store.** A copy of the release `soyuz.exe` was signed with
   `sign-windows.ps1 -Thumbprint … -AllowUntrustedRoot`, timestamped by the DigiCert TSA.
3. **MSI through the CI path.** The certificate was exported to a password-protected PFX and
   passed as `CERT_PFX_BASE64` / `CERT_PASSWORD`; the script then signed an MSI.
   - WiX v5 is not installed on this machine, so the real installer was not built.
   - Instead, a minimal MSI database was created through `msi.dll` automation
     (`WindowsInstaller.Installer`). It exercises the same MSI SIP in signtool.
4. **Negative check.** The same script *without* `-AllowUntrustedRoot` refused the result:
   `signtool verify` failed and the script exited with an error.
5. **Verification.** Results:

   | File | `Get-AuthenticodeSignature` | `signtool verify /pa` | app verifier (`WinVerifyTrust`) |
   |---|---|---|---|
   | soyuz.exe (signed copy) | `UnknownError`: chain terminated in an untrusted root; signer = test cert; timestamp = DigiCert SHA256 RSA4096 Timestamp Responder 2026 1 | fails only on the untrusted root | `Invalid(0x800B0109)` = `CERT_E_UNTRUSTEDROOT` |
   | soyuz-test.msi (signed) | same | same | `Invalid(0x800B0109)` |
   | target\release\soyuz.exe (unsigned) | — | — | `Unsigned` |

   This is the expected outcome for a self-signed certificate. The signature structure,
   digest and timestamp are valid, and only the trust anchor is missing. With a CA-issued
   certificate the same commands report `Valid`.
6. **Cleanup.** The certificate and its private key were deleted from `CurrentUser\My` and
   `CurrentUser\CA` (`Remove-Item -DeleteKey`).
   - A search of all CurrentUser and LocalMachine stores found **0** remaining copies of the
     thumbprint.
   - The PFX and the signed test files were deleted.

## Binaries per OS

- **Every push to `main`** (and manual runs of *CI*): the `package` job builds
  `soyuz-<v>-windows-x64.zip`, `soyuz-<v>-linux-x64.tar.gz` and
  `soyuz-<v>-macos-arm64.tar.gz` (plus `.sha256` files) with `scripts/package.ps1`; download
  them from the run's *Artifacts*.
- **Tags `v*`** (*Release*): Windows zip / portable zip / MSI (Authenticode-signed when the
  secrets exist), a Linux x64 tarball, and either the signed + notarized macOS universal
  app (Apple secrets) or an unsigned macOS arm64 tarball.
- Linux and macOS binaries are built and clippy-clean but **untested** on real machines;
  their test runs in CI are reported as warnings, not failures. Linux runners
  need `libasound2-dev` (audio preview via rodio / cpal / ALSA) besides the X11 / Wayland /
  GL / fontconfig dev packages.
