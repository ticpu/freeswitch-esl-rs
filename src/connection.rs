//! Connection management for ESL

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::{debug, info, trace, warn};

use crate::{
    command::{EslCommand, EslResponse},
    constants::*,
    error::{EslError, EslResult},
    event::{EslEvent, EslEventType, EventFormat},
    protocol::{EslParser, MessageType},
};

/// Connection mode for ESL
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionMode {
    /// Inbound connection - client connects to FreeSWITCH
    Inbound,
    /// Outbound connection - FreeSWITCH connects to client
    Outbound,
}

/// ESL connection handle
pub struct EslHandle {
    /// TCP stream
    stream: TcpStream,
    /// Protocol parser
    parser: EslParser,
    /// Connection mode
    mode: ConnectionMode,
    /// Connection state
    connected: bool,
    /// Authentication state
    authenticated: bool,
    /// Event queue for received events
    event_queue: Arc<Mutex<VecDeque<EslEvent>>>,
    /// Socket read buffer
    read_buffer: [u8; SOCKET_BUF_SIZE],
    /// Current event format
    event_format: EventFormat,
}

impl EslHandle {
    /// Connect to FreeSWITCH (inbound mode)
    pub async fn connect(host: &str, port: u16, password: &str) -> EslResult<Self> {
        info!("Connecting to FreeSWITCH at {}:{}", host, port);

        debug!(
            "[CONNECT] Starting TCP connect with {}ms timeout",
            DEFAULT_TIMEOUT_MS
        );
        let tcp_result = timeout(
            Duration::from_millis(DEFAULT_TIMEOUT_MS),
            TcpStream::connect((host, port)),
        )
        .await;

        let stream = match tcp_result {
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

        debug!("[CONNECT] Creating ESL handle");
        let mut handle = Self {
            stream,
            parser: EslParser::new(),
            mode: ConnectionMode::Inbound,
            connected: true,
            authenticated: false,
            event_queue: Arc::new(Mutex::new(VecDeque::new())),
            read_buffer: [0u8; SOCKET_BUF_SIZE],
            event_format: EventFormat::Plain,
        };

        debug!("[CONNECT] Starting authentication");
        handle.authenticate(password).await?;

        info!("Successfully connected and authenticated to FreeSWITCH");
        Ok(handle)
    }

    /// Connect with user authentication
    ///
    /// The user must be in the format `user@domain` (e.g., `admin@default`).
    /// FreeSWITCH requires the domain to look up the user in the directory.
    pub async fn connect_with_user(
        host: &str,
        port: u16,
        user: &str,
        password: &str,
    ) -> EslResult<Self> {
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

        let stream = TcpStream::connect((host, port))
            .await
            .map_err(EslError::Io)?;

        let mut handle = Self {
            stream,
            parser: EslParser::new(),
            mode: ConnectionMode::Inbound,
            connected: true,
            authenticated: false,
            event_queue: Arc::new(Mutex::new(VecDeque::new())),
            read_buffer: [0u8; SOCKET_BUF_SIZE],
            event_format: EventFormat::Plain,
        };

        // Wait for auth request and authenticate with user
        handle.authenticate_user(user, password).await?;

        info!("Successfully connected and authenticated to FreeSWITCH");
        Ok(handle)
    }

    /// Accept outbound connection from FreeSWITCH
    pub async fn accept_outbound(listener: TcpListener) -> EslResult<Self> {
        info!("Waiting for outbound connection from FreeSWITCH");

        let (stream, addr) = listener.accept().await.map_err(EslError::Io)?;
        info!("Accepted outbound connection from {}", addr);

        let handle = Self {
            stream,
            parser: EslParser::new(),
            mode: ConnectionMode::Outbound,
            connected: true,
            authenticated: true, // Outbound connections don't require auth
            event_queue: Arc::new(Mutex::new(VecDeque::new())),
            read_buffer: [0u8; SOCKET_BUF_SIZE],
            event_format: EventFormat::Plain,
        };

        Ok(handle)
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        self.connected && self.authenticated
    }

    /// Check if authenticated
    pub fn is_authenticated(&self) -> bool {
        self.authenticated
    }

    /// Get connection mode
    pub fn mode(&self) -> ConnectionMode {
        self.mode
    }

    /// Disconnect from FreeSWITCH
    pub async fn disconnect(&mut self) -> EslResult<()> {
        if self.connected {
            info!("Disconnecting from FreeSWITCH");

            // Just drop the stream - don't call shutdown() which can hang
            // This matches the behavior of the C ESL library which just calls closesocket()
            self.connected = false;
            self.authenticated = false;
        }
        Ok(())
    }

    /// Authenticate with password
    async fn authenticate(&mut self, password: &str) -> EslResult<()> {
        debug!("Starting authentication");

        debug!("[AUTH] Waiting for auth request from FreeSWITCH");
        let message = self.recv_message().await?;
        debug!("[AUTH] Received message type: {:?}", message.message_type);

        if message.message_type != MessageType::AuthRequest {
            return Err(EslError::protocol_error("Expected auth request"));
        }

        debug!("[AUTH] Sending auth command");
        let auth_cmd = EslCommand::Auth {
            password: password.to_string(),
        };
        let response = self.send_command(auth_cmd).await?;
        debug!(
            "[AUTH] Received auth response: success={}",
            response.is_success()
        );

        if !response.is_success() {
            return Err(EslError::auth_failed(
                response
                    .reply_text()
                    .cloned()
                    .unwrap_or_else(|| "Authentication failed".to_string()),
            ));
        }

        self.authenticated = true;
        debug!("Authentication successful");
        Ok(())
    }

    /// Authenticate with user and password
    async fn authenticate_user(&mut self, user: &str, password: &str) -> EslResult<()> {
        debug!("Starting user authentication for user: {}", user);

        // Wait for auth request
        let message = self.recv_message().await?;
        if message.message_type != MessageType::AuthRequest {
            return Err(EslError::protocol_error("Expected auth request"));
        }

        // Send user authentication
        let auth_cmd = EslCommand::UserAuth {
            user: user.to_string(),
            password: password.to_string(),
        };
        let response = self.send_command(auth_cmd).await?;

        if !response.is_success() {
            return Err(EslError::auth_failed(
                response
                    .reply_text()
                    .cloned()
                    .unwrap_or_else(|| "User authentication failed".to_string()),
            ));
        }

        self.authenticated = true;
        debug!("User authentication successful");
        Ok(())
    }

    /// Send command and wait for response
    pub async fn send_command(&mut self, command: EslCommand) -> EslResult<EslResponse> {
        if !self.connected {
            return Err(EslError::NotConnected);
        }

        let command_str = command.to_wire_format();
        debug!("Sending command: {}", command_str.trim());

        // Send command
        self.stream
            .write_all(command_str.as_bytes())
            .await
            .map_err(EslError::Io)?;

        // Wait for response, filtering out log events
        let message = loop {
            let message = self.recv_message().await?;
            match message.message_type {
                MessageType::ApiResponse | MessageType::CommandReply => break message,
                MessageType::Event => {
                    // This is a log event or other event, ignore it and continue waiting
                    debug!(
                        "Ignoring event message while waiting for command response: {:?}",
                        message.message_type
                    );
                    continue;
                }
                MessageType::Disconnect => {
                    warn!("Received disconnect notice while waiting for command response");
                    self.connected = false;
                    return Err(EslError::ConnectionClosed);
                }
                _ => {
                    debug!(
                        "Ignoring unexpected message type while waiting for command response: {:?}",
                        message.message_type
                    );
                    continue;
                }
            }
        };
        let response = message.into_response();

        debug!("Received response: success={}", response.is_success());
        Ok(response)
    }

    /// Execute API command
    pub async fn api(&mut self, command: &str) -> EslResult<EslResponse> {
        let cmd = EslCommand::Api {
            command: command.to_string(),
        };
        self.send_command(cmd).await
    }

    /// Execute background API command
    pub async fn bgapi(&mut self, command: &str) -> EslResult<EslResponse> {
        let cmd = EslCommand::BgApi {
            command: command.to_string(),
        };
        self.send_command(cmd).await
    }

    /// Subscribe to events
    pub async fn subscribe_events(
        &mut self,
        format: EventFormat,
        events: &[EslEventType],
    ) -> EslResult<()> {
        self.event_format = format;

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

        let response = self.send_command(cmd).await?;
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

    /// Set event filter
    pub async fn filter_events(&mut self, header: &str, value: &str) -> EslResult<()> {
        let cmd = EslCommand::Filter {
            header: header.to_string(),
            value: value.to_string(),
        };

        let response = self.send_command(cmd).await?;
        response.into_result()?;

        debug!("Set event filter: {} = {}", header, value);
        Ok(())
    }

    /// Receive next event
    pub async fn recv_event(&mut self) -> EslResult<Option<EslEvent>> {
        // Check queue first
        {
            let mut queue = self.event_queue.lock().await;
            if let Some(event) = queue.pop_front() {
                return Ok(Some(event));
            }
        }

        // Read from network
        loop {
            let message = self.recv_message().await?;

            match message.message_type {
                MessageType::Event => {
                    let event = self.parser.parse_event(message, self.event_format)?;
                    return Ok(Some(event));
                }
                MessageType::Disconnect => {
                    warn!("Received disconnect notice");
                    self.connected = false;
                    return Ok(None);
                }
                _ => {
                    debug!("Ignoring non-event message: {:?}", message.message_type);
                    continue;
                }
            }
        }
    }

    /// Receive event with timeout
    pub async fn recv_event_timeout(&mut self, timeout_ms: u64) -> EslResult<Option<EslEvent>> {
        timeout(Duration::from_millis(timeout_ms), self.recv_event())
            .await
            .map_err(|_| EslError::Timeout { timeout_ms })?
    }

    /// Receive a protocol message
    async fn recv_message(&mut self) -> EslResult<crate::protocol::EslMessage> {
        loop {
            // Try to parse existing buffer first
            if let Some(message) = self.parser.parse_message()? {
                trace!(
                    "[RECV] Parsed message from buffer: {:?}",
                    message.message_type
                );
                return Ok(message);
            }

            trace!("[RECV] Buffer empty, reading from socket");
            // Add timeout to socket read to prevent hanging when outer timeout fires
            let read_result = timeout(
                Duration::from_millis(DEFAULT_TIMEOUT_MS),
                self.stream.read(&mut self.read_buffer),
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

            // Add to parser
            self.parser.add_data(&self.read_buffer[..bytes_read])?;
        }
    }

    /// Execute application on channel
    pub async fn execute(
        &mut self,
        app: &str,
        args: Option<&str>,
        uuid: Option<&str>,
    ) -> EslResult<EslResponse> {
        let cmd = EslCommand::Execute {
            app: app.to_string(),
            args: args.map(|s| s.to_string()),
            uuid: uuid.map(|s| s.to_string()),
        };
        self.send_command(cmd).await
    }

    /// Send message to channel
    pub async fn sendmsg(&mut self, uuid: Option<&str>, event: EslEvent) -> EslResult<EslResponse> {
        let cmd = EslCommand::SendMsg {
            uuid: uuid.map(|s| s.to_string()),
            event,
        };
        self.send_command(cmd).await
    }
}

// Clean shutdown on drop
impl Drop for EslHandle {
    fn drop(&mut self) {
        if self.connected {
            debug!("EslHandle dropped - connection will be closed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connection_mode() {
        // This is a placeholder test - you'd need a running FreeSWITCH for integration tests
        assert_eq!(ConnectionMode::Inbound, ConnectionMode::Inbound);
        assert_ne!(ConnectionMode::Inbound, ConnectionMode::Outbound);
    }
}
