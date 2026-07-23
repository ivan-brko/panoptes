//! Application state and main event loop
//!
//! This module contains the central application state and the main event loop
//! that ties together session management, hook handling, and terminal UI.

// Submodules
mod input_mode;
mod state;
mod view;

// Re-exports from submodules
pub use input_mode::InputMode;
pub use state::{
    cycle_next, cycle_prev, AppState, ClaudeSettingsCopyState, ClaudeSettingsMigrateState,
    FolderMoveTarget, HomepageFocus, SessionDraft, WorktreeWizardState,
};
pub use view::View;

// Re-exports from wizards (for backwards compatibility)
pub use crate::wizards::worktree::{BranchRef, BranchRefType, WorktreeCreationType};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyEvent, MouseEvent, MouseEventKind};

use crate::claude_config::ClaudeConfigStore;
use crate::codex_config::CodexConfigStore;
use crate::config::{Config, NotificationMethod};
use crate::hooks::{
    self, HookEventReceiver, HookEventSender, ServerHandle, ServerStatus, DEFAULT_CHANNEL_BUFFER,
};
use crate::input::agent_configs::AgentKind;
use crate::logging::{LogBuffer, LogFileInfo};
use crate::project::{BranchId, ProjectId, ProjectStore};
use crate::session::{mouse_event_to_bytes, SessionId, SessionManager, SessionType};
use crate::transcript::{TranscriptKind, TranscriptWatcher, WatchTarget};
use crate::tui::frame::{FrameConfig, FrameLayout};
use crate::tui::views::{
    render_agent_config_delete_dialog, render_agent_config_name_input_dialog,
    render_agent_config_path_input_dialog, render_agent_config_selector, render_agent_configs,
    render_agent_type_selector, render_branch_detail, render_claude_settings_copy_dialog,
    render_claude_settings_migrate_dialog, render_custom_shortcut_dialogs, render_help_overlay,
    render_loading_indicator, render_log_viewer, render_project_detail, render_projects_overview,
    render_session_view,
};
use crate::tui::Tui;
use crate::wizards::worktree::{
    filter_branch_refs, update_worktree_filtered_base_branches, update_worktree_filtered_branches,
    worktree_select_first_selectable,
};

// === Input Length Limits ===
// Maximum lengths for text input fields to prevent memory exhaustion from large pastes

/// Maximum length for project paths (generous for deeply nested paths)
pub const MAX_PROJECT_PATH_LEN: usize = 4096;
/// Maximum length for session names
pub const MAX_SESSION_NAME_LEN: usize = 256;
/// Maximum length for branch names
pub const MAX_BRANCH_NAME_LEN: usize = 256;
/// Maximum length for project names
pub const MAX_PROJECT_NAME_LEN: usize = 256;
/// Maximum length for custom shortcut names (paste only; typing is unbounded)
pub const MAX_SHORTCUT_NAME_LEN: usize = 256;
/// Maximum length for custom shortcut commands (paste only; typing is unbounded)
pub const MAX_SHORTCUT_COMMAND_LEN: usize = 4096;
/// Mouse-wheel line step used by local scroll handlers
const MOUSE_SCROLL_STEP: usize = 3;

/// Main application struct
pub struct App {
    /// Application configuration (used for project flows)
    pub(crate) config: Config,
    /// Application state
    pub(crate) state: AppState,
    /// Project store for project/branch persistence
    pub(crate) project_store: ProjectStore,
    /// Claude config store for managing Claude configurations
    pub(crate) claude_config_store: ClaudeConfigStore,
    /// Codex config store for managing Codex configurations
    pub(crate) codex_config_store: CodexConfigStore,
    /// Session manager
    pub(crate) sessions: SessionManager,
    /// Hook event receiver
    hook_rx: HookEventReceiver,
    /// Hook server handle (kept alive and used for dropped events tracking)
    hook_server: ServerHandle,
    /// Terminal UI
    pub(crate) tui: Tui,
    /// Log buffer for real-time log viewing
    pub(crate) log_buffer: Arc<LogBuffer>,
    /// Information about the current log file
    pub(crate) log_file_info: LogFileInfo,
    /// Whether verbose mouse diagnostics are enabled
    mouse_debug_enabled: bool,
    /// When the Codex rollout directory was last scanned for conversation IDs
    last_codex_id_scan: Option<Instant>,
    /// Background reader of agent transcripts
    transcripts: TranscriptWatcher,
    /// Which transcript each session is currently being followed at
    watched_transcripts: HashMap<SessionId, PathBuf>,
    /// When transcript watching was last reconciled against live sessions
    last_transcript_sync: Option<Instant>,
}

/// How often to reconcile transcript watching against the live session list
///
/// Only decides how quickly a *new* session starts being followed; the watcher
/// thread polls the files themselves far more often.
const TRANSCRIPT_SYNC_INTERVAL: Duration = Duration::from_secs(2);

/// The default `CLAUDE_CONFIG_DIR`, used when a session ran on the default account
fn default_claude_config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".claude")
}

/// How often to scan for Codex rollout files while any session lacks an ID
///
/// Codex writes its rollout within a moment of starting, so this resolves on the
/// first or second scan and then stops costing anything.
const CODEX_ID_SCAN_INTERVAL: Duration = Duration::from_secs(2);

/// The default `CODEX_HOME`, used when a session ran on the default account
fn default_codex_home() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(".codex")
}

impl App {
    /// Create a new application instance
    pub async fn new(log_buffer: Arc<LogBuffer>, log_file_info: LogFileInfo) -> Result<Self> {
        // Track any startup warnings to show as notifications
        let mut startup_warnings: Vec<String> = Vec::new();

        // Load config (or fall back to defaults if config.toml is corrupted)
        let (config, config_warning) = Config::load_with_status();
        if let Some(warning) = config_warning {
            startup_warnings.push(warning);
        }
        let mouse_debug_enabled = mouse_debug_enabled_from_env();

        // Load project store (or create empty if doesn't exist)
        let (project_store, corruption_warning) = ProjectStore::load_with_status();
        if let Some(warning) = corruption_warning {
            startup_warnings.push(warning);
        }
        tracing::debug!(
            "Loaded {} projects, {} branches",
            project_store.project_count(),
            project_store.branch_count()
        );

        // Load Claude config store (or create empty if doesn't exist)
        let (claude_config_store, claude_warning) = ClaudeConfigStore::load_with_status();
        if let Some(warning) = claude_warning {
            startup_warnings.push(warning);
        }
        tracing::debug!("Loaded {} claude configs", claude_config_store.count());

        // Load Codex config store (or create empty if doesn't exist)
        let (codex_config_store, codex_warning) = CodexConfigStore::load_with_status();
        if let Some(warning) = codex_warning {
            startup_warnings.push(warning);
        }
        tracing::debug!("Loaded {} codex configs", codex_config_store.count());

        // Create hook event channel with large buffer to avoid dropping events
        let (hook_tx, hook_rx): (HookEventSender, HookEventReceiver) =
            hooks::server::create_channel(DEFAULT_CHANNEL_BUFFER);

        // Start hook server
        let hook_server = hooks::server::start(config.hook_port, hook_tx).await?;
        tracing::debug!("Hook server started on port {}", hook_server.addr().port());

        // Create session manager
        let sessions = SessionManager::new(config.clone());

        // Start reading agent transcripts. Runs on its own thread: the reads
        // are incremental, but a burst of tool output can append a lot at once
        // and parsing that on the render thread would show as a stutter.
        let transcripts = TranscriptWatcher::spawn();
        transcripts.set_debug_log_dir(
            config
                .log_agent_events
                .then(|| crate::config::logs_dir().join("agent-events")),
        );

        // Create TUI
        let tui = Tui::new()?;

        let mut state = AppState::default();
        // Add any startup warnings as notifications
        for warning in startup_warnings {
            state.header_notifications.push(warning);
        }

        if mouse_debug_enabled {
            tracing::info!(
                target: "panoptes::mouse",
                log_file = %log_file_info.path.display(),
                "Mouse debug logging enabled (PANOPTES_MOUSE_DEBUG)"
            );
        }

        Ok(Self {
            config,
            state,
            project_store,
            claude_config_store,
            codex_config_store,
            sessions,
            hook_rx,
            hook_server,
            tui,
            log_buffer,
            log_file_info,
            mouse_debug_enabled,
            last_codex_id_scan: None,
            transcripts,
            watched_transcripts: HashMap::new(),
            last_transcript_sync: None,
        })
    }

    /// Run the main application loop
    pub async fn run(&mut self) -> Result<()> {
        // Enter TUI mode
        self.tui.enter()?;

        tracing::info!("Panoptes started. Press 'n' to create a session, 'q' to quit.");

        // Main event loop
        let result = self.event_loop().await;

        // Shutdown all sessions to prevent orphaned Claude Code processes
        self.sessions.shutdown_all();

        // Exit TUI mode (also done in Drop, but explicit is clearer)
        self.tui.exit()?;

        result
    }

    /// Main event loop
    ///
    /// Kept as a table of contents: input events are dispatched here, and every
    /// periodic concern lives in its own `tick_*` method (returning whether it
    /// changed anything worth rendering).
    async fn event_loop(&mut self) -> Result<()> {
        let tick_rate = Duration::from_millis(16); // ~60fps for smooth rendering

        // Always render on first frame
        self.state.needs_render = true;

        loop {
            self.tick_mouse_capture_safety_net();

            // Only render when something has changed
            if self.state.needs_render {
                self.render()?;
                self.state.needs_render = false;
            }

            // Poll for events with timeout
            if event::poll(tick_rate)? {
                match event::read()? {
                    Event::Key(key) => {
                        // Clear error message on any keypress
                        if self.state.error_message.is_some() {
                            self.state.error_message = None;
                        }
                        crate::input::dispatcher::handle_key_event(self, key)?;
                        self.state.needs_render = true;
                    }
                    Event::Paste(text) => {
                        if let Err(e) = self.handle_paste_event(&text) {
                            self.state.error_message = Some(format!("Paste failed: {}", e));
                        }
                        self.state.needs_render = true;
                    }
                    Event::Resize(_, _) => {
                        // Debounce resize events - record time and mark pending
                        // We still need to render (for UI layout), but PTY resize is deferred
                        self.state.last_resize = Some(Instant::now());
                        self.state.pending_resize = true;
                        self.state.needs_render = true;
                    }
                    Event::Mouse(mouse) => {
                        if self.handle_mouse_event_logged(mouse)? {
                            self.state.needs_render = true;
                        }
                    }
                    // Focus reporting is not enabled, so these do not arrive;
                    // the arms exist only to keep the match exhaustive
                    Event::FocusGained | Event::FocusLost => {}
                }
            }

            let mut dirty = false;
            dirty |= self.tick_resize_debounce()?;
            dirty |= self.process_hook_events();
            dirty |= self.tick_output_polling();
            dirty |= self.tick_crash_detection();
            // Give Codex sessions a resumable pointer as soon as their rollout
            // file appears; no-ops once every session has one
            self.resolve_pending_codex_session_ids();
            // Follow each session's transcript. For Codex this is the only
            // source of state at all; for Claude it adds usage figures the
            // hooks do not carry.
            self.sync_transcript_watchers();
            dirty |= self.process_transcript_events();
            dirty |= self.tick_state_timeouts();
            // Whatever the events above flagged, the session filling the screen
            // is not one the user needs pointing at
            self.acknowledge_visible_session();
            dirty |= self.tick_shell_state_notifications();
            dirty |= self.tick_auto_close();
            dirty |= self.tick_idle_suspension();
            dirty |= self.tick_exited_cleanup();
            dirty |= self.tick_dropped_events();
            dirty |= self.tick_server_health();
            if dirty {
                self.state.needs_render = true;
            }

            // Tick notifications (remove expired)
            self.state.header_notifications.tick();

            // Check if we should quit
            if self.state.should_quit {
                break;
            }
        }

        Ok(())
    }

    /// Safety net: Codex session mode requires mouse capture for wheel events.
    fn tick_mouse_capture_safety_net(&mut self) {
        if self.state.view == View::SessionView
            && self.state.input_mode == InputMode::Session
            && self
                .state
                .active_session
                .and_then(|session_id| self.sessions.get(session_id))
                .is_some_and(|session| session.info.session_type == SessionType::OpenAICodex)
        {
            self.tui.enable_mouse_capture();
        }
    }

    /// Handle a mouse event, logging scroll diagnostics when enabled
    ///
    /// Returns true if the event caused a state change requiring re-render.
    fn handle_mouse_event_logged(&mut self, mouse: MouseEvent) -> Result<bool> {
        let is_scroll_event = matches!(
            mouse.kind,
            MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight
        );

        if self.mouse_debug_enabled && is_scroll_event {
            tracing::info!(
                target: "panoptes::mouse",
                kind = ?mouse.kind,
                column = mouse.column,
                row = mouse.row,
                modifiers = ?mouse.modifiers,
                view = ?self.state.view,
                input_mode = ?self.state.input_mode,
                mouse_capture_enabled = self.tui.is_mouse_capture_enabled(),
                active_session = ?self.state.active_session,
                "Received mouse scroll event"
            );
        }

        let handled = self.handle_mouse_event(mouse)?;
        if self.mouse_debug_enabled && is_scroll_event {
            tracing::info!(
                target: "panoptes::mouse",
                handled,
                session_scroll_offset = self.state.session_scroll_offset,
                "Processed mouse scroll event"
            );
        }

        Ok(handled)
    }

    /// Process debounced resize: wait 50ms after last resize event before actually resizing.
    /// Resize ALL sessions to keep their PTYs in sync with terminal dimensions.
    fn tick_resize_debounce(&mut self) -> Result<bool> {
        if !self.state.pending_resize {
            return Ok(false);
        }
        let Some(last_resize) = self.state.last_resize else {
            return Ok(false);
        };
        if last_resize.elapsed() < Duration::from_millis(50) {
            return Ok(false);
        }

        self.state.pending_resize = false;
        let size = self.tui.size()?;

        // Use FrameLayout for consistent size calculation
        let frame_config = FrameConfig::default();
        let layout = FrameLayout::calculate(
            ratatui::prelude::Rect::new(0, 0, size.width, size.height),
            &frame_config,
        );
        let (rows, cols) = layout.pty_size();
        self.sessions.resize_all(cols, rows);
        Ok(true)
    }

    /// Poll session outputs - true if any session has new output.
    ///
    /// Freezes active session PTY reads while the user is scrolled up so the
    /// visible history doesn't shift under the cursor.
    fn tick_output_polling(&mut self) -> bool {
        let frozen_session = if self.state.session_scroll_offset > 0 {
            self.state.active_session.and_then(|session_id| {
                self.sessions.get(session_id).and_then(|session| {
                    (session.info.session_type == SessionType::OpenAICodex).then_some(session_id)
                })
            })
        } else {
            None
        };
        !self.sessions.poll_outputs_except(frozen_session).is_empty()
    }

    /// Check for dead sessions and notify about crashes
    fn tick_crash_detection(&mut self) -> bool {
        let crashed_sessions = self.sessions.check_alive();
        if crashed_sessions.is_empty() {
            return false;
        }
        // Notify about each crashed session
        for (_session_id, session_name, exit_reason) in &crashed_sessions {
            self.state.header_notifications.push(format!(
                "Session '{}' crashed: {}",
                session_name, exit_reason
            ));
        }
        true
    }

    /// Check for sessions stuck in Executing state too long
    fn tick_state_timeouts(&mut self) -> bool {
        self.sessions
            .check_state_timeouts(self.config.state_timeout_secs)
    }

    /// Check shell session states via foreground detection
    fn tick_shell_state_notifications(&mut self) -> bool {
        let shell_notifications = self.sessions.check_shell_states(self.state.active_session);
        if shell_notifications.is_empty() {
            return false;
        }

        // Send notifications for shell sessions that finished commands
        for session_id in shell_notifications {
            let is_active = self.state.active_session == Some(session_id);
            if !is_active {
                let session_name = self
                    .sessions
                    .get(session_id)
                    .map(|s| s.info.name.as_str())
                    .unwrap_or("Shell");
                SessionManager::send_notification(self.config.notification_method, session_name);
            }
        }
        true
    }

    /// Auto-close sessions whose commands have finished.
    ///
    /// Uses state_entered_at (when Waiting was entered) so the grace period
    /// starts after the command finishes, not after session creation.
    fn tick_auto_close(&mut self) -> bool {
        let auto_close_ids: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, session)| session.info.should_auto_close(3))
            .map(|(&id, _)| id)
            .collect();

        let mut closed_any = false;
        for session_id in auto_close_ids {
            // return_from_session must be called before destroy_session so
            // the session's project/branch context is still available for
            // navigation.
            if self.state.active_session == Some(session_id) {
                self.state.return_from_session(&self.sessions);
                self.tui.enable_mouse_capture();
                self.state.header_notifications.push("Session auto-closed");
            }
            let _ = self.sessions.destroy_session(session_id);
            closed_any = true;
        }
        closed_any
    }

    /// Suspend agent sessions left idle, reclaiming their memory.
    ///
    /// Runs before cleanup so a session suspended this tick is already
    /// excluded from the Exited path that would delete its record.
    fn tick_idle_suspension(&mut self) -> bool {
        let suspended = self
            .sessions
            .suspend_idle_sessions(self.config.suspend_after_secs, self.state.active_session);
        !suspended.is_empty()
    }

    /// Clean up old exited sessions to prevent memory growth, repairing a
    /// stale active-session reference left behind by the cleanup
    fn tick_exited_cleanup(&mut self) -> bool {
        let cleaned_up = self
            .sessions
            .cleanup_exited_sessions(self.config.exited_retention_secs);
        if cleaned_up == 0 {
            return false;
        }
        // Validate active_session reference - clear if pointing to cleaned session
        if let Some(session_id) = self.state.active_session {
            if self.sessions.get(session_id).is_none() {
                tracing::debug!(
                    session_id = %session_id,
                    "Clearing stale active_session reference after cleanup"
                );
                self.state.active_session = None;
            }
        }
        true
    }

    /// Check for dropped hook events and update warning
    fn tick_dropped_events(&mut self) -> bool {
        let dropped = self.hook_server.take_dropped_events();
        if dropped == 0 {
            return false;
        }
        self.state.dropped_events_count += dropped;
        tracing::warn!(
            "Dropped {} hook events due to channel overflow (total: {})",
            dropped,
            self.state.dropped_events_count
        );
        true
    }

    /// Check hook server health status
    fn tick_server_health(&mut self) -> bool {
        let Some(status) = self.hook_server.check_status() else {
            return false;
        };
        match status {
            ServerStatus::Error(msg) => {
                tracing::error!("Hook server error: {}", msg);
                self.state.header_notifications.set_persistent(
                    "Hook server stopped - session state updates unavailable".to_string(),
                );
                true
            }
            ServerStatus::Shutdown => {
                tracing::debug!("Hook server shut down normally");
                false
            }
            ServerStatus::Running => {
                // Normal operation, nothing to do
                false
            }
        }
    }

    /// Process pending hook events from the channel
    ///
    /// Processes all events sequentially to ensure no events are dropped.
    /// With subagents sharing a session_id, multiple important events (e.g.,
    /// Notification + PostToolUse) can arrive in a single batch and all must
    /// be applied to preserve correctness.
    ///
    /// Returns true if any events were processed
    fn process_hook_events(&mut self) -> bool {
        // Collect all pending events
        let mut events = Vec::new();

        while let Ok(event) = self.hook_rx.try_recv() {
            events.push(event);
        }

        if events.is_empty() {
            return false;
        }

        // Process all events sequentially
        for event in &events {
            tracing::debug!(
                "Hook event: session={}, event={}, tool={:?}",
                event.session_id,
                event.event,
                event.tool_name()
            );
            // handle_hook_event returns Some(session_id) if notification should be sent
            if let Some(session_id) = self.sessions.handle_hook_event(event) {
                self.notify_session_needs_attention(session_id);
            }
        }
        true
    }

    /// Handle paste event (for clipboard paste support)
    fn handle_paste_event(&mut self, text: &str) -> Result<()> {
        // Clean the pasted text (take first line, trim whitespace)
        let cleaned = text.lines().next().unwrap_or("").trim();

        // Session mode: send the raw pasted text to the PTY (wrapped in
        // brackets if the app enabled bracketed paste), not to a text field
        if self.state.input_mode == InputMode::Session {
            if let Some(session_id) = self.state.active_session {
                if self.sessions.is_suspended(session_id) && !self.wake_session(session_id)? {
                    return Ok(());
                }
                if let Some(session) = self.sessions.get_mut(session_id) {
                    session.write_paste(text)?;
                }
            }
            return Ok(());
        }

        let truncated_to = Self::paste_into_mode_field(&mut self.state, cleaned);

        // Keep completions/filters in step with the new field content,
        // mirroring what character input does in each mode
        match self.state.input_mode {
            InputMode::AddingProject => self.update_path_completions(),
            InputMode::AddingClaudeConfigPath | InputMode::AddingCodexConfigPath => {
                crate::input::agent_configs::update_config_path_completions(self)
            }
            InputMode::MovingToFolder => crate::input::text_input::update_folder_completions(self),
            InputMode::SelectingDefaultBase => {
                self.state.filtered_branch_refs = filter_branch_refs(
                    &self.state.available_branch_refs,
                    &self.state.new_branch_name,
                );
                self.select_default_base_branch();
            }
            InputMode::WorktreeSelectBranch => {
                update_worktree_filtered_branches(self);
                worktree_select_first_selectable(self);
                self.state.worktree_wizard.branch_validation_error = None;
            }
            InputMode::WorktreeSelectBase => {
                // Reset index when search changes (same as character input)
                self.state.worktree_wizard.base_list_index = 0;
                update_worktree_filtered_base_branches(self);
            }
            _ => {}
        }

        if let Some(limit) = truncated_to {
            self.state
                .header_notifications
                .push(format!("Pasted text truncated to {} characters", limit));
        }
        Ok(())
    }

    /// Paste text into whichever field the current input mode is editing
    ///
    /// Pure `AppState` manipulation so it can be tested without a TUI. Modes
    /// without a text field ignore the paste. Returns the byte limit the
    /// paste was truncated to, or `None` if nothing was cut off.
    fn paste_into_mode_field(state: &mut AppState, cleaned: &str) -> Option<usize> {
        use crate::input::agent_configs::{MAX_CONFIG_NAME_LEN, MAX_CONFIG_PATH_LEN};
        use crate::input::text_input::MAX_FOLDER_PATH_LEN;

        let (field, max): (&mut String, usize) = match state.input_mode {
            InputMode::AddingProject => (&mut state.new_project_path, MAX_PROJECT_PATH_LEN),
            InputMode::AddingProjectName | InputMode::RenamingProject => {
                (&mut state.new_project_name, MAX_PROJECT_NAME_LEN)
            }
            InputMode::CreatingSession
            | InputMode::CreatingCodexSession
            | InputMode::CreatingShellSession => {
                (&mut state.session_draft.name, MAX_SESSION_NAME_LEN)
            }
            InputMode::SelectingDefaultBase => (&mut state.new_branch_name, MAX_BRANCH_NAME_LEN),
            InputMode::WorktreeSelectBranch => {
                (&mut state.worktree_wizard.search_text, MAX_BRANCH_NAME_LEN)
            }
            InputMode::WorktreeSelectBase => (
                &mut state.worktree_wizard.base_search_text,
                MAX_BRANCH_NAME_LEN,
            ),
            InputMode::AddingClaudeConfigName | InputMode::AddingCodexConfigName => {
                (&mut state.config_draft.name, MAX_CONFIG_NAME_LEN)
            }
            InputMode::AddingClaudeConfigPath | InputMode::AddingCodexConfigPath => {
                (&mut state.config_draft.path, MAX_CONFIG_PATH_LEN)
            }
            InputMode::MovingToFolder | InputMode::RenamingFolder => {
                state.folder_error = None;
                (&mut state.folder_input, MAX_FOLDER_PATH_LEN)
            }
            InputMode::AddingCustomShortcutName => {
                (&mut state.new_shortcut_name, MAX_SHORTCUT_NAME_LEN)
            }
            InputMode::AddingCustomShortcutCommand => {
                state.shortcut_error = None;
                (&mut state.new_shortcut_command, MAX_SHORTCUT_COMMAND_LEN)
            }
            _ => return None,
        };

        paste_into(field, cleaned, max).then_some(max)
    }

    /// Truncate text to fit within max_len bytes when appended to existing string
    ///
    /// Returns (truncated_text, was_truncated). The cut lands on a char
    /// boundary, so multibyte input never exceeds the byte budget or panics.
    fn truncate_to_limit(text: &str, existing: &str, max_len: usize) -> (String, bool) {
        let available = max_len.saturating_sub(existing.len());
        if text.len() <= available {
            return (text.to_string(), false);
        }
        // Find the last char boundary that fits within `available` bytes
        let mut end = 0;
        for (i, c) in text.char_indices() {
            let next = i + c.len_utf8();
            if next > available {
                break;
            }
            end = next;
        }
        (text[..end].to_string(), true)
    }

    /// Handle a mouse event
    ///
    /// Returns true if the event caused a state change requiring re-render.
    fn handle_mouse_event(&mut self, mouse: MouseEvent) -> Result<bool> {
        // Only handle mouse events in session view with active session
        if self.state.view != View::SessionView {
            return Ok(false);
        }

        let Some(session_id) = self.state.active_session else {
            return Ok(false);
        };

        let Some(session) = self.sessions.get(session_id) else {
            return Ok(false);
        };
        let is_codex_session = session.info.session_type == SessionType::OpenAICodex;

        if is_codex_session && self.handle_mouse_wheel(session_id, mouse.kind, true) {
            return Ok(true);
        }

        if self.forward_mouse_event_to_pty_if_enabled(session_id, mouse, is_codex_session)? {
            return Ok(true);
        }

        if !is_codex_session && self.handle_mouse_wheel(session_id, mouse.kind, false) {
            return Ok(true);
        }

        Ok(false)
    }

    /// Scroll the session in response to a mouse wheel event
    ///
    /// Delegates to the shared scroll engine in [`crate::input::session_scroll`],
    /// which handles the Codex vterm-scrollback-with-fallback logic. Returns
    /// true if the event was a wheel event that was applied.
    fn handle_mouse_wheel(
        &mut self,
        session_id: SessionId,
        kind: MouseEventKind,
        is_codex_session: bool,
    ) -> bool {
        use crate::input::session_scroll;

        let (outcome, message) = match kind {
            MouseEventKind::ScrollUp => (
                session_scroll::scroll_up_by(self, session_id, MOUSE_SCROLL_STEP),
                "Handled mouse scroll up as Codex local scrollback",
            ),
            MouseEventKind::ScrollDown => (
                session_scroll::scroll_down_by(self, session_id, MOUSE_SCROLL_STEP),
                "Handled mouse scroll down as Codex local scrollback",
            ),
            _ => return false,
        };
        let Some(outcome) = outcome else {
            return false;
        };
        if is_codex_session {
            self.log_codex_mouse_scroll(session_id, outcome, message);
        }
        true
    }

    fn forward_mouse_event_to_pty_if_enabled(
        &mut self,
        session_id: SessionId,
        mouse: MouseEvent,
        is_codex_session: bool,
    ) -> Result<bool> {
        // A suspended session has no PTY to forward to. Deliberately does not
        // wake it either: scrolling back through a suspended session is the
        // point, and a stray click should not relaunch an agent.
        if self.sessions.is_suspended(session_id) {
            return Ok(false);
        }

        let mouse_enabled = self.sessions.get(session_id).is_some_and(|session| {
            session.vterm.mouse_protocol_mode() != vt100::MouseProtocolMode::None
        });
        if !should_forward_mouse_to_pty(self.state.input_mode, mouse_enabled) {
            return Ok(false);
        }

        if self.state.session_scroll_offset > 0 {
            self.state.session_scroll_offset = 0;
            if let Some(session) = self.sessions.get_mut(session_id) {
                session.vterm.scroll_to_bottom();
                if is_codex_session {
                    session.fallback_scroll_to_bottom();
                }
            }
        }

        let content_area = self.session_content_area()?;
        if let Some(bytes) = mouse_event_to_bytes(mouse, content_area) {
            if let Some(session) = self.sessions.get_mut(session_id) {
                session.write(&bytes)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn session_content_area(&self) -> Result<ratatui::prelude::Rect> {
        let terminal_size = self.tui.size()?;
        let frame_config = FrameConfig::default();
        let layout = FrameLayout::calculate(
            ratatui::prelude::Rect::new(0, 0, terminal_size.width, terminal_size.height),
            &frame_config,
        );
        Ok(layout.content)
    }

    fn log_codex_mouse_scroll(
        &self,
        session_id: SessionId,
        outcome: crate::input::session_scroll::ScrollOutcome,
        message: &'static str,
    ) {
        if self.mouse_debug_enabled {
            tracing::info!(
                target: "panoptes::mouse",
                session_id = %session_id,
                is_codex_session = true,
                requested_offset = outcome.requested_offset,
                vterm_offset = outcome.vterm_offset,
                fallback_offset = outcome.fallback_offset,
                scroll_offset = self.state.session_scroll_offset,
                message
            );
        }
    }

    // ========================================================================
    // Input Handlers (called by input::dispatcher)
    // ========================================================================

    /// Handle key in normal mode
    pub(crate) fn handle_normal_mode_key(&mut self, key: KeyEvent) -> Result<()> {
        use crate::input::normal;
        match self.state.view {
            View::ProjectsOverview => {
                normal::projects_overview::handle_projects_overview_key(self, key)
            }
            View::ProjectDetail(_) => normal::project_detail::handle_project_detail_key(self, key),
            View::BranchDetail(project_id, branch_id) => {
                normal::branch_detail::handle_branch_detail_key(self, key, project_id, branch_id)
            }
            View::SessionView => normal::session_view::handle_session_view_normal_key(self, key),
            View::LogViewer => normal::log_viewer::handle_log_viewer_key(self, key),
            View::ClaudeConfigs => crate::input::agent_configs::handle_configs_view_key(
                self,
                key,
                crate::input::agent_configs::AgentKind::Claude,
            ),
            View::CodexConfigs => crate::input::agent_configs::handle_configs_view_key(
                self,
                key,
                crate::input::agent_configs::AgentKind::Codex,
            ),
        }
    }

    /// Select the default base branch in the filtered list
    fn select_default_base_branch(&mut self) {
        // Find default base branch in the available (unfiltered) list first
        // This ensures we track the actual default even when filtered out
        if let Some(default_branch) = self
            .state
            .available_branch_refs
            .iter()
            .find(|b| b.is_default_base)
        {
            self.state.selected_base_branch = Some(default_branch.clone());
        } else if let Some(first) = self.state.available_branch_refs.first() {
            // If no default, use first available branch
            self.state.selected_base_branch = Some(first.clone());
        } else {
            self.state.selected_base_branch = None;
        }

        // Find index of default base branch in filtered list for UI highlighting
        if let Some(idx) = self
            .state
            .filtered_branch_refs
            .iter()
            .position(|b| b.is_default_base)
        {
            self.state.base_branch_selector_index = idx;
        } else if !self.state.filtered_branch_refs.is_empty() {
            // If no default in filtered list, select first item
            self.state.base_branch_selector_index = 0;
        }
    }

    /// Update path completions based on current input
    fn update_path_completions(&mut self) {
        let completions = crate::path_complete::get_completions(&self.state.new_project_path);
        self.state.path_completions = completions;
        self.state.path_completion_index = 0;
        self.state.show_path_completions = !self.state.path_completions.is_empty();
    }

    /// Start the new worktree creation wizard (Step 1)
    ///
    /// On failure the wizard is not entered; the caller surfaces the error to
    /// the user.
    pub(crate) fn start_worktree_wizard(&mut self, project_id: ProjectId) -> Result<()> {
        // Clear all wizard state
        self.state.worktree_wizard = WorktreeWizardState::default();
        self.state.fetch_error = None;

        let (project_name, repo_path) = {
            let project = self
                .project_store
                .get_project(project_id)
                .context("Project not found")?;
            (project.name.clone(), project.repo_path.clone())
        };
        self.state.worktree_wizard.project_name = project_name;

        // Get tracked branch names for this project
        let tracked_branches: std::collections::HashSet<String> = self
            .project_store
            .branches_for_project(project_id)
            .iter()
            .map(|b| b.name.clone())
            .collect();

        // Get existing git worktrees to detect untracked worktrees
        let git = crate::git::GitOps::open(&repo_path).context("Failed to open git repository")?;
        let git_worktree_branches: std::collections::HashSet<String> =
            match crate::git::worktree::list_worktrees(git.repository()) {
                Ok(worktrees) => worktrees.into_iter().filter_map(|wt| wt.branch).collect(),
                Err(e) => {
                    tracing::warn!("Failed to list git worktrees: {}", e);
                    std::collections::HashSet::new()
                }
            };
        drop(git);

        // Fetch remotes, list branch refs, and stamp on the wizard's
        // tracking flags (which the git layer knows nothing about)
        let refs = self
            .fetch_branch_refs(project_id)
            .context("Failed to list branches")?;
        self.state.worktree_wizard.all_branches = refs
            .into_iter()
            .map(|mut r| {
                r.is_already_tracked = tracked_branches.contains(&r.name);
                // Branch has untracked git worktree if git knows about it but Panoptes doesn't track it
                r.has_git_worktree =
                    !r.is_already_tracked && git_worktree_branches.contains(&r.name);
                r
            })
            .collect();
        self.state.worktree_wizard.filtered_branches =
            self.state.worktree_wizard.all_branches.clone();

        // Transition to step 1
        self.state.input_mode = InputMode::WorktreeSelectBranch;
        Ok(())
    }

    /// Start default base branch selection flow
    pub(crate) fn start_default_base_selection(&mut self, project_id: ProjectId) {
        self.state.new_branch_name.clear();
        self.state.base_branch_selector_index = 0;
        self.state.fetch_error = None;

        // Fetch branches (synchronous for now)
        self.fetch_and_populate_branch_refs(project_id);

        // Transition to selection mode
        self.state.input_mode = InputMode::SelectingDefaultBase;
    }

    /// Fetch remotes and populate branch refs for the default-base selector
    ///
    /// An empty list is a valid outcome (the selector renders empty); failures
    /// are logged and degrade to that.
    fn fetch_and_populate_branch_refs(&mut self, project_id: ProjectId) {
        match self.fetch_branch_refs(project_id) {
            Ok(refs) => {
                self.state.available_branch_refs = refs;
                self.state.filtered_branch_refs = self.state.available_branch_refs.clone();
                // Select the default base branch
                self.select_default_base_branch();
            }
            Err(e) => {
                tracing::error!("Failed to list branches: {:#}", e);
                self.state.available_branch_refs.clear();
                self.state.filtered_branch_refs.clear();
            }
        }
    }

    /// Fetch remotes and list all branch refs for a project
    ///
    /// The shared front half of the worktree wizard and the default-base
    /// selector: shows a loading indicator, fetches all remotes (a fetch
    /// failure only sets `fetch_error` - local refs still list fine offline),
    /// and maps the git layer's refs into [`BranchRef`]s.
    fn fetch_branch_refs(&mut self, project_id: ProjectId) -> Result<Vec<BranchRef>> {
        // Get project info and clone what we need
        let (repo_path, default_base_branch) = {
            let project = self
                .project_store
                .get_project(project_id)
                .context("Project not found")?;
            (
                project.repo_path.clone(),
                project.default_base_branch.clone(),
            )
        };

        let git = crate::git::GitOps::open(&repo_path).context("Failed to open git repository")?;

        // Try to fetch from remotes (may fail if offline, continue anyway)
        self.show_loading("Fetching branches from remotes...");
        if let Err(e) = git.fetch_all_remotes() {
            tracing::warn!("Failed to fetch remotes: {}", e);
            self.state.fetch_error = Some(format!("Fetch failed: {}", e));
        }
        self.clear_loading();

        // Get all branch refs
        let refs = git
            .list_all_branch_refs(default_base_branch.as_deref())
            .context("Failed to list branch refs")?;
        Ok(refs.into_iter().map(BranchRef::from).collect())
    }

    /// Create a worktree for a branch
    ///
    /// Returns the BranchId of the newly created branch on success.
    pub(crate) fn create_worktree(
        &mut self,
        project_id: ProjectId,
        branch_name: &str,
        create_branch: bool,
        base_ref: Option<&str>,
    ) -> Result<BranchId> {
        // Get project info and clone what we need
        let (repo_path, project_name) = {
            let Some(project) = self.project_store.get_project(project_id) else {
                anyhow::bail!("Project not found");
            };
            (project.repo_path.clone(), project.name.clone())
        };

        let git = crate::git::GitOps::open(&repo_path).context("Failed to open git repository")?;
        let worktree_path = crate::git::worktree::worktree_path_for_branch(
            &self.config.worktrees_dir,
            &project_name,
            branch_name,
        );

        // Show loading indicator for worktree creation
        self.show_loading(&format!("Creating worktree for '{}'...", branch_name));

        crate::git::worktree::create_worktree(
            git.repository(),
            branch_name,
            &worktree_path,
            create_branch,
            base_ref,
        )
        .with_context(|| format!("Failed to create worktree for '{}'", branch_name))?;

        self.clear_loading();

        let branch_id = self.register_worktree_branch(project_id, branch_name, worktree_path)?;
        tracing::info!("Created worktree for branch: {}", branch_name);

        Ok(branch_id)
    }

    /// Import an existing git worktree that is not tracked by Panoptes
    ///
    /// This finds the worktree path from git's worktree list and creates a Branch
    /// entry pointing to it, without modifying the worktree on disk.
    ///
    /// Returns the BranchId of the imported branch on success.
    pub(crate) fn import_existing_worktree(
        &mut self,
        project_id: ProjectId,
        branch_name: &str,
    ) -> Result<BranchId> {
        // Get project info
        let repo_path = {
            let Some(project) = self.project_store.get_project(project_id) else {
                anyhow::bail!("Project not found");
            };
            project.repo_path.clone()
        };

        let git = crate::git::GitOps::open(&repo_path).context("Failed to open git repository")?;

        // Find the worktree path for this branch from git
        let worktrees = crate::git::worktree::list_worktrees(git.repository())
            .context("Failed to list git worktrees")?;
        let worktree = worktrees
            .into_iter()
            .find(|wt| wt.branch.as_deref() == Some(branch_name))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No git worktree found for branch '{}'. It may have been removed.",
                    branch_name
                )
            })?;

        let branch_id = self.register_worktree_branch(project_id, branch_name, worktree.path)?;
        tracing::info!("Imported existing worktree for branch: {}", branch_name);

        Ok(branch_id)
    }

    /// Record a worktree as a branch of the project and persist it
    ///
    /// The shared tail of creating and importing worktrees: applies the
    /// project's `session_subdir` to get the effective working dir for
    /// sessions, adds the branch entry, and saves the store.
    fn register_worktree_branch(
        &mut self,
        project_id: ProjectId,
        branch_name: &str,
        worktree_path: PathBuf,
    ) -> Result<BranchId> {
        let session_subdir = self
            .project_store
            .get_project(project_id)
            .and_then(|p| p.session_subdir.clone());

        // Apply project's session_subdir to get effective working dir for sessions
        let effective_working_dir = match &session_subdir {
            Some(subdir) => worktree_path.join(subdir),
            None => worktree_path,
        };

        let branch = crate::project::Branch::new(
            project_id,
            branch_name.to_string(),
            effective_working_dir,
            false, // is_default
            true,  // is_worktree
        );
        let branch_id = branch.id;
        self.project_store.add_branch(branch);
        self.project_store
            .save()
            .context("Failed to save project store")?;

        Ok(branch_id)
    }

    /// Enter a session: the shared epilogue for every path that opens one
    ///
    /// Navigates to the session view (which auto-activates session mode),
    /// clears its attention flag and any lingering title notification, and
    /// resizes its PTY to the current viewport.
    ///
    /// Mouse capture is enabled unconditionally, for all session types: Codex
    /// sessions need wheel events delivered so they can be translated into
    /// local scrollback, and for every other type `handle_mouse_event` scrolls
    /// the vterm app-side (or forwards to the PTY when the child enabled a
    /// mouse protocol) - neither works without capture. Capture is only
    /// dropped when the user explicitly leaves session mode (Esc), so text
    /// selection remains possible there.
    pub(crate) fn activate_session(&mut self, session_id: SessionId) -> Result<()> {
        self.state.navigate_to_session(session_id);
        self.tui.enable_mouse_capture();
        self.sessions.acknowledge_attention(session_id);
        self.clear_title_notification();
        self.resize_active_session_pty()
    }

    /// Jump to the next session needing attention (oldest first)
    pub(crate) fn jump_to_next_attention(&mut self) -> Result<()> {
        let attention_sessions = self.sessions.sessions_needing_attention();

        if let Some(session) = attention_sessions.first() {
            let session_id = session.info.id;
            // Auto-enters session mode so the user is immediately active
            self.activate_session(session_id)?;
        } else {
            // Show a transient header notification instead of an error
            self.state
                .header_notifications
                .push("No session waiting for input");
        }
        Ok(())
    }

    /// Resize the active session's PTY to match the output viewport
    ///
    /// Uses FrameLayout to calculate the correct PTY dimensions, ensuring
    /// consistency with how the session view is rendered.
    pub(crate) fn resize_active_session_pty(&mut self) -> Result<()> {
        if let Some(session_id) = self.state.active_session {
            let size = self.tui.size()?;
            let frame_config = FrameConfig::default();
            let layout = FrameLayout::calculate(
                ratatui::prelude::Rect::new(0, 0, size.width, size.height),
                &frame_config,
            );
            let (rows, cols) = layout.pty_size();

            if let Some(session) = self.sessions.get_mut(session_id) {
                session.resize(cols, rows)?;
            }
        }
        Ok(())
    }

    /// Resolve conversation IDs for Codex sessions that do not have one yet
    ///
    /// Codex offers no flag to dictate its session ID, so unlike Claude Code it
    /// has to be discovered from the rollout file it writes shortly after
    /// starting. Until that resolves, the session has no pointer and would not
    /// be resumable after a crash.
    ///
    /// Throttled, and does no filesystem work at all once every Codex session
    /// has an ID - which is the steady state within seconds of starting one.
    pub(crate) fn resolve_pending_codex_session_ids(&mut self) {
        let pending = self.sessions.sessions_pending_codex_id();
        if pending.is_empty() {
            return;
        }

        if let Some(last) = self.last_codex_id_scan {
            if last.elapsed() < CODEX_ID_SCAN_INTERVAL {
                return;
            }
        }
        self.last_codex_id_scan = Some(Instant::now());

        // Conversations already spoken for, so that two Codex sessions sharing
        // a working directory cannot be handed the same rollout. Grows as this
        // sweep resolves sessions, which is why `pending` is ordered oldest
        // first: each session claims the rollout it actually created.
        let mut claimed = self.sessions.claimed_agent_session_ids();

        for (session_id, working_dir, created_at) in pending {
            let codex_home = self
                .sessions
                .get(session_id)
                .and_then(|session| session.info.codex_config_id)
                .and_then(|id| self.codex_config_store.get(id))
                .and_then(|config| config.codex_home.clone())
                .unwrap_or_else(default_codex_home);

            if let Some(agent_session_id) = crate::agent::codex::discover_session_id(
                &codex_home,
                &working_dir,
                created_at,
                &claimed,
            ) {
                claimed.insert(agent_session_id.clone());
                if self
                    .sessions
                    .set_agent_session_id(session_id, agent_session_id.clone())
                {
                    tracing::info!(
                        session_id = %session_id,
                        codex_session_id = %agent_session_id,
                        "Resolved Codex conversation ID; session is now resumable"
                    );
                }
            }
        }
    }

    /// Keep transcript watching in step with the live session list
    ///
    /// Sessions appear, get their conversation ID resolved, and go away, and
    /// each of those changes what should be followed. Reconciling on a timer
    /// rather than hooking every lifecycle path means a session cannot be
    /// created down some route that forgot to register it.
    pub(crate) fn sync_transcript_watchers(&mut self) {
        if let Some(last) = self.last_transcript_sync {
            if last.elapsed() < TRANSCRIPT_SYNC_INTERVAL {
                return;
            }
        }
        self.last_transcript_sync = Some(Instant::now());

        let live: Vec<(SessionId, SessionType, PathBuf, Option<String>)> = self
            .sessions
            .iter()
            .map(|(&id, session)| {
                (
                    id,
                    session.info.session_type,
                    session.info.working_dir.clone(),
                    session.info.agent_session_id.clone(),
                )
            })
            .collect();

        let mut still_here = Vec::new();

        for (session_id, session_type, working_dir, conversation_id) in live {
            still_here.push(session_id);

            // A shell has no conversation, and a session whose ID has not been
            // resolved yet has no file to point at. Both resolve themselves.
            let Some(conversation_id) = conversation_id else {
                continue;
            };
            let Some(target) =
                self.watch_target(session_id, session_type, &working_dir, &conversation_id)
            else {
                continue;
            };

            // Re-watching the same file would re-attach at EOF and skip
            // anything written since, so only act on a genuine change
            if self.watched_transcripts.get(&session_id) == Some(&target.path) {
                continue;
            }
            self.watched_transcripts
                .insert(session_id, target.path.clone());
            self.transcripts.watch(target);
        }

        self.watched_transcripts.retain(|session_id, _| {
            let kept = still_here.contains(session_id);
            if !kept {
                self.transcripts.forget(*session_id);
            }
            kept
        });
    }

    /// Work out which file to follow for a session, if there is one
    fn watch_target(
        &self,
        session_id: SessionId,
        session_type: SessionType,
        working_dir: &std::path::Path,
        conversation_id: &str,
    ) -> Option<WatchTarget> {
        let info = self.sessions.get(session_id).map(|s| &s.info);
        let claude_config_id = info.and_then(|i| i.claude_config_id);
        let codex_config_id = info.and_then(|i| i.codex_config_id);
        // A session that started its own conversation owns everything in the
        // file, including whatever it did in the seconds before Panoptes
        // managed to locate it
        let from_start = info.is_some_and(|i| !i.resumed_conversation);

        match session_type {
            SessionType::Shell => None,

            SessionType::ClaudeCode => {
                let config_dir = claude_config_id
                    .and_then(|id| self.claude_config_store.get(id))
                    .and_then(|config| config.config_dir.clone())
                    .unwrap_or_else(default_claude_config_dir);

                Some(WatchTarget {
                    session_id,
                    kind: TranscriptKind::Claude,
                    path: crate::transcript::claude::transcript_path(
                        &config_dir,
                        working_dir,
                        conversation_id,
                    ),
                    codex_sessions_dir: None,
                    conversation_id: None,
                    from_start,
                })
            }

            SessionType::OpenAICodex => {
                let codex_home = codex_config_id
                    .and_then(|id| self.codex_config_store.get(id))
                    .and_then(|config| config.codex_home.clone())
                    .unwrap_or_else(default_codex_home);

                // Codex names its rollouts by timestamp as well as ID, so
                // unlike Claude the path has to be found rather than derived
                let path = crate::agent::codex::rollout_path(&codex_home, conversation_id)?;

                Some(WatchTarget {
                    session_id,
                    kind: TranscriptKind::Codex,
                    path,
                    codex_sessions_dir: Some(codex_home.join("sessions")),
                    conversation_id: Some(conversation_id.to_string()),
                    from_start,
                })
            }
        }
    }

    /// Apply everything the transcript watcher has observed
    fn process_transcript_events(&mut self) -> bool {
        let events = self.transcripts.drain();
        if events.is_empty() {
            return false;
        }

        for (session_id, event) in events {
            if let Some(session_id) = self.sessions.apply_agent_event(session_id, event) {
                self.notify_session_needs_attention(session_id);
            }
        }
        true
    }

    /// Clear the attention flag on the session currently filling the screen
    ///
    /// The flag exists to point the user at a session that wants them. The one
    /// they are already watching cannot be that, so an event arriving while it
    /// is open must not leave it badged. Only the flag is cleared, never the
    /// state: a session blocked on a permission dialog still reads as
    /// `AwaitingApproval`, which is visible on screen anyway.
    fn acknowledge_visible_session(&mut self) {
        if self.state.view != View::SessionView {
            return;
        }
        if let Some(session_id) = self.state.active_session {
            self.sessions.acknowledge_attention(session_id);
        }
    }

    /// Sound the configured notification for a session, unless the user is
    /// already looking at it
    fn notify_session_needs_attention(&self, session_id: SessionId) {
        let is_active_session = self.state.active_session == Some(session_id);
        if !is_active_session {
            let session_name = self
                .sessions
                .get(session_id)
                .map(|s| s.info.name.as_str())
                .unwrap_or("Session");
            SessionManager::send_notification(self.config.notification_method, session_name);
        }
    }

    /// Reset the terminal title if title notifications are in use
    ///
    /// A "title" notification persists in the terminal's tab bar until
    /// something overwrites it, so every path that acknowledges a session's
    /// attention flag also calls this - otherwise the tab would keep saying a
    /// session needs attention after the user has already looked at it.
    pub(crate) fn clear_title_notification(&mut self) {
        if self.config.notification_method == NotificationMethod::Title {
            SessionManager::reset_terminal_title();
        }
    }

    /// Bring a session recovered from a previous run back to life
    ///
    /// Resolves the account configuration the session originally ran under, so
    /// a resumed session reattaches using the same Claude or Codex account
    /// rather than silently falling back to the default one.
    ///
    /// Returns `Ok(false)` when the ID does not name a recovered session, so
    /// callers can fall through to their normal open path.
    pub(crate) fn resume_recovered_session(
        &mut self,
        session_id: crate::session::SessionId,
    ) -> Result<bool> {
        let Some(info) = self.sessions.get_recovered(session_id) else {
            return Ok(false);
        };

        // Read the config IDs before borrowing the stores
        let claude_config_id = info.claude_config_id;
        let codex_config_id = info.codex_config_id;
        let (claude_config_dir, codex_home) =
            self.resolve_agent_config_dirs(session_id, claude_config_id, codex_config_id);

        let size = self.tui.size()?;
        let frame_config = FrameConfig::default();
        let layout = FrameLayout::calculate(
            ratatui::prelude::Rect::new(0, 0, size.width, size.height),
            &frame_config,
        );
        let (rows, cols) = layout.pty_size();

        self.sessions.resume_session(
            session_id,
            rows as usize,
            cols as usize,
            claude_config_dir,
            codex_home,
        )?;

        Ok(true)
    }

    /// Relaunch the agent of a session Panoptes suspended
    ///
    /// Returns `Ok(false)` when the session could not be brought back - its
    /// worktree deleted, its conversation gone - having told the user why. The
    /// caller must then not go on to write to a PTY that is not there.
    pub(crate) fn wake_session(&mut self, session_id: crate::session::SessionId) -> Result<bool> {
        let Some(session) = self.sessions.get(session_id) else {
            return Ok(false);
        };
        if session.info.state != crate::session::SessionState::Suspended {
            return Ok(true);
        }

        let claude_config_id = session.info.claude_config_id;
        let codex_config_id = session.info.codex_config_id;
        let (claude_config_dir, codex_home) =
            self.resolve_agent_config_dirs(session_id, claude_config_id, codex_config_id);

        let size = self.tui.size()?;
        let frame_config = FrameConfig::default();
        let layout = FrameLayout::calculate(
            ratatui::prelude::Rect::new(0, 0, size.width, size.height),
            &frame_config,
        );
        let (rows, cols) = layout.pty_size();

        match self.sessions.wake_session(
            session_id,
            rows as usize,
            cols as usize,
            claude_config_dir,
            codex_home,
        ) {
            Ok(()) => {
                self.state.needs_render = true;
                Ok(true)
            }
            Err(e) => {
                // Same treatment a recovered session gets: say why rather than
                // failing obscurely, and leave the session suspended
                tracing::warn!(session_id = %session_id, error = %e, "Failed to wake session");
                self.state
                    .header_notifications
                    .push(format!("Could not wake session: {}", e));
                self.state.needs_render = true;
                Ok(false)
            }
        }
    }

    /// Resolve the account config directories a session should run under
    ///
    /// A config the user has since deleted is worth saying out loud: the
    /// session will come back on the default account, which may be wrong.
    fn resolve_agent_config_dirs(
        &self,
        session_id: crate::session::SessionId,
        claude_config_id: Option<crate::claude_config::ClaudeConfigId>,
        codex_config_id: Option<crate::codex_config::CodexConfigId>,
    ) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
        let claude_config_dir = claude_config_id
            .and_then(|id| self.claude_config_store.get(id))
            .and_then(|config| config.config_dir.clone());
        let codex_home = codex_config_id
            .and_then(|id| self.codex_config_store.get(id))
            .and_then(|config| config.codex_home.clone());

        if claude_config_id.is_some() && claude_config_dir.is_none() {
            tracing::warn!(
                session_id = %session_id,
                "Claude config for this session no longer exists; resuming on the default account"
            );
        }
        if codex_config_id.is_some() && codex_home.is_none() {
            tracing::warn!(
                session_id = %session_id,
                "Codex config for this session no longer exists; resuming on the default account"
            );
        }

        (claude_config_dir, codex_home)
    }

    /// Refresh git state for all projects
    ///
    /// This checks if each worktree's working directory still exists and marks
    /// branches as stale if their directories are missing.
    pub(crate) fn refresh_all_git_state(&mut self) {
        let mut total_stale = 0;
        let project_ids: Vec<_> = self.project_store.projects().map(|p| p.id).collect();

        for project_id in project_ids {
            total_stale += self.project_store.refresh_branches(project_id);
        }

        if total_stale > 0 {
            self.state.header_notifications.push(format!(
                "{} worktree(s) marked stale - directories not found",
                total_stale
            ));
        }
    }

    /// Show a loading indicator with the given message and force a render
    ///
    /// This is used before blocking operations to provide visual feedback
    /// to the user that something is happening. A failed render is only
    /// logged: the loading overlay is cosmetic and the blocking operation
    /// should proceed regardless.
    pub(crate) fn show_loading(&mut self, message: &str) {
        self.state.loading_message = Some(message.to_string());
        if let Err(e) = self.render() {
            tracing::warn!("Failed to render loading indicator: {}", e);
        }
    }

    /// Clear the loading indicator
    pub(crate) fn clear_loading(&mut self) {
        self.state.loading_message = None;
    }

    /// Render the current state
    fn render(&mut self) -> Result<()> {
        let state = &self.state;
        let project_store = &self.project_store;
        let claude_config_store = &self.claude_config_store;
        let codex_config_store = &self.codex_config_store;
        let sessions = &self.sessions;
        let config = &self.config;
        let log_buffer = &self.log_buffer;
        let log_file_info = &self.log_file_info;

        self.tui.draw(|frame| {
            let area = frame.size();

            match state.view {
                View::ProjectsOverview => {
                    render_projects_overview(
                        frame,
                        area,
                        state,
                        project_store,
                        sessions,
                        config,
                        &state.header_notifications,
                    );
                }
                View::ProjectDetail(project_id) => {
                    render_project_detail(
                        frame,
                        area,
                        state,
                        project_id,
                        project_store,
                        sessions,
                        config,
                        &state.header_notifications,
                    );
                }
                View::BranchDetail(project_id, branch_id) => {
                    render_branch_detail(
                        frame,
                        area,
                        state,
                        project_id,
                        branch_id,
                        project_store,
                        sessions,
                        config,
                        &state.header_notifications,
                    );
                }
                View::SessionView => {
                    render_session_view(
                        frame,
                        area,
                        state,
                        sessions,
                        project_store,
                        config,
                        &state.header_notifications,
                    );
                }
                View::LogViewer => {
                    let attention_count = sessions.total_attention_count();
                    render_log_viewer(
                        frame,
                        area,
                        log_buffer,
                        log_file_info,
                        state.log_viewer_scroll,
                        state.log_viewer_auto_scroll,
                        &state.header_notifications,
                        attention_count,
                    );
                }
                View::ClaudeConfigs => {
                    let attention_count = sessions.total_attention_count();
                    render_agent_configs(
                        frame,
                        area,
                        AgentKind::Claude,
                        claude_config_store,
                        state.claude_configs_selected_index,
                        &state.header_notifications,
                        attention_count,
                    );
                }
                View::CodexConfigs => {
                    let attention_count = sessions.total_attention_count();
                    render_agent_configs(
                        frame,
                        area,
                        AgentKind::Codex,
                        codex_config_store,
                        state.codex_configs_selected_index,
                        &state.header_notifications,
                        attention_count,
                    );
                }
            }

            // Render the overlay belonging to the current input mode.
            // Exhaustive on purpose: adding a mode without deciding what it
            // renders here is a compile error.
            match state.input_mode {
                InputMode::AddingClaudeConfigName => {
                    render_agent_config_name_input_dialog(
                        frame,
                        area,
                        AgentKind::Claude,
                        &state.config_draft.name,
                    );
                }
                InputMode::AddingClaudeConfigPath => {
                    render_agent_config_path_input_dialog(
                        frame,
                        area,
                        AgentKind::Claude,
                        &state.config_draft.name,
                        &state.config_draft.path,
                        &state.path_completions,
                        state.path_completion_index,
                        state.show_path_completions,
                    );
                }
                InputMode::ConfirmingClaudeConfigDelete => {
                    if let Some(config) = state
                        .pending_delete_agent_config
                        .and_then(|id| claude_config_store.get(id))
                    {
                        // Find projects using this config
                        let affected_projects: Vec<String> = project_store
                            .projects()
                            .filter(|p| p.default_claude_config == Some(config.id))
                            .map(|p| p.name.clone())
                            .collect();
                        render_agent_config_delete_dialog(
                            frame,
                            area,
                            &config.name,
                            &affected_projects,
                        );
                    }
                }
                InputMode::SelectingClaudeConfig => {
                    render_agent_config_selector(
                        frame,
                        area,
                        AgentKind::Claude,
                        &state.available_claude_configs,
                        state.config_selector_index,
                        claude_config_store.get_default_id(),
                    );
                }
                InputMode::ConfirmingClaudeSettingsCopy => {
                    if let Some(ref copy_state) = state.pending_claude_settings_copy {
                        render_claude_settings_copy_dialog(frame, area, copy_state);
                    }
                }
                InputMode::ConfirmingClaudeSettingsMigrate => {
                    if let Some(ref migrate_state) = state.pending_claude_settings_migrate {
                        render_claude_settings_migrate_dialog(frame, area, migrate_state);
                    }
                }
                InputMode::SelectingAgentType => {
                    render_agent_type_selector(frame, area, state.agent_type_selector_index);
                }
                InputMode::AddingCodexConfigName => {
                    render_agent_config_name_input_dialog(
                        frame,
                        area,
                        AgentKind::Codex,
                        &state.config_draft.name,
                    );
                }
                InputMode::AddingCodexConfigPath => {
                    render_agent_config_path_input_dialog(
                        frame,
                        area,
                        AgentKind::Codex,
                        &state.config_draft.name,
                        &state.config_draft.path,
                        &state.path_completions,
                        state.path_completion_index,
                        state.show_path_completions,
                    );
                }
                InputMode::ConfirmingCodexConfigDelete => {
                    if let Some(config) = state
                        .pending_delete_agent_config
                        .and_then(|id| codex_config_store.get(id))
                    {
                        // Find projects using this config
                        let affected_projects: Vec<String> = project_store
                            .projects()
                            .filter(|p| p.default_codex_config == Some(config.id))
                            .map(|p| p.name.clone())
                            .collect();
                        render_agent_config_delete_dialog(
                            frame,
                            area,
                            &config.name,
                            &affected_projects,
                        );
                    }
                }
                InputMode::SelectingCodexConfig => {
                    render_agent_config_selector(
                        frame,
                        area,
                        AgentKind::Codex,
                        &state.available_codex_configs,
                        state.config_selector_index,
                        codex_config_store.get_default_id(),
                    );
                }
                InputMode::ManagingCustomShortcuts
                | InputMode::AddingCustomShortcutKey
                | InputMode::AddingCustomShortcutName
                | InputMode::AddingCustomShortcutCommand
                | InputMode::AddingCustomShortcutAutoClose
                | InputMode::ConfirmingCustomShortcutDelete => {
                    render_custom_shortcut_dialogs(frame, area, state, config);
                }
                // These modes draw no overlay from here: their dialogs are
                // rendered by the per-view renderers above (session-name
                // inputs, delete confirmations, worktree wizard, folder
                // dialogs), or they need no overlay at all (Normal, Session).
                InputMode::Normal
                | InputMode::Session
                | InputMode::CreatingSession
                | InputMode::CreatingShellSession
                | InputMode::CreatingCodexSession
                | InputMode::AddingProject
                | InputMode::AddingProjectName
                | InputMode::SelectingDefaultBase
                | InputMode::ConfirmingSessionDelete
                | InputMode::ConfirmingBranchDelete
                | InputMode::ConfirmingProjectDelete
                | InputMode::ConfirmingQuit
                | InputMode::RenamingProject
                | InputMode::MovingToFolder
                | InputMode::RenamingFolder
                | InputMode::ConfirmingFolderRemove
                | InputMode::WorktreeSelectBranch
                | InputMode::WorktreeSelectBase
                | InputMode::WorktreeConfirm => {}
            }

            // Render help overlay if active
            if state.show_help_overlay {
                render_help_overlay(frame, area, &state.view);
            }

            // Render loading overlay if a blocking operation is in progress
            if let Some(message) = &state.loading_message {
                render_loading_indicator(frame, area, message);
            }
        })?;

        Ok(())
    }

    /// Get a reference to the project store
    pub fn project_store(&self) -> &ProjectStore {
        &self.project_store
    }

    /// Get a mutable reference to the project store
    pub fn project_store_mut(&mut self) -> &mut ProjectStore {
        &mut self.project_store
    }
}

/// Append pasted text to a field, staying within its byte limit
///
/// Returns true when the paste had to be truncated, so the caller can tell
/// the user. The cut lands on a char boundary (see [`App::truncate_to_limit`]).
fn paste_into(field: &mut String, text: &str, max: usize) -> bool {
    let (truncated, was_truncated) = App::truncate_to_limit(text, field, max);
    field.push_str(&truncated);
    was_truncated
}

fn should_forward_mouse_to_pty(input_mode: InputMode, mouse_enabled: bool) -> bool {
    input_mode == InputMode::Session && mouse_enabled
}

fn mouse_debug_enabled_from_env() -> bool {
    std::env::var("PANOPTES_MOUSE_DEBUG").ok().is_some_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_input_mode_default() {
        assert_eq!(InputMode::default(), InputMode::Normal);
    }

    #[test]
    fn test_truncate_to_limit_ascii() {
        assert_eq!(App::truncate_to_limit("abc", "", 10), ("abc".into(), false));
        assert_eq!(
            App::truncate_to_limit("abcdef", "1234", 8),
            ("abcd".into(), true)
        );
        assert_eq!(App::truncate_to_limit("abc", "12345", 5), ("".into(), true));
    }

    #[test]
    fn test_truncate_to_limit_multibyte_respects_byte_budget() {
        // "é" is 2 bytes; with 3 bytes available only one full char fits
        let (truncated, was_truncated) = App::truncate_to_limit("ééé", "", 3);
        assert_eq!(truncated, "é");
        assert!(was_truncated);

        // The result appended to existing must never exceed max_len bytes
        let existing = "abc";
        let (truncated, _) = App::truncate_to_limit("wörld", existing, 7);
        assert!(existing.len() + truncated.len() <= 7);
        assert_eq!(truncated, "wör");
    }

    #[test]
    fn test_paste_appends_to_current_mode_field() {
        let mut state = AppState {
            input_mode: InputMode::CreatingShellSession,
            ..Default::default()
        };
        state.session_draft.name = "build ".to_string();

        let truncated = App::paste_into_mode_field(&mut state, "and test");
        assert_eq!(state.session_draft.name, "build and test");
        assert!(truncated.is_none());

        // Folder dialogs paste into folder_input and clear the stale error
        state.input_mode = InputMode::MovingToFolder;
        state.folder_error = Some("old error".to_string());
        let truncated = App::paste_into_mode_field(&mut state, "clients/acme");
        assert_eq!(state.folder_input, "clients/acme");
        assert!(state.folder_error.is_none());
        assert!(truncated.is_none());
    }

    #[test]
    fn test_paste_truncates_at_field_limit() {
        let mut state = AppState {
            input_mode: InputMode::CreatingCodexSession,
            ..Default::default()
        };
        let long = "x".repeat(MAX_SESSION_NAME_LEN + 50);

        let truncated = App::paste_into_mode_field(&mut state, &long);
        assert_eq!(truncated, Some(MAX_SESSION_NAME_LEN));
        assert_eq!(state.session_draft.name.len(), MAX_SESSION_NAME_LEN);

        // A second paste into the already-full field adds nothing
        let truncated = App::paste_into_mode_field(&mut state, "more");
        assert_eq!(truncated, Some(MAX_SESSION_NAME_LEN));
        assert_eq!(state.session_draft.name.len(), MAX_SESSION_NAME_LEN);
    }

    #[test]
    fn test_paste_ignored_in_modes_without_text_field() {
        let mut state = AppState {
            input_mode: InputMode::ConfirmingQuit,
            ..Default::default()
        };
        let truncated = App::paste_into_mode_field(&mut state, "ignored");
        assert!(truncated.is_none());
        assert!(state.session_draft.name.is_empty());
        assert!(state.new_project_path.is_empty());
    }

    #[test]
    fn test_return_from_session_uses_session_context() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config = Config {
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            ..Config::default()
        };
        let mut sessions = SessionManager::with_store(
            config,
            crate::session::SessionStore::with_path(temp_dir.path().join("sessions.json")),
        );

        let project_a = Uuid::new_v4();
        let branch_a = Uuid::new_v4();
        let project_b = Uuid::new_v4();
        let branch_b = Uuid::new_v4();

        // Create test sessions in different projects/branches
        let _session_a = sessions
            .insert_test_session("Session A", project_a, branch_a)
            .unwrap();
        let session_b = sessions
            .insert_test_session("Session B", project_b, branch_b)
            .unwrap();

        let mut state = AppState {
            view: View::LogViewer,
            ..Default::default()
        };

        // Start somewhere unrelated, navigate to session A
        state.navigate_to_session(_session_a);
        assert_eq!(state.session_return_view, Some(View::LogViewer));

        // Jump to session B (simulates Space key)
        state.active_session = Some(session_b);
        state.session_return_view = Some(View::SessionView); // This is what causes the bug

        // Return from session - should go to session B's branch detail, not where we came from
        state.return_from_session(&sessions);
        assert_eq!(state.view, View::BranchDetail(project_b, branch_b));
        assert!(state.active_session.is_none());
        assert!(state.session_return_view.is_none());
    }

    #[test]
    fn test_should_forward_mouse_to_pty() {
        assert!(should_forward_mouse_to_pty(InputMode::Session, true));
        assert!(!should_forward_mouse_to_pty(InputMode::Session, false));
        assert!(!should_forward_mouse_to_pty(InputMode::Normal, true));
    }
}
