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
let cmd = Originate::new(endpoint, app);
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

```
src/
├── command.rs              # ESL protocol: EslCommand, CommandBuilder, EslResponse (unchanged)
├── app/
│   ├── mod.rs
│   └── dptools.rs          # AppCommand (moved from command.rs) — answer, hangup, bridge, etc.
├── commands/               # API command string builders (→ api()/bgapi())
│   ├── mod.rs              # Re-exports, originate_split() tokenizer
│   ├── originate.rs        # Variables, Endpoint, Application, Originate
│   ├── channel.rs          # uuid_answer, uuid_bridge, uuid_kill, uuid_setvar, ...
│   └── conference.rs       # conference mute/unmute/hold/dtmf
├── variables/              # Channel variable format parsers
│   ├── mod.rs
│   ├── esl_array.rs        # ARRAY::item1|:item2 format
│   └── sip_multipart.rs    # SIP multipart body extraction
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

Ported from Python `c911p/freeswitch/esl/originate.py`.

**Variables** — channel variable bag with scope. FreeSWITCH uses three bracket types:

- `{k=v}` — default scope (set on all legs)
- `<k=v>` — enterprise scope (set on all endpoints in an enterprise originate)
- `[k=v]` — channel scope (set on the immediately following endpoint only)

Escaping rules (from FreeSWITCH source): commas → `\,`, single quotes → `\'`,
values containing spaces → wrapped in single quotes.

Uses `indexmap::IndexMap` to preserve insertion order — variable order matters for
readability and debugging, and round-trip parsing should produce identical output.

**Endpoint** — enum with three variants matching FreeSWITCH's endpoint formats:

- `Generic` — `{vars}uri` (sofia/user, verto, etc.)
- `Loopback` — `{vars}loopback/uri/context`
- `SofiaGateway` — `{vars}sofia/gateway/name/uri`

**Application** — inline (`name:args`) or XML (`&name(args)`) format.

**Originate** — full command: `originate {endpoint} {apps} [dialplan] [context] [cid_name] [cid_num] [timeout]`

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

## Dependencies Added

- `indexmap` — ordered map for `Variables` (preserves insertion order with O(1) lookup)

## What This Does Not Cover

- Automatic dispatch (no `client.originate(cmd)` method — just `client.bgapi(&cmd.to_string())`)
- Response parsing for specific commands (e.g., parsing `uuid_dump` output into a struct)
- SIP URI type (future extension point)
- Endpoint groups / enterprise originate with `|` separator
