//! Log viewer view
//!
//! Displays log entries with scrolling and colored levels.

use std::sync::Arc;

use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use crate::logging::{LogBuffer, LogFileInfo};
use crate::tui::header::Header;
use crate::tui::header_notifications::HeaderNotificationManager;
use crate::tui::layout::ScreenLayout;
use crate::tui::theme::theme;
use crate::tui::views::{render_footer, Breadcrumb};

/// Render the log viewer showing all log entries
#[allow(clippy::too_many_arguments)]
pub fn render_log_viewer(
    frame: &mut Frame,
    area: Rect,
    log_buffer: &Arc<LogBuffer>,
    log_file_info: &LogFileInfo,
    scroll_offset: usize,
    auto_scroll: bool,
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

    let t = theme();

    // Log entries list
    if entries.is_empty() {
        let empty = Paragraph::new("No log entries yet.")
            .style(t.muted_style())
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

                // Build the line with colored level
                let content = Line::from(vec![
                    Span::styled(format!("{} ", time), t.muted_style()),
                    Span::styled(
                        format!("{:5} ", entry.level.as_str()),
                        Style::default().fg(t.log_level_color(entry.level)).bold(),
                    ),
                    Span::styled(format!("{}: ", entry.target), Style::default().fg(t.accent)),
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
    render_footer(
        frame,
        areas.footer(),
        "↑/↓: scroll | g: top | G: bottom (auto) | PgUp/PgDn: page | ?: help | Esc: back",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::{LogEntry, LogLevel};
    use crate::tui::views::test_util::{contains_line, render_to_lines};
    use std::path::PathBuf;

    fn render_logs(buffer: &Arc<LogBuffer>) -> Vec<String> {
        let info = LogFileInfo {
            path: PathBuf::from("/tmp/panoptes.log"),
        };
        let header_notifications = HeaderNotificationManager::default();

        render_to_lines(100, 24, |frame| {
            render_log_viewer(
                frame,
                frame.size(),
                buffer,
                &info,
                0,
                true,
                &header_notifications,
                0,
            )
        })
    }

    #[test]
    fn test_empty_log_renders_placeholder() {
        let buffer = Arc::new(LogBuffer::new(10));

        let lines = render_logs(&buffer);

        assert!(contains_line(&lines, "No log entries yet."), "{:?}", lines);
    }

    #[test]
    fn test_entries_show_level_target_and_message() {
        let buffer = Arc::new(LogBuffer::new(10));
        buffer.push(LogEntry::new(
            LogLevel::Warn,
            "panoptes::hooks",
            "port busy",
        ));

        let lines = render_logs(&buffer);

        assert!(contains_line(&lines, "Logs [1-1 of 1]"), "{:?}", lines);
        assert!(
            contains_line(&lines, "WARN  panoptes::hooks: port busy"),
            "{:?}",
            lines
        );
    }
}
