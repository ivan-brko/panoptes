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
    /// Creating a new Claude Code session - typing session name
    CreatingSession,
    /// Creating a new shell session - typing session name
    CreatingShellSession,
    /// Adding a new project - typing path
    AddingProject,
    /// Adding a new project - typing optional name (after path validation)
    AddingProjectName,
    /// Selecting the project's default base branch (via 'b' in project view)
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
    /// Moving a project or folder into a folder - typing the folder path
    MovingToFolder,
    /// Renaming a folder in the projects overview
    RenamingFolder,
    /// Confirming that a folder should be dissolved (contents move up a level)
    ConfirmingFolderRemove,
    /// Worktree creation Step 1: Search/select existing branch or create new
    WorktreeSelectBranch,
    /// Worktree creation Step 2: Select base branch for new branch
    WorktreeSelectBase,
    /// Worktree creation Step 3: Confirmation before creation
    WorktreeConfirm,
    /// Adding a new Claude config - entering name
    AddingClaudeConfigName,
    /// Adding a new Claude config - entering path
    AddingClaudeConfigPath,
    /// Confirming Claude config deletion
    ConfirmingClaudeConfigDelete,
    /// Selecting Claude config for session creation or project default
    SelectingClaudeConfig,
    /// Confirming Claude settings copy after worktree creation
    ConfirmingClaudeSettingsCopy,
    /// Confirming Claude settings migration before worktree deletion
    ConfirmingClaudeSettingsMigrate,
    /// Managing custom shell shortcuts - list view with add/delete
    ManagingCustomShortcuts,
    /// Adding a custom shortcut - capturing the key
    AddingCustomShortcutKey,
    /// Adding a custom shortcut - entering the name
    AddingCustomShortcutName,
    /// Adding a custom shortcut - entering the command
    AddingCustomShortcutCommand,
    /// Adding a custom shortcut - toggling auto-close option
    AddingCustomShortcutAutoClose,
    /// Confirming custom shortcut deletion
    ConfirmingCustomShortcutDelete,
    /// Selecting agent type (Claude Code vs Codex) for new session
    SelectingAgentType,
    /// Creating a new Codex session - typing session name
    CreatingCodexSession,
    /// Adding a new Codex config - entering name
    AddingCodexConfigName,
    /// Adding a new Codex config - entering path
    AddingCodexConfigPath,
    /// Confirming Codex config deletion
    ConfirmingCodexConfigDelete,
    /// Selecting Codex config for session creation or project default
    SelectingCodexConfig,
}

impl InputMode {
    /// Every input mode, for table-driven tests
    ///
    /// Keep in sync with the enum; `test_all_lists_every_mode_once` fails if
    /// an entry is duplicated, and the dispatcher's routing-table test fails
    /// to compile if a new variant is missing from its match.
    pub const ALL: [InputMode; 36] = [
        InputMode::Normal,
        InputMode::Session,
        InputMode::CreatingSession,
        InputMode::CreatingShellSession,
        InputMode::AddingProject,
        InputMode::AddingProjectName,
        InputMode::SelectingDefaultBase,
        InputMode::ConfirmingSessionDelete,
        InputMode::ConfirmingBranchDelete,
        InputMode::ConfirmingProjectDelete,
        InputMode::ConfirmingQuit,
        InputMode::RenamingProject,
        InputMode::MovingToFolder,
        InputMode::RenamingFolder,
        InputMode::ConfirmingFolderRemove,
        InputMode::WorktreeSelectBranch,
        InputMode::WorktreeSelectBase,
        InputMode::WorktreeConfirm,
        InputMode::AddingClaudeConfigName,
        InputMode::AddingClaudeConfigPath,
        InputMode::ConfirmingClaudeConfigDelete,
        InputMode::SelectingClaudeConfig,
        InputMode::ConfirmingClaudeSettingsCopy,
        InputMode::ConfirmingClaudeSettingsMigrate,
        InputMode::ManagingCustomShortcuts,
        InputMode::AddingCustomShortcutKey,
        InputMode::AddingCustomShortcutName,
        InputMode::AddingCustomShortcutCommand,
        InputMode::AddingCustomShortcutAutoClose,
        InputMode::ConfirmingCustomShortcutDelete,
        InputMode::SelectingAgentType,
        InputMode::CreatingCodexSession,
        InputMode::AddingCodexConfigName,
        InputMode::AddingCodexConfigPath,
        InputMode::ConfirmingCodexConfigDelete,
        InputMode::SelectingCodexConfig,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_lists_every_mode_once() {
        let mut seen: Vec<String> = InputMode::ALL.iter().map(|m| format!("{:?}", m)).collect();
        seen.sort();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len(), "InputMode::ALL contains a duplicate");
    }
}
