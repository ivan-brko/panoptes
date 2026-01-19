//! Project detail view
//!
//! Shows branches for a specific project.

use ratatui::prelude::*;

use crate::app::AppState;
use crate::project::{ProjectId, ProjectStore};
use crate::session::SessionManager;

use super::placeholder::render_placeholder;

/// Render the project detail view showing branches
/// TODO: Ticket 25 will implement this fully
pub fn render_project_detail(
    frame: &mut Frame,
    area: Rect,
    _state: &AppState,
    _project_id: ProjectId,
    _project_store: &ProjectStore,
    _sessions: &SessionManager,
) {
    // Placeholder until Ticket 25
    render_placeholder(frame, area, "Project Detail (Coming Soon)");
}
