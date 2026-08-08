Perform a release of the rust-freeswitch-platform-ng workspace.

Optional override: $ARGUMENTS (format: vX.Y.Z). If provided, use that version.

## Release Workflow

This is a two-crate workspace. `freeswitch-esl-tokio` depends on
`freeswitch-types`, so **types must be published first**.

`scripts/pre-release.sh` is the gate: fmt, feature matrix, clippy, workspace and
live tests, Windows cross-check, semver-checks, and a publish dry-run.

**A pushed tag is immutable — it is created only once CI is green on the commit
it is built on. Never push a tag and its commit together.**

**Never `cargo publish` without completing these steps first:**

1. `scripts/pre-release.sh` passes
2. Push the release commit alone (`git push`) and wait for CI to pass on it
3. Detach, pin `Cargo.lock` on a child commit, and tag it (`git tag -as`) with a
   brief changelog in the tag message (use `git log --oneline <previous-tag>..HEAD`
   to generate it)
4. Push the tag (`git push --tags`)
5. Only then publish, from the tagged commit and types first:

```sh
git checkout vX.Y.Z
cargo publish -p freeswitch-types
cargo publish -p freeswitch-esl-tokio
git switch master
```

## Version determination

1. Find the last release tag (`git tag --sort=-v:refname | head -1`).
2. Examine commits since that tag to classify the release type:
   - **Patch**: only bug fixes, dependency bumps, build changes, docs.
   - **Minor**: new features (`feat:`), new crates, new public API surface.
   - **Major**: breaking API changes, removed crates, incompatible config changes.
3. Bump the version accordingly from the last tag.
4. If the computed type is **major**, stop and confirm with the user before proceeding.
   For patch and minor, proceed automatically.

## Version bumping rules

- **Patch releases**: only bump the version of crates that were actually modified
  since the last release tag. Use `git log` from the last tag to identify changed
  crates.
- **Minor/major releases**: bump all workspace crates together.

## Steps

1. Identify the last release tag and which crates changed since then.

2. Bump `version` in the appropriate `Cargo.toml` files (`freeswitch-types/Cargo.toml`
   and/or the root `Cargo.toml` for `freeswitch-esl-tokio`). The root's exact
   `freeswitch-types` pin moves with it.

3. Commit the bump — the gate ends in `cargo publish --dry-run`, which refuses a
   dirty working tree:

```sh
git add freeswitch-types/Cargo.toml Cargo.toml
git commit -m "release: vX.Y.Z"
```

4. Run `scripts/pre-release.sh` — stop and report on any failure. Nothing is
   published or tagged yet, so amend the release commit and re-run.

5. Draft a changelog from `git log --oneline <last-tag>..HEAD`.

   **Rules:**
   - Group entries under section headings: `New features:`, `Bug fixes:`,
     `Build:`, `Refactoring:` — omit empty sections.
   - Within each section, group by component/crate when there are multiple
     (e.g. `- freeswitch-types: ...`, `- freeswitch-esl-tokio: ...`).
   - Describe user-visible behavior, not implementation details.
   - Merge related commits for the same feature into one bullet.
   - No git hashes, no raw commit subjects, no co-author lines in the changelog.

   The tag annotation format is:
   ```
   vX.Y.Z

   New features:
   - component: what changed

   Bug fixes:
   - component: what was fixed

   Build:
   - what changed
   ```

6. Push the release commit **alone** and wait for CI to go green on it. The tag
   is immutable once pushed, so it is not created until this passes; a red run
   here is fixed with another commit, not a retag:

```sh
git push
gh run watch "$(gh run list --branch master --limit 1 --json databaseId --jq '.[0].databaseId')"
```

7. Build the tag locally — nothing is pushed yet. It sits on a detached child of
   the green commit that pins `Cargo.lock`, so a tagged tree resolves the exact
   dependency set CI validated rather than whatever is caret-compatible a year
   from now, while the lock never lands on master:

```sh
git checkout --detach
git symbolic-ref -q HEAD
git add -f Cargo.lock
git commit -m "build: pin Cargo.lock for vX.Y.Z"
git tag -as vX.Y.Z -m "$(cat <<'EOF'
vX.Y.Z

<changelog>
EOF
)"
git switch master
```

   Run these as **separate** commands, never chained with `&&`. If a chained
   command is rejected part-way — a hook, a denied permission — the untried half
   is silently skipped, and the failure mode here is committing `Cargo.lock` onto
   master because the `git checkout --detach` never ran. `git symbolic-ref -q HEAD`
   must **fail** before the lock is staged; that is the confirmation the detach
   took. `pre-commit` rejects a staged `Cargo.lock` on a branch and `pre-push`
   rejects a branch tip that tracks it, but neither replaces checking.

   The tagged commit differs from the green one by `Cargo.lock` alone.

8. Push the tag on its own. `ci.yml` triggers on `push` with no ref filter, so the
   tag gets its own run — the only one that builds against the pinned lock. Wait
   for it before publishing:

```sh
git push --tags
gh run watch "$(gh run list --branch vX.Y.Z --limit 1 --json databaseId --jq '.[0].databaseId')"
```

   Red here means the pin resolved something the floating build did not. Never
   retag; fix on master and cut a new patch release.

9. Publish from the tagged commit, types first:

```sh
git checkout vX.Y.Z
cargo publish -p freeswitch-types
cargo publish -p freeswitch-esl-tokio
git switch master
```

   `git switch master` deletes the working-tree `Cargo.lock` (untracked there);
   the next cargo command regenerates it.

10. Report the tag and the changelog.

## Important

- **Cargo.lock never reaches master** — library workspace, gitignored there. It
  exists only on the tag's own commit, so a release build is reproducible.
- The tag is IMMUTABLE once pushed — never retag. If something is wrong,
  make a new patch release.
