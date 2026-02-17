//! Integration tests for FreeSWITCH ESL client
//!
//! These tests use a mock server for full functionality testing.
//! Unit tests that don't require FreeSWITCH are included in individual modules.

use freeswitch_esl_rs::{
    buffer::EslBuffer,
    command::{AppCommand, EslCommand},
    protocol::{EslParser, MessageType},
    EslError, EslEventType, EventFormat,
};

#[tokio::test]
async fn test_buffer_operations() {
    let mut buffer = EslBuffer::new();

    assert!(buffer.is_empty());
    assert_eq!(buffer.len(), 0);

    buffer.extend_from_slice(b"Hello World");
    assert!(!buffer.is_empty());
    assert_eq!(buffer.len(), 11);
    assert_eq!(buffer.data(), b"Hello World");

    buffer.advance(6);
    assert_eq!(buffer.len(), 5);
    assert_eq!(buffer.data(), b"World");

    buffer.extend_from_slice(b"\n\nBody");
    let pos = buffer.find_pattern(b"\n\n");
    assert!(pos.is_some());

    let before_pattern = buffer.extract_until_pattern(b"\n\n").unwrap();
    assert_eq!(before_pattern, b"World");
    assert_eq!(buffer.data(), b"Body");
}

#[tokio::test]
async fn test_protocol_parsing() {
    let mut parser = EslParser::new();

    let auth_data = b"Content-Type: auth/request\n\n";
    parser.add_data(auth_data).unwrap();

    let message = parser.parse_message().unwrap().unwrap();
    assert_eq!(message.message_type, MessageType::AuthRequest);
    assert!(message.body.is_none());
    assert_eq!(
        message.header("Content-Type"),
        Some(&"auth/request".to_string())
    );

    let api_data = b"Content-Type: api/response\nReply-Text: +OK accepted\nContent-Length: 2\n\nOK";
    parser.add_data(api_data).unwrap();

    let message = parser.parse_message().unwrap().unwrap();
    assert_eq!(message.message_type, MessageType::ApiResponse);
    assert_eq!(message.body, Some("OK".to_string()));
    assert!(message.is_success());
}

#[tokio::test]
async fn test_event_parsing() {
    let mut parser = EslParser::new();

    // Correct two-part wire format
    let body = "Event-Name: CHANNEL_ANSWER\n\
                Unique-ID: test-uuid-123\n\
                Caller-Caller-ID-Number: 1000\n\
                Caller-Destination-Number: 2000\n\n";
    let envelope = format!(
        "Content-Length: {}\nContent-Type: text/event-plain\n\n",
        body.len()
    );
    let data = format!("{}{}", envelope, body);

    parser.add_data(data.as_bytes()).unwrap();
    let message = parser.parse_message().unwrap().unwrap();
    let event = parser.parse_event(message, EventFormat::Plain).unwrap();

    assert_eq!(event.event_type, Some(EslEventType::ChannelAnswer));
    assert_eq!(event.unique_id(), Some(&"test-uuid-123".to_string()));
    assert_eq!(
        event.header("Caller-Caller-ID-Number"),
        Some(&"1000".to_string())
    );
    assert_eq!(
        event.header("Caller-Destination-Number"),
        Some(&"2000".to_string())
    );
}

#[tokio::test]
async fn test_command_generation() {
    let auth = EslCommand::Auth {
        password: "ClueCon".to_string(),
    };
    assert_eq!(auth.to_wire_format(), "auth ClueCon\n\n");

    let api = EslCommand::Api {
        command: "status".to_string(),
    };
    assert_eq!(api.to_wire_format(), "api status\n\n");

    let events = EslCommand::Events {
        format: "plain".to_string(),
        events: "ALL".to_string(),
    };
    assert_eq!(events.to_wire_format(), "event plain ALL\n\n");

    let answer = AppCommand::answer();
    let wire_format = answer.to_wire_format();
    assert!(wire_format.contains("call-command: execute"));
    assert!(wire_format.contains("execute-app-name: answer"));

    let hangup = AppCommand::hangup(Some("NORMAL_CLEARING"));
    let wire_format = hangup.to_wire_format();
    assert!(wire_format.contains("execute-app-name: hangup"));
    assert!(wire_format.contains("execute-app-arg: NORMAL_CLEARING"));

    let playback = AppCommand::playback("test.wav");
    let wire_format = playback.to_wire_format();
    assert!(wire_format.contains("execute-app-name: playback"));
    assert!(wire_format.contains("execute-app-arg: test.wav"));
}

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
async fn test_disconnect_notice_parsing() {
    let mut parser = EslParser::new();
    let disconnect_data = b"Content-Type: text/disconnect-notice\n\n";

    parser.add_data(disconnect_data).unwrap();
    let message = parser.parse_message().unwrap().unwrap();

    assert_eq!(message.message_type, MessageType::Disconnect);
}

#[tokio::test]
async fn test_json_event_parsing() {
    let json_body = r#"{
        "Event-Name": "CHANNEL_ANSWER",
        "Unique-ID": "json-test-uuid",
        "Caller-Caller-ID-Number": "1000",
        "Answer-State": "answered"
    }"#;

    let mut headers = std::collections::HashMap::new();
    headers.insert("Content-Type".to_string(), "text/event-json".to_string());

    let message = freeswitch_esl_rs::protocol::EslMessage::new(
        MessageType::Event,
        headers,
        Some(json_body.to_string()),
    );

    let parser = EslParser::new();
    let event = parser.parse_event(message, EventFormat::Json).unwrap();

    assert_eq!(event.event_type, Some(EslEventType::ChannelAnswer));
    assert_eq!(
        event.header("Unique-ID"),
        Some(&"json-test-uuid".to_string())
    );
    assert_eq!(
        event.header("Caller-Caller-ID-Number"),
        Some(&"1000".to_string())
    );
}

#[tokio::test]
async fn test_incomplete_messages() {
    let mut parser = EslParser::new();

    parser.add_data(b"Content-Type: api/response\n").unwrap();
    let result = parser.parse_message().unwrap();
    assert!(result.is_none());

    parser.add_data(b"Content-Length: 12\n\n").unwrap();
    let result = parser.parse_message().unwrap();
    assert!(result.is_none());

    parser.add_data(b"partial").unwrap();
    let result = parser.parse_message().unwrap();
    assert!(result.is_none());

    parser.add_data(b"_body").unwrap();
    let result = parser.parse_message().unwrap();
    assert!(result.is_some());

    let message = result.unwrap();
    assert_eq!(message.body, Some("partial_body".to_string()));
}

#[tokio::test]
async fn test_large_message_handling() {
    let mut parser = EslParser::new();

    let large_body = "x".repeat(1024 * 1024);
    let header = format!(
        "Content-Type: api/response\nContent-Length: {}\n\n",
        large_body.len()
    );

    parser.add_data(header.as_bytes()).unwrap();
    parser.add_data(large_body.as_bytes()).unwrap();

    let result = parser.parse_message().unwrap();
    assert!(result.is_some());

    let message = result.unwrap();
    assert_eq!(message.body.as_ref().unwrap().len(), 1024 * 1024);
}

#[tokio::test]
async fn test_connection_states() {
    use freeswitch_esl_rs::connection::ConnectionMode;
    assert_eq!(ConnectionMode::Inbound, ConnectionMode::Inbound);
    assert_ne!(ConnectionMode::Inbound, ConnectionMode::Outbound);

    assert_eq!(EventFormat::Plain.to_string(), "plain");
    assert_eq!(EventFormat::Json.to_string(), "json");
    assert_eq!(EventFormat::Xml.to_string(), "xml");
}

#[tokio::test]
async fn test_parsing_performance() {
    let mut parser = EslParser::new();

    // Correct two-part wire format for performance test
    let body = "Event-Name: CHANNEL_CREATE\n\
                Unique-ID: perf-test-uuid\n\
                Test-Header-1: Value1\n\
                Test-Header-2: Value2\n\
                Test-Header-3: Value3\n\n";
    let envelope = format!(
        "Content-Length: {}\nContent-Type: text/event-plain\n\n",
        body.len()
    );
    let test_message = format!("{}{}", envelope, body);

    let start = std::time::Instant::now();

    for _ in 0..1000 {
        parser.add_data(test_message.as_bytes()).unwrap();
        let message = parser.parse_message().unwrap().unwrap();
        assert_eq!(message.message_type, MessageType::Event);
    }

    let duration = start.elapsed();
    assert!(
        duration.as_millis() < 1000,
        "Parsing too slow: {:?}",
        duration
    );
}

#[tokio::test]
async fn test_buffer_stress() {
    let mut buffer = EslBuffer::new();

    for i in 0..1000 {
        let data = format!("chunk-{}-{}\n", i, "x".repeat(100));
        buffer.extend_from_slice(data.as_bytes());
    }

    let mut consumed = 0;
    while !buffer.is_empty() {
        let chunk_size = std::cmp::min(1024, buffer.len());
        buffer.advance(chunk_size);
        consumed += chunk_size;

        if consumed % 10240 == 0 {
            buffer.compact();
        }
    }

    assert!(buffer.is_empty());
}
