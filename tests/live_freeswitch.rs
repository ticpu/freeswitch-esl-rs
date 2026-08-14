//! Integration tests against a live FreeSWITCH instance.
//!
//! These tests require FreeSWITCH ESL on 127.0.0.1:8022 with password ClueCon.
//! Run with: cargo test --test live_freeswitch -- --ignored

use freeswitch_esl_tokio::commands::originate::{OriginateTarget, Variables, VariablesType};
use freeswitch_esl_tokio::commands::DialStringCarrier;
use freeswitch_esl_tokio::commands::{
    LoopbackEndpoint, UuidGetVar, UuidKill, UuidSetVar, UuidTransfer,
};
use freeswitch_esl_tokio::variables::LoopbackVariable;
use freeswitch_esl_tokio::ExecuteOptions;
use freeswitch_esl_tokio::{
    parse_api_body, Application, ChannelState, ConnectionStatus, DialplanType, DisconnectReason,
    Endpoint, EslClient, EslConnectOptions, EslError, EslEvent, EslEventPriority, EslEventType,
    EventFormat, EventHeader, HeaderLookup, LoopbackHangupCause, Originate, ReplyStatus,
    DEFAULT_ESL_PASSWORD,
};
use std::time::Duration;
use tokio::sync::{OnceCell, Semaphore};
use tokio::time::Instant;

const ESL_HOST: &str = "127.0.0.1";
const ESL_PORT: u16 = 8022;
const ESL_PASSWORD: &str = DEFAULT_ESL_PASSWORD;
const MAX_CONCURRENT_CONNECTIONS: usize = 5;
const REQUIRED_SPS: u32 = 1000;

static CONN_SEMAPHORE: Semaphore = Semaphore::const_new(MAX_CONCURRENT_CONNECTIONS);
static SPS_RAISED: OnceCell<()> = OnceCell::const_new();

/// Raise the switch's session admission rate for the whole suite.
///
/// Each loopback originate costs two sessions and the bowout pair costs four,
/// so a parallel run bursts far past a stock `sessions-per-second`. Past it,
/// `switch_core_session_request_uuid` returns NULL and the originate comes
/// back `-ERR DESTINATION_OUT_OF_ORDER` -- surfacing as a random unrelated
/// test failing, a different one each run.
///
/// Raised once and deliberately left raised: a parallel suite has no reliable
/// last-test-finished hook to restore it from, and a half-restored throttle
/// would reintroduce exactly the flakiness this removes.
async fn raise_session_throttle(client: &EslClient) {
    SPS_RAISED
        .get_or_init(|| async {
            let resp = client
                .api(&format!("fsctl sps {}", REQUIRED_SPS))
                .await
                .expect("fsctl sps: transport error");
            resp.api_result()
                .expect("fsctl sps rejected -- the ESL user needs it in esl-allowed-api");
        })
        .await;
}

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
    raise_session_throttle(&client).await;
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
    let uuid = resp
        .api_result()
        .expect("originate failed")
        .to_string();

    // `park` holds the channel open indefinitely, so nothing reaps it for us.
    let mut reaper = Reaper::new(&client);
    reaper.track(&uuid);

    // Wait for CHANNEL_CREATE with our UUID
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut created_event = None;
    while Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.event_type() == Some(EslEventType::ChannelCreate)
                    && evt.unique_id() == Some(&uuid)
                {
                    created_event = Some(evt);
                    break;
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("event stream closed"),
            Err(_) => break,
        }
    }

    reaper
        .reap()
        .await;

    let evt = created_event
        .unwrap_or_else(|| panic!("did not receive CHANNEL_CREATE with timetable for {}", uuid));
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
    if let Err(e) = client
        .api(&cmd.to_string())
        .await
    {
        eprintln!("cleanup: uuid_kill {} failed: {}", uuid, e);
    }
}

/// The channels a test created, so it can kill them before it asserts.
///
/// Cleanup has to run *before* the assertions. A panic between creating a
/// channel and killing it strands that channel for the rest of the run, and
/// stranded channels burn the switch's session budget until later originates
/// start failing with `-ERR DESTINATION_OUT_OF_ORDER` -- which surfaces as some
/// unrelated test failing, not this one.
struct Reaper<'a> {
    client: &'a EslClient,
    uuids: Vec<String>,
}

impl<'a> Reaper<'a> {
    fn new(client: &'a EslClient) -> Self {
        Self {
            client,
            uuids: Vec::new(),
        }
    }

    /// Register a channel to kill. Repeats are ignored, so a uuid learned from
    /// several events is still killed once.
    fn track(&mut self, uuid: impl Into<String>) {
        let uuid = uuid.into();
        if !self
            .uuids
            .contains(&uuid)
        {
            self.uuids
                .push(uuid);
        }
    }

    async fn reap(&mut self) {
        for uuid in self
            .uuids
            .drain(..)
        {
            kill_channel(self.client, &uuid).await;
        }
    }
}

/// Whether the switch still has this channel.
///
/// Cleanup swallows "already gone", so a test that needs to prove a channel
/// died has to ask before reaping.
async fn channel_exists(client: &EslClient, uuid: &str) -> bool {
    let resp = client
        .api(&format!("uuid_exists {}", uuid))
        .await
        .unwrap_or_else(|e| panic!("uuid_exists {}: transport error: {}", uuid, e));
    match resp.api_result() {
        Ok("true") => true,
        Ok("false") => false,
        Ok(other) => panic!("uuid_exists {}: unexpected reply {:?}", uuid, other),
        Err(e) => panic!("uuid_exists {}: {}", uuid, e),
    }
}

// --- L12: YAML-configured loopback originate (see docs/originate-loopback-yaml.md) ---

/// The same YAML the example and the docs use, so a drift in any of the three
/// breaks the build rather than only the prose.
const LOOPBACK_YAML: &str = include_str!("../examples/originate_loopback.yaml");
const LOOPBACK_BOWOUT_YAML: &str = include_str!("../examples/originate_loopback_bowout.yaml");
const LOOPBACK_SCOPED_YAML: &str = include_str!("../examples/originate_loopback_scoped_vars.yaml");

/// Channel variables the YAML sets. FreeSWITCH must expose each on *both*
/// loopback legs: `switch_ivr_originate` applies the originate variable block
/// to the A leg, and mod_loopback replays it onto the B leg.
const LOOPBACK_YAML_VARS: &[(&str, &str)] = &[
    ("customer_id", "CUST-42"),
    ("tenant", "acme"),
    // On the wire this is `'T-1001\, urgent'`: comma escaped, whole value
    // quoted for the space. FreeSWITCH unescapes both.
    ("sip_h_X-Ticket", "T-1001, urgent"),
];

fn loopback_yaml_originate() -> Originate {
    yaml_serde::from_str(LOOPBACK_YAML).expect("examples/originate_loopback.yaml must deserialize")
}

/// Read a channel variable, mapping "not set" to `None`.
///
/// `uuid_getvar` writes the literal `_undef_` when the variable is unset, so an
/// absent variable arrives as a successful reply rather than an error. Only
/// that is `None`: a dead channel answers `-ERR no such channel`, and folding
/// that into `None` too would let an assertion expecting an unset variable pass
/// against a channel that had already gone away.
async fn getvar(client: &EslClient, uuid: &str, name: &str) -> Option<String> {
    let cmd = UuidGetVar::new(uuid, name);
    let resp = client
        .api(&cmd.to_string())
        .await
        .unwrap_or_else(|e| panic!("uuid_getvar {} {}: transport error: {}", uuid, name, e));
    match resp.api_result() {
        Ok("_undef_") => None,
        Ok(value) => Some(value.to_string()),
        Err(e) => panic!("uuid_getvar {} {}: {}", uuid, name, e),
    }
}

#[test]
fn yaml_loopback_originate_parses() {
    let cmd = loopback_yaml_originate();

    let Endpoint::Loopback(ref ep) = *cmd.endpoint() else {
        panic!("expected a loopback endpoint, got {:?}", cmd.endpoint());
    };
    assert_eq!(ep.extension, "9199");
    assert_eq!(
        ep.context
            .as_deref(),
        Some("test")
    );

    let vars = ep
        .variables
        .as_ref()
        .expect("endpoint must carry variables");
    // `scope: channel` in YAML -> [] brackets on the wire.
    assert_eq!(vars.scope(), VariablesType::Channel);
    for (name, value) in LOOPBACK_YAML_VARS {
        assert_eq!(vars.get(name), Some(*value), "variable {}", name);
    }
    assert_eq!(vars.get("origination_caller_id_name"), Some("Sales Desk"));

    assert!(matches!(
        cmd.target(),
        OriginateTarget::Application(app) if app.name() == "park" && app.args().is_none()
    ));
    assert_eq!(cmd.dialplan_type(), Some(&DialplanType::Xml));
    assert_eq!(cmd.context_str(), Some("test"));
    assert_eq!(cmd.caller_id_name(), Some("Fallback CID"));
    assert_eq!(cmd.caller_id_number(), Some("5550199"));
    assert_eq!(cmd.timeout_seconds(), Some(30));

    assert_eq!(
        cmd.to_string(),
        "originate [origination_caller_id_name='Sales Desk',\
origination_caller_id_number=5550100,ignore_early_media=true,customer_id=CUST-42,\
tenant=acme,sip_h_X-Ticket='T-1001\\, urgent']loopback/9199/test \
&park() XML test 'Fallback CID' 5550199 30"
    );
}

#[test]
fn yaml_loopback_bowout_parses() {
    let cmd: Originate = yaml_serde::from_str(LOOPBACK_BOWOUT_YAML)
        .expect("examples/originate_loopback_bowout.yaml must deserialize");

    let Endpoint::Loopback(ref ep) = *cmd.endpoint() else {
        panic!("expected a loopback endpoint, got {:?}", cmd.endpoint());
    };
    // mod_loopback's `app=<application>[:<args>]` destination form.
    assert_eq!(ep.extension, "app=bridge:null/farend");
    assert!(ep
        .context
        .is_none());

    // A bare YAML mapping (no scope/vars wrapper) means Default scope -> {}.
    let vars = ep
        .variables
        .as_ref()
        .expect("endpoint must carry variables");
    assert_eq!(vars.scope(), VariablesType::Default);
    assert_eq!(vars.get("loopback_bowout"), Some("true"));

    assert!(matches!(
        cmd.target(),
        OriginateTarget::Application(app)
            if app.name() == "bridge" && app.args() == Some("null/nearend")
    ));

    assert_eq!(
        cmd.to_string(),
        "originate {loopback_bowout=true}loopback/app=bridge:null/farend &bridge(null/nearend)"
    );
}

/// Originate the YAML command against FreeSWITCH and confirm the pair really
/// comes up: both legs are created and answered, and every variable from the
/// originate block is readable on each leg.
#[tokio::test]
#[ignore]
async fn live_originate_loopback_from_yaml() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(
            EventFormat::Plain,
            &[EslEventType::ChannelCreate, EslEventType::ChannelAnswer],
        )
        .await
        .unwrap();

    // api originate blocks until the A leg answers, which happens once the B
    // leg's dialplan (9199 -> answer) answers. Events queue while it blocks.
    let cmd = loopback_yaml_originate();
    let resp = client
        .api(&cmd.to_string())
        .await
        .expect("originate transport error");
    let a_leg = resp
        .api_result()
        .expect("originate returned an error")
        .to_string();

    // mod_loopback cross-links the two legs with this variable.
    let b_leg = getvar(&client, &a_leg, "other_loopback_leg_uuid")
        .await
        .expect("A leg must expose other_loopback_leg_uuid");
    assert_ne!(a_leg, b_leg, "the two loopback legs must be distinct");

    assert_eq!(
        getvar(&client, &a_leg, "loopback_leg")
            .await
            .as_deref(),
        Some("A")
    );
    assert_eq!(
        getvar(&client, &b_leg, "loopback_leg")
            .await
            .as_deref(),
        Some("B")
    );

    // Every originate variable must be visible on both legs.
    for (name, expected) in LOOPBACK_YAML_VARS {
        for (leg, uuid) in [("A", &a_leg), ("B", &b_leg)] {
            assert_eq!(
                getvar(&client, uuid, name)
                    .await
                    .as_deref(),
                Some(*expected),
                "{} leg is missing variable {}",
                leg,
                name
            );
        }
    }

    // The B leg is the one the dialplan runs on, so it carries the caller ID.
    // `origination_caller_id_*` wins over the positional cid_name/cid_num,
    // which the YAML deliberately sets to different values.
    assert_eq!(
        getvar(&client, &b_leg, "caller_id_name")
            .await
            .as_deref(),
        Some("Sales Desk")
    );
    assert_eq!(
        getvar(&client, &b_leg, "caller_id_number")
            .await
            .as_deref(),
        Some("5550100")
    );

    // Both legs must have been created and answered.
    let mut created = std::collections::HashSet::new();
    let mut answered = std::collections::HashSet::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while (created.len() < 2 || answered.len() < 2) && Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                let Some(uuid) = evt.unique_id() else {
                    continue;
                };
                if uuid != a_leg && uuid != b_leg {
                    continue;
                }
                match evt.event_type() {
                    Some(EslEventType::ChannelCreate) => {
                        created.insert(uuid.to_string());
                    }
                    Some(EslEventType::ChannelAnswer) => {
                        answered.insert(uuid.to_string());
                    }
                    _ => {}
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("event stream closed"),
            Err(_) => break,
        }
    }

    kill_channel(&client, &a_leg).await;

    assert_eq!(created.len(), 2, "expected CHANNEL_CREATE for both legs");
    assert_eq!(answered.len(), 2, "expected CHANNEL_ANSWER for both legs");
}

#[test]
fn yaml_loopback_scoped_vars_parses() {
    let cmd: Originate = yaml_serde::from_str(LOOPBACK_SCOPED_YAML)
        .expect("examples/originate_loopback_scoped_vars.yaml must deserialize");
    assert_eq!(
        cmd.to_string(),
        "originate {leg_a_only=outer}loopback/9199/test &bridge({leg_b_only=inner}null/far)"
    );
}

/// A variable set in the originate's own block reaches the loopback pair and
/// stops there; a variable set in the bridge's dial string reaches only the
/// bridged leg. This is the only way to give the two sides of a call
/// different variables, since neither `{}` nor `[]` can address one loopback
/// leg on its own.
#[tokio::test]
#[ignore]
async fn live_originate_loopback_nested_bridge_scopes_vars() {
    let (client, _events, _permit) = connect().await;

    let cmd: Originate = yaml_serde::from_str(LOOPBACK_SCOPED_YAML).unwrap();
    let resp = client
        .api(&cmd.to_string())
        .await
        .expect("originate transport error");
    let a_leg = resp
        .api_result()
        .expect("originate returned an error")
        .to_string();

    let b_leg = getvar(&client, &a_leg, "other_loopback_leg_uuid")
        .await
        .expect("A leg must expose other_loopback_leg_uuid");

    // The A leg runs &bridge(...), so the far channel shows up as its bridge
    // partner once the bridge is established.
    let mut far = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(uuid) = getvar(&client, &a_leg, "bridge_uuid").await {
            far = Some(uuid);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let far = far.expect("A leg never bridged to null/far");

    // The originate block reaches both loopback legs...
    for (leg, uuid) in [("A", &a_leg), ("B", &b_leg)] {
        assert_eq!(
            getvar(&client, uuid, "leg_a_only")
                .await
                .as_deref(),
            Some("outer"),
            "{} leg should carry the originate block variable",
            leg
        );
    }
    // ...but does not cross the bridge into the far leg.
    assert_eq!(
        getvar(&client, &far, "leg_a_only").await,
        None,
        "originate block variables must not leak across the bridge"
    );

    // The bridge dial string reaches only the leg it dials.
    assert_eq!(
        getvar(&client, &far, "leg_b_only")
            .await
            .as_deref(),
        Some("inner")
    );
    for (leg, uuid) in [("A", &a_leg), ("B", &b_leg)] {
        assert_eq!(
            getvar(&client, uuid, "leg_b_only").await,
            None,
            "{} leg must not see the bridge dial string variable",
            leg
        );
    }

    kill_channel(&client, &a_leg).await;
    kill_channel(&client, &far).await;
}

/// Drive a loopback pair through a bowout and confirm mod_loopback removed
/// itself: both loopback legs resign, and the two real channels end up
/// bridged straight to each other.
#[tokio::test]
#[ignore]
async fn live_originate_loopback_bowout_from_yaml() {
    let (client, mut events, _permit) = connect().await;

    // Subscribe first: the pair can bow out within milliseconds of the A leg's
    // bridge starting, and those hangups are the evidence.
    client
        .subscribe_events(
            EventFormat::Plain,
            &[
                EslEventType::ChannelBridge,
                EslEventType::ChannelHangupComplete,
            ],
        )
        .await
        .unwrap();

    let cmd: Originate = yaml_serde::from_str(LOOPBACK_BOWOUT_YAML).unwrap();
    let resp = client
        .api(&cmd.to_string())
        .await
        .expect("originate transport error");
    let a_leg = resp
        .api_result()
        .expect("originate returned an error")
        .to_string();

    let mut resigned: Vec<(String, String)> = Vec::new();
    let mut legs: Vec<String> = Vec::new();
    // uuid_bridge fires before mod_loopback stamps the legs, so the splice can
    // arrive before the survivors are known. Keep every real-to-real bridge and
    // pick ours out once the resignations name them.
    let mut real_bridges: Vec<(String, String)> = Vec::new();

    // Our splice is the real-to-real bridge whose two ends are exactly the
    // channels our resignations handed over to. The bridge event can arrive on
    // either side of the hangups, so both have to be in hand before deciding.
    let spliced = |resigned: &[(String, String)], bridges: &[(String, String)]| {
        let survivors: std::collections::HashSet<&str> = resigned
            .iter()
            .map(|(_, s)| s.as_str())
            .collect();
        survivors.len() == 2
            && bridges
                .iter()
                .any(|(a, b)| survivors.contains(a.as_str()) && survivors.contains(b.as_str()))
    };

    let deadline = Instant::now() + Duration::from_secs(15);
    while !spliced(&resigned, &real_bridges) && Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => match evt.event_type() {
                Some(EslEventType::ChannelHangupComplete) => {
                    // Both legs carry the partner's uuid, so either one ties
                    // back to the leg this test originated. Without that, a
                    // foreign bowout on the shared switch lands here too.
                    let ours = evt.unique_id() == Some(a_leg.as_str())
                        || evt.variable(LoopbackVariable::OtherLoopbackLegUuid)
                            == Some(a_leg.as_str());
                    if !ours {
                        continue;
                    }
                    // mod_loopback stamps this on both legs right before it
                    // bridges the real channels together.
                    let Some(r) = evt.loopback_resignation() else {
                        continue;
                    };
                    // This YAML drives the frame-count path specifically.
                    assert_eq!(r.cause(), Ok(LoopbackHangupCause::Bridge));
                    let survivor = r
                        .other_uuid()
                        .expect("a resigning leg must name the real channel it hands over to");
                    if let Some(uuid) = evt.unique_id() {
                        legs.push(uuid.to_string());
                    }
                    resigned.push((
                        evt.header(EventHeader::ChannelName)
                            .unwrap_or_default()
                            .to_string(),
                        survivor.to_string(),
                    ));
                }
                Some(EslEventType::ChannelBridge) => {
                    // Three bridges occur: loopback-a=null/nearend,
                    // loopback-b=null/farend, then the bowout's uuid_bridge.
                    // Only the last has a real channel on both sides.
                    let (Some(this), Some(other)) = (
                        evt.header(EventHeader::ChannelName),
                        evt.header(EventHeader::OtherLegChannelName),
                    ) else {
                        continue;
                    };
                    if this.starts_with("loopback/") || other.starts_with("loopback/") {
                        continue;
                    }
                    if let (Some(a), Some(b)) =
                        (evt.unique_id(), evt.header(EventHeader::OtherLegUniqueId))
                    {
                        real_bridges.push((a.to_string(), b.to_string()));
                    }
                }
                _ => {}
            },
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("event stream closed"),
            Err(_) => break,
        }
    }

    // A resigned leg hung itself up, so it must already be gone -- one that is
    // still around means mod_loopback left the audio path without tearing the
    // leg down, which would strand a channel per bowout on a live switch.
    // Checked before the reap below, which would otherwise hide it.
    let mut zombies = Vec::new();
    for leg in &legs {
        if channel_exists(&client, leg).await {
            zombies.push(leg.clone());
        }
    }

    // The survivors are a live call by design, so nothing else will end them.
    let mut reaper = Reaper::new(&client);
    reaper.track(&a_leg);
    for (_, survivor) in &resigned {
        reaper.track(survivor);
    }
    reaper
        .reap()
        .await;

    assert!(
        zombies.is_empty(),
        "resigned loopback legs must be gone, still present: {:?}",
        zombies
    );
    assert_eq!(
        resigned.len(),
        2,
        "both loopback legs must resign, saw {:?}",
        resigned
    );
    assert!(
        spliced(&resigned, &real_bridges),
        "the two real channels this test created must end up bridged to each other; \
         resigned {:?}, real bridges seen {:?}",
        resigned,
        real_bridges
    );
}

/// mod_loopback's other bowout trigger, which reports a different token.
///
/// `loopback_bowout_on_execute` resigns the leg as soon as it executes an
/// application, masquerading its extension onto the real channel behind its
/// partner instead of waiting for audio to flow. `loopback_bowout=false`
/// vetoes the frame-count path so only this one can fire.
///
/// This is the token a consumer bug matched against, so it earns live coverage
/// rather than a synthesized header map: the two paths must stay
/// indistinguishable to `loopback_resignation()` and distinguishable to
/// `cause()`.
///
/// Setting the trigger in the originate would be a race, not a test.
/// mod_loopback bows out only if the partner leg already carries a signal bond
/// to a non-loopback channel when this leg executes, and does nothing at all
/// when it does not — `switch_ivr_multi_threaded_bridge` writes that bond
/// *after* it fires `CHANNEL_BRIDGE`, while `originate` returns as soon as the
/// leg answers. So park the leg, wait for the far channel to reach
/// `CS_EXCHANGE_MEDIA` (which the switch sets only after writing the bond),
/// then arm the trigger and transfer. Every step is ordered by the switch.
#[tokio::test]
#[ignore]
async fn live_originate_loopback_bowout_on_execute() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(
            EventFormat::Plain,
            &[
                EslEventType::ChannelState,
                EslEventType::ChannelHangupComplete,
            ],
        )
        .await
        .unwrap();

    // No trigger yet: parking means this leg's first execute pass cannot race.
    let mut vars = Variables::new(VariablesType::Default);
    vars.insert("loopback_bowout", "false");
    let cmd = Originate::application(
        Endpoint::Loopback(LoopbackEndpoint::new("app=bridge:null/farend").with_variables(vars)),
        Application::park(),
    );

    let resp = client
        .api(&cmd.to_string())
        .await
        .expect("originate transport error");
    let a_leg = resp
        .api_result()
        .expect("originate returned an error")
        .to_string();

    let mut reaper = Reaper::new(&client);
    reaper.track(&a_leg);

    // The leg is parked and alive, so this is safe to ask for.
    let b_leg = getvar(&client, &a_leg, "other_loopback_leg_uuid")
        .await
        .expect("a loopback leg must name its partner");
    reaper.track(&b_leg);

    // The far channel bonded to the partner is the one whose CS_EXCHANGE_MEDIA
    // proves the bond exists; keying on the bond value keeps a concurrent
    // test's identical topology out of it.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut bonded = false;
    while !bonded && Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.event_type() != Some(EslEventType::ChannelState) {
                    continue;
                }
                // CHANNEL_STATE carries no channel variables, so the bond
                // itself is not observable here -- but the far channel names
                // the partner leg it was originated by, and the switch only
                // moves it to CS_EXCHANGE_MEDIA after writing that bond.
                if evt.header(EventHeader::OtherLegUniqueId) != Some(b_leg.as_str()) {
                    continue;
                }
                if evt.channel_state() == Ok(Some(ChannelState::CsExchangeMedia)) {
                    bonded = true;
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("event stream closed"),
            Err(_) => break,
        }
    }

    if bonded {
        client
            .api(&UuidSetVar::new(&a_leg, "loopback_bowout_on_execute", "true").to_string())
            .await
            .expect("uuid_setvar transport error")
            .api_result()
            .expect("uuid_setvar rejected");

        // Re-entering CS_EXECUTE runs channel_on_execute again, this time with
        // the trigger armed and the partner's bond already in place.
        client
            .api(
                &UuidTransfer::new(&a_leg, "bridge:null/nearend")
                    .with_dialplan(DialplanType::Inline)
                    .to_string(),
            )
            .await
            .expect("uuid_transfer transport error")
            .api_result()
            .expect("uuid_transfer rejected");
    }

    let mut resignation: Option<(String, Option<String>)> = None;
    while bonded && resignation.is_none() && Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.event_type() != Some(EslEventType::ChannelHangupComplete) {
                    continue;
                }
                let ours = evt.unique_id() == Some(a_leg.as_str())
                    || evt.variable(LoopbackVariable::OtherLoopbackLegUuid) == Some(a_leg.as_str());
                if !ours {
                    continue;
                }
                if let Some(r) = evt.loopback_resignation() {
                    resignation = Some((
                        r.cause_raw()
                            .to_string(),
                        r.other_uuid()
                            .map(str::to_string),
                    ));
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("event stream closed"),
            Err(_) => break,
        }
    }

    if let Some((_, Some(ref survivor))) = resignation {
        reaper.track(survivor);
    }
    reaper
        .reap()
        .await;

    assert!(
        bonded,
        "the partner leg never bonded to a real channel, so the execute path \
         could not be reached"
    );
    let (cause_raw, other_uuid) =
        resignation.expect("the execute path must report a resignation on the leg that bowed out");
    assert_eq!(
        cause_raw.parse::<LoopbackHangupCause>(),
        Ok(LoopbackHangupCause::Bowout),
        "the execute path writes its own token, got {:?}",
        cause_raw
    );
    assert!(
        other_uuid.is_some(),
        "a resigning leg must name the real channel it hands over to"
    );
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

    // Filter before subscribing: the switch is shared, so any window between the
    // two queues a parallel test's BACKGROUND_JOB on this listener.
    client
        .filter(EventHeader::EventName, "HEARTBEAT")
        .await
        .unwrap();

    client
        .subscribe_events(
            EventFormat::Plain,
            &[EslEventType::Heartbeat, EslEventType::BackgroundJob],
        )
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

    let mut reaper = Reaper::new(&client);
    reaper.track(&uuid);

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut created_event = None;

    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                // Other tests originate against the same switch, so match our
                // own channel rather than the first CHANNEL_CREATE to arrive.
                if evt.event_type() != Some(EslEventType::ChannelCreate)
                    || evt.unique_id() != Some(&uuid)
                {
                    continue;
                }
                created_event = Some(evt);
                break;
            }
            Ok(Some(Err(e))) => panic!("event error: {e}"),
            Ok(None) => panic!("connection closed before CHANNEL_CREATE"),
            Err(_) => break,
        }
    }

    reaper
        .reap()
        .await;

    let evt = created_event.expect("never received CHANNEL_CREATE event");

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

    let mut reaper = Reaper::new(&client);
    reaper.track(&uuid);

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut codec_event = None;

    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                // Other tests originate against the same switch, so match our
                // own channel rather than the first CODEC event to arrive.
                if evt.event_type() != Some(EslEventType::Codec) || evt.unique_id() != Some(&uuid) {
                    continue;
                }
                codec_event = Some(evt);
                break;
            }
            Ok(Some(Err(e))) => panic!("event error: {e}"),
            Ok(None) => panic!("connection closed before CODEC event"),
            Err(_) => break,
        }
    }

    reaper
        .reap()
        .await;

    let evt = codec_event.expect("never received CODEC event");

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

    let (inbound, mut events, permit) = connect().await;

    // The originate's outcome is the only thing that explains a listener that
    // never gets dialed, so watch for it rather than inferring from a timeout.
    inbound
        .subscribe_events(EventFormat::Plain, &[EslEventType::BackgroundJob])
        .await
        .expect("subscribe BACKGROUND_JOB");

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
    let job = inbound
        .bgapi(&cmd.to_string())
        .await
        .expect("bgapi originate")
        .job_uuid()
        .expect("bgapi must return a Job-UUID")
        .to_string();

    // Race the accept against the job result. A failed originate means FS never
    // dials, so without this the only symptom is an opaque accept timeout that
    // says nothing about why.
    let accept = tokio::time::timeout(
        Duration::from_secs(10),
        EslClient::accept_outbound(&listener),
    );
    tokio::pin!(accept);

    let (outbound, _outbound_events) = loop {
        tokio::select! {
            accepted = &mut accept => {
                break accepted
                    .expect("timed out waiting for outbound connection")
                    .expect("accept_outbound failed");
            }
            event = events.recv() => match event {
                Some(Ok(evt)) => {
                    if evt.event_type() == Some(EslEventType::BackgroundJob)
                        && evt.job_uuid() == Some(job.as_str())
                    {
                        if let Some(body) = evt.body() {
                            if let Err(e) = parse_api_body(body) {
                                panic!("originate failed, so FS never dialed us: {}", e);
                            }
                        }
                    }
                }
                Some(Err(e)) => panic!("event error: {}", e),
                None => panic!("event stream closed"),
            },
        }
    };

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

// --- Dial-string escaping, per carrier ---

/// Values whose escaping is not obvious, each paired with a sentinel so a value
/// that eats its separator shows up as damage to a *later* variable.
///
/// Two quoted values, never one: a block carrying a single quote has no partner
/// for it to pair with and passes under encodings that corrupt a realistic
/// block. The empty value is absent because no dial string can express it —
/// `Variables` refuses it at the boundaries that can.
const ESCAPING_CASES: &[(&str, &[(&str, &str)])] = &[
    ("plain comma", &[("p1", "a,b"), ("p2", "SENTINEL")]),
    (
        "comma and space",
        &[("p1", "T-1001, urgent"), ("p2", "SENTINEL")],
    ),
    (
        "two quoted values",
        &[("p1", "it's"), ("p2", "don't"), ("p3", "SENTINEL")],
    ),
    (
        "backslash before an inert character",
        &[("p1", r"C:\path"), ("p2", "SENTINEL")],
    ),
    (
        "backslash before one the switch reads as an escape",
        &[("p1", r"a\nb"), ("p2", "SENTINEL")],
    ),
];

fn escaping_block(pairs: &[(&str, &str)]) -> Variables {
    let mut vars = Variables::new(VariablesType::Default);
    for (k, v) in pairs {
        vars.insert(*k, *v);
    }
    vars
}

/// Appears in none of [`ESCAPING_CASES`], which is what `with_separator`
/// demands and what keeps a comma ordinary text inside the block.
const ESCAPING_SEPARATOR: char = '~';

/// The `originate` API splits its argument list before the block is parsed, so
/// this carrier needs one escaping level more than a dialplan application.
#[tokio::test]
#[ignore]
async fn live_escaping_survives_the_api_carrier() {
    let (client, _events, permit) = connect().await;

    for (label, pairs) in ESCAPING_CASES {
        let vars = escaping_block(pairs);
        let resp = client
            .api(&format!("originate {vars}null/escaping &park()"))
            .await
            .unwrap_or_else(|e| panic!("{label}: originate transport error: {e}"));
        let uuid = resp
            .api_result()
            .unwrap_or_else(|e| panic!("{label}: originate rejected {vars}: {e}"))
            .to_string();

        let mut reaper = Reaper::new(&client);
        reaper.track(&uuid);
        let mut results = Vec::new();
        for (key, want) in *pairs {
            results.push((*key, *want, getvar(&client, &uuid, key).await));
        }
        reaper
            .reap()
            .await;

        for (key, want, got) in results {
            assert_eq!(got.as_deref(), Some(want), "{label}: {key} arrived wrong");
        }
    }

    drop(permit);
}

/// A dialplan application receives its argument whole, so the same values need
/// one level less. Rendering with the API default here would corrupt a quoted
/// value, which is the whole reason the carrier is nameable.
#[tokio::test]
#[ignore]
async fn live_escaping_survives_the_dialplan_carrier() {
    let (client, _events, permit) = connect().await;

    for (label, pairs) in ESCAPING_CASES {
        let b_uuid = client
            .api("create_uuid")
            .await
            .expect("create_uuid transport error")
            .api_result()
            .expect("create_uuid failed")
            .to_string();
        let a_uuid = client
            .api("originate null/anchor &park()")
            .await
            .expect("anchor transport error")
            .api_result()
            .expect("anchor originate failed")
            .to_string();

        // Pre-assigning the B leg's uuid is what makes the far side findable;
        // it leads the block so a later value cannot take it with it.
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("origination_uuid", &b_uuid);
        for (k, v) in *pairs {
            vars.insert(*k, *v);
        }

        let mut reaper = Reaper::new(&client);
        reaper.track(&a_uuid);
        reaper.track(&b_uuid);

        client
            .execute_with_options(
                "bridge",
                Some(&format!(
                    "{}null/escaping",
                    vars.display_for(DialStringCarrier::Dialplan)
                )),
                Some(&a_uuid),
                ExecuteOptions::new().with_async(),
            )
            .await
            .unwrap_or_else(|e| panic!("{label}: execute bridge failed: {e}"));

        // The bridge is async, so the far leg appears a moment later.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mut results = Vec::new();
        for (key, want) in *pairs {
            results.push((*key, *want, getvar(&client, &b_uuid, key).await));
        }
        reaper
            .reap()
            .await;

        for (key, want, got) in results {
            assert_eq!(got.as_deref(), Some(want), "{label}: {key} arrived wrong");
        }
    }

    drop(permit);
}

/// A separator the values do not contain is the only way a `${...}`-expanded
/// value carries a comma, since substitution happens before the block is parsed.
#[tokio::test]
#[ignore]
async fn live_chosen_separator_carries_commas_unescaped() {
    let (client, _events, permit) = connect().await;

    let mut vars = Variables::new(VariablesType::Default);
    vars.insert("codecs", "PCMA,PCMU,G729");
    vars.insert("tenant", "acme");
    let vars = vars
        .with_separator(':')
        .expect("':' does not appear in any value here");

    let uuid = client
        .api(&format!("originate {vars}null/escaping &park()"))
        .await
        .expect("originate transport error")
        .api_result()
        .unwrap_or_else(|e| panic!("originate rejected {vars}: {e}"))
        .to_string();

    let mut reaper = Reaper::new(&client);
    reaper.track(&uuid);
    let codecs = getvar(&client, &uuid, "codecs").await;
    let tenant = getvar(&client, &uuid, "tenant").await;
    reaper
        .reap()
        .await;

    assert_eq!(codecs.as_deref(), Some("PCMA,PCMU,G729"));
    assert_eq!(tenant.as_deref(), Some("acme"));

    drop(permit);
}

/// A `^^` block reaches the switch's tokenizer the same number of times as the
/// comma form, so every value that needed escaping there still needs it here.
/// Only the comma stops being special.
#[tokio::test]
#[ignore]
async fn live_separated_escaping_survives_the_api_carrier() {
    let (client, _events, permit) = connect().await;

    for (label, pairs) in ESCAPING_CASES {
        let vars = escaping_block(pairs)
            .with_separator(ESCAPING_SEPARATOR)
            .unwrap_or_else(|e| panic!("{label}: {ESCAPING_SEPARATOR:?} rejected: {e}"));
        let resp = client
            .api(&format!("originate {vars}null/escaping &park()"))
            .await
            .unwrap_or_else(|e| panic!("{label}: originate transport error: {e}"));
        let uuid = resp
            .api_result()
            .unwrap_or_else(|e| panic!("{label}: originate rejected {vars}: {e}"))
            .to_string();

        let mut reaper = Reaper::new(&client);
        reaper.track(&uuid);
        let mut results = Vec::new();
        for (key, want) in *pairs {
            results.push((*key, *want, getvar(&client, &uuid, key).await));
        }
        reaper
            .reap()
            .await;

        for (key, want, got) in results {
            assert_eq!(
                got.as_deref(),
                Some(want),
                "{label}: {key} arrived wrong from {vars}"
            );
        }
    }

    drop(permit);
}

/// The dialplan half of the pair above: one escaping level less, same block
/// shape, so a rule that only holds on one carrier shows up as a difference
/// between these two tests rather than as a passing suite.
#[tokio::test]
#[ignore]
async fn live_separated_escaping_survives_the_dialplan_carrier() {
    let (client, _events, permit) = connect().await;

    for (label, pairs) in ESCAPING_CASES {
        let b_uuid = client
            .api("create_uuid")
            .await
            .expect("create_uuid transport error")
            .api_result()
            .expect("create_uuid failed")
            .to_string();
        let a_uuid = client
            .api("originate null/anchor &park()")
            .await
            .expect("anchor transport error")
            .api_result()
            .expect("anchor originate failed")
            .to_string();

        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("origination_uuid", &b_uuid);
        for (k, v) in *pairs {
            vars.insert(*k, *v);
        }
        let vars = vars
            .with_separator(ESCAPING_SEPARATOR)
            .unwrap_or_else(|e| panic!("{label}: {ESCAPING_SEPARATOR:?} rejected: {e}"));

        let mut reaper = Reaper::new(&client);
        reaper.track(&a_uuid);
        reaper.track(&b_uuid);

        let dial_string = format!(
            "{}null/escaping",
            vars.display_for(DialStringCarrier::Dialplan)
        );
        client
            .execute_with_options(
                "bridge",
                Some(&dial_string),
                Some(&a_uuid),
                ExecuteOptions::new().with_async(),
            )
            .await
            .unwrap_or_else(|e| panic!("{label}: execute bridge failed: {e}"));

        tokio::time::sleep(Duration::from_millis(500)).await;
        let mut results = Vec::new();
        for (key, want) in *pairs {
            results.push((*key, *want, getvar(&client, &b_uuid, key).await));
        }
        reaper
            .reap()
            .await;

        for (key, want, got) in results {
            assert_eq!(
                got.as_deref(),
                Some(want),
                "{label}: {key} arrived wrong from {dial_string}"
            );
        }
    }

    drop(permit);
}

/// The refusal in `Variables` rests on this: an empty value never reaches the
/// channel. The block is written by hand because the typed API will not build
/// one, and the point is to check the switch rather than the crate.
///
/// Failing here is good news — it would mean FreeSWITCH started honouring an
/// empty pair, and the refusal should then be revisited rather than kept.
#[tokio::test]
#[ignore]
async fn live_empty_value_still_never_reaches_the_channel() {
    let (client, _events, permit) = connect().await;

    let uuid = client
        .api("originate {p1=,p2=SENTINEL}null/escaping &park()")
        .await
        .expect("originate transport error")
        .api_result()
        .expect("originate rejected")
        .to_string();

    let mut reaper = Reaper::new(&client);
    reaper.track(&uuid);
    let empty = getvar(&client, &uuid, "p1").await;
    let sentinel = getvar(&client, &uuid, "p2").await;
    reaper
        .reap()
        .await;

    assert_eq!(
        empty, None,
        "the switch now keeps an empty pair; Variables refuses to build one, so \
         that refusal is no longer justified"
    );
    assert_eq!(
        sentinel.as_deref(),
        Some("SENTINEL"),
        "the block itself must be well-formed, or the assertion above proves nothing"
    );

    drop(permit);
}
