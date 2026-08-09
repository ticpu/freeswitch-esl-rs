//! ESL protocol parsing and message handling

use crate::{
    buffer::EslBuffer,
    command::EslResponse,
    constants::{
        CONTENT_TYPE_API_RESPONSE, CONTENT_TYPE_AUTH_REQUEST, CONTENT_TYPE_COMMAND_REPLY,
        CONTENT_TYPE_LOG_DATA, CONTENT_TYPE_TEXT_EVENT_JSON, CONTENT_TYPE_TEXT_EVENT_PLAIN,
        CONTENT_TYPE_TEXT_EVENT_XML, HEADER_CONTENT_LENGTH, HEADER_CONTENT_TYPE, HEADER_TERMINATOR,
        MAX_MESSAGE_SIZE,
    },
    error::{EslError, EslResult},
    event::{EslEvent, EslEventType, EventFormat},
    headers::{normalize_header_key, EventHeader},
    LossyValue, LossyValues,
};
use indexmap::IndexMap;
use percent_encoding::percent_decode_str;

/// ESL message types.
///
/// Marked `#[non_exhaustive]` because new ESL `Content-Type` values added
/// upstream will turn into new variants here rather than a catch-all
/// `Unknown` — unrecognized content-types are now hard protocol errors,
/// surfaced by [`MessageType::from_content_type`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum MessageType {
    /// Authentication request from server
    AuthRequest,
    /// Command reply
    CommandReply,
    /// API response
    ApiResponse,
    /// Event message
    Event,
    /// Disconnect notice
    Disconnect,
    /// Connection rejected by ACL (text/rude-rejection)
    RudeRejection,
}

impl MessageType {
    /// Parse message type from a `Content-Type` header value.
    ///
    /// Returns [`EslError::ProtocolError`] if the value is not one of
    /// the recognized wire content-types — there is no `Unknown` fallback
    /// since correctness over recovery is preferred (see project
    /// CLAUDE.md). Mirrors the precedent set for `EventFormat`.
    pub fn from_content_type(content_type: &str) -> EslResult<Self> {
        match content_type {
            CONTENT_TYPE_AUTH_REQUEST => Ok(MessageType::AuthRequest),
            CONTENT_TYPE_COMMAND_REPLY => Ok(MessageType::CommandReply),
            CONTENT_TYPE_API_RESPONSE => Ok(MessageType::ApiResponse),
            CONTENT_TYPE_TEXT_EVENT_PLAIN
            | CONTENT_TYPE_TEXT_EVENT_JSON
            | CONTENT_TYPE_TEXT_EVENT_XML
            | CONTENT_TYPE_LOG_DATA => Ok(MessageType::Event),
            "text/disconnect-notice" => Ok(MessageType::Disconnect),
            "text/rude-rejection" => Ok(MessageType::RudeRejection),
            other => Err(EslError::protocol_error(format!(
                "Unrecognized Content-Type: {other}"
            ))),
        }
    }
}

/// Parsed ESL message.
///
/// This is an internal protocol-parsing intermediate, not part of the
/// publicly-stable surface. It is constructed by the parser and consumed
/// by the connection module within this crate; public visibility exists
/// for module organization only. External API stability comes from
/// [`EslResponse`] and [`crate::event::EslEvent`], not from `EslMessage` —
/// which is why this struct intentionally does not carry `#[non_exhaustive]`
/// despite having public fields.
#[derive(Debug, Clone)]
pub struct EslMessage {
    /// Message type
    pub message_type: MessageType,
    /// Message headers
    pub headers: IndexMap<String, String>,
    /// Message body (optional)
    pub body: Option<String>,
    /// Exact wire bytes of a body that was not valid UTF-8; `body` then
    /// holds the U+FFFD-substituted string. `None` in the normal case.
    pub raw_body: Option<Vec<u8>>,
    /// Header keys whose percent-decoded value was not valid UTF-8 and was
    /// decoded lossily. Empty for framing-only envelopes; populated for
    /// serialized responses such as the outbound `connect` channel data.
    pub lossy_values: LossyValues,
}

impl EslMessage {
    /// Create new message
    pub fn new(
        message_type: MessageType,
        headers: IndexMap<String, String>,
        body: Option<String>,
    ) -> Self {
        Self {
            message_type,
            headers,
            body,
            raw_body: None,
            lossy_values: LossyValues::default(),
        }
    }

    /// Attach the lossy-decode signal recorded while parsing the headers.
    pub fn with_lossy_values(mut self, lossy_values: LossyValues) -> Self {
        self.lossy_values = lossy_values;
        self
    }

    /// Convert to EslResponse
    pub fn into_response(self) -> EslResponse {
        EslResponse::new(self.headers, self.body)
            .with_lossy_values(self.lossy_values)
            .with_raw_body(self.raw_body)
    }
}

/// Parser state for handling incomplete messages
#[derive(Debug)]
enum ParseState {
    WaitingForHeaders,
    WaitingForBody {
        message_type: MessageType,
        headers: IndexMap<String, String>,
        body_length: usize,
    },
}

/// ESL protocol parser
pub struct EslParser {
    buffer: EslBuffer,
    state: ParseState,
    strict_header_utf8: bool,
}

impl EslParser {
    /// Create new parser
    pub fn new() -> Self {
        Self {
            buffer: EslBuffer::new(),
            state: ParseState::WaitingForHeaders,
            strict_header_utf8: false,
        }
    }

    /// Set strict UTF-8 validation on event-body header values.
    pub fn with_strict_header_utf8(mut self, strict: bool) -> Self {
        self.strict_header_utf8 = strict;
        self
    }

    /// Unconsumed bytes remaining in the parser buffer.
    pub fn remaining_bytes(&self) -> &[u8] {
        self.buffer
            .data()
    }

    /// Returns `true` if the parser is between messages (not mid-body).
    pub fn is_waiting_for_headers(&self) -> bool {
        matches!(self.state, ParseState::WaitingForHeaders)
    }

    /// Discard all buffered data and reset to `WaitingForHeaders`.
    ///
    /// Used after salvaging a truncated auth response where the data
    /// was parsed externally via `parse_headers()`.
    pub(crate) fn drain_buffer(&mut self) {
        debug_assert!(
            self.is_waiting_for_headers(),
            "drain_buffer called outside WaitingForHeaders state"
        );
        let remaining = self
            .buffer
            .len();
        if remaining > 0 {
            self.buffer
                .advance(remaining)
                .expect(
                    "advance(remaining) is in bounds: remaining was just measured by buffer.len()",
                );
            self.buffer
                .compact();
        }
    }

    /// Add data to the parser buffer
    pub fn add_data(&mut self, data: &[u8]) -> EslResult<()> {
        self.buffer
            .extend_from_slice(data);
        self.buffer
            .check_size_limits()?;
        Ok(())
    }

    /// Try to parse a complete message from the buffer
    pub fn parse_message(&mut self) -> EslResult<Option<EslMessage>> {
        // Take ownership of the state so the WaitingForBody arm can move its
        // fields into the finished message without cloning. Every path below
        // either restores the taken state or assigns the correct next one.
        match std::mem::replace(&mut self.state, ParseState::WaitingForHeaders) {
            ParseState::WaitingForHeaders => {
                // Check if we have complete headers
                let terminator = HEADER_TERMINATOR.as_bytes();

                if let Some(headers_data) = self
                    .buffer
                    .extract_until_pattern(terminator)
                {
                    // Compact buffer to free consumed header data
                    self.buffer
                        .compact();

                    // Parse headers
                    let headers_str = String::from_utf8(headers_data)
                        .map_err(|_| EslError::protocol_error("Invalid UTF-8 in headers"))?;

                    let (headers, lossy_values) = self.parse_headers(&headers_str)?;

                    // Every ESL message must have Content-Type. Missing means
                    // protocol desync (e.g. from a corrupted Content-Length).
                    let content_type = headers
                        .get(HEADER_CONTENT_TYPE)
                        .ok_or_else(|| {
                            EslError::protocol_error(
                                "Missing Content-Type header -- likely protocol desync",
                            )
                        })?;
                    let message_type = MessageType::from_content_type(content_type)?;

                    // Check if we need a body
                    if let Some(length_str) = headers.get(HEADER_CONTENT_LENGTH) {
                        let length: usize = length_str
                            .parse()
                            .map_err(|_| EslError::InvalidHeader {
                                header: format!("Content-Length: {}", length_str),
                            })?;

                        // Validate message size to prevent protocol errors or memory exhaustion
                        if length > MAX_MESSAGE_SIZE {
                            return Err(EslError::protocol_error(format!(
                                "Message too large: Content-Length {} exceeds limit {}. Protocol error or corrupted data.",
                                length, MAX_MESSAGE_SIZE
                            )));
                        }

                        if length > 0 {
                            // Body-bearing messages have framing-only envelopes
                            // (Content-Length/Content-Type/Reply-Text), which
                            // FreeSWITCH never percent-encodes, so no value here
                            // can decode lossily.
                            debug_assert!(
                                lossy_values.is_empty(),
                                "framing envelope produced a lossy value"
                            );
                            // Transition to waiting for body
                            self.state = ParseState::WaitingForBody {
                                message_type,
                                headers,
                                body_length: length,
                            };
                            // Try to parse body immediately
                            self.parse_message()
                        } else {
                            // No body needed, complete message
                            let message = EslMessage::new(message_type, headers, None)
                                .with_lossy_values(lossy_values);
                            self.state = ParseState::WaitingForHeaders;
                            Ok(Some(message))
                        }
                    } else {
                        // No Content-Length header, complete message without body.
                        // The outbound `connect` response arrives this way: a
                        // serialized event whose channel-data values ARE
                        // percent-encoded, so lossy_values may be populated.
                        let message = EslMessage::new(message_type, headers, None)
                            .with_lossy_values(lossy_values);
                        self.state = ParseState::WaitingForHeaders;
                        Ok(Some(message))
                    }
                } else {
                    // No complete headers yet
                    Ok(None)
                }
            }
            ParseState::WaitingForBody {
                message_type,
                headers,
                body_length,
            } => {
                if self
                    .buffer
                    .len()
                    < body_length
                {
                    self.state = ParseState::WaitingForBody {
                        message_type,
                        headers,
                        body_length,
                    };
                    return Ok(None);
                }
                let body_data = self
                    .buffer
                    .extract_bytes(body_length)
                    .expect("body_length <= buffer.len(): verified above");
                self.buffer
                    .compact();
                // Content-Length frames the body in bytes, so non-UTF-8 here
                // is no desync: raw payloads (sendevent bodies, api output)
                // arrive un-encoded. Decode lossily and keep the wire bytes
                // as raw_body, unless strict mode restores the hard fail.
                let mut raw_body = None;
                let body_str = match String::from_utf8(body_data) {
                    Ok(s) => s,
                    Err(e) if self.strict_header_utf8 => {
                        return Err(EslError::protocol_error(format!(
                            "Invalid UTF-8 in body: {}",
                            e.utf8_error()
                        )));
                    }
                    Err(e) => {
                        let bytes = e.into_bytes();
                        let lossy = String::from_utf8_lossy(&bytes).into_owned();
                        raw_body = Some(bytes);
                        lossy
                    }
                };
                let mut message = EslMessage::new(message_type, headers, Some(body_str));
                message.raw_body = raw_body;
                Ok(Some(message))
            }
        }
    }

    /// Parse a single `Key: value` line, stripping `\r` and normalizing the key.
    ///
    /// Returns the on-wire `(key, value)` without percent-decoding. Returns
    /// `Ok(None)` for blank lines (caller keeps looping), `Ok(Some((k, v)))`
    /// on success, `Err` when the line is non-blank but lacks a colon.
    fn parse_header_line(line: &str) -> EslResult<Option<(String, String)>> {
        let line = line
            .strip_suffix('\r')
            .unwrap_or(line);
        if line.is_empty() {
            return Ok(None);
        }
        let Some(colon_pos) = line.find(':') else {
            return Err(EslError::InvalidHeader {
                header: line.to_string(),
            });
        };
        let key = normalize_header_key(&line[..colon_pos]);
        let raw_value = line[colon_pos + 1..]
            .strip_prefix(' ')
            .unwrap_or(&line[colon_pos + 1..]);
        Ok(Some((key, raw_value.to_string())))
    }

    /// Parse a header block, percent-decoding each value.
    ///
    /// FreeSWITCH percent-encodes serialized-event values, including the
    /// outbound `connect` response channel data that flows through this path.
    /// Pushed-event framing headers (`Content-Length`/`Content-Type`) are not
    /// encoded, but decoding them is a no-op. Non-UTF-8 values are decoded
    /// lossily and recorded in the returned `LossyValues` unless
    /// `strict_header_utf8` is set, in which case they error.
    pub(crate) fn parse_headers(
        &self,
        headers_str: &str,
    ) -> EslResult<(IndexMap<String, String>, LossyValues)> {
        let mut headers = IndexMap::new();
        let mut lossy = LossyValues::default();
        for line in headers_str.lines() {
            if let Some((key, raw_value)) = Self::parse_header_line(line)? {
                let value = Self::decode_value(
                    &key,
                    &raw_value,
                    "header",
                    if self.strict_header_utf8 {
                        None
                    } else {
                        Some(&mut lossy)
                    },
                )?;
                headers.insert(key, value);
            }
        }
        Ok((headers, lossy))
    }

    /// Parse event from message, handling different formats.
    ///
    /// log/data messages use single-level framing (metadata in outer envelope,
    /// raw log text as body) unlike normal events which use two-level framing.
    pub fn parse_event(&self, message: EslMessage, format: EventFormat) -> EslResult<EslEvent> {
        if message
            .headers
            .get(HEADER_CONTENT_TYPE)
            .map(|s| s.as_str())
            == Some(CONTENT_TYPE_LOG_DATA)
        {
            return Self::parse_log_event(message);
        }

        let event = match format {
            EventFormat::Plain => self.parse_plain_event(message),
            EventFormat::Json => self.parse_json_event(message),
            EventFormat::Xml => self.parse_xml_event(message),
            _ => {
                return Err(EslError::ProtocolError {
                    message: format!("unsupported event format: {format}"),
                })
            }
        }?;

        Ok(event)
    }

    /// Parse log/data message.
    ///
    /// FreeSWITCH log/data wire format uses single-level framing, unlike
    /// normal events. Log metadata (Log-Level, Log-File, etc.) lives in the
    /// outer envelope headers and the body is raw log text.
    fn parse_log_event(message: EslMessage) -> EslResult<EslEvent> {
        let mut event = EslEvent::new();
        for (key, value) in &message.headers {
            event.set_header(key.clone(), value.clone());
        }
        if let Some(body) = message.body {
            event.set_body(body);
        }
        if let Some(raw) = message.raw_body {
            event.set_raw_body(raw);
        }
        // Synthesize Event-Name so downstream event_type() resolves to
        // EslEventType::Log; FreeSWITCH does not include this header on
        // log/data envelopes.
        event.set_header(EventHeader::EventName.as_str(), EslEventType::Log.as_str());
        Ok(event)
    }

    /// Percent-decode a header value (FreeSWITCH percent-encodes serialized
    /// event values). `context` labels the error site (`"header"` for the
    /// envelope/response block, `"event header"` for an event body).
    ///
    /// - `lossy == None` (strict): invalid UTF-8 returns `InvalidUtf8InHeader`.
    /// - `lossy == Some` (lenient): invalid UTF-8 is decoded lossily (U+FFFD)
    ///   and the key + on-wire `raw_value` are recorded in the accumulator.
    fn decode_value(
        key: &str,
        raw_value: &str,
        context: &'static str,
        lossy: Option<&mut LossyValues>,
    ) -> EslResult<String> {
        match percent_decode_str(raw_value).decode_utf8() {
            Ok(cow) => Ok(cow.into_owned()),
            Err(source) => match lossy {
                None => Err(EslError::InvalidUtf8InHeader {
                    context,
                    key: key.to_string(),
                    source,
                }),
                Some(acc) => {
                    acc.push(LossyValue::new(key.to_string(), raw_value.to_string()));
                    Ok(percent_decode_str(raw_value)
                        .decode_utf8_lossy()
                        .into_owned())
                }
            },
        }
    }

    /// Parse plain text event
    ///
    /// FreeSWITCH text/event-plain wire format uses a two-part structure:
    /// - Outer envelope: Content-Length + Content-Type headers
    /// - Body: URL-encoded key: value lines (the actual event headers)
    ///
    /// If the event body itself contains a Content-Length, there's an inner
    /// body after the event headers.
    fn parse_plain_event(&self, mut message: EslMessage) -> EslResult<EslEvent> {
        if message.message_type != MessageType::Event {
            return Err(EslError::protocol_error("Not an event message"));
        }

        let body = message
            .body
            .as_deref()
            .ok_or_else(|| EslError::protocol_error("Plain event missing body"))?;

        let mut event = EslEvent::new();
        let mut lossy = LossyValues::default();

        // Split event body into headers and optional inner body.
        // Event headers are terminated by \n\n; anything after is the inner body.
        let (header_section, inner_body) = if let Some(pos) = body.find("\n\n") {
            (&body[..pos], Some(&body[pos + 2..]))
        } else {
            (body, None)
        };

        // Parse and decode event-body headers
        for line in header_section.lines() {
            if let Some((key, raw_value)) = Self::parse_header_line(line)? {
                let value = Self::decode_value(
                    &key,
                    &raw_value,
                    "event header",
                    if self.strict_header_utf8 {
                        None
                    } else {
                        Some(&mut lossy)
                    },
                )?;
                event.set_header(key, value);
            }
        }

        event.set_lossy_values(lossy);

        // If the event headers contain their own Content-Length, the inner body
        // is that many bytes after the header section
        if let Some(ib) = inner_body {
            if !ib.is_empty() {
                event.set_body(ib.to_string());
                if let Some(mut raw) = message
                    .raw_body
                    .take()
                {
                    // Event headers are percent-encoded ASCII and U+FFFD
                    // substitution never touches newline bytes, so the first
                    // \n\n falls at the same logical spot in bytes as in the
                    // decoded string: the tail is exactly the inner body.
                    if let Some(pos) = raw
                        .windows(2)
                        .position(|w| w == b"\n\n")
                    {
                        raw.drain(..pos + 2);
                        event.set_raw_body(raw);
                    }
                }
            }
        }

        Ok(event)
    }

    /// Parse JSON event
    fn parse_json_event(&self, message: EslMessage) -> EslResult<EslEvent> {
        let body = message
            .body
            .ok_or_else(|| EslError::protocol_error("JSON event missing body"))?;

        // Parse JSON body
        let json_value: serde_json::Value = serde_json::from_str(&body)?;

        let serde_json::Value::Object(map) = json_value else {
            return Err(EslError::protocol_error("JSON event body is not an object"));
        };

        let mut event = EslEvent::new();
        for (key, value) in map {
            // FreeSWITCH puts the event body under a "_body" key in JSON events
            if key == "_body" {
                let body_str = match value {
                    serde_json::Value::String(s) => s,
                    _ => value.to_string(),
                };
                event.set_body(body_str);
                continue;
            }
            let value_str = match value {
                serde_json::Value::String(s) => s,
                _ => value.to_string(),
            };
            event.set_header(key, value_str);
        }

        Self::carry_lossy_signal(&mut event, message.lossy_values, message.raw_body);
        Ok(event)
    }

    /// Carry the envelope's lossy-decode signal onto the parsed event.
    ///
    /// JSON/XML cannot map wire bytes back to the decoded body, so `raw_body`
    /// is the whole envelope body — the signal (and source bytes) must still
    /// be observable per the warnings-ride-as-data policy.
    fn carry_lossy_signal(event: &mut EslEvent, lossy: LossyValues, raw_body: Option<Vec<u8>>) {
        if !lossy.is_empty() {
            event.set_lossy_values(lossy);
        }
        if let Some(raw) = raw_body {
            event.set_raw_body(raw);
        }
    }

    /// Map a `quick_xml::Error` to `EslError::XmlError` without leaking
    /// the dependency type into our public `From` impls.
    fn xml_err<E: std::fmt::Display>(e: E) -> EslError {
        EslError::XmlError(format!("XML parse error: {e}"))
    }

    /// Parse XML event using quick_xml.
    ///
    /// FreeSWITCH XML event format:
    /// ```xml
    /// <event>
    ///   <headers>
    ///     <Event-Name>HEARTBEAT</Event-Name>
    ///     <Core-UUID>abc-123</Core-UUID>
    ///   </headers>
    ///   <body>...</body>
    /// </event>
    /// ```
    fn parse_xml_event(&self, message: EslMessage) -> EslResult<EslEvent> {
        use quick_xml::events::Event as XmlEvent;
        use quick_xml::Reader;

        let EslMessage {
            body,
            raw_body,
            lossy_values,
            ..
        } = message;
        let body = body.ok_or_else(|| EslError::protocol_error("XML event missing body"))?;

        let mut reader = Reader::from_str(&body);
        let mut event = EslEvent::new();
        let mut in_headers = false;
        let mut current_tag: Option<String> = None;
        let mut in_body = false;
        // quick-xml splits text around entity references ("Smith &amp; Jones"
        // arrives as Text, GeneralRef, Text), so fragments accumulate here and
        // flush on the End tag.
        let mut text_buf = String::new();

        loop {
            match reader.read_event() {
                Ok(XmlEvent::Start(ref e)) => {
                    let tag = String::from_utf8_lossy(
                        e.name()
                            .as_ref(),
                    )
                    .to_string();
                    match tag.as_str() {
                        "headers" => in_headers = true,
                        "body" => in_body = true,
                        _ if in_headers => {
                            text_buf.clear();
                            current_tag = Some(tag);
                        }
                        _ => {}
                    }
                }
                Ok(XmlEvent::End(ref e)) => {
                    let tag = String::from_utf8_lossy(
                        e.name()
                            .as_ref(),
                    )
                    .to_string();
                    match tag.as_str() {
                        "headers" => in_headers = false,
                        "body" => {
                            if !text_buf.is_empty() {
                                event.set_body(std::mem::take(&mut text_buf));
                            }
                            in_body = false;
                        }
                        _ if in_headers => {
                            if let Some(ref tag) = current_tag {
                                if !text_buf.is_empty() {
                                    event.set_header(tag.clone(), std::mem::take(&mut text_buf));
                                }
                            }
                            current_tag = None;
                        }
                        _ => {}
                    }
                }
                Ok(XmlEvent::Text(ref e)) => {
                    let decoded = e
                        .decode()
                        .map_err(Self::xml_err)?;
                    if in_body || current_tag.is_some() {
                        text_buf.push_str(&decoded);
                    }
                }
                Ok(XmlEvent::GeneralRef(ref e)) if in_body || current_tag.is_some() => {
                    let resolved = Self::resolve_entity(e)?;
                    text_buf.push_str(&resolved);
                }
                Ok(XmlEvent::Eof) => break,
                Err(e) => return Err(Self::xml_err(e)),
                _ => {}
            }
        }

        Self::carry_lossy_signal(&mut event, lossy_values, raw_body);
        Ok(event)
    }

    /// Resolve an XML entity reference (`&name;` or `&#num;`) to its string value.
    fn resolve_entity(entity: &quick_xml::events::BytesRef<'_>) -> EslResult<String> {
        if let Some(ch) = entity
            .resolve_char_ref()
            .map_err(Self::xml_err)?
        {
            return Ok(ch.to_string());
        }
        let name = entity
            .decode()
            .map_err(Self::xml_err)?;
        match quick_xml::escape::resolve_xml_entity(&name) {
            Some(s) => Ok(s.to_string()),
            None => Err(EslError::protocol_error(format!(
                "unknown XML entity: &{};",
                name
            ))),
        }
    }
}

impl Default for EslParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lookup::HeaderLookup;

    #[test]
    fn test_parse_headers() {
        let parser = EslParser::new();
        let headers_str = "Content-Type: auth/request\r\nContent-Length: 0";
        let (headers, _lossy) = parser
            .parse_headers(headers_str)
            .unwrap();

        assert_eq!(
            headers
                .get("Content-Type")
                .map(|s| s.as_str()),
            Some("auth/request")
        );
        assert_eq!(
            headers
                .get("Content-Length")
                .map(|s| s.as_str()),
            Some("0")
        );
    }

    #[test]
    fn parsed_headers_preserve_insertion_order() {
        let parser = EslParser::new();
        let headers_str = "Alpha: 1\r\nBravo: 2\r\nCharlie: 3\r\nDelta: 4";
        let (headers, _lossy) = parser
            .parse_headers(headers_str)
            .unwrap();
        let keys: Vec<&str> = headers
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(keys, vec!["Alpha", "Bravo", "Charlie", "Delta"]);
    }

    #[test]
    fn test_parse_auth_request() {
        let mut parser = EslParser::new();
        let data = b"Content-Type: auth/request\n\n";

        parser
            .add_data(data)
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();

        assert_eq!(message.message_type, MessageType::AuthRequest);
        assert!(message
            .body
            .is_none());
    }

    #[test]
    fn test_parse_api_response() {
        let mut parser = EslParser::new();
        let data = b"Content-Type: api/response\nContent-Length: 2\n\nOK";

        parser
            .add_data(data)
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();

        assert_eq!(message.message_type, MessageType::ApiResponse);
        assert_eq!(message.body, Some("OK".to_string()));
    }

    #[test]
    fn test_parse_event_plain() {
        let mut parser = EslParser::new();
        // Correct two-part wire format: outer envelope + body with event headers
        let body = "Event-Name: CHANNEL_ANSWER\nUnique-ID: test-uuid\n\n";
        let envelope = format!(
            "Content-Length: {}\nContent-Type: text/event-plain\n\n",
            body.len()
        );
        let data = format!("{}{}", envelope, body);

        parser
            .add_data(data.as_bytes())
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();
        let event = parser
            .parse_event(message, EventFormat::Plain)
            .unwrap();

        assert_eq!(event.event_type(), Some(EslEventType::ChannelAnswer));
        assert_eq!(event.unique_id(), Some("test-uuid"));
    }

    #[test]
    fn test_parse_event_plain_percent_decoding() {
        let mut parser = EslParser::new();
        let body = "Event-Name: HEARTBEAT\nUp-Time: 0%20years%2C%200%20days\nEvent-Info: System%20Ready\n\n";
        let envelope = format!(
            "Content-Length: {}\nContent-Type: text/event-plain\n\n",
            body.len()
        );
        let data = format!("{}{}", envelope, body);

        parser
            .add_data(data.as_bytes())
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();
        let event = parser
            .parse_event(message, EventFormat::Plain)
            .unwrap();

        assert_eq!(event.event_type(), Some(EslEventType::Heartbeat));
        assert_eq!(event.header_str("Up-Time"), Some("0 years, 0 days"));
        assert_eq!(event.header_str("Event-Info"), Some("System Ready"));
    }

    #[test]
    fn test_parse_event_plain_with_inner_body() {
        let mut parser = EslParser::new();
        // Event with inner body (e.g., BACKGROUND_JOB result)
        let inner_body = "+OK Status\n";
        let event_headers = format!(
            "Event-Name: BACKGROUND_JOB\nJob-UUID: abc-123\nContent-Length: {}\n",
            inner_body.len()
        );
        let body = format!("{}\n{}", event_headers, inner_body);
        let envelope = format!(
            "Content-Length: {}\nContent-Type: text/event-plain\n\n",
            body.len()
        );
        let data = format!("{}{}", envelope, body);

        parser
            .add_data(data.as_bytes())
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();
        let event = parser
            .parse_event(message, EventFormat::Plain)
            .unwrap();

        assert_eq!(event.event_type(), Some(EslEventType::BackgroundJob));
        assert_eq!(event.header(EventHeader::JobUuid), Some("abc-123"));
        assert_eq!(event.body(), Some("+OK Status\n"));
    }

    /// log/data uses single-level framing: metadata in outer envelope,
    /// raw log text as body. This matches mod_event_socket.c's output.
    #[test]
    fn test_parse_log_data_event() {
        let mut parser = EslParser::new();
        let log_text = "2024-01-01 00:00:00.000000 [INFO] mod_sofia.c:1234 Registration ok\n";
        let envelope = format!(
            "Content-Type: log/data\nContent-Length: {}\nLog-Level: 6\nText-Channel: 0\nLog-File: mod_sofia.c\nLog-Func: sofia_reg_handle\nLog-Line: 1234\nUser-Data: \n\n{}",
            log_text.len(),
            log_text,
        );

        parser
            .add_data(envelope.as_bytes())
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();

        assert_eq!(message.message_type, MessageType::Event);

        let event = parser
            .parse_event(message, EventFormat::Plain)
            .unwrap();

        assert_eq!(event.event_type(), Some(EslEventType::Log));
        assert_eq!(event.header(EventHeader::LogLevel), Some("6"));
        assert_eq!(event.header_str("Content-Type"), Some("log/data"));
        assert_eq!(event.header_str("Log-File"), Some("mod_sofia.c"));
        assert_eq!(event.header_str("Log-Func"), Some("sofia_reg_handle"));
        assert_eq!(event.header_str("Log-Line"), Some("1234"));
        assert_eq!(event.body(), Some(log_text));
    }

    #[test]
    fn test_incomplete_message() {
        let mut parser = EslParser::new();
        let data = b"Content-Type: api/response\nContent-Length: 10\n\ntest"; // Only 4 bytes instead of 10

        parser
            .add_data(data)
            .unwrap();
        let result = parser
            .parse_message()
            .unwrap();

        assert!(result.is_none()); // Should return None for incomplete message
    }

    #[test]
    fn test_notify_in_event_with_pl_data() {
        let mut parser = EslParser::new();
        // NOTIFY_IN event with percent-encoded pl_data containing JSON
        let json_payload = r#"{"Invite":"INVITE urn:service:sos SIP/2.0","InviteTimestamp":"2025-01-15T12:00:00Z"}"#;
        let encoded_payload =
            percent_encoding::utf8_percent_encode(json_payload, percent_encoding::NON_ALPHANUMERIC)
                .to_string();
        let body = format!(
            "Event-Name: NOTIFY_IN\nevent: emergency-AbandonedCall\npl_data: {}\nsip_content_type: application%2Fjson\ngateway_name: ng911-bcf\n\n",
            encoded_payload
        );
        let envelope = format!(
            "Content-Length: {}\nContent-Type: text/event-plain\n\n",
            body.len()
        );
        let data = format!("{}{}", envelope, body);

        parser
            .add_data(data.as_bytes())
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();
        let event = parser
            .parse_event(message, EventFormat::Plain)
            .unwrap();

        assert_eq!(event.event_type(), Some(EslEventType::NotifyIn));
        assert_eq!(event.header_str("event"), Some("emergency-AbandonedCall"));
        // pl_data must be percent-decoded back to raw JSON
        assert_eq!(event.header_str("pl_data"), Some(json_payload));
        assert_eq!(
            event.header_str("sip_content_type"),
            Some("application/json")
        );
        assert_eq!(event.header_str("gateway_name"), Some("ng911-bcf"));
    }

    #[test]
    fn test_parse_event_xml_heartbeat() {
        let mut parser = EslParser::new();
        let xml_body = "\
<event>\n\
  <headers>\n\
    <Event-Name>HEARTBEAT</Event-Name>\n\
    <Core-UUID>abc-123</Core-UUID>\n\
    <Up-Time>0 years, 1 day</Up-Time>\n\
  </headers>\n\
</event>";
        let envelope = format!(
            "Content-Length: {}\nContent-Type: text/event-xml\n\n",
            xml_body.len()
        );
        let data = format!("{}{}", envelope, xml_body);

        parser
            .add_data(data.as_bytes())
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();
        let event = parser
            .parse_event(message, EventFormat::Xml)
            .unwrap();

        assert_eq!(event.event_type(), Some(EslEventType::Heartbeat));
        assert_eq!(event.header(EventHeader::CoreUuid), Some("abc-123"));
        assert_eq!(event.header_str("Up-Time"), Some("0 years, 1 day"));
    }

    #[test]
    fn test_parse_event_xml_with_body() {
        let mut parser = EslParser::new();
        let xml_body = "\
<event>\n\
  <headers>\n\
    <Event-Name>BACKGROUND_JOB</Event-Name>\n\
    <Job-UUID>def-456</Job-UUID>\n\
  </headers>\n\
  <body>+OK result data</body>\n\
</event>";
        let envelope = format!(
            "Content-Length: {}\nContent-Type: text/event-xml\n\n",
            xml_body.len()
        );
        let data = format!("{}{}", envelope, xml_body);

        parser
            .add_data(data.as_bytes())
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();
        let event = parser
            .parse_event(message, EventFormat::Xml)
            .unwrap();

        assert_eq!(event.event_type(), Some(EslEventType::BackgroundJob));
        assert_eq!(event.header(EventHeader::JobUuid), Some("def-456"));
        assert_eq!(event.body(), Some("+OK result data"));
    }

    #[test]
    fn test_crlf_header_terminator_not_matched() {
        // ESL uses \n\n, not \r\n\r\n. If something injects \r\n line endings,
        // the parser must not hang -- but it won't find the terminator either.
        // This documents the current behavior: \r\n\r\n is NOT recognized as
        // a header terminator, so the message stays incomplete.
        let mut parser = EslParser::new();
        let data = b"Content-Type: auth/request\r\n\r\n";

        parser
            .add_data(data)
            .unwrap();
        let result = parser
            .parse_message()
            .unwrap();
        assert!(
            result.is_none(),
            "\\r\\n\\r\\n should not match \\n\\n terminator"
        );
    }

    #[test]
    fn test_crlf_in_header_values_parsed_correctly() {
        // If \r\n appears within a \n\n-framed message, parse_headers()
        // uses .lines() which strips \r, so header values stay clean.
        let mut parser = EslParser::new();
        let data = b"Content-Type: auth/request\r\nSome-Header: some-value\n\n";

        parser
            .add_data(data)
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();
        assert_eq!(message.message_type, MessageType::AuthRequest);
        assert_eq!(
            message
                .headers
                .get("Some-Header")
                .map(|s| s.as_str()),
            Some("some-value")
        );
    }

    #[test]
    fn test_oversized_content_length_rejected() {
        let mut parser = EslParser::new();
        let data = format!(
            "Content-Type: api/response\nContent-Length: {}\n\n",
            MAX_MESSAGE_SIZE + 1
        );

        parser
            .add_data(data.as_bytes())
            .unwrap();
        let result = parser.parse_message();
        assert!(
            result.is_err(),
            "Content-Length exceeding MAX_MESSAGE_SIZE must be rejected"
        );
    }

    #[test]
    fn test_undersized_content_length_corrupts_next_message() {
        // Content-Length: 2 but body is "Hello" (5 bytes). The parser trusts
        // Content-Length and reads only 2 bytes, leaving "llo" in the buffer.
        // The next parse attempt sees "llo" as the start of a new message,
        // which won't have a valid header terminator -- so it returns None.
        let mut parser = EslParser::new();
        let data = b"Content-Type: api/response\nContent-Length: 2\n\nHello";

        parser
            .add_data(data)
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();
        assert_eq!(message.message_type, MessageType::ApiResponse);
        assert_eq!(message.body, Some("He".to_string()));

        // Leftover "llo" is now junk in the buffer -- next parse finds nothing
        let next = parser
            .parse_message()
            .unwrap();
        assert!(
            next.is_none(),
            "Leftover bytes should not form a valid message"
        );
    }

    #[test]
    fn test_undersized_content_length_followed_by_valid_message() {
        // Same scenario but a valid second message follows the junk.
        // The leftover bytes merge with the next message's headers,
        // making recovery impossible without reconnecting.
        let mut parser = EslParser::new();
        let msg1 = b"Content-Type: api/response\nContent-Length: 2\n\nHello";
        let msg2 = b"Content-Type: auth/request\n\n";

        parser
            .add_data(msg1)
            .unwrap();
        let first = parser
            .parse_message()
            .unwrap()
            .unwrap();
        assert_eq!(first.body, Some("He".to_string()));

        parser
            .add_data(msg2)
            .unwrap();
        let second = parser.parse_message();
        // "llo" + msg2 bytes = "lloContent-Type: auth/request\n\n"
        // The parser finds \n\n and parses "lloContent-Type: auth/request"
        // as key="lloContent-Type" value="auth/request". No real Content-Type
        // header exists, so the parser returns a protocol error -- signaling
        // the caller to disconnect.
        assert!(
            second.is_err(),
            "Desync must be detected as a protocol error"
        );
    }

    #[test]
    fn test_non_numeric_content_length_rejected() {
        let mut parser = EslParser::new();
        let data = b"Content-Type: api/response\nContent-Length: abc\n\n";

        parser
            .add_data(data)
            .unwrap();
        let result = parser.parse_message();
        assert!(
            result.is_err(),
            "Non-numeric Content-Length must be rejected"
        );
    }

    #[test]
    fn test_parse_headers_percent_decodes_values() {
        // parse_headers decodes values: the outbound connect response is a
        // serialized event (switch_event_serialize SWITCH_TRUE) whose values
        // are percent-encoded, and that response flows through this path.
        let parser = EslParser::new();
        let (headers, lossy) = parser
            .parse_headers("Channel-Name: sofia/internal/1000%40example.com\nX-Space: a%20b")
            .unwrap();

        assert_eq!(
            headers
                .get("Channel-Name")
                .map(|s| s.as_str()),
            Some("sofia/internal/1000@example.com")
        );
        assert_eq!(
            headers
                .get("X-Space")
                .map(|s| s.as_str()),
            Some("a b")
        );
        assert!(lossy.is_empty());
    }

    #[test]
    fn test_parse_plain_event_invalid_utf8_strict_error() {
        // Event-body header with invalid UTF-8 in strict mode
        let parser = EslParser::new().with_strict_header_utf8(true);
        let body = "Event-Name: HEARTBEAT\nX-Bad: %FF\n\n";
        let msg = EslMessage::new(
            MessageType::Event,
            {
                let mut h = IndexMap::new();
                h.insert("Content-Type".to_string(), "text/event-plain".to_string());
                h
            },
            Some(body.to_string()),
        );
        let result = parser.parse_event(msg, EventFormat::Plain);
        assert!(
            result.is_err(),
            "invalid UTF-8 in event header with strict mode must be an error"
        );
    }

    #[test]
    fn test_parse_connect_response() {
        let mut parser = EslParser::new();

        // FreeSWITCH serializes the outbound `connect` response with
        // switch_event_serialize(SWITCH_TRUE): every value is percent-encoded,
        // including the channel data. The parser must percent-decode them.
        let data = "Content-Type: command/reply\n\
             Reply-Text: +OK\n\
             Socket-Mode: async\n\
             Control: full\n\
             Event-Name: CHANNEL_DATA\n\
             Channel-Name: sofia/internal/1000%40example.com\n\
             Unique-ID: abcd-1234\n\
             Caller-Caller-ID-Name: Test%20User\n\
             \n";

        parser
            .add_data(data.as_bytes())
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();

        assert_eq!(message.message_type, MessageType::CommandReply);
        assert_eq!(
            message
                .headers
                .get("Channel-Name")
                .map(|s| s.as_str()),
            Some("sofia/internal/1000@example.com")
        );
        assert_eq!(
            message
                .headers
                .get("Caller-Caller-ID-Name")
                .map(|s| s.as_str()),
            Some("Test User")
        );
        assert_eq!(
            message
                .headers
                .get("Socket-Mode")
                .map(|s| s.as_str()),
            Some("async")
        );
        assert_eq!(
            message
                .headers
                .get("Control")
                .map(|s| s.as_str()),
            Some("full")
        );

        let response = message.into_response();
        assert!(response.is_success());
        assert_eq!(response.reply_text(), Some("+OK"));
        assert!(response
            .lossy_values()
            .is_empty());
    }

    #[test]
    fn test_connect_response_non_utf8_value_lossy() {
        // A channel-data value that is not valid UTF-8 after percent-decoding
        // (a Latin-1 caller name) is decoded lossily by default and surfaced
        // on the response, not a hard error.
        let mut parser = EslParser::new();
        let data = "Content-Type: command/reply\n\
             Reply-Text: +OK\n\
             Caller-Caller-ID-Name: Andr%E9\n\
             \n";
        parser
            .add_data(data.as_bytes())
            .unwrap();
        let response = parser
            .parse_message()
            .unwrap()
            .unwrap()
            .into_response();

        assert!(response.is_success());
        assert_eq!(
            response.header("Caller-Caller-ID-Name"),
            Some("Andr\u{FFFD}")
        );
        let lossy = response.lossy_values();
        assert_eq!(
            lossy
                .iter()
                .count(),
            1
        );
        let entry = lossy
            .iter()
            .next()
            .unwrap();
        assert_eq!(entry.key(), "Caller-Caller-ID-Name");
        assert_eq!(entry.raw_value(), "Andr%E9");
    }

    #[test]
    fn test_connect_response_non_utf8_value_strict_error() {
        let mut parser = EslParser::new().with_strict_header_utf8(true);
        let data = "Content-Type: command/reply\nCaller-Caller-ID-Name: Andr%E9\n\n";
        parser
            .add_data(data.as_bytes())
            .unwrap();
        let result = parser.parse_message();
        assert!(matches!(result, Err(EslError::InvalidUtf8InHeader { .. })));
    }

    #[test]
    fn test_message_body_non_utf8_lossy_default() {
        // A raw event body is framed by Content-Length in bytes and is not
        // percent-encoded; non-UTF-8 bytes there must not kill the connection.
        let mut parser = EslParser::new();
        let mut data = b"Content-Type: text/event-plain\nContent-Length: 4\n\n".to_vec();
        data.extend_from_slice(b"caf\xE9");
        parser
            .add_data(&data)
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();

        assert_eq!(message.message_type, MessageType::Event);
        assert_eq!(
            message
                .body
                .as_deref(),
            Some("caf\u{FFFD}")
        );
        assert_eq!(
            message
                .raw_body
                .as_deref(),
            Some(&b"caf\xE9"[..])
        );
    }

    #[test]
    fn test_message_body_non_utf8_strict_error() {
        let mut parser = EslParser::new().with_strict_header_utf8(true);
        let mut data = b"Content-Type: text/event-plain\nContent-Length: 4\n\n".to_vec();
        data.extend_from_slice(b"caf\xE9");
        parser
            .add_data(&data)
            .unwrap();
        assert!(matches!(
            parser.parse_message(),
            Err(EslError::ProtocolError { .. })
        ));
    }

    #[test]
    fn test_plain_event_non_utf8_inner_body_lossy() {
        // sendevent payloads ride as a raw inner body after the
        // percent-encoded event headers; the raw bytes must reach the event.
        let mut parser = EslParser::new();
        let event_body: &[u8] = b"Event-Name: NOTIFY_IN\nContent-Length: 4\n\ncaf\xE9";
        let mut data = format!(
            "Content-Type: text/event-plain\nContent-Length: {}\n\n",
            event_body.len()
        )
        .into_bytes();
        data.extend_from_slice(event_body);
        parser
            .add_data(&data)
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();
        let event = parser
            .parse_event(message, EventFormat::Plain)
            .unwrap();

        assert_eq!(event.body(), Some("caf\u{FFFD}"));
        assert_eq!(event.raw_body(), Some(&b"caf\xE9"[..]));
    }

    #[test]
    fn test_api_response_non_utf8_body_raw_body() {
        let mut parser = EslParser::new();
        let mut data = b"Content-Type: api/response\nContent-Length: 4\n\n".to_vec();
        data.extend_from_slice(b"caf\xE9");
        parser
            .add_data(&data)
            .unwrap();
        let response = parser
            .parse_message()
            .unwrap()
            .unwrap()
            .into_response();

        assert_eq!(response.body(), Some("caf\u{FFFD}"));
        assert_eq!(response.raw_body(), Some(&b"caf\xE9"[..]));
    }

    #[test]
    fn test_parse_json_event_body_key() {
        let parser = EslParser::new();
        let json = r#"{"Event-Name":"BACKGROUND_JOB","Job-UUID":"abc-123","_body":"+OK result"}"#;
        let msg = EslMessage::new(
            MessageType::Event,
            {
                let mut h = IndexMap::new();
                h.insert("Content-Type".to_string(), "text/event-json".to_string());
                h
            },
            Some(json.to_string()),
        );
        let event = parser
            .parse_event(msg, EventFormat::Json)
            .unwrap();
        assert_eq!(event.event_type(), Some(EslEventType::BackgroundJob));
        assert_eq!(event.body(), Some("+OK result"));
        assert!(
            event
                .header_str("_body")
                .is_none(),
            "_body must be mapped to event body, not stored as a header"
        );
    }

    // --- T2: JSON event format end-to-end through parser pipeline ---

    #[test]
    fn test_json_event_end_to_end() {
        let mut parser = EslParser::new();
        let json_body = r#"{"Event-Name":"CHANNEL_CREATE","Unique-ID":"test-uuid-123","Channel-Name":"sofia/internal/1000@example.com","variable_sip_call_id":"call-456"}"#;
        let envelope = format!(
            "Content-Length: {}\nContent-Type: text/event-json\n\n",
            json_body.len()
        );
        let data = format!("{}{}", envelope, json_body);

        parser
            .add_data(data.as_bytes())
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();
        assert_eq!(message.message_type, MessageType::Event);

        let event = parser
            .parse_event(message, EventFormat::Json)
            .unwrap();
        assert_eq!(event.event_type(), Some(EslEventType::ChannelCreate));
        assert_eq!(event.unique_id(), Some("test-uuid-123"));
        assert_eq!(
            event.header_str("Channel-Name"),
            Some("sofia/internal/1000@example.com")
        );
        assert_eq!(event.variable_str("sip_call_id"), Some("call-456"));
    }

    #[test]
    fn test_json_event_with_body_end_to_end() {
        let mut parser = EslParser::new();
        let json_body = r#"{"Event-Name":"BACKGROUND_JOB","Job-UUID":"job-789","_body":"+OK result data\nline 2"}"#;
        let envelope = format!(
            "Content-Length: {}\nContent-Type: text/event-json\n\n",
            json_body.len()
        );
        let data = format!("{}{}", envelope, json_body);

        parser
            .add_data(data.as_bytes())
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();
        let event = parser
            .parse_event(message, EventFormat::Json)
            .unwrap();
        assert_eq!(event.event_type(), Some(EslEventType::BackgroundJob));
        assert!(event
            .body()
            .is_some());
        assert!(event
            .header_str("_body")
            .is_none());
    }

    // --- T5: XML event parsing with &amp; escaped characters ---

    #[test]
    fn test_parse_event_xml_ampersand_escaped() {
        let mut parser = EslParser::new();
        let xml_body = "\
<event>\n\
  <headers>\n\
    <Event-Name>CHANNEL_CREATE</Event-Name>\n\
    <Caller-Caller-ID-Name>Smith &amp; Jones</Caller-Caller-ID-Name>\n\
    <variable_sip_h_Subject>Test &lt;1&gt; &amp; Test &lt;2&gt;</variable_sip_h_Subject>\n\
  </headers>\n\
</event>";
        let envelope = format!(
            "Content-Length: {}\nContent-Type: text/event-xml\n\n",
            xml_body.len()
        );
        let data = format!("{}{}", envelope, xml_body);

        parser
            .add_data(data.as_bytes())
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();
        let event = parser
            .parse_event(message, EventFormat::Xml)
            .unwrap();

        assert_eq!(event.event_type(), Some(EslEventType::ChannelCreate));
        assert_eq!(
            event.header_str("Caller-Caller-ID-Name"),
            Some("Smith & Jones")
        );
        assert_eq!(
            event.variable_str("sip_h_Subject"),
            Some("Test <1> & Test <2>")
        );
    }

    // --- T6: ParseState::WaitingForBody multi-chunk completion ---

    #[test]
    fn test_waiting_for_body_multi_chunk() {
        let mut parser = EslParser::new();

        // Send headers first (with body length)
        let headers = b"Content-Type: api/response\nContent-Length: 20\n\n";
        parser
            .add_data(headers)
            .unwrap();

        // Parser transitions to WaitingForBody, returns None
        let result = parser
            .parse_message()
            .unwrap();
        assert!(result.is_none(), "should be waiting for body data");

        // Send first chunk (10 of 20 bytes)
        parser
            .add_data(b"0123456789")
            .unwrap();
        let result = parser
            .parse_message()
            .unwrap();
        assert!(result.is_none(), "still waiting for remaining body data");

        // Send remaining 10 bytes
        parser
            .add_data(b"abcdefghij")
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();
        assert_eq!(message.message_type, MessageType::ApiResponse);
        assert_eq!(message.body, Some("0123456789abcdefghij".to_string()));
    }

    #[test]
    fn test_rude_rejection_message_type() {
        let mt = MessageType::from_content_type("text/rude-rejection").unwrap();
        assert_eq!(mt, MessageType::RudeRejection);
    }

    #[test]
    fn test_unknown_content_type_is_protocol_error() {
        let err = MessageType::from_content_type("text/something-new").unwrap_err();
        assert!(matches!(err, EslError::ProtocolError { .. }));
    }

    #[test]
    fn test_to_plain_format_round_trip() {
        use crate::event::{EslEvent, EslEventType, EventFormat};
        use indexmap::IndexMap;

        let mut original = EslEvent::with_type(EslEventType::Heartbeat);
        original.set_header("Event-Name", "HEARTBEAT");
        original.set_header("Core-UUID", "abc-123");
        original.set_header("Up-Time", "0 years, 0 days, 1 hour");
        original.set_header("Event-Info", "System Ready");

        let plain1 = original.to_plain_format();

        let msg1 = EslMessage::new(
            MessageType::Event,
            {
                let mut h = IndexMap::new();
                h.insert("Content-Type".to_string(), "text/event-plain".to_string());
                h
            },
            Some(plain1.clone()),
        );
        let parsed1 = EslParser::new()
            .parse_event(msg1, EventFormat::Plain)
            .unwrap();

        assert_eq!(parsed1.event_type(), original.event_type());
        assert_eq!(parsed1.headers(), original.headers());
        assert_eq!(parsed1.body(), original.body());

        let plain2 = parsed1.to_plain_format();
        let msg2 = EslMessage::new(
            MessageType::Event,
            {
                let mut h = IndexMap::new();
                h.insert("Content-Type".to_string(), "text/event-plain".to_string());
                h
            },
            Some(plain2),
        );
        let parsed2 = EslParser::new()
            .parse_event(msg2, EventFormat::Plain)
            .unwrap();

        assert_eq!(parsed2.event_type(), original.event_type());
        assert_eq!(parsed2.headers(), original.headers());
        assert_eq!(parsed2.body(), original.body());
    }

    #[test]
    fn test_to_plain_format_wire_round_trip() {
        use crate::event::EventFormat;
        use crate::headers::EventHeader;
        use indexmap::IndexMap;

        // Realistic wire payload as FreeSWITCH would send it (percent-encoded
        // values, headers in FS emission order)
        let wire_body = "\
Event-Name: CHANNEL_CREATE\n\
Core-UUID: 2bde6598-0f10-4b90-b70e-d21f4c9e270f\n\
FreeSWITCH-Hostname: fs01%2Eexample%2Ecom\n\
FreeSWITCH-IPv4: 10%2E0%2E0%2E1\n\
Event-Date-Local: 2025-06-15%2010%3A30%3A00\n\
Unique-ID: a1b2c3d4-5678-9abc-def0-123456789abc\n\
Channel-Name: sofia%2Finternal%2F1000%40example.com\n\
Caller-Caller-ID-Name: J%C3%A9r%C3%B4me%20Poulin\n\
Call-Direction: inbound\n\
Channel-State: CS_INIT\n\
\n";

        let msg = EslMessage::new(
            MessageType::Event,
            {
                let mut h = IndexMap::new();
                h.insert("Content-Type".to_string(), "text/event-plain".to_string());
                h
            },
            Some(wire_body.to_string()),
        );
        let event = EslParser::new()
            .parse_event(msg, EventFormat::Plain)
            .unwrap();

        assert_eq!(
            event.header(EventHeader::FreeswitchHostname),
            Some("fs01.example.com")
        );
        assert_eq!(
            event.header(EventHeader::CallerCallerIdName),
            Some("Jérôme Poulin")
        );

        let regenerated = event.to_plain_format();

        // Parse the regenerated output back and compare
        let msg2 = EslMessage::new(
            MessageType::Event,
            {
                let mut h = IndexMap::new();
                h.insert("Content-Type".to_string(), "text/event-plain".to_string());
                h
            },
            Some(regenerated.clone()),
        );
        let reparsed = EslParser::new()
            .parse_event(msg2, EventFormat::Plain)
            .unwrap();
        assert_eq!(event.headers(), reparsed.headers());
        assert_eq!(event.body(), reparsed.body());

        // Verify header order is preserved (wire order, not alphabetical)
        let keys: Vec<&str> = regenerated
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| {
                l.split(':')
                    .next()
                    .unwrap()
            })
            .collect();
        assert_eq!(keys[0], "Event-Name");
        assert_eq!(keys[1], "Core-UUID");
        assert_eq!(keys[2], "FreeSWITCH-Hostname");
        assert_eq!(keys[3], "FreeSWITCH-IPv4");
    }

    #[test]
    fn test_to_plain_format_round_trip_with_body() {
        use crate::event::{EslEvent, EslEventType, EventFormat};
        use indexmap::IndexMap;

        let body_text = "+OK Status\nLine 2\n";
        let mut original = EslEvent::with_type(EslEventType::BackgroundJob);
        original.set_header("Event-Name", "BACKGROUND_JOB");
        original.set_header("Job-UUID", "job-789");
        original.set_header(
            "Content-Length".to_string(),
            body_text
                .len()
                .to_string(),
        );
        original.set_body(body_text.to_string());

        let plain = original.to_plain_format();
        let msg = EslMessage::new(
            MessageType::Event,
            {
                let mut h = IndexMap::new();
                h.insert("Content-Type".to_string(), "text/event-plain".to_string());
                h
            },
            Some(plain),
        );
        let parsed = EslParser::new()
            .parse_event(msg, EventFormat::Plain)
            .unwrap();

        assert_eq!(parsed.event_type(), original.event_type());
        assert_eq!(parsed.headers(), original.headers());
        assert_eq!(parsed.body(), original.body());
    }

    #[test]
    fn test_event_body_lossy_decode_default() {
        // Event-body value with %E9 (invalid UTF-8) is decoded lossily by default
        let parser = EslParser::new();
        let body = "Event-Name: HEARTBEAT\nvariable_dp_match: %E9foo\nvalid_key: normal\n\n";
        let msg = EslMessage::new(
            MessageType::Event,
            {
                let mut h = IndexMap::new();
                h.insert("Content-Type".to_string(), "text/event-plain".to_string());
                h
            },
            Some(body.to_string()),
        );
        let event = parser
            .parse_event(msg, EventFormat::Plain)
            .unwrap();

        // Lossy decode produced U+FFFD
        assert!(event
            .header_str("variable_dp_match")
            .unwrap()
            .contains('\u{FFFD}'));
        // LossyValues tracks the affected key
        let lossy = event.lossy_values();
        assert!(!lossy.is_empty());
        assert_eq!(
            lossy
                .iter()
                .count(),
            1
        );
        let entry = lossy
            .iter()
            .next()
            .unwrap();
        assert_eq!(entry.key(), "variable_dp_match");
        assert_eq!(entry.raw_value(), "%E9foo");
        // Well-formed header unaffected
        assert_eq!(event.header_str("valid_key"), Some("normal"));
    }

    #[test]
    fn test_event_body_strict_utf8_fails() {
        // Same input with strict option => error
        let parser = EslParser::new().with_strict_header_utf8(true);
        let body = "Event-Name: HEARTBEAT\nvariable_dp_match: %E9foo\n\n";
        let msg = EslMessage::new(
            MessageType::Event,
            {
                let mut h = IndexMap::new();
                h.insert("Content-Type".to_string(), "text/event-plain".to_string());
                h
            },
            Some(body.to_string()),
        );
        let result = parser.parse_event(msg, EventFormat::Plain);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, EslError::InvalidUtf8InHeader { .. }),
            "expected InvalidUtf8InHeader, got: {:?}",
            err
        );
    }

    #[test]
    fn test_missing_colon_invalid_header() {
        let result = EslParser::parse_header_line("NoColonHere");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EslError::InvalidHeader { .. }
        ));
    }

    #[test]
    fn test_lossy_values_serde_roundtrip() {
        let parser = EslParser::new();
        let body = "Event-Name: HEARTBEAT\nkey1: %E9value\n\n";
        let msg = EslMessage::new(
            MessageType::Event,
            {
                let mut h = IndexMap::new();
                h.insert("Content-Type".to_string(), "text/event-plain".to_string());
                h
            },
            Some(body.to_string()),
        );
        let event = parser
            .parse_event(msg, EventFormat::Plain)
            .unwrap();

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: EslEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(
            deserialized
                .lossy_values()
                .iter()
                .count(),
            1
        );
        assert_eq!(
            deserialized
                .lossy_values()
                .iter()
                .next()
                .unwrap()
                .key(),
            "Key1"
        );
        assert_eq!(
            deserialized
                .lossy_values()
                .iter()
                .next()
                .unwrap()
                .raw_value(),
            "%E9value"
        );
    }

    #[test]
    fn test_lossy_values_old_json_without_field() {
        // Old JSON without lossy_values field deserializes to empty
        let json = r#"{"headers":{"Event-Name":"HEARTBEAT"},"body":null}"#;
        let event: EslEvent = serde_json::from_str(json).unwrap();
        assert!(event
            .lossy_values()
            .is_empty());
    }

    #[test]
    fn json_event_non_utf8_value_carries_lossy_signal() {
        // FreeSWITCH serializes raw header values into event-json with no
        // percent-encoding or UTF-8 validation (switch_event.c cJSON path),
        // so a Latin-1 value reaches the wire raw. The envelope body decodes
        // lossily; the JSON parses fine — but the lossy signal must ride on
        // the event, not vanish.
        let mut parser = EslParser::new();
        let json_body: &[u8] =
            b"{\"Event-Name\":\"CHANNEL_CREATE\",\"Caller-Caller-ID-Name\":\"Andr\xE9\"}";
        let mut data = format!(
            "Content-Length: {}\nContent-Type: text/event-json\n\n",
            json_body.len()
        )
        .into_bytes();
        data.extend_from_slice(json_body);

        parser
            .add_data(&data)
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();
        let event = parser
            .parse_event(message, EventFormat::Json)
            .unwrap();

        assert_eq!(event.event_type(), Some(EslEventType::ChannelCreate));
        assert_eq!(
            event.header_str("Caller-Caller-ID-Name"),
            Some("Andr\u{FFFD}")
        );
        // Whole-envelope wire bytes: JSON cannot map bytes back to the
        // decoded body, but the signal and the source bytes must be carried.
        assert_eq!(event.raw_body(), Some(json_body));
    }

    #[test]
    fn json_event_non_object_body_is_error() {
        // A framed event whose body is well-formed JSON of the wrong shape is
        // a protocol violation: Err, not a silent empty event that is
        // indistinguishable from a headerless one.
        let parser = EslParser::new();
        for body in ["[1,2,3]", "\"foo\"", "123", "null"] {
            let msg = EslMessage::new(
                MessageType::Event,
                {
                    let mut h = IndexMap::new();
                    h.insert("Content-Type".to_string(), "text/event-json".to_string());
                    h
                },
                Some(body.to_string()),
            );
            let result = parser.parse_event(msg, EventFormat::Json);
            assert!(
                matches!(result, Err(EslError::ProtocolError { .. })),
                "body {body:?} must be a protocol error, got: {result:?}"
            );
        }
    }

    #[test]
    fn xml_event_non_utf8_text_carries_lossy_signal() {
        let mut parser = EslParser::new();
        let xml_body: &[u8] = b"<event>\n  <headers>\n    <Event-Name>CHANNEL_CREATE</Event-Name>\n    <Caller-Caller-ID-Name>Andr\xE9</Caller-Caller-ID-Name>\n  </headers>\n</event>";
        let mut data = format!(
            "Content-Length: {}\nContent-Type: text/event-xml\n\n",
            xml_body.len()
        )
        .into_bytes();
        data.extend_from_slice(xml_body);

        parser
            .add_data(&data)
            .unwrap();
        let message = parser
            .parse_message()
            .unwrap()
            .unwrap();
        let event = parser
            .parse_event(message, EventFormat::Xml)
            .unwrap();

        assert_eq!(event.event_type(), Some(EslEventType::ChannelCreate));
        assert_eq!(
            event.header_str("Caller-Caller-ID-Name"),
            Some("Andr\u{FFFD}")
        );
        assert_eq!(event.raw_body(), Some(xml_body));
    }

    #[test]
    fn test_lossy_values_display_keys_only() {
        let parser = EslParser::new();
        let body = "Event-Name: HEARTBEAT\nkey1: %E9foo\nkey2: %FFbar\n\n";
        let msg = EslMessage::new(
            MessageType::Event,
            {
                let mut h = IndexMap::new();
                h.insert("Content-Type".to_string(), "text/event-plain".to_string());
                h
            },
            Some(body.to_string()),
        );
        let event = parser
            .parse_event(msg, EventFormat::Plain)
            .unwrap();

        let display = event
            .lossy_values()
            .to_string();
        assert_eq!(display, "Key1, Key2");
        assert!(!display.contains("%E9"));
        assert!(!display.contains("foo"));
    }

    /// Feed `data` split at `split`, driving parse_message across the two
    /// chunks, and parse the resulting frame as an event of `format`.
    fn parse_wire_split(data: &[u8], split: usize, format: EventFormat) -> EslEvent {
        let mut parser = EslParser::new();
        parser
            .add_data(&data[..split])
            .unwrap();
        let mut message = parser
            .parse_message()
            .unwrap();
        assert!(
            message.is_none(),
            "frame must be incomplete at split {split}"
        );
        parser
            .add_data(&data[split..])
            .unwrap();
        message = parser
            .parse_message()
            .unwrap();
        let message =
            message.unwrap_or_else(|| panic!("frame incomplete after both chunks (split {split})"));
        parser
            .parse_event(message, format)
            .unwrap()
    }

    #[test]
    fn plain_event_frame_split_all_points() {
        // Inner body included so splits land mid-envelope-header, between the
        // two terminator \n bytes, mid-event-header, and mid-inner-body.
        let inner_body = "+OK result\n";
        let body = format!(
            "Event-Name: BACKGROUND_JOB\nJob-UUID: abc-123\nContent-Length: {}\n\n{}",
            inner_body.len(),
            inner_body
        );
        let data = format!(
            "Content-Length: {}\nContent-Type: text/event-plain\n\n{}",
            body.len(),
            body
        )
        .into_bytes();

        for split in 1..data.len() {
            let event = parse_wire_split(&data, split, EventFormat::Plain);
            assert_eq!(
                event.event_type(),
                Some(EslEventType::BackgroundJob),
                "split {split}"
            );
            assert_eq!(
                event.header(EventHeader::JobUuid),
                Some("abc-123"),
                "split {split}"
            );
            assert_eq!(event.body(), Some(inner_body), "split {split}");
        }
    }

    #[test]
    fn json_event_frame_split_all_points() {
        let json_body =
            r#"{"Event-Name":"BACKGROUND_JOB","Job-UUID":"abc-123","_body":"+OK result"}"#;
        let data = format!(
            "Content-Length: {}\nContent-Type: text/event-json\n\n{}",
            json_body.len(),
            json_body
        )
        .into_bytes();

        for split in 1..data.len() {
            let event = parse_wire_split(&data, split, EventFormat::Json);
            assert_eq!(
                event.event_type(),
                Some(EslEventType::BackgroundJob),
                "split {split}"
            );
            assert_eq!(event.body(), Some("+OK result"), "split {split}");
        }
    }

    #[test]
    fn xml_event_frame_split_all_points() {
        let xml_body = "<event>\n  <headers>\n    <Event-Name>BACKGROUND_JOB</Event-Name>\n    <Job-UUID>abc-123</Job-UUID>\n  </headers>\n  <body>+OK result</body>\n</event>";
        let data = format!(
            "Content-Length: {}\nContent-Type: text/event-xml\n\n{}",
            xml_body.len(),
            xml_body
        )
        .into_bytes();

        for split in 1..data.len() {
            let event = parse_wire_split(&data, split, EventFormat::Xml);
            assert_eq!(
                event.event_type(),
                Some(EslEventType::BackgroundJob),
                "split {split}"
            );
            assert_eq!(event.body(), Some("+OK result"), "split {split}");
        }
    }

    #[test]
    fn multiple_event_frames_in_single_read() {
        let mut parser = EslParser::new();

        let bodies = [
            "Event-Name: CHANNEL_CREATE\nUnique-ID: uuid-1\n\n",
            "Event-Name: CHANNEL_ANSWER\nUnique-ID: uuid-2\n\n",
            "Event-Name: CHANNEL_DESTROY\nUnique-ID: uuid-3\n\n",
        ];
        let mut data = Vec::new();
        for body in &bodies {
            data.extend_from_slice(
                format!(
                    "Content-Length: {}\nContent-Type: text/event-plain\n\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            );
        }

        parser
            .add_data(&data)
            .unwrap();

        let expected = [
            (EslEventType::ChannelCreate, "uuid-1"),
            (EslEventType::ChannelAnswer, "uuid-2"),
            (EslEventType::ChannelDestroy, "uuid-3"),
        ];
        for (event_type, uuid) in expected {
            let message = parser
                .parse_message()
                .unwrap()
                .expect("all frames arrived in one read");
            let event = parser
                .parse_event(message, EventFormat::Plain)
                .unwrap();
            assert_eq!(event.event_type(), Some(event_type));
            assert_eq!(event.unique_id(), Some(uuid));
        }
        assert!(parser
            .parse_message()
            .unwrap()
            .is_none());
    }
}
