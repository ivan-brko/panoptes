//! Projects overview view
//!
//! Displays a list of projects with their branch/session counts,
//! and a "quick sessions" section for sessions in the current directory.

use chrono::Utc;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{AppState, FolderMoveTarget, HomepageFocus, InputMode};
use crate::config::Config;
use crate::project::{
    branch_count_label, folder_path_key, project_count_label, ProjectStore, TreeRow,
    MAX_FOLDER_DEPTH,
};
use crate::session::{Session, SessionManager};
use crate::tui::header::Header;
use crate::tui::header_notifications::HeaderNotificationManager;
use crate::tui::layout::ScreenLayout;
use crate::tui::theme::theme;
use crate::tui::views::render_project_delete_confirmation;
use crate::tui::views::Breadcrumb;
use crate::tui::views::{footer_with_attention, render_footer, status_parts, visible_window};
use crate::tui::widgets::selection::{
    activity_style, selection_prefix, selection_style, selection_style_with_accent,
};

/// Render the projects overview
#[allow(clippy::too_many_arguments)]
pub fn render_projects_overview(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
    config: &Config,
    header_notifications: &HeaderNotificationManager,
) {
    let attention_count = sessions.total_attention_count();
    let attention_sessions = sessions.sessions_needing_attention();
    let has_dropped_events = state.dropped_events_count > 0;
    let t = theme();

    // Build header
    let active_count = sessions.total_active_count();
    let breadcrumb = Breadcrumb::new();
    let mut parts = vec![format!("{} projects", project_store.project_count())];
    parts.extend(status_parts(active_count, attention_count));
    let suffix = format!("({})", parts.join(", "));

    let header = Header::new(breadcrumb)
        .with_suffix(suffix)
        .with_notifications(Some(header_notifications))
        .with_attention_count(attention_count);

    // Create layout with header and footer
    let areas = ScreenLayout::new(area).with_header(header).render(frame);

    // Dynamic content layout based on warning banner and attention section
    let mut content_constraints: Vec<Constraint> = Vec::new();

    // Warning banner for dropped events
    if has_dropped_events {
        content_constraints.push(Constraint::Length(1));
    }

    // Attention section
    if attention_count > 0 && state.input_mode == InputMode::Normal {
        let attention_height = (attention_sessions.len() + 2).min(8) as u16;
        content_constraints.push(Constraint::Length(attention_height));
    }

    // Main content
    content_constraints.push(Constraint::Min(0));

    let content_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(content_constraints)
        .split(areas.content);

    let mut chunk_idx = 0;

    // Dropped events warning banner
    if has_dropped_events {
        let warning_text = format!(
            "⚠ {} hook events dropped due to channel overflow - session states may be inaccurate",
            state.dropped_events_count
        );
        let warning = Paragraph::new(warning_text).style(t.warning_banner_style());
        frame.render_widget(warning, content_chunks[chunk_idx]);
        chunk_idx += 1;
    }

    // Attention section (if needed and in normal mode)
    if attention_count > 0 && state.input_mode == InputMode::Normal {
        render_attention_section(
            frame,
            content_chunks[chunk_idx],
            &attention_sessions,
            project_store,
        );
        chunk_idx += 1;
    }

    let main_area = content_chunks[chunk_idx];

    // Main content area
    match state.input_mode {
        InputMode::AddingProject => {
            render_project_addition(frame, main_area, state);
        }
        InputMode::AddingProjectName => {
            render_project_name_input(frame, main_area, state);
        }
        InputMode::ConfirmingProjectDelete => {
            // Get the project being deleted
            let project = state
                .pending_delete_project
                .and_then(|id| project_store.get_project(id));
            render_project_delete_confirmation(frame, main_area, state, project, sessions, config);
        }
        InputMode::MovingToFolder => {
            render_folder_move(frame, main_area, state);
        }
        InputMode::RenamingFolder => {
            render_folder_rename(frame, main_area, state);
        }
        InputMode::ConfirmingFolderRemove => {
            render_folder_remove_confirmation(frame, main_area, state, project_store);
        }
        _ => {
            render_main_content(frame, main_area, state, project_store, sessions);
        }
    }

    // Footer with help
    let help_text = match state.input_mode {
        InputMode::AddingProject => {
            "Tab: autocomplete | Enter: select/validate | Esc: cancel".to_string()
        }
        InputMode::AddingProjectName => "Enter: create project | Esc: cancel".to_string(),
        InputMode::ConfirmingProjectDelete => "y: confirm delete | n/Esc: cancel".to_string(),
        InputMode::ConfirmingQuit => "y/Enter: quit | n/Esc: cancel".to_string(),
        InputMode::MovingToFolder => "Tab: complete | Enter: move | Esc: cancel".to_string(),
        InputMode::RenamingFolder => "Enter: rename | Esc: cancel".to_string(),
        InputMode::ConfirmingFolderRemove => "y: remove folder | n/Esc: cancel".to_string(),
        _ => footer_with_attention(normal_mode_footer(state, project_store, sessions), sessions),
    };
    render_footer(frame, areas.footer(), &help_text);
}

/// Build the normal-mode footer hint
///
/// The keys are context-sensitive: selecting a folder heading offers folder
/// actions, so the expand/collapse binding is advertised exactly when it
/// applies. The footer is one line, so the full list lives in the '?' overlay.
fn normal_mode_footer(
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
) -> String {
    let has_projects = project_store.project_count() > 0;
    let has_sessions = !sessions.is_empty();
    let switch_hint = if has_projects && has_sessions {
        "Tab: switch | "
    } else {
        ""
    };

    if !has_projects {
        return "n: new | c/x: configs | k: shortcuts | ?: help | Esc: quit".to_string();
    }

    // Which list the keys currently act on
    let projects_focused = !has_sessions || state.homepage_focus == HomepageFocus::Projects;
    let selected_folder = projects_focused
        && matches!(
            crate::project::row_at(project_store, state.selected_project_index),
            Some(crate::project::RowRef::Folder { .. })
        );

    if selected_folder {
        // Folder context drops the global keys to keep the line inside a
        // narrow terminal; "ungroup" signals that nothing gets deleted
        format!(
            "Enter/←→: expand/collapse | m: move | r: rename | d: ungroup | {}Esc: quit",
            switch_hint
        )
    } else {
        format!(
            "Enter: open | n: new | d: delete | {}c/x: configs | ?: help | Esc: quit",
            switch_hint
        )
    }
}

/// Render the "Needs Attention" section
fn render_attention_section(
    frame: &mut Frame,
    area: Rect,
    attention_sessions: &[&Session],
    project_store: &ProjectStore,
) {
    let t = theme();
    let now = Utc::now();

    let items: Vec<ListItem> = attention_sessions
        .iter()
        .map(|session| {
            // Get project/branch info
            let project_name = project_store
                .get_project(session.info.project_id)
                .map(|p| p.name.as_str())
                .unwrap_or("?");
            let branch_name = project_store
                .get_branch(session.info.branch_id)
                .map(|b| b.name.as_str())
                .unwrap_or("?");

            let (_, badge_color) = super::attention_badge(&session.info, true);
            let state_text = format!("[{}]", super::session_state_display(&session.info, now));

            let content = Line::from(vec![
                Span::styled("● ", Style::default().fg(badge_color)),
                Span::styled(
                    format!("{} ", session.info.session_type.short_tag()),
                    t.muted_style(),
                ),
                Span::raw(format!(
                    "{} / {} / {} {}",
                    project_name, branch_name, session.info.name, state_text
                )),
            ]);

            ListItem::new(content)
        })
        .collect();

    let title = format!("Needs Attention ({})", attention_sessions.len());
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(t.border_warning)),
    );
    frame.render_widget(list, area);
}

/// Render the project addition input with path completion
fn render_project_addition(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();

    // Calculate layout: input area + completions list
    let show_completions = state.show_path_completions && !state.path_completions.is_empty();
    let completions_height = if show_completions {
        // Show up to 8 completions
        (state.path_completions.len().min(8) + 2) as u16
    } else {
        0
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6), // Input area (hint + input line)
            Constraint::Length(completions_height),
            Constraint::Min(0), // Spacer
        ])
        .split(area);

    // Input area
    let hint = "Enter path to git repository (Tab: autocomplete, ~/):";
    let input_text = format!("{}\n\n> {}_", hint, state.new_project_path);
    let input = Paragraph::new(input_text)
        .style(t.input_style())
        .block(Block::default().borders(Borders::ALL).title("Add Project"));
    frame.render_widget(input, chunks[0]);

    // Completions list
    if show_completions {
        let completions = &state.path_completions;
        let selected_idx = state.path_completion_index;

        // Calculate visible range with scroll, keeping the selection in view
        let max_visible = 8;
        let total = completions.len();
        let (start, end) = visible_window(total, selected_idx, max_visible);

        let items: Vec<ListItem> = completions[start..end]
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let actual_idx = start + i;
                let is_selected = actual_idx == selected_idx;
                let prefix = selection_prefix(is_selected);
                let display = crate::path_complete::path_to_display(path);
                let content = format!("{}{}/", prefix, display);

                let style = selection_style_with_accent(is_selected, t);
                ListItem::new(content).style(style)
            })
            .collect();

        // Build title with scroll indicator
        let title = if total > max_visible {
            format!("Completions ({}/{}) ↑↓", selected_idx + 1, total)
        } else {
            format!("Completions ({})", total)
        };

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(t.border_focused)),
        );
        frame.render_widget(list, chunks[1]);
    }
}

/// Render the project name input (second step of project addition)
fn render_project_name_input(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();

    let path_display = state.pending_project_path.display();
    let subdir_info = state
        .pending_session_subdir
        .as_ref()
        .map(|s| format!("\nSubfolder: {}", s.display()))
        .unwrap_or_default();

    let hint = format!(
        "Repository: {}{}\n\nEnter project name (or press Enter for default):\n\n> {}_",
        path_display, subdir_info, state.new_project_name
    );

    let input = Paragraph::new(hint)
        .style(t.input_style())
        .block(Block::default().borders(Borders::ALL).title("Project Name"));
    frame.render_widget(input, area);
}

/// Render the main content area with projects and sessions
fn render_main_content(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
) {
    let has_projects = project_store.project_count() > 0;
    let has_sessions = !sessions.is_empty();

    if !has_projects && !has_sessions {
        // Empty state
        let t = theme();
        let empty_text = "No projects yet.\n\n\
            Press 'n' to add a git repository as a project.\n\
            Press '?' for help, or 'Esc' to quit.";
        let empty = Paragraph::new(empty_text)
            .style(t.muted_style())
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title("Welcome"));
        frame.render_widget(empty, area);
        return;
    }

    // Split area: projects on top, sessions on bottom (if both exist)
    if has_projects && has_sessions {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);

        let projects_focused = state.homepage_focus == HomepageFocus::Projects;
        render_project_list(
            frame,
            split[0],
            state,
            project_store,
            sessions,
            projects_focused,
        );
        render_quick_sessions(
            frame,
            split[1],
            state,
            project_store,
            sessions,
            !projects_focused,
        );
    } else if has_projects {
        render_project_list(
            frame,
            area,
            state,
            project_store,
            sessions,
            true, // Always focused when alone
        );
    } else {
        render_quick_sessions(
            frame,
            area,
            state,
            project_store,
            sessions,
            true, // Always focused when alone
        );
    }
}

/// Render the project list
fn render_project_list(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
    focused: bool,
) {
    let t = theme();
    let selected_index = state.selected_project_index;
    let rows = crate::project::visible_rows(project_store, project_store.collapsed_folders());

    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let selected = i == selected_index && focused;
            let prefix = selection_prefix(selected);
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

                    let mut parts = vec![project_count_label(folder.descendants.len())];
                    parts.extend(status_parts(active, attention));

                    (
                        format!("{}/  ({})", folder.name(), parts.join(", ")),
                        active,
                        attention,
                    )
                }
                TreeRow::Project(entry) => {
                    let project = entry.project;
                    let branch_count = project_store.branch_count_for_project(project.id);
                    let session_count = sessions.session_count_for_project(project.id);
                    let active = sessions.active_session_count_for_project(project.id);
                    let attention = sessions.attention_count_for_project(project.id);

                    let branches = branch_count_label(branch_count);
                    let status = if active > 0 {
                        format!("{}, {} active", branches, active)
                    } else if session_count > 0 {
                        format!("{}, {} sessions", branches, session_count)
                    } else {
                        branches
                    };

                    (format!("{} ({})", project.name, status), active, attention)
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
            let content = format!("{}{}{}{}", prefix, indent, twisty, label);

            // Folders are structure, not status. Every hue in the theme
            // already means something about a session, so headings are set
            // apart by weight and keep the plain text color.
            let fallback = if matches!(row, TreeRow::Folder(_)) {
                Style::default().fg(t.text).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.text)
            };

            // Color precedence: attention > active > selected > default
            let style = activity_style(selected, attention_count, active_count, fallback, t);

            ListItem::new(content).style(style)
        })
        .collect();

    let border_color = if focused { t.border_focused } else { t.border };
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Projects")
            .border_style(Style::default().fg(border_color)),
    );
    frame.render_widget(list, area);
}

/// Render the "move to folder" input with folder autocomplete
fn render_folder_move(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();

    let what = match &state.moving_to_folder {
        Some(FolderMoveTarget::Folder(path)) => {
            format!("folder '{}' and its contents", folder_path_key(path))
        }
        _ => "the selected project".to_string(),
    };

    let show_completions = state.show_folder_completions && !state.folder_completions.is_empty();
    let completions_height = if show_completions {
        (state.folder_completions.len().min(8) + 2) as u16
    } else {
        0
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if state.folder_error.is_some() { 8 } else { 7 }),
            Constraint::Length(completions_height),
            Constraint::Min(0),
        ])
        .split(area);

    let mut text = format!(
        "Move {} into a folder.\n\
         Use '/' to nest (max {} levels), leave empty for the root level.\n\n\
         > {}_",
        what, MAX_FOLDER_DEPTH, state.folder_input
    );
    if let Some(error) = &state.folder_error {
        text.push_str(&format!("\n\n✖ {}", error));
    }

    let input = Paragraph::new(text).style(t.input_style()).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Move to Folder"),
    );
    frame.render_widget(input, chunks[0]);

    if show_completions {
        render_folder_completions(frame, chunks[1], state);
    }
}

/// Render the folder completions list
fn render_folder_completions(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();
    let completions = &state.folder_completions;
    let selected_idx = state.folder_completion_index;

    let max_visible = 8;
    let total = completions.len();
    let (start, end) = visible_window(total, selected_idx, max_visible);

    let items: Vec<ListItem> = completions[start..end]
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let is_selected = start + i == selected_idx;
            let content = format!("{}{}/", selection_prefix(is_selected), path);
            ListItem::new(content).style(selection_style_with_accent(is_selected, t))
        })
        .collect();

    let title = if total > max_visible {
        format!("Existing folders ({}/{}) ↑↓", selected_idx + 1, total)
    } else {
        format!("Existing folders ({})", total)
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(t.border_focused)),
    );
    frame.render_widget(list, area);
}

/// Render the folder rename input
fn render_folder_rename(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();

    let current = state
        .renaming_folder
        .as_ref()
        .map(|path| folder_path_key(path))
        .unwrap_or_default();

    let mut text = format!(
        "Renaming folder '{}'.\n\nEnter a new name:\n\n> {}_",
        current, state.folder_input
    );
    if let Some(error) = &state.folder_error {
        text.push_str(&format!("\n\n✖ {}", error));
    }

    let input = Paragraph::new(text).style(t.input_style()).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Rename Folder"),
    );
    frame.render_widget(input, area);
}

/// Render the confirmation for dissolving a folder
fn render_folder_remove_confirmation(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
) {
    let t = theme();

    let Some(path) = &state.pending_remove_folder else {
        return;
    };
    let affected = project_store
        .projects()
        .filter(|p| p.is_under_folder(path))
        .count();
    let destination = if path.len() > 1 {
        format!("'{}'", folder_path_key(&path[..path.len() - 1]))
    } else {
        "the root level".to_string()
    };

    let text = format!(
        "Remove folder '{}'?\n\n\
         Its {} move up to {}.\n\
         No projects or sessions are deleted.\n\n\
         y: remove folder    n/Esc: cancel",
        folder_path_key(path),
        project_count_label(affected),
        destination
    );

    let dialog = Paragraph::new(text)
        .style(Style::default().fg(t.text))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Remove Folder")
                .border_style(Style::default().fg(t.border_warning)),
        );
    frame.render_widget(dialog, area);
}

/// Render quick sessions (sessions not tied to a specific project)
fn render_quick_sessions(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
    focused: bool,
) {
    let t = theme();
    let now = Utc::now();
    // For now, show all sessions. Later we can filter by project_id == nil
    let selected_index = state.selected_session_index;
    let session_list = sessions.sessions_in_order();

    let items: Vec<ListItem> = session_list
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let info = &session.info;
            let selected = i == selected_index && focused;
            let prefix = selection_prefix(selected);

            // Check if session needs attention
            let needs_attention = info.needs_attention();

            // Get project/branch info
            let project_name = project_store
                .get_project(info.project_id)
                .map(|p| p.name.as_str())
                .unwrap_or("?");
            let branch_name = project_store
                .get_branch(info.branch_id)
                .map(|b| b.name.as_str())
                .unwrap_or("?");

            let state_display = super::session_state_display(info, now);
            let (badge, badge_color) = super::attention_badge(info, needs_attention);

            // Format: project / branch / session [state]
            let content = Line::from(vec![
                Span::raw(prefix),
                Span::styled(badge, Style::default().fg(badge_color)),
                Span::styled(
                    format!("{} ", info.session_type.short_tag()),
                    t.muted_style(),
                ),
                Span::raw(format!(
                    "{}: {} / {} / {} [{}]",
                    i + 1,
                    project_name,
                    branch_name,
                    info.name,
                    state_display
                )),
            ]);

            let style = selection_style(selected, info.state.color());
            ListItem::new(content).style(style)
        })
        .collect();

    let title = format!("Sessions ({})", session_list.len());
    let border_color = if focused { t.border_focused } else { t.border };
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(border_color)),
    );
    frame.render_widget(list, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Project;
    use crate::session::store::SessionStore;
    use crate::tui::views::test_util::{buffer_lines, column_of, contains_line, style_of_row_with};
    use std::path::PathBuf;

    /// Build a store holding projects at the given folder paths
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

    /// Render the projects overview into a buffer
    fn render_to_buffer(state: &AppState, project_store: &ProjectStore) -> Buffer {
        let config = Config::default();
        let sessions = SessionManager::with_store(config.clone(), SessionStore::new());
        let header_notifications = HeaderNotificationManager::default();

        crate::tui::views::test_util::render_to_buffer(80, 24, |frame| {
            render_projects_overview(
                frame,
                frame.size(),
                state,
                project_store,
                &sessions,
                &config,
                &header_notifications,
            );
        })
    }

    /// Render the projects overview and return its lines, trimmed of padding
    fn render_to_lines(state: &AppState, project_store: &ProjectStore) -> Vec<String> {
        buffer_lines(&render_to_buffer(state, project_store))
    }

    #[test]
    fn test_renders_nested_folders_with_indentation() {
        let store = store_with(&[
            ("panoptes", &[][..]),
            ("api-gateway", &["Acme"][..]),
            ("auth-service", &["Acme", "Platform"][..]),
        ]);
        let state = AppState::default();

        let lines = render_to_lines(&state, &store);

        // Leading "│" is the list border, so these pin the exact indentation.
        // Row 0 is selected by default, so it carries the "▶ " marker instead
        // of the two-space prefix - both are two columns wide. The twisty
        // occupies its own column, which project rows leave blank.
        assert!(contains_line(&lines, "│▶ ▾ Acme/"), "{:?}", lines);
        assert!(contains_line(&lines, "│    ▾ Platform/"), "{:?}", lines);
        assert!(
            contains_line(&lines, "│        auth-service"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "│      api-gateway"), "{:?}", lines);
        assert!(contains_line(&lines, "│    panoptes"), "{:?}", lines);
    }

    #[test]
    fn test_children_are_indented_past_their_folder_name() {
        let store = store_with(&[
            ("api-gateway", &["Acme"][..]),
            ("auth-service", &["Acme", "Platform"][..]),
            ("panoptes", &[][..]),
        ]);
        let state = AppState::default();

        let lines = render_to_lines(&state, &store);

        // A project inside a folder must start right of the folder's name,
        // not level with it
        assert!(
            column_of(&lines, "api-gateway") > column_of(&lines, "Acme/"),
            "{:?}",
            lines
        );
        assert!(
            column_of(&lines, "auth-service") > column_of(&lines, "Platform/"),
            "{:?}",
            lines
        );

        // Siblings line up regardless of whether they are a folder or a project
        assert_eq!(
            column_of(&lines, "Platform/"),
            column_of(&lines, "api-gateway"),
            "{:?}",
            lines
        );
        assert_eq!(
            column_of(&lines, "Acme/"),
            column_of(&lines, "panoptes"),
            "{:?}",
            lines
        );
    }

    #[test]
    fn test_folder_rows_are_bold_plain_text_not_a_status_hue() {
        let store = store_with(&[("api-gateway", &["Acme"][..])]);
        // Select nothing in the tree so neither row picks up selection styling
        let state = AppState {
            selected_project_index: 99,
            ..Default::default()
        };
        let t = theme();

        let buffer = render_to_buffer(&state, &store);
        let folder = style_of_row_with(&buffer, "▾ Acme/");
        let project = style_of_row_with(&buffer, "api-gateway");

        // Folder: same color as a project row, set apart by weight alone
        assert_eq!(
            folder.fg,
            Some(t.text),
            "folder should use plain text color"
        );
        assert_eq!(project.fg, Some(t.text));
        assert!(folder.add_modifier.contains(Modifier::BOLD));
        assert!(!project.add_modifier.contains(Modifier::BOLD));
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
        let state = AppState::default();

        let lines = render_to_lines(&state, &store);

        assert!(contains_line(&lines, "solo (1 branch)"), "{:?}", lines);
        assert!(!contains_line(&lines, "1 branches"), "{:?}", lines);
    }

    #[test]
    fn test_rows_are_not_numbered() {
        let store = store_with(&[("panoptes", &[][..]), ("api-gateway", &["Acme"][..])]);
        let state = AppState::default();

        let lines = render_to_lines(&state, &store);

        assert!(!contains_line(&lines, "1:"), "{:?}", lines);
        assert!(!contains_line(&lines, "2:"), "{:?}", lines);
    }

    #[test]
    fn test_collapsed_folder_hides_contents_and_shows_rollup() {
        let mut store = store_with(&[
            ("api-gateway", &["Acme"][..]),
            ("auth-service", &["Acme", "Platform"][..]),
        ]);
        store.set_folder_collapsed(&["Acme".to_string()], true);
        let state = AppState::default();

        let lines = render_to_lines(&state, &store);

        assert!(
            contains_line(&lines, "▸ Acme/  (2 projects)"),
            "{:?}",
            lines
        );
        assert!(!contains_line(&lines, "auth-service"), "{:?}", lines);
        assert!(!contains_line(&lines, "api-gateway"), "{:?}", lines);
    }

    #[test]
    fn test_footer_advertises_fold_when_folder_selected() {
        let store = store_with(&[("api-gateway", &["Acme"][..])]);
        let state = AppState {
            selected_project_index: 0, // the "Acme" heading
            ..Default::default()
        };

        let lines = render_to_lines(&state, &store);

        assert!(
            contains_line(&lines, "Enter/←→: expand/collapse"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "d: ungroup"), "{:?}", lines);
    }

    #[test]
    fn test_footer_uses_esc_to_quit_and_offers_help_not_q() {
        // Navigation is Esc-only now: the overview footer must offer Esc to
        // quit and ? for help, and must never advertise a `q` key again.
        let store = store_with(&[("api-gateway", &[][..])]);
        let state = AppState::default();

        let lines = render_to_lines(&state, &store);

        assert!(contains_line(&lines, "Esc: quit"), "{:?}", lines);
        assert!(contains_line(&lines, "?: help"), "{:?}", lines);
        assert!(
            !lines.iter().any(|l| l.contains("q: quit")),
            "footer must not advertise a q key: {:?}",
            lines
        );
    }

    #[test]
    fn test_footer_advertises_open_when_project_selected() {
        let store = store_with(&[("api-gateway", &["Acme"][..])]);
        let state = AppState {
            selected_project_index: 1, // the project under "Acme"
            ..Default::default()
        };

        let lines = render_to_lines(&state, &store);

        assert!(contains_line(&lines, "Enter: open"), "{:?}", lines);
        assert!(!contains_line(&lines, "expand/collapse"), "{:?}", lines);
    }

    #[test]
    fn test_selection_marks_folder_row() {
        let store = store_with(&[("api-gateway", &["Acme"][..])]);
        let state = AppState {
            selected_project_index: 0,
            ..Default::default()
        };

        let lines = render_to_lines(&state, &store);

        assert!(contains_line(&lines, "▶ ▾ Acme/"), "{:?}", lines);
    }

    #[test]
    fn test_folder_remove_confirmation_states_projects_are_kept() {
        let store = store_with(&[
            ("api-gateway", &["Acme"][..]),
            ("auth-service", &["Acme", "Platform"][..]),
        ]);
        let state = AppState {
            input_mode: InputMode::ConfirmingFolderRemove,
            pending_remove_folder: Some(vec!["Acme".to_string()]),
            ..Default::default()
        };

        let lines = render_to_lines(&state, &store);

        assert!(
            contains_line(&lines, "Remove folder 'Acme'?"),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "Its 2 projects move up to the root level."),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "No projects or sessions are deleted."),
            "{:?}",
            lines
        );
    }

    #[test]
    fn test_move_dialog_shows_error_and_completions() {
        let store = store_with(&[("api-gateway", &["Acme"][..])]);
        let project_id = store.projects().next().unwrap().id;
        let state = AppState {
            input_mode: InputMode::MovingToFolder,
            moving_to_folder: Some(FolderMoveTarget::Project(project_id)),
            folder_input: "a/b/c/d".to_string(),
            folder_error: Some("Folders can nest at most 3 levels deep (got 4)".to_string()),
            folder_completions: vec!["Acme".to_string()],
            show_folder_completions: true,
            ..Default::default()
        };

        let lines = render_to_lines(&state, &store);

        assert!(contains_line(&lines, "Move to Folder"), "{:?}", lines);
        assert!(contains_line(&lines, "> a/b/c/d_"), "{:?}", lines);
        assert!(
            contains_line(&lines, "at most 3 levels deep"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "Acme/"), "{:?}", lines);
    }
}
