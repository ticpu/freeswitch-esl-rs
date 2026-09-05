//! Integration tests against a live FreeSWITCH instance: connection
//! lifecycle, liveness, command timeout, userauth, outbound connect-response
//! case preservation, log events, and header/codec normalization.
//!
//! These tests require FreeSWITCH ESL on 127.0.0.1:8022 with password ClueCon.
//! Run with: cargo test --test live_freeswitch -- --ignored

mod live_common;

use freeswitch_esl_tokio::commands::originate::{Variables, VariablesType};
use freeswitch_esl_tokio::commands::LoopbackEndpoint;
use freeswitch_esl_tokio::connection::AuthMethod;
use freeswitch_esl_tokio::{
    parse_api_body, Application, ConnectionStatus, DisconnectReason, Endpoint, EslClient,
    EslConnectOptions, EslError, EslEventType, EventFormat, EventHeader, HeaderLookup, Originate,
};
use live_common::{
    connect, kill_channel, ChannelReaper, CONN_SEMAPHORE, ESL_HOST, ESL_PASSWORD, ESL_PORT,
};
use std::time::Duration;
use tokio::time::Instant;

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
    let (client, _events) = EslClient::connect_with_auth(
        ESL_HOST,
        ESL_PORT,
        AuthMethod::user("many-events@default", ESL_PASSWORD),
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
        .unwrap()
        .api_result()
        .expect("status rejected");

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

                    // Connection drops on return, so a failed nolog changes nothing.
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

    // Connection drops on panic, so a failed nolog changes nothing.
    let _ = client
        .nolog()
        .await;
    panic!("did not receive any log event with EslEventType::Log");
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

    let mut reaper = ChannelReaper::new(&client);
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

    let mut reaper = ChannelReaper::new(&client);
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
