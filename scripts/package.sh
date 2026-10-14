#!/usr/bin/env bash
# Build a Linux release of Soyuz and package it as a relocatable PREFIX tree.
#
# Runs `cargo build --release -p soyuz-app -p soyuz-tui --locked`, then creates
# dist/soyuz-<version>-linux-<arch>.tar.gz with one top-level folder laid out like a prefix:
#
#   soyuz-<version>-linux-<arch>/
#     bin/soyuz                      the app
#     bin/soyuz-tui                  the terminal UI (`soyuz-tui [PATH]`)
#     share/applications/soyuz.desktop
#     share/icons/hicolor/<size>x<size>/apps/soyuz.png
#     share/metainfo/io.github.ulaas.Soyuz.metainfo.xml
#     share/licenses/soyuz/{LICENSE,THIRD-PARTY-NOTICES.md}
#     share/doc/soyuz/{README.md,USER_GUIDE.md,config.example.toml}
#     lib/soyuz/lib{avutil,swresample,swscale,avcodec,avformat}.so.NN   (with --ffmpeg-dir)
#     share/licenses/soyuz/ffmpeg/{LICENSE.md,BUILD.txt,SOURCES.txt}     (with --ffmpeg-dir)
#
# Install for the current user (no root, no install script):
#   tar -C ~/.local --strip-components=1 -xf soyuz-<version>-linux-<arch>.tar.gz
# or system-wide with `sudo tar -C /usr/local --strip-components=1 -xf …`.
#
# Every artifact gets a `<file>.sha256` next to it, and dist/SHA256SUMS-linux.txt lists them
# all in `sha256sum` format (what the in-app update check verifies against). This is the
# Linux counterpart of scripts/package.ps1 (same version source, out dir and checksums).
#
# Options:
#   --skip-build             package the existing target/release/soyuz and soyuz-tui
#   --features <list>        soyuz-app cargo features (e.g. glow; passed as soyuz-app/<f>)
#   --no-default-features
#   --out-dir <dir>          default: dist
#   --rpm                    also build an .rpm from the tarball (installer/linux/soyuz.spec,
#                            needs rpmbuild)
#   --ffmpeg-dir <dir>       add the FFmpeg runtime (the output of scripts/ffmpeg-build.sh or an
#                            unpacked ffmpeg-runtime-<ver>-linux-<arch>.tar.xz): lib*.so.* into
#                            lib/soyuz/ (the exe looks in <exe dir>/../lib/soyuz), LICENSE.md /
#                            BUILD.txt / SOURCES.txt into share/licenses/soyuz/ffmpeg/; the rpm
#                            then installs them under %{_libdir}/soyuz/
#   --dry-run                print the plan without building or writing anything
#
# Examples:
#   scripts/package.sh
#   scripts/package.sh --skip-build --rpm --ffmpeg-dir ffmpeg-rt
#   scripts/package.sh --dry-run
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

skip_build=0 features="" no_default=0 out_dir="dist" rpm=0 dry_run=0 ffmpeg_dir=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-build) skip_build=1 ;;
        --features) features="${2:?--features needs a value}"; shift ;;
        --features=*) features="${1#*=}" ;;
        --no-default-features) no_default=1 ;;
        --out-dir) out_dir="${2:?--out-dir needs a value}"; shift ;;
        --out-dir=*) out_dir="${1#*=}" ;;
        --rpm) rpm=1 ;;
        --ffmpeg-dir) ffmpeg_dir="${2:?--ffmpeg-dir needs a value}"; shift ;;
        --ffmpeg-dir=*) ffmpeg_dir="${1#*=}" ;;
        --dry-run) dry_run=1 ;;
        -h | --help) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
        *) echo "package.sh: unknown option '$1' (see --help)" >&2; exit 2 ;;
    esac
    shift
done

version="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)"
[[ -n "$version" ]] || { echo "package.sh: no version in Cargo.toml" >&2; exit 1; }
arch="$(uname -m)" # x86_64 / aarch64: the names the update check matches (services/updates.rs)
name="soyuz-$version-linux-$arch"
tarball="$out_dir/$name.tar.gz"
sums="$out_dir/SHA256SUMS-linux.txt"
bin="target/release/soyuz"
tui_bin="target/release/soyuz-tui"
metainfo="io.github.ulaas.Soyuz.metainfo.xml"

# What goes into the tree: "source|path inside the tree|mode".
files=(
    "$bin|bin/soyuz|755"
    "$tui_bin|bin/soyuz-tui|755"
    "crates/soyuz-app/assets/soyuz.desktop|share/applications/soyuz.desktop|644"
    "installer/linux/$metainfo|share/metainfo/$metainfo|644"
    "LICENSE|share/licenses/soyuz/LICENSE|644"
    "THIRD-PARTY-NOTICES.md|share/licenses/soyuz/THIRD-PARTY-NOTICES.md|644"
    "README.md|share/doc/soyuz/README.md|644"
    "docs/USER_GUIDE.md|share/doc/soyuz/USER_GUIDE.md|644"
    "docs/config.example.toml|share/doc/soyuz/config.example.toml|644"
)
for icon in installer/linux/icons/hicolor/*/apps/soyuz.png installer/linux/icons/hicolor/scalable/apps/soyuz.svg; do
    [[ -f "$icon" ]] && files+=("$icon|share/icons/${icon#installer/linux/icons/}|644")
done
# The FFmpeg runtime: private libraries (mode 644: dlopen() needs no execute bit, and
# dependency scanners skip non-executables) plus their licence texts.
if [[ -n "$ffmpeg_dir" ]]; then
    n=0
    for lib in "$ffmpeg_dir"/lib*.so.*; do
        [[ -f "$lib" ]] || continue
        files+=("$lib|lib/soyuz/$(basename "$lib")|644")
        n=$((n + 1))
    done
    [[ $n -gt 0 ]] || { echo "package.sh: no lib*.so.* in --ffmpeg-dir $ffmpeg_dir" >&2; exit 1; }
    for doc in LICENSE.md BUILD.txt SOURCES.txt; do
        [[ -f "$ffmpeg_dir/$doc" ]] || { echo "package.sh: missing $ffmpeg_dir/$doc (the runtime folder carries its licence texts)" >&2; exit 1; }
        files+=("$ffmpeg_dir/$doc|share/licenses/soyuz/ffmpeg/$doc|644")
    done
fi
rpm_ffmpeg=0; [[ -n "$ffmpeg_dir" ]] && rpm_ffmpeg=1

cargo_args=(build --release -p soyuz-app -p soyuz-tui --locked)
if [[ -n "$features" ]]; then
    # Unqualified names are soyuz-app's (they set the matching soyuz-tui features too).
    qualified=()
    for f in ${features//,/ }; do
        if [[ "$f" == */* ]]; then qualified+=("$f"); else qualified+=("soyuz-app/$f"); fi
    done
    cargo_args+=(--features "$(IFS=,; echo "${qualified[*]}")")
fi
[[ $no_default -eq 1 ]] && cargo_args+=(--no-default-features)

if [[ $dry_run -eq 1 ]]; then
    echo "version:  $version"
    if [[ $skip_build -eq 1 ]]; then echo "build:    (skipped)"; else echo "build:    cargo ${cargo_args[*]}"; fi
    echo "tarball:  $tarball"
    for f in "${files[@]}"; do
        IFS='|' read -r src dest mode <<<"$f"
        printf '  %-58s <- %s (%s)\n' "$name/$dest" "$src" "$mode"
    done
    [[ $rpm -eq 1 ]] && echo "rpm:      rpmbuild -bb installer/linux/soyuz.spec (Source0 = $name.tar.gz, soyuz_ffmpeg $rpm_ffmpeg)"
    echo "checksums: <artifact>.sha256 and $sums"
    echo "dry run: nothing built or written"
    exit 0
fi

if [[ $skip_build -eq 0 ]]; then
    echo "cargo ${cargo_args[*]}"
    cargo "${cargo_args[@]}"
fi
for b in "$bin" "$tui_bin"; do
    [[ -x "$b" ]] || { echo "package.sh: missing $b" >&2; exit 1; }
done

mkdir -p "$out_dir"
stage="$(mktemp -d "${TMPDIR:-/tmp}/soyuz-package.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
for f in "${files[@]}"; do
    IFS='|' read -r src dest mode <<<"$f"
    install -D -m "$mode" "$src" "$stage/$name/$dest"
done

# Reproducible-ish archive: fixed owner, sorted entries, mtimes from the last commit.
mtime="@$(git log -1 --format=%ct 2>/dev/null || date +%s)"
rm -f "$tarball"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="$mtime" \
    -C "$stage" -czf "$tarball" "$name"
artifacts=("$tarball")

if [[ $rpm -eq 1 ]]; then
    command -v rpmbuild >/dev/null || { echo "package.sh: --rpm needs rpmbuild" >&2; exit 1; }
    rpm_top="$stage/rpmbuild"
    mkdir -p "$rpm_top/SOURCES"
    cp "$tarball" "$rpm_top/SOURCES/"
    rpmbuild -bb installer/linux/soyuz.spec \
        --define "_topdir $rpm_top" \
        --define "soyuz_version $version" \
        --define "soyuz_arch $arch" \
        --define "soyuz_ffmpeg $rpm_ffmpeg"
    for r in "$rpm_top"/RPMS/*/*.rpm; do
        cp "$r" "$out_dir/"
        artifacts+=("$out_dir/$(basename "$r")")
    done
fi

# Checksums: one file per artifact plus a combined sha256sum-style list, merged with entries
# from earlier runs whose files still exist.
declare -A lines=()
if [[ -f "$sums" ]]; then
    while read -r hash file; do
        file="${file#\*}"
        [[ "$hash" =~ ^[0-9a-f]{64}$ && -f "$out_dir/$file" ]] && lines["$file"]="$hash"
    done <"$sums"
fi
for a in "${artifacts[@]}"; do
    leaf="$(basename "$a")"
    hash="$(sha256sum "$a" | cut -d' ' -f1)"
    printf '%s  %s\n' "$hash" "$leaf" >"$a.sha256"
    lines["$leaf"]="$hash"
done
for leaf in "${!lines[@]}"; do printf '%s  %s\n' "${lines[$leaf]}" "$leaf"; done | sort -k2 >"$sums"

for a in "${artifacts[@]}"; do
    printf '%s: %s\n' "$a" "$(du -h "$a" | cut -f1)"
done
echo "checksums in $sums"
echo "install: tar -C ~/.local --strip-components=1 -xf $tarball"
