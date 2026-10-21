# urlet deployment guide

**English** | [繁體中文](zh-TW/DEPLOYMENT.md)

## 1. Requirements

| Item | Minimum | Recommended |
|------|---------|-------------|
| CPU | 1 vCPU | 2 vCPU |
| Memory | 128 MB | 256 MB |
| Disk | 100 MB + data | 1 GB |
| Docker | 24.x | Latest |
| Docker Compose | 2.24 (`!override` / `!reset` are used by the overlays) | Latest |

No external database, Redis or reverse proxy is needed. A single container can serve traffic on
its own (putting a TLS terminator in front of it is recommended).

## 2. Quick start (HTTP)

```bash
git clone <this repo> urlet && cd urlet
cp .env.example .env
```

Edit `.env`:

```env
URLET_BASE_URL=http://localhost:8080      # public URL; use https:// once TLS is in front (§3)
URLET_PORT=8080                            # host port to publish (read by docker compose only)
URLET_ADMIN_EMAIL=admin@example.com
URLET_ADMIN_PASSWORD=<strong password>
URLET_REGISTRATION=invite                  # optional: open / invite / closed
```

> Keep the scheme of `URLET_BASE_URL` in step with how visitors actually reach the instance: an
> `https://` value makes urlet mark the session cookie `Secure`, so serving plain HTTP while
> `URLET_BASE_URL` says `https://` signs users out again immediately (§4, §10).

Start it:

```bash
docker compose up -d --build
docker compose logs -f urlet
```

Verify:

```bash
curl -fsS http://localhost:8080/health
docker compose exec urlet /usr/local/bin/urlet healthcheck
```

### 2.1 Prebuilt image

Every `v*` tag publishes a multi-arch image (`linux/amd64`, `linux/arm64`) to GHCR. To run a
published release instead of building locally:

```bash
export URLET_IMAGE=ghcr.io/cxyfer/urlet:v1.2.0   # pin the release you want
docker compose -f docker-compose.yml -f docker-compose.image.yml up -d
```

- `URLET_IMAGE` defaults to `ghcr.io/cxyfer/urlet:latest`; pin an exact version in production
  rather than tracking `latest`. When you do use `latest`, run `docker compose -f
  docker-compose.yml -f docker-compose.image.yml pull` before `up -d` to fetch a newer release.
- The overlay removes the `build` section, so compose pulls the image and never falls back to a
  local Rust build.
- The package defaults to **private**. Either authenticate (`docker login ghcr.io` with a token
  that has `read:packages`) or make the package public under *Package settings → Change
  visibility*.
- The image runs as uid/gid `10001`, exactly like a locally built one, so the `/data` volume
  permissions are unchanged.

## 3. Automatic HTTPS (Caddy)

```bash
export URLET_DOMAIN=s.example.com
docker compose -f docker-compose.yml -f docker-compose.tls.yml up -d --build
```

- Point the A/AAAA record for `s.example.com` at this host first; Caddy obtains and renews the
  certificate automatically.
- The overlay stops publishing the urlet port (it is only reachable inside the compose network)
  and sets `URLET_BASE_URL`, `URLET_SECURE_COOKIES=true` and `URLET_TRUST_PROXY=true` for you.
- Certificates live in the `urlet-caddy-data` volume.

If you already run Nginx or Traefik, reverse proxy to `127.0.0.1:8080` yourself and set:

```env
URLET_BASE_URL=https://s.example.com
URLET_SECURE_COOKIES=true
URLET_TRUST_PROXY=true      # only when you are sure the proxy overwrites X-Forwarded-*
```

## 4. Configuration reference

| Variable | Default | Description |
|----------|---------|-------------|
| `URLET_BASE_URL` | `http://localhost:8080` | Public URL; used to build short links, for the CSRF same-origin check and to decide whether cookies are `Secure`. The host must be ASCII (punycode for IDN); IPv6 literals and underscore service names are accepted for the canonical host only |
| `URLET_DOMAINS` | Empty | Extra redirect-only hostnames, comma separated. Inserted as active on first boot; never overrides an existing status. Every domain must use the same http/https scheme as `URLET_BASE_URL` and needs DNS + TLS |
| `URLET_BIND` | `0.0.0.0:8080` | Listen address |
| `URLET_DATABASE_URL` | `sqlite:///data/urlet.db?mode=rwc` | SQLite connection string |
| `URLET_DATA_DIR` | Directory holding the database | Where the auto-generated secret is written |
| `URLET_THEMES_DIR` | `<URLET_DATA_DIR>/themes` | Where uploaded theme stylesheets are stored. docker compose mounts the `urlet-themes` volume at `/data/themes` |
| `URLET_SECRET` | Auto-generated | HMAC secret (at least 16 characters, 32+ recommended; the generated one is 44 characters of base64); **changing it invalidates every session and unlock cookie and makes existing invite codes unreadable, though they still redeem** |
| `URLET_ADMIN_EMAIL` / `URLET_ADMIN_PASSWORD` | — | Create or promote the administrator at startup |
| `URLET_REGISTRATION` | Read from the settings table | Override the registration mode: `open` / `invite` / `closed` |
| `URLET_SECURE_COOKIES` | Derived from `BASE_URL` | Force the `Secure` cookie flag. The compose files always pass a value through (default `false`), so in Docker set it to `true` explicitly when you terminate TLS yourself; the TLS overlay does that for you |
| `URLET_TRUST_PROXY` | `false` | Trust `X-Forwarded-For` / `X-Forwarded-Proto` |
| `URLET_SESSION_TTL_HOURS` | `168` | Session lifetime |
| `URLET_UNLOCK_TTL_MINUTES` | `30` | Lifetime of the short link unlock cookie |
| `URLET_DEFAULT_SLUG_LENGTH` | Read from the settings table | Override the generated slug length (4–16) |
| `URLET_CAPTCHA_PROVIDER` | `none` | Bot verification provider: `none` / `turnstile` / `recaptcha` |
| `URLET_CAPTCHA_SITE_KEY` | Empty | Public site key rendered into the widget |
| `URLET_CAPTCHA_SECRET_KEY` | Empty | Secret key used for server-side verification (server only; the admin UI never echoes it back) |
| `URLET_CAPTCHA_ON_REGISTER` | `false` | Apply bot verification to the registration form |
| `URLET_CAPTCHA_ON_LOGIN` | `false` | Apply bot verification to the sign-in form |
| `URLET_CAPTCHA_ON_CREATE` | `false` | Apply bot verification to the create-short-link form |
| `URLET_CAPTCHA_ALLOWED_HOSTNAMES` | Empty | Extra hostnames accepted in the provider's `hostname` response, comma-separated (up to 10); the host of `URLET_BASE_URL` is always accepted |
| `URLET_LOG` | `info` | tracing filter, for example `urlet=debug,tower=warn` |
| `URLET_LOG_FORMAT` | `compact` | `compact` or `json` |
| `URLET_HEALTHCHECK_URL` | `http://127.0.0.1:<port>/health` | Target of `urlet healthcheck` |

### Multi-domain deployments

> **Upgrading from 1.2 or older:** back up the SQLite database first. The multi-domain migration
> (`0003_multi_domain`) rebuilds the `urls` table so the slug namespace becomes per domain; the
> migration is tested to preserve clicks, tags and daily stats, but a backup is the only rollback.

The canonical host keeps the application (`/`, `/app`, `/api/v1`). Extra domains are
redirect-only: `/{slug}` and `/{slug}/unlock` work, `/` shows a small landing page, `/app` is
redirected to the canonical host and `/api` returns 404. Each domain has its own slug namespace.

- Seed extra domains with `URLET_DOMAINS=go.example.com,links.example.com`, or add them from
  `/app/admin/domains`.
- Domains added in the admin UI start as `pending`. Point DNS at this instance, add the hostname
  to the Caddy site label (or your reverse proxy), verify `https://<domain>/health`, then press
  **Activate**.
- The canonical domain cannot be disabled or deleted. The current default domain must be replaced
  before it can be disabled or deleted. If `URLET_BASE_URL` changes, the previous canonical domain
  is demoted to ordinary admin management on the next boot (its links keep working) and the default
  moves to the new canonical domain when the old one held it.
- All domains must use the same scheme as `URLET_BASE_URL`; do not mix http and https domains.
- The reverse proxy must preserve the original `Host` header. `X-Forwarded-Host` is not trusted in
  this version.
- `URLET_DOMAINS` is insert-only: disabling a seeded domain in the UI survives a restart. When the
  variable is set, removing a hostname from it detaches that domain to ordinary admin management on
  the next boot (status, links and default are preserved); it is not deleted automatically.
  Docker Compose always passes the variable, so an empty `URLET_DOMAINS` is treated as an explicit
  empty list.

Three further variables are read by the compose files, not by the urlet binary:

| Variable | Default | Description |
|----------|---------|-------------|
| `URLET_PORT` | `8080` | Host port that `docker-compose.yml` publishes (the container itself always listens on 8080) |
| `URLET_DOMAIN` | — | Domain the `docker-compose.tls.yml` overlay requests a certificate for; required by that overlay |
| `URLET_IMAGE` | `ghcr.io/cxyfer/urlet:latest` | Image used by the `docker-compose.image.yml` overlay (ignored without it) |

Instance-level settings (site name, registration mode, quota, default redirect type, analytics
switch, click retention) are edited in **Admin → Settings**; they are stored in the database and
take effect immediately. A non-empty environment variable always overrides the value stored
there.

## 5. Bot verification (CAPTCHA)

urlet can require a CAPTCHA on the three public forms that create state: the web registration
form, the web sign-in form, and the web create-short-link form. The JSON API endpoints
`POST /api/v1/auth/register` and `POST /api/v1/auth/login` are covered as well; they take the
widget token in the optional `captcha_token` field. `POST /api/v1/urls` is covered too, but only
for session-cookie calls (the link-creation switch); a call carrying
`Authorization: Bearer ur_...` stays exempt, so automation keeps working. Verification is disabled
by default.

### Supported providers

| Provider | Value | Notes |
|----------|-------|-------|
| Disabled | `none` | Default. No widget is rendered and nothing is verified |
| Cloudflare Turnstile | `turnstile` | Privacy-friendly, free tier, usually invisible to the visitor |
| Google reCAPTCHA v2 | `recaptcha` | The "I'm not a robot" checkbox |

### Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `URLET_CAPTCHA_PROVIDER` | `none` | `none`, `turnstile` or `recaptcha` |
| `URLET_CAPTCHA_SITE_KEY` | Empty | Public site key |
| `URLET_CAPTCHA_SECRET_KEY` | Empty | Secret key, server side only |
| `URLET_CAPTCHA_ON_REGISTER` | `false` | Verify the registration form |
| `URLET_CAPTCHA_ON_LOGIN` | `false` | Verify the sign-in form |
| `URLET_CAPTCHA_ON_CREATE` | `false` | Verify the create-short-link form |
| `URLET_CAPTCHA_ALLOWED_HOSTNAMES` | Empty | Extra hostnames whose tokens are accepted, comma-separated (up to 10) |

Verification becomes active for a form only when **all four** conditions hold: the provider is
not `none`, the site key is non-empty, the secret key is non-empty, and the matching `ON_*`
switch (`URLET_CAPTCHA_ON_REGISTER`, `URLET_CAPTCHA_ON_LOGIN` or `URLET_CAPTCHA_ON_CREATE`) is
`true`. Setting a provider without keys, or keys without a provider, silently leaves verification
off.

Failure is **closed**: a missing, malformed, expired or already-used token — including a timeout
or any non-2xx response from the provider — rejects the submission. On the web forms the visitor
sees a validation error; `POST /api/v1/auth/register`, `POST /api/v1/auth/login` and a
session-cookie `POST /api/v1/urls` return `400` with `error.code = "captcha_failed"` when the
matching switch is enabled.

### Token hostname (deployments with several hostnames)

Both providers report the hostname the token was solved on in the `siteverify` response, and
urlet verifies it: the token must have been solved **on this instance**. The reported hostname
must therefore equal the host of `URLET_BASE_URL`, or one of the extra hostnames listed in
`URLET_CAPTCHA_ALLOWED_HOSTNAMES`. A response whose `hostname` is missing or empty is accepted —
not every provider or widget configuration returns the field.

`URLET_CAPTCHA_ALLOWED_HOSTNAMES` takes a comma-separated list of bare hostnames: each item is
trimmed and lowercased, empty items are dropped, and at most **10** items of 1–253 characters are
kept. Entries must not contain whitespace, `/`, `:` or `@`. Leaving it empty means "only the host
of `URLET_BASE_URL`", which is the right setting for the common single-domain deployment.

> **If your instance serves the forms on more than one hostname** (for example a short-link domain
> and a separate application domain, or an extra alias), list every one of those hostnames here.
> Tokens solved on a hostname that is neither the `URLET_BASE_URL` host nor on the list are
> rejected, which shows up as "the visitor solves the CAPTCHA but still cannot register or sign
> in". On a mismatch urlet logs a `WARN` line containing the reported hostname and the list of
> accepted candidates, which is the fastest way to find the missing entry.

### Getting the keys

**Cloudflare Turnstile**

1. Sign in to the Cloudflare dashboard and open **Turnstile**.
2. Choose **Add widget**, name it, and add the hostname(s) visitors will use to reach urlet
   (for example `s.example.com`; add `localhost` as well if you want to test locally).
3. Pick a widget mode (Managed is a good default) and create the widget.
4. Copy the **Site Key** and the **Secret Key**.

**Google reCAPTCHA v2**

1. Open the reCAPTCHA admin console and register a new site.
2. Select **reCAPTCHA v2** → **"I'm not a robot" Checkbox**.
3. Add the domain(s) of your instance.
4. Copy the **Site key** and the **Secret key**.

### Configuring it

In `.env` (docker compose):

```env
URLET_CAPTCHA_PROVIDER=turnstile
URLET_CAPTCHA_SITE_KEY=0x4AAAAAAA...
URLET_CAPTCHA_SECRET_KEY=0x4AAAAAAA...
URLET_CAPTCHA_ON_REGISTER=true
URLET_CAPTCHA_ON_LOGIN=true
URLET_CAPTCHA_ON_CREATE=true
# Optional: extra hostnames whose tokens are accepted (comma-separated)
# URLET_CAPTCHA_ALLOWED_HOSTNAMES=www.example.com,alt.example.com
```

The shipped `docker-compose.yml` passes all seven `URLET_CAPTCHA_*` variables through to the
container, so setting them in `.env` is enough — no compose edit is needed. Then
`docker compose up -d` to recreate the container.

The same values can be entered in the admin UI under **Admin → Settings**, which takes effect
without a restart. The secret key is write-only there: the stored value is never displayed, and
leaving the field blank keeps it as it is. Remember that a non-empty environment variable wins
over the admin setting, so configure each value in exactly one place.

### `remoteip` behind a reverse proxy

Server-side verification sends the visitor's IP address as `remoteip` so the provider can score
the request. urlet resolves that address exactly like the rate limiter does:

- `URLET_TRUST_PROXY=false` (the default) — the address is the direct peer, which is the proxy
  itself when one sits in front of urlet. Every visitor then looks like the same client to the
  provider, and only the token's own signals are used for scoring.
- `URLET_TRUST_PROXY=true` — `X-Real-IP` is preferred, falling back to the rightmost entry of
  `X-Forwarded-For`. The shipped `deploy/Caddyfile` sets
  `header_up X-Real-IP {remote_host}`, so Caddy passes the real visitor address.
- When no address can be determined, `remoteip` is omitted; both provider APIs treat it as
  optional, so verification still works.

Only enable `URLET_TRUST_PROXY` when the proxy is guaranteed to overwrite `X-Real-IP` /
`X-Forwarded-For`; otherwise a client can forge its address and also evade the rate limits.

## 6. Data and backups

All state lives in `/data`: the `urlet-data` volume holds the database and secret, and the
`urlet-themes` volume (mounted at `/data/themes`) holds uploaded theme stylesheets:

```
/data/urlet.db          # main database
/data/urlet.db-wal      # write-ahead log
/data/urlet.db-shm
/data/secret.key         # auto-generated secret when URLET_SECRET is unset (0600)
/data/themes/*.css       # uploaded themes (urlet-themes volume)
```

### Backup

SQLite runs in WAL mode, so copying the main file on its own can miss transactions that are still
in the WAL. Two safe approaches:

**Option A (host has `sqlite3`, no downtime)**

```bash
VOL=$(docker volume inspect -f '{{ .Mountpoint }}' urlet-data)
sudo sqlite3 "$VOL/urlet.db" ".backup '/tmp/urlet-$(date +%F).db'"
mv /tmp/urlet-$(date +%F).db ./backups/
```

`.backup` uses the SQLite online backup API and produces a consistent snapshot.

**Option B (no `sqlite3`)**

```bash
docker compose stop urlet          # a clean shutdown checkpoints the WAL
docker run --rm -v urlet-data:/data -v "$PWD":/backup alpine \
  cp /data/urlet.db "/backup/urlet-$(date +%F).db"
docker compose start urlet
```

Do not forget `secret.key`: if you rely on the auto-generated secret, back it up (or switch to the
`URLET_SECRET` environment variable), otherwise every session and unlock cookie is invalidated
after a restore and existing invite codes can no longer be displayed (they still redeem). A
data-directory backup contains both the database and that key, and invite codes are low-entropy, so
treat the whole backup as sensitive.

Uploaded themes are split across two volumes: the database holds the catalogue row and metadata,
while the CSS files live in `urlet-themes`. Back up both, otherwise a restore lists themes whose
stylesheets return 404. For example:

```bash
docker run --rm -v urlet-themes:/themes -v "$PWD":/backup alpine \
  tar -czf "/backup/urlet-themes-$(date +%F).tar.gz" -C /themes .
```

Restore them with the same helper and fix ownership (`chown -R 10001:10001`).

### Restore

```bash
docker compose down
docker run --rm -v urlet-data:/data -v "$PWD":/backup alpine \
  sh -c 'rm -f /data/urlet.db*; cp /backup/urlet-YYYY-MM-DD.db /data/urlet.db; chown 10001:10001 /data/urlet.db'
docker run --rm -v urlet-themes:/themes -v "$PWD":/backup alpine \
  sh -c 'rm -rf /themes/*; tar -xzf /backup/urlet-themes-YYYY-MM-DD.tar.gz -C /themes; chown -R 10001:10001 /themes'
docker compose up -d
```

### Upgrading

```bash
git pull
docker compose up -d --build      # migrations are applied at startup
```

With a prebuilt image, change `URLET_IMAGE` to the new version and recreate the container:

```bash
export URLET_IMAGE=ghcr.io/cxyfer/urlet:v1.2.0
docker compose -f docker-compose.yml -f docker-compose.image.yml up -d
```

Migrations are forward-only additive files (`migrations/`), and applied files are checksum
protected: editing one makes startup fail instead of silently diverging. Back up before
upgrading.

## 7. Storage and disk usage

| Data | Growth |
|------|--------|
| `urls` | About 300 bytes per row |
| `clicks` | About 150 bytes per row (can be pruned with `click_retention_days`) |
| `url_daily_stats` | One row per short link per day |
| `audit_logs` | About 200 bytes per row, retained for 180 days |
| `sessions` | Removed within a day of expiring or being revoked |
| `login_attempts` | Retained for 30 days |

100,000 clicks take roughly 15 MB. If you never prune, set `click_retention_days`.

## 8. Reverse proxy notes

- Always set `URLET_BASE_URL` to the URL users actually see, otherwise short links point at the
  wrong domain.
- Only enable `URLET_TRUST_PROXY` when the proxy is **guaranteed to overwrite**
  `X-Forwarded-For` / `X-Real-IP`; otherwise clients can forge their IP and bypass the rate
  limits. The application prefers `X-Real-IP` (set by the proxy itself) and otherwise takes the
  rightmost entry of `X-Forwarded-For`, so appending proxies are safe too. The shipped
  `deploy/Caddyfile` already sets `header_up X-Real-IP {remote_host}`.
- `Host`, `X-Forwarded-For` and `X-Forwarded-Proto` must be forwarded; Caddy handles this
  automatically.
- Invite links carry the code in `?invite=`; reverse-proxy access logs that record query strings
  expose it. Restrict or redact those logs, or share the code instead.

## 9. Monitoring

- Health check: `GET /health` (200 = healthy, 503 = database problem)
- The container ships a `HEALTHCHECK` that runs `urlet healthcheck` (the image needs no curl)
- Logs go to stdout; `URLET_LOG_FORMAT=json` pairs well with a collector such as Loki or
  CloudWatch
- Every request carries an `x-request-id` response header, and the same id is written to the log
- **Admin → Overview** in the admin UI shows the user count, short link count, total clicks and
  database size

## 10. Troubleshooting

| Symptom | Cause and fix |
|---------|---------------|
| Startup fails with a migration checksum mismatch | A file in `migrations/` that was already applied has been edited; restore it or add a new migration instead |
| Signed out immediately after signing in | `URLET_BASE_URL` does not match the real URL, or `URLET_SECURE_COOKIES=true` while browsing over HTTP |
| Short links point at `http://localhost:8080` | `URLET_BASE_URL` is not set |
| `/health` returns 503 after the container starts | Volume permission problem: `/data` must be writable (the container runs as uid 10001) |
| Rate limits hit legitimate users (many users behind NAT) | Loosen the limits or switch those clients to API tokens; limits are counted per IP |
| Login returns 429 even with the correct password | The account is temporarily locked (30 failures within 15 minutes). Wait for the window to slide past, or run `sqlite3 /data/urlet.db "DELETE FROM login_attempts"` to clear it immediately |
| The CAPTCHA widget never appears | All four conditions must hold: a provider other than `none`, both keys, and the matching `ON_*` switch set to `true` |
| Every submission fails with `captcha_failed` | The secret key does not belong to the configured provider, the widget's hostname list does not include the domain users visit, or an API client forgot to send the widget token as `captcha_token` |
| The visitor solves the CAPTCHA but registration or sign-in still fails | The token was solved on a hostname that urlet does not accept: add it to `URLET_CAPTCHA_ALLOWED_HOSTNAMES` (the `WARN` log line names the reported hostname and the accepted candidates) |
| Want token access after forgetting the admin password | Resetting a password revokes every API token for that account (by design); create a new token afterwards |
| Forgot the admin password | The startup bootstrap runs only when **both** `URLET_ADMIN_EMAIL` and `URLET_ADMIN_PASSWORD` are non-empty, and for an account that already exists it merely promotes it to administrator — it does **not** change the password (the password is used only when the account does not exist yet). To recover, point `URLET_ADMIN_EMAIL` at another account you can still sign in to (any non-empty `URLET_ADMIN_PASSWORD` makes the bootstrap run), restart, and use that account; or overwrite the password hash directly with `sqlite3` |
| A short link expired and you want it back | Open `/app/urls/{id}` and clear or extend the expiry; the data was never deleted |
| Want to reuse a slug after deleting a short link | Deliberately not allowed (so old links cannot be hijacked); pick a new slug |

## 11. Security checklist

- [ ] `URLET_BASE_URL` uses `https://`
- [ ] `URLET_ADMIN_PASSWORD` is a strong password, changed at `/app/account` after the first sign-in
- [ ] `URLET_SECRET` is set (at least 32 random characters), or the auto-generated `secret.key` (0600) is backed up
- [ ] Only 80/443 are exposed publicly (Caddy or your existing proxy terminates TLS)
- [ ] `URLET_TRUST_PROXY` matches the actual proxy behavior
- [ ] The registration mode is `invite` or `closed` (unless you really want open registration)
- [ ] `click_retention_days` and a backup schedule are planned
- [ ] A regular update routine exists (rebuild locally or pull a new `URLET_IMAGE` tag), with a
      backup before each upgrade
- [ ] Bot verification is enabled on registration and sign-in if the instance is reachable by the public internet
