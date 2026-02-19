use std::fmt;
use std::str::FromStr;

use indexmap::IndexMap;

use super::originate_split;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialplanType {
    Inline,
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

impl FromStr for DialplanType {
    type Err = OriginateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "inline" => Ok(Self::Inline),
            "XML" => Ok(Self::Xml),
            _ => Err(OriginateError::ParseError(format!(
                "unknown dialplan type: {}",
                s
            ))),
        }
    }
}

/// Scope for channel variables in an originate command.
///
/// - `Enterprise` (`<>`) — applies across all threads (`:_:` separated)
/// - `Default` (`{}`) — applies to all channels in this originate
/// - `Channel` (`[]`) — applies only to one specific channel
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariablesType {
    Enterprise,
    Default,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variables {
    pub vars_type: VariablesType,
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
    pub fn new(vars_type: VariablesType) -> Self {
        Self {
            vars_type,
            inner: IndexMap::new(),
        }
    }

    pub fn with_vars(vars_type: VariablesType, vars: IndexMap<String, String>) -> Self {
        Self {
            vars_type,
            inner: vars,
        }
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.inner
            .insert(key.into(), value.into());
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.inner
            .get(key)
            .map(|s| s.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.inner
            .is_empty()
    }

    pub fn len(&self) -> usize {
        self.inner
            .len()
    }

    pub fn iter(&self) -> indexmap::map::Iter<'_, String, String> {
        self.inner
            .iter()
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
        // Split on commas not preceded by backslash
        for part in split_unescaped_commas(inner_str) {
            let (key, value) = part
                .split_once('=')
                .ok_or_else(|| {
                    OriginateError::ParseError(format!("missing = in variable: {}", part))
                })?;
            inner.insert(key.to_string(), unescape_value(value));
        }

        Ok(Self { vars_type, inner })
    }
}

/// Split on commas that are not preceded by a backslash.
fn split_unescaped_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let bytes = s.as_bytes();

    for i in 0..bytes.len() {
        if bytes[i] == b',' && !(i > 0 && bytes[i - 1] == b'\\') {
            parts.push(&s[start..i]);
            start = i + 1;
        }
    }
    parts.push(&s[start..]);
    parts
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Generic {
        uri: String,
        variables: Option<Variables>,
    },
    Loopback {
        uri: String,
        context: String,
        variables: Option<Variables>,
    },
    SofiaGateway {
        uri: String,
        gateway: String,
        variables: Option<Variables>,
    },
}

impl Endpoint {
    fn write_variables(f: &mut fmt::Formatter<'_>, vars: &Option<Variables>) -> fmt::Result {
        if let Some(vars) = vars {
            if !vars.is_empty() {
                write!(f, "{}", vars)?;
            }
        }
        Ok(())
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Generic { uri, variables } => {
                Self::write_variables(f, variables)?;
                f.write_str(uri)
            }
            Self::Loopback {
                uri,
                context,
                variables,
            } => {
                Self::write_variables(f, variables)?;
                write!(f, "loopback/{}/{}", uri, context)
            }
            Self::SofiaGateway {
                uri,
                gateway,
                variables,
            } => {
                Self::write_variables(f, variables)?;
                write!(f, "sofia/gateway/{}/{}", gateway, uri)
            }
        }
    }
}

impl FromStr for Endpoint {
    type Err = OriginateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (variables, uri_part) = if s.contains('{') {
            let close = s
                .find('}')
                .ok_or_else(|| OriginateError::ParseError("unclosed { in endpoint".into()))?;
            let var_str = &s[..=close];
            let vars: Variables = var_str.parse()?;
            let vars = if vars.is_empty() { None } else { Some(vars) };
            (vars, s[close + 1..].trim())
        } else {
            (None, s)
        };

        Ok(Self::Generic {
            uri: uri_part.to_string(),
            variables,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Application {
    pub name: String,
    pub args: Option<String>,
}

impl Application {
    pub fn new(name: impl Into<String>, args: Option<impl Into<String>>) -> Self {
        Self {
            name: name.into(),
            args: args.map(|a| a.into()),
        }
    }

    pub fn to_string_with_dialplan(&self, dialplan: &DialplanType) -> String {
        let args = self
            .args
            .as_deref()
            .unwrap_or("");
        match dialplan {
            DialplanType::Inline => format!("{}:{}", self.name, args),
            DialplanType::Xml => format!("&{}({})", self.name, args),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationList(pub Vec<Application>);

impl ApplicationList {
    pub fn to_string_with_dialplan(
        &self,
        dialplan: &DialplanType,
    ) -> Result<String, OriginateError> {
        match dialplan {
            DialplanType::Inline => {
                let parts: Vec<String> = self
                    .0
                    .iter()
                    .map(|app| app.to_string_with_dialplan(dialplan))
                    .collect();
                Ok(parts.join(","))
            }
            DialplanType::Xml => {
                if self
                    .0
                    .len()
                    > 1
                {
                    return Err(OriginateError::TooManyApplications);
                }
                Ok(self.0[0].to_string_with_dialplan(dialplan))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Originate {
    pub endpoint: Endpoint,
    pub applications: ApplicationList,
    pub dialplan: Option<DialplanType>,
    pub context: Option<String>,
    pub cid_name: Option<String>,
    pub cid_num: Option<String>,
    pub timeout: Option<u32>,
}

impl fmt::Display for Originate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let dialplan = self
            .dialplan
            .unwrap_or(DialplanType::Xml);
        let apps = self
            .applications
            .to_string_with_dialplan(&dialplan)
            .map_err(|_| fmt::Error)?;

        write!(f, "originate {} {}", self.endpoint, apps)?;

        if let Some(ref dp) = self.dialplan {
            write!(f, " {}", dp)?;
        }
        if let Some(ref ctx) = self.context {
            write!(f, " {}", ctx)?;
        }
        if let Some(ref name) = self.cid_name {
            write!(f, " {}", name)?;
        }
        if let Some(ref num) = self.cid_num {
            write!(f, " {}", num)?;
        }
        if let Some(timeout) = self.timeout {
            write!(f, " {}", timeout)?;
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
                "missing application in originate".into(),
            ));
        }

        let app_str = args.remove(0);

        let dialplan = args
            .first()
            .and_then(|s| {
                s.parse::<DialplanType>()
                    .ok()
            });
        if dialplan.is_some() {
            args.remove(0);
        }

        let applications = super::parse_application_list(&app_str, dialplan.as_ref())?;

        let context = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let cid_name = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let cid_num = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let timeout = if !args.is_empty() {
            Some(
                args.remove(0)
                    .parse::<u32>()
                    .map_err(|e| OriginateError::ParseError(format!("invalid timeout: {}", e)))?,
            )
        } else {
            None
        };

        Ok(Self {
            endpoint,
            applications,
            dialplan,
            context,
            cid_name,
            cid_num,
            timeout,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OriginateError {
    #[error("unclosed quote at: {0}")]
    UnclosedQuote(String),
    #[error("too many applications for non-inline dialplan")]
    TooManyApplications,
    #[error("parse error: {0}")]
    ParseError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // --- Endpoint ---

    #[test]
    fn endpoint_uri_only() {
        let ep = Endpoint::Generic {
            uri: "sofia/internal/123@cauca.ca".into(),
            variables: None,
        };
        assert_eq!(ep.to_string(), "sofia/internal/123@cauca.ca");
    }

    #[test]
    fn endpoint_uri_with_variable() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("one_variable", "1");
        let ep = Endpoint::Generic {
            uri: "sofia/internal/123@cauca.ca".into(),
            variables: Some(vars),
        };
        assert_eq!(
            ep.to_string(),
            "{one_variable=1}sofia/internal/123@cauca.ca"
        );
    }

    #[test]
    fn endpoint_variable_with_quote() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("one_variable", "one'quote");
        let ep = Endpoint::Generic {
            uri: "sofia/internal/123@cauca.ca".into(),
            variables: Some(vars),
        };
        assert_eq!(
            ep.to_string(),
            "{one_variable=one\\'quote}sofia/internal/123@cauca.ca"
        );
    }

    #[test]
    fn loopback_endpoint_display() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("one_variable", "1");
        let ep = Endpoint::Loopback {
            uri: "aUri".into(),
            context: "aContext".into(),
            variables: Some(vars),
        };
        assert_eq!(ep.to_string(), "{one_variable=1}loopback/aUri/aContext");
    }

    #[test]
    fn sofia_gateway_endpoint_display() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("one_variable", "1");
        let ep = Endpoint::SofiaGateway {
            uri: "aUri".into(),
            gateway: "internal".into(),
            variables: Some(vars),
        };
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

    // --- ApplicationList ---

    #[test]
    fn application_list_single_xml() {
        let list = ApplicationList(vec![Application::new("testApp1", Some("testArg1"))]);
        assert_eq!(
            list.to_string_with_dialplan(&DialplanType::Xml)
                .unwrap(),
            "&testApp1(testArg1)"
        );
    }

    #[test]
    fn application_list_single_inline() {
        let list = ApplicationList(vec![Application::new("testApp1", Some("testArg1"))]);
        assert_eq!(
            list.to_string_with_dialplan(&DialplanType::Inline)
                .unwrap(),
            "testApp1:testArg1"
        );
    }

    #[test]
    fn application_list_empty_xml_errors() {
        let list = ApplicationList(vec![]);
        assert!(list
            .to_string_with_dialplan(&DialplanType::Xml)
            .is_err());
    }

    #[test]
    fn application_list_empty_inline() {
        let list = ApplicationList(vec![]);
        assert_eq!(
            list.to_string_with_dialplan(&DialplanType::Inline)
                .unwrap(),
            ""
        );
    }

    #[test]
    fn application_list_two_xml_errors() {
        let list = ApplicationList(vec![
            Application::new("testApp1", Some("testArg1")),
            Application::new("testApp2", Some("testArg2")),
        ]);
        assert!(list
            .to_string_with_dialplan(&DialplanType::Xml)
            .is_err());
    }

    #[test]
    fn application_list_two_inline() {
        let list = ApplicationList(vec![
            Application::new("testApp1", Some("testArg1")),
            Application::new("testApp2", Some("testArg2")),
        ]);
        assert_eq!(
            list.to_string_with_dialplan(&DialplanType::Inline)
                .unwrap(),
            "testApp1:testArg1,testApp2:testArg2"
        );
    }

    // --- Originate ---

    #[test]
    fn originate_xml_display() {
        let ep = Endpoint::Generic {
            uri: "sofia/internal/123@cauca.ca".into(),
            variables: None,
        };
        let apps = ApplicationList(vec![Application::new("conference", Some("1"))]);
        let orig = Originate {
            endpoint: ep,
            applications: apps,
            dialplan: Some(DialplanType::Xml),
            context: None,
            cid_name: None,
            cid_num: None,
            timeout: None,
        };
        assert_eq!(
            orig.to_string(),
            "originate sofia/internal/123@cauca.ca &conference(1) XML"
        );
    }

    #[test]
    fn originate_inline_display() {
        let ep = Endpoint::Generic {
            uri: "sofia/internal/123@cauca.ca".into(),
            variables: None,
        };
        let apps = ApplicationList(vec![Application::new("conference", Some("1"))]);
        let orig = Originate {
            endpoint: ep,
            applications: apps,
            dialplan: Some(DialplanType::Inline),
            context: None,
            cid_name: None,
            cid_num: None,
            timeout: None,
        };
        assert_eq!(
            orig.to_string(),
            "originate sofia/internal/123@cauca.ca conference:1 inline"
        );
    }

    #[test]
    fn originate_from_string_round_trip() {
        let input = "originate {test='variable with quote'}sofia/test 123";
        let orig: Originate = input
            .parse()
            .unwrap();
        assert!(orig
            .endpoint
            .to_string()
            .contains("sofia/test"));
    }

    #[test]
    fn originate_display_round_trip() {
        let ep = Endpoint::Generic {
            uri: "sofia/internal/123@cauca.ca".into(),
            variables: None,
        };
        let apps = ApplicationList(vec![Application::new("conference", Some("1"))]);
        let orig = Originate {
            endpoint: ep,
            applications: apps,
            dialplan: Some(DialplanType::Xml),
            context: None,
            cid_name: None,
            cid_num: None,
            timeout: None,
        };
        let s = orig.to_string();
        let parsed: Originate = s
            .parse()
            .unwrap();
        assert_eq!(parsed.to_string(), s);
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
}
