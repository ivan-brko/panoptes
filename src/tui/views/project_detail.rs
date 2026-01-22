//! Project detail view
//!
//! Shows branches for a specific project.

use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{AppState, BranchRef, BranchRefType, InputMode, WorktreeCreationType};
use crate::config::Config;
use crate::git::GitOps;
use crate::project::{Project, ProjectId, ProjectStore};
use crate::session::SessionManager;
use crate::tui::theme::theme;
use crate::tui::views::confirm::{render_confirm_dialog, ConfirmDialogConfig};
use crate::tui::views::format_attention_hint;
use crate::tui::views::Breadcrumb;

/// Render the project detail view showing branches
pub fn render_project_detail(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_id: ProjectId,
    project_store: &ProjectStore,
    sessions: &SessionManager,
    config: &Config,
) {
    let idle_threshold = config.idle_threshold_secs;
    let project = project_store.get_project(project_id);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Branch list
            Constraint::Length(3), // Footer
        ])
        .split(area);

    // Header with breadcrumb
    let t = theme();
    let header_text = if let Some(project) = project {
        let active_count = sessions.active_session_count_for_project(project_id);
        let attention_count = sessions.attention_count_for_project(project_id, idle_threshold);

        let breadcrumb = Breadcrumb::new().push(&project.name);
        let mut status_parts = vec![];
        if active_count > 0 {
            status_parts.push(format!("{} active", active_count));
        }
        if attention_count > 0 {
            status_parts.push(format!("{} need attention", attention_count));
        }
        if status_parts.is_empty() {
            breadcrumb.display()
        } else {
            breadcrumb.display_with_suffix(&format!("({})", status_parts.join(", ")))
        }
    } else {
        "Panoptes > ?".to_string()
    };

    let header = Paragraph::new(header_text)
        .style(t.header_style())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    // Branch list, worktree creation dialog, or delete confirmation
    if state.input_mode == InputMode::ConfirmingBranchDelete {
        render_branch_delete_confirmation(frame, chunks[1], state, project_store, sessions, config);
    } else if state.input_mode == InputMode::WorktreeSelectBranch {
        render_worktree_select_branch(frame, chunks[1], state, config);
    } else if state.input_mode == InputMode::WorktreeSelectBase {
        render_worktree_select_base(frame, chunks[1], state, config);
    } else if state.input_mode == InputMode::WorktreeConfirm {
        render_worktree_confirm(frame, chunks[1], state, config);
    } else if state.input_mode == InputMode::CreatingWorktree {
        render_worktree_creation(frame, chunks[1], state, project);
    } else if state.input_mode == InputMode::SelectingDefaultBase {
        render_default_base_selection(frame, chunks[1], state);
    } else if state.input_mode == InputMode::FetchingBranches {
        render_fetching_branches(frame, chunks[1]);
    } else if state.input_mode == InputMode::RenamingProject {
        render_rename_dialog(frame, chunks[1], state);
    } else if let Some(project) = project {
        let branches = project_store.branches_for_project_sorted(project_id);

        if branches.is_empty() {
            let empty = Paragraph::new(
                "No branches tracked yet.\n\n\
                Press 'n' to create a worktree for a branch.",
            )
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::ALL).title("Branches"));
            frame.render_widget(empty, chunks[1]);
        } else {
            let selected_index = state.selected_branch_index;

            // Split branches into local checkout (main worktree) and worktrees
            let local_checkout: Option<&crate::project::Branch> =
                branches.iter().find(|b| b.is_default).copied();
            let worktrees: Vec<&crate::project::Branch> =
                branches.iter().filter(|b| b.is_worktree).copied().collect();

            // Query git for current branch name in the main repo
            // Returns: Some(name) for normal branch, Some("detached HEAD") for detached, or falls back to stored name on error
            let current_branch_display = match GitOps::open(&project.repo_path) {
                Ok(git) => match git.current_branch() {
                    Ok(Some(name)) => Some(name),
                    Ok(None) => Some("detached HEAD".to_string()), // Detached HEAD state
                    Err(_) => None, // Fall back to stored name on error
                },
                Err(_) => None, // Fall back to stored name on error
            };

            let mut items: Vec<ListItem> = Vec::new();
            let mut item_index = 0;

            // Render local checkout section
            if let Some(branch) = local_checkout {
                // Section header
                items.push(
                    ListItem::new("Local checkout:").style(Style::default().fg(Color::DarkGray)),
                );

                let selected = item_index == selected_index;
                let prefix = if selected { "▶ " } else { "  " };

                // Count sessions for this branch
                let active_count = sessions.active_session_count_for_branch(branch.id);
                let attention_count =
                    sessions.attention_count_for_branch(branch.id, idle_threshold);

                // Use the dynamically queried branch name, fall back to stored name on git error
                let display_name = current_branch_display.as_deref().unwrap_or(&branch.name);

                let status = if active_count > 0 {
                    format!("  {} active", active_count)
                } else {
                    String::new()
                };

                let content = format!("{}{}: {}{}", prefix, item_index + 1, display_name, status);

                // Color precedence: Yellow (attention) > Green (active) > Cyan > White
                let style = if selected {
                    if attention_count > 0 {
                        Style::default().fg(Color::Yellow).bold()
                    } else if active_count > 0 {
                        Style::default().fg(Color::Green).bold()
                    } else {
                        Style::default().fg(Color::Cyan).bold()
                    }
                } else if attention_count > 0 {
                    Style::default().fg(Color::Yellow)
                } else if active_count > 0 {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Cyan)
                };

                items.push(ListItem::new(content).style(style));
                item_index += 1;
            }

            // Render separator and worktrees section if there are worktrees
            if !worktrees.is_empty() {
                // Add separator if we showed local checkout
                if local_checkout.is_some() {
                    items.push(
                        ListItem::new("───────────────────────────────────────")
                            .style(Style::default().fg(Color::DarkGray)),
                    );
                }

                // Section header
                items.push(ListItem::new("Worktrees:").style(Style::default().fg(Color::DarkGray)));

                for branch in &worktrees {
                    let selected = item_index == selected_index;
                    let prefix = if selected { "▶ " } else { "  " };

                    // Count sessions for this branch
                    let active_count = sessions.active_session_count_for_branch(branch.id);
                    let attention_count =
                        sessions.attention_count_for_branch(branch.id, idle_threshold);

                    let status = if active_count > 0 {
                        format!("  {} active", active_count)
                    } else {
                        String::new()
                    };

                    let content =
                        format!("{}{}: {}{}", prefix, item_index + 1, branch.name, status);

                    // Color precedence: Yellow (attention) > Green (active) > White
                    let style = if selected {
                        if attention_count > 0 {
                            Style::default().fg(Color::Yellow).bold()
                        } else if active_count > 0 {
                            Style::default().fg(Color::Green).bold()
                        } else {
                            Style::default().fg(Color::White).bold()
                        }
                    } else if attention_count > 0 {
                        Style::default().fg(Color::Yellow)
                    } else if active_count > 0 {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::White)
                    };

                    items.push(ListItem::new(content).style(style));
                    item_index += 1;
                }
            }

            let title = format!("Branches ({})", branches.len());
            let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
            frame.render_widget(list, chunks[1]);
        }
    } else {
        let error = Paragraph::new("Project not found")
            .style(Style::default().fg(Color::Red))
            .block(Block::default().borders(Borders::ALL).title("Error"));
        frame.render_widget(error, chunks[1]);
    }

    // Footer
    let help_text = match state.input_mode {
        InputMode::ConfirmingBranchDelete => {
            "w: toggle worktree deletion | y: confirm | n/Esc: cancel".to_string()
        }
        InputMode::WorktreeSelectBranch => {
            "Type to search/create | ↑/↓: navigate | Enter: select | Esc: cancel".to_string()
        }
        InputMode::WorktreeSelectBase => {
            "Type: filter | ↑/↓: navigate | Enter: confirm | Esc: back".to_string()
        }
        InputMode::WorktreeConfirm => "Enter: create | Esc: back".to_string(),
        InputMode::CreatingWorktree => {
            "Type: name | ↑/↓: select base | s: set default | Enter: create | Esc: cancel"
                .to_string()
        }
        InputMode::SelectingDefaultBase => {
            "Type: filter | ↑/↓: navigate | Enter: set default | Esc: cancel".to_string()
        }
        InputMode::FetchingBranches => "Fetching branches... | Esc: cancel".to_string(),
        InputMode::RenamingProject => "Type: project name | Enter: save | Esc: cancel".to_string(),
        _ => {
            let base =
                "n: new worktree | b: set default base | r: rename | d: delete branch | D: delete project | ↑/↓: navigate | Enter: open | Esc: back | q: quit";
            if let Some(hint) = format_attention_hint(sessions, config) {
                format!("{} | {}", hint, base)
            } else {
                base.to_string()
            }
        }
    };
    let footer = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
}

/// Render the worktree creation dialog with base branch selector
fn render_worktree_creation(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project: Option<&Project>,
) {
    let t = theme();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Branch name input
            Constraint::Length(1), // Instructions
            Constraint::Min(0),    // Base branch list
        ])
        .split(area);

    // Branch name input
    let input_title = if state.new_branch_name.is_empty() {
        "New Branch Name (leave empty to checkout existing)"
    } else {
        "New Branch Name"
    };
    let input_text = format!("> {}_", state.new_branch_name);
    let input = Paragraph::new(input_text)
        .style(t.input_style())
        .block(Block::default().borders(Borders::ALL).title(input_title));
    frame.render_widget(input, chunks[0]);

    // Instructions line
    let fetch_warning = if state.fetch_error.is_some() {
        " (fetch failed, showing cached)"
    } else {
        ""
    };
    let default_info = project
        .and_then(|p| p.default_base_branch.as_ref())
        .map(|b| format!(" Default: {}", b))
        .unwrap_or_default();
    let instructions = format!(
        "Select base branch{}{} | s: set as default",
        fetch_warning, default_info
    );
    let instruction_widget = Paragraph::new(instructions).style(Style::default().fg(t.text_muted));
    frame.render_widget(instruction_widget, chunks[1]);

    // Base branch list
    let items: Vec<ListItem> = state
        .filtered_branch_refs
        .iter()
        .enumerate()
        .map(|(i, branch_ref)| {
            let selected = i == state.base_branch_selector_index;
            let prefix = if selected { "▶ " } else { "  " };
            let type_prefix = match branch_ref.ref_type {
                BranchRefType::Local => "[L]",
                BranchRefType::Remote => "[R]",
            };
            let default_marker = if branch_ref.is_default_base { " *" } else { "" };

            let content = format!(
                "{}{} {}{}",
                prefix, type_prefix, branch_ref.name, default_marker
            );

            let style = if selected {
                if branch_ref.is_default_base {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    t.selected_style()
                }
            } else if branch_ref.is_default_base {
                Style::default().fg(Color::Cyan)
            } else if branch_ref.ref_type == BranchRefType::Local {
                Style::default().fg(t.text)
            } else {
                Style::default().fg(t.text_muted)
            };

            ListItem::new(content).style(style)
        })
        .collect();

    let title = format!("Base Branch ({} options)", state.filtered_branch_refs.len());
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, chunks[2]);
}

/// Render the default base branch selection dialog
fn render_default_base_selection(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Filter input
            Constraint::Min(0),    // Branch list
        ])
        .split(area);

    // Filter input
    let input_text = format!("> {}_", state.new_branch_name);
    let input = Paragraph::new(input_text).style(t.input_style()).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Filter branches"),
    );
    frame.render_widget(input, chunks[0]);

    // Branch list
    let items: Vec<ListItem> = state
        .filtered_branch_refs
        .iter()
        .enumerate()
        .map(|(i, branch_ref)| {
            let selected = i == state.base_branch_selector_index;
            let prefix = if selected { "▶ " } else { "  " };
            let type_prefix = match branch_ref.ref_type {
                BranchRefType::Local => "[L]",
                BranchRefType::Remote => "[R]",
            };
            let default_marker = if branch_ref.is_default_base {
                " (current default)"
            } else {
                ""
            };

            let content = format!(
                "{}{} {}{}",
                prefix, type_prefix, branch_ref.name, default_marker
            );

            let style = if selected {
                if branch_ref.is_default_base {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    t.selected_style()
                }
            } else if branch_ref.is_default_base {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(t.text)
            };

            ListItem::new(content).style(style)
        })
        .collect();

    let title = format!(
        "Select Default Base Branch ({} options)",
        state.filtered_branch_refs.len()
    );
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, chunks[1]);
}

/// Render a spinner while fetching branches
fn render_fetching_branches(frame: &mut Frame, area: Rect) {
    let t = theme();
    let content = Paragraph::new("Fetching branches from remotes...")
        .style(Style::default().fg(t.text_muted))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Please wait"));
    frame.render_widget(content, area);
}

/// Render the project rename dialog
fn render_rename_dialog(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();
    let input_text = format!("> {}_", state.new_project_name);
    let input = Paragraph::new(input_text).style(t.input_style()).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Rename Project"),
    );
    frame.render_widget(input, area);
}

/// Render the project delete confirmation dialog
pub fn render_project_delete_confirmation(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project: Option<&Project>,
    sessions: &SessionManager,
    _config: &Config,
) {
    let project_name = project
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    let project_id = state.pending_delete_project;

    // Count active sessions for this project
    let (session_count, active_count) = if let Some(pid) = project_id {
        (
            sessions.session_count_for_project(pid),
            sessions.active_session_count_for_project(pid),
        )
    } else {
        (0, 0)
    };

    let mut warnings = vec![];
    let mut notes = vec![];

    // Warning about active sessions
    if active_count > 0 {
        warnings.push(format!(
            "{} active session{} will be terminated",
            active_count,
            if active_count == 1 { "" } else { "s" }
        ));
    } else if session_count > 0 {
        notes.push(format!(
            "{} session{} will be removed",
            session_count,
            if session_count == 1 { "" } else { "s" }
        ));
    }

    // Note about worktrees
    notes.push("Git worktrees on disk will NOT be deleted.".to_string());

    let config = ConfirmDialogConfig {
        title: "Confirm Delete",
        item_label: "project",
        item_name: &project_name,
        warnings,
        notes,
    };
    render_confirm_dialog(frame, area, config);
}

/// Render the branch delete confirmation dialog
pub fn render_branch_delete_confirmation(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
    _config: &Config,
) {
    let t = theme();

    let branch = state
        .pending_delete_branch
        .and_then(|id| project_store.get_branch(id));
    let branch_name = branch
        .map(|b| b.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    let is_worktree = branch.map(|b| b.is_worktree).unwrap_or(false);

    // Count sessions for this branch
    let (session_count, active_count) = if let Some(branch_id) = state.pending_delete_branch {
        (
            sessions.session_count_for_branch(branch_id),
            sessions.active_session_count_for_branch(branch_id),
        )
    } else {
        (0, 0)
    };

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Delete branch: ", Style::default().fg(t.text)),
            Span::styled(
                &branch_name,
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("?", Style::default().fg(t.text)),
        ]),
        Line::from(""),
    ];

    // Warning about active sessions
    if active_count > 0 {
        lines.push(Line::from(vec![Span::styled(
            format!(
                "⚠  {} active session{} will be terminated",
                active_count,
                if active_count == 1 { "" } else { "s" }
            ),
            Style::default()
                .fg(t.border_warning)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));
    } else if session_count > 0 {
        lines.push(Line::from(vec![Span::styled(
            format!(
                "{} session{} will be removed",
                session_count,
                if session_count == 1 { "" } else { "s" }
            ),
            Style::default().fg(t.text_muted),
        )]));
        lines.push(Line::from(""));
    }

    // Worktree deletion toggle
    if is_worktree {
        let checkbox = if state.delete_worktree_on_disk {
            "[x]"
        } else {
            "[ ]"
        };
        let checkbox_style = if state.delete_worktree_on_disk {
            Style::default()
                .fg(t.border_warning)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.text_muted)
        };
        lines.push(Line::from(vec![
            Span::styled(checkbox, checkbox_style),
            Span::styled(
                " Also delete worktree from disk",
                Style::default().fg(t.text),
            ),
            Span::styled(" (press w to toggle)", Style::default().fg(t.text_muted)),
        ]));
        if state.delete_worktree_on_disk {
            lines.push(Line::from(vec![Span::styled(
                "    ⚠  This will permanently delete the directory!",
                Style::default()
                    .fg(t.border_warning)
                    .add_modifier(Modifier::BOLD),
            )]));
        }
        lines.push(Line::from(""));
    } else {
        lines.push(Line::from(vec![Span::styled(
            "This branch is not a worktree (tracked branch only)",
            Style::default().fg(t.text_muted),
        )]));
        lines.push(Line::from(""));
    }

    // Confirmation prompt
    lines.push(Line::from(vec![
        Span::styled("Press ", Style::default().fg(t.text)),
        Span::styled(
            "y",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" to confirm, ", Style::default().fg(t.text)),
        Span::styled(
            "n",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" or ", Style::default().fg(t.text)),
        Span::styled(
            "Esc",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" to cancel", Style::default().fg(t.text)),
    ]));

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border_warning))
            .title("Confirm Delete"),
    );

    frame.render_widget(paragraph, area);
}

// ============================================================================
// New Worktree Creation Wizard Render Functions
// ============================================================================

/// Render the WorktreeSelectBranch dialog (Step 1)
///
/// UI Layout:
/// ```text
/// ┌─ Select Branch ─────────────────────────────────────────┐
/// │ Search: feature-auth_                                   │
/// ├─────────────────────────────────────────────────────────┤
/// │ ▸ [L] feature-auth                                      │
/// │   [L] feature-authentication                            │
/// │   [R] origin/feature-auth-v2                            │
/// │   ...                                                   │
/// ├─────────────────────────────────────────────────────────┤
/// │   + Create new branch "feature-auth"                    │
/// └─────────────────────────────────────────────────────────┘
/// ```
fn render_worktree_select_branch(frame: &mut Frame, area: Rect, state: &AppState, config: &Config) {
    let t = theme();
    let _ = config; // May be used later for worktree path preview

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Search input
            Constraint::Length(1), // Instruction text
            Constraint::Min(0),    // Branch list
        ])
        .split(area);

    // Search input
    let search_text = format!("> {}_", state.worktree_search_text);
    let fetch_warning = if state.fetch_error.is_some() {
        " (fetch failed, showing cached)"
    } else {
        ""
    };
    let search_title = format!("Search or type new branch name{}", fetch_warning);
    let search_input = Paragraph::new(search_text)
        .style(t.input_style())
        .block(Block::default().borders(Borders::ALL).title(search_title));
    frame.render_widget(search_input, chunks[0]);

    // Instruction text
    let instruction = "Select existing branch or type a new name to create one";
    let instruction_widget = Paragraph::new(instruction).style(Style::default().fg(t.text_muted));
    frame.render_widget(instruction_widget, chunks[1]);

    // Branch list with "Create new" option
    let filtered_count = state.worktree_filtered_branches.len();
    let has_create_option = !state.worktree_search_text.is_empty();

    let mut items: Vec<ListItem> = state
        .worktree_filtered_branches
        .iter()
        .enumerate()
        .map(|(i, branch)| {
            // Already-tracked branches are greyed out and not selectable
            if branch.is_already_tracked {
                let type_prefix = branch.ref_type.prefix();
                let content = format!("  {} {} (already open)", type_prefix, branch.name);
                let style = Style::default().fg(Color::DarkGray);
                return ListItem::new(content).style(style);
            }

            let selected = i == state.worktree_list_index;
            let prefix = if selected { "▸ " } else { "  " };
            let type_prefix = branch.ref_type.prefix();
            let default_marker = if branch.is_default_base { " *" } else { "" };

            let content = format!(
                "{}{} {}{}",
                prefix, type_prefix, branch.name, default_marker
            );

            let style = if selected {
                if branch.is_default_base {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    t.selected_style()
                }
            } else if branch.is_default_base {
                Style::default().fg(Color::Cyan)
            } else if branch.ref_type == BranchRefType::Local {
                Style::default().fg(t.text)
            } else {
                Style::default().fg(t.text_muted)
            };

            ListItem::new(content).style(style)
        })
        .collect();

    // Add "Create new branch" option if search text is non-empty
    if has_create_option {
        // Add separator
        items.push(
            ListItem::new("───────────────────────────────────────")
                .style(Style::default().fg(t.border)),
        );

        let selected = state.worktree_list_index == filtered_count;
        let prefix = if selected { "▸ " } else { "  " };
        let content = format!(
            "{}+ Create new branch \"{}\"",
            prefix, state.worktree_search_text
        );
        let style = if selected {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green)
        };
        items.push(ListItem::new(content).style(style));
    }

    let title = format!("Select Branch ({} found)", filtered_count);
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, chunks[2]);
}

/// Render the WorktreeSelectBase dialog (Step 2)
///
/// UI Layout:
/// ```text
/// ┌─ Create New Branch ─────────────────────────────────────┐
/// │ Branch name: feature-auth                          [Edit]│
/// ├─────────────────────────────────────────────────────────┤
/// │ Base branch: (search to filter)                         │
/// │ Search: _                                               │
/// ├─────────────────────────────────────────────────────────┤
/// │ ▸ * main (default)                                      │
/// │     develop                                             │
/// │     [R] origin/release-2.0                              │
/// │     ...                                                 │
/// └─────────────────────────────────────────────────────────┘
/// ```
fn render_worktree_select_base(frame: &mut Frame, area: Rect, state: &AppState, _config: &Config) {
    let t = theme();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Branch name display
            Constraint::Length(3), // Search input
            Constraint::Min(0),    // Base branch list
        ])
        .split(area);

    // Branch name display
    let name_text = format!("Branch name: {}", state.worktree_branch_name);
    let name_display = Paragraph::new(name_text)
        .style(Style::default().fg(t.accent))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Create New Branch"),
        );
    frame.render_widget(name_display, chunks[0]);

    // Search input for base branch
    let search_text = format!("> {}_", state.worktree_base_search_text);
    let search_input = Paragraph::new(search_text).style(t.input_style()).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Filter base branches"),
    );
    frame.render_widget(search_input, chunks[1]);

    // Filter branches based on search
    let filtered: Vec<&BranchRef> = if state.worktree_base_search_text.is_empty() {
        state.worktree_all_branches.iter().collect()
    } else {
        let query = state.worktree_base_search_text.to_lowercase();
        state
            .worktree_all_branches
            .iter()
            .filter(|b| b.name.to_lowercase().contains(&query))
            .collect()
    };

    // Base branch list
    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(i, branch)| {
            let selected = i == state.worktree_base_list_index;
            let prefix = if selected { "▸ " } else { "  " };
            let type_prefix = branch.ref_type.prefix();
            let default_marker = if branch.is_default_base {
                " * (default)"
            } else {
                ""
            };

            let content = format!(
                "{}{} {}{}",
                prefix, type_prefix, branch.name, default_marker
            );

            let style = if selected {
                if branch.is_default_base {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    t.selected_style()
                }
            } else if branch.is_default_base {
                Style::default().fg(Color::Cyan)
            } else if branch.ref_type == BranchRefType::Local {
                Style::default().fg(t.text)
            } else {
                Style::default().fg(t.text_muted)
            };

            ListItem::new(content).style(style)
        })
        .collect();

    let title = format!("Select Base Branch ({} options)", filtered.len());
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, chunks[2]);
}

/// Render the WorktreeConfirm dialog (Step 3)
///
/// Shows different content based on WorktreeCreationType.
fn render_worktree_confirm(frame: &mut Frame, area: Rect, state: &AppState, config: &Config) {
    let t = theme();

    // Calculate worktree path for display
    let worktree_path = crate::git::worktree::worktree_path_for_branch(
        &config.worktrees_dir,
        &state.worktree_project_name,
        &state.worktree_branch_name,
    );
    let worktree_display = worktree_path.display().to_string();

    let mut lines = vec![Line::from("")];

    match state.worktree_creation_type {
        WorktreeCreationType::ExistingLocal => {
            lines.push(Line::from(vec![Span::styled(
                "You are about to create a worktree from branch:",
                Style::default().fg(t.text),
            )]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                format!("    {}", state.worktree_branch_name),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            )]));
        }
        WorktreeCreationType::RemoteTracking => {
            let remote_name = state
                .worktree_source_branch
                .as_ref()
                .map(|b| b.name.as_str())
                .unwrap_or("unknown");

            lines.push(Line::from(vec![Span::styled(
                "You are about to create a worktree from:",
                Style::default().fg(t.text),
            )]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                format!("    {}", remote_name),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            )]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                format!(
                    "This will create local branch \"{}\"",
                    state.worktree_branch_name
                ),
                Style::default().fg(t.text),
            )]));
            lines.push(Line::from(vec![Span::styled(
                "tracking the remote.",
                Style::default().fg(t.text),
            )]));
        }
        WorktreeCreationType::NewBranch => {
            let base_name = state
                .worktree_base_branch
                .as_ref()
                .map(|b| b.name.as_str())
                .unwrap_or("unknown");

            lines.push(Line::from(vec![Span::styled(
                "You are about to create a new branch:",
                Style::default().fg(t.text),
            )]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                format!("    {}", state.worktree_branch_name),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            )]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "Branched from:",
                Style::default().fg(t.text),
            )]));
            lines.push(Line::from(vec![Span::styled(
                format!("    {}", base_name),
                Style::default().fg(Color::Cyan),
            )]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "Worktree location:",
        Style::default().fg(t.text),
    )]));
    lines.push(Line::from(vec![Span::styled(
        format!("  {}", worktree_display),
        Style::default().fg(t.text_muted),
    )]));

    lines.push(Line::from(""));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Press ", Style::default().fg(t.text)),
        Span::styled(
            "Enter",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" to create, ", Style::default().fg(t.text)),
        Span::styled(
            "Esc",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" to go back", Style::default().fg(t.text)),
    ]));

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border))
            .title("Create Worktree"),
    );

    frame.render_widget(paragraph, area);
}
