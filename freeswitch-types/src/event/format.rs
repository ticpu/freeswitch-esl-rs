use std::fmt;
use std::str::FromStr;

/// Event format types supported by FreeSWITCH ESL
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum EventFormat {
    /// Plain text format (default)
    Plain,
    /// JSON format
    Json,
    /// XML format
    Xml,
}

impl EventFormat {
    /// Determine event format from a Content-Type header value.
    ///
    /// Returns `Err` for unrecognized content types to avoid silently
    /// misparsing events if FreeSWITCH adds a new format.
    pub fn from_content_type(ct: &str) -> Result<Self, ParseEventFormatError> {
        match ct {
            "text/event-json" => Ok(Self::Json),
            "text/event-xml" => Ok(Self::Xml),
            "text/event-plain" => Ok(Self::Plain),
            _ => Err(ParseEventFormatError(ct.to_string())),
        }
    }
}

impl fmt::Display for EventFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventFormat::Plain => write!(f, "plain"),
            EventFormat::Json => write!(f, "json"),
            EventFormat::Xml => write!(f, "xml"),
        }
    }
}

impl FromStr for EventFormat {
    type Err = ParseEventFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("plain") {
            Ok(Self::Plain)
        } else if s.eq_ignore_ascii_case("json") {
            Ok(Self::Json)
        } else if s.eq_ignore_ascii_case("xml") {
            Ok(Self::Xml)
        } else {
            Err(ParseEventFormatError(s.to_string()))
        }
    }
}

parse_error! { ParseEventFormatError("event format"); }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_format_from_str() {
        assert_eq!("plain".parse::<EventFormat>(), Ok(EventFormat::Plain));
        assert_eq!("json".parse::<EventFormat>(), Ok(EventFormat::Json));
        assert_eq!("xml".parse::<EventFormat>(), Ok(EventFormat::Xml));
        assert!("foo"
            .parse::<EventFormat>()
            .is_err());
    }

    #[test]
    fn test_event_format_from_str_case_insensitive() {
        assert_eq!("PLAIN".parse::<EventFormat>(), Ok(EventFormat::Plain));
        assert_eq!("Json".parse::<EventFormat>(), Ok(EventFormat::Json));
        assert_eq!("XML".parse::<EventFormat>(), Ok(EventFormat::Xml));
        assert_eq!("Xml".parse::<EventFormat>(), Ok(EventFormat::Xml));
    }

    #[test]
    fn test_event_format_from_content_type() {
        assert_eq!(
            EventFormat::from_content_type("text/event-json"),
            Ok(EventFormat::Json)
        );
        assert_eq!(
            EventFormat::from_content_type("text/event-xml"),
            Ok(EventFormat::Xml)
        );
        assert_eq!(
            EventFormat::from_content_type("text/event-plain"),
            Ok(EventFormat::Plain)
        );
        assert!(EventFormat::from_content_type("unknown").is_err());
    }
}
