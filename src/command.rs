//! Command execution and response handling

use crate::{
    constants::*,
    error::{EslError, EslResult},
    event::EslEvent,
};
use std::collections::HashMap;
use std::fmt;

/// Validate that a user-provided string contains no newline characters.
///
/// ESL commands are line-delimited; embedded newlines would allow injection
/// of arbitrary protocol commands.
fn validate_no_newlines(s: &str, context: &str) -> EslResult<()> {
    if s.contains('\n') || s.contains('\r') {
        return Err(EslError::ProtocolError {
            message: format!("{} must not contain newlines", context),
        });
    }
    Ok(())
}

/// Reply-Text classification per the ESL wire protocol.
///
/// FreeSWITCH commands return `+OK …` on success and `-ERR …` on failure.
/// A handful of commands (`getvar`) return the raw value with no prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyStatus {
    /// Reply-Text starts with `+OK` or is absent/empty.
    Ok,
    /// Reply-Text starts with `-ERR`.
    Err,
    /// Reply-Text present but matches neither `+OK` nor `-ERR`.
    /// This is normal for `getvar` (which returns the bare variable value)
    /// but unexpected for most other commands.
    Other,
}

/// Response from ESL command execution
#[derive(Debug, Clone, PartialEq)]
pub struct EslResponse {
    headers: HashMap<String, String>,
    body: Option<String>,
    status: ReplyStatus,
}

impl EslResponse {
    /// Create new response
    pub fn new(headers: HashMap<String, String>, body: Option<String>) -> Self {
        let status = match headers
            .get(HEADER_REPLY_TEXT)
            .map(|s| s.as_str())
        {
            None | Some("") => ReplyStatus::Ok,
            Some(t) if t.starts_with("+OK") => ReplyStatus::Ok,
            Some(t) if t.starts_with("-ERR") => ReplyStatus::Err,
            Some(_) => ReplyStatus::Other,
        };

        Self {
            headers,
            body,
            status,
        }
    }

    /// Check if command was successful (`+OK` or no Reply-Text).
    pub fn is_success(&self) -> bool {
        self.status == ReplyStatus::Ok
    }

    /// Reply status classification.
    pub fn reply_status(&self) -> ReplyStatus {
        self.status
    }

    /// Get response body
    pub fn body(&self) -> Option<&str> {
        self.body
            .as_deref()
    }

    /// Get response body as string, empty if None
    pub fn body_string(&self) -> String {
        self.body
            .as_ref()
            .cloned()
            .unwrap_or_default()
    }

    /// Get header value
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(name)
            .map(|s| s.as_str())
    }

    /// Get all headers
    pub fn headers(&self) -> &HashMap<String, String> {
        &self.headers
    }

    /// Get reply text
    pub fn reply_text(&self) -> Option<&str> {
        self.headers
            .get(HEADER_REPLY_TEXT)
            .map(|s| s.as_str())
    }

    /// Get job UUID for background commands.
    ///
    /// For `bgapi` responses, FreeSWITCH returns the Job-UUID both in the
    /// `Reply-Text` header (`+OK Job-UUID: <uuid>`) and as a separate
    /// `Job-UUID` header. This method reads the dedicated header.
    pub fn job_uuid(&self) -> Option<&str> {
        self.headers
            .get(HEADER_JOB_UUID)
            .map(|s| s.as_str())
    }

    /// Convert to result based on success status.
    ///
    /// ```
    /// # use freeswitch_esl_tokio::EslResponse;
    /// # use std::collections::HashMap;
    /// let headers: HashMap<String, String> = [("Reply-Text".into(), "+OK".into())].into();
    /// let resp = EslResponse::new(headers, None);
    /// assert!(resp.into_result().is_ok());
    /// ```
    pub fn into_result(self) -> EslResult<Self> {
        match self.status {
            ReplyStatus::Ok => Ok(self),
            ReplyStatus::Err => {
                let reply_text = self
                    .reply_text()
                    .unwrap_or("-ERR")
                    .to_string();
                Err(EslError::CommandFailed { reply_text })
            }
            ReplyStatus::Other => {
                let reply_text = self
                    .reply_text()
                    .unwrap_or("")
                    .to_string();
                Err(EslError::UnexpectedReply { reply_text })
            }
        }
    }
}

/// Builder for ESL commands
#[derive(Debug)]
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

    /// Add header to command.
    ///
    /// Returns an error if the name or value contains newline characters.
    pub fn header(mut self, name: &str, value: &str) -> EslResult<Self> {
        validate_no_newlines(name, "header name")?;
        validate_no_newlines(value, "header value")?;
        self.headers
            .insert(name.to_string(), value.to_string());
        Ok(self)
    }

    /// Set command body.
    ///
    /// The body is length-delimited so it may contain newlines.
    pub fn body(mut self, body: &str) -> Self {
        self.body = Some(body.to_string());
        self
    }

    /// Build the command string
    pub fn build(self) -> String {
        use std::fmt::Write;
        let mut result = self.command;
        result.push_str(LINE_TERMINATOR);

        for (key, value) in &self.headers {
            let _ = write!(result, "{}: {}{}", key, value, LINE_TERMINATOR);
        }

        if let Some(body) = &self.body {
            let _ = write!(result, "Content-Length: {}{}", body.len(), LINE_TERMINATOR);
            result.push_str(LINE_TERMINATOR);
            result.push_str(body);
        } else {
            result.push_str(LINE_TERMINATOR);
        }

        result
    }
}

/// ESL command types
#[derive(Clone)]
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
    /// Request channel data in outbound mode
    Connect,
}

impl fmt::Debug for EslCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EslCommand::Auth { .. } => f
                .debug_struct("Auth")
                .field("password", &"[REDACTED]")
                .finish(),
            EslCommand::UserAuth { user, .. } => f
                .debug_struct("UserAuth")
                .field("user", user)
                .field("password", &"[REDACTED]")
                .finish(),
            EslCommand::Api { command } => f
                .debug_struct("Api")
                .field("command", command)
                .finish(),
            EslCommand::BgApi { command } => f
                .debug_struct("BgApi")
                .field("command", command)
                .finish(),
            EslCommand::Events { format, events } => f
                .debug_struct("Events")
                .field("format", format)
                .field("events", events)
                .finish(),
            EslCommand::Filter { header, value } => f
                .debug_struct("Filter")
                .field("header", header)
                .field("value", value)
                .finish(),
            EslCommand::SendMsg { uuid, event } => f
                .debug_struct("SendMsg")
                .field("uuid", uuid)
                .field("event", event)
                .finish(),
            EslCommand::Execute { app, args, uuid } => f
                .debug_struct("Execute")
                .field("app", app)
                .field("args", args)
                .field("uuid", uuid)
                .finish(),
            EslCommand::Exit => write!(f, "Exit"),
            EslCommand::Log { level } => f
                .debug_struct("Log")
                .field("level", level)
                .finish(),
            EslCommand::NoLog => write!(f, "NoLog"),
            EslCommand::NoOp => write!(f, "NoOp"),
            EslCommand::SendEvent { event } => f
                .debug_struct("SendEvent")
                .field("event", event)
                .finish(),
            EslCommand::MyEvents { format, uuid } => f
                .debug_struct("MyEvents")
                .field("format", format)
                .field("uuid", uuid)
                .finish(),
            EslCommand::Linger { timeout } => f
                .debug_struct("Linger")
                .field("timeout", timeout)
                .finish(),
            EslCommand::NoLinger => write!(f, "NoLinger"),
            EslCommand::Resume => write!(f, "Resume"),
            EslCommand::NixEvent { events } => f
                .debug_struct("NixEvent")
                .field("events", events)
                .finish(),
            EslCommand::NoEvents => write!(f, "NoEvents"),
            EslCommand::FilterDelete { header, value } => f
                .debug_struct("FilterDelete")
                .field("header", header)
                .field("value", value)
                .finish(),
            EslCommand::DivertEvents { on } => f
                .debug_struct("DivertEvents")
                .field("on", on)
                .finish(),
            EslCommand::GetVar { name } => f
                .debug_struct("GetVar")
                .field("name", name)
                .finish(),
            EslCommand::Connect => write!(f, "Connect"),
        }
    }
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

    /// Validate all user-supplied fields, then convert to wire format.
    pub fn to_wire_format(&self) -> EslResult<String> {
        match self {
            EslCommand::Auth { password } => {
                validate_no_newlines(password, "password")?;
                Ok(Self::format_simple_command("auth", &[password]))
            }
            EslCommand::UserAuth { user, password } => {
                validate_no_newlines(user, "user")?;
                validate_no_newlines(password, "password")?;
                Ok(Self::format_simple_command(
                    "userauth",
                    &[&format!("{}:{}", user, password)],
                ))
            }
            EslCommand::Api { command } => {
                validate_no_newlines(command, "api command")?;
                Ok(Self::format_simple_command("api", &[command]))
            }
            EslCommand::BgApi { command } => {
                validate_no_newlines(command, "bgapi command")?;
                Ok(Self::format_simple_command("bgapi", &[command]))
            }
            EslCommand::Events { format, events } => {
                validate_no_newlines(format, "event format")?;
                validate_no_newlines(events, "event list")?;
                Ok(Self::format_simple_command("event", &[format, events]))
            }
            EslCommand::Filter { header, value } => {
                validate_no_newlines(header, "filter header")?;
                validate_no_newlines(value, "filter value")?;
                Ok(Self::format_simple_command("filter", &[header, value]))
            }
            EslCommand::SendMsg { uuid, event } => {
                if let Some(u) = uuid {
                    validate_no_newlines(u, "sendmsg uuid")?;
                }
                let cmd_str = format!(
                    "sendmsg{}",
                    uuid.as_ref()
                        .map(|u| format!(" {}", u))
                        .unwrap_or_default()
                );
                let mut builder = CommandBuilder::new(&cmd_str);

                for (key, value) in event.headers() {
                    builder = builder.header(key, value)?;
                }

                if let Some(body) = event.body() {
                    builder = builder.body(body);
                }

                Ok(builder.build())
            }
            EslCommand::Execute { app, args, uuid } => {
                validate_no_newlines(app, "execute app")?;
                if let Some(a) = args {
                    validate_no_newlines(a, "execute args")?;
                }
                if let Some(u) = uuid {
                    validate_no_newlines(u, "execute uuid")?;
                }

                let mut event = EslEvent::new();
                event.set_header("call-command", "execute");
                event.set_header("execute-app-name", app.clone());

                if let Some(args) = args {
                    event.set_header("execute-app-arg", args.clone());
                }

                EslCommand::SendMsg {
                    uuid: uuid.clone(),
                    event,
                }
                .to_wire_format()
            }
            EslCommand::Exit => Ok(Self::format_simple_command("exit", &[])),
            EslCommand::Log { level } => {
                validate_no_newlines(level, "log level")?;
                Ok(Self::format_simple_command("log", &[level]))
            }
            EslCommand::NoLog => Ok(Self::format_simple_command("nolog", &[])),
            EslCommand::NoOp => Ok(Self::format_simple_command("noop", &[])),
            EslCommand::SendEvent { event } => {
                let event_name = event
                    .event_type()
                    .map(|t| t.to_string())
                    .or_else(|| {
                        event
                            .header("Event-Name")
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_else(|| "CUSTOM".to_string());

                let mut builder = CommandBuilder::new(&format!("sendevent {}", event_name));

                for (key, value) in event.headers() {
                    builder = builder.header(key, value)?;
                }

                if let Some(body) = event.body() {
                    builder = builder.body(body);
                }

                Ok(builder.build())
            }
            EslCommand::MyEvents { format, uuid } => {
                validate_no_newlines(format, "myevents format")?;
                if let Some(u) = uuid {
                    validate_no_newlines(u, "myevents uuid")?;
                }
                Ok(match uuid {
                    Some(u) => Self::format_simple_command("myevents", &[u, format]),
                    None => Self::format_simple_command("myevents", &[format]),
                })
            }
            EslCommand::Linger { timeout } => Ok(match timeout {
                Some(n) => Self::format_simple_command("linger", &[&n.to_string()]),
                None => Self::format_simple_command("linger", &[]),
            }),
            EslCommand::NoLinger => Ok(Self::format_simple_command("nolinger", &[])),
            EslCommand::Resume => Ok(Self::format_simple_command("resume", &[])),
            EslCommand::NixEvent { events } => {
                validate_no_newlines(events, "nixevent list")?;
                Ok(Self::format_simple_command("nixevent", &[events]))
            }
            EslCommand::NoEvents => Ok(Self::format_simple_command("noevents", &[])),
            EslCommand::FilterDelete { header, value } => {
                validate_no_newlines(header, "filter delete header")?;
                if let Some(v) = value {
                    validate_no_newlines(v, "filter delete value")?;
                }
                if header == "all" {
                    Ok(Self::format_simple_command("filter", &["delete", "all"]))
                } else {
                    Ok(match value {
                        Some(v) => Self::format_simple_command("filter", &["delete", header, v]),
                        None => Self::format_simple_command("filter", &["delete", header]),
                    })
                }
            }
            EslCommand::DivertEvents { on } => {
                let arg = if *on { "on" } else { "off" };
                Ok(Self::format_simple_command("divert_events", &[arg]))
            }
            EslCommand::GetVar { name } => {
                validate_no_newlines(name, "getvar name")?;
                Ok(Self::format_simple_command("getvar", &[name]))
            }
            EslCommand::Connect => Ok(Self::format_simple_command("connect", &[])),
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
            .unwrap()
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
        assert_eq!(
            auth.to_wire_format()
                .unwrap(),
            "auth test\n\n"
        );

        let api = EslCommand::Api {
            command: "status".to_string(),
        };
        assert_eq!(
            api.to_wire_format()
                .unwrap(),
            "api status\n\n"
        );

        let events = EslCommand::Events {
            format: "plain".to_string(),
            events: "ALL".to_string(),
        };
        assert_eq!(
            events
                .to_wire_format()
                .unwrap(),
            "event plain ALL\n\n"
        );
    }

    #[test]
    fn test_app_commands() {
        use crate::app::dptools::AppCommand;

        let answer = AppCommand::answer()
            .to_wire_format()
            .unwrap();
        assert!(answer.contains("execute-app-name: answer"));

        let hangup = AppCommand::hangup(Some("NORMAL_CLEARING"))
            .to_wire_format()
            .unwrap();
        assert!(hangup.contains("execute-app-name: hangup"));
        assert!(hangup.contains("execute-app-arg: NORMAL_CLEARING"));
    }

    #[test]
    fn test_myevents_wire_format() {
        let cmd = EslCommand::MyEvents {
            format: "plain".to_string(),
            uuid: None,
        };
        assert_eq!(
            cmd.to_wire_format()
                .unwrap(),
            "myevents plain\n\n"
        );
    }

    #[test]
    fn test_myevents_uuid_wire_format() {
        let cmd = EslCommand::MyEvents {
            format: "json".to_string(),
            uuid: Some("abc-123".to_string()),
        };
        assert_eq!(
            cmd.to_wire_format()
                .unwrap(),
            "myevents abc-123 json\n\n"
        );
    }

    #[test]
    fn test_linger_wire_format() {
        let cmd = EslCommand::Linger { timeout: None };
        assert_eq!(
            cmd.to_wire_format()
                .unwrap(),
            "linger\n\n"
        );
    }

    #[test]
    fn test_linger_timeout_wire_format() {
        let cmd = EslCommand::Linger { timeout: Some(600) };
        assert_eq!(
            cmd.to_wire_format()
                .unwrap(),
            "linger 600\n\n"
        );
    }

    #[test]
    fn test_nolinger_wire_format() {
        let cmd = EslCommand::NoLinger;
        assert_eq!(
            cmd.to_wire_format()
                .unwrap(),
            "nolinger\n\n"
        );
    }

    #[test]
    fn test_resume_wire_format() {
        let cmd = EslCommand::Resume;
        assert_eq!(
            cmd.to_wire_format()
                .unwrap(),
            "resume\n\n"
        );
    }

    #[test]
    fn test_sendevent_wire_format() {
        let mut event = EslEvent::with_type(EslEventType::Custom);
        event.set_header("Event-Name", "CUSTOM");
        event.set_header("Event-Subclass", "my::test_event");

        let cmd = EslCommand::SendEvent { event };
        let wire = cmd
            .to_wire_format()
            .unwrap();

        assert!(wire.starts_with("sendevent CUSTOM\n"));
        assert!(wire.contains("Event-Name: CUSTOM\n"));
        assert!(wire.contains("Event-Subclass: my::test_event\n"));
        assert!(wire.ends_with("\n\n"));
    }

    #[test]
    fn test_sendevent_wire_format_with_body() {
        let mut event = EslEvent::with_type(EslEventType::Custom);
        event.set_header("Event-Name", "CUSTOM");
        event.set_body("hello world".to_string());

        let cmd = EslCommand::SendEvent { event };
        let wire = cmd
            .to_wire_format()
            .unwrap();

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
            cmd.to_wire_format()
                .unwrap(),
            "nixevent CHANNEL_CREATE CHANNEL_DESTROY\n\n"
        );
    }

    #[test]
    fn test_noevents_wire_format() {
        let cmd = EslCommand::NoEvents;
        assert_eq!(
            cmd.to_wire_format()
                .unwrap(),
            "noevents\n\n"
        );
    }

    #[test]
    fn test_filter_delete_wire_format() {
        let cmd = EslCommand::FilterDelete {
            header: "Event-Name".to_string(),
            value: None,
        };
        assert_eq!(
            cmd.to_wire_format()
                .unwrap(),
            "filter delete Event-Name\n\n"
        );
    }

    #[test]
    fn test_filter_delete_value_wire_format() {
        let cmd = EslCommand::FilterDelete {
            header: "Event-Name".to_string(),
            value: Some("CHANNEL_CREATE".to_string()),
        };
        assert_eq!(
            cmd.to_wire_format()
                .unwrap(),
            "filter delete Event-Name CHANNEL_CREATE\n\n"
        );
    }

    #[test]
    fn test_filter_delete_all_wire_format() {
        let cmd = EslCommand::FilterDelete {
            header: "all".to_string(),
            value: None,
        };
        assert_eq!(
            cmd.to_wire_format()
                .unwrap(),
            "filter delete all\n\n"
        );
    }

    #[test]
    fn test_divert_events_wire_format() {
        let cmd_on = EslCommand::DivertEvents { on: true };
        assert_eq!(
            cmd_on
                .to_wire_format()
                .unwrap(),
            "divert_events on\n\n"
        );

        let cmd_off = EslCommand::DivertEvents { on: false };
        assert_eq!(
            cmd_off
                .to_wire_format()
                .unwrap(),
            "divert_events off\n\n"
        );
    }

    #[test]
    fn test_getvar_wire_format() {
        let cmd = EslCommand::GetVar {
            name: "caller_id_name".to_string(),
        };
        assert_eq!(
            cmd.to_wire_format()
                .unwrap(),
            "getvar caller_id_name\n\n"
        );
    }

    #[test]
    fn test_connect_wire_format() {
        let cmd = EslCommand::Connect;
        assert_eq!(
            cmd.to_wire_format()
                .unwrap(),
            "connect\n\n"
        );
    }

    #[test]
    fn test_sendevent_no_event_type() {
        let mut event = EslEvent::new();
        event.set_header("Event-Name", "CUSTOM");

        let cmd = EslCommand::SendEvent { event };
        let wire = cmd
            .to_wire_format()
            .unwrap();
        assert!(wire.starts_with("sendevent CUSTOM\n"));

        let bare_event = EslEvent::new();
        let cmd2 = EslCommand::SendEvent { event: bare_event };
        let wire2 = cmd2
            .to_wire_format()
            .unwrap();
        assert!(wire2.starts_with("sendevent CUSTOM\n"));
    }

    #[test]
    fn test_newline_injection_rejected() {
        let api = EslCommand::Api {
            command: "status\n\nevent plain ALL".to_string(),
        };
        assert!(api
            .to_wire_format()
            .is_err());

        let auth = EslCommand::Auth {
            password: "test\napi status".to_string(),
        };
        assert!(auth
            .to_wire_format()
            .is_err());

        let filter = EslCommand::Filter {
            header: "Event-Name\r\n".to_string(),
            value: "CHANNEL_CREATE".to_string(),
        };
        assert!(filter
            .to_wire_format()
            .is_err());
    }

    #[test]
    fn test_debug_redacts_password() {
        let auth = EslCommand::Auth {
            password: "secret".to_string(),
        };
        let debug_str = format!("{:?}", auth);
        assert!(!debug_str.contains("secret"));
        assert!(debug_str.contains("REDACTED"));

        let user_auth = EslCommand::UserAuth {
            user: "admin@default".to_string(),
            password: "secret".to_string(),
        };
        let debug_str = format!("{:?}", user_auth);
        assert!(!debug_str.contains("secret"));
        assert!(debug_str.contains("admin@default"));
        assert!(debug_str.contains("REDACTED"));
    }

    #[test]
    fn test_reply_status_ok() {
        let headers: HashMap<String, String> =
            [("Reply-Text".into(), "+OK accepted".into())].into();
        let resp = EslResponse::new(headers, None);
        assert_eq!(resp.reply_status(), ReplyStatus::Ok);
        assert!(resp.is_success());
        assert!(resp
            .into_result()
            .is_ok());
    }

    #[test]
    fn test_reply_status_ok_prefix_only() {
        let headers: HashMap<String, String> = [("Reply-Text".into(), "+OK".into())].into();
        let resp = EslResponse::new(headers, None);
        assert_eq!(resp.reply_status(), ReplyStatus::Ok);
        assert!(resp.is_success());
    }

    #[test]
    fn test_reply_status_empty() {
        let headers: HashMap<String, String> = [("Reply-Text".into(), String::new())].into();
        let resp = EslResponse::new(headers, None);
        assert_eq!(resp.reply_status(), ReplyStatus::Ok);
        assert!(resp.is_success());
    }

    #[test]
    fn test_reply_status_missing_header() {
        let resp = EslResponse::new(HashMap::new(), None);
        assert_eq!(resp.reply_status(), ReplyStatus::Ok);
        assert!(resp.is_success());
    }

    #[test]
    fn test_reply_status_err() {
        let headers: HashMap<String, String> =
            [("Reply-Text".into(), "-ERR invalid command".into())].into();
        let resp = EslResponse::new(headers, None);
        assert_eq!(resp.reply_status(), ReplyStatus::Err);
        assert!(!resp.is_success());
        let err = resp
            .into_result()
            .unwrap_err();
        assert!(
            matches!(err, EslError::CommandFailed { ref reply_text } if reply_text == "-ERR invalid command")
        );
    }

    #[test]
    fn test_reply_status_err_bare() {
        let headers: HashMap<String, String> = [("Reply-Text".into(), "-ERR".into())].into();
        let resp = EslResponse::new(headers, None);
        assert_eq!(resp.reply_status(), ReplyStatus::Err);
        assert!(!resp.is_success());
    }

    #[test]
    fn test_reply_status_other_getvar() {
        let headers: HashMap<String, String> =
            [("Reply-Text".into(), "sip_from_user".into())].into();
        let resp = EslResponse::new(headers, None);
        assert_eq!(resp.reply_status(), ReplyStatus::Other);
        assert!(!resp.is_success());
        let err = resp
            .into_result()
            .unwrap_err();
        assert!(
            matches!(err, EslError::UnexpectedReply { ref reply_text } if reply_text == "sip_from_user")
        );
    }

    #[test]
    fn test_reply_status_other_random() {
        let headers: HashMap<String, String> =
            [("Reply-Text".into(), "something unexpected".into())].into();
        let resp = EslResponse::new(headers, None);
        assert_eq!(resp.reply_status(), ReplyStatus::Other);
        assert!(!resp.is_success());
    }

    #[test]
    fn test_header_newline_rejected() {
        let result = CommandBuilder::new("test").header("X-Bad\n", "value");
        assert!(result.is_err());

        let result = CommandBuilder::new("test").header("X-Key", "bad\nvalue");
        assert!(result.is_err());
    }
}
