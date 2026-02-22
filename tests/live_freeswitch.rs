//! Integration tests against a live FreeSWITCH instance.
//!
//! These tests require FreeSWITCH ESL on 127.0.0.1:8022 with password ClueCon.
//! Run with: cargo test --test live_freeswitch -- --ignored

use freeswitch_esl_tokio::{
    EslClient, EslError, EslEvent, EslEventPriority, EslEventType, EventFormat, ReplyStatus,
};
use std::time::Duration;
use tokio::time::Instant;

const ESL_HOST: &str = "127.0.0.1";
const ESL_PORT: u16 = 8022;
const ESL_PASSWORD: &str = "ClueCon";

async fn connect() -> (EslClient, freeswitch_esl_tokio::EslEventStream) {
    let (client, events) = EslClient::connect(ESL_HOST, ESL_PORT, ESL_PASSWORD)
        .await
        .expect("failed to connect to FreeSWITCH");
    client.set_command_timeout(Duration::from_secs(10));
    (client, events)
}

#[tokio::test]
#[ignore]
async fn live_connect_and_status() {
    let (client, _events) = connect().await;
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
    let (client, mut events) = connect().await;

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
        .header("Core-UUID")
        .is_some());
}

#[tokio::test]
#[ignore]
async fn live_sendevent_with_priority() {
    let (client, _events) = connect().await;

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
    let (client, _events) = connect().await;

    let mut event = EslEvent::with_type(EslEventType::Custom);
    event.set_header("Event-Name", "CUSTOM");
    event.set_header("Event-Subclass", "esl_test::array");
    event.push_header("X-Test-Array", "value1");
    event.push_header("X-Test-Array", "value2");
    event.push_header("X-Test-Array", "value3");

    assert_eq!(
        event.header("X-Test-Array"),
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
    let (client, mut events) = connect().await;

    let subclass = format!("esl_test::live_{}", std::process::id());

    client
        .subscribe_events_raw(EventFormat::Plain, &format!("CUSTOM {}", subclass))
        .await
        .unwrap();
    let mut event = EslEvent::with_type(EslEventType::Custom);
    event.set_header("Event-Name", "CUSTOM");
    event.set_header("Event-Subclass", subclass.clone());
    event.set_priority(EslEventPriority::Normal);
    event.push_header("X-Test-Data", "hello");
    event.push_header("X-Test-Data", "world");

    client
        .sendevent(event)
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.header("Event-Subclass") == Some(subclass.as_str()) {
                    assert_eq!(evt.header("priority"), Some("NORMAL"));
                    assert_eq!(evt.header("X-Test-Data"), Some("ARRAY::hello|:world"));
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
    let (client, _events) = connect().await;

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
    let (client, _events) = connect().await;

    // subscribe_events uses send_command_ok → into_result(), so Ok means +OK
    client
        .subscribe_events(EventFormat::Plain, &[EslEventType::Heartbeat])
        .await
        .expect("subscribe should return +OK");
}

#[tokio::test]
#[ignore]
async fn live_reply_status_err() {
    let (client, _events) = connect().await;

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
    let (client, mut events) = connect().await;
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
            Ok(Some(Ok(evt))) if evt.header("Event-Subclass") == Some(subclass.as_str()) => break,
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
            evt.header("X-Phase")
                .unwrap_or("?")
        ),
        Ok(Some(Err(e))) => panic!("event error: {}", e),
        Ok(None) => {}
    }
}

#[tokio::test]
#[ignore]
async fn live_nixevent_selective_unsubscribe() {
    let (client, mut events) = connect().await;
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
                if evt.header("Event-Subclass") == Some(subclass.as_str()) {
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
    let (client, _events) = connect().await;

    // api with a non-existent command returns -ERR in the body
    let resp = client
        .api("nonexistent_command_xyz")
        .await
        .unwrap();
    let body = resp
        .body()
        .expect("api error should have body");
    assert!(
        body.contains("-ERR") || body.contains("-USAGE"),
        "expected error in body: {}",
        body
    );
}
