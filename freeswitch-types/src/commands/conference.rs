//! Builders for `conference` API sub-commands (mute, hold, DTMF).

use std::fmt;

/// Conference member mute/unmute action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MuteAction {
    /// Mute the member's audio.
    Mute,
    /// Unmute the member's audio.
    Unmute,
}

impl fmt::Display for MuteAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mute => f.write_str("mute"),
            Self::Unmute => f.write_str("unmute"),
        }
    }
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

/// Conference member hold/unhold action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HoldAction {
    /// Place the member on hold with music-on-hold.
    Hold,
    /// Return the member to the conference.
    Unhold,
}

impl fmt::Display for HoldAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hold => f.write_str("hold"),
            Self::Unhold => f.write_str("unhold"),
        }
    }
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

    #[test]
    fn conference_mute() {
        let cmd = ConferenceMute {
            name: "conf1".into(),
            action: MuteAction::Mute,
            member: "5".into(),
        };
        assert_eq!(cmd.to_string(), "conference conf1 mute 5");
    }

    #[test]
    fn conference_unmute() {
        let cmd = ConferenceMute {
            name: "conf1".into(),
            action: MuteAction::Unmute,
            member: "5".into(),
        };
        assert_eq!(cmd.to_string(), "conference conf1 unmute 5");
    }

    #[test]
    fn conference_hold_all() {
        let cmd = ConferenceHold {
            name: "conf1".into(),
            action: HoldAction::Hold,
            member: "all".into(),
            stream: None,
        };
        assert_eq!(cmd.to_string(), "conference conf1 hold all");
    }

    #[test]
    fn conference_hold_with_stream() {
        let cmd = ConferenceHold {
            name: "conf1".into(),
            action: HoldAction::Hold,
            member: "all".into(),
            stream: Some("local_stream://moh".into()),
        };
        assert_eq!(
            cmd.to_string(),
            "conference conf1 hold all local_stream://moh"
        );
    }

    #[test]
    fn conference_unhold() {
        let cmd = ConferenceHold {
            name: "conf1".into(),
            action: HoldAction::Unhold,
            member: "all".into(),
            stream: None,
        };
        assert_eq!(cmd.to_string(), "conference conf1 unhold all");
    }

    #[test]
    fn conference_dtmf() {
        let cmd = ConferenceDtmf {
            name: "conf1".into(),
            member: "all".into(),
            dtmf: "1234".into(),
        };
        assert_eq!(cmd.to_string(), "conference conf1 dtmf all 1234");
    }

    #[test]
    fn mute_action_from_str() {
        assert_eq!(
            "mute"
                .parse::<MuteAction>()
                .unwrap(),
            MuteAction::Mute
        );
        assert_eq!(
            "unmute"
                .parse::<MuteAction>()
                .unwrap(),
            MuteAction::Unmute
        );
        assert!("Mute"
            .parse::<MuteAction>()
            .is_err());
        assert!("MUTE"
            .parse::<MuteAction>()
            .is_err());
    }

    #[test]
    fn hold_action_from_str() {
        assert_eq!(
            "hold"
                .parse::<HoldAction>()
                .unwrap(),
            HoldAction::Hold
        );
        assert_eq!(
            "unhold"
                .parse::<HoldAction>()
                .unwrap(),
            HoldAction::Unhold
        );
        assert!("Hold"
            .parse::<HoldAction>()
            .is_err());
        assert!("HOLD"
            .parse::<HoldAction>()
            .is_err());
    }
}
