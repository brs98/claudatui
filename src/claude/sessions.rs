use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Parsed sessions-index.json file
#[derive(Debug, Deserialize)]
struct SessionsIndex {
    #[allow(dead_code)]
    version: u32,
    entries: Vec<SessionEntryRaw>,
}

/// Raw entry from sessions-index.json (matches JSON structure)
#[derive(Debug, Clone, Deserialize)]
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

/// Represents a discovered session file
struct SessionFile {
    path: PathBuf,
    file_mtime: i64,
}

/// Check if a string looks like a UUID
fn is_uuid(s: &str) -> bool {
    // UUID format: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx (36 chars with hyphens)
    if s.len() != 36 {
        return false;
    }
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 {
        return false;
    }
    // Check lengths: 8-4-4-4-12
    let expected_lens = [8, 4, 4, 4, 12];
    for (part, &expected_len) in parts.iter().zip(&expected_lens) {
        if part.len() != expected_len || !part.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
    }
    true
}

/// Discover all session files (.jsonl) in a project directory
fn discover_session_files(project_dir: &Path) -> HashMap<String, SessionFile> {
    let mut sessions = HashMap::new();

    let entries = match fs::read_dir(project_dir) {
        Ok(e) => e,
        Err(_) => return sessions,
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // Only look at .jsonl files
        if path.extension().is_none_or(|ext| ext != "jsonl") {
            continue;
        }

        // Extract session ID from filename (stem)
        let session_id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(id) if is_uuid(id) => id.to_string(),
            _ => continue,
        };

        // Get file modification time
        let file_mtime = match entry.metadata() {
            Ok(meta) => meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            Err(_) => 0,
        };

        sessions.insert(
            session_id,
            SessionFile {
                path,
                file_mtime,
            },
        );
    }

    sessions
}

/// Parse the first user prompt from a session JSONL file
fn parse_first_user_prompt(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines().map_while(Result::ok) {
        // Parse each line as JSON
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Check if it's a user message
        if value.get("type").and_then(|t| t.as_str()) != Some("user") {
            continue;
        }

        // Extract content from message
        let content = value.get("message")?.get("content")?;

        // Content can be a string or an array
        let text = match content {
            Value::String(s) => s.clone(),
            Value::Array(arr) => {
                // Look for text content in the array
                for item in arr {
                    if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                        if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                            return Some(truncate_prompt(text));
                        }
                    }
                    // Skip tool_result items - keep looking for actual text
                    if item.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        continue;
                    }
                }
                continue; // No text found in array, try next message
            }
            _ => continue,
        };

        // Skip empty prompts
        if text.trim().is_empty() {
            continue;
        }

        return Some(truncate_prompt(&text));
    }

    None
}

/// Truncate a prompt to a reasonable length for display
fn truncate_prompt(text: &str) -> String {
    const MAX_LEN: usize = 200;
    let text = text.trim();
    if text.len() <= MAX_LEN {
        text.to_string()
    } else {
        format!("{}...", &text[..MAX_LEN])
    }
}

/// Load sessions-index.json as a HashMap for O(1) lookup
fn load_index_cache(project_dir: &Path) -> HashMap<String, SessionEntryRaw> {
    let index_path = project_dir.join("sessions-index.json");
    if !index_path.exists() {
        return HashMap::new();
    }

    let file = match File::open(&index_path) {
        Ok(f) => f,
        Err(_) => return HashMap::new(),
    };

    let reader = BufReader::new(file);
    let index: SessionsIndex = match serde_json::from_reader(reader) {
        Ok(i) => i,
        Err(_) => return HashMap::new(),
    };

    index
        .entries
        .into_iter()
        .map(|e| (e.session_id.clone(), e))
        .collect()
}

/// Extract project path from escaped directory name
fn extract_project_path(dir_name: &str) -> String {
    // Claude Code escapes paths by replacing / with -
    // e.g., "-Users-brandon-personal-claudatui" -> "/Users/brandon/personal/claudatui"
    if dir_name.starts_with('-') {
        dir_name.replacen('-', "/", 1).replace('-', "/")
    } else {
        dir_name.replace('-', "/")
    }
}

/// Parse all sessions from ~/.claude/projects/*/
///
/// Uses two-phase discovery:
/// 1. Scan .jsonl files (authoritative source of which sessions exist)
/// 2. Enrich with metadata from sessions-index.json where available
///
/// Returns sessions sorted by file modification time (most recent first),
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

        let project_dir = entry.path();
        if !project_dir.is_dir() {
            continue;
        }

        // Extract project path from directory name
        let project_path = project_dir
            .file_name()
            .and_then(|n| n.to_str())
            .map(extract_project_path)
            .unwrap_or_default();

        // Phase 1: Discover all session files (authoritative)
        let session_files = discover_session_files(&project_dir);

        if session_files.is_empty() {
            continue;
        }

        // Phase 2: Load index cache for metadata enrichment
        let index_cache = load_index_cache(&project_dir);

        // Build session entries
        for (session_id, session_file) in session_files {
            // Check if this session is in the index cache
            if let Some(cached) = index_cache.get(&session_id) {
                // Skip sidechain sessions
                if cached.is_sidechain {
                    continue;
                }
                // Use cached metadata
                all_entries.push(SessionEntry::from(cached.clone()));
            } else {
                // Not in cache - parse first prompt on-demand
                let first_prompt = parse_first_user_prompt(&session_file.path)
                    .unwrap_or_else(|| "New session".to_string());

                // Create entry with minimal metadata
                all_entries.push(SessionEntry {
                    session_id,
                    full_path: session_file.path.to_string_lossy().to_string(),
                    file_mtime: session_file.file_mtime,
                    first_prompt,
                    summary: None,
                    message_count: 0,
                    created: String::new(),
                    modified: String::new(),
                    git_branch: None,
                    project_path: project_path.clone(),
                });
            }
        }
    }

    // Sort by file_mtime descending (most recent first)
    // For sessions with cached data, use their modified timestamp if available
    all_entries.sort_by(|a, b| {
        // Use file_mtime as primary sort key (most reliable for recency)
        b.file_mtime.cmp(&a.file_mtime)
    });

    Ok(all_entries)
}

/// Parse a single sessions-index.json file (kept for potential future use)
#[allow(dead_code)]
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

    #[test]
    fn test_is_uuid() {
        // Valid UUIDs
        assert!(is_uuid("d90ed21d-ed03-4e94-87d7-dbc5de6cc828"));
        assert!(is_uuid("00000000-0000-0000-0000-000000000000"));
        assert!(is_uuid("ffffffff-ffff-ffff-ffff-ffffffffffff"));
        assert!(is_uuid("ABCDEF12-3456-7890-abcd-ef1234567890"));

        // Invalid UUIDs
        assert!(!is_uuid("not-a-uuid"));
        assert!(!is_uuid("sessions-index"));
        assert!(!is_uuid(""));
        assert!(!is_uuid("d90ed21d-ed03-4e94-87d7-dbc5de6cc82")); // Too short
        assert!(!is_uuid("d90ed21d-ed03-4e94-87d7-dbc5de6cc8289")); // Too long
        assert!(!is_uuid("g90ed21d-ed03-4e94-87d7-dbc5de6cc828")); // Invalid char
    }

    #[test]
    fn test_extract_project_path() {
        assert_eq!(
            extract_project_path("-Users-brandon-personal-claudatui"),
            "/Users/brandon/personal/claudatui"
        );
        assert_eq!(
            extract_project_path("-Users-brandon--dotfiles"),
            "/Users/brandon//dotfiles"
        );
    }

    #[test]
    fn test_truncate_prompt() {
        // Short text should not be truncated
        assert_eq!(truncate_prompt("Hello world"), "Hello world");
        assert_eq!(truncate_prompt("  Hello world  "), "Hello world");

        // Long text should be truncated
        let long_text = "a".repeat(300);
        let truncated = truncate_prompt(&long_text);
        assert!(truncated.ends_with("..."));
        assert_eq!(truncated.len(), 203); // 200 chars + "..."
    }
}
