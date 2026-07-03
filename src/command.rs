//! Command execution and response handling

use crate::{
    constants::{HEADER_REPLY_TEXT, HEADER_TERMINATOR, LINE_TERMINATOR},
    error::{EslError, EslResult},
    event::EslEvent,
    headers::{case_alias_key, EventHeader},
    lookup::HeaderLookup,
    LossyValues,
};
pub(crate) use freeswitch_types::wire_safety::contains_wire_terminator;
use indexmap::IndexMap;
use std::borrow::Cow;
use std::fmt;
use std::time::Duration;

/// Wraps a string so `Debug` prints `[REDACTED]` instead of the value.
///
/// Used for password fields in [`EslCommand`] to prevent accidental exposure
/// in debug logs. The inner value is not accessible from external crates.
#[derive(Clone)]
pub struct Secret(pub(crate) String);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// Parse a FreeSWITCH API response body into a result.
///
/// FreeSWITCH API commands return results in varying formats:
///
/// - **Action commands** (`originate`, `uuid_kill`, …) prefix success
///   with `+OK` -- this function strips it and returns the payload.
/// - **Query commands** (`show channels as json`, `uuid_dump`, …) return
///   raw data with no prefix -- returned as-is.
/// - **Errors** (`-ERR …`, `-USAGE: …`) produce [`EslError::CommandFailed`].
///
/// A trailing `\n` (the standard FreeSWITCH API output terminator) is
/// stripped; all other content is preserved verbatim.
/// Returns [`EslError::ProtocolError`] if the body is empty after
/// stripping.
///
/// This is the same parser used by [`EslResponse::api_result`]. Use it
/// directly on [`EslEvent::body`](freeswitch_types::EslEvent::body) for
/// `BACKGROUND_JOB` results:
///
/// ```rust,no_run
/// # use freeswitch_esl_tokio::{parse_api_body, EslEvent, HeaderLookup};
/// # fn example(event: &EslEvent) {
/// if let Some(body) = event.body() {
///     match parse_api_body(body) {
///         Ok(data) => println!("result: {}", data),
///         Err(e) => eprintln!("command failed: {}", e),
///     }
/// }
/// # }
/// ```
pub fn parse_api_body(body: &str) -> EslResult<&str> {
    // Strip the trailing \n that FreeSWITCH appends to API output.
    let body = body
        .strip_suffix('\n')
        .unwrap_or(body);
    let body = body
        .strip_suffix('\r')
        .unwrap_or(body);
    if body.is_empty() {
        return Err(EslError::protocol_error("api response body is empty"));
    }
    if let Some(rest) = body.strip_prefix("+OK") {
        Ok(rest
            .strip_prefix(' ')
            .unwrap_or(rest))
    } else if body.starts_with("-ERR") || body.starts_with("-USAGE") {
        Err(EslError::CommandFailed {
            reply_text: body.to_string(),
        })
    } else {
        Ok(body)
    }
}

/// Validate that a user-provided string contains no newline characters.
///
/// ESL commands are line-delimited; embedded newlines would allow injection
/// of arbitrary protocol commands.
fn validate_no_newlines(s: &str, context: &str) -> EslResult<()> {
    if contains_wire_terminator(s) {
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
#[non_exhaustive]
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EslResponse {
    headers: IndexMap<String, String>,
    body: Option<String>,
    status: ReplyStatus,
    /// Lowercase alias map for case-insensitive lookup of dash-cased FS
    /// framing headers (`Reply-Text`, `Content-Type`, `Job-UUID`, ...).
    /// Keys containing `_` (channel variables and `sip_h_*` / `sip_i_*`
    /// passthrough headers) are intentionally excluded so SIP wire
    /// casing is preserved — same rule as `normalize_header_key`.
    case_index: IndexMap<String, String>,
    /// Header keys whose percent-decoded value was not valid UTF-8 and was
    /// decoded lossily. Populated for serialized responses such as the
    /// outbound `connect` channel data; empty in the normal case.
    lossy_values: LossyValues,
}

impl EslResponse {
    /// `ReplyStatus` is derived from the `Reply-Text` header. Header
    /// lookups via [`header()`](Self::header) are case-insensitive for
    /// FS framing headers but case-sensitive for `variable_*`,
    /// `sip_h_*`, and `sip_i_*` keys, which must preserve original SIP
    /// wire casing.
    pub fn new(headers: IndexMap<String, String>, body: Option<String>) -> Self {
        let case_index = headers
            .keys()
            .filter_map(|k| case_alias_key(k).map(|alias| (alias, k.clone())))
            .collect();
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
            case_index,
            lossy_values: LossyValues::default(),
        }
    }

    /// Attach the lossy-decode signal recorded while parsing the response
    /// headers (used by [`EslMessage::into_response`](crate::EslMessage)).
    pub(crate) fn with_lossy_values(mut self, lossy_values: LossyValues) -> Self {
        self.lossy_values = lossy_values;
        self
    }

    /// Header keys whose percent-decoded value was not valid UTF-8 and was
    /// decoded lossily (U+FFFD). Each entry carries the on-wire value. Mainly
    /// relevant to the outbound `connect` response, whose channel-data values
    /// are percent-encoded by FreeSWITCH. Empty in the normal case.
    pub fn lossy_values(&self) -> &LossyValues {
        &self.lossy_values
    }

    /// `true` if Reply-Text is `+OK` or absent.
    pub fn is_success(&self) -> bool {
        self.status == ReplyStatus::Ok
    }

    /// Classification of the `Reply-Text` header.
    pub fn reply_status(&self) -> ReplyStatus {
        self.status
    }

    /// Response body (the `api/` response payload, or `bgapi` result).
    pub fn body(&self) -> Option<&str> {
        self.body
            .as_deref()
    }

    /// Look up a response header by name. Case-insensitive.
    pub fn header(&self, name: impl AsRef<str>) -> Option<&str> {
        self.lookup_header(name.as_ref())
    }

    fn lookup_header(&self, name: &str) -> Option<&str> {
        if let Some(v) = self
            .headers
            .get(name)
        {
            return Some(v.as_str());
        }
        self.case_index
            .get(&name.to_ascii_lowercase())
            .and_then(|canonical| {
                self.headers
                    .get(canonical)
            })
            .map(|s| s.as_str())
    }

    /// All response headers.
    pub fn headers(&self) -> &IndexMap<String, String> {
        &self.headers
    }

    /// Raw `Reply-Text` header value (e.g. `+OK`, `-ERR invalid command`).
    pub fn reply_text(&self) -> Option<&str> {
        self.headers
            .get(HEADER_REPLY_TEXT)
            .map(|s| s.as_str())
    }

    /// `Job-UUID` header from `bgapi` responses.
    ///
    /// FreeSWITCH returns the Job-UUID both in Reply-Text (`+OK Job-UUID: <uuid>`)
    /// and as a separate `Job-UUID` header. This reads the dedicated header.
    pub fn job_uuid(&self) -> Option<&str> {
        self.headers
            .get(EventHeader::JobUuid.as_str())
            .map(|s| s.as_str())
    }

    /// UUID of the event fired by `sendevent`.
    ///
    /// FreeSWITCH returns `+OK <event-uuid>` in the Reply-Text for
    /// `sendevent` commands. Returns `None` if the reply doesn't
    /// contain a UUID after `+OK `.
    pub fn event_uuid(&self) -> Option<&str> {
        self.reply_text()
            .and_then(|t| t.strip_prefix("+OK "))
            .filter(|s| !s.is_empty())
    }

    /// Parse the response body as an API result.
    ///
    /// FreeSWITCH `api` commands return the result in the response body.
    /// The format varies by command:
    ///
    /// - **Action commands** (`originate`, `uuid_kill`, `uuid_setvar`, …)
    ///   return `+OK <data>` on success -- this method strips the prefix
    ///   and returns the payload.
    /// - **Query commands** (`show channels as json`, `uuid_dump`,
    ///   `uuid_getvar`, `status`, …) return raw data with no prefix --
    ///   this method returns the body as-is.
    /// - **Error responses** (`-ERR <message>`, `-USAGE: <usage>`)
    ///   return [`EslError::CommandFailed`].
    ///
    /// On success, a single trailing `\n` (then a single trailing `\r`, for
    /// `\r\n` wire endings) is stripped along with the `+OK ` prefix on
    /// action commands. No leading-whitespace trimming is performed.
    ///
    /// Returns [`EslError::ProtocolError`] if the body is missing or empty.
    ///
    /// ```
    /// # use freeswitch_esl_tokio::EslResponse;
    /// # use indexmap::IndexMap;
    /// let headers = IndexMap::from([("Reply-Text".into(), "+OK".into())]);
    ///
    /// // Action command: +OK prefix stripped
    /// let resp = EslResponse::new(headers.clone(), Some("+OK d4f3a2b1-1234\n".into()));
    /// assert_eq!(resp.api_result().unwrap(), "d4f3a2b1-1234");
    ///
    /// // Query command: raw body returned as-is
    /// let resp = EslResponse::new(headers.clone(), Some(r#"{"rows":[]}"#.into()));
    /// assert_eq!(resp.api_result().unwrap(), r#"{"rows":[]}"#);
    ///
    /// // Error: Err variant
    /// let resp = EslResponse::new(headers, Some("-ERR no such channel\n".into()));
    /// assert!(resp.api_result().is_err());
    /// ```
    pub fn api_result(&self) -> EslResult<&str> {
        let body = self
            .body
            .as_deref()
            .unwrap_or("");
        parse_api_body(body)
    }

    /// Convert to result based on success status.
    ///
    /// ```
    /// # use freeswitch_esl_tokio::EslResponse;
    /// # use indexmap::IndexMap;
    /// let headers: IndexMap<String, String> = [("Reply-Text".into(), "+OK".into())].into();
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

impl freeswitch_types::sip_header::SipHeaderLookup for EslResponse {
    fn sip_header_str(&self, name: &str) -> Option<&str> {
        self.lookup_header(name)
    }

    fn call_info(
        &self,
    ) -> Result<
        Option<freeswitch_types::sip_header::UriInfo>,
        freeswitch_types::sip_header::UriInfoError,
    > {
        match self.sip_header(freeswitch_types::sip_header::SipHeader::CallInfo) {
            Some(s) => freeswitch_types::EslHeaders::parse_uri_info(s).map(Some),
            None => Ok(None),
        }
    }

    fn history_info(
        &self,
    ) -> Result<
        Option<freeswitch_types::sip_header::HistoryInfo>,
        freeswitch_types::sip_header::HistoryInfoError,
    > {
        match self.sip_header(freeswitch_types::sip_header::SipHeader::HistoryInfo) {
            Some(s) => freeswitch_types::EslHeaders::parse_history_info(s).map(Some),
            None => Ok(None),
        }
    }

    fn alert_info(
        &self,
    ) -> Result<
        Option<freeswitch_types::sip_header::UriInfo>,
        freeswitch_types::sip_header::UriInfoError,
    > {
        match self.sip_header(freeswitch_types::sip_header::SipHeader::AlertInfo) {
            Some(s) => freeswitch_types::EslHeaders::parse_uri_info(s).map(Some),
            None => Ok(None),
        }
    }
}

impl HeaderLookup for EslResponse {
    fn header_str(&self, name: &str) -> Option<&str> {
        self.lookup_header(name)
    }

    fn variable_str(&self, name: &str) -> Option<&str> {
        let key = format!("variable_{}", name);
        self.header_str(&key)
    }
}

/// Builder for custom ESL commands not covered by [`EslClient`](crate::EslClient) methods.
///
/// Produces the wire-format string including headers and optional body.
///
/// ```
/// use freeswitch_esl_tokio::CommandBuilder;
///
/// let cmd = CommandBuilder::new("mycommand")
///     .header("X-Custom", "value").unwrap()
///     .body("payload data")
///     .build();
/// assert!(cmd.starts_with("mycommand\n"));
/// assert!(cmd.contains("X-Custom: value"));
/// assert!(cmd.contains("Content-Length: 12"));
/// ```
#[derive(Debug)]
pub struct CommandBuilder {
    command: String,
    headers: IndexMap<String, String>,
    body: Option<String>,
}

impl CommandBuilder {
    /// Start building a command with the given command line.
    pub fn new(command: &str) -> Self {
        Self {
            command: command.to_string(),
            headers: IndexMap::new(),
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
            write!(result, "{}: {}{}", key, value, LINE_TERMINATOR)
                .expect("writing to String is infallible");
        }

        if let Some(body) = &self.body {
            write!(result, "Content-Length: {}{}", body.len(), LINE_TERMINATOR)
                .expect("writing to String is infallible");
            result.push_str(LINE_TERMINATOR);
            result.push_str(body);
        } else {
            result.push_str(LINE_TERMINATOR);
        }

        result
    }
}

/// Options for `sendmsg execute` commands.
///
/// Controls optional headers that modify execution behavior in outbound
/// ESL mode (socket application with `async full`).
#[derive(Debug, Clone, Default)]
pub struct ExecuteOptions {
    event_lock: bool,
    async_mode: bool,
    loops: Option<u32>,
}

impl ExecuteOptions {
    /// Create default options (no event-lock, no async, no loops).
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable the `Event-Lock` header.
    pub fn with_event_lock(mut self) -> Self {
        self.event_lock = true;
        self
    }

    /// Enable the `async` header.
    pub fn with_async(mut self) -> Self {
        self.async_mode = true;
        self
    }

    /// Set the `loops` header.
    pub fn with_loops(mut self, count: u32) -> Self {
        self.loops = Some(count);
        self
    }

    /// Lock the event queue during execution so events are serialized
    /// with the application (prevents race conditions on fast-executing apps).
    pub fn event_lock(&self) -> bool {
        self.event_lock
    }

    /// Return immediately instead of waiting for the application to finish.
    /// Only meaningful in outbound `async full` mode.
    pub fn async_mode(&self) -> bool {
        self.async_mode
    }

    /// Repeat the application N times.
    pub fn loops(&self) -> Option<u32> {
        self.loops
    }
}

/// ESL command types for the wire protocol.
///
/// Most users won't construct these directly -- use [`EslClient`](crate::EslClient)
/// methods or [`AppCommand`](crate::AppCommand) instead. This enum is public so
/// that [`send_command()`](crate::EslClient::send_command) callers can name the type.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum EslCommand {
    /// Authenticate with password.
    Auth {
        /// ESL password (from `event_socket.conf.xml`).
        password: Secret,
    },
    /// Authenticate with user and password.
    UserAuth {
        /// Username (e.g. `admin@default`).
        user: String,
        /// Password for this user.
        password: Secret,
    },
    /// Execute API command.
    Api {
        /// Full command string (e.g. `"status"`, `"sofia status"`).
        command: String,
    },
    /// Execute background API command.
    BgApi {
        /// Full command string.
        command: String,
    },
    /// Subscribe to events.
    Events {
        /// Event format (`plain`, `json`, `xml`).
        format: String,
        /// Space-separated event names or `ALL`.
        events: String,
    },
    /// Set event filters.
    Filter {
        /// Header name to filter on.
        header: String,
        /// Required header value.
        value: String,
    },
    /// Send message to channel.
    SendMsg {
        /// Target channel UUID (omit in outbound mode).
        uuid: Option<String>,
        /// Event containing sendmsg headers and optional body.
        event: EslEvent,
    },
    /// Execute application on channel.
    Execute {
        /// Application name (e.g. `"answer"`, `"playback"`).
        app: String,
        /// Application arguments.
        args: Option<String>,
        /// Target channel UUID (omit in outbound mode).
        uuid: Option<String>,
        /// Optional execution flags (event-lock, async, loops).
        options: ExecuteOptions,
    },
    /// Exit/logout.
    Exit,
    /// Enable log forwarding at the given level.
    Log {
        /// Log level (e.g. `"debug"`, `"warning"`).
        level: String,
    },
    /// Disable log forwarding.
    NoLog,
    /// No operation / keepalive.
    NoOp,
    /// Fire an event into FreeSWITCH's event bus.
    SendEvent {
        /// Event to fire.
        event: EslEvent,
    },
    /// Subscribe to session events (outbound: no uuid, inbound: with uuid).
    MyEvents {
        /// Event format (`plain`, `json`, `xml`).
        format: String,
        /// Channel UUID for inbound mode; omit for outbound.
        uuid: Option<String>,
    },
    /// Keep socket open after channel hangup.
    Linger {
        /// Linger timeout, or `None` for indefinite.
        timeout: Option<Duration>,
    },
    /// Cancel linger mode.
    NoLinger,
    /// Resume dialplan execution on socket disconnect.
    Resume,
    /// Unsubscribe from specific events.
    NixEvent {
        /// Space-separated event names.
        events: String,
    },
    /// Unsubscribe from all events.
    NoEvents,
    /// Remove event filters.
    FilterDelete {
        /// Header name to remove filter from, or `"all"`.
        header: String,
        /// Specific value to remove, or `None` for all values.
        value: Option<String>,
    },
    /// Redirect session events to ESL (outbound mode).
    DivertEvents {
        /// `true` to enable, `false` to disable.
        on: bool,
    },
    /// Read a channel variable (outbound mode).
    GetVar {
        /// Variable name.
        name: String,
    },
    /// Request channel data in outbound mode.
    Connect,
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
                validate_no_newlines(&password.0, "password")?;
                Ok(Self::format_simple_command("auth", &[&password.0]))
            }
            EslCommand::UserAuth { user, password } => {
                validate_no_newlines(user, "user")?;
                validate_no_newlines(&password.0, "password")?;
                Ok(Self::format_simple_command(
                    "userauth",
                    &[&format!("{}:{}", user, password.0)],
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
            EslCommand::Execute {
                app,
                args,
                uuid,
                options,
            } => {
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

                if options.event_lock {
                    event.set_header("event-lock", "true");
                }
                if options.async_mode {
                    event.set_header("async", "true");
                }
                if let Some(loops) = options.loops {
                    event.set_header("loops", loops.to_string());
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
                            .header(EventHeader::EventName)
                            .map(|s| s.to_string())
                    })
                    .ok_or_else(|| {
                        EslError::protocol_error(
                            "sendevent requires Event-Name header or event_type set on the EslEvent",
                        )
                    })?;
                validate_no_newlines(&event_name, "sendevent event name")?;

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
                Some(d) => Self::format_simple_command(
                    "linger",
                    &[&d.as_secs()
                        .to_string()],
                ),
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

    /// Return a log-safe version of the wire-format string.
    ///
    /// Passwords are replaced with `[REDACTED]`. The mandatory `\n\n` wire
    /// terminator is stripped so the result fits on one log line; all other
    /// content is preserved verbatim. Non-sensitive commands borrow the input
    /// (zero allocation).
    pub fn redact_wire<'a>(&self, wire: &'a str) -> Cow<'a, str> {
        match self {
            EslCommand::Auth { .. } => Cow::Owned("auth [REDACTED]".into()),
            EslCommand::UserAuth { user, .. } => {
                Cow::Owned(format!("userauth {}:[REDACTED]", user))
            }
            _ => Cow::Borrowed(
                wire.strip_suffix(HEADER_TERMINATOR)
                    .unwrap_or(wire),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EslEventType;

    #[test]
    fn esl_response_header_lookup_is_case_insensitive() {
        let mut headers = IndexMap::new();
        headers.insert("Reply-Text".to_string(), "+OK".to_string());
        let r = EslResponse::new(headers, None);
        assert_eq!(r.header("Reply-Text"), Some("+OK"));
        assert_eq!(r.header("reply-text"), Some("+OK"));
        assert_eq!(r.header("REPLY-TEXT"), Some("+OK"));
    }

    #[test]
    fn esl_response_underscored_keys_preserve_case() {
        // variable_*, sip_h_*, sip_i_* must preserve original SIP wire
        // casing — the lowercase fallback must not match these keys, or
        // distinct headers like X-Foo and X-foo would collide.
        let mut headers = IndexMap::new();
        headers.insert(
            "variable_sip_h_X-MixedCase-Hdr".to_string(),
            "value".to_string(),
        );
        headers.insert("variable_MyVar".to_string(), "vv".to_string());
        let r = EslResponse::new(headers, None);

        // Exact case hits.
        assert_eq!(r.header("variable_sip_h_X-MixedCase-Hdr"), Some("value"));
        assert_eq!(r.header("variable_MyVar"), Some("vv"));

        // Wrong case must NOT resolve via the lowercase fallback.
        assert_eq!(r.header("variable_sip_h_x-mixedcase-hdr"), None);
        assert_eq!(r.header("VARIABLE_SIP_H_X-MIXEDCASE-HDR"), None);
        assert_eq!(r.header("variable_myvar"), None);
    }

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
            password: Secret("test".to_string()),
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
        assert!(answer.contains("Execute-App-Name: answer"));

        let hangup = AppCommand::hangup(Some(crate::channel::HangupCause::NormalClearing))
            .to_wire_format()
            .unwrap();
        assert!(hangup.contains("Execute-App-Name: hangup"));
        assert!(hangup.contains("Execute-App-Arg: NORMAL_CLEARING"));
    }

    #[test]
    fn test_execute_with_options_wire_format() {
        let cmd = EslCommand::Execute {
            app: "playback".to_string(),
            args: Some("tone_stream://%(200,100,440)".to_string()),
            uuid: Some("abc-123".to_string()),
            options: ExecuteOptions::new()
                .with_event_lock()
                .with_async()
                .with_loops(3),
        };
        let wire = cmd
            .to_wire_format()
            .unwrap();
        assert!(wire.contains("Event-Lock: true"));
        assert!(wire.contains("Async: true"));
        assert!(wire.contains("Loops: 3"));
        assert!(wire.contains("Execute-App-Name: playback"));
    }

    #[test]
    fn test_execute_default_options_no_extra_headers() {
        let cmd = EslCommand::Execute {
            app: "answer".to_string(),
            args: None,
            uuid: None,
            options: ExecuteOptions::default(),
        };
        let wire = cmd
            .to_wire_format()
            .unwrap();
        assert!(!wire.contains("event-lock"));
        assert!(!wire.contains("async"));
        assert!(!wire.contains("loops"));
    }

    #[test]
    fn test_event_uuid_from_sendevent_reply() {
        let headers: IndexMap<String, String> = [(
            "Reply-Text".into(),
            "+OK 7d54c1e6-4a31-11e9-b1e3-001a4a160100".into(),
        )]
        .into();
        let resp = EslResponse::new(headers, None);
        assert_eq!(
            resp.event_uuid(),
            Some("7d54c1e6-4a31-11e9-b1e3-001a4a160100")
        );
    }

    #[test]
    fn test_event_uuid_none_for_plain_ok() {
        let headers: IndexMap<String, String> = [("Reply-Text".into(), "+OK".into())].into();
        let resp = EslResponse::new(headers, None);
        assert_eq!(resp.event_uuid(), None);
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
        let cmd = EslCommand::Linger {
            timeout: Some(Duration::from_secs(600)),
        };
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
    fn test_sendevent_event_name_only_no_typed_variant() {
        // Event-Name set as a raw header (no event_type) is accepted —
        // the wire-format serializer falls back to the header.
        let mut event = EslEvent::new();
        event.set_header("Event-Name", "CUSTOM");

        let cmd = EslCommand::SendEvent { event };
        let wire = cmd
            .to_wire_format()
            .unwrap();
        assert!(wire.starts_with("sendevent CUSTOM\n"));
    }

    #[test]
    fn test_sendevent_without_event_name_errors() {
        // Bare EslEvent with no event_type and no Event-Name header is
        // a hard error — we refuse to silently mislabel as "CUSTOM".
        let bare_event = EslEvent::new();
        let cmd = EslCommand::SendEvent { event: bare_event };
        let err = cmd
            .to_wire_format()
            .unwrap_err();
        assert!(matches!(err, EslError::ProtocolError { .. }));
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
            password: Secret("test\napi status".to_string()),
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
            password: Secret("secret".to_string()),
        };
        let debug_str = format!("{:?}", auth);
        assert!(!debug_str.contains("secret"));
        assert!(debug_str.contains("REDACTED"));

        let user_auth = EslCommand::UserAuth {
            user: "admin@default".to_string(),
            password: Secret("secret".to_string()),
        };
        let debug_str = format!("{:?}", user_auth);
        assert!(!debug_str.contains("secret"));
        assert!(debug_str.contains("admin@default"));
        assert!(debug_str.contains("REDACTED"));
    }

    #[test]
    fn test_reply_status_ok() {
        let headers: IndexMap<String, String> =
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
        let headers: IndexMap<String, String> = [("Reply-Text".into(), "+OK".into())].into();
        let resp = EslResponse::new(headers, None);
        assert_eq!(resp.reply_status(), ReplyStatus::Ok);
        assert!(resp.is_success());
    }

    #[test]
    fn test_reply_status_empty() {
        let headers: IndexMap<String, String> = [("Reply-Text".into(), String::new())].into();
        let resp = EslResponse::new(headers, None);
        assert_eq!(resp.reply_status(), ReplyStatus::Ok);
        assert!(resp.is_success());
    }

    #[test]
    fn test_reply_status_missing_header() {
        let resp = EslResponse::new(IndexMap::new(), None);
        assert_eq!(resp.reply_status(), ReplyStatus::Ok);
        assert!(resp.is_success());
    }

    #[test]
    fn test_reply_status_err() {
        let headers: IndexMap<String, String> =
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
        let headers: IndexMap<String, String> = [("Reply-Text".into(), "-ERR".into())].into();
        let resp = EslResponse::new(headers, None);
        assert_eq!(resp.reply_status(), ReplyStatus::Err);
        assert!(!resp.is_success());
    }

    #[test]
    fn test_reply_status_other_getvar() {
        let headers: IndexMap<String, String> =
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
        let headers: IndexMap<String, String> =
            [("Reply-Text".into(), "something unexpected".into())].into();
        let resp = EslResponse::new(headers, None);
        assert_eq!(resp.reply_status(), ReplyStatus::Other);
        assert!(!resp.is_success());
    }

    // --- T6: user_auth wire format ---

    #[test]
    fn test_user_auth_wire_format() {
        let cmd = EslCommand::UserAuth {
            user: "admin@default".to_string(),
            password: Secret("secret123".to_string()),
        };
        assert_eq!(
            cmd.to_wire_format()
                .unwrap(),
            "userauth admin@default:secret123\n\n"
        );
    }

    #[test]
    fn test_user_auth_newline_in_user_rejected() {
        let cmd = EslCommand::UserAuth {
            user: "admin\n@default".to_string(),
            password: Secret("pass".to_string()),
        };
        assert!(cmd
            .to_wire_format()
            .is_err());
    }

    #[test]
    fn test_user_auth_newline_in_password_rejected() {
        let cmd = EslCommand::UserAuth {
            user: "admin@default".to_string(),
            password: Secret("pass\nword".to_string()),
        };
        assert!(cmd
            .to_wire_format()
            .is_err());
    }

    // --- T6: redact_wire for SendMsg/SendEvent ---

    #[test]
    fn test_redact_wire_sendmsg() {
        let mut event = EslEvent::new();
        event.set_header("call-command", "execute");
        event.set_header("execute-app-name", "answer");
        let cmd = EslCommand::SendMsg {
            uuid: Some("abc-123".to_string()),
            event,
        };
        let wire = cmd
            .to_wire_format()
            .unwrap();
        let redacted = cmd.redact_wire(&wire);
        // SendMsg is not sensitive, wire content is preserved (minus terminator)
        assert!(redacted.contains("sendmsg"));
        assert!(redacted.contains("Execute-App-Name: answer"));
        assert!(!redacted.ends_with("\n\n"));
    }

    #[test]
    fn test_redact_wire_sendevent() {
        let mut event = EslEvent::with_type(EslEventType::Custom);
        event.set_header("Event-Name", "CUSTOM");
        event.set_header("Event-Subclass", "test::redact");
        let cmd = EslCommand::SendEvent { event };
        let wire = cmd
            .to_wire_format()
            .unwrap();
        let redacted = cmd.redact_wire(&wire);
        assert!(redacted.contains("sendevent"));
        assert!(!redacted.ends_with("\n\n"));
    }

    #[test]
    fn test_redact_wire_auth() {
        let cmd = EslCommand::Auth {
            password: Secret("secret".to_string()),
        };
        let wire = cmd
            .to_wire_format()
            .unwrap();
        let redacted = cmd.redact_wire(&wire);
        assert!(!redacted.contains("secret"));
        assert!(redacted.contains("REDACTED"));
    }

    #[test]
    fn test_redact_wire_user_auth() {
        let cmd = EslCommand::UserAuth {
            user: "admin@default".to_string(),
            password: Secret("secret".to_string()),
        };
        let wire = cmd
            .to_wire_format()
            .unwrap();
        let redacted = cmd.redact_wire(&wire);
        assert!(!redacted.contains("secret"));
        assert!(redacted.contains("admin@default"));
        assert!(redacted.contains("REDACTED"));
    }

    #[test]
    fn test_header_newline_rejected() {
        let result = CommandBuilder::new("test").header("X-Bad\n", "value");
        assert!(result.is_err());

        let result = CommandBuilder::new("test").header("X-Key", "bad\nvalue");
        assert!(result.is_err());
    }

    #[test]
    fn test_response_header_lookup_trait() {
        use crate::headers::EventHeader;
        use crate::lookup::HeaderLookup;

        let headers: IndexMap<String, String> = [
            ("Reply-Text".into(), "+OK".into()),
            ("Channel-Name".into(), "sofia/internal/1000@test".into()),
            ("Channel-State".into(), "CS_EXECUTE".into()),
            ("variable_sip_call_id".into(), "abc-123".into()),
        ]
        .into();
        let resp = EslResponse::new(headers, None);

        assert_eq!(
            resp.header(EventHeader::ChannelName),
            Some("sofia/internal/1000@test")
        );
        assert_eq!(
            resp.channel_state()
                .unwrap(),
            Some(crate::channel::ChannelState::CsExecute)
        );
        assert_eq!(resp.variable_str("sip_call_id"), Some("abc-123"));
        assert_eq!(resp.variable_str("nonexistent"), None);
    }

    // --- parse_api_body() tests ---

    #[test]
    fn parse_api_body_ok_with_data() {
        let data = parse_api_body("+OK d4f3a2b1-1234-5678-abcd-ef0123456789\n").unwrap();
        assert_eq!(data, "d4f3a2b1-1234-5678-abcd-ef0123456789");
    }

    #[test]
    fn parse_api_body_ok_no_data() {
        let data = parse_api_body("+OK\n").unwrap();
        assert_eq!(data, "");
    }

    #[test]
    fn parse_api_body_ok_bare() {
        let data = parse_api_body("+OK").unwrap();
        assert_eq!(data, "");
    }

    #[test]
    fn parse_api_body_err() {
        let err = parse_api_body("-ERR invalid command\n").unwrap_err();
        assert!(matches!(
            err,
            EslError::CommandFailed { ref reply_text } if reply_text == "-ERR invalid command"
        ));
    }

    #[test]
    fn parse_api_body_usage() {
        let err = parse_api_body("-USAGE: originate <call_url> <exten>\n").unwrap_err();
        assert!(matches!(err, EslError::CommandFailed { .. }));
    }

    #[test]
    fn parse_api_body_raw_json() {
        let data = parse_api_body(r#"{"row_count":2,"rows":[]}"#).unwrap();
        assert_eq!(data, r#"{"row_count":2,"rows":[]}"#);
    }

    #[test]
    fn parse_api_body_raw_value() {
        let data = parse_api_body("hello_world\n").unwrap();
        assert_eq!(data, "hello_world");
    }

    #[test]
    fn parse_api_body_raw_multiline() {
        let dump = "Variable-Name: value\nOther-Name: other\n";
        let data = parse_api_body(dump).unwrap();
        assert_eq!(data, "Variable-Name: value\nOther-Name: other");
    }

    #[test]
    fn parse_api_body_empty() {
        let err = parse_api_body("").unwrap_err();
        assert!(matches!(err, EslError::ProtocolError { .. }));
    }

    #[test]
    fn parse_api_body_whitespace_only() {
        // Only trailing \n is stripped; other whitespace is preserved verbatim
        let data = parse_api_body("  \n").unwrap();
        assert_eq!(data, "  ");
    }

    // --- api_result() tests ---

    #[test]
    fn api_result_delegates_to_parse() {
        let headers: IndexMap<String, String> = [("Reply-Text".into(), "+OK".into())].into();
        let resp = EslResponse::new(headers, Some("+OK uuid-123\n".into()));
        assert_eq!(
            resp.api_result()
                .unwrap(),
            "uuid-123"
        );
    }

    #[test]
    fn api_result_no_body() {
        let headers: IndexMap<String, String> = [("Reply-Text".into(), "+OK".into())].into();
        let resp = EslResponse::new(headers, None);
        assert!(matches!(
            resp.api_result()
                .unwrap_err(),
            EslError::ProtocolError { .. }
        ));
    }

    // --- Finding 2: EslResponse SipHeaderLookup ARRAY encoding ---

    #[test]
    fn esl_response_call_info_array_encoding() {
        use freeswitch_types::sip_header::SipHeaderLookup;

        let headers: IndexMap<String, String> = [(
            "Call-Info".into(),
            "ARRAY::<urn:emergency:uid:callid:abc>;purpose=emergency-CallId\
             |:<urn:emergency:uid:incidentid:def>;purpose=emergency-IncidentId"
                .into(),
        )]
        .into();
        let resp = EslResponse::new(headers, None);
        let ci = resp
            .call_info()
            .expect("should parse")
            .expect("should be present");
        assert_eq!(
            ci.entries()
                .len(),
            2,
            "ARRAY:: entries should expand"
        );
    }

    #[test]
    fn esl_response_call_info_plain_value_unchanged() {
        use freeswitch_types::sip_header::SipHeaderLookup;

        let headers: IndexMap<String, String> = [(
            "Call-Info".into(),
            "<sip:pbx.example.com>;purpose=icon".into(),
        )]
        .into();
        let resp = EslResponse::new(headers, None);
        let ci = resp
            .call_info()
            .expect("plain value should parse")
            .expect("should be present");
        assert_eq!(
            ci.entries()
                .len(),
            1
        );
    }

    #[test]
    fn esl_response_alert_info_array_encoding() {
        use freeswitch_types::sip_header::SipHeaderLookup;

        let headers: IndexMap<String, String> = [(
            "Alert-Info".into(),
            "ARRAY::<http://pbx.example.com/bell.wav>|:<http://pbx.example.com/siren.wav>".into(),
        )]
        .into();
        let resp = EslResponse::new(headers, None);
        let ai = resp
            .alert_info()
            .expect("should parse")
            .expect("should be present");
        assert_eq!(
            ai.entries()
                .len(),
            2,
            "ARRAY:: entries should expand"
        );
    }

    #[test]
    fn test_filter_delete_all_variant_wire_format() {
        let cmd = EslCommand::FilterDeleteAll;
        assert_eq!(
            cmd.to_wire_format()
                .unwrap(),
            "filter delete all\n\n"
        );
    }
}
