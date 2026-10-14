# FFmpeg runtime (Phase V)

Soyuz loads the FFmpeg shared libraries at run time (`crates/soyuz-media`, `libloading`);
nothing from FFmpeg is compiled or linked into the Rust binaries. This folder holds what
the CI build of those libraries needs that is not a script:

| File | Role |
|---|---|
| `pins.env` | Versions, URLs and SHA-256 sums of every input tarball (shell syntax; AMF is pinned by tag + commit, see below), plus `FFMPEG_MAJORS`, the five library majors the build must produce. |
| `layout_probe.c` | Prints `offsetof()` of every struct field `soyuz-media` reads, compiled against the headers of the pinned tarball (and of Debian's `-dev` packages in the 7.1 CI leg); `SOYUZ_FFMPEG_LAYOUT=<its output> cargo test -p soyuz-media -- layout` compares the output with the `#[repr(C)]` prefix structs in `crates/soyuz-media/src/sys/layout.rs`. |
| `README.md` | This file. |

The build script is `scripts/ffmpeg-build.sh <linux64 | win64 | macos-arm64 | macos-x86_64 |
macos-universal> <out dir> [--dry-run]`; `docs/RELEASING.md`, "FFmpeg runtime", describes
the CI jobs, the cache, the packaging and the separate release.

## What is built

FFmpeg **9.0.x** (`avutil` 61, `swresample` 7, `swscale` 10, `avcodec` 63, `avformat` 63)
as shared libraries, configured

```
--enable-shared --disable-static --disable-gpl --disable-nonfree --disable-version3
--disable-programs --disable-doc --disable-avdevice --disable-avfilter --disable-network
--enable-pic --disable-debug --disable-sdl2 --disable-xlib --disable-vulkan --disable-opencl
--disable-bzlib --disable-lzma --disable-iconv --disable-libxml2 --disable-openssl
--disable-gnutls --disable-schannel --disable-securetransport --disable-libmfx
--enable-libdav1d --enable-libopus --enable-libvpx --pkg-config-flags=--static
```

(`libpostproc` no longer exists in 9.0) plus per target:

| Target | Extra configure flags | Post-processing |
|---|---|---|
| `win64` (mingw-w64 cross from Linux) | `--arch=x86_64 --target-os=mingw32 --cross-prefix=x86_64-w64-mingw32- --enable-cross-compile --disable-pthreads --enable-w32threads --enable-nvenc --enable-ffnvcodec --enable-amf --enable-libvpl --extra-ldflags="-static-libgcc -static-libstdc++ -Wl,-Bstatic"` | DLL import gate: `x86_64-w64-mingw32-objdump -p` may list only Windows system DLLs (`kernel32`, `user32`, `advapi32`, `ole32`, `gdi32`, `bcrypt`, `ws2_32`, the CRT) and the four sibling DLLs; libgcc, libstdc++ (libvpl's dispatcher is C++), winpthread and zlib (`libz-mingw-w64-dev`) are linked statically. D3D11 / D3D12 / DXVA2 / MediaFoundation / NVENC / AMF and the Intel runtime are loaded with `LoadLibrary` by FFmpeg itself. |
| `linux64` | `--enable-vaapi --enable-nvenc --enable-ffnvcodec` (libva / libva-drm / libdrm from the runner's `libva-dev`; no X11) | `patchelf --set-rpath '$ORIGIN'` so `dlopen()` of one library finds its siblings in the same folder. The libraries need `libva.so.2`, `libva-drm.so.2`, `libdrm.so.2` and `libz.so.1` at run time (the rpm picks them up as automatic requires). |
| `macos-arm64` / `macos-x86_64` | `--arch=<arch> --target-os=darwin --enable-cross-compile --cc="clang -arch <arch>" --enable-videotoolbox --enable-audiotoolbox --disable-appkit --disable-avfoundation --disable-coreimage --disable-metal --install-name-dir=@rpath`, `-mmacosx-version-min=11.0` | `install_name_tool -add_rpath @loader_path`; ad-hoc `codesign` (arm64 refuses unsigned code). |
| `macos-universal` | both slices above | `lipo -create` per library, then ad-hoc signed. |

All decoders, demuxers, parsers and bitstream filters stay enabled. Encoders are the native
LGPL ones (aac, flac, alac, pcm_*, mjpeg, png, mpeg4, prores_ks, …) plus libopus and libvpx
and the header-only hardware encoders (NVENC, AMF, QSV, VideoToolbox, VAAPI); there is no
x264 / x265 (GPL), so "MP4 / H.264" is a hardware-only preset. SVT-AV1 is a later addition.
No network protocols (`--disable-network`): every source is opened through Soyuz's own
`AVIOContext`.

dav1d (meson, static, PIC), libopus and libvpx (static, PIC) are built first into a private
prefix that is the only thing `pkg-config` sees (plus the system's `.pc` files on Linux for
libva); nv-codec-headers (`make install`), the AMF headers (`amf/public/include` →
`include/AMF`) and, on Windows, the libvpl dispatcher (cmake, static) follow; then FFmpeg.

### Output folder

```
<out dir>/
  avcodec-63.dll …            the five libraries (Linux: real files named by soname, no symlinks)
  LICENSE.md                  LGPL-2.1 text, upstream LICENSE.md, dav1d / Opus / libvpx / MIT notices, component table, patent note
  BUILD.txt                   every configure / meson / cmake command, toolchain versions, avcodec_configuration(), the pins
  SOURCES.txt                 `sha256sum -c`-style list of the inputs with their URLs
  include/                    the FFmpeg headers (CI compiles layout_probe.c against them; not shipped)
  sources/                    every input archive (uploaded as the ffmpeg-sources artifact; the runtime release)
```

The script fails when `avcodec_configuration()` in the built `libavcodec` contains
`--enable-gpl`, `--enable-nonfree` or `--enable-version3`, when it lacks
`--enable-libdav1d` / `--enable-libopus` / `--enable-libvpx` / `--disable-network`, when a
DLL imports a non-system DLL, or when the install does not produce the file names derived
from `FFMPEG_MAJORS`.

## Where the libraries go

Shipped inside every Soyuz artifact (Windows: next to the exe; Linux tarball / rpm / deb:
`lib/soyuz/`; macOS tarball: next to the binary; `Soyuz.app`: `Contents/Frameworks`;
Flatpak: none, the runtime's 7.1 + `codecs-extra` are used) and published on their own
release tag `ffmpeg-runtime-<FFMPEG_VERSION>` as `ffmpeg-runtime-<ver>-<os>-<arch>.{zip,tar.xz}`
(the five libraries, `LICENSE.md`, `BUILD.txt`, `SOURCES.txt`) together with every input
archive of `pins.env`, never on an app release, so an older Soyuz's update check cannot
mistake them for the app. Distro / Flatpak-runtime libraries of the 8.x (62) and 7.1 (61)
lines are accepted at run time as well (`search_dirs` in `crates/soyuz-media/src/lib.rs`
lists the folders in order; `SOYUZ_FFMPEG_DIR` goes first).

## Licences

| Component | Licence | Notes |
|---|---|---|
| FFmpeg | LGPL-2.1-or-later | Built without `--enable-gpl` / `--enable-nonfree` / `--enable-version3`; loaded dynamically; the user can replace the libraries (`[media] ffmpeg_dir`, `<config dir>/ffmpeg`, next to the exe). The complete corresponding source (the pinned archives, `BUILD.txt` with every command and toolchain version, the build script in this repository) is attached to the runtime release. No patches. |
| dav1d | BSD-2-Clause | Linked statically into `avcodec`. |
| libopus | BSD-3-Clause | Linked statically into `avcodec`. |
| libvpx | BSD-3-Clause | Linked statically into `avcodec` (its `PATENTS` grant is reproduced in `LICENSE.md`). |
| nv-codec-headers | MIT | Headers only; the driver's `nvEncodeAPI` is loaded at run time. |
| AMF | MIT | Headers only (`amf/public/include` of a sparse checkout pinned to the tag's commit); the driver's `amfrt64` is loaded at run time. |
| libvpl | MIT | Dispatcher, linked statically; the driver's runtime is loaded at run time. |

Soyuz itself stays MIT; `THIRD-PARTY-NOTICES.md` carries the full LGPL-2.1 text and this
component list (`scripts/notices/about.hbs`), and the same texts travel with the libraries
in `LICENSE.md`. Soyuz ships no patent licence: H.264, HEVC, AAC and others may be patented
in some jurisdictions (see `docs/USER_GUIDE.md`, "Codecs and patents", wording from
https://ffmpeg.org/legal.html). Always spell the project name "FFmpeg".

## Updating a pin

1. Pick the newest point release (FFmpeg: https://ffmpeg.org/releases/; keep the line, a
   new major needs a layout table and a CI leg in `soyuz-media` and a new `FFMPEG_MAJORS`).
2. Download it, check the signature where one exists (`gpg --verify ffmpeg-<v>.tar.xz.asc`,
   release key `FCF986EA15E6E293A5644F10B4322F04D67658D8`), `sha256sum` it, and put version,
   URL and sum into `pins.env`. AMF: new tag plus the tag's commit id
   (`https://api.github.com/repos/GPUOpen-LibrariesAndSDKs/AMF/git/refs/tags/v<ver>`).
3. Push: the `ffmpeg` CI job's cache key hashes `pins.env` and the build script, so the
   libraries are rebuilt and the `media` job runs the with-libs tests on all three OSes.
4. Note the bump in `CHANGELOG.md`; a new runtime release tag follows the next app release.

Development on a workstation never needs this build: `SOYUZ_FFMPEG_DIR=<folder with the
five libraries>` points the tests (and the app) at any 9.0 / 8.x / 7.1 shared build, and
`SOYUZ_MEDIA_TESTS=1` turns a missing runtime into a test failure instead of a skip.
