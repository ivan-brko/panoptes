//! Project detail view
//!
//! Shows branches for a specific project.

use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{AppState, InputMode};
use crate::config::Config;
use crate::project::{Project, ProjectId, ProjectStore};
use crate::session::SessionManager;
use crate::tui::theme::theme;

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
        InputMode::ConfirmingProjectDelete => "y: confirm delete | n/Esc: cancel",
        InputMode::CreatingWorktree => "Type: search | j/k: navigate | Enter: select | Esc: cancel",
        _ => "d: delete project | w: new worktree | j/k: navigate | Enter: open branch | Esc: back | q: quit",
    };
    let footer = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
}

/// Render the worktree creation dialog with branch selector
fn render_worktree_creation(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Search input
            Constraint::Min(0),    // Branch list
        ])
        .split(area);

    // Search input
    let input_text = format!("> {}_", state.new_branch_name);
    let input = Paragraph::new(input_text).style(t.input_style()).block(
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
        Style::default().fg(t.active).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.text_muted)
    };
    items.push(ListItem::new(create_new_text).style(create_style));

    // Filtered branches
    for (i, branch) in state.filtered_branches.iter().enumerate() {
        let selected = state.branch_selector_index == i + 1;
        let prefix = if selected { "▶ " } else { "  " };
        let style = if selected {
            t.selected_style()
        } else {
            Style::default().fg(t.text)
        };
        items.push(ListItem::new(format!("{}{}", prefix, branch)).style(style));
    }

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Branches"));
    frame.render_widget(list, chunks[1]);
}

/// Render the project delete confirmation dialog
fn render_project_delete_confirmation(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project: Option<&Project>,
    sessions: &SessionManager,
    _config: &Config,
) {
    let t = theme();

    let project_name = project.map(|p| p.name.as_str()).unwrap_or("Unknown");
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

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Delete project: ", Style::default().fg(t.text)),
            Span::styled(project_name, Style::default().fg(Color::Cyan).bold()),
            Span::styled("?", Style::default().fg(t.text)),
        ]),
        Line::from(""),
    ];

    // Warning about active sessions
    if active_count > 0 {
        lines.push(Line::from(vec![Span::styled(
            format!(
                "⚠  Warning: {} active session{} will be terminated",
                active_count,
                if active_count == 1 { "" } else { "s" }
            ),
            Style::default().fg(Color::Yellow).bold(),
        )]));
        lines.push(Line::from(""));
    } else if session_count > 0 {
        lines.push(Line::from(vec![Span::styled(
            format!(
                "Note: {} session{} will be removed",
                session_count,
                if session_count == 1 { "" } else { "s" }
            ),
            Style::default().fg(t.text_muted),
        )]));
        lines.push(Line::from(""));
    }

    // Note about worktrees
    lines.push(Line::from(vec![Span::styled(
        "Note: Git worktrees on disk will NOT be deleted.",
        Style::default().fg(t.text_muted),
    )]));
    lines.push(Line::from(""));
    lines.push(Line::from(""));

    // Confirmation prompt
    lines.push(Line::from(vec![
        Span::styled("Press ", Style::default().fg(t.text)),
        Span::styled("y", Style::default().fg(Color::Green).bold()),
        Span::styled(" to confirm, ", Style::default().fg(t.text)),
        Span::styled("n", Style::default().fg(Color::Red).bold()),
        Span::styled(" or ", Style::default().fg(t.text)),
        Span::styled("Esc", Style::default().fg(Color::Red).bold()),
        Span::styled(" to cancel", Style::default().fg(t.text)),
    ]));

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title("Confirm Delete"),
    );

    frame.render_widget(paragraph, area);
}
