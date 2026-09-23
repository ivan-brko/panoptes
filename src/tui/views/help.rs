//! Help overlay showing keyboard shortcuts for wherever the user is
//!
//! Structured per pane and per sub-screen, mirroring the navigation model: a
//! global section that applies everywhere, then the keys of the focused pane's
//! current level. Dismissible with `?` or `Esc`.
//!
//! The session section runs past 25 lines, which is taller than the overlay
//! gets on an 80x24 terminal, so it scrolls with `↑↓`/`PgUp`/`PgDn` and names
//! on its bottom border how much is still below.

use ratatui::prelude::*;
use ratatui::widgets::block::{Position, Title};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{AppState, Focus, ProjectsNav, SettingsNav, Tab};
use crate::tui::theme::theme;
use crate::tui::widgets::dialog::{centered_rect, DialogSize};

/// Size of the help overlay
const HELP_WIDTH: DialogSize = DialogSize::Percent {
    pct: 70,
    min: 40,
    max: 72,
};
const HELP_HEIGHT: DialogSize = DialogSize::Percent {
    pct: 70,
    min: 10,
    max: 28,
};

/// How far the help overlay can scroll, and one screenful of it
///
/// Both come from the terminal the overlay would be drawn into, rather than
/// from anything a previous render recorded: the overlay is a fixed fraction of
/// that terminal, and the content is a pure function of where the user is. That
/// lets the input handler clamp `↓` at the real end of the list on the first
/// press, instead of letting a held key run the offset off into nothing.
pub fn help_scroll_limits(state: &AppState, area: Rect) -> (u16, u16) {
    let (_, content) = shortcuts_for(state);
    // The overlay's borders take a row at each end
    let page = centered_rect(area, HELP_WIDTH, HELP_HEIGHT)
        .height
        .saturating_sub(2);
    let max_scroll = (content.len() as u16).saturating_sub(page);
    (max_scroll, page.max(1))
}

/// Render the help overlay for the current pane and level
pub fn render_help_overlay(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();
    let (title, content) = shortcuts_for(state);
    let (max_scroll, _) = help_scroll_limits(state, area);
    let scroll = state.help_scroll.min(max_scroll);

    let overlay = centered_rect(area, HELP_WIDTH, HELP_HEIGHT);
    frame.render_widget(Clear, overlay);

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent))
        .title(format!(" {} ", title));
    // Nothing clipped, nothing to say: the border stays quiet when the whole
    // list fits
    if max_scroll > 0 {
        block = block.title(
            Title::from(Span::styled(
                scroll_hint(scroll, max_scroll),
                t.muted_style(),
            ))
            .position(Position::Bottom)
            .alignment(Alignment::Right),
        );
    }

    frame.render_widget(
        Paragraph::new(content)
            .alignment(Alignment::Left)
            .scroll((scroll, 0))
            .block(block),
        overlay,
    );
}

/// What the bottom border says about the rows that are off screen
fn scroll_hint(scroll: u16, max_scroll: u16) -> String {
    match (scroll, max_scroll - scroll) {
        (0, below) => format!(" {} more ↓ ", below),
        (_, 0) => " ↑ end ".to_string(),
        (_, below) => format!(" ↑ {} more ↓ ", below),
    }
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
            SettingsNav::Theme => ("Keyboard Shortcuts - Theme", settings_theme_shortcuts()),
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
        "↑↓ / PgUp / PgDn to scroll · ? or Esc to close",
        Style::default().fg(t.text_dim),
    )])
}

/// The keys that mean the same thing from every pane
fn global_section() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Global"),
        shortcut_line("→ / Tab", "Next pane (wraps)"),
        shortcut_line("← / ⇧Tab", "Previous pane (wraps)"),
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
            shortcut_line("↑ / ↓", "Select a branch"),
            shortcut_line("Enter", "Open the branch"),
            shortcut_line("⚙ row", "Last row of the list: per-project settings"),
            shortcut_line("n", "Create a worktree"),
            shortcut_line("d", "Delete the selected branch"),
            shortcut_line("R", "Refresh branches"),
        ],
    )
}

fn branch_shortcuts() -> Vec<Line<'static>> {
    with_global(
        "Pane 1 - Branch",
        vec![
            shortcut_line("↑ / ↓", "Select a session"),
            shortcut_line("Enter", "Open session (resumes if [Resumable])"),
            shortcut_line("n", "New AI session (Claude/Codex)"),
            shortcut_line("s", "New shell session"),
            shortcut_line("i", "Import a conversation started outside Panoptes"),
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
            shortcut_line(
                "Space / Enter",
                "Toggle the row, or change how you are notified",
            ),
            shortcut_line("Esc", "Back to the sections list"),
        ],
    )
}

fn settings_theme_shortcuts() -> Vec<Line<'static>> {
    with_global(
        "Settings - Theme",
        vec![
            shortcut_line("↑ / ↓", "Try a preset - the whole UI is the preview"),
            shortcut_line("Enter", "Keep the highlighted preset"),
            shortcut_line("Esc", "Back to the sections list, undoing the preview"),
            shortcut_line("", "Leaving this section any other way undoes it too"),
            shortcut_line("★", "Marks the preset that is saved"),
        ],
    )
}

fn settings_about_shortcuts() -> Vec<Line<'static>> {
    with_global(
        "Settings - About / paths",
        vec![
            shortcut_line("↑ / ↓", "Move through the rows, scrolling the list"),
            shortcut_line("Esc", "Back to the sections list"),
            shortcut_line("", "Everything here is read-only; edit config.toml"),
        ],
    )
}

fn session_shortcuts() -> Vec<Line<'static>> {
    vec![
        empty_line(),
        section_header("Session"),
        shortcut_line("All keys", "Forwarded to the agent"),
        shortcut_line("Esc", "Back to the pane it was opened from"),
        shortcut_line("\u{21E7}Esc", "Send Esc to the agent"),
        shortcut_line("Ctrl+Home", "Jump to the oldest line kept"),
        shortcut_line("Ctrl+End", "Back to live output"),
        shortcut_line("", "Both go to the agent instead when it draws"),
        shortcut_line("", "its own screen (claude, vim, less)"),
        empty_line(),
        section_header("Session - mouse"),
        shortcut_line("Scroll wheel", "Scroll back through the output; goes to"),
        shortcut_line("", "the agent when the agent owns the mouse"),
        shortcut_line("Drag", "Select and copy"),
        shortcut_line("\u{21E7}Drag", "The same, over an agent that owns the"),
        shortcut_line("", "mouse (claude, vim, htop) - shift reaches"),
        shortcut_line("", "past it, as in any terminal"),
        shortcut_line("Ctrl+Drag", "Select a rectangle: one column of a table"),
        shortcut_line("", "without the rest of each line"),
        shortcut_line("Double/triple click", "Select a word / a whole line"),
        empty_line(),
        // Said plainly, because the keys these replace were in this list
        // until the session view had one mode. Someone reaching for PgUp
        // should find out here why it typed into their agent instead.
        section_header("Not in a session"),
        shortcut_line("", "Switching sessions and custom shortcuts live"),
        shortcut_line("", "in the panes. Every other key here belongs"),
        shortcut_line("", "to the agent. Esc, then go."),
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

    /// The branch level's help names the import key, alongside the footer
    /// and `RESERVED_KEYS` (the three places a new key must go)
    #[test]
    fn test_branch_help_lists_the_import_key() {
        let state = AppState {
            projects_nav: ProjectsNav::Branch(Uuid::new_v4(), Uuid::new_v4()),
            ..Default::default()
        };
        let lines = render(&state);
        assert!(
            contains_line(&lines, "Import a conversation started outside Panoptes"),
            "{lines:?}"
        );
    }

    #[test]
    fn test_pane_help_leads_with_the_global_keys() {
        for tab in Tab::ALL {
            let state = AppState {
                focus: Focus::Panes(tab),
                ..Default::default()
            };
            let lines = render(&state);
            assert!(contains_line(&lines, "Next pane"), "{tab:?}: {lines:?}");
            assert!(contains_line(&lines, "Previous pane"), "{tab:?}: {lines:?}");
            assert!(contains_line(&lines, "q"), "{tab:?}: {lines:?}");
        }
    }

    /// The session section runs past what an 80x24 terminal gives the overlay,
    /// so the bottom of it is only reachable by scrolling
    #[test]
    fn test_a_clipped_help_list_scrolls_to_its_last_line() {
        let state = AppState {
            focus: Focus::Session,
            ..Default::default()
        };
        let area = Rect::new(0, 0, 80, 24);
        let (max_scroll, page) = help_scroll_limits(&state, area);
        assert!(max_scroll > 0, "the session help fits, so nothing to test");
        assert!(page > 1);

        let unscrolled = render_to_lines(80, 24, |frame| {
            render_help_overlay(frame, frame.size(), &state)
        });
        assert!(
            !contains_line(&unscrolled, "Esc, then go."),
            "the tail is visible unscrolled: {unscrolled:?}"
        );

        let scrolled_state = AppState {
            help_scroll: max_scroll,
            ..state
        };
        let scrolled = render_to_lines(80, 24, |frame| {
            render_help_overlay(frame, frame.size(), &scrolled_state)
        });
        assert!(contains_line(&scrolled, "Esc, then go."), "{scrolled:?}");
        assert!(
            contains_line(&scrolled, "? or Esc to close"),
            "{scrolled:?}"
        );
    }

    /// A pane whose help fits reports nothing to scroll, so `↓` there stays a
    /// no-op rather than sliding the list under its own border
    #[test]
    fn test_a_help_list_that_fits_has_nothing_to_scroll() {
        let state = AppState {
            focus: Focus::Panes(Tab::Projects),
            ..Default::default()
        };
        let (max_scroll, _) = help_scroll_limits(&state, Rect::new(0, 0, 100, 60));
        assert_eq!(max_scroll, 0);
    }

    /// A stale offset - from a resize, or from a list that is now shorter -
    /// must not scroll the content away entirely
    #[test]
    fn test_an_offset_past_the_end_still_shows_the_tail() {
        let state = AppState {
            focus: Focus::Panes(Tab::Projects),
            help_scroll: 500,
            ..Default::default()
        };
        let lines = render(&state);
        assert!(contains_line(&lines, "? or Esc to close"), "{lines:?}");
    }

    /// The border says how much is below, and stays quiet when nothing is
    #[test]
    fn test_the_border_names_what_is_off_screen() {
        assert_eq!(scroll_hint(0, 7), " 7 more ↓ ");
        assert_eq!(scroll_hint(3, 7), " ↑ 4 more ↓ ");
        assert_eq!(scroll_hint(7, 7), " ↑ end ");

        let state = AppState {
            focus: Focus::Session,
            ..Default::default()
        };
        let clipped = render_to_lines(80, 24, |frame| {
            render_help_overlay(frame, frame.size(), &state)
        });
        assert!(contains_line(&clipped, "more ↓"), "{clipped:?}");

        // A list that fits its overlay draws no indicator at all
        let short = AppState {
            focus: Focus::Panes(Tab::Projects),
            ..Default::default()
        };
        let roomy = render_to_lines(100, 60, |frame| {
            render_help_overlay(frame, frame.size(), &short)
        });
        assert!(!contains_line(&roomy, "more ↓"), "{roomy:?}");
        assert!(contains_line(&roomy, "Refresh git state"), "{roomy:?}");
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
