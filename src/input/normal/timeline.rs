//! Activity timeline input handler
//!
//! Handles keyboard input in the activity timeline view.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::App;
use crate::session::SessionManager;

/// Handle key in activity timeline (normal mode)
pub fn handle_timeline_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Only process key press events (not release/repeat)
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    // Handle focus timer shortcuts (t, T, Ctrl+t)
    if app.handle_focus_timer_shortcut(key) {
        return Ok(());
    }

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.state.navigate_back();
        }
        KeyCode::Down => {
            app.state.select_next(app.sessions.len());
        }
        KeyCode::Up => {
            app.state.select_prev(app.sessions.len());
        }
        KeyCode::Enter => {
            let index = app.state.current_selected_index();
            if let Some(session) = app.sessions.get_by_index(index) {
                let session_id = session.info.id;
                app.state.navigate_to_session(session_id);
                app.tui.enable_mouse_capture();
                app.sessions.acknowledge_attention(session_id);
                if app.config.notification_method == "title" {
                    SessionManager::reset_terminal_title();
                }
                app.resize_active_session_pty()?;
            }
        }
        _ => {}
    }
    Ok(())
}
