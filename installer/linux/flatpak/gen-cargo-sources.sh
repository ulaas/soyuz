#!/usr/bin/env bash
# Generate installer/linux/flatpak/cargo-sources.json from Cargo.lock for the offline
# Flatpak build (io.github.ulaas.Soyuz.yml). Run it again whenever Cargo.lock changes.
#
# Needs flatpak-cargo-generator from https://github.com/flatpak/flatpak-builder-tools
# (cargo/flatpak-cargo-generator.py, or the `flatpak-cargo-generator` command its
# package installs) and its Python dependencies (aiohttp, toml / tomlkit). This script
# does not download it. It is looked up as:
#   1. $FLATPAK_CARGO_GENERATOR (path to the script or command)
#   2. flatpak-cargo-generator or flatpak-cargo-generator.py on PATH
#
# Usage: installer/linux/flatpak/gen-cargo-sources.sh [OUTPUT]   (default: next to this script)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
out="${1:-$here/cargo-sources.json}"

gen="${FLATPAK_CARGO_GENERATOR:-}"
if [[ -z "$gen" ]]; then
    for c in flatpak-cargo-generator flatpak-cargo-generator.py; do
        if command -v "$c" >/dev/null 2>&1; then
            gen="$(command -v "$c")"
            break
        fi
    done
fi
if [[ -z "$gen" ]]; then
    cat >&2 <<'MSG'
gen-cargo-sources.sh: flatpak-cargo-generator not found.
Get it from https://github.com/flatpak/flatpak-builder-tools (cargo/), install its Python
dependencies, then either put it on PATH or run:
  FLATPAK_CARGO_GENERATOR=/path/to/flatpak-cargo-generator.py installer/linux/flatpak/gen-cargo-sources.sh
MSG
    exit 1
fi

# A .py file runs with python3 (it may not be executable); a command runs directly.
if [[ "$gen" == *.py ]]; then
    python3 "$gen" "$root/Cargo.lock" -o "$out"
else
    "$gen" "$root/Cargo.lock" -o "$out"
fi
echo "wrote $out"
