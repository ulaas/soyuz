#!/usr/bin/env bash
# Build the FFmpeg runtime that Soyuz ships (Phase V): five LGPL shared libraries, built from
# the tarballs pinned in third_party/ffmpeg/pins.env, with dav1d, libopus and libvpx linked
# in statically and the header-only hardware encoders of each OS.
#
#   scripts/ffmpeg-build.sh <target> <out-dir> [--dry-run] [--work <dir>] [--downloads <dir>]
#                           [--jobs <n>] [--keep]
#
#   target     linux64 | win64 | macos-arm64 | macos-x86_64 | macos-universal
#              linux64 and win64 build on Linux (win64 with the mingw-w64 cross toolchain);
#              the macOS targets build on an Apple-silicon Mac, macos-universal joins an
#              arm64 and an x86_64 slice with lipo.
#   out-dir    receives:
#                the five libraries        avcodec-63.dll | libavcodec.so.63 | libavcodec.63.dylib …
#                LICENSE.md                LGPL-2.1 + the BSD / MIT notices + the component list
#                BUILD.txt                 exact configure lines, toolchain versions, pins
#                SOURCES.txt               URL + SHA-256 of every input
#                include/                  the FFmpeg headers (for third_party/ffmpeg/layout_probe.c
#                                          in CI; not shipped)
#                sources/                  every input archive (attached to the runtime release)
#   --dry-run  print the plan (inputs, sums, tools, configure lines) and exit; no network, no disk
#   --work     scratch folder (default: <out-dir>/../ffmpeg-work-<target>), removed unless --keep
#   --downloads where the input archives are kept between runs (default: <work>/downloads)
#   --jobs     parallel make jobs (default: all cores)
#
# Tools: curl, tar, sha256sum (or shasum), make, nasm, meson + ninja (dav1d), cmake (libvpl,
# win64 only), pkg-config, a C/C++ compiler; patchelf + readelf (linux64),
# x86_64-w64-mingw32-gcc + objdump (win64), clang + lipo + install_name_tool + codesign (macOS).
# gpg is optional: when present, the FFmpeg tarball's .asc signature is checked against the
# FFmpeg release key; the SHA-256 is checked always.
#
# Licence gates: the build fails when the libraries' avcodec_configuration() mentions
# --enable-gpl, --enable-nonfree or --enable-version3, and (win64) when a DLL imports anything
# but Windows system DLLs and the four sibling libraries (no libwinpthread-1.dll, no
# libgcc_s_seh-1.dll, no libstdc++-6.dll: those are linked statically).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
pins="$root/third_party/ffmpeg/pins.env"
# FFmpeg release signing key (https://ffmpeg.org/download.html, "FFmpeg Release Signing Key").
ffmpeg_key_fpr="FCF986EA15E6E293A5644F10B4322F04D67658D8"
# Windows DLLs the libraries may import (case-insensitive, besides each other): the CRT, the
# kernel, COM / GDI (libvpl's dispatcher), bcrypt (random numbers), ws2_32 (unused with
# --disable-network, harmless). d3d11 / dxgi / d3d12 / dxva2 / mfplat / nvEncodeAPI64 / amfrt64
# and the Intel runtime are loaded with LoadLibrary at run time and never appear here.
win_allowed_imports='^(kernel32|user32|advapi32|ole32|gdi32|bcrypt|ws2_32|msvcrt|ucrtbase|api-ms-win-crt-[a-z0-9-]+)\.dll$'

usage() { sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; }

target="" out="" dry_run=0 work="" downloads="" jobs="" keep=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run) dry_run=1 ;;
        --work) work="${2:?--work needs a value}"; shift ;;
        --work=*) work="${1#*=}" ;;
        --downloads) downloads="${2:?--downloads needs a value}"; shift ;;
        --downloads=*) downloads="${1#*=}" ;;
        --jobs) jobs="${2:?--jobs needs a value}"; shift ;;
        --jobs=*) jobs="${1#*=}" ;;
        --keep) keep=1 ;;
        -h | --help) usage; exit 0 ;;
        -*) echo "ffmpeg-build.sh: unknown option '$1' (see --help)" >&2; exit 2 ;;
        *)
            if [[ -z "$target" ]]; then target="$1"
            elif [[ -z "$out" ]]; then out="$1"
            else echo "ffmpeg-build.sh: unexpected argument '$1'" >&2; exit 2
            fi ;;
    esac
    shift
done
[[ -n "$target" && -n "$out" ]] || { usage >&2; exit 2; }
case "$target" in
    linux64 | win64 | macos-arm64 | macos-x86_64 | macos-universal) ;;
    *) echo "ffmpeg-build.sh: unknown target '$target'" >&2; exit 2 ;;
esac

# A Windows checkout may have CRLF endings (core.autocrlf); strip them before sourcing.
# shellcheck source=../third_party/ffmpeg/pins.env
source <(tr -d '' <"$pins")
for v in FFMPEG_VERSION FFMPEG_URL FFMPEG_SHA256 FFMPEG_ASC_URL FFMPEG_ASC_SHA256 FFMPEG_MAJORS \
         DAV1D_VERSION DAV1D_URL DAV1D_SHA256 OPUS_VERSION OPUS_URL OPUS_SHA256 \
         LIBVPX_VERSION LIBVPX_URL LIBVPX_SHA256 NVCODEC_VERSION NVCODEC_URL NVCODEC_SHA256 \
         AMF_VERSION AMF_GIT AMF_COMMIT LIBVPL_VERSION LIBVPL_URL LIBVPL_SHA256; do
    [[ -n "${!v:-}" ]] || { echo "ffmpeg-build.sh: $v is not set in $pins" >&2; exit 1; }
done

# ---------------------------------------------------------------------------------------------
# Per-target settings
# ---------------------------------------------------------------------------------------------
if [[ -z "$jobs" ]]; then
    jobs="$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)"
fi
[[ -n "$work" ]] || work="$(cd "$(dirname "$out")" 2>/dev/null && pwd || echo "$(dirname "$out")")/ffmpeg-work-$target"
[[ -n "$downloads" ]] || downloads="$work/downloads"

# The options every target shares. Nothing that needs --enable-gpl / --enable-nonfree /
# --enable-version3; no programs, filters, devices or network protocols (untrusted files are
# demuxed in-process, so the smaller the surface the better); dav1d, libopus and libvpx
# statically; every decoder, demuxer, parser and bitstream filter enabled (the default).
common_configure=(
    --enable-shared --disable-static
    --disable-gpl --disable-nonfree --disable-version3
    --disable-programs --disable-doc --disable-htmlpages --disable-manpages
    --disable-avdevice --disable-avfilter
    --disable-network
    --enable-pic --disable-debug
    --disable-sdl2 --disable-xlib --disable-vulkan --disable-opencl
    --disable-bzlib --disable-lzma --disable-iconv --disable-libxml2
    --disable-openssl --disable-gnutls --disable-schannel --disable-securetransport
    --disable-libmfx
    --enable-libdav1d --enable-libopus --enable-libvpx
    --pkg-config=pkg-config --pkg-config-flags=--static
)

# `settings <slice>` fills the variables below for one build slice (one architecture).
settings() {
    local slice="$1"
    host="" cross_prefix="" vpx_target="" meson_cross="" cmake_cross=() ffmpeg_target=()
    cc="cc" cxx="c++" lib_ext="" lib_dir="lib" deployment_target=""
    opus_host=() cflags="-O2" ldflags=""
    case "$slice" in
        linux64)
            lib_ext="so"; vpx_target="x86_64-linux-gcc"
            cc="${CC:-gcc}"; cxx="${CXX:-g++}"
            # VAAPI (libva, libva-drm, libdrm; no X11) and NVENC (ffnvcodec headers, the driver
            # is loaded at run time). The $ORIGIN rpath is set with patchelf after the install.
            ffmpeg_target=(--enable-vaapi --enable-nvenc --enable-ffnvcodec)
            ;;
        win64)
            lib_ext="dll"; lib_dir="bin"; host="x86_64-w64-mingw32"; cross_prefix="x86_64-w64-mingw32-"
            cc="${cross_prefix}gcc"; cxx="${cross_prefix}g++"; vpx_target="x86_64-win64-gcc"
            opus_host=(--host="$host")
            meson_cross="$work/cross-mingw.meson"
            cmake_cross=(-DCMAKE_SYSTEM_NAME=Windows -DCMAKE_SYSTEM_PROCESSOR=AMD64
                -DCMAKE_C_COMPILER="$cc" -DCMAKE_CXX_COMPILER="$cxx"
                -DCMAKE_RC_COMPILER="${cross_prefix}windres" -DCMAKE_FIND_ROOT_PATH_MODE_PROGRAM=NEVER)
            # -static-libgcc -static-libstdc++ -Wl,-Bstatic: libgcc, libstdc++ (libvpl's
            # dispatcher is C++), winpthread and zlib are linked in, so the DLLs import only
            # Windows system DLLs (the gate below checks). Win32 threads, not winpthread.
            ldflags="-static-libgcc -static-libstdc++ -Wl,-Bstatic"
            ffmpeg_target=(--arch=x86_64 --target-os=mingw32 --cross-prefix="$cross_prefix" --enable-cross-compile
                --disable-pthreads --enable-w32threads
                --enable-nvenc --enable-ffnvcodec --enable-amf --enable-libvpl
                --extra-ldflags="$ldflags")
            ;;
        macos-arm64 | macos-x86_64)
            lib_ext="dylib"; deployment_target="11.0"
            local arch="${slice#macos-}"
            cc="clang -arch $arch"; cxx="clang++ -arch $arch"
            cflags="-O2 -mmacosx-version-min=$deployment_target"
            ldflags="-mmacosx-version-min=$deployment_target"
            vpx_target="${arch}-darwin20-gcc"
            opus_host=(--host="${arch}-apple-darwin")
            meson_cross="$work/cross-$arch.meson"
            # Install names are @rpath/libX.N.dylib; LC_RPATH @loader_path is added afterwards.
            ffmpeg_target=(--arch="$arch" --target-os=darwin --enable-cross-compile --cc="$cc"
                --enable-videotoolbox --enable-audiotoolbox
                --disable-appkit --disable-avfoundation --disable-coreimage --disable-metal
                --install-name-dir=@rpath
                --extra-cflags="$cflags" --extra-ldflags="$ldflags")
            ;;
    esac
}

# The five library file names in load order for a slice, from the majors in pins.env
# ("avutil swresample swscale avcodec avformat").
lib_names() {
    local slice="$1" i=0 names=(avutil swresample swscale avcodec avformat) majors
    read -r -a majors <<<"$FFMPEG_MAJORS"
    for n in "${names[@]}"; do
        case "$slice" in
            win64) echo "$n-${majors[$i]}.dll" ;;
            macos-*) echo "lib$n.${majors[$i]}.dylib" ;;
            *) echo "lib$n.so.${majors[$i]}" ;;
        esac
        i=$((i + 1))
    done
}

inputs() {
    # name|version|url|sha256
    echo "ffmpeg|$FFMPEG_VERSION|$FFMPEG_URL|$FFMPEG_SHA256"
    echo "ffmpeg-asc|$FFMPEG_VERSION|$FFMPEG_ASC_URL|$FFMPEG_ASC_SHA256"
    echo "dav1d|$DAV1D_VERSION|$DAV1D_URL|$DAV1D_SHA256"
    echo "opus|$OPUS_VERSION|$OPUS_URL|$OPUS_SHA256"
    echo "libvpx|$LIBVPX_VERSION|$LIBVPX_URL|$LIBVPX_SHA256"
    echo "nv-codec-headers|$NVCODEC_VERSION|$NVCODEC_URL|$NVCODEC_SHA256"
    echo "libvpl|$LIBVPL_VERSION|$LIBVPL_URL|$LIBVPL_SHA256"
}

slices=("$target")
[[ "$target" == macos-universal ]] && slices=(macos-arm64 macos-x86_64)

# ---------------------------------------------------------------------------------------------
# Dry run
# ---------------------------------------------------------------------------------------------
if [[ $dry_run -eq 1 ]]; then
    echo "target:     $target"
    echo "out:        $out"
    echo "work:       $work"
    echo "downloads:  $downloads"
    echo "jobs:       $jobs"
    echo "inputs (URL, SHA-256):"
    while IFS='|' read -r name ver url sum; do
        printf '  %-18s %-8s %s\n  %-18s %-8s sha256 %s\n' "$name" "$ver" "$url" "" "" "$sum"
    done < <(inputs)
    printf '  %-18s %-8s git %s @ %s (amf/public/include only, sparse checkout)\n' "amf" "$AMF_VERSION" "$AMF_GIT" "$AMF_COMMIT"
    echo "ffmpeg majors (avutil swresample swscale avcodec avformat): $FFMPEG_MAJORS"
    for slice in "${slices[@]}"; do
        settings "$slice"
        echo "slice $slice:"
        echo "  libraries: $(lib_names "$slice" | tr '\n' ' ')"
        echo "  dav1d:     meson setup --default-library=static -Denable_tools=false -Denable_tests=false -Denable_examples=false -Denable_docs=false${meson_cross:+ --cross-file $meson_cross}"
        echo "  opus:      ./configure --disable-shared --enable-static --disable-extra-programs --disable-doc --with-pic ${opus_host[*]+"${opus_host[*]}"}"
        echo "  libvpx:    ./configure --target=$vpx_target --disable-shared --enable-static --enable-pic --enable-vp8 --enable-vp9 --enable-vp9-highbitdepth --disable-examples --disable-tools --disable-docs --disable-unit-tests --disable-install-bins --disable-install-docs"
        echo "  headers:   nv-codec-headers (make install), AMF amf/public/include -> include/AMF"
        [[ "$slice" == win64 ]] && echo "  libvpl:    cmake -DBUILD_SHARED_LIBS=OFF -DBUILD_TESTS=OFF ${cmake_cross[*]+"${cmake_cross[*]}"}"
        echo "  ffmpeg:    ./configure ${common_configure[*]} ${ffmpeg_target[*]}"
    done
    [[ "$target" == macos-universal ]] && echo "universal: lipo -create <arm64> <x86_64> per library"
    echo "gates: avcodec_configuration() without --enable-gpl / --enable-nonfree / --enable-version3$([[ "$target" == win64 ]] && echo '; DLL imports limited to Windows system DLLs')"
    echo "outputs: the libraries, LICENSE.md, BUILD.txt, SOURCES.txt, include/, sources/"
    echo "dry run: nothing downloaded or written"
    exit 0
fi

# ---------------------------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------------------------
log() { printf '\n==> %s\n' "$*"; }
die() { echo "ffmpeg-build.sh: $*" >&2; exit 1; }
need() { for t in "$@"; do command -v "$t" >/dev/null 2>&1 || die "missing tool: $t"; done; }

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
    else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# fetch <url> <sha256> -> prints the local path; downloads only when the verified file is absent.
fetch() {
    local url="$1" sum="$2" file="$downloads/$(basename "$1")"
    if [[ -f "$file" ]] && [[ "$(sha256_of "$file")" == "$sum" ]]; then
        echo "$file"; return
    fi
    rm -f "$file"
    curl -sSfL --retry 3 -o "$file.part" "$url"
    local got
    got="$(sha256_of "$file.part")"
    [[ "$got" == "$sum" ]] || { rm -f "$file.part"; die "SHA-256 mismatch for $url: expected $sum, got $got"; }
    mv "$file.part" "$file"
    echo "$file"
}

# AMF: the tag archive is several hundred MB (samples, binaries); only amf/public/include
# is needed, so it is a sparse checkout pinned to the tag's commit (verified after the
# clone). The headers are archived into downloads/ for SOURCES.txt and the release.
fetch_amf() {
    local dir="$downloads/AMF-$AMF_COMMIT" tarball="$downloads/AMF-headers-$AMF_COMMIT.tar.gz"
    if [[ ! -f "$tarball" ]]; then
        rm -rf "$dir"
        git clone --quiet --depth 1 --filter=blob:none --sparse --branch "v$AMF_VERSION" "$AMF_GIT" "$dir"
        git -C "$dir" sparse-checkout set amf/public/include
        local head
        head="$(git -C "$dir" rev-parse HEAD)"
        [[ "$head" == "$AMF_COMMIT" ]] || die "AMF v$AMF_VERSION is at $head, pins.env says $AMF_COMMIT"
        tar -C "$dir" -czf "$tarball.part" LICENSE.txt amf/public/include
        mv "$tarball.part" "$tarball"
    fi
    echo "$tarball"
}

extract() { # <archive> <dest dir>: the archive's single top-level folder lands as <dest dir>
    local archive="$1" dest="$2" tmp="$2.extract"
    rm -rf "$dest" "$tmp"; mkdir -p "$tmp"
    tar -xf "$archive" -C "$tmp"
    local top
    top="$(find "$tmp" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
    [[ -n "$top" ]] || die "no folder in $archive"
    mv "$top" "$dest"; rm -rf "$tmp"
}

tool_version() { # first line of `<tool> --version`, or "(not found)"
    "$@" --version 2>/dev/null | head -n 1 || echo "(not found)"
}

# ---------------------------------------------------------------------------------------------
# Download everything first (so a missing input fails before an hour of compiling)
# ---------------------------------------------------------------------------------------------
need curl tar make git pkg-config meson ninja nasm
case "$target" in
    linux64) need gcc patchelf readelf strings ;;
    win64) need x86_64-w64-mingw32-gcc x86_64-w64-mingw32-g++ x86_64-w64-mingw32-objdump x86_64-w64-mingw32-strings cmake ;;
    macos-*) need clang lipo install_name_tool codesign strings ;;
esac
mkdir -p "$downloads" "$work" "$out"
out="$(cd "$out" && pwd)"
work="$(cd "$work" && pwd)"
downloads="$(cd "$downloads" && pwd)"

log "inputs -> $downloads"
ffmpeg_tar="$(fetch "$FFMPEG_URL" "$FFMPEG_SHA256")"
ffmpeg_asc="$(fetch "$FFMPEG_ASC_URL" "$FFMPEG_ASC_SHA256")"
dav1d_tar="$(fetch "$DAV1D_URL" "$DAV1D_SHA256")"
opus_tar="$(fetch "$OPUS_URL" "$OPUS_SHA256")"
vpx_tar="$(fetch "$LIBVPX_URL" "$LIBVPX_SHA256")"
nvcodec_tar="$(fetch "$NVCODEC_URL" "$NVCODEC_SHA256")"
vpl_tar="$(fetch "$LIBVPL_URL" "$LIBVPL_SHA256")"
amf_tar="$(fetch_amf)"

# The signature is a second, independent check of the FFmpeg tarball; the key comes from a
# keyserver, which may be unreachable, so a missing key is a warning and a bad signature an error.
signature_note="not checked (gpg not installed)"
if command -v gpg >/dev/null 2>&1; then
    export GNUPGHOME="$work/gnupg"; rm -rf "$GNUPGHOME"; mkdir -p "$GNUPGHOME"; chmod 700 "$GNUPGHOME"
    if gpg --batch --quiet --keyserver hkps://keys.openpgp.org --recv-keys "$ffmpeg_key_fpr" 2>/dev/null \
        || gpg --batch --quiet --keyserver hkps://keyserver.ubuntu.com --recv-keys "$ffmpeg_key_fpr" 2>/dev/null; then
        # GNUPGHOME is empty and only the full pinned fingerprint was imported above; rely on
        # gpg's verification result rather than parsing version-dependent status output.
        if gpg --batch --quiet --verify "$ffmpeg_asc" "$ffmpeg_tar" 2>/dev/null; then
            signature_note="valid, signed by $ffmpeg_key_fpr"
        else
            die "the signature of $(basename "$ffmpeg_tar") does not verify against the FFmpeg release key $ffmpeg_key_fpr"
        fi
    else
        signature_note="not checked (the FFmpeg release key $ffmpeg_key_fpr could not be fetched from a keyserver)"
        echo "::warning::$signature_note"
    fi
fi
echo "FFmpeg tarball signature: $signature_note"

# ---------------------------------------------------------------------------------------------
# One slice
# ---------------------------------------------------------------------------------------------
build_lines=()   # BUILD.txt body, accumulated over the slices

build_slice() { # <slice> <prefix>: builds everything into <prefix> and leaves the libs there
    local slice="$1" prefix="$2" src="$work/$1/src"
    settings "$slice"
    rm -rf "$prefix" "$src"; mkdir -p "$prefix/lib/pkgconfig" "$src"
    # Only our prefix is visible to pkg-config, plus the system's on Linux (libva, libdrm).
    local pc_libdir="$prefix/lib/pkgconfig"
    [[ "$slice" == linux64 ]] && pc_libdir="$pc_libdir:/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/lib/pkgconfig:/usr/share/pkgconfig"
    export PKG_CONFIG_LIBDIR="$pc_libdir" PKG_CONFIG_PATH=""
    [[ -n "$deployment_target" ]] && export MACOSX_DEPLOYMENT_TARGET="$deployment_target"

    if [[ -n "$meson_cross" ]]; then
        case "$slice" in
            win64) cat >"$meson_cross" <<EOF
[binaries]
c = '${cross_prefix}gcc'
cpp = '${cross_prefix}g++'
ar = '${cross_prefix}ar'
strip = '${cross_prefix}strip'
windres = '${cross_prefix}windres'
pkg-config = 'pkg-config'
nasm = 'nasm'

[properties]
c_link_args = ['-static-libgcc']

[host_machine]
system = 'windows'
cpu_family = 'x86_64'
cpu = 'x86_64'
endian = 'little'
EOF
                ;;
            macos-*) cat >"$meson_cross" <<EOF
[binaries]
c = ['clang', '-arch', '${slice#macos-}']
cpp = ['clang++', '-arch', '${slice#macos-}']
ar = 'ar'
strip = 'strip'
pkg-config = 'pkg-config'
nasm = 'nasm'

[built-in options]
c_args = ['-mmacosx-version-min=$deployment_target']
c_link_args = ['-mmacosx-version-min=$deployment_target']

[host_machine]
system = 'darwin'
cpu_family = '$([[ "$slice" == macos-arm64 ]] && echo aarch64 || echo x86_64)'
cpu = '$([[ "$slice" == macos-arm64 ]] && echo aarch64 || echo x86_64)'
endian = 'little'
EOF
                ;;
        esac
    fi

    # dav1d (meson, static, PIC)
    log "[$slice] dav1d $DAV1D_VERSION"
    extract "$dav1d_tar" "$src/dav1d"
    local dav1d_args=(--prefix="$prefix" --libdir=lib --buildtype=release --default-library=static
        -Db_staticpic=true -Denable_tools=false -Denable_tests=false -Denable_examples=false -Denable_docs=false)
    [[ -n "$meson_cross" ]] && dav1d_args+=(--cross-file "$meson_cross")
    (cd "$src/dav1d" && meson setup build "${dav1d_args[@]}" >"$work/$slice-dav1d.log" 2>&1 \
        && ninja -C build -j "$jobs" install >>"$work/$slice-dav1d.log" 2>&1) || { tail -n 50 "$work/$slice-dav1d.log"; die "dav1d failed (log: $work/$slice-dav1d.log)"; }
    build_lines+=("[$slice] dav1d: meson setup build ${dav1d_args[*]} && ninja -C build install")

    # libopus (autotools, static)
    log "[$slice] opus $OPUS_VERSION"
    extract "$opus_tar" "$src/opus"
    local opus_args=(--prefix="$prefix" --libdir="$prefix/lib" --disable-shared --enable-static --disable-extra-programs --disable-doc --with-pic ${opus_host[@]+"${opus_host[@]}"})
    (cd "$src/opus" && CC="$cc" CFLAGS="$cflags" LDFLAGS="$ldflags" ./configure "${opus_args[@]}" >"$work/$slice-opus.log" 2>&1 \
        && make -j "$jobs" >>"$work/$slice-opus.log" 2>&1 && make install >>"$work/$slice-opus.log" 2>&1) || { tail -n 50 "$work/$slice-opus.log"; die "opus failed (log: $work/$slice-opus.log)"; }
    build_lines+=("[$slice] opus: CC=\"$cc\" CFLAGS=\"$cflags\" LDFLAGS=\"$ldflags\" ./configure ${opus_args[*]} && make && make install")

    # libvpx (own configure, static, PIC)
    log "[$slice] libvpx $LIBVPX_VERSION"
    extract "$vpx_tar" "$src/libvpx"
    local vpx_args=(--prefix="$prefix" --libdir="$prefix/lib" --target="$vpx_target" --disable-shared --enable-static --enable-pic
        --enable-vp8 --enable-vp9 --enable-vp9-highbitdepth --disable-examples --disable-tools --disable-docs
        --disable-unit-tests --disable-install-bins --disable-install-docs --as=nasm)
    local vpx_env=()
    case "$slice" in
        win64) vpx_env=(CROSS="$cross_prefix") ;;
        macos-*) vpx_env=(CC="$cc" CXX="$cxx" LD="$cc") ;;
        linux64) vpx_env=(CC="$cc" CXX="$cxx") ;;
    esac
    (cd "$src/libvpx" && env ${vpx_env[@]+"${vpx_env[@]}"} ./configure "${vpx_args[@]}" >"$work/$slice-libvpx.log" 2>&1 \
        && make -j "$jobs" >>"$work/$slice-libvpx.log" 2>&1 && make install >>"$work/$slice-libvpx.log" 2>&1) || { tail -n 50 "$work/$slice-libvpx.log"; die "libvpx failed (log: $work/$slice-libvpx.log)"; }
    build_lines+=("[$slice] libvpx: ${vpx_env[*]+"${vpx_env[*]}"} ./configure ${vpx_args[*]} && make && make install")

    # Header-only hardware SDKs: nv-codec-headers (every target that enables nvenc), AMF
    # and libvpl (win64).
    if [[ "$slice" == linux64 || "$slice" == win64 ]]; then
        log "[$slice] nv-codec-headers $NVCODEC_VERSION"
        extract "$nvcodec_tar" "$src/nv-codec-headers"
        make -C "$src/nv-codec-headers" install PREFIX="$prefix" LIBDIR=lib >"$work/$slice-nvcodec.log" 2>&1 || die "nv-codec-headers failed"
        build_lines+=("[$slice] nv-codec-headers: make install PREFIX=<prefix> LIBDIR=lib")
    fi
    if [[ "$slice" == win64 ]]; then
        log "[$slice] AMF headers $AMF_VERSION ($AMF_COMMIT)"
        rm -rf "$src/amf"; mkdir -p "$src/amf"; tar -xzf "$amf_tar" -C "$src/amf"
        [[ -d "$src/amf/amf/public/include" ]] || die "AMF headers not found in $amf_tar"
        rm -rf "$prefix/include/AMF"; cp -R "$src/amf/amf/public/include" "$prefix/include/AMF"
        build_lines+=("[$slice] AMF: amf/public/include copied to <prefix>/include/AMF (headers only)")

        log "[$slice] libvpl $LIBVPL_VERSION (dispatcher, static)"
        extract "$vpl_tar" "$src/libvpl"
        local vpl_args=(-S "$src/libvpl" -B "$src/libvpl/build" -G Ninja -DCMAKE_BUILD_TYPE=Release
            -DCMAKE_INSTALL_PREFIX="$prefix" -DCMAKE_INSTALL_LIBDIR=lib -DBUILD_SHARED_LIBS=OFF -DBUILD_TESTS=OFF
            -DBUILD_TOOLS=OFF -DBUILD_EXAMPLES=OFF -DINSTALL_EXAMPLE_CODE=OFF -DCMAKE_POSITION_INDEPENDENT_CODE=ON ${cmake_cross[@]+"${cmake_cross[@]}"})
        (cmake "${vpl_args[@]}" >"$work/$slice-libvpl.log" 2>&1 && cmake --build "$src/libvpl/build" -j "$jobs" >>"$work/$slice-libvpl.log" 2>&1 \
            && cmake --install "$src/libvpl/build" >>"$work/$slice-libvpl.log" 2>&1) || { tail -n 50 "$work/$slice-libvpl.log"; die "libvpl failed (log: $work/$slice-libvpl.log)"; }
        build_lines+=("[$slice] libvpl: cmake ${vpl_args[*]} && cmake --build && cmake --install")
    fi

    # FFmpeg
    log "[$slice] FFmpeg $FFMPEG_VERSION"
    extract "$ffmpeg_tar" "$src/ffmpeg"
    local ff_args=(--prefix="$prefix" --libdir="$prefix/lib" --shlibdir="$prefix/$lib_dir" --incdir="$prefix/include"
        "${common_configure[@]}" "${ffmpeg_target[@]}")
    [[ "$slice" == linux64 ]] && ff_args+=(--extra-cflags="$cflags")
    (cd "$src/ffmpeg" && ./configure "${ff_args[@]}" >"$work/$slice-ffmpeg.log" 2>&1 \
        && make -j "$jobs" >>"$work/$slice-ffmpeg.log" 2>&1 && make install >>"$work/$slice-ffmpeg.log" 2>&1) \
        || { tail -n 60 "$work/$slice-ffmpeg.log"; [[ -f "$src/ffmpeg/ffbuild/config.log" ]] && tail -n 40 "$src/ffmpeg/ffbuild/config.log"; die "FFmpeg failed (log: $work/$slice-ffmpeg.log)"; }
    build_lines+=("[$slice] ffmpeg: ./configure ${ff_args[*]} && make && make install")
    # shellcheck disable=SC2086 # $cc may carry "-arch x86_64"
    build_lines+=("[$slice] toolchain: $(tool_version $cc) | nasm: $(tool_version nasm) | meson: $(tool_version meson) | ninja: $(tool_version ninja) | make: $(tool_version make)$([[ "$slice" == win64 ]] && echo " | cmake: $(tool_version cmake)")")

    # Collect the five libraries (real files, soname names; the versioned files and the
    # unversioned development symlinks stay behind) into <prefix>/out.
    mkdir -p "$prefix/out"
    local n
    while read -r n; do
        [[ -e "$prefix/$lib_dir/$n" ]] || die "[$slice] expected $prefix/$lib_dir/$n after the install (FFMPEG_MAJORS in pins.env wrong?)"
        cp -L "$prefix/$lib_dir/$n" "$prefix/out/$n"
    done < <(lib_names "$slice")

    case "$slice" in
        linux64)
            # dlopen() of any one library finds its siblings in the same folder.
            for f in "$prefix"/out/*; do
                patchelf --set-rpath '$ORIGIN' "$f"
                readelf -d "$f" | grep -q 'RUNPATH.*\$ORIGIN' || die "no \$ORIGIN rpath in $f"
            done
            ;;
        macos-*)
            for f in "$prefix"/out/*; do
                install_name_tool -add_rpath @loader_path "$f"
            done
            ;;
        win64)
            local bad=0 imports
            for f in "$prefix"/out/*.dll; do
                imports="$(x86_64-w64-mingw32-objdump -p "$f" | sed -n 's/^[[:space:]]*DLL Name: //p' | tr '[:upper:]' '[:lower:]')"
                echo "$(basename "$f") imports: $(echo "$imports" | tr '\n' ' ')"
                while read -r imp; do
                    [[ -z "$imp" ]] && continue
                    if ! [[ "$imp" =~ $win_allowed_imports ]] && ! lib_names win64 | grep -qxF "$imp"; then
                        echo "::error::$(basename "$f") imports $imp, which is not a Windows system DLL"
                        bad=1
                    fi
                done <<<"$imports"
            done
            [[ $bad -eq 0 ]] || die "DLL import gate failed (link libgcc / libstdc++ / winpthread statically)"
            ;;
    esac
}

for slice in "${slices[@]}"; do
    build_slice "$slice" "$work/$slice/prefix"
done

# ---------------------------------------------------------------------------------------------
# Assemble <out>
# ---------------------------------------------------------------------------------------------
log "assembling $out"
find "$out" -mindepth 1 -maxdepth 1 ! -name sources -exec rm -rf {} +
mkdir -p "$out/sources"
first="${slices[0]}"
if [[ "$target" == macos-universal ]]; then
    while read -r n; do
        lipo -create -output "$out/$n" "$work/macos-arm64/prefix/out/$n" "$work/macos-x86_64/prefix/out/$n"
        lipo -info "$out/$n"
    done < <(lib_names macos-arm64)
else
    cp "$work/$first/prefix/out"/* "$out/"
fi
if [[ "$target" == macos-* ]]; then
    # Ad-hoc signatures: arm64 refuses unsigned code, and the release job re-signs them with
    # the Developer ID anyway. `-f` replaces the per-slice ad-hoc signatures lipo kept.
    for f in "$out"/*.dylib; do codesign --force --sign - "$f"; done
fi
cp -R "$work/$first/prefix/include" "$out/include"
for a in "$ffmpeg_tar" "$ffmpeg_asc" "$dav1d_tar" "$opus_tar" "$vpx_tar" "$nvcodec_tar" "$vpl_tar" "$amf_tar"; do
    cp "$a" "$out/sources/"
done

# Licence gate: the configuration string compiled into libavcodec must not carry the GPL /
# non-free / version-3 switches, and must carry the libraries we expect.
avcodec_lib="$out/$(lib_names "$first" | sed -n 4p)"
strings_tool="strings"; [[ "$target" == win64 ]] && strings_tool="x86_64-w64-mingw32-strings"
configuration="$($strings_tool "$avcodec_lib" | grep -m1 -- '--enable-shared' || true)"
[[ -n "$configuration" ]] || die "avcodec_configuration() string not found in $avcodec_lib"
for bad in --enable-gpl --enable-nonfree --enable-version3; do
    [[ "$configuration" != *"$bad"* ]] || die "the build is not LGPL-2.1: configuration contains $bad"
done
for want in --enable-libdav1d --enable-libopus --enable-libvpx --disable-network; do
    [[ "$configuration" == *"$want"* ]] || die "configuration lacks $want: $configuration"
done
echo "avcodec_configuration(): $configuration"

# SOURCES.txt
{
    echo "# Inputs of the FFmpeg runtime $FFMPEG_VERSION ($target) built by scripts/ffmpeg-build.sh;"
    echo "# pinned in third_party/ffmpeg/pins.env. \`sha256sum -c SOURCES.txt\` checks the archives in sources/."
    while IFS='|' read -r name ver url sum; do
        printf '%s  %s\n# %s %s: %s\n' "$sum" "$(basename "$url")" "$name" "$ver" "$url"
    done < <(inputs)
    printf '%s  %s\n# AMF %s: git %s commit %s, amf/public/include + LICENSE.txt only (MIT headers)\n' \
        "$(sha256_of "$amf_tar")" "$(basename "$amf_tar")" "$AMF_VERSION" "$AMF_GIT" "$AMF_COMMIT"
} >"$out/SOURCES.txt"

# BUILD.txt
git_sha="$(git -C "$root" rev-parse --short HEAD 2>/dev/null || echo unknown)"
{
    echo "FFmpeg runtime for Soyuz: FFmpeg $FFMPEG_VERSION, target $target"
    echo "built $(date -u +%Y-%m-%dT%H:%M:%SZ) on $(uname -srm) by scripts/ffmpeg-build.sh (soyuz commit $git_sha)"
    echo "library majors (avutil swresample swscale avcodec avformat): $FFMPEG_MAJORS"
    echo "libraries: $(lib_names "$first" | tr '\n' ' ')"
    echo "FFmpeg tarball signature: $signature_note"
    echo "avcodec_configuration(): $configuration"
    echo
    echo "Inputs: see SOURCES.txt (the archives are in sources/ next to this file on the runtime release)."
    echo "Rebuild: scripts/ffmpeg-build.sh $target <out-dir> from the Soyuz repository at the commit above."
    echo
    echo "Exact commands (<prefix> = the per-slice install folder):"
    for l in ${build_lines[@]+"${build_lines[@]}"}; do echo "  $l"; done
    if [[ "$target" == macos-universal ]]; then
        echo "  [universal] lipo -create <arm64 lib> <x86_64 lib> per library; codesign --sign - (ad-hoc)"
    fi
    case "$target" in
        linux64) echo "  [post] patchelf --set-rpath '\$ORIGIN' <lib> (the siblings load from the same folder)" ;;
        macos-*) echo "  [post] install_name_tool -add_rpath @loader_path <lib>; install names are @rpath/<lib> (--install-name-dir)" ;;
        win64) echo "  [gate] x86_64-w64-mingw32-objdump -p: imports limited to $win_allowed_imports and the sibling DLLs" ;;
    esac
} | sed "s|$work|<work>|g" >"$out/BUILD.txt"

# LICENSE.md: LGPL-2.1 text, the BSD / MIT notices of the statically linked and header-only
# inputs, and what is in the build.
ff_src="$work/$first/src/ffmpeg"
notices="$work/notices"
rm -rf "$notices"; mkdir -p "$notices/nv-codec-headers" "$notices/libvpl" "$notices/amf"
tar -xzf "$nvcodec_tar" -C "$notices/nv-codec-headers"
tar -xzf "$vpl_tar" -C "$notices/libvpl"
tar -xzf "$amf_tar" -C "$notices/amf"
{
    cat <<EOF
# FFmpeg runtime for Soyuz — licences

This folder holds shared libraries built from FFmpeg $FFMPEG_VERSION by the Soyuz project
(\`scripts/ffmpeg-build.sh\`, configuration in \`BUILD.txt\`). Soyuz loads them at run time
(\`crates/soyuz-media\`); nothing of FFmpeg is compiled into Soyuz itself. FFmpeg is a trademark
of Fabrice Bellard, originator of the FFmpeg project.

| Component | Version | Licence | How it is used |
|---|---|---|---|
| FFmpeg (libavutil, libswresample, libswscale, libavcodec, libavformat) | $FFMPEG_VERSION | LGPL-2.1-or-later | the five libraries; built **without** \`--enable-gpl\`, \`--enable-nonfree\`, \`--enable-version3\` |
| dav1d | $DAV1D_VERSION | BSD-2-Clause | AV1 decoder, linked statically into libavcodec |
| Opus (libopus) | $OPUS_VERSION | BSD-3-Clause | Opus encoder / decoder, linked statically into libavcodec |
| libvpx | $LIBVPX_VERSION | BSD-3-Clause | VP8 / VP9 encoders and decoders, linked statically into libavcodec |
| nv-codec-headers (ffnvcodec) | $NVCODEC_VERSION | MIT | headers only (NVENC / NVDEC); the driver's library is loaded at run time |
| AMD AMF | $AMF_VERSION | MIT | headers only (AMF encoders, Windows); the driver's library is loaded at run time |
| Intel VPL (libvpl) | $LIBVPL_VERSION | MIT | dispatcher linked statically into libavcodec (QSV, Windows); the driver's runtime is loaded at run time |

## Your rights under the LGPL

The libraries are licensed under the GNU Lesser General Public License, version 2.1 or (at
your option) any later version; the full text follows. The complete corresponding source
code is the set of archives listed in \`SOURCES.txt\` (published next to these libraries on the
\`ffmpeg-runtime-$FFMPEG_VERSION\` release of the Soyuz repository), built with the exact commands in
\`BUILD.txt\`; Soyuz applies no patches. You may replace the libraries with your own build:
Soyuz loads whatever libraries of a supported FFmpeg major it finds in \`[media] ffmpeg_dir\`,
next to its executable, or in \`<config dir>/ffmpeg\` (see the Soyuz user guide, "Media").

## Patents

Soyuz and these libraries come with no patent licence. Some of the codecs they implement
(H.264, HEVC, AAC and others) are covered by patents in some jurisdictions; whether you
need a licence to use them depends on where you are and what you do (see
https://ffmpeg.org/legal.html). The Soyuz project cannot give legal advice.

## GNU Lesser General Public License, version 2.1

\`\`\`text
EOF
    cat "$ff_src/COPYING.LGPLv2.1"
    cat <<EOF
\`\`\`

## FFmpeg LICENSE.md (upstream notice)

\`\`\`text
EOF
    cat "$ff_src/LICENSE.md"
    cat <<EOF
\`\`\`

## dav1d (BSD-2-Clause)

\`\`\`text
EOF
    cat "$work/$first/src/dav1d/COPYING"
    cat <<EOF
\`\`\`

## Opus (BSD-3-Clause)

\`\`\`text
EOF
    cat "$work/$first/src/opus/COPYING"
    cat <<EOF
\`\`\`

## libvpx (BSD-3-Clause)

\`\`\`text
EOF
    cat "$work/$first/src/libvpx/LICENSE"
    echo
    cat "$work/$first/src/libvpx/PATENTS"
    cat <<EOF
\`\`\`

## nv-codec-headers (MIT)

The headers carry this notice ("This copyright notice applies to this header file only"):

\`\`\`text
EOF
    sed -n '2,/\*\//p' "$(find "$notices/nv-codec-headers" -name nvEncodeAPI.h | head -n 1)" | sed 's/^ \* \{0,1\}//; s/^\*\///'
    cat <<EOF
\`\`\`

## AMD AMF (MIT)

\`\`\`text
EOF
    cat "$notices/amf/LICENSE.txt"
    cat <<EOF
\`\`\`

## Intel VPL (MIT)

\`\`\`text
EOF
    cat "$(find "$notices/libvpl" -maxdepth 2 -name LICENSE | head -n 1)"
    echo '```'
} >"$out/LICENSE.md"

[[ $keep -eq 1 ]] || rm -rf "${work:?}"/{linux64,win64,macos-arm64,macos-x86_64,gnupg,notices} "$work"/*.meson

echo
echo "done: $out"
ls -la "$out"
