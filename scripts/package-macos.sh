#!/usr/bin/env bash
# Build Soyuz for Apple silicon and package it as a signed Soyuz.app inside a disk image.
#
# Runs `cargo build --release --locked -p soyuz-app -p soyuz-tui --target aarch64-apple-darwin`
# (arm64 only: no universal or x86_64 slice), then writes into the output folder:
#
#   soyuz-<version>-macos-arm64.dmg        Soyuz.app + a link to /Applications (drag to install)
#   soyuz-<version>-macos-arm64.dmg.sha256
#   SHA256SUMS-macos.txt                   merged with the entries of earlier runs (package.ps1)
#   Soyuz.app                              the bundle itself, as it is in the image
#
#   Soyuz.app/Contents/
#     Info.plist                           installer/macos/Info.plist, version from Cargo.toml
#     MacOS/soyuz                          the app (what Finder, the Dock and Spotlight start)
#     MacOS/soyuz-tui                      the terminal UI, for a terminal:
#                                          /Applications/Soyuz.app/Contents/MacOS/soyuz-tui
#     Resources/Soyuz.icns                 16 … 1024 px, from the repository's PNG icons
#     Resources/{LICENSE,THIRD-PARTY-NOTICES.md}
#     Frameworks/lib{avutil,swresample,swscale,avcodec,avformat}.N.dylib   (with --ffmpeg-dir)
#     Resources/ffmpeg/{LICENSE.md,BUILD.txt,SOURCES.txt}                  (with --ffmpeg-dir)
#
# Signing: ad-hoc (`codesign -s -`) unless MACOS_SIGN_IDENTITY names a Developer ID
# Application identity in the keychain. Nested code is signed first (dylibs, soyuz-tui), then
# the bundle, with the hardened runtime and installer/macos/entitlements.plist. An ad-hoc
# signed app is not notarized: Gatekeeper asks once on first open (Control-click > Open, or
# System Settings > Privacy & Security > Open Anyway).
#
# Notarization (only with a real identity and MACOS_NOTARIZE=1): either
#   MACOS_NOTARY_PROFILE=<keychain profile from `xcrun notarytool store-credentials`>, or
#   APPLE_ID + APPLE_TEAM_ID + APPLE_APP_PASSWORD (an app-specific password).
# The disk image is submitted, waited for and stapled.
#
# Options:
#   --skip-build             package the existing target/aarch64-apple-darwin/release binaries
#   --ffmpeg-dir <dir>       bundle the FFmpeg runtime (output of `scripts/ffmpeg-build.sh
#                            macos-arm64|macos-universal`, or an unpacked ffmpeg-runtime-*-macos-*
#                            archive): *.dylib into Contents/Frameworks (universal libraries are
#                            thinned to arm64), the licence texts into Contents/Resources/ffmpeg
#   --features <list>        soyuz-app cargo features (passed as soyuz-app/<f>)
#   --out-dir <dir>          default: dist
#   --dry-run                print the plan without building or writing anything
#
# Needs: Xcode command line tools (codesign, lipo, otool, install_name_tool, sips, iconutil,
# hdiutil, plutil), rustup or a Rust toolchain with the aarch64-apple-darwin target.
# Works with macOS's /bin/bash 3.2.
#
# Examples:
#   scripts/package-macos.sh
#   scripts/package-macos.sh --ffmpeg-dir ffmpeg-rt
#   MACOS_SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" MACOS_NOTARIZE=1 \
#     MACOS_NOTARY_PROFILE=soyuz scripts/package-macos.sh --ffmpeg-dir ffmpeg-rt
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

skip_build=0 features="" out_dir="dist" dry_run=0 ffmpeg_dir=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-build) skip_build=1 ;;
        --features) features="${2:?--features needs a value}"; shift ;;
        --features=*) features="${1#*=}" ;;
        --out-dir) out_dir="${2:?--out-dir needs a value}"; shift ;;
        --out-dir=*) out_dir="${1#*=}" ;;
        --ffmpeg-dir) ffmpeg_dir="${2:?--ffmpeg-dir needs a value}"; shift ;;
        --ffmpeg-dir=*) ffmpeg_dir="${1#*=}" ;;
        --dry-run) dry_run=1 ;;
        -h | --help) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
        *) echo "package-macos.sh: unknown option '$1' (see --help)" >&2; exit 2 ;;
    esac
    shift
done

die() { echo "package-macos.sh: $*" >&2; exit 1; }

version="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)"
[[ -n "$version" ]] || die "no version in Cargo.toml"
target="aarch64-apple-darwin"
min_macos="11.0"
name="soyuz-$version-macos-arm64"
dmg="$out_dir/$name.dmg"
sums="$out_dir/SHA256SUMS-macos.txt"
app_name="Soyuz.app"
bin_dir="target/$target/release"
identity="${MACOS_SIGN_IDENTITY:--}"
entitlements="installer/macos/entitlements.plist"
plist_template="installer/macos/Info.plist"
# CFBundleVersion: the commit count keeps rising from release to release.
build_number="$(git rev-list --count HEAD 2>/dev/null || echo 1)"

cargo_args=(build --release --locked --target "$target" -p soyuz-app -p soyuz-tui)
if [[ -n "$features" ]]; then
    qualified=""
    for f in ${features//,/ }; do
        if [[ "$f" == */* ]]; then qualified="$qualified,$f"; else qualified="$qualified,soyuz-app/$f"; fi
    done
    cargo_args+=(--features "${qualified#,}")
fi

ffmpeg_libs=()
if [[ -n "$ffmpeg_dir" ]]; then
    for lib in "$ffmpeg_dir"/*.dylib; do
        [[ -f "$lib" ]] && ffmpeg_libs+=("$lib")
    done
    [[ ${#ffmpeg_libs[@]} -gt 0 ]] || die "no *.dylib in --ffmpeg-dir $ffmpeg_dir"
    for doc in LICENSE.md BUILD.txt SOURCES.txt; do
        [[ -f "$ffmpeg_dir/$doc" ]] || die "missing $ffmpeg_dir/$doc (the runtime folder carries its licence texts)"
    done
fi

if [[ $dry_run -eq 1 ]]; then
    echo "version:   $version (CFBundleVersion $build_number)"
    if [[ $skip_build -eq 1 ]]; then echo "build:     (skipped)"; else echo "build:     cargo ${cargo_args[*]}"; fi
    echo "bundle:    $out_dir/$app_name"
    echo "  Contents/Info.plist <- $plist_template"
    echo "  Contents/MacOS/soyuz <- $bin_dir/soyuz"
    echo "  Contents/MacOS/soyuz-tui <- $bin_dir/soyuz-tui"
    echo "  Contents/Resources/Soyuz.icns <- crates/soyuz-app/assets/icon.png (sips + iconutil)"
    echo "  Contents/Resources/LICENSE, THIRD-PARTY-NOTICES.md"
    if [[ ${#ffmpeg_libs[@]} -gt 0 ]]; then
        for lib in "${ffmpeg_libs[@]}"; do echo "  Contents/Frameworks/$(basename "$lib") <- $lib (arm64)"; done
        for doc in LICENSE.md BUILD.txt SOURCES.txt; do echo "  Contents/Resources/ffmpeg/$doc <- $ffmpeg_dir/$doc"; done
    fi
    if [[ "$identity" == "-" ]]; then echo "sign:      ad-hoc (set MACOS_SIGN_IDENTITY for a Developer ID)"; else echo "sign:      $identity"; fi
    echo "image:     $dmg (UDZO, with an /Applications link)"
    echo "checksums: $dmg.sha256 and $sums"
    echo "dry run: nothing built or written"
    exit 0
fi

[[ "$(uname -s)" == Darwin ]] || die "builds the macOS app; run it on a Mac"
for tool in codesign lipo otool install_name_tool sips iconutil hdiutil plutil shasum; do
    command -v "$tool" >/dev/null || die "needs $tool (install the Xcode command line tools)"
done

# --- build -----------------------------------------------------------------------------------
if [[ $skip_build -eq 0 ]]; then
    if command -v rustup >/dev/null && ! rustup target list --installed 2>/dev/null | grep -qx "$target"; then
        rustup target add "$target"
    fi
    echo "cargo ${cargo_args[*]}"
    MACOSX_DEPLOYMENT_TARGET="$min_macos" cargo "${cargo_args[@]}"
fi
for b in soyuz soyuz-tui; do
    [[ -x "$bin_dir/$b" ]] || die "missing $bin_dir/$b (build first or drop --skip-build)"
done

# --- assemble the bundle ---------------------------------------------------------------------
stage="$(mktemp -d "${TMPDIR:-/tmp}/soyuz-package-macos.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
app="$stage/$app_name"
contents="$app/Contents"
mkdir -p "$contents/MacOS" "$contents/Resources"

sed -e "s|@VERSION@|$version|g" -e "s|@BUILD@|$build_number|g" "$plist_template" >"$contents/Info.plist"
plutil -lint "$contents/Info.plist" >/dev/null
printf 'APPL????' >"$contents/PkgInfo"

install -m 755 "$bin_dir/soyuz" "$contents/MacOS/soyuz"
install -m 755 "$bin_dir/soyuz-tui" "$contents/MacOS/soyuz-tui"
install -m 644 LICENSE THIRD-PARTY-NOTICES.md "$contents/Resources/"

# Icon: every size from 16 to 512 at @1x and @2x. The hand-drawn hicolor PNGs are used where
# one has the exact pixel size, the 512 px app icon is resampled for the others.
iconset="$stage/Soyuz.iconset"
mkdir -p "$iconset"
icon_src="crates/soyuz-app/assets/icon.png"
for size in 16 32 128 256 512; do
    for scale in 1 2; do
        px=$((size * scale))
        file="icon_${size}x${size}.png"
        [[ $scale -eq 2 ]] && file="icon_${size}x${size}@2x.png"
        exact="installer/linux/icons/hicolor/${px}x${px}/apps/soyuz.png"
        if [[ -f "$exact" ]]; then
            cp "$exact" "$iconset/$file"
        else
            sips -s format png -z "$px" "$px" "$icon_src" --out "$iconset/$file" >/dev/null
        fi
    done
done
iconutil -c icns -o "$contents/Resources/Soyuz.icns" "$iconset"

# The FFmpeg runtime: arm64 slices only, install names @rpath/<file> (the libraries find each
# other through their LC_RPATH @loader_path; soyuz-media dlopens them by full path from
# Contents/Frameworks).
if [[ ${#ffmpeg_libs[@]} -gt 0 ]]; then
    mkdir -p "$contents/Frameworks" "$contents/Resources/ffmpeg"
    for lib in "${ffmpeg_libs[@]}"; do
        dest="$contents/Frameworks/$(basename "$lib")"
        archs="$(lipo -archs "$lib")"
        case " $archs " in
            *" arm64 "*) ;;
            *) die "$lib has no arm64 slice ($archs)" ;;
        esac
        if [[ "$archs" == arm64 ]]; then cp "$lib" "$dest"; else lipo -thin arm64 -output "$dest" "$lib"; fi
        chmod 644 "$dest"
        install_name_tool -id "@rpath/$(basename "$lib")" "$dest" 2>/dev/null
    done
    for doc in LICENSE.md BUILD.txt SOURCES.txt; do
        install -m 644 "$ffmpeg_dir/$doc" "$contents/Resources/ffmpeg/$doc"
    done
fi

# --- check what is in it ---------------------------------------------------------------------
macho=("$contents/MacOS/soyuz" "$contents/MacOS/soyuz-tui")
if [[ -d "$contents/Frameworks" ]]; then
    for lib in "$contents/Frameworks"/*.dylib; do macho+=("$lib"); done
fi
for f in "${macho[@]}"; do
    archs="$(lipo -archs "$f")"
    [[ "$archs" == arm64 ]] || die "$(basename "$f") is '$archs', expected arm64 only"
    # Every load command must point at the system or inside the bundle.
    bad="$(otool -L "$f" | tail -n +2 | awk '{print $1}' |
        grep -Ev '^(/usr/lib/|/System/Library/|@rpath/|@loader_path/|@executable_path/)' || true)"
    [[ -z "$bad" ]] || die "$(basename "$f") links outside the system and the bundle: $bad"
done

# --- sign ------------------------------------------------------------------------------------
sign_args=(--force --options runtime)
if [[ "$identity" == "-" ]]; then sign_args+=(--timestamp=none); else sign_args+=(--timestamp); fi
if [[ -d "$contents/Frameworks" ]]; then
    for lib in "$contents/Frameworks"/*.dylib; do codesign "${sign_args[@]}" -s "$identity" "$lib"; done
fi
codesign "${sign_args[@]}" -s "$identity" "$contents/MacOS/soyuz-tui"
codesign "${sign_args[@]}" --deep --entitlements "$entitlements" -s "$identity" "$app"
codesign --verify --deep --strict --verbose=2 "$app"
ents="$(codesign --display --entitlements - "$app" 2>&1)"
[[ "$ents" == *disable-library-validation* ]] || die "the app lost its entitlements"

# --- disk image ------------------------------------------------------------------------------
mkdir -p "$out_dir"
rm -rf "${out_dir:?}/$app_name" "$dmg" "$dmg.sha256"
dmg_root="$stage/dmg"
mkdir -p "$dmg_root"
ditto "$app" "$dmg_root/$app_name"
ln -s /Applications "$dmg_root/Applications"
hdiutil create -quiet -volname "Soyuz $version" -srcfolder "$dmg_root" -fs HFS+ -format UDZO \
    -imagekey zlib-level=9 -ov "$dmg"
ditto "$app" "$out_dir/$app_name"
if [[ "$identity" != "-" ]]; then
    codesign --force --timestamp -s "$identity" "$dmg"
fi

if [[ "${MACOS_NOTARIZE:-0}" == 1 ]]; then
    [[ "$identity" != "-" ]] || die "MACOS_NOTARIZE=1 needs MACOS_SIGN_IDENTITY (ad-hoc apps cannot be notarized)"
    if [[ -n "${MACOS_NOTARY_PROFILE:-}" ]]; then
        xcrun notarytool submit "$dmg" --keychain-profile "$MACOS_NOTARY_PROFILE" --wait
    else
        : "${APPLE_ID:?MACOS_NOTARIZE=1 needs MACOS_NOTARY_PROFILE or APPLE_ID}"
        : "${APPLE_TEAM_ID:?MACOS_NOTARIZE=1 needs APPLE_TEAM_ID}"
        : "${APPLE_APP_PASSWORD:?MACOS_NOTARIZE=1 needs APPLE_APP_PASSWORD}"
        xcrun notarytool submit "$dmg" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
            --password "$APPLE_APP_PASSWORD" --wait
    fi
    xcrun stapler staple "$dmg"
    spctl --assess --type open --context context:primary-signature --verbose "$dmg"
fi

# --- checksums -------------------------------------------------------------------------------
leaf="$(basename "$dmg")"
hash="$(shasum -a 256 "$dmg" | cut -d' ' -f1)"
printf '%s  %s\n' "$hash" "$leaf" >"$dmg.sha256"
{
    if [[ -f "$sums" ]]; then
        # Keep the entries of other artifacts that still exist (package.ps1's tarball).
        while read -r h f; do
            f="${f#\*}"
            [[ "$f" != "$leaf" && -f "$out_dir/$f" ]] && printf '%s  %s\n' "$h" "$f"
        done <"$sums"
    fi
    printf '%s  %s\n' "$hash" "$leaf"
} | sort -k2 >"$sums.tmp"
mv "$sums.tmp" "$sums"

printf '%s: %s\n' "$dmg" "$(du -h "$dmg" | cut -f1)"
echo "bundle:    $out_dir/$app_name"
echo "checksums in $sums"
