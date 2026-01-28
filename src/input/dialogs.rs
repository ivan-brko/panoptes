//! Confirmation dialog handlers
//!
//! Handles keyboard input for various confirmation dialogs.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{App, InputMode, View};
use crate::claude_json::ClaudeJsonStore;
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
                // Validate session still exists before deleting
                if app.sessions.get(session_id).is_none() {
                    tracing::warn!(
                        session_id = %session_id,
                        "Session no longer exists when confirming delete"
                    );
                    app.state.input_mode = InputMode::Normal;
                    return Ok(());
                }

                // Get branch_id before destroying (for selection adjustment)
                let branch_id = app.state.view.branch_id();
                let project_id = app.state.view.project_id();
                let was_active = app.state.active_session == Some(session_id);
                let was_in_session_view = app.state.view == View::SessionView;

                // Clear active_session if it was the destroyed session
                if was_active {
                    app.state.active_session = None;
                    app.state.session_return_view = None;
                    // Navigate back from session view if we're there
                    if was_in_session_view {
                        // Navigate to branch detail or project detail or projects overview
                        if let (Some(pid), Some(bid)) = (project_id, branch_id) {
                            app.state.view = View::BranchDetail(pid, bid);
                        } else if let Some(pid) = project_id {
                            app.state.view = View::ProjectDetail(pid);
                        } else {
                            app.state.view = View::ProjectsOverview;
                        }
                        app.state
                            .header_notifications
                            .push("Session ended".to_string());
                    }
                }

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
                // Validate branch still exists before deleting
                let branch_info = app.project_store.get_branch(branch_id).cloned();
                if branch_info.is_none() {
                    tracing::warn!(
                        branch_id = %branch_id,
                        "Branch no longer exists when confirming delete"
                    );
                    app.state.input_mode = InputMode::Normal;
                    app.state.delete_worktree_on_disk = false;
                    return Ok(());
                }

                // Destroy all sessions for this branch
                let sessions_to_destroy: Vec<_> = app
                    .sessions
                    .sessions_for_branch(branch_id)
                    .iter()
                    .map(|s| s.info.id)
                    .collect();

                for session_id in sessions_to_destroy {
                    // Clear active_session if it was destroyed
                    if app.state.active_session == Some(session_id) {
                        app.state.active_session = None;
                    }
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
                                                tracing::error!("Failed to remove worktree: {}", e);
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

                // Clean up Claude permissions for deleted worktree
                if let Some(branch) = &branch_info {
                    if branch.is_worktree {
                        let worktree_path = branch.working_dir.to_string_lossy().to_string();

                        // Get the Claude config to use (project default or global default)
                        if let Some(project_id) = app.state.view.project_id() {
                            let config_dir = app
                                .project_store
                                .get_project(project_id)
                                .and_then(|p| p.default_claude_config)
                                .or_else(|| app.claude_config_store.get_default_id())
                                .and_then(|id| app.claude_config_store.get(id))
                                .and_then(|c| c.config_dir.clone());

                            if let Some(store) =
                                ClaudeJsonStore::for_config_dir(config_dir.as_deref())
                            {
                                match store.remove_settings(&worktree_path) {
                                    Ok(true) => {
                                        tracing::info!(
                                            "Removed Claude permissions for deleted worktree: {}",
                                            worktree_path
                                        );
                                    }
                                    Ok(false) => {
                                        tracing::debug!(
                                            "No Claude permissions found for worktree: {}",
                                            worktree_path
                                        );
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to remove Claude permissions for {}: {}",
                                            worktree_path,
                                            e
                                        );
                                    }
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
                    app.state.error_message = Some(format!("Failed to save project store: {}", e));
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
                // Validate project still exists before deleting
                if app.project_store.get_project(project_id).is_none() {
                    tracing::warn!(
                        project_id = %project_id,
                        "Project no longer exists when confirming delete"
                    );
                    app.state.input_mode = InputMode::Normal;
                    return Ok(());
                }

                // Destroy all sessions for this project
                let sessions_to_destroy: Vec<_> = app
                    .sessions
                    .sessions_for_project(project_id)
                    .iter()
                    .map(|s| s.info.id)
                    .collect();

                for session_id in sessions_to_destroy {
                    // Clear active_session if it was destroyed
                    if app.state.active_session == Some(session_id) {
                        app.state.active_session = None;
                    }
                    if let Err(e) = app.sessions.destroy_session(session_id) {
                        tracing::error!("Failed to destroy session: {}", e);
                    }
                }

                // Remove project and its branches from the store
                app.project_store.remove_project(project_id);

                // Save to disk
                if let Err(e) = app.project_store.save() {
                    tracing::error!("Failed to save project store: {}", e);
                    app.state.error_message = Some(format!("Failed to save project store: {}", e));
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

/// Handle key when confirming Claude config deletion
pub fn handle_confirming_claude_config_delete_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            // Confirm deletion
            if let Some(config_id) = app.state.pending_delete_claude_config.take() {
                // Validate config still exists
                if app.claude_config_store.get(config_id).is_none() {
                    tracing::warn!(
                        config_id = %config_id,
                        "Config no longer exists when confirming delete"
                    );
                    app.state.input_mode = InputMode::Normal;
                    return Ok(());
                }

                // Clear default_claude_config from any projects using this config
                let affected_projects: Vec<_> = app
                    .project_store
                    .projects()
                    .filter(|p| p.default_claude_config == Some(config_id))
                    .map(|p| p.id)
                    .collect();

                for project_id in affected_projects {
                    if let Some(project) = app.project_store.get_project_mut(project_id) {
                        project.default_claude_config = None;
                    }
                }

                // Save project store if any projects were affected
                if let Err(e) = app.project_store.save() {
                    tracing::error!("Failed to save project store: {}", e);
                }

                // Remove the config
                app.claude_config_store.remove(config_id);

                // Save config store
                if let Err(e) = app.claude_config_store.save() {
                    tracing::error!("Failed to save claude config store: {}", e);
                    app.state.error_message = Some(format!("Failed to save: {}", e));
                }

                tracing::info!("Deleted Claude config: {}", config_id);

                // Adjust selection if needed
                let new_count = app.claude_config_store.count();
                if app.state.claude_configs_selected_index >= new_count && new_count > 0 {
                    app.state.claude_configs_selected_index = new_count - 1;
                } else if new_count == 0 {
                    app.state.claude_configs_selected_index = 0;
                }
            }
            app.state.input_mode = InputMode::Normal;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            // Cancel deletion
            app.state.pending_delete_claude_config = None;
            app.state.input_mode = InputMode::Normal;
        }
        _ => {}
    }
    Ok(())
}

/// Handle key when confirming Claude settings copy after worktree creation
pub fn handle_confirming_claude_settings_copy_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    match key.code {
        KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
            // Toggle selection
            if let Some(ref mut state) = app.state.pending_claude_settings_copy {
                state.selected_yes = !state.selected_yes;
            }
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            // Force yes
            if let Some(ref mut state) = app.state.pending_claude_settings_copy {
                state.selected_yes = true;
            }
            confirm_claude_settings_copy(app);
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            // Force no
            if let Some(ref mut state) = app.state.pending_claude_settings_copy {
                state.selected_yes = false;
            }
            confirm_claude_settings_copy(app);
        }
        KeyCode::Enter => {
            confirm_claude_settings_copy(app);
        }
        KeyCode::Esc => {
            // Skip without copying, still navigate
            let copy_state = app.state.pending_claude_settings_copy.take();
            app.state.input_mode = InputMode::Normal;
            if let Some(state) = copy_state {
                app.state
                    .navigate_to_branch(state.project_id, state.branch_id);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Confirm the Claude settings copy action
fn confirm_claude_settings_copy(app: &mut App) {
    let copy_state = app.state.pending_claude_settings_copy.take();
    app.state.input_mode = InputMode::Normal;

    if let Some(state) = copy_state {
        if state.selected_yes {
            let _ = app.show_loading("Copying Claude settings...");

            // Copy modern local settings first (.claude/settings.local.json)
            if state.has_local_settings {
                match crate::claude_json::copy_local_settings(
                    &state.source_path,
                    &state.target_path,
                ) {
                    Ok(true) => {
                        tracing::info!(
                            "Copied local Claude settings from {} to {}",
                            state.source_path.display(),
                            state.target_path.display()
                        );
                    }
                    Ok(false) => {
                        tracing::debug!(
                            "Local settings not copied (source missing or dest exists)"
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Failed to copy local Claude settings: {}", e);
                    }
                }
            }

            // Copy legacy settings (.claude.json keyed by path)
            let source = state.source_path.to_string_lossy().to_string();
            let target = state.target_path.to_string_lossy().to_string();

            if let Some(store) = ClaudeJsonStore::for_config_dir(state.claude_config_dir.as_deref())
            {
                match store.copy_settings(&source, &target) {
                    Ok(()) => {
                        tracing::info!(
                            "Copied legacy Claude settings from {} to {}",
                            source,
                            target
                        );
                    }
                    Err(e) => {
                        tracing::error!("Failed to copy legacy Claude settings: {}", e);
                    }
                }
            }

            app.state
                .header_notifications
                .push("Claude settings copied to worktree");
            app.clear_loading();
        }

        // Navigate to the new branch
        app.state
            .navigate_to_branch(state.project_id, state.branch_id);
    }
}

/// Handle key when confirming Claude settings migration before worktree deletion
pub fn handle_confirming_claude_settings_migrate_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    match key.code {
        KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
            // Toggle selection
            if let Some(ref mut state) = app.state.pending_claude_settings_migrate {
                state.selected_yes = !state.selected_yes;
            }
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            // Force yes
            if let Some(ref mut state) = app.state.pending_claude_settings_migrate {
                state.selected_yes = true;
            }
            confirm_claude_settings_migrate(app);
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            // Force no
            if let Some(ref mut state) = app.state.pending_claude_settings_migrate {
                state.selected_yes = false;
            }
            confirm_claude_settings_migrate(app);
        }
        KeyCode::Enter => {
            confirm_claude_settings_migrate(app);
        }
        KeyCode::Esc => {
            // Cancel entire deletion
            app.state.pending_claude_settings_migrate = None;
            app.state.input_mode = InputMode::Normal;
        }
        _ => {}
    }
    Ok(())
}

/// Confirm the Claude settings migration action
fn confirm_claude_settings_migrate(app: &mut App) {
    let migrate_state = app.state.pending_claude_settings_migrate.take();

    if let Some(state) = migrate_state {
        let branch_id = state.branch_id;

        if state.selected_yes {
            let _ = app.show_loading("Migrating permissions...");
            let mut total_migrated = 0;

            // Merge modern local settings first (.claude/settings.local.json)
            if state.has_local_settings {
                match crate::claude_json::merge_local_settings(
                    &state.worktree_path,
                    &state.main_path,
                ) {
                    Ok(added) => {
                        if !added.is_empty() {
                            tracing::info!(
                                "Merged {} local settings from worktree to main: {:?}",
                                added.len(),
                                added
                            );
                            total_migrated += added.len();
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to merge local settings: {}", e);
                    }
                }
            }

            // Merge legacy settings (tools from .claude.json)
            if !state.unique_tools.is_empty() {
                let worktree = state.worktree_path.to_string_lossy().to_string();
                let main = state.main_path.to_string_lossy().to_string();

                // Use the stored config_dir from state
                if let Some(store) =
                    ClaudeJsonStore::for_config_dir(state.claude_config_dir.as_deref())
                {
                    match store.merge_settings(&worktree, &main) {
                        Ok(added) => {
                            if !added.is_empty() {
                                tracing::info!(
                                    "Migrated {} legacy permissions from {} to {}",
                                    added.len(),
                                    worktree,
                                    main
                                );
                                total_migrated += added.len();
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to migrate legacy permissions: {}", e);
                            app.state
                                .header_notifications
                                .push(format!("Failed to migrate permissions: {}", e));
                        }
                    }
                }
            }

            if total_migrated > 0 {
                app.state.header_notifications.push(format!(
                    "Migrated {} setting{} to main repo",
                    total_migrated,
                    if total_migrated == 1 { "" } else { "s" }
                ));
            }

            app.clear_loading();
        }

        // Continue to branch delete confirmation
        // Get branch info to set up delete state
        if let Some(branch) = app.project_store.get_branch(branch_id) {
            app.state.pending_delete_branch = Some(branch_id);
            app.state.delete_worktree_on_disk = branch.is_worktree;
            app.state.input_mode = InputMode::ConfirmingBranchDelete;
        } else {
            // Branch no longer exists, just go back to normal
            app.state.input_mode = InputMode::Normal;
        }
    } else {
        app.state.input_mode = InputMode::Normal;
    }
}
