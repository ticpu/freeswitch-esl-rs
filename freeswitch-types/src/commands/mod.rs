//! Command string builders for [`api()`] and [`bgapi()`].
//!
//! [`api()`]: https://docs.rs/freeswitch-esl-tokio/latest/freeswitch_esl_tokio/connection/struct.EslClient.html#method.api
//! [`bgapi()`]: https://docs.rs/freeswitch-esl-tokio/latest/freeswitch_esl_tokio/connection/struct.EslClient.html#method.bgapi
//!
//! Each builder implements [`Display`](std::fmt::Display), producing the argument
//! string for the corresponding FreeSWITCH API command.  The builders perform
//! escaping and validation so callers don't need to worry about wire-format
//! details.

pub mod bridge;
pub mod channel;
pub mod conference;
pub mod endpoint;
pub mod originate;
pub mod variables;

pub use bridge::BridgeDialString;
pub use channel::{
    UuidAnswer, UuidBridge, UuidDeflect, UuidGetVar, UuidHold, UuidKill, UuidSendDtmf, UuidSetVar,
    UuidTransfer,
};
pub use conference::{
    ConferenceDtmf, ConferenceHold, ConferenceMute, HoldAction, MuteAction, ParseHoldActionError,
    ParseMuteActionError,
};
pub use endpoint::{
    AudioEndpoint, DialString, EndpointDisplay, ErrorEndpoint, GroupCall, GroupCallOrder,
    LoopbackEndpoint, ParseGroupCallOrderError, SofiaContact, SofiaEndpoint, SofiaGateway,
    UserEndpoint,
};
pub use originate::{
    Application, DialplanType, Endpoint, Originate, OriginateError, OriginateTarget,
    ParseDialplanTypeError, Variables, VariablesType,
};
pub use variables::{DialStringCarrier, VariablesDisplay};

/// Find the index of the closing bracket matching the opener at position 0.
///
/// Tracks nesting depth so that inner pairs of the same bracket type are
/// skipped. Returns `None` if the string never reaches depth 0.
pub(crate) fn find_matching_bracket(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    for (i, ch) in s.char_indices() {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Wrap a token in single quotes for originate command strings.
///
/// If `token` contains spaces, it is wrapped in `'...'` with any inner
/// single quotes escaped as `\'`.  Tokens without spaces are returned as-is.
pub fn originate_quote(token: &str) -> String {
    if token.contains(' ') {
        let escaped = token.replace('\'', "\\'");
        format!("'{}'", escaped)
    } else {
        token.to_string()
    }
}

/// Escape and single-quote a value for the argument string of `uuid_setvar`.
///
/// That command splits its arguments on spaces through `cleanup_separated_string`,
/// which honours `'` grouping and processes `\` escapes inside the quoted region,
/// so an unquoted value is silently truncated at its first space and a bare `'`
/// or `\` inside the quotes ends or eats a character. The inline originate
/// `{var=…}` block is a different carrier with different escaping.
pub fn quote_for_uuid_setvar(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        match ch {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// Strip single-quote wrapping added by [`originate_quote`].
///
/// If the token starts and ends with `'`, the outer quotes are removed
/// and `\'` sequences are unescaped back to `'`.
pub fn originate_unquote(token: &str) -> String {
    match token
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
    {
        Some(inner) => inner.replace("\\'", "'"),
        None => token.to_string(),
    }
}

/// Quote-aware tokenizer for originate command strings.
///
/// Splits `line` on `split_at` (default: space), respecting single-quote
/// pairing to avoid splitting inside quoted values. Backslash-escaped quotes
/// are not treated as quote boundaries.
///
/// Ported from Python `originate_split()`.
pub fn originate_split(line: &str, split_at: char) -> Result<Vec<String>, OriginateError> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut in_quote = false;
    let chars: Vec<char> = line
        .chars()
        .collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

        if ch == split_at
            && !in_quote
            && !token
                .trim()
                .is_empty()
        {
            tokens.push(
                token
                    .trim()
                    .to_string(),
            );
            token.clear();
            i += 1;
            continue;
        }

        if ch == '\'' && !(i > 0 && chars[i - 1] == '\\') {
            in_quote = !in_quote;
        }

        token.push(ch);
        i += 1;
    }

    if in_quote {
        return Err(OriginateError::UnclosedQuote(token));
    }

    let token = token
        .trim()
        .to_string();
    if !token.is_empty() {
        tokens.push(token);
    }

    Ok(tokens)
}

/// Parse the target argument of an originate command.
///
/// Determines whether the target is a dialplan extension or application(s):
/// - If dialplan is `Inline`: parse as inline apps → `InlineApplications`
/// - If string starts with `&`: parse as XML app → `Application`
/// - Otherwise: bare string → `Extension`
pub fn parse_originate_target(
    s: &str,
    dialplan: Option<&DialplanType>,
) -> Result<OriginateTarget, OriginateError> {
    if matches!(dialplan, Some(DialplanType::Inline)) {
        let mut apps = Vec::new();
        for part in originate_split(s, ',')? {
            let (name, args) = match part.split_once(':') {
                Some((n, "")) => (n, None),
                Some((n, a)) => (n, Some(a)),
                None => (part.as_str(), None),
            };
            apps.push(Application::new(name, args));
        }
        Ok(OriginateTarget::InlineApplications(apps))
    } else if let Some(rest) = s.strip_prefix('&') {
        let rest = rest
            .strip_suffix(')')
            .ok_or_else(|| OriginateError::ParseError("missing closing paren".into()))?;
        let (name, args) = rest
            .split_once('(')
            .ok_or_else(|| OriginateError::ParseError("missing opening paren".into()))?;
        let args = if args.is_empty() { None } else { Some(args) };
        Ok(OriginateTarget::Application(Application::new(name, args)))
    } else {
        Ok(OriginateTarget::Extension(s.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_matching_bracket_simple() {
        assert_eq!(find_matching_bracket("{abc}", '{', '}'), Some(4));
    }

    #[test]
    fn find_matching_bracket_nested() {
        assert_eq!(find_matching_bracket("{a={b}}", '{', '}'), Some(6));
    }

    #[test]
    fn find_matching_bracket_unclosed() {
        assert_eq!(find_matching_bracket("{a={b}", '{', '}'), None);
    }

    #[test]
    fn find_matching_bracket_angle() {
        assert_eq!(find_matching_bracket("<a=<b>>rest", '<', '>'), Some(6));
    }

    #[test]
    fn split_with_quotes_ignores_spaces_inside() {
        let result =
            originate_split("originate {test='variable with quote'}sofia/test 123", ' ').unwrap();
        assert_eq!(result[0], "originate");
        assert_eq!(result[1], "{test='variable with quote'}sofia/test");
        assert_eq!(result[2], "123");
    }

    #[test]
    fn split_missing_quote_returns_error() {
        let result = originate_split(
            "originate {test='variable with missing quote}sofia/test 123",
            ' ',
        );
        assert!(result.is_err());
    }

    #[test]
    fn split_string_starting_ending_with_quote() {
        let result = originate_split("'this is test'", ' ').unwrap();
        assert_eq!(result[0], "'this is test'");
    }

    #[test]
    fn split_comma_separated() {
        let result = originate_split("item1,item2", ',').unwrap();
        assert_eq!(result[0], "item1");
        assert_eq!(result[1], "item2");
    }

    #[test]
    fn split_with_escaped_quotes() {
        let result = originate_split(
            "originate {test='variable with quote'}sofia/test let\\'s add a quote",
            ' ',
        )
        .unwrap();
        assert_eq!(result[0], "originate");
        assert_eq!(result[1], "{test='variable with quote'}sofia/test");
        assert_eq!(result[2], "let\\'s");
        assert_eq!(result[3], "add");
        assert_eq!(result[4], "a");
        assert_eq!(result[5], "quote");
    }

    #[test]
    fn quote_without_spaces_returns_as_is() {
        assert_eq!(originate_quote("&park()"), "&park()");
    }

    #[test]
    fn quote_with_spaces_wraps_in_single_quotes() {
        assert_eq!(
            originate_quote("&socket(127.0.0.1:8040 async full)"),
            "'&socket(127.0.0.1:8040 async full)'"
        );
    }

    #[test]
    fn quote_with_single_quote_and_spaces_escapes_quote() {
        assert_eq!(
            originate_quote("&playback(it's a test file)"),
            "'&playback(it\\'s a test file)'"
        );
    }

    #[test]
    fn unquote_non_quoted_returns_as_is() {
        assert_eq!(originate_unquote("&park()"), "&park()");
    }

    #[test]
    fn unquote_strips_outer_quotes() {
        assert_eq!(
            originate_unquote("'&socket(127.0.0.1:8040 async full)'"),
            "&socket(127.0.0.1:8040 async full)"
        );
    }

    #[test]
    fn unquote_unescapes_inner_quotes() {
        assert_eq!(
            originate_unquote("'&playback(it\\'s a test file)'"),
            "&playback(it's a test file)"
        );
    }

    #[test]
    fn quote_unquote_round_trip() {
        let original = "&socket(127.0.0.1:8040 async full)";
        assert_eq!(originate_unquote(&originate_quote(original)), original);
    }

    #[test]
    fn quote_unquote_round_trip_with_inner_quote() {
        let original = "&playback(it's a test file)";
        assert_eq!(originate_unquote(&originate_quote(original)), original);
    }

    #[test]
    fn split_multiple_consecutive_spaces() {
        let result = originate_split("originate  sofia/test  123", ' ').unwrap();
        // Multiple consecutive spaces produce empty tokens that are trimmed/skipped
        assert_eq!(result[0], "originate");
        assert_eq!(result[1], "sofia/test");
        assert_eq!(result[2], "123");
    }

    #[test]
    fn split_leading_trailing_spaces() {
        let result = originate_split("  originate sofia/test  ", ' ').unwrap();
        assert_eq!(result[0], "originate");
        assert_eq!(result[1], "sofia/test");
    }

    #[test]
    fn parse_target_bare_extension() {
        let target = parse_originate_target("123", None).unwrap();
        assert!(matches!(target, OriginateTarget::Extension(ref e) if e == "123"));
    }

    #[test]
    fn parse_target_xml_no_args() {
        let target = parse_originate_target("&conference()", None).unwrap();
        if let OriginateTarget::Application(app) = target {
            assert_eq!(app.name(), "conference");
            assert!(app
                .args()
                .is_none());
        } else {
            panic!("expected Application");
        }
    }

    #[test]
    fn parse_target_xml_with_args() {
        let target = parse_originate_target("&conference(1)", None).unwrap();
        if let OriginateTarget::Application(app) = target {
            assert_eq!(app.name(), "conference");
            assert_eq!(app.args(), Some("1"));
        } else {
            panic!("expected Application");
        }
    }

    #[test]
    fn parse_target_two_inline_apps() {
        let target = parse_originate_target(
            "conference:1,hangup:NORMAL_CLEARING",
            Some(&DialplanType::Inline),
        )
        .unwrap();
        if let OriginateTarget::InlineApplications(apps) = target {
            assert_eq!(apps.len(), 2);
            assert_eq!(apps[0].name(), "conference");
            assert_eq!(apps[0].args(), Some("1"));
            assert_eq!(apps[1].name(), "hangup");
            assert_eq!(apps[1].args(), Some("NORMAL_CLEARING"));
        } else {
            panic!("expected InlineApplications");
        }
    }

    #[test]
    fn parse_target_inline_bare_name() {
        let target = parse_originate_target("hangup", Some(&DialplanType::Inline)).unwrap();
        if let OriginateTarget::InlineApplications(apps) = target {
            assert_eq!(apps.len(), 1);
            assert_eq!(apps[0].name(), "hangup");
            assert!(apps[0]
                .args()
                .is_none());
        } else {
            panic!("expected InlineApplications");
        }
    }

    #[test]
    fn parse_target_inline_mixed_bare_and_args() {
        let target =
            parse_originate_target("park,hangup:NORMAL_CLEARING", Some(&DialplanType::Inline))
                .unwrap();
        if let OriginateTarget::InlineApplications(apps) = target {
            assert_eq!(apps.len(), 2);
            assert_eq!(apps[0].name(), "park");
            assert!(apps[0]
                .args()
                .is_none());
            assert_eq!(apps[1].name(), "hangup");
            assert_eq!(apps[1].args(), Some("NORMAL_CLEARING"));
        } else {
            panic!("expected InlineApplications");
        }
    }

    #[test]
    fn setvar_quoting_escapes_for_the_setvar_tokenizer() {
        let cases: &[(&str, &str)] = &[
            ("PCMU,PCMA", "'PCMU,PCMA'"),
            ("mode-set=0; octet-align=1", "'mode-set=0; octet-align=1'"),
            ("a'b", "'a\\'b'"),
            ("a\\b", "'a\\\\b'"),
        ];
        for (value, expected) in cases {
            assert_eq!(&quote_for_uuid_setvar(value), expected);
        }
    }

    #[test]
    fn parse_target_inline_trailing_colon_collapses_to_none() {
        let target = parse_originate_target("park:", Some(&DialplanType::Inline)).unwrap();
        if let OriginateTarget::InlineApplications(apps) = target {
            assert!(apps[0]
                .args()
                .is_none());
        } else {
            panic!("expected InlineApplications");
        }
    }
}
