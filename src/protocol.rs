//! ESL protocol parsing and message handling

use crate::{
    buffer::EslBuffer,
    command::EslResponse,
    constants::*,
    error::{EslError, EslResult},
    event::{EslEvent, EslEventType, EventFormat},
};
use std::collections::HashMap;

/// ESL message types
#[derive(Debug, Clone, PartialEq)]
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
    /// Unknown message type
    Unknown(String),
}

impl MessageType {
    /// Parse message type from Content-Type header
    pub fn from_content_type(content_type: &str) -> Self {
        match content_type {
            CONTENT_TYPE_AUTH_REQUEST => MessageType::AuthRequest,
            CONTENT_TYPE_COMMAND_REPLY => MessageType::CommandReply,
            CONTENT_TYPE_API_RESPONSE => MessageType::ApiResponse,
            CONTENT_TYPE_TEXT_EVENT_PLAIN
            | CONTENT_TYPE_TEXT_EVENT_JSON
            | CONTENT_TYPE_TEXT_EVENT_XML
            | "log/data" => MessageType::Event,
            "text/disconnect-notice" => MessageType::Disconnect,
            _ => MessageType::Unknown(content_type.to_string()),
        }
    }
}

/// Parsed ESL message
#[derive(Debug, Clone)]
pub struct EslMessage {
    /// Message type
    pub message_type: MessageType,
    /// Message headers
    pub headers: HashMap<String, String>,
    /// Message body (optional)
    pub body: Option<String>,
}

impl EslMessage {
    /// Create new message
    pub fn new(
        message_type: MessageType,
        headers: HashMap<String, String>,
        body: Option<String>,
    ) -> Self {
        Self {
            message_type,
            headers,
            body,
        }
    }

    /// Get header value
    pub fn header(&self, name: &str) -> Option<&String> {
        self.headers.get(name)
    }

    /// Convert to EslResponse
    pub fn into_response(self) -> EslResponse {
        EslResponse::new(self.headers, self.body)
    }

    /// Convert to EslEvent
    pub fn into_event(self) -> EslResult<EslEvent> {
        if self.message_type != MessageType::Event {
            return Err(EslError::protocol_error("Message is not an event"));
        }

        let mut event = EslEvent::new();
        event.headers = self.headers;
        event.body = self.body;

        // Parse event type from Event-Name header
        if let Some(event_name) = event.header(HEADER_EVENT_NAME) {
            event.event_type = EslEventType::parse_event_type(event_name);
        }

        Ok(event)
    }

    /// Check if this is a successful response
    pub fn is_success(&self) -> bool {
        if let Some(reply_text) = self.header(HEADER_REPLY_TEXT) {
            reply_text.starts_with("+OK")
        } else {
            true
        }
    }
}

/// Parser state for handling incomplete messages
#[derive(Debug)]
enum ParseState {
    WaitingForHeaders,
    WaitingForBody {
        message_type: MessageType,
        headers: std::collections::HashMap<String, String>,
        body_length: usize,
    },
}

/// ESL protocol parser
pub struct EslParser {
    buffer: EslBuffer,
    state: ParseState,
}

impl EslParser {
    /// Create new parser
    pub fn new() -> Self {
        Self {
            buffer: EslBuffer::new(),
            state: ParseState::WaitingForHeaders,
        }
    }

    /// Add data to the parser buffer
    pub fn add_data(&mut self, data: &[u8]) -> EslResult<()> {
        self.buffer.extend_from_slice(data);
        self.buffer.check_size_limits()?;
        Ok(())
    }

    /// Try to parse a complete message from the buffer
    pub fn parse_message(&mut self) -> EslResult<Option<EslMessage>> {
        match &self.state {
            ParseState::WaitingForHeaders => {
                // Check if we have complete headers
                let terminator = HEADER_TERMINATOR.as_bytes();

                if let Some(headers_data) = self.buffer.extract_until_pattern(terminator) {
                    // Compact buffer to free consumed header data
                    self.buffer.compact();

                    // Parse headers
                    let headers_str = String::from_utf8(headers_data)
                        .map_err(|_| EslError::protocol_error("Invalid UTF-8 in headers"))?;

                    let headers = self.parse_headers(&headers_str)?;

                    // Determine message type
                    let content_type = headers
                        .get(HEADER_CONTENT_TYPE)
                        .map(|s| s.as_str())
                        .unwrap_or("unknown");
                    let message_type = MessageType::from_content_type(content_type);

                    // Check if we need a body
                    if let Some(length_str) = headers.get(HEADER_CONTENT_LENGTH) {
                        let length: usize =
                            length_str
                                .trim()
                                .parse()
                                .map_err(|_| EslError::InvalidHeader {
                                    header: format!("Content-Length: {}", length_str),
                                })?;

                        // Validate message size to prevent protocol errors or memory exhaustion
                        if length > MAX_MESSAGE_SIZE {
                            return Err(EslError::protocol_error(&format!(
                                "Message too large: Content-Length {} exceeds limit {}. Protocol error or corrupted data.",
                                length, MAX_MESSAGE_SIZE
                            )));
                        }

                        if length > 0 {
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
                            let message = EslMessage::new(message_type, headers, None);
                            self.state = ParseState::WaitingForHeaders;
                            Ok(Some(message))
                        }
                    } else {
                        // No Content-Length header, complete message without body
                        let message = EslMessage::new(message_type, headers, None);
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
                if let Some(body_data) = self.buffer.extract_bytes(*body_length) {
                    // Compact buffer to free consumed body data
                    self.buffer.compact();

                    let body_str = String::from_utf8(body_data)
                        .map_err(|_| EslError::protocol_error("Invalid UTF-8 in body"))?;

                    let message =
                        EslMessage::new(message_type.clone(), headers.clone(), Some(body_str));
                    self.state = ParseState::WaitingForHeaders;
                    Ok(Some(message))
                } else {
                    // Not enough body data yet
                    Ok(None)
                }
            }
        }
    }

    /// Parse headers from string
    fn parse_headers(&self, headers_str: &str) -> EslResult<HashMap<String, String>> {
        let mut headers = HashMap::new();

        for line in headers_str.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if let Some(colon_pos) = line.find(':') {
                let key = line[..colon_pos].trim().to_string();
                let value = line[colon_pos + 1..].trim().to_string();
                headers.insert(key, value);
            } else {
                return Err(EslError::InvalidHeader {
                    header: line.to_string(),
                });
            }
        }

        Ok(headers)
    }

    /// Parse event from message, handling different formats
    pub fn parse_event(&self, message: EslMessage, format: EventFormat) -> EslResult<EslEvent> {
        match format {
            EventFormat::Plain => self.parse_plain_event(message),
            EventFormat::Json => self.parse_json_event(message),
            EventFormat::Xml => self.parse_xml_event(message),
        }
    }

    /// Parse plain text event
    fn parse_plain_event(&self, message: EslMessage) -> EslResult<EslEvent> {
        if message.message_type != MessageType::Event {
            return Err(EslError::protocol_error("Not an event message"));
        }

        let mut event = EslEvent::new();

        // For plain events, headers contain the event data
        event.headers = message.headers;
        event.body = message.body;

        // Extract event type from Event-Name header
        if let Some(event_name) = event.header(HEADER_EVENT_NAME) {
            event.event_type = EslEventType::parse_event_type(event_name);
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

        let mut event = EslEvent::new();

        if let Some(obj) = json_value.as_object() {
            // Convert JSON object to headers
            for (key, value) in obj {
                let value_str = match value {
                    serde_json::Value::String(s) => s.clone(),
                    _ => value.to_string(),
                };
                event.headers.insert(key.clone(), value_str);
            }

            // Extract event type
            if let Some(event_name) = event.header("Event-Name") {
                event.event_type = EslEventType::parse_event_type(event_name);
            }
        }

        Ok(event)
    }

    /// Extract XML attribute from a line
    fn extract_xml_attribute(line: &str) -> Option<(String, String)> {
        let eq_pos = line.find('=')?;
        let start = line.find('"')?;
        let end = line.rfind('"')?;

        if start != end {
            let key = line[1..eq_pos].trim();
            let value = &line[start + 1..end];
            Some((key.to_string(), value.to_string()))
        } else {
            None
        }
    }

    /// Parse XML event
    fn parse_xml_event(&self, message: EslMessage) -> EslResult<EslEvent> {
        let body = message
            .body
            .ok_or_else(|| EslError::protocol_error("XML event missing body"))?;

        // Parse XML - simplified implementation
        // In a full implementation, you'd use proper XML parsing
        let mut event = EslEvent::new();

        // For now, just extract text content between XML tags
        // This is a simplified parser - you might want to use quick-xml properly
        let lines = body.lines();
        for line in lines {
            let line = line.trim();
            if line.starts_with('<') && line.ends_with('>') && line.contains("=") {
                if let Some((key, value)) = Self::extract_xml_attribute(line) {
                    event.headers.insert(key, value);
                }
            }
        }

        // Extract event type
        if let Some(event_name) = event.header("Event-Name") {
            event.event_type = EslEventType::parse_event_type(event_name);
        }

        Ok(event)
    }

    /// Get current buffer length
    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }

    /// Compact the internal buffer
    pub fn compact_buffer(&mut self) {
        self.buffer.compact();
    }

    /// Clear the buffer
    pub fn clear_buffer(&mut self) {
        self.buffer.clear();
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

    #[test]
    fn test_parse_headers() {
        let parser = EslParser::new();
        let headers_str = "Content-Type: auth/request\r\nContent-Length: 0";
        let headers = parser.parse_headers(headers_str).unwrap();

        assert_eq!(
            headers.get("Content-Type"),
            Some(&"auth/request".to_string())
        );
        assert_eq!(headers.get("Content-Length"), Some(&"0".to_string()));
    }

    #[test]
    fn test_parse_auth_request() {
        let mut parser = EslParser::new();
        let data = b"Content-Type: auth/request\r\n\r\n";

        parser.add_data(data).unwrap();
        let message = parser.parse_message().unwrap().unwrap();

        assert_eq!(message.message_type, MessageType::AuthRequest);
        assert!(message.body.is_none());
    }

    #[test]
    fn test_parse_api_response() {
        let mut parser = EslParser::new();
        let data = b"Content-Type: api/response\r\nContent-Length: 2\r\n\r\nOK";

        parser.add_data(data).unwrap();
        let message = parser.parse_message().unwrap().unwrap();

        assert_eq!(message.message_type, MessageType::ApiResponse);
        assert_eq!(message.body, Some("OK".to_string()));
    }

    #[test]
    fn test_parse_event() {
        let mut parser = EslParser::new();
        let data = b"Content-Type: text/event-plain\r\nEvent-Name: CHANNEL_ANSWER\r\nUnique-ID: test-uuid\r\n\r\n";

        parser.add_data(data).unwrap();
        let message = parser.parse_message().unwrap().unwrap();
        let event = parser.parse_event(message, EventFormat::Plain).unwrap();

        assert_eq!(event.event_type, Some(EslEventType::ChannelAnswer));
        assert_eq!(event.unique_id(), Some(&"test-uuid".to_string()));
    }

    #[test]
    fn test_incomplete_message() {
        let mut parser = EslParser::new();
        let data = b"Content-Type: api/response\r\nContent-Length: 10\r\n\r\ntest"; // Only 4 bytes instead of 10

        parser.add_data(data).unwrap();
        let result = parser.parse_message().unwrap();

        assert!(result.is_none()); // Should return None for incomplete message
    }
}
