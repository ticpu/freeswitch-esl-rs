//! Connection management for ESL

use std::borrow::Borrow;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::time::{timeout, Instant};
use tracing::{debug, info, trace, warn};

#[cfg(unix)]
use crate::constants::REEXEC_DRAIN_TIMEOUT_MS;
use crate::{
    command::{EslCommand, EslResponse, ExecuteOptions, Secret},
    constants::{
        CONTENT_TYPE_LOG_DATA, DEFAULT_TIMEOUT_MS, HEADER_CONTENT_TYPE, MAX_EVENT_QUEUE_SIZE,
        SOCKET_BUF_SIZE,
    },
    error::{EslError, EslResult},
    event::{EslEvent, EslEventType, EventFormat},
    headers::EventHeader,
    protocol::{EslMessage, EslParser, MessageType},
};

/// Result type for the reexec drain operation (residual bytes or error).
#[cfg(unix)]
type ReexecResult = Result<Vec<u8>, EslError>;

/// Caller-side half of the re-exec channel (stored in `SharedState`).
///
/// Taken by [`EslClient::teardown_for_reexec()`] to signal the reader and
/// receive residual bytes.
#[cfg(unix)]
struct ReexecCaller {
    stop_tx: oneshot::Sender<()>,
    result_rx: oneshot::Receiver<ReexecResult>,
}

/// Reader-side half of the re-exec channel.
///
/// Owned by the reader loop task. The stop receiver fires when teardown is
/// requested; the result sender delivers residual bytes back to the caller.
/// On non-unix platforms this is a zero-size struct (re-exec is unix-only).
struct ReexecReader {
    #[cfg(unix)]
    stop_rx: oneshot::Receiver<()>,
    #[cfg(unix)]
    result_tx: Option<oneshot::Sender<ReexecResult>>,
}

fn event_types_to_string<T: Borrow<EslEventType>>(events: impl IntoIterator<Item = T>) -> String {
    events
        .into_iter()
        .map(|e| {
            e.borrow()
                .as_str()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Connection status for ESL client
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectionStatus {
    /// ESL session is active.
    Connected,
    /// ESL session ended.
    Disconnected(DisconnectReason),
}

/// Reason for disconnection
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DisconnectReason {
    /// Server sent a `text/disconnect-notice` with `Content-Disposition: disconnect`.
    ///
    /// The notice may include a `Controlled-Session-UUID` header identifying
    /// the session and a body with a human-readable message.
    ServerNotice {
        /// UUID of the controlled session, if present in the notice.
        controlled_session_uuid: Option<String>,
        /// Body text from the disconnect notice (e.g. "Disconnected, goodbye.").
        body: Option<String>,
    },
    /// Liveness timeout exceeded without any inbound traffic
    HeartbeatExpired,
    /// TCP I/O error (io::Error is not Clone, so we store the message)
    IoError(String),
    /// Clean EOF on the TCP connection
    ConnectionClosed,
    /// Client called disconnect()
    ClientRequested,
    /// Client initiated re-exec teardown
    #[cfg(unix)]
    ReexecTeardown,
    /// Protocol-level desync (e.g. unsolicited auth/request after login)
    ProtocolError(String),
    /// Connection rejected by ACL (`text/rude-rejection`)
    AccessDenied(String),
}

impl std::fmt::Display for DisconnectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DisconnectReason::ServerNotice {
                controlled_session_uuid,
                ..
            } => {
                if let Some(uuid) = controlled_session_uuid {
                    write!(f, "server sent disconnect notice (session {})", uuid)
                } else {
                    write!(f, "server sent disconnect notice")
                }
            }
            DisconnectReason::HeartbeatExpired => write!(f, "liveness timeout expired"),
            DisconnectReason::IoError(msg) => write!(f, "I/O error: {}", msg),
            DisconnectReason::ConnectionClosed => write!(f, "connection closed"),
            DisconnectReason::ClientRequested => write!(f, "client requested disconnect"),
            #[cfg(unix)]
            DisconnectReason::ReexecTeardown => write!(f, "re-exec teardown"),
            DisconnectReason::ProtocolError(msg) => write!(f, "protocol error: {}", msg),
            DisconnectReason::AccessDenied(msg) => write!(f, "access denied: {}", msg),
        }
    }
}

/// Establish a TCP connection with a timeout.
async fn tcp_connect_with_timeout(
    host: &str,
    port: u16,
    connect_timeout: Duration,
) -> EslResult<TcpStream> {
    let timeout_ms = connect_timeout.as_millis() as u64;
    let tcp_result = timeout(connect_timeout, TcpStream::connect((host, port))).await;

    match tcp_result {
        Ok(Ok(s)) => {
            debug!("[CONNECT] TCP connection established");
            Ok(s)
        }
        Ok(Err(e)) => {
            warn!("[CONNECT] TCP connect failed: {}", e);
            Err(EslError::Io(e))
        }
        Err(_) => {
            warn!("[CONNECT] TCP connect timed out after {}ms", timeout_ms);
            Err(EslError::Timeout { timeout_ms })
        }
    }
}

/// Connection mode for ESL
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectionMode {
    /// Inbound connection - client connects to FreeSWITCH
    Inbound,
    /// Outbound connection - FreeSWITCH connects to client
    Outbound,
}

/// Default command timeout in milliseconds (5 seconds)
const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 5000;

/// Slot for the in-flight command waiter and its stale-reply counter.
///
/// ESL is a serial protocol with no correlation IDs. After a command times
/// out the server may still send the late reply. `stale_replies` tracks how
/// many such late replies to silently discard so the next command's reply
/// is not consumed by the wrong waiter.
///
/// Invariant: `stale_replies > 0` only while `waiting` is `None` (the timed-
/// out sender was already taken at timeout; the count tells the reader how
/// many more unsolicited replies to skip before resuming normal dispatch).
struct PendingReply {
    /// Waiter for the currently in-flight command (`None` between commands).
    waiting: Option<oneshot::Sender<EslMessage>>,
    /// Number of stale replies to discard before resuming normal dispatch.
    stale_replies: u32,
}

impl PendingReply {
    fn new() -> Self {
        Self {
            waiting: None,
            stale_replies: 0,
        }
    }
}

/// Shared state between EslClient and the reader task
struct SharedState {
    pending_reply: Mutex<PendingReply>,
    /// Connection status sender (shared so disconnect() can set ClientRequested)
    status_tx: watch::Sender<ConnectionStatus>,
    /// Liveness timeout in milliseconds (0 = disabled)
    liveness_timeout_ms: AtomicU64,
    /// Command response timeout in milliseconds
    command_timeout_ms: AtomicU64,
    /// Set when events have been dropped due to a full queue
    event_overflow: AtomicBool,
    /// Total count of dropped events
    dropped_event_count: AtomicU64,
    /// Auth response from inbound connect (None for outbound)
    auth_response: Option<EslResponse>,
    /// Re-exec channel caller half (taken by teardown_for_reexec)
    #[cfg(unix)]
    reexec: Mutex<Option<ReexecCaller>>,
}

/// Options for ESL connection configuration.
///
/// Controls parameters that are fixed at connection time, such as the event
/// queue capacity and connect timeout. Use [`Default::default()`] for standard settings.
#[derive(Debug, Clone)]
pub struct EslConnectOptions {
    event_queue_size: usize,
    connect_timeout: Duration,
    strict_header_utf8: bool,
}

impl EslConnectOptions {
    /// Create default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the event queue capacity.
    pub fn with_event_queue_size(mut self, size: usize) -> Self {
        self.event_queue_size = size;
        self
    }

    /// Set the timeout for TCP connect and auth handshake.
    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Strict UTF-8 validation on event-body header values.
    ///
    /// When `true`, invalid UTF-8 in percent-decoded event-body values returns
    /// `EslError::InvalidUtf8InHeader` and stops the stream. When `false`
    /// (default), invalid bytes are decoded lossily (U+FFFD) and surfaced via
    /// `EslEvent::lossy_values()` for inspection/recovery.
    pub fn with_strict_header_utf8(mut self, strict: bool) -> Self {
        self.strict_header_utf8 = strict;
        self
    }

    /// Capacity of the mpsc channel delivering events. Default: 1000.
    pub fn event_queue_size(&self) -> usize {
        self.event_queue_size
    }

    /// Timeout for TCP connect and each auth handshake read. Default: 2s.
    pub fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    /// Whether to fail on invalid UTF-8 in event-body values. Default: false.
    pub fn strict_header_utf8(&self) -> bool {
        self.strict_header_utf8
    }
}

impl Default for EslConnectOptions {
    fn default() -> Self {
        Self {
            event_queue_size: MAX_EVENT_QUEUE_SIZE,
            connect_timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            strict_header_utf8: false,
        }
    }
}

/// Authentication method for inbound connections.
enum AuthMethod<'a> {
    Password(&'a str),
    User { user: &'a str, password: &'a str },
}

/// ESL client handle (Clone + Send)
///
/// Commands are serialized through the writer mutex. The reader task
/// routes replies to the pending oneshot channel.
#[derive(Clone)]
pub struct EslClient {
    writer: Arc<Mutex<OwnedWriteHalf>>,
    shared: Arc<SharedState>,
    status_rx: watch::Receiver<ConnectionStatus>,
}

impl std::fmt::Debug for EslClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EslClient")
            .field("connected", &self.is_connected())
            .finish()
    }
}

/// Event stream receiver (!Clone)
///
/// Receives events from the background reader task via an mpsc channel.
///
/// Events are delivered as `Result<EslEvent, EslError>`. An `Err(EslError::QueueFull)`
/// indicates that one or more events were dropped because the application fell behind.
/// Use [`EslClient::dropped_event_count`] for the exact count.
pub struct EslEventStream {
    rx: mpsc::Receiver<Result<EslEvent, EslError>>,
    status_rx: watch::Receiver<ConnectionStatus>,
}

impl std::fmt::Debug for EslEventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EslEventStream")
            .field("connected", &self.is_connected())
            .finish()
    }
}

/// Read a single ESL message from the socket into the parser.
///
/// Used during auth handshake (on unsplit TcpStream) and would be the
/// basis for the reader loop, but the reader loop inlines this logic
/// to handle liveness tracking.
async fn recv_message(
    stream: &mut TcpStream,
    parser: &mut EslParser,
    read_buffer: &mut [u8],
    read_timeout: Duration,
) -> EslResult<EslMessage> {
    let timeout_ms = read_timeout.as_millis() as u64;
    loop {
        if let Some(message) = parser.parse_message()? {
            trace!(
                "[RECV] Parsed message from buffer: {:?}",
                message.message_type
            );
            return Ok(message);
        }

        trace!("[RECV] Buffer needs more data, reading from socket");
        let read_result = timeout(read_timeout, stream.read(read_buffer)).await;

        let bytes_read = match read_result {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(EslError::Io(e)),
            Err(_) => return Err(EslError::Timeout { timeout_ms }),
        };

        trace!("[RECV] Read {} bytes from socket", bytes_read);
        if bytes_read == 0 {
            return Err(EslError::ConnectionClosed);
        }

        parser.add_data(&read_buffer[..bytes_read])?;
    }
}

/// Perform authentication on the stream, returning the auth response.
///
/// For `userauth`, the response contains `Allowed-Events`, `Allowed-API`,
/// and `Allowed-LOG` headers describing the user's access policy.
async fn authenticate(
    stream: &mut TcpStream,
    parser: &mut EslParser,
    read_buffer: &mut [u8],
    method: AuthMethod<'_>,
    connect_timeout: Duration,
) -> EslResult<EslResponse> {
    debug!("[AUTH] Waiting for auth request from FreeSWITCH");
    let message = recv_message(stream, parser, read_buffer, connect_timeout).await?;

    if message.message_type == MessageType::RudeRejection {
        let reason = message
            .body
            .unwrap_or_else(|| "rude-rejection without body".to_string());
        return Err(EslError::AccessDenied { reason });
    }

    if message.message_type != MessageType::AuthRequest {
        return Err(EslError::protocol_error("Expected auth request"));
    }

    let auth_cmd = match method {
        AuthMethod::Password(password) => EslCommand::Auth {
            password: Secret(password.to_string()),
        },
        AuthMethod::User { user, password } => EslCommand::UserAuth {
            user: user.to_string(),
            password: Secret(password.to_string()),
        },
    };

    let command_str = auth_cmd.to_wire_format()?;
    debug!(">> {}", auth_cmd.redact_wire(&command_str));
    stream
        .write_all(command_str.as_bytes())
        .await
        .map_err(EslError::Io)?;

    let response_msg = match recv_message(stream, parser, read_buffer, connect_timeout).await {
        Ok(msg) => msg,
        Err(EslError::Timeout { timeout_ms }) => {
            // FreeSWITCH mod_event_socket.c uses char reply[512] for the
            // userauth response. When Allowed-Events is long, switch_snprintf
            // truncates the output and the \n\n terminator is never sent.
            match salvage_truncated_auth_response(parser) {
                Ok(Some(msg)) => {
                    warn!(
                        "FreeSWITCH sent a truncated auth response \
                         (mod_event_socket.c reply[512] overflow). \
                         Allowed-API/Allowed-LOG headers may be incomplete or missing."
                    );
                    msg
                }
                Ok(None) => return Err(EslError::Timeout { timeout_ms }),
                Err(e) => {
                    debug!("Truncated auth response salvage failed: {}", e);
                    return Err(EslError::Timeout { timeout_ms });
                }
            }
        }
        Err(e) => return Err(e),
    };
    let response = response_msg.into_response();

    if !response.is_success() {
        return Err(match response.reply_text() {
            Some(text) => EslError::auth_failed(text.to_string()),
            None => EslError::protocol_error("auth response missing Reply-Text header"),
        });
    }

    debug!("Authentication successful");
    Ok(response)
}

/// Salvage a truncated `userauth` response from the parser buffer.
///
/// FreeSWITCH `mod_event_socket.c` formats the userauth reply into
/// `char reply[512]`. When the `Allowed-Events` list is long,
/// `switch_snprintf` truncates the output and the `\n\n` terminator
/// the parser expects is never written, causing a read timeout.
///
/// This function extracts whatever headers arrived before the
/// truncation point. Only valid during the auth handshake.
fn salvage_truncated_auth_response(parser: &mut EslParser) -> EslResult<Option<EslMessage>> {
    if !parser.is_waiting_for_headers() {
        return Ok(None);
    }

    let data = parser.remaining_bytes();
    if data.is_empty() {
        return Ok(None);
    }

    let data_str = std::str::from_utf8(data)
        .map_err(|_| EslError::protocol_error("invalid UTF-8 in truncated auth response"))?;

    // Find the last newline. Everything before it is complete header lines.
    // Everything after may be a truncated line.
    let last_nl = match data_str.rfind('\n') {
        Some(pos) => pos,
        None => return Ok(None),
    };

    let trailing = &data_str[last_nl + 1..];

    // Build the header block to parse: all complete lines, plus the
    // trailing fragment only if it contains a colon (truncated value
    // is acceptable; truncated header name is not).
    let header_block = if trailing.is_empty() || trailing.contains(':') {
        data_str
    } else {
        &data_str[..last_nl]
    };

    let (headers, lossy_values) = parser.parse_headers(header_block)?;

    // Validate this is actually a command/reply (auth response).
    let content_type = headers
        .get(HEADER_CONTENT_TYPE)
        .ok_or_else(|| {
            EslError::protocol_error(
                "truncated response missing Content-Type header — not an auth response",
            )
        })?;

    if content_type != crate::constants::CONTENT_TYPE_COMMAND_REPLY {
        return Err(EslError::protocol_error(format!(
            "truncated response has Content-Type '{}', expected command/reply",
            content_type
        )));
    }

    let message_type = MessageType::from_content_type(content_type)?;
    let message = EslMessage::new(message_type, headers, None).with_lossy_values(lossy_values);

    parser.drain_buffer();

    Ok(Some(message))
}

/// Try to send an event (or error) to the application via try_send.
///
/// If the channel is full, drop the item, set the overflow flag, and
/// increment the dropped counter. Before each dispatch, check the overflow
/// flag and attempt to deliver a QueueFull error notification first.
fn dispatch_event(
    event_tx: &mpsc::Sender<Result<EslEvent, EslError>>,
    shared: &SharedState,
    item: Result<EslEvent, EslError>,
) -> bool {
    if shared
        .event_overflow
        .load(Ordering::Relaxed)
    {
        match event_tx.try_send(Err(EslError::QueueFull)) {
            Ok(()) => {
                shared
                    .event_overflow
                    .store(false, Ordering::Relaxed);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => return false,
            Err(mpsc::error::TrySendError::Full(_)) => {}
        }
    }

    match event_tx.try_send(item) {
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Closed(_)) => false,
        Err(mpsc::error::TrySendError::Full(_)) => {
            shared
                .event_overflow
                .store(true, Ordering::Relaxed);
            shared
                .dropped_event_count
                .fetch_add(1, Ordering::Relaxed);
            warn!("Event queue full, dropping event");
            true
        }
    }
}

/// Background reader loop
async fn reader_loop(
    reader: OwnedReadHalf,
    parser: EslParser,
    shared: Arc<SharedState>,
    event_tx: mpsc::Sender<Result<EslEvent, EslError>>,
    reexec: ReexecReader,
) {
    let result = std::panic::AssertUnwindSafe(reader_loop_inner(
        reader,
        parser,
        shared.clone(),
        event_tx,
        reexec,
    ));
    if futures_util::FutureExt::catch_unwind(result)
        .await
        .is_err()
    {
        tracing::error!("reader task panicked");
        let _ = shared
            .status_tx
            .send(ConnectionStatus::Disconnected(DisconnectReason::IoError(
                "reader task panicked".to_string(),
            )));
    }
}

async fn reader_loop_inner(
    mut reader: OwnedReadHalf,
    mut parser: EslParser,
    shared: Arc<SharedState>,
    event_tx: mpsc::Sender<Result<EslEvent, EslError>>,
    #[cfg_attr(not(unix), allow(unused_variables, unused_mut))] mut reexec: ReexecReader,
) {
    let mut read_buffer = [0u8; SOCKET_BUF_SIZE];
    let mut last_recv = Instant::now();
    #[cfg(unix)]
    let mut draining = false;

    loop {
        // Try to parse a complete message from buffered data first
        match parser.parse_message() {
            Ok(Some(message)) => {
                match message.message_type {
                    MessageType::Event => {
                        let ct = message
                            .headers
                            .get(HEADER_CONTENT_TYPE)
                            .map(|s| s.as_str());

                        // log/data uses single-level framing handled inside
                        // parse_event; skip the format check for it.
                        let format = if ct == Some(CONTENT_TYPE_LOG_DATA) {
                            EventFormat::Plain
                        } else {
                            match ct.map(EventFormat::from_content_type) {
                                Some(Ok(f)) => f,
                                Some(Err(e)) => {
                                    warn!("Unknown event content type: {}", e);
                                    if !dispatch_event(
                                        &event_tx,
                                        &shared,
                                        Err(EslError::InvalidEventFormat {
                                            format: e
                                                .0
                                                .clone(),
                                        }),
                                    ) {
                                        return;
                                    }
                                    continue;
                                }
                                None => EventFormat::Plain,
                            }
                        };

                        let event_result = parser.parse_event(message, format);
                        if !dispatch_event(&event_tx, &shared, event_result) {
                            debug!("Event channel closed, reader exiting");
                            return;
                        }
                    }
                    MessageType::CommandReply | MessageType::ApiResponse => {
                        let mut pending = shared
                            .pending_reply
                            .lock()
                            .await;
                        if pending.stale_replies > 0 {
                            // A previous command timed out and its server reply
                            // arrived late. Discard to preserve correlation.
                            pending.stale_replies -= 1;
                            let reply_text = message
                                .headers
                                .get("Reply-Text")
                                .map(|s| s.as_str())
                                .unwrap_or("<none>");
                            warn!(
                                "Discarded stale {:?} reply (Reply-Text: {}) to preserve \
                                 command-reply correlation; {} stale replies remaining",
                                message.message_type, reply_text, pending.stale_replies,
                            );
                        } else if let Some(tx) = pending
                            .waiting
                            .take()
                        {
                            if tx
                                .send(message)
                                .is_err()
                            {
                                // Caller's receiver was dropped: timeout raced the
                                // delivery. The timeout arm did not increment
                                // stale_replies (it saw waiting=None), so this reply
                                // is already accounted for — no counter adjustment.
                                debug!(
                                    "Reply channel closed before delivery (timeout race); \
                                     reply discarded"
                                );
                            }
                        } else {
                            warn!(
                                "Received unsolicited {:?} with no pending command",
                                message.message_type,
                            );
                        }
                    }
                    MessageType::Disconnect => {
                        let disposition = message
                            .headers
                            .get("Content-Disposition")
                            .map(|s| s.as_str());
                        if disposition == Some("linger") {
                            debug!("Received disconnect notice with linger disposition, ignoring");
                            continue;
                        }
                        let controlled_session_uuid = message
                            .headers
                            .get("Controlled-Session-UUID")
                            .cloned();
                        info!("Received disconnect notice from server");
                        let _ = shared
                            .status_tx
                            .send(ConnectionStatus::Disconnected(
                                DisconnectReason::ServerNotice {
                                    controlled_session_uuid,
                                    body: message.body,
                                },
                            ));
                        return;
                    }
                    MessageType::RudeRejection => {
                        let reason = message
                            .body
                            .unwrap_or_else(|| "rude-rejection without body".to_string());
                        warn!("Rude rejection from server: {}", reason);
                        let _ = dispatch_event(
                            &event_tx,
                            &shared,
                            Err(EslError::AccessDenied {
                                reason: reason.clone(),
                            }),
                        );
                        let _ = shared
                            .status_tx
                            .send(ConnectionStatus::Disconnected(
                                DisconnectReason::AccessDenied(reason),
                            ));
                        return;
                    }
                    MessageType::AuthRequest => {
                        // After authentication, an unsolicited auth/request
                        // means FreeSWITCH and the client are out of sync —
                        // treat as a protocol error and tear down.
                        let reason =
                            "unsolicited auth/request received after authentication".to_string();
                        warn!("{reason}");
                        let _ = dispatch_event(
                            &event_tx,
                            &shared,
                            Err(EslError::protocol_error(reason.clone())),
                        );
                        let _ = shared
                            .status_tx
                            .send(ConnectionStatus::Disconnected(
                                DisconnectReason::ProtocolError(reason),
                            ));
                        return;
                    }
                }
                continue;
            }
            Ok(None) => {
                // Need more data from socket
            }
            Err(e) => {
                warn!("Parser error: {}", e);
                let _ = shared
                    .status_tx
                    .send(ConnectionStatus::Disconnected(
                        DisconnectReason::ProtocolError(e.to_string()),
                    ));
                return;
            }
        }

        // Re-exec drain completion: parser needs more data and is between messages
        #[cfg(unix)]
        if draining && parser.is_waiting_for_headers() {
            let residual = parser
                .remaining_bytes()
                .to_vec();
            debug!("Re-exec drain complete, {} residual bytes", residual.len());
            // Broadcast ReexecTeardown BEFORE returning so the status watch is
            // updated before event_tx is dropped. Without this, consumers that
            // see recv()→None and check events.status() would find stale
            // Connected and treat it as an unexpected connection loss.
            let _ = shared
                .status_tx
                .send(ConnectionStatus::Disconnected(
                    DisconnectReason::ReexecTeardown,
                ));
            if let Some(tx) = reexec
                .result_tx
                .take()
            {
                let _ = tx.send(Ok(residual));
            }
            return;
        }

        // Re-exec drain: WaitingForBody, need more socket data to finish the
        // current message before we can stop cleanly.
        #[cfg(unix)]
        if draining {
            let drain_timeout = Duration::from_millis(REEXEC_DRAIN_TIMEOUT_MS);
            match timeout(drain_timeout, reader.read(&mut read_buffer)).await {
                Ok(Ok(0)) => {
                    warn!("Connection closed during re-exec drain");
                    if let Some(tx) = reexec
                        .result_tx
                        .take()
                    {
                        let _ = tx.send(Err(EslError::ReexecFailed {
                            reason: "connection closed during drain".into(),
                        }));
                    }
                    return;
                }
                Ok(Ok(n)) => {
                    if let Err(e) = parser.add_data(&read_buffer[..n]) {
                        if let Some(tx) = reexec
                            .result_tx
                            .take()
                        {
                            let _ = tx.send(Err(e));
                        }
                        return;
                    }
                }
                Ok(Err(e)) => {
                    warn!("Read error during re-exec drain: {}", e);
                    if let Some(tx) = reexec
                        .result_tx
                        .take()
                    {
                        let _ = tx.send(Err(EslError::Io(e)));
                    }
                    return;
                }
                Err(_) => {
                    warn!("Re-exec drain timeout waiting for message body");
                    if let Some(tx) = reexec
                        .result_tx
                        .take()
                    {
                        let _ = tx.send(Err(EslError::ReexecFailed {
                            reason: "drain timeout waiting for message body".into(),
                        }));
                    }
                    return;
                }
            }
            continue;
        }

        // Normal read path with optional reexec stop signal
        #[cfg(unix)]
        let read_result = tokio::select! {
            biased;
            _ = &mut reexec.stop_rx, if !draining => {
                debug!("Re-exec stop signal received, draining parser");
                draining = true;
                continue;
            }
            result = timeout(Duration::from_secs(2), reader.read(&mut read_buffer)) => result,
        };

        #[cfg(not(unix))]
        let read_result = timeout(Duration::from_secs(2), reader.read(&mut read_buffer)).await;

        match read_result {
            Ok(Ok(0)) => {
                info!("Connection closed (EOF)");
                let _ = shared
                    .status_tx
                    .send(ConnectionStatus::Disconnected(
                        DisconnectReason::ConnectionClosed,
                    ));
                return;
            }
            Ok(Ok(n)) => {
                last_recv = Instant::now();
                if let Err(e) = parser.add_data(&read_buffer[..n]) {
                    warn!("Buffer error: {}", e);
                    let _ = shared
                        .status_tx
                        .send(ConnectionStatus::Disconnected(
                            DisconnectReason::ProtocolError(e.to_string()),
                        ));
                    return;
                }
            }
            Ok(Err(e)) => {
                warn!("Read error: {}", e);
                let _ = shared
                    .status_tx
                    .send(ConnectionStatus::Disconnected(DisconnectReason::IoError(
                        e.to_string(),
                    )));
                return;
            }
            Err(_) => {
                // Timeout -- check liveness
                let threshold_ms = shared
                    .liveness_timeout_ms
                    .load(Ordering::Relaxed);
                if threshold_ms > 0 {
                    let elapsed = last_recv.elapsed();
                    if elapsed > Duration::from_millis(threshold_ms) {
                        warn!(
                            "Liveness timeout: {}ms without traffic (threshold {}ms)",
                            elapsed.as_millis(),
                            threshold_ms
                        );
                        let _ = shared
                            .status_tx
                            .send(ConnectionStatus::Disconnected(
                                DisconnectReason::HeartbeatExpired,
                            ));
                        return;
                    }
                }
            }
        }
    }
}

impl EslClient {
    /// Connect to FreeSWITCH (inbound mode) with password authentication
    pub async fn connect(
        host: &str,
        port: u16,
        password: &str,
    ) -> EslResult<(Self, EslEventStream)> {
        Self::connect_inner(
            host,
            port,
            AuthMethod::Password(password),
            EslConnectOptions::default(),
        )
        .await
    }

    /// Connect to FreeSWITCH (inbound mode) with password authentication and custom options
    pub async fn connect_with_options(
        host: &str,
        port: u16,
        password: &str,
        options: EslConnectOptions,
    ) -> EslResult<(Self, EslEventStream)> {
        Self::connect_inner(host, port, AuthMethod::Password(password), options).await
    }

    /// Connect with user authentication
    ///
    /// The user must be in the format `user@domain` (e.g., `admin@default`).
    pub async fn connect_with_user(
        host: &str,
        port: u16,
        user: &str,
        password: &str,
    ) -> EslResult<(Self, EslEventStream)> {
        Self::connect_inner(
            host,
            port,
            AuthMethod::User { user, password },
            EslConnectOptions::default(),
        )
        .await
    }

    /// Connect with user authentication and custom options
    ///
    /// The user must be in the format `user@domain` (e.g., `admin@default`).
    pub async fn connect_with_user_and_options(
        host: &str,
        port: u16,
        user: &str,
        password: &str,
        options: EslConnectOptions,
    ) -> EslResult<(Self, EslEventStream)> {
        Self::connect_inner(host, port, AuthMethod::User { user, password }, options).await
    }

    async fn connect_inner(
        host: &str,
        port: u16,
        method: AuthMethod<'_>,
        options: EslConnectOptions,
    ) -> EslResult<(Self, EslEventStream)> {
        if let AuthMethod::User { user, .. } = &method {
            if !user.contains('@') {
                return Err(EslError::auth_failed(format!(
                    "Invalid username format '{}': must be user@domain (e.g., admin@default)",
                    user
                )));
            }
        }

        info!("Connecting to FreeSWITCH at {}:{}", host, port);

        let connect_timeout = options.connect_timeout;
        let mut stream = tcp_connect_with_timeout(host, port, connect_timeout).await?;
        let mut parser = EslParser::new().with_strict_header_utf8(options.strict_header_utf8());
        let mut read_buffer = [0u8; SOCKET_BUF_SIZE];

        let auth_response = authenticate(
            &mut stream,
            &mut parser,
            &mut read_buffer,
            method,
            connect_timeout,
        )
        .await?;

        info!("Successfully connected and authenticated to FreeSWITCH");
        Ok(Self::split_and_spawn_with_options(
            stream,
            parser,
            options,
            Some(auth_response),
        ))
    }

    /// Accept outbound connection from FreeSWITCH.
    ///
    /// After `accept_outbound`, you MUST call [`Self::connect_session`] before
    /// any other command. Calling [`Self::api`], [`Self::subscribe_events`],
    /// etc. before `connect_session()` will leave the channel in an undefined
    /// state. See [`docs/outbound-esl-quirks.md`](https://github.com/ticpu/freeswitch-esl-tokio/blob/master/docs/outbound-esl-quirks.md)
    /// for full context.
    pub async fn accept_outbound(listener: &TcpListener) -> EslResult<(Self, EslEventStream)> {
        Self::accept_outbound_with_options(listener, EslConnectOptions::default()).await
    }

    /// Accept outbound connection from FreeSWITCH with custom options.
    ///
    /// After accepting, you MUST call [`Self::connect_session`] before any
    /// other command. Calling [`Self::api`], [`Self::subscribe_events`], etc.
    /// before `connect_session()` will leave the channel in an undefined
    /// state. See [`docs/outbound-esl-quirks.md`](https://github.com/ticpu/freeswitch-esl-tokio/blob/master/docs/outbound-esl-quirks.md)
    /// for full context.
    pub async fn accept_outbound_with_options(
        listener: &TcpListener,
        options: EslConnectOptions,
    ) -> EslResult<(Self, EslEventStream)> {
        info!("Waiting for outbound connection from FreeSWITCH");

        let (stream, addr) = listener
            .accept()
            .await
            .map_err(EslError::Io)?;
        info!("Accepted outbound connection from {}", addr);

        Ok(Self::accept_outbound_stream_with_options(stream, options))
    }

    /// Create an outbound-mode client from an already-accepted `TcpStream`.
    ///
    /// Use this when you need control over the accept step (e.g. for
    /// timeouts, TLS wrapping, or custom accept logic).
    ///
    /// After this call, you MUST call [`Self::connect_session`] before any
    /// other command. Calling [`Self::api`], [`Self::subscribe_events`], etc.
    /// before `connect_session()` will leave the channel in an undefined
    /// state. See [`docs/outbound-esl-quirks.md`](https://github.com/ticpu/freeswitch-esl-tokio/blob/master/docs/outbound-esl-quirks.md)
    /// for full context.
    pub fn accept_outbound_stream(stream: TcpStream) -> (Self, EslEventStream) {
        Self::accept_outbound_stream_with_options(stream, EslConnectOptions::default())
    }

    /// Create an outbound-mode client from an already-accepted `TcpStream`
    /// with custom options.
    ///
    /// After this call, you MUST call [`Self::connect_session`] before any
    /// other command. Calling [`Self::api`], [`Self::subscribe_events`], etc.
    /// before `connect_session()` will leave the channel in an undefined
    /// state. See [`docs/outbound-esl-quirks.md`](https://github.com/ticpu/freeswitch-esl-tokio/blob/master/docs/outbound-esl-quirks.md)
    /// for full context.
    pub fn accept_outbound_stream_with_options(
        stream: TcpStream,
        options: EslConnectOptions,
    ) -> (Self, EslEventStream) {
        Self::split_and_spawn_with_options(
            stream,
            EslParser::new().with_strict_header_utf8(options.strict_header_utf8()),
            options,
            None,
        )
    }

    /// Create an `EslClient` from an already-authenticated TCP stream.
    ///
    /// Used after re-exec: the previous process already authenticated and
    /// subscribed to events. The `residual_bytes` are any unparsed data
    /// left in the previous parser buffer (from
    /// [`teardown_for_reexec()`](Self::teardown_for_reexec)).
    ///
    /// FreeSWITCH server-side event subscriptions survive across `exec()`.
    /// Events will arrive immediately, so the caller should be ready to
    /// consume them before calling this method.
    pub fn adopt_stream(
        stream: TcpStream,
        residual_bytes: &[u8],
    ) -> EslResult<(Self, EslEventStream)> {
        Self::adopt_stream_with_options(stream, residual_bytes, EslConnectOptions::default())
    }

    /// Create an `EslClient` from an already-authenticated TCP stream
    /// with custom options.
    ///
    /// See [`adopt_stream()`](Self::adopt_stream) for details.
    pub fn adopt_stream_with_options(
        stream: TcpStream,
        residual_bytes: &[u8],
        options: EslConnectOptions,
    ) -> EslResult<(Self, EslEventStream)> {
        let mut parser = EslParser::new().with_strict_header_utf8(options.strict_header_utf8());
        if !residual_bytes.is_empty() {
            parser.add_data(residual_bytes)?;
        }
        Ok(Self::split_and_spawn_with_options(
            stream, parser, options, None,
        ))
    }

    fn split_and_spawn_with_options(
        stream: TcpStream,
        parser: EslParser,
        options: EslConnectOptions,
        auth_response: Option<EslResponse>,
    ) -> (Self, EslEventStream) {
        let queue_size = options
            .event_queue_size
            .max(1);

        let (read_half, write_half) = stream.into_split();

        let (status_tx, status_rx) = watch::channel(ConnectionStatus::Connected);
        let status_rx2 = status_tx.subscribe();

        #[cfg(unix)]
        let (stop_tx, stop_rx) = oneshot::channel();
        #[cfg(unix)]
        let (result_tx, result_rx) = oneshot::channel();

        let shared = Arc::new(SharedState {
            pending_reply: Mutex::new(PendingReply::new()),
            status_tx,
            liveness_timeout_ms: AtomicU64::new(0),
            command_timeout_ms: AtomicU64::new(DEFAULT_COMMAND_TIMEOUT_MS),
            event_overflow: AtomicBool::new(false),
            dropped_event_count: AtomicU64::new(0),
            auth_response,
            #[cfg(unix)]
            reexec: Mutex::new(Some(ReexecCaller { stop_tx, result_rx })),
        });
        let (event_tx, event_rx) = mpsc::channel(queue_size);

        #[cfg(unix)]
        let reexec_reader = ReexecReader {
            stop_rx,
            result_tx: Some(result_tx),
        };
        #[cfg(not(unix))]
        let reexec_reader = ReexecReader {};

        tokio::spawn(reader_loop(
            read_half,
            parser,
            shared.clone(),
            event_tx,
            reexec_reader,
        ));

        let client = EslClient {
            writer: Arc::new(Mutex::new(write_half)),
            shared,
            status_rx,
        };

        let stream = EslEventStream {
            rx: event_rx,
            status_rx: status_rx2,
        };

        (client, stream)
    }

    /// Send a command and wait for the reply.
    ///
    /// The writer lock is held through the entire send-and-receive cycle to
    /// prevent concurrent commands from overwriting the pending reply slot
    /// (ESL is a sequential request/response protocol).
    pub async fn send_command(&self, command: EslCommand) -> EslResult<EslResponse> {
        if !self.is_connected() {
            return Err(EslError::NotConnected);
        }

        let command_str = command.to_wire_format()?;
        debug!(">> {}", command.redact_wire(&command_str));

        // Lock writer -- serializes concurrent commands and holds through reply.
        let mut writer = self
            .writer
            .lock()
            .await;

        // Set up reply channel
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self
                .shared
                .pending_reply
                .lock()
                .await;
            pending.waiting = Some(tx);
        }

        // Write command
        writer
            .write_all(command_str.as_bytes())
            .await
            .map_err(EslError::Io)?;

        // Wait for reply from reader task with command timeout (writer still locked)
        let timeout_ms = self
            .shared
            .command_timeout_ms
            .load(Ordering::Relaxed);
        let message = match timeout(Duration::from_millis(timeout_ms), rx).await {
            Ok(Ok(message)) => message,
            Ok(Err(e)) => {
                debug!("pending reply channel closed: {e}");
                drop(writer);
                return Err(EslError::ConnectionClosed);
            }
            Err(_) => {
                let mut pending = self
                    .shared
                    .pending_reply
                    .lock()
                    .await;
                // waiting.take() == Some means the reader has NOT yet consumed
                // this sender — the server reply is still in flight and will
                // arrive at the next waiter. Count it so the reader can discard
                // that one stale reply instead of routing it to the next command.
                // waiting.take() == None means the reader already took the sender
                // and raced the timeout (its send() failed into the dropped rx).
                // The reply is already consumed; do not increment.
                if pending
                    .waiting
                    .take()
                    .is_some()
                {
                    pending.stale_replies += 1;
                }
                drop(writer);
                return Err(EslError::Timeout { timeout_ms });
            }
        };

        drop(writer);

        let response = message.into_response();
        debug!("Received response: success={}", response.is_success());
        Ok(response)
    }

    /// Send a command and require a successful response, discarding the body.
    async fn send_command_ok(&self, command: EslCommand) -> EslResult<()> {
        self.send_command(command)
            .await?
            .into_result()
            .map(|_| ())
    }

    /// Execute API command synchronously.
    ///
    /// **Warning: this blocks the entire ESL socket.** FreeSWITCH processes
    /// `api` commands inline -- no events are delivered and no other commands
    /// can be sent on this connection until the command finishes. For commands
    /// that may take a long time (`originate`, `conference`, bulk operations),
    /// use [`bgapi`](Self::bgapi) instead so events keep flowing.
    ///
    /// Use [`EslResponse::api_result`] to parse the response body, or
    /// [`parse_api_body`](crate::parse_api_body) for `BACKGROUND_JOB` event
    /// bodies.
    ///
    /// ```rust,no_run
    /// # async fn example(client: &freeswitch_esl_tokio::EslClient) -> Result<(), freeswitch_esl_tokio::EslError> {
    /// let resp = client.api("status").await?;
    /// let status = resp.api_result()?;
    /// println!("{}", status);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn api(&self, command: &str) -> EslResult<EslResponse> {
        let cmd = EslCommand::Api {
            command: command.to_string(),
        };
        self.send_command(cmd)
            .await
    }

    /// Execute background API command.
    ///
    /// Returns immediately with a `Job-UUID` in the response. The actual result
    /// arrives later as a [`EslEventType::BackgroundJob`] event -- subscribe to it
    /// and correlate via [`HeaderLookup::job_uuid()`](crate::HeaderLookup::job_uuid) / [`EslResponse::job_uuid`]:
    ///
    /// ```rust,no_run
    /// # async fn example(client: &freeswitch_esl_tokio::EslClient) -> Result<(), freeswitch_esl_tokio::EslError> {
    /// let resp = client.bgapi("originate user/1000 &park").await?;
    /// let job_uuid = resp.job_uuid().expect("bgapi returns Job-UUID header");
    /// // Match against event.job_uuid() in the event loop
    /// # Ok(())
    /// # }
    /// ```
    pub async fn bgapi(&self, command: &str) -> EslResult<EslResponse> {
        let cmd = EslCommand::BgApi {
            command: command.to_string(),
        };
        self.send_command(cmd)
            .await
    }

    /// Subscribe to all events.
    ///
    /// Convenience wrapper for `subscribe_events(format, &[EslEventType::All])`.
    ///
    /// ```rust,no_run
    /// # async fn example(client: &freeswitch_esl_tokio::EslClient) -> Result<(), freeswitch_esl_tokio::EslError> {
    /// use freeswitch_esl_tokio::EventFormat;
    /// client.subscribe_all_events(EventFormat::Plain).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe_all_events(&self, format: EventFormat) -> EslResult<()> {
        self.subscribe_events(format, &[EslEventType::All])
            .await
    }

    /// Subscribe to events by typed enum variants.
    ///
    /// To subscribe to all events, use
    /// [`subscribe_all_events`](Self::subscribe_all_events).
    ///
    /// For `CUSTOM` event subclasses (e.g., `sofia::register`), use
    /// [`subscribe_events_raw`](Self::subscribe_events_raw) instead -- this method
    /// sends bare `CUSTOM` which subscribes to **all** custom events:
    ///
    /// ```rust,no_run
    /// # async fn example(client: &freeswitch_esl_tokio::EslClient) -> Result<(), freeswitch_esl_tokio::EslError> {
    /// use freeswitch_esl_tokio::EventFormat;
    /// // Subscribe to specific CUSTOM subclasses:
    /// client.subscribe_events_raw(EventFormat::Plain, "CUSTOM sofia::register sofia::unregister").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe_events<T: Borrow<EslEventType>>(
        &self,
        format: EventFormat,
        events: impl IntoIterator<Item = T>,
    ) -> EslResult<()> {
        let events: Vec<EslEventType> = events
            .into_iter()
            .map(|e| *e.borrow())
            .collect();
        let events_str = if events.contains(&EslEventType::All) {
            "ALL".to_string()
        } else {
            event_types_to_string(events)
        };

        let cmd = EslCommand::Events {
            format: format.to_string(),
            events: events_str,
        };

        self.send_command_ok(cmd)
            .await?;
        info!("Subscribed to events with format {:?}", format);
        Ok(())
    }

    /// Subscribe to events using raw event name strings.
    ///
    /// Use this for `CUSTOM` subclasses or event types not yet covered by
    /// [`EslEventType`]. For typed events, prefer
    /// [`subscribe_events`](Self::subscribe_events) or
    /// [`subscribe_all_events`](Self::subscribe_all_events).
    ///
    /// ```rust,no_run
    /// # async fn example(client: &freeswitch_esl_tokio::EslClient) -> Result<(), freeswitch_esl_tokio::EslError> {
    /// use freeswitch_esl_tokio::EventFormat;
    /// // CUSTOM subclasses require raw strings -- the typed API sends bare CUSTOM
    /// client.subscribe_events_raw(EventFormat::Plain, "CUSTOM sofia::register sofia::unregister").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe_events_raw(&self, format: EventFormat, events: &str) -> EslResult<()> {
        let cmd = EslCommand::Events {
            format: format.to_string(),
            events: events.to_string(),
        };

        self.send_command_ok(cmd)
            .await?;
        info!(
            "Subscribed to raw events '{}' with format {:?}",
            events, format
        );
        Ok(())
    }

    /// Set event filter using a typed [`EventHeader`].
    pub async fn filter(&self, header: EventHeader, value: &str) -> EslResult<()> {
        self.filter_raw(header.as_str(), value)
            .await
    }

    /// Set event filter using a raw header name string.
    ///
    /// Prefer [`filter`](Self::filter) when the header has an [`EventHeader`]
    /// variant. Use this only for headers not yet covered by the typed enum.
    pub async fn filter_raw(&self, header: &str, value: &str) -> EslResult<()> {
        let cmd = EslCommand::Filter {
            header: header.to_string(),
            value: value.to_string(),
        };

        self.send_command_ok(cmd)
            .await?;
        debug!("Set event filter: {} = {}", header, value);
        Ok(())
    }

    /// Apply an [`EventSubscription`](freeswitch_types::EventSubscription) by sending its filters and event command.
    ///
    /// Sends each filter via [`filter_raw`](Self::filter_raw), then subscribes
    /// to the configured events. Does nothing if the subscription is empty.
    ///
    /// FreeSWITCH's `event` command is additive and idempotent -- subscribing
    /// to an event type that is already subscribed is a no-op (the server
    /// stores subscriptions as a boolean-per-type array). It is safe to call
    /// this on a connection that already has subscriptions; new types are
    /// added, existing ones are unaffected.
    ///
    /// The caller retains ownership of the subscription for use during
    /// reconnection.
    pub async fn apply_subscription(
        &self,
        sub: &freeswitch_types::EventSubscription,
    ) -> EslResult<()> {
        for (header, value) in sub.filters() {
            self.filter_raw(header, value)
                .await?;
        }
        if let Some(events_str) = sub.to_event_string() {
            let cmd = EslCommand::Events {
                format: sub
                    .format()
                    .to_string(),
                events: events_str,
            };
            self.send_command_ok(cmd)
                .await?;
        }
        Ok(())
    }

    /// Replace the current subscription with a new one.
    ///
    /// Minimizes the event gap by applying the new subscription *before*
    /// tearing down the old one. The sequence is:
    ///
    /// 1. Send the new `event` command (additive -- no events are lost)
    /// 2. Clear all filters and re-add the new ones
    /// 3. `noevents` + re-send `event` to remove stale event types
    ///
    /// Between steps 2 and 3 there is a brief window where stale event
    /// types from the old subscription may still be delivered. This is
    /// harmless (extra events, not missing events). Filter state is
    /// atomic per `filter delete all` + re-add, so the filter gap is
    /// minimal.
    ///
    /// For truly gap-free transitions where even stale events are
    /// unacceptable, use [`resubscribe_from`](Self::resubscribe_from)
    /// which diffs the old and new subscriptions and uses `nixevent`
    /// to remove only the delta.
    pub async fn resubscribe(&self, sub: &freeswitch_types::EventSubscription) -> EslResult<()> {
        // Step 1: add new event types (additive, no gap)
        if let Some(events_str) = sub.to_event_string() {
            let cmd = EslCommand::Events {
                format: sub
                    .format()
                    .to_string(),
                events: events_str.clone(),
            };
            self.send_command_ok(cmd)
                .await?;
        }

        // Step 2: replace filters
        self.filter_delete_all()
            .await?;
        for (header, value) in sub.filters() {
            self.filter_raw(header, value)
                .await?;
        }

        // Step 3: clear stale event types and re-apply clean
        self.noevents()
            .await?;
        if let Some(events_str) = sub.to_event_string() {
            let cmd = EslCommand::Events {
                format: sub
                    .format()
                    .to_string(),
                events: events_str,
            };
            self.send_command_ok(cmd)
                .await?;
        }

        Ok(())
    }

    /// Replace the current subscription using a diff against the old one.
    ///
    /// Unlike [`resubscribe`](Self::resubscribe), this method produces
    /// no event gap at all. It computes the difference between `old` and
    /// `new` and applies only the minimal changes:
    ///
    /// 1. Subscribe to event types in `new` but not in `old` (additive)
    /// 2. Replace filters (clear all, re-add new)
    /// 3. `nixevent` event types in `old` but not in `new` (selective removal)
    /// 4. `nixevent` custom subclasses in `old` but not in `new`
    ///
    /// Because new subscriptions are applied before old ones are removed,
    /// there is never a moment where a desired event type is unsubscribed.
    ///
    /// The caller must provide the old subscription that was previously
    /// applied. If the old subscription doesn't match what the server
    /// actually has (e.g. after a reconnection), use [`resubscribe`](Self::resubscribe)
    /// instead.
    pub async fn resubscribe_from(
        &self,
        old: &freeswitch_types::EventSubscription,
        new: &freeswitch_types::EventSubscription,
    ) -> EslResult<()> {
        use std::collections::HashSet;

        let old_types: HashSet<EslEventType> = old
            .event_types()
            .iter()
            .copied()
            .collect();
        let new_types: HashSet<EslEventType> = new
            .event_types()
            .iter()
            .copied()
            .collect();

        // Step 1: subscribe to added event types + custom subclasses
        if let Some(events_str) = new.to_event_string() {
            let cmd = EslCommand::Events {
                format: new
                    .format()
                    .to_string(),
                events: events_str,
            };
            self.send_command_ok(cmd)
                .await?;
        }

        // Step 2: replace filters
        self.filter_delete_all()
            .await?;
        for (header, value) in new.filters() {
            self.filter_raw(header, value)
                .await?;
        }

        // Step 3: nixevent removed event types
        let removed_types: Vec<EslEventType> = old_types
            .difference(&new_types)
            .copied()
            .collect();
        if !removed_types.is_empty() {
            self.nixevent(removed_types)
                .await?;
        }

        // Step 4: nixevent removed raw-named events
        let old_raw: HashSet<&str> = old
            .event_types_raw()
            .iter()
            .map(|s| s.as_str())
            .collect();
        let new_raw: HashSet<&str> = new
            .event_types_raw()
            .iter()
            .map(|s| s.as_str())
            .collect();
        let removed_raw: Vec<&&str> = old_raw
            .difference(&new_raw)
            .collect();
        if !removed_raw.is_empty() {
            let nixevent_str = removed_raw
                .iter()
                .map(|s| **s)
                .collect::<Vec<_>>()
                .join(" ");
            self.nixevent_raw(&nixevent_str)
                .await?;
        }

        // Step 5: nixevent removed custom subclasses
        let old_subclasses: HashSet<&str> = old
            .custom_subclass_list()
            .iter()
            .map(|s| s.as_str())
            .collect();
        let new_subclasses: HashSet<&str> = new
            .custom_subclass_list()
            .iter()
            .map(|s| s.as_str())
            .collect();
        let removed_subclasses: Vec<&&str> = old_subclasses
            .difference(&new_subclasses)
            .collect();
        if !removed_subclasses.is_empty() {
            let nixevent_str = removed_subclasses
                .iter()
                .map(|s| **s)
                .collect::<Vec<_>>()
                .join(" ");
            self.nixevent_raw(&format!("CUSTOM {nixevent_str}"))
                .await?;
        }

        Ok(())
    }

    /// Execute application on channel.
    pub async fn execute(
        &self,
        app: &str,
        args: Option<&str>,
        uuid: Option<&str>,
    ) -> EslResult<EslResponse> {
        self.execute_with_options(app, args, uuid, ExecuteOptions::default())
            .await
    }

    /// Execute application on channel with custom options.
    ///
    /// See [`ExecuteOptions`] for available flags (`event-lock`, `async`, `loops`).
    pub async fn execute_with_options(
        &self,
        app: &str,
        args: Option<&str>,
        uuid: Option<&str>,
        options: ExecuteOptions,
    ) -> EslResult<EslResponse> {
        let cmd = EslCommand::Execute {
            app: app.to_string(),
            args: args.map(|s| s.to_string()),
            uuid: uuid.map(|s| s.to_string()),
            options,
        };
        self.send_command(cmd)
            .await
    }

    /// Send message to channel
    pub async fn sendmsg(&self, uuid: Option<&str>, event: EslEvent) -> EslResult<EslResponse> {
        let cmd = EslCommand::SendMsg {
            uuid: uuid.map(|s| s.to_string()),
            event,
        };
        self.send_command(cmd)
            .await
    }

    /// Fire an event into FreeSWITCH's event bus.
    ///
    /// Headers and body from the event are sent as-is (not percent-encoded).
    /// If the event has a `unique-id` header, FreeSWITCH also queues it
    /// directly to that session.
    pub async fn sendevent(&self, event: EslEvent) -> EslResult<EslResponse> {
        self.send_command(EslCommand::SendEvent { event })
            .await
    }

    /// Subscribe to session events (outbound mode, no UUID needed).
    ///
    /// Subscribes to all channel-related events for the attached session.
    /// In outbound mode, FreeSWITCH already knows the session UUID.
    pub async fn myevents(&self, format: EventFormat) -> EslResult<()> {
        let cmd = EslCommand::MyEvents {
            format: format.to_string(),
            uuid: None,
        };
        self.send_command_ok(cmd)
            .await
    }

    /// Subscribe to session events for a specific UUID (inbound mode).
    ///
    /// Subscribes to all channel-related events for the given session UUID.
    /// Use this in inbound mode where no session is attached to the socket.
    pub async fn myevents_uuid(&self, uuid: &str, format: EventFormat) -> EslResult<()> {
        let cmd = EslCommand::MyEvents {
            format: format.to_string(),
            uuid: Some(uuid.to_string()),
        };
        self.send_command_ok(cmd)
            .await
    }

    /// Keep the socket open after the channel hangs up (outbound mode).
    ///
    /// Without linger, the socket closes immediately on hangup. With linger,
    /// FreeSWITCH sends a `text/disconnect-notice` with `Content-Disposition: linger`
    /// and keeps the socket open so the client can drain remaining events.
    ///
    /// Pass `None` for indefinite linger, or `Some(Duration)` for a timeout.
    pub async fn linger(&self, timeout: Option<Duration>) -> EslResult<()> {
        self.send_command_ok(EslCommand::Linger { timeout })
            .await
    }

    /// Cancel linger mode (outbound mode).
    ///
    /// Only effective before the channel hangs up. After the disconnect notice
    /// has been sent, it's too late to cancel.
    pub async fn nolinger(&self) -> EslResult<()> {
        self.send_command_ok(EslCommand::NoLinger)
            .await
    }

    /// Resume dialplan execution when the socket disconnects (outbound mode).
    ///
    /// Without resume, the channel is hung up when the socket application exits.
    /// With resume, FreeSWITCH continues dialplan execution from where it left off.
    pub async fn resume(&self) -> EslResult<()> {
        self.send_command_ok(EslCommand::Resume)
            .await
    }

    /// Establish the outbound session by sending `connect` and receiving channel data.
    ///
    /// In outbound mode, this **must** be the first command sent after
    /// [`accept_outbound`](Self::accept_outbound). FreeSWITCH replies with a
    /// `command/reply` containing all channel variables for the session.
    ///
    /// The returned [`EslResponse`] contains the channel data as headers.
    /// Use [`EslResponse::header()`] to read individual channel variables
    /// (e.g., `Caller-Caller-ID-Number`, `Channel-Name`).
    pub async fn connect_session(&self) -> EslResult<EslResponse> {
        self.send_command(EslCommand::Connect)
            .await
    }

    /// Unsubscribe from specific events by typed enum variants.
    ///
    /// The inverse of [`subscribe_events`](Self::subscribe_events). Accepts
    /// multiple event types to unsubscribe from at once.
    pub async fn nixevent<T: Borrow<EslEventType>>(
        &self,
        events: impl IntoIterator<Item = T>,
    ) -> EslResult<()> {
        self.nixevent_raw(&event_types_to_string(events))
            .await
    }

    /// Unsubscribe from events using raw event name strings.
    ///
    /// Prefer [`nixevent`](Self::nixevent) when all event types have
    /// [`EslEventType`] variants. Use this only for `CUSTOM` subclasses or
    /// event types not yet covered by the typed enum.
    pub async fn nixevent_raw(&self, events: &str) -> EslResult<()> {
        let cmd = EslCommand::NixEvent {
            events: events.to_string(),
        };
        self.send_command_ok(cmd)
            .await
    }

    /// Unsubscribe from all events.
    ///
    /// Clears all event subscriptions. The server flushes any queued events.
    pub async fn noevents(&self) -> EslResult<()> {
        self.send_command_ok(EslCommand::NoEvents)
            .await
    }

    /// Remove an event filter for a typed [`EventHeader`].
    ///
    /// Without a value, removes all filters for the given header.
    /// With a value, removes only the filter matching that header+value pair.
    pub async fn filter_delete(&self, header: EventHeader, value: Option<&str>) -> EslResult<()> {
        self.filter_delete_raw(header.as_str(), value)
            .await
    }

    /// Remove an event filter using a raw header name string.
    ///
    /// Prefer [`filter_delete`](Self::filter_delete) when the header has an
    /// [`EventHeader`] variant. Use this only for headers not yet covered by
    /// the typed enum.
    ///
    /// Without a value, removes all filters for the given header.
    /// With a value, removes only the filter matching that header+value pair.
    pub async fn filter_delete_raw(&self, header: &str, value: Option<&str>) -> EslResult<()> {
        let cmd = EslCommand::FilterDelete {
            header: header.to_string(),
            value: value.map(|v| v.to_string()),
        };
        self.send_command_ok(cmd)
            .await
    }

    /// Remove all event filters.
    pub async fn filter_delete_all(&self) -> EslResult<()> {
        self.filter_delete_raw("all", None)
            .await
    }

    /// Redirect session events to the ESL connection (outbound mode).
    ///
    /// When `on` is true, events that would normally be processed internally
    /// by FreeSWITCH are instead sent to the ESL connection.
    pub async fn divert_events(&self, on: bool) -> EslResult<()> {
        self.send_command_ok(EslCommand::DivertEvents { on })
            .await
    }

    /// Read a channel variable (outbound mode).
    ///
    /// **Protocol quirk:** Unlike every other ESL command, `getvar` returns
    /// the raw variable value directly in `Reply-Text` with no `+OK`/`-ERR`
    /// prefix. A non-existent variable returns an empty string (never `-ERR`).
    /// This method reads the raw Reply-Text; do not use `into_result()` on
    /// the response -- it would misclassify the bare value as
    /// [`UnexpectedReply`](crate::EslError::UnexpectedReply).
    pub async fn getvar(&self, name: &str) -> EslResult<String> {
        let cmd = EslCommand::GetVar {
            name: name.to_string(),
        };
        let response = self
            .send_command(cmd)
            .await?;
        response
            .reply_text()
            .map(|s| s.to_string())
            .ok_or_else(|| EslError::protocol_error("getvar response missing Reply-Text header"))
    }

    /// Read a channel variable, distinguishing "unset" from a present value.
    ///
    /// FreeSWITCH does not signal a missing variable with `-ERR`; some
    /// versions reply with the literal string `_undef_`, others reply with
    /// an empty `Reply-Text`. This method normalizes both to `Ok(None)`
    /// so callers don't have to special-case either sentinel.
    pub async fn getvar_opt(&self, name: &str) -> EslResult<Option<String>> {
        let v = self
            .getvar(name)
            .await?;
        Ok(if v.is_empty() || v == "_undef_" {
            None
        } else {
            Some(v)
        })
    }

    /// Enable FreeSWITCH log forwarding at the given level.
    ///
    /// Log messages stream as events with `Content-Type: log/data`.
    /// Valid levels: `DEBUG`, `INFO`, `NOTICE`, `WARNING`, `ERROR`,
    /// `CRIT`, `ALERT`, `EMERG` (or numeric 0–7).
    pub async fn log(&self, level: &str) -> EslResult<EslResponse> {
        let cmd = EslCommand::Log {
            level: level.to_string(),
        };
        self.send_command(cmd)
            .await
    }

    /// Disable log forwarding.
    pub async fn nolog(&self) -> EslResult<EslResponse> {
        self.send_command(EslCommand::NoLog)
            .await
    }

    /// Send a no-op command (keepalive).
    pub async fn noop(&self) -> EslResult<EslResponse> {
        self.send_command(EslCommand::NoOp)
            .await
    }

    /// Send the `exit` command to gracefully close the ESL session.
    ///
    /// Unlike [`disconnect()`](Self::disconnect) which shuts down the TCP
    /// write half immediately, this sends the ESL `exit` command and waits
    /// for the server's reply before the connection closes.
    pub async fn exit(&self) -> EslResult<EslResponse> {
        self.send_command(EslCommand::Exit)
            .await
    }

    /// Authentication response from the inbound connect handshake.
    ///
    /// For `userauth`, contains `Allowed-Events`, `Allowed-API`, and
    /// `Allowed-LOG` headers describing the user's access policy.
    /// Returns `None` for outbound connections (no auth handshake).
    pub fn auth_response(&self) -> Option<&EslResponse> {
        self.shared
            .auth_response
            .as_ref()
    }

    /// Number of events dropped due to a full event queue.
    pub fn dropped_event_count(&self) -> u64 {
        self.shared
            .dropped_event_count
            .load(Ordering::Relaxed)
    }

    /// Set liveness timeout. Any inbound TCP traffic resets the timer.
    /// Set to zero to disable (default).
    ///
    /// The library never sends keepalives on its own -- the timer is fed only
    /// by traffic the server pushes. On a busy connection, ordinary event
    /// traffic keeps it alive. On an **idle** connection you must arrange the
    /// traffic yourself, typically by subscribing to [`EslEventType::Heartbeat`]
    /// (`subscribe_events(.., &[EslEventType::Heartbeat])`); FreeSWITCH then
    /// emits a `HEARTBEAT` every ~20s.
    ///
    /// If that subscription is **denied** -- a permission-restricted user
    /// (`esl-allowed-events` without `HEARTBEAT`) gets `-ERR permission denied`,
    /// detectable via [`EslError::is_permission_denied`] -- the subscribe call
    /// returns a recoverable error and the connection stays usable, but no
    /// heartbeats arrive. Do not enable this timeout for such a connection, or
    /// it will trip on idle while the socket is perfectly healthy. See
    /// `examples/reconnecting_client.rs` for the gated pattern.
    ///
    /// [`EslError::is_permission_denied`]: crate::EslError::is_permission_denied
    pub fn set_liveness_timeout(&self, duration: Duration) {
        self.shared
            .liveness_timeout_ms
            .store(duration.as_millis() as u64, Ordering::Relaxed);
    }

    /// Set command response timeout (default: 5 seconds).
    ///
    /// Applies to `send_command()`, `api()`, `bgapi()`, `subscribe_events()`,
    /// and all other methods that send a command and await a reply.
    ///
    /// If increased for long-running `api()` calls, also increase or disable
    /// the liveness timeout -- `api` blocks the socket, starving the liveness timer.
    pub fn set_command_timeout(&self, duration: Duration) {
        self.shared
            .command_timeout_ms
            .store(duration.as_millis() as u64, Ordering::Relaxed);
    }

    /// Whether the connection is alive (not yet disconnected).
    pub fn is_connected(&self) -> bool {
        matches!(
            *self
                .status_rx
                .borrow(),
            ConnectionStatus::Connected
        )
    }

    /// Current connection status snapshot.
    pub fn status(&self) -> ConnectionStatus {
        self.status_rx
            .borrow()
            .clone()
    }

    /// Disconnect from FreeSWITCH by shutting down the write half.
    ///
    /// Sets the connection status to [`DisconnectReason::ClientRequested`]
    /// before closing the socket, so callers can distinguish client-initiated
    /// disconnects from server-initiated ones.
    pub async fn disconnect(&self) -> EslResult<()> {
        info!("Client requested disconnect");
        let _ = self
            .shared
            .status_tx
            .send(ConnectionStatus::Disconnected(
                DisconnectReason::ClientRequested,
            ));
        let mut writer = self
            .writer
            .lock()
            .await;
        writer
            .shutdown()
            .await
            .map_err(EslError::Io)?;
        Ok(())
    }
}

#[cfg(unix)]
impl EslClient {
    /// Gracefully stop the reader loop and return the raw socket fd and any
    /// unparsed bytes remaining in the parser buffer.
    ///
    /// Used for zero-downtime binary upgrades via `exec()`. The caller
    /// serializes application state, clears `CLOEXEC` on the returned fd,
    /// and calls `exec()`. The new process reconstructs the connection with
    /// [`adopt_stream()`](Self::adopt_stream).
    ///
    /// # Preconditions
    ///
    /// - No ESL command may be in-flight (pending reply). Returns
    ///   [`EslError::ReexecFailed`] if a command is pending.
    /// - May only be called once. Returns an error on subsequent calls.
    ///
    /// # After this call
    ///
    /// - The connection status is set to
    ///   [`DisconnectReason::ReexecTeardown`], so all clones see the
    ///   connection as dead.
    /// - The caller **must not drop** the `EslClient` before `exec()` (or
    ///   must [`std::mem::forget`] it) to keep the fd open.
    /// - The caller is responsible for clearing `CLOEXEC` on the fd.
    pub async fn teardown_for_reexec(&self) -> EslResult<(std::os::unix::io::RawFd, Vec<u8>)> {
        use std::os::unix::io::AsRawFd;

        // Reject if a command is in-flight
        {
            let pending = self
                .shared
                .pending_reply
                .lock()
                .await;
            if pending
                .waiting
                .is_some()
            {
                return Err(EslError::ReexecFailed {
                    reason: "command still in-flight".into(),
                });
            }
        }

        // Take the reexec channel (one-shot: fails on second call)
        let reexec = {
            let mut guard = self
                .shared
                .reexec
                .lock()
                .await;
            guard
                .take()
                .ok_or_else(|| EslError::ReexecFailed {
                    reason: "teardown already called".into(),
                })?
        };

        // Disable liveness to prevent HeartbeatExpired race
        self.shared
            .liveness_timeout_ms
            .store(0, Ordering::Relaxed);

        let _ = reexec
            .stop_tx
            .send(());

        // Wait for reader to drain and return residual bytes.
        // Extra 1s margin so the reader's own drain timeout fires first
        // with a descriptive error.
        let outer_timeout = Duration::from_millis(REEXEC_DRAIN_TIMEOUT_MS) + Duration::from_secs(1);
        let residual = timeout(outer_timeout, reexec.result_rx)
            .await
            .map_err(|e| EslError::ReexecFailed {
                reason: format!("timed out waiting for reader to stop: {e}"),
            })?
            .map_err(|e| EslError::ReexecFailed {
                reason: format!("reader task exited without sending result: {e}"),
            })??;

        // Get fd from writer
        let writer = self
            .writer
            .lock()
            .await;
        let fd = writer
            .as_ref()
            .as_raw_fd();

        // Mark connection as dead so other clones see it
        let _ = self
            .shared
            .status_tx
            .send(ConnectionStatus::Disconnected(
                DisconnectReason::ReexecTeardown,
            ));

        info!(
            "Re-exec teardown complete, fd={}, {} residual bytes",
            fd,
            residual.len()
        );
        Ok((fd, residual))
    }
}

impl EslEventStream {
    /// Receive the next event, or None if the channel is closed.
    ///
    /// Returns `Err(EslError::QueueFull)` if events were dropped because the
    /// application was not draining events fast enough. This is a one-time
    /// notification per overflow episode -- subsequent calls return real events.
    /// Parse errors from the reader task are also surfaced here.
    pub async fn recv(&mut self) -> Option<Result<EslEvent, EslError>> {
        self.rx
            .recv()
            .await
    }

    /// Whether the connection is alive (not yet disconnected).
    pub fn is_connected(&self) -> bool {
        matches!(
            *self
                .status_rx
                .borrow(),
            ConnectionStatus::Connected
        )
    }

    /// Current connection status snapshot.
    pub fn status(&self) -> ConnectionStatus {
        self.status_rx
            .borrow()
            .clone()
    }
}

impl futures_util::Stream for EslEventStream {
    type Item = Result<EslEvent, EslError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx
            .poll_recv(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connection_mode() {
        assert_eq!(ConnectionMode::Inbound, ConnectionMode::Inbound);
        assert_ne!(ConnectionMode::Inbound, ConnectionMode::Outbound);
    }

    #[tokio::test]
    async fn test_connection_status_eq() {
        assert_eq!(ConnectionStatus::Connected, ConnectionStatus::Connected);
        assert_eq!(
            ConnectionStatus::Disconnected(DisconnectReason::ServerNotice {
                controlled_session_uuid: None,
                body: None,
            }),
            ConnectionStatus::Disconnected(DisconnectReason::ServerNotice {
                controlled_session_uuid: None,
                body: None,
            })
        );
        assert_ne!(
            ConnectionStatus::Connected,
            ConnectionStatus::Disconnected(DisconnectReason::ConnectionClosed)
        );
    }

    #[test]
    fn salvage_truncated_auth_response_realistic() {
        // Simulates FreeSWITCH reply[512] overflow: long Allowed-Events
        // header causes Allowed-API to be truncated and \n\n never sent.
        let wire_data = "\
            Content-Type: command/reply\n\
            Reply-Text: +OK accepted\n\
            Allowed-Events: HEARTBEAT BACKGROUND_JOB CHANNEL_CREATE \
            CHANNEL_ANSWER CHANNEL_HANGUP_COMPLETE CHANNEL_STATE \
            CHANNEL_DATA CHANNEL_CALLSTATE CHANNEL_EXECUTE \
            CHANNEL_EXECUTE_COMPLETE CHANNEL_BRIDGE NOTIFY_IN \
            CHANNEL_DESTROY CHANNEL_HANGUP CHANNEL_HOLD CHANNEL_UNHOLD \
            CHANNEL_UNBRIDGE CHANNEL_PROGRESS CHANNEL_PROGRESS_MEDIA \
            CHANNEL_OUTGOING CHANNEL_PARK CHANNEL_UNPARK \
            CHANNEL_APPLICATION CHANNEL_ORIGINATE CHANNEL_UUID CUSTOM \
            sofia::gateway_state sofia::gateway_delete\n\
            Allowed-API: show";

        let mut parser = EslParser::new();
        parser
            .add_data(wire_data.as_bytes())
            .unwrap();

        // Normal parse should return None — no \n\n terminator
        assert!(parser
            .parse_message()
            .unwrap()
            .is_none());

        let msg = salvage_truncated_auth_response(&mut parser)
            .unwrap()
            .expect("salvage should succeed");

        assert_eq!(msg.message_type, MessageType::CommandReply);
        assert_eq!(
            msg.headers
                .get("Reply-Text")
                .map(|s| s.as_str()),
            Some("+OK accepted")
        );
        assert!(msg
            .headers
            .get("Allowed-Events")
            .is_some());
        // Truncated value is preserved (normalize_header_key title-cases "API" → "Api")
        assert_eq!(
            msg.headers
                .get("Allowed-Api")
                .map(|s| s.as_str()),
            Some("show")
        );

        // Parser buffer is drained
        assert_eq!(
            parser
                .remaining_bytes()
                .len(),
            0
        );
    }

    #[test]
    fn salvage_truncated_mid_header_name() {
        // Truncation cut mid-header-name (no colon in trailing fragment)
        let wire_data = "Content-Type: command/reply\nReply-Text: +OK accepted\nAllo";

        let mut parser = EslParser::new();
        parser
            .add_data(wire_data.as_bytes())
            .unwrap();

        let msg = salvage_truncated_auth_response(&mut parser)
            .unwrap()
            .expect("salvage should succeed with partial line dropped");

        assert_eq!(msg.message_type, MessageType::CommandReply);
        assert_eq!(
            msg.headers
                .get("Reply-Text")
                .map(|s| s.as_str()),
            Some("+OK accepted")
        );
        // "Allo" fragment was dropped
        assert!(msg
            .headers
            .get("Allo")
            .is_none());
    }

    #[test]
    fn salvage_empty_buffer_returns_none() {
        let mut parser = EslParser::new();
        assert!(salvage_truncated_auth_response(&mut parser)
            .unwrap()
            .is_none());
    }

    #[test]
    fn salvage_no_newline_returns_none() {
        let mut parser = EslParser::new();
        parser
            .add_data(b"Content-Type: command/reply")
            .unwrap();
        assert!(salvage_truncated_auth_response(&mut parser)
            .unwrap()
            .is_none());
    }

    #[test]
    fn salvage_wrong_content_type_returns_error() {
        let wire_data = "Content-Type: auth/request\n";
        let mut parser = EslParser::new();
        parser
            .add_data(wire_data.as_bytes())
            .unwrap();

        let result = salvage_truncated_auth_response(&mut parser);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("command/reply"),);
    }
}
