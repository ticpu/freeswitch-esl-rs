//! Originate command builder with endpoint configuration, variable scoping,
//! and automatic quoting for socket application arguments.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use indexmap::IndexMap;

use super::{originate_quote, originate_split, originate_unquote};

/// FreeSWITCH keyword for omitted positional arguments.
///
/// `switch_separate_string` converts `"undef"` to NULL, making it the
/// canonical placeholder when a later positional arg forces earlier ones
/// to be present on the wire.
const UNDEF: &str = "undef";

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

/// Scope for channel variables in an originate command.
///
/// - `Enterprise` (`<>`) -- applies across all threads (`:_:` separated)
/// - `Default` (`{}`) -- applies to all channels in this originate
/// - `Channel` (`[]`) -- applies only to one specific channel
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[non_exhaustive]
pub enum VariablesType {
    /// `<>` scope -- applies across all `:_:` separated threads.
    Enterprise,
    /// `{}` scope -- applies to all channels in this originate.
    Default,
    /// `[]` scope -- applies to one specific channel.
    Channel,
}

impl VariablesType {
    fn delimiters(self) -> (char, char) {
        match self {
            Self::Enterprise => ('<', '>'),
            Self::Default => ('{', '}'),
            Self::Channel => ('[', ']'),
        }
    }
}

/// Ordered set of channel variables with FreeSWITCH escaping.
///
/// Values containing commas are escaped with `\,`, single quotes with `\'`,
/// and values with spaces are wrapped in single quotes.
///
/// # Serde format
///
/// [`Default`](VariablesType::Default) scope serializes as a flat JSON map:
/// `{"key": "value", ...}`. Non-default scopes serialize as
/// `{"scope": "Enterprise", "vars": {"key": "value"}}`.
/// Deserialization accepts both formats; a flat map implies `Default` scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variables {
    vars_type: VariablesType,
    inner: IndexMap<String, String>,
}

fn escape_value(value: &str) -> String {
    let escaped = value
        .replace('\'', "\\'")
        .replace(',', "\\,");
    if escaped.contains(' ') {
        format!("'{}'", escaped)
    } else {
        escaped
    }
}

fn unescape_value(value: &str) -> String {
    let s = value
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .unwrap_or(value);
    s.replace("\\,", ",")
        .replace("\\'", "'")
}

impl Variables {
    /// Create an empty variable set with the given scope.
    pub fn new(vars_type: VariablesType) -> Self {
        Self {
            vars_type,
            inner: IndexMap::new(),
        }
    }

    /// Create from an existing set of key-value pairs.
    pub fn with_vars(
        vars_type: VariablesType,
        vars: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        Self {
            vars_type,
            inner: vars
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    /// Insert or overwrite a variable.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.inner
            .insert(key.into(), value.into());
    }

    /// Remove a variable by name, returning its value if it existed.
    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.inner
            .shift_remove(key)
    }

    /// Look up a variable by name.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.inner
            .get(key)
            .map(|s| s.as_str())
    }

    /// Whether the set contains no variables.
    pub fn is_empty(&self) -> bool {
        self.inner
            .is_empty()
    }

    /// Number of variables.
    pub fn len(&self) -> usize {
        self.inner
            .len()
    }

    /// Variable scope (Enterprise, Default, or Channel).
    pub fn scope(&self) -> VariablesType {
        self.vars_type
    }

    /// Change the variable scope.
    pub fn set_scope(&mut self, scope: VariablesType) {
        self.vars_type = scope;
    }

    /// Iterate over key-value pairs in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.inner
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Mutable iterator over key-value pairs in insertion order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&str, &mut String)> {
        self.inner
            .iter_mut()
            .map(|(k, v)| (k.as_str(), v))
    }

    /// Mutable iterator over values in insertion order.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut String> {
        self.inner
            .values_mut()
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Variables {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.vars_type == VariablesType::Default {
            self.inner
                .serialize(serializer)
        } else {
            use serde::ser::SerializeStruct;
            let mut s = serializer.serialize_struct("Variables", 2)?;
            s.serialize_field("scope", &self.vars_type)?;
            s.serialize_field("vars", &self.inner)?;
            s.end()
        }
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Variables {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum VariablesRepr {
            Scoped {
                scope: VariablesType,
                vars: IndexMap<String, String>,
            },
            Flat(IndexMap<String, String>),
        }

        match VariablesRepr::deserialize(deserializer)? {
            VariablesRepr::Scoped { scope, vars } => Ok(Self {
                vars_type: scope,
                inner: vars,
            }),
            VariablesRepr::Flat(map) => Ok(Self {
                vars_type: VariablesType::Default,
                inner: map,
            }),
        }
    }
}

impl fmt::Display for Variables {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (open, close) = self
            .vars_type
            .delimiters();
        f.write_fmt(format_args!("{}", open))?;
        for (i, (key, value)) in self
            .inner
            .iter()
            .enumerate()
        {
            if i > 0 {
                f.write_str(",")?;
            }
            write!(f, "{}={}", key, escape_value(value))?;
        }
        f.write_fmt(format_args!("{}", close))
    }
}

impl FromStr for Variables {
    type Err = OriginateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.len() < 2 {
            return Err(OriginateError::ParseError(
                "variable block too short".into(),
            ));
        }

        let (vars_type, inner_str) = match (s.as_bytes()[0], s.as_bytes()[s.len() - 1]) {
            (b'{', b'}') => (VariablesType::Default, &s[1..s.len() - 1]),
            (b'<', b'>') => (VariablesType::Enterprise, &s[1..s.len() - 1]),
            (b'[', b']') => (VariablesType::Channel, &s[1..s.len() - 1]),
            _ => {
                return Err(OriginateError::ParseError(format!(
                    "unknown variable delimiters: {}",
                    s
                )));
            }
        };

        let mut inner = IndexMap::new();
        if !inner_str.is_empty() {
            if let Some(rest) = inner_str.strip_prefix("^^") {
                let sep = rest
                    .chars()
                    .next()
                    .ok_or_else(|| {
                        OriginateError::ParseError("^^ without separator character".into())
                    })?;
                let (_, close) = vars_type.delimiters();
                if sep == close || sep == '=' {
                    return Err(OriginateError::ParseError(format!(
                        "invalid ^^ separator: '{sep}'"
                    )));
                }
                let var_str = &rest[sep.len_utf8()..];
                if !var_str.is_empty() {
                    for part in var_str.split(sep) {
                        let (key, value) = part
                            .split_once('=')
                            .ok_or_else(|| {
                                OriginateError::ParseError(format!("missing = in variable: {part}"))
                            })?;
                        inner.insert(key.to_string(), value.to_string());
                    }
                }
            } else {
                for part in split_unescaped_commas(inner_str) {
                    let (key, value) = part
                        .split_once('=')
                        .ok_or_else(|| {
                            OriginateError::ParseError(format!("missing = in variable: {part}"))
                        })?;
                    inner.insert(key.to_string(), unescape_value(value));
                }
            }
        }

        Ok(Self { vars_type, inner })
    }
}

/// Split on commas that are not escaped by a backslash.
///
/// A comma preceded by an odd number of backslashes is escaped (e.g. `\,`).
/// A comma preceded by an even number of backslashes is a real split point
/// (e.g. `\\,` means escaped backslash followed by comma delimiter).
fn split_unescaped_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let bytes = s.as_bytes();

    for i in 0..bytes.len() {
        if bytes[i] == b',' {
            let mut backslashes = 0;
            let mut j = i;
            while j > 0 && bytes[j - 1] == b'\\' {
                backslashes += 1;
                j -= 1;
            }
            if backslashes % 2 == 0 {
                parts.push(&s[start..i]);
                start = i + 1;
            }
        }
    }
    parts.push(&s[start..]);
    parts
}

// Endpoint is now defined in endpoint.rs -- re-exported via pub use below.
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
            // XML and custom dialplans use the &app(args) syntax.
            _ => {
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

        // Positional args: dialplan, context, cid_name, cid_num, timeout.
        // FreeSWITCH parses by position, so if a later arg is present,
        // all preceding ones must be emitted with defaults.
        let dialplan = match &self.target {
            OriginateTarget::InlineApplications(_) => Some(
                self.dialplan
                    .unwrap_or(DialplanType::Inline),
            ),
            _ => self.dialplan,
        };
        let has_ctx = self
            .context
            .is_some();
        let has_name = self
            .cid_name
            .is_some();
        let has_num = self
            .cid_num
            .is_some();
        let has_timeout = self
            .timeout
            .is_some();

        // Positional args; FS expects the literal "UNDEF" / "default" sentinels for missing slots, not an empty string.
        if dialplan.is_some() || has_ctx || has_name || has_num || has_timeout {
            let dp = dialplan
                .as_ref()
                .cloned()
                .unwrap_or(DialplanType::Xml);
            write!(f, " {}", dp)?;
        }
        if has_ctx || has_name || has_num || has_timeout {
            write!(
                f,
                " {}",
                self.context
                    .as_deref()
                    .unwrap_or("default")
            )?;
        }
        if has_name || has_num || has_timeout {
            let name = self
                .cid_name
                .as_deref()
                .unwrap_or(UNDEF);
            write!(f, " {}", originate_quote(name))?;
        }
        if has_num || has_timeout {
            let num = self
                .cid_num
                .as_deref()
                .unwrap_or(UNDEF);
            write!(f, " {}", originate_quote(num))?;
        }
        if let Some(ref timeout) = self.timeout {
            write!(f, " {}", timeout.as_secs())?;
        }
        Ok(())
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

        let dialplan = args
            .first()
            .and_then(|s| {
                s.parse::<DialplanType>()
                    .ok()
            });
        if dialplan.is_some() {
            args.remove(0);
        }

        let target = super::parse_originate_target(&target_str, dialplan.as_ref())?;

        let context = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let cid_name = if !args.is_empty() {
            let v = args.remove(0);
            if v.eq_ignore_ascii_case(UNDEF) {
                None
            } else {
                Some(v)
            }
        } else {
            None
        };
        let cid_num = if !args.is_empty() {
            let v = args.remove(0);
            if v.eq_ignore_ascii_case(UNDEF) {
                None
            } else {
                Some(v)
            }
        } else {
            None
        };
        let timeout = if !args.is_empty() {
            Some(Duration::from_secs(
                args.remove(0)
                    .parse::<u64>()
                    .map_err(|e| OriginateError::ParseError(format!("invalid timeout: {}", e)))?,
            ))
        } else {
            None
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
}

impl std::fmt::Display for OriginateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnclosedQuote(s) => write!(f, "unclosed quote at: {s}"),
            Self::ParseError(s) => write!(f, "parse error: {s}"),
            Self::EmptyInlineApplications => {
                f.write_str("inline originate requires at least one application")
            }
            Self::ExtensionWithInlineDialplan => {
                f.write_str("extension target is incompatible with inline dialplan")
            }
        }
    }
}

impl std::error::Error for OriginateError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::endpoint::{LoopbackEndpoint, SofiaEndpoint, SofiaGateway};

    // --- Variables ---

    #[test]
    fn variables_standard_chars() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("test_key", "this_value");
        let result = vars.to_string();
        assert!(result.contains("test_key"));
        assert!(result.contains("this_value"));
    }

    #[test]
    fn variables_comma_escaped() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("test_key", "this,is,a,value");
        let result = vars.to_string();
        assert!(result.contains("\\,"));
    }

    #[test]
    fn variables_spaces_quoted() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("test_key", "this is a value");
        let result = vars.to_string();
        assert_eq!(
            result
                .matches('\'')
                .count(),
            2
        );
    }

    #[test]
    fn variables_single_quote_escaped() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("test_key", "let's_this_be_a_value");
        let result = vars.to_string();
        assert!(result.contains("\\'"));
    }

    #[test]
    fn variables_enterprise_delimiters() {
        let mut vars = Variables::new(VariablesType::Enterprise);
        vars.insert("k", "v");
        let result = vars.to_string();
        assert!(result.starts_with('<'));
        assert!(result.ends_with('>'));
    }

    #[test]
    fn variables_channel_delimiters() {
        let mut vars = Variables::new(VariablesType::Channel);
        vars.insert("k", "v");
        let result = vars.to_string();
        assert!(result.starts_with('['));
        assert!(result.ends_with(']'));
    }

    #[test]
    fn variables_default_delimiters() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("k", "v");
        let result = vars.to_string();
        assert!(result.starts_with('{'));
        assert!(result.ends_with('}'));
    }

    #[test]
    fn variables_parse_round_trip() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("origination_caller_id_number", "9005551212");
        vars.insert("sip_h_Call-Info", "<url>;meta=123,<uri>");
        let s = vars.to_string();
        let parsed: Variables = s
            .parse()
            .unwrap();
        assert_eq!(
            parsed.get("origination_caller_id_number"),
            Some("9005551212")
        );
        assert_eq!(parsed.get("sip_h_Call-Info"), Some("<url>;meta=123,<uri>"));
    }

    #[test]
    fn split_unescaped_commas_basic() {
        assert_eq!(split_unescaped_commas("a,b,c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn split_unescaped_commas_escaped() {
        assert_eq!(split_unescaped_commas(r"a\,b,c"), vec![r"a\,b", "c"]);
    }

    #[test]
    fn split_unescaped_commas_double_backslash() {
        // \\, = escaped backslash + comma delimiter
        assert_eq!(split_unescaped_commas(r"a\\,b"), vec![r"a\\", "b"]);
    }

    #[test]
    fn split_unescaped_commas_triple_backslash() {
        // \\\, = escaped backslash + escaped comma (no split)
        assert_eq!(split_unescaped_commas(r"a\\\,b"), vec![r"a\\\,b"]);
    }

    #[test]
    fn variables_caret_caret_separator() {
        let vars: Variables =
            "[^^:sip_invite_domain=pbx.example.com:presence_id=1211@pbx.example.com]"
                .parse()
                .unwrap();
        assert_eq!(vars.scope(), VariablesType::Channel);
        assert_eq!(vars.get("sip_invite_domain"), Some("pbx.example.com"));
        assert_eq!(vars.get("presence_id"), Some("1211@pbx.example.com"));
    }

    #[test]
    fn variables_caret_caret_display_uses_canonical_comma() {
        let vars: Variables = "[^^:a=1:b=2]"
            .parse()
            .unwrap();
        assert_eq!(vars.to_string(), "[a=1,b=2]");
    }

    #[test]
    fn variables_caret_caret_default_scope() {
        let vars: Variables = "{^^|x=1|y=2}"
            .parse()
            .unwrap();
        assert_eq!(vars.scope(), VariablesType::Default);
        assert_eq!(vars.get("x"), Some("1"));
        assert_eq!(vars.get("y"), Some("2"));
    }

    #[test]
    fn variables_caret_caret_enterprise_scope() {
        let vars: Variables = "<^^;a=1;b=2>"
            .parse()
            .unwrap();
        assert_eq!(vars.scope(), VariablesType::Enterprise);
        assert_eq!(vars.get("a"), Some("1"));
    }

    #[test]
    fn variables_caret_caret_no_unescape() {
        let vars: Variables = r"[^^:key=val\,ue:other=x]"
            .parse()
            .unwrap();
        assert_eq!(vars.get("key"), Some(r"val\,ue"));
    }

    #[test]
    fn variables_caret_caret_values_with_commas() {
        let vars: Variables = "[^^|sip_h_X-Call-Info=<urn:foo>;purpose=bar,<urn:baz>|other=val]"
            .parse()
            .unwrap();
        assert_eq!(
            vars.get("sip_h_X-Call-Info"),
            Some("<urn:foo>;purpose=bar,<urn:baz>")
        );
        assert_eq!(vars.get("other"), Some("val"));
    }

    #[test]
    fn variables_caret_caret_empty_vars() {
        let vars: Variables = "[^^:]"
            .parse()
            .unwrap();
        assert!(vars.is_empty());
        assert_eq!(vars.scope(), VariablesType::Channel);
    }

    #[test]
    fn variables_caret_caret_missing_separator() {
        assert!("[^^]"
            .parse::<Variables>()
            .is_err());
    }

    #[test]
    fn variables_caret_caret_closing_bracket_as_sep() {
        assert!("[^^]]"
            .parse::<Variables>()
            .is_err());
    }

    #[test]
    fn variables_caret_caret_equals_as_sep() {
        assert!("[^^=a=1]"
            .parse::<Variables>()
            .is_err());
    }

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
        assert_eq!(
            ep.to_string(),
            "{one_variable=one\\'quote}sofia/internal/123@example.com"
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
    fn serde_variables_type() {
        let json = serde_json::to_string(&VariablesType::Enterprise).unwrap();
        assert_eq!(json, "\"enterprise\"");
        let parsed: VariablesType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, VariablesType::Enterprise);
    }

    #[test]
    fn serde_variables_flat_default() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("key1", "val1");
        vars.insert("key2", "val2");
        let json = serde_json::to_string(&vars).unwrap();
        // Default scope serializes as a flat map
        let parsed: Variables = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.scope(), VariablesType::Default);
        assert_eq!(parsed.get("key1"), Some("val1"));
        assert_eq!(parsed.get("key2"), Some("val2"));
    }

    #[test]
    fn serde_variables_scoped_enterprise() {
        let mut vars = Variables::new(VariablesType::Enterprise);
        vars.insert("key1", "val1");
        let json = serde_json::to_string(&vars).unwrap();
        // Non-default scope serializes as {scope, vars}
        assert!(json.contains("\"enterprise\""));
        let parsed: Variables = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.scope(), VariablesType::Enterprise);
        assert_eq!(parsed.get("key1"), Some("val1"));
    }

    #[test]
    fn serde_variables_flat_map_deserializes_as_default() {
        let json = r#"{"key1":"val1","key2":"val2"}"#;
        let vars: Variables = serde_json::from_str(json).unwrap();
        assert_eq!(vars.scope(), VariablesType::Default);
        assert_eq!(vars.get("key1"), Some("val1"));
        assert_eq!(vars.get("key2"), Some("val2"));
    }

    #[test]
    fn serde_variables_scoped_deserializes() {
        let json = r#"{"scope":"channel","vars":{"k":"v"}}"#;
        let vars: Variables = serde_json::from_str(json).unwrap();
        assert_eq!(vars.scope(), VariablesType::Channel);
        assert_eq!(vars.get("k"), Some("v"));
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

    /// `context: None` with a later positional arg emits `"default"` as a
    /// gap-filler. `FromStr` reads it back as `Some("default")` because
    /// `"default"` is also a valid user-specified context. The wire format
    /// round-trips correctly (identical string), but the struct-level
    /// representation differs. This is an accepted asymmetry.
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

    // --- T1: Full Originate serde round-trip ---

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

    // --- T5: Variables::from_str with empty block ---

    #[test]
    fn variables_from_str_empty_block() {
        let result = "{}".parse::<Variables>();
        assert!(
            result.is_ok(),
            "empty variable block should parse successfully"
        );
        let vars = result.unwrap();
        assert!(
            vars.is_empty(),
            "parsed empty block should have no variables"
        );
    }

    #[test]
    fn variables_from_str_empty_channel_block() {
        let result = "[]".parse::<Variables>();
        assert!(result.is_ok());
        let vars = result.unwrap();
        assert!(vars.is_empty());
        assert_eq!(vars.scope(), VariablesType::Channel);
    }

    #[test]
    fn variables_from_str_empty_enterprise_block() {
        let result = "<>".parse::<Variables>();
        assert!(result.is_ok());
        let vars = result.unwrap();
        assert!(vars.is_empty());
        assert_eq!(vars.scope(), VariablesType::Enterprise);
    }

    // --- T5: Originate::from_str with context named "inline" or "XML" ---

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
