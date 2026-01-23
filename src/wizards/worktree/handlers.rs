//! Worktree wizard input handlers
//!
//! Handles keyboard input for the multi-step worktree creation wizard.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{App, InputMode};
use crate::project::ProjectId;
use crate::wizards::worktree::{filter_branch_refs, BranchRef, BranchRefType, WorktreeCreationType};

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
    project_id: ProjectId,
) -> Result<()> {
    // project_id reserved for potential future use
    let _ = project_id;

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

            if app.state.worktree_wizard.list_index < filtered_count {
                // Selected an existing branch
                let selected = app.state.worktree_wizard.filtered_branches
                    [app.state.worktree_wizard.list_index]
                    .clone();

                // Block selection of already-tracked branches
                if selected.is_already_tracked {
                    return Ok(());
                }

                if selected.ref_type == BranchRefType::Local {
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
            } else if has_create_option {
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

                // Initialize base branch selection
                app.state.worktree_wizard.base_search_text.clear();
                app.state.worktree_wizard.base_list_index = 0;

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
    project_id: ProjectId,
) -> Result<()> {
    // project_id reserved for potential future use (e.g., setting default base branch)
    let _ = project_id;

    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    // Filter branches based on base search text
    let filtered: Vec<BranchRef> = if app.state.worktree_wizard.base_search_text.is_empty() {
        app.state.worktree_wizard.all_branches.clone()
    } else {
        let query = app.state.worktree_wizard.base_search_text.to_lowercase();
        app.state
            .worktree_wizard
            .all_branches
            .iter()
            .filter(|b| b.name.to_lowercase().contains(&query))
            .cloned()
            .collect()
    };
    let filtered_count = filtered.len();

    match key.code {
        KeyCode::Esc => {
            // Go back to step 1
            app.state.input_mode = InputMode::WorktreeSelectBranch;
            app.state.worktree_wizard.base_search_text.clear();
        }
        KeyCode::Up => {
            if filtered_count > 0 {
                app.state.worktree_wizard.base_list_index = app
                    .state
                    .worktree_wizard
                    .base_list_index
                    .checked_sub(1)
                    .unwrap_or(filtered_count - 1);
                // Update selected base branch
                if let Some(branch) = filtered.get(app.state.worktree_wizard.base_list_index) {
                    app.state.worktree_wizard.base_branch = Some(branch.clone());
                }
            }
        }
        KeyCode::Down => {
            if filtered_count > 0 {
                app.state.worktree_wizard.base_list_index =
                    (app.state.worktree_wizard.base_list_index + 1) % filtered_count;
                // Update selected base branch
                if let Some(branch) = filtered.get(app.state.worktree_wizard.base_list_index) {
                    app.state.worktree_wizard.base_branch = Some(branch.clone());
                }
            }
        }
        KeyCode::Enter => {
            // Confirm base branch selection, go to confirmation
            if let Some(branch) = filtered.get(app.state.worktree_wizard.base_list_index) {
                app.state.worktree_wizard.base_branch = Some(branch.clone());
            }
            app.state.input_mode = InputMode::WorktreeConfirm;
        }
        KeyCode::Backspace => {
            app.state.worktree_wizard.base_search_text.pop();
            app.state.worktree_wizard.base_list_index = 0;
        }
        KeyCode::Char(c) => {
            app.state.worktree_wizard.base_search_text.push(c);
            app.state.worktree_wizard.base_list_index = 0;
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
            // Create the worktree
            let result = match app.state.worktree_wizard.creation_type {
                WorktreeCreationType::ExistingLocal => {
                    // Create worktree from existing local branch (don't create branch)
                    app.create_worktree(
                        project_id,
                        &app.state.worktree_wizard.branch_name.clone(),
                        false,
                        None,
                    )
                }
                WorktreeCreationType::RemoteTracking => {
                    // Create local tracking branch from remote, then worktree
                    let base_ref = app
                        .state
                        .worktree_wizard
                        .source_branch
                        .as_ref()
                        .map(|b| b.name.clone());
                    app.create_worktree(
                        project_id,
                        &app.state.worktree_wizard.branch_name.clone(),
                        true,
                        base_ref.as_deref(),
                    )
                }
                WorktreeCreationType::NewBranch => {
                    // Create new branch from base, then worktree
                    let base_ref = app
                        .state
                        .worktree_wizard
                        .base_branch
                        .as_ref()
                        .map(|b| b.name.clone());
                    app.create_worktree(
                        project_id,
                        &app.state.worktree_wizard.branch_name.clone(),
                        true,
                        base_ref.as_deref(),
                    )
                }
            };

            // Capture branch_id before canceling wizard (which clears state)
            let created_branch_id = match &result {
                Ok(branch_id) => Some(*branch_id),
                Err(e) => {
                    tracing::error!("Failed to create worktree: {}", e);
                    app.state.error_message = Some(format!("Failed to create worktree: {}", e));
                    None
                }
            };

            cancel_worktree_wizard(app);

            // Navigate to the newly created branch
            if let Some(branch_id) = created_branch_id {
                app.state.navigate_to_branch(project_id, branch_id);
            }
        }
        _ => {}
    }
    Ok(())
}

// ============================================================================
// Legacy Worktree Handlers (still used for some flows)
// ============================================================================

/// Handle key when creating worktree (legacy flow)
///
/// New flow:
/// - Type branch name to create NEW branch (leave empty to checkout existing)
/// - Navigate list to select base branch (for new) or target branch (for checkout)
/// - Press 's' to set current selection as default base
pub fn handle_creating_worktree_key(
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
            // Cancel worktree creation
            app.state.input_mode = InputMode::Normal;
            app.state.new_branch_name.clear();
            app.state.available_branch_refs.clear();
            app.state.filtered_branch_refs.clear();
            app.state.selected_base_branch = None;
            app.state.fetch_error = None;
        }
        KeyCode::Up => {
            // Navigate up (wrapping)
            let count = app.state.filtered_branch_refs.len();
            if count > 0 {
                app.state.base_branch_selector_index = app
                    .state
                    .base_branch_selector_index
                    .checked_sub(1)
                    .unwrap_or(count - 1);
                // Update selected_base_branch when navigating
                app.state.selected_base_branch = app
                    .state
                    .filtered_branch_refs
                    .get(app.state.base_branch_selector_index)
                    .cloned();
            }
        }
        KeyCode::Down => {
            // Navigate down (wrapping)
            let count = app.state.filtered_branch_refs.len();
            if count > 0 {
                app.state.base_branch_selector_index =
                    (app.state.base_branch_selector_index + 1) % count;
                // Update selected_base_branch when navigating
                app.state.selected_base_branch = app
                    .state
                    .filtered_branch_refs
                    .get(app.state.base_branch_selector_index)
                    .cloned();
            }
        }
        KeyCode::Char('s') if key.modifiers.is_empty() => {
            // Set current selection as default base branch
            if let Some(selected) = app
                .state
                .filtered_branch_refs
                .get(app.state.base_branch_selector_index)
            {
                let branch_name = selected.name.clone();
                if let Some(project) = app.project_store.get_project_mut(project_id) {
                    project.set_default_base_branch(Some(branch_name.clone()));
                    if let Err(e) = app.project_store.save() {
                        tracing::error!("Failed to save default base branch: {}", e);
                        app.state.error_message = Some(format!("Failed to save default: {}", e));
                    } else {
                        // Update the is_default_base flags in our list
                        for branch_ref in &mut app.state.available_branch_refs {
                            branch_ref.is_default_base = branch_ref.name == branch_name;
                        }
                        app.state.filtered_branch_refs = filter_branch_refs(
                            &app.state.available_branch_refs,
                            &app.state.new_branch_name,
                        );
                        tracing::debug!("Set default base branch to: {}", branch_name);
                    }
                }
            }
        }
        KeyCode::Enter => {
            let branch_name_typed = std::mem::take(&mut app.state.new_branch_name);
            let selected_idx = app.state.base_branch_selector_index;
            let selected_branch = app.state.filtered_branch_refs.get(selected_idx).cloned();

            let result: Result<()> = if !branch_name_typed.is_empty() {
                // Create NEW branch from selected base, then create worktree
                // Use selected_base_branch which is preserved even when filtered out
                let base_ref = app
                    .state
                    .selected_base_branch
                    .as_ref()
                    .map(|b| b.name.clone());
                app.create_worktree(project_id, &branch_name_typed, true, base_ref.as_deref())
                    .map(|_| ())
            } else if let Some(selected) = selected_branch {
                // Checkout existing branch as worktree (empty name = checkout selected)
                // For local branches, just create worktree
                // For remote branches, need to create tracking branch first
                let branch_name = if selected.ref_type == BranchRefType::Remote {
                    // Extract branch name from remote ref (e.g., "origin/feature" -> "feature")
                    selected
                        .name
                        .split_once('/')
                        .map(|(_, b)| b.to_string())
                        .unwrap_or(selected.name.clone())
                } else {
                    selected.name.clone()
                };

                // For remote branches, we create a new local branch tracking it
                let create_branch = selected.ref_type == BranchRefType::Remote;
                let base_ref = if create_branch {
                    Some(selected.name.as_str())
                } else {
                    None
                };

                app.create_worktree(project_id, &branch_name, create_branch, base_ref)
                    .map(|_| ())
            } else {
                Ok(())
            };

            if let Err(e) = result {
                tracing::error!("Failed to create worktree: {}", e);
                app.state.error_message = Some(format!("Failed to create worktree: {}", e));
            }
            app.state.input_mode = InputMode::Normal;
            app.state.available_branch_refs.clear();
            app.state.filtered_branch_refs.clear();
            app.state.selected_base_branch = None;
            app.state.fetch_error = None;
        }
        KeyCode::Backspace => {
            app.state.new_branch_name.pop();
            app.state.filtered_branch_refs = filter_branch_refs(
                &app.state.available_branch_refs,
                &app.state.new_branch_name,
            );
            // Find and select the default base branch if exists
            select_default_base_branch(app);
        }
        KeyCode::Char(c) => {
            app.state.new_branch_name.push(c);
            app.state.filtered_branch_refs = filter_branch_refs(
                &app.state.available_branch_refs,
                &app.state.new_branch_name,
            );
            // Find and select the default base branch if exists
            select_default_base_branch(app);
        }
        _ => {}
    }
    Ok(())
}

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
            let count = app.state.filtered_branch_refs.len();
            if count > 0 {
                app.state.base_branch_selector_index = app
                    .state
                    .base_branch_selector_index
                    .checked_sub(1)
                    .unwrap_or(count - 1);
            }
        }
        KeyCode::Down => {
            let count = app.state.filtered_branch_refs.len();
            if count > 0 {
                app.state.base_branch_selector_index =
                    (app.state.base_branch_selector_index + 1) % count;
            }
        }
        KeyCode::Enter => {
            // Set selected branch as default base
            if let Some(selected) = app
                .state
                .filtered_branch_refs
                .get(app.state.base_branch_selector_index)
            {
                let branch_name = selected.name.clone();
                if let Some(project) = app.project_store.get_project_mut(project_id) {
                    project.set_default_base_branch(Some(branch_name.clone()));
                    if let Err(e) = app.project_store.save() {
                        tracing::error!("Failed to save default base branch: {}", e);
                        app.state.error_message = Some(format!("Failed to save default: {}", e));
                    } else {
                        tracing::debug!("Set default base branch to: {}", branch_name);
                    }
                }
            }
            app.state.input_mode = InputMode::Normal;
            app.state.available_branch_refs.clear();
            app.state.filtered_branch_refs.clear();
            app.state.new_branch_name.clear();
            app.state.fetch_error = None;
        }
        KeyCode::Backspace => {
            app.state.new_branch_name.pop();
            app.state.filtered_branch_refs = filter_branch_refs(
                &app.state.available_branch_refs,
                &app.state.new_branch_name,
            );
            select_default_base_branch(app);
        }
        KeyCode::Char(c) => {
            app.state.new_branch_name.push(c);
            app.state.filtered_branch_refs = filter_branch_refs(
                &app.state.available_branch_refs,
                &app.state.new_branch_name,
            );
            select_default_base_branch(app);
        }
        _ => {}
    }
    Ok(())
}

// ============================================================================
// Helper Functions
// ============================================================================

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
        return;
    }

    let current = app.state.worktree_wizard.list_index;
    let mut next = current;

    for _ in 0..total_options {
        if direction > 0 {
            next = (next + 1) % total_options;
        } else {
            next = next.checked_sub(1).unwrap_or(total_options - 1);
        }
        // The "Create new branch" option (at filtered_count) is always selectable
        if next >= filtered_count {
            app.state.worktree_wizard.list_index = next;
            return;
        }
        // Check if this branch is selectable (not already tracked)
        if !app.state.worktree_wizard.filtered_branches[next].is_already_tracked {
            app.state.worktree_wizard.list_index = next;
            return;
        }
    }
    // If all branches are tracked, stay at current position
}

/// Select the first non-tracked branch in the list
fn worktree_select_first_selectable(app: &mut App) {
    let filtered_count = app.state.worktree_wizard.filtered_branches.len();
    let has_create_option = !app.state.worktree_wizard.search_text.is_empty();

    // First, try to find a non-tracked branch
    for (i, branch) in app.state.worktree_wizard.filtered_branches.iter().enumerate() {
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
fn update_worktree_filtered_branches(app: &mut App) {
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
}

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
