# Outbound ESL Mode Quirks

Findings from testing against FreeSWITCH 1.10.13-dev (git 8bb2a39).

## `connect` is mandatory

In outbound mode, FreeSWITCH waits for the client to send `connect\n\n`
before the session is established. Without it, some commands silently fail
or time out. The C ESL library sends `connect` in `esl_attach_handle()`.

The `connect_session()` method sends this command and returns the channel
data (all channel variables as response headers).

## `async full` mode required for full command set

mod_event_socket.c (line ~2299) has a guard:

```c
if (switch_test_flag(listener, LFLAG_OUTBOUND) && !switch_test_flag(listener, LFLAG_FULL)) {
    goto done;
}
```

This skips all commands after `sendmsg` for outbound connections without
`LFLAG_FULL`. Commands blocked include: `linger`, `nolinger`, `event`,
`nixevent`, `noevents`, `sendevent`, `api`, `bgapi`, `log`, `nolog`.

The `full` flag is set from the socket application data:
`&socket(host:port async full)`

A command skipped by that early-out leaves the reply empty, and the `done:`
label fills an empty reply with `-ERR command not found`. So a blocked command
on a non-`full` socket is not spelled `-ERR permission denied` and
`EslError::is_permission_denied()` does not answer for it — it is
indistinguishable on the wire from a command mod_event_socket never heard of.

## `myevents` scopes delivery; `event` does not

Every listener joins the global list, outbound included, and the event
dispatcher matches on the subscription alone. `LFLAG_MYEVENTS`, set only by
`myevents`, is what adds the session check — it compares the event's `unique-id`
against the listener's session and drops what does not match. An outbound
session that subscribes with the `event` command therefore receives every other
call's events, and an IVR built on it answers and collects DTMF for calls that
are not its own.

An event carrying no `unique-id` is dropped under `LFLAG_MYEVENTS` too, unless
its `Job-Owner-UUID` names the session — which is how a `myevents` session still
receives the `BACKGROUND_JOB` results of its own `bgapi` commands.

## Command availability without `full`

Without `full`, you only have: `connect`, `myevents`, `getvar`, `resume`,
`filter`, `divert_events`, `sendmsg`.

The `connect_session()` response confirms the mode via headers:

- `Control: full` vs `Control: single-channel`
- `Socket-Mode: async` vs `Socket-Mode: static`

## Socket application arguments need quoting in originate

FreeSWITCH's originate parser (`switch_separate_string`) splits on spaces.
The socket application data `127.0.0.1:8040 async full` contains spaces,
so originate splits it into three tokens and the socket app only receives
the host:port.

Solution: single-quote the application argument in the originate command:

```
originate loopback/9199/test '&socket(127.0.0.1:8040 async full)'
```

The `Originate` builder handles this automatically — `originate_quote()`
wraps tokens containing spaces in single quotes with `\'` escaping for
inner quotes.

## Command availability by mode

| Command | single-channel | full |
|---|---|---|
| connect | yes | yes |
| myevents | yes | yes |
| getvar | yes | yes |
| resume | yes | yes |
| filter | yes | yes |
| divert_events | yes | yes |
| sendmsg | yes | yes |
| linger / nolinger | no | yes |
| event / nixevent / noevents | no | yes |
| api / bgapi | no | yes |
| sendevent | no | yes |
| log / nolog | no | yes |
