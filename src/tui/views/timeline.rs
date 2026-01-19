//! Activity timeline view
//!
//! Shows all sessions sorted by recent activity.

use ratatui::prelude::*;

use crate::app::AppState;
use crate::session::SessionManager;

use super::placeholder::render_placeholder;

/// Render the activity timeline view
/// TODO: Ticket 27 will implement this fully
pub fn render_timeline(
    frame: &mut Frame,
    area: Rect,
    _state: &AppState,
    _sessions: &SessionManager,
) {
    // Placeholder until Ticket 27
    render_placeholder(frame, area, "Activity Timeline (Coming Soon)");
}
