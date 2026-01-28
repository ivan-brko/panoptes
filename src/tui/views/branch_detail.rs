//! Branch detail view
//!
//! Shows sessions for a specific branch.

use chrono::Utc;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{AppState, InputMode};
use crate::config::Config;
use crate::focus_timing::FocusTimer;
use crate::project::{BranchId, ProjectId, ProjectStore};
use crate::session::{SessionManager, SessionState, SessionType};
use crate::tui::header::Header;
use crate::tui::header_notifications::HeaderNotificationManager;
use crate::tui::layout::ScreenLayout;
use crate::tui::theme::theme;
use crate::tui::views::confirm::{render_confirm_dialog, ConfirmDialogConfig};
use crate::tui::views::Breadcrumb;
use crate::tui::views::{format_attention_hint, format_focus_timer_hint};
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
    focus_timer: Option<&FocusTimer>,
    header_notifications: &HeaderNotificationManager,
) {
    let idle_threshold = config.idle_threshold_secs;
    let project = project_store.get_project(project_id);
    let branch = project_store.get_branch(branch_id);

    // Build header
    let attention_count = sessions.attention_count_for_branch(branch_id, idle_threshold);
    let (breadcrumb, suffix) = match (project, branch) {
        (Some(project), Some(branch)) => {
            let active_count = sessions.active_session_count_for_branch(branch_id);

            let bc = Breadcrumb::new().push(&project.name).push(&branch.name);
            let mut status_parts = vec![];
            if active_count > 0 {
                status_parts.push(format!("{} active", active_count));
            }
            if attention_count > 0 {
                status_parts.push(format!("{} need attention", attention_count));
            }
            let suffix = if status_parts.is_empty() {
                String::new()
            } else {
                format!("({})", status_parts.join(", "))
            };
            (bc, suffix)
        }
        _ => (Breadcrumb::new().push("?").push("?"), String::new()),
    };

    let header = Header::new(breadcrumb)
        .with_suffix(suffix)
        .with_timer(focus_timer)
        .with_notifications(Some(header_notifications))
        .with_attention_count(attention_count);

    // Create layout with header and footer
    let areas = ScreenLayout::new(area).with_header(header).render(frame);

    // Main content area - either session creation input, delete confirmation, or session list
    if state.input_mode == InputMode::CreatingSession {
        render_session_creation(frame, areas.content, state, "Claude Code");
    } else if state.input_mode == InputMode::CreatingShellSession {
        render_session_creation(frame, areas.content, state, "Shell");
    } else if state.input_mode == InputMode::ConfirmingSessionDelete {
        render_delete_confirmation(frame, areas.content, state, sessions);
    } else if let Some(branch) = branch {
        let branch_sessions = sessions.sessions_for_branch(branch_id);

        if branch_sessions.is_empty() {
            let empty_text = format!(
                "No sessions on this branch yet.\n\n\
                Press 'n' to create a Claude Code session.\n\
                Press 's' to create a shell session.\n\n\
                Working directory: {}",
                branch.working_dir.display()
            );
            let empty = Paragraph::new(empty_text)
                .style(Style::default().fg(Color::DarkGray))
                .block(Block::default().borders(Borders::ALL).title("Sessions"));
            frame.render_widget(empty, areas.content);
        } else {
            let selected_index = state.selected_session_index;
            let now = Utc::now();

            let items: Vec<ListItem> = branch_sessions
                .iter()
                .enumerate()
                .map(|(i, session)| {
                    let selected = i == selected_index;
                    let prefix = selection_prefix(selected);

                    // Check if session needs attention
                    let needs_attention = sessions.session_needs_attention(session, idle_threshold);

                    // Build state display with idle duration if applicable
                    // Shell sessions show simpler state (Running/Ready instead of Thinking/Executing)
                    let state_display = match (&session.info.session_type, &session.info.state) {
                        (_, SessionState::Idle) => {
                            let duration = now.signed_duration_since(session.info.last_activity);
                            let mins = duration.num_minutes();
                            format!("Idle - {}m", mins)
                        }
                        (SessionType::Shell, SessionState::Executing(_)) => "Running".to_string(),
                        (SessionType::Shell, SessionState::Waiting) => "Ready".to_string(),
                        (_, state) => state.display_name().to_string(),
                    };

                    // Type badge for shell sessions (Claude Code sessions don't need a badge)
                    let type_badge = match session.info.session_type {
                        SessionType::Shell => "󰆍 ", // shell icon or "$ " for terminals without nerd fonts
                        SessionType::ClaudeCode => "",
                    };

                    // Build attention badge
                    let (badge, badge_color) = if needs_attention {
                        match &session.info.state {
                            SessionState::Waiting => ("● ", Color::Green),
                            SessionState::Idle => ("● ", Color::Yellow),
                            _ => ("  ", Color::White),
                        }
                    } else {
                        ("  ", Color::White)
                    };

                    let content = Line::from(vec![
                        Span::raw(prefix),
                        Span::styled(badge, Style::default().fg(badge_color)),
                        Span::raw(type_badge),
                        Span::raw(format!(
                            "{}: {} [{}]",
                            i + 1,
                            session.info.name,
                            state_display
                        )),
                    ]);

                    let style = selection_style(selected, session.info.state.color());

                    ListItem::new(content).style(style)
                })
                .collect();

            let title = format!("Sessions ({})", branch_sessions.len());
            let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
            frame.render_widget(list, areas.content);
        }
    } else {
        let error = Paragraph::new("Branch not found")
            .style(Style::default().fg(Color::Red))
            .block(Block::default().borders(Borders::ALL).title("Error"));
        frame.render_widget(error, areas.content);
    }

    // Footer
    let help_text = match state.input_mode {
        InputMode::CreatingSession | InputMode::CreatingShellSession => {
            "Enter: create | Esc: cancel".to_string()
        }
        InputMode::ConfirmingSessionDelete => "y: confirm delete | n/Esc: cancel".to_string(),
        _ => {
            let timer_hint = format_focus_timer_hint(state.focus_timer.is_some());
            let base = format!(
                "n: claude | s: shell | d: delete | {} | ↑/↓: navigate | Enter: open | Esc/q: back",
                timer_hint
            );
            if let Some(hint) = format_attention_hint(sessions, config) {
                format!("{} | {}", hint, base)
            } else {
                base
            }
        }
    };
    let footer = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, areas.footer());
}

/// Render the session creation input
fn render_session_creation(frame: &mut Frame, area: Rect, state: &AppState, session_type: &str) {
    let t = theme();
    let title = format!("Create {} Session", session_type);
    let input = Paragraph::new(format!("New session name: {}_", state.new_session_name))
        .style(t.input_style())
        .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(input, area);
}

/// Render the delete confirmation dialog
fn render_delete_confirmation(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    sessions: &SessionManager,
) {
    let session = state.pending_delete_session.and_then(|id| sessions.get(id));

    let session_name = session
        .map(|s| s.info.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());

    let warning = session
        .map(|s| match s.info.session_type {
            SessionType::ClaudeCode => "This will kill the Claude Code process.",
            SessionType::Shell => "This will kill the shell process.",
        })
        .unwrap_or("This will kill the process.")
        .to_string();

    let config = ConfirmDialogConfig {
        title: "Confirm Delete",
        item_label: "session",
        item_name: &session_name,
        warnings: vec![warning],
        notes: vec![],
    };
    render_confirm_dialog(frame, area, config);
}
