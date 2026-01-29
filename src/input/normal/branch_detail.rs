//! Branch detail input handler
//!
//! Handles keyboard input in the branch detail view.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{App, InputMode};
use crate::project::{BranchId, ProjectId};
use crate::session::SessionManager;
use crate::tui::frame::{FrameConfig, FrameLayout};

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

    // Handle focus timer shortcuts (t, T, Ctrl+t)
    if app.handle_focus_timer_shortcut(key) {
        return Ok(());
    }

    let branch_sessions = app.sessions.sessions_for_branch(branch_id);
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
            if let Some(session) = branch_sessions.get(app.state.selected_session_index) {
                let session_id = session.info.id;
                app.state.navigate_to_session(session_id);
                app.tui.enable_mouse_capture();
                app.sessions.acknowledge_attention(session_id);
                if app.config.notification_method == "title" {
                    SessionManager::reset_terminal_title();
                }
                app.resize_active_session_pty()?;
            }
        }
        KeyCode::Char('n') => {
            // Prompt for session name before creating Claude Code session
            if let Some(branch) = app.project_store.get_branch(branch_id) {
                app.state.creating_session_project_id = Some(project_id);
                app.state.creating_session_branch_id = Some(branch_id);
                app.state.creating_session_working_dir = Some(branch.working_dir.clone());
                app.state.new_session_name.clear();
                app.state.input_mode = InputMode::CreatingSession;
            }
        }
        KeyCode::Char('s') => {
            // Prompt for session name before creating shell session
            if let Some(branch) = app.project_store.get_branch(branch_id) {
                app.state.creating_session_project_id = Some(project_id);
                app.state.creating_session_branch_id = Some(branch_id);
                app.state.creating_session_working_dir = Some(branch.working_dir.clone());
                app.state.new_session_name.clear();
                app.state.input_mode = InputMode::CreatingShellSession;
            }
        }
        KeyCode::Char('d') => {
            // Prompt for confirmation before deleting session (use checked access)
            if let Some(session) = branch_sessions.get(app.state.selected_session_index) {
                let session_id = session.info.id;
                app.state.pending_delete_session = Some(session_id);
                app.state.input_mode = InputMode::ConfirmingSessionDelete;
            }
        }
        KeyCode::Char(c) => {
            // Check for custom shortcut trigger
            if let Some(shortcut) = app.config.get_shortcut(c).cloned() {
                if let Some(branch) = app.project_store.get_branch(branch_id) {
                    let working_dir = branch.working_dir.clone();
                    let session_name = shortcut.short_display_name();

                    // Get terminal size
                    let terminal_size = app.tui.size().unwrap_or_default();
                    let frame_config = FrameConfig::default();
                    let layout = FrameLayout::calculate(terminal_size, &frame_config);
                    let rows = layout.content.height as usize;
                    let cols = layout.content.width as usize;

                    // Create shell session with command
                    match app.sessions.create_shell_session_with_command(
                        session_name,
                        working_dir,
                        project_id,
                        branch_id,
                        shortcut.command.clone(),
                        rows,
                        cols,
                    ) {
                        Ok(new_session_id) => {
                            app.state.navigate_to_session(new_session_id);
                            app.tui.enable_mouse_capture();
                            app.state.input_mode = InputMode::Session;
                        }
                        Err(e) => {
                            tracing::error!("Failed to create shell session: {}", e);
                            app.state.error_message =
                                Some(format!("Failed to create session: {}", e));
                        }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}
