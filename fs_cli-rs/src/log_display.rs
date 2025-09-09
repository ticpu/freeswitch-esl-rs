//! Log display functionality for fs_cli-rs

use anyhow::Result;
use colored::*;
use freeswitch_esl_rs::{EslEvent, EslHandle};
use tokio::time::{timeout, Duration};
use tracing::debug;

/// Log display helper functions
pub struct LogDisplay;

impl LogDisplay {
    /// Check for pending log events and display them
    pub async fn check_and_display_logs(handle: &mut EslHandle, no_color: bool) -> Result<()> {
        // Check for pending events with very short timeout
        while let Ok(Some(event)) = timeout(Duration::from_millis(1), handle.recv_event()).await? {
            if Self::is_log_event(&event) {
                Self::display_log_event(&event, no_color);
            } else {
                debug!("Received non-log event: {:?}", event.event_type);
            }
        }
        Ok(())
    }

    /// Check if an event is a log event based on Content-Type header
    fn is_log_event(event: &EslEvent) -> bool {
        event.headers.get("Content-Type") == Some(&"log/data".to_string())
    }

    /// Display a log event with appropriate formatting and colors
    fn display_log_event(event: &EslEvent, no_color: bool) {
        // Extract log level
        let log_level = event
            .headers
            .get("Log-Level")
            .and_then(|level| level.parse::<u32>().ok())
            .unwrap_or(7); // Default to debug level

        // Get log message body
        let message = event.body.as_deref().unwrap_or("");
        if message.trim().is_empty() {
            return;
        }

        // Format and display the log message
        if no_color {
            println!("{}", message.trim());
        } else {
            let colored_message = Self::colorize_log_message(message.trim(), log_level);
            println!("{}", colored_message);
        }
    }

    /// Apply color coding based on log level (0-7 scale)
    fn colorize_log_message(message: &str, log_level: u32) -> ColoredString {
        match log_level {
            0 => message.white().bold(), // CONSOLE
            1 => message.red().bold(),   // ALERT
            2 => message.red(),          // CRIT
            3 => message.red(),          // ERR
            4 => message.yellow(),       // WARNING
            5 => message.cyan(),         // NOTICE
            6 => message.white(),        // INFO
            7 => message.bright_black(), // DEBUG
            _ => message.bright_black(), // DEBUG1-10
        }
    }
}
