# urlet JSON API reference

**English** | [繁體中文](zh-TW/API.md)

- Base path: `/api/v1`
- Authentication (choose one)
  - **Session cookie**: `urlet_session`, issued by `POST /api/v1/auth/login` (the browser form flow uses the same cookie)
  - **API token**: `Authorization: Bearer ur_...` (create one under `/app/account`)
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
| 409 | `conflict` | Uniqueness conflict (email, slug on a domain, domain hostname) |
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

An invite code may carry a user group. In `invite` mode a code is mandatory; in `open` mode it is
optional, but when one is supplied it is validated and its group wins over the default group. The
returned `user` object includes `group_id`.

`captcha_token` carries the token produced by the CAPTCHA widget on the web registration form.
It is only checked when bot verification is enabled for registration; otherwise the field is
ignored. See [Bot verification](#bot-verification).

> `POST /api/v1/urls` is exempt from bot verification **only when it authenticates with an API
> token** (`Authorization: Bearer ur_...`), so automation and CI pipelines are unaffected. A
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
`URLET_CAPTCHA_ON_LOGIN`); otherwise the field is ignored. See
[Bot verification](#bot-verification).

### `POST /api/v1/auth/logout`

`204`, revokes the current session and clears the cookie (may be called anonymously).

### `GET /api/v1/auth/me`

`200` → the user, the group they belong to (or `null`) and the effective limits:

```json
{
  "user": { "id": 7, "email": "ada@example.com", "role": "user", "group_id": 2, "...": "..." },
  "group": { "id": 2, "name": "Pro" },
  "limits": {
    "max_urls": 500,
    "max_api_tokens": 5,
    "allow_custom_slug": true,
    "allow_password": true,
    "allow_expiry": true,
    "allow_max_clicks": true,
    "allow_tags": true,
    "allow_api_tokens": true,
    "allow_domain_selection": true,
    "default_domain": "links.example.com",
    "allowed_domains": ["links.example.com", "go.example.com"]
  }
}
```

`group` is `null` for an ungrouped account, which falls back to the site-wide settings. The
`limits` object is the resolved value: a group overrides a quota only when it is not `NULL`, and
`0` means unlimited. `default_domain` is the effective group default (falling back to the site
default when it is missing or disabled); `allowed_domains` is the effective selection list. When
`allow_domain_selection` is `false`, only that default is accepted for new links or moves.

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
  "token": "ur_XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX",
  "api_token": { "id": 1, "name": "ci pipeline", "prefix": "ur_abc12345", "scopes": "...", "created_at": "...", "last_used_at": null, "expires_at": "2030-01-01T00:00:00.000Z", "revoked": false, "active": true }
}
```

`DELETE` → `204`.

Token creation is refused with `403` when the group disables API tokens (`allow_api_tokens`) or
the effective `max_api_tokens` is exhausted. Only usable tokens count (not revoked, not expired),
so revoking one frees a slot immediately.

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
  "domain": "go.example.com",
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
- `domain` may be omitted (the effective group default is used). A supplied hostname must be an
  active domain, otherwise the request fails with `422`; it must also satisfy the group policy
  (`allow_domain_selection`, `restrict_domains` / `domain_ids`) or the request fails with `403`.
  The slug namespace is per domain, so the same slug can exist on two domains
- `slug` may be omitted (a 7-character base62 slug is generated); only `[A-Za-z0-9_-]` is
  allowed, length 1–64, and reserved words are rejected. A slug already used **on the selected
  domain** returns `409`
- `expires_at` must be in the future; `max_clicks` ≥ 1
- `redirect_type` ∈ {301, 302, 307, 308}
- Exceeding the `max_urls_per_user` quota → `403`

### `GET /api/v1/urls/{id}`

`200` → `UrlDto` (`state` is `active` / `expired` / `limit_reached` / `disabled`).

### `PATCH /api/v1/urls/{id}`

Partial update: **fields that are absent stay unchanged, while an explicit `null` clears them**.

```json
{ "title": "New title", "domain": "links.example.com", "password": null, "max_clicks": null, "tags": ["new"], "status": "disabled" }
```

`200` → the updated `UrlDto`. An empty object → `400`. Moving a link to another active domain
must satisfy the group policy and re-checks the slug there (`409` on a collision). Keeping a link
on its current domain — even a disabled one — is always allowed, so existing links remain
editable.

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
  "domain": "s.example.com",
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

**Group limits.** Link creation is refused with `403` when the effective `max_urls` is exhausted.
A group feature toggle blocks submitting the matching field on create (`slug`, `password`,
`expires_at`, `max_clicks`, `tags`) and refuses to newly enable it on `PATCH`. Omitting a field
keeps its current value, and clearing a blocked field is accepted; a non-empty link password is
always a new value and is refused while passwords are blocked.

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
{ "status": "ok", "version": "1.3.0-rc.1", "database": "ok", "uptime_seconds": 42 }
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

### `GET /api/v1/public/urls/{slug}?domain=<hostname>`

Public metadata without the target URL. Without `domain` the default domain is used, which keeps
callers that predate multi-domain working for links on the default domain.

```json
{ "domain": "go.example.com", "slug": "promo", "title": "Launch", "has_password": true, "is_expired": false, "state": "active" }
```

### `GET /api/v1/domains`

Authenticated users can list the active domains available for new links, already filtered by
their group policy. `is_default` marks the effective group default:

```json
[ { "id": 1, "hostname": "s.example.com", "is_default": true }, { "id": 2, "hostname": "go.example.com", "is_default": false } ]
```

---

## UI language

The web UI ships in English (`en`), Traditional Chinese (`zh-Hant`) and Simplified Chinese
(`zh-Hans`). On a first visit the language is chosen from the browser's `Accept-Language`
header; when nothing matches, English is used. The selection is stored in the `urlet_lang`
cookie.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/app/lang/{code}?next=<path>` | Switch the UI language, set the `urlet_lang` cookie, then return `302` to `next` (or to `/app` when `next` is missing or not a same-site relative path) |

`code` is one of `en`, `zh-Hant`, `zh-Hans`; `next` must be a same-site relative path.
An unrecognized `code` falls back to English (it is never an error), and a visitor without a
cookie is negotiated from `Accept-Language` instead — a present cookie always wins.

The language setting only affects rendered HTML. **JSON responses are always English** and
never change with the selected language.

---

## Bot verification

urlet can require a CAPTCHA on the three public forms that create state: the web registration
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
A response whose `hostname` is missing or empty is accepted, and a mismatch is rejected and logged
at `WARN`.

| Surface | Verification |
|---------|--------------|
| Web registration form (`POST /app/register`) | Applies when enabled for registration |
| Web sign-in form (`POST /app/login`) | Applies when enabled for sign-in |
| Web create-short-link form (`POST /app/urls`) | Applies when enabled for link creation |
| `POST /api/v1/auth/register` | Applies when enabled for registration; send the widget token in `captcha_token`; failure returns `400` with `error.code = "captcha_failed"` |
| `POST /api/v1/auth/login` | Applies when enabled for sign-in; send the widget token in `captcha_token`; failure returns `400` with `error.code = "captcha_failed"` |
| `POST /api/v1/urls` | **API tokens only** — exempt when the request carries `Authorization: Bearer ur_...`; a session-cookie call is subject to the link-creation switch and takes an optional `captcha_token` |

---

## Invite codes (member)

Members can issue invite codes for their **own group** when that group has
`allow_invites` enabled. Codes are pinned to the group, so anyone who redeems one
joins it. The group's calendar quota (`max_invites` / `invite_period`) applies and
is shared with administrator-issued codes; revoking a code does not free its slot.

### `GET /api/v1/invites`

`GET /api/v1/invites?page=&per_page=` → `200`: the caller's own codes (prefixes
only, newest first) plus the effective quota.

```json
{
  "items": [
    { "id": 4, "prefix": "RW6M", "note": "team", "group_id": 4, "max_uses": 2,
      "used_count": 1, "remaining": 1, "expires_at": null, "status": "active",
      "usable": true, "created_at": "2026-03-18T10:00:00.000Z" }
  ],
  "page": 1, "per_page": 20, "total": 1,
  "quota": { "group_id": 4, "group_name": "Invite Team", "allowed": true,
             "limit": 5, "period": "month", "used": 1, "remaining": 4 }
}
```

`per_page` defaults to 20 and is capped at 100. `quota.allowed` is `false` when
the group does not enable member invites: creating a code then returns `403`,
but codes issued before the toggle was turned off remain listed and revocable.
`remaining` is `null` when `limit` is `0` (unlimited). Accounts whose group link
is missing fall back to the default group.

Member issuance is throttled to 20 codes per hour per account (`429` when the
burst is exhausted); the group's invite quota remains the primary limit. A group
with no finite quota is otherwise unlimited.

### `POST /api/v1/invites`

`{ "note"?: "team", "max_uses"?: 2 }` → `201`:

```json
{ "code": "RW6MF6C6JG", "invite": { "...": "..." }, "quota": { "...": "..." } }
```

The plaintext code is returned exactly once. `max_uses` must be between 1 and 10
for member codes. `403` when the group does not allow invites or the period quota
is exhausted (`error.code = "forbidden"`), and `429` when the hourly burst limit
is exhausted.

### `DELETE /api/v1/invites/{id}`

`204`, revokes one of the caller's own codes. Another member's code is reported
as `404`; administrators use `DELETE /api/v1/admin/invites/{id}` for any code.

---

## Administration (requires `role = admin`)

Every `/api/v1/admin/**` endpoint returns `403 forbidden` for non-administrators.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/admin/users?q=&role=&status=&group_id=&page=` | User list (includes `link_count`, `group_id`, `group_name`) |
| PATCH | `/admin/users/{id}` | `{ "role"?, "status"?, "display_name"?, "group_id"? }`; `group_id: null` makes the user ungrouped |
| POST | `/admin/users/{id}/password` | `{ "new_password": "..." }` → `204`, revokes every session for that account |
| DELETE | `/admin/users/{id}` | `204`, soft-deletes the account and all of its short links |
| GET | `/admin/domains` | Every domain with link counts, status and source |
| POST | `/admin/domains` | `{ "hostname": "go.example.com" }` → `201`, created as `pending` |
| PATCH | `/admin/domains/{id}` | `{ "status"?: "active"\|"disabled", "is_default"?: true }` |
| DELETE | `/admin/domains/{id}` | `204`; only a zero-link UI domain that is not the default |
| GET | `/admin/groups?page=&per_page=` | Group list with member counts (paginated) |
| POST | `/admin/groups` | `{ "name", "description"?, "max_urls"?, "max_api_tokens"?, "max_invites"?, "invite_period"?, "allow_invites"?, "allow_custom_slug"?, "allow_password"?, "allow_expiry"?, "allow_max_clicks"?, "allow_tags"?, "allow_api_tokens"?, "allow_domain_selection"?, "default_domain_id"?, "restrict_domains"?, "domain_ids"?, "is_default"? }` → `201`; `default_domain_id` must reference an active domain; `domain_ids` must reference existing domains (disabled entries stay stored but are not selectable) |
| PUT | `/admin/groups/{id}` | `name` is required; other omitted fields keep their current value; `max_urls` / `max_api_tokens` / `max_invites` set to `null` inherit the site quota; `description: null` clears it; `default_domain_id: null` means the site default; `domain_ids: null` or `[]` clears the allowlist; `is_default: true` moves the single default marker |
| DELETE | `/admin/groups/{id}?reassign_to={id}` | `204`, moves the members then deletes; `409` for the default group or a missing/invalid target |

`GroupDto` also returns `default_domain_id`, `default_domain` (the stored value, even if the domain
is disabled), `restrict_domains`, `allowed_domain_ids` and `allowed_domains`, plus the invite
quota (`max_invites`, `invite_period`) and its live counters (`invites_used`,
`invites_remaining`; the latter is `null` when unlimited). The request field is
`domain_ids` (`null` or `[]` clears it; omitting keeps it); the response uses `allowed_*`. `GET /api/v1/auth/me` returns the *effective* fallback
default, which can differ from `default_domain` when the stored group default is disabled.
| GET | `/admin/urls?q=&status=&user_id=&page=` | Every short link on the instance |
| PATCH | `/admin/urls/{id}` | `{ "status"?, "redirect_type"? }` |
| DELETE | `/admin/urls/{id}` | `204` |
| GET | `/admin/invites` | Invite code list (prefixes only, plus `group_id` / `group_name`) |
| POST | `/admin/invites` | `{ "note"?, "max_uses"?, "expires_at"?, "group_id"? }` → `201`, **the plaintext invite code is returned once**; `403` when the group's per-period invite quota is exhausted |
| DELETE | `/admin/invites/{id}` | `204`, revoke |
| GET | `/admin/settings` | Current settings |
| PUT | `/admin/settings` | `{ "site_name": "...", "registration_mode": "invite", ... }` |
| GET | `/admin/audit?action=&target_type=&actor=&page=` | Audit log (`metadata` is parsed into JSON) |
| GET | `/admin/stats` | User / short link / click totals, database size, version, uptime |

**User groups.** A group object is
`{ id, name, description, max_urls, max_api_tokens, max_invites, invite_period, allow_*,
is_default, members, created_at, updated_at }`. `allow_invites` is opt-in (`false` on create) and
lets the group's members issue their own codes, drawn from the same quota as administrator-issued
codes.
Quotas are nullable: `null` inherits the site-wide setting, `0` means unlimited. Feature toggles
default to `true`; they gate new uses and never alter existing links or tokens. Administrators
follow the same rules as other users. `PUT` accepts a partial group: omitted fields keep their
current value, quotas set to `null` inherit the site quota, `description: null` clears it, and
`is_default: true` moves the marker. `POST` keeps the create defaults (omitted toggles are allowed,
omitted quotas inherit).

**Invite quotas.** Each group may issue a limited number of invite codes per calendar period.
`max_invites` is nullable (`null` inherits the site-wide `max_invites_per_period`, `0` is
unlimited) and `invite_period` is `day`, `week` (Monday start) or `month`, evaluated in UTC.
Issuing past the limit returns `403` with `error.code = "forbidden"` and a message naming the
limit. The counter lives in an issuance ledger, so revoking a code does **not** return its slot;
it resets by itself at the next period boundary. Invites created without `group_id` consume the
site-wide quota instead.

**Guard rails**: an administrator cannot change their own role or status, cannot delete
themselves, and the instance always keeps at least one active administrator (otherwise the
request fails with `409`). The default group cannot be deleted; move the marker with `PUT` first.

Editable setting keys: `site_name`, `registration_mode` (`open`/`invite`/`closed`),
`default_slug_length` (4–16), `max_urls_per_user` (0 = unlimited),
`max_api_tokens_per_user` (0 = unlimited, default 20),
`max_invites_per_period` (0 = unlimited, default 0), `invite_period` (`day`/`week`/`month`,
default `month`), `default_redirect_type` (301/302/307/308), `analytics_enabled`,
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
| `admin.group_created` / `admin.group_updated` / `admin.group_deleted` | Group changes |
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

The IP used for login comes from `X-Real-IP` when `URLET_TRUST_PROXY=true`, falling back to the
rightmost address in `X-Forwarded-For`.
