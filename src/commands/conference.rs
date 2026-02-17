use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteAction {
    Mute,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConferenceMute {
    pub name: String,
    pub action: MuteAction,
    pub member_id: String,
}

impl fmt::Display for ConferenceMute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "conference {} {} {}",
            self.name, self.action, self.member_id
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldAction {
    Hold,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConferenceHold {
    pub name: String,
    pub action: HoldAction,
    pub member: String,
    pub stream: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConferenceDtmf {
    pub name: String,
    pub member: String,
    pub dtmf: String,
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
            member_id: "5".into(),
        };
        assert_eq!(cmd.to_string(), "conference conf1 mute 5");
    }

    #[test]
    fn conference_unmute() {
        let cmd = ConferenceMute {
            name: "conf1".into(),
            action: MuteAction::Unmute,
            member_id: "5".into(),
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
}
