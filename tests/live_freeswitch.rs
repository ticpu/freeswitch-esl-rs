//! Integration tests against a live FreeSWITCH instance.
//!
//! These tests require FreeSWITCH ESL on 127.0.0.1:8022 with password ClueCon.
//! Run with: cargo test --test live_freeswitch -- --ignored

use freeswitch_esl_tokio::commands::originate::{Variables, VariablesType};
use freeswitch_esl_tokio::commands::{LoopbackEndpoint, UuidGetVar, UuidKill, UuidSetVar};
use freeswitch_esl_tokio::{
    parse_api_body, Application, ConnectionStatus, DialplanType, DisconnectReason, Endpoint,
    EslClient, EslConnectOptions, EslError, EslEvent, EslEventPriority, EslEventType, EventFormat,
    EventHeader, HeaderLookup, Originate, ReplyStatus, DEFAULT_ESL_PASSWORD,
};
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::time::Instant;

const ESL_HOST: &str = "127.0.0.1";
const ESL_PORT: u16 = 8022;
const ESL_PASSWORD: &str = DEFAULT_ESL_PASSWORD;
const MAX_CONCURRENT_CONNECTIONS: usize = 5;

static CONN_SEMAPHORE: Semaphore = Semaphore::const_new(MAX_CONCURRENT_CONNECTIONS);

async fn connect() -> (
    EslClient,
    freeswitch_esl_tokio::EslEventStream,
    tokio::sync::SemaphorePermit<'static>,
) {
    let permit = CONN_SEMAPHORE
        .acquire()
        .await
        .expect("semaphore closed");
    let opts = EslConnectOptions::new().with_connect_timeout(Duration::from_secs(30));
    let (client, events) = EslClient::connect_with_options(ESL_HOST, ESL_PORT, ESL_PASSWORD, opts)
        .await
        .expect("failed to connect to FreeSWITCH");
    client.set_command_timeout(Duration::from_secs(10));
    (client, events, permit)
}

#[tokio::test]
#[ignore]
async fn live_connect_and_status() {
    let (client, _events, _permit) = connect().await;
    assert!(client.is_connected());

    let resp = client
        .api("status")
        .await
        .unwrap();
    let body = resp
        .body()
        .expect("status should have body");
    assert!(body.contains("UP"), "expected UP in status: {}", body);
}

#[tokio::test]
#[ignore]
async fn live_subscribe_and_recv_heartbeat() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(EventFormat::Plain, &[EslEventType::Heartbeat])
        .await
        .unwrap();

    let event = tokio::time::timeout(Duration::from_secs(25), events.recv())
        .await
        .expect("timeout waiting for heartbeat")
        .expect("channel closed")
        .expect("event error");

    assert_eq!(event.event_type(), Some(EslEventType::Heartbeat));
    assert!(event
        .header(EventHeader::CoreUuid)
        .is_some());
}

#[tokio::test]
#[ignore]
async fn live_sendevent_with_priority() {
    let (client, _events, _permit) = connect().await;

    let mut event = EslEvent::with_type(EslEventType::Custom);
    event.set_header("Event-Name", "CUSTOM");
    event.set_header("Event-Subclass", "esl_test::priority");
    event.set_priority(EslEventPriority::High);

    let resp = client
        .sendevent(event)
        .await
        .unwrap();
    assert!(
        resp.is_success(),
        "sendevent failed: {:?}",
        resp.reply_text()
    );
}

#[tokio::test]
#[ignore]
async fn live_sendevent_with_array_header() {
    let (client, _events, _permit) = connect().await;

    let mut event = EslEvent::with_type(EslEventType::Custom);
    event.set_header("Event-Name", "CUSTOM");
    event.set_header("Event-Subclass", "esl_test::array");
    event
        .push_header("X-Test-Array", "value1")
        .unwrap();
    event
        .push_header("X-Test-Array", "value2")
        .unwrap();
    event
        .push_header("X-Test-Array", "value3")
        .unwrap();

    assert_eq!(
        event.header_str("X-Test-Array"),
        Some("ARRAY::value1|:value2|:value3")
    );

    let resp = client
        .sendevent(event)
        .await
        .unwrap();
    assert!(
        resp.is_success(),
        "sendevent failed: {:?}",
        resp.reply_text()
    );
}

#[tokio::test]
#[ignore]
async fn live_recv_custom_sendevent() {
    let (client, mut events, _permit) = connect().await;

    let subclass = format!("esl_test::live_{}", std::process::id());

    client
        .subscribe_events_raw(EventFormat::Plain, &format!("CUSTOM {}", subclass))
        .await
        .unwrap();
    let mut event = EslEvent::with_type(EslEventType::Custom);
    event.set_header("Event-Name", "CUSTOM");
    event.set_header("Event-Subclass", subclass.clone());
    event.set_priority(EslEventPriority::Normal);
    event
        .push_header("X-Test-Data", "hello")
        .unwrap();
    event
        .push_header("X-Test-Data", "world")
        .unwrap();

    client
        .sendevent(event)
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.header(EventHeader::EventSubclass) == Some(subclass.as_str()) {
                    assert_eq!(evt.header(EventHeader::Priority), Some("NORMAL"));
                    assert_eq!(evt.header_str("X-Test-Data"), Some("ARRAY::hello|:world"),);
                    return;
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("event stream closed"),
            Err(_) => break,
        }
    }
    panic!("did not receive custom event with subclass {}", subclass);
}

/// End-to-end percent-decode against real FreeSWITCH: a header value with
/// characters FS percent-encodes (space, `@`, and a multibyte UTF-8 char)
/// must arrive decoded. Events delivered to ESL go through
/// `switch_event_serialize(SWITCH_TRUE)`, so the value is `%20`/`%40`/`%C3%A9`
/// on the wire and the parser must reconstruct the original. The lossy path
/// (genuinely non-UTF-8 bytes) can't be exercised here because a Rust `&str`
/// is always valid UTF-8 -- it stays unit-tested.
#[tokio::test]
#[ignore]
async fn live_recv_custom_sendevent_percent_decoded() {
    let (client, mut events, _permit) = connect().await;

    let subclass = format!("esl_test::decode_{}", std::process::id());
    let value = "Jean Dupont@héllo";

    client
        .subscribe_events_raw(EventFormat::Plain, &format!("CUSTOM {}", subclass))
        .await
        .unwrap();
    let mut event = EslEvent::with_type(EslEventType::Custom);
    event.set_header("Event-Name", "CUSTOM");
    event.set_header("Event-Subclass", subclass.clone());
    event.set_priority(EslEventPriority::Normal);
    event.set_header("X-Decode-Test", value);

    client
        .sendevent(event)
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.header(EventHeader::EventSubclass) == Some(subclass.as_str()) {
                    assert_eq!(
                        evt.header_str("X-Decode-Test"),
                        Some(value),
                        "FS percent-encodes the value on the wire; the parser must decode it"
                    );
                    assert!(
                        evt.lossy_values()
                            .is_empty(),
                        "valid UTF-8 is not lossy"
                    );
                    return;
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("event stream closed"),
            Err(_) => break,
        }
    }
    panic!("did not receive custom event with subclass {}", subclass);
}

#[tokio::test]
#[ignore]
async fn live_api_multiple_commands() {
    let (client, _events, _permit) = connect().await;

    let version = client
        .api("version")
        .await
        .unwrap();
    assert!(
        version
            .body()
            .is_some(),
        "version should have body"
    );

    let hostname = client
        .api("hostname")
        .await
        .unwrap();
    assert!(
        hostname
            .body()
            .is_some(),
        "hostname should have body"
    );

    let global = client
        .api("global_getvar")
        .await
        .unwrap();
    assert!(
        global
            .body()
            .is_some(),
        "global_getvar should have body"
    );
}

#[tokio::test]
#[ignore]
async fn live_reply_status_ok() {
    let (client, _events, _permit) = connect().await;

    // subscribe_events uses send_command_ok → into_result(), so Ok means +OK
    client
        .subscribe_events(EventFormat::Plain, &[EslEventType::Heartbeat])
        .await
        .expect("subscribe should return +OK");
}

#[tokio::test]
#[ignore]
async fn live_reply_status_err() {
    let (client, _events, _permit) = connect().await;

    // log with an invalid level triggers -ERR from FreeSWITCH.
    // log() returns the raw EslResponse (not through send_command_ok),
    // so we can inspect the reply status directly.
    let resp = client
        .log("BOGUS_LEVEL_12345")
        .await
        .expect("send_command should not fail at transport level");

    assert_eq!(
        resp.reply_status(),
        ReplyStatus::Err,
        "expected -ERR reply, got: {:?}",
        resp.reply_text()
    );
    assert!(
        resp.reply_text()
            .unwrap_or("")
            .starts_with("-ERR"),
        "reply text should start with -ERR: {:?}",
        resp.reply_text()
    );

    // into_result() should convert to CommandFailed
    let err = resp
        .into_result()
        .unwrap_err();
    assert!(
        matches!(err, EslError::CommandFailed { .. }),
        "expected CommandFailed, got: {:?}",
        err
    );
}

#[tokio::test]
#[ignore]
async fn live_noevents_stops_delivery() {
    let (client, mut events, _permit) = connect().await;
    let subclass = format!("esl_test::noev_{}", std::process::id());

    client
        .subscribe_events_raw(EventFormat::Plain, &format!("CUSTOM {}", subclass))
        .await
        .unwrap();

    // Fire event and confirm delivery
    let mut evt1 = EslEvent::with_type(EslEventType::Custom);
    evt1.set_header("Event-Name", "CUSTOM");
    evt1.set_header("Event-Subclass", subclass.clone());
    evt1.set_header("X-Phase", "before");
    client
        .sendevent(evt1)
        .await
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt)))
                if evt.header(EventHeader::EventSubclass) == Some(subclass.as_str()) =>
            {
                break
            }
            Ok(Some(Ok(_))) => continue,
            other => panic!("expected custom event before noevents: {:?}", other),
        }
    }

    // Unsubscribe from all events
    client
        .noevents()
        .await
        .unwrap();

    // Fire another event — should not arrive
    let mut evt2 = EslEvent::with_type(EslEventType::Custom);
    evt2.set_header("Event-Name", "CUSTOM");
    evt2.set_header("Event-Subclass", subclass.clone());
    evt2.set_header("X-Phase", "after");
    client
        .sendevent(evt2)
        .await
        .unwrap();

    match tokio::time::timeout(Duration::from_secs(2), events.recv()).await {
        Err(_) => {} // timeout — correct
        Ok(Some(Ok(evt))) => panic!(
            "received event after noevents: {:?} phase={}",
            evt.event_type(),
            evt.header_str("X-Phase")
                .unwrap_or("?")
        ),
        Ok(Some(Err(e))) => panic!("event error: {}", e),
        Ok(None) => {}
    }
}

#[tokio::test]
#[ignore]
async fn live_nixevent_selective_unsubscribe() {
    let (client, mut events, _permit) = connect().await;
    let subclass = format!("esl_test::nix_{}", std::process::id());

    // Subscribe to both HEARTBEAT and CUSTOM
    client
        .subscribe_events_raw(
            EventFormat::Plain,
            &format!("HEARTBEAT CUSTOM {}", subclass),
        )
        .await
        .unwrap();

    // Unsubscribe only HEARTBEAT
    client
        .nixevent(&[EslEventType::Heartbeat])
        .await
        .unwrap();

    // Send a custom event — should still arrive
    let mut event = EslEvent::with_type(EslEventType::Custom);
    event.set_header("Event-Name", "CUSTOM");
    event.set_header("Event-Subclass", subclass.clone());
    client
        .sendevent(event)
        .await
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                assert_ne!(
                    evt.event_type(),
                    Some(EslEventType::Heartbeat),
                    "received HEARTBEAT after nixevent"
                );
                if evt.header(EventHeader::EventSubclass) == Some(subclass.as_str()) {
                    return; // custom event delivered — nixevent was selective
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("event stream closed"),
            Err(_) => break,
        }
    }
    panic!("did not receive custom event after nixevent HEARTBEAT");
}

#[tokio::test]
#[ignore]
async fn live_api_err_body() {
    let (client, _events, _permit) = connect().await;

    // api with a non-existent command returns -ERR in the body
    let resp = client
        .api("nonexistent_command_xyz")
        .await
        .unwrap();
    let err = resp
        .api_result()
        .unwrap_err();
    assert!(
        matches!(err, EslError::CommandFailed { .. }),
        "expected CommandFailed, got: {}",
        err
    );
}

#[tokio::test]
#[ignore]
async fn live_channel_timetable_on_create() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(
            EventFormat::Plain,
            &[EslEventType::ChannelCreate, EslEventType::ChannelDestroy],
        )
        .await
        .unwrap();

    // Originate a call to &park() — creates a channel that immediately parks
    let resp = client
        .api("originate null/test &park()")
        .await
        .unwrap();
    let uuid = match resp.api_result() {
        Ok(uuid) => uuid.to_string(),
        Err(e) => {
            eprintln!("originate failed ({}), skipping timetable test", e);
            return;
        }
    };

    // Wait for CHANNEL_CREATE with our UUID
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut found_timetable = false;
    while Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.event_type() == Some(EslEventType::ChannelCreate)
                    && evt.unique_id() == Some(&uuid)
                {
                    let tt = evt
                        .caller_timetable()
                        .expect("timetable should parse without error")
                        .expect("CHANNEL_CREATE should have Caller timetable");

                    // created must be a positive epoch-microsecond timestamp
                    let created = tt
                        .created
                        .expect("created should be present on CHANNEL_CREATE");
                    assert!(
                        created > 1_000_000_000_000_000,
                        "created timestamp should be a recent epoch-us value: {}",
                        created
                    );
                    let profile_created = tt
                        .profile_created
                        .expect("profile_created should be present on CHANNEL_CREATE");
                    assert!(
                        profile_created > 1_000_000_000_000_000,
                        "profile_created should be a recent epoch-us value: {}",
                        profile_created
                    );
                    // answered/hungup should be 0 at creation time
                    assert_eq!(tt.answered, Some(0), "answered should be 0 at create");
                    assert_eq!(tt.hungup, Some(0), "hungup should be 0 at create");

                    found_timetable = true;
                    break;
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("event stream closed"),
            Err(_) => break,
        }
    }

    // Clean up: hang up the parked channel
    let _ = client
        .api(&format!("uuid_kill {}", uuid))
        .await;

    assert!(
        found_timetable,
        "did not receive CHANNEL_CREATE with timetable for {}",
        uuid
    );
}

// --- L11: Repeating SIP header round-trip tests ---

#[tokio::test]
#[ignore]
async fn live_sendevent_comma_separated_sip_header() {
    let (client, mut events, _permit) = connect().await;

    let subclass = format!("esl_test::pai_csv_{}", std::process::id());

    client
        .subscribe_events_raw(EventFormat::Plain, &format!("CUSTOM {}", subclass))
        .await
        .unwrap();

    let mut event = EslEvent::with_type(EslEventType::Custom);
    event.set_header("Event-Name", "CUSTOM");
    event.set_header("Event-Subclass", subclass.clone());
    // RFC 3325 comma-separated format: two identities in one header value
    event.set_header(
        "variable_sip_P-Asserted-Identity",
        "<sip:alice@atlanta.example.com>, <tel:+15551234567>",
    );

    client
        .sendevent(event)
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.header(EventHeader::EventSubclass) == Some(subclass.as_str()) {
                    assert_eq!(
                        evt.variable_str("sip_P-Asserted-Identity"),
                        Some("<sip:alice@atlanta.example.com>, <tel:+15551234567>"),
                        "comma-separated P-Asserted-Identity should survive round-trip"
                    );
                    return;
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("event stream closed"),
            Err(_) => break,
        }
    }
    panic!("did not receive custom event with subclass {}", subclass);
}

#[tokio::test]
#[ignore]
async fn live_sendevent_array_sip_header() {
    use freeswitch_types::EslArray;

    let (client, mut events, _permit) = connect().await;

    let subclass = format!("esl_test::pai_arr_{}", std::process::id());

    client
        .subscribe_events_raw(EventFormat::Plain, &format!("CUSTOM {}", subclass))
        .await
        .unwrap();

    let mut event = EslEvent::with_type(EslEventType::Custom);
    event.set_header("Event-Name", "CUSTOM");
    event.set_header("Event-Subclass", subclass.clone());
    // ARRAY format: repeating SIP header stored as separate values
    event
        .push_header(
            "variable_sip_P-Asserted-Identity",
            "<sip:alice@atlanta.example.com>",
        )
        .unwrap();
    event
        .push_header("variable_sip_P-Asserted-Identity", "<tel:+15551234567>")
        .unwrap();

    client
        .sendevent(event)
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.header(EventHeader::EventSubclass) == Some(subclass.as_str()) {
                    let raw = evt
                        .variable_str("sip_P-Asserted-Identity")
                        .expect("P-Asserted-Identity should be present");
                    let arr = EslArray::parse(raw).expect("should parse as ARRAY");
                    assert_eq!(arr.len(), 2, "expected 2 identities in ARRAY");
                    assert_eq!(arr.items()[0], "<sip:alice@atlanta.example.com>");
                    assert_eq!(arr.items()[1], "<tel:+15551234567>");
                    return;
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("event stream closed"),
            Err(_) => break,
        }
    }
    panic!("did not receive custom event with subclass {}", subclass);
}

#[tokio::test]
#[ignore]
async fn live_sendevent_repeated_diversion_header() {
    use freeswitch_types::EslArray;

    let (client, mut events, _permit) = connect().await;

    let subclass = format!("esl_test::diversion_{}", std::process::id());

    client
        .subscribe_events_raw(EventFormat::Plain, &format!("CUSTOM {}", subclass))
        .await
        .unwrap();

    let mut event = EslEvent::with_type(EslEventType::Custom);
    event.set_header("Event-Name", "CUSTOM");
    event.set_header("Event-Subclass", subclass.clone());
    // SIP Diversion header (RFC 5806) with history info containing SIP URI params
    event
        .push_header(
            "variable_sip_h_Diversion",
            "<sip:+15551234567@gw.example.com;reason=unconditional>",
        )
        .unwrap();
    event
        .push_header(
            "variable_sip_h_Diversion",
            "<sip:+15559876543@proxy.example.com;reason=no-answer;counter=3>",
        )
        .unwrap();

    client
        .sendevent(event)
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.header(EventHeader::EventSubclass) == Some(subclass.as_str()) {
                    let raw = evt
                        .variable_str("sip_h_Diversion")
                        .expect("Diversion variable should be present");
                    let arr = EslArray::parse(raw).expect("should parse as ARRAY");
                    assert_eq!(arr.len(), 2, "expected 2 Diversion entries");
                    assert_eq!(
                        arr.items()[0],
                        "<sip:+15551234567@gw.example.com;reason=unconditional>"
                    );
                    assert_eq!(
                        arr.items()[1],
                        "<sip:+15559876543@proxy.example.com;reason=no-answer;counter=3>"
                    );
                    return;
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("event stream closed"),
            Err(_) => break,
        }
    }
    panic!("did not receive custom event with subclass {}", subclass);
}

/// bgapi originate via the builder, wait for BACKGROUND_JOB, return the UUID.
async fn bgapi_originate_ok(
    client: &EslClient,
    events: &mut freeswitch_esl_tokio::EslEventStream,
    cmd: &Originate,
) -> String {
    let resp = client
        .bgapi(&cmd.to_string())
        .await
        .expect("bgapi originate transport error");
    let job_uuid = resp
        .job_uuid()
        .expect("bgapi should return Job-UUID header")
        .to_string();

    // Wait for the BACKGROUND_JOB event with our Job-UUID
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.event_type() == Some(EslEventType::BackgroundJob)
                    && evt.job_uuid() == Some(&job_uuid)
                {
                    let body = evt
                        .body()
                        .expect("BACKGROUND_JOB should have a body");
                    let uuid = parse_api_body(body).expect("originate failed");
                    return uuid.to_string();
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("event stream closed"),
            Err(_) => break,
        }
    }
    panic!("timeout waiting for BACKGROUND_JOB {}", job_uuid);
}

/// Kill a channel by UUID, ignoring errors (channel may already be gone).
async fn kill_channel(client: &EslClient, uuid: &str) {
    let cmd = UuidKill::new(uuid);
    let _ = client
        .api(&cmd.to_string())
        .await;
}

#[tokio::test]
#[ignore]
async fn live_originate_application_target() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(EventFormat::Plain, &[EslEventType::BackgroundJob])
        .await
        .unwrap();

    // Single application target: &park() holds the channel, bgapi returns immediately
    let cmd = Originate::application(
        Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("test")),
        Application::simple("park"),
    );

    let uuid = bgapi_originate_ok(&client, &mut events, &cmd).await;
    kill_channel(&client, &uuid).await;
}

#[tokio::test]
#[ignore]
async fn live_originate_extension_target() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(EventFormat::Plain, &[EslEventType::BackgroundJob])
        .await
        .unwrap();

    // Extension target: route through XML dialplan to 9199 (echo) in test context
    let cmd = Originate::extension(
        Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("test")),
        "9199",
    )
    .dialplan(DialplanType::Xml)
    .unwrap()
    .context("test");

    let uuid = bgapi_originate_ok(&client, &mut events, &cmd).await;
    kill_channel(&client, &uuid).await;
}

#[tokio::test]
#[ignore]
async fn live_originate_inline_target() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(EventFormat::Plain, &[EslEventType::BackgroundJob])
        .await
        .unwrap();

    // Inline dialplan: answer then hangup (instant)
    let cmd = Originate::inline(
        Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("test")),
        vec![
            Application::simple("answer"),
            Application::new("hangup", Some("NORMAL_CLEARING")),
        ],
    )
    .unwrap();

    // answer+hangup is instant, bgapi returns the result quickly
    let uuid = bgapi_originate_ok(&client, &mut events, &cmd).await;
    // Channel already hung up, but kill just in case
    kill_channel(&client, &uuid).await;
}

#[tokio::test]
#[ignore]
async fn live_originate_timeout_fills_positional_gaps() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(EventFormat::Plain, &[EslEventType::BackgroundJob])
        .await
        .unwrap();

    // Timeout without cid_name/cid_num forces `undef` placeholders on the wire.
    // Verifies FreeSWITCH accepts `undef` as a NULL positional arg.
    let cmd = Originate::application(
        Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("test")),
        Application::simple("park"),
    )
    .timeout(Duration::from_secs(5));

    let uuid = bgapi_originate_ok(&client, &mut events, &cmd).await;
    kill_channel(&client, &uuid).await;
}

#[tokio::test]
#[ignore]
async fn live_log_events_have_log_type() {
    let (client, mut events, _permit) = connect().await;

    // Enable log forwarding at DEBUG level to generate log traffic
    let resp = client
        .log("DEBUG")
        .await
        .unwrap();
    assert!(
        resp.is_success(),
        "log command failed: {:?}",
        resp.reply_text()
    );

    // Trigger log output by running an API command
    client
        .api("status")
        .await
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.event_type() == Some(EslEventType::Log) {
                    // Verify log-specific headers are present
                    assert!(
                        evt.header(EventHeader::LogLevel)
                            .is_some(),
                        "log event should have Log-Level header"
                    );
                    assert!(
                        evt.header_str("Log-File")
                            .is_some(),
                        "log event should have Log-File header"
                    );
                    assert!(
                        evt.body()
                            .is_some(),
                        "log event should have a body with the log text"
                    );

                    // Disable log forwarding before returning
                    let _ = client
                        .nolog()
                        .await;
                    return;
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("event stream closed"),
            Err(_) => break,
        }
    }

    let _ = client
        .nolog()
        .await;
    panic!("did not receive any log event with EslEventType::Log");
}

// --- L2: Liveness detection live tests ---

#[tokio::test]
#[ignore]
async fn live_liveness_heartbeat_resets_timer() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(EventFormat::Plain, &[EslEventType::Heartbeat])
        .await
        .unwrap();

    // Set liveness timeout to 30s, well above heartbeat interval (~20s)
    client.set_liveness_timeout(Duration::from_secs(30));

    // Wait for two heartbeats to confirm the timer resets
    let mut heartbeat_count = 0;
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.event_type() == Some(EslEventType::Heartbeat) {
                    heartbeat_count += 1;
                    if heartbeat_count >= 2 {
                        break;
                    }
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("connection closed unexpectedly"),
            Err(_) => break,
        }
    }

    assert!(client.is_connected(), "connection should still be alive");
    assert!(
        heartbeat_count >= 2,
        "expected at least 2 heartbeats, got {}",
        heartbeat_count
    );
}

// --- L3: Command timeout live tests ---

#[tokio::test]
#[ignore]
async fn live_command_timeout_msleep() {
    let (client, _events, _permit) = connect().await;

    // Set a short command timeout, then send a blocking api call
    client.set_command_timeout(Duration::from_secs(1));

    let result = client
        .api("msleep 5000")
        .await;

    assert!(
        matches!(result, Err(EslError::Timeout { .. })),
        "expected Timeout error, got: {:?}",
        result
    );

    // Verify the connection is still usable after timeout.
    // Increase timeout for the recovery command.
    client.set_command_timeout(Duration::from_secs(10));

    // msleep result may arrive late and consume the next reply slot.
    // Wait for the blocked msleep to complete on the server side.
    tokio::time::sleep(Duration::from_secs(5)).await;

    let resp = client
        .api("status")
        .await;
    assert!(
        resp.is_ok(),
        "command after timeout should succeed: {:?}",
        resp
    );
}

// --- L4: Event filter live tests ---

#[tokio::test]
#[ignore]
async fn live_filter_event_name() {
    let (client, mut events, _permit) = connect().await;

    // Subscribe to multiple event types
    client
        .subscribe_events(
            EventFormat::Plain,
            &[EslEventType::Heartbeat, EslEventType::BackgroundJob],
        )
        .await
        .unwrap();

    // Filter: only receive HEARTBEAT
    client
        .filter(EventHeader::EventName, "HEARTBEAT")
        .await
        .unwrap();

    // Fire a bgapi to generate a BACKGROUND_JOB event
    client
        .bgapi("status")
        .await
        .unwrap();

    // We should only see HEARTBEAT, not BACKGROUND_JOB
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut got_heartbeat = false;
    while Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                assert_ne!(
                    evt.event_type(),
                    Some(EslEventType::BackgroundJob),
                    "BACKGROUND_JOB should have been filtered out"
                );
                if evt.event_type() == Some(EslEventType::Heartbeat) {
                    got_heartbeat = true;
                    break;
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("connection closed"),
            Err(_) => break,
        }
    }
    assert!(
        got_heartbeat,
        "should have received HEARTBEAT through filter"
    );

    // Delete filter, verify BACKGROUND_JOB now arrives
    client
        .filter_delete(EventHeader::EventName, Some("HEARTBEAT"))
        .await
        .unwrap();

    client
        .bgapi("status")
        .await
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.event_type() == Some(EslEventType::BackgroundJob) {
                    return; // filter successfully removed
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("connection closed"),
            Err(_) => break,
        }
    }
    panic!("BACKGROUND_JOB not received after filter_delete");
}

// --- L6: Command builder verification against real FS ---

#[tokio::test]
#[ignore]
async fn live_uuid_setvar_getvar_round_trip() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(EventFormat::Plain, &[EslEventType::BackgroundJob])
        .await
        .unwrap();

    // Create a channel to work with
    let cmd = Originate::application(
        Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("test")),
        Application::simple("park"),
    );
    let uuid = bgapi_originate_ok(&client, &mut events, &cmd).await;

    // Set a variable on the channel
    let set_cmd = UuidSetVar::new(&uuid, "esl_test_var", "hello_world");
    let resp = client
        .api(&set_cmd.to_string())
        .await
        .unwrap();
    resp.api_result()
        .expect("uuid_setvar failed");

    // Get the variable back (uuid_getvar returns the raw value, no +OK prefix)
    let get_cmd = UuidGetVar::new(&uuid, "esl_test_var");
    let resp = client
        .api(&get_cmd.to_string())
        .await
        .unwrap();
    assert_eq!(
        resp.api_result()
            .unwrap(),
        "hello_world",
        "uuid_getvar should return the value we set"
    );

    kill_channel(&client, &uuid).await;
}

#[tokio::test]
#[ignore]
async fn live_uuid_kill_with_cause() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(
            EventFormat::Plain,
            &[
                EslEventType::BackgroundJob,
                EslEventType::ChannelHangupComplete,
            ],
        )
        .await
        .unwrap();

    let cmd = Originate::application(
        Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("test")),
        Application::simple("park"),
    );
    let uuid = bgapi_originate_ok(&client, &mut events, &cmd).await;

    // Kill with a specific hangup cause
    let kill_cmd = UuidKill::with_cause(&uuid, freeswitch_esl_tokio::HangupCause::UserBusy);
    let resp = client
        .api(&kill_cmd.to_string())
        .await
        .unwrap();
    resp.api_result()
        .expect("uuid_kill failed");

    // Verify the hangup cause in the CHANNEL_HANGUP_COMPLETE event
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.event_type() == Some(EslEventType::ChannelHangupComplete)
                    && evt.unique_id() == Some(&uuid)
                {
                    let cause = evt
                        .hangup_cause()
                        .expect("should parse hangup cause")
                        .expect("should have hangup cause");
                    assert_eq!(
                        cause,
                        freeswitch_esl_tokio::HangupCause::UserBusy,
                        "hangup cause should be USER_BUSY"
                    );
                    return;
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("connection closed"),
            Err(_) => break,
        }
    }
    panic!("did not receive CHANNEL_HANGUP_COMPLETE for {}", uuid);
}

// --- L7: Connection lifecycle tests ---

#[tokio::test]
#[ignore]
async fn live_disconnect_status() {
    let (client, _events, _permit) = connect().await;
    assert!(client.is_connected());

    client
        .disconnect()
        .await
        .unwrap();

    // Allow the reader loop to notice the shutdown
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(
        !client.is_connected(),
        "should be disconnected after disconnect()"
    );
    // The final status may be ClientRequested or ServerNotice depending on
    // timing: we set ClientRequested before shutdown, but the reader loop
    // may process the server's goodbye message and overwrite with ServerNotice.
    assert!(
        matches!(
            client.status(),
            ConnectionStatus::Disconnected(
                DisconnectReason::ClientRequested | DisconnectReason::ServerNotice { .. }
            )
        ),
        "status should be ClientRequested or ServerNotice, got: {:?}",
        client.status()
    );
}

#[tokio::test]
#[ignore]
async fn live_reconnect_clean_state() {
    // Connect, disconnect, then reconnect and verify clean state
    let (client1, _events1, _permit1) = connect().await;
    assert!(client1.is_connected());

    let resp1 = client1
        .api("hostname")
        .await
        .unwrap();
    let hostname = resp1
        .body()
        .unwrap()
        .trim()
        .to_string();

    client1
        .disconnect()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!client1.is_connected());

    // Reconnect
    let (client2, _events2, _permit2) = connect().await;
    assert!(client2.is_connected());

    let resp2 = client2
        .api("hostname")
        .await
        .unwrap();
    assert_eq!(
        resp2
            .body()
            .unwrap()
            .trim(),
        hostname,
        "hostname should match after reconnect"
    );
}

// --- L8: sendevent UUID in response ---

#[tokio::test]
#[ignore]
async fn live_sendevent_returns_event_uuid() {
    let (client, _events, _permit) = connect().await;

    let mut event = EslEvent::with_type(EslEventType::Custom);
    event.set_header("Event-Name", "CUSTOM");
    event.set_header("Event-Subclass", "esl_test::uuid_check");

    let resp = client
        .sendevent(event)
        .await
        .unwrap();
    assert!(resp.is_success());

    let uuid = resp.event_uuid();
    assert!(
        uuid.is_some(),
        "sendevent should return event UUID in +OK reply, got: {:?}",
        resp.reply_text()
    );
    // UUID should look like a UUID (36 chars with dashes)
    let uuid = uuid.unwrap();
    assert!(
        uuid.len() >= 36,
        "event UUID should be at least 36 chars: {}",
        uuid
    );
}

// --- L9: bgapi correlation ---

#[tokio::test]
#[ignore]
async fn live_bgapi_correlation() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(EventFormat::Plain, &[EslEventType::BackgroundJob])
        .await
        .unwrap();

    // Send multiple bgapi commands and collect their Job-UUIDs
    let resp1 = client
        .bgapi("status")
        .await
        .unwrap();
    let job1 = resp1
        .job_uuid()
        .expect("bgapi should return Job-UUID")
        .to_string();

    let resp2 = client
        .bgapi("version")
        .await
        .unwrap();
    let job2 = resp2
        .job_uuid()
        .expect("bgapi should return Job-UUID")
        .to_string();

    let resp3 = client
        .bgapi("hostname")
        .await
        .unwrap();
    let job3 = resp3
        .job_uuid()
        .expect("bgapi should return Job-UUID")
        .to_string();

    assert_ne!(job1, job2, "Job-UUIDs should be unique");
    assert_ne!(job2, job3, "Job-UUIDs should be unique");

    // Collect BACKGROUND_JOB events and match them to our Job-UUIDs
    let expected: std::collections::HashSet<String> = [job1.clone(), job2.clone(), job3.clone()]
        .into_iter()
        .collect();
    let mut matched = std::collections::HashSet::new();

    let deadline = Instant::now() + Duration::from_secs(10);
    while matched.len() < 3 && Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.event_type() == Some(EslEventType::BackgroundJob) {
                    if let Some(job_uuid) = evt.job_uuid() {
                        if expected.contains(job_uuid) {
                            assert!(
                                evt.body()
                                    .is_some(),
                                "BACKGROUND_JOB for {} should have body",
                                job_uuid
                            );
                            matched.insert(job_uuid.to_string());
                        }
                    }
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("connection closed"),
            Err(_) => break,
        }
    }

    assert_eq!(
        matched.len(),
        3,
        "should match all 3 bgapi jobs, matched: {:?}, expected: {:?}",
        matched,
        expected
    );
}

// --- L10: bgapi single round-trip ---

#[tokio::test]
#[ignore]
async fn live_bgapi_single_round_trip() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(EventFormat::Plain, &[EslEventType::BackgroundJob])
        .await
        .unwrap();

    let resp = client
        .bgapi("status")
        .await
        .unwrap();
    let job_uuid = resp
        .job_uuid()
        .expect("bgapi should return Job-UUID")
        .to_string();

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.event_type() == Some(EslEventType::BackgroundJob)
                    && evt.job_uuid() == Some(job_uuid.as_str())
                {
                    let body = evt
                        .body()
                        .expect("BACKGROUND_JOB should have a body");
                    assert!(!body.is_empty(), "body should contain status output");
                    assert_eq!(
                        evt.job_uuid(),
                        Some(job_uuid.as_str()),
                        "event Job-UUID must match"
                    );
                    return;
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("connection closed"),
            Err(_) => panic!("timeout waiting for BACKGROUND_JOB {}", job_uuid),
        }
    }
}

/// Verify that header key normalization works against real FreeSWITCH.
///
/// FreeSWITCH emits headers with inconsistent casing from multiple C code paths.
/// This test confirms that known EventHeader variants resolve, no duplicate keys
/// exist after normalization, and channel variables preserve underscore keys.
#[tokio::test]
#[ignore]
async fn live_header_normalization() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(EventFormat::Plain, &[EslEventType::ChannelCreate])
        .await
        .expect("subscribe failed");

    // api originate is synchronous — events are queued while it blocks
    let originate = Originate::application(
        Endpoint::from(LoopbackEndpoint::new("9199")),
        Application::park(),
    );
    let resp = client
        .api(&originate.to_string())
        .await
        .expect("originate failed");
    let uuid = resp
        .api_result()
        .expect("originate returned error")
        .to_string();

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut saw_channel_create = false;

    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.event_type() != Some(EslEventType::ChannelCreate) {
                    continue;
                }

                // Known EventHeader lookups must work
                assert!(evt
                    .header(EventHeader::UniqueId)
                    .is_some());
                assert!(evt
                    .header(EventHeader::ChannelState)
                    .is_some());
                assert!(evt
                    .header(EventHeader::EventName)
                    .is_some());

                // No duplicate keys (normalization collapsed different casings)
                let headers = evt.headers();
                let unique_lower: std::collections::HashSet<String> = headers
                    .keys()
                    .map(|k| k.to_ascii_lowercase())
                    .collect();
                assert_eq!(
                    headers.len(),
                    unique_lower.len(),
                    "duplicate keys with different casing found in headers"
                );

                // Channel variables preserve underscore keys
                if let Some(dir) = evt.variable_str("direction") {
                    assert!(!dir.is_empty());
                }

                saw_channel_create = true;
                break;
            }
            Ok(Some(Err(e))) => panic!("event error: {e}"),
            Ok(None) => panic!("connection closed before CHANNEL_CREATE"),
            Err(_) => break,
        }
    }

    kill_channel(&client, &uuid).await;
    assert!(saw_channel_create, "never received CHANNEL_CREATE event");
}

/// Verify that CODEC events have normalized codec headers.
///
/// switch_core_codec.c emits lowercase headers while switch_channel.c
/// emits Title-Case — both should normalize to the same canonical key.
#[tokio::test]
#[ignore]
async fn live_codec_header_normalization() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(EventFormat::Plain, &[EslEventType::Codec])
        .await
        .expect("subscribe failed");

    let originate = Originate::application(
        Endpoint::from(LoopbackEndpoint::new("9199")),
        Application::park(),
    );
    let resp = client
        .api(&originate.to_string())
        .await
        .expect("originate failed");
    let uuid = resp
        .api_result()
        .expect("originate returned error")
        .to_string();

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut saw_codec = false;

    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.event_type() != Some(EslEventType::Codec) {
                    continue;
                }

                // Codec headers accessible via typed API
                if let Some(codec) = evt.header(EventHeader::ChannelReadCodecName) {
                    assert!(!codec.is_empty());
                }

                // No duplicate keys after normalization
                let headers = evt.headers();
                let unique_lower: std::collections::HashSet<String> = headers
                    .keys()
                    .map(|k| k.to_ascii_lowercase())
                    .collect();
                assert_eq!(
                    headers.len(),
                    unique_lower.len(),
                    "CODEC event has duplicate keys with different casing: {:?}",
                    headers
                        .keys()
                        .filter(|k| {
                            headers
                                .keys()
                                .any(|other| other != *k && other.eq_ignore_ascii_case(k))
                        })
                        .collect::<Vec<_>>()
                );

                saw_codec = true;
                break;
            }
            Ok(Some(Err(e))) => panic!("event error: {e}"),
            Ok(None) => panic!("connection closed before CODEC event"),
            Err(_) => break,
        }
    }

    kill_channel(&client, &uuid).await;
    assert!(saw_codec, "never received CODEC event");
}

/// Verify that userauth with a long Allowed-Events list survives
/// FreeSWITCH's reply[512] truncation in mod_event_socket.c.
///
/// Requires the `many-events@default` user configured in FreeSWITCH with
/// a long esl-allowed-events list that triggers the 512-byte overflow.
/// The user has `esl-allowed-api = "show uuid_dump originate uuid_kill"`
/// and `esl-allowed-log = false`, but the 512-byte reply buffer overflow
/// truncates these headers (Allowed-LOG is always fully lost).
#[tokio::test]
#[ignore]
async fn live_connect_userauth_truncated_response() {
    let permit = CONN_SEMAPHORE
        .acquire()
        .await
        .expect("semaphore closed");
    let opts = EslConnectOptions::new().with_connect_timeout(Duration::from_secs(5));
    let (client, _events) = EslClient::connect_with_user_and_options(
        ESL_HOST,
        ESL_PORT,
        "many-events@default",
        ESL_PASSWORD,
        opts,
    )
    .await
    .expect("userauth with truncated response should succeed");

    assert!(client.is_connected());

    let auth = client
        .auth_response()
        .expect("auth_response should be present for inbound connection");
    assert_eq!(auth.reply_text(), Some("+OK accepted"));
    assert!(
        auth.header("Allowed-Events")
            .is_some(),
        "Allowed-Events header should be present (possibly truncated)"
    );

    // Allowed-LOG is the last header in the reply — with the 512-byte
    // overflow it's always fully truncated. Its absence proves the
    // salvage path was triggered (a non-truncated response would have it).
    assert!(
        auth.header("Allowed-Log")
            .is_none(),
        "Allowed-Log should be missing due to reply[512] truncation. \
         If this fails, FreeSWITCH may have been patched — \
         the workaround is no longer needed for this configuration."
    );

    // Allowed-API should be present but truncated (fewer commands than
    // the configured "show uuid_dump originate uuid_kill").
    if let Some(api) = auth.header("Allowed-Api") {
        let commands: Vec<&str> = api
            .split_whitespace()
            .collect();
        assert!(
            commands.len() < 4,
            "Allowed-API has all 4 commands ({api}), expected truncation"
        );
    }

    drop(permit);
}

/// The outbound `connect` reply carries every channel variable as a
/// `variable_<name>` header. SIP-passthrough variables (`variable_sip_h_*`)
/// must preserve the original SIP wire casing — `EslResponse::header()`
/// must not collapse `variable_sip_h_X-MixedCase` into a lowercase bucket.
///
/// Drives a real outbound socket against FS to verify the wire path.
#[tokio::test]
#[ignore]
async fn live_outbound_connect_response_preserves_underscored_case() {
    use tokio::net::TcpListener;

    let (inbound, _events, permit) = connect().await;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind outbound listener");
    let port = listener
        .local_addr()
        .unwrap()
        .port();

    // Inject a channel variable with mixed-case underscored name.
    // FS preserves the variable name verbatim and emits it as
    // `variable_<name>` in the connect reply.
    let mut vars = Variables::new(VariablesType::Default);
    vars.insert("MyMixed_Case_Var", "preserved");

    let endpoint = LoopbackEndpoint::new("9199")
        .with_context("test")
        .with_variables(vars);

    let cmd = Originate::application(
        Endpoint::Loopback(endpoint),
        Application::new("socket", Some(format!("127.0.0.1:{} async full", port))),
    );

    // bgapi returns immediately; the call proceeds asynchronously and
    // FS dials our listener.
    inbound
        .bgapi(&cmd.to_string())
        .await
        .expect("bgapi originate");

    let (outbound, _outbound_events) = tokio::time::timeout(
        Duration::from_secs(10),
        EslClient::accept_outbound(&listener),
    )
    .await
    .expect("timed out waiting for outbound connection")
    .expect("accept_outbound failed");

    let resp = outbound
        .connect_session()
        .await
        .expect("connect_session failed");

    // Case-sensitive: exact-cased variable name must hit.
    assert_eq!(
        resp.header("variable_MyMixed_Case_Var"),
        Some("preserved"),
        "exact-case variable lookup must succeed"
    );

    // Case-sensitive: wrong-cased variants must NOT match. If the
    // case_index were unfiltered, lowercased lookups would resolve
    // through the alias and return the wrong value (or worse, collapse
    // distinct headers like X-Foo and X-foo).
    assert_eq!(
        resp.header("variable_mymixed_case_var"),
        None,
        "lowercased variable_* must not match — SIP wire casing is preserved"
    );
    assert_eq!(
        resp.header("VARIABLE_MYMIXED_CASE_VAR"),
        None,
        "uppercased variable_* must not match"
    );

    // Case-insensitive: framing headers (no underscore) resolve in any case.
    let canonical = resp
        .header("Reply-Text")
        .expect("Reply-Text must be present on connect reply");
    assert_eq!(resp.header("reply-text"), Some(canonical));
    assert_eq!(resp.header("REPLY-TEXT"), Some(canonical));

    // Cleanup.
    if let Some(uuid) = resp
        .header("Channel-Unique-ID")
        .or_else(|| resp.header("Unique-ID"))
        .map(String::from)
    {
        kill_channel(&inbound, &uuid).await;
    }

    drop(permit);
}

/// The reported ordering defect, end to end: `Custom` listed first with another
/// event type after it. Before the fix the wire read `event plain CUSTOM
/// HEARTBEAT`, FreeSWITCH registered `HEARTBEAT` as a subclass name, and the
/// heartbeat never arrived while the `+OK` said the subscription was healthy.
///
/// A bare `CUSTOM` delivers nothing of its own -- FreeSWITCH gates custom
/// events on the listener's subclass hash, which only `ALL` populates -- so the
/// first event to arrive here is the heartbeat or nothing.
#[tokio::test]
#[ignore]
async fn live_custom_first_does_not_swallow_later_event_types() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(
            EventFormat::Plain,
            &[EslEventType::Custom, EslEventType::Heartbeat],
        )
        .await
        .unwrap();

    let event = tokio::time::timeout(Duration::from_secs(25), events.recv())
        .await
        .expect("HEARTBEAT never arrived -- swallowed as a CUSTOM subclass name")
        .expect("channel closed")
        .expect("event error");

    assert_eq!(event.event_type(), Some(EslEventType::Heartbeat));
}
