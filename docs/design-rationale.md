# Design Rationale

Why this library exists and the architectural decisions behind it.

## Why a new ESL library

The existing Rust ESL crates each have fundamental limitations that make them
unsuitable for production telephony applications:

- **freeswitch-esl-rs** (6k downloads) — synchronous, single-threaded, blocking
  I/O. Cannot read events while sending commands. Not thread-safe.

- **freeswitch-esl** (3k downloads) — async/tokio but self-described WIP.
  JSON-only events, no liveness detection, no command timeouts, no structured
  command builders. Stale since September 2023.

- **eslrs** (300 downloads) — newest async contender, still in release candidate.
  Unified stream (not split reader/writer), silently discards unexpected
  responses, no liveness detection or timeouts.

None of them match the feature set of the C `libesl` library that ships with
FreeSWITCH, let alone the higher-level patterns from .NET's NEventSocket. We
needed a library that could handle production call control — concurrent commands
and events, connection health monitoring, structured command building, and
correct wire format handling.

## Split reader/writer architecture

Previous designs used a single handle that owned the TCP stream. Every method
took `&mut self`, making it impossible to send commands while receiving events.
The borrow checker enforced mutual exclusion: an event loop had to stop, send a
command, wait for the reply, then resume polling.

v1.0 splits the TCP stream and spawns a background reader task:

```
connect() → (EslClient, EslEventStream)

EslClient (Clone + Send)         EslEventStream
├ send commands from any task    ├ events via mpsc channel
├ writer half behind Arc<Mutex>  └ connection status via watch
└ replies via oneshot channel

Background reader task
├ owns the read half + parser
├ routes CommandReply/ApiResponse → pending oneshot
├ routes Event → mpsc channel
├ tracks liveness (any TCP traffic resets timer)
└ broadcasts ConnectionStatus on disconnect
```

`EslClient` is `Clone` — pass it to multiple tasks. Commands are serialized
through the writer mutex (ESL is a sequential protocol). The reader task
determines event format from each message's `Content-Type` header rather than
storing state.

## Liveness detection

FreeSWITCH sends `HEARTBEAT` events every 20 seconds by default (configurable
via `event-heartbeat-interval` in `switch.conf`). The library does not implement
its own keepalive; instead it relies on the server's heartbeat as the
idle-traffic source — the same approach the C ESL library takes.

`set_liveness_timeout()` configures a threshold. Any inbound TCP traffic (not
just heartbeats) resets the timer. If the threshold is exceeded, the reader task
sets the connection status to `Disconnected(HeartbeatExpired)` and exits, which
closes the event channel.

The caller must subscribe to `HEARTBEAT` events for liveness detection to work
on idle connections. On busy connections, regular event traffic keeps the timer
alive.

## Disconnection and reconnection

The library detects disconnection but never reconnects automatically. The caller
sees disconnection through:

- `events.recv()` returning `None` (channel closed)
- `events.status()` / `client.is_connected()` returning the `DisconnectReason`
- `client.api()` returning `Err(NotConnected)` after disconnect

Reconnection is the caller's responsibility. This keeps the library predictable
— the caller controls backoff strategy, re-subscription, and state recovery.

## Correct wire format

The ESL `text/event-plain` format uses two-part framing: an outer envelope
(`Content-Length` + `Content-Type`) followed by a body containing URL-encoded
event headers. Header values are percent-decoded on parse. This matches the real
FreeSWITCH wire protocol as implemented in `mod_event_socket.c` and consumed by
the C ESL library in `esl.c`.

## Error classification

`EslError` variants carry `is_connection_error()` and `is_recoverable()` helpers
so callers can decide handling without matching every variant. Connection errors
(`Io`, `NotConnected`, `ConnectionClosed`, `HeartbeatExpired`) mean the TCP
session is dead. Recoverable errors (`Timeout`, `CommandFailed`,
`UnexpectedReply`, `QueueFull`) mean the connection is still usable.

## Command builders as pure Display types

Command builders in `commands/`, `app/`, and `variables/` implement `Display`
and `FromStr` with no dependency on `EslClient`. They produce strings,
`EslClient` calls `.to_string()`. This enables:

- Unit testing without a FreeSWITCH connection
- Round-trip testing (`parse` ↔ `to_string`)
- Reuse in contexts beyond this library (logging, debugging, CLI tools)
