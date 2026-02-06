use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::claude::conversation::Conversation;

/// How conversations are grouped
#[derive(Debug, Clone)]
pub enum ConversationGroup {
    /// Git worktree grouping
    Worktree {
        repo_path: PathBuf,
        branch: String,
        conversations: Vec<Conversation>,
    },
    /// Regular directory grouping (by parent)
    Directory {
        parent: String,
        project: String,
        conversations: Vec<Conversation>,
    },
    /// Ungrouped/unknown
    Ungrouped {
        path: PathBuf,
        conversations: Vec<Conversation>,
    },
}

impl ConversationGroup {
    /// Get a unique key for this group
    pub fn key(&self) -> String {
        match self {
            Self::Worktree {
                repo_path, branch, ..
            } => {
                format!("worktree:{}:{}", repo_path.display(), branch)
            }
            Self::Directory {
                parent, project, ..
            } => {
                format!("dir:{}:{}", parent, project)
            }
            Self::Ungrouped { path, .. } => {
                format!("ungrouped:{}", path.display())
            }
        }
    }

    /// Get display name for the group
    pub fn display_name(&self) -> String {
        match self {
            Self::Worktree {
                repo_path, branch, ..
            } => {
                let repo_name = repo_path
                    .file_name()
                    .map(|n| n.to_string_lossy().replace(".git", ""))
                    .unwrap_or_else(|| "repo".to_string());
                format!("{} ({})", repo_name, branch)
            }
            Self::Directory {
                parent, project, ..
            } => {
                format!("{}/{}", parent, project)
            }
            Self::Ungrouped { path, .. } => path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string()),
        }
    }

    /// Get conversations in this group
    pub fn conversations(&self) -> &[Conversation] {
        match self {
            Self::Worktree { conversations, .. } => conversations,
            Self::Directory { conversations, .. } => conversations,
            Self::Ungrouped { conversations, .. } => conversations,
        }
    }

    /// Get mutable conversations in this group
    pub fn conversations_mut(&mut self) -> &mut Vec<Conversation> {
        match self {
            Self::Worktree { conversations, .. } => conversations,
            Self::Directory { conversations, .. } => conversations,
            Self::Ungrouped { conversations, .. } => conversations,
        }
    }

    /// Get the project path for this group (for spawning new conversations)
    pub fn project_path(&self) -> Option<PathBuf> {
        match self {
            Self::Worktree { conversations, .. }
            | Self::Directory { conversations, .. }
            | Self::Ungrouped { conversations, .. } => {
                conversations.first().map(|c| c.project_path.clone())
            }
        }
    }
}

/// Extract group info from a project path
fn extract_group_key(project_path: &str) -> GroupKey {
    let path_str = project_path;

    // Pattern 1: Git worktree - /path/to/repo.git/branch-name
    if let Some(git_idx) = path_str.find(".git/") {
        let base = &path_str[..git_idx + 4]; // Include ".git"
        let branch = &path_str[git_idx + 5..]; // After ".git/"
        return GroupKey::Worktree {
            repo_path: PathBuf::from(base),
            branch: branch.to_string(),
        };
    }

    // Pattern 2: Non-bare worktree - .git is a file pointing to parent repo
    let path = Path::new(project_path);
    let dot_git = path.join(".git");
    if dot_git.is_file() {
        if let Ok(contents) = std::fs::read_to_string(&dot_git) {
            if let Some(gitdir) = contents.trim().strip_prefix("gitdir: ") {
                let gitdir_path = PathBuf::from(gitdir);
                // gitdir_path = /repo/.git/worktrees/<name>
                // Navigate up to repo root: worktrees -> .git -> repo
                if let Some(repo_root) = gitdir_path
                    .parent()
                    .and_then(|p| p.parent())
                    .and_then(|p| p.parent())
                {
                    let branch = gitdir_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    return GroupKey::Worktree {
                        repo_path: repo_root.to_path_buf(),
                        branch,
                    };
                }
            }
        }
    }

    // Pattern 3: Regular directory - group by parent/project
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        if let Some(parent_name) = parent.file_name() {
            return GroupKey::Directory {
                parent: parent_name.to_string_lossy().to_string(),
                project: name.to_string_lossy().to_string(),
            };
        }
    }

    GroupKey::Ungrouped {
        path: PathBuf::from(project_path),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum GroupKey {
    Worktree { repo_path: PathBuf, branch: String },
    Directory { parent: String, project: String },
    Ungrouped { path: PathBuf },
}

/// Group conversations by their project paths (with full sorting by recency)
pub fn group_conversations(conversations: Vec<Conversation>) -> Vec<ConversationGroup> {
    let mut groups = group_conversations_unordered(conversations);
    sort_groups_by_recency(&mut groups);
    groups
}

/// Group conversations without sorting groups (caller handles ordering).
/// Conversations within each group are still sorted by timestamp (most recent first).
pub fn group_conversations_unordered(conversations: Vec<Conversation>) -> Vec<ConversationGroup> {
    let mut groups: HashMap<GroupKey, Vec<Conversation>> = HashMap::new();

    for conv in conversations {
        let key = extract_group_key(&conv.project_path.to_string_lossy());
        groups.entry(key).or_default().push(conv);
    }

    let mut result: Vec<ConversationGroup> = groups
        .into_iter()
        .map(|(key, convs)| match key {
            GroupKey::Worktree { repo_path, branch } => ConversationGroup::Worktree {
                repo_path,
                branch,
                conversations: convs,
            },
            GroupKey::Directory { parent, project } => ConversationGroup::Directory {
                parent,
                project,
                conversations: convs,
            },
            GroupKey::Ungrouped { path } => ConversationGroup::Ungrouped {
                path,
                conversations: convs,
            },
        })
        .collect();

    // Sort conversations within each group by timestamp (most recent first)
    for group in &mut result {
        group
            .conversations_mut()
            .sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    }

    result
}

/// Sort groups by most recent conversation (for initial/manual refresh)
pub fn sort_groups_by_recency(groups: &mut [ConversationGroup]) {
    groups.sort_by(|a, b| {
        let a_time = a
            .conversations()
            .iter()
            .map(|c| c.timestamp)
            .max()
            .unwrap_or(0);
        let b_time = b
            .conversations()
            .iter()
            .map(|c| c.timestamp)
            .max()
            .unwrap_or(0);
        b_time.cmp(&a_time)
    });
}

/// Order groups according to a specified key order, with new groups at front.
///
/// Groups that exist in `key_order` are placed in that order.
/// Groups that are new (not in `key_order`) are placed at the front, sorted by recency.
/// Returns the ordered groups and the updated key order.
pub fn order_groups_by_keys(
    mut groups: Vec<ConversationGroup>,
    key_order: &[String],
) -> (Vec<ConversationGroup>, Vec<String>) {
    // Separate new groups (not in key_order) from existing groups
    let key_set: std::collections::HashSet<&String> = key_order.iter().collect();
    let (mut new_groups, existing_groups): (Vec<_>, Vec<_>) =
        groups.drain(..).partition(|g| !key_set.contains(&g.key()));

    // Sort new groups by recency (most recent first)
    sort_groups_by_recency(&mut new_groups);

    // Build a map for quick lookup of existing groups by key
    let mut groups_by_key: HashMap<String, ConversationGroup> =
        existing_groups.into_iter().map(|g| (g.key(), g)).collect();

    // Build result: new groups first (sorted by recency), then existing groups in key_order
    let mut result: Vec<ConversationGroup> = new_groups;
    for key in key_order {
        if let Some(group) = groups_by_key.remove(key) {
            result.push(group);
        }
        // Groups that were in key_order but no longer exist are naturally skipped
    }

    // Build updated key order from result
    let updated_order: Vec<String> = result.iter().map(|g| g.key()).collect();

    (result, updated_order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::conversation::{Conversation, ConversationStatus};

    fn make_conversation(project_path: &str, timestamp: i64) -> Conversation {
        Conversation {
            session_id: format!("session-{}", timestamp),
            display: format!("Conv at {}", timestamp),
            summary: None,
            timestamp,
            modified: format!("2024-01-01T00:00:{}Z", timestamp),
            project_path: PathBuf::from(project_path),
            status: ConversationStatus::Idle,
            message_count: 1,
            git_branch: None,
            is_plan_implementation: false,
            is_archived: false,
            archived_at: None,
        }
    }

    #[test]
    fn test_extract_worktree_group() {
        let key = extract_group_key("/Users/brandon/work/repo.git/feature-branch");
        assert!(matches!(
            key,
            GroupKey::Worktree { ref branch, .. } if branch == "feature-branch"
        ));
    }

    #[test]
    fn test_extract_directory_group() {
        let key = extract_group_key("/Users/brandon/personal/myproject");
        assert!(matches!(
            key,
            GroupKey::Directory { ref parent, ref project, .. }
            if parent == "personal" && project == "myproject"
        ));
    }

    #[test]
    fn test_order_groups_by_keys_preserves_order() {
        // Create conversations for different projects
        let convs = vec![
            make_conversation("/Users/brandon/personal/project-a", 100),
            make_conversation("/Users/brandon/personal/project-b", 200),
            make_conversation("/Users/brandon/personal/project-c", 300),
        ];

        // Group them (will be sorted by recency: c, b, a)
        let groups = group_conversations(convs);
        assert_eq!(groups.len(), 3);

        // Capture the initial order
        let key_order: Vec<String> = groups.iter().map(|g| g.key()).collect();

        // Create new conversations with updated timestamps
        // project-a now has the most recent activity
        let new_convs = vec![
            make_conversation("/Users/brandon/personal/project-a", 500), // Most recent!
            make_conversation("/Users/brandon/personal/project-b", 200),
            make_conversation("/Users/brandon/personal/project-c", 300),
        ];

        // Use unordered grouping + order_by_keys to preserve original order
        let new_groups = group_conversations_unordered(new_convs);
        let (ordered_groups, _) = order_groups_by_keys(new_groups, &key_order);

        // Order should be preserved (c, b, a) even though a has newer activity
        let result_order: Vec<String> = ordered_groups.iter().map(|g| g.key()).collect();
        assert_eq!(result_order, key_order);
    }

    #[test]
    fn test_order_groups_by_keys_new_groups_at_front() {
        // Create conversations for two projects
        let convs = vec![
            make_conversation("/Users/brandon/personal/project-a", 100),
            make_conversation("/Users/brandon/personal/project-b", 200),
        ];

        let groups = group_conversations(convs);
        let key_order: Vec<String> = groups.iter().map(|g| g.key()).collect();

        // Now add a new project
        let new_convs = vec![
            make_conversation("/Users/brandon/personal/project-a", 100),
            make_conversation("/Users/brandon/personal/project-b", 200),
            make_conversation("/Users/brandon/personal/project-new", 150), // New project!
        ];

        let new_groups = group_conversations_unordered(new_convs);
        let (ordered_groups, updated_order) = order_groups_by_keys(new_groups, &key_order);

        // New project should be at front
        assert_eq!(ordered_groups.len(), 3);
        assert!(ordered_groups[0].key().contains("project-new"));

        // Existing projects should maintain their relative order
        assert!(ordered_groups[1].key().contains("project-b"));
        assert!(ordered_groups[2].key().contains("project-a"));

        // Updated order should reflect new state
        assert_eq!(updated_order.len(), 3);
    }

    #[test]
    fn test_plan_implementation_filtering() {
        // Create a mix of regular and plan implementation conversations
        let mut regular = make_conversation("/Users/brandon/personal/project-a", 100);
        regular.display = "Regular conversation".to_string();

        let mut plan_impl = make_conversation("/Users/brandon/personal/project-a", 200);
        plan_impl.display = "Implement the following plan: ...".to_string();
        plan_impl.is_plan_implementation = true;

        let mut another_regular = make_conversation("/Users/brandon/personal/project-a", 300);
        another_regular.display = "Another regular one".to_string();

        let convs = vec![regular, plan_impl, another_regular];
        let groups = group_conversations(convs);

        assert_eq!(groups.len(), 1);
        let conversations = groups[0].conversations();
        assert_eq!(conversations.len(), 3); // All 3 are in the group

        // But when we filter (simulating sidebar behavior):
        let filtered: Vec<_> = conversations
            .iter()
            .filter(|c| !c.is_plan_implementation)
            .collect();
        assert_eq!(filtered.len(), 2); // Only 2 visible
        assert!(!filtered.iter().any(|c| c.is_plan_implementation));
    }
}
