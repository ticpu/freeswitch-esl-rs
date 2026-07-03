//! Integration tests using mock ESL server

mod mock_server;

use freeswitch_esl_tokio::{
    ConnectionStatus, DisconnectReason, EslClient, EslConnectOptions, EslError, EslEvent,
    EslEventStream, EslEventType, EventFormat, EventHeader, HeaderLookup, DEFAULT_ESL_PASSWORD,
};
use mock_server::{
    setup_connected_pair, setup_connected_pair_with_options, MockClient, MockEslServer,
};
use std::collections::HashMap;
use std::time::Duration;

async fn recv_event(events: &mut EslEventStream) -> EslEvent {
    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("timeout")
        .expect("channel closed")
        .expect("event error")
}

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

#[tokio::test]
async fn test_disconnect_notice() {
    let (mut mock, _client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    mock.send_disconnect_notice("Disconnected, goodbye.\nSee you later.\n")
        .await;

    // events.recv() should return None after disconnect
    let result = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("timeout");
    assert!(result.is_none());

    assert!(!_client.is_connected());
    match _client.status() {
        ConnectionStatus::Disconnected(DisconnectReason::ServerNotice { .. }) => {}
        other => panic!("Expected ServerNotice, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_tcp_disconnect() {
    let (mock, _client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    // Drop the mock's TCP connection
    mock.drop_connection()
        .await;

    // events.recv() should return None
    let result = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("timeout");
    assert!(result.is_none());

    assert!(!_client.is_connected());
    match _client.status() {
        ConnectionStatus::Disconnected(DisconnectReason::ConnectionClosed) => {}
        other => panic!("Expected ConnectionClosed, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_command_after_disconnect() {
    let (mock, client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    mock.drop_connection()
        .await;

    // Wait for the reader to detect the disconnect
    let _ = tokio::time::timeout(Duration::from_secs(5), events.recv()).await;

    // Commands should fail with NotConnected
    let result = client
        .api("status")
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        EslError::NotConnected => {}
        e => panic!("Expected NotConnected, got: {}", e),
    }
}

#[tokio::test]
async fn test_subscribe_permission_denied_recoverable() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    // Restricted user: FreeSWITCH rejects `event plain HEARTBEAT` with
    // -ERR permission denied. The subscribe call must surface that as a
    // recoverable error and leave the connection usable.
    let subscribe_task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .subscribe_events(EventFormat::Plain, &[EslEventType::Heartbeat])
                .await
        }
    });

    let _cmd = mock
        .read_command()
        .await;
    mock.reply_err("permission denied")
        .await;

    let err = subscribe_task
        .await
        .unwrap()
        .expect_err("subscribe should fail");
    assert!(err.is_permission_denied(), "got: {err}");
    assert!(err.is_recoverable());
    assert!(!err.is_connection_error());

    // Connection stays up: a follow-up command on the same socket still works.
    assert!(client.is_connected());
    let api_task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .api("status")
                .await
        }
    });
    let cmd = mock
        .read_command()
        .await;
    assert!(cmd.starts_with("api status"));
    mock.reply_api("UP 0 years")
        .await;
    let resp = api_task
        .await
        .unwrap()
        .expect("api should succeed");
    assert_eq!(resp.body(), Some("UP 0 years"));
}

#[tokio::test]
async fn test_liveness_expired() {
    let (_mock, client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    // Set a very short liveness timeout
    client.set_liveness_timeout(Duration::from_secs(1));

    // Don't send any traffic from mock — liveness should expire
    // The reader loop checks every 2s, so we need to wait a bit
    let result = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("timeout waiting for heartbeat expiry");
    assert!(result.is_none());

    assert!(!client.is_connected());
    match client.status() {
        ConnectionStatus::Disconnected(DisconnectReason::HeartbeatExpired) => {}
        other => panic!("Expected HeartbeatExpired, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_liveness_reset_by_traffic() {
    let (mut mock, client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    // Set liveness timeout to 3s
    client.set_liveness_timeout(Duration::from_secs(3));

    // Send events every 2s to keep connection alive
    let mock_task = tokio::spawn(async move {
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_secs(2)).await;
            mock.send_heartbeat()
                .await;
        }
        // After sending 3 heartbeats, stop — liveness should expire
        mock
    });

    // Receive the 3 heartbeats
    let mut count = 0;
    while let Some(result) = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("timeout")
    {
        if let Ok(event) = result {
            if event.event_type() == Some(EslEventType::Heartbeat) {
                count += 1;
                if count >= 3 {
                    break;
                }
            }
        }
    }
    assert_eq!(count, 3);

    // Now wait for liveness expiry (no more traffic)
    let _mock = mock_task
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("timeout");
    assert!(result.is_none());

    match client.status() {
        ConnectionStatus::Disconnected(DisconnectReason::HeartbeatExpired) => {}
        other => panic!("Expected HeartbeatExpired, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_stall_detected() {
    let (_mock, client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    // Set short timeout — auth traffic already happened, then nothing
    client.set_liveness_timeout(Duration::from_secs(1));

    let result = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("timeout");
    assert!(result.is_none());

    match client.status() {
        ConnectionStatus::Disconnected(DisconnectReason::HeartbeatExpired) => {}
        other => panic!("Expected HeartbeatExpired, got: {:?}", other),
    }
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

#[tokio::test]
async fn test_command_timeout() {
    let (_mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    // Set a very short command timeout
    client.set_command_timeout(Duration::from_millis(200));

    // Send a command but mock never replies — should timeout
    let result = client
        .api("status")
        .await;

    match result {
        Err(EslError::Timeout { .. }) => {}
        Err(e) => panic!("Expected Timeout, got: {}", e),
        Ok(_) => panic!("Expected timeout error, got success"),
    }
}

#[tokio::test]
async fn test_command_timeout_default() {
    let (_mock, _client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    // Default timeout should be 5 seconds — verify a command still works
    // by having the mock reply within that window
    // (This test just verifies the default doesn't break normal flow)
    let (mut mock, client2, _events2) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let api_task = tokio::spawn(async move {
        client2
            .api("status")
            .await
    });

    let _cmd = mock
        .read_command()
        .await;
    mock.reply_api("OK")
        .await;

    let result = api_task
        .await
        .unwrap();
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_command_timeout_cleanup() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    // Set short timeout
    client.set_command_timeout(Duration::from_millis(200));

    // First command times out (mock doesn't reply)
    let result = client
        .api("status")
        .await;
    assert!(matches!(result, Err(EslError::Timeout { .. })));

    // Second command should still work — pending_reply slot was cleaned up
    let api_task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .api("version")
                .await
        }
    });

    // Mock reads the timed-out command then the new one
    let _cmd1 = mock
        .read_command()
        .await;
    let _cmd2 = mock
        .read_command()
        .await;
    mock.reply_api("1.0")
        .await;

    let result = api_task
        .await
        .unwrap();
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_sendevent_command() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let mut event = EslEvent::with_type(EslEventType::Custom);
    event.set_header("Event-Name".to_string(), "CUSTOM".to_string());
    event.set_header("Event-Subclass".to_string(), "test::my_event".to_string());

    let send_task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .sendevent(event)
                .await
        }
    });

    let cmd = mock
        .read_command()
        .await;
    assert!(cmd.starts_with("sendevent CUSTOM\n"));
    assert!(cmd.contains("Event-Subclass: test::my_event\n"));
    mock.reply_ok()
        .await;

    let response = send_task
        .await
        .unwrap()
        .unwrap();
    assert!(response.is_success());
}

#[tokio::test]
async fn test_myevents_command() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .myevents(EventFormat::Plain)
                .await
        }
    });

    let cmd = mock
        .read_command()
        .await;
    assert_eq!(cmd, "myevents plain\n\n");
    mock.reply_ok()
        .await;

    task.await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn test_myevents_uuid_command() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .myevents_uuid("abc-123-def", EventFormat::Json)
                .await
        }
    });

    let cmd = mock
        .read_command()
        .await;
    assert_eq!(cmd, "myevents abc-123-def json\n\n");
    mock.reply_ok()
        .await;

    task.await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn test_linger_command() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .linger(None)
                .await
        }
    });

    let cmd = mock
        .read_command()
        .await;
    assert_eq!(cmd, "linger\n\n");
    mock.reply_ok()
        .await;

    task.await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn test_linger_timeout_command() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .linger(Some(std::time::Duration::from_secs(300)))
                .await
        }
    });

    let cmd = mock
        .read_command()
        .await;
    assert_eq!(cmd, "linger 300\n\n");
    mock.reply_ok()
        .await;

    task.await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn test_nolinger_command() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .nolinger()
                .await
        }
    });

    let cmd = mock
        .read_command()
        .await;
    assert_eq!(cmd, "nolinger\n\n");
    mock.reply_ok()
        .await;

    task.await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn test_resume_command() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .resume()
                .await
        }
    });

    let cmd = mock
        .read_command()
        .await;
    assert_eq!(cmd, "resume\n\n");
    mock.reply_ok()
        .await;

    task.await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn test_nixevent_command() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .nixevent(&[EslEventType::ChannelCreate, EslEventType::ChannelDestroy])
                .await
        }
    });

    let cmd = mock
        .read_command()
        .await;
    assert_eq!(cmd, "nixevent CHANNEL_CREATE CHANNEL_DESTROY\n\n");
    mock.reply_ok()
        .await;

    task.await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn test_noevents_command() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .noevents()
                .await
        }
    });

    let cmd = mock
        .read_command()
        .await;
    assert_eq!(cmd, "noevents\n\n");
    mock.reply_ok()
        .await;

    task.await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn test_filter_delete_command() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .filter_delete_raw("Event-Name", Some("CHANNEL_CREATE"))
                .await
        }
    });

    let cmd = mock
        .read_command()
        .await;
    assert_eq!(cmd, "filter delete Event-Name CHANNEL_CREATE\n\n");
    mock.reply_ok()
        .await;

    task.await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn test_divert_events_command() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .divert_events(true)
                .await
        }
    });

    let cmd = mock
        .read_command()
        .await;
    assert_eq!(cmd, "divert_events on\n\n");
    mock.reply_ok()
        .await;

    task.await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn test_getvar_command() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .getvar("caller_id_name")
                .await
        }
    });

    let cmd = mock
        .read_command()
        .await;
    assert_eq!(cmd, "getvar caller_id_name\n\n");
    mock.reply_raw_text("John Doe")
        .await;

    let value = task
        .await
        .unwrap()
        .unwrap();
    assert_eq!(value, "John Doe");
}

#[tokio::test]
async fn test_outbound_connect_session() {
    use tokio::net::{TcpListener, TcpStream};

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap();
    let port = listener
        .local_addr()
        .unwrap()
        .port();

    // Mock FreeSWITCH connects to our listener, then we send connect
    let (accept_result, mock_stream) = tokio::join!(
        EslClient::accept_outbound(&listener),
        TcpStream::connect(("127.0.0.1", port))
    );

    let (client, _events) = accept_result.unwrap();
    let mut mock = MockClient::from_stream(mock_stream.unwrap());

    // Client sends connect, mock replies with serialized channel data
    let connect_task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .connect_session()
                .await
        }
    });

    let cmd = mock
        .read_command()
        .await;
    assert_eq!(cmd, "connect\n\n");

    let mut channel_headers = HashMap::new();
    channel_headers.insert("Event-Name".to_string(), "CHANNEL_DATA".to_string());
    channel_headers.insert(
        "Channel-Name".to_string(),
        "sofia/internal/1000@example.com".to_string(),
    );
    channel_headers.insert("Unique-ID".to_string(), "abcd-1234-efgh".to_string());
    channel_headers.insert("Caller-Caller-ID-Name".to_string(), "Test User".to_string());
    channel_headers.insert("Caller-Caller-ID-Number".to_string(), "1000".to_string());
    mock.send_connect_response(&channel_headers)
        .await;

    let response = connect_task
        .await
        .unwrap()
        .unwrap();

    assert!(response.is_success());
    assert_eq!(response.reply_text(), Some("+OK"));
    assert_eq!(
        response.header("Channel-Name"),
        Some("sofia/internal/1000@example.com")
    );
    assert_eq!(response.header("Unique-ID"), Some("abcd-1234-efgh"));
    assert_eq!(response.header("Caller-Caller-ID-Name"), Some("Test User"));
    assert_eq!(response.header("Caller-Caller-ID-Number"), Some("1000"));
    assert_eq!(response.header("Socket-Mode"), Some("async"));
    assert_eq!(response.header("Control"), Some("full"));
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

// ── Re-exec tests ───────────────────────────────────────────────────────

#[cfg(unix)]
mod reexec {
    use super::*;

    #[tokio::test]
    async fn teardown_returns_valid_fd_and_residual() {
        let (_mock, client, events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

        let result = client
            .teardown_for_reexec()
            .await;
        assert!(result.is_ok(), "teardown failed: {:?}", result.err());

        let (fd, residual) = result.unwrap();
        assert!(fd >= 0, "fd should be non-negative");
        // No data was in-flight, so residual should be empty
        assert!(
            residual.is_empty(),
            "expected empty residual, got {} bytes",
            residual.len()
        );

        // Connection should be marked as disconnected
        assert!(!client.is_connected());
        assert_eq!(
            client.status(),
            ConnectionStatus::Disconnected(DisconnectReason::ReexecTeardown)
        );
        // Event stream must also see ReexecTeardown — this is what downstream
        // consumers check when the event channel closes.
        assert_eq!(
            events.status(),
            ConnectionStatus::Disconnected(DisconnectReason::ReexecTeardown)
        );
    }

    #[tokio::test]
    async fn teardown_with_buffered_event_delivers_then_stops() {
        let (mut mock, client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

        // Send an event then immediately teardown.
        // The event should be delivered before teardown completes.
        let mut headers = HashMap::new();
        headers.insert("Unique-ID".to_string(), "reexec-test-uuid".to_string());
        mock.send_event_plain("HEARTBEAT", &headers)
            .await;

        // Give the reader a moment to buffer the event
        tokio::time::sleep(Duration::from_millis(50)).await;

        let (fd, _residual) = client
            .teardown_for_reexec()
            .await
            .unwrap();
        assert!(fd >= 0);

        // The event should have been dispatched before the drain completed
        let event = tokio::time::timeout(Duration::from_secs(1), events.recv()).await;
        assert!(
            event.is_ok(),
            "event should have been dispatched before drain"
        );
    }

    #[tokio::test]
    async fn teardown_twice_fails() {
        let (_mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

        let first = client
            .teardown_for_reexec()
            .await;
        assert!(first.is_ok());

        let second = client
            .teardown_for_reexec()
            .await;
        assert!(second.is_err());
        match second.unwrap_err() {
            EslError::ReexecFailed { reason } => {
                assert!(
                    reason.contains("already called"),
                    "unexpected reason: {}",
                    reason
                );
            }
            e => panic!("expected ReexecFailed, got: {}", e),
        }
    }

    #[tokio::test]
    async fn teardown_with_pending_command_fails() {
        let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

        // Start a command but don't reply from mock
        let client2 = client.clone();
        let cmd_handle = tokio::spawn(async move {
            client2
                .api("status")
                .await
        });

        // Wait for the command to be in-flight
        tokio::time::sleep(Duration::from_millis(50)).await;

        let result = client
            .teardown_for_reexec()
            .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            EslError::ReexecFailed { reason } => {
                assert!(
                    reason.contains("in-flight"),
                    "unexpected reason: {}",
                    reason
                );
            }
            e => panic!("expected ReexecFailed, got: {}", e),
        }

        // Clean up: reply to the pending command so the task completes
        let _cmd = mock
            .read_command()
            .await;
        mock.reply_api("OK")
            .await;
        let _ = cmd_handle.await;
    }

    #[tokio::test]
    async fn adopt_stream_with_empty_residual() {
        // Set up a mock that acts as an already-authenticated server
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let port = listener
            .local_addr()
            .unwrap()
            .port();

        let server_handle = tokio::spawn(async move {
            let (stream, _) = listener
                .accept()
                .await
                .unwrap();
            MockClient::from_stream(stream)
        });

        let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();

        let mut mock = server_handle
            .await
            .unwrap();

        // adopt_stream bypasses auth
        let (client, mut events) = EslClient::adopt_stream(stream, &[]).unwrap();
        assert!(client.is_connected());

        // Send an event from mock, verify it arrives
        let mut headers = HashMap::new();
        headers.insert("Unique-ID".to_string(), "adopted-uuid".to_string());
        mock.send_event_plain("CHANNEL_CREATE", &headers)
            .await;

        let event = recv_event(&mut events).await;
        assert_eq!(event.event_type(), Some(EslEventType::ChannelCreate));
    }

    #[tokio::test]
    async fn adopt_stream_with_residual_bytes() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let port = listener
            .local_addr()
            .unwrap()
            .port();

        let server_handle = tokio::spawn(async move {
            let (stream, _) = listener
                .accept()
                .await
                .unwrap();
            MockClient::from_stream(stream)
        });

        let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();

        let _mock = server_handle
            .await
            .unwrap();

        // Pre-seed with a complete event in the residual buffer
        let event_body = "Event-Name: HEARTBEAT\nCore-UUID: test-core\n\n";
        let envelope = format!(
            "Content-Length: {}\nContent-Type: text/event-plain\n\n{}",
            event_body.len(),
            event_body
        );

        let (_client, mut events) = EslClient::adopt_stream(stream, envelope.as_bytes()).unwrap();

        // The pre-seeded event should be delivered from the residual buffer
        let event = recv_event(&mut events).await;
        assert_eq!(event.event_type(), Some(EslEventType::Heartbeat));
    }

    #[tokio::test]
    async fn adopt_stream_with_options() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let port = listener
            .local_addr()
            .unwrap()
            .port();

        let server_handle = tokio::spawn(async move {
            let (stream, _) = listener
                .accept()
                .await
                .unwrap();
            MockClient::from_stream(stream)
        });

        let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();

        let _mock = server_handle
            .await
            .unwrap();

        let options = EslConnectOptions::new().with_event_queue_size(10);
        let (client, _events) = EslClient::adopt_stream_with_options(stream, &[], options).unwrap();
        assert!(client.is_connected());
    }

    #[tokio::test]
    async fn sequential_teardown_adopt_teardown() {
        use std::os::unix::io::FromRawFd;

        let (_mock, client, events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

        // First teardown
        let (fd, residual) = client
            .teardown_for_reexec()
            .await
            .expect("first teardown failed");
        assert_eq!(
            events.status(),
            ConnectionStatus::Disconnected(DisconnectReason::ReexecTeardown)
        );

        // Prevent EslClient from closing the fd on drop (as documented).
        // In production the process calls exec(), destroying the old runtime.
        std::mem::forget(client);

        // dup() the fd so the new TcpStream gets a fresh reactor registration
        // (the original fd is still registered by the forgotten OwnedWriteHalf).
        let new_fd = unsafe { libc::dup(fd) };
        assert!(
            new_fd >= 0,
            "dup() failed: {}",
            std::io::Error::last_os_error()
        );
        // Close the original fd that was leaked by mem::forget
        unsafe { libc::close(fd) };

        let std_stream = unsafe { std::net::TcpStream::from_raw_fd(new_fd) };
        std_stream
            .set_nonblocking(true)
            .expect("set_nonblocking failed");
        let stream = tokio::net::TcpStream::from_std(std_stream).expect("from_std failed");
        let (client2, events2) = EslClient::adopt_stream(stream, &residual).unwrap();
        assert!(client2.is_connected());

        // Second teardown
        let (_fd2, _residual2) = client2
            .teardown_for_reexec()
            .await
            .expect("second teardown failed");
        assert_eq!(
            events2.status(),
            ConnectionStatus::Disconnected(DisconnectReason::ReexecTeardown)
        );
    }
}

// --- bgapi correlation with mock ---

#[tokio::test]
async fn bgapi_returns_job_uuid_from_reply() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let api_task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .bgapi("status")
                .await
        }
    });

    let cmd = mock
        .read_command()
        .await;
    assert!(cmd.starts_with("bgapi status"));

    mock.send_raw(
        "Content-Type: command/reply\nReply-Text: +OK Job-UUID: d8efc742-test-uuid\nJob-UUID: d8efc742-test-uuid\n\n",
    )
    .await;

    let response = api_task
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.job_uuid(), Some("d8efc742-test-uuid"));
}

#[tokio::test]
async fn bgapi_background_job_event_delivered() {
    let (mut mock, client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;
    let job_uuid = "aabb1122-bgapi-test";

    let api_task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .bgapi("status")
                .await
        }
    });

    let _cmd = mock
        .read_command()
        .await;
    mock.send_raw(&format!(
        "Content-Type: command/reply\nReply-Text: +OK Job-UUID: {job_uuid}\nJob-UUID: {job_uuid}\n\n"
    ))
    .await;

    let response = api_task
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.job_uuid(), Some(job_uuid));

    let mut headers = HashMap::new();
    headers.insert("Job-UUID".to_string(), job_uuid.to_string());
    mock.send_event_plain_with_body("BACKGROUND_JOB", &headers, "+OK status output\n")
        .await;

    let event = recv_event(&mut events).await;
    assert_eq!(event.event_type(), Some(EslEventType::BackgroundJob));
    assert_eq!(event.job_uuid(), Some(job_uuid));
    assert_eq!(event.body(), Some("+OK status output\n"));
}

#[tokio::test]
async fn bgapi_multiple_jobs_correlated() {
    let (mut mock, client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let uuids = ["job-uuid-001", "job-uuid-002", "job-uuid-003"];
    let bodies = ["+OK status\n", "+OK version\n", "+OK hostname\n"];

    for uuid in &uuids {
        let api_task = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .bgapi("status")
                    .await
            }
        });
        let _cmd = mock
            .read_command()
            .await;
        mock.send_raw(&format!(
            "Content-Type: command/reply\nReply-Text: +OK Job-UUID: {uuid}\nJob-UUID: {uuid}\n\n"
        ))
        .await;
        let resp = api_task
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resp.job_uuid(), Some(*uuid));
    }

    // Send events in reverse order
    for i in (0..3).rev() {
        let mut headers = HashMap::new();
        headers.insert("Job-UUID".to_string(), uuids[i].to_string());
        mock.send_event_plain_with_body("BACKGROUND_JOB", &headers, bodies[i])
            .await;
    }

    let mut matched = std::collections::HashSet::new();
    for _ in 0..3 {
        let event = recv_event(&mut events).await;
        assert_eq!(event.event_type(), Some(EslEventType::BackgroundJob));
        let job_uuid = event
            .job_uuid()
            .expect("BACKGROUND_JOB should have Job-UUID")
            .to_string();
        let idx = uuids
            .iter()
            .position(|u| *u == job_uuid)
            .expect("Job-UUID should match one of the sent commands");
        assert_eq!(event.body(), Some(bodies[idx]));
        matched.insert(job_uuid);
    }
    assert_eq!(matched.len(), 3);
}

// --- Repeating SIP header wire format tests ---

#[tokio::test]
async fn test_sip_comma_separated_header_wire_round_trip() {
    let (mut mock, _client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    // Simulate a CHANNEL_CREATE with comma-separated P-Asserted-Identity,
    // as FreeSWITCH would send it from a SIP INVITE with two identities
    let mut headers = HashMap::new();
    headers.insert("Unique-ID".to_string(), "pai-test-uuid".to_string());
    headers.insert(
        "variable_sip_P-Asserted-Identity".to_string(),
        "<sip:alice@atlanta.example.com>, <tel:+15551234567>".to_string(),
    );
    mock.send_event_plain("CHANNEL_CREATE", &headers)
        .await;

    let event = recv_event(&mut events).await;
    assert_eq!(event.event_type(), Some(EslEventType::ChannelCreate));
    // Comma-separated value should survive percent-encoding round-trip intact
    assert_eq!(
        event.variable_str("sip_P-Asserted-Identity"),
        Some("<sip:alice@atlanta.example.com>, <tel:+15551234567>")
    );
}

#[tokio::test]
async fn test_sip_array_header_wire_round_trip() {
    use freeswitch_types::EslArray;

    let (mut mock, _client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    // Simulate an event with ARRAY-formatted repeating SIP header
    let mut headers = HashMap::new();
    headers.insert("Unique-ID".to_string(), "pai-array-uuid".to_string());
    headers.insert(
        "variable_sip_P-Asserted-Identity".to_string(),
        "ARRAY::<sip:alice@atlanta.example.com>|:<tel:+15551234567>".to_string(),
    );
    mock.send_event_plain("CHANNEL_CREATE", &headers)
        .await;

    let event = recv_event(&mut events).await;
    assert_eq!(event.event_type(), Some(EslEventType::ChannelCreate));

    let raw = event
        .variable_str("sip_P-Asserted-Identity")
        .expect("P-Asserted-Identity variable should be present");
    let arr = EslArray::parse(raw).expect("should parse as ARRAY");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr.items()[0], "<sip:alice@atlanta.example.com>");
    assert_eq!(arr.items()[1], "<tel:+15551234567>");
}

#[tokio::test]
async fn test_sip_diversion_repeated_header_wire_round_trip() {
    use freeswitch_types::EslArray;

    let (mut mock, _client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    // SIP Diversion header (RFC 5806) can repeat with history entries
    let mut headers = HashMap::new();
    headers.insert("Unique-ID".to_string(), "diversion-uuid".to_string());
    headers.insert(
        "variable_sip_h_Diversion".to_string(),
        "ARRAY::<sip:+15551234567@gw.example.com;reason=unconditional>|:<sip:+15559876543@proxy.example.com;reason=no-answer;counter=3>".to_string(),
    );
    mock.send_event_plain("CHANNEL_CREATE", &headers)
        .await;

    let event = recv_event(&mut events).await;
    let raw = event
        .variable_str("sip_h_Diversion")
        .expect("Diversion variable should be present");
    let arr = EslArray::parse(raw).expect("should parse as ARRAY");
    assert_eq!(arr.len(), 2);
    assert_eq!(
        arr.items()[0],
        "<sip:+15551234567@gw.example.com;reason=unconditional>"
    );
    assert_eq!(
        arr.items()[1],
        "<sip:+15559876543@proxy.example.com;reason=no-answer;counter=3>"
    );
}

// --- Rude rejection ---

#[tokio::test]
async fn rude_rejection_returns_access_denied() {
    let (mut mock, client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    mock.send_raw("Content-Type: text/rude-rejection\n\n")
        .await;

    let result = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("timeout");
    match result {
        Some(Err(EslError::AccessDenied { .. })) => {}
        other => panic!("expected AccessDenied error, got: {:?}", other),
    }

    let closed = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("timeout");
    assert!(
        closed.is_none(),
        "stream should be closed after rude rejection"
    );

    match client.status() {
        ConnectionStatus::Disconnected(DisconnectReason::AccessDenied(_)) => {}
        other => panic!("expected Disconnected(AccessDenied), got: {:?}", other),
    }
}

#[tokio::test]
async fn parser_error_reports_protocol_error() {
    let (mut mock, client, mut events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    // An unrecognized Content-Type triggers a ProtocolError from parse_message().
    mock.send_raw("Content-Type: text/garbage\n\n")
        .await;

    // Event stream should close (no valid event emitted for a protocol error).
    let closed = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("timeout");
    assert!(
        closed.is_none(),
        "stream should close on parser error, got: {:?}",
        closed
    );

    match client.status() {
        ConnectionStatus::Disconnected(DisconnectReason::ProtocolError(_)) => {}
        other => panic!("expected Disconnected(ProtocolError), got: {:?}", other),
    }
}

// --- TCP connection refused ---

#[tokio::test]
async fn connect_refused_returns_connection_error() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap();
    let port = listener
        .local_addr()
        .unwrap()
        .port();
    drop(listener);

    let err = EslClient::connect("127.0.0.1", port, "pw")
        .await
        .unwrap_err();
    assert!(
        err.is_connection_error(),
        "connection refused should be a connection error, got: {err}"
    );
}
