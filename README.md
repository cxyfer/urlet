**English** | [繁體中文](README.zh-TW.md)

# urlet

**A lightweight, self-hosted, multi-user URL shortener.** One Rust binary and one SQLite
file: a single `docker compose up -d` and you are live. No external database, no cache
service, no frontend build step.

![Landing page](docs/screenshots/landing.png)

## Features

| Feature | Description |
|---------|-------------|
| 👥 Multi-user | Every user manages their own links; cross-account access always returns 404 |
| 🏠 Landing page | `/` is an introduction and navigation page with sign-in and sign-up entries; it no longer redirects straight to the login form |
| 🔗 Link management | Custom or generated slugs, titles, notes, tags, enable/disable, soft delete |
| 🔒 Password protection | Per-link passwords (Argon2id); after unlocking, a signed cookie remembers the visitor for 30 minutes |
| ⏳ Expiry | Absolute expiration time and click limits; expired links return 410 Gone |
| 📊 Click analytics | Daily trends, referrer domains, device/browser/OS breakdown, recent clicks |
| 🧩 Tags | Filter and group links by tag, with usage counts |
| 🔑 API tokens | `Authorization: Bearer ur_...`, with optional expiry and revocation |
| 🌐 Multi-domain | Add extra domains in the admin panel; each domain has its own slug namespace and serves redirects only |
| 🛠 Admin panel | User management, all site links, domains, invite codes, site settings, audit log |
| 🌐 Localized UI | English, Traditional Chinese, and Simplified Chinese; the first visit follows the browser's `Accept-Language` header and falls back to English |
| 🤖 Bot protection | Cloudflare Turnstile or Google reCAPTCHA v2, enabled through environment variables or the admin panel; applies to the registration, sign-in and link creation forms |
| 🐳 Deployment | Single container plus a named volume; published multi-arch images on GHCR; optional Caddy overlay for automatic HTTPS |
| 🪶 Lightweight | 5.7 MB (glibc) / 5.8 MB (musl) binary, 14.8 MB container image, 8.7 MB RSS after 500 requests, and sub-millisecond redirect latency at P95 (see the note below) |

> The binary and image figures include the embedded locale catalogs and the TLS stack used by the
> optional bot protection; the redirect latency was measured on v1.0 and is expected to be
> unchanged, since captcha verification only runs on the registration, sign-in and link-creation
> forms — never on the redirect path.

## Quick start

### Docker Compose (recommended)

```bash
git clone <this repo> urlet && cd urlet
cp .env.example .env      # set at least URLET_BASE_URL and the admin credentials
docker compose up -d --build
```

Open `http://localhost:8080`. The landing page links to sign-in and sign-up; sign in with
the `URLET_ADMIN_EMAIL` / `URLET_ADMIN_PASSWORD` from `.env` (the server creates that
administrator on startup).

To let Caddy obtain HTTPS certificates automatically:

```bash
export URLET_DOMAIN=s.example.com
docker compose -f docker-compose.yml -f docker-compose.tls.yml up -d --build
```

> Requires Docker Compose ≥ 2.24 (the overlay uses the `!override` tag).

### Prebuilt image

Every `v*` tag publishes a multi-arch image (`linux/amd64` and `linux/arm64`) to the GitHub
Container Registry, so you can run a release without compiling Rust yourself:

```bash
export URLET_IMAGE=ghcr.io/cxyfer/urlet:latest   # pin a release tag in production
docker compose -f docker-compose.yml -f docker-compose.image.yml up -d
```

The overlay removes the local `build` section, so compose pulls the image instead of rebuilding.
The package may be private; run `docker login ghcr.io` first (a token with `read:packages`), or
make the package public under *Package settings → Change visibility*. See
[`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md#21-prebuilt-image) for pinning and upgrades.

### Running the binary directly

```bash
cargo build --release
URLET_BASE_URL=http://localhost:8080 \
URLET_DATA_DIR=./data \
URLET_ADMIN_EMAIL=admin@example.com \
URLET_ADMIN_PASSWORD=change-me-please \
./target/release/urlet serve
```

### Command line

```
urlet serve                 # start the server (default)
urlet migrate               # apply database migrations, then exit
urlet healthcheck [URL]     # health check (used by the container HEALTHCHECK, no curl needed)
urlet version | help
```

## First run

1. If `URLET_ADMIN_EMAIL` / `URLET_ADMIN_PASSWORD` are set, the administrator is created
   at startup.
2. If the database holds **no accounts at all**, the first user to register becomes an
   administrator automatically.
3. After that, generate invite codes under **Admin → Invites** and switch the registration
   mode to `invite` (invite code only) or `closed` (registration disabled) under
   **Admin → Settings**.

## Screenshots

The landing page, and the same page after a visitor's browser language is honoured:

| Landing page | Traditional Chinese (automatic) |
|---|---|
| ![Landing page](docs/screenshots/landing.png) | ![Traditional Chinese landing page](docs/screenshots/landing-zh-hant.png) |

Signed in:

| Dashboard | Link detail and stats |
|---|---|
| ![Dashboard](docs/screenshots/dashboard.png) | ![URL detail](docs/screenshots/url-detail.png) |

| Admin panel | Bot protection settings |
|---|---|
| ![Admin](docs/screenshots/admin-overview.png) | ![Bot protection](docs/screenshots/admin-captcha.png) |

| Password page | Expired link (one-click extend) |
|---|---|
| ![Password page](docs/screenshots/password-page.png) | ![Expired link](docs/screenshots/expired-link.png) |

Dark mode: ![Dark mode](docs/screenshots/dark.png)

Regenerate them with `SHOTS=docs/screenshots ./scripts/demo.sh` (needs Playwright and
Chromium).

## Technical choices

| Layer | Choice | Why |
|-------|--------|-----|
| HTTP | Axum 0.8 + Tokio | Type-safe, mature ecosystem, single binary |
| Database | SQLite (sqlx, WAL) | No external service, single-file backups, plenty for self-hosted scale |
| Migrations | Embedded SQL with checksum verification | Migrations run on startup; the binary ships its own schema |
| UI | Askama SSR with hand-written CSS/JS | No Node/Vite build step, very small image |
| Authentication | Server-side sessions + API tokens | Revocable immediately, without adding a JWT dependency |
| Passwords | Argon2id (19 MiB, t=2, p=1) | OWASP-recommended parameters |

**Deliberately out of scope**: OAuth, user group permission matrices, GeoIP, keyword
blocklists, webhooks, email delivery. The tables and architecture leave room for them
(see `docs/SPEC.md` §12).

## Project layout

```
urlet/
├── .github/workflows/release-image.yml  # tag-driven multi-arch image release
├── Cargo.toml
├── Dockerfile                  # multi-stage build (rust:alpine → alpine)
├── docker-compose.yml          # single container + named volume
├── docker-compose.image.yml    # run a published image instead of building
├── docker-compose.tls.yml      # optional: Caddy with automatic HTTPS
├── deploy/Caddyfile
├── migrations/                 # embedded SQL migrations
├── templates/                  # Askama templates (checked at compile time)
├── scripts/
│   ├── smoke.sh                # end-to-end acceptance test (curl)
│   ├── demo.sh                 # generate demo data and screenshots
│   └── screenshot.py           # Playwright screenshot script
├── src/
│   ├── assets/                 # embedded CSS/JS/favicon
│   ├── db/                     # all SQL (one module per table)
│   ├── middleware/             # auth, CSRF, security headers, request id
│   ├── routes/                 # HTTP: api_*, pages/*, redirect, health
│   ├── services/               # business logic: auth, urls, slug, click, admin...
│   ├── config.rs  error.rs  state.rs  server.rs
│   └── main.rs                 # CLI entry point
└── docs/{SPEC.md,PLAN.md,API.md,DEPLOYMENT.md}
```

## Development

```bash
cargo test                 # unit + integration tests (285)
cargo clippy --all-targets -- -D warnings
cargo fmt --check
bash scripts/smoke.sh      # end-to-end acceptance run against the real binary
bash scripts/demo.sh       # build a demo site and write screenshots to .tmp/demo/shots
```

Rust 1.80 or newer is required. Integration tests use their own temporary SQLite database
and never touch your development data.

## Releasing

Releases are tag-driven and governed by [`RELEASING.md`](RELEASING.md) — read it before tagging.
In short: bump `version` in [`Cargo.toml`](Cargo.toml), refresh `Cargo.lock`, commit, then push
an annotated `vX.Y.Z` tag. The workflow validates the tag, builds `linux/amd64` and
`linux/arm64`, and publishes `ghcr.io/cxyfer/urlet` with semver tags. Only the newest stable
release moves `X`, `X.Y` and `latest`; pre-releases never do. The GitHub Release is written by
hand afterwards.

To rebuild an existing tag, run *Actions → Release container image → Run workflow* (moving tags
off unless it is the newest stable release).

## Documentation

- [`docs/SPEC.md`](docs/SPEC.md) — specification: requirements, data model, API, non-functional requirements
- [`docs/PLAN.md`](docs/PLAN.md) — phased implementation plan and acceptance criteria
- [`docs/API.md`](docs/API.md) — JSON API reference
- [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md) — deployment, configuration, backup, upgrades, troubleshooting
- [`RELEASING.md`](RELEASING.md) — release rules: versioning, tag/image rules, checklist, rollback

Chinese translations of these documents live under `docs/zh-TW/`.

## Security

- Argon2id password hashing; sessions, API tokens, and invite codes are stored as hashes only
- Cookies: `HttpOnly`, `SameSite=Lax`, `Secure` when enforced
- CSRF: state-changing requests authenticated by cookie are checked for same origin; form
  submissions are always checked
- Security headers: CSP (`default-src 'self'`), `X-Content-Type-Options`,
  `X-Frame-Options`, `Referrer-Policy`, plus HSTS over HTTPS
- Open redirect protection: targets must be `http` or `https`; `javascript:`, `data:`, and
  `file:` are always rejected
- Privacy: raw IP addresses are never stored, only an HMAC'd `ip_hash` and a daily-rotating
  `visitor_hash`
- Rate limiting: sign-in (per IP and per account), registration, unlock, link creation,
  password change
- Bot verification: optional Turnstile or reCAPTCHA v2 on registration, sign-in and link
  creation, toggled per form with `URLET_CAPTCHA_ON_REGISTER`, `URLET_CAPTCHA_ON_LOGIN` and
  `URLET_CAPTCHA_ON_CREATE`
- Account lockout: 30 failed sign-ins for the same account within 15 minutes temporarily
  blocks it (persisted, so a restart does not reset the counter)
- Reverse proxy: the proxy's `X-Real-IP` is trusted first, otherwise the rightmost
  `X-Forwarded-For` entry is used
- Password hashing runs on a blocking thread pool with at most 4 concurrent Argon2
  computations
- Audit log: sign-in, registration, link changes, tokens, admin actions, settings changes

## License

MIT, see [`LICENSE`](LICENSE).
