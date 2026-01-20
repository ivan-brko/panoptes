//! Unified confirmation dialog component
//!
//! Provides a consistent look and feel for all confirmation dialogs in the application.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::tui::theme::theme;

/// Configuration for a confirmation dialog
pub struct ConfirmDialogConfig<'a> {
    /// Dialog title (e.g., "Confirm Delete")
    pub title: &'a str,
    /// Label for the item type (e.g., "project" or "session")
    pub item_label: &'a str,
    /// Name of the item being acted on
    pub item_name: &'a str,
    /// Warning lines (displayed in yellow with ⚠ prefix)
    pub warnings: Vec<String>,
    /// Note lines (displayed in muted gray)
    pub notes: Vec<String>,
}

/// Render a unified confirmation dialog
///
/// Unified style:
/// - Border: Yellow (warning color, indicates destructive action)
/// - Item name: Cyan + Bold
/// - Warnings: Yellow + Bold with ⚠ prefix
/// - Notes: Muted gray
/// - Prompt: "Press y to confirm, n or Esc to cancel" (y=green, n/Esc=red)
/// - Alignment: Center
pub fn render_confirm_dialog(frame: &mut Frame, area: Rect, config: ConfirmDialogConfig) {
    let t = theme();

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                format!("Delete {}: ", config.item_label),
                Style::default().fg(t.text),
            ),
            Span::styled(
                config.item_name,
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("?", Style::default().fg(t.text)),
        ]),
        Line::from(""),
    ];

    // Add warnings (yellow with ⚠ prefix)
    for warning in &config.warnings {
        lines.push(Line::from(vec![Span::styled(
            format!("⚠  {}", warning),
            Style::default()
                .fg(t.border_warning)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));
    }

    // Add notes (muted gray)
    for note in &config.notes {
        lines.push(Line::from(vec![Span::styled(
            note.as_str(),
            Style::default().fg(t.text_muted),
        )]));
        lines.push(Line::from(""));
    }

    // Add some spacing before the prompt
    if !config.warnings.is_empty() || !config.notes.is_empty() {
        // Already have spacing from above
    } else {
        lines.push(Line::from(""));
    }

    // Confirmation prompt with styled keys
    lines.push(Line::from(vec![
        Span::styled("Press ", Style::default().fg(t.text)),
        Span::styled(
            "y",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" to confirm, ", Style::default().fg(t.text)),
        Span::styled(
            "n",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" or ", Style::default().fg(t.text)),
        Span::styled(
            "Esc",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" to cancel", Style::default().fg(t.text)),
    ]));

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border_warning))
            .title(config.title),
    );

    frame.render_widget(paragraph, area);
}
