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
        self.body
            .as_ref()
    }

    /// Get response body as string, empty if None
    pub fn body_string(&self) -> String {
        self.body
            .as_ref()
            .cloned()
            .unwrap_or_default()
    }

    /// Get header value
    pub fn header(&self, name: &str) -> Option<&String> {
        self.headers
            .get(name)
    }

    /// Get all headers
    pub fn headers(&self) -> &HashMap<String, String> {
        &self.headers
    }

    /// Get reply text
    pub fn reply_text(&self) -> Option<&String> {
        self.headers
            .get(HEADER_REPLY_TEXT)
    }

    /// Get job UUID for background commands.
    ///
    /// For `bgapi` responses, FreeSWITCH returns the Job-UUID both in the
    /// `Reply-Text` header (`+OK Job-UUID: <uuid>`) and as a separate
    /// `Job-UUID` header. This method reads the dedicated header.
    pub fn job_uuid(&self) -> Option<&String> {
        self.headers
            .get(HEADER_JOB_UUID)
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
        self.headers
            .insert(name.to_string(), value.to_string());
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
    /// Enable log forwarding at the given level
    Log { level: String },
    /// Disable log forwarding
    NoLog,
    /// No operation / keepalive
    NoOp,
    /// Fire an event into FreeSWITCH's event bus
    SendEvent { event: EslEvent },
    /// Subscribe to session events (outbound: no uuid, inbound: with uuid)
    MyEvents {
        format: String,
        uuid: Option<String>,
    },
    /// Keep socket open after channel hangup
    Linger { timeout: Option<u32> },
    /// Cancel linger mode
    NoLinger,
    /// Resume dialplan execution on socket disconnect
    Resume,
    /// Unsubscribe from specific events
    NixEvent { events: String },
    /// Unsubscribe from all events
    NoEvents,
    /// Remove event filters
    FilterDelete {
        header: String,
        value: Option<String>,
    },
    /// Redirect session events to ESL (outbound mode)
    DivertEvents { on: bool },
    /// Read a channel variable (outbound mode)
    GetVar { name: String },
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
                    uuid.as_ref()
                        .map(|u| format!(" {}", u))
                        .unwrap_or_default()
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
            EslCommand::NoLog => Self::format_simple_command("nolog", &[]),
            EslCommand::NoOp => Self::format_simple_command("noop", &[]),
            EslCommand::SendEvent { event } => {
                let event_name = event
                    .event_type
                    .map(|t| t.to_string())
                    .or_else(|| {
                        event
                            .headers
                            .get("Event-Name")
                            .cloned()
                    })
                    .unwrap_or_else(|| "CUSTOM".to_string());

                let mut builder = CommandBuilder::new(&format!("sendevent {}", event_name));

                for (key, value) in &event.headers {
                    builder = builder.header(key, value);
                }

                if let Some(body) = &event.body {
                    builder = builder.body(body);
                }

                builder.build()
            }
            EslCommand::MyEvents { format, uuid } => match uuid {
                Some(u) => Self::format_simple_command("myevents", &[u, format]),
                None => Self::format_simple_command("myevents", &[format]),
            },
            EslCommand::Linger { timeout } => match timeout {
                Some(n) => Self::format_simple_command("linger", &[&n.to_string()]),
                None => Self::format_simple_command("linger", &[]),
            },
            EslCommand::NoLinger => Self::format_simple_command("nolinger", &[]),
            EslCommand::Resume => Self::format_simple_command("resume", &[]),
            EslCommand::NixEvent { events } => Self::format_simple_command("nixevent", &[events]),
            EslCommand::NoEvents => Self::format_simple_command("noevents", &[]),
            EslCommand::FilterDelete { header, value } => {
                if header == "all" {
                    Self::format_simple_command("filter", &["delete", "all"])
                } else {
                    match value {
                        Some(v) => Self::format_simple_command("filter", &["delete", header, v]),
                        None => Self::format_simple_command("filter", &["delete", header]),
                    }
                }
            }
            EslCommand::DivertEvents { on } => {
                let arg = if *on { "on" } else { "off" };
                Self::format_simple_command("divert_events", &[arg])
            }
            EslCommand::GetVar { name } => Self::format_simple_command("getvar", &[name]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EslEventType;

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
        use crate::app::dptools::AppCommand;

        let answer = AppCommand::answer().to_wire_format();
        assert!(answer.contains("execute-app-name: answer"));

        let hangup = AppCommand::hangup(Some("NORMAL_CLEARING")).to_wire_format();
        assert!(hangup.contains("execute-app-name: hangup"));
        assert!(hangup.contains("execute-app-arg: NORMAL_CLEARING"));
    }

    #[test]
    fn test_myevents_wire_format() {
        let cmd = EslCommand::MyEvents {
            format: "plain".to_string(),
            uuid: None,
        };
        assert_eq!(cmd.to_wire_format(), "myevents plain\n\n");
    }

    #[test]
    fn test_myevents_uuid_wire_format() {
        let cmd = EslCommand::MyEvents {
            format: "json".to_string(),
            uuid: Some("abc-123".to_string()),
        };
        assert_eq!(cmd.to_wire_format(), "myevents abc-123 json\n\n");
    }

    #[test]
    fn test_linger_wire_format() {
        let cmd = EslCommand::Linger { timeout: None };
        assert_eq!(cmd.to_wire_format(), "linger\n\n");
    }

    #[test]
    fn test_linger_timeout_wire_format() {
        let cmd = EslCommand::Linger { timeout: Some(600) };
        assert_eq!(cmd.to_wire_format(), "linger 600\n\n");
    }

    #[test]
    fn test_nolinger_wire_format() {
        let cmd = EslCommand::NoLinger;
        assert_eq!(cmd.to_wire_format(), "nolinger\n\n");
    }

    #[test]
    fn test_resume_wire_format() {
        let cmd = EslCommand::Resume;
        assert_eq!(cmd.to_wire_format(), "resume\n\n");
    }

    #[test]
    fn test_sendevent_wire_format() {
        let mut event = EslEvent::with_type(EslEventType::Custom);
        event.set_header("Event-Name".to_string(), "CUSTOM".to_string());
        event.set_header("Event-Subclass".to_string(), "my::test_event".to_string());

        let cmd = EslCommand::SendEvent { event };
        let wire = cmd.to_wire_format();

        assert!(wire.starts_with("sendevent CUSTOM\n"));
        assert!(wire.contains("Event-Name: CUSTOM\n"));
        assert!(wire.contains("Event-Subclass: my::test_event\n"));
        assert!(wire.ends_with("\n\n"));
    }

    #[test]
    fn test_sendevent_wire_format_with_body() {
        let mut event = EslEvent::with_type(EslEventType::Custom);
        event.set_header("Event-Name".to_string(), "CUSTOM".to_string());
        event.set_body("hello world".to_string());

        let cmd = EslCommand::SendEvent { event };
        let wire = cmd.to_wire_format();

        assert!(wire.starts_with("sendevent CUSTOM\n"));
        assert!(wire.contains("Content-Length: 11\n"));
        assert!(wire.ends_with("hello world"));
    }

    #[test]
    fn test_nixevent_wire_format() {
        let cmd = EslCommand::NixEvent {
            events: "CHANNEL_CREATE CHANNEL_DESTROY".to_string(),
        };
        assert_eq!(
            cmd.to_wire_format(),
            "nixevent CHANNEL_CREATE CHANNEL_DESTROY\n\n"
        );
    }

    #[test]
    fn test_noevents_wire_format() {
        let cmd = EslCommand::NoEvents;
        assert_eq!(cmd.to_wire_format(), "noevents\n\n");
    }

    #[test]
    fn test_filter_delete_wire_format() {
        let cmd = EslCommand::FilterDelete {
            header: "Event-Name".to_string(),
            value: None,
        };
        assert_eq!(cmd.to_wire_format(), "filter delete Event-Name\n\n");
    }

    #[test]
    fn test_filter_delete_value_wire_format() {
        let cmd = EslCommand::FilterDelete {
            header: "Event-Name".to_string(),
            value: Some("CHANNEL_CREATE".to_string()),
        };
        assert_eq!(
            cmd.to_wire_format(),
            "filter delete Event-Name CHANNEL_CREATE\n\n"
        );
    }

    #[test]
    fn test_filter_delete_all_wire_format() {
        let cmd = EslCommand::FilterDelete {
            header: "all".to_string(),
            value: None,
        };
        assert_eq!(cmd.to_wire_format(), "filter delete all\n\n");
    }

    #[test]
    fn test_divert_events_wire_format() {
        let cmd_on = EslCommand::DivertEvents { on: true };
        assert_eq!(cmd_on.to_wire_format(), "divert_events on\n\n");

        let cmd_off = EslCommand::DivertEvents { on: false };
        assert_eq!(cmd_off.to_wire_format(), "divert_events off\n\n");
    }

    #[test]
    fn test_getvar_wire_format() {
        let cmd = EslCommand::GetVar {
            name: "caller_id_name".to_string(),
        };
        assert_eq!(cmd.to_wire_format(), "getvar caller_id_name\n\n");
    }

    #[test]
    fn test_sendevent_no_event_type() {
        let mut event = EslEvent::new();
        event.set_header("Event-Name".to_string(), "CUSTOM".to_string());

        let cmd = EslCommand::SendEvent { event };
        let wire = cmd.to_wire_format();
        assert!(wire.starts_with("sendevent CUSTOM\n"));

        let bare_event = EslEvent::new();
        let cmd2 = EslCommand::SendEvent { event: bare_event };
        let wire2 = cmd2.to_wire_format();
        assert!(wire2.starts_with("sendevent CUSTOM\n"));
    }
}
