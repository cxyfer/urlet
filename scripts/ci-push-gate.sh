#!/usr/bin/env bash
#
# Decide whether a push to the default branch needs the full CI suite.
#
# A `pull_request` run executes against the merged tree, and the squash or merge
# commit that lands on `master` keeps that tree. When the pull request's latest
# checks all passed, re-running them on the push repeats work that is already
# done. This script recognises exactly that case and nothing else: any doubt,
# API error or unexpected payload runs the suite.
#
# "The pull request passed" has to mean the checks this gate is allowed to skip
# actually ran and passed there, not merely that whatever did run was green. A
# pull request that drops, renames or conditions the real jobs away leaves its
# own green gate check behind, and skipping the push would then hide a suite
# that never ran; the names come from $REQUIRED.
#
# A base branch that moved after the pull request's last run cannot be seen here
# once the pull request is closed. Requiring "branches up to date" in a branch
# ruleset closes that gap; without it, such a merge is skipped like any other.
#
# Writes `run=true|false` to $GITHUB_OUTPUT and exits 0 with a verdict. The
# workflow treats a missing verdict as "run", so a crash never skips CI.
#
# Environment (set by .github/workflows/ci.yml):
#   EVENT    github.event_name
#   REPO     owner/name
#   SHA      github.sha
#   BEFORE   github.event.before
#   FORCED   github.event.forced
#   REQUIRED comma-separated check names the gate is allowed to skip
#   GH_TOKEN token used by `gh api`
#
set -uo pipefail

EVENT="${EVENT:-push}"
REPO="${REPO:-}"
SHA="${SHA:-}"
BEFORE="${BEFORE:-}"
FORCED="${FORCED:-false}"
REQUIRED="${REQUIRED:-}"

run() {
  printf 'run=true\n' >>"${GITHUB_OUTPUT:?}"
  printf '::notice::%s\n' "$*"
  exit 0
}

skip() {
  printf 'run=false\n' >>"${GITHUB_OUTPUT:?}"
  printf '::notice::%s\n' "$*"
  exit 0
}

api() {
  gh api "$@" 2>/dev/null
}

[[ -n "$REPO" && -n "$SHA" ]] || run "gate has no repository or commit to inspect"

# Pull requests are where CI gates a merge, so always run there.
[[ "$EVENT" == "push" ]] || run "not a push; pull requests always run CI"

# A force push replaces history that no run ever checked out.
[[ "$FORCED" == "true" ]] && run "force push; the new history was not tested"

# 1. Which pull request, if any, produced this commit?
pulls="$(api "repos/$REPO/commits/$SHA/pulls")" || run "could not look up the commit's pull requests"
merged="$(jq '[.[] | select(.merged_at != null)] | length' <<<"$pulls" 2>/dev/null)" \
  || run "unexpected pull request payload"
[[ "$merged" == "1" ]] || run "commit is not the merge of exactly one pull request"

read -r pr head_sha merge_sha < <(
  jq -r '[.[] | select(.merged_at != null)][0] | "\(.number) \(.head.sha) \(.merge_commit_sha)"' <<<"$pulls"
)

# 2. The commit itself has to be the merge commit the pull request recorded.
#    A rebase rewrites the commits, and a hand-made merge commit is not the
#    commit the pull request's checks ran against.
[[ -n "$merge_sha" && "$merge_sha" == "$SHA" ]] || run "commit is not the pull request's merge commit"

# 3. The push must have landed that one commit and nothing else. `before` is the
#    previous tip of the branch, so it has to be the merge commit's first parent.
parent="$(api "repos/$REPO/commits/$SHA" | jq -r '.parents[0].sha // empty' 2>/dev/null)" \
  || run "could not read the commit's parent"
[[ -n "$parent" && "$parent" == "$BEFORE" ]] || run "push contains more than the merge commit"

# 4. Every check on the pull request head. A re-run adds another check run with
#    the same name, so the newest attempt decides for each name; a check that is
#    still queued or running runs CI rather than guessing.
checks="$(api "repos/$REPO/commits/$head_sha/check-runs?per_page=100")" \
  || run "could not read the pull request's checks"
total="$(jq -r '.total_count // 0' <<<"$checks" 2>/dev/null)" || run "unexpected check-run payload"
[[ "$total" -gt 0 ]] || run "the pull request has no checks to rely on"
seen="$(jq -r '.check_runs | length' <<<"$checks" 2>/dev/null)" || run "unexpected check-run payload"
[[ "$seen" -ge "$total" ]] || run "too many check runs to judge from one page"
pending="$(jq -r '[.check_runs[] | select(.status != "completed")] | length' <<<"$checks" 2>/dev/null)" \
  || run "unexpected check-run payload"
[[ "$pending" == "0" ]] || run "the pull request's checks are still running"
newest="$(jq -c '
  [.check_runs[]]
  | group_by(.name)
  | map(max_by(.completed_at // "") | {name, conclusion})' <<<"$checks" 2>/dev/null)" \
  || run "unexpected check-run payload"
not_green="$(jq -r '
  [.[] | select(.conclusion != "success" and .conclusion != "neutral" and .conclusion != "skipped")]
  | length' <<<"$newest" 2>/dev/null)" || run "unexpected check-run payload"
[[ "$not_green" == "0" ]] || run "the pull request's checks did not all pass"

# 5. The checks this gate exists to skip have to be among them, and have to have
#    passed. Everything above only says that whatever ran was green, which a
#    pull request that drops, renames or conditions the real jobs away also
#    satisfies — its own gate check is green by itself. Naming them keeps the
#    premise true: the suite ran against this tree and passed.
required_names="$(jq -cn --arg want "$REQUIRED" '
  $want | split(",") | map(gsub("^\\s+|\\s+$"; "")) | map(select(length > 0))' 2>/dev/null)" \
  || run "unexpected required-check list"
[[ "$required_names" != "[]" ]] || run "gate does not know which checks it may skip"
missing="$(jq -r --argjson names "$required_names" '
  [.[] | .name] as $seen
  | [$names[] | . as $n | select(($seen | index($n)) == null)]
  | join(", ")' <<<"$newest" 2>/dev/null)" || run "unexpected check-run payload"
[[ -z "$missing" ]] || run "the pull request never ran: $missing"
# `success` only: a required job that was skipped, cancelled or neutral did not
# prove anything about this tree, so the push runs the suite instead.
unproven="$(jq -r --argjson names "$required_names" '
  [.[] | select(.name as $n | ($names | index($n)) != null) | select(.conclusion != "success") | .name]
  | join(", ")' <<<"$newest" 2>/dev/null)" || run "unexpected check-run payload"
[[ -z "$unproven" ]] || run "the pull request did not pass: $unproven"

# 6. Commit statuses from integrations outside Actions, if there are any.
statuses="$(api "repos/$REPO/commits/$head_sha/status")" || run "could not read commit statuses"
status_total="$(jq -r '.total_count // 0' <<<"$statuses" 2>/dev/null)" \
  || run "unexpected status payload"
status_state="$(jq -r '.state // "pending"' <<<"$statuses" 2>/dev/null)" \
  || run "unexpected status payload"
[[ "$status_total" == "0" || "$status_state" == "success" ]] \
  || run "the pull request's commit statuses are not green"

skip "pull request #$pr already passed its checks at $head_sha"
