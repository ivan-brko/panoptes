//! Log viewer view
//!
//! Displays log entries with scrolling and colored levels.

use std::sync::Arc;

use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use crate::focus_timing::FocusTimer;
use crate::logging::{LogBuffer, LogFileInfo, LogLevel};
use crate::tui::header::Header;
use crate::tui::header_notifications::HeaderNotificationManager;
use crate::tui::layout::ScreenLayout;
use crate::tui::views::{format_focus_timer_hint, Breadcrumb};

/// Render the log viewer showing all log entries
#[allow(clippy::too_many_arguments)]
pub fn render_log_viewer(
    frame: &mut Frame,
    area: Rect,
    log_buffer: &Arc<LogBuffer>,
    log_file_info: &LogFileInfo,
    scroll_offset: usize,
    auto_scroll: bool,
    focus_timer: Option<&FocusTimer>,
    header_notifications: &HeaderNotificationManager,
    attention_count: usize,
) {
    let entries = log_buffer.all_entries();
    let entry_count = entries.len();

    // Build header
    let auto_scroll_status = if auto_scroll { " [auto-scroll]" } else { "" };
    let breadcrumb = Breadcrumb::new().push("Logs");
    let suffix = format!(
        "- {} ({} entries{})",
        log_file_info.path.display(),
        entry_count,
        auto_scroll_status
    );

    let header = Header::new(breadcrumb)
        .with_suffix(suffix)
        .with_timer(focus_timer)
        .with_notifications(Some(header_notifications))
        .with_attention_count(attention_count);

    // Create layout with header and footer
    let areas = ScreenLayout::new(area).with_header(header).render(frame);

    // Calculate visible area height (minus borders)
    let visible_height = areas.content.height.saturating_sub(2) as usize;

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
        frame.render_widget(empty, areas.content);
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
        frame.render_widget(list, areas.content);

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
                x: areas.content.x + areas.content.width - 1,
                y: areas.content.y + 1,
                width: 1,
                height: areas.content.height.saturating_sub(2),
            };
            frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
        }
    }

    // Footer with navigation help
    let timer_hint = format_focus_timer_hint(focus_timer.map(|t| t.is_running()).unwrap_or(false));
    let footer_text = format!(
        "{} | ↑/k ↓/j: scroll | g: top | G: bottom (auto) | PgUp/PgDn: page | Esc/q: back",
        timer_hint
    );
    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, areas.footer());
}
