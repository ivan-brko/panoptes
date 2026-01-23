//! Focus stats input handler
//!
//! Handles keyboard input in the focus stats view.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{App, InputMode};

/// Handle key in focus stats view (normal mode)
pub fn handle_focus_stats_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Only process key press events (not release/repeat)
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    // Handle focus timer shortcuts (t, T, Ctrl+t)
    if app.handle_focus_timer_shortcut(key) {
        return Ok(());
    }

    let session_count = app.state.focus_sessions.len();

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.state.navigate_back();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.state.select_next(session_count);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.state.select_prev(session_count);
        }
        KeyCode::Enter => {
            // Show session details
            if !app.state.focus_sessions.is_empty() {
                // Get selected session from sorted list (same order as rendering)
                let mut sorted: Vec<_> = app.state.focus_sessions.iter().collect();
                sorted.sort_by(|a, b| b.completed_at.cmp(&a.completed_at));
                if let Some(session) = sorted.get(app.state.focus_stats_selected_index) {
                    app.state.viewing_focus_session = Some((*session).clone());
                    app.state.input_mode = InputMode::ViewingFocusSessionDetail;
                }
            }
        }
        KeyCode::Char('d') => {
            // Prompt for confirmation before deleting focus session
            if !app.state.focus_sessions.is_empty() {
                // Get selected session from sorted list (same order as rendering)
                let mut sorted: Vec<_> = app.state.focus_sessions.iter().collect();
                sorted.sort_by(|a, b| b.completed_at.cmp(&a.completed_at));
                if let Some(session) = sorted.get(app.state.focus_stats_selected_index) {
                    app.state.pending_delete_focus_session = Some(session.id);
                    app.state.input_mode = InputMode::ConfirmingFocusSessionDelete;
                }
            }
        }
        _ => {}
    }
    Ok(())
}
