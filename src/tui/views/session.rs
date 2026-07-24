//! Session view
//!
//! Fullscreen view for interacting with a single Claude Code session.
//! Uses FrameLayout for pre-calculated areas and separate border/content rendering.

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::app::{AppState, InputMode};
use crate::config::Config;
use crate::project::ProjectStore;
use crate::session::{Session, SessionInfo, SessionManager, SessionState, SessionType};
use crate::tui::frame::{render_frame_border, render_pty_content, FrameConfig, FrameLayout};
use crate::tui::header::{Header, LogoKind};
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

    // === HEADER ===
    // Built before the layout, because how many rows it needs depends on how
    // much of the wordmark this terminal can afford
    let (breadcrumb, suffix) = build_header_breadcrumb(session, state, project_store);

    // Session header has custom coloring based on session state
    let header_color = session
        .map(|s| s.info.state.color())
        .unwrap_or(t.text_muted);
    let custom_style = Style::default().fg(header_color).bold();

    // The wordmark only, without the tagline and version the pane screen
    // carries: every row here is agent output the user is reading
    let header = Header::new(breadcrumb)
        .with_logo(LogoKind::Wordmark)
        .with_suffix(suffix)
        .with_notifications(Some(header_notifications))
        .with_attention_count(attention_count)
        .with_custom_style(custom_style);

    // Pre-calculate layout using FrameLayout
    let frame_config = FrameConfig {
        header_height: header.height(area),
        footer_height: 3,
        title: Some("Output".to_string()),
    };
    let layout = FrameLayout::calculate(area, &frame_config);

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

/// What the session header says after the breadcrumb
///
/// Everything the terminal below cannot say for itself, and nothing it can.
fn header_suffix(info: &SessionInfo, mode: InputMode) -> String {
    let mode_indicator = match mode {
        InputMode::Session => "[SESSION]",
        _ => "[NORMAL]",
    };
    // The terminal below is the status. A session that is Thinking or Executing
    // is visibly doing so, and naming it again here only takes room from what
    // the screen cannot say. The two states the scrollback cannot report are
    // the ones that survive: a process that died, and one Panoptes killed to
    // reclaim memory - both leave the output frozen mid-page, looking exactly
    // like a session sitting at its prompt.
    let state_display = match info.state {
        SessionState::Exited => {
            let reason = info
                .exit_reason
                .as_ref()
                .map(|r| format!(" ({})", r))
                .unwrap_or_default();
            format!(" - Exited{}", reason)
        }
        SessionState::Suspended => " - Suspended".to_string(),
        _ => String::new(),
    };
    // The agent and the account it runs as are one fact - "Claude Code, signed
    // in as dot-lambda" - so they share a bracket rather than sitting either
    // side of the state text that used to separate them
    let agent_display = match info.account_name() {
        Some(account) => format!("[{} \u{00b7} {}]", info.session_type.code(), account),
        None => info.session_type.short_tag().to_string(),
    };
    // Token and rate-limit figures read from the agent's own transcript.
    // Absent until the tailer has something to report, and absent for shells,
    // which have no conversation to measure.
    let usage_display = info
        .usage
        .summary()
        .map(|summary| format!(" \u{00b7} {}", summary))
        .unwrap_or_default();
    let subagent_display = match info.subagents {
        0 => String::new(),
        1 => " \u{00b7} 1 subagent".to_string(),
        n => format!(" \u{00b7} {} subagents", n),
    };

    format!(
        "{}{}{}{} {}",
        agent_display, state_display, subagent_display, usage_display, mode_indicator
    )
}

/// Build breadcrumb and suffix for the session header
fn build_header_breadcrumb(
    session: Option<&Session>,
    state: &AppState,
    project_store: &ProjectStore,
) -> (Breadcrumb, String) {
    let Some(session) = session else {
        return (
            Breadcrumb::new().push("?").push("?").push("?"),
            "- No session".to_string(),
        );
    };

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

    (breadcrumb, header_suffix(&session.info, state.input_mode))
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
        assert!(contains_line(&lines, "? > ? > ?"), "{:?}", lines);
        assert!(contains_line(&lines, "Enter: session mode"), "{:?}", lines);
    }

    fn info(session_type: SessionType) -> SessionInfo {
        let mut info = SessionInfo::new(
            "sess".to_string(),
            std::path::PathBuf::from("/tmp"),
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
        );
        info.session_type = session_type;
        info
    }

    /// The user is looking at the terminal that is doing the work, so the
    /// header naming the same thing costs a row and says nothing
    #[test]
    fn test_a_working_session_does_not_narrate_its_state() {
        for state in [
            SessionState::Starting,
            SessionState::Thinking,
            SessionState::Executing,
            SessionState::AwaitingApproval,
            SessionState::Waiting,
        ] {
            let mut info = info(SessionType::ClaudeCode);
            info.state = state;

            let suffix = header_suffix(&info, InputMode::Session);
            assert_eq!(suffix, "[CC] [SESSION]", "{state:?} leaked into {suffix:?}");
        }
    }

    /// Both of these leave the output frozen mid-page, which looks exactly like
    /// a session sitting at its prompt - the header is the only thing that can
    /// tell the user the process is gone
    #[test]
    fn test_a_session_with_no_process_still_says_so() {
        let mut exited = info(SessionType::ClaudeCode);
        exited.state = SessionState::Exited;
        exited.exit_reason = Some("killed by signal 9".to_string());
        assert_eq!(
            header_suffix(&exited, InputMode::Session),
            "[CC] - Exited (killed by signal 9) [SESSION]"
        );

        // A crash Panoptes could not explain still reports the crash
        exited.exit_reason = None;
        assert_eq!(
            header_suffix(&exited, InputMode::Session),
            "[CC] - Exited [SESSION]"
        );

        let mut suspended = info(SessionType::ClaudeCode);
        suspended.state = SessionState::Suspended;
        assert_eq!(
            header_suffix(&suspended, InputMode::Session),
            "[CC] - Suspended [SESSION]"
        );
    }

    /// "Claude Code, signed in as dot-lambda" is one fact, so it gets one
    /// bracket rather than two with unrelated text between them
    #[test]
    fn test_the_agent_and_its_account_share_a_bracket() {
        let mut claude = info(SessionType::ClaudeCode);
        claude.claude_config_name = Some("dot-lambda".to_string());
        assert_eq!(
            header_suffix(&claude, InputMode::Session),
            "[CC \u{00b7} dot-lambda] [SESSION]"
        );

        // Codex keeps its account in its own field, and used to show nothing
        let mut codex = info(SessionType::OpenAICodex);
        codex.codex_config_name = Some("work".to_string());
        assert_eq!(
            header_suffix(&codex, InputMode::Session),
            "[CX \u{00b7} work] [SESSION]"
        );

        // A session with no account named keeps the bare tag
        assert_eq!(
            header_suffix(&info(SessionType::Shell), InputMode::Session),
            "[SH] [SESSION]"
        );
    }

    /// Everything the terminal cannot report stays, and keeps its order
    #[test]
    fn test_the_suffix_keeps_what_the_screen_cannot_say() {
        let mut info = info(SessionType::ClaudeCode);
        info.claude_config_name = Some("dot-lambda".to_string());
        info.state = SessionState::Suspended;
        info.subagents = 2;

        assert_eq!(
            header_suffix(&info, InputMode::Normal),
            "[CC \u{00b7} dot-lambda] - Suspended \u{00b7} 2 subagents [NORMAL]"
        );
    }

    /// The session header wears the wordmark but not the tagline or version:
    /// every row it takes is a row of agent output the user cannot read
    #[test]
    fn test_session_header_wears_the_wordmark_alone() {
        let state = AppState::default();
        let config = Config::default();
        let sessions = SessionManager::with_store(config.clone(), SessionStore::new());
        let store = ProjectStore::new();
        let header_notifications = HeaderNotificationManager::default();

        let lines = render_to_lines(100, 24, |frame| {
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

        assert!(
            lines
                .iter()
                .any(|l| l.starts_with(crate::tui::logo::WORDMARK[0])),
            "{lines:?}"
        );
        assert!(
            !contains_line(&lines, crate::tui::logo::TAGLINE),
            "{lines:?}"
        );
        assert!(
            !contains_line(&lines, &crate::tui::logo::version()),
            "{lines:?}"
        );
        // The breadcrumb sits beside the wordmark, on its second row
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with(crate::tui::logo::WORDMARK[1]) && l.contains("? > ? > ?")),
            "{lines:?}"
        );
    }
}
