#!/usr/bin/env bash
#
# Reject hard-wrapped prose in a release-notes file.
#
#   ./scripts/release-notes-lint.sh .tmp/release-notes-v1.3.0.md
#
# GitHub renders a newline inside a release body as a hard line break, so the
# notes follow the same one-line-per-paragraph rule as every Markdown file in
# the repository; see scripts/prose-lint.sh for the check itself.
#
set -euo pipefail

if [[ $# -ne 1 ]]; then sed -n '3p;5p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 2; fi
exec "$(dirname "${BASH_SOURCE[0]}")/prose-lint.sh" "$1"
