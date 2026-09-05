# The live test switch

Every `tests/live_*.rs` file runs against a real FreeSWITCH. The tests are
`#[ignore]`d so a normal `cargo test` skips them:

```sh
ss -tlnp sport = :8022
cargo test --test 'live_*' -- --ignored
```

Everything below is what those tests assume. A missing piece shows up as one
test failing for a reason that has nothing to do with the library, so check
here first when a live test fails in isolation.

## Connection

| | |
|---|---|
| Listener | `127.0.0.1:8022` |
| Password | `ClueCon` (the crate's `DEFAULT_ESL_PASSWORD`) |
| Concurrent connections | up to `MAX_CONCURRENT_CONNECTIONS` (5) |

The suite runs tests in parallel against this one switch, so every connection
sees every other test's events. Tests must correlate on their own channel's
UUID; see [Writing a live test](#writing-a-live-test).

## Session admission rate

**This is the one that bites.** The suite raises `sessions-per-second` to 1000
before the first test connects, and leaves it raised. It's issued once per
live test binary rather than once per run -- `fsctl sps` is idempotent, so a
switch shared across binaries just gets the same value set more than once.

A loopback originate costs two sessions and the bowout pair costs four, so five
tests originating at once burst well past a stock rate limit — peaks of ~90/s
are normal. Over the limit, `switch_core_session_request_uuid()` returns NULL
(`src/switch_core_session.c`, "Throttle Error!"), mod_loopback turns that into
`SWITCH_CAUSE_DESTINATION_OUT_OF_ORDER`, and the test sees
`-ERR DESTINATION_OUT_OF_ORDER` from an originate that is perfectly valid.

The symptom is distinctive and misleading: a different test fails each run,
every one passes on its own, and nothing correlates with the code under test.
FreeSWITCH's own default is 30, which is not enough either.

It is raised at runtime rather than required in config so a fresh switch works
unconfigured. It is not restored: a parallel suite has no reliable
last-test-finished hook, and a half-restored throttle would reintroduce exactly
the flakiness this removes. If that matters on your switch, set
`sessions-per-second` in `autoload_configs/switch.conf.xml` and know that the
suite will raise it anyway.

The ESL user therefore needs `fsctl` — an `esl-allowed-api` that omits it makes
every live test fail at `connect()` with a clear message.

## Heartbeat interval, and why the suite takes ~40s

Two tests wait on FreeSWITCH's own `HEARTBEAT` event, which is the floor on
the suite's wall time — not CPU, and not test concurrency. Raising
`MAX_CONCURRENT_CONNECTIONS` buys nothing for that reason.

`event-heartbeat-interval` defaults to 20 seconds and is read from
`autoload_configs/switch.conf.xml` at startup only; there is no runtime API
for it, so changing it needs a restart. The scheduler re-reads the value on
every tick, so a lower value takes effect from the next heartbeat:

```xml
<param name="event-heartbeat-interval" value="5"/>
```

At 5 seconds the suite finishes in roughly a third of the time. Nothing
depends on the default, so this is optional.

## Dialplan

Context `test`, extension `9199`:

```xml
<extension name="echo_9199">
  <condition field="destination_number" expression="^9199$">
    <action application="answer"/>
    <action application="sched_hangup" data="+8"/>
    <action application="echo"/>
  </condition>
</extension>
```

Tests reach it as `loopback/9199/test` and as a bare `loopback/9199`. The
`answer` matters — bowout requires both legs answered — and `sched_hangup`
bounds anything a test fails to reap.

## Endpoints and modules

- **mod_loopback** — provides both the `loopback/` and `null/` endpoints. Every
  originate in the suite uses one or both. The bowout tests additionally depend
  on its default configuration: an absent `loopback.conf.xml` is fine, but
  `fire-bowout-on-bridge` must stay off (the default) because the tests key on
  channel variables rather than the `loopback::direct` event.
- **mod_commands** — `originate`, `uuid_kill`, `uuid_exists`, `uuid_getvar`,
  `uuid_setvar`, `hupall`, `fsctl`, `status`.
- **mod_dptools** — `park`, `echo`, `bridge`, `sched_hangup`, `socket`.

## Directory users

`live_connect_userauth_truncated_response` needs a `many-events@default` user
whose `esl-allowed-events` list is long enough to overflow mod_event_socket's
512-byte reply buffer, plus:

```xml
<param name="esl-allowed-api" value="show uuid_dump originate uuid_kill"/>
<param name="esl-allowed-log" value="false"/>
```

The test asserts the reply arrives truncated, so shortening the event list
breaks it. See [freeswitch-directory-users.md](freeswitch-directory-users.md).

## Outbound

`live_outbound_connect_response_preserves_underscored_case` binds an ephemeral
local port and originates with the `socket` application pointed back at it, so
FreeSWITCH must be able to reach `127.0.0.1` on an arbitrary high port.

## Writing a live test

Two rules, both learned from tests that passed for the wrong reason:

**Correlate to your own channel.** Keep the UUID the originate returns and
filter every event against it. Matching on event type alone, on a channel-name
prefix, or on "the first event to arrive" will eventually assert against
another test's channel. For a loopback pair, both legs carry the partner's UUID
in `other_loopback_leg_uuid`, which ties either leg back to the originate
without a second API call.

**Reap what you create, before you assert.** A panic between creating a channel
and killing it strands that channel for the rest of the run, and stranded
channels burn the session budget until later originates start failing. Collect
UUIDs as you learn them, kill them, then assert. Use `kill_channel()` (logs
rather than swallowing) for cleanup, and `channel_exists()` when a test needs
to *prove* a channel died — cleanup deliberately tolerates "already gone", so
it cannot tell you that.

## Checking state by hand

```sh
fs_cli -H 127.0.0.1 -P 8022 -p ClueCon -x "show channels count"
fs_cli -H 127.0.0.1 -P 8022 -p ClueCon -x "fsctl sps"
fs_cli -H 127.0.0.1 -P 8022 -p ClueCon -x "status"     # sessions per Sec out of max
fs_cli -H 127.0.0.1 -P 8022 -p ClueCon -x "hupall NORMAL_CLEARING"
```

A clean switch reports `0 total.` before and after a suite run. Anything left
behind is a test that failed to reap.
