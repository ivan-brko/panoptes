//! Worktree wizard type definitions
//!
//! Types used for the worktree creation wizard workflow.

/// Type of branch reference (local or remote)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchRefType {
    /// Local branch (e.g., "main")
    Local,
    /// Remote tracking branch (e.g., "origin/main")
    Remote,
}

impl BranchRefType {
    /// Get display prefix for UI
    pub fn prefix(&self) -> &'static str {
        match self {
            BranchRefType::Local => "[L]",
            BranchRefType::Remote => "[R]",
        }
    }
}

/// A reference to a git branch (local or remote)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRef {
    /// Type of branch (local or remote)
    pub ref_type: BranchRefType,
    /// Full reference name (e.g., "main" or "origin/main")
    pub name: String,
    /// Display name for UI
    pub display_name: String,
    /// Whether this is the default base branch
    pub is_default_base: bool,
    /// Whether this branch already has a worktree tracked in Panoptes
    pub is_already_tracked: bool,
    /// Whether this branch has a git worktree that is NOT tracked by Panoptes
    pub has_git_worktree: bool,
}

impl BranchRef {
    /// Create a new branch reference
    pub fn new(ref_type: BranchRefType, name: String) -> Self {
        let display_name = name.clone();
        Self {
            ref_type,
            name,
            display_name,
            is_default_base: false,
            is_already_tracked: false,
            has_git_worktree: false,
        }
    }

    /// Mark this branch as the default base
    pub fn with_default_base(mut self, is_default: bool) -> Self {
        self.is_default_base = is_default;
        self
    }
}

/// Type of worktree creation being performed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorktreeCreationType {
    /// Checkout existing local branch into a worktree
    #[default]
    ExistingLocal,
    /// Create local tracking branch from remote and checkout into worktree
    RemoteTracking,
    /// Create a new branch from a base and checkout into worktree
    NewBranch,
    /// Import an existing git worktree that is not tracked by Panoptes
    ImportExisting,
}

/// Filter branch refs by fuzzy substring match
pub fn filter_branch_refs(branch_refs: &[BranchRef], query: &str) -> Vec<BranchRef> {
    if query.is_empty() {
        return branch_refs.to_vec();
    }
    let query_lower = query.to_lowercase();
    branch_refs
        .iter()
        .filter(|b| b.name.to_lowercase().contains(&query_lower))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_branch_refs() -> Vec<BranchRef> {
        vec![
            BranchRef::new(BranchRefType::Local, "main".to_string()),
            BranchRef::new(BranchRefType::Local, "feature/auth".to_string()),
            BranchRef::new(BranchRefType::Local, "feature/user-profile".to_string()),
            BranchRef::new(BranchRefType::Remote, "origin/main".to_string()),
            BranchRef::new(BranchRefType::Remote, "origin/develop".to_string()),
        ]
    }

    // BranchRefType tests
    #[test]
    fn test_branch_ref_type_prefix() {
        assert_eq!(BranchRefType::Local.prefix(), "[L]");
        assert_eq!(BranchRefType::Remote.prefix(), "[R]");
    }

    // BranchRef tests
    #[test]
    fn test_branch_ref_new() {
        let branch = BranchRef::new(BranchRefType::Local, "main".to_string());
        assert_eq!(branch.ref_type, BranchRefType::Local);
        assert_eq!(branch.name, "main");
        assert_eq!(branch.display_name, "main");
        assert!(!branch.is_default_base);
        assert!(!branch.is_already_tracked);
        assert!(!branch.has_git_worktree);
    }

    #[test]
    fn test_branch_ref_with_default_base() {
        let branch =
            BranchRef::new(BranchRefType::Local, "main".to_string()).with_default_base(true);
        assert!(branch.is_default_base);

        let branch2 = branch.clone().with_default_base(false);
        assert!(!branch2.is_default_base);
    }

    // WorktreeCreationType tests
    #[test]
    fn test_worktree_creation_type_default() {
        let default = WorktreeCreationType::default();
        assert_eq!(default, WorktreeCreationType::ExistingLocal);
    }

    // filter_branch_refs tests
    #[test]
    fn test_filter_branch_refs_empty_query() {
        let branches = create_test_branch_refs();
        let filtered = filter_branch_refs(&branches, "");
        assert_eq!(filtered.len(), branches.len());
    }

    #[test]
    fn test_filter_branch_refs_exact_match() {
        let branches = create_test_branch_refs();
        let filtered = filter_branch_refs(&branches, "main");
        assert_eq!(filtered.len(), 2); // "main" and "origin/main"
        assert!(filtered.iter().any(|b| b.name == "main"));
        assert!(filtered.iter().any(|b| b.name == "origin/main"));
    }

    #[test]
    fn test_filter_branch_refs_partial_match() {
        let branches = create_test_branch_refs();
        let filtered = filter_branch_refs(&branches, "feature");
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|b| b.name.contains("feature")));
    }

    #[test]
    fn test_filter_branch_refs_case_insensitive() {
        let branches = create_test_branch_refs();
        let filtered = filter_branch_refs(&branches, "MAIN");
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|b| b.name == "main"));
    }

    #[test]
    fn test_filter_branch_refs_no_match() {
        let branches = create_test_branch_refs();
        let filtered = filter_branch_refs(&branches, "nonexistent");
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_branch_refs_special_characters() {
        let branches = create_test_branch_refs();
        // Search for "/" which is in "feature/auth" and all remote branches
        let filtered = filter_branch_refs(&branches, "/");
        assert_eq!(filtered.len(), 4); // feature/auth, feature/user-profile, origin/main, origin/develop
    }
}
