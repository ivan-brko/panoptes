//! Text input mode handlers
//!
//! Handles text input for names, paths, and other text fields.

use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use uuid::Uuid;

use crate::agent::AgentType;
use crate::app::{
    cycle_next, cycle_prev, App, FolderMoveTarget, InputMode, MAX_PROJECT_NAME_LEN,
    MAX_PROJECT_PATH_LEN, MAX_SESSION_NAME_LEN,
};
use crate::session::{AgentAccount, NewSessionSpec};
use crate::tui::frame::{FrameConfig, FrameLayout};

/// Handle key while creating a new shell session
pub fn handle_creating_shell_session_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Only process key press events (not release/repeat)
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Esc => {
            // Cancel session creation
            app.state.input_mode = InputMode::Normal;
            app.state.session_draft.reset();
        }
        KeyCode::Enter => {
            // Create the shell session
            create_session(app, AgentType::Shell, None)?;
        }
        KeyCode::Backspace => {
            app.state.session_draft.name.pop();
        }
        KeyCode::Char(c) => {
            // Enforce length limit for session names
            if app.state.session_draft.name.len() < MAX_SESSION_NAME_LEN {
                app.state.session_draft.name.push(c);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Create a session of the given agent type from the current draft
///
/// The one create path for Claude, Codex, and shell sessions started from the
/// name-input dialogs. Consumes `app.state.session_draft`; `account` carries
/// the Claude/Codex profile when one was selected.
pub(crate) fn create_session(
    app: &mut App,
    agent: AgentType,
    account: Option<AgentAccount>,
) -> Result<()> {
    let draft = app.state.session_draft.take();

    // A blank name means Panoptes made one up, which the agent's own title may
    // later replace; a name the user typed is theirs and is never overwritten.
    let auto_named = draft.name.is_empty();
    let name = if auto_named {
        let prefix = match agent {
            AgentType::ClaudeCode => "Session",
            AgentType::OpenAICodex => "Codex",
            AgentType::Shell => "Shell",
        };
        format!("{} {}", prefix, app.sessions.len() + 1)
    } else {
        draft.name
    };

    let working_dir = draft
        .working_dir
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let project_id = draft.project_id.unwrap_or(Uuid::nil());
    let branch_id = draft.branch_id.unwrap_or(Uuid::nil());

    // Size the new PTY exactly like the session view renders it (and like
    // resize_active_session_pty computes it), so the session never starts
    // with briefly-wrong dimensions.
    let (rows, cols) = if let Ok(size) = app.tui.size() {
        let layout = FrameLayout::calculate(size, &FrameConfig::default());
        let (rows, cols) = layout.pty_size();
        (rows as usize, cols as usize)
    } else {
        (24, 80)
    };

    match app.sessions.create_session(
        agent,
        NewSessionSpec {
            name: name.clone(),
            working_dir,
            project_id,
            branch_id,
            initial_prompt: None,
            account,
            auto_close: false,
        },
        rows,
        cols,
    ) {
        Ok(session_id) => {
            tracing::info!(agent = ?agent, "Created session: {} ({})", name, session_id);
            if agent != AgentType::Shell {
                // Shells have no agent title that could replace the name
                app.sessions.set_auto_named(session_id, auto_named);
            }

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

            app.activate_session(session_id)?;
        }
        Err(e) => {
            tracing::error!("Failed to create session: {}", e);
            app.state.error_message = Some(format!("Failed to create session: {}", e));
            app.state.input_mode = InputMode::Normal;
        }
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
        KeyCode::BackTab | KeyCode::Up => {
            // Cycle backward through completions
            if app.state.show_path_completions {
                app.state.path_completion_index = cycle_prev(
                    app.state.path_completion_index,
                    app.state.path_completions.len(),
                );
            }
        }
        KeyCode::Down => {
            // Navigate down in completions
            if app.state.show_path_completions {
                app.state.path_completion_index = cycle_next(
                    app.state.path_completion_index,
                    app.state.path_completions.len(),
                );
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
            // Enforce length limit for project paths
            if app.state.new_project_path.len() < MAX_PROJECT_PATH_LEN {
                app.state.new_project_path.push(c);
                update_path_completions(app);
            }
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
            // Enforce length limit for project names
            if app.state.new_project_name.len() < MAX_PROJECT_NAME_LEN {
                app.state.new_project_name.push(c);
            }
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
            // Enforce length limit for project names (used for renaming)
            if app.state.new_project_name.len() < MAX_PROJECT_NAME_LEN {
                app.state.new_project_name.push(c);
            }
        }
        _ => {}
    }
    Ok(())
}

// ========================================================================
// Folder Organization Input Handlers
// ========================================================================

/// Maximum length for a folder path input
pub(crate) const MAX_FOLDER_PATH_LEN: usize = 150;

/// Handle key while typing the destination folder for a project or folder
pub fn handle_moving_to_folder_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Esc => {
            if app.state.show_folder_completions {
                // First Esc hides completions
                clear_folder_completions(app);
            } else {
                cancel_folder_input(app);
            }
        }
        KeyCode::Tab => {
            if app.state.show_folder_completions && !app.state.folder_completions.is_empty() {
                apply_folder_completion(app);
            } else {
                update_folder_completions(app);
            }
        }
        KeyCode::BackTab | KeyCode::Up => {
            cycle_folder_completion(app, false);
        }
        KeyCode::Down => {
            cycle_folder_completion(app, true);
        }
        KeyCode::Enter => {
            apply_folder_move(app);
        }
        KeyCode::Backspace => {
            app.state.folder_input.pop();
            app.state.folder_error = None;
            update_folder_completions(app);
        }
        KeyCode::Char(c) => {
            if app.state.folder_input.len() < MAX_FOLDER_PATH_LEN {
                app.state.folder_input.push(c);
                app.state.folder_error = None;
                update_folder_completions(app);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Handle key while renaming a folder
pub fn handle_renaming_folder_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Esc => {
            cancel_folder_input(app);
        }
        KeyCode::Enter => {
            let Some(path) = app.state.renaming_folder.clone() else {
                cancel_folder_input(app);
                return Ok(());
            };
            let new_name = app.state.folder_input.trim().to_string();

            match app.project_store.rename_folder(&path, &new_name) {
                Ok(count) => {
                    save_project_store(app);
                    // Follow the folder to its new position in the list
                    let mut renamed = path.clone();
                    if let Some(last) = renamed.last_mut() {
                        *last = new_name;
                    }
                    if let Some(index) =
                        crate::project::row_index_of_folder(&app.project_store, &renamed)
                    {
                        app.state.selected_project_index = index;
                    }
                    app.state.header_notifications.push(format!(
                        "Renamed folder ({})",
                        crate::project::project_count_label(count)
                    ));
                    cancel_folder_input(app);
                }
                Err(e) => {
                    app.state.folder_error = Some(e.to_string());
                }
            }
        }
        KeyCode::Backspace => {
            app.state.folder_input.pop();
            app.state.folder_error = None;
        }
        KeyCode::Char(c) => {
            if app.state.folder_input.len() < MAX_FOLDER_PATH_LEN {
                app.state.folder_input.push(c);
                app.state.folder_error = None;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Apply the typed destination to the pending move target
fn apply_folder_move(app: &mut App) {
    let Some(target) = app.state.moving_to_folder.clone() else {
        cancel_folder_input(app);
        return;
    };

    let destination = match crate::project::parse_folder_path(&app.state.folder_input) {
        Ok(segments) => segments,
        Err(e) => {
            app.state.folder_error = Some(e.to_string());
            return;
        }
    };

    let outcome = match &target {
        FolderMoveTarget::Project(id) => app
            .project_store
            .set_project_folder(*id, destination.clone())
            .map(|_| 1),
        FolderMoveTarget::Folder(path) => app.project_store.move_folder(path, &destination),
    };

    match outcome {
        Ok(count) => {
            save_project_store(app);

            // Keep the moved row selected, expanding collapsed ancestors so it
            // stays visible
            for depth in 1..=destination.len() {
                app.project_store
                    .set_folder_collapsed(&destination[..depth], false);
            }
            let index = match &target {
                FolderMoveTarget::Project(id) => {
                    crate::project::row_index_of_project(&app.project_store, *id)
                }
                FolderMoveTarget::Folder(path) => {
                    let mut moved = destination.clone();
                    if let Some(name) = path.last() {
                        moved.push(name.clone());
                    }
                    crate::project::row_index_of_folder(&app.project_store, &moved)
                }
            };
            if let Some(index) = index {
                app.state.selected_project_index = index;
            }

            let where_to = if destination.is_empty() {
                "the root level".to_string()
            } else {
                format!("'{}'", crate::project::folder_path_key(&destination))
            };
            app.state.header_notifications.push(format!(
                "Moved {} to {}",
                crate::project::project_count_label(count),
                where_to
            ));
            cancel_folder_input(app);
        }
        Err(e) => {
            app.state.folder_error = Some(e.to_string());
        }
    }
}

/// Persist the project store, surfacing failures to the user
fn save_project_store(app: &mut App) {
    if let Err(e) = app.project_store.save() {
        tracing::error!("Failed to save project store: {}", e);
        app.state.error_message = Some(format!("Failed to save projects: {}", e));
    }
}

/// Clear all folder dialog state and return to normal mode
fn cancel_folder_input(app: &mut App) {
    app.state.input_mode = InputMode::Normal;
    app.state.folder_input.clear();
    app.state.moving_to_folder = None;
    app.state.renaming_folder = None;
    app.state.folder_error = None;
    clear_folder_completions(app);
}

/// Refresh folder completions for the current input
pub(crate) fn update_folder_completions(app: &mut App) {
    let query = app.state.folder_input.to_lowercase();
    app.state.folder_completions = crate::project::all_folder_paths(&app.project_store)
        .iter()
        .map(|path| crate::project::folder_path_key(path))
        .filter(|key| key.to_lowercase().starts_with(&query))
        .collect();
    app.state.folder_completion_index = 0;
    app.state.show_folder_completions = !app.state.folder_completions.is_empty();
}

/// Clear folder completion state
fn clear_folder_completions(app: &mut App) {
    app.state.folder_completions.clear();
    app.state.folder_completion_index = 0;
    app.state.show_folder_completions = false;
}

/// Move the folder completion selection forward or backward
fn cycle_folder_completion(app: &mut App, forward: bool) {
    if !app.state.show_folder_completions {
        return;
    }
    let count = app.state.folder_completions.len();
    app.state.folder_completion_index = if forward {
        cycle_next(app.state.folder_completion_index, count)
    } else {
        cycle_prev(app.state.folder_completion_index, count)
    };
}

/// Apply the selected folder completion to the input field
fn apply_folder_completion(app: &mut App) {
    if let Some(path) = app
        .state
        .folder_completions
        .get(app.state.folder_completion_index)
    {
        app.state.folder_input = path.clone();
        update_folder_completions(app);
    }
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

// ========================================================================
// Agent Type Selection Handler
// ========================================================================

/// Handle key when selecting agent type (Claude Code vs Codex)
pub fn handle_selecting_agent_type_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    match key.code {
        KeyCode::Esc => {
            app.state.input_mode = InputMode::Normal;
            app.state.agent_type_selector_index = 0;
            app.state.session_draft.reset();
        }
        KeyCode::Down => {
            app.state.agent_type_selector_index =
                cycle_next(app.state.agent_type_selector_index, 2);
        }
        KeyCode::Up => {
            app.state.agent_type_selector_index =
                cycle_prev(app.state.agent_type_selector_index, 2);
        }
        KeyCode::Enter => {
            if app.state.agent_type_selector_index == 0 {
                // Claude Code selected
                app.state.input_mode = InputMode::CreatingSession;
            } else {
                // Codex selected
                app.state.input_mode = InputMode::CreatingCodexSession;
            }
            app.state.agent_type_selector_index = 0;
        }
        _ => {}
    }
    Ok(())
}
