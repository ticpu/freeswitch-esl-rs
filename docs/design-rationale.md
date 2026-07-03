# Design Rationale

Why this library exists and the architectural decisions behind it.

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

`set_liveness_timeout()` arms a threshold the reader trips
(`Disconnected(HeartbeatExpired)`) when no inbound traffic arrives in time. The
deliberate choice is that the library feeds this timer with nothing of its own:
it sends no keepalive, ping, or `noop`. An internal keepalive was considered and
rejected — it would put commands on the wire the caller never asked for, which
spams the FreeSWITCH command log on every interval and breaks the rule that the
caller owns and can account for every byte sent. Liveness therefore watches only
server-pushed traffic; on idle connections the caller supplies it, conventionally
by subscribing to `HEARTBEAT`.

That subscription can be denied — a permission-restricted user
(`esl-allowed-events` without `HEARTBEAT`) is rejected with `-ERR permission
denied`, so no heartbeats arrive and an enabled timer would trip on a healthy
idle socket. The denial is surfaced as recoverable data
(`EslError::is_permission_denied()`, a `CommandFailed`, not a connection error)
rather than worked around: the caller keeps the connection, warns, and declines
to enable idle-liveness for that user. Whether an idle restricted connection is
worth keeping is the caller's call to make, not the library's.

## Disconnection and reconnection

The library detects disconnection but never reconnects automatically. The caller
sees disconnection through:

- `events.recv()` returning `None` (channel closed)
- `events.status()` / `client.is_connected()` returning the `DisconnectReason`
- `client.api()` returning `Err(NotConnected)` after disconnect

Reconnection is the caller's responsibility. This keeps the library predictable
— the caller controls backoff strategy, re-subscription, and state recovery.

### Auto-reconnect and the ESL event model

Early on we considered adding auto-reconnect with backoff, the way most
client libraries do. The more we looked at it, the clearer it became that
ESL's event-driven nature makes transparent reconnection fundamentally
unsound — not just for this library, but for any ESL transport layer.

The core issue is that ESL is a stateful, event-driven protocol with no
replay capability. There are no sequence numbers, no gap detection, no way
to ask FreeSWITCH "what did I miss while I was gone." Events that fired
during the disconnect window are gone. A call tracker that missed a
CHANNEL_DESTROY now has a ghost channel in its map. One that missed a
CHANNEL_CREATE doesn't know a call exists. The transport layer can reconnect
the TCP socket and re-subscribe to events, but it cannot reconstruct the
application state that was lost — only the caller knows that it needs to
re-dump active channels via `show channels`, re-query registrations, or
reconcile its internal maps.

Then there are commands in flight. If bytes were written to the socket but
the reply never came back, did FreeSWITCH execute the command? An originate
might have succeeded — a call is now ringing with nobody tracking it. A
uuid_kill might have gone through and the channel is already gone. The
transport cannot make the right call here. Retry? Skip? That depends on
whether the command is idempotent and what application-level cleanup is
needed. Only the caller has that context.

`bgapi` makes it worse. Each bgapi command returns a Job-UUID immediately;
the actual result arrives later as a BACKGROUND_JOB event on the same
connection. After reconnection, the new connection has a new event stream.
Those BACKGROUND_JOB events for commands sent on the old connection will
never arrive. The transport is stuck choosing between fabricating an error
response (lying), blocking forever (leaking), or silently dropping the
pending job (losing data). None of these are acceptable.

We looked at how `cgrates/fsock` (the Go ESL library used by cgrates and
fsagent) handles this, and it confirms all three failure modes. Pending
bgapi channels block forever — the old channel map is abandoned when a new
`FSConn` is created, so callers waiting on `<-out` never receive a value.
In-flight `Send()` calls get `context.DeadlineExceeded` instead of a
disconnection error, so the caller cannot distinguish "command timed out
but connection is fine" from "connection died mid-command." Event handlers
get no notification of the reconnection at all — they just see a gap in
events with no way to detect it. On top of that, the reconnect handler
takes a read lock while performing write operations on the connection
pointer, creating a data race with concurrent readers.

The alternative we chose — return a connection error, let the caller
reconnect and rebuild state from a known-good starting point — is more
work for the caller but produces correct behavior. The `reconnecting_client`
example shows the pattern. In practice though, production ESL workloads
(call tracking, CDR generation, active call control) cannot tolerate an
event gap at all — the only scenario where the ESL connection drops without
FreeSWITCH also restarting is a bug or a network partition, and in both
cases the application state is already compromised. This is exactly the
problem that drove the re-exec mechanism (`teardown_for_reexec`): preserve
the authenticated TCP socket across binary upgrades so the event stream is
never interrupted, because reconnecting and rebuilding is not good enough
when you are the system of record for active calls.

## Correct wire format

The ESL `text/event-plain` format uses two-part framing: an outer envelope
(`Content-Length` + `Content-Type`) followed by a body containing URL-encoded
event headers. Header values are percent-decoded on parse. This matches the real
FreeSWITCH wire protocol as implemented in `mod_event_socket.c` and consumed by
the C ESL library in `esl.c`.

## Typed state and header enums

FreeSWITCH is an entirely string-based system — channel state, call direction,
header names, and variable names are all plain strings on the wire. The C ESL
ecosystem and most client libraries preserve this. The problem: typos in header
names are silent, state comparisons are fragile string matches, and there's no
way to know at compile time whether `"Channal-State"` is a valid header.

Typed state enums (`ChannelState`, `CallState`, `AnswerState`, `CallDirection`),
`ChannelTimetable` for call lifecycle timestamps, and `EventHeader` /
`ChannelVariable` for header and variable name checking all implement
`FromStr`/`Display`. Typos surface at compile time; comparisons are exhaustive
match arms instead of fragile string equality.

### Decoupling from EslEvent

`ChannelTimetable::from_lookup()` accepts a closure `|key| -> Option<&str>`
rather than requiring `&EslEvent`. This lets callers use the same extraction
logic with `HashMap`, `BTreeMap`, JSON objects, or any other key-value store.
`EslEvent` convenience methods (`caller_timetable()`, `other_leg_timetable()`)
are thin wrappers, not the primary API.

The same principle applies to the state enums: `ChannelState::from_str("CS_EXECUTE")`
works on any string, not just one pulled from an `EslEvent`. The `channel_tracker`
example demonstrates this — it stores headers in a flat `HashMap` and parses
typed state on demand without going through `EslEvent`.

### No silent failures on protocol data

Parsing channel state, timetable timestamps, and header names returns `Result`
or typed errors — never `.ok()` collapsing that hides parse failures.
`ParseTimetableError` includes the header name and unparseable value so callers
can diagnose protocol issues. This follows the crate's correctness-over-recovery
philosophy: if FreeSWITCH sends an unparseable timestamp, that's a signal, not
something to silently ignore.

### define_header_enum! macro

`EventHeader` and `ChannelVariable` are generated by a `define_header_enum!`
macro that produces `Display`, `FromStr` (case-insensitive), `as_str()`, and
`AsRef<str>` for each variant. Application-specific crates can use the same
macro to define their own header enums without depending on core types.

## Header key normalization

FreeSWITCH's C ESL library stores header names verbatim but looks them up
with `strcasecmp` and a case-insensitive hash (`esl_ci_hashfunc_default` in
`esl_event.c`). This means FreeSWITCH itself doesn't care about header
casing — but Rust's `HashMap<String, String>` does.

The problem is pervasive. Multiple C code paths emit the same logical header
with different casing:

- `switch_channel.c` (`switch_channel_event_set_basic_data`) emits
  `Unique-ID`, `Channel-State`, `Channel-Read-Codec-Bit-Rate` — Title-Case.
- `switch_event.c` emits `unique-id`, `channel-state`, `answer-state` —
  all lowercase.
- `switch_core_codec.c` is internally inconsistent: read codec headers are
  all lowercase (`channel-read-codec-bit-rate`), write codec headers are
  mixed (`Channel-Write-Codec-Name` but `Channel-Write-codec-bit-rate`),
  video codec headers are all lowercase.

Because `switch_event_add_header` doesn't deduplicate, a CODEC event can
contain *both* `Channel-Read-Codec-Bit-Rate` and `channel-read-codec-bit-rate`
as separate entries. The C library finds whichever comes first via its
linked-list scan with `strcasecmp`. A Rust `HashMap` stores both as distinct
keys, and `event.header(EventHeader::ChannelReadCodecBitRate)` silently picks
whichever one `HashMap::get` hashes to.

`normalize_header_key()` canonicalizes header keys at parse time so that all
casing variants collapse to a single `HashMap` entry:

1. **Known `EventHeader` match** — the key is parsed through
   `EventHeader::from_str()` (already case-insensitive). If it matches, the
   canonical `as_str()` form is returned. This preserves acronyms (`Unique-ID`,
   `DTMF-Digit`, `Channel-Call-UUID`) and special-case names (`priority`,
   `pl_data`) exactly as defined in the enum.

2. **Underscore passthrough** — keys containing underscores are returned
   unchanged. These are channel variables (`variable_sip_call_id`) or
   `sip_h_*` passthrough headers (`variable_sip_h_X-My-Custom-Header`) where
   the suffix preserves the original SIP header casing from the wire.
   FreeSWITCH emits all `variable_*` keys from a single code path
   (`switch_channel_event_set_extended_data`), so casing is already consistent.

3. **Title-Case fallback** — unknown dash-separated keys are Title-Cased
   (capitalize first letter of each segment, lowercase the rest). This matches
   FreeSWITCH's dominant convention for event and framing headers.

The underscore passthrough is critical for `sip_h_*` variables. `sofia.c`
and `sofia_glue.c` store raw SIP header names verbatim after the `sip_h_`
prefix — `sip_h_X-My-Custom-Header` preserves the exact casing from the
SIP peer. Lowercasing these would break outbound header passthrough, since
`sofia_glue_get_extra_headers()` strips the prefix and emits the remainder
as the SIP header name on the wire.

Normalization applies at every entry point: the wire parser, `set_header()`,
and serde deserialization all funnel through `normalize_header_key()`.
`EslEvent` maintains an `original_keys` alias map (`original → normalized`)
populated when the original key differs from its normalized form, so that
`header_str("unique-id")` resolves to the `"Unique-ID"` entry without
allocating on every lookup — one extra hash probe in the fallback path.
The alias map is derived state (`#[serde(skip)]`), rebuilt during
deserialization by routing all headers through `set_header()`.

## Command builders as pure Display types

Command builders in `commands/`, `app/`, and `variables/` implement `Display`
and `FromStr` with no dependency on `EslClient`. They produce strings,
`EslClient` calls `.to_string()`. This enables:

- Unit testing without a FreeSWITCH connection
- Round-trip testing (`parse` ↔ `to_string`)
- Reuse in contexts beyond this library (logging, debugging, CLI tools)

### Serde on command builders for config-driven deployments

The serde derives on `Originate`, `Endpoint`, `Variables`, and
`BridgeDialString` exist because production callers need **config-driven
command construction**. A deployment's originate command — which gateway,
which SIP headers, which timeout — varies between environments and should
live in a YAML config file, not hardcoded in Rust.

The concrete driver was an NG911 abandoned-call callback daemon which
previously hardcoded deployment-specific SIP headers in a
`build_originate_command()` function. After adding serde to the command
builders, the entire originate command became a YAML block with `${placeholder}`
template substitution:

```yaml
originate:
  command:
    endpoint:
      sofia:
        profile: internal
        destination: "${contact}"
        variables:
          sip_h_X-Incident-Id: "${incident_id}"
    applications:
    - name: park
```

This is the pattern: **the library provides typed builders with serde, the
caller deserializes from config and calls `.to_string()` at originate time**.
No FreeSWITCH-specific knowledge is needed in the config layer.

## freeswitch-types as a separate, async-free crate

The domain types crate (`freeswitch-types`) has **zero async dependencies** —
no tokio, no futures. This split exists because the types are useful without
a network connection:

- CLI tools that parse and validate originate strings
- Config parsers that deserialize `Originate` from YAML
- Logging and debugging tools that format dial strings
- Other ESL transport implementations (sync, other runtimes)

Pulling in `freeswitch-esl-tokio` for types alone would force tokio as a
transitive dependency — unacceptable for a config parser or a static analysis
tool. The split keeps the dependency boundary clean: `freeswitch-types` is
pure data, `freeswitch-esl-tokio` is transport. The two crates version
independently so a breaking change in either layer does not force a major
bump on the other.

## Wire security: newline injection prevention

ESL is a text protocol where `\n\n` terminates a command. Any user-provided
string that reaches the wire without validation can inject arbitrary ESL
commands. For example, `api("status\n\nevent plain ALL")` would execute
`status` then silently subscribe to all events.

This was discovered during the pre-v1.0 security review. The fix:
`to_wire_format()` validates all user-supplied fields (command strings,
header names/values, passwords, app names/args) and returns
`EslError::ProtocolError` if `\n` or `\r` is present. The validation
happens at the wire boundary, not at construction time, because command
builders are `Display` types (infallible formatting) and the wire format
is the only place where newlines are dangerous.

The same principle applies to `CommandBuilder::header()` and `body()` —
they reject newlines in both names and values.

## Credential safety

ESL authentication sends passwords in cleartext over TCP. Two protections
prevent accidental exposure in logs:

1. **Manual `Debug` on `EslCommand`** — the derived `Debug` would print
   `Auth { password: "ClueCon" }` in any debug log. The manual impl redacts
   the password field.

2. **`redact_wire()` for wire logging** — debug-level wire logging uses
   `redact_wire()` which replaces the password in `auth` and `userauth`
   commands and strips the `\n\n` terminator for cleaner output.

These exist because production ESL daemons run with debug logging enabled
during incident investigation. A sysadmin grepping logs should not find
ESL passwords.

## Sequential command serialization

ESL is a strictly sequential protocol: one command in flight, one reply.
There are no request IDs, no multiplexing, no out-of-order replies. The
server processes commands in the order received and responds in the same
order.

The writer half is behind `Arc<Mutex>`, and the lock is held through the
entire send-and-wait-for-reply cycle — not just through the write. This was
a deliberate fix for a race condition found in the pre-v1.0 review: if the
lock was released after writing (before the reply arrived), two concurrent
`send_command()` calls could interleave, and the second caller's
`pending_reply` oneshot would overwrite the first's, causing misrouted
replies.

The simpler approach (hold the lock longer) was chosen over a queue-based
design because ESL doesn't support pipelining anyway — a command queue would
add complexity with no throughput benefit.

## Error classification: auth vs transient

`EslError` carries two classification helpers:

- `is_connection_error()` — TCP session is dead, must reconnect
- `is_recoverable()` — connection is still usable, retry the command

`is_recoverable()` returns `false` for `AuthenticationFailed` and
`AccessDenied`, which prevents reconnect loops on permanent configuration
errors. The motivation: production ESL daemons (fs-eventd, noans-worker)
were observed spinning in infinite reconnect loops on auth failure — retrying
every 500ms with exponential backoff to 30s, forever. The fix was a pattern:
auth failure exits with code 78 (`EX_CONFIG`), and systemd's
`RestartPreventExitStatus=78` keeps it down. Transient failures (connection
lost, timeout) exit with code 1, and systemd restarts normally.

The library's job is to classify the error accurately. The caller's job is to
decide what to do with it. This is why the library never reconnects
automatically — it can't know whether a failure is permanent or transient in
the caller's context.

## Re-exec preserves the authenticated socket across binary upgrades

Production ESL daemons like fs-eventd maintain a persistent TCP connection to
FreeSWITCH and track live channel state (active calls, channel variables,
timetables). A normal service restart loses the connection and all tracked
state. The state can be rebuilt from `show channels as json`, but events
during the reconnection gap are lost — missed hangups, missed creates, stale
channels in the tracking map.

The re-exec mechanism (`teardown_for_reexec()` + `adopt_stream()`) preserves
the TCP socket file descriptor across `exec()`, so the new binary image
inherits the already-authenticated, already-subscribed ESL connection. No
events are lost because the kernel TCP receive buffer holds data during the
brief exec window.

The drain protocol is the critical detail: the reader loop must stop at a
clean message boundary. ESL's two-part framing means that if the parser is
mid-body (headers consumed, waiting for body bytes), the residual would be
a partial body without headers — corrupt and unusable. The drain logic
continues reading until the parser returns to `WaitingForHeaders` state,
then returns the residual bytes for the new process to pre-seed its parser.

See [docs/reexec.md](docs/reexec.md) for the full API and drain protocol.

## HeaderLookup as a trait, not methods on EslEvent

Production ESL daemons don't keep `EslEvent` objects around. fs-eventd's
channel tracker stores headers in a flat `HashMap<String, String>` and
accumulates them from multiple events over a channel's lifetime. The
`HeaderLookup` trait provides typed accessor methods (`channel_state()`,
`call_direction()`, `hangup_cause()`, etc.) that work on any type
implementing two methods: `header_str(&str) -> Option<&str>` and
`variable_str(&str) -> Option<&str>`.

This means the same accessors work on:

- `EslEvent` — direct event from the wire
- `EslResponse` — connect_session response with channel data
- `TrackedChannel` — accumulated state in a HashMap
- Any custom type the caller defines

The alternative — putting accessors only on `EslEvent` — would force callers
to either keep `EslEvent` objects alive or reimplement the accessors on their
own types. The trait makes the typed API composable.

## Stop on protocol desync rather than absorb

NEventSocket (.NET) is the most mature high-level ESL client and absorbs
protocol errors to keep the stream alive: messages without Content-Type are
accepted silently, Content-Length is trusted as an allocation hint with no
upper bound, and parse exceptions are caught and discarded. In telephony a
desynced connection produces wrong call control decisions, so this library
makes the opposite trade. Content-Type is required on every message, message
and buffer sizes are capped (8 MB per message, 16 MB total), and parse errors
propagate via `EslResult` with `is_connection_error()` /
`is_recoverable()` so the caller can disconnect and reconnect from a
known-good state. The one carve-out — lossy decode of non-UTF-8 event *values* —
is the next section.

## Lossy event-value decode is not a desync

A non-UTF-8 byte that appears only *after* percent-decoding a header value is no
framing break: FreeSWITCH percent-encodes serialized-event values, so the stream
is synchronized; the decoded bytes just aren't UTF-8 (a Latin-1 dialed string or
caller name, say). A production consumer took the hard `InvalidUtf8InHeader` as
non-recoverable, exited, and dragged a supervised sibling down with it — over one
stray byte. So such values now decode lossily (U+FFFD) by default while the
connection lives, and the affected keys plus their unparsed on-wire value ride
back as *data* — `EslEvent::lossy_values()` for events, `EslResponse::lossy_values()`
for command/connect replies — not a library log line or a collapsed `None`: the
signal is inspectable, and the caller owns the warning and the PII call on the
value. `strict_header_utf8` restores the old hard-fail; framing faults (missing
`Content-Type`, no colon, oversize) stay fatal. Both paths need this: the
outbound `connect` response is itself a serialized event
(`switch_event_serialize(..., SWITCH_TRUE)`), so its channel-data values are
percent-encoded and can carry the same non-UTF-8 bytes as an inbound event body —
decoding is required on both, and so is the lossy carve-out.

## RFC 4575 conference-info XML namespace handling

RFC 4575 documents use the XML namespace `urn:ietf:params:xml:ns:conference-info`,
but producers choose their own prefix: Bell's BCF uses `confInfo:`, others use
`ci:`, and some declare it as the default namespace (no prefix). The element
names are identical in all cases — only the prefix varies.

quick-xml's serde deserializer matches element names literally, including any
prefix. A field annotated `#[serde(rename = "users")]` matches `<users>` but
not `<confInfo:users>`. The serde layer has no namespace awareness.

quick-xml does provide `NsReader` for namespace-aware event-based parsing, but
it cannot be combined with serde. Using it would mean writing a manual
event-driven parser for every RFC 4575 type — hundreds of lines of brittle code
that discards the entire value of serde derivation.

The chosen approach: **pre-process the XML to strip namespace prefixes** before
feeding it to `quick_xml::de::from_str()`. An internal `normalize` function
uses quick-xml's `Reader`/`Writer` event loop to rewrite element names
(`confInfo:users` → `users`), remove `xmlns` declarations, and preserve all
other attributes. This is correct because RFC 4575 uses a single namespace —
there are no competing prefixes to disambiguate.

The normalizer is an internal implementation detail behind `ConferenceInfo::from_xml()`.
Callers never see it. Serialization with `to_xml()` emits prefix-free XML,
which is valid RFC 4575 (using the default namespace).

## BgJobTracker: bgapi correlation as a data structure

Every production ESL daemon we've built — fs-eventd's channel tracker,
noans-worker's originate monitor, the bgapi benchmark — contained the same
boilerplate: a `HashMap<String, Context>` mapping Job-UUID to application
state, a check for `BackgroundJob` type + Job-UUID match on every event,
removal from the map, and `parse_api_body()` on the body. The pattern was
identical each time, differing only in what context was attached (channel
UUID, send timestamp, call ID).

`BgJobTracker<C>` extracts this into a generic `HashMap` wrapper. The type
parameter `C` is caller-defined context attached at send time and returned
by `try_complete()` when the matching event arrives. The caller's match arm
in the event loop is the handler — the same code that would have lived in
the manual `BackgroundJob` branch, but without the UUID bookkeeping.

A callback-based dispatcher was considered but rejected: `Box<dyn FnOnce>`
handlers run inside `dispatch()` which holds `&mut self`, preventing the
handler from accessing the surrounding `&mut app_state` without
`Arc<Mutex<>>`. A future-based design (oneshot-backed handles) creates two
consumption paths for one result — the caller must both drive `dispatch()`
and `.await` handles elsewhere, hanging silently if they forget one side.
The context-return approach sidesteps both problems because the caller
already has `&mut app_state` at the `try_complete` call site.

`BgJobResult<'a>` borrows from the event rather than cloning, matching the
library's general pattern where `event.body()` and `event.job_uuid()`
return `Option<&str>`. The result is always consumed in the same event loop
iteration. The tracker lives in `freeswitch-esl-tokio` rather than
`freeswitch-types` because its `bgapi()` convenience method calls
`EslClient::bgapi()`.

## sip-header as a standalone crate

A substantial fraction of `freeswitch-types` was pure RFC SIP standard code
with zero FreeSWITCH coupling: the `SipHeaderAddr`
name-addr parser (RFC 3261), `UriInfo` (RFC 3261 §20.9), `HistoryInfo`
(RFC 7044), `SipGeolocation` (RFC 6442), the `SipHeaderLookup` trait, SIP
message header extraction, and the full RFC 4575 conference-info XML
parser. These modules had a strict "no FreeSWITCH references in sip_*
modules" policy and clean module boundaries, but they lived in a crate
named `freeswitch-types` — forcing anyone who needed RFC SIP types to
depend on FreeSWITCH-specific ESL infrastructure they had no use for.

The problem surfaced through `eido`, the NG9-1-1 (NENA-STA-024.1a) library.
`eido` depended on `freeswitch-types` solely for `UriInfo`, `UriInfoEntry`,
`SipGeolocation`, `SipGeolocationRef`, `SipHeaderAddr`, and `sip_uri` — all
RFC-standard SIP types. It used zero FreeSWITCH-specific types: no
`EslEventType`, no `ChannelState`, no `Originate`, no `HeaderLookup`.
Anyone building NG9-1-1 tooling against a non-FreeSWITCH BCF — or a SIP
Call-Info parser for a testing harness — had to pull in ESL event types,
channel state machines, and originate command builders they would never
use, from a crate whose name signalled "this is for FreeSWITCH users."

Those modules were extracted into a standalone `sip-header` crate
(MIT OR Apache-2.0, matching `sip-uri`), which `freeswitch-types` now
depends on. The result is a layered ecosystem: `sip-uri` handles URI
parsing (RFC 3261 addr-spec), `sip-header` handles header-level parsing
(name-addr with header params, Call-Info, History-Info, Geolocation,
conference-info), and `freeswitch-types` re-exports everything from
`sip-header` while adding ESL protocol types, channel state, and command
builders on top. Existing users see no API change — all types remain
importable from `freeswitch_types::`.

### The ARRAY encoding problem

The extraction was not a simple file move because of `EslArray`. FreeSWITCH
encodes multi-value SIP headers using a proprietary pipe-delimited format:
`ARRAY::value1|:value2|:value3`. Both `UriInfo::parse()` and
`HistoryInfo::parse()` had previously imported `EslArray` to detect and
split this format alongside standard RFC comma-separated values, and
`HistoryInfo::parse()` also stripped `[...]` bracket wrapping from
FreeSWITCH log output. Those were FreeSWITCH transport-encoding details
leaking into RFC-pure parsers.

The question was whether to inline the trivial ARRAY detection (three
lines of prefix check) inside `sip-header` itself — keeping `parse()`
transparently handling both formats — or make `parse()` strictly RFC-only
and push ESL-specific handling up into `freeswitch-types`. The pragmatic
argument for inlining: it's three lines, no dependency, and the
`SipHeaderLookup` trait's default methods call `parse()`; if `parse()` is
RFC-only, then `HashMap` users with ESL data get parse failures on ARRAY
values, and Rust's orphan rules prevent `freeswitch-types` from overriding
the `HashMap` impl that `sip-header` provides.

The purity argument won. Making `parse()` RFC-only forced the design to be
honest about the abstraction boundary: a `HashMap<String, String>` holding
ESL data is not the same thing as a `HashMap` holding standard SIP headers.
The difference is real — ESL values can be ARRAY-encoded, bracket-wrapped,
and carry `variable_` prefixed keys. Papering over that inside `sip-header`
would hide a transport distinction that callers need to reason about.

### EslHeaders: making the transport boundary visible

The clean solution was a newtype. [`EslHeaders`](../freeswitch-types/src/variables/esl_headers.rs)
wraps `IndexMap<String, String>` and overrides the `SipHeaderLookup`
default methods that do RFC parsing (`call_info`, `history_info`,
`alert_info`) to peel the FreeSWITCH encoding — `ARRAY::` splitting via
`EslArray` and `[...]` bracket stripping — before delegating to the
RFC parsers (`UriInfo::from_entries`, `HistoryInfo::from_entries`).
Raw lookups (`sip_header_str`) return the stored value untouched, so
callers who want the wire form see exactly what FreeSWITCH sent.

The type makes visible what was previously hidden:

| Type | `call_info()` handles | `variable_str()` | ARRAY decoding |
|------|----------------------|-------------------|----------------|
| `HashMap<String, String>` | RFC comma-separated | no | no |
| `EslHeaders` | RFC + ARRAY + brackets | yes (`variable_` prefix) | yes |

`sip-header` provides `UriInfo::from_entries()` and
`HistoryInfo::from_entries()` that accept
`impl IntoIterator<Item = &str>`, so `EslHeaders` can split via
`EslArray` and pass pre-split entries without `sip-header` knowing
anything about the ARRAY format.

The same peeling is exposed as the associated functions
`EslHeaders::parse_uri_info` / `parse_history_info`, for callers that hold a
raw channel-variable string (e.g. `sip_call_info` fetched over ESL) rather than
a populated map — so they reuse the canonical decoder instead of re-deriving
the ARRAY/bracket handling.

### HeaderLookup as a supertrait of SipHeaderLookup

After extraction, `SipHeaderLookup` lives in `sip-header` along with its
`HashMap<String, String>` impl, so the previous blanket
`impl<T: HeaderLookup> SipHeaderLookup for T` would conflict with that
foreign impl under Rust coherence. The fix was making `SipHeaderLookup` a
supertrait of `HeaderLookup`. The same orphan rules forced dropping the
`IndexMap<String, String>` blanket `HeaderLookup` impl: ESL callers want
the `EslHeaders` newtype anyway because ARRAY decoding, bracket stripping,
and `variable_` prefix handling are correct for ESL data and wrong for a
plain `IndexMap`.

## Unified SIP passthrough headers over per-prefix enums

FreeSWITCH exposes SIP headers as channel variables through six prefixes:
`sip_i_` (incoming INVITE), `sip_h_` (outgoing request), `sip_rh_`
(outgoing response), `sip_ph_` (provisional response), `sip_bye_h_`
(BYE), and `sip_nobye_h_` (suppress on BYE). The first implementation
only covered `sip_i_*` via a fixed `SipInviteHeader` enum with ~32
variants. The other five prefixes had no typed support — users had to
pass raw strings like `"sip_h_Call-Info"` to `variable_str()`.

This was both incomplete and the wrong shape. A fixed enum cannot
cover the open-ended nature of SIP headers: `X-*` custom headers,
extension RFCs, and vendor-specific headers all pass through the same
`sip_h_*` mechanism. Meanwhile the typed `SipHeader` catalog in the
`sip-header` crate already enumerates all IANA-registered headers.
Duplicating that catalog as `SipInviteHeader` variants created a
maintenance burden (keeping two lists in sync) with no extra value.

`SipPassthroughHeader` unifies all six prefixes into one struct that
pairs a `SipHeaderPrefix` with a header name. The header name comes
from either `SipHeader` (for known headers) or a raw string (for custom
headers, validated against `\n`/`\r` injection). The struct pre-computes
the wire variable name and implements `VariableName` for use with
`HeaderLookup::variable()`.

The key asymmetry: the `sip_i_` prefix uses a lossy wire format
(lowercase, hyphens replaced by underscores: `sip_i_call_info`), while
all other prefixes preserve the canonical SIP header casing
(`sip_h_Call-Info`). `FromStr` reverses the `sip_i_` transformation by
trying `SipHeader::from_str` on the re-hyphenated suffix. For unknown
headers the reversal is lossy — original casing is lost — but this is
inherent to FreeSWITCH's wire format, not a library limitation.

## Salvage rather than fail on truncated userauth

FreeSWITCH `mod_event_socket.c` truncates long `userauth` replies into a
512-byte buffer and drops the `\n\n` terminator (see the code comment on
`salvage_truncated_auth_response` in `connection.rs` for the C-side
mechanics). The auth itself is valid on the FreeSWITCH side — `LFLAG_AUTHED`
is set before the reply is sent — and the truncated `Allowed-API` /
`Allowed-LOG` headers are informational access-policy metadata, not
required for the session to function. So `authenticate()` salvages the
partial reply, validates `Content-Type: command/reply` to be sure it's
actually an auth response (not arbitrary parser-recovery), and logs a
WARN so operators can shrink their `esl-allowed-events` list or patch
FreeSWITCH. The salvage is private to the auth path; the rest of the
parser remains strict.

## Bounded ARRAY parsing

`EslArray::parse()` caps at `MAX_ARRAY_ITEMS = 4000` to match the only
engineered ceiling in FreeSWITCH itself (`switch_event_base_add_header()`
silently drops index-addressed writes above 4000) and to prevent
heap-amplification OOM from a crafted wire value — each `|:` separator
is two bytes on the wire but ~56 bytes of heap per resulting `String`.
The cap is a wire-parsing defense only; programmatic constructors
(`new()`, `push()`, `unshift()`) build arrays in trusted code and do not
enforce it.

## EventSubscription as a reusable, serializable builder

Production ESL daemons don't subscribe to events once at startup and
forget — they re-subscribe after every reconnect, and deployments vary
which events matter. fs-eventd's channel tracker, noans-worker, and the
NG9-1-1 abandoned-call monitor all load their subscription from a
config file: which event types, which CUSTOM subclasses, which header
filters, what wire format. An earlier design bolted each piece onto the
client separately (`client.subscribe_events(...)`, `client.filter(...)`,
`client.filter(...)` again, etc.); on reconnect, the caller had to
remember the exact sequence and redo each call in the right order.

`EventSubscription` captures the whole thing — format, typed events,
raw-named events, custom subclasses, filters — as one value. The client
has `apply_subscription(&sub)` which issues the right wire commands in
the right order, and `resubscribe_from(&old, &new)` which diffs the two
and sends additive subscribes before removing the gone entries, so no
desired event type is ever briefly unsubscribed during the transition.

The serde impl exists for the same reason as on the command builders:
YAML config is the source of truth for deployments. The NG9-1-1
abandoned-call daemon's `subscription:` block deserializes directly
into `EventSubscription`, the same value is passed to
`apply_subscription` on the initial connect and on every reconnect.
Deserialization validates newline injection, space injection, and
empty strings at the boundary — an invalid config fails at load time
rather than on the wire.

The `event_raw()` / `events_raw()` pair is the escape hatch for events
FreeSWITCH adds before we update [`EslEventType`](../freeswitch-types/src/event/event_type.rs).
Without it, a freshly-added upstream event would force callers to bypass
`EventSubscription` entirely and drop down to raw `subscribe_events_raw`,
defeating the whole config-driven story. Raw events appear on the wire
alongside typed events in the same `event` command, and
`resubscribe_from` diffs them the same way.

## HangupCause::from_sip_response mapping

FreeSWITCH's mod_sofia translates SIP response codes to Q.850 hangup
causes in `sofia_glue_sip_cause_to_freeswitch()`. Callers bridging
inbound SIP provisional/final responses into their own hangup-tracking
logic need the same mapping — writing it again by hand is error-prone
and drifts from the FS source. `HangupCause::from_sip_response(code)`
is a direct port of that C function's match block, one-for-one, with
no reinterpretation.

The return is `Option<Self>`: codes without an explicit mapping
return `None` rather than defaulting to `NormalClearing` or
`NormalUnspecified`. Absence is a signal (an unusual SIP code the
caller should probably log) not a cause to fabricate. This matches
the crate's correctness-over-recovery policy — the library won't
invent a cause that FreeSWITCH itself wouldn't produce.

The mapping is neither injective nor surjective: multiple SIP codes
collapse to the same cause (`401/402/403/407/603/608 → CallRejected`),
and some Q.850 causes have no SIP counterpart in FS's table. Both are
a property of the source mapping, not the port. If
`sofia_glue_sip_cause_to_freeswitch` gains a new branch upstream, the
fix here is one added match arm — the test suite
(`from_sip_response_*` tests) enumerates every current code so drift
from the C source is visible in diff.

## CHANNEL_STATE drives lifecycle tracking, not CHANNEL_CREATE/DESTROY

FreeSWITCH fires `CHANNEL_STATE(CS_INIT)` *before* `CHANNEL_CREATE` and
`CHANNEL_STATE(CS_DESTROY)` *after* `CHANNEL_DESTROY`, so the intuitive
`CREATE → DESTROY` window misses state at both ends. The typed accessors
and the `channel_tracker` example use `CS_INIT` and `CS_DESTROY` from
`CHANNEL_STATE` as the start- and end-of-life triggers. Full ordering
notes live in the README — they belong with usage docs, not here.

