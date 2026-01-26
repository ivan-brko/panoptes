//! Input mode enum
//!
//! Defines how keyboard input is handled based on the current mode.

/// Input mode determines how keyboard input is handled
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    /// Normal mode - keys are handled as commands
    #[default]
    Normal,
    /// Session mode - keys are sent to the active session's PTY
    Session,
    /// Creating a new session - typing session name
    CreatingSession,
    /// Adding a new project - typing path
    AddingProject,
    /// Adding a new project - typing optional name (after path validation)
    AddingProjectName,
    /// Creating a new worktree - typing branch name - DEPRECATED (use WorktreeSelectBranch)
    CreatingWorktree,
    /// Selecting default base branch - DEPRECATED (use WorktreeSelectBase)
    SelectingDefaultBase,
    /// Confirming session deletion
    ConfirmingSessionDelete,
    /// Confirming branch/worktree deletion
    ConfirmingBranchDelete,
    /// Confirming project deletion
    ConfirmingProjectDelete,
    /// Confirming application quit
    ConfirmingQuit,
    /// Renaming a project
    RenamingProject,
    /// Worktree creation Step 1: Search/select existing branch or create new
    WorktreeSelectBranch,
    /// Worktree creation Step 2: Select base branch for new branch
    WorktreeSelectBase,
    /// Worktree creation Step 3: Confirmation before creation
    WorktreeConfirm,
    /// Starting a focus timer - entering duration
    StartingFocusTimer,
    /// Confirming focus session deletion
    ConfirmingFocusSessionDelete,
    /// Viewing focus session details
    ViewingFocusSessionDetail,
    /// Adding a new Claude config - entering name
    AddingClaudeConfigName,
    /// Adding a new Claude config - entering path
    AddingClaudeConfigPath,
    /// Confirming Claude config deletion
    ConfirmingClaudeConfigDelete,
    /// Selecting Claude config for session creation or project default
    SelectingClaudeConfig,
}
