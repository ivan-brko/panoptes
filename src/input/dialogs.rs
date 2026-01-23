//! Confirmation dialog handlers
//!
//! Handles keyboard input for various confirmation dialogs.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{App, InputMode, View};
use crate::focus_timing::store::FocusStore;

/// Handle key when starting a focus timer (entering duration)
pub fn handle_starting_focus_timer_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Only process key press events (not release/repeat)
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    match key.code {
        KeyCode::Esc => {
            // Cancel timer start
            app.state.input_mode = InputMode::Normal;
            app.state.focus_timer_input.clear();
        }
        KeyCode::Enter => {
            // Start the timer with entered duration (or default)
            let minutes = if app.state.focus_timer_input.is_empty() {
                app.config.focus_timer_minutes
            } else {
                app.state
                    .focus_timer_input
                    .parse()
                    .unwrap_or(app.config.focus_timer_minutes)
            };

            app.start_focus_timer(minutes);
            app.state.input_mode = InputMode::Normal;
            app.state.focus_timer_input.clear();
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            // Only allow digits
            if app.state.focus_timer_input.len() < 3 {
                app.state.focus_timer_input.push(c);
            }
        }
        KeyCode::Backspace => {
            app.state.focus_timer_input.pop();
        }
        _ => {}
    }
    Ok(())
}

/// Handle key when confirming focus session deletion
pub fn handle_confirming_focus_session_delete_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            // Confirm deletion
            if let Some(session_id) = app.state.pending_delete_focus_session.take() {
                // Delete from persistent storage
                let store = FocusStore::new();
                if let Err(e) = store.delete_session(session_id) {
                    tracing::error!("Failed to delete focus session: {}", e);
                }

                // Reload sessions from disk to sync state
                app.load_focus_sessions();

                // Adjust selection if needed
                let session_count = app.state.focus_sessions.len();
                if app.state.focus_stats_selected_index >= session_count && session_count > 0 {
                    app.state.focus_stats_selected_index = session_count - 1;
                }
            }
            app.state.input_mode = InputMode::Normal;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            // Cancel deletion
            app.state.pending_delete_focus_session = None;
            app.state.input_mode = InputMode::Normal;
        }
        _ => {}
    }
    Ok(())
}

/// Handle key when viewing focus session details
pub fn handle_viewing_focus_session_detail_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    // Any key closes the detail dialog
    match key.code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
            app.state.viewing_focus_session = None;
            app.state.input_mode = InputMode::Normal;
        }
        _ => {
            // Close on any other key as well
            app.state.viewing_focus_session = None;
            app.state.input_mode = InputMode::Normal;
        }
    }
    Ok(())
}

/// Handle key when confirming session deletion
pub fn handle_confirming_delete_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            // Confirm deletion
            if let Some(session_id) = app.state.pending_delete_session.take() {
                // Get branch_id before destroying (for selection adjustment)
                let branch_id = app.state.view.branch_id();

                if let Err(e) = app.sessions.destroy_session(session_id) {
                    tracing::error!("Failed to destroy session: {}", e);
                }

                // Adjust selection if needed
                if let Some(branch_id) = branch_id {
                    let new_count = app.sessions.sessions_for_branch(branch_id).len();
                    if app.state.selected_session_index >= new_count && new_count > 0 {
                        app.state.selected_session_index = new_count - 1;
                    }
                }
            }
            app.state.input_mode = InputMode::Normal;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            // Cancel deletion
            app.state.pending_delete_session = None;
            app.state.input_mode = InputMode::Normal;
        }
        _ => {}
    }
    Ok(())
}

/// Handle key when confirming branch/worktree deletion
pub fn handle_confirming_branch_delete_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Char('w') => {
            // Toggle worktree deletion option
            app.state.delete_worktree_on_disk = !app.state.delete_worktree_on_disk;
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            // Confirm deletion
            if let Some(branch_id) = app.state.pending_delete_branch.take() {
                // Get branch info before removing
                let branch_info = app.project_store.get_branch(branch_id).cloned();

                // Destroy all sessions for this branch
                let sessions_to_destroy: Vec<_> = app
                    .sessions
                    .sessions_for_branch(branch_id)
                    .iter()
                    .map(|s| s.info.id)
                    .collect();

                for session_id in sessions_to_destroy {
                    if let Err(e) = app.sessions.destroy_session(session_id) {
                        tracing::error!("Failed to destroy session: {}", e);
                    }
                }

                // If user opted to delete worktree on disk
                if app.state.delete_worktree_on_disk {
                    if let Some(branch) = &branch_info {
                        if branch.is_worktree {
                            // Get the project to access the repo
                            if let Some(project_id) = app.state.view.project_id() {
                                // Clone the repo_path to avoid borrow conflicts
                                let repo_path = app
                                    .project_store
                                    .get_project(project_id)
                                    .map(|p| p.repo_path.clone());

                                if let Some(repo_path) = repo_path {
                                    // Show loading indicator
                                    let _ = app.show_loading(&format!(
                                        "Removing worktree '{}'...",
                                        branch.name
                                    ));

                                    match crate::git::GitOps::open(&repo_path) {
                                        Ok(git) => {
                                            if let Err(e) = crate::git::worktree::remove_worktree(
                                                git.repository(),
                                                &branch.name,
                                                true,
                                            ) {
                                                tracing::error!(
                                                    "Failed to remove worktree: {}",
                                                    e
                                                );
                                                app.state.error_message = Some(format!(
                                                    "Failed to remove worktree: {}",
                                                    e
                                                ));
                                            } else {
                                                tracing::info!(
                                                    "Removed worktree for branch: {}",
                                                    branch.name
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!("Failed to open git repo: {}", e);
                                            app.state.error_message =
                                                Some(format!("Failed to open git repo: {}", e));
                                        }
                                    }

                                    app.clear_loading();
                                }
                            }
                        }
                    }
                }

                // Remove branch from the store
                app.project_store.remove_branch(branch_id);

                // Save to disk
                if let Err(e) = app.project_store.save() {
                    tracing::error!("Failed to save project store: {}", e);
                    app.state.error_message =
                        Some(format!("Failed to save project store: {}", e));
                }

                tracing::info!("Deleted branch: {}", branch_id);

                // Adjust selected index if needed
                if let Some(project_id) = app.state.view.project_id() {
                    let new_count = app.project_store.branches_for_project(project_id).len();
                    if app.state.selected_branch_index >= new_count && new_count > 0 {
                        app.state.selected_branch_index = new_count - 1;
                    } else if new_count == 0 {
                        app.state.selected_branch_index = 0;
                    }
                }
            }
            app.state.delete_worktree_on_disk = false;
            app.state.input_mode = InputMode::Normal;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            // Cancel deletion
            app.state.pending_delete_branch = None;
            app.state.delete_worktree_on_disk = false;
            app.state.input_mode = InputMode::Normal;
        }
        _ => {}
    }
    Ok(())
}

/// Handle key when confirming project deletion
pub fn handle_confirming_project_delete_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            // Confirm deletion
            if let Some(project_id) = app.state.pending_delete_project.take() {
                // Destroy all sessions for this project
                let sessions_to_destroy: Vec<_> = app
                    .sessions
                    .sessions_for_project(project_id)
                    .iter()
                    .map(|s| s.info.id)
                    .collect();

                for session_id in sessions_to_destroy {
                    if let Err(e) = app.sessions.destroy_session(session_id) {
                        tracing::error!("Failed to destroy session: {}", e);
                    }
                }

                // Remove project and its branches from the store
                app.project_store.remove_project(project_id);

                // Save to disk
                if let Err(e) = app.project_store.save() {
                    tracing::error!("Failed to save project store: {}", e);
                    app.state.error_message =
                        Some(format!("Failed to save project store: {}", e));
                }

                tracing::info!("Deleted project: {}", project_id);

                // Navigate back to projects overview
                app.state.view = View::ProjectsOverview;

                // Adjust selected index if needed
                let new_project_count = app.project_store.project_count();
                if app.state.selected_project_index >= new_project_count && new_project_count > 0 {
                    app.state.selected_project_index = new_project_count - 1;
                } else if new_project_count == 0 {
                    app.state.selected_project_index = 0;
                }
            }
            app.state.input_mode = InputMode::Normal;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            // Cancel deletion
            app.state.pending_delete_project = None;
            app.state.input_mode = InputMode::Normal;
        }
        _ => {}
    }
    Ok(())
}

/// Handle key while confirming quit
pub fn handle_confirming_quit_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            // Confirm quit
            app.state.should_quit = true;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            // Cancel quit
            app.state.input_mode = InputMode::Normal;
        }
        _ => {}
    }
    Ok(())
}
