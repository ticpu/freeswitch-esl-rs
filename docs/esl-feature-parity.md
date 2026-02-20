# ESL Feature Gaps vs C libesl

Reference: FreeSWITCH `libs/esl/` and `src/mod/event_handlers/mod_event_socket/`.

## Protocol Commands

### High Priority (outbound mode essentials)

- `myevents [uuid] [format]` — filter events to session UUID only
- `linger [seconds]` — keep socket open after hangup to drain events
- `nolinger` — cancel linger mode
- `resume` — resume dialplan execution after socket disconnect

### Medium Priority

- `nixevent <event1> [event2...]` — unsubscribe specific events
- `noevents` — unsubscribe all events, flush queue
- `filter delete <header> [value]` — remove event filters

### Low Priority

- `divert_events on|off` — redirect internal session events to ESL socket
- `getvar <variable>` — read channel variable (outbound shortcut, `api uuid_getvar` works)

## Event Construction

- `EslEvent::set_priority()` — event priority (NORMAL/LOW/HIGH)
- Header array support (PUSH/UNSHIFT stack modes for multi-value headers)
