//! Activity timeline view
//!
//! Shows all sessions sorted by recent activity.

use chrono::Utc;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::AppState;
use crate::config::Config;
use crate::focus_timing::FocusTimer;
use crate::project::ProjectStore;
use crate::session::{Session, SessionManager, SessionState};
use crate::tui::theme::theme;
use crate::tui::views::Breadcrumb;
use crate::tui::views::{format_attention_hint, format_focus_timer_hint, format_header_with_timer};

/// Render the activity timeline view showing all sessions sorted by activity
#[allow(clippy::too_many_arguments)]
pub fn render_timeline(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    sessions: &SessionManager,
    project_store: &ProjectStore,
    config: &Config,
    focus_timer: Option<&FocusTimer>,
) {
    let idle_threshold = config.idle_threshold_secs;
    let attention_count = sessions.total_attention_count(idle_threshold);
    let attention_sessions = sessions.sessions_needing_attention(idle_threshold);

    // Dynamic layout based on whether we have sessions needing attention
    let chunks = if attention_count > 0 {
        let attention_height = (attention_sessions.len() + 2).min(8) as u16; // Cap at 8 lines
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),                // Header
                Constraint::Length(attention_height), // Attention section
                Constraint::Min(0),                   // Session list
                Constraint::Length(3),                // Footer
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(0),    // Session list
                Constraint::Length(3), // Footer
            ])
            .split(area)
    };

    // Get all sessions and sort by last_activity (most recent first)
    let mut all_sessions: Vec<_> = sessions.sessions_in_order();
    all_sessions.sort_by(|a, b| b.info.last_activity.cmp(&a.info.last_activity));

    // Header with breadcrumb
    let t = theme();
    let active_count = sessions.total_active_count();
    let breadcrumb_text = {
        let breadcrumb = Breadcrumb::new().push("Timeline");
        let mut status_parts = vec![format!("{} sessions", all_sessions.len())];
        if active_count > 0 {
            status_parts.push(format!("{} active", active_count));
        }
        if attention_count > 0 {
            status_parts.push(format!("{} need attention", attention_count));
        }
        breadcrumb.display_with_suffix(&format!("({})", status_parts.join(", ")))
    };
    let header_text = format_header_with_timer(&breadcrumb_text, focus_timer, area.width);

    let header = Paragraph::new(header_text)
        .style(t.header_style())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    // Attention section (if needed)
    let main_chunk_index = if attention_count > 0 {
        render_attention_section(
            frame,
            chunks[1],
            &attention_sessions,
            project_store,
            idle_threshold,
        );
        2
    } else {
        1
    };

    // Session list
    if all_sessions.is_empty() {
        let empty = Paragraph::new(
            "No sessions yet.\n\n\
            Press Esc to go back and create a session from a project branch.",
        )
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::ALL).title("Sessions"));
        frame.render_widget(empty, chunks[main_chunk_index]);
    } else {
        let selected_index = state.selected_timeline_index;
        let now = Utc::now();

        let items: Vec<ListItem> = all_sessions
            .iter()
            .enumerate()
            .map(|(i, session)| {
                let selected = i == selected_index;
                let prefix = if selected { "▶ " } else { "  " };

                // Check if session needs attention
                let needs_attention = sessions.session_needs_attention(session, idle_threshold);

                // Get project/branch info
                let project_name = project_store
                    .get_project(session.info.project_id)
                    .map(|p| p.name.as_str())
                    .unwrap_or("?");
                let branch_name = project_store
                    .get_branch(session.info.branch_id)
                    .map(|b| b.name.as_str())
                    .unwrap_or("?");

                // Format time ago
                let duration = now.signed_duration_since(session.info.last_activity);
                let time_ago = format_duration(duration);

                // Build state display with idle duration if applicable
                let state_display = match &session.info.state {
                    SessionState::Idle => {
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

                // Format: project / branch / session [state] - time
                let content = Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(badge, Style::default().fg(badge_color)),
                    Span::raw(format!(
                        "{}: {} / {} / {} [{}] - {}",
                        i + 1,
                        project_name,
                        branch_name,
                        session.info.name,
                        state_display,
                        time_ago
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

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Recent Activity"),
        );
        frame.render_widget(list, chunks[main_chunk_index]);
    }

    // Footer
    let footer_index = if attention_count > 0 { 3 } else { 2 };
    let timer_hint = format_focus_timer_hint(state.focus_timer.is_some());
    let base_help = format!("{} | ↑/↓: navigate | Enter: open | Esc/q: back", timer_hint);
    let help_text = if let Some(hint) = format_attention_hint(sessions, config) {
        format!("{} | {}", hint, base_help)
    } else {
        base_help
    };
    let footer = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[footer_index]);
}

/// Render the "Needs Attention" section for timeline view
fn render_attention_section(
    frame: &mut Frame,
    area: Rect,
    attention_sessions: &[&Session],
    project_store: &ProjectStore,
    _idle_threshold_secs: u64,
) {
    let now = Utc::now();

    let items: Vec<ListItem> = attention_sessions
        .iter()
        .map(|session| {
            // Get project/branch info
            let project_name = project_store
                .get_project(session.info.project_id)
                .map(|p| p.name.as_str())
                .unwrap_or("?");
            let branch_name = project_store
                .get_branch(session.info.branch_id)
                .map(|b| b.name.as_str())
                .unwrap_or("?");

            let (badge_color, state_text) = match &session.info.state {
                SessionState::Waiting => (Color::Green, "[Waiting]".to_string()),
                SessionState::Idle => {
                    let duration = now.signed_duration_since(session.info.last_activity);
                    let mins = duration.num_minutes();
                    (Color::Yellow, format!("[Idle - {}m]", mins))
                }
                _ => (
                    Color::White,
                    format!("[{}]", session.info.state.display_name()),
                ),
            };

            let content = Line::from(vec![
                Span::styled("● ", Style::default().fg(badge_color)),
                Span::raw(format!(
                    "{} / {} / {} {}",
                    project_name, branch_name, session.info.name, state_text
                )),
            ]);

            ListItem::new(content)
        })
        .collect();

    let title = format!("Needs Attention ({})", attention_sessions.len());
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(list, area);
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
