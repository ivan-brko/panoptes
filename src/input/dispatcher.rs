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
            app.state.error_message = Some("Ctrl+C disabled. Press 'q' to quit.".to_string());
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

    match app.state.input_mode {
        InputMode::Normal => app.handle_normal_mode_key(key),
        InputMode::Session => super::session_mode::handle_session_mode_key(app, key),
        InputMode::CreatingSession => super::text_input::handle_creating_session_key(app, key),
        InputMode::CreatingShellSession => {
            super::text_input::handle_creating_shell_session_key(app, key)
        }
        InputMode::AddingProject => super::text_input::handle_adding_project_key(app, key),
        InputMode::AddingProjectName => super::text_input::handle_adding_project_name_key(app, key),
        InputMode::CreatingWorktree => {
            // Need to get project_id from current view
            if let View::ProjectDetail(project_id) = app.state.view {
                crate::wizards::worktree::handle_creating_worktree_key(app, key, project_id)
            } else {
                Ok(())
            }
        }
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
        InputMode::StartingFocusTimer => super::dialogs::handle_starting_focus_timer_key(app, key),
        InputMode::ConfirmingFocusSessionDelete => {
            super::dialogs::handle_confirming_focus_session_delete_key(app, key)
        }
        InputMode::ViewingFocusSessionDetail => {
            super::dialogs::handle_viewing_focus_session_detail_key(app, key)
        }
        InputMode::AddingClaudeConfigName => {
            super::text_input::handle_adding_claude_config_name_key(app, key)
        }
        InputMode::AddingClaudeConfigPath => {
            super::text_input::handle_adding_claude_config_path_key(app, key)
        }
        InputMode::ConfirmingClaudeConfigDelete => {
            super::dialogs::handle_confirming_claude_config_delete_key(app, key)
        }
        InputMode::SelectingClaudeConfig => {
            super::text_input::handle_selecting_claude_config_key(app, key)
        }
        InputMode::ConfirmingClaudeSettingsCopy => {
            super::dialogs::handle_confirming_claude_settings_copy_key(app, key)
        }
        InputMode::ConfirmingClaudeSettingsMigrate => {
            super::dialogs::handle_confirming_claude_settings_migrate_key(app, key)
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
            | InputMode::CreatingWorktree
            | InputMode::SelectingDefaultBase,
            View::ProjectDetail(_),
        ) => true,
        (
            InputMode::WorktreeSelectBranch
            | InputMode::WorktreeSelectBase
            | InputMode::WorktreeConfirm
            | InputMode::CreatingWorktree
            | InputMode::SelectingDefaultBase,
            _,
        ) => false,

        // Session delete confirmation only valid in SessionView or BranchDetail
        (InputMode::ConfirmingSessionDelete, View::SessionView) => true,
        (InputMode::ConfirmingSessionDelete, View::BranchDetail(_, _)) => true,
        (InputMode::ConfirmingSessionDelete, _) => false,

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
        let mut state = AppState::default();
        state.input_mode = InputMode::Session;
        state.view = View::SessionView;

        validate_mode_view_consistency(&mut state);

        assert_eq!(state.input_mode, InputMode::Session); // Should remain
    }

    #[test]
    fn test_validate_session_mode_in_other_view_invalid() {
        let mut state = AppState::default();
        state.input_mode = InputMode::Session;
        state.view = View::ProjectsOverview;

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
        let mut state = AppState::default();
        state.input_mode = InputMode::WorktreeSelectBranch;
        state.view = View::ProjectsOverview;

        validate_mode_view_consistency(&mut state);

        assert_eq!(state.input_mode, InputMode::Normal); // Should be reset
    }

    #[test]
    fn test_validate_normal_mode_always_valid() {
        let mut state = AppState::default();
        state.input_mode = InputMode::Normal;

        // Test in various views
        for view in [
            View::ProjectsOverview,
            View::SessionView,
            View::ActivityTimeline,
        ] {
            state.view = view;
            validate_mode_view_consistency(&mut state);
            assert_eq!(state.input_mode, InputMode::Normal);
        }
    }
}
