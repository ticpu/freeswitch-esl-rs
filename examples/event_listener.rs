//! Example ESL event listener
//!
//! This example shows how to subscribe to FreeSWITCH events and process them.
//!
//! Usage: cargo run --example event_listener

use freeswitch_esl_tokio::{EslClient, EslError, EslEventType, EventFormat, DEFAULT_ESL_PORT};
use std::collections::HashMap;
use tracing::{debug, error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let (client, mut events) =
        match EslClient::connect("localhost", DEFAULT_ESL_PORT, "ClueCon").await {
            Ok(pair) => {
                info!("Successfully connected to FreeSWITCH");
                pair
            }
            Err(EslError::Io(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                error!("Failed to connect to FreeSWITCH - is it running on localhost:8021?");
                return Err(e.into());
            }
            Err(e) => {
                error!("Failed to connect: {}", e);
                return Err(e.into());
            }
        };

    info!("Subscribing to events...");
    client
        .subscribe_events(
            EventFormat::Plain,
            &[
                EslEventType::ChannelCreate,
                EslEventType::ChannelAnswer,
                EslEventType::ChannelHangup,
                EslEventType::ChannelHangupComplete,
                EslEventType::Dtmf,
                EslEventType::Heartbeat,
                EslEventType::BackgroundJob,
            ],
        )
        .await?;

    let mut active_calls: HashMap<String, CallInfo> = HashMap::new();
    let mut event_count = 0u64;

    info!("Listening for events... Press Ctrl+C to exit");

    while let Some(result) = events
        .recv()
        .await
    {
        let event = match result {
            Ok(event) => event,
            Err(e) => {
                error!("Event error: {}", e);
                continue;
            }
        };
        event_count += 1;
        debug!("Received event #{}: {:?}", event_count, event.event_type());

        if let Err(e) = process_event(&event, &mut active_calls) {
            error!("Error processing event: {}", e);
        }
    }

    info!("Connection closed, total events: {}", event_count);
    client
        .disconnect()
        .await?;

    Ok(())
}

fn process_event(
    event: &freeswitch_esl_tokio::EslEvent,
    active_calls: &mut HashMap<String, CallInfo>,
) -> Result<(), EslError> {
    match event.event_type() {
        Some(EslEventType::ChannelCreate) => {
            if let Some(uuid) = event.unique_id() {
                let caller_id = event
                    .caller_id_number()
                    .unwrap_or("Unknown");
                let destination = event
                    .header("Caller-Destination-Number")
                    .unwrap_or("Unknown");
                let direction = event
                    .call_direction()
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "Unknown".into());

                let call_info = CallInfo {
                    caller_id: caller_id.to_string(),
                    start_time: std::time::Instant::now(),
                    answered_time: None,
                };

                info!("New call: {} -> {} ({})", caller_id, destination, direction);
                active_calls.insert(uuid.to_string(), call_info);
            }
        }
        Some(EslEventType::ChannelAnswer) => {
            if let Some(uuid) = event.unique_id() {
                if let Some(call_info) = active_calls.get_mut(uuid) {
                    call_info.answered_time = Some(std::time::Instant::now());
                    let duration = call_info
                        .start_time
                        .elapsed();
                    info!(
                        "Call answered: {} (ring time: {:.2}s)",
                        call_info.caller_id,
                        duration.as_secs_f64()
                    );
                }
            }
        }
        Some(EslEventType::ChannelHangup) => {
            if let Some(uuid) = event.unique_id() {
                if let Some(call_info) = active_calls.get(uuid) {
                    let cause = event
                        .hangup_cause()
                        .unwrap_or("UNKNOWN");
                    let talk_time = call_info
                        .answered_time
                        .map(|t| t.elapsed());

                    if let Some(talk_duration) = talk_time {
                        info!(
                            "Call ended: {} (cause: {}, talk time: {:.2}s)",
                            call_info.caller_id,
                            cause,
                            talk_duration.as_secs_f64()
                        );
                    } else {
                        info!(
                            "Call ended: {} (cause: {}, not answered)",
                            call_info.caller_id, cause
                        );
                    }
                }
            }
        }
        Some(EslEventType::ChannelHangupComplete) => {
            if let Some(uuid) = event.unique_id() {
                active_calls.remove(uuid);
                debug!("Cleaned up call: {}", uuid);
            }
        }
        Some(EslEventType::Dtmf) => {
            if let (Some(uuid), Some(digit)) = (event.unique_id(), event.header("DTMF-Digit")) {
                info!("DTMF: {} pressed '{}'", uuid, digit);
            }
        }
        Some(EslEventType::Heartbeat) => {
            if let Some(sessions) = event.header("Session-Count") {
                info!("Heartbeat - active sessions: {}", sessions);
            }
        }
        Some(EslEventType::BackgroundJob) => {
            if let Some(job_uuid) = event.job_uuid() {
                info!("Background job completed: {}", job_uuid);
                if let Some(body) = event.body() {
                    debug!("Job result: {}", body.trim());
                }
            }
        }
        _ => {
            debug!("Ignoring event type: {:?}", event.event_type());
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct CallInfo {
    caller_id: String,
    start_time: std::time::Instant,
    answered_time: Option<std::time::Instant>,
}
