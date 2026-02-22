//! Channel-related data types extracted from ESL event headers.

use crate::event::EslEvent;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

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

/// Channel timing data from FreeSWITCH's `switch_channel_timetable_t`.
///
/// Timestamps are epoch microseconds (`i64`). A value of `0` means the
/// corresponding event never occurred (e.g., `hungup == Some(0)` means
/// the channel has not hung up yet). `None` means the header was absent
/// or unparseable.
///
/// Extracted from ESL event headers using a prefix (typically `"Caller"`
/// or `"Other-Leg"`). The wire header format is `{prefix}-{suffix}`,
/// e.g. `Caller-Channel-Created-Time`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChannelTimetable {
    /// When the caller profile was created.
    pub profile_created: Option<i64>,
    /// When the channel was created.
    pub created: Option<i64>,
    /// When the channel was answered.
    pub answered: Option<i64>,
    /// When early media (183) was received.
    pub progress: Option<i64>,
    /// When media-bearing early media arrived.
    pub progress_media: Option<i64>,
    /// When the channel hung up.
    pub hungup: Option<i64>,
    /// When the channel was transferred.
    pub transferred: Option<i64>,
    /// When the channel was resurrected.
    pub resurrected: Option<i64>,
    /// When the channel was bridged.
    pub bridged: Option<i64>,
    /// Timestamp of the last hold event.
    pub last_hold: Option<i64>,
    /// Accumulated hold time in microseconds.
    pub hold_accum: Option<i64>,
}

impl ChannelTimetable {
    /// Extract a timetable from event headers with the given prefix.
    ///
    /// Returns `None` if no timestamp headers with this prefix are present
    /// or parseable. Common prefixes: `"Caller"`, `"Other-Leg"`.
    pub fn from_event(event: &EslEvent, prefix: &str) -> Option<Self> {
        let mut tt = Self::default();
        let mut found = false;

        macro_rules! field {
            ($field:ident, $suffix:literal) => {
                let header = format!("{}-{}", prefix, $suffix);
                if let Some(v) = event
                    .header(&header)
                    .and_then(|s| {
                        s.parse()
                            .ok()
                    })
                {
                    tt.$field = Some(v);
                    found = true;
                }
            };
        }

        field!(profile_created, "Profile-Created-Time");
        field!(created, "Channel-Created-Time");
        field!(answered, "Channel-Answered-Time");
        field!(progress, "Channel-Progress-Time");
        field!(progress_media, "Channel-Progress-Media-Time");
        field!(hungup, "Channel-Hangup-Time");
        field!(transferred, "Channel-Transfer-Time");
        field!(resurrected, "Channel-Resurrect-Time");
        field!(bridged, "Channel-Bridged-Time");
        field!(last_hold, "Channel-Last-Hold");
        field!(hold_accum, "Channel-Hold-Accum");

        if found {
            Some(tt)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // --- ChannelTimetable tests ---

    #[test]
    fn caller_timetable_all_fields() {
        let mut event = EslEvent::new();
        event.set_header("Caller-Profile-Created-Time", "1700000000000000");
        event.set_header("Caller-Channel-Created-Time", "1700000001000000");
        event.set_header("Caller-Channel-Answered-Time", "1700000005000000");
        event.set_header("Caller-Channel-Progress-Time", "1700000002000000");
        event.set_header("Caller-Channel-Progress-Media-Time", "1700000003000000");
        event.set_header("Caller-Channel-Hangup-Time", "0");
        event.set_header("Caller-Channel-Transfer-Time", "0");
        event.set_header("Caller-Channel-Resurrect-Time", "0");
        event.set_header("Caller-Channel-Bridged-Time", "1700000006000000");
        event.set_header("Caller-Channel-Last-Hold", "0");
        event.set_header("Caller-Channel-Hold-Accum", "0");

        let tt = event
            .caller_timetable()
            .expect("should parse");
        assert_eq!(tt.profile_created, Some(1700000000000000));
        assert_eq!(tt.created, Some(1700000001000000));
        assert_eq!(tt.answered, Some(1700000005000000));
        assert_eq!(tt.progress, Some(1700000002000000));
        assert_eq!(tt.progress_media, Some(1700000003000000));
        assert_eq!(tt.hungup, Some(0));
        assert_eq!(tt.transferred, Some(0));
        assert_eq!(tt.resurrected, Some(0));
        assert_eq!(tt.bridged, Some(1700000006000000));
        assert_eq!(tt.last_hold, Some(0));
        assert_eq!(tt.hold_accum, Some(0));
    }

    #[test]
    fn other_leg_timetable() {
        let mut event = EslEvent::new();
        event.set_header("Other-Leg-Profile-Created-Time", "1700000000000000");
        event.set_header("Other-Leg-Channel-Created-Time", "1700000001000000");
        event.set_header("Other-Leg-Channel-Answered-Time", "1700000005000000");
        event.set_header("Other-Leg-Channel-Progress-Time", "0");
        event.set_header("Other-Leg-Channel-Progress-Media-Time", "0");
        event.set_header("Other-Leg-Channel-Hangup-Time", "0");
        event.set_header("Other-Leg-Channel-Transfer-Time", "0");
        event.set_header("Other-Leg-Channel-Resurrect-Time", "0");
        event.set_header("Other-Leg-Channel-Bridged-Time", "1700000006000000");
        event.set_header("Other-Leg-Channel-Last-Hold", "0");
        event.set_header("Other-Leg-Channel-Hold-Accum", "0");

        let tt = event
            .other_leg_timetable()
            .expect("should parse");
        assert_eq!(tt.created, Some(1700000001000000));
        assert_eq!(tt.bridged, Some(1700000006000000));
    }

    #[test]
    fn timetable_no_headers() {
        let event = EslEvent::new();
        assert!(event
            .caller_timetable()
            .is_none());
        assert!(event
            .other_leg_timetable()
            .is_none());
    }

    #[test]
    fn timetable_partial_headers() {
        let mut event = EslEvent::new();
        event.set_header("Caller-Channel-Created-Time", "1700000001000000");

        let tt = event
            .caller_timetable()
            .expect("at least one field parsed");
        assert_eq!(tt.created, Some(1700000001000000));
        assert_eq!(tt.answered, None);
        assert_eq!(tt.profile_created, None);
    }

    #[test]
    fn timetable_invalid_value_only() {
        let mut event = EslEvent::new();
        event.set_header("Caller-Channel-Created-Time", "not_a_number");

        assert!(event
            .caller_timetable()
            .is_none());
    }

    #[test]
    fn timetable_zero_preserved() {
        let mut event = EslEvent::new();
        event.set_header("Caller-Channel-Hangup-Time", "0");

        let tt = event
            .caller_timetable()
            .expect("should parse");
        assert_eq!(tt.hungup, Some(0));
    }

    #[test]
    fn timetable_custom_prefix() {
        let mut event = EslEvent::new();
        event.set_header("Channel-Channel-Created-Time", "1700000001000000");

        let tt = event
            .timetable("Channel")
            .expect("custom prefix should work");
        assert_eq!(tt.created, Some(1700000001000000));
    }
}
