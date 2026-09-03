# Deferred to the next major

Changes that are right but cannot ship under the current major. Read this when
bumping `freeswitch-types` to 2.0 or `freeswitch-esl-tokio` to 3.0, and delete
each entry as it lands.

Every entry names a symbol. If the symbol is gone, the entry is stale — remove
it rather than guess what it meant. Nothing here is a promise; a decision may be
revisited when it is finally actionable.

## freeswitch-types 2.0

### `AudioEndpoint::fmt_with_prefix` should be private

It takes a `&mut fmt::Formatter`, which a caller can only obtain inside a
`Display` impl, and nothing outside the module calls it. It stays public only
because it shipped in 1.4.0; the carrier-aware `write_with_prefix` beside it is
already `pub(super)`. Make the public one private and drop the duplicate.

### `DialString: fmt::Display` forces an invalid endpoint string

The supertrait bound obliges `AudioEndpoint` to have a `Display` impl, and that
impl has no module prefix to render, so it emits `audio` — not a FreeSWITCH
endpoint. Its own rustdoc warns against calling it. Either drop the `Display`
bound from `DialString`, or drop the impl and let the three `Endpoint` variants
be the only way to render an audio endpoint.

### `Variables::insert` and `with_vars` need to be fallible

Some values cannot be represented in a bracket block at all: an empty value is
discarded by the switch under every encoding, and a value carrying an unbalanced
closing bracket truncates the block. A value carrying the block's own `^^`
separator is the third: `with_separator` checks the values present when it is
called, and an `insert` after it splits into a pair nobody wrote.
Refusing them is the only correct handling,
and it cannot live at render time — `Display` is infallible and `ToString`
panics on a `fmt::Error`, which would put a panic in a library. Until these
return `Result`, such a value is built silently and lost on the wire.

`Deserialize` and `FromStr` already return `Result`, so they can reject at the
boundary without waiting for the major — that covers config-driven construction
but not the programmatic path.

### Consider removing `Display` for `Variables` and `Endpoint`

Both render for `DialStringCarrier::EslApi`, which is right for what this crate
mostly drives but silently wrong for a block hand-spliced into a dialplan
string. Removing the impls would force `display_for(carrier)` at every call
site, making a wrong carrier unrepresentable rather than merely unlikely. The
cost is ergonomic and it breaks `format!("{vars}")` everywhere, so it is worth
weighing against how often the default is actually wrong in practice.

## freeswitch-esl-tokio 3.0

### `subscribe_events_raw` and `nixevent_raw` should return what they swallowed

Both send their token list verbatim, and `CUSTOM` is terminal, so an event type
placed after it is registered as a subclass name and never subscribed — with
`+OK` on the wire and nothing in the reply to say so. `swallowed_event_types`
detects that case today, but only for a caller who already knows to ask.

Returning it instead of `()`, behind a `#[must_use]` carrier, puts the warning
in front of a caller who does not: existing `.await?;` call sites keep
compiling and start warning. It waits for the major because changing the `Ok`
type is a break, not because the diagnostic is optional.

### It inherits whatever `freeswitch-types` 2.0 breaks

The types above are re-exported from the crate root, so any change to their
public surface breaks this crate's too. There is no separate work item — the
bump is the work. Sequence the release accordingly: `freeswitch-types` 2.0
publishes first, then this crate.
