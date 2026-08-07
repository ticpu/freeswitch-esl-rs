#!/bin/bash
# Collect the counts behind the README badges.
#
# Usage: ./ci-metrics.sh
#
# Emits KEY=VALUE lines to $GITHUB_ENV when set, stdout otherwise. A missing
# FreeSWITCH source tree is not a failure: the C-derived counts are skipped and
# C_SOURCE_AVAILABLE=false tells the workflow to leave those badges alone.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CRATE_DIR="$(dirname "$SCRIPT_DIR")"
cd "$CRATE_DIR"

emit() {
    if [ -n "${GITHUB_ENV:-}" ]; then
        echo "$1" >>"$GITHUB_ENV"
    else
        echo "$1"
    fi
}

# grep -c exits 1 on a zero count, which set -e would treat as a failure.
count_matches() {
    grep -c "$1" "$2" || true
}

if json=$(python3 "$SCRIPT_DIR/../hooks/check-enums.py" --json); then
    emit "C_SOURCE_AVAILABLE=true"
    while read -r line; do
        emit "$line"
    done < <(jq -r '
        {
            EslEventType:      "EVENT_TYPE",
            HangupCause:       "HANGUP_CAUSE",
            ChannelState:      "CHANNEL_STATE",
            CallState:         "CALL_STATE",
            CoreMediaVariable: "CORE_MEDIA_VAR",
            EventHeader:       "EVENT_HEADER"
        } as $badges
        | to_entries[]
        | select($badges[.key])
        | "\($badges[.key])_COUNT=\(.value.badge_message)",
          "\($badges[.key])_COLOR=\(.value.badge_color)"
    ' <<<"$json")
else
    [ -n "${GITHUB_ENV:-}" ] && echo "::warning::C source enum check failed"
    emit "C_SOURCE_AVAILABLE=false"
fi

emit "CHANNEL_VAR_COUNT=$(count_matches '=> "' freeswitch-types/src/variables/core.rs)"
emit "SIP_HEADER_PREFIX_COUNT=$(count_matches '=> "' freeswitch-types/src/variables/sip_passthrough.rs)"
emit "SOFIA_VARIABLE_COUNT=$(count_matches '=> "' freeswitch-types/src/variables/sofia.rs)"
emit "HEADER_LOOKUP_COUNT=$(sed -n '/^pub trait HeaderLookup/,/^}/p' freeswitch-types/src/lookup.rs | grep -c '^ *fn ' || true)"
