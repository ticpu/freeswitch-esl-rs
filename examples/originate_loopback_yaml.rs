//! Originate a loopback call described entirely by a YAML file, then read the
//! resulting channel variables back off both loopback legs.
//!
//! A `loopback/<ext>/<ctx>` originate creates two channels: the A leg, which
//! is the UUID `originate` hands back, and the B leg, which is routed into
//! `<ctx>` at extension `<ext>`. Every variable in the originate's `[...]`
//! block lands on both, so a dialplan running on the B leg can read what the
//! originator set. No bracket block can target one leg on its own; for that
//! see the "Keeping a variable off a leg" section of
//! docs/originate-loopback-yaml.md.
//!
//! Usage: cargo run --example originate_loopback_yaml
//!   Configure via ESL_HOST, ESL_PORT, ESL_PASSWORD env vars.
//!   Requires FreeSWITCH with `mod_loopback` and extension 9199 in context
//!   `test` (answer + echo).

use freeswitch_esl_tokio::commands::{UuidGetVar, UuidKill};
use freeswitch_esl_tokio::{
    EslClient, EslEventType, EventFormat, EventHeader, HeaderLookup, Originate,
    DEFAULT_ESL_PASSWORD, DEFAULT_ESL_PORT,
};
use std::time::Duration;

/// The originate command lives in a YAML file, not in this source.
const ORIGINATE_YAML: &str = include_str!("originate_loopback.yaml");

/// Variables the YAML sets. Each must be readable on *both* loopback legs.
const EXPECTED_VARS: &[(&str, &str)] = &[
    ("customer_id", "CUST-42"),
    ("tenant", "acme"),
    // Escaped as `'T-1001\, urgent'` on the wire; FreeSWITCH unescapes it.
    ("sip_h_X-Ticket", "T-1001, urgent"),
];

/// Read a channel variable, mapping "not set" to `None`. `uuid_getvar` writes
/// the literal `_undef_` for an unset variable, so its absence arrives as a
/// successful reply rather than an error.
async fn getvar(client: &EslClient, uuid: &str, name: &str) -> Option<String> {
    let cmd = UuidGetVar::new(uuid, name);
    let resp = client
        .api(&cmd.to_string())
        .await
        .ok()?;
    match resp.api_result() {
        Ok("_undef_") | Err(_) => None,
        Ok(value) => Some(value.to_string()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let originate: Originate = yaml_serde::from_str(ORIGINATE_YAML)?;

    println!("=== YAML -> Originate ===");
    println!("{}\n", originate);

    let host = std::env::var("ESL_HOST").unwrap_or_else(|_| "localhost".to_string());
    let port: u16 = std::env::var("ESL_PORT")
        .ok()
        .and_then(|p| {
            p.parse()
                .ok()
        })
        .unwrap_or(DEFAULT_ESL_PORT);
    let password =
        std::env::var("ESL_PASSWORD").unwrap_or_else(|_| DEFAULT_ESL_PASSWORD.to_string());

    let (client, mut events) = EslClient::connect(&host, port, &password).await?;

    client
        .subscribe_events(
            EventFormat::Plain,
            &[EslEventType::ChannelCreate, EslEventType::ChannelAnswer],
        )
        .await?;

    // `api originate` blocks until the channel answers and returns its UUID.
    // The A leg answers once the B leg's dialplan does.
    let resp = client
        .api(&originate.to_string())
        .await?;
    let a_leg = resp
        .api_result()?
        .to_string();
    println!("=== Channels ===");
    println!("A leg (originate result): {}", a_leg);

    // mod_loopback cross-links the pair with this variable.
    let b_leg = getvar(&client, &a_leg, "other_loopback_leg_uuid")
        .await
        .ok_or("A leg has no other_loopback_leg_uuid -- is this really a loopback channel?")?;
    println!("B leg (other_loopback_leg_uuid): {}\n", b_leg);

    // Drain the events the originate produced, so the reader sees the two
    // channels appear rather than just trusting the UUIDs above.
    println!("=== Events ===");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while let Ok(Some(Ok(evt))) = tokio::time::timeout_at(deadline, events.recv()).await {
        let Some(uuid) = evt.unique_id() else {
            continue;
        };
        if uuid != a_leg && uuid != b_leg {
            continue;
        }
        let leg = if uuid == a_leg { "A" } else { "B" };
        println!(
            "{:<16} leg={} name={}",
            evt.event_type()
                .map(|t| t.to_string())
                .unwrap_or_default(),
            leg,
            evt.header(EventHeader::ChannelName)
                .unwrap_or("?")
        );
    }

    println!("\n=== Variables on both legs ===");
    let mut all_present = true;
    for (name, expected) in EXPECTED_VARS {
        for (leg, uuid) in [("A", &a_leg), ("B", &b_leg)] {
            let got = getvar(&client, uuid, name).await;
            let ok = got.as_deref() == Some(*expected);
            all_present &= ok;
            println!(
                "{} {} leg {:<16} = {:?}",
                if ok { "ok  " } else { "FAIL" },
                leg,
                name,
                got.unwrap_or_else(|| "<unset>".into())
            );
        }
    }

    // The caller ID reaches the B leg, which is where the dialplan runs.
    // `origination_caller_id_*` beats the positional cid_name/cid_num args.
    println!("\n=== Caller ID as seen by the B leg dialplan ===");
    for name in ["caller_id_name", "caller_id_number"] {
        println!(
            "  {:<18} = {:?}",
            name,
            getvar(&client, &b_leg, name)
                .await
                .unwrap_or_else(|| "<unset>".into())
        );
    }

    // Hanging up the A leg tears the pair down.
    let kill = UuidKill::new(&a_leg);
    client
        .api(&kill.to_string())
        .await?;
    println!("\nhung up {}", a_leg);

    if !all_present {
        return Err("some variables did not reach both legs".into());
    }
    Ok(())
}
