//! Connection management for ESL

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
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

/// Connection status for ESL client
#[derive(Debug, Clone)]
pub enum ConnectionStatus {
    Connected,
    Disconnected(DisconnectReason),
}

/// Reason for disconnection
#[derive(Debug, Clone)]
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

/// Event stream receiver (!Clone)
///
/// Receives events from the background reader task via an mpsc channel.
pub struct EslEventStream {
    rx: mpsc::Receiver<EslEvent>,
    status_rx: watch::Receiver<ConnectionStatus>,
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

/// Perform password authentication on the stream.
async fn authenticate(
    stream: &mut TcpStream,
    parser: &mut EslParser,
    read_buffer: &mut [u8],
    password: &str,
) -> EslResult<()> {
    debug!("[AUTH] Waiting for auth request from FreeSWITCH");
    let message = recv_message(stream, parser, read_buffer).await?;

    if message.message_type != MessageType::AuthRequest {
        return Err(EslError::protocol_error("Expected auth request"));
    }

    let auth_cmd = EslCommand::Auth {
        password: password.to_string(),
    };
    let command_str = auth_cmd.to_wire_format();
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
                .cloned()
                .unwrap_or_else(|| "Authentication failed".to_string()),
        ));
    }

    debug!("Authentication successful");
    Ok(())
}

/// Perform user authentication on the stream.
async fn authenticate_user(
    stream: &mut TcpStream,
    parser: &mut EslParser,
    read_buffer: &mut [u8],
    user: &str,
    password: &str,
) -> EslResult<()> {
    debug!("Starting user authentication for user: {}", user);

    let message = recv_message(stream, parser, read_buffer).await?;
    if message.message_type != MessageType::AuthRequest {
        return Err(EslError::protocol_error("Expected auth request"));
    }

    let auth_cmd = EslCommand::UserAuth {
        user: user.to_string(),
        password: password.to_string(),
    };
    let command_str = auth_cmd.to_wire_format();
    debug!("Sending command: userauth {}:[REDACTED]", user);
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
                .cloned()
                .unwrap_or_else(|| "User authentication failed".to_string()),
        ));
    }

    debug!("User authentication successful");
    Ok(())
}

/// Background reader loop
async fn reader_loop(
    mut reader: OwnedReadHalf,
    mut parser: EslParser,
    shared: Arc<SharedState>,
    status_tx: watch::Sender<ConnectionStatus>,
    event_tx: mpsc::Sender<EslEvent>,
) {
    let mut read_buffer = [0u8; SOCKET_BUF_SIZE];
    let mut last_recv = Instant::now();

    loop {
        // Try to parse a complete message from buffered data first
        match parser.parse_message() {
            Ok(Some(message)) => {
                match message.message_type {
                    MessageType::Event => {
                        // Determine format from Content-Type
                        let format = message
                            .headers
                            .get(HEADER_CONTENT_TYPE)
                            .map(|ct| match ct.as_str() {
                                CONTENT_TYPE_TEXT_EVENT_JSON => EventFormat::Json,
                                CONTENT_TYPE_TEXT_EVENT_XML => EventFormat::Xml,
                                _ => EventFormat::Plain,
                            })
                            .unwrap_or(EventFormat::Plain);

                        let event_result = parser.parse_event(message, format);
                        match event_result {
                            Ok(event) => {
                                if event_tx
                                    .send(event)
                                    .await
                                    .is_err()
                                {
                                    debug!("Event channel closed, reader exiting");
                                    return;
                                }
                            }
                            Err(e) => {
                                warn!("Failed to parse event: {}", e);
                            }
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
                        // Check Content-Disposition: if "linger", don't disconnect
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
        info!("Connecting to FreeSWITCH at {}:{}", host, port);

        let tcp_result = timeout(
            Duration::from_millis(DEFAULT_TIMEOUT_MS),
            TcpStream::connect((host, port)),
        )
        .await;

        let mut stream = match tcp_result {
            Ok(Ok(s)) => {
                debug!("[CONNECT] TCP connection established");
                s
            }
            Ok(Err(e)) => {
                warn!("[CONNECT] TCP connect failed: {}", e);
                return Err(EslError::Io(e));
            }
            Err(_) => {
                warn!(
                    "[CONNECT] TCP connect timed out after {}ms",
                    DEFAULT_TIMEOUT_MS
                );
                return Err(EslError::Timeout {
                    timeout_ms: DEFAULT_TIMEOUT_MS,
                });
            }
        };

        let mut parser = EslParser::new();
        let mut read_buffer = [0u8; SOCKET_BUF_SIZE];

        authenticate(&mut stream, &mut parser, &mut read_buffer, password).await?;

        info!("Successfully connected and authenticated to FreeSWITCH");
        Ok(Self::split_and_spawn(stream, parser))
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
        if !user.contains('@') {
            return Err(EslError::auth_failed(format!(
                "Invalid username format '{}': must be user@domain (e.g., admin@default)",
                user
            )));
        }

        info!(
            "Connecting to FreeSWITCH at {}:{} with user {}",
            host, port, user
        );

        let mut stream = TcpStream::connect((host, port))
            .await
            .map_err(EslError::Io)?;

        let mut parser = EslParser::new();
        let mut read_buffer = [0u8; SOCKET_BUF_SIZE];

        authenticate_user(&mut stream, &mut parser, &mut read_buffer, user, password).await?;

        info!("Successfully connected and authenticated to FreeSWITCH");
        Ok(Self::split_and_spawn(stream, parser))
    }

    /// Accept outbound connection from FreeSWITCH
    pub async fn accept_outbound(listener: &TcpListener) -> EslResult<(Self, EslEventStream)> {
        info!("Waiting for outbound connection from FreeSWITCH");

        let (stream, addr) = listener
            .accept()
            .await
            .map_err(EslError::Io)?;
        info!("Accepted outbound connection from {}", addr);

        Ok(Self::split_and_spawn(stream, EslParser::new()))
    }

    /// Split a TcpStream and spawn the reader task
    fn split_and_spawn(stream: TcpStream, parser: EslParser) -> (Self, EslEventStream) {
        let (read_half, write_half) = stream.into_split();

        let shared = Arc::new(SharedState {
            pending_reply: Mutex::new(None),
            liveness_timeout_ms: AtomicU64::new(0),
            command_timeout_ms: AtomicU64::new(DEFAULT_COMMAND_TIMEOUT_MS),
        });

        let (status_tx, status_rx) = watch::channel(ConnectionStatus::Connected);
        let status_rx2 = status_tx.subscribe();
        let (event_tx, event_rx) = mpsc::channel(MAX_EVENT_QUEUE_SIZE);

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

    /// Send a command and wait for the reply
    pub async fn send_command(&self, command: EslCommand) -> EslResult<EslResponse> {
        if !self.is_connected() {
            return Err(EslError::NotConnected);
        }

        let command_str = command.to_wire_format();
        match &command {
            EslCommand::Auth { .. } => debug!("Sending command: auth [REDACTED]"),
            EslCommand::UserAuth { user, .. } => {
                debug!("Sending command: userauth {}:[REDACTED]", user)
            }
            _ => debug!("Sending command: {}", command_str.trim()),
        }

        // Lock writer — serializes concurrent commands (ESL is sequential)
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

        // Drop writer lock so other operations can proceed
        drop(writer);

        // Wait for reply from reader task with command timeout
        let timeout_ms = self
            .shared
            .command_timeout_ms
            .load(Ordering::Relaxed);
        let message = timeout(Duration::from_millis(timeout_ms), rx)
            .await
            .map_err(|_| {
                // Timeout — clean up the pending reply slot so the reader
                // doesn't later try to send to a closed oneshot
                let shared = self
                    .shared
                    .clone();
                tokio::spawn(async move {
                    let mut pending = shared
                        .pending_reply
                        .lock()
                        .await;
                    pending.take();
                });
                EslError::Timeout { timeout_ms }
            })?
            .map_err(|_| EslError::ConnectionClosed)?;
        let response = message.into_response();

        debug!("Received response: success={}", response.is_success());
        Ok(response)
    }

    /// Execute API command
    pub async fn api(&self, command: &str) -> EslResult<EslResponse> {
        let cmd = EslCommand::Api {
            command: command.to_string(),
        };
        self.send_command(cmd)
            .await
    }

    /// Execute background API command
    pub async fn bgapi(&self, command: &str) -> EslResult<EslResponse> {
        let cmd = EslCommand::BgApi {
            command: command.to_string(),
        };
        self.send_command(cmd)
            .await
    }

    /// Subscribe to events
    pub async fn subscribe_events(
        &self,
        format: EventFormat,
        events: &[EslEventType],
    ) -> EslResult<()> {
        let events_str = if events.contains(&EslEventType::All) {
            "ALL".to_string()
        } else {
            events
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        };

        let cmd = EslCommand::Events {
            format: format.to_string(),
            events: events_str,
        };

        let response = self
            .send_command(cmd)
            .await?;
        if !response.is_success() {
            return Err(EslError::CommandFailed {
                reply_text: response
                    .reply_text()
                    .cloned()
                    .unwrap_or_else(|| "Event subscription failed".to_string()),
            });
        }

        info!("Subscribed to events with format {:?}", format);
        Ok(())
    }

    /// Subscribe to events using raw event name strings.
    ///
    /// Use this for event types not covered by `EslEventType`, or for
    /// forward compatibility with new FreeSWITCH events without a library update.
    ///
    /// ```rust,no_run
    /// # async fn example(client: &freeswitch_esl_rs::EslClient) -> Result<(), freeswitch_esl_rs::EslError> {
    /// use freeswitch_esl_rs::EventFormat;
    /// client.subscribe_events_raw(EventFormat::Plain, "NOTIFY_IN CHANNEL_CREATE").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe_events_raw(&self, format: EventFormat, events: &str) -> EslResult<()> {
        let cmd = EslCommand::Events {
            format: format.to_string(),
            events: events.to_string(),
        };

        let response = self
            .send_command(cmd)
            .await?;
        if !response.is_success() {
            return Err(EslError::CommandFailed {
                reply_text: response
                    .reply_text()
                    .cloned()
                    .unwrap_or_else(|| "Event subscription failed".to_string()),
            });
        }

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

        let response = self
            .send_command(cmd)
            .await?;
        response.into_result()?;

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

    /// Check if the connection is alive
    pub fn is_connected(&self) -> bool {
        matches!(
            *self
                .status_rx
                .borrow(),
            ConnectionStatus::Connected
        )
    }

    /// Get current connection status
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
    /// Receive the next event, or None if the connection is closed
    pub async fn recv(&mut self) -> Option<EslEvent> {
        self.rx
            .recv()
            .await
    }

    /// Check if the connection is alive
    pub fn is_connected(&self) -> bool {
        matches!(
            *self
                .status_rx
                .borrow(),
            ConnectionStatus::Connected
        )
    }

    /// Get current connection status
    pub fn status(&self) -> ConnectionStatus {
        self.status_rx
            .borrow()
            .clone()
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
}
