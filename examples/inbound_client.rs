//! Example inbound ESL client
//!
//! This example shows how to connect to FreeSWITCH and execute commands.
//!
//! Usage: cargo run --example inbound_client

use freeswitch_esl_rs::{EslError, EslHandle, constants::DEFAULT_ESL_PORT};
use tracing::{error, info};

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

    // Execute some API commands
    info!("Executing API commands...");

    // Get FreeSWITCH status
    match handle.api("status").await {
        Ok(response) => {
            info!("FreeSWITCH Status:");
            if let Some(body) = response.body() {
                println!("{}", body);
            }
        }
        Err(e) => error!("Failed to get status: {}", e),
    }

    // Get channel count
    match handle.api("show channels count").await {
        Ok(response) => {
            info!("Channel Count:");
            if let Some(body) = response.body() {
                println!("{}", body);
            }
        }
        Err(e) => error!("Failed to get channel count: {}", e),
    }

    // List SIP registrations
    match handle.api("sofia status").await {
        Ok(response) => {
            info!("Sofia Status:");
            if let Some(body) = response.body() {
                println!("{}", body);
            }
        }
        Err(e) => error!("Failed to get Sofia status: {}", e),
    }

    // Execute a background API command
    match handle.bgapi("version").await {
        Ok(response) => {
            info!("Background API command sent");
            if let Some(job_uuid) = response.job_uuid() {
                info!("Job UUID: {}", job_uuid);
            }
        }
        Err(e) => error!("Failed to execute bgapi: {}", e),
    }

    // Show some global variables
    let vars = ["hostname", "domain", "local_ip_v4", "switch_serial"];
    for var in &vars {
        match handle.api(&format!("global_getvar {}", var)).await {
            Ok(response) => {
                if let Some(body) = response.body() {
                    info!("{}: {}", var, body.trim());
                }
            }
            Err(e) => error!("Failed to get {}: {}", var, e),
        }
    }

    // Demonstrate originate command (commented out to avoid actually making a call)
    /*
    match handle.api("originate user/1000 &park").await {
        Ok(response) => {
            info!("Originate response: {:?}", response.body());
        }
        Err(e) => error!("Failed to originate: {}", e),
    }
    */

    // Clean disconnect
    info!("Disconnecting...");
    handle.disconnect().await?;
    info!("Disconnected successfully");

    Ok(())
}

/// Helper function to demonstrate reloading XML
#[allow(dead_code)]
async fn reload_xml(handle: &mut EslHandle) -> Result<(), EslError> {
    info!("Reloading XML configuration...");
    let response = handle.api("reloadxml").await?;

    if response.is_success() {
        info!("XML configuration reloaded successfully");
    } else {
        error!("Failed to reload XML: {:?}", response.reply_text());
    }

    Ok(())
}

/// Helper function to demonstrate log level changes
#[allow(dead_code)]
async fn set_log_level(handle: &mut EslHandle, level: &str) -> Result<(), EslError> {
    info!("Setting log level to: {}", level);
    let response = handle.api(&format!("fsctl loglevel {}", level)).await?;

    if response.is_success() {
        info!("Log level set to: {}", level);
    } else {
        error!("Failed to set log level: {:?}", response.reply_text());
    }

    Ok(())
}
