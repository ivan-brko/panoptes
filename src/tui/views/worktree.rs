//! Worktree flows, and the delete confirmations that belong with them
//!
//! Every one of these shows a list or a paragraph, so all of them are centred
//! overlays anchored to the terminal rather than to a pane.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

use crate::app::{AppState, BranchRef, BranchRefType, InputMode, WorktreeCreationType};
use crate::config::Config;
use crate::project::{Project, ProjectStore};
use crate::session::SessionManager;
use crate::tui::theme::theme;
use crate::tui::views::confirm::{render_confirm_dialog, ConfirmDialogConfig};
use crate::tui::widgets::dialog::{centered_rect, render_dialog, DialogSize, DialogSpec};
use crate::tui::widgets::selection::{
    selection_prefix, selection_style, selection_style_with_accent,
};

/// Size of the list-shaped overlays (wizard steps, base selector)
const SELECTOR_WIDTH: DialogSize = DialogSize::Percent {
    pct: 70,
    min: 40,
    max: 80,
};
const SELECTOR_HEIGHT: DialogSize = DialogSize::Percent {
    pct: 70,
    min: 10,
    max: 26,
};

/// Render whichever worktree wizard step is open
pub fn render_worktree_wizard(frame: &mut Frame, area: Rect, state: &AppState, config: &Config) {
    match state.input_mode {
        InputMode::WorktreeSelectBranch => render_select_branch(frame, area, state),
        InputMode::WorktreeSelectBase => render_select_base(frame, area, state),
        InputMode::WorktreeConfirm => render_confirm(frame, area, state, config),
        _ => {}
    }
}

/// A centred overlay split into a header box and a list box
fn render_selector_overlay(
    frame: &mut Frame,
    area: Rect,
    header_lines: Vec<Line<'static>>,
    list_title: String,
    items: Vec<ListItem<'static>>,
) {
    let t = theme();
    let overlay = centered_rect(area, SELECTOR_WIDTH, SELECTOR_HEIGHT);
    frame.render_widget(Clear, overlay);

    let header_height = header_lines.len() as u16 + 2;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(header_height), Constraint::Min(3)])
        .split(overlay);

    frame.render_widget(
        Paragraph::new(header_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.accent)),
        ),
        chunks[0],
    );
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.border_focused))
                .title(list_title),
        ),
        chunks[1],
    );
}

/// One row of a branch-ref selector list
///
/// Shared by the default-base selector and the wizard steps: they all build
/// `{prefix}{[L]/[R]} {name}{default marker}{suffix}` with the same
/// selected/default/local/remote color cascade, and differ only in the default
/// marker text, whether remote refs are dimmed, and the style of a selected
/// non-default row.
fn branch_ref_item(
    branch: &BranchRef,
    selected: bool,
    default_marker: &'static str,
    dim_remote: bool,
    selected_plain: Style,
) -> ListItem<'static> {
    let t = theme();

    let content = format!(
        "{}{} {}{}",
        selection_prefix(selected),
        branch.ref_type.prefix(),
        branch.name,
        if branch.is_default_base {
            default_marker
        } else {
            ""
        },
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

/// Step 1: search for an existing branch, or type a new name
fn render_select_branch(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();
    let wizard = &state.worktree_wizard;

    let mut header = vec![
        Line::from(vec![
            Span::styled("Search or new branch name", t.muted_style()),
            Span::styled(
                if state.fetch_error.is_some() {
                    " (fetch failed, showing cached)"
                } else {
                    ""
                },
                Style::default().fg(t.border_warning),
            ),
        ]),
        Line::from(Span::styled(
            format!("> {}_", wizard.search_text),
            t.input_style(),
        )),
    ];
    if let Some(error) = &wizard.branch_validation_error {
        header.push(Line::from(Span::styled(
            format!("⚠ {}", error),
            Style::default().fg(t.error_bg),
        )));
    }

    let filtered_count = wizard.filtered_branches.len();
    let has_create_option = !wizard.search_text.is_empty();

    let mut items: Vec<ListItem> = wizard
        .filtered_branches
        .iter()
        .enumerate()
        .map(|(i, branch)| {
            // Already-tracked branches are greyed out and not selectable
            if branch.is_already_tracked {
                return ListItem::new(format!(
                    "  {} {} (already open)",
                    branch.ref_type.prefix(),
                    branch.name
                ))
                .style(Style::default().fg(t.text_muted));
            }

            let selected = i == wizard.list_index;

            // Branches with untracked git worktrees are highlighted in yellow
            if branch.has_git_worktree {
                return ListItem::new(format!(
                    "{}{} {}{} (has worktree)",
                    selection_prefix(selected),
                    branch.ref_type.prefix(),
                    branch.name,
                    if branch.is_default_base { " *" } else { "" },
                ))
                .style(selection_style(selected, Color::Yellow));
            }

            branch_ref_item(branch, selected, " *", true, t.selected_style())
        })
        .collect();

    if has_create_option {
        items.push(ListItem::new("─".repeat(30)).style(Style::default().fg(t.border)));
        let selected = wizard.list_index == filtered_count;
        items.push(
            ListItem::new(format!(
                "{}+ Create new branch \"{}\"",
                selection_prefix(selected),
                wizard.search_text
            ))
            .style(selection_style(selected, Color::Green)),
        );
    }

    render_selector_overlay(
        frame,
        area,
        header,
        format!("Select Branch ({} found)", filtered_count),
        items,
    );
}

/// Step 2: pick the base for a new branch
fn render_select_base(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();
    let wizard = &state.worktree_wizard;

    let header = vec![
        Line::from(vec![
            Span::styled("Branch name: ", t.muted_style()),
            Span::styled(
                wizard.branch_name.clone(),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            format!("> {}_", wizard.base_search_text),
            t.input_style(),
        )),
    ];

    // The input handlers keep the cached filter in step with the search text
    let items: Vec<ListItem> = wizard
        .filtered_base_branches
        .iter()
        .enumerate()
        .map(|(i, branch)| {
            branch_ref_item(
                branch,
                i == wizard.base_list_index,
                " * (default)",
                true,
                t.selected_style(),
            )
        })
        .collect();

    render_selector_overlay(
        frame,
        area,
        header,
        format!(
            "Select Base Branch ({} options)",
            wizard.filtered_base_branches.len()
        ),
        items,
    );
}

/// Step 3: say what is about to happen, and where
fn render_confirm(frame: &mut Frame, area: Rect, state: &AppState, config: &Config) {
    let t = theme();
    let wizard = &state.worktree_wizard;

    let worktree_path = crate::git::worktree::worktree_path_for_branch(
        &config.worktrees_dir,
        &wizard.project_name,
        &wizard.branch_name,
    );

    let mut lines = vec![Line::from("")];
    let is_import = wizard.creation_type == WorktreeCreationType::ImportExisting;

    match wizard.creation_type {
        WorktreeCreationType::ExistingLocal => {
            lines.push(Line::from(Span::styled(
                "You are about to create a worktree from branch:",
                Style::default().fg(t.text),
            )));
            lines.push(Line::from(""));
            lines.push(emphasis(&wizard.branch_name, t.accent));
        }
        WorktreeCreationType::RemoteTracking => {
            let remote_name = wizard
                .source_branch
                .as_ref()
                .map(|b| b.name.as_str())
                .unwrap_or("unknown");
            lines.push(Line::from(Span::styled(
                "You are about to create a worktree from:",
                Style::default().fg(t.text),
            )));
            lines.push(Line::from(""));
            lines.push(emphasis(remote_name, t.accent));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!(
                    "This will create local branch \"{}\" tracking the remote.",
                    wizard.branch_name
                ),
                Style::default().fg(t.text),
            )));
        }
        WorktreeCreationType::NewBranch => {
            let base_name = wizard
                .base_branch
                .as_ref()
                .map(|b| b.name.as_str())
                .unwrap_or("unknown");
            lines.push(Line::from(Span::styled(
                "You are about to create a new branch:",
                Style::default().fg(t.text),
            )));
            lines.push(Line::from(""));
            lines.push(emphasis(&wizard.branch_name, t.accent));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Branched from:",
                Style::default().fg(t.text),
            )));
            lines.push(emphasis(base_name, t.accent));
        }
        WorktreeCreationType::ImportExisting => {
            lines.push(Line::from(Span::styled(
                "Import existing worktree for branch:",
                Style::default().fg(t.text),
            )));
            lines.push(Line::from(""));
            lines.push(emphasis(&wizard.branch_name, Color::Yellow));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "This worktree already exists in git but is not tracked by",
                t.muted_style(),
            )));
            lines.push(Line::from(Span::styled(
                "Panoptes. Importing adds it without modifying the worktree.",
                t.muted_style(),
            )));
        }
    }

    if !is_import {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Worktree location:",
            Style::default().fg(t.text),
        )));
        lines.push(Line::from(Span::styled(
            worktree_path.display().to_string(),
            t.muted_style(),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Press ", Style::default().fg(t.text)),
        Span::styled(
            "Enter",
            Style::default()
                .fg(t.confirm_key)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            if is_import {
                " to import, "
            } else {
                " to create, "
            },
            Style::default().fg(t.text),
        ),
        Span::styled(
            "Esc",
            Style::default()
                .fg(t.cancel_key)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" to go back", Style::default().fg(t.text)),
    ]));

    let height = lines.len() as u16 + 2;
    render_dialog(
        frame,
        area,
        DialogSpec {
            title: if is_import {
                " Import Worktree "
            } else {
                " Create Worktree "
            },
            border_color: t.accent,
            alignment: Alignment::Center,
            width: SELECTOR_WIDTH,
            height: DialogSize::Fixed(height),
        },
        lines,
    );
}

fn emphasis(text: &str, color: Color) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
}

/// The default-base-branch selector, opened from per-project settings
pub fn render_default_base_selector(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();

    let header = vec![
        Line::from(Span::styled(
            "New worktrees branch from this ref by default.",
            t.muted_style(),
        )),
        Line::from(Span::styled(
            format!("> {}_", state.new_branch_name),
            t.input_style(),
        )),
    ];

    let items: Vec<ListItem> = state
        .filtered_branch_refs
        .iter()
        .enumerate()
        .map(|(i, branch_ref)| {
            branch_ref_item(
                branch_ref,
                i == state.base_branch_selector_index,
                " (current default)",
                false,
                selection_style_with_accent(true, t),
            )
        })
        .collect();

    render_selector_overlay(
        frame,
        area,
        header,
        format!(
            "Select Default Base Branch ({} options)",
            state.filtered_branch_refs.len()
        ),
        items,
    );
}

/// The project delete confirmation
pub fn render_project_delete_confirmation(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project: Option<&Project>,
    sessions: &SessionManager,
) {
    let project_name = project
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());

    let (session_count, active_count) = match state.pending_delete_project {
        Some(pid) => (
            sessions.session_count_for_project(pid),
            sessions.active_session_count_for_project(pid),
        ),
        None => (0, 0),
    };

    let mut warnings = Vec::new();
    let mut notes = Vec::new();

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
    notes.push("Git worktrees on disk will NOT be deleted.".to_string());

    let height = 10 + (warnings.len() + notes.len()) as u16 * 2;
    let config = ConfirmDialogConfig {
        warnings,
        notes,
        overlay: Some((DialogSize::Fixed(64), DialogSize::Fixed(height))),
        ..ConfirmDialogConfig::new("Confirm Delete", "project", &project_name)
    };
    render_confirm_dialog(frame, area, config);
}

/// The branch/worktree delete confirmation, with its on-disk toggle
pub fn render_branch_delete_confirmation(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &ProjectStore,
    sessions: &SessionManager,
) {
    let t = theme();

    let branch = state
        .pending_delete_branch
        .and_then(|id| project_store.get_branch(id));
    let branch_name = branch
        .map(|b| b.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    let is_worktree = branch.map(|b| b.is_worktree).unwrap_or(false);

    let (session_count, active_count) = match state.pending_delete_branch {
        Some(branch_id) => (
            sessions.session_count_for_branch(branch_id),
            sessions.active_session_count_for_branch(branch_id),
        ),
        None => (0, 0),
    };

    let mut warnings = Vec::new();
    let mut notes = Vec::new();

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

    // Whichever it is, the git branch itself survives - say so plainly
    if is_worktree {
        notes.push("The git branch itself is NOT deleted.".to_string());
    } else {
        notes.push("Not a worktree: this only removes it from Panoptes.".to_string());
    }

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
                " Also delete the worktree directory from disk",
                Style::default().fg(t.text),
            ),
            Span::styled(" (press w to toggle)", Style::default().fg(t.text_muted)),
        ]));
        if state.delete_worktree_on_disk {
            extra_lines.push(Line::from(Span::styled(
                "⚠  This permanently deletes the directory!",
                Style::default()
                    .fg(t.border_warning)
                    .add_modifier(Modifier::BOLD),
            )));
        }
    }

    let item_label = if is_worktree { "worktree" } else { "branch" };
    let height = 8 + (warnings.len() + notes.len()) as u16 * 2 + extra_lines.len() as u16;
    let config = ConfirmDialogConfig {
        warnings,
        notes,
        extra_lines,
        overlay: Some((DialogSize::Fixed(68), DialogSize::Fixed(height))),
        ..ConfirmDialogConfig::new("Confirm Delete", item_label, &branch_name)
    };
    render_confirm_dialog(frame, area, config);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Branch, ProjectId};
    use crate::session::store::SessionStore;
    use crate::tui::views::test_util::{contains_line, render_to_lines};
    use crate::wizards::worktree::BranchRefType as WizardRefType;
    use std::path::PathBuf;

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

    fn render_wizard(state: &AppState) -> Vec<String> {
        let config = Config::default();
        render_to_lines(120, 30, |frame| {
            render_worktree_wizard(frame, frame.size(), state, &config)
        })
    }

    #[test]
    fn test_select_branch_lists_branches_and_the_create_option() {
        let mut state = AppState {
            input_mode: InputMode::WorktreeSelectBranch,
            ..Default::default()
        };
        state.worktree_wizard.search_text = "feat".to_string();
        state.worktree_wizard.filtered_branches = vec![BranchRef::new(
            WizardRefType::Local,
            "feature-y".to_string(),
        )];

        let lines = render_wizard(&state);

        assert!(
            contains_line(&lines, "Select Branch (1 found)"),
            "{lines:?}"
        );
        assert!(contains_line(&lines, "[L] feature-y"), "{lines:?}");
        assert!(
            contains_line(&lines, "+ Create new branch \"feat\""),
            "{lines:?}"
        );
        // Centred overlay, not a pane: the top row stays clear
        assert!(lines[0].is_empty(), "{lines:?}");
    }

    /// Same guard as `prompts.rs`: these are centred overlays with `Clear`,
    /// which panics rather than clipping when the rect escapes the buffer
    #[test]
    fn test_worktree_overlays_survive_a_terminal_smaller_than_they_want() {
        let (store, project_id) = store_with_project();
        let sessions = SessionManager::with_store(Config::default(), SessionStore::new());
        let config = Config::default();

        let branch_id = store.branches_for_project_sorted(project_id)[0].id;
        let mut state = AppState {
            pending_delete_branch: Some(branch_id),
            pending_delete_project: Some(project_id),
            filtered_branch_refs: vec![BranchRef::new(WizardRefType::Local, "main".to_string())],
            ..Default::default()
        };
        state.worktree_wizard.branch_name = "feature".to_string();
        state.worktree_wizard.project_name = "panoptes".to_string();
        state.worktree_wizard.search_text = "feat".to_string();

        let project = store.get_project(project_id);
        for width in [1_u16, 10, 20, 36, 44, 60] {
            for height in [1_u16, 3, 8, 12] {
                for mode in [
                    InputMode::WorktreeSelectBranch,
                    InputMode::WorktreeSelectBase,
                    InputMode::WorktreeConfirm,
                ] {
                    state.input_mode = mode;
                    render_to_lines(width, height, |frame| {
                        render_worktree_wizard(frame, frame.size(), &state, &config)
                    });
                }
                render_to_lines(width, height, |frame| {
                    render_default_base_selector(frame, frame.size(), &state)
                });
                render_to_lines(width, height, |frame| {
                    render_branch_delete_confirmation(
                        frame,
                        frame.size(),
                        &state,
                        &store,
                        &sessions,
                    )
                });
                render_to_lines(width, height, |frame| {
                    render_project_delete_confirmation(
                        frame,
                        frame.size(),
                        &state,
                        project,
                        &sessions,
                    )
                });
            }
        }
    }

    #[test]
    fn test_select_base_reads_the_filtered_cache() {
        let mut state = AppState {
            input_mode: InputMode::WorktreeSelectBase,
            ..Default::default()
        };
        state.worktree_wizard.branch_name = "new-branch".to_string();
        state.worktree_wizard.filtered_base_branches =
            vec![BranchRef::new(WizardRefType::Local, "main".to_string()).with_default_base(true)];

        let lines = render_wizard(&state);

        assert!(
            contains_line(&lines, "Branch name: new-branch"),
            "{lines:?}"
        );
        assert!(
            contains_line(&lines, "Select Base Branch (1 options)"),
            "{lines:?}"
        );
        assert!(contains_line(&lines, "[L] main * (default)"), "{lines:?}");
    }

    #[test]
    fn test_confirm_step_shows_the_worktree_location() {
        let mut state = AppState {
            input_mode: InputMode::WorktreeConfirm,
            ..Default::default()
        };
        state.worktree_wizard.branch_name = "feature-z".to_string();
        state.worktree_wizard.project_name = "panoptes".to_string();
        state.worktree_wizard.creation_type = WorktreeCreationType::ExistingLocal;

        let lines = render_wizard(&state);

        assert!(contains_line(&lines, "Create Worktree"), "{lines:?}");
        assert!(contains_line(&lines, "feature-z"), "{lines:?}");
        assert!(contains_line(&lines, "Worktree location:"), "{lines:?}");
    }

    #[test]
    fn test_default_base_selector_marks_the_current_default() {
        let state = AppState {
            input_mode: InputMode::SelectingDefaultBase,
            filtered_branch_refs: vec![
                BranchRef::new(WizardRefType::Local, "main".to_string()).with_default_base(true),
                BranchRef::new(WizardRefType::Remote, "origin/dev".to_string()),
            ],
            ..Default::default()
        };

        let lines = render_to_lines(120, 30, |frame| {
            render_default_base_selector(frame, frame.size(), &state)
        });

        assert!(
            contains_line(&lines, "Select Default Base Branch (2 options)"),
            "{lines:?}"
        );
        assert!(
            contains_line(&lines, "[L] main (current default)"),
            "{lines:?}"
        );
        assert!(contains_line(&lines, "[R] origin/dev"), "{lines:?}");
    }

    #[test]
    fn test_branch_delete_confirmation_offers_the_worktree_toggle() {
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
        let sessions = SessionManager::with_store(Config::default(), SessionStore::new());

        let lines = render_to_lines(100, 30, |frame| {
            render_branch_delete_confirmation(frame, frame.size(), &state, &store, &sessions)
        });

        assert!(
            contains_line(&lines, "Delete worktree: feature-x?"),
            "{lines:?}"
        );
        assert!(
            contains_line(&lines, "The git branch itself is NOT deleted."),
            "{lines:?}"
        );
        assert!(
            contains_line(&lines, "Also delete the worktree directory from disk"),
            "{lines:?}"
        );
    }

    #[test]
    fn test_project_delete_confirmation_keeps_worktrees_on_disk() {
        let (store, project_id) = store_with_project();
        let state = AppState {
            input_mode: InputMode::ConfirmingProjectDelete,
            pending_delete_project: Some(project_id),
            ..Default::default()
        };
        let sessions = SessionManager::with_store(Config::default(), SessionStore::new());
        let project = store.get_project(project_id);

        let lines = render_to_lines(100, 30, |frame| {
            render_project_delete_confirmation(frame, frame.size(), &state, project, &sessions)
        });

        assert!(
            contains_line(&lines, "Delete project: panoptes?"),
            "{lines:?}"
        );
        assert!(
            contains_line(&lines, "Git worktrees on disk will NOT be deleted."),
            "{lines:?}"
        );
    }
}
