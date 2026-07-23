//! The three-pane frame: one header, three bordered panes, one footer
//!
//! Panes are laid out from the animated widths handed in by the caller, so a
//! transition never has to be replayed here: whatever widths arrive are the
//! widths drawn, and each pane picks its own density from the width it got.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders};

use crate::app::{AppState, InputMode, ProjectsNav, SettingsNav, Tab};
use crate::claude_config::ClaudeConfigStore;
use crate::codex_config::CodexConfigStore;
use crate::config::Config;
use crate::logging::LogFileInfo;
use crate::project::ProjectStore;
use crate::session::SessionManager;
use crate::tui::header::Header;
use crate::tui::layout::ScreenLayout;
use crate::tui::panes::{side_mode, SideMode};
use crate::tui::theme::theme;
use crate::tui::views::pane_settings::SettingsPaneContext;
use crate::tui::views::{
    footer_with_attention, format_custom_shortcuts_hint, render_footer, status_parts,
    truncate_string, Breadcrumb,
};

/// Everything the three panes need to draw themselves
pub struct PaneContext<'a> {
    pub state: &'a AppState,
    pub project_store: &'a ProjectStore,
    pub sessions: &'a SessionManager,
    pub config: &'a Config,
    pub claude_config_store: &'a ClaudeConfigStore,
    pub codex_config_store: &'a CodexConfigStore,
    pub log_file_info: &'a LogFileInfo,
    /// Current pane widths, summing exactly to the terminal width
    pub widths: [u16; 3],
    pub hook_port: u16,
    pub hook_healthy: bool,
}

/// Render the whole three-pane screen
pub fn render_panes(frame: &mut Frame, area: Rect, ctx: PaneContext) {
    let state = ctx.state;
    let sessions = ctx.sessions;
    let attention_count = sessions.total_attention_count();

    // One global header, visible from every pane - which is what keeps the
    // blinking attention indicator reachable from deep inside Settings
    let mut parts = vec![format!("{} projects", ctx.project_store.project_count())];
    parts.extend(status_parts(sessions.total_active_count(), attention_count));
    let header = Header::new(Breadcrumb::new())
        .with_suffix(format!("({})", parts.join(", ")))
        .with_notifications(Some(&state.header_notifications))
        .with_attention_count(attention_count);

    let areas = ScreenLayout::new(area).with_header(header).render(frame);

    if state.dropped_events_count > 0 {
        render_dropped_events_banner(frame, areas.content, state);
    }
    let content = if state.dropped_events_count > 0 {
        Rect {
            y: areas.content.y + 1,
            height: areas.content.height.saturating_sub(1),
            ..areas.content
        }
    } else {
        areas.content
    };

    render_pane_row(frame, content, &ctx);
    render_footer(frame, areas.footer(), &footer_text(&ctx));
}

/// The dropped-hook-events warning, above the panes
fn render_dropped_events_banner(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();
    let text = truncate_string(
        &format!(
            "⚠ {} hook events dropped - session states may be inaccurate",
            state.dropped_events_count
        ),
        area.width as usize,
    );
    let banner = Rect { height: 1, ..area };
    frame.render_widget(
        ratatui::widgets::Paragraph::new(text).style(t.warning_banner_style()),
        banner,
    );
}

/// Lay the three panes out side by side and draw each one
///
/// The widths handed in already sum to the terminal width. This re-derives
/// them if they somehow do not - a terminal that resized without emitting an
/// event, say - because a mismatch here is the one that shows: a gap down the
/// right-hand side, or a pane drawn past the edge.
fn render_pane_row(frame: &mut Frame, area: Rect, ctx: &PaneContext) {
    let widths = if ctx.widths.iter().sum::<u16>() == area.width {
        ctx.widths
    } else {
        let focused = ctx.state.focus.tab().map(Tab::index).unwrap_or(0);
        crate::tui::panes::pane_widths(area.width, focused)
    };

    let mut x = area.x;
    for (index, width) in widths.iter().copied().enumerate() {
        let pane_area = Rect {
            x,
            y: area.y,
            width,
            height: area.height,
        };
        x = x.saturating_add(width);
        if width == 0 {
            continue;
        }
        render_pane(frame, pane_area, Tab::ALL[index], ctx);
    }
}

/// One bordered pane, titled with its own breadcrumb
fn render_pane(frame: &mut Frame, area: Rect, tab: Tab, ctx: &PaneContext) {
    let t = theme();
    let focused = ctx.state.is_focused(tab);
    // Density comes from the width this pane has *right now*, never from the
    // width it is heading for - which is what lets a pane cross
    // strip -> compact part-way through a transition
    // One density for the whole pane, from the width it has *right now*.
    // Measured on the outer rect, which is what `SIDE_COMPACT_MIN` and the
    // sizing table are expressed in - the body is handed the same answer so a
    // pane can never wear a full title over a strip.
    let mode = side_mode(area.width);

    // A title has this pane's width minus its two border corners
    let title_width = area.width.saturating_sub(2) as usize;
    let title = match tab {
        Tab::Projects => match mode {
            SideMode::Strip | SideMode::Hidden => String::new(),
            _ => {
                super::pane_projects::projects_breadcrumb(ctx.state, ctx.project_store, title_width)
            }
        },
        Tab::Sessions => super::pane_sessions::sessions_title(ctx.sessions, mode),
        Tab::Settings => super::pane_settings::settings_title(ctx.state, mode),
    };

    // Truncated against this pane's own width, so a long title cannot spill
    // across the border into its neighbour
    let title = truncate_string(&title, title_width);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(if focused { t.border_focused } else { t.border }));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    match tab {
        Tab::Projects => super::pane_projects::render_projects_pane(
            frame,
            inner,
            ctx.state,
            ctx.project_store,
            ctx.sessions,
            mode,
        ),
        Tab::Sessions => super::pane_sessions::render_sessions_pane(
            frame,
            inner,
            ctx.state,
            ctx.project_store,
            ctx.sessions,
            mode,
        ),
        Tab::Settings => super::pane_settings::render_settings_pane(
            frame,
            inner,
            mode,
            &SettingsPaneContext {
                state: ctx.state,
                config: ctx.config,
                claude_config_store: ctx.claude_config_store,
                codex_config_store: ctx.codex_config_store,
                log_file_info: ctx.log_file_info,
                hook_port: ctx.hook_port,
                hook_healthy: ctx.hook_healthy,
            },
        ),
    }
}

/// The global footer: the focused pane's keys, plus the attention hint
fn footer_text(ctx: &PaneContext) -> String {
    let state = ctx.state;

    // A prompt owns the footer while it is open
    if let Some(prompt) = prompt_footer(state.input_mode) {
        return prompt.to_string();
    }

    let base = match state.focus.tab() {
        Some(Tab::Projects) => projects_footer(state, ctx.project_store, ctx.config),
        Some(Tab::Sessions) => "↑↓/1-9: select | Enter: open | d: delete".to_string(),
        Some(Tab::Settings) => settings_footer(state, ctx.config),
        None => String::new(),
    };

    let global = "Tab: pane | q: quit | ?: help";
    footer_with_attention(format!("{} | {}", base, global), ctx.sessions)
}

/// The footer a prompt owns while it is open, if any
fn prompt_footer(mode: InputMode) -> Option<&'static str> {
    Some(match mode {
        InputMode::AddingProject => "Tab: autocomplete | Enter: select | Esc: cancel",
        InputMode::AddingProjectName => "Enter: create project | Esc: back",
        InputMode::RenamingProject | InputMode::RenamingFolder => "Enter: save | Esc: cancel",
        InputMode::MovingToFolder => "Tab: complete | Enter: move | Esc: cancel",
        InputMode::CreatingSession
        | InputMode::CreatingCodexSession
        | InputMode::CreatingShellSession => "Enter: create | Esc: cancel",
        InputMode::SelectingAgentType => "↑↓: navigate | Enter: select | Esc: cancel",
        InputMode::ConfirmingBranchDelete => {
            "w: also delete the directory | y: confirm | n/Esc: cancel"
        }
        InputMode::ConfirmingSessionDelete
        | InputMode::ConfirmingProjectDelete
        | InputMode::ConfirmingFolderRemove => "y: confirm | n/Esc: cancel",
        InputMode::ConfirmingQuit => "y/Enter: quit | n/Esc: cancel",
        InputMode::WorktreeSelectBranch => {
            "Type to search/create | ↑↓: navigate | Enter: select | Esc: cancel"
        }
        InputMode::WorktreeSelectBase => "Type: filter | ↑↓: navigate | Enter: confirm | Esc: back",
        InputMode::WorktreeConfirm => "Enter: create | Esc: back",
        InputMode::SelectingDefaultBase => {
            "Type: filter | ↑↓: navigate | Enter: set default | Esc: cancel"
        }
        InputMode::SelectingClaudeConfig | InputMode::SelectingCodexConfig => {
            "↑↓: navigate | Enter: select | Esc: cancel"
        }
        _ => return None,
    })
}

/// Pane 1's keys, which depend on the level it is drilled into
///
/// The overview's keys are context-sensitive: a folder heading offers folder
/// actions, so the expand/collapse binding is advertised exactly when it
/// applies and "ungroup" signals that nothing gets deleted.
fn projects_footer(state: &AppState, project_store: &ProjectStore, config: &Config) -> String {
    match state.projects_nav {
        ProjectsNav::Overview => {
            let on_folder = matches!(
                crate::project::row_at(project_store, state.selected_project_index),
                Some(crate::project::RowRef::Folder { .. })
            );
            if on_folder {
                "Enter/←→: expand/collapse | m: move | r: rename | d: ungroup".to_string()
            } else {
                "↑↓/Enter: open | n: new | d: delete | m: move | R: refresh".to_string()
            }
        }
        ProjectsNav::Project(_) => {
            "↑↓/1-9/Enter | n: new worktree | d: delete | R: refresh | ,: settings".to_string()
        }
        ProjectsNav::Branch(_, _) => {
            let shortcuts = format_custom_shortcuts_hint(&config.custom_shortcuts);
            format!(
                "↑↓/1-9/Enter | n: new AI | s: shell | d: delete | {}Esc: back",
                shortcuts
            )
        }
        ProjectsNav::ProjectSettings(_) => "↑↓/Enter | Esc: back".to_string(),
    }
}

/// Pane 3's keys, plus the description of whatever row is highlighted
fn settings_footer(state: &AppState, config: &Config) -> String {
    let keys = match state.settings_nav {
        SettingsNav::Sections => "↑↓/Enter",
        SettingsNav::ClaudeConfigs | SettingsNav::CodexConfigs => {
            "↑↓ | n: add | d: delete | s: set default | Esc: back"
        }
        SettingsNav::Shortcuts => "↑↓ | n: add | d: delete | Esc: back",
        SettingsNav::Notifications => "↑↓ | Space/←→: change | Esc: back",
        SettingsNav::About => "Esc: back",
    };
    let description = super::pane_settings::settings_description(state, config);
    if description.is_empty() {
        keys.to_string()
    } else {
        format!("{} — {}", keys, description)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Focus;
    use crate::session::store::SessionStore;
    use crate::tui::panes::pane_widths;
    use crate::tui::views::test_util::{contains_line, render_to_lines};
    use std::path::PathBuf;

    struct Fixture {
        project_store: ProjectStore,
        sessions: SessionManager,
        config: Config,
        claude: ClaudeConfigStore,
        codex: CodexConfigStore,
        log: LogFileInfo,
    }

    fn fixture() -> Fixture {
        let mut project_store = ProjectStore::new();
        project_store.add_project(crate::project::Project::new(
            "panoptes".to_string(),
            PathBuf::from("/tmp/panoptes"),
            "main".to_string(),
        ));
        Fixture {
            project_store,
            sessions: SessionManager::with_store(Config::default(), SessionStore::new()),
            config: Config::default(),
            claude: ClaudeConfigStore::new(),
            codex: CodexConfigStore::new(),
            log: LogFileInfo {
                path: PathBuf::from("/tmp/panoptes.log"),
            },
        }
    }

    fn render(width: u16, state: &AppState, f: &Fixture) -> Vec<String> {
        let focused = state.focus.tab().map(|t| t.index()).unwrap_or(0);
        let widths = pane_widths(width, focused);
        render_to_lines(width, 24, |frame| {
            render_panes(
                frame,
                frame.size(),
                PaneContext {
                    state,
                    project_store: &f.project_store,
                    sessions: &f.sessions,
                    config: &f.config,
                    claude_config_store: &f.claude,
                    codex_config_store: &f.codex,
                    log_file_info: &f.log,
                    widths,
                    hook_port: 9999,
                    hook_healthy: true,
                },
            )
        })
    }

    #[test]
    fn test_three_panes_render_at_once() {
        let f = fixture();
        let lines = render(160, &AppState::default(), &f);

        assert!(contains_line(&lines, "Projects (1)"), "{lines:?}");
        assert!(contains_line(&lines, "Sessions (0)"), "{lines:?}");
        assert!(contains_line(&lines, "Settings"), "{lines:?}");
    }

    #[test]
    fn test_one_global_header_and_footer() {
        let f = fixture();
        let lines = render(160, &AppState::default(), &f);

        // The header names the app once, not once per pane
        let header_rows = lines.iter().filter(|l| l.contains("Panoptes")).count();
        assert_eq!(header_rows, 1, "{lines:?}");
        assert!(
            contains_line(&lines, "Tab: pane | q: quit | ?: help"),
            "{lines:?}"
        );
    }

    #[test]
    fn test_footer_follows_the_focused_pane() {
        let f = fixture();

        let lines = render(160, &AppState::default(), &f);
        assert!(contains_line(&lines, "Enter: open | n: new"), "{lines:?}");

        let state = AppState {
            focus: Focus::Panes(Tab::Settings),
            ..Default::default()
        };
        let lines = render(160, &state, &f);
        assert!(
            contains_line(&lines, SettingsNav::ClaudeConfigs.description()),
            "{lines:?}"
        );
    }

    #[test]
    fn test_narrow_terminal_shows_only_the_focused_pane() {
        let f = fixture();
        let lines = render(50, &AppState::default(), &f);

        assert!(contains_line(&lines, "Projects"), "{lines:?}");
        assert!(!contains_line(&lines, "Sessions ("), "{lines:?}");
    }

    #[test]
    fn test_nothing_renders_past_the_terminal_width() {
        let f = fixture();
        for width in [60_u16, 80, 88, 100, 140, 200] {
            for tab in Tab::ALL {
                let state = AppState {
                    focus: Focus::Panes(tab),
                    ..Default::default()
                };
                let lines = render(width, &state, &f);
                for line in &lines {
                    assert!(
                        line.chars().count() <= width as usize,
                        "row {line:?} overflows a {width}-column terminal"
                    );
                }
            }
        }
    }

    /// A pane's title and its body must agree about density. They are two
    /// columns apart - the border - so recomputing the mode from the inner
    /// rect put full titles over strip bodies at the 88-column threshold.
    #[test]
    fn test_pane_title_and_body_agree_about_density() {
        let f = fixture();

        // 88 is the width at which three readable panes first fit: side panes
        // are 22 columns, which is exactly `SIDE_COMPACT_MIN`
        let lines = render(88, &AppState::default(), &f);
        assert!(contains_line(&lines, "Sessions (0)"), "{lines:?}");
        assert!(
            !contains_line(&lines, "S 0"),
            "a titled pane must not hold a strip body: {lines:?}"
        );

        // 87 is the other side of the discontinuity: strips, and no titles
        let lines = render(87, &AppState::default(), &f);
        assert!(contains_line(&lines, "S 0"), "{lines:?}");
        assert!(!contains_line(&lines, "Sessions (0)"), "{lines:?}");
    }

    /// Overlays are anchored to the terminal, and `Clear` does not clip, so a
    /// terminal too small for a dialog's minimum size used to panic
    #[test]
    fn test_a_tiny_terminal_never_panics() {
        let f = fixture();
        for width in [1_u16, 8, 20, 36, 59, 60, 88] {
            for height in [1_u16, 3, 6, 8, 24] {
                for tab in Tab::ALL {
                    let state = AppState {
                        focus: Focus::Panes(tab),
                        ..Default::default()
                    };
                    let widths = pane_widths(width, tab.index());
                    render_to_lines(width, height, |frame| {
                        render_panes(
                            frame,
                            frame.size(),
                            PaneContext {
                                state: &state,
                                project_store: &f.project_store,
                                sessions: &f.sessions,
                                config: &f.config,
                                claude_config_store: &f.claude,
                                codex_config_store: &f.codex,
                                log_file_info: &f.log,
                                widths,
                                hook_port: 9999,
                                hook_healthy: true,
                            },
                        )
                    });
                }
            }
        }
    }

    #[test]
    fn test_quit_is_offered_from_every_pane() {
        let f = fixture();
        for tab in Tab::ALL {
            let state = AppState {
                focus: Focus::Panes(tab),
                ..Default::default()
            };
            let lines = render(160, &state, &f);
            assert!(
                contains_line(&lines, "q: quit"),
                "pane {tab:?} must offer q: {lines:?}"
            );
        }
    }
}
