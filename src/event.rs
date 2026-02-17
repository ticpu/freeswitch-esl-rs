//! ESL event types and structures

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

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

/// FreeSWITCH event types based on the 143 events from esl_event.h
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
    ChannelOutbound,
    ChannelPark,
    ChannelUnpark,
    ChannelApplication,
    ChannelOriginate,
    ChannelUuid,
    ApiCommand,
    ReSchedule,
    ReloadXml,
    Notify,
    NotifyIn,
    SendMessage,
    RecvMessage,
    RequestParams,
    ChannelData,
    General,
    Command,
    SessionBegin,
    SessionEnd,
    SessionHearbeat,
    ClientDisconnected,
    ServerDisconnected,
    SendInfo,
    RecvInfo,
    RecvRtcpMessage,
    CallSecure,
    Nat,
    RecordStart,
    RecordStop,
    RecordPause,
    RecordUnpause,
    PlaybackStart,
    PlaybackStop,
    PlaybackPause,
    PlaybackUnpause,
    DtmfCapture,
    DetectedSpeech,
    DetectedTone,
    PrivateCommand,
    Heartbeat,
    Trap,
    AddSchedule,
    DelSchedule,
    ExeSchedule,
    ReSchedule2,
    LogLevel,
    Dtmf,
    Message,
    Presence,
    MessageQuery,
    Rosterin,
    Rosterout,
    Codec,
    BackgroundJob,
    DetectedTone2,
    ConferenceDataQuery,
    ConferenceData,
    CallSetupReq,
    CallSetupResult,
    CallDetail,
    DeviceState,
    AllWsEvent,
    PopupIn,
    PopupOut,
    Zrtp,
    TextMessages,
    /// Subscribe to all events
    All,
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
            EslEventType::ChannelOutbound => "CHANNEL_OUTBOUND",
            EslEventType::ChannelPark => "CHANNEL_PARK",
            EslEventType::ChannelUnpark => "CHANNEL_UNPARK",
            EslEventType::ChannelApplication => "CHANNEL_APPLICATION",
            EslEventType::ChannelOriginate => "CHANNEL_ORIGINATE",
            EslEventType::ChannelUuid => "CHANNEL_UUID",
            EslEventType::ApiCommand => "API_COMMAND",
            EslEventType::ReSchedule => "RESCHEDULE",
            EslEventType::ReloadXml => "RELOAD_XML",
            EslEventType::Notify => "NOTIFY",
            EslEventType::NotifyIn => "NOTIFY_IN",
            EslEventType::SendMessage => "SEND_MESSAGE",
            EslEventType::RecvMessage => "RECV_MESSAGE",
            EslEventType::RequestParams => "REQUEST_PARAMS",
            EslEventType::ChannelData => "CHANNEL_DATA",
            EslEventType::General => "GENERAL",
            EslEventType::Command => "COMMAND",
            EslEventType::SessionBegin => "SESSION_BEGIN",
            EslEventType::SessionEnd => "SESSION_END",
            EslEventType::SessionHearbeat => "SESSION_HEARTBEAT",
            EslEventType::ClientDisconnected => "CLIENT_DISCONNECTED",
            EslEventType::ServerDisconnected => "SERVER_DISCONNECTED",
            EslEventType::SendInfo => "SEND_INFO",
            EslEventType::RecvInfo => "RECV_INFO",
            EslEventType::RecvRtcpMessage => "RECV_RTCP_MESSAGE",
            EslEventType::CallSecure => "CALL_SECURE",
            EslEventType::Nat => "NAT",
            EslEventType::RecordStart => "RECORD_START",
            EslEventType::RecordStop => "RECORD_STOP",
            EslEventType::RecordPause => "RECORD_PAUSE",
            EslEventType::RecordUnpause => "RECORD_UNPAUSE",
            EslEventType::PlaybackStart => "PLAYBACK_START",
            EslEventType::PlaybackStop => "PLAYBACK_STOP",
            EslEventType::PlaybackPause => "PLAYBACK_PAUSE",
            EslEventType::PlaybackUnpause => "PLAYBACK_UNPAUSE",
            EslEventType::DtmfCapture => "DTMF_CAPTURE",
            EslEventType::DetectedSpeech => "DETECTED_SPEECH",
            EslEventType::DetectedTone => "DETECTED_TONE",
            EslEventType::PrivateCommand => "PRIVATE_COMMAND",
            EslEventType::Heartbeat => "HEARTBEAT",
            EslEventType::Trap => "TRAP",
            EslEventType::AddSchedule => "ADD_SCHEDULE",
            EslEventType::DelSchedule => "DEL_SCHEDULE",
            EslEventType::ExeSchedule => "EXE_SCHEDULE",
            EslEventType::ReSchedule2 => "RE_SCHEDULE",
            EslEventType::LogLevel => "LOG_LEVEL",
            EslEventType::Dtmf => "DTMF",
            EslEventType::Message => "MESSAGE",
            EslEventType::Presence => "PRESENCE",
            EslEventType::MessageQuery => "MESSAGE_QUERY",
            EslEventType::Rosterin => "ROSTER_IN",
            EslEventType::Rosterout => "ROSTER_OUT",
            EslEventType::Codec => "CODEC",
            EslEventType::BackgroundJob => "BACKGROUND_JOB",
            EslEventType::DetectedTone2 => "DETECTED_TONE",
            EslEventType::ConferenceDataQuery => "CONFERENCE_DATA_QUERY",
            EslEventType::ConferenceData => "CONFERENCE_DATA",
            EslEventType::CallSetupReq => "CALL_SETUP_REQ",
            EslEventType::CallSetupResult => "CALL_SETUP_RESULT",
            EslEventType::CallDetail => "CALL_DETAIL",
            EslEventType::DeviceState => "DEVICE_STATE",
            EslEventType::AllWsEvent => "ALL",
            EslEventType::PopupIn => "POPUP_IN",
            EslEventType::PopupOut => "POPUP_OUT",
            EslEventType::Zrtp => "ZRTP",
            EslEventType::TextMessages => "TEXT_MESSAGES",
            EslEventType::All => "ALL",
        };
        write!(f, "{}", name)
    }
}

impl EslEventType {
    /// Parse event type from string name
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
            "CHANNEL_OUTBOUND" => Some(EslEventType::ChannelOutbound),
            "CHANNEL_PARK" => Some(EslEventType::ChannelPark),
            "CHANNEL_UNPARK" => Some(EslEventType::ChannelUnpark),
            "CHANNEL_APPLICATION" => Some(EslEventType::ChannelApplication),
            "CHANNEL_ORIGINATE" => Some(EslEventType::ChannelOriginate),
            "CHANNEL_UUID" => Some(EslEventType::ChannelUuid),
            "API_COMMAND" => Some(EslEventType::ApiCommand),
            "RESCHEDULE" => Some(EslEventType::ReSchedule),
            "RELOAD_XML" => Some(EslEventType::ReloadXml),
            "NOTIFY" => Some(EslEventType::Notify),
            "NOTIFY_IN" => Some(EslEventType::NotifyIn),
            "SEND_MESSAGE" => Some(EslEventType::SendMessage),
            "RECV_MESSAGE" => Some(EslEventType::RecvMessage),
            "REQUEST_PARAMS" => Some(EslEventType::RequestParams),
            "CHANNEL_DATA" => Some(EslEventType::ChannelData),
            "GENERAL" => Some(EslEventType::General),
            "COMMAND" => Some(EslEventType::Command),
            "SESSION_BEGIN" => Some(EslEventType::SessionBegin),
            "SESSION_END" => Some(EslEventType::SessionEnd),
            "SESSION_HEARTBEAT" => Some(EslEventType::SessionHearbeat),
            "CLIENT_DISCONNECTED" => Some(EslEventType::ClientDisconnected),
            "SERVER_DISCONNECTED" => Some(EslEventType::ServerDisconnected),
            "SEND_INFO" => Some(EslEventType::SendInfo),
            "RECV_INFO" => Some(EslEventType::RecvInfo),
            "RECV_RTCP_MESSAGE" => Some(EslEventType::RecvRtcpMessage),
            "CALL_SECURE" => Some(EslEventType::CallSecure),
            "NAT" => Some(EslEventType::Nat),
            "RECORD_START" => Some(EslEventType::RecordStart),
            "RECORD_STOP" => Some(EslEventType::RecordStop),
            "RECORD_PAUSE" => Some(EslEventType::RecordPause),
            "RECORD_UNPAUSE" => Some(EslEventType::RecordUnpause),
            "PLAYBACK_START" => Some(EslEventType::PlaybackStart),
            "PLAYBACK_STOP" => Some(EslEventType::PlaybackStop),
            "PLAYBACK_PAUSE" => Some(EslEventType::PlaybackPause),
            "PLAYBACK_UNPAUSE" => Some(EslEventType::PlaybackUnpause),
            "DTMF_CAPTURE" => Some(EslEventType::DtmfCapture),
            "DETECTED_SPEECH" => Some(EslEventType::DetectedSpeech),
            "DETECTED_TONE" => Some(EslEventType::DetectedTone),
            "PRIVATE_COMMAND" => Some(EslEventType::PrivateCommand),
            "HEARTBEAT" => Some(EslEventType::Heartbeat),
            "TRAP" => Some(EslEventType::Trap),
            "ADD_SCHEDULE" => Some(EslEventType::AddSchedule),
            "DEL_SCHEDULE" => Some(EslEventType::DelSchedule),
            "EXE_SCHEDULE" => Some(EslEventType::ExeSchedule),
            "RE_SCHEDULE" => Some(EslEventType::ReSchedule2),
            "LOG_LEVEL" => Some(EslEventType::LogLevel),
            "DTMF" => Some(EslEventType::Dtmf),
            "MESSAGE" => Some(EslEventType::Message),
            "PRESENCE" => Some(EslEventType::Presence),
            "MESSAGE_QUERY" => Some(EslEventType::MessageQuery),
            "ROSTER_IN" => Some(EslEventType::Rosterin),
            "ROSTER_OUT" => Some(EslEventType::Rosterout),
            "CODEC" => Some(EslEventType::Codec),
            "BACKGROUND_JOB" => Some(EslEventType::BackgroundJob),
            "CONFERENCE_DATA_QUERY" => Some(EslEventType::ConferenceDataQuery),
            "CONFERENCE_DATA" => Some(EslEventType::ConferenceData),
            "CALL_SETUP_REQ" => Some(EslEventType::CallSetupReq),
            "CALL_SETUP_RESULT" => Some(EslEventType::CallSetupResult),
            "CALL_DETAIL" => Some(EslEventType::CallDetail),
            "DEVICE_STATE" => Some(EslEventType::DeviceState),
            "POPUP_IN" => Some(EslEventType::PopupIn),
            "POPUP_OUT" => Some(EslEventType::PopupOut),
            "ZRTP" => Some(EslEventType::Zrtp),
            "TEXT_MESSAGES" => Some(EslEventType::TextMessages),
            "ALL" => Some(EslEventType::All),
            _ => None,
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

    /// Get event body
    pub fn body(&self) -> Option<&String> {
        self.body
            .as_ref()
    }

    /// Set event body
    pub fn set_body(&mut self, body: String) {
        self.body = Some(body);
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
}
