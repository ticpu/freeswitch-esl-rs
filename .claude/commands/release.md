Perform a release of the freeswitch-esl-tokio workspace.

Optional override: $ARGUMENTS (format: vX.Y.Z). If provided, use that version.

## Release Workflow

This is a two-crate workspace. `freeswitch-esl-tokio` depends on
`freeswitch-types`, so when both go out, **types is published first**. A release
that changed only one crate publishes only that one.

**A pushed tag is immutable — it is created only once CI is green on the commit
it is built on. Never push a tag and its commit together.**

### Pre-release checks

```sh
cargo fmt --all && \
cargo clippy --workspace --release -- -D warnings && \
cargo test --workspace --release && \
cargo test --test live_freeswitch -- --ignored && \
cargo build --workspace --release && \
cargo build --examples && \
cargo check --workspace --target x86_64-pc-windows-msvc && \
cargo semver-checks check-release -p freeswitch-types && \
cargo semver-checks check-release -p freeswitch-esl-tokio && \
cargo publish --dry-run -p freeswitch-types
```

### Publish order

Only the crates whose version changed, types first when both go out:

```sh
cargo publish -p freeswitch-types      # only if its version changed
cargo publish -p freeswitch-esl-tokio  # only if its version changed
```

**Never `cargo publish` without completing these steps first:**

1. Push the release commit **alone** (`git push`) and wait for CI to pass on it
2. Only then create the signed annotated tag (`git tag -as`) with a brief
   changelog in the tag message (use `git log --oneline <previous-tag>..HEAD` to
   generate it)
3. Push the tag (`git push --tags`) and wait for its own CI run
4. Only then `cargo publish`, whichever crates were bumped

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
   and/or the root `Cargo.toml` for `freeswitch-esl-tokio`).

3. Commit the bump — the validation below ends in `cargo publish --dry-run`,
   which refuses a dirty working tree:

```sh
git add freeswitch-types/Cargo.toml Cargo.toml
git commit -m "release: vX.Y.Z"
```

4. Run the full release validation sequence — stop and report on any failure.
   Nothing is pushed or tagged yet, so amend the release commit and re-run:

```sh
cargo fmt --all && \
cargo clippy --workspace --release -- -D warnings && \
cargo test --workspace --release && \
cargo test --test live_freeswitch -- --ignored && \
cargo build --workspace --release && \
cargo build --examples && \
cargo check --workspace --target x86_64-pc-windows-msvc && \
cargo semver-checks check-release -p freeswitch-types && \
cargo semver-checks check-release -p freeswitch-esl-tokio && \
cargo publish --dry-run -p freeswitch-types
```

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

6. Push the release commit **alone**, then wait for CI. The tag is immutable
   once pushed, so it is not created until this passes; a red run here is fixed
   with another commit, not a retag:

```sh
git push
./scripts/watch-ci.sh
```

   Never select the run to watch with `--branch ... --limit 1`. This repository
   runs GitHub's default-setup CodeQL scan alongside `ci.yml`, and it is a
   separate run on the same commit with no workflow file behind it. It usually
   finishes first, so the most recent run on the branch is regularly the scan
   rather than CI, and reading it green says nothing about CI. `watch-ci.sh`
   pins both the workflow and the commit SHA, and passes `--exit-status`,
   without which `gh run watch` exits 0 on a run that failed.

7. Tag the green commit and push the tag on its own, then wait for its run:

```sh
git tag -as vX.Y.Z -m "$(cat <<'EOF'
vX.Y.Z

<changelog>
EOF
)"
git push --tags
./scripts/watch-ci.sh vX.Y.Z
```

8. Publish, **only the crates whose version was bumped in step 2**:

```sh
cargo publish -p freeswitch-types      # only if its version changed
cargo publish -p freeswitch-esl-tokio  # only if its version changed
```

   A crate whose version did not change is already on crates.io at that version,
   and publishing it again is refused. "Types first" orders the two; it does not
   mean types is always part of a release.

9. Report the tag and the changelog.

## Important

- **Never commit Cargo.lock** — this is a library crate. Cargo.lock stays gitignored.
- The tag is IMMUTABLE once pushed — never retag. If something is wrong,
  make a new patch release.
