//! fs_cli-rs: Interactive FreeSWITCH CLI client using ESL
//!
//! A modern Rust-based FreeSWITCH CLI client with readline capabilities,
//! command history, and comprehensive logging.

use anyhow::{Context, Result};
use clap::Parser;
use colored::*;
use freeswitch_esl_rs::{EslEventType, EslHandle, EventFormat};
use rustyline::history::FileHistory;
use rustyline::{Cmd, Editor, ExternalPrinter, KeyCode, KeyEvent, Modifiers};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::{timeout, Duration};
use tracing::{error, info, warn};

mod commands;
mod completion;
mod esl_debug;
mod log_display;

use commands::{CommandProcessor, LogLevel};
use completion::FsCliCompleter;
use esl_debug::EslDebugLevel;
use log_display::LogDisplay;

/// Default FreeSWITCH function key bindings
fn get_default_fnkeys() -> Vec<&'static str> {
    vec![
        "help",                          // F1
        "status",                        // F2
        "show channels",                 // F3
        "show calls",                    // F4
        "sofia status",                  // F5
        "reloadxml",                     // F6
        "/log console",                  // F7
        "/log debug",                    // F8
        "sofia status profile internal", // F9
        "fsctl pause",                   // F10
        "fsctl resume",                  // F11
        "version",                       // F12
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
#[derive(Parser, Debug, Clone)]
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

    /// ESL debug level (0-7, higher = more verbose)
    #[arg(short, long, default_value_t = EslDebugLevel::None)]
    debug: EslDebugLevel,

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

    /// Log level for FreeSWITCH logs
    #[arg(short = 'l', long, default_value = "debug")]
    log_level: LogLevel,

    /// Disable automatic log subscription on startup
    #[arg(long)]
    quiet: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    setup_logging(args.debug)?;

    // Connect to FreeSWITCH
    args.debug
        .debug_print(EslDebugLevel::Debug, "About to connect to FreeSWITCH");
    let mut handle = match connect_to_freeswitch(&args).await {
        Ok(handle) => {
            args.debug
                .debug_print(EslDebugLevel::Debug, "Successfully connected to FreeSWITCH");
            handle
        }
        Err(e) => {
            eprintln!(
                "Failed to connect to FreeSWITCH at {}:{}",
                args.host, args.port
            );
            if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
                match io_err.kind() {
                    std::io::ErrorKind::ConnectionRefused => {
                        eprintln!(
                            "Connection refused - is FreeSWITCH running and listening on port {}?",
                            args.port
                        );
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
        args.debug
            .debug_print(EslDebugLevel::Debug, "Subscribing to events");
        subscribe_to_events(&mut handle).await?;
    }

    // Enable logging if not quiet
    if !args.quiet {
        args.debug.debug_print(
            EslDebugLevel::Debug,
            &format!("Enabling logging at level: {}", args.log_level.as_str()),
        );
        enable_logging(&mut handle, args.log_level).await?;
    }

    // Execute single command or start interactive mode
    if let Some(ref command) = args.execute {
        execute_single_command(&mut handle, command, &args).await?;
        // Clean disconnect
        info!("Disconnecting from FreeSWITCH...");
        handle.disconnect().await?;
    } else {
        run_interactive_mode(handle, &args).await?;
        // Handle is consumed by run_interactive_mode, no need to disconnect
    }

    Ok(())
}

/// Set up logging based on debug level
fn setup_logging(debug_level: EslDebugLevel) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(debug_level.tracing_filter())
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

/// Enable logging at the specified level
async fn enable_logging(handle: &mut EslHandle, log_level: LogLevel) -> Result<()> {
    info!("Enabling logging at level: {}", log_level.as_str());

    use freeswitch_esl_rs::command::EslCommand;

    let cmd = if log_level == LogLevel::NoLog {
        EslCommand::Api {
            command: "nolog".to_string(),
        }
    } else {
        EslCommand::Log {
            level: log_level.as_str().to_string(),
        }
    };

    let response = handle.send_command(cmd).await?;

    if !response.is_success() {
        if let Some(reply) = response.reply_text() {
            warn!("Failed to set log level: {}", reply);
        }
    }

    Ok(())
}

/// Execute a single command and exit
async fn execute_single_command(handle: &mut EslHandle, command: &str, args: &Args) -> Result<()> {
    let processor = CommandProcessor::new(args.no_color, args.debug);
    processor.execute_command(handle, command).await?;
    Ok(())
}

/// Run the readline loop in a blocking thread
fn run_readline_loop(
    cmd_tx: mpsc::UnboundedSender<String>,
    quit_tx: oneshot::Sender<()>,
    printer_tx: oneshot::Sender<Arc<Mutex<dyn ExternalPrinter + Send>>>,
    args: &Args,
) -> Result<()> {
    // Set up readline editor
    let mut rl = Editor::<FsCliCompleter, FileHistory>::new()?;

    // Create completer and provide ESL handle
    let completer = FsCliCompleter::new();
    rl.set_helper(Some(completer));

    // Set up function key bindings
    setup_function_key_bindings(&mut rl)?;

    // Create external printer for background log output
    let printer = rl.create_external_printer()?;
    let printer_arc = Arc::new(Mutex::new(printer));

    // Send printer to main thread
    let _ = printer_tx.send(printer_arc);

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

    // Readline loop
    loop {
        let prompt = format!("freeswitch@{}> ", args.host);

        match rl.readline(&prompt) {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                // Add to history
                let _ = rl.add_history_entry(line);

                // Handle quit commands
                if matches!(line, "/quit" | "/exit" | "/bye") {
                    println!("Goodbye!");
                    let _ = quit_tx.send(());
                    break;
                }

                // Handle history command locally (since we have access to rl here)
                if line == "history" {
                    println!("Command History:");
                    let history = rl.history();
                    for (i, entry) in history
                        .iter()
                        .enumerate()
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .take(20)
                    {
                        println!("  {}: {}", i + 1, entry);
                    }
                    continue;
                }

                // Send command to main thread
                if cmd_tx.send(line.to_string()).is_err() {
                    break; // Main thread has closed
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(rustyline::error::ReadlineError::Eof) => {
                println!("Goodbye!");
                let _ = quit_tx.send(());
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

/// Run interactive CLI mode
async fn run_interactive_mode(handle: EslHandle, args: &Args) -> Result<()> {
    let mut processor = CommandProcessor::new(args.no_color, args.debug);

    // Create channels for communication between rustyline thread and main async thread
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<String>();
    let (quit_tx, mut quit_rx) = oneshot::channel::<()>();
    let (printer_tx, printer_rx) = oneshot::channel::<Arc<Mutex<dyn ExternalPrinter + Send>>>();

    println!("FreeSWITCH CLI ready. Type 'help' for commands, '/quit' to exit.\n");

    // Spawn rustyline in a blocking thread
    let args_clone = args.clone();
    let readline_handle = tokio::task::spawn_blocking(move || {
        run_readline_loop(cmd_tx, quit_tx, printer_tx, &args_clone)
    });

    // Wait for external printer to be ready
    let external_printer = match printer_rx.await {
        Ok(printer) => Some(printer),
        Err(_) => {
            error!("Failed to receive external printer");
            None
        }
    };

    // Set the external printer on the command processor
    processor.set_printer(external_printer.clone());

    // Wrap handle in Arc<Mutex> for sharing between tasks
    let handle_arc = Arc::new(Mutex::new(handle));
    let log_handle = if !args.quiet {
        let handle_clone = handle_arc.clone();
        let no_color = args.no_color;
        let printer_clone = external_printer.clone();
        Some(tokio::spawn(async move {
            loop {
                {
                    let mut h = handle_clone.lock().await;
                    if let Err(e) =
                        LogDisplay::check_and_display_logs(&mut h, no_color, printer_clone.clone())
                            .await
                    {
                        warn!("Error in background log monitoring: {}", e);
                    }
                }
                // Small delay to prevent excessive CPU usage
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }))
    } else {
        None
    };

    // Main command processing loop
    loop {
        tokio::select! {
            // Handle commands from readline thread
            Some(command) = cmd_rx.recv() => {
                let mut handle = handle_arc.lock().await;

                // Handle client-side commands first (start with /)
                if command.starts_with('/') {
                    match command.as_str() {
                        "/help" => {
                            processor.show_help().await;
                            continue;
                        }
                        _ => {
                            // Let the command processor handle other /commands
                            if let Err(e) = processor.execute_command(&mut handle, &command).await {
                                processor.handle_error(e).await;
                            }
                            continue;
                        }
                    }
                }

                // Handle other built-in commands
                match command.as_str() {
                    "clear" => {
                        // Use external printer for clear screen sequence
                        if let Some(printer_arc) = &external_printer {
                            if let Ok(mut p) = printer_arc.try_lock() {
                                let _ = p.print("\x1B[2J\x1B[1;1H".to_string());
                            } else {
                                print!("\x1B[2J\x1B[1;1H");
                            }
                        } else {
                            print!("\x1B[2J\x1B[1;1H");
                        }
                        continue;
                    }
                    "help" => {
                        processor.show_help().await;
                        continue;
                    }
                    _ => {
                        // Check for function key shortcuts (F1-F12) typed manually
                        if let Some(fn_command) = parse_function_key(&command) {
                            if let Err(e) = processor.execute_command(&mut handle, fn_command).await {
                                processor.handle_error(e).await;
                            }
                            continue;
                        }

                        // Execute FreeSWITCH command and show output immediately
                        if let Err(e) = processor.execute_command(&mut handle, &command).await {
                            processor.handle_error(e).await;
                        }
                    }
                }
            }
            // Handle quit signal from readline thread
            _ = &mut quit_rx => {
                break;
            }
        }
    }

    // Clean up background tasks
    if let Some(handle) = log_handle {
        handle.abort();
    }

    // Wait for readline thread to finish
    if let Err(e) = readline_handle.await {
        warn!("Error waiting for readline thread: {}", e);
    }

    // Clean disconnect
    info!("Disconnecting from FreeSWITCH...");
    let mut handle = Arc::try_unwrap(handle_arc)
        .map_err(|_| anyhow::anyhow!("Failed to unwrap handle"))?
        .into_inner();
    handle.disconnect().await?;

    Ok(())
}
