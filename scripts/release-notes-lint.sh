#!/usr/bin/env bash
#
# Reject hard-wrapped prose in a release-notes file.
#
#   ./scripts/release-notes-lint.sh .tmp/release-notes-v1.3.0.md
#
# GitHub renders a newline inside a release body as a hard line break, so a
# paragraph wrapped at 80 columns reaches the reader broken mid-sentence. Keep
# every callout paragraph, highlight bullet and generated entry on one line and
# let the editor soft-wrap instead.
#
# The check is structural: a non-empty line that continues the previous
# paragraph, bullet or blockquote is an error. Fenced code, HTML comments,
# headings, tables, rules and raw HTML are ignored.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

usage() { sed -n '3p;5p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

if [[ $# -ne 1 ]]; then usage; exit 2; fi
FILE="$1"
[[ -f "$FILE" ]] || { echo "error: $FILE not found" >&2; exit 1; }

exec awk -v file="$FILE" '
function fail(previous, current) {
    printf "%s:%d: hard-wrapped prose (line %d continues here)\n", file, current, previous
    failed = 1
}
BEGIN { fence = 0; comment = 0; prev = 0; failed = 0 }
{
    line = $0

    if (line ~ /^[[:space:]]*(```|~~~)/) { fence = !fence; prev = 0; next }
    if (fence) next

    if (comment) {
        if (line ~ /-->/) comment = 0
        prev = 0
        next
    }
    if (line ~ /^[[:space:]]*<!--/) {
        if (line !~ /-->/) comment = 1
        prev = 0
        next
    }

    if (line ~ /^[[:space:]]*$/) { prev = 0; next }

    # Headings, rules, tables and raw HTML open a block of their own.
    if (line ~ /^[[:space:]]*(#|[-*_][-*_][-*_][-*_]*[[:space:]]*$|[|]|<)/) { prev = 0; next }

    if (line ~ /^[[:space:]]*>/) {
        quote = line
        sub(/^[[:space:]]*>[[:space:]]?/, "", quote)
        if (quote == "" || quote ~ /^\[![A-Z]+\]/) { prev = 0; next }
        if (quote ~ /^([*+-]|[0-9]+[.)])[[:space:]]/) { prev = 2; next }
        if (prev == 2) fail(NR - 1, NR)
        prev = 2
        next
    }

    if (line ~ /^[[:space:]]*([*+-]|[0-9]+[.)])[[:space:]]/) { prev = 1; next }

    if (prev == 1) fail(NR - 1, NR)
    prev = 1
}
END { exit failed }
' "$FILE"
