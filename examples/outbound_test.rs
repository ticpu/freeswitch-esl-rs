//! The outbound session verbs, in the order a session needs them.
//!
//! `outbound_server` shows call control. This one shows the protocol around it:
//! `connect` first, then `myevents` to scope delivery, `linger` so the socket
//! outlives the hangup, `resume` so the dialplan carries on if we go away, and
//! `nolinger` to give the socket back.
//!
//! It drives itself: it originates a loopback call into its own listener over a
//! second, inbound connection, so nothing has to be dialled by hand.
//!
//! Needs FreeSWITCH with `mod_loopback` and extension 9199 in context `test`
//! (echo, auto-hangup after ~8s). Configure the inbound connection with
//! ESL_HOST / ESL_PORT / ESL_PASSWORD.
//!
//! Usage: ESL_PORT=8022 cargo run --example outbound_test

use freeswitch_esl_tokio::commands::endpoint::LoopbackEndpoint;
use freeswitch_esl_tokio::commands::originate::{Application, Endpoint, Originate};
use freeswitch_esl_tokio::{
    EslClient, EslEventType, EventFormat, HeaderLookup, DEFAULT_ESL_PASSWORD, DEFAULT_ESL_PORT,
};
use std::time::Duration;
use tokio::net::TcpListener;

/// Long enough for FreeSWITCH to route the loopback call into our listener.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(15);

/// Extension 9199 hangs up on its own after roughly eight seconds.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let host = std::env::var("ESL_HOST").unwrap_or_else(|_| "localhost".to_string());
    let port: u16 = match std::env::var("ESL_PORT") {
        Ok(value) => value.parse()?,
        Err(_) => DEFAULT_ESL_PORT,
    };
    let password =
        std::env::var("ESL_PASSWORD").unwrap_or_else(|_| DEFAULT_ESL_PASSWORD.to_string());

    // Port 0 lets the kernel pick, so concurrent runs do not collide.
    let listener = TcpListener::bind("[::]:0").await?;
    let outbound_port = listener
        .local_addr()?
        .port();
    println!("outbound listener on port {outbound_port}");

    let (inbound, _inbound_events) = EslClient::connect(&host, port, &password).await?;
    // api originate blocks until the call answers.
    inbound.set_command_timeout(Duration::from_secs(30));

    // The socket app's argument contains spaces, and originate splits on them;
    // the Originate builder quotes it. `async full` is what makes linger and
    // the api verbs available at all -- see docs/outbound-esl-quirks.md.
    let originate = Originate::application(
        Endpoint::Loopback(LoopbackEndpoint::new("9199").with_context("test")),
        Application::new(
            "socket",
            Some(format!("127.0.0.1:{outbound_port} async full")),
        ),
    );

    // The loopback answers on its own, so this returns before the socket app on
    // the A leg has reached our listener.
    let response = inbound
        .api(&originate.to_string())
        .await?;
    let call_uuid = response.api_result()?;
    println!("originated {call_uuid}");

    let (client, mut events) =
        tokio::time::timeout(ACCEPT_TIMEOUT, EslClient::accept_outbound(&listener)).await??;

    // connect must come first; its reply is the channel data.
    let session = client
        .connect_session()
        .await?
        .into_result()?;
    println!(
        "connected: channel={} control={} mode={}",
        session
            .channel_name()
            .unwrap_or("(unknown)"),
        // Control and Socket-Mode are connect-reply headers with no typed
        // accessor: they describe the socket, not the channel.
        session
            .header_str("Control")
            .unwrap_or("(missing)"),
        session
            .header_str("Socket-Mode")
            .unwrap_or("(missing)"),
    );

    client
        .myevents(EventFormat::Plain)
        .await?;
    // No timeout argument: not every FreeSWITCH build accepts `linger <seconds>`.
    client
        .linger(None)
        .await?;
    client
        .resume()
        .await?;
    println!("myevents, linger and resume accepted");

    // The call is already in echo mode, so create and answer have fired. What
    // is left to see is 9199 hanging up on its own.
    println!("waiting for the call to end...");
    let deadline = tokio::time::Instant::now() + CALL_TIMEOUT;
    let mut seen = 0u32;
    let mut hung_up = false;
    while !hung_up {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(event))) => {
                seen += 1;
                if let Some(event_type) = event.event_type() {
                    println!("  event: {event_type}");
                    hung_up = matches!(
                        event_type,
                        EslEventType::ChannelHangup | EslEventType::ChannelHangupComplete
                    );
                }
            }
            Ok(Some(Err(e))) => {
                return Err(format!("event stream error after {seen} events: {e}").into())
            }
            Ok(None) => return Err(format!("event stream closed after {seen} events").into()),
            Err(_) => {
                return Err(format!("no hangup within {CALL_TIMEOUT:?} ({seen} events)").into())
            }
        }
    }

    // linger is what keeps the socket up past the hangup; without it the
    // events above would be the last thing this connection ever saw.
    tokio::time::sleep(Duration::from_secs(1)).await;
    if !client.is_connected() {
        return Err("socket closed at hangup despite linger".into());
    }
    println!("still connected after hangup, as linger promises");

    client
        .nolinger()
        .await?;
    client
        .exit()
        .await?
        .check()?;
    inbound
        .exit()
        .await?
        .check()?;

    println!("done");
    Ok(())
}
