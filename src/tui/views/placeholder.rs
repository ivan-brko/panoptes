//! Placeholder view for unimplemented views

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Render a placeholder view for views not yet implemented
pub fn render_placeholder(frame: &mut Frame, area: Rect, title: &str) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);

    let header = Paragraph::new(title)
        .style(Style::default().fg(Color::Cyan).bold())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    let content = Paragraph::new(
        "This view will be implemented in a future ticket.\n\nPress Esc to go back.",
    )
    .style(Style::default().fg(Color::DarkGray))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(content, chunks[1]);

    let footer = Paragraph::new("Esc/q: back")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
}
