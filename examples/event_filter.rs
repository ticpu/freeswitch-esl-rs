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
//!   # With userauth (user@domain format required)
//!   cargo run --example event_filter -- -u admin@default -p secret -e ALL

use freeswitch_esl_tokio::{EslClient, EslError, EslEventType, EventFormat};

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

Output Options:
  -j, --json               Output events as JSON
  -r, --raw                Output raw event format
  -q, --quiet              Only output matching event headers, not full event

Examples:
  # Filter CHANNEL_CREATE events where caller contains "ra232"
  event_filter -e CHANNEL_CREATE -f Caller-Caller-ID-Number -v '/ra232/'

  # Exact match on call direction
  event_filter -e CHANNEL_CREATE -f Call-Direction -v inbound

  # Multiple events with JSON output
  event_filter -e CHANNEL_CREATE -e CHANNEL_ANSWER -f Caller-Context -v public -j

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
    json_output: bool,
    raw_output: bool,
    quiet: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 8021,
            user: None,
            password: "ClueCon".to_string(),
            events: vec!["CHANNEL_CREATE".to_string()],
            filter_header: None,
            filter_value: None,
            json_output: false,
            raw_output: false,
            quiet: false,
        }
    }
}

fn parse_args() -> Result<Args, String> {
    let args: Vec<String> = std::env::args().collect();
    let mut result = Args::default();
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

fn parse_event_type(name: &str) -> Result<EslEventType, String> {
    EslEventType::parse_event_type(name).ok_or_else(|| format!("Unknown event type: {}", name))
}

fn format_event_summary(event: &freeswitch_esl_tokio::EslEvent) -> String {
    let event_name = event
        .header("Event-Name")
        .unwrap_or("UNKNOWN");
    let caller_id = event
        .header("Caller-Caller-ID-Number")
        .unwrap_or("-");
    let dest = event
        .header("Caller-Destination-Number")
        .unwrap_or("-");
    let direction = event
        .header("Call-Direction")
        .unwrap_or("-");
    let uuid = event
        .unique_id()
        .map(|s| &s[..8.min(s.len())])
        .unwrap_or("-");

    format!(
        "[{}] {} -> {} ({}) uuid:{}...",
        event_name, caller_id, dest, direction, uuid
    )
}

fn format_event_full(event: &freeswitch_esl_tokio::EslEvent) -> String {
    let mut output = String::new();
    output.push_str("---EVENT---\n");
    for (key, value) in event.headers() {
        output.push_str(&format!("{}: {}\n", key, value));
    }
    if let Some(body) = event.body() {
        output.push_str(&format!("\n{}\n", body));
    }
    output.push_str("-----------\n");
    output
}

fn format_event_json(event: &freeswitch_esl_tokio::EslEvent) -> String {
    serde_json::to_string(event.headers()).unwrap_or_else(|_| "{}".to_string())
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

    let connect_result = if let Some(ref user) = args.user {
        EslClient::connect_with_user(&args.host, args.port, user, &args.password).await
    } else {
        EslClient::connect(&args.host, args.port, &args.password).await
    };

    let (client, mut events) = match connect_result {
        Ok(pair) => pair,
        Err(EslError::AuthenticationFailed { ref reason }) => {
            eprintln!("Authentication failed: {}", reason);
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

    let event_types: Result<Vec<EslEventType>, String> = args
        .events
        .iter()
        .map(|e| parse_event_type(e))
        .collect();

    let event_types = match event_types {
        Ok(types) => types,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    let format = if args.json_output {
        EventFormat::Json
    } else {
        EventFormat::Plain
    };

    eprintln!("Subscribing to events: {:?}", args.events);
    client
        .subscribe_events(format, &event_types)
        .await?;

    if let (Some(header), Some(value)) = (&args.filter_header, &args.filter_value) {
        eprintln!("Applying filter: {} = {}", header, value);
        client
            .filter_events(header, value)
            .await?;
    }

    eprintln!("Listening for events... (Ctrl+C to exit)\n");

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
            format_event_json(&event)
        } else if args.raw_output {
            format_event_full(&event)
        } else if args.quiet {
            format_event_summary(&event)
        } else {
            format_event_full(&event)
        };
        println!("{}", output);
    }

    eprintln!("Connection closed by server");
    client
        .disconnect()
        .await?;

    Ok(())
}
