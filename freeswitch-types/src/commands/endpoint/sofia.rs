use std::str::FromStr;

use super::{extract_variables, strip_endpoint_prefix};
use crate::commands::originate::OriginateError;
use crate::commands::variables::DialStringCarrier;
use crate::commands::variables::Variables;

/// SIP endpoint via a named profile: `sofia/{profile}/{destination}`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct SofiaEndpoint {
    /// SIP profile name (e.g. `internal`, `external`).
    pub profile: String,
    /// SIP URI or destination number.
    pub destination: String,
    /// Per-channel variables prepended as `{key=value}`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub variables: Option<Variables>,
}

/// SIP endpoint via a configured gateway:
/// `sofia/gateway/[{profile}::]{gateway}/{destination}`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct SofiaGateway {
    /// Gateway name as configured in the SIP profile.
    pub gateway: String,
    /// Destination number or SIP user part.
    pub destination: String,
    /// SIP profile name to qualify the gateway lookup.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub profile: Option<String>,
    /// Per-channel variables prepended as `{key=value}`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub variables: Option<Variables>,
}

/// Runtime expression resolving registered SIP contacts:
/// `${sofia_contact([profile/]user@domain)}`.
///
/// The library produces the expression string; FreeSWITCH evaluates it
/// at call time.
///
/// The special profile `"*"` makes FreeSWITCH iterate over every loaded
/// SIP profile to find the user's registration, instead of searching a
/// single named profile (see `sofia_contact_function` in
/// `mod_sofia.c` where `strcmp(profile_name, "*")` skips the single-profile
/// lookup and falls through to the all-profiles hash iteration).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct SofiaContact {
    /// User part of the contact lookup.
    pub user: String,
    /// Domain for the contact lookup.
    pub domain: String,
    /// SIP profile name, or `"*"` for all profiles.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub profile: Option<String>,
    /// Per-channel variables prepended as `{key=value}`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub variables: Option<Variables>,
}

impl SofiaEndpoint {
    /// Create a new SIP profile endpoint.
    pub fn new(profile: impl Into<String>, destination: impl Into<String>) -> Self {
        Self {
            profile: profile.into(),
            destination: destination.into(),
            variables: None,
        }
    }
}

impl SofiaGateway {
    /// Create a new SIP gateway endpoint.
    pub fn new(gateway: impl Into<String>, destination: impl Into<String>) -> Self {
        Self {
            gateway: gateway.into(),
            destination: destination.into(),
            profile: None,
            variables: None,
        }
    }

    /// Set the SIP profile qualifier.
    pub fn with_profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = Some(profile.into());
        self
    }
}

impl SofiaContact {
    /// Create a new sofia_contact runtime expression.
    pub fn new(user: impl Into<String>, domain: impl Into<String>) -> Self {
        Self {
            user: user.into(),
            domain: domain.into(),
            profile: None,
            variables: None,
        }
    }

    /// Set the SIP profile for the contact lookup.
    ///
    /// Pass `"*"` to search all loaded SIP profiles for the user's
    /// registration instead of a single named profile.
    pub fn with_profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = Some(profile.into());
        self
    }
}

impl_dial_string_with_variables!(SofiaEndpoint, |this, f| write!(
    f,
    "sofia/{}/{}",
    this.profile, this.destination
));

impl_dial_string_with_variables!(SofiaGateway, |this, f| match &this.profile {
    Some(p) => write!(
        f,
        "sofia/gateway/{}::{}/{}",
        p, this.gateway, this.destination
    ),
    None => write!(f, "sofia/gateway/{}/{}", this.gateway, this.destination),
});

impl_dial_string_with_variables!(SofiaContact, |this, f| match &this.profile {
    Some(p) => write!(f, "${{sofia_contact({}/{}@{})}}", p, this.user, this.domain),
    None => write!(f, "${{sofia_contact({}@{})}}", this.user, this.domain),
});

impl FromStr for SofiaEndpoint {
    type Err = OriginateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (variables, rest) =
            strip_endpoint_prefix(s, "sofia/", "sofia", DialStringCarrier::EslApi)?;
        let (profile, destination) = rest
            .split_once('/')
            .ok_or_else(|| {
                OriginateError::ParseError("sofia endpoint needs profile/destination".into())
            })?;
        Ok(Self {
            profile: profile.into(),
            destination: destination.into(),
            variables,
        })
    }
}

impl FromStr for SofiaGateway {
    type Err = OriginateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (variables, rest) = strip_endpoint_prefix(
            s,
            "sofia/gateway/",
            "sofia gateway",
            DialStringCarrier::EslApi,
        )?;
        let (gateway_part, destination) = rest
            .split_once('/')
            .ok_or_else(|| {
                OriginateError::ParseError("sofia gateway needs gateway/destination".into())
            })?;
        let (profile, gateway) = if let Some((p, g)) = gateway_part.split_once("::") {
            (Some(p.to_string()), g.to_string())
        } else {
            (None, gateway_part.to_string())
        };
        Ok(Self {
            gateway,
            destination: destination.into(),
            profile,
            variables,
        })
    }
}

impl FromStr for SofiaContact {
    type Err = OriginateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (variables, uri) = extract_variables(s, DialStringCarrier::EslApi)?;
        let inner = uri
            .strip_prefix("${sofia_contact(")
            .and_then(|r| r.strip_suffix(")}"))
            .ok_or_else(|| OriginateError::ParseError("not a sofia_contact expression".into()))?;
        let (profile, user_at_domain) = if let Some((p, rest)) = inner.split_once('/') {
            (Some(p.to_string()), rest)
        } else {
            (None, inner)
        };
        let (user, domain) = user_at_domain
            .split_once('@')
            .ok_or_else(|| OriginateError::ParseError("sofia_contact needs user@domain".into()))?;
        Ok(Self {
            user: user.into(),
            domain: domain.into(),
            profile,
            variables,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::variables::VariablesType;

    // --- SofiaEndpoint ---

    #[test]
    fn sofia_endpoint_display() {
        let ep = SofiaEndpoint {
            profile: "internal".into(),
            destination: "1000@example.com".into(),
            variables: None,
        };
        assert_eq!(ep.to_string(), "sofia/internal/1000@example.com");
    }

    #[test]
    fn sofia_endpoint_display_with_variables() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("originate_timeout", "30");
        let ep = SofiaEndpoint {
            profile: "internal".into(),
            destination: "1000@example.com".into(),
            variables: Some(vars),
        };
        assert_eq!(
            ep.to_string(),
            "{originate_timeout=30}sofia/internal/1000@example.com"
        );
    }

    #[test]
    fn sofia_endpoint_from_str() {
        let ep: SofiaEndpoint = "sofia/internal/1000@example.com"
            .parse()
            .unwrap();
        assert_eq!(ep.profile, "internal");
        assert_eq!(ep.destination, "1000@example.com");
        assert!(ep
            .variables
            .is_none());
    }

    #[test]
    fn sofia_endpoint_from_str_with_variables() {
        let ep: SofiaEndpoint = "{originate_timeout=30}sofia/internal/1000@example.com"
            .parse()
            .unwrap();
        assert_eq!(ep.profile, "internal");
        assert_eq!(ep.destination, "1000@example.com");
        assert_eq!(
            ep.variables
                .unwrap()
                .get("originate_timeout"),
            Some("30")
        );
    }

    #[test]
    fn sofia_endpoint_round_trip() {
        let ep = SofiaEndpoint {
            profile: "external".into(),
            destination: "sip:user@host:5060".into(),
            variables: None,
        };
        let s = ep.to_string();
        let parsed: SofiaEndpoint = s
            .parse()
            .unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn serde_sofia_endpoint() {
        let ep = SofiaEndpoint {
            profile: "internal".into(),
            destination: "1000@example.com".into(),
            variables: None,
        };
        let json = serde_json::to_string(&ep).unwrap();
        let parsed: SofiaEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn serde_sofia_endpoint_with_variables() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("originate_timeout", "30");
        let ep = SofiaEndpoint {
            profile: "internal".into(),
            destination: "1000@example.com".into(),
            variables: Some(vars),
        };
        let json = serde_json::to_string(&ep).unwrap();
        let parsed: SofiaEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }

    // --- SofiaGateway ---

    #[test]
    fn sofia_gateway_display() {
        let ep = SofiaGateway {
            gateway: "my_provider".into(),
            destination: "18005551234".into(),
            profile: None,
            variables: None,
        };
        assert_eq!(ep.to_string(), "sofia/gateway/my_provider/18005551234");
    }

    #[test]
    fn sofia_gateway_display_with_profile() {
        let ep = SofiaGateway {
            gateway: "my_provider".into(),
            destination: "18005551234".into(),
            profile: Some("external".into()),
            variables: None,
        };
        assert_eq!(
            ep.to_string(),
            "sofia/gateway/external::my_provider/18005551234"
        );
    }

    #[test]
    fn sofia_gateway_display_with_variables() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("sip_h_X-Custom", "value");
        let ep = SofiaGateway {
            gateway: "gw1".into(),
            destination: "1234".into(),
            profile: None,
            variables: Some(vars),
        };
        assert_eq!(
            ep.to_string(),
            "{sip_h_X-Custom=value}sofia/gateway/gw1/1234"
        );
    }

    #[test]
    fn sofia_gateway_from_str() {
        let ep: SofiaGateway = "sofia/gateway/my_provider/18005551234"
            .parse()
            .unwrap();
        assert_eq!(ep.gateway, "my_provider");
        assert_eq!(ep.destination, "18005551234");
        assert!(ep
            .profile
            .is_none());
    }

    #[test]
    fn sofia_gateway_from_str_with_profile() {
        let ep: SofiaGateway = "sofia/gateway/external::my_provider/18005551234"
            .parse()
            .unwrap();
        assert_eq!(ep.gateway, "my_provider");
        assert_eq!(ep.destination, "18005551234");
        assert_eq!(
            ep.profile
                .as_deref(),
            Some("external")
        );
    }

    #[test]
    fn sofia_gateway_round_trip() {
        let ep = SofiaGateway {
            gateway: "carrier".into(),
            destination: "15551234567".into(),
            profile: Some("external".into()),
            variables: None,
        };
        let s = ep.to_string();
        let parsed: SofiaGateway = s
            .parse()
            .unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn serde_sofia_gateway() {
        let ep = SofiaGateway {
            gateway: "my_provider".into(),
            destination: "18005551234".into(),
            profile: None,
            variables: None,
        };
        let json = serde_json::to_string(&ep).unwrap();
        let parsed: SofiaGateway = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn serde_sofia_gateway_with_profile() {
        let ep = SofiaGateway {
            gateway: "my_provider".into(),
            destination: "18005551234".into(),
            profile: Some("external".into()),
            variables: None,
        };
        let json = serde_json::to_string(&ep).unwrap();
        let parsed: SofiaGateway = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }

    // --- SofiaContact ---

    #[test]
    fn sofia_contact_display() {
        let ep = SofiaContact {
            user: "1000".into(),
            domain: "example.com".into(),
            profile: None,
            variables: None,
        };
        assert_eq!(ep.to_string(), "${sofia_contact(1000@example.com)}");
    }

    #[test]
    fn sofia_contact_display_with_profile() {
        let ep = SofiaContact {
            user: "1000".into(),
            domain: "example.com".into(),
            profile: Some("internal".into()),
            variables: None,
        };
        assert_eq!(
            ep.to_string(),
            "${sofia_contact(internal/1000@example.com)}"
        );
    }

    #[test]
    fn sofia_contact_display_all_profiles() {
        let ep = SofiaContact {
            user: "1000".into(),
            domain: "example.com".into(),
            profile: Some("*".into()),
            variables: None,
        };
        assert_eq!(ep.to_string(), "${sofia_contact(*/1000@example.com)}");
    }

    #[test]
    fn sofia_contact_display_with_variables() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("presence_id", "1000@example.com");
        let ep = SofiaContact {
            user: "1000".into(),
            domain: "example.com".into(),
            profile: None,
            variables: Some(vars),
        };
        assert_eq!(
            ep.to_string(),
            "{presence_id=1000@example.com}${sofia_contact(1000@example.com)}"
        );
    }

    #[test]
    fn sofia_contact_from_str() {
        let ep: SofiaContact = "${sofia_contact(1000@example.com)}"
            .parse()
            .unwrap();
        assert_eq!(ep.user, "1000");
        assert_eq!(ep.domain, "example.com");
        assert!(ep
            .profile
            .is_none());
    }

    #[test]
    fn sofia_contact_from_str_with_profile() {
        let ep: SofiaContact = "${sofia_contact(internal/1000@example.com)}"
            .parse()
            .unwrap();
        assert_eq!(ep.user, "1000");
        assert_eq!(ep.domain, "example.com");
        assert_eq!(
            ep.profile
                .as_deref(),
            Some("internal")
        );
    }

    #[test]
    fn sofia_contact_round_trip() {
        let ep = SofiaContact {
            user: "bob".into(),
            domain: "example.com".into(),
            profile: Some("*".into()),
            variables: None,
        };
        let s = ep.to_string();
        let parsed: SofiaContact = s
            .parse()
            .unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn sofia_contact_profile_with_at_sign() {
        let ep = SofiaContact::new("1000", "example.com").with_profile("user@realm");
        let s = ep.to_string();
        assert_eq!(s, "${sofia_contact(user@realm/1000@example.com)}");
        // Round-trip: the first @ is in profile, the parser splits on / first
        // then finds @ in the user_at_domain part
        let parsed: SofiaContact = s
            .parse()
            .unwrap();
        assert_eq!(
            parsed
                .profile
                .as_deref(),
            Some("user@realm")
        );
        assert_eq!(parsed.user, "1000");
        assert_eq!(parsed.domain, "example.com");
    }

    #[test]
    fn serde_sofia_contact() {
        let ep = SofiaContact {
            user: "1000".into(),
            domain: "example.com".into(),
            profile: Some("*".into()),
            variables: None,
        };
        let json = serde_json::to_string(&ep).unwrap();
        let parsed: SofiaContact = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }
}
