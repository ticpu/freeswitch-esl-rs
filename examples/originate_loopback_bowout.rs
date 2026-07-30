//! Drive a loopback pair through a *bowout*: mod_loopback notices that both
//! of its legs are bridged to real endpoints, splices those two endpoints
//! together with `uuid_bridge`, and removes itself from the audio path.
//!
//! Before:  null/nearend = loopback-a : loopback-b = null/farend
//! After:   null/nearend = null/farend
//!
//! Bowout needs all of: both loopback legs bridged to a non-loopback channel,
//! both answered, and neither leg setting `loopback_bowout=false`. The B leg
//! gets its bridge from the `app=bridge:null/farend` destination form; the A
//! leg gets its own from the `&bridge(null/nearend)` originate target.
//!
//! Usage: cargo run --example originate_loopback_bowout
//!   Configure via ESL_HOST, ESL_PORT, ESL_PASSWORD env vars.

use freeswitch_esl_tokio::commands::UuidKill;
use freeswitch_esl_tokio::{
    EslClient, EslEventType, EventFormat, EventHeader, HeaderLookup, Originate,
    DEFAULT_ESL_PASSWORD, DEFAULT_ESL_PORT,
};
use std::time::Duration;

const BOWOUT_YAML: &str = include_str!("originate_loopback_bowout.yaml");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let originate: Originate = yaml_serde::from_str(BOWOUT_YAML)?;

    println!("=== YAML -> Originate ===");
    println!("{}\n", originate);

    let host = std::env::var("ESL_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
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

    // Subscribe before originating: the pair can bow out within milliseconds
    // of the A leg's bridge starting, and those hangups are the evidence.
    client
        .subscribe_events(
            EventFormat::Plain,
            &[
                EslEventType::ChannelBridge,
                EslEventType::ChannelHangupComplete,
            ],
        )
        .await?;

    let resp = client
        .api(&originate.to_string())
        .await?;
    let a_leg = resp
        .api_result()?
        .to_string();
    println!("A leg (loopback): {}\n", a_leg);

    println!("=== Waiting for bowout ===");
    // Two loopback hangups carrying loopback_hangup_cause=bridge, plus the
    // uuid_bridge that joins the two surviving real channels.
    let mut resigned = Vec::new();
    let mut spliced: Option<(String, String)> = None;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while resigned.len() < 2 || spliced.is_none() {
        let Ok(Some(Ok(evt))) = tokio::time::timeout_at(deadline, events.recv()).await else {
            break;
        };
        match evt.event_type() {
            Some(EslEventType::ChannelHangupComplete) => {
                // Some means the leg resigned and its call lives on elsewhere;
                // None is a real teardown. Never test the cause value to decide
                // that -- mod_loopback writes a different token depending on
                // which path resigned, so matching one silently misses the
                // other. Presence is the signal.
                let Some(resignation) = evt.loopback_resignation() else {
                    continue;
                };
                let name = evt
                    .header(EventHeader::ChannelName)
                    .unwrap_or("?");
                println!(
                    "resigned {:<20} -> real channel {}",
                    name,
                    resignation
                        .other_uuid()
                        .unwrap_or("?")
                );
                resigned.push(name.to_string());
            }
            Some(EslEventType::ChannelBridge) => {
                // Three bridges happen here: loopback-a=null/nearend,
                // loopback-b=null/farend, and finally the bowout's
                // uuid_bridge. Only the last one has a real channel on
                // both sides.
                let (Some(this), Some(other)) = (
                    evt.header(EventHeader::ChannelName),
                    evt.header(EventHeader::OtherLegChannelName),
                ) else {
                    continue;
                };
                if this.starts_with("loopback/") || other.starts_with("loopback/") {
                    continue;
                }
                let (Some(a), Some(b)) =
                    (evt.unique_id(), evt.header(EventHeader::OtherLegUniqueId))
                else {
                    continue;
                };
                println!("bridged  {} = {}", this, other);
                spliced = Some((a.to_string(), b.to_string()));
            }
            _ => {}
        }
    }

    let Some((near, far)) = spliced else {
        return Err("no uuid_bridge between the real legs -- bowout did not happen".into());
    };
    if resigned.len() != 2 {
        return Err(format!("expected 2 loopback legs to resign, saw {}", resigned.len()).into());
    }

    println!("\n=== After bowout ===");
    println!(
        "the loopback pair is gone; {} talks straight to {}",
        near, far
    );

    // Killing either survivor tears down what is now a plain two-party call.
    let kill = UuidKill::new(&near);
    client
        .api(&kill.to_string())
        .await?;
    println!("hung up {}", near);

    Ok(())
}
