//! Projects overview view
//!
//! Displays a list of projects with their branch/session counts,
//! and a "quick sessions" section for sessions in the current directory.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{AppState, InputMode};
use crate::project::ProjectStore;
use crate::session::SessionManager;

/// Render the projects overview
pub fn render_projects_overview(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Main content
            Constraint::Length(3), // Footer/help
        ])
        .split(area);

    // Header
    let active_count = sessions.total_active_count();
    let header_text = if active_count > 0 {
        format!(
            "Panoptes - {} projects, {} active sessions",
            project_store.project_count(),
            active_count
        )
    } else {
        format!("Panoptes - {} projects", project_store.project_count())
    };
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan).bold())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    // Main content area
    if state.input_mode == InputMode::CreatingSession {
        render_session_creation(frame, chunks[1], state);
    } else {
        render_main_content(frame, chunks[1], state, project_store, sessions);
    }

    // Footer with help
    let help_text = match state.input_mode {
        InputMode::CreatingSession => "Enter: create | Esc: cancel",
        _ => {
            if project_store.project_count() > 0 || !sessions.is_empty() {
                "a: add project | n: new session | t: timeline | j/k: navigate | Enter: open | q: quit"
            } else {
                "a: add project | n: new session | t: timeline | q: quit"
            }
        }
    };
    let footer = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
}

/// Render the session creation input
fn render_session_creation(frame: &mut Frame, area: Rect, state: &AppState) {
    let input = Paragraph::new(format!("New session name: {}_", state.new_session_name))
        .style(Style::default().fg(Color::Yellow))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Create Session"),
        );
    frame.render_widget(input, area);
}

/// Render the main content area with projects and sessions
fn render_main_content(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
) {
    let has_projects = project_store.project_count() > 0;
    let has_sessions = !sessions.is_empty();

    if !has_projects && !has_sessions {
        // Empty state
        let empty_text = "No projects yet.\n\n\
            Press 'a' to add a git repository as a project,\n\
            or 'n' to create a quick session in the current directory.";
        let empty = Paragraph::new(empty_text)
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title("Welcome"));
        frame.render_widget(empty, area);
        return;
    }

    // Split area: projects on top, sessions on bottom (if both exist)
    if has_projects && has_sessions {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);

        render_project_list(frame, split[0], state, project_store, sessions);
        render_quick_sessions(frame, split[1], state, sessions);
    } else if has_projects {
        render_project_list(frame, area, state, project_store, sessions);
    } else {
        render_quick_sessions(frame, area, state, sessions);
    }
}

/// Render the project list
fn render_project_list(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
) {
    let selected_index = state.selected_project_index;
    let projects = project_store.projects_sorted();

    let items: Vec<ListItem> = projects
        .iter()
        .enumerate()
        .map(|(i, project)| {
            let selected = i == selected_index;
            let prefix = if selected { "▶ " } else { "  " };

            // Count branches and active sessions for this project
            let branch_count = project_store.branch_count_for_project(project.id);
            let session_count = sessions.session_count_for_project(project.id);
            let active_count = sessions.active_session_count_for_project(project.id);

            let status = if active_count > 0 {
                format!("{} branches, {} active", branch_count, active_count)
            } else if session_count > 0 {
                format!("{} branches, {} sessions", branch_count, session_count)
            } else {
                format!("{} branches", branch_count)
            };

            let content = format!("{}{}: {} ({})", prefix, i + 1, project.name, status);

            let style = if selected {
                if active_count > 0 {
                    Style::default().fg(Color::Green).bold()
                } else {
                    Style::default().fg(Color::Cyan).bold()
                }
            } else if active_count > 0 {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };

            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Projects"));
    frame.render_widget(list, area);
}

/// Render quick sessions (sessions not tied to a specific project)
fn render_quick_sessions(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    sessions: &SessionManager,
) {
    // For now, show all sessions. Later we can filter by project_id == nil
    let selected_index = state.selected_session_index;
    let session_list = sessions.sessions_in_order();

    let items: Vec<ListItem> = session_list
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let state_color = session.info.state.color();
            let selected = i == selected_index;
            let prefix = if selected { "▶ " } else { "  " };
            let content = format!(
                "{}{}: {} [{}]",
                prefix,
                i + 1,
                session.info.name,
                session.info.state.display_name()
            );
            let style = if selected {
                Style::default().fg(state_color).bold()
            } else {
                Style::default().fg(state_color)
            };
            ListItem::new(content).style(style)
        })
        .collect();

    let title = format!("Sessions ({})", session_list.len());
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, area);
}
