# API documentation implementation plan

Goal: give urlet browsable, bilingual API documentation in three separately reviewable stages. (1) An OpenAPI 3.1 contract plus a Scalar reference embedded in the single binary and served at `/docs`. (2) A Fumadocs site built from the same sources and published on a `gh-pages` branch. (3) Drift protection that keeps the contract and the axum router in step.

Architecture: the contract is authored once under `openapi/` as a language-neutral `components.yaml` plus one `paths` file per language; a generator inlines them into self-contained specs that the binary embeds and the docs site consumes. The binary serves its own reference: `/docs` returns the Scalar page and `/docs/openapi.json` and `/docs/openapi.zh-TW.json` return a spec. Scalar is vendored as one file and makes no runtime network request. Scalar owns its own stylesheet, which satisfies the requirement that the reference look independent of the site themes; the entry point reuses the existing shared footer partial rather than adding a second navigation owner. The Fumadocs site lives in `docs-site/` on `master` and is published from CI to `gh-pages`; it never enters the Docker build context.

Tech stack: Rust/Axum/Askama/SQLite unchanged. OpenAPI 3.1. Scalar `@scalar/api-reference` 1.76.x, MIT, vendored. Python 3 generator in `scripts/` (same category as the existing `scripts/screenshot.py`; a development tool, not a build dependency). Fumadocs 16 / Next.js 16 for the site, development-only.

Baseline/Authority Refs: user decisions 2026-10-26 (Scalar in the binary; `/docs`; bilingual spec sharing components; Fumadocs source in `docs-site/`; Fumadocs output on `gh-pages`); AGENTS.md; docs/aegis/BASELINE-GOVERNANCE.md; docs/SPEC.md (P5 "no Node build", §4 API); docs/PLAN.md; src/server.rs; src/routes/api.rs; src/routes/dto.rs; src/services/slug.rs; src/assets.rs; src/middleware/security.rs; src/i18n/*.json; templates/partials/repo_link.html and the six base templates; Dockerfile; .dockerignore; .github/workflows/ci.yml; docs/API.md and docs/zh-TW/API.md. Required baseline references have been read; no missing requirement blocks execution. The user authorises the plan followed immediately by implementation and an independently reviewed PR for each stage.

Compatibility Boundary: `/docs` becomes a reserved path. A short link whose slug is already `docs` becomes unreachable, because axum matches the static segment first. Mitigation is reserve-and-disclose, not silent migration: `docs` joins `RESERVED`, startup logs a warning when a live link still holds that slug, and the release notes name the change. No link is renamed, deleted or rewritten. The JSON API, redirect, splash, password and theme behaviour are unchanged. The `/docs` page and its spec endpoints are public and unauthenticated, matching the existing public API surface; they expose no credential and no per-user data. No release, tag, merge or deployment is authorised by this plan.

TDD Route: mode `auto`; decision `strict` for the behaviour-bearing slices (HTTP handlers, slug reservation, contract-to-router coverage, generator determinism) because this work adds a public contract, a new HTTP surface and a compatibility change. The spec's prose content is authored data, verified by schema validity, route coverage and generator determinism rather than by RED/GREEN steps. Authority: no explicit user TDD request; the `auto` decision itself is recorded here.

Verification: `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test`; `bash scripts/smoke.sh`; `bash scripts/merge-check.sh`; real HTTP checks of `/docs` and both spec endpoints; generator rerun produces no diff; Docker image builds and stays under the 40 MB budget; independent subagent review before each PR.

## Readiness and architecture

Requirement Ready Check: ready. The three stages, the serving path, the spec languages, the docs-site location and the publish target are all confirmed by the user. Defaults taken and disclosed: the bilingual contract uses a generator (the alternative, runtime `$ref` rewriting, was rejected as less robust because it depends on `base_url` correctness); `docs/API.md` stays hand-written as the narrative reference while the spec becomes the machine contract, with path coverage enforced by a test; the Fumadocs content stays in `docs/` and is reached by widening the Turbopack root, avoiding a copy/sync step.

Change Necessity: no documentation surface exists that a machine can consume, and the binary serves no reference page. Documentation-only or config-only change is insufficient because the deliverable is an HTTP surface inside the shipped artifact plus a contract other tools can read. The minimum code boundary is one new route module, one generator script, one reserved-word entry, one vendored asset and its embedded accessor.

Existence Check: reuse the existing asset embedding (`src/assets.rs`, `include_str!`), the existing router registration in `src/server.rs`, the existing reserved-slug list, the existing shared footer partial and `theme-ui.css`, the existing trilingual catalogs, and the existing startup-warning pattern (`warn_about_first_user`). No new storage, no new middleware, no second navigation owner, no second error format. Scalar is a rendering dependency only; it owns no urlet state.

Architecture Integrity Lens: the contract must have exactly one source. `openapi/` is that source; the generated specs and the Fumadocs pages are derived artifacts and must never be edited by hand. The generated specs are committed because `include_str!` needs them at compile time; a CI step regenerates and fails on any diff, so the derived artifact cannot silently diverge. The route-coverage test in stage 3 compares the axum router against the spec's `paths`, which is the only mechanism that catches a handler added without a contract entry.

Alternatives: (a) annotate all ~40 handlers with `utoipa` — rejected: the prose documentation already exists, and annotations add a permanent two-place maintenance tax plus a known axum/utoipa version-pairing hazard. (b) Hand-write two fully self-contained specs and assert their `components` are equal — simpler operationally but edits schemas twice; rejected in favour of the generator, which the user selected by choosing shared components. (c) Serve the spec with a placeholder rewritten to an absolute base URL at request time — rejected because it makes the contract depend on `base_url` correctness. (d) Publish the Fumadocs source on a long-lived branch — rejected: no clean Git mechanism exists for a branch that only absorbs part of `master`.

Complexity Budget: PR 1 adds one route module, one generator script, one update script and one vendored asset, and touches the reserved list, the asset accessor, the router, the footer partial across six templates and the trilingual catalogs. PR 2 adds a self-contained Node project under `docs-site/` and one workflow; it touches no Rust file. PR 3 adds one test module and one CI step. No existing owner is split; no fallback or adapter is introduced.

Execution Readiness View: intent locked to the three stages above; scope fence is the files listed under Tasks. Baseline locked to `master` `d2d341d`. Compatibility boundary locked to reserve-and-disclose for the `docs` slug. Execution is inline; test-first applies to the slices named in the TDD Route. Independent review is required before each PR. Branches are stacked because stage 2 consumes stage 1's contract: `feat/api-docs` from `master`, then `feat/api-docs-site` from `feat/api-docs`, then `feat/api-docs-drift` from `feat/api-docs-site`. Each PR is opened against its predecessor and retargeted to `master` once that predecessor merges.

## Tasks

### PR 1 — Contract and in-binary reference

1. Contract sources
   - Add `openapi/components.yaml`: schemas for every DTO in src/routes/dto.rs, the shared error envelope and its nine codes, and the two security schemes (session cookie, bearer token).
   - Add `openapi/en.yaml` and `openapi/zh-TW.yaml`: `info` and all 34 `paths` with per-language summaries, parameter descriptions and response descriptions, referencing `components.yaml`.
   - Verify by hand against docs/API.md and src/routes/api.rs.
2. Generator
   - Add `scripts/build-openapi.py`: load the three sources, inline the shared components, write `openapi/dist/urlet.en.json` and `openapi/dist/urlet.zh-TW.json` deterministically (source key order, no timestamps), expand the version token from `Cargo.toml`, and fail on any reference that does not resolve.
   - Add the generated files to the repository and a CI step that reruns the generator and fails on any diff.
   - Verify: rerun produces byte-identical output.
3. Vendored Scalar asset
   - Add `src/assets/scalar.js` (the single self-contained browser bundle) and `scripts/update-scalar.sh`, which pins the version, downloads the bundle and verifies a recorded SHA-256.
   - Verify: the bundle loads with no sibling chunk request.
4. HTTP surface
   - Add `src/routes/docs.rs`: `GET /docs` returns the reference page, `GET /docs/openapi.json` returns the English spec and `GET /docs/openapi.zh-TW.json` the Traditional Chinese one, each with an explicit content type and a long-lived cache header. The contract names the configured base URL as its server, substituted into a placeholder the generator leaves in place: the API is canonical-only while the reference is served from every host its footer appears on.
   - Register the routes in src/server.rs and the embedded specs and bundle in src/assets.rs. Initialise Scalar with `withDefaultFonts: false` so no request leaves the instance.
   - Verify: `/docs` returns 200 with an HTML document; both spec endpoints return 200 with parseable JSON; no external host is referenced.
5. Compatibility
   - Add `docs` to `RESERVED` in src/services/slug.rs.
   - Extend startup with a warning when a live short link still holds the `docs` slug, mirroring `warn_about_first_user`.
   - Verify: creating or updating a link with slug `docs` is rejected with a validation error; the warning fires only when such a link exists.
6. Entry point
   - Add a shared footer partial for the API reference and include it from the six base templates and Gilded's separate landing footer; add the shared rules to `theme-ui.css`; add the label to all three catalogs.
   - Verify: every theme renders the link in all three locales; the existing theme tests stay green.
7. Tests and documentation
   - Add integration tests for the page, both spec endpoints, the reserved slug, the startup warning, and spec-to-router path coverage for the routes that exist at this stage.
   - Add `COPY openapi ./openapi` to the Dockerfile; confirm the Docker build still succeeds and the image stays under budget.
   - Update docs/API.md and docs/zh-TW/API.md (link to `/docs`), README.md, docs/SPEC.md and its mirror, docs/PLAN.md, and add `docs-site/` to .dockerignore.
8. Verification and delivery
   - Run every required check; run the real HTTP checks; obtain independent review; fix material findings; commit, push `feat/api-docs` and open the PR against `master`. Never merge.

### PR 2 — Published documentation site

1. Site scaffold
   - Add `docs-site/` from the Fumadocs static template, pinned to the versions recorded in its lockfile.
2. Content wiring
   - Widen the Turbopack root to the repository root and point the docs collection at `../docs`, so no content is copied and no sync step exists.
   - Add frontmatter `title` to every document that is published; exclude the internal `aegis/` tree and the specification and plan documents from the published set.
3. Localisation and contract
   - Configure the `en` and `zh-TW` locales and supply the interface strings the framework does not ship.
   - Wire `fumadocs-openapi` to the generated specs so the site and the binary publish the same contract.
4. Publication
   - Add `.github/workflows/docs.yml`, triggered only by changes under `docs-site/`, `docs/` or `openapi/`, that builds the site and publishes it to `gh-pages`.
   - Verify: the workflow produces the site; the published pages resolve; the language switch works.
5. Documentation and delivery
   - Document the site, its build and its publication in README and the bilingual docs; obtain independent review; commit, push `feat/api-docs-site` and open the PR against `feat/api-docs`. Never merge.

### PR 3 — Drift protection

1. Contract-to-router coverage
   - Add a test that records every `(method, path)` the axum router registers and asserts set equality with the spec's `paths`, failing on either a missing or an undocumented route.
   - Verify: the test fails when a route or a contract entry is removed.
2. Contract lint
   - Add a pinned lint step for the generated specs so structural regressions fail CI.
3. Documentation and delivery
   - Record the drift boundary in docs/SPEC.md and its mirror; obtain independent review; commit, push `feat/api-docs-drift` and open the PR against `feat/api-docs-site`. Never merge.

## Risks, rollback and evidence

The `docs` slug reservation can shadow one existing link per instance. The warning makes it visible at startup and the release notes name it; the change is reversible by removing the route, and no data is altered either way. The vendored Scalar bundle adds roughly 4.9 MB to the binary, moving the image from about 14.8 MB to about 19.7 MB against a 40 MB budget; the asset is one file with no sibling chunk, verified by loading it in isolation. Scalar is MIT-licensed and its paid tiers cover only the hosted platform, not the self-hosted reference. Scalar's own stylesheet means the page does not follow the instance theme; that is the accepted requirement, not a defect, and `withDefaultFonts: false` keeps the page free of third-party requests. The Fumadocs dependency tree is large and development-only; it never enters the Docker context. The generator's committed output can drift from its sources if the CI check is removed, which is why that check is part of stage 1 rather than stage 3. No credential value is recorded anywhere in this work.
