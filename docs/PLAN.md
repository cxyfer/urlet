**English** | [繁體中文](zh-TW/PLAN.md)

# uwurls — Phased Implementation Plan

> Spec: `docs/SPEC.md` v1.1
> Principle: at the end of every phase the code must **compile, test, and commit**; each phase gets one (or a few) local commits.

## Results (completed)

| Phase | Status | Commit |
|-------|--------|--------|
| P0 Spec and plan | ✅ Done | `4109fc8 📝 docs: add spec and phased implementation plan` |
| P1 Project skeleton and data layer | ✅ Done | `06f91a7 🎉 init: bootstrap uwurls project skeleton` |
| P2 Authentication and sessions | ✅ Done | `e0d8902 ✨ feat(auth): add sessions, registration, login and CSRF protection` |
| P3 Short link core | ✅ Done | `e1db4fc ✨ feat(urls): add short link CRUD, slugs, tags and expiry rules` |
| P4 Redirect and unlock | ✅ Done | `2a8ca77 ✨ feat(redirect): add redirects, password unlock and click analytics` |
| P5 Web UI | ✅ Done | `eae0e0b ✨ feat(ui): add server rendered dashboard, detail and account pages` |
| P6 Admin panel and API | ✅ Done | `c74302a ✨ feat(admin): add admin panel, invites, settings and maintenance` |
| P7 Tests and documentation | ✅ Done | See `git log` (smoke.sh, README, API.md, DEPLOYMENT.md) |
| P8 Final verification | ✅ Done | See `git log` (release build, image strategy, wrap-up) |

Acceptance data: `cargo test` is all green with 187 tests, `cargo clippy --all-targets -- -D warnings`
and `cargo fmt --check` report no warnings, and all 81 end-to-end checks in `scripts/smoke.sh` pass.

---

## Milestone Overview

| Phase | Name | Deliverable | Acceptance | Commit |
|-------|------|-------------|------------|--------|
| P0 | Spec and plan | `docs/SPEC.md`, `docs/PLAN.md` | The spec covers every requirement and is traceable | `📝 docs: add spec and phased implementation plan` |
| P1 | Project skeleton and data layer | Cargo project, config, 14-table migrations, db modules, error handling, health, Dockerfile, compose | `cargo run` starts, `/health` returns 200, migrations succeed | `🎉 init: bootstrap uwurls project skeleton` |
| P2 | Authentication and sessions | Register/login/logout, Argon2, sessions, CSRF, rate limiting, admin bootstrap | Integration tests cover register/login, 401 when unauthorized, CSRF 403 | `✨ feat(auth): ...` |
| P3 | Short link core | Slug service, URL CRUD, tags, expiry/click-cap/password columns, quota | Integration tests cover CRUD, ownership isolation, slug validation | `✨ feat(urls): ...` |
| P4 | Redirect and unlock | Redirect handler, click recording, UA parsing, unlock cookie, 410/404 pages | Integration tests cover redirects, the password flow, expiry 410 | `✨ feat(redirect): ...` |
| P5 | Web UI | Askama templates + CSS/JS, dashboard, link detail, account page | Pages verified manually and by smoke tests | `✨ feat(ui): ...` |
| P6 | Admin panel and API | User management, invite codes, settings, audit, API tokens, complete `/api/v1` | Integration tests cover admin authorization and tokens | `✨ feat(admin): ...` |
| P7 | Tests and documentation | Completed integration tests, `scripts/smoke.sh`, README, API.md, DEPLOYMENT.md | `cargo test` all green, smoke tests all pass | `🧪 test: ...` / `📝 docs: ...` |
| P8 | Final verification and wrap-up | clippy/fmt clean, release build, compose config validation, image strategy confirmed | Every acceptance item checked | `🔖 chore: finalize v1.0.0` |
| P9 | i18n, bot protection and navigation homepage | Three embedded locale catalogs, language negotiation and switching, optional Turnstile/reCAPTCHA, landing page at `/` | Negotiation, cookie switching and captcha fail-closed covered by tests; `/` returns 200 | `📝 docs: ...` / `✨ feat(i18n): ...` |

---

## P1 — Project Skeleton and Data Layer

**Tasks**

1. `Cargo.toml`: axum 0.8, tokio, sqlx (sqlite, runtime-tokio, migrate), askama, argon2, sha2, hmac, time, serde, tracing, tower-http, rand/getrandom, base64, subtle, thiserror, anyhow, uuid? (not needed)
2. `[profile.release]`: `lto=true`, `codegen-units=1`, `strip=true`, `opt-level="z"`, `panic="abort"`
3. `src/config.rs`: environment variable parsing + defaults + validation (BASE_URL, DB path, auto-generated secret)
4. `migrations/0001_init.sql`: all 14 tables from §6.3 + indexes + seed settings
5. `src/db/`: pool setup (WAL/pragmas), `migrate()`, CRUD module skeletons for every table
6. `src/error.rs`: the `AppError` enum + `IntoResponse` (HTML and JSON modes)
7. `src/server.rs` + `src/main.rs`: router, `/health`, `/assets/*`, graceful shutdown
8. `Dockerfile`, `docker-compose.yml`, `docker-compose.tls.yml`, `deploy/Caddyfile`, `.env.example`, `.gitignore`
9. `rustfmt.toml`, clippy configuration

**Acceptance**: `cargo build` succeeds; after startup `curl /health` returns 200 JSON; the DB file is created automatically with all 14 tables present.

---

## P2 — Authentication and Sessions

**Tasks**

1. `services/password.rs`: Argon2id hashing/verification + constant-time comparison
2. `services/secret.rs`: HMAC-SHA256 helpers (IP hashing, unlock cookie signatures)
3. `services/ratelimit.rs`: in-memory token buckets (key → bucket, periodically pruned)
4. `db/users.rs`, `db/sessions.rs`, `db/login_attempts.rs`
5. `services/auth.rs`: registration (mode checks, invite codes), login, logout, password change, session creation/validation/revocation
6. `middleware/auth.rs`: the `CurrentUser` extractor (session cookie or Bearer token)
7. `middleware/csrf.rs`: check `Origin`/`Referer` on non-GET requests that use cookie authentication
8. `middleware/security.rs`: security headers; `middleware/request_id.rs`
9. `routes/api_auth.rs`: the §7.1 endpoints
10. The first user becomes admin automatically; `UWURLS_ADMIN_EMAIL/PASSWORD` bootstrap

**Acceptance**: integration tests — register/login/logout/`/me`/change password, 401 when not logged in, 403 for cross-origin POST, and 429 after repeated failed logins.

---

## P3 — Short Link Core

**Tasks**

1. `services/slug.rs`: base62 generation, validation (length/characters/reserved words), collision retries
2. `services/url_target.rs`: target URL normalization and scheme allowlist (open redirect protection)
3. `db/urls.rs`, `db/tags.rs`, `db/domains.rs`: CRUD, list queries (search/tag/status/sort/pagination), quota checks
4. `services/urls.rs`: create/update/delete business logic (tag upserts, expiry and cap validation, password hashing)
5. `routes/api_urls.rs`: the §7.2 endpoints (`/stats` and `/clicks` return structured data first)
6. `routes/api_tags.rs`: the §7.3 tag endpoints
7. Audit log writes (`services/audit.rs`)

**Acceptance**: integration tests — CRUD, custom/automatic slugs, reserved-word rejection, invalid target rejection, 404 for another user's link, quota limits, tag filtering.

---

## P4 — Redirect and Unlock

**Tasks**

1. `routes/redirect.rs`: the `GET /{slug}` state machine (missing 404 / deleted 404 / disabled 404 / expired 410 / password required 200 password page / unlocked 302)
2. `services/unlock.rs`: issuing and verifying unlock cookies (HMAC + slug binding + TTL)
3. `routes/redirect.rs`: `POST /{slug}/unlock` (form + JSON), rate limiting
4. `services/ua.rs`: lightweight UA parsing (device/browser/os/bot)
5. `services/click.rs`: asynchronous writes to `clicks` + `url_daily_stats` upsert + `click_count` increment + unique visitor detection
6. Expiry/404/password page templates (Askama, minimal)
7. `middleware/security.rs`: cache headers for redirect responses

**Acceptance**: integration tests — normal 302 redirect, custom 301/307, password page does not leak the target, wrong password 401, correct password redirects, expired 410, click cap reached 410, deleted/disabled 404, and click counts plus rollups increment correctly.

---

## P5 — Web UI

**Tasks**

1. `templates/base.html` + `assets/app.css` (responsive, dark mode) + `assets/app.js` (copy, confirm, form enhancements)
2. `templates/login.html`, `register.html`
3. `templates/dashboard.html`: stat cards, SVG trend chart, link list (search/tag/status/pagination), create form (with advanced options: password, expiry, click cap, tags)
4. `templates/url_detail.html`: edit, analytics charts, recent clicks, delete
5. `templates/account.html`: profile, change password, session list, API token management
6. `routes/pages_*.rs`: `/app/**` routes + form POST handlers (HTML redirects + flash messages)
7. SVG chart generator (`services/chart.rs`)

**Acceptance**: manually/scripted against the real binary — register → create (with password and expiry) → copy the short link → see analytics on the detail page → create a token on the account page.

---

## P6 — Admin Panel and Complete API

**Tasks**

1. `middleware/admin.rs`: admin authorization
2. `routes/api_admin.rs`: all §7.4 endpoints (users, site-wide links, invite codes, settings, audit, system info)
3. `routes/api_tokens.rs`: the §7.3 token endpoints
4. `routes/api_public.rs`: the public metadata endpoint
5. `templates/admin/*.html` + `/app/admin/**` pages
6. `db/settings.rs`: settings reads/writes + cache (`RwLock` in-memory cache invalidated on write)
7. Daily maintenance task (purge expired sessions, prune clicks by retention)

**Acceptance**: integration tests — non-admin access 403, admin can suspend a user (their sessions become invalid immediately), the invite-code registration flow, settings updates take effect, the audit log records entries, and API token authentication succeeds.

---

## P7 — Tests and Documentation

**Tasks**

1. `tests/common/mod.rs`: test app factory (temporary SQLite, in-memory settings)
2. `tests/integration.rs`: every integration test case from §10
3. Fill in unit tests (slug, UA, expiry, settings, CSRF, password, chart)
4. `scripts/smoke.sh`: real binary + curl end-to-end (including 410, unlock, ownership isolation)
5. `README.md` (quick start, deployment, backup, upgrade), `docs/API.md`, `docs/DEPLOYMENT.md`
6. `LICENSE` (MIT)

**Acceptance**: `cargo test` all green, `scripts/smoke.sh` all pass.

---

## P8 — Final Verification and Wrap-up

**Tasks**

1. `cargo fmt`, `cargo clippy --all-targets -- -D warnings` clean
2. `cargo build --release` and measure the binary size (target ≤ 15 MB)
3. `docker compose config` / YAML validation; confirm the Dockerfile logic is correct (when no local Docker engine is available, use YAML parsing and state this explicitly)
4. Run the smoke test and health check once against the release binary
5. Update the checkboxes in `docs/PLAN.md` and the README acceptance list

**Acceptance**: every item in the `docs/SPEC.md` §11 acceptance list is checked.

---

## P9 — i18n, Bot Protection and Navigation Homepage (v1.1)

**Tasks**

1. Locale catalogs `src/i18n/{en,zh-Hant,zh-Hans}.json` and `services/i18n.rs`: load the catalogs with `include_str!`, negotiate from `Accept-Language` honoring `q=` weights with **English** as the fallback, and fall back to the English string for a missing key.
2. `middleware/lang.rs`: resolve the active language from the `uwurls_lang` cookie or `Accept-Language`, publish it through a task-local context for handlers and error pages, and render `<html lang>` from it.
3. `GET /app/lang/{code}?next=<path>`: set the cookie (`Path=/`, `SameSite=Lax`, `Max-Age` one year, not HttpOnly) and return 302; only same-site relative paths are accepted for `next`, and anything else falls back to `/`.
4. `services/captcha.rs`: a provider abstraction for Cloudflare Turnstile and Google reCAPTCHA v2, settings values overridden by non-empty environment variables, a 5-second HTTPS POST to the provider siteverify endpoint, and a fail-closed result (timeout, non-2xx, or unparsable response rejects the request).
5. Enforce captcha on the web registration form, the web create-short-link form, and `POST /api/v1/auth/register` (JSON field `captcha_token`, 400 `captcha_failed`); skip it for `POST /api/v1/urls`, which authenticates with an API token.
6. Admin settings: the five new keys `captcha_provider`, `captcha_site_key`, `captcha_secret_key`, `captcha_on_register`, `captcha_on_create`, with a write-only secret key field (an empty submission keeps the current value).
7. `GET /` landing page: hero, feature highlights, login and register entry points, language switcher, and footer, plus a dashboard entry point for signed-in users; remove the 302 redirect to `/app`.
8. Tests: language negotiation (`q=` weights, fallback to English), cookie switching and `next` validation, API JSON staying English, the captcha enablement matrix, captcha fail-closed behavior, and `GET /` returning 200.
9. Docs: update `docs/SPEC.md` and the Chinese mirror `docs/zh-TW/SPEC.md` (FR tables, route list, environment variables, settings keys), and both `PLAN.md` files.

**Acceptance**: `GET /` returns 200 for anonymous and signed-in visitors and no longer redirects; `Accept-Language` negotiation and the language cookie switch the UI, while JSON API responses stay English; captcha is enabled only when provider ≠ `none`, both keys are non-empty, and the matching switch is true, and it fails closed on timeout/non-2xx/parse failure; captcha does not apply to API-token authenticated endpoints; the admin secret key is never rendered back; `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` are clean.

---

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| No local Docker engine (Docker Desktop WSL integration disabled) | Cannot actually run `docker build`/`compose up` | Validate compose with YAML parsing, verify behavior with the release binary, and follow the standard multi-stage Dockerfile pattern while documenting that it was not verified on real hardware |
| musl static build fails | The image cannot run | Keep a `debian-slim` runtime as a fallback (documented) |
| SQLite write lock contention | Latency on high-concurrency click writes | WAL + `busy_timeout` + asynchronous writes + merged writes through rollup upserts |
| Askama templates drift from routes | Runtime 500s | Templates are checked at compile time; integration tests cover a GET of every page |
| Dependency version drift | Build failures | Commit `Cargo.lock`; pin the toolchain image version in the build stage |

---

## P8 Acceptance Measurements

| Item | Target (`docs/SPEC.md` §3.3, §5.2) | Measured |
|------|-------------------------------------|----------|
| Release binary size | ≤ 15 MB | **4.2 MB** (stripped, LTO, `opt-level=z`) |
| Idle memory RSS | ≤ 30 MB | **6.8 MB** (6.8 MB after 500 requests) |
| Redirect latency (P95) | < 10 ms | **0.74 ms** (p50 0.40 ms, p99 1.05 ms, 500 keep-alive requests locally) |
| Cold start to serving | ≤ 1 second | **~0.2 s** (including migrations) |
| Container image | ≤ 40 MB | **13.2 MB** (built and `up -d` verified on real hardware with Docker Compose 27.4) |
| Tests | All green | `cargo test` 187 items, `scripts/smoke.sh` 81 items |
| Static checks | No warnings | `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass |
