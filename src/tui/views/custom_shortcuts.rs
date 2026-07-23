//! Custom shortcuts dialog rendering
//!
//! Renders dialogs for managing custom shell session shortcuts.

use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, Paragraph};

use crate::app::{AppState, InputMode};
use crate::config::{reserved_keys_display, Config};
use crate::tui::theme::theme;
use crate::tui::widgets::dialog::{render_dialog, yes_no_line, DialogSize, DialogSpec};
use crate::tui::widgets::selection::{selection_prefix, selection_style_with_accent};

/// Width of the wider dialogs (management, command, auto-close)
const WIDE: DialogSize = DialogSize::Percent {
    pct: 60,
    min: 40,
    max: 60,
};

/// Width of the narrower input dialogs (key, name)
const NARROW: DialogSize = DialogSize::Percent {
    pct: 50,
    min: 35,
    max: 50,
};

/// Render the custom shortcuts list into a settings-pane rect
///
/// The pane owns the border and title, so this draws rows only. Fields are
/// dropped whole as the pane narrows rather than truncating one long string.
pub fn render_shortcuts_list(frame: &mut Frame, area: Rect, config: &Config, selected: usize) {
    let t = theme();

    if config.custom_shortcuts.is_empty() {
        frame.render_widget(
            Paragraph::new(
                "No custom shortcuts yet.\n\n\
                 Press 'n' to bind a key to a shell command.",
            )
            .style(Style::default().fg(t.text_muted)),
            area,
        );
        return;
    }

    let width = area.width as usize;
    let show_command = width >= 30;

    let items: Vec<ListItem> = config
        .custom_shortcuts
        .iter()
        .enumerate()
        .map(|(i, shortcut)| {
            let is_selected = i == selected;
            let name_display = if shortcut.name.is_empty() {
                "-".to_string()
            } else {
                shortcut.name.clone()
            };
            let auto_close = if shortcut.auto_close { " [AC]" } else { "" };

            let content = if show_command {
                format!(
                    "{}{}  {}  {}{}",
                    selection_prefix(is_selected),
                    shortcut.key,
                    name_display,
                    shortcut.command,
                    auto_close
                )
            } else {
                format!(
                    "{}{}  {}{}",
                    selection_prefix(is_selected),
                    shortcut.key,
                    name_display,
                    auto_close
                )
            };

            ListItem::new(crate::tui::views::truncate_string(&content, width))
                .style(selection_style_with_accent(is_selected, t))
        })
        .collect();

    frame.render_widget(List::new(items), area);
}

/// Render the add shortcut key input dialog
pub fn render_add_shortcut_key_dialog(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();

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

    render_dialog(
        frame,
        area,
        DialogSpec {
            title: " Add Custom Shortcut ",
            border_color: t.accent,
            alignment: Alignment::Center,
            width: NARROW,
            height: DialogSize::Fixed(10),
        },
        lines,
    );
}

/// Render the add shortcut name input dialog
pub fn render_add_shortcut_name_dialog(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();

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

    render_dialog(
        frame,
        area,
        DialogSpec {
            title: " Add Custom Shortcut ",
            border_color: t.accent,
            alignment: Alignment::Center,
            width: NARROW,
            height: DialogSize::Fixed(10),
        },
        lines,
    );
}

/// Render the add shortcut command input dialog
pub fn render_add_shortcut_command_dialog(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();

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

    render_dialog(
        frame,
        area,
        DialogSpec {
            title: " Add Custom Shortcut ",
            border_color: t.accent,
            alignment: Alignment::Center,
            width: WIDE,
            height: DialogSize::Fixed(12),
        },
        lines,
    );
}

/// Render the add shortcut auto-close toggle dialog
pub fn render_add_shortcut_auto_close_dialog(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();

    let key_display = state
        .new_shortcut_key
        .map(|c| c.to_string())
        .unwrap_or_else(|| "?".to_string());
    let name_display = if state.new_shortcut_name.is_empty() {
        "(auto)".to_string()
    } else {
        state.new_shortcut_name.clone()
    };

    let lines = vec![
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
        Line::from(vec![
            Span::styled("Cmd: ", Style::default().fg(t.text_muted)),
            Span::styled(&state.new_shortcut_command, Style::default().fg(t.text)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Auto-close after command?",
            Style::default().fg(t.text),
        )),
        Line::from(""),
        yes_no_line(state.new_shortcut_auto_close, ""),
        Line::from(""),
        Line::from(Span::styled(
            "Tab: toggle | y/n: select | Enter: save | Esc: back",
            Style::default().fg(t.text_muted),
        )),
    ];

    render_dialog(
        frame,
        area,
        DialogSpec {
            title: " Add Custom Shortcut ",
            border_color: t.accent,
            alignment: Alignment::Center,
            width: WIDE,
            height: DialogSize::Fixed(12),
        },
        lines,
    );
}

/// Render the delete shortcut confirmation dialog
pub fn render_delete_shortcut_confirm_dialog(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    config: &Config,
) {
    let t = theme();

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

    render_dialog(
        frame,
        area,
        DialogSpec {
            title: " Confirm Delete ",
            border_color: t.border_warning,
            alignment: Alignment::Center,
            width: DialogSize::Percent {
                pct: 40,
                min: 30,
                max: 45,
            },
            height: DialogSize::Fixed(8),
        },
        lines,
    );
}

/// Render the appropriate custom shortcut dialog based on input mode
pub fn render_custom_shortcut_dialogs(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    config: &Config,
) {
    match state.input_mode {
        InputMode::AddingCustomShortcutKey => {
            render_add_shortcut_key_dialog(frame, area, state);
        }
        InputMode::AddingCustomShortcutName => {
            render_add_shortcut_name_dialog(frame, area, state);
        }
        InputMode::AddingCustomShortcutCommand => {
            render_add_shortcut_command_dialog(frame, area, state);
        }
        InputMode::AddingCustomShortcutAutoClose => {
            render_add_shortcut_auto_close_dialog(frame, area, state);
        }
        InputMode::ConfirmingCustomShortcutDelete => {
            render_delete_shortcut_confirm_dialog(frame, area, state, config);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::views::test_util::{contains_line, render_to_lines};

    fn render_in_mode(mode: InputMode, config: &Config) -> Vec<String> {
        let state = AppState {
            input_mode: mode,
            ..Default::default()
        };
        render_to_lines(80, 24, |frame| {
            render_custom_shortcut_dialogs(frame, frame.size(), &state, config)
        })
    }

    #[test]
    fn test_shortcuts_list_empty_state_points_at_the_add_key() {
        let lines = render_to_lines(40, 10, |frame| {
            render_shortcuts_list(frame, frame.size(), &Config::default(), 0)
        });

        assert!(
            contains_line(&lines, "No custom shortcuts yet."),
            "{:?}",
            lines
        );
    }

    #[test]
    fn test_shortcuts_list_drops_the_command_in_a_narrow_pane() {
        let mut config = Config::default();
        config
            .custom_shortcuts
            .push(crate::config::CustomShortcut::new(
                'v',
                "VSCode".to_string(),
                "code . &".to_string(),
                true,
            ));

        let wide = render_to_lines(60, 10, |frame| {
            render_shortcuts_list(frame, frame.size(), &config, 0)
        });
        assert!(contains_line(&wide, "v  VSCode  code . & [AC]"), "{wide:?}");

        let narrow = render_to_lines(24, 10, |frame| {
            render_shortcuts_list(frame, frame.size(), &config, 0)
        });
        assert!(contains_line(&narrow, "v  VSCode [AC]"), "{narrow:?}");
        for line in &narrow {
            assert!(line.chars().count() <= 24, "{line:?}");
        }
    }

    #[test]
    fn test_add_key_dialog_shows_reserved_keys() {
        let lines = render_in_mode(InputMode::AddingCustomShortcutKey, &Config::default());

        assert!(
            contains_line(&lines, "Press the key for this shortcut:"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "(Reserved:"), "{:?}", lines);
    }

    #[test]
    fn test_auto_close_dialog_offers_yes_no() {
        let lines = render_in_mode(InputMode::AddingCustomShortcutAutoClose, &Config::default());

        assert!(
            contains_line(&lines, "Auto-close after command?"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "Yes"), "{:?}", lines);
        assert!(contains_line(&lines, "No"), "{:?}", lines);
    }

    #[test]
    fn test_delete_dialog_asks_for_confirmation() {
        let lines = render_in_mode(
            InputMode::ConfirmingCustomShortcutDelete,
            &Config::default(),
        );

        assert!(contains_line(&lines, "Delete shortcut?"), "{:?}", lines);
        assert!(
            contains_line(&lines, "y: confirm | n/Esc: cancel"),
            "{:?}",
            lines
        );
    }
}
