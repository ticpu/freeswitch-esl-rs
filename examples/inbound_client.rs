//! Example inbound ESL client
//!
//! This example shows how to connect to FreeSWITCH and execute commands.
//!
//! Usage: cargo run --example inbound_client

use freeswitch_esl_rs::{constants::DEFAULT_ESL_PORT, EslClient, EslError};
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let (client, _events) = match EslClient::connect("localhost", DEFAULT_ESL_PORT, "ClueCon").await
    {
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

    info!("Executing API commands...");

    match client
        .api("status")
        .await
    {
        Ok(response) => {
            info!("FreeSWITCH Status:");
            if let Some(body) = response.body() {
                println!("{}", body);
            }
        }
        Err(e) => error!("Failed to get status: {}", e),
    }

    match client
        .api("show channels count")
        .await
    {
        Ok(response) => {
            info!("Channel Count:");
            if let Some(body) = response.body() {
                println!("{}", body);
            }
        }
        Err(e) => error!("Failed to get channel count: {}", e),
    }

    match client
        .api("sofia status")
        .await
    {
        Ok(response) => {
            info!("Sofia Status:");
            if let Some(body) = response.body() {
                println!("{}", body);
            }
        }
        Err(e) => error!("Failed to get Sofia status: {}", e),
    }

    match client
        .bgapi("version")
        .await
    {
        Ok(response) => {
            info!("Background API command sent");
            if let Some(job_uuid) = response.job_uuid() {
                info!("Job UUID: {}", job_uuid);
            }
        }
        Err(e) => error!("Failed to execute bgapi: {}", e),
    }

    let vars = ["hostname", "domain", "local_ip_v4", "switch_serial"];
    for var in &vars {
        match client
            .api(&format!("global_getvar {}", var))
            .await
        {
            Ok(response) => {
                if let Some(body) = response.body() {
                    info!("{}: {}", var, body.trim());
                }
            }
            Err(e) => error!("Failed to get {}: {}", var, e),
        }
    }

    info!("Disconnecting...");
    client
        .disconnect()
        .await?;
    info!("Disconnected successfully");

    Ok(())
}
