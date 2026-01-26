//! Session view input handler (normal mode)
//!
//! Handles keyboard input in session view when NOT in session mode.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{App, InputMode};
use crate::session::SessionManager;
use crate::tui::frame::{FrameConfig, FrameLayout};

/// Handle key in session view (normal mode)
pub fn handle_session_view_normal_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Only process key press events (not release/repeat)
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    // Handle focus timer shortcuts (t, T, Ctrl+t) - only in Normal mode
    if app.handle_focus_timer_shortcut(key) {
        return Ok(());
    }

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
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
        KeyCode::PageUp => {
            // Scroll up in session output (toward older content)
            if let Some(session_id) = app.state.active_session {
                if let Some(session) = app.sessions.get_mut(session_id) {
                    // Calculate viewport height for scroll amount
                    let terminal_size = app.tui.size().unwrap_or_default();
                    let frame_config = FrameConfig::default();
                    let layout = FrameLayout::calculate(terminal_size, &frame_config);
                    let viewport_height = layout.content.height as usize;
                    let max_scroll = session.vterm.max_scrollback();

                    // Update app-level scroll offset with constraints
                    app.state.session_scroll_offset = app
                        .state
                        .session_scroll_offset
                        .saturating_add(viewport_height)
                        .min(max_scroll);
                    session
                        .vterm
                        .set_scrollback(app.state.session_scroll_offset);
                }
            }
        }
        KeyCode::PageDown => {
            // Scroll down in session output (toward newer content)
            if let Some(session_id) = app.state.active_session {
                if let Some(session) = app.sessions.get_mut(session_id) {
                    // Calculate viewport height for scroll amount
                    let terminal_size = app.tui.size().unwrap_or_default();
                    let frame_config = FrameConfig::default();
                    let layout = FrameLayout::calculate(terminal_size, &frame_config);
                    let viewport_height = layout.content.height as usize;

                    // Update app-level scroll offset
                    app.state.session_scroll_offset = app
                        .state
                        .session_scroll_offset
                        .saturating_sub(viewport_height);
                    session
                        .vterm
                        .set_scrollback(app.state.session_scroll_offset);
                }
            }
        }
        KeyCode::Home => {
            // Scroll to top (oldest content)
            if let Some(session_id) = app.state.active_session {
                if let Some(session) = app.sessions.get_mut(session_id) {
                    let max_scroll = session.vterm.max_scrollback();
                    app.state.session_scroll_offset = max_scroll;
                    session
                        .vterm
                        .set_scrollback(app.state.session_scroll_offset);
                }
            }
        }
        KeyCode::End => {
            // Scroll to bottom (live view)
            if let Some(session_id) = app.state.active_session {
                if let Some(session) = app.sessions.get_mut(session_id) {
                    app.state.session_scroll_offset = 0;
                    session.vterm.scroll_to_bottom();
                }
            }
        }
        KeyCode::Tab => {
            // Switch to next session
            let count = app.sessions.len();
            if count > 0 {
                // Clamp current index to valid range before incrementing
                let current = app
                    .state
                    .selected_timeline_index
                    .min(count.saturating_sub(1));
                // Use the timeline index for cycling through all sessions
                app.state.selected_timeline_index = (current + 1) % count;
                if let Some(session) = app.sessions.get_by_index(app.state.selected_timeline_index)
                {
                    let session_id = session.info.id;
                    app.state.active_session = Some(session_id);
                    // Reset scroll offset when switching sessions
                    app.state.session_scroll_offset = 0;
                    app.sessions.acknowledge_attention(session_id);
                    if app.config.notification_method == "title" {
                        SessionManager::reset_terminal_title();
                    }
                    app.resize_active_session_pty()?;
                }
            } else {
                // No sessions - reset index
                app.state.selected_timeline_index = 0;
            }
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            // Jump to session by number (1-indexed, 0 means session 10)
            if let Some(num) = c.to_digit(10) {
                let target_index = if num == 0 { 9 } else { (num as usize) - 1 };
                // Use checked access for safety
                if let Some(session) = app.sessions.get_by_index(target_index) {
                    let session_id = session.info.id;
                    app.state.selected_timeline_index = target_index;
                    app.state.active_session = Some(session_id);
                    // Reset scroll offset when switching sessions
                    app.state.session_scroll_offset = 0;
                    app.sessions.acknowledge_attention(session_id);
                    if app.config.notification_method == "title" {
                        SessionManager::reset_terminal_title();
                    }
                    app.resize_active_session_pty()?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}
