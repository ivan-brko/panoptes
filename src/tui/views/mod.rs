//! View rendering modules
//!
//! Each view in the application has its own module for rendering logic.

use crate::config::Config;
use crate::focus_timing::FocusTimer;
use crate::session::SessionManager;

mod branch_detail;
mod claude_configs;
mod confirm;
mod focus_stats;
mod logs;
mod notifications;
mod project_detail;
mod projects;
mod session;
mod timeline;

pub use branch_detail::render_branch_detail;
pub use claude_configs::{
    render_claude_configs, render_config_delete_dialog, render_config_name_input_dialog,
    render_config_path_input_dialog, render_config_selector,
};
pub use confirm::{
    render_confirm_dialog, render_loading_indicator, render_quit_confirm_dialog,
    ConfirmDialogConfig,
};
pub use focus_stats::{
    render_focus_session_delete_dialog, render_focus_session_detail_dialog, render_focus_stats,
    render_timer_input_dialog,
};
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

/// Format focus timer hint for footer. Shows different text based on timer state.
pub fn format_focus_timer_hint(timer_running: bool) -> &'static str {
    if timer_running {
        "Ctrl+t: stop timer | T: stats"
    } else {
        "t: timer | T: stats"
    }
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

/// Format header text with optional right-aligned timer display.
/// Returns a string with the breadcrumb on the left and timer on the right.
pub fn format_header_with_timer(
    breadcrumb_text: &str,
    timer: Option<&FocusTimer>,
    width: u16,
) -> String {
    let timer_text = match timer {
        Some(t) if t.is_running() => format!("⏱ {}", t.format_remaining()),
        _ => String::new(),
    };

    if timer_text.is_empty() {
        return breadcrumb_text.to_string();
    }

    // Calculate padding needed (width - breadcrumb_len - timer_len - 2 for borders)
    let available = width.saturating_sub(2) as usize;
    let breadcrumb_len = breadcrumb_text.chars().count();
    let timer_len = timer_text.chars().count();
    let padding = available.saturating_sub(breadcrumb_len + timer_len);

    format!("{}{}{}", breadcrumb_text, " ".repeat(padding), timer_text)
}
