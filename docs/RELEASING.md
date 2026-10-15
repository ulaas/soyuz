# Releasing and code signing

## Windows Authenticode signing

`scripts/sign-windows.ps1` signs one file (`soyuz.exe`, `soyuz-tui.exe` or an `.msi`) with signtool from the
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
4. **Order matters.** Sign both executables, `soyuz.exe` and `soyuz-tui.exe`, *before* they
   are packed into the MSI / zip, then sign the MSI. `release.yml` already does it in that
   order.
5. **Check the result:**
   - `Get-AuthenticodeSignature soyuz.exe, soyuz-tui.exe` must report `Valid` for both.
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

Every package ships two executables, built together by
`cargo build --release -p soyuz-app -p soyuz-tui`: `soyuz` (the app; on Windows a
GUI-subsystem exe) and `soyuz-tui` (the terminal UI, crate `soyuz-tui`; console subsystem on
Windows, so it runs in the terminal that starts it; same icon and version resource). The zips
and tarballs hold both side by side, the MSI installs both into the install folder (covered
by its PATH option), the MSIX gives each an execution alias (`soyuz-tui.exe` is declared as
a console app, `desktop4:Subsystem="console"`), the rpm and the Flatpak put both in `bin/`,
and the signed macOS app carries `soyuz-tui` as a helper tool in `Contents/MacOS/`. The
Linux in-place updater (`services/updates.rs`) replaces both from the tarball.

- **Every push to `main`** (and manual runs of *CI*): the `package` job builds
  `soyuz-<v>-windows-x64.zip`, `soyuz-<v>-linux-x64.tar.gz` and
  `soyuz-<v>-macos-arm64.tar.gz` (plus `.sha256` files) with `scripts/package.ps1`; download
  them from the run's *Artifacts*.
- **Tags `v*`** (*Release*): Windows zip / portable zip / MSI (Authenticode-signed when the
  secrets exist), a Linux x86_64 PREFIX tarball and `.rpm` (see "Linux" below), and either the signed + notarized macOS universal
  app (Apple secrets) or an unsigned macOS arm64 tarball.
- Linux and macOS builds are supported and verified on real machines. In CI the Linux tests
  are a gate; the macOS test step is still `continue-on-error` (reported as warnings) until it
  has been confirmed green on hosted runners. Linux runners
  need `libasound2-dev` (audio preview via rodio / cpal / ALSA) besides the X11 / Wayland /
  GL / fontconfig dev packages.

## FFmpeg runtime

Soyuz loads FFmpeg as shared libraries at run time (`crates/soyuz-media`; Phase V). The
libraries are **our own LGPL-2.1 build**, made by `scripts/ffmpeg-build.sh` from the
tarballs pinned in `third_party/ffmpeg/pins.env` (FFmpeg 9.0.x, dav1d, libopus, libvpx,
the NVIDIA / AMD / Intel header-only SDKs; `third_party/ffmpeg/README.md` has the table),
and every artifact carries them:

| Package | Libraries | Licence texts (`LICENSE.md`, `BUILD.txt`, `SOURCES.txt`) |
|---|---|---|
| Windows zip / portable zip / MSI / MSIX | next to `soyuz.exe` (`avutil-61.dll`, `swresample-7.dll`, `swscale-10.dll`, `avcodec-63.dll`, `avformat-63.dll`) | `ffmpeg\` |
| Linux tarball / rpm / deb | `lib/soyuz/libav*.so.NN` (`%{_libdir}/soyuz/` in the rpm, `/usr/lib/soyuz/` in the deb); the exe looks in `<exe dir>/../lib/soyuz` and `../lib64/soyuz` | `share/licenses/soyuz/ffmpeg/` |
| macOS tarball (unsigned) | next to `soyuz` (`libav*.NN.dylib`, install names `@rpath/…` + `LC_RPATH @loader_path`) | `ffmpeg/` |
| macOS `Soyuz.app` (signed) | `Contents/Frameworks/` | `Contents/Resources/ffmpeg/` |
| Flatpak | none: the 25.08 runtime's FFmpeg 7.1 plus `codecs-extra` (searched first inside the sandbox) | — |

`scripts/package.ps1 -FfmpegDir <dir>`, `scripts/package.sh --ffmpeg-dir <dir>`,
`installer/windows/build-msi.ps1 -FfmpegDir` and `installer/msix/build-msix.ps1 -FfmpegDir`
take the folder (the output of the build script or an unpacked runtime archive) and fail
when it lacks the libraries or the three texts; without the option the package has no
runtime. `cargo deb` expects it unpacked in `ffmpeg-rt/` at the repository root.

### CI and the cache

- **`ffmpeg` job** (`ci.yml` and `release.yml`, three targets: `linux64` and `win64` on
  Ubuntu — Windows is a mingw-w64 cross build — and `macos-universal` on the arm64 runner,
  arm64 + x86_64 slices joined with `lipo`). `actions/cache` is keyed on
  `hashFiles('third_party/ffmpeg/pins.env', 'scripts/ffmpeg-build.sh')`: a pin or script
  change rebuilds (10–20 minutes per target), otherwise the cache is restored in seconds.
  A manual run (*Run workflow*) has a `rebuild_ffmpeg` switch that ignores the cache. The
  job uploads `ffmpeg-<target>` artifacts (the libraries, the texts, `include/` for the
  layout probe) and `ffmpeg-sources` (every input archive).
- **`media` job** (`ci.yml`): downloads the artifact and runs
  `SOYUZ_FFMPEG_DIR=… SOYUZ_MEDIA_TESTS=1 cargo test -p soyuz-media -p soyuz-appcore -p soyuz-jobs`
  on all three OSes (a missing runtime is a failure, not a skip), after the layout oracle:
  `third_party/ffmpeg/layout_probe.c` compiled against the built headers prints the C
  `offsetof()` of every field `soyuz-media` reads, and `SOYUZ_FFMPEG_LAYOUT=<json> cargo test
  -p soyuz-media -- layout` compares them with the Rust prefix structs.
- **`media-v7` job**: the same crate tests against Debian 13's FFmpeg 7.1 runtime packages
  (`container: debian:trixie`, `libavcodec61` et al.; the `-dev` headers are installed only
  afterwards, for the C probe). Ubuntu's own packages are not used (24.04 ships 6.1, which
  Soyuz refuses by design).
- The `package` job (pushes to `main` / `phase-*`) and every `release.yml` job take the
  artifact and pass `-FfmpegDir` / `--ffmpeg-dir`; the Linux release job also checks the
  tarball layout (`lib/soyuz/libavcodec.so.NN`, the texts) and that the rpm neither
  provides nor requires the bundled sonames (`__provides_exclude_from`, `__requires_exclude`
  in `soyuz.spec`; libva / libdrm stay ordinary requires of the VAAPI build).

### The separate `ffmpeg-runtime-<version>` release

The `publish` job never attaches FFmpeg files to the app release (an older Soyuz's update
check matches assets by OS / arch words, and `services::updates::pick_asset` only takes
`soyuz-<version>-…` names since 0.4.0). Instead it creates or updates the release
**`ffmpeg-runtime-<FFMPEG_VERSION>`** (`gh release create … --latest=false`, so
`/releases/latest` keeps pointing at the app) with:

- `ffmpeg-runtime-<ver>-windows-x64.zip`, `…-linux-x86_64.tar.xz`, `…-macos-universal.tar.xz`:
  the five libraries plus `LICENSE.md`, `BUILD.txt`, `SOURCES.txt`;
- every input source archive from `pins.env` (`ffmpeg-<ver>.tar.xz` and its `.asc`, dav1d,
  opus, libvpx, nv-codec-headers, libvpl, `AMF-headers-<commit>.tar.gz`), `BUILD-<target>.txt`,
  `LICENSE.md`, `SOURCES.txt` and `SHA256SUMS.txt`.

That release is the LGPL "complete corresponding source" offer and the place users (and the
in-app runtime download, Phase V4c) get replacement libraries from. It is published
immediately (not a draft); app releases stay drafts for review.

### Updating the pins

1. Pick the newest point release of the 9.0 line (https://ffmpeg.org/releases/; point
   releases are mostly security fixes for demuxers and decoders that run on untrusted
   files, so do not fall behind). A new *major* also needs a layout table in
   `soyuz-media/src/sys/layout.rs`, `FFMPEG_MAJORS` in `pins.env` and the version row in
   `Majors`.
2. Download the tarball and its `.asc`, verify (`gpg --verify`; key
   `FCF986EA15E6E293A5644F10B4322F04D67658D8`), `sha256sum` both, and write version, URL and
   sums into `pins.env`. Same for dav1d / opus / libvpx / nv-codec-headers / libvpl; AMF is
   pinned by tag **and commit** (sparse checkout; read the commit from
   `https://api.github.com/repos/GPUOpen-LibrariesAndSDKs/AMF/git/refs/tags/v<ver>`).
3. Push: the cache key changes, the `ffmpeg` job rebuilds, the `media` job proves the
   libraries on all three OSes. Note the bump in `CHANGELOG.md`; the next tagged app release
   publishes the new `ffmpeg-runtime-<ver>` release.

Locally, `scripts/ffmpeg-build.sh <target> <out dir> --dry-run` prints the full plan
(inputs, sums, every configure line) without touching the network, and the real build runs
on any Linux box with the tools listed in the script's header (Windows DLLs are
cross-compiled from Linux; macOS needs an Apple-silicon Mac with Xcode's command-line
tools and `brew install nasm meson ninja pkgconf`).

### Checking LGPL compliance of a build

- `BUILD.txt` must show `avcodec_configuration()` without `--enable-gpl`,
  `--enable-nonfree` or `--enable-version3` (the script fails otherwise; FFmpeg's own
  `LICENSE.md` lists what each switch would pull in).
- The libraries are loaded dynamically and replaceable (see the table above and the
  `disable-library-validation` entitlement in `installer/macos/entitlements.plist`), which
  is the LGPL 2.1 §6 "relinking" condition.
- The corresponding source is on the runtime release; `SOURCES.txt` lists every archive
  with its SHA-256 and `sha256sum -c SOURCES.txt` checks a download.
- `THIRD-PARTY-NOTICES.md` ("FFmpeg libraries") and `LICENSE.md` next to the libraries
  carry the LGPL-2.1 text, the dav1d / Opus / libvpx BSD notices, the MIT notices of the
  header-only SDKs and the patent note (wording from https://ffmpeg.org/legal.html); the
  About dialog names the loaded versions. Always spell the project name "FFmpeg".
- Windows DLLs may import only Windows system DLLs and each other (the script's objdump
  gate: libgcc, libstdc++, winpthread and zlib are linked statically); Linux libraries carry
  `RUNPATH $ORIGIN`; macOS dylibs are universal, `@rpath` + `@loader_path`, and signed
  (ad-hoc by the script, with the Developer ID by the release job, before the app).

## Changelog

`CHANGELOG.md` is generated from `git log`, grouped by commit prefix the same way as
`cliff.toml`: run `scripts/changelog.ps1` on Windows or `scripts/changelog.sh` on Linux /
macOS (both produce byte-identical output; `--no-cliff` / `-NoCliff` forces the built-in
grouping). Both use `git cliff` when it is installed, only read the repository and never
commit. Commits after the newest tag go under *Unreleased*; regenerate before tagging.
Hand edits to entries are overwritten on the next run; re-apply them afterwards.

## Linux

### Artifacts

`scripts/package.sh` (the Linux counterpart of `package.ps1`; `--help` lists its options)
builds `cargo build --release -p soyuz-app -p soyuz-tui --locked` and writes to `dist/`:

- `soyuz-<v>-linux-<arch>.tar.gz`: a relocatable PREFIX tree, `<arch>` from `uname -m`
  (`x86_64`, `aarch64`):

  ```
  soyuz-<v>-linux-x86_64/
    bin/soyuz
    bin/soyuz-tui
    share/applications/soyuz.desktop
    share/icons/hicolor/{16,22,24,32,48,64,128,256,512}x…/apps/soyuz.png
    share/metainfo/io.github.ulaas.Soyuz.metainfo.xml
    share/licenses/soyuz/{LICENSE,THIRD-PARTY-NOTICES.md}
    share/doc/soyuz/{README.md,USER_GUIDE.md,config.example.toml}
  ```

  There is no install script. Unpack it into a prefix:

  ```sh
  tar -C ~/.local --strip-components=1 -xf soyuz-0.1.0-linux-x86_64.tar.gz      # current user
  sudo tar -C /usr/local --strip-components=1 -xf soyuz-0.1.0-linux-x86_64.tar.gz # everyone
  ```

  To uninstall, delete the same paths (`tar -tzf … | sed 's|^[^/]*/||'` lists them).
- `--rpm` also builds `soyuz-<v>-1.<dist>.<arch>.rpm` with `installer/linux/soyuz.spec`
  (needs `rpmbuild`). The spec only repackages the tarball: nothing is compiled, so the RPM
  carries the same binary as the tarball. Libraries that winit / wgpu load with `dlopen()`
  (xkbcommon, Wayland, X11, Vulkan, EGL) are listed as `Requires` / `Recommends` by hand,
  because the automatic ELF scan does not see them.
- `.sha256` files and `SHA256SUMS-linux.txt`, as on Windows (the in-app update check
  matches `linux` + `x86_64` / `aarch64` in asset names).

`--dry-run` prints the plan; `--skip-build` repackages the existing `target/release/soyuz`.
The tarball is reproducible apart from the binary: sorted entries, owner 0, mtimes from the
last commit.

The *Release* workflow (`linux` job, Ubuntu runner) installs `rpm`, `desktop-file-utils`
and `appstream`, validates the desktop entry and the metainfo, runs
`scripts/package.sh --rpm` and checks the tarball layout. The RPM is built on Ubuntu, so
its release tag has no `.fcNN` suffix and it links against the runner's glibc: it installs
on distributions with that glibc or newer. A Debian package is possible with
`cargo deb -p soyuz-app` (`[package.metadata.deb]` in `crates/soyuz-app/Cargo.toml`; run
`cargo build --release -p soyuz-tui` first, its binary is one of the assets) but is not built
by CI.

### Desktop integration

- **Desktop entry:** `crates/soyuz-app/assets/soyuz.desktop` (one copy for every package).
  Its name matches the window's Wayland `app_id` / X11 `WM_CLASS` (`soyuz`, set in
  `crates/soyuz-app/src/main.rs`, also `StartupWMClass`), so docks and task switchers group
  the window under the launcher and show its icon. `Exec=soyuz %U` passes any number of
  paths as `file://` URIs: the app opens each folder as a tab (a file opens its folder with
  it selected). The `new-window`
  action runs `soyuz --new-window`. `MimeType=inode/directory` makes Soyuz *available* for
  folders; it does not make it the default. Check it with
  `desktop-file-validate crates/soyuz-app/assets/soyuz.desktop`.
- **AppStream:** `installer/linux/io.github.ulaas.Soyuz.metainfo.xml` (component id
  `io.github.ulaas.Soyuz`, `<launchable>` pointing at `soyuz.desktop`). Screenshots are
  the website's (`site/assets/shots/`, regenerated by `scripts/screenshots.sh`). Add a
  `<release>` entry for every tagged version. Check it with
  `appstreamcli validate --no-net installer/linux/io.github.ulaas.Soyuz.metainfo.xml`.
- **Icons:** `installer/linux/icons/hicolor/<n>x<n>/apps/soyuz.png`. The 16–256 px PNGs are
  the frames of `crates/soyuz-app/assets/icon.ico` (each rendered at its own size by
  `app_icon.rs`), 512 px is `icon.png`, and 22 px is a Lanczos downscale of `icon.png`.
  When the icon changes, regenerate `icon.ico` / `icon.png`
  (`cargo test -p soyuz-app --lib app_icon -- --ignored`) and then the PNGs, for example
  `magick 'crates/soyuz-app/assets/icon.ico[2]' PNG32:installer/linux/icons/hicolor/32x32/apps/soyuz.png`
  (frames 0–6 are 16, 24, 32, 48, 64, 128, 256 px).
- **Caches after installing into a prefix by hand.** RPM / deb triggers do this for system
  installs. For a tarball install, refresh them yourself if the launcher or icon does not
  show up:

  ```sh
  gtk-update-icon-cache -f -t ~/.local/share/icons/hicolor   # only if that dir has an index.theme
  update-desktop-database ~/.local/share/applications
  ```

  `gtk-update-icon-cache` refuses to run on a theme directory without `index.theme`. A
  per-user `~/.local/share/icons/hicolor` usually has none, and GTK / Qt then look the icon
  up without a cache, so the command is optional there. For `/usr/local`, run it on
  `/usr/local/share/icons/hicolor` only if that directory already has a cache file
  (`icon-theme.cache`); a stale cache hides new icons. Most desktops pick up new `.desktop`
  files on their own; GNOME Shell may need a log out and back in on X11.

### Default file manager and `org.freedesktop.FileManager1` (opt-in)

No package makes Soyuz the default file manager or claims the D-Bus name
`org.freedesktop.FileManager1`. Both stay the user's choice:

- **Default for folders.** The `inode/directory` default lives in the user's
  `$XDG_CONFIG_HOME/mimeapps.list` (`~/.config/mimeapps.list`), under
  `[Default Applications]`. Turning on "default file manager" in Settings writes
  `inode/directory=soyuz.desktop;` there (keeping the rest of the file) and turning it off
  restores the previous value. The equivalent by hand is
  `xdg-mime default soyuz.desktop inode/directory`.
- **"Show in folder" from other apps** (browsers' download lists, terminals, IDEs) calls
  `org.freedesktop.FileManager1` (`ShowFolders`, `ShowItems`, `ShowItemProperties`) on the
  session bus. With the same opt-in, a running Soyuz claims that name, and Soyuz writes a
  D-Bus activation file (`Exec="<path of the running soyuz>" --dbus-file-manager`) to
  `$XDG_DATA_HOME/dbus-1/services/org.freedesktop.FileManager1.service`
  (`~/.local/share/dbus-1/services/`), so D-Bus starts Soyuz when no file manager is
  running. Turning the option off deletes that file and releases the name. A user service
  file overrides the system one (`/usr/share/dbus-1/services/`, e.g. Nautilus or Dolphin)
  for that user only.
- `installer/linux/org.freedesktop.FileManager1.service.in` is a reference copy of that
  file (the app generates the real one; replace `@BINDIR@` to install it by hand). It is
  **not** installed by `package.sh` or `soyuz.spec`, on purpose: a package
  that ships it would take "show in folder" away from the desktop's own file manager for
  every user of the machine.

### Flatpak

`installer/linux/flatpak/io.github.ulaas.Soyuz.yml` builds Soyuz as the Flatpak app
`io.github.ulaas.Soyuz` on `org.freedesktop.Platform` 25.08 with the
`org.freedesktop.Sdk.Extension.rust-stable` extension. Not built by CI yet and not on
Flathub (the permissions below, `--filesystem=host` and `--talk-name=org.freedesktop.Flatpak`
in particular, would need Flathub's review).

**Prerequisites** (the SDK and the Rust extension are only needed to build; nothing here is
needed to run the repo's other packages):

```sh
flatpak install --user flathub org.freedesktop.Platform//25.08 org.freedesktop.Sdk//25.08 \
    org.freedesktop.Sdk.Extension.rust-stable//25.08
# flatpak-builder: `dnf install flatpak-builder` / `apt install flatpak-builder`, or
# `flatpak install flathub org.flatpak.Builder` (then `flatpak run org.flatpak.Builder …`)
```

**Offline crates.** flatpak-builder builds without network, so every crate in `Cargo.lock`
is listed in `installer/linux/flatpak/cargo-sources.json` (git-ignored, regenerate after
every `Cargo.lock` change) by `flatpak-cargo-generator` from
[flatpak-builder-tools](https://github.com/flatpak/flatpak-builder-tools) (`cargo/`; needs
Python with `aiohttp` and `tomlkit`):

```sh
FLATPAK_CARGO_GENERATOR=~/src/flatpak-builder-tools/cargo/flatpak-cargo-generator.py \
    installer/linux/flatpak/gen-cargo-sources.sh
# = python3 flatpak-cargo-generator.py Cargo.lock -o installer/linux/flatpak/cargo-sources.json
```

The crates are unpacked to `cargo/vendor` with a source-replacement config, and the manifest
points `CARGO_HOME` at `/run/build/soyuz/cargo` and runs
`cargo --offline build --release --locked -p soyuz-app -p soyuz-tui`. The vendored `accesskit_unix`
(`third_party/`) comes with the checkout. `rust-toolchain.toml` is not used (no rustup in
the SDK); the extension's cargo must be at least the workspace `rust-version`.

**Build, install, run** (from the repository root; the manifest's `dir` source is the
checkout itself, minus `target/`, `.git/`, `site/` and build folders):

```sh
flatpak-builder --user --install --force-clean build-dir \
    installer/linux/flatpak/io.github.ulaas.Soyuz.yml
flatpak run io.github.ulaas.Soyuz
flatpak run --command=sh io.github.ulaas.Soyuz        # a shell inside the sandbox
# a single-file bundle instead of installing:
flatpak-builder --repo=flatpak-repo --force-clean build-dir installer/linux/flatpak/io.github.ulaas.Soyuz.yml
flatpak build-bundle flatpak-repo soyuz.flatpak io.github.ulaas.Soyuz
```

Remove with `flatpak uninstall --user io.github.ulaas.Soyuz` (add `--delete-data` to remove
`~/.var/app/io.github.ulaas.Soyuz`).

**Names.** Flatpak exports only files named after the app id, so the manifest installs
`soyuz.desktop` as `io.github.ulaas.Soyuz.desktop` with `Icon=io.github.ulaas.Soyuz`, the
hicolor PNGs as `io.github.ulaas.Soyuz.png`, and the metainfo with its `<launchable>` changed
to the new desktop id. The window keeps Wayland `app_id` / `WM_CLASS` `soyuz` (one value for
every package), and the desktop entry keeps `StartupWMClass=soyuz`: GNOME Shell and KDE
Plasma use it to match the window to the launcher when app id and desktop id differ.
Validate the renamed files the way the build makes them:

```sh
desktop-file-validate build-dir/files/share/applications/io.github.ulaas.Soyuz.desktop
appstreamcli validate --no-net build-dir/files/share/metainfo/io.github.ulaas.Soyuz.metainfo.xml
```

**Permissions** (each is justified in the manifest): Wayland with X11 fallback, `ipc`, `dri`,
PulseAudio (audio previews), network (remote locations, AI providers, update check),
ssh-agent (SFTP), `--filesystem=host` (it is a file manager; covers `$HOME`, `/mnt`, `/media`,
`/run/media`, and the host OS read-only under `/run/host`), `xdg-run/gvfs` (desktop SMB /
SFTP / MTP mounts, read-write), `xdg-cache/thumbnails` (the shared thumbnail cache),
`org.freedesktop.secrets` (Secret Service), UDisks2 on the system bus (devices),
`org.freedesktop.FileManager1` (talk, for "show in folder"; own, used only when the opt-in
setting is on), `org.freedesktop.Flatpak` (`flatpak-spawn --host`, below).

**Sandbox-aware code** (all keyed on `/.flatpak-info`, so other packages are unchanged):

- `soyuz-fs` `launch::linux`: default-app opening uses the OpenURI portal (`OpenFile`,
  `OpenURI`); "show in folder" asks `FileManager1`, then the portal's `OpenDirectory`.
  External terminals and *Open with* entries run on the host through `flatpak-spawn --host`
  (the terminal list is probed on the host with `flatpak-spawn --host which`).
- Desktop-shared files use the host's XDG folders (`$HOST_XDG_*_HOME`, else `~/.config`,
  `~/.local/share`, `~/.cache`), not the app's private `~/.var/app/<id>/`: `mimeapps.list`
  and the D-Bus service file (default file manager), the home trash, `user-dirs.dirs`, GTK
  bookmarks, GTK / KDE theme files, the thumbnail cache and the per-user font folder.
  Registering as the default file manager writes `inode/directory=io.github.ulaas.Soyuz.desktop`
  and no desktop file of its own; the D-Bus service runs
  `/usr/bin/flatpak run --command=soyuz io.github.ulaas.Soyuz --dbus-file-manager`.
- Fonts: the host's fonts are scanned under `/run/host/fonts`, `/run/host/local-fonts` and
  `/run/host/user-fonts` besides the runtime's.
- Drives: the sandbox's own mounts (`/`, `/usr`, `/app`, `/etc`, Flatpak's folders) are
  hidden, and the disk holding home is listed as its `/home` mount.
- Updates: `InstallKind::Flatpak` only notifies.

**Known limitations:** the built-in terminal runs inside the sandbox (runtime tools, not the
host's); *Open with* lists only applications visible in the sandbox (other Flatpaks);
*Run as administrator* (`pkexec`) is unavailable; the CLI companion / MCP attach sockets live
in the sandbox's `$XDG_RUNTIME_DIR/soyuz/` (reach them with
`flatpak run --command=soyuz io.github.ulaas.Soyuz …`). Not yet built end to end: the
manifest was checked (YAML, renamed desktop entry and metainfo validated), but
flatpak-builder, the SDK and the Rust extension were not installed on the machine that wrote
it.

### Screenshots and demo tree on Linux

`scripts/demo-fixture.sh [ROOT] [BIG_FILE_MB]` builds the same demo tree as
`demo-fixture.ps1` (default `/tmp/SoyuzDemo`; needs python3 with Pillow).
`scripts/screenshots.sh` is the port of `screenshots.ps1`: portable mode next to the
binary, `SOYUZ_SCRIPT` sessions that save their own window with `shot:`, ImageMagick for
the downscale into `site/assets/shots/`, and the same cleanup. It sends no input to the
desktop. On Wayland the compositor places the window (`SOYUZ_WINDOW_POS` works only under
X11 / XWayland). The media shots (`video-player.png`, `media-tools.png`) and the demo
video (`Videos/Launch replay.mp4`) are made only by the PowerShell scripts so far: they
need `SOYUZ_FFMPEG_DIR` (the libraries, plus an `ffmpeg` program there or on the `PATH`)
and are skipped without it, keeping the previous pictures.
