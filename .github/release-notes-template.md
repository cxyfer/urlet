# Standard release notes

Every GitHub Release uses this format. Do not write notes from scratch.

```bash
./scripts/release-notes.sh vX.Y.Z          # writes .tmp/release-notes-vX.Y.Z.md
$EDITOR .tmp/release-notes-vX.Y.Z.md       # fill in the TODO markers
```

The script fills in `What's Changed`, `New Contributors` and `Full Changelog` from the commits
and pull requests since the previous tag, and leaves the summary and the highlights for you.

## The shape

```markdown
> [!IMPORTANT]
> One paragraph on what this release is and why it matters.
>
> What changes for an existing install: migrations that run at startup, defaults that moved,
> anything the reader must do. Drop this paragraph when there is nothing to say.

## Highlights

* **Area**: What changed, in plain user-facing prose, and what it means for the reader.
* **Area**: Another group of related work.

---

## What's Changed
* <generated>

## New Contributors
* <generated; omitted when nobody new contributed>

**Full Changelog**: <generated>
```

## Rules

1. **Lead with the callout.** Most readers never scroll past it. Say what the release is, then
   say what breaks or changes for an existing install. Pre-releases say so and name the exact tag
   to pull.
2. **Write the highlights for users, not for the git history.** One bullet per area, a bold area
   name, then prose: what a person can now do, or must now do differently. Group related commits
   into one bullet. Never paste commit subjects here.
3. **Explain defaults that changed, with the fix.** "API tokens per account is now 20; set it to
   `0` in Admin → Settings for unlimited" beats "token quota added".
4. **`What's Changed` is generated.** Only edit it to fix a title that reads badly. It is built
   from the `(#N)` suffix in commit subjects, because this repository rebases before merging and
   GitHub's own generator drops those PRs when the merge commit is rewritten.
5. **English only.** The docs are bilingual; the release notes are not.
6. **Pre-releases stay pre-releases.** A `-rc.N` tag is published with `--prerelease` and never
   takes `latest`. See [RELEASING.md](../RELEASING.md).
