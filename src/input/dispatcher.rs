//! Main input dispatch logic
//!
//! Routes keyboard events to appropriate handlers based on current mode.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{App, AppState, Focus, InputMode, ProjectsNav, SettingsNav, Tab};

/// Handle a key event by routing to the appropriate mode handler
pub fn handle_key_event(app: &mut App, key: KeyEvent) -> Result<()> {
    // Validate mode/focus consistency before dispatch
    validate_mode_focus_consistency(&mut app.state);

    // Handle help overlay first - it captures all keys when visible
    if app.state.show_help_overlay {
        match key.code {
            KeyCode::Char('?') | KeyCode::Esc if key.kind == KeyEventKind::Press => {
                app.state.show_help_overlay = false;
            }
            _ => {} // Ignore other keys while overlay is visible
        }
        return Ok(());
    }

    // Handle Ctrl+C specially
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        // In Session mode, fall through to forward Ctrl+C to PTY
        if app.state.input_mode != InputMode::Session {
            // Show warning in all other modes
            if key.kind == KeyEventKind::Press {
                app.state.error_message = Some("Ctrl+C disabled. Press q to quit.".to_string());
            }
            return Ok(());
        }
    }

    // Global keys exist in normal mode only, which is what lets every other
    // mode own `Tab` completely
    if globals_apply(app.state.input_mode) && handle_global_normal_key(app, key)? {
        return Ok(());
    }

    use super::agent_configs::{self, AgentKind};

    match app.state.input_mode {
        InputMode::Normal => app.handle_normal_mode_key(key),
        InputMode::Session => super::session_mode::handle_session_mode_key(app, key),
        InputMode::CreatingSession => {
            agent_configs::handle_creating_agent_session_key(app, key, AgentKind::Claude)
        }
        InputMode::CreatingShellSession => {
            super::text_input::handle_creating_shell_session_key(app, key)
        }
        InputMode::AddingProject => super::text_input::handle_adding_project_key(app, key),
        InputMode::AddingProjectName => super::text_input::handle_adding_project_name_key(app, key),
        InputMode::SelectingDefaultBase => {
            // The selector is opened from a project's settings level, which is
            // where its project ID comes from
            match app.state.projects_nav.project_id() {
                Some(project_id) => crate::wizards::worktree::handle_selecting_default_base_key(
                    app, key, project_id,
                ),
                None => Ok(()),
            }
        }
        InputMode::ConfirmingSessionDelete => {
            super::dialogs::handle_confirming_delete_key(app, key)
        }
        InputMode::ConfirmingBranchDelete => {
            super::dialogs::handle_confirming_branch_delete_key(app, key)
        }
        InputMode::ConfirmingProjectDelete => {
            super::dialogs::handle_confirming_project_delete_key(app, key)
        }
        InputMode::ConfirmingQuit => super::dialogs::handle_confirming_quit_key(app, key),
        InputMode::RenamingProject => super::text_input::handle_renaming_project_key(app, key),
        InputMode::WorktreeSelectBranch => match app.state.projects_nav.project_id() {
            Some(project_id) => {
                crate::wizards::worktree::handle_worktree_select_branch_key(app, key, project_id)
            }
            None => Ok(()),
        },
        InputMode::WorktreeSelectBase => match app.state.projects_nav.project_id() {
            Some(project_id) => {
                crate::wizards::worktree::handle_worktree_select_base_key(app, key, project_id)
            }
            None => Ok(()),
        },
        InputMode::WorktreeConfirm => match app.state.projects_nav.project_id() {
            Some(project_id) => {
                crate::wizards::worktree::handle_worktree_confirm_key(app, key, project_id)
            }
            None => Ok(()),
        },
        InputMode::AddingClaudeConfigName => {
            agent_configs::handle_adding_config_name_key(app, key, AgentKind::Claude)
        }
        InputMode::AddingClaudeConfigPath => {
            agent_configs::handle_adding_config_path_key(app, key, AgentKind::Claude)
        }
        InputMode::ConfirmingClaudeConfigDelete => {
            agent_configs::handle_confirming_config_delete_key(app, key, AgentKind::Claude)
        }
        InputMode::SelectingClaudeConfig => {
            agent_configs::handle_selecting_config_key(app, key, AgentKind::Claude)
        }
        InputMode::ConfirmingClaudeSettingsCopy => {
            super::dialogs::handle_confirming_claude_settings_copy_key(app, key)
        }
        InputMode::ConfirmingClaudeSettingsMigrate => {
            super::dialogs::handle_confirming_claude_settings_migrate_key(app, key)
        }
        InputMode::AddingCustomShortcutKey => {
            super::dialogs::handle_adding_custom_shortcut_key_key(app, key)
        }
        InputMode::AddingCustomShortcutName => {
            super::dialogs::handle_adding_custom_shortcut_name_key(app, key)
        }
        InputMode::AddingCustomShortcutCommand => {
            super::dialogs::handle_adding_custom_shortcut_command_key(app, key)
        }
        InputMode::AddingCustomShortcutAutoClose => {
            super::dialogs::handle_adding_custom_shortcut_auto_close_key(app, key)
        }
        InputMode::ConfirmingCustomShortcutDelete => {
            super::dialogs::handle_confirming_custom_shortcut_delete_key(app, key)
        }
        InputMode::SelectingAgentType => {
            super::text_input::handle_selecting_agent_type_key(app, key)
        }
        InputMode::CreatingCodexSession => {
            agent_configs::handle_creating_agent_session_key(app, key, AgentKind::Codex)
        }
        InputMode::AddingCodexConfigName => {
            agent_configs::handle_adding_config_name_key(app, key, AgentKind::Codex)
        }
        InputMode::AddingCodexConfigPath => {
            agent_configs::handle_adding_config_path_key(app, key, AgentKind::Codex)
        }
        InputMode::ConfirmingCodexConfigDelete => {
            agent_configs::handle_confirming_config_delete_key(app, key, AgentKind::Codex)
        }
        InputMode::SelectingCodexConfig => {
            agent_configs::handle_selecting_config_key(app, key, AgentKind::Codex)
        }
        InputMode::MovingToFolder => super::text_input::handle_moving_to_folder_key(app, key),
        InputMode::RenamingFolder => super::text_input::handle_renaming_folder_key(app, key),
        InputMode::ConfirmingFolderRemove => {
            super::dialogs::handle_confirming_folder_remove_key(app, key)
        }
    }
}

/// Whether the global keys (`Tab`, `q`, `?`, `Space`) apply in this mode
///
/// `Tab` is the load-bearing one: it switches panes *only* in normal mode,
/// which means every other input mode owns it completely - path autocomplete
/// (`text_input`), config-path autocomplete (`agent_configs`), and Yes/No
/// toggling in the Claude-settings dialogs all keep it. One rule, and it
/// covers every existing consumer.
fn globals_apply(mode: InputMode) -> bool {
    mode == InputMode::Normal
}

/// What a global key event should do, independent of app state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlobalIntent {
    /// A key release: act on nothing, so an action never runs twice.
    /// Panoptes no longer asks the terminal for release events, but a
    /// terminal may report them unasked
    Ignore,
    /// Tab / Shift+Tab: move pane focus
    CyclePane { forward: bool },
    /// `?`: open the help overlay
    ShowHelp,
    /// `q`: ask before quitting
    ConfirmQuit,
    /// `Space`: jump to the next session needing attention
    JumpToAttention,
    /// Not a global key: the mode handler owns it
    NotGlobal,
}

/// Classify a key event against the global keys
///
/// A `Repeat` acts like a `Press` so held keys keep working; only a `Release`
/// is dropped.
fn global_intent(key: &KeyEvent) -> GlobalIntent {
    if key.kind == KeyEventKind::Release {
        return GlobalIntent::Ignore;
    }
    match key.code {
        // kitty and Ghostty report Shift+Tab as Tab+SHIFT rather than
        // BackTab, so direction comes from the modifier, not the code alone
        KeyCode::Tab | KeyCode::BackTab => GlobalIntent::CyclePane {
            forward: key.code == KeyCode::Tab && !key.modifiers.contains(KeyModifiers::SHIFT),
        },
        KeyCode::Char('?') => GlobalIntent::ShowHelp,
        KeyCode::Char('q') => GlobalIntent::ConfirmQuit,
        KeyCode::Char(' ') => GlobalIntent::JumpToAttention,
        _ => GlobalIntent::NotGlobal,
    }
}

/// Keys that mean the same thing from every pane, in normal mode only
///
/// Returns whether the key was consumed.
fn handle_global_normal_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match global_intent(&key) {
        GlobalIntent::CyclePane { forward } => {
            // Cycling only makes sense while the panes are what is on screen
            if app.state.cycle_pane(forward) {
                app.sync_pane_focus();
            }
            Ok(true)
        }
        GlobalIntent::ShowHelp => {
            app.state.show_help_overlay = true;
            Ok(true)
        }
        // Quit from every pane and from session-view normal mode. In session
        // *mode* this never runs, so `q` keeps reaching the agent.
        GlobalIntent::ConfirmQuit => {
            app.state.input_mode = InputMode::ConfirmingQuit;
            Ok(true)
        }
        // Jump to the next session needing attention - except in the
        // Notifications section, where Space is the toggle
        GlobalIntent::JumpToAttention if !owns_space(&app.state) => {
            app.jump_to_next_attention()?;
            Ok(true)
        }
        GlobalIntent::Ignore | GlobalIntent::JumpToAttention | GlobalIntent::NotGlobal => Ok(false),
    }
}

/// Whether the focused pane has a stronger claim on `Space` than the jump
fn owns_space(state: &AppState) -> bool {
    state.focus == Focus::Panes(Tab::Settings) && state.settings_nav == SettingsNav::Notifications
}

/// Validate that the current [`InputMode`] can exist where the user is.
///
/// If an invalid combination is detected, reset to Normal mode to prevent
/// UI state corruption.
fn validate_mode_focus_consistency(state: &mut AppState) {
    let on = |tab: Tab| state.focus == Focus::Panes(tab);
    let is_valid = match state.input_mode {
        // Session mode only means anything with a session on screen
        InputMode::Session => state.focus == Focus::Session,

        // The worktree wizard carries a project ID taken from pane 1's
        // drill-down, so it can only exist at a level that has one
        InputMode::WorktreeSelectBranch
        | InputMode::WorktreeSelectBase
        | InputMode::WorktreeConfirm => {
            on(Tab::Projects) && matches!(state.projects_nav, ProjectsNav::Project(_))
        }

        // The default-base selector is reached only from per-project settings
        InputMode::SelectingDefaultBase => {
            on(Tab::Projects) && matches!(state.projects_nav, ProjectsNav::ProjectSettings(_))
        }

        // Renaming a project is likewise a per-project settings row
        InputMode::RenamingProject => {
            on(Tab::Projects) && matches!(state.projects_nav, ProjectsNav::ProjectSettings(_))
        }

        // Session deletion is valid wherever a session list lives: the session
        // view, pane 1's branch drill-down, and pane 2
        InputMode::ConfirmingSessionDelete => {
            state.focus == Focus::Session
                || on(Tab::Sessions)
                || (on(Tab::Projects) && matches!(state.projects_nav, ProjectsNav::Branch(_, _)))
        }

        // Folder organization only happens in pane 1's overview
        InputMode::MovingToFolder
        | InputMode::RenamingFolder
        | InputMode::ConfirmingFolderRemove => {
            on(Tab::Projects) && state.projects_nav == ProjectsNav::Overview
        }

        // Adding a project likewise starts at the overview
        InputMode::AddingProject | InputMode::AddingProjectName => {
            on(Tab::Projects) && state.projects_nav == ProjectsNav::Overview
        }

        // Deleting a project is an overview action; deleting a branch belongs
        // to the project and branch levels
        InputMode::ConfirmingProjectDelete => {
            on(Tab::Projects) && state.projects_nav == ProjectsNav::Overview
        }
        InputMode::ConfirmingBranchDelete | InputMode::ConfirmingClaudeSettingsMigrate => {
            on(Tab::Projects)
                && matches!(
                    state.projects_nav,
                    ProjectsNav::Project(_) | ProjectsNav::Branch(_, _)
                )
        }

        // Creating a session starts from pane 1's branch level
        InputMode::SelectingAgentType
        | InputMode::CreatingSession
        | InputMode::CreatingCodexSession
        | InputMode::CreatingShellSession => {
            on(Tab::Projects) && matches!(state.projects_nav, ProjectsNav::Branch(_, _))
        }

        // The shortcut and config editors live in pane 3
        InputMode::AddingCustomShortcutKey
        | InputMode::AddingCustomShortcutName
        | InputMode::AddingCustomShortcutCommand
        | InputMode::AddingCustomShortcutAutoClose
        | InputMode::ConfirmingCustomShortcutDelete => {
            on(Tab::Settings) && state.settings_nav == SettingsNav::Shortcuts
        }
        InputMode::AddingClaudeConfigName
        | InputMode::AddingClaudeConfigPath
        | InputMode::ConfirmingClaudeConfigDelete => {
            on(Tab::Settings) && state.settings_nav == SettingsNav::ClaudeConfigs
        }
        InputMode::AddingCodexConfigName
        | InputMode::AddingCodexConfigPath
        | InputMode::ConfirmingCodexConfigDelete => {
            on(Tab::Settings) && state.settings_nav == SettingsNav::CodexConfigs
        }

        // Reachable from a session being created (pane 1) and from a project's
        // settings (also pane 1), so pinning them further would be wrong
        InputMode::SelectingClaudeConfig | InputMode::SelectingCodexConfig => on(Tab::Projects),

        // Quit, the settings-copy offer, and normal mode are valid everywhere
        InputMode::Normal | InputMode::ConfirmingQuit | InputMode::ConfirmingClaudeSettingsCopy => {
            true
        }
    };

    if !is_valid {
        tracing::warn!(
            "Mode/focus mismatch detected: {:?} at {:?}/{:?}, resetting to Normal",
            state.input_mode,
            state.focus,
            state.projects_nav
        );
        state.input_mode = InputMode::Normal;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn key(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> KeyEvent {
        let mut key = KeyEvent::new(code, modifiers);
        key.kind = kind;
        key
    }

    /// The global keys, one event each, as iTerm2/kitty/Ghostty would send them
    const GLOBAL_KEYS: [(KeyCode, KeyModifiers); 5] = [
        (KeyCode::Tab, KeyModifiers::NONE),
        (KeyCode::Tab, KeyModifiers::SHIFT),
        (KeyCode::BackTab, KeyModifiers::SHIFT),
        (KeyCode::Char('?'), KeyModifiers::NONE),
        (KeyCode::Char('q'), KeyModifiers::NONE),
    ];

    #[test]
    fn test_a_release_is_a_noop_for_every_global_key() {
        for (code, modifiers) in GLOBAL_KEYS {
            let event = key(code, modifiers, KeyEventKind::Release);
            assert_eq!(
                global_intent(&event),
                GlobalIntent::Ignore,
                "release of {code:?}+{modifiers:?} must not re-run the action"
            );
        }
        let space = key(
            KeyCode::Char(' '),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        assert_eq!(global_intent(&space), GlobalIntent::Ignore);
    }

    /// Auto-repeat arrives as `Repeat` under the kitty protocol; a held Tab
    /// must keep cycling
    #[test]
    fn test_a_repeat_acts_like_a_press() {
        for kind in [KeyEventKind::Press, KeyEventKind::Repeat] {
            let event = key(KeyCode::Tab, KeyModifiers::NONE, kind);
            assert_eq!(
                global_intent(&event),
                GlobalIntent::CyclePane { forward: true },
                "for {kind:?}"
            );
        }
    }

    /// iTerm2 reports Shift+Tab as `BackTab`+SHIFT; kitty and Ghostty report
    /// it as `Tab`+SHIFT. Both must cycle backwards, and only an unshifted
    /// `Tab` cycles forwards.
    #[test]
    fn test_pane_cycle_direction_comes_from_the_modifier() {
        let cases = [
            (KeyCode::Tab, KeyModifiers::NONE, true),
            (KeyCode::Tab, KeyModifiers::SHIFT, false),
            (KeyCode::BackTab, KeyModifiers::SHIFT, false),
            // Belt and braces: a BackTab without the modifier is still Shift+Tab
            (KeyCode::BackTab, KeyModifiers::NONE, false),
        ];
        for (code, modifiers, forward) in cases {
            let event = key(code, modifiers, KeyEventKind::Press);
            assert_eq!(
                global_intent(&event),
                GlobalIntent::CyclePane { forward },
                "{code:?}+{modifiers:?}"
            );
        }
    }

    #[test]
    fn test_non_global_keys_fall_through_to_the_mode_handler() {
        for code in [KeyCode::Char('j'), KeyCode::Esc, KeyCode::Down] {
            let event = key(code, KeyModifiers::NONE, KeyEventKind::Press);
            assert_eq!(global_intent(&event), GlobalIntent::NotGlobal, "{code:?}");
        }
    }

    fn state_at(focus: Focus, mode: InputMode) -> AppState {
        AppState {
            focus,
            input_mode: mode,
            ..Default::default()
        }
    }

    #[test]
    fn test_session_mode_only_survives_with_a_session_on_screen() {
        let mut state = state_at(Focus::Session, InputMode::Session);
        validate_mode_focus_consistency(&mut state);
        assert_eq!(state.input_mode, InputMode::Session);

        for tab in Tab::ALL {
            let mut state = state_at(Focus::Panes(tab), InputMode::Session);
            validate_mode_focus_consistency(&mut state);
            assert_eq!(state.input_mode, InputMode::Normal, "pane {tab:?}");
        }
    }

    #[test]
    fn test_worktree_modes_need_pane_ones_project_level() {
        let project_id = Uuid::new_v4();
        for mode in [
            InputMode::WorktreeSelectBranch,
            InputMode::WorktreeSelectBase,
            InputMode::WorktreeConfirm,
        ] {
            let mut state = state_at(Focus::Panes(Tab::Projects), mode);
            state.projects_nav = ProjectsNav::Project(project_id);
            validate_mode_focus_consistency(&mut state);
            assert_eq!(state.input_mode, mode);

            // Anywhere else the project ID the handler needs is not there
            let mut state = state_at(Focus::Panes(Tab::Projects), mode);
            state.projects_nav = ProjectsNav::Overview;
            validate_mode_focus_consistency(&mut state);
            assert_eq!(state.input_mode, InputMode::Normal, "{mode:?} at overview");

            let mut state = state_at(Focus::Panes(Tab::Settings), mode);
            validate_mode_focus_consistency(&mut state);
            assert_eq!(state.input_mode, InputMode::Normal, "{mode:?} in settings");
        }
    }

    #[test]
    fn test_session_delete_is_valid_from_every_session_list() {
        let project_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();

        let valid = [
            (Focus::Session, ProjectsNav::Overview),
            (Focus::Panes(Tab::Sessions), ProjectsNav::Overview),
            (
                Focus::Panes(Tab::Projects),
                ProjectsNav::Branch(project_id, branch_id),
            ),
        ];
        for (focus, nav) in valid {
            let mut state = state_at(focus, InputMode::ConfirmingSessionDelete);
            state.projects_nav = nav;
            validate_mode_focus_consistency(&mut state);
            assert_eq!(
                state.input_mode,
                InputMode::ConfirmingSessionDelete,
                "{focus:?}/{nav:?}"
            );
        }

        // Not from the settings pane, which has no session list
        let mut state = state_at(
            Focus::Panes(Tab::Settings),
            InputMode::ConfirmingSessionDelete,
        );
        validate_mode_focus_consistency(&mut state);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_settings_editors_are_pinned_to_their_own_sections() {
        let cases = [
            (InputMode::AddingCustomShortcutKey, SettingsNav::Shortcuts),
            (
                InputMode::AddingClaudeConfigName,
                SettingsNav::ClaudeConfigs,
            ),
            (InputMode::AddingCodexConfigPath, SettingsNav::CodexConfigs),
        ];
        for (mode, section) in cases {
            let mut state = state_at(Focus::Panes(Tab::Settings), mode);
            state.settings_nav = section;
            validate_mode_focus_consistency(&mut state);
            assert_eq!(state.input_mode, mode, "{mode:?} in {section:?}");

            let mut state = state_at(Focus::Panes(Tab::Settings), mode);
            state.settings_nav = SettingsNav::About;
            validate_mode_focus_consistency(&mut state);
            assert_eq!(state.input_mode, InputMode::Normal, "{mode:?} in About");
        }
    }

    #[test]
    fn test_normal_and_quit_are_valid_anywhere() {
        for focus in [
            Focus::Session,
            Focus::Panes(Tab::Projects),
            Focus::Panes(Tab::Sessions),
            Focus::Panes(Tab::Settings),
        ] {
            for mode in [InputMode::Normal, InputMode::ConfirmingQuit] {
                let mut state = state_at(focus, mode);
                validate_mode_focus_consistency(&mut state);
                assert_eq!(state.input_mode, mode, "{mode:?} at {focus:?}");
            }
        }
    }

    /// Every mode has a decision recorded above; this catches a new variant
    /// that was added to `InputMode::ALL` but never routed
    #[test]
    fn test_every_mode_has_a_place_it_is_valid() {
        let project_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();

        let places: Vec<(Focus, ProjectsNav, SettingsNav)> = vec![
            (Focus::Session, ProjectsNav::Overview, SettingsNav::Sections),
            (
                Focus::Panes(Tab::Projects),
                ProjectsNav::Overview,
                SettingsNav::Sections,
            ),
            (
                Focus::Panes(Tab::Projects),
                ProjectsNav::Project(project_id),
                SettingsNav::Sections,
            ),
            (
                Focus::Panes(Tab::Projects),
                ProjectsNav::Branch(project_id, branch_id),
                SettingsNav::Sections,
            ),
            (
                Focus::Panes(Tab::Projects),
                ProjectsNav::ProjectSettings(project_id),
                SettingsNav::Sections,
            ),
            (
                Focus::Panes(Tab::Sessions),
                ProjectsNav::Overview,
                SettingsNav::Sections,
            ),
            (
                Focus::Panes(Tab::Settings),
                ProjectsNav::Overview,
                SettingsNav::Shortcuts,
            ),
            (
                Focus::Panes(Tab::Settings),
                ProjectsNav::Overview,
                SettingsNav::ClaudeConfigs,
            ),
            (
                Focus::Panes(Tab::Settings),
                ProjectsNav::Overview,
                SettingsNav::CodexConfigs,
            ),
        ];

        for &mode in &InputMode::ALL {
            let survives_somewhere = places.iter().any(|&(focus, nav, settings)| {
                let mut state = AppState {
                    focus,
                    projects_nav: nav,
                    settings_nav: settings,
                    input_mode: mode,
                    ..Default::default()
                };
                validate_mode_focus_consistency(&mut state);
                state.input_mode == mode
            });
            assert!(
                survives_somewhere,
                "{mode:?} is rejected everywhere, so it can never be entered"
            );
        }
    }

    /// Every mode that consumes `Tab` for its own purposes must keep it: the
    /// add-project prompt completes a path, it does not switch panes
    #[test]
    fn test_only_normal_mode_gives_tab_to_the_panes() {
        assert!(globals_apply(InputMode::Normal));

        for &mode in &InputMode::ALL {
            if mode == InputMode::Normal {
                continue;
            }
            assert!(
                !globals_apply(mode),
                "{mode:?} must own Tab, q, ? and Space itself"
            );
        }

        // The modes that would visibly break are worth naming
        for mode in [
            InputMode::AddingProject,
            InputMode::AddingClaudeConfigPath,
            InputMode::AddingCodexConfigPath,
            InputMode::MovingToFolder,
            InputMode::ConfirmingClaudeSettingsCopy,
            InputMode::ConfirmingClaudeSettingsMigrate,
            InputMode::AddingCustomShortcutAutoClose,
            InputMode::Session,
        ] {
            assert!(!globals_apply(mode), "{mode:?}");
        }
    }

    #[test]
    fn test_space_belongs_to_the_notifications_section_and_nowhere_else() {
        let mut state = AppState {
            focus: Focus::Panes(Tab::Settings),
            settings_nav: SettingsNav::Notifications,
            ..Default::default()
        };
        assert!(owns_space(&state));

        state.settings_nav = SettingsNav::About;
        assert!(!owns_space(&state));

        state.focus = Focus::Panes(Tab::Projects);
        state.settings_nav = SettingsNav::Notifications;
        assert!(!owns_space(&state), "another pane has focus");

        assert!(!owns_space(&AppState::default()));
    }
}
