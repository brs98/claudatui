use anyhow::Result;
use chrono::{DateTime, Utc};
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
    pub summary: Option<String>,
    /// Timestamp in milliseconds (file_mtime for sorting)
    pub timestamp: i64,
    /// ISO 8601 modified timestamp
    pub modified: String,
    /// Project path
    pub project_path: PathBuf,
    /// Current status
    pub status: ConversationStatus,
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

/// Entry from a conversation JSONL file (top-level structure)
#[derive(Debug, Deserialize)]
struct JournalEntry {
    /// Entry type: "assistant", "user", "system", "summary", "progress"
    #[serde(rename = "type")]
    entry_type: String,
    /// Subtype for system entries (e.g., "turn_duration")
    subtype: Option<String>,
    /// Present on "user" entries that are tool-use approval/rejection results,
    /// not real user messages
    #[serde(rename = "toolUseResult")]
    tool_use_result: Option<serde_json::Value>,
}

/// Parse only the last few lines of a conversation to detect status efficiently.
///
/// Uses the entry-level `type` field (not `message.stop_reason`, which is always None
/// in modern transcripts). Skips `progress` entries (tool output noise) to find the
/// last meaningful entry.
pub fn detect_status_fast(path: &Path) -> Result<ConversationStatus> {
    if !path.exists() {
        return Ok(ConversationStatus::Idle);
    }

    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();

    // Read the last 8KB to find the last meaningful entry
    let read_from = file_size.saturating_sub(8192);
    file.seek(SeekFrom::Start(read_from))?;

    let reader = BufReader::new(file);
    let mut last_entry_type: Option<String> = None;
    let mut last_subtype: Option<String> = None;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        if line.trim().is_empty() {
            continue;
        }

        let entry: JournalEntry = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Skip noise entries that don't indicate conversation state
        if matches!(
            entry.entry_type.as_str(),
            "progress" | "file-history-snapshot" | "pr-link" | "queue-operation"
        ) {
            continue;
        }

        // Skip system entries that aren't turn completion markers
        if entry.entry_type == "system" && entry.subtype.as_deref() != Some("turn_duration") {
            continue;
        }

        // Skip "user" entries that are tool-use results (approval/rejection),
        // not real user messages — these are tool-loop noise
        if entry.entry_type == "user" && entry.tool_use_result.is_some() {
            continue;
        }

        last_subtype = entry.subtype;
        last_entry_type = Some(entry.entry_type);
    }

    Ok(match last_entry_type.as_deref() {
        // Turn completed: Claude finished and is waiting for user input
        Some("summary") => ConversationStatus::WaitingForInput,
        Some("system") if last_subtype.as_deref() == Some("turn_duration") => {
            ConversationStatus::WaitingForInput
        }
        // User just sent a message — Claude should be processing
        Some("user") => ConversationStatus::Active,
        // Claude wrote a response — ball is with the user (or a tool is executing)
        Some("assistant") => ConversationStatus::WaitingForInput,
        _ => ConversationStatus::Idle,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_jsonl(path: &Path, lines: &[&str]) {
        let mut f = File::create(path).unwrap();
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
    }

    #[test]
    fn summary_entry_returns_waiting_for_input() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.jsonl");
        write_jsonl(&p, &[
            r#"{"type":"user"}"#,
            r#"{"type":"assistant"}"#,
            r#"{"type":"system","subtype":"turn_duration"}"#,
            r#"{"type":"summary"}"#,
        ]);
        assert_eq!(detect_status_fast(&p).unwrap(), ConversationStatus::WaitingForInput);
    }

    #[test]
    fn turn_duration_entry_returns_waiting_for_input() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.jsonl");
        write_jsonl(&p, &[
            r#"{"type":"user"}"#,
            r#"{"type":"assistant"}"#,
            r#"{"type":"system","subtype":"turn_duration"}"#,
        ]);
        assert_eq!(detect_status_fast(&p).unwrap(), ConversationStatus::WaitingForInput);
    }

    #[test]
    fn user_entry_returns_active() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.jsonl");
        write_jsonl(&p, &[
            r#"{"type":"assistant"}"#,
            r#"{"type":"system","subtype":"turn_duration"}"#,
            r#"{"type":"user"}"#,
        ]);
        assert_eq!(detect_status_fast(&p).unwrap(), ConversationStatus::Active);
    }

    #[test]
    fn assistant_entry_returns_waiting_for_input() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.jsonl");
        write_jsonl(&p, &[
            r#"{"type":"user"}"#,
            r#"{"type":"assistant"}"#,
        ]);
        assert_eq!(detect_status_fast(&p).unwrap(), ConversationStatus::WaitingForInput);
    }

    #[test]
    fn progress_entries_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.jsonl");
        write_jsonl(&p, &[
            r#"{"type":"user"}"#,
            r#"{"type":"assistant"}"#,
            r#"{"type":"progress","data":{"tool":"bash"}}"#,
            r#"{"type":"progress","data":{"tool":"bash"}}"#,
        ]);
        // Last non-progress entry is "assistant" → WaitingForInput
        assert_eq!(detect_status_fast(&p).unwrap(), ConversationStatus::WaitingForInput);
    }

    #[test]
    fn empty_file_returns_idle() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.jsonl");
        std::fs::write(&p, "").unwrap();
        assert_eq!(detect_status_fast(&p).unwrap(), ConversationStatus::Idle);
    }

    #[test]
    fn nonexistent_file_returns_idle() {
        let p = Path::new("/tmp/claudatui_nonexistent_test.jsonl");
        assert_eq!(detect_status_fast(p).unwrap(), ConversationStatus::Idle);
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.jsonl");
        write_jsonl(&p, &[
            r#"{"type":"user"}"#,
            "this is not json",
            r#"{"type":"summary"}"#,
        ]);
        assert_eq!(detect_status_fast(&p).unwrap(), ConversationStatus::WaitingForInput);
    }

    #[test]
    fn metadata_entries_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.jsonl");
        write_jsonl(&p, &[
            r#"{"type":"user"}"#,
            r#"{"type":"assistant"}"#,
            r#"{"type":"system","subtype":"turn_duration"}"#,
            r#"{"type":"file-history-snapshot"}"#,
            r#"{"type":"pr-link"}"#,
        ]);
        // file-history-snapshot and pr-link are skipped → turn_duration is last → WaitingForInput
        assert_eq!(detect_status_fast(&p).unwrap(), ConversationStatus::WaitingForInput);
    }

    #[test]
    fn only_metadata_entries_returns_idle() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.jsonl");
        write_jsonl(&p, &[
            r#"{"type":"file-history-snapshot"}"#,
        ]);
        // Brand new session with only a file snapshot → Idle
        assert_eq!(detect_status_fast(&p).unwrap(), ConversationStatus::Idle);
    }

    #[test]
    fn queue_operation_entries_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.jsonl");
        write_jsonl(&p, &[
            r#"{"type":"user"}"#,
            r#"{"type":"assistant"}"#,
            r#"{"type":"queue-operation"}"#,
        ]);
        // queue-operation is skipped → assistant is last → WaitingForInput
        assert_eq!(detect_status_fast(&p).unwrap(), ConversationStatus::WaitingForInput);
    }

    #[test]
    fn system_local_command_entries_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.jsonl");
        write_jsonl(&p, &[
            r#"{"type":"user"}"#,
            r#"{"type":"assistant"}"#,
            r#"{"type":"system","subtype":"local_command"}"#,
        ]);
        // system:local_command is skipped → assistant is last → WaitingForInput
        assert_eq!(detect_status_fast(&p).unwrap(), ConversationStatus::WaitingForInput);
    }

    #[test]
    fn tool_result_user_entries_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.jsonl");
        write_jsonl(&p, &[
            r#"{"type":"user"}"#,
            r#"{"type":"assistant"}"#,
            r#"{"type":"user","toolUseResult":{"type":"tool_result","content":"ok"}}"#,
        ]);
        // Tool-result user entry is skipped → assistant is last → WaitingForInput
        assert_eq!(detect_status_fast(&p).unwrap(), ConversationStatus::WaitingForInput);
    }

    #[test]
    fn real_user_message_after_tool_result_returns_active() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.jsonl");
        write_jsonl(&p, &[
            r#"{"type":"assistant"}"#,
            r#"{"type":"user","toolUseResult":{"type":"tool_result","content":"ok"}}"#,
            r#"{"type":"user"}"#,
        ]);
        // Real user message (no toolUseResult) is last → Active
        assert_eq!(detect_status_fast(&p).unwrap(), ConversationStatus::Active);
    }

    #[test]
    fn multiple_tool_results_all_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.jsonl");
        write_jsonl(&p, &[
            r#"{"type":"user"}"#,
            r#"{"type":"assistant"}"#,
            r#"{"type":"user","toolUseResult":{"type":"tool_result","content":"1"}}"#,
            r#"{"type":"user","toolUseResult":{"type":"tool_result","content":"2"}}"#,
            r#"{"type":"user","toolUseResult":{"type":"tool_result","content":"3"}}"#,
        ]);
        // All tool-result entries skipped → assistant is last → WaitingForInput
        assert_eq!(detect_status_fast(&p).unwrap(), ConversationStatus::WaitingForInput);
    }

    #[test]
    fn non_turn_duration_system_entries_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.jsonl");
        write_jsonl(&p, &[
            r#"{"type":"user"}"#,
            r#"{"type":"assistant"}"#,
            r#"{"type":"system","subtype":"compact_boundary"}"#,
            r#"{"type":"system","subtype":"api_error"}"#,
            r#"{"type":"system","subtype":"microcompact_boundary"}"#,
        ]);
        // All non-turn_duration system entries skipped → assistant is last → WaitingForInput
        assert_eq!(detect_status_fast(&p).unwrap(), ConversationStatus::WaitingForInput);
    }
}
