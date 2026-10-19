#!/usr/bin/env bash
#
# Post-merge acceptance check for #12 (user groups), #13 (multi-domain) and
# #14 (per-group default domains and allowlists).
#
# Boots the real binary against a throwaway SQLite database and drives the
# combined behaviour through the HTTP API with curl. Exits non-zero when any
# check fails.
#
#   ./scripts/merge-check.sh                 # uses ./target/debug/urlet
#   BIN=./target/release/urlet ./scripts/merge-check.sh
#
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

BIN="${BIN:-$ROOT/target/debug/urlet}"
PORT="${PORT:-18211}"
BASE="http://127.0.0.1:${PORT}"
DATA="$(mktemp -d)"
ADMIN_JAR="$DATA/admin.jar"
MEMBER_JAR="$DATA/member.jar"
BODY=""
STATUS=""

PASS=0
FAIL=0

pass() { PASS=$((PASS + 1)); printf '  \033[32m✓\033[0m %s\n' "$1"; }
fail() { FAIL=$((FAIL + 1)); printf '  \033[31m✗\033[0m %s\n' "$1"; }

check() { # check <description> <expected> <actual>
    if [ "$2" = "$3" ]; then pass "$1"; else fail "$1 (expected '$2', got '$3')"; fi
}

contains() { # contains <description> <needle> <haystack>
    case "$3" in *"$2"*) pass "$1" ;; *) fail "$1 (missing '$2' in '${3:0:300}')" ;; esac
}

section() { printf '\n\033[1m%s\033[0m\n' "$1"; }

cleanup() {
    [ -n "${SERVER_PID:-}" ] && kill "$SERVER_PID" 2>/dev/null
    rm -rf "$DATA"
}
trap cleanup EXIT

if [ ! -x "$BIN" ]; then
    echo "building $BIN ..."
    cargo build --quiet || exit 1
fi

# request <method> <path> [json] [jar] [host]
# Fills BODY and STATUS. Sends the same-origin header a browser would send.
request() {
    local method="$1" path="$2" json="${3:-}" jar="${4:-$ADMIN_JAR}" host="${5:-}"
    local args=(-sS -X "$method" -b "$jar" -c "$jar" -H "origin: $BASE")
    [ -n "$json" ] && args+=(-H 'content-type: application/json' -d "$json")
    [ -n "$host" ] && args+=(-H "Host: $host")
    local raw
    raw="$(curl "${args[@]}" -w $'\n%{http_code}' "$BASE$path")"
    STATUS="${raw##*$'\n'}"
    BODY="${raw%$'\n'*}"
}

field() { printf '%s' "$BODY" | jq -r "$1"; }

echo "urlet post-merge acceptance (#12 + #13 + #14)"
echo "  binary: $BIN"
echo "  base:   $BASE"
echo

URLET_DATABASE_URL="sqlite://$DATA/check.db?mode=rwc" \
URLET_DATA_DIR="$DATA" \
URLET_BIND="127.0.0.1:$PORT" \
URLET_BASE_URL="$BASE" \
URLET_DOMAINS="go.example.com,links.example.com" \
URLET_LOG=warn \
"$BIN" serve >> "$DATA/server.log" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 60); do
    curl -sf -o /dev/null "$BASE/health" && break
    sleep 0.2
done
if ! curl -sf -o /dev/null "$BASE/health"; then
    echo "server did not start; log:" >&2
    cat "$DATA/server.log" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
section "#13 multi-domain: env domains are seeded, links live per host"
# ---------------------------------------------------------------------------
request POST /api/v1/auth/register '{"email":"owner@example.com","password":"password123","display_name":"Owner"}' "$ADMIN_JAR"
check "first registration becomes the administrator" 201 "$STATUS"
request GET /api/v1/auth/me '' "$ADMIN_JAR"
check "administrator role" admin "$(field .user.role)"
SITE_HOST="$(field .limits.default_domain)"

request GET /api/v1/admin/domains
check "admin domains listing" 200 "$STATUS"
GO_ID="$(printf '%s' "$BODY" | jq -r '.[] | select(.hostname=="go.example.com") | .id')"
LINKS_ID="$(printf '%s' "$BODY" | jq -r '.[] | select(.hostname=="links.example.com") | .id')"
check "go.example.com is active" active "$(printf '%s' "$BODY" | jq -r '.[] | select(.hostname=="go.example.com") | .status')"
check "links.example.com is active" active "$(printf '%s' "$BODY" | jq -r '.[] | select(.hostname=="links.example.com") | .status')"

request POST /api/v1/urls '{"target_url":"https://example.com/promo","slug":"promo","domain":"go.example.com"}' "$ADMIN_JAR"
check "link created on go.example.com" 201 "$STATUS"
check "short URL uses the chosen host" "http://go.example.com/promo" "$(field .short_url)"

request GET /promo '' "$ADMIN_JAR" "go.example.com"
check "go.example.com/promo redirects" 302 "$STATUS"
request GET /promo '' "$ADMIN_JAR" "links.example.com"
check "same slug is not visible on links.example.com" 404 "$STATUS"

request POST /api/v1/urls '{"target_url":"https://example.com/promo2","slug":"promo","domain":"links.example.com"}' "$ADMIN_JAR"
check "the same slug can live on a second domain" 201 "$STATUS"
request GET /promo '' "$ADMIN_JAR" "links.example.com"
check "links.example.com/promo now redirects" 302 "$STATUS"
check "both hosts keep their own target" "https://example.com/promo2" \
    "$(curl -s -o /dev/null -D - -H 'Host: links.example.com' "$BASE/promo" | tr -d '\r' | awk -F': ' 'tolower($1)=="location"{print $2}')"

# ---------------------------------------------------------------------------
section "#12 user groups: quotas and feature toggles apply to members"
# ---------------------------------------------------------------------------
request POST /api/v1/admin/groups \
    '{"name":"Strict Team","max_urls":2,"allow_custom_slug":false,"allow_password":false,"allow_expiry":true}' \
    "$ADMIN_JAR"
check "group created with a link quota and toggles" 201 "$STATUS"
GROUP_ID="$(field .id)"
check "quota stored" 2 "$(field .max_urls)"
check "custom slugs disabled" false "$(field .allow_custom_slug)"
check "password links disabled" false "$(field .allow_password)"

request POST /api/v1/auth/register '{"email":"member@example.com","password":"password123"}' "$MEMBER_JAR"
check "member registration" 201 "$STATUS"
MEMBER_ID="$(field .user.id)"
request GET /api/v1/auth/me '' "$MEMBER_JAR"
check "member starts as a normal user" user "$(field .user.role)"

request PATCH "/api/v1/admin/users/$MEMBER_ID" "{\"group_id\":$GROUP_ID}" "$ADMIN_JAR"
check "member assigned to the group" 200 "$STATUS"
request GET /api/v1/auth/me '' "$MEMBER_JAR"
check "member inherits the quota" 2 "$(field .limits.max_urls)"

request POST /api/v1/urls '{"target_url":"https://example.com/one","slug":"chosen"}' "$MEMBER_JAR"
check "custom slug rejected by the toggle" 403 "$STATUS"
check "rejection is a forbidden error" forbidden "$(field .error.code)"

request POST /api/v1/urls '{"target_url":"https://example.com/secret","password":"linkpass"}' "$MEMBER_JAR"
check "password link rejected by the toggle" 403 "$STATUS"

request POST /api/v1/urls '{"target_url":"https://example.com/one"}' "$MEMBER_JAR"
check "member link 1 of 2" 201 "$STATUS"
request POST /api/v1/urls '{"target_url":"https://example.com/two"}' "$MEMBER_JAR"
check "member link 2 of 2" 201 "$STATUS"
request POST /api/v1/urls '{"target_url":"https://example.com/three"}' "$MEMBER_JAR"
check "third link exceeds the group quota" 403 "$STATUS"
contains "quota message names the limit" "limit of 2 links" "$(field .error.message)"

# ---------------------------------------------------------------------------
section "#14 per-group default domain + allowlist (the three merges combined)"
# ---------------------------------------------------------------------------
request POST /api/v1/admin/groups \
    "{\"name\":\"Domain Team\",\"default_domain_id\":$GO_ID,\"restrict_domains\":true,\"domain_ids\":[$GO_ID,$LINKS_ID]}" \
    "$ADMIN_JAR"
check "group default domain and allowlist saved" 201 "$STATUS"
DOMAIN_GROUP_ID="$(field .id)"
check "group default domain" "$GO_ID" "$(field .default_domain_id)"
check "restriction enabled" true "$(field .restrict_domains)"
check "allowlist has two entries" 2 "$(field '.allowed_domain_ids | length')"

request PATCH "/api/v1/admin/users/$MEMBER_ID" "{\"group_id\":$DOMAIN_GROUP_ID}" "$ADMIN_JAR"
check "member moved to the domain group" 200 "$STATUS"

request GET /api/v1/auth/me '' "$MEMBER_JAR"
check "member default domain comes from the group" "go.example.com" "$(field .limits.default_domain)"
check "member sees only two allowed domains" 2 "$(field '.limits.allowed_domains | length')"

request POST /api/v1/urls '{"target_url":"https://example.com/group-default"}' "$MEMBER_JAR"
check "link without a domain succeeds" 201 "$STATUS"
check "it lands on the group default, not the site default" "go.example.com" "$(field .domain)"
GROUP_SLUG="$(field .slug)"

request POST /api/v1/urls '{"target_url":"https://example.com/on-links","domain":"links.example.com"}' "$MEMBER_JAR"
check "explicitly allowed domain accepted" 201 "$STATUS"
check "link stored on the allowed domain" "links.example.com" "$(field .domain)"

request POST /api/v1/urls "{\"target_url\":\"https://example.com/blocked\",\"domain\":\"$SITE_HOST\"}" "$MEMBER_JAR"
check "canonical host outside the allowlist rejected" 403 "$STATUS"

request GET "/$GROUP_SLUG" '' "$MEMBER_JAR" "go.example.com"
check "group link redirects on its own host" 302 "$STATUS"

request GET /api/v1/domains '' "$MEMBER_JAR"
check "member domain picker lists the allowlist" 2 "$(printf '%s' "$BODY" | jq 'length')"
check "group default is flagged in the picker" true \
    "$(printf '%s' "$BODY" | jq -r '.[] | select(.hostname=="go.example.com") | .is_default')"

# ---------------------------------------------------------------------------
printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"
if [ "$FAIL" -ne 0 ]; then
    echo "server log: $DATA/server.log" >&2
    exit 1
fi
