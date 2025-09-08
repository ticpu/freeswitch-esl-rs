# fs_cli-rs

Interactive FreeSWITCH CLI client written in Rust using the `freeswitch-esl-rs` library.

## Features

- 🚀 **Async/Modern**: Built with Tokio for high performance
- 📝 **Readline Support**: Full command history, editing, and tab completion
- 🎨 **Colorized Output**: Beautiful colored output (can be disabled)
- ⚡ **Tab Completion**: Smart completion for FreeSWITCH commands
- 📚 **Command History**: Persistent history with search
- 🔧 **Flexible Connection**: Support for custom host/port/password/user
- 📊 **Real-time Logging**: Optional debug logging support

## Installation

```bash
# From the freeswitch-esl-rs directory
cd fs_cli-rs
cargo build --release
```

## Usage

### Basic Connection

```bash
# Connect to local FreeSWITCH with default settings
./target/release/fs_cli

# Connect to remote FreeSWITCH
./target/release/fs_cli -H 192.168.1.100 -P 8022 -p mypassword

# Connect with username authentication
./target/release/fs_cli -H localhost -u admin -p secret
```

### Command Line Options

```
fs_cli-rs 0.1.0
Interactive FreeSWITCH CLI client

USAGE:
    fs_cli [OPTIONS]

OPTIONS:
    -H, --host <HOST>              FreeSWITCH hostname or IP address [default: localhost]
    -P, --port <PORT>              FreeSWITCH ESL port [default: 8022]
    -p, --password <PASSWORD>      ESL password [default: ClueCon]
    -u, --user <USER>              Username for authentication (optional)
    -d, --debug                    Enable debug logging
        --no-color                 Disable colored output
    -x <EXECUTE>                   Execute single command and exit
        --history-file <HISTORY_FILE>  History file path
    -t, --timeout <TIMEOUT>        Connection timeout in seconds [default: 10]
        --events                   Subscribe to events on startup
    -h, --help                     Print help information
    -V, --version                  Print version information
```

### Examples

```bash
# Execute single command and exit (fs_cli compatibility)
./target/release/fs_cli -P 8022 -x "sofia status"

# Execute single command with custom password
./target/release/fs_cli -p mypassword -x "status"

# Connect with debug logging
./target/release/fs_cli -d

# Connect and subscribe to events
./target/release/fs_cli --events

# Use custom history file
./target/release/fs_cli --history-file /tmp/my_fs_history

# Disable colors for scripting
./target/release/fs_cli --no-color -x "show channels"
```

## Interactive Usage

Once connected, you can use any FreeSWITCH API command:

```
freeswitch@localhost> status
Executing: status
FreeSWITCH Version 1.10.9-release (git a3c6a43 2023-01-01 10:00:00Z 64bit)

Uptime: 0 years, 0 days, 1 hour, 23 minutes, 45 seconds, 123 milliseconds, 456 microseconds
FreeSWITCH (Version 1.10.9-release git a3c6a43 2023-01-01 10:00:00Z 64bit) is ready
1 session(s) since startup
0 session(s) - peak 1, last 5min 0
0 session(s) per Sec out of max 30, peak 1, last 5min 0
1000 session(s) max
min idle cpu 0.00/99.33

Command completed in 15.23ms

freeswitch@localhost> show channels
Executing: show channels
uuid,direction,created,created_epoch,name,state,cid_name,cid_num,ip_addr,dest,application,application_data,dialplan,context,read_codec,write_codec,secure,hostname,presence_id,presence_data,accountcode,callstate,callee_name,callee_num,callee_direction,call_uuid,sent_callee_name,sent_callee_num

0 total.

Command completed in 8.45ms

freeswitch@localhost> help
FreeSWITCH CLI Commands:

Basic Commands:
  status                    - Show system status
  version                   - Show FreeSWITCH version
  uptime                    - Show system uptime
  help                      - Show this help

Show Commands:
  show channels             - List active channels
  show channels count       - Show channel count
  show calls                - Show active calls
  show registrations        - Show SIP registrations
  show modules              - List loaded modules
  show interfaces           - Show interfaces
...
```

### Built-in Commands

- **help** - Show available commands
- **history** - Show command history
- **clear** - Clear screen
- **quit/exit/bye** - Exit the CLI

### Tab Completion

The CLI supports tab completion for:
- FreeSWITCH API commands
- Common command patterns
- File paths (where appropriate)

Use `Tab` to complete commands and `Up/Down` arrows to navigate history.

### Enhanced Commands

Some commands have enhanced formatting:

- **status** - Colorized status with uptime highlighting
- **show channels** - Column-formatted with state coloring
- **show calls** - Enhanced call display
- **version** - Clean version output
- **uptime** - Extract just uptime info from status

## Logging

Enable debug logging to see detailed ESL protocol information:

```bash
./target/release/fs_cli -d
```

This will show:
- Connection establishment
- Command execution timing
- ESL protocol messages
- Error details

## History

Command history is automatically saved to `~/.fs_cli_history` (or specify with `--history-file`).

The history includes:
- All executed commands
- Timestamps
- Deduplication of consecutive identical commands

## Configuration

### FreeSWITCH ESL Configuration

Ensure your FreeSWITCH has ESL enabled in `autoload_configs/event_socket.conf.xml`:

```xml
<configuration name="event_socket.conf" description="Socket Client">
  <settings>
    <param name="nat-map" value="false"/>
    <param name="listen-ip" value="127.0.0.1"/>
    <param name="listen-port" value="8022"/>
    <param name="password" value="ClueCon"/>
    <param name="apply-inbound-acl" value="loopback.auto"/>
  </settings>
</configuration>
```

## Development

To build from source:

```bash
git clone <repository>
cd freeswitch-esl-rs/fs_cli-rs
cargo build --release
```

For development with hot reload:

```bash
cargo run -- --help
cargo run -- -H localhost status
```

## License

Same as parent project (MIT OR Apache-2.0).