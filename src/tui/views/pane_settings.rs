//! Pane 3: settings
//!
//! Not a config editor. Five sections in a single scrollable drill-down list -
//! the btop/weechat shape, one list plus a description of the highlighted item,
//! rather than htop's two columns, which needs width this pane does not have.
//!
//! Only the six Notification rows are editable, and deliberately so: they are
//! exactly the fields the runtime re-reads on every event, so a toggle takes
//! effect on the next event with no restart and no "restart required" badge.
//! Everything numeric or path-shaped is shown read-only under About / paths.

use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, Paragraph};

use crate::app::{AppState, SettingsNav, Tab};
use crate::claude_config::ClaudeConfigStore;
use crate::codex_config::CodexConfigStore;
use crate::config::{Config, NotificationMethod};
use crate::logging::LogFileInfo;
use crate::tui::panes::{side_mode, SideMode};
use crate::tui::theme::theme;
use crate::tui::views::truncate_string;
use crate::tui::widgets::selection::{selection_prefix, selection_style_with_accent};

/// The six editable notification rows, in list order
///
/// Editable *only* because these are the fields the runtime re-reads on every
/// event; nothing here needs a restart, so nothing here can be stale.
pub const NOTIFICATION_ROWS: [&str; 6] = [
    "Notify me by",
    "…on approval needed",
    "…on turn finished",
    "…on tool stalled",
    "…on session crashed",
    "Idle nudge counts as attention",
];

/// Pane 3's block title at the given density
pub fn settings_title(state: &AppState, mode: SideMode) -> String {
    match mode {
        SideMode::Strip | SideMode::Hidden => String::new(),
        _ => match state.settings_nav {
            SettingsNav::Sections => "Settings".to_string(),
            section => format!("Settings > {}", section.title()),
        },
    }
}

/// The one-line description of the highlighted row, for the global footer
pub fn settings_description(state: &AppState, config: &Config) -> String {
    match state.settings_nav {
        SettingsNav::Sections => SettingsNav::at(state.settings_section_index)
            .map(|section| section.description().to_string())
            .unwrap_or_default(),
        SettingsNav::Notifications => notification_description(state, config),
        section => section.description().to_string(),
    }
}

/// What the highlighted notification row currently means
fn notification_description(state: &AppState, config: &Config) -> String {
    match state.notifications_index {
        0 => format!(
            "←/→ to change · currently {}",
            method_label(config.notification_method)
        ),
        1..=5 => "Space to toggle · takes effect on the next event".to_string(),
        _ => String::new(),
    }
}

fn method_label(method: NotificationMethod) -> &'static str {
    match method {
        NotificationMethod::Bell => "Bell",
        NotificationMethod::Title => "Title",
        NotificationMethod::None => "Silent",
    }
}

/// Everything pane 3 needs to draw itself
pub struct SettingsPaneContext<'a> {
    pub state: &'a AppState,
    pub config: &'a Config,
    pub claude_config_store: &'a ClaudeConfigStore,
    pub codex_config_store: &'a CodexConfigStore,
    pub log_file_info: &'a LogFileInfo,
    pub hook_port: u16,
    pub hook_healthy: bool,
}

/// Render pane 3's content into `area` (already inside the pane border)
pub fn render_settings_pane(frame: &mut Frame, area: Rect, ctx: &SettingsPaneContext) {
    let mode = side_mode(area.width);
    if mode == SideMode::Hidden || area.height == 0 {
        return;
    }
    if mode == SideMode::Strip {
        frame.render_widget(Paragraph::new("⚙").style(theme().muted_style()), area);
        return;
    }

    match ctx.state.settings_nav {
        SettingsNav::Sections => render_sections(frame, area, ctx.state),
        SettingsNav::ClaudeConfigs => super::render_agent_config_list(
            frame,
            area,
            ctx.claude_config_store,
            ctx.state.claude_configs_selected_index,
        ),
        SettingsNav::CodexConfigs => super::render_agent_config_list(
            frame,
            area,
            ctx.codex_config_store,
            ctx.state.codex_configs_selected_index,
        ),
        SettingsNav::Shortcuts => super::render_shortcuts_list(
            frame,
            area,
            ctx.config,
            ctx.state.custom_shortcuts_selected,
        ),
        SettingsNav::Notifications => render_notifications(frame, area, ctx.state, ctx.config),
        SettingsNav::About => render_about(frame, area, ctx),
    }
}

/// The five sections
fn render_sections(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme();
    let focused = state.is_focused(Tab::Settings);
    let width = area.width as usize;

    let items: Vec<ListItem> = SettingsNav::SECTIONS
        .iter()
        .enumerate()
        .map(|(i, section)| {
            let selected = i == state.settings_section_index && focused;
            let content = format!("{}{}", selection_prefix(selected), section.title());
            ListItem::new(truncate_string(&content, width))
                .style(selection_style_with_accent(selected, t))
        })
        .collect();

    frame.render_widget(List::new(items), area);
}

/// The six live notification toggles
fn render_notifications(frame: &mut Frame, area: Rect, state: &AppState, config: &Config) {
    let t = theme();
    let focused = state.is_focused(Tab::Settings);
    let width = area.width as usize;

    let values = [
        format!("< {} >", method_label(config.notification_method)),
        checkbox(config.notify_on.approval),
        checkbox(config.notify_on.turn_complete),
        checkbox(config.notify_on.stalled),
        checkbox(config.notify_on.crashed),
        checkbox(config.attention_on_idle),
    ];

    let items: Vec<ListItem> = NOTIFICATION_ROWS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let selected = i == state.notifications_index && focused;
            let content = format!("{}{} {}", selection_prefix(selected), values[i], label);
            ListItem::new(truncate_string(&content, width))
                .style(selection_style_with_accent(selected, t))
        })
        .collect();

    frame.render_widget(List::new(items), area);
}

fn checkbox(on: bool) -> String {
    if on {
        "[x]".to_string()
    } else {
        "[ ]".to_string()
    }
}

/// Version, hook health, where the files live, and the startup-only settings
fn render_about(frame: &mut Frame, area: Rect, ctx: &SettingsPaneContext) {
    let t = theme();
    let config = ctx.config;
    let width = area.width as usize;

    let hook_status = if ctx.hook_healthy {
        format!("listening on :{}", ctx.hook_port)
    } else {
        format!("STOPPED (was :{})", ctx.hook_port)
    };

    let rows: Vec<(String, String)> = vec![
        ("Version".to_string(), env!("CARGO_PKG_VERSION").to_string()),
        ("Hook server".to_string(), hook_status),
        (
            "config.toml".to_string(),
            crate::config::config_file_path().display().to_string(),
        ),
        (
            "logs/".to_string(),
            ctx.log_file_info.path.display().to_string(),
        ),
        (
            "projects.json".to_string(),
            crate::project::store::projects_file_path()
                .display()
                .to_string(),
        ),
        (
            "sessions.json".to_string(),
            crate::session::store::sessions_file_path()
                .display()
                .to_string(),
        ),
        (
            "worktrees/".to_string(),
            config.worktrees_dir.display().to_string(),
        ),
        ("hooks/".to_string(), config.hooks_dir.display().to_string()),
        (
            "scrollback_lines".to_string(),
            format!("{} (new sessions only)", config.scrollback_lines),
        ),
        (
            "log_agent_events".to_string(),
            format!("{} (startup only)", config.log_agent_events),
        ),
    ];

    let label_width = rows
        .iter()
        .map(|(l, _)| l.chars().count())
        .max()
        .unwrap_or(0);
    let lines: Vec<Line> = rows
        .iter()
        .map(|(label, value)| {
            let text = format!("{:<label_width$}  {}", label, value);
            Line::from(Span::styled(
                truncate_string(&text, width),
                Style::default().fg(t.text),
            ))
        })
        .collect();

    let mut all = vec![Line::from(Span::styled(
        "Read-only. Edit config.toml for anything not offered above.",
        t.muted_style(),
    ))];
    all.extend(lines);

    frame.render_widget(Paragraph::new(all), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::views::test_util::{contains_line, render_to_lines};
    use std::path::PathBuf;

    fn context<'a>(
        state: &'a AppState,
        config: &'a Config,
        claude: &'a ClaudeConfigStore,
        codex: &'a CodexConfigStore,
        log: &'a LogFileInfo,
    ) -> SettingsPaneContext<'a> {
        SettingsPaneContext {
            state,
            config,
            claude_config_store: claude,
            codex_config_store: codex,
            log_file_info: log,
            hook_port: 9999,
            hook_healthy: true,
        }
    }

    fn render(width: u16, state: &AppState, config: &Config) -> Vec<String> {
        let claude = ClaudeConfigStore::new();
        let codex = CodexConfigStore::new();
        let log = LogFileInfo {
            path: PathBuf::from("/tmp/panoptes/logs/panoptes-now.log"),
        };
        let ctx = context(state, config, &claude, &codex, &log);
        render_to_lines(width, 16, |frame| {
            render_settings_pane(frame, frame.size(), &ctx)
        })
    }

    fn focused(nav: SettingsNav) -> AppState {
        AppState {
            focus: crate::app::Focus::Panes(Tab::Settings),
            settings_nav: nav,
            ..Default::default()
        }
    }

    #[test]
    fn test_sections_list_offers_all_five() {
        let lines = render(40, &focused(SettingsNav::Sections), &Config::default());
        for section in SettingsNav::SECTIONS {
            assert!(
                contains_line(&lines, section.title()),
                "{} missing from {lines:?}",
                section.title()
            );
        }
        assert!(contains_line(&lines, "▶ Claude configs"), "{lines:?}");
    }

    #[test]
    fn test_notifications_shows_six_editable_rows() {
        let config = Config::default();
        let lines = render(60, &focused(SettingsNav::Notifications), &config);

        assert!(contains_line(&lines, "< Bell > Notify me by"), "{lines:?}");
        assert!(
            contains_line(&lines, "[x] …on approval needed"),
            "{lines:?}"
        );
        assert!(contains_line(&lines, "[ ] …on tool stalled"), "{lines:?}");
        assert!(
            contains_line(&lines, "[ ] Idle nudge counts as attention"),
            "{lines:?}"
        );
    }

    #[test]
    fn test_about_shows_version_hook_and_paths() {
        let lines = render(70, &focused(SettingsNav::About), &Config::default());

        assert!(
            contains_line(&lines, env!("CARGO_PKG_VERSION")),
            "{lines:?}"
        );
        assert!(contains_line(&lines, "listening on :9999"), "{lines:?}");
        assert!(contains_line(&lines, "config.toml"), "{lines:?}");
        assert!(contains_line(&lines, "panoptes-now.log"), "{lines:?}");
        assert!(contains_line(&lines, "scrollback_lines"), "{lines:?}");
    }

    #[test]
    fn test_about_says_when_the_hook_server_is_gone() {
        let state = focused(SettingsNav::About);
        let config = Config::default();
        let claude = ClaudeConfigStore::new();
        let codex = CodexConfigStore::new();
        let log = LogFileInfo {
            path: PathBuf::from("/tmp/x.log"),
        };
        let mut ctx = context(&state, &config, &claude, &codex, &log);
        ctx.hook_healthy = false;

        let lines = render_to_lines(70, 16, |frame| {
            render_settings_pane(frame, frame.size(), &ctx)
        });
        assert!(contains_line(&lines, "STOPPED"), "{lines:?}");
    }

    #[test]
    fn test_strip_density_is_a_glyph() {
        let lines = render(10, &focused(SettingsNav::Sections), &Config::default());
        assert!(contains_line(&lines, "⚙"), "{lines:?}");
        assert!(!contains_line(&lines, "Claude configs"), "{lines:?}");
    }

    #[test]
    fn test_no_row_renders_past_the_pane_width() {
        for nav in SettingsNav::SECTIONS {
            for width in [22_u16, 30, 44] {
                let lines = render(width, &focused(nav), &Config::default());
                for line in &lines {
                    assert!(
                        line.chars().count() <= width as usize,
                        "row {line:?} overflows a {width}-column pane in {nav:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_footer_description_follows_the_highlighted_row() {
        let mut state = focused(SettingsNav::Sections);
        let config = Config::default();

        state.settings_section_index = 3;
        assert_eq!(
            settings_description(&state, &config),
            SettingsNav::Notifications.description()
        );

        state.settings_nav = SettingsNav::Notifications;
        state.notifications_index = 0;
        assert!(settings_description(&state, &config).contains("Bell"));
        state.notifications_index = 2;
        assert!(settings_description(&state, &config).contains("Space to toggle"));
    }

    #[test]
    fn test_title_names_the_open_section() {
        let state = focused(SettingsNav::Sections);
        assert_eq!(settings_title(&state, SideMode::Full), "Settings");

        let state = focused(SettingsNav::About);
        assert_eq!(
            settings_title(&state, SideMode::Full),
            "Settings > About / paths"
        );
        assert_eq!(settings_title(&state, SideMode::Strip), "");
    }
}
