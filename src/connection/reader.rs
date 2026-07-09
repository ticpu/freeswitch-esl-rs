use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::tcp::OwnedReadHalf;
use tokio::sync::mpsc;
use tokio::time::{timeout, Instant};
use tracing::{debug, info, warn};

use crate::{
    constants::{CONTENT_TYPE_LOG_DATA, HEADER_CONTENT_TYPE, SOCKET_BUF_SIZE},
    error::EslError,
    event::{EslEvent, EventFormat},
    protocol::{EslParser, MessageType},
};

use super::reexec::ReexecReader;
use super::{ConnectionStatus, DisconnectReason, SharedState};

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
    use std::sync::atomic::Ordering;

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
pub(super) async fn reader_loop(
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
    fail_pending_reply(&shared).await;
}

/// A reader-loop exit means no further replies can arrive on this socket.
///
/// The writer lock is held through the whole send-and-wait cycle, so at most
/// one waiter exists. Dropping its `oneshot::Sender` resolves the awaiting
/// `send_command`'s `rx` to `Err`, which maps to `EslError::ConnectionClosed`
/// — instead of the caller waiting out the full command timeout.
///
/// `reader_dead` is set under the same lock so a `send_command` that reaches
/// its install block after this ran fails fast instead of installing a
/// waiter no task will ever wake.
async fn fail_pending_reply(shared: &SharedState) {
    let mut pending = shared
        .pending_reply
        .lock()
        .await;
    pending.reader_dead = true;
    if pending
        .waiting
        .take()
        .is_some()
    {
        debug!("Failing in-flight command waiter: reader loop exited");
    }
    pending.stale_replies = 0;
}

async fn reader_loop_inner(
    mut reader: OwnedReadHalf,
    mut parser: EslParser,
    shared: Arc<SharedState>,
    event_tx: mpsc::Sender<Result<EslEvent, EslError>>,
    #[cfg_attr(not(unix), allow(unused_variables, unused_mut))] mut reexec: ReexecReader,
) {
    use std::sync::atomic::Ordering;

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
            use crate::constants::REEXEC_DRAIN_TIMEOUT_MS;
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
