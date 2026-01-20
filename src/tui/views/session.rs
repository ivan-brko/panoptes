//! Session view
//!
//! Fullscreen view for interacting with a single Claude Code session.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{AppState, InputMode};
use crate::config::Config;
use crate::project::ProjectStore;
use crate::session::{SessionManager, SessionState};
use crate::tui::theme::theme;
use crate::tui::views::format_attention_hint;

/// Render the session view
pub fn render_session_view(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    sessions: &SessionManager,
    project_store: &ProjectStore,
    config: &Config,
) {
    let t = theme();
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
        let exit_info = if session.info.state == SessionState::Exited {
            session
                .info
                .exit_reason
                .as_ref()
                .map(|r| format!(" ({})", r))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let project_name = project_store
            .get_project(session.info.project_id)
            .map(|p| p.name.as_str())
            .unwrap_or("?");
        format!(
            "{} / {} - {}{}{}",
            project_name,
            session.info.name,
            session.info.state.display_name(),
            exit_info,
            mode_indicator
        )
    } else {
        "No session selected".to_string()
    };

    let header_color = session
        .map(|s| s.info.state.color())
        .unwrap_or(t.text_muted);

    let header = Paragraph::new(header_text)
        .style(Style::default().fg(header_color).bold())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    // Output area - active (Session mode) uses focused border, otherwise muted
    let frame_color = if state.input_mode == InputMode::Session {
        t.active
    } else {
        t.text_muted
    };

    if let Some(session) = session {
        let output_height = chunks[1].height.saturating_sub(2) as usize; // Account for borders
        let styled_lines = session.visible_styled_lines(output_height);
        // Dereference Rc to get the Vec, then clone for Paragraph (cheap since Lines contain Rc'd spans)
        let output = Paragraph::new((*styled_lines).clone()).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(frame_color))
                .title("Output"),
        );
        frame.render_widget(output, chunks[1]);
    } else {
        let empty = Paragraph::new("Session not found")
            .style(Style::default().fg(t.error_bg))
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
        InputMode::Session => "Esc: deactivate | ⌥Esc: send Esc | Keys sent to session".to_string(),
        _ => {
            let base = "Enter: activate | Tab: next | 1-9: jump | Esc/q: back";
            if let Some(hint) = format_attention_hint(sessions, config) {
                format!("{} | {}", hint, base)
            } else {
                base.to_string()
            }
        }
    };
    let footer = Paragraph::new(help_text)
        .style(t.muted_style())
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
}
