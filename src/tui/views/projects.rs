//! Projects overview view
//!
//! Displays a list of projects with their branch/session counts,
//! and a "quick sessions" section for sessions in the current directory.

use chrono::Utc;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{AppState, InputMode};
use crate::config::Config;
use crate::project::ProjectStore;
use crate::session::{Session, SessionManager, SessionState};
use crate::tui::theme::theme;
use crate::tui::views::format_attention_hint;
use crate::tui::views::render_project_delete_confirmation;
use crate::tui::views::render_quit_confirm_dialog;

/// Render the projects overview
pub fn render_projects_overview(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
    config: &Config,
) {
    let idle_threshold = config.idle_threshold_secs;
    let attention_count = sessions.total_attention_count(idle_threshold);
    let attention_sessions = sessions.sessions_needing_attention(idle_threshold);
    let has_dropped_events = state.dropped_events_count > 0;
    let has_error = state.error_message.is_some();

    // Build constraints dynamically based on what we need to show
    let mut constraints = vec![Constraint::Length(3)]; // Header

    // Error message banner
    if has_error {
        constraints.push(Constraint::Length(1));
    }

    // Warning banner for dropped events
    if has_dropped_events {
        constraints.push(Constraint::Length(1));
    }

    // Attention section
    if attention_count > 0 && state.input_mode == InputMode::Normal {
        let attention_height = (attention_sessions.len() + 2).min(8) as u16;
        constraints.push(Constraint::Length(attention_height));
    }

    constraints.push(Constraint::Min(0)); // Main content
    constraints.push(Constraint::Length(3)); // Footer

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut chunk_idx = 0;

    // Header
    let active_count = sessions.total_active_count();
    let header_text = {
        let mut parts = vec![format!(
            "Panoptes - {} projects",
            project_store.project_count()
        )];
        if active_count > 0 {
            parts.push(format!("{} active", active_count));
        }
        if attention_count > 0 {
            parts.push(format!("{} need attention", attention_count));
        }
        if parts.len() == 1 {
            parts[0].clone()
        } else {
            format!("{}, {}", parts[0], parts[1..].join(", "))
        }
    };
    let t = theme();
    let header = Paragraph::new(header_text)
        .style(t.header_style())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[chunk_idx]);
    chunk_idx += 1;

    // Error message banner
    if let Some(error_msg) = &state.error_message {
        let error_text = format!("✖ {} (press any key to dismiss)", error_msg);
        let error_banner = Paragraph::new(error_text).style(t.error_banner_style());
        frame.render_widget(error_banner, chunks[chunk_idx]);
        chunk_idx += 1;
    }

    // Dropped events warning banner
    if has_dropped_events {
        let warning_text = format!(
            "⚠ {} hook events dropped due to channel overflow - session states may be inaccurate",
            state.dropped_events_count
        );
        let warning = Paragraph::new(warning_text).style(t.warning_banner_style());
        frame.render_widget(warning, chunks[chunk_idx]);
        chunk_idx += 1;
    }

    // Attention section (if needed and in normal mode)
    if attention_count > 0 && state.input_mode == InputMode::Normal {
        render_attention_section(
            frame,
            chunks[chunk_idx],
            &attention_sessions,
            project_store,
            idle_threshold,
        );
        chunk_idx += 1;
    }

    let main_chunk_index = chunk_idx;

    // Main content area
    match state.input_mode {
        InputMode::CreatingSession => {
            render_session_creation(frame, chunks[main_chunk_index], state);
        }
        InputMode::AddingProject => {
            render_project_addition(frame, chunks[main_chunk_index], state);
        }
        InputMode::AddingProjectName => {
            render_project_name_input(frame, chunks[main_chunk_index], state);
        }
        InputMode::ConfirmingProjectDelete => {
            // Get the project being deleted
            let project = state
                .pending_delete_project
                .and_then(|id| project_store.get_project(id));
            render_project_delete_confirmation(
                frame,
                chunks[main_chunk_index],
                state,
                project,
                sessions,
                config,
            );
        }
        _ => {
            render_main_content(
                frame,
                chunks[main_chunk_index],
                state,
                project_store,
                sessions,
                idle_threshold,
            );
        }
    }

    // Footer with help (always last chunk)
    let footer_index = chunks.len() - 1;
    let help_text = match state.input_mode {
        InputMode::CreatingSession => "Enter: create | Esc: cancel".to_string(),
        InputMode::AddingProject => {
            "Tab: autocomplete | Enter: select/validate | Esc: cancel".to_string()
        }
        InputMode::AddingProjectName => "Enter: create project | Esc: cancel".to_string(),
        InputMode::ConfirmingProjectDelete => "y: confirm delete | n/Esc: cancel".to_string(),
        InputMode::ConfirmingQuit => "y/Enter: quit | n/Esc: cancel".to_string(),
        _ => {
            let base = if project_store.project_count() > 0 {
                "n: new project | d: delete | t: timeline | j/k: navigate | Enter: open | q: quit"
            } else if !sessions.is_empty() {
                "n: new project | t: timeline | j/k: navigate | Enter: open | q: quit"
            } else {
                "n: new project | t: timeline | q: quit"
            };
            if let Some(hint) = format_attention_hint(sessions, config) {
                format!("{} | {}", hint, base)
            } else {
                base.to_string()
            }
        }
    };
    let footer = Paragraph::new(help_text)
        .style(t.muted_style())
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[footer_index]);

    // Render quit confirmation dialog as overlay
    if state.input_mode == InputMode::ConfirmingQuit {
        render_quit_confirm_dialog(frame, area);
    }
}

/// Render the "Needs Attention" section
fn render_attention_section(
    frame: &mut Frame,
    area: Rect,
    attention_sessions: &[&Session],
    project_store: &ProjectStore,
    _idle_threshold_secs: u64,
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

            let (badge_color, state_text) = match &session.info.state {
                SessionState::Waiting => (t.attention_waiting, "[Waiting]".to_string()),
                SessionState::Idle => {
                    let duration = now.signed_duration_since(session.info.last_activity);
                    let mins = duration.num_minutes();
                    (t.attention_idle, format!("[Idle - {}m]", mins))
                }
                _ => (t.text, format!("[{}]", session.info.state.display_name())),
            };

            let content = Line::from(vec![
                Span::styled("● ", Style::default().fg(badge_color)),
                Span::raw(format!(
                    "{} ({}/{}) {}",
                    session.info.name, project_name, branch_name, state_text
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

/// Render the session creation input
fn render_session_creation(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();
    let input = Paragraph::new(format!("New session name: {}_", state.new_session_name))
        .style(t.input_style())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Create Session"),
        );
    frame.render_widget(input, area);
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

        // Calculate visible range with scroll
        let max_visible = 8;
        let total = completions.len();
        let (start, end) = if total <= max_visible {
            (0, total)
        } else {
            // Keep selected item visible with some context
            let half = max_visible / 2;
            let start = if selected_idx < half {
                0
            } else if selected_idx >= total - half {
                total - max_visible
            } else {
                selected_idx - half
            };
            (start, start + max_visible)
        };

        let items: Vec<ListItem> = completions[start..end]
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let actual_idx = start + i;
                let is_selected = actual_idx == selected_idx;
                let prefix = if is_selected { "▶ " } else { "  " };
                let display = crate::path_complete::path_to_display(path);
                let content = format!("{}{}/", prefix, display);

                let style = if is_selected {
                    t.selected_style()
                } else {
                    Style::default().fg(t.text)
                };
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
    idle_threshold_secs: u64,
) {
    let has_projects = project_store.project_count() > 0;
    let has_sessions = !sessions.is_empty();

    if !has_projects && !has_sessions {
        // Empty state
        let t = theme();
        let empty_text = "No projects yet.\n\n\
            Press 'a' to add a git repository as a project,\n\
            or 'n' to create a quick session in the current directory.";
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

        render_project_list(
            frame,
            split[0],
            state,
            project_store,
            sessions,
            idle_threshold_secs,
        );
        render_quick_sessions(
            frame,
            split[1],
            state,
            project_store,
            sessions,
            idle_threshold_secs,
        );
    } else if has_projects {
        render_project_list(
            frame,
            area,
            state,
            project_store,
            sessions,
            idle_threshold_secs,
        );
    } else {
        render_quick_sessions(
            frame,
            area,
            state,
            project_store,
            sessions,
            idle_threshold_secs,
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
    idle_threshold_secs: u64,
) {
    let t = theme();
    let selected_index = state.selected_project_index;
    let projects = project_store.projects_sorted();

    let items: Vec<ListItem> = projects
        .iter()
        .enumerate()
        .map(|(i, project)| {
            let selected = i == selected_index;
            let prefix = if selected { "▶ " } else { "  " };

            // Count branches and active sessions for this project
            let branch_count = project_store.branch_count_for_project(project.id);
            let session_count = sessions.session_count_for_project(project.id);
            let active_count = sessions.active_session_count_for_project(project.id);
            let attention_count =
                sessions.attention_count_for_project(project.id, idle_threshold_secs);

            let status = if active_count > 0 {
                format!("{} branches, {} active", branch_count, active_count)
            } else if session_count > 0 {
                format!("{} branches, {} sessions", branch_count, session_count)
            } else {
                format!("{} branches", branch_count)
            };

            let content = format!("{}{}: {} ({})", prefix, i + 1, project.name, status);

            // Color precedence: attention > active > selected > default
            let style = if selected {
                if attention_count > 0 {
                    Style::default().fg(t.attention_badge).bold()
                } else if active_count > 0 {
                    Style::default().fg(t.active).bold()
                } else {
                    t.selected_style()
                }
            } else if attention_count > 0 {
                Style::default().fg(t.attention_badge)
            } else if active_count > 0 {
                Style::default().fg(t.active)
            } else {
                Style::default().fg(t.text)
            };

            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Projects"));
    frame.render_widget(list, area);
}

/// Render quick sessions (sessions not tied to a specific project)
fn render_quick_sessions(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
    idle_threshold_secs: u64,
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
            let selected = i == selected_index;
            let prefix = if selected { "▶ " } else { "  " };

            // Check if session needs attention
            let needs_attention = sessions.session_needs_attention(session, idle_threshold_secs);

            // Get project/branch info
            let project_name = project_store
                .get_project(session.info.project_id)
                .map(|p| p.name.as_str())
                .unwrap_or("?");
            let branch_name = project_store
                .get_branch(session.info.branch_id)
                .map(|b| b.name.as_str())
                .unwrap_or("?");
            let context = format!("{}/{}", project_name, branch_name);

            // Build the state display with idle duration if applicable
            let state_display = match &session.info.state {
                SessionState::Idle => {
                    let duration = now.signed_duration_since(session.info.last_activity);
                    let mins = duration.num_minutes();
                    format!("Idle - {}m", mins)
                }
                state => state.display_name().to_string(),
            };

            // Build content with attention badge
            let (badge, badge_color) = if needs_attention {
                match &session.info.state {
                    SessionState::Waiting => ("● ", t.attention_waiting),
                    SessionState::Idle => ("● ", t.attention_idle),
                    _ => ("  ", t.text),
                }
            } else {
                ("  ", t.text)
            };

            let content = Line::from(vec![
                Span::raw(prefix),
                Span::styled(badge, Style::default().fg(badge_color)),
                Span::raw(format!(
                    "{}: {} ({}) [{}]",
                    i + 1,
                    session.info.name,
                    context,
                    state_display
                )),
            ]);

            let style = if selected {
                Style::default().fg(session.info.state.color()).bold()
            } else {
                Style::default().fg(session.info.state.color())
            };
            ListItem::new(content).style(style)
        })
        .collect();

    let title = format!("Sessions ({})", session_list.len());
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, area);
}
