# FreeSWITCH ESL Rust Client

Async Rust client for FreeSWITCH's Event Socket Library (ESL).

## Design

### Why the split reader/writer architecture

The original library used a single `EslHandle` that owned the TCP stream directly.
Every method took `&mut self`, which made it impossible to send commands while
receiving events — the borrow checker enforced mutual exclusion. A caller's event
loop had to stop, send a command, wait for the reply, then resume polling. For an
interactive CLI this was workable; for anything with concurrent concerns (sending
keepalives, background API calls, multiple event consumers) it was a dead end.

The internal 2-second socket read timeout also leaked through `recv_event()` to
callers, making `recv_event_timeout()` broken for durations longer than 2 seconds.

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

`EslClient` is `Clone` — pass it to multiple tasks. Commands are serialized through
the writer mutex (ESL is a sequential protocol). The reader task determines event
format from each message's `Content-Type` header rather than storing state.

### Liveness detection

FreeSWITCH sends `HEARTBEAT` events every 20 seconds by default (configurable via
`event-heartbeat-interval` in `switch.conf`). The library does not implement its own
keepalive; instead it relies on the server's heartbeat as the idle-traffic source, the
same approach the C ESL library takes.

`set_liveness_timeout()` configures a threshold. Any inbound TCP traffic (not just
heartbeats) resets the timer. If the threshold is exceeded, the reader task sets the
connection status to `Disconnected(HeartbeatExpired)` and exits, which closes the
event channel.

The caller must subscribe to `HEARTBEAT` events for liveness detection to work on
idle connections. On busy connections, regular event traffic keeps the timer alive.

### Disconnection and reconnection

The library detects disconnection but never reconnects automatically. The caller sees
disconnection through:

- `events.recv()` returning `None` (channel closed)
- `events.status()` / `client.is_connected()` returning the `DisconnectReason`
- `client.api()` returning `Err(NotConnected)` after disconnect

Reconnection is the caller's responsibility. This keeps the library predictable —
the caller controls backoff strategy, re-subscription, and state recovery.

### Correct wire format

The ESL `text/event-plain` format uses two-part framing: an outer envelope
(`Content-Length` + `Content-Type`) followed by a body containing URL-encoded event
headers. Header values are percent-decoded on parse. This matches the real FreeSWITCH
wire protocol as implemented in `mod_event_socket.c` and consumed by the C ESL
library in `esl.c`.

### Error classification

`EslError` variants carry `is_connection_error()` and `is_recoverable()` helpers so
callers can decide handling without matching every variant. `HeartbeatExpired` is
classified as a connection error (non-recoverable).

## Quick Start

```toml
[dependencies]
freeswitch-esl-rs = "1.0.0"
tokio = { version = "1.0", features = ["full"] }
```

### Inbound Connection

```rust
use freeswitch_esl_rs::{EslClient, EslError};

#[tokio::main]
async fn main() -> Result<(), EslError> {
    let (client, mut events) = EslClient::connect("localhost", 8021, "ClueCon").await?;

    let response = client.api("status").await?;
    println!("FreeSWITCH Status: {}", response.body_string());

    client.disconnect().await?;
    Ok(())
}
```

### Event Loop with Liveness Detection

```rust
use freeswitch_esl_rs::{EslClient, EslEventType, EventFormat};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client, mut events) = EslClient::connect("localhost", 8021, "ClueCon").await?;

    // 60s without any TCP traffic → Disconnected(HeartbeatExpired)
    client.set_liveness_timeout(Duration::from_secs(60));

    // HEARTBEAT subscription ensures traffic on idle connections
    client.subscribe_events(EventFormat::Plain, &[
        EslEventType::Heartbeat,
        EslEventType::ChannelAnswer,
        EslEventType::ChannelHangup,
    ]).await?;

    while let Some(event) = events.recv().await {
        println!("{:?}", event.event_type());
    }

    // None → reader task exited (disconnect, EOF, or liveness timeout)
    println!("Disconnected: {:?}", events.status());
    Ok(())
}
```

### `api` vs `bgapi`

`api()` blocks until FreeSWITCH finishes the command — subject to the command timeout
(default 5s, `set_command_timeout()` to adjust). `bgapi()` returns immediately with a
Job-UUID; the result arrives later as a `BACKGROUND_JOB` event. You must subscribe to
this event type and correlate by UUID:

```rust
client.subscribe_events(EventFormat::Plain, &[
    EslEventType::BackgroundJob,
]).await?;

let response = client.bgapi("originate user/1000 &park").await?;
let job_uuid = response.job_uuid().expect("bgapi returns Job-UUID");

// Later, in the event loop:
if event.is_event_type(EslEventType::BackgroundJob) {
    if event.job_uuid() == Some(&job_uuid) {
        let result = event.body().map(|s| s.as_str()).unwrap_or("");
        // result is e.g. "+OK <channel-uuid>" or "-ERR ..."
    }
}
```

Use `bgapi` for slow commands (`originate`, `conference`) to avoid blocking the ESL
command pipeline and hitting the command timeout.

### Command Builders

The `commands` module provides typed builders for FreeSWITCH API commands. All
types implement `Display` (producing the command string) and have no dependency on
`EslClient` — they are pure string builders suitable for unit testing without a
FreeSWITCH connection.

```rust
use freeswitch_esl_rs::commands::{Originate, Endpoint, ApplicationList, Application,
    DialplanType, Variables, VariablesType, UuidKill, ConferenceDtmf};

// Originate with typed endpoint and application
let ep = Endpoint::Generic {
    uri: "sofia/gateway/gw1/18005551212".into(),
    variables: None,
};
let apps = ApplicationList(vec![Application::new("conference", Some("room1"))]);
let cmd = Originate {
    endpoint: ep,
    applications: apps,
    dialplan: Some(DialplanType::Inline),
    context: None, cid_name: None, cid_num: None, timeout: None,
};
client.bgapi(&cmd.to_string()).await?;

// Round-trip: parse ↔ display
let parsed: Originate = cmd.to_string().parse().unwrap();
assert_eq!(parsed.to_string(), cmd.to_string());

// UUID commands
let kill = UuidKill { uuid: channel_id.into(), cause: Some("NORMAL_CLEARING".into()) };
client.api(&kill.to_string()).await?;

// Conference commands
let dtmf = ConferenceDtmf { name: "room1".into(), member: "all".into(), dtmf: "1".into() };
client.api(&dtmf.to_string()).await?;
```

The `variables` module parses FreeSWITCH structured channel variable formats:

```rust
use freeswitch_esl_rs::variables::{EslArray, MultipartBody};

// Parse ARRAY:: format from channel variables
let arr = EslArray::parse("ARRAY::item1|:item2|:item3").unwrap();
assert_eq!(arr.items(), &["item1", "item2", "item3"]);

// Extract PIDF+XML from SIP multipart body
let body = MultipartBody::parse(event.header("variable_sip_multipart").unwrap()).unwrap();
let pidf = body.by_mime_type("application/pidf+xml");
```

## Requirements

- Rust 1.70+
- Tokio async runtime
- FreeSWITCH with ESL enabled

## License

MPL-2.0 — see [LICENSE](LICENSE).
