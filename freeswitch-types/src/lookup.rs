//! Shared trait for typed header lookups from any key-value store.
//!
//! [`HeaderLookup`] provides convenience accessors (typed channel state,
//! call direction, timetable extraction, etc.) to any type that can look up
//! headers and variables by name. Implement the two required methods and get
//! everything else for free.

use crate::channel::{
    AnswerState, CallDirection, CallState, ChannelState, ChannelTimetable, HangupCause,
    ParseAnswerStateError, ParseCallDirectionError, ParseCallStateError, ParseChannelStateError,
    ParseHangupCauseError, ParseTimetableError,
};
#[cfg(feature = "esl")]
use crate::event::{EslEventPriority, ParsePriorityError};
use crate::headers::EventHeader;
use crate::sofia::{
    GatewayPingStatus, GatewayRegState, ParseGatewayPingStatusError, ParseGatewayRegStateError,
    ParseSipUserPingStatusError, ParseSofiaEventSubclassError, SipUserPingStatus,
    SofiaEventSubclass,
};
use crate::variables::{LoopbackResignation, LoopbackVariable, VariableName};
use sip_header::SipHeaderLookup;
use std::str::FromStr;

/// Parse an optional wire header value into a typed result.
///
/// Collapses the common `match self.header(...) { Some(s) => Ok(Some(s.parse()?)),
/// None => Ok(None) }` pattern used by many `HeaderLookup` accessors.
#[inline]
fn parse_opt<T: FromStr>(raw: Option<&str>) -> Result<Option<T>, T::Err> {
    raw.map(str::parse)
        .transpose()
}

/// Trait for looking up ESL headers and channel variables from any key-value store.
///
/// Implementors provide two methods -- `header_str(&str)` and `variable_str(&str)` --
/// and get all typed accessors (`channel_state()`, `call_direction()`, `timetable()`,
/// etc.) as default implementations.
///
/// `HeaderLookup: SipHeaderLookup` — every `HeaderLookup` implementor must also
/// implement [`sip_header::SipHeaderLookup`] so callers can reach the typed RFC
/// SIP parsers (`call_info()`, `history_info()`, `geolocation()`) directly from
/// the same value. For stores that treat ESL headers and SIP headers as the
/// same flat key-value namespace, a one-line delegation from `sip_header_str`
/// to `header_str` is the intended impl — see [`EslEvent`](crate::EslEvent)
/// for the pattern. Stores with FreeSWITCH-specific transport encoding (ARRAY,
/// `[bracket]` wrapping) should use [`EslHeaders`](crate::EslHeaders), which
/// overrides the `SipHeaderLookup` parsers to strip those quirks.
///
/// This trait must be in scope to call its methods on `EslEvent` -- including
/// `unique_id()`, `hangup_cause()`, and `channel_state()`. Import it directly
/// or via the prelude:
///
/// ```ignore
/// use freeswitch_esl_tokio::prelude::*;
/// // or: use freeswitch_esl_tokio::HeaderLookup;
/// // or: use freeswitch_types::HeaderLookup;
/// ```
///
/// # Example
///
/// ```
/// use std::collections::HashMap;
/// use freeswitch_types::{HeaderLookup, EventHeader, ChannelVariable, VARIABLE_PREFIX};
/// use freeswitch_types::sip_header::SipHeaderLookup;
///
/// struct MyStore(HashMap<String, String>);
///
/// // HeaderLookup has SipHeaderLookup as a supertrait. For stores that
/// // treat ESL and SIP headers as one flat namespace, delegate
/// // sip_header_str to the same lookup.
/// impl SipHeaderLookup for MyStore {
///     fn sip_header_str(&self, name: &str) -> Option<&str> {
///         self.0.get(name).map(|s| s.as_str())
///     }
/// }
///
/// impl HeaderLookup for MyStore {
///     fn header_str(&self, name: &str) -> Option<&str> {
///         self.0.get(name).map(|s| s.as_str())
///     }
///     fn variable_str(&self, name: &str) -> Option<&str> {
///         self.0.get(&format!("{VARIABLE_PREFIX}{name}")).map(|s| s.as_str())
///     }
/// }
///
/// let mut map = HashMap::new();
/// map.insert("Channel-State".into(), "CS_EXECUTE".into());
/// map.insert("variable_read_codec".into(), "PCMU".into());
/// let store = MyStore(map);
///
/// // Typed accessor from the trait (returns Result<Option<T>, E>):
/// assert!(store.channel_state().unwrap().is_some());
///
/// // Enum-based lookups:
/// assert_eq!(store.header(EventHeader::ChannelState), Some("CS_EXECUTE"));
/// assert_eq!(store.variable(ChannelVariable::ReadCodec), Some("PCMU"));
/// ```
pub trait HeaderLookup: SipHeaderLookup {
    /// Look up a header by its raw wire name (e.g. `"Unique-ID"`).
    fn header_str(&self, name: &str) -> Option<&str>;

    /// Look up a channel variable by its bare name (e.g. `"sip_call_id"`).
    ///
    /// Implementations typically prepend `variable_` and delegate to `header_str`.
    fn variable_str(&self, name: &str) -> Option<&str>;

    /// Look up a header by its [`EventHeader`] enum variant.
    fn header(&self, name: EventHeader) -> Option<&str> {
        self.header_str(name.as_str())
    }

    /// Look up a channel variable by its typed enum variant.
    fn variable(&self, name: impl VariableName) -> Option<&str> {
        self.variable_str(name.as_str())
    }

    /// `Unique-ID` header, falling back to `Caller-Unique-ID`.
    fn unique_id(&self) -> Option<&str> {
        self.header(EventHeader::UniqueId)
            .or_else(|| self.header(EventHeader::CallerUniqueId))
    }

    /// `Job-UUID` header from `bgapi` `BACKGROUND_JOB` events.
    ///
    /// The only thing separating your job's result from another ESL client's:
    /// `BACKGROUND_JOB` is fired globally, so seeing one you did not issue is
    /// routine rather than exceptional.
    fn job_uuid(&self) -> Option<&str> {
        self.header(EventHeader::JobUuid)
    }

    /// `Channel-Name` header (e.g. `sofia/internal/1000@domain`).
    fn channel_name(&self) -> Option<&str> {
        self.header(EventHeader::ChannelName)
    }

    /// `Caller-Caller-ID-Number` header.
    fn caller_id_number(&self) -> Option<&str> {
        self.header(EventHeader::CallerCallerIdNumber)
    }

    /// `Caller-Caller-ID-Name` header.
    fn caller_id_name(&self) -> Option<&str> {
        self.header(EventHeader::CallerCallerIdName)
    }

    /// `Caller-Destination-Number` header.
    fn destination_number(&self) -> Option<&str> {
        self.header(EventHeader::CallerDestinationNumber)
    }

    /// `Caller-Callee-ID-Number` header.
    fn callee_id_number(&self) -> Option<&str> {
        self.header(EventHeader::CallerCalleeIdNumber)
    }

    /// `Caller-Callee-ID-Name` header.
    fn callee_id_name(&self) -> Option<&str> {
        self.header(EventHeader::CallerCalleeIdName)
    }

    /// `Channel-Presence-ID` header (e.g. `1000@example.com`).
    fn channel_presence_id(&self) -> Option<&str> {
        self.header(EventHeader::ChannelPresenceId)
    }

    /// `Presence-Call-Direction` header, parsed into a [`CallDirection`].
    fn presence_call_direction(&self) -> Result<Option<CallDirection>, ParseCallDirectionError> {
        parse_opt(self.header(EventHeader::PresenceCallDirection))
    }

    /// `Event-Date-Timestamp` header (microseconds since epoch).
    fn event_date_timestamp(&self) -> Option<&str> {
        self.header(EventHeader::EventDateTimestamp)
    }

    /// `Event-Sequence` header (sequential event counter).
    fn event_sequence(&self) -> Option<&str> {
        self.header(EventHeader::EventSequence)
    }

    /// `DTMF-Duration` header (digit duration in milliseconds).
    fn dtmf_duration(&self) -> Option<&str> {
        self.header(EventHeader::DtmfDuration)
    }

    /// `DTMF-Source` header (e.g. `rtp`, `inband`).
    fn dtmf_source(&self) -> Option<&str> {
        self.header(EventHeader::DtmfSource)
    }

    /// Parse the `Hangup-Cause` header into a [`HangupCause`].
    ///
    /// Returns `Ok(None)` if the header is absent, `Err` if present but unparseable.
    fn hangup_cause(&self) -> Result<Option<HangupCause>, ParseHangupCauseError> {
        parse_opt(self.header(EventHeader::HangupCause))
    }

    /// Detect a mod_loopback bowout on a `CHANNEL_HANGUP_COMPLETE`.
    ///
    /// `Some` means the loopback leg resigned and the call it carried is still
    /// up on [`other_uuid()`](LoopbackResignation::other_uuid) -- re-anchor
    /// tracking there rather than treating the hangup as a teardown. `None` is
    /// a genuine teardown.
    ///
    /// Keyed on the presence of `loopback_hangup_cause`, never its value:
    /// mod_loopback writes a different token depending on which internal path
    /// resigned, and matching one of them silently mis-disposes every call
    /// that took the other.
    fn loopback_resignation(&self) -> Option<LoopbackResignation<'_>> {
        self.variable(LoopbackVariable::LoopbackHangupCause)
            .map(|cause| {
                LoopbackResignation::new(
                    cause,
                    self.variable(LoopbackVariable::LoopbackBowoutOtherUuid),
                )
            })
    }

    /// `Event-Subclass` header for `CUSTOM` events (e.g. `sofia::register`).
    fn event_subclass(&self) -> Option<&str> {
        self.header(EventHeader::EventSubclass)
    }

    /// Parse `Event-Subclass` as a typed [`SofiaEventSubclass`].
    ///
    /// Returns `Ok(None)` if the header is absent, `Err` if present but not a
    /// recognized `sofia::*` subclass.
    fn sofia_event_subclass(
        &self,
    ) -> Result<Option<SofiaEventSubclass>, ParseSofiaEventSubclassError> {
        parse_opt(self.event_subclass())
    }

    /// `Gateway` header from `sofia::gateway_state` / `sofia::gateway_add` events.
    fn gateway(&self) -> Option<&str> {
        self.header(EventHeader::Gateway)
    }

    /// `profile-name` header from `sofia::sip_user_state` events.
    fn profile_name(&self) -> Option<&str> {
        self.header(EventHeader::ProfileName)
    }

    /// `Phrase` header (SIP reason phrase) from sofia state events.
    fn phrase(&self) -> Option<&str> {
        self.header(EventHeader::Phrase)
    }

    /// `Status` header (SIP response code) from `sofia::gateway_state` and
    /// `sofia::sip_user_state` events.
    fn sip_status_code(&self) -> Result<Option<u16>, std::num::ParseIntError> {
        parse_opt(self.header(EventHeader::Status))
    }

    /// Parse the `State` header as a [`GatewayRegState`].
    ///
    /// Returns `Ok(None)` if absent, `Err` if present but unparseable.
    fn gateway_reg_state(&self) -> Result<Option<GatewayRegState>, ParseGatewayRegStateError> {
        parse_opt(self.header(EventHeader::State))
    }

    /// Parse `Ping-Status` as a [`GatewayPingStatus`].
    ///
    /// Use on `sofia::gateway_state` events. For `sofia::sip_user_state`, use
    /// [`sip_user_ping_status()`](Self::sip_user_ping_status) instead.
    fn gateway_ping_status(
        &self,
    ) -> Result<Option<GatewayPingStatus>, ParseGatewayPingStatusError> {
        parse_opt(self.header(EventHeader::PingStatus))
    }

    /// Parse `Ping-Status` as a [`SipUserPingStatus`].
    ///
    /// Use on `sofia::sip_user_state` events. For `sofia::gateway_state`, use
    /// [`gateway_ping_status()`](Self::gateway_ping_status) instead.
    fn sip_user_ping_status(
        &self,
    ) -> Result<Option<SipUserPingStatus>, ParseSipUserPingStatusError> {
        parse_opt(self.header(EventHeader::PingStatus))
    }

    /// `pl_data` header -- SIP NOTIFY body content from `NOTIFY_IN` events.
    ///
    /// Contains the JSON payload (already percent-decoded by the ESL parser).
    /// For NG9-1-1 events this is the inner object without the wrapper key
    /// (FreeSWITCH strips it).
    fn pl_data(&self) -> Option<&str> {
        self.header(EventHeader::PlData)
    }

    /// `event` header -- SIP event package name from `NOTIFY_IN` events.
    ///
    /// Examples: `emergency-AbandonedCall`, `emergency-ServiceState`.
    fn sip_event(&self) -> Option<&str> {
        self.header(EventHeader::SipEvent)
    }

    /// `gateway_name` header -- gateway that received a SIP NOTIFY.
    fn gateway_name(&self) -> Option<&str> {
        self.header(EventHeader::GatewayName)
    }

    /// Parse the `Channel-State` header into a [`ChannelState`].
    ///
    /// Returns `Ok(None)` if the header is absent, `Err` if present but unparseable.
    fn channel_state(&self) -> Result<Option<ChannelState>, ParseChannelStateError> {
        parse_opt(self.header(EventHeader::ChannelState))
    }

    /// Parse the `Channel-State-Number` header into a [`ChannelState`].
    ///
    /// Returns `Ok(None)` if the header is absent, `Err` if present but unparseable.
    fn channel_state_number(&self) -> Result<Option<ChannelState>, ParseChannelStateError> {
        match self.header(EventHeader::ChannelStateNumber) {
            Some(s) => {
                let n: u8 = s
                    .parse()
                    .map_err(|_| ParseChannelStateError(s.to_string()))?;
                ChannelState::from_number(n)
                    .ok_or_else(|| ParseChannelStateError(s.to_string()))
                    .map(Some)
            }
            None => Ok(None),
        }
    }

    /// Parse the `Channel-Call-State` header into a [`CallState`].
    ///
    /// Returns `Ok(None)` if the header is absent, `Err` if present but unparseable.
    fn call_state(&self) -> Result<Option<CallState>, ParseCallStateError> {
        parse_opt(self.header(EventHeader::ChannelCallState))
    }

    /// Parse the `Answer-State` header into an [`AnswerState`].
    ///
    /// Returns `Ok(None)` if the header is absent, `Err` if present but unparseable.
    fn answer_state(&self) -> Result<Option<AnswerState>, ParseAnswerStateError> {
        parse_opt(self.header(EventHeader::AnswerState))
    }

    /// Parse the `Call-Direction` header into a [`CallDirection`].
    ///
    /// Returns `Ok(None)` if the header is absent, `Err` if present but unparseable.
    fn call_direction(&self) -> Result<Option<CallDirection>, ParseCallDirectionError> {
        parse_opt(self.header(EventHeader::CallDirection))
    }

    /// Parse the `priority` header value.
    ///
    /// Returns `Ok(None)` if the header is absent, `Err` if present but unparseable.
    #[cfg(feature = "esl")]
    fn priority(&self) -> Result<Option<EslEventPriority>, ParsePriorityError> {
        parse_opt(self.header(EventHeader::Priority))
    }

    /// Extract timetable from timestamp headers with the given prefix.
    ///
    /// Returns `Ok(None)` if no timestamp headers with this prefix are present.
    /// Returns `Err` if a header is present but contains an invalid value.
    fn timetable(&self, prefix: &str) -> Result<Option<ChannelTimetable>, ParseTimetableError> {
        ChannelTimetable::from_lookup(prefix, |key| self.header_str(key))
    }

    /// Caller-leg channel timetable (`Caller-*-Time` headers).
    fn caller_timetable(&self) -> Result<Option<ChannelTimetable>, ParseTimetableError> {
        self.timetable("Caller")
    }

    /// Other-leg channel timetable (`Other-Leg-*-Time` headers).
    fn other_leg_timetable(&self) -> Result<Option<ChannelTimetable>, ParseTimetableError> {
        self.timetable("Other-Leg")
    }
}

impl HeaderLookup for std::collections::HashMap<String, String> {
    fn header_str(&self, name: &str) -> Option<&str> {
        self.get(name)
            .map(|s| s.as_str())
    }

    fn variable_str(&self, name: &str) -> Option<&str> {
        self.get(&format!("{}{name}", crate::VARIABLE_PREFIX))
            .map(|s| s.as_str())
    }
}

// No blanket `HeaderLookup` / `SipHeaderLookup` on `indexmap::IndexMap<String,
// String>` — both traits are external, so the orphan rules forbid the pair.
// Wrap in [`EslHeaders`](crate::EslHeaders) instead, which is what callers
// actually want: ARRAY-aware parsing, `variable_` prefix handling, and
// bracket stripping are only correct for ESL-sourced headers anyway.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::variables::{
        ChannelVariable, LoopbackChannelName, LoopbackHangupCause, LoopbackLeg,
    };
    use std::collections::HashMap;

    struct TestStore(HashMap<String, String>);

    impl SipHeaderLookup for TestStore {
        fn sip_header_str(&self, name: &str) -> Option<&str> {
            self.0
                .get(name)
                .map(|s| s.as_str())
        }
    }

    impl HeaderLookup for TestStore {
        fn header_str(&self, name: &str) -> Option<&str> {
            self.0
                .get(name)
                .map(|s| s.as_str())
        }
        fn variable_str(&self, name: &str) -> Option<&str> {
            self.0
                .get(&format!("{}{name}", crate::VARIABLE_PREFIX))
                .map(|s| s.as_str())
        }
    }

    fn store_with(pairs: &[(&str, &str)]) -> TestStore {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        TestStore(map)
    }

    #[test]
    fn header_str_direct() {
        let s = store_with(&[("Unique-ID", "abc-123")]);
        assert_eq!(s.header_str("Unique-ID"), Some("abc-123"));
        assert_eq!(s.header_str("Missing"), None);
    }

    #[test]
    fn header_by_enum() {
        let s = store_with(&[("Unique-ID", "abc-123")]);
        assert_eq!(s.header(EventHeader::UniqueId), Some("abc-123"));
    }

    #[test]
    fn variable_str_direct() {
        let s = store_with(&[("variable_read_codec", "PCMU")]);
        assert_eq!(s.variable_str("read_codec"), Some("PCMU"));
        assert_eq!(s.variable_str("missing"), None);
    }

    #[test]
    fn variable_by_enum() {
        let s = store_with(&[("variable_read_codec", "PCMU")]);
        assert_eq!(s.variable(ChannelVariable::ReadCodec), Some("PCMU"));
    }

    #[test]
    fn unique_id_primary() {
        let s = store_with(&[("Unique-ID", "uuid-1")]);
        assert_eq!(s.unique_id(), Some("uuid-1"));
    }

    #[test]
    fn unique_id_fallback() {
        let s = store_with(&[("Caller-Unique-ID", "uuid-2")]);
        assert_eq!(s.unique_id(), Some("uuid-2"));
    }

    #[test]
    fn unique_id_none() {
        let s = store_with(&[]);
        assert_eq!(s.unique_id(), None);
    }

    #[test]
    fn job_uuid() {
        let s = store_with(&[("Job-UUID", "job-1")]);
        assert_eq!(s.job_uuid(), Some("job-1"));
    }

    #[test]
    fn channel_name() {
        let s = store_with(&[("Channel-Name", "sofia/internal/1000@example.com")]);
        assert_eq!(s.channel_name(), Some("sofia/internal/1000@example.com"));
    }

    #[test]
    fn caller_id_number_and_name() {
        let s = store_with(&[
            ("Caller-Caller-ID-Number", "1000"),
            ("Caller-Caller-ID-Name", "Alice"),
        ]);
        assert_eq!(s.caller_id_number(), Some("1000"));
        assert_eq!(s.caller_id_name(), Some("Alice"));
    }

    #[test]
    fn hangup_cause_typed() {
        let s = store_with(&[("Hangup-Cause", "NORMAL_CLEARING")]);
        assert_eq!(
            s.hangup_cause()
                .unwrap(),
            Some(crate::channel::HangupCause::NormalClearing)
        );
    }

    #[test]
    fn hangup_cause_invalid_is_error() {
        let s = store_with(&[("Hangup-Cause", "BOGUS_CAUSE")]);
        assert!(s
            .hangup_cause()
            .is_err());
    }

    #[test]
    fn destination_number() {
        let s = store_with(&[("Caller-Destination-Number", "1000")]);
        assert_eq!(s.destination_number(), Some("1000"));
    }

    #[test]
    fn callee_id() {
        let s = store_with(&[
            ("Caller-Callee-ID-Number", "2000"),
            ("Caller-Callee-ID-Name", "Bob"),
        ]);
        assert_eq!(s.callee_id_number(), Some("2000"));
        assert_eq!(s.callee_id_name(), Some("Bob"));
    }

    #[test]
    fn event_subclass() {
        let s = store_with(&[("Event-Subclass", "sofia::register")]);
        assert_eq!(s.event_subclass(), Some("sofia::register"));
    }

    #[test]
    fn sofia_event_subclass_typed() {
        let s = store_with(&[("Event-Subclass", "sofia::gateway_state")]);
        assert_eq!(
            s.sofia_event_subclass()
                .unwrap(),
            Some(crate::sofia::SofiaEventSubclass::GatewayState)
        );
    }

    #[test]
    fn sofia_event_subclass_absent() {
        let s = store_with(&[]);
        assert_eq!(
            s.sofia_event_subclass()
                .unwrap(),
            None
        );
    }

    #[test]
    fn sofia_event_subclass_non_sofia_is_error() {
        let s = store_with(&[("Event-Subclass", "conference::maintenance")]);
        assert!(s
            .sofia_event_subclass()
            .is_err());
    }

    #[test]
    fn gateway_reg_state_typed() {
        let s = store_with(&[("State", "REGED")]);
        assert_eq!(
            s.gateway_reg_state()
                .unwrap(),
            Some(crate::sofia::GatewayRegState::Reged)
        );
    }

    #[test]
    fn gateway_reg_state_invalid_is_error() {
        let s = store_with(&[("State", "BOGUS")]);
        assert!(s
            .gateway_reg_state()
            .is_err());
    }

    #[test]
    fn gateway_ping_status_typed() {
        let s = store_with(&[("Ping-Status", "UP")]);
        assert_eq!(
            s.gateway_ping_status()
                .unwrap(),
            Some(crate::sofia::GatewayPingStatus::Up)
        );
    }

    #[test]
    fn sip_user_ping_status_typed() {
        let s = store_with(&[("Ping-Status", "REACHABLE")]);
        assert_eq!(
            s.sip_user_ping_status()
                .unwrap(),
            Some(crate::sofia::SipUserPingStatus::Reachable)
        );
    }

    #[test]
    fn gateway_accessor() {
        let s = store_with(&[("Gateway", "my-gateway")]);
        assert_eq!(s.gateway(), Some("my-gateway"));
    }

    #[test]
    fn profile_name_accessor() {
        let s = store_with(&[("profile-name", "internal")]);
        assert_eq!(s.profile_name(), Some("internal"));
    }

    #[test]
    fn phrase_accessor() {
        let s = store_with(&[("Phrase", "OK")]);
        assert_eq!(s.phrase(), Some("OK"));
    }

    #[test]
    fn channel_state_typed() {
        let s = store_with(&[("Channel-State", "CS_EXECUTE")]);
        assert_eq!(
            s.channel_state()
                .unwrap(),
            Some(ChannelState::CsExecute)
        );
    }

    #[test]
    fn channel_state_number_typed() {
        let s = store_with(&[("Channel-State-Number", "4")]);
        assert_eq!(
            s.channel_state_number()
                .unwrap(),
            Some(ChannelState::CsExecute)
        );
    }

    #[test]
    fn call_state_typed() {
        let s = store_with(&[("Channel-Call-State", "ACTIVE")]);
        assert_eq!(
            s.call_state()
                .unwrap(),
            Some(CallState::Active)
        );
    }

    #[test]
    fn answer_state_typed() {
        let s = store_with(&[("Answer-State", "answered")]);
        assert_eq!(
            s.answer_state()
                .unwrap(),
            Some(AnswerState::Answered)
        );
    }

    #[test]
    fn call_direction_typed() {
        let s = store_with(&[("Call-Direction", "inbound")]);
        assert_eq!(
            s.call_direction()
                .unwrap(),
            Some(CallDirection::Inbound)
        );
    }

    #[test]
    fn priority_typed() {
        let s = store_with(&[("priority", "HIGH")]);
        assert_eq!(
            s.priority()
                .unwrap(),
            Some(EslEventPriority::High)
        );
    }

    #[test]
    fn timetable_extraction() {
        let s = store_with(&[
            ("Caller-Channel-Created-Time", "1700000001000000"),
            ("Caller-Channel-Answered-Time", "1700000005000000"),
        ]);
        let tt = s
            .caller_timetable()
            .unwrap()
            .expect("should have timetable");
        assert_eq!(tt.created, Some(1700000001000000));
        assert_eq!(tt.answered, Some(1700000005000000));
        assert_eq!(tt.hungup, None);
    }

    #[test]
    fn timetable_other_leg() {
        let s = store_with(&[("Other-Leg-Channel-Created-Time", "1700000001000000")]);
        let tt = s
            .other_leg_timetable()
            .unwrap()
            .expect("should have timetable");
        assert_eq!(tt.created, Some(1700000001000000));
    }

    #[test]
    fn timetable_none_when_absent() {
        let s = store_with(&[]);
        assert_eq!(
            s.caller_timetable()
                .unwrap(),
            None
        );
    }

    #[test]
    fn timetable_invalid_is_error() {
        let s = store_with(&[("Caller-Channel-Created-Time", "not_a_number")]);
        let err = s
            .caller_timetable()
            .unwrap_err();
        assert_eq!(err.header, "Caller-Channel-Created-Time");
    }

    // --- loopback resignation ---

    #[test]
    fn loopback_resignation_absent_on_a_real_teardown() {
        let s = store_with(&[("variable_hangup_cause", "NORMAL_CLEARING")]);
        assert!(s
            .loopback_resignation()
            .is_none());
    }

    #[test]
    fn loopback_resignation_on_execute_masquerade_path() {
        let s = store_with(&[
            ("variable_loopback_hangup_cause", "bowout"),
            ("variable_loopback_bowout_other_uuid", "survivor-uuid"),
        ]);
        let r = s
            .loopback_resignation()
            .expect("marker present");
        assert_eq!(r.cause(), Ok(LoopbackHangupCause::Bowout));
        assert_eq!(r.cause_raw(), "bowout");
        assert_eq!(r.other_uuid(), Some("survivor-uuid"));
    }

    #[test]
    fn loopback_resignation_on_frame_count_path() {
        let s = store_with(&[
            ("variable_loopback_hangup_cause", "bridge"),
            ("variable_loopback_bowout_other_uuid", "survivor-uuid"),
        ]);
        let r = s
            .loopback_resignation()
            .expect("marker present");
        assert_eq!(r.cause(), Ok(LoopbackHangupCause::Bridge));
        assert_eq!(r.other_uuid(), Some("survivor-uuid"));
    }

    // The whole point of the shape: a path mod_loopback grows later still
    // reports a resignation, so the surviving leg is never mis-disposed.
    #[test]
    fn loopback_resignation_survives_an_unknown_token() {
        let s = store_with(&[
            ("variable_loopback_hangup_cause", "some_future_path"),
            ("variable_loopback_bowout_other_uuid", "survivor-uuid"),
        ]);
        let r = s
            .loopback_resignation()
            .expect("presence is the signal, not the value");
        assert_eq!(r.other_uuid(), Some("survivor-uuid"));
        assert_eq!(r.cause_raw(), "some_future_path");
        assert!(r
            .cause()
            .is_err());
    }

    #[test]
    fn loopback_resignation_without_a_surviving_uuid() {
        let s = store_with(&[("variable_loopback_hangup_cause", "bridge")]);
        let r = s
            .loopback_resignation()
            .expect("marker present");
        assert_eq!(r.other_uuid(), None);
    }

    // --- channel kind ---

    #[test]
    fn the_resigning_leg_names_itself() {
        let leg = store_with(&[
            ("Channel-Name", "loopback/9199-a"),
            ("variable_loopback_hangup_cause", "bowout"),
            ("variable_loopback_leg", "A"),
        ]);
        assert_eq!(leg.channel_driver(), Some("loopback"));
        let name = leg
            .channel_name()
            .and_then(LoopbackChannelName::parse)
            .expect("a loopback name");
        assert_eq!(name.extension(), "9199");
        assert_eq!(name.leg(), LoopbackLeg::A);
        assert_eq!(
            leg.loopback_leg()
                .unwrap(),
            Some(LoopbackLeg::A)
        );
    }

    // The execute-time masquerade copies every variable and clones the caller
    // profile onto the survivor, so only its own Channel-Name still answers
    // which driver it belongs to.
    #[test]
    fn a_survivor_that_inherited_the_marker_is_not_a_loopback() {
        let survivor = store_with(&[
            ("Channel-Name", "sofia/internal/1000@example.com"),
            ("Caller-Channel-Name", "loopback/9199-a"),
            ("variable_loopback_hangup_cause", "bowout"),
            ("variable_is_loopback", "true"),
            ("variable_loopback_leg", "A"),
        ]);
        assert!(survivor
            .loopback_resignation()
            .is_some());
        assert_eq!(
            survivor
                .loopback_leg()
                .unwrap(),
            Some(LoopbackLeg::A)
        );

        assert_eq!(survivor.channel_driver(), Some("sofia"));
        assert_eq!(
            survivor
                .channel_name()
                .and_then(LoopbackChannelName::parse),
            None
        );
    }

    #[test]
    fn channel_kind_accessors_absent() {
        let s = store_with(&[]);
        assert_eq!(s.channel_driver(), None);
        assert_eq!(
            s.loopback_leg()
                .unwrap(),
            None
        );
    }

    #[test]
    fn loopback_leg_invalid_is_error() {
        let s = store_with(&[("variable_loopback_leg", "a")]);
        assert!(s
            .loopback_leg()
            .is_err());
    }

    #[test]
    fn missing_headers_return_none() {
        let s = store_with(&[]);
        assert_eq!(
            s.channel_state()
                .unwrap(),
            None
        );
        assert_eq!(
            s.channel_state_number()
                .unwrap(),
            None
        );
        assert_eq!(
            s.call_state()
                .unwrap(),
            None
        );
        assert_eq!(
            s.answer_state()
                .unwrap(),
            None
        );
        assert_eq!(
            s.call_direction()
                .unwrap(),
            None
        );
        assert_eq!(
            s.priority()
                .unwrap(),
            None
        );
        assert_eq!(
            s.hangup_cause()
                .unwrap(),
            None
        );
        assert_eq!(s.channel_name(), None);
        assert_eq!(s.caller_id_number(), None);
        assert_eq!(s.caller_id_name(), None);
        assert_eq!(s.destination_number(), None);
        assert_eq!(s.callee_id_number(), None);
        assert_eq!(s.callee_id_name(), None);
        assert_eq!(s.event_subclass(), None);
        assert_eq!(s.job_uuid(), None);
        assert_eq!(s.pl_data(), None);
        assert_eq!(s.sip_event(), None);
        assert_eq!(s.gateway_name(), None);
        assert_eq!(s.channel_presence_id(), None);
        assert_eq!(
            s.presence_call_direction()
                .unwrap(),
            None
        );
        assert_eq!(s.event_date_timestamp(), None);
        assert_eq!(s.event_sequence(), None);
        assert_eq!(s.dtmf_duration(), None);
        assert_eq!(s.dtmf_source(), None);
    }

    #[test]
    fn notify_in_headers() {
        let s = store_with(&[
            ("pl_data", r#"{"invite":"INVITE ..."}"#),
            ("event", "emergency-AbandonedCall"),
            ("gateway_name", "ng911-bcf"),
        ]);
        assert_eq!(s.pl_data(), Some(r#"{"invite":"INVITE ..."}"#));
        assert_eq!(s.sip_event(), Some("emergency-AbandonedCall"));
        assert_eq!(s.gateway_name(), Some("ng911-bcf"));
    }

    #[test]
    fn channel_presence_id() {
        let s = store_with(&[("Channel-Presence-ID", "1000@example.com")]);
        assert_eq!(s.channel_presence_id(), Some("1000@example.com"));
    }

    #[test]
    fn presence_call_direction_typed() {
        let s = store_with(&[("Presence-Call-Direction", "outbound")]);
        assert_eq!(
            s.presence_call_direction()
                .unwrap(),
            Some(CallDirection::Outbound)
        );
    }

    #[test]
    fn event_date_timestamp() {
        let s = store_with(&[("Event-Date-Timestamp", "1700000001000000")]);
        assert_eq!(s.event_date_timestamp(), Some("1700000001000000"));
    }

    #[test]
    fn event_sequence() {
        let s = store_with(&[("Event-Sequence", "12345")]);
        assert_eq!(s.event_sequence(), Some("12345"));
    }

    #[test]
    fn dtmf_duration() {
        let s = store_with(&[("DTMF-Duration", "2000")]);
        assert_eq!(s.dtmf_duration(), Some("2000"));
    }

    #[test]
    fn dtmf_source() {
        let s = store_with(&[("DTMF-Source", "rtp")]);
        assert_eq!(s.dtmf_source(), Some("rtp"));
    }

    #[test]
    fn invalid_values_return_err() {
        let s = store_with(&[
            ("Channel-State", "BOGUS"),
            ("Channel-State-Number", "999"),
            ("Channel-Call-State", "BOGUS"),
            ("Answer-State", "bogus"),
            ("Call-Direction", "bogus"),
            ("Presence-Call-Direction", "bogus"),
            ("priority", "BOGUS"),
            ("Hangup-Cause", "BOGUS"),
        ]);
        assert!(s
            .channel_state()
            .is_err());
        assert!(s
            .channel_state_number()
            .is_err());
        assert!(s
            .call_state()
            .is_err());
        assert!(s
            .answer_state()
            .is_err());
        assert!(s
            .call_direction()
            .is_err());
        assert!(s
            .presence_call_direction()
            .is_err());
        assert!(s
            .priority()
            .is_err());
        assert!(s
            .hangup_cause()
            .is_err());
    }
}
