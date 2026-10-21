**English** | [繁體中文](zh-TW/THEMES.md)

# Themes

urlet ships six visual themes inside the single binary. A theme is **code**: an
Askama template directory under `templates/themes/<key>/` plus one embedded CSS
file. Nothing is uploaded, and no theme can read a path from the request.

| Key | Name | Notes |
|-----|------|-------|
| `verdant` | Verdant Systems | The v1.2 production design; also the base templates and fallback. Supports the persisted light/dark control |
| `aurora` | Aurora Glass | Dark glass command centre with a fixed sidebar. Dark-only |
| `paper` | Paper Ledger | Editorial paper ledger; follows `prefers-color-scheme` |
| `signal` | Signal | Brutalist signal board; follows `prefers-color-scheme` |
| `gilded` | Gilded Circuit | Art Deco ivory/emerald/gold; follows `prefers-color-scheme` |
| `swiss` | Swiss Telemetry | Swiss grid with numbered sections; follows `prefers-color-scheme` |

The five non-default themes are the designs proposed in the closed pull requests
#5, #6, #7, #9 and #10. They were ported to the v1.2 page model rather than frozen
at their original feature set.

## Administrator workflow

1. Open **Admin → Themes** (`/app/admin/themes`).
2. Tick the designs that should be offered. At least one theme must stay enabled.
3. Pick the **Default** theme. It must be enabled; visitors without a cookie see it.
4. Choose **Who can change their theme**:
   - `Everyone` — signed-in members may switch (the default).
   - `Administrators only` — administrators keep the picker for previews; members
     do not see it, and a direct switch returns `403`.
5. **Preview** applies a theme to the current administrator only, then returns to
   the theme page.
6. Save. The selection is applied immediately; no restart is needed.

The policy is enforced when pages render, not only by hiding the picker. Under
**Administrators only**, non-admin accounts and anonymous visitors always render
the site default; an existing `urlet_theme` cookie (including one set while the
policy was `Everyone`) is ignored. Administrators keep the picker and can use
**Preview** to inspect any compiled theme, including one that is not enabled yet,
without changing the visitor-facing default. The preview cookie lasts one hour.
A normal picker click or **Stop preview** leaves preview mode.

The same operation is available as `GET/PUT /api/v1/admin/themes`. The JSON API
accepts `enabled`, `default` and `switching`; omitted fields keep their value.

## How a visitor's theme is chosen

1. `urlet_theme` cookie, when it names an enabled theme.
2. Otherwise the administrator's default.
3. A cookie for a disabled or unknown theme is ignored and the default is used.

The picker is a native `<details>` element and works without JavaScript. The
cookie is `HttpOnly`, `SameSite=Lax`, lives for one year, and is `Secure` when
the instance is configured for secure cookies. It is a cosmetic preference, not
a per-user database column; a new browser starts at the default. The picker's
**Site default** entry clears the cookie and follows the administrator's default
again.

Like the language picker, the switch endpoint is a `GET`. It only writes a
cosmetic cookie; it never changes data or permissions. A cross-site top-level
navigation can therefore change the look, but cannot grant access or alter an
account.

## Templates and the fallback

`templates/` at the repository root is **both** the Verdant theme and the
fallback. A theme directory may override any page:

```
templates/
├── base.html                 # Verdant base / fallback
├── app_base.html
├── dashboard.html
├── ...
├── content/                  # Shared v1.2 page bodies (macros)
│   ├── dashboard.html
│   └── admin/...
└── themes/
    ├── aurora/
    │   ├── base.html
    │   ├── app_base.html
    │   ├── landing.html
    │   ├── dashboard.html     # thin wrapper around content/dashboard.html
    │   └── ...
    ├── paper/
    └── ...
```

Pages with a large amount of v1.2-only state (quotas, domains, groups, invites,
settings) are shared through Askama macros in `templates/content/`. A theme
overrides the chrome and the public/auth pages from its original design, and
wraps the shared body in its own shell. This keeps every theme feature-complete
when the product grows, at the cost of the older designs looking more like the
base content on dense admin pages. The shared `src/assets/theme-ui.css` fills in
classes those older stylesheets did not know about; it uses `:where()` so the
original design always wins where it has an opinion.

The template is selected at compile time by `themed_page!` in
`src/routes/pages/themed.rs`. Each page context gets `.themed()`, and the macro
generates one small wrapper per theme that dereferences to the same context.
Askama's compile-time field checking therefore still applies to every theme.

## Adding a theme

1. Add a variant to `ThemeKey` in `src/services/theme.rs` (key, name key,
   description key, CSS URL, dark-only flag).
2. Add the CSS under `src/assets/themes/<key>.css` and register it in
   `src/assets.rs` and the `/assets/themes/{key}` route.
3. Add the catalogue strings to the three `src/i18n/*.json` files.
4. Create `templates/themes/<key>/` with at least `base.html`, `app_base.html`,
   `landing.html`, the auth/link/error pages, the admin tabs and the app pages.
   Reuse the content macros for the dense app pages.
5. Add `templates/themes/<key>/...` paths to every `themed_page!` call. Keep the
   `Verdant` entry in each invocation: it is also the fallback used when a page
   has not listed a newly added theme yet.
6. Add the theme to `ALL`, the migration seed and the admin tests. A new release
   can also rely on `db::themes::ensure_defaults` at startup for an existing
   database.
7. Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
   `cargo test` and `scripts/smoke.sh`.

## Deliberate limits

- No theme editor or upload: themes are compiled in so the binary stays
  self-contained and the CSP stays `default-src 'self'`.
- No fine-grained per-account permission: the policy is site-wide (`all` or
  `admin`). The existing user-group feature toggles cover per-group data
  capabilities, not cosmetics.
- The five ported designs do not add a manual light/dark switch; they follow the
  operating system. Aurora is dark-only by design.
