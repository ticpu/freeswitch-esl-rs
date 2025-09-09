# FreeSWITCH ESL Rust Client

A comprehensive, async-first Rust client library for FreeSWITCH's Event Socket Library (ESL).

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
    let mut handle = EslHandle::connect("localhost", 8021, "ClueCon").await?;

    // Execute API command
    let response = handle.api("status").await?;
    println!("FreeSWITCH Status: {}", response.body_string());

    // Clean disconnect
    handle.disconnect().await?;
    Ok(())
}
```

## fs_cli-rs

This repository also includes `fs_cli-rs`, a modern Rust re-implementation of FreeSWITCH's `fs_cli` command-line interface. It provides an interactive CLI client with enhanced features like:

- **Async/Modern**: Built with Tokio for high performance
- **Readline Support**: Full command history, editing, and tab completion
- **Colorized Output**: Beautiful colored output
- **Tab Completion**: Smart completion for FreeSWITCH commands
- **Command History**: Persistent history with search

See [`fs_cli-rs/README.md`](fs_cli-rs/README.md) for installation and usage details.

*Note: fs_cli-rs may be split into a separate repository in the future.*

## Requirements

- Rust 1.70+
- Tokio async runtime
- FreeSWITCH with ESL enabled

## License

This project is licensed under the Mozilla Public License 2.0 - see the [LICENSE](LICENSE) file for details.

The MPL-2.0 is a copyleft license that allows you to combine this library with proprietary code while ensuring that modifications to the library itself remain open source.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## Acknowledgments

This library is based on the FreeSWITCH ESL protocol and inspired by the official C ESL library implementation.
