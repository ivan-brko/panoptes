//! Shared session scrolling helpers
//!
//! The session-level engine functions implement the Codex
//! vterm-scrollback-with-fallback dance in one place; the `App`-level wrappers
//! resolve the session and viewport.
//!
//! Every scroll now arrives from the mouse wheel or from a selection dragged
//! past the edge of the screen. The keyboard entry points this module was
//! written for are gone with the mode that owned them - a session forwards
//! every key but `Esc` to the agent - so the wrappers they called
//! (`scroll_page_up`, `scroll_lines_up`, `scroll_to_top` and friends) have no
//! production callers left.
//!
//! Both clamps ask the same question, whichever buffer answers: **stop at the
//! history that exists**, never at the capacity that was configured.

use crate::app::App;
use crate::session::{Session, SessionId, SessionType};

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

/// How many rows a page is: the content area the session view is drawing
///
/// Straight from [`App::session_frame_layout`], because a page has to be the
/// page the reader can see. Guessing the header's height here made `PgUp`
/// step one row further than the screen did, so a line of output fell through
/// the seam on every press.
fn viewport_height(app: &App) -> usize {
    app.session_frame_layout()
        .map(|layout| layout.content.height as usize)
        .unwrap_or_default()
}

/// Whether the terminal emulator holds this session's history
///
/// The one question that decides which buffer a Codex scroll reads, and it is
/// asked the same way everywhere. `true` when vt100 has scrollback rows: the
/// session is drawing on the primary screen and the vterm *is* the history.
/// `false` for an agent on the alternate screen, where vt100 keeps none and
/// the plain-text fallback buffer is the only history there is.
///
/// Deliberately not "did the vterm advance this step". That conflates "there
/// is no vterm history" with "you have reached the top of the vterm history",
/// and the second is a clamp, not a reason to change buffers.
fn vterm_holds_history(session: &Session) -> bool {
    session.vterm.history_rows() > 0
}

/// Scroll a session up (toward older content) by `amount` lines.
///
/// For Codex sessions this reads vterm scrollback when the vterm has any, and
/// the plain-text fallback buffer when it does not. `scroll_offset` is the
/// app-level offset shown in the UI (0 = live view).
fn scroll_session_up(
    session: &mut Session,
    scroll_offset: &mut usize,
    viewport_height: usize,
    amount: usize,
) -> ScrollOutcome {
    if session.info.session_type == SessionType::OpenAICodex {
        let requested = session.vterm.scrollback_offset().saturating_add(amount);
        if vterm_holds_history(session) {
            // `set_scrollback` clamps to the rows that exist, so paging past
            // the oldest line leaves the view on the oldest line - the same
            // stop a non-Codex session makes. It must not fall through to the
            // fallback buffer here: that discards the reader's position and
            // drops them into a different, shallower history, so scrolling up
            // moves the view down.
            session.vterm.set_scrollback(requested);
            session.fallback_scroll_to_bottom();
            *scroll_offset = session.vterm.scrollback_offset();
        } else {
            session.fallback_scroll_up_with_viewport(amount, viewport_height);
            *scroll_offset = session.fallback_scroll_offset();
        }
        ScrollOutcome {
            requested_offset: requested,
            vterm_offset: session.vterm.scrollback_offset(),
            fallback_offset: session.fallback_scroll_offset(),
        }
    } else {
        // Clamped against the history that exists, not the capacity that was
        // configured. The vterm clamps internally either way, so a capacity
        // clamp let this counter climb to 10000 while the screen stopped at
        // the oldest row - and every notch back down then moved the counter
        // without moving the view.
        let max_scroll = session.vterm.history_rows();
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
        if vterm_holds_history(session) {
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
        if vterm_holds_history(session) {
            session.vterm.set_scrollback(usize::MAX);
            session.fallback_scroll_to_bottom();
            *scroll_offset = session.vterm.scrollback_offset();
        } else {
            session.fallback_scroll_to_top_with_viewport(viewport_height);
            *scroll_offset = session.fallback_scroll_offset();
        }
    } else {
        *scroll_offset = session.vterm.history_rows();
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

    /// Paging past the oldest line of a Codex session must stop there.
    ///
    /// The regression: reaching the top of real vterm scrollback made the
    /// vterm fail to advance, which used to be read as "this session has no
    /// vterm history" and handed the reader to the fallback buffer - from the
    /// live view. Scrolling up moved the view down.
    #[test]
    fn codex_scroll_up_clamps_at_the_top_of_vterm_history() {
        // Primary screen, so the vterm holds the history.
        let mut session = spawn_session(true, false, 100);
        let mut offset = 0usize;

        // Walk up a page at a time until the offset stops climbing.
        let mut top = 0usize;
        for _ in 0..200 {
            scroll_session_up(&mut session, &mut offset, VIEWPORT, VIEWPORT);
            if offset == top {
                break;
            }
            assert!(
                offset > top,
                "scrolling up moved the view down: {top} -> {offset}"
            );
            top = offset;
        }
        assert!(top > 0, "the session should have had scrollback to walk");

        // At the top, further page-ups stay put rather than resetting.
        for _ in 0..5 {
            scroll_session_up(&mut session, &mut offset, VIEWPORT, VIEWPORT);
            assert_eq!(offset, top, "scrolling past the top left the top");
        }

        // And the history is still the vterm's, not the fallback's.
        assert_eq!(session.vterm.scrollback_offset(), top);
        assert_eq!(session.fallback_scroll_offset(), 0);
    }

    /// `Home` on a primary-screen Codex session lands on the oldest row and
    /// stays there, rather than taking the fallback.
    #[test]
    fn codex_scroll_to_top_uses_vterm_history_when_it_exists() {
        let mut session = spawn_session(true, false, 100);
        let mut offset = 0usize;

        scroll_session_to_top(&mut session, &mut offset, VIEWPORT);

        assert_eq!(offset, session.vterm.history_rows());
        assert_eq!(session.fallback_scroll_offset(), 0);

        scroll_session_up(&mut session, &mut offset, VIEWPORT, 1000);
        assert_eq!(offset, session.vterm.history_rows());
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

    /// Wheeling up past the oldest line must not run a counter off into
    /// space, or wheeling back down does nothing until it unwinds
    ///
    /// The app-level offset used to clamp against `scrollback_capacity()` -
    /// the *configured* 10000 rows, not the history that exists - while the
    /// vterm clamped internally to the real thing. Twenty lines of output and
    /// a determined wheel left the offset at 10000 and the view at 20, and
    /// every notch down then moved the counter without moving the screen.
    #[test]
    fn non_codex_scroll_up_stops_at_the_history_that_exists() {
        let mut session = spawn_session(false, false, 50);
        let mut offset = 0usize;

        // Wheel up far past the top
        for _ in 0..60 {
            scroll_session_up(&mut session, &mut offset, VIEWPORT, 3);
        }
        let history = session.vterm.history_rows();
        assert!(history > 0, "the session should have had history to scroll");
        assert_eq!(offset, history, "the offset outran the history that exists");

        // One notch down has to move the screen, not just the counter
        let before = session.vterm.scrollback_offset();
        scroll_session_down(&mut session, &mut offset, 3);
        assert!(
            session.vterm.scrollback_offset() < before,
            "scrolling down moved the counter but not the view ({before} -> {})",
            session.vterm.scrollback_offset()
        );
    }

    #[test]
    fn non_codex_scroll_clamps_at_the_history_and_the_bottom() {
        let mut session = spawn_session(false, false, 50);
        let mut offset = 0usize;
        let history = session.vterm.history_rows();
        assert!(history > 0 && history < session.vterm.scrollback_capacity());

        // Ordinary step moves by the requested amount.
        scroll_session_up(&mut session, &mut offset, VIEWPORT, 3);
        assert_eq!(offset, 3);

        // A huge step stops on the oldest row that exists. It used to stop at
        // the configured capacity instead, which the vterm never reached.
        scroll_session_up(&mut session, &mut offset, VIEWPORT, usize::MAX);
        assert_eq!(offset, history);
        assert_eq!(session.vterm.scrollback_offset(), history);

        // scroll_to_top lands on the same row, by the same measure.
        scroll_session_to_bottom(&mut session, &mut offset);
        scroll_session_to_top(&mut session, &mut offset, VIEWPORT);
        assert_eq!(offset, history);

        // Scrolling down past the bottom clamps at 0.
        scroll_session_down(&mut session, &mut offset, usize::MAX);
        assert_eq!(offset, 0);

        scroll_session_up(&mut session, &mut offset, VIEWPORT, 3);
        scroll_session_to_bottom(&mut session, &mut offset);
        assert_eq!(offset, 0);
        assert_eq!(session.vterm.scrollback_offset(), 0);
    }
}
