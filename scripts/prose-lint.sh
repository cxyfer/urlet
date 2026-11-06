#!/usr/bin/env bash
#
# Reject hard-wrapped prose in Markdown files.
#
#   ./scripts/prose-lint.sh README.md docs/API.md
#   ./scripts/prose-lint.sh $(git ls-files '*.md')
#
# Every paragraph, list item and blockquote stays on one source line. GitHub
# renders a newline inside a paragraph as a space, which leaves stray gaps in
# Chinese text, and inside a release body as a hard line break, which breaks
# sentences in two. Let the editor soft-wrap instead.
#
# The check is structural: a non-empty line that continues the previous
# paragraph, bullet or blockquote is an error. Fenced code, HTML comments,
# headings, tables (with or without the outer pipes), rules and raw HTML are
# ignored.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

usage() { sed -n '3p;5,6p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

if [[ $# -eq 0 ]]; then usage; exit 2; fi
for file in "$@"; do
    [[ -f "$file" ]] || { echo "error: $file not found" >&2; exit 1; }
done

exec awk '
function fail(previous, current) {
    printf "%s:%d: hard-wrapped prose (line %d continues here)\n", FILENAME, current, previous
    failed = 1
}
BEGIN { failed = 0 }
FNR == 1 { fence = 0; comment = 0; table = 0; prev = 0; last = "" }
{
    line = $0
    previous = last
    last = line

    if (line ~ /^[[:space:]]*(```|~~~)/) { fence = !fence; table = 0; prev = 0; next }
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

    if (line ~ /^[[:space:]]*$/) { table = 0; prev = 0; next }

    # Continue a GFM table; its rows may omit the outer pipes.
    if (table) {
        if (line ~ /\|/ && line !~ /^[[:space:]]*(#|>|<)/) { prev = 0; next }
        table = 0
    }

    # A GFM delimiter row can omit the outer pipes (`--- | ---`). The header
    # row above it (`Header | Header`) looks like prose, so pair them here.
    if (line ~ /^[[:space:]|:-]+$/ && line ~ /-/ && line ~ /\|/ && previous ~ /\|/) {
        table = 1
        prev = 0
        next
    }

    # Headings, rules, table rows and raw HTML open a block of their own.
    if (line ~ /^[[:space:]]*(#|[-*_][-*_][-*_][-*_]*[[:space:]]*$|[|]|<)/) { prev = 0; next }

    if (line ~ /^[[:space:]]*>/) {
        quote = line
        sub(/^[[:space:]]*>[[:space:]]?/, "", quote)
        if (quote == "" || quote ~ /^\[![A-Z]+\]/) { prev = 0; next }
        if (quote ~ /^([*+-]|[0-9]+[.)])[[:space:]]/) { prev = 2; next }
        if (prev == 2) fail(FNR - 1, FNR)
        prev = 2
        next
    }

    if (line ~ /^[[:space:]]*([*+-]|[0-9]+[.)])[[:space:]]/) { prev = 1; next }

    if (prev == 1) fail(FNR - 1, FNR)
    prev = 1
}
END { exit failed }
' "$@"
