//! Branch detail view
//!
//! Shows sessions for a specific branch.

use ratatui::prelude::*;

use crate::app::AppState;
use crate::project::{BranchId, ProjectId, ProjectStore};
use crate::session::SessionManager;

use super::placeholder::render_placeholder;

/// Render the branch detail view showing sessions
/// TODO: Ticket 26 will implement this fully
pub fn render_branch_detail(
    frame: &mut Frame,
    area: Rect,
    _state: &AppState,
    _project_id: ProjectId,
    _branch_id: BranchId,
    _project_store: &ProjectStore,
    _sessions: &SessionManager,
) {
    // Placeholder until Ticket 26
    render_placeholder(frame, area, "Branch Detail (Coming Soon)");
}
