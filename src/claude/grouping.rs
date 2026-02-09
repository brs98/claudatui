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
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
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

    /// Get the project key for grouping multiple worktrees/branches under one project.
    ///
    /// - Worktree → repo_path (shared across branches)
    /// - Directory → "dir:{parent}:{project}" from project_path() or key
    /// - Ungrouped → path string
    pub fn project_key(&self) -> String {
        match self {
            Self::Worktree { repo_path, .. } => repo_path.to_string_lossy().into_owned(),
            Self::Directory {
                parent, project, ..
            } => format!("dir:{}:{}", parent, project),
            Self::Ungrouped { path, .. } => path.to_string_lossy().into_owned(),
        }
    }

    /// Get the display name for a project header (parent of worktrees).
    ///
    /// - Worktree → repo name (e.g., "claudatui")
    /// - Directory → "parent/project"
    /// - Ungrouped → file_name of path
    pub fn project_display_name(&self) -> String {
        match self {
            Self::Worktree { repo_path, .. } => repo_path
                .file_name()
                .map(|n| n.to_string_lossy().replace(".git", ""))
                .unwrap_or_else(|| "repo".to_string()),
            Self::Directory {
                parent, project, ..
            } => format!("{}/{}", parent, project),
            Self::Ungrouped { path, .. } => path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
        }
    }

    /// Get the simplified group label when displayed under a ProjectHeader.
    ///
    /// - Worktree → branch name (e.g., "main", "feature-branch")
    /// - Directory → project name
    /// - Ungrouped → file_name of path
    pub fn group_label(&self) -> String {
        match self {
            Self::Worktree { branch, .. } => branch.clone(),
            Self::Directory { project, .. } => project.clone(),
            Self::Ungrouped { path, .. } => path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
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

    /// Returns true if the group's project directory still exists on disk.
    /// Groups with no project path (empty groups) are considered valid.
    pub fn project_path_exists(&self) -> bool {
        self.project_path().is_none_or(|p| p.exists())
    }
}

/// Read the current branch name from a `.git/HEAD` file.
///
/// Returns the branch name for symbolic refs (e.g., `ref: refs/heads/main` → `"main"`),
/// or the first 8 hex chars for a detached HEAD (raw SHA).
/// Returns `None` if the file is unreadable or unparseable.
fn read_git_head_branch(git_dir: &Path) -> Option<String> {
    let head_path = git_dir.join("HEAD");
    let contents = std::fs::read_to_string(head_path).ok()?;
    let trimmed = contents.trim();

    if let Some(ref_path) = trimmed.strip_prefix("ref: ") {
        // Symbolic ref: "ref: refs/heads/<branch>"
        ref_path.strip_prefix("refs/heads/").map(String::from)
    } else if trimmed.len() >= 8 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        // Detached HEAD: raw SHA
        Some(trimmed[..8].to_string())
    } else {
        None
    }
}

/// Check if a `.git/worktrees/` directory exists and has at least one subdirectory.
fn has_worktree_subdirs(git_dir: &Path) -> bool {
    let worktrees_dir = git_dir.join("worktrees");
    match std::fs::read_dir(worktrees_dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .any(|e| e.file_type().is_ok_and(|ft| ft.is_dir())),
        Err(_) => false,
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
                // Navigate up 2 levels to the "git container":
                //   normal → /repo/.git
                //   bare   → /repo.git
                let git_container = gitdir_path.parent().and_then(|p| p.parent());

                if let Some(container) = git_container {
                    let branch = gitdir_path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();

                    let repo_root = if container.file_name().is_some_and(|n| n == ".git") {
                        // Normal repo: container is .git dir, repo root is its parent
                        container.parent()
                    } else if container.join("HEAD").exists() && container.join("refs").is_dir() {
                        // Bare repo: container IS the repo
                        Some(container)
                    } else {
                        None
                    };

                    if let Some(root) = repo_root {
                        return GroupKey::Worktree {
                            repo_path: root.to_path_buf(),
                            branch,
                        };
                    }
                }
            }
        }
    }

    // Pattern 2.5: Main checkout of a non-bare repo with worktrees
    // .git is a directory (not a file), and .git/worktrees/ has subdirectories
    if dot_git.is_dir() && has_worktree_subdirs(&dot_git) {
        if let Some(branch) = read_git_head_branch(&dot_git) {
            return GroupKey::Worktree {
                repo_path: path.to_path_buf(),
                branch,
            };
        }
    }

    // Pattern 3: Regular directory - group by parent/project
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        if let Some(parent_name) = parent.file_name() {
            return GroupKey::Directory {
                parent: parent_name.to_string_lossy().into_owned(),
                project: name.to_string_lossy().into_owned(),
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

/// Remove groups whose project directory no longer exists on disk.
pub fn retain_existing_groups(groups: &mut Vec<ConversationGroup>) {
    groups.retain(ConversationGroup::project_path_exists);
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
    let updated_order: Vec<String> = result.iter().map(ConversationGroup::key).collect();

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
    fn extract_group_key_identifies_bare_worktree_from_dot_git_path() {
        let key = extract_group_key("/Users/brandon/work/repo.git/feature-branch");
        assert!(matches!(
            key,
            GroupKey::Worktree { ref branch, .. } if branch == "feature-branch"
        ));
    }

    #[test]
    fn extract_group_key_parses_regular_directory_into_parent_and_project() {
        let key = extract_group_key("/Users/brandon/personal/myproject");
        assert!(matches!(
            key,
            GroupKey::Directory { ref parent, ref project, .. }
            if parent == "personal" && project == "myproject"
        ));
    }

    #[test]
    fn order_groups_by_keys_preserves_existing_group_order() {
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
        let key_order: Vec<String> = groups.iter().map(ConversationGroup::key).collect();

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
        let result_order: Vec<String> = ordered_groups.iter().map(ConversationGroup::key).collect();
        assert_eq!(result_order, key_order);
    }

    #[test]
    fn order_groups_by_keys_places_new_groups_at_front() {
        // Create conversations for two projects
        let convs = vec![
            make_conversation("/Users/brandon/personal/project-a", 100),
            make_conversation("/Users/brandon/personal/project-b", 200),
        ];

        let groups = group_conversations(convs);
        let key_order: Vec<String> = groups.iter().map(ConversationGroup::key).collect();

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
    fn extract_group_key_returns_ungrouped_for_root_path() {
        let key = extract_group_key("/");
        assert!(matches!(key, GroupKey::Ungrouped { .. }));
    }

    #[test]
    fn extract_group_key_returns_ungrouped_for_path_with_no_parent_name() {
        // A path like "/singledir" has parent "/" which has no file_name
        let key = extract_group_key("/singledir");
        // Parent is "/" which has no file_name(), so falls through to Ungrouped
        assert!(matches!(key, GroupKey::Ungrouped { .. }));
    }

    #[test]
    fn display_name_handles_worktree_with_dot_git_suffix() {
        let group = ConversationGroup::Worktree {
            repo_path: PathBuf::from("/repos/myrepo.git"),
            branch: "feature-branch".to_string(),
            conversations: vec![],
        };
        assert_eq!(group.display_name(), "myrepo (feature-branch)");
    }

    #[test]
    fn display_name_handles_worktree_without_dot_git_suffix() {
        let group = ConversationGroup::Worktree {
            repo_path: PathBuf::from("/repos/myrepo"),
            branch: "main".to_string(),
            conversations: vec![],
        };
        assert_eq!(group.display_name(), "myrepo (main)");
    }

    #[test]
    fn project_path_returns_none_for_empty_group() {
        let group = ConversationGroup::Directory {
            parent: "personal".to_string(),
            project: "empty".to_string(),
            conversations: vec![],
        };
        assert!(group.project_path().is_none());
    }

    #[test]
    fn plan_implementation_conversations_are_conditionally_hidden() {
        use std::collections::HashSet;

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

        // Helper: mirrors is_hidden_plan_implementation logic
        let is_hidden = |c: &&Conversation, running: &HashSet<String>, has_parent: bool| -> bool {
            c.is_plan_implementation && !running.contains(&c.session_id) && has_parent
        };

        // Case 1: Parent session is running (orchestrating) — plan impl hidden
        let mut running = HashSet::new();
        running.insert("session-100".to_string()); // parent is running
        let has_parent = true;

        let filtered: Vec<_> = conversations
            .iter()
            .filter(|c| !is_hidden(c, &running, has_parent))
            .collect();
        assert_eq!(filtered.len(), 2); // Plan impl hidden (orchestrated by parent)

        // Case 2: No parent running (orphaned) — plan impl visible
        let no_running: HashSet<String> = HashSet::new();

        let filtered: Vec<_> = conversations
            .iter()
            .filter(|c| !is_hidden(c, &no_running, false))
            .collect();
        assert_eq!(filtered.len(), 3); // All visible

        // Case 3: User directly activated plan impl (it has its own PTY) — visible
        let mut running = HashSet::new();
        running.insert("session-200".to_string()); // plan impl running directly
        let has_parent = false; // no non-plan session running

        let filtered: Vec<_> = conversations
            .iter()
            .filter(|c| !is_hidden(c, &running, has_parent))
            .collect();
        assert_eq!(filtered.len(), 3); // All visible (plan impl has its own PTY)
    }

    // --- Helper unit tests ---

    #[test]
    fn read_git_head_branch_parses_symbolic_ref() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        assert_eq!(read_git_head_branch(git_dir), Some("main".to_string()));
    }

    #[test]
    fn read_git_head_branch_parses_feature_branch() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/feature/my-branch\n").unwrap();
        assert_eq!(
            read_git_head_branch(git_dir),
            Some("feature/my-branch".to_string())
        );
    }

    #[test]
    fn read_git_head_branch_returns_short_sha_for_detached_head() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path();
        std::fs::write(
            git_dir.join("HEAD"),
            "abc123def456789012345678901234567890abcd\n",
        )
        .unwrap();
        assert_eq!(read_git_head_branch(git_dir), Some("abc123de".to_string()));
    }

    #[test]
    fn read_git_head_branch_returns_none_for_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path();
        std::fs::write(git_dir.join("HEAD"), "not a valid HEAD\n").unwrap();
        assert_eq!(read_git_head_branch(git_dir), None);
    }

    #[test]
    fn read_git_head_branch_returns_none_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_git_head_branch(dir.path()), None);
    }

    #[test]
    fn has_worktree_subdirs_true_when_subdirs_exist() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path();
        let wt_dir = git_dir.join("worktrees").join("feature-branch");
        std::fs::create_dir_all(&wt_dir).unwrap();
        assert!(has_worktree_subdirs(git_dir));
    }

    #[test]
    fn has_worktree_subdirs_false_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path();
        std::fs::create_dir_all(git_dir.join("worktrees")).unwrap();
        assert!(!has_worktree_subdirs(git_dir));
    }

    #[test]
    fn has_worktree_subdirs_false_when_only_files() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path();
        std::fs::create_dir_all(git_dir.join("worktrees")).unwrap();
        std::fs::write(git_dir.join("worktrees").join("somefile"), "data").unwrap();
        assert!(!has_worktree_subdirs(git_dir));
    }

    #[test]
    fn has_worktree_subdirs_false_when_no_worktrees_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!has_worktree_subdirs(dir.path()));
    }

    // --- Pattern 2.5 integration tests ---

    #[test]
    fn extract_group_key_main_checkout_with_worktrees_returns_worktree_group() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("myrepo");
        let git_dir = repo.join(".git");
        std::fs::create_dir_all(git_dir.join("worktrees").join("feature-branch")).unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let key = extract_group_key(&repo.to_string_lossy());
        match key {
            GroupKey::Worktree { repo_path, branch } => {
                assert_eq!(repo_path, repo);
                assert_eq!(branch, "main");
            }
            other => panic!("Expected Worktree, got {:?}", other),
        }
    }

    #[test]
    fn extract_group_key_main_checkout_detached_head_uses_short_sha() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("myrepo");
        let git_dir = repo.join(".git");
        std::fs::create_dir_all(git_dir.join("worktrees").join("feature-branch")).unwrap();
        std::fs::write(
            git_dir.join("HEAD"),
            "abc123def456789012345678901234567890abcd\n",
        )
        .unwrap();

        let key = extract_group_key(&repo.to_string_lossy());
        match key {
            GroupKey::Worktree { branch, .. } => {
                assert_eq!(branch, "abc123de");
            }
            other => panic!("Expected Worktree, got {:?}", other),
        }
    }

    #[test]
    fn extract_group_key_empty_worktrees_dir_falls_through_to_directory() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("parent").join("myrepo");
        let git_dir = repo.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::create_dir_all(git_dir.join("worktrees")).unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let key = extract_group_key(&repo.to_string_lossy());
        assert!(
            matches!(key, GroupKey::Directory { .. }),
            "Expected Directory, got {:?}",
            key
        );
    }

    #[test]
    fn extract_group_key_no_worktrees_dir_falls_through_to_directory() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("parent").join("myrepo");
        let git_dir = repo.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let key = extract_group_key(&repo.to_string_lossy());
        assert!(
            matches!(key, GroupKey::Directory { .. }),
            "Expected Directory, got {:?}",
            key
        );
    }

    #[test]
    fn extract_group_key_garbage_head_falls_through_to_directory() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("parent").join("myrepo");
        let git_dir = repo.join(".git");
        std::fs::create_dir_all(git_dir.join("worktrees").join("feature")).unwrap();
        std::fs::write(git_dir.join("HEAD"), "garbage content\n").unwrap();

        let key = extract_group_key(&repo.to_string_lossy());
        assert!(
            matches!(key, GroupKey::Directory { .. }),
            "Expected Directory, got {:?}",
            key
        );
    }

    #[test]
    fn main_checkout_and_added_worktree_share_same_project_key() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("myrepo");

        // Set up main checkout (.git is a directory with worktrees/)
        let git_dir = repo.join(".git");
        std::fs::create_dir_all(git_dir.join("worktrees").join("feature")).unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        // Set up added worktree (.git is a file pointing back)
        let worktree_path = dir.path().join("feature");
        std::fs::create_dir_all(&worktree_path).unwrap();
        let gitdir_target = git_dir.join("worktrees").join("feature");
        std::fs::write(
            worktree_path.join(".git"),
            format!("gitdir: {}", gitdir_target.display()),
        )
        .unwrap();

        let main_conv = make_conversation(&repo.to_string_lossy(), 100);
        let wt_conv = make_conversation(&worktree_path.to_string_lossy(), 200);

        let groups = group_conversations(vec![main_conv, wt_conv]);

        // Both should be Worktree groups with the same repo_path
        let worktree_groups: Vec<_> = groups
            .iter()
            .filter(|g| matches!(g, ConversationGroup::Worktree { .. }))
            .collect();
        assert_eq!(
            worktree_groups.len(),
            2,
            "Expected 2 Worktree groups, got: {:?}",
            groups
                .iter()
                .map(ConversationGroup::key)
                .collect::<Vec<_>>()
        );

        // Their project_key() should match (same repo_path)
        assert_eq!(
            worktree_groups[0].project_key(),
            worktree_groups[1].project_key(),
            "Main checkout and added worktree should have the same project_key"
        );
    }

    #[test]
    fn extract_group_key_bare_repo_worktree_from_dot_git_file() {
        let dir = tempfile::tempdir().unwrap();

        // Create a bare repo structure: repo.git/ with HEAD + refs/
        let bare_repo = dir.path().join("myrepo.git");
        std::fs::create_dir_all(bare_repo.join("refs")).unwrap();
        std::fs::write(bare_repo.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        // Create a worktrees entry inside the bare repo
        let worktree_gitdir = bare_repo.join("worktrees").join("feature");
        std::fs::create_dir_all(&worktree_gitdir).unwrap();

        // Create the worktree directory with a .git file pointing to the bare repo
        let worktree_dir = dir.path().join("feature");
        std::fs::create_dir_all(&worktree_dir).unwrap();
        std::fs::write(
            worktree_dir.join(".git"),
            format!("gitdir: {}", worktree_gitdir.display()),
        )
        .unwrap();

        let key = extract_group_key(&worktree_dir.to_string_lossy());
        match key {
            GroupKey::Worktree { repo_path, branch } => {
                assert_eq!(repo_path, bare_repo);
                assert_eq!(branch, "feature");
            }
            other => panic!("Expected Worktree with bare repo path, got {:?}", other),
        }
    }
}
