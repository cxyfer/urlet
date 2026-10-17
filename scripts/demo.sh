#!/usr/bin/env bash
# Visual verification: seed a demo instance and screenshot the UI.
# Runs the server, seeds data, screenshots, then stops the server.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PORT=18090
BASE="http://127.0.0.1:${PORT}"
DATA="$ROOT/.tmp/demo"
# Point SHOTS at docs/screenshots to regenerate the images the README uses:
#   SHOTS=docs/screenshots ./scripts/demo.sh
SHOTS="${SHOTS:-$DATA/shots}"

rm -rf "$DATA"
mkdir -p "$DATA" "$SHOTS"

UWURLS_DATABASE_URL="sqlite://$DATA/db.sqlite?mode=rwc" \
UWURLS_DATA_DIR="$DATA" \
UWURLS_BIND="127.0.0.1:${PORT}" \
UWURLS_BASE_URL="$BASE" \
UWURLS_LOG=warn \
./target/debug/uwurls serve > "$DATA/server.log" 2>&1 &
SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
    if curl -sf -o /dev/null "$BASE/health"; then break; fi
    sleep 0.2
done

curl -sf -o /dev/null -c "$DATA/cookies.txt" -X POST "$BASE/api/v1/auth/register" \
    -H 'content-type: application/json' \
    -d '{"email":"owner@example.com","password":"password123","display_name":"Site Owner"}'

create_link() {
    curl -s -o /dev/null -b "$DATA/cookies.txt" -X POST "$BASE/api/v1/urls" \
        -H 'content-type: application/json' -H "origin: $BASE" -d "$1"
}

create_link '{"target_url":"https://example.com/blog/announcing-uwurls-1-0","slug":"launch-2026","title":"Launch announcement","tags":["campaign","blog"],"expires_at":"2030-06-30","max_clicks":500}'
create_link '{"target_url":"https://docs.example.com/getting-started","slug":"docs","title":"Getting started guide","tags":["docs","product"],"expires_at":"2030-06-30"}'
create_link '{"target_url":"https://events.example.com/register?ref=email","slug":"webinar","title":"Webinar signup","tags":["campaign"],"expires_at":"2030-06-30"}'
create_link '{"target_url":"https://secret.example.com/board-deck","slug":"deck","title":"Board deck","password":"linkpass","tags":["internal"]}'
create_link '{"target_url":"https://example.com/old-campaign","slug":"archived","title":"Old campaign","expires_at":"2030-01-01"}'
# Force it into the past so the 410 page can be captured.
sqlite3 "$DATA/db.sqlite" "UPDATE urls SET expires_at = '2020-01-01T00:00:00.000Z' WHERE slug = 'archived';"

UA_DESKTOP="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
UA_MOBILE="Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1"
UA_MAC="Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.1 Safari/605.1.15"

for _ in $(seq 1 34); do curl -s -o /dev/null -A "$UA_DESKTOP" -e "https://news.example.com/story" "$BASE/launch-2026"; done
for _ in $(seq 1 7); do curl -s -o /dev/null -A "$UA_MOBILE" -e "https://twitter.com/home" "$BASE/launch-2026"; done
for _ in $(seq 1 19); do curl -s -o /dev/null -A "$UA_MOBILE" -e "https://www.google.com/search" "$BASE/docs"; done
for _ in $(seq 1 11); do curl -s -o /dev/null -A "$UA_MAC" "$BASE/webinar"; done
curl -s -o /dev/null -A "Googlebot/2.1 (+http://www.google.com/bot.html)" "$BASE/docs"
curl -s -o /dev/null -A "Mozilla/5.0 (compatible; Bingbot/2.0)" "$BASE/launch-2026"

# A second account plus an invite code, so the admin pages have content.
curl -s -o /dev/null -b "$DATA/cookies.txt" -X POST "$BASE/api/v1/urls" \
    -H 'content-type: application/json' -H "origin: $BASE" \
    -d '{"target_url":"https://partner.example.com/landing","slug":"partner","title":"Partner landing","tags":["partner"]}'
curl -s -o /dev/null -X POST "$BASE/api/v1/auth/register" -H 'content-type: application/json' \
    -d '{"email":"member@example.com","password":"password123","display_name":"Team Member"}'
curl -s -o /dev/null -b "$DATA/cookies.txt" -X POST "$BASE/api/v1/admin/invites" \
    -H 'content-type: application/json' -H "origin: $BASE" \
    -d '{"note":"design team","max_uses":3,"expires_at":"2030-01-01"}'

sleep 1

python3 "$ROOT/scripts/screenshot.py" "$BASE" "$SHOTS"

echo "screenshots:"
ls -la "$SHOTS"
echo "clicks: $(sqlite3 "$DATA/db.sqlite" 'SELECT COUNT(*) FROM clicks;')"
