//! Protocol constants and configuration values

/// Default FreeSWITCH ESL port for inbound connections
pub const DEFAULT_ESL_PORT: u16 = 8021;

/// Socket buffer size for reading from TCP stream (64KB) - standard TCP receive window
pub const SOCKET_BUF_SIZE: usize = 65536;

/// Buffer allocation size (64KB) - used for both initial allocation and growth increments
/// Handles 99% of ESL messages without reallocation
pub const BUF_CHUNK: usize = 64 * 1024;

/// Maximum single message size (8MB) - validates Content-Length header
/// No legitimate ESL message should exceed this (largest is sofia status ~1-2MB)
pub const MAX_MESSAGE_SIZE: usize = 8 * 1024 * 1024;

/// Maximum total buffer size (16MB) - safety limit to prevent runaway memory
/// Should hold 2 max messages + overhead. Indicates a bug if exceeded.
pub const MAX_BUFFER_SIZE: usize = 16 * 1024 * 1024;

/// Protocol message terminators
pub const HEADER_TERMINATOR: &str = "\n\n";
pub const LINE_TERMINATOR: &str = "\n";

/// Content-Type header values
pub const CONTENT_TYPE_AUTH_REQUEST: &str = "auth/request";
pub const CONTENT_TYPE_COMMAND_REPLY: &str = "command/reply";
pub const CONTENT_TYPE_API_RESPONSE: &str = "api/response";
pub const CONTENT_TYPE_TEXT_EVENT_PLAIN: &str = "text/event-plain";
pub const CONTENT_TYPE_TEXT_EVENT_JSON: &str = "text/event-json";
pub const CONTENT_TYPE_TEXT_EVENT_XML: &str = "text/event-xml";

/// Header names
pub const HEADER_CONTENT_TYPE: &str = "Content-Type";
pub const HEADER_CONTENT_LENGTH: &str = "Content-Length";
pub const HEADER_REPLY_TEXT: &str = "Reply-Text";
pub const HEADER_EVENT_NAME: &str = "Event-Name";
pub const HEADER_UNIQUE_ID: &str = "Unique-ID";
pub const HEADER_CALLER_UUID: &str = "Caller-Unique-ID";
pub const HEADER_JOB_UUID: &str = "Job-UUID";

/// Connection timeout in milliseconds
pub const DEFAULT_TIMEOUT_MS: u64 = 2000;

/// Maximum number of queued events before dropping
pub const MAX_EVENT_QUEUE_SIZE: usize = 1000;
