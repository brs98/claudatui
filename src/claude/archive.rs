use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Default number of days before auto-archiving idle conversations
const DEFAULT_AUTO_ARCHIVE_DAYS: u32 = 30;

/// Archive metadata for a single session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveEntry {
    pub archived_at: DateTime<Utc>,
    pub auto_archived: bool,
}

/// Full archive state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveState {
    pub version: u32,
    #[serde(default = "default_auto_archive_days")]
    pub auto_archive_days: Option<u32>, // None = disabled
    pub archived_sessions: HashMap<String, ArchiveEntry>,
}

fn default_auto_archive_days() -> Option<u32> {
    Some(DEFAULT_AUTO_ARCHIVE_DAYS)
}

impl Default for ArchiveState {
    fn default() -> Self {
        Self {
            version: 1,
            auto_archive_days: Some(DEFAULT_AUTO_ARCHIVE_DAYS),
            archived_sessions: HashMap::new(),
        }
    }
}

/// Archive manager for persistence
pub struct ArchiveManager {
    archive_path: PathBuf,
    state: ArchiveState,
    dirty: bool, // Track if save needed
}

impl ArchiveManager {
    /// Create a new ArchiveManager, loading existing state or creating defaults
    pub fn new(claude_dir: &Path) -> Result<Self> {
        let archive_path = claude_dir.join("claudatui-archive.json");

        let state = if archive_path.exists() {
            // Load existing archive file
            let content = fs::read_to_string(&archive_path)
                .with_context(|| format!("Failed to read archive file: {:?}", archive_path))?;

            serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse archive file: {:?}", archive_path))?
        } else {
            // Create default state
            ArchiveState::default()
        };

        Ok(Self {
            archive_path,
            state,
            dirty: false,
        })
    }

    /// Check if a session is archived
    pub fn is_archived(&self, session_id: &str) -> bool {
        self.state.archived_sessions.contains_key(session_id)
    }

    /// Get archive entry for a session (if archived)
    pub fn get_entry(&self, session_id: &str) -> Option<&ArchiveEntry> {
        self.state.archived_sessions.get(session_id)
    }

    /// Archive a session
    pub fn archive(&mut self, session_id: &str, auto_archived: bool) {
        let entry = ArchiveEntry {
            archived_at: Utc::now(),
            auto_archived,
        };
        self.state
            .archived_sessions
            .insert(session_id.to_string(), entry);
        self.dirty = true;
    }

    /// Unarchive a session
    pub fn unarchive(&mut self, session_id: &str) {
        if self.state.archived_sessions.remove(session_id).is_some() {
            self.dirty = true;
        }
    }

    /// Check if a conversation should be auto-archived based on its timestamp
    pub fn should_auto_archive(&self, timestamp_ms: i64) -> bool {
        let Some(days) = self.state.auto_archive_days else {
            return false; // Auto-archive disabled
        };

        let now = Utc::now();
        let Some(conv_time) = DateTime::from_timestamp_millis(timestamp_ms) else {
            return false;
        };

        let age = now.signed_duration_since(conv_time);
        let days_old = age.num_days();

        days_old >= days as i64
    }

    /// Set auto-archive days (None to disable)
    pub fn set_auto_archive_days(&mut self, days: Option<u32>) {
        self.state.auto_archive_days = days;
        self.dirty = true;
    }

    /// Get auto-archive days (None if disabled)
    pub fn get_auto_archive_days(&self) -> Option<u32> {
        self.state.auto_archive_days
    }

    /// Get all archived sessions
    pub fn get_archived_sessions(&self) -> &HashMap<String, ArchiveEntry> {
        &self.state.archived_sessions
    }

    /// Check if state needs to be saved
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Save archive state to disk
    /// Uses atomic write (write to temp, then rename) for safety
    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }

        // Serialize to JSON
        let json = serde_json::to_string_pretty(&self.state)
            .context("Failed to serialize archive state")?;

        // Write to temp file first (atomic write)
        let temp_path = self.archive_path.with_extension("tmp");
        fs::write(&temp_path, json)
            .with_context(|| format!("Failed to write temp archive file: {:?}", temp_path))?;

        // Rename temp file to actual file (atomic on most filesystems)
        fs::rename(&temp_path, &self.archive_path)
            .with_context(|| format!("Failed to rename archive file: {:?}", self.archive_path))?;

        self.dirty = false;
        Ok(())
    }

    /// Get archive timestamp for a session (if archived)
    pub fn get_archived_at(&self, session_id: &str) -> Option<DateTime<Utc>> {
        self.state
            .archived_sessions
            .get(session_id)
            .map(|e| e.archived_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_has_version_one_and_empty_sessions() {
        let state = ArchiveState::default();
        assert_eq!(state.version, 1);
        assert_eq!(state.auto_archive_days, Some(DEFAULT_AUTO_ARCHIVE_DAYS));
        assert!(state.archived_sessions.is_empty());
    }

    #[test]
    fn archive_then_unarchive_toggles_session_state() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut manager = ArchiveManager::new(temp_dir.path()).unwrap();

        // Initially not archived
        assert!(!manager.is_archived("session-1"));

        // Archive it
        manager.archive("session-1", false);
        assert!(manager.is_archived("session-1"));
        assert!(manager.is_dirty());

        // Check entry
        let entry = manager.get_entry("session-1").unwrap();
        assert!(!entry.auto_archived);

        // Unarchive it
        manager.unarchive("session-1");
        assert!(!manager.is_archived("session-1"));
    }

    #[test]
    fn should_auto_archive_respects_day_threshold_and_disabled_state() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut manager = ArchiveManager::new(temp_dir.path()).unwrap();

        // Set auto-archive to 7 days
        manager.set_auto_archive_days(Some(7));

        let now = Utc::now().timestamp_millis();
        let eight_days_ago = now - (8 * 24 * 60 * 60 * 1000);
        let five_days_ago = now - (5 * 24 * 60 * 60 * 1000);

        // Should auto-archive (older than 7 days)
        assert!(manager.should_auto_archive(eight_days_ago));

        // Should not auto-archive (newer than 7 days)
        assert!(!manager.should_auto_archive(five_days_ago));

        // Disable auto-archive
        manager.set_auto_archive_days(None);
        assert!(!manager.should_auto_archive(eight_days_ago));
    }

    #[test]
    fn save_and_load_preserves_archive_entries_and_settings() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Create and populate manager
        {
            let mut manager = ArchiveManager::new(temp_dir.path()).unwrap();
            manager.archive("session-1", false);
            manager.archive("session-2", true);
            manager.set_auto_archive_days(Some(14));
            manager.save().unwrap();
        }

        // Load and verify
        {
            let manager = ArchiveManager::new(temp_dir.path()).unwrap();
            assert!(manager.is_archived("session-1"));
            assert!(manager.is_archived("session-2"));

            let entry1 = manager.get_entry("session-1").unwrap();
            assert!(!entry1.auto_archived);

            let entry2 = manager.get_entry("session-2").unwrap();
            assert!(entry2.auto_archived);

            assert_eq!(manager.get_auto_archive_days(), Some(14));
        }
    }

    #[test]
    fn get_archived_at_returns_timestamp_within_archive_window() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut manager = ArchiveManager::new(temp_dir.path()).unwrap();

        // Not archived
        assert!(manager.get_archived_at("session-1").is_none());

        // Archive it
        let before_archive = Utc::now();
        manager.archive("session-1", false);
        let after_archive = Utc::now();

        let archived_at = manager.get_archived_at("session-1").unwrap();
        assert!(archived_at >= before_archive);
        assert!(archived_at <= after_archive);
    }
}
