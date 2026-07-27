//! Session mode input handling
//!
//! The session view has one mode, and this is it: every key reaches the agent
//! except `Esc`, which leaves. There is no second mode to be in and no
//! keystroke Panoptes takes for itself along the way - no scroll keys, no
//! session-switching digits, no custom shortcuts. Those all still exist, in
//! the panes, one `Esc` away.
//!
//! Scrolling is therefore the wheel's job. For a session whose child asked
//! for the mouse, the wheel goes to the child and the child's own scrollback
//! answers - which is what a real terminal does with an agent that draws its
//! own screen.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::App;
use crate::input::session_scroll;
use crate::session::{SessionId, SessionManager};

/// Handle key in session mode (keys go to PTY)
pub fn handle_session_mode_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Handle Esc key
    if key.code == KeyCode::Esc {
        return handle_session_mode_esc(app, key);
    }

    // Reset scroll to live view when typing (only on Press/Repeat, not Release)
    if key.kind == KeyEventKind::Release {
        return Ok(());
    }
    if app.state.session_scroll_offset > 0 {
        if let Some(session_id) = app.state.active_session {
            session_scroll::scroll_to_bottom(app, session_id);
        }
    }

    // Send key to active session
    if let Some(session_id) = app.state.active_session {
        // A suspended session has no process to write to. Wake it first, so the
        // keystroke reaches the relaunched agent instead of vanishing into a
        // dead PTY.
        if app.sessions.is_suspended(session_id) && !app.wake_session(session_id)? {
            return Ok(());
        }
        // Neither has an exited one, and there is nothing to wake: the agent
        // is gone and its scrollback is all that is left. Typing at it is a
        // reasonable thing for a user to try - the screen still looks like a
        // session - so it has to be a no-op rather than an error.
        if !app
            .sessions
            .get(session_id)
            .is_some_and(|session| session.info.state.has_process())
        {
            return Ok(());
        }
        match forward_key_to_session(&mut app.sessions, session_id, key) {
            Ok(true) => app.clear_title_notification(),
            Ok(false) => {}
            // A failed write means the process died between the check above
            // and here. Never fatal: this error used to propagate out of the
            // event loop and take Panoptes down with the session.
            Err(e) => {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "Keystroke could not be delivered; the session's process is gone"
                );
                app.state.error_message = Some("This session's process has exited".to_string());
            }
        }
    }
    Ok(())
}

/// Send a key to a session's PTY and acknowledge its attention flag
///
/// The user is actively interacting with this session, so a pending attention
/// flag is cleared. Returns `true` when a flag was cleared — the caller should
/// then also clear the terminal-title notification.
fn forward_key_to_session(
    sessions: &mut SessionManager,
    session_id: SessionId,
    key: KeyEvent,
) -> Result<bool> {
    if let Some(session) = sessions.get_mut(session_id) {
        session.send_key(key)?;
    }
    if sessions
        .get(session_id)
        .is_some_and(|s| s.info.attention.is_some())
    {
        sessions.acknowledge_attention(session_id);
        return Ok(true);
    }
    Ok(false)
}

/// What an Esc key event should do in session mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EscIntent {
    /// Not a key press (repeat/release): do nothing
    Ignore,
    /// Shift+Esc: forward Esc to the agent in the PTY
    ForwardToPty,
    /// Plain Esc: leave the session view for the pane it was opened from
    LeaveSessionView,
}

/// Classify an Esc key event in session mode
fn esc_intent(key: &KeyEvent) -> EscIntent {
    if key.kind != KeyEventKind::Press {
        EscIntent::Ignore
    } else if key.modifiers.contains(KeyModifiers::SHIFT) {
        EscIntent::ForwardToPty
    } else {
        EscIntent::LeaveSessionView
    }
}

/// Handle Esc key in session mode
///
/// One press, all the way out. `Shift+Esc` is what an agent that wants a
/// literal `Esc` - `vim` leaving insert mode, Claude Code interrupting a turn
/// - gets instead, and is now the only way to send one.
fn handle_session_mode_esc(app: &mut App, key: KeyEvent) -> Result<()> {
    match esc_intent(&key) {
        EscIntent::Ignore => {}
        EscIntent::ForwardToPty => forward_esc_to_pty(app)?,
        EscIntent::LeaveSessionView => {
            // Clears the selection on the way out, which is also what releases
            // a drag's output hold: the hold is re-derived from the live
            // selection every tick, so `Esc` mid-drag cannot strand a session
            // with its output held and no release coming.
            app.state.return_from_session(&app.sessions);
        }
    }
    Ok(())
}

/// Forward an Esc key press to the active session's PTY
fn forward_esc_to_pty(app: &mut App) -> Result<()> {
    if let Some(session_id) = app.state.active_session {
        // Same reason as the ordinary key path: a suspended session has no
        // process, so the byte would disappear into an orphaned PTY master
        // and Shift+Esc would be the one keystroke that fails to wake it.
        if app.sessions.is_suspended(session_id) && !app.wake_session(session_id)? {
            return Ok(());
        }
        if let Some(session) = app.sessions.get_mut(session_id) {
            let esc_key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
            session.send_key(esc_key)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::session::{AttentionReason, SessionStore};
    use tempfile::TempDir;
    use uuid::Uuid;

    /// Build a manager backed by a temp store (never the real ~/.panoptes)
    fn test_manager(temp_dir: &TempDir) -> SessionManager {
        let config = Config {
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            ..Config::default()
        };
        SessionManager::with_store(
            config,
            SessionStore::with_path(temp_dir.path().join("sessions.json")),
        )
    }

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn test_esc_intent_plain_press_leaves_the_session_view() {
        let key = press(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(esc_intent(&key), EscIntent::LeaveSessionView);
    }

    /// Every key that is not `Esc` reaches the agent's PTY
    ///
    /// Each of these used to be taken by Panoptes somewhere in the session
    /// view - the scroll keys in this handler, the digits and shortcut
    /// characters in the detached mode that no longer exists. Writing them
    /// through `forward_key_to_session` is what one mode means: the handler
    /// above has no branch that can swallow them.
    #[test]
    fn test_every_key_but_esc_goes_to_the_pty() {
        let temp_dir = TempDir::new().unwrap();
        let mut sessions = test_manager(&temp_dir);
        let session_id = sessions
            .insert_test_session("forward", Uuid::new_v4(), Uuid::new_v4())
            .unwrap();

        for key in [
            press(KeyCode::PageUp, KeyModifiers::NONE),
            press(KeyCode::PageDown, KeyModifiers::NONE),
            press(KeyCode::Home, KeyModifiers::CONTROL),
            press(KeyCode::End, KeyModifiers::CONTROL),
            press(KeyCode::Up, KeyModifiers::NONE),
            press(KeyCode::Down, KeyModifiers::NONE),
            press(KeyCode::Enter, KeyModifiers::NONE),
            press(KeyCode::Char('3'), KeyModifiers::NONE),
            press(KeyCode::Char('q'), KeyModifiers::NONE),
        ] {
            assert_ne!(key.code, KeyCode::Esc);
            forward_key_to_session(&mut sessions, session_id, key)
                .unwrap_or_else(|e| panic!("{:?} should reach the PTY: {e}", key.code));
        }
    }

    #[test]
    fn test_esc_intent_shift_press_forwards_to_pty() {
        let key = press(KeyCode::Esc, KeyModifiers::SHIFT);
        assert_eq!(esc_intent(&key), EscIntent::ForwardToPty);
    }

    #[test]
    fn test_esc_intent_non_press_events_are_ignored() {
        for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
            let mut key = press(KeyCode::Esc, KeyModifiers::NONE);
            key.kind = kind;
            assert_eq!(esc_intent(&key), EscIntent::Ignore, "for {kind:?}");
        }
    }

    #[test]
    fn test_forward_key_writes_to_session_pty() {
        let temp_dir = TempDir::new().unwrap();
        let mut sessions = test_manager(&temp_dir);
        let session_id = sessions
            .insert_test_session("fwd", Uuid::new_v4(), Uuid::new_v4())
            .unwrap();

        // The sleep-backed PTY absorbs the write; no attention flag was set
        let key = press(KeyCode::Char('a'), KeyModifiers::NONE);
        let cleared = forward_key_to_session(&mut sessions, session_id, key).unwrap();
        assert!(!cleared);
    }

    #[test]
    fn test_forward_key_acknowledges_attention() {
        let temp_dir = TempDir::new().unwrap();
        let mut sessions = test_manager(&temp_dir);
        let session_id = sessions
            .insert_test_session("attn", Uuid::new_v4(), Uuid::new_v4())
            .unwrap();
        sessions.get_mut(session_id).unwrap().info.attention = Some(AttentionReason::TurnComplete);

        let key = press(KeyCode::Char('a'), KeyModifiers::NONE);
        let cleared = forward_key_to_session(&mut sessions, session_id, key).unwrap();

        assert!(
            cleared,
            "caller must be told to clear the title notification"
        );
        assert!(sessions.get(session_id).unwrap().info.attention.is_none());
    }

    #[test]
    fn test_forward_key_to_unknown_session_is_noop() {
        let temp_dir = TempDir::new().unwrap();
        let mut sessions = test_manager(&temp_dir);

        let key = press(KeyCode::Char('a'), KeyModifiers::NONE);
        let cleared = forward_key_to_session(&mut sessions, Uuid::new_v4(), key).unwrap();
        assert!(!cleared);
    }
}
