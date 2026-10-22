# SNS preview implementation plan

Goal: let each signed-in link owner set a public SNS card title, description and image, using either an HTTPS image URL or an uploaded PNG/JPEG/GIF/WebP.
Architecture: retain the existing URL lifecycle and redirect handler; add one optional `url_previews` row per link. The preview service owns validation, the database module owns SQL, and shared routes own form/API operations. Uploaded bytes live in SQLite, with a new opaque key after replacement. Only recognised social crawlers receive metadata HTML; normal browsers keep existing redirects and splash/password flows.
Tech stack: Rust/Axum/SQLx/Askama, SQLite, existing HTML forms and trilingual catalogs; image 0.25.6 (pinned; image-webp requires Rust 1.80.1) with bounded decoding.
Baseline/Authority Refs: user requests on 2026-10-23, AGENTS.md, docs/SPEC.md sections FR-SPLASH and FR-PUB, docs/PLAN.md, src/db/mod.rs, src/routes/redirect.rs, src/services/ua.rs, src/middleware/security.rs. Required baseline references have been read; no missing requirement blocks execution. User explicitly authorises plan followed immediately by implementation and an independently reviewed PR.
Compatibility Boundary: never edit released migrations; never publish management title/notes/destination through preview HTML; links without custom preview and ordinary browser flows remain unchanged; lifecycle checks precede preview serving. Preview metadata and its uploaded image are explicitly public, including on password links. No remote scraping or fetching, guest uploads, release, deployment or merge.
Verification: cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test; bash scripts/smoke.sh; bash scripts/merge-check.sh; real HTTP multipart/metadata/image checks; independent subagent review.

## Readiness and architecture

Requirement Ready Check: ready. Title, description and both image-input modes confirmed. Defaults: title falls back only to site name; no destination metadata is fetched. Clearing all preview content disables custom previews.
Change Necessity: existing redirects carry no custom metadata and current settings cannot configure it; code changes are necessary in persistence, preview validation, HTTP delivery and the existing link-detail UI.
Existence Check: reuse URL ownership/lifecycle, existing middleware, link detail and shared templates. A separate optional table avoids adding image BLOBs to ordinary URL-list queries. One preview service is the canonical policy owner; no competing crawler parser or image storage subsystem.
Architecture Integrity Lens: generic analytics bot detection must not decide preview delivery (it includes curl, monitors and search crawlers). Add a narrow SNS classifier in the existing UA module. SQLite remains the sole new persistent storage owner.
Alternatives: redirect-only cannot supply custom metadata; serving HTML to all browsers changes existing redirect behavior; selected social-crawler responses preserve browser behavior. External-only images cannot satisfy the confirmed upload requirement; filesystem upload storage would require a second backup/storage contract.
Complexity Budget: add bounded db/service/route modules and one shared form partial instead of expanding the existing large URL management handlers. No source retirement: existing redirect, splash, password and metadata paths remain authoritative for their existing scenarios.
Execution Readiness View: intent and scope locked to custom public SNS cards; baseline locked to the exported local origin/master (4798f46). Inline execution is authorised. Test-first TDD is not requested; add meaningful integration tests and run the complete repository checks. Independent review is required before closeout. Initial worktree/fetch attempts were rejected while Git administration was read-only, so implementation and verification proceeded in /tmp/urlet-social-previews. After verifying repository identity, ADMIN access, the clean checkout and an ignored worktree path, the authorised worktree operation was approved. Integration now uses .worktrees/social-previews on feat/social-previews, based on cb21e88, with its own target/.

## Tasks

1. Persistence and validation
   - Add migrations/0016_url_previews.sql and register it in src/db/migrate.rs.
   - Add src/db/url_previews.rs: optional row, owner-checked atomic UPSERT, deletion, metadata-only reads and image lookup; replacement retires previous bytes/key.
   - Add src/services/url_preview.rs: title <=200 characters, description <=1000, HTTPS image URL <=2048 without credentials/control characters; uploads <=2 MiB and <=4096x4096, bounded raster decoding off the async worker. Keep/set/remove image are explicit operations.
   - Verify via image/validation unit tests and cargo test --test previews.
2. HTTP and user interface
   - Add src/routes/previews.rs: GET/PATCH/DELETE /api/v1/urls/{id}/preview; POST /api/v1/urls/{id}/preview/image (multipart); POST /app/urls/{id}/preview (multipart). Require owner authentication, CSRF and bounded bodies; rejected changes leave prior state untouched.
   - Add a shared preview-settings partial to the link-detail content for all six themes, trilingual labels, public-data/cache notice and errors. Support image URL, uploaded image, keep and remove; all-empty settings remove custom preview.
   - Preserve fields on failed submissions in a dedicated shared edit form; display success in the existing flash flow.
3. Crawler and media delivery
   - Add the narrow SNS classifier in src/services/ua.rs. Serve a standalone escaped OG/Twitter metadata template only for active links with custom content and a known SNS UA, before password handling; never include destination or private fields.
   - Emit no-store and Vary: User-Agent for preview-capable short-link responses. Preview fetches do not record clicks or spend click limits.
   - Serve uploaded bytes at /assets/link-previews/{opaque-key}, with verified MIME, nosniff, no-store and link lifecycle enforcement. Replacement/removal invalidates old keys.
4. Tests and documentation
   - Test normal redirects, known/unknown bots, protected/expired/disabled/deleted/capped links, escaped HTML, ownership/CSRF/limits, URL/upload replacement and clearing, old-key revocation, failed-save atomicity, persistence on reopen, all six themes and three locales.
   - Update docs/SPEC.md, docs/API.md, docs/PLAN.md and Traditional Chinese mirrors, plus README where useful. Explain upload limits, public previews and SNS cache limitations.
5. Verification and integration
   - Run all required checks and record output in the work checkpoint. Perform real HTTP checks, then start an independent reviewer against the source export and original baseline; fix all material findings and rerun affected checks.
   - Prepare an exact patch and PR title/body. Use feat/social-previews in the approved dedicated worktree; apply changes to the confirmed base, commit using repository gitmoji style, push and open a PR against cxyfer/urlet:master. Never merge.

## Risks, rollback and evidence

SNS user agents and third-party caches vary; local HTTP tests establish generated response behavior, not actual Discord rendering. Display makes no promise of a specific card layout or instant refresh. Uploaded image bytes enlarge SQLite (2 MiB per link maximum); backups must include the database as usual. The additive migration can remain when disabling the feature; do not edit or delete applied migrations or existing links. No credential values are recorded.

Independent review hardening: two global decode permits; parse WebP RIFF, canvas and nested VP8/VP8L/ANMF dimensions before decoding; scan GIF descriptors to keep every frame inside its bounded screen. The 64 MiB decoder buffer budget is not a process RSS ceiling. Raise the declared minimum Rust version from 1.80 to 1.80.1 for image-webp.

PR review follow-up: integrate master 5a0646b while retaining tracking cleanup; number previews as section 4.18 in both specifications. Add a trigger in the unreleased migration to remove preview data on first link soft-delete, including account deletion, while preserving previews on disabled/expired links and other owners. HTML submissions with an expired session redirect to login with the link-detail return path; API authentication is unchanged. Verify both fixes with integration tests and rerun the required checks.

## Administrator image policy follow-up

Add `preview_image_policy` to the existing seeded settings and administrator form/API: `none`, `url`, `upload`, `both` (default). One typed policy controls the owner editor, API/form writes, OG/Twitter image output and uploaded-image delivery. Reject prohibited new sources on the server before decoding; retain clear/keep and text-only edits. Hide prohibited source controls and explain the active policy. Changes suppress existing disallowed images without deleting stored data; restoring the policy restores delivery. Include all six themes and three locales, missing-field compatibility for older settings pages, strict value validation, persistence, access control, the four-source matrix and transitions in tests. Run the repository checks and independently review the updated PR. Reply to and resolve the existing soft-delete review thread using the verified fix and regression evidence.
