# Multi-domain support

Status: implemented on `feat/multi-domain`
Applies to: urlet (next release)
Last updated: 2026-10-18

## 1. Summary

urlet serves short links on one **canonical host** plus any number of **extra
domains**. The canonical host keeps the full application (`/`, `/app/*`,
`/api/v1/*`). Extra domains are redirect-only: they answer `GET /{slug}` and
`POST /{slug}/unlock`, show a small branded landing page at `/`, and send
everything else back to the canonical host (or reject it).

Each domain owns its own slug namespace: `(domain_id, slug)` is unique. The same
slug may exist on `a.example.com` and `b.example.com` and resolve to different
targets.

## 2. Product decisions

| # | Decision | Choice |
|---|----------|--------|
| 1 | Extra-domain scope | Redirect-only. `/app/*` GET/HEAD → 308 to canonical; other methods → 404. `/api/v1/*` → 404 JSON. |
| 2 | Extra-domain root | A small branded landing page (`noindex`), not the canonical marketing page and not a 404. |
| 3 | Default domain | Administrators may change it among active domains. The canonical host always exists and is always active, but need not be the default. |
| 4 | New domains | Created as `pending`; an administrator activates them after DNS/TLS is ready. Only `active` domains are selectable. |
| 5 | Scheme | One scheme for the whole instance, inherited from `URLET_BASE_URL`. No per-domain scheme column. |
| 6 | Public metadata API | Add an optional `?domain=<hostname>` parameter; omitted means the default domain. |
| 7 | Environment seed | `URLET_DOMAINS` inserts missing domains only; it never changes status, default or verification of existing rows. |

Implementation defaults agreed for the open points:

- Domains seeded through `URLET_DOMAINS` are inserted as `active` and verified:
  they are declared by the operator in deployment configuration.
- The simple landing page shows the site name, one explanatory sentence and a
  link to the canonical host. No login, registration or slug input.
- The current default domain cannot be disabled or deleted until another active
  domain is made default.
- `domains.source` distinguishes `system`, `env` and `ui`. The system/canonical
  domain can be neither disabled nor deleted; environment-managed domains can be
  disabled but not deleted from the UI.

## 3. Host roles and routing

| Host | `/` | `/{slug}` | `/app/*` | `/api/v1/*` | `/health` |
|------|-----|-----------|----------|-------------|-----------|
| canonical | existing landing page | redirect | full app | full API | 200 |
| extra, `active` | simple landing page | redirect | GET/HEAD 308 to canonical; other methods 404 | 404 JSON | 200 |
| extra, `pending`/`disabled` | 404 | 404 | 404 | 404 JSON | 200 |
| unknown host | 404 | 404 | 404 | 404 JSON | 200 |

`/assets/*`, `/favicon.ico`, `/favicon.svg`, `/health` and `/healthz` bypass the
host gate so that error pages and container health checks keep working on every
host.

## 4. Data model

### `domains`

Added by migration `0003_multi_domain`:

| Column | Meaning |
|--------|---------|
| `status` | `pending`, `active` or `disabled` |
| `source` | `system` (canonical), `env` (`URLET_DOMAINS`) or `ui` |
| `updated_at` | Last lifecycle change |

`hostname` is stored lower-case without a trailing dot and with an optional
non-default port. A `NOCASE` unique index backs case-insensitive lookups. The
existing `is_default` partial unique index keeps exactly one default domain.

### `urls`

- `domain_id INTEGER NOT NULL REFERENCES domains(id) ON DELETE RESTRICT`
- `UNIQUE (domain_id, slug)` with the existing `COLLATE NOCASE` on `slug`
- legacy `NULL` values are back-filled to the default domain
- soft-deleted rows keep their `(domain_id, slug)` reservation

## 5. Migration `0003_multi_domain`

Before renaming, the migration normalises hostnames into a temporary mapping and
merges rows that become identical (case, trailing dot, default port). The
survivor is the default row when the group has one, otherwise the lowest id;
`urls.domain_id` is remapped to it and duplicate rows are deleted. This handles
a v2 installation that changed `URLET_BASE_URL` from `example.com:443` to
`example.com` and therefore already had both rows.

SQLite cannot drop the inline `UNIQUE` on `urls.slug`, so the table is rebuilt
with the official 12-step procedure. Because `clicks`, `url_tags` and
`url_daily_stats` reference `urls` with `ON DELETE CASCADE`, the rebuild runs on
a dedicated connection with `PRAGMA foreign_keys = OFF` **outside** the
transaction. The migration runner distinguishes transactional migrations from
rebuild migrations and always restores `PRAGMA foreign_keys = ON` before the
connection is closed. `PRAGMA foreign_key_check` must return no rows before
commit.

Before upgrading: back up the SQLite file. The migration has no down migration;
rollback is restoring the backup and the previous binary. Tests seed a v2
database with URLs, clicks, tags and daily stats, apply `0003`, and assert row
counts, foreign keys, back-filled domains, per-domain slug uniqueness and a
working `sqlite_sequence`.

## 6. Host resolution

- Read the `Host` header. An absent `Host` (HTTP/1.0 and tests) is treated as the
  canonical host. A present but undecodable/invalid `Host` is an unknown host.
- Normalise: trim, lower-case, strip a trailing dot, strip `:80`/`:443`, keep
  other ports, reject schemes, paths and control characters.
- The canonical host may be an IPv6 literal or a Docker-style underscore service
  name; those are matched through an opaque ASCII fallback because extra domains
  still have to be normal hostnames. A Unicode canonical host must be supplied
  as punycode; an unparseable Unicode `URLET_BASE_URL` fails startup instead of
  silently serving 404s.
- `X-Forwarded-Host` is not trusted in this version. The reverse proxy must
  preserve the original `Host` (Caddy does by default).
- A `DomainRegistry` snapshot in `AppState` maps hostnames and ids to domains and
  remembers the active host list and default id. It is loaded during bootstrap
  and reloaded after every admin mutation. Direct database edits are picked up
  on the next restart.
- The domain gate runs after locale negotiation and before CSRF. It inserts a
  `HostContext` extension (`canonical` or `active` domain) for handlers.
- Unknown, pending and disabled hosts never reach redirect handlers.

## 7. Creating and displaying links

- `CreateInput` and `POST /api/v1/urls` accept an optional `domain` hostname.
  Omitted means the current default domain. The target must be an `active`
  domain; `pending` and `disabled` domains are rejected.
- All slug checks, generated-slug retries and update checks are scoped to the
  selected domain. Conflicts are reported with the hostname.
- `PATCH /api/v1/urls/{id}` accepts `domain`; moving a link re-checks the slug on
  the target domain and returns `409` on a conflict.
- `UrlDto` gains `domain`; `short_url` is `{scheme}://{hostname}/{slug}` using the
  instance scheme.
- HTML create and edit forms show a domain select populated from active domains;
  the dashboard and detail pages open `short_url` rather than a relative path.
- `GET /api/v1/domains` lists active domains for authenticated users.
- `GET /api/v1/public/urls/{slug}?domain=<hostname>` resolves the default domain
  when the parameter is absent, preserving existing callers. The response gains
  `domain`.

## 8. Administration

- New `/app/admin/domains` page and `/api/v1/admin/domains` endpoints.
- Actions: add (`pending`), activate, disable, set default, delete.
- Rules:
  - canonical (`source = system`) is always active and cannot be disabled or
    deleted. If `URLET_BASE_URL` changes, bootstrap creates the new canonical
    row, transfers the default when the previous system row held it, and demotes
    the previous system row to `ui`; its links keep resolving until an
    administrator disables it;
  - `pending`/`disabled` domains cannot be default;
  - the current default cannot be disabled or deleted until another active domain
    is made default;
  - `source = system|env` domains cannot be deleted from the UI (they would be
    re-seeded); they can be disabled;
  - a `ui` domain can be deleted only when it has no links at all, including
    soft-deleted links.
- Every mutation is audited (`domain.created`, `domain.updated`,
  `domain.deleted`, `domain.default_changed`) and reloads the registry.

## 9. Security

- Session cookies remain host-only; login and the application live on the
  canonical host only.
- Unlock tokens and cookie names bind both hostname and slug, so a token issued
  for `a.example.com/promo` cannot unlock `b.example.com/promo`.
- Click recording excludes every active hostname (plus the canonical host,
  `localhost` and `127.0.0.1`) from referrer statistics.
- The unlock rate-limit key includes the domain.
- Redirect `Location` values always come from the stored target or configured
  canonical base URL; request `Host` is never used to build a redirect.
- Captcha flows remain canonical-only in this version; a custom-domain
  white-label app would need per-domain site keys.
- The existing CSRF same-origin check already accepts `Origin`/`Referer` that
  match the request `Host`, which covers the unlock form on extra domains.

## 10. Deployment

- `URLET_BASE_URL` remains the canonical host and the only scheme source.
- `URLET_DOMAINS=go.example.com,links.example.com` seeds extra domains. Seed
  rows are inserted as `active`, `source = env`, verified. They are never
  updated by the seed. When `URLET_DOMAINS` is set, removing a hostname from it
  detaches that domain to `source = ui` on the next boot (status, links and
  default are preserved); it is not deleted automatically.
- Every added domain needs DNS pointing at the instance and a matching Caddy
  site block (automatic TLS). Caddy must be reloaded after adding a domain
  outside `URLET_DOMAINS`.
- All domains must use the same http/https scheme as `URLET_BASE_URL`. An
  http-only extra domain cannot be represented while the canonical host is
  https.
- The registry is in-process; this remains a single-instance SQLite deployment.

## 11. Tests

| Area | Cases |
|------|-------|
| Migration | v2 seeded DB → 0003; clicks/tags/stats preserved; FK check empty; NULL back-fill; new insert id |
| Redirect | canonical/active/pending/disabled/unknown hosts; same slug on two domains; `/app` 308; `/api` 404; `/health` any host |
| API | create with/without domain; invalid/disabled domain; slug conflict; PATCH move; public metadata `?domain=` |
| Admin | add/activate/disable/default/delete; canonical and default protections; source rules; audit |
| Security | unlock cookie A does not unlock B; `X-Forwarded-Host` ignored; open redirect |
| UI | domain selects; dashboard/detail open correct host; three locales |

## 12. Out of scope (P2)

- DNS TXT ownership verification and automatic activation.
- Per-domain branding, themes or settings.
- Per-user domain allowlists.
- Full white-label application on extra domains (login/dashboard/API).
- Dynamic Caddy configuration from the admin UI.
- Per-domain scheme and per-domain captcha keys.
