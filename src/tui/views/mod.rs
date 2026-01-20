//! View rendering modules
//!
//! Each view in the application has its own module for rendering logic.

use crate::config::Config;
use crate::session::SessionManager;

mod branch_detail;
mod confirm;
mod placeholder;
mod project_detail;
mod projects;
mod session;
mod timeline;

pub use branch_detail::render_branch_detail;
pub use confirm::{render_confirm_dialog, ConfirmDialogConfig};
pub use placeholder::render_placeholder;
pub use project_detail::{render_project_delete_confirmation, render_project_detail};
pub use projects::render_projects_overview;
pub use session::render_session_view;
pub use timeline::render_timeline;

// Re-export for convenience
pub use placeholder::render_placeholder as placeholder;

/// Format the attention hint for the footer showing the next session needing attention.
/// Returns None if no sessions need attention.
pub fn format_attention_hint(sessions: &SessionManager, config: &Config) -> Option<String> {
    let attention = sessions.sessions_needing_attention(config.idle_threshold_secs);
    attention
        .first()
        .map(|s| format!("Space: → {}", s.info.name))
}
