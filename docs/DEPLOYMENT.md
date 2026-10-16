# uwurls deployment guide

**English** | [繁體中文](zh-TW/DEPLOYMENT.md)

## 1. Requirements

| Item | Minimum | Recommended |
|------|---------|-------------|
| CPU | 1 vCPU | 2 vCPU |
| Memory | 128 MB | 256 MB |
| Disk | 100 MB + data | 1 GB |
| Docker | 24.x | Latest |
| Docker Compose | 2.24 (`!override` is required by the TLS overlay) | Latest |

No external database, Redis or reverse proxy is needed. A single container can serve traffic on
its own (putting a TLS terminator in front of it is recommended).

## 2. Quick start (HTTP)

```bash
git clone <this repo> uwurls && cd uwurls
cp .env.example .env
```

Edit `.env`:

```env
UWURLS_BASE_URL=http://localhost:8080      # public URL; use https:// once TLS is in front (§3)
UWURLS_PORT=8080                            # host port to publish (read by docker compose only)
UWURLS_ADMIN_EMAIL=admin@example.com
UWURLS_ADMIN_PASSWORD=<strong password>
UWURLS_REGISTRATION=invite                  # optional: open / invite / closed
```

> Keep the scheme of `UWURLS_BASE_URL` in step with how visitors actually reach the instance: an
> `https://` value makes uwurls mark the session cookie `Secure`, so serving plain HTTP while
> `UWURLS_BASE_URL` says `https://` signs users out again immediately (§4, §10).

Start it:

```bash
docker compose up -d --build
docker compose logs -f uwurls
```

Verify:

```bash
curl -fsS http://localhost:8080/health
docker compose exec uwurls /usr/local/bin/uwurls healthcheck
```

## 3. Automatic HTTPS (Caddy)

```bash
export UWURLS_DOMAIN=s.example.com
docker compose -f docker-compose.yml -f docker-compose.tls.yml up -d --build
```

- Point the A/AAAA record for `s.example.com` at this host first; Caddy obtains and renews the
  certificate automatically.
- The overlay stops publishing the uwurls port (it is only reachable inside the compose network)
  and sets `UWURLS_BASE_URL`, `UWURLS_SECURE_COOKIES=true` and `UWURLS_TRUST_PROXY=true` for you.
- Certificates live in the `uwurls-caddy-data` volume.

If you already run Nginx or Traefik, reverse proxy to `127.0.0.1:8080` yourself and set:

```env
UWURLS_BASE_URL=https://s.example.com
UWURLS_SECURE_COOKIES=true
UWURLS_TRUST_PROXY=true      # only when you are sure the proxy overwrites X-Forwarded-*
```

## 4. Configuration reference

| Variable | Default | Description |
|----------|---------|-------------|
| `UWURLS_BASE_URL` | `http://localhost:8080` | Public URL; used to build short links, for the CSRF same-origin check and to decide whether cookies are `Secure` |
| `UWURLS_BIND` | `0.0.0.0:8080` | Listen address |
| `UWURLS_DATABASE_URL` | `sqlite:///data/uwurls.db?mode=rwc` | SQLite connection string |
| `UWURLS_DATA_DIR` | Directory holding the database | Where the auto-generated secret is written |
| `UWURLS_SECRET` | Auto-generated | HMAC secret (at least 16 characters, 32+ recommended; the generated one is 44 characters of base64); **changing it invalidates every session and unlock cookie** |
| `UWURLS_ADMIN_EMAIL` / `UWURLS_ADMIN_PASSWORD` | — | Create or promote the administrator at startup |
| `UWURLS_REGISTRATION` | Read from the settings table | Override the registration mode: `open` / `invite` / `closed` |
| `UWURLS_SECURE_COOKIES` | Derived from `BASE_URL` | Force the `Secure` cookie flag. The compose files always pass a value through (default `false`), so in Docker set it to `true` explicitly when you terminate TLS yourself; the TLS overlay does that for you |
| `UWURLS_TRUST_PROXY` | `false` | Trust `X-Forwarded-For` / `X-Forwarded-Proto` |
| `UWURLS_SESSION_TTL_HOURS` | `168` | Session lifetime |
| `UWURLS_UNLOCK_TTL_MINUTES` | `30` | Lifetime of the short link unlock cookie |
| `UWURLS_DEFAULT_SLUG_LENGTH` | Read from the settings table | Override the generated slug length (4–16) |
| `UWURLS_CAPTCHA_PROVIDER` | `none` | Bot verification provider: `none` / `turnstile` / `recaptcha` |
| `UWURLS_CAPTCHA_SITE_KEY` | Empty | Public site key rendered into the widget |
| `UWURLS_CAPTCHA_SECRET_KEY` | Empty | Secret key used for server-side verification (server only; the admin UI never echoes it back) |
| `UWURLS_CAPTCHA_ON_REGISTER` | `false` | Apply bot verification to the registration form |
| `UWURLS_CAPTCHA_ON_LOGIN` | `false` | Apply bot verification to the sign-in form |
| `UWURLS_CAPTCHA_ON_CREATE` | `false` | Apply bot verification to the create-short-link form |
| `UWURLS_LOG` | `info` | tracing filter, for example `uwurls=debug,tower=warn` |
| `UWURLS_LOG_FORMAT` | `compact` | `compact` or `json` |
| `UWURLS_HEALTHCHECK_URL` | `http://127.0.0.1:<port>/health` | Target of `uwurls healthcheck` |

Two further variables are read by the compose files, not by the uwurls binary:

| Variable | Default | Description |
|----------|---------|-------------|
| `UWURLS_PORT` | `8080` | Host port that `docker-compose.yml` publishes (the container itself always listens on 8080) |
| `UWURLS_DOMAIN` | — | Domain the `docker-compose.tls.yml` overlay requests a certificate for; required by that overlay |

Instance-level settings (site name, registration mode, quota, default redirect type, analytics
switch, click retention) are edited in **Admin → Settings**; they are stored in the database and
take effect immediately. A non-empty environment variable always overrides the value stored
there.

## 5. Bot verification (CAPTCHA)

uwurls can require a CAPTCHA on the three public forms that create state: the web registration
form, the web sign-in form, and the web create-short-link form. The JSON API endpoints
`POST /api/v1/auth/register` and `POST /api/v1/auth/login` are covered as well; they take the
widget token in the optional `captcha_token` field. `POST /api/v1/urls` is covered too, but only
for session-cookie calls (the link-creation switch); a call carrying
`Authorization: Bearer uw_...` stays exempt, so automation keeps working. Verification is disabled
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
| `UWURLS_CAPTCHA_PROVIDER` | `none` | `none`, `turnstile` or `recaptcha` |
| `UWURLS_CAPTCHA_SITE_KEY` | Empty | Public site key |
| `UWURLS_CAPTCHA_SECRET_KEY` | Empty | Secret key, server side only |
| `UWURLS_CAPTCHA_ON_REGISTER` | `false` | Verify the registration form |
| `UWURLS_CAPTCHA_ON_LOGIN` | `false` | Verify the sign-in form |
| `UWURLS_CAPTCHA_ON_CREATE` | `false` | Verify the create-short-link form |

Verification becomes active for a form only when **all four** conditions hold: the provider is
not `none`, the site key is non-empty, the secret key is non-empty, and the matching `ON_*`
switch (`UWURLS_CAPTCHA_ON_REGISTER`, `UWURLS_CAPTCHA_ON_LOGIN` or `UWURLS_CAPTCHA_ON_CREATE`) is
`true`. Setting a provider without keys, or keys without a provider, silently leaves verification
off.

Failure is **closed**: a missing, malformed, expired or already-used token — including a timeout
or any non-2xx response from the provider — rejects the submission. On the web forms the visitor
sees a validation error; `POST /api/v1/auth/register`, `POST /api/v1/auth/login` and a
session-cookie `POST /api/v1/urls` return `400` with `error.code = "captcha_failed"` when the
matching switch is enabled.

### Getting the keys

**Cloudflare Turnstile**

1. Sign in to the Cloudflare dashboard and open **Turnstile**.
2. Choose **Add widget**, name it, and add the hostname(s) visitors will use to reach uwurls
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
UWURLS_CAPTCHA_PROVIDER=turnstile
UWURLS_CAPTCHA_SITE_KEY=0x4AAAAAAA...
UWURLS_CAPTCHA_SECRET_KEY=0x4AAAAAAA...
UWURLS_CAPTCHA_ON_REGISTER=true
UWURLS_CAPTCHA_ON_LOGIN=true
UWURLS_CAPTCHA_ON_CREATE=true
```

The shipped `docker-compose.yml` passes all six `UWURLS_CAPTCHA_*` variables through to the
container, so setting them in `.env` is enough — no compose edit is needed. Then
`docker compose up -d` to recreate the container.

The same values can be entered in the admin UI under **Admin → Settings**, which takes effect
without a restart. The secret key is write-only there: the stored value is never displayed, and
leaving the field blank keeps it as it is. Remember that a non-empty environment variable wins
over the admin setting, so configure each value in exactly one place.

### `remoteip` behind a reverse proxy

Server-side verification sends the visitor's IP address as `remoteip` so the provider can score
the request. uwurls resolves that address exactly like the rate limiter does:

- `UWURLS_TRUST_PROXY=false` (the default) — the address is the direct peer, which is the proxy
  itself when one sits in front of uwurls. Every visitor then looks like the same client to the
  provider, and only the token's own signals are used for scoring.
- `UWURLS_TRUST_PROXY=true` — `X-Real-IP` is preferred, falling back to the rightmost entry of
  `X-Forwarded-For`. The shipped `deploy/Caddyfile` sets
  `header_up X-Real-IP {remote_host}`, so Caddy passes the real visitor address.
- When no address can be determined, `remoteip` is omitted; both provider APIs treat it as
  optional, so verification still works.

Only enable `UWURLS_TRUST_PROXY` when the proxy is guaranteed to overwrite `X-Real-IP` /
`X-Forwarded-For`; otherwise a client can forge its address and also evade the rate limits.

## 6. Data and backups

All state lives in `/data` (the `uwurls-data` volume):

```
/data/uwurls.db         # main database
/data/uwurls.db-wal     # write-ahead log
/data/uwurls.db-shm
/data/secret.key        # auto-generated secret when UWURLS_SECRET is unset (0600)
```

### Backup

SQLite runs in WAL mode, so copying the main file on its own can miss transactions that are still
in the WAL. Two safe approaches:

**Option A (host has `sqlite3`, no downtime)**

```bash
VOL=$(docker volume inspect -f '{{ .Mountpoint }}' uwurls-data)
sudo sqlite3 "$VOL/uwurls.db" ".backup '/tmp/uwurls-$(date +%F).db'"
mv /tmp/uwurls-$(date +%F).db ./backups/
```

`.backup` uses the SQLite online backup API and produces a consistent snapshot.

**Option B (no `sqlite3`)**

```bash
docker compose stop uwurls          # a clean shutdown checkpoints the WAL
docker run --rm -v uwurls-data:/data -v "$PWD":/backup alpine \
  cp /data/uwurls.db "/backup/uwurls-$(date +%F).db"
docker compose start uwurls
```

Do not forget `secret.key`: if you rely on the auto-generated secret, back it up (or switch to the
`UWURLS_SECRET` environment variable), otherwise every session and unlock cookie is invalidated
after a restore.

### Restore

```bash
docker compose down
docker run --rm -v uwurls-data:/data -v "$PWD":/backup alpine \
  sh -c 'rm -f /data/uwurls.db*; cp /backup/uwurls-YYYY-MM-DD.db /data/uwurls.db; chown 10001:10001 /data/uwurls.db'
docker compose up -d
```

### Upgrading

```bash
git pull
docker compose up -d --build      # migrations are applied at startup
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

- Always set `UWURLS_BASE_URL` to the URL users actually see, otherwise short links point at the
  wrong domain.
- Only enable `UWURLS_TRUST_PROXY` when the proxy is **guaranteed to overwrite**
  `X-Forwarded-For` / `X-Real-IP`; otherwise clients can forge their IP and bypass the rate
  limits. The application prefers `X-Real-IP` (set by the proxy itself) and otherwise takes the
  rightmost entry of `X-Forwarded-For`, so appending proxies are safe too. The shipped
  `deploy/Caddyfile` already sets `header_up X-Real-IP {remote_host}`.
- `Host`, `X-Forwarded-For` and `X-Forwarded-Proto` must be forwarded; Caddy handles this
  automatically.

## 9. Monitoring

- Health check: `GET /health` (200 = healthy, 503 = database problem)
- The container ships a `HEALTHCHECK` that runs `uwurls healthcheck` (the image needs no curl)
- Logs go to stdout; `UWURLS_LOG_FORMAT=json` pairs well with a collector such as Loki or
  CloudWatch
- Every request carries an `x-request-id` response header, and the same id is written to the log
- **Admin → Overview** in the admin UI shows the user count, short link count, total clicks and
  database size

## 10. Troubleshooting

| Symptom | Cause and fix |
|---------|---------------|
| Startup fails with a migration checksum mismatch | A file in `migrations/` that was already applied has been edited; restore it or add a new migration instead |
| Signed out immediately after signing in | `UWURLS_BASE_URL` does not match the real URL, or `UWURLS_SECURE_COOKIES=true` while browsing over HTTP |
| Short links point at `http://localhost:8080` | `UWURLS_BASE_URL` is not set |
| `/health` returns 503 after the container starts | Volume permission problem: `/data` must be writable (the container runs as uid 10001) |
| Rate limits hit legitimate users (many users behind NAT) | Loosen the limits or switch those clients to API tokens; limits are counted per IP |
| Login returns 429 even with the correct password | The account is temporarily locked (30 failures within 15 minutes). Wait for the window to slide past, or run `sqlite3 /data/uwurls.db "DELETE FROM login_attempts"` to clear it immediately |
| The CAPTCHA widget never appears | All four conditions must hold: a provider other than `none`, both keys, and the matching `ON_*` switch set to `true` |
| Every submission fails with `captcha_failed` | The secret key does not belong to the configured provider, the widget's hostname list does not include the domain users visit, or an API client forgot to send the widget token as `captcha_token` |
| Want token access after forgetting the admin password | Resetting a password revokes every API token for that account (by design); create a new token afterwards |
| Forgot the admin password | The startup bootstrap runs only when **both** `UWURLS_ADMIN_EMAIL` and `UWURLS_ADMIN_PASSWORD` are non-empty, and for an account that already exists it merely promotes it to administrator — it does **not** change the password (the password is used only when the account does not exist yet). To recover, point `UWURLS_ADMIN_EMAIL` at another account you can still sign in to (any non-empty `UWURLS_ADMIN_PASSWORD` makes the bootstrap run), restart, and use that account; or overwrite the password hash directly with `sqlite3` |
| A short link expired and you want it back | Open `/app/urls/{id}` and clear or extend the expiry; the data was never deleted |
| Want to reuse a slug after deleting a short link | Deliberately not allowed (so old links cannot be hijacked); pick a new slug |

## 11. Security checklist

- [ ] `UWURLS_BASE_URL` uses `https://`
- [ ] `UWURLS_ADMIN_PASSWORD` is a strong password, changed at `/app/account` after the first sign-in
- [ ] `UWURLS_SECRET` is set (at least 32 random characters), or the auto-generated `secret.key` (0600) is backed up
- [ ] Only 80/443 are exposed publicly (Caddy or your existing proxy terminates TLS)
- [ ] `UWURLS_TRUST_PROXY` matches the actual proxy behavior
- [ ] The registration mode is `invite` or `closed` (unless you really want open registration)
- [ ] `click_retention_days` and a backup schedule are planned
- [ ] `docker compose pull && docker compose up -d --build` runs regularly
- [ ] Bot verification is enabled on registration and sign-in if the instance is reachable by the public internet
