#!/bin/bash
# Verify every freeswitch-types feature compiles on its own.
#
# Usage: ./check-feature-matrix.sh
#
# A consumer enabling one feature must not be forced to enable another: the
# default set is not a floor. Run before a release, and in CI on every push.
# Deliberately not in the pre-commit hook -- it is a full rebuild per feature.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CRATE_DIR="$(dirname "$SCRIPT_DIR")"
cd "$CRATE_DIR"

# Keep in sync with [features] in freeswitch-types/Cargo.toml.
FEATURES=(esl serde conference-info sdp)

echo "checking freeswitch-types with no default features"
cargo check -p freeswitch-types --no-default-features --message-format=short

for feature in "${FEATURES[@]}"; do
    echo "checking freeswitch-types with only --features $feature"
    cargo check -p freeswitch-types \
        --no-default-features \
        --features "$feature" \
        --message-format=short
done

echo "feature matrix ok"
