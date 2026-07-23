//! Worktree wizard input handlers
//!
//! Handles keyboard input for the multi-step worktree creation wizard.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{
    cycle_next, cycle_prev, App, ClaudeSettingsCopyState, InputMode, MAX_BRANCH_NAME_LEN,
};
use crate::project::ProjectId;
use crate::wizards::worktree::{filter_branch_refs, BranchRefType, WorktreeCreationType};

// ============================================================================
// New Worktree Creation Wizard Handlers
// ============================================================================

/// Handle key in WorktreeSelectBranch mode (Step 1)
///
/// User can:
/// - Type to filter existing branches
/// - Arrow keys to navigate the list
/// - Enter on existing local branch -> WorktreeConfirm (ExistingLocal)
/// - Enter on remote branch -> WorktreeConfirm (RemoteTracking)
/// - Enter on "Create new branch" -> WorktreeSelectBase
/// - Esc to cancel
pub fn handle_worktree_select_branch_key(
    app: &mut App,
    key: KeyEvent,
    _project_id: ProjectId,
) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    match key.code {
        KeyCode::Esc => {
            cancel_worktree_wizard(app);
        }
        KeyCode::Up => {
            worktree_navigate_branches(app, -1);
        }
        KeyCode::Down => {
            worktree_navigate_branches(app, 1);
        }
        KeyCode::Enter => {
            let filtered_count = app.state.worktree_wizard.filtered_branches.len();
            let has_create_option = !app.state.worktree_wizard.search_text.is_empty();

            // Use checked access for safety
            if let Some(selected) = app
                .state
                .worktree_wizard
                .filtered_branches
                .get(app.state.worktree_wizard.list_index)
                .cloned()
            {
                // Block selection of already-tracked branches
                if selected.is_already_tracked {
                    return Ok(());
                }

                // Check if branch has an untracked git worktree - import instead of create
                if selected.has_git_worktree {
                    app.state.worktree_wizard.source_branch = Some(selected.clone());
                    app.state.worktree_wizard.branch_name = selected.name.clone();
                    app.state.worktree_wizard.creation_type = WorktreeCreationType::ImportExisting;
                    app.state.input_mode = InputMode::WorktreeConfirm;
                } else if selected.ref_type == BranchRefType::Local {
                    // Existing local branch -> go directly to confirm
                    app.state.worktree_wizard.source_branch = Some(selected.clone());
                    app.state.worktree_wizard.branch_name = selected.name.clone();
                    app.state.worktree_wizard.creation_type = WorktreeCreationType::ExistingLocal;
                    app.state.input_mode = InputMode::WorktreeConfirm;
                } else {
                    // Remote branch -> will create tracking branch
                    // Extract local name from remote ref (e.g., "origin/feature" -> "feature")
                    let local_name = selected
                        .name
                        .split_once('/')
                        .map(|(_, b)| b.to_string())
                        .unwrap_or_else(|| selected.name.clone());

                    app.state.worktree_wizard.source_branch = Some(selected);
                    app.state.worktree_wizard.branch_name = local_name;
                    app.state.worktree_wizard.creation_type = WorktreeCreationType::RemoteTracking;
                    app.state.input_mode = InputMode::WorktreeConfirm;
                }
            } else if has_create_option && app.state.worktree_wizard.list_index >= filtered_count {
                // Selected "Create new branch" option - validate the branch name first
                let branch_name = &app.state.worktree_wizard.search_text;
                if let Err(error) = crate::git::validate_branch_name(branch_name) {
                    // Show validation error and don't proceed
                    app.state.worktree_wizard.branch_validation_error = Some(error);
                    return Ok(());
                }

                // Clear any previous error and proceed
                app.state.worktree_wizard.branch_validation_error = None;
                app.state.worktree_wizard.branch_name =
                    app.state.worktree_wizard.search_text.clone();
                app.state.worktree_wizard.creation_type = WorktreeCreationType::NewBranch;

                // Initialize base branch selection (empty search = all branches)
                app.state.worktree_wizard.base_search_text.clear();
                app.state.worktree_wizard.base_list_index = 0;
                update_worktree_filtered_base_branches(app);

                // Find and select the default base branch
                if let Some(idx) = app
                    .state
                    .worktree_wizard
                    .all_branches
                    .iter()
                    .position(|b| b.is_default_base)
                {
                    app.state.worktree_wizard.base_list_index = idx;
                    app.state.worktree_wizard.base_branch =
                        Some(app.state.worktree_wizard.all_branches[idx].clone());
                } else if let Some(first) = app.state.worktree_wizard.all_branches.first() {
                    app.state.worktree_wizard.base_branch = Some(first.clone());
                }

                app.state.input_mode = InputMode::WorktreeSelectBase;
            }
        }
        KeyCode::Backspace => {
            app.state.worktree_wizard.search_text.pop();
            update_worktree_filtered_branches(app);
            // Reset selection to first selectable
            worktree_select_first_selectable(app);
            // Clear any previous validation error when user modifies input
            app.state.worktree_wizard.branch_validation_error = None;
        }
        KeyCode::Char(c) => {
            // Enforce length limit for branch names
            if app.state.worktree_wizard.search_text.len() >= MAX_BRANCH_NAME_LEN {
                return Ok(());
            }
            // Convert space to underscore for branch names
            let c = if c == ' ' { '_' } else { c };
            // Filter out obviously invalid characters as they are typed
            if !crate::git::INVALID_BRANCH_CHARS.contains(&c) && !c.is_ascii_control() {
                app.state.worktree_wizard.search_text.push(c);
                update_worktree_filtered_branches(app);
                // Reset selection to first selectable
                worktree_select_first_selectable(app);
                // Clear any previous validation error when user modifies input
                app.state.worktree_wizard.branch_validation_error = None;
            }
            // If character is invalid, silently reject it (no feedback needed for single chars)
        }
        _ => {}
    }
    Ok(())
}

/// Handle key in WorktreeSelectBase mode (Step 2)
///
/// User can:
/// - Type to filter base branches
/// - Arrow keys to navigate the list
/// - Enter to confirm and go to WorktreeConfirm
/// - Esc to go back to WorktreeSelectBranch
pub fn handle_worktree_select_base_key(
    app: &mut App,
    key: KeyEvent,
    _project_id: ProjectId,
) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    match key.code {
        KeyCode::Esc => {
            // Go back to step 1
            app.state.input_mode = InputMode::WorktreeSelectBranch;
            app.state.worktree_wizard.base_search_text.clear();
            update_worktree_filtered_base_branches(app);
        }
        KeyCode::Up => {
            let wizard = &mut app.state.worktree_wizard;
            wizard.base_list_index =
                cycle_prev(wizard.base_list_index, wizard.filtered_base_branches.len());
            // Update selected base branch
            if let Some(branch) = wizard.filtered_base_branches.get(wizard.base_list_index) {
                wizard.base_branch = Some(branch.clone());
            }
        }
        KeyCode::Down => {
            let wizard = &mut app.state.worktree_wizard;
            wizard.base_list_index =
                cycle_next(wizard.base_list_index, wizard.filtered_base_branches.len());
            // Update selected base branch
            if let Some(branch) = wizard.filtered_base_branches.get(wizard.base_list_index) {
                wizard.base_branch = Some(branch.clone());
            }
        }
        KeyCode::Enter => {
            // Confirm base branch selection, go to confirmation
            let wizard = &mut app.state.worktree_wizard;
            if let Some(branch) = wizard.filtered_base_branches.get(wizard.base_list_index) {
                wizard.base_branch = Some(branch.clone());
            }
            app.state.input_mode = InputMode::WorktreeConfirm;
        }
        KeyCode::Backspace => {
            app.state.worktree_wizard.base_search_text.pop();
            // Reset index when search changes
            app.state.worktree_wizard.base_list_index = 0;
            update_worktree_filtered_base_branches(app);
        }
        KeyCode::Char(c) => {
            // Enforce length limit for base search (same as branch names)
            if app.state.worktree_wizard.base_search_text.len() < MAX_BRANCH_NAME_LEN {
                app.state.worktree_wizard.base_search_text.push(c);
                // Reset index when search changes
                app.state.worktree_wizard.base_list_index = 0;
                update_worktree_filtered_base_branches(app);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Handle key in WorktreeConfirm mode (Step 3)
///
/// User can:
/// - Enter to create the worktree
/// - Esc to go back to the previous step
pub fn handle_worktree_confirm_key(
    app: &mut App,
    key: KeyEvent,
    project_id: ProjectId,
) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    match key.code {
        KeyCode::Esc => {
            // Go back to appropriate step
            match app.state.worktree_wizard.creation_type {
                WorktreeCreationType::NewBranch => {
                    app.state.input_mode = InputMode::WorktreeSelectBase;
                }
                _ => {
                    app.state.input_mode = InputMode::WorktreeSelectBranch;
                }
            }
        }
        KeyCode::Enter => {
            // Everything the wizard collected, before its state is cleared
            let creation_type = app.state.worktree_wizard.creation_type;
            let branch_name = app.state.worktree_wizard.branch_name.clone();
            let base_ref = match creation_type {
                // Create local tracking branch from the selected remote branch
                WorktreeCreationType::RemoteTracking => app
                    .state
                    .worktree_wizard
                    .source_branch
                    .as_ref()
                    .map(|b| b.name.clone()),
                // Create a new branch from the selected base
                WorktreeCreationType::NewBranch => app
                    .state
                    .worktree_wizard
                    .base_branch
                    .as_ref()
                    .map(|b| b.name.clone()),
                _ => None,
            };

            // The wizard is done either way: creating runs in the background
            // (the loading overlay takes over), importing right here.
            cancel_worktree_wizard(app);

            if creation_type == WorktreeCreationType::ImportExisting {
                // Import an existing git worktree that Panoptes does not track.
                // No git work involved, so it stays on this thread.
                match app.import_existing_worktree(project_id, &branch_name) {
                    Ok(branch_id) => app.enter_new_worktree_branch(project_id, branch_id),
                    Err(e) => {
                        tracing::error!("Failed to import worktree: {:#}", e);
                        app.state.error_message = Some(format!("Failed to import worktree: {}", e));
                    }
                }
            } else {
                // Only an existing local branch is checked out as-is
                let create_branch = creation_type != WorktreeCreationType::ExistingLocal;
                if let Err(e) = app.create_worktree(
                    project_id,
                    &branch_name,
                    create_branch,
                    base_ref.as_deref(),
                ) {
                    tracing::error!("Failed to create worktree: {:#}", e);
                    app.state.error_message = Some(format!("Failed to create worktree: {}", e));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

// ============================================================================
// Legacy Worktree Handlers (still used for some flows)
// ============================================================================

/// Handle key when selecting default base branch (via 'b' in project view)
pub fn handle_selecting_default_base_key(
    app: &mut App,
    key: KeyEvent,
    project_id: ProjectId,
) -> Result<()> {
    // Only process key press events (not release/repeat)
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Esc => {
            // Cancel selection
            app.state.input_mode = InputMode::Normal;
            app.state.available_branch_refs.clear();
            app.state.filtered_branch_refs.clear();
            app.state.new_branch_name.clear();
            app.state.fetch_error = None;
        }
        KeyCode::Up => {
            app.state.base_branch_selector_index = cycle_prev(
                app.state.base_branch_selector_index,
                app.state.filtered_branch_refs.len(),
            );
        }
        KeyCode::Down => {
            app.state.base_branch_selector_index = cycle_next(
                app.state.base_branch_selector_index,
                app.state.filtered_branch_refs.len(),
            );
        }
        KeyCode::Enter => {
            // Set selected branch as default base
            if let Some(selected) = app
                .state
                .filtered_branch_refs
                .get(app.state.base_branch_selector_index)
            {
                let branch_name = selected.name.clone();
                set_project_default_base(app, project_id, &branch_name);
            }
            app.state.input_mode = InputMode::Normal;
            app.state.available_branch_refs.clear();
            app.state.filtered_branch_refs.clear();
            app.state.new_branch_name.clear();
            app.state.fetch_error = None;
        }
        KeyCode::Backspace => {
            app.state.new_branch_name.pop();
            app.state.filtered_branch_refs =
                filter_branch_refs(&app.state.available_branch_refs, &app.state.new_branch_name);
            select_default_base_branch(app);
        }
        KeyCode::Char(c) => {
            // Enforce length limit for branch name filter
            if app.state.new_branch_name.len() < MAX_BRANCH_NAME_LEN {
                app.state.new_branch_name.push(c);
                app.state.filtered_branch_refs = filter_branch_refs(
                    &app.state.available_branch_refs,
                    &app.state.new_branch_name,
                );
                select_default_base_branch(app);
            }
        }
        _ => {}
    }
    Ok(())
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Set a project's default base branch and persist it, surfacing failures
fn set_project_default_base(app: &mut App, project_id: ProjectId, branch_name: &str) {
    if let Some(project) = app.project_store.get_project_mut(project_id) {
        project.set_default_base_branch(Some(branch_name.to_string()));
        if let Err(e) = app.project_store.save() {
            tracing::error!("Failed to save default base branch: {}", e);
            app.state.error_message = Some(format!("Failed to save default: {}", e));
        } else {
            tracing::debug!("Set default base branch to: {}", branch_name);
        }
    }
}

/// Update the cached base-branch filter (step 2) for the current search text
///
/// Mirrors [`update_worktree_filtered_branches`] for step 1: handlers and the
/// render path both read `filtered_base_branches` instead of re-filtering.
pub fn update_worktree_filtered_base_branches(app: &mut App) {
    app.state.worktree_wizard.filtered_base_branches = filter_branch_refs(
        &app.state.worktree_wizard.all_branches,
        &app.state.worktree_wizard.base_search_text,
    );
    let count = app.state.worktree_wizard.filtered_base_branches.len();
    app.state.worktree_wizard.clamp_base_list_index(count);
}

/// Navigate up/down in the worktree branch list, skipping already-tracked branches
fn worktree_navigate_branches(app: &mut App, direction: i32) {
    let filtered_count = app.state.worktree_wizard.filtered_branches.len();
    let has_create_option = !app.state.worktree_wizard.search_text.is_empty();
    let total_options = if has_create_option {
        filtered_count + 1
    } else {
        filtered_count
    };

    if total_options == 0 {
        app.state.worktree_wizard.list_index = 0;
        return;
    }

    // cycle_next/cycle_prev clamp a stale index into range before stepping
    let mut next = app.state.worktree_wizard.list_index;

    for _ in 0..total_options {
        next = if direction > 0 {
            cycle_next(next, total_options)
        } else {
            cycle_prev(next, total_options)
        };
        // The "Create new branch" option (at filtered_count) is always selectable
        if next >= filtered_count {
            app.state.worktree_wizard.list_index = next;
            return;
        }
        // Check if this branch is selectable (not already tracked) - use checked access
        if let Some(branch) = app.state.worktree_wizard.filtered_branches.get(next) {
            if !branch.is_already_tracked {
                app.state.worktree_wizard.list_index = next;
                return;
            }
        }
    }
    // If all branches are tracked, stay at current position
}

/// Select the first non-tracked branch in the list
pub fn worktree_select_first_selectable(app: &mut App) {
    let filtered_count = app.state.worktree_wizard.filtered_branches.len();
    let has_create_option = !app.state.worktree_wizard.search_text.is_empty();

    // First, try to find a non-tracked branch
    for (i, branch) in app
        .state
        .worktree_wizard
        .filtered_branches
        .iter()
        .enumerate()
    {
        if !branch.is_already_tracked {
            app.state.worktree_wizard.list_index = i;
            return;
        }
    }
    // If all branches are tracked and there's a create option, select it
    if has_create_option {
        app.state.worktree_wizard.list_index = filtered_count;
        return;
    }
    // Otherwise default to 0
    app.state.worktree_wizard.list_index = 0;
}

/// Cancel and clean up the worktree wizard state
fn cancel_worktree_wizard(app: &mut App) {
    app.state.input_mode = InputMode::Normal;
    app.state.worktree_wizard = Default::default();
    app.state.fetch_error = None;
}

/// Update filtered branches based on search text
pub fn update_worktree_filtered_branches(app: &mut App) {
    if app.state.worktree_wizard.search_text.is_empty() {
        app.state.worktree_wizard.filtered_branches =
            app.state.worktree_wizard.all_branches.clone();
    } else {
        let query = app.state.worktree_wizard.search_text.to_lowercase();
        app.state.worktree_wizard.filtered_branches = app
            .state
            .worktree_wizard
            .all_branches
            .iter()
            .filter(|b| b.name.to_lowercase().contains(&query))
            .cloned()
            .collect();
    }
    // Clamp index to valid range after filtering
    app.state.worktree_wizard.clamp_list_index();
}

/// Check if Claude settings should be copied to a new worktree
///
/// Returns Some(ClaudeSettingsCopyState) if the main repo has Claude settings
/// that should be offered for copying to the new worktree. The file
/// inspection itself lives in [`crate::claude_json::check_settings_for_copy`];
/// this resolves the project's Claude config and packs the dialog state.
pub(crate) fn check_claude_settings_for_copy(
    app: &App,
    project_id: ProjectId,
    branch_id: crate::project::BranchId,
) -> Option<ClaudeSettingsCopyState> {
    // Get project and branch info
    let project = app.project_store.get_project(project_id)?;
    let branch = app.project_store.get_branch(branch_id)?;

    // Get the Claude config to use (project default or global default)
    let config_id = project
        .default_claude_config
        .or_else(|| app.claude_config_store.get_default_id());
    let claude_config = config_id.and_then(|id| app.claude_config_store.get(id));
    let config_dir = claude_config.and_then(|c| c.config_dir.clone());

    let check =
        crate::claude_json::check_settings_for_copy(&project.repo_path, config_dir.as_deref())?;

    Some(ClaudeSettingsCopyState {
        source_path: project.repo_path.clone(),
        target_path: branch.working_dir.clone(),
        project_id,
        branch_id,
        tools_preview: check.tools_preview,
        has_mcp_servers: check.has_mcp_servers,
        selected_yes: true,
        claude_config_dir: config_dir,
        has_local_settings: check.has_local_settings,
    })
}

// TODO: Codex permissions sharing
// When Codex CLI supports per-project permissions (similar to Claude's
// .claude/settings.local.json or .claude.json), implement detection and
// copy dialog here. The pattern should mirror check_claude_settings_for_copy().
// See also: CodexAdapter::setup_hooks() for where Codex config is modified.
//
// Similarly, when Codex CLI supports per-project permissions, implement
// migration of unique worktree permissions back to main repo on deletion.
// Pattern should mirror check_claude_settings_for_migrate() in
// src/input/normal/project_detail.rs.

/// Select the default base branch in the filtered list (legacy flow)
fn select_default_base_branch(app: &mut App) {
    // Find default base branch in the available (unfiltered) list first
    // This ensures we track the actual default even when filtered out
    if let Some(default_branch) = app
        .state
        .available_branch_refs
        .iter()
        .find(|b| b.is_default_base)
    {
        app.state.selected_base_branch = Some(default_branch.clone());
    } else if let Some(first) = app.state.available_branch_refs.first() {
        // If no default, use first available branch
        app.state.selected_base_branch = Some(first.clone());
    } else {
        app.state.selected_base_branch = None;
    }

    // Find index of default base branch in filtered list for UI highlighting
    if let Some(idx) = app
        .state
        .filtered_branch_refs
        .iter()
        .position(|b| b.is_default_base)
    {
        app.state.base_branch_selector_index = idx;
    } else if !app.state.filtered_branch_refs.is_empty() {
        // If no default in filtered list, select first item
        app.state.base_branch_selector_index = 0;
    }
}
