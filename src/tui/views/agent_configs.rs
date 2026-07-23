//! Agent configs view (Claude Code and Codex accounts)
//!
//! Displays and manages agent configs. Claude and Codex configs go
//! through identical screens - a list view, name/path input dialogs, a
//! selector overlay, and a delete confirmation - differing only in wording
//! and in which profile type they show. The render functions are written
//! once, generic over [`AgentProfile`]; [`AgentKind`] picks the wording.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use uuid::Uuid;

use crate::agent_profiles::{AgentProfile, ProfileStore};
use crate::input::agent_configs::AgentKind;
use crate::tui::header::Header;
use crate::tui::header_notifications::HeaderNotificationManager;
use crate::tui::layout::ScreenLayout;
use crate::tui::theme::theme;
use crate::tui::views::confirm::{render_confirm_dialog, ConfirmDialogConfig};
use crate::tui::views::{render_footer, Breadcrumb};
use crate::tui::widgets::dialog::{render_dialog, DialogSize, DialogSpec};
use crate::tui::widgets::selection::{
    selection_name_style, selection_prefix, selection_style_with_accent,
};

/// The wording that differs between the Claude and Codex config screens
struct AgentConfigCopy {
    /// Breadcrumb segment (e.g. "Claude Configs")
    breadcrumb: &'static str,
    /// List/empty-state title (e.g. "Claude Configs")
    list_title: &'static str,
    /// Empty state: nothing found line
    none_found: &'static str,
    /// Empty state: what a config points at
    dir_line: &'static str,
    /// Empty state: what multiple configs enable
    accounts_line: &'static str,
    /// Empty state: which directory is used by default
    default_note: &'static str,
    /// Path dialog: prompt for the directory
    dir_prompt: &'static str,
    /// Path dialog: hint about the default directory
    default_dir_hint: &'static str,
    /// Title of the add-config dialogs (e.g. " New Claude Config ")
    new_dialog_title: &'static str,
}

const CLAUDE_COPY: AgentConfigCopy = AgentConfigCopy {
    breadcrumb: "Claude Configs",
    list_title: "Claude Configs",
    none_found: "No Claude configs found.",
    dir_line: "Each config points to a different Claude config directory,",
    accounts_line: "so you can switch between Claude accounts.",
    default_note: "The default config (~/.claude) is used if you don't specify a path.",
    dir_prompt: "Enter the path to the Claude config directory:",
    default_dir_hint: "(Leave empty for default ~/.claude)",
    new_dialog_title: " New Claude Config ",
};

const CODEX_COPY: AgentConfigCopy = AgentConfigCopy {
    breadcrumb: "Codex Configs",
    list_title: "Codex Configs",
    none_found: "No Codex configs found.",
    dir_line: "Each config points to a different CODEX_HOME directory,",
    accounts_line: "so you can switch between Codex accounts.",
    default_note: "The default config (~/.codex) is used if you don't specify a path.",
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

/// Render the agent configs view
pub fn render_agent_configs<C: AgentProfile>(
    frame: &mut Frame,
    area: Rect,
    kind: AgentKind,
    config_store: &ProfileStore<C>,
    selected_index: usize,
    header_notifications: &HeaderNotificationManager,
    attention_count: usize,
) {
    let copy = copy_for(kind);

    // Build header
    let breadcrumb = Breadcrumb::new().push(copy.breadcrumb);
    let suffix = format!("({} configs)", config_store.count());

    let header = Header::new(breadcrumb)
        .with_suffix(suffix)
        .with_notifications(Some(header_notifications))
        .with_attention_count(attention_count);

    // Create layout with header and footer
    let areas = ScreenLayout::new(area).with_header(header).render(frame);

    // Render config list or empty state
    let configs = config_store.configs_sorted();
    if configs.is_empty() {
        render_empty_state(frame, areas.content, copy);
    } else {
        render_config_list(
            frame,
            areas.content,
            copy,
            &configs,
            config_store.get_default_id(),
            selected_index,
        );
    }

    // Footer
    let footer_text = if configs.is_empty() {
        "n: new config | ?: help | Esc: back"
    } else {
        "n: new | s: set default | d: delete | ?: help | Esc: back"
    };
    render_footer(frame, areas.footer(), footer_text);
}

/// Render the empty state message
fn render_empty_state(frame: &mut Frame, area: Rect, copy: &AgentConfigCopy) {
    let t = theme();

    let message = vec![
        Line::from(""),
        Line::from(Span::styled(copy.none_found, Style::default().fg(t.text))),
        Line::from(""),
        Line::from(Span::styled(
            "Press 'n' to add a config.",
            Style::default().fg(t.text_muted),
        )),
        Line::from(""),
        Line::from(Span::styled(
            copy.dir_line,
            Style::default().fg(t.text_muted),
        )),
        Line::from(Span::styled(
            copy.accounts_line,
            Style::default().fg(t.text_muted),
        )),
        Line::from(""),
        Line::from(Span::styled(
            copy.default_note,
            Style::default().fg(t.text_muted),
        )),
    ];

    let paragraph = Paragraph::new(message).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .title(copy.list_title),
    );
    frame.render_widget(paragraph, area);
}

/// Render the list of configs
fn render_config_list<C: AgentProfile>(
    frame: &mut Frame,
    area: Rect,
    copy: &AgentConfigCopy,
    configs: &[&C],
    default_id: Option<Uuid>,
    selected_index: usize,
) {
    let t = theme();

    let items: Vec<ListItem> = configs
        .iter()
        .enumerate()
        .map(|(i, config)| {
            let is_selected = i == selected_index;
            let is_default = default_id == Some(config.id());

            let prefix = selection_prefix(is_selected);
            let default_marker = if is_default { " ★" } else { "" };

            let content = Line::from(vec![
                Span::raw(prefix),
                Span::styled(
                    config.name().to_string(),
                    selection_name_style(is_selected, t),
                ),
                Span::styled(default_marker, Style::default().fg(t.default_marker)),
                Span::styled(
                    format!("  {}", config.home_dir_display()),
                    Style::default().fg(t.text_muted),
                ),
            ]);

            ListItem::new(content)
        })
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(format!(
        "{} ({})",
        copy.list_title,
        configs.len()
    )));
    frame.render_widget(list, area);
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

    fn render_view<C: AgentProfile>(
        kind: AgentKind,
        store: &ProfileStore<C>,
        selected: usize,
    ) -> Vec<String> {
        let header_notifications = HeaderNotificationManager::default();
        render_to_lines(80, 24, |frame| {
            render_agent_configs(
                frame,
                frame.size(),
                kind,
                store,
                selected,
                &header_notifications,
                0,
            )
        })
    }

    #[test]
    fn test_claude_empty_state() {
        let lines = render_view(AgentKind::Claude, &ClaudeConfigStore::new(), 0);

        assert!(
            contains_line(&lines, "No Claude configs found."),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "Press 'n' to add a config."),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "The default config (~/.claude) is used"),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "n: new config | ?: help | Esc: back"),
            "{:?}",
            lines
        );
    }

    #[test]
    fn test_codex_empty_state() {
        let lines = render_view(AgentKind::Codex, &CodexConfigStore::new(), 0);

        assert!(
            contains_line(&lines, "No Codex configs found."),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "CODEX_HOME directory,"),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "The default config (~/.codex) is used"),
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

        let lines = render_view(AgentKind::Claude, &store, 0);

        assert!(contains_line(&lines, "Claude Configs (1)"), "{:?}", lines);
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

        let lines = render_view(AgentKind::Codex, &store, 0);

        assert!(contains_line(&lines, "Codex Configs (1)"), "{:?}", lines);
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
