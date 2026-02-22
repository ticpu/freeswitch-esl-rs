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
//! ## Outbound Mode
//!
//! In outbound mode, FreeSWITCH connects to *your* application via the
//! [`socket`](https://developer.signalwire.com/freeswitch/FreeSWITCH-Explained/Modules/mod_event_socket_1048924/)
//! dialplan application. You run a TCP listener and accept connections:
//!
//! ```rust,no_run
//! use freeswitch_esl_tokio::{EslClient, EslError, AppCommand, EventFormat};
//! use tokio::net::TcpListener;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), EslError> {
//!     let listener = TcpListener::bind("0.0.0.0:8040").await
//!         .map_err(EslError::from)?;
//!
//!     let (client, mut events) = EslClient::accept_outbound(&listener).await?;
//!
//!     // First command must be connect_session — establishes the session
//!     // and returns all channel variables as headers.
//!     let channel_data = client.connect_session().await?;
//!     println!("Channel: {}", channel_data.header("Channel-Name").unwrap_or("?"));
//!
//!     client.myevents(EventFormat::Plain).await?;
//!     client.linger(None).await?; // keep socket open after hangup
//!     client.resume().await?;     // resume dialplan on disconnect
//!
//!     client.send_command(AppCommand::answer()).await?;
//!     client.send_command(AppCommand::playback("ivr/ivr-welcome.wav")).await?;
//!
//!     while let Some(Ok(event)) = events.recv().await {
//!         println!("{:?}", event.event_type());
//!     }
//!     Ok(())
//! }
//! ```
//!
//! Configure FreeSWITCH to connect to your app:
//! ```xml
//! <action application="socket" data="127.0.0.1:8040 async full"/>
//! ```
//!
//! See [`docs/outbound-esl-quirks.md`](https://github.com/ticpu/freeswitch-esl-tokio/blob/master/docs/outbound-esl-quirks.md)
//! for protocol details and command availability by mode.
//!
//! ## Command Builders
//!
//! Typed builders for common API commands — no raw string assembly needed:
//!
//! ```rust
//! use freeswitch_esl_tokio::{Originate, Endpoint, ApplicationList, Application};
//!
//! let cmd = Originate {
//!     endpoint: Endpoint::SofiaGateway {
//!         uri: "18005551234".into(),
//!         profile: None,
//!         gateway: "my_provider".into(),
//!         variables: None,
//!     },
//!     applications: ApplicationList(vec![
//!         Application::new("park", None::<&str>),
//!     ]),
//!     dialplan: None,
//!     context: None,
//!     cid_name: Some("Outbound Call".into()),
//!     cid_num: Some("5551234".into()),
//!     timeout: Some(30),
//! };
//!
//! // Use with client.api(&cmd.to_string()) or client.bgapi(&cmd.to_string())
//! assert!(cmd.to_string().contains("sofia/gateway/my_provider/18005551234"));
//! ```
//!
//! See the [`commands`] module for `Originate`, `UuidBridge`, `UuidTransfer`,
//! and other builders.
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
pub mod channel;
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
pub use channel::ChannelTimetable;
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
pub use event::{
    AnswerState, CallDirection, CallState, ChannelState, EslEvent, EslEventPriority, EslEventType,
    EventFormat,
};
pub use variables::{EslArray, MultipartBody, MultipartItem};
