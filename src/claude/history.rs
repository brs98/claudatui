use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Entry from ~/.claude/history.jsonl
/// Note: This is kept for reference but no longer used.
/// We now use sessions-index.json instead (see sessions.rs).
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct HistoryEntry {
    /// Session ID (UUID)
    #[serde(rename = "sessionId")]
    pub session_id: String,
    /// Project path (e.g., "/Users/brandon/personal/myproject")
    pub project: String,
    /// Display text (first user message)
    pub display: String,
    /// Timestamp in milliseconds
    pub timestamp: i64,
}

/// Parse the Claude history file
/// Note: This is kept for reference but no longer used.
/// We now use sessions-index.json instead (see sessions.rs).
#[allow(dead_code)]
pub fn parse_history(claude_dir: &Path) -> Result<Vec<HistoryEntry>> {
    let history_path = claude_dir.join("history.jsonl");

    if !history_path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(&history_path)
        .with_context(|| format!("Failed to open history file: {:?}", history_path))?;

    let reader = BufReader::new(file);
    let mut entries = Vec::new();

    for line in reader.lines() {
        let line = line.context("Failed to read line from history")?;
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<HistoryEntry>(&line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                // Skip malformed entries but log them
                eprintln!("Warning: Failed to parse history entry: {}", e);
            }
        }
    }

    // Sort by timestamp descending (most recent first)
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    Ok(entries)
}

/// Get the conversation JSONL file path for a history entry
#[allow(dead_code)]
pub fn get_conversation_path(claude_dir: &Path, entry: &HistoryEntry) -> PathBuf {
    // The path is escaped by replacing / with -
    // e.g., /Users/brandon/personal/myproject -> -Users-brandon-personal-myproject
    let escaped_path = escape_path(&entry.project);
    claude_dir
        .join("projects")
        .join(&escaped_path)
        .join(format!("{}.jsonl", entry.session_id))
}

/// Escape a path for use in the projects directory
/// "/" becomes "-"
fn escape_path(path: &str) -> String {
    path.replace('/', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_path() {
        assert_eq!(
            escape_path("/Users/brandon/personal/myproject"),
            "-Users-brandon-personal-myproject"
        );
    }
}
