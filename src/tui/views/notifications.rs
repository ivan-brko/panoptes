//! Notification rendering for TUI
//!
//! Renders notifications in the top-right corner of the screen.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::tui::notifications::{Notification, NotificationType};

/// Width of notification popups
const NOTIFICATION_WIDTH: u16 = 45;
/// Height of each notification
const NOTIFICATION_HEIGHT: u16 = 4;
/// Margin from screen edge
const NOTIFICATION_MARGIN: u16 = 2;

/// Render notifications in the top-right corner
pub fn render_notifications(frame: &mut Frame, area: Rect, notifications: &[&Notification]) {
    if notifications.is_empty() {
        return;
    }

    for (i, notification) in notifications.iter().enumerate() {
        // Calculate position (stacked from top)
        let y_offset = NOTIFICATION_MARGIN + (i as u16 * (NOTIFICATION_HEIGHT + 1));
        let x = area
            .width
            .saturating_sub(NOTIFICATION_WIDTH + NOTIFICATION_MARGIN);

        if y_offset + NOTIFICATION_HEIGHT > area.height {
            break; // Don't render if it would go off screen
        }

        let notif_area = Rect {
            x,
            y: y_offset,
            width: NOTIFICATION_WIDTH,
            height: NOTIFICATION_HEIGHT,
        };

        // Determine style based on notification type
        let (border_color, title_style) = match &notification.notification_type {
            NotificationType::TimerComplete { .. } => {
                (Color::Green, Style::default().fg(Color::Green).bold())
            }
            NotificationType::Info { .. } => (Color::Cyan, Style::default().fg(Color::Cyan).bold()),
            NotificationType::Warning { .. } => {
                (Color::Yellow, Style::default().fg(Color::Yellow).bold())
            }
        };

        // Build the title with optional countdown
        let title = if let Some(remaining) = notification.remaining_time() {
            format!(
                " {} ({}s) ",
                notification.notification_type.title(),
                remaining.as_secs()
            )
        } else {
            format!(" {} ", notification.notification_type.title())
        };

        // Clear the area first
        frame.render_widget(Clear, notif_area);

        // Render the notification
        let message = notification.notification_type.message();
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(Span::styled(title, title_style));

        let paragraph = Paragraph::new(message)
            .style(Style::default().fg(Color::White))
            .block(block)
            .wrap(ratatui::widgets::Wrap { trim: true });

        frame.render_widget(paragraph, notif_area);
    }
}

/// Render a small notification hint if there are notifications
/// (shown when notifications are minimized or as a badge)
pub fn render_notification_badge(frame: &mut Frame, area: Rect, count: usize) {
    if count == 0 {
        return;
    }

    let badge_text = format!(" {} ", count);
    let badge_width = badge_text.len() as u16 + 2;

    let badge_area = Rect {
        x: area.width.saturating_sub(badge_width + 1),
        y: 0,
        width: badge_width,
        height: 1,
    };

    let badge =
        Paragraph::new(badge_text).style(Style::default().fg(Color::White).bg(Color::Red).bold());

    frame.render_widget(badge, badge_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert!(NOTIFICATION_WIDTH > 0);
        assert!(NOTIFICATION_HEIGHT > 0);
        // NOTIFICATION_MARGIN is u16, always >= 0
        assert!(NOTIFICATION_MARGIN <= 10); // sanity check
    }
}
