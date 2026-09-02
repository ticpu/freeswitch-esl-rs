# Command Builder Architecture

## Problem

The library has solid ESL transport but no typed command construction. Developers
pass raw strings to `api()`/`bgapi()` and manually format channel variable strings.
This is error-prone: malformed originate strings, forgotten escaping, wrong variable
scope brackets.

## Design

Pure `Display`/`FromStr` types with no transport coupling. They produce and parse
strings. `EslClient` just calls `.to_string()`.

```rust
let cmd = Originate::application(endpoint, app).timeout(Duration::from_secs(30));
client.bgapi(&cmd.to_string()).await?;

let parsed: Originate = cmd.to_string().parse()?;
assert_eq!(cmd.to_string(), parsed.to_string());
```

### Why Display/FromStr, not a trait on EslClient

- Round-trip testing without a FreeSWITCH connection
- Downstream crates can build commands without depending on tokio/transport
- Application-specific extensions (NGCS, SIP URI builders) compose naturally
- FreeSWITCH command strings are the stable interface — types are convenience

### Why not a Command trait

A `Command` trait with `fn to_command_string(&self) -> String` would add ceremony
for no benefit over `Display`. Every FreeSWITCH API command is ultimately a string.
`Display` is idiomatic Rust for "this type serializes to a string representation".

## Module Layout

Command builders and domain types live in the `freeswitch-types` crate (no async
deps). The ESL transport crate re-exports everything.

```
freeswitch-types/src/
├── channel.rs              # ChannelState, CallState, AnswerState, CallDirection,
│                           # HangupCause, ChannelTimetable, TimetablePrefix
├── headers.rs              # EventHeader enum — typed event header names
├── macros.rs               # define_header_enum! — generates Display/FromStr/as_str for header enums
├── commands/               # API command string builders (→ api()/bgapi())
│   ├── mod.rs              # Re-exports, originate_split() tokenizer
│   ├── endpoint/           # Endpoint types, Variables, DialString trait
│   ├── originate.rs        # Application, Originate, DialplanType
│   ├── bridge.rs           # BridgeDialString (multi-endpoint dial strings)
│   ├── channel.rs          # uuid_answer, uuid_bridge, uuid_kill, uuid_setvar, ...
│   └── conference.rs       # conference mute/unmute/hold/dtmf
└── variables/              # Channel variable format parsers
    ├── mod.rs              # VariableName trait, re-exports
    ├── core.rs             # ChannelVariable enum — typed variable names
    ├── sofia.rs            # SofiaVariable enum — mod_sofia / SIP variables
    ├── esl_array.rs        # ARRAY::item1|:item2 format
    └── sip_multipart.rs    # SIP multipart body extraction

src/
├── command.rs              # ESL protocol: EslCommand, CommandBuilder, EslResponse
└── app/
    └── dptools.rs          # AppCommand — answer, hangup, bridge, playback, ...
```

### app/ vs commands/

- **app/** — dialplan applications executed via `sendmsg` (outbound mode). These
  produce `EslCommand::Execute` values for `client.send_command()`.
- **commands/** — API commands sent via `api()`/`bgapi()`. These produce strings.
  The distinction matches FreeSWITCH's own split: `sendmsg` targets a specific
  channel, API commands are global.

### variables/

Parsing types for FreeSWITCH's structured channel variable formats. These are not
commands — they parse values found in event headers. Separate module because they
have no relationship to command construction.

## Key Types

### Originate

Ported from a Python originate builder implementation.

**Variables** — channel variable bag with scope. FreeSWITCH uses three bracket types:

- `{k=v}` — default scope (set on all legs)
- `<k=v>` — enterprise scope (set on all endpoints in an enterprise originate)
- `[k=v]` — channel scope (set on the immediately following endpoint only)

Escaping rules (from FreeSWITCH source): commas → `\,`, single quotes → `\'`,
values containing spaces → wrapped in single quotes.

Uses `indexmap::IndexMap` to preserve insertion order — variable order matters for
readability and debugging, and round-trip parsing should produce identical output.

**Endpoint** — enum wrapping concrete structs, one per FreeSWITCH endpoint module.
Each struct implements `Display`, `FromStr`, `Serialize`, `Deserialize`, and the
`DialString` trait. The enum provides serde-compatible polymorphism.

Real endpoints:

- `SofiaEndpoint` — `{vars}sofia/profile/destination`
- `SofiaGateway` — `{vars}sofia/gateway/[profile::]name/destination`
- `LoopbackEndpoint` — `{vars}loopback/extension/context`
- `UserEndpoint` — `{vars}user/name[@domain]`

Expression endpoints (produce FS runtime expressions, not expanded by library):

- `SofiaContact` — `{vars}${sofia_contact([profile/]user@domain)}`
- `GroupCall` — `{vars}${group_call(group@domain[+order])}`
- `ErrorEndpoint` — `error/cause`

Audio device endpoints (shared `AudioEndpoint` struct):

- `PortAudio` — `{vars}portaudio[/destination]`
- `PulseAudio` — `{vars}pulseaudio[/destination]`
- `Alsa` — `{vars}alsa[/destination]`

See [dial-string-format.md](dial-string-format.md) for full endpoint and
variable scoping documentation.

**Application** — inline (`name` or `name:args`) or XML (`&name(args)`) format.

**OriginateTarget** — the second argument to originate, one of:

- `Extension(String)` — route through the dialplan engine (e.g. `1000`)
- `Application(Application)` — single XML-format app (e.g. `&park()`)
- `InlineApplications(Vec<Application>)` — one or more inline apps (e.g. `park,hangup:NORMAL_CLEARING`)

**Originate** — full command: `originate {endpoint} {target} [dialplan] [context] [cid_name] [cid_num] [timeout]`

**originate_split()** — quote-aware tokenizer. Splits on a delimiter (space or comma)
while respecting single-quoted regions and backslash escapes. Ported from the Python
`originate_split()` function.

### Channel Commands

Thin wrappers producing `uuid_*` command strings. No parsing needed — these are
write-only commands.

| Type | Output |
|---|---|
| `UuidAnswer` | `uuid_answer {uuid}` |
| `UuidBridge` | `uuid_bridge {uuid} {other}` |
| `UuidDeflect` | `uuid_deflect {uuid} {uri}` |
| `UuidHold` | `uuid_hold [off] {uuid}` |
| `UuidKill` | `uuid_kill {uuid} [cause]` |
| `UuidGetVar` | `uuid_getvar {uuid} {key}` |
| `UuidSetVar` | `uuid_setvar {uuid} {key} {value}` |
| `UuidTransfer` | `uuid_transfer {uuid} {dest} [dialplan]` |
| `UuidSendDtmf` | `uuid_send_dtmf {uuid} {dtmf}` |

### Conference Commands

| Type | Output |
|---|---|
| `ConferenceMute` | `conference {name} mute\|unmute {member_id}` |
| `ConferenceHold` | `conference {name} hold\|unhold all [stream]` |
| `ConferenceDtmf` | `conference {name} dtmf {member} {dtmf}` |

### EslArray

Parses FreeSWITCH's `ARRAY::item1|:item2|:item3` format found in channel variables
when a variable holds multiple values. `Display` reproduces the wire format.

### MultipartBody

Parses SIP multipart bodies stored in `variable_sip_multipart` channel variables.
Each element is `mime/type:body_data` within an `ARRAY::` container. Provides
`by_mime_type()` for typed extraction (e.g., getting PIDF+XML geolocation data).


### BridgeDialString

Typed builder for bridge dial strings with multiple endpoints, simultaneous
ring, and sequential failover. Implements `Display`/`FromStr`/`Serialize`/
`Deserialize`.

Structure: `Vec<Vec<Endpoint>>` — outer vec is sequential groups (`|`),
inner vec is simultaneous endpoints (`,`). Global `{variables}` apply to
all endpoints. Per-endpoint `[variables]` are carried on each `Endpoint`.

```rust
let bridge = BridgeDialString {
    variables: Some(vars),
    groups: vec![
        vec![ep1, ep2],  // ring ep1 and ep2 simultaneously
        vec![ep3],       // if both fail, try ep3
    ],
};
// Wire: {vars}ep1,ep2|ep3
```

See [dial-string-format.md](dial-string-format.md) for full separator
and variable scoping semantics.

### DialString trait

Common interface for anything that formats as a FreeSWITCH dial string:

```rust
pub trait DialString: fmt::Display {
    fn variables(&self) -> Option<&Variables>;
    fn variables_mut(&mut self) -> Option<&mut Variables>;
    fn set_variables(&mut self, vars: Option<Variables>);
}
```

Implemented on each concrete endpoint struct and on the `Endpoint` enum.
Downstream crates can implement `DialString` on custom endpoint types.

## Serde Support

All command builder types implement `Serialize`/`Deserialize` for config-driven
command construction (YAML/JSON -> struct -> wire format).

Key design choices:

- **`DialplanType`** — serde uses `"xml"`/`"inline"` (lowercase, config-friendly).
  `Display` uses `"XML"`/`"inline"` (wire format). Independent representations.
- **`Variables`** — flat YAML map deserializes as `VariablesType::Default` (the
  99% case). Explicit `{scope, vars}` form for Enterprise/Channel scopes.
- **`Endpoint`** — externally tagged enum with `snake_case` variant names.
- **`Originate`** — manual `Serialize`/`Deserialize` via `OriginateRaw` intermediate
  type. Validates invariants (no Extension+Inline, no empty InlineApplications) on
  deserialize. `BridgeDialString` uses straightforward derives.

## Dependencies

- `indexmap` — ordered map for `Variables` (preserves insertion order, O(1) lookup,
  serde support via `features = ["serde"]`)

## What This Does Not Cover

- Automatic dispatch (no `client.originate(cmd)` — just `client.bgapi(&cmd.to_string())`)
- Modelling a command's output schema (`status`, `sofia status`, `show` rows)
- SIP URI type (future extension point)
- Enterprise originate with `:_:` separator (deferred, documented in
  [dial-string-format.md](dial-string-format.md))
