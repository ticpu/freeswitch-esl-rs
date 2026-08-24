//! Typed event header names for FreeSWITCH ESL events.

sip_header::define_header_enum! {
    tests_mod: event_header_generated_tests,
    error_type: ParseEventHeaderError => "unknown event header",
    /// Top-level header names that appear in FreeSWITCH ESL events.
    ///
    /// These are the headers on the parsed event itself (not protocol framing
    /// headers like `Content-Type`). Use with [`EslEvent::header()`](crate::EslEvent::header) for
    /// type-safe lookups.
    pub enum EventHeader {
        EventName => "Event-Name",
        EventSubclass => "Event-Subclass",
        UniqueId => "Unique-ID",
        CallerUniqueId => "Caller-Unique-ID",
        OtherLegUniqueId => "Other-Leg-Unique-ID",
        ChannelCallUuid => "Channel-Call-UUID",
        JobUuid => "Job-UUID",
        ChannelName => "Channel-Name",
        ChannelState => "Channel-State",
        ChannelStateNumber => "Channel-State-Number",
        ChannelCallState => "Channel-Call-State",
        AnswerState => "Answer-State",
        CallDirection => "Call-Direction",
        HangupCause => "Hangup-Cause",
        CallerCallerIdName => "Caller-Caller-ID-Name",
        CallerCallerIdNumber => "Caller-Caller-ID-Number",
        CallerOrigCallerIdName => "Caller-Orig-Caller-ID-Name",
        CallerOrigCallerIdNumber => "Caller-Orig-Caller-ID-Number",
        CallerCalleeIdName => "Caller-Callee-ID-Name",
        CallerCalleeIdNumber => "Caller-Callee-ID-Number",
        CallerDestinationNumber => "Caller-Destination-Number",
        CallerContext => "Caller-Context",
        CallerDirection => "Caller-Direction",
        CallerNetworkAddr => "Caller-Network-Addr",
        CoreUuid => "Core-UUID",
        DtmfDigit => "DTMF-Digit",
        Priority => "priority",
        LogLevel => "Log-Level",
        /// SIP NOTIFY body content (JSON payload from `NOTIFY_IN` events).
        PlData => "pl_data",
        /// SIP event package name from `NOTIFY_IN` events (e.g. `emergency-AbandonedCall`).
        SipEvent => "event",
        /// SIP content type from `NOTIFY_IN` events.
        SipContentType => "sip_content_type",
        /// Gateway that received the SIP NOTIFY.
        GatewayName => "gateway_name",

        // --- Codec (from switch_channel_event_set_data / switch_core_codec.c) ---
        // Audio read
        ChannelReadCodecName => "Channel-Read-Codec-Name",
        ChannelReadCodecRate => "Channel-Read-Codec-Rate",
        ChannelReadCodecBitRate => "Channel-Read-Codec-Bit-Rate",
        /// Only present when actual_samples_per_second != samples_per_second.
        ChannelReportedReadCodecRate => "Channel-Reported-Read-Codec-Rate",
        // Audio write
        ChannelWriteCodecName => "Channel-Write-Codec-Name",
        ChannelWriteCodecRate => "Channel-Write-Codec-Rate",
        ChannelWriteCodecBitRate => "Channel-Write-Codec-Bit-Rate",
        /// Only present when actual_samples_per_second != samples_per_second.
        ChannelReportedWriteCodecRate => "Channel-Reported-Write-Codec-Rate",
        // Video read/write
        ChannelVideoReadCodecName => "Channel-Video-Read-Codec-Name",
        ChannelVideoReadCodecRate => "Channel-Video-Read-Codec-Rate",
        ChannelVideoWriteCodecName => "Channel-Video-Write-Codec-Name",
        ChannelVideoWriteCodecRate => "Channel-Video-Write-Codec-Rate",
        /// Active session count from `HEARTBEAT` events.
        SessionCount => "Session-Count",
        FreeswitchHostname => "FreeSWITCH-Hostname",
        FreeswitchSwitchname => "FreeSWITCH-Switchname",
        FreeswitchIpv4 => "FreeSWITCH-IPv4",
        FreeswitchIpv6 => "FreeSWITCH-IPv6",
        FreeswitchVersion => "FreeSWITCH-Version",
        FreeswitchDomain => "FreeSWITCH-Domain",
        FreeswitchUser => "FreeSWITCH-User",

        // --- Application (from switch_core_session.c) ---
        Application => "Application",
        ApplicationData => "Application-Data",
        ApplicationResponse => "Application-Response",
        ApplicationUuid => "Application-UUID",

        // --- Event metadata (from switch_event_prep_for_delivery_detailed) ---
        EventDateLocal => "Event-Date-Local",
        EventDateGmt => "Event-Date-GMT",
        EventDateTimestamp => "Event-Date-Timestamp",
        EventCallingFile => "Event-Calling-File",
        EventCallingFunction => "Event-Calling-Function",
        EventCallingLineNumber => "Event-Calling-Line-Number",
        EventSequence => "Event-Sequence",

        // --- Channel basic data (from switch_channel_event_set_basic_data) ---
        ChannelPresenceId => "Channel-Presence-ID",
        ChannelPresenceData => "Channel-Presence-Data",
        PresenceDataCols => "Presence-Data-Cols",
        PresenceCallDirection => "Presence-Call-Direction",
        ChannelHitDialplan => "Channel-HIT-Dialplan",
        SessionExternalId => "Session-External-ID",
        /// `originator` or `originatee` on bridged channel events.
        OtherType => "Other-Type",

        // --- Callstate change (from switch_channel_perform_set_callstate) ---
        ChannelCallStateNumber => "Channel-Call-State-Number",
        OriginalChannelCallState => "Original-Channel-Call-State",

        // --- DTMF (from switch_channel_dequeue_dtmf) ---
        DtmfDuration => "DTMF-Duration",
        DtmfSource => "DTMF-Source",

        // --- Caller profile (from switch_caller_profile_event_set_data, "Caller-" prefix) ---
        CallerLogicalDirection => "Caller-Logical-Direction",
        CallerUsername => "Caller-Username",
        CallerDialplan => "Caller-Dialplan",
        CallerAni => "Caller-ANI",
        CallerAniii => "Caller-ANI-II",
        CallerSource => "Caller-Source",
        CallerTransferSource => "Caller-Transfer-Source",
        CallerRdnis => "Caller-RDNIS",
        CallerChannelName => "Caller-Channel-Name",
        CallerProfileIndex => "Caller-Profile-Index",
        CallerScreenBit => "Caller-Screen-Bit",
        CallerPrivacyHideName => "Caller-Privacy-Hide-Name",
        CallerPrivacyHideNumber => "Caller-Privacy-Hide-Number",

        // --- Other-leg profile (from switch_caller_profile_event_set_data, "Other-Leg" prefix) ---
        OtherLegDirection => "Other-Leg-Direction",
        OtherLegLogicalDirection => "Other-Leg-Logical-Direction",
        OtherLegUsername => "Other-Leg-Username",
        OtherLegDialplan => "Other-Leg-Dialplan",
        OtherLegCallerIdName => "Other-Leg-Caller-ID-Name",
        OtherLegCallerIdNumber => "Other-Leg-Caller-ID-Number",
        OtherLegOrigCallerIdName => "Other-Leg-Orig-Caller-ID-Name",
        OtherLegOrigCallerIdNumber => "Other-Leg-Orig-Caller-ID-Number",
        OtherLegCalleeIdName => "Other-Leg-Callee-ID-Name",
        OtherLegCalleeIdNumber => "Other-Leg-Callee-ID-Number",
        OtherLegNetworkAddr => "Other-Leg-Network-Addr",
        OtherLegAni => "Other-Leg-ANI",
        OtherLegAniii => "Other-Leg-ANI-II",
        OtherLegDestinationNumber => "Other-Leg-Destination-Number",
        OtherLegSource => "Other-Leg-Source",
        OtherLegTransferSource => "Other-Leg-Transfer-Source",
        OtherLegContext => "Other-Leg-Context",
        OtherLegRdnis => "Other-Leg-RDNIS",
        OtherLegChannelName => "Other-Leg-Channel-Name",
        OtherLegProfileIndex => "Other-Leg-Profile-Index",
        OtherLegScreenBit => "Other-Leg-Screen-Bit",
        OtherLegPrivacyHideName => "Other-Leg-Privacy-Hide-Name",
        OtherLegPrivacyHideNumber => "Other-Leg-Privacy-Hide-Number",

        // --- Heartbeat (from send_heartbeat in switch_core.c) ---
        /// Seconds since FreeSWITCH startup.
        UpTime => "Up-Time",
        /// Milliseconds since FreeSWITCH startup.
        UptimeMsec => "Uptime-msec",
        MaxSessions => "Max-Sessions",
        SessionPeakMax => "Session-Peak-Max",
        SessionPeakFiveMin => "Session-Peak-FiveMin",
        SessionPerSec => "Session-Per-Sec",
        SessionPerSecFiveMin => "Session-Per-Sec-FiveMin",
        SessionPerSecMax => "Session-Per-Sec-Max",
        SessionPerSecLast => "Session-Per-Sec-Last",
        SessionSinceStartup => "Session-Since-Startup",
        IdleCpu => "Idle-CPU",
        HeartbeatInterval => "Heartbeat-Interval",
        EventInfo => "Event-Info",

        // --- Log (from switch_log_meta_vprintf in switch_log.c) ---
        LogData => "Log-Data",
        LogFile => "Log-File",
        LogFunction => "Log-Function",
        LogLine => "Log-Line",
        UserData => "User-Data",

        // --- Application (from switch_core_session_exec in switch_core_session.c) ---
        ApplicationUuidName => "Application-UUID-Name",

        // --- Sofia event headers (from mod_sofia CUSTOM events) ---
        Gateway => "Gateway",
        State => "State",
        PingStatus => "Ping-Status",
        Phrase => "Phrase",
        ProfileName => "profile-name",
        /// SIP response code (integer) from gateway_state and sip_user_state events.
        Status => "Status",
    }
}

/// Return a lowercase case-alias for `key` when case-insensitive aliasing applies.
///
/// The aliasing rule: keys containing `_` (channel variables such as
/// `variable_*`, and SIP passthrough headers such as `sip_h_*` / `sip_i_*`)
/// are excluded from case-insensitive aliasing because their suffix encodes
/// original SIP wire casing that must be preserved verbatim. All other keys
/// (dash-separated event and framing headers) may be looked up case-insensitively
/// via their lowercase form.
///
/// Returns `Some(key.to_ascii_lowercase())` when aliasing applies, `None` for
/// underscore-containing keys.
pub fn case_alias_key(key: &str) -> Option<String> {
    if key.contains('_') {
        None
    } else {
        Some(key.to_ascii_lowercase())
    }
}

/// Normalize a header key to its canonical form for case-insensitive storage.
///
/// FreeSWITCH's C ESL uses case-insensitive header lookups (`strcasecmp`), but
/// stores header names verbatim. Multiple C code paths emit the same logical
/// header with different casing (e.g. `switch_channel.c` sends `Unique-ID`
/// while `switch_event.c` sends `unique-id`). This function normalizes keys
/// so that both resolve to the same `HashMap` entry.
///
/// **Strategy:**
/// 1. Known [`EventHeader`] variants are matched first (case-insensitive) and
///    returned in their canonical wire form (e.g. `unique-id` → `Unique-ID`).
/// 2. Keys containing underscores pass through verbatim (see [`case_alias_key`]).
/// 3. Unknown dash-separated keys are Title-Cased to match FreeSWITCH's
///    dominant convention for event and framing headers.
pub fn normalize_header_key(raw: &str) -> String {
    if let Ok(eh) = raw.parse::<EventHeader>() {
        return eh
            .as_str()
            .to_string();
    }
    if case_alias_key(raw).is_none() {
        raw.to_string()
    } else {
        title_case_dashes(raw)
    }
}

fn title_case_dashes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut capitalize_next = true;
    for c in s.chars() {
        if c == '-' {
            result.push('-');
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c.to_ascii_lowercase());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_round_trip() {
        assert_eq!(EventHeader::UniqueId.to_string(), "Unique-ID");
        assert_eq!(
            EventHeader::ChannelCallState.to_string(),
            "Channel-Call-State"
        );
        assert_eq!(
            EventHeader::CallerCallerIdName.to_string(),
            "Caller-Caller-ID-Name"
        );
        assert_eq!(EventHeader::Priority.to_string(), "priority");
    }

    #[test]
    fn as_ref_str() {
        let h: &str = EventHeader::UniqueId.as_ref();
        assert_eq!(h, "Unique-ID");
    }

    #[test]
    fn from_str_case_insensitive() {
        assert_eq!(
            "unique-id".parse::<EventHeader>(),
            Ok(EventHeader::UniqueId)
        );
        assert_eq!(
            "UNIQUE-ID".parse::<EventHeader>(),
            Ok(EventHeader::UniqueId)
        );
        assert_eq!(
            "Unique-ID".parse::<EventHeader>(),
            Ok(EventHeader::UniqueId)
        );
        assert_eq!(
            "channel-call-state".parse::<EventHeader>(),
            Ok(EventHeader::ChannelCallState)
        );
    }

    #[test]
    fn from_str_unknown() {
        let err = "X-Custom-Not-In-Enum".parse::<EventHeader>();
        assert!(err.is_err());
        assert_eq!(
            err.unwrap_err()
                .to_string(),
            "unknown event header: X-Custom-Not-In-Enum"
        );
    }

    // --- normalize_header_key tests ---
    // FreeSWITCH C ESL uses strcasecmp for header lookups but stores names
    // verbatim. Multiple C code paths emit the same logical header with
    // different casing (switch_channel.c Title-Case vs switch_event.c lowercase
    // vs switch_core_codec.c mixed). normalize_header_key canonicalizes keys
    // so they collapse to a single HashMap entry.

    #[test]
    fn normalize_known_enum_variants_return_canonical_form() {
        // EventHeader::from_str is case-insensitive; canonical as_str() is returned
        assert_eq!(normalize_header_key("unique-id"), "Unique-ID");
        assert_eq!(normalize_header_key("UNIQUE-ID"), "Unique-ID");
        assert_eq!(normalize_header_key("Unique-ID"), "Unique-ID");
        assert_eq!(normalize_header_key("dtmf-digit"), "DTMF-Digit");
        assert_eq!(normalize_header_key("DTMF-DIGIT"), "DTMF-Digit");
        assert_eq!(
            normalize_header_key("channel-call-uuid"),
            "Channel-Call-UUID"
        );
        assert_eq!(normalize_header_key("event-name"), "Event-Name");
    }

    #[test]
    fn normalize_known_underscore_variants_return_canonical_form() {
        // Headers whose canonical form contains underscores
        assert_eq!(normalize_header_key("priority"), "priority");
        assert_eq!(normalize_header_key("PRIORITY"), "priority");
        assert_eq!(normalize_header_key("pl_data"), "pl_data");
        assert_eq!(normalize_header_key("PL_DATA"), "pl_data");
        assert_eq!(normalize_header_key("sip_content_type"), "sip_content_type");
        assert_eq!(normalize_header_key("gateway_name"), "gateway_name");
        assert_eq!(normalize_header_key("event"), "event");
        assert_eq!(normalize_header_key("EVENT"), "event");
    }

    #[test]
    fn normalize_codec_headers_from_switch_core_codec() {
        // switch_core_codec.c sends lowercase, switch_channel_event_set_data sends Title-Case
        // Both must normalize to the canonical EventHeader form
        assert_eq!(
            normalize_header_key("channel-read-codec-bit-rate"),
            "Channel-Read-Codec-Bit-Rate"
        );
        assert_eq!(
            normalize_header_key("Channel-Read-Codec-Bit-Rate"),
            "Channel-Read-Codec-Bit-Rate"
        );
        // switch_core_codec.c mixed case for write: "Channel-Write-codec-bit-rate"
        assert_eq!(
            normalize_header_key("Channel-Write-codec-bit-rate"),
            "Channel-Write-Codec-Bit-Rate"
        );
        assert_eq!(
            normalize_header_key("channel-video-read-codec-name"),
            "Channel-Video-Read-Codec-Name"
        );
    }

    #[test]
    fn normalize_unknown_underscore_keys_passthrough() {
        // Channel variables and sip_h_* passthrough preserve original casing
        assert_eq!(
            normalize_header_key("variable_sip_call_id"),
            "variable_sip_call_id"
        );
        assert_eq!(
            normalize_header_key("variable_sip_h_X-My-CUSTOM-Header"),
            "variable_sip_h_X-My-CUSTOM-Header"
        );
        assert_eq!(
            normalize_header_key("variable_sip_h_Diversion"),
            "variable_sip_h_Diversion"
        );
    }

    #[test]
    fn normalize_unknown_dash_keys_title_case() {
        // Framing and unknown event headers get Title-Cased
        assert_eq!(normalize_header_key("content-type"), "Content-Type");
        assert_eq!(normalize_header_key("Content-Type"), "Content-Type");
        assert_eq!(normalize_header_key("CONTENT-TYPE"), "Content-Type");
        assert_eq!(normalize_header_key("x-custom-header"), "X-Custom-Header");
        assert_eq!(
            normalize_header_key("Content-Disposition"),
            "Content-Disposition"
        );
        assert_eq!(normalize_header_key("reply-text"), "Reply-Text");
    }

    #[test]
    fn normalize_idempotent_for_all_enum_variants() {
        // Normalizing an already-canonical wire string must return it unchanged
        let variants = [
            EventHeader::EventName,
            EventHeader::UniqueId,
            EventHeader::ChannelCallUuid,
            EventHeader::DtmfDigit,
            EventHeader::Priority,
            EventHeader::PlData,
            EventHeader::SipEvent,
            EventHeader::GatewayName,
            EventHeader::SipContentType,
            EventHeader::ChannelReadCodecBitRate,
            EventHeader::ChannelVideoWriteCodecRate,
            EventHeader::LogLevel,
        ];
        for v in variants {
            let canonical = v.as_str();
            assert_eq!(
                normalize_header_key(canonical),
                canonical,
                "normalization not idempotent for {canonical}"
            );
        }
    }

    #[test]
    fn parse_missing_event_default_headers() {
        // From switch_event_prep_for_delivery_detailed (switch_event.c)
        assert!("Event-Date-Local"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Event-Date-GMT"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Event-Date-Timestamp"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Event-Calling-File"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Event-Calling-Function"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Event-Calling-Line-Number"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Event-Sequence"
            .parse::<EventHeader>()
            .is_ok());
    }

    #[test]
    fn parse_missing_channel_basic_data_headers() {
        // From switch_channel_event_set_basic_data (switch_channel.c)
        assert!("Channel-Presence-ID"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Channel-Presence-Data"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Presence-Data-Cols"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Presence-Call-Direction"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Channel-HIT-Dialplan"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Session-External-ID"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Type"
            .parse::<EventHeader>()
            .is_ok());
    }

    #[test]
    fn parse_missing_callstate_and_dtmf_headers() {
        // From switch_channel_perform_set_callstate (switch_channel.c)
        assert!("Channel-Call-State-Number"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Original-Channel-Call-State"
            .parse::<EventHeader>()
            .is_ok());
        // From switch_channel_dequeue_dtmf (switch_channel.c)
        assert!("DTMF-Duration"
            .parse::<EventHeader>()
            .is_ok());
        assert!("DTMF-Source"
            .parse::<EventHeader>()
            .is_ok());
    }

    #[test]
    fn parse_missing_caller_profile_headers() {
        // From switch_caller_profile_event_set_data (switch_caller.c) with "Caller-" prefix
        assert!("Caller-Logical-Direction"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Caller-Username"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Caller-Dialplan"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Caller-ANI"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Caller-ANI-II"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Caller-Source"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Caller-Transfer-Source"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Caller-RDNIS"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Caller-Channel-Name"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Caller-Profile-Index"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Caller-Screen-Bit"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Caller-Privacy-Hide-Name"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Caller-Privacy-Hide-Number"
            .parse::<EventHeader>()
            .is_ok());
    }

    #[test]
    fn parse_heartbeat_headers() {
        assert!("Up-Time"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Uptime-msec"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Max-Sessions"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Session-Peak-Max"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Session-Peak-FiveMin"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Session-Per-Sec"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Session-Per-Sec-FiveMin"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Session-Per-Sec-Max"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Session-Per-Sec-Last"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Session-Since-Startup"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Idle-CPU"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Heartbeat-Interval"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Event-Info"
            .parse::<EventHeader>()
            .is_ok());
    }

    #[test]
    fn parse_log_headers() {
        assert!("Log-Data"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Log-File"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Log-Function"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Log-Line"
            .parse::<EventHeader>()
            .is_ok());
        assert!("User-Data"
            .parse::<EventHeader>()
            .is_ok());
    }

    #[test]
    fn parse_application_uuid_name() {
        assert!("Application-UUID-Name"
            .parse::<EventHeader>()
            .is_ok());
    }

    #[test]
    fn parse_sofia_event_headers() {
        assert_eq!("Gateway".parse::<EventHeader>(), Ok(EventHeader::Gateway));
        assert_eq!("State".parse::<EventHeader>(), Ok(EventHeader::State));
        assert_eq!(
            "Ping-Status".parse::<EventHeader>(),
            Ok(EventHeader::PingStatus)
        );
        assert_eq!("Phrase".parse::<EventHeader>(), Ok(EventHeader::Phrase));
        assert_eq!(
            "profile-name".parse::<EventHeader>(),
            Ok(EventHeader::ProfileName)
        );
    }

    #[test]
    fn parse_sofia_profile_headers_are_spelling_exact() {
        assert_eq!(
            "profile-name".parse::<EventHeader>(),
            Ok(EventHeader::ProfileName)
        );
        assert_eq!(
            "profile_name".parse::<EventHeader>(),
            Ok(EventHeader::ProfileNameUnderscore)
        );
        assert_eq!(
            "module_name".parse::<EventHeader>(),
            Ok(EventHeader::ModuleName)
        );
        assert_eq!(
            "profile_uri".parse::<EventHeader>(),
            Ok(EventHeader::ProfileUri)
        );
    }

    #[test]
    fn normalize_keeps_profile_name_spellings_apart() {
        assert_eq!(normalize_header_key("profile-name"), "profile-name");
        assert_eq!(normalize_header_key("PROFILE-NAME"), "profile-name");
        assert_eq!(normalize_header_key("profile_name"), "profile_name");
    }

    #[test]
    fn parse_missing_other_leg_headers() {
        // From switch_caller_profile_event_set_data with "Other-Leg" prefix
        assert!("Other-Leg-Direction"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Logical-Direction"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Username"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Dialplan"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Caller-ID-Name"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Caller-ID-Number"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Orig-Caller-ID-Name"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Orig-Caller-ID-Number"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Callee-ID-Name"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Callee-ID-Number"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Network-Addr"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-ANI"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-ANI-II"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Destination-Number"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Source"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Transfer-Source"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Context"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-RDNIS"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Channel-Name"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Profile-Index"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Screen-Bit"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Privacy-Hide-Name"
            .parse::<EventHeader>()
            .is_ok());
        assert!("Other-Leg-Privacy-Hide-Number"
            .parse::<EventHeader>()
            .is_ok());
    }

    #[test]
    fn case_alias_key_underscore_excluded() {
        // Underscore keys are excluded from case-insensitive aliasing.
        assert_eq!(case_alias_key("variable_foo"), None);
        assert_eq!(case_alias_key("sip_h_Call-Info"), None);
        assert_eq!(case_alias_key("sip_i_contact"), None);
    }

    #[test]
    fn case_alias_key_dash_keys_lowercased() {
        // Dash-separated keys return their lowercase alias.
        assert_eq!(
            case_alias_key("Content-Type"),
            Some("content-type".to_string())
        );
        assert_eq!(case_alias_key("UNIQUE-ID"), Some("unique-id".to_string()));
        assert_eq!(case_alias_key("reply-text"), Some("reply-text".to_string()));
    }
}
