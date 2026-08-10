//! FreeSWITCH endpoint types for originate and bridge dial strings.
//!
//! Each endpoint type corresponds to a real FreeSWITCH endpoint module
//! or runtime expression. Concrete structs implement the [`DialString`]
//! trait independently; the [`Endpoint`] enum wraps them for
//! serialization and polymorphic storage.

mod audio;
mod error;
mod group_call;
mod loopback;
mod sofia;
mod user;

pub use audio::AudioEndpoint;
pub use error::ErrorEndpoint;
pub use group_call::{GroupCall, GroupCallOrder, ParseGroupCallOrderError};
pub use loopback::LoopbackEndpoint;
pub use sofia::{SofiaContact, SofiaEndpoint, SofiaGateway};
pub use user::UserEndpoint;

use std::fmt;
use std::str::FromStr;

use super::find_matching_bracket;
use super::originate::OriginateError;
use super::variables::{DialStringCarrier, Variables};

/// Common interface for anything that formats as a FreeSWITCH dial string.
///
/// Implemented on each concrete endpoint struct and on the [`Endpoint`] enum.
/// Downstream crates can implement this on custom endpoint types.
pub trait DialString: fmt::Display {
    /// Per-endpoint variables, if any.
    fn variables(&self) -> Option<&Variables>;
    /// Mutable access to per-endpoint variables.
    fn variables_mut(&mut self) -> Option<&mut Variables>;
    /// Replace per-endpoint variables.
    fn set_variables(&mut self, vars: Option<Variables>);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_variables(
    f: &mut fmt::Formatter<'_>,
    vars: &Option<Variables>,
    carrier: DialStringCarrier,
) -> fmt::Result {
    if let Some(vars) = vars {
        if !vars.is_empty() {
            write!(f, "{}", vars.display_for(carrier))?;
        }
    }
    Ok(())
}

/// Strip the leading variable block then a fixed endpoint prefix.
///
/// Returns `(variables, rest_after_prefix)`. The `kind` label is used in the
/// `not a {kind} endpoint` error when the prefix is missing.
pub(super) fn strip_endpoint_prefix<'a>(
    s: &'a str,
    prefix: &str,
    kind: &str,
    carrier: DialStringCarrier,
) -> Result<(Option<Variables>, &'a str), OriginateError> {
    let (variables, uri) = extract_variables(s, carrier)?;
    let rest = uri
        .strip_prefix(prefix)
        .ok_or_else(|| OriginateError::ParseError(format!("not a {} endpoint", kind)))?;
    Ok((variables, rest))
}

/// Extract a leading variable block (`{...}`, `[...]`, or `<...>`) from a
/// dial string, returning the parsed variables and the remaining URI portion.
///
/// Uses depth-aware bracket matching so nested brackets in values (e.g.
/// `<sip_h_Call-Info=<url>>`) don't cause premature closure.
fn extract_variables(
    s: &str,
    carrier: DialStringCarrier,
) -> Result<(Option<Variables>, &str), OriginateError> {
    let (open, close_ch) = match s
        .as_bytes()
        .first()
    {
        Some(b'{') => ('{', '}'),
        Some(b'[') => ('[', ']'),
        Some(b'<') => ('<', '>'),
        _ => return Ok((None, s)),
    };
    let close = find_matching_bracket(s, open, close_ch)
        .ok_or_else(|| OriginateError::ParseError(format!("unclosed {} in endpoint", open)))?;
    let var_str = &s[..=close];
    let vars = Variables::parse_for(var_str, carrier)?;
    let vars = if vars.is_empty() { None } else { Some(vars) };
    Ok((vars, s[close + 1..].trim()))
}

// ---------------------------------------------------------------------------
// Endpoint enum
// ---------------------------------------------------------------------------

/// Polymorphic endpoint wrapping all concrete types.
///
/// Use this in [`Originate`](super::originate::Originate) and
/// [`BridgeDialString`](super::bridge::BridgeDialString) where any endpoint type must be accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum Endpoint {
    /// `sofia/{profile}/{destination}`
    Sofia(SofiaEndpoint),
    /// `sofia/gateway/[{profile}::]{gateway}/{destination}`
    SofiaGateway(SofiaGateway),
    /// `loopback/{extension}[/{context}]`
    Loopback(LoopbackEndpoint),
    /// `user/{name}[@{domain}]`
    User(UserEndpoint),
    /// `${sofia_contact([profile/]user@domain)}`
    SofiaContact(SofiaContact),
    /// `${group_call(group@domain[+order])}`
    GroupCall(GroupCall),
    /// `error/{cause}`
    Error(ErrorEndpoint),
    /// `portaudio[/{destination}]`
    #[cfg_attr(feature = "serde", serde(rename = "portaudio"))]
    PortAudio(AudioEndpoint),
    /// `pulseaudio[/{destination}]`
    #[cfg_attr(feature = "serde", serde(rename = "pulseaudio"))]
    PulseAudio(AudioEndpoint),
    /// `alsa[/{destination}]`
    Alsa(AudioEndpoint),
}

// ---------------------------------------------------------------------------
// From impls
// ---------------------------------------------------------------------------

impl From<SofiaEndpoint> for Endpoint {
    fn from(ep: SofiaEndpoint) -> Self {
        Self::Sofia(ep)
    }
}

impl From<SofiaGateway> for Endpoint {
    fn from(ep: SofiaGateway) -> Self {
        Self::SofiaGateway(ep)
    }
}

impl From<LoopbackEndpoint> for Endpoint {
    fn from(ep: LoopbackEndpoint) -> Self {
        Self::Loopback(ep)
    }
}

impl From<UserEndpoint> for Endpoint {
    fn from(ep: UserEndpoint) -> Self {
        Self::User(ep)
    }
}

impl From<SofiaContact> for Endpoint {
    fn from(ep: SofiaContact) -> Self {
        Self::SofiaContact(ep)
    }
}

impl From<GroupCall> for Endpoint {
    fn from(ep: GroupCall) -> Self {
        Self::GroupCall(ep)
    }
}

impl From<ErrorEndpoint> for Endpoint {
    fn from(ep: ErrorEndpoint) -> Self {
        Self::Error(ep)
    }
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

impl Endpoint {
    pub(crate) fn write_for(
        &self,
        f: &mut fmt::Formatter<'_>,
        carrier: DialStringCarrier,
    ) -> fmt::Result {
        match self {
            Self::Sofia(ep) => ep.write_for(f, carrier),
            Self::SofiaGateway(ep) => ep.write_for(f, carrier),
            Self::Loopback(ep) => ep.write_for(f, carrier),
            Self::User(ep) => ep.write_for(f, carrier),
            Self::SofiaContact(ep) => ep.write_for(f, carrier),
            Self::GroupCall(ep) => ep.write_for(f, carrier),
            Self::Error(ep) => fmt::Display::fmt(ep, f),
            Self::PortAudio(ep) => ep.write_with_prefix(f, "portaudio", carrier),
            Self::PulseAudio(ep) => ep.write_with_prefix(f, "pulseaudio", carrier),
            Self::Alsa(ep) => ep.write_with_prefix(f, "alsa", carrier),
        }
    }

    /// Render for a named carrier rather than the
    /// [`DialStringCarrier::EslApi`] default of [`Display`](fmt::Display).
    pub fn display_for(&self, carrier: DialStringCarrier) -> EndpointDisplay<'_> {
        EndpointDisplay {
            endpoint: self,
            carrier,
        }
    }

    /// Parse a dial string written for a named carrier, mirroring
    /// [`display_for`](Self::display_for). [`FromStr`] uses the
    /// [`DialStringCarrier::EslApi`] default.
    pub fn parse_for(s: &str, carrier: DialStringCarrier) -> Result<Self, OriginateError> {
        // Take the leading block at the caller's carrier, then let the endpoint
        // parse what is left; re-attaching avoids every endpoint's FromStr
        // having to thread a carrier it would only forward.
        let (variables, rest) = extract_variables(s, carrier)?;
        let mut endpoint: Self = rest.parse()?;
        if variables.is_some() {
            endpoint.set_variables(variables);
        }
        Ok(endpoint)
    }
}

/// Renders an [`Endpoint`] for one carrier. Returned by
/// [`Endpoint::display_for`].
#[derive(Debug, Clone, Copy)]
pub struct EndpointDisplay<'a> {
    endpoint: &'a Endpoint,
    carrier: DialStringCarrier,
}

impl fmt::Display for EndpointDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.endpoint
            .write_for(f, self.carrier)
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_for(f, DialStringCarrier::EslApi)
    }
}

// ---------------------------------------------------------------------------
// FromStr
// ---------------------------------------------------------------------------

impl FromStr for Endpoint {
    type Err = OriginateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (variables, uri) = extract_variables(s, DialStringCarrier::EslApi)?;
        // Re-assemble with variables for individual FromStr impls
        let full = if variables.is_some() {
            s.to_string()
        } else {
            uri.to_string()
        };

        if uri.starts_with("${sofia_contact(") {
            Ok(Self::SofiaContact(full.parse()?))
        } else if uri.starts_with("${group_call(") {
            Ok(Self::GroupCall(full.parse()?))
        } else if uri.starts_with("error/") {
            Ok(Self::Error(full.parse()?))
        } else if uri.starts_with("loopback/") {
            Ok(Self::Loopback(full.parse()?))
        } else if uri.starts_with("sofia/gateway/") {
            Ok(Self::SofiaGateway(full.parse()?))
        } else if uri.starts_with("sofia/") {
            Ok(Self::Sofia(full.parse()?))
        } else if uri.starts_with("user/") {
            Ok(Self::User(full.parse()?))
        } else if uri.starts_with("portaudio") {
            Ok(Self::PortAudio(AudioEndpoint::parse_with_prefix(
                &full,
                "portaudio",
            )?))
        } else if uri.starts_with("pulseaudio") {
            Ok(Self::PulseAudio(AudioEndpoint::parse_with_prefix(
                &full,
                "pulseaudio",
            )?))
        } else if uri.starts_with("alsa") {
            Ok(Self::Alsa(AudioEndpoint::parse_with_prefix(&full, "alsa")?))
        } else {
            Err(OriginateError::ParseError(format!(
                "unknown endpoint type: {}",
                uri
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// DialString impls
// ---------------------------------------------------------------------------

macro_rules! impl_dial_string_with_variables {
    ($ty:ty) => {
        impl DialString for $ty {
            fn variables(&self) -> Option<&Variables> {
                self.variables
                    .as_ref()
            }
            fn variables_mut(&mut self) -> Option<&mut Variables> {
                self.variables
                    .as_mut()
            }
            fn set_variables(&mut self, vars: Option<Variables>) {
                self.variables = vars;
            }
        }
    };
}

impl_dial_string_with_variables!(SofiaEndpoint);
impl_dial_string_with_variables!(SofiaGateway);
impl_dial_string_with_variables!(LoopbackEndpoint);
impl_dial_string_with_variables!(UserEndpoint);
impl_dial_string_with_variables!(SofiaContact);
impl_dial_string_with_variables!(GroupCall);
impl_dial_string_with_variables!(AudioEndpoint);

impl DialString for ErrorEndpoint {
    fn variables(&self) -> Option<&Variables> {
        None
    }
    fn variables_mut(&mut self) -> Option<&mut Variables> {
        None
    }
    fn set_variables(&mut self, _vars: Option<Variables>) {}
}

impl DialString for Endpoint {
    fn variables(&self) -> Option<&Variables> {
        match self {
            Self::Sofia(ep) => ep.variables(),
            Self::SofiaGateway(ep) => ep.variables(),
            Self::Loopback(ep) => ep.variables(),
            Self::User(ep) => ep.variables(),
            Self::SofiaContact(ep) => ep.variables(),
            Self::GroupCall(ep) => ep.variables(),
            Self::Error(ep) => ep.variables(),
            Self::PortAudio(ep) | Self::PulseAudio(ep) | Self::Alsa(ep) => ep.variables(),
        }
    }
    fn variables_mut(&mut self) -> Option<&mut Variables> {
        match self {
            Self::Sofia(ep) => ep.variables_mut(),
            Self::SofiaGateway(ep) => ep.variables_mut(),
            Self::Loopback(ep) => ep.variables_mut(),
            Self::User(ep) => ep.variables_mut(),
            Self::SofiaContact(ep) => ep.variables_mut(),
            Self::GroupCall(ep) => ep.variables_mut(),
            Self::Error(ep) => ep.variables_mut(),
            Self::PortAudio(ep) | Self::PulseAudio(ep) | Self::Alsa(ep) => ep.variables_mut(),
        }
    }
    fn set_variables(&mut self, vars: Option<Variables>) {
        match self {
            Self::Sofia(ep) => ep.set_variables(vars),
            Self::SofiaGateway(ep) => ep.set_variables(vars),
            Self::Loopback(ep) => ep.set_variables(vars),
            Self::User(ep) => ep.set_variables(vars),
            Self::SofiaContact(ep) => ep.set_variables(vars),
            Self::GroupCall(ep) => ep.set_variables(vars),
            Self::Error(ep) => ep.set_variables(vars),
            Self::PortAudio(ep) | Self::PulseAudio(ep) | Self::Alsa(ep) => ep.set_variables(vars),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::variables::VariablesType;

    // --- extract_variables depth-aware bracket matching ---

    #[test]
    fn extract_variables_nested_angle_brackets() {
        let (vars, rest) = extract_variables(
            "<sip_h_Call-Info=<url>>sofia/gw/x",
            DialStringCarrier::EslApi,
        )
        .unwrap();
        assert_eq!(rest, "sofia/gw/x");
        assert!(vars.is_some());
    }

    #[test]
    fn extract_variables_nested_curly_brackets() {
        let (vars, rest) =
            extract_variables("{a={b}}sofia/internal/1000", DialStringCarrier::EslApi).unwrap();
        assert_eq!(rest, "sofia/internal/1000");
        assert!(vars.is_some());
    }

    #[test]
    fn extract_variables_unclosed_returns_error() {
        let result = extract_variables("{a=b", DialStringCarrier::EslApi);
        assert!(result.is_err());
    }

    // --- Endpoint enum FromStr dispatch ---

    #[test]
    fn endpoint_from_str_sofia() {
        let ep: Endpoint = "sofia/internal/1000@example.com"
            .parse()
            .unwrap();
        assert!(matches!(ep, Endpoint::Sofia(_)));
    }

    #[test]
    fn endpoint_from_str_sofia_gateway() {
        let ep: Endpoint = "sofia/gateway/my_gw/1234"
            .parse()
            .unwrap();
        assert!(matches!(ep, Endpoint::SofiaGateway(_)));
    }

    #[test]
    fn endpoint_from_str_loopback() {
        let ep: Endpoint = "loopback/9199/test"
            .parse()
            .unwrap();
        assert!(matches!(ep, Endpoint::Loopback(_)));
    }

    #[test]
    fn endpoint_from_str_user() {
        let ep: Endpoint = "user/1000@example.com"
            .parse()
            .unwrap();
        assert!(matches!(ep, Endpoint::User(_)));
    }

    #[test]
    fn endpoint_from_str_sofia_contact() {
        let ep: Endpoint = "${sofia_contact(1000@example.com)}"
            .parse()
            .unwrap();
        assert!(matches!(ep, Endpoint::SofiaContact(_)));
    }

    #[test]
    fn endpoint_from_str_group_call() {
        let ep: Endpoint = "${group_call(support@example.com+A)}"
            .parse()
            .unwrap();
        assert!(matches!(ep, Endpoint::GroupCall(_)));
    }

    #[test]
    fn endpoint_from_str_error() {
        let ep: Endpoint = "error/USER_BUSY"
            .parse()
            .unwrap();
        assert!(matches!(ep, Endpoint::Error(_)));
        assert!("error/user_busy"
            .parse::<Endpoint>()
            .is_err());
    }

    #[test]
    fn endpoint_from_str_unknown_errors() {
        let result = "verto/1234".parse::<Endpoint>();
        assert!(result.is_err());
    }

    #[test]
    fn endpoint_from_str_with_variables() {
        let ep: Endpoint = "{timeout=30}sofia/internal/1000@example.com"
            .parse()
            .unwrap();
        if let Endpoint::Sofia(inner) = &ep {
            assert_eq!(inner.profile, "internal");
            assert!(inner
                .variables
                .is_some());
        } else {
            panic!("expected Sofia variant");
        }
    }

    // --- Display delegation ---

    #[test]
    fn endpoint_display_delegates_to_inner() {
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "1000@example.com".into(),
            variables: None,
        });
        assert_eq!(ep.to_string(), "sofia/internal/1000@example.com");
    }

    // --- DialString trait ---

    #[test]
    fn dial_string_variables_returns_some() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("k", "v");
        let ep = SofiaEndpoint {
            profile: "internal".into(),
            destination: "1000".into(),
            variables: Some(vars),
        };
        assert!(ep
            .variables()
            .is_some());
        assert_eq!(
            ep.variables()
                .unwrap()
                .get("k"),
            Some("v")
        );
    }

    #[test]
    fn dial_string_variables_returns_none() {
        let ep = SofiaEndpoint {
            profile: "internal".into(),
            destination: "1000".into(),
            variables: None,
        };
        assert!(ep
            .variables()
            .is_none());
    }

    #[test]
    fn dial_string_set_variables() {
        let mut ep = SofiaEndpoint {
            profile: "internal".into(),
            destination: "1000".into(),
            variables: None,
        };
        let mut vars = Variables::new(VariablesType::Channel);
        vars.insert("k", "v");
        ep.set_variables(Some(vars));
        assert!(ep
            .variables()
            .is_some());
    }

    #[test]
    fn dial_string_error_endpoint_no_variables() {
        let ep = ErrorEndpoint::new(crate::channel::HangupCause::UserBusy);
        assert!(ep
            .variables()
            .is_none());
    }

    #[test]
    fn dial_string_on_endpoint_enum() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("k", "v");
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "1000".into(),
            variables: Some(vars),
        });
        assert!(ep
            .variables()
            .is_some());
    }

    // --- Serde: Endpoint enum ---

    #[test]
    fn serde_endpoint_enum_sofia() {
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "1000@example.com".into(),
            variables: None,
        });
        let json = serde_json::to_string(&ep).unwrap();
        assert!(json.contains("\"sofia\""));
        let parsed: Endpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn serde_endpoint_enum_sofia_gateway() {
        let ep = Endpoint::SofiaGateway(SofiaGateway {
            gateway: "gw1".into(),
            destination: "1234".into(),
            profile: None,
            variables: None,
        });
        let json = serde_json::to_string(&ep).unwrap();
        assert!(json.contains("\"sofia_gateway\""));
        let parsed: Endpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn serde_endpoint_enum_loopback() {
        let ep = Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("default"));
        let json = serde_json::to_string(&ep).unwrap();
        assert!(json.contains("\"loopback\""));
        let parsed: Endpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn serde_endpoint_enum_user() {
        let ep = Endpoint::User(UserEndpoint {
            name: "bob".into(),
            domain: Some("example.com".into()),
            variables: None,
        });
        let json = serde_json::to_string(&ep).unwrap();
        assert!(json.contains("\"user\""));
        let parsed: Endpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn serde_endpoint_enum_sofia_contact() {
        let ep = Endpoint::SofiaContact(SofiaContact {
            user: "1000".into(),
            domain: "example.com".into(),
            profile: None,
            variables: None,
        });
        let json = serde_json::to_string(&ep).unwrap();
        assert!(json.contains("\"sofia_contact\""));
        let parsed: Endpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn serde_endpoint_enum_group_call() {
        let ep = Endpoint::GroupCall(GroupCall {
            group: "support".into(),
            domain: "example.com".into(),
            order: Some(GroupCallOrder::All),
            variables: None,
        });
        let json = serde_json::to_string(&ep).unwrap();
        assert!(json.contains("\"group_call\""));
        let parsed: Endpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn serde_endpoint_enum_error() {
        let ep = Endpoint::Error(ErrorEndpoint::new(crate::channel::HangupCause::UserBusy));
        let json = serde_json::to_string(&ep).unwrap();
        assert!(json.contains("\"error\""));
        let parsed: Endpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn serde_endpoint_skips_none_variables() {
        let ep = SofiaEndpoint {
            profile: "internal".into(),
            destination: "1000".into(),
            variables: None,
        };
        let json = serde_json::to_string(&ep).unwrap();
        assert!(!json.contains("variables"));
    }

    #[test]
    fn serde_endpoint_skips_none_profile() {
        let ep = SofiaGateway {
            gateway: "gw".into(),
            destination: "1234".into(),
            profile: None,
            variables: None,
        };
        let json = serde_json::to_string(&ep).unwrap();
        assert!(!json.contains("profile"));
    }

    // --- Audio endpoints through Endpoint enum ---

    #[test]
    fn portaudio_display() {
        let ep = AudioEndpoint {
            destination: Some("auto_answer".into()),
            variables: None,
        };
        let endpoint = Endpoint::PortAudio(ep);
        assert_eq!(endpoint.to_string(), "portaudio/auto_answer");
    }

    #[test]
    fn portaudio_bare_display() {
        let ep = AudioEndpoint {
            destination: None,
            variables: None,
        };
        let endpoint = Endpoint::PortAudio(ep);
        assert_eq!(endpoint.to_string(), "portaudio");
    }

    #[test]
    fn portaudio_from_str() {
        let ep: Endpoint = "portaudio/auto_answer"
            .parse()
            .unwrap();
        if let Endpoint::PortAudio(audio) = ep {
            assert_eq!(
                audio
                    .destination
                    .as_deref(),
                Some("auto_answer")
            );
        } else {
            panic!("expected PortAudio");
        }
    }

    #[test]
    fn portaudio_bare_from_str() {
        let ep: Endpoint = "portaudio"
            .parse()
            .unwrap();
        if let Endpoint::PortAudio(audio) = ep {
            assert!(audio
                .destination
                .is_none());
        } else {
            panic!("expected PortAudio");
        }
    }

    #[test]
    fn portaudio_round_trip() {
        let input = "portaudio/auto_answer";
        let ep: Endpoint = input
            .parse()
            .unwrap();
        assert_eq!(ep.to_string(), input);
    }

    #[test]
    fn portaudio_bare_round_trip() {
        let input = "portaudio";
        let ep: Endpoint = input
            .parse()
            .unwrap();
        assert_eq!(ep.to_string(), input);
    }

    #[test]
    fn portaudio_with_variables() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("codec", "PCMU");
        let ep = Endpoint::PortAudio(AudioEndpoint {
            destination: Some("auto_answer".into()),
            variables: Some(vars),
        });
        assert_eq!(ep.to_string(), "{codec=PCMU}portaudio/auto_answer");
        let parsed: Endpoint = ep
            .to_string()
            .parse()
            .unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn pulseaudio_display() {
        let ep = Endpoint::PulseAudio(AudioEndpoint {
            destination: Some("auto_answer".into()),
            variables: None,
        });
        assert_eq!(ep.to_string(), "pulseaudio/auto_answer");
    }

    #[test]
    fn pulseaudio_from_str() {
        let ep: Endpoint = "pulseaudio/auto_answer"
            .parse()
            .unwrap();
        assert!(matches!(ep, Endpoint::PulseAudio(_)));
    }

    #[test]
    fn pulseaudio_round_trip() {
        let input = "pulseaudio/auto_answer";
        let ep: Endpoint = input
            .parse()
            .unwrap();
        assert_eq!(ep.to_string(), input);
    }

    #[test]
    fn alsa_display() {
        let ep = Endpoint::Alsa(AudioEndpoint {
            destination: Some("auto_answer".into()),
            variables: None,
        });
        assert_eq!(ep.to_string(), "alsa/auto_answer");
    }

    #[test]
    fn alsa_from_str() {
        let ep: Endpoint = "alsa/auto_answer"
            .parse()
            .unwrap();
        assert!(matches!(ep, Endpoint::Alsa(_)));
    }

    #[test]
    fn alsa_bare_round_trip() {
        let input = "alsa";
        let ep: Endpoint = input
            .parse()
            .unwrap();
        assert_eq!(ep.to_string(), input);
    }

    #[test]
    fn serde_portaudio() {
        let ep = Endpoint::PortAudio(AudioEndpoint {
            destination: Some("auto_answer".into()),
            variables: None,
        });
        let json = serde_json::to_string(&ep).unwrap();
        assert!(json.contains("\"portaudio\""));
        let parsed: Endpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn serde_pulseaudio() {
        let ep = Endpoint::PulseAudio(AudioEndpoint {
            destination: Some("auto_answer".into()),
            variables: None,
        });
        let json = serde_json::to_string(&ep).unwrap();
        assert!(json.contains("\"pulseaudio\""));
        let parsed: Endpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn serde_alsa() {
        let ep = Endpoint::Alsa(AudioEndpoint {
            destination: None,
            variables: None,
        });
        let json = serde_json::to_string(&ep).unwrap();
        assert!(json.contains("\"alsa\""));
        let parsed: Endpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ep);
    }

    // --- From impls ---

    #[test]
    fn from_sofia_endpoint() {
        let inner = SofiaEndpoint {
            profile: "internal".into(),
            destination: "1000@example.com".into(),
            variables: None,
        };
        let ep: Endpoint = inner
            .clone()
            .into();
        assert_eq!(ep, Endpoint::Sofia(inner));
    }

    #[test]
    fn from_sofia_gateway() {
        let inner = SofiaGateway {
            gateway: "gw1".into(),
            destination: "1234".into(),
            profile: None,
            variables: None,
        };
        let ep: Endpoint = inner
            .clone()
            .into();
        assert_eq!(ep, Endpoint::SofiaGateway(inner));
    }

    #[test]
    fn from_loopback_endpoint() {
        let inner = LoopbackEndpoint::new("9199").with_context("default");
        let ep: Endpoint = inner
            .clone()
            .into();
        assert_eq!(ep, Endpoint::Loopback(inner));
    }

    #[test]
    fn from_user_endpoint() {
        let inner = UserEndpoint {
            name: "bob".into(),
            domain: Some("example.com".into()),
            variables: None,
        };
        let ep: Endpoint = inner
            .clone()
            .into();
        assert_eq!(ep, Endpoint::User(inner));
    }

    #[test]
    fn from_sofia_contact() {
        let inner = SofiaContact {
            user: "1000".into(),
            domain: "example.com".into(),
            profile: None,
            variables: None,
        };
        let ep: Endpoint = inner
            .clone()
            .into();
        assert_eq!(ep, Endpoint::SofiaContact(inner));
    }

    #[test]
    fn from_group_call() {
        let inner = GroupCall::new("support", "example.com").with_order(GroupCallOrder::All);
        let ep: Endpoint = inner
            .clone()
            .into();
        assert_eq!(ep, Endpoint::GroupCall(inner));
    }

    #[test]
    fn from_error_endpoint() {
        let inner = ErrorEndpoint::new(crate::channel::HangupCause::UserBusy);
        let ep: Endpoint = inner.into();
        assert_eq!(ep, Endpoint::Error(inner));
    }
}
