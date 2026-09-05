use std::ops::ControlFlow;
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
    protocol::{EslMessage, EslParser, MessageType},
};

use super::reexec::ReexecReader;
use super::{ConnectionStatus, DisconnectReason, SharedState};

/// Hands one event (or error) to the application, returning `false` once the
/// receiver is gone. A full channel drops the item and arms a `QueueFull`
/// notice for the next dispatch to deliver.
#[must_use]
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
        &event_tx,
        reexec,
    ));
    let reason = match futures_util::FutureExt::catch_unwind(result).await {
        Ok(reason) => reason,
        Err(_) => {
            tracing::error!("reader task panicked");
            Some(DisconnectReason::IoError(
                "reader task panicked".to_string(),
            ))
        }
    };
    // `event_tx` is still alive here: a consumer that sees recv() -> None must
    // find the disconnect status already published, not a stale Connected.
    if let Some(reason) = reason {
        if shared
            .status_tx
            .send(ConnectionStatus::Disconnected(reason))
            .is_err()
        {
            debug!("No status receiver left to observe the disconnect");
        }
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

/// Routes one parsed message to the event channel or to the waiting command.
///
/// `Break` ends the reader loop, carrying the same reason its exits return.
async fn dispatch_message(
    message: EslMessage,
    parser: &EslParser,
    shared: &SharedState,
    event_tx: &mpsc::Sender<Result<EslEvent, EslError>>,
) -> ControlFlow<Option<DisconnectReason>> {
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
                            event_tx,
                            shared,
                            Err(EslError::InvalidEventFormat {
                                format: e
                                    .0
                                    .clone(),
                            }),
                        ) {
                            debug!("Event channel closed, reader exiting");
                            return ControlFlow::Break(None);
                        }
                        return ControlFlow::Continue(());
                    }
                    None => EventFormat::Plain,
                }
            };

            let event_result = parser.parse_event(message, format);
            if !dispatch_event(event_tx, shared, event_result) {
                debug!("Event channel closed, reader exiting");
                return ControlFlow::Break(None);
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
                    debug!("Reply channel closed before delivery (timeout race); reply discarded");
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
                return ControlFlow::Continue(());
            }
            let controlled_session_uuid = message
                .headers
                .get("Controlled-Session-UUID")
                .cloned();
            info!("Received disconnect notice from server");
            return ControlFlow::Break(Some(DisconnectReason::ServerNotice {
                controlled_session_uuid,
                body: message.body,
            }));
        }
        MessageType::RudeRejection => {
            let reason = message
                .body
                .unwrap_or_else(|| "rude-rejection without body".to_string());
            warn!("Rude rejection from server: {}", reason);
            if !dispatch_event(
                event_tx,
                shared,
                Err(EslError::AccessDenied {
                    reason: reason.clone(),
                }),
            ) {
                debug!("Event channel closed before the rude rejection was delivered");
            }
            return ControlFlow::Break(Some(DisconnectReason::AccessDenied(reason)));
        }
        MessageType::AuthRequest => {
            // Post-authentication it means FreeSWITCH and the client are out of
            // sync, so the session cannot be trusted to continue.
            let reason = "unsolicited auth/request received after authentication".to_string();
            warn!("{reason}");
            if !dispatch_event(
                event_tx,
                shared,
                Err(EslError::protocol_error(reason.clone())),
            ) {
                debug!("Event channel closed before the desync error was delivered");
            }
            return ControlFlow::Break(Some(DisconnectReason::ProtocolError(reason)));
        }
    }
    ControlFlow::Continue(())
}

/// The teardown caller awaits this channel, so a drain that ends without
/// sending leaves it blocked until its own timeout.
#[cfg(unix)]
fn fail_reexec(reexec: &mut ReexecReader, error: EslError) {
    if let Some(tx) = reexec
        .result_tx
        .take()
    {
        if tx
            .send(Err(error))
            .is_err()
        {
            debug!("Re-exec caller gone before the drain failure was delivered");
        }
    }
}

/// One drain step once the re-exec stop signal has fired.
///
/// Stops only at a clean message boundary: mid-body, the residual would be a
/// partial body without its headers, which the new process cannot parse.
#[cfg(unix)]
async fn drain_for_reexec(
    reader: &mut OwnedReadHalf,
    parser: &mut EslParser,
    read_buffer: &mut [u8],
    reexec: &mut ReexecReader,
) -> ControlFlow<Option<DisconnectReason>> {
    use crate::constants::REEXEC_DRAIN_TIMEOUT_MS;

    if parser.is_waiting_for_headers() {
        let residual = parser
            .remaining_bytes()
            .to_vec();
        debug!("Re-exec drain complete, {} residual bytes", residual.len());
        if let Some(tx) = reexec
            .result_tx
            .take()
        {
            if tx
                .send(Ok(residual))
                .is_err()
            {
                warn!("Re-exec caller gone before the residual bytes were delivered");
            }
        }
        return ControlFlow::Break(Some(DisconnectReason::ReexecTeardown));
    }

    // WaitingForBody: more socket data is needed to finish the current message.
    let drain_timeout = Duration::from_millis(REEXEC_DRAIN_TIMEOUT_MS);
    match timeout(drain_timeout, reader.read(read_buffer)).await {
        Ok(Ok(0)) => {
            warn!("Connection closed during re-exec drain");
            fail_reexec(
                reexec,
                EslError::ReexecFailed {
                    reason: "connection closed during drain".into(),
                },
            );
            ControlFlow::Break(None)
        }
        Ok(Ok(n)) => {
            if let Err(e) = parser.add_data(&read_buffer[..n]) {
                warn!("Buffer error during re-exec drain: {}", e);
                fail_reexec(reexec, e);
                return ControlFlow::Break(None);
            }
            ControlFlow::Continue(())
        }
        Ok(Err(e)) => {
            warn!("Read error during re-exec drain: {}", e);
            fail_reexec(reexec, EslError::Io(e));
            ControlFlow::Break(None)
        }
        Err(_) => {
            warn!("Re-exec drain timeout waiting for message body");
            fail_reexec(
                reexec,
                EslError::ReexecFailed {
                    reason: "drain timeout waiting for message body".into(),
                },
            );
            ControlFlow::Break(None)
        }
    }
}

/// Traffic-idle check for the read-timeout tick; a zero threshold disables it.
fn liveness_expired(shared: &SharedState, last_recv: Instant) -> bool {
    use std::sync::atomic::Ordering;

    let threshold_ms = shared
        .liveness_timeout_ms
        .load(Ordering::Relaxed);
    if threshold_ms == 0 {
        return false;
    }
    let elapsed = last_recv.elapsed();
    if elapsed <= Duration::from_millis(threshold_ms) {
        return false;
    }
    warn!(
        "Liveness timeout: {}ms without traffic (threshold {}ms)",
        elapsed.as_millis(),
        threshold_ms
    );
    true
}

/// Returns the reason its caller broadcasts, or `None` for the exits that
/// publish none: a closed event channel, or a re-exec failure already delivered
/// on the teardown result channel.
async fn reader_loop_inner(
    mut reader: OwnedReadHalf,
    mut parser: EslParser,
    shared: Arc<SharedState>,
    event_tx: &mpsc::Sender<Result<EslEvent, EslError>>,
    #[cfg_attr(not(unix), allow(unused_variables, unused_mut))] mut reexec: ReexecReader,
) -> Option<DisconnectReason> {
    let mut read_buffer = [0u8; SOCKET_BUF_SIZE];
    let mut last_recv = Instant::now();
    #[cfg(unix)]
    let mut draining = false;

    loop {
        // Try to parse a complete message from buffered data first
        match parser.parse_message() {
            Ok(Some(message)) => {
                match dispatch_message(message, &parser, &shared, event_tx).await {
                    ControlFlow::Continue(()) => continue,
                    ControlFlow::Break(reason) => return reason,
                }
            }
            Ok(None) => {
                // Need more data from socket
            }
            Err(e) => {
                warn!("Parser error: {}", e);
                return Some(DisconnectReason::ProtocolError(e.to_string()));
            }
        }

        // Only reached with no complete message buffered, which is the state
        // the drain inspects for a clean stop.
        #[cfg(unix)]
        if draining {
            match drain_for_reexec(&mut reader, &mut parser, &mut read_buffer, &mut reexec).await {
                ControlFlow::Continue(()) => continue,
                ControlFlow::Break(reason) => return reason,
            }
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
                return Some(DisconnectReason::ConnectionClosed);
            }
            Ok(Ok(n)) => {
                last_recv = Instant::now();
                if let Err(e) = parser.add_data(&read_buffer[..n]) {
                    warn!("Buffer error: {}", e);
                    return Some(DisconnectReason::ProtocolError(e.to_string()));
                }
            }
            Ok(Err(e)) => {
                warn!("Read error: {}", e);
                return Some(DisconnectReason::IoError(e.to_string()));
            }
            Err(_) => {
                if liveness_expired(&shared, last_recv) {
                    return Some(DisconnectReason::HeartbeatExpired);
                }
            }
        }
    }
}
