//! Session view
//!
//! Fullscreen view for interacting with a single Claude Code session.
//! Uses FrameLayout for pre-calculated areas and separate border/content rendering.

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::app::{AppState, InputMode};
use crate::config::Config;
use crate::project::ProjectStore;
use crate::session::{Session, SessionManager, SessionState, SessionType};
use crate::tui::frame::{render_frame_border, render_pty_content, FrameConfig, FrameLayout};
use crate::tui::header::Header;
use crate::tui::header_notifications::HeaderNotificationManager;
use crate::tui::theme::theme;
use crate::tui::views::Breadcrumb;
use crate::tui::views::{footer_with_attention, format_custom_shortcuts_hint, render_footer};

/// Render the session view
pub fn render_session_view(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    sessions: &SessionManager,
    project_store: &ProjectStore,
    config: &Config,
    header_notifications: &HeaderNotificationManager,
) {
    let t = theme();
    let session = state.active_session.and_then(|id| sessions.get(id));
    let attention_count = sessions.total_attention_count();

    // Pre-calculate layout using FrameLayout
    let frame_config = FrameConfig {
        header_height: 3,
        footer_height: 3,
        title: Some("Output".to_string()),
    };
    let layout = FrameLayout::calculate(area, &frame_config);

    // === HEADER ===
    // Build breadcrumb and suffix
    let (breadcrumb, suffix) = build_header_breadcrumb(session, state, project_store);

    // Session header has custom coloring based on session state
    let header_color = session
        .map(|s| s.info.state.color())
        .unwrap_or(t.text_muted);
    let custom_style = Style::default().fg(header_color).bold();

    let header = Header::new(breadcrumb)
        .with_suffix(suffix)
        .with_notifications(Some(header_notifications))
        .with_attention_count(attention_count)
        .with_custom_style(custom_style);

    header.render(frame, layout.header);

    // === FRAME BORDER ===
    let frame_color = if state.input_mode == InputMode::Session {
        t.active
    } else {
        t.text_muted
    };

    // Build title with scroll indicator
    let title = if let Some(session) = session {
        let scroll_offset = if session.info.session_type == SessionType::OpenAICodex {
            state.session_scroll_offset
        } else {
            session.vterm.scrollback_offset()
        };
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
        let use_fallback_history = session.info.session_type == SessionType::OpenAICodex
            && state.session_scroll_offset > 0
            && session.vterm.scrollback_offset() == 0;

        if use_fallback_history {
            let lines = session
                .fallback_visible_lines(layout.content.height as usize)
                .into_iter()
                .map(Line::raw)
                .collect::<Vec<_>>();
            let content = Paragraph::new(lines);
            frame.render_widget(content, layout.content);
        } else {
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
        }
    } else {
        let empty = Paragraph::new("Session not found").style(Style::default().fg(t.error_bg));
        frame.render_widget(empty, layout.content);
    }

    // === FOOTER ===
    let is_scrolled = session
        .map(|s| {
            if s.info.session_type == SessionType::OpenAICodex {
                state.session_scroll_offset > 0
            } else {
                s.vterm.scrollback_offset() > 0
            }
        })
        .unwrap_or(false);
    let suspended = session.is_some_and(|s| s.info.state == SessionState::Suspended);
    let help_text = build_footer_text(state, is_scrolled, suspended, sessions, config);
    render_footer(frame, layout.footer, &help_text);
}

/// Build breadcrumb and suffix for the session header
fn build_header_breadcrumb(
    session: Option<&Session>,
    state: &AppState,
    project_store: &ProjectStore,
) -> (Breadcrumb, String) {
    if let Some(session) = session {
        let mode_indicator = match state.input_mode {
            InputMode::Session => "[SESSION]",
            _ => "[NORMAL]",
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
        // Show Claude config name if present
        let config_display = session
            .info
            .claude_config_name
            .as_ref()
            .map(|n| format!(" [{}]", n))
            .unwrap_or_default();
        let project_name = project_store
            .get_project(session.info.project_id)
            .map(|p| p.name.as_str())
            .unwrap_or("?");
        let branch_name = project_store
            .get_branch(session.info.branch_id)
            .map(|b| b.name.as_str())
            .unwrap_or("?");
        let breadcrumb = Breadcrumb::new()
            .push(project_name)
            .push(branch_name)
            .push(&session.info.name);
        // Token and rate-limit figures read from the agent's own transcript.
        // Absent until the tailer has something to report, and absent for
        // shells, which have no conversation to measure.
        let usage_display = session
            .info
            .usage
            .summary()
            .map(|summary| format!(" · {}", summary))
            .unwrap_or_default();
        let subagent_display = match session.info.subagents {
            0 => String::new(),
            1 => " · 1 subagent".to_string(),
            n => format!(" · {} subagents", n),
        };
        let suffix = format!(
            "{} - {}{}{}{}{} {}",
            session.info.session_type.short_tag(),
            session.info.state.display_name(),
            exit_info,
            config_display,
            subagent_display,
            usage_display,
            mode_indicator
        );
        (breadcrumb, suffix)
    } else {
        (
            Breadcrumb::new().push("?").push("?").push("?"),
            "- No session".to_string(),
        )
    }
}

fn build_footer_text(
    state: &AppState,
    is_scrolled: bool,
    suspended: bool,
    sessions: &SessionManager,
    config: &Config,
) -> String {
    // Say what a suspended session is before saying what to do with it: the
    // scrollback still reads as a live session, so without this the missing
    // process looks like a hang rather than a deliberate saving.
    if suspended {
        return match state.input_mode {
            InputMode::Session => {
                "Suspended to save memory | Type to wake | PgUp: scroll history".to_string()
            }
            _ => "Suspended to save memory | Enter: session mode, then type to wake | \u{2191}\u{2193}/PgUp/Dn: scroll"
                .to_string(),
        };
    }

    match state.input_mode {
        InputMode::Session => {
            if is_scrolled {
                "Esc: exit session mode | PgUp/PgDn: scroll | Ctrl+End: live view | Exit to copy text"
                    .to_string()
            } else {
                "Esc: exit session mode | \u{21E7}Esc: send Esc | PgUp: scroll | Exit to copy text"
                    .to_string()
            }
        }
        _ => {
            let scroll_hint = if is_scrolled { "End: live view | " } else { "" };
            // Build custom shortcuts hint
            let shortcuts_hint = format_custom_shortcuts_hint(&config.custom_shortcuts);

            let base = format!(
                "{}{}Enter: session mode | 1-9: switch | \u{2191}\u{2193}/PgUp/Dn: scroll | q: quit | ?: help | Esc: back",
                scroll_hint, shortcuts_hint
            );
            footer_with_attention(base, sessions)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::store::SessionStore;
    use crate::tui::views::test_util::{contains_line, render_to_lines};

    #[test]
    fn test_missing_session_renders_placeholder() {
        let state = AppState::default();
        let config = Config::default();
        let sessions = SessionManager::with_store(config.clone(), SessionStore::new());
        let store = ProjectStore::new();
        let header_notifications = HeaderNotificationManager::default();

        let lines = render_to_lines(80, 24, |frame| {
            render_session_view(
                frame,
                frame.size(),
                &state,
                &sessions,
                &store,
                &config,
                &header_notifications,
            )
        });

        assert!(contains_line(&lines, "Session not found"), "{:?}", lines);
        assert!(contains_line(&lines, "Panoptes > ? > ? > ?"), "{:?}", lines);
        assert!(contains_line(&lines, "Enter: session mode"), "{:?}", lines);
    }
}
