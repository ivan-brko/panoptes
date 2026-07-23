//! Shared session scrolling helpers
//!
//! Centralizes scroll behavior so Session mode, Session view (normal mode),
//! and mouse-wheel handling stay consistent. The session-level engine
//! functions implement the Codex vterm-scrollback-with-fallback dance in one
//! place; the `App`-level wrappers resolve the session and viewport.

use crate::app::App;
use crate::session::{Session, SessionId, SessionType};
use crate::tui::frame::{FrameConfig, FrameLayout};

/// Number of lines to scroll per arrow key press.
const ARROW_SCROLL_STEP: usize = 3;

/// What a scroll step did, for caller-side debug logging.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScrollOutcome {
    /// The vterm scrollback offset that was requested
    pub(crate) requested_offset: usize,
    /// The vterm scrollback offset after the step
    pub(crate) vterm_offset: usize,
    /// The Codex fallback-buffer scroll offset after the step
    pub(crate) fallback_offset: usize,
}

fn viewport_height(app: &App) -> usize {
    let terminal_size = app.tui.size().unwrap_or_default();
    let frame_config = FrameConfig::default();
    let layout = FrameLayout::calculate(terminal_size, &frame_config);
    layout.content.height as usize
}

/// Scroll a session up (toward older content) by `amount` lines.
///
/// For Codex sessions this prefers vterm scrollback and switches to the
/// plain-text fallback buffer when the vterm cannot advance (Codex runs in
/// the alternate screen, where vt100 keeps no scrollback). `scroll_offset`
/// is the app-level offset shown in the UI (0 = live view).
fn scroll_session_up(
    session: &mut Session,
    scroll_offset: &mut usize,
    viewport_height: usize,
    amount: usize,
) -> ScrollOutcome {
    if session.info.session_type == SessionType::OpenAICodex {
        let current_vterm = session.vterm.scrollback_offset();
        let requested = current_vterm.saturating_add(amount);
        session.vterm.set_scrollback(requested);
        let vterm_offset = session.vterm.scrollback_offset();
        let vterm_advanced = vterm_offset > current_vterm;
        if vterm_offset > 0 && vterm_advanced {
            session.fallback_scroll_to_bottom();
            *scroll_offset = vterm_offset;
        } else {
            // vterm scrollback can stop advancing at a shallow offset.
            // Switch to fallback mode for continued upward scrolling.
            session.vterm.scroll_to_bottom();
            session.fallback_scroll_up_with_viewport(amount, viewport_height);
            *scroll_offset = session.fallback_scroll_offset();
        }
        ScrollOutcome {
            requested_offset: requested,
            vterm_offset: session.vterm.scrollback_offset(),
            fallback_offset: session.fallback_scroll_offset(),
        }
    } else {
        let max_scroll = session.vterm.scrollback_capacity();
        *scroll_offset = scroll_offset.saturating_add(amount).min(max_scroll);
        session.vterm.set_scrollback(*scroll_offset);
        ScrollOutcome {
            requested_offset: *scroll_offset,
            vterm_offset: session.vterm.scrollback_offset(),
            fallback_offset: 0,
        }
    }
}

/// Scroll a session down (toward newer content) by `amount` lines.
fn scroll_session_down(
    session: &mut Session,
    scroll_offset: &mut usize,
    amount: usize,
) -> ScrollOutcome {
    if session.info.session_type == SessionType::OpenAICodex {
        let current_vterm = session.vterm.scrollback_offset();
        let requested = current_vterm.saturating_sub(amount);
        if current_vterm > 0 {
            session.vterm.set_scrollback(requested);
            let vterm_offset = session.vterm.scrollback_offset();
            if vterm_offset == 0 {
                session.fallback_scroll_to_bottom();
            }
            *scroll_offset = vterm_offset;
        } else {
            session.fallback_scroll_down(amount);
            *scroll_offset = session.fallback_scroll_offset();
        }
        ScrollOutcome {
            requested_offset: requested,
            vterm_offset: session.vterm.scrollback_offset(),
            fallback_offset: session.fallback_scroll_offset(),
        }
    } else {
        *scroll_offset = scroll_offset.saturating_sub(amount);
        session.vterm.set_scrollback(*scroll_offset);
        ScrollOutcome {
            requested_offset: *scroll_offset,
            vterm_offset: session.vterm.scrollback_offset(),
            fallback_offset: 0,
        }
    }
}

/// Scroll a session to the oldest available output.
fn scroll_session_to_top(session: &mut Session, scroll_offset: &mut usize, viewport_height: usize) {
    if session.info.session_type == SessionType::OpenAICodex {
        session.vterm.set_scrollback(usize::MAX);
        let vterm_offset = session.vterm.scrollback_offset();
        if vterm_offset > 0 {
            session.fallback_scroll_to_bottom();
            *scroll_offset = vterm_offset;
        } else {
            session.fallback_scroll_to_top_with_viewport(viewport_height);
            *scroll_offset = session.fallback_scroll_offset();
        }
    } else {
        let max_scroll = session.vterm.scrollback_capacity();
        *scroll_offset = max_scroll;
        session.vterm.set_scrollback(*scroll_offset);
    }
}

/// Return a session to live output (bottom).
fn scroll_session_to_bottom(session: &mut Session, scroll_offset: &mut usize) {
    *scroll_offset = 0;
    session.vterm.scroll_to_bottom();
    if session.info.session_type == SessionType::OpenAICodex {
        session.fallback_scroll_to_bottom();
    }
}

/// Scroll up by a given number of lines.
///
/// Returns `None` when the session does not exist, `Some(outcome)` otherwise
/// so callers can log what happened.
pub(crate) fn scroll_up_by(
    app: &mut App,
    session_id: SessionId,
    amount: usize,
) -> Option<ScrollOutcome> {
    let viewport_height = viewport_height(app);
    let mut offset = app.state.session_scroll_offset;
    let session = app.sessions.get_mut(session_id)?;
    let outcome = scroll_session_up(session, &mut offset, viewport_height, amount);
    app.state.session_scroll_offset = offset;
    Some(outcome)
}

/// Scroll down by a given number of lines.
///
/// Returns `None` when the session does not exist, `Some(outcome)` otherwise
/// so callers can log what happened.
pub(crate) fn scroll_down_by(
    app: &mut App,
    session_id: SessionId,
    amount: usize,
) -> Option<ScrollOutcome> {
    let mut offset = app.state.session_scroll_offset;
    let session = app.sessions.get_mut(session_id)?;
    let outcome = scroll_session_down(session, &mut offset, amount);
    app.state.session_scroll_offset = offset;
    Some(outcome)
}

/// Scroll up by one viewport page.
pub fn scroll_page_up(app: &mut App, session_id: SessionId) {
    let height = viewport_height(app);
    scroll_up_by(app, session_id, height);
}

/// Scroll down by one viewport page.
pub fn scroll_page_down(app: &mut App, session_id: SessionId) {
    let height = viewport_height(app);
    scroll_down_by(app, session_id, height);
}

/// Scroll to oldest available output.
pub fn scroll_to_top(app: &mut App, session_id: SessionId) {
    let viewport_height = viewport_height(app);
    let mut offset = app.state.session_scroll_offset;
    if let Some(session) = app.sessions.get_mut(session_id) {
        scroll_session_to_top(session, &mut offset, viewport_height);
        app.state.session_scroll_offset = offset;
    }
}

/// Return to live output (bottom).
pub fn scroll_to_bottom(app: &mut App, session_id: SessionId) {
    let mut offset = app.state.session_scroll_offset;
    if let Some(session) = app.sessions.get_mut(session_id) {
        scroll_session_to_bottom(session, &mut offset);
        app.state.session_scroll_offset = offset;
    }
}

/// Scroll up by a few lines (arrow key).
pub fn scroll_lines_up(app: &mut App, session_id: SessionId) {
    scroll_up_by(app, session_id, ARROW_SCROLL_STEP);
}

/// Scroll down by a few lines (arrow key).
pub fn scroll_lines_down(app: &mut App, session_id: SessionId) {
    scroll_down_by(app, session_id, ARROW_SCROLL_STEP);
}

/// Reset app-level scroll when changing active session.
pub fn reset_for_session_switch(app: &mut App, session_id: SessionId) {
    app.state.session_scroll_offset = 0;
    if let Some(session) = app.sessions.get_mut(session_id) {
        if session.info.session_type == SessionType::OpenAICodex {
            session.fallback_scroll_to_bottom();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{PtyHandle, SessionInfo};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    const VIEWPORT: usize = 5;

    /// Spawn a real PTY that prints `lines` numbered lines (ending with
    /// "line-END") and stays alive, wrapped in a Session of the given type.
    ///
    /// `alt_screen` enters the alternate screen first, which is how Codex
    /// actually runs: vt100 keeps no scrollback there, forcing the fallback
    /// buffer path.
    fn spawn_session(codex: bool, alt_screen: bool, lines: usize) -> Session {
        let mut script = String::new();
        if alt_screen {
            script.push_str("printf '\\033[?1049h'; ");
        }
        script.push_str(&format!(
            "i=1; while [ $i -le {} ]; do echo line-$i; i=$((i+1)); done; echo line-END; sleep 30",
            lines
        ));

        let pty = PtyHandle::spawn(
            "sh",
            &["-c", &script],
            &PathBuf::from("/tmp"),
            HashMap::new(),
            VIEWPORT as u16,
            80,
        )
        .expect("failed to spawn PTY");

        let info = if codex {
            SessionInfo::codex(
                "scroll-test".to_string(),
                PathBuf::from("/tmp"),
                uuid::Uuid::new_v4(),
                uuid::Uuid::new_v4(),
            )
        } else {
            SessionInfo::new(
                "scroll-test".to_string(),
                PathBuf::from("/tmp"),
                uuid::Uuid::new_v4(),
                uuid::Uuid::new_v4(),
            )
        };

        let mut session = Session::new(info, pty, VIEWPORT, 80);
        wait_for_marker(&mut session, codex);
        session
    }

    /// Poll the PTY until the end marker has been ingested.
    fn wait_for_marker(session: &mut Session, codex: bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            session.poll_output();
            let seen = if codex {
                // The fallback buffer sees everything; check it for Codex.
                let mut all = session.fallback_visible_lines(usize::MAX);
                all.retain(|l| l.contains("line-END"));
                !all.is_empty()
            } else {
                session
                    .visible_styled_lines(VIEWPORT)
                    .iter()
                    .any(|line| format!("{:?}", line).contains("line-END"))
            };
            if seen {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("PTY output marker never arrived");
    }

    #[test]
    fn codex_scroll_up_uses_vterm_scrollback_when_available() {
        // Primary screen: vt100 accumulates scrollback, so the vterm path
        // engages and the fallback buffer stays at the bottom.
        let mut session = spawn_session(true, false, 100);
        let mut offset = 0usize;

        let outcome = scroll_session_up(&mut session, &mut offset, VIEWPORT, 3);

        assert_eq!(outcome.requested_offset, 3);
        assert_eq!(outcome.vterm_offset, 3);
        assert_eq!(outcome.fallback_offset, 0);
        assert_eq!(offset, 3);
    }

    #[test]
    fn codex_scroll_up_falls_back_when_vterm_cannot_advance() {
        // Alternate screen: vt100 has no scrollback, so the fallback buffer
        // must take over.
        let mut session = spawn_session(true, true, 100);
        let mut offset = 0usize;

        let outcome = scroll_session_up(&mut session, &mut offset, VIEWPORT, 3);

        assert_eq!(outcome.vterm_offset, 0);
        assert!(outcome.fallback_offset > 0, "fallback should have engaged");
        assert_eq!(offset, outcome.fallback_offset);

        // Scrolling further keeps advancing the fallback offset.
        let prev = offset;
        scroll_session_up(&mut session, &mut offset, VIEWPORT, 3);
        assert!(offset > prev);
    }

    #[test]
    fn codex_scroll_down_returns_to_live_view() {
        let mut session = spawn_session(true, true, 100);
        let mut offset = 0usize;

        scroll_session_up(&mut session, &mut offset, VIEWPORT, 6);
        assert!(offset > 0);

        // Scroll down more than we scrolled up: clamps at the bottom.
        scroll_session_down(&mut session, &mut offset, 1000);
        assert_eq!(offset, 0);
        assert_eq!(session.fallback_scroll_offset(), 0);

        // Scrolling down at the bottom stays at the bottom.
        scroll_session_down(&mut session, &mut offset, 3);
        assert_eq!(offset, 0);
    }

    #[test]
    fn codex_scroll_to_top_clamps_and_bottom_restores_live() {
        let mut session = spawn_session(true, true, 100);
        let mut offset = 0usize;

        scroll_session_to_top(&mut session, &mut offset, VIEWPORT);
        let top = offset;
        assert!(top > 0, "scroll_to_top should move away from live view");

        // Scrolling up at the top does not go past it.
        scroll_session_up(&mut session, &mut offset, VIEWPORT, 1000);
        assert_eq!(offset, top);

        scroll_session_to_bottom(&mut session, &mut offset);
        assert_eq!(offset, 0);
        assert_eq!(session.vterm.scrollback_offset(), 0);
        assert_eq!(session.fallback_scroll_offset(), 0);
    }

    #[test]
    fn non_codex_scroll_clamps_at_capacity_and_bottom() {
        let mut session = spawn_session(false, false, 50);
        let mut offset = 0usize;
        let capacity = session.vterm.scrollback_capacity();

        // Ordinary step moves by the requested amount.
        scroll_session_up(&mut session, &mut offset, VIEWPORT, 3);
        assert_eq!(offset, 3);

        // A huge step clamps to scrollback capacity (matching pre-refactor
        // behavior: the offset tracks the request clamped to capacity, while
        // the vterm clamps internally to the history it actually has).
        scroll_session_up(&mut session, &mut offset, VIEWPORT, usize::MAX);
        assert_eq!(offset, capacity);

        // scroll_to_top pins the offset at capacity too.
        scroll_session_to_top(&mut session, &mut offset, VIEWPORT);
        assert_eq!(offset, capacity);

        // Scrolling down past the bottom clamps at 0.
        scroll_session_down(&mut session, &mut offset, usize::MAX);
        assert_eq!(offset, 0);

        scroll_session_up(&mut session, &mut offset, VIEWPORT, 3);
        scroll_session_to_bottom(&mut session, &mut offset);
        assert_eq!(offset, 0);
        assert_eq!(session.vterm.scrollback_offset(), 0);
    }
}
