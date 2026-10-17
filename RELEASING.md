# Releasing uwurls

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
| `v1.2.3` (newest stable) | `1.2.3`, `1.2`, `1`, `latest` | Stable release |
| `v1.2.4` (older than newest) | `1.2.4` | Only the exact tag; moving tags stay on the newest release |
| `v1.2.3-rc.1` | `1.2.3-rc.1` | Pre-release: never gets moving tags |
| Manual rebuild, defaults | `1.2.3` | Exact version only |
| Manual rebuild + “update moving tags” | `1.2.3`, `1.2`, `1`, `latest` | Only when `1.2.3` is the newest stable tag |

The full image name is `ghcr.io/cxyfer/uwurls`. It is derived from `github.repository`, so a fork
publishes to its own namespace.

## Rules

1. **The tag must equal the `Cargo.toml` version.** `v1.2.3` requires `version = "1.2.3"`; the
   workflow fails before building otherwise.
2. **Refresh `Cargo.lock` after bumping.** The [Dockerfile](Dockerfile) runs
   `cargo build --locked`, so a stale root-package version breaks the image build:
   ```bash
   cargo update -p uwurls
   cargo metadata --locked --format-version 1 >/dev/null
   ```
3. **Tag only merged commits.** The workflow file must exist on the tagged commit, so push
   `master` before the tag.
4. **Never move or reuse a published tag.** The tag is the immutable identity of an image.
   Publish a new patch version instead.
5. **Only the newest stable tag moves `X`, `X.Y` and `latest`.** A maintenance or backport release
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
9. **The GitHub Release is manual.** Write the notes yourself after the image is published.
10. **GHCR visibility is a repository setting.** A private repository produces a private package.
    Authenticate with `docker login ghcr.io` (token scope `read:packages`) or make the package
    public.
11. **The `image:` value in [docker-compose.yml](docker-compose.yml) is only a local build tag.**
    Updating it is cosmetic; published images always come from the workflow.

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

Edit [`Cargo.toml`](Cargo.toml):

```toml
version = "1.2.3"
```

Refresh the lockfile and verify it is current:

```bash
cargo update -p uwurls
cargo metadata --locked --format-version 1 >/dev/null
```

Commit both files:

```bash
git add Cargo.toml Cargo.lock
git commit -m "🔖 release: v1.2.3"
git push origin master
```

The repository uses gitmoji-style subjects; `🔖 release: vX.Y.Z` matches the existing history.

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
docker buildx imagetools inspect ghcr.io/cxyfer/uwurls:1.2.3
docker run --rm ghcr.io/cxyfer/uwurls:1.2.3 version   # prints "uwurls 1.2.3"
```

`imagetools inspect` must list both `linux/amd64` and `linux/arm64`.

### 5. Publish the GitHub Release

*Releases → Draft a new release → choose the existing tag → write the notes → publish.*
The image is already available at this point.

### 6. Optional: allow anonymous pulls

If anonymous `docker compose pull` should work, open the package page and set
*Package settings → Danger zone → Change visibility → Public*.

## Manual rebuilds

*Actions → Release container image → Run workflow*:

- `tag`: the existing tag to rebuild (for example `v1.2.3`).
- `update_moving_tags`: leave **off** unless you are rebuilding the **newest stable** release and
  want `X`, `X.Y` and `latest` to follow the new digest. The newest-stable guard rejects moving
  tags for older releases.

Typical uses: a base-image CVE fix or a runner/toolchain change without a new application version.

## Failure handling

| Situation | What to do |
|-----------|------------|
| Version mismatch (the validate job fails) | Delete the tag (`git tag -d vX.Y.Z` and `git push origin :refs/tags/vX.Y.Z`), fix `Cargo.toml`/`Cargo.lock`, commit, then tag again |
| Build fails and nothing was published | Fix on `master`, delete the tag, then recreate it |
| Image published but broken | Do not reuse the tag. Bump the patch version and release again; pin the previous version in deployments meanwhile |
| Tag pushed before the workflow existed | Run the workflow manually with that tag (moving tags off) |
| The package cannot be pulled | `docker login ghcr.io`, or set the package public |
| Need the previous image | Pin `UWURLS_IMAGE=ghcr.io/cxyfer/uwurls:<old-version>` and recreate the container |

## Quick copy-paste

```bash
VERSION=1.2.3

sed -i "s/^version = \".*\"$/version = \"$VERSION\"/" Cargo.toml
cargo update -p uwurls
cargo metadata --locked --format-version 1 >/dev/null

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

git add Cargo.toml Cargo.lock
git commit -m "🔖 release: v$VERSION"
git push origin master
git tag -a "v$VERSION" -m "v$VERSION"
git push origin "v$VERSION"
```

Check the current version with:

```bash
grep '^version' Cargo.toml
```
