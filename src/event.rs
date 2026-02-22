//! ESL event types and structures

use crate::constants::{
    HEADER_ANSWER_STATE, HEADER_CALLER_UUID, HEADER_CALL_DIRECTION, HEADER_CHANNEL_CALL_STATE,
    HEADER_CHANNEL_STATE, HEADER_CHANNEL_STATE_NUMBER, HEADER_UNIQUE_ID,
};
use crate::variables::EslArray;
use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

/// Event format types supported by FreeSWITCH ESL
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EventFormat {
    /// Plain text format (default)
    Plain,
    /// JSON format
    Json,
    /// XML format
    Xml,
}

impl EventFormat {
    /// Determine event format from a Content-Type header value.
    pub fn from_content_type(ct: &str) -> Self {
        match ct {
            "text/event-json" => Self::Json,
            "text/event-xml" => Self::Xml,
            _ => Self::Plain,
        }
    }
}

impl fmt::Display for EventFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventFormat::Plain => write!(f, "plain"),
            EventFormat::Json => write!(f, "json"),
            EventFormat::Xml => write!(f, "xml"),
        }
    }
}

impl FromStr for EventFormat {
    type Err = ParseEventFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "plain" => Ok(Self::Plain),
            "json" => Ok(Self::Json),
            "xml" => Ok(Self::Xml),
            _ => Err(ParseEventFormatError(s.to_string())),
        }
    }
}

/// Error returned when parsing an invalid event format string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseEventFormatError(pub String);

impl fmt::Display for ParseEventFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown event format: {}", self.0)
    }
}

impl std::error::Error for ParseEventFormatError {}

/// Generates `EslEventType` enum with `Display`, `FromStr`, and `parse_event_type`.
macro_rules! esl_event_types {
    (
        $(
            $(#[$attr:meta])*
            $variant:ident => $wire:literal
        ),+ $(,)?
        ;
        // Extra variants not in the main match (after All)
        $(
            $(#[$extra_attr:meta])*
            $extra_variant:ident => $extra_wire:literal
        ),* $(,)?
    ) => {
        /// FreeSWITCH event types matching the canonical order from `esl_event.h`
        /// and `switch_event.c` EVENT_NAMES[].
        ///
        /// Variant names are the canonical wire names (e.g. `ChannelCreate` = `CHANNEL_CREATE`).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[non_exhaustive]
        #[allow(missing_docs)]
        pub enum EslEventType {
            $(
                $(#[$attr])*
                $variant,
            )+
            $(
                $(#[$extra_attr])*
                $extra_variant,
            )*
        }

        impl fmt::Display for EslEventType {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let name = match self {
                    $( EslEventType::$variant => $wire, )+
                    $( EslEventType::$extra_variant => $extra_wire, )*
                };
                f.write_str(name)
            }
        }

        impl EslEventType {
            /// Parse event type from wire name (case-insensitive).
            pub fn parse_event_type(s: &str) -> Option<Self> {
                match s.to_uppercase().as_str() {
                    $( $wire => Some(EslEventType::$variant), )+
                    $( $extra_wire => Some(EslEventType::$extra_variant), )*
                    _ => None,
                }
            }
        }

        impl FromStr for EslEventType {
            type Err = ParseEventTypeError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::parse_event_type(s).ok_or_else(|| ParseEventTypeError(s.to_string()))
            }
        }
    };
}

esl_event_types! {
    Custom => "CUSTOM",
    Clone => "CLONE",
    ChannelCreate => "CHANNEL_CREATE",
    ChannelDestroy => "CHANNEL_DESTROY",
    ChannelState => "CHANNEL_STATE",
    ChannelCallstate => "CHANNEL_CALLSTATE",
    ChannelAnswer => "CHANNEL_ANSWER",
    ChannelHangup => "CHANNEL_HANGUP",
    ChannelHangupComplete => "CHANNEL_HANGUP_COMPLETE",
    ChannelExecute => "CHANNEL_EXECUTE",
    ChannelExecuteComplete => "CHANNEL_EXECUTE_COMPLETE",
    ChannelHold => "CHANNEL_HOLD",
    ChannelUnhold => "CHANNEL_UNHOLD",
    ChannelBridge => "CHANNEL_BRIDGE",
    ChannelUnbridge => "CHANNEL_UNBRIDGE",
    ChannelProgress => "CHANNEL_PROGRESS",
    ChannelProgressMedia => "CHANNEL_PROGRESS_MEDIA",
    ChannelOutgoing => "CHANNEL_OUTGOING",
    ChannelPark => "CHANNEL_PARK",
    ChannelUnpark => "CHANNEL_UNPARK",
    ChannelApplication => "CHANNEL_APPLICATION",
    ChannelOriginate => "CHANNEL_ORIGINATE",
    ChannelUuid => "CHANNEL_UUID",
    Api => "API",
    Log => "LOG",
    InboundChan => "INBOUND_CHAN",
    OutboundChan => "OUTBOUND_CHAN",
    Startup => "STARTUP",
    Shutdown => "SHUTDOWN",
    Publish => "PUBLISH",
    Unpublish => "UNPUBLISH",
    Talk => "TALK",
    Notalk => "NOTALK",
    SessionCrash => "SESSION_CRASH",
    ModuleLoad => "MODULE_LOAD",
    ModuleUnload => "MODULE_UNLOAD",
    Dtmf => "DTMF",
    Message => "MESSAGE",
    PresenceIn => "PRESENCE_IN",
    NotifyIn => "NOTIFY_IN",
    PresenceOut => "PRESENCE_OUT",
    PresenceProbe => "PRESENCE_PROBE",
    MessageWaiting => "MESSAGE_WAITING",
    MessageQuery => "MESSAGE_QUERY",
    Roster => "ROSTER",
    Codec => "CODEC",
    BackgroundJob => "BACKGROUND_JOB",
    DetectedSpeech => "DETECTED_SPEECH",
    DetectedTone => "DETECTED_TONE",
    PrivateCommand => "PRIVATE_COMMAND",
    Heartbeat => "HEARTBEAT",
    Trap => "TRAP",
    AddSchedule => "ADD_SCHEDULE",
    DelSchedule => "DEL_SCHEDULE",
    ExeSchedule => "EXE_SCHEDULE",
    ReSchedule => "RE_SCHEDULE",
    ReloadXml => "RELOADXML",
    Notify => "NOTIFY",
    PhoneFeature => "PHONE_FEATURE",
    PhoneFeatureSubscribe => "PHONE_FEATURE_SUBSCRIBE",
    SendMessage => "SEND_MESSAGE",
    RecvMessage => "RECV_MESSAGE",
    RequestParams => "REQUEST_PARAMS",
    ChannelData => "CHANNEL_DATA",
    General => "GENERAL",
    Command => "COMMAND",
    SessionHeartbeat => "SESSION_HEARTBEAT",
    ClientDisconnected => "CLIENT_DISCONNECTED",
    ServerDisconnected => "SERVER_DISCONNECTED",
    SendInfo => "SEND_INFO",
    RecvInfo => "RECV_INFO",
    RecvRtcpMessage => "RECV_RTCP_MESSAGE",
    SendRtcpMessage => "SEND_RTCP_MESSAGE",
    CallSecure => "CALL_SECURE",
    Nat => "NAT",
    RecordStart => "RECORD_START",
    RecordStop => "RECORD_STOP",
    PlaybackStart => "PLAYBACK_START",
    PlaybackStop => "PLAYBACK_STOP",
    CallUpdate => "CALL_UPDATE",
    Failure => "FAILURE",
    SocketData => "SOCKET_DATA",
    MediaBugStart => "MEDIA_BUG_START",
    MediaBugStop => "MEDIA_BUG_STOP",
    ConferenceDataQuery => "CONFERENCE_DATA_QUERY",
    ConferenceData => "CONFERENCE_DATA",
    CallSetupReq => "CALL_SETUP_REQ",
    CallSetupResult => "CALL_SETUP_RESULT",
    CallDetail => "CALL_DETAIL",
    DeviceState => "DEVICE_STATE",
    Text => "TEXT",
    ShutdownRequested => "SHUTDOWN_REQUESTED",
    /// Subscribe to all events
    All => "ALL";
    // --- Not in libs/esl/ EVENT_NAMES[], only in switch_event.c ---
    // check-event-types.sh stops scanning at the All variant above.
    /// Present in `switch_event.c` but not in `libs/esl/` EVENT_NAMES[].
    StartRecording => "START_RECORDING",
}

/// Error returned when parsing an unknown event type string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseEventTypeError(pub String);

impl fmt::Display for ParseEventTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown event type: {}", self.0)
    }
}

impl std::error::Error for ParseEventTypeError {}

/// Event priority levels matching FreeSWITCH `esl_priority_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum EslEventPriority {
    /// Default priority.
    Normal,
    /// Lower than normal.
    Low,
    /// Higher than normal.
    High,
}

impl fmt::Display for EslEventPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EslEventPriority::Normal => write!(f, "NORMAL"),
            EslEventPriority::Low => write!(f, "LOW"),
            EslEventPriority::High => write!(f, "HIGH"),
        }
    }
}

/// Error returned when parsing an invalid priority string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsePriorityError(pub String);

impl fmt::Display for ParsePriorityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown priority: {}", self.0)
    }
}

impl std::error::Error for ParsePriorityError {}

impl FromStr for EslEventPriority {
    type Err = ParsePriorityError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s
            .to_uppercase()
            .as_str()
        {
            "NORMAL" => Ok(EslEventPriority::Normal),
            "LOW" => Ok(EslEventPriority::Low),
            "HIGH" => Ok(EslEventPriority::High),
            _ => Err(ParsePriorityError(s.to_string())),
        }
    }
}

/// Channel state from `switch_channel_state_t` — carried in the `Channel-State` header
/// as a string (`CS_ROUTING`) and in `Channel-State-Number` as an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
#[repr(u8)]
#[allow(missing_docs)]
pub enum ChannelState {
    CsNew = 0,
    CsInit = 1,
    CsRouting = 2,
    CsSoftExecute = 3,
    CsExecute = 4,
    CsExchangeMedia = 5,
    CsPark = 6,
    CsConsumeMedia = 7,
    CsHibernate = 8,
    CsReset = 9,
    CsHangup = 10,
    CsReporting = 11,
    CsDestroy = 12,
    CsNone = 13,
}

impl ChannelState {
    /// Parse from the `Channel-State-Number` integer header value.
    pub fn from_number(n: u8) -> Option<Self> {
        match n {
            0 => Some(Self::CsNew),
            1 => Some(Self::CsInit),
            2 => Some(Self::CsRouting),
            3 => Some(Self::CsSoftExecute),
            4 => Some(Self::CsExecute),
            5 => Some(Self::CsExchangeMedia),
            6 => Some(Self::CsPark),
            7 => Some(Self::CsConsumeMedia),
            8 => Some(Self::CsHibernate),
            9 => Some(Self::CsReset),
            10 => Some(Self::CsHangup),
            11 => Some(Self::CsReporting),
            12 => Some(Self::CsDestroy),
            13 => Some(Self::CsNone),
            _ => None,
        }
    }

    /// Integer discriminant matching `switch_channel_state_t`.
    pub fn as_number(&self) -> u8 {
        *self as u8
    }
}

impl fmt::Display for ChannelState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::CsNew => "CS_NEW",
            Self::CsInit => "CS_INIT",
            Self::CsRouting => "CS_ROUTING",
            Self::CsSoftExecute => "CS_SOFT_EXECUTE",
            Self::CsExecute => "CS_EXECUTE",
            Self::CsExchangeMedia => "CS_EXCHANGE_MEDIA",
            Self::CsPark => "CS_PARK",
            Self::CsConsumeMedia => "CS_CONSUME_MEDIA",
            Self::CsHibernate => "CS_HIBERNATE",
            Self::CsReset => "CS_RESET",
            Self::CsHangup => "CS_HANGUP",
            Self::CsReporting => "CS_REPORTING",
            Self::CsDestroy => "CS_DESTROY",
            Self::CsNone => "CS_NONE",
        };
        f.write_str(name)
    }
}

/// Error returned when parsing an invalid channel state string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseChannelStateError(pub String);

impl fmt::Display for ParseChannelStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown channel state: {}", self.0)
    }
}

impl std::error::Error for ParseChannelStateError {}

impl FromStr for ChannelState {
    type Err = ParseChannelStateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s
            .to_uppercase()
            .as_str()
        {
            "CS_NEW" => Ok(Self::CsNew),
            "CS_INIT" => Ok(Self::CsInit),
            "CS_ROUTING" => Ok(Self::CsRouting),
            "CS_SOFT_EXECUTE" => Ok(Self::CsSoftExecute),
            "CS_EXECUTE" => Ok(Self::CsExecute),
            "CS_EXCHANGE_MEDIA" => Ok(Self::CsExchangeMedia),
            "CS_PARK" => Ok(Self::CsPark),
            "CS_CONSUME_MEDIA" => Ok(Self::CsConsumeMedia),
            "CS_HIBERNATE" => Ok(Self::CsHibernate),
            "CS_RESET" => Ok(Self::CsReset),
            "CS_HANGUP" => Ok(Self::CsHangup),
            "CS_REPORTING" => Ok(Self::CsReporting),
            "CS_DESTROY" => Ok(Self::CsDestroy),
            "CS_NONE" => Ok(Self::CsNone),
            _ => Err(ParseChannelStateError(s.to_string())),
        }
    }
}

/// Call state from `switch_channel_callstate_t` — carried in the `Channel-Call-State` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
#[allow(missing_docs)]
pub enum CallState {
    Down,
    Dialing,
    Ringing,
    Early,
    Active,
    Held,
    RingWait,
    Hangup,
    Unheld,
}

impl fmt::Display for CallState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Down => "DOWN",
            Self::Dialing => "DIALING",
            Self::Ringing => "RINGING",
            Self::Early => "EARLY",
            Self::Active => "ACTIVE",
            Self::Held => "HELD",
            Self::RingWait => "RING_WAIT",
            Self::Hangup => "HANGUP",
            Self::Unheld => "UNHELD",
        };
        f.write_str(name)
    }
}

/// Error returned when parsing an invalid call state string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseCallStateError(pub String);

impl fmt::Display for ParseCallStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown call state: {}", self.0)
    }
}

impl std::error::Error for ParseCallStateError {}

impl FromStr for CallState {
    type Err = ParseCallStateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s
            .to_uppercase()
            .as_str()
        {
            "DOWN" => Ok(Self::Down),
            "DIALING" => Ok(Self::Dialing),
            "RINGING" => Ok(Self::Ringing),
            "EARLY" => Ok(Self::Early),
            "ACTIVE" => Ok(Self::Active),
            "HELD" => Ok(Self::Held),
            "RING_WAIT" => Ok(Self::RingWait),
            "HANGUP" => Ok(Self::Hangup),
            "UNHELD" => Ok(Self::Unheld),
            _ => Err(ParseCallStateError(s.to_string())),
        }
    }
}

/// Answer state from the `Answer-State` header. Wire format is lowercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
#[allow(missing_docs)]
pub enum AnswerState {
    Hangup,
    Answered,
    Early,
    Ringing,
}

impl fmt::Display for AnswerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Hangup => "hangup",
            Self::Answered => "answered",
            Self::Early => "early",
            Self::Ringing => "ringing",
        };
        f.write_str(name)
    }
}

/// Error returned when parsing an invalid answer state string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseAnswerStateError(pub String);

impl fmt::Display for ParseAnswerStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown answer state: {}", self.0)
    }
}

impl std::error::Error for ParseAnswerStateError {}

impl FromStr for AnswerState {
    type Err = ParseAnswerStateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s
            .to_lowercase()
            .as_str()
        {
            "hangup" => Ok(Self::Hangup),
            "answered" => Ok(Self::Answered),
            "early" => Ok(Self::Early),
            "ringing" => Ok(Self::Ringing),
            _ => Err(ParseAnswerStateError(s.to_string())),
        }
    }
}

/// Call direction from the `Call-Direction` header. Wire format is lowercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
#[allow(missing_docs)]
pub enum CallDirection {
    Inbound,
    Outbound,
}

impl fmt::Display for CallDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Inbound => "inbound",
            Self::Outbound => "outbound",
        };
        f.write_str(name)
    }
}

/// Error returned when parsing an invalid call direction string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseCallDirectionError(pub String);

impl fmt::Display for ParseCallDirectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown call direction: {}", self.0)
    }
}

impl std::error::Error for ParseCallDirectionError {}

impl FromStr for CallDirection {
    type Err = ParseCallDirectionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s
            .to_lowercase()
            .as_str()
        {
            "inbound" => Ok(Self::Inbound),
            "outbound" => Ok(Self::Outbound),
            _ => Err(ParseCallDirectionError(s.to_string())),
        }
    }
}

/// ESL Event structure containing headers and optional body
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EslEvent {
    event_type: Option<EslEventType>,
    headers: HashMap<String, String>,
    body: Option<String>,
}

impl EslEvent {
    /// Create a new empty event
    pub fn new() -> Self {
        Self {
            event_type: None,
            headers: HashMap::new(),
            body: None,
        }
    }

    /// Create event with specified type
    pub fn with_type(event_type: EslEventType) -> Self {
        Self {
            event_type: Some(event_type),
            headers: HashMap::new(),
            body: None,
        }
    }

    /// Parsed event type, if recognized.
    pub fn event_type(&self) -> Option<EslEventType> {
        self.event_type
    }

    /// Override the event type.
    pub fn set_event_type(&mut self, event_type: Option<EslEventType>) {
        self.event_type = event_type;
    }

    /// Look up a header by name (case-sensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(name)
            .map(|s| s.as_str())
    }

    /// All headers as a map.
    pub fn headers(&self) -> &HashMap<String, String> {
        &self.headers
    }

    /// Set or overwrite a header.
    pub fn set_header(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.headers
            .insert(name.into(), value.into());
    }

    /// Remove a header, returning its value if it existed.
    pub fn del_header(&mut self, name: &str) -> Option<String> {
        self.headers
            .remove(name)
    }

    /// Event body (the content after the blank line in plain-text events).
    pub fn body(&self) -> Option<&str> {
        self.body
            .as_deref()
    }

    /// Set the event body.
    pub fn set_body(&mut self, body: String) {
        self.body = Some(body);
    }

    /// Sets the `priority` header carried on the event.
    ///
    /// FreeSWITCH stores this as metadata but does **not** use it for dispatch
    /// ordering — all events are delivered FIFO regardless of priority.
    pub fn set_priority(&mut self, priority: EslEventPriority) {
        self.set_header("priority", priority.to_string());
    }

    /// Parse the `priority` header value.
    pub fn priority(&self) -> Option<EslEventPriority> {
        self.header("priority")?
            .parse()
            .ok()
    }

    /// Append a value to a multi-value header (PUSH semantics).
    ///
    /// If the header doesn't exist, sets it as a plain value.
    /// If it exists as a plain value, converts to `ARRAY::old|:new`.
    /// If it already has an `ARRAY::` prefix, appends the new value.
    ///
    /// ```
    /// # use freeswitch_esl_tokio::EslEvent;
    /// let mut event = EslEvent::new();
    /// event.push_header("X-Test", "first");
    /// event.push_header("X-Test", "second");
    /// assert_eq!(event.header("X-Test"), Some("ARRAY::first|:second"));
    /// ```
    pub fn push_header(&mut self, name: &str, value: &str) {
        self.stack_header(name, value, EslArray::push);
    }

    /// Prepend a value to a multi-value header (UNSHIFT semantics).
    ///
    /// Same conversion rules as `push_header()`, but inserts at the front.
    ///
    /// ```
    /// # use freeswitch_esl_tokio::EslEvent;
    /// let mut event = EslEvent::new();
    /// event.set_header("X-Test", "ARRAY::b|:c");
    /// event.unshift_header("X-Test", "a");
    /// assert_eq!(event.header("X-Test"), Some("ARRAY::a|:b|:c"));
    /// ```
    pub fn unshift_header(&mut self, name: &str, value: &str) {
        self.stack_header(name, value, EslArray::unshift);
    }

    fn stack_header(&mut self, name: &str, value: &str, op: fn(&mut EslArray, String)) {
        match self
            .headers
            .get(name)
        {
            None => {
                self.set_header(name, value);
            }
            Some(existing) => {
                let mut arr = match EslArray::parse(existing) {
                    Some(arr) => arr,
                    None => EslArray::new(vec![existing.clone()]),
                };
                op(&mut arr, value.into());
                self.set_header(name, arr.to_string());
            }
        }
    }

    /// `Unique-ID` header, falling back to `Caller-Unique-ID`.
    pub fn unique_id(&self) -> Option<&str> {
        self.header(HEADER_UNIQUE_ID)
            .or_else(|| self.header(HEADER_CALLER_UUID))
    }

    /// `Job-UUID` header from `bgapi` `BACKGROUND_JOB` events.
    pub fn job_uuid(&self) -> Option<&str> {
        self.header("Job-UUID")
    }

    /// `Channel-Name` header (e.g. `sofia/internal/1000@domain`).
    pub fn channel_name(&self) -> Option<&str> {
        self.header("Channel-Name")
    }

    /// `Caller-Caller-ID-Number` header.
    pub fn caller_id_number(&self) -> Option<&str> {
        self.header("Caller-Caller-ID-Number")
    }

    /// `Caller-Caller-ID-Name` header.
    pub fn caller_id_name(&self) -> Option<&str> {
        self.header("Caller-Caller-ID-Name")
    }

    /// `Hangup-Cause` header (e.g. `NORMAL_CLEARING`, `USER_BUSY`).
    pub fn hangup_cause(&self) -> Option<&str> {
        self.header("Hangup-Cause")
    }

    /// Parse the `Channel-State` header into a [`ChannelState`].
    pub fn channel_state(&self) -> Option<ChannelState> {
        self.header(HEADER_CHANNEL_STATE)?
            .parse()
            .ok()
    }

    /// Parse the `Channel-State-Number` header into a [`ChannelState`].
    pub fn channel_state_number(&self) -> Option<ChannelState> {
        let n: u8 = self
            .header(HEADER_CHANNEL_STATE_NUMBER)?
            .parse()
            .ok()?;
        ChannelState::from_number(n)
    }

    /// Parse the `Channel-Call-State` header into a [`CallState`].
    pub fn call_state(&self) -> Option<CallState> {
        self.header(HEADER_CHANNEL_CALL_STATE)?
            .parse()
            .ok()
    }

    /// Parse the `Answer-State` header into an [`AnswerState`].
    pub fn answer_state(&self) -> Option<AnswerState> {
        self.header(HEADER_ANSWER_STATE)?
            .parse()
            .ok()
    }

    /// Parse the `Call-Direction` header into a [`CallDirection`].
    pub fn call_direction(&self) -> Option<CallDirection> {
        self.header(HEADER_CALL_DIRECTION)?
            .parse()
            .ok()
    }

    /// `Event-Subclass` header for `CUSTOM` events (e.g. `sofia::register`).
    pub fn event_subclass(&self) -> Option<&str> {
        self.header("Event-Subclass")
    }

    /// Look up a channel variable by name.
    ///
    /// Checks the `variable_{name}` header, which is how FreeSWITCH exposes
    /// channel variables in events.
    pub fn variable(&self, name: &str) -> Option<&str> {
        let key = format!("variable_{}", name);
        self.header(&key)
    }

    /// Check whether this event matches the given type.
    pub fn is_event_type(&self, event_type: EslEventType) -> bool {
        self.event_type == Some(event_type)
    }

    /// Serialize to ESL plain text wire format with percent-encoded header values.
    ///
    /// This is the inverse of `EslParser::parse_plain_event()`. The output can
    /// be fed back through the parser to reconstruct an equivalent `EslEvent`
    /// (round-trip).
    ///
    /// `Event-Name` is emitted first, remaining headers are sorted alphabetically
    /// for deterministic output. `Content-Length` from stored headers is skipped
    /// and recomputed from the body if present.
    pub fn to_plain_format(&self) -> String {
        use std::fmt::Write;
        let mut result = String::new();

        if let Some(event_name) = self
            .headers
            .get("Event-Name")
        {
            let _ = writeln!(
                result,
                "Event-Name: {}",
                percent_encode(event_name.as_bytes(), NON_ALPHANUMERIC)
            );
        }

        let mut sorted_headers: Vec<_> = self
            .headers
            .iter()
            .filter(|(k, _)| k.as_str() != "Event-Name" && k.as_str() != "Content-Length")
            .collect();
        sorted_headers.sort_by_key(|(k, _)| k.as_str());

        for (key, value) in sorted_headers {
            let _ = writeln!(
                result,
                "{}: {}",
                key,
                percent_encode(value.as_bytes(), NON_ALPHANUMERIC)
            );
        }

        if let Some(body) = &self.body {
            let _ = writeln!(result, "Content-Length: {}", body.len());
            result.push('\n');
            result.push_str(body);
        } else {
            result.push('\n');
        }

        result
    }
}

impl Default for EslEvent {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notify_in_parse() {
        assert_eq!(
            EslEventType::parse_event_type("NOTIFY_IN"),
            Some(EslEventType::NotifyIn)
        );
        assert_eq!(
            EslEventType::parse_event_type("notify_in"),
            Some(EslEventType::NotifyIn)
        );
    }

    #[test]
    fn test_notify_in_display() {
        assert_eq!(EslEventType::NotifyIn.to_string(), "NOTIFY_IN");
    }

    #[test]
    fn test_notify_in_distinct_from_notify() {
        assert_ne!(EslEventType::Notify, EslEventType::NotifyIn);
        assert_ne!(
            EslEventType::Notify.to_string(),
            EslEventType::NotifyIn.to_string()
        );
    }

    #[test]
    fn test_wire_names_match_c_esl() {
        assert_eq!(
            EslEventType::ChannelOutgoing.to_string(),
            "CHANNEL_OUTGOING"
        );
        assert_eq!(EslEventType::Api.to_string(), "API");
        assert_eq!(EslEventType::ReloadXml.to_string(), "RELOADXML");
        assert_eq!(EslEventType::PresenceIn.to_string(), "PRESENCE_IN");
        assert_eq!(EslEventType::Roster.to_string(), "ROSTER");
        assert_eq!(EslEventType::Text.to_string(), "TEXT");
        assert_eq!(EslEventType::ReSchedule.to_string(), "RE_SCHEDULE");

        assert_eq!(
            EslEventType::parse_event_type("CHANNEL_OUTGOING"),
            Some(EslEventType::ChannelOutgoing)
        );
        assert_eq!(
            EslEventType::parse_event_type("API"),
            Some(EslEventType::Api)
        );
        assert_eq!(
            EslEventType::parse_event_type("RELOADXML"),
            Some(EslEventType::ReloadXml)
        );
        assert_eq!(
            EslEventType::parse_event_type("PRESENCE_IN"),
            Some(EslEventType::PresenceIn)
        );
    }

    #[test]
    fn test_event_type_from_str() {
        assert_eq!(
            "CHANNEL_ANSWER".parse::<EslEventType>(),
            Ok(EslEventType::ChannelAnswer)
        );
        assert_eq!(
            "channel_answer".parse::<EslEventType>(),
            Ok(EslEventType::ChannelAnswer)
        );
        assert!("UNKNOWN_EVENT"
            .parse::<EslEventType>()
            .is_err());
    }

    #[test]
    fn test_del_header() {
        let mut event = EslEvent::new();
        event.set_header("Foo", "bar");
        event.set_header("Baz", "qux");

        let removed = event.del_header("Foo");
        assert_eq!(removed, Some("bar".to_string()));
        assert!(event
            .header("Foo")
            .is_none());
        assert_eq!(event.header("Baz"), Some("qux"));

        let removed_again = event.del_header("Foo");
        assert_eq!(removed_again, None);
    }

    #[test]
    fn test_to_plain_format_basic() {
        let mut event = EslEvent::with_type(EslEventType::Heartbeat);
        event.set_header("Event-Name", "HEARTBEAT");
        event.set_header("Core-UUID", "abc-123");

        let plain = event.to_plain_format();

        assert!(plain.starts_with("Event-Name: "));
        assert!(plain.contains("Core-UUID: "));
        assert!(plain.ends_with("\n\n"));
    }

    #[test]
    fn test_to_plain_format_percent_encoding() {
        let mut event = EslEvent::with_type(EslEventType::Heartbeat);
        event.set_header("Event-Name", "HEARTBEAT");
        event.set_header("Up-Time", "0 years, 0 days");

        let plain = event.to_plain_format();

        assert!(!plain.contains("0 years, 0 days"));
        assert!(plain.contains("Up-Time: "));
        assert!(plain.contains("%20"));
    }

    #[test]
    fn test_to_plain_format_with_body() {
        let mut event = EslEvent::with_type(EslEventType::BackgroundJob);
        event.set_header("Event-Name", "BACKGROUND_JOB");
        event.set_header("Job-UUID", "def-456");
        event.set_body("+OK result\n".to_string());

        let plain = event.to_plain_format();

        assert!(plain.contains("Content-Length: 11\n"));
        assert!(plain.ends_with("\n\n+OK result\n"));
    }

    #[test]
    fn test_to_plain_format_round_trip() {
        use crate::protocol::{EslMessage, EslParser, MessageType};

        let mut original = EslEvent::with_type(EslEventType::Heartbeat);
        original.set_header("Event-Name", "HEARTBEAT");
        original.set_header("Core-UUID", "abc-123");
        original.set_header("Up-Time", "0 years, 0 days, 1 hour");
        original.set_header("Event-Info", "System Ready");

        let plain1 = original.to_plain_format();

        let msg1 = EslMessage::new(
            MessageType::Event,
            {
                let mut h = HashMap::new();
                h.insert("Content-Type".to_string(), "text/event-plain".to_string());
                h
            },
            Some(plain1.clone()),
        );
        let parsed1 = EslParser::new()
            .parse_event(msg1, crate::event::EventFormat::Plain)
            .unwrap();

        assert_eq!(parsed1.event_type, original.event_type);
        assert_eq!(parsed1.headers, original.headers);
        assert_eq!(parsed1.body, original.body);

        let plain2 = parsed1.to_plain_format();
        let msg2 = EslMessage::new(
            MessageType::Event,
            {
                let mut h = HashMap::new();
                h.insert("Content-Type".to_string(), "text/event-plain".to_string());
                h
            },
            Some(plain2),
        );
        let parsed2 = EslParser::new()
            .parse_event(msg2, crate::event::EventFormat::Plain)
            .unwrap();

        assert_eq!(parsed2.event_type, original.event_type);
        assert_eq!(parsed2.headers, original.headers);
        assert_eq!(parsed2.body, original.body);
    }

    #[test]
    fn test_to_plain_format_round_trip_with_body() {
        use crate::protocol::{EslMessage, EslParser, MessageType};

        let body_text = "+OK Status\nLine 2\n";
        let mut original = EslEvent::with_type(EslEventType::BackgroundJob);
        original.set_header("Event-Name", "BACKGROUND_JOB");
        original.set_header("Job-UUID", "job-789");
        original.set_header(
            "Content-Length".to_string(),
            body_text
                .len()
                .to_string(),
        );
        original.set_body(body_text.to_string());

        let plain = original.to_plain_format();
        let msg = EslMessage::new(
            MessageType::Event,
            {
                let mut h = HashMap::new();
                h.insert("Content-Type".to_string(), "text/event-plain".to_string());
                h
            },
            Some(plain),
        );
        let parsed = EslParser::new()
            .parse_event(msg, crate::event::EventFormat::Plain)
            .unwrap();

        assert_eq!(parsed.event_type, original.event_type);
        assert_eq!(parsed.headers, original.headers);
        assert_eq!(parsed.body, original.body);
    }

    #[test]
    fn test_set_priority_normal() {
        let mut event = EslEvent::new();
        event.set_priority(EslEventPriority::Normal);
        assert_eq!(event.priority(), Some(EslEventPriority::Normal));
        assert_eq!(event.header("priority"), Some("NORMAL"));
    }

    #[test]
    fn test_set_priority_high() {
        let mut event = EslEvent::new();
        event.set_priority(EslEventPriority::High);
        assert_eq!(event.priority(), Some(EslEventPriority::High));
        assert_eq!(event.header("priority"), Some("HIGH"));
    }

    #[test]
    fn test_priority_display() {
        assert_eq!(EslEventPriority::Normal.to_string(), "NORMAL");
        assert_eq!(EslEventPriority::Low.to_string(), "LOW");
        assert_eq!(EslEventPriority::High.to_string(), "HIGH");
    }

    #[test]
    fn test_priority_from_str() {
        assert_eq!(
            "NORMAL".parse::<EslEventPriority>(),
            Ok(EslEventPriority::Normal)
        );
        assert_eq!("LOW".parse::<EslEventPriority>(), Ok(EslEventPriority::Low));
        assert_eq!(
            "HIGH".parse::<EslEventPriority>(),
            Ok(EslEventPriority::High)
        );
        assert!("INVALID"
            .parse::<EslEventPriority>()
            .is_err());
    }

    #[test]
    fn test_priority_from_str_case_insensitive() {
        assert_eq!(
            "normal".parse::<EslEventPriority>(),
            Ok(EslEventPriority::Normal)
        );
        assert_eq!("Low".parse::<EslEventPriority>(), Ok(EslEventPriority::Low));
        assert_eq!(
            "hIgH".parse::<EslEventPriority>(),
            Ok(EslEventPriority::High)
        );
    }

    #[test]
    fn test_push_header_new() {
        let mut event = EslEvent::new();
        event.push_header("X-Test", "first");
        assert_eq!(event.header("X-Test"), Some("first"));
    }

    #[test]
    fn test_push_header_existing_plain() {
        let mut event = EslEvent::new();
        event.set_header("X-Test", "first");
        event.push_header("X-Test", "second");
        assert_eq!(event.header("X-Test"), Some("ARRAY::first|:second"));
    }

    #[test]
    fn test_push_header_existing_array() {
        let mut event = EslEvent::new();
        event.set_header("X-Test", "ARRAY::a|:b");
        event.push_header("X-Test", "c");
        assert_eq!(event.header("X-Test"), Some("ARRAY::a|:b|:c"));
    }

    #[test]
    fn test_unshift_header_new() {
        let mut event = EslEvent::new();
        event.unshift_header("X-Test", "only");
        assert_eq!(event.header("X-Test"), Some("only"));
    }

    #[test]
    fn test_unshift_header_existing_array() {
        let mut event = EslEvent::new();
        event.set_header("X-Test", "ARRAY::b|:c");
        event.unshift_header("X-Test", "a");
        assert_eq!(event.header("X-Test"), Some("ARRAY::a|:b|:c"));
    }

    #[test]
    fn test_sendevent_with_priority_wire_format() {
        let mut event = EslEvent::with_type(EslEventType::Custom);
        event.set_header("Event-Name", "CUSTOM");
        event.set_header("Event-Subclass", "test::priority");
        event.set_priority(EslEventPriority::High);

        let plain = event.to_plain_format();
        assert!(plain.contains("priority: HIGH\n"));
    }

    #[test]
    fn test_convenience_accessors() {
        let mut event = EslEvent::new();
        event.set_header("Channel-Name", "sofia/internal/1000@example.com");
        event.set_header("Caller-Caller-ID-Number", "1000");
        event.set_header("Caller-Caller-ID-Name", "Alice");
        event.set_header("Hangup-Cause", "NORMAL_CLEARING");
        event.set_header("Event-Subclass", "sofia::register");
        event.set_header("variable_sip_from_display", "Bob");

        assert_eq!(
            event.channel_name(),
            Some("sofia/internal/1000@example.com")
        );
        assert_eq!(event.caller_id_number(), Some("1000"));
        assert_eq!(event.caller_id_name(), Some("Alice"));
        assert_eq!(event.hangup_cause(), Some("NORMAL_CLEARING"));
        assert_eq!(event.event_subclass(), Some("sofia::register"));
        assert_eq!(event.variable("sip_from_display"), Some("Bob"));
        assert_eq!(event.variable("nonexistent"), None);
    }

    #[test]
    fn test_event_format_from_str() {
        assert_eq!("plain".parse::<EventFormat>(), Ok(EventFormat::Plain));
        assert_eq!("json".parse::<EventFormat>(), Ok(EventFormat::Json));
        assert_eq!("xml".parse::<EventFormat>(), Ok(EventFormat::Xml));
        assert!("foo"
            .parse::<EventFormat>()
            .is_err());
    }

    #[test]
    fn test_event_format_from_content_type() {
        assert_eq!(
            EventFormat::from_content_type("text/event-json"),
            EventFormat::Json
        );
        assert_eq!(
            EventFormat::from_content_type("text/event-xml"),
            EventFormat::Xml
        );
        assert_eq!(
            EventFormat::from_content_type("text/event-plain"),
            EventFormat::Plain
        );
        assert_eq!(
            EventFormat::from_content_type("unknown"),
            EventFormat::Plain
        );
    }

    // --- ChannelState tests ---

    #[test]
    fn test_channel_state_display() {
        assert_eq!(ChannelState::CsNew.to_string(), "CS_NEW");
        assert_eq!(ChannelState::CsInit.to_string(), "CS_INIT");
        assert_eq!(ChannelState::CsRouting.to_string(), "CS_ROUTING");
        assert_eq!(ChannelState::CsSoftExecute.to_string(), "CS_SOFT_EXECUTE");
        assert_eq!(ChannelState::CsExecute.to_string(), "CS_EXECUTE");
        assert_eq!(
            ChannelState::CsExchangeMedia.to_string(),
            "CS_EXCHANGE_MEDIA"
        );
        assert_eq!(ChannelState::CsPark.to_string(), "CS_PARK");
        assert_eq!(ChannelState::CsConsumeMedia.to_string(), "CS_CONSUME_MEDIA");
        assert_eq!(ChannelState::CsHibernate.to_string(), "CS_HIBERNATE");
        assert_eq!(ChannelState::CsReset.to_string(), "CS_RESET");
        assert_eq!(ChannelState::CsHangup.to_string(), "CS_HANGUP");
        assert_eq!(ChannelState::CsReporting.to_string(), "CS_REPORTING");
        assert_eq!(ChannelState::CsDestroy.to_string(), "CS_DESTROY");
        assert_eq!(ChannelState::CsNone.to_string(), "CS_NONE");
    }

    #[test]
    fn test_channel_state_from_str() {
        assert_eq!("CS_NEW".parse::<ChannelState>(), Ok(ChannelState::CsNew));
        assert_eq!(
            "CS_EXECUTE".parse::<ChannelState>(),
            Ok(ChannelState::CsExecute)
        );
        assert_eq!(
            "CS_HANGUP".parse::<ChannelState>(),
            Ok(ChannelState::CsHangup)
        );
        assert_eq!(
            "CS_DESTROY".parse::<ChannelState>(),
            Ok(ChannelState::CsDestroy)
        );
    }

    #[test]
    fn test_channel_state_from_str_case_insensitive() {
        assert_eq!("cs_new".parse::<ChannelState>(), Ok(ChannelState::CsNew));
        assert_eq!(
            "Cs_Routing".parse::<ChannelState>(),
            Ok(ChannelState::CsRouting)
        );
    }

    #[test]
    fn test_channel_state_from_str_unknown() {
        assert!("CS_BOGUS"
            .parse::<ChannelState>()
            .is_err());
        assert!(""
            .parse::<ChannelState>()
            .is_err());
    }

    #[test]
    fn test_channel_state_from_number() {
        assert_eq!(ChannelState::from_number(0), Some(ChannelState::CsNew));
        assert_eq!(ChannelState::from_number(4), Some(ChannelState::CsExecute));
        assert_eq!(ChannelState::from_number(10), Some(ChannelState::CsHangup));
        assert_eq!(ChannelState::from_number(13), Some(ChannelState::CsNone));
        assert_eq!(ChannelState::from_number(14), None);
        assert_eq!(ChannelState::from_number(255), None);
    }

    #[test]
    fn test_channel_state_as_number() {
        assert_eq!(ChannelState::CsNew.as_number(), 0);
        assert_eq!(ChannelState::CsExecute.as_number(), 4);
        assert_eq!(ChannelState::CsHangup.as_number(), 10);
        assert_eq!(ChannelState::CsNone.as_number(), 13);
    }

    // --- CallState tests ---

    #[test]
    fn test_call_state_display() {
        assert_eq!(CallState::Down.to_string(), "DOWN");
        assert_eq!(CallState::Dialing.to_string(), "DIALING");
        assert_eq!(CallState::Ringing.to_string(), "RINGING");
        assert_eq!(CallState::Early.to_string(), "EARLY");
        assert_eq!(CallState::Active.to_string(), "ACTIVE");
        assert_eq!(CallState::Held.to_string(), "HELD");
        assert_eq!(CallState::RingWait.to_string(), "RING_WAIT");
        assert_eq!(CallState::Hangup.to_string(), "HANGUP");
        assert_eq!(CallState::Unheld.to_string(), "UNHELD");
    }

    #[test]
    fn test_call_state_from_str() {
        assert_eq!("DOWN".parse::<CallState>(), Ok(CallState::Down));
        assert_eq!("ACTIVE".parse::<CallState>(), Ok(CallState::Active));
        assert_eq!("RING_WAIT".parse::<CallState>(), Ok(CallState::RingWait));
        assert_eq!("UNHELD".parse::<CallState>(), Ok(CallState::Unheld));
    }

    #[test]
    fn test_call_state_from_str_case_insensitive() {
        assert_eq!("down".parse::<CallState>(), Ok(CallState::Down));
        assert_eq!("Active".parse::<CallState>(), Ok(CallState::Active));
    }

    #[test]
    fn test_call_state_from_str_unknown() {
        assert!("BOGUS"
            .parse::<CallState>()
            .is_err());
    }

    // --- AnswerState tests ---

    #[test]
    fn test_answer_state_display() {
        assert_eq!(AnswerState::Hangup.to_string(), "hangup");
        assert_eq!(AnswerState::Answered.to_string(), "answered");
        assert_eq!(AnswerState::Early.to_string(), "early");
        assert_eq!(AnswerState::Ringing.to_string(), "ringing");
    }

    #[test]
    fn test_answer_state_from_str() {
        assert_eq!("hangup".parse::<AnswerState>(), Ok(AnswerState::Hangup));
        assert_eq!("answered".parse::<AnswerState>(), Ok(AnswerState::Answered));
        assert_eq!("early".parse::<AnswerState>(), Ok(AnswerState::Early));
        assert_eq!("ringing".parse::<AnswerState>(), Ok(AnswerState::Ringing));
    }

    #[test]
    fn test_answer_state_from_str_case_insensitive() {
        assert_eq!("HANGUP".parse::<AnswerState>(), Ok(AnswerState::Hangup));
        assert_eq!("Answered".parse::<AnswerState>(), Ok(AnswerState::Answered));
    }

    #[test]
    fn test_answer_state_from_str_unknown() {
        assert!("bogus"
            .parse::<AnswerState>()
            .is_err());
    }

    // --- CallDirection tests ---

    #[test]
    fn test_call_direction_display() {
        assert_eq!(CallDirection::Inbound.to_string(), "inbound");
        assert_eq!(CallDirection::Outbound.to_string(), "outbound");
    }

    #[test]
    fn test_call_direction_from_str() {
        assert_eq!(
            "inbound".parse::<CallDirection>(),
            Ok(CallDirection::Inbound)
        );
        assert_eq!(
            "outbound".parse::<CallDirection>(),
            Ok(CallDirection::Outbound)
        );
    }

    #[test]
    fn test_call_direction_from_str_case_insensitive() {
        assert_eq!(
            "INBOUND".parse::<CallDirection>(),
            Ok(CallDirection::Inbound)
        );
        assert_eq!(
            "Outbound".parse::<CallDirection>(),
            Ok(CallDirection::Outbound)
        );
    }

    #[test]
    fn test_call_direction_from_str_unknown() {
        assert!("bogus"
            .parse::<CallDirection>()
            .is_err());
    }

    // --- EslEvent accessor tests ---

    #[test]
    fn test_event_channel_state_accessor() {
        let mut event = EslEvent::new();
        event.set_header("Channel-State", "CS_EXECUTE");
        assert_eq!(event.channel_state(), Some(ChannelState::CsExecute));
    }

    #[test]
    fn test_event_channel_state_number_accessor() {
        let mut event = EslEvent::new();
        event.set_header("Channel-State-Number", "4");
        assert_eq!(event.channel_state_number(), Some(ChannelState::CsExecute));
    }

    #[test]
    fn test_event_call_state_accessor() {
        let mut event = EslEvent::new();
        event.set_header("Channel-Call-State", "ACTIVE");
        assert_eq!(event.call_state(), Some(CallState::Active));
    }

    #[test]
    fn test_event_answer_state_accessor() {
        let mut event = EslEvent::new();
        event.set_header("Answer-State", "answered");
        assert_eq!(event.answer_state(), Some(AnswerState::Answered));
    }

    #[test]
    fn test_event_call_direction_accessor() {
        let mut event = EslEvent::new();
        event.set_header("Call-Direction", "inbound");
        assert_eq!(event.call_direction(), Some(CallDirection::Inbound));
    }

    #[test]
    fn test_event_typed_accessors_missing_headers() {
        let event = EslEvent::new();
        assert_eq!(event.channel_state(), None);
        assert_eq!(event.channel_state_number(), None);
        assert_eq!(event.call_state(), None);
        assert_eq!(event.answer_state(), None);
        assert_eq!(event.call_direction(), None);
    }

    #[test]
    fn test_event_typed_accessors_invalid_values() {
        let mut event = EslEvent::new();
        event.set_header("Channel-State", "BOGUS");
        event.set_header("Channel-State-Number", "999");
        event.set_header("Channel-Call-State", "BOGUS");
        event.set_header("Answer-State", "bogus");
        event.set_header("Call-Direction", "bogus");
        assert_eq!(event.channel_state(), None);
        assert_eq!(event.channel_state_number(), None);
        assert_eq!(event.call_state(), None);
        assert_eq!(event.answer_state(), None);
        assert_eq!(event.call_direction(), None);
    }
}
