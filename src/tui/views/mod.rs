//! View rendering modules
//!
//! Each view in the application has its own module for rendering logic.

use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::config::CustomShortcut;
use crate::session::{SessionInfo, SessionManager, SessionState, SessionType};
use crate::tui::theme::theme;

mod agent_configs;
mod agent_select;
mod claude_settings;
mod confirm;
mod custom_shortcuts;
mod help;
pub(crate) mod pane_projects;
pub(crate) mod pane_sessions;
pub(crate) mod pane_settings;
mod panes;
mod prompts;
mod session;
mod worktree;

#[cfg(test)]
pub(crate) mod test_util;

pub use agent_configs::{
    render_agent_config_delete_dialog, render_agent_config_list,
    render_agent_config_name_input_dialog, render_agent_config_path_input_dialog,
    render_agent_config_selector,
};
pub use agent_select::render_agent_type_selector;
pub use claude_settings::{
    render_claude_settings_copy_dialog, render_claude_settings_migrate_dialog,
};
pub use confirm::{
    render_confirm_dialog, render_error_overlay, render_loading_indicator,
    render_quit_confirm_dialog, render_session_delete_confirmation, render_startup_notice_overlay,
    ConfirmDialogConfig,
};
pub use custom_shortcuts::{render_custom_shortcut_dialogs, render_shortcuts_list};
pub use help::render_help_overlay;
pub use panes::{render_panes, PaneContext};
pub use prompts::{
    render_folder_move_dialog, render_folder_remove_confirmation, render_project_addition_dialog,
};
pub use session::render_session_view;
pub use worktree::{
    render_branch_delete_confirmation, render_default_base_selector,
    render_project_delete_confirmation, render_worktree_wizard,
};

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
/// `needs_attention` comes from `SessionInfo::needs_attention`, and
/// is true exactly when a reason is attached, so the fallback arm is
/// unreachable in practice. It stays as a defined answer rather than a panic:
/// a badge is not worth taking the render down for.
pub fn attention_badge(info: &SessionInfo, needs_attention: bool) -> (&'static str, Color) {
    let t = theme();
    if !needs_attention {
        return ("  ", t.text);
    }
    match &info.attention {
        Some(reason) => ("● ", t.attention_color(reason)),
        None => ("● ", t.success),
    }
}

/// Format the attention hint for the footer showing the next session needing attention.
/// Returns None if no sessions need attention.
pub fn format_attention_hint(sessions: &SessionManager) -> Option<String> {
    sessions
        .sessions_needing_attention()
        .first()
        .map(|s| format!("Space: → {}", s.info.name))
}

/// Prepend the attention hint to a footer's base text when one applies
pub(crate) fn footer_with_attention(base: String, sessions: &SessionManager) -> String {
    match format_attention_hint(sessions) {
        Some(hint) => format!("{} | {}", hint, base),
        None => base,
    }
}

/// Render the standard one-line footer: muted text above a top border
pub(crate) fn render_footer(frame: &mut Frame, area: Rect, text: &str) {
    let t = theme();
    let footer = Paragraph::new(text.to_string())
        .style(t.muted_style())
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, area);
}

/// The "N active" / "M need attention" status fragments, in display order
pub(crate) fn status_parts(active: usize, attention: usize) -> Vec<String> {
    let mut parts = Vec::new();
    if active > 0 {
        parts.push(format!("{} active", active));
    }
    if attention > 0 {
        parts.push(format!("{} need attention", attention));
    }
    parts
}

/// Window of a scrolling list that keeps the selection visible with context
///
/// Returns the `(start, end)` range of items to show. Lists shorter than
/// `max_visible` show everything; longer lists keep the selection roughly
/// centered, pinned at the edges.
pub(crate) fn visible_window(total: usize, selected: usize, max_visible: usize) -> (usize, usize) {
    if total <= max_visible {
        return (0, total);
    }
    let half = max_visible / 2;
    let start = if selected < half {
        0
    } else if selected >= total - half {
        total - max_visible
    } else {
        selected - half
    };
    (start, start + max_visible)
}

/// Scroll a list so the selected row stays on screen
///
/// A pane is a fraction of the terminal, so a list that used to fit a
/// full-width screen no longer does. Without this the selection walks off the
/// bottom and `Enter` opens something the user cannot see.
///
/// `selected_row` counts rendered rows, not items: lists with section headings
/// have more of the former than the latter.
pub(crate) fn window_rows<T>(rows: Vec<T>, selected_row: usize, height: u16) -> Vec<T> {
    let height = height as usize;
    if height == 0 || rows.len() <= height {
        return rows;
    }
    let (start, end) = visible_window(rows.len(), selected_row.min(rows.len() - 1), height);
    rows.into_iter().take(end).skip(start).collect()
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
    /// Create an empty breadcrumb
    ///
    /// There is no "Panoptes" root segment: the wordmark in the header says
    /// the app's name, so repeating it two columns to the right only pushed
    /// the part that identifies where you are off the edge.
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
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
        let path = self.display();
        if suffix.is_empty() {
            path
        } else if path.is_empty() {
            suffix.to_string()
        } else {
            format!("{} {}", path, suffix)
        }
    }
}

impl Default for Breadcrumb {
    fn default() -> Self {
        Self::new()
    }
}

/// Truncate a path for display, keeping the end (most relevant part)
///
/// `max_len` is measured in characters, so multibyte text truncates safely.
/// The result never exceeds `max_len`: below four columns there is no room for
/// both content and an ellipsis, so the ellipsis is what goes.
pub(crate) fn truncate_path(path: &str, max_len: usize) -> String {
    let char_count = path.chars().count();
    if char_count <= max_len {
        return path.to_string();
    }
    if max_len <= 3 {
        return path.chars().skip(char_count - max_len).collect();
    }
    let tail: String = path.chars().skip(char_count - (max_len - 3)).collect();
    format!("...{}", tail)
}

/// Truncate a string for display, keeping the beginning
///
/// `max_len` is measured in characters, so multibyte text truncates safely.
/// The result never exceeds `max_len`, ellipsis included - which matters now
/// that a pane can be ten columns wide.
pub(crate) fn truncate_string(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        return s.to_string();
    }
    if max_len <= 3 {
        return s.chars().take(max_len).collect();
    }
    let head: String = s.chars().take(max_len - 3).collect();
    format!("{}...", head)
}

/// Truncate a string for display, keeping both ends
///
/// Branch names are slugified ticket titles, so two of them can agree for
/// dozens of characters at either end; a middle ellipsis keeps `pan-10-...`
/// and `pan-11-...` distinguishable where a head or tail cut would not. The
/// head gets the odd column. `max_len` is measured in characters and the
/// result never exceeds it, ellipsis included; below five columns there is
/// no room for both ends, so this degrades to a head-truncate.
pub(crate) fn elide_middle(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        return s.to_string();
    }
    if max_len <= 4 {
        return truncate_string(s, max_len);
    }
    let kept = max_len - 3;
    let head_len = (kept + 1) / 2;
    let head: String = s.chars().take(head_len).collect();
    let tail: String = s.chars().skip(char_count - (kept - head_len)).collect();
    format!("{}...{}", head, tail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_parts_only_names_what_is_happening() {
        assert!(status_parts(0, 0).is_empty());
        assert_eq!(status_parts(2, 0), vec!["2 active"]);
        assert_eq!(status_parts(0, 1), vec!["1 need attention"]);
        assert_eq!(status_parts(2, 1), vec!["2 active", "1 need attention"]);
    }

    #[test]
    fn test_window_rows_keeps_the_selection_on_screen() {
        let rows: Vec<usize> = (0..30).collect();

        // Short enough to fit: untouched
        assert_eq!(window_rows(rows.clone(), 0, 30).len(), 30);
        assert_eq!(window_rows(rows.clone(), 29, 40), rows);

        // A selection past the viewport scrolls it into view
        let visible = window_rows(rows.clone(), 25, 10);
        assert_eq!(visible.len(), 10);
        assert!(visible.contains(&25), "{visible:?}");

        let visible = window_rows(rows.clone(), 0, 10);
        assert_eq!(visible, (0..10).collect::<Vec<_>>());

        // A stale index past the end, and a zero-height pane, are survivable
        assert!(window_rows(rows.clone(), 99, 10).len() == 10);
        assert_eq!(window_rows(rows, 5, 0).len(), 30);
    }

    #[test]
    fn test_visible_window_short_list_shows_all() {
        assert_eq!(visible_window(5, 3, 8), (0, 5));
        assert_eq!(visible_window(8, 0, 8), (0, 8));
        assert_eq!(visible_window(0, 0, 8), (0, 0));
    }

    #[test]
    fn test_visible_window_pins_to_edges() {
        // Selection near the top: window starts at 0
        assert_eq!(visible_window(20, 0, 8), (0, 8));
        assert_eq!(visible_window(20, 3, 8), (0, 8));
        // Selection near the bottom: window ends at total
        assert_eq!(visible_window(20, 19, 8), (12, 20));
        assert_eq!(visible_window(20, 16, 8), (12, 20));
    }

    #[test]
    fn test_visible_window_centers_selection_in_the_middle() {
        assert_eq!(visible_window(20, 10, 8), (6, 14));
    }

    #[test]
    fn test_visible_window_total_half_boundary() {
        // selected == total - half is the first index that pins to the end
        let (total, max) = (10, 8);
        let half = max / 2;
        assert_eq!(visible_window(total, total - half, max), (2, 10));
        // One below the boundary still centers
        assert_eq!(visible_window(total, total - half - 1, max), (1, 9));
    }

    #[test]
    fn test_truncate_path_short_passes_through() {
        assert_eq!(truncate_path("/tmp/x", 45), "/tmp/x");
    }

    #[test]
    fn test_truncate_path_keeps_the_end() {
        assert_eq!(truncate_path("abcdefghij", 8), "...fghij");
    }

    #[test]
    fn test_truncate_path_multibyte_does_not_panic() {
        // Byte-based slicing panicked here: é and ö straddle byte boundaries
        let path = "/Users/héllo wörld/projects/áéíóú/wörkspace";
        let truncated = truncate_path(path, 20);
        assert!(truncated.starts_with("..."));
        assert_eq!(truncated.chars().count(), 20);
        assert!(path.ends_with(&truncated[3..]));
    }

    #[test]
    fn test_truncate_string_short_passes_through() {
        assert_eq!(truncate_string("Bash(ls)", 50), "Bash(ls)");
    }

    #[test]
    fn test_truncate_string_keeps_the_beginning() {
        assert_eq!(truncate_string("abcdefghij", 8), "abcde...");
    }

    /// A ten-column pane leaves less room than the ellipsis needs; the result
    /// must still fit, or the row spills across the pane border
    #[test]
    fn test_truncate_never_exceeds_the_width_it_was_given() {
        for max_len in 0..=6 {
            assert!(truncate_string("abcdefghij", max_len).chars().count() <= max_len);
            assert!(truncate_path("/a/very/long/path", max_len).chars().count() <= max_len);
        }
        assert_eq!(truncate_string("abcdefghij", 0), "");
        assert_eq!(truncate_string("abcdefghij", 3), "abc");
        assert_eq!(truncate_string("abcdefghij", 4), "a...");
        assert_eq!(truncate_path("abcdefghij", 3), "hij");
        assert_eq!(truncate_path("abcdefghij", 4), "...j");
    }

    #[test]
    fn test_truncate_string_multibyte_does_not_panic() {
        let s = "héllo wörld héllo wörld";
        let truncated = truncate_string(s, 10);
        assert!(truncated.ends_with("..."));
        assert_eq!(truncated.chars().count(), 10);
        assert_eq!(truncated, "héllo w...");
    }

    #[test]
    fn test_elide_middle_short_passes_through() {
        assert_eq!(elide_middle("pan-12-a-bug", 50), "pan-12-a-bug");
    }

    /// Two branch slugs that differ only in their ticket number must stay
    /// distinguishable after elision - both ends survive, head gets the odd
    /// column
    #[test]
    fn test_elide_middle_keeps_both_ends() {
        assert_eq!(elide_middle("abcdefghijklm", 8), "abc...lm");
        assert_ne!(
            elide_middle("pan-10-esc-backs-out-to-the-projects-pane", 20),
            elide_middle("pan-11-esc-backs-out-to-the-projects-pane", 20),
        );
    }

    #[test]
    fn test_elide_middle_never_exceeds_the_width_it_was_given() {
        for max_len in 0..=8 {
            assert!(elide_middle("abcdefghij", max_len).chars().count() <= max_len);
        }
        // Below five columns there is no room for two ends and an ellipsis
        assert_eq!(elide_middle("abcdefghij", 4), "a...");
        assert_eq!(elide_middle("abcdefghij", 5), "a...j");
    }

    #[test]
    fn test_elide_middle_multibyte_does_not_panic() {
        let s = "héllo wörld héllo wörld";
        let elided = elide_middle(s, 10);
        assert_eq!(elided.chars().count(), 10);
        assert_eq!(elided, "héll...rld");
    }
}
