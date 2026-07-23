//! Agent configs view (Claude Code and Codex accounts)
//!
//! Displays and manages agent configs. Claude and Codex configs go
//! through identical screens - a list view, name/path input dialogs, a
//! selector overlay, and a delete confirmation - differing only in wording
//! and in which profile type they show. The render functions are written
//! once, generic over [`AgentProfile`]; [`AgentKind`] picks the wording.

use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, Paragraph};
use uuid::Uuid;

use crate::agent_profiles::{AgentProfile, ProfileStore};
use crate::input::agent_configs::AgentKind;
use crate::tui::theme::theme;
use crate::tui::views::confirm::{render_confirm_dialog, ConfirmDialogConfig};
use crate::tui::widgets::dialog::{render_dialog, DialogSize, DialogSpec};
use crate::tui::widgets::selection::{
    selection_name_style, selection_prefix, selection_style_with_accent,
};

/// The wording that differs between the Claude and Codex config dialogs
///
/// The list itself lives in pane 3, which titles and borders it, so only the
/// add-config dialogs still need agent-specific copy.
struct AgentConfigCopy {
    /// Path dialog: prompt for the directory
    dir_prompt: &'static str,
    /// Path dialog: hint about the default directory
    default_dir_hint: &'static str,
    /// Title of the add-config dialogs (e.g. " New Claude Config ")
    new_dialog_title: &'static str,
}

const CLAUDE_COPY: AgentConfigCopy = AgentConfigCopy {
    dir_prompt: "Enter the path to the Claude config directory:",
    default_dir_hint: "(Leave empty for default ~/.claude)",
    new_dialog_title: " New Claude Config ",
};

const CODEX_COPY: AgentConfigCopy = AgentConfigCopy {
    dir_prompt: "Enter the path to the CODEX_HOME directory:",
    default_dir_hint: "(Leave empty for default ~/.codex)",
    new_dialog_title: " New Codex Config ",
};

/// Wording for the given agent
fn copy_for(kind: AgentKind) -> &'static AgentConfigCopy {
    match kind {
        AgentKind::Claude => &CLAUDE_COPY,
        AgentKind::Codex => &CODEX_COPY,
    }
}

/// Render an agent's config list into a settings-pane rect
///
/// The pane owns the border and title, so this draws rows only, truncated
/// against the pane's current width.
pub fn render_agent_config_list<C: AgentProfile>(
    frame: &mut Frame,
    area: Rect,
    config_store: &ProfileStore<C>,
    selected_index: usize,
) {
    let t = theme();
    let configs = config_store.configs_sorted();
    if configs.is_empty() {
        frame.render_widget(
            Paragraph::new("No configs yet.\n\nPress 'n' to add one.").style(t.muted_style()),
            area,
        );
        return;
    }

    let default_id = config_store.get_default_id();
    let width = area.width as usize;

    let items: Vec<ListItem> = configs
        .iter()
        .enumerate()
        .map(|(i, config)| {
            let is_selected = i == selected_index;
            let is_default = default_id == Some(config.id());

            let line = Line::from(vec![
                Span::raw(selection_prefix(is_selected)),
                Span::styled(
                    config.name().to_string(),
                    selection_name_style(is_selected, t),
                ),
                Span::styled(
                    if is_default { " ★" } else { "" },
                    Style::default().fg(t.default_marker),
                ),
                Span::styled(
                    format!("  {}", config.home_dir_display()),
                    Style::default().fg(t.text_muted),
                ),
            ]);

            ListItem::new(crate::tui::views::pane_projects::clamp_line(line, width))
        })
        .collect();

    let items = crate::tui::views::window_rows(items, selected_index, area.height);
    frame.render_widget(List::new(items), area);
}

/// Render the config name input dialog
pub fn render_agent_config_name_input_dialog(
    frame: &mut Frame,
    area: Rect,
    kind: AgentKind,
    input: &str,
) {
    let t = theme();
    let copy = copy_for(kind);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Enter a name for this config:",
            Style::default().fg(t.text),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{}_", input), Style::default().fg(t.accent)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "[Enter] Continue  [Esc] Cancel",
            Style::default().fg(t.text_muted),
        )),
    ];

    render_dialog(
        frame,
        area,
        DialogSpec {
            title: copy.new_dialog_title,
            border_color: t.accent,
            alignment: Alignment::Center,
            width: DialogSize::Fixed(50),
            height: DialogSize::Fixed(9),
        },
        lines,
    );
}

/// Render the config path input dialog
#[allow(clippy::too_many_arguments)]
pub fn render_agent_config_path_input_dialog(
    frame: &mut Frame,
    area: Rect,
    kind: AgentKind,
    name: &str,
    input: &str,
    completions: &[std::path::PathBuf],
    completion_index: usize,
    show_completions: bool,
) {
    let t = theme();
    let copy = copy_for(kind);

    let base_height = 12_u16;
    let completion_height = if show_completions && !completions.is_empty() {
        (completions.len().min(5) + 1) as u16
    } else {
        0
    };
    let dialog_width = 60_u16.min(area.width.saturating_sub(4));

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Name: ", Style::default().fg(t.text)),
            Span::styled(name, Style::default().fg(t.accent)),
        ]),
        Line::from(""),
        Line::from(Span::styled(copy.dir_prompt, Style::default().fg(t.text))),
        Line::from(Span::styled(
            copy.default_dir_hint,
            Style::default().fg(t.text_muted),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{}_", input), Style::default().fg(t.accent)),
        ]),
    ];

    // Add completions if showing
    if show_completions && !completions.is_empty() {
        lines.push(Line::from(""));
        for (i, path) in completions.iter().take(5).enumerate() {
            let is_selected = i == completion_index;

            // Shorten path for display
            let display = path
                .to_string_lossy()
                .chars()
                .take(dialog_width.saturating_sub(6) as usize)
                .collect::<String>();

            lines.push(Line::from(vec![
                Span::raw(selection_prefix(is_selected)),
                Span::styled(display, selection_style_with_accent(is_selected, t)),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "[Tab] Complete  [Enter] Confirm  [Esc] Cancel",
        Style::default().fg(t.text_muted),
    )));

    render_dialog(
        frame,
        area,
        DialogSpec {
            title: copy.new_dialog_title,
            border_color: t.accent,
            alignment: Alignment::Left,
            width: DialogSize::Fixed(60),
            height: DialogSize::Fixed(base_height + completion_height),
        },
        lines,
    );
}

/// Render the config delete confirmation dialog
pub fn render_agent_config_delete_dialog(
    frame: &mut Frame,
    area: Rect,
    config_name: &str,
    affected_projects: &[String],
) {
    let t = theme();

    let has_affected = !affected_projects.is_empty();
    let base_height = 10_u16;
    let affected_height = if has_affected {
        (affected_projects.len().min(5) + 2) as u16
    } else {
        0
    };

    let mut body_lines = Vec::new();
    if has_affected {
        body_lines.push(Line::from(Span::styled(
            "This config is used by:",
            Style::default().fg(t.border_warning),
        )));
        for project in affected_projects.iter().take(5) {
            body_lines.push(Line::from(vec![
                Span::raw("  - "),
                Span::styled(project.as_str(), Style::default().fg(t.text)),
            ]));
        }
        if affected_projects.len() > 5 {
            body_lines.push(Line::from(Span::styled(
                format!("  ... and {} more", affected_projects.len() - 5),
                Style::default().fg(t.text_muted),
            )));
        }
        body_lines.push(Line::from(Span::styled(
            "Projects will revert to global default.",
            Style::default().fg(t.border_warning),
        )));
        body_lines.push(Line::from(""));
    }

    let config = ConfirmDialogConfig {
        body_lines,
        overlay: Some((
            DialogSize::Fixed(55),
            DialogSize::Fixed(base_height + affected_height),
        )),
        ..ConfirmDialogConfig::new("Confirm Delete", "config", config_name)
    };
    render_confirm_dialog(frame, area, config);
}

/// Render the config selector overlay
pub fn render_agent_config_selector<C: AgentProfile>(
    frame: &mut Frame,
    area: Rect,
    kind: AgentKind,
    configs: &[C],
    selected_index: usize,
    default_id: Option<Uuid>,
) {
    let t = theme();

    let list_height = configs.len().min(8) as u16;

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("Select a {} config:", kind.label()),
            Style::default().fg(t.text),
        )),
        Line::from(""),
    ];

    for (i, config) in configs.iter().enumerate() {
        let is_selected = i == selected_index;
        let is_default = default_id == Some(config.id());
        let default_marker = if is_default { " (default)" } else { "" };

        lines.push(Line::from(vec![
            Span::raw(selection_prefix(is_selected)),
            Span::styled(
                config.name().to_string(),
                selection_name_style(is_selected, t),
            ),
            Span::styled(default_marker, Style::default().fg(t.default_marker)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "[Enter] Select  [Esc] Cancel",
        Style::default().fg(t.text_muted),
    )));

    render_dialog(
        frame,
        area,
        DialogSpec {
            title: " Select Config ",
            border_color: t.accent,
            alignment: Alignment::Left,
            width: DialogSize::Fixed(50),
            height: DialogSize::Fixed(list_height + 6),
        },
        lines,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_config::{ClaudeConfig, ClaudeConfigStore};
    use crate::codex_config::{CodexConfig, CodexConfigStore};
    use crate::tui::views::test_util::{contains_line, render_to_lines};
    use std::path::PathBuf;

    fn render_list<C: AgentProfile>(store: &ProfileStore<C>, selected: usize) -> Vec<String> {
        render_to_lines(60, 12, |frame| {
            render_agent_config_list(frame, frame.size(), store, selected)
        })
    }

    #[test]
    fn test_empty_list_points_at_the_add_key() {
        let lines = render_list(&ClaudeConfigStore::new(), 0);

        assert!(contains_line(&lines, "No configs yet."), "{:?}", lines);
        assert!(
            contains_line(&lines, "Press 'n' to add one."),
            "{:?}",
            lines
        );
    }

    #[test]
    fn test_claude_list_marks_default_and_shows_dir() {
        let mut store = ClaudeConfigStore::new();
        store.add(ClaudeConfig::new(
            "Work".to_string(),
            Some(PathBuf::from("/tmp/claude-work")),
        ));

        let lines = render_list(&store, 0);

        // First (and only) config becomes the default and carries the star
        assert!(
            contains_line(&lines, "Work ★  /tmp/claude-work"),
            "{:?}",
            lines
        );
    }

    #[test]
    fn test_codex_list_marks_default_and_shows_dir() {
        let mut store = CodexConfigStore::new();
        store.add(CodexConfig::new(
            "Work".to_string(),
            Some(PathBuf::from("/tmp/codex-work")),
        ));

        let lines = render_list(&store, 0);

        assert!(
            contains_line(&lines, "Work ★  /tmp/codex-work"),
            "{:?}",
            lines
        );
    }

    #[test]
    fn test_name_input_dialog_shows_cursor_and_title() {
        let lines = render_to_lines(80, 24, |frame| {
            render_agent_config_name_input_dialog(frame, frame.size(), AgentKind::Claude, "Wor")
        });

        assert!(contains_line(&lines, "New Claude Config"), "{:?}", lines);
        assert!(
            contains_line(&lines, "Enter a name for this config:"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "Wor_"), "{:?}", lines);
    }

    #[test]
    fn test_path_input_dialog_uses_agent_wording() {
        let lines = render_to_lines(80, 24, |frame| {
            render_agent_config_path_input_dialog(
                frame,
                frame.size(),
                AgentKind::Codex,
                "Work",
                "~/co",
                &[],
                0,
                false,
            )
        });

        assert!(contains_line(&lines, "New Codex Config"), "{:?}", lines);
        assert!(
            contains_line(&lines, "Enter the path to the CODEX_HOME directory:"),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "(Leave empty for default ~/.codex)"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "~/co_"), "{:?}", lines);
    }

    #[test]
    fn test_delete_dialog_lists_affected_projects() {
        let lines = render_to_lines(80, 24, |frame| {
            render_agent_config_delete_dialog(
                frame,
                frame.size(),
                "Work",
                &["panoptes".to_string()],
            )
        });

        assert!(contains_line(&lines, "Delete config: Work?"), "{:?}", lines);
        assert!(
            contains_line(&lines, "This config is used by:"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "- panoptes"), "{:?}", lines);
        assert!(
            contains_line(&lines, "Projects will revert to global default."),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "Press y to confirm, n or Esc to cancel"),
            "{:?}",
            lines
        );
    }

    #[test]
    fn test_selector_marks_default_and_selection() {
        let default = ClaudeConfig::new("Default".to_string(), None);
        let default_id = default.id;
        let configs = vec![
            default,
            ClaudeConfig::new("Work".to_string(), Some(PathBuf::from("/tmp/claude-work"))),
        ];

        let lines = render_to_lines(80, 24, |frame| {
            render_agent_config_selector(
                frame,
                frame.size(),
                AgentKind::Claude,
                &configs,
                1,
                Some(default_id),
            )
        });

        assert!(
            contains_line(&lines, "Select a Claude config:"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "Default (default)"), "{:?}", lines);
        assert!(contains_line(&lines, "▶ Work"), "{:?}", lines);
    }
}
