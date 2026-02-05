//! Types for the search feature.

use crate::claude::conversation::Conversation;

/// Filter type for search queries
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchFilterType {
    /// Search all fields (title/summary and project)
    #[default]
    All,
    /// Search only content (title/summary)
    Content,
    /// Search only project paths
    Project,
}

impl SearchFilterType {
    /// Get display name for the filter
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Content => "Content",
            Self::Project => "Project",
        }
    }

    /// Cycle to the next filter type
    pub fn next(&self) -> Self {
        match self {
            Self::All => Self::Content,
            Self::Content => Self::Project,
            Self::Project => Self::All,
        }
    }
}

/// A search query with text and filter type
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// The search text
    pub text: String,
    /// The filter type
    pub filter_type: SearchFilterType,
}

impl SearchQuery {
    /// Create a new search query
    pub fn new(text: impl Into<String>, filter_type: SearchFilterType) -> Self {
        Self {
            text: text.into(),
            filter_type,
        }
    }

    /// Check if the query is empty
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// A search result with the matching conversation and preview snippet
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// The matching conversation
    pub conversation: Conversation,
    /// Preview snippet showing where the match occurred
    pub preview_snippet: String,
}

impl SearchResult {
    /// Create a new search result
    pub fn new(conversation: Conversation, preview_snippet: impl Into<String>) -> Self {
        Self {
            conversation,
            preview_snippet: preview_snippet.into(),
        }
    }
}
