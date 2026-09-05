//! Builders for `conference` API sub-commands (mute, hold, DTMF).

use std::fmt;

wire_enum! {
    /// Conference member mute/unmute action.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum MuteAction {
        /// Mute the member's audio.
        Mute => "mute",
        /// Unmute the member's audio.
        Unmute => "unmute",
    }
    error ParseMuteActionError("mute action");
    tests: mute_action_tests;
}

/// Mute or unmute a conference member: `conference <name> mute|unmute <member>`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConferenceMute {
    /// Conference room name.
    pub name: String,
    /// Mute or unmute.
    pub action: MuteAction,
    /// Member ID, or `"all"` for all members.
    pub member: String,
}

impl ConferenceMute {
    /// Create a new conference mute/unmute command.
    pub fn new(name: impl Into<String>, action: MuteAction, member: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            action,
            member: member.into(),
        }
    }
}

impl fmt::Display for ConferenceMute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "conference {} {} {}",
            self.name, self.action, self.member
        )
    }
}

wire_enum! {
    /// Conference member hold/unhold action.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum HoldAction {
        /// Place the member on hold with music-on-hold.
        Hold => "hold",
        /// Return the member to the conference.
        Unhold => "unhold",
    }
    error ParseHoldActionError("hold action");
    tests: hold_action_tests;
}

/// Hold or unhold a conference member: `conference <name> hold|unhold <member> [stream]`.
///
/// `Hold` plays music-on-hold to the member; `Unhold` returns them to the conference.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConferenceHold {
    /// Conference room name.
    pub name: String,
    /// Hold or unhold.
    pub action: HoldAction,
    /// Member ID, or `"all"` for all members.
    pub member: String,
    /// MOH stream URI (e.g. `local_stream://moh`). Only meaningful with `HoldAction::Hold`.
    pub stream: Option<String>,
}

impl ConferenceHold {
    /// Create a new conference hold/unhold command.
    pub fn new(name: impl Into<String>, action: HoldAction, member: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            action,
            member: member.into(),
            stream: None,
        }
    }

    /// Set the MOH stream URI for hold.
    pub fn with_stream(mut self, stream: impl Into<String>) -> Self {
        self.stream = Some(stream.into());
        self
    }
}

impl fmt::Display for ConferenceHold {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "conference {} {} {}",
            self.name, self.action, self.member
        )?;
        if let Some(ref stream) = self.stream {
            write!(f, " {}", stream)?;
        }
        Ok(())
    }
}

/// Send DTMF to conference members: `conference <name> dtmf <member> <digits>`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConferenceDtmf {
    /// Conference room name.
    pub name: String,
    /// Member ID, or `"all"`.
    pub member: String,
    /// DTMF digit string (e.g. `"1234#"`).
    pub dtmf: String,
}

impl ConferenceDtmf {
    /// Create a new conference DTMF command.
    pub fn new(
        name: impl Into<String>,
        member: impl Into<String>,
        dtmf: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            member: member.into(),
            dtmf: dtmf.into(),
        }
    }
}

impl fmt::Display for ConferenceDtmf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "conference {} dtmf {} {}",
            self.name, self.member, self.dtmf
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These types are `#[non_exhaustive]`, so a constructor is the only way an
    /// external caller can build one and the only path worth pinning.
    #[test]
    fn conference_mute_renders_both_actions() {
        for (action, wire) in [
            (MuteAction::Mute, "conference conf1 mute 5"),
            (MuteAction::Unmute, "conference conf1 unmute 5"),
        ] {
            assert_eq!(ConferenceMute::new("conf1", action, "5").to_string(), wire);
        }
    }

    #[test]
    fn conference_hold_renders_both_actions_and_the_stream() {
        assert_eq!(
            ConferenceHold::new("conf1", HoldAction::Hold, "all").to_string(),
            "conference conf1 hold all"
        );
        assert_eq!(
            ConferenceHold::new("conf1", HoldAction::Unhold, "all").to_string(),
            "conference conf1 unhold all"
        );
        assert_eq!(
            ConferenceHold::new("conf1", HoldAction::Hold, "all")
                .with_stream("local_stream://moh")
                .to_string(),
            "conference conf1 hold all local_stream://moh"
        );
    }

    #[test]
    fn conference_dtmf_renders() {
        assert_eq!(
            ConferenceDtmf::new("conf1", "all", "1234").to_string(),
            "conference conf1 dtmf all 1234"
        );
    }
}
