//! Protocol constants and configuration values

/// Default FreeSWITCH ESL port for inbound connections
pub const DEFAULT_ESL_PORT: u16 = 8021;

/// Default password for ESL authentication
pub const DEFAULT_PASSWORD: &str = "ClueCon";

/// Socket buffer size for reading from TCP stream (64KB)
pub const SOCKET_BUF_SIZE: usize = 65536;

/// Buffer chunk size for packet processing (3.2MB)
pub const BUF_CHUNK: usize = 65536 * 50;

/// Initial buffer size (6.5MB)  
pub const BUF_START: usize = 65536 * 100;

/// Maximum reply size for command responses (1KB)
pub const MAX_REPLY_SIZE: usize = 1024;

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

/// Reply text values
pub const REPLY_TEXT_OK: &str = "+OK accepted";
pub const REPLY_TEXT_ERR: &str = "-ERR";

/// Header names
pub const HEADER_CONTENT_TYPE: &str = "Content-Type";
pub const HEADER_CONTENT_LENGTH: &str = "Content-Length";
pub const HEADER_REPLY_TEXT: &str = "Reply-Text";
pub const HEADER_EVENT_NAME: &str = "Event-Name";
pub const HEADER_UNIQUE_ID: &str = "Unique-ID";
pub const HEADER_CALLER_UUID: &str = "Caller-Unique-ID";
pub const HEADER_JOB_UUID: &str = "Job-UUID";

/// Connection timeout in milliseconds  
pub const DEFAULT_TIMEOUT_MS: u64 = 25000;

/// Maximum number of queued events before dropping
pub const MAX_EVENT_QUEUE_SIZE: usize = 1000;
