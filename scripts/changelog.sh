#!/usr/bin/env bash
# Write CHANGELOG.md from `git log`, grouped the same way as cliff.toml.
#
# Uses git-cliff when it is installed (`cargo install git-cliff`); otherwise groups the
# commits itself. Only reads the repository (git log / git tag); it never commits.
# Commits between two tags form one release; commits after the newest tag are "Unreleased".
# This is the bash port of scripts/changelog.ps1 and produces byte-identical output.
#
# Usage: scripts/changelog.sh [--output <file>] [--no-cliff]
#   --output <file>   target file (default: CHANGELOG.md in the repository root)
#   --no-cliff        use the built-in grouping even when git-cliff is available

set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="$repo/CHANGELOG.md"
no_cliff=0

while [ $# -gt 0 ]; do
    case "$1" in
        -o | --output)
            output="$2"
            shift 2
            ;;
        --no-cliff)
            no_cliff=1
            shift
            ;;
        -h | --help)
            sed -n '2,11p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "unknown option: $1" >&2
            exit 2
            ;;
    esac
done

if [ "$no_cliff" -eq 0 ] && command -v git-cliff >/dev/null 2>&1; then
    git -C "$repo" cliff --config "$repo/cliff.toml" -o "$output"
    echo "CHANGELOG written by git-cliff: $output"
    exit 0
fi

# Same order and names as cliff.toml's commit_parsers. Matching is case-insensitive,
# like PowerShell's -match in changelog.ps1.
group_names=(
    'Milestones (M0–M13)'
    'Rigidity (R)'
    'Looks (B)'
    'Features (F)'
    'AI (A)'
    'Distribution (D)'
    'Other features'
    'Fixes and security'
    'Documentation'
    'Other'
)
group_patterns=(
    '^M[0-9]'
    '^R[0-9]'
    '^B[0-9]'
    '^(F[0-9]|fonts)'
    '^(A[0-9]|AI )'
    '^D[0-9]'
    '^(feat|feature)'
    '^(fix|security)'
    '^docs'
    '.*'
)

sep=$'\x1f'

# Prints one release section for the commits in <range> (empty = all of history).
format_release() {
    local title="$1" range="$2"
    local -a ids=() subjects=() taken=()
    local id date subject
    local -a args=(-C "$repo" log --no-merges --reverse "--format=%h${sep}%ad${sep}%s" --date=short)
    [ -n "$range" ] && args+=("$range")
    while IFS="$sep" read -r id date subject; do
        ids+=("$id")
        subjects+=("$subject")
        taken+=(0)
        last_date="$date"
    done < <(git "${args[@]}")

    [ -z "$title" ] && title="Unreleased ($version, last change $last_date)"
    printf '## %s\n\n' "$title"
    local g i any
    shopt -s nocasematch
    for g in "${!group_names[@]}"; do
        any=0
        for i in "${!ids[@]}"; do
            [ "${taken[$i]}" -eq 1 ] && continue
            if [[ ${subjects[$i]} =~ ${group_patterns[$g]} ]]; then
                if [ "$any" -eq 0 ]; then
                    printf '### %s\n\n' "${group_names[$g]}"
                    any=1
                fi
                taken[i]=1
                printf -- '- %s (%s)\n' "${subjects[$i]}" "${ids[$i]}"
            fi
        done
        [ "$any" -eq 1 ] && printf '\n'
    done
    shopt -u nocasematch
}

version="$(sed -nE 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "$repo/Cargo.toml" | head -n1)"
mapfile -t tags < <(git -C "$repo" tag --sort=-creatordate)
last_date=""

text="$(
    printf '# Changelog\n\n'
    printf 'All notable changes to Soyuz. Grouped by the commit prefix: M = the\n'
    printf 'original milestones, R = rigidity, B = looks, F = features, A = AI,\n'
    printf 'D = distribution. Generated from `git log` by `scripts/changelog.ps1` (or `git cliff`\n'
    printf 'with `cliff.toml`); commit hashes refer to the main branch.\n\n'

    if [ "${#tags[@]}" -eq 0 ]; then
        # An empty title means "Unreleased (<version>, last change <date>)".
        format_release '' ''
    else
        if [ -n "$(git -C "$repo" log --no-merges --format=%h "${tags[0]}..HEAD")" ]; then
            format_release 'Unreleased' "${tags[0]}..HEAD"
        fi
        for i in "${!tags[@]}"; do
            if [ $((i + 1)) -lt "${#tags[@]}" ]; then
                range="${tags[$((i + 1))]}..${tags[$i]}"
            else
                range="${tags[$i]}"
            fi
            date="$(git -C "$repo" log -1 --format=%ad --date=short "${tags[$i]}")"
            format_release "${tags[$i]#v} ($date)" "$range"
        done
    fi
)"

# $(...) already trimmed the trailing newlines; end the file with exactly one.
printf '%s\n' "$text" >"$output"
echo "CHANGELOG written: $output"
