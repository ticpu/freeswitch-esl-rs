# freeswitch-esl-tokio

[![CI](https://github.com/ticpu/freeswitch-esl-tokio/actions/workflows/ci.yml/badge.svg)][ci]
[![Tests][tests-badge]][ci]
[![crates.io](https://img.shields.io/crates/v/freeswitch-esl-tokio)](https://crates.io/crates/freeswitch-esl-tokio)
[![docs.rs](https://img.shields.io/docsrs/freeswitch-esl-tokio)][docs]

| C-verified enums | Typed API |
|---|---|
| [![EslEventType][evt-badge]][ci] [![HangupCause][hc-badge]][ci] | [![EventHeader][eh-badge]][docs] [![ChannelVariable][cv-badge]][docs] |
| [![ChannelState][cs-badge]][ci] [![CallState][ccs-badge]][ci] | [![HeaderLookup][hl-badge]][docs] |
| [![SipHeaderPrefix][sph-badge]][ci] | [![SofiaVariable][sv-badge]][docs] |
| [![CoreMediaVariable][cmv-badge]][ci] | |

[ci]: https://github.com/ticpu/freeswitch-esl-tokio/actions/workflows/ci.yml
[docs]: https://docs.rs/freeswitch-esl-tokio
[tests-badge]: https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/ticpu/def178758b6a88effff310aca87b6b50/raw/test-count.json
[evt-badge]: https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/ticpu/def178758b6a88effff310aca87b6b50/raw/event-type-count.json
[hc-badge]: https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/ticpu/def178758b6a88effff310aca87b6b50/raw/hangup-cause-count.json
[cs-badge]: https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/ticpu/def178758b6a88effff310aca87b6b50/raw/channel-state-count.json
[ccs-badge]: https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/ticpu/def178758b6a88effff310aca87b6b50/raw/call-state-count.json
[eh-badge]: https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/ticpu/def178758b6a88effff310aca87b6b50/raw/event-header-count.json
[cv-badge]: https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/ticpu/def178758b6a88effff310aca87b6b50/raw/channel-var-count.json
[hl-badge]: https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/ticpu/def178758b6a88effff310aca87b6b50/raw/header-lookup-count.json
[sph-badge]: https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/ticpu/def178758b6a88effff310aca87b6b50/raw/sip-header-prefix-count.json
[sv-badge]: https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/ticpu/def178758b6a88effff310aca87b6b50/raw/sofia-variable-count.json
[cmv-badge]: https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/ticpu/def178758b6a88effff310aca87b6b50/raw/core-media-var-count.json

Async Rust client for FreeSWITCH
[ESL](https://developer.signalwire.com/freeswitch/FreeSWITCH-Explained/Client-and-Developer-Interfaces/Event-Socket-Library/).
Typed endpoints, typed events, serde support, split reader/writer, liveness
detection.

## Quick start

Originate through a gateway, chain a playback and a hangup inline on the
answered channel, follow that channel to its last event and read the cause it
hung up with.

```rust,no_run
use std::time::Duration;
use freeswitch_esl_tokio::*;
use freeswitch_esl_tokio::commands::*;

#[tokio::main]
async fn main() -> Result<(), EslError> {
    let (client, mut events) = EslClient::connect("localhost", 8021, "ClueCon").await?;

    // No CHANNEL_CREATE: it fires before the reply carries the UUID to match on.
    client.subscribe_events(EventFormat::Plain, &[
        EslEventType::BackgroundJob,
        EslEventType::ChannelState,
        EslEventType::ChannelDestroy,
    ]).await?;

    let cmd = Originate::inline(
        Endpoint::SofiaGateway(SofiaGateway::new("my_provider", "18005551234")),
        [
            Application::new(
                "playback",
                Some("/usr/share/freeswitch/sounds/en/us/callie/ivr/ivr-welcome.wav"),
            ),
            Application::new("hangup", Some(HangupCause::NormalClearing.to_string())),
        ],
    )?
    .timeout(Duration::from_secs(30));

    // BACKGROUND_JOB is a switch-wide event, so the Job-UUID is what makes a
    // result yours. BgJobTracker keeps that bookkeeping.
    let mut jobs: BgJobTracker<()> = BgJobTracker::new();
    jobs.bgapi(&client, &cmd.to_string(), ()).await?;

    let call_uuid = loop {
        let Some(event) = events.recv().await.transpose()? else {
            return Ok(());
        };
        if let Some(((), job)) = jobs.try_complete(&event) {
            break job.parse_body()?.to_string();
        }
    };

    while let Some(event) = events.recv().await.transpose()? {
        if event.unique_id() != Some(call_uuid.as_str()) {
            continue;
        }
        match event.event_type() {
            Some(EslEventType::ChannelDestroy) => {
                // The cause lands here, but CHANNEL_STATE with CS_DESTROY comes
                // after this event, so this is not where the loop ends.
                let cause = match event.hangup_cause() {
                    Ok(Some(c)) => c.to_string(),
                    Ok(None) => "no cause header".into(),
                    Err(e) => format!("unparseable: {e}"),
                };
                println!("channel destroyed: {call_uuid} ({cause})");
            }
            Some(EslEventType::ChannelState) => match event.is_terminal_channel_state() {
                Ok(true) => break,
                Ok(false) => {}
                Err(e) => eprintln!("unparseable channel state: {e}"),
            },
            _ => {}
        }
    }
    Ok(())
}
```

[Channel event ordering](#channel-event-ordering) spells out why the teardown
ends on the state event; `examples/channel_tracker.rs` implements the full
lifecycle.

```toml
[dependencies]
freeswitch-esl-tokio = "2"
tokio = { version = "1.0", features = ["full"] }
```

## Features

- **Split reader/writer** -- `EslClient` is `Clone + Send`, events arrive on
  a separate channel. Send commands from any task without blocking the event loop.
- **Typed endpoints** -- `SofiaEndpoint`, `SofiaGateway`, `LoopbackEndpoint`,
  `UserEndpoint`, `SofiaContact`, `GroupCall`, `ErrorEndpoint` with a
  `DialString` trait. Extensible by downstream crates.
- **Typed events** -- `ChannelState`, `CallDirection`, `EventHeader`,
  `ChannelVariable` enums. `HeaderLookup` trait gives typed accessors to any
  key-value store, not just `EslEvent`.
- **Loopback bowout detection** -- `loopback_resignation()` tells a leg that
  mod_loopback removed from a live call apart from a real teardown, keyed on
  the marker's presence so neither of its two paths is missed. Qualify it with
  `LoopbackChannelName::parse()` over the channel's own name: the marker is
  copied onto the real channel that continues the call.
- **Failed replies read as data** -- `EslError::command_failure()` hands back the
  text behind the `-ERR` / `-USAGE` marker, so `-ERR USER_BUSY` parses straight
  into a `HangupCause` without the caller knowing the prefix spellings.
- **Channel dumps** -- `parse_channel_dump()` decodes a `uuid_dump` body through
  the same parser as an event, so a rebuilt state map gets the crate's header-key
  normalization instead of a second convention.
- **Command builders** -- `Originate`, `BridgeDialString`, `UuidKill`,
  `ConferenceDtmf`, dptools -- all `Display`/`FromStr`, no transport coupling.
- **Serde** -- all builder types implement `Serialize`/`Deserialize`.
  Config-driven originate and bridge from YAML/JSON.
- **Connection health** -- liveness detection, command timeouts (default 5s),
  `is_connection_error()` / `is_recoverable()` error classification.
- **Correct wire format** -- two-part framing, percent-decoded headers,
  Content-Type detection. Matches `mod_event_socket.c`.
- **Re-exec support** (Unix) -- `teardown_for_reexec()` extracts the socket fd
  and residual parser bytes; `adopt_stream()` reconstructs the client in the
  new binary. Zero-downtime upgrades without dropping the ESL connection.

## Architecture

```text
connect() -> (EslClient, EslEventStream)

EslClient (Clone + Send)         EslEventStream
|- send commands from any task    |- events via mpsc channel
|- writer half behind Arc<Mutex>  '- connection status via watch
'- replies via oneshot channel

Background reader task
|- owns the read half + parser
|- routes CommandReply/ApiResponse -> pending oneshot
|- routes Event -> mpsc channel
|- tracks liveness (any TCP traffic resets timer)
'- broadcasts ConnectionStatus on disconnect
```

See [docs/design-rationale.md](https://github.com/ticpu/freeswitch-esl-tokio/blob/master/docs/design-rationale.md) for the full story.

## Usage

### Inbound connection

```rust,no_run
# use freeswitch_esl_tokio::{EslClient, EslError};
# #[tokio::main]
# async fn main() -> Result<(), EslError> {
let (client, mut events) = EslClient::connect("localhost", 8021, "ClueCon").await?;

let response = client.api("status").await?;
// api_result() is the whole check. A command the switch refuses is answered as
// a reply with no body; one that runs and fails reports in the body. Both come
// back as Err here. It also strips the +OK prefix action commands carry, and
// returns a query's body as-is.
println!("{}", response.api_result()?);
# let _ = &mut events;
# Ok(())
# }
```

Multi-tenant with per-user ACL:

```rust,no_run
# use freeswitch_esl_tokio::{AuthMethod, EslClient, EslConnectOptions, EslError};
# #[tokio::main]
# async fn main() -> Result<(), EslError> {
let (client, mut events) = EslClient::connect_with_auth(
    "localhost",
    8021,
    AuthMethod::user("admin@default", "ClueCon"),
    EslConnectOptions::default(),
)
.await?;
# let _ = (&client, &mut events);
# Ok(())
# }
```

### Event loop with liveness detection

`set_liveness_timeout` fires `Disconnected(HeartbeatExpired)` when no inbound
traffic arrives for the threshold, catching a silently dead TCP connection. The
library **never sends keepalives on its own** -- the timer is fed only by what
the server pushes. On a busy connection ordinary event traffic feeds it; on an
**idle** connection you supply the traffic, normally by subscribing to
`HEARTBEAT` (FreeSWITCH emits one every ~20s).

Subscribe to `HEARTBEAT` on its own command, separate from your functional
events: a permission-restricted user (`esl-allowed-events` without `HEARTBEAT`)
is rejected with `-ERR permission denied`, and bundling would sink the whole
subscription. That rejection is recoverable -- detect it with
`EslError::is_permission_denied()`, keep the connection, and skip
`set_liveness_timeout` for that user (nothing would feed the timer, so it would
trip on a healthy idle socket).

See [`examples/reconnecting_client.rs`](examples/reconnecting_client.rs) for the
full gated pattern inside a reconnection loop.

### Background API calls

`api()` **blocks the entire ESL socket** until FreeSWITCH finishes the
command -- no events are delivered and no other commands can be sent on the
connection until it returns. Use `bgapi()` for anything that may take time
(originate, conference operations, bulk queries). `bgapi()` returns
immediately with a Job-UUID; the result arrives as a `BACKGROUND_JOB` event.

`BgJobTracker` handles the Job-UUID correlation so you don't have to
maintain a pending-jobs HashMap yourself:

```rust,no_run
# use freeswitch_esl_tokio::{EslClient, EslError, EventFormat, EslEventType};
use freeswitch_esl_tokio::{BgJobTracker, EventSubscription};
# #[tokio::main]
# async fn main() -> Result<(), EslError> {
# let (client, mut events) = EslClient::connect("localhost", 8021, "ClueCon").await?;

client.apply_subscription(
    &EventSubscription::new(EventFormat::Plain)
        .event(EslEventType::BackgroundJob),
).await?;

let mut bg = BgJobTracker::new();
bg.send(&client, "sofia xmlstatus profile internal").await?;

while let Some(Ok(event)) = events.recv().await {
    if let Some(((), result)) = bg.try_complete(&event) {
        match result.parse_body() {
            Ok(data) => println!("{}", data),
            Err(e) => eprintln!("command failed: {}", e),
        }
        break;
    }
}
# Ok(())
# }
```

Attach caller context to each job for dispatch without a separate map.
The context is returned alongside the result:

```rust,no_run
# use freeswitch_esl_tokio::{EslClient, EslError, BgJobTracker};
# #[tokio::main]
# async fn main() -> Result<(), EslError> {
# let (client, mut events) = EslClient::connect("localhost", 8021, "ClueCon").await?;
# let channel_uuids: Vec<String> = Vec::new();
let mut bg: BgJobTracker<String> = BgJobTracker::new();

for uuid in &channel_uuids {
    bg.bgapi(&client, &format!("uuid_dump {uuid}"), uuid.clone()).await?;
}

while let Some(Ok(event)) = events.recv().await {
    if let Some((channel_uuid, result)) = bg.try_complete(&event) {
        // parse_body(), not body(): a job that failed reports it in the body,
        // so the raw string reads as output.
        match result.parse_body() {
            Ok(dump) => println!("dump for {channel_uuid}: {dump}"),
            Err(e) => eprintln!("dump for {channel_uuid} failed: {e}"),
        }
    }
    // ... handle other events
}
# Ok(())
# }
```

### Outbound mode

FreeSWITCH connects to your application via the `socket` dialplan app.
After accepting, send `connect` to establish the session:

```rust,no_run
use freeswitch_esl_tokio::{EslClient, AppCommand, EventFormat, HeaderLookup};
use tokio::net::TcpListener;
# use freeswitch_esl_tokio::EslError;
# #[tokio::main]
# async fn main() -> Result<(), EslError> {
let listener = TcpListener::bind("[::]:8040").await?;
let (client, mut events) = EslClient::accept_outbound(&listener).await?;

// Must be the first command after accept, returns channel info as an EslResponse
let channel_data = client.connect_session().await?;
// Channel-Name is always present in connect response
println!("Channel: {}", channel_data.channel_name().unwrap());

// Subscribe, enable linger, resume dialplan
client.myevents(EventFormat::Plain).await?;
client.linger(None).await?;
client.resume().await?;

// Control the call
client.send_command(AppCommand::answer()).await?;
client.send_command(AppCommand::playback("ivr/ivr-welcome.wav")).await?;

while let Some(Ok(event)) = events.recv().await {
    // handle events...
}
# Ok(())
# }
```

See [docs/outbound-esl-quirks.md](https://github.com/ticpu/freeswitch-esl-tokio/blob/master/docs/outbound-esl-quirks.md) for outbound
mode gotchas (`connect_session` ordering, `async full` requirement, socket app
quoting).

## Command builders

Typed builders for FreeSWITCH API commands. All implement `Display`, are
independent of `EslClient`, and can be unit tested without a connection.

### Endpoint types

Each endpoint type is a concrete struct implementing the `DialString` trait.
The `Endpoint` enum wraps them for polymorphic storage and serde.

| Type | Wire format | Description |
|---|---|---|
| `SofiaEndpoint` | `sofia/{profile}/{destination}` | Direct SIP profile routing |
| `SofiaGateway` | `sofia/gateway/{gateway}/{destination}` | SIP gateway routing |
| `LoopbackEndpoint` | `loopback/{extension}/{context}` | Internal loopback |
| `UserEndpoint` | `user/{name}@{domain}` | Directory-based dial-string lookup |
| `SofiaContact` | `${sofia_contact(user@domain)}` | Resolve registered contacts (FS runtime expression) |
| `GroupCall` | `${group_call(group@domain+A)}` | Resolve group members (FS runtime expression) |
| `ErrorEndpoint` | `error/{cause}` | Bridge to hangup cause |

```rust
use freeswitch_esl_tokio::commands::*;

// Direct SIP profile routing
let ep = Endpoint::Sofia(SofiaEndpoint::new("internal", "1000@example.com"));
assert_eq!(ep.to_string(), "sofia/internal/1000@example.com");

// SIP gateway routing
let ep = Endpoint::SofiaGateway(SofiaGateway::new("my_provider", "18005551234"));
assert_eq!(ep.to_string(), "sofia/gateway/my_provider/18005551234");

// Parse from wire format
let ep: Endpoint = "sofia/gateway/my_provider/18005551234".parse().unwrap();

// Downstream crates can implement DialString on custom endpoint types
```

### Originate

```rust,no_run
# use freeswitch_esl_tokio::{EslClient, EslError};
use freeswitch_esl_tokio::commands::*;
# #[tokio::main]
# async fn main() -> Result<(), EslError> {
# let (client, _events) = EslClient::connect("localhost", 8021, "ClueCon").await?;

let gw = || Endpoint::SofiaGateway(SofiaGateway::new("my_provider", "18005551234"));

// Inline applications
let cmd = Originate::inline(gw(), vec![
    Application::new("conference", Some("room1")),
]).unwrap();
// -> "originate sofia/gateway/my_provider/18005551234 conference:room1 inline"

// Extension target with dialplan and context
let ext_cmd = Originate::extension(gw(), "1000")
    .dialplan(DialplanType::Xml).unwrap()
    .context("default");
// -> "originate sofia/gateway/my_provider/18005551234 1000 XML default"
client.bgapi(&cmd.to_string()).await?;

// Round-trip: parse <-> display
let parsed: Originate = cmd.to_string().parse().unwrap();
assert_eq!(parsed.to_string(), cmd.to_string());
# let _ = ext_cmd;
# Ok(())
# }
```

### Bridge dial strings

`BridgeDialString` builds multi-endpoint bridge arguments with simultaneous
ring (`,`) and sequential failover (`|`):

```rust,no_run
# use freeswitch_esl_tokio::{EslClient, EslError, AppCommand};
use freeswitch_esl_tokio::commands::*;
# #[tokio::main]
# async fn main() -> Result<(), EslError> {
# let (client, _events) = EslClient::connect("localhost", 8021, "ClueCon").await?;

// Try primary and secondary simultaneously, then failover to backup
let bridge = BridgeDialString::new(vec![
    vec![
        Endpoint::SofiaGateway(SofiaGateway::new("primary", "18005551234")),
        Endpoint::SofiaGateway(SofiaGateway::new("secondary", "18005551234")),
    ],
    vec![Endpoint::SofiaGateway(SofiaGateway::new("backup", "18005551234"))],
]);
// -> "sofia/gateway/primary/18005551234,sofia/gateway/secondary/18005551234|sofia/gateway/backup/18005551234"

// Use with the bridge dptools application
client.send_command(AppCommand::bridge(bridge)).await?;
# Ok(())
# }
```

See [docs/dial-string-format.md](https://github.com/ticpu/freeswitch-esl-tokio/blob/master/docs/dial-string-format.md) for the complete
dial string reference (variable scoping, `^^:` custom delimiters, enterprise
`:_:` originate).

### UUID and conference commands

```rust,no_run
# use freeswitch_esl_tokio::{EslClient, EslError};
use freeswitch_esl_tokio::commands::*;
use freeswitch_esl_tokio::HangupCause;
# #[tokio::main]
# async fn main() -> Result<(), EslError> {
# let (client, _events) = EslClient::connect("localhost", 8021, "ClueCon").await?;
# let uuid = "11111111-1111-1111-1111-111111111111";

// UUID commands
let kill = UuidKill::with_cause(uuid, HangupCause::NormalClearing);
// -> "uuid_kill <uuid> NORMAL_CLEARING"
client.api(&kill.to_string()).await?;

// Conference commands
let dtmf = ConferenceDtmf::new("room1", "all", "1");
// -> "conference room1 dtmf all 1"
client.api(&dtmf.to_string()).await?;
# Ok(())
# }
```

> Output strings verified by unit tests in
> [`commands/originate.rs`](freeswitch-types/src/commands/originate.rs),
> [`commands/endpoint/`](freeswitch-types/src/commands/endpoint/),
> [`commands/bridge.rs`](freeswitch-types/src/commands/bridge.rs),
> [`commands/channel.rs`](freeswitch-types/src/commands/channel.rs), and
> [`commands/conference.rs`](freeswitch-types/src/commands/conference.rs).

See [docs/command-builders.md](https://github.com/ticpu/freeswitch-esl-tokio/blob/master/docs/command-builders.md) for the full builder
architecture, all channel/conference command types, and escaping rules.

## Config-driven commands (serde)

All command builder types implement `Serialize`/`Deserialize`, so originate
and bridge commands can be driven entirely from config files:

```yaml
endpoint: !sofia_gateway
  gateway: my_provider
  destination: "18005551234"
application:
  name: park
timeout_secs: 30
```

```rust,no_run
# use freeswitch_esl_tokio::{EslClient, Originate};
# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let (client, _events) = EslClient::connect("localhost", 8021, "ClueCon").await?;
# let yaml = r#"
# endpoint: !sofia_gateway
#   gateway: my_provider
#   destination: "18005551234"
# application:
#   name: park
# timeout_secs: 30
# "#;
let originate: Originate = yaml_serde::from_str(yaml)?;
client.bgapi(&originate.to_string()).await?;
# Ok(())
# }
```

`EventSubscription` also serializes, so subscriptions can live in config files:

```yaml
format: Plain
events:
- CHANNEL_CREATE
- CHANNEL_ANSWER
- CHANNEL_HANGUP_COMPLETE
- HEARTBEAT
custom_subclasses:
- "sofia::register"
# Each filter is a (header, value) tuple
filters:
- [Call-Direction, inbound]
```

The order of `events` does not matter: `CUSTOM` is terminal on the wire and the
serializer always emits it last. See
[docs/event-command-grammar.md](https://github.com/ticpu/freeswitch-esl-tokio/blob/master/docs/event-command-grammar.md) for the grammar,
what the raw string commands do not guarantee, and why a bare `CUSTOM`
subscribes to nothing.

`Variables` deserializes ergonomically -- a flat map defaults to `Default` scope:

```yaml
originate_timeout: "600"
sip_h_X-Custom: value
```

Other scopes use the explicit form:

```yaml
scope: enterprise
vars:
  key: value
```

### How endpoint types appear in YAML

The `!sofia_gateway` prefix in the example above is a YAML tag -- it tells
the deserializer which endpoint type to build from the fields that follow.
Each variant of the `Endpoint` enum has its own tag:

```yaml
# SIP gateway routing
endpoint: !sofia_gateway
  gateway: my_provider
  destination: "18005551234"

# Direct SIP profile routing
endpoint: !sofia
  profile: internal
  destination: "1000@example.com"

# Internal loopback
endpoint: !loopback
  extension: "9199"

# Directory-based routing
endpoint: !user
  name: "1001"
  domain: example.com
```

This is the format produced by `yaml_serde`. JSON libraries represent the
same data differently (`{"sofia_gateway": {"gateway": ...}}` instead of a
YAML tag), but both deserialize into the same Rust types.

See [docs/originate-loopback-yaml.md](https://github.com/ticpu/freeswitch-esl-tokio/blob/master/docs/originate-loopback-yaml.md) for a
complete YAML originate covering every field, how variables reach both
loopback legs, and how to make a loopback pair bow out.

## Variable parsers

```rust
use freeswitch_esl_tokio::variables::{EslArray, MultipartBody, SipPassthroughHeader};
use freeswitch_esl_tokio::HeaderLookup;
use freeswitch_esl_tokio::sip_header::SipHeader;
# use freeswitch_esl_tokio::commands::Variables;
# fn demo(event: &impl HeaderLookup, vars: &mut Variables, raw_multipart: &str) {

// ARRAY:: delimited values (used by FreeSWITCH for repeating SIP headers)
let arr = EslArray::parse("ARRAY::item1|:item2|:item3").unwrap();
assert_eq!(arr.items(), &["item1", "item2", "item3"]);

// SIP passthrough headers: typed access to sip_i_*, sip_h_*, sip_rh_*, etc.
// Reading incoming INVITE headers (requires parse-all-invite-headers on the sofia profile)
let pai = event.variable(SipPassthroughHeader::invite(SipHeader::PAssertedIdentity));
if let Some(raw) = pai {
    if let Ok(arr) = EslArray::parse(raw) {
        for identity in arr.items() {
            println!("P-Asserted-Identity: {}", identity);
        }
    }
}

// Setting outgoing SIP headers via channel variables
vars.insert(SipPassthroughHeader::request(SipHeader::CallInfo), "<sip:example.com>;answer-after=0");

// SIP multipart body extraction
let body = MultipartBody::parse(raw_multipart).unwrap().unwrap();

// by_mime_type matches the stored Content-Type verbatim, parameters included.
let pidf = body.by_mime_type("application/pidf+xml");

// by_media_type/MultipartItem::media_type ignore parameters and case --
// reach for this pair unless the switch is known to emit one exact spelling.
let pidf = body.by_media_type("application/pidf+xml");
# let _ = pidf;
# }
# fn main() {}
```

> Verified in [`variables/esl_array.rs`](freeswitch-types/src/variables/esl_array.rs),
> [`variables/sip_passthrough.rs`](freeswitch-types/src/variables/sip_passthrough.rs), and
> [`variables/sip_multipart.rs`](freeswitch-types/src/variables/sip_multipart.rs).

## Codec strings and SDP (`sdp` feature)

Off by default -- add `features = ["sdp"]` to the `freeswitch-esl-tokio`
dependency line to reach the `sdp` module.

`CodecString` models the FreeSWITCH codec-string grammar in both directions, so a
codec string can be read back, rewritten, or checked rather than assembled by
concatenation. `SdpCodecs` turns a peer's SDP offer into a typed codec list. The
crate supplies the grammar's own operations — append, deduplicate, filter — and
leaves the policy to you: there is no merge, because intersecting an offer while
*generating* one is not something FreeSWITCH does, and it silently narrows a list
that some interfaces require to stay complete. The block below compiles only
with the `sdp` feature enabled.

```rust,ignore
# use freeswitch_esl_tokio::EslClient;
use freeswitch_esl_tokio::sdp::{
    CodecImplementation, CodecString, CodecStringOptions, SdpCodecs,
};
use freeswitch_esl_tokio::ChannelVariable;
# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let (client, _events) = EslClient::connect("localhost", 8021, "ClueCon").await?;
# let uuid = "11111111-1111-1111-1111-111111111111";
# let remote_sdp = "v=0";

// The peer's offer, e.g. from the switch_r_sdp channel variable.
let offer = SdpCodecs::parse(remote_sdp)?;
let mut codecs = offer.audio_codec_string(&CodecStringOptions::audio(), None)?;

// Append what must always be offered. Nothing from the peer is removed or
// reordered; these land at the tail as a floor.
codecs.extend_from(&"PCMU,PCMA".parse::<CodecString>()?);

// FreeSWITCH's own dedup key, so a bare PCMU and PCMU@8000h@20i collapse.
// The first occurrence wins, which is why concatenation order is the policy.
codecs.dedup();

// What this switch has loaded. No ESL API exposes the real table, so this is
// yours to supply; entries matching nothing come back rather than vanishing.
let loaded = [CodecImplementation::new("PCMU"), CodecImplementation::new("PCMA")];
for dropped in codecs.retain_available(&loaded) {
    eprintln!("not offering {dropped}");
}

// Single-quote the value: uuid_setvar splits on spaces, and AMR format
// parameters routinely contain "; ".
client.api(&format!("uuid_setvar {uuid} {} '{codecs}'", ChannelVariable::AbsoluteCodecString)).await?;
# Ok(())
# }
```

Ordering is whatever you concatenate, since `dedup()` keeps the first occurrence:
offer-then-backup preserves the peer's preference and its qualifiers, backup-then-offer
preserves yours.

See [docs/codec-string-format.md](https://github.com/ticpu/freeswitch-esl-tokio/blob/master/docs/codec-string-format.md) for the grammar, the
characters a format-parameter value cannot carry, the G.722 rate/packetization trap,
and the two places FreeSWITCH drops a codec-string entry without logging it.

## Typed event accessors

`EslEvent` provides typed accessors that parse header values into enums
instead of returning raw strings:

```rust
use freeswitch_esl_tokio::{ChannelState, HeaderLookup};
# fn demo(event: &impl HeaderLookup) {

// Typed enums parsed from headers, no string matching needed
if let Ok(Some(state)) = event.channel_state() {
    match state {
        ChannelState::CsExecute => println!("Executing app"),
        ChannelState::CsHangup => println!("Hanging up"),
        _ => {}
    }
}

// String accessors return Option<&str>: None if the header is absent
let cid = event.caller_id_number();     // Option<&str>
// Typed accessors return Result<Option<T>, _> -- each accessor has its own parse-error type
let direction = event.call_direction(); // Result<Option<CallDirection>, _>
let cause = event.hangup_cause();       // Result<Option<HangupCause>, _>
# let _ = (cid, direction, cause);
# }
# fn main() {}
```

A header value that isn't valid UTF-8 after percent-decoding (e.g. a Latin-1
byte in a dialed string or caller name) is decoded lossily (U+FFFD) by default
rather than failing. The affected keys, with their unparsed on-wire value, are
exposed as data on `event.lossy_values()` (events) and `response.lossy_values()`
(command/`connect` replies, whose channel data FreeSWITCH percent-encodes) for
the caller to log or recover -- the library never logs them itself. Opt back
into the old hard `InvalidUtf8InHeader` error with
`EslConnectOptions::with_strict_header_utf8(true)`.

### Channel timetable

Call lifecycle timestamps via `ChannelTimetable`:

```rust
use freeswitch_esl_tokio::{HeaderLookup, ChannelTimetable};
# fn demo(event: &impl HeaderLookup) -> Result<(), Box<dyn std::error::Error>> {

// Extracts all Caller-*-Time headers from the event
let timetable = event.caller_timetable()?;

if let Some(tt) = timetable {
    // All fields are Option<i64> (microseconds since epoch):
    println!("Created: {:?}", tt.created);          // Caller-Channel-Created-Time
    println!("Answered: {:?}", tt.answered);        // Caller-Channel-Answered-Time
    println!("Hungup: {:?}", tt.hungup);            // Caller-Channel-Hangup-Time
    println!("Bridged: {:?}", tt.bridged);          // Caller-Channel-Bridged-Time
    println!("Progress: {:?}", tt.progress);        // Caller-Channel-Progress-Time
    println!("Progress media: {:?}", tt.progress_media); // Caller-Channel-Progress-Media-Time
    println!("Transferred: {:?}", tt.transferred);  // Caller-Channel-Transfer-Time
    println!("Hold accum: {:?}", tt.hold_accum);    // Caller-Channel-Hold-Accum
    // Also: profile_created, resurrected, last_hold
}

// Other-Leg timetable (bridged party):
let other = event.other_leg_timetable()?;
# let _ = other;
# Ok(())
# }
# fn main() {}
```

`ChannelTimetable::from_lookup` works the same way against any key-value
store, not just `EslEvent` -- illustrative only below, since `headers` and
`subscription_headers` stand for an arbitrary lookup and an arbitrary
subscription-building collection:

```rust,ignore
use freeswitch_esl_tokio::{ChannelTimetable, TimetablePrefix};

// Works with any key-value store, not coupled to EslEvent:
let timetable = ChannelTimetable::from_lookup(
    TimetablePrefix::Caller,
    |key| headers.get(key).map(|v| v.as_str()),
)?;

// Custom prefix for dynamic headers (e.g. "Hunt-Channel-Created-Time"):
let hunt_tt = ChannelTimetable::from_lookup("Hunt", |key| headers.get(key))?;

// Build subscription filters using SUFFIXES constant:
let prefix = TimetablePrefix::Caller.as_str();
for suffix in ChannelTimetable::SUFFIXES {
    subscription_headers.insert(format!("{prefix}-{suffix}"));
}
```

### Header and variable enums

Compile-time header and variable name enums via `HeaderLookup`:

```rust
use freeswitch_esl_tokio::{HeaderLookup, EventHeader, ChannelVariable};
# fn demo(event: &impl HeaderLookup) {

// HeaderLookup trait provides typed enum lookups on EslEvent
let uid = event.header(EventHeader::UniqueId);             // Option<&str>
let codec = event.variable(ChannelVariable::ReadCodec);    // Option<&str>
# let _ = (uid, codec);
# }
# fn main() {}
```

### Custom channel tracker with `HeaderLookup`

The `HeaderLookup` trait lets any `HashMap<String, String>` wrapper share
the same typed accessors as `EslEvent`. `HeaderLookup` requires the
`SipHeaderLookup` supertrait, so implement three methods, get all typed
accessors for free:

```rust
use std::collections::HashMap;
use freeswitch_esl_tokio::{HeaderLookup, SipHeaderLookup};

struct TrackedChannel {
    data: HashMap<String, String>,
}

impl SipHeaderLookup for TrackedChannel {
    fn sip_header_str(&self, name: &str) -> Option<&str> {
        self.data.get(name).map(|s| s.as_str())
    }
}

impl HeaderLookup for TrackedChannel {
    fn header_str(&self, name: &str) -> Option<&str> {
        self.data.get(name).map(|s| s.as_str())
    }
    fn variable_str(&self, name: &str) -> Option<&str> {
        self.data.get(&format!("variable_{}", name)).map(|s| s.as_str())
    }
}

// Now TrackedChannel has all the same typed accessors:
// ch.channel_state(), ch.call_direction(), ch.hangup_cause(),
// ch.caller_timetable(), ch.header(EventHeader::UniqueId), etc.
```

See [`examples/README.md`](examples/README.md) for what each example teaches and
what it needs, or `cargo run --example channel_tracker` for a complete reference
implementation using `HeaderLookup` for channel lifecycle monitoring.

### Channel event ordering

FreeSWITCH does not guarantee that `CHANNEL_CREATE` is the first event for a
given UUID. The state machine fires `CHANNEL_STATE` (CS_INIT) *before*
`CHANNEL_CREATE` because `set_running_state()` happens at the top of the loop
iteration, while the `CHANNEL_CREATE` event fires inside the `CS_INIT` case
block (`switch_core_state_machine.c`).

Similarly, `CHANNEL_DESTROY` is not the last event. `CHANNEL_STATE` with
CS_DESTROY fires *after* `CHANNEL_DESTROY` because
`switch_core_session_destroy_state()` is called after the destroy event
(`switch_core_session.c`).

Per-channel creation order:

1. `CHANNEL_STATE` (CS_INIT)
2. `CHANNEL_CREATE`
3. `CHANNEL_ORIGINATE` (outbound only)

Per-channel teardown order:

1. `CHANNEL_HANGUP`
2. `CHANNEL_STATE` (CS_HANGUP)
3. `CHANNEL_HANGUP_COMPLETE`
4. `CHANNEL_STATE` (CS_REPORTING)
5. `CHANNEL_DESTROY`
6. `CHANNEL_STATE` (CS_DESTROY) — true final event

The two state headers do not report the same field. `switch_channel_event_set_basic_data()`
in `switch_channel.c` fills `Channel-State` from the channel's `running_state`
and `Channel-State-Number` from its `state`, and during teardown `state` leads.
So `CHANNEL_DESTROY` carries `Channel-State: CS_REPORTING` while its
`Channel-State-Number` already reads CS_DESTROY, and only the `CHANNEL_STATE`
that follows reports `Channel-State: CS_DESTROY`. Read end-of-life from
`Channel-State`, never from the number.

Events from different channels can interleave freely on the ESL wire. If you
are tracking channel lifecycle, use `CHANNEL_STATE` (CS_INIT) as the
start-of-life trigger and `CHANNEL_STATE` (CS_DESTROY) as end-of-life rather
than relying on `CHANNEL_CREATE`/`CHANNEL_DESTROY`.

Start-of-life is two steps, though: `switch_channel_event_set_extended_data()`
adds the `variable_*` block only for the event ids on its whitelist, and
`CHANNEL_STATE` is not one of them (unless the switch runs with
`verbose-events`, the channel carries `CF_VERBOSE_EVENTS`, or the event was
given a `presence-data-cols` header). So CS_INIT names a channel without
describing one, and `CHANNEL_CREATE` -- which is whitelisted, and fires after
the endpoint's `on_init` chain -- is the first event carrying channel
variables. `CHANNEL_DESTROY` is whitelisted too, so the final variable block
arrives there rather than on the CS_DESTROY state event that ends the life.

## Benchmarks

bgapi throughput on localhost (N=10000, `bgapi status`, single connection):

| Metric | Rust | C ESL |
|---|---|---|
| send_rate_per_sec | 914 | 918 |
| rtt_median_us | 1104 | 5,472,455 |

Send rate is identical (~915 cmd/sec) -- both bottlenecked by serial ESL.
The RTT difference reflects architecture: Rust's reader task receives events
concurrently, while C ESL queues them internally during `esl_send_recv` and
only drains them afterward.

See [bench/](bench/) for build instructions and details.

## Development

```sh
./hooks/install.sh   # symlinks the pre-commit and pre-push hooks
```

The pre-commit hook enforces:

- Cargo.lock stays off branches -- it may only be committed on the release
  tag's own detached commit
- `cargo fmt --check` -- formatting
- `cargo clippy --all-features --all-targets -- -D warnings` -- lint warnings as errors
- `RUSTDOCFLAGS="-D missing_docs" cargo doc` -- all public items documented
- `cargo test --workspace --all-features` -- the full test suite, doctests included
- `hooks/check-enums.py` -- validates `EslEventType`, `HangupCause`, `ChannelState`, `CallState`,
  `CoreMediaVariable` (`core-media-vars`), `ConferenceVariable` (`conference-vars`),
  `SipHeaderPrefix` (`sip-header-prefixes`), and `EventHeader` (`event-headers`) against FreeSWITCH C source
- `hooks/check-source-refs.py` -- verifies every `file.c:NNN` citation against the pinned FreeSWITCH commit

The pre-push hook backstops the Cargo.lock check against a rebase or
cherry-pick that reintroduces it after the commit gate ran.

### Testing

Unit and mock-server tests run without external dependencies:

```sh
cargo test --lib
cargo test --test connection_tests --test command_wire_tests \
    --test connection_failure_tests --test reexec_tests
```

Live integration tests require FreeSWITCH ESL on `127.0.0.1:8022`
(password `ClueCon`). They are `#[ignore]` by default:

```sh
cargo test --test 'live_*' -- --ignored
```

They run in parallel against that one switch and raise its
`sessions-per-second` to make that safe.
[docs/live-test-switch.md](https://github.com/ticpu/freeswitch-esl-tokio/blob/master/docs/live-test-switch.md) documents the dialplan,
modules, and directory users they expect, and the two rules for writing a new
one.

## Requirements

- Rust 1.86+
- Tokio async runtime

## Migrating from 1.x

Breaking changes in 2.0:

| 1.x | 2.0 replacement |
|-----|-----------------|
| `Originate { endpoint, applications, .. }` struct literal | `Originate::application()`, `Originate::extension()`, `Originate::inline()` builders |
| `Endpoint::SofiaGateway { gateway, uri, .. }` | `Endpoint::SofiaGateway(SofiaGateway::new(gateway, uri))` |
| `Endpoint::Sofia { profile, uri, .. }` | `Endpoint::Sofia(SofiaEndpoint::new(profile, uri))` |
| `Endpoint::Loopback { extension, .. }` | `Endpoint::Loopback(LoopbackEndpoint::new(extension))` |
| `Endpoint::User { user, .. }` | `Endpoint::User(UserEndpoint::new(user))` |
| `HeaderLookup` typed accessors return `Option<T>` | Now return `Result<Option<T>, _>` (a parse error type per field; parse errors no longer silently become `None`) |
| `HeaderLookup` trait alone | Requires `SipHeaderLookup` supertrait |
| `Variables::vars_type` public field | Private; use `scope()` accessor |
| `ChannelVariable` | Renamed to `VariableName` |
| `linger(Option<u32>)` | `linger_timeout(Option<Duration>)` (deprecated in 1.x) |

New in 2.0:

- **Serde support** for all command builders (feature-gated)
- **`EventSubscription`** unifies format/events/filters into reusable config
- **Typed endpoint builders** with `_mut()` accessors for deserialized configs
- **`EslResponse::api_result()`** convenience method
- **`getvar_opt()`** distinguishes unset variables from empty strings

Upgrade steps:

1. Replace endpoint struct literals with typed constructors (`SofiaGateway::new()`, etc.)
2. Replace `Originate` struct literals with builder methods
3. Add `SipHeaderLookup` supertrait to custom `HeaderLookup` impls
4. Handle `Result` wrapper on typed header accessors (add `?` or `.unwrap()`)

## Other Rust ESL crates

- [freeswitch-esl](https://crates.io/crates/freeswitch-esl) -- async/tokio,
  JSON-only events, no split reader/writer, no liveness detection, no command
  builders or typed state. Stale since 2023.
- [eslrs](https://crates.io/crates/eslrs) -- async, still in RC. Unified
  stream (not split), silently discards unexpected responses, no timeouts.
- [freeswitch-esl-rs](https://crates.io/crates/freeswitch-esl-rs) --
  synchronous/blocking, inbound only, plain events only.

None of them offer typed endpoints, serde support, command builders,
`HeaderLookup`, or connection health monitoring.

## License

MIT OR Apache-2.0 -- see [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
