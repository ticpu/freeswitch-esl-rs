use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidAnswer {
    pub uuid: String,
}

impl fmt::Display for UuidAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "uuid_answer {}", self.uuid)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidBridge {
    pub uuid: String,
    pub other: String,
}

impl fmt::Display for UuidBridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "uuid_bridge {} {}", self.uuid, self.other)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidDeflect {
    pub uuid: String,
    pub uri: String,
}

impl fmt::Display for UuidDeflect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "uuid_deflect {} {}", self.uuid, self.uri)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidHold {
    pub uuid: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidKill {
    pub uuid: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidGetVar {
    pub uuid: String,
    pub key: String,
}

impl fmt::Display for UuidGetVar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "uuid_getvar {} {}", self.uuid, self.key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidSetVar {
    pub uuid: String,
    pub key: String,
    pub value: String,
}

impl fmt::Display for UuidSetVar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "uuid_setvar {} {} {}", self.uuid, self.key, self.value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidTransfer {
    pub uuid: String,
    pub destination: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidSendDtmf {
    pub uuid: String,
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
