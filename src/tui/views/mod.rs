//! View rendering modules
//!
//! Each view in the application has its own module for rendering logic.

use chrono::{DateTime, Utc};
use ratatui::style::Color;

use crate::config::{Config, CustomShortcut};
use crate::focus_timing::FocusTimer;
use crate::session::{SessionInfo, SessionManager, SessionState, SessionType};

mod branch_detail;
mod claude_configs;
mod claude_settings;
mod codex_configs;
mod confirm;
mod custom_shortcuts;
mod focus_stats;
mod help;
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
pub use claude_settings::{
    render_claude_settings_copy_dialog, render_claude_settings_migrate_dialog,
};
pub use codex_configs::{
    render_agent_type_selector, render_codex_config_delete_dialog,
    render_codex_config_name_input_dialog, render_codex_config_path_input_dialog,
    render_codex_config_selector, render_codex_configs,
};
pub use confirm::{
    render_confirm_dialog, render_loading_indicator, render_quit_confirm_dialog,
    ConfirmDialogConfig,
};
pub use custom_shortcuts::render_custom_shortcut_dialogs;
pub use focus_stats::{
    render_focus_session_delete_dialog, render_focus_session_detail_dialog, render_focus_stats,
    render_timer_input_dialog,
};
pub use help::render_help_overlay;
pub use logs::render_log_viewer;
pub use notifications::{render_notification_badge, render_notifications};
pub use project_detail::{render_project_delete_confirmation, render_project_detail};
pub use projects::render_projects_overview;
pub use session::render_session_view;
pub use timeline::render_timeline;

/// How a session's state should read in a list
///
/// Shared by every view that lists sessions, so a session cannot describe
/// itself differently depending on which screen you are looking at.
///
/// Agent vocabulary is translated for shell sessions, which have no notion of
/// thinking: they are `Running` or `Ready`.
pub fn session_state_display(info: &SessionInfo, now: DateTime<Utc>) -> String {
    let base = base_state_display(info, now);

    // Codex subagents run in their own processes and write their own rollouts,
    // so a parent with three children working looks completely idle without
    // this. The count is inferred from recent writes and will sometimes be
    // stale, which is why it is a count rather than a claim about what they
    // are doing.
    match info.subagents {
        0 => base,
        1 => format!("{} · 1 subagent", base),
        n => format!("{} · {} subagents", base, n),
    }
}

fn base_state_display(info: &SessionInfo, now: DateTime<Utc>) -> String {
    match (info.session_type, info.state) {
        // A recovered session that cannot come back says why, rather than
        // offering an action that will fail
        (_, SessionState::Resumable) => match info.resume_blocker() {
            Some(reason) => format!("Unavailable - {}", reason),
            None => "Resumable".to_string(),
        },
        // Says why the process is gone, rather than leaving it looking hung
        (_, SessionState::Suspended) => {
            let mins = now
                .signed_duration_since(info.last_engagement)
                .num_minutes();
            if mins >= 60 {
                format!("Suspended - idle {}h", mins / 60)
            } else {
                "Suspended".to_string()
            }
        }
        (SessionType::Shell, SessionState::Executing) => "Running".to_string(),
        (SessionType::Shell, SessionState::Waiting) => "Ready".to_string(),
        // Name what is actually running. Several tools run at once whenever
        // subagents are involved, which is why this is a set and not a name.
        (_, SessionState::Executing) => match info.in_flight_summary() {
            Some(tools) => format!("Executing: {}", tools),
            None => "Executing".to_string(),
        },
        // A finished turn nobody has come back to is worth aging visibly
        (_, SessionState::Waiting) => {
            let mins = now.signed_duration_since(info.last_activity).num_minutes();
            if mins >= 1 {
                format!("Waiting - {}m", mins)
            } else {
                "Waiting".to_string()
            }
        }
        (_, state) => state.display_name().to_string(),
    }
}

/// Badge marking a session that wants the user, and its colour
///
/// `needs_attention` comes from `SessionManager::session_needs_attention`,
/// which also covers the time-based case where a session has simply been left
/// sitting; that has no explicit reason attached and reads as a plain
/// turn-complete.
pub fn attention_badge(info: &SessionInfo, needs_attention: bool) -> (&'static str, Color) {
    if !needs_attention {
        return ("  ", Color::White);
    }
    match &info.attention {
        Some(reason) => ("● ", reason.badge_color()),
        None => ("● ", Color::Green),
    }
}

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

/// Format custom shortcuts for footer display (e.g., "v:VSCode e:vim | ")
pub fn format_custom_shortcuts_hint(shortcuts: &[CustomShortcut]) -> String {
    if shortcuts.is_empty() {
        return String::new();
    }

    // Show up to 3 shortcuts to avoid cluttering the footer
    let display: Vec<String> = shortcuts
        .iter()
        .take(3)
        .map(|s| format!("{}:{}", s.key, s.short_display_name()))
        .collect();

    let suffix = if shortcuts.len() > 3 { "..." } else { "" };
    format!("{}{} | ", display.join(" "), suffix)
}

/// Format custom shortcuts for empty state display (multiline, one per line)
pub fn format_custom_shortcuts_list(shortcuts: &[CustomShortcut]) -> String {
    shortcuts
        .iter()
        .map(|s| format!("Press '{}' to run {}.", s.key, s.display_name()))
        .collect::<Vec<_>>()
        .join("\n")
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
