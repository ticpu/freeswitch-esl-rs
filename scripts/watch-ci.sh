#!/bin/bash
# Wait for the CI run on one commit, and fail if it fails.
#
# Usage: ./watch-ci.sh [ref]      (default: HEAD)
#
# Two things this exists to get right:
#
#   - The run is selected by workflow AND commit SHA. This repository also runs
#     GitHub's default-setup CodeQL scan, which is a separate run on the same
#     commit with no workflow file behind it, and it usually finishes first --
#     so "the most recent run on the branch" is regularly the scan, not CI.
#   - `gh run watch` exits 0 on a run that failed unless it is given
#     --exit-status.
#
# A run does not exist the instant a push returns, so the id is polled for.

set -euo pipefail

WORKFLOW=ci.yml
POLL_TIMEOUT=120
POLL_INTERVAL=5

ref="${1:-HEAD}"
# ^{commit} or an annotated tag resolves to the tag object, which no run
# matches: the release tag is always annotated.
sha="$(git rev-parse "${ref}^{commit}")"

run_id=""
waited=0
while [ -z "$run_id" ]; do
	run_id="$(gh run list --workflow "$WORKFLOW" --commit "$sha" --limit 1 \
		--json databaseId --jq '.[0].databaseId // empty')"
	[ -n "$run_id" ] && break
	if [ "$waited" -ge "$POLL_TIMEOUT" ]; then
		echo "no $WORKFLOW run appeared for $ref ($sha) within ${POLL_TIMEOUT}s" >&2
		echo "push it first, or check that the workflow triggers on this ref" >&2
		exit 1
	fi
	sleep "$POLL_INTERVAL"
	waited=$((waited + POLL_INTERVAL))
done

echo "watching $WORKFLOW run $run_id for $ref ($sha)"
gh run watch --exit-status "$run_id"
