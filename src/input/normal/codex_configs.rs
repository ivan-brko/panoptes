//! Codex configs input handler
//!
//! Handles keyboard input in the Codex configs management view.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{App, InputMode};

/// Handle key in Codex configs view (normal mode)
pub fn handle_codex_configs_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Only process key press events (not release/repeat)
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    // Handle focus timer shortcuts (t, T, Ctrl+t)
    if app.handle_focus_timer_shortcut(key) {
        return Ok(());
    }

    let config_count = app.codex_config_store.count();

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.state.navigate_back();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if config_count > 0 {
                app.state.select_next(config_count);
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if config_count > 0 {
                app.state.select_prev(config_count);
            }
        }
        KeyCode::Char('n') => {
            // Start creating a new config
            app.state.new_codex_config_name.clear();
            app.state.new_codex_config_path.clear();
            app.state.input_mode = InputMode::AddingCodexConfigName;
        }
        KeyCode::Char('s') => {
            // Set selected config as default
            if config_count > 0 {
                let configs = app.codex_config_store.configs_sorted();
                if let Some(config) = configs.get(app.state.codex_configs_selected_index) {
                    let config_id = config.id;
                    if app.codex_config_store.set_default(config_id) {
                        if let Err(e) = app.codex_config_store.save() {
                            tracing::error!("Failed to save codex config store: {}", e);
                            app.state.error_message = Some(format!("Failed to save: {}", e));
                        }
                    }
                }
            }
        }
        KeyCode::Char('d') => {
            // Prompt for confirmation before deleting
            if config_count > 0 {
                let configs = app.codex_config_store.configs_sorted();
                if let Some(config) = configs.get(app.state.codex_configs_selected_index) {
                    app.state.pending_delete_codex_config = Some(config.id);
                    app.state.input_mode = InputMode::ConfirmingCodexConfigDelete;
                }
            }
        }
        _ => {}
    }
    Ok(())
}
