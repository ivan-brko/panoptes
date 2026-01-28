//! Help overlay showing keyboard shortcuts for the current view
//!
//! Displays a modal overlay with keybindings specific to the current view.
//! Dismissible with `?` or `Esc`.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::View;
use crate::tui::theme::theme;

/// Render the help overlay showing keyboard shortcuts for the current view
pub fn render_help_overlay(frame: &mut Frame, area: Rect, current_view: &View) {
    let t = theme();

    // Get shortcuts for current view
    let (title, content) = get_shortcuts_for_view(current_view);

    // Calculate centered dialog (70% width max 70 chars, 60% height max 25 lines)
    let width = (area.width * 70 / 100).clamp(40, 70);
    let height = (area.height * 60 / 100).clamp(10, 25);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    // Clear background
    frame.render_widget(Clear, dialog_area);

    // Render content
    let paragraph = Paragraph::new(content).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.accent))
            .title(format!(" {} ", title)),
    );

    frame.render_widget(paragraph, dialog_area);
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
        View::ActivityTimeline => ("Keyboard Shortcuts - Timeline", timeline_shortcuts()),
        View::LogViewer => ("Keyboard Shortcuts - Logs", log_viewer_shortcuts()),
        View::FocusStats => ("Keyboard Shortcuts - Focus Stats", focus_stats_shortcuts()),
        View::ClaudeConfigs => (
            "Keyboard Shortcuts - Claude Configs",
            claude_configs_shortcuts(),
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
        shortcut_line("j / Down", "Move down"),
        shortcut_line("k / Up", "Move up"),
        shortcut_line("Enter", "Select project"),
        shortcut_line("Tab", "Switch focus (projects/sessions)"),
        shortcut_line("1-9", "Jump to item by number"),
        empty_line(),
        section_header("Actions"),
        shortcut_line("n", "Add new project"),
        shortcut_line("d", "Delete selected"),
        shortcut_line("a", "Activity timeline"),
        shortcut_line("c", "Claude configurations"),
        shortcut_line("l", "View logs"),
        shortcut_line("R", "Refresh git state"),
        shortcut_line("Space", "Jump to next session needing attention"),
        empty_line(),
        section_header("Focus Timer"),
        shortcut_line("t", "Start focus timer"),
        shortcut_line("T", "View focus stats"),
        shortcut_line("Ctrl+t", "Stop timer (when running)"),
        empty_line(),
        shortcut_line("q", "Quit"),
        empty_line(),
        footer_hint(),
    ]
}

fn project_detail_shortcuts() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Navigation"),
        shortcut_line("j / Down", "Move down"),
        shortcut_line("k / Up", "Move up"),
        shortcut_line("Enter", "Select branch"),
        shortcut_line("Esc", "Back to projects"),
        shortcut_line("1-9", "Jump to item by number"),
        empty_line(),
        section_header("Actions"),
        shortcut_line("n", "Create new worktree"),
        shortcut_line("d", "Delete selected branch"),
        shortcut_line("b", "Set default base branch"),
        shortcut_line("r", "Rename project"),
        shortcut_line("a", "Activity timeline"),
        shortcut_line("Space", "Jump to next session needing attention"),
        empty_line(),
        section_header("Focus Timer"),
        shortcut_line("t", "Start focus timer"),
        shortcut_line("T", "View focus stats"),
        shortcut_line("Ctrl+t", "Stop timer (when running)"),
        empty_line(),
        shortcut_line("q", "Quit"),
        empty_line(),
        footer_hint(),
    ]
}

fn branch_detail_shortcuts() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Navigation"),
        shortcut_line("j / Down", "Move down"),
        shortcut_line("k / Up", "Move up"),
        shortcut_line("Enter", "Open session"),
        shortcut_line("Esc", "Back to project"),
        shortcut_line("1-9", "Jump to item by number"),
        empty_line(),
        section_header("Actions"),
        shortcut_line("n", "New Claude Code session"),
        shortcut_line("s", "New shell session"),
        shortcut_line("d", "Delete selected session"),
        shortcut_line("a", "Activity timeline"),
        shortcut_line("Space", "Jump to next session needing attention"),
        empty_line(),
        section_header("Focus Timer"),
        shortcut_line("t", "Start focus timer"),
        shortcut_line("T", "View focus stats"),
        shortcut_line("Ctrl+t", "Stop timer (when running)"),
        empty_line(),
        shortcut_line("q", "Quit"),
        empty_line(),
        footer_hint(),
    ]
}

fn session_view_shortcuts() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Mode Switching"),
        shortcut_line("i / Enter", "Enter session mode (type in PTY)"),
        shortcut_line("Esc", "Exit session mode / Go back"),
        empty_line(),
        section_header("Navigation (Normal Mode)"),
        shortcut_line("Tab", "Next session"),
        shortcut_line("Shift+Tab", "Previous session"),
        shortcut_line("Space", "Jump to next session needing attention"),
        empty_line(),
        section_header("Scrollback (Normal Mode)"),
        shortcut_line("Page Up", "Scroll up"),
        shortcut_line("Page Down", "Scroll down"),
        shortcut_line("Home / g", "Jump to top"),
        shortcut_line("End / G", "Jump to bottom (live)"),
        empty_line(),
        section_header("Session Mode"),
        shortcut_line("All keys", "Sent to Claude Code"),
        shortcut_line("Mouse scroll", "Scroll when PTY supports it"),
        empty_line(),
        footer_hint(),
    ]
}

fn timeline_shortcuts() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Navigation"),
        shortcut_line("j / Down", "Move down"),
        shortcut_line("k / Up", "Move up"),
        shortcut_line("Enter", "Open session"),
        shortcut_line("Esc", "Back to projects"),
        shortcut_line("1-9", "Jump to item by number"),
        empty_line(),
        section_header("Actions"),
        shortcut_line("Space", "Jump to next session needing attention"),
        empty_line(),
        section_header("Focus Timer"),
        shortcut_line("t", "Start focus timer"),
        shortcut_line("T", "View focus stats"),
        shortcut_line("Ctrl+t", "Stop timer (when running)"),
        empty_line(),
        shortcut_line("q", "Quit"),
        empty_line(),
        footer_hint(),
    ]
}

fn log_viewer_shortcuts() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Navigation"),
        shortcut_line("j / Down", "Scroll down"),
        shortcut_line("k / Up", "Scroll up"),
        shortcut_line("Page Down", "Page down"),
        shortcut_line("Page Up", "Page up"),
        shortcut_line("Home / g", "Jump to start"),
        shortcut_line("End / G", "Jump to end (live tail)"),
        shortcut_line("Esc", "Back to projects"),
        empty_line(),
        section_header("Options"),
        shortcut_line("f", "Toggle auto-follow"),
        empty_line(),
        section_header("Focus Timer"),
        shortcut_line("t", "Start focus timer"),
        shortcut_line("T", "View focus stats"),
        shortcut_line("Ctrl+t", "Stop timer (when running)"),
        empty_line(),
        shortcut_line("q", "Quit"),
        empty_line(),
        footer_hint(),
    ]
}

fn focus_stats_shortcuts() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Navigation"),
        shortcut_line("j / Down", "Move down"),
        shortcut_line("k / Up", "Move up"),
        shortcut_line("Enter", "View session details"),
        shortcut_line("Esc", "Back to projects"),
        shortcut_line("1-9", "Jump to item by number"),
        empty_line(),
        section_header("Actions"),
        shortcut_line("d", "Delete selected session"),
        empty_line(),
        section_header("Focus Timer"),
        shortcut_line("t", "Start focus timer"),
        shortcut_line("Ctrl+t", "Stop timer (when running)"),
        empty_line(),
        shortcut_line("q", "Quit"),
        empty_line(),
        footer_hint(),
    ]
}

fn claude_configs_shortcuts() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Navigation"),
        shortcut_line("j / Down", "Move down"),
        shortcut_line("k / Up", "Move up"),
        shortcut_line("Esc", "Back to projects"),
        shortcut_line("1-9", "Jump to item by number"),
        empty_line(),
        section_header("Actions"),
        shortcut_line("n", "Add new configuration"),
        shortcut_line("d", "Delete selected"),
        shortcut_line("s", "Set as default"),
        empty_line(),
        section_header("Focus Timer"),
        shortcut_line("t", "Start focus timer"),
        shortcut_line("T", "View focus stats"),
        shortcut_line("Ctrl+t", "Stop timer (when running)"),
        empty_line(),
        shortcut_line("q", "Quit"),
        empty_line(),
        footer_hint(),
    ]
}
