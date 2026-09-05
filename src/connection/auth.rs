use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tracing::{debug, trace, warn};

use crate::{
    command::{EslCommand, EslResponse, Secret},
    constants::{CONTENT_TYPE_COMMAND_REPLY, HEADER_CONTENT_TYPE},
    error::{EslError, EslResult},
    protocol::{EslMessage, EslParser, MessageType},
};

/// Authentication method for inbound connections.
pub(super) enum AuthMethod<'a> {
    Password(&'a str),
    User { user: &'a str, password: &'a str },
}

/// Read a single ESL message from the socket into the parser.
///
/// Used during auth handshake (on unsplit TcpStream) and would be the
/// basis for the reader loop, but the reader loop inlines this logic
/// to handle liveness tracking.
pub(super) async fn recv_message(
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
pub(super) async fn authenticate(
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
                // Nothing to salvage: the read timeout is the whole story. A
                // salvage that refused for a named reason is that reason.
                Ok(None) => return Err(EslError::Timeout { timeout_ms }),
                Err(e) => return Err(e),
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

    if content_type != CONTENT_TYPE_COMMAND_REPLY {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
    async fn salvage_failure_surfaces_its_own_reason() {
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("[::1]:0")
            .await
            .unwrap();
        let addr = listener
            .local_addr()
            .unwrap();

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener
                .accept()
                .await
                .unwrap();
            sock.write_all(b"Content-Type: auth/request\n\n")
                .await
                .unwrap();
            let mut discard = [0u8; 128];
            let n = sock
                .read(&mut discard)
                .await
                .unwrap();
            assert!(n > 0, "client sent no auth command");
            // A truncated block that is not an auth reply at all: the salvage
            // must say so rather than let the read timeout stand as the cause.
            sock.write_all(b"Content-Type: text/event-plain\nReply-Text: +OK\n")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
        });

        let mut stream = TcpStream::connect(addr)
            .await
            .unwrap();
        let mut parser = EslParser::new();
        let mut read_buffer = [0u8; 1024];
        let err = authenticate(
            &mut stream,
            &mut parser,
            &mut read_buffer,
            AuthMethod::Password("ClueCon"),
            Duration::from_millis(200),
        )
        .await
        .expect_err("a non-auth truncated reply must not authenticate");

        assert!(
            matches!(err, EslError::ProtocolError { .. }),
            "got: {err:?}"
        );
        assert!(
            err.to_string()
                .contains("command/reply"),
            "error must name the check that failed: {err}"
        );
        server.abort();
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
