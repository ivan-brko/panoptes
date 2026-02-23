//! Shared session scrolling helpers
//!
//! Centralizes keyboard-driven scroll behavior so Session mode and
//! Session view (normal mode) stay consistent.

use crate::app::App;
use crate::session::{SessionId, SessionType};
use crate::tui::frame::{FrameConfig, FrameLayout};

/// Number of lines to scroll per arrow key press.
const ARROW_SCROLL_STEP: usize = 3;

fn viewport_height(app: &App) -> usize {
    let terminal_size = app.tui.size().unwrap_or_default();
    let frame_config = FrameConfig::default();
    let layout = FrameLayout::calculate(terminal_size, &frame_config);
    layout.content.height as usize
}

/// Shared logic for scrolling up by a given number of lines.
fn scroll_up_by(app: &mut App, session_id: SessionId, amount: usize) {
    let viewport_height = viewport_height(app);
    if let Some(session) = app.sessions.get_mut(session_id) {
        if session.info.session_type == SessionType::OpenAICodex {
            let requested = session.vterm.scrollback_offset().saturating_add(amount);
            let current_vterm = session.vterm.scrollback_offset();
            session.vterm.set_scrollback(requested);
            let vterm_offset = session.vterm.scrollback_offset();
            let vterm_advanced = vterm_offset > current_vterm;
            if vterm_offset > 0 && vterm_advanced {
                session.fallback_scroll_to_bottom();
                app.state.session_scroll_offset = vterm_offset;
            } else {
                // vterm scrollback can stop advancing at a shallow offset.
                // Switch to fallback mode for continued upward scrolling.
                session.vterm.scroll_to_bottom();
                session.fallback_scroll_up_with_viewport(amount, viewport_height);
                app.state.session_scroll_offset = session.fallback_scroll_offset();
            }
        } else {
            let max_scroll = session.vterm.max_scrollback();
            app.state.session_scroll_offset = app
                .state
                .session_scroll_offset
                .saturating_add(amount)
                .min(max_scroll);
            session
                .vterm
                .set_scrollback(app.state.session_scroll_offset);
        }
    }
}

/// Shared logic for scrolling down by a given number of lines.
fn scroll_down_by(app: &mut App, session_id: SessionId, amount: usize) {
    if let Some(session) = app.sessions.get_mut(session_id) {
        if session.info.session_type == SessionType::OpenAICodex {
            let current_vterm = session.vterm.scrollback_offset();
            if current_vterm > 0 {
                let requested = current_vterm.saturating_sub(amount);
                session.vterm.set_scrollback(requested);
                let vterm_offset = session.vterm.scrollback_offset();
                if vterm_offset == 0 {
                    session.fallback_scroll_to_bottom();
                }
                app.state.session_scroll_offset = vterm_offset;
            } else {
                session.fallback_scroll_down(amount);
                app.state.session_scroll_offset = session.fallback_scroll_offset();
            }
        } else {
            app.state.session_scroll_offset =
                app.state.session_scroll_offset.saturating_sub(amount);
            session
                .vterm
                .set_scrollback(app.state.session_scroll_offset);
        }
    }
}

/// Scroll up by one viewport page.
pub fn scroll_page_up(app: &mut App, session_id: SessionId) {
    let height = viewport_height(app);
    scroll_up_by(app, session_id, height);
}

/// Scroll down by one viewport page.
pub fn scroll_page_down(app: &mut App, session_id: SessionId) {
    let height = viewport_height(app);
    scroll_down_by(app, session_id, height);
}

/// Scroll to oldest available output.
pub fn scroll_to_top(app: &mut App, session_id: SessionId) {
    let viewport_height = viewport_height(app);
    if let Some(session) = app.sessions.get_mut(session_id) {
        if session.info.session_type == SessionType::OpenAICodex {
            session.vterm.set_scrollback(usize::MAX);
            let vterm_offset = session.vterm.scrollback_offset();
            if vterm_offset > 0 {
                session.fallback_scroll_to_bottom();
                app.state.session_scroll_offset = vterm_offset;
            } else {
                session.fallback_scroll_to_top_with_viewport(viewport_height);
                app.state.session_scroll_offset = session.fallback_scroll_offset();
            }
        } else {
            let max_scroll = session.vterm.max_scrollback();
            app.state.session_scroll_offset = max_scroll;
            session
                .vterm
                .set_scrollback(app.state.session_scroll_offset);
        }
    }
}

/// Return to live output (bottom).
pub fn scroll_to_bottom(app: &mut App, session_id: SessionId) {
    if let Some(session) = app.sessions.get_mut(session_id) {
        app.state.session_scroll_offset = 0;
        session.vterm.scroll_to_bottom();
        if session.info.session_type == SessionType::OpenAICodex {
            session.fallback_scroll_to_bottom();
        }
    }
}

/// Scroll up by a few lines (arrow key).
pub fn scroll_lines_up(app: &mut App, session_id: SessionId) {
    scroll_up_by(app, session_id, ARROW_SCROLL_STEP);
}

/// Scroll down by a few lines (arrow key).
pub fn scroll_lines_down(app: &mut App, session_id: SessionId) {
    scroll_down_by(app, session_id, ARROW_SCROLL_STEP);
}

/// Reset app-level scroll when changing active session.
pub fn reset_for_session_switch(app: &mut App, session_id: SessionId) {
    app.state.session_scroll_offset = 0;
    if let Some(session) = app.sessions.get_mut(session_id) {
        if session.info.session_type == SessionType::OpenAICodex {
            session.fallback_scroll_to_bottom();
        }
    }
}
