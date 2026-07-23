//! Session mode input handling
//!
//! Handles keyboard input when in session mode (PTY forwarding).

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{App, InputMode};
use crate::input::session_scroll;
use crate::session::{SessionId, SessionManager};

/// Handle key in session mode (keys go to PTY)
pub fn handle_session_mode_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Handle Esc key
    if key.code == KeyCode::Esc {
        return handle_session_mode_esc(app, key);
    }

    // Intercept scroll keys - don't forward to PTY
    // Only handle Press events for scroll keys (not repeat) to prevent rapid scrolling
    match key.code {
        KeyCode::PageUp if key.kind == KeyEventKind::Press => {
            if let Some(session_id) = app.state.active_session {
                session_scroll::scroll_page_up(app, session_id);
            }
            return Ok(());
        }
        KeyCode::PageDown if key.kind == KeyEventKind::Press => {
            if let Some(session_id) = app.state.active_session {
                session_scroll::scroll_page_down(app, session_id);
            }
            return Ok(());
        }
        KeyCode::Home
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.kind == KeyEventKind::Press =>
        {
            // Ctrl+Home: scroll to top
            if let Some(session_id) = app.state.active_session {
                session_scroll::scroll_to_top(app, session_id);
            }
            return Ok(());
        }
        KeyCode::End
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.kind == KeyEventKind::Press =>
        {
            // Ctrl+End: scroll to bottom (live view)
            if let Some(session_id) = app.state.active_session {
                session_scroll::scroll_to_bottom(app, session_id);
            }
            return Ok(());
        }
        _ => {}
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
        if forward_key_to_session(&mut app.sessions, session_id, key)? {
            app.clear_title_notification();
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
    /// Plain Esc: switch to Normal mode, staying in the session view
    LeaveSessionMode,
}

/// Classify an Esc key event in session mode
fn esc_intent(key: &KeyEvent) -> EscIntent {
    if key.kind != KeyEventKind::Press {
        EscIntent::Ignore
    } else if key.modifiers.contains(KeyModifiers::SHIFT) {
        EscIntent::ForwardToPty
    } else {
        EscIntent::LeaveSessionMode
    }
}

/// Handle Esc key in session mode
fn handle_session_mode_esc(app: &mut App, key: KeyEvent) -> Result<()> {
    match esc_intent(&key) {
        EscIntent::Ignore => {}
        EscIntent::ForwardToPty => forward_esc_to_pty(app)?,
        EscIntent::LeaveSessionMode => {
            app.state.input_mode = InputMode::Normal;
            // Disable mouse capture so user can select and copy text
            app.tui.disable_mouse_capture();
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
    fn test_esc_intent_plain_press_leaves_session_mode() {
        let key = press(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(esc_intent(&key), EscIntent::LeaveSessionMode);
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
