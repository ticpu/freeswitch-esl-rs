# Examples

Start with `inbound_client`, then `event_listener`. Between them they cover the
whole shape of an ESL client: authenticate, run a command and read its reply,
and process the stream FreeSWITCH pushes back.

All of them read `ESL_HOST`, `ESL_PORT` and `ESL_PASSWORD`, defaulting to
`localhost:8021` with the stock password. `ESL_HOST` takes a bare IPv6 literal
without brackets.

| Example | What it teaches | Needs |
|---|---|---|
| `inbound_client` | Connect, `api` and its two failure layers, `bgapi` correlated with `BgJobTracker` | a switch |
| `event_listener` | `EventSubscription`, per-call state off the event stream, `-d` wire dump | a switch |
| `event_filter` | Server-side `filter`, userauth, a bounded run, four output formats | a switch |
| `reconnecting_client` | Error classification as policy: backoff, `EX_CONFIG`, liveness gated on a heartbeat subscription that may be denied | a switch |
| `channel_tracker` | `HeaderLookup` on your own type, and rebuilding channel state from `uuid_dump` | a switch |
| `codec_monitor` | CODEC event headers, which are not channel variables; transcoding across a bridge | a switch with calls |
| `originate_examples` | Every endpoint type and targeting mode as a wire string (part 1 needs nothing), then a live call | part 2 needs a switch |
| `outbound_server` | Outbound call control: `myevents` scoping, `linger`, an IVR | a dialplan `socket` action |
| `outbound_test` | The outbound verbs in order, driving itself through a loopback call | `mod_loopback`, ext 9199 in context `test` |
| `bgapi_bench` | `bgapi` throughput and round-trip latency | a switch |
| `reexec_demo` | Handing the authenticated socket to a new binary image without dropping the event stream | Unix, a switch |
