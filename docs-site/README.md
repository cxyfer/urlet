# urlet documentation site

The published documentation for [urlet](../), built with
[Fumadocs](https://fumadocs.dev) on Next.js and exported as static files.

This is a **separate artifact**. It is not part of the Rust build, it is not in
the container image, and the binary's own API reference at `/docs` does not
depend on it. The workflow in [`../.github/workflows/docs.yml`](../.github/workflows/docs.yml)
builds it and publishes `out/` to the `gh-pages` branch.

## Where the content comes from

`content/docs/` is **generated and gitignored**. The repository's
[`../docs/`](../docs) tree is the single source of truth; `scripts/sync-content.mjs`
copies the published documents into it on every build and does three things the
site needs and the repository should not carry:

- it writes one directory per locale, because Fumadocs resolves a locale by
  directory while the repository keeps English at the root of `docs/` and
  Traditional Chinese in `docs/zh-TW/`;
- it derives each page title from the document's own first heading, so the
  thirteen repository files need no frontmatter;
- it rewrites the links between documents to site routes, and unwraps the ones
  that point at something with no page here.

Only the documents named in that script are published. The specification, the
plan, the release contract and the `aegis/` records are internal and stay out by
being absent from the list, rather than through a filter someone can forget.

## Working on it

```bash
npm ci
npm run dev      # syncs the content, then serves on http://localhost:3000
npm run build    # syncs the content, then exports to out/
npm run preview  # serves the exported out/
```

`npm run sync` runs the content sync on its own; `dev` and `build` run it first.

## Layout

| Path | What it is |
| ---- | ---------- |
| `scripts/sync-content.mjs` | Copies `../docs` into `content/docs/`; the only place the published set is defined |
| `lib/i18n.ts` | The two languages and the i18n behaviour |
| `lib/i18n-ui.ts` | The interface strings Fumadocs does not ship for Traditional Chinese |
| `lib/source.ts` | The documentation collection and the loader the pages read |
| `app/[lang]/` | Every route, under a locale segment |
| `public/index.html` | Sends the bare root to `/en`; a static export cannot redirect |

## Notes

- Both languages carry their prefix (`/en/docs`, `/zh-TW/docs`). Next.js does not
  run middleware in a static export, so Fumadocs' `hideLocale` cannot hide the
  default one.
- `trailingSlash` is on, so every route is a directory with an `index.html`,
  which is what a plain static host expects.
- `basePath` is the repository name, because GitHub Pages serves a project site
  from `https://<owner>.github.io/<repo>/`. Without it every asset and route in
  the export points at the domain root, which resolves to another site's path:
  the pages load unstyled, never hydrate, and every internal link 404s. A custom
  domain sets it to an empty string and adds a `CNAME` to `public/`.
- The search index is a **route handler**, so it stays at `app/api/search/`
  rather than under `app/[lang]/`. The client fetches one path for every locale;
  under a locale segment the index was exported to `/en/api/search` and
  `/zh-TW/api/search`, which nothing requests. One index covers both languages —
  exporting it per locale only duplicated 1.6 MB.
- The build makes no third-party request: no web font is loaded, and the syntax
  highlighter runs at build time. `env` fences are written as `ini`, the closest
  grammar Shiki ships.
- GitHub Pages has to be switched on for the repository, with `gh-pages` as the
  source. The workflow publishes the branch; it cannot change that setting.
