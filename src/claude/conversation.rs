use chrono::{DateTime, Utc};
use std::path::PathBuf;

/// A Claude conversation
#[derive(Debug, Clone)]
pub struct Conversation {
    /// Session ID
    pub session_id: String,
    /// Display text (first user message, truncated)
    pub display: String,
    /// AI-generated summary (if available)
    pub summary: Option<String>,
    /// Timestamp in milliseconds (file_mtime for sorting)
    pub timestamp: i64,
    /// ISO 8601 modified timestamp
    pub modified: String,
    /// Project path
    pub project_path: PathBuf,
    /// Number of messages in the conversation
    pub message_count: u32,
    /// Git branch (if in a git repo)
    pub git_branch: Option<String>,
    /// Whether this is a plan implementation conversation (hidden from sidebar)
    pub is_plan_implementation: bool,
    /// Whether this conversation is archived
    pub is_archived: bool,
    /// When this conversation was archived (if archived)
    pub archived_at: Option<DateTime<Utc>>,
}
