//! Shared session scrolling helpers
//!
//! Centralizes keyboard-driven scroll behavior so Session mode and
//! Session view (normal mode) stay consistent.

use crate::app::App;
use crate::session::{SessionId, SessionType};
use crate::tui::frame::{FrameConfig, FrameLayout};

fn viewport_height(app: &App) -> usize {
    let terminal_size = app.tui.size().unwrap_or_default();
    let frame_config = FrameConfig::default();
    let layout = FrameLayout::calculate(terminal_size, &frame_config);
    layout.content.height as usize
}

/// Scroll up by one viewport page.
pub fn scroll_page_up(app: &mut App, session_id: SessionId) {
    let viewport_height = viewport_height(app);
    if let Some(session) = app.sessions.get_mut(session_id) {
        if session.info.session_type == SessionType::OpenAICodex {
            let requested = session
                .vterm
                .scrollback_offset()
                .saturating_add(viewport_height);
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
                session.fallback_scroll_up_with_viewport(viewport_height, viewport_height);
                app.state.session_scroll_offset = session.fallback_scroll_offset();
            }
        } else {
            let max_scroll = session.vterm.max_scrollback();
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

/// Scroll down by one viewport page.
pub fn scroll_page_down(app: &mut App, session_id: SessionId) {
    let viewport_height = viewport_height(app);
    if let Some(session) = app.sessions.get_mut(session_id) {
        if session.info.session_type == SessionType::OpenAICodex {
            let current_vterm = session.vterm.scrollback_offset();
            if current_vterm > 0 {
                let requested = current_vterm.saturating_sub(viewport_height);
                session.vterm.set_scrollback(requested);
                let vterm_offset = session.vterm.scrollback_offset();
                if vterm_offset == 0 {
                    session.fallback_scroll_to_bottom();
                }
                app.state.session_scroll_offset = vterm_offset;
            } else {
                session.fallback_scroll_down(viewport_height);
                app.state.session_scroll_offset = session.fallback_scroll_offset();
            }
        } else {
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

/// Reset app-level scroll when changing active session.
pub fn reset_for_session_switch(app: &mut App, session_id: SessionId) {
    app.state.session_scroll_offset = 0;
    if let Some(session) = app.sessions.get_mut(session_id) {
        if session.info.session_type == SessionType::OpenAICodex {
            session.fallback_scroll_to_bottom();
        }
    }
}
