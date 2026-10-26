**English** | [繁體中文](zh-TW/THEMES.md)

# Themes

urlet ships six visual themes inside the single binary, and an administrator can
upload additional stylesheet-only themes at runtime. A **compiled-in theme** is
code: an Askama template directory under `templates/themes/<key>/` plus one
embedded CSS file. An **uploaded theme** is a CSS file stored on the persistent
volume; it reuses the base page layout because Askama templates are compiled into
the binary and cannot be added at runtime.

| Key | Name | Notes |
|-----|------|-------|
| `verdant` | Verdant Systems | The v1.2 production design; also the base templates and fallback. Ships both palettes and the persisted light/dark control |
| `aurora` | Aurora Glass | Dark glass command centre with a fixed sidebar. Dark-only: no light palette, so the light/dark control is hidden |
| `paper` | Paper Ledger | Editorial paper ledger. Ships both palettes and the light/dark control |
| `signal` | Signal | Brutalist signal board. Ships both palettes and the light/dark control |
| `gilded` | Gilded Circuit | Art Deco ivory/emerald/gold. Ships both palettes and the light/dark control |
| `swiss` | Swiss Telemetry | Swiss grid with numbered sections. Ships both palettes and the light/dark control |
| `custom-…` | Uploaded | A stylesheet added from **Admin → Themes**; reuses the Verdant templates |

The five non-default compiled-in themes are the designs proposed in the closed
pull requests #5, #6, #7, #9 and #10. They were ported to the v1.2 page model
rather than frozen at their original feature set.

Every design except Aurora follows `prefers-color-scheme` until the visitor picks
a mode, and then honours that choice in both directions: its stylesheet carries
the dark palette twice, once behind `@media (prefers-color-scheme: dark)` guarded
by `:not([data-theme="light"])` and once behind `:root[data-theme="dark"]`. A new
port that skips the second copy still follows the operating system, but its
light/dark control would be a lie — the test
`every_dual_palette_design_serves_the_system_and_the_explicit_palette` catches
that.

## Administrator workflow

1. Open **Admin → Themes** (`/app/admin/themes`).
2. Upload a custom stylesheet: give it a name, an optional description, then
   drag a `.css` file onto the drop zone (or click it to choose a file). Choose
   an appearance — **Follow system**, **Light only (white)** or **Dark only
   (black)**. The upload appears in the catalogue immediately, enabled.
3. Tick the designs that should be offered. At least one theme must stay enabled.
4. Pick the **Default** theme. It must be enabled; visitors without a cookie see it.
5. Choose **Who can change their theme**:
   - `Everyone` — members and visitors may switch (the default). A visitor
     without a session gets the same picker on the pages that carry site chrome:
     the landing page, sign-in and registration, and error pages.
   - `Administrators only` — administrators keep the picker for previews; members
     do not see it, and a direct switch returns `403`. A visitor without a
     session is sent to the sign-in screen instead.
6. **Preview** applies a theme to the current administrator only, then returns to
   the theme page.
7. Save. The selection is applied immediately; no restart is needed.

An uploaded theme can be renamed, have its appearance and stylesheet
replaced from its edit page. **Delete** removes the catalogue row and the CSS
file; a theme that is currently the default cannot be deleted, so choose another
default first. The original file name is kept for display only; the stored path
is derived from a generated key and the content hash.

The policy is enforced when pages render, not only by hiding the picker. Under
**Administrators only**, non-admin accounts and anonymous visitors always render
the site default; an existing `urlet_theme` cookie (including one set while the
policy was `Everyone`) is ignored. Administrators keep the picker and can use
**Preview** to inspect any theme, including one that is not enabled yet, without
changing the visitor-facing default. The preview cookie lasts one hour. A normal
picker click or **Stop preview** leaves preview mode.

The redirect surface — a short link, its password prompt and the splash pages —
never offers the picker, even under `Everyone`. Those pages stay minimal and only
follow a cookie the visitor already has; the picker is one click away on the
landing page they point back to.

The same selection is available as `GET/PUT /api/v1/admin/themes`. The JSON API
accepts `enabled`, `default` and `switching`; omitted fields keep their value.
Uploads themselves are a browser multipart form: the JSON API is read/update for
the catalogue, not a file-upload endpoint. `GET` includes uploaded themes with
`"custom": true` and a hashed `css_url`.

## Custom theme storage

| Item | Location |
|------|----------|
| Metadata | `custom_themes` table in the SQLite database |
| Catalogue state | A matching `themes` row (`key`, enabled, default, order) |
| Stylesheet | `<URLET_THEMES_DIR>/<key>-<content hash>.css` |
| Default directory | `<URLET_DATA_DIR>/themes` |
| Docker | The `urlet-themes` volume, mounted at `/data/themes` |

The content hash is part of `/assets/custom-themes/{key}?v=<hash>`, so replacing
a stylesheet changes the URL and cannot be masked by the one-hour asset cache.
The metadata and the file live on the persistent `/data` volume; a restart
rebuilds the in-memory registry from the database and the upload remains
selectable.

The upload is deliberately restricted:

- one `.css` file, UTF-8, 256 KiB at most;
- at most 32 custom themes per instance;
- no external URL anywhere: `url(...)`, `image-set(...)` and a bare `https://`
  all count, and CSS escapes are decoded first, so `u\72l(` cannot smuggle one
  past the check;
- no `@import`, `expression(...)` or legacy `behavior`;
- served as `text/css` with `X-Content-Type-Options: nosniff`, so it cannot turn
  into an HTML document.

An upload also chooses how the palette behaves:

- **Follow system** keeps the ordinary light/dark control and follows the
  operating system until the visitor picks a mode.
- **Light only** and **Dark only** force `data-theme="light"`/`"dark"` on the
  server, hide the light/dark control, and cannot be overridden by a saved
  browser preference. This is the "pure white" / "pure black" choice.

Uploaded CSS is served **after** the Verdant stylesheet, so a small colour or
layout override composes with the default design and wins the cascade. The base
stylesheet is linked from the upload panel for reference. Custom uploads never
change the document templates: they can restyle every page, but cannot add
markup or scripts.

## How a visitor's theme is chosen

1. `urlet_theme` cookie, when it names an enabled theme (compiled in or uploaded).
2. Otherwise the administrator's default.
3. A cookie for a disabled or unknown theme is ignored and the default is used.

The picker is a native `<details>` element and works without JavaScript. Under
the `Everyone` policy a visitor who is not signed in sees it too, so a look
chosen on the landing page survives the sign-in that follows. The
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
fallback. A compiled-in theme directory may override any page:

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
    │   ├── link_splash.html   # public interstitial page
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
Uploaded themes select the Verdant wrapper at compile time and only replace the
stylesheet URL.

## Adding a compiled-in theme

1. Add a variant to `ThemeKey` in `src/services/theme.rs` (key, name key,
   description key, CSS URL, palette policy). A design that ships only one
   palette declares it through `is_dark_only`/`is_light_only`; `appearance()` is
   the single source of truth for the catalogue's `appearance` field and its
   light-only/dark-only badges.
2. Add the CSS under `src/assets/themes/<key>.css` and register it in
   `src/assets.rs` and the `/assets/themes/{key}` route.
3. Add the catalogue strings to the three `src/i18n/*.json` files.
4. Create `templates/themes/<key>/` with at least `base.html`, `app_base.html`,
   `landing.html`, the auth/link/splash/error pages, the admin tabs and the app pages.
   Reuse the content macros for the dense app pages. Load `theme.js` in `base.html`'s
   `<head>`, and render the picker and the light/dark switch exactly once per page: in
   the top bar of `landing.html` and `app_base.html`, and through
   `partials/site_tools.html` (switch, picker plus language switcher) wherever the design
   keeps its footer tools. A design that empties `footer_lang` for its own top bar must put
   the tools there instead, as Swiss's auth pages do. Copy the `data-theme`/`data-theme-lock`
   attributes of `templates/base.html` onto `<html>`, so a forced palette is marked on the
   document as well as in the catalogue.
5. Add `templates/themes/<key>/...` paths to every `themed_page!` call. Keep the
   `Verdant` entry in each invocation: it is also the fallback used when a page
   has not listed a newly added theme yet.
6. Add the theme to `ALL`, the migration seed and the admin tests. A new release
   can also rely on `db::themes::ensure_defaults` at startup for an existing
   database.
7. Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
   `cargo test` and `scripts/smoke.sh`.

## Deliberate limits

- Uploads are stylesheets, not templates. The CSP stays `default-src 'self'` and
  the binary stays self-contained; a custom theme cannot add markup or scripts.
- Uploads are administrator-only. There is no per-user theme marketplace or
  per-account upload quota; the site-wide catalogue stays the single source of
  truth.
- No remote theme fetch: the server never downloads a stylesheet, and external
  `url(...)` / `@import` are rejected in addition to being blocked by the CSP.
- No fine-grained per-account permission: the policy is site-wide (`all` or
  `admin`). The existing user-group feature toggles cover per-group data
  capabilities, not cosmetics.
- Every design except Aurora ships both palettes and the same persisted
  light/dark control; the control is a cosmetic preference, so it never needs a
  per-account column. Aurora is dark-only by design and has no light palette to
  offer, so it hides the control instead of shipping a dead one.
