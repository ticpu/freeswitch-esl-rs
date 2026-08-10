#!/bin/bash
# Everything that must pass before tagging and publishing a release.
#
# Usage: ./pre-release.sh
#
# Requires a live FreeSWITCH ESL listener for the live_freeswitch suite, the
# x86_64-pc-windows-msvc target for the cross-check, and cargo-semver-checks.
# Traced (set -x) so a failure names the gate that stopped it.

set -euxo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CRATE_DIR="$(dirname "$SCRIPT_DIR")"
cd "$CRATE_DIR"

cargo fmt --all
"$SCRIPT_DIR/check-feature-matrix.sh"
cargo clippy --workspace --release --all-features -- -D warnings
cargo test --workspace --release --all-features
cargo test --test live_freeswitch -- --ignored
cargo build --workspace --release --all-features
cargo build --examples --all-features
cargo check --workspace --all-features --target x86_64-pc-windows-msvc
# --all-features or the check is blind to sdp and conference-info, which are
# off by default: a removed method there passes an unqualified run untouched.
cargo semver-checks check-release --all-features -p freeswitch-types
cargo semver-checks check-release --all-features -p freeswitch-esl-tokio
# Only types: freeswitch-esl-tokio requires a freeswitch-types floor that is not
# on crates.io until the publish step below actually runs.
cargo publish --dry-run -p freeswitch-types

# docs/next-major.md is actionable only while a major bump is in flight, and
# nothing else in the release path surfaces it.
announce_deferred_breaking_changes() {
	local crate="$1" manifest="$2"
	local local_major stable_max

	local_major="$(sed -n '0,/^version = /s/^version = "\([0-9]\+\)\..*/\1/p' "$manifest")"
	# Prereleases are excluded in jq rather than by grep: a prerelease of the
	# new major must not count as the baseline it is being compared against.
	stable_max="$(
		curl -sSf "https://index.crates.io/fr/ee/$crate" |
			jq -r 'select(.yanked | not) | .vers | select(contains("-") | not)' |
			sort -V | tail -1
	)"

	if [ -z "$stable_max" ] || [ "$local_major" -le "${stable_max%%.*}" ]; then
		return
	fi

	# Untraced so the list reads as a list rather than interleaved with set -x.
	set +x
	printf '\n=== %s %s.x follows %s: docs/next-major.md ===\n\n' \
		"$crate" "$local_major" "$stable_max"
	cat "$CRATE_DIR/docs/next-major.md"
	set -x
}

announce_deferred_breaking_changes freeswitch-types freeswitch-types/Cargo.toml
announce_deferred_breaking_changes freeswitch-esl-tokio Cargo.toml
