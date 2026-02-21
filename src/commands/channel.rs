//! Builders for `uuid_*` API commands that target a specific channel by UUID.

use std::fmt;

/// Answer a channel: `uuid_answer <uuid>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidAnswer {
    /// Channel UUID.
    pub uuid: String,
}

impl fmt::Display for UuidAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "uuid_answer {}", self.uuid)
    }
}

/// Bridge two channels: `uuid_bridge <uuid> <other_uuid>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidBridge {
    /// Channel UUID.
    pub uuid: String,
    /// UUID of the channel to bridge to.
    pub other: String,
}

impl fmt::Display for UuidBridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "uuid_bridge {} {}", self.uuid, self.other)
    }
}

/// Deflect (redirect) a channel to a new SIP URI: `uuid_deflect <uuid> <uri>`.
///
/// Sends a SIP REFER to the endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidDeflect {
    /// Channel UUID.
    pub uuid: String,
    /// SIP URI to redirect to (e.g. `sip:user@host`).
    pub uri: String,
}

impl fmt::Display for UuidDeflect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "uuid_deflect {} {}", self.uuid, self.uri)
    }
}

/// Place a channel on hold or take it off hold: `uuid_hold [off] <uuid>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidHold {
    /// Channel UUID.
    pub uuid: String,
    /// `true` = take off hold; `false` = place on hold.
    pub off: bool,
}

impl fmt::Display for UuidHold {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.off {
            write!(f, "uuid_hold off {}", self.uuid)
        } else {
            write!(f, "uuid_hold {}", self.uuid)
        }
    }
}

/// Kill a channel: `uuid_kill <uuid> [cause]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidKill {
    /// Channel UUID.
    pub uuid: String,
    /// Hangup cause string (e.g. `NORMAL_CLEARING`). If `None`, uses FreeSWITCH default.
    pub cause: Option<String>,
}

impl fmt::Display for UuidKill {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "uuid_kill {}", self.uuid)?;
        if let Some(ref cause) = self.cause {
            write!(f, " {}", cause)?;
        }
        Ok(())
    }
}

/// Get a channel variable: `uuid_getvar <uuid> <key>`.
///
/// Note: FreeSWITCH returns the bare value (no `+OK` prefix), so
/// [`EslResponse::reply_status()`](crate::EslResponse::reply_status) will be
/// [`ReplyStatus::Other`](crate::ReplyStatus::Other).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidGetVar {
    /// Channel UUID.
    pub uuid: String,
    /// Variable name to read.
    pub key: String,
}

impl fmt::Display for UuidGetVar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "uuid_getvar {} {}", self.uuid, self.key)
    }
}

/// Set a channel variable: `uuid_setvar <uuid> <key> <value>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidSetVar {
    /// Channel UUID.
    pub uuid: String,
    /// Variable name to set.
    pub key: String,
    /// Variable value.
    pub value: String,
}

impl fmt::Display for UuidSetVar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "uuid_setvar {} {} {}", self.uuid, self.key, self.value)
    }
}

/// Transfer a channel to a new destination: `uuid_transfer <uuid> <dest> [dialplan]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidTransfer {
    /// Channel UUID.
    pub uuid: String,
    /// Destination extension or dial string.
    pub destination: String,
    /// Dialplan to use (e.g. `"XML"`). If `None`, uses the channel's current dialplan.
    pub dialplan: Option<String>,
}

impl fmt::Display for UuidTransfer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "uuid_transfer {} {}", self.uuid, self.destination)?;
        if let Some(ref dp) = self.dialplan {
            write!(f, " {}", dp)?;
        }
        Ok(())
    }
}

/// Send DTMF digits to a channel: `uuid_send_dtmf <uuid> <digits>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidSendDtmf {
    /// Channel UUID.
    pub uuid: String,
    /// DTMF digit string (e.g. `"1234#"`).
    pub dtmf: String,
}

impl fmt::Display for UuidSendDtmf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "uuid_send_dtmf {} {}", self.uuid, self.dtmf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "abc12345-6789-0abc-def0-123456789abc";
    const OTHER: &str = "def12345-6789-0abc-def0-123456789abc";

    #[test]
    fn uuid_answer() {
        let cmd = UuidAnswer { uuid: UUID.into() };
        assert_eq!(cmd.to_string(), format!("uuid_answer {}", UUID));
    }

    #[test]
    fn uuid_bridge() {
        let cmd = UuidBridge {
            uuid: UUID.into(),
            other: OTHER.into(),
        };
        assert_eq!(cmd.to_string(), format!("uuid_bridge {} {}", UUID, OTHER));
    }

    #[test]
    fn uuid_deflect() {
        let cmd = UuidDeflect {
            uuid: UUID.into(),
            uri: "sip:user@host".into(),
        };
        assert_eq!(
            cmd.to_string(),
            format!("uuid_deflect {} sip:user@host", UUID)
        );
    }

    #[test]
    fn uuid_hold_on() {
        let cmd = UuidHold {
            uuid: UUID.into(),
            off: false,
        };
        assert_eq!(cmd.to_string(), format!("uuid_hold {}", UUID));
    }

    #[test]
    fn uuid_hold_off() {
        let cmd = UuidHold {
            uuid: UUID.into(),
            off: true,
        };
        assert_eq!(cmd.to_string(), format!("uuid_hold off {}", UUID));
    }

    #[test]
    fn uuid_kill_no_cause() {
        let cmd = UuidKill {
            uuid: UUID.into(),
            cause: None,
        };
        assert_eq!(cmd.to_string(), format!("uuid_kill {}", UUID));
    }

    #[test]
    fn uuid_kill_with_cause() {
        let cmd = UuidKill {
            uuid: UUID.into(),
            cause: Some("NORMAL_CLEARING".into()),
        };
        assert_eq!(
            cmd.to_string(),
            format!("uuid_kill {} NORMAL_CLEARING", UUID)
        );
    }

    #[test]
    fn uuid_getvar() {
        let cmd = UuidGetVar {
            uuid: UUID.into(),
            key: "sip_call_id".into(),
        };
        assert_eq!(cmd.to_string(), format!("uuid_getvar {} sip_call_id", UUID));
    }

    #[test]
    fn uuid_setvar() {
        let cmd = UuidSetVar {
            uuid: UUID.into(),
            key: "hangup_after_bridge".into(),
            value: "true".into(),
        };
        assert_eq!(
            cmd.to_string(),
            format!("uuid_setvar {} hangup_after_bridge true", UUID)
        );
    }

    #[test]
    fn uuid_transfer_no_dialplan() {
        let cmd = UuidTransfer {
            uuid: UUID.into(),
            destination: "1000".into(),
            dialplan: None,
        };
        assert_eq!(cmd.to_string(), format!("uuid_transfer {} 1000", UUID));
    }

    #[test]
    fn uuid_transfer_with_dialplan() {
        let cmd = UuidTransfer {
            uuid: UUID.into(),
            destination: "1000".into(),
            dialplan: Some("XML".into()),
        };
        assert_eq!(cmd.to_string(), format!("uuid_transfer {} 1000 XML", UUID));
    }

    #[test]
    fn uuid_send_dtmf() {
        let cmd = UuidSendDtmf {
            uuid: UUID.into(),
            dtmf: "1234#".into(),
        };
        assert_eq!(cmd.to_string(), format!("uuid_send_dtmf {} 1234#", UUID));
    }
}
