//! FreeSWITCH Event Socket Library (ESL) client for Rust
//!
//! This crate provides an async Rust client for FreeSWITCH's Event Socket Library (ESL),
//! allowing applications to connect to FreeSWITCH, execute commands, and receive events.
//!
//! # Architecture
//!
//! The library uses a split reader/writer design:
//! - [`EslClient`] (Clone + Send) — send commands from any task
//! - [`EslEventStream`] — receive events from a background reader task
//!
//! # Examples
//!
//! ## Inbound Connection
//!
//! ```rust,no_run
//! use freeswitch_esl_rs::{EslClient, EslError};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), EslError> {
//!     let (client, mut events) = EslClient::connect("localhost", 8021, "ClueCon").await?;
//!
//!     let response = client.api("status").await?;
//!     println!("Status: {}", response.body().unwrap_or(&"No body".to_string()));
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Event Subscription
//!
//! ```rust,no_run
//! use freeswitch_esl_rs::{EslClient, EslEventType, EventFormat};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let (client, mut events) = EslClient::connect("localhost", 8021, "ClueCon").await?;
//!
//!     client.subscribe_events(EventFormat::Plain, &[
//!         EslEventType::ChannelAnswer,
//!         EslEventType::ChannelHangup
//!     ]).await?;
//!
//!     while let Some(event) = events.recv().await {
//!         println!("Received event: {:?}", event.event_type());
//!     }
//!
//!     Ok(())
//! }
//! ```

pub mod app;
pub mod buffer;
pub mod command;
pub mod connection;
pub mod constants;
pub mod error;
pub mod event;
pub mod protocol;

pub use app::dptools::AppCommand;
pub use command::{CommandBuilder, EslResponse};
pub use connection::{
    ConnectionMode, ConnectionStatus, DisconnectReason, EslClient, EslEventStream,
};
pub use error::{EslError, EslResult};
pub use event::{EslEvent, EslEventType, EventFormat};
