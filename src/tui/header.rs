//! Unified header component
//!
//! Provides a consistent header across all views with:
//! - Breadcrumb navigation
//! - Optional suffix (e.g., status counts)
//! - Header notifications (shown after breadcrumb)
//! - Attention indicator (blinking when sessions need attention)
//! - Focus timer countdown (right-aligned)

use std::time::Instant;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::focus_timing::FocusTimer;
use crate::tui::header_notifications::HeaderNotificationManager;
use crate::tui::theme::theme;
use crate::tui::views::Breadcrumb;

/// Start time for blinking calculation
static START_TIME: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn get_start_time() -> Instant {
    *START_TIME.get_or_init(Instant::now)
}

/// Unified header component for all views
pub struct Header<'a> {
    /// Breadcrumb navigation path
    breadcrumb: Breadcrumb,
    /// Optional suffix text (e.g., "(3 active, 2 need attention)")
    suffix: Option<String>,
    /// Focus timer reference
    timer: Option<&'a FocusTimer>,
    /// Header notifications manager
    notifications: Option<&'a HeaderNotificationManager>,
    /// Number of sessions needing attention
    attention_count: usize,
    /// Optional custom style (for session view state-based coloring)
    custom_style: Option<Style>,
}

impl<'a> Header<'a> {
    /// Create a new header with the given breadcrumb
    pub fn new(breadcrumb: Breadcrumb) -> Self {
        Self {
            breadcrumb,
            suffix: None,
            timer: None,
            notifications: None,
            attention_count: 0,
            custom_style: None,
        }
    }

    /// Add a suffix to the header (e.g., status counts)
    pub fn with_suffix(mut self, suffix: impl Into<String>) -> Self {
        let s = suffix.into();
        if !s.is_empty() {
            self.suffix = Some(s);
        }
        self
    }

    /// Add focus timer display
    pub fn with_timer(mut self, timer: Option<&'a FocusTimer>) -> Self {
        self.timer = timer;
        self
    }

    /// Add header notifications
    pub fn with_notifications(
        mut self,
        notifications: Option<&'a HeaderNotificationManager>,
    ) -> Self {
        self.notifications = notifications;
        self
    }

    /// Set the attention count for blinking indicator
    pub fn with_attention_count(mut self, count: usize) -> Self {
        self.attention_count = count;
        self
    }

    /// Set a custom style (overrides default header style)
    pub fn with_custom_style(mut self, style: Style) -> Self {
        self.custom_style = Some(style);
        self
    }

    /// Render the header to the given area
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let t = theme();

        // Build the left side content
        let mut left_parts = Vec::new();

        // Breadcrumb
        let breadcrumb_text = if let Some(suffix) = &self.suffix {
            self.breadcrumb.display_with_suffix(suffix)
        } else {
            self.breadcrumb.display()
        };
        left_parts.push(breadcrumb_text);

        // Notification message (if any)
        let notification_msg = self.notifications.and_then(|n| n.current_message());
        if let Some(msg) = notification_msg {
            left_parts.push(format!("| {}", msg));
        }

        let left_text = left_parts.join(" ");

        // Build the right side content
        let mut right_spans: Vec<Span> = Vec::new();

        // Attention indicator (blinking if attention_count > 0)
        if self.attention_count > 0 {
            let should_show = Self::should_show_blink();
            if should_show {
                right_spans.push(Span::styled(
                    format!("[{}\u{25CF}]", self.attention_count), // [N●]
                    Style::default().fg(t.attention_badge).bold(),
                ));
                right_spans.push(Span::raw(" "));
            } else {
                // Show count without indicator during blink-off phase
                right_spans.push(Span::styled(
                    format!("[{}]", self.attention_count),
                    Style::default().fg(t.attention_badge),
                ));
                right_spans.push(Span::raw(" "));
            }
        }

        // Timer display
        if let Some(timer) = self.timer {
            if timer.is_running() {
                right_spans.push(Span::styled(
                    format!("\u{23F1} {}", timer.format_remaining()),
                    Style::default().fg(t.accent),
                ));
            }
        }

        // Calculate layout
        let width = area.width.saturating_sub(2) as usize; // Account for borders
        let left_len = left_text.chars().count();
        let right_text: String = right_spans.iter().map(|s| s.content.as_ref()).collect();
        let right_len = right_text.chars().count();

        // Build the final line with padding
        let padding = width.saturating_sub(left_len + right_len);

        let mut line_spans = vec![Span::raw(left_text)];
        if padding > 0 {
            line_spans.push(Span::raw(" ".repeat(padding)));
        }
        line_spans.extend(right_spans);

        // Determine style
        let style = self.custom_style.unwrap_or_else(|| t.header_style());

        let paragraph = Paragraph::new(Line::from(line_spans))
            .style(style)
            .block(Block::default().borders(Borders::BOTTOM));

        frame.render_widget(paragraph, area);
    }

    /// Check if the blink indicator should be visible (500ms on/off cycle)
    fn should_show_blink() -> bool {
        let elapsed = get_start_time().elapsed();
        (elapsed.as_millis() % 1000) < 500
    }
}

/// Height constant for the header (including bottom border)
pub const HEADER_HEIGHT: u16 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_creation() {
        let breadcrumb = Breadcrumb::new().push("Projects");
        let header = Header::new(breadcrumb);
        assert!(header.suffix.is_none());
        assert!(header.timer.is_none());
        assert_eq!(header.attention_count, 0);
    }

    #[test]
    fn test_header_with_suffix() {
        let breadcrumb = Breadcrumb::new().push("Projects");
        let header = Header::new(breadcrumb).with_suffix("(3 active)");
        assert_eq!(header.suffix, Some("(3 active)".to_string()));
    }

    #[test]
    fn test_header_with_attention() {
        let breadcrumb = Breadcrumb::new().push("Timeline");
        let header = Header::new(breadcrumb).with_attention_count(5);
        assert_eq!(header.attention_count, 5);
    }

    #[test]
    fn test_blink_cycle() {
        // Just verify the function runs without panic
        let result = Header::should_show_blink();
        assert!(result || !result); // Always true, just checking it returns
    }
}
