//! Project detail input handler
//!
//! Handles keyboard input in the project detail view.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{App, ClaudeSettingsMigrateState, InputMode, View};
use crate::claude_json::ClaudeJsonStore;

/// Handle key in project detail view (normal mode)
pub fn handle_project_detail_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Only process key press events (not release/repeat)
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    // Handle focus timer shortcuts (t, T, Ctrl+t)
    if app.handle_focus_timer_shortcut(key) {
        return Ok(());
    }

    let project_id = match app.state.view {
        View::ProjectDetail(id) => id,
        _ => return Ok(()),
    };

    let branch_count = app.project_store.branches_for_project(project_id).len();

    match key.code {
        KeyCode::Esc => {
            app.state.navigate_back();
        }
        KeyCode::Char('q') => {
            app.state.input_mode = InputMode::ConfirmingQuit;
        }
        KeyCode::Down => {
            if branch_count > 0 {
                app.state.selected_branch_index =
                    (app.state.selected_branch_index + 1) % branch_count;
            }
        }
        KeyCode::Up => {
            if branch_count > 0 {
                app.state.selected_branch_index = app
                    .state
                    .selected_branch_index
                    .checked_sub(1)
                    .unwrap_or(branch_count - 1);
            }
        }
        KeyCode::Enter => {
            // Open selected branch
            let branches = app.project_store.branches_for_project_sorted(project_id);
            if let Some(branch) = branches.get(app.state.selected_branch_index) {
                app.state.navigate_to_branch(project_id, branch.id);
            }
        }
        KeyCode::Char('n') => {
            // Start creating a new worktree (new wizard flow)
            app.start_worktree_wizard(project_id);
        }
        KeyCode::Char('b') => {
            // Set default base branch
            app.start_default_base_selection(project_id);
        }
        KeyCode::Char('d') => {
            // Delete selected branch/worktree
            let branches = app.project_store.branches_for_project_sorted(project_id);
            if let Some(branch) = branches.get(app.state.selected_branch_index) {
                // Check for permissions migration first (worktrees only)
                if branch.is_worktree {
                    if let Some(project) = app.project_store.get_project(project_id) {
                        if let Some(migrate_state) = check_claude_settings_for_migrate(
                            app,
                            &branch.working_dir,
                            &project.repo_path,
                            project_id,
                            branch.id,
                        ) {
                            // Show migrate dialog first
                            app.state.pending_claude_settings_migrate = Some(migrate_state);
                            app.state.input_mode = InputMode::ConfirmingClaudeSettingsMigrate;
                            return Ok(());
                        }
                    }
                }

                // No migration needed - proceed to delete confirmation
                app.state.pending_delete_branch = Some(branch.id);
                app.state.delete_worktree_on_disk = branch.is_worktree;
                app.state.input_mode = InputMode::ConfirmingBranchDelete;
            }
        }
        KeyCode::Char('r') => {
            // Start renaming project
            if let Some(project) = app.project_store.get_project(project_id) {
                app.state.new_project_name = project.name.clone();
                app.state.renaming_project = Some(project_id);
                app.state.input_mode = InputMode::RenamingProject;
            }
        }
        KeyCode::Char('R') => {
            // Refresh branch status (check for stale worktrees)
            let stale_count = app.project_store.refresh_branches(project_id);
            if stale_count > 0 {
                app.state.header_notifications.push(format!(
                    "Refreshed: {} worktree{} missing",
                    stale_count,
                    if stale_count == 1 { "" } else { "s" }
                ));
            } else {
                app.state
                    .header_notifications
                    .push("Refreshed: all worktrees OK");
            }
        }
        KeyCode::Char('c') => {
            // Set default Claude config for this project
            let config_count = app.claude_config_store.count();
            if config_count > 0 {
                app.state.available_claude_configs = app
                    .claude_config_store
                    .configs_sorted()
                    .iter()
                    .cloned()
                    .cloned()
                    .collect();

                // Pre-select current project default or global default
                let project_default = app
                    .project_store
                    .get_project(project_id)
                    .and_then(|p| p.default_claude_config);
                let global_default = app.claude_config_store.get_default_id();
                let preferred_id = project_default.or(global_default);

                app.state.claude_config_selector_index = app
                    .state
                    .available_claude_configs
                    .iter()
                    .position(|c| Some(c.id) == preferred_id)
                    .unwrap_or(0);

                app.state.setting_project_default_config = Some(project_id);
                app.state.show_claude_config_selector = true;
                app.state.input_mode = InputMode::SelectingClaudeConfig;
            } else {
                app.state
                    .header_notifications
                    .push("No Claude configs defined. Press 'c' from homepage.");
            }
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            if let Some(num) = c.to_digit(10) {
                if num > 0 && (num as usize) <= branch_count {
                    app.state.selected_branch_index = (num as usize) - 1;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Check if Claude settings should be migrated before worktree deletion
///
/// Returns Some(ClaudeSettingsMigrateState) if the worktree has unique permissions
/// that should be offered for migration to the main repo.
fn check_claude_settings_for_migrate(
    app: &App,
    worktree_path: &std::path::Path,
    main_path: &std::path::Path,
    project_id: crate::project::ProjectId,
    branch_id: crate::project::BranchId,
) -> Option<ClaudeSettingsMigrateState> {
    // Get the Claude config to use (project default or global default)
    let project = app.project_store.get_project(project_id)?;
    let config_id = project
        .default_claude_config
        .or_else(|| app.claude_config_store.get_default_id());

    let claude_config = config_id.and_then(|id| app.claude_config_store.get(id));
    let config_dir = claude_config.and_then(|c| c.config_dir.clone());

    // Create store for this config directory
    let store = ClaudeJsonStore::for_config_dir(config_dir.as_deref())?;

    let worktree_str = worktree_path.to_string_lossy().to_string();
    let main_str = main_path.to_string_lossy().to_string();

    // Compare tools between worktree and main (legacy format)
    let (_, unique_to_worktree, _) = store.compare_tools(&worktree_str, &main_str).ok()?;

    // Check modern format (.claude/settings.local.json)
    let has_local_settings =
        crate::claude_json::has_unique_local_settings(worktree_path, main_path).unwrap_or(false);

    // Only show dialog if EITHER format has unique settings
    if unique_to_worktree.is_empty() && !has_local_settings {
        return None;
    }

    Some(ClaudeSettingsMigrateState {
        worktree_path: worktree_path.to_path_buf(),
        main_path: main_path.to_path_buf(),
        branch_id,
        unique_tools: unique_to_worktree,
        selected_yes: true,
        claude_config_dir: config_dir,
        has_local_settings,
    })
}
