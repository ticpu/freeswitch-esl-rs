//! Real FreeSWITCH outbound socket integration test
//!
//! Exercises the outbound socket path against a local FreeSWITCH container.
//! Requires FreeSWITCH ESL on port 8022 with password ClueCon and extension 9199
//! (echo + auto-hangup after ~8s) in the `test` context.
//!
//! Usage: cargo run --example outbound_test

use freeswitch_esl_tokio::{EslClient, EslEventType, EventFormat};
use std::time::Duration;
use tokio::net::TcpListener;

const ESL_HOST: &str = "127.0.0.1";
const ESL_PORT: u16 = 8022;
const ESL_PASSWORD: &str = "ClueCon";
const STEP_TIMEOUT: Duration = Duration::from_secs(15);

fn step_ok(name: &str) {
    println!("  PASS  {}", name);
}

fn step_fail(name: &str, err: &str) {
    println!("  FAIL  {}: {}", name, err);
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let mut pass = 0u32;
    let mut fail = 0u32;

    // Step 1: Bind outbound listener on a free port
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let outbound_port = listener
        .local_addr()?
        .port();
    step_ok(&format!("bind outbound listener on port {}", outbound_port));
    pass += 1;

    // Step 2: Connect inbound ESL
    let (inbound, _inbound_events) = EslClient::connect(ESL_HOST, ESL_PORT, ESL_PASSWORD).await?;
    inbound.set_command_timeout(Duration::from_secs(30));
    step_ok("connect inbound ESL");
    pass += 1;

    // Step 3: Originate call to outbound socket
    let originate_cmd = format!(
        "originate loopback/9199/test &socket(127.0.0.1:{} async full)",
        outbound_port
    );
    let resp = inbound
        .api(&originate_cmd)
        .await?;
    if resp.is_success()
        || resp
            .body()
            .map_or(false, |b| b.starts_with("+OK"))
    {
        step_ok("originate call");
        pass += 1;
    } else {
        step_fail(
            "originate call",
            &format!(
                "{:?}",
                resp.reply_text()
                    .or(resp.body())
            ),
        );
        fail += 1;
    }

    // Step 4: Accept outbound connection
    let (client, mut events) =
        match tokio::time::timeout(STEP_TIMEOUT, EslClient::accept_outbound(&listener)).await {
            Ok(Ok(pair)) => {
                step_ok("accept outbound connection");
                pass += 1;
                pair
            }
            Ok(Err(e)) => {
                step_fail("accept outbound connection", &e.to_string());
                fail += 1;
                print_summary(pass, fail);
                return Ok(());
            }
            Err(_) => {
                step_fail("accept outbound connection", "timeout");
                fail += 1;
                print_summary(pass, fail);
                return Ok(());
            }
        };

    // Step 5a: myevents
    match client
        .myevents(EventFormat::Plain)
        .await
    {
        Ok(()) => {
            step_ok("myevents plain");
            pass += 1;
        }
        Err(e) => {
            step_fail("myevents plain", &e.to_string());
            fail += 1;
        }
    }

    // Step 5b: linger
    match client
        .linger(Some(10))
        .await
    {
        Ok(()) => {
            step_ok("linger 10");
            pass += 1;
        }
        Err(e) => {
            step_fail("linger 10", &e.to_string());
            fail += 1;
        }
    }

    // Step 5c: resume
    match client
        .resume()
        .await
    {
        Ok(()) => {
            step_ok("resume");
            pass += 1;
        }
        Err(e) => {
            step_fail("resume", &e.to_string());
            fail += 1;
        }
    }

    // Step 6: Receive events, look for channel events
    let mut got_channel_event = false;
    let mut got_hangup = false;

    println!("  ...   waiting for channel events (up to 30s)...");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(Ok(event))) => {
                let etype = event.event_type();
                if !got_channel_event {
                    if matches!(
                        etype,
                        Some(
                            EslEventType::ChannelCreate
                                | EslEventType::ChannelAnswer
                                | EslEventType::ChannelState
                                | EslEventType::ChannelCallstate
                                | EslEventType::ChannelExecute
                                | EslEventType::ChannelExecuteComplete
                        )
                    ) {
                        step_ok(&format!("received channel event: {:?}", etype.unwrap()));
                        pass += 1;
                        got_channel_event = true;
                    }
                }
                if matches!(
                    etype,
                    Some(EslEventType::ChannelHangup | EslEventType::ChannelHangupComplete)
                ) {
                    step_ok(&format!("received hangup event: {:?}", etype.unwrap()));
                    pass += 1;
                    got_hangup = true;
                    break;
                }
            }
            Ok(Some(Err(e))) => {
                eprintln!("  event error: {}", e);
            }
            Ok(None) => {
                break;
            }
            Err(_) => {
                break;
            }
        }
    }

    if !got_channel_event {
        step_fail("received channel event", "no channel events seen");
        fail += 1;
    }
    if !got_hangup {
        step_fail("received hangup event", "no hangup event seen");
        fail += 1;
    }

    // Step 8: After hangup, check for linger disconnect-notice
    // The connection should stay open due to linger
    if got_hangup {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if client.is_connected() {
            step_ok("connection still alive after hangup (linger)");
            pass += 1;
        } else {
            step_fail(
                "connection still alive after hangup (linger)",
                "disconnected too early",
            );
            fail += 1;
        }
    }

    // Step 9: Cancel linger
    match client
        .nolinger()
        .await
    {
        Ok(()) => {
            step_ok("nolinger");
            pass += 1;
        }
        Err(e) => {
            // May fail if connection already closing
            step_fail("nolinger", &e.to_string());
            fail += 1;
        }
    }

    // Cleanup
    let _ = client
        .exit()
        .await;
    let _ = inbound
        .exit()
        .await;

    print_summary(pass, fail);
    Ok(())
}

fn print_summary(pass: u32, fail: u32) {
    println!();
    println!("Results: {} passed, {} failed", pass, fail);
    if fail > 0 {
        std::process::exit(1);
    }
}
