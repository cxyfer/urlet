# Evidence

Validated task worktree: .worktrees/social-previews, branch feat/social-previews, integrated with master 5a0646b (tracking parameter cleanup). Both specification sections are retained; previews are section 4.18. Required checks ran after the final decoder, deletion-lifecycle and expired-session fixes, using an independent target/ and CARGO_INCREMENTAL=0. A writable Cargo registry copy was used because the host registry is read-only. No shared Cargo build directory or hard-linked build cache was used.

- cargo fmt --check: pass.
- cargo clippy --locked --offline --all-targets -- -D warnings: pass.
- cargo test --locked --offline: 558 passed, zero failed, two existing ignored (live provider HTTPS and template macro documentation).
- cargo build --locked --offline: pass.
- bash scripts/smoke.sh: 177 passed, zero failed.
- bash scripts/merge-check.sh: 42 passed, zero failed.
- node --check src/assets/app.js and git diff --check: pass.
- Thirteen new integration tests cover valid/malformed rasters, malicious first/second GIF frames, ownership/CSRF/limits, metadata escaping/privacy, image replacement, persistence, six-theme/three-locale rendering, custom domains and browser flows. Link/account soft-deletion removes preview rows and images while preserving other owners and reversible lifecycle states. A signed-out form redirects to login with the link-detail return path without writing data. Two WebP unit tests cover oversized hidden frames and truncated RIFF.
- Browser verification on the original feature source: all six themes and four image-source choices have correct computed display and input disabled state. Both HTTPS URL and uploaded WebP submissions persisted. A 390px Traditional Chinese viewport had document width 390px and no horizontal overflow. Actual HTTP Discordbot response contained the saved OG/Twitter title, description and uploaded-image URL.
- Independent implementation review found no remaining actionable issues after re-review. Its standalone decoder harness accepted nine valid WebP variants and valid animated GIF; invalid GIF first/second frames were rejected. Independent PR review identified retained image BLOBs after soft-delete and the HTML handler's JSON authentication error; the fixes and integration diff were reviewed without further actionable issues. Reviewers did not independently run the full Cargo suite or third-party SNS rendering.
- GitHub API confirmed cxyfer/urlet, ADMIN permission and master 5a0646b2bf2959e1f16b3331cd694eb97621600f. The authorised task worktree, branch and PR #35 were created. No merge into master or deployment was performed.

Logs and browser screenshots are temporary local artifacts, not product files. Actual third-party card layout, image acceptance and cache refresh remain unverified. Minimum declared Rust version is 1.80.1; checks used the installed toolchain rather than an MSRV toolchain. Incremental compilation was disabled to reduce build artifacts, trading later recompilation speed for disk space. SQLite can reuse deleted image pages but does not automatically shrink its file.

## Administrator image policy verification

After adding preview_image_policy=none/url/upload/both, fresh checks passed on the task worktree: fmt, Clippy with warnings denied, 562 Cargo tests (zero failed, two existing ignored), build, 177 smoke checks and 42 group/domain checks. All 17 preview integration tests passed. The existing checksum-protected migrations were unchanged by this follow-up; startup seeds the new setting only when missing.

Browser evidence: the administrator form saved none, the owner editor exposed only the permitted source options in all four modes, active source fields had correct visibility/disabled state, and a 390px Traditional Chinese viewport had no overflow. Screenshots and checks are under /tmp/urlet-preview-policy-ui. An independent HTTP/SQLite harness checked all four policies across API/form writes, OG metadata, asset status/no-store, DTO behavior and unchanged stored image data after switching. No remaining actionable review findings. The existing soft-delete thread PRRT_kwDOVLtob86vW_AP was replied to with a244478 evidence and resolved; actual third-party SNS cache/rendering and MSRV remain outside verification.
