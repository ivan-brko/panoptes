//! Shared handlers for the Claude and Codex config flows
//!
//! Claude Code and Codex configs go through identical add / select / delete
//! dialogs, differing only in which [`ProfileStore`] they touch, which
//! [`InputMode`] variants they move between, and which `Project` field holds
//! the per-project default. [`AgentKind`] carries those differences; the
//! handlers themselves are written once.
//!
//! Handlers that only need parts of [`App`] take those parts directly
//! (`AppState`, the store, the project store) so they can be unit tested
//! without a terminal; thin `handle_*` wrappers destructure `App` for the
//! dispatcher.

use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use uuid::Uuid;

use crate::agent::AgentType;
use crate::agent_profiles::{AgentProfile, ProfileStore};
use crate::app::{cycle_next, cycle_prev, App, AppState, InputMode};
use crate::project::{Project, ProjectId, ProjectStore};
use crate::session::AgentAccount;

/// Maximum length for an agent config name
pub(crate) const MAX_CONFIG_NAME_LEN: usize = 50;
/// Maximum length for an agent config path
pub(crate) const MAX_CONFIG_PATH_LEN: usize = 500;

/// Which agent a config flow operates on
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    /// Claude Code (`CLAUDE_CONFIG_DIR` profiles)
    Claude,
    /// OpenAI Codex (`CODEX_HOME` profiles)
    Codex,
}

impl AgentKind {
    /// Human-readable label for log and error messages
    pub fn label(self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude",
            AgentKind::Codex => "Codex",
        }
    }

    /// The agent type sessions of this kind run as
    pub fn agent_type(self) -> AgentType {
        match self {
            AgentKind::Claude => AgentType::ClaudeCode,
            AgentKind::Codex => AgentType::OpenAICodex,
        }
    }

    /// Input mode for the config-name step of this agent's add flow
    pub fn adding_name_mode(self) -> InputMode {
        match self {
            AgentKind::Claude => InputMode::AddingClaudeConfigName,
            AgentKind::Codex => InputMode::AddingCodexConfigName,
        }
    }

    /// Input mode for the config-path step of this agent's add flow
    pub fn adding_path_mode(self) -> InputMode {
        match self {
            AgentKind::Claude => InputMode::AddingClaudeConfigPath,
            AgentKind::Codex => InputMode::AddingCodexConfigPath,
        }
    }

    /// Input mode for this agent's config selector
    pub fn selecting_mode(self) -> InputMode {
        match self {
            AgentKind::Claude => InputMode::SelectingClaudeConfig,
            AgentKind::Codex => InputMode::SelectingCodexConfig,
        }
    }

    /// Input mode for this agent's config delete confirmation
    pub fn confirming_delete_mode(self) -> InputMode {
        match self {
            AgentKind::Claude => InputMode::ConfirmingClaudeConfigDelete,
            AgentKind::Codex => InputMode::ConfirmingCodexConfigDelete,
        }
    }

    /// Hint shown when a project default is requested but no configs exist
    pub fn no_configs_hint(self) -> &'static str {
        match self {
            AgentKind::Claude => "No Claude configs defined. Press 'c' from homepage.",
            AgentKind::Codex => "No Codex configs defined. Press 'x' from homepage.",
        }
    }

    /// Number of configs currently offered in the selector
    fn available_len(self, state: &AppState) -> usize {
        match self {
            AgentKind::Claude => state.available_claude_configs.len(),
            AgentKind::Codex => state.available_codex_configs.len(),
        }
    }

    /// ID of the selector entry at `index`
    fn available_id_at(self, state: &AppState, index: usize) -> Option<Uuid> {
        match self {
            AgentKind::Claude => state.available_claude_configs.get(index).map(|c| c.id),
            AgentKind::Codex => state.available_codex_configs.get(index).map(|c| c.id),
        }
    }

    /// Account profile for the selector entry at `index`
    fn available_account_at(self, state: &AppState, index: usize) -> Option<AgentAccount> {
        match self {
            AgentKind::Claude => state.available_claude_configs.get(index).map(account_of),
            AgentKind::Codex => state.available_codex_configs.get(index).map(account_of),
        }
    }

    /// Position of `id` in the selector list
    fn available_position(self, state: &AppState, id: Option<Uuid>) -> Option<usize> {
        match self {
            AgentKind::Claude => state
                .available_claude_configs
                .iter()
                .position(|c| Some(c.id) == id),
            AgentKind::Codex => state
                .available_codex_configs
                .iter()
                .position(|c| Some(c.id) == id),
        }
    }

    /// Clear the selector list and selection
    fn clear_selector(self, state: &mut AppState) {
        match self {
            AgentKind::Claude => state.available_claude_configs.clear(),
            AgentKind::Codex => state.available_codex_configs.clear(),
        }
        state.config_selector_index = 0;
    }

    /// Selected index in this agent's configs management view
    fn view_selected_index(self, state: &AppState) -> usize {
        match self {
            AgentKind::Claude => state.claude_configs_selected_index,
            AgentKind::Codex => state.codex_configs_selected_index,
        }
    }

    /// Set the selected index in this agent's configs management view
    fn set_view_selected_index(self, state: &mut AppState, index: usize) {
        match self {
            AgentKind::Claude => state.claude_configs_selected_index = index,
            AgentKind::Codex => state.codex_configs_selected_index = index,
        }
    }

    /// This agent's default-config field on a project
    fn project_default(self, project: &Project) -> Option<Uuid> {
        match self {
            AgentKind::Claude => project.default_claude_config,
            AgentKind::Codex => project.default_codex_config,
        }
    }

    /// Set this agent's default-config field on a project
    fn set_project_default(self, project: &mut Project, value: Option<Uuid>) {
        match self {
            AgentKind::Claude => project.default_claude_config = value,
            AgentKind::Codex => project.default_codex_config = value,
        }
    }
}

/// Build the account profile for a selected config, either agent
fn account_of<C: AgentProfile>(config: &C) -> AgentAccount {
    AgentAccount {
        id: config.id(),
        name: config.name().to_string(),
        dir: config.home_dir().map(|p| p.to_path_buf()),
    }
}

// ========================================================================
// App-level wrappers (called by the dispatcher and normal-mode handlers)
// ========================================================================

/// Handle key while creating a new Claude or Codex session (typing the name)
pub fn handle_creating_agent_session_key(
    app: &mut App,
    key: KeyEvent,
    kind: AgentKind,
) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Esc => {
            app.state.input_mode = InputMode::Normal;
            app.state.session_draft.reset();
        }
        KeyCode::Enter => {
            let config_count = match kind {
                AgentKind::Claude => app.claude_config_store.count(),
                AgentKind::Codex => app.codex_config_store.count(),
            };

            if config_count > 1 {
                // Multiple configs - show selector, pre-selecting the project
                // default (or global default)
                let project_id = app.state.session_draft.project_id;
                open_config_selector(app, kind, project_id);
            } else if config_count == 1 {
                // Single config - use it directly
                let account = match kind {
                    AgentKind::Claude => account_of(app.claude_config_store.configs_sorted()[0]),
                    AgentKind::Codex => account_of(app.codex_config_store.configs_sorted()[0]),
                };
                crate::input::text_input::create_session(app, kind.agent_type(), Some(account))?;
            } else {
                // No configs - create without config
                crate::input::text_input::create_session(app, kind.agent_type(), None)?;
            }
        }
        KeyCode::Backspace => {
            app.state.session_draft.name.pop();
        }
        KeyCode::Char(c) => {
            if app.state.session_draft.name.len() < crate::app::MAX_SESSION_NAME_LEN {
                app.state.session_draft.name.push(c);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Populate and open the config selector for an agent
///
/// The one place that fills the "available configs" list: used both when a
/// session needs an account picked and when a project default is being set.
/// Pre-selects the project's default config when `project_id` names one,
/// falling back to the store's global default.
pub fn open_config_selector(app: &mut App, kind: AgentKind, project_id: Option<ProjectId>) {
    let project_default = project_id
        .and_then(|pid| app.project_store.get_project(pid))
        .and_then(|p| kind.project_default(p));

    let global_default = match kind {
        AgentKind::Claude => {
            app.state.available_claude_configs = app
                .claude_config_store
                .configs_sorted()
                .iter()
                .cloned()
                .cloned()
                .collect();
            app.claude_config_store.get_default_id()
        }
        AgentKind::Codex => {
            app.state.available_codex_configs = app
                .codex_config_store
                .configs_sorted()
                .iter()
                .cloned()
                .cloned()
                .collect();
            app.codex_config_store.get_default_id()
        }
    };

    let preferred_id = project_default.or(global_default);
    app.state.config_selector_index = kind
        .available_position(&app.state, preferred_id)
        .unwrap_or(0);
    app.state.input_mode = kind.selecting_mode();
}

/// Handle key while entering a config name (step 1)
pub fn handle_adding_config_name_key(app: &mut App, key: KeyEvent, kind: AgentKind) -> Result<()> {
    match kind {
        AgentKind::Claude => {
            adding_config_name_key(&mut app.state, &app.claude_config_store, kind, key)
        }
        AgentKind::Codex => {
            adding_config_name_key(&mut app.state, &app.codex_config_store, kind, key)
        }
    }
}

/// Handle key while entering a config path (step 2)
pub fn handle_adding_config_path_key(app: &mut App, key: KeyEvent, kind: AgentKind) -> Result<()> {
    match kind {
        AgentKind::Claude => {
            adding_config_path_key(&mut app.state, &mut app.claude_config_store, kind, key)
        }
        AgentKind::Codex => {
            adding_config_path_key(&mut app.state, &mut app.codex_config_store, kind, key)
        }
    }
}

/// Handle key while a config selector is open
pub fn handle_selecting_config_key(app: &mut App, key: KeyEvent, kind: AgentKind) -> Result<()> {
    if let Some(account) = selecting_config_key(&mut app.state, &mut app.project_store, kind, key)?
    {
        crate::input::text_input::create_session(app, kind.agent_type(), Some(account))?;
    }
    Ok(())
}

/// Handle key while confirming a config deletion
pub fn handle_confirming_config_delete_key(
    app: &mut App,
    key: KeyEvent,
    kind: AgentKind,
) -> Result<()> {
    match kind {
        AgentKind::Claude => confirming_config_delete_key(
            &mut app.state,
            &mut app.claude_config_store,
            &mut app.project_store,
            kind,
            key,
        ),
        AgentKind::Codex => confirming_config_delete_key(
            &mut app.state,
            &mut app.codex_config_store,
            &mut app.project_store,
            kind,
            key,
        ),
    }
}

/// Handle key in a configs management view (normal mode)
pub fn handle_configs_view_key(app: &mut App, key: KeyEvent, kind: AgentKind) -> Result<()> {
    match kind {
        AgentKind::Claude => {
            configs_view_key(&mut app.state, &mut app.claude_config_store, kind, key)
        }
        AgentKind::Codex => {
            configs_view_key(&mut app.state, &mut app.codex_config_store, kind, key)
        }
    }
}

// ========================================================================
// Parts-based handler bodies (unit-testable without a terminal)
// ========================================================================

/// Config-name step of the add flow
pub(crate) fn adding_config_name_key<C: AgentProfile>(
    state: &mut AppState,
    store: &ProfileStore<C>,
    kind: AgentKind,
    key: KeyEvent,
) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Esc => {
            state.input_mode = InputMode::Normal;
            state.config_draft.name.clear();
        }
        KeyCode::Enter => {
            let name = state.config_draft.name.trim().to_string();
            if name.is_empty() {
                state.error_message = Some("Config name cannot be empty".to_string());
                return Ok(());
            }
            // Check if name already exists
            if store.find_by_name(&name).is_some() {
                state.error_message = Some(format!("Config '{}' already exists", name));
                return Ok(());
            }
            // Move to path input step
            state.input_mode = kind.adding_path_mode();
            state.config_draft.path.clear();
            clear_config_path_completions(state);
        }
        KeyCode::Backspace => {
            state.config_draft.name.pop();
        }
        KeyCode::Char(c) => {
            if state.config_draft.name.len() < MAX_CONFIG_NAME_LEN {
                state.config_draft.name.push(c);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Config-path step of the add flow
pub(crate) fn adding_config_path_key<C: AgentProfile>(
    state: &mut AppState,
    store: &mut ProfileStore<C>,
    kind: AgentKind,
    key: KeyEvent,
) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Esc => {
            if state.show_path_completions {
                // First Esc hides completions
                clear_config_path_completions(state);
            } else {
                // Second Esc steps back to the name entry (keeping the name),
                // consistent with Esc meaning "back one level" elsewhere
                clear_config_path_completions(state);
                state.config_draft.path.clear();
                state.input_mode = kind.adding_name_mode();
            }
        }
        KeyCode::Tab => {
            if state.show_path_completions && !state.path_completions.is_empty() {
                apply_config_path_completion(state);
            } else {
                update_config_path_completions_state(state);
            }
        }
        KeyCode::BackTab | KeyCode::Up => {
            if state.show_path_completions {
                state.path_completion_index =
                    cycle_prev(state.path_completion_index, state.path_completions.len());
            }
        }
        KeyCode::Down => {
            if state.show_path_completions {
                state.path_completion_index =
                    cycle_next(state.path_completion_index, state.path_completions.len());
            }
        }
        KeyCode::Enter => {
            clear_config_path_completions(state);

            let name = std::mem::take(&mut state.config_draft.name);
            let path_str = std::mem::take(&mut state.config_draft.path);

            // Empty path means default config
            let home_dir = if path_str.trim().is_empty() {
                None
            } else {
                let expanded = PathBuf::from(shellexpand::tilde(&path_str).into_owned());
                // Validate the directory exists
                if !expanded.is_dir() {
                    state.error_message =
                        Some(format!("Directory does not exist: {}", expanded.display()));
                    state.config_draft.name = name;
                    state.config_draft.path = path_str;
                    return Ok(());
                }
                Some(expanded)
            };

            // Check if path is already used
            if store.is_home_dir_used(home_dir.as_deref()) {
                let path_display = home_dir
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "default".to_string());
                state.error_message = Some(format!("Path already used: {}", path_display));
                state.config_draft.name = name;
                state.config_draft.path = path_str;
                return Ok(());
            }

            // Create the config
            store.add(C::new_profile(name.clone(), home_dir));

            if let Err(e) = store.save() {
                tracing::error!("Failed to save {} config store: {}", kind.label(), e);
                state.error_message = Some(format!("Failed to save: {}", e));
            }

            tracing::info!("Added {} config: {}", kind.label(), name);
            state.input_mode = InputMode::Normal;
        }
        KeyCode::Backspace => {
            state.config_draft.path.pop();
            update_config_path_completions_state(state);
        }
        KeyCode::Char(c) => {
            if state.config_draft.path.len() < MAX_CONFIG_PATH_LEN {
                state.config_draft.path.push(c);
                update_config_path_completions_state(state);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Config selector body
///
/// Returns the account to create a session under when the selection was made
/// for session creation; setting a project default is applied here directly.
pub(crate) fn selecting_config_key(
    state: &mut AppState,
    project_store: &mut ProjectStore,
    kind: AgentKind,
    key: KeyEvent,
) -> Result<Option<AgentAccount>> {
    if key.kind != KeyEventKind::Press {
        return Ok(None);
    }

    let config_count = kind.available_len(state);

    match key.code {
        KeyCode::Esc => {
            // Cancel selection - abort session creation or project config setting
            state.input_mode = InputMode::Normal;
            kind.clear_selector(state);
            // Also clear session creation state
            state.session_draft.reset();
            state.setting_project_default_config = None;
        }
        KeyCode::Down => {
            state.config_selector_index = cycle_next(state.config_selector_index, config_count);
        }
        KeyCode::Up => {
            state.config_selector_index = cycle_prev(state.config_selector_index, config_count);
        }
        KeyCode::Enter => {
            let selected_id = kind.available_id_at(state, state.config_selector_index);
            let selected_account = kind.available_account_at(state, state.config_selector_index);

            let mut account_for_session = None;
            if let Some(config_id) = selected_id {
                // Check if we're setting project default or creating a session
                if let Some(project_id) = state.setting_project_default_config.take() {
                    if let Some(project) = project_store.get_project_mut(project_id) {
                        kind.set_project_default(project, Some(config_id));
                    }
                    if let Err(e) = project_store.save() {
                        state.error_message = Some(format!("Failed to save: {}", e));
                    }
                    state.input_mode = InputMode::Normal;
                } else {
                    // Creating a session - hand the account back to the caller
                    account_for_session = selected_account;
                }
            }

            kind.clear_selector(state);
            return Ok(account_for_session);
        }
        _ => {}
    }
    Ok(None)
}

/// Config delete confirmation body
pub(crate) fn confirming_config_delete_key<C: AgentProfile>(
    state: &mut AppState,
    store: &mut ProfileStore<C>,
    project_store: &mut ProjectStore,
    kind: AgentKind,
    key: KeyEvent,
) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            // Confirm deletion
            if let Some(config_id) = state.pending_delete_agent_config.take() {
                // Validate config still exists
                if store.get(config_id).is_none() {
                    tracing::warn!(
                        config_id = %config_id,
                        "{} config no longer exists when confirming delete",
                        kind.label()
                    );
                    state.input_mode = InputMode::Normal;
                    return Ok(());
                }

                // Clear the default-config pointer from any projects using it
                let affected_projects: Vec<_> = project_store
                    .projects()
                    .filter(|p| kind.project_default(p) == Some(config_id))
                    .map(|p| p.id)
                    .collect();

                for project_id in affected_projects {
                    if let Some(project) = project_store.get_project_mut(project_id) {
                        kind.set_project_default(project, None);
                    }
                }

                // Save project store if any projects were affected
                if let Err(e) = project_store.save() {
                    tracing::error!("Failed to save project store: {}", e);
                }

                // Remove the config
                store.remove(config_id);

                // Save config store
                if let Err(e) = store.save() {
                    tracing::error!("Failed to save {} config store: {}", kind.label(), e);
                    state.error_message = Some(format!("Failed to save: {}", e));
                }

                tracing::info!("Deleted {} config: {}", kind.label(), config_id);

                // Adjust selection if needed
                let new_count = store.count();
                let selected = kind.view_selected_index(state);
                if selected >= new_count && new_count > 0 {
                    kind.set_view_selected_index(state, new_count - 1);
                } else if new_count == 0 {
                    kind.set_view_selected_index(state, 0);
                }
            }
            state.input_mode = InputMode::Normal;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            // Cancel deletion
            state.pending_delete_agent_config = None;
            state.input_mode = InputMode::Normal;
        }
        _ => {}
    }
    Ok(())
}

/// Configs management view body (normal mode)
pub(crate) fn configs_view_key<C: AgentProfile>(
    state: &mut AppState,
    store: &mut ProfileStore<C>,
    kind: AgentKind,
    key: KeyEvent,
) -> Result<()> {
    if key.kind != KeyEventKind::Press {
        return Ok(());
    }

    let config_count = store.count();

    match key.code {
        KeyCode::Esc => {
            state.navigate_back();
        }
        KeyCode::Down => {
            if config_count > 0 {
                state.select_next(config_count);
            }
        }
        KeyCode::Up => {
            if config_count > 0 {
                state.select_prev(config_count);
            }
        }
        KeyCode::Char('n') => {
            // Start creating a new config
            state.config_draft.reset();
            state.input_mode = kind.adding_name_mode();
        }
        KeyCode::Char('s') => {
            // Set selected config as default
            if config_count > 0 {
                let config_id = store
                    .configs_sorted()
                    .get(kind.view_selected_index(state))
                    .map(|c| c.id());
                if let Some(config_id) = config_id {
                    if store.set_default(config_id) {
                        if let Err(e) = store.save() {
                            tracing::error!("Failed to save {} config store: {}", kind.label(), e);
                            state.error_message = Some(format!("Failed to save: {}", e));
                        }
                    }
                }
            }
        }
        KeyCode::Char('d') => {
            // Prompt for confirmation before deleting
            if config_count > 0 {
                let config_id = store
                    .configs_sorted()
                    .get(kind.view_selected_index(state))
                    .map(|c| c.id());
                if let Some(config_id) = config_id {
                    state.pending_delete_agent_config = Some(config_id);
                    state.input_mode = kind.confirming_delete_mode();
                }
            }
        }
        _ => {}
    }
    Ok(())
}

// ========================================================================
// Config Path Completion Helpers
// ========================================================================

/// Update path completions for the config path input
pub(crate) fn update_config_path_completions(app: &mut App) {
    update_config_path_completions_state(&mut app.state);
}

fn update_config_path_completions_state(state: &mut AppState) {
    state.path_completions = crate::path_complete::get_completions(&state.config_draft.path);
    state.path_completion_index = 0;
    state.show_path_completions = !state.path_completions.is_empty();
}

/// Clear config path completion state
fn clear_config_path_completions(state: &mut AppState) {
    state.path_completions.clear();
    state.path_completion_index = 0;
    state.show_path_completions = false;
}

/// Apply the selected completion to the config path input field
fn apply_config_path_completion(state: &mut AppState) {
    if let Some(path) = state.path_completions.get(state.path_completion_index) {
        state.config_draft.path = crate::path_complete::path_to_input(path);
        update_config_path_completions_state(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_config::{ClaudeConfig, ClaudeConfigStore};
    use crate::codex_config::{CodexConfig, CodexConfigStore};
    use tempfile::TempDir;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    fn type_str(state: &mut AppState, store: &ClaudeConfigStore, kind: AgentKind, text: &str) {
        for c in text.chars() {
            adding_config_name_key(state, store, kind, press(KeyCode::Char(c))).unwrap();
        }
    }

    /// One store per agent kind, both backed by temp paths
    struct Fixture {
        _temp: TempDir,
        state: AppState,
        claude: ClaudeConfigStore,
        codex: CodexConfigStore,
        projects: ProjectStore,
    }

    fn fixture() -> Fixture {
        let temp = TempDir::new().unwrap();
        Fixture {
            state: AppState::default(),
            claude: ClaudeConfigStore::with_path(temp.path().join("claude_configs.json")),
            codex: CodexConfigStore::with_path(temp.path().join("codex_configs.json")),
            projects: ProjectStore::with_path(temp.path().join("projects.json")),
            _temp: temp,
        }
    }

    fn add_name_path_flow<C: AgentProfile>(
        state: &mut AppState,
        store: &mut ProfileStore<C>,
        kind: AgentKind,
        name: &str,
    ) {
        state.input_mode = kind.adding_name_mode();
        for c in name.chars() {
            adding_config_name_key(state, &*store, kind, press(KeyCode::Char(c))).unwrap();
        }
        adding_config_name_key(state, &*store, kind, press(KeyCode::Enter)).unwrap();
        assert_eq!(state.input_mode, kind.adding_path_mode());

        // Empty path = default dir; Enter creates the config
        adding_config_path_key(state, store, kind, press(KeyCode::Enter)).unwrap();
    }

    #[test]
    fn test_add_config_name_then_path_creates_config_both_kinds() {
        let mut f = fixture();

        add_name_path_flow(&mut f.state, &mut f.claude, AgentKind::Claude, "Work");
        assert_eq!(f.state.input_mode, InputMode::Normal);
        assert_eq!(f.claude.count(), 1);
        assert!(f.claude.find_by_name("Work").is_some());
        assert!(f.state.config_draft.name.is_empty());

        add_name_path_flow(&mut f.state, &mut f.codex, AgentKind::Codex, "Personal");
        assert_eq!(f.state.input_mode, InputMode::Normal);
        assert_eq!(f.codex.count(), 1);
        assert!(f.codex.find_by_name("Personal").is_some());
    }

    #[test]
    fn test_add_config_rejects_empty_and_duplicate_names() {
        let mut f = fixture();
        f.claude
            .add(ClaudeConfig::new("Work".to_string(), Some("/tmp/w".into())));

        // Empty name is rejected
        f.state.input_mode = AgentKind::Claude.adding_name_mode();
        adding_config_name_key(
            &mut f.state,
            &f.claude,
            AgentKind::Claude,
            press(KeyCode::Enter),
        )
        .unwrap();
        assert_eq!(f.state.input_mode, AgentKind::Claude.adding_name_mode());
        assert!(f.state.error_message.as_deref().unwrap().contains("empty"));

        // Duplicate name is rejected
        f.state.error_message = None;
        type_str(&mut f.state, &f.claude, AgentKind::Claude, "Work");
        adding_config_name_key(
            &mut f.state,
            &f.claude,
            AgentKind::Claude,
            press(KeyCode::Enter),
        )
        .unwrap();
        assert_eq!(f.state.input_mode, AgentKind::Claude.adding_name_mode());
        assert!(f
            .state
            .error_message
            .as_deref()
            .unwrap()
            .contains("already exists"));
    }

    #[test]
    fn test_add_config_path_rejects_nonexistent_dir_and_reused_path() {
        let mut f = fixture();

        // Nonexistent directory
        f.state.input_mode = AgentKind::Codex.adding_path_mode();
        f.state.config_draft.name = "A".to_string();
        f.state.config_draft.path = "/definitely/not/a/real/dir".to_string();
        adding_config_path_key(
            &mut f.state,
            &mut f.codex,
            AgentKind::Codex,
            press(KeyCode::Enter),
        )
        .unwrap();
        assert_eq!(f.codex.count(), 0);
        assert!(f
            .state
            .error_message
            .as_deref()
            .unwrap()
            .contains("does not exist"));
        // Draft is restored so the user can fix it
        assert_eq!(f.state.config_draft.name, "A");

        // Default dir (empty path) already used by another config
        f.codex.add(CodexConfig::new("Default".to_string(), None));
        f.state.error_message = None;
        f.state.config_draft.name = "B".to_string();
        f.state.config_draft.path.clear();
        adding_config_path_key(
            &mut f.state,
            &mut f.codex,
            AgentKind::Codex,
            press(KeyCode::Enter),
        )
        .unwrap();
        assert_eq!(f.codex.count(), 1);
        assert!(f
            .state
            .error_message
            .as_deref()
            .unwrap()
            .contains("already used"));
    }

    #[test]
    fn test_selector_cycles_and_sets_project_default_both_kinds() {
        let mut f = fixture();

        let project = Project::new("p".to_string(), "/tmp/p".into(), "main".to_string());
        let project_id = project.id;
        f.projects.add_project(project);

        // Claude flow
        let c1 = ClaudeConfig::new("Alpha".to_string(), Some("/tmp/a".into()));
        let c2 = ClaudeConfig::new("Beta".to_string(), Some("/tmp/b".into()));
        let c2_id = c2.id;
        f.state.available_claude_configs = vec![c1, c2];
        f.state.input_mode = InputMode::SelectingClaudeConfig;
        f.state.setting_project_default_config = Some(project_id);
        f.state.config_selector_index = 0;

        // Down cycles forward, wraps
        selecting_config_key(
            &mut f.state,
            &mut f.projects,
            AgentKind::Claude,
            press(KeyCode::Down),
        )
        .unwrap();
        assert_eq!(f.state.config_selector_index, 1);
        selecting_config_key(
            &mut f.state,
            &mut f.projects,
            AgentKind::Claude,
            press(KeyCode::Down),
        )
        .unwrap();
        assert_eq!(f.state.config_selector_index, 0);
        selecting_config_key(
            &mut f.state,
            &mut f.projects,
            AgentKind::Claude,
            press(KeyCode::Up),
        )
        .unwrap();
        assert_eq!(f.state.config_selector_index, 1);

        // Enter applies the selection as the project default
        let account = selecting_config_key(
            &mut f.state,
            &mut f.projects,
            AgentKind::Claude,
            press(KeyCode::Enter),
        )
        .unwrap();
        assert!(account.is_none(), "project-default flow creates no session");
        assert_eq!(f.state.input_mode, InputMode::Normal);
        assert!(f.state.available_claude_configs.is_empty());
        assert_eq!(
            f.projects
                .get_project(project_id)
                .unwrap()
                .default_claude_config,
            Some(c2_id)
        );

        // Codex flow through the same code path
        let x1 = CodexConfig::new("One".to_string(), Some("/tmp/1".into()));
        let x1_id = x1.id;
        f.state.available_codex_configs = vec![x1];
        f.state.input_mode = InputMode::SelectingCodexConfig;
        f.state.setting_project_default_config = Some(project_id);
        f.state.config_selector_index = 0;

        selecting_config_key(
            &mut f.state,
            &mut f.projects,
            AgentKind::Codex,
            press(KeyCode::Enter),
        )
        .unwrap();
        assert_eq!(
            f.projects
                .get_project(project_id)
                .unwrap()
                .default_codex_config,
            Some(x1_id)
        );
    }

    #[test]
    fn test_selector_enter_for_session_returns_account() {
        let mut f = fixture();
        let config = ClaudeConfig::new("Work".to_string(), Some("/tmp/work".into()));
        let config_id = config.id;
        f.state.available_claude_configs = vec![config];
        f.state.config_selector_index = 0;
        f.state.setting_project_default_config = None;

        let account = selecting_config_key(
            &mut f.state,
            &mut f.projects,
            AgentKind::Claude,
            press(KeyCode::Enter),
        )
        .unwrap()
        .expect("session flow returns the selected account");
        assert_eq!(account.id, config_id);
        assert_eq!(account.name, "Work");
        assert_eq!(
            account.dir.as_deref(),
            Some(std::path::Path::new("/tmp/work"))
        );
        assert!(f.state.available_claude_configs.is_empty());
    }

    #[test]
    fn test_selector_esc_cancels_and_clears_state() {
        let mut f = fixture();
        f.state.available_codex_configs = vec![CodexConfig::new("One".to_string(), None)];
        f.state.session_draft.name = "half-typed".to_string();
        f.state.setting_project_default_config = Some(uuid::Uuid::new_v4());
        f.state.input_mode = InputMode::SelectingCodexConfig;

        selecting_config_key(
            &mut f.state,
            &mut f.projects,
            AgentKind::Codex,
            press(KeyCode::Esc),
        )
        .unwrap();
        assert_eq!(f.state.input_mode, InputMode::Normal);
        assert!(f.state.available_codex_configs.is_empty());
        assert!(f.state.session_draft.name.is_empty());
        assert!(f.state.setting_project_default_config.is_none());
    }

    #[test]
    fn test_delete_confirm_removes_config_and_clears_project_default() {
        let mut f = fixture();

        let config = CodexConfig::new("Doomed".to_string(), Some("/tmp/d".into()));
        let config_id = config.id;
        f.codex.add(config);

        let mut project = Project::new("p".to_string(), "/tmp/p".into(), "main".to_string());
        project.default_codex_config = Some(config_id);
        let project_id = project.id;
        f.projects.add_project(project);

        f.state.pending_delete_agent_config = Some(config_id);
        f.state.input_mode = InputMode::ConfirmingCodexConfigDelete;

        confirming_config_delete_key(
            &mut f.state,
            &mut f.codex,
            &mut f.projects,
            AgentKind::Codex,
            press(KeyCode::Char('y')),
        )
        .unwrap();

        assert_eq!(f.state.input_mode, InputMode::Normal);
        assert_eq!(f.codex.count(), 0);
        assert!(f.state.pending_delete_agent_config.is_none());
        assert_eq!(
            f.projects
                .get_project(project_id)
                .unwrap()
                .default_codex_config,
            None
        );
    }

    #[test]
    fn test_delete_cancel_keeps_config_both_kinds() {
        let mut f = fixture();

        let claude = ClaudeConfig::new("Kept".to_string(), None);
        let claude_id = claude.id;
        f.claude.add(claude);

        f.state.pending_delete_agent_config = Some(claude_id);
        f.state.input_mode = InputMode::ConfirmingClaudeConfigDelete;
        confirming_config_delete_key(
            &mut f.state,
            &mut f.claude,
            &mut f.projects,
            AgentKind::Claude,
            press(KeyCode::Char('n')),
        )
        .unwrap();
        assert_eq!(f.state.input_mode, InputMode::Normal);
        assert_eq!(f.claude.count(), 1);
        assert!(f.state.pending_delete_agent_config.is_none());

        let codex = CodexConfig::new("Kept".to_string(), None);
        let codex_id = codex.id;
        f.codex.add(codex);

        f.state.pending_delete_agent_config = Some(codex_id);
        f.state.input_mode = InputMode::ConfirmingCodexConfigDelete;
        confirming_config_delete_key(
            &mut f.state,
            &mut f.codex,
            &mut f.projects,
            AgentKind::Codex,
            press(KeyCode::Esc),
        )
        .unwrap();
        assert_eq!(f.state.input_mode, InputMode::Normal);
        assert_eq!(f.codex.count(), 1);
    }

    #[test]
    fn test_delete_confirm_on_missing_config_resets_mode() {
        let mut f = fixture();
        f.state.pending_delete_agent_config = Some(uuid::Uuid::new_v4());
        f.state.input_mode = InputMode::ConfirmingClaudeConfigDelete;

        confirming_config_delete_key(
            &mut f.state,
            &mut f.claude,
            &mut f.projects,
            AgentKind::Claude,
            press(KeyCode::Char('y')),
        )
        .unwrap();
        assert_eq!(f.state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_configs_view_starts_add_and_delete_flows() {
        let mut f = fixture();
        f.state.view = crate::app::View::CodexConfigs;

        // 'n' starts the add flow with a clean draft
        f.state.config_draft.name = "stale".to_string();
        configs_view_key(
            &mut f.state,
            &mut f.codex,
            AgentKind::Codex,
            press(KeyCode::Char('n')),
        )
        .unwrap();
        assert_eq!(f.state.input_mode, InputMode::AddingCodexConfigName);
        assert!(f.state.config_draft.name.is_empty());

        // 'd' with a config selected opens the confirmation
        let config = CodexConfig::new("A".to_string(), None);
        let config_id = config.id;
        f.codex.add(config);
        f.state.input_mode = InputMode::Normal;
        configs_view_key(
            &mut f.state,
            &mut f.codex,
            AgentKind::Codex,
            press(KeyCode::Char('d')),
        )
        .unwrap();
        assert_eq!(f.state.input_mode, InputMode::ConfirmingCodexConfigDelete);
        assert_eq!(f.state.pending_delete_agent_config, Some(config_id));

        // 's' sets the selected config as default
        f.state.input_mode = InputMode::Normal;
        f.state.pending_delete_agent_config = None;
        configs_view_key(
            &mut f.state,
            &mut f.codex,
            AgentKind::Codex,
            press(KeyCode::Char('s')),
        )
        .unwrap();
        assert_eq!(f.codex.get_default_id(), Some(config_id));
    }
}
