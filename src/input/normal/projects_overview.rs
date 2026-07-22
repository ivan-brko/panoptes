//! Projects overview input handler
//!
//! Handles keyboard input in the projects overview view. The project list is a
//! folder tree flattened into rows, so `selected_project_index` counts folder
//! headings as well as projects.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{App, FolderMoveTarget, HomepageFocus, InputMode, View};
use crate::project::{self, RowRef};
use crate::session::{SessionManager, SessionType};

/// Handle key in projects overview (normal mode)
pub fn handle_projects_overview_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Only process key press events (not release/repeat)
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    let row_count = project::row_count(&app.project_store);
    let session_count = app.sessions.len();
    let both_exist = row_count > 0 && session_count > 0;
    let projects_focused = !both_exist || app.state.homepage_focus == HomepageFocus::Projects;

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.state.input_mode = InputMode::ConfirmingQuit;
        }
        KeyCode::Char('a') => {
            // Activity timeline
            app.state.navigate_to_timeline();
        }
        KeyCode::Char('n') => {
            // Start adding a new project
            app.state.input_mode = InputMode::AddingProject;
            app.state.new_project_path.clear();
        }
        KeyCode::Tab => {
            // Toggle focus between projects and sessions (only when both exist)
            if both_exist {
                app.state.homepage_focus = match app.state.homepage_focus {
                    HomepageFocus::Projects => HomepageFocus::Sessions,
                    HomepageFocus::Sessions => HomepageFocus::Projects,
                };
            }
        }
        KeyCode::Down => {
            if projects_focused && row_count > 0 {
                app.state.selected_project_index =
                    (app.state.selected_project_index + 1) % row_count;
            } else if session_count > 0 {
                app.state.selected_session_index =
                    (app.state.selected_session_index + 1) % session_count;
            }
        }
        KeyCode::Up => {
            if projects_focused && row_count > 0 {
                app.state.selected_project_index = app
                    .state
                    .selected_project_index
                    .checked_sub(1)
                    .unwrap_or(row_count - 1);
            } else if session_count > 0 {
                app.state.selected_session_index = app
                    .state
                    .selected_session_index
                    .checked_sub(1)
                    .unwrap_or(session_count - 1);
            }
        }
        KeyCode::Right => {
            // Expand the selected folder
            if projects_focused {
                if let Some(RowRef::Folder { path, expanded, .. }) = selected_row(app) {
                    if !expanded {
                        set_folder_collapsed(app, &path, false);
                    }
                }
            }
        }
        KeyCode::Left => {
            // Collapse the selected folder, or jump to the parent folder
            if projects_focused {
                match selected_row(app) {
                    Some(RowRef::Folder { path, expanded, .. }) if expanded => {
                        set_folder_collapsed(app, &path, true);
                    }
                    Some(_) => {
                        if let Some(parent) = project::parent_row_index(
                            &app.project_store,
                            app.state.selected_project_index,
                        ) {
                            app.state.selected_project_index = parent;
                        }
                    }
                    None => {}
                }
            }
        }
        KeyCode::Enter => {
            // Open the selected project, toggle the selected folder, or open
            // the selected session, depending on focus
            if projects_focused {
                match selected_row(app) {
                    Some(RowRef::Project(project_id)) => {
                        app.state.navigate_to_project(project_id);
                    }
                    Some(RowRef::Folder { path, expanded, .. }) => {
                        set_folder_collapsed(app, &path, expanded);
                    }
                    None => {}
                }
            } else if session_count > 0 {
                open_selected_session(app)?;
            }
        }
        KeyCode::Char('m') => {
            // Move the selected project or folder into a folder
            if projects_focused {
                let target = match selected_row(app) {
                    Some(RowRef::Project(id)) => Some(FolderMoveTarget::Project(id)),
                    Some(RowRef::Folder { path, .. }) => Some(FolderMoveTarget::Folder(path)),
                    None => None,
                };
                if let Some(target) = target {
                    // Prefill with the current location so nudging one level is easy
                    app.state.folder_input = match &target {
                        FolderMoveTarget::Project(id) => app
                            .project_store
                            .get_project(*id)
                            .map(|p| crate::project::folder_path_key(&p.folder))
                            .unwrap_or_default(),
                        FolderMoveTarget::Folder(path) => {
                            crate::project::folder_path_key(&path[..path.len() - 1])
                        }
                    };
                    app.state.moving_to_folder = Some(target);
                    app.state.folder_error = None;
                    app.state.input_mode = InputMode::MovingToFolder;
                }
            }
        }
        KeyCode::Char('r') => {
            // Rename the selected folder
            if projects_focused {
                if let Some(RowRef::Folder { path, .. }) = selected_row(app) {
                    app.state.folder_input = path.last().cloned().unwrap_or_default();
                    app.state.renaming_folder = Some(path);
                    app.state.folder_error = None;
                    app.state.input_mode = InputMode::RenamingFolder;
                }
            }
        }
        KeyCode::Char('d') => {
            // Delete from the currently focused list
            if projects_focused {
                match selected_row(app) {
                    Some(RowRef::Project(project_id)) => {
                        app.state.pending_delete_project = Some(project_id);
                        app.state.input_mode = InputMode::ConfirmingProjectDelete;
                    }
                    Some(RowRef::Folder { path, .. }) => {
                        app.state.pending_remove_folder = Some(path);
                        app.state.input_mode = InputMode::ConfirmingFolderRemove;
                    }
                    None => {}
                }
            } else if session_count > 0 {
                destroy_selected_session(app);
            }
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            // The project tree is not numbered, so digits would mean counting
            // rows by hand. The sessions list still shows numbers, so digit
            // selection stays there.
            if !projects_focused {
                if let Some(num) = c.to_digit(10) {
                    let num = num as usize;
                    if num > 0 && num <= session_count {
                        app.state.selected_session_index = num - 1;
                    }
                }
            }
        }
        KeyCode::Char('l') => {
            // Open log viewer
            app.state.view = View::LogViewer;
            app.state.log_viewer_scroll = 0;
            app.state.log_viewer_auto_scroll = true;
        }
        KeyCode::Char('c') => {
            // Open Claude configs management
            app.state.navigate_to_claude_configs();
        }
        KeyCode::Char('x') => {
            // Open Codex configs management
            app.state.navigate_to_codex_configs();
        }
        KeyCode::Char('R') => {
            // Refresh git state for all projects
            app.refresh_all_git_state();
            app.state
                .header_notifications
                .push("Git state refreshed".to_string());
        }
        _ => {}
    }
    Ok(())
}

/// Snapshot of the currently selected tree row
fn selected_row(app: &App) -> Option<RowRef> {
    project::row_at(&app.project_store, app.state.selected_project_index)
}

/// Collapse or expand a folder and persist the change
fn set_folder_collapsed(app: &mut App, path: &[String], collapsed: bool) {
    app.project_store.set_folder_collapsed(path, collapsed);
    if let Err(e) = app.project_store.save() {
        tracing::warn!("Failed to persist folder collapse state: {}", e);
    }
    // Collapsing removes rows, so the selection may now point past the end
    let row_count = project::row_count(&app.project_store);
    if app.state.selected_project_index >= row_count {
        app.state.selected_project_index = row_count.saturating_sub(1);
    }
}

/// Open the session currently selected in the sessions list
fn open_selected_session(app: &mut App) -> Result<()> {
    let Some(session) = app.sessions.get_by_index(app.state.selected_session_index) else {
        return Ok(());
    };
    let session_id = session.info.id;
    let is_codex = session.info.session_type == SessionType::OpenAICodex;

    app.state.navigate_to_session(session_id);
    if is_codex {
        app.tui.enable_mouse_capture();
    }
    app.sessions.acknowledge_attention(session_id);
    if app.config.notification_method == "title" {
        SessionManager::reset_terminal_title();
    }
    app.resize_active_session_pty()
}

/// Destroy the session currently selected in the sessions list
fn destroy_selected_session(app: &mut App) {
    let Some(session) = app.sessions.get_by_index(app.state.selected_session_index) else {
        return;
    };
    let id = session.info.id;

    // Clear active_session if it was the destroyed session
    if app.state.active_session == Some(id) {
        app.state.active_session = None;
    }
    if let Err(e) = app.sessions.destroy_session(id) {
        tracing::error!("Failed to destroy session: {}", e);
    }
    let new_count = app.sessions.len();
    if app.state.selected_session_index >= new_count && app.state.selected_session_index > 0 {
        app.state.selected_session_index -= 1;
    }
}
