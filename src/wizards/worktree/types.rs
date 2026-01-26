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
