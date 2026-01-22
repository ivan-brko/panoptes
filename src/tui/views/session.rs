//! Session view
//!
//! Fullscreen view for interacting with a single Claude Code session.
//! Uses FrameLayout for pre-calculated areas and separate border/content rendering.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{AppState, InputMode};
use crate::config::Config;
use crate::project::ProjectStore;
use crate::session::{Session, SessionManager, SessionState};
use crate::tui::frame::{render_frame_border, render_pty_content, FrameConfig, FrameLayout};
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

    // Pre-calculate layout using FrameLayout
    let frame_config = FrameConfig {
        header_height: 3,
        footer_height: 3,
        title: Some("Output".to_string()),
    };
    let layout = FrameLayout::calculate(area, &frame_config);

    // === HEADER ===
    let header_text = build_header_text(session, state, project_store);
    let header_color = session
        .map(|s| s.info.state.color())
        .unwrap_or(t.text_muted);

    let header = Paragraph::new(header_text)
        .style(Style::default().fg(header_color).bold())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, layout.header);

    // === FRAME BORDER ===
    let frame_color = if state.input_mode == InputMode::Session {
        t.active
    } else {
        t.text_muted
    };

    // Build title with scroll indicator
    let title = if let Some(session) = session {
        let scroll_offset = session.vterm.scrollback_offset();
        if scroll_offset > 0 {
            format!("Output [{}{}]", '\u{2191}', scroll_offset)
        } else {
            "Output".to_string()
        }
    } else {
        "Output".to_string()
    };

    render_frame_border(frame, layout.frame, frame_color, Some(&title));

    // === CONTENT ===
    if let Some(session) = session {
        let styled_lines = session.visible_styled_lines(layout.content.height as usize);

        // Get cursor info
        let cursor_pos = session.vterm.cursor_position();
        let cursor_visible = state.input_mode == InputMode::Session
            && session.vterm.cursor_visible()
            && session.vterm.scrollback_offset() == 0;

        render_pty_content(
            frame,
            layout.content,
            &styled_lines,
            Some(cursor_pos),
            cursor_visible,
        );
    } else {
        let empty = Paragraph::new("Session not found").style(Style::default().fg(t.error_bg));
        frame.render_widget(empty, layout.content);
    }

    // === FOOTER ===
    let is_scrolled = session
        .map(|s| s.vterm.scrollback_offset() > 0)
        .unwrap_or(false);
    let help_text = build_footer_text(state, is_scrolled, sessions, config);

    let footer = Paragraph::new(help_text)
        .style(t.muted_style())
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, layout.footer);
}

fn build_header_text(
    session: Option<&Session>,
    state: &AppState,
    project_store: &ProjectStore,
) -> String {
    if let Some(session) = session {
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
        let branch_name = project_store
            .get_branch(session.info.branch_id)
            .map(|b| b.name.as_str())
            .unwrap_or("?");
        format!(
            "{} / {} / {} - {}{}{}",
            project_name,
            branch_name,
            session.info.name,
            session.info.state.display_name(),
            exit_info,
            mode_indicator
        )
    } else {
        "No session selected".to_string()
    }
}

fn build_footer_text(
    state: &AppState,
    is_scrolled: bool,
    sessions: &SessionManager,
    config: &Config,
) -> String {
    match state.input_mode {
        InputMode::Session => {
            if is_scrolled {
                "Esc: deactivate | PgUp/PgDn: scroll | Ctrl+End: live view | Deactivate to copy text"
                    .to_string()
            } else {
                "Esc: deactivate | \u{2325}Esc: send Esc | PgUp: scroll history | Deactivate to copy text"
                    .to_string()
            }
        }
        _ => {
            let scroll_hint = if is_scrolled { "End: live view | " } else { "" };
            let base = format!(
                "{}Enter: activate | Tab: next | PgUp/Dn: scroll | Activate for mouse scroll",
                scroll_hint
            );
            if let Some(hint) = format_attention_hint(sessions, config) {
                format!("{} | {}", hint, base)
            } else {
                base
            }
        }
    }
}
