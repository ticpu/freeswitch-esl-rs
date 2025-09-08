//! Error types for FreeSWITCH ESL operations

use thiserror::Error;

/// Result type alias for ESL operations
pub type EslResult<T> = Result<T, EslError>;

/// Comprehensive error types for ESL operations
#[derive(Error, Debug)]
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

    /// Command execution failed
    #[error("Command failed: {reply_text}")]
    CommandFailed { reply_text: String },

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

    /// Invalid UUID format
    #[error("Invalid UUID format: {uuid}")]
    InvalidUuid { uuid: String },

    /// Event queue full
    #[error("Event queue is full - dropping events")]
    QueueFull,

    /// Generic error with custom message
    #[error("ESL error: {message}")]
    Generic { message: String },
}

impl EslError {
    /// Create a generic error with a custom message
    pub fn generic(message: impl Into<String>) -> Self {
        Self::Generic {
            message: message.into(),
        }
    }

    /// Create a protocol error
    pub fn protocol_error(message: impl Into<String>) -> Self {
        Self::ProtocolError {
            message: message.into(),
        }
    }

    /// Create an authentication error
    pub fn auth_failed(reason: impl Into<String>) -> Self {
        Self::AuthenticationFailed {
            reason: reason.into(),
        }
    }

    /// Check if this is a recoverable error
    pub fn is_recoverable(&self) -> bool {
        match self {
            EslError::Io(_) => false,
            EslError::NotConnected => false,
            EslError::ConnectionClosed => false,
            EslError::AuthenticationFailed { .. } => false,
            EslError::Timeout { .. } => true,
            EslError::CommandFailed { .. } => true,
            EslError::QueueFull => true,
            _ => false,
        }
    }

    /// Check if this error indicates a connection problem
    pub fn is_connection_error(&self) -> bool {
        matches!(
            self,
            EslError::Io(_) | EslError::NotConnected | EslError::ConnectionClosed
        )
    }
}
