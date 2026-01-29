//! Custom shortcuts dialog rendering
//!
//! Renders dialogs for managing custom shell session shortcuts.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

use crate::app::{AppState, InputMode};
use crate::config::{reserved_keys_display, Config};
use crate::tui::theme::theme;
use crate::tui::widgets::selection::{selection_prefix, selection_style_with_accent};

/// Render the custom shortcuts management dialog
pub fn render_custom_shortcuts_dialog(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    config: &Config,
) {
    let t = theme();

    // Calculate centered dialog (60% width max 60 chars, 50% height max 20 lines)
    let width = (area.width * 60 / 100).clamp(40, 60);
    let height = (area.height * 50 / 100).clamp(10, 20);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    // Clear background
    frame.render_widget(Clear, dialog_area);

    // Split dialog into content and footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(dialog_area);

    let content_area = chunks[0];
    let footer_area = chunks[1];

    // Render list of shortcuts or empty message
    if config.custom_shortcuts.is_empty() {
        let empty_msg = Paragraph::new("No custom shortcuts defined.\n\nPress 'n' to add one.")
            .style(Style::default().fg(t.text_muted))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.accent))
                    .title(" Custom Shortcuts "),
            );
        frame.render_widget(empty_msg, content_area);
    } else {
        let items: Vec<ListItem> = config
            .custom_shortcuts
            .iter()
            .enumerate()
            .map(|(i, shortcut)| {
                let selected = i == state.custom_shortcuts_selected;
                let prefix = selection_prefix(selected);

                // Format: "  v  VSCode    code . &"
                let name_display = if shortcut.name.is_empty() {
                    "-".to_string()
                } else {
                    shortcut.name.clone()
                };

                // Truncate command for display
                let cmd_display: String = shortcut.command.chars().take(25).collect();
                let cmd_suffix = if shortcut.command.chars().count() > 25 {
                    "..."
                } else {
                    ""
                };

                let content = format!(
                    "{}{}  {:10}  {}{}",
                    prefix, shortcut.key, name_display, cmd_display, cmd_suffix
                );

                let style = selection_style_with_accent(selected, t);
                ListItem::new(content).style(style)
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.accent))
                .title(" Custom Shortcuts "),
        );
        frame.render_widget(list, content_area);
    }

    // Footer with instructions
    let footer = Paragraph::new("n: add | d: delete | j/k: navigate | Esc: close")
        .style(Style::default().fg(t.text_muted))
        .alignment(Alignment::Center);
    frame.render_widget(footer, footer_area);
}

/// Render the add shortcut key input dialog
pub fn render_add_shortcut_key_dialog(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();

    // Calculate centered dialog
    let width = (area.width * 50 / 100).clamp(35, 50);
    let height = 10;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    // Clear background
    frame.render_widget(Clear, dialog_area);

    // Build content
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Press the key for this shortcut:",
            Style::default().fg(t.text),
        )),
        Line::from(""),
    ];

    // Show error if any
    if let Some(error) = &state.shortcut_error {
        lines.push(Line::from(Span::styled(
            error,
            Style::default().fg(t.error_bg),
        )));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        format!("(Reserved: {})", reserved_keys_display()),
        Style::default().fg(t.text_muted),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Esc: cancel",
        Style::default().fg(t.text_muted),
    )));

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.accent))
            .title(" Add Custom Shortcut "),
    );

    frame.render_widget(paragraph, dialog_area);
}

/// Render the add shortcut name input dialog
pub fn render_add_shortcut_name_dialog(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();

    // Calculate centered dialog
    let width = (area.width * 50 / 100).clamp(35, 50);
    let height = 10;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    // Clear background
    frame.render_widget(Clear, dialog_area);

    let key_display = state
        .new_shortcut_key
        .map(|c| c.to_string())
        .unwrap_or_else(|| "?".to_string());

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Key: ", Style::default().fg(t.text_muted)),
            Span::styled(
                &key_display,
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Display name (optional):",
            Style::default().fg(t.text),
        )),
        Line::from(Span::styled(
            format!("{}_", state.new_shortcut_name),
            Style::default().fg(t.text),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Enter: continue | Esc: cancel",
            Style::default().fg(t.text_muted),
        )),
    ];

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.accent))
            .title(" Add Custom Shortcut "),
    );

    frame.render_widget(paragraph, dialog_area);
}

/// Render the add shortcut command input dialog
pub fn render_add_shortcut_command_dialog(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();

    // Calculate centered dialog
    let width = (area.width * 60 / 100).clamp(40, 60);
    let height = 12;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    // Clear background
    frame.render_widget(Clear, dialog_area);

    let key_display = state
        .new_shortcut_key
        .map(|c| c.to_string())
        .unwrap_or_else(|| "?".to_string());
    let name_display = if state.new_shortcut_name.is_empty() {
        "(auto)".to_string()
    } else {
        state.new_shortcut_name.clone()
    };

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Key: ", Style::default().fg(t.text_muted)),
            Span::styled(
                &key_display,
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::raw("     "),
            Span::styled("Name: ", Style::default().fg(t.text_muted)),
            Span::styled(&name_display, Style::default().fg(t.text)),
        ]),
        Line::from(""),
        Line::from(Span::styled("Command to run:", Style::default().fg(t.text))),
        Line::from(Span::styled(
            format!("{}_", state.new_shortcut_command),
            Style::default().fg(t.text),
        )),
    ];

    // Show error if any
    if let Some(error) = &state.shortcut_error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            error,
            Style::default().fg(t.error_bg),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter: save | Esc: cancel",
        Style::default().fg(t.text_muted),
    )));

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.accent))
            .title(" Add Custom Shortcut "),
    );

    frame.render_widget(paragraph, dialog_area);
}

/// Render the delete shortcut confirmation dialog
pub fn render_delete_shortcut_confirm_dialog(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    config: &Config,
) {
    let t = theme();

    // Calculate centered dialog
    let width = (area.width * 40 / 100).clamp(30, 45);
    let height = 8;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    // Clear background
    frame.render_widget(Clear, dialog_area);

    // Get shortcut info
    let shortcut_info = state
        .pending_delete_shortcut_index
        .and_then(|i| config.custom_shortcuts.get(i))
        .map(|s| format!("'{}' ({})", s.key, s.display_name()))
        .unwrap_or_else(|| "?".to_string());

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Delete shortcut?",
            Style::default().fg(t.text),
        )),
        Line::from(""),
        Line::from(Span::styled(&shortcut_info, Style::default().fg(t.accent))),
        Line::from(""),
        Line::from(Span::styled(
            "y: confirm | n/Esc: cancel",
            Style::default().fg(t.text_muted),
        )),
    ];

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border_warning))
            .title(" Confirm Delete "),
    );

    frame.render_widget(paragraph, dialog_area);
}

/// Render the appropriate custom shortcut dialog based on input mode
pub fn render_custom_shortcut_dialogs(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    config: &Config,
) {
    match state.input_mode {
        InputMode::ManagingCustomShortcuts => {
            render_custom_shortcuts_dialog(frame, area, state, config);
        }
        InputMode::AddingCustomShortcutKey => {
            render_add_shortcut_key_dialog(frame, area, state);
        }
        InputMode::AddingCustomShortcutName => {
            render_add_shortcut_name_dialog(frame, area, state);
        }
        InputMode::AddingCustomShortcutCommand => {
            render_add_shortcut_command_dialog(frame, area, state);
        }
        InputMode::ConfirmingCustomShortcutDelete => {
            render_delete_shortcut_confirm_dialog(frame, area, state, config);
        }
        _ => {}
    }
}
