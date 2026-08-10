# FreeSWITCH Dial String Format

Reference for endpoint strings, variable scoping, and bridge semantics as they
appear on the ESL wire and in FreeSWITCH configuration. Based on FreeSWITCH
1.10.x source code (`switch_ivr_originate.c`, `mod_sofia.c`, `mod_loopback.c`,
`mod_dptools.c`). Line numbers index FreeSWITCH `v1.11.1`
(commit `c2c59645f6911a76589e5008c4d73349ded44b65`).

## Endpoint types

FreeSWITCH endpoints are **module-specific**. The core splits the dial string
on the first `/`, looks up the module name in an endpoint hash table
(`switch_loadable_module_get_endpoint_interface()`), and delegates all
remaining parsing to the module's `outgoing_channel` callback. There is no
universal endpoint grammar -- each module defines its own format.

### sofia (SIP)

Direct profile routing:

```
sofia/{profile}/{destination}
sofia/internal/1000@pbx.example.com
sofia/external/18005551234@carrier.example.com
```

Gateway routing (uses pre-configured gateway credentials and transport):

```
sofia/gateway/{gateway_name}/{destination}
sofia/gateway/my_provider/18005551234
```

Gateway with explicit profile qualifier:

```
sofia/gateway/{profile}::{gateway_name}/{destination}
sofia/gateway/internal::my_provider/18005551234
```

Source: `mod_sofia.c` -- checks if remainder starts with `gateway/`, then
parses as 3-part `gateway/name/dest`; otherwise parses as 2-part
`profile/dest` and formats the destination as a SIP URI.

### loopback

Re-enters the dialplan on a new channel pair. Useful for applying dialplan
logic to an originated call or for codec renegotiation.

```
loopback/{extension}[/{context}[/{dialplan}]]
loopback/9199
loopback/9199/default
loopback/9199/default/xml
loopback/app=lua:script.lua
```

Context defaults to `"default"`, dialplan defaults to `"xml"`.

Source: `mod_loopback.c` -- checks for `app=` prefix (direct application
execution), otherwise splits on `/` for extension/context/dialplan.

### user

Directory-based endpoint. Resolves via the user's `dial-string` parameter
in the FreeSWITCH directory XML. Typically expands to a `sofia_contact()`
expression.

```
user/{name}[@{domain}]
user/1000
user/bob@pbx.example.com
```

Common directory `dial-string` configuration:

```xml
<param name="dial-string"
  value="{^^:sip_invite_domain=${dialed_domain}:presence_id=${dialed_user}@${dialed_domain}}${sofia_contact(*/${dialed_user}@${dialed_domain})}"/>
```

### error

Pseudo-endpoint that terminates the call with a specific hangup cause.
Used in bridge dial strings for explicit failure routing.

```
error/{hangup_cause}
error/user_busy
error/user_not_registered
error/no_route_destination
```

### group

Directory-based group endpoint. Resolves to the `group_call()` API function
result.

```
group/{group_name}@{domain}
group/support@pbx.example.com
```

Equivalent to `${group_call(support@pbx.example.com)}`.

### portaudio / pulseaudio / alsa

Audio device endpoints for local sound hardware. The destination is optional
and typically `auto_answer` or a device identifier.

```
portaudio
portaudio/auto_answer
pulseaudio
pulseaudio/auto_answer
alsa
alsa/auto_answer
```

All three share the same `AudioEndpoint` struct in the library and differ
only in the module prefix.

## Variable scoping

Channel variables can be set on the B-leg (destination) of an originate or
bridge via bracket notation in the dial string. Three bracket types exist
with different scopes, listed in order of precedence (highest first):

### `[k=v]` -- channel (local) scope

Applies only to the **immediately following endpoint**. Highest precedence --
overrides global and enterprise variables.

```
[origination_caller_id_number=1234]sofia/internal/1000@domain
```

Multiple blocks accumulate:

```
[var1=a][var2=b]sofia/internal/1000@domain
```

### `{k=v}` -- default (global) scope

Applies to **all endpoints** in the current originate/bridge set. Multiple
blocks accumulate.

```
{hangup_after_bridge=true}sofia/gateway/gw/1234
{ignore_early_media=true}{call_timeout=30}sofia/gateway/gw/1234
```

### `<k=v>` -- enterprise (ultra-global) scope

Applies across **all threads** in an enterprise originate (`:_:` separated
sections). Lowest precedence.

```
<originate_timeout=60>{thread1_var=a}endpoint1:_:{thread2_var=b}endpoint2
```

### Combined example

```
<ultra_global=1>{thread_global=2}[per_endpoint=3]sofia/internal/1000@domain
```

Effective variables on the channel: `ultra_global=1`, `thread_global=2`,
`per_endpoint=3`. If a key appears in multiple scopes, the narrower scope wins.

## Variable value escaping

How much escaping a value needs depends on **which command carries the block**,
because the switch escape-processes it a different number of times per carrier.
See [Parse depth](#parse-depth) below before relying on any of the forms here;
the rules in this section are what the `freeswitch-esl-tokio` crate emits, and
were measured against a live switch rather than derived from the source.

### Backslash escaping

A comma is escaped with a backslash, on either carrier:

```
{sip_h_Call-Info=<url>;meta=123\,<uri>}endpoint
```

A literal backslash needs **eight**, on either carrier. Fewer and the switch
reads the sequence as an escape and substitutes the character it names, so
`a\nb` arrives carrying a newline:

```
{path=C:\\\\\\\\Users}endpoint
```

A single quote is the one rule that differs by carrier — two backslashes
through a dialplan application, three through the `originate` API:

```
{greeting=it\\'s_me}endpoint      <- dialplan: bridge, sendmsg execute
{greeting=it\\\'s_me}endpoint     <- api originate, bgapi originate
```

No count satisfies both: sweeping one to nine backslashes, each carrier succeeds
only at counts the other fails. A block carrying a single quoted value is more
forgiving than one carrying two, so test with two.

Values containing spaces are wrapped in single quotes. Those wrapping quotes are
balanced, so unlike a quote *inside* a value they behave identically on both
carriers:

```
{sip_h_X-Info='value with spaces'}endpoint
```

### Values that cannot be expressed at all

- **An empty value.** `{k=}` never reaches the channel: the switch splits the
  pair on `=`, requires exactly two fields, and `k=` yields one. The only
  `switch_log_printf` in that loop is inside the successful branch, so nothing
  is logged at any level. Quoting does not help — `k=''`, `k=\'\'` and
  `k=\\'\\'` were all measured discarded on both carriers.
- **A value closing a bracket it never opened.** `switch_find_end_paren` counts
  depth and honours no escape while doing so, so a lone `}`, `]` or `>` ends the
  block early and the remainder becomes dial-string text. A balanced pair such
  as `${var}` is fine and ordinary.

### `^^X` block separator

Placed **immediately after the opening bracket**, `^^` and a replacement
character change the separator between pairs for the whole block, so values may
contain commas with no escaping:

```
{^^:sip_invite_domain=example.com:presence_id=bob@example.com}endpoint
{^^:codecs=PCMA,PCMU,G729:tenant=acme}endpoint
```

This is the only mechanism available when values arrive by `${...}` expansion,
because substitution happens *before* the block is parsed and no escaping can be
inserted into the result. It works identically on both carriers.

It does not help with a quoted value: the quote pairing suppresses splitting on
whichever separator is in use, so two quoted values still merge.

**A `^^X` prefix on an individual value is not a general mechanism.** Writing
`{k=^^:a:b}` sets `k` to the literal `^^:a:b` — measured on both carriers. Only
consumers that specifically decode it, such as the codec-string parser reading
`absolute_codec_string`, interpret the form; the bracket parser stores it
verbatim.

### Parse depth

`switch_event_create_brackets` tokenizes a block **twice on its own** — once
splitting the pairs on the separator, once splitting each pair on `=` — and both
calls run the full quote-stripping, backslash-consuming cleanup over their
results. Reached through the `originate` API a third pass applies, because
`mod_commands` splits its argument list first with a tokenizer that treats a
quote as opening a quoted region regardless of any delimiter.

| Carrier | Passes |
|---|---|
| `bridge` and other dialplan applications, incl. `sendmsg execute` | 2 |
| `api originate`, `bgapi originate` | 3 |

Consequences worth knowing before hand-writing a block:

- An unescaped quote reaching the `originate` argv pass turns off space
  splitting for the rest of the line, so the command fails with a usage error
  rather than corrupting a value. That is the loud case.
- Two quotes that survive to the block parse pair with each other across the
  separator, so the pair between them is not split: the *first* value absorbs
  the second and the second is never set. The variable that goes missing is not
  the one that contained the quote.
- A log line is not evidence either way. `mod_logfile` splits its own output
  with the same tokenizer, so a value is mangled in the log whether or not it
  was mangled on the wire. Read values back with `uuid_getvar` or `uuid_dump`.

## Bridge separators

Bridge and originate dial strings support multiple endpoints with different
failure/concurrency semantics.

### `,` -- simultaneous ring (forked dialing)

All endpoints in a comma-separated group ring at the same time. The first
endpoint to **provide media** (answer or early media) wins; others stop
ringing.

```
sofia/internal/100@domain,sofia/internal/101@domain
```

Use `ignore_early_media=true` on the A-leg to prevent early media (ringback,
music) from prematurely selecting a winner -- common with cell phones.

### `|` -- sequential failover

Endpoints separated by pipe are tried **one at a time**, in order. The next
endpoint is tried only after the previous one fails.

```
sofia/gateway/primary/1234|sofia/gateway/secondary/1234|sofia/gateway/backup/1234
```

### `:_:` -- enterprise originate

Each `:_:`-separated section is originated in a **separate thread**.
Enterprise-scope `<>` variables apply across all threads. Each thread can
have its own `{}` global variables.

```
<originate_timeout=30>{thread1_cid=100}sofia/gw/a/1234:_:{thread2_cid=200}sofia/gw/b/1234
```

Constant: `SWITCH_ENT_ORIGINATE_DELIM = ":_:"` in `switch_types.h`.

### Combined example

```
{hangup_after_bridge=true}[t=10]sofia/gw/a/1234,[t=10]sofia/gw/b/1234|sofia/gw/backup/1234
```

Ring gateways `a` and `b` simultaneously (10s timeout each). If both fail,
try `backup` sequentially.

## Runtime expressions

FreeSWITCH supports `${}` variable expansion in dial strings. Some
expressions resolve to endpoint strings at call time.

### `sofia_contact`

Resolves the current registered SIP contact URI for a directory user. Returns
`error/user_not_registered` if no active registration.

```
${sofia_contact(user@domain)}
${sofia_contact(profile/user@domain)}
${sofia_contact(*/user@domain)}
```

The `*` searches all profiles. An optional `~user_agent` suffix filters by
User-Agent header.

Source: `sofia_contact_function` (`mod_sofia.c:4105-4236`).

### `group_call`

Resolves directory group members to a multi-endpoint dial string.

```
${group_call(group@domain)}
${group_call(group@domain+A)}
${group_call(group@domain+E)}
${group_call(group@domain+F)}
```

Flags: `A` = all (simultaneous), `E` = enterprise (`:_:` separated),
`F` = first match only.

### `eval` prefix (API evaluation)

Some applications evaluate an `eval` prefix by calling the FreeSWITCH API
first, then using the result as the dial string:

```
eval ${group_call(calltakers@${domain_name}+A)}
```

This is an application-level convention, not a core FreeSWITCH feature.

## Special bridge features

### `^` -- SIP To: header override

Appended after `@host` to override the To: header in the outbound SIP INVITE.
Useful for number portability routing where the Request-URI needs extra
parameters but the To: header should contain the clean number.

```
sip:12135551212;rn=12135550000;npdi=yes@1.2.3.4:5060^12135551212
```

### Bridge control variables

Common variables that affect bridge behavior (set on the A-leg before bridge):

| Variable | Effect |
|----------|--------|
| `call_timeout` | Seconds to wait for answer |
| `originate_timeout` | Per-endpoint timeout in originate |
| `hangup_after_bridge` | Hang up A-leg after B-leg disconnects |
| `bypass_media` | SDP passthrough (RTP flows directly between endpoints) |
| `ignore_early_media` | Don't select winner on early media (183/180+SDP) |
| `ringback` | Play tone/file to A-leg during ringing |
| `transfer_ringback` | Play during attended transfer |
| `fail_on_single_reject` | Fail entire bridge if any endpoint rejects |
| `hangup_on_single_reject` | Hang up if any endpoint rejects |
| `continue_on_fail` | Continue dialplan after bridge failure |
| `bridge_early_media` | Bridge early media to A-leg |

## Wire format in ESL

When constructing dial strings via ESL (`api originate`, `bgapi originate`,
`sendmsg execute bridge`), the complete format is:

```
originate <[vars]endpoint> <app> [dialplan] [context] [cid_name] [cid_num] [timeout]
```

For bridge (via sendmsg):

```
execute bridge <[vars]endpoint[,endpoint][|endpoint]>
```

Application arguments containing spaces must be single-quoted in originate:

```
originate loopback/9199/test '&socket(127.0.0.1:8040 async full)'
```

The `freeswitch-esl-tokio` library handles this quoting automatically via
`originate_quote()` / `originate_unquote()`.

These two lines are the two carriers of [Parse depth](#parse-depth), and the
typed API picks the right escaping for each without being told: `Originate`
renders for the API carrier, `BridgeDialString` for the dialplan one. `Display`
on a bare `Variables` or `Endpoint` means the API carrier, because that is what
this crate mostly drives — so a block rendered on its own and spliced into a
dialplan string by hand is the one case that needs
`display_for(DialStringCarrier::Dialplan)` and will otherwise be escaped one
level too deep.
