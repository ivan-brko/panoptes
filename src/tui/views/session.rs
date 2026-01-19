//! Session view
//!
//! Fullscreen view for interacting with a single Claude Code session.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{AppState, InputMode};
use crate::session::SessionManager;

/// Render the session view
pub fn render_session_view(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    sessions: &SessionManager,
) {
    let session = state.active_session.and_then(|id| sessions.get(id));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Output
            Constraint::Length(3), // Footer
        ])
        .split(area);

    // Header with session info
    let header_text = if let Some(session) = session {
        let mode_indicator = match state.input_mode {
            InputMode::Session => " [SESSION MODE]",
            _ => " [NORMAL]",
        };
        format!(
            "{} - {}{}",
            session.info.name,
            session.info.state.display_name(),
            mode_indicator
        )
    } else {
        "No session selected".to_string()
    };

    let header_color = session
        .map(|s| s.info.state.color())
        .unwrap_or(Color::DarkGray);

    let header = Paragraph::new(header_text)
        .style(Style::default().fg(header_color).bold())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    // Output area - green frame when active (Session mode), gray when inactive
    let frame_color = if state.input_mode == InputMode::Session {
        Color::Green
    } else {
        Color::DarkGray
    };

    if let Some(session) = session {
        let output_height = chunks[1].height.saturating_sub(2) as usize; // Account for borders
        let styled_lines = session.visible_styled_lines(output_height);
        let output = Paragraph::new(styled_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(frame_color))
                .title("Output"),
        );
        frame.render_widget(output, chunks[1]);
    } else {
        let empty = Paragraph::new("Session not found")
            .style(Style::default().fg(Color::Red))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(frame_color))
                    .title("Output"),
            );
        frame.render_widget(empty, chunks[1]);
    }

    // Footer with help
    let help_text = match state.input_mode {
        InputMode::Session => "Esc: deactivate | Keys sent to session",
        _ => "Enter: activate | Tab: next | 1-9: jump | Esc/q: back",
    };
    let footer = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
}
