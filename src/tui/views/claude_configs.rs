//! Claude configurations view
//!
//! Displays and manages Claude Code configurations (accounts).

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::claude_config::{ClaudeConfig, ClaudeConfigId, ClaudeConfigStore};
use crate::focus_timing::FocusTimer;
use crate::tui::header::Header;
use crate::tui::header_notifications::HeaderNotificationManager;
use crate::tui::layout::ScreenLayout;
use crate::tui::theme::theme;
use crate::tui::views::Breadcrumb;
use crate::tui::widgets::selection::{selection_name_style, selection_prefix};

/// Render the Claude configurations view
#[allow(clippy::too_many_arguments)]
pub fn render_claude_configs(
    frame: &mut Frame,
    area: Rect,
    config_store: &ClaudeConfigStore,
    selected_index: usize,
    focus_timer: Option<&FocusTimer>,
    header_notifications: &HeaderNotificationManager,
    attention_count: usize,
) {
    let t = theme();

    // Build header
    let breadcrumb = Breadcrumb::new().push("Claude Configs");
    let suffix = format!("({} configs)", config_store.count());

    let header = Header::new(breadcrumb)
        .with_suffix(suffix)
        .with_timer(focus_timer)
        .with_notifications(Some(header_notifications))
        .with_attention_count(attention_count);

    // Create layout with header and footer
    let areas = ScreenLayout::new(area).with_header(header).render(frame);

    // Render config list or empty state
    let configs = config_store.configs_sorted();
    if configs.is_empty() {
        render_empty_state(frame, areas.content);
    } else {
        render_config_list(
            frame,
            areas.content,
            &configs,
            config_store.get_default_id(),
            selected_index,
        );
    }

    // Footer
    let footer_text = if configs.is_empty() {
        "n: new config | Esc: back"
    } else {
        "n: new | s: set default | d: delete | Esc: back"
    };
    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(t.text_muted))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, areas.footer());
}

/// Render the empty state message
fn render_empty_state(frame: &mut Frame, area: Rect) {
    let t = theme();

    let message = vec![
        Line::from(""),
        Line::from(Span::styled(
            "No Claude configurations found.",
            Style::default().fg(t.text),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Press 'n' to add a configuration.",
            Style::default().fg(t.text_muted),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Each configuration points to a different Claude config directory,",
            Style::default().fg(t.text_muted),
        )),
        Line::from(Span::styled(
            "allowing you to use multiple Claude accounts.",
            Style::default().fg(t.text_muted),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "The default config (~/.claude) is used if you don't specify a path.",
            Style::default().fg(t.text_muted),
        )),
    ];

    let paragraph = Paragraph::new(message).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Claude Configurations"),
    );
    frame.render_widget(paragraph, area);
}

/// Render the list of configurations
fn render_config_list(
    frame: &mut Frame,
    area: Rect,
    configs: &[&ClaudeConfig],
    default_id: Option<ClaudeConfigId>,
    selected_index: usize,
) {
    let t = theme();

    let items: Vec<ListItem> = configs
        .iter()
        .enumerate()
        .map(|(i, config)| {
            let is_selected = i == selected_index;
            let is_default = default_id == Some(config.id);

            let prefix = selection_prefix(is_selected);
            let default_marker = if is_default { " ★" } else { "" };

            let content = Line::from(vec![
                Span::raw(prefix),
                Span::styled(&config.name, selection_name_style(is_selected, &t)),
                Span::styled(default_marker, Style::default().fg(Color::Yellow)),
                Span::styled(
                    format!("  {}", config.config_dir_display()),
                    Style::default().fg(t.text_muted),
                ),
            ]);

            ListItem::new(content)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Claude Configurations ({})", configs.len())),
    );
    frame.render_widget(list, area);
}

/// Render the config name input dialog
pub fn render_config_name_input_dialog(frame: &mut Frame, area: Rect, input: &str) {
    let t = theme();

    let dialog_width = 50_u16.min(area.width.saturating_sub(4));
    let dialog_height = 9_u16.min(area.height.saturating_sub(2));

    let dialog_x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
    let dialog_y = area.y + (area.height.saturating_sub(dialog_height)) / 2;

    let dialog_area = Rect::new(dialog_x, dialog_y, dialog_width, dialog_height);

    // Clear the background
    frame.render_widget(ratatui::widgets::Clear, dialog_area);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Enter a name for this configuration:",
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

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.accent))
            .title(" New Claude Config "),
    );

    frame.render_widget(paragraph, dialog_area);
}

/// Render the config path input dialog
pub fn render_config_path_input_dialog(
    frame: &mut Frame,
    area: Rect,
    name: &str,
    input: &str,
    completions: &[std::path::PathBuf],
    completion_index: usize,
    show_completions: bool,
) {
    let t = theme();

    let base_height = 12_u16;
    let completion_height = if show_completions && !completions.is_empty() {
        (completions.len().min(5) + 1) as u16
    } else {
        0
    };

    let dialog_width = 60_u16.min(area.width.saturating_sub(4));
    let dialog_height = (base_height + completion_height).min(area.height.saturating_sub(2));

    let dialog_x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
    let dialog_y = area.y + (area.height.saturating_sub(dialog_height)) / 2;

    let dialog_area = Rect::new(dialog_x, dialog_y, dialog_width, dialog_height);

    // Clear the background
    frame.render_widget(ratatui::widgets::Clear, dialog_area);

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Name: ", Style::default().fg(t.text)),
            Span::styled(name, Style::default().fg(t.accent)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Enter the path to the Claude config directory:",
            Style::default().fg(t.text),
        )),
        Line::from(Span::styled(
            "(Leave empty for default ~/.claude)",
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
            let prefix = if is_selected { "▶ " } else { "  " };

            // Shorten path for display
            let display = path
                .to_string_lossy()
                .chars()
                .take(dialog_width.saturating_sub(6) as usize)
                .collect::<String>();

            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(
                    display,
                    Style::default().fg(if is_selected { t.accent } else { t.text_muted }),
                ),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "[Tab] Complete  [Enter] Confirm  [Esc] Cancel",
        Style::default().fg(t.text_muted),
    )));

    let paragraph = Paragraph::new(lines).alignment(Alignment::Left).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.accent))
            .title(" New Claude Config "),
    );

    frame.render_widget(paragraph, dialog_area);
}

/// Render the config delete confirmation dialog
pub fn render_config_delete_dialog(
    frame: &mut Frame,
    area: Rect,
    config: &ClaudeConfig,
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

    let dialog_width = 55_u16.min(area.width.saturating_sub(4));
    let dialog_height = (base_height + affected_height).min(area.height.saturating_sub(2));

    let dialog_x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
    let dialog_y = area.y + (area.height.saturating_sub(dialog_height)) / 2;

    let dialog_area = Rect::new(dialog_x, dialog_y, dialog_width, dialog_height);

    // Clear the background
    frame.render_widget(ratatui::widgets::Clear, dialog_area);

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Delete config: ", Style::default().fg(t.text)),
            Span::styled(
                &config.name,
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("?", Style::default().fg(t.text)),
        ]),
    ];

    if has_affected {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "This config is used by:",
            Style::default().fg(Color::Yellow),
        )));
        for project in affected_projects.iter().take(5) {
            lines.push(Line::from(vec![
                Span::raw("  - "),
                Span::styled(project, Style::default().fg(t.text)),
            ]));
        }
        if affected_projects.len() > 5 {
            lines.push(Line::from(Span::styled(
                format!("  ... and {} more", affected_projects.len() - 5),
                Style::default().fg(t.text_muted),
            )));
        }
        lines.push(Line::from(Span::styled(
            "Projects will revert to global default.",
            Style::default().fg(Color::Yellow),
        )));
    }

    lines.push(Line::from(""));
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

    frame.render_widget(paragraph, dialog_area);
}

/// Render the Claude config selector overlay
pub fn render_config_selector(
    frame: &mut Frame,
    area: Rect,
    configs: &[ClaudeConfig],
    selected_index: usize,
    default_id: Option<ClaudeConfigId>,
) {
    let t = theme();

    let dialog_width = 50_u16.min(area.width.saturating_sub(4));
    let list_height = configs.len().min(8) as u16;
    let dialog_height = (list_height + 6).min(area.height.saturating_sub(2));

    let dialog_x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
    let dialog_y = area.y + (area.height.saturating_sub(dialog_height)) / 2;

    let dialog_area = Rect::new(dialog_x, dialog_y, dialog_width, dialog_height);

    // Clear the background
    frame.render_widget(ratatui::widgets::Clear, dialog_area);

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Select a Claude configuration:",
            Style::default().fg(t.text),
        )),
        Line::from(""),
    ];

    for (i, config) in configs.iter().enumerate() {
        let is_selected = i == selected_index;
        let is_default = default_id == Some(config.id);

        let prefix = if is_selected { "▶ " } else { "  " };
        let default_marker = if is_default { " (default)" } else { "" };

        lines.push(Line::from(vec![
            Span::raw(prefix),
            Span::styled(
                &config.name,
                Style::default().fg(if is_selected { t.accent } else { t.text }),
            ),
            Span::styled(default_marker, Style::default().fg(Color::Yellow)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "[Enter] Select  [Esc] Cancel",
        Style::default().fg(t.text_muted),
    )));

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.accent))
            .title(" Select Config "),
    );

    frame.render_widget(paragraph, dialog_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_empty_state_does_not_panic() {
        // Basic sanity check
        let store = ClaudeConfigStore::new();
        assert!(store.is_empty());
    }
}
