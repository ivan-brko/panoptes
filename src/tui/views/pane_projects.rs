//! Pane 1: the project tree and its drill-downs
//!
//! Four levels live here - the folder tree, a project's branches, a branch's
//! sessions, and per-project settings - and each row is rendered at whatever
//! density the pane's *current* width allows, so a pane can cross
//! strip -> compact -> full part-way through an accordion transition.

use chrono::Utc;
use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, Paragraph};

use crate::app::{AppState, InputMode, ProjectsNav};
use crate::project::{
    branch_count_label, folder_path_key, project_count_label, Branch, Project, ProjectId,
    ProjectStore, TreeRow,
};
use crate::session::SessionManager;
use crate::tui::panes::SideMode;
use crate::tui::theme::theme;
use crate::tui::views::{status_parts, truncate_string, window_rows};
use crate::tui::widgets::selection::{activity_style, selection_prefix, selection_style};

/// The per-project settings rows, in list order
pub const PROJECT_SETTINGS_ROWS: [&str; 4] = [
    "Default Claude config",
    "Default Codex config",
    "Default base branch",
    "Rename project",
];

/// Breadcrumb shown in pane 1's own block title, fitted to `width`
///
/// A two-segment breadcrumb drops its leading segment rather than truncating
/// the trailing one: at the branch level the project name is context the pane
/// beside it already carries, while the branch name is the thing being looked
/// at, so it is the last to go.
pub fn projects_breadcrumb(state: &AppState, project_store: &ProjectStore, width: usize) -> String {
    let name_of = |id: ProjectId| {
        project_store
            .get_project(id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "?".to_string())
    };

    let segments: Vec<String> = match state.projects_nav {
        ProjectsNav::Overview => {
            vec![format!("Projects ({})", project_store.project_count())]
        }
        ProjectsNav::Project(id) => vec![name_of(id)],
        ProjectsNav::Branch(id, branch_id) => {
            let branch = project_store
                .get_branch(branch_id)
                .map(|b| b.name.clone())
                .unwrap_or_else(|| "?".to_string());
            vec![name_of(id), branch]
        }
        ProjectsNav::ProjectSettings(id) => vec![name_of(id), "settings".to_string()],
    };

    for start in 0..segments.len() {
        let candidate = segments[start..].join(" > ");
        if candidate.chars().count() <= width {
            return candidate;
        }
    }
    truncate_string(segments.last().map(String::as_str).unwrap_or(""), width)
}

/// Render pane 1's content into `area` (already inside the pane border)
///
/// `mode` is the pane's density, decided once by the caller from the *outer*
/// pane width; recomputing it here from the inner rect would put the body two
/// columns out of step with its own title.
pub fn render_projects_pane(
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
        render_strip(frame, area, state, project_store, sessions);
        return;
    }

    // One-line inputs replace the pane's content; lists and paragraphs are
    // overlays anchored to the terminal instead (see `views::prompts`).
    match state.input_mode {
        InputMode::AddingProjectName => {
            render_inline_input(frame, area, "Project name", &state.new_project_name);
            return;
        }
        InputMode::RenamingProject => {
            render_inline_input(frame, area, "Rename project", &state.new_project_name);
            return;
        }
        InputMode::RenamingFolder => {
            let current = state
                .renaming_folder
                .as_ref()
                .map(|path| folder_path_key(path))
                .unwrap_or_default();
            render_inline_input(
                frame,
                area,
                &format!("Rename '{}'", current),
                &state.folder_input,
            );
            if let Some(error) = &state.folder_error {
                render_inline_error(frame, area, error);
            }
            return;
        }
        InputMode::CreatingSession => {
            render_inline_input(frame, area, "New Claude session", &state.session_draft.name);
            return;
        }
        InputMode::CreatingCodexSession => {
            render_inline_input(frame, area, "New Codex session", &state.session_draft.name);
            return;
        }
        InputMode::CreatingShellSession => {
            render_inline_input(frame, area, "New shell session", &state.session_draft.name);
            return;
        }
        _ => {}
    }

    match state.projects_nav {
        ProjectsNav::Overview => render_tree(frame, area, state, project_store, sessions, mode),
        ProjectsNav::Project(project_id) => render_branches(
            frame,
            area,
            state,
            project_id,
            project_store,
            sessions,
            mode,
        ),
        ProjectsNav::Branch(_, branch_id) => {
            render_branch_sessions(frame, area, state, branch_id, sessions, mode)
        }
        ProjectsNav::ProjectSettings(project_id) => {
            render_project_settings(frame, area, state, project_id, project_store, mode)
        }
    }
}

/// The whole pane in ten columns: a counter for whatever level it is on
fn render_strip(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
) {
    let count = match state.projects_nav {
        ProjectsNav::Overview => format!("P {}", project_store.project_count()),
        ProjectsNav::Project(id) => {
            format!("B {}", project_store.branches_for_project(id).len())
        }
        ProjectsNav::Branch(_, branch_id) => {
            format!("S {}", sessions.entries_for_branch(branch_id).len())
        }
        ProjectsNav::ProjectSettings(_) => "P cfg".to_string(),
    };
    frame.render_widget(Paragraph::new(count).style(theme().muted_style()), area);
}

/// A one-line text input filling the pane's content rect
fn render_inline_input(frame: &mut Frame, area: Rect, label: &str, value: &str) {
    let t = theme();
    let width = area.width as usize;
    let lines = vec![
        Line::from(Span::styled(truncate_string(label, width), t.muted_style())),
        Line::from(Span::styled(
            truncate_string(&format!("> {}_", value), width),
            t.input_style(),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

/// A validation message under an inline input
fn render_inline_error(frame: &mut Frame, area: Rect, error: &str) {
    if area.height < 4 {
        return;
    }
    let t = theme();
    let error_area = Rect::new(area.x, area.y + 3, area.width, 1);
    frame.render_widget(
        Paragraph::new(truncate_string(
            &format!("✖ {}", error),
            area.width as usize,
        ))
        .style(Style::default().fg(t.error_bg)),
        error_area,
    );
}

/// The folder tree of projects
fn render_tree(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
    mode: SideMode,
) {
    let t = theme();
    let rows = crate::project::visible_rows(project_store, project_store.collapsed_folders());
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new("No projects yet.\n\nPress 'n' to add a git repository.")
                .style(t.muted_style()),
            area,
        );
        return;
    }

    let selected_index = state.selected_project_index;
    let width = area.width as usize;
    let focused = state.is_focused(crate::app::Tab::Projects);

    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let selected = i == selected_index;
            let prefix = selection_prefix(selected && focused);
            let indent = "  ".repeat(row.depth());

            let (label, active_count, attention_count) = match row {
                TreeRow::Folder(folder) => {
                    // Roll up the whole subtree so a collapsed folder still
                    // shows what is happening inside it
                    let active: usize = folder
                        .descendants
                        .iter()
                        .map(|id| sessions.active_session_count_for_project(*id))
                        .sum();
                    let attention: usize = folder
                        .descendants
                        .iter()
                        .map(|id| sessions.attention_count_for_project(*id))
                        .sum();

                    // Fields are dropped whole rather than truncating one long
                    // string, so a narrow pane still reads as a list
                    let label = if mode == SideMode::Full {
                        let mut parts = vec![project_count_label(folder.descendants.len())];
                        parts.extend(status_parts(active, attention));
                        format!("{}/  ({})", folder.name(), parts.join(", "))
                    } else {
                        format!("{}/ ({})", folder.name(), folder.descendants.len())
                    };
                    (label, active, attention)
                }
                TreeRow::Project(entry) => {
                    let project = entry.project;
                    let branch_count = project_store.branch_count_for_project(project.id);
                    let session_count = sessions.session_count_for_project(project.id);
                    let active = sessions.active_session_count_for_project(project.id);
                    let attention = sessions.attention_count_for_project(project.id);

                    let label = if mode == SideMode::Full {
                        let branches = branch_count_label(branch_count);
                        let status = if active > 0 {
                            format!("{}, {} active", branches, active)
                        } else if session_count > 0 {
                            format!("{}, {} sessions", branches, session_count)
                        } else {
                            branches
                        };
                        format!("{} ({})", project.name, status)
                    } else {
                        format!("{} ({})", project.name, branch_count)
                    };
                    (label, active, attention)
                }
            };

            // The twisty gets its own column, which projects reserve as blank.
            // Otherwise the marker shifts a folder's name right by its own
            // width, cancelling out the extra indent level of its children and
            // leaving parent and child names in the same column.
            let twisty = match row {
                TreeRow::Folder(folder) if folder.expanded => "▾ ",
                TreeRow::Folder(_) => "▸ ",
                TreeRow::Project(_) => "  ",
            };
            let content =
                truncate_string(&format!("{}{}{}{}", prefix, indent, twisty, label), width);

            // Folders are structure, not status. Every hue in the theme
            // already means something about a session, so headings are set
            // apart by weight and keep the plain text color.
            let fallback = if matches!(row, TreeRow::Folder(_)) {
                Style::default().fg(t.text).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.text)
            };

            // Color precedence: attention > active > selected > default
            let style = activity_style(
                selected && focused,
                attention_count,
                active_count,
                fallback,
                t,
            );

            ListItem::new(content).style(style)
        })
        .collect();

    // The tree row index is the rendered row index here, so it doubles as the
    // scroll anchor
    let items = window_rows(items, selected_index, area.height);
    frame.render_widget(List::new(items), area);
}

/// One row of the branch list
struct BranchItem<'a> {
    display_name: &'a str,
    number: usize,
    selected: bool,
    active_count: usize,
    attention_count: usize,
    status: String,
    /// Worktree directory is missing; overrides every other color
    stale: bool,
    fallback: Style,
    width: usize,
}

fn branch_item(item: BranchItem) -> ListItem<'static> {
    let t = theme();
    let content = truncate_string(
        &format!(
            "{}{}: {}{}",
            selection_prefix(item.selected),
            item.number,
            item.display_name,
            item.status
        ),
        item.width,
    );

    // Color precedence: stale > attention > active > selected > fallback
    let style = if item.stale {
        selection_style(item.selected, t.danger)
    } else {
        activity_style(
            item.selected,
            item.attention_count,
            item.active_count,
            item.fallback,
            t,
        )
    };

    ListItem::new(content).style(style)
}

/// A project's branches: the local checkout, then its worktrees
fn render_branches(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_id: ProjectId,
    project_store: &ProjectStore,
    sessions: &SessionManager,
    mode: SideMode,
) {
    let t = theme();
    let Some(project) = project_store.get_project(project_id) else {
        frame.render_widget(
            Paragraph::new("Project not found").style(Style::default().fg(t.error_bg)),
            area,
        );
        return;
    };
    let branches = project_store.branches_for_project_sorted(project_id);
    if branches.is_empty() {
        frame.render_widget(
            Paragraph::new("No branches tracked yet.\n\nPress 'n' to create a worktree.")
                .style(t.muted_style()),
            area,
        );
        return;
    }

    let width = area.width as usize;
    let focused = state.is_focused(crate::app::Tab::Projects);
    let selected_index = state.selected_branch_index;

    let local_checkout: Option<&Branch> = branches.iter().find(|b| b.is_default).copied();
    let worktrees: Vec<&Branch> = branches.iter().filter(|b| b.is_worktree).copied().collect();

    // The main repo's HEAD can move outside Panoptes, so it is read from git
    // rather than trusted from the store
    let current_branch_display = current_branch_name(project);

    let mut items: Vec<ListItem> = Vec::new();
    let mut item_index = 0;
    // Section headings are rows too, so the branch index is not the row index
    let mut selected_row = 0;

    if let Some(branch) = local_checkout {
        if mode == SideMode::Full {
            items.push(ListItem::new("Local checkout:").style(t.muted_style()));
        }

        let active_count = sessions.active_session_count_for_branch(branch.id);
        let attention_count = sessions.attention_count_for_branch(branch.id);
        let status = if active_count > 0 && mode == SideMode::Full {
            format!("  {} active", active_count)
        } else {
            String::new()
        };

        if item_index == selected_index {
            selected_row = items.len();
        }
        items.push(branch_item(BranchItem {
            display_name: current_branch_display.as_deref().unwrap_or(&branch.name),
            number: item_index + 1,
            selected: item_index == selected_index && focused,
            active_count,
            attention_count,
            status,
            stale: false,
            fallback: Style::default().fg(t.accent),
            width,
        }));
        item_index += 1;
    }

    if !worktrees.is_empty() {
        if mode == SideMode::Full {
            if local_checkout.is_some() {
                items.push(ListItem::new("─".repeat(width.min(40))).style(t.muted_style()));
            }
            items.push(ListItem::new("Worktrees:").style(t.muted_style()));
        }

        for branch in &worktrees {
            let active_count = sessions.active_session_count_for_branch(branch.id);
            let attention_count = sessions.attention_count_for_branch(branch.id);

            let status = if mode == SideMode::Full {
                let mut parts = Vec::new();
                if active_count > 0 {
                    parts.push(format!("{} active", active_count));
                }
                if branch.stale {
                    parts.push("⚠ missing".to_string());
                }
                if parts.is_empty() {
                    String::new()
                } else {
                    format!("  ({})", parts.join(", "))
                }
            } else if branch.stale {
                " ⚠".to_string()
            } else {
                String::new()
            };

            if item_index == selected_index {
                selected_row = items.len();
            }
            items.push(branch_item(BranchItem {
                display_name: &branch.name,
                number: item_index + 1,
                selected: item_index == selected_index && focused,
                active_count,
                attention_count,
                status,
                stale: branch.stale,
                fallback: Style::default().fg(t.text),
                width,
            }));
            item_index += 1;
        }
    }

    let items = window_rows(items, selected_row, area.height);
    frame.render_widget(List::new(items), area);
}

/// The current branch of the main checkout, straight from git
///
/// Falls back to the stored name when git cannot answer, and says so plainly
/// when HEAD is detached rather than showing a stale branch name.
fn current_branch_name(project: &Project) -> Option<String> {
    match crate::git::GitOps::open(&project.repo_path) {
        Ok(git) => match git.current_branch() {
            Ok(Some(name)) => Some(name),
            Ok(None) => Some("detached HEAD".to_string()),
            Err(_) => None,
        },
        Err(_) => None,
    }
}

/// The sessions of one branch
fn render_branch_sessions(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    branch_id: crate::project::BranchId,
    sessions: &SessionManager,
    mode: SideMode,
) {
    let t = theme();
    let entries = sessions.entries_for_branch(branch_id);
    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new(
                "No sessions on this branch yet.\n\n\
                 n: new AI session\ns: shell session",
            )
            .style(t.muted_style()),
            area,
        );
        return;
    }

    let now = Utc::now();
    let width = area.width as usize;
    let focused = state.is_focused(crate::app::Tab::Projects);
    let selected_index = state.branch_session_index;

    let items: Vec<ListItem> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let info = entry.info;
            let selected = i == selected_index && focused;
            let (badge, badge_color) = super::attention_badge(info, info.needs_attention());
            let state_display = super::session_state_display(info, now);

            let mut spans = vec![
                Span::raw(selection_prefix(selected)),
                Span::styled(badge, Style::default().fg(badge_color)),
                Span::styled(
                    format!("{} ", info.session_type.short_tag()),
                    t.muted_style(),
                ),
            ];

            if mode == SideMode::Full {
                // What the session wants, or failing that what it last said.
                // The reason is the more useful of the two, so it wins.
                let trailer = info
                    .attention
                    .as_ref()
                    .map(|reason| reason.summary())
                    .or_else(|| info.last_message.clone());
                spans.push(Span::raw(format!(
                    "{}: {} [{}]",
                    i + 1,
                    info.name,
                    state_display
                )));
                if let Some(trailer) = trailer {
                    spans.push(Span::styled(format!(" — {}", trailer), t.muted_style()));
                }
            } else {
                spans.push(Span::raw(format!(
                    "{} [{}]",
                    info.name,
                    compact_state(&state_display)
                )));
            }

            ListItem::new(clamp_line(Line::from(spans), width))
                .style(selection_style(selected, info.state.color()))
        })
        .collect();

    let items = window_rows(items, selected_index, area.height);
    frame.render_widget(List::new(items), area);
}

/// The per-project settings list, opened with `,`
fn render_project_settings(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_id: ProjectId,
    project_store: &ProjectStore,
    mode: SideMode,
) {
    let t = theme();
    let project = project_store.get_project(project_id);
    let width = area.width as usize;
    let focused = state.is_focused(crate::app::Tab::Projects);

    let values = [
        project
            .and_then(|p| p.default_claude_config)
            .map(|_| "set".to_string())
            .unwrap_or_else(|| "global default".to_string()),
        project
            .and_then(|p| p.default_codex_config)
            .map(|_| "set".to_string())
            .unwrap_or_else(|| "global default".to_string()),
        project
            .and_then(|p| p.default_base_branch.clone())
            .unwrap_or_else(|| "repo default".to_string()),
        project
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "?".to_string()),
    ];

    let items: Vec<ListItem> = PROJECT_SETTINGS_ROWS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let selected = i == state.project_settings_index && focused;
            let content = if mode == SideMode::Full {
                format!("{}{}: {}", selection_prefix(selected), label, values[i])
            } else {
                format!("{}{}", selection_prefix(selected), label)
            };
            ListItem::new(truncate_string(&content, width)).style(
                crate::tui::widgets::selection::selection_style_with_accent(selected, t),
            )
        })
        .collect();

    let items = window_rows(items, state.project_settings_index, area.height);
    frame.render_widget(List::new(items), area);
}

/// Shorten a state string for the compact density
///
/// `Executing: Bash(ls)` becomes `Exec`, `Waiting - 3m` becomes `Waiting`: the
/// qualifier is what a narrow pane cannot afford, not the state itself.
pub(crate) fn compact_state(state_display: &str) -> String {
    let head = state_display
        .split([':', '·', '-'])
        .next()
        .unwrap_or(state_display)
        .trim();
    match head {
        "Executing" => "Exec".to_string(),
        other => other.to_string(),
    }
}

/// Truncate a styled line to `width` characters, dropping whole spans first
pub(crate) fn clamp_line(line: Line<'static>, width: usize) -> Line<'static> {
    let total: usize = line
        .spans
        .iter()
        .map(|s| s.content.chars().count())
        .sum::<usize>();
    if total <= width {
        return line;
    }

    let mut used = 0;
    let mut spans = Vec::new();
    for span in line.spans {
        let len = span.content.chars().count();
        if used + len <= width {
            used += len;
            spans.push(span);
        } else {
            let room = width.saturating_sub(used);
            if room > 0 {
                let style = span.style;
                spans.push(Span::styled(truncate_string(&span.content, room), style));
            }
            break;
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::project::Project;
    use crate::session::store::SessionStore;
    use crate::tui::views::test_util::{column_of, contains_line, style_of_row_with};
    use ratatui::buffer::Buffer;
    use std::path::PathBuf;

    fn store_with(entries: &[(&str, &[&str])]) -> ProjectStore {
        let mut store = ProjectStore::new();
        for (name, folder) in entries {
            let mut project = Project::new(
                name.to_string(),
                PathBuf::from(format!("/tmp/{}", name)),
                "main".to_string(),
            );
            project.folder = folder.iter().map(|s| s.to_string()).collect();
            store.add_project(project);
        }
        store
    }

    fn render_buffer(width: u16, state: &AppState, store: &ProjectStore) -> Buffer {
        let config = Config::default();
        let sessions = SessionManager::with_store(config, SessionStore::new());
        // Mirror the caller: density comes from the *outer* pane width, which
        // is two columns wider than the content rect a body is handed
        let mode = crate::tui::panes::side_mode(width + 2);
        crate::tui::views::test_util::render_to_buffer(width, 12, |frame| {
            render_projects_pane(frame, frame.size(), state, store, &sessions, mode)
        })
    }

    fn render(width: u16, state: &AppState, store: &ProjectStore) -> Vec<String> {
        crate::tui::views::test_util::buffer_lines(&render_buffer(width, state, store))
    }

    #[test]
    fn test_full_density_shows_the_rolled_up_status() {
        let store = store_with(&[("api-gateway", &["Acme"][..]), ("panoptes", &[][..])]);
        let lines = render(60, &AppState::default(), &store);

        assert!(contains_line(&lines, "▾ Acme/  (1 project)"), "{lines:?}");
        assert!(contains_line(&lines, "panoptes (0 branches)"), "{lines:?}");
    }

    #[test]
    fn test_children_are_indented_past_their_folder_name() {
        let store = store_with(&[
            ("api-gateway", &["Acme"][..]),
            ("auth-service", &["Acme", "Platform"][..]),
            ("panoptes", &[][..]),
        ]);
        let lines = render(70, &AppState::default(), &store);

        // A project inside a folder must start right of the folder's name,
        // not level with it
        assert!(
            column_of(&lines, "api-gateway") > column_of(&lines, "Acme/"),
            "{lines:?}"
        );
        assert!(
            column_of(&lines, "auth-service") > column_of(&lines, "Platform/"),
            "{lines:?}"
        );

        // Siblings line up regardless of whether they are a folder or a project
        assert_eq!(
            column_of(&lines, "Platform/"),
            column_of(&lines, "api-gateway"),
            "{lines:?}"
        );
        assert_eq!(
            column_of(&lines, "Acme/"),
            column_of(&lines, "panoptes"),
            "{lines:?}"
        );
    }

    #[test]
    fn test_folder_rows_are_bold_plain_text_not_a_status_hue() {
        let store = store_with(&[("api-gateway", &["Acme"][..])]);
        // Select nothing so neither row picks up selection styling
        let state = AppState {
            selected_project_index: 99,
            ..Default::default()
        };
        let t = theme();

        let buffer = render_buffer(60, &state, &store);
        let folder = style_of_row_with(&buffer, "▾ Acme/");
        let project = style_of_row_with(&buffer, "api-gateway");

        // Folder: same color as a project row, set apart by weight alone
        assert_eq!(folder.fg, Some(t.text), "folders keep the plain text color");
        assert_eq!(project.fg, Some(t.text));
        assert!(folder.add_modifier.contains(Modifier::BOLD));
        assert!(!project.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_collapsed_folder_hides_contents_and_shows_the_rollup() {
        let mut store = store_with(&[
            ("api-gateway", &["Acme"][..]),
            ("auth-service", &["Acme", "Platform"][..]),
        ]);
        store.set_folder_collapsed(&["Acme".to_string()], true);

        let lines = render(60, &AppState::default(), &store);

        assert!(contains_line(&lines, "▸ Acme/  (2 projects)"), "{lines:?}");
        assert!(!contains_line(&lines, "auth-service"), "{lines:?}");
        assert!(!contains_line(&lines, "api-gateway"), "{lines:?}");
    }

    #[test]
    fn test_branch_count_is_singular_for_one() {
        let mut store = store_with(&[("solo", &[][..])]);
        let project_id = store.projects().next().unwrap().id;
        store.add_branch(crate::project::Branch::default_for_project(
            project_id,
            "main".to_string(),
            PathBuf::from("/tmp/solo"),
        ));

        let lines = render(60, &AppState::default(), &store);

        assert!(contains_line(&lines, "solo (1 branch)"), "{lines:?}");
        assert!(!contains_line(&lines, "1 branches"), "{lines:?}");
    }

    #[test]
    fn test_compact_density_drops_fields_rather_than_truncating() {
        let store = store_with(&[("api-gateway", &["Acme"][..])]);
        let lines = render(30, &AppState::default(), &store);

        // The count survives; the "1 project" wording does not
        assert!(contains_line(&lines, "Acme/ (1)"), "{lines:?}");
        assert!(!contains_line(&lines, "1 project)"), "{lines:?}");
    }

    /// A pane is a fraction of the terminal, so a list long enough to overflow
    /// it must scroll - otherwise `Enter` opens something off-screen
    #[test]
    fn test_a_long_list_scrolls_to_keep_the_selection_visible() {
        let names: Vec<String> = (0..40).map(|i| format!("project-{i:02}")).collect();
        let entries: Vec<(&str, &[&str])> = names.iter().map(|n| (n.as_str(), &[][..])).collect();
        let store = store_with(&entries);

        // Selection at the top: the first rows are what shows
        let state = AppState::default();
        let lines = render(60, &state, &store);
        assert!(contains_line(&lines, "project-00"), "{lines:?}");
        assert!(!contains_line(&lines, "project-39"), "{lines:?}");

        // Selection at the bottom: it must be on screen, and the top gone
        let state = AppState {
            selected_project_index: 39,
            ..Default::default()
        };
        let lines = render(60, &state, &store);
        assert!(
            contains_line(&lines, "▶   project-39"),
            "the selected row scrolled off: {lines:?}"
        );
        assert!(!contains_line(&lines, "project-00"), "{lines:?}");
    }

    #[test]
    fn test_strip_density_is_a_counter() {
        let store = store_with(&[("a", &[][..]), ("b", &[][..]), ("c", &[][..])]);
        let lines = render(10, &AppState::default(), &store);

        assert!(contains_line(&lines, "P 3"), "{lines:?}");
        assert!(!contains_line(&lines, "branches"), "{lines:?}");
    }

    #[test]
    fn test_no_row_renders_past_the_pane_width() {
        let store = store_with(&[("a-project-with-a-very-long-name-indeed", &["Acme"][..])]);
        for width in [10_u16, 22, 30, 44] {
            let lines = render(width, &AppState::default(), &store);
            for line in &lines {
                assert!(
                    line.chars().count() <= width as usize,
                    "row {line:?} overflows a {width}-column pane"
                );
            }
        }
    }

    #[test]
    fn test_hidden_pane_renders_nothing() {
        let store = store_with(&[("panoptes", &[][..])]);
        let lines = render(0, &AppState::default(), &store);
        assert!(lines.iter().all(|l| l.is_empty()), "{lines:?}");
    }

    #[test]
    fn test_project_settings_lists_the_four_relocated_flows() {
        let store = store_with(&[("panoptes", &[][..])]);
        let project_id = store.projects().next().unwrap().id;
        let state = AppState {
            projects_nav: ProjectsNav::ProjectSettings(project_id),
            ..Default::default()
        };

        let lines = render(60, &state, &store);

        for row in PROJECT_SETTINGS_ROWS {
            assert!(contains_line(&lines, row), "{row} missing from {lines:?}");
        }
        assert!(
            contains_line(&lines, "▶ Default Claude config"),
            "{lines:?}"
        );
    }

    #[test]
    fn test_inline_rename_replaces_the_list() {
        let store = store_with(&[("panoptes", &[][..])]);
        let project_id = store.projects().next().unwrap().id;
        let state = AppState {
            projects_nav: ProjectsNav::ProjectSettings(project_id),
            input_mode: InputMode::RenamingProject,
            new_project_name: "renamed".to_string(),
            ..Default::default()
        };

        let lines = render(60, &state, &store);

        assert!(contains_line(&lines, "Rename project"), "{lines:?}");
        assert!(contains_line(&lines, "> renamed_"), "{lines:?}");
        assert!(!contains_line(&lines, "Default Claude config"), "{lines:?}");
    }

    #[test]
    fn test_breadcrumb_names_each_level() {
        let mut store = store_with(&[("panoptes", &[][..])]);
        let project_id = store.projects().next().unwrap().id;
        let branch = crate::project::Branch::default_for_project(
            project_id,
            "main".to_string(),
            PathBuf::from("/tmp/panoptes"),
        );
        let branch_id = branch.id;
        store.add_branch(branch);

        let mut state = AppState::default();
        assert_eq!(projects_breadcrumb(&state, &store, 60), "Projects (1)");

        state.projects_nav = ProjectsNav::Project(project_id);
        assert_eq!(projects_breadcrumb(&state, &store, 60), "panoptes");

        state.projects_nav = ProjectsNav::Branch(project_id, branch_id);
        assert_eq!(projects_breadcrumb(&state, &store, 60), "panoptes > main");

        state.projects_nav = ProjectsNav::ProjectSettings(project_id);
        assert_eq!(
            projects_breadcrumb(&state, &store, 60),
            "panoptes > settings"
        );
    }

    /// A breadcrumb that will not fit drops its leading segment whole: the
    /// branch is what the pane is showing, so it is the last thing to go
    #[test]
    fn test_breadcrumb_drops_the_project_before_it_cuts_the_branch() {
        let mut store = ProjectStore::new();
        let project = Project::new(
            "a-project-with-a-really-long-name".to_string(),
            PathBuf::from("/tmp/x"),
            "main".to_string(),
        );
        let project_id = project.id;
        store.add_project(project);
        let branch = crate::project::Branch::default_for_project(
            project_id,
            "pan-6-accordion".to_string(),
            PathBuf::from("/tmp/x"),
        );
        let branch_id = branch.id;
        store.add_branch(branch);

        let state = AppState {
            projects_nav: ProjectsNav::Branch(project_id, branch_id),
            ..Default::default()
        };

        assert_eq!(
            projects_breadcrumb(&state, &store, 60),
            "a-project-with-a-really-long-name > pan-6-accordion"
        );
        // Too narrow for both: the branch survives intact
        assert_eq!(projects_breadcrumb(&state, &store, 30), "pan-6-accordion");
        // Too narrow even for that: truncate, but never past the width
        assert_eq!(projects_breadcrumb(&state, &store, 10).chars().count(), 10);
    }

    #[test]
    fn test_compact_state_keeps_the_state_and_drops_the_qualifier() {
        assert_eq!(compact_state("Executing: Bash(ls)"), "Exec");
        assert_eq!(compact_state("Waiting - 3m"), "Waiting");
        assert_eq!(compact_state("Waiting"), "Waiting");
        assert_eq!(compact_state("Suspended - idle 2h"), "Suspended");
    }

    #[test]
    fn test_clamp_line_drops_whole_spans_then_truncates() {
        let line = Line::from(vec![
            Span::raw("abcde"),
            Span::raw("fghij"),
            Span::raw("klmno"),
        ]);
        let clamped = clamp_line(line, 7);
        let text: String = clamped
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert_eq!(text.chars().count(), 7);
        assert!(text.starts_with("abcde"));
    }
}
