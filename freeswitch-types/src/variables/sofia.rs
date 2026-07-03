//! Typed mod_sofia / SIP channel variable names.

parse_error! { ParseSofiaVariableError("sofia variable"); }

sip_header::define_header_enum! {
    error_type: ParseSofiaVariableError,
    /// mod_sofia / SIP channel variable names (the part after the `variable_` prefix).
    ///
    /// Use with [`HeaderLookup::variable()`](crate::HeaderLookup::variable) for type-safe lookups.
    /// Core FreeSWITCH variables belong in [`ChannelVariable`](super::ChannelVariable).
    pub enum SofiaVariable {
        // --- SIP From ---
        SipFromUser => "sip_from_user",
        SipFromHost => "sip_from_host",
        SipFromPort => "sip_from_port",
        SipFromUri => "sip_from_uri",
        SipFromDisplay => "sip_from_display",
        SipFromTag => "sip_from_tag",
        SipFromComment => "sip_from_comment",
        SipFromUserStripped => "sip_from_user_stripped",
        SipFullFrom => "sip_full_from",

        // --- SIP To ---
        SipToUser => "sip_to_user",
        SipToHost => "sip_to_host",
        SipToPort => "sip_to_port",
        SipToUri => "sip_to_uri",
        SipToDisplay => "sip_to_display",
        SipToTag => "sip_to_tag",
        SipToComment => "sip_to_comment",
        SipFullTo => "sip_full_to",

        // --- SIP Contact ---
        SipContactUser => "sip_contact_user",
        SipContactHost => "sip_contact_host",
        SipContactPort => "sip_contact_port",
        SipContactUri => "sip_contact_uri",
        SipContactParams => "sip_contact_params",

        // --- SIP Request ---
        SipReqUser => "sip_req_user",
        SipReqHost => "sip_req_host",
        SipReqPort => "sip_req_port",
        SipReqUri => "sip_req_uri",

        // --- SIP Via ---
        SipViaHost => "sip_via_host",
        SipViaPort => "sip_via_port",
        SipViaRport => "sip_via_rport",
        SipViaProtocol => "sip_via_protocol",
        SipFullVia => "sip_full_via",
        SipFullRoute => "sip_full_route",

        // --- SIP Session ---
        SipCallId => "sip_call_id",
        SipCseq => "sip_cseq",
        SipUserAgent => "sip_user_agent",
        SipSubject => "sip_subject",
        SipAllow => "sip_allow",
        SipAcceptLanguage => "sip_accept_language",
        SipCallInfo => "sip_call_info",
        SipDateEpochTime => "sip_date_epoch_time",

        // --- SIP Network ---
        SipReceivedIp => "sip_received_ip",
        SipReceivedPort => "sip_received_port",
        SipNetworkIp => "sip_network_ip",
        SipNetworkPort => "sip_network_port",
        SipNatDetected => "sip_nat_detected",
        SipTransport => "sip_transport",
        SipReplyHost => "sip_reply_host",

        // --- SIP Auth ---
        SipAuthUsername => "sip_auth_username",
        SipAuthPassword => "sip_auth_password",
        SipAuthorized => "sip_authorized",
        SipAclAuthedBy => "sip_acl_authed_by",
        SipAclToken => "sip_acl_token",
        SipChallengeRealm => "sip_challenge_realm",

        // --- SIP Failure / Hangup ---
        SipInviteFailureStatus => "sip_invite_failure_status",
        SipInviteFailurePhrase => "sip_invite_failure_phrase",
        SipHangupDisposition => "sip_hangup_disposition",
        SipTermStatus => "sip_term_status",
        SipTermCause => "sip_term_cause",
        SipReason => "sip_reason",

        // --- SIP Identity / Privacy ---
        SipPAssertedIdentity => "sip_P-Asserted-Identity",
        SipPPreferredIdentity => "sip_P-Preferred-Identity",
        SipPrivacy => "sip_Privacy",
        SipRemotePartyId => "sip_Remote-Party-ID",
        SipStirShakenAttest => "sip_stir_shaken_attest",
        SipVerstat => "sip_verstat",
        SipVerstatDetailed => "sip_verstat_detailed",

        // --- SIP Invite Details ---
        SipInviteCallId => "sip_invite_call_id",
        SipInviteCseq => "sip_invite_cseq",
        SipInviteFullFrom => "sip_invite_full_from",
        SipInviteFullTo => "sip_invite_full_to",
        SipInviteFullVia => "sip_invite_full_via",
        SipInviteFromUri => "sip_invite_from_uri",
        SipInviteToUri => "sip_invite_to_uri",
        SipInviteReqUri => "sip_invite_req_uri",
        SipInviteRecordRoute => "sip_invite_record_route",
        SipInviteRouteUri => "sip_invite_route_uri",
        SipInviteDomain => "sip_invite_domain",
        SipInviteParams => "sip_invite_params",

        // --- SIP Features ---
        SipAutoAnswer => "sip_auto_answer",
        SipAutoSimplify => "sip_auto_simplify",
        SipEnableSoa => "sip_enable_soa",
        SipCopyCustomHeaders => "sip_copy_custom_headers",
        SipCopyMultipart => "sip_copy_multipart",
        SipLoopedCall => "sip_looped_call",

        // --- SIP Redirect / Transfer ---
        SipRedirectedTo => "sip_redirected_to",
        SipRedirectedBy => "sip_redirected_by",
        SipRedirectDialstring => "sip_redirect_dialstring",
        SipReferReply => "sip_refer_reply",
        SipReferStatusCode => "sip_refer_status_code",
        SipReferredByFull => "sip_referred_by_full",
        SipReferredByCid => "sip_referred_by_cid",
        SipReinviteSdp => "sip_reinvite_sdp",

        // --- SIP Gateway ---
        SipGateway => "sip_gateway",
        SipGatewayName => "sip_gateway_name",
        SipUseGateway => "sip_use_gateway",
        SipDestinationUrl => "sip_destination_url",

        // --- Sofia Profile ---
        SipProfileName => "sip_profile_name",
        SofiaProfileName => "sofia_profile_name",
        SofiaProfileUrl => "sofia_profile_url",
        SofiaProfileDomainName => "sofia_profile_domain_name",

        // --- RTP / SRTP (set via mod_sofia / switch_core_media) ---
        RtpSecureMediaConfirmed => "rtp_secure_media_confirmed",
        Rtp2833SendPayload => "rtp_2833_send_payload",
        Rtp2833RecvPayload => "rtp_2833_recv_payload",
        RtpDisableHold => "rtp_disable_hold",
        RtpJitterBufferPlc => "rtp_jitter_buffer_plc",
        RtpVideoMaxBandwidthIn => "rtp_video_max_bandwidth_in",
        RtpVideoMaxBandwidthOut => "rtp_video_max_bandwidth_out",

        // --- SIP Callee / Display ---
        SipCalleeIdName => "sip_callee_id_name",
        SipCalleeIdNumber => "sip_callee_id_number",
        SipCidType => "sip_cid_type",

        // --- SIP RTP Stats ---
        SipRtpRxstat => "sip_rtp_rxstat",
        SipRtpTxstat => "sip_rtp_txstat",
        SipPRtpStat => "sip_p_rtp_stat",

        // --- SIP History ---
        SipHistoryInfo => "sip_history_info",
        SipGeolocation => "sip_geolocation",

        // --- SIP Body ---
        SipMultipart => "sip_multipart",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_round_trip() {
        assert_eq!(SofiaVariable::SipCallId.to_string(), "sip_call_id");
        assert_eq!(
            SofiaVariable::SipFromDisplay.to_string(),
            "sip_from_display"
        );
        assert_eq!(
            SofiaVariable::SofiaProfileName.to_string(),
            "sofia_profile_name"
        );
    }

    #[test]
    fn as_ref_str() {
        let v: &str = SofiaVariable::SipNetworkIp.as_ref();
        assert_eq!(v, "sip_network_ip");
    }

    #[test]
    fn from_str_case_insensitive() {
        assert_eq!(
            "sip_call_id".parse::<SofiaVariable>(),
            Ok(SofiaVariable::SipCallId)
        );
        assert_eq!(
            "SIP_CALL_ID".parse::<SofiaVariable>(),
            Ok(SofiaVariable::SipCallId)
        );
    }

    #[test]
    fn from_str_unknown() {
        let err = "nonexistent_sip_var".parse::<SofiaVariable>();
        assert!(err.is_err());
    }

    #[test]
    fn from_str_round_trip_sample() {
        let variants = [
            SofiaVariable::SipCallId,
            SofiaVariable::SipFromUser,
            SofiaVariable::SipToHost,
            SofiaVariable::SipNetworkIp,
            SofiaVariable::SipHangupDisposition,
            SofiaVariable::SipPAssertedIdentity,
            SofiaVariable::SofiaProfileName,
            SofiaVariable::RtpSecureMediaConfirmed,
            SofiaVariable::SipGatewayName,
            SofiaVariable::SipInviteCallId,
            SofiaVariable::SipMultipart,
        ];
        for v in variants {
            let wire = v.to_string();
            let parsed: SofiaVariable = wire
                .parse()
                .unwrap();
            assert_eq!(parsed, v, "round-trip failed for {wire}");
        }
    }
}
