**English** | [繁體中文](zh-TW/SPEC.md)

# uwurls — Lightweight Multi-User URL Shortener Specification

> Version: 1.1
> Status: Approved for implementation
> Last updated: 2026-10-16

---

## 1. Overview

`uwurls` is a **self-hosted, multi-user** URL shortener. Every user gets an isolated space
for their own short links, can attach **password protection** and **expiry rules**, review click
analytics, and drive the service programmatically with a personal API token.

The top design principle is **as lightweight as possible**: a single Rust binary plus a single
SQLite file, deployed with one `docker compose up -d`, with no external database, cache, or
frontend build step. The v1.1 line adds a trilingual interface (English / Traditional Chinese /
Simplified Chinese), optional bot protection, and a public landing page.

### 1.1 Core Design Principles (from the requester)

| # | Principle | How it is realized |
|---|-----------|--------------------|
| P1 | Multi-user, each user manages their own links | Every resource query is forced through a `user_id` ownership filter; cross-user access always returns 404 |
| P2 | Password protection | Per-link Argon2id password hash + signed unlock cookie (TTL 30 minutes) |
| P3 | Expiry | Absolute `expires_at` plus a `max_clicks` cap; an expired link returns 410 Gone |
| P4 | Docker Compose deployment | A single `docker-compose.yml` (app + named volume), with an optional Caddy TLS overlay |
| P5 | As lightweight as possible | One binary, SQLite (no MySQL/Redis), SSR templates (no Node build), image < 40 MB |
| P6 | Rust implementation | Rust 2021 + Axum 0.8 + Tokio + sqlx (SQLite) |
| P7 | Well-designed schema | 14 tables, 27 indexes, soft deletes, audit log, time-series rollup table, ready to grow |
| P8 | Sensible extra features | Tags, click analytics, API tokens, invite codes, admin panel, audit log, daily rollups, trilingual UI, optional bot protection |

### 1.2 Relationship to the Earlier Prototypes (OURLS / OURLS2)

Two directions were explored in earlier projects. This project **inherits their data model and
feature ideas** but re-selects the stack to satisfy P5:

| Aspect | OURLS (prototype) | OURLS2 (prototype) | **uwurls (this project)** |
|--------|-------------------|--------------------|---------------------------|
| Runtime | Axum + MySQL + Redis | Cloudflare Workers (WASM) + D1 | **Axum + SQLite (single binary)** |
| Deployment | docker compose (3 containers) | Wrangler / Cloudflare | **docker compose (1 container)** |
| Auth | JWT + refresh token + OAuth | JWT + OAuth | **Server-side sessions + API tokens** |
| Frontend | React + Vite + Tailwind (Node build) | React + Vite + Pages | **Askama SSR + vanilla CSS/JS (zero build)** |
| Permissions | User groups + permission matrix | Groups + permission matrix | **Role (user/admin) + status** |
| Analytics | Full (GeoIP, MaxMind) | Full | **Lean (UA parsing, daily rollups, no GeoIP)** |
| Inherited | Password protection, expiry, click cap, tags, API tokens, invite codes, audit log | Same + public preview API | All of the above, trimmed to essentials |

**Deliberate non-goals**: OAuth third-party login, user-group permission matrices, GeoIP/MMDB,
keyword blocklists, A/B testing, webhooks, QR codes, link-in-bio pages, and email delivery.
Multilingual support was a non-goal in v1.0 and is now in scope as of v1.1 (§4.11).

---

## 2. Glossary

| Term | Description |
|------|-------------|
| Slug | The path segment of a short link, for example `https://s.example.com/`**`aB3xK9z`** |
| Target URL | The original URL that a short link redirects to |
| Unlock | The flow that exchanges a correct link password for a short-lived authorization cookie |
| Expired | `expires_at` has passed **or** `max_clicks` has been reached; the response is 410 Gone |
| Session | Server-side login state (`sessions` table + HttpOnly cookie) |
| API token | Long-lived credential for programmatic access (`Authorization: Bearer <token>`) |
| Language tag | One of the supported UI locales: `en`, `zh-Hant`, `zh-Hans` |
| Locale catalog | The JSON file embedded in the binary (`src/i18n/<tag>.json`) that holds one language's message keys |
| Captcha token | The provider-issued response token submitted with a form and verified server-side |

---

## 3. System Architecture

### 3.1 Architecture Diagram

```
                        ┌─────────────────────────────────────────────────┐
                        │ uwurls (single container)                       │
Browser / curl          │                                                 │
───────────────────►    │ Axum Router                                     │
:8080                   │  ├─ /                 landing / navigation page │
                        │  ├─ /{slug}           redirect / password page  │
                        │  ├─ /api/v1/**        JSON API                  │
                        │  ├─ /app/**           SSR user + admin UI       │
                        │  ├─ /app/lang/{code}  language switch (302)     │
                        │  ├─ /assets/**        embedded CSS/JS           │
                        │  └─ /health           health check              │
                        │                                                 │
                        │ Middleware: security headers → request ID → log │
                        │             → session/token auth → language     │
                        │             → CSRF → rate limit → captcha       │
                        │                                                 │
                        │ Services: slug / url / click / ua / ratelimit / │
                        │           audit / settings / i18n / captcha     │
                        │                                                 │
                        │ sqlx Pool (WAL)  ──►  /data/uwurls.db           │
                        └─────────────────────────────────────────────────┘
                         named volume: uwurls-data
```

### 3.2 Technology Choices and Rationale

| Layer | Choice | Rationale (relative to the earlier prototypes) |
|-------|--------|-----------------------------------------------|
| HTTP | `axum` 0.8 + `tokio` | Mature ecosystem, type safety, single binary |
| DB | `sqlx` 0.8 + SQLite (`bundled`) | No external service, single-file backup, WAL gives enough concurrent reads for a self-hosted scale |
| Migrations | SQL embedded with `include_str!` + a small custom runner | Embedded at compile time, applied automatically on startup, checksums recorded in `schema_migrations`, no CLI needed |
| Templates | `askama` 0.12 | Compile-time-checked SSR, no Node/Vite build pipeline |
| Static assets | CSS/JS embedded with `include_str!` | The binary carries its own assets, so the image needs no writable filesystem |
| i18n | JSON catalogs embedded with `include_str!` (`src/i18n/mod.rs` + `src/i18n/{en,zh-Hant,zh-Hans}.json`) | Ships with the binary, no external locale files; a missing key falls back to English |
| Passwords | `argon2` (Argon2id) | Industry standard, resistant to GPU brute force |
| Sessions/tokens | Custom `sha2` hashing + random `getrandom` | No JWT dependency; server-side revocation is immediate |
| Bot protection | Optional Cloudflare Turnstile or Google reCAPTCHA v2 over HTTPS, through `ureq` 2 on rustls + webpki-roots | Off by default, no vendor lock-in, no OpenSSL dependency, fail-closed verification |
| Logging | `tracing` + `tracing-subscriber` | Structured logs controlled by `RUST_LOG` |
| Time | `time` 0.3 | Lighter than `chrono`; ISO-8601 formatting is all that is needed |

**Dependencies deliberately avoided**: Redis, a MySQL client, jsonwebtoken, MaxMind/GeoIP, and the
React/Node toolchain. No outbound request ever fetches the target URL, which also removes the SSRF
risk. The only outbound HTTP call in the system is the optional captcha siteverify request
(`src/services/captcha.rs`): it goes through `ureq` 2 with the rustls + webpki-roots stack (pure
Rust, so the `rust:1-alpine` musl image needs no OpenSSL), it is disabled unless an administrator
configures a provider, it times out after **5 seconds**, and any failure rejects the request
(fail closed).

### 3.3 Binary and Image Size Targets

| Item | Target | v1.0 | v1.1 |
|------|--------|------|------|
| Release binary (stripped, LTO, `opt-level=z`) | ≤ 15 MB | 4.2 MB (glibc) / 4.5 MB (musl) | **5.7 MB** (glibc, local) / **5.8 MB** (musl, in container) |
| Runtime image (`alpine` + binary + CA certificates) | ≤ 40 MB | 13.2 MB | **14.8 MB** |
| Idle / under-load memory | ≤ 30 MB RSS | 6.8 MB (after 500 requests) | **8.7 MB** (after 500 requests) |
| Cold start to serving | ≤ 1 second | ~0.2 s (including migrations) | **~0.1 s** (fresh database, including migrations) |

The v1.1 column includes the embedded locale catalogs and the rustls-based captcha client. Every
target is still met.

---

## 4. Functional Requirements

### 4.1 Users and Authentication (FR-AUTH)

| ID | Requirement | Description |
|----|-------------|-------------|
| FR-AUTH-1 | Registration | Email + password. The registration mode is controlled by the `registration_mode` setting: `open` / `invite` / `closed` |
| FR-AUTH-2 | Login | Email + password; on success a session is created (HttpOnly cookie, default TTL 7 days). Each activity (at most one write per minute) refreshes both `last_seen_at` and `expires_at` and re-issues the cookie, so continuous use never logs the user out |
| FR-AUTH-3 | Logout | Revoke the current session (`revoked_at`) and clear the cookie |
| FR-AUTH-4 | Change password | Requires the current password; on success all other sessions of that user are revoked |
| FR-AUTH-5 | Profile | Change the display name; view email, role, and creation time |
| FR-AUTH-6 | Session management | List your own signed-in devices and revoke any session |
| FR-AUTH-7 | First user | If the database has no users, the first registrant automatically becomes `admin`; an admin can also be created at startup with `UWURLS_ADMIN_EMAIL/PASSWORD` |
| FR-AUTH-8 | Suspension | A user with `status = suspended` cannot log in and existing sessions become invalid immediately |
| FR-AUTH-9 | Rate limiting | Login, registration, and unlock are protected by in-memory token buckets, and `login_attempts` is recorded for auditing |

**Password policy**: 8–128 characters; Argon2id parameters `m=19456 KiB, t=2, p=1` (OWASP recommendation).

### 4.2 Short Link Management (FR-URL)

| ID | Requirement | Description |
|----|-------------|-------------|
| FR-URL-1 | Create | `target_url` is required; `slug`, title, notes, tags, password, expiry, click cap, and redirect type are optional |
| FR-URL-2 | Automatic slug | When no slug is given, generate a 7-character base62 slug and retry on collision (at most 5 times) |
| FR-URL-3 | Custom slug | Length 1–64, only `[A-Za-z0-9_-]`, must not be a reserved word, globally unique |
| FR-URL-4 | Edit | Change target, title, notes, tags, password (set/clear), expiry, click cap, redirect type, and status |
| FR-URL-5 | Delete | Soft delete (`deleted_at`); the slug is retained and never released (so links cannot be hijacked), and click records are kept |
| FR-URL-6 | List | Pagination, keyword search (slug/target/title), tag filter, status filter, sorting (creation time / click count) |
| FR-URL-7 | Status toggle | `active` / `disabled`; a `disabled` link does not redirect (returns a 404 page) |
| FR-URL-8 | Tags | Per-user tags with unique names (within a user); missing tags are created automatically when a link is created |
| FR-URL-9 | Quota | The per-user link limit is controlled by the `max_urls_per_user` setting (0 = unlimited) |
| FR-URL-10 | Ownership isolation | Every `/api/v1/urls*` query is forced to `user_id = current_user`; another user's resource always returns 404 |

### 4.3 Password Protection (FR-PWD)

| ID | Requirement | Description |
|----|-------------|-------------|
| FR-PWD-1 | Set | The create/edit forms accept a plaintext password, which is stored as an Argon2id hash in `urls.password_hash` |
| FR-PWD-2 | Access | Visiting a protected link returns 200 with a password entry page (**the target URL is not leaked**) |
| FR-PWD-3 | Unlock | `POST /{slug}/unlock` verifies the password; on success it sets the signed cookie `uwurls_unlock_<slug>` (HMAC-SHA256, TTL 30 minutes) |
| FR-PWD-4 | Already unlocked | With a valid unlock cookie the visitor is redirected immediately and the click is recorded with `password_protected = 1` |
| FR-PWD-5 | Rate limiting | At most 10 attempts per (slug, IP) in 5 minutes; further attempts return 429 |
| FR-PWD-6 | Error handling | A wrong password returns 401 and shows an error on the page without revealing the target URL |

### 4.4 Expiry (FR-EXP)

| ID | Requirement | Description |
|----|-------------|-------------|
| FR-EXP-1 | Time expiry | `expires_at` (ISO-8601 UTC). Once reached, the link stops working and returns a **410 Gone** page |
| FR-EXP-2 | Click cap | When `max_clicks` is set and `click_count >= max_clicks`, the link stops working and returns 410. The counter is incremented asynchronously by a background writer, so concurrent requests may slightly exceed the cap (known trade-off: the redirect path never blocks on a write) |
| FR-EXP-3 | Visibility | The UI marks links with "expired / cap reached" badges and offers one-click extend or raise-cap actions |
| FR-EXP-4 | Expiry does not delete | Expiry only affects redirection; data and analytics are retained |
| FR-EXP-5 | Deleted | A non-null `deleted_at` → 404 (indistinguishable from a missing link) |
| FR-EXP-6 | Disabled | `status = disabled` → 404 |

### 4.5 Redirection and Click Recording (FR-REDIR)

| ID | Requirement | Description |
|----|-------------|-------------|
| FR-REDIR-1 | Routing | `GET /{slug}`; `/api/**`, `/app/**`, `/assets/**`, and `/health` are reserved prefixes |
| FR-REDIR-2 | Redirect type | Supports 301 / 302 (default) / 307 / 308 |
| FR-REDIR-3 | Click recording | Written asynchronously (`tokio::spawn`) to `clicks` and `url_daily_stats`, never blocking the redirect |
| FR-REDIR-4 | User-agent parsing | Parses device_type / browser / os; bot user agents are flagged with `is_bot = 1` |
| FR-REDIR-5 | Privacy | Only `ip_hash = HMAC(secret, ip)` and `visitor_hash = HMAC(secret, ip + ua + date)` are stored; **the raw IP is never stored** |
| FR-REDIR-6 | Unique visitors | The first click for a given `visitor_hash` on a given URL is recorded with `is_unique = 1` |
| FR-REDIR-7 | Cache control | Password pages / 410 / 404 are `no-store`; 302/307 redirects are `private, no-store`; 301/308 redirects are `private, max-age=300` (permanent redirects may be cached briefly; everything else is uncached to keep click counts accurate) |
| FR-REDIR-8 | Analytics switch | When `analytics_enabled = false`, no click is written |

### 4.6 Analytics (FR-STAT)

| ID | Requirement | Description |
|----|-------------|-------------|
| FR-STAT-1 | Overview | Link count, total clicks, clicks over the last 7/30 days, unique visitors, bot share |
| FR-STAT-2 | Single link | Daily click trend (last 30 days), top referrer domains, device/browser/OS breakdown, latest 50 clicks |
| FR-STAT-3 | Charts | Server-rendered SVG (no frontend chart library) |
| FR-STAT-4 | Retention | `click_retention_days` (default 0 = forever); when > 0, a daily maintenance task prunes old records |

### 4.7 API Tokens (FR-TOK)

| ID | Requirement | Description |
|----|-------------|-------------|
| FR-TOK-1 | Create | Generates `uw_` + 32 bytes of CSPRNG entropy (base64url, 43 characters); the plaintext is shown **only once** at creation time |
| FR-TOK-2 | Storage | Only the SHA-256 hash plus the first 8 characters as a prefix (for identification) are stored |
| FR-TOK-3 | Usage | `Authorization: Bearer <token>`; works alongside session authentication |
| FR-TOK-4 | Lifecycle | Supports `expires_at`, revocation (`revoked_at`), and `last_used_at`; when a user changes their password, or an admin resets a password or deletes an account, all of that account's tokens are revoked too |
| FR-TOK-5 | Scopes | The `scopes` column is reserved (v1 provides `urls:read`, `urls:write`, `admin`), but v1 only checks whether the token is an admin token |

### 4.8 Admin Panel (FR-ADMIN)

| ID | Requirement | Description |
|----|-------------|-------------|
| FR-ADMIN-1 | User management | List, search, change role, suspend/restore, reset password, soft delete |
| FR-ADMIN-2 | All links | List, search, force-disable/delete |
| FR-ADMIN-3 | Invite codes | Create (uses/expiry/note), list, revoke; registration in `invite` mode requires an invite code |
| FR-ADMIN-4 | Site settings | Edit the `settings` table (site name, registration mode, quotas, default redirect type, analytics switch, retention days, captcha) |
| FR-ADMIN-5 | Audit log | Records logins, registrations, link changes, admin actions, and setting changes; filterable by action/target |
| FR-ADMIN-6 | System info | Version, database size, user/link/click totals, uptime |
| FR-ADMIN-7 | Authorization | Every `/api/v1/admin/**` and `/app/admin/**` route requires `role = admin`, otherwise 403 |

### 4.9 JSON API (FR-API)

- Prefix `/api/v1`, responses are `application/json`.
- Uniform error format:
  ```json
  { "error": { "code": "validation_error", "message": "slug is invalid", "details": { "field": "slug" } } }
  ```
- Error codes: `bad_request` `unauthorized` `forbidden` `not_found` `conflict` `gone` `validation_error` `captcha_failed` `rate_limited` `internal_error`
- Pagination format: `{ "items": [...], "page": 1, "per_page": 20, "total": 42, "total_pages": 3 }`
- Authentication: session cookie **or** `Authorization: Bearer <api_token>`.
- **All JSON API responses are English** and are not affected by the request language or the `uwurls_lang` cookie.
- See §7 for the endpoint list.

### 4.10 Web Interface (FR-UI)

| Path | Page | Description |
|------|------|-------------|
| `/` | Landing / navigation | Hero, feature highlights, login and register entry points, language switcher, and footer; a signed-in visitor also gets an entry point to the dashboard. This is no longer a 302 redirect to `/app` |
| `/app/login` | Login | Email + password |
| `/app/register` | Register | Shows the invite-code field or a "closed" notice depending on the registration mode |
| `/app` | Dashboard | Stat cards, daily trend chart, link list (search/filter/pagination), create form |
| `/app/urls/{id}` | Link detail | Edit form, analytics charts, recent clicks, delete (QR codes are a non-goal) |
| `/app/account` | Account | Profile, change password, session list, API token management |
| `/app/admin` | Admin panel | Users, all links, invite codes, settings, audit log |
| `/app/lang/{code}` | Language switch | Sets the `uwurls_lang` cookie and 302-redirects to `next` (or to `/app`) |
| `/{slug}` | Redirect / password page / 410 / 404 | Public |

**UI technology**: Askama templates + hand-written CSS (including `prefers-color-scheme` dark mode) +
roughly 150 lines of vanilla JS (copy to clipboard, confirmation dialogs, form enhancements).
**No** frontend framework or CDN resource is used (so the CSP can be restricted to `'self'`).

### 4.11 Multilingual UI (FR-I18N)

| ID | Requirement | Description |
|----|-------------|-------------|
| FR-I18N-1 | Supported languages | Three interface languages: English (`en`, the default), Traditional Chinese (`zh-Hant`), and Simplified Chinese (`zh-Hans`). Catalogs are JSON files embedded in the binary at `src/i18n/{en,zh-Hant,zh-Hans}.json` and loaded by `src/i18n/mod.rs` |
| FR-I18N-2 | Language selection | On the first visit (no cookie) the language is negotiated from `Accept-Language`, honoring `q=` weights in descending order; `zh-TW`, `zh-HK` and `zh-MO` map to `zh-Hant`, every other Chinese tag (including a bare `zh`) maps to `zh-Hans`, and an unrecognized or absent header falls back to **English**. The `uwurls_lang` cookie always wins over the header |
| FR-I18N-3 | Persistence | The chosen language is stored in the cookie `uwurls_lang` (`Path=/`, `SameSite=Lax`, `Max-Age` one year, **not** HttpOnly; `Secure` when `UWURLS_SECURE_COOKIES=true`) |
| FR-I18N-4 | Switching | `GET /app/lang/{code}?next=<path>` sets the cookie and returns 302; `next` accepts only same-site relative paths, and anything else — including a missing `next` — falls back to `/app`. An unknown `code` is treated as `en` and never fails |
| FR-I18N-5 | Markup and headers | `<html lang>` reflects the active language, a language switcher is available on the public pages and in the app shell, and HTML responses carry `Content-Language` plus `Vary: Accept-Language, Cookie` (static assets set no `Vary`) |
| FR-I18N-6 | Fallback | A key missing from a catalog falls back to the English string |
| FR-I18N-7 | API responses | All JSON API responses are English regardless of the request language or cookie |
| FR-I18N-8 | Error pages | Error pages (404/410/500 and friends) are rendered in the active language through a task-local language context |

### 4.12 Bot Protection (FR-BOT)

| ID | Requirement | Description |
|----|-------------|-------------|
| FR-BOT-1 | Providers | Optional Cloudflare Turnstile (widget response field `cf-turnstile-response`) or Google reCAPTCHA v2 checkbox (`g-recaptcha-response`); the default is `none` (disabled) |
| FR-BOT-2 | Configuration | Configurable through both environment variables and the admin settings table (new keys `captcha_provider`, `captcha_site_key`, `captcha_secret_key`, `captcha_on_register`, `captcha_on_login`, `captcha_on_create`). A non-empty environment variable overrides the stored value, consistent with the `registration_mode` override rule |
| FR-BOT-3 | Enablement | All four conditions must hold at once: provider ≠ `none`, both keys non-empty, and the switch for the relevant form (`on_register`, `on_login` or `on_create`) is `true` |
| FR-BOT-4 | Verification | The server issues a form-encoded HTTPS POST (`secret`, `response`, plus `remoteip` when the client address can be determined) to the provider's siteverify endpoint — Cloudflare `https://challenges.cloudflare.com/turnstile/v0/siteverify`, Google `https://www.google.com/recaptcha/api/siteverify` — with a 5-second timeout, through `ureq` 2 over rustls with webpki-roots |
| FR-BOT-5 | Fail closed | Any failure (timeout, non-2xx status, unparsable response) rejects the request |
| FR-BOT-6 | Coverage | Enforced on the web registration form, the web sign-in form, the web create-short-link form, `POST /api/v1/auth/register` and `POST /api/v1/auth/login` (JSON field `captcha_token`; a failure returns 400 `captcha_failed`) |
| FR-BOT-7 | API token routes | `POST /api/v1/urls` authenticates with an API token and is not subject to captcha |
| FR-BOT-8 | Admin UI | The secret key field is write-only: the stored value is never rendered back, and submitting the field empty keeps the current value |

---

## 5. Non-Functional Requirements

### 5.1 Security (NFR-SEC)

| ID | Requirement |
|----|-------------|
| NFR-SEC-1 | Passwords: Argon2id; sessions/API tokens/invite codes store only a SHA-256 hash and are looked up by indexed hash comparison (no plaintext is stored, so there is no comparable length difference); the unlock cookie signature is compared in constant time |
| NFR-SEC-2 | Cookies: `HttpOnly`, `SameSite=Lax`, `Secure` (when `UWURLS_SECURE_COOKIES=true` or the base URL is https) |
| NFR-SEC-3 | CSRF: state-changing requests check that `Origin`/`Referer` are same-origin; cookie-authenticated non-GET requests are always verified; Bearer token requests are exempt |
| NFR-SEC-4 | Security headers: `Content-Security-Policy: default-src 'self'; frame-ancestors 'none'`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: strict-origin-when-cross-origin`, `X-Frame-Options: DENY`, and `Strict-Transport-Security` over https |
| NFR-SEC-5 | Open redirect protection: targets must be `http` / `https`; `javascript:`, `data:`, `file:` and similar schemes are rejected |
| NFR-SEC-6 | Reserved words: `api`, `app`, `assets`, `health`, `static`, `favicon.ico`, and similar names cannot be used as slugs |
| NFR-SEC-7 | Error messages never leak internal details; a failed login always responds with "invalid email or password" |
| NFR-SEC-8 | User enumeration protection: registering a duplicate email returns 409 (an accepted, common trade-off); login uses a uniform error message |
| NFR-SEC-9 | Rate limiting: login 10 per 5 minutes per (IP,email) **plus** 40 per 5 minutes per IP, registration 5 per hour per IP, unlock 10 per 5 minutes per (slug,IP), link creation 120 per hour per user, password change 10 per hour per user |
| NFR-SEC-9b | Account lockout: 30 accumulated failures for the same email within 15 minutes temporarily refuse login (429); this is persisted in the `login_attempts` table and survives a restart; in-memory rate limiter keys are stored as SHA-256 and capped at 50,000 entries |
| NFR-SEC-10 | All SQL is parameterized (sqlx bind) with no string concatenation (no SQL outside `src/db/`) |
| NFR-SEC-11 | The audit log covers every sensitive operation |
| NFR-SEC-12 | If `UWURLS_SECRET` is unset at startup, a random key is generated and persisted in the data directory (created with mode 0600; an existing file with overly broad permissions is tightened automatically) |
| NFR-SEC-13 | Reverse proxy headers: with `UWURLS_TRUST_PROXY=true`, prefer the proxy-rewritten `X-Real-IP`, then the **rightmost** entry of `X-Forwarded-For` (the leftmost can be forged by the client) |
| NFR-SEC-14 | Password hashing runs in `spawn_blocking` with a semaphore limiting it to 4 concurrent Argon2 computations (about 19 MiB each) |
| NFR-SEC-15 | Captcha verification fails closed: a form-encoded HTTPS POST with a 5-second timeout to the configured provider, sent through `ureq` 2 over rustls + webpki-roots (no OpenSSL); a timeout, a non-2xx status, or an unparsable body rejects the request. The secret key is never rendered back into the admin UI |
| NFR-SEC-16 | The language cookie `uwurls_lang` is intentionally not HttpOnly (client code reads it) and holds only a whitelisted language tag; the `next` parameter of the switch endpoint accepts only same-site relative paths to prevent open redirects |

### 5.2 Performance (NFR-PERF)

| ID | Requirement |
|----|-------------|
| NFR-PERF-1 | Redirect path: a single indexed `SELECT` plus a queued write; P95 < 10 ms (measured 0.74 ms) |
| NFR-PERF-2 | SQLite: `journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`, `busy_timeout=5000` |
| NFR-PERF-3 | Connection pool: `max_connections=8`; click writes are serialized through a single background writer (so the WAL cannot grow without bound because concurrent writes prevent checkpointing) |
| NFR-PERF-4 | List queries are always paginated (default 20, maximum 100) |
| NFR-PERF-5 | Analytics queries use `url_daily_stats` and indexes to avoid full table scans |

### 5.3 Operations (NFR-OPS)

| ID | Requirement |
|----|-------------|
| NFR-OPS-1 | `docker compose up -d` completes the deployment; the first start migrates automatically and creates the admin |
| NFR-OPS-2 | Health checks: `GET /health` (200 JSON) and the `uwurls healthcheck` subcommand (used by the Docker HEALTHCHECK) |
| NFR-OPS-3 | Data is persisted in the named volume `uwurls-data` at `/data` |
| NFR-OPS-4 | Logs go to stdout (`tracing`; JSON or compact controlled by `UWURLS_LOG_FORMAT`) |
| NFR-OPS-5 | All configuration comes from environment variables (12-factor); `.env.example` documents everything |
| NFR-OPS-6 | Daily maintenance task: purge expired sessions and optionally prune expired click records |
| NFR-OPS-7 | Backup = copy `uwurls.db` (after a WAL checkpoint), as described in the README |
| NFR-OPS-8 | The container runs as a non-root user and works with a read-only root filesystem |
| NFR-OPS-9 | Locale catalogs are embedded in the binary with `include_str!`, so a deployment is still a single file with no writable asset directory |

### 5.4 Maintainability (NFR-MAINT)

| ID | Requirement |
|----|-------------|
| NFR-MAINT-1 | Layering: `routes/` (HTTP) → `services/` (business) → `db/` (SQL); no cross-layer direct queries |
| NFR-MAINT-2 | A single `AppError` type converted centrally into HTTP responses |
| NFR-MAINT-3 | All SQL lives in the `src/db/` modules and uses runtime queries (the project compiles without `DATABASE_URL`) |
| NFR-MAINT-4 | `cargo clippy -- -D warnings` and `cargo fmt --check` pass |
| NFR-MAINT-5 | Unit tests plus integration tests (including `tower::ServiceExt::oneshot` end-to-end) cover the core flows |
| NFR-MAINT-6 | Every UI string lives in the three locale catalogs; templates and handlers never hard-code display text |

---

## 6. Data Model

### 6.1 ER Overview

```
users ─┬─< sessions
       ├─< api_tokens
       ├─< urls ─┬─< clicks
       │         ├─< url_daily_stats
       │         └─< url_tags >─ tags
       ├─< tags
       ├─< invite_redemptions >─ invite_codes
       └─< audit_logs (actor)
domains ─< urls
settings (key-value)
login_attempts (auditing)
```

### 6.2 Design Principles

1. **Time columns**: all are `TEXT` holding ISO-8601 UTC (`YYYY-MM-DDTHH:MM:SS.sssZ`), directly comparable and sortable as strings.
2. **Primary keys**: `INTEGER PRIMARY KEY AUTOINCREMENT` (SQLite rowid optimization).
3. **Soft delete**: `users.deleted_at`, `urls.deleted_at`; every query adds `deleted_at IS NULL`.
4. **Enums**: `TEXT + CHECK` constraints, which leave room to grow (SQLite has no native ENUM).
5. **Booleans**: `INTEGER 0/1`.
6. **Rollup table**: `url_daily_stats` is the acceleration layer for analytics queries, upserted on write.
7. **Reserved extension columns**: `api_tokens.scopes`, `urls.notes`, the `domains` table, `audit_logs.metadata`.

### 6.3 DDL (`migrations/0001_init.sql` summary)

> The complete and only source of truth is `migrations/0001_init.sql`; the table below is a structural summary.

| Table | Main columns | Purpose |
|-------|--------------|---------|
| `users` | id, email, email_normalized(UNIQUE), password_hash, display_name, role, status, email_verified_at, last_login_at, created_at, updated_at, deleted_at | Users |
| `sessions` | id, user_id, token_hash(UNIQUE), user_agent, ip_hash, created_at, last_seen_at, expires_at, revoked_at | Login state |
| `api_tokens` | id, user_id, name, token_prefix, token_hash(UNIQUE), scopes, last_used_at, expires_at, created_at, revoked_at | API credentials |
| `domains` | id, hostname(UNIQUE), is_default, is_verified, created_by, created_at | Reserved for multi-domain (v1 uses only the default domain) |
| `urls` | id, user_id, domain_id, slug(UNIQUE), target_url, title, notes, password_hash, expires_at, max_clicks, click_count, unique_click_count, redirect_type, status, is_custom_slug, last_clicked_at, created_at, updated_at, deleted_at | Short links |
| `tags` | id, user_id, name, color, created_at, UNIQUE(user_id,name) | Tags |
| `url_tags` | url_id, tag_id (PK) | Tag associations |
| `clicks` | id, url_id, clicked_at, ip_hash, visitor_hash, user_agent, referrer, referrer_host, country_code, device_type, browser, os, is_bot, is_unique, password_protected | Click details |
| `url_daily_stats` | url_id, day, clicks, unique_clicks, bot_clicks (PK: url_id+day) | Daily rollups |
| `audit_logs` | id, actor_user_id, actor_role, action, target_type, target_id, metadata, ip_hash, created_at | Auditing |
| `settings` | key(PK), value, updated_at, updated_by | Site settings |
| `invite_codes` | id, code_hash(UNIQUE), code_prefix, note, created_by, max_uses, used_count, expires_at, status, created_at | Invite codes |
| `invite_redemptions` | id, invite_id, user_id, redeemed_at, UNIQUE(invite_id,user_id) | Invite redemption records |
| `login_attempts` | id, identifier_hash, ip_hash, success, created_at | Login auditing/lockout |

### 6.4 Index Strategy

| Index | Purpose |
|-------|---------|
| `urls(slug)` UNIQUE | Redirect lookup (hottest path) |
| `urls(user_id, created_at DESC)` | "My links" listing |
| `urls(expires_at) WHERE expires_at IS NOT NULL` | Expiry pruning/filtering |
| `clicks(url_id, clicked_at DESC)` | Per-link analytics |
| `clicks(clicked_at)` | Retention pruning |
| `sessions(user_id, expires_at)`, `sessions(expires_at)` | Session validation and cleanup |
| `api_tokens(token_hash)` UNIQUE | Token authentication |
| `url_daily_stats(day)` | Site-wide trends |
| `audit_logs(created_at DESC)`, `audit_logs(actor_user_id, created_at DESC)`, `audit_logs(target_type,target_id)` | Audit queries |
| `login_attempts(identifier_hash, created_at DESC)` | Brute-force detection |

The v1.1 changes add settings **keys** only; because `settings` is a key-value table, the schema
still has **14 tables and 27 indexes** (§6.1, §6.3, §6.4 are unchanged).

### 6.5 Seed Data

- `settings`: `site_name=uwurls`, `registration_mode=open`, `default_slug_length=7`,
  `max_urls_per_user=1000`, `default_redirect_type=302`, `analytics_enabled=true`, `click_retention_days=0`.
- `settings` (v1.1 captcha keys): `captcha_provider=none`, `captcha_site_key=` (empty),
  `captcha_secret_key=` (empty), `captcha_on_register=false`, `captcha_on_login=false`,
  `captcha_on_create=false`. A non-empty environment variable overrides the stored value (FR-BOT-2).
- `domains`: at startup the default domain is inserted/updated from the host of `UWURLS_BASE_URL` (`is_default=1`).

---

## 7. API Specification

### 7.1 Authentication

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/v1/auth/register` | `{email, password, invite_code?, captcha_token?}` → 201 `{user}`; 400 `captcha_failed` when captcha verification fails |
| POST | `/api/v1/auth/login` | `{email, password, captcha_token?}` → 200 `{user}` + Set-Cookie; 400 `captcha_failed` when captcha verification fails |
| POST | `/api/v1/auth/logout` | Revoke the current session → 204 |
| GET | `/api/v1/auth/me` | Current user → 200 `{user}` |
| PATCH | `/api/v1/auth/me` | `{display_name}` → 200 `{user}` |
| POST | `/api/v1/auth/password` | `{current_password, new_password}` → 204 |
| GET | `/api/v1/auth/sessions` | List my sessions |
| DELETE | `/api/v1/auth/sessions/{id}` | Revoke the given session → 204 |

### 7.2 Short Links

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/urls` | Query parameters: `q, tag, status, page, per_page, sort(created_at\|clicks)` |
| POST | `/api/v1/urls` | `{target_url, slug?, title?, notes?, tags?, password?, expires_at?, max_clicks?, redirect_type?}`; API token authentication, captcha not applied |
| GET | `/api/v1/urls/{id}` | Single link (with `short_url`, `is_expired`, `has_password`) |
| PATCH | `/api/v1/urls/{id}` | Partial update; `password: null` clears the password; `expires_at: null` clears the expiry |
| DELETE | `/api/v1/urls/{id}` | Soft delete → 204 |
| GET | `/api/v1/urls/{id}/stats?days=30` | Daily trend + dimension breakdown + summary |
| GET | `/api/v1/urls/{id}/clicks?page=1` | Click details (paginated) |

### 7.3 Tags / Tokens

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/tags` | My tags (with usage counts) |
| DELETE | `/api/v1/tags/{id}` | Delete a tag (only removes the association) |
| GET | `/api/v1/tokens` | My API tokens (never includes plaintext) |
| POST | `/api/v1/tokens` | `{name, expires_at?}` → 201 `{token: "uw_...", api_token}` **shown only once** |
| DELETE | `/api/v1/tokens/{id}` | Revoke → 204 |

### 7.4 Admin (requires admin)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/admin/users` | User list (`q, role, status, page`) |
| PATCH | `/api/v1/admin/users/{id}` | `{role?, status?, display_name?}` |
| POST | `/api/v1/admin/users/{id}/password` | `{new_password}` reset |
| DELETE | `/api/v1/admin/users/{id}` | Soft delete (their links are soft-deleted too) |
| GET | `/api/v1/admin/urls` | Site-wide link list |
| PATCH | `/api/v1/admin/urls/{id}` | Force disable/enable/delete |
| GET | `/api/v1/admin/invites` | Invite code list |
| POST | `/api/v1/admin/invites` | `{max_uses?, expires_at?, note?}` → 201 `{code: "inv_...", invite}` |
| DELETE | `/api/v1/admin/invites/{id}` | Revoke → 204 |
| GET | `/api/v1/admin/settings` | Site settings |
| PUT | `/api/v1/admin/settings` | Update settings (whitelisted keys) |
| GET | `/api/v1/admin/audit` | Audit log (`action, target_type, actor, page`) |
| GET | `/api/v1/admin/stats` | System info and totals |

### 7.5 Public

| Method | Path | Description |
|--------|------|-------------|
| GET | `/` | Landing / navigation page (HTML) |
| GET | `/health` | `{status, version, database, uptime_seconds}` |
| GET | `/app/lang/{code}` | Set the `uwurls_lang` cookie and 302 to `next`, or to `/app` when `next` is missing or not a same-site relative path |
| GET | `/{slug}` | Redirect (302 and friends), password page (HTML), or a 410/404 page |
| POST | `/{slug}/unlock` | Form or JSON `{password}`; on success 303 → `/{slug}` |
| GET | `/api/v1/public/urls/{slug}` | Public metadata: `{slug, has_password, is_expired, title?}` (**never includes the target**) |

---

## 8. Configuration (Environment Variables)

| Variable | Default | Description |
|----------|---------|-------------|
| `UWURLS_BIND` | `0.0.0.0:8080` | Listen address |
| `UWURLS_BASE_URL` | `http://localhost:8080` | Public base URL (used to build short links, check CSRF same-origin, and decide cookie `Secure`) |
| `UWURLS_DATABASE_URL` | `sqlite:///data/uwurls.db?mode=rwc` | SQLite connection string |
| `UWURLS_DATA_DIR` | Directory containing the database | Where an auto-generated secret is stored |
| `UWURLS_HEALTHCHECK_URL` | Derived from `UWURLS_BIND` | Target URL for `uwurls healthcheck` |
| `UWURLS_SECRET` | Auto-generated and persisted | HMAC key for sessions/unlock cookies/IP hashing (at least 16 characters; auto-generated keys are 44-character base64) |
| `UWURLS_SECRET_FILE` | `<data dir>/secret.key` | Where an auto-generated secret is stored |
| `UWURLS_ADMIN_EMAIL` | — | Admin email to create/upgrade at startup |
| `UWURLS_ADMIN_PASSWORD` | — | Password for that admin (used only on first creation) |
| `UWURLS_REGISTRATION` | Read from the settings table | Override the registration mode `open\|invite\|closed` |
| `UWURLS_SECURE_COOKIES` | Derived from `BASE_URL` | Force the cookie `Secure` flag |
| `UWURLS_TRUST_PROXY` | `false` | Trust `X-Forwarded-For` / `X-Forwarded-Proto` |
| `UWURLS_SESSION_TTL_HOURS` | `168` | Session lifetime (hours) |
| `UWURLS_UNLOCK_TTL_MINUTES` | `30` | Short-link unlock cookie lifetime (minutes) |
| `UWURLS_LOG` | `info` | `tracing` filter |
| `UWURLS_LOG_FORMAT` | `compact` | `compact` \| `json` |
| `UWURLS_DEFAULT_SLUG_LENGTH` | Read from the settings table | Override the automatic slug length (4–16) |
| `UWURLS_CAPTCHA_PROVIDER` | `none` | Bot protection provider: `none` \| `turnstile` \| `recaptcha`; overrides `captcha_provider` when non-empty |
| `UWURLS_CAPTCHA_SITE_KEY` | — | Provider site key; overrides `captcha_site_key` when non-empty |
| `UWURLS_CAPTCHA_SECRET_KEY` | — | Provider secret key; overrides `captcha_secret_key` when non-empty |
| `UWURLS_CAPTCHA_ON_REGISTER` | `false` | Require captcha on the registration forms; overrides `captcha_on_register` when non-empty |
| `UWURLS_CAPTCHA_ON_LOGIN` | `false` | Require captcha on the web sign-in form and on `POST /api/v1/auth/login`; overrides `captcha_on_login` when non-empty |
| `UWURLS_CAPTCHA_ON_CREATE` | `false` | Require captcha on the web create-short-link form; overrides `captcha_on_create` when non-empty |

The captcha variables follow the same override rule as `UWURLS_REGISTRATION` and
`UWURLS_DEFAULT_SLUG_LENGTH`: a non-empty environment variable wins over the value stored in the
`settings` table (FR-BOT-2).

---

## 9. Deployment

### 9.1 Directory Structure

```
uwurls/
├── Cargo.toml  Cargo.lock  rustfmt.toml
├── Dockerfile  .dockerignore
├── docker-compose.yml            # single container + named volume
├── docker-compose.tls.yml        # optional: Caddy automatic HTTPS
├── deploy/Caddyfile
├── .env.example
├── migrations/0001_init.sql  migrations/0002_clicks_indexes.sql
├── docs/{SPEC.md,PLAN.md,API.md,DEPLOYMENT.md,screenshots/,zh-TW/{SPEC.md,PLAN.md,API.md,DEPLOYMENT.md}}
├── scripts/{smoke.sh,demo.sh,screenshot.py}
├── templates/                    # Askama templates (base, app_base, landing.html + pages + admin/)
├── src/
│   ├── main.rs  lib.rs  config.rs  state.rs  error.rs  server.rs  assets.rs
│   ├── assets/{app.css,app.js,favicon.svg}
│   ├── i18n/{mod.rs,en.json,zh-Hant.json,zh-Hans.json}
│   ├── db/{mod,migrate,users,sessions,tokens,urls,tags,clicks,settings,invites,audit,domains,login_attempts}.rs
│   ├── services/{auth,urls,admin,slug,target,click,ua,unlock,cookies,chart,audit,settings,time,secret,password,ratelimit,tokens,captcha}.rs
│   ├── middleware/{mod,auth,csrf,security,locale}.rs
│   └── routes/{mod,api,api_auth,api_urls,api_tags,api_tokens,api_stats,api_admin,api_public,redirect,health,assets,dto,extract}.rs
│       └── pages/{mod,auth,dashboard,urls,account,admin,home}.rs
└── tests/{common/mod.rs,auth.rs,urls.rs,redirect.rs,pages.rs,admin.rs,i18n.rs,captcha.rs}
```

### 9.2 Dockerfile Strategy

Multi-stage build:
1. `rust:1-alpine` + `build-base` → `cargo build --release --target x86_64-unknown-linux-musl` (LTO, `strip`, `opt-level=z`)
2. `alpine:3.21` → copy the binary plus `ca-certificates` and `tzdata`, create a non-root user, `HEALTHCHECK CMD ["/usr/local/bin/uwurls","healthcheck"]`

### 9.3 Compose Services

```yaml
services:
  uwurls:
    build: .
    image: uwurls:latest
    ports: ["8080:8080"]
    env_file: [.env]
    volumes: [uwurls-data:/data]
    healthcheck: ...   # uwurls healthcheck
    restart: unless-stopped
volumes:
  uwurls-data:
```

`docker-compose.tls.yml`: adds Caddy (80/443, automatic Let's Encrypt) and stops exposing the
uwurls port, so Caddy reverse-proxies it.

---

## 10. Test Strategy

| Layer | Scope | Tooling |
|-------|-------|---------|
| Unit tests | Slug generation/validation/reserved words, UA parsing, expiry decisions, settings parsing, password hashing, IP hashing, CSRF same-origin checks, `Accept-Language` negotiation (`q=` weights, fallback to English), locale fallback, the captcha enablement matrix, and captcha fail-closed classification | `#[cfg(test)]` modules |
| Integration tests | Register → login → create → redirect → analytics; the password protection flow; expiry and click caps; ownership isolation; admin authorization; API tokens; CSRF rejection; language negotiation and cookie switching; API JSON staying English; captcha rejection on register, sign-in and create (web forms and JSON API); the landing page | `tests/integration.rs` + `tower::ServiceExt::oneshot` + a temporary SQLite database |
| End-to-end smoke test | Runs the main flows against the real binary with curl | `scripts/smoke.sh` |
| Static checks | `cargo fmt --check`, `cargo clippy -D warnings` | CI commands (documented in the README) |

---

## 11. Definition of Done

v1.0:

- [x] `cargo build --release` succeeds and the binary is **4.2 MB** ≤ 15 MB
- [x] `cargo test` is all green: **178** items (93 unit + 84 integration + 1 binary)
- [x] `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` report no warnings
- [x] `scripts/smoke.sh` passes all **81** checks against the real binary (including password protection, expiry, ownership isolation, CSRF, invite codes, and restart persistence)
- [x] `docker compose config` passes (both the base file and the TLS overlay verified with Docker Compose 27.4)
- [x] `docker build` succeeds, the image is **13.2 MB**, and the container reports `healthy` after `docker compose up -d`
- [x] The container runs as uid 10001 with a read-only root filesystem and `cap_drop: ALL`; data is written only to the `/data` volume
- [x] Container verification: health, admin login, creating a link with password/expiry/click cap, unlock-and-redirect, all admin pages returning 200, and sessions plus unlock cookies still valid after a restart
- [x] 14 tables + indexes created per §6, migrated automatically at startup (`schema_migrations` is checksum-protected)
- [x] Every FR/NFR item has a corresponding implementation or test
- [x] The README lets a new user deploy within 5 minutes
- [x] Every phase has a corresponding local commit (see `docs/PLAN.md`)

v1.1 additions (this batch):

- [ ] Three locale catalogs (`en`, `zh-Hant`, `zh-Hans`) are embedded in the binary and cover every UI string, with a missing key falling back to English
- [ ] `Accept-Language` negotiation honors `q=` weights and defaults to English for an unrecognized or absent header
- [ ] `GET /app/lang/{code}?next=<path>` sets `uwurls_lang` (Path=/, SameSite=Lax, Max-Age one year, not HttpOnly) and returns 302 to `next` or to `/app`, and `next` accepts only same-site relative paths
- [ ] `<html lang>` reflects the active language and error pages are localized
- [ ] HTML responses carry `Content-Language` and `Vary: Accept-Language, Cookie`, while static assets set no `Vary`
- [ ] JSON API responses stay English regardless of the request language
- [ ] Captcha is enabled only when provider ≠ none, both keys are non-empty, and the matching switch (`on_register`, `on_login` or `on_create`) is true; a non-empty environment variable overrides the stored setting
- [ ] Captcha verification fails closed on timeout, non-2xx status, or an unparsable response, and is enforced on web register, web sign-in, web create, `POST /api/v1/auth/register` and `POST /api/v1/auth/login` (400 `captcha_failed`)
- [ ] The siteverify call is the only outbound HTTP request, goes through `ureq` 2 over rustls + webpki-roots (no OpenSSL), and is off by default
- [ ] Captcha is not applied to `POST /api/v1/urls`, and the admin secret-key field is write-only
- [ ] `GET /` returns 200 as a landing/navigation page for anonymous and signed-in visitors, and no longer redirects to `/app`
- [ ] `cargo test` (245 tests), `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are clean, `scripts/smoke.sh` passes all 81 end-to-end checks, and the container image builds and runs under musl (§3.3)

---

## 12. Future Extension Paths (this version only reserves schema/architecture room)

| Feature | Already reserved |
|---------|------------------|
| Multiple / custom domains | `domains` table + `urls.domain_id` |
| User groups and fine-grained permissions | `api_tokens.scopes`; `users.role` can grow into a `groups` table |
| Password reset / email verification | `users.email_verified_at`; a `password_resets` table can be added |
| GeoIP country analytics | `clicks.country_code` already exists; only a resolver is missing |
| Click export / CSV | The full `clicks` column set + indexes |
| Team collaboration | A `url_members` table can be added, with `urls.user_id` as the owner |
| Audit compliance | `audit_logs.metadata` keeps the full JSON context |
