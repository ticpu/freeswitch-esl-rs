//! Log display functionality for fs_cli-rs

use anyhow::Result;
use colored::*;
use freeswitch_esl_rs::{EslEvent, EslHandle};
use rustyline::ExternalPrinter;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};
use tracing::debug;
use crate::commands::ColorMode;

/// Log display helper functions
pub struct LogDisplay;

impl LogDisplay {
    /// Check for pending log events and display them using ExternalPrinter
    pub async fn check_and_display_logs(
        handle: &mut EslHandle,
        color_mode: ColorMode,
        printer: Option<Arc<Mutex<dyn ExternalPrinter + Send>>>,
    ) -> Result<()> {
        // First, check for immediately available events
        while let Ok(Some(event)) = timeout(Duration::from_millis(1), handle.recv_event()).await? {
            debug!("Received event with headers: {:?}", event.headers);
            if Self::is_log_event(&event) {
                debug!("Found log event!");
                Self::display_log_event(&event, color_mode, &printer).await;
            } else {
                debug!("Received non-log event: {:?}", event.event_type);
                if let Some(ct) = event.headers.get("Content-Type") {
                    debug!("Content-Type: {}", ct);
                }
            }
        }

        // Then, do one longer wait for delayed log events
        if let Ok(Some(event)) = timeout(Duration::from_millis(50), handle.recv_event()).await? {
            debug!("Received delayed event with headers: {:?}", event.headers);
            if Self::is_log_event(&event) {
                debug!("Found delayed log event!");
                Self::display_log_event(&event, color_mode, &printer).await;
            } else {
                debug!("Received delayed non-log event: {:?}", event.event_type);
                if let Some(ct) = event.headers.get("Content-Type") {
                    debug!("Content-Type: {}", ct);
                }
            }
        }
        Ok(())
    }

    /// Check if an event is a log event based on Content-Type header
    fn is_log_event(event: &EslEvent) -> bool {
        if let Some(content_type) = event.headers.get("Content-Type") {
            content_type.eq_ignore_ascii_case("log/data")
        } else {
            false
        }
    }

    /// Display a log event with appropriate formatting and colors using ExternalPrinter
    async fn display_log_event(
        event: &EslEvent,
        color_mode: ColorMode,
        printer: &Option<Arc<Mutex<dyn ExternalPrinter + Send>>>,
    ) {
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
        let formatted_message = match color_mode {
            ColorMode::Never => message.trim().to_string(),
            ColorMode::Tag => Self::format_colored_log_tag_only(message.trim(), log_level),
            ColorMode::Line => Self::format_colored_log_full_line(message.trim(), log_level),
        };

        // Use ExternalPrinter if available, otherwise fallback to println!
        if let Some(printer_arc) = printer {
            if let Ok(mut p) = printer_arc.try_lock() {
                let _ = p.print(formatted_message);
            } else {
                // Fallback if printer is locked
                println!("{}", formatted_message);
            }
        } else {
            println!("{}", formatted_message);
        }
    }

    /// Apply color based on log level
    fn colorize_by_level(text: &str, log_level: u32) -> ColoredString {
        match log_level {
            0 => text.white().bold(), // CONSOLE
            1 => text.red().bold(),   // ALERT
            2 => text.red().bold(),   // CRIT
            3 => text.red(),          // ERR
            4 => text.yellow(),       // WARNING
            5 => text.cyan(),         // NOTICE
            6 => text.green(),        // INFO - green like real fs_cli
            7 => text.bright_black(), // DEBUG - dim like real fs_cli
            _ => text.bright_black(), // DEBUG1-10
        }
    }

    /// Format colored log message with only tag colorized
    fn format_colored_log_tag_only(message: &str, log_level: u32) -> String {
        // Parse the message to find the log level tag and colorize it specifically
        if let Some(level_start) = message.find('[') {
            if let Some(level_end) = message[level_start..].find(']') {
                let level_end = level_start + level_end + 1;
                let before = &message[..level_start];
                let level_tag = &message[level_start..level_end];
                let after = &message[level_end..];

                let colored_level = Self::colorize_by_level(level_tag, log_level);

                return format!("{}{}{}", before, colored_level, after);
            }
        }

        // Fallback: no coloring if tag not found
        message.to_string()
    }

    /// Format colored log message with entire line colorized
    fn format_colored_log_full_line(message: &str, log_level: u32) -> String {
        let colored_message = Self::colorize_by_level(message, log_level);
        format!("{}", colored_message)
    }
}
