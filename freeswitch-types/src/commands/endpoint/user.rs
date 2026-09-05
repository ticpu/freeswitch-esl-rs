use std::str::FromStr;

use super::strip_endpoint_prefix;
use crate::commands::originate::OriginateError;
use crate::commands::variables::DialStringCarrier;
use crate::commands::variables::Variables;

/// Directory-based endpoint: `user/{name}[@{domain}]`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct UserEndpoint {
    /// User name from the directory.
    pub name: String,
    /// Domain name (optional, uses default domain if absent).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub domain: Option<String>,
    /// Per-channel variables prepended as `{key=value}`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub variables: Option<Variables>,
}

impl UserEndpoint {
    /// Create a new user endpoint.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            domain: None,
            variables: None,
        }
    }

    /// Set the domain.
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }
}

impl_dial_string_with_variables!(UserEndpoint, |this, f| match &this.domain {
    Some(d) => write!(f, "user/{}@{}", this.name, d),
    None => write!(f, "user/{}", this.name),
});

impl FromStr for UserEndpoint {
    type Err = OriginateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (variables, rest) =
            strip_endpoint_prefix(s, "user/", "user", DialStringCarrier::EslApi)?;
        let (name, domain) = if let Some((n, d)) = rest.split_once('@') {
            (n.to_string(), Some(d.to_string()))
        } else {
            (rest.to_string(), None)
        };
        Ok(Self {
            name,
            domain,
            variables,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_endpoint_display() {
        let ep = UserEndpoint {
            name: "1000".into(),
            domain: Some("example.com".into()),
            variables: None,
        };
        assert_eq!(ep.to_string(), "user/1000@example.com");
    }

    #[test]
    fn user_endpoint_display_no_domain() {
        let ep = UserEndpoint {
            name: "1000".into(),
            domain: None,
            variables: None,
        };
        assert_eq!(ep.to_string(), "user/1000");
    }

    #[test]
    fn user_endpoint_from_str() {
        let ep: UserEndpoint = "user/1000@example.com"
            .parse()
            .unwrap();
        assert_eq!(ep.name, "1000");
        assert_eq!(
            ep.domain
                .as_deref(),
            Some("example.com")
        );
    }

    #[test]
    fn user_endpoint_from_str_no_domain() {
        let ep: UserEndpoint = "user/1000"
            .parse()
            .unwrap();
        assert_eq!(ep.name, "1000");
        assert!(ep
            .domain
            .is_none());
    }

    #[test]
    fn user_endpoint_round_trip() {
        let ep = UserEndpoint {
            name: "bob".into(),
            domain: Some("example.com".into()),
            variables: None,
        };
        let s = ep.to_string();
        let parsed: UserEndpoint = s
            .parse()
            .unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn serde_user_endpoint() {
        let ep = UserEndpoint {
            name: "1000".into(),
            domain: Some("example.com".into()),
            variables: None,
        };
        let json = serde_json::to_string(&ep).unwrap();
        let parsed: UserEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn serde_user_endpoint_no_domain() {
        let ep = UserEndpoint {
            name: "1000".into(),
            domain: None,
            variables: None,
        };
        let json = serde_json::to_string(&ep).unwrap();
        let parsed: UserEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }
}
