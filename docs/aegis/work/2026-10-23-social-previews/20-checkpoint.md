# Checkpoint

Implemented: additive migration 0016; SQLite preview persistence and image BLOBs; validation, owner-only form/API, six-theme editor, three catalogs, OG/Twitter delivery and lifecycle-bound public media. Independent review findings were fixed: WebP internal dimensions, GIF frame boundaries, global decode concurrency, negative media caching, CSS hidden specificity, MSRV, audit-test freshness, soft-delete image cleanup and HTML login recovery.

Final full verification after integration with master 5a0646b passed: 558 tests, two existing ignored, 177 smoke checks and 42 group/domain checks, plus formatting/Clippy/build. Browser checks passed all four image-source states in six themes, external URL/upload submissions and a 390px Traditional Chinese viewport without overflow.

Task worktree: .worktrees/social-previews, feat/social-previews. PR #35 targets master. The newer tracking-cleanup feature and admin-tab fix are preserved; conflicting bilingual specification sections are resolved. Independent implementation and PR reviews examined the fixes without remaining actionable issues. The branch follows the authorised commit/push/PR workflow; no merge into master or deployment is included.
