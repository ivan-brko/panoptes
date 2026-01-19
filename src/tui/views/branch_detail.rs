//! Branch detail view
//!
//! Shows sessions for a specific branch.

use chrono::Utc;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{AppState, InputMode};
use crate::config::Config;
use crate::project::{BranchId, ProjectId, ProjectStore};
use crate::session::{SessionManager, SessionState};
use crate::tui::theme::theme;

/// Render the branch detail view showing sessions
#[allow(clippy::too_many_arguments)]
pub fn render_branch_detail(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_id: ProjectId,
    branch_id: BranchId,
    project_store: &ProjectStore,
    sessions: &SessionManager,
    config: &Config,
) {
    let idle_threshold = config.idle_threshold_secs;
    let project = project_store.get_project(project_id);
    let branch = project_store.get_branch(branch_id);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Session list
            Constraint::Length(3), // Footer
        ])
        .split(area);

    // Header
    let header_text = match (project, branch) {
        (Some(project), Some(branch)) => {
            let active_count = sessions.active_session_count_for_branch(branch_id);
            let attention_count = sessions.attention_count_for_branch(branch_id, idle_threshold);

            let mut parts = vec![format!("{} / {}", project.name, branch.name)];
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
        }
        _ => "Branch not found".to_string(),
    };

    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan).bold())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    // Main content area - either session creation input, delete confirmation, or session list
    if state.input_mode == InputMode::CreatingSession {
        render_session_creation(frame, chunks[1], state);
    } else if state.input_mode == InputMode::ConfirmingSessionDelete {
        render_delete_confirmation(frame, chunks[1], state, sessions);
    } else if let Some(branch) = branch {
        let branch_sessions = sessions.sessions_for_branch(branch_id);

        if branch_sessions.is_empty() {
            let empty_text = format!(
                "No sessions on this branch yet.\n\n\
                Press 'n' to create a new session.\n\n\
                Working directory: {}",
                branch.working_dir.display()
            );
            let empty = Paragraph::new(empty_text)
                .style(Style::default().fg(Color::DarkGray))
                .block(Block::default().borders(Borders::ALL).title("Sessions"));
            frame.render_widget(empty, chunks[1]);
        } else {
            let selected_index = state.selected_session_index;
            let now = Utc::now();

            let items: Vec<ListItem> = branch_sessions
                .iter()
                .enumerate()
                .map(|(i, session)| {
                    let selected = i == selected_index;
                    let prefix = if selected { "▶ " } else { "  " };

                    // Check if session needs attention
                    let needs_attention = sessions.session_needs_attention(session, idle_threshold);

                    // Build state display with idle duration if applicable
                    let state_display = match &session.info.state {
                        SessionState::Idle => {
                            let duration = now.signed_duration_since(session.info.last_activity);
                            let mins = duration.num_minutes();
                            format!("Idle - {}m", mins)
                        }
                        state => state.display_name().to_string(),
                    };

                    // Build attention badge
                    let (badge, badge_color) = if needs_attention {
                        match &session.info.state {
                            SessionState::Waiting => ("● ", Color::Green),
                            SessionState::Idle => ("● ", Color::Yellow),
                            _ => ("  ", Color::White),
                        }
                    } else {
                        ("  ", Color::White)
                    };

                    let content = Line::from(vec![
                        Span::raw(prefix),
                        Span::styled(badge, Style::default().fg(badge_color)),
                        Span::raw(format!(
                            "{}: {} [{}]",
                            i + 1,
                            session.info.name,
                            state_display
                        )),
                    ]);

                    let style = if selected {
                        Style::default().fg(session.info.state.color()).bold()
                    } else {
                        Style::default().fg(session.info.state.color())
                    };

                    ListItem::new(content).style(style)
                })
                .collect();

            let title = format!("Sessions ({})", branch_sessions.len());
            let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
            frame.render_widget(list, chunks[1]);
        }
    } else {
        let error = Paragraph::new("Branch not found")
            .style(Style::default().fg(Color::Red))
            .block(Block::default().borders(Borders::ALL).title("Error"));
        frame.render_widget(error, chunks[1]);
    }

    // Footer
    let help_text = match state.input_mode {
        InputMode::CreatingSession => "Enter: create | Esc: cancel",
        InputMode::ConfirmingSessionDelete => "y: confirm delete | n/Esc: cancel",
        _ => {
            "n: new session | d: delete | j/k: navigate | Enter: open session | Esc: back | q: quit"
        }
    };
    let footer = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
}

/// Render the session creation input
fn render_session_creation(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();
    let input = Paragraph::new(format!("New session name: {}_", state.new_session_name))
        .style(t.input_style())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Create Session"),
        );
    frame.render_widget(input, area);
}

/// Render the delete confirmation dialog
fn render_delete_confirmation(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    sessions: &SessionManager,
) {
    let t = theme();
    let session_name = state
        .pending_delete_session
        .and_then(|id| sessions.get(id))
        .map(|s| s.info.name.as_str())
        .unwrap_or("Unknown");

    let text = format!(
        "Are you sure you want to delete session '{}'?\n\n\
        This will kill the Claude Code process.\n\n\
        Press 'y' to confirm or 'n' to cancel.",
        session_name
    );

    let dialog = Paragraph::new(text)
        .style(t.input_style())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Confirm Delete")
                .border_style(Style::default().fg(t.error_bg)),
        );
    frame.render_widget(dialog, area);
}
