//! Branch detail view
//!
//! Shows sessions for a specific branch.

use chrono::Utc;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{AppState, InputMode};
use crate::config::Config;
use crate::project::{BranchId, ProjectId, ProjectStore};
use crate::session::SessionManager;
use crate::tui::header::Header;
use crate::tui::header_notifications::HeaderNotificationManager;
use crate::tui::layout::ScreenLayout;
use crate::tui::theme::theme;
use crate::tui::views::Breadcrumb;
use crate::tui::views::{
    footer_with_attention, format_custom_shortcuts_hint, format_custom_shortcuts_list,
    render_footer, status_suffix,
};
use crate::tui::widgets::selection::{selection_prefix, selection_style};

/// Render the branch detail view showing sessions
#[allow(clippy::too_many_arguments)]
pub fn render_branch_detail(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_id: ProjectId,
    branch_id: BranchId,
    project_store: &ProjectStore,
    sessions: &SessionManager,
    config: &Config,
    header_notifications: &HeaderNotificationManager,
) {
    let project = project_store.get_project(project_id);
    let branch = project_store.get_branch(branch_id);

    // Build header
    let attention_count = sessions.attention_count_for_branch(branch_id);
    let (breadcrumb, suffix) = match (project, branch) {
        (Some(project), Some(branch)) => {
            let active_count = sessions.active_session_count_for_branch(branch_id);
            (
                Breadcrumb::new().push(&project.name).push(&branch.name),
                status_suffix(active_count, attention_count),
            )
        }
        _ => (Breadcrumb::new().push("?").push("?"), String::new()),
    };

    let header = Header::new(breadcrumb)
        .with_suffix(suffix)
        .with_notifications(Some(header_notifications))
        .with_attention_count(attention_count);

    // Create layout with header and footer
    let areas = ScreenLayout::new(area).with_header(header).render(frame);

    // Main content area - either session creation input, delete confirmation, or session list
    if state.input_mode == InputMode::CreatingSession {
        render_session_creation(frame, areas.content, state, "Claude Code");
    } else if state.input_mode == InputMode::CreatingShellSession {
        render_session_creation(frame, areas.content, state, "Shell");
    } else if state.input_mode == InputMode::CreatingCodexSession {
        render_session_creation(frame, areas.content, state, "Codex");
    } else if state.input_mode == InputMode::ConfirmingSessionDelete {
        super::render_session_delete_confirmation(frame, areas.content, state, sessions);
    } else if let Some(branch) = branch {
        let branch_sessions = sessions.entries_for_branch(branch_id);

        if branch_sessions.is_empty() {
            // Build custom shortcuts text if any exist
            let custom_shortcuts_text = if config.custom_shortcuts.is_empty() {
                String::new()
            } else {
                format!(
                    "\n{}",
                    format_custom_shortcuts_list(&config.custom_shortcuts)
                )
            };

            let empty_text = format!(
                "No sessions on this branch yet.\n\n\
                Press 'n' to create a new agent session.\n\
                Press 's' to create a shell session.{}\n\n\
                Working directory: {}",
                custom_shortcuts_text,
                branch.working_dir.display()
            );
            let empty = Paragraph::new(empty_text)
                .style(theme().muted_style())
                .block(Block::default().borders(Borders::ALL).title("Sessions"));
            frame.render_widget(empty, areas.content);
        } else {
            let selected_index = state.selected_session_index;
            let now = Utc::now();

            let items: Vec<ListItem> = branch_sessions
                .iter()
                .enumerate()
                .map(|(i, entry)| {
                    let info = entry.info;
                    let selected = i == selected_index;
                    let prefix = selection_prefix(selected);

                    // Check if session needs attention
                    let needs_attention = info.needs_attention();

                    let state_display = super::session_state_display(info, now);

                    let t = theme();
                    let short_tag = info.session_type.short_tag();

                    let (badge, badge_color) = super::attention_badge(info, needs_attention);

                    // What the session wants, or failing that what it last said.
                    // The reason is the more useful of the two, so it wins.
                    let trailer = info
                        .attention
                        .as_ref()
                        .map(|reason| reason.summary())
                        .or_else(|| info.last_message.clone());

                    let mut spans = vec![
                        Span::raw(prefix),
                        Span::styled(badge, Style::default().fg(badge_color)),
                        Span::styled(format!("{} ", short_tag), t.muted_style()),
                        Span::raw(format!("{}: {} [{}]", i + 1, info.name, state_display)),
                    ];
                    if let Some(trailer) = trailer {
                        spans.push(Span::styled(format!(" — {}", trailer), t.muted_style()));
                    }
                    let content = Line::from(spans);

                    let style = selection_style(selected, info.state.color());

                    ListItem::new(content).style(style)
                })
                .collect();

            let title = format!("Sessions ({})", branch_sessions.len());
            let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
            frame.render_widget(list, areas.content);
        }
    } else {
        let error = Paragraph::new("Branch not found")
            .style(Style::default().fg(theme().error_bg))
            .block(Block::default().borders(Borders::ALL).title("Error"));
        frame.render_widget(error, areas.content);
    }

    // Footer
    let help_text = match state.input_mode {
        InputMode::CreatingSession
        | InputMode::CreatingShellSession
        | InputMode::CreatingCodexSession => "Enter: create | Esc: cancel".to_string(),
        InputMode::SelectingAgentType => "↑/↓: navigate | Enter: select | Esc: cancel".to_string(),
        InputMode::ConfirmingSessionDelete => "y: confirm delete | n/Esc: cancel".to_string(),
        _ => {
            let shortcuts_hint = format_custom_shortcuts_hint(&config.custom_shortcuts);
            let base = format!(
                "n: new AI | s: shell | d: delete | {}Enter: open/resume | ?: help | Esc: back",
                shortcuts_hint
            );
            footer_with_attention(base, sessions)
        }
    };
    render_footer(frame, areas.footer(), &help_text);
}

/// Render the session creation input
fn render_session_creation(frame: &mut Frame, area: Rect, state: &AppState, session_type: &str) {
    let t = theme();
    let title = format!("Create {} Session", session_type);
    let input = Paragraph::new(format!("New session name: {}_", state.session_draft.name))
        .style(t.input_style())
        .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(input, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Branch, Project, ProjectStore};
    use crate::session::store::SessionStore;
    use crate::tui::views::test_util::{contains_line, render_to_lines};
    use std::path::PathBuf;

    fn store_with_branch() -> (ProjectStore, ProjectId, BranchId) {
        let mut store = ProjectStore::new();
        let project = Project::new(
            "panoptes".to_string(),
            PathBuf::from("/tmp/panoptes"),
            "main".to_string(),
        );
        let project_id = project.id;
        store.add_project(project);
        let branch = Branch::default_for_project(
            project_id,
            "main".to_string(),
            PathBuf::from("/tmp/panoptes"),
        );
        let branch_id = branch.id;
        store.add_branch(branch);
        (store, project_id, branch_id)
    }

    #[test]
    fn test_empty_branch_lists_creation_hints() {
        let (store, project_id, branch_id) = store_with_branch();
        let state = AppState::default();
        let config = Config::default();
        let sessions = SessionManager::with_store(config.clone(), SessionStore::new());
        let header_notifications = HeaderNotificationManager::default();

        let lines = render_to_lines(80, 24, |frame| {
            render_branch_detail(
                frame,
                frame.size(),
                &state,
                project_id,
                branch_id,
                &store,
                &sessions,
                &config,
                &header_notifications,
            )
        });

        assert!(
            contains_line(&lines, "No sessions on this branch yet."),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "Press 'n' to create a new agent session."),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "Working directory: /tmp/panoptes"),
            "{:?}",
            lines
        );
        // Breadcrumb and footer
        assert!(
            contains_line(&lines, "Panoptes > panoptes > main"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "n: new AI | s: shell"), "{:?}", lines);
    }
}
