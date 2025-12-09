# FreeSWITCH Directory User Configuration for ESL

This document describes how to configure FreeSWITCH directory users for ESL (Event Socket Layer) authentication using the `userauth` command.

## Overview

FreeSWITCH supports per-user ESL authentication through the directory XML configuration. This allows fine-grained control over which events and API commands each user can access.

## User Authentication Format

When connecting with userauth, the format is:

```
userauth user@domain:password
```

An alternative format is also supported:

```
userauth user:password@domain
```

For example:

```
userauth admin@default:MySecretPassword
userauth admin:MySecretPassword@default
```

The domain must match the domain name in your FreeSWITCH directory configuration.

## ESL Parameters

The following parameters can be set in the `<params>` section of a user, group, or domain configuration:

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `esl-password` | string | (required) | Password for ESL authentication |
| `esl-allowed-events` | string | `all` | Comma/space-separated list of allowed event types |
| `esl-allowed-api` | string | `all` | Comma/space-separated list of allowed API commands |
| `esl-allowed-log` | boolean | `true` | Whether the user can receive log output |
| `esl-disable-command-logging` | boolean | `false` | Disable logging of commands from this user |

### Parameter Details

#### esl-password

The password required for authentication. Must match exactly what is provided in the `userauth` command.

#### esl-allowed-events

Controls which events the user can subscribe to.

Values:

- `all` - Allow all events including custom events (default)
- Space-separated or comma-separated list of event names (not both)

The delimiter is automatically detected: if the value contains a space, space is used as delimiter; otherwise comma is used.

Standard FreeSWITCH event types: `CHANNEL_CREATE`, `CHANNEL_DESTROY`, `CHANNEL_STATE`, `CHANNEL_CALLSTATE`, `CHANNEL_ANSWER`, `CHANNEL_HANGUP`, `CHANNEL_HANGUP_COMPLETE`, `CHANNEL_EXECUTE`, `CHANNEL_EXECUTE_COMPLETE`, `CHANNEL_BRIDGE`, `CHANNEL_UNBRIDGE`, `CHANNEL_PROGRESS`, `CHANNEL_PROGRESS_MEDIA`, `CHANNEL_OUTGOING`, `CHANNEL_PARK`, `CHANNEL_UNPARK`, `CHANNEL_APPLICATION`, `CHANNEL_ORIGINATE`, `CHANNEL_UUID`, `API`, `LOG`, `INBOUND_CHAN`, `OUTBOUND_CHAN`, `STARTUP`, `SHUTDOWN`, `PUBLISH`, `UNPUBLISH`, `TALK`, `NOTALK`, `SESSION_HEARTBEAT`, `CLIENT_DISCONNECTED`, `SERVER_DISCONNECTED`, `SEND_INFO`, `RECV_INFO`, `RECV_RTCP_MESSAGE`, `SEND_MESSAGE`, `RECV_MESSAGE`, `REQUEST_PARAMS`, `CHANNEL_DATA`, `GENERAL`, `COMMAND`, `SESSION_DATA`, `MESSAGE`, `PRESENCE_IN`, `PRESENCE_OUT`, `PRESENCE_PROBE`, `MESSAGE_WAITING`, `MESSAGE_QUERY`, `ROSTER`, `CODEC`, `BACKGROUND_JOB`, `DETECTED_SPEECH`, `DETECTED_TONE`, `PRIVATE_COMMAND`, `HEARTBEAT`, `TRAP`, `ADD_SCHEDULE`, `DEL_SCHEDULE`, `EXE_SCHEDULE`, `RE_SCHEDULE`, `RELOADXML`, `NOTIFY`, `PHONE_FEATURE`, `PHONE_FEATURE_SUBSCRIBE`, `MODULE_LOAD`, `MODULE_UNLOAD`, `DTMF`, `SESSION_CRASH`, `TEXT`, `CUSTOM`, `ALL`.

Custom event subclasses from modules are also supported (see section below).

#### esl-allowed-api

Controls which API commands the user can execute.

Values:

- `all` - Allow all API commands (default)
- Empty string `""` - Disable all API access
- Space-separated or comma-separated list of command names (not both)

The delimiter is automatically detected: if the value contains a space, space is used as delimiter; otherwise comma is used.

**Important:** Only the command name (first word) is checked. Subcommands cannot be filtered individually. For example, allowing `sofia` permits all sofia subcommands (`sofia status`, `sofia profile`, `sofia xmlstatus`, etc.).

Example restricted list: `show sofia status version uptime`

#### esl-allowed-log

Boolean (`true`/`false`) controlling whether the user can receive log output. Default: `true`.

#### esl-disable-command-logging

Boolean (`true`/`false`) to suppress logging of commands executed by this user. Default: `false`.

## Configuration Hierarchy

Parameters can be set at three levels (in order of precedence, later overrides earlier):

1. Domain level (`<domain>` → `<params>`)
2. Group level (`<group>` → `<params>`)
3. User level (`<user>` → `<params>`)

## Example Configurations

### Basic User with Full Access

```xml
<user id="admin">
  <params>
    <param name="esl-password" value="SuperSecretPassword"/>
  </params>
</user>
```

### Restricted Monitoring User

```xml
<user id="monitor">
  <params>
    <param name="esl-password" value="MonitorPass123"/>
    <param name="esl-allowed-events" value="CHANNEL_CREATE CHANNEL_ANSWER CHANNEL_HANGUP"/>
    <param name="esl-allowed-api" value="show status version uptime"/>
    <param name="esl-allowed-log" value="false"/>
  </params>
</user>
```

### Event-Only User (No API Access)

```xml
<user id="events">
  <params>
    <param name="esl-password" value="EventsOnly"/>
    <param name="esl-allowed-events" value="all"/>
    <param name="esl-allowed-api" value=""/>
    <param name="esl-allowed-log" value="true"/>
  </params>
</user>
```

### Domain-Wide Defaults with User Override

```xml
<domain name="default">
  <params>
    <param name="esl-allowed-log" value="false"/>
    <param name="esl-allowed-api" value="show status version"/>
  </params>

  <groups>
    <group name="admins">
      <params>
        <param name="esl-allowed-log" value="true"/>
      </params>
      <users>
        <user id="superadmin">
          <params>
            <param name="esl-password" value="SuperAdmin123"/>
            <param name="esl-allowed-api" value="all"/>
          </params>
        </user>
      </users>
    </group>

    <group name="operators">
      <users>
        <user id="operator1">
          <params>
            <param name="esl-password" value="Operator1Pass"/>
          </params>
        </user>
      </users>
    </group>
  </groups>
</domain>
```

## Authentication Response

Upon successful authentication, FreeSWITCH returns the granted permissions:

```
Content-Type: command/reply
Reply-Text: +OK accepted
Allowed-Events: all
Allowed-API: show sofia status version uptime
Allowed-LOG: true
```

## Connecting with freeswitch-esl-rs

```rust
use freeswitch_esl_rs::EslHandle;

// Connect with userauth
let handle = EslHandle::connect_with_user(
    "localhost",
    8021,
    "admin@default",      // user@domain format required
    "SuperSecretPassword"
).await?;
```

Or using the event_filter example:

```bash
cargo run --example event_filter -- -u admin@default -p SuperSecretPassword -e ALL
```

## Custom Event Subclasses

Custom events use the `CUSTOM` event type with a subclass name. These can be used in `esl-allowed-events`.

### mod_sofia Events

`sofia::register`, `sofia::pre_register`, `sofia::register_attempt`, `sofia::register_failure`, `sofia::unregister`, `sofia::expire`, `sofia::gateway_state`, `sofia::gateway_add`, `sofia::gateway_delete`, `sofia::gateway_invalid_digest_req`, `sofia::sip_user_state`, `sofia::notify_refer`, `sofia::reinvite`, `sofia::recovery_recv`, `sofia::recovery_send`, `sofia::recovery_recovered`, `sofia::error`, `sofia::profile_start`, `sofia::notify_watched_header`, `sofia::wrong_call_state`, `sofia::transferor`, `sofia::transferee`, `sofia::replaced`, `sofia::intercepted`, `sofia::bye_response`.

### mod_conference Events

`conference::maintenance`, `conference::cdr`.

## File Location

User configurations are typically stored in:

- `/etc/freeswitch/directory/default/*.xml` (Debian/Ubuntu)
- `/usr/local/freeswitch/conf/directory/default/*.xml` (source install)

The domain name in the directory XML must match the domain portion of the `user@domain` authentication string.
