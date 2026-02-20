//! Integration tests for FreeSWITCH ESL client
//!
//! These tests use only the public API. Tests for internal modules (buffer,
//! protocol, command) live as unit tests inside the respective modules.

use freeswitch_esl_tokio::{ConnectionMode, EslError, EslEventType, EventFormat};

#[tokio::test]
async fn test_event_types() {
    assert_eq!(EslEventType::ChannelAnswer.to_string(), "CHANNEL_ANSWER");
    assert_eq!(EslEventType::ChannelCreate.to_string(), "CHANNEL_CREATE");
    assert_eq!(EslEventType::Heartbeat.to_string(), "HEARTBEAT");

    assert_eq!(
        EslEventType::parse_event_type("CHANNEL_ANSWER"),
        Some(EslEventType::ChannelAnswer)
    );
    assert_eq!(
        EslEventType::parse_event_type("channel_answer"),
        Some(EslEventType::ChannelAnswer)
    );
    assert_eq!(
        EslEventType::parse_event_type("DTMF"),
        Some(EslEventType::Dtmf)
    );
    assert_eq!(EslEventType::parse_event_type("UNKNOWN_EVENT"), None);
}

#[tokio::test]
async fn test_error_handling() {
    let io_error = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "Connection refused");
    let esl_error = EslError::from(io_error);
    assert!(esl_error.is_connection_error());

    let timeout_error = EslError::Timeout { timeout_ms: 5000 };
    assert!(timeout_error.is_recoverable());

    let protocol_error = EslError::protocol_error("Invalid message format");
    assert!(!protocol_error.is_recoverable());

    let auth_error = EslError::auth_failed("Invalid password");
    assert!(!auth_error.is_connection_error());

    let heartbeat_error = EslError::HeartbeatExpired { interval_ms: 60000 };
    assert!(heartbeat_error.is_connection_error());
    assert!(!heartbeat_error.is_recoverable());
}

#[tokio::test]
async fn test_connection_error_detection() {
    let closed_error = EslError::ConnectionClosed;
    assert!(closed_error.is_connection_error());
    assert!(!closed_error.is_recoverable());

    let not_connected_error = EslError::NotConnected;
    assert!(not_connected_error.is_connection_error());
    assert!(!not_connected_error.is_recoverable());

    for error_kind in [
        std::io::ErrorKind::ConnectionReset,
        std::io::ErrorKind::ConnectionAborted,
        std::io::ErrorKind::BrokenPipe,
        std::io::ErrorKind::UnexpectedEof,
    ] {
        let io_error = std::io::Error::new(error_kind, "test error");
        let esl_error = EslError::from(io_error);
        assert!(
            esl_error.is_connection_error(),
            "{:?} should be a connection error",
            error_kind
        );
    }
}

#[tokio::test]
async fn test_connection_states() {
    assert_eq!(ConnectionMode::Inbound, ConnectionMode::Inbound);
    assert_ne!(ConnectionMode::Inbound, ConnectionMode::Outbound);

    assert_eq!(EventFormat::Plain.to_string(), "plain");
    assert_eq!(EventFormat::Json.to_string(), "json");
    assert_eq!(EventFormat::Xml.to_string(), "xml");
}
