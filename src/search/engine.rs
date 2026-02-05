//! Search engine for finding conversations by content or project.

use std::path::PathBuf;

use crate::claude::grouping::ConversationGroup;
use crate::search::types::{SearchFilterType, SearchQuery, SearchResult};

/// Search engine for finding conversations.
pub struct SearchEngine {
    /// Path to ~/.claude directory
    #[allow(dead_code)]
    claude_dir: PathBuf,
}

impl SearchEngine {
    /// Create a new search engine.
    pub fn new(claude_dir: PathBuf) -> Self {
        Self { claude_dir }
    }

    /// Search conversations by content (title/summary).
    pub fn search_content(
        &self,
        query: &str,
        groups: &[ConversationGroup],
    ) -> Vec<SearchResult> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for group in groups {
            for conv in group.conversations() {
                // Skip archived conversations
                if conv.is_archived {
                    continue;
                }

                // Search in display text (which contains summary or first message)
                if conv.display.to_lowercase().contains(&query_lower) {
                    let snippet = self.create_snippet(&conv.display, &query_lower);
                    results.push(SearchResult::new(conv.clone(), snippet));
                    continue;
                }

                // Search in summary if available
                if let Some(ref summary) = conv.summary {
                    if summary.to_lowercase().contains(&query_lower) {
                        let snippet = self.create_snippet(summary, &query_lower);
                        results.push(SearchResult::new(conv.clone(), snippet));
                    }
                }
            }
        }

        results
    }

    /// Search conversations by project path.
    pub fn search_project(
        &self,
        query: &str,
        groups: &[ConversationGroup],
    ) -> Vec<SearchResult> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for group in groups {
            // Check if the group's project path matches
            let group_matches = if let Some(project_path) = group.project_path() {
                project_path
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&query_lower)
            } else {
                false
            };

            if group_matches {
                // Add all non-archived conversations from this group
                for conv in group.conversations() {
                    if conv.is_archived {
                        continue;
                    }
                    let snippet = format!("Project: {}", conv.project_path.display());
                    results.push(SearchResult::new(conv.clone(), snippet));
                }
            }
        }

        results
    }

    /// Combined search with all filters.
    pub fn search(
        &self,
        query: &SearchQuery,
        groups: &[ConversationGroup],
    ) -> Vec<SearchResult> {
        if query.is_empty() {
            return Vec::new();
        }

        match query.filter_type {
            SearchFilterType::All => {
                // Search both content and project, deduplicate by session_id
                let mut results = self.search_content(&query.text, groups);
                let project_results = self.search_project(&query.text, groups);

                // Add project results that aren't already in content results
                for result in project_results {
                    if !results.iter().any(|r| r.conversation.session_id == result.conversation.session_id) {
                        results.push(result);
                    }
                }

                results
            }
            SearchFilterType::Content => self.search_content(&query.text, groups),
            SearchFilterType::Project => self.search_project(&query.text, groups),
        }
    }

    /// Create a snippet with the query highlighted (indicated by context).
    /// Uses character indices to handle UTF-8 safely.
    fn create_snippet(&self, text: &str, query: &str) -> String {
        let text_lower = text.to_lowercase();
        let query_lower = query.to_lowercase();

        // Find character position of match (convert byte index to char index)
        let match_char_pos = text_lower
            .char_indices()
            .position(|(byte_idx, _)| text_lower[byte_idx..].starts_with(&query_lower));

        if let Some(pos) = match_char_pos {
            let char_count = text.chars().count();
            let query_char_len = query.chars().count();

            // Get context around the match (in character indices)
            let start = pos.saturating_sub(20);
            let end = (pos + query_char_len + 40).min(char_count);

            let mut snippet = String::new();
            if start > 0 {
                snippet.push_str("...");
            }
            let slice: String = text.chars().skip(start).take(end - start).collect();
            snippet.push_str(&slice);
            if end < char_count {
                snippet.push_str("...");
            }
            snippet
        } else {
            // Truncate if no match found
            let char_count = text.chars().count();
            if char_count > 60 {
                let truncated: String = text.chars().take(60).collect();
                format!("{}...", truncated)
            } else {
                text.to_string()
            }
        }
    }
}
