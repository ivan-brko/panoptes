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
use crate::tui::panes::SideMode;
use crate::tui::theme::theme;
use crate::tui::views::pane_projects::{clamp_line, compact_state};
use crate::tui::views::{elide_middle, window_rows};
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
///
/// `mode` is decided once by the caller from the *outer* pane width; see
/// [`super::pane_projects::render_projects_pane`].
pub fn render_sessions_pane(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
    mode: SideMode,
) {
    if mode == SideMode::Hidden || area.height == 0 {
        return;
    }
    if mode == SideMode::Strip {
        render_strip(frame, area, sessions);
        return;
    }

    // The state summary strip: what makes the screen *look* different when
    // things are on fire versus when everything is calm, without reading a
    // single row. It only earns its row when there is height to spare.
    let area = if !sessions.is_empty() && area.height >= 6 {
        render_summary_strip(frame, Rect { height: 1, ..area }, sessions);
        Rect {
            y: area.y + 1,
            height: area.height - 1,
            ..area
        }
    } else {
        area
    };

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

/// Below this many columns an elided field is noise, not a name
const ELIDE_MIN: usize = 12;

/// The fields a session row is assembled from
///
/// `prefix` is the selection index (`"4: "`) in the numbered list and empty
/// in the attention section. It is part of the row's identity: the digit is
/// how the session is selected, so no degradation may drop it.
struct SessionRow<'a> {
    prefix: &'a str,
    project: &'a str,
    branch: &'a str,
    name: &'a str,
    state: &'a str,
}

/// Build the densest session row that fits in `room` columns
///
/// The row's identity - index, session name, state - stays whole at every
/// density. When the full row does not fit, the shortfall is charged to the
/// context fields alone: project and branch share what is left, the longer
/// one elided first, middle-ellipsis so branch slugs that agree at one end
/// stay distinguishable at the other. This is a deliberate exception to the
/// drop-fields-whole convention: eliding only the offending field keeps four
/// fields readable instead of discarding them all to protect one. Below
/// [`ELIDE_MIN`] an elided field is noise, so the row falls back to the
/// compact form whole - which keeps its index too.
fn session_body(mode: SideMode, room: usize, row: &SessionRow) -> String {
    let compact = format!("{}{} [{}]", row.prefix, row.name, compact_state(row.state));
    if mode != SideMode::Full {
        return compact;
    }
    let full = format!(
        "{}{} / {} / {} [{}]",
        row.prefix, row.project, row.branch, row.name, row.state
    );
    if full.chars().count() <= room {
        return full;
    }

    // Everything but project and branch is kept whole; the 9 is the two
    // " / " separators plus " [" and "]"
    let fixed = [row.prefix, row.name, row.state]
        .iter()
        .map(|s| s.chars().count())
        .sum::<usize>()
        + 9;
    let budget = room.saturating_sub(fixed);
    let (project_max, branch_max) = split_budget(
        row.project.chars().count(),
        row.branch.chars().count(),
        budget,
    );
    let too_short = |len: usize, max: usize| max < len && max < ELIDE_MIN;
    if too_short(row.project.chars().count(), project_max)
        || too_short(row.branch.chars().count(), branch_max)
    {
        return compact;
    }
    format!(
        "{}{} / {} / {} [{}]",
        row.prefix,
        elide_middle(row.project, project_max),
        elide_middle(row.branch, branch_max),
        row.name,
        row.state
    )
}

/// Share `budget` columns between two fields, charging the longer one first
///
/// A field that fits in half the budget keeps its full length and the other
/// takes the remainder; when both exceed half, they split it, first field
/// getting the odd column.
fn split_budget(a: usize, b: usize, budget: usize) -> (usize, usize) {
    if a + b <= budget {
        (a, b)
    } else if a <= budget / 2 {
        (a, budget - a)
    } else if b <= budget / 2 {
        (budget - b, b)
    } else {
        (budget - budget / 2, budget / 2)
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

/// One line of coloured state counts: `2 waiting · 1 thinking · 1 exec`
///
/// States are listed most-urgent first - what is blocked on you before what
/// is working before what is dead - and each count wears its state's colour,
/// so the strip reads at a glance even from across the room.
fn render_summary_strip(frame: &mut Frame, area: Rect, sessions: &SessionManager) {
    use crate::session::SessionState;

    let t = theme();
    let order = [
        SessionState::AwaitingApproval,
        SessionState::Waiting,
        SessionState::Thinking,
        SessionState::Executing,
        SessionState::Starting,
        SessionState::Resumable,
        SessionState::Suspended,
        SessionState::Exited,
    ];

    let list = sessions.sessions_in_order();
    let mut spans: Vec<Span> = Vec::new();
    for state in order {
        let count = list.iter().filter(|s| s.info.state == state).count();
        if count == 0 {
            continue;
        }
        if !spans.is_empty() {
            spans.push(Span::styled(" · ", t.muted_style()));
        }
        spans.push(Span::styled(
            format!("{} {}", count, summary_label(&state)),
            Style::default().fg(t.session_state_color(&state)),
        ));
    }

    let line = clamp_line(Line::from(spans), area.width as usize);
    frame.render_widget(Paragraph::new(line), area);
}

/// The strip's compact, lowercase name for a state
fn summary_label(state: &crate::session::SessionState) -> &'static str {
    use crate::session::SessionState;
    match state {
        SessionState::Starting => "starting",
        SessionState::Thinking => "thinking",
        SessionState::Executing => "exec",
        SessionState::AwaitingApproval => "approval",
        SessionState::Waiting => "waiting",
        SessionState::Suspended => "suspended",
        SessionState::Exited => "exited",
        SessionState::Resumable => "resumable",
    }
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
            let (badge, badge_color) = super::attention_badge(info, true);
            let state_text = super::session_state_display(info, now);

            // Two leading spans, seven columns: "● " plus the agent tag
            // ("[CC] "). An elided body fills its room exactly, so an
            // undercount here is not slack - it is two columns for
            // clamp_line to cut off the row's tail
            let body = session_body(
                mode,
                width.saturating_sub(7),
                &SessionRow {
                    prefix: "",
                    project: project_name_of(project_store, info),
                    branch: branch_name_of(project_store, info),
                    name: &info.name,
                    state: &state_text,
                },
            );

            let line = Line::from(vec![
                Span::styled(badge, Style::default().fg(badge_color)),
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

            // Three leading spans, nine columns: selection marker (2),
            // badge (2), agent tag "[CC] " (5). An elided body fills its
            // room exactly, so an undercount here is not slack - it is
            // columns for clamp_line to cut off the row's tail
            let body = session_body(
                mode,
                width.saturating_sub(9),
                &SessionRow {
                    prefix: &format!("{}: ", i + 1),
                    project: project_name_of(project_store, info),
                    branch: branch_name_of(project_store, info),
                    name: &info.name,
                    state: &state_display,
                },
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

    let items = window_rows(items, selected_index, area.height);
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
        render_with(width, sessions, &ProjectStore::new())
    }

    fn render_with(width: u16, sessions: &SessionManager, store: &ProjectStore) -> Vec<String> {
        let state = AppState {
            focus: crate::app::Focus::Panes(Tab::Sessions),
            ..Default::default()
        };
        let mode = crate::tui::panes::side_mode(width + 2);
        render_to_lines(width, 12, |frame| {
            render_sessions_pane(frame, frame.size(), &state, store, sessions, mode)
        })
    }

    /// The PAN-12 rows through the real render pipeline, leading spans and
    /// clamp_line included. An elided body fills the room it is given
    /// exactly, so this holds only while the room subtracted at the call
    /// site matches the true width of the leading spans - undercount it and
    /// clamp_line replaces the row's tail, state and all, with an ellipsis
    #[test]
    fn test_a_long_branch_elides_end_to_end_without_losing_the_row_tail() {
        let temp = TempDir::new().unwrap();

        let mut store = ProjectStore::new();
        let project = crate::project::Project::new(
            "panoptes".to_string(),
            temp.path().to_path_buf(),
            "main".to_string(),
        );
        let branch = crate::project::Branch::new(
            project.id,
            "pan-10-esc-backs-out-to-the-projects-pane-once-a-pane-has-nothing-left-to-pop"
                .to_string(),
            temp.path().to_path_buf(),
            false,
            true,
        );
        let config = Config {
            worktrees_dir: temp.path().join("worktrees"),
            hooks_dir: temp.path().join("hooks"),
            ..Config::default()
        };
        let mut sessions = SessionManager::with_store(
            config,
            SessionStore::with_path(temp.path().join("sessions.json")),
        );
        sessions
            .insert_test_session("feature", project.id, branch.id)
            .unwrap();
        store.add_branch(branch);
        store.add_project(project);

        // Full density, but well short of the ~105-column full row
        let lines = render_with(93, &sessions, &store);
        let row = lines
            .iter()
            .map(|l| l.trim_end())
            .find(|l| l.contains("feature"))
            .expect("the session row rendered");

        // Identity and the branch's head survive...
        assert!(row.contains("1: panoptes / pan-10-esc"), "{row:?}");
        // ...as do the branch's tail, the name, and the whole state
        assert!(row.contains("left-to-pop / feature ["), "{row:?}");
        assert!(row.ends_with(']'), "{row:?}");
        assert!(row.chars().count() <= 93, "{row:?}");
    }

    #[test]
    fn test_full_density_names_the_project_and_branch() {
        let temp = TempDir::new().unwrap();
        let sessions = sessions_with(&temp, &["review"]);

        let lines = render(60, &sessions);

        assert!(contains_line(&lines, "1: ? / ? / review"), "{lines:?}");
    }

    #[test]
    fn test_compact_density_keeps_the_index_name_and_state() {
        let temp = TempDir::new().unwrap();
        let sessions = sessions_with(&temp, &["review"]);

        let lines = render(30, &sessions);

        assert!(contains_line(&lines, "1: review ["), "{lines:?}");
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

    /// The offending rows from PAN-12: a worktree branch slugified from a
    /// ticket title, on a pane that is 19 columns too narrow for it
    fn pan_12_row() -> SessionRow<'static> {
        SessionRow {
            prefix: "4: ",
            project: "panoptes",
            branch: "pan-10-esc-backs-out-to-the-projects-pane-once-a-pane-has-nothing-left-to-pop",
            name: "feature",
            state: "Thinking",
        }
    }

    #[test]
    fn test_a_full_row_that_fits_is_untouched() {
        let row = pan_12_row();
        assert_eq!(
            session_body(SideMode::Full, 200, &row),
            "4: panoptes / pan-10-esc-backs-out-to-the-projects-pane-once-a-pane-has-nothing-left-to-pop / feature [Thinking]"
        );
    }

    /// A row that does not fit charges the shortfall to the long field: the
    /// index, project, name and state all survive, and the elided branch
    /// keeps both of its ends
    #[test]
    fn test_a_long_branch_is_elided_rather_than_the_row_collapsing() {
        let row = pan_12_row();

        let body = session_body(SideMode::Full, 93, &row);

        assert_eq!(body.chars().count(), 93);
        assert!(body.starts_with("4: panoptes / pan-10-esc"), "{body}");
        assert!(body.ends_with("left-to-pop / feature [Thinking]"), "{body}");
        assert!(body.contains("..."), "{body}");
    }

    /// Two sessions whose branch slugs differ only in the ticket number must
    /// not elide into the same row
    #[test]
    fn test_sibling_branches_stay_distinguishable_after_elision() {
        let pan_10 = pan_12_row();
        let pan_11 = SessionRow {
            branch: "pan-11-esc-backs-out-to-the-projects-pane-once-a-pane-has-nothing-left-to-pop",
            ..pan_12_row()
        };

        assert_ne!(
            session_body(SideMode::Full, 93, &pan_10),
            session_body(SideMode::Full, 93, &pan_11),
        );
    }

    /// A field elided below the floor is noise, so the row falls back to the
    /// compact form whole - which keeps the digit that selects the session,
    /// because two numberless identical stubs are the worst outcome available
    #[test]
    fn test_below_the_elision_floor_compact_keeps_its_index() {
        let row = pan_12_row();

        assert_eq!(
            session_body(SideMode::Full, 40, &row),
            "4: feature [Thinking]"
        );
        // Compact density never reaches for the full row at all
        assert_eq!(
            session_body(SideMode::Compact, 200, &row),
            "4: feature [Thinking]"
        );
    }

    #[test]
    fn test_split_budget_charges_the_longer_field_first() {
        // Both fit: untouched
        assert_eq!(split_budget(8, 10, 30), (8, 10));
        // The short one keeps itself whole, the long one takes the rest
        assert_eq!(split_budget(8, 77, 66), (8, 58));
        assert_eq!(split_budget(77, 8, 66), (58, 8));
        // Both long: an even split, first field getting the odd column
        assert_eq!(split_budget(50, 60, 33), (17, 16));
    }

    /// The summary strip: coloured counts that make a busy screen look
    /// different from a calm one without reading a single row
    #[test]
    fn test_summary_strip_counts_states() {
        let temp = TempDir::new().unwrap();
        let sessions = sessions_with(&temp, &["a", "b"]);

        let lines = render(60, &sessions);

        // Test sessions sit in their initial state
        assert!(contains_line(&lines, "2 starting"), "{lines:?}");
    }

    /// The strip never costs a row the list cannot spare
    #[test]
    fn test_summary_strip_gives_way_on_a_short_pane() {
        let temp = TempDir::new().unwrap();
        let sessions = sessions_with(&temp, &["a", "b"]);
        let store = ProjectStore::new();
        let state = AppState {
            focus: crate::app::Focus::Panes(Tab::Sessions),
            ..Default::default()
        };
        let mode = crate::tui::panes::side_mode(62);
        let lines = render_to_lines(60, 4, |frame| {
            render_sessions_pane(frame, frame.size(), &state, &store, &sessions, mode)
        });

        assert!(!contains_line(&lines, "2 starting"), "{lines:?}");
        assert!(contains_line(&lines, "1:"), "{lines:?}");
    }

    #[test]
    fn test_empty_pane_says_so() {
        let temp = TempDir::new().unwrap();
        let sessions = sessions_with(&temp, &[]);
        let lines = render(40, &sessions);
        assert!(contains_line(&lines, "No sessions yet."), "{lines:?}");
    }
}
