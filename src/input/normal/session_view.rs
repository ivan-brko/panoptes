//! Session view input handler (normal mode)
//!
//! Handles keyboard input in session view when NOT in session mode.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{App, InputMode, View};
use crate::input::session_scroll;
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
                    session_scroll::reset_for_session_switch(app, session_id);
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
                    session_scroll::reset_for_session_switch(app, session_id);
                    app.sessions.acknowledge_attention(session_id);
                    if app.config.notification_method == "title" {
                        SessionManager::reset_terminal_title();
                    }
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

                        // Generate a session name from the shortcut
                        let session_name = shortcut.short_display_name();

                        // Get terminal size
                        let terminal_size = app.tui.size().unwrap_or_default();
                        let frame_config = FrameConfig::default();
                        let layout = FrameLayout::calculate(terminal_size, &frame_config);
                        let rows = layout.content.height as usize;
                        let cols = layout.content.width as usize;

                        // Create shell session with command
                        match app.sessions.create_shell_session_with_command(
                            session_name,
                            working_dir,
                            project_id,
                            branch_id,
                            shortcut.command.clone(),
                            rows,
                            cols,
                        ) {
                            Ok(new_session_id) => {
                                // Navigate to the new session
                                app.state.active_session = Some(new_session_id);
                                session_scroll::reset_for_session_switch(app, new_session_id);
                                app.state.input_mode = InputMode::Session;
                                app.state.view = View::SessionView;
                            }
                            Err(e) => {
                                tracing::error!("Failed to create shell session: {}", e);
                                app.state.error_message =
                                    Some(format!("Failed to create session: {}", e));
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}
