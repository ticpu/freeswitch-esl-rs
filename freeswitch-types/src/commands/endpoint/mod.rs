//! FreeSWITCH endpoint types for originate and bridge dial strings.
//!
//! Each endpoint type corresponds to a real FreeSWITCH endpoint module
//! or runtime expression. Concrete structs implement the [`DialString`]
//! trait independently; the [`Endpoint`] enum wraps them for
//! serialization and polymorphic storage.

/// Emit the [`DialString`] impl and the `with_variables` builder for an endpoint
/// struct holding `variables: Option<Variables>`. Given a body, also emit the
/// carrier-aware `write_for` — variable-block prologue included — and the
/// [`Display`](std::fmt::Display) that renders it for the default carrier.
macro_rules! impl_dial_string_with_variables {
    ($ty:ty) => {
        impl $crate::commands::endpoint::DialString for $ty {
            fn variables(&self) -> Option<&$crate::commands::variables::Variables> {
                self.variables
                    .as_ref()
            }
            fn variables_mut(&mut self) -> Option<&mut $crate::commands::variables::Variables> {
                self.variables
                    .as_mut()
            }
            fn set_variables(&mut self, vars: Option<$crate::commands::variables::Variables>) {
                self.variables = vars;
            }
        }

        impl $ty {
            /// Set per-channel variables.
            pub fn with_variables(
                mut self,
                variables: $crate::commands::variables::Variables,
            ) -> Self {
                self.variables = Some(variables);
                self
            }
        }
    };
    ($ty:ty, |$this:ident, $f:ident| $body:expr) => {
        impl_dial_string_with_variables!($ty);

        impl $ty {
            pub(super) fn write_for(
                &self,
                $f: &mut ::std::fmt::Formatter<'_>,
                carrier: $crate::commands::variables::DialStringCarrier,
            ) -> ::std::fmt::Result {
                $crate::commands::endpoint::write_variables($f, &self.variables, carrier)?;
                let $this = self;
                $body
            }
        }

        impl ::std::fmt::Display for $ty {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                self.write_for(f, $crate::commands::variables::DialStringCarrier::EslApi)
            }
        }
    };
}

/// Dispatch a [`DialString`] method to whichever concrete endpoint the
/// [`Endpoint`] variant wraps.
macro_rules! forward_to_variant {
    ($self:ident, $method:ident $(, $arg:expr)?) => {
        match $self {
            Self::Sofia(ep) => ep.$method($($arg)?),
            Self::SofiaGateway(ep) => ep.$method($($arg)?),
            Self::Loopback(ep) => ep.$method($($arg)?),
            Self::User(ep) => ep.$method($($arg)?),
            Self::SofiaContact(ep) => ep.$method($($arg)?),
            Self::GroupCall(ep) => ep.$method($($arg)?),
            Self::Error(ep) => ep.$method($($arg)?),
            Self::PortAudio(ep) | Self::PulseAudio(ep) | Self::Alsa(ep) => ep.$method($($arg)?),
        }
    };
}

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
use super::variables::{DialStringCarrier, Variables, VariablesType};

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

/// Strip the leading variable block then a fixed endpoint prefix. A prefix that
/// does not itself end a path segment (`alsa`, not `sofia/`) must be followed by
/// `/` or by nothing, or `alsafoo/bar` strips to a bare `alsa`.
pub(super) fn strip_endpoint_prefix<'a>(
    s: &'a str,
    prefix: &str,
    kind: &str,
    carrier: DialStringCarrier,
) -> Result<(Option<Variables>, &'a str), OriginateError> {
    let (variables, uri) = extract_variables(s, carrier)?;
    let rest = uri
        .strip_prefix(prefix)
        .filter(|rest| prefix.ends_with('/') || rest.is_empty() || rest.starts_with('/'))
        .ok_or_else(|| OriginateError::ParseError(format!("not a {} endpoint", kind)))?;
    Ok((variables, rest))
}

/// Every scope an endpoint may carry directly ahead of its module name.
const ANY_SCOPE: &[VariablesType] = &[
    VariablesType::Default,
    VariablesType::Enterprise,
    VariablesType::Channel,
];

/// Extract a leading variable block (`{...}`, `[...]`, or `<...>`) from a
/// dial string, returning the parsed variables and the remaining URI portion.
fn extract_variables(
    s: &str,
    carrier: DialStringCarrier,
) -> Result<(Option<Variables>, &str), OriginateError> {
    extract_scoped_variables(s, carrier, ANY_SCOPE)
}

/// Extract a leading variable block whose brackets name one of `scopes`, so a
/// caller that owns only some of them leaves the rest to whoever follows.
///
/// Uses depth-aware bracket matching so nested brackets in values (e.g.
/// `<sip_h_Call-Info=<url>>`) don't cause premature closure.
pub(super) fn extract_scoped_variables<'a>(
    s: &'a str,
    carrier: DialStringCarrier,
    scopes: &[VariablesType],
) -> Result<(Option<Variables>, &'a str), OriginateError> {
    let first = s
        .as_bytes()
        .first()
        .copied();
    let Some((open, close_ch)) = scopes
        .iter()
        .map(|scope| scope.delimiters())
        .find(|(open, _)| first == Some(*open as u8))
    else {
        return Ok((None, s));
    };
    let close = find_matching_bracket(s, open, close_ch)
        .ok_or_else(|| OriginateError::ParseError(format!("unclosed {} in dial string", open)))?;
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
        let mut endpoint = Self::parse_bare(rest)?;
        if variables.is_some() {
            endpoint.set_variables(variables);
            if endpoint
                .variables()
                .is_none()
            {
                return Err(OriginateError::VariablesNotSupported(endpoint.kind()));
            }
        }
        Ok(endpoint)
    }

    /// The module name this variant renders, for diagnostics.
    fn kind(&self) -> &'static str {
        match self {
            Self::Sofia(_) => "sofia",
            Self::SofiaGateway(_) => "sofia gateway",
            Self::Loopback(_) => "loopback",
            Self::User(_) => "user",
            Self::SofiaContact(_) => "sofia_contact",
            Self::GroupCall(_) => "group_call",
            Self::Error(_) => "error",
            Self::PortAudio(_) => "portaudio",
            Self::PulseAudio(_) => "pulseaudio",
            Self::Alsa(_) => "alsa",
        }
    }

    /// Dispatch on the module prefix of a dial string whose variable block has
    /// already been taken off.
    fn parse_bare(uri: &str) -> Result<Self, OriginateError> {
        if uri.starts_with("${sofia_contact(") {
            Ok(Self::SofiaContact(uri.parse()?))
        } else if uri.starts_with("${group_call(") {
            Ok(Self::GroupCall(uri.parse()?))
        } else if uri.starts_with("error/") {
            Ok(Self::Error(uri.parse()?))
        } else if uri.starts_with("loopback/") {
            Ok(Self::Loopback(uri.parse()?))
        } else if uri.starts_with("sofia/gateway/") {
            Ok(Self::SofiaGateway(uri.parse()?))
        } else if uri.starts_with("sofia/") {
            Ok(Self::Sofia(uri.parse()?))
        } else if uri.starts_with("user/") {
            Ok(Self::User(uri.parse()?))
        } else if uri.starts_with("portaudio") {
            Ok(Self::PortAudio(AudioEndpoint::parse_with_prefix(
                uri,
                "portaudio",
            )?))
        } else if uri.starts_with("pulseaudio") {
            Ok(Self::PulseAudio(AudioEndpoint::parse_with_prefix(
                uri,
                "pulseaudio",
            )?))
        } else if uri.starts_with("alsa") {
            Ok(Self::Alsa(AudioEndpoint::parse_with_prefix(uri, "alsa")?))
        } else {
            Err(OriginateError::UnknownEndpointType(uri.to_string()))
        }
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
        Self::parse_for(s, DialStringCarrier::EslApi)
    }
}

// ---------------------------------------------------------------------------
// DialString impls
// ---------------------------------------------------------------------------

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
        forward_to_variant!(self, variables)
    }
    fn variables_mut(&mut self) -> Option<&mut Variables> {
        forward_to_variant!(self, variables_mut)
    }
    fn set_variables(&mut self, vars: Option<Variables>) {
        forward_to_variant!(self, set_variables, vars)
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
    }

    /// `ErrorEndpoint` has nowhere to keep a block, so accepting one loses
    /// every variable it named without a word to the caller.
    #[test]
    fn a_block_on_an_endpoint_that_cannot_hold_one_is_refused() {
        for carrier in [DialStringCarrier::EslApi, DialStringCarrier::Dialplan] {
            let msg = Endpoint::parse_for("{a=b}error/USER_BUSY", carrier)
                .expect_err(&format!("accepted at {carrier:?}"))
                .to_string();
            assert!(msg.contains("error"), "does not name the type: {msg}");
        }
        assert!("{a=b}error/USER_BUSY"
            .parse::<Endpoint>()
            .is_err());
    }

    /// The two entry points have to agree: `from_str` is `parse_for` at the
    /// default carrier, not a second dispatch with its own rules.
    #[test]
    fn from_str_matches_parse_for_at_the_default_carrier() {
        for input in [
            "sofia/internal/1000@example.com",
            "{a=b}sofia/internal/1000@example.com",
            "<a=b>loopback/9199/default",
            "[a=b]user/bob@example.com",
            "{a=b}error/USER_BUSY",
            "verto/1234",
        ] {
            assert_eq!(
                input
                    .parse::<Endpoint>()
                    .is_ok(),
                Endpoint::parse_for(input, DialStringCarrier::EslApi).is_ok(),
                "{input}"
            );
        }
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

    /// One case per variant: the externally tagged name is the config key a
    /// deployment writes, so a rename is a break and has to show up here.
    #[test]
    fn serde_endpoint_enum_tags_round_trip() {
        let cases: [(&str, Endpoint); 7] = [
            (
                "sofia",
                SofiaEndpoint::new("internal", "1000@example.com").into(),
            ),
            ("sofia_gateway", SofiaGateway::new("gw1", "1234").into()),
            (
                "loopback",
                LoopbackEndpoint::new("9199")
                    .with_context("default")
                    .into(),
            ),
            (
                "user",
                UserEndpoint::new("bob")
                    .with_domain("example.com")
                    .into(),
            ),
            (
                "sofia_contact",
                SofiaContact::new("1000", "example.com").into(),
            ),
            (
                "group_call",
                GroupCall::new("support", "example.com")
                    .with_order(GroupCallOrder::All)
                    .into(),
            ),
            (
                "error",
                ErrorEndpoint::new(crate::channel::HangupCause::UserBusy).into(),
            ),
        ];
        for (tag, ep) in cases {
            let json = serde_json::to_string(&ep).unwrap();
            assert!(json.contains(&format!("\"{tag}\"")), "{tag}: {json}");
            assert_eq!(serde_json::from_str::<Endpoint>(&json).unwrap(), ep);
        }
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

    /// The three audio modules share one struct and differ only in the prefix
    /// their variant supplies, so each row has to name its own module.
    #[test]
    fn audio_endpoints_render_and_parse_per_module() {
        type Variant = fn(AudioEndpoint) -> Endpoint;
        let cases: [(Variant, &str, &str); 6] = [
            (Endpoint::PortAudio, "portaudio", "portaudio/auto_answer"),
            (Endpoint::PortAudio, "portaudio", "portaudio"),
            (Endpoint::PulseAudio, "pulseaudio", "pulseaudio/auto_answer"),
            (Endpoint::PulseAudio, "pulseaudio", "pulseaudio"),
            (Endpoint::Alsa, "alsa", "alsa/auto_answer"),
            (Endpoint::Alsa, "alsa", "alsa"),
        ];
        for (variant, module, wire) in cases {
            let destination = wire
                .strip_prefix(module)
                .and_then(|rest| rest.strip_prefix('/'))
                .map(str::to_string);
            let ep = variant(AudioEndpoint {
                destination: destination.clone(),
                variables: None,
            });
            assert_eq!(ep.to_string(), wire);

            let parsed: Endpoint = wire
                .parse()
                .unwrap();
            assert_eq!(parsed, ep, "{wire}");
            assert!(
                parsed
                    .variables()
                    .is_none(),
                "{wire}"
            );

            let json = serde_json::to_string(&ep).unwrap();
            assert!(json.contains(&format!("\"{module}\"")), "{wire}: {json}");
            assert_eq!(serde_json::from_str::<Endpoint>(&json).unwrap(), ep);
        }
    }

    #[test]
    fn audio_endpoint_carries_variables() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("codec", "PCMU");
        let ep = Endpoint::PortAudio(
            AudioEndpoint::new()
                .with_destination("auto_answer")
                .with_variables(vars),
        );
        assert_eq!(ep.to_string(), "{codec=PCMU}portaudio/auto_answer");
        assert_eq!(
            ep.to_string()
                .parse::<Endpoint>()
                .unwrap(),
            ep
        );
    }

    /// A quoted value is escaped one level shallower for a dialplan carrier, so
    /// a renderer and a parser that disagree on the carrier hand back a
    /// different value than was put in — silently, for every endpoint type.
    #[test]
    fn every_endpoint_type_round_trips_at_the_dialplan_carrier() {
        let mut vars = Variables::new(VariablesType::Channel);
        vars.insert("cid", "it's");
        vars.insert("other", "don't");

        let with_vars = |mut ep: Endpoint| {
            ep.set_variables(Some(vars.clone()));
            ep
        };
        let cases: [Endpoint; 10] = [
            with_vars(SofiaEndpoint::new("internal", "1000@example.com").into()),
            with_vars(
                SofiaGateway::new("gw", "1234")
                    .with_profile("external")
                    .into(),
            ),
            with_vars(
                LoopbackEndpoint::new("9199")
                    .with_context("default")
                    .into(),
            ),
            with_vars(
                UserEndpoint::new("bob")
                    .with_domain("example.com")
                    .into(),
            ),
            with_vars(
                SofiaContact::new("1000", "example.com")
                    .with_profile("*")
                    .into(),
            ),
            with_vars(
                GroupCall::new("support", "example.com")
                    .with_order(GroupCallOrder::All)
                    .into(),
            ),
            ErrorEndpoint::new(crate::channel::HangupCause::UserBusy).into(),
            with_vars(Endpoint::PortAudio(
                AudioEndpoint::new().with_destination("auto_answer"),
            )),
            with_vars(Endpoint::PulseAudio(AudioEndpoint::new())),
            with_vars(Endpoint::Alsa(
                AudioEndpoint::new().with_destination("auto_answer"),
            )),
        ];

        for ep in cases {
            let rendered = ep
                .display_for(DialStringCarrier::Dialplan)
                .to_string();
            let back = Endpoint::parse_for(&rendered, DialStringCarrier::Dialplan)
                .unwrap_or_else(|e| panic!("{rendered} failed to parse: {e}"));
            assert_eq!(back, ep, "rendered {rendered}");
        }
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
