use std::fmt;
use std::str::FromStr;

use indexmap::IndexMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialplanType {
    Inline,
    Xml,
}

impl fmt::Display for DialplanType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

impl FromStr for DialplanType {
    type Err = OriginateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        todo!()
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

/// Ordered set of channel variables with FreeSWITCH escaping.
///
/// Values containing commas are escaped with `\,`, single quotes with `\'`,
/// and values with spaces are wrapped in single quotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variables {
    pub vars_type: VariablesType,
    inner: IndexMap<String, String>,
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
        todo!()
    }
}

impl FromStr for Variables {
    type Err = OriginateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        todo!()
    }
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

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

impl FromStr for Endpoint {
    type Err = OriginateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        todo!()
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
        todo!()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationList(pub Vec<Application>);

impl ApplicationList {
    pub fn to_string_with_dialplan(
        &self,
        dialplan: &DialplanType,
    ) -> Result<String, OriginateError> {
        todo!()
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
        todo!()
    }
}

impl FromStr for Originate {
    type Err = OriginateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        todo!()
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
