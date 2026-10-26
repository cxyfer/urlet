# Agent notes

Instructions for AI agents working in this repository. They complement
[`RELEASING.md`](RELEASING.md), which is the release contract, and [`README.md`](README.md).

## What this is

urlet is a self-hosted, multi-user URL shortener: one Rust binary (Axum, SQLx, Askama) and one
SQLite file. No JavaScript build step, no external database, no cache. Read
[`docs/SPEC.md`](docs/SPEC.md) before changing behaviour and [`docs/PLAN.md`](docs/PLAN.md) for
where the work is heading.

## Before you call a change done

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
bash scripts/smoke.sh        # builds and drives the real binary over HTTP
bash scripts/merge-check.sh  # the merged group and domain flows
```

The first four run on every pull request — see
[`.github/workflows/ci.yml`](.github/workflows/ci.yml) — and on every push to `master` that a
green pull request did not already cover. A push that merges a passing pull request is skipped;
direct pushes, force pushes and merges of untested changes still run in full. `merge-check.sh`
stays a post-merge check for the group and domain flows.

A fifth job covers the OpenAPI contracts. Changing anything under `openapi/` means regenerating
them with `python3 scripts/build-openapi.py` — it needs PyYAML — and committing the result; the
job reruns the same script with `--check`, which fails when the committed documents are stale,
and lints both with Redocly. The contract-to-router tests in `tests/contract.rs` run with
`cargo test` like any other.

Adding a job to `ci.yml` means adding its `name:` to the gate's `REQUIRED` list.
`tests/ci_workflow.rs` checks that the two agree, so `cargo test` fails rather than letting a pull
request drop a job and still look tested.

Schema changes go in `migrations/` and run at startup. Migrations are additive and
checksum-protected: never edit one that has already been released, add a new file instead.

## Commit messages

Gitmoji subject, imperative, scoped — `✨ feat(groups): ...`, `🐛 fix(domains): ...`,
`📝 docs: ...`, `👷 ci: ...`. Merged pull requests keep their `(#N)` suffix in the subject;
[`scripts/release-notes.sh`](scripts/release-notes.sh) relies on it to find the pull request.

## Documentation

Prose docs are bilingual: every file under `docs/` has a Traditional Chinese mirror under
`docs/zh-TW/`, and `RELEASING.md` mirrors to `docs/zh-TW/RELEASING.md`. Update both sides. UI
strings live in `src/i18n/{en,zh-Hant,zh-Hans}.json`.

## Releases — the notes format is mandatory

**Every GitHub Release uses the standard format.** Never hand-write a release body, and never
ship GitHub's generated notes as the final body.

1. Read [`RELEASING.md`](RELEASING.md) before tagging anything.
2. Scaffold the notes with [`scripts/release-notes.sh`](scripts/release-notes.sh); it fills in
   `What's Changed`, `New Contributors` and `Full Changelog` from the commits and pull requests.
3. Shape the rest with [`.github/release-notes-template.md`](.github/release-notes-template.md):
   a `> [!IMPORTANT]` summary with the upgrade notes, a `## Highlights` list grouped by area,
   then `## What's Changed`, `## New Contributors` when it applies, and `**Full Changelog**`.
4. English only. The docs are bilingual; the release notes are not.
5. A `-rc.N` tag is published with `--prerelease` and never moves `latest`.
6. Image tags mirror the git tag and keep the `v` prefix (`v1.3.0`, `v1.3`, `v1`); `latest` is
   the only tag without it. The exact tag is generated with `pattern={{raw}}` because
   `metadata-action` ignores custom patterns for pre-releases.
7. Keep every prose paragraph and highlight bullet on one source line: GitHub renders a newline
   inside a release body as a hard break. Check the draft with
   [`scripts/release-notes-lint.sh`](scripts/release-notes-lint.sh) before publishing.

Never push a release tag until the version bump and a refreshed `Cargo.lock` are on `master`: the
workflow validates the tag against `Cargo.toml`, and a published tag cannot be reused.
