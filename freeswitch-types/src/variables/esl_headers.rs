//! [`EslHeaders`] — a flat header store that understands FreeSWITCH's
//! transport encodings.
//!
//! FreeSWITCH's ESL wire format carries headers and channel variables in the
//! same flat key-value namespace, but with two transport quirks that plain
//! RFC-SIP parsers don't account for:
//!
//! - **ARRAY encoding** — repeating SIP headers arrive as
//!   `ARRAY::value1|:value2|:value3` (see [`EslArray`]).
//! - **Bracket wrapping** — some log-sourced headers arrive as `[value]`.
//!
//! Routing those values through the default [`SipHeaderLookup`] methods
//! produces parse errors because the string doesn't match RFC syntax.
//! [`EslHeaders`] wraps an [`IndexMap<String, String>`] and overrides the
//! relevant `SipHeaderLookup` methods to strip both quirks before parsing.
//! The design-rationale doc §"EslHeaders: making the transport boundary
//! visible" explains the layering.

use indexmap::IndexMap;
use sip_header::{HistoryInfo, HistoryInfoError, SipHeaderLookup, UriInfo, UriInfoError};

use crate::lookup::HeaderLookup;
use crate::variables::{EslArray, EslArrayError};

/// A flat header store that decodes FreeSWITCH ARRAY and bracket encoding
/// when answering typed SIP header queries.
///
/// Construct with [`EslHeaders::new`] or [`EslHeaders::from_map`]. Use it
/// anywhere a [`HeaderLookup`] or [`SipHeaderLookup`] implementor is
/// expected:
///
/// ```
/// use freeswitch_types::{EslHeaders, HeaderLookup};
/// use freeswitch_types::sip_header::SipHeaderLookup;
///
/// let mut h = EslHeaders::new();
/// h.insert("Unique-ID", "abc-123");
/// h.insert("Call-Info", "ARRAY::<sip:a@example.com>;purpose=icon|:<sip:b@example.com>");
///
/// assert_eq!(h.header_str("Unique-ID"), Some("abc-123"));
/// let ci = h.call_info().unwrap().unwrap();
/// assert_eq!(ci.entries().len(), 2);
/// ```
///
/// `HeaderLookup` delegates straight to the map; `SipHeaderLookup` methods
/// that parse RFC-structured values (`call_info`, `history_info`, and any
/// future multi-value parsers) first peel the FreeSWITCH encoding and then
/// hand pre-split entries to `sip-header`. Non-parsing lookups
/// (`sip_header_str`, `sip_header`) return the raw stored value untouched —
/// the caller sees exactly what FreeSWITCH put on the wire.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EslHeaders(IndexMap<String, String>);

impl EslHeaders {
    /// Create an empty store.
    pub fn new() -> Self {
        Self(IndexMap::new())
    }

    /// Wrap an existing map.
    pub fn from_map(map: IndexMap<String, String>) -> Self {
        Self(map)
    }

    /// Access the underlying map.
    pub fn as_map(&self) -> &IndexMap<String, String> {
        &self.0
    }

    /// Consume and return the underlying map.
    pub fn into_map(self) -> IndexMap<String, String> {
        self.0
    }

    /// Insert a header, replacing any existing entry at the same key.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.0
            .insert(key.into(), value.into());
    }

    /// Remove a header by key.
    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.0
            .shift_remove(key)
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.0
            .len()
    }

    /// `true` if there are no entries.
    pub fn is_empty(&self) -> bool {
        self.0
            .is_empty()
    }
}

impl From<IndexMap<String, String>> for EslHeaders {
    fn from(map: IndexMap<String, String>) -> Self {
        Self(map)
    }
}

/// Strip a single pair of outer `[...]` brackets from FreeSWITCH log-derived
/// header values. If the value is not bracket-wrapped, returns it unchanged.
fn strip_brackets(s: &str) -> &str {
    if let Some(inner) = s.strip_prefix('[') {
        if let Some(inner) = inner.strip_suffix(']') {
            return inner;
        }
    }
    s
}

impl EslHeaders {
    /// Parse a FreeSWITCH-transported SIP URI-list value into typed [`UriInfo`]
    /// entries, handling both `ARRAY::` encoding and bracket wrapping.
    ///
    /// Use this when you hold a *raw* value — e.g. the `sip_call_info` /
    /// `sip_alert_info` channel variable fetched over ESL — rather than a
    /// populated [`EslHeaders`]. It accepts any of the forms FreeSWITCH emits:
    ///
    /// - **Single RFC entry**: `<sip:a@example.test>;purpose=emergency-CallId`
    /// - **ARRAY encoding**: `ARRAY::<sip:a@example.test>;purpose=icon|:<sip:b@example.test>`
    /// - **Bracket-wrapped**: `[<sip:a@example.test>;purpose=icon]`
    ///
    /// This is the same decoding the [`call_info`](SipHeaderLookup::call_info)
    /// and [`alert_info`](SipHeaderLookup::alert_info) methods apply; iterate
    /// the result via `.entries()`.
    ///
    /// # Errors
    ///
    /// Returns [`UriInfoError`] if the value is malformed or if the `ARRAY::`
    /// structure is invalid. Structural `EslArrayError` cases (e.g.
    /// `TooManyItems`) are surfaced as [`UriInfoError::Malformed`] carrying
    /// the cause.
    ///
    /// # Example
    ///
    /// ```
    /// use freeswitch_types::EslHeaders;
    ///
    /// let value = "ARRAY::<urn:emergency:uid:callid:bcf.test>;purpose=emergency-CallId\
    ///              |:<urn:emergency:uid:incidentid:bcf.test>;purpose=emergency-IncidentId";
    /// let info = EslHeaders::parse_uri_info(value).unwrap();
    /// assert_eq!(info.entries().len(), 2);
    /// ```
    pub fn parse_uri_info(value: &str) -> Result<UriInfo, UriInfoError> {
        let value = strip_brackets(value);
        match EslArray::parse(value) {
            Ok(array) => UriInfo::from_entries(
                array
                    .items()
                    .iter()
                    .map(String::as_str),
            ),
            Err(EslArrayError::MissingPrefix) => UriInfo::parse(value),
            Err(other) => Err(UriInfoError::Malformed(other.to_string())),
        }
    }

    /// Parse a FreeSWITCH-transported `History-Info` value into a typed
    /// [`HistoryInfo`], handling both `ARRAY::` encoding and bracket wrapping.
    ///
    /// The raw-value counterpart to [`history_info`](SipHeaderLookup::history_info).
    ///
    /// # Errors
    ///
    /// Structural `EslArrayError` cases (e.g. `TooManyItems`) are surfaced as
    /// [`HistoryInfoError::Malformed`] carrying the cause.
    pub fn parse_history_info(value: &str) -> Result<HistoryInfo, HistoryInfoError> {
        let value = strip_brackets(value);
        match EslArray::parse(value) {
            Ok(array) => HistoryInfo::from_entries(
                array
                    .items()
                    .iter()
                    .map(String::as_str),
            ),
            Err(EslArrayError::MissingPrefix) => HistoryInfo::parse(value),
            Err(other) => Err(HistoryInfoError::Malformed(other.to_string())),
        }
    }
}

/// Internal: emits the [`SipHeaderLookup`] method overrides that peel
/// FreeSWITCH's ARRAY and bracket encoding before RFC parsing. Invoke inside
/// an `impl SipHeaderLookup for T` block. Not part of the stable API.
#[doc(hidden)]
#[macro_export]
macro_rules! esl_sip_header_overrides {
    () => {
        fn call_info(
            &self,
        ) -> Result<Option<$crate::sip_header::UriInfo>, $crate::sip_header::UriInfoError> {
            match self.sip_header($crate::sip_header::SipHeader::CallInfo) {
                Some(s) => $crate::variables::EslHeaders::parse_uri_info(s).map(Some),
                None => Ok(None),
            }
        }

        fn history_info(
            &self,
        ) -> Result<Option<$crate::sip_header::HistoryInfo>, $crate::sip_header::HistoryInfoError> {
            match self.sip_header($crate::sip_header::SipHeader::HistoryInfo) {
                Some(s) => $crate::variables::EslHeaders::parse_history_info(s).map(Some),
                None => Ok(None),
            }
        }

        fn alert_info(
            &self,
        ) -> Result<Option<$crate::sip_header::UriInfo>, $crate::sip_header::UriInfoError> {
            match self.sip_header($crate::sip_header::SipHeader::AlertInfo) {
                Some(s) => $crate::variables::EslHeaders::parse_uri_info(s).map(Some),
                None => Ok(None),
            }
        }
    };
}

impl SipHeaderLookup for EslHeaders {
    fn sip_header_str(&self, name: &str) -> Option<&str> {
        self.0
            .get(name)
            .map(|s| s.as_str())
    }

    crate::esl_sip_header_overrides!();
}

impl HeaderLookup for EslHeaders {
    fn header_str(&self, name: &str) -> Option<&str> {
        self.0
            .get(name)
            .map(|s| s.as_str())
    }

    fn variable_str(&self, name: &str) -> Option<&str> {
        self.0
            .get(&format!("{}{name}", crate::VARIABLE_PREFIX))
            .map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::headers::EventHeader;

    #[test]
    fn header_str_passthrough() {
        let mut h = EslHeaders::new();
        h.insert("Unique-ID", "abc-123");
        assert_eq!(h.header_str("Unique-ID"), Some("abc-123"));
    }

    #[test]
    fn variable_str_prepends_variable_prefix() {
        let mut h = EslHeaders::new();
        h.insert("variable_sip_call_id", "call-1");
        assert_eq!(h.variable_str("sip_call_id"), Some("call-1"));
        assert_eq!(h.variable_str("missing"), None);
    }

    #[test]
    fn call_info_single_value_rfc() {
        let mut h = EslHeaders::new();
        h.insert(
            "Call-Info",
            "<sip:alice@example.com>;purpose=emergency-CallId",
        );
        let ci = h
            .call_info()
            .unwrap()
            .expect("present");
        assert_eq!(
            ci.entries()
                .len(),
            1
        );
        assert_eq!(ci.entries()[0].purpose(), Some("emergency-CallId"));
    }

    #[test]
    fn call_info_array_encoding() {
        let mut h = EslHeaders::new();
        h.insert(
            "Call-Info",
            "ARRAY::<sip:a@example.com>;purpose=icon|:<sip:b@example.com>;purpose=info",
        );
        let ci = h
            .call_info()
            .unwrap()
            .expect("present");
        assert_eq!(
            ci.entries()
                .len(),
            2
        );
        assert_eq!(ci.entries()[0].purpose(), Some("icon"));
        assert_eq!(ci.entries()[1].purpose(), Some("info"));
    }

    #[test]
    fn call_info_bracket_wrapped() {
        let mut h = EslHeaders::new();
        h.insert(
            "Call-Info",
            "[<sip:alice@example.com>;purpose=emergency-CallId]",
        );
        let ci = h
            .call_info()
            .unwrap()
            .expect("present");
        assert_eq!(
            ci.entries()
                .len(),
            1
        );
    }

    #[test]
    fn call_info_absent_is_ok_none() {
        let h = EslHeaders::new();
        assert!(h
            .call_info()
            .unwrap()
            .is_none());
    }

    #[test]
    fn history_info_array_encoding() {
        let mut h = EslHeaders::new();
        h.insert(
            "History-Info",
            "ARRAY::<sip:a@example.com>;index=1|:<sip:b@example.com>;index=1.1",
        );
        let hi = h
            .history_info()
            .unwrap()
            .expect("present");
        assert_eq!(
            hi.entries()
                .len(),
            2
        );
    }

    #[test]
    fn header_lookup_typed_accessors() {
        let mut h = EslHeaders::new();
        h.insert(EventHeader::UniqueId.as_str(), "uuid-1");
        h.insert(EventHeader::ChannelName.as_str(), "sofia/a/b");
        assert_eq!(h.unique_id(), Some("uuid-1"));
        assert_eq!(h.channel_name(), Some("sofia/a/b"));
    }

    #[test]
    fn parse_uri_info_array_form() {
        let value = "ARRAY::<urn:emergency:uid:callid:bcf.example.test>;purpose=emergency-CallId\
                     |:<urn:emergency:uid:incidentid:bcf.example.test>;purpose=emergency-IncidentId\
                     |:<https://eido.example.test/v1/bcf.example.test/abc?test-call=true>;purpose=emergency-eido";
        let info = EslHeaders::parse_uri_info(value).expect("parse ARRAY form");
        let entries = info.entries();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].purpose(), Some("emergency-CallId"));
        assert_eq!(entries[1].purpose(), Some("emergency-IncidentId"));
        assert_eq!(entries[2].purpose(), Some("emergency-eido"));
    }

    #[test]
    fn parse_uri_info_single_entry() {
        let value = "<urn:emergency:uid:callid:test>;purpose=emergency-CallId";
        let info = EslHeaders::parse_uri_info(value).expect("parse single entry");
        assert_eq!(
            info.entries()
                .len(),
            1
        );
        assert_eq!(info.entries()[0].purpose(), Some("emergency-CallId"));
    }

    #[test]
    fn parse_uri_info_empty_value() {
        // Empty string is an error (no angle brackets)
        let result = EslHeaders::parse_uri_info("");
        assert!(result.is_err());
    }

    #[test]
    fn parse_uri_info_malformed_no_panic() {
        // sip-header UriInfo is lenient - this parses without angle brackets
        let info = EslHeaders::parse_uri_info("sip:bare@example.test").expect("lenient parse");
        assert_eq!(
            info.entries()
                .len(),
            1
        );
    }

    #[test]
    fn parse_uri_info_bracket_wrapped() {
        let value = "[<urn:emergency:uid:callid:test>;purpose=emergency-CallId]";
        let info = EslHeaders::parse_uri_info(value).expect("parse bracket-wrapped");
        assert_eq!(
            info.entries()
                .len(),
            1
        );
        assert_eq!(info.entries()[0].purpose(), Some("emergency-CallId"));
    }

    #[test]
    fn parse_uri_info_structural_failure_is_malformed() {
        let value = format!(
            "ARRAY::{}",
            vec!["<sip:a@example.com>"; crate::variables::MAX_ARRAY_ITEMS + 1].join("|:")
        );
        let err = EslHeaders::parse_uri_info(&value).expect_err("over-limit array must fail");
        assert!(matches!(err, UriInfoError::Malformed(_)), "got {err:?}");
    }

    #[test]
    fn parse_history_info_structural_failure_is_malformed() {
        let value = format!(
            "ARRAY::{}",
            vec!["<sip:a@example.com>;index=1"; crate::variables::MAX_ARRAY_ITEMS + 1].join("|:")
        );
        let err = EslHeaders::parse_history_info(&value).expect_err("over-limit array must fail");
        assert!(matches!(err, HistoryInfoError::Malformed(_)), "got {err:?}");
    }

    #[test]
    fn parse_history_info_array_form() {
        let value = "ARRAY::<sip:a@example.com>;index=1|:<sip:b@example.com>;index=1.1";
        let info = EslHeaders::parse_history_info(value).expect("parse ARRAY form");
        assert_eq!(
            info.entries()
                .len(),
            2
        );
    }
}
