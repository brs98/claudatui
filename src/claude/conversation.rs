use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Status of a conversation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConversationStatus {
    /// Claude is currently processing
    Active,
    /// Claude finished, waiting for user input
    WaitingForInput,
    /// Not currently running
    #[default]
    Idle,
}

/// A Claude conversation
#[derive(Debug, Clone)]
pub struct Conversation {
    /// Session ID
    pub session_id: String,
    /// Display text (first user message, truncated)
    pub display: String,
    /// AI-generated summary (if available)
    #[allow(dead_code)]
    pub summary: Option<String>,
    /// Timestamp in milliseconds (file_mtime for sorting)
    pub timestamp: i64,
    /// ISO 8601 modified timestamp
    #[allow(dead_code)]
    pub modified: String,
    /// Project path
    pub project_path: PathBuf,
    /// Current status
    pub status: ConversationStatus,
    /// Number of messages in the conversation
    #[allow(dead_code)]
    pub message_count: u32,
    /// Git branch (if in a git repo)
    #[allow(dead_code)]
    pub git_branch: Option<String>,
    /// Whether this is a plan implementation conversation (hidden from sidebar)
    pub is_plan_implementation: bool,
}

/// Message from a conversation JSONL file
#[derive(Debug, Deserialize)]
struct ConversationMessage {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    msg_type: String,
    message: Option<MessageContent>,
}

#[derive(Debug, Deserialize)]
struct MessageContent {
    role: Option<String>,
    content: Option<serde_json::Value>,
    #[serde(rename = "stop_reason")]
    stop_reason: Option<String>,
}

/// Parse a conversation JSONL file to extract display text and status
/// Note: This is kept for reference but no longer used.
/// We now get display text from sessions-index.json (see sessions.rs).
#[allow(dead_code)]
pub fn parse_conversation(path: &Path) -> Result<(String, ConversationStatus)> {
    if !path.exists() {
        return Ok(("(No messages)".to_string(), ConversationStatus::Idle));
    }

    let file =
        File::open(path).with_context(|| format!("Failed to open conversation: {:?}", path))?;
    let reader = BufReader::new(file);

    let mut first_user_message: Option<String> = None;
    let mut last_message_role: Option<String> = None;
    let mut last_stop_reason: Option<String> = None;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        if line.trim().is_empty() {
            continue;
        }

        let msg: ConversationMessage = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if let Some(ref content) = msg.message {
            if let Some(ref role) = content.role {
                last_message_role = Some(role.clone());
                last_stop_reason = content.stop_reason.clone();

                // Extract first user message for display
                if first_user_message.is_none() && role == "user" {
                    if let Some(ref c) = content.content {
                        first_user_message = Some(extract_text_content(c));
                    }
                }
            }
        }
    }

    let display = first_user_message.unwrap_or_else(|| "(No messages)".to_string());
    let status = detect_status(&last_message_role, &last_stop_reason);

    Ok((display, status))
}

/// Parse only the last few lines of a conversation to detect status efficiently
#[allow(dead_code)]
pub fn detect_status_fast(path: &Path) -> Result<ConversationStatus> {
    if !path.exists() {
        return Ok(ConversationStatus::Idle);
    }

    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();

    // Read the last 8KB to find the last message
    let read_from = file_size.saturating_sub(8192);
    file.seek(SeekFrom::Start(read_from))?;

    let reader = BufReader::new(file);
    let mut last_message_role: Option<String> = None;
    let mut last_stop_reason: Option<String> = None;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        if line.trim().is_empty() {
            continue;
        }

        let msg: ConversationMessage = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if let Some(ref content) = msg.message {
            if let Some(ref role) = content.role {
                last_message_role = Some(role.clone());
                last_stop_reason = content.stop_reason.clone();
            }
        }
    }

    Ok(detect_status(&last_message_role, &last_stop_reason))
}

fn detect_status(last_role: &Option<String>, stop_reason: &Option<String>) -> ConversationStatus {
    match last_role.as_deref() {
        Some("assistant") => {
            if stop_reason.as_deref() == Some("end_turn") {
                ConversationStatus::WaitingForInput
            } else {
                // stop_reason is null - could be streaming or interrupted
                // Default to Idle since we can't detect if Claude is actually running
                ConversationStatus::Idle
            }
        }
        // User sent a message but no assistant response yet - waiting for Claude
        Some("user") => ConversationStatus::WaitingForInput,
        _ => ConversationStatus::Idle,
    }
}

fn extract_text_content(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => {
            // Handle array of content blocks
            for item in arr {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    return text.to_string();
                }
            }
            "(Complex content)".to_string()
        }
        _ => "(Complex content)".to_string(),
    }
}
