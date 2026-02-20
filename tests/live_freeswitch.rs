//! Integration tests against a live FreeSWITCH instance.
//!
//! These tests require FreeSWITCH ESL on 127.0.0.1:8022 with password ClueCon.
//! Run with: cargo test --test live_freeswitch -- --ignored

use freeswitch_esl_tokio::{EslClient, EslEvent, EslEventPriority, EslEventType, EventFormat};
use std::time::Duration;

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
    event.set_header("Event-Name".into(), "CUSTOM".into());
    event.set_header("Event-Subclass".into(), "esl_test::priority".into());
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
    event.set_header("Event-Name".into(), "CUSTOM".into());
    event.set_header("Event-Subclass".into(), "esl_test::array".into());
    event.push_header("X-Test-Array", "value1");
    event.push_header("X-Test-Array", "value2");
    event.push_header("X-Test-Array", "value3");

    assert_eq!(
        event.header("X-Test-Array"),
        Some(&"ARRAY::value1|:value2|:value3".to_string())
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
    event.set_header("Event-Name".into(), "CUSTOM".into());
    event.set_header("Event-Subclass".into(), subclass.clone());
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
                if evt.header("Event-Subclass") == Some(&subclass) {
                    assert_eq!(evt.header("priority"), Some(&"NORMAL".to_string()));
                    assert_eq!(
                        evt.header("X-Test-Data"),
                        Some(&"ARRAY::hello|:world".to_string())
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
