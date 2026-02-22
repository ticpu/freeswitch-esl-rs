## Project Type

This is a **library-first** crate. There is an examples/ folder buildable binaries
`Cargo.lock` is gitignored per Cargo convention for libraries.

## Build & Test Workflow

**Always run `cargo fmt` before every commit.** The pre-commit hook enforces
formatting, clippy warnings, `-D missing_docs` doc coverage, and EslEventType
sync with C ESL.

```sh
cargo fmt
cargo check --message-format=short
cargo clippy --fix --allow-dirty --message-format=short
cargo test --lib
```

## Release Workflow

Before tagging a release:

```sh
cargo clippy --release -- -D warnings
cargo test --release
cargo build --release
```

Tag with a signed annotated tag. Include a brief changelog in the tag message:

```sh
git tag -as v0.X.0 -m "v0.X.0

- Brief changelog entry
- Another change"
git push --tags
```

## Documentation Style

All public items must have doc comments — the pre-commit hook enforces
`-D missing_docs`. Brief one-liners are fine for self-evident items.

No "captain obvious" docs. Don't restate the struct/function name as the doc comment.
Only document when it adds value: non-obvious behavior, FreeSWITCH-specific semantics,
wire format details, gotchas. Silence over noise. If the name and signature tell the
whole story, a brief one-liner suffices.

## Correctness Over Recovery

Correctness is the highest priority. Never silently absorb protocol violations
or leave the system in an unknown state to "recover." If an invariant is broken
(e.g. missing mandatory header, impossible framing), return an error and let
the caller disconnect. A clean reconnection from a known-good state is always
preferable to continuing with a potentially corrupt stream.

Concretely: never use `unwrap_or` / default values to paper over missing
mandatory protocol fields. If the ESL spec says a field must be present,
its absence is a hard error — not a recoverable condition.

## Design Principles

### Transport layer (connection, protocol, event)

- **Split reader/writer**: Background reader task + channel-based event delivery.
  `EslClient` is Clone+Send for commands; `EslEventStream` for events.
- **Liveness detection**: Any inbound TCP traffic resets the timer. HEARTBEAT
  subscription ensures idle-connection traffic. `set_liveness_timeout()` to enable.
- **Command timeout**: Default 5s timeout on all commands. `set_command_timeout()`.
  Cleans up pending reply slot on timeout so subsequent commands aren't blocked.
- **No automatic reconnection**: The library detects disconnection via
  `ConnectionStatus`/`DisconnectReason`. The caller controls reconnection strategy.
- **Error classification**: `is_connection_error()` / `is_recoverable()` let callers
  decide handling without matching every variant.
- **Correct wire format**: Events use two-part framing (outer envelope + body).
  Header values are percent-decoded. Event format determined from Content-Type.

### Command builders (commands/, app/, variables/)

- **Pure `Display`/`FromStr`**: No transport coupling. Builders produce strings,
  `EslClient` calls `.to_string()`. Enables round-trip unit testing without ESL.
- **`app/`** = sendmsg-based dptools (answer, hangup, bridge, etc.)
- **`commands/`** = API command strings for `api()`/`bgapi()` (originate, uuid_*, conference)
- **`variables/`** = channel variable format parsers (ARRAY::, SIP multipart)
- **Foundation for extension**: Application-specific crates (NGCS, X-Call-Info, SIP
  URI) can depend on these base types without reimplementing escaping or parsing.

## Outbound ESL Mode

See [docs/outbound-esl-quirks.md](docs/outbound-esl-quirks.md) for details.

- `connect_session()` must be the first command after `accept_outbound()`
- `async full` mode required for api/bgapi/linger/event commands
- Socket app args need quoting in originate — `Originate` builder handles this
  via `originate_quote()`/`originate_unquote()` in `commands/mod.rs`
- `cargo run --example outbound_test` exercises outbound against real FS on port 8022

## Live Integration Tests

When FreeSWITCH ESL is available on `127.0.0.1:8022` (password `ClueCon`),
run the live tests after unit tests pass:

```sh
cargo test --test live_freeswitch -- --ignored
```

These tests exercise real ESL connections: auth, api commands, event
subscription, sendevent with priority/array headers, and round-trip
custom event delivery.

To check if FreeSWITCH is listening: `ss -tlnp sport = :8022`

## Development Methodology — TDD

This project follows test-driven development:

1. Write failing tests that reproduce the bug or specify the new behavior
2. Confirm tests fail (`cargo test --lib`)
3. `cargo fmt && git commit --no-verify` (red phase — clippy/tests will fail, but code must be formatted)
4. Implement the fix/feature
5. Confirm all tests pass
6. Commit the implementation (hooks run normally)
