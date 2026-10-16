# uwurls JSON API reference

**English** | [繁體中文](zh-TW/API.md)

- Base path: `/api/v1`
- Authentication (choose one)
  - **Session cookie**: `uwurls_session`, issued by `POST /api/v1/auth/login` (the browser form flow uses the same cookie)
  - **API token**: `Authorization: Bearer uw_...` (create one under `/app/account`)
- Requests and responses are `application/json; charset=utf-8`
- Timestamps are always ISO-8601 UTC (`2030-01-31T12:00:00.000Z`); input also accepts
  `2030-01-31T12:00:00Z`, `2030-01-31 12:00`, `2030-01-31`, and similar forms
- JSON responses are always English, whatever UI language the visitor selected
  (see [UI language](#ui-language))

## Error format

```json
{
  "error": {
    "code": "validation_error",
    "message": "slug may only contain letters, digits, '-' and '_'",
    "details": { "field": "slug" }
  }
}
```

| HTTP | `code` | Meaning |
|------|--------|---------|
| 400 | `bad_request` | Malformed request, or nothing to change |
| 400 | `captcha_failed` | Bot verification failed, expired, or was rejected by the provider (registration and sign-in) |
| 401 | `unauthorized` | Not authenticated, token revoked, wrong password |
| 403 | `forbidden` | Insufficient permission, CSRF rejection, registration closed, quota exhausted |
| 404 | `not_found` | Resource does not exist (including resources that belong to someone else) |
| 409 | `conflict` | Uniqueness conflict (email, slug) |
| 410 | `gone` | Short link expired or reached its click limit |
| 422 | `validation_error` | Field validation failed; `details.field` names the offending field |
| 429 | `rate_limited` | Rate limited; the response carries a `Retry-After` header |
| 500 | `internal_error` | Server error (details go to the log only) |

Pagination envelope:

```json
{ "items": [], "page": 1, "per_page": 20, "total": 0, "total_pages": 1 }
```

`per_page` defaults to 20 and is capped at 100.

---

## Authentication

### `POST /api/v1/auth/register`

```json
{ "email": "user@example.com", "password": "at-least-8-chars", "invite_code": "optional", "display_name": "optional", "captcha_token": "optional" }
```

`201` → `{ "user": { ... } }`, and the response sets a session cookie (registering signs you in).

Errors: `409` duplicate email, `422` validation failure, `403` registration closed or invalid
invite code, `400` `captcha_failed` when bot verification is enabled and rejects the request.

`captcha_token` carries the token produced by the CAPTCHA widget on the web registration form.
It is only checked when bot verification is enabled for registration; otherwise the field is
ignored. See [Bot verification](#bot-verification).

> `POST /api/v1/urls` is exempt from bot verification **only when it authenticates with an API
> token** (`Authorization: Bearer uw_...`), so automation and CI pipelines are unaffected. A
> request authenticated with a **session cookie** is a browser session and is subject to the
> link-creation switch; send the widget token in `captcha_token`.

### `POST /api/v1/auth/login`

```json
{ "email": "user@example.com", "password": "...", "captcha_token": "optional" }
```

`200` → `{ "user": { ... } }` + `Set-Cookie`.
Errors: `401` wrong credentials (the message is fixed and does not reveal whether the account
exists), `403` account suspended, `400` `captcha_failed` when bot verification is enabled for
sign-in and rejects the request, `429` rate limited.

`captcha_token` carries the token produced by the CAPTCHA widget on the web sign-in form. It is
only checked when bot verification is enabled for sign-in (`captcha_on_login` /
`UWURLS_CAPTCHA_ON_LOGIN`); otherwise the field is ignored. See
[Bot verification](#bot-verification).

### `POST /api/v1/auth/logout`

`204`, revokes the current session and clears the cookie (may be called anonymously).

### `GET /api/v1/auth/me`

`200` → `{ "user": { ... } }`

### `PATCH /api/v1/auth/me`

```json
{ "display_name": "Ada" }
```

An empty string or `null` clears the display name. `200` → `{ "user": { ... } }`

### `POST /api/v1/auth/password`

```json
{ "current_password": "...", "new_password": "..." }
```

`204`. On success every **other** session is revoked (the current one survives) **and all API
tokens for the account are revoked**.
Errors: `401` wrong current password, `422` new password does not meet the policy.

### `GET /api/v1/auth/sessions`

`200` → `{ "items": [ { "id", "user_agent", "created_at", "last_seen_at", "expires_at", "current", "revoked" } ] }`

Sessions slide: any activity (written at most once per minute) pushes `expires_at` forward and
re-issues the cookie, so continuous use never expires.

### `DELETE /api/v1/auth/sessions/{id}`

`204`. You can only revoke your own sessions; anything else is `404`.

### `GET /api/v1/tokens` · `POST /api/v1/tokens` · `DELETE /api/v1/tokens/{id}`

`POST` body: `{ "name": "ci pipeline", "expires_at": "2030-01-01" }`

`201` → the plaintext token is shown **this one time only**:

```json
{
  "token": "uw_XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX",
  "api_token": { "id": 1, "name": "ci pipeline", "prefix": "uw_abc12345", "scopes": "...", "created_at": "...", "last_used_at": null, "expires_at": "2030-01-01T00:00:00.000Z", "revoked": false, "active": true }
}
```

`DELETE` → `204`.

---

## Short links

### `GET /api/v1/urls`

Query parameters:

| Parameter | Meaning |
|-----------|---------|
| `q` | Search slug, title and target URL (`%` and `_` are treated literally) |
| `tag` | Filter by tag name |
| `status` | `active` or `disabled` |
| `sort` | `created_at` (default), `clicks`, `slug`, `updated`, `expires` |
| `page` / `per_page` | Pagination |

`200` → a paginated array of `UrlDto`.

### `POST /api/v1/urls`

```json
{
  "target_url": "example.com/path",
  "slug": "promo",
  "title": "Launch",
  "notes": "internal note",
  "tags": ["campaign", "docs"],
  "password": "linkpass",
  "expires_at": "2030-06-30 12:00",
  "max_clicks": 500,
  "redirect_type": 302,
  "captcha_token": "0.abc..."
}
```

`201` → `UrlDto`. Rules:

- `captcha_token` is only read when the request is authenticated with a **session cookie** and
  bot verification is enabled for link creation; a `Bearer` token call ignores it. A failed
  check returns `400` with `error.code = "captcha_failed"`
- `target_url` is required; only `http`/`https` are accepted, and a bare domain without a scheme
  is given a `https://` prefix
- `slug` may be omitted (a 7-character base62 slug is generated); only `[A-Za-z0-9_-]` is
  allowed, length 1–64, and reserved words are rejected
- `expires_at` must be in the future; `max_clicks` ≥ 1
- `redirect_type` ∈ {301, 302, 307, 308}
- Exceeding the `max_urls_per_user` quota → `403`

### `GET /api/v1/urls/{id}`

`200` → `UrlDto` (`state` is `active` / `expired` / `limit_reached` / `disabled`).

### `PATCH /api/v1/urls/{id}`

Partial update: **fields that are absent stay unchanged, while an explicit `null` clears them**.

```json
{ "title": "New title", "password": null, "max_clicks": null, "tags": ["new"], "status": "disabled" }
```

`200` → the updated `UrlDto`. An empty object → `400`.

### `DELETE /api/v1/urls/{id}`

`204`, soft delete. The slug is retained and never released (so old links cannot be hijacked),
and analytics data is kept.

### `GET /api/v1/urls/{id}/stats?days=30`

```json
{
  "url": { ... },
  "summary": { "total_clicks": 42, "human_clicks": 41, "unique_clicks": 3, "bot_clicks": 1, "first_click_at": "...", "last_click_at": "..." },
  "daily": [ { "day": "2026-02-01", "clicks": 5, "unique_clicks": 2, "bot_clicks": 0 } ],
  "referrers": [ { "label": "news.example.com", "count": 34 } ],
  "devices": [], "browsers": [], "operating_systems": []
}
```

`days` ranges from 1 to 365 (default 30), and `daily` is padded with the days that had no clicks.

### `GET /api/v1/urls/{id}/clicks?page=1`

`200` → a paginated click log (**no** IP addresses or visitor hashes):

```json
{ "items": [ { "clicked_at": "...", "referrer_host": "news.example.com", "country_code": null, "device_type": "Desktop", "browser": "Chrome", "os": "Windows", "is_bot": false, "is_unique": true, "password_protected": false } ], "page": 1, "per_page": 20, "total": 42, "total_pages": 3 }
```

### `UrlDto`

```json
{
  "id": 1,
  "slug": "promo",
  "short_url": "https://s.example.com/promo",
  "target_url": "https://example.com/landing",
  "title": "Launch",
  "notes": null,
  "tags": ["campaign"],
  "has_password": true,
  "expires_at": "2030-06-30T12:00:00.000Z",
  "expires_in": "in 52 months",
  "max_clicks": 500,
  "click_count": 42,
  "unique_click_count": 3,
  "redirect_type": 302,
  "status": "active",
  "is_expired": false,
  "state": "active",
  "is_custom_slug": true,
  "last_clicked_at": "2026-02-11T09:00:00.000Z",
  "created_at": "...",
  "updated_at": "..."
}
```

---

## Tags

### `GET /api/v1/tags`

`200` → `{ "items": [ { "id": 1, "name": "campaign", "color": null, "url_count": 4 } ] }`

### `DELETE /api/v1/tags/{id}`

`204`, deletes the tag (it only unlinks the tag from short links; the links themselves are
untouched).

---

## Public endpoints

### `GET /`

The landing page: an introduction to the service with sign-in and sign-up entry points. It no
longer redirects to `/app`; a visitor who is already signed in sees a button that opens the
dashboard.

### `GET /health`

```json
{ "status": "ok", "version": "1.0.0", "database": "ok", "uptime_seconds": 42 }
```

When the database cannot be queried the response is `503` with `status` set to `degraded`.
`/healthz` is an alias of the same endpoint.

### `GET /{slug}`

- Valid and unprotected → `30x` + `Location` (301/308 send `private, max-age=300`; 302/307 send
  `private, no-store`)
- Password protected → `200` password prompt (**the target URL is never disclosed**)
- Expired or click limit reached → `410`
- Missing / deleted / disabled → `404`

### `POST /{slug}/unlock`

Accepts `application/json` or `application/x-www-form-urlencoded`:

```json
{ "password": "linkpass" }
```

- Successful JSON request → `200` `{ "ok": true, "redirect": "/slug" }` + unlock cookie
- Successful form request → `303` redirect to `/{slug}` + unlock cookie
- Wrong password → JSON `401`; forms re-render the password page (`401`)
- Rate limited → `429`

### `GET /api/v1/public/urls/{slug}`

Public metadata without the target URL:

```json
{ "slug": "promo", "title": "Launch", "has_password": true, "is_expired": false, "state": "active" }
```

---

## UI language

The web UI ships in English (`en`), Traditional Chinese (`zh-Hant`) and Simplified Chinese
(`zh-Hans`). On a first visit the language is chosen from the browser's `Accept-Language`
header; when nothing matches, English is used. The selection is stored in the `uwurls_lang`
cookie.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/app/lang/{code}?next=<path>` | Switch the UI language, set the `uwurls_lang` cookie, then return `302` to `next` (or to `/app` when `next` is missing or not a same-site relative path) |

`code` is one of `en`, `zh-Hant`, `zh-Hans`; `next` must be a same-site relative path.
An unrecognized `code` falls back to English (it is never an error), and a visitor without a
cookie is negotiated from `Accept-Language` instead — a present cookie always wins.

The language setting only affects rendered HTML. **JSON responses are always English** and
never change with the selected language.

---

## Bot verification

uwurls can require a CAPTCHA on the three public forms that create state: the web registration
form, the web sign-in form, and the web create-short-link form. The JSON API endpoints
`POST /api/v1/auth/register` and `POST /api/v1/auth/login` accept the widget token in the optional
`captcha_token` field. It is off by default and configured either through environment variables or
in the admin UI under **Admin → Settings**; a non-empty environment variable overrides the admin
setting. See the deployment guide for the variables and for how to obtain provider keys.

| Provider | `provider` value | Widget |
|----------|------------------|--------|
| Disabled (default) | `none` | None |
| Cloudflare Turnstile | `turnstile` | Managed Turnstile widget |
| Google reCAPTCHA v2 | `recaptcha` | "I'm not a robot" checkbox |

Verification applies to a form only when all four conditions hold: the provider is not `none`,
the site key is non-empty, the secret key is non-empty, and the matching per-form switch
(`captcha_on_register`, `captcha_on_login` or `captcha_on_create`) is `true`.

Failure is **closed**: a missing, malformed, expired or already-used token — including a timeout
or any non-2xx response from the provider — rejects the submission.

The `hostname` reported by the provider is also checked: it must equal the host of the instance's
base URL or one of the extra hostnames in `captcha_allowed_hostnames` (comma-separated, up to 10).
A response without a `hostname` is accepted, and a mismatch is rejected and logged at `WARN`.

| Surface | Verification |
|---------|--------------|
| Web registration form (`POST /app/register`) | Applies when enabled for registration |
| Web sign-in form (`POST /app/login`) | Applies when enabled for sign-in |
| Web create-short-link form (`POST /app/urls`) | Applies when enabled for link creation |
| `POST /api/v1/auth/register` | Applies when enabled for registration; send the widget token in `captcha_token`; failure returns `400` with `error.code = "captcha_failed"` |
| `POST /api/v1/auth/login` | Applies when enabled for sign-in; send the widget token in `captcha_token`; failure returns `400` with `error.code = "captcha_failed"` |
| `POST /api/v1/urls` | **API tokens only** — exempt when the request carries `Authorization: Bearer uw_...`; a session-cookie call is subject to the link-creation switch and takes an optional `captcha_token` |

---

## Administration (requires `role = admin`)

Every `/api/v1/admin/**` endpoint returns `403 forbidden` for non-administrators.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/admin/users?q=&role=&status=&page=` | User list (includes `link_count`) |
| PATCH | `/admin/users/{id}` | `{ "role"?, "status"?, "display_name"? }` |
| POST | `/admin/users/{id}/password` | `{ "new_password": "..." }` → `204`, revokes every session for that account |
| DELETE | `/admin/users/{id}` | `204`, soft-deletes the account and all of its short links |
| GET | `/admin/urls?q=&status=&user_id=&page=` | Every short link on the instance |
| PATCH | `/admin/urls/{id}` | `{ "status"?, "redirect_type"? }` |
| DELETE | `/admin/urls/{id}` | `204` |
| GET | `/admin/invites` | Invite code list (prefixes only) |
| POST | `/admin/invites` | `{ "note"?, "max_uses"?, "expires_at"? }` → `201`, **the plaintext invite code is returned once** |
| DELETE | `/admin/invites/{id}` | `204`, revoke |
| GET | `/admin/settings` | Current settings |
| PUT | `/admin/settings` | `{ "site_name": "...", "registration_mode": "invite", ... }` |
| GET | `/admin/audit?action=&target_type=&actor=&page=` | Audit log (`metadata` is parsed into JSON) |
| GET | `/admin/stats` | User / short link / click totals, database size, version, uptime |

**Guard rails**: an administrator cannot change their own role or status, cannot delete
themselves, and the instance always keeps at least one active administrator (otherwise the
request fails with `409`).

Editable setting keys: `site_name`, `registration_mode` (`open`/`invite`/`closed`),
`default_slug_length` (4–16), `max_urls_per_user` (0 = unlimited),
`default_redirect_type` (301/302/307/308), `analytics_enabled`,
`click_retention_days` (0 = keep forever), plus the bot verification keys
`captcha_provider`, `captcha_site_key`, `captcha_secret_key`, `captcha_on_register`,
`captcha_on_login`, `captcha_on_create` and `captcha_allowed_hostnames`. The secret key is
write-only: it is never returned in plaintext, and submitting it empty keeps the stored value.

---

## Audit action codes

| Action | Meaning |
|--------|---------|
| `auth.register` / `auth.login` / `auth.login_failed` / `auth.logout` | Authentication events |
| `auth.password_changed` / `auth.session_revoked` / `user.profile_updated` | Account changes |
| `url.created` / `url.updated` / `url.deleted` | Short link changes |
| `token.created` / `token.revoked` | API tokens |
| `invite.created` / `invite.revoked` / `invite.redeemed` | Invite codes |
| `admin.user_updated` / `admin.user_deleted` / `admin.password_reset` | Administrative actions |
| `admin.url_updated` / `admin.settings_updated` | Administrative actions |

## Rate limits

| Scope | Limit |
|-------|-------|
| Login | 10 attempts per 5 minutes per (IP, email), **and** 40 per 5 minutes per IP (across accounts) |
| Registration | 5 attempts per hour per IP |
| Short link unlock | 10 attempts per 5 minutes per (slug, IP) |
| Short link creation | 120 per hour per user |
| Password change | 10 per hour per user |

Exceeding a limit returns `429` with `Retry-After` (seconds). The in-memory counters reset on
restart, but **account lockout** is persistent: once one email accumulates 30 failures within
15 minutes, even the correct password returns `429` (`Retry-After: 900`) until the window slides
past.

The IP used for login comes from `X-Real-IP` when `UWURLS_TRUST_PROXY=true`, falling back to the
rightmost address in `X-Forwarded-For`.
