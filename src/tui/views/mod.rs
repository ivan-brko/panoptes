//! View rendering modules
//!
//! Each view in the application has its own module for rendering logic.

use crate::config::Config;
use crate::session::SessionManager;

mod branch_detail;
mod confirm;
mod focus_stats;
mod logs;
mod notifications;
mod project_detail;
mod projects;
mod session;
mod timeline;

pub use branch_detail::render_branch_detail;
pub use confirm::{
    render_confirm_dialog, render_loading_indicator, render_quit_confirm_dialog,
    ConfirmDialogConfig,
};
pub use focus_stats::{render_focus_stats, render_timer_input_dialog};
pub use logs::render_log_viewer;
pub use notifications::{render_notification_badge, render_notifications};
pub use project_detail::{render_project_delete_confirmation, render_project_detail};
pub use projects::render_projects_overview;
pub use session::render_session_view;
pub use timeline::render_timeline;

/// Format the attention hint for the footer showing the next session needing attention.
/// Returns None if no sessions need attention.
pub fn format_attention_hint(sessions: &SessionManager, config: &Config) -> Option<String> {
    let attention = sessions.sessions_needing_attention(config.idle_threshold_secs);
    attention
        .first()
        .map(|s| format!("Space: → {}", s.info.name))
}

/// Breadcrumb navigation path segments
pub struct Breadcrumb {
    segments: Vec<String>,
}

impl Breadcrumb {
    /// Create a new breadcrumb with the root "Panoptes" segment
    pub fn new() -> Self {
        Self {
            segments: vec!["Panoptes".to_string()],
        }
    }

    /// Add a segment to the breadcrumb path
    pub fn push(mut self, segment: impl Into<String>) -> Self {
        self.segments.push(segment.into());
        self
    }

    /// Format the breadcrumb as a display string with " > " separators
    pub fn display(&self) -> String {
        self.segments.join(" > ")
    }

    /// Format the breadcrumb with an optional suffix (e.g., status info)
    pub fn display_with_suffix(&self, suffix: &str) -> String {
        if suffix.is_empty() {
            self.display()
        } else {
            format!("{} {}", self.display(), suffix)
        }
    }
}

impl Default for Breadcrumb {
    fn default() -> Self {
        Self::new()
    }
}
