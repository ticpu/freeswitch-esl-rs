//! FreeSWITCH protocol types: channel state, events, headers, commands, and variables.
// The esl_event_types! macro uses TT-munching filter arms over ~90 variants,
// requiring a higher recursion limit than the default 128.
#![recursion_limit = "512"]
//!
//! This crate provides the domain types for FreeSWITCH's Event Socket Library (ESL)
//! protocol without any async runtime dependency. Use it standalone for CDR parsing,
//! config generation, command building, or channel variable validation.
//!
//! For async ESL transport (connecting to FreeSWITCH, sending commands, receiving events),
//! see the [`freeswitch-esl-tokio`](https://docs.rs/freeswitch-esl-tokio) crate which
//! re-exports everything from this crate.
//!
//! # SIP header types
//!
//! General-purpose SIP header parsing is provided by the
//! [`sip-header`](https://docs.rs/sip-header) crate, re-exported here for convenience.
//! Types like [`SipHeaderAddr`], [`UriInfo`], [`HistoryInfo`], and [`SipGeolocation`]
//! are available from the crate root.

pub use sip_header;
pub use sip_header::define_header_enum;
pub use sip_header::sip_uri;

#[macro_use]
mod macros;

pub mod channel;
#[cfg(feature = "esl")]
pub mod commands;
#[cfg(feature = "esl")]
pub mod event;
pub mod headers;
pub mod lookup;
#[cfg(feature = "esl")]
pub mod lossy_values;
pub mod prelude;
#[cfg(feature = "sdp")]
pub mod sdp;
pub mod sofia;
pub mod variables;
#[doc(hidden)]
pub mod wire_safety;

/// Default FreeSWITCH ESL port for inbound connections.
pub const DEFAULT_ESL_PORT: u16 = 8021;

/// Default FreeSWITCH ESL password (`ClueCon`).
pub const DEFAULT_ESL_PASSWORD: &str = "ClueCon";

/// Header-name prefix a channel variable carries in an event.
///
/// Compose with [`VariableName::header_name()`](crate::VariableName::header_name)
/// rather than by hand.
pub const VARIABLE_PREFIX: &str = "variable_";

pub use channel::{
    channel_driver, AnswerState, CallDirection, CallState, ChannelState, ChannelTimetable,
    HangupCause, ParseAnswerStateError, ParseCallDirectionError, ParseCallStateError,
    ParseChannelStateError, ParseHangupCauseError, ParseTimetableError, TimetableField,
    TimetablePrefix,
};
#[cfg(feature = "esl")]
pub use commands::{
    Application, BridgeDialString, DialString, DialStringCarrier, DialplanType, Endpoint,
    EndpointDisplay, GroupCallOrder, Originate, OriginateError, OriginateTarget,
    ParseDialplanTypeError, ParseGroupCallOrderError, UuidAnswer, UuidBridge, UuidDeflect,
    UuidGetVar, UuidHold, UuidKill, UuidSendDtmf, UuidSetVar, UuidTransfer, Variables,
    VariablesDisplay, VariablesType,
};
#[cfg(feature = "esl")]
pub use event::{
    EslEvent, EslEventPriority, EslEventType, EventFormat, EventSubscription,
    EventSubscriptionError, ParseEventFormatError, ParseEventTypeError, ParsePriorityError,
};
pub use headers::{case_alias_key, normalize_header_key, EventHeader, ParseEventHeaderError};
pub use lookup::HeaderLookup;
#[cfg(feature = "esl")]
pub use lossy_values::{LossyValue, LossyValues};
pub use sip_header::{
    extract_header, HistoryInfo, HistoryInfoEntry, HistoryInfoError, HistoryInfoReason,
    ParseSipHeaderAddrError, ParseSipHeaderError, SipGeolocation, SipGeolocationRef, SipHeader,
    SipHeaderAddr, SipHeaderLookup, UriInfo, UriInfoEntry, UriInfoError,
};
pub use sofia::{
    GatewayPingStatus, GatewayRegState, ParseGatewayPingStatusError, ParseGatewayRegStateError,
    ParseSipUserPingStatusError, ParseSofiaEventSubclassError, SipUserPingStatus, SofiaChannelName,
    SofiaEventSubclass,
};
#[cfg(feature = "esl")]
pub use variables::EslHeaders;
pub use variables::{
    ChannelVariable, CoreMediaVariable, EslArray, EslArrayError, LoopbackChannelName,
    LoopbackHangupCause, LoopbackLeg, LoopbackResignation, MultipartBody, MultipartBodyError,
    MultipartItem, ParseChannelVariableError, ParseCoreMediaVariableError,
    ParseLoopbackHangupCauseError, ParseLoopbackLegError, RtpStatUnit, SipHeaderPrefix,
    SipPassthroughHeader, VariableName, MAX_ARRAY_ITEMS,
};
