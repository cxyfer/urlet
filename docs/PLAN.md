**English** | [繁體中文](zh-TW/PLAN.md)

# urlet — Phased Implementation Plan

> Spec: `docs/SPEC.md` v1.7
> Principle: at the end of every phase the code must **compile, test, and commit**; each phase gets one (or a few) local commits.

## Results (completed)

| Phase | Status | Commit |
|-------|--------|--------|
| P0 Spec and plan | ✅ Done | `4109fc8 📝 docs: add spec and phased implementation plan` |
| P1 Project skeleton and data layer | ✅ Done | `06f91a7 🎉 init: bootstrap urlet project skeleton` |
| P2 Authentication and sessions | ✅ Done | `e0d8902 ✨ feat(auth): add sessions, registration, login and CSRF protection` |
| P3 Short link core | ✅ Done | `e1db4fc ✨ feat(urls): add short link CRUD, slugs, tags and expiry rules` |
| P4 Redirect and unlock | ✅ Done | `2a8ca77 ✨ feat(redirect): add redirects, password unlock and click analytics` |
| P5 Web UI | ✅ Done | `eae0e0b ✨ feat(ui): add server rendered dashboard, detail and account pages` |
| P6 Admin panel and API | ✅ Done | `c74302a ✨ feat(admin): add admin panel, invites, settings and maintenance` |
| P7 Tests and documentation | ✅ Done | See `git log` (smoke.sh, README, API.md, DEPLOYMENT.md) |
| P8 Final verification | ✅ Done | See `git log` (release build, image strategy, wrap-up) |
| P10 User groups (v1.2) | ✅ Done | See `git log` (migration 0004, group API/UI, trilingual strings, docs) |
| P11 Group domain policy (v1.2) | ✅ Done | See `git log` (migration 0005, default domain + allowlist, admin UI/API, tests) |
| P12 Switchable themes (v1.4) | ✅ Done | See `git log` (migration 0010, theme registry, admin catalogue, six template sets, tests) |
| P13 Splash pages and redirect-mode policy (v1.5) | ✅ Done | See `git log` (migration 0011, splash templates for six themes, per-group policy, trilingual strings, docs, tests) |
| P14 Uploadable themes (v1.5) | ✅ Done | See `git log` (migration 0012, CSS upload/edit/delete, hashed assets, persistent theme volume, tests) |
| P15 Public shortener (v1.6) | ✅ Done | See `git log` (migration 0015, reserved Guest account, access level, field switches, two visitor budgets, landing form, public API, trilingual strings, docs, tests) |

Acceptance data (v1.0): `cargo test` is all green with 187 tests, `cargo clippy --all-targets -- -D warnings`
and `cargo fmt --check` report no warnings, and all 81 end-to-end checks in `scripts/smoke.sh` pass.

---

## Milestone Overview

| Phase | Name | Deliverable | Acceptance | Commit |
|-------|------|-------------|------------|--------|
| P0 | Spec and plan | `docs/SPEC.md`, `docs/PLAN.md` | The spec covers every requirement and is traceable | `📝 docs: add spec and phased implementation plan` |
| P1 | Project skeleton and data layer | Cargo project, config, 14-table migrations, db modules, error handling, health, Dockerfile, compose | `cargo run` starts, `/health` returns 200, migrations succeed | `🎉 init: bootstrap urlet project skeleton` |
| P2 | Authentication and sessions | Register/login/logout, Argon2, sessions, CSRF, rate limiting, admin bootstrap | Integration tests cover register/login, 401 when unauthorized, CSRF 403 | `✨ feat(auth): ...` |
| P3 | Short link core | Slug service, URL CRUD, tags, expiry/click-cap/password columns, quota | Integration tests cover CRUD, ownership isolation, slug validation | `✨ feat(urls): ...` |
| P4 | Redirect and unlock | Redirect handler, click recording, UA parsing, unlock cookie, 410/404 pages | Integration tests cover redirects, the password flow, expiry 410 | `✨ feat(redirect): ...` |
| P5 | Web UI | Askama templates + CSS/JS, dashboard, link detail, account page | Pages verified manually and by smoke tests | `✨ feat(ui): ...` |
| P6 | Admin panel and API | User management, invite codes, settings, audit, API tokens, complete `/api/v1` | Integration tests cover admin authorization and tokens | `✨ feat(admin): ...` |
| P7 | Tests and documentation | Completed integration tests, `scripts/smoke.sh`, README, API.md, DEPLOYMENT.md | `cargo test` all green, smoke tests all pass | `🧪 test: ...` / `📝 docs: ...` |
| P8 | Final verification and wrap-up | clippy/fmt clean, release build, compose config validation, image strategy confirmed | Every acceptance item checked | `🔖 chore: finalize v1.0.0` |
| P9 | i18n, bot protection and navigation homepage | Three embedded locale catalogs, language negotiation and switching, optional Turnstile/reCAPTCHA on register, sign-in and create, landing page at `/` | Negotiation, cookie switching, captcha fail-closed (including sign-in) and the landing page covered by tests; `/` returns 200 | `📝 docs: ...` / `✨ feat(i18n): ...` |
| P10 | User groups (v1.2) | `user_groups` with per-group link/token quotas and seven feature toggles (including domain selection), assignment through users and invite codes, admin page and API | 16 tables / 31 indexes, upgrade changes no behaviour, quotas/toggles enforced (admins included), deletion reassigns members, tests and smoke green | `✨ feat(groups): ...` |
| P11 | Group domain policy (v1.2) | Per-group `default_domain_id`, `restrict_domains` and `group_domains` allowlist; group default/allowlist enforced on create/move, exposed through `/api/v1/auth/me` and `/api/v1/domains`, editable in the admin group form | 16 tables / 31 indexes, existing groups keep every active domain, empty restricted list allows only the default, disabled default falls back to the site default, tests and smoke green | `✨ feat(domains): ...` |
| P12 | Switchable themes (v1.4) | Six compiled-in template/CSS themes, `themes` table and registry, admin enable/default/policy page and API, `urlet_theme` cookie, theme picker and shared fallback content | 18 tables / 34 indexes, every theme renders every page, permission and fallback covered, fmt/clippy/test/smoke green | `✨ feat(themes): ...` |
| P13 | Splash pages and redirect-mode policy (v1.5) | `urls.redirect_mode` and splash copy, six theme templates, `user_groups.redirect_mode_policy`, forms/API/`/auth/me`, locale catalogs | Existing links stay direct, password pages stay the only interstitial, mode policy enforced on create/update only, fmt/clippy/test/smoke green | `✨ feat(splash): ...` |
| P14 | Uploadable themes (v1.5) | `custom_themes` metadata, CSS upload/edit/delete from the admin theme page, content-hashed `/assets/custom-themes/{key}`, `auto`/`light`/`dark` appearance, validation and limits, `URLET_THEMES_DIR` + `urlet-themes` volume | 19 tables / 35 indexes, uploads survive a reload/restart, light/dark forcing, default/delete rules and member authorization covered, fmt/clippy/test/smoke green | `✨ feat(themes): ...` |
| P15 | Public shortener (v1.6) | Landing-page shortening form plus `POST /api/v1/public/urls`, the reserved Guest account that owns anonymous links, `public_create_access`, eight `public_allow_*` field switches, `public_captcha`, per-address and shared visitor budgets, admin settings section, trilingual strings, six landing templates | 19 tables / 36 indexes, an upgrade keeps creation closed to visitors, the access level gates every creation path, disabled fields are refused server side, budgets and the Guest account covered, fmt/clippy/test/smoke/merge-check green | `✨ feat(public): ...` |
| P16 | API reference and contract (v1.6) | `openapi/` contract (shared components plus one file per language) merged by `scripts/build-openapi.py` into two embedded specs; Scalar vendored by `scripts/update-scalar.sh`; `/docs`, `/docs/openapi.json` and `/docs/openapi.zh-TW.json` served by the binary; `docs` reserved with a startup report for a shadowed link; footer entry point in every design and locale | Both contracts answer over real HTTP with no third-party request; the two documents describe the same operations; the generator is deterministic and CI-guarded; no CSP change; image stays under 40 MB; every theme renders the link in three locales; fmt/clippy/test/smoke green | `✨ feat(docs): ...` |
| P17 | Published documentation site (v1.6) | `docs-site/` built from the repository's `docs/` tree by a prebuild sync, in both languages, with search; deployed to `gh-pages` by a path-filtered workflow; excluded from the Docker context | The export contains both locales and every published page; the sync derives titles and rewrites inter-document links; `content/docs/` is gitignored so the repository keeps one copy; the site makes no third-party request; the Rust build, image and binary are untouched; fmt/clippy/test/smoke green | `📝 docs(site): ...` |

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
10. The first user becomes admin automatically; `URLET_ADMIN_EMAIL/PASSWORD` bootstrap

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

1. Locale catalogs `src/i18n/{en,zh-Hant,zh-Hans}.json` loaded by `src/i18n/mod.rs`: load the catalogs with `include_str!`, negotiate from `Accept-Language` honoring `q=` weights with **English** as the fallback, and fall back to the English string for a missing key.
2. `src/middleware/locale.rs`: resolve the active language from the `urlet_lang` cookie or `Accept-Language` (the cookie wins), publish it through a task-local context for handlers and error pages, render `<html lang>` from it, and add `Content-Language` plus `Vary: Accept-Language, Cookie` to HTML responses.
3. `GET /app/lang/{code}?next=<path>`: set the cookie (`Path=/`, `SameSite=Lax`, `Max-Age` one year, not HttpOnly, `Secure` when `URLET_SECURE_COOKIES=true`) and return 302; only same-site relative paths are accepted for `next`, and anything else (including a missing `next`) falls back to `/app`; an unknown `code` is treated as `en`.
4. `src/services/captcha.rs`: a provider abstraction for Cloudflare Turnstile and Google reCAPTCHA v2, settings values overridden by non-empty environment variables, a form-encoded 5-second HTTPS POST through `ureq` 2 (rustls + webpki-roots, no OpenSSL) to the provider siteverify endpoint, a check that the reported `hostname` is the base-URL host or on the allow-list (a missing `hostname` passes, a mismatch is rejected and logged at `WARN`), and a fail-closed result (timeout, non-2xx, or unparsable response rejects the request). Tests live in `tests/captcha.rs`.
5. Enforce captcha on the web registration, sign-in and create-short-link forms, and on `POST /api/v1/auth/register` and `POST /api/v1/auth/login` (JSON field `captcha_token`, 400 `captcha_failed`); on `POST /api/v1/urls` enforce it for session-cookie calls only and exempt `Authorization: Bearer ur_...` calls, so automation keeps working.
6. Admin settings: the seven new keys `captcha_provider`, `captcha_site_key`, `captcha_secret_key`, `captcha_on_register`, `captcha_on_login`, `captcha_on_create`, `captcha_allowed_hostnames`, each with a matching `URLET_CAPTCHA_*` environment variable (including `URLET_CAPTCHA_ON_LOGIN` and `URLET_CAPTCHA_ALLOWED_HOSTNAMES`) that overrides the stored value when non-empty; the secret key field is write-only (an empty submission keeps the current value).
7. `GET /` landing page (`src/routes/pages/home.rs` + `templates/landing.html`): hero, feature highlights, login and register entry points, language switcher, and footer, plus a dashboard entry point for signed-in users; remove the 302 redirect to `/app`.
8. Tests: language negotiation (`q=` weights, fallback to English), cookie switching and `next` validation, API JSON staying English, the captcha enablement matrix, captcha fail-closed behavior on register, sign-in and create, and `GET /` returning 200.
9. Docs: update `docs/SPEC.md` and the Chinese mirror `docs/zh-TW/SPEC.md` (FR tables, route list, environment variables, settings keys), both `PLAN.md` files, both READMEs, `docs/API.md`, `docs/DEPLOYMENT.md`, their `docs/zh-TW/` mirrors and `.env.example`.

**Acceptance**: `GET /` returns 200 for anonymous and signed-in visitors and no longer redirects; `Accept-Language` negotiation and the language cookie switch the UI, while JSON API responses stay English; captcha is enabled only when provider ≠ `none`, both keys are non-empty, and the matching switch (`on_register`, `on_login` or `on_create`) is true, and it fails closed on timeout/non-2xx/parse failure on register, sign-in and create; on `POST /api/v1/urls` captcha applies to session-cookie calls and `Authorization: Bearer ur_...` calls stay exempt; the admin secret key is never rendered back; `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` are clean.

---

## P10 — User Groups (v1.2)

**Tasks**

1. `migrations/0004_user_groups.sql`: `user_groups` (nullable link/token quotas, seven toggles including `allow_domain_selection`, `is_default`), `users.group_id`, `invite_codes.group_id`, the partial unique default index, and a seeded `Standard` group that every existing user joins; seed `max_api_tokens_per_user=20`.
2. `src/db/user_groups.rs` + `src/services/limits.rs`: group CRUD helpers, member counts, reassignment delete, and the effective-limits resolver (`NULL` inherits, `0` is unlimited, toggles can only narrow the site defaults).
3. `src/services/groups.rs` + `AdminContext`: validation, duplicate/default guards, audited create/update/delete, default-marker transfer, and user/invite assignment.
4. Enforcement: link quota and the seven toggles on create/update, token toggle and usable-token quota, and the invite-code group at registration (validated in open mode too).
5. API: `/api/v1/admin/groups` CRUD, `group_id` on users and invites, and `group`/`limits` in `GET /api/v1/auth/me`.
6. UI: `/app/admin/groups`, group filter and assignment on the users page, invite group select, token quota setting, group-aware dashboard/detail/account pages, and trilingual strings.
7. Tests: unit coverage for limit inheritance and group validation, plus `tests/groups.rs` for authorization, CRUD, quotas, toggles, invites, deletion, default transfer and page rendering.
8. Docs: SPEC/PLAN/API/README in both languages, and a smoke section for groups.

**Acceptance**: 16 tables and 31 indexes; the seeded default group changes no behaviour; quotas and toggles are enforced per group (administrators included); deletion requires a reassignment target; `/me` exposes the effective limits; `cargo test`, clippy, fmt and smoke are clean.

---

## P11 — Group Domain Policy (v1.2 follow-up)

**Tasks**

1. `migrations/0005_group_domains.sql`: `user_groups.default_domain_id` (nullable, `ON DELETE SET NULL`), `user_groups.restrict_domains`, `group_domains(group_id, domain_id)` and its domain index.
2. `services/domain_policy.rs`: resolve the effective default (group default when active, otherwise the site default), the allowlist and the `allow_domain_selection` rule; expose filtered `DomainOption`s.
3. Link service: create without `domain` uses the group default; create/move must pass `allows()`; keeping the current domain always succeeds.
4. API/UI: group payload/DTO fields, `/api/v1/auth/me` `default_domain` + `allowed_domains`, `/api/v1/domains` filtered, admin group form selects/checkboxes.
5. Tests: migration from v4, empty allowlist, explicit allowlist, disabled default fallback, selection off, partial updates, HTML form, API/UI and smoke.

**Acceptance**: an existing group keeps every active domain; a restricted group can only create/move to its allowlist plus its default; an empty restricted list allows only the default; disabling/deleting the group default falls back to the site default; `cargo test`, clippy/fmt and smoke are clean.

## P12 — Switchable Themes (v1.4)

**Tasks**

1. `migrations/0010_themes.sql`: `themes` table, partial unique default index, the six seed rows and the `theme_switching=all` setting.
2. `src/db/themes.rs` + `src/services/theme.rs`: theme keys, registry cache, default/fallback resolution, admin list, switching permission and transactional selection updates.
3. `src/routes/pages/themed.rs`: compile-time `themed_page!` wrapper so each page selects one of six Askama templates without trusting a request path; page contexts opt in with `.themed()`.
4. Middleware and assets: `urlet_theme` cookie resolution, `GET /app/theme/{key}`, `/assets/themes/{key}`, `/assets/theme-ui.css`, and the shared picker macro.
5. Admin: `/app/admin/themes` page, `GET/PUT /api/v1/admin/themes`, audit action, validation (at least one enabled, default enabled), and cache reload.
6. Themes: import the five closed design proposals (Aurora, Paper, Signal, Gilded, Swiss) as template directories and CSS assets; share v1.2 app-page content through macros so quotas, domains, groups, invites and settings keep working in every theme.
7. Tests: registry defaults, admin CRUD/policy, switch permission, disabled/unknown fallback, asset routes, and a full render loop over every theme and page.
8. Docs: SPEC, PLAN, API, README and the `zh-TW` mirrors; `.env.example` needs no new variable because themes are database-only.

**Acceptance**: six themes are selectable, the default applies without a cookie, disabled themes fall back, administrators can preview any compiled theme, including a disabled one, `all`/`admin` switching is enforced, every page renders under every theme, and `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` and `scripts/smoke.sh` are clean.

---

## P14 — Uploadable Themes (v1.5)

Uploads extend the theme catalogue without changing the compile-time template model.

1. `migrations/0012_custom_themes.sql` + `0013_custom_theme_appearance.sql`: metadata table for
   uploaded stylesheets, keyed by a generated `custom-…` key; a matching `themes` row carries
   enabled/default/order, and the appearance migration converts the old dark-only flag.
2. `src/db/custom_themes.rs`: create/update/delete metadata and the catalogue row in one transaction.
3. `src/services/custom_theme.rs`: CSS validation (UTF-8, size, no external resources), the
   `auto`/`light`/`dark` appearance, atomic file writes, content hashes and the settings-lock
   serialization shared with `update_selection`.
4. `src/services/theme.rs`: one dynamic `ThemeDef` for compiled-in and uploaded themes; the picker,
   default resolution, preview and admin rows no longer assume a closed `ThemeKey` set.
5. Routes and templates: multipart upload from `/app/admin/themes`, an edit page, delete, audit
   actions and the `custom` flag in the catalogue API.
6. Assets: `/assets/custom-themes/{key}` reads the current database row and serves
   `URLET_THEMES_DIR/<key>-<hash>.css` with a one-hour cache; replacing the CSS changes the URL.
7. Persistence: `URLET_THEMES_DIR` defaults to `<data dir>/themes`, Docker creates `/data/themes` and
   compose mounts the dedicated `urlet-themes` volume.
8. Tests and docs: upload/manage/delete, validation, authorization, persisted reload, audit entries,
   both language mirrors and the Docker/compose/backup notes.

**Acceptance**: an administrator can upload a CSS theme, select its `auto`/`light`/`dark` appearance,
make it the default, replace its stylesheet, and delete it after moving the default; members receive
403 for every upload route; a reload rebuilds the catalogue from SQLite; forced light/dark modes
override a saved browser preference; the CSS file exists in `URLET_THEMES_DIR`; and
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` and
`scripts/smoke.sh` are clean.

---

## P15 — Public Shortener (v1.6)

**Tasks**

1. `migrations/0015_public_shorten.sql`: `users.is_system` plus the partial unique index, the reserved Guest account (`guest@urlet.invalid`, suspended, unparsable hash, no group) and the twelve public-shortener settings
2. `src/db/users.rs`: the `is_system` column, `find_system`, and `is_system = 0` on every listing, count and administrative write
3. `src/services/settings.rs`: `PublicCreateAccess` (`guest`/`user`/`admin`), `PublicFields`, the `public_*` accessors and validation
4. `src/services/public_urls.rs`: the access check, the server-side field enforcement, the per-address and shared budgets, and the create flow that resolves the Guest actor
5. `src/services/urls.rs`: the access level also gates the dashboard and the authenticated API
6. `src/routes/public_urls.rs` + `src/routes/pages/home.rs`: `POST /`, `POST /api/v1/public/urls`, the form/result view, and the `?shortened=` lookup that never trusts the query string
7. `templates/partials/shorten.html` plus an include in all six landing templates; a dashboard notice when creation is reserved for administrators
8. Admin settings section, three locale catalogues, `tests/public_urls.rs`, `scripts/smoke.sh` and both doc languages
9. Guest quota follow-up: migration `0017_public_creation_quota.sql` persists atomic natural-period counters; independent minute/hour/day/week/month settings follow the server timezone, survive restart and keep authenticated callers on group quotas and their existing account throttle. Reveal guest settings only for guest access, preserve hidden values, and test timezone/DST/calendar boundaries.

**Acceptance**: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`,
`scripts/smoke.sh` and `scripts/merge-check.sh` are green; an upgrade leaves the public form closed
until an administrator opens it.

---

## P16 — Tracking parameter cleanup

Approved scope (2026-10-23): optional cleanup on **creation only**, disabled by default. Cover Facebook, Instagram, Threads, X/Twitter, YouTube, TikTok and Bilibili, plus administrator-defined domain/parameter rules. Existing links and target edits retain their behavior. This implements the approved feature request; configuration alone cannot filter submitted targets.

**Owners and boundaries:** `src/services/url_cleaning.rs` owns bounded rule validation and pure query filtering; `UrlContext::create` remains the single owner of creation for authenticated/public API and browser forms. Reuse the settings table/cache, atomic settings updates, admin authorization/audit and shared themed settings template. No new dependencies, tables, network calls, redirect-time cleanup or URL expansion. `SPEC.md` is the behavior baseline; `API.md` documents the settings contract.

**TDD Route:** off / skipped (local Aegis configuration; no strict TDD request). Add behavior-focused regression tests after implementation. The clean worktree baseline has 530 passing tests, 0 failures and 2 ignored tests.

1. Add a pure cleaner with conservative built-in rules maintained in `src/rules/url-cleaning.txt`, embedded with `include_str!` and shared by the cleaner and administrator UI. Use the same format as custom rules; validate the list with the existing parser and behavior tests. Each custom line is `domain parameter [parameter ...]`, with domain and subdomain matching and optional trailing `*` parameter prefixes; names match ASCII case-insensitively. Limit custom input to 8192 bytes, 100 rules, 32 parameters per rule, and 64 bytes per parameter. Reject schemes, paths, ports, wildcard domains and invalid patterns. Preserve kept query components byte-for-byte, their order, repeated keys, path and fragment; decode parameter names only for matching.
2. Add `url_cleaning_enabled=false` and `url_cleaning_rules=` through existing defaults, editable keys and validation. Read both settings under one cache lock when creating a link. Validate the original target before cleanup. Invalid setting batches must leave stored values/cache unchanged. Startup defaults seed absent keys without a migration or overwriting existing settings.
3. Extend the shared settings page with an enable switch, custom-rule textarea, built-in rule disclosure, limits and examples in all three locales. Keep invalid submitted cleanup rules visible with a localized error and no writes. The same controls must render across all six themes; clearing rules and disabling cleanup must work.
4. Cover built-in providers, unknown/spoofed hosts, credentials/ports/subdomains, percent-encoded parameter names, duplicate/blank values, preserved encoding/fragments, rule validation and idempotence. Exercise admin-only settings, persistence after reload, atomic rejection, authenticated/public APIs, both browser creation forms, returned/stored targets and actual redirect location. Verify existing links and edits stay unchanged.
5. Update English and Traditional Chinese SPEC/API/PLAN. Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `bash scripts/smoke.sh`, `bash scripts/merge-check.sh`, using an isolated worktree target directory. Inspect the final diff, commit with the repository's Gitmoji style, push `feat/url-cleaning` and open a PR against `cxyfer/urlet:master`. Start an independent reviewer subagent alongside PR creation; address actionable findings and update/recheck the PR before reporting completion. Do not merge.

**Plan self-review:** scope and acceptance match the user's approved design; all four create entry points converge on the existing service, admin settings remain the sole policy owner, no persistent data is rewritten, and the existing SPEC/API docs carry the changed contract. New code is necessary only for filtering, validation and the settings UI. Main risks are removing meaningful parameters or re-encoding untouched components; conservative exact/prefix rules and byte-preservation regression tests cover those boundaries. Built-in rules remain deliberately limited and custom rules can remove meaningful parameters if an administrator chooses them; explain this in the UI. No old path needs retirement.

---

## P17 — Editable Guest Destination Allowlist

**Goal**: anonymous visitors may shorten only explicitly allowed destinations; administrators can edit the built-in list without restarting. Scope: local allowlist only, with no reputation API, blocklists, group overrides or redirect-time enforcement.

**Architecture / baseline**: reuse SQLite settings and the atomic, audited admin API/form (`docs/SPEC.md` §4.16). Add one focused matcher shared by public HTML/JSON creation. The existing domain policy governs short-link hosts and cannot restrict destinations, so source changes are necessary. Parse with `url::Url`, already in the dependency graph. No schema change is needed; seed missing settings through the existing defaults owner.

**Compatibility**: default `guest_allowlist_enabled=true`; never overwrite edited or empty lists. Empty enabled lists deny all guest targets. Enforce before captcha and the shared guest budget. Signed-in callers and existing redirects keep their behavior. Old settings forms that omit the new controls preserve them. Keep matching logic out of the large admin/settings handlers; those receive wiring only.

**Tasks**

1. Add `src/services/guest_allowlist.rs` and a compiled-in list of established service hosts without path restrictions; exclude separate public website hosting domains. Support exact ASCII DNS hosts or a leading `*.` for all subdomain levels (excluding the root), plus optional case-sensitive path prefixes with segment boundaries. Refuse credentials and nonstandard ports. Bound and validate edits; handle dot segments and reject ambiguous encoded separators for scoped paths. Unit tests cover spoofed hosts, paths, empty lists and defaults.
2. Wire `guest_allowlist_enabled` / `guest_allowlist` into `src/db/settings.rs`, `src/services/settings.rs`, and `src/services/public_urls.rs`. Test anonymous form/API enforcement, immediate edits, disabled/empty states, signed-in session/token exemption, and persistence after settings reload.
3. Extend the existing admin settings view/form and shared landing partial. Add all three locales, a specific refusal message and a guest-visible list of allowed destinations. Test authorization, atomic invalid edits, old forms and every compiled-in theme.
4. Update the spec, API docs, this plan and their Traditional Chinese mirrors; extend `scripts/smoke.sh` with default enforcement and editable rules over real HTTP.
5. Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `bash scripts/smoke.sh`, `bash scripts/merge-check.sh`; review, commit, push `feat/guest-allowlist` and open a PR against `cxyfer/urlet:master` without merging. No strict TDD route is requested; commit the completed business change after verification.

**Wildcard follow-up (approved 2026-10-23)**: extend the existing matcher in place; configuration-only changes cannot work while the parser rejects `*`. Accept only a leading `*.` before a valid DNS hostname; match at a label boundary and require a nonempty subdomain, preserving exact-host behavior and optional path restrictions. Normalize without dropping the wildcard. Replace the five YouTube rules with `*.youtube.com` and `youtube.com`; consolidate Wikipedia language entries with `*.wikipedia.org` and list `wikipedia.org` separately. Preserve stored edits and empty lists; only missing settings receive the revised defaults. TDD mode is off; add regression coverage after implementation. Update three UI locales, API/spec mirrors and existing smoke checks. Verify admin API/form saving, reload persistence, root exclusion, nested subdomains, suffix spoofing, malformed stars and scoped paths, then run all required checks and update PR #34. No new owner, migration, dependency or setting is needed.

**Built-in policy expansion (approved 2026-10-23)**: expand the default list to 24 services / 43 host rules, covering knowledge/development, Taiwan discussion, video/music, social, news and product support. Use established service hosts without path restrictions; do not include separate public website hosting domains such as `*.github.io`, `*.gitlab.io` or `sites.google.com`. Reuse the matcher and settings owners unchanged. Update the existing policy regression cases, real-router defaults and smoke checks, plus the bilingual API/spec/plan. Verify broadened and newly added destinations, spoofed hosts, excluded hosting domains and saved-list preservation; run the required checks and update PR #34. This bounded defaults refinement needs no new owner, migration, dependency or runtime reputation lookup. TDD remains off; update existing tests for changed behavior.

**Risks / retirement**: known sites can host user content and redirects; the built-in list is an editable policy, not a safety certification. Do not fetch targets. Preserve the existing settings/rate-limit owners; no legacy path is retired. Verify real router paths and settings reload, beyond isolated matcher tests.

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

The table above is the v1.0 record; the current v1.1 numbers (5.7 MB glibc / 5.8 MB musl binary,
14.8 MB image, 8.7 MB RSS, ~0.1 s cold start, the full test suite) are tracked in `docs/SPEC.md` §3.3 — read
them there rather than quoting this table.

## SNS preview cards

Opt-in per-link title/description plus HTTPS or uploaded raster image, a four-mode administrator image policy, SQLite storage, owner-only form/API, narrow social crawler metadata output, lifecycle/privacy boundaries and tests. Implementation plan: [SNS preview plan](aegis/plans/2026-10-23-social-previews.md).
