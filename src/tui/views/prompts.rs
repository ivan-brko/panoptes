//! Prompts that outgrew a pane
//!
//! The rule, for anything added later: if it shows a list or a paragraph it is
//! a centred overlay; if it is one line you type into, it is inline in the pane
//! that owns it. Overlays are anchored to the terminal, so an animating pane
//! can never resize a prompt under the user mid-typing - and a list of paths
//! all truncated to the same prefix is not a list you can choose from.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

use crate::app::{AppState, FolderMoveTarget};
use crate::project::{folder_path_key, MAX_FOLDER_DEPTH};
use crate::tui::theme::theme;
use crate::tui::views::visible_window;
use crate::tui::widgets::dialog::{centered_rect, DialogSize};
use crate::tui::widgets::selection::{selection_prefix, selection_style_with_accent};

/// How many completions a prompt offers at once
const MAX_COMPLETIONS: usize = 8;

/// Width of the path-shaped prompts
const PROMPT_WIDTH: DialogSize = DialogSize::Percent {
    pct: 70,
    min: 40,
    max: 76,
};

/// A prompt overlay: an input box, and a completions list under it
///
/// Both boxes are drawn into one cleared rectangle so the pane content behind
/// never shows through between them.
fn render_prompt_overlay(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    input_lines: Vec<Line<'static>>,
    completions_title: Option<String>,
    completions: Vec<ListItem<'static>>,
) {
    let t = theme();
    let input_height = input_lines.len() as u16 + 2;
    let completions_height = if completions.is_empty() {
        0
    } else {
        completions.len() as u16 + 2
    };

    let overlay = centered_rect(
        area,
        PROMPT_WIDTH,
        DialogSize::Fixed(input_height + completions_height),
    );
    frame.render_widget(Clear, overlay);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(input_height),
            Constraint::Length(completions_height),
        ])
        .split(overlay);

    frame.render_widget(
        Paragraph::new(input_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.accent))
                .title(title.to_string()),
        ),
        chunks[0],
    );

    if let Some(completions_title) = completions_title {
        frame.render_widget(
            List::new(completions).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.border_focused))
                    .title(completions_title),
            ),
            chunks[1],
        );
    }
}

/// Build the completion rows and their title, or `None` when none are showing
fn completion_rows<T, F>(
    items: &[T],
    selected: usize,
    showing: bool,
    label: &str,
    render_row: F,
) -> (Option<String>, Vec<ListItem<'static>>)
where
    F: Fn(&T) -> String,
{
    if !showing || items.is_empty() {
        return (None, Vec::new());
    }
    let total = items.len();
    let (start, end) = visible_window(total, selected, MAX_COMPLETIONS);
    let t = theme();

    let rows = items[start..end]
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = start + i == selected;
            ListItem::new(format!(
                "{}{}",
                selection_prefix(is_selected),
                render_row(item)
            ))
            .style(selection_style_with_accent(is_selected, t))
        })
        .collect();

    let title = if total > MAX_COMPLETIONS {
        format!("{} ({}/{}) ↑↓", label, selected + 1, total)
    } else {
        format!("{} ({})", label, total)
    };
    (Some(title), rows)
}

/// The add-project path prompt, with its path completions
pub fn render_project_addition_dialog(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();
    let lines = vec![
        Line::from(Span::styled(
            "Enter the path to a git repository (Tab: autocomplete, ~/ works):",
            Style::default().fg(t.text),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("> {}_", state.new_project_path),
            t.input_style(),
        )),
    ];

    let (title, rows) = completion_rows(
        &state.path_completions,
        state.path_completion_index,
        state.show_path_completions,
        "Completions",
        |path| format!("{}/", crate::path_complete::path_to_display(path)),
    );

    render_prompt_overlay(frame, area, " Add Project ", lines, title, rows);
}

/// The move-to-folder prompt, with its folder completions
pub fn render_folder_move_dialog(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();

    let what = match &state.moving_to_folder {
        Some(FolderMoveTarget::Folder(path)) => {
            format!("folder '{}' and its contents", folder_path_key(path))
        }
        _ => "the selected project".to_string(),
    };

    let mut lines = vec![
        Line::from(Span::styled(
            format!("Move {} into a folder.", what),
            Style::default().fg(t.text),
        )),
        Line::from(Span::styled(
            format!(
                "Use '/' to nest (max {} levels); leave empty for the root level.",
                MAX_FOLDER_DEPTH
            ),
            t.muted_style(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("> {}_", state.folder_input),
            t.input_style(),
        )),
    ];
    if let Some(error) = &state.folder_error {
        lines.push(Line::from(Span::styled(
            format!("✖ {}", error),
            Style::default().fg(t.error_bg),
        )));
    }

    let (title, rows) = completion_rows(
        &state.folder_completions,
        state.folder_completion_index,
        state.show_folder_completions,
        "Existing folders",
        |path| format!("{}/", path),
    );

    render_prompt_overlay(frame, area, " Move to Folder ", lines, title, rows);
}

/// The folder-removal confirmation
///
/// Deliberately not the shared delete dialog: dissolving a folder deletes
/// nothing, and "Delete folder: X?" would say otherwise.
pub fn render_folder_remove_confirmation(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    project_store: &crate::project::ProjectStore,
) {
    use crate::tui::views::confirm::confirm_prompt_line;
    use crate::tui::widgets::dialog::{render_dialog, DialogSpec};

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

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Remove folder ", Style::default().fg(t.text)),
            Span::styled(
                format!("'{}'", folder_path_key(path)),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("?", Style::default().fg(t.text)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            format!(
                "Its {} move up to {}.",
                crate::project::project_count_label(affected),
                destination
            ),
            t.muted_style(),
        )),
        Line::from(Span::styled(
            "No projects or sessions are deleted.",
            t.muted_style(),
        )),
        Line::from(""),
        confirm_prompt_line(),
    ];

    render_dialog(
        frame,
        area,
        DialogSpec {
            title: " Remove Folder ",
            border_color: t.border_warning,
            alignment: Alignment::Center,
            width: DialogSize::Fixed(62),
            height: DialogSize::Fixed(9),
        },
        lines,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::InputMode;
    use crate::tui::views::test_util::{contains_line, render_to_lines};
    use std::path::PathBuf;

    #[test]
    fn test_add_project_prompt_is_an_overlay_with_completions() {
        let state = AppState {
            input_mode: InputMode::AddingProject,
            new_project_path: "~/pro".to_string(),
            path_completions: vec![PathBuf::from("/home/me/projects")],
            show_path_completions: true,
            ..Default::default()
        };

        let lines = render_to_lines(120, 24, |frame| {
            render_project_addition_dialog(frame, frame.size(), &state)
        });

        assert!(contains_line(&lines, "Add Project"), "{lines:?}");
        assert!(contains_line(&lines, "> ~/pro_"), "{lines:?}");
        assert!(contains_line(&lines, "Completions (1)"), "{lines:?}");
        // Centred overlay: the top row of the terminal stays empty
        assert!(lines[0].is_empty(), "{lines:?}");
    }

    #[test]
    fn test_add_project_prompt_hides_the_completions_box_when_empty() {
        let state = AppState {
            input_mode: InputMode::AddingProject,
            new_project_path: "/tmp".to_string(),
            ..Default::default()
        };

        let lines = render_to_lines(120, 24, |frame| {
            render_project_addition_dialog(frame, frame.size(), &state)
        });

        assert!(!contains_line(&lines, "Completions"), "{lines:?}");
    }

    #[test]
    fn test_folder_move_prompt_shows_error_and_completions() {
        let state = AppState {
            input_mode: InputMode::MovingToFolder,
            moving_to_folder: Some(FolderMoveTarget::Project(uuid::Uuid::new_v4())),
            folder_input: "a/b/c/d".to_string(),
            folder_error: Some("Folders can nest at most 3 levels deep (got 4)".to_string()),
            folder_completions: vec!["Acme".to_string()],
            show_folder_completions: true,
            ..Default::default()
        };

        let lines = render_to_lines(120, 24, |frame| {
            render_folder_move_dialog(frame, frame.size(), &state)
        });

        assert!(contains_line(&lines, "Move to Folder"), "{lines:?}");
        assert!(contains_line(&lines, "> a/b/c/d_"), "{lines:?}");
        assert!(contains_line(&lines, "at most 3 levels deep"), "{lines:?}");
        assert!(contains_line(&lines, "Acme/"), "{lines:?}");
    }

    #[test]
    fn test_folder_remove_confirmation_states_projects_are_kept() {
        let mut store = crate::project::ProjectStore::new();
        for name in ["api-gateway", "auth-service"] {
            let mut project = crate::project::Project::new(
                name.to_string(),
                PathBuf::from("/tmp"),
                "main".to_string(),
            );
            project.folder = vec!["Acme".to_string()];
            store.add_project(project);
        }
        let state = AppState {
            input_mode: InputMode::ConfirmingFolderRemove,
            pending_remove_folder: Some(vec!["Acme".to_string()]),
            ..Default::default()
        };

        let lines = render_to_lines(100, 24, |frame| {
            render_folder_remove_confirmation(frame, frame.size(), &state, &store)
        });

        assert!(contains_line(&lines, "Remove folder 'Acme'?"), "{lines:?}");
        assert!(
            contains_line(&lines, "Its 2 projects move up to the root level."),
            "{lines:?}"
        );
        assert!(
            contains_line(&lines, "No projects or sessions are deleted."),
            "{lines:?}"
        );
    }
}
