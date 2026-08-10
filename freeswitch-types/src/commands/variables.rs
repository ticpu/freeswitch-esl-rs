//! Channel variable scope and ordered key-value storage for originate commands.

use indexmap::IndexMap;
use std::fmt;
use std::str::FromStr;

use super::originate::OriginateError;

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
    pub(super) fn delimiters(self) -> (char, char) {
        match self {
            Self::Enterprise => ('<', '>'),
            Self::Default => ('{', '}'),
            Self::Channel => ('[', ']'),
        }
    }
}

/// Ordered set of channel variables with FreeSWITCH escaping.
///
/// Backslashes are doubled, commas are escaped with `\,`, single quotes with
/// `\'`, and values with spaces are wrapped in single quotes. This form
/// round-trips through [`FromStr`]; what the switch itself decodes depends on
/// which command carries the block, and is documented in
/// `docs/dial-string-format.md`.
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

pub(super) fn escape_value(value: &str) -> String {
    // The backslash goes first, or the ones introduced below get doubled.
    let escaped = value
        .replace('\\', "\\\\")
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

    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(next) => out.push(next),
            // Hand-written input can end in a backslash that escapes nothing.
            None => out.push('\\'),
        }
    }
    out
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
            (open, close) => {
                return Err(OriginateError::ParseError(format!(
                    "unknown variable delimiters: {:?}..{:?}",
                    open as char, close as char
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
                    for (i, part) in var_str
                        .split(sep)
                        .enumerate()
                    {
                        let (key, value) = part
                            .split_once('=')
                            .ok_or_else(|| {
                                OriginateError::ParseError(format!("missing = in variable {i}"))
                            })?;
                        inner.insert(key.to_string(), value.to_string());
                    }
                }
            } else {
                for (i, part) in split_unescaped_commas(inner_str)
                    .into_iter()
                    .enumerate()
                {
                    let (key, value) = part
                        .split_once('=')
                        .ok_or_else(|| {
                            OriginateError::ParseError(format!("missing = in variable {i}"))
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
pub(super) fn split_unescaped_commas(s: &str) -> Vec<&str> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Measured against a live switch, with two awkward values in one block —
    /// a block carrying only one is more forgiving and hides the failure.
    /// Sweeping the backslash count, each carrier succeeds at counts the other
    /// fails, so these forms are the wire contract and not a preference.
    #[test]
    fn escaping_pins_the_measured_wire_forms() {
        let cases = [
            (DialStringCarrier::Dialplan, "it's", r"it\\'s"),
            (DialStringCarrier::EslApi, "it's", r"it\\\'s"),
            (DialStringCarrier::Dialplan, r"a\nb", r"a\\\\\\\\nb"),
            (DialStringCarrier::EslApi, r"a\nb", r"a\\\\\\\\nb"),
            (DialStringCarrier::Dialplan, "a,b", r"a\,b"),
            (DialStringCarrier::EslApi, "a,b", r"a\,b"),
        ];
        for (carrier, value, want) in cases {
            assert_eq!(
                escape_value(value, carrier),
                want,
                "{value:?} for {carrier:?}"
            );
        }
    }

    /// The crate exists to drive `api originate`, so a block rendered with no
    /// carrier named is rendered for that one.
    #[test]
    fn display_defaults_to_the_api_carrier() {
        let mut vars = Variables::new(VariablesType::Default);
        vars.insert("k", "it's");
        assert_eq!(vars.to_string(), r"{k=it\\\'s}");
        assert_eq!(
            vars.display_for(DialStringCarrier::Dialplan)
                .to_string(),
            r"{k=it\\'s}"
        );
    }

    #[test]
    fn round_trips_at_either_carrier() {
        for carrier in [DialStringCarrier::EslApi, DialStringCarrier::Dialplan] {
            for value in ["it's", r"a\,b", r"C:\path", "a,b", "with space"] {
                let mut vars = Variables::new(VariablesType::Default);
                vars.insert("k", value);
                vars.insert("after", "sentinel");
                let rendered = vars
                    .display_for(carrier)
                    .to_string();

                let back = Variables::parse_for(&rendered, carrier).unwrap_or_else(|e| {
                    panic!("{value:?} for {carrier:?} rendered {rendered}: {e}")
                });
                assert_eq!(back.get("k"), Some(value), "rendered {rendered}");
                assert_eq!(back.get("after"), Some("sentinel"), "rendered {rendered}");
            }
        }
    }

    /// `split_unescaped_commas` reads a comma behind an even number of
    /// backslashes as a real separator, so a value ending in a backslash has to
    /// be written with its own backslash escaped or the writer contradicts the
    /// reader and the block no longer parses.
    #[test]
    fn value_with_backslash_round_trips() {
        for value in [r"a\,b", r"C:\path", r"trailing\", r"\\", r"a\nb"] {
            let mut vars = Variables::new(VariablesType::Default);
            vars.insert("k", value);
            vars.insert("after", "sentinel");
            let rendered = vars.to_string();

            let back: Variables = rendered
                .parse()
                .unwrap_or_else(|e| {
                    panic!("{value:?} rendered {rendered} and failed to parse: {e}")
                });
            assert_eq!(back.get("k"), Some(value), "rendered {rendered}");
            assert_eq!(
                back.get("after"),
                Some("sentinel"),
                "value {value:?} ate the next variable: {rendered}"
            );
        }
    }

    /// A variable block holds dialled numbers and passthrough header values, so a
    /// malformed part is reported by position.
    #[test]
    fn missing_equals_error_omits_the_fragment() {
        let msg = "{origination_caller_id_number=15551234567,15550009999}"
            .parse::<Variables>()
            .unwrap_err()
            .to_string();
        assert!(
            !msg.contains("15550009999"),
            "error quoted its input: {msg}"
        );
        assert!(
            msg.contains("variable 1"),
            "error does not name the part: {msg}"
        );
    }

    #[test]
    fn unknown_delimiters_error_omits_the_block() {
        let msg = "(origination_caller_id_number=15551234567)"
            .parse::<Variables>()
            .unwrap_err()
            .to_string();
        assert!(
            !msg.contains("15551234567"),
            "error quoted its input: {msg}"
        );
    }

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
}
