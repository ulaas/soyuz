#!/usr/bin/env bash
# Regenerates the screenshots in site/assets/shots/ from the release build on Linux
# (port of scripts/screenshots.ps1).
#
# 1. Builds `soyuz` in release mode (skip with --skip-build).
# 2. Creates the privacy-safe demo tree with scripts/demo-fixture.sh (default
#    /tmp/SoyuzDemo), so no personal paths appear in the pictures.
# 3. Runs the app in portable mode (an empty `portable` file next to the executable,
#    settings in `portable-data/`) several times. Each run gets a fresh config.toml and
#    session.toml (tabs, favorites pointing into the demo tree) and a SOYUZ_SCRIPT that
#    navigates, switches views / panels / themes and saves window screenshots with `shot:`.
#    The app captures its own window; no mouse or keyboard input is sent to the desktop
#    and no external screenshot tool is used.
# 4. Downscales every PNG to at most --width pixels (ImageMagick) into site/assets/shots/.
# 5. Removes the portable marker, portable-data/, the raw captures and the demo tree.
#
# Options:
#   --exe <path>        default target/release/soyuz
#   --demo <dir>        default /tmp/SoyuzDemo
#   --out-dir <dir>     default site/assets/shots
#   --width <px>        maximum width of the written PNGs (default 1600)
#   --only <name>       run only this session (main, activity, treemap); repeatable
#   --skip-build
#   --keep-demo
#   --timeout <s>       per session (default 150)
#
# On Wayland the compositor places the window (SOYUZ_WINDOW_POS only works under X11).
# SOYUZ_SHOTS_KEEP_RAW=1 keeps the raw captures and logs.
#
# Examples:
#   scripts/screenshots.sh
#   scripts/screenshots.sh --skip-build --only main --keep-demo
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exe="$repo/target/release/soyuz"
demo="/tmp/SoyuzDemo"
out_dir="$repo/site/assets/shots"
width=1600
only=()
skip_build=0 keep_demo=0 timeout_s=150
while [[ $# -gt 0 ]]; do
    case "$1" in
        --exe) exe="${2:?}"; shift ;;
        --demo) demo="${2:?}"; shift ;;
        --out-dir) out_dir="${2:?}"; shift ;;
        --width) width="${2:?}"; shift ;;
        --only) only+=("${2:?}"); shift ;;
        --skip-build) skip_build=1 ;;
        --keep-demo) keep_demo=1 ;;
        --timeout) timeout_s="${2:?}"; shift ;;
        -h | --help) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
        *) echo "screenshots.sh: unknown option '$1' (see --help)" >&2; exit 2 ;;
    esac
    shift
done
command -v magick >/dev/null || { echo "screenshots.sh: needs ImageMagick 7 (magick) to downscale" >&2; exit 1; }

if [[ $skip_build -eq 0 ]]; then
    (cd "$repo" && cargo build --release -p soyuz-app)
fi
[[ -x "$exe" ]] || { echo "screenshots.sh: no binary at $exe (build it or pass --exe)" >&2; exit 1; }

exe_dir="$(cd "$(dirname "$exe")" && pwd)"
marker="$exe_dir/portable"
data="$exe_dir/portable-data"
if [[ -e "$marker" || -e "$data" ]]; then
    echo "screenshots.sh: $exe_dir already has a portable marker or portable-data/; move them away first (they may hold real settings)." >&2
    exit 1
fi

"$repo/scripts/demo-fixture.sh" "$demo"
demo="$(cd "$demo" && pwd)"
raw="$(mktemp -d "${TMPDIR:-/tmp}/soyuz-shots-XXXXXXXX")"
mkdir -p "$out_dir"

cleanup() {
    rm -f -- "$marker"
    rm -rf -- "$data"
    if [[ "${SOYUZ_SHOTS_KEEP_RAW:-}" == 1 ]]; then echo "raw captures and logs kept in $raw"; else rm -rf -- "$raw"; fi
    [[ $keep_demo -eq 1 ]] || rm -rf -- "$demo"
}
trap cleanup EXIT

q() { printf "'%s'" "$1"; } # TOML literal string (no escapes needed for paths)
d() { if [[ -n "$1" ]]; then printf '%s/%s' "$demo" "$1"; else printf '%s' "$demo"; fi; }

write_config() {
    cat >"$data/config.toml" <<EOF
schema = 2

[general]
start_location = $(q "$demo")
language = "en-US"

[appearance]
theme = "dark"

[behaviour]
restore_session = true

[jobs]
auto_open_activity = true

[sidebar]
# No devices / cloud sections: they would list this machine's drives (SOYUZ_DEMO_MASK also
# genericizes volume labels and sizes wherever a volume still shows, e.g. the breadcrumb).
mode = "places"
sections = ["favorites", "portal", "tags", "smart", "trash"]

[[sidebar.portal]]
path = $(q "$(d 'Projects/orbit-tracker')")

[[sidebar.portal]]
path = $(q "$(d 'Photos/2026 Summer trip')")
name = "Summer trip"

[[sidebar.portal]]
path = $(q "$(d 'Documents/Reports')")

[terminal]
shell = "bash"
args = ["-c", "ls -1; exec bash --noprofile --norc"]

[ai]
enabled = true
EOF
}

# write_session <active> <tab>...: each tab is one demo-relative folder, or two joined by
# '|' for a dual-pane tab ("" is the demo root).
write_session() {
    local active="$1"
    shift
    {
        printf 'version = 1\nactive_tab = %s\nwelcome_seen = true\n' "$active"
        printf 'show_hidden = false\ntheme = "dark"\n'
        local favs="" f
        for f in Documents Photos Projects Music Downloads; do
            favs+="${favs:+, }{ name = \"$f\", path = $(q "$(d "$f")") }"
        done
        printf 'favorites = [%s]\n' "$favs"
        local tab pane panes
        for tab in "$@"; do
            IFS='|' read -r -a panes <<<"$tab"
            [[ ${#panes[@]} -eq 0 ]] && panes=("")
            printf '\n[[tabs]]\ndual = %s\nfocused = 0\nsplit = 0.5\n' "$([[ ${#panes[@]} -gt 1 ]] && echo true || echo false)"
            for pane in "${panes[@]}"; do printf '[[tabs.panes]]\nlocation = %s\n' "$(q "$(d "$pane")")"; done
        done
    } >"$data/session.toml"
}

# run_session <name> <steps (';'-separated, {raw} = capture dir)> [ENV=value…]
run_session() {
    local name="$1" steps="$2"
    shift 2
    if [[ ${#only[@]} -gt 0 ]] && ! printf '%s\n' "${only[@]}" | grep -qxF "$name"; then return 0; fi
    echo "== session $name"
    local log="$raw/$name.log" status=0
    (
        cd "$exe_dir"
        env SOYUZ_SCRIPT="${steps//\{raw\}/$raw}" \
            RUST_LOG="info,wgpu_core=warn,wgpu_hal=warn,naga=warn" \
            SOYUZ_DEMO_MASK=1 SOYUZ_WINDOW_POS=-4000,-4000 "$@" \
            timeout --kill-after=5 "$timeout_s" "$exe"
    ) >"$log" 2>&1 || status=$?
    [[ $status -eq 124 || $status -eq 137 ]] && echo "   WARNING: session $name timed out; killed it" >&2
    echo "   saved $(grep -c 'SOYUZ_SCRIPT: saved' "$log" || true) screenshot(s)"
    grep -E 'SOYUZ_SCRIPT.*(cannot|unknown|FAILED)' "$log" | sed 's/^/   WARNING: /' >&2 || true
}

shot() { printf 'wait:300;shot:{raw}/%s' "$1"; }

mkdir -p "$data"
: >"$marker"

# ---- Main tour: the screenshots used by the website and the README.
write_config
write_session 0 'Photos' 'Documents/Reports|Archive/Reports' 'Projects/orbit-tracker' 'Music' 'Documents'
main_steps=(
    'size:1600x1000' 'wait:2500'
    'select:Sunset over the bay.jpg' 'toggle_preview_pane' 'wait:2000' "$(shot hero.png)"
    'toggle_preview_pane'
    'select:Northern lights.jpg' 'quick_look' 'wait:1500' "$(shot quick-look.png)"
    'keys:{Escape}' 'wait:400'
    'select_tab:2' 'wait:1500' "$(shot dual-pane.png)"
    'select_tab:3' 'wait:1200' 'select:README.md' 'toggle_terminal' 'wait:3000' "$(shot terminal.png)"
    'toggle_terminal' 'focus_list' 'wait:300'
    'select_tab:1' 'wait:500' 'command_palette' 'wait:300' 'keys:copy' 'wait:800' "$(shot palette.png)"
    # The first Esc clears the query, the second closes the palette.
    'keys:{Escape}' 'wait:200' 'keys:{Escape}' 'wait:300' 'expect:palette=closed'
    'view_grid' 'theme:light' 'wait:2500' "$(shot light-grid.png)"
    'view_details' 'theme:dark' 'quit'
)
run_session main "$(IFS=';'; echo "${main_steps[*]}")"

# ---- A copy job in flight (throttled) with the activity panel.
write_config
write_session 0 'Downloads|Archive'
activity_steps=(
    'size:1600x1000' 'wait:2500' 'select:Telemetry dump.dat' 'copy_to_other_pane' 'wait:4500' "$(shot jobs.png)"
    'jobs_cancel_all' 'wait:1500' 'quit'
)
run_session activity "$(IFS=';'; echo "${activity_steps[*]}")" SOYUZ_THROTTLE_MBPS=6

# ---- Tools opened at startup (without the big job file, which would fill the treemap).
rm -f -- "$demo/Downloads/Telemetry dump.dat"
write_config
write_session 0 ''
run_session treemap "size:1600x1000;wait:4000;$(shot treemap.png);quit" "SOYUZ_OPEN_TOOL=treemap=$demo"

# ---- Downscale into the output folder (replacing the previous screenshots).
shopt -s nullglob
captures=("$raw"/*.png)
if [[ ${#captures[@]} -gt 0 && ${#only[@]} -eq 0 ]]; then rm -f -- "$out_dir"/*.png; fi
total=0
for src in "${captures[@]}"; do
    dst="$out_dir/$(basename "$src")"
    magick "$src" -filter Lanczos -resize "${width}x>" -alpha off -strip "PNG24:$dst"
    len=$(stat -c %s "$dst")
    total=$((total + len))
    printf '   %-28s %11s %7d KB\n' "$(basename "$src")" "$(magick identify -format '%wx%h' "$dst")" $((len / 1024))
done
pngs=("$out_dir"/*.png)
echo "Screenshots: ${#pngs[@]} files, $((total / 1024)) KB in $out_dir"
