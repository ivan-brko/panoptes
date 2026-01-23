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
        InputMode::CreatingSession => app.handle_creating_session_key(key),
        InputMode::AddingProject => app.handle_adding_project_key(key),
        InputMode::AddingProjectName => app.handle_adding_project_name_key(key),
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
        InputMode::ConfirmingSessionDelete => app.handle_confirming_delete_key(key),
        InputMode::ConfirmingBranchDelete => app.handle_confirming_branch_delete_key(key),
        InputMode::ConfirmingProjectDelete => app.handle_confirming_project_delete_key(key),
        InputMode::ConfirmingQuit => app.handle_confirming_quit_key(key),
        InputMode::RenamingProject => app.handle_renaming_project_key(key),
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
        InputMode::StartingFocusTimer => app.handle_starting_focus_timer_key(key),
        InputMode::ConfirmingFocusSessionDelete => {
            app.handle_confirming_focus_session_delete_key(key)
        }
        InputMode::ViewingFocusSessionDetail => app.handle_viewing_focus_session_detail_key(key),
    }
}
