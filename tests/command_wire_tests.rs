//! Integration tests using mock ESL server: wire-format verification for
//! individual commands (sendevent, myevents, linger, resume, nixevent,
//! noevents, filter delete, divert_events, getvar), outbound connect_session,
//! repeating SIP header round trips, and bgapi correlation.

mod mock_server;

use freeswitch_esl_tokio::{
    EslClient, EslEvent, EslEventType, EventFormat, HeaderLookup, DEFAULT_ESL_PASSWORD,
};
use mock_server::{recv_event, setup_connected_pair, MockClient};
use std::collections::HashMap;

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

/// `nixevent` shares the `event` grammar, so a `CUSTOM` ahead of another type
/// would delete a subclass by that name instead of unsubscribing the type.
#[tokio::test]
async fn test_nixevent_custom_orders_last() {
    let (mut mock, client, _events) = setup_connected_pair(DEFAULT_ESL_PASSWORD).await;

    let task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .nixevent(&[EslEventType::Custom, EslEventType::ChannelCreate])
                .await
        }
    });

    let cmd = mock
        .read_command()
        .await;
    assert_eq!(cmd, "nixevent CHANNEL_CREATE CUSTOM\n\n");
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
