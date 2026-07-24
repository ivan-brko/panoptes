//! Help overlay showing keyboard shortcuts for wherever the user is
//!
//! Structured per pane and per sub-screen, mirroring the navigation model: a
//! global section that applies everywhere, then the keys of the focused pane's
//! current level. Dismissible with `?` or `Esc`.

use ratatui::prelude::*;

use crate::app::{AppState, Focus, ProjectsNav, SettingsNav, Tab};
use crate::tui::theme::theme;
use crate::tui::widgets::dialog::{render_dialog, DialogSize, DialogSpec};

/// Render the help overlay for the current pane and level
pub fn render_help_overlay(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();
    let (title, content) = shortcuts_for(state);

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
                max: 72,
            },
            height: DialogSize::Percent {
                pct: 70,
                min: 10,
                max: 28,
            },
        },
        content,
    );
}

/// The title and shortcut list for wherever the user currently is
fn shortcuts_for(state: &AppState) -> (&'static str, Vec<Line<'static>>) {
    match state.focus {
        Focus::Session => ("Keyboard Shortcuts - Session", session_shortcuts()),
        Focus::Panes(Tab::Projects) => match state.projects_nav {
            ProjectsNav::Overview => ("Keyboard Shortcuts - Projects", projects_shortcuts()),
            ProjectsNav::Project(_) => ("Keyboard Shortcuts - Project", project_shortcuts()),
            ProjectsNav::Branch(_, _) => ("Keyboard Shortcuts - Branch", branch_shortcuts()),
            ProjectsNav::ProjectSettings(_) => (
                "Keyboard Shortcuts - Project settings",
                project_settings_shortcuts(),
            ),
        },
        Focus::Panes(Tab::Sessions) => ("Keyboard Shortcuts - Sessions", sessions_shortcuts()),
        Focus::Panes(Tab::Settings) => match state.settings_nav {
            SettingsNav::Sections => ("Keyboard Shortcuts - Settings", settings_shortcuts()),
            SettingsNav::ClaudeConfigs | SettingsNav::CodexConfigs => {
                ("Keyboard Shortcuts - Configs", settings_configs_shortcuts())
            }
            SettingsNav::Shortcuts => (
                "Keyboard Shortcuts - Custom shortcuts",
                settings_shortcuts_section(),
            ),
            SettingsNav::Notifications => (
                "Keyboard Shortcuts - Notifications",
                settings_notifications_shortcuts(),
            ),
            SettingsNav::About => ("Keyboard Shortcuts - About", settings_about_shortcuts()),
        },
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
        Style::default().fg(t.text_dim),
    )])
}

/// The keys that mean the same thing from every pane
fn global_section() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Global"),
        shortcut_line("Tab / ⇧Tab", "Switch pane (wraps)"),
        shortcut_line("Esc", "Back one level, then out to Projects"),
        shortcut_line("q", "Quit (asks to confirm)"),
        shortcut_line("Space", "Jump to next session needing attention"),
        shortcut_line("?", "Toggle this help"),
        empty_line(),
    ]
}

/// Build a pane's help: the global section, then this level's own keys
fn with_global(header: &'static str, lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    let mut all = global_section();
    all.push(section_header(header));
    all.extend(lines);
    all.push(empty_line());
    all.push(footer_hint());
    all
}

fn projects_shortcuts() -> Vec<Line<'static>> {
    with_global(
        "Pane 1 - Projects",
        vec![
            shortcut_line("↑ / ↓", "Move through the tree"),
            shortcut_line("Enter", "Open project, or expand/collapse folder"),
            shortcut_line("→ / ←", "Expand folder / collapse or go to parent"),
            shortcut_line("n", "Add a project"),
            shortcut_line("d", "Delete project, or ungroup folder"),
            shortcut_line("m", "Move project or folder into a folder"),
            shortcut_line("r", "Rename the selected folder"),
            shortcut_line("R", "Refresh git state"),
        ],
    )
}

fn project_shortcuts() -> Vec<Line<'static>> {
    with_global(
        "Pane 1 - Project",
        vec![
            shortcut_line("↑ / ↓ / 1-9", "Select a branch"),
            shortcut_line("Enter", "Open the branch"),
            shortcut_line("n", "Create a worktree"),
            shortcut_line("d", "Delete the selected branch"),
            shortcut_line("R", "Refresh branches"),
            shortcut_line(",", "Project settings"),
        ],
    )
}

fn branch_shortcuts() -> Vec<Line<'static>> {
    with_global(
        "Pane 1 - Branch",
        vec![
            shortcut_line("↑ / ↓ / 1-9", "Select a session (0 = 10)"),
            shortcut_line("Enter", "Open session (resumes if [Resumable])"),
            shortcut_line("n", "New AI session (Claude/Codex)"),
            shortcut_line("s", "New shell session"),
            shortcut_line("d", "Delete session (or discard a resumable one)"),
            shortcut_line("<key>", "Run a custom shortcut"),
        ],
    )
}

fn project_settings_shortcuts() -> Vec<Line<'static>> {
    with_global(
        "Pane 1 - Project settings",
        vec![
            shortcut_line("↑ / ↓", "Move through the rows"),
            shortcut_line("Enter", "Open the selected setting"),
            shortcut_line("Esc", "Back to the branch list"),
        ],
    )
}

fn sessions_shortcuts() -> Vec<Line<'static>> {
    with_global(
        "Pane 2 - Sessions",
        vec![
            shortcut_line("↑ / ↓ / 1-9", "Select a session (0 = 10)"),
            shortcut_line("Enter", "Open the session full-screen"),
            shortcut_line("d", "Delete the selected session"),
            shortcut_line("Esc", "Back to the Projects pane"),
        ],
    )
}

fn settings_shortcuts() -> Vec<Line<'static>> {
    with_global(
        "Pane 3 - Settings",
        vec![
            shortcut_line("↑ / ↓", "Move through the sections"),
            shortcut_line("Enter", "Open the selected section"),
            shortcut_line("Esc", "Back to the Projects pane"),
        ],
    )
}

fn settings_configs_shortcuts() -> Vec<Line<'static>> {
    with_global(
        "Settings - Configs",
        vec![
            shortcut_line("↑ / ↓", "Select a config"),
            shortcut_line("n", "Add a config"),
            shortcut_line("d", "Delete the selected config"),
            shortcut_line("s", "Set as default"),
            shortcut_line("Esc", "Back to the sections list"),
        ],
    )
}

fn settings_shortcuts_section() -> Vec<Line<'static>> {
    with_global(
        "Settings - Custom shortcuts",
        vec![
            shortcut_line("↑ / ↓", "Select a shortcut"),
            shortcut_line("n", "Bind a key to a shell command"),
            shortcut_line("d", "Delete the selected shortcut"),
            shortcut_line("Esc", "Back to the sections list"),
        ],
    )
}

fn settings_notifications_shortcuts() -> Vec<Line<'static>> {
    with_global(
        "Settings - Notifications",
        vec![
            shortcut_line("↑ / ↓", "Move through the rows"),
            shortcut_line("Space", "Toggle the selected option"),
            shortcut_line("← / →", "Change how you are notified"),
            shortcut_line("Esc", "Back to the sections list"),
        ],
    )
}

fn settings_about_shortcuts() -> Vec<Line<'static>> {
    with_global(
        "Settings - About / paths",
        vec![
            shortcut_line("Esc", "Back to the sections list"),
            shortcut_line("", "Everything here is read-only; edit config.toml"),
        ],
    )
}

fn session_shortcuts() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Session - normal mode"),
        shortcut_line("Enter", "Enter session mode (type in the PTY)"),
        shortcut_line("Esc", "Back to the pane it was opened from"),
        shortcut_line("q", "Quit (asks to confirm)"),
        shortcut_line("1-9", "Switch to session by number (0 = 10)"),
        shortcut_line("Space", "Jump to next session needing attention"),
        shortcut_line("↑ / ↓", "Scroll (3 lines)"),
        shortcut_line("PgUp/PgDn", "Scroll a page"),
        shortcut_line("Home / End", "Jump to top / live view"),
        shortcut_line("<key>", "Run a custom shortcut"),
        empty_line(),
        section_header("Session - attached"),
        shortcut_line("All keys", "Forwarded to the agent"),
        shortcut_line("Esc", "Detach back to normal mode"),
        shortcut_line("\u{21E7}Esc", "Send Esc to the agent"),
        shortcut_line("Ctrl+Home/End", "Scroll without detaching"),
        shortcut_line("Mouse scroll", "Scroll when the PTY supports it"),
        empty_line(),
        footer_hint(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::views::test_util::{contains_line, render_to_lines};
    use uuid::Uuid;

    fn render(state: &AppState) -> Vec<String> {
        render_to_lines(90, 34, |frame| {
            render_help_overlay(frame, frame.size(), state)
        })
    }

    #[test]
    fn test_every_pane_and_level_has_its_own_help() {
        let project_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();

        let mut cases: Vec<(AppState, &str)> = vec![
            (AppState::default(), "Keyboard Shortcuts - Projects"),
            (
                AppState {
                    focus: Focus::Session,
                    ..Default::default()
                },
                "Keyboard Shortcuts - Session",
            ),
            (
                AppState {
                    focus: Focus::Panes(Tab::Sessions),
                    ..Default::default()
                },
                "Keyboard Shortcuts - Sessions",
            ),
        ];
        for (nav, title) in [
            (
                ProjectsNav::Project(project_id),
                "Keyboard Shortcuts - Project",
            ),
            (
                ProjectsNav::Branch(project_id, branch_id),
                "Keyboard Shortcuts - Branch",
            ),
            (
                ProjectsNav::ProjectSettings(project_id),
                "Keyboard Shortcuts - Project settings",
            ),
        ] {
            cases.push((
                AppState {
                    projects_nav: nav,
                    ..Default::default()
                },
                title,
            ));
        }
        for (nav, title) in [
            (SettingsNav::Sections, "Keyboard Shortcuts - Settings"),
            (SettingsNav::ClaudeConfigs, "Keyboard Shortcuts - Configs"),
            (
                SettingsNav::Shortcuts,
                "Keyboard Shortcuts - Custom shortcuts",
            ),
            (
                SettingsNav::Notifications,
                "Keyboard Shortcuts - Notifications",
            ),
            (SettingsNav::About, "Keyboard Shortcuts - About"),
        ] {
            cases.push((
                AppState {
                    focus: Focus::Panes(Tab::Settings),
                    settings_nav: nav,
                    ..Default::default()
                },
                title,
            ));
        }

        for (state, title) in cases {
            let lines = render(&state);
            assert!(contains_line(&lines, title), "{title}: {lines:?}");
        }
    }

    #[test]
    fn test_pane_help_leads_with_the_global_keys() {
        for tab in Tab::ALL {
            let state = AppState {
                focus: Focus::Panes(tab),
                ..Default::default()
            };
            let lines = render(&state);
            assert!(contains_line(&lines, "Switch pane"), "{tab:?}: {lines:?}");
            assert!(contains_line(&lines, "q"), "{tab:?}: {lines:?}");
        }
    }

    /// The keys that were retired must not be advertised anywhere
    #[test]
    fn test_help_never_mentions_the_retired_keys() {
        let states = [
            AppState::default(),
            AppState {
                focus: Focus::Session,
                ..Default::default()
            },
            AppState {
                focus: Focus::Panes(Tab::Settings),
                ..Default::default()
            },
        ];
        for state in states {
            let lines = render(&state);
            for gone in ["View logs", "Custom shortcuts overlay", "Next session"] {
                assert!(
                    !contains_line(&lines, gone),
                    "help still offers {gone:?}: {lines:?}"
                );
            }
        }
    }
}
