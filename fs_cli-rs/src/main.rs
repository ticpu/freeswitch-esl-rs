//! fs_cli-rs: Interactive FreeSWITCH CLI client using ESL
//!
//! A modern Rust-based FreeSWITCH CLI client with readline capabilities,
//! command history, and comprehensive logging.

use anyhow::{Context, Result};
use clap::Parser;
use colored::*;
use freeswitch_esl_rs::{EslEventType, EslHandle, EventFormat};
use rustyline::history::FileHistory;
use rustyline::{Editor, KeyEvent, KeyCode, Cmd, Modifiers};
use std::path::PathBuf;
use tokio::time::{timeout, Duration};
use tracing::{error, info, warn};

mod commands;
mod completion;

use commands::CommandProcessor;
use completion::FsCliCompleter;

/// Default FreeSWITCH function key bindings
fn get_default_fnkeys() -> Vec<&'static str> {
    vec![
        "help",                           // F1
        "status",                         // F2
        "show channels",                  // F3
        "show calls",                     // F4
        "sofia status",                   // F5
        "reloadxml",                      // F6
        "/log console",                   // F7
        "/log debug",                     // F8
        "sofia status profile internal",  // F9
        "fsctl pause",                    // F10
        "fsctl resume",                   // F11
        "version",                        // F12
    ]
}

/// Parse function key shortcuts (F1-F12)
fn parse_function_key(input: &str) -> Option<&'static str> {
    let fnkeys = get_default_fnkeys();
    
    match input.to_lowercase().as_str() {
        "f1" => Some(fnkeys[0]),
        "f2" => Some(fnkeys[1]),
        "f3" => Some(fnkeys[2]),
        "f4" => Some(fnkeys[3]),
        "f5" => Some(fnkeys[4]),
        "f6" => Some(fnkeys[5]),
        "f7" => Some(fnkeys[6]),
        "f8" => Some(fnkeys[7]),
        "f9" => Some(fnkeys[8]),
        "f10" => Some(fnkeys[9]),
        "f11" => Some(fnkeys[10]),
        "f12" => Some(fnkeys[11]),
        _ => None,
    }
}


/// Set up function key bindings for readline  
fn setup_function_key_bindings(rl: &mut Editor<FsCliCompleter, FileHistory>) -> Result<()> {
    let fnkeys = get_default_fnkeys();
    
    // Bind F1-F12 to Cmd::Macro for automatic execution
    for (i, &command) in fnkeys.iter().enumerate() {
        let f_key = KeyEvent(KeyCode::F((i + 1) as u8), Modifiers::NONE);
        // Use Cmd::Macro to replay the command + newline (which triggers AcceptLine)
        rl.bind_sequence(f_key, Cmd::Macro(format!("{}\n", command)));
    }
    
    Ok(())
}

/// Interactive FreeSWITCH CLI client
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// FreeSWITCH hostname or IP address
    #[arg(short = 'H', long, default_value = "localhost")]
    host: String,

    /// FreeSWITCH ESL port
    #[arg(short = 'P', long, default_value_t = 8021)]
    port: u16,

    /// ESL password
    #[arg(short = 'p', long, default_value = "ClueCon")]
    password: String,

    /// Username for authentication (optional)
    #[arg(short, long)]
    user: Option<String>,

    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,

    /// Disable colored output
    #[arg(long)]
    no_color: bool,

    /// Execute single command and exit
    #[arg(short = 'x')]
    execute: Option<String>,

    /// History file path
    #[arg(long)]
    history_file: Option<PathBuf>,

    /// Connection timeout in seconds
    #[arg(short, long, default_value_t = 10)]
    timeout: u64,

    /// Subscribe to events on startup
    #[arg(long)]
    events: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    setup_logging(args.debug)?;

    // Connect to FreeSWITCH
    let mut handle = match connect_to_freeswitch(&args).await {
        Ok(handle) => handle,
        Err(e) => {
            eprintln!("Failed to connect to FreeSWITCH at {}:{}", args.host, args.port);
            if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
                match io_err.kind() {
                    std::io::ErrorKind::ConnectionRefused => {
                        eprintln!("Connection refused - is FreeSWITCH running and listening on port {}?", args.port);
                    }
                    std::io::ErrorKind::TimedOut => {
                        eprintln!("Connection timed out after {} seconds", args.timeout);
                    }
                    _ => {
                        eprintln!("Connection error: {}", io_err);
                    }
                }
            } else {
                eprintln!("Error: {}", e);
            }
            std::process::exit(1);
        }
    };

    // Subscribe to events if requested
    if args.events {
        subscribe_to_events(&mut handle).await?;
    }

    // Execute single command or start interactive mode
    if let Some(ref command) = args.execute {
        execute_single_command(&mut handle, command, &args).await?;
    } else {
        run_interactive_mode(&mut handle, &args).await?;
    }

    // Clean disconnect
    info!("Disconnecting from FreeSWITCH...");
    handle.disconnect().await?;

    Ok(())
}

/// Set up logging based on debug level
fn setup_logging(debug: bool) -> Result<()> {
    let filter = if debug { "debug" } else { "error" };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .init();

    Ok(())
}


/// Connect to FreeSWITCH with timeout
async fn connect_to_freeswitch(args: &Args) -> Result<EslHandle> {
    info!("Connecting to FreeSWITCH at {}:{}", args.host, args.port);

    let connect_result = if let Some(ref user) = args.user {
        info!("Using user authentication: {}", user);
        timeout(
            Duration::from_secs(args.timeout),
            EslHandle::connect_with_user(&args.host, args.port, user, &args.password),
        )
        .await
    } else {
        info!("Using password authentication");
        timeout(
            Duration::from_secs(args.timeout),
            EslHandle::connect(&args.host, args.port, &args.password),
        )
        .await
    };

    let handle = connect_result
        .context("Connection timed out")?
        .context("Failed to connect to FreeSWITCH")?;

    if !args.no_color {
        println!("{}", "✓ Connected successfully".green());
    } else {
        println!("Connected successfully");
    }

    Ok(handle)
}

/// Subscribe to events for monitoring
async fn subscribe_to_events(handle: &mut EslHandle) -> Result<()> {
    info!("Subscribing to events...");

    handle
        .subscribe_events(
            EventFormat::Plain,
            &[
                EslEventType::ChannelCreate,
                EslEventType::ChannelAnswer,
                EslEventType::ChannelHangup,
                EslEventType::Heartbeat,
            ],
        )
        .await?;

    println!("Event monitoring enabled");
    Ok(())
}

/// Execute a single command and exit
async fn execute_single_command(handle: &mut EslHandle, command: &str, args: &Args) -> Result<()> {
    let processor = CommandProcessor::new(args.no_color);
    processor.execute_command(handle, command).await?;
    Ok(())
}

/// Run interactive CLI mode
async fn run_interactive_mode(handle: &mut EslHandle, args: &Args) -> Result<()> {
    // Set up readline editor
    let mut rl = Editor::<FsCliCompleter, FileHistory>::new()?;
    
    // Create completer and provide ESL handle
    let completer = FsCliCompleter::new();
    rl.set_helper(Some(completer));
    
    // Set up function key bindings
    setup_function_key_bindings(&mut rl)?;
    

    // Load history
    let history_file = args.history_file.clone().unwrap_or_else(|| {
        let mut path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push(".fs_cli_history");
        path
    });

    if history_file.exists() {
        if let Err(e) = rl.load_history(&history_file) {
            warn!("Could not load history: {}", e);
        }
    }

    let processor = CommandProcessor::new(args.no_color);

    println!("FreeSWITCH CLI ready. Type 'help' for commands, 'quit' to exit.\n");

    // Main interactive REPL loop
    loop {
        // Create prompt
        let prompt = format!("freeswitch@{}> ", args.host);

        // Get user input using readline
        match rl.readline(&prompt) {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                // Add to history
                let _ = rl.add_history_entry(line);

                // Handle built-in commands first
                match line {
                    "quit" | "exit" | "bye" => {
                        println!("Goodbye!");
                        break;
                    }
                    "clear" => {
                        print!("\x1B[2J\x1B[1;1H");
                        continue;
                    }
                    "history" => {
                        processor.show_history(&rl);
                        continue;
                    }
                    "help" => {
                        processor.show_help();
                        continue;
                    }
                    _ => {
                        // Check for function key shortcuts (F1-F12) typed manually
                        if let Some(fn_command) = parse_function_key(line) {
                            if let Err(e) = processor.execute_command(handle, fn_command).await {
                                if !args.no_color {
                                    eprintln!("{}: {}", "Error".red().bold(), e);
                                } else {
                                    eprintln!("Error: {}", e);
                                }
                            }
                            continue;
                        }
                        
                        // Function key commands are automatically executed (no special handling needed)
                        // since they're inserted directly by the key binding system
                        
                        // Execute FreeSWITCH command and show output immediately
                        if let Err(e) = processor.execute_command(handle, line).await {
                            if !args.no_color {
                                eprintln!("{}: {}", "Error".red().bold(), e);
                            } else {
                                eprintln!("Error: {}", e);
                            }
                        }
                    }
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(rustyline::error::ReadlineError::Eof) => {
                println!("Goodbye!");
                break;
            }
            Err(e) => {
                error!("Error reading input: {}", e);
                break;
            }
        }
    }

    // Save history
    if let Err(e) = rl.save_history(&history_file) {
        warn!("Could not save history: {}", e);
    }

    Ok(())
}
