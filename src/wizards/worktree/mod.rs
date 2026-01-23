//! Worktree creation wizard
//!
//! Multi-step workflow for creating git worktrees from branches.

mod types;

pub use types::{filter_branch_refs, BranchRef, BranchRefType, WorktreeCreationType};
