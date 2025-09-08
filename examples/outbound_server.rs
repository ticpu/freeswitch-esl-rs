//! Example outbound ESL server
//!
//! This example shows how to accept outbound connections from FreeSWITCH
//! and handle call control.
//!
//! Usage: cargo run --example outbound_server
//!
//! To test this, configure FreeSWITCH with:
//! <action application="socket" data="localhost:8040 async full"/>

use freeswitch_esl_rs::{command::AppCommand, EslError, EslEventType, EslHandle, EventFormat};
use tokio::net::TcpListener;
use tracing::{debug, error, info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::init();

    let bind_addr = "0.0.0.0:8040";
    info!("Starting outbound ESL server on {}", bind_addr);

    let listener = TcpListener::bind(bind_addr).await?;
    info!("Listening for outbound connections from FreeSWITCH...");

    loop {
        match EslHandle::accept_outbound(listener.try_clone().unwrap()).await {
            Ok(mut handle) => {
                info!("Accepted new outbound connection");

                // Spawn a task to handle this connection
                tokio::spawn(async move {
                    if let Err(e) = handle_call(&mut handle).await {
                        error!("Error handling call: {}", e);
                    }
                });
            }
            Err(e) => {
                error!("Failed to accept connection: {}", e);
                break;
            }
        }
    }

    Ok(())
}

/// Handle an individual call
async fn handle_call(handle: &mut EslHandle) -> Result<(), EslError> {
    info!("Handling new call...");

    // Subscribe to events for this call
    handle
        .subscribe_events(
            EventFormat::Plain,
            &[
                EslEventType::ChannelAnswer,
                EslEventType::ChannelHangup,
                EslEventType::Dtmf,
                EslEventType::PlaybackStart,
                EslEventType::PlaybackStop,
            ],
        )
        .await?;

    // Get channel information
    let channel_info = get_channel_info(handle).await?;
    info!(
        "Call from: {} to: {}",
        channel_info.caller_id, channel_info.destination
    );

    // Answer the call
    info!("Answering call...");
    handle.send_command(AppCommand::answer()).await?;

    // Wait for answer confirmation
    while let Some(event) = handle.recv_event().await? {
        debug!("Received event: {:?}", event.event_type());

        match event.event_type() {
            Some(EslEventType::ChannelAnswer) => {
                info!("Call answered successfully");
                break;
            }
            Some(EslEventType::ChannelHangup) => {
                info!("Call hung up before answer");
                return Ok(());
            }
            _ => continue,
        }
    }

    // Play a greeting
    info!("Playing greeting message...");
    handle
        .send_command(AppCommand::playback("ivr/ivr-welcome.wav"))
        .await?;

    // Main call handling loop
    let mut dtmf_buffer = String::new();
    let mut playback_finished = false;

    while let Some(event) = handle.recv_event().await? {
        debug!("Received event: {:?}", event.event_type());

        match event.event_type() {
            Some(EslEventType::ChannelHangup) => {
                info!("Call hung up");
                break;
            }
            Some(EslEventType::PlaybackStop) => {
                playback_finished = true;
                info!("Playback finished");

                // Prompt for DTMF input
                handle
                    .send_command(AppCommand::playback(
                        "ivr/ivr-please_enter_extension_followed_by_pound.wav",
                    ))
                    .await?;
            }
            Some(EslEventType::Dtmf) => {
                if let Some(digit) = event.header("DTMF-Digit") {
                    info!("Received DTMF: {}", digit);

                    if digit == "#" {
                        // Process the entered number
                        info!("User entered: {}", dtmf_buffer);
                        handle_dtmf_input(handle, &dtmf_buffer).await?;
                        dtmf_buffer.clear();
                    } else {
                        dtmf_buffer.push_str(digit);
                    }
                }
            }
            _ => {
                // Handle other events as needed
            }
        }
    }

    Ok(())
}

/// Extract channel information from the connection
async fn get_channel_info(handle: &mut EslHandle) -> Result<ChannelInfo, EslError> {
    // In outbound mode, channel info is available via variables
    let caller_id = handle
        .api("channel_var Caller-Caller-ID-Number")
        .await?
        .body_string();
    let destination = handle
        .api("channel_var Caller-Destination-Number")
        .await?
        .body_string();

    Ok(ChannelInfo {
        caller_id: if caller_id.is_empty() {
            "Unknown".to_string()
        } else {
            caller_id
        },
        destination: if destination.is_empty() {
            "Unknown".to_string()
        } else {
            destination
        },
    })
}

/// Handle DTMF input from the user
async fn handle_dtmf_input(handle: &mut EslHandle, input: &str) -> Result<(), EslError> {
    info!("Processing DTMF input: {}", input);

    match input {
        "1000" | "1001" | "1002" | "1003" => {
            // Transfer to extension
            info!("Transferring to extension: {}", input);
            handle
                .send_command(AppCommand::playback("ivr/ivr-hold_connect_call.wav"))
                .await?;
            handle
                .send_command(AppCommand::transfer(input, None, None))
                .await?;
        }
        "0" => {
            // Transfer to operator
            info!("Transferring to operator");
            handle
                .send_command(AppCommand::playback("ivr/ivr-hold_connect_call.wav"))
                .await?;
            handle
                .send_command(AppCommand::transfer("operator", None, None))
                .await?;
        }
        "9" => {
            // Hang up
            info!("Hanging up call per user request");
            handle
                .send_command(AppCommand::playback("voicemail/vm-goodbye.wav"))
                .await?;
            handle
                .send_command(AppCommand::hangup(Some("NORMAL_CLEARING")))
                .await?;
        }
        "" => {
            // Empty input - play instructions
            handle
                .send_command(AppCommand::playback(
                    "ivr/ivr-please_enter_extension_followed_by_pound.wav",
                ))
                .await?;
        }
        _ => {
            // Invalid input
            info!("Invalid extension: {}", input);
            handle
                .send_command(AppCommand::playback(
                    "ivr/ivr-that_was_an_invalid_entry.wav",
                ))
                .await?;
            handle
                .send_command(AppCommand::playback(
                    "ivr/ivr-please_enter_extension_followed_by_pound.wav",
                ))
                .await?;
        }
    }

    Ok(())
}

/// Channel information structure
#[derive(Debug)]
struct ChannelInfo {
    caller_id: String,
    destination: String,
}

/// Demonstrate advanced call control features
#[allow(dead_code)]
async fn advanced_call_control(handle: &mut EslHandle) -> Result<(), EslError> {
    // Set channel variables
    handle
        .send_command(AppCommand::set_var("custom_var", "example_value"))
        .await?;

    // Record the call
    handle
        .execute("record_session", Some("/tmp/recorded_call.wav"), None)
        .await?;

    // Start music on hold
    handle
        .send_command(AppCommand::playback("local_stream://moh"))
        .await?;

    // Bridge to another call (example)
    // handle.send_command(AppCommand::bridge("user/1000")).await?;

    // Park the call
    handle.send_command(AppCommand::park()).await?;

    Ok(())
}

/// Example of handling background jobs
#[allow(dead_code)]
async fn handle_background_jobs(handle: &mut EslHandle) -> Result<(), EslError> {
    // Start a background API call
    let response = handle.bgapi("status").await?;

    if let Some(job_uuid) = response.job_uuid() {
        info!("Started background job: {}", job_uuid);

        // Listen for the job completion event
        while let Some(event) = handle.recv_event().await? {
            if event.is_event_type(EslEventType::BackgroundJob) {
                if let Some(event_job_uuid) = event.job_uuid() {
                    if event_job_uuid == job_uuid {
                        info!("Background job completed");
                        if let Some(body) = event.body() {
                            info!("Job result: {}", body);
                        }
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}
