//! claudatui-daemon: Background process that owns PTY sessions.
//!
//! This daemon allows the TUI to restart (for hot reload) without losing
//! active Claude Code sessions.
//!
//! Usage:
//!   claudatui-daemon          # Run daemon (normally started by TUI)
//!   claudatui-daemon --status # Show running sessions
//!   claudatui-daemon --stop   # Graceful shutdown

use std::fs;
use std::io::{BufReader, BufWriter};
use std::os::unix::net::{UnixListener, UnixStream};
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

// Import from the library crate
use claudatui::daemon::protocol::{framing, Request, Response};
use claudatui::daemon::session_manager::SessionManager;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Handle command-line flags
    if args.len() > 1 {
        match args[1].as_str() {
            "--status" => return show_status(),
            "--stop" => return stop_daemon(),
            "--help" | "-h" => {
                println!("claudatui-daemon: Background process for PTY sessions");
                println!();
                println!("Usage:");
                println!("  claudatui-daemon          Run daemon (normally started by TUI)");
                println!("  claudatui-daemon --status Show running sessions");
                println!("  claudatui-daemon --stop   Graceful shutdown");
                return Ok(());
            }
            _ => {
                eprintln!("Unknown argument: {}", args[1]);
                std::process::exit(1);
            }
        }
    }

    // Run the daemon
    run_daemon()
}

fn socket_path() -> std::path::PathBuf {
    let uid = nix::unistd::getuid();
    std::path::PathBuf::from(format!("/tmp/claudatui-daemon-{}.sock", uid))
}

fn show_status() -> Result<()> {
    let sock_path = socket_path();

    let stream = UnixStream::connect(&sock_path).context("Daemon not running")?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let mut writer = BufWriter::new(&stream);
    framing::write_message(&mut writer, &Request::ListSessions)?;
    drop(writer);

    let mut reader = BufReader::new(&stream);
    let response: Response = framing::read_message(&mut reader)?;

    match response {
        Response::SessionList { sessions } => {
            if sessions.is_empty() {
                println!("Daemon running, no active sessions");
            } else {
                println!("Daemon running with {} session(s):", sessions.len());
                for session in sessions {
                    println!(
                        "  {} [{}x{}] {} {}",
                        session.session_id,
                        session.cols,
                        session.rows,
                        session.working_dir,
                        if session.is_alive { "alive" } else { "dead" }
                    );
                }
            }
        }
        Response::Error { message } => {
            eprintln!("Error: {}", message);
        }
        _ => {
            eprintln!("Unexpected response");
        }
    }

    Ok(())
}

fn stop_daemon() -> Result<()> {
    let sock_path = socket_path();

    let stream = match UnixStream::connect(&sock_path) {
        Ok(s) => s,
        Err(_) => {
            println!("Daemon not running");
            return Ok(());
        }
    };
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let mut writer = BufWriter::new(&stream);
    framing::write_message(&mut writer, &Request::Shutdown)?;
    drop(writer);

    let mut reader = BufReader::new(&stream);
    match framing::read_message::<_, Response>(&mut reader) {
        Ok(Response::ShuttingDown) => println!("Daemon shutting down"),
        Ok(Response::Error { message }) => eprintln!("Error: {}", message),
        Ok(_) => eprintln!("Unexpected response"),
        Err(_) => println!("Daemon shutting down"),
    }

    Ok(())
}

fn run_daemon() -> Result<()> {
    let sock_path = socket_path();

    // Remove stale socket if it exists
    if sock_path.exists() {
        // Check if there's already a daemon running
        if let Ok(_stream) = UnixStream::connect(&sock_path) {
            eprintln!("Daemon already running");
            std::process::exit(1);
        }
        // Stale socket, remove it
        fs::remove_file(&sock_path)?;
    }

    // Create the socket listener
    let listener = UnixListener::bind(&sock_path).context("Failed to bind socket")?;
    listener
        .set_nonblocking(true)
        .context("Failed to set non-blocking")?;

    // Session manager
    let mut session_manager = SessionManager::new();

    // Shutdown flag
    let shutdown = Arc::new(AtomicBool::new(false));

    eprintln!("claudatui-daemon started on {}", sock_path.display());

    // Main loop
    while !shutdown.load(Ordering::SeqCst) {
        // Accept new connections (non-blocking)
        match listener.accept() {
            Ok((stream, _)) => {
                // Handle client connection
                if let Err(e) = handle_client(&mut session_manager, stream, &shutdown) {
                    eprintln!("Client error: {}", e);
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // No pending connections, continue
            }
            Err(e) => {
                eprintln!("Accept error: {}", e);
            }
        }

        // Process PTY output for all sessions
        session_manager.process_all();

        // Clean up dead sessions
        let dead = session_manager.cleanup_dead_sessions();
        for id in dead {
            eprintln!("Session {} terminated", id);
        }

        // Small sleep to avoid busy-waiting
        thread::sleep(Duration::from_millis(10));
    }

    // Cleanup
    fs::remove_file(&sock_path).ok();
    eprintln!("claudatui-daemon stopped");

    Ok(())
}

fn handle_client(
    session_manager: &mut SessionManager,
    stream: UnixStream,
    shutdown: &Arc<AtomicBool>,
) -> Result<()> {
    // Ensure stream is in blocking mode (listener is non-blocking, but connections should block)
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    // Read request
    let mut reader = BufReader::new(&stream);
    let request: Request = framing::read_message(&mut reader)?;

    // Process request with panic catching for robustness
    let response = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        process_request(session_manager, request, shutdown)
    }))
    .unwrap_or_else(|e| {
        let msg = if let Some(s) = e.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = e.downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic".to_string()
        };
        Response::Error {
            message: format!("Internal error: {}", msg),
        }
    });

    // Write response
    let mut writer = BufWriter::new(&stream);
    framing::write_message(&mut writer, &response)?;

    Ok(())
}

fn process_request(
    session_manager: &mut SessionManager,
    request: Request,
    shutdown: &Arc<AtomicBool>,
) -> Response {
    match request {
        Request::Ping => Response::Pong,

        Request::Shutdown => {
            shutdown.store(true, Ordering::SeqCst);
            Response::ShuttingDown
        }

        Request::CreateSession(req) => match session_manager.create_session(req) {
            Ok(session_id) => Response::SessionCreated { session_id },
            Err(e) => Response::Error {
                message: e.to_string(),
            },
        },

        Request::CloseSession { session_id } => {
            if session_manager.close_session(&session_id) {
                Response::SessionClosed { session_id }
            } else {
                Response::Error {
                    message: "Session not found".to_string(),
                }
            }
        }

        Request::ListSessions => {
            let sessions = session_manager.list_sessions();
            Response::SessionList { sessions }
        }

        Request::WriteToSession { session_id, data } => {
            match session_manager.get_session_mut(&session_id) {
                Some(session) => match session.write(&data) {
                    Ok(_) => Response::WriteAck { session_id },
                    Err(e) => Response::Error {
                        message: e.to_string(),
                    },
                },
                None => Response::Error {
                    message: "Session not found".to_string(),
                },
            }
        }

        Request::GetSessionState { session_id } => {
            // Process output first to get latest state
            if let Some(session) = session_manager.get_session_mut(&session_id) {
                session.process_output();
            }

            match session_manager.get_session(&session_id) {
                Some(session) => Response::SessionState(session.state()),
                None => Response::Error {
                    message: "Session not found".to_string(),
                },
            }
        }

        Request::ResizeSession {
            session_id,
            rows,
            cols,
        } => match session_manager.get_session_mut(&session_id) {
            Some(session) => match session.resize(rows, cols) {
                Ok(_) => Response::Resized { session_id },
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            },
            None => Response::Error {
                message: "Session not found".to_string(),
            },
        },
    }
}
