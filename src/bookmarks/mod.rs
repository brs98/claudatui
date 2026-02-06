//! Bookmark management for quick access to projects and conversations.
//!
//! Bookmarks allow users to quickly jump to frequently used projects
//! and conversations using hotkeys (1-9).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Bookmark entry - can reference a project (group) or specific conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    /// Unique bookmark ID
    pub id: String,
    /// Hotkey slot (1-9)
    pub slot: u8,
    /// Display name (user-customizable)
    pub name: String,
    /// Target of the bookmark
    pub target: BookmarkTarget,
    /// Unix timestamp when bookmark was created
    pub created_at: i64,
}

/// Target of a bookmark - can be a project or specific conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BookmarkTarget {
    /// Reference to a project (group) - opens most recent/active conversation
    Project {
        /// Path to the project directory
        project_path: PathBuf,
        /// Group key for lookup
        group_key: String,
    },
    /// Reference to a specific conversation
    Conversation {
        /// Claude session ID
        session_id: String,
        /// Path to the project directory
        project_path: PathBuf,
        /// Group key for lookup
        group_key: String,
    },
}

impl Bookmark {
    /// Create a new bookmark for a project
    pub fn new_project(slot: u8, name: String, project_path: PathBuf, group_key: String) -> Self {
        Self {
            id: format!(
                "bookmark_{}_{}",
                slot,
                chrono::Utc::now().timestamp_millis()
            ),
            slot,
            name,
            target: BookmarkTarget::Project {
                project_path,
                group_key,
            },
            created_at: chrono::Utc::now().timestamp_millis(),
        }
    }

    /// Create a new bookmark for a conversation
    pub fn new_conversation(
        slot: u8,
        name: String,
        session_id: String,
        project_path: PathBuf,
        group_key: String,
    ) -> Self {
        Self {
            id: format!(
                "bookmark_{}_{}",
                slot,
                chrono::Utc::now().timestamp_millis()
            ),
            slot,
            name,
            target: BookmarkTarget::Conversation {
                session_id,
                project_path,
                group_key,
            },
            created_at: chrono::Utc::now().timestamp_millis(),
        }
    }

    /// Get the group key associated with this bookmark
    pub fn group_key(&self) -> &str {
        match &self.target {
            BookmarkTarget::Project { group_key, .. } => group_key,
            BookmarkTarget::Conversation { group_key, .. } => group_key,
        }
    }

    /// Get the project path associated with this bookmark
    pub fn project_path(&self) -> &PathBuf {
        match &self.target {
            BookmarkTarget::Project { project_path, .. } => project_path,
            BookmarkTarget::Conversation { project_path, .. } => project_path,
        }
    }
}

pub mod manager;

pub use manager::BookmarkManager;
