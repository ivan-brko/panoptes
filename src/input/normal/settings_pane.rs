//! Pane 3 input: settings sections and their drill-downs
//!
//! The Notification toggles write straight through [`Config::save`] on every
//! keystroke. That is safe precisely because these six fields are the ones the
//! runtime re-reads on every event: nothing caches them, so a toggle takes
//! effect on the next event with no reload path to build.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{cycle_next, cycle_prev, App, InputMode, SettingsNav};
use crate::config::NotificationMethod;
use crate::input::agent_configs::AgentKind;
use crate::tui::views::pane_settings::NOTIFICATION_ROWS;

/// Handle a normal-mode key while pane 3 has focus
pub fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    match app.state.settings_nav {
        SettingsNav::Sections => handle_sections_key(app, key),
        SettingsNav::ClaudeConfigs => {
            crate::input::agent_configs::handle_configs_section_key(app, key, AgentKind::Claude)
        }
        SettingsNav::CodexConfigs => {
            crate::input::agent_configs::handle_configs_section_key(app, key, AgentKind::Codex)
        }
        SettingsNav::Shortcuts => handle_shortcuts_key(app, key),
        SettingsNav::Notifications => handle_notifications_key(app, key),
        SettingsNav::About => handle_about_key(app, key),
    }
}

fn handle_sections_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let count = SettingsNav::SECTIONS.len();
    match key.code {
        KeyCode::Esc => {
            // Root of the pane: Esc is a no-op, deliberately
        }
        KeyCode::Down => {
            app.state.settings_section_index = cycle_next(app.state.settings_section_index, count);
        }
        KeyCode::Up => {
            app.state.settings_section_index = cycle_prev(app.state.settings_section_index, count);
        }
        KeyCode::Enter => {
            if let Some(section) = SettingsNav::at(app.state.settings_section_index) {
                app.state.settings_nav = section;
                // Each section starts at the top, so a stale index from a
                // previous visit cannot point past a list that has shrunk
                match section {
                    SettingsNav::ClaudeConfigs => app.state.claude_configs_selected_index = 0,
                    SettingsNav::CodexConfigs => app.state.codex_configs_selected_index = 0,
                    SettingsNav::Shortcuts => app.state.custom_shortcuts_selected = 0,
                    SettingsNav::Notifications => app.state.notifications_index = 0,
                    _ => {}
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_shortcuts_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let count = app.config.custom_shortcuts.len();
    match key.code {
        KeyCode::Esc => {
            app.state.navigate_back();
        }
        KeyCode::Down => {
            app.state.custom_shortcuts_selected =
                cycle_next(app.state.custom_shortcuts_selected, count);
        }
        KeyCode::Up => {
            app.state.custom_shortcuts_selected =
                cycle_prev(app.state.custom_shortcuts_selected, count);
        }
        KeyCode::Char('n') => {
            app.state.new_shortcut_key = None;
            app.state.new_shortcut_name.clear();
            app.state.new_shortcut_command.clear();
            app.state.new_shortcut_auto_close = false;
            app.state.shortcut_error = None;
            app.state.input_mode = InputMode::AddingCustomShortcutKey;
        }
        KeyCode::Char('d') => {
            if count > 0 {
                app.state.pending_delete_shortcut_index = Some(app.state.custom_shortcuts_selected);
                app.state.input_mode = InputMode::ConfirmingCustomShortcutDelete;
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_notifications_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let count = NOTIFICATION_ROWS.len();
    match key.code {
        KeyCode::Esc => {
            app.state.navigate_back();
            return Ok(());
        }
        KeyCode::Down => {
            app.state.notifications_index = cycle_next(app.state.notifications_index, count);
            return Ok(());
        }
        KeyCode::Up => {
            app.state.notifications_index = cycle_prev(app.state.notifications_index, count);
            return Ok(());
        }
        KeyCode::Char(' ') | KeyCode::Enter => {
            // Space is the global attention jump everywhere except here; the
            // dispatcher defers to this section so the toggle keeps the key
            if !toggle_row(app) {
                return Ok(());
            }
        }
        KeyCode::Left => {
            if app.state.notifications_index != 0 {
                return Ok(());
            }
            app.config.notification_method = prev_method(app.config.notification_method);
        }
        KeyCode::Right => {
            if app.state.notifications_index != 0 {
                return Ok(());
            }
            app.config.notification_method = next_method(app.config.notification_method);
        }
        _ => return Ok(()),
    }

    persist(app);
    Ok(())
}

/// Flip the boolean the highlighted row controls; returns whether one moved
fn toggle_row(app: &mut App) -> bool {
    match app.state.notifications_index {
        0 => {
            app.config.notification_method = next_method(app.config.notification_method);
            true
        }
        1 => {
            app.config.notify_on.approval = !app.config.notify_on.approval;
            true
        }
        2 => {
            app.config.notify_on.turn_complete = !app.config.notify_on.turn_complete;
            true
        }
        3 => {
            app.config.notify_on.stalled = !app.config.notify_on.stalled;
            true
        }
        4 => {
            app.config.notify_on.crashed = !app.config.notify_on.crashed;
            true
        }
        5 => {
            app.config.attention_on_idle = !app.config.attention_on_idle;
            true
        }
        _ => false,
    }
}

/// Apply the change to the running app, then write it out
///
/// The session manager keeps its own copy of the config and the state machine
/// reads that copy on every event, so pushing the change across is what makes
/// "takes effect on the next event" true rather than aspirational.
fn persist(app: &mut App) {
    app.sessions.apply_runtime_config(&app.config);
    if let Err(e) = app.config.save() {
        tracing::error!("Failed to save config: {}", e);
        app.state.error_message = Some(format!("Failed to save config: {}", e));
    }
}

fn handle_about_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.code == KeyCode::Esc {
        app.state.navigate_back();
    }
    Ok(())
}

/// The notification methods in `←/→` order
const METHODS: [NotificationMethod; 3] = [
    NotificationMethod::Bell,
    NotificationMethod::Title,
    NotificationMethod::None,
];

fn next_method(method: NotificationMethod) -> NotificationMethod {
    let index = METHODS.iter().position(|m| *m == method).unwrap_or(0);
    METHODS[(index + 1) % METHODS.len()]
}

fn prev_method(method: NotificationMethod) -> NotificationMethod {
    let index = METHODS.iter().position(|m| *m == method).unwrap_or(0);
    METHODS[(index + METHODS.len() - 1) % METHODS.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_method_cycles_both_ways_with_wraparound() {
        assert_eq!(
            next_method(NotificationMethod::Bell),
            NotificationMethod::Title
        );
        assert_eq!(
            next_method(NotificationMethod::Title),
            NotificationMethod::None
        );
        assert_eq!(
            next_method(NotificationMethod::None),
            NotificationMethod::Bell
        );

        assert_eq!(
            prev_method(NotificationMethod::Bell),
            NotificationMethod::None
        );
        assert_eq!(
            prev_method(NotificationMethod::None),
            NotificationMethod::Title
        );
    }
}
