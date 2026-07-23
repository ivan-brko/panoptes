//! Main input dispatch logic
//!
//! Routes keyboard events to appropriate handlers based on current mode.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, AppState, InputMode, View};

/// Handle a key event by routing to the appropriate mode handler
pub fn handle_key_event(app: &mut App, key: KeyEvent) -> Result<()> {
    // Validate mode/view consistency before dispatch
    validate_mode_view_consistency(&mut app.state);

    // Handle help overlay first - it captures all keys when visible
    if app.state.show_help_overlay {
        match key.code {
            KeyCode::Char('?') | KeyCode::Esc => {
                app.state.show_help_overlay = false;
            }
            _ => {} // Ignore other keys while overlay is visible
        }
        return Ok(());
    }

    // Handle Ctrl+C specially
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        // In Session mode, fall through to forward Ctrl+C to PTY
        if app.state.input_mode != InputMode::Session {
            // Show warning in all other modes
            app.state.error_message =
                Some("Ctrl+C disabled. Press 'q' or Esc to quit.".to_string());
            return Ok(());
        }
    }

    // Toggle help overlay with ? key in Normal mode
    if key.code == KeyCode::Char('?') && app.state.input_mode == InputMode::Normal {
        app.state.show_help_overlay = true;
        return Ok(());
    }

    // Global: Jump to next session needing attention (Space key)
    // Only works in Normal mode (not in text input modes or Session mode)
    if key.code == KeyCode::Char(' ') && app.state.input_mode == InputMode::Normal {
        return app.jump_to_next_attention();
    }

    // Global: Open custom shortcuts management dialog (k key)
    // Works in Normal mode across all views except Session mode
    if key.code == KeyCode::Char('k') && app.state.input_mode == InputMode::Normal {
        app.state.custom_shortcuts_selected = 0;
        app.state.input_mode = InputMode::ManagingCustomShortcuts;
        return Ok(());
    }

    use super::agent_configs::{self, AgentKind};

    match app.state.input_mode {
        InputMode::Normal => app.handle_normal_mode_key(key),
        InputMode::Session => super::session_mode::handle_session_mode_key(app, key),
        InputMode::CreatingSession => {
            agent_configs::handle_creating_agent_session_key(app, key, AgentKind::Claude)
        }
        InputMode::CreatingShellSession => {
            super::text_input::handle_creating_shell_session_key(app, key)
        }
        InputMode::AddingProject => super::text_input::handle_adding_project_key(app, key),
        InputMode::AddingProjectName => super::text_input::handle_adding_project_name_key(app, key),
        InputMode::SelectingDefaultBase => {
            // Need to get project_id from current view
            if let View::ProjectDetail(project_id) = app.state.view {
                crate::wizards::worktree::handle_selecting_default_base_key(app, key, project_id)
            } else {
                Ok(())
            }
        }
        InputMode::ConfirmingSessionDelete => {
            super::dialogs::handle_confirming_delete_key(app, key)
        }
        InputMode::ConfirmingBranchDelete => {
            super::dialogs::handle_confirming_branch_delete_key(app, key)
        }
        InputMode::ConfirmingProjectDelete => {
            super::dialogs::handle_confirming_project_delete_key(app, key)
        }
        InputMode::ConfirmingQuit => super::dialogs::handle_confirming_quit_key(app, key),
        InputMode::RenamingProject => super::text_input::handle_renaming_project_key(app, key),
        InputMode::WorktreeSelectBranch => {
            if let View::ProjectDetail(project_id) = app.state.view {
                crate::wizards::worktree::handle_worktree_select_branch_key(app, key, project_id)
            } else {
                Ok(())
            }
        }
        InputMode::WorktreeSelectBase => {
            if let View::ProjectDetail(project_id) = app.state.view {
                crate::wizards::worktree::handle_worktree_select_base_key(app, key, project_id)
            } else {
                Ok(())
            }
        }
        InputMode::WorktreeConfirm => {
            if let View::ProjectDetail(project_id) = app.state.view {
                crate::wizards::worktree::handle_worktree_confirm_key(app, key, project_id)
            } else {
                Ok(())
            }
        }
        InputMode::AddingClaudeConfigName => {
            agent_configs::handle_adding_config_name_key(app, key, AgentKind::Claude)
        }
        InputMode::AddingClaudeConfigPath => {
            agent_configs::handle_adding_config_path_key(app, key, AgentKind::Claude)
        }
        InputMode::ConfirmingClaudeConfigDelete => {
            agent_configs::handle_confirming_config_delete_key(app, key, AgentKind::Claude)
        }
        InputMode::SelectingClaudeConfig => {
            agent_configs::handle_selecting_config_key(app, key, AgentKind::Claude)
        }
        InputMode::ConfirmingClaudeSettingsCopy => {
            super::dialogs::handle_confirming_claude_settings_copy_key(app, key)
        }
        InputMode::ConfirmingClaudeSettingsMigrate => {
            super::dialogs::handle_confirming_claude_settings_migrate_key(app, key)
        }
        InputMode::ManagingCustomShortcuts => {
            super::dialogs::handle_managing_custom_shortcuts_key(app, key)
        }
        InputMode::AddingCustomShortcutKey => {
            super::dialogs::handle_adding_custom_shortcut_key_key(app, key)
        }
        InputMode::AddingCustomShortcutName => {
            super::dialogs::handle_adding_custom_shortcut_name_key(app, key)
        }
        InputMode::AddingCustomShortcutCommand => {
            super::dialogs::handle_adding_custom_shortcut_command_key(app, key)
        }
        InputMode::AddingCustomShortcutAutoClose => {
            super::dialogs::handle_adding_custom_shortcut_auto_close_key(app, key)
        }
        InputMode::ConfirmingCustomShortcutDelete => {
            super::dialogs::handle_confirming_custom_shortcut_delete_key(app, key)
        }
        InputMode::SelectingAgentType => {
            super::text_input::handle_selecting_agent_type_key(app, key)
        }
        InputMode::CreatingCodexSession => {
            agent_configs::handle_creating_agent_session_key(app, key, AgentKind::Codex)
        }
        InputMode::AddingCodexConfigName => {
            agent_configs::handle_adding_config_name_key(app, key, AgentKind::Codex)
        }
        InputMode::AddingCodexConfigPath => {
            agent_configs::handle_adding_config_path_key(app, key, AgentKind::Codex)
        }
        InputMode::ConfirmingCodexConfigDelete => {
            agent_configs::handle_confirming_config_delete_key(app, key, AgentKind::Codex)
        }
        InputMode::SelectingCodexConfig => {
            agent_configs::handle_selecting_config_key(app, key, AgentKind::Codex)
        }
        InputMode::MovingToFolder => super::text_input::handle_moving_to_folder_key(app, key),
        InputMode::RenamingFolder => super::text_input::handle_renaming_folder_key(app, key),
        InputMode::ConfirmingFolderRemove => {
            super::dialogs::handle_confirming_folder_remove_key(app, key)
        }
    }
}

/// Validate that the current InputMode is valid for the current View.
///
/// If an invalid combination is detected, reset to Normal mode to prevent
/// UI state corruption.
fn validate_mode_view_consistency(state: &mut AppState) {
    let is_valid = match (&state.input_mode, &state.view) {
        // Session mode only valid in SessionView
        (InputMode::Session, view) => *view == View::SessionView,

        // Worktree wizard modes only valid in ProjectDetail
        (
            InputMode::WorktreeSelectBranch
            | InputMode::WorktreeSelectBase
            | InputMode::WorktreeConfirm
            | InputMode::SelectingDefaultBase,
            View::ProjectDetail(_),
        ) => true,
        (
            InputMode::WorktreeSelectBranch
            | InputMode::WorktreeSelectBase
            | InputMode::WorktreeConfirm
            | InputMode::SelectingDefaultBase,
            _,
        ) => false,

        // Session delete confirmation only valid in SessionView or BranchDetail
        (InputMode::ConfirmingSessionDelete, View::SessionView) => true,
        (InputMode::ConfirmingSessionDelete, View::BranchDetail(_, _)) => true,
        (InputMode::ConfirmingSessionDelete, _) => false,

        // Folder organization only happens in the projects overview
        (
            InputMode::MovingToFolder
            | InputMode::RenamingFolder
            | InputMode::ConfirmingFolderRemove,
            view,
        ) => *view == View::ProjectsOverview,

        // Branch delete confirmation only valid in ProjectDetail or BranchDetail
        (InputMode::ConfirmingBranchDelete, View::ProjectDetail(_)) => true,
        (InputMode::ConfirmingBranchDelete, View::BranchDetail(_, _)) => true,
        (InputMode::ConfirmingBranchDelete, _) => false,

        // All other combinations are assumed valid
        _ => true,
    };

    if !is_valid {
        tracing::warn!(
            "Mode/view mismatch detected: {:?} in {:?}, resetting to Normal",
            state.input_mode,
            state.view
        );
        state.input_mode = InputMode::Normal;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_validate_session_mode_in_session_view_valid() {
        let mut state = AppState {
            input_mode: InputMode::Session,
            view: View::SessionView,
            ..Default::default()
        };

        validate_mode_view_consistency(&mut state);

        assert_eq!(state.input_mode, InputMode::Session); // Should remain
    }

    #[test]
    fn test_validate_session_mode_in_other_view_invalid() {
        let mut state = AppState {
            input_mode: InputMode::Session,
            view: View::ProjectsOverview,
            ..Default::default()
        };

        validate_mode_view_consistency(&mut state);

        assert_eq!(state.input_mode, InputMode::Normal); // Should be reset
    }

    #[test]
    fn test_validate_worktree_mode_in_project_detail_valid() {
        let mut state = AppState::default();
        let project_id = Uuid::new_v4();
        state.input_mode = InputMode::WorktreeSelectBranch;
        state.view = View::ProjectDetail(project_id);

        validate_mode_view_consistency(&mut state);

        assert_eq!(state.input_mode, InputMode::WorktreeSelectBranch); // Should remain
    }

    #[test]
    fn test_validate_worktree_mode_in_other_view_invalid() {
        let mut state = AppState {
            input_mode: InputMode::WorktreeSelectBranch,
            view: View::ProjectsOverview,
            ..Default::default()
        };

        validate_mode_view_consistency(&mut state);

        assert_eq!(state.input_mode, InputMode::Normal); // Should be reset
    }

    #[test]
    fn test_validate_normal_mode_always_valid() {
        let mut state = AppState {
            input_mode: InputMode::Normal,
            ..Default::default()
        };

        // Test in various views
        for view in [View::ProjectsOverview, View::SessionView, View::LogViewer] {
            state.view = view;
            validate_mode_view_consistency(&mut state);
            assert_eq!(state.input_mode, InputMode::Normal);
        }
    }

    // ------------------------------------------------------------------
    // Routing table
    // ------------------------------------------------------------------

    use crate::input::agent_configs::AgentKind;

    /// The handler family each mode routes to in [`handle_key_event`]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum HandlerFamily {
        /// Per-view normal-mode handlers
        Normal,
        /// Keys forwarded to the active session's PTY
        Session,
        /// Text-input handlers in `input::text_input`
        TextInput,
        /// Dialog handlers in `input::dialogs`
        Dialog,
        /// Worktree wizard handlers in `wizards::worktree` (need a
        /// ProjectDetail view for their project ID)
        Wizard,
        /// Shared Claude/Codex config handlers in `input::agent_configs`
        AgentConfig(AgentKind),
    }

    /// Mirror of the dispatcher's match, kept exhaustive on purpose:
    /// adding an `InputMode` variant without deciding its routing here is a
    /// compile error, and `test_every_mode_reaches_intended_handler_family`
    /// then asserts the properties that matter.
    fn handler_family(mode: InputMode) -> HandlerFamily {
        use HandlerFamily::*;
        match mode {
            InputMode::Normal => Normal,
            InputMode::Session => Session,
            InputMode::CreatingShellSession
            | InputMode::AddingProject
            | InputMode::AddingProjectName
            | InputMode::RenamingProject
            | InputMode::MovingToFolder
            | InputMode::RenamingFolder
            | InputMode::SelectingAgentType => TextInput,
            InputMode::ConfirmingSessionDelete
            | InputMode::ConfirmingBranchDelete
            | InputMode::ConfirmingProjectDelete
            | InputMode::ConfirmingQuit
            | InputMode::ConfirmingFolderRemove
            | InputMode::ConfirmingClaudeSettingsCopy
            | InputMode::ConfirmingClaudeSettingsMigrate
            | InputMode::ManagingCustomShortcuts
            | InputMode::AddingCustomShortcutKey
            | InputMode::AddingCustomShortcutName
            | InputMode::AddingCustomShortcutCommand
            | InputMode::AddingCustomShortcutAutoClose
            | InputMode::ConfirmingCustomShortcutDelete => Dialog,
            InputMode::SelectingDefaultBase
            | InputMode::WorktreeSelectBranch
            | InputMode::WorktreeSelectBase
            | InputMode::WorktreeConfirm => Wizard,
            InputMode::CreatingSession
            | InputMode::AddingClaudeConfigName
            | InputMode::AddingClaudeConfigPath
            | InputMode::ConfirmingClaudeConfigDelete
            | InputMode::SelectingClaudeConfig => AgentConfig(AgentKind::Claude),
            InputMode::CreatingCodexSession
            | InputMode::AddingCodexConfigName
            | InputMode::AddingCodexConfigPath
            | InputMode::ConfirmingCodexConfigDelete
            | InputMode::SelectingCodexConfig => AgentConfig(AgentKind::Codex),
        }
    }

    #[test]
    fn test_every_mode_reaches_intended_handler_family() {
        // Every mode has a routing decision (the match above is exhaustive,
        // so this mostly guards InputMode::ALL against going stale)
        for &mode in &InputMode::ALL {
            let _ = handler_family(mode);
        }

        // The merged Claude/Codex flows must route to the shared handlers
        // with the right agent kind - this is the regression the merge could
        // introduce silently
        let pairs = [
            (InputMode::CreatingSession, InputMode::CreatingCodexSession),
            (
                InputMode::AddingClaudeConfigName,
                InputMode::AddingCodexConfigName,
            ),
            (
                InputMode::AddingClaudeConfigPath,
                InputMode::AddingCodexConfigPath,
            ),
            (
                InputMode::SelectingClaudeConfig,
                InputMode::SelectingCodexConfig,
            ),
            (
                InputMode::ConfirmingClaudeConfigDelete,
                InputMode::ConfirmingCodexConfigDelete,
            ),
        ];
        for (claude_mode, codex_mode) in pairs {
            assert_eq!(
                handler_family(claude_mode),
                HandlerFamily::AgentConfig(AgentKind::Claude)
            );
            assert_eq!(
                handler_family(codex_mode),
                HandlerFamily::AgentConfig(AgentKind::Codex)
            );
        }

        // Wizard modes carry a project ID taken from the current view, so
        // they must be exactly the modes the view-consistency check pins to
        // ProjectDetail
        for &mode in &InputMode::ALL {
            let is_wizard = handler_family(mode) == HandlerFamily::Wizard;
            let mut state = AppState {
                input_mode: mode,
                view: View::LogViewer,
                ..Default::default()
            };
            validate_mode_view_consistency(&mut state);
            if is_wizard {
                assert_eq!(
                    state.input_mode,
                    InputMode::Normal,
                    "{mode:?} must be reset outside ProjectDetail"
                );
            }
        }
    }

    #[test]
    fn test_mode_view_consistency_covers_all_modes() {
        let project_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();
        let views = [
            View::ProjectsOverview,
            View::ProjectDetail(project_id),
            View::BranchDetail(project_id, branch_id),
            View::SessionView,
            View::LogViewer,
            View::ClaudeConfigs,
            View::CodexConfigs,
        ];

        // Which views each mode may appear in; wildcard = every view
        let allowed = |mode: InputMode, view: View| -> bool {
            match mode {
                InputMode::Session => view == View::SessionView,
                InputMode::WorktreeSelectBranch
                | InputMode::WorktreeSelectBase
                | InputMode::WorktreeConfirm
                | InputMode::SelectingDefaultBase => {
                    matches!(view, View::ProjectDetail(_))
                }
                InputMode::ConfirmingSessionDelete => {
                    view == View::SessionView || matches!(view, View::BranchDetail(_, _))
                }
                InputMode::ConfirmingBranchDelete => {
                    matches!(view, View::ProjectDetail(_) | View::BranchDetail(_, _))
                }
                InputMode::MovingToFolder
                | InputMode::RenamingFolder
                | InputMode::ConfirmingFolderRemove => view == View::ProjectsOverview,
                _ => true,
            }
        };

        for &mode in &InputMode::ALL {
            for &view in &views {
                let mut state = AppState {
                    input_mode: mode,
                    view,
                    ..Default::default()
                };
                validate_mode_view_consistency(&mut state);
                let expected = if allowed(mode, view) {
                    mode
                } else {
                    InputMode::Normal
                };
                assert_eq!(state.input_mode, expected, "mode {mode:?} in view {view:?}");
            }
        }
    }
}
