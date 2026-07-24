//! Claude Code settings dialogs
//!
//! Provides dialogs for copying and migrating Claude Code permissions
//! when creating and deleting worktrees.

use ratatui::prelude::*;

use super::{truncate_path, truncate_string};
use crate::app::{ClaudeSettingsCopyState, ClaudeSettingsMigrateState};
use crate::tui::theme::theme;
use crate::tui::widgets::dialog::{render_dialog, yes_no_line, DialogSize, DialogSpec};

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

    // Base height: title (1) + spacing (1) + source/target (2) + spacing (1) + sources (1-2) + tools header (1) + tools + buttons (2) + footer (1)
    let base_height = 11_u16;
    let tools_height = tools_to_show as u16 + if has_more_tools { 1 } else { 0 };
    let mcp_height = if state.has_mcp_servers { 1 } else { 0 };
    let dialog_height = base_height + tools_height + mcp_height;

    // Truncate paths for display
    let source_display = truncate_path(&state.source_path.to_string_lossy(), 45);
    let target_display = truncate_path(&state.target_path.to_string_lossy(), 45);

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  From: ", Style::default().fg(t.text_dim)),
            Span::styled(source_display, Style::default().fg(t.text)),
        ]),
        Line::from(vec![
            Span::styled("  To:   ", Style::default().fg(t.text_dim)),
            Span::styled(target_display, Style::default().fg(t.accent)),
        ]),
        Line::from(""),
    ];

    // Show which settings sources will be copied
    let sources: Vec<&str> = [
        state.has_local_settings.then_some("Local settings"),
        (!state.tools_preview.is_empty() || state.has_mcp_servers).then_some("Legacy config"),
    ]
    .into_iter()
    .flatten()
    .collect();

    if !sources.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("  Sources: ", Style::default().fg(t.text_dim)),
            Span::styled(sources.join(", "), Style::default().fg(t.text)),
        ]));
    }

    // Add tools preview
    if !state.tools_preview.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Permissions to copy:",
            Style::default().fg(t.text).add_modifier(Modifier::BOLD),
        )));

        for tool in state.tools_preview.iter().take(max_tool_lines) {
            let display_tool = truncate_string(tool, 50);
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(display_tool, Style::default().fg(t.success)),
            ]));
        }

        if has_more_tools {
            let remaining = state.tools_preview.len() - max_tool_lines;
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    format!("...and {} more", remaining),
                    Style::default().fg(t.text_dim),
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
                Style::default().fg(t.text_dim),
            ),
        ]));
    }

    lines.push(Line::from(""));

    // Yes/No buttons
    lines.push(yes_no_line(state.selected_yes, "           "));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter: confirm | Left/Right: toggle | Esc: skip",
        Style::default().fg(t.text_dim),
    )));

    render_dialog(
        frame,
        area,
        DialogSpec {
            title: " Copy Claude Code Settings? ",
            border_color: t.accent,
            alignment: Alignment::Left,
            width: DialogSize::Fixed(60),
            height: DialogSize::Fixed(dialog_height),
        },
        lines,
    );
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

    // Base height: title (1) + spacing (1) + message (2) + spacing (1) + sources (1) + tools header (1) + tools + buttons (2) + footer (1)
    let base_height = 12_u16;
    let tools_height = if state.unique_tools.is_empty() {
        0
    } else {
        tools_to_show as u16 + if has_more_tools { 1 } else { 0 } + 1 // +1 for header
    };
    let dialog_height = base_height + tools_height;

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  This worktree has unique Claude Code settings",
            Style::default().fg(t.text),
        )),
        Line::from(Span::styled(
            "  not in the main repository.",
            Style::default().fg(t.text),
        )),
        Line::from(""),
    ];

    // Show which settings sources will be migrated
    let sources: Vec<&str> = [
        state.has_local_settings.then_some("Local settings"),
        (!state.unique_tools.is_empty()).then_some("Legacy permissions"),
    ]
    .into_iter()
    .flatten()
    .collect();

    if !sources.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("  Sources: ", Style::default().fg(t.text_dim)),
            Span::styled(sources.join(", "), Style::default().fg(t.text)),
        ]));
    }

    // Add unique tools (legacy format) if present
    if !state.unique_tools.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Permissions to migrate:",
            Style::default().fg(t.text).add_modifier(Modifier::BOLD),
        )));

        for tool in state.unique_tools.iter().take(max_tool_lines) {
            let display_tool = truncate_string(tool, 50);
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(display_tool, Style::default().fg(t.warning)),
            ]));
        }

        if has_more_tools {
            let remaining = state.unique_tools.len() - max_tool_lines;
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    format!("...and {} more", remaining),
                    Style::default().fg(t.text_dim),
                ),
            ]));
        }
    }

    lines.push(Line::from(""));

    // Yes/No buttons
    lines.push(yes_no_line(state.selected_yes, "           "));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter: continue | Left/Right: toggle | Esc: cancel",
        Style::default().fg(t.text_dim),
    )));

    render_dialog(
        frame,
        area,
        DialogSpec {
            title: " Migrate Permissions? ",
            border_color: t.border_warning,
            alignment: Alignment::Left,
            width: DialogSize::Fixed(60),
            height: DialogSize::Fixed(dialog_height),
        },
        lines,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::views::test_util::{contains_line, render_to_lines};
    use std::path::PathBuf;
    use uuid::Uuid;

    #[test]
    fn test_copy_dialog_lists_permissions_and_buttons() {
        let state = ClaudeSettingsCopyState {
            source_path: PathBuf::from("/tmp/main-repo"),
            target_path: PathBuf::from("/tmp/worktree"),
            project_id: Uuid::new_v4(),
            branch_id: Uuid::new_v4(),
            tools_preview: vec!["Bash(ls)".to_string()],
            has_mcp_servers: true,
            selected_yes: true,
            claude_config_dir: None,
            has_local_settings: false,
        };

        let lines = render_to_lines(80, 24, |frame| {
            render_claude_settings_copy_dialog(frame, frame.size(), &state)
        });

        assert!(
            contains_line(&lines, "Copy Claude Code Settings?"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "Permissions to copy:"), "{:?}", lines);
        assert!(contains_line(&lines, "Bash(ls)"), "{:?}", lines);
        assert!(
            contains_line(&lines, "MCP servers will also be copied"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "Yes"), "{:?}", lines);
        assert!(contains_line(&lines, "No"), "{:?}", lines);
    }

    #[test]
    fn test_migrate_dialog_explains_unique_settings() {
        let state = ClaudeSettingsMigrateState {
            worktree_path: PathBuf::from("/tmp/worktree"),
            main_path: PathBuf::from("/tmp/main-repo"),
            branch_id: Uuid::new_v4(),
            unique_tools: vec!["Bash(cargo test)".to_string()],
            selected_yes: true,
            claude_config_dir: None,
            has_local_settings: false,
        };

        let lines = render_to_lines(80, 24, |frame| {
            render_claude_settings_migrate_dialog(frame, frame.size(), &state)
        });

        assert!(contains_line(&lines, "Migrate Permissions?"), "{:?}", lines);
        assert!(
            contains_line(&lines, "This worktree has unique Claude Code settings"),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "Permissions to migrate:"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "Bash(cargo test)"), "{:?}", lines);
    }
}
