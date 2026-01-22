//! Focus statistics view
//!
//! Displays completed focus timer sessions and aggregated statistics.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::focus_timing::stats::{aggregate_by_project, calculate_overall_stats, FocusSession};
use crate::focus_timing::FocusTimer;
use crate::project::ProjectStore;
use crate::tui::theme::theme;
use crate::tui::views::{format_focus_timer_hint, format_header_with_timer, Breadcrumb};

/// Render the focus statistics view
#[allow(clippy::too_many_arguments)]
pub fn render_focus_stats(
    frame: &mut Frame,
    area: Rect,
    sessions: &[FocusSession],
    project_store: &ProjectStore,
    selected_index: usize,
    focus_events_supported: bool,
    focus_timer: Option<&FocusTimer>,
) {
    let t = theme();

    // Layout: header, stats summary, session list, footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(6), // Summary stats
            Constraint::Min(0),    // Session list
            Constraint::Length(3), // Footer
        ])
        .split(area);

    // Header with breadcrumb
    let overall = calculate_overall_stats(sessions);
    let breadcrumb_text = {
        let breadcrumb = Breadcrumb::new().push("Focus Stats");
        let status = format!(
            "({} sessions, {} avg focus)",
            overall.session_count,
            overall.format_average()
        );
        breadcrumb.display_with_suffix(&status)
    };
    let header_text = format_header_with_timer(&breadcrumb_text, focus_timer, area.width);

    let header = Paragraph::new(header_text)
        .style(t.header_style())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    // Summary statistics
    render_summary_section(
        frame,
        chunks[1],
        &overall,
        sessions,
        project_store,
        focus_events_supported,
    );

    // Session list
    render_session_list(frame, chunks[2], sessions, project_store, selected_index);

    // Footer
    let timer_hint = format_focus_timer_hint(focus_timer.map(|t| t.is_running()).unwrap_or(false));
    let footer_text = format!(
        "{} | j/k: navigate | Esc/q: back | Enter: details",
        timer_hint
    );
    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[3]);
}

/// Render the summary statistics section
fn render_summary_section(
    frame: &mut Frame,
    area: Rect,
    overall: &crate::focus_timing::stats::AggregatedStats,
    sessions: &[FocusSession],
    project_store: &ProjectStore,
    focus_events_supported: bool,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Left: Overall stats
    let mut overall_lines = vec![
        Line::from(vec![
            Span::raw("Sessions: "),
            Span::styled(
                overall.session_count.to_string(),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::raw("Total focused: "),
            Span::styled(
                overall.format_total_focused(),
                Style::default().fg(Color::Green),
            ),
        ]),
        Line::from(vec![
            Span::raw("Avg focus: "),
            Span::styled(overall.format_average(), Style::default().fg(Color::Yellow)),
        ]),
    ];

    if !focus_events_supported {
        overall_lines.push(Line::from(Span::styled(
            "(Focus tracking unavailable)",
            Style::default().fg(Color::DarkGray).italic(),
        )));
    }

    let overall_widget = Paragraph::new(overall_lines)
        .block(Block::default().borders(Borders::ALL).title("Overall"));
    frame.render_widget(overall_widget, chunks[0]);

    // Right: Per-project breakdown (top 3)
    let project_stats = aggregate_by_project(sessions);
    let mut project_lines: Vec<Line> = Vec::new();

    // Sort projects by session count
    let mut project_vec: Vec<_> = project_stats.iter().collect();
    project_vec.sort_by(|a, b| b.1.session_count.cmp(&a.1.session_count));

    for (project_id, stats) in project_vec.iter().take(3) {
        let project_name = project_store
            .get_project(**project_id)
            .map(|p| p.name.as_str())
            .unwrap_or("Unknown");

        project_lines.push(Line::from(vec![
            Span::raw(format!("{}: ", project_name)),
            Span::styled(
                format!("{} sess", stats.session_count),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(", "),
            Span::styled(stats.format_average(), Style::default().fg(Color::Yellow)),
        ]));
    }

    if project_lines.is_empty() {
        project_lines.push(Line::from(Span::styled(
            "No project data yet",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let project_widget = Paragraph::new(project_lines)
        .block(Block::default().borders(Borders::ALL).title("By Project"));
    frame.render_widget(project_widget, chunks[1]);
}

/// Render the session list
fn render_session_list(
    frame: &mut Frame,
    area: Rect,
    sessions: &[FocusSession],
    project_store: &ProjectStore,
    selected_index: usize,
) {
    if sessions.is_empty() {
        let empty = Paragraph::new(
            "No focus sessions yet.\n\n\
            Press 't' from any view to start a focus timer.",
        )
        .style(Style::default().fg(Color::DarkGray))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Recent Sessions"),
        );
        frame.render_widget(empty, area);
        return;
    }

    // Sort sessions by completed_at (most recent first)
    let mut sorted_sessions: Vec<_> = sessions.iter().collect();
    sorted_sessions.sort_by(|a, b| b.completed_at.cmp(&a.completed_at));

    let items: Vec<ListItem> = sorted_sessions
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let selected = i == selected_index;
            let prefix = if selected { "▶ " } else { "  " };

            // Get project/branch names
            let project_name = session
                .project_id
                .and_then(|id| project_store.get_project(id))
                .map(|p| p.name.as_str())
                .unwrap_or("-");

            let branch_name = session
                .branch_id
                .and_then(|id| project_store.get_branch(id))
                .map(|b| b.name.as_str())
                .unwrap_or("-");

            // Format the time
            let time_str = session.completed_at.format("%m/%d %H:%M").to_string();

            // Build the display line
            let content = Line::from(vec![
                Span::raw(prefix),
                Span::styled(
                    session.format_percentage(),
                    Style::default().fg(if session.focus_percentage >= 80.0 {
                        Color::Green
                    } else if session.focus_percentage >= 50.0 {
                        Color::Yellow
                    } else {
                        Color::Red
                    }),
                ),
                Span::raw(format!(
                    " | {} → {} | {} / {} | {}",
                    session.format_target(),
                    session.format_focused(),
                    project_name,
                    branch_name,
                    time_str
                )),
            ]);

            let style = if selected {
                Style::default().bold()
            } else {
                Style::default()
            };

            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Recent Sessions ({})", sessions.len())),
    );
    frame.render_widget(list, area);
}

/// Render the timer input dialog
pub fn render_timer_input_dialog(frame: &mut Frame, area: Rect, input: &str, default_minutes: u64) {
    // Center the dialog
    let dialog_width = 30_u16;
    let dialog_height = 7_u16;

    let x = area.width.saturating_sub(dialog_width) / 2;
    let y = area.height.saturating_sub(dialog_height) / 2;

    let dialog_area = Rect {
        x,
        y,
        width: dialog_width,
        height: dialog_height,
    };

    // Clear the background
    frame.render_widget(ratatui::widgets::Clear, dialog_area);

    // Build content
    let display_value = if input.is_empty() {
        format!("{}", default_minutes)
    } else {
        input.to_string()
    };

    let content = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Duration: "),
            Span::styled(
                format!("{}_", display_value),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(" min"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  [Enter] Start  [Esc] Cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let dialog = Paragraph::new(content).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" Start Focus Timer "),
    );

    frame.render_widget(dialog, dialog_area);
}

#[cfg(test)]
mod tests {
    use crate::focus_timing::stats::format_duration;

    #[test]
    fn test_format_duration_helper() {
        use std::time::Duration;
        assert_eq!(format_duration(Duration::from_secs(65)), "01:05");
    }
}
