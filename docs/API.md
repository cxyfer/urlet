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

The web form accepts the same code as `/app/register?invite=CODE`; the link produced for an invite
prefills the field. The JSON body is unchanged.

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
    "redirect_mode_policy": "choice",
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
`redirect_mode_policy` is `direct`, `splash` or `choice`; it limits the redirect mode of **new**
links and never rewrites existing ones. An ungrouped account gets `choice`.

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
  "redirect_mode": "splash",
  "splash_delay_seconds": 7,
  "splash_title": "Launch day",
  "splash_message": "Welcome aboard",
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
- `redirect_mode` is `direct` (default) or `splash`. Omitted, it takes the group policy's default
  (splash under a splash-only policy, otherwise direct). A mode the group blocks returns `403`
- `splash_delay_seconds` is 1–60 (default 5). `splash_title` (at most 120 characters) and
  `splash_message` (at most 500 characters, newlines allowed) are optional; empty/omitted uses the
  localised built-in copy. They are stored even while the link is direct, so switching modes keeps
  the copy
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
  "redirect_mode": "splash",
  "splash_delay_seconds": 7,
  "splash_title": "Launch day",
  "splash_message": "Welcome aboard",
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
always a new value and is refused while passwords are blocked. The redirect-mode policy blocks a
new `redirect_mode` (or switching to it on `PATCH`) but accepts keeping the current mode and
clearing splash copy.

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

When the visitor is allowed to create a link — see the `public_create_access` setting below — the
page also carries the public shortener. `?shortened=<slug>&domain=<hostname>` renders the short URL
of a link that was just created; the value is resolved against the database, so the result box never
prints a query-string value verbatim and never discloses the destination.

### `POST /`

The public shortener form (`application/x-www-form-urlencoded`), available on the canonical host
only. It accepts `target_url` (required) plus the optional inputs the administrator enabled:
`slug`, `title`, `tags` (comma separated), `password`, `expires_at`, `max_clicks`, `redirect_mode`,
`splash_delay_seconds`, `splash_title`, `splash_message`, `domain`, and the provider token field
(`cf-turnstile-response` or `g-recaptcha-response`).

A successful submission is `303` to `/?shortened=<slug>&domain=<hostname>`. A failure is `303` to
`/?err=<code>`, where `<code>` is one of `denied` (below the access level, or a disabled field),
`rate_limited`, `captcha`, `quota` (the `public_max_urls` budget of the Guest account is full),
`conflict`, `slug`, `target`, `expiry`, `tags`, `password_rules`, `domain`, `mode_denied` or
`invalid`. A non-numeric `max_clicks` or `splash_delay_seconds` is reported as `invalid` rather
than silently dropped.

The CSRF same-origin rule applies to the form, and the ordinary security headers are unchanged.

### `POST /api/v1/public/urls`

The same operation as JSON, for scripts. No credential is required when `public_create_access` is
`guest`; a session cookie or a Bearer token may be supplied instead, and the link is then owned by
that account.

```json
{
  "target_url": "https://example.com/launch",
  "slug": "launch",
  "title": "Launch day",
  "captcha_token": "0.abc..."
}
```

`201`:

```json
{
  "domain": "s.example.com",
  "slug": "launch",
  "short_url": "https://s.example.com/launch",
  "target_url": "https://example.com/launch",
  "created_at": "2026-10-22T12:00:00.000Z"
}
```

The link id is deliberately not returned: the caller has no account to manage the link with.

- `403` `forbidden` — the caller is below the access level, or submitted a field the administrator
  disabled
- `400` `captcha_failed` — the bot protection check failed (fail closed)
- `429` `rate_limited` — the per-address or the shared visitor budget is exhausted (`Retry-After`)
- `409` `conflict` — the requested slug is already taken
- `422` `validation_error` — an invalid target URL or option

### `GET /health`

```json
{ "status": "ok", "version": "1.4.0-rc.1", "database": "ok", "uptime_seconds": 42 }
```

When the database cannot be queried the response is `503` with `status` set to `degraded`.
`/healthz` is an alias of the same endpoint.

### `GET /{slug}`

- Valid, unprotected and `redirect_mode = direct` → `30x` + `Location` (301/308 send
  `private, max-age=300`; 302/307 send `private, no-store`)
- Valid, unprotected and `redirect_mode = splash` → `200` splash page (`no-store`). It
  shows the custom copy, the destination host and a countdown, sends the visitor on with a
  `<meta http-equiv="refresh">` and also offers a "Continue now" link. The click is recorded when
  the splash page is served, not when the visitor leaves
- Password protected → `200` password prompt (**the target URL is never disclosed**). After a
  successful unlock the destination is served directly: the password page is the only
  interstitial, so a splash page is never shown for a protected link
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
{ "domain": "go.example.com", "slug": "promo", "title": "Launch", "has_password": true, "redirect_mode": "splash", "is_expired": false, "state": "active" }
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
is shared with administrator-issued codes. Revoking a code that nobody redeemed
returns its slot; a redeemed code keeps it.

### `GET /api/v1/invites`

`GET /api/v1/invites?page=&per_page=` → `200`: the caller's own codes (newest
first), including the displayable code and its registration link, plus the
effective quota.

```json
{
  "items": [
    { "id": 4, "prefix": "RW6M", "code": "RW6MF6C6JG",
      "link": "https://s.example.com/app/register?invite=RW6MF6C6JG",
      "note": "team", "group_id": 4, "max_uses": 2,
      "used_count": 1, "remaining": 1, "expires_at": null, "status": "active",
      "usable": true, "created_at": "2026-03-18T10:00:00.000Z" }
  ],
  "page": 1, "per_page": 20, "total": 1,
  "quota": { "group_id": 4, "group_name": "Invite Team", "allowed": true,
             "limit": 5, "period": "month", "used": 1, "remaining": 4 }
}
```

`code` and `link` are `null` for codes created before sealed storage or after
`URLET_SECRET` changed; `prefix` is always present. `link` is also `null` when
the code is revoked, exhausted or expired, while `code` stays visible for
history. `per_page` defaults to 20 and
is capped at 100. `quota.allowed` is `false` when the group does not enable
member invites: creating a code then returns `403`, but codes issued before the
toggle was turned off remain listed and revocable. `remaining` is `null` when
`limit` is `0` (unlimited). Accounts whose group link is missing fall back to the
default group.

Member issuance is throttled to 20 codes per hour per account (`429` when the
burst is exhausted); the group's invite quota remains the primary limit. A group
with no finite quota is otherwise unlimited.

### `POST /api/v1/invites`

`{ "note"?: "team", "max_uses"?: 2 }` → `201`:

```json
{ "code": "RW6MF6C6JG", "link": "https://s.example.com/app/register?invite=RW6MF6C6JG",
  "invite": { "...": "..." }, "quota": { "...": "..." } }
```

The code and link are also returned by `GET /api/v1/invites`, so they are not
lost when the response is dismissed. The top-level `link` mirrors `invite.link`;
both are `null` if the created code is already unusable. `max_uses` must be
between 1 and 10 for
member codes. `403` when the group does not allow invites or the period quota is
exhausted (`error.code = "forbidden"`), and `429` when the hourly burst limit is
exhausted.

### `DELETE /api/v1/invites/{id}`

`204`, revokes one of the caller's own codes. Another member's code is reported
as `404`; administrators use `DELETE /api/v1/admin/invites/{id}` for any code.
Revoking a code that nobody redeemed returns its quota slot; a code that was
already redeemed keeps it.

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
| POST | `/admin/groups` | `{ "name", "description"?, "max_urls"?, "max_api_tokens"?, "max_invites"?, "invite_period"?, "allow_invites"?, "allow_custom_slug"?, "allow_password"?, "allow_expiry"?, "allow_max_clicks"?, "allow_tags"?, "allow_api_tokens"?, "allow_domain_selection"?, "redirect_mode_policy"?, "default_domain_id"?, "restrict_domains"?, "domain_ids"?, "is_default"? }` → `201`; `default_domain_id` must reference an active domain; `domain_ids` must reference existing domains (disabled entries stay stored but are not selectable) |
| PUT | `/admin/groups/{id}` | `name` is required; other omitted fields keep their current value; `max_urls` / `max_api_tokens` / `max_invites` set to `null` inherit the site quota; `description: null` clears it; `redirect_mode_policy` is `direct`, `splash` or `choice` and is omitted to keep the current value; `default_domain_id: null` means the site default; `domain_ids: null` or `[]` clears the allowlist; `is_default: true` moves the single default marker |
| DELETE | `/admin/groups/{id}?reassign_to={id}` | `204`, moves the members then deletes; `409` for the default group or a missing/invalid target |
| GET | `/admin/themes` | Theme catalogue (key, name, description, `enabled`, `default`, `appearance`, `dark_only`, `light_only`, `css_url`, `custom`) plus `switching` (`all` or `admin`); includes administrator uploads |
| PUT | `/admin/themes` | `{ "enabled"?: ["verdant", ...], "default"?: "verdant", "switching"?: "all"\|"admin" }`; omitted fields keep their value; at least one enabled theme and an enabled default are required (`422` otherwise). `enabled` and `default` may name `custom-…` uploads |

Theme uploads are administrator-only browser multipart forms at `/app/admin/themes/custom` (create),
`/app/admin/themes/custom/{id}` (edit/replace CSS) and `/app/admin/themes/custom/{id}/delete`. The
JSON API deliberately has no file-upload endpoint. Uploads are CSS-only, at most 256 KiB, UTF-8,
and may not load external resources; see [`THEMES.md`](THEMES.md).

`GroupDto` also returns `default_domain_id`, `default_domain` (the stored value, even if the domain
is disabled), `restrict_domains`, `allowed_domain_ids` and `allowed_domains`, plus the invite
quota (`max_invites`, `invite_period`) and its live counters (`invites_used`,
`invites_remaining`; the latter is `null` when unlimited). The request field is
`domain_ids` (`null` or `[]` clears it; omitting keeps it); the response uses `allowed_*`. `GET /api/v1/auth/me` returns the *effective* fallback
default, which can differ from `default_domain` when the stored group default is disabled.
| GET | `/admin/urls?q=&status=&user_id=&page=` | Every short link on the instance |
| PATCH | `/admin/urls/{id}` | `{ "status"?, "redirect_type"? }` |
| DELETE | `/admin/urls/{id}` | `204` |
| GET | `/admin/invites` | Invite code list with `code`, `link`, `prefix`, `group_id` / `group_name`; `code` and `link` are `null` for legacy or secret-rotated rows, and `link` is also `null` for revoked, exhausted or expired codes |
| POST | `/admin/invites` | `{ "note"?, "max_uses"?, "expires_at"?, "group_id"? }` → `201` `{ "code", "link", "invite" }`; `link` mirrors `invite.link` and is `null` if the code is already unusable; the code stays available from `GET` and the link is returned while usable; `403` when the group's per-period invite quota is exhausted |
| DELETE | `/admin/invites/{id}` | `204`, revoke; an unused code returns its quota slot, a redeemed code keeps it |
| GET | `/admin/settings` | Current settings, including `theme_switching` (`all` or `admin`) |
| PUT | `/admin/settings` | `{ "site_name": "...", "registration_mode": "invite", "theme_switching": "admin", ... }` |
| GET | `/admin/audit?action=&target_type=&actor=&page=` | Audit log (`metadata` is parsed into JSON) |
| GET | `/admin/stats` | User / short link / click totals, database size, version, uptime |

**User groups.** A group object is
`{ id, name, description, max_urls, max_api_tokens, max_invites, invite_period, allow_*,
is_default, members, created_at, updated_at }`. `redirect_mode_policy` is `direct`, `splash` or
`choice` (create default `choice`). `allow_invites` is opt-in (`false` on create) and
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
limit. The counter lives in an issuance ledger: revoking a code that nobody redeemed releases its
slot, while a redeemed code keeps it. The quota resets by itself at the next period boundary.
Upgrading releases codes that were already revoked and never redeemed, so the counter can drop once.
Invites created without `group_id` consume the site-wide quota instead.

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

The public shortener adds `public_create_access` (`guest`/`user`/`admin`, default `user`),
`public_captcha` (default `true`), `public_rate_limit_per_ip` (creations per hour, default 20,
`0` = unlimited), `public_rate_limit_global` (shared creations per hour, default 500, `0` =
unlimited), `public_max_urls` (total links the reserved Guest account may own, default 1000, `0` =
unlimited; separate from `max_urls_per_user`) and the field switches `public_allow_slug`, `public_allow_title` (both default `true`)
plus `public_allow_tags`, `public_allow_password`, `public_allow_expiry`, `public_allow_max_clicks`,
`public_allow_redirect` and `public_allow_domain` (all default `false`).

---

### Tracking parameter cleanup

Administrators can enable cleanup under **Admin → Settings** or with `PUT /api/v1/admin/settings`:

```json
{
  "url_cleaning_enabled": "true",
  "url_cleaning_rules": "example.com utm_* ref\nexample.org campaign_id"
}
```

Both settings are returned by `GET /api/v1/admin/settings`; defaults are `"false"` and `""`. Setting values are strings, as with all other settings. Only administrators may read or change them. Invalid settings return `422`; a rejected batch changes neither the database nor the settings cache.

Each rule is `hostname parameter [parameter ...]`, one per line. It matches the named host and its subdomains, ignoring ASCII case in hostnames and decoded parameter names. A final `*` on a nonempty parameter name matches a prefix. Blank lines and whole-line `#` comments are ignored. Limits: 8192 bytes total, 100 rules, 32 parameters per rule and 64 bytes per parameter (including `*`). Domains must be ASCII hostnames with at least two nonempty labels, at most 253 bytes total and 63 bytes per label; labels use letters, digits or hyphens and cannot start/end with a hyphen. Schemes, ports, paths, userinfo and wildcard domains are rejected. Parameters use letters, digits, `_`, `-` or `.`, optionally followed by one final `*`; bare `*` and regex are rejected.

Built-in rules cover Facebook (`facebook.com`), Instagram, Threads (`threads.net`, `threads.com`), X/Twitter, YouTube (`youtube.com`, `youtu.be`), TikTok and Bilibili, including subdomains. They remove `utm_*` plus explicit platform tracking parameters, including `fbclid`, `igsh`, YouTube's `si` and Bilibili's `spm_id_from`; video IDs, playlists and playback times are kept. The complete list is visible in the administrator settings page. Custom rules add to built-ins; an empty custom setting keeps only built-ins. Unknown domains remain untouched unless a custom rule matches them. Custom rules can change meaningful content if they remove required parameters.

Cleanup applies **only when creating links**, through `POST /api/v1/urls`, `POST /api/v1/public/urls`, the dashboard and the public form. For example, `https://youtu.be/ABC?si=tracking&t=90` is stored and returned as `https://youtu.be/ABC?t=90`. The original target must pass validation before cleanup. Kept query components retain their original encoding, values and order; fragments remain intact. Share shortlinks are not expanded and targets are never fetched. Existing links, target edits and redirects are unchanged. Disabling cleanup preserves custom rules for later use.

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
| `admin.url_updated` / `admin.settings_updated` / `admin.themes_updated` / `admin.theme_created` / `admin.theme_updated` / `admin.theme_deleted` | Administrative actions |

## Rate limits

| Scope | Limit |
|-------|-------|
| Login | 10 attempts per 5 minutes per (IP, email), **and** 40 per 5 minutes per IP (across accounts) |
| Registration | 5 attempts per hour per IP |
| Short link unlock | 10 attempts per 5 minutes per (slug, IP) |
| Short link creation | 120 per hour per user |
| Public shortener | `public_rate_limit_per_ip` per hour per client address (default 20) **and** `public_rate_limit_global` per hour across all anonymous visitors (default 500); `0` disables either budget. The per-address budget is charged before the bot protection call, the shared one only after it succeeded |
| Password change | 10 per hour per user |

Exceeding a limit returns `429` with `Retry-After` (seconds). The in-memory counters reset on
restart, but **account lockout** is persistent: once one email accumulates 30 failures within
15 minutes, even the correct password returns `429` (`Retry-After: 900`) until the window slides
past.

The IP used for login comes from `X-Real-IP` when `URLET_TRUST_PROXY=true`, falling back to the
rightmost address in `X-Forwarded-For`.
