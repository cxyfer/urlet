# Checkpoint

Implemented: additive migration 0016; SQLite preview persistence and image BLOBs; validation, owner-only form/API, six-theme editor, three catalogs, OG/Twitter delivery and lifecycle-bound public media. Independent review findings were fixed: WebP internal dimensions, GIF frame boundaries, global decode concurrency, negative media caching, CSS hidden specificity, MSRV, audit-test freshness, soft-delete image cleanup and HTML login recovery.

Final full verification after integration with master b36a1e3 passed: 577 tests, two existing ignored, 192 smoke checks and 42 group/domain checks, plus formatting/Clippy/build. Browser checks passed all four image-source states in six themes, external URL/upload submissions and a 390px Traditional Chinese viewport without overflow.

Task worktree: .worktrees/social-previews, feat/social-previews. PR #35 targets master. The newer tracking-cleanup, admin-tab and guest destination allowlist changes are preserved; all three locale conflicts retain both sets of keys. Independent implementation and PR reviews examined the fixes without remaining actionable issues. The branch follows the authorised commit/push/PR workflow; no merge into master or deployment is included.

Administrator image-policy follow-up is complete: four modes apply to form/API writes, metadata and media delivery, with reversible suppression of stored images. All 17 preview tests passed, including settings access, atomic validation, reload/old forms, every policy transition and six-theme/three-locale rendering. Browser controls and a 390px Traditional Chinese viewport passed; an independent HTTP/SQLite matrix passed. The existing soft-delete PR review thread was replied to and resolved.

The user authorised retrying the unavailable reviewer and, if unsuccessful, switching to deepseek-v4.1-flash. The model catalog override was updated and a fresh catalog read verified it; automatic approval remains enabled. Commit f856b4d and its push succeeded through the existing approval mechanism; tool output does not reveal the actual reviewer model for those requests. The existing PR review thread is resolved.
