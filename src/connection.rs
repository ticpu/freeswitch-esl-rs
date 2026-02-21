//! Connection management for ESL

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

use crate::{
    command::{EslCommand, EslResponse},
    constants::*,
    error::{EslError, EslResult},
    event::{EslEvent, EslEventType, EventFormat},
    protocol::{EslMessage, EslParser, MessageType},
};

fn event_types_to_string(events: &[EslEventType]) -> String {
    events
        .iter()
        .map(|e| e.to_string())
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
    /// Server sent a text/disconnect-notice with Content-Disposition: disconnect
    ServerNotice,
    /// Liveness timeout exceeded without any inbound traffic
    HeartbeatExpired,
    /// TCP I/O error (io::Error is not Clone, so we store the message)
    IoError(String),
    /// Clean EOF on the TCP connection
    ConnectionClosed,
    /// Client called disconnect()
    ClientRequested,
}

impl std::fmt::Display for DisconnectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DisconnectReason::ServerNotice => write!(f, "server sent disconnect notice"),
            DisconnectReason::HeartbeatExpired => write!(f, "liveness timeout expired"),
            DisconnectReason::IoError(msg) => write!(f, "I/O error: {}", msg),
            DisconnectReason::ConnectionClosed => write!(f, "connection closed"),
            DisconnectReason::ClientRequested => write!(f, "client requested disconnect"),
        }
    }
}

/// Establish a TCP connection with a timeout.
async fn tcp_connect_with_timeout(host: &str, port: u16) -> EslResult<TcpStream> {
    let tcp_result = timeout(
        Duration::from_millis(DEFAULT_TIMEOUT_MS),
        TcpStream::connect((host, port)),
    )
    .await;

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
            warn!(
                "[CONNECT] TCP connect timed out after {}ms",
                DEFAULT_TIMEOUT_MS
            );
            Err(EslError::Timeout {
                timeout_ms: DEFAULT_TIMEOUT_MS,
            })
        }
    }
}

/// Connection mode for ESL
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionMode {
    /// Inbound connection - client connects to FreeSWITCH
    Inbound,
    /// Outbound connection - FreeSWITCH connects to client
    Outbound,
}

/// Default command timeout in milliseconds (5 seconds)
const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 5000;

/// Shared state between EslClient and the reader task
struct SharedState {
    pending_reply: Mutex<Option<oneshot::Sender<EslMessage>>>,
    /// Liveness timeout in milliseconds (0 = disabled)
    liveness_timeout_ms: AtomicU64,
    /// Command response timeout in milliseconds
    command_timeout_ms: AtomicU64,
    /// Set when events have been dropped due to a full queue
    event_overflow: AtomicBool,
    /// Total count of dropped events
    dropped_event_count: AtomicU64,
}

/// Options for ESL connection configuration.
///
/// Controls parameters that are fixed at connection time, such as the event
/// queue capacity. Use [`Default::default()`] for standard settings.
#[derive(Debug, Clone)]
pub struct EslConnectOptions {
    /// Capacity of the mpsc channel delivering events. Default: 1000.
    pub event_queue_size: usize,
}

impl Default for EslConnectOptions {
    fn default() -> Self {
        Self {
            event_queue_size: MAX_EVENT_QUEUE_SIZE,
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
) -> EslResult<EslMessage> {
    loop {
        if let Some(message) = parser.parse_message()? {
            trace!(
                "[RECV] Parsed message from buffer: {:?}",
                message.message_type
            );
            return Ok(message);
        }

        trace!("[RECV] Buffer needs more data, reading from socket");
        let read_result = timeout(
            Duration::from_millis(DEFAULT_TIMEOUT_MS),
            stream.read(read_buffer),
        )
        .await;

        let bytes_read = match read_result {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(EslError::Io(e)),
            Err(_) => {
                return Err(EslError::Timeout {
                    timeout_ms: DEFAULT_TIMEOUT_MS,
                })
            }
        };

        trace!("[RECV] Read {} bytes from socket", bytes_read);
        if bytes_read == 0 {
            return Err(EslError::ConnectionClosed);
        }

        parser.add_data(&read_buffer[..bytes_read])?;
    }
}

/// Perform authentication on the stream.
async fn authenticate(
    stream: &mut TcpStream,
    parser: &mut EslParser,
    read_buffer: &mut [u8],
    method: AuthMethod<'_>,
) -> EslResult<()> {
    debug!("[AUTH] Waiting for auth request from FreeSWITCH");
    let message = recv_message(stream, parser, read_buffer).await?;

    if message.message_type != MessageType::AuthRequest {
        return Err(EslError::protocol_error("Expected auth request"));
    }

    let auth_cmd = match method {
        AuthMethod::Password(password) => EslCommand::Auth {
            password: password.to_string(),
        },
        AuthMethod::User { user, password } => EslCommand::UserAuth {
            user: user.to_string(),
            password: password.to_string(),
        },
    };

    let command_str = auth_cmd.to_wire_format()?;
    debug!("Sending command: auth [REDACTED]");
    stream
        .write_all(command_str.as_bytes())
        .await
        .map_err(EslError::Io)?;

    let response_msg = recv_message(stream, parser, read_buffer).await?;
    let response = response_msg.into_response();

    if !response.is_success() {
        return Err(EslError::auth_failed(
            response
                .reply_text()
                .unwrap_or("Authentication failed")
                .to_string(),
        ));
    }

    debug!("Authentication successful");
    Ok(())
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
    status_tx: watch::Sender<ConnectionStatus>,
    event_tx: mpsc::Sender<Result<EslEvent, EslError>>,
) {
    let result = std::panic::AssertUnwindSafe(reader_loop_inner(
        reader,
        parser,
        shared,
        status_tx.clone(),
        event_tx,
    ));
    if futures_util::FutureExt::catch_unwind(result)
        .await
        .is_err()
    {
        tracing::error!("reader task panicked");
        let _ = status_tx.send(ConnectionStatus::Disconnected(DisconnectReason::IoError(
            "reader task panicked".to_string(),
        )));
    }
}

async fn reader_loop_inner(
    mut reader: OwnedReadHalf,
    mut parser: EslParser,
    shared: Arc<SharedState>,
    status_tx: watch::Sender<ConnectionStatus>,
    event_tx: mpsc::Sender<Result<EslEvent, EslError>>,
) {
    let mut read_buffer = [0u8; SOCKET_BUF_SIZE];
    let mut last_recv = Instant::now();

    loop {
        // Try to parse a complete message from buffered data first
        match parser.parse_message() {
            Ok(Some(message)) => {
                match message.message_type {
                    MessageType::Event => {
                        let format = message
                            .headers
                            .get(HEADER_CONTENT_TYPE)
                            .map(|ct| EventFormat::from_content_type(ct))
                            .unwrap_or(EventFormat::Plain);

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
                        if let Some(tx) = pending.take() {
                            let _ = tx.send(message);
                        } else {
                            warn!(
                                "Received {:?} but no pending command",
                                "CommandReply/ApiResponse"
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
                        info!("Received disconnect notice from server");
                        let _ = status_tx.send(ConnectionStatus::Disconnected(
                            DisconnectReason::ServerNotice,
                        ));
                        return;
                    }
                    MessageType::AuthRequest | MessageType::Unknown(_) => {
                        debug!("Ignoring unexpected message: {:?}", message.message_type);
                    }
                }
                continue;
            }
            Ok(None) => {
                // Need more data from socket
            }
            Err(e) => {
                warn!("Parser error: {}", e);
                let _ = status_tx.send(ConnectionStatus::Disconnected(DisconnectReason::IoError(
                    e.to_string(),
                )));
                return;
            }
        }

        // Read from socket with 2s timeout (for liveness checking)
        let read_result = timeout(Duration::from_secs(2), reader.read(&mut read_buffer)).await;

        match read_result {
            Ok(Ok(0)) => {
                info!("Connection closed (EOF)");
                let _ = status_tx.send(ConnectionStatus::Disconnected(
                    DisconnectReason::ConnectionClosed,
                ));
                return;
            }
            Ok(Ok(n)) => {
                last_recv = Instant::now();
                if let Err(e) = parser.add_data(&read_buffer[..n]) {
                    warn!("Buffer error: {}", e);
                    let _ = status_tx.send(ConnectionStatus::Disconnected(
                        DisconnectReason::IoError(e.to_string()),
                    ));
                    return;
                }
            }
            Ok(Err(e)) => {
                warn!("Read error: {}", e);
                let _ = status_tx.send(ConnectionStatus::Disconnected(DisconnectReason::IoError(
                    e.to_string(),
                )));
                return;
            }
            Err(_) => {
                // Timeout — check liveness
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
                        let _ = status_tx.send(ConnectionStatus::Disconnected(
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

        let mut stream = tcp_connect_with_timeout(host, port).await?;
        let mut parser = EslParser::new();
        let mut read_buffer = [0u8; SOCKET_BUF_SIZE];

        authenticate(&mut stream, &mut parser, &mut read_buffer, method).await?;

        info!("Successfully connected and authenticated to FreeSWITCH");
        Ok(Self::split_and_spawn_with_options(stream, parser, options))
    }

    /// Accept outbound connection from FreeSWITCH
    pub async fn accept_outbound(listener: &TcpListener) -> EslResult<(Self, EslEventStream)> {
        Self::accept_outbound_with_options(listener, EslConnectOptions::default()).await
    }

    /// Accept outbound connection from FreeSWITCH with custom options
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

        Ok(Self::split_and_spawn_with_options(
            stream,
            EslParser::new(),
            options,
        ))
    }

    fn split_and_spawn_with_options(
        stream: TcpStream,
        parser: EslParser,
        options: EslConnectOptions,
    ) -> (Self, EslEventStream) {
        let queue_size = options
            .event_queue_size
            .max(1);

        let (read_half, write_half) = stream.into_split();

        let shared = Arc::new(SharedState {
            pending_reply: Mutex::new(None),
            liveness_timeout_ms: AtomicU64::new(0),
            command_timeout_ms: AtomicU64::new(DEFAULT_COMMAND_TIMEOUT_MS),
            event_overflow: AtomicBool::new(false),
            dropped_event_count: AtomicU64::new(0),
        });

        let (status_tx, status_rx) = watch::channel(ConnectionStatus::Connected);
        let status_rx2 = status_tx.subscribe();
        let (event_tx, event_rx) = mpsc::channel(queue_size);

        tokio::spawn(reader_loop(
            read_half,
            parser,
            shared.clone(),
            status_tx,
            event_tx,
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
        match &command {
            EslCommand::Auth { .. } => debug!("Sending command: auth [REDACTED]"),
            EslCommand::UserAuth { user, .. } => {
                debug!("Sending command: userauth {}:[REDACTED]", user)
            }
            _ => debug!("Sending command: {}", command_str.trim()),
        }

        // Lock writer — serializes concurrent commands and holds through reply.
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
            *pending = Some(tx);
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
            Ok(Err(_)) => {
                drop(writer);
                return Err(EslError::ConnectionClosed);
            }
            Err(_) => {
                let mut pending = self
                    .shared
                    .pending_reply
                    .lock()
                    .await;
                pending.take();
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

    /// Execute API command.
    ///
    /// ```rust,no_run
    /// # async fn example(client: &freeswitch_esl_tokio::EslClient) -> Result<(), freeswitch_esl_tokio::EslError> {
    /// let resp = client.api("status").await?;
    /// println!("{}", resp.body().unwrap_or(""));
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
    /// arrives later as a [`EslEventType::BackgroundJob`] event — subscribe to it
    /// and correlate via [`EslEvent::job_uuid`] / [`EslResponse::job_uuid`]:
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

    /// Subscribe to events by typed enum variants.
    ///
    /// For `CUSTOM` event subclasses (e.g., `sofia::register`), use
    /// [`subscribe_events_raw`](Self::subscribe_events_raw) instead — this method
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
    pub async fn subscribe_events(
        &self,
        format: EventFormat,
        events: &[EslEventType],
    ) -> EslResult<()> {
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
    /// Use this for event types not covered by `EslEventType`, or for
    /// forward compatibility with new FreeSWITCH events without a library update.
    ///
    /// ```rust,no_run
    /// # async fn example(client: &freeswitch_esl_tokio::EslClient) -> Result<(), freeswitch_esl_tokio::EslError> {
    /// use freeswitch_esl_tokio::EventFormat;
    /// client.subscribe_events_raw(EventFormat::Plain, "NOTIFY_IN CHANNEL_CREATE").await?;
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

    /// Set event filter
    pub async fn filter_events(&self, header: &str, value: &str) -> EslResult<()> {
        let cmd = EslCommand::Filter {
            header: header.to_string(),
            value: value.to_string(),
        };

        self.send_command_ok(cmd)
            .await?;
        debug!("Set event filter: {} = {}", header, value);
        Ok(())
    }

    /// Execute application on channel
    pub async fn execute(
        &self,
        app: &str,
        args: Option<&str>,
        uuid: Option<&str>,
    ) -> EslResult<EslResponse> {
        let cmd = EslCommand::Execute {
            app: app.to_string(),
            args: args.map(|s| s.to_string()),
            uuid: uuid.map(|s| s.to_string()),
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
    /// Pass `None` for indefinite linger, or `Some(seconds)` for a timeout.
    pub async fn linger(&self, timeout: Option<u32>) -> EslResult<()> {
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
    pub async fn nixevent(&self, events: &[EslEventType]) -> EslResult<()> {
        self.nixevent_raw(&event_types_to_string(events))
            .await
    }

    /// Unsubscribe from events using raw event name strings.
    ///
    /// Use this for event types not covered by `EslEventType`, or for
    /// forward compatibility with new FreeSWITCH events without a library update.
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

    /// Remove an event filter for a specific header.
    ///
    /// Without a value, removes all filters for the given header.
    /// With a value, removes only the filter matching that header+value pair.
    pub async fn filter_delete(&self, header: &str, value: Option<&str>) -> EslResult<()> {
        let cmd = EslCommand::FilterDelete {
            header: header.to_string(),
            value: value.map(|v| v.to_string()),
        };
        self.send_command_ok(cmd)
            .await
    }

    /// Remove all event filters.
    pub async fn filter_delete_all(&self) -> EslResult<()> {
        self.filter_delete("all", None)
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
    /// the response — it would misclassify the bare value as
    /// [`UnexpectedReply`](crate::EslError::UnexpectedReply).
    pub async fn getvar(&self, name: &str) -> EslResult<String> {
        let cmd = EslCommand::GetVar {
            name: name.to_string(),
        };
        let response = self
            .send_command(cmd)
            .await?;
        Ok(response
            .reply_text()
            .unwrap_or("")
            .to_string())
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

    /// Number of events dropped due to a full event queue.
    pub fn dropped_event_count(&self) -> u64 {
        self.shared
            .dropped_event_count
            .load(Ordering::Relaxed)
    }

    /// Set liveness timeout. Any inbound TCP traffic resets the timer.
    /// Set to zero to disable (default).
    pub fn set_liveness_timeout(&self, duration: Duration) {
        self.shared
            .liveness_timeout_ms
            .store(duration.as_millis() as u64, Ordering::Relaxed);
    }

    /// Set command response timeout (default: 5 seconds).
    ///
    /// Applies to `send_command()`, `api()`, `bgapi()`, `subscribe_events()`,
    /// and all other methods that send a command and await a reply.
    /// For long-running API calls (e.g., `originate`), increase this value.
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

    /// Disconnect from FreeSWITCH by shutting down the write half
    pub async fn disconnect(&self) -> EslResult<()> {
        info!("Client requested disconnect");
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

impl EslEventStream {
    /// Receive the next event, or None if the channel is closed.
    ///
    /// Returns `Err(EslError::QueueFull)` if events were dropped because the
    /// application was not draining events fast enough. This is a one-time
    /// notification per overflow episode — subsequent calls return real events.
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
            ConnectionStatus::Disconnected(DisconnectReason::ServerNotice),
            ConnectionStatus::Disconnected(DisconnectReason::ServerNotice)
        );
        assert_ne!(
            ConnectionStatus::Connected,
            ConnectionStatus::Disconnected(DisconnectReason::ConnectionClosed)
        );
    }
}
