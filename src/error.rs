//! Error types for FreeSWITCH ESL operations.
//!
//! All fallible operations in this crate return [`EslResult<T>`].  Errors are
//! classified into two axes for caller convenience:
//!
//! - **Connection errors** ([`EslError::is_connection_error`]) — the TCP session
//!   is dead and the caller should reconnect.
//! - **Recoverable errors** ([`EslError::is_recoverable`]) — the command failed
//!   but the connection is still usable (e.g., timeout, command rejected).

use crate::commands::OriginateError;
use thiserror::Error;

/// Result type alias for ESL operations
pub type EslResult<T> = Result<T, EslError>;

/// Comprehensive error types for ESL operations
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum EslError {
    /// IO error from underlying TCP operations
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Connection is not established or lost
    #[error("Not connected to FreeSWITCH")]
    NotConnected,

    /// Authentication failed
    #[error("Authentication failed: {reason}")]
    AuthenticationFailed { reason: String },

    /// Protocol error - invalid message format
    #[error("Protocol error: {message}")]
    ProtocolError { message: String },

    /// Command returned `-ERR` with an error message from FreeSWITCH
    #[error("Command failed: {reply_text}")]
    CommandFailed { reply_text: String },

    /// Reply-Text did not match the expected `+OK`/`-ERR` protocol format.
    ///
    /// Most ESL commands return `+OK ...` on success and `-ERR ...` on failure.
    /// A reply that matches neither indicates a protocol-level anomaly or a
    /// command with non-standard reply format (e.g. `getvar`).
    #[error("Unexpected reply: {reply_text}")]
    UnexpectedReply { reply_text: String },

    /// Timeout waiting for response
    #[error("Operation timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },

    /// Invalid event format
    #[error("Invalid event format: {format}")]
    InvalidEventFormat { format: String },

    /// JSON parsing error
    #[error("JSON parsing error: {0}")]
    JsonError(#[from] serde_json::Error),

    /// XML parsing error  
    #[error("XML parsing error: {0}")]
    XmlError(#[from] quick_xml::Error),

    /// UTF-8 conversion error
    #[error("UTF-8 conversion error: {0}")]
    Utf8Error(#[from] std::str::Utf8Error),

    /// Buffer overflow - message too large
    #[error("Buffer overflow: message size {size} exceeds limit {limit}")]
    BufferOverflow { size: usize, limit: usize },

    /// Invalid header format
    #[error("Invalid header format: {header}")]
    InvalidHeader { header: String },

    /// Missing required header
    #[error("Missing required header: {header}")]
    MissingHeader { header: String },

    /// Connection closed by remote
    #[error("Connection closed by FreeSWITCH")]
    ConnectionClosed,

    /// Heartbeat/liveness timeout expired
    #[error("Heartbeat expired after {interval_ms}ms without traffic")]
    HeartbeatExpired { interval_ms: u64 },

    /// Invalid UUID format
    #[error("Invalid UUID format: {uuid}")]
    InvalidUuid { uuid: String },

    /// Event queue full
    #[error("Event queue is full - dropping events")]
    QueueFull,

    /// Generic error with custom message
    #[error("ESL error: {message}")]
    Generic { message: String },

    /// Originate command builder error
    #[error("Originate error: {0}")]
    Originate(#[from] OriginateError),
}

impl EslError {
    pub fn generic(message: impl Into<String>) -> Self {
        Self::Generic {
            message: message.into(),
        }
    }

    pub fn protocol_error(message: impl Into<String>) -> Self {
        Self::ProtocolError {
            message: message.into(),
        }
    }

    pub fn auth_failed(reason: impl Into<String>) -> Self {
        Self::AuthenticationFailed {
            reason: reason.into(),
        }
    }

    /// `true` if the connection is still usable and the caller can retry.
    ///
    /// Recoverable: `Timeout`, `CommandFailed`, `UnexpectedReply`, `QueueFull`.
    /// Non-recoverable errors (I/O, auth, disconnect) mean the connection is dead
    /// and the caller should reconnect.
    pub fn is_recoverable(&self) -> bool {
        match self {
            EslError::Io(_) => false,
            EslError::NotConnected => false,
            EslError::ConnectionClosed => false,
            EslError::AuthenticationFailed { .. } => false,
            EslError::HeartbeatExpired { .. } => false,
            EslError::Timeout { .. } => true,
            EslError::CommandFailed { .. } => true,
            EslError::UnexpectedReply { .. } => true,
            EslError::QueueFull => true,
            _ => false,
        }
    }

    /// `true` if the TCP session is dead and the caller should reconnect.
    ///
    /// Matches: `Io`, `NotConnected`, `ConnectionClosed`, `HeartbeatExpired`.
    pub fn is_connection_error(&self) -> bool {
        matches!(
            self,
            EslError::Io(_)
                | EslError::NotConnected
                | EslError::ConnectionClosed
                | EslError::HeartbeatExpired { .. }
        )
    }
}
