//! Originate command builder with endpoint configuration, variable scoping,
//! and automatic quoting for socket application arguments.

use std::fmt;
use std::num::ParseIntError;
use std::str::FromStr;
use std::time::Duration;

use super::endpoint::ParseGroupCallOrderError;
use super::{originate_quote, originate_split, originate_unquote};
use crate::channel::ParseHangupCauseError;

pub use super::variables::{Variables, VariablesType};

/// FreeSWITCH keyword for omitted positional arguments.
///
/// `switch_separate_string` converts `"undef"` to NULL, making it the
/// canonical placeholder when a later positional arg forces earlier ones
/// to be present on the wire.
const UNDEF: &str = "undef";

/// The context FreeSWITCH itself falls back to, emitted when a later
/// positional argument forces the slot to be present.
const DEFAULT_CONTEXT: &str = "default";

/// FreeSWITCH dialplan type for originate commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[non_exhaustive]
pub enum DialplanType {
    /// Inline dialplan: applications execute directly without XML lookup.
    Inline,
    /// XML dialplan: route through the XML dialplan engine.
    Xml,
}

impl fmt::Display for DialplanType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inline => f.write_str("inline"),
            Self::Xml => f.write_str("XML"),
        }
    }
}

parse_error! { ParseDialplanTypeError("dialplan type"); }

impl FromStr for DialplanType {
    type Err = ParseDialplanTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("inline") {
            Ok(Self::Inline)
        } else if s.eq_ignore_ascii_case("xml") {
            Ok(Self::Xml)
        } else {
            Err(ParseDialplanTypeError(s.to_string()))
        }
    }
}

pub use super::endpoint::Endpoint;

/// A single dialplan application with optional arguments.
///
/// Formats differently depending on [`DialplanType`]:
/// - Inline: `name` or `name:args`
/// - XML: `&name(args)`
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Application {
    name: String,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    args: Option<String>,
}

impl Application {
    /// Create an application with optional arguments.
    pub fn new(name: impl Into<String>, args: Option<impl Into<String>>) -> Self {
        Self {
            name: name.into(),
            args: args.map(|a| a.into()),
        }
    }

    /// Create an application with no arguments.
    pub fn simple(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            args: None,
        }
    }

    /// Park the channel (hold in place without bridging).
    pub fn park() -> Self {
        Self::simple("park")
    }

    /// Application name (e.g. `park`, `conference`, `socket`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Application arguments, if any.
    pub fn args(&self) -> Option<&str> {
        self.args
            .as_deref()
    }

    /// Mutable reference to the application name.
    pub fn name_mut(&mut self) -> &mut String {
        &mut self.name
    }

    /// Mutable reference to the application arguments.
    pub fn args_mut(&mut self) -> &mut Option<String> {
        &mut self.args
    }

    /// Format as inline (`name:args`) or XML (`&name(args)`) syntax.
    pub fn to_string_with_dialplan(&self, dialplan: &DialplanType) -> String {
        match dialplan {
            DialplanType::Inline => match &self.args {
                Some(args) => format!("{}:{}", self.name, args),
                None => self
                    .name
                    .clone(),
            },
            DialplanType::Xml => {
                let args = self
                    .args
                    .as_deref()
                    .unwrap_or("");
                format!("&{}({})", self.name, args)
            }
        }
    }
}

/// The target of an originate command: either a dialplan extension or
/// application(s) to execute directly.
///
/// FreeSWITCH syntax: `originate <endpoint> <target> [dialplan] ...`
/// where `<target>` is either a bare extension string (routes through
/// the dialplan engine) or `&app(args)` / `app:args` (executes inline).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum OriginateTarget {
    /// Route through the dialplan engine to this extension.
    Extension(String),
    /// Single application for XML dialplan: `&app(args)`.
    Application(Application),
    /// One or more applications for inline dialplan: `app:args,app:args`.
    InlineApplications(Vec<Application>),
}

impl From<Application> for OriginateTarget {
    fn from(app: Application) -> Self {
        Self::Application(app)
    }
}

impl From<Vec<Application>> for OriginateTarget {
    fn from(apps: Vec<Application>) -> Self {
        Self::InlineApplications(apps)
    }
}

/// Originate command builder: `originate <endpoint> <target> [dialplan] [context] [cid_name] [cid_num] [timeout]`.
///
/// Constructed via [`Originate::extension`], [`Originate::application`], or
/// [`Originate::inline`]. Invalid states (Extension + Inline dialplan, empty
/// inline apps) are rejected at construction time rather than at `Display`.
///
/// Optional fields are set via consuming-self chaining methods:
///
/// ```
/// # use std::time::Duration;
/// # use freeswitch_types::commands::*;
/// let cmd = Originate::application(
///     Endpoint::Loopback(LoopbackEndpoint::new("9196").with_context("default")),
///     Application::simple("park"),
/// )
/// .cid_name("Alice")
/// .cid_num("5551234")
/// .timeout(Duration::from_secs(30));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Originate {
    endpoint: Endpoint,
    target: OriginateTarget,
    dialplan: Option<DialplanType>,
    context: Option<String>,
    cid_name: Option<String>,
    cid_num: Option<String>,
    timeout: Option<Duration>,
}

#[cfg(feature = "serde")]
mod serde_support {
    use super::*;

    /// Intermediate type for serde, mirroring the old public-field layout.
    #[derive(serde::Serialize, serde::Deserialize)]
    pub(super) struct OriginateRaw {
        pub endpoint: Endpoint,
        #[serde(flatten)]
        pub target: OriginateTarget,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub dialplan: Option<DialplanType>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub context: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub cid_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub cid_num: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub timeout_secs: Option<u64>,
    }

    impl TryFrom<OriginateRaw> for Originate {
        type Error = OriginateError;

        fn try_from(raw: OriginateRaw) -> Result<Self, Self::Error> {
            if matches!(raw.target, OriginateTarget::Extension(_))
                && matches!(raw.dialplan, Some(DialplanType::Inline))
            {
                return Err(OriginateError::ExtensionWithInlineDialplan);
            }
            if let OriginateTarget::InlineApplications(ref apps) = raw.target {
                if apps.is_empty() {
                    return Err(OriginateError::EmptyInlineApplications);
                }
            }
            Ok(Self {
                endpoint: raw.endpoint,
                target: raw.target,
                dialplan: raw.dialplan,
                context: raw.context,
                cid_name: raw.cid_name,
                cid_num: raw.cid_num,
                timeout: raw
                    .timeout_secs
                    .map(Duration::from_secs),
            })
        }
    }

    impl From<Originate> for OriginateRaw {
        fn from(o: Originate) -> Self {
            Self {
                endpoint: o.endpoint,
                target: o.target,
                dialplan: o.dialplan,
                context: o.context,
                cid_name: o.cid_name,
                cid_num: o.cid_num,
                timeout_secs: o
                    .timeout
                    .map(|d| d.as_secs()),
            }
        }
    }

    impl serde::Serialize for Originate {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            OriginateRaw::from(self.clone()).serialize(serializer)
        }
    }

    impl<'de> serde::Deserialize<'de> for Originate {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let raw = OriginateRaw::deserialize(deserializer)?;
            Originate::try_from(raw).map_err(serde::de::Error::custom)
        }
    }
}

impl Originate {
    /// Route through the dialplan engine to an extension.
    pub fn extension(endpoint: Endpoint, extension: impl Into<String>) -> Self {
        Self {
            endpoint,
            target: OriginateTarget::Extension(extension.into()),
            dialplan: None,
            context: None,
            cid_name: None,
            cid_num: None,
            timeout: None,
        }
    }

    /// Execute a single XML-format application on the answered channel.
    pub fn application(endpoint: Endpoint, app: Application) -> Self {
        Self {
            endpoint,
            target: OriginateTarget::Application(app),
            dialplan: None,
            context: None,
            cid_name: None,
            cid_num: None,
            timeout: None,
        }
    }

    /// Execute inline applications on the answered channel.
    ///
    /// Returns `Err` if the iterator yields no applications.
    pub fn inline(
        endpoint: Endpoint,
        apps: impl IntoIterator<Item = Application>,
    ) -> Result<Self, OriginateError> {
        let apps: Vec<Application> = apps
            .into_iter()
            .collect();
        if apps.is_empty() {
            return Err(OriginateError::EmptyInlineApplications);
        }
        Ok(Self {
            endpoint,
            target: OriginateTarget::InlineApplications(apps),
            dialplan: None,
            context: None,
            cid_name: None,
            cid_num: None,
            timeout: None,
        })
    }

    /// Set the dialplan type.
    ///
    /// Returns `Err` if setting `Inline` on an `Extension` target.
    pub fn dialplan(mut self, dp: DialplanType) -> Result<Self, OriginateError> {
        if matches!(self.target, OriginateTarget::Extension(_)) && dp == DialplanType::Inline {
            return Err(OriginateError::ExtensionWithInlineDialplan);
        }
        self.dialplan = Some(dp);
        Ok(self)
    }

    /// Set the dialplan context.
    pub fn context(mut self, ctx: impl Into<String>) -> Self {
        self.context = Some(ctx.into());
        self
    }

    /// Set the caller ID name.
    pub fn cid_name(mut self, name: impl Into<String>) -> Self {
        self.cid_name = Some(name.into());
        self
    }

    /// Set the caller ID number.
    pub fn cid_num(mut self, num: impl Into<String>) -> Self {
        self.cid_num = Some(num.into());
        self
    }

    /// Set the originate timeout. Sub-second precision is truncated to whole
    /// seconds on the wire and in serde round-trips.
    pub fn timeout(mut self, duration: Duration) -> Self {
        self.timeout = Some(duration);
        self
    }

    /// The dial endpoint.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Mutable reference to the dial endpoint.
    pub fn endpoint_mut(&mut self) -> &mut Endpoint {
        &mut self.endpoint
    }

    /// The originate target (extension, application, or inline apps).
    pub fn target(&self) -> &OriginateTarget {
        &self.target
    }

    /// Mutable reference to the originate target.
    pub fn target_mut(&mut self) -> &mut OriginateTarget {
        &mut self.target
    }

    /// The dialplan type, if explicitly set.
    pub fn dialplan_type(&self) -> Option<&DialplanType> {
        self.dialplan
            .as_ref()
    }

    /// The dialplan context, if set.
    pub fn context_str(&self) -> Option<&str> {
        self.context
            .as_deref()
    }

    /// The caller ID name, if set.
    pub fn caller_id_name(&self) -> Option<&str> {
        self.cid_name
            .as_deref()
    }

    /// The caller ID number, if set.
    pub fn caller_id_number(&self) -> Option<&str> {
        self.cid_num
            .as_deref()
    }

    /// The timeout as a `Duration`, if set.
    pub fn timeout_duration(&self) -> Option<Duration> {
        self.timeout
    }

    /// The timeout in whole seconds, if set.
    pub fn timeout_seconds(&self) -> Option<u64> {
        self.timeout
            .map(|d| d.as_secs())
    }

    /// Override the dialplan type after construction.
    pub fn set_dialplan(&mut self, dp: Option<DialplanType>) {
        self.dialplan = dp;
    }

    /// Override the dialplan context after construction.
    pub fn set_context(&mut self, ctx: Option<impl Into<String>>) {
        self.context = ctx.map(|c| c.into());
    }

    /// Override the caller ID name after construction.
    pub fn set_cid_name(&mut self, name: Option<impl Into<String>>) {
        self.cid_name = name.map(|n| n.into());
    }

    /// Override the caller ID number after construction.
    pub fn set_cid_num(&mut self, num: Option<impl Into<String>>) {
        self.cid_num = num.map(|n| n.into());
    }

    /// Override the timeout after construction.
    pub fn set_timeout(&mut self, timeout: Option<Duration>) {
        self.timeout = timeout;
    }

    /// Write dialplan, context, cid_name, cid_num and timeout. FreeSWITCH reads
    /// them by position, so a later one present forces its placeholder onto
    /// every earlier slot.
    fn write_positional_tail(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let dialplan = match &self.target {
            OriginateTarget::InlineApplications(_) => Some(
                self.dialplan
                    .unwrap_or(DialplanType::Inline),
            ),
            _ => self.dialplan,
        };
        let timeout = self
            .timeout
            .is_some();
        let cid_num = timeout
            || self
                .cid_num
                .is_some();
        let cid_name = cid_num
            || self
                .cid_name
                .is_some();
        let context = cid_name
            || self
                .context
                .is_some();

        if context || dialplan.is_some() {
            write!(f, " {}", dialplan.unwrap_or(DialplanType::Xml))?;
        }
        if context {
            write!(
                f,
                " {}",
                self.context
                    .as_deref()
                    .unwrap_or(DEFAULT_CONTEXT)
            )?;
        }
        if cid_name {
            let name = self
                .cid_name
                .as_deref()
                .unwrap_or(UNDEF);
            write!(f, " {}", originate_quote(name))?;
        }
        if cid_num {
            let num = self
                .cid_num
                .as_deref()
                .unwrap_or(UNDEF);
            write!(f, " {}", originate_quote(num))?;
        }
        if let Some(timeout) = self.timeout {
            write!(f, " {}", timeout.as_secs())?;
        }
        Ok(())
    }
}

impl fmt::Display for Originate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let target_str = match &self.target {
            OriginateTarget::Extension(ext) => ext.clone(),
            OriginateTarget::Application(app) => app.to_string_with_dialplan(&DialplanType::Xml),
            OriginateTarget::InlineApplications(apps) => {
                // Constructor guarantees non-empty
                let parts: Vec<String> = apps
                    .iter()
                    .map(|a| a.to_string_with_dialplan(&DialplanType::Inline))
                    .collect();
                parts.join(",")
            }
        };

        write!(
            f,
            "originate {} {}",
            self.endpoint,
            originate_quote(&target_str)
        )?;

        self.write_positional_tail(f)
    }
}

impl FromStr for Originate {
    type Err = OriginateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s
            .strip_prefix("originate")
            .unwrap_or(s)
            .trim();
        let mut args = originate_split(s, ' ')?;

        if args.is_empty() {
            return Err(OriginateError::ParseError("empty originate".into()));
        }

        let endpoint_str = args.remove(0);
        let endpoint: Endpoint = endpoint_str.parse()?;

        if args.is_empty() {
            return Err(OriginateError::ParseError(
                "missing target in originate".into(),
            ));
        }

        let target_str = originate_unquote(&args.remove(0));

        // The slot is positional and optional, so a token that is neither
        // spelling is the context rather than a malformed dialplan.
        let dialplan = match args
            .first()
            .map(String::as_str)
        {
            Some(token) if token.eq_ignore_ascii_case("inline") => Some(DialplanType::Inline),
            Some(token) if token.eq_ignore_ascii_case("xml") => Some(DialplanType::Xml),
            _ => None,
        };
        if dialplan.is_some() {
            args.remove(0);
        }

        let target = super::parse_originate_target(&target_str, dialplan.as_ref())?;

        let context = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let cid_name = take_undef(&mut args);
        let cid_num = take_undef(&mut args);
        let timeout = if args.is_empty() {
            None
        } else {
            let value = args.remove(0);
            let secs = value
                .parse::<u64>()
                .map_err(|source| OriginateError::InvalidTimeout {
                    value: value.clone(),
                    source,
                })?;
            Some(Duration::from_secs(secs))
        };

        // Validate via constructors then set parsed fields directly (same module)
        let mut orig = match target {
            OriginateTarget::Extension(ref ext) => Self::extension(endpoint, ext.clone()),
            OriginateTarget::Application(ref app) => Self::application(endpoint, app.clone()),
            OriginateTarget::InlineApplications(ref apps) => Self::inline(endpoint, apps.clone())?,
        };
        orig.dialplan = dialplan;
        orig.context = context;
        orig.cid_name = cid_name;
        orig.cid_num = cid_num;
        orig.timeout = timeout;
        Ok(orig)
    }
}

/// Take the next positional argument, undoing the quoting [`originate_quote`]
/// applies to the same slot and reading FreeSWITCH's `undef` placeholder as the
/// absent value it stands for.
fn take_undef(args: &mut Vec<String>) -> Option<String> {
    if args.is_empty() {
        return None;
    }
    let value = originate_unquote(&args.remove(0));
    (!value.eq_ignore_ascii_case(UNDEF)).then_some(value)
}

/// Errors from originate command parsing or construction.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OriginateError {
    /// A single-quoted token was never closed.
    UnclosedQuote(String),
    /// General parse failure with a description.
    ParseError(String),
    /// Inline originate requires at least one application.
    EmptyInlineApplications,
    /// Extension target cannot use inline dialplan.
    ExtensionWithInlineDialplan,
    /// A dial string carried a variable block for an endpoint type that has
    /// nowhere to keep it, such as `error/`.
    VariablesNotSupported,
    /// The timeout argument is not a whole number of seconds.
    InvalidTimeout {
        /// The rejected token.
        value: String,
        /// Why it is not a number.
        source: ParseIntError,
    },
    /// An `error/` endpoint named a cause this crate does not know.
    UnknownHangupCause {
        /// The rejected token.
        value: String,
        /// The hangup-cause parse failure.
        source: ParseHangupCauseError,
    },
    /// A `group_call` expression carried an unknown order suffix.
    UnknownGroupCallOrder {
        /// The rejected token.
        value: String,
        /// The order parse failure.
        source: ParseGroupCallOrderError,
    },
    /// A dial string whose leading path segment names no endpoint type.
    UnknownEndpointType(String),
}

impl std::fmt::Display for OriginateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnclosedQuote(s) => {
                write!(f, "unclosed quote in the final token ({} bytes)", s.len())
            }
            Self::ParseError(s) => write!(f, "parse error: {s}"),
            Self::EmptyInlineApplications => {
                f.write_str("inline originate requires at least one application")
            }
            Self::ExtensionWithInlineDialplan => {
                f.write_str("extension target is incompatible with inline dialplan")
            }
            Self::VariablesNotSupported => {
                f.write_str("this endpoint type carries no variable block")
            }
            Self::InvalidTimeout { value, .. } => write!(
                f,
                "timeout is not a whole number of seconds ({} bytes)",
                value.len()
            ),
            Self::UnknownHangupCause { value, .. } => write!(
                f,
                "unknown hangup cause in an error endpoint ({} bytes)",
                value.len()
            ),
            Self::UnknownGroupCallOrder { value, .. } => {
                write!(f, "unknown group_call order suffix ({} bytes)", value.len())
            }
            Self::UnknownEndpointType(s) => {
                write!(f, "unknown endpoint type ({} bytes)", s.len())
            }
        }
    }
}

impl std::error::Error for OriginateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidTimeout { source, .. } => Some(source),
            Self::UnknownHangupCause { source, .. } => Some(source),
            Self::UnknownGroupCallOrder { source, .. } => Some(source),
            Self::UnclosedQuote(_)
            | Self::ParseError(_)
            | Self::EmptyInlineApplications
            | Self::ExtensionWithInlineDialplan
            | Self::VariablesNotSupported
            | Self::UnknownEndpointType(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::endpoint::{LoopbackEndpoint, SofiaEndpoint, SofiaGateway};

    // --- Endpoint ---

    #[test]
    fn endpoint_uri_only() {
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "123@example.com".into(),
            variables: None,
        });
        assert_eq!(ep.to_string(), "sofia/internal/123@example.com");
    }

    #[test]
    fn endpoint_uri_with_variable() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("one_variable", "1");
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "123@example.com".into(),
            variables: Some(vars),
        });
        assert_eq!(
            ep.to_string(),
            "{one_variable=1}sofia/internal/123@example.com"
        );
    }

    #[test]
    fn endpoint_variable_with_quote() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("one_variable", "one'quote");
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "123@example.com".into(),
            variables: Some(vars),
        });
        // An endpoint renders for the ESL API carrier, which is the deeper of
        // the two escapings.
        assert_eq!(
            ep.to_string(),
            r"{one_variable=one\\\'quote}sofia/internal/123@example.com"
        );
    }

    #[test]
    fn loopback_endpoint_display() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("one_variable", "1");
        let ep = Endpoint::Loopback(
            LoopbackEndpoint::new("aUri")
                .with_context("aContext")
                .with_variables(vars),
        );
        assert_eq!(ep.to_string(), "{one_variable=1}loopback/aUri/aContext");
    }

    #[test]
    fn sofia_gateway_endpoint_display() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("one_variable", "1");
        let ep = Endpoint::SofiaGateway(SofiaGateway {
            destination: "aUri".into(),
            profile: None,
            gateway: "internal".into(),
            variables: Some(vars),
        });
        assert_eq!(
            ep.to_string(),
            "{one_variable=1}sofia/gateway/internal/aUri"
        );
    }

    // --- Application ---

    #[test]
    fn application_xml_format() {
        let app = Application::new("testApp", Some("testArg"));
        assert_eq!(
            app.to_string_with_dialplan(&DialplanType::Xml),
            "&testApp(testArg)"
        );
    }

    #[test]
    fn application_inline_format() {
        let app = Application::new("testApp", Some("testArg"));
        assert_eq!(
            app.to_string_with_dialplan(&DialplanType::Inline),
            "testApp:testArg"
        );
    }

    #[test]
    fn application_inline_no_args() {
        let app = Application::simple("park");
        assert_eq!(app.to_string_with_dialplan(&DialplanType::Inline), "park");
    }

    // --- Originate ---

    #[test]
    fn originate_xml_display() {
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "123@example.com".into(),
            variables: None,
        });
        let orig = Originate::application(ep, Application::new("conference", Some("1")))
            .dialplan(DialplanType::Xml)
            .unwrap();
        assert_eq!(
            orig.to_string(),
            "originate sofia/internal/123@example.com &conference(1) XML"
        );
    }

    #[test]
    fn originate_inline_display() {
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "123@example.com".into(),
            variables: None,
        });
        let orig = Originate::inline(ep, vec![Application::new("conference", Some("1"))])
            .unwrap()
            .dialplan(DialplanType::Inline)
            .unwrap();
        assert_eq!(
            orig.to_string(),
            "originate sofia/internal/123@example.com conference:1 inline"
        );
    }

    #[test]
    fn originate_extension_display() {
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "123@example.com".into(),
            variables: None,
        });
        let orig = Originate::extension(ep, "1000")
            .dialplan(DialplanType::Xml)
            .unwrap()
            .context("default");
        assert_eq!(
            orig.to_string(),
            "originate sofia/internal/123@example.com 1000 XML default"
        );
    }

    #[test]
    fn originate_extension_round_trip() {
        let input = "originate sofia/internal/test@example.com 1000 XML default";
        let parsed: Originate = input
            .parse()
            .unwrap();
        assert_eq!(parsed.to_string(), input);
        assert!(matches!(parsed.target(), OriginateTarget::Extension(ref e) if e == "1000"));
    }

    #[test]
    fn originate_extension_no_dialplan() {
        let input = "originate sofia/internal/test@example.com 1000";
        let parsed: Originate = input
            .parse()
            .unwrap();
        assert!(matches!(parsed.target(), OriginateTarget::Extension(ref e) if e == "1000"));
        assert_eq!(parsed.to_string(), input);
    }

    #[test]
    fn originate_extension_with_inline_errors() {
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "123@example.com".into(),
            variables: None,
        });
        let result = Originate::extension(ep, "1000").dialplan(DialplanType::Inline);
        assert!(result.is_err());
    }

    #[test]
    fn originate_empty_inline_errors() {
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "123@example.com".into(),
            variables: None,
        });
        let result = Originate::inline(ep, vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn originate_from_string_round_trip() {
        let input = "originate {test='variable with quote'}sofia/internal/test@example.com 123";
        let orig: Originate = input
            .parse()
            .unwrap();
        assert!(matches!(orig.target(), OriginateTarget::Extension(ref e) if e == "123"));
        assert_eq!(orig.to_string(), input);
    }

    #[test]
    fn originate_socket_app_quoted() {
        let ep = Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("test"));
        let orig = Originate::application(
            ep,
            Application::new("socket", Some("127.0.0.1:8040 async full")),
        );
        assert_eq!(
            orig.to_string(),
            "originate loopback/9199/test '&socket(127.0.0.1:8040 async full)'"
        );
    }

    #[test]
    fn originate_socket_round_trip() {
        let input = "originate loopback/9199/test '&socket(127.0.0.1:8040 async full)'";
        let parsed: Originate = input
            .parse()
            .unwrap();
        assert_eq!(parsed.to_string(), input);
        if let OriginateTarget::Application(ref app) = parsed.target() {
            assert_eq!(app.args(), Some("127.0.0.1:8040 async full"));
        } else {
            panic!("expected Application target");
        }
    }

    #[test]
    fn originate_display_round_trip() {
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "123@example.com".into(),
            variables: None,
        });
        let orig = Originate::application(ep, Application::new("conference", Some("1")))
            .dialplan(DialplanType::Xml)
            .unwrap();
        let s = orig.to_string();
        let parsed: Originate = s
            .parse()
            .unwrap();
        assert_eq!(parsed.to_string(), s);
    }

    #[test]
    fn originate_inline_no_args_round_trip() {
        let input = "originate sofia/internal/123@example.com park inline";
        let parsed: Originate = input
            .parse()
            .unwrap();
        assert_eq!(parsed.to_string(), input);
        if let OriginateTarget::InlineApplications(ref apps) = parsed.target() {
            assert!(apps[0]
                .args()
                .is_none());
        } else {
            panic!("expected InlineApplications target");
        }
    }

    #[test]
    fn originate_inline_multi_app_round_trip() {
        let input =
            "originate sofia/internal/123@example.com playback:/tmp/test.wav,hangup:NORMAL_CLEARING inline";
        let parsed: Originate = input
            .parse()
            .unwrap();
        assert_eq!(parsed.to_string(), input);
    }

    #[test]
    fn originate_inline_auto_dialplan() {
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "123@example.com".into(),
            variables: None,
        });
        let orig = Originate::inline(ep, vec![Application::simple("park")]).unwrap();
        assert!(orig
            .to_string()
            .contains("inline"));
    }

    // --- DialplanType ---

    #[test]
    fn dialplan_type_display() {
        assert_eq!(DialplanType::Inline.to_string(), "inline");
        assert_eq!(DialplanType::Xml.to_string(), "XML");
    }

    #[test]
    fn dialplan_type_from_str() {
        assert_eq!(
            "inline"
                .parse::<DialplanType>()
                .unwrap(),
            DialplanType::Inline
        );
        assert_eq!(
            "XML"
                .parse::<DialplanType>()
                .unwrap(),
            DialplanType::Xml
        );
    }

    #[test]
    fn dialplan_type_from_str_case_insensitive() {
        assert_eq!(
            "xml"
                .parse::<DialplanType>()
                .unwrap(),
            DialplanType::Xml
        );
        assert_eq!(
            "Xml"
                .parse::<DialplanType>()
                .unwrap(),
            DialplanType::Xml
        );
        assert_eq!(
            "INLINE"
                .parse::<DialplanType>()
                .unwrap(),
            DialplanType::Inline
        );
        assert_eq!(
            "Inline"
                .parse::<DialplanType>()
                .unwrap(),
            DialplanType::Inline
        );
    }

    // --- Serde ---

    #[test]
    fn serde_dialplan_type_xml() {
        let json = serde_json::to_string(&DialplanType::Xml).unwrap();
        assert_eq!(json, "\"xml\"");
        let parsed: DialplanType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, DialplanType::Xml);
    }

    #[test]
    fn serde_dialplan_type_inline() {
        let json = serde_json::to_string(&DialplanType::Inline).unwrap();
        assert_eq!(json, "\"inline\"");
        let parsed: DialplanType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, DialplanType::Inline);
    }

    #[test]
    fn serde_application() {
        let app = Application::new("park", None::<&str>);
        let json = serde_json::to_string(&app).unwrap();
        let parsed: Application = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, app);
    }

    #[test]
    fn serde_application_with_args() {
        let app = Application::new("conference", Some("1"));
        let json = serde_json::to_string(&app).unwrap();
        let parsed: Application = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, app);
    }

    #[test]
    fn serde_application_skips_none_args() {
        let app = Application::new("park", None::<&str>);
        let json = serde_json::to_string(&app).unwrap();
        assert!(!json.contains("args"));
    }

    #[test]
    fn serde_originate_application_round_trip() {
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "123@example.com".into(),
            variables: None,
        });
        let orig = Originate::application(ep, Application::new("park", None::<&str>))
            .dialplan(DialplanType::Xml)
            .unwrap()
            .context("default")
            .cid_name("Test")
            .cid_num("5551234")
            .timeout(Duration::from_secs(30));
        let json = serde_json::to_string(&orig).unwrap();
        assert!(json.contains("\"application\""));
        let parsed: Originate = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, orig);
    }

    #[test]
    fn serde_originate_extension() {
        let json = r#"{
            "endpoint": {"sofia": {"profile": "internal", "destination": "123@example.com"}},
            "extension": "1000",
            "dialplan": "xml",
            "context": "default"
        }"#;
        let orig: Originate = serde_json::from_str(json).unwrap();
        assert!(matches!(orig.target(), OriginateTarget::Extension(ref e) if e == "1000"));
        assert_eq!(
            orig.to_string(),
            "originate sofia/internal/123@example.com 1000 XML default"
        );
    }

    #[test]
    fn serde_originate_extension_with_inline_rejected() {
        let json = r#"{
            "endpoint": {"sofia": {"profile": "internal", "destination": "123@example.com"}},
            "extension": "1000",
            "dialplan": "inline"
        }"#;
        let result = serde_json::from_str::<Originate>(json);
        assert!(result.is_err());
    }

    #[test]
    fn serde_originate_empty_inline_rejected() {
        let json = r#"{
            "endpoint": {"sofia": {"profile": "internal", "destination": "123@example.com"}},
            "inline_applications": []
        }"#;
        let result = serde_json::from_str::<Originate>(json);
        assert!(result.is_err());
    }

    #[test]
    fn serde_originate_inline_applications() {
        let json = r#"{
            "endpoint": {"sofia": {"profile": "internal", "destination": "123@example.com"}},
            "inline_applications": [
                {"name": "playback", "args": "/tmp/test.wav"},
                {"name": "hangup", "args": "NORMAL_CLEARING"}
            ]
        }"#;
        let orig: Originate = serde_json::from_str(json).unwrap();
        if let OriginateTarget::InlineApplications(ref apps) = orig.target() {
            assert_eq!(apps.len(), 2);
        } else {
            panic!("expected InlineApplications");
        }
        assert!(orig
            .to_string()
            .contains("inline"));
    }

    #[test]
    fn serde_originate_skips_none_fields() {
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "123@example.com".into(),
            variables: None,
        });
        let orig = Originate::application(ep, Application::new("park", None::<&str>));
        let json = serde_json::to_string(&orig).unwrap();
        assert!(!json.contains("dialplan"));
        assert!(!json.contains("context"));
        assert!(!json.contains("cid_name"));
        assert!(!json.contains("cid_num"));
        assert!(!json.contains("timeout"));
    }

    #[test]
    fn serde_originate_to_wire_format() {
        let json = r#"{
            "endpoint": {"sofia": {"profile": "internal", "destination": "123@example.com"}},
            "application": {"name": "park"},
            "dialplan": "xml",
            "context": "default"
        }"#;
        let orig: Originate = serde_json::from_str(json).unwrap();
        let wire = orig.to_string();
        assert!(wire.starts_with("originate"));
        assert!(wire.contains("sofia/internal/123@example.com"));
        assert!(wire.contains("&park()"));
        assert!(wire.contains("XML"));
    }

    // --- Application::simple ---

    #[test]
    fn application_simple_no_args() {
        let app = Application::simple("park");
        assert_eq!(app.name(), "park");
        assert!(app
            .args()
            .is_none());
    }

    #[test]
    fn application_simple_xml_format() {
        let app = Application::simple("park");
        assert_eq!(app.to_string_with_dialplan(&DialplanType::Xml), "&park()");
    }

    // --- OriginateTarget From impls ---

    #[test]
    fn originate_target_from_application() {
        let target: OriginateTarget = Application::simple("park").into();
        assert!(matches!(target, OriginateTarget::Application(_)));
    }

    #[test]
    fn originate_target_from_vec() {
        let target: OriginateTarget = vec![
            Application::new("conference", Some("1")),
            Application::new("hangup", Some("NORMAL_CLEARING")),
        ]
        .into();
        if let OriginateTarget::InlineApplications(apps) = target {
            assert_eq!(apps.len(), 2);
        } else {
            panic!("expected InlineApplications");
        }
    }

    #[test]
    fn originate_target_application_wire_format() {
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "123@example.com".into(),
            variables: None,
        });
        let orig = Originate::application(ep, Application::simple("park"));
        assert_eq!(
            orig.to_string(),
            "originate sofia/internal/123@example.com &park()"
        );
    }

    #[test]
    fn originate_timeout_only_fills_positional_gaps() {
        let ep = Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("test"));
        let cmd = Originate::application(ep, Application::simple("park"))
            .timeout(Duration::from_secs(30));
        // timeout is arg 7; dialplan/context/cid must be filled so FS
        // doesn't interpret "30" as the dialplan name
        assert_eq!(
            cmd.to_string(),
            "originate loopback/9199/test &park() XML default undef undef 30"
        );
    }

    #[test]
    fn originate_cid_num_only_fills_preceding_gaps() {
        let ep = Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("test"));
        let cmd = Originate::application(ep, Application::simple("park")).cid_num("5551234");
        assert_eq!(
            cmd.to_string(),
            "originate loopback/9199/test &park() XML default undef 5551234"
        );
    }

    #[test]
    fn originate_context_only_fills_dialplan() {
        let ep = Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("test"));
        let cmd = Originate::extension(ep, "1000").context("myctx");
        assert_eq!(
            cmd.to_string(),
            "originate loopback/9199/test 1000 XML myctx"
        );
    }

    /// The gap-filler context is a context a caller could also have written,
    /// so it reads back as one: the wire round-trips, the struct does not.
    #[test]
    fn originate_context_gap_filler_round_trip_asymmetry() {
        let ep = Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("test"));
        let cmd = Originate::application(ep, Application::simple("park")).cid_name("Alice");
        let wire = cmd.to_string();
        assert!(wire.contains("default"), "gap-filler should emit 'default'");

        let parsed: Originate = wire
            .parse()
            .unwrap();
        // Struct-level asymmetry: None became Some("default")
        assert_eq!(parsed.context_str(), Some("default"));

        // Wire format is identical (the important invariant)
        assert_eq!(parsed.to_string(), wire);
    }

    #[test]
    fn serde_originate_full_round_trip_with_variables() {
        let mut ep_vars = Variables::new(VariablesType::Default);
        ep_vars.insert("originate_timeout", "30");
        ep_vars.insert("sip_h_X-Custom", "value with spaces");
        let ep = Endpoint::SofiaGateway(SofiaGateway {
            gateway: "my_provider".into(),
            destination: "18005551234".into(),
            profile: Some("external".into()),
            variables: Some(ep_vars),
        });
        let orig = Originate::application(ep, Application::new("park", None::<&str>))
            .dialplan(DialplanType::Xml)
            .unwrap()
            .context("public")
            .cid_name("Test Caller")
            .cid_num("5551234")
            .timeout(Duration::from_secs(60));
        let json = serde_json::to_string(&orig).unwrap();
        let parsed: Originate = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, orig);
        assert_eq!(parsed.to_string(), orig.to_string());
    }

    #[test]
    fn serde_originate_inline_round_trip_with_all_fields() {
        let ep = Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("default"));
        let orig = Originate::inline(
            ep,
            vec![
                Application::new("playback", Some("/tmp/test.wav")),
                Application::new("hangup", Some("NORMAL_CLEARING")),
            ],
        )
        .unwrap()
        .dialplan(DialplanType::Inline)
        .unwrap()
        .context("default")
        .cid_name("IVR")
        .cid_num("0000")
        .timeout(Duration::from_secs(45));
        let json = serde_json::to_string(&orig).unwrap();
        let parsed: Originate = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, orig);
        assert_eq!(parsed.to_string(), orig.to_string());
    }

    #[test]
    fn originate_context_named_inline() {
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "123@example.com".into(),
            variables: None,
        });
        let orig = Originate::extension(ep, "1000")
            .dialplan(DialplanType::Xml)
            .unwrap()
            .context("inline");
        let wire = orig.to_string();
        assert!(wire.contains("XML inline"), "wire: {}", wire);
        let parsed: Originate = wire
            .parse()
            .unwrap();
        // "inline" is consumed as the dialplan type, not the context
        // This is an accepted limitation of positional parsing
        assert_eq!(parsed.to_string(), wire);
    }

    #[test]
    fn originate_context_named_xml() {
        let ep = Endpoint::Sofia(SofiaEndpoint {
            profile: "internal".into(),
            destination: "123@example.com".into(),
            variables: None,
        });
        let orig = Originate::extension(ep, "1000")
            .dialplan(DialplanType::Xml)
            .unwrap()
            .context("XML");
        let wire = orig.to_string();
        // "XML XML" - first is dialplan, second is context
        assert!(wire.contains("XML XML"), "wire: {}", wire);
        let parsed: Originate = wire
            .parse()
            .unwrap();
        assert_eq!(parsed.to_string(), wire);
    }

    /// A dial string carries caller-id and `sip_h_*` values, so a rejection
    /// names the field it failed on and leaves the bytes on the error.
    #[test]
    fn errors_name_the_field_and_never_quote_the_input() {
        let cases: [(&str, &str); 4] = [
            (
                "originate sofia/a/b 1000 XML default undef undef 30s",
                "30s",
            ),
            ("originate error/NO_SUCH_CAUSE 1000", "NO_SUCH_CAUSE"),
            ("originate ${group_call(support@example.com+Z)} 1000", "+Z"),
            ("originate verto/15551234567 1000", "15551234567"),
        ];
        for (input, secret) in cases {
            let msg = input
                .parse::<Originate>()
                .expect_err(input)
                .to_string();
            assert!(!msg.contains(secret), "{input} quoted its input: {msg}");
        }

        let msg = originate_split("originate 'never closed", ' ')
            .expect_err("unclosed quote")
            .to_string();
        assert!(!msg.contains("never closed"), "quoted its input: {msg}");
    }

    /// A rejected timeout, cause or order has a cause of its own; stringifying
    /// it into a message drops the chain a caller would match on.
    #[test]
    fn errors_keep_their_source() {
        use std::error::Error;

        for input in [
            "originate sofia/a/b 1000 XML default undef undef 30s",
            "originate error/NO_SUCH_CAUSE 1000",
            "originate ${group_call(support@example.com+Z)} 1000",
        ] {
            assert!(
                input
                    .parse::<Originate>()
                    .expect_err(input)
                    .source()
                    .is_some(),
                "{input} has no source"
            );
        }
    }

    /// A caller-id with a space is quoted on the way out, so the quotes are
    /// this crate's own framing and have to come back off on the way in. Left
    /// on, they are re-quoted at every hop and the switch dials the quotes.
    #[test]
    fn quoted_caller_id_round_trips_without_gaining_quotes() {
        let ep = Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("test"));
        let cmd = Originate::application(ep, Application::simple("park"))
            .cid_name("Outbound Call")
            .cid_num("555 1234");
        let wire = cmd.to_string();
        assert_eq!(
            wire,
            "originate loopback/9199/test &park() XML default 'Outbound Call' '555 1234'"
        );

        let parsed: Originate = wire
            .parse()
            .unwrap();
        assert_eq!(parsed.caller_id_name(), Some("Outbound Call"));
        assert_eq!(parsed.caller_id_number(), Some("555 1234"));
        assert_eq!(parsed.to_string(), wire);
    }

    /// The mutators are the config-driven path: deserialize a template, then
    /// override per call. Clearing a field is half of that and had no coverage.
    #[test]
    fn originate_mutators_set_and_clear_every_optional_field() {
        let ep = Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("default"));
        let mut cmd = Originate::extension(ep, "1000");

        cmd.set_dialplan(Some(DialplanType::Xml));
        cmd.set_context(Some("public"));
        cmd.set_cid_name(Some("Alice"));
        cmd.set_cid_num(Some("5551234"));
        cmd.set_timeout(Some(Duration::from_secs(30)));
        assert_eq!(
            cmd.to_string(),
            "originate loopback/9199/default 1000 XML public Alice 5551234 30"
        );
        assert_eq!(cmd.timeout_duration(), Some(Duration::from_secs(30)));

        // The turbofish is not decoration: `Option<impl Into<String>>` leaves a
        // bare `None` with no type to infer, so clearing a field needs one.
        cmd.set_dialplan(None);
        cmd.set_context(None::<String>);
        cmd.set_cid_name(None::<String>);
        cmd.set_cid_num(None::<String>);
        cmd.set_timeout(None);
        assert_eq!(cmd.to_string(), "originate loopback/9199/default 1000");
    }

    #[test]
    fn originate_mut_accessors_reach_the_endpoint_and_the_target() {
        let ep = Endpoint::Sofia(SofiaEndpoint::new("internal", "1000@example.com"));
        let mut cmd = Originate::application(ep, Application::new("socket", Some("old")));

        *cmd.endpoint_mut() = Endpoint::Loopback(LoopbackEndpoint::new("9199"));
        if let OriginateTarget::Application(app) = cmd.target_mut() {
            *app.name_mut() = "playback".into();
            *app.args_mut() = Some("/tmp/test.wav".into());
        } else {
            panic!("expected Application target");
        }

        assert_eq!(
            cmd.to_string(),
            "originate loopback/9199 &playback(/tmp/test.wav)"
        );
    }

    #[test]
    fn originate_accessors() {
        let ep = Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("default"));
        let cmd = Originate::extension(ep, "1000")
            .dialplan(DialplanType::Xml)
            .unwrap()
            .context("default")
            .cid_name("Alice")
            .cid_num("5551234")
            .timeout(Duration::from_secs(30));

        assert!(matches!(cmd.target(), OriginateTarget::Extension(ref e) if e == "1000"));
        assert_eq!(cmd.dialplan_type(), Some(&DialplanType::Xml));
        assert_eq!(cmd.context_str(), Some("default"));
        assert_eq!(cmd.caller_id_name(), Some("Alice"));
        assert_eq!(cmd.caller_id_number(), Some("5551234"));
        assert_eq!(cmd.timeout_seconds(), Some(30));
    }
}
