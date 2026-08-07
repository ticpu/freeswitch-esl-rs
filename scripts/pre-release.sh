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
cargo semver-checks check-release -p freeswitch-types
cargo semver-checks check-release -p freeswitch-esl-tokio
# Only types: freeswitch-esl-tokio pins an exact freeswitch-types version that
# is not on crates.io until the publish step below actually runs.
cargo publish --dry-run -p freeswitch-types
