# Releasing urlet

**English** | [繁體中文](docs/zh-TW/RELEASING.md)

This file is the release contract for the repository. Read it before creating a version tag.
The automation lives in [`.github/workflows/release-image.yml`](.github/workflows/release-image.yml).

## What a release does

A release is a git tag matching `vMAJOR.MINOR.PATCH` (optionally with a `-PRERELEASE` suffix).
Pushing it runs one GitHub Actions workflow that:

1. validates the tag against `Cargo.toml`,
2. builds `linux/amd64` and `linux/arm64`, and
3. pushes the image to the GitHub Container Registry.

It does **not** bump the version, run the tests, or create the GitHub Release. Those steps stay
manual on purpose (see [Rules](#rules)).

## Image tags

| Release | Tags pushed | Notes |
|---------|-------------|-------|
| `v1.2.3` (newest stable) | `v1.2.3`, `v1.2`, `v1`, `latest` | Stable release |
| `v1.2.4` (older than newest) | `v1.2.4` | Only the exact tag; moving tags stay on the newest release |
| `v1.2.3-rc.1` | `v1.2.3-rc.1` | Pre-release: never gets moving tags |
| Manual rebuild, defaults | `v1.2.3` | Exact version only |
| Manual rebuild + “update moving tags” | `v1.2.3`, `v1.2`, `v1`, `latest` | Only when `v1.2.3` is the newest stable tag |

The image tag mirrors the git tag, including the `v` prefix; `latest` is the only tag without it.
Tags published before 1.3.0 used bare versions (`1.2.0`, `1.2`, `1`) and no longer move — pin
`vX.Y.Z` or `latest` from 1.3.0 onwards.

The full image name is `ghcr.io/cxyfer/urlet`. It is derived from `github.repository`, so a fork
publishes to its own namespace.

## Rules

1. **The tag must equal the `Cargo.toml` version.** `v1.2.3` requires `version = "1.2.3"`; the
   workflow fails before building otherwise.
2. **Bump three files together.** The version lives in `Cargo.toml`, `Cargo.lock` and the
   `image:` tag in [docker-compose.yml](docker-compose.yml); all three must end up on the release
   version. Refresh the lockfile after editing `Cargo.toml` — the [Dockerfile](Dockerfile) runs
   `cargo build --locked`, so a stale root-package version breaks the image build:
   ```bash
   cargo update -p urlet
   cargo metadata --locked --format-version 1 >/dev/null
   ```
3. **Tag only merged commits.** The workflow file must exist on the tagged commit, so push
   `master` before the tag.
4. **Never move or reuse a published tag.** The tag is the immutable identity of an image.
   Publish a new patch version instead.
5. **Only the newest stable tag moves `vX`, `vX.Y` and `latest`.** A maintenance or backport release
   older than the newest stable version publishes only its exact tag. The workflow compares
   against all fetched tags before building.
6. **Release runs are serialized and queued.** A single `release-image` concurrency group with
   `queue: max` keeps up to 100 pending releases, so a burst of tags still publishes every exact
   version and a slow older build cannot finish after a newer one and move the moving tags
   backwards.
7. **Do not rebuild an old tag with moving tags enabled.** That would point `latest` (and `X`,
   `X.Y`) back at the old version. Use the default (exact version only) for old tags; the
   newest-stable guard also blocks it, but the default makes the intent explicit.
8. **Pre-releases never get `latest`.** Use suffixes such as `-rc.1`, `-beta.1`, `-alpha.1`.
9. **The GitHub Release is manual, in the standard format.** Publish it after the image exists,
   with notes scaffolded by [`scripts/release-notes.sh`](scripts/release-notes.sh) and shaped by
   [`.github/release-notes-template.md`](.github/release-notes-template.md). Keep every prose
   paragraph and highlight bullet on one source line — GitHub turns a newline inside a release
   body into a hard break — and run
   [`scripts/release-notes-lint.sh`](scripts/release-notes-lint.sh) on the draft.
10. **GHCR visibility is a repository setting.** A private repository produces a private package.
    Authenticate with `docker login ghcr.io` (token scope `read:packages`) or make the package
    public.
11. **The `image:` value in [docker-compose.yml](docker-compose.yml) is a local build tag, but it
    must still carry the release version.** It publishes nothing — every released image comes from
    the workflow — yet leaving it behind makes `docker compose up --build` label a local build with
    an older version, which is how `v1.3.0-rc.6` shipped a Compose file still naming
    `v1.3.0-rc.5`. Keep it in step with `Cargo.toml` and `Cargo.lock`.

## Release checklist

### 1. Pre-flight

```bash
git switch master
git pull --ff-only
git status --short          # must be empty

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
bash scripts/smoke.sh       # recommended: builds and exercises the real binary
```

There is currently no CI that runs these checks, so this step is the quality gate.

### 2. Bump the version

The version appears in three files and all three change together:

| File | What changes |
|------|--------------|
| [`Cargo.toml`](Cargo.toml) | `version = "1.2.3"` |
| [`Cargo.lock`](Cargo.lock) | the `urlet` package entry, refreshed by `cargo update -p urlet` |
| [`docker-compose.yml`](docker-compose.yml) | `image: urlet:v1.2.3` |

```toml
# Cargo.toml
version = "1.2.3"
```

```yaml
# docker-compose.yml
image: urlet:v1.2.3
```

Refresh the lockfile and verify it is current:

```bash
cargo update -p urlet
cargo metadata --locked --format-version 1 >/dev/null
```

Commit all three files:

```bash
git add Cargo.toml Cargo.lock docker-compose.yml
git commit -m "🔖 release: v1.2.3"
git push origin master
```

The repository uses gitmoji-style subjects; `🔖 release: vX.Y.Z` matches the existing history.

Check that all three agree before you tag:

```bash
grep -m1 '^version' Cargo.toml
grep -m1 -A2 '^name = "urlet"' Cargo.lock
grep -m1 'image: urlet:' docker-compose.yml
```

### 3. Tag and push

```bash
git tag -a v1.2.3 -m v1.2.3
git push origin v1.2.3
```

The tag push starts the workflow.

### 4. Verify the image

```bash
gh run list --workflow=release-image.yml --limit 5
gh run watch <run-id>

docker login ghcr.io      # only if the package is private
docker buildx imagetools inspect ghcr.io/cxyfer/urlet:v1.2.3
docker run --rm ghcr.io/cxyfer/urlet:v1.2.3 version   # prints "urlet 1.2.3"
```

`imagetools inspect` must list both `linux/amd64` and `linux/arm64`.

### 5. Publish the GitHub Release

Generate the skeleton, then write the summary and the highlights:

```bash
./scripts/release-notes.sh v1.2.3      # What's Changed, New Contributors, Full Changelog
$EDITOR .tmp/release-notes-v1.2.3.md   # the callout and the ## Highlights list
./scripts/release-notes-lint.sh .tmp/release-notes-v1.2.3.md

gh release create v1.2.3 --title "urlet 1.2.3" \
  --notes-file .tmp/release-notes-v1.2.3.md --verify-tag
```

Write each callout paragraph and each highlight bullet on one line; the lint command fails the
draft if the prose is still wrapped. GitHub turns a newline inside a release body into a hard
line break, so wrapped source reaches the release page broken mid-sentence.

Add `--prerelease` for a `-rc.N` tag. The shape is fixed by
[`.github/release-notes-template.md`](.github/release-notes-template.md): a short
`> [!IMPORTANT]` summary with the upgrade notes, a `## Highlights` list grouped by area, then
`## What's Changed`, `## New Contributors` when it applies, and `**Full Changelog**`.
The image is already available at this point.

### 6. Optional: allow anonymous pulls

If anonymous `docker compose pull` should work, open the package page and set
*Package settings → Danger zone → Change visibility → Public*.

## Manual rebuilds

*Actions → Release container image → Run workflow*:

- `tag`: the existing tag to rebuild (for example `v1.2.3`).
- `update_moving_tags`: leave **off** unless you are rebuilding the **newest stable** release and
  want `vX`, `vX.Y` and `latest` to follow the new digest. The newest-stable guard rejects moving
  tags for older releases.

Typical uses: a base-image CVE fix or a runner/toolchain change without a new application version.

## Cleaning up a rebuilt version

A rebuild produces a new digest under the same tag and leaves the old package version behind.
GHCR deletes versions, not tags, so removing the old one removes every tag on it. Check that the
`v`-prefixed tags and `latest` already point at the new digest, then run *Actions → Delete a
container image version* with the now-unused bare tag (for example `1.3.0-rc.1`). The workflow
refuses to delete a v-prefixed tag, or a version that still carries one or `latest`; the old
version's bare tags disappear with it.

## Failure handling

| Situation | What to do |
|-----------|------------|
| Version mismatch (the validate job fails) | Delete the tag (`git tag -d vX.Y.Z` and `git push origin :refs/tags/vX.Y.Z`), fix `Cargo.toml`/`Cargo.lock`, commit, then tag again |
| Build fails and nothing was published | Fix on `master`, delete the tag, then recreate it |
| Image published but broken | Do not reuse the tag. Bump the patch version and release again; pin the previous version in deployments meanwhile |
| Tag pushed before the workflow existed | Run the workflow manually with that tag (moving tags off) |
| The package cannot be pulled | `docker login ghcr.io`, or set the package public |
| Need the previous image | Pin `URLET_IMAGE=ghcr.io/cxyfer/urlet:<old-version>` and recreate the container |

## Quick copy-paste

```bash
VERSION=1.2.3

sed -i "s/^version = \".*\"$/version = \"$VERSION\"/" Cargo.toml
sed -i "s/^\(    image: urlet:v\).*$/\1$VERSION/" docker-compose.yml
cargo update -p urlet
cargo metadata --locked --format-version 1 >/dev/null

grep -m1 '^version' Cargo.toml            # 1.2.3
grep -m1 -A2 '^name = "urlet"' Cargo.lock # 1.2.3
grep -m1 'image: urlet:' docker-compose.yml  # urlet:v1.2.3

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

git add Cargo.toml Cargo.lock docker-compose.yml
git commit -m "🔖 release: v$VERSION"
git push origin master
git tag -a "v$VERSION" -m "v$VERSION"
git push origin "v$VERSION"

# After the image is published:
./scripts/release-notes.sh "v$VERSION"
$EDITOR ".tmp/release-notes-v$VERSION.md"
./scripts/release-notes-lint.sh ".tmp/release-notes-v$VERSION.md"
gh release create "v$VERSION" --title "urlet $VERSION" \
  --notes-file ".tmp/release-notes-v$VERSION.md" --verify-tag
```

Check the current version with:

```bash
grep '^version' Cargo.toml
```
