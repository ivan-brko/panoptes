//! Text input mode handlers
//!
//! Handles text input for names, paths, and other text fields.

use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use uuid::Uuid;

use crate::app::{App, InputMode};
use crate::session::SessionManager;

/// Handle key while creating a new session
pub fn handle_creating_session_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Only process key press events (not release/repeat)
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Esc => {
            // Cancel session creation
            app.state.input_mode = InputMode::Normal;
            app.state.new_session_name.clear();
            app.state.creating_session_project_id = None;
            app.state.creating_session_branch_id = None;
            app.state.creating_session_working_dir = None;
        }
        KeyCode::Enter => {
            // Create the session
            let name = if app.state.new_session_name.is_empty() {
                format!("Session {}", app.sessions.len() + 1)
            } else {
                std::mem::take(&mut app.state.new_session_name)
            };

            // Use context working directory, or current directory as fallback
            let working_dir = app
                .state
                .creating_session_working_dir
                .take()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

            // Get project/branch context (nil if unassociated)
            let project_id = app
                .state
                .creating_session_project_id
                .take()
                .unwrap_or(Uuid::nil());
            let branch_id = app
                .state
                .creating_session_branch_id
                .take()
                .unwrap_or(Uuid::nil());

            // Get terminal dimensions for the session
            let (rows, cols) = if let Ok(size) = app.tui.size() {
                (
                    size.height.saturating_sub(8) as usize,
                    size.width.saturating_sub(2) as usize,
                )
            } else {
                (24, 80) // Fallback dimensions
            };

            match app.sessions.create_session(
                name.clone(),
                working_dir,
                project_id,
                branch_id,
                None,
                rows,
                cols,
            ) {
                Ok(session_id) => {
                    tracing::info!("Created session: {} ({})", name, session_id);

                    // Update project/branch activity timestamps if associated
                    if !project_id.is_nil() {
                        if let Some(project) = app.project_store.get_project_mut(project_id) {
                            project.touch();
                        }
                    }
                    if !branch_id.is_nil() {
                        if let Some(branch) = app.project_store.get_branch_mut(branch_id) {
                            branch.touch();
                        }
                    }

                    // Navigate to the new session (auto-activates Session mode)
                    app.state.navigate_to_session(session_id);
                    app.tui.enable_mouse_capture();
                    app.sessions.acknowledge_attention(session_id);
                    if app.config.notification_method == "title" {
                        SessionManager::reset_terminal_title();
                    }
                    app.resize_active_session_pty()?;
                }
                Err(e) => {
                    tracing::error!("Failed to create session: {}", e);
                    // Only reset to Normal mode on failure
                    app.state.input_mode = InputMode::Normal;
                }
            }
        }
        KeyCode::Backspace => {
            app.state.new_session_name.pop();
        }
        KeyCode::Char(c) => {
            app.state.new_session_name.push(c);
        }
        _ => {}
    }
    Ok(())
}

/// Handle key when adding a new project (path input step)
pub fn handle_adding_project_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Only process key press events (not release/repeat)
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Esc => {
            if app.state.show_path_completions {
                // First Esc hides completions
                clear_path_completions(app);
            } else {
                // Second Esc cancels input
                app.state.input_mode = InputMode::Normal;
                app.state.new_project_path.clear();
                clear_path_completions(app);
            }
        }
        KeyCode::Tab => {
            if app.state.show_path_completions && !app.state.path_completions.is_empty() {
                // Apply selected completion (standard shell behavior)
                apply_path_completion(app);
            } else {
                // Show completions
                update_path_completions(app);
            }
        }
        KeyCode::BackTab => {
            // Cycle backward through completions
            if app.state.show_path_completions {
                let count = app.state.path_completions.len();
                if count > 0 {
                    app.state.path_completion_index = app
                        .state
                        .path_completion_index
                        .checked_sub(1)
                        .unwrap_or(count - 1);
                }
            }
        }
        KeyCode::Up => {
            // Navigate up in completions
            if app.state.show_path_completions {
                let count = app.state.path_completions.len();
                if count > 0 {
                    app.state.path_completion_index = app
                        .state
                        .path_completion_index
                        .checked_sub(1)
                        .unwrap_or(count - 1);
                }
            }
        }
        KeyCode::Down => {
            // Navigate down in completions
            if app.state.show_path_completions {
                let count = app.state.path_completions.len();
                if count > 0 {
                    app.state.path_completion_index = (app.state.path_completion_index + 1) % count;
                }
            }
        }
        KeyCode::Enter => {
            // Always validate path and transition to name input
            clear_path_completions(app);
            let path_str = std::mem::take(&mut app.state.new_project_path);
            let user_path = PathBuf::from(shellexpand::tilde(&path_str).into_owned());
            let user_path = user_path.canonicalize().unwrap_or(user_path);

            // Check if it's a git repository
            match crate::git::GitOps::discover(&user_path) {
                Ok(git) => {
                    let repo_path = git.repo_path().to_path_buf();
                    let repo_path = repo_path.canonicalize().unwrap_or(repo_path);

                    // Calculate session_subdir if user_path is inside repo_path
                    let session_subdir = if user_path != repo_path {
                        user_path
                            .strip_prefix(&repo_path)
                            .ok()
                            .map(|p| p.to_path_buf())
                    } else {
                        None
                    };

                    // Check if already added (with same subdir)
                    if app
                        .project_store
                        .find_by_repo_and_subdir(&repo_path, session_subdir.as_deref())
                        .is_some()
                    {
                        let path_display = if let Some(ref subdir) = session_subdir {
                            format!("{}/{}", repo_path.display(), subdir.display())
                        } else {
                            repo_path.display().to_string()
                        };
                        app.state.error_message =
                            Some(format!("Project already exists: {}", path_display));
                        tracing::warn!("Project already exists: {}", path_display);
                        app.state.input_mode = InputMode::Normal;
                        return Ok(());
                    }

                    // Get default branch
                    let default_branch = git
                        .default_branch_name()
                        .unwrap_or_else(|_| "main".to_string());

                    // Compute default project name from subdir folder or repo folder
                    let default_name = session_subdir
                        .as_ref()
                        .and_then(|s| s.file_name())
                        .or_else(|| repo_path.file_name())
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();

                    // Store pending values and transition to name input
                    app.state.pending_project_path = repo_path;
                    app.state.pending_session_subdir = session_subdir;
                    app.state.pending_default_branch = default_branch;
                    app.state.new_project_name = default_name;
                    app.state.input_mode = InputMode::AddingProjectName;
                }
                Err(e) => {
                    app.state.error_message =
                        Some(format!("Not a git repository: {}", user_path.display()));
                    tracing::error!("Not a git repository: {} ({})", user_path.display(), e);
                    app.state.input_mode = InputMode::Normal;
                }
            }
        }
        KeyCode::Backspace => {
            app.state.new_project_path.pop();
            update_path_completions(app);
        }
        KeyCode::Char(c) => {
            app.state.new_project_path.push(c);
            update_path_completions(app);
        }
        _ => {}
    }
    Ok(())
}

/// Handle key when entering project name (second step of project addition)
pub fn handle_adding_project_name_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Only process key press events (not release/repeat)
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Esc => {
            // Cancel project addition entirely
            app.state.input_mode = InputMode::Normal;
            app.state.new_project_name.clear();
            app.state.new_project_path.clear();
            app.state.pending_project_path = PathBuf::new();
            app.state.pending_session_subdir = None;
            app.state.pending_default_branch.clear();
        }
        KeyCode::Enter => {
            // Create project with custom (or default) name
            let name = if app.state.new_project_name.trim().is_empty() {
                // Use folder name as fallback
                app.state
                    .pending_project_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            } else {
                std::mem::take(&mut app.state.new_project_name)
                    .trim()
                    .to_string()
            };

            let repo_path = std::mem::take(&mut app.state.pending_project_path);
            let session_subdir = app.state.pending_session_subdir.take();
            let default_branch = std::mem::take(&mut app.state.pending_default_branch);

            // Create project
            let mut project = crate::project::Project::new(
                name.clone(),
                repo_path.clone(),
                default_branch.clone(),
            );
            project.session_subdir = session_subdir;
            let project_id = project.id;
            app.project_store.add_project(project);

            // Create default branch entry with effective working dir
            let effective_working_dir = app
                .project_store
                .get_project(project_id)
                .map(|p| p.effective_working_dir(&repo_path))
                .unwrap_or(repo_path);

            let branch = crate::project::Branch::default_for_project(
                project_id,
                default_branch,
                effective_working_dir,
            );
            app.project_store.add_branch(branch);

            // Save to disk
            if let Err(e) = app.project_store.save() {
                tracing::error!("Failed to save project store: {}", e);
                app.state.error_message = Some(format!("Failed to save project: {}", e));
            }

            tracing::info!("Added project: {}", name);

            // Select the newly added project
            let project_count = app.project_store.project_count();
            app.state.selected_project_index = project_count.saturating_sub(1);

            app.state.input_mode = InputMode::Normal;
        }
        KeyCode::Backspace => {
            app.state.new_project_name.pop();
        }
        KeyCode::Char(c) => {
            app.state.new_project_name.push(c);
        }
        _ => {}
    }
    Ok(())
}

/// Handle key while renaming a project
pub fn handle_renaming_project_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Esc => {
            app.state.input_mode = InputMode::Normal;
            app.state.new_project_name.clear();
            app.state.renaming_project = None;
        }
        KeyCode::Enter => {
            if let Some(project_id) = app.state.renaming_project {
                let new_name = app.state.new_project_name.trim().to_string();
                if !new_name.is_empty() {
                    if let Some(project) = app.project_store.get_project_mut(project_id) {
                        project.name = new_name;
                    }
                    if let Err(e) = app.project_store.save() {
                        app.state.error_message = Some(format!("Failed to save: {}", e));
                    }
                }
            }
            app.state.input_mode = InputMode::Normal;
            app.state.new_project_name.clear();
            app.state.renaming_project = None;
        }
        KeyCode::Backspace => {
            app.state.new_project_name.pop();
        }
        KeyCode::Char(c) => {
            app.state.new_project_name.push(c);
        }
        _ => {}
    }
    Ok(())
}

// ========================================================================
// Path Completion Helpers
// ========================================================================

/// Update path completions based on current input
fn update_path_completions(app: &mut App) {
    let completions = crate::path_complete::get_completions(&app.state.new_project_path);
    app.state.path_completions = completions;
    app.state.path_completion_index = 0;
    app.state.show_path_completions = !app.state.path_completions.is_empty();
}

/// Clear path completion state
fn clear_path_completions(app: &mut App) {
    app.state.path_completions.clear();
    app.state.path_completion_index = 0;
    app.state.show_path_completions = false;
}

/// Apply the selected completion to the input field
fn apply_path_completion(app: &mut App) {
    if let Some(path) = app
        .state
        .path_completions
        .get(app.state.path_completion_index)
    {
        app.state.new_project_path = crate::path_complete::path_to_input(path);
        // After applying, refresh completions for the new path
        update_path_completions(app);
    }
}
