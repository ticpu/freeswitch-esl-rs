//! Typed mod_loopback channel variable names and the bowout marker.

sip_header::define_header_enum! {
    tests_mod: loopback_variable_generated_tests,
    error_type: ParseLoopbackVariableError => "unknown loopback variable",
    /// mod_loopback channel variable names (the part after the `variable_` prefix).
    ///
    /// Use with [`HeaderLookup::variable()`](crate::HeaderLookup::variable) for
    /// type-safe lookups.
    pub enum LoopbackVariable {
        // --- Set by mod_loopback ---
        IsLoopback => "is_loopback",
        LoopbackLeg => "loopback_leg",
        LoopbackFromUuid => "loopback_from_uuid",
        OtherLoopbackFromUuid => "other_loopback_from_uuid",
        OtherLoopbackLegUuid => "other_loopback_leg_uuid",
        /// Set on the *real* channel bridged to a loopback leg, not on the leg.
        OtherLegTrueId => "other_leg_true_id",
        /// Application named by the `loopback/app=<application>[:<args>]`
        /// destination form, which runs instead of a dialplan lookup.
        LoopbackApp => "loopback_app",
        LoopbackAppArg => "loopback_app_arg",
        /// Presence marks a bowout. See [`LoopbackResignation`].
        LoopbackHangupCause => "loopback_hangup_cause",
        /// The real channel that outlives the bowout.
        LoopbackBowoutOtherUuid => "loopback_bowout_other_uuid",

        // --- Set by the caller ---
        /// Vetoes the bowout when false. Unset means enabled.
        LoopbackBowout => "loopback_bowout",
        /// Bows out as soon as the leg executes an application, instead of
        /// waiting for audio to flow.
        LoopbackBowoutOnExecute => "loopback_bowout_on_execute",
        /// Space-separated variable names to copy onto the B leg.
        LoopbackExport => "loopback_export",
        LoopbackInitialCodec => "loopback_initial_codec",
    }
}

wire_enum! {
    /// Which mod_loopback path resigned, from the `loopback_hangup_cause` value.
    ///
    /// Both variants mean the same thing to a consumer: the loopback leg is
    /// gone and the call continues elsewhere. Branch on this only to log or
    /// to tell the two triggers apart -- never to decide whether a bowout
    /// happened. That is [`LoopbackResignation`]'s job.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum LoopbackHangupCause {
        /// Execute-time masquerade, triggered by `loopback_bowout_on_execute`
        /// or by an application flagged `SAF_NO_LOOPBACK`.
        Bowout => "bowout",
        /// Frame-count path: both legs bridged and answered, so mod_loopback
        /// `uuid_bridge`s the two real channels together.
        Bridge => "bridge",
    }
    error ParseLoopbackHangupCauseError("loopback hangup cause");
    tests: loopback_hangup_cause_generated_tests;
}

/// A loopback leg that resigned from a call which is still up.
///
/// mod_loopback takes itself out of the path once it can connect the two real
/// channels directly, so the leg emits `CHANNEL_HANGUP_COMPLETE` for a call
/// that is alive and connected on [`other_uuid()`](Self::other_uuid). Track
/// that channel instead of treating the hangup as a teardown.
///
/// Obtained from
/// [`HeaderLookup::loopback_resignation()`](crate::HeaderLookup::loopback_resignation),
/// which returns `Some` whenever the marker variable is present regardless of
/// its value -- mod_loopback writes a different token per path, and a third
/// one would still be a resignation. [`cause()`](Self::cause) asks the separate
/// question of which path it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopbackResignation<'a> {
    cause_raw: &'a str,
    other_uuid: Option<&'a str>,
}

impl<'a> LoopbackResignation<'a> {
    pub(crate) fn new(cause_raw: &'a str, other_uuid: Option<&'a str>) -> Self {
        Self {
            cause_raw,
            other_uuid,
        }
    }

    /// The `loopback_hangup_cause` value verbatim.
    pub fn cause_raw(&self) -> &'a str {
        self.cause_raw
    }

    /// Which path resigned.
    ///
    /// `Err` means mod_loopback used a path this crate does not know yet --
    /// worth logging, but the resignation itself still stands.
    pub fn cause(&self) -> Result<LoopbackHangupCause, ParseLoopbackHangupCauseError> {
        self.cause_raw
            .parse()
    }

    /// The real channel that outlives the loopback leg, from
    /// `loopback_bowout_other_uuid`.
    pub fn other_uuid(&self) -> Option<&'a str> {
        self.other_uuid
    }
}

#[cfg(test)]
mod channel_name_tests {
    use super::*;

    #[test]
    fn both_legs_of_a_dialled_extension() {
        let a = LoopbackChannelName::parse("loopback/9199-a").expect("loopback name parses");
        assert_eq!(a.extension(), "9199");
        assert_eq!(a.leg(), LoopbackLeg::A);

        let b = LoopbackChannelName::parse("loopback/9199-b").expect("loopback name parses");
        assert_eq!(b.extension(), "9199");
        assert_eq!(b.leg(), LoopbackLeg::B);
    }

    // `loopback/app=bridge:x/y` is reduced to the bare application token before
    // the name is built.
    #[test]
    fn app_form_carries_the_application_token() {
        let n = LoopbackChannelName::parse("loopback/bridge-a").expect("loopback name parses");
        assert_eq!(n.extension(), "bridge");
    }

    // The suffix is always present, so exactly one is stripped.
    #[test]
    fn an_extension_may_itself_end_in_a_leg_suffix() {
        let n = LoopbackChannelName::parse("loopback/park-a-b").expect("loopback name parses");
        assert_eq!(n.extension(), "park-a");
        assert_eq!(n.leg(), LoopbackLeg::B);
    }

    #[test]
    fn non_loopback_and_malformed_are_none() {
        for name in [
            "loopback/9199",
            "loopback/9199-A",
            "loopback/9199-c",
            "loopback/-a",
            "loopback/",
            "loopback",
            "sofia/internal/1000@example.com",
            "Loopback/9199-a",
            "",
        ] {
            assert_eq!(LoopbackChannelName::parse(name), None, "{name:?}");
        }
    }

    // The variable is uppercase; the name suffix is lowercase and never parses
    // through FromStr.
    #[test]
    fn leg_wire_form_is_the_variable_spelling() {
        assert_eq!(LoopbackLeg::A.as_str(), "A");
        assert_eq!("B".parse(), Ok(LoopbackLeg::B));
        assert!("a"
            .parse::<LoopbackLeg>()
            .is_err());
    }
}
