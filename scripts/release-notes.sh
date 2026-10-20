#!/usr/bin/env bash
#
# Scaffold the standard release notes for a tag.
#
#   ./scripts/release-notes.sh v1.3.0-rc.1
#   ./scripts/release-notes.sh v1.3.0-rc.1 v1.2.0     # explicit previous tag
#
# Writes .tmp/release-notes-<tag>.md (override with NOTES_OUT). The summary and
# the highlights are left as TODOs for the maintainer; What's Changed, New
# Contributors and Full Changelog are filled in from the commits and pull
# requests between the two tags.
#
# Why not GitHub's "Generate release notes": it matches pull requests by merge
# commit, and this repository rebases before merging. A rewritten merge commit
# silently drops the pull request (#12 and #14 were missing for v1.3.0-rc.1).
# The `(#N)` suffix in the commit subjects is the source of truth here.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

usage() { sed -n '3p;5,6p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

TAG="${1:-}"
PREV="${2:-}"
if [[ -z "$TAG" ]]; then usage; exit 2; fi

git rev-parse -q --verify "refs/tags/$TAG" >/dev/null \
    || { echo "error: tag '$TAG' not found locally (run git fetch --tags)" >&2; exit 1; }

if [[ -z "$PREV" ]]; then
    PREV="$(git describe --tags --abbrev=0 "$TAG^" 2>/dev/null || true)"
fi
[[ -n "$PREV" ]] \
    || { echo "error: cannot work out the previous tag; pass it as the second argument" >&2; exit 1; }
git rev-parse -q --verify "refs/tags/$PREV" >/dev/null \
    || { echo "error: previous tag '$PREV' not found locally" >&2; exit 1; }

# The tag is the release contract; a mismatch means the workflow will reject it.
version="$(git show "$TAG:Cargo.toml" | awk -F'"' '/^version = / { print $2; exit }')"
[[ "v$version" == "$TAG" ]] \
    || echo "warning: $TAG does not match Cargo.toml ($version)" >&2

REPO="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"
OUT="${NOTES_OUT:-.tmp/release-notes-$TAG.md}"
if [[ -e "$OUT" ]]; then
    echo "error: $OUT already exists; move it aside or set NOTES_OUT to keep the edits" >&2
    exit 1
fi
mkdir -p "$(dirname "$OUT")"

CHANGES="$(mktemp)"
AUTHORS="$(mktemp)"
NEWCOMERS="$(mktemp)"
trap 'rm -f "$CHANGES" "$AUTHORS" "$NEWCOMERS"' EXIT

# ---------------------------------------------------------------- what changed
while IFS=$'\t' read -r sha subject; do
    # The release commit is not a change.
    [[ "$subject" == "🔖 release:"* ]] && continue

    if [[ "$subject" =~ \(#([0-9]+)\)$ ]]; then
        pr="${BASH_REMATCH[1]}"
        IFS=$'\t' read -r title author url < <(
            gh pr view "$pr" --repo "$REPO" --json title,author,url \
                --jq '[.title, .author.login, .url] | @tsv'
        )
        printf '* %s by @%s in %s\n' "$title" "$author" "$url" >> "$CHANGES"
        printf '%s\t%s\n' "$author" "$url" >> "$AUTHORS"
    else
        # A direct commit: no pull request to link to, so name the SHA.
        printf '* %s (%s)\n' "$subject" "$sha" >> "$CHANGES"
    fi
done < <(git log --no-merges --reverse --format='%h%x09%s' "$PREV..$TAG")

# ------------------------------------------------------------- new contributors
# Approximate: anyone whose first merged pull request predates the previous tag
# is a returning contributor. A PR merged on the same day as the previous tag
# can be miscounted, which is rare enough to accept.
prev_date="$(git log -1 --format=%cs "$PREV")"
while IFS=$'\t' read -r author url; do
    [[ -n "$author" ]] || continue
    prior="$(gh api -X GET search/issues \
        -f q="repo:$REPO is:pr is:merged author:$author merged:<$prev_date" \
        --jq .total_count)"
    if [[ "$prior" == "0" ]]; then
        printf '* @%s made their first contribution in %s\n' "$author" "$url" >> "$NEWCOMERS"
    fi
done < <(sort -u -k1,1 "$AUTHORS")

# ------------------------------------------------------------------------ write
{
    cat <<'EOF'
> [!IMPORTANT]
> TODO: one paragraph on what this release is and why it matters.
>
> TODO: what changes for an existing install — migrations that run at startup, defaults that
> moved, anything the reader must do before or after upgrading. Delete this paragraph when
> there is nothing to say.

## Highlights

<!-- One bullet per area. Bold area name, then plain user-facing prose: what changed and what
     it means for the reader. Cover features, behaviour changes and the fixes worth knowing
     about. Do not paste commit subjects here. -->

* **Area**: TODO

---

## What's Changed
EOF
    cat "$CHANGES"

    if [[ -s "$NEWCOMERS" ]]; then
        printf '\n## New Contributors\n'
        cat "$NEWCOMERS"
    fi

    printf '\n**Full Changelog**: https://github.com/%s/compare/%s...%s\n' "$REPO" "$PREV" "$TAG"
} > "$OUT"

echo "wrote $OUT ($PREV -> $TAG)"
