//! Integration tests against a live FreeSWITCH instance: originate targets
//! (application/extension/inline), the channel timetable, uuid_setvar/getvar
//! and uuid_kill round trips, and dial-string escaping per carrier.
//!
//! These tests require FreeSWITCH ESL on 127.0.0.1:8022 with password ClueCon.
//! Run with: cargo test --test live_channel -- --ignored

mod live_common;

use freeswitch_esl_tokio::commands::originate::{Variables, VariablesType};
use freeswitch_esl_tokio::commands::{
    DialStringCarrier, LoopbackEndpoint, UuidGetVar, UuidKill, UuidSetVar,
};
use freeswitch_esl_tokio::ExecuteOptions;
use freeswitch_esl_tokio::{
    parse_channel_dump, Application, ChannelTimetable, CommandFailure, DialplanType, Endpoint,
    EslEventType, EventFormat, EventHeader, HeaderLookup, Originate, TimetableField,
    TimetablePrefix,
};
use live_common::{
    bgapi_originate_ok, channel_exists, connect, getvar, kill_channel, ChannelReaper,
};
use std::collections::HashSet;
use std::time::Duration;
use tokio::time::Instant;

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
    let mut reaper = ChannelReaper::new(&client);
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

        let mut reaper = ChannelReaper::new(&client);
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

        let mut reaper = ChannelReaper::new(&client);
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

    let mut reaper = ChannelReaper::new(&client);
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

        let mut reaper = ChannelReaper::new(&client);
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

        let mut reaper = ChannelReaper::new(&client);
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

    let mut reaper = ChannelReaper::new(&client);
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

// --- Channel dump parsing: the connect-time state rebuild loop ---

/// Headers that identify the channel and so must read the same from a dump as
/// from the event stream; anything time-varying (state, timestamps) will not.
const DUMP_IDENTITY_HEADERS: &[&str] = &[
    "Unique-ID",
    "Channel-Name",
    "Core-UUID",
    "FreeSWITCH-Hostname",
    "Call-Direction",
    "Caller-Destination-Number",
];

#[tokio::test]
#[ignore]
async fn live_channel_dump_rebuild_loop() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(EventFormat::Plain, &[EslEventType::ChannelCreate])
        .await
        .unwrap();

    // Two channels, so the loop below is a loop and not a single dump.
    // Originated over `api` rather than `bgapi`: waiting for a BACKGROUND_JOB
    // drains the event stream, and these channels' CHANNEL_CREATE is what the
    // dump is compared against.
    let mut reaper = ChannelReaper::new(&client);
    let mut uuids = Vec::new();
    for _ in 0..2 {
        let cmd = Originate::application(
            Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("test")),
            Application::simple("park"),
        );
        let uuid = client
            .api(&cmd.to_string())
            .await
            .expect("originate: transport error")
            .api_result()
            .expect("originate rejected")
            .to_string();
        reaper.track(&uuid);
        uuids.push(uuid);
    }

    // Every CHANNEL_CREATE is correlated to one of our own UUIDs; the switch is
    // shared, so anything else on the stream belongs to another test.
    let mut created: Vec<(String, freeswitch_esl_tokio::EslEvent)> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while created.len() < uuids.len() && Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.event_type() != Some(EslEventType::ChannelCreate) {
                    continue;
                }
                if let Some(uuid) = evt
                    .unique_id()
                    .filter(|u| {
                        uuids
                            .iter()
                            .any(|ours| ours == u)
                    })
                    .map(|u| u.to_string())
                {
                    created.push((uuid, evt));
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("event stream closed"),
            Err(_) => break,
        }
    }

    // Collect everything before reaping; the assertions run after.
    let mut dumps = Vec::new();
    for uuid in &uuids {
        let body = client
            .api(&format!("uuid_dump {}", uuid))
            .await
            .expect("uuid_dump: transport error")
            .body()
            .expect("uuid_dump must return a body")
            .to_string();
        let parsed = parse_channel_dump(&body);
        dumps.push((uuid.clone(), parsed));
    }

    reaper
        .reap()
        .await;

    assert_eq!(
        created.len(),
        uuids.len(),
        "did not observe CHANNEL_CREATE for every originated channel"
    );
    for (uuid, parsed) in dumps {
        let dump = parsed.unwrap_or_else(|e| panic!("uuid_dump {} did not parse: {}", uuid, e));

        assert_eq!(
            dump.event_type(),
            Some(EslEventType::ChannelData),
            "a dump is a serialized CHANNEL_DATA event"
        );
        assert_eq!(dump.raw_body(), None, "a dump arrives already decoded");

        let event = &created
            .iter()
            .find(|(u, _)| *u == uuid)
            .expect("every uuid has a CHANNEL_CREATE")
            .1;
        for name in DUMP_IDENTITY_HEADERS {
            assert_eq!(
                dump.header_str(name),
                event.header_str(name),
                "{} disagrees between the dump and the event for {}",
                name,
                uuid
            );
        }

        // Channel variables reach through the one normalised convention, which
        // is what an inline splitter over the same body loses.
        assert_eq!(
            dump.variable_str("uuid"),
            Some(uuid.as_str()),
            "the dump's channel variables must resolve through variable_str"
        );

        // An unset variable reads as absent. A channel this test can build
        // carries no empty value -- the switch deletes a variable set to one --
        // so the skip itself is exercised by the fixture in
        // src/command/response.rs; here the sentinel must simply never survive.
        assert!(
            dump.headers()
                .values()
                .all(|v| v != "_undef_"),
            "an empty value must read as absent, not as the sentinel, for {}",
            uuid
        );
    }
}

/// The race the rebuild loop hits in production: the channel hung up before
/// its dump.
#[tokio::test]
#[ignore]
async fn live_channel_dump_of_reaped_uuid_is_skippable() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(EventFormat::Plain, &[EslEventType::BackgroundJob])
        .await
        .unwrap();

    let cmd = Originate::application(
        Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("test")),
        Application::simple("park"),
    );
    let uuid = bgapi_originate_ok(&client, &mut events, &cmd).await;
    kill_channel(&client, &uuid).await;

    let deadline = Instant::now() + Duration::from_secs(10);
    while channel_exists(&client, &uuid).await {
        assert!(Instant::now() < deadline, "{} never went away", uuid);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let body = client
        .api(&format!("uuid_dump {}", uuid))
        .await
        .expect("uuid_dump: transport error")
        .body()
        .expect("uuid_dump must return a body")
        .to_string();

    let err =
        parse_channel_dump(&body).expect_err("a dump of a dead channel must not parse as an event");
    let failure = err
        .command_failure()
        .expect("the loop has to recognise this as a command failure");
    let payload = match failure {
        CommandFailure::Err(payload) => payload,
        other => panic!("expected an -ERR reply, got {:?}", other),
    };
    assert!(
        payload.contains("No such channel"),
        "unexpected -ERR payload: {:?}",
        payload
    );
    assert!(err.is_recoverable(), "the loop skips this and carries on");
}

/// `switch_event_serialize` writes this for an empty value. `parse_channel_dump`
/// reads it back as absent while the live decoder keeps it, so a create-side
/// sentinel is not a header the dump lost.
const UNDEF_VALUE: &str = "_undef_";

/// Headers the switch moves on between a channel's CHANNEL_CREATE and a dump
/// taken later: `originate` returns on answer, so the dump is past the state
/// the create reported.
const VOLATILE_HEADERS: &[&str] = &[
    "Channel-State",
    "Channel-State-Number",
    "Channel-Call-State",
    "Answer-State",
    // Reads back as the session UUID until something sets `call_uuid`.
    "Channel-Call-UUID",
    // Flips once the channel reaches the dialplan.
    "Channel-HIT-Dialplan",
    // An originate stamps a placeholder callee id and clears it once the call
    // is up; the dialplan rewrites the caller id it routes on.
    "Caller-Callee-ID-Name",
    "Caller-Callee-ID-Number",
    "Caller-Caller-ID-Name",
    "Caller-Caller-ID-Number",
];

/// [`VOLATILE_HEADERS`] plus the timetable, which fills in as the call
/// progresses. The two creation stamps are fixed by the time the channel is
/// announced, so they stay in the comparison.
fn volatile_headers() -> HashSet<String> {
    let mut set: HashSet<String> = VOLATILE_HEADERS
        .iter()
        .map(|h| h.to_string())
        .collect();
    let fixed = [
        TimetableField::ProfileCreated.as_str(),
        TimetableField::Created.as_str(),
    ];
    for prefix in [TimetablePrefix::Caller, TimetablePrefix::OtherLeg] {
        for suffix in ChannelTimetable::SUFFIXES {
            if fixed.contains(suffix) {
                continue;
            }
            set.insert(format!("{}-{}", prefix.as_str(), suffix));
        }
    }
    set
}

/// Every row's `uuid` -- the one field a bootstrap reads out of the listing.
/// A row without it is a broken contract, not a row to skip.
fn show_channel_uuids(body: &str) -> Vec<String> {
    let json: serde_json::Value =
        serde_json::from_str(body).expect("show channels as json must answer JSON");
    let Some(rows) = json
        .get("rows")
        .and_then(|v| v.as_array())
    else {
        // An empty result carries a row count and no rows key at all.
        return Vec::new();
    };
    rows.iter()
        .map(|row| {
            row.get("uuid")
                .and_then(|v| v.as_str())
                .expect("every listed channel must carry a uuid")
                .to_string()
        })
        .collect()
}

/// The bootstrap the `channel_tracker` example runs: the listing hands over a
/// UUID, the dump hands over the channel. What that rebuilds has to be the
/// CHANNEL_CREATE the switch already sent, or the example is inventing state
/// no consumer could have got from the wire.
#[tokio::test]
#[ignore]
async fn live_show_bootstrap_rebuilds_channel_create() {
    let (client, mut events, _permit) = connect().await;

    client
        .subscribe_events(EventFormat::Plain, &[EslEventType::ChannelCreate])
        .await
        .unwrap();

    // `api`, not `bgapi_originate_ok`: waiting on a BACKGROUND_JOB drains the
    // stream this test reads its CHANNEL_CREATE off.
    let mut reaper = ChannelReaper::new(&client);
    let cmd = Originate::application(
        Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("test")),
        Application::simple("park"),
    );
    let uuid = client
        .api(&cmd.to_string())
        .await
        .expect("originate: transport error")
        .api_result()
        .expect("originate rejected")
        .to_string();
    reaper.track(&uuid);

    let mut created = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while created.is_none() && Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(evt))) => {
                if evt.event_type() == Some(EslEventType::ChannelCreate)
                    && evt.unique_id() == Some(uuid.as_str())
                {
                    created = Some(evt);
                }
            }
            Ok(Some(Err(e))) => panic!("event error: {}", e),
            Ok(None) => panic!("event stream closed"),
            Err(_) => break,
        }
    }

    // The core writes its channel row through an async queue, so the listing
    // trails the event by however long that queue takes to drain.
    let mut listed = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !listed && Instant::now() < deadline {
        let body = client
            .api("show channels as json")
            .await
            .expect("show channels: transport error")
            .api_result()
            .expect("show channels rejected")
            .to_string();
        listed = show_channel_uuids(&body).contains(&uuid);
        if !listed {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    let dump = client
        .api(&format!("uuid_dump {}", uuid))
        .await
        .expect("uuid_dump: transport error")
        .body()
        .expect("uuid_dump must return a body")
        .to_string();

    reaper
        .reap()
        .await;

    let created = created.unwrap_or_else(|| panic!("no CHANNEL_CREATE for {}", uuid));
    assert!(listed, "{} never reached the channel listing", uuid);

    let mut rebuilt = parse_channel_dump(&dump)
        .unwrap_or_else(|e| panic!("uuid_dump {} did not parse: {}", uuid, e));
    rebuilt.set_header(
        EventHeader::EventName.as_str(),
        EslEventType::ChannelCreate.as_str(),
    );
    assert_eq!(
        rebuilt.event_type(),
        Some(EslEventType::ChannelCreate),
        "renaming the dump is the whole translation a bootstrap performs"
    );

    let volatile = volatile_headers();
    // A comparison that skipped its way past the channel's identity would pass
    // on an empty dump.
    for name in DUMP_IDENTITY_HEADERS {
        assert!(
            !volatile.contains(*name)
                && created
                    .header_str(name)
                    .is_some(),
            "{} has to be inside the comparison",
            name
        );
    }

    let mut absent = Vec::new();
    let mut differs = Vec::new();
    for (key, value) in created.headers() {
        if key.starts_with("Event-") || volatile.contains(key.as_str()) || value == UNDEF_VALUE {
            continue;
        }
        match rebuilt.header_str(key) {
            None => absent.push(format!("{key}={value:?}")),
            Some(dumped) if dumped != value => {
                differs.push(format!("{key} create={value:?} dump={dumped:?}"))
            }
            Some(_) => {}
        }
    }
    assert!(
        absent.is_empty(),
        "the rebuilt channel is missing {:?}",
        absent
    );
    assert!(
        differs.is_empty(),
        "the rebuilt channel disagrees: {:?}",
        differs
    );
}
