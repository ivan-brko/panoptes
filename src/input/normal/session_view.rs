//! Session view input handler (normal mode)
//!
//! Handles keyboard input in session view when NOT in session mode.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{App, InputMode};
use crate::input::session_scroll;

/// Handle key in session view (normal mode)
pub fn handle_session_view_normal_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Only process key press events (not release/repeat)
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    match key.code {
        KeyCode::Esc => {
            // Go back to the session's branch detail view
            app.state.return_from_session(&app.sessions);
            // Re-enable mouse capture when leaving session view
            app.tui.enable_mouse_capture();
        }
        KeyCode::Enter => {
            // Re-activate session mode (send keys to PTY)
            app.state.input_mode = InputMode::Session;
            // Re-enable mouse capture for scroll wheel
            app.tui.enable_mouse_capture();
        }
        KeyCode::Up => {
            // Scroll up a few lines (toward older content)
            if let Some(session_id) = app.state.active_session {
                session_scroll::scroll_lines_up(app, session_id);
            }
        }
        KeyCode::Down => {
            // Scroll down a few lines (toward newer content)
            if let Some(session_id) = app.state.active_session {
                session_scroll::scroll_lines_down(app, session_id);
            }
        }
        KeyCode::PageUp => {
            // Scroll up in session output (toward older content)
            if let Some(session_id) = app.state.active_session {
                session_scroll::scroll_page_up(app, session_id);
            }
        }
        KeyCode::PageDown => {
            // Scroll down in session output (toward newer content)
            if let Some(session_id) = app.state.active_session {
                session_scroll::scroll_page_down(app, session_id);
            }
        }
        KeyCode::Home => {
            // Scroll to top (oldest content)
            if let Some(session_id) = app.state.active_session {
                session_scroll::scroll_to_top(app, session_id);
            }
        }
        KeyCode::End => {
            // Scroll to bottom (live view)
            if let Some(session_id) = app.state.active_session {
                session_scroll::scroll_to_bottom(app, session_id);
            }
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            // Jump to session by number (1-indexed, 0 means session 10)
            if let Some(num) = c.to_digit(10) {
                let target_index = if num == 0 { 9 } else { (num as usize) - 1 };
                // Use checked access for safety
                if let Some(session) = app.sessions.get_by_index(target_index) {
                    let session_id = session.info.id;
                    app.state.active_session = Some(session_id);
                    // Reset scroll offset when switching sessions
                    session_scroll::reset_for_session_switch(app, session_id);
                    app.sessions.acknowledge_attention(session_id);
                    app.clear_title_notification();
                    app.resize_active_session_pty()?;
                }
            }
        }
        KeyCode::Char(c) => {
            // Check for custom shortcut trigger
            if let Some(shortcut) = app.config.get_shortcut(c).cloned() {
                // Get the current session's project/branch context
                if let Some(session_id) = app.state.active_session {
                    if let Some(session) = app.sessions.get(session_id) {
                        let project_id = session.info.project_id;
                        let branch_id = session.info.branch_id;
                        let working_dir = session.info.working_dir.clone();

                        if let Some(new_session_id) = super::launch_shortcut_session(
                            app,
                            &shortcut,
                            project_id,
                            branch_id,
                            working_dir,
                        ) {
                            // Swap the screen over to the new session, keeping
                            // the pane the original was opened from
                            app.state.active_session = Some(new_session_id);
                            session_scroll::reset_for_session_switch(app, new_session_id);
                            app.state.input_mode = InputMode::Session;
                        }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}
