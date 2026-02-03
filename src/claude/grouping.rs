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
            Self::Worktree { repo_path, branch, .. } => {
                format!("worktree:{}:{}", repo_path.display(), branch)
            }
            Self::Directory { parent, project, .. } => {
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
            Self::Worktree { repo_path, branch, .. } => {
                let repo_name = repo_path
                    .file_name()
                    .map(|n| n.to_string_lossy().replace(".git", ""))
                    .unwrap_or_else(|| "repo".to_string());
                format!("{} ({})", repo_name, branch)
            }
            Self::Directory { parent, project, .. } => {
                format!("{}/{}", parent, project)
            }
            Self::Ungrouped { path, .. } => {
                path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string_lossy().to_string())
            }
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

    // Pattern 2: Regular directory - group by parent/project
    let path = Path::new(project_path);
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

/// Group conversations by their project paths
pub fn group_conversations(conversations: Vec<Conversation>) -> Vec<ConversationGroup> {
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

    // Sort groups by most recent conversation
    result.sort_by(|a, b| {
        let a_time = a.conversations().iter().map(|c| c.timestamp).max().unwrap_or(0);
        let b_time = b.conversations().iter().map(|c| c.timestamp).max().unwrap_or(0);
        b_time.cmp(&a_time)
    });

    // Sort conversations within each group by timestamp
    for group in &mut result {
        group.conversations_mut().sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
