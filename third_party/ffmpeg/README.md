# FFmpeg runtime (Phase V)

Soyuz loads the FFmpeg shared libraries at run time (`crates/soyuz-media`, `libloading`);
nothing from FFmpeg is compiled or linked into the Rust binaries. This folder holds what
the CI build of those libraries needs that is not a script:

| File | Role |
|---|---|
| `pins.env` | Versions, URLs and SHA-256 sums of every input tarball (shell syntax). |
| `layout_probe.c` | Prints `offsetof()` of every struct field `soyuz-media` reads, compiled against the headers of the pinned tarball (and of a distro's `-dev` package in the 7.1 CI leg); a Rust test compares the output with the `#[repr(C)]` prefix structs in `crates/soyuz-media/src/sys/layout.rs`. Compile-only in V0, run from V1a. |
| `README.md` | This file. |

The build script itself is `scripts/ffmpeg-build.sh` (V1b): `ffmpeg-build.sh <linux64 |
win64 | macos-universal> <out dir>`.

## What is built

FFmpeg **9.0.x** (`avutil` 61, `swresample` 7, `swscale` 10, `avcodec` 63, `avformat` 63)
as shared libraries, configured

```
--enable-shared --disable-static --disable-gpl --disable-nonfree --disable-programs
--disable-doc --disable-avdevice --disable-avfilter --disable-postproc --disable-network
--enable-pic --disable-debug --enable-libdav1d --enable-libopus --enable-libvpx
```

plus per target: Windows x64 `--arch=x86_64 --target-os=mingw32
--cross-prefix=x86_64-w64-mingw32- --extra-ldflags=-static-libgcc --enable-nvenc
--enable-amf --enable-libvpl`; Linux x64 `--extra-ldflags='-Wl,-rpath,$ORIGIN'
--enable-nvenc --enable-vaapi` (VAAPI only when `libva-dev` is on the runner); macOS
`--enable-videotoolbox`, one slice per `--arch=arm64` / `--arch=x86_64` joined with `lipo`,
install names rewritten to `@rpath/libX.N.dylib` + `LC_RPATH @loader_path`, ad-hoc signed.

All decoders, demuxers, parsers and bitstream filters stay enabled. Encoders are the native
LGPL ones (aac, flac, alac, pcm_*, mjpeg, png, mpeg4, prores_ks, …) plus libopus and libvpx
and the header-only hardware encoders (NVENC, AMF, QSV, VideoToolbox, VAAPI); there is no
x264 / x265 (GPL), so "MP4 / H.264" is a hardware-only preset. SVT-AV1 is a later addition.

The libraries are shipped inside every Soyuz artifact and published on their own release
tag `ffmpeg-runtime-<FFMPEG_VERSION>` as `ffmpeg-runtime-<ver>-<os>-<arch>.{zip,tar.xz}`
(the five libraries, `LICENSE.md`, `BUILD.txt`, `SOURCES.txt`) together with every input
tarball of `pins.env` — never on an app release, so an older Soyuz's update check cannot
mistake them for the app. Distro / Flatpak-runtime libraries of the 8.x (62) and 7.1 (61)
lines are accepted at run time as well.

## Licences

| Component | Licence | Notes |
|---|---|---|
| FFmpeg | LGPL-2.1-or-later | Built without `--enable-gpl` / `--enable-nonfree`; loaded dynamically; the user can replace the libraries (`[media] ffmpeg_dir`, `<config dir>/ffmpeg`). The complete corresponding source (the pinned tarballs, `BUILD.txt` with the configure line and toolchain versions, the build script) is attached to the runtime release. |
| dav1d | BSD-2-Clause | Linked statically into `avcodec`. |
| libopus | BSD-3-Clause | Linked statically into `avcodec`. |
| libvpx | BSD-3-Clause | Linked statically into `avcodec`. |
| nv-codec-headers | MIT | Headers only; the driver's `nvEncodeAPI` is loaded at run time. |
| AMF | MIT | Headers only (`amf/public/include`); the driver's `amfrt` is loaded at run time. |
| libvpl | MIT | Dispatcher, linked statically; the driver's runtime is loaded at run time. |

Soyuz itself stays MIT; `THIRD-PARTY-NOTICES.md` carries the full LGPL-2.1 text and this
component list (`scripts/notices/about.hbs`, V1b). Soyuz ships no patent licence: H.264,
HEVC, AAC and others may be patented in some jurisdictions (see `docs/USER_GUIDE.md`,
"Codecs and patents"). Always spell the project name "FFmpeg".

## Updating a pin

1. Pick the newest point release (FFmpeg: https://ffmpeg.org/releases/; keep the line, a
   new major needs a layout table and a CI leg in `soyuz-media`).
2. Download it, check the signature where one exists (`gpg --verify ffmpeg-<v>.tar.xz.asc`),
   `sha256sum` it, and put version, URL and sum into `pins.env`.
3. Push: the `ffmpeg` CI job's cache key hashes `pins.env` and the build script, so the
   libraries are rebuilt and the `media` job runs the with-libs tests on all three OSes.
4. Note the bump in `CHANGELOG.md`; a new runtime release tag follows the next app release.

Development on a workstation never needs this build: `SOYUZ_FFMPEG_DIR=<folder with the
five libraries>` points the tests (and the app) at any 9.0 / 8.x / 7.1 shared build, and
`SOYUZ_MEDIA_TESTS=1` turns a missing runtime into a test failure instead of a skip.
