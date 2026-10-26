# Checkpoint

Plan: docs/aegis/plans/2026-10-26-api-documentation.md. Baseline master d2d341d. Worktree .worktrees/api-docs on feat/api-docs; the three stages use stacked branches (feat/api-docs -> feat/api-docs-site -> feat/api-docs-drift) because stage 2 consumes stage 1's contract.

Environment note: the host file sandbox rejects writes outside the session workspace, so the shared cargo registry at ~/.cargo is read-only here. A workspace-local CARGO_HOME was seeded from it at .cargo-home (already gitignored); every cargo command in this worktree must export CARGO_HOME=/data/home/workspace/urlet/.cargo-home. PyYAML is not installed either, so the generator was run with PYTHONPATH pointing at a workspace-local install. Both are environment workarounds, not repository changes.

Todo map:
1. PR 1 - contract sources, generator, vendored Scalar, /docs route, slug reservation and startup warning, footer entry point, tests, bilingual docs, Dockerfile COPY. IN PROGRESS.
2. PR 2 - docs-site scaffold, external content dir via widened Turbopack root, frontmatter, locales, fumadocs-openapi wiring, docs.yml to gh-pages.
3. PR 3 - contract-to-router coverage test, spec lint, drift boundary documentation.

PR 1 implemented: `openapi/components.yaml` (64 schemas, 9 responses, 2 security schemes), `openapi/en.yaml` and `openapi/zh-TW.yaml` (40 paths / 57 operations each, identical operation sets and operationIds), `scripts/build-openapi.py` merging them into two committed self-contained specs, `scripts/update-scalar.sh` pinning Scalar 1.76.0 by digest, the vendored `src/assets/scalar.js`, `src/assets/scalar-init.js`, `src/routes/docs.rs` serving `/docs` and both contracts, the `docs` reserved slug with a startup report for a shadowed live link, and the footer entry point across seven templates and three catalogues. `src/db/urls.rs` gained `count_live_with_slug`; the Dockerfile copies `openapi/`; `.dockerignore` now excludes `.worktrees`; CI gained a contracts job.

Two findings came from browser verification rather than from the tests, and both are fixed:
- The mount element used `id="api-reference"` with `data-spec-url`, which triggers Scalar's deprecated HTML auto-detection. Scalar then configured itself and ignored the options passed in, so `withDefaultFonts: false` never applied: the page still referenced fonts.scalar.com and the CSP blocked fifteen font loads. The element is now `id="urlet-api-reference"` with `data-openapi-url`, which the init script reads explicitly. Re-verified: zero third-party hosts, zero console errors.
- The contract named `https://sho.rt` as its server, so the reference's Test Request would have sent a visitor's credentials to a domain nobody in this project controls. The server is now the relative `/`, which resolves to the instance serving the contract.

Also from verification: the Chinese contract's group names were left in English, which would have made the reference sidebar English in the Chinese page. All nine declarations and fifty-seven operation references are now translated, and the test asserts both documents declare the same number of groups, reference only declared groups, and carry different group names.

Verification so far: `cargo fmt --check` clean; `cargo clippy --all-targets -- -D warnings` clean; `cargo test` green across every test binary (ten new tests in tests/api_docs.rs); `scripts/build-openapi.py --check` reports the committed specs match their sources; `scripts/smoke.sh` 209 passed / 0 failed; `scripts/merge-check.sh` 42 passed / 0 failed. Browser verification of `/docs` in English and Traditional Chinese: reference renders, no third-party host contacted, no console error, server selector shows the serving instance. Docker image built and measured against a `master` baseline: 28.99 MiB before, 35.47 MiB after, a 6.48 MiB increase of which the vendored Scalar bundle is 4.9 MB. The image stays under the 40 MB budget but now uses most of it; the SPEC's size table was stale at v1.1 and has been refreshed with the current numbers.

Independent review found two blocking issues and several smaller ones. Both blockers are fixed:

- `/docs` answered 200 on an extra domain while `/docs/openapi.json` answered 404, because the domain gate treats a one-segment path as a redirect but rejects a two-segment one. The footer links the reference from the extra-domain short landing too, so the reference rendered broken there. `/docs` and `/docs/*` are now host-agnostic, alongside the health probes and the assets. Because the API itself stays canonical-only, the contract now names the configured base URL as its server, substituted at serve time from a placeholder the generator leaves in place; otherwise the reference's Test Request would target a host that answers 404 for every `/api/v1` path. Two tests and four smoke checks cover both.
- The `components.yaml` header documented a `$ref` shape the generator cannot rewrite, so an author following it would have shipped an unresolved reference while `--check` passed. The comment is corrected, the generator now fails on any document-local reference that does not resolve, and it also fails on an unexpected file left in `dist/`.

Also fixed from the review: the version is no longer hardcoded in five places (the generator expands a token from `Cargo.toml`, so a version bump that is not regenerated fails CI, and `RELEASING.md` and its mirror say so); `docs/zh-TW/PLAN.md` gained the P16 row it was missing; the `sort` parameter now documents the direction-suffixed aliases the handler accepts; and the plan's stale endpoint paths and determinism note are corrected.

Two review findings did not survive checking. The claim that Aurora's footer pushes the repository link right, leaving the new link behind, does not reproduce: measuring all six themes at 1200px shows the API link immediately before the repository link everywhere (gap 9.6-24px), and Aurora's repository link is not right-aligned. The missing route-coverage test is the deliberate split the plan records: stage 3 owns it.


## PR 2 — published documentation site

Two constraints discovered while building it, neither of which the plan anticipated:

- Fumadocs resolves a locale by directory, so every language needs its own folder. The repository keeps English at the root of `docs/` and Traditional Chinese in `docs/zh-TW/`, which does not match. The plan's "content stays in place, no sync step" option is therefore impossible.
- Next.js does not run middleware in a static export, so Fumadocs' `hideLocale` cannot rewrite `/docs` to `/en/docs`.

Both are handled by generating the content tree instead of restructuring the repository: `docs-site/scripts/sync-content.mjs` runs as a prebuild step, copies the published documents into `content/docs/<locale>/`, derives each page title from the document's own first heading — so the thirty repository files need no frontmatter — and rewrites the links between documents. The generated directory is gitignored, so the repository still holds exactly one copy. `hideLocale` stays `never`: both languages carry their prefix, and `public/index.html` sends the bare root to English.

Implemented: the `docs-site/` project with both locales, search, per-page markdown export and OG images; the sync script; `trailingSlash` so a plain static host resolves every route; `.dockerignore` excluding `docs-site/`; and `.github/workflows/docs.yml`, path-filtered and publishing to `gh-pages` with a post-build check that the export contains both locales.

Four build failures were real and are fixed: Shiki has no `env` grammar (the deployment guide's `.env` fences are now `ini`, which is what a dotenv file is); `i18n.parser` defaults to `dot`, so the locale directory was becoming part of every slug and each page was generated twice under a doubled prefix; `I18nProvider` must be reached through `RootProvider` rather than rendered above it, or it calls the framework hooks before the provider that supplies them exists; and static-export route handlers each need `generateStaticParams`. A fifth was cosmetic but user-visible: the sync's link rewrite left bare brackets where a document pointed at the repository's `openapi/` directory, and now unwraps that link to its text.

Verified: `next build` exports 34 pages; every route answers 200 from a static server; the pages render in both languages with translated sidebar and headings, the language switcher is present, sidebar navigation works, and the browser reports no third-party host and no error beyond Next's RSC prefetch misses, which a static export cannot answer. `npm ci` is consistent with the lockfile, which the workflow relies on.


## PR 3 — contract drift protection

`tests/contract.rs` reads the router's registered surface from the router's own source and asserts set equality with both contracts. axum exposes no way to read a built router back, so the `.route(...)` calls are the registration; the parse fails on a shape it does not understand, and a size guard makes a parser that silently matched nothing fail rather than pass. The check was verified to have teeth: adding `.route("/undocumented-probe", get(...))` fails both contract tests and names the offending method and path.

The lint the plan called for turned out to be worth more than expected. Redocly's `minimal` ruleset flagged four components that nothing referenced, and checking each against the handlers showed they were not oversights in the wiring but genuine mistakes in the contract: `OverviewDto` and `TopLinkDto` are dashboard payloads for the HTML page and were never API responses, `OkResponse` had no handler at all, and `GET /api/v1/admin/audit` returns `Page<AuditDto>` while the contract referenced the generic `Page` and left the item type in prose. The three unused schemas are removed, `AuditPage` is added and referenced from both languages, and both documents now lint clean.

## Independent review of PR 2 and PR 3

The review found two blocking defects in the documentation site, both of which the build and the browser checks had missed, and both are fixed and re-verified:

- Search was dead on the published site. The index is a route handler and had been placed under `app/[lang]/`, so the export produced `/en/api/search` and `/zh-TW/api/search` while the client fetches one `/api/search` for every locale. The two locale files were also byte-identical, so the fix removes 1.6 MB of duplication from the export.
- The site was broken at the URL the README advertises. GitHub Pages serves a project site from `/<repo>/`, and without `basePath` every asset and route in the export pointed at the domain root: unstyled, unhydrated, and every internal link dead.

The workflow's verification step only asserted that five files existed, which is why it passed while both defects were present. It now checks the search index path and the asset prefix as well.

Also fixed from the review: the content sync fails on a relative link with no site equivalent and on a document with no level-one heading outside a code fence, naming the file; `GET /api/v1/admin/groups` had the same generic-`Page` defect as the audit operation and now has a `GroupPage`; and the contract test's coverage had a real false-negative hole — a module reached through `merge` or `nest` was invisible, so an undocumented route there would have passed. `every_merged_router_is_read` fails and names the module, and the size guard now compares distinct paths with `.route(` calls, which is the comparison that actually holds (one call can register several methods). Both guards were verified by defeating them.

The review also noted that nothing builds `docs-site/` on a pull request, so a broken site is found after merge. That remains open and is recorded in the pull request.

Two parser robustness issues the review raised as "noisy but safe" are also fixed, because a false positive is still a test that fails on correct code: whole-line `//` comments are stripped before parsing, so a commented-out `.route(...)` is no longer a phantom registration, and `.nest(` now fails with a message saying the parser does not read a nest prefix rather than silently producing wrong paths. The registration-count guard had to move to the same comment-stripped text; on the raw source the two counts disagreed the moment a commented call was added.

Status after the review: the stack was rebased onto the current `master` (`33f5bae`, which had moved while the work was in progress and touched the same specification file) with both sides' changes preserved and no conflict. fmt, clippy, `build-openapi.py --check`, 616 tests and smoke (209) are green. PR 1 is on feat/api-docs (65e2d07), PR 2 on feat/api-docs-site (8217d83), PR 3 on feat/api-docs-drift (e680456). No branch is pushed: the `gh` token lacks the `workflow` scope, and an interactive refresh is waiting on the user.
