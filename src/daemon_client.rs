//! Client for communicating with the claudatui daemon.
//!
//! Provides methods that match the App session API, backed by IPC to the daemon.
//!
//! The daemon uses a connection-per-request model: each request creates a new
//! Unix socket connection, sends the request, receives the response, then closes.
//! This is simpler and more robust than maintaining persistent connections.

#![allow(dead_code)]

use std::io::{BufReader, BufWriter, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::daemon::protocol::{
    framing, CreateSessionRequest, Request, Response, SessionId, SessionInfo, SessionState,
};

/// Get the socket path for the daemon.
pub fn socket_path() -> PathBuf {
    let uid = nix::unistd::getuid();
    PathBuf::from(format!("/tmp/claudatui-daemon-{}.sock", uid))
}

/// Client for communicating with the daemon.
///
/// This client creates a new connection for each request, matching the daemon's
/// connection-per-request model. The struct mainly exists to provide a clean API
/// and manage daemon lifecycle (starting it if needed).
pub struct DaemonClient {
    /// Cached socket path for efficiency
    sock_path: PathBuf,
}

impl DaemonClient {
    /// Connect to the daemon, starting it if necessary.
    ///
    /// This verifies the daemon is reachable but doesn't maintain a persistent connection.
    pub fn connect() -> Result<Self> {
        let sock_path = socket_path();

        // Try to verify daemon is running with a ping
        if Self::try_ping(&sock_path).is_ok() {
            return Ok(Self { sock_path });
        }

        // Spawn the daemon
        Self::spawn_daemon()?;

        // Wait for daemon to start
        for i in 0..50 {
            std::thread::sleep(Duration::from_millis(100));
            if Self::try_ping(&sock_path).is_ok() {
                return Ok(Self { sock_path });
            }

            if i == 49 {
                anyhow::bail!("Daemon failed to start after 5 seconds");
            }
        }

        anyhow::bail!("Failed to connect to daemon");
    }

    /// Try to ping the daemon at the given socket path.
    fn try_ping(sock_path: &PathBuf) -> Result<()> {
        let stream = UnixStream::connect(sock_path)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        let mut writer = BufWriter::new(&stream);
        framing::write_message(&mut writer, &Request::Ping)?;
        writer.flush()?;
        drop(writer);

        let mut reader = BufReader::new(&stream);
        let response: Response = framing::read_message(&mut reader)?;

        match response {
            Response::Pong => Ok(()),
            Response::Error { message } => anyhow::bail!("Ping failed: {}", message),
            _ => anyhow::bail!("Unexpected response to ping"),
        }
    }

    /// Spawn the daemon process.
    fn spawn_daemon() -> Result<()> {
        // Get the path to the daemon binary
        let daemon_path = std::env::current_exe()?
            .parent()
            .context("No parent directory for executable")?
            .join("claudatui-daemon");

        // Spawn daemon in background
        Command::new(&daemon_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("Failed to spawn daemon: {:?}", daemon_path))?;

        Ok(())
    }

    /// Send a request and receive a response (creates a new connection).
    fn request(&self, req: Request) -> Result<Response> {
        let stream = UnixStream::connect(&self.sock_path)
            .context("Failed to connect to daemon - it may have stopped")?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        let mut writer = BufWriter::new(&stream);
        framing::write_message(&mut writer, &req)?;
        writer.flush()?;
        drop(writer);

        let mut reader = BufReader::new(&stream);
        let response: Response = framing::read_message(&mut reader)?;
        Ok(response)
    }

    /// Ping the daemon.
    pub fn ping(&self) -> Result<()> {
        match self.request(Request::Ping)? {
            Response::Pong => Ok(()),
            Response::Error { message } => anyhow::bail!("Ping failed: {}", message),
            _ => anyhow::bail!("Unexpected response to ping"),
        }
    }

    /// Check if the daemon connection is still alive.
    pub fn is_connected(&self) -> bool {
        self.ping().is_ok()
    }

    /// Create a new session.
    pub fn create_session(
        &self,
        working_dir: &str,
        resume_session_id: Option<&str>,
        rows: u16,
        cols: u16,
    ) -> Result<SessionId> {
        let req = Request::CreateSession(CreateSessionRequest {
            working_dir: working_dir.to_string(),
            resume_session_id: resume_session_id.map(|s| s.to_string()),
            rows,
            cols,
        });

        match self.request(req)? {
            Response::SessionCreated { session_id } => Ok(session_id),
            Response::Error { message } => anyhow::bail!("Create session failed: {}", message),
            _ => anyhow::bail!("Unexpected response to create session"),
        }
    }

    /// Close a session.
    pub fn close_session(&self, session_id: &str) -> Result<()> {
        let req = Request::CloseSession {
            session_id: session_id.to_string(),
        };

        match self.request(req)? {
            Response::SessionClosed { .. } => Ok(()),
            Response::Error { message } => anyhow::bail!("Close session failed: {}", message),
            _ => anyhow::bail!("Unexpected response to close session"),
        }
    }

    /// List all sessions.
    pub fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        match self.request(Request::ListSessions)? {
            Response::SessionList { sessions } => Ok(sessions),
            Response::Error { message } => anyhow::bail!("List sessions failed: {}", message),
            _ => anyhow::bail!("Unexpected response to list sessions"),
        }
    }

    /// Write input to a session.
    pub fn write_to_session(&self, session_id: &str, data: &[u8]) -> Result<()> {
        let req = Request::WriteToSession {
            session_id: session_id.to_string(),
            data: data.to_vec(),
        };

        match self.request(req)? {
            Response::WriteAck { .. } => Ok(()),
            Response::Error { message } => anyhow::bail!("Write failed: {}", message),
            _ => anyhow::bail!("Unexpected response to write"),
        }
    }

    /// Get session state.
    pub fn get_session_state(&self, session_id: &str) -> Result<SessionState> {
        let req = Request::GetSessionState {
            session_id: session_id.to_string(),
        };

        match self.request(req)? {
            Response::SessionState(state) => Ok(state),
            Response::Error { message } => anyhow::bail!("Get state failed: {}", message),
            _ => anyhow::bail!("Unexpected response to get state"),
        }
    }

    /// Resize a session.
    pub fn resize_session(&self, session_id: &str, rows: u16, cols: u16) -> Result<()> {
        let req = Request::ResizeSession {
            session_id: session_id.to_string(),
            rows,
            cols,
        };

        match self.request(req)? {
            Response::Resized { .. } => Ok(()),
            Response::Error { message } => anyhow::bail!("Resize failed: {}", message),
            _ => anyhow::bail!("Unexpected response to resize"),
        }
    }

    /// Shutdown the daemon.
    pub fn shutdown(&self) -> Result<()> {
        match self.request(Request::Shutdown)? {
            Response::ShuttingDown => Ok(()),
            Response::Error { message } => anyhow::bail!("Shutdown failed: {}", message),
            _ => anyhow::bail!("Unexpected response to shutdown"),
        }
    }
}

/// Check if the daemon is running.
pub fn is_daemon_running() -> bool {
    DaemonClient::try_ping(&socket_path()).is_ok()
}

/// Stop the daemon if running.
pub fn stop_daemon() -> Result<()> {
    if is_daemon_running() {
        let client = DaemonClient::connect()?;
        client.shutdown()?;
    }
    Ok(())
}
