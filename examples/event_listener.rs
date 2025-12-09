//! Example ESL event listener
//!
//! This example shows how to subscribe to FreeSWITCH events and process them.
//!
//! Usage: cargo run --example event_listener

use freeswitch_esl_rs::{
    constants::DEFAULT_ESL_PORT, EslError, EslEventType, EslHandle, EventFormat,
};
use std::collections::HashMap;
use tracing::{debug, error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Connect to FreeSWITCH
    let mut handle = match EslHandle::connect("localhost", DEFAULT_ESL_PORT, "ClueCon").await {
        Ok(handle) => {
            info!("Successfully connected to FreeSWITCH");
            handle
        }
        Err(EslError::Io(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            error!("Failed to connect to FreeSWITCH - is it running on localhost:8022?");
            return Err(e.into());
        }
        Err(e) => {
            error!("Failed to connect: {}", e);
            return Err(e.into());
        }
    };

    // Subscribe to events we're interested in
    info!("Subscribing to events...");
    handle
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

    // Optionally filter events (uncomment to filter by a specific UUID)
    // handle.filter_events("Unique-ID", "specific-uuid-here").await?;

    // Set up call tracking
    let mut active_calls: HashMap<String, CallInfo> = HashMap::new();
    let mut event_count = 0u64;

    info!("Listening for events... Press Ctrl+C to exit");

    // Main event loop
    loop {
        match handle.recv_event_timeout(30000).await {
            Ok(Some(event)) => {
                event_count += 1;
                debug!("Received event #{}: {:?}", event_count, event.event_type());

                // Process the event
                if let Err(e) = process_event(&event, &mut active_calls).await {
                    error!("Error processing event: {}", e);
                }
            }
            Ok(None) => {
                info!("No events received - connection may be closed");
                break;
            }
            Err(EslError::Timeout { .. }) => {
                debug!("No events received in last 30 seconds");
                info!(
                    "Status: {} active calls, {} total events processed",
                    active_calls.len(),
                    event_count
                );
                continue;
            }
            Err(e) => {
                error!("Error receiving event: {}", e);
                break;
            }
        }
    }

    // Clean disconnect
    info!("Disconnecting...");
    handle.disconnect().await?;
    info!("Disconnected successfully");

    Ok(())
}

/// Process individual events
async fn process_event(
    event: &freeswitch_esl_rs::EslEvent,
    active_calls: &mut HashMap<String, CallInfo>,
) -> Result<(), EslError> {
    match event.event_type() {
        Some(EslEventType::ChannelCreate) => {
            handle_channel_create(event, active_calls).await?;
        }
        Some(EslEventType::ChannelAnswer) => {
            handle_channel_answer(event, active_calls).await?;
        }
        Some(EslEventType::ChannelHangup) => {
            handle_channel_hangup(event, active_calls).await?;
        }
        Some(EslEventType::ChannelHangupComplete) => {
            handle_channel_hangup_complete(event, active_calls).await?;
        }
        Some(EslEventType::Dtmf) => {
            handle_dtmf(event).await?;
        }
        Some(EslEventType::Heartbeat) => {
            handle_heartbeat(event).await?;
        }
        Some(EslEventType::BackgroundJob) => {
            handle_background_job(event).await?;
        }
        _ => {
            debug!("Ignoring event type: {:?}", event.event_type());
        }
    }

    Ok(())
}

/// Handle channel creation
async fn handle_channel_create(
    event: &freeswitch_esl_rs::EslEvent,
    active_calls: &mut HashMap<String, CallInfo>,
) -> Result<(), EslError> {
    if let Some(uuid) = event.unique_id() {
        let caller_id = event
            .header("Caller-Caller-ID-Number")
            .unwrap_or(&"Unknown".to_string())
            .clone();
        let destination = event
            .header("Caller-Destination-Number")
            .unwrap_or(&"Unknown".to_string())
            .clone();
        let direction = event
            .header("Call-Direction")
            .unwrap_or(&"Unknown".to_string())
            .clone();

        let call_info = CallInfo {
            uuid: uuid.clone(),
            caller_id: caller_id.clone(),
            destination: destination.clone(),
            direction: direction.clone(),
            start_time: std::time::Instant::now(),
            answered_time: None,
            hangup_time: None,
        };

        active_calls.insert(uuid.clone(), call_info);

        info!(
            "📞 New call created: {} -> {} ({})",
            caller_id, destination, direction
        );
    }

    Ok(())
}

/// Handle channel answer
async fn handle_channel_answer(
    event: &freeswitch_esl_rs::EslEvent,
    active_calls: &mut HashMap<String, CallInfo>,
) -> Result<(), EslError> {
    if let Some(uuid) = event.unique_id() {
        if let Some(call_info) = active_calls.get_mut(uuid) {
            call_info.answered_time = Some(std::time::Instant::now());
            let duration = call_info.start_time.elapsed();

            info!(
                "✅ Call answered: {} (ring time: {:.2}s)",
                call_info.caller_id,
                duration.as_secs_f64()
            );
        }
    }

    Ok(())
}

/// Handle channel hangup
async fn handle_channel_hangup(
    event: &freeswitch_esl_rs::EslEvent,
    active_calls: &mut HashMap<String, CallInfo>,
) -> Result<(), EslError> {
    if let Some(uuid) = event.unique_id() {
        if let Some(call_info) = active_calls.get_mut(uuid) {
            call_info.hangup_time = Some(std::time::Instant::now());

            let unknown_cause = "UNKNOWN".to_string();
            let cause = event.header("Hangup-Cause").unwrap_or(&unknown_cause);
            let total_duration = call_info.start_time.elapsed();
            let talk_time = call_info.answered_time.map(|t| t.elapsed());

            if let Some(talk_duration) = talk_time {
                info!(
                    "📱 Call ended: {} (cause: {}, talk time: {:.2}s)",
                    call_info.caller_id,
                    cause,
                    talk_duration.as_secs_f64()
                );
            } else {
                info!(
                    "📱 Call ended: {} (cause: {}, not answered, total: {:.2}s)",
                    call_info.caller_id,
                    cause,
                    total_duration.as_secs_f64()
                );
            }
        }
    }

    Ok(())
}

/// Handle channel hangup complete (cleanup)
async fn handle_channel_hangup_complete(
    event: &freeswitch_esl_rs::EslEvent,
    active_calls: &mut HashMap<String, CallInfo>,
) -> Result<(), EslError> {
    if let Some(uuid) = event.unique_id() {
        active_calls.remove(uuid);
        debug!("🗑️  Cleaned up call: {}", uuid);
    }

    Ok(())
}

/// Handle DTMF events
async fn handle_dtmf(event: &freeswitch_esl_rs::EslEvent) -> Result<(), EslError> {
    if let (Some(uuid), Some(digit)) = (event.unique_id(), event.header("DTMF-Digit")) {
        info!(
            "🔢 DTMF: {} pressed '{}' (duration: {}ms)",
            uuid,
            digit,
            event.header("DTMF-Duration").unwrap_or(&"0".to_string())
        );
    }

    Ok(())
}

/// Handle heartbeat events
async fn handle_heartbeat(event: &freeswitch_esl_rs::EslEvent) -> Result<(), EslError> {
    if let Some(uptime) = event.header("Up-Time") {
        debug!("💓 Heartbeat: FreeSWITCH uptime {}", uptime);

        // You can extract more system information from heartbeat events
        if let Some(sessions) = event.header("Session-Count") {
            info!(
                "📊 System stats - Active sessions: {}, Uptime: {}",
                sessions, uptime
            );
        }
    }

    Ok(())
}

/// Handle background job completion
async fn handle_background_job(event: &freeswitch_esl_rs::EslEvent) -> Result<(), EslError> {
    if let Some(job_uuid) = event.job_uuid() {
        info!("⚙️  Background job completed: {}", job_uuid);

        if let Some(body) = event.body() {
            debug!("Job result: {}", body.trim());
        }
    }

    Ok(())
}

/// Call information tracking structure
#[derive(Debug, Clone)]
struct CallInfo {
    uuid: String,
    caller_id: String,
    destination: String,
    direction: String,
    start_time: std::time::Instant,
    answered_time: Option<std::time::Instant>,
    hangup_time: Option<std::time::Instant>,
}

/// Example of event filtering and advanced processing
#[allow(dead_code)]
async fn advanced_event_processing(handle: &mut EslHandle) -> Result<(), EslError> {
    // Subscribe to specific events with JSON format for easier parsing
    handle
        .subscribe_events(
            EventFormat::Json,
            &[
                EslEventType::ChannelCreate,
                EslEventType::ChannelAnswer,
                EslEventType::ChannelHangup,
            ],
        )
        .await?;

    // Set up multiple filters
    handle.filter_events("Call-Direction", "inbound").await?;
    handle.filter_events("Caller-Context", "public").await?;

    // Process events with structured data
    while let Some(event) = handle.recv_event().await? {
        // With JSON format, you can access structured data more easily
        info!("Event with structured data: {:?}", event.headers);
    }

    Ok(())
}

/// Example of call detail record (CDR) generation
#[allow(dead_code)]
fn generate_cdr(call_info: &CallInfo) {
    let cdr = serde_json::json!({
        "uuid": call_info.uuid,
        "caller_id": call_info.caller_id,
        "destination": call_info.destination,
        "direction": call_info.direction,
        "start_time": call_info.start_time.elapsed().as_secs(),
        "answered": call_info.answered_time.is_some(),
        "talk_time": call_info.answered_time.map(|t| t.elapsed().as_secs()),
        "total_time": call_info.start_time.elapsed().as_secs()
    });

    info!("CDR: {}", cdr);

    // In a real application, you might save this to a database or file
}
