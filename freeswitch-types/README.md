# freeswitch-types

[![crates.io](https://img.shields.io/crates/v/freeswitch-types)](https://crates.io/crates/freeswitch-types)
[![docs.rs](https://img.shields.io/docsrs/freeswitch-types)](https://docs.rs/freeswitch-types)

FreeSWITCH protocol types and general-purpose SIP header parser. No async
runtime dependency.

Includes `SipHeaderAddr`, a standalone RFC 3261 `name-addr` parser with
header-level parameters — usable in any SIP project, not just FreeSWITCH.
With `default-features = false`, the only dependencies are `sip-uri` and
`percent-encoding`.

Also provides FreeSWITCH ESL types (channel state, events, commands,
variables) for CDR parsing, config generation, command building, or
channel variable validation without pulling in tokio.

For async ESL transport (connecting to FreeSWITCH, sending commands, receiving
events), see [`freeswitch-esl-tokio`](https://crates.io/crates/freeswitch-esl-tokio)
which re-exports everything from this crate.

## What's included

| Module | Contents |
|--------|----------|
| `sip_header_addr` | `SipHeaderAddr` — RFC 3261 `name-addr` parser with header-level parameters |
| `sip_message` | `extract_header` — extract header values from raw SIP message text (RFC 3261 §7.3.1: folding, case-insensitive, multi-occurrence) |
| `sip_header` | `SipHeader` enum, `SipHeaderLookup` trait, `extract_from()` for raw messages |
| `channel` | `ChannelState`, `CallState`, `AnswerState`, `CallDirection`, `HangupCause`, `ChannelTimetable` (+ `TimetableField`, one named header at a time) |
| `headers` | `EventHeader` enum (typed event header names) |
| `lookup` | `HeaderLookup` trait (typed accessors for any key-value store) |
| `sofia` | `SofiaChannelName` (borrow-based `sofia/<profile>/<user>@<host>` parser), `SofiaEventSubclass`, `GatewayRegState`, `SipUserPingStatus` |
| `variables` | `ChannelVariable`, `CoreMediaVariable` (`unit()` → `RtpStatUnit`), `SofiaVariable`, `LoopbackVariable` (+ `LoopbackResignation`, the bowout marker), `ConferenceVariable`, `SipPassthroughHeader` (unified `sip_h_*`/`sip_i_*`/etc. with `extract_from()`), `EslArray`, `MultipartBody` |
| `event` | `EslEvent`, `EslEventType`, `EventFormat`, `EslEventPriority`, `LossyValues`/`LossyValue` (non-UTF-8 header-value signal) *(requires `esl` feature)* |
| `commands` | `Originate`, `BridgeDialString`, `UuidKill`, `UuidBridge`, endpoint types *(requires `esl` feature)* |
| `sdp` | `CodecString`/`CodecStringEntry` (the FreeSWITCH codec-string grammar, parse and emit, with `dedup`/`simplify` ported from the switch), `SdpCodecs` (SDP offer → typed codec list, plus `SdpMediaSection` for every `m=` line the offer carried — held streams included — and `NonCodecPayload` for what the switch negotiates outside the string), `CodecImplementation` (filter a codec string against what a switch has loaded) *(requires `sdp` feature)* |
| `conference_info` | RFC 4575 `conference-info+xml` types *(requires `conference-info` feature)* |

## Features

- **`esl`** (enabled by default) — ESL event and command types (`EslEvent`,
  `Originate`, `Variables`, etc.). Pulls in `indexmap` for ordered header
  storage. Disable if you only need `SipHeaderAddr` or channel state enums.
- **`serde`** (enabled by default) — adds `Serialize`/`Deserialize` impls for
  all public types. Disable with `default-features = false` if you only need
  wire-format parsing (`Display`/`FromStr`) without pulling in serde.
- **`conference-info`** — enables `ConferenceInfo::from_xml()`/`to_xml()` for
  parsing RFC 4575 `application/conference-info+xml` documents (pulls in
  `quick-xml`). Type definitions are always available without this feature.

## Usage

```toml
[dependencies]
freeswitch-types = "1"
```

SIP header parsing only (no FreeSWITCH dependencies):

```toml
[dependencies]
freeswitch-types = { version = "1", default-features = false }
```

### SIP header address parsing

`SipHeaderAddr` parses the `(name-addr / addr-spec) *(SEMI generic-param)`
production from SIP headers like `From`, `To`, `Contact`, and `Refer-To`.
It replaces `sip_uri::NameAddr` (deprecated since sip-uri 0.2.0) by
handling header-level parameters that follow the URI. General-purpose SIP
— no FreeSWITCH dependency.

```rust
use freeswitch_types::SipHeaderAddr;

let addr: SipHeaderAddr =
    r#""Alice" <sip:alice@example.com>;tag=abc123"#.parse().unwrap();
assert_eq!(addr.display_name(), Some("Alice"));
assert_eq!(addr.tag(), Some("abc123"));
assert_eq!(addr.sip_uri().unwrap().user(), Some("alice"));
```

### Command builders (requires `esl` feature)

```rust
use std::time::Duration;
use freeswitch_types::commands::*;

let cmd = Originate::application(
    Endpoint::SofiaGateway(SofiaGateway::new("my_provider", "18005551234")),
    Application::simple("park"),
)
.cid_name("Outbound Call")
.cid_num("5551234")
.timeout(Duration::from_secs(30));

// All builders implement Display, producing the FreeSWITCH wire format
assert!(cmd.to_string().starts_with("originate sofia/gateway/"));

// All builders implement FromStr for round-trip parsing
let parsed: Originate = cmd.to_string().parse().unwrap();
assert_eq!(parsed.to_string(), cmd.to_string());
```

### Typed event accessors

```rust
use freeswitch_types::{HeaderLookup, EventHeader, ChannelVariable};

// HeaderLookup works with any key-value store, not just EslEvent
struct MyHeaders(std::collections::HashMap<String, String>);

impl HeaderLookup for MyHeaders {
    fn header_str(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(|s| s.as_str())
    }
    fn variable_str(&self, name: &str) -> Option<&str> {
        self.0.get(&format!("variable_{}", name)).map(|s| s.as_str())
    }
}

// Now MyHeaders has all typed accessors:
// h.channel_state(), h.call_direction(), h.hangup_cause(),
// h.header(EventHeader::UniqueId), h.variable(ChannelVariable::ReadCodec), etc.
```

### Serde support (requires `serde` feature, enabled by default)

All builder types implement `Serialize`/`Deserialize` for config-driven usage:

```rust
use freeswitch_types::Originate;

let json = r#"{
    "endpoint": {"sofia_gateway": {"gateway": "carrier", "destination": "18005551234"}},
    "application": {"name": "park"},
    "timeout_secs": 30
}"#;
let cmd: Originate = serde_json::from_str(json).unwrap();
println!("{}", cmd);
```

### Raw SIP message header extraction

`extract_header` pulls header values from raw SIP message text, handling
case-insensitive matching, header folding, and multi-occurrence extraction
per RFC 3261 §7.3.1. Pairs naturally with the existing value parsers:

```rust
use freeswitch_types::{extract_header, SipHeaderAddr, UriInfo};

let raw_invite = "INVITE sip:sos@bcf.example.com SIP/2.0\r\n\
    Call-Info: <urn:emergency:uid:callid:abc>;purpose=emergency-CallId\r\n\
    P-Asserted-Identity: \"Alice\" <sip:+15551234567@example.com>\r\n\
    \r\n";

let ci_vals = extract_header(raw_invite, "Call-Info");
let ci = UriInfo::parse(&ci_vals[0]).unwrap();
assert_eq!(ci.entries()[0].purpose(), Some("emergency-CallId"));

let pai_vals = extract_header(raw_invite, "P-Asserted-Identity");
let pai: SipHeaderAddr = pai_vals[0].parse().unwrap();
assert_eq!(pai.display_name(), Some("Alice"));
```

`SipPassthroughHeader` and `SipHeader` also provide `extract_from()` for
convenience when working with typed header enums.

### Variable parsers

```rust
use freeswitch_types::variables::{EslArray, MultipartBody};

let arr = EslArray::parse("ARRAY::item1|:item2|:item3").unwrap();
assert_eq!(arr.items(), &["item1", "item2", "item3"]);
```

## Relationship to freeswitch-esl-tokio

This crate contains all domain types extracted from the
[`freeswitch-esl-tokio`](https://crates.io/crates/freeswitch-esl-tokio)
workspace. The ESL crate re-exports everything, so users of `freeswitch-esl-tokio`
don't need to depend on this crate directly.

Depend on `freeswitch-types` directly when you need FreeSWITCH types without
async transport (CDR processors, config validators, CLI tools, test harnesses).

## License

MIT OR Apache-2.0
