//! Project detail view
//!
//! Shows branches for a specific project.

use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{AppState, InputMode};
use crate::project::{ProjectId, ProjectStore};
use crate::session::SessionManager;

/// Render the project detail view showing branches
pub fn render_project_detail(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_id: ProjectId,
    project_store: &ProjectStore,
    sessions: &SessionManager,
) {
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
        if active_count > 0 {
            format!("Project: {} ({} active)", project.name, active_count)
        } else {
            format!("Project: {}", project.name)
        }
    } else {
        "Project not found".to_string()
    };

    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan).bold())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    // Branch list or worktree creation dialog
    if state.input_mode == InputMode::CreatingWorktree {
        render_worktree_creation(frame, chunks[1], state);
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

                    let style = if selected {
                        if active_count > 0 {
                            Style::default().fg(Color::Green).bold()
                        } else if branch.is_default {
                            Style::default().fg(Color::Cyan).bold()
                        } else {
                            Style::default().fg(Color::White).bold()
                        }
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
        InputMode::CreatingWorktree => "Type: search | j/k: navigate | Enter: select | Esc: cancel",
        _ => "w: new worktree | j/k: navigate | Enter: open branch | Esc: back | q: quit",
    };
    let footer = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
}

/// Render the worktree creation dialog with branch selector
fn render_worktree_creation(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Search input
            Constraint::Min(0),    // Branch list
        ])
        .split(area);

    // Search input
    let input_text = format!("> {}_", state.new_branch_name);
    let input = Paragraph::new(input_text)
        .style(Style::default().fg(Color::Yellow))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Search / New Branch Name"),
        );
    frame.render_widget(input, chunks[0]);

    // Branch list with "Create new" option
    let mut items: Vec<ListItem> = Vec::new();

    // First item: "Create new branch"
    let create_new_text = if state.new_branch_name.is_empty() {
        "  + Create new branch (type name above)".to_string()
    } else {
        format!("  + Create new branch: '{}'", state.new_branch_name)
    };
    let create_style = if state.branch_selector_index == 0 {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    items.push(ListItem::new(create_new_text).style(create_style));

    // Filtered branches
    for (i, branch) in state.filtered_branches.iter().enumerate() {
        let selected = state.branch_selector_index == i + 1;
        let prefix = if selected { "▶ " } else { "  " };
        let style = if selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        items.push(ListItem::new(format!("{}{}", prefix, branch)).style(style));
    }

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Branches"));
    frame.render_widget(list, chunks[1]);
}
