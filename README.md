# FreeSWITCH ESL Rust Client

A comprehensive, async-first Rust client library for FreeSWITCH's Event Socket Library (ESL).

## Features

- **Full ESL Protocol Support**: Complete implementation of the FreeSWITCH ESL wire protocol
- **Async/Await**: Built on Tokio for high-performance async operations
- **Connection Modes**: Support for both inbound (client-to-FS) and outbound (FS-to-client) connections
- **Event Streaming**: Efficient event subscription and processing with multiple format support (Plain, JSON, XML)
- **Command Execution**: Full API command support including background jobs
- **Type Safety**: Comprehensive error handling and type-safe event structures
- **Zero-Copy Parsing**: Efficient buffer management for high-throughput applications
- **Thread Safe**: Designed for concurrent usage across multiple tasks

## Quick Start

Add this to your `Cargo.toml`:

```toml
[dependencies]
freeswitch-esl-rs = "0.1.0"
tokio = { version = "1.0", features = ["full"] }
```

### Inbound Connection Example

Connect to FreeSWITCH and execute commands:

```rust
use freeswitch_esl_rs::{EslHandle, EslError};

#[tokio::main]
async fn main() -> Result<(), EslError> {
    // Connect to FreeSWITCH
    let mut handle = EslHandle::connect("localhost", 8022, "ClueCon").await?;
    
    // Execute API command
    let response = handle.api("status").await?;
    println!("FreeSWITCH Status: {}", response.body_string());
    
    // Clean disconnect
    handle.disconnect().await?;
    Ok(())
}
```

### Event Subscription Example

Listen for FreeSWITCH events:

```rust
use freeswitch_esl_rs::{EslHandle, EslEventType, EventFormat};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut handle = EslHandle::connect("localhost", 8022, "ClueCon").await?;
    
    // Subscribe to events
    handle.subscribe_events(EventFormat::Plain, &[
        EslEventType::ChannelAnswer,
        EslEventType::ChannelHangup,
        EslEventType::Dtmf,
    ]).await?;
    
    // Process events
    while let Some(event) = handle.recv_event().await? {
        match event.event_type() {
            Some(EslEventType::ChannelAnswer) => {
                println!("Call answered: {}", event.unique_id().unwrap_or(&"unknown".to_string()));
            }
            Some(EslEventType::Dtmf) => {
                println!("DTMF: {}", event.header("DTMF-Digit").unwrap_or(&"unknown".to_string()));
            }
            _ => {}
        }
    }
    
    Ok(())
}
```

### Outbound Server Example

Accept connections from FreeSWITCH for call control:

```rust
use freeswitch_esl_rs::{EslHandle, command::AppCommand};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("0.0.0.0:8040").await?;
    
    while let Ok(mut handle) = EslHandle::accept_outbound(listener.try_clone().unwrap()).await {
        tokio::spawn(async move {
            // Answer the call
            handle.send_command(AppCommand::answer()).await.unwrap();
            
            // Play a message
            handle.send_command(AppCommand::playback("welcome.wav")).await.unwrap();
            
            // Hangup
            handle.send_command(AppCommand::hangup(Some("NORMAL_CLEARING"))).await.unwrap();
        });
    }
    
    Ok(())
}
```

## Examples

The repository includes comprehensive examples:

- **`inbound_client.rs`**: Connect to FreeSWITCH and execute various API commands
- **`outbound_server.rs`**: Accept outbound connections and handle call control
- **`event_listener.rs`**: Subscribe to and process FreeSWITCH events with call tracking

Run examples with:

```bash
cargo run --example inbound_client
cargo run --example outbound_server
cargo run --example event_listener
```

## API Reference

### Connection Management

- `EslHandle::connect(host, port, password)` - Inbound connection
- `EslHandle::connect_with_user(host, port, user, password)` - User-based auth
- `EslHandle::accept_outbound(listener)` - Accept outbound connections
- `handle.disconnect()` - Clean disconnection

### Command Execution

- `handle.api(command)` - Synchronous API commands
- `handle.bgapi(command)` - Background API commands
- `handle.execute(app, args, uuid)` - Execute dialplan applications
- `handle.sendmsg(uuid, event)` - Send messages to channels

### Event Handling

- `handle.subscribe_events(format, events)` - Subscribe to events
- `handle.filter_events(header, value)` - Filter events
- `handle.recv_event()` - Receive next event
- `handle.recv_event_timeout(ms)` - Receive with timeout

### Application Commands

Pre-built commands for common operations:

- `AppCommand::answer()` - Answer call
- `AppCommand::hangup(cause)` - Hang up call
- `AppCommand::playback(file)` - Play audio file
- `AppCommand::bridge(destination)` - Bridge channels
- `AppCommand::transfer(extension, dialplan, context)` - Transfer call
- `AppCommand::park()` - Park call

## Event Types

The library supports all 143+ FreeSWITCH event types including:

- Channel events: `CHANNEL_CREATE`, `CHANNEL_ANSWER`, `CHANNEL_HANGUP`
- DTMF events: `DTMF`
- System events: `HEARTBEAT`, `BACKGROUND_JOB`
- And many more...

## Event Formats

- **Plain**: Default text format for high performance
- **JSON**: Structured data for complex processing
- **XML**: XML format when needed for compatibility

## Error Handling

Comprehensive error types:

- `EslError::Io` - Network/IO errors
- `EslError::NotConnected` - Connection state errors
- `EslError::AuthenticationFailed` - Authentication errors
- `EslError::CommandFailed` - Command execution errors
- `EslError::Timeout` - Operation timeout errors
- And more...

## Testing

Run the test suite:

```bash
# Unit tests
cargo test

# Integration tests (requires FreeSWITCH running on localhost:8022)
cargo test --ignored
```

## Requirements

- Rust 1.70+
- Tokio async runtime
- FreeSWITCH with ESL enabled

## FreeSWITCH Configuration

For inbound connections, ensure FreeSWITCH has ESL configured in `autoload_configs/event_socket.conf.xml`:

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

For outbound connections, use in your dialplan:

```xml
<action application="socket" data="your.app.host:8040 async full"/>
```

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## Acknowledgments

This library is based on the FreeSWITCH ESL protocol and inspired by the official C ESL library implementation.