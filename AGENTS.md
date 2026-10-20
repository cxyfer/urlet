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

Never push a release tag until the version bump and a refreshed `Cargo.lock` are on `master`: the
workflow validates the tag against `Cargo.toml`, and a published tag cannot be reused.
