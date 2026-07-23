//! Help overlay showing keyboard shortcuts for the current view
//!
//! Displays a modal overlay with keybindings specific to the current view.
//! Dismissible with `?` or `Esc`.

use ratatui::prelude::*;

use crate::app::View;
use crate::tui::theme::theme;
use crate::tui::widgets::dialog::{render_dialog, DialogSize, DialogSpec};

/// Render the help overlay showing keyboard shortcuts for the current view
pub fn render_help_overlay(frame: &mut Frame, area: Rect, current_view: &View) {
    let t = theme();

    // Get shortcuts for current view
    let (title, content) = get_shortcuts_for_view(current_view);

    // Centered dialog (70% width max 70 chars, 60% height max 25 lines)
    render_dialog(
        frame,
        area,
        DialogSpec {
            title: &format!(" {} ", title),
            border_color: t.accent,
            alignment: Alignment::Left,
            width: DialogSize::Percent {
                pct: 70,
                min: 40,
                max: 70,
            },
            height: DialogSize::Percent {
                pct: 60,
                min: 10,
                max: 25,
            },
        },
        content,
    );
}

/// Get the title and shortcuts content for a specific view
fn get_shortcuts_for_view(view: &View) -> (&'static str, Vec<Line<'static>>) {
    match view {
        View::ProjectsOverview => (
            "Keyboard Shortcuts - Projects",
            projects_overview_shortcuts(),
        ),
        View::ProjectDetail(_) => ("Keyboard Shortcuts - Project", project_detail_shortcuts()),
        View::BranchDetail(_, _) => ("Keyboard Shortcuts - Branch", branch_detail_shortcuts()),
        View::SessionView => ("Keyboard Shortcuts - Session", session_view_shortcuts()),
        View::LogViewer => ("Keyboard Shortcuts - Logs", log_viewer_shortcuts()),
        View::ClaudeConfigs => (
            "Keyboard Shortcuts - Claude Configs",
            claude_configs_shortcuts(),
        ),
        View::CodexConfigs => (
            "Keyboard Shortcuts - Codex Configs",
            codex_configs_shortcuts(),
        ),
    }
}

/// Format a shortcut line with key and description
fn shortcut_line(key: &'static str, desc: &'static str) -> Line<'static> {
    let t = theme();
    Line::from(vec![
        Span::styled(
            format!("{:>12}", key),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(desc, Style::default().fg(t.text)),
    ])
}

/// Format a section header
fn section_header(title: &'static str) -> Line<'static> {
    let t = theme();
    Line::from(vec![Span::styled(
        title,
        Style::default()
            .fg(t.text)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    )])
}

/// Empty line for spacing
fn empty_line() -> Line<'static> {
    Line::from("")
}

/// Footer hint
fn footer_hint() -> Line<'static> {
    let t = theme();
    Line::from(vec![Span::styled(
        "Press ? or Esc to close",
        Style::default().fg(t.text_muted),
    )])
}

fn projects_overview_shortcuts() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Navigation"),
        shortcut_line("Down", "Move down"),
        shortcut_line("Up", "Move up"),
        shortcut_line("Enter", "Open project, or expand/collapse folder"),
        shortcut_line("Right", "Expand folder"),
        shortcut_line("Left", "Collapse folder / go to parent"),
        shortcut_line("Tab", "Switch focus (projects/sessions)"),
        shortcut_line("1-9", "Jump to session by number (Sessions list)"),
        empty_line(),
        section_header("Folders"),
        shortcut_line("Enter", "Expand or collapse selected folder"),
        shortcut_line("m", "Move project/folder into a folder"),
        shortcut_line("r", "Rename selected folder"),
        shortcut_line("d", "Remove folder (contents move up)"),
        empty_line(),
        section_header("Actions"),
        shortcut_line("n", "Add new project"),
        shortcut_line("d", "Delete selected project"),
        shortcut_line("c", "Claude configs"),
        shortcut_line("x", "Codex configs"),
        shortcut_line("k", "Custom shortcuts"),
        shortcut_line("l", "View logs"),
        shortcut_line("R", "Refresh git state"),
        shortcut_line("Space", "Jump to next session needing attention"),
        shortcut_line("?", "Toggle this help"),
        empty_line(),
        shortcut_line("Esc", "Quit (asks to confirm)"),
        empty_line(),
        footer_hint(),
    ]
}

fn project_detail_shortcuts() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Navigation"),
        shortcut_line("Down", "Move down"),
        shortcut_line("Up", "Move up"),
        shortcut_line("Enter", "Select branch"),
        shortcut_line("Esc", "Back to projects"),
        shortcut_line("1-9", "Jump to item by number"),
        empty_line(),
        section_header("Actions"),
        shortcut_line("n", "Create new worktree"),
        shortcut_line("d", "Delete selected branch"),
        shortcut_line("b", "Set default base branch"),
        shortcut_line("c", "Set Claude config"),
        shortcut_line("x", "Set Codex config"),
        shortcut_line("r", "Rename project"),
        shortcut_line("R", "Refresh branches"),
        shortcut_line("k", "Custom shortcuts"),
        shortcut_line("Space", "Jump to next session needing attention"),
        shortcut_line("?", "Toggle this help"),
        empty_line(),
        footer_hint(),
    ]
}

fn branch_detail_shortcuts() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Navigation"),
        shortcut_line("Down", "Move down"),
        shortcut_line("Up", "Move up"),
        shortcut_line("Enter", "Open session (resumes if [Resumable])"),
        shortcut_line("1-9", "Jump to session by number (0 = 10)"),
        shortcut_line("Esc", "Back to project"),
        empty_line(),
        section_header("Actions"),
        shortcut_line("n", "New AI session (Claude/Codex)"),
        shortcut_line("s", "New shell session"),
        shortcut_line("d", "Delete session (or discard a resumable one)"),
        shortcut_line("k", "Custom shortcuts"),
        shortcut_line("<key>", "Run a custom shortcut"),
        shortcut_line("Space", "Jump to next session needing attention"),
        shortcut_line("?", "Toggle this help"),
        empty_line(),
        footer_hint(),
    ]
}

fn session_view_shortcuts() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Mode Switching"),
        shortcut_line("Enter", "Enter session mode (type in PTY)"),
        shortcut_line("Esc", "Exit session mode / Go back"),
        empty_line(),
        section_header("Navigation (Normal Mode)"),
        shortcut_line("Tab", "Next session"),
        shortcut_line("1-9", "Switch to session by number (0 = 10)"),
        shortcut_line("Space", "Jump to next session needing attention"),
        shortcut_line("?", "Toggle this help"),
        empty_line(),
        section_header("Scrollback (Normal Mode)"),
        shortcut_line("Up / Down", "Scroll up/down (3 lines)"),
        shortcut_line("Page Up", "Scroll up (full page)"),
        shortcut_line("Page Down", "Scroll down (full page)"),
        shortcut_line("Home", "Jump to top"),
        shortcut_line("End", "Jump to bottom (live)"),
        empty_line(),
        section_header("Custom Shortcuts (Normal Mode)"),
        shortcut_line("k", "Manage custom shortcuts"),
        shortcut_line("<key>", "Run shortcut (e.g., 'v' for VSCode)"),
        empty_line(),
        section_header("Session Mode"),
        shortcut_line("All keys", "Forwarded to session"),
        shortcut_line("\u{21E7}Esc", "Send Esc to the agent"),
        shortcut_line("Ctrl+End", "Jump back to live view"),
        shortcut_line("Mouse scroll", "Scroll when PTY supports it"),
        empty_line(),
        footer_hint(),
    ]
}

fn log_viewer_shortcuts() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Navigation"),
        shortcut_line("Down", "Scroll down"),
        shortcut_line("Up", "Scroll up"),
        shortcut_line("Page Down", "Page down"),
        shortcut_line("Page Up", "Page up"),
        shortcut_line("g", "Jump to start"),
        shortcut_line("G", "Jump to end (resumes live tail)"),
        shortcut_line("?", "Toggle this help"),
        shortcut_line("Esc", "Back to projects"),
        empty_line(),
        footer_hint(),
    ]
}

fn codex_configs_shortcuts() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Navigation"),
        shortcut_line("Down", "Move down"),
        shortcut_line("Up", "Move up"),
        shortcut_line("Esc", "Back to projects"),
        empty_line(),
        section_header("Actions"),
        shortcut_line("n", "Add new config"),
        shortcut_line("d", "Delete selected"),
        shortcut_line("s", "Set as default"),
        shortcut_line("?", "Toggle this help"),
        empty_line(),
        footer_hint(),
    ]
}

fn claude_configs_shortcuts() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Navigation"),
        shortcut_line("Down", "Move down"),
        shortcut_line("Up", "Move up"),
        shortcut_line("Esc", "Back to projects"),
        empty_line(),
        section_header("Actions"),
        shortcut_line("n", "Add new config"),
        shortcut_line("d", "Delete selected"),
        shortcut_line("s", "Set as default"),
        shortcut_line("?", "Toggle this help"),
        empty_line(),
        footer_hint(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::views::test_util::{contains_line, render_to_lines};

    #[test]
    fn test_overlay_renders_shortcuts_for_each_view() {
        let views = [
            (View::ProjectsOverview, "Keyboard Shortcuts - Projects"),
            (View::SessionView, "Keyboard Shortcuts - Session"),
            (View::LogViewer, "Keyboard Shortcuts - Logs"),
            (View::ClaudeConfigs, "Keyboard Shortcuts - Claude Configs"),
            (View::CodexConfigs, "Keyboard Shortcuts - Codex Configs"),
        ];

        for (view, title) in views {
            let lines = render_to_lines(80, 30, |frame| {
                render_help_overlay(frame, frame.size(), &view)
            });

            // Long lists clip at the dialog height, so assert on the title
            // and a section header near the top rather than the footer
            assert!(contains_line(&lines, title), "{:?}", lines);
            assert!(contains_line(&lines, "Navigation"), "{:?}", lines);
        }
    }
}
