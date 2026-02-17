//! Integration tests using mock ESL server

mod mock_server;

use freeswitch_esl_rs::{ConnectionStatus, DisconnectReason, EslClient, EslError, EslEventType};
use mock_server::{setup_connected_pair, MockEslServer};
use std::collections::HashMap;
use std::time::Duration;

#[tokio::test]
async fn test_connect_and_authenticate() {
    let (_, client, _events) = setup_connected_pair("ClueCon").await;
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
    let (mut mock, client, mut events) = setup_connected_pair("ClueCon").await;

    // Subscribe to events (mock just replies OK)
    let subscribe_task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .subscribe_events(freeswitch_esl_rs::EventFormat::Plain, &[EslEventType::All])
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

    // Receive event
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("timeout waiting for event")
        .expect("event stream closed");

    assert_eq!(event.event_type(), Some(EslEventType::ChannelCreate));
    assert_eq!(event.unique_id(), Some(&"test-uuid-abc".to_string()));
}

#[tokio::test]
async fn test_concurrent_command_and_events() {
    let (mut mock, client, mut events) = setup_connected_pair("ClueCon").await;

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
    assert_eq!(response.body(), Some(&"UP 0 years".to_string()));

    // The event should still be available
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("timeout")
        .expect("closed");
    assert_eq!(event.event_type(), Some(EslEventType::ChannelCreate));
}

#[tokio::test]
async fn test_disconnect_notice() {
    let (mut mock, _client, mut events) = setup_connected_pair("ClueCon").await;

    mock.send_disconnect_notice("Disconnected, goodbye.\nSee you later.\n")
        .await;

    // events.recv() should return None after disconnect
    let result = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("timeout");
    assert!(result.is_none());

    assert!(!_client.is_connected());
    match _client.status() {
        ConnectionStatus::Disconnected(DisconnectReason::ServerNotice) => {}
        other => panic!("Expected ServerNotice, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_tcp_disconnect() {
    let (mock, _client, mut events) = setup_connected_pair("ClueCon").await;

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
    let (mock, client, mut events) = setup_connected_pair("ClueCon").await;

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
async fn test_liveness_expired() {
    let (_mock, client, mut events) = setup_connected_pair("ClueCon").await;

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
    let (mut mock, client, mut events) = setup_connected_pair("ClueCon").await;

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
    while let Some(event) = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("timeout")
    {
        if event.event_type() == Some(EslEventType::Heartbeat) {
            count += 1;
            if count >= 3 {
                break;
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
    let (_mock, client, mut events) = setup_connected_pair("ClueCon").await;

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
    let (mut mock, client, _events) = setup_connected_pair("ClueCon").await;

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
    let (mut mock, _client, mut events) = setup_connected_pair("ClueCon").await;

    mock.send_heartbeat()
        .await;

    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("timeout")
        .expect("closed");

    assert_eq!(event.event_type(), Some(EslEventType::Heartbeat));
    // Values should be percent-decoded
    assert_eq!(
        event.header("Event-Info"),
        Some(&"System Ready".to_string())
    );
    assert_eq!(
        event.header("Up-Time"),
        Some(&"0 years, 0 days, 1 hour, 23 minutes".to_string())
    );
    assert_eq!(event.header("Session-Count"), Some(&"5".to_string()));
    assert_eq!(event.header("Heartbeat-Interval"), Some(&"20".to_string()));
}

#[tokio::test]
async fn test_url_decoded_headers() {
    let (mut mock, _client, mut events) = setup_connected_pair("ClueCon").await;

    let mut headers = HashMap::new();
    headers.insert("Caller-Caller-ID-Name".to_string(), "John Doe".to_string());
    headers.insert(
        "variable_sip_from_display".to_string(),
        "Test User (123)".to_string(),
    );
    mock.send_event_plain("CHANNEL_CREATE", &headers)
        .await;

    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("timeout")
        .expect("closed");

    // Percent-encoded values should be decoded
    assert_eq!(
        event.header("Caller-Caller-ID-Name"),
        Some(&"John Doe".to_string())
    );
    assert_eq!(
        event.header("variable_sip_from_display"),
        Some(&"Test User (123)".to_string())
    );
}

#[tokio::test]
async fn test_command_timeout() {
    let (_mock, client, _events) = setup_connected_pair("ClueCon").await;

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
    let (_mock, client, _events) = setup_connected_pair("ClueCon").await;

    // Default timeout should be 5 seconds — verify a command still works
    // by having the mock reply within that window
    // (This test just verifies the default doesn't break normal flow)
    let (mut mock, client2, _events2) = setup_connected_pair("ClueCon").await;

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
    let (mut mock, client, _events) = setup_connected_pair("ClueCon").await;

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
