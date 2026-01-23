//! Session mode input handling
//!
//! Handles keyboard input when in session mode (PTY forwarding).

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{App, InputMode};
use crate::tui::frame::{FrameConfig, FrameLayout};

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
            return Ok(());
        }
        KeyCode::PageDown if key.kind == KeyEventKind::Press => {
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
            return Ok(());
        }
        KeyCode::Home
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && key.kind == KeyEventKind::Press =>
        {
            // Ctrl+Home: scroll to top
            if let Some(session_id) = app.state.active_session {
                if let Some(session) = app.sessions.get_mut(session_id) {
                    let max_scroll = session.vterm.max_scrollback();
                    app.state.session_scroll_offset = max_scroll;
                    session
                        .vterm
                        .set_scrollback(app.state.session_scroll_offset);
                }
            }
            return Ok(());
        }
        KeyCode::End
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && key.kind == KeyEventKind::Press =>
        {
            // Ctrl+End: scroll to bottom (live view)
            if let Some(session_id) = app.state.active_session {
                if let Some(session) = app.sessions.get_mut(session_id) {
                    app.state.session_scroll_offset = 0;
                    session.vterm.scroll_to_bottom();
                }
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
        app.state.session_scroll_offset = 0;
        if let Some(session_id) = app.state.active_session {
            if let Some(session) = app.sessions.get_mut(session_id) {
                session.vterm.scroll_to_bottom();
            }
        }
    }

    // Send key to active session
    if let Some(session_id) = app.state.active_session {
        if let Some(session) = app.sessions.get_mut(session_id) {
            session.send_key(key)?;
        }
    }
    Ok(())
}

/// Handle Esc key in session mode
fn handle_session_mode_esc(app: &mut App, key: KeyEvent) -> Result<()> {
    // Only handle key press events
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    if key.modifiers.contains(KeyModifiers::SHIFT) {
        // Shift+Escape: forward Escape to Claude Code
        forward_esc_to_pty(app)?;
    } else {
        // Plain Escape: deactivate session mode (switch to Normal), stay in SessionView
        app.state.input_mode = InputMode::Normal;
        // Disable mouse capture so user can select and copy text
        app.tui.disable_mouse_capture();
    }
    Ok(())
}

/// Forward an Esc key press to the active session's PTY
fn forward_esc_to_pty(app: &mut App) -> Result<()> {
    if let Some(session_id) = app.state.active_session {
        if let Some(session) = app.sessions.get_mut(session_id) {
            let esc_key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
            session.send_key(esc_key)?;
        }
    }
    Ok(())
}
