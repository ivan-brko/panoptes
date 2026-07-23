//! Branch detail input handler
//!
//! Handles keyboard input in the branch detail view.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{App, InputMode, SessionDraft};
use crate::project::{BranchId, ProjectId};

/// Handle key in branch detail view (normal mode)
pub fn handle_branch_detail_key(
    app: &mut App,
    key: KeyEvent,
    project_id: ProjectId,
    branch_id: BranchId,
) -> Result<()> {
    // Only process key press events (not release/repeat)
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    // Entries mix live sessions with ones recoverable from a previous run, so
    // the selected index may land on a session that has no process attached
    let branch_sessions: Vec<(uuid::Uuid, bool)> = app
        .sessions
        .entries_for_branch(branch_id)
        .iter()
        .map(|entry| (entry.info.id, entry.live))
        .collect();
    let session_count = branch_sessions.len();

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.state.navigate_back();
        }
        KeyCode::Down => {
            app.state.select_next(session_count);
        }
        KeyCode::Up => {
            app.state.select_prev(session_count);
        }
        KeyCode::Enter => {
            // Use checked access to handle potential race conditions
            if let Some(&(session_id, live)) = branch_sessions.get(app.state.selected_session_index)
            {
                // A recovered session has no process yet: bring it back before
                // opening a view onto it
                if !live {
                    match app.resume_recovered_session(session_id) {
                        Ok(true) => {}
                        Ok(false) => return Ok(()),
                        Err(e) => {
                            tracing::error!(
                                session_id = %session_id,
                                error = %e,
                                "Failed to resume session"
                            );
                            // Leave the entry in place so it can be retried or
                            // discarded, and tell the user why
                            app.state.error_message = Some(format!("Could not resume: {}", e));
                            return Ok(());
                        }
                    }
                }
                app.activate_session(session_id)?;
            }
        }
        KeyCode::Char('n') => {
            // Show agent type selector (Claude Code / Codex)
            if let Some(branch) = app.project_store.get_branch(branch_id) {
                app.state.session_draft =
                    SessionDraft::for_branch(project_id, branch_id, branch.working_dir.clone());
                app.state.agent_type_selector_index = 0;
                app.state.input_mode = InputMode::SelectingAgentType;
            }
        }
        KeyCode::Char('s') => {
            // Prompt for session name before creating shell session
            if let Some(branch) = app.project_store.get_branch(branch_id) {
                app.state.session_draft =
                    SessionDraft::for_branch(project_id, branch_id, branch.working_dir.clone());
                app.state.input_mode = InputMode::CreatingShellSession;
            }
        }
        KeyCode::Char('d') => {
            // Prompt for confirmation before deleting session (use checked access)
            if let Some(&(session_id, _)) = branch_sessions.get(app.state.selected_session_index) {
                app.state.pending_delete_session = Some(session_id);
                app.state.input_mode = InputMode::ConfirmingSessionDelete;
            }
        }
        KeyCode::Char(c) => {
            // Check for custom shortcut trigger
            if let Some(shortcut) = app.config.get_shortcut(c).cloned() {
                if let Some(branch) = app.project_store.get_branch(branch_id) {
                    let working_dir = branch.working_dir.clone();
                    if let Some(new_session_id) = super::launch_shortcut_session(
                        app,
                        &shortcut,
                        project_id,
                        branch_id,
                        working_dir,
                    ) {
                        app.state.navigate_to_session(new_session_id);
                        app.tui.enable_mouse_capture();
                        app.state.input_mode = InputMode::Session;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}
