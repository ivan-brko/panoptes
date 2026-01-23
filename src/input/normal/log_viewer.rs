//! Log viewer input handler
//!
//! Handles keyboard input in the log viewer view.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{App, View};

/// Handle key in log viewer (normal mode)
pub fn handle_log_viewer_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Only process key press events (not release/repeat)
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    // Handle focus timer shortcuts (t, T, Ctrl+t)
    if app.handle_focus_timer_shortcut(key) {
        return Ok(());
    }

    let entry_count = app.log_buffer.len();

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            // Go back to projects overview
            app.state.view = View::ProjectsOverview;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            // Scroll down (disable auto-scroll)
            app.state.log_viewer_auto_scroll = false;
            if app.state.log_viewer_scroll < entry_count.saturating_sub(1) {
                app.state.log_viewer_scroll += 1;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            // Scroll up (disable auto-scroll)
            app.state.log_viewer_auto_scroll = false;
            app.state.log_viewer_scroll = app.state.log_viewer_scroll.saturating_sub(1);
        }
        KeyCode::Char('g') => {
            // Jump to top (disable auto-scroll)
            app.state.log_viewer_auto_scroll = false;
            app.state.log_viewer_scroll = 0;
        }
        KeyCode::Char('G') => {
            // Jump to bottom and enable auto-scroll
            app.state.log_viewer_auto_scroll = true;
            app.state.log_viewer_scroll = entry_count.saturating_sub(1);
        }
        KeyCode::PageDown => {
            // Page down (disable auto-scroll)
            app.state.log_viewer_auto_scroll = false;
            app.state.log_viewer_scroll =
                (app.state.log_viewer_scroll + 20).min(entry_count.saturating_sub(1));
        }
        KeyCode::PageUp => {
            // Page up (disable auto-scroll)
            app.state.log_viewer_auto_scroll = false;
            app.state.log_viewer_scroll = app.state.log_viewer_scroll.saturating_sub(20);
        }
        _ => {}
    }
    Ok(())
}
