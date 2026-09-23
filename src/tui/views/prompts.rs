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
use crate::transcript::scan::FoundConversation;
use crate::transcript::TranscriptKind;
use crate::tui::theme::theme;
use crate::tui::views::{truncate_path, truncate_string, visible_window};
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
                    .border_style(Style::default().fg(t.border_focus))
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

/// How many conversations the import picker shows at once
const MAX_IMPORT_ROWS: usize = 12;

/// Width of the import picker: titles are prose, so it is given room
const IMPORT_WIDTH: DialogSize = DialogSize::Percent {
    pct: 80,
    min: 40,
    max: 110,
};

/// The fewest title characters a picker row keeps before it drops a field
const MIN_IMPORT_TITLE: usize = 16;

/// The conversation import picker
///
/// A list, so an overlay (see the module doc). One row per conversation:
/// agent badge, title, then how long ago it was used and the account it
/// belongs to. The row drops the account, then the time, whole as the
/// overlay narrows, rather than squeezing the title to nothing - the badge
/// and title are what the choice is made on.
pub fn render_conversation_import(frame: &mut Frame, area: Rect, state: &AppState) {
    let Some(import) = &state.conversation_import else {
        return;
    };
    let t = theme();
    let total = import.conversations.len();
    let (start, end) = visible_window(total, import.selected, MAX_IMPORT_ROWS);

    // Rows, plus the heading and its gap, the warning if any, and the border
    let extra_lines = 2 + u16::from(import.truncated);
    let overlay = centered_rect(
        area,
        IMPORT_WIDTH,
        DialogSize::Fixed((end - start) as u16 + extra_lines + 2),
    );
    let inner_width = overlay.width.saturating_sub(2) as usize;
    let now = chrono::Utc::now();

    let mut lines = vec![
        Line::from(Span::styled(
            // The path last, so a long one loses its start rather than the
            // part that tells worktrees apart
            truncate_path(
                &format!("Newest first, started in {}", import.working_dir.display()),
                inner_width,
            ),
            t.muted_style(),
        )),
        Line::from(""),
    ];
    lines.extend(
        import.conversations[start..end]
            .iter()
            .enumerate()
            .map(|(i, conversation)| {
                import_row(conversation, start + i == import.selected, inner_width, now)
            }),
    );
    if import.truncated {
        lines.push(Line::from(Span::styled(
            truncate_string(
                "Search stopped early: older conversations may not be listed",
                inner_width,
            ),
            Style::default().fg(t.warning),
        )));
    }

    let title = if total > MAX_IMPORT_ROWS {
        format!(
            " Import Conversation ({}/{}) ↑↓ ",
            import.selected + 1,
            total
        )
    } else {
        format!(" Import Conversation ({}) ", total)
    };

    frame.render_widget(Clear, overlay);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.border_focus))
                .title(title),
        ),
        overlay,
    );
}

/// One picker row, fitted to `width` columns
fn import_row(
    conversation: &FoundConversation,
    selected: bool,
    width: usize,
    now: chrono::DateTime<chrono::Utc>,
) -> Line<'static> {
    let t = theme();
    let prefix = selection_prefix(selected);
    let badge = match conversation.kind {
        TranscriptKind::Claude => "[CC] ",
        TranscriptKind::Codex => "[CX] ",
    };
    let title = conversation
        .title
        .clone()
        .unwrap_or_else(|| "(no prompt)".to_string());
    let (time, account) = import_row_fields(conversation, now);

    // Right-hand fields, most expendable first to go
    let fixed = prefix.chars().count() + badge.chars().count();
    let mut trailer: Vec<String> = vec![time, account];
    let trailer_len = |fields: &[String]| -> usize {
        fields.iter().map(|f| f.chars().count() + 2).sum::<usize>()
    };
    while !trailer.is_empty()
        && fixed + MIN_IMPORT_TITLE.min(title.chars().count()) + trailer_len(&trailer) > width
    {
        trailer.pop();
    }

    let title_room = width.saturating_sub(fixed + trailer_len(&trailer));
    let title = truncate_string(&title, title_room);
    let pad = title_room.saturating_sub(title.chars().count());

    let mut spans = vec![
        Span::raw(prefix),
        Span::styled(badge, t.muted_style()),
        Span::styled(title, selection_style_with_accent(selected, t)),
        Span::raw(" ".repeat(pad)),
    ];
    for field in trailer {
        spans.push(Span::styled(format!("  {}", field), t.muted_style()));
    }
    Line::from(spans)
}

/// The time and account columns of a picker row
fn import_row_fields(
    conversation: &FoundConversation,
    now: chrono::DateTime<chrono::Utc>,
) -> (String, String) {
    let account = conversation
        .account_name
        .clone()
        .unwrap_or_else(|| "default".to_string());
    (relative_time(conversation.last_active, now), account)
}

/// How long ago, as briefly as a column allows
fn relative_time(
    then: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let secs = (now - then).num_seconds().max(0);
    match secs {
        0..=59 => "just now".to_string(),
        60..=3_599 => format!("{}m ago", secs / 60),
        3_600..=86_399 => format!("{}h ago", secs / 3_600),
        86_400..=604_799 => format!("{}d ago", secs / 86_400),
        604_800..=2_419_199 => format!("{}w ago", secs / 604_800),
        _ => then.format("%Y-%m-%d").to_string(),
    }
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

    /// `Clear` does not clip, so a prompt wider or taller than the terminal is
    /// a panic rather than an overflow. Every prompt must survive a terminal
    /// smaller than its preferred size.
    #[test]
    fn test_prompts_survive_a_terminal_smaller_than_they_want() {
        let state = AppState {
            new_project_path: "/tmp/x".to_string(),
            path_completions: vec![PathBuf::from("/tmp/one"), PathBuf::from("/tmp/two")],
            show_path_completions: true,
            folder_input: "Acme".to_string(),
            folder_completions: vec!["Acme".to_string()],
            show_folder_completions: true,
            pending_remove_folder: Some(vec!["Acme".to_string()]),
            ..Default::default()
        };
        let store = crate::project::ProjectStore::new();

        for width in [1_u16, 10, 20, 36, 44, 60] {
            for height in [1_u16, 3, 8, 12] {
                render_to_lines(width, height, |frame| {
                    render_project_addition_dialog(frame, frame.size(), &state)
                });
                render_to_lines(width, height, |frame| {
                    render_folder_move_dialog(frame, frame.size(), &state)
                });
                render_to_lines(width, height, |frame| {
                    render_folder_remove_confirmation(frame, frame.size(), &state, &store)
                });
                let import = import_state(3, true);
                render_to_lines(width, height, |frame| {
                    render_conversation_import(frame, frame.size(), &import)
                });
            }
        }
    }

    fn import_state(count: usize, truncated: bool) -> AppState {
        use crate::app::ConversationImport;
        let now = chrono::Utc::now();
        let conversations = (0..count)
            .map(|i| FoundConversation {
                kind: if i % 2 == 0 {
                    TranscriptKind::Claude
                } else {
                    TranscriptKind::Codex
                },
                id: format!("conv-{i}"),
                title: Some(format!("Conversation number {i} about the header layout")),
                last_active: now - chrono::Duration::hours(i as i64 + 1),
                config_id: None,
                account_name: (i == 1).then(|| "work".to_string()),
                path: PathBuf::from("/tmp/x.jsonl"),
            })
            .collect();
        AppState {
            input_mode: InputMode::ImportingConversation,
            conversation_import: Some(ConversationImport {
                project_id: uuid::Uuid::new_v4(),
                branch_id: uuid::Uuid::new_v4(),
                working_dir: PathBuf::from("/home/me/panoptes"),
                conversations,
                selected: 0,
                truncated,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn test_import_picker_is_an_overlay_listing_each_conversation() {
        let state = import_state(2, false);
        let lines = render_to_lines(120, 24, |frame| {
            render_conversation_import(frame, frame.size(), &state)
        });

        assert!(
            contains_line(&lines, "Import Conversation (2)"),
            "{lines:?}"
        );
        assert!(contains_line(&lines, "/home/me/panoptes"), "{lines:?}");
        let first = lines
            .iter()
            .find(|l| l.contains("number 0"))
            .expect("first row");
        assert!(first.contains("▶ [CC] Conversation number 0"), "{first:?}");
        assert!(
            first.contains("1h ago") && first.contains("default"),
            "{first:?}"
        );
        let second = lines.iter().find(|l| l.contains("number 1")).unwrap();
        assert!(
            second.contains("[CX]") && second.contains("work"),
            "{second:?}"
        );
        // Centred overlay: the top row of the terminal stays empty
        assert!(lines[0].is_empty(), "{lines:?}");
        assert!(!contains_line(&lines, "stopped early"), "{lines:?}");
    }

    #[test]
    fn test_import_picker_says_when_the_search_stopped_early() {
        let state = import_state(1, true);
        let lines = render_to_lines(120, 24, |frame| {
            render_conversation_import(frame, frame.size(), &state)
        });
        assert!(contains_line(&lines, "Search stopped early"), "{lines:?}");
    }

    /// Narrowing drops the account, then the time, whole - never half a field
    #[test]
    fn test_import_rows_drop_fields_whole_as_they_narrow() {
        let state = import_state(1, false);
        let conversation = &state.conversation_import.as_ref().unwrap().conversations[0];
        let now = chrono::Utc::now();
        let text = |width: usize| -> String {
            import_row(conversation, false, width, now)
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect()
        };

        let wide = text(100);
        assert!(
            wide.contains("1h ago") && wide.contains("default"),
            "{wide:?}"
        );
        let middle = text(38);
        assert!(
            middle.contains("1h ago") && !middle.contains("defa"),
            "{middle:?}"
        );
        let narrow = text(28);
        assert!(
            !narrow.contains("ago") && !narrow.contains("defa"),
            "{narrow:?}"
        );
        assert!(narrow.starts_with("  [CC] Conversation"), "{narrow:?}");
        for width in [100, 38, 28, 10] {
            assert!(text(width).chars().count() <= width.max(7), "{width}");
        }
    }

    #[test]
    fn test_import_picker_scrolls_a_long_list() {
        let mut state = import_state(30, false);
        state.conversation_import.as_mut().unwrap().selected = 20;
        let lines = render_to_lines(120, 30, |frame| {
            render_conversation_import(frame, frame.size(), &state)
        });
        assert!(
            contains_line(&lines, "Import Conversation (21/30)"),
            "{lines:?}"
        );
        assert!(
            contains_line(&lines, "▶ [CC] Conversation number 20"),
            "{lines:?}"
        );
    }

    #[test]
    fn test_relative_time_reads_briefly() {
        let now = chrono::Utc::now();
        let ago = |secs: i64| relative_time(now - chrono::Duration::seconds(secs), now);
        assert_eq!(ago(5), "just now");
        assert_eq!(ago(300), "5m ago");
        assert_eq!(ago(3 * 3_600), "3h ago");
        assert_eq!(ago(2 * 86_400), "2d ago");
        assert_eq!(ago(15 * 86_400), "2w ago");
        assert_eq!(ago(90 * 86_400).len(), "2026-01-01".len());
        // A clock skewed into the future is not "in 3m"
        assert_eq!(ago(-180), "just now");
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
