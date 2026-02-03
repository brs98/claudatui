use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::Path;

/// Parsed sessions-index.json file
#[derive(Debug, Deserialize)]
struct SessionsIndex {
    #[allow(dead_code)]
    version: u32,
    entries: Vec<SessionEntryRaw>,
}

/// Raw entry from sessions-index.json (matches JSON structure)
#[derive(Debug, Deserialize)]
struct SessionEntryRaw {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "fullPath")]
    full_path: String,
    #[serde(rename = "fileMtime")]
    file_mtime: i64,
    #[serde(rename = "firstPrompt")]
    first_prompt: String,
    summary: Option<String>,
    #[serde(rename = "messageCount")]
    message_count: u32,
    created: String,
    modified: String,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
    #[serde(rename = "projectPath")]
    project_path: String,
    #[serde(rename = "isSidechain")]
    is_sidechain: bool,
}

/// Processed session entry for use in the application
#[derive(Debug, Clone)]
pub struct SessionEntry {
    pub session_id: String,
    pub full_path: String,
    pub file_mtime: i64,
    pub first_prompt: String,
    pub summary: Option<String>,
    pub message_count: u32,
    #[allow(dead_code)]
    pub created: String,
    pub modified: String,
    pub git_branch: Option<String>,
    pub project_path: String,
}

impl From<SessionEntryRaw> for SessionEntry {
    fn from(raw: SessionEntryRaw) -> Self {
        Self {
            session_id: raw.session_id,
            full_path: raw.full_path,
            file_mtime: raw.file_mtime,
            first_prompt: raw.first_prompt,
            summary: raw.summary,
            message_count: raw.message_count,
            created: raw.created,
            modified: raw.modified,
            git_branch: if raw.git_branch.as_deref() == Some("") {
                None
            } else {
                raw.git_branch
            },
            project_path: raw.project_path,
        }
    }
}

/// Parse all sessions-index.json files from ~/.claude/projects/*/
///
/// Returns sessions sorted by modified timestamp (most recent first),
/// filtering out sidechain sessions (background agents).
pub fn parse_all_sessions(claude_dir: &Path) -> Result<Vec<SessionEntry>> {
    let projects_dir = claude_dir.join("projects");

    if !projects_dir.exists() {
        return Ok(Vec::new());
    }

    let mut all_entries = Vec::new();

    // Read all project directories
    let entries = fs::read_dir(&projects_dir)
        .with_context(|| format!("Failed to read projects directory: {:?}", projects_dir))?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let sessions_index_path = path.join("sessions-index.json");
        if !sessions_index_path.exists() {
            continue;
        }

        match parse_sessions_index(&sessions_index_path) {
            Ok(sessions) => {
                all_entries.extend(sessions);
            }
            Err(e) => {
                eprintln!(
                    "Warning: Failed to parse sessions-index.json at {:?}: {}",
                    sessions_index_path, e
                );
            }
        }
    }

    // Sort by modified timestamp descending (most recent first)
    // Parse ISO 8601 dates for comparison
    all_entries.sort_by(|a, b| b.modified.cmp(&a.modified));

    Ok(all_entries)
}

/// Parse a single sessions-index.json file
fn parse_sessions_index(path: &Path) -> Result<Vec<SessionEntry>> {
    let file = File::open(path).with_context(|| format!("Failed to open: {:?}", path))?;

    let reader = BufReader::new(file);
    let index: SessionsIndex =
        serde_json::from_reader(reader).with_context(|| format!("Failed to parse: {:?}", path))?;

    // Filter out sidechain sessions and convert to SessionEntry
    let entries: Vec<SessionEntry> = index
        .entries
        .into_iter()
        .filter(|e| !e.is_sidechain)
        .map(SessionEntry::from)
        .collect();

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_git_branch_becomes_none() {
        let raw = SessionEntryRaw {
            session_id: "test".to_string(),
            full_path: "/path/to/test.jsonl".to_string(),
            file_mtime: 1234567890,
            first_prompt: "Hello".to_string(),
            summary: Some("Test summary".to_string()),
            message_count: 5,
            created: "2026-01-01T00:00:00Z".to_string(),
            modified: "2026-01-01T00:00:00Z".to_string(),
            git_branch: Some("".to_string()),
            project_path: "/path/to/project".to_string(),
            is_sidechain: false,
        };

        let entry = SessionEntry::from(raw);
        assert!(entry.git_branch.is_none());
    }
}
