//! Command execution and response handling

use crate::{
    constants::*,
    error::{EslError, EslResult},
    event::EslEvent,
};
use std::collections::HashMap;

/// Response from ESL command execution
#[derive(Debug, Clone)]
pub struct EslResponse {
    /// Response headers
    headers: HashMap<String, String>,
    /// Response body (optional)
    body: Option<String>,
    /// Whether the command was successful
    success: bool,
}

impl EslResponse {
    /// Create new response
    pub fn new(headers: HashMap<String, String>, body: Option<String>) -> Self {
        let reply_text = headers
            .get(HEADER_REPLY_TEXT)
            .map(|s| s.as_str())
            .unwrap_or("");
        let success = reply_text.starts_with("+OK") || reply_text.is_empty();

        Self {
            headers,
            body,
            success,
        }
    }

    /// Check if command was successful
    pub fn is_success(&self) -> bool {
        self.success
    }

    /// Get response body
    pub fn body(&self) -> Option<&String> {
        self.body.as_ref()
    }

    /// Get response body as string, empty if None
    pub fn body_string(&self) -> String {
        self.body.as_ref().cloned().unwrap_or_default()
    }

    /// Get header value
    pub fn header(&self, name: &str) -> Option<&String> {
        self.headers.get(name)
    }

    /// Get all headers
    pub fn headers(&self) -> &HashMap<String, String> {
        &self.headers
    }

    /// Get reply text
    pub fn reply_text(&self) -> Option<&String> {
        self.headers.get(HEADER_REPLY_TEXT)
    }

    /// Get job UUID for background commands
    pub fn job_uuid(&self) -> Option<&String> {
        self.headers.get(HEADER_JOB_UUID)
    }

    /// Convert to result based on success status
    pub fn into_result(self) -> EslResult<Self> {
        if self.success {
            Ok(self)
        } else {
            let reply_text = self
                .reply_text()
                .cloned()
                .unwrap_or_else(|| "Command failed".to_string());
            Err(EslError::CommandFailed { reply_text })
        }
    }
}

/// Builder for ESL commands
pub struct CommandBuilder {
    command: String,
    headers: HashMap<String, String>,
    body: Option<String>,
}

impl CommandBuilder {
    /// Create new command builder
    pub fn new(command: &str) -> Self {
        Self {
            command: command.to_string(),
            headers: HashMap::new(),
            body: None,
        }
    }

    /// Add header to command
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.insert(name.to_string(), value.to_string());
        self
    }

    /// Set command body
    pub fn body(mut self, body: &str) -> Self {
        self.body = Some(body.to_string());
        self
    }

    /// Build the command string
    pub fn build(self) -> String {
        let mut result = self.command;
        result.push_str(LINE_TERMINATOR);

        // Add headers
        for (key, value) in &self.headers {
            result.push_str(&format!("{}: {}{}", key, value, LINE_TERMINATOR));
        }

        // Add body if present
        if let Some(body) = &self.body {
            result.push_str(&format!(
                "Content-Length: {}{}",
                body.len(),
                LINE_TERMINATOR
            ));
            result.push_str(LINE_TERMINATOR);
            result.push_str(body);
        } else {
            result.push_str(LINE_TERMINATOR);
        }

        result
    }
}

/// ESL command types
#[derive(Debug, Clone)]
pub enum EslCommand {
    /// Authenticate with password
    Auth { password: String },
    /// Authenticate with user and password
    UserAuth { user: String, password: String },
    /// Execute API command
    Api { command: String },
    /// Execute background API command  
    BgApi { command: String },
    /// Subscribe to events
    Events { format: String, events: String },
    /// Set event filters
    Filter { header: String, value: String },
    /// Send message to channel
    SendMsg {
        uuid: Option<String>,
        event: EslEvent,
    },
    /// Execute application on channel
    Execute {
        app: String,
        args: Option<String>,
        uuid: Option<String>,
    },
    /// Exit/logout
    Exit,
    /// Log level
    Log { level: String },
    /// No operation / keepalive
    NoOp,
}

impl EslCommand {
    /// Format a simple command with optional arguments
    fn format_simple_command(cmd: &str, args: &[&str]) -> String {
        let mut result = String::from(cmd);
        for arg in args {
            result.push(' ');
            result.push_str(arg);
        }
        result.push_str(HEADER_TERMINATOR);
        result
    }

    /// Convert command to wire format string
    pub fn to_wire_format(&self) -> String {
        match self {
            EslCommand::Auth { password } => Self::format_simple_command("auth", &[password]),
            EslCommand::UserAuth { user, password } => {
                Self::format_simple_command("userauth", &[&format!("{}:{}", user, password)])
            }
            EslCommand::Api { command } => Self::format_simple_command("api", &[command]),
            EslCommand::BgApi { command } => Self::format_simple_command("bgapi", &[command]),
            EslCommand::Events { format, events } => {
                Self::format_simple_command("event", &[format, events])
            }
            EslCommand::Filter { header, value } => {
                Self::format_simple_command("filter", &[header, value])
            }
            EslCommand::SendMsg { uuid, event } => {
                let mut builder = CommandBuilder::new(&format!(
                    "sendmsg{}",
                    uuid.as_ref().map(|u| format!(" {}", u)).unwrap_or_default()
                ));

                // Add event headers
                for (key, value) in &event.headers {
                    builder = builder.header(key, value);
                }

                // Add event body if present
                if let Some(body) = &event.body {
                    builder = builder.body(body);
                }

                builder.build()
            }
            EslCommand::Execute { app, args, uuid } => {
                let mut event = EslEvent::new();
                event.set_header("call-command".to_string(), "execute".to_string());
                event.set_header("execute-app-name".to_string(), app.clone());

                if let Some(args) = args {
                    event.set_header("execute-app-arg".to_string(), args.clone());
                }

                EslCommand::SendMsg {
                    uuid: uuid.clone(),
                    event,
                }
                .to_wire_format()
            }
            EslCommand::Exit => Self::format_simple_command("exit", &[]),
            EslCommand::Log { level } => Self::format_simple_command("log", &[level]),
            EslCommand::NoOp => Self::format_simple_command("noop", &[]),
        }
    }
}

/// Execute application commands
pub struct AppCommand;

impl AppCommand {
    /// Answer call
    pub fn answer() -> EslCommand {
        EslCommand::Execute {
            app: "answer".to_string(),
            args: None,
            uuid: None,
        }
    }

    /// Hangup call
    pub fn hangup(cause: Option<&str>) -> EslCommand {
        EslCommand::Execute {
            app: "hangup".to_string(),
            args: cause.map(|c| c.to_string()),
            uuid: None,
        }
    }

    /// Play audio file
    pub fn playback(file: &str) -> EslCommand {
        EslCommand::Execute {
            app: "playback".to_string(),
            args: Some(file.to_string()),
            uuid: None,
        }
    }

    /// Bridge two channels
    pub fn bridge(destination: &str) -> EslCommand {
        EslCommand::Execute {
            app: "bridge".to_string(),
            args: Some(destination.to_string()),
            uuid: None,
        }
    }

    /// Set channel variable
    pub fn set_var(name: &str, value: &str) -> EslCommand {
        EslCommand::Execute {
            app: "set".to_string(),
            args: Some(format!("{}={}", name, value)),
            uuid: None,
        }
    }

    /// Park call
    pub fn park() -> EslCommand {
        EslCommand::Execute {
            app: "park".to_string(),
            args: None,
            uuid: None,
        }
    }

    /// Transfer call
    pub fn transfer(extension: &str, dialplan: Option<&str>, context: Option<&str>) -> EslCommand {
        let mut args = extension.to_string();
        if let Some(dp) = dialplan {
            args.push(' ');
            args.push_str(dp);
        }
        if let Some(ctx) = context {
            args.push(' ');
            args.push_str(ctx);
        }

        EslCommand::Execute {
            app: "transfer".to_string(),
            args: Some(args),
            uuid: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_builder() {
        let cmd = CommandBuilder::new("api status")
            .header("Custom-Header", "value")
            .body("test body")
            .build();

        assert!(cmd.contains("api status"));
        assert!(cmd.contains("Custom-Header: value"));
        assert!(cmd.contains("Content-Length: 9"));
        assert!(cmd.contains("test body"));
    }

    #[test]
    fn test_esl_commands() {
        let auth = EslCommand::Auth {
            password: "test".to_string(),
        };
        assert_eq!(auth.to_wire_format(), "auth test\n\n");

        let api = EslCommand::Api {
            command: "status".to_string(),
        };
        assert_eq!(api.to_wire_format(), "api status\n\n");

        let events = EslCommand::Events {
            format: "plain".to_string(),
            events: "ALL".to_string(),
        };
        assert_eq!(events.to_wire_format(), "event plain ALL\n\n");
    }

    #[test]
    fn test_app_commands() {
        let answer = AppCommand::answer().to_wire_format();
        assert!(answer.contains("execute-app-name: answer"));

        let hangup = AppCommand::hangup(Some("NORMAL_CLEARING")).to_wire_format();
        assert!(hangup.contains("execute-app-name: hangup"));
        assert!(hangup.contains("execute-app-arg: NORMAL_CLEARING"));
    }
}
