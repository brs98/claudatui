//! Search engine for finding conversations by content or project.

use std::path::PathBuf;

use crate::claude::grouping::ConversationGroup;
use crate::search::types::{SearchFilterType, SearchQuery, SearchResult};

/// Search engine for finding conversations.
pub struct SearchEngine {
    /// Path to ~/.claude directory
    #[expect(dead_code, reason = "planned for future use")]
    claude_dir: PathBuf,
}

impl SearchEngine {
    /// Create a new search engine.
    pub fn new(claude_dir: PathBuf) -> Self {
        Self { claude_dir }
    }

    /// Search conversations by content (title/summary).
    pub fn search_content(&self, query: &str, groups: &[ConversationGroup]) -> Vec<SearchResult> {
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
                    let snippet = Self::create_snippet(&conv.display, &query_lower);
                    results.push(SearchResult::new(conv.clone(), snippet));
                    continue;
                }

                // Search in summary if available
                if let Some(ref summary) = conv.summary {
                    if summary.to_lowercase().contains(&query_lower) {
                        let snippet = Self::create_snippet(summary, &query_lower);
                        results.push(SearchResult::new(conv.clone(), snippet));
                    }
                }
            }
        }

        results
    }

    /// Search conversations by project path.
    pub fn search_project(&self, query: &str, groups: &[ConversationGroup]) -> Vec<SearchResult> {
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
    pub fn search(&self, query: &SearchQuery, groups: &[ConversationGroup]) -> Vec<SearchResult> {
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
                    if !results
                        .iter()
                        .any(|r| r.conversation.session_id == result.conversation.session_id)
                    {
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
    pub(crate) fn create_snippet(text: &str, query: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::conversation::Conversation;

    fn make_conversation(
        session_id: &str,
        display: &str,
        summary: Option<&str>,
        project_path: &str,
    ) -> Conversation {
        Conversation {
            session_id: session_id.to_string(),
            display: display.to_string(),
            summary: summary.map(ToString::to_string),
            timestamp: 1000,
            modified: "2026-01-01T00:00:00Z".to_string(),
            project_path: PathBuf::from(project_path),
            message_count: 1,
            git_branch: None,
            is_plan_implementation: false,
            is_archived: false,
            archived_at: None,
        }
    }

    fn make_group(conversations: Vec<Conversation>) -> ConversationGroup {
        ConversationGroup::Directory {
            parent: "personal".to_string(),
            project: "test-project".to_string(),
            conversations,
        }
    }

    #[test]
    fn search_content_matches_display_text_case_insensitively() {
        let engine = SearchEngine::new(PathBuf::from("/tmp"));
        let conv = make_conversation("s1", "Fix the Login Bug", None, "/projects/app");
        let groups = vec![make_group(vec![conv])];

        let results = engine.search_content("login bug", &groups);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].conversation.session_id, "s1");
    }

    #[test]
    fn search_content_matches_summary_when_display_does_not_match() {
        let engine = SearchEngine::new(PathBuf::from("/tmp"));
        let conv = make_conversation(
            "s1",
            "Some unrelated display text",
            Some("Refactored the authentication module"),
            "/projects/app",
        );
        let groups = vec![make_group(vec![conv])];

        let results = engine.search_content("authentication", &groups);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].conversation.session_id, "s1");
    }

    #[test]
    fn search_content_skips_archived_conversations() {
        let engine = SearchEngine::new(PathBuf::from("/tmp"));
        let mut conv = make_conversation("s1", "Fix login bug", None, "/projects/app");
        conv.is_archived = true;
        let groups = vec![make_group(vec![conv])];

        let results = engine.search_content("login", &groups);
        assert!(results.is_empty());
    }

    #[test]
    fn search_content_returns_empty_for_no_match() {
        let engine = SearchEngine::new(PathBuf::from("/tmp"));
        let conv = make_conversation("s1", "Fix login bug", None, "/projects/app");
        let groups = vec![make_group(vec![conv])];

        let results = engine.search_content("database migration", &groups);
        assert!(results.is_empty());
    }

    #[test]
    fn search_project_matches_project_paths_case_insensitively() {
        let engine = SearchEngine::new(PathBuf::from("/tmp"));
        let conv = make_conversation("s1", "Some task", None, "/Users/brandon/MyProject");
        let groups = vec![ConversationGroup::Directory {
            parent: "brandon".to_string(),
            project: "MyProject".to_string(),
            conversations: vec![conv],
        }];

        let results = engine.search_project("myproject", &groups);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_project_skips_archived_conversations() {
        let engine = SearchEngine::new(PathBuf::from("/tmp"));
        let mut conv = make_conversation("s1", "Some task", None, "/Users/brandon/MyProject");
        conv.is_archived = true;
        let groups = vec![ConversationGroup::Directory {
            parent: "brandon".to_string(),
            project: "MyProject".to_string(),
            conversations: vec![conv],
        }];

        let results = engine.search_project("myproject", &groups);
        assert!(results.is_empty());
    }

    #[test]
    fn create_snippet_extracts_context_around_match() {
        let text = "This is a long text that contains the word authentication somewhere in the middle of it all";
        let snippet = SearchEngine::create_snippet(text, "authentication");
        assert!(snippet.contains("authentication"));
    }

    #[test]
    fn create_snippet_handles_utf8_text() {
        let text = "Hllo wrld with mji and special chars";
        let snippet = SearchEngine::create_snippet(text, "mji");
        assert!(snippet.contains("mji"));
    }

    #[test]
    fn create_snippet_adds_ellipsis_for_long_text() {
        let text = "a".repeat(100) + "target" + &"b".repeat(100);
        let snippet = SearchEngine::create_snippet(&text, "target");
        assert!(snippet.contains("..."));
        assert!(snippet.contains("target"));
    }

    #[test]
    fn create_snippet_truncates_when_no_match_found() {
        let text = "a".repeat(100);
        let snippet = SearchEngine::create_snippet(&text, "xyz");
        assert!(snippet.ends_with("..."));
        // 60 chars + "..." = 63
        assert_eq!(snippet.len(), 63);
    }

    #[test]
    fn create_snippet_returns_full_text_when_short_and_no_match() {
        let snippet = SearchEngine::create_snippet("short text", "xyz");
        assert_eq!(snippet, "short text");
    }

    #[test]
    fn search_returns_empty_for_empty_query() {
        let engine = SearchEngine::new(PathBuf::from("/tmp"));
        let conv = make_conversation("s1", "Some task", None, "/projects/app");
        let groups = vec![make_group(vec![conv])];

        let query = SearchQuery::new("", SearchFilterType::All);
        let results = engine.search(&query, &groups);
        assert!(results.is_empty());
    }

    #[test]
    fn search_with_whitespace_only_query_returns_empty() {
        let engine = SearchEngine::new(PathBuf::from("/tmp"));
        let conv = make_conversation("s1", "Some task", None, "/projects/app");
        let groups = vec![make_group(vec![conv])];

        let query = SearchQuery::new("   ", SearchFilterType::All);
        let results = engine.search(&query, &groups);
        assert!(results.is_empty());
    }

    #[test]
    fn search_all_deduplicates_results_from_content_and_project() {
        let engine = SearchEngine::new(PathBuf::from("/tmp"));
        // Display text also matches "myproject" - same as project path
        let conv = make_conversation(
            "s1",
            "Working on myproject",
            None,
            "/Users/brandon/myproject",
        );
        let groups = vec![ConversationGroup::Directory {
            parent: "brandon".to_string(),
            project: "myproject".to_string(),
            conversations: vec![conv],
        }];

        let query = SearchQuery::new("myproject", SearchFilterType::All);
        let results = engine.search(&query, &groups);
        // Should only appear once despite matching both content and project
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_content_filter_only_searches_display_and_summary() {
        let engine = SearchEngine::new(PathBuf::from("/tmp"));
        let conv = make_conversation("s1", "Fix bug", None, "/Users/brandon/myproject");
        let groups = vec![ConversationGroup::Directory {
            parent: "brandon".to_string(),
            project: "myproject".to_string(),
            conversations: vec![conv],
        }];

        let query = SearchQuery::new("myproject", SearchFilterType::Content);
        let results = engine.search(&query, &groups);
        // "myproject" is only in the project path, not in display/summary
        assert!(results.is_empty());
    }

    #[test]
    fn search_project_filter_only_searches_project_paths() {
        let engine = SearchEngine::new(PathBuf::from("/tmp"));
        let conv = make_conversation("s1", "Fix login bug", None, "/Users/brandon/app");
        let groups = vec![ConversationGroup::Directory {
            parent: "brandon".to_string(),
            project: "app".to_string(),
            conversations: vec![conv],
        }];

        let query = SearchQuery::new("login", SearchFilterType::Project);
        let results = engine.search(&query, &groups);
        // "login" is only in display, not in project path
        assert!(results.is_empty());
    }
}
