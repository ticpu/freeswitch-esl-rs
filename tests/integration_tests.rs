//! Integration tests for FreeSWITCH ESL client
//!
//! These tests require a running FreeSWITCH instance for full functionality.
//! Unit tests that don't require FreeSWITCH are included in individual modules.

use freeswitch_esl_rs::{
    buffer::EslBuffer,
    command::{AppCommand, EslCommand},
    protocol::{EslParser, MessageType},
    EslError, EslEventType, EventFormat,
};

/// Test basic buffer operations
#[tokio::test]
async fn test_buffer_operations() {
    let mut buffer = EslBuffer::new();

    // Test empty buffer
    assert!(buffer.is_empty());
    assert_eq!(buffer.len(), 0);

    // Add data
    buffer.extend_from_slice(b"Hello World");
    assert!(!buffer.is_empty());
    assert_eq!(buffer.len(), 11);
    assert_eq!(buffer.data(), b"Hello World");

    // Test advance
    buffer.advance(6);
    assert_eq!(buffer.len(), 5);
    assert_eq!(buffer.data(), b"World");

    // Test pattern finding
    buffer.extend_from_slice(b"\r\n\r\nBody");
    let pos = buffer.find_pattern(b"\r\n\r\n");
    assert!(pos.is_some());

    // Extract until pattern
    let before_pattern = buffer.extract_until_pattern(b"\r\n\r\n").unwrap();
    assert_eq!(before_pattern, b"World");
    assert_eq!(buffer.data(), b"Body");
}

/// Test protocol message parsing
#[tokio::test]
async fn test_protocol_parsing() {
    let mut parser = EslParser::new();

    // Test auth request parsing
    let auth_data = b"Content-Type: auth/request\r\n\r\n";
    parser.add_data(auth_data).unwrap();

    let message = parser.parse_message().unwrap().unwrap();
    assert_eq!(message.message_type, MessageType::AuthRequest);
    assert!(message.body.is_none());
    assert_eq!(
        message.header("Content-Type"),
        Some(&"auth/request".to_string())
    );

    // Test API response with body
    let api_data =
        b"Content-Type: api/response\r\nReply-Text: +OK accepted\r\nContent-Length: 2\r\n\r\nOK";
    parser.add_data(api_data).unwrap();

    let message = parser.parse_message().unwrap().unwrap();
    assert_eq!(message.message_type, MessageType::ApiResponse);
    assert_eq!(message.body, Some("OK".to_string()));
    assert!(message.is_success());
}

/// Test event parsing
#[tokio::test]
async fn test_event_parsing() {
    let mut parser = EslParser::new();

    let event_data = b"Content-Type: text/event-plain\r\n\
                      Event-Name: CHANNEL_ANSWER\r\n\
                      Unique-ID: test-uuid-123\r\n\
                      Caller-Caller-ID-Number: 1000\r\n\
                      Caller-Destination-Number: 2000\r\n\r\n";

    parser.add_data(event_data).unwrap();
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

/// Test command generation
#[tokio::test]
async fn test_command_generation() {
    // Test auth command
    let auth = EslCommand::Auth {
        password: "ClueCon".to_string(),
    };
    assert_eq!(auth.to_wire_format(), "auth ClueCon\r\n\r\n");

    // Test API command
    let api = EslCommand::Api {
        command: "status".to_string(),
    };
    assert_eq!(api.to_wire_format(), "api status\r\n\r\n");

    // Test events command
    let events = EslCommand::Events {
        format: "plain".to_string(),
        events: "ALL".to_string(),
    };
    assert_eq!(events.to_wire_format(), "event plain ALL\r\n\r\n");

    // Test app commands
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

/// Test event type parsing
#[tokio::test]
async fn test_event_types() {
    // Test event type string conversion
    assert_eq!(EslEventType::ChannelAnswer.to_string(), "CHANNEL_ANSWER");
    assert_eq!(EslEventType::ChannelCreate.to_string(), "CHANNEL_CREATE");
    assert_eq!(EslEventType::Heartbeat.to_string(), "HEARTBEAT");

    // Test parsing from string
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

/// Test error handling
#[tokio::test]
async fn test_error_handling() {
    // Test error types
    let io_error = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "Connection refused");
    let esl_error = EslError::from(io_error);
    assert!(esl_error.is_connection_error());

    let timeout_error = EslError::Timeout { timeout_ms: 5000 };
    assert!(timeout_error.is_recoverable());

    let protocol_error = EslError::protocol_error("Invalid message format");
    assert!(!protocol_error.is_recoverable());

    let auth_error = EslError::auth_failed("Invalid password");
    assert!(!auth_error.is_connection_error());
}

/// Test JSON event parsing
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

/// Test incomplete message handling
#[tokio::test]
async fn test_incomplete_messages() {
    let mut parser = EslParser::new();

    // Add partial header
    parser.add_data(b"Content-Type: api/response\r\n").unwrap();
    let result = parser.parse_message().unwrap();
    assert!(result.is_none());

    // Complete the headers
    parser.add_data(b"Content-Length: 12\r\n\r\n").unwrap();
    let result = parser.parse_message().unwrap();
    assert!(result.is_none()); // Still no body

    // Add partial body
    parser.add_data(b"partial").unwrap();
    let result = parser.parse_message().unwrap();
    assert!(result.is_none()); // Body not complete

    // Complete the body
    parser.add_data(b"_body").unwrap();
    let result = parser.parse_message().unwrap();
    assert!(result.is_some()); // Now we have a complete message

    let message = result.unwrap();
    assert_eq!(message.body, Some("partial_body".to_string()));
}

/// Test large message handling
#[tokio::test]
async fn test_large_message_handling() {
    let mut parser = EslParser::new();

    // Create a large message
    let large_body = "x".repeat(1024 * 1024); // 1MB body
    let header = format!(
        "Content-Type: api/response\r\nContent-Length: {}\r\n\r\n",
        large_body.len()
    );

    parser.add_data(header.as_bytes()).unwrap();
    parser.add_data(large_body.as_bytes()).unwrap();

    let result = parser.parse_message().unwrap();
    assert!(result.is_some());

    let message = result.unwrap();
    assert_eq!(message.body.as_ref().unwrap().len(), 1024 * 1024);
}

/// Mock connection test (doesn't require FreeSWITCH)
#[tokio::test]
async fn test_connection_states() {
    // Test connection mode comparison
    use freeswitch_esl_rs::connection::ConnectionMode;
    assert_eq!(ConnectionMode::Inbound, ConnectionMode::Inbound);
    assert_ne!(ConnectionMode::Inbound, ConnectionMode::Outbound);

    // Test event format display
    assert_eq!(EventFormat::Plain.to_string(), "plain");
    assert_eq!(EventFormat::Json.to_string(), "json");
    assert_eq!(EventFormat::Xml.to_string(), "xml");
}

/// Performance test for message parsing
#[tokio::test]
async fn test_parsing_performance() {
    let mut parser = EslParser::new();
    let test_message = b"Content-Type: text/event-plain\r\n\
                        Event-Name: CHANNEL_CREATE\r\n\
                        Unique-ID: perf-test-uuid\r\n\
                        Test-Header-1: Value1\r\n\
                        Test-Header-2: Value2\r\n\
                        Test-Header-3: Value3\r\n\r\n";

    let start = std::time::Instant::now();

    // Parse many messages
    for _ in 0..1000 {
        parser.add_data(test_message).unwrap();
        let message = parser.parse_message().unwrap().unwrap();
        assert_eq!(message.message_type, MessageType::Event);
    }

    let duration = start.elapsed();
    println!(
        "Parsed 1000 messages in {:?} ({:.2} msg/sec)",
        duration,
        1000.0 / duration.as_secs_f64()
    );

    // Should be reasonably fast
    assert!(
        duration.as_millis() < 1000,
        "Parsing too slow: {:?}",
        duration
    );
}

/// Test buffer management under stress
#[tokio::test]
async fn test_buffer_stress() {
    let mut buffer = EslBuffer::new();

    // Add lots of data in chunks
    for i in 0..1000 {
        let data = format!("chunk-{}-{}\r\n", i, "x".repeat(100));
        buffer.extend_from_slice(data.as_bytes());
    }

    // Consume data progressively
    let mut consumed = 0;
    while !buffer.is_empty() {
        let chunk_size = std::cmp::min(1024, buffer.len());
        buffer.advance(chunk_size);
        consumed += chunk_size;

        // Compact occasionally
        if consumed % 10240 == 0 {
            buffer.compact();
        }
    }

    assert!(buffer.is_empty());
    println!("Successfully processed {} bytes", consumed);
}
