//! Projects overview input handler
//!
//! Handles keyboard input in the projects overview view.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{App, HomepageFocus, InputMode, View};
use crate::session::SessionManager;

/// Handle key in projects overview (normal mode)
pub fn handle_projects_overview_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Only process key press events (not release/repeat)
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    // Handle focus timer shortcuts (t, T, Ctrl+t)
    if app.handle_focus_timer_shortcut(key) {
        return Ok(());
    }

    let project_count = app.project_store.project_count();
    let session_count = app.sessions.len();
    let both_exist = project_count > 0 && session_count > 0;

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
            // Navigate based on current focus
            if both_exist {
                match app.state.homepage_focus {
                    HomepageFocus::Projects => {
                        app.state.selected_project_index =
                            (app.state.selected_project_index + 1) % project_count;
                    }
                    HomepageFocus::Sessions => {
                        app.state.selected_session_index =
                            (app.state.selected_session_index + 1) % session_count;
                    }
                }
            } else if project_count > 0 {
                app.state.selected_project_index =
                    (app.state.selected_project_index + 1) % project_count;
            } else if session_count > 0 {
                app.state.selected_session_index =
                    (app.state.selected_session_index + 1) % session_count;
            }
        }
        KeyCode::Up => {
            // Navigate based on current focus
            if both_exist {
                match app.state.homepage_focus {
                    HomepageFocus::Projects => {
                        app.state.selected_project_index = app
                            .state
                            .selected_project_index
                            .checked_sub(1)
                            .unwrap_or(project_count - 1);
                    }
                    HomepageFocus::Sessions => {
                        app.state.selected_session_index = app
                            .state
                            .selected_session_index
                            .checked_sub(1)
                            .unwrap_or(session_count - 1);
                    }
                }
            } else if project_count > 0 {
                app.state.selected_project_index = app
                    .state
                    .selected_project_index
                    .checked_sub(1)
                    .unwrap_or(project_count - 1);
            } else if session_count > 0 {
                app.state.selected_session_index = app
                    .state
                    .selected_session_index
                    .checked_sub(1)
                    .unwrap_or(session_count - 1);
            }
        }
        KeyCode::Enter => {
            // Open selected project or session based on focus
            if both_exist {
                match app.state.homepage_focus {
                    HomepageFocus::Projects => {
                        let projects = app.project_store.projects_sorted();
                        if let Some(project) = projects.get(app.state.selected_project_index) {
                            app.state.navigate_to_project(project.id);
                        }
                    }
                    HomepageFocus::Sessions => {
                        if let Some(session) =
                            app.sessions.get_by_index(app.state.selected_session_index)
                        {
                            let session_id = session.info.id;
                            app.state.navigate_to_session(session_id);
                            app.sessions.acknowledge_attention(session_id);
                            if app.config.notification_method == "title" {
                                SessionManager::reset_terminal_title();
                            }
                            app.resize_active_session_pty()?;
                        }
                    }
                }
            } else if project_count > 0 {
                let projects = app.project_store.projects_sorted();
                if let Some(project) = projects.get(app.state.selected_project_index) {
                    app.state.navigate_to_project(project.id);
                }
            } else if session_count > 0 {
                if let Some(session) = app.sessions.get_by_index(app.state.selected_session_index) {
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
        }
        KeyCode::Char('d') => {
            // Delete from currently focused list
            if both_exist {
                match app.state.homepage_focus {
                    HomepageFocus::Projects => {
                        let projects = app.project_store.projects_sorted();
                        if let Some(project) = projects.get(app.state.selected_project_index) {
                            app.state.pending_delete_project = Some(project.id);
                            app.state.input_mode = InputMode::ConfirmingProjectDelete;
                        }
                    }
                    HomepageFocus::Sessions => {
                        if let Some(session) =
                            app.sessions.get_by_index(app.state.selected_session_index)
                        {
                            let id = session.info.id;
                            // Clear active_session if it was the destroyed session
                            if app.state.active_session == Some(id) {
                                app.state.active_session = None;
                            }
                            if let Err(e) = app.sessions.destroy_session(id) {
                                tracing::error!("Failed to destroy session: {}", e);
                            }
                            let new_count = app.sessions.len();
                            if app.state.selected_session_index >= new_count
                                && app.state.selected_session_index > 0
                            {
                                app.state.selected_session_index -= 1;
                            }
                        }
                    }
                }
            } else if project_count > 0 {
                // Only projects - delete selected project
                let projects = app.project_store.projects_sorted();
                if let Some(project) = projects.get(app.state.selected_project_index) {
                    app.state.pending_delete_project = Some(project.id);
                    app.state.input_mode = InputMode::ConfirmingProjectDelete;
                }
            } else if session_count > 0 {
                // Only sessions - delete selected session
                if let Some(session) = app.sessions.get_by_index(app.state.selected_session_index) {
                    let id = session.info.id;
                    // Clear active_session if it was the destroyed session
                    if app.state.active_session == Some(id) {
                        app.state.active_session = None;
                    }
                    if let Err(e) = app.sessions.destroy_session(id) {
                        tracing::error!("Failed to destroy session: {}", e);
                    }
                    let new_count = app.sessions.len();
                    if app.state.selected_session_index >= new_count
                        && app.state.selected_session_index > 0
                    {
                        app.state.selected_session_index -= 1;
                    }
                }
            }
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            if let Some(num) = c.to_digit(10) {
                if both_exist {
                    match app.state.homepage_focus {
                        HomepageFocus::Projects => {
                            if num > 0 && (num as usize) <= project_count {
                                app.state.selected_project_index = (num as usize) - 1;
                            }
                        }
                        HomepageFocus::Sessions => {
                            if num > 0 && (num as usize) <= session_count {
                                app.state.selected_session_index = (num as usize) - 1;
                            }
                        }
                    }
                } else if project_count > 0 && num > 0 && (num as usize) <= project_count {
                    app.state.selected_project_index = (num as usize) - 1;
                } else if project_count == 0 && num > 0 && (num as usize) <= session_count {
                    app.state.selected_session_index = (num as usize) - 1;
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
        _ => {}
    }
    Ok(())
}
