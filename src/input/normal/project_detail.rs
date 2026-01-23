//! Project detail input handler
//!
//! Handles keyboard input in the project detail view.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{App, InputMode, View};

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
                app.state.pending_delete_branch = Some(branch.id);
                app.state.delete_worktree_on_disk = branch.is_worktree; // Default to deleting if it's a worktree
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
