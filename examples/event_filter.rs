//! ESL Event Filter - Filter events by header values
//!
//! Usage: cargo run --example event_filter -- [OPTIONS]
//!
//! Examples:
//!   # Filter CHANNEL_CREATE events where caller ID contains "ra232"
//!   cargo run --example event_filter -- -e CHANNEL_CREATE -f Caller-Caller-ID-Number -v '/ra232/'
//!
//!   # Exact match filter
//!   cargo run --example event_filter -- -e CHANNEL_CREATE -f Caller-Caller-ID-Number -v 1001
//!
//!   # Multiple events
//!   cargo run --example event_filter -- -e CHANNEL_CREATE -e CHANNEL_ANSWER -f Call-Direction -v inbound
//!
//!   # Exit after the first 5 matching events
//!   cargo run --example event_filter -- -e CHANNEL_CREATE -c 5
//!
//!   # With userauth (user@domain format required)
//!   cargo run --example event_filter -- -u admin@default -p secret -e ALL

#[path = "common/env.rs"]
mod env;

use freeswitch_esl_tokio::connection::{AuthMethod, EslConnectOptions};
use freeswitch_esl_tokio::{
    EslClient, EslError, EslEventType, EventFormat, EventSubscription, HeaderLookup,
};

fn print_usage() {
    eprintln!(
        r#"ESL Event Filter - Filter FreeSWITCH events by header values

Usage: event_filter [OPTIONS]

Connection Options:
  -H, --host <HOST>        FreeSWITCH host (default: localhost)
  -P, --port <PORT>        ESL port (default: 8021)
  -p, --password <PASS>    ESL password (default: ClueCon)
  -u, --user <USER>        Username for userauth (format: user@domain)

Filter Options:
  -e, --event <EVENT>      Event type to subscribe to (can be repeated)
                           Examples: CHANNEL_CREATE, CHANNEL_ANSWER, ALL
  -f, --filter <HEADER>    Header name to filter on
  -v, --value <VALUE>      Value to match (use /regex/ for regex matching)
  -c, --max-count <N>      Exit after N matching events (default: run forever)

Output Options (default: every header, in a delimited block)
  -j, --json               One JSON object per event, body included
  -r, --raw                The event exactly as it arrived on the wire
  -q, --quiet              One summary line per event

Examples:
  # Filter CHANNEL_CREATE events where caller contains "ra232"
  event_filter -e CHANNEL_CREATE -f Caller-Caller-ID-Number -v '/ra232/'

  # Exact match on call direction
  event_filter -e CHANNEL_CREATE -f Call-Direction -v inbound

  # Multiple events with JSON output
  event_filter -e CHANNEL_CREATE -e CHANNEL_ANSWER -f Caller-Context -v public -j

  # Print the next 3 matching events then exit
  event_filter -e CHANNEL_CREATE -f Call-Direction -v inbound -c 3

  # With userauth (user@domain format)
  event_filter -u admin@default -p secret -e ALL

Common Header Names:
  Caller-Caller-ID-Number   Calling party number
  Caller-Caller-ID-Name     Calling party name
  Caller-Destination-Number Destination number
  Call-Direction            inbound/outbound
  Caller-Context            Dialplan context
  variable_sip_from_user    SIP From username
  variable_sip_to_user      SIP To username
  Unique-ID                 Channel UUID
"#
    );
}

#[derive(Debug)]
struct Args {
    host: String,
    port: u16,
    user: Option<String>,
    password: String,
    events: Vec<String>,
    filter_header: Option<String>,
    filter_value: Option<String>,
    max_count: Option<usize>,
    json_output: bool,
    raw_output: bool,
    quiet: bool,
}

impl Args {
    /// Defaults from the environment, before the flags below override them.
    fn from_env() -> Result<Self, String> {
        let env = env::EslEnv::from_env()?;
        Ok(Self {
            host: env.host,
            port: env.port,
            user: None,
            password: env.password,
            events: vec!["CHANNEL_CREATE".to_string()],
            filter_header: None,
            filter_value: None,
            max_count: None,
            json_output: false,
            raw_output: false,
            quiet: false,
        })
    }
}

fn parse_args() -> Result<Args, String> {
    let args: Vec<String> = std::env::args().collect();
    let mut result = Args::from_env()?;
    result
        .events
        .clear();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "-H" | "--host" => {
                i += 1;
                result.host = args
                    .get(i)
                    .ok_or("Missing host value")?
                    .clone();
            }
            "-P" | "--port" => {
                i += 1;
                result.port = args
                    .get(i)
                    .ok_or("Missing port value")?
                    .parse()
                    .map_err(|_| "Invalid port number")?;
            }
            "-p" | "--password" => {
                i += 1;
                result.password = args
                    .get(i)
                    .ok_or("Missing password value")?
                    .clone();
            }
            "-u" | "--user" => {
                i += 1;
                let user = args
                    .get(i)
                    .ok_or("Missing user value")?
                    .clone();
                if !user.contains('@') {
                    return Err(format!(
                        "Invalid user format '{}': must be user@domain (e.g., admin@default)",
                        user
                    ));
                }
                result.user = Some(user);
            }
            "-e" | "--event" => {
                i += 1;
                result
                    .events
                    .push(
                        args.get(i)
                            .ok_or("Missing event value")?
                            .clone(),
                    );
            }
            "-f" | "--filter" => {
                i += 1;
                result.filter_header = Some(
                    args.get(i)
                        .ok_or("Missing filter header")?
                        .clone(),
                );
            }
            "-v" | "--value" => {
                i += 1;
                result.filter_value = Some(
                    args.get(i)
                        .ok_or("Missing filter value")?
                        .clone(),
                );
            }
            "-c" | "--max-count" => {
                i += 1;
                let count: usize = args
                    .get(i)
                    .ok_or("Missing max-count value")?
                    .parse()
                    .map_err(|_| "Invalid max-count: expected a positive integer")?;
                if count == 0 {
                    return Err("--max-count must be at least 1".to_string());
                }
                result.max_count = Some(count);
            }
            "-j" | "--json" => {
                result.json_output = true;
            }
            "-r" | "--raw" => {
                result.raw_output = true;
            }
            "-q" | "--quiet" => {
                result.quiet = true;
            }
            arg => {
                return Err(format!("Unknown argument: {}", arg));
            }
        }
        i += 1;
    }

    if result
        .events
        .is_empty()
    {
        result
            .events
            .push("CHANNEL_CREATE".to_string());
    }

    if result
        .filter_header
        .is_some()
        != result
            .filter_value
            .is_some()
    {
        return Err("Both --filter and --value must be specified together".to_string());
    }

    Ok(result)
}

/// Enough of the UUID to correlate lines, truncated by character: a value off
/// the wire may have decoded lossily, and slicing a replacement in half panics.
fn short_uuid(uuid: &str) -> String {
    uuid.chars()
        .take(8)
        .collect()
}

fn format_event_summary(event: &freeswitch_esl_tokio::EslEvent) -> String {
    let event_name = event
        .event_type()
        .map(|t| t.to_string())
        .unwrap_or_else(|| "UNKNOWN".into());
    let caller_id = event
        .caller_id_number()
        .unwrap_or("-");
    let dest = event
        .destination_number()
        .unwrap_or("-");
    let direction = match event.call_direction() {
        Ok(Some(d)) => d.to_string(),
        Ok(None) => "-".into(),
        Err(e) => format!("!ERR({e})"),
    };
    let uuid = event
        .unique_id()
        .map_or_else(|| "-".to_string(), short_uuid);

    format!("[{event_name}] {caller_id} -> {dest} ({direction}) uuid:{uuid}...")
}

fn format_event_full(event: &freeswitch_esl_tokio::EslEvent) -> String {
    use std::fmt::Write;

    let mut output = String::from("---EVENT---\n");
    for (key, value) in event.headers() {
        // Writing to a String cannot fail, so there is no error to handle.
        let _ = writeln!(output, "{key}: {value}");
    }
    if let Some(body) = event.body() {
        let _ = writeln!(output, "\n{body}");
    }
    output.push_str("-----------\n");
    output
}

/// The whole event, body included. Serializing only the headers would drop the
/// payload that DTMF and BACKGROUND_JOB events carry, which the other output
/// modes print.
fn format_event_json(event: &freeswitch_esl_tokio::EslEvent) -> Result<String, serde_json::Error> {
    serde_json::to_string(event)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = match parse_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("Error: {}", e);
            eprintln!();
            print_usage();
            std::process::exit(1);
        }
    };

    if let Some(ref user) = args.user {
        eprintln!(
            "Connecting to FreeSWITCH at {}:{} as {}...",
            args.host, args.port, user
        );
    } else {
        eprintln!("Connecting to FreeSWITCH at {}:{}...", args.host, args.port);
    }

    let auth = match args.user {
        Some(ref user) => AuthMethod::user(user, &args.password),
        None => AuthMethod::password(&args.password),
    };
    let connect_result =
        EslClient::connect_with_auth(&args.host, args.port, auth, EslConnectOptions::new()).await;

    let (client, mut events) = match connect_result {
        Ok(pair) => pair,
        Err(EslError::AuthenticationFailed { ref reason }) => {
            eprintln!("Authentication failed: {}", reason);
            std::process::exit(1);
        }
        // An ACL rejection arrives as text/rude-rejection before auth, so it
        // reads as a connection failure unless it is named.
        Err(EslError::AccessDenied { ref reason }) => {
            eprintln!("Rejected by the ESL ACL: {}", reason);
            std::process::exit(1);
        }
        Err(EslError::Io(ref e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            eprintln!(
                "Connection refused - is FreeSWITCH running on {}:{}?",
                args.host, args.port
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Failed to connect: {}", e);
            std::process::exit(1);
        }
    };

    eprintln!("Connected successfully");

    // FromStr carries the crate's own error rather than a stringified one.
    let event_types = match args
        .events
        .iter()
        .map(|name| name.parse::<EslEventType>())
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(types) => types,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    let format = if args.json_output {
        EventFormat::Json
    } else {
        EventFormat::Plain
    };

    // Build an EventSubscription combining events and filters into one unit
    let mut sub = EventSubscription::new(format).events(event_types);
    if let (Some(header), Some(value)) = (&args.filter_header, &args.filter_value) {
        sub = sub.filter_raw(header, value)?;
        eprintln!(
            "Subscribing to events: {:?} with filter {}={}",
            args.events, header, value
        );
    } else {
        eprintln!("Subscribing to events: {:?}", args.events);
    }
    client
        .apply_subscription(&sub)
        .await?;

    match args.max_count {
        Some(n) => eprintln!("Listening for {} event(s)... (Ctrl+C to exit early)\n", n),
        None => eprintln!("Listening for events... (Ctrl+C to exit)\n"),
    }

    let mut matched = 0usize;
    let mut reached_max = false;

    while let Some(result) = events
        .recv()
        .await
    {
        let event = match result {
            Ok(event) => event,
            Err(e) => {
                eprintln!("Event error: {}", e);
                continue;
            }
        };
        let output = if args.json_output {
            format_event_json(&event)?
        } else if args.raw_output {
            // The wire form the reader would see on the socket, not a rendering
            // of it: that is what makes --raw worth having.
            event.to_plain_format()
        } else if args.quiet {
            format_event_summary(&event)
        } else {
            format_event_full(&event)
        };
        println!("{output}");

        matched += 1;
        if args
            .max_count
            .is_some_and(|max| matched >= max)
        {
            reached_max = true;
            break;
        }
    }

    if reached_max {
        eprintln!("Reached max count of {} event(s)", matched);
    } else {
        eprintln!("Connection closed by server");
    }
    client
        .disconnect()
        .await?;

    Ok(())
}
