//! Re-exec teardown and adopt demo
//!
//! Demonstrates the full re-exec cycle without actually calling exec():
//! 1. Connect to FreeSWITCH, subscribe to events
//! 2. Teardown: stop the reader, get the fd and residual bytes
//! 3. Adopt: reconstruct a new EslClient from the same fd
//! 4. Verify events still flow on the adopted connection
//!
//! Requires FreeSWITCH ESL. Configure via ESL_HOST, ESL_PORT, ESL_PASSWORD env vars.
//!
//! Usage: cargo run --example reexec_demo

#[cfg(unix)]
mod demo {
    use freeswitch_esl_tokio::{
        EslClient, EslError, EslEventStream, EslEventType, EventFormat, EventHeader,
        DEFAULT_ESL_PASSWORD, DEFAULT_ESL_PORT,
    };
    use std::os::unix::io::BorrowedFd;
    use std::time::Duration;
    use tracing::{error, info};

    /// FreeSWITCH fires HEARTBEAT about every 20s, so one that has not arrived
    /// by now is not late.
    const HEARTBEAT_WAIT: Duration = Duration::from_secs(25);

    /// Wait for one heartbeat and report what it carried.
    ///
    /// Failing here has to be an error: the demo's only claim is that events
    /// flow, and a version that logged and carried on printed "complete" and
    /// exited 0 having proved nothing.
    async fn expect_heartbeat(
        events: &mut EslEventStream,
        stage: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match tokio::time::timeout(HEARTBEAT_WAIT, events.recv()).await {
            Ok(Some(Ok(event))) => {
                info!(
                    "{stage}: heartbeat, {}",
                    event
                        .header(EventHeader::EventInfo)
                        .unwrap_or("(no info)")
                );
                Ok(())
            }
            Ok(Some(Err(e))) => Err(format!("{stage}: event error: {e}").into()),
            Ok(None) => Err(format!("{stage}: event stream closed").into()),
            Err(_) => Err(format!("{stage}: no heartbeat within {HEARTBEAT_WAIT:?}").into()),
        }
    }

    pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
        tracing_subscriber::fmt::init();

        let host = std::env::var("ESL_HOST").unwrap_or_else(|_| "localhost".to_string());
        let port: u16 = match std::env::var("ESL_PORT") {
            Ok(value) => value.parse()?,
            Err(_) => DEFAULT_ESL_PORT,
        };
        let password =
            std::env::var("ESL_PASSWORD").unwrap_or_else(|_| DEFAULT_ESL_PASSWORD.to_string());

        // Phase 1: connect and subscribe
        let (client, mut events) = match EslClient::connect(&host, port, &password).await {
            Ok(pair) => pair,
            Err(EslError::Io(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                error!("nothing listening on {host}:{port} (set ESL_HOST / ESL_PORT)");
                return Err(e.into());
            }
            Err(e) => return Err(e.into()),
        };
        info!("connected to {host}:{port}");

        client
            .subscribe_events(EventFormat::Plain, &[EslEventType::Heartbeat])
            .await?;

        expect_heartbeat(&mut events, "before teardown").await?;
        drop(events);

        // Phase 2: teardown
        info!("Tearing down for re-exec...");
        let (fd, residual) = client
            .teardown_for_reexec()
            .await?;
        info!(
            "Teardown complete: fd={}, {} residual bytes",
            fd,
            residual.len()
        );

        // In a real re-exec scenario, you would:
        //   1. Serialize app state + residual to disk/env
        //   2. Clear CLOEXEC: nix::fcntl::fcntl(fd, F_SETFD(FdFlag::empty()))
        //   3. std::mem::forget(client)
        //   4. exec() the new binary
        //
        // Here we simulate the new process side without exec().
        //
        // Demo caveat: in a real exec(), mem::forget(client) is sufficient
        // because the old tokio reactor (epoll fd) has CLOEXEC and is gone
        // after exec. In this same-process demo, the reactor still has the
        // original fd registered. We dup() to get a clean fd not known to
        // the reactor, then forget the client (leaks the old registration,
        // but keeps the TCP connection alive by not sending FIN).
        // Safety: fd is a valid open descriptor from teardown_for_reexec().
        // We borrow it for dup() without taking ownership (client still holds it).
        let dup_fd = nix::unistd::dup(unsafe { BorrowedFd::borrow_raw(fd) })?;
        std::mem::forget(client);

        // Phase 3: adopt the stream (simulating new process)
        info!("Adopting stream (simulating new process)...");

        // Reconstruct a TcpStream from the dup'd fd.
        // OwnedFd transfers ownership to TcpStream (no close-on-drop race).
        let std_stream = std::net::TcpStream::from(dup_fd);
        std_stream.set_nonblocking(true)?;
        let tokio_stream = tokio::net::TcpStream::from_std(std_stream)?;

        let (new_client, mut new_events) = EslClient::adopt_stream(tokio_stream, &residual)?;

        // Exercise the write half too. Reading alone would leave the demo
        // claiming a working connection on the evidence of one direction.
        let response = new_client
            .api("status")
            .await?;
        let status = response.api_result()?;
        info!(
            "adopted connection answers commands: {}",
            status
                .lines()
                .next()
                .unwrap_or("(empty)")
        );

        // The old subscription survives on the TCP connection, so
        // heartbeats arrive without re-subscribing.
        expect_heartbeat(&mut new_events, "after adopt").await?;

        info!("re-exec demo complete");
        new_client
            .disconnect()
            .await?;
        Ok(())
    }
}

#[cfg(unix)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    demo::run().await
}

#[cfg(not(unix))]
fn main() {
    eprintln!("reexec_demo requires unix (raw fd support)");
    std::process::exit(1);
}
