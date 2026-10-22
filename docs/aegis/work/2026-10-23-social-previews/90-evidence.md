# Evidence

Validated source export: /tmp/urlet-social-previews, based on 4798f46. Final checks ran after the GIF/WebP/audit fixes, with separate target/ and CARGO_INCREMENTAL=0. A writable Cargo registry copy was used because the host registry is read-only. No shared Cargo build directory or hard-linked build cache was used.

- cargo fmt --check: pass.
- cargo clippy --locked --offline --all-targets -- -D warnings: pass.
- cargo test --locked --offline: 543 passed, zero failed, two existing ignored (live provider HTTPS and template macro documentation).
- cargo build --locked --offline: pass.
- bash scripts/smoke.sh: 177 passed, zero failed.
- bash scripts/merge-check.sh: 42 passed, zero failed.
- node --check src/assets/app.js: pass.
- Eleven new integration tests include valid/malformed rasters, malicious first/second GIF frames, ownership/CSRF/limits, metadata escaping/privacy, image replacement, persistence, six-theme/three-locale rendering, custom domains and browser flows. Two WebP unit tests cover oversized hidden frames and truncated RIFF.
- Browser: all six themes and four image-source choices have correct computed display and input disabled state. Both HTTPS URL and uploaded WebP submissions persisted. A 390px Traditional Chinese viewport had document width 390px and no horizontal overflow. Actual HTTP Discordbot response contained the saved OG/Twitter title, description and uploaded-image URL.
- Independent read-only reviewer found no remaining actionable issues after re-review. Its standalone decoder harness accepted nine valid WebP variants and valid animated GIF; invalid GIF first/second frames were rejected. It did not independently run the Cargo suite or third-party SNS rendering.
- GitHub API confirmed cxyfer/urlet, ADMIN permission and master cb21e8836f3ed340abeb67a395c74284012ea03a. The feature patch passes git apply --check against both 4798f46 and cb21e88, and the primary worktree remains clean.

Logs and browser screenshots are temporary local artifacts, not product files. Actual third-party card layout, image acceptance and cache refresh remain unverified. Minimum declared Rust version is 1.80.1; checks used the installed toolchain rather than an MSRV toolchain.

Final integration verification: .worktrees/social-previews, feat/social-previews, based on cb21e88. Fresh target/ passed formatting, Clippy, 545 Cargo tests (two existing ignored), cargo build, 177 smoke checks, 42 group/domain checks, node --check and git diff --check. The two extra tests come from the newer upstream baseline. The authorised worktree operation succeeded after the repository/access checks. Incremental compilation was disabled to reduce build artifacts, trading later recompilation speed for disk space.
