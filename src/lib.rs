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
//! use freeswitch_esl_tokio::{EslClient, EslError};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), EslError> {
//!     let (client, mut events) = EslClient::connect("localhost", 8021, "ClueCon").await?;
//!
//!     let response = client.api("status").await?;
//!     println!("Status: {}", response.body().unwrap_or("No body"));
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Event Subscription
//!
//! ```rust,no_run
//! use freeswitch_esl_tokio::{EslClient, EslEventType, EventFormat};
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
//!     while let Some(Ok(event)) = events.recv().await {
//!         println!("Received event: {:?}", event.event_type());
//!     }
//!
//!     Ok(())
//! }
//! ```

pub mod app;
pub mod commands;
pub mod connection;
pub mod error;
pub mod event;
pub mod variables;

pub(crate) mod buffer;
pub(crate) mod command;
pub(crate) mod constants;
pub(crate) mod protocol;

pub use app::dptools::AppCommand;
pub use command::{CommandBuilder, EslResponse, ReplyStatus};
pub use commands::{
    Application, ApplicationList, ConferenceDtmf, ConferenceHold, ConferenceMute, DialplanType,
    Endpoint, HoldAction, MuteAction, Originate, OriginateError, UuidAnswer, UuidBridge,
    UuidDeflect, UuidGetVar, UuidHold, UuidKill, UuidSendDtmf, UuidSetVar, UuidTransfer, Variables,
    VariablesType,
};
pub use connection::{
    ConnectionMode, ConnectionStatus, DisconnectReason, EslClient, EslConnectOptions,
    EslEventStream,
};
pub use constants::DEFAULT_ESL_PORT;
pub use error::{EslError, EslResult};
pub use event::{EslEvent, EslEventPriority, EslEventType, EventFormat};
pub use variables::{EslArray, MultipartBody, MultipartItem};
