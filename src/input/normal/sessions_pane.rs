//! Pane 2 input: the flat session list
//!
//! Nothing to drill into, so `Esc` is a no-op here by construction.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{cycle_next, cycle_prev, App, InputMode};

/// Handle a normal-mode key while pane 2 has focus
pub fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    let session_count = app.sessions.len();

    match key.code {
        KeyCode::Esc => {
            // A flat list has no level to pop
        }
        KeyCode::Down => {
            app.state.sessions_pane_index =
                cycle_next(app.state.sessions_pane_index, session_count);
        }
        KeyCode::Up => {
            app.state.sessions_pane_index =
                cycle_prev(app.state.sessions_pane_index, session_count);
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            // The list is numbered, so digits select by number (0 means 10)
            if let Some(num) = c.to_digit(10) {
                let target = if num == 0 { 9 } else { (num as usize) - 1 };
                if target < session_count {
                    app.state.sessions_pane_index = target;
                }
            }
        }
        KeyCode::Enter => {
            if let Some(session) = app.sessions.get_by_index(app.state.sessions_pane_index) {
                let session_id = session.info.id;
                app.activate_session(session_id)?;
            }
        }
        KeyCode::Char('d') => {
            // Ask first, like every other delete in the app
            if let Some(session) = app.sessions.get_by_index(app.state.sessions_pane_index) {
                app.state.pending_delete_session = Some(session.info.id);
                app.state.input_mode = InputMode::ConfirmingSessionDelete;
            }
        }
        _ => {}
    }
    Ok(())
}
