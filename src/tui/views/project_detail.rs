//! Project detail view
//!
//! Shows branches for a specific project.

use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{AppState, BranchRefType, InputMode};
use crate::config::Config;
use crate::project::{Project, ProjectId, ProjectStore};
use crate::session::SessionManager;
use crate::tui::theme::theme;
use crate::tui::views::confirm::{render_confirm_dialog, ConfirmDialogConfig};
use crate::tui::views::format_attention_hint;

/// Render the project detail view showing branches
pub fn render_project_detail(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_id: ProjectId,
    project_store: &ProjectStore,
    sessions: &SessionManager,
    config: &Config,
) {
    let idle_threshold = config.idle_threshold_secs;
    let project = project_store.get_project(project_id);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Branch list
            Constraint::Length(3), // Footer
        ])
        .split(area);

    // Header
    let header_text = if let Some(project) = project {
        let active_count = sessions.active_session_count_for_project(project_id);
        let attention_count = sessions.attention_count_for_project(project_id, idle_threshold);

        let mut parts = vec![format!("Project: {}", project.name)];
        if active_count > 0 {
            parts.push(format!("{} active", active_count));
        }
        if attention_count > 0 {
            parts.push(format!("{} need attention", attention_count));
        }
        if parts.len() == 1 {
            parts[0].clone()
        } else {
            format!("{} ({})", parts[0], parts[1..].join(", "))
        }
    } else {
        "Project not found".to_string()
    };

    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan).bold())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    // Branch list, worktree creation dialog, or delete confirmation
    if state.input_mode == InputMode::ConfirmingProjectDelete {
        render_project_delete_confirmation(frame, chunks[1], state, project, sessions, config);
    } else if state.input_mode == InputMode::CreatingWorktree {
        render_worktree_creation(frame, chunks[1], state, project);
    } else if state.input_mode == InputMode::SelectingDefaultBase {
        render_default_base_selection(frame, chunks[1], state);
    } else if state.input_mode == InputMode::FetchingBranches {
        render_fetching_branches(frame, chunks[1]);
    } else if state.input_mode == InputMode::RenamingProject {
        render_rename_dialog(frame, chunks[1], state);
    } else if let Some(_project) = project {
        let branches = project_store.branches_for_project_sorted(project_id);

        if branches.is_empty() {
            let empty = Paragraph::new(
                "No branches tracked yet.\n\n\
                Press 'w' to create a worktree for a branch.",
            )
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::ALL).title("Branches"));
            frame.render_widget(empty, chunks[1]);
        } else {
            let selected_index = state.selected_branch_index;

            let items: Vec<ListItem> = branches
                .iter()
                .enumerate()
                .map(|(i, branch)| {
                    let selected = i == selected_index;
                    let prefix = if selected { "▶ " } else { "  " };

                    // Count sessions for this branch
                    let session_count = sessions.session_count_for_branch(branch.id);
                    let active_count = sessions.active_session_count_for_branch(branch.id);
                    let attention_count =
                        sessions.attention_count_for_branch(branch.id, idle_threshold);

                    let status = if branch.is_default {
                        if active_count > 0 {
                            format!("(default) {} active", active_count)
                        } else if session_count > 0 {
                            format!("(default) {} sessions", session_count)
                        } else {
                            "(default)".to_string()
                        }
                    } else if branch.is_worktree {
                        if active_count > 0 {
                            format!("(worktree) {} active", active_count)
                        } else if session_count > 0 {
                            format!("(worktree) {} sessions", session_count)
                        } else {
                            "(worktree)".to_string()
                        }
                    } else if active_count > 0 {
                        format!("{} active", active_count)
                    } else if session_count > 0 {
                        format!("{} sessions", session_count)
                    } else {
                        String::new()
                    };

                    let content = if status.is_empty() {
                        format!("{}{}: {}", prefix, i + 1, branch.name)
                    } else {
                        format!("{}{}: {} {}", prefix, i + 1, branch.name, status)
                    };

                    // Color precedence: Yellow (attention) > Green (active) > Cyan (default) > White
                    let style = if selected {
                        if attention_count > 0 {
                            Style::default().fg(Color::Yellow).bold()
                        } else if active_count > 0 {
                            Style::default().fg(Color::Green).bold()
                        } else if branch.is_default {
                            Style::default().fg(Color::Cyan).bold()
                        } else {
                            Style::default().fg(Color::White).bold()
                        }
                    } else if attention_count > 0 {
                        Style::default().fg(Color::Yellow)
                    } else if active_count > 0 {
                        Style::default().fg(Color::Green)
                    } else if branch.is_default {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::White)
                    };

                    ListItem::new(content).style(style)
                })
                .collect();

            let title = format!("Branches ({})", branches.len());
            let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
            frame.render_widget(list, chunks[1]);
        }
    } else {
        let error = Paragraph::new("Project not found")
            .style(Style::default().fg(Color::Red))
            .block(Block::default().borders(Borders::ALL).title("Error"));
        frame.render_widget(error, chunks[1]);
    }

    // Footer
    let help_text = match state.input_mode {
        InputMode::ConfirmingProjectDelete => "y: confirm delete | n/Esc: cancel".to_string(),
        InputMode::CreatingWorktree => {
            "Type: name | ↑/↓: select base | s: set default | Enter: create | Esc: cancel"
                .to_string()
        }
        InputMode::SelectingDefaultBase => {
            "Type: filter | ↑/↓: navigate | Enter: set default | Esc: cancel".to_string()
        }
        InputMode::FetchingBranches => "Fetching branches... | Esc: cancel".to_string(),
        InputMode::RenamingProject => "Type: project name | Enter: save | Esc: cancel".to_string(),
        _ => {
            let base =
                "w: new worktree | b: set default base | r: rename | d: delete | ↑/↓: navigate | Enter: open | Esc: back | q: quit";
            if let Some(hint) = format_attention_hint(sessions, config) {
                format!("{} | {}", hint, base)
            } else {
                base.to_string()
            }
        }
    };
    let footer = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
}

/// Render the worktree creation dialog with base branch selector
fn render_worktree_creation(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project: Option<&Project>,
) {
    let t = theme();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Branch name input
            Constraint::Length(1), // Instructions
            Constraint::Min(0),    // Base branch list
        ])
        .split(area);

    // Branch name input
    let input_title = if state.new_branch_name.is_empty() {
        "New Branch Name (leave empty to checkout existing)"
    } else {
        "New Branch Name"
    };
    let input_text = format!("> {}_", state.new_branch_name);
    let input = Paragraph::new(input_text)
        .style(t.input_style())
        .block(Block::default().borders(Borders::ALL).title(input_title));
    frame.render_widget(input, chunks[0]);

    // Instructions line
    let fetch_warning = if state.fetch_error.is_some() {
        " (fetch failed, showing cached)"
    } else {
        ""
    };
    let default_info = project
        .and_then(|p| p.default_base_branch.as_ref())
        .map(|b| format!(" Default: {}", b))
        .unwrap_or_default();
    let instructions = format!(
        "Select base branch{}{} | s: set as default",
        fetch_warning, default_info
    );
    let instruction_widget = Paragraph::new(instructions).style(Style::default().fg(t.text_muted));
    frame.render_widget(instruction_widget, chunks[1]);

    // Base branch list
    let items: Vec<ListItem> = state
        .filtered_branch_refs
        .iter()
        .enumerate()
        .map(|(i, branch_ref)| {
            let selected = i == state.base_branch_selector_index;
            let prefix = if selected { "▶ " } else { "  " };
            let type_prefix = match branch_ref.ref_type {
                BranchRefType::Local => "[L]",
                BranchRefType::Remote => "[R]",
            };
            let default_marker = if branch_ref.is_default_base { " *" } else { "" };

            let content = format!(
                "{}{} {}{}",
                prefix, type_prefix, branch_ref.name, default_marker
            );

            let style = if selected {
                if branch_ref.is_default_base {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    t.selected_style()
                }
            } else if branch_ref.is_default_base {
                Style::default().fg(Color::Cyan)
            } else if branch_ref.ref_type == BranchRefType::Local {
                Style::default().fg(t.text)
            } else {
                Style::default().fg(t.text_muted)
            };

            ListItem::new(content).style(style)
        })
        .collect();

    let title = format!("Base Branch ({} options)", state.filtered_branch_refs.len());
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, chunks[2]);
}

/// Render the default base branch selection dialog
fn render_default_base_selection(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Filter input
            Constraint::Min(0),    // Branch list
        ])
        .split(area);

    // Filter input
    let input_text = format!("> {}_", state.new_branch_name);
    let input = Paragraph::new(input_text).style(t.input_style()).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Filter branches"),
    );
    frame.render_widget(input, chunks[0]);

    // Branch list
    let items: Vec<ListItem> = state
        .filtered_branch_refs
        .iter()
        .enumerate()
        .map(|(i, branch_ref)| {
            let selected = i == state.base_branch_selector_index;
            let prefix = if selected { "▶ " } else { "  " };
            let type_prefix = match branch_ref.ref_type {
                BranchRefType::Local => "[L]",
                BranchRefType::Remote => "[R]",
            };
            let default_marker = if branch_ref.is_default_base {
                " (current default)"
            } else {
                ""
            };

            let content = format!(
                "{}{} {}{}",
                prefix, type_prefix, branch_ref.name, default_marker
            );

            let style = if selected {
                if branch_ref.is_default_base {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    t.selected_style()
                }
            } else if branch_ref.is_default_base {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(t.text)
            };

            ListItem::new(content).style(style)
        })
        .collect();

    let title = format!(
        "Select Default Base Branch ({} options)",
        state.filtered_branch_refs.len()
    );
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, chunks[1]);
}

/// Render a spinner while fetching branches
fn render_fetching_branches(frame: &mut Frame, area: Rect) {
    let t = theme();
    let content = Paragraph::new("Fetching branches from remotes...")
        .style(Style::default().fg(t.text_muted))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Please wait"));
    frame.render_widget(content, area);
}

/// Render the project rename dialog
fn render_rename_dialog(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();
    let input_text = format!("> {}_", state.new_project_name);
    let input = Paragraph::new(input_text).style(t.input_style()).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Rename Project"),
    );
    frame.render_widget(input, area);
}

/// Render the project delete confirmation dialog
pub fn render_project_delete_confirmation(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project: Option<&Project>,
    sessions: &SessionManager,
    _config: &Config,
) {
    let project_name = project
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    let project_id = state.pending_delete_project;

    // Count active sessions for this project
    let (session_count, active_count) = if let Some(pid) = project_id {
        (
            sessions.session_count_for_project(pid),
            sessions.active_session_count_for_project(pid),
        )
    } else {
        (0, 0)
    };

    let mut warnings = vec![];
    let mut notes = vec![];

    // Warning about active sessions
    if active_count > 0 {
        warnings.push(format!(
            "{} active session{} will be terminated",
            active_count,
            if active_count == 1 { "" } else { "s" }
        ));
    } else if session_count > 0 {
        notes.push(format!(
            "{} session{} will be removed",
            session_count,
            if session_count == 1 { "" } else { "s" }
        ));
    }

    // Note about worktrees
    notes.push("Git worktrees on disk will NOT be deleted.".to_string());

    let config = ConfirmDialogConfig {
        title: "Confirm Delete",
        item_label: "project",
        item_name: &project_name,
        warnings,
        notes,
    };
    render_confirm_dialog(frame, area, config);
}
