#!/bin/bash
# Release-build the workspace, examples and benches with every feature enabled.
#
# Usage: ./ci-build.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CRATE_DIR="$(dirname "$SCRIPT_DIR")"
cd "$CRATE_DIR"

cargo build --workspace --release --all-features --all-targets
