pub mod channel;
pub mod conference;
pub mod originate;

pub use channel::{
    UuidAnswer, UuidBridge, UuidDeflect, UuidGetVar, UuidHold, UuidKill, UuidSendDtmf, UuidSetVar,
    UuidTransfer,
};
pub use conference::{ConferenceDtmf, ConferenceHold, ConferenceMute, HoldAction, MuteAction};
pub use originate::{
    Application, ApplicationList, DialplanType, Endpoint, Originate, OriginateError, Variables,
    VariablesType,
};

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

/// Parse an application list string into individual applications.
///
/// Handles three formats:
/// - Bare extension: `"123"` → single app with name=extension, no args
/// - XML format: `"&app(args)"` → single app
/// - Inline format: `"app1:args1,app2:args2"` → multiple apps
pub fn parse_application_list(
    s: &str,
    dialplan: Option<&DialplanType>,
) -> Result<ApplicationList, OriginateError> {
    if matches!(dialplan, Some(DialplanType::Inline)) {
        let mut apps = Vec::new();
        for part in originate_split(s, ',')? {
            let (name, args) = part
                .split_once(':')
                .ok_or_else(|| {
                    OriginateError::ParseError(format!("invalid inline application: {}", part))
                })?;
            apps.push(Application::new(name, Some(args)));
        }
        Ok(ApplicationList(apps))
    } else if let Some(rest) = s.strip_prefix('&') {
        let rest = rest
            .strip_suffix(')')
            .ok_or_else(|| OriginateError::ParseError("missing closing paren".into()))?;
        let (name, args) = rest
            .split_once('(')
            .ok_or_else(|| OriginateError::ParseError("missing opening paren".into()))?;
        let args = if args.is_empty() { None } else { Some(args) };
        Ok(ApplicationList(vec![Application::new(name, args)]))
    } else {
        Ok(ApplicationList(vec![Application::new(s, None::<&str>)]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parse_application_list_bare_extension() {
        let list = parse_application_list("123", None).unwrap();
        assert_eq!(list.0[0].name, "123");
        assert!(list.0[0]
            .args
            .is_none());
    }

    #[test]
    fn parse_application_list_xml_no_args() {
        let list = parse_application_list("&conference()", None).unwrap();
        assert_eq!(list.0[0].name, "conference");
        assert!(list.0[0]
            .args
            .is_none());
    }

    #[test]
    fn parse_application_list_xml_with_args() {
        let list = parse_application_list("&conference(1)", None).unwrap();
        assert_eq!(
            list.0
                .len(),
            1
        );
        assert_eq!(list.0[0].name, "conference");
        assert_eq!(
            list.0[0]
                .args
                .as_deref(),
            Some("1")
        );
    }

    #[test]
    fn parse_application_list_two_inline_apps() {
        let list = parse_application_list(
            "conference:1,hangup:NORMAL_CLEARING",
            Some(&DialplanType::Inline),
        )
        .unwrap();
        assert_eq!(
            list.0
                .len(),
            2
        );
        assert_eq!(list.0[0].name, "conference");
        assert_eq!(
            list.0[0]
                .args
                .as_deref(),
            Some("1")
        );
        assert_eq!(list.0[1].name, "hangup");
        assert_eq!(
            list.0[1]
                .args
                .as_deref(),
            Some("NORMAL_CLEARING")
        );
    }
}
