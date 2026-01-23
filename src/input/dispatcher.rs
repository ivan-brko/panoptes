//! Main input dispatch logic
//!
//! Routes keyboard events to appropriate handlers based on current mode.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, InputMode, View};

/// Handle a key event by routing to the appropriate mode handler
pub fn handle_key_event(app: &mut App, key: KeyEvent) -> Result<()> {
    // Handle Ctrl+C specially
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        // In Session mode, fall through to forward Ctrl+C to PTY
        if app.state.input_mode != InputMode::Session {
            // Show warning in all other modes
            app.state.error_message = Some("Ctrl+C disabled. Press 'q' to quit.".to_string());
            return Ok(());
        }
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
        InputMode::AddingProject => super::text_input::handle_adding_project_key(app, key),
        InputMode::AddingProjectName => super::text_input::handle_adding_project_name_key(app, key),
        InputMode::FetchingBranches => {
            // While fetching, only allow Esc to cancel
            if key.code == KeyCode::Esc {
                app.state.input_mode = InputMode::Normal;
            }
            Ok(())
        }
        InputMode::CreatingWorktree => {
            // Need to get project_id from current view
            if let View::ProjectDetail(project_id) = app.state.view {
                app.handle_creating_worktree_key(key, project_id)
            } else {
                Ok(())
            }
        }
        InputMode::SelectingDefaultBase => {
            // Need to get project_id from current view
            if let View::ProjectDetail(project_id) = app.state.view {
                app.handle_selecting_default_base_key(key, project_id)
            } else {
                Ok(())
            }
        }
        InputMode::ConfirmingSessionDelete => super::dialogs::handle_confirming_delete_key(app, key),
        InputMode::ConfirmingBranchDelete => super::dialogs::handle_confirming_branch_delete_key(app, key),
        InputMode::ConfirmingProjectDelete => super::dialogs::handle_confirming_project_delete_key(app, key),
        InputMode::ConfirmingQuit => super::dialogs::handle_confirming_quit_key(app, key),
        InputMode::RenamingProject => super::text_input::handle_renaming_project_key(app, key),
        InputMode::WorktreeSelectBranch => {
            if let View::ProjectDetail(project_id) = app.state.view {
                app.handle_worktree_select_branch_key(key, project_id)
            } else {
                Ok(())
            }
        }
        InputMode::WorktreeSelectBase => {
            if let View::ProjectDetail(project_id) = app.state.view {
                app.handle_worktree_select_base_key(key, project_id)
            } else {
                Ok(())
            }
        }
        InputMode::WorktreeConfirm => {
            if let View::ProjectDetail(project_id) = app.state.view {
                app.handle_worktree_confirm_key(key, project_id)
            } else {
                Ok(())
            }
        }
        InputMode::StartingFocusTimer => super::dialogs::handle_starting_focus_timer_key(app, key),
        InputMode::ConfirmingFocusSessionDelete => {
            super::dialogs::handle_confirming_focus_session_delete_key(app, key)
        }
        InputMode::ViewingFocusSessionDetail => super::dialogs::handle_viewing_focus_session_detail_key(app, key),
    }
}
