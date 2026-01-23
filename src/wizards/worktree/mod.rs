//! Worktree creation wizard
//!
//! Multi-step workflow for creating git worktrees from branches.

mod handlers;
mod types;

pub use handlers::{
    handle_creating_worktree_key, handle_selecting_default_base_key, handle_worktree_confirm_key,
    handle_worktree_select_base_key, handle_worktree_select_branch_key,
};
pub use types::{filter_branch_refs, BranchRef, BranchRefType, WorktreeCreationType};
