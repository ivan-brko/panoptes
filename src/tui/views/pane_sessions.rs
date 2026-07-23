//! Pane 2: every session, flat and sorted
//!
//! Sessions carry the widest content in the application, which is why this
//! pane benefits most from focus-expand and needs the fullest degradation. The
//! "Needs Attention" list is pinned to the top of the pane; the blinking
//! indicator stays in the global header so it is visible from every pane.

use chrono::Utc;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{AppState, Tab};
use crate::project::ProjectStore;
use crate::session::SessionManager;
use crate::tui::panes::{side_mode, SideMode};
use crate::tui::theme::theme;
use crate::tui::views::pane_projects::{clamp_line, compact_state};
use crate::tui::widgets::selection::{selection_prefix, selection_style};

/// Rows the attention section may take, borders included
const ATTENTION_MAX_HEIGHT: u16 = 8;

/// Pane 2's block title at the given density
pub fn sessions_title(sessions: &SessionManager, mode: SideMode) -> String {
    match mode {
        SideMode::Strip | SideMode::Hidden => String::new(),
        _ => format!("Sessions ({})", sessions.len()),
    }
}

/// Render pane 2's content into `area` (already inside the pane border)
pub fn render_sessions_pane(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
) {
    let mode = side_mode(area.width);
    if mode == SideMode::Hidden || area.height == 0 {
        return;
    }
    if mode == SideMode::Strip {
        render_strip(frame, area, sessions);
        return;
    }

    let attention = sessions.sessions_needing_attention();
    // The pinned section only earns its rows when there is something in it and
    // enough height left for a list underneath
    let attention_height = if attention.is_empty() || area.height < 8 {
        0
    } else {
        ((attention.len() + 2) as u16).min(ATTENTION_MAX_HEIGHT)
    };

    let (attention_area, list_area) = if attention_height > 0 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(attention_height), Constraint::Min(0)])
            .split(area);
        (Some(chunks[0]), chunks[1])
    } else {
        (None, area)
    };

    if let Some(attention_area) = attention_area {
        render_attention_section(frame, attention_area, &attention, project_store, mode);
    }
    render_session_list(frame, list_area, state, project_store, sessions, mode);
}

/// Pick the densest session row that actually fits
///
/// The full row leads with project and branch, which is context worth having
/// until it costs the session's own name - so a full row that would be cut
/// falls back to the compact one whole, rather than truncating into nonsense.
/// This is what keeps a long project name from eating the entire row.
fn session_body(mode: SideMode, room: usize, full: &str, compact: &str) -> String {
    if mode == SideMode::Full && full.chars().count() <= room {
        full.to_string()
    } else {
        compact.to_string()
    }
}

fn project_name_of<'a>(store: &'a ProjectStore, info: &crate::session::SessionInfo) -> &'a str {
    store
        .get_project(info.project_id)
        .map(|p| p.name.as_str())
        .unwrap_or("?")
}

fn branch_name_of<'a>(store: &'a ProjectStore, info: &crate::session::SessionInfo) -> &'a str {
    store
        .get_branch(info.branch_id)
        .map(|b| b.name.as_str())
        .unwrap_or("?")
}

/// The whole pane in ten columns: `S 7●2`
fn render_strip(frame: &mut Frame, area: Rect, sessions: &SessionManager) {
    let attention = sessions.total_attention_count();
    let text = if attention > 0 {
        format!("S {}●{}", sessions.len(), attention)
    } else {
        format!("S {}", sessions.len())
    };
    frame.render_widget(Paragraph::new(text).style(theme().muted_style()), area);
}

/// The pinned "Needs Attention" section
fn render_attention_section(
    frame: &mut Frame,
    area: Rect,
    attention: &[&crate::session::Session],
    project_store: &ProjectStore,
    mode: SideMode,
) {
    let t = theme();
    let now = Utc::now();
    let width = area.width.saturating_sub(2) as usize;

    let items: Vec<ListItem> = attention
        .iter()
        .map(|session| {
            let info = &session.info;
            let (_, badge_color) = super::attention_badge(info, true);
            let state_text = super::session_state_display(info, now);

            // Two leading spans, five columns: "● " plus the agent tag
            let body = session_body(
                mode,
                width.saturating_sub(5),
                &format!(
                    "{} / {} / {} [{}]",
                    project_name_of(project_store, info),
                    branch_name_of(project_store, info),
                    info.name,
                    state_text
                ),
                &format!("{} [{}]", info.name, compact_state(&state_text)),
            );

            let line = Line::from(vec![
                Span::styled("● ", Style::default().fg(badge_color)),
                Span::styled(
                    format!("{} ", info.session_type.short_tag()),
                    t.muted_style(),
                ),
                Span::raw(body),
            ]);
            ListItem::new(clamp_line(line, width))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Needs Attention ({})", attention.len()))
            .border_style(Style::default().fg(t.border_warning)),
    );
    frame.render_widget(list, area);
}

/// The flat, sorted session list
fn render_session_list(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
    mode: SideMode,
) {
    let t = theme();
    let session_list = sessions.sessions_in_order();
    if session_list.is_empty() {
        frame.render_widget(
            Paragraph::new("No sessions yet.").style(t.muted_style()),
            area,
        );
        return;
    }

    let now = Utc::now();
    let width = area.width as usize;
    let focused = state.is_focused(Tab::Sessions);
    let selected_index = state.sessions_pane_index;

    let items: Vec<ListItem> = session_list
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let info = &session.info;
            let selected = i == selected_index && focused;
            let (badge, badge_color) = super::attention_badge(info, info.needs_attention());
            let state_display = super::session_state_display(info, now);

            // Three leading spans, seven columns: marker, badge, agent tag
            let body = session_body(
                mode,
                width.saturating_sub(7),
                &format!(
                    "{}: {} / {} / {} [{}]",
                    i + 1,
                    project_name_of(project_store, info),
                    branch_name_of(project_store, info),
                    info.name,
                    state_display
                ),
                &format!("{} [{}]", info.name, compact_state(&state_display)),
            );

            let line = Line::from(vec![
                Span::raw(selection_prefix(selected)),
                Span::styled(badge, Style::default().fg(badge_color)),
                Span::styled(
                    format!("{} ", info.session_type.short_tag()),
                    t.muted_style(),
                ),
                Span::raw(body),
            ]);

            ListItem::new(clamp_line(line, width))
                .style(selection_style(selected, info.state.color()))
        })
        .collect();

    frame.render_widget(List::new(items), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::session::store::SessionStore;
    use crate::tui::views::test_util::{contains_line, render_to_lines};
    use tempfile::TempDir;

    fn sessions_with(temp: &TempDir, names: &[&str]) -> SessionManager {
        let config = Config {
            worktrees_dir: temp.path().join("worktrees"),
            hooks_dir: temp.path().join("hooks"),
            ..Config::default()
        };
        let mut sessions = SessionManager::with_store(
            config,
            SessionStore::with_path(temp.path().join("sessions.json")),
        );
        for name in names {
            sessions
                .insert_test_session(name, uuid::Uuid::new_v4(), uuid::Uuid::new_v4())
                .unwrap();
        }
        sessions
    }

    fn render(width: u16, sessions: &SessionManager) -> Vec<String> {
        let store = ProjectStore::new();
        let state = AppState {
            focus: crate::app::Focus::Panes(Tab::Sessions),
            ..Default::default()
        };
        render_to_lines(width, 12, |frame| {
            render_sessions_pane(frame, frame.size(), &state, &store, sessions)
        })
    }

    #[test]
    fn test_full_density_names_the_project_and_branch() {
        let temp = TempDir::new().unwrap();
        let sessions = sessions_with(&temp, &["review"]);

        let lines = render(60, &sessions);

        assert!(contains_line(&lines, "1: ? / ? / review"), "{lines:?}");
    }

    #[test]
    fn test_compact_density_keeps_only_the_name_and_state() {
        let temp = TempDir::new().unwrap();
        let sessions = sessions_with(&temp, &["review"]);

        let lines = render(30, &sessions);

        assert!(contains_line(&lines, "review ["), "{lines:?}");
        assert!(!contains_line(&lines, "? / ?"), "{lines:?}");
    }

    #[test]
    fn test_strip_density_counts_sessions_and_attention() {
        let temp = TempDir::new().unwrap();
        let sessions = sessions_with(&temp, &["a", "b"]);

        let lines = render(10, &sessions);

        assert!(contains_line(&lines, "S 2"), "{lines:?}");
    }

    #[test]
    fn test_no_row_renders_past_the_pane_width() {
        let temp = TempDir::new().unwrap();
        let sessions = sessions_with(&temp, &["a-session-with-an-extremely-long-name"]);

        for width in [10_u16, 22, 30, 44] {
            let lines = render(width, &sessions);
            for line in &lines {
                assert!(
                    line.chars().count() <= width as usize,
                    "row {line:?} overflows a {width}-column pane"
                );
            }
        }
    }

    /// A full row that would be cut falls back to the compact one whole: a
    /// long project name must not eat the session's own name
    #[test]
    fn test_full_row_falls_back_to_compact_rather_than_being_cut() {
        let full = "1: a-very-long-project / a-very-long-branch / review [Waiting]";
        let compact = "review [Waiting]";

        assert_eq!(session_body(SideMode::Full, 80, full, compact), full);
        assert_eq!(session_body(SideMode::Full, 20, full, compact), compact);
        // Compact density never reaches for the full row at all
        assert_eq!(session_body(SideMode::Compact, 200, full, compact), compact);
    }

    #[test]
    fn test_empty_pane_says_so() {
        let temp = TempDir::new().unwrap();
        let sessions = sessions_with(&temp, &[]);
        let lines = render(40, &sessions);
        assert!(contains_line(&lines, "No sessions yet."), "{lines:?}");
    }
}
