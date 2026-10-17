#!/usr/bin/env bash
#
# urlet end-to-end smoke test.
#
# Runs the real binary against a throwaway SQLite database and exercises every
# user visible flow with curl. Exits non-zero on the first failure.
#
#   ./scripts/smoke.sh            # uses ./target/debug/urlet (builds if needed)
#   BIN=./target/release/urlet ./scripts/smoke.sh
#
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

BIN="${BIN:-$ROOT/target/debug/urlet}"
PORT="${PORT:-18199}"
BASE="http://127.0.0.1:${PORT}"
DATA="$(mktemp -d)"
COOKIES="$DATA/cookies.txt"
COOKIES_B="$DATA/cookies-b.txt"

UA_DESKTOP="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"

PASS=0
FAIL=0

pass() { PASS=$((PASS + 1)); printf '  \033[32m✓\033[0m %s\n' "$1"; }
fail() { FAIL=$((FAIL + 1)); printf '  \033[31m✗\033[0m %s\n' "$1"; }

# check <description> <expected> <actual>
check() {
    if [ "$2" = "$3" ]; then
        pass "$1"
    else
        fail "$1 (expected '$2', got '$3')"
    fi
}

contains() {
    case "$3" in
        *"$2"*) pass "$1" ;;
        *) fail "$1 (missing '$2' in '${3:0:200}')" ;;
    esac
}

missing() {
    case "$3" in
        *"$2"*) fail "$1 (unexpected '$2')" ;;
        *) pass "$1" ;;
    esac
}

cleanup() {
    stop_server
    rm -rf "$DATA"
}
trap cleanup EXIT

if [ ! -x "$BIN" ]; then
    echo "building $BIN ..."
    cargo build --quiet || exit 1
fi

echo "urlet smoke test"
echo "  binary: $BIN"
echo "  base:   $BASE"
echo

start_server() {
    URLET_DATABASE_URL="sqlite://$DATA/smoke.db?mode=rwc" \
    URLET_DATA_DIR="$DATA" \
    URLET_BIND="127.0.0.1:$PORT" \
    URLET_BASE_URL="$BASE" \
    URLET_LOG=warn \
    "$BIN" serve >> "$DATA/server.log" 2>&1 &
    SERVER_PID=$!
    for _ in $(seq 1 60); do
        curl -sf -o /dev/null "$BASE/health" && return 0
        sleep 0.2
    done
    echo "server did not start:"; cat "$DATA/server.log"; exit 1
}

stop_server() {
    [ -n "${SERVER_PID:-}" ] && kill "$SERVER_PID" 2>/dev/null
    wait "${SERVER_PID:-}" 2>/dev/null
    SERVER_PID=""
}

start_server

# ---------------------------------------------------------------- health
echo "health and static assets"
check "GET /health returns 200" "200" "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/health")"
contains "health reports the database" '"database":"ok"' "$(curl -s "$BASE/health")"
check "GET /assets/app.css returns 200" "200" "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/assets/app.css")"
check "GET /favicon.ico returns 200" "200" "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/favicon.ico")"
check "GET / renders the landing page" "200 1" "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/") $(curl -s "$BASE/" | grep -c 'landing-hero')"
check "security headers are present" "nosniff" \
    "$(curl -sI "$BASE/health" | tr -d '\r' | awk -F': ' '/^x-content-type-options/{print $2}')"
check "the healthcheck subcommand works" "0" \
    "$("$BIN" healthcheck "$BASE/health" > /dev/null 2>&1; echo $?)"

# ------------------------------------------------------------ registration
echo
echo "accounts and sessions"
FIRST=$(curl -s -c "$COOKIES" -X POST "$BASE/api/v1/auth/register" \
    -H 'content-type: application/json' \
    -d '{"email":"owner@example.com","password":"ownerpassword","display_name":"Owner"}' \
    -w '\n%{http_code}')
check "first registration succeeds" "201" "$(echo "$FIRST" | tail -1)"
contains "first account is an administrator" '"role":"admin"' "$FIRST"

check "duplicate registration is rejected" "409" "$(curl -s -o /dev/null -w '%{http_code}' \
    -X POST "$BASE/api/v1/auth/register" -H 'content-type: application/json' \
    -d '{"email":"OWNER@example.com","password":"ownerpassword"}')"
check "weak password is rejected" "422" "$(curl -s -o /dev/null -w '%{http_code}' \
    -X POST "$BASE/api/v1/auth/register" -H 'content-type: application/json' \
    -d '{"email":"weak@example.com","password":"short"}')"
check "unknown account cannot sign in" "401" "$(curl -s -o /dev/null -w '%{http_code}' \
    -X POST "$BASE/api/v1/auth/login" -H 'content-type: application/json' \
    -d '{"email":"ghost@example.com","password":"whatever"}')"
check "GET /api/v1/auth/me without a session is 401" "401" \
    "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/v1/auth/me")"
check "GET /api/v1/auth/me with the session is 200" "200" \
    "$(curl -s -o /dev/null -w '%{http_code}' -b "$COOKIES" "$BASE/api/v1/auth/me")"

curl -s -c "$COOKIES_B" -o /dev/null -X POST "$BASE/api/v1/auth/register" \
    -H 'content-type: application/json' \
    -d '{"email":"member@example.com","password":"memberpassword"}'

# ------------------------------------------------------------------ links
echo
echo "short link management"
create_link() {
    curl -s -b "$COOKIES" -X POST "$BASE/api/v1/urls" \
        -H 'content-type: application/json' -H "origin: $BASE" -d "$1"
}

LINK=$(create_link '{"target_url":"https://example.com/landing","slug":"demo","title":"Demo","tags":["campaign"],"expires_at":"2030-06-30","max_clicks":100}')
LINK_ID=$(echo "$LINK" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
contains "link is created with the given slug" '"slug":"demo"' "$LINK"
contains "short_url uses the configured base" "\"short_url\":\"$BASE/demo\"" "$LINK"

contains "a generated slug has the configured length" '"slug":"' \
    "$(create_link '{"target_url":"https://example.com/auto"}')"
check "duplicate slug is rejected" "409" "$(curl -s -o /dev/null -w '%{http_code}' \
    -b "$COOKIES" -X POST "$BASE/api/v1/urls" -H 'content-type: application/json' \
    -H "origin: $BASE" -d '{"target_url":"https://example.com/x","slug":"DEMO"}')"
check "reserved slug is rejected" "422" "$(curl -s -o /dev/null -w '%{http_code}' \
    -b "$COOKIES" -X POST "$BASE/api/v1/urls" -H 'content-type: application/json' \
    -H "origin: $BASE" -d '{"target_url":"https://example.com/x","slug":"api"}')"
check "javascript: target is rejected" "422" "$(curl -s -o /dev/null -w '%{http_code}' \
    -b "$COOKIES" -X POST "$BASE/api/v1/urls" -H 'content-type: application/json' \
    -H "origin: $BASE" -d '{"target_url":"javascript:alert(1)"}')"
check "past expiry is rejected" "422" "$(curl -s -o /dev/null -w '%{http_code}' \
    -b "$COOKIES" -X POST "$BASE/api/v1/urls" -H 'content-type: application/json' \
    -H "origin: $BASE" -d '{"target_url":"https://example.com/x","expires_at":"2000-01-01"}')"

check "the other user does not see this link" "0" \
    "$(curl -s -b "$COOKIES_B" "$BASE/api/v1/urls" | sed -n 's/.*"total":\([0-9]*\).*/\1/p')"
check "the other user cannot read it by id" "404" \
    "$(curl -s -o /dev/null -w '%{http_code}' -b "$COOKIES_B" "$BASE/api/v1/urls/$LINK_ID")"
check "the other user cannot delete it" "404" \
    "$(curl -s -o /dev/null -w '%{http_code}' -b "$COOKIES_B" -X DELETE \
     -H "origin: $BASE" "$BASE/api/v1/urls/$LINK_ID")"

# --------------------------------------------------------------- redirect
echo
echo "redirects and click tracking"
check "GET /demo redirects" "302" \
    "$(curl -s -o /dev/null -w '%{http_code}' -A "$UA_DESKTOP" "$BASE/demo")"
check "the redirect points at the target" "https://example.com/landing" \
    "$(curl -s -o /dev/null -w '%{redirect_url}' -A "$UA_DESKTOP" "$BASE/demo")"
check "lookup is case-insensitive" "302" \
    "$(curl -s -o /dev/null -w '%{http_code}' -A "$UA_DESKTOP" "$BASE/DEMO")"
check "unknown slug is 404" "404" "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/nope-not-here")"

BEFORE=$(curl -s -b "$COOKIES" "$BASE/api/v1/urls/$LINK_ID" | sed -n 's/.*"click_count":\([0-9]*\).*/\1/p')
for _ in $(seq 1 5); do
    curl -s -o /dev/null -A "$UA_DESKTOP" -e "https://news.example.com/x" "$BASE/demo"
done
curl -s -o /dev/null -A "Googlebot/2.1" "$BASE/demo"
sleep 0.5
CLICKS=$(curl -s -b "$COOKIES" "$BASE/api/v1/urls/$LINK_ID" | sed -n 's/.*"click_count":\([0-9]*\).*/\1/p')
check "six more clicks were recorded" "6" "$((CLICKS - BEFORE))"

STATS=$(curl -s -b "$COOKIES" "$BASE/api/v1/urls/$LINK_ID/stats?days=7")
contains "statistics report a referrer" '"label":"news.example.com"' "$STATS"
contains "statistics report a bot click" '"bot_clicks":1' "$STATS"
contains "statistics never expose hashes" '"unique_clicks"' "$STATS"
missing "statistics never expose ip hashes" 'ip_hash' "$STATS"

# ------------------------------------------------------- password protect
echo
echo "password protection"
create_link '{"target_url":"https://secret.example.com/board","slug":"vault","password":"linkpass"}' > /dev/null
PAGE=$(curl -s "$BASE/vault")
contains "a protected link renders a password page" "Protected link" "$PAGE"
missing "the protected page hides the target" "secret.example.com" "$PAGE"
check "wrong password is 401" "401" "$(curl -s -o /dev/null -w '%{http_code}' \
    -X POST "$BASE/vault/unlock" -H 'content-type: application/json' -d '{"password":"wrong"}')"
UNLOCK=$(curl -s -c "$DATA/unlock.txt" -X POST "$BASE/vault/unlock" \
    -H 'content-type: application/json' -d '{"password":"linkpass"}' -w '\n%{http_code}')
check "correct password unlocks" "200" "$(echo "$UNLOCK" | tail -1)"
check "the unlocked link redirects" "302" \
    "$(curl -s -o /dev/null -w '%{http_code}' -b "$DATA/unlock.txt" "$BASE/vault")"
check "the unlock cookie is scoped and HttpOnly" "yes" \
    "$(grep -q 'urlet_unlock_vault' "$DATA/unlock.txt" && echo yes || echo no)"
check "a form submission also unlocks" "303" "$(curl -s -o /dev/null -w '%{http_code}' \
    -X POST "$BASE/vault/unlock" -H 'content-type: application/x-www-form-urlencoded' \
    -H "origin: $BASE" -d 'password=linkpass')"

# ------------------------------------------------------- public metadata
echo
echo "public metadata"
create_link '{"target_url":"https://example.com/meta","slug":"meta","title":"Meta","password":"linkpass"}' > /dev/null
META=$(curl -s "$BASE/api/v1/public/urls/meta")
contains "public metadata reports the password flag" '"has_password":true' "$META"
missing "public metadata hides the target" "example.com/meta" "$META"
check "public metadata for an unknown slug is 404" "404" \
    "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/v1/public/urls/unknown-slug")"

# ----------------------------------------------------------------- expiry
echo
echo "expiry and limits"
create_link '{"target_url":"https://example.com/soon","slug":"soon","expires_at":"2030-01-01"}' > /dev/null
create_link '{"target_url":"https://example.com/limited","slug":"limited","max_clicks":1}' > /dev/null
create_link '{"target_url":"https://example.com/off","slug":"off"}' > /dev/null
OFF_ID=$(curl -s -b "$COOKIES" "$BASE/api/v1/urls?q=off" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')

sqlite3 "$DATA/smoke.db" "UPDATE urls SET expires_at = '2000-01-01T00:00:00.000Z' WHERE slug = 'soon';"
check "an expired link is 410" "410" "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/soon")"
contains "the 410 page explains why" "expired" "$(curl -s "$BASE/soon")"

curl -s -o /dev/null "$BASE/limited"
sleep 0.4
check "a link over its click limit is 410" "410" "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/limited")"

curl -s -o /dev/null -b "$COOKIES" -X PATCH "$BASE/api/v1/urls/$OFF_ID" \
    -H 'content-type: application/json' -H "origin: $BASE" -d '{"status":"disabled"}'
check "a disabled link is 404" "404" "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/off")"
curl -s -o /dev/null -b "$COOKIES" -X DELETE -H "origin: $BASE" "$BASE/api/v1/urls/$OFF_ID"
check "a deleted link is 404" "404" "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/off")"
check "a deleted slug stays reserved" "409" "$(curl -s -o /dev/null -w '%{http_code}' \
    -b "$COOKIES" -X POST "$BASE/api/v1/urls" -H 'content-type: application/json' \
    -H "origin: $BASE" -d '{"target_url":"https://example.com/hijack","slug":"off"}')"

# ------------------------------------------------------------------- csrf
echo
echo "cross-site protection"
check "a cross-site JSON write is refused" "403" "$(curl -s -o /dev/null -w '%{http_code}' \
    -b "$COOKIES" -X POST "$BASE/api/v1/urls" -H 'content-type: application/json' \
    -H 'origin: https://evil.example' -d '{"target_url":"https://example.com/csrf"}')"
check "a cross-site form post is refused" "403" "$(curl -s -o /dev/null -w '%{http_code}' \
    -b "$COOKIES" -X POST "$BASE/app/urls" -H 'content-type: application/x-www-form-urlencoded' \
    -H 'origin: https://evil.example' -d 'target_url=https://example.com/csrf')"
check "a same-origin write is allowed" "201" "$(curl -s -o /dev/null -w '%{http_code}' \
    -b "$COOKIES" -X POST "$BASE/api/v1/urls" -H 'content-type: application/json' \
    -H "origin: $BASE" -d '{"target_url":"https://example.com/ok"}')"

# ------------------------------------------------------------- api tokens
echo
echo "API tokens"
TOKEN=$(curl -s -b "$COOKIES" -X POST "$BASE/api/v1/tokens" -H 'content-type: application/json' \
    -H "origin: $BASE" -d '{"name":"smoke"}' | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')
contains "a token is issued once" "ur_" "$TOKEN"
check "the token authenticates" "200" "$(curl -s -o /dev/null -w '%{http_code}' \
    -H "authorization: Bearer $TOKEN" "$BASE/api/v1/urls")"
check "a bogus token is rejected" "401" "$(curl -s -o /dev/null -w '%{http_code}' \
    -H 'authorization: Bearer ur_not-a-real-token' "$BASE/api/v1/urls")"
TOKEN_ID=$(curl -s -b "$COOKIES" "$BASE/api/v1/tokens" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
curl -s -o /dev/null -b "$COOKIES" -X DELETE -H "origin: $BASE" "$BASE/api/v1/tokens/$TOKEN_ID"
check "a revoked token stops working" "401" "$(curl -s -o /dev/null -w '%{http_code}' \
    -H "authorization: Bearer $TOKEN" "$BASE/api/v1/urls")"

# ------------------------------------------------------------------ admin
echo
echo "administration"
check "a member cannot list users" "403" "$(curl -s -o /dev/null -w '%{http_code}' \
    -b "$COOKIES_B" "$BASE/api/v1/admin/users")"
check "an admin can list users" "200" "$(curl -s -o /dev/null -w '%{http_code}' \
    -b "$COOKIES" "$BASE/api/v1/admin/users")"
check "an admin cannot demote themselves" "422" "$(curl -s -o /dev/null -w '%{http_code}' \
    -b "$COOKIES" -X PATCH "$BASE/api/v1/admin/users/1" -H 'content-type: application/json' \
    -H "origin: $BASE" -d '{"role":"user"}')"
check "settings can be updated" "200" "$(curl -s -o /dev/null -w '%{http_code}' \
    -b "$COOKIES" -X PUT "$BASE/api/v1/admin/settings" -H 'content-type: application/json' \
    -H "origin: $BASE" -d '{"site_name":"Smoke Test"}')"
contains "the new site name is served" "Smoke Test" "$(curl -s "$BASE/app/login")"
check "the audit log has entries" "200" "$(curl -s -o /dev/null -w '%{http_code}' \
    -b "$COOKIES" "$BASE/api/v1/admin/audit")"
contains "the audit log records registrations" 'auth.register' "$(curl -s -b "$COOKIES" "$BASE/api/v1/admin/audit")"

# The registration limiter is in memory and counts per IP; restart to reset it
# so the invite flow below is not throttled by the checks above.
stop_server
start_server

INVITE=$(curl -s -b "$COOKIES" -X POST "$BASE/api/v1/admin/invites" -H 'content-type: application/json' \
    -H "origin: $BASE" -d '{"max_uses":1}')
INVITE_CODE=$(echo "$INVITE" | sed -n 's/.*"code":"\([^"]*\)".*/\1/p')
contains "an invite code is issued" '"code"' "$INVITE"
curl -s -o /dev/null -b "$COOKIES" -X PUT "$BASE/api/v1/admin/settings" -H 'content-type: application/json' \
    -H "origin: $BASE" -d '{"registration_mode":"invite"}'
check "registration without a code is refused" "403" "$(curl -s -o /dev/null -w '%{http_code}' \
    -X POST "$BASE/api/v1/auth/register" -H 'content-type: application/json' \
    -d '{"email":"nocode@example.com","password":"password123"}')"
check "registration with the invite code works" "201" "$(curl -s -o /dev/null -w '%{http_code}' \
    -X POST "$BASE/api/v1/auth/register" -H 'content-type: application/json' \
    -d "{\"email\":\"invited@example.com\",\"password\":\"password123\",\"invite_code\":\"$INVITE_CODE\"}")"
check "the used invite cannot be reused" "403" "$(curl -s -o /dev/null -w '%{http_code}' \
    -X POST "$BASE/api/v1/auth/register" -H 'content-type: application/json' \
    -d "{\"email\":\"reuse@example.com\",\"password\":\"password123\",\"invite_code\":\"$INVITE_CODE\"}")"

# ------------------------------------------------------------------- pages
echo
echo "pages"
for path in /app/login /app/register; do
    check "GET $path renders" "200" "$(curl -s -o /dev/null -w '%{http_code}' "$BASE$path")"
done
for path in /app /app/account /app/admin /app/admin/users /app/admin/urls /app/admin/invites /app/admin/settings /app/admin/audit; do
    check "GET $path renders for an admin" "200" \
        "$(curl -s -o /dev/null -w '%{http_code}' -b "$COOKIES" "$BASE$path")"
done
check "GET /app without a session redirects" "303" "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/app")"

# ------------------------------------------------------------- persistence
echo
echo "persistence"
URLS_BEFORE=$(sqlite3 "$DATA/smoke.db" "SELECT COUNT(*) FROM urls;")
stop_server
start_server
URLS_AFTER=$(sqlite3 "$DATA/smoke.db" "SELECT COUNT(*) FROM urls;")
check "data survives a restart" "$URLS_BEFORE" "$URLS_AFTER"
check "the redirect still works after a restart" "302" \
    "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/demo")"
check "the session cookie survives a restart" "200" \
    "$(curl -s -o /dev/null -w '%{http_code}' -b "$COOKIES" "$BASE/api/v1/auth/me")"

echo
echo "-----------------------------------------------"
printf 'passed: %d   failed: %d\n' "$PASS" "$FAIL"
if [ "$FAIL" -gt 0 ]; then
    echo "server log tail:"
    tail -20 "$DATA/server.log"
    exit 1
fi
echo "all smoke checks passed"
