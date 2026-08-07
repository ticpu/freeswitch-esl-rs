Perform a release of the rust-freeswitch-platform-ng workspace.

Optional override: $ARGUMENTS (format: vX.Y.Z). If provided, use that version.

## Release Workflow

This is a two-crate workspace. `freeswitch-esl-tokio` depends on
`freeswitch-types`, so **types must be published first**.

`scripts/pre-release.sh` is the gate: fmt, feature matrix, clippy, workspace and
live tests, Windows cross-check, semver-checks, and a publish dry-run.

**A pushed tag is immutable — it is created only once CI is green on the commit
it will point at. Never push a tag and its commit together.**

**Never `cargo publish` without completing these steps first:**

1. `scripts/pre-release.sh` passes
2. Push the release commit alone (`git push`) and wait for CI to pass on it
3. Create signed annotated tags (`git tag -as`) with a brief changelog
   in the tag message (use `git log --oneline <previous-tag>..HEAD` to
   generate it)
4. Push the tag (`git push --tags`)
5. Only then publish, types first:

```sh
cargo publish -p freeswitch-types
cargo publish -p freeswitch-esl-tokio
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

7. Tag the green commit and push the tag on its own:

```sh
git tag -as vX.Y.Z -m "$(cat <<'EOF'
vX.Y.Z

<changelog>
EOF
)"
git push --tags
```

8. Publish, types first:

```sh
cargo publish -p freeswitch-types
cargo publish -p freeswitch-esl-tokio
```

9. Report the tag and the changelog.

## Important

- **Never commit Cargo.lock** — this is a library crate. Cargo.lock stays gitignored.
- The tag is IMMUTABLE once pushed — never retag. If something is wrong,
  make a new patch release.
