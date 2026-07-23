//! Project detail view
//!
//! Shows branches for a specific project.

use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{AppState, BranchRef, BranchRefType, InputMode, WorktreeCreationType};
use crate::config::Config;
use crate::git::GitOps;
use crate::project::{Branch, Project, ProjectId, ProjectStore};
use crate::session::SessionManager;
use crate::tui::header::Header;
use crate::tui::header_notifications::HeaderNotificationManager;
use crate::tui::layout::ScreenLayout;
use crate::tui::theme::theme;
use crate::tui::views::confirm::{render_confirm_dialog, ConfirmDialogConfig};
use crate::tui::views::{footer_with_attention, render_footer, status_suffix, Breadcrumb};
use crate::tui::widgets::selection::{
    activity_style, selection_prefix, selection_style, selection_style_with_accent,
};

/// Render the project detail view showing branches
#[allow(clippy::too_many_arguments)]
pub fn render_project_detail(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_id: ProjectId,
    project_store: &ProjectStore,
    sessions: &SessionManager,
    config: &Config,
    header_notifications: &HeaderNotificationManager,
) {
    let project = project_store.get_project(project_id);

    // Build header
    let attention_count = sessions.attention_count_for_project(project_id);
    let (breadcrumb, suffix) = if let Some(project) = project {
        let active_count = sessions.active_session_count_for_project(project_id);
        (
            Breadcrumb::new().push(&project.name),
            status_suffix(active_count, attention_count),
        )
    } else {
        (Breadcrumb::new().push("?"), String::new())
    };

    let header = Header::new(breadcrumb)
        .with_suffix(suffix)
        .with_notifications(Some(header_notifications))
        .with_attention_count(attention_count);

    // Create layout with header and footer
    let areas = ScreenLayout::new(area).with_header(header).render(frame);

    // Branch list, worktree creation dialog, or delete confirmation
    if state.input_mode == InputMode::ConfirmingBranchDelete {
        render_branch_delete_confirmation(
            frame,
            areas.content,
            state,
            project_store,
            sessions,
            config,
        );
    } else if state.input_mode == InputMode::WorktreeSelectBranch {
        render_worktree_select_branch(frame, areas.content, state, config);
    } else if state.input_mode == InputMode::WorktreeSelectBase {
        render_worktree_select_base(frame, areas.content, state, config);
    } else if state.input_mode == InputMode::WorktreeConfirm {
        render_worktree_confirm(frame, areas.content, state, config);
    } else if state.input_mode == InputMode::SelectingDefaultBase {
        render_default_base_selection(frame, areas.content, state);
    } else if state.input_mode == InputMode::RenamingProject {
        render_rename_dialog(frame, areas.content, state);
    } else if let Some(project) = project {
        render_branch_list(
            frame,
            areas.content,
            state,
            project,
            project_id,
            project_store,
            sessions,
        );
    } else {
        let t = theme();
        let error = Paragraph::new("Project not found")
            .style(Style::default().fg(t.error_bg))
            .block(Block::default().borders(Borders::ALL).title("Error"));
        frame.render_widget(error, areas.content);
    }

    render_footer(frame, areas.footer(), &footer_text(state, sessions));
}

/// Build the footer help text for the current input mode
fn footer_text(state: &AppState, sessions: &SessionManager) -> String {
    match state.input_mode {
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
        InputMode::SelectingDefaultBase => {
            "Type: filter | ↑/↓: navigate | Enter: set default | Esc: cancel".to_string()
        }
        InputMode::RenamingProject => "Type: project name | Enter: save | Esc: cancel".to_string(),
        InputMode::SelectingClaudeConfig | InputMode::SelectingCodexConfig => {
            "↑/↓: navigate | Enter: select | Esc: cancel".to_string()
        }
        _ => {
            let base =
                "n: new worktree | b: base | c/x: config | r: rename | d: delete | k: shortcuts | q: quit"
                    .to_string();
            footer_with_attention(base, sessions)
        }
    }
}

/// One row of the branch list
struct BranchItem<'a> {
    /// Name to show (may differ from the stored branch name)
    display_name: &'a str,
    /// 1-based list number, matching the digit-jump shortcuts
    number: usize,
    selected: bool,
    active_count: usize,
    attention_count: usize,
    /// Pre-built status suffix (differs between sections)
    status: String,
    /// Worktree directory is missing; overrides every other color
    stale: bool,
    /// Style when the row is quiet and unselected
    fallback: Style,
}

/// Render one branch row with the shared numbering and color cascade
fn branch_item(item: BranchItem) -> ListItem<'static> {
    let t = theme();
    let content = format!(
        "{}{}: {}{}",
        selection_prefix(item.selected),
        item.number,
        item.display_name,
        item.status
    );

    // Color precedence: stale > attention > active > selected > fallback
    let style = if item.stale {
        selection_style(item.selected, Color::Red)
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

/// Render the branch list: local checkout section, then worktrees
#[allow(clippy::too_many_arguments)]
fn render_branch_list(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project: &Project,
    project_id: ProjectId,
    project_store: &ProjectStore,
    sessions: &SessionManager,
) {
    let t = theme();
    let branches = project_store.branches_for_project_sorted(project_id);

    if branches.is_empty() {
        let empty = Paragraph::new(
            "No branches tracked yet.\n\n\
            Press 'n' to create a worktree for a branch.",
        )
        .style(Style::default().fg(t.text_muted))
        .block(Block::default().borders(Borders::ALL).title("Branches"));
        frame.render_widget(empty, area);
        return;
    }

    let selected_index = state.selected_branch_index;

    // Split branches into local checkout (main worktree) and worktrees
    let local_checkout: Option<&Branch> = branches.iter().find(|b| b.is_default).copied();
    let worktrees: Vec<&Branch> = branches.iter().filter(|b| b.is_worktree).copied().collect();

    // Query git for current branch name in the main repo
    // Returns: Some(name) for normal branch, Some("detached HEAD") for detached, or falls back to stored name on error
    let current_branch_display = match GitOps::open(&project.repo_path) {
        Ok(git) => match git.current_branch() {
            Ok(Some(name)) => Some(name),
            Ok(None) => Some("detached HEAD".to_string()), // Detached HEAD state
            Err(_) => None,                                // Fall back to stored name on error
        },
        Err(_) => None, // Fall back to stored name on error
    };

    let mut items: Vec<ListItem> = Vec::new();
    let mut item_index = 0;

    // Render local checkout section
    if let Some(branch) = local_checkout {
        // Section header
        items.push(ListItem::new("Local checkout:").style(t.muted_style()));

        let active_count = sessions.active_session_count_for_branch(branch.id);
        let attention_count = sessions.attention_count_for_branch(branch.id);
        let status = if active_count > 0 {
            format!("  {} active", active_count)
        } else {
            String::new()
        };

        items.push(branch_item(BranchItem {
            // Use the dynamically queried branch name, fall back to stored name on git error
            display_name: current_branch_display.as_deref().unwrap_or(&branch.name),
            number: item_index + 1,
            selected: item_index == selected_index,
            active_count,
            attention_count,
            status,
            stale: false,
            fallback: Style::default().fg(t.accent),
        }));
        item_index += 1;
    }

    // Render separator and worktrees section if there are worktrees
    if !worktrees.is_empty() {
        // Add separator if we showed local checkout
        if local_checkout.is_some() {
            items.push(
                ListItem::new("───────────────────────────────────────").style(t.muted_style()),
            );
        }

        // Section header
        items.push(ListItem::new("Worktrees:").style(t.muted_style()));

        for branch in &worktrees {
            let active_count = sessions.active_session_count_for_branch(branch.id);
            let attention_count = sessions.attention_count_for_branch(branch.id);

            let mut status_parts = Vec::new();
            if active_count > 0 {
                status_parts.push(format!("{} active", active_count));
            }
            if branch.stale {
                status_parts.push("⚠ missing".to_string());
            }
            let status = if status_parts.is_empty() {
                String::new()
            } else {
                format!("  ({})", status_parts.join(", "))
            };

            items.push(branch_item(BranchItem {
                display_name: &branch.name,
                number: item_index + 1,
                selected: item_index == selected_index,
                active_count,
                attention_count,
                status,
                stale: branch.stale,
                fallback: Style::default().fg(t.text),
            }));
            item_index += 1;
        }
    }

    let title = format!("Branches ({})", branches.len());
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, area);
}

/// Render one row of a branch-ref selector list
///
/// Shared by the default-base selector and the worktree wizard steps. They
/// all build `{prefix}{[L]/[R]} {name}{default marker}{suffix}` with the same
/// selected/default/local/remote color cascade, but differ in the default
/// marker text, whether remote refs are dimmed, and the style of a selected
/// non-default row.
fn branch_ref_item(
    branch: &BranchRef,
    selected: bool,
    default_marker: &'static str,
    suffix: &str,
    dim_remote: bool,
    selected_plain: Style,
) -> ListItem<'static> {
    let t = theme();

    let content = format!(
        "{}{} {}{}{}",
        selection_prefix(selected),
        branch.ref_type.prefix(),
        branch.name,
        if branch.is_default_base {
            default_marker
        } else {
            ""
        },
        suffix
    );

    let style = if selected {
        if branch.is_default_base {
            selection_style(true, t.accent)
        } else {
            selected_plain
        }
    } else if branch.is_default_base {
        Style::default().fg(t.accent)
    } else if dim_remote && branch.ref_type == BranchRefType::Remote {
        Style::default().fg(t.text_muted)
    } else {
        Style::default().fg(t.text)
    };

    ListItem::new(content).style(style)
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
            branch_ref_item(
                branch_ref,
                i == state.base_branch_selector_index,
                " (current default)",
                "",
                false,
                selection_style_with_accent(true, t),
            )
        })
        .collect();

    let title = format!(
        "Select Default Base Branch ({} options)",
        state.filtered_branch_refs.len()
    );
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, chunks[1]);
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
        warnings,
        notes,
        ..ConfirmDialogConfig::new("Confirm Delete", "project", &project_name)
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

    let mut warnings = Vec::new();
    let mut notes = Vec::new();

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

    // Non-worktree info (shown before confirmation prompt)
    if !is_worktree {
        notes.push("This branch is not a worktree (tracked branch only)".to_string());
    }

    // Worktree deletion toggle (shown after confirmation prompt)
    let mut extra_lines = Vec::new();
    if is_worktree {
        extra_lines.push(Line::from(""));
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
        extra_lines.push(Line::from(vec![
            Span::styled(checkbox, checkbox_style),
            Span::styled(
                " Also delete worktree from disk",
                Style::default().fg(t.text),
            ),
            Span::styled(" (press w to toggle)", Style::default().fg(t.text_muted)),
        ]));
        if state.delete_worktree_on_disk {
            extra_lines.push(Line::from(vec![Span::styled(
                "    ⚠  This will permanently delete the directory!",
                Style::default()
                    .fg(t.border_warning)
                    .add_modifier(Modifier::BOLD),
            )]));
        }
    }

    let config = ConfirmDialogConfig {
        warnings,
        notes,
        extra_lines,
        ..ConfirmDialogConfig::new("Confirm Delete", "branch", &branch_name)
    };
    render_confirm_dialog(frame, area, config);
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

    // Determine if we need to show a validation error
    let has_validation_error = state.worktree_wizard.branch_validation_error.is_some();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),                                        // Search input
            Constraint::Length(1),                                        // Instruction text
            Constraint::Length(if has_validation_error { 1 } else { 0 }), // Validation error (conditional)
            Constraint::Min(0),                                           // Branch list
        ])
        .split(area);

    // Search input
    let search_text = format!("> {}_", state.worktree_wizard.search_text);
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

    // Validation error (if any)
    if let Some(error) = &state.worktree_wizard.branch_validation_error {
        let error_text = format!("⚠ {}", error);
        let error_widget = Paragraph::new(error_text).style(Style::default().fg(t.error_bg));
        frame.render_widget(error_widget, chunks[2]);
    }

    // Branch list with "Create new" option
    let filtered_count = state.worktree_wizard.filtered_branches.len();
    let has_create_option = !state.worktree_wizard.search_text.is_empty();

    let mut items: Vec<ListItem> = state
        .worktree_wizard
        .filtered_branches
        .iter()
        .enumerate()
        .map(|(i, branch)| {
            // Already-tracked branches are greyed out and not selectable
            if branch.is_already_tracked {
                let content = format!(
                    "  {} {} (already open)",
                    branch.ref_type.prefix(),
                    branch.name
                );
                return ListItem::new(content).style(Style::default().fg(t.text_muted));
            }

            let selected = i == state.worktree_wizard.list_index;

            // Branches with untracked git worktrees are highlighted in yellow
            if branch.has_git_worktree {
                let content = format!(
                    "{}{} {}{} (has worktree)",
                    selection_prefix(selected),
                    branch.ref_type.prefix(),
                    branch.name,
                    if branch.is_default_base { " *" } else { "" },
                );
                return ListItem::new(content).style(selection_style(selected, Color::Yellow));
            }

            branch_ref_item(branch, selected, " *", "", true, t.selected_style())
        })
        .collect();

    // Add "Create new branch" option if search text is non-empty
    if has_create_option {
        // Add separator
        items.push(
            ListItem::new("───────────────────────────────────────")
                .style(Style::default().fg(t.border)),
        );

        let selected = state.worktree_wizard.list_index == filtered_count;
        let content = format!(
            "{}+ Create new branch \"{}\"",
            selection_prefix(selected),
            state.worktree_wizard.search_text
        );
        items.push(ListItem::new(content).style(selection_style(selected, Color::Green)));
    }

    let title = format!("Select Branch ({} found)", filtered_count);
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, chunks[3]);
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
    let name_text = format!("Branch name: {}", state.worktree_wizard.branch_name);
    let name_display = Paragraph::new(name_text)
        .style(Style::default().fg(t.accent))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Create New Branch"),
        );
    frame.render_widget(name_display, chunks[0]);

    // Search input for base branch
    let search_text = format!("> {}_", state.worktree_wizard.base_search_text);
    let search_input = Paragraph::new(search_text).style(t.input_style()).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Filter base branches"),
    );
    frame.render_widget(search_input, chunks[1]);

    // Base branch list; the input handlers keep the cached filter in step
    // with the search text
    let items: Vec<ListItem> = state
        .worktree_wizard
        .filtered_base_branches
        .iter()
        .enumerate()
        .map(|(i, branch)| {
            branch_ref_item(
                branch,
                i == state.worktree_wizard.base_list_index,
                " * (default)",
                "",
                true,
                t.selected_style(),
            )
        })
        .collect();

    let title = format!(
        "Select Base Branch ({} options)",
        state.worktree_wizard.filtered_base_branches.len()
    );
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
        &state.worktree_wizard.project_name,
        &state.worktree_wizard.branch_name,
    );
    let worktree_display = worktree_path.display().to_string();

    let mut lines = vec![Line::from("")];

    match state.worktree_wizard.creation_type {
        WorktreeCreationType::ExistingLocal => {
            lines.push(Line::from(vec![Span::styled(
                "You are about to create a worktree from branch:",
                Style::default().fg(t.text),
            )]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                format!("    {}", state.worktree_wizard.branch_name),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            )]));
        }
        WorktreeCreationType::RemoteTracking => {
            let remote_name = state
                .worktree_wizard
                .source_branch
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
                    state.worktree_wizard.branch_name
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
                .worktree_wizard
                .base_branch
                .as_ref()
                .map(|b| b.name.as_str())
                .unwrap_or("unknown");

            lines.push(Line::from(vec![Span::styled(
                "You are about to create a new branch:",
                Style::default().fg(t.text),
            )]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                format!("    {}", state.worktree_wizard.branch_name),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            )]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "Branched from:",
                Style::default().fg(t.text),
            )]));
            lines.push(Line::from(vec![Span::styled(
                format!("    {}", base_name),
                Style::default().fg(t.accent),
            )]));
        }
        WorktreeCreationType::ImportExisting => {
            lines.push(Line::from(vec![Span::styled(
                "Import existing worktree for branch:",
                Style::default().fg(t.text),
            )]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                format!("    {}", state.worktree_wizard.branch_name),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "This worktree already exists in git but is not",
                Style::default().fg(t.text_muted),
            )]));
            lines.push(Line::from(vec![Span::styled(
                "tracked by Panoptes. Importing will add it to",
                Style::default().fg(t.text_muted),
            )]));
            lines.push(Line::from(vec![Span::styled(
                "your project without modifying the worktree.",
                Style::default().fg(t.text_muted),
            )]));
        }
    }

    // For ImportExisting, we need to find the actual worktree path from git
    let is_import = state.worktree_wizard.creation_type == WorktreeCreationType::ImportExisting;

    if !is_import {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "Worktree location:",
            Style::default().fg(t.text),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!("  {}", worktree_display),
            Style::default().fg(t.text_muted),
        )]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(""));

    let action_text = if is_import {
        " to import, "
    } else {
        " to create, "
    };
    lines.push(Line::from(vec![
        Span::styled("Press ", Style::default().fg(t.text)),
        Span::styled(
            "Enter",
            Style::default()
                .fg(t.confirm_key)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(action_text, Style::default().fg(t.text)),
        Span::styled(
            "Esc",
            Style::default()
                .fg(t.cancel_key)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" to go back", Style::default().fg(t.text)),
    ]));

    let title = if is_import {
        "Import Worktree"
    } else {
        "Create Worktree"
    };
    let paragraph = Paragraph::new(lines).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border))
            .title(title),
    );

    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Branch;
    use crate::session::store::SessionStore;
    use crate::tui::header_notifications::HeaderNotificationManager;
    use crate::tui::views::test_util::{contains_line, render_to_lines};
    use crate::wizards::worktree::BranchRefType as WizardRefType;
    use std::path::PathBuf;

    /// A store with one project ("panoptes"), its local checkout and a worktree
    fn store_with_project() -> (ProjectStore, ProjectId) {
        let mut store = ProjectStore::new();
        let project = Project::new(
            "panoptes".to_string(),
            PathBuf::from("/tmp/panoptes"),
            "main".to_string(),
        );
        let project_id = project.id;
        store.add_project(project);
        store.add_branch(Branch::default_for_project(
            project_id,
            "main".to_string(),
            PathBuf::from("/tmp/panoptes"),
        ));
        store.add_branch(Branch::new(
            project_id,
            "feature-x".to_string(),
            PathBuf::from("/tmp/worktrees/feature-x"),
            false,
            true,
        ));
        (store, project_id)
    }

    fn render_detail(state: &AppState, store: &ProjectStore, project_id: ProjectId) -> Vec<String> {
        let config = Config::default();
        let sessions = SessionManager::with_store(config.clone(), SessionStore::new());
        let header_notifications = HeaderNotificationManager::default();

        render_to_lines(80, 24, |frame| {
            render_project_detail(
                frame,
                frame.size(),
                state,
                project_id,
                store,
                &sessions,
                &config,
                &header_notifications,
            )
        })
    }

    #[test]
    fn test_branch_list_shows_local_checkout_and_worktrees() {
        let (store, project_id) = store_with_project();
        let state = AppState::default();

        let lines = render_detail(&state, &store, project_id);

        assert!(contains_line(&lines, "Branches (2)"), "{:?}", lines);
        assert!(contains_line(&lines, "Local checkout:"), "{:?}", lines);
        assert!(contains_line(&lines, "Worktrees:"), "{:?}", lines);
        assert!(contains_line(&lines, "feature-x"), "{:?}", lines);
        assert!(contains_line(&lines, "n: new worktree"), "{:?}", lines);
    }

    #[test]
    fn test_wizard_select_branch_lists_branches_and_create_option() {
        let (store, project_id) = store_with_project();
        let mut state = AppState {
            input_mode: InputMode::WorktreeSelectBranch,
            ..Default::default()
        };
        state.worktree_wizard.search_text = "feat".to_string();
        state.worktree_wizard.filtered_branches = vec![crate::wizards::worktree::BranchRef::new(
            WizardRefType::Local,
            "feature-y".to_string(),
        )];

        let lines = render_detail(&state, &store, project_id);

        assert!(
            contains_line(&lines, "Select Branch (1 found)"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "[L] feature-y"), "{:?}", lines);
        assert!(
            contains_line(&lines, "+ Create new branch \"feat\""),
            "{:?}",
            lines
        );
    }

    #[test]
    fn test_wizard_select_base_reads_filtered_cache() {
        let (store, project_id) = store_with_project();
        let mut state = AppState {
            input_mode: InputMode::WorktreeSelectBase,
            ..Default::default()
        };
        state.worktree_wizard.branch_name = "new-branch".to_string();
        state.worktree_wizard.filtered_base_branches =
            vec![crate::wizards::worktree::BranchRef::new(
                WizardRefType::Local,
                "main".to_string(),
            )
            .with_default_base(true)];

        let lines = render_detail(&state, &store, project_id);

        assert!(
            contains_line(&lines, "Branch name: new-branch"),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "Select Base Branch (1 options)"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "[L] main * (default)"), "{:?}", lines);
    }

    #[test]
    fn test_wizard_confirm_shows_worktree_location() {
        let (store, project_id) = store_with_project();
        let mut state = AppState {
            input_mode: InputMode::WorktreeConfirm,
            ..Default::default()
        };
        state.worktree_wizard.branch_name = "feature-z".to_string();
        state.worktree_wizard.project_name = "panoptes".to_string();
        state.worktree_wizard.creation_type = WorktreeCreationType::ExistingLocal;

        let lines = render_detail(&state, &store, project_id);

        assert!(contains_line(&lines, "Create Worktree"), "{:?}", lines);
        assert!(
            contains_line(&lines, "You are about to create a worktree from branch:"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "feature-z"), "{:?}", lines);
        assert!(contains_line(&lines, "Worktree location:"), "{:?}", lines);
    }

    #[test]
    fn test_default_base_selection_marks_current_default() {
        let (store, project_id) = store_with_project();
        let state = AppState {
            input_mode: InputMode::SelectingDefaultBase,
            filtered_branch_refs: vec![
                BranchRef::new(WizardRefType::Local, "main".to_string()).with_default_base(true),
                BranchRef::new(WizardRefType::Remote, "origin/dev".to_string()),
            ],
            ..Default::default()
        };

        let lines = render_detail(&state, &store, project_id);

        assert!(
            contains_line(&lines, "Select Default Base Branch (2 options)"),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "[L] main (current default)"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "[R] origin/dev"), "{:?}", lines);
    }

    #[test]
    fn test_branch_delete_confirmation_offers_worktree_toggle() {
        let (store, project_id) = store_with_project();
        let worktree_branch_id = store
            .branches_for_project_sorted(project_id)
            .iter()
            .find(|b| b.is_worktree)
            .map(|b| b.id)
            .unwrap();
        let state = AppState {
            input_mode: InputMode::ConfirmingBranchDelete,
            pending_delete_branch: Some(worktree_branch_id),
            ..Default::default()
        };

        let lines = render_detail(&state, &store, project_id);

        assert!(
            contains_line(&lines, "Delete branch: feature-x?"),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "Press y to confirm, n or Esc to cancel"),
            "{:?}",
            lines
        );
        assert!(
            contains_line(
                &lines,
                "[ ] Also delete worktree from disk (press w to toggle)"
            ),
            "{:?}",
            lines
        );
    }
}
