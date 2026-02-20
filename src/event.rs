//! ESL event types and structures

use crate::variables::EslArray;
use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

/// Event format types supported by FreeSWITCH ESL
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventFormat {
    /// Plain text format (default)
    Plain,
    /// JSON format
    Json,
    /// XML format
    Xml,
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

/// FreeSWITCH event types matching the canonical order from `esl_event.h`
/// and `switch_event.c` EVENT_NAMES[].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EslEventType {
    Custom,
    Clone,
    ChannelCreate,
    ChannelDestroy,
    ChannelState,
    ChannelCallstate,
    ChannelAnswer,
    ChannelHangup,
    ChannelHangupComplete,
    ChannelExecute,
    ChannelExecuteComplete,
    ChannelHold,
    ChannelUnhold,
    ChannelBridge,
    ChannelUnbridge,
    ChannelProgress,
    ChannelProgressMedia,
    ChannelOutgoing,
    ChannelPark,
    ChannelUnpark,
    ChannelApplication,
    ChannelOriginate,
    ChannelUuid,
    Api,
    Log,
    InboundChan,
    OutboundChan,
    Startup,
    Shutdown,
    Publish,
    Unpublish,
    Talk,
    Notalk,
    SessionCrash,
    ModuleLoad,
    ModuleUnload,
    Dtmf,
    Message,
    PresenceIn,
    NotifyIn,
    PresenceOut,
    PresenceProbe,
    MessageWaiting,
    MessageQuery,
    Roster,
    Codec,
    BackgroundJob,
    DetectedSpeech,
    DetectedTone,
    PrivateCommand,
    Heartbeat,
    Trap,
    AddSchedule,
    DelSchedule,
    ExeSchedule,
    ReSchedule,
    ReloadXml,
    Notify,
    PhoneFeature,
    PhoneFeatureSubscribe,
    SendMessage,
    RecvMessage,
    RequestParams,
    ChannelData,
    General,
    Command,
    SessionHeartbeat,
    ClientDisconnected,
    ServerDisconnected,
    SendInfo,
    RecvInfo,
    RecvRtcpMessage,
    SendRtcpMessage,
    CallSecure,
    Nat,
    RecordStart,
    RecordStop,
    PlaybackStart,
    PlaybackStop,
    CallUpdate,
    Failure,
    SocketData,
    MediaBugStart,
    MediaBugStop,
    ConferenceDataQuery,
    ConferenceData,
    CallSetupReq,
    CallSetupResult,
    CallDetail,
    DeviceState,
    Text,
    ShutdownRequested,
    /// Subscribe to all events
    All,
    // --- Not in libs/esl/ EVENT_NAMES[], only in switch_event.c ---
    // check-event-types.sh stops scanning at the All variant above.
    StartRecording,
}

impl fmt::Display for EslEventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            EslEventType::Custom => "CUSTOM",
            EslEventType::Clone => "CLONE",
            EslEventType::ChannelCreate => "CHANNEL_CREATE",
            EslEventType::ChannelDestroy => "CHANNEL_DESTROY",
            EslEventType::ChannelState => "CHANNEL_STATE",
            EslEventType::ChannelCallstate => "CHANNEL_CALLSTATE",
            EslEventType::ChannelAnswer => "CHANNEL_ANSWER",
            EslEventType::ChannelHangup => "CHANNEL_HANGUP",
            EslEventType::ChannelHangupComplete => "CHANNEL_HANGUP_COMPLETE",
            EslEventType::ChannelExecute => "CHANNEL_EXECUTE",
            EslEventType::ChannelExecuteComplete => "CHANNEL_EXECUTE_COMPLETE",
            EslEventType::ChannelHold => "CHANNEL_HOLD",
            EslEventType::ChannelUnhold => "CHANNEL_UNHOLD",
            EslEventType::ChannelBridge => "CHANNEL_BRIDGE",
            EslEventType::ChannelUnbridge => "CHANNEL_UNBRIDGE",
            EslEventType::ChannelProgress => "CHANNEL_PROGRESS",
            EslEventType::ChannelProgressMedia => "CHANNEL_PROGRESS_MEDIA",
            EslEventType::ChannelOutgoing => "CHANNEL_OUTGOING",
            EslEventType::ChannelPark => "CHANNEL_PARK",
            EslEventType::ChannelUnpark => "CHANNEL_UNPARK",
            EslEventType::ChannelApplication => "CHANNEL_APPLICATION",
            EslEventType::ChannelOriginate => "CHANNEL_ORIGINATE",
            EslEventType::ChannelUuid => "CHANNEL_UUID",
            EslEventType::Api => "API",
            EslEventType::Log => "LOG",
            EslEventType::InboundChan => "INBOUND_CHAN",
            EslEventType::OutboundChan => "OUTBOUND_CHAN",
            EslEventType::Startup => "STARTUP",
            EslEventType::Shutdown => "SHUTDOWN",
            EslEventType::Publish => "PUBLISH",
            EslEventType::Unpublish => "UNPUBLISH",
            EslEventType::Talk => "TALK",
            EslEventType::Notalk => "NOTALK",
            EslEventType::SessionCrash => "SESSION_CRASH",
            EslEventType::ModuleLoad => "MODULE_LOAD",
            EslEventType::ModuleUnload => "MODULE_UNLOAD",
            EslEventType::Dtmf => "DTMF",
            EslEventType::Message => "MESSAGE",
            EslEventType::PresenceIn => "PRESENCE_IN",
            EslEventType::NotifyIn => "NOTIFY_IN",
            EslEventType::PresenceOut => "PRESENCE_OUT",
            EslEventType::PresenceProbe => "PRESENCE_PROBE",
            EslEventType::MessageWaiting => "MESSAGE_WAITING",
            EslEventType::MessageQuery => "MESSAGE_QUERY",
            EslEventType::Roster => "ROSTER",
            EslEventType::Codec => "CODEC",
            EslEventType::BackgroundJob => "BACKGROUND_JOB",
            EslEventType::DetectedSpeech => "DETECTED_SPEECH",
            EslEventType::DetectedTone => "DETECTED_TONE",
            EslEventType::PrivateCommand => "PRIVATE_COMMAND",
            EslEventType::Heartbeat => "HEARTBEAT",
            EslEventType::Trap => "TRAP",
            EslEventType::AddSchedule => "ADD_SCHEDULE",
            EslEventType::DelSchedule => "DEL_SCHEDULE",
            EslEventType::ExeSchedule => "EXE_SCHEDULE",
            EslEventType::ReSchedule => "RE_SCHEDULE",
            EslEventType::ReloadXml => "RELOADXML",
            EslEventType::Notify => "NOTIFY",
            EslEventType::PhoneFeature => "PHONE_FEATURE",
            EslEventType::PhoneFeatureSubscribe => "PHONE_FEATURE_SUBSCRIBE",
            EslEventType::SendMessage => "SEND_MESSAGE",
            EslEventType::RecvMessage => "RECV_MESSAGE",
            EslEventType::RequestParams => "REQUEST_PARAMS",
            EslEventType::ChannelData => "CHANNEL_DATA",
            EslEventType::General => "GENERAL",
            EslEventType::Command => "COMMAND",
            EslEventType::SessionHeartbeat => "SESSION_HEARTBEAT",
            EslEventType::ClientDisconnected => "CLIENT_DISCONNECTED",
            EslEventType::ServerDisconnected => "SERVER_DISCONNECTED",
            EslEventType::SendInfo => "SEND_INFO",
            EslEventType::RecvInfo => "RECV_INFO",
            EslEventType::RecvRtcpMessage => "RECV_RTCP_MESSAGE",
            EslEventType::SendRtcpMessage => "SEND_RTCP_MESSAGE",
            EslEventType::CallSecure => "CALL_SECURE",
            EslEventType::Nat => "NAT",
            EslEventType::RecordStart => "RECORD_START",
            EslEventType::RecordStop => "RECORD_STOP",
            EslEventType::PlaybackStart => "PLAYBACK_START",
            EslEventType::PlaybackStop => "PLAYBACK_STOP",
            EslEventType::CallUpdate => "CALL_UPDATE",
            EslEventType::Failure => "FAILURE",
            EslEventType::SocketData => "SOCKET_DATA",
            EslEventType::MediaBugStart => "MEDIA_BUG_START",
            EslEventType::MediaBugStop => "MEDIA_BUG_STOP",
            EslEventType::ConferenceDataQuery => "CONFERENCE_DATA_QUERY",
            EslEventType::ConferenceData => "CONFERENCE_DATA",
            EslEventType::CallSetupReq => "CALL_SETUP_REQ",
            EslEventType::CallSetupResult => "CALL_SETUP_RESULT",
            EslEventType::CallDetail => "CALL_DETAIL",
            EslEventType::DeviceState => "DEVICE_STATE",
            EslEventType::Text => "TEXT",
            EslEventType::ShutdownRequested => "SHUTDOWN_REQUESTED",
            EslEventType::All => "ALL",
            // Not in libs/esl/ EVENT_NAMES[]
            EslEventType::StartRecording => "START_RECORDING",
        };
        write!(f, "{}", name)
    }
}

impl EslEventType {
    /// Parse event type from wire name (case-insensitive).
    pub fn parse_event_type(s: &str) -> Option<Self> {
        match s
            .to_uppercase()
            .as_str()
        {
            "CUSTOM" => Some(EslEventType::Custom),
            "CLONE" => Some(EslEventType::Clone),
            "CHANNEL_CREATE" => Some(EslEventType::ChannelCreate),
            "CHANNEL_DESTROY" => Some(EslEventType::ChannelDestroy),
            "CHANNEL_STATE" => Some(EslEventType::ChannelState),
            "CHANNEL_CALLSTATE" => Some(EslEventType::ChannelCallstate),
            "CHANNEL_ANSWER" => Some(EslEventType::ChannelAnswer),
            "CHANNEL_HANGUP" => Some(EslEventType::ChannelHangup),
            "CHANNEL_HANGUP_COMPLETE" => Some(EslEventType::ChannelHangupComplete),
            "CHANNEL_EXECUTE" => Some(EslEventType::ChannelExecute),
            "CHANNEL_EXECUTE_COMPLETE" => Some(EslEventType::ChannelExecuteComplete),
            "CHANNEL_HOLD" => Some(EslEventType::ChannelHold),
            "CHANNEL_UNHOLD" => Some(EslEventType::ChannelUnhold),
            "CHANNEL_BRIDGE" => Some(EslEventType::ChannelBridge),
            "CHANNEL_UNBRIDGE" => Some(EslEventType::ChannelUnbridge),
            "CHANNEL_PROGRESS" => Some(EslEventType::ChannelProgress),
            "CHANNEL_PROGRESS_MEDIA" => Some(EslEventType::ChannelProgressMedia),
            "CHANNEL_OUTGOING" => Some(EslEventType::ChannelOutgoing),
            "CHANNEL_PARK" => Some(EslEventType::ChannelPark),
            "CHANNEL_UNPARK" => Some(EslEventType::ChannelUnpark),
            "CHANNEL_APPLICATION" => Some(EslEventType::ChannelApplication),
            "CHANNEL_ORIGINATE" => Some(EslEventType::ChannelOriginate),
            "CHANNEL_UUID" => Some(EslEventType::ChannelUuid),
            "API" => Some(EslEventType::Api),
            "LOG" => Some(EslEventType::Log),
            "INBOUND_CHAN" => Some(EslEventType::InboundChan),
            "OUTBOUND_CHAN" => Some(EslEventType::OutboundChan),
            "STARTUP" => Some(EslEventType::Startup),
            "SHUTDOWN" => Some(EslEventType::Shutdown),
            "PUBLISH" => Some(EslEventType::Publish),
            "UNPUBLISH" => Some(EslEventType::Unpublish),
            "TALK" => Some(EslEventType::Talk),
            "NOTALK" => Some(EslEventType::Notalk),
            "SESSION_CRASH" => Some(EslEventType::SessionCrash),
            "MODULE_LOAD" => Some(EslEventType::ModuleLoad),
            "MODULE_UNLOAD" => Some(EslEventType::ModuleUnload),
            "DTMF" => Some(EslEventType::Dtmf),
            "MESSAGE" => Some(EslEventType::Message),
            "PRESENCE_IN" => Some(EslEventType::PresenceIn),
            "NOTIFY_IN" => Some(EslEventType::NotifyIn),
            "PRESENCE_OUT" => Some(EslEventType::PresenceOut),
            "PRESENCE_PROBE" => Some(EslEventType::PresenceProbe),
            "MESSAGE_WAITING" => Some(EslEventType::MessageWaiting),
            "MESSAGE_QUERY" => Some(EslEventType::MessageQuery),
            "ROSTER" => Some(EslEventType::Roster),
            "CODEC" => Some(EslEventType::Codec),
            "BACKGROUND_JOB" => Some(EslEventType::BackgroundJob),
            "DETECTED_SPEECH" => Some(EslEventType::DetectedSpeech),
            "DETECTED_TONE" => Some(EslEventType::DetectedTone),
            "PRIVATE_COMMAND" => Some(EslEventType::PrivateCommand),
            "HEARTBEAT" => Some(EslEventType::Heartbeat),
            "TRAP" => Some(EslEventType::Trap),
            "ADD_SCHEDULE" => Some(EslEventType::AddSchedule),
            "DEL_SCHEDULE" => Some(EslEventType::DelSchedule),
            "EXE_SCHEDULE" => Some(EslEventType::ExeSchedule),
            "RE_SCHEDULE" => Some(EslEventType::ReSchedule),
            "RELOADXML" => Some(EslEventType::ReloadXml),
            "NOTIFY" => Some(EslEventType::Notify),
            "PHONE_FEATURE" => Some(EslEventType::PhoneFeature),
            "PHONE_FEATURE_SUBSCRIBE" => Some(EslEventType::PhoneFeatureSubscribe),
            "SEND_MESSAGE" => Some(EslEventType::SendMessage),
            "RECV_MESSAGE" => Some(EslEventType::RecvMessage),
            "REQUEST_PARAMS" => Some(EslEventType::RequestParams),
            "CHANNEL_DATA" => Some(EslEventType::ChannelData),
            "GENERAL" => Some(EslEventType::General),
            "COMMAND" => Some(EslEventType::Command),
            "SESSION_HEARTBEAT" => Some(EslEventType::SessionHeartbeat),
            "CLIENT_DISCONNECTED" => Some(EslEventType::ClientDisconnected),
            "SERVER_DISCONNECTED" => Some(EslEventType::ServerDisconnected),
            "SEND_INFO" => Some(EslEventType::SendInfo),
            "RECV_INFO" => Some(EslEventType::RecvInfo),
            "RECV_RTCP_MESSAGE" => Some(EslEventType::RecvRtcpMessage),
            "SEND_RTCP_MESSAGE" => Some(EslEventType::SendRtcpMessage),
            "CALL_SECURE" => Some(EslEventType::CallSecure),
            "NAT" => Some(EslEventType::Nat),
            "RECORD_START" => Some(EslEventType::RecordStart),
            "RECORD_STOP" => Some(EslEventType::RecordStop),
            "PLAYBACK_START" => Some(EslEventType::PlaybackStart),
            "PLAYBACK_STOP" => Some(EslEventType::PlaybackStop),
            "CALL_UPDATE" => Some(EslEventType::CallUpdate),
            "FAILURE" => Some(EslEventType::Failure),
            "SOCKET_DATA" => Some(EslEventType::SocketData),
            "MEDIA_BUG_START" => Some(EslEventType::MediaBugStart),
            "MEDIA_BUG_STOP" => Some(EslEventType::MediaBugStop),
            "CONFERENCE_DATA_QUERY" => Some(EslEventType::ConferenceDataQuery),
            "CONFERENCE_DATA" => Some(EslEventType::ConferenceData),
            "CALL_SETUP_REQ" => Some(EslEventType::CallSetupReq),
            "CALL_SETUP_RESULT" => Some(EslEventType::CallSetupResult),
            "CALL_DETAIL" => Some(EslEventType::CallDetail),
            "DEVICE_STATE" => Some(EslEventType::DeviceState),
            "TEXT" => Some(EslEventType::Text),
            "SHUTDOWN_REQUESTED" => Some(EslEventType::ShutdownRequested),
            "ALL" => Some(EslEventType::All),
            // Not in libs/esl/ EVENT_NAMES[]
            "START_RECORDING" => Some(EslEventType::StartRecording),
            _ => None,
        }
    }
}

/// Event priority levels matching FreeSWITCH `esl_priority_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EslEventPriority {
    Normal,
    Low,
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

impl FromStr for EslEventPriority {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s
            .to_uppercase()
            .as_str()
        {
            "NORMAL" => Ok(EslEventPriority::Normal),
            "LOW" => Ok(EslEventPriority::Low),
            "HIGH" => Ok(EslEventPriority::High),
            _ => Err(()),
        }
    }
}

/// ESL Event structure containing headers and optional body
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EslEvent {
    /// Event type
    pub event_type: Option<EslEventType>,
    /// Event headers as key-value pairs
    pub headers: HashMap<String, String>,
    /// Optional event body content
    pub body: Option<String>,
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

    /// Get event type
    pub fn event_type(&self) -> Option<EslEventType> {
        self.event_type
    }

    /// Get header value by name
    pub fn header(&self, name: &str) -> Option<&String> {
        self.headers
            .get(name)
    }

    /// Set header value
    pub fn set_header(&mut self, name: String, value: String) {
        self.headers
            .insert(name, value);
    }

    /// Remove a header, returning its value if it existed
    pub fn del_header(&mut self, name: &str) -> Option<String> {
        self.headers
            .remove(name)
    }

    /// Get event body
    pub fn body(&self) -> Option<&String> {
        self.body
            .as_ref()
    }

    /// Set event body
    pub fn set_body(&mut self, body: String) {
        self.body = Some(body);
    }

    /// Set event priority, adding a `priority` header.
    pub fn set_priority(&mut self, priority: EslEventPriority) {
        self.set_header("priority".into(), priority.to_string());
    }

    /// Get event priority from the `priority` header.
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
    pub fn push_header(&mut self, name: &str, value: &str) {
        self.stack_header(name, value, EslArray::push);
    }

    /// Prepend a value to a multi-value header (UNSHIFT semantics).
    ///
    /// Same conversion rules as `push_header()`, but inserts at the front.
    pub fn unshift_header(&mut self, name: &str, value: &str) {
        self.stack_header(name, value, EslArray::unshift);
    }

    fn stack_header(&mut self, name: &str, value: &str, op: fn(&mut EslArray, String)) {
        match self
            .headers
            .get(name)
        {
            None => {
                self.set_header(name.into(), value.into());
            }
            Some(existing) => {
                let mut arr = match EslArray::parse(existing) {
                    Some(arr) => arr,
                    None => EslArray::new(vec![existing.clone()]),
                };
                op(&mut arr, value.into());
                self.set_header(name.into(), arr.to_string());
            }
        }
    }

    /// Get unique ID for the event/channel
    pub fn unique_id(&self) -> Option<&String> {
        self.header("Unique-ID")
            .or_else(|| self.header("Caller-Unique-ID"))
    }

    /// Get job UUID for background jobs
    pub fn job_uuid(&self) -> Option<&String> {
        self.header("Job-UUID")
    }

    /// Check if this is a specific event type
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
        let mut result = String::new();

        if let Some(event_name) = self
            .headers
            .get("Event-Name")
        {
            result.push_str(&format!(
                "Event-Name: {}\n",
                percent_encode(event_name.as_bytes(), NON_ALPHANUMERIC)
            ));
        }

        let mut sorted_headers: Vec<_> = self
            .headers
            .iter()
            .filter(|(k, _)| k.as_str() != "Event-Name" && k.as_str() != "Content-Length")
            .collect();
        sorted_headers.sort_by_key(|(k, _)| k.as_str());

        for (key, value) in sorted_headers {
            result.push_str(&format!(
                "{}: {}\n",
                key,
                percent_encode(value.as_bytes(), NON_ALPHANUMERIC)
            ));
        }

        if let Some(body) = &self.body {
            result.push_str(&format!("Content-Length: {}\n", body.len()));
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
    fn test_del_header() {
        let mut event = EslEvent::new();
        event.set_header("Foo".to_string(), "bar".to_string());
        event.set_header("Baz".to_string(), "qux".to_string());

        let removed = event.del_header("Foo");
        assert_eq!(removed, Some("bar".to_string()));
        assert!(event
            .header("Foo")
            .is_none());
        assert_eq!(event.header("Baz"), Some(&"qux".to_string()));

        let removed_again = event.del_header("Foo");
        assert_eq!(removed_again, None);
    }

    #[test]
    fn test_to_plain_format_basic() {
        let mut event = EslEvent::with_type(EslEventType::Heartbeat);
        event.set_header("Event-Name".to_string(), "HEARTBEAT".to_string());
        event.set_header("Core-UUID".to_string(), "abc-123".to_string());

        let plain = event.to_plain_format();

        assert!(plain.starts_with("Event-Name: "));
        assert!(plain.contains("Core-UUID: "));
        assert!(plain.ends_with("\n\n"));
    }

    #[test]
    fn test_to_plain_format_percent_encoding() {
        let mut event = EslEvent::with_type(EslEventType::Heartbeat);
        event.set_header("Event-Name".to_string(), "HEARTBEAT".to_string());
        event.set_header("Up-Time".to_string(), "0 years, 0 days".to_string());

        let plain = event.to_plain_format();

        assert!(!plain.contains("0 years, 0 days"));
        assert!(plain.contains("Up-Time: "));
        assert!(plain.contains("%20"));
    }

    #[test]
    fn test_to_plain_format_with_body() {
        let mut event = EslEvent::with_type(EslEventType::BackgroundJob);
        event.set_header("Event-Name".to_string(), "BACKGROUND_JOB".to_string());
        event.set_header("Job-UUID".to_string(), "def-456".to_string());
        event.set_body("+OK result\n".to_string());

        let plain = event.to_plain_format();

        assert!(plain.contains("Content-Length: 11\n"));
        assert!(plain.ends_with("\n\n+OK result\n"));
    }

    #[test]
    fn test_to_plain_format_round_trip() {
        use crate::protocol::{EslMessage, EslParser, MessageType};

        let mut original = EslEvent::with_type(EslEventType::Heartbeat);
        original.set_header("Event-Name".to_string(), "HEARTBEAT".to_string());
        original.set_header("Core-UUID".to_string(), "abc-123".to_string());
        original.set_header("Up-Time".to_string(), "0 years, 0 days, 1 hour".to_string());
        original.set_header("Event-Info".to_string(), "System Ready".to_string());

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
        original.set_header("Event-Name".to_string(), "BACKGROUND_JOB".to_string());
        original.set_header("Job-UUID".to_string(), "job-789".to_string());
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
        assert_eq!(event.header("priority"), Some(&"NORMAL".to_string()));
    }

    #[test]
    fn test_set_priority_high() {
        let mut event = EslEvent::new();
        event.set_priority(EslEventPriority::High);
        assert_eq!(event.priority(), Some(EslEventPriority::High));
        assert_eq!(event.header("priority"), Some(&"HIGH".to_string()));
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
        assert_eq!(event.header("X-Test"), Some(&"first".to_string()));
    }

    #[test]
    fn test_push_header_existing_plain() {
        let mut event = EslEvent::new();
        event.set_header("X-Test".into(), "first".into());
        event.push_header("X-Test", "second");
        assert_eq!(
            event.header("X-Test"),
            Some(&"ARRAY::first|:second".to_string())
        );
    }

    #[test]
    fn test_push_header_existing_array() {
        let mut event = EslEvent::new();
        event.set_header("X-Test".into(), "ARRAY::a|:b".into());
        event.push_header("X-Test", "c");
        assert_eq!(event.header("X-Test"), Some(&"ARRAY::a|:b|:c".to_string()));
    }

    #[test]
    fn test_unshift_header_new() {
        let mut event = EslEvent::new();
        event.unshift_header("X-Test", "only");
        assert_eq!(event.header("X-Test"), Some(&"only".to_string()));
    }

    #[test]
    fn test_unshift_header_existing_array() {
        let mut event = EslEvent::new();
        event.set_header("X-Test".into(), "ARRAY::b|:c".into());
        event.unshift_header("X-Test", "a");
        assert_eq!(event.header("X-Test"), Some(&"ARRAY::a|:b|:c".to_string()));
    }

    #[test]
    fn test_sendevent_with_priority_wire_format() {
        let mut event = EslEvent::with_type(EslEventType::Custom);
        event.set_header("Event-Name".into(), "CUSTOM".into());
        event.set_header("Event-Subclass".into(), "test::priority".into());
        event.set_priority(EslEventPriority::High);

        let plain = event.to_plain_format();
        assert!(plain.contains("priority: HIGH\n"));
    }
}
