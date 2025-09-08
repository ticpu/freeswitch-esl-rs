//! Command processing and execution for fs_cli-rs

use anyhow::Result;
use colored::*;
use freeswitch_esl_rs::EslHandle;
use rustyline::{history::FileHistory, Editor};

use crate::completion::FsCliCompleter;

/// Command processor for FreeSWITCH CLI commands
pub struct CommandProcessor {
    no_color: bool,
}

impl CommandProcessor {
    /// Create new command processor
    pub fn new(no_color: bool) -> Self {
        Self { no_color }
    }

    /// Execute a FreeSWITCH command
    pub async fn execute_command(&self, handle: &mut EslHandle, command: &str) -> Result<()> {

        // Handle special commands
        if let Some(result) = self.handle_special_command(handle, command).await? {
            println!("{}", result);
            return Ok(());
        }

        // Execute as API command
        match handle.api(command).await {
            Ok(response) => {
                if !response.is_success() {
                    if let Some(reply) = response.reply_text() {
                        if !self.no_color {
                            eprintln!("{}: {}", "API Error".red().bold(), reply);
                        } else {
                            eprintln!("API Error: {}", reply);
                        }
                        return Ok(()); // Don't treat API errors as fatal
                    }
                }

                let body = response.body_string();
                if !body.trim().is_empty() {
                    println!("{}", body);
                }
            }
            Err(e) => {
                return Err(e.into());
            }
        }

        Ok(())
    }

    /// Handle special CLI commands that need custom processing
    async fn handle_special_command(
        &self,
        handle: &mut EslHandle,
        command: &str,
    ) -> Result<Option<String>> {
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(None);
        }

        match parts[0].to_lowercase().as_str() {
            "show" if parts.len() > 1 => self.handle_show_command(handle, &parts[1..]).await,
            "status" => {
                let response = handle.api("status").await?;
                Ok(Some(response.body_string()))
            }
            "version" => {
                let response = handle.api("version").await?;
                Ok(Some(response.body_string()))
            }
            "uptime" => {
                let response = handle.api("status").await?;
                Ok(Some(self.extract_uptime(&response.body_string())))
            }
            "reload" => {
                if parts.len() > 1 {
                    let module = parts[1];
                    let response = handle.api(&format!("reload {}", module)).await?;
                    Ok(Some(format!(
                        "Reloaded module: {}\n{}",
                        module,
                        response.body_string()
                    )))
                } else {
                    let response = handle.api("reloadxml").await?;
                    Ok(Some(format!(
                        "Reloaded XML configuration\n{}",
                        response.body_string()
                    )))
                }
            }
            "originate" => {
                if parts.len() >= 3 {
                    let call_string = parts[1..].join(" ");
                    let response = handle.api(&format!("originate {}", call_string)).await?;
                    Ok(Some(format!(
                        "Originate command executed\n{}",
                        response.body_string()
                    )))
                } else {
                    Ok(Some(
                        "Usage: originate <call_url> <destination>".to_string(),
                    ))
                }
            }
            _ => Ok(None), // Not a special command
        }
    }

    /// Handle 'show' commands with enhanced formatting
    async fn handle_show_command(
        &self,
        handle: &mut EslHandle,
        parts: &[&str],
    ) -> Result<Option<String>> {
        if parts.is_empty() {
            return Ok(Some(
                "Usage: show <channels|calls|registrations|modules|...>".to_string(),
            ));
        }

        let subcommand = parts[0].to_lowercase();
        let command = match subcommand.as_str() {
            "channels" => {
                if parts.len() > 1 && parts[1] == "count" {
                    "show channels count"
                } else {
                    "show channels"
                }
            }
            "calls" => "show calls",
            "registrations" => "sofia status",
            "modules" => "show modules",
            "interfaces" => "show interfaces",
            "api" => "show api",
            "application" => "show application",
            "codec" => "show codec",
            "file" => "show file",
            "timer" => "show timer",
            "tasks" => "show tasks",
            "complete" => "show complete",
            _ => {
                return Ok(Some(format!("Unknown show command: {}\n\
                Available: channels, calls, registrations, modules, interfaces, api, application, codec, file, timer, tasks", 
                subcommand)));
            }
        };

        let response = handle.api(command).await?;
        Ok(Some(response.body_string()))
    }



    /// Extract uptime information from status output
    fn extract_uptime(&self, status_output: &str) -> String {
        for line in status_output.lines() {
            if line.contains("UP")
                && (line.contains("years") || line.contains("days") || line.contains("hours"))
            {
                return line.trim().to_string();
            }
        }
        "Uptime information not found".to_string()
    }


    /// Show command history
    pub fn show_history(&self, rl: &Editor<FsCliCompleter, FileHistory>) {
        if !self.no_color {
            println!("{}", "Command History:".cyan().bold());
        } else {
            println!("Command History:");
        }

        let history = rl.history();
        for (i, entry) in history
            .iter()
            .enumerate()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .take(20)
        {
            if !self.no_color {
                println!("  {}: {}", (i + 1).to_string().dimmed(), entry);
            } else {
                println!("  {}: {}", i + 1, entry);
            }
        }
    }

    /// Show help information
    pub fn show_help(&self) {
        let help_text = r#"
FreeSWITCH CLI Commands:

Basic Commands:
  status                    - Show system status
  version                   - Show FreeSWITCH version
  uptime                    - Show system uptime
  help                      - Show this help

Show Commands:
  show channels             - List active channels
  show channels count       - Show channel count
  show calls                - Show active calls
  show registrations        - Show SIP registrations
  show modules              - List loaded modules
  show interfaces           - Show interfaces

Control Commands:
  reload [module]           - Reload module or XML config
  originate <url> <dest>    - Originate a call

Function Key Shortcuts:
  F1  = help                F7  = /log console
  F2  = status              F8  = /log debug  
  F3  = show channels       F9  = sofia status profile internal
  F4  = show calls          F10 = fsctl pause
  F5  = sofia status        F11 = fsctl resume
  F6  = reloadxml           F12 = version

Built-in Commands:
  history                   - Show command history
  clear                     - Clear screen
  quit/exit/bye            - Exit the CLI

You can execute any FreeSWITCH API command directly.
Use Tab for command completion and Up/Down arrows for history.
"#;

        if !self.no_color {
            println!("{}", help_text.cyan());
        } else {
            println!("{}", help_text);
        }
    }
}
