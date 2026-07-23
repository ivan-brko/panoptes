//! Confirmation dialog handlers
//!
//! Handles keyboard input for various confirmation dialogs.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{App, AppState, InputMode, View};
use crate::claude_config::ClaudeConfigStore;
use crate::claude_json::ClaudeJsonStore;
use crate::config::{is_reserved_key, CustomShortcut};
use crate::project::{Branch, ProjectStore};
use crate::session::SessionManager;

/// Handle key when confirming session deletion
pub fn handle_confirming_delete_key(app: &mut App, key: KeyEvent) -> Result<()> {
    confirming_session_delete_key(&mut app.state, &mut app.sessions, key)
}

/// Session delete confirmation body
///
/// Takes the parts of `App` it needs so the flow - which destroys sessions -
/// can be unit tested against a temp-backed [`SessionManager`].
pub(crate) fn confirming_session_delete_key(
    state: &mut AppState,
    sessions: &mut SessionManager,
    key: KeyEvent,
) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            // Confirm deletion
            if let Some(session_id) = state.pending_delete_session.take() {
                // Validate session still exists before deleting. A recovered
                // session counts: it has no process, but it does have a record
                // to discard, and this dialog is the only way to clear one.
                if sessions.get(session_id).is_none()
                    && sessions.get_recovered(session_id).is_none()
                {
                    tracing::warn!(
                        session_id = %session_id,
                        "Session no longer exists when confirming delete"
                    );
                    state.input_mode = InputMode::Normal;
                    return Ok(());
                }

                // Get branch_id before destroying (for selection adjustment)
                let branch_id = state.view.branch_id();
                let project_id = state.view.project_id();
                let was_active = state.active_session == Some(session_id);
                let was_in_session_view = state.view == View::SessionView;

                // Clear active_session if it was the destroyed session
                if was_active {
                    state.active_session = None;
                    state.session_return_view = None;
                    // Navigate back from session view if we're there
                    if was_in_session_view {
                        // Navigate to branch detail or project detail or projects overview
                        if let (Some(pid), Some(bid)) = (project_id, branch_id) {
                            state.view = View::BranchDetail(pid, bid);
                        } else if let Some(pid) = project_id {
                            state.view = View::ProjectDetail(pid);
                        } else {
                            state.view = View::ProjectsOverview;
                        }
                        state.header_notifications.push("Session ended".to_string());
                    }
                }

                // A recovered session has no process to kill - discarding it just
                // drops its record, so it is no longer offered on next launch
                if sessions.discard_recovered(session_id) {
                    tracing::info!(session_id = %session_id, "Discarded recovered session");
                } else if let Err(e) = sessions.destroy_session(session_id) {
                    tracing::error!("Failed to destroy session: {}", e);
                }

                // Adjust selection in whichever list the delete came from
                let remaining = match branch_id {
                    Some(branch_id) => Some(sessions.entries_for_branch(branch_id).len()),
                    // The overview's sessions list is keyed off the same index
                    None if state.view == View::ProjectsOverview => Some(sessions.len()),
                    None => None,
                };
                if let Some(new_count) = remaining {
                    if state.selected_session_index >= new_count && new_count > 0 {
                        state.selected_session_index = new_count - 1;
                    }
                }
            }
            state.input_mode = InputMode::Normal;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            // Cancel deletion
            state.pending_delete_session = None;
            state.input_mode = InputMode::Normal;
        }
        _ => {}
    }
    Ok(())
}

/// Handle key when confirming branch/worktree deletion
///
/// The 'y' arm is split into testable phases: [`begin_branch_delete`]
/// (validate + destroy sessions), the on-disk worktree removal (the only part
/// needing the full `App`, for the loading overlay), and
/// [`finish_branch_delete`] (permission cleanup + store removal).
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
            let Some(branch) =
                begin_branch_delete(&mut app.state, &mut app.sessions, &app.project_store)
            else {
                return Ok(());
            };

            // If user opted to delete worktree on disk
            if app.state.delete_worktree_on_disk && branch.is_worktree {
                app.remove_worktree_on_disk(&branch);
            }

            finish_branch_delete(
                &mut app.state,
                &mut app.project_store,
                &app.claude_config_store,
                &branch,
            );
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            cancel_branch_delete(&mut app.state);
        }
        _ => {}
    }
    Ok(())
}

/// Cancel a pending branch deletion, resetting the dialog state
pub(crate) fn cancel_branch_delete(state: &mut AppState) {
    state.pending_delete_branch = None;
    state.delete_worktree_on_disk = false;
    state.input_mode = InputMode::Normal;
}

/// First phase of a confirmed branch delete: validate and destroy sessions
///
/// Takes the pending branch ID, verifies the branch still exists (resetting
/// the dialog and returning `None` when it does not - or when nothing was
/// pending), and destroys every session of the branch, including recovered
/// ones - deleting the branch invalidates their working dir too.
pub(crate) fn begin_branch_delete(
    state: &mut AppState,
    sessions: &mut SessionManager,
    project_store: &ProjectStore,
) -> Option<Branch> {
    let Some(branch_id) = state.pending_delete_branch.take() else {
        // Nothing pending - leave the dialog as the original flow did
        state.delete_worktree_on_disk = false;
        state.input_mode = InputMode::Normal;
        return None;
    };

    // Validate branch still exists before deleting
    let Some(branch) = project_store.get_branch(branch_id).cloned() else {
        tracing::warn!(
            branch_id = %branch_id,
            "Branch no longer exists when confirming delete"
        );
        state.input_mode = InputMode::Normal;
        state.delete_worktree_on_disk = false;
        return None;
    };

    let sessions_to_destroy: Vec<_> = sessions
        .entries_for_branch(branch_id)
        .iter()
        .map(|entry| entry.info.id)
        .collect();

    for session_id in sessions_to_destroy {
        // Clear active_session if it was destroyed
        if state.active_session == Some(session_id) {
            state.active_session = None;
        }
        if sessions.discard_recovered(session_id) {
            continue;
        }
        if let Err(e) = sessions.destroy_session(session_id) {
            tracing::error!("Failed to destroy session: {}", e);
        }
    }

    Some(branch)
}

/// Final phase of a confirmed branch delete: cleanup and store removal
///
/// Cleans up Claude permissions recorded for a deleted worktree, removes the
/// branch from the store, persists it, and resets the dialog state.
pub(crate) fn finish_branch_delete(
    state: &mut AppState,
    project_store: &mut ProjectStore,
    claude_config_store: &ClaudeConfigStore,
    branch: &Branch,
) {
    // Clean up Claude permissions for deleted worktree
    if branch.is_worktree {
        let worktree_path = branch.working_dir.to_string_lossy().to_string();

        // Get the Claude config to use (project default or global default)
        if let Some(project_id) = state.view.project_id() {
            let config_dir = project_store
                .get_project(project_id)
                .and_then(|p| p.default_claude_config)
                .or_else(|| claude_config_store.get_default_id())
                .and_then(|id| claude_config_store.get(id))
                .and_then(|c| c.config_dir.clone());

            if let Some(store) = ClaudeJsonStore::for_config_dir(config_dir.as_deref()) {
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

    // Remove branch from the store
    project_store.remove_branch(branch.id);

    // Save to disk
    if let Err(e) = project_store.save() {
        tracing::error!("Failed to save project store: {}", e);
        state.error_message = Some(format!("Failed to save project store: {}", e));
    }

    tracing::info!("Deleted branch: {}", branch.id);

    // Adjust selected index if needed
    if let Some(project_id) = state.view.project_id() {
        let new_count = project_store.branches_for_project(project_id).len();
        if state.selected_branch_index >= new_count && new_count > 0 {
            state.selected_branch_index = new_count - 1;
        } else if new_count == 0 {
            state.selected_branch_index = 0;
        }
    }

    state.delete_worktree_on_disk = false;
    state.input_mode = InputMode::Normal;
}

impl App {
    /// Remove a branch's git worktree directory on disk
    ///
    /// Best-effort: failures are logged and surfaced as an error message, and
    /// the branch deletion continues regardless. Lives on `App` for the
    /// loading overlay shown around the blocking git call.
    fn remove_worktree_on_disk(&mut self, branch: &Branch) {
        // Get the project to access the repo
        let Some(project_id) = self.state.view.project_id() else {
            return;
        };
        // Clone the repo_path to avoid borrow conflicts
        let Some(repo_path) = self
            .project_store
            .get_project(project_id)
            .map(|p| p.repo_path.clone())
        else {
            return;
        };

        // Show loading indicator
        self.show_loading(&format!("Removing worktree '{}'...", branch.name));

        match crate::git::GitOps::open(&repo_path) {
            Ok(git) => {
                if let Err(e) =
                    crate::git::worktree::remove_worktree(git.repository(), &branch.name, true)
                {
                    tracing::error!("Failed to remove worktree: {}", e);
                    self.state.error_message = Some(format!("Failed to remove worktree: {}", e));
                } else {
                    tracing::info!("Removed worktree for branch: {}", branch.name);
                }
            }
            Err(e) => {
                tracing::error!("Failed to open git repo: {}", e);
                self.state.error_message = Some(format!("Failed to open git repo: {}", e));
            }
        }

        self.clear_loading();
    }
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
                    .entries_for_project(project_id)
                    .iter()
                    .map(|entry| entry.info.id)
                    .collect();

                for session_id in sessions_to_destroy {
                    // Clear active_session if it was destroyed
                    if app.state.active_session == Some(session_id) {
                        app.state.active_session = None;
                    }
                    // Recovered sessions have no process; removing the project
                    // must still drop their records
                    if app.sessions.discard_recovered(session_id) {
                        continue;
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

                // Adjust selected index if needed (counted over visible tree rows)
                let new_row_count = crate::project::row_count(&app.project_store);
                if app.state.selected_project_index >= new_row_count {
                    app.state.selected_project_index = new_row_count.saturating_sub(1);
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

/// Handle key while confirming that a folder should be dissolved
///
/// Removing a folder never deletes projects: its contents move up one level.
pub fn handle_confirming_folder_remove_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            if let Some(path) = app.state.pending_remove_folder.take() {
                let moved = app.project_store.remove_folder(&path);
                if let Err(e) = app.project_store.save() {
                    tracing::error!("Failed to save project store: {}", e);
                    app.state.error_message = Some(format!("Failed to save projects: {}", e));
                }
                tracing::info!(
                    "Removed folder '{}', moved {} project(s) up a level",
                    crate::project::folder_path_key(&path),
                    moved
                );
                app.state.header_notifications.push(format!(
                    "Ungrouped {}",
                    crate::project::project_count_label(moved)
                ));

                // The heading row is gone, so clamp the selection
                let row_count = crate::project::row_count(&app.project_store);
                if app.state.selected_project_index >= row_count {
                    app.state.selected_project_index = row_count.saturating_sub(1);
                }
            }
            app.state.input_mode = InputMode::Normal;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.state.pending_remove_folder = None;
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
            app.show_loading("Copying Claude settings...");

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
            app.show_loading("Migrating permissions...");
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

// ============================================================================
// Custom Shortcuts Dialog Handlers
// ============================================================================

/// Handle key when managing custom shortcuts (list view)
pub fn handle_managing_custom_shortcuts_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    let shortcut_count = app.config.custom_shortcuts.len();

    match key.code {
        KeyCode::Esc => {
            // Close the dialog
            app.state.input_mode = InputMode::Normal;
        }
        KeyCode::Down => {
            // Navigate down
            app.state.custom_shortcuts_selected =
                crate::app::cycle_next(app.state.custom_shortcuts_selected, shortcut_count);
        }
        KeyCode::Up => {
            // Navigate up
            app.state.custom_shortcuts_selected =
                crate::app::cycle_prev(app.state.custom_shortcuts_selected, shortcut_count);
        }
        KeyCode::Char('n') => {
            // Start adding a new shortcut
            app.state.new_shortcut_key = None;
            app.state.new_shortcut_name.clear();
            app.state.new_shortcut_command.clear();
            app.state.new_shortcut_auto_close = false;
            app.state.shortcut_error = None;
            app.state.input_mode = InputMode::AddingCustomShortcutKey;
        }
        KeyCode::Char('d') => {
            // Delete selected shortcut (if any)
            if shortcut_count > 0 {
                app.state.pending_delete_shortcut_index = Some(app.state.custom_shortcuts_selected);
                app.state.input_mode = InputMode::ConfirmingCustomShortcutDelete;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Handle key when adding a custom shortcut - capturing the key
pub fn handle_adding_custom_shortcut_key_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    match key.code {
        KeyCode::Esc => {
            // Cancel and go back to management dialog
            app.state.shortcut_error = None;
            app.state.input_mode = InputMode::ManagingCustomShortcuts;
        }
        KeyCode::Char(c) => {
            // Validate the key
            if is_reserved_key(c) {
                app.state.shortcut_error = Some(format!("Key '{}' is reserved", c));
            } else if app.config.custom_shortcuts.iter().any(|s| s.key == c) {
                app.state.shortcut_error = Some(format!("Key '{}' is already in use", c));
            } else {
                // Valid key, proceed to name input
                app.state.new_shortcut_key = Some(c);
                app.state.shortcut_error = None;
                app.state.input_mode = InputMode::AddingCustomShortcutName;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Handle key when adding a custom shortcut - entering the name
pub fn handle_adding_custom_shortcut_name_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    match key.code {
        KeyCode::Esc => {
            // Cancel and go back to key input
            app.state.input_mode = InputMode::AddingCustomShortcutKey;
        }
        KeyCode::Enter => {
            // Proceed to command input (name can be empty)
            app.state.input_mode = InputMode::AddingCustomShortcutCommand;
        }
        KeyCode::Char(c) => {
            app.state.new_shortcut_name.push(c);
        }
        KeyCode::Backspace => {
            app.state.new_shortcut_name.pop();
        }
        _ => {}
    }
    Ok(())
}

/// Handle key when adding a custom shortcut - entering the command
pub fn handle_adding_custom_shortcut_command_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    match key.code {
        KeyCode::Esc => {
            // Cancel and go back to name input
            app.state.input_mode = InputMode::AddingCustomShortcutName;
        }
        KeyCode::Enter => {
            // Proceed to auto-close toggle if command is not empty
            if app.state.new_shortcut_command.is_empty() {
                app.state.shortcut_error = Some("Command cannot be empty".to_string());
            } else {
                app.state.shortcut_error = None;
                app.state.input_mode = InputMode::AddingCustomShortcutAutoClose;
            }
        }
        KeyCode::Char(c) => {
            app.state.new_shortcut_command.push(c);
            app.state.shortcut_error = None;
        }
        KeyCode::Backspace => {
            app.state.new_shortcut_command.pop();
        }
        _ => {}
    }
    Ok(())
}

/// Handle key when adding a custom shortcut - toggling auto-close option
pub fn handle_adding_custom_shortcut_auto_close_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    match key.code {
        KeyCode::Esc => {
            // Go back to command input
            app.state.input_mode = InputMode::AddingCustomShortcutCommand;
        }
        KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
            // Toggle auto-close
            app.state.new_shortcut_auto_close = !app.state.new_shortcut_auto_close;
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            app.state.new_shortcut_auto_close = true;
            save_new_shortcut(app);
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            app.state.new_shortcut_auto_close = false;
            save_new_shortcut(app);
        }
        KeyCode::Enter => {
            save_new_shortcut(app);
        }
        _ => {}
    }
    Ok(())
}

/// Save the new shortcut being created and return to the management dialog
fn save_new_shortcut(app: &mut App) {
    if let Some(key) = app.state.new_shortcut_key {
        let shortcut = CustomShortcut::new(
            key,
            app.state.new_shortcut_name.clone(),
            app.state.new_shortcut_command.clone(),
            app.state.new_shortcut_auto_close,
        );

        if let Err(e) = app.config.add_shortcut(shortcut) {
            app.state.shortcut_error = Some(e.to_string());
            return;
        }

        // Save config to disk
        if let Err(e) = app.config.save() {
            tracing::error!("Failed to save config: {}", e);
            app.state.error_message = Some(format!("Failed to save config: {}", e));
        }

        // Clear state and go back to management dialog
        app.state.new_shortcut_key = None;
        app.state.new_shortcut_name.clear();
        app.state.new_shortcut_command.clear();
        app.state.new_shortcut_auto_close = false;
        app.state.shortcut_error = None;
        app.state.input_mode = InputMode::ManagingCustomShortcuts;
    }
}

/// Handle key when confirming custom shortcut deletion
pub fn handle_confirming_custom_shortcut_delete_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            // Confirm deletion
            if let Some(index) = app.state.pending_delete_shortcut_index.take() {
                if index < app.config.custom_shortcuts.len() {
                    app.config.custom_shortcuts.remove(index);

                    // Save config to disk
                    if let Err(e) = app.config.save() {
                        tracing::error!("Failed to save config: {}", e);
                        app.state.error_message = Some(format!("Failed to save config: {}", e));
                    }

                    // Adjust selection if needed
                    let new_count = app.config.custom_shortcuts.len();
                    if app.state.custom_shortcuts_selected >= new_count && new_count > 0 {
                        app.state.custom_shortcuts_selected = new_count - 1;
                    }
                }
            }
            app.state.input_mode = InputMode::ManagingCustomShortcuts;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            // Cancel deletion
            app.state.pending_delete_shortcut_index = None;
            app.state.input_mode = InputMode::ManagingCustomShortcuts;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::session::SessionStore;
    use tempfile::TempDir;
    use uuid::Uuid;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    /// Session manager backed by a temp store - never the real ~/.panoptes
    fn test_sessions(temp_dir: &TempDir) -> SessionManager {
        let config = Config {
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            ..Config::default()
        };
        SessionManager::with_store(
            config,
            SessionStore::with_path(temp_dir.path().join("sessions.json")),
        )
    }

    // ------------------------------------------------------------------
    // Session delete confirmation
    // ------------------------------------------------------------------

    #[test]
    fn test_session_delete_confirm_destroys_session_and_resets_mode() {
        let temp_dir = TempDir::new().unwrap();
        let mut sessions = test_sessions(&temp_dir);

        let project_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();
        let session_id = sessions
            .insert_test_session("doomed", project_id, branch_id)
            .unwrap();

        let mut state = AppState {
            view: View::BranchDetail(project_id, branch_id),
            input_mode: InputMode::ConfirmingSessionDelete,
            pending_delete_session: Some(session_id),
            ..Default::default()
        };

        confirming_session_delete_key(&mut state, &mut sessions, press(KeyCode::Char('y')))
            .unwrap();

        assert!(sessions.get(session_id).is_none(), "session must be gone");
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(state.pending_delete_session.is_none());
    }

    #[test]
    fn test_session_delete_cancel_keeps_session() {
        let temp_dir = TempDir::new().unwrap();
        let mut sessions = test_sessions(&temp_dir);

        let session_id = sessions
            .insert_test_session("kept", Uuid::new_v4(), Uuid::new_v4())
            .unwrap();

        for cancel_key in [KeyCode::Char('n'), KeyCode::Esc] {
            let mut state = AppState {
                input_mode: InputMode::ConfirmingSessionDelete,
                pending_delete_session: Some(session_id),
                ..Default::default()
            };

            confirming_session_delete_key(&mut state, &mut sessions, press(cancel_key)).unwrap();

            assert!(
                sessions.get(session_id).is_some(),
                "cancel must not destroy the session"
            );
            assert_eq!(state.input_mode, InputMode::Normal);
            assert!(state.pending_delete_session.is_none());
        }
    }

    #[test]
    fn test_session_delete_confirm_on_missing_session_resets_mode() {
        let temp_dir = TempDir::new().unwrap();
        let mut sessions = test_sessions(&temp_dir);

        let mut state = AppState {
            input_mode: InputMode::ConfirmingSessionDelete,
            pending_delete_session: Some(Uuid::new_v4()),
            ..Default::default()
        };

        confirming_session_delete_key(&mut state, &mut sessions, press(KeyCode::Char('y')))
            .unwrap();

        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(state.pending_delete_session.is_none());
    }

    // ------------------------------------------------------------------
    // Branch delete confirmation
    // ------------------------------------------------------------------

    #[test]
    fn test_branch_delete_cancel_resets_dialog_state() {
        let mut state = AppState {
            input_mode: InputMode::ConfirmingBranchDelete,
            pending_delete_branch: Some(Uuid::new_v4()),
            delete_worktree_on_disk: true,
            ..Default::default()
        };

        cancel_branch_delete(&mut state);

        assert!(state.pending_delete_branch.is_none());
        assert!(!state.delete_worktree_on_disk);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_branch_delete_confirm_on_missing_branch_resets_without_deleting() {
        let temp_dir = TempDir::new().unwrap();
        let mut sessions = test_sessions(&temp_dir);
        let project_store = ProjectStore::with_path(temp_dir.path().join("projects.json"));

        let mut state = AppState {
            input_mode: InputMode::ConfirmingBranchDelete,
            pending_delete_branch: Some(Uuid::new_v4()),
            delete_worktree_on_disk: true,
            ..Default::default()
        };

        let branch = begin_branch_delete(&mut state, &mut sessions, &project_store);

        assert!(branch.is_none(), "missing branch must abort the delete");
        assert!(state.pending_delete_branch.is_none());
        assert!(!state.delete_worktree_on_disk);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_branch_delete_confirm_destroys_sessions_and_removes_branch() {
        let temp_dir = TempDir::new().unwrap();
        let mut sessions = test_sessions(&temp_dir);
        let mut project_store = ProjectStore::with_path(temp_dir.path().join("projects.json"));
        let claude_config_store =
            ClaudeConfigStore::with_path(temp_dir.path().join("claude_configs.json"));

        let project = crate::project::Project::new(
            "proj".to_string(),
            temp_dir.path().to_path_buf(),
            "main".to_string(),
        );
        let project_id = project.id;
        project_store.add_project(project);

        // A plain (non-worktree) branch: the full 'y' flow runs without any
        // git or on-disk worktree involvement
        let branch = Branch::new(
            project_id,
            "feature".to_string(),
            temp_dir.path().join("feature"),
            false,
            false,
        );
        let branch_id = branch.id;
        project_store.add_branch(branch);

        let session_id = sessions
            .insert_test_session("on-branch", project_id, branch_id)
            .unwrap();

        let mut state = AppState {
            view: View::ProjectDetail(project_id),
            input_mode: InputMode::ConfirmingBranchDelete,
            pending_delete_branch: Some(branch_id),
            active_session: Some(session_id),
            ..Default::default()
        };

        // Mirror the 'y' arm of handle_confirming_branch_delete_key, minus
        // the on-disk removal (delete_worktree_on_disk is false)
        let branch = begin_branch_delete(&mut state, &mut sessions, &project_store)
            .expect("branch exists and must be returned");
        assert!(
            sessions.get(session_id).is_none(),
            "branch sessions must be destroyed"
        );
        assert!(state.active_session.is_none());

        finish_branch_delete(
            &mut state,
            &mut project_store,
            &claude_config_store,
            &branch,
        );

        assert!(project_store.get_branch(branch_id).is_none());
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(!state.delete_worktree_on_disk);
    }
}
