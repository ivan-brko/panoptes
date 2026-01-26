//! Claude Code settings dialogs
//!
//! Provides dialogs for copying and migrating Claude Code permissions
//! when creating and deleting worktrees.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{ClaudeSettingsCopyState, ClaudeSettingsMigrateState};
use crate::tui::theme::theme;

/// Render the Claude settings copy dialog
///
/// Shown after worktree creation when the main repo has Claude settings configured.
pub fn render_claude_settings_copy_dialog(
    frame: &mut Frame,
    area: Rect,
    state: &ClaudeSettingsCopyState,
) {
    let t = theme();

    // Calculate dialog size based on content
    let max_tool_lines = 5;
    let tools_to_show = state.tools_preview.len().min(max_tool_lines);
    let has_more_tools = state.tools_preview.len() > max_tool_lines;

    // Base height: title (1) + spacing (1) + source/target (2) + spacing (1) + tools header (1) + tools + buttons (2) + footer (1)
    let base_height = 10_u16;
    let tools_height = tools_to_show as u16 + if has_more_tools { 1 } else { 0 };
    let mcp_height = if state.has_mcp_servers { 1 } else { 0 };
    let dialog_height =
        (base_height + tools_height + mcp_height).min(area.height.saturating_sub(2));
    let dialog_width = 60_u16.min(area.width.saturating_sub(4));

    let dialog_x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
    let dialog_y = area.y + (area.height.saturating_sub(dialog_height)) / 2;

    let dialog_area = Rect::new(dialog_x, dialog_y, dialog_width, dialog_height);

    // Clear the background
    frame.render_widget(Clear, dialog_area);

    // Truncate paths for display
    let source_display = truncate_path(&state.source_path.to_string_lossy(), 45);
    let target_display = truncate_path(&state.target_path.to_string_lossy(), 45);

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  From: ", Style::default().fg(t.text_muted)),
            Span::styled(source_display, Style::default().fg(t.text)),
        ]),
        Line::from(vec![
            Span::styled("  To:   ", Style::default().fg(t.text_muted)),
            Span::styled(target_display, Style::default().fg(t.accent)),
        ]),
        Line::from(""),
    ];

    // Add tools preview
    if !state.tools_preview.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Tools to copy:",
            Style::default().fg(t.text).add_modifier(Modifier::BOLD),
        )));

        for tool in state.tools_preview.iter().take(max_tool_lines) {
            let display_tool = truncate_string(tool, 50);
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(display_tool, Style::default().fg(Color::Green)),
            ]));
        }

        if has_more_tools {
            let remaining = state.tools_preview.len() - max_tool_lines;
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    format!("...and {} more", remaining),
                    Style::default().fg(t.text_muted),
                ),
            ]));
        }
    }

    // Mention MCP servers if present
    if state.has_mcp_servers {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "MCP servers will also be copied",
                Style::default().fg(t.text_muted),
            ),
        ]));
    }

    lines.push(Line::from(""));

    // Yes/No buttons
    let yes_style = if state.selected_yes {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    };

    let no_style = if !state.selected_yes {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red)
    };

    lines.push(Line::from(vec![
        Span::raw("           "),
        Span::styled(" Yes ", yes_style),
        Span::raw("    "),
        Span::styled(" No ", no_style),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter: confirm | Left/Right: toggle | Esc: skip",
        Style::default().fg(t.text_muted),
    )));

    let paragraph = Paragraph::new(lines).alignment(Alignment::Left).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.accent))
            .title(" Copy Claude Code Settings? "),
    );

    frame.render_widget(paragraph, dialog_area);
}

/// Render the Claude settings migrate dialog
///
/// Shown before worktree deletion when the worktree has unique permissions
/// not present in the main repository.
pub fn render_claude_settings_migrate_dialog(
    frame: &mut Frame,
    area: Rect,
    state: &ClaudeSettingsMigrateState,
) {
    let t = theme();

    // Calculate dialog size based on content
    let max_tool_lines = 5;
    let tools_to_show = state.unique_tools.len().min(max_tool_lines);
    let has_more_tools = state.unique_tools.len() > max_tool_lines;

    // Base height: title (1) + spacing (1) + message (2) + spacing (1) + tools header (1) + tools + buttons (2) + footer (1)
    let base_height = 11_u16;
    let tools_height = tools_to_show as u16 + if has_more_tools { 1 } else { 0 };
    let dialog_height = (base_height + tools_height).min(area.height.saturating_sub(2));
    let dialog_width = 60_u16.min(area.width.saturating_sub(4));

    let dialog_x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
    let dialog_y = area.y + (area.height.saturating_sub(dialog_height)) / 2;

    let dialog_area = Rect::new(dialog_x, dialog_y, dialog_width, dialog_height);

    // Clear the background
    frame.render_widget(Clear, dialog_area);

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  This worktree has unique Claude Code permissions",
            Style::default().fg(t.text),
        )),
        Line::from(Span::styled(
            "  not in the main repository.",
            Style::default().fg(t.text),
        )),
        Line::from(""),
    ];

    // Add unique tools
    lines.push(Line::from(Span::styled(
        "  Permissions to migrate:",
        Style::default().fg(t.text).add_modifier(Modifier::BOLD),
    )));

    for tool in state.unique_tools.iter().take(max_tool_lines) {
        let display_tool = truncate_string(tool, 50);
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(display_tool, Style::default().fg(Color::Yellow)),
        ]));
    }

    if has_more_tools {
        let remaining = state.unique_tools.len() - max_tool_lines;
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                format!("...and {} more", remaining),
                Style::default().fg(t.text_muted),
            ),
        ]));
    }

    lines.push(Line::from(""));

    // Yes/No buttons
    let yes_style = if state.selected_yes {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    };

    let no_style = if !state.selected_yes {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red)
    };

    lines.push(Line::from(vec![
        Span::raw("           "),
        Span::styled(" Yes ", yes_style),
        Span::raw("    "),
        Span::styled(" No ", no_style),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter: continue | Left/Right: toggle | Esc: cancel",
        Style::default().fg(t.text_muted),
    )));

    let paragraph = Paragraph::new(lines).alignment(Alignment::Left).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border_warning))
            .title(" Migrate Permissions? "),
    );

    frame.render_widget(paragraph, dialog_area);
}

/// Truncate a path for display, keeping the end (most relevant part)
fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        path.to_string()
    } else {
        format!("...{}", &path[path.len() - max_len + 3..])
    }
}

/// Truncate a string for display, keeping the beginning
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}
