//! Integration tests using mock ESL server: connect/authenticate, event
//! delivery, concurrent commands, client clone, header decoding, event queue
//! sizing, connect-refused, and connection mode.

mod mock_server;

use freeswitch_esl_tokio::{
    ConnectionMode, EslClient, EslConnectOptions, EslError, EslEventType, EventHeader,
    HeaderLookup, DEFAULT_ESL_PASSWORD,
};
use mock_server::{
    recv_event, setup_connected_pair, setup_connected_pair_with_options, setup_raw_pair,
    MockEslServer,
};
use std::collections::HashMap;
use std::time::Duration;

#[tokio::test]
async fn test_connect_and_authenticate() {
    let (_, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;
    assert!(client.is_connected());
}

#[tokio::test]
async fn test_auth_failure() {
    let server = MockEslServer::start("correct_password").await;
    let port = server.port();

    let (_, result) = tokio::join!(
        server.accept(),
        EslClient::connect("127.0.0.1", port, "wrong_password")
    );

    match result {
        Err(EslError::AuthenticationFailed { .. }) => {}
        Err(e) => panic!("Expected AuthenticationFailed, got: {}", e),
        Ok(_) => panic!("Expected error, got success"),
    }
}

#[tokio::test]
async fn test_recv_event_plain() {
    let (mut mock, client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    // Subscribe to events (mock just replies OK)
    let subscribe_task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .subscribe_events(
                    freeswitch_esl_tokio::EventFormat::Plain,
                    &[EslEventType::All],
                )
                .await
                .unwrap();
        }
    });

    // Mock reads the subscribe command and replies
    let _cmd = mock
        .read_command()
        .await;
    mock.reply_ok()
        .await;
    subscribe_task
        .await
        .unwrap();

    // Send an event from mock
    let mut headers = HashMap::new();
    headers.insert("Unique-ID".to_string(), "test-uuid-abc".to_string());
    headers.insert("Caller-Caller-ID-Number".to_string(), "1001".to_string());
    mock.send_event_plain("CHANNEL_CREATE", &headers)
        .await;

    let event = recv_event(&mut events).await;
    assert_eq!(event.event_type(), Some(EslEventType::ChannelCreate));
    assert_eq!(event.unique_id(), Some("test-uuid-abc"));
}

#[tokio::test]
async fn test_concurrent_command_and_events() {
    let (mut mock, client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    // Send an event from mock first (before any command)
    let mut headers = HashMap::new();
    headers.insert("Unique-ID".to_string(), "event-uuid".to_string());
    mock.send_event_plain("CHANNEL_CREATE", &headers)
        .await;

    // Now send an api command
    let api_task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .api("status")
                .await
                .unwrap()
        }
    });

    // Mock reads the api command and replies
    let cmd = mock
        .read_command()
        .await;
    assert!(cmd.starts_with("api status"));
    mock.reply_api("UP 0 years")
        .await;

    let response = api_task
        .await
        .unwrap();
    assert_eq!(response.body(), Some("UP 0 years"));

    // The event should still be available
    let event = recv_event(&mut events).await;
    assert_eq!(event.event_type(), Some(EslEventType::ChannelCreate));
}

// --- T3: Concurrent command test ---

#[tokio::test]
async fn test_concurrent_api_commands() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    // Launch two api() calls concurrently from different tasks
    let client1 = client.clone();
    let client2 = client.clone();
    let task1 = tokio::spawn(async move {
        client1
            .api("status")
            .await
    });
    let task2 = tokio::spawn(async move {
        client2
            .api("version")
            .await
    });

    // The writer mutex serializes them: read cmd1, reply, read cmd2, reply
    let cmd1 = mock
        .read_command()
        .await;
    assert!(cmd1.starts_with("api "), "first command: {}", cmd1);
    mock.reply_api("response-1")
        .await;

    let cmd2 = mock
        .read_command()
        .await;
    assert!(cmd2.starts_with("api "), "second command: {}", cmd2);
    mock.reply_api("response-2")
        .await;

    let result1 = task1
        .await
        .unwrap()
        .unwrap();
    let result2 = task2
        .await
        .unwrap()
        .unwrap();

    // Both should succeed with their respective responses
    let bodies: Vec<&str> = vec![
        result1
            .body()
            .unwrap(),
        result2
            .body()
            .unwrap(),
    ];
    assert!(bodies.contains(&"response-1"));
    assert!(bodies.contains(&"response-2"));
}

#[tokio::test]
async fn test_client_clone() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let client2 = client.clone();

    // Send command from clone
    let task = tokio::spawn(async move {
        client2
            .api("status")
            .await
    });

    let cmd = mock
        .read_command()
        .await;
    assert!(cmd.starts_with("api status"));
    mock.reply_api("OK")
        .await;

    let result = task
        .await
        .unwrap();
    assert!(result.is_ok());

    // Original client should also work
    let task2 = tokio::spawn(async move {
        client
            .api("version")
            .await
    });

    let cmd2 = mock
        .read_command()
        .await;
    assert!(cmd2.starts_with("api version"));
    mock.reply_api("1.0")
        .await;

    let result2 = task2
        .await
        .unwrap();
    assert!(result2.is_ok());
}

#[tokio::test]
async fn test_heartbeat_event_headers() {
    let (mut mock, _client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    mock.send_heartbeat()
        .await;

    let event = recv_event(&mut events).await;

    assert_eq!(event.event_type(), Some(EslEventType::Heartbeat));
    // Values should be percent-decoded
    assert_eq!(event.header_str("Event-Info"), Some("System Ready"));
    assert_eq!(
        event.header_str("Up-Time"),
        Some("0 years, 0 days, 1 hour, 23 minutes")
    );
    assert_eq!(event.header_str("Session-Count"), Some("5"));
    assert_eq!(event.header_str("Heartbeat-Interval"), Some("20"));
}

#[tokio::test]
async fn test_url_decoded_headers() {
    let (mut mock, _client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let mut headers = HashMap::new();
    headers.insert("Caller-Caller-ID-Name".to_string(), "John Doe".to_string());
    headers.insert(
        "variable_sip_from_display".to_string(),
        "Test User (123)".to_string(),
    );
    mock.send_event_plain("CHANNEL_CREATE", &headers)
        .await;

    let event = recv_event(&mut events).await;

    // Percent-encoded values should be decoded
    assert_eq!(
        event.header(EventHeader::CallerCallerIdName),
        Some("John Doe")
    );
    assert_eq!(
        event.header_str("variable_sip_from_display"),
        Some("Test User (123)")
    );
}

// --- T4: Event overflow/QueueFull notification test ---

#[tokio::test]
async fn test_event_overflow_queue_full() {
    let options = EslConnectOptions::new().with_event_queue_size(2);
    let (mut mock, client, mut events) =
        setup_connected_pair_with_options(DEFAULT_ESL_PASSWORD, options).await;

    // Fill the queue (capacity 2) then overflow it.
    for i in 0..5 {
        let mut headers = HashMap::new();
        headers.insert("Unique-ID".to_string(), format!("uuid-{}", i));
        mock.send_event_plain("CHANNEL_CREATE", &headers)
            .await;
    }

    // Let the reader loop process all events (queue fills, rest overflow).
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Drain the 2 buffered events to make room in the channel.
    for _ in 0..2 {
        let result = tokio::time::timeout(Duration::from_millis(500), events.recv()).await;
        assert!(matches!(result, Ok(Some(Ok(_)))));
    }

    // QueueFull is delivered piggy-backed on the next dispatch_event call.
    // Send one more event to trigger it.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut headers = HashMap::new();
    headers.insert("Unique-ID".to_string(), "uuid-trigger".to_string());
    mock.send_event_plain("CHANNEL_CREATE", &headers)
        .await;

    // Should get QueueFull followed by the trigger event.
    let mut got_queue_full = false;
    let mut event_count = 0;
    loop {
        match tokio::time::timeout(Duration::from_millis(500), events.recv()).await {
            Ok(Some(Ok(_event))) => event_count += 1,
            Ok(Some(Err(EslError::QueueFull))) => got_queue_full = true,
            Ok(Some(Err(e))) => panic!("unexpected error: {}", e),
            Ok(None) => break,
            Err(_) => break,
        }
    }

    assert!(
        got_queue_full,
        "expected QueueFull notification (got {} events)",
        event_count
    );
    assert!(
        client.dropped_event_count() > 0,
        "dropped_event_count should be > 0"
    );
}

// --- T6: Event queue size 0 clamped to 1 ---

#[tokio::test]
async fn test_event_queue_size_zero_clamped() {
    let options = EslConnectOptions::new().with_event_queue_size(0);
    let (mut mock, _client, mut events) =
        setup_connected_pair_with_options(DEFAULT_ESL_PASSWORD, options).await;

    // Should still work: size 0 is clamped to 1
    let mut headers = HashMap::new();
    headers.insert("Unique-ID".to_string(), "test-uuid".to_string());
    mock.send_event_plain("CHANNEL_CREATE", &headers)
        .await;

    let event = recv_event(&mut events).await;
    assert_eq!(event.event_type(), Some(EslEventType::ChannelCreate));
}

// --- TCP connection refused ---

#[tokio::test]
async fn connect_refused_returns_connection_error() {
    let (listener, port) = setup_raw_pair().await;
    drop(listener);

    let err = EslClient::connect("127.0.0.1", port, "pw")
        .await
        .unwrap_err();
    assert!(
        err.is_connection_error(),
        "connection refused should be a connection error, got: {err}"
    );
}

#[tokio::test]
async fn test_connection_mode_inbound() {
    let (_, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;
    assert_eq!(client.connection_mode(), ConnectionMode::Inbound);
}

#[tokio::test]
async fn test_connection_mode_outbound() {
    use tokio::net::TcpStream;

    let (listener, port) = setup_raw_pair().await;

    let (accept_result, _mock_stream) = tokio::join!(
        EslClient::accept_outbound(&listener),
        TcpStream::connect(("127.0.0.1", port))
    );

    let (client, _events) = accept_result.unwrap();
    assert_eq!(client.connection_mode(), ConnectionMode::Outbound);
}
