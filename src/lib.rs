//! FreeSWITCH Event Socket Library (ESL) client for Rust
//!
//! This crate provides an async Rust client for FreeSWITCH's Event Socket Library (ESL),
//! allowing applications to connect to FreeSWITCH, execute commands, and receive events.
//!
//! # Examples
//!
//! ## Inbound Connection (Client connects to FreeSWITCH)
//!
//! ```rust,no_run
//! use freeswitch_esl_rs::{EslHandle, EslError};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), EslError> {
//!     let mut handle = EslHandle::connect("localhost", 8021, "ClueCon").await?;
//!     
//!     let response = handle.api("status").await?;
//!     println!("Status: {}", response.body().unwrap_or(&"No body".to_string()));
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## Event Subscription
//!
//! ```rust,no_run
//! use freeswitch_esl_rs::{EslHandle, EslEventType, EventFormat};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut handle = EslHandle::connect("localhost", 8021, "ClueCon").await?;
//!     
//!     handle.subscribe_events(EventFormat::Plain, &[
//!         EslEventType::ChannelAnswer,
//!         EslEventType::ChannelHangup
//!     ]).await?;
//!     
//!     while let Some(event) = handle.recv_event().await? {
//!         println!("Received event: {:?}", event.event_type());
//!     }
//!     
//!     Ok(())
//! }
//! ```

pub mod buffer;
pub mod command;
pub mod connection;
pub mod constants;
pub mod error;
pub mod event;
pub mod protocol;

pub use command::{CommandBuilder, EslResponse};
pub use connection::{ConnectionMode, ConnectionStatus, DisconnectReason, EslHandle};
pub use error::{EslError, EslResult};
pub use event::{EslEvent, EslEventType, EventFormat};
