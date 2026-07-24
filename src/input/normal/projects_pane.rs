//! Pane 1 input: the project tree and its drill-downs
//!
//! One handler per level, dispatched on [`ProjectsNav`]. `Esc` pops one level
//! and does nothing at the root: this pane is home, the place `Esc` backs out
//! to from everywhere else, and it never quits.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{
    cycle_next, cycle_prev, App, FolderMoveTarget, InputMode, ProjectsNav, SessionDraft,
};
use crate::claude_json::ClaudeJsonStore;
use crate::input::agent_configs::{open_config_selector, AgentKind};
use crate::project::{self, BranchId, ProjectId, RowRef};
use crate::tui::views::pane_projects::PROJECT_SETTINGS_ROWS;

/// Handle a normal-mode key while pane 1 has focus
pub fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    match app.state.projects_nav {
        ProjectsNav::Overview => handle_overview_key(app, key),
        ProjectsNav::Project(project_id) => handle_project_key(app, key, project_id),
        ProjectsNav::Branch(project_id, branch_id) => {
            handle_branch_key(app, key, project_id, branch_id)
        }
        ProjectsNav::ProjectSettings(project_id) => {
            handle_project_settings_key(app, key, project_id)
        }
    }
}

// ========================================================================
// Overview: the folder tree
// ========================================================================

fn handle_overview_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let row_count = project::row_count(&app.project_store);

    match key.code {
        KeyCode::Esc => {
            // Root of the pane, and Esc's home: nowhere further to back out
        }
        KeyCode::Char('n') => {
            app.state.input_mode = InputMode::AddingProject;
            app.state.new_project_path.clear();
        }
        KeyCode::Down => {
            app.state.selected_project_index =
                cycle_next(app.state.selected_project_index, row_count);
        }
        KeyCode::Up => {
            app.state.selected_project_index =
                cycle_prev(app.state.selected_project_index, row_count);
        }
        KeyCode::Right => {
            if let Some(RowRef::Folder { path, expanded, .. }) = selected_row(app) {
                if !expanded {
                    set_folder_collapsed(app, &path, false);
                }
            }
        }
        KeyCode::Left => match selected_row(app) {
            Some(RowRef::Folder { path, expanded, .. }) if expanded => {
                set_folder_collapsed(app, &path, true);
            }
            Some(_) => {
                if let Some(parent) =
                    project::parent_row_index(&app.project_store, app.state.selected_project_index)
                {
                    app.state.selected_project_index = parent;
                }
            }
            None => {}
        },
        KeyCode::Enter => match selected_row(app) {
            Some(RowRef::Project(project_id)) => app.state.navigate_to_project(project_id),
            Some(RowRef::Folder { path, expanded, .. }) => {
                set_folder_collapsed(app, &path, expanded)
            }
            None => {}
        },
        KeyCode::Char('m') => {
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
                        .map(|p| project::folder_path_key(&p.folder))
                        .unwrap_or_default(),
                    FolderMoveTarget::Folder(path) => {
                        project::folder_path_key(&path[..path.len() - 1])
                    }
                };
                app.state.moving_to_folder = Some(target);
                app.state.folder_error = None;
                app.state.input_mode = InputMode::MovingToFolder;
            }
        }
        KeyCode::Char('r') => {
            if let Some(RowRef::Folder { path, .. }) = selected_row(app) {
                app.state.folder_input = path.last().cloned().unwrap_or_default();
                app.state.renaming_folder = Some(path);
                app.state.folder_error = None;
                app.state.input_mode = InputMode::RenamingFolder;
            }
        }
        KeyCode::Char('d') => match selected_row(app) {
            Some(RowRef::Project(project_id)) => {
                app.state.pending_delete_project = Some(project_id);
                app.state.input_mode = InputMode::ConfirmingProjectDelete;
            }
            Some(RowRef::Folder { path, .. }) => {
                app.state.pending_remove_folder = Some(path);
                app.state.input_mode = InputMode::ConfirmingFolderRemove;
            }
            None => {}
        },
        KeyCode::Char('R') => {
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

// ========================================================================
// Project: the branch list
// ========================================================================

fn handle_project_key(app: &mut App, key: KeyEvent, project_id: ProjectId) -> Result<()> {
    let branch_count = app.project_store.branches_for_project(project_id).len();

    match key.code {
        KeyCode::Esc => {
            app.escape_back();
        }
        KeyCode::Down => {
            app.state.selected_branch_index =
                cycle_next(app.state.selected_branch_index, branch_count);
        }
        KeyCode::Up => {
            app.state.selected_branch_index =
                cycle_prev(app.state.selected_branch_index, branch_count);
        }
        KeyCode::Enter => {
            let branches = app.project_store.branches_for_project_sorted(project_id);
            if let Some(branch) = branches.get(app.state.selected_branch_index) {
                let branch_id = branch.id;
                app.state.navigate_to_branch(project_id, branch_id);
            }
        }
        KeyCode::Char(',') => {
            app.state.navigate_to_project_settings(project_id);
        }
        KeyCode::Char('n') => {
            if let Err(e) = app.start_worktree_wizard(project_id) {
                tracing::error!("Failed to start worktree wizard: {:#}", e);
                app.state.error_message = Some(format!("{:#}", e));
            }
        }
        KeyCode::Char('d') => {
            let branches = app.project_store.branches_for_project_sorted(project_id);
            let Some(branch) = branches.get(app.state.selected_branch_index) else {
                return Ok(());
            };
            let (branch_id, is_worktree, working_dir) =
                (branch.id, branch.is_worktree, branch.working_dir.clone());

            // Offer to migrate Claude permissions before a worktree goes
            if is_worktree {
                if let Some(project) = app.project_store.get_project(project_id) {
                    let repo_path = project.repo_path.clone();
                    if let Some(migrate_state) = check_claude_settings_for_migrate(
                        app,
                        &working_dir,
                        &repo_path,
                        project_id,
                        branch_id,
                    ) {
                        app.state.pending_claude_settings_migrate = Some(migrate_state);
                        app.state.input_mode = InputMode::ConfirmingClaudeSettingsMigrate;
                        return Ok(());
                    }
                }
            }

            app.state.pending_delete_branch = Some(branch_id);
            app.state.delete_worktree_on_disk = is_worktree;
            app.state.input_mode = InputMode::ConfirmingBranchDelete;
        }
        KeyCode::Char('R') => {
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
        KeyCode::Char(c) if c.is_ascii_digit() => {
            if let Some(num) = c.to_digit(10) {
                let num = num as usize;
                if num > 0 && num <= branch_count {
                    app.state.selected_branch_index = num - 1;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Check if Claude settings should be migrated before worktree deletion
///
/// Returns `Some` when the worktree has permissions the main repo does not,
/// which are worth offering to keep.
fn check_claude_settings_for_migrate(
    app: &App,
    worktree_path: &std::path::Path,
    main_path: &std::path::Path,
    project_id: ProjectId,
    branch_id: BranchId,
) -> Option<crate::app::ClaudeSettingsMigrateState> {
    let project = app.project_store.get_project(project_id)?;
    let config_id = project
        .default_claude_config
        .or_else(|| app.claude_config_store.get_default_id());

    let claude_config = config_id.and_then(|id| app.claude_config_store.get(id));
    let config_dir = claude_config.and_then(|c| c.config_dir.clone());

    let store = ClaudeJsonStore::for_config_dir(config_dir.as_deref())?;

    let worktree_str = worktree_path.to_string_lossy().to_string();
    let main_str = main_path.to_string_lossy().to_string();

    // Legacy format (.claude.json keyed by path)
    let (_, unique_to_worktree, _) = store.compare_tools(&worktree_str, &main_str).ok()?;

    // Modern format (.claude/settings.local.json)
    let has_local_settings =
        crate::claude_json::has_unique_local_settings(worktree_path, main_path).unwrap_or(false);

    if unique_to_worktree.is_empty() && !has_local_settings {
        return None;
    }

    Some(crate::app::ClaudeSettingsMigrateState {
        worktree_path: worktree_path.to_path_buf(),
        main_path: main_path.to_path_buf(),
        branch_id,
        unique_tools: unique_to_worktree,
        selected_yes: true,
        claude_config_dir: config_dir,
        has_local_settings,
    })
}

// ========================================================================
// Branch: the session list
// ========================================================================

fn handle_branch_key(
    app: &mut App,
    key: KeyEvent,
    project_id: ProjectId,
    branch_id: BranchId,
) -> Result<()> {
    // Entries mix live sessions with ones recoverable from a previous run, so
    // the selected index may land on a session that has no process attached
    let branch_sessions: Vec<(uuid::Uuid, bool)> = app
        .sessions
        .entries_for_branch(branch_id)
        .iter()
        .map(|entry| (entry.info.id, entry.live))
        .collect();
    let session_count = branch_sessions.len();

    match key.code {
        KeyCode::Esc => {
            app.escape_back();
        }
        KeyCode::Down => {
            app.state.branch_session_index =
                cycle_next(app.state.branch_session_index, session_count);
        }
        KeyCode::Up => {
            app.state.branch_session_index =
                cycle_prev(app.state.branch_session_index, session_count);
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            // Sessions are numbered in the list; jump to one by its number
            // (1-indexed, 0 means session 10). Digits are reserved, so this
            // never collides with a custom shortcut.
            if let Some(num) = c.to_digit(10) {
                let target = if num == 0 { 9 } else { (num as usize) - 1 };
                if target < session_count {
                    app.state.branch_session_index = target;
                }
            }
        }
        KeyCode::Enter => {
            if let Some(&(session_id, live)) = branch_sessions.get(app.state.branch_session_index) {
                // A recovered session has no process yet: bring it back before
                // opening a view onto it
                if !live {
                    match app.resume_recovered_session(session_id) {
                        Ok(true) => {}
                        Ok(false) => return Ok(()),
                        Err(e) => {
                            tracing::error!(
                                session_id = %session_id,
                                error = %e,
                                "Failed to resume session"
                            );
                            app.state.error_message = Some(format!("Could not resume: {}", e));
                            return Ok(());
                        }
                    }
                }
                app.activate_session(session_id)?;
            }
        }
        KeyCode::Char('n') => {
            if let Some(branch) = app.project_store.get_branch(branch_id) {
                app.state.session_draft =
                    SessionDraft::for_branch(project_id, branch_id, branch.working_dir.clone());
                app.state.agent_type_selector_index = 0;
                app.state.input_mode = InputMode::SelectingAgentType;
            }
        }
        KeyCode::Char('s') => {
            if let Some(branch) = app.project_store.get_branch(branch_id) {
                app.state.session_draft =
                    SessionDraft::for_branch(project_id, branch_id, branch.working_dir.clone());
                app.state.input_mode = InputMode::CreatingShellSession;
            }
        }
        KeyCode::Char('d') => {
            if let Some(&(session_id, _)) = branch_sessions.get(app.state.branch_session_index) {
                app.state.pending_delete_session = Some(session_id);
                app.state.input_mode = InputMode::ConfirmingSessionDelete;
            }
        }
        KeyCode::Char(c) => {
            if let Some(shortcut) = app.config.get_shortcut(c).cloned() {
                if let Some(branch) = app.project_store.get_branch(branch_id) {
                    let working_dir = branch.working_dir.clone();
                    if let Some(new_session_id) = super::launch_shortcut_session(
                        app,
                        &shortcut,
                        project_id,
                        branch_id,
                        working_dir,
                    ) {
                        app.state.navigate_to_session(new_session_id);
                        app.tui.enable_mouse_capture();
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

// ========================================================================
// Project settings: the four flows `,` replaced c/x/b/r with
// ========================================================================

fn handle_project_settings_key(app: &mut App, key: KeyEvent, project_id: ProjectId) -> Result<()> {
    let row_count = PROJECT_SETTINGS_ROWS.len();

    match key.code {
        KeyCode::Esc => {
            app.escape_back();
        }
        KeyCode::Down => {
            app.state.project_settings_index =
                cycle_next(app.state.project_settings_index, row_count);
        }
        KeyCode::Up => {
            app.state.project_settings_index =
                cycle_prev(app.state.project_settings_index, row_count);
        }
        KeyCode::Enter => match app.state.project_settings_index {
            0 => open_project_default_config(app, project_id, AgentKind::Claude),
            1 => open_project_default_config(app, project_id, AgentKind::Codex),
            2 => app.start_default_base_selection(project_id),
            3 => {
                if let Some(project) = app.project_store.get_project(project_id) {
                    app.state.new_project_name = project.name.clone();
                    app.state.renaming_project = Some(project_id);
                    app.state.input_mode = InputMode::RenamingProject;
                }
            }
            _ => {}
        },
        _ => {}
    }
    Ok(())
}

/// Open the config selector to pick a project's default Claude/Codex config
///
/// Shows a hint instead when no configs of that kind exist yet.
fn open_project_default_config(app: &mut App, project_id: ProjectId, kind: AgentKind) {
    let config_count = match kind {
        AgentKind::Claude => app.claude_config_store.count(),
        AgentKind::Codex => app.codex_config_store.count(),
    };
    if config_count > 0 {
        app.state.setting_project_default_config = Some(project_id);
        open_config_selector(app, kind, Some(project_id));
    } else {
        app.state.header_notifications.push(kind.no_configs_hint());
    }
}
