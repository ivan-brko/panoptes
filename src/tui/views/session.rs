//! Session view
//!
//! Fullscreen view for interacting with a single agent session: the header,
//! the agent's screen edge to edge, and the footer. Nothing is drawn around
//! the middle - the header and footer each draw their own rule, and the rows
//! and columns a box would have cost belong to the agent.

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::app::{selection, AppState, SessionSelection};
use crate::project::ProjectStore;
use crate::session::{Session, SessionInfo, SessionManager, SessionState, SessionType};
use crate::tui::frame::{render_pty_content, FrameConfig, FrameLayout};
use crate::tui::header::{Header, LogoKind};
use crate::tui::header_notifications::HeaderNotificationManager;
use crate::tui::theme::theme;
use crate::tui::views::Breadcrumb;
use crate::tui::views::{footer_with_attention, render_footer};

/// Render the session view
pub fn render_session_view(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    sessions: &SessionManager,
    project_store: &ProjectStore,
    header_notifications: &HeaderNotificationManager,
) {
    let t = theme();
    let session = state.active_session.and_then(|id| sessions.get(id));
    let attention_count = sessions.total_attention_count();

    // How far back the reader is from live output. Codex keeps its own answer
    // because a scrolled Codex session may be reading the plain-text fallback
    // buffer, which the vterm knows nothing about.
    let scroll_offset = session
        .map(|s| {
            if s.info.session_type == SessionType::OpenAICodex {
                state.session_scroll_offset
            } else {
                s.vterm.scrollback_offset()
            }
        })
        .unwrap_or(0);

    // === HEADER ===
    // Built before the layout, because how many rows it needs depends on how
    // much of the wordmark this terminal can afford
    let (breadcrumb, suffix) = build_header_breadcrumb(session, project_store, scroll_offset);

    // Session header has custom coloring based on session state
    let header_color = session.map(|s| s.info.state.color()).unwrap_or(t.text_dim);
    let custom_style = Style::default().fg(header_color).bold();

    // The wordmark only, without the tagline and version the pane screen
    // carries: every row here is agent output the user is reading
    let header = Header::new(breadcrumb)
        .with_logo(LogoKind::Wordmark)
        .with_suffix(suffix)
        .with_notifications(Some(header_notifications))
        .with_attention_count(attention_count)
        .with_custom_style(custom_style);

    // Pre-calculate layout using FrameLayout. The header height must come
    // from the same answer `FrameConfig::for_terminal` gives the off-screen
    // layout math (PTY sizing, mouse translation), or clicks land a row off.
    let layout = FrameLayout::calculate(area, &FrameConfig::for_terminal(area));

    header.render(frame, layout.header);

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

            // Get cursor info. Scrolled back there is no cursor to show: the
            // rows on screen are history, and the agent's cursor is somewhere
            // below them.
            let cursor_pos = session.vterm.cursor_position();
            let cursor_visible =
                session.vterm.cursor_visible() && session.vterm.scrollback_offset() == 0;

            render_pty_content(
                frame,
                layout.content,
                &styled_lines,
                Some(cursor_pos),
                cursor_visible,
            );

            // Painted over the rendered cells, never baked into them:
            // `visible_styled_lines` is cached per viewport and scroll offset,
            // and a highlight folded into that cache would outlive the drag
            if let Some(selection) = state.selection_for(session.info.id) {
                paint_selection(frame.buffer_mut(), layout.content, session, selection);
            }
        }
    } else {
        let empty = Paragraph::new("Session not found").style(Style::default().fg(t.error_bg));
        frame.render_widget(empty, layout.content);
    }

    // === FOOTER ===
    let is_scrolled = scroll_offset > 0;
    let suspended = session.is_some_and(|s| s.info.state == SessionState::Suspended);
    // Who owns the mouse decides which drag copies: when the child asked for
    // mouse reporting a plain drag drives its own selection, exactly as in a
    // real terminal tab, and \u{21E7}drag is the way past it
    let child_owns_mouse =
        session.is_some_and(|s| s.vterm.mouse_protocol_mode() != vt100::MouseProtocolMode::None);
    let help_text = build_footer_text(is_scrolled, suspended, child_owns_mouse, sessions);
    render_footer(frame, layout.footer, &help_text);
}

/// Paint the mouse selection over the rendered terminal content
///
/// Two shapes. A stream selection is what every terminal draws: the first row
/// runs from the anchor column to the end of the line, whole rows follow, and
/// the last row stops at the head column. A block selection is the same column
/// range on every row.
///
/// Rows the view has scrolled past are simply not drawn - the selection is
/// anchored in absolute rows, so it can be taller than the screen.
fn paint_selection(buf: &mut Buffer, area: Rect, session: &Session, selection: &SessionSelection) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let style = theme().text_selection_style();
    let last_col = area.width - 1;
    let ((start_row, start_col), (end_row, end_col)) = selection.ordered();
    let ((top, bottom), (left, right)) = selection.block_bounds();
    let block = selection.shape == selection::Shape::Block;

    // Which rows the shape covers at all. A rectangle is bounded by its own
    // top and bottom, which are ordered independently of the drag's direction.
    let (from_row, to_row) = if block {
        (top, bottom)
    } else {
        (start_row, end_row)
    };

    // Only the rows on screen are worth walking: a selection dragged through
    // a long scrollback can be thousands of rows tall
    let viewport_top = session.vterm.viewport_top_row();
    let first_row = from_row.max(viewport_top);
    let last_row = to_row.min(viewport_top + usize::from(area.height) - 1);
    if last_row < first_row {
        return;
    }

    for row in first_row..=last_row {
        let Some(view_row) = selection::view_row(viewport_top, row, area.height) else {
            continue;
        };
        let (first, last) = if block {
            (left, right)
        } else {
            (
                if row == start_row { start_col } else { 0 },
                if row == end_row { end_col } else { last_col },
            )
        };
        if first > last_col {
            continue;
        }

        let y = area.y + view_row;
        for x in first..=last.min(last_col) {
            buf.get_mut(area.x + x, y).set_style(style);
        }
    }
}

/// What the session header says after the breadcrumb
///
/// Everything the terminal below cannot say for itself, and nothing it can.
/// No mode tag: there is one mode, and a tag that never changes is not
/// information.
///
/// `scroll_offset` is how far back from live output the reader is. It used to
/// be the title of the box drawn around the content; with the box gone this
/// is where it lives, and it goes first because it is the one thing here that
/// changes what the rows below *mean* - they are history, not what the agent
/// is doing now.
fn header_suffix(info: &SessionInfo, scroll_offset: usize) -> String {
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

    let scroll_display = if scroll_offset > 0 {
        format!("[\u{2191}{}] ", scroll_offset)
    } else {
        String::new()
    };

    format!(
        "{}{}{}{}{}",
        scroll_display, agent_display, state_display, subagent_display, usage_display
    )
}

/// Build breadcrumb and suffix for the session header
fn build_header_breadcrumb(
    session: Option<&Session>,
    project_store: &ProjectStore,
    scroll_offset: usize,
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

    (breadcrumb, header_suffix(&session.info, scroll_offset))
}

/// The session footer names only what Panoptes still answers
///
/// Which is very little, and deliberately: every key but `Esc` belongs to the
/// agent, so a footer listing scroll keys and session-switching digits would
/// be listing keys that now type into Claude Code. What is left is the way
/// out, the way to send a literal `Esc`, and how the mouse behaves.
///
/// The attention badge stays. It is the one thing here that is not about the
/// session on screen, and being buried in one session is exactly when another
/// one wanting you is worth knowing.
fn build_footer_text(
    is_scrolled: bool,
    suspended: bool,
    child_owns_mouse: bool,
    sessions: &SessionManager,
) -> String {
    // Panoptes selects for itself while the mouse is ours. When the child
    // asked for mouse reporting a plain drag belongs to it - so the footer
    // names the one that still copies here, which is \u{21E7}drag: shift
    // claims the event for the terminal before anything is forwarded.
    let drag = if child_owns_mouse {
        "\u{21E7}drag: copy"
    } else {
        "drag: copy"
    };

    // Say what a suspended session is before saying what to do with it: the
    // scrollback still reads as a live session, so without this the missing
    // process looks like a hang rather than a deliberate saving.
    if suspended {
        return footer_with_attention(
            format!("Esc: back | Suspended to save memory | Type to wake | {drag}"),
            sessions,
        );
    }

    // Scrolled back, there is no key that returns to live output - typing does
    // it, because a keystroke belongs to the agent and reaching the agent
    // means being where the agent is.
    let live = if is_scrolled {
        "type: live view | "
    } else {
        ""
    };
    footer_with_attention(
        format!("Esc: back | \u{21E7}Esc: send Esc | {live}{drag}"),
        sessions,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::session::store::SessionStore;
    use crate::tui::views::test_util::{contains_line, render_to_buffer, render_to_lines};

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
                &header_notifications,
            )
        });

        assert!(contains_line(&lines, "Session not found"), "{:?}", lines);
        assert!(contains_line(&lines, "? > ? > ?"), "{:?}", lines);
        assert!(contains_line(&lines, "Esc: back"), "{:?}", lines);
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

            let suffix = header_suffix(&info, 0);
            assert_eq!(suffix, "[CC]", "{state:?} leaked into {suffix:?}");
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
            header_suffix(&exited, 0),
            "[CC] - Exited (killed by signal 9)"
        );

        // A crash Panoptes could not explain still reports the crash
        exited.exit_reason = None;
        assert_eq!(header_suffix(&exited, 0), "[CC] - Exited");

        let mut suspended = info(SessionType::ClaudeCode);
        suspended.state = SessionState::Suspended;
        assert_eq!(header_suffix(&suspended, 0), "[CC] - Suspended");
    }

    /// "Claude Code, signed in as dot-lambda" is one fact, so it gets one
    /// bracket rather than two with unrelated text between them
    #[test]
    fn test_the_agent_and_its_account_share_a_bracket() {
        let mut claude = info(SessionType::ClaudeCode);
        claude.claude_config_name = Some("dot-lambda".to_string());
        assert_eq!(header_suffix(&claude, 0), "[CC \u{00b7} dot-lambda]");

        // Codex keeps its account in its own field, and used to show nothing
        let mut codex = info(SessionType::OpenAICodex);
        codex.codex_config_name = Some("work".to_string());
        assert_eq!(header_suffix(&codex, 0), "[CX \u{00b7} work]");

        // A session with no account named keeps the bare tag
        assert_eq!(header_suffix(&info(SessionType::Shell), 0), "[SH]");
    }

    /// Everything the terminal cannot report stays, and keeps its order
    #[test]
    fn test_the_suffix_keeps_what_the_screen_cannot_say() {
        let mut info = info(SessionType::ClaudeCode);
        info.claude_config_name = Some("dot-lambda".to_string());
        info.state = SessionState::Suspended;
        info.subagents = 2;

        assert_eq!(
            header_suffix(&info, 0),
            "[CC \u{00b7} dot-lambda] - Suspended \u{00b7} 2 subagents"
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

    // Mouse selection

    /// A session manager holding one live session showing `text` on its first
    /// row, plus the state that puts it on screen in session mode
    fn session_showing(text: &str) -> (AppState, SessionManager, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let config = Config::default();
        let mut sessions = SessionManager::with_store(
            config,
            SessionStore::with_path(dir.path().join("sessions.json")),
        );
        let session_id = sessions
            .insert_test_session("selected", uuid::Uuid::new_v4(), uuid::Uuid::new_v4())
            .unwrap();
        sessions
            .get_mut(session_id)
            .unwrap()
            .vterm
            .process(text.as_bytes());

        let mut state = AppState::default();
        state.navigate_to_session(session_id);
        (state, sessions, dir)
    }

    /// A selection covering `span`, with the button already released
    fn finished(
        session_id: uuid::Uuid,
        span: (selection::Cell, selection::Cell),
        pointer: (u16, u16),
    ) -> SessionSelection {
        let mut sel = SessionSelection::started(
            session_id,
            span,
            selection::Granularity::Cell,
            pointer,
            selection::Shape::Stream,
        );
        sel.dragging = false;
        sel
    }

    fn render_session(state: &AppState, sessions: &SessionManager) -> ratatui::buffer::Buffer {
        let store = ProjectStore::new();
        let header_notifications = HeaderNotificationManager::default();
        render_to_buffer(80, 24, |frame| {
            render_session_view(
                frame,
                frame.size(),
                state,
                sessions,
                &store,
                &header_notifications,
            )
        })
    }

    /// The cell the drag covered is highlighted and the one past it is not -
    /// which is also how the user sees where the copy will stop
    #[test]
    fn test_the_selection_is_painted_over_exactly_the_cells_it_covers() {
        let (mut state, sessions, _dir) = session_showing("FIDELITY-42 and more");
        let session_id = state.active_session.unwrap();
        state.selection = Some(finished(session_id, ((0, 0), (0, 7)), (0, 7)));

        let buffer = render_session(&state, &sessions);
        let content = FrameLayout::calculate(
            Rect::new(0, 0, 80, 24),
            &FrameConfig::for_terminal(Rect::new(0, 0, 80, 24)),
        )
        .content;
        let selected = theme().text_selection_style();

        let row = content.y;
        for col in 0..8 {
            let cell = buffer.get(content.x + col, row);
            assert_eq!(cell.style().bg, selected.bg, "column {col}: {:?}", cell);
        }
        // "FIDELITY" ends at column 7; the hyphen after it is not selected
        let after = buffer.get(content.x + 8, row);
        assert_eq!(after.symbol(), "-");
        assert_ne!(after.style().bg, selected.bg, "{after:?}");
    }

    /// A selection whose rows have all scrolled out of view paints nothing -
    /// and must not paint the wrong rows instead
    #[test]
    fn test_an_off_screen_selection_paints_nothing() {
        let (mut state, sessions, _dir) = session_showing("FIDELITY-42 and more");
        let session_id = state.active_session.unwrap();
        // Absolute rows far below the live screen
        state.selection = Some(finished(session_id, ((900, 0), (900, 7)), (0, 7)));

        let buffer = render_session(&state, &sessions);
        let selected = theme().text_selection_style();
        let painted = (0..buffer.area.height)
            .any(|y| (0..buffer.area.width).any(|x| buffer.get(x, y).style().bg == selected.bg));
        assert!(!painted, "nothing on screen is part of the selection");
    }

    /// Another session's selection is never drawn over this one's output
    #[test]
    fn test_a_selection_from_another_session_is_not_drawn() {
        let (mut state, sessions, _dir) = session_showing("FIDELITY-42 and more");
        state.selection = Some(finished(uuid::Uuid::new_v4(), ((0, 0), (0, 7)), (0, 7)));

        let buffer = render_session(&state, &sessions);
        let selected = theme().text_selection_style();
        let painted = (0..buffer.area.height)
            .any(|y| (0..buffer.area.width).any(|x| buffer.get(x, y).style().bg == selected.bg));
        assert!(!painted);
    }

    /// The footer says whose drag it is: a plain drag copies while the mouse
    /// is ours, and \u{21E7}drag is the way past a child that took it
    #[test]
    fn test_the_footer_names_the_drag_that_copies() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = Config::default();
        let sessions = SessionManager::with_store(
            config.clone(),
            SessionStore::with_path(dir.path().join("sessions.json")),
        );

        let ours = build_footer_text(false, false, false, &sessions);
        assert!(ours.contains("drag: copy"), "{ours}");
        assert!(!ours.contains('\u{2325}'), "{ours}");
        // `\u{21E7}Esc` is always there, so the check has to name the drag
        assert!(
            !ours.contains("\u{21E7}drag"),
            "no modifier needed when the mouse is ours: {ours}"
        );

        let theirs = build_footer_text(false, false, true, &sessions);
        assert!(theirs.contains("\u{21E7}drag: copy"), "{theirs}");
        // Option-drag was the old answer, and is the terminal's own selection
        // rather than Panoptes' - it must not be offered as if it were ours
        assert!(!theirs.contains('\u{2325}'), "{theirs}");

        // Scrolled up, typing is what returns to live output
        let scrolled = build_footer_text(true, false, false, &sessions);
        assert!(scrolled.contains("type: live view"), "{scrolled}");
    }

    /// The footer must not offer a key the agent now takes
    ///
    /// Every one of these was a real binding in the session view before the
    /// modes collapsed. Naming any of them now would be telling the user to
    /// press something that types into Claude Code instead.
    #[test]
    fn test_the_footer_offers_no_key_that_belongs_to_the_agent() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = Config::default();
        let sessions = SessionManager::with_store(
            config.clone(),
            SessionStore::with_path(dir.path().join("sessions.json")),
        );

        for suspended in [false, true] {
            for is_scrolled in [false, true] {
                let footer = build_footer_text(is_scrolled, suspended, false, &sessions);
                for gone in [
                    "PgUp",
                    "PgDn",
                    "Ctrl+End",
                    "Ctrl+Home",
                    "1-9",
                    "Enter:",
                    "session mode",
                ] {
                    assert!(
                        !footer.contains(gone),
                        "{gone:?} still offered in {footer:?}"
                    );
                }
                // What is left is the way out and the literal Esc
                assert!(footer.contains("Esc: back"), "{footer}");
                assert!(
                    footer.contains("\u{21E7}Esc: send Esc") || suspended,
                    "{footer}"
                );
            }
        }
    }
}
