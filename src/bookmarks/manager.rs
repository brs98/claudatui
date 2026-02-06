//! Bookmark manager for persistence and retrieval.

use super::{Bookmark, BookmarkTarget};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;

/// Bookmark manager for persistence and retrieval
pub struct BookmarkManager {
    /// slot -> bookmark mapping
    bookmarks: HashMap<u8, Bookmark>,
    /// Path to the bookmarks file
    config_path: PathBuf,
}

impl BookmarkManager {
    /// Load bookmarks from the config directory
    pub fn load() -> Result<Self> {
        let config_dir = dirs::config_dir()
            .context("Could not find config directory")?
            .join("claudatui");

        // Ensure the config directory exists
        std::fs::create_dir_all(&config_dir).context("Failed to create config directory")?;

        let path = config_dir.join("bookmarks.json");

        // Load existing bookmarks or create empty
        let bookmarks: HashMap<u8, Bookmark> = if path.exists() {
            let content =
                std::fs::read_to_string(&path).context("Failed to read bookmarks file")?;
            serde_json::from_str(&content).context("Failed to parse bookmarks file")?
        } else {
            HashMap::new()
        };

        Ok(Self {
            bookmarks,
            config_path: path,
        })
    }

    /// Create a new empty bookmark manager (for testing)
    pub fn empty() -> Self {
        Self {
            bookmarks: HashMap::new(),
            config_path: PathBuf::new(),
        }
    }

    /// Save bookmarks to disk
    pub fn save(&self) -> Result<()> {
        if self.config_path.as_os_str().is_empty() {
            return Ok(()); // Skip saving if no path set (empty manager)
        }

        // Ensure parent directory exists
        if let Some(parent) = self.config_path.parent() {
            std::fs::create_dir_all(parent).context("Failed to create config directory")?;
        }

        let content = serde_json::to_string_pretty(&self.bookmarks)
            .context("Failed to serialize bookmarks")?;

        std::fs::write(&self.config_path, content).context("Failed to write bookmarks file")?;

        Ok(())
    }

    /// Get a bookmark by slot
    pub fn get(&self, slot: u8) -> Option<&Bookmark> {
        self.bookmarks.get(&slot)
    }

    /// Get all bookmarks as a sorted vector by slot
    pub fn get_all(&self) -> Vec<&Bookmark> {
        let mut bookmarks: Vec<&Bookmark> = self.bookmarks.values().collect();
        bookmarks.sort_by_key(|b| b.slot);
        bookmarks
    }

    /// Add or replace a bookmark at the given slot
    pub fn set(&mut self, bookmark: Bookmark) -> Result<()> {
        self.bookmarks.insert(bookmark.slot, bookmark);
        self.save()
    }

    /// Remove a bookmark at the given slot
    pub fn remove(&mut self, slot: u8) -> Result<bool> {
        let removed = self.bookmarks.remove(&slot).is_some();
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    /// Check if a slot is occupied
    pub fn has_slot(&self, slot: u8) -> bool {
        self.bookmarks.contains_key(&slot)
    }

    /// Get the number of bookmarks
    pub fn count(&self) -> usize {
        self.bookmarks.len()
    }

    /// Check if a group is bookmarked
    pub fn is_group_bookmarked(&self, group_key: &str) -> Option<u8> {
        self.bookmarks
            .iter()
            .find(|(_, b)| b.group_key() == group_key)
            .map(|(slot, _)| *slot)
    }

    /// Check if a conversation is bookmarked
    pub fn is_conversation_bookmarked(&self, session_id: &str) -> Option<u8> {
        self.bookmarks.iter()
            .find(|(_, b)| {
                matches!(&b.target, BookmarkTarget::Conversation { session_id: sid, .. } if sid == session_id)
            })
            .map(|(slot, _)| *slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn empty_manager_has_zero_count_and_no_bookmarks() {
        let manager = BookmarkManager::empty();
        assert_eq!(manager.count(), 0);
        assert!(manager.get(1).is_none());
    }

    #[test]
    fn set_bookmark_increments_count_and_is_retrievable() {
        let mut manager = BookmarkManager::empty();

        let bookmark = Bookmark::new_project(
            1,
            "Test Project".to_string(),
            PathBuf::from("/test/path"),
            "test_group".to_string(),
        );

        manager.set(bookmark.clone()).unwrap();

        assert_eq!(manager.count(), 1);
        assert!(manager.get(1).is_some());
        assert_eq!(manager.get(1).unwrap().name, "Test Project");
    }

    #[test]
    fn remove_bookmark_decrements_count_and_second_remove_returns_false() {
        let mut manager = BookmarkManager::empty();

        let bookmark = Bookmark::new_project(
            1,
            "Test Project".to_string(),
            PathBuf::from("/test/path"),
            "test_group".to_string(),
        );

        manager.set(bookmark).unwrap();
        assert_eq!(manager.count(), 1);

        let removed = manager.remove(1).unwrap();
        assert!(removed);
        assert_eq!(manager.count(), 0);

        let removed_again = manager.remove(1).unwrap();
        assert!(!removed_again);
    }

    #[test]
    fn is_group_bookmarked_returns_slot_for_matching_group_key() {
        let mut manager = BookmarkManager::empty();

        let bookmark = Bookmark::new_project(
            2,
            "Test Project".to_string(),
            PathBuf::from("/test/path"),
            "my_group".to_string(),
        );

        manager.set(bookmark).unwrap();

        assert_eq!(manager.is_group_bookmarked("my_group"), Some(2));
        assert_eq!(manager.is_group_bookmarked("other_group"), None);
    }
}
