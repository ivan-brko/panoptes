//! Log viewer view
//!
//! Displays log entries with scrolling and colored levels.

use std::sync::Arc;

use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use crate::logging::{LogBuffer, LogFileInfo, LogLevel};

/// Render the log viewer showing all log entries
pub fn render_log_viewer(
    frame: &mut Frame,
    area: Rect,
    log_buffer: &Arc<LogBuffer>,
    log_file_info: &LogFileInfo,
    scroll_offset: usize,
    auto_scroll: bool,
) {
    let entries = log_buffer.all_entries();
    let entry_count = entries.len();

    // Layout: header, content, footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Log entries
            Constraint::Length(3), // Footer
        ])
        .split(area);

    // Header with log file path and entry count
    let header_text = format!(
        "Log File: {} ({} entries{})",
        log_file_info.path.display(),
        entry_count,
        if auto_scroll { ", auto-scroll ON" } else { "" }
    );
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan).bold())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    // Calculate visible area height (minus borders)
    let visible_height = chunks[1].height.saturating_sub(2) as usize;

    // If auto-scroll is enabled, calculate scroll to show latest entries
    let effective_scroll = if auto_scroll && entry_count > visible_height {
        entry_count.saturating_sub(visible_height)
    } else {
        scroll_offset.min(entry_count.saturating_sub(visible_height.max(1)))
    };

    // Log entries list
    if entries.is_empty() {
        let empty = Paragraph::new("No log entries yet.")
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::ALL).title("Logs"));
        frame.render_widget(empty, chunks[1]);
    } else {
        // Get visible entries based on scroll position
        let visible_entries: Vec<_> = entries
            .iter()
            .skip(effective_scroll)
            .take(visible_height)
            .collect();

        let items: Vec<ListItem> = visible_entries
            .iter()
            .map(|entry| {
                // Format timestamp
                let time = entry.timestamp.format("%H:%M:%S%.3f");

                // Get level color
                let level_color = match entry.level {
                    LogLevel::Trace => Color::DarkGray,
                    LogLevel::Debug => Color::Gray,
                    LogLevel::Info => Color::Blue,
                    LogLevel::Warn => Color::Yellow,
                    LogLevel::Error => Color::Red,
                };

                // Build the line with colored level
                let content = Line::from(vec![
                    Span::styled(format!("{} ", time), Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{:5} ", entry.level.as_str()),
                        Style::default().fg(level_color).bold(),
                    ),
                    Span::styled(
                        format!("{}: ", entry.target),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(&entry.message),
                ]);

                ListItem::new(content)
            })
            .collect();

        let title = format!(
            "Logs [{}-{} of {}]",
            effective_scroll + 1,
            (effective_scroll + visible_entries.len()).min(entry_count),
            entry_count
        );

        let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
        frame.render_widget(list, chunks[1]);

        // Scrollbar
        if entry_count > visible_height {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"));

            let mut scrollbar_state = ScrollbarState::new(entry_count)
                .position(effective_scroll)
                .viewport_content_length(visible_height);

            // Render scrollbar in the content area (inside the right border)
            let scrollbar_area = Rect {
                x: chunks[1].x + chunks[1].width - 1,
                y: chunks[1].y + 1,
                width: 1,
                height: chunks[1].height.saturating_sub(2),
            };
            frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
        }
    }

    // Footer with navigation help
    let footer_text = "↑/k ↓/j: scroll | g: top | G: bottom (auto) | PgUp/PgDn: page | Esc/q: back";
    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
}
