#!/bin/bash
# Run the workspace test suite with every feature enabled.
#
# Usage: ./ci-test.sh
#
# Emits TEST_PASSED to $GITHUB_ENV when set, stdout otherwise.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CRATE_DIR="$(dirname "$SCRIPT_DIR")"
cd "$CRATE_DIR"

OUTPUT="target/ci-test-output.txt"
mkdir -p target

CARGO_TERM_COLOR=never cargo test --workspace --all-features 2>&1 | tee "$OUTPUT"

# One "test result:" line per binary; sum the passed counts across all of them.
passed=$(awk '/^test result:/ {
    for (i = 2; i <= NF; i++)
        if ($i == "passed;") total += $(i - 1)
} END { print total + 0 }' "$OUTPUT")

if [ -n "${GITHUB_ENV:-}" ]; then
    echo "TEST_PASSED=$passed" >>"$GITHUB_ENV"
else
    echo "TEST_PASSED=$passed"
fi
