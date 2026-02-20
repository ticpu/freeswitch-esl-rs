//! Example outbound ESL server
//!
//! This example shows how to accept outbound connections from FreeSWITCH
//! and handle call control.
//!
//! Usage: cargo run --example outbound_server
//!
//! To test this, configure FreeSWITCH with:
//! <action application="socket" data="localhost:8040 async full"/>

use freeswitch_esl_tokio::{AppCommand, EslClient, EslEventType, EventFormat};
use tokio::net::TcpListener;
use tracing::{debug, error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let bind_addr = "0.0.0.0:8040";
    info!("Starting outbound ESL server on {}", bind_addr);

    let listener = TcpListener::bind(bind_addr).await?;
    info!("Listening for outbound connections from FreeSWITCH...");

    loop {
        match EslClient::accept_outbound(&listener).await {
            Ok((client, mut events)) => {
                info!("Accepted new outbound connection");
                tokio::spawn(async move {
                    if let Err(e) = handle_call(&client, &mut events).await {
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

async fn handle_call(
    client: &EslClient,
    events: &mut freeswitch_esl_tokio::EslEventStream,
) -> Result<(), freeswitch_esl_tokio::EslError> {
    info!("Handling new call...");

    client
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

    info!("Answering call...");
    client
        .send_command(AppCommand::answer())
        .await?;

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

    info!("Playing greeting message...");
    client
        .send_command(AppCommand::playback("ivr/ivr-welcome.wav"))
        .await?;

    let mut dtmf_buffer = String::new();

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
        debug!("Received event: {:?}", event.event_type());

        match event.event_type() {
            Some(EslEventType::ChannelHangup) => {
                info!("Call hung up");
                break;
            }
            Some(EslEventType::PlaybackStop) => {
                info!("Playback finished");
                client
                    .send_command(AppCommand::playback(
                        "ivr/ivr-please_enter_extension_followed_by_pound.wav",
                    ))
                    .await?;
            }
            Some(EslEventType::Dtmf) => {
                if let Some(digit) = event.header("DTMF-Digit") {
                    info!("Received DTMF: {}", digit);

                    if digit == "#" {
                        info!("User entered: {}", dtmf_buffer);
                        handle_dtmf_input(client, &dtmf_buffer).await?;
                        dtmf_buffer.clear();
                    } else {
                        dtmf_buffer.push_str(digit);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
}

async fn handle_dtmf_input(
    client: &EslClient,
    input: &str,
) -> Result<(), freeswitch_esl_tokio::EslError> {
    info!("Processing DTMF input: {}", input);

    match input {
        "1000" | "1001" | "1002" | "1003" => {
            info!("Transferring to extension: {}", input);
            client
                .send_command(AppCommand::playback("ivr/ivr-hold_connect_call.wav"))
                .await?;
            client
                .send_command(AppCommand::transfer(input, None, None))
                .await?;
        }
        "0" => {
            info!("Transferring to operator");
            client
                .send_command(AppCommand::playback("ivr/ivr-hold_connect_call.wav"))
                .await?;
            client
                .send_command(AppCommand::transfer("operator", None, None))
                .await?;
        }
        "9" => {
            info!("Hanging up call per user request");
            client
                .send_command(AppCommand::playback("voicemail/vm-goodbye.wav"))
                .await?;
            client
                .send_command(AppCommand::hangup(Some("NORMAL_CLEARING")))
                .await?;
        }
        "" => {
            client
                .send_command(AppCommand::playback(
                    "ivr/ivr-please_enter_extension_followed_by_pound.wav",
                ))
                .await?;
        }
        _ => {
            info!("Invalid extension: {}", input);
            client
                .send_command(AppCommand::playback(
                    "ivr/ivr-that_was_an_invalid_entry.wav",
                ))
                .await?;
            client
                .send_command(AppCommand::playback(
                    "ivr/ivr-please_enter_extension_followed_by_pound.wav",
                ))
                .await?;
        }
    }

    Ok(())
}
