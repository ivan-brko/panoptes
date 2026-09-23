//! Pane 3: settings
//!
//! Not a config editor. Six sections in a single scrollable drill-down list -
//! the btop/weechat shape, one list plus a description of the highlighted item,
//! rather than htop's two columns, which needs width this pane does not have.
//!
//! Only the seven Notification rows and the Theme presets are editable, and
//! deliberately so: they are exactly the settings the runtime re-reads rather
//! than caches, so a change takes effect immediately with no restart and no
//! "restart required" badge. Everything numeric or path-shaped is shown
//! read-only under About / paths.

use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, Paragraph};

use crate::app::{AppState, SettingsNav, Tab};
use crate::claude_config::ClaudeConfigStore;
use crate::codex_config::CodexConfigStore;
use crate::config::{Config, NotificationMethod, Palette};
use crate::logging::LogFileInfo;
use crate::tui::panes::SideMode;
use crate::tui::theme::{theme, Theme};
use crate::tui::views::pane_projects::clamp_line;
use crate::tui::views::{truncate_string, window_rows};
use crate::tui::widgets::selection::{selection_prefix, selection_style_with_accent};

/// The seven editable notification rows, in list order
///
/// Editable *only* because these are the fields the runtime re-reads on every
/// event; nothing here needs a restart, so nothing here can be stale.
pub const NOTIFICATION_ROWS: [&str; 7] = [
    "Notify me by",
    "…on approval needed",
    "…on turn finished",
    "…on tool stalled",
    "…on session crashed",
    "…on turn failed",
    "Idle nudge counts as attention",
];

/// The read-only About rows, in list order
///
/// Split from their values the way [`NOTIFICATION_ROWS`] is: the input handler
/// needs the row count to move a cursor through them, and has no business
/// building the paths and the hook's health to get it.
pub const ABOUT_ROWS: [&str; 11] = [
    "Version",
    "Hook server",
    "config.toml",
    "logs/",
    "projects.json",
    "sessions.json",
    "worktrees/",
    "hooks/",
    "scrollback_lines",
    "claude_status_line",
    "log_agent_events",
    "codex_shared_history",
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

/// Blank columns between a row's label and its description
const DESCRIPTION_GAP: &str = "   ";

/// The highlighted row, split at the point where it starts scrolling
///
/// The head - selection arrow, value, label - is what identifies the row, so
/// it holds still; only the description pans when the pane cannot hold both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptionRow {
    /// Everything left of the description, drawn in the row's own style
    pub head: String,
    /// The muted text trailing the label
    pub description: String,
}

impl DescriptionRow {
    /// Columns of the description that do not fit a `width`-wide row
    ///
    /// Zero when the row is too narrow to show any of it: there is nothing on
    /// screen to scroll, so nothing to animate either.
    pub fn overflow(&self, width: usize) -> usize {
        let room = self.room(width);
        if room == 0 {
            0
        } else {
            self.description.chars().count().saturating_sub(room)
        }
    }

    /// Columns left for the description after the head and its gap
    fn room(&self, width: usize) -> usize {
        width.saturating_sub(self.head.chars().count() + DESCRIPTION_GAP.chars().count())
    }

    /// The row as a styled line, panned `offset` columns into the description
    fn line(&self, width: usize, offset: usize, t: Theme) -> Line<'static> {
        let mut spans = vec![Span::raw(self.head.clone())];
        let room = self.room(width);
        if room > 0 {
            let offset = offset.min(self.overflow(width));
            let visible: String = self.description.chars().skip(offset).take(room).collect();
            spans.push(Span::styled(
                format!("{}{}", DESCRIPTION_GAP, visible),
                t.muted_style(),
            ));
        }
        Line::from(spans)
    }
}

/// The highlighted row's description, where the level has one
///
/// Only the lists whose rows are a *choice* carry one - descriptions are for
/// choosing among rows. Inside the other sections the pane title already names
/// them, and their rows speak for themselves.
///
/// `None` while pane 3 is unfocused: an unfocused pane draws no selection, so
/// there is no row for a description to belong to.
pub fn description_row(state: &AppState, config: &Config) -> Option<DescriptionRow> {
    if !state.is_focused(Tab::Settings) {
        return None;
    }
    match state.settings_nav {
        SettingsNav::Sections => {
            let section = SettingsNav::at(state.settings_section_index)?;
            Some(DescriptionRow {
                head: format!("{}{}", selection_prefix(true), section.title()),
                description: section.description().to_string(),
            })
        }
        SettingsNav::Notifications => {
            let index = state.notifications_index;
            let label = NOTIFICATION_ROWS.get(index)?;
            Some(DescriptionRow {
                head: format!(
                    "{}{} {}",
                    selection_prefix(true),
                    notification_values(config)[index],
                    label
                ),
                description: notification_description(index).to_string(),
            })
        }
        SettingsNav::Theme => {
            let palette = Palette::at(state.palette_index)?;
            // The description says only what the preset looks like. How to
            // keep it is the footer's job, and which one is saved is already
            // said by the `★` two columns to the left.
            Some(DescriptionRow {
                head: format!(
                    "{}{}{}",
                    selection_prefix(true),
                    palette_marker(palette, config),
                    palette.label()
                ),
                description: palette.blurb().to_string(),
            })
        }
        _ => None,
    }
}

/// `★` on the saved preset, blank columns on the rest so labels stay aligned
fn palette_marker(palette: Palette, config: &Config) -> &'static str {
    if palette == config.palette {
        "★ "
    } else {
        "  "
    }
}

/// What the highlighted notification row offers
///
/// Deliberately silent about the current value: the row itself already shows
/// it, two columns to the left.
fn notification_description(index: usize) -> &'static str {
    match index {
        0 => "Space/Enter to change",
        1..=6 => "Space/Enter to toggle · takes effect on the next event",
        _ => "",
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
    /// Columns the highlighted row's description is scrolled by
    pub marquee_offset: usize,
}

/// Render pane 3's content into `area` (already inside the pane border)
///
/// `mode` is decided once by the caller from the *outer* pane width; see
/// [`super::pane_projects::render_projects_pane`].
pub fn render_settings_pane(
    frame: &mut Frame,
    area: Rect,
    mode: SideMode,
    ctx: &SettingsPaneContext,
) {
    if mode == SideMode::Hidden || area.height == 0 {
        return;
    }
    if mode == SideMode::Strip {
        frame.render_widget(Paragraph::new("⚙").style(theme().muted_style()), area);
        return;
    }

    // Selection is only shown while this pane has focus, like every other
    // list - a bright selected row inside a dimmed pane reads as a second
    // focus
    let focused = ctx.state.is_focused(Tab::Settings);
    match ctx.state.settings_nav {
        SettingsNav::Sections => render_sections(frame, area, ctx),
        SettingsNav::ClaudeConfigs => super::render_agent_config_list(
            frame,
            area,
            ctx.claude_config_store,
            ctx.state.claude_configs_selected_index,
            focused,
        ),
        SettingsNav::CodexConfigs => super::render_agent_config_list(
            frame,
            area,
            ctx.codex_config_store,
            ctx.state.codex_configs_selected_index,
            focused,
        ),
        SettingsNav::Shortcuts => super::render_shortcuts_list(
            frame,
            area,
            ctx.config,
            ctx.state.custom_shortcuts_selected,
            focused,
        ),
        SettingsNav::Notifications => render_notifications(frame, area, ctx),
        SettingsNav::Theme => render_palettes(frame, area, ctx),
        SettingsNav::About => render_about(frame, area, ctx),
    }
}

/// The six sections
fn render_sections(frame: &mut Frame, area: Rect, ctx: &SettingsPaneContext) {
    let t = theme();
    let state = ctx.state;
    let focused = state.is_focused(Tab::Settings);
    let width = area.width as usize;
    let highlighted = description_row(state, ctx.config);

    let items: Vec<ListItem> = SettingsNav::SECTIONS
        .iter()
        .enumerate()
        .map(|(i, section)| {
            let selected = i == state.settings_section_index && focused;
            let line = row_line(
                if selected { highlighted.as_ref() } else { None },
                || format!("{}{}", selection_prefix(selected), section.title()),
                width,
                ctx.marquee_offset,
                t,
            );
            ListItem::new(line).style(selection_style_with_accent(selected, t))
        })
        .collect();

    let items = window_rows(items, state.settings_section_index, area.height);
    frame.render_widget(List::new(items), area);
}

/// The seven live notification rows
fn render_notifications(frame: &mut Frame, area: Rect, ctx: &SettingsPaneContext) {
    let t = theme();
    let state = ctx.state;
    let focused = state.is_focused(Tab::Settings);
    let width = area.width as usize;
    let values = notification_values(ctx.config);
    let highlighted = description_row(state, ctx.config);

    let items: Vec<ListItem> = NOTIFICATION_ROWS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let selected = i == state.notifications_index && focused;
            let line = row_line(
                if selected { highlighted.as_ref() } else { None },
                || format!("{}{} {}", selection_prefix(selected), values[i], label),
                width,
                ctx.marquee_offset,
                t,
            );
            ListItem::new(line).style(selection_style_with_accent(selected, t))
        })
        .collect();

    let items = window_rows(items, state.notifications_index, area.height);
    frame.render_widget(List::new(items), area);
}

/// The colour presets, with the saved one marked
///
/// The marker names the *saved* preset, not the highlighted one, because the
/// highlighted one is already being worn - without the marker there would be
/// nothing on screen saying which one survives an Esc.
fn render_palettes(frame: &mut Frame, area: Rect, ctx: &SettingsPaneContext) {
    let t = theme();
    let state = ctx.state;
    let focused = state.is_focused(Tab::Settings);
    let width = area.width as usize;
    let highlighted = description_row(state, ctx.config);

    let items: Vec<ListItem> = Palette::ALL
        .iter()
        .enumerate()
        .map(|(i, palette)| {
            let selected = i == state.palette_index && focused;
            let saved = *palette == ctx.config.palette;
            let line = row_line(
                if selected { highlighted.as_ref() } else { None },
                || {
                    format!(
                        "{}{}{}",
                        selection_prefix(selected),
                        palette_marker(*palette, ctx.config),
                        palette.label()
                    )
                },
                width,
                ctx.marquee_offset,
                t,
            );
            // The saved-but-unhighlighted row wears the marker's own colour,
            // so the star reads as a marker rather than as more list
            let style = if saved && !selected {
                Style::default().fg(t.default_marker)
            } else {
                selection_style_with_accent(selected, t)
            };
            ListItem::new(line).style(style)
        })
        .collect();

    let items = window_rows(items, state.palette_index, area.height);
    frame.render_widget(List::new(items), area);
}

/// One list row: the highlighted one carries its description, the rest are bare
fn row_line(
    highlighted: Option<&DescriptionRow>,
    plain: impl FnOnce() -> String,
    width: usize,
    offset: usize,
    t: Theme,
) -> Line<'static> {
    match highlighted {
        Some(row) => clamp_line(row.line(width, offset, t), width),
        None => Line::from(Span::raw(truncate_string(&plain(), width))),
    }
}

/// The value each notification row shows, in list order
fn notification_values(config: &Config) -> [String; 7] {
    [
        format!("< {} >", method_label(config.notification_method)),
        checkbox(config.notify_on.approval),
        checkbox(config.notify_on.turn_complete),
        checkbox(config.notify_on.stalled),
        checkbox(config.notify_on.crashed),
        checkbox(config.notify_on.failed),
        checkbox(config.attention_on_idle),
    ]
}

fn checkbox(on: bool) -> String {
    if on {
        "[x]".to_string()
    } else {
        "[ ]".to_string()
    }
}

/// The value each About row shows, in list order
fn about_values(ctx: &SettingsPaneContext) -> [String; 11] {
    let config = ctx.config;
    [
        env!("CARGO_PKG_VERSION").to_string(),
        if ctx.hook_healthy {
            format!("listening on :{}", ctx.hook_port)
        } else {
            format!("STOPPED (was :{})", ctx.hook_port)
        },
        crate::config::config_file_path().display().to_string(),
        ctx.log_file_info.path.display().to_string(),
        crate::project::store::projects_file_path()
            .display()
            .to_string(),
        crate::session::store::sessions_file_path()
            .display()
            .to_string(),
        config.worktrees_dir.display().to_string(),
        config.hooks_dir.display().to_string(),
        format!("{} (new sessions only)", config.scrollback_lines),
        format!("{} (new sessions only)", config.claude_status_line),
        format!("{} (startup only)", config.log_agent_events),
        codex_shared_history_value(config, &ctx.state.codex_history_warnings),
    ]
}

/// The shared-history row: off, or on and where to, and whether the startup
/// check of the shadow homes found anything (the log has the details)
fn codex_shared_history_value(config: &Config, warnings: &[String]) -> String {
    if !config.codex_shared_history {
        return "false (startup only)".to_string();
    }
    let homes = crate::codex_config::CodexHomes::from_config(config);
    let shared = homes.shared_home().display();
    match warnings.len() {
        0 => format!("true, in {} (startup only)", shared),
        // The count leads, so a narrow pane truncates the path, not the news
        n => format!(
            "true, {} warning{} (see log), in {}",
            n,
            if n == 1 { "" } else { "s" },
            shared
        ),
    }
}

/// Version, hook health, where the files live, and the startup-only settings
///
/// Read-only, but it carries a cursor anyway - not to act on a row, but so the
/// list can scroll: eleven rows is more than a short pane holds, and without a
/// selection to follow the tail was clipped with nothing on screen saying so.
fn render_about(frame: &mut Frame, area: Rect, ctx: &SettingsPaneContext) {
    let t = theme();
    let state = ctx.state;
    let focused = state.is_focused(Tab::Settings);
    let width = area.width as usize;

    let values = about_values(ctx);
    let label_width = ABOUT_ROWS
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);

    let items: Vec<ListItem> = ABOUT_ROWS
        .iter()
        .zip(values.iter())
        .enumerate()
        .map(|(i, (label, value))| {
            let selected = i == state.about_index && focused;
            let text = format!(
                "{}{:<label_width$}  {}",
                selection_prefix(selected),
                label,
                value
            );
            ListItem::new(Line::from(Span::raw(truncate_string(&text, width))))
                .style(selection_style_with_accent(selected, t))
        })
        .collect();

    // The note keeps its own row above the list, so scrolling cannot take away
    // the one line that says why nothing here responds to Enter
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Read-only. Edit config.toml for anything not offered above.",
            t.muted_style(),
        ))),
        chunks[0],
    );
    let items = window_rows(items, state.about_index, chunks[1].height);
    frame.render_widget(List::new(items), chunks[1]);
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
            marquee_offset: 0,
        }
    }

    fn render(width: u16, state: &AppState, config: &Config) -> Vec<String> {
        render_scrolled(width, state, config, 0)
    }

    /// Render with the description panned `offset` columns
    fn render_scrolled(
        width: u16,
        state: &AppState,
        config: &Config,
        offset: usize,
    ) -> Vec<String> {
        let claude = ClaudeConfigStore::new();
        let codex = CodexConfigStore::new();
        let log = LogFileInfo {
            path: PathBuf::from("/tmp/panoptes/logs/panoptes-now.log"),
        };
        let mut ctx = context(state, config, &claude, &codex, &log);
        ctx.marquee_offset = offset;
        let mode = crate::tui::panes::side_mode(width + 2);
        render_to_lines(width, 16, |frame| {
            render_settings_pane(frame, frame.size(), mode, &ctx)
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
    fn test_sections_list_offers_every_section() {
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
    fn test_notifications_shows_seven_editable_rows() {
        let config = Config::default();
        let lines = render(60, &focused(SettingsNav::Notifications), &config);

        assert!(contains_line(&lines, "< Bell > Notify me by"), "{lines:?}");
        assert!(
            contains_line(&lines, "[x] …on approval needed"),
            "{lines:?}"
        );
        assert!(contains_line(&lines, "[ ] …on tool stalled"), "{lines:?}");
        assert!(contains_line(&lines, "[x] …on turn failed"), "{lines:?}");
        assert!(
            contains_line(&lines, "[ ] Idle nudge counts as attention"),
            "{lines:?}"
        );
    }

    #[test]
    fn test_theme_lists_every_preset_and_marks_the_saved_one() {
        let config = Config {
            palette: Palette::Hera,
            ..Config::default()
        };
        let lines = render(60, &focused(SettingsNav::Theme), &config);

        for palette in Palette::ALL {
            assert!(
                contains_line(&lines, palette.label()),
                "{} missing from {lines:?}",
                palette.label()
            );
        }
        assert!(contains_line(&lines, "★ Hera"), "{lines:?}");
        // Exactly one star: the marker names the saved preset, and there is
        // only ever one of those
        assert_eq!(
            lines.iter().filter(|l| l.contains('★')).count(),
            1,
            "{lines:?}"
        );
    }

    /// The picker's description says what a preset looks like; the `★` says
    /// which one is saved, and the footer says how to keep the other
    #[test]
    fn test_theme_description_describes_the_highlighted_preset() {
        let mut state = focused(SettingsNav::Theme);
        let config = Config::default();

        state.palette_index = Palette::Peacock.index();
        let row = description_row(&state, &config).unwrap();
        assert!(row.head.contains("★ Peacock"), "{}", row.head);
        assert_eq!(row.description, Palette::Peacock.blurb());

        // The star stays behind on the row that owns it
        state.palette_index = Palette::Argus.index();
        let row = description_row(&state, &config).unwrap();
        assert!(!row.head.contains('★'), "{}", row.head);
        assert_eq!(row.description, Palette::Argus.blurb());
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
            render_settings_pane(frame, frame.size(), SideMode::Full, &ctx)
        });
        assert!(contains_line(&lines, "STOPPED"), "{lines:?}");
    }

    /// A short pane cannot hold all eleven rows, so the cursor has to drag the
    /// list along with it - the last row used to be clipped silently
    #[test]
    fn test_about_scrolls_to_the_selected_row_in_a_short_pane() {
        let mut state = focused(SettingsNav::About);
        state.about_index = ABOUT_ROWS.len() - 1;
        let config = Config::default();
        let claude = ClaudeConfigStore::new();
        let codex = CodexConfigStore::new();
        let log = LogFileInfo {
            path: PathBuf::from("/tmp/x.log"),
        };
        let ctx = context(&state, &config, &claude, &codex, &log);

        let lines = render_to_lines(70, 6, |frame| {
            render_settings_pane(frame, frame.size(), SideMode::Full, &ctx)
        });

        assert!(contains_line(&lines, "▶ codex_shared_history"), "{lines:?}");
        // The window moved rather than growing: the first row is gone, and the
        // note that is not part of the list held its place
        assert!(!contains_line(&lines, "Version"), "{lines:?}");
        assert!(contains_line(&lines, "Read-only."), "{lines:?}");
    }

    #[test]
    fn test_about_says_whether_codex_history_is_shared_and_warns_by_count() {
        assert_eq!(
            codex_shared_history_value(&Config::default(), &[]),
            "false (startup only)"
        );

        let config = Config {
            codex_shared_history: true,
            codex_shared_home: Some(PathBuf::from("/srv/codex")),
            ..Config::default()
        };
        assert_eq!(
            codex_shared_history_value(&config, &[]),
            "true, in /srv/codex (startup only)"
        );
        assert_eq!(
            codex_shared_history_value(&config, &["a".to_string(), "b".to_string()]),
            "true, 2 warnings (see log), in /srv/codex"
        );
    }

    /// Selection is a focus affordance here as everywhere else
    #[test]
    fn test_about_draws_no_cursor_while_the_pane_is_unfocused() {
        let state = AppState {
            settings_nav: SettingsNav::About,
            ..Default::default()
        };
        let lines = render(70, &state, &Config::default());
        assert!(!contains_line(&lines, "▶ "), "{lines:?}");
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

    /// The description belongs to the row the user is on, and moves with it
    #[test]
    fn test_the_description_follows_the_highlighted_row() {
        let mut state = focused(SettingsNav::Sections);
        let config = Config::default();

        state.settings_section_index = 3;
        let row = description_row(&state, &config).unwrap();
        assert_eq!(row.head, "▶ Notifications");
        assert_eq!(row.description, SettingsNav::Notifications.description());

        state.settings_nav = SettingsNav::Notifications;
        state.notifications_index = 0;
        let row = description_row(&state, &config).unwrap();
        // The description names the key that actually works here: the arrows
        // cycle panes now, so advertising them would send the user away
        assert_eq!(row.description, "Space/Enter to change");
        // ...and not the current method, which the row already shows
        assert!(row.head.contains("< Bell >"), "{}", row.head);
        assert!(!row.description.contains("Bell"), "{}", row.description);

        state.notifications_index = 2;
        let row = description_row(&state, &config).unwrap();
        assert!(row.description.contains("Space/Enter to toggle"));
    }

    /// Descriptions are for choosing *among* rows. Inside a section the title
    /// already names it, so nothing static trails the rows there.
    #[test]
    fn test_only_the_choosing_lists_carry_a_description() {
        let config = Config::default();
        for nav in [
            SettingsNav::ClaudeConfigs,
            SettingsNav::CodexConfigs,
            SettingsNav::Shortcuts,
            SettingsNav::About,
        ] {
            assert!(
                description_row(&focused(nav), &config).is_none(),
                "{nav:?} should carry no row description"
            );
        }
    }

    /// An unfocused pane draws no selection, so it has no row to describe
    #[test]
    fn test_an_unfocused_pane_describes_nothing() {
        let state = AppState {
            focus: crate::app::Focus::Panes(Tab::Projects),
            settings_nav: SettingsNav::Sections,
            ..Default::default()
        };
        assert!(description_row(&state, &Config::default()).is_none());

        let lines = render(60, &state, &Config::default());
        assert!(contains_line(&lines, "Claude configs"), "{lines:?}");
        assert!(
            !contains_line(&lines, SettingsNav::ClaudeConfigs.description()),
            "{lines:?}"
        );
    }

    #[test]
    fn test_the_highlighted_row_shows_its_description_inline() {
        let config = Config::default();

        let lines = render(60, &focused(SettingsNav::Sections), &config);
        assert!(
            contains_line(
                &lines,
                &format!(
                    "▶ Claude configs   {}",
                    SettingsNav::ClaudeConfigs.description()
                )
            ),
            "{lines:?}"
        );
        // Only the highlighted row: the others stay clean
        assert!(
            !contains_line(&lines, SettingsNav::CodexConfigs.description()),
            "{lines:?}"
        );

        let lines = render(60, &focused(SettingsNav::Notifications), &config);
        assert!(
            contains_line(&lines, "▶ < Bell > Notify me by   Space/Enter to change"),
            "{lines:?}"
        );
        assert!(
            !contains_line(&lines, "takes effect on the next event"),
            "{lines:?}"
        );
    }

    /// The description is a second voice on the row, so it is muted while the
    /// label keeps the selection's accent
    #[test]
    fn test_the_description_is_muted() {
        let claude = ClaudeConfigStore::new();
        let codex = CodexConfigStore::new();
        let config = Config::default();
        let log = LogFileInfo {
            path: PathBuf::from("/tmp/x.log"),
        };
        let state = focused(SettingsNav::Sections);
        let ctx = context(&state, &config, &claude, &codex, &log);
        let buffer = crate::tui::views::test_util::render_to_buffer(60, 16, |frame| {
            render_settings_pane(frame, frame.size(), SideMode::Full, &ctx)
        });

        let t = theme();
        let description = crate::tui::views::test_util::style_of_row_with(
            &buffer,
            SettingsNav::ClaudeConfigs.description(),
        );
        assert_eq!(description.fg, Some(t.muted_style().fg.unwrap()));
        assert_eq!(
            crate::tui::views::test_util::style_of_row_with(&buffer, "Claude configs").fg,
            Some(t.accent)
        );
    }

    /// Only the description pans: the arrow and the label are how the user
    /// knows which row they are on, so they never move
    #[test]
    fn test_scrolling_pans_the_description_and_nothing_else() {
        let config = Config::default();
        let state = focused(SettingsNav::Sections);

        // 34 columns holds "▶ Claude configs" plus a stub of its description
        let head = "▶ Claude configs   ";
        let full = render_scrolled(34, &state, &config, 0);
        let panned = render_scrolled(34, &state, &config, 4);

        let row = |lines: &[String]| {
            lines
                .iter()
                .find(|l| l.contains("Claude configs"))
                .unwrap()
                .clone()
        };
        let (before, after) = (row(&full), row(&panned));
        let description = SettingsNav::ClaudeConfigs.description();

        // The label holds still...
        let before = before.strip_prefix(head).expect("{before:?}");
        let after = after.strip_prefix(head).expect("{after:?}");
        // ...while the description slides four columns to the left under it
        let room = before.chars().count();
        assert_eq!(before, &description[..room]);
        assert_eq!(after, &description[4..4 + room]);
    }

    /// A row too narrow for any description at all has nothing to scroll -
    /// which is also what keeps the tick loop quiet at a strip's width
    #[test]
    fn test_a_row_with_no_room_for_a_description_never_scrolls() {
        let row = DescriptionRow {
            head: "▶ Notifications".to_string(),
            description: "What interrupts you, and how".to_string(),
        };
        assert_eq!(row.overflow(10), 0);
        assert_eq!(row.overflow(18), 0, "the gap alone leaves no room");
        assert_eq!(row.overflow(1_000), 0, "it all fits");

        // 15 head + 3 gap + 8 columns of a 28-column description
        assert_eq!(row.overflow(26), 28 - 8);
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
