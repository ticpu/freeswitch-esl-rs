#!/bin/bash
# Build each crate on the rust-version it declares so the floor stays true.
#
# Usage: ./check-msrv.sh
#
# The stable resolver ignores dependency rust-version floors, so a dependency
# bump can silently raise the real MSRV above the declared one. Installs the
# toolchain through rustup when missing. Run before a release, and in CI.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CRATE_DIR="$(dirname "$SCRIPT_DIR")"
cd "$CRATE_DIR"

for manifest in Cargo.toml freeswitch-types/Cargo.toml; do
	crate="$(sed -n '0,/^name = /s/^name = "\(.*\)"$/\1/p' "$manifest")"
	msrv="$(sed -n 's/^rust-version = "\(.*\)"$/\1/p' "$manifest")"
	echo "checking $crate on rust $msrv"
	rustup toolchain install "$msrv" --profile minimal --no-self-update
	cargo "+$msrv" check -p "$crate" --all-features --all-targets --message-format=short
done

echo "msrv ok"
