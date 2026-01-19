//! Activity timeline view
//!
//! Shows all sessions sorted by recent activity.

use chrono::Utc;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::AppState;
use crate::project::ProjectStore;
use crate::session::SessionManager;

/// Render the activity timeline view showing all sessions sorted by activity
pub fn render_timeline(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    sessions: &SessionManager,
    project_store: &ProjectStore,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Session list
            Constraint::Length(3), // Footer
        ])
        .split(area);

    // Get all sessions and sort by last_activity (most recent first)
    let mut all_sessions: Vec<_> = sessions.sessions_in_order();
    all_sessions.sort_by(|a, b| b.info.last_activity.cmp(&a.info.last_activity));

    // Header
    let active_count = sessions.total_active_count();
    let header_text = if active_count > 0 {
        format!(
            "Activity Timeline ({} sessions, {} active)",
            all_sessions.len(),
            active_count
        )
    } else {
        format!("Activity Timeline ({} sessions)", all_sessions.len())
    };

    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan).bold())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    // Session list
    if all_sessions.is_empty() {
        let empty = Paragraph::new(
            "No sessions yet.\n\n\
            Press Esc to go back and create a session from a project branch.",
        )
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::ALL).title("Sessions"));
        frame.render_widget(empty, chunks[1]);
    } else {
        let selected_index = state.selected_timeline_index;
        let now = Utc::now();

        let items: Vec<ListItem> = all_sessions
            .iter()
            .enumerate()
            .map(|(i, session)| {
                let selected = i == selected_index;
                let prefix = if selected { "▶ " } else { "  " };

                // Get project/branch info
                let project_name = project_store
                    .get_project(session.info.project_id)
                    .map(|p| p.name.as_str())
                    .unwrap_or("?");
                let branch_name = project_store
                    .get_branch(session.info.branch_id)
                    .map(|b| b.name.as_str())
                    .unwrap_or("?");
                let context = format!("{}/{}", project_name, branch_name);

                // Format time ago
                let duration = now.signed_duration_since(session.info.last_activity);
                let time_ago = format_duration(duration);

                let state_display = session.info.state.display_name();
                let content = format!(
                    "{}{}: {} ({}) [{}] - {}",
                    prefix,
                    i + 1,
                    session.info.name,
                    context,
                    state_display,
                    time_ago
                );

                let style = if selected {
                    Style::default().fg(session.info.state.color()).bold()
                } else {
                    Style::default().fg(session.info.state.color())
                };

                ListItem::new(content).style(style)
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Recent Activity"),
        );
        frame.render_widget(list, chunks[1]);
    }

    // Footer
    let help_text = "j/k: navigate | Enter: open session | Esc: back | q: quit";
    let footer = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
}

/// Format a duration as a human-readable string
fn format_duration(duration: chrono::Duration) -> String {
    let secs = duration.num_seconds();
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        let mins = secs / 60;
        if mins == 1 {
            "1 min ago".to_string()
        } else {
            format!("{} mins ago", mins)
        }
    } else if secs < 86400 {
        let hours = secs / 3600;
        if hours == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{} hours ago", hours)
        }
    } else {
        let days = secs / 86400;
        if days == 1 {
            "1 day ago".to_string()
        } else {
            format!("{} days ago", days)
        }
    }
}
