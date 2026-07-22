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
    AppState, ClaudeSettingsCopyState, ClaudeSettingsMigrateState, FolderMoveTarget, HomepageFocus,
    WorktreeWizardState,
};
pub use view::View;

// Re-exports from wizards (for backwards compatibility)
pub use crate::wizards::worktree::{BranchRef, BranchRefType, WorktreeCreationType};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyEvent, MouseEvent, MouseEventKind};

use crate::claude_config::ClaudeConfigStore;
use crate::codex_config::CodexConfigStore;
use crate::config::Config;
use crate::hooks::{
    self, HookEventReceiver, HookEventSender, ServerHandle, ServerStatus, DEFAULT_CHANNEL_BUFFER,
};
use crate::logging::{LogBuffer, LogFileInfo};
use crate::project::{BranchId, ProjectId, ProjectStore};
use crate::session::{mouse_event_to_bytes, SessionId, SessionManager, SessionType};
use crate::transcript::{TranscriptKind, TranscriptWatcher, WatchTarget};
use crate::tui::frame::{FrameConfig, FrameLayout};
use crate::tui::views::{
    render_agent_type_selector, render_branch_detail, render_claude_configs,
    render_claude_settings_copy_dialog, render_claude_settings_migrate_dialog,
    render_codex_config_delete_dialog, render_codex_config_name_input_dialog,
    render_codex_config_path_input_dialog, render_codex_config_selector, render_codex_configs,
    render_config_delete_dialog, render_config_name_input_dialog, render_config_path_input_dialog,
    render_config_selector, render_custom_shortcut_dialogs, render_help_overlay,
    render_loading_indicator, render_log_viewer, render_project_detail, render_projects_overview,
    render_session_view, render_timeline,
};
use crate::tui::Tui;
use crate::wizards::worktree::{
    filter_branch_refs, update_worktree_filtered_branches, worktree_select_first_selectable,
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
        let config = Config::load()?;
        let mouse_debug_enabled = mouse_debug_enabled_from_env();

        // Track any startup warnings to show as notifications
        let mut startup_warnings: Vec<String> = Vec::new();

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
        let claude_config_store = ClaudeConfigStore::load().unwrap_or_else(|e| {
            tracing::warn!("Failed to load claude config store: {}, starting fresh", e);
            ClaudeConfigStore::new()
        });
        tracing::debug!("Loaded {} claude configs", claude_config_store.count());

        // Load Codex config store (or create empty if doesn't exist)
        let codex_config_store = CodexConfigStore::load().unwrap_or_else(|e| {
            tracing::warn!("Failed to load codex config store: {}, starting fresh", e);
            CodexConfigStore::new()
        });
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
    async fn event_loop(&mut self) -> Result<()> {
        let tick_rate = Duration::from_millis(16); // ~60fps for smooth rendering

        // Always render on first frame
        self.state.needs_render = true;

        loop {
            // Safety net: Codex session mode requires mouse capture for wheel events.
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

                        if handled {
                            self.state.needs_render = true;
                        }
                    }
                    // Focus reporting is not enabled, so these do not arrive;
                    // the arms exist only to keep the match exhaustive
                    Event::FocusGained | Event::FocusLost => {}
                }
            }

            // Process debounced resize: wait 50ms after last resize event before actually resizing
            // Resize ALL sessions to keep their PTYs in sync with terminal dimensions
            if self.state.pending_resize {
                if let Some(last_resize) = self.state.last_resize {
                    if last_resize.elapsed() >= Duration::from_millis(50) {
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
                        self.state.needs_render = true;
                    }
                }
            }

            // Process any pending hook events
            if self.process_hook_events() {
                self.state.needs_render = true;
            }

            // Poll session outputs - mark dirty if any session has new output.
            // Freeze active session PTY reads while user is scrolled up so the
            // visible history doesn't shift under the cursor.
            let frozen_session = if self.state.session_scroll_offset > 0 {
                self.state.active_session.and_then(|session_id| {
                    self.sessions.get(session_id).and_then(|session| {
                        (session.info.session_type == SessionType::OpenAICodex)
                            .then_some(session_id)
                    })
                })
            } else {
                None
            };
            if !self.sessions.poll_outputs_except(frozen_session).is_empty() {
                self.state.needs_render = true;
            }

            // Check for dead sessions and notify about crashes
            let crashed_sessions = self.sessions.check_alive();
            if !crashed_sessions.is_empty() {
                // Notify about each crashed session
                for (_session_id, session_name, exit_reason) in &crashed_sessions {
                    self.state.header_notifications.push(format!(
                        "Session '{}' crashed: {}",
                        session_name, exit_reason
                    ));
                }
                self.state.needs_render = true;
            }

            // Give Codex sessions a resumable pointer as soon as their rollout
            // file appears; no-ops once every session has one
            self.resolve_pending_codex_session_ids();

            // Follow each session's transcript. For Codex this is the only
            // source of state at all; for Claude it adds usage figures the
            // hooks do not carry.
            self.sync_transcript_watchers();
            if self.process_transcript_events() {
                self.state.needs_render = true;
            }

            // Check for sessions stuck in Executing state too long
            let had_timeout_changes = self
                .sessions
                .check_state_timeouts(self.config.state_timeout_secs);
            if had_timeout_changes {
                self.state.needs_render = true;
            }

            // Whatever the events above flagged, the session filling the screen
            // is not one the user needs pointing at
            self.acknowledge_visible_session();

            // Check shell session states via foreground detection
            let shell_notifications = self.sessions.check_shell_states(self.state.active_session);
            if !shell_notifications.is_empty() {
                self.state.needs_render = true;

                // Send notifications for shell sessions that finished commands
                for session_id in shell_notifications {
                    let is_active = self.state.active_session == Some(session_id);
                    if !is_active {
                        let session_name = self
                            .sessions
                            .get(session_id)
                            .map(|s| s.info.name.as_str())
                            .unwrap_or("Shell");
                        SessionManager::send_notification(
                            &self.config.notification_method,
                            session_name,
                        );
                    }
                }
            }

            // Auto-close sessions whose commands have finished.
            // Uses state_entered_at (when Waiting was entered) so the grace period
            // starts after the command finishes, not after session creation.
            {
                let auto_close_ids: Vec<SessionId> = self
                    .sessions
                    .iter()
                    .filter(|(_, session)| session.info.should_auto_close(3))
                    .map(|(&id, _)| id)
                    .collect();

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
                    self.state.needs_render = true;
                }
            }

            // Suspend agent sessions left idle, reclaiming their memory. Runs
            // before cleanup so a session suspended this tick is already
            // excluded from the Exited path that would delete its record.
            let suspended = self
                .sessions
                .suspend_idle_sessions(self.config.suspend_after_secs, self.state.active_session);
            if !suspended.is_empty() {
                self.state.needs_render = true;
            }

            // Clean up old exited sessions to prevent memory growth
            let cleaned_up = self
                .sessions
                .cleanup_exited_sessions(self.config.exited_retention_secs);
            if cleaned_up > 0 {
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
                self.state.needs_render = true;
            }

            // Check for dropped hook events and update warning
            let dropped = self.hook_server.take_dropped_events();
            if dropped > 0 {
                self.state.dropped_events_count += dropped;
                tracing::warn!(
                    "Dropped {} hook events due to channel overflow (total: {})",
                    dropped,
                    self.state.dropped_events_count
                );
                self.state.needs_render = true;
            }

            // Check hook server health status
            if let Some(status) = self.hook_server.check_status() {
                match status {
                    ServerStatus::Error(msg) => {
                        tracing::error!("Hook server error: {}", msg);
                        self.state.header_notifications.set_persistent(
                            "Hook server stopped - session state updates unavailable".to_string(),
                        );
                        self.state.needs_render = true;
                    }
                    ServerStatus::Shutdown => {
                        tracing::debug!("Hook server shut down normally");
                    }
                    ServerStatus::Running => {
                        // Normal operation, nothing to do
                    }
                }
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

        match self.state.input_mode {
            InputMode::AddingProject => {
                let (truncated, was_truncated) = Self::truncate_to_limit(
                    cleaned,
                    &self.state.new_project_path,
                    MAX_PROJECT_PATH_LEN,
                );
                self.state.new_project_path.push_str(&truncated);
                self.update_path_completions();
                if was_truncated {
                    self.state.header_notifications.push(format!(
                        "Pasted text truncated to {} characters",
                        MAX_PROJECT_PATH_LEN
                    ));
                }
            }
            InputMode::AddingProjectName | InputMode::RenamingProject => {
                let (truncated, was_truncated) = Self::truncate_to_limit(
                    cleaned,
                    &self.state.new_project_name,
                    MAX_PROJECT_NAME_LEN,
                );
                self.state.new_project_name.push_str(&truncated);
                if was_truncated {
                    self.state.header_notifications.push(format!(
                        "Pasted text truncated to {} characters",
                        MAX_PROJECT_NAME_LEN
                    ));
                }
            }
            InputMode::CreatingSession | InputMode::CreatingCodexSession => {
                let (truncated, was_truncated) = Self::truncate_to_limit(
                    cleaned,
                    &self.state.new_session_name,
                    MAX_SESSION_NAME_LEN,
                );
                self.state.new_session_name.push_str(&truncated);
                if was_truncated {
                    self.state.header_notifications.push(format!(
                        "Pasted text truncated to {} characters",
                        MAX_SESSION_NAME_LEN
                    ));
                }
            }
            InputMode::CreatingWorktree | InputMode::SelectingDefaultBase => {
                let (truncated, was_truncated) = Self::truncate_to_limit(
                    cleaned,
                    &self.state.new_branch_name,
                    MAX_BRANCH_NAME_LEN,
                );
                self.state.new_branch_name.push_str(&truncated);
                // Update filtered branches
                self.state.filtered_branch_refs = filter_branch_refs(
                    &self.state.available_branch_refs,
                    &self.state.new_branch_name,
                );
                self.select_default_base_branch();
                if was_truncated {
                    self.state.header_notifications.push(format!(
                        "Pasted text truncated to {} characters",
                        MAX_BRANCH_NAME_LEN
                    ));
                }
            }
            InputMode::WorktreeSelectBranch => {
                let (truncated, was_truncated) = Self::truncate_to_limit(
                    cleaned,
                    &self.state.worktree_wizard.search_text,
                    MAX_BRANCH_NAME_LEN,
                );
                self.state.worktree_wizard.search_text.push_str(&truncated);
                // Update filtered branches and selection (same as character input)
                update_worktree_filtered_branches(self);
                worktree_select_first_selectable(self);
                self.state.worktree_wizard.branch_validation_error = None;
                if was_truncated {
                    self.state.header_notifications.push(format!(
                        "Pasted text truncated to {} characters",
                        MAX_BRANCH_NAME_LEN
                    ));
                }
            }
            InputMode::WorktreeSelectBase => {
                let (truncated, was_truncated) = Self::truncate_to_limit(
                    cleaned,
                    &self.state.worktree_wizard.base_search_text,
                    MAX_BRANCH_NAME_LEN,
                );
                self.state
                    .worktree_wizard
                    .base_search_text
                    .push_str(&truncated);
                // Reset index when search changes (same as character input)
                self.state.worktree_wizard.base_list_index = 0;
                if was_truncated {
                    self.state.header_notifications.push(format!(
                        "Pasted text truncated to {} characters",
                        MAX_BRANCH_NAME_LEN
                    ));
                }
            }
            InputMode::Session => {
                // Send pasted text to PTY (wrapped in brackets if app enabled it)
                if let Some(session_id) = self.state.active_session {
                    if self.sessions.is_suspended(session_id) && !self.wake_session(session_id)? {
                        return Ok(());
                    }
                    if let Some(session) = self.sessions.get_mut(session_id) {
                        session.write_paste(text)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Truncate text to fit within max_len when appended to existing string
    ///
    /// Returns (truncated_text, was_truncated)
    fn truncate_to_limit(text: &str, existing: &str, max_len: usize) -> (String, bool) {
        let available = max_len.saturating_sub(existing.len());
        if text.len() <= available {
            (text.to_string(), false)
        } else if available == 0 {
            (String::new(), true)
        } else {
            // Truncate at char boundary
            let truncated: String = text.chars().take(available).collect();
            (truncated, true)
        }
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

        let is_codex_session = self
            .sessions
            .get(session_id)
            .is_some_and(|session| session.info.session_type == SessionType::OpenAICodex);
        if self.sessions.get(session_id).is_none() {
            return Ok(false);
        }

        if is_codex_session && self.handle_codex_mouse_wheel(session_id, mouse.kind)? {
            return Ok(true);
        }

        if self.forward_mouse_event_to_pty_if_enabled(session_id, mouse, is_codex_session)? {
            return Ok(true);
        }

        if !is_codex_session && self.handle_non_codex_mouse_wheel(session_id, mouse.kind) {
            return Ok(true);
        }

        Ok(false)
    }

    fn handle_codex_mouse_wheel(
        &mut self,
        session_id: SessionId,
        kind: MouseEventKind,
    ) -> Result<bool> {
        match kind {
            MouseEventKind::ScrollUp => {
                let viewport_height = self.session_viewport_height();
                let (requested, new_vterm, fallback_offset) = {
                    let Some(session) = self.sessions.get_mut(session_id) else {
                        return Ok(false);
                    };

                    let current_vterm = session.vterm.scrollback_offset();
                    let requested = current_vterm.saturating_add(MOUSE_SCROLL_STEP);
                    session.vterm.set_scrollback(requested);
                    let new_vterm = session.vterm.scrollback_offset();
                    let vterm_advanced = new_vterm > current_vterm;

                    if new_vterm > 0 && vterm_advanced {
                        session.fallback_scroll_to_bottom();
                        self.state.session_scroll_offset = new_vterm;
                    } else {
                        // vterm scrollback may get "stuck" at a shallow offset (e.g. 1).
                        // Force fallback rendering path for deeper history scrolling.
                        session.vterm.scroll_to_bottom();
                        session
                            .fallback_scroll_up_with_viewport(MOUSE_SCROLL_STEP, viewport_height);
                        self.state.session_scroll_offset = session.fallback_scroll_offset();
                    }
                    (requested, new_vterm, session.fallback_scroll_offset())
                };
                self.log_codex_mouse_scroll(
                    session_id,
                    requested,
                    new_vterm,
                    fallback_offset,
                    "Handled mouse scroll up as Codex local scrollback",
                );
                Ok(true)
            }
            MouseEventKind::ScrollDown => {
                let Some(current_vterm) = self
                    .sessions
                    .get(session_id)
                    .map(|session| session.vterm.scrollback_offset())
                else {
                    return Ok(false);
                };

                if current_vterm > 0 {
                    let (requested, new_vterm, fallback_offset) = {
                        let Some(session) = self.sessions.get_mut(session_id) else {
                            return Ok(false);
                        };
                        let requested = current_vterm.saturating_sub(MOUSE_SCROLL_STEP);
                        session.vterm.set_scrollback(requested);
                        let new_vterm = session.vterm.scrollback_offset();
                        if new_vterm == 0 {
                            session.fallback_scroll_to_bottom();
                        }
                        self.state.session_scroll_offset = new_vterm;
                        (requested, new_vterm, session.fallback_scroll_offset())
                    };
                    self.log_codex_mouse_scroll(
                        session_id,
                        requested,
                        new_vterm,
                        fallback_offset,
                        "Handled mouse scroll down as Codex local scrollback",
                    );
                    return Ok(true);
                }

                let fallback_offset = {
                    let Some(session) = self.sessions.get_mut(session_id) else {
                        return Ok(false);
                    };
                    session.fallback_scroll_down(MOUSE_SCROLL_STEP);
                    self.state.session_scroll_offset = session.fallback_scroll_offset();
                    session.fallback_scroll_offset()
                };
                self.log_codex_mouse_scroll(
                    session_id,
                    0,
                    current_vterm,
                    fallback_offset,
                    "Handled mouse scroll down as Codex local scrollback",
                );
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn handle_non_codex_mouse_wheel(
        &mut self,
        session_id: SessionId,
        kind: MouseEventKind,
    ) -> bool {
        let Some(session) = self.sessions.get_mut(session_id) else {
            return false;
        };

        match kind {
            MouseEventKind::ScrollUp => {
                let max_scroll = session.vterm.max_scrollback();
                self.state.session_scroll_offset = self
                    .state
                    .session_scroll_offset
                    .saturating_add(MOUSE_SCROLL_STEP)
                    .min(max_scroll);
                session
                    .vterm
                    .set_scrollback(self.state.session_scroll_offset);
                true
            }
            MouseEventKind::ScrollDown => {
                self.state.session_scroll_offset = self
                    .state
                    .session_scroll_offset
                    .saturating_sub(MOUSE_SCROLL_STEP);
                session
                    .vterm
                    .set_scrollback(self.state.session_scroll_offset);
                true
            }
            _ => false,
        }
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

    fn session_viewport_height(&self) -> usize {
        let terminal_size = self.tui.size().unwrap_or_default();
        let frame_config = FrameConfig::default();
        let layout = FrameLayout::calculate(terminal_size, &frame_config);
        layout.content.height as usize
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
        requested_offset: usize,
        vterm_offset: usize,
        fallback_offset: usize,
        message: &'static str,
    ) {
        if self.mouse_debug_enabled {
            tracing::info!(
                target: "panoptes::mouse",
                session_id = %session_id,
                is_codex_session = true,
                requested_offset,
                vterm_offset,
                fallback_offset,
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
            View::ActivityTimeline => normal::timeline::handle_timeline_key(self, key),
            View::LogViewer => normal::log_viewer::handle_log_viewer_key(self, key),
            View::ClaudeConfigs => normal::claude_configs::handle_claude_configs_key(self, key),
            View::CodexConfigs => normal::codex_configs::handle_codex_configs_key(self, key),
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
    pub(crate) fn start_worktree_wizard(&mut self, project_id: ProjectId) {
        // Clear all wizard state
        self.state.worktree_wizard = WorktreeWizardState::default();
        self.state.fetch_error = None;

        // Get project info and clone what we need
        let (project_name, repo_path, default_base_branch) = {
            let Some(project) = self.project_store.get_project(project_id) else {
                self.state.error_message = Some("Project not found".to_string());
                return;
            };
            (
                project.name.clone(),
                project.repo_path.clone(),
                project.default_base_branch.clone(),
            )
        };
        self.state.worktree_wizard.project_name = project_name;

        // Get tracked branch names for this project
        let tracked_branches: std::collections::HashSet<String> = self
            .project_store
            .branches_for_project(project_id)
            .iter()
            .map(|b| b.name.clone())
            .collect();

        // Try to open git repo and fetch branches
        let Ok(git) = crate::git::GitOps::open(&repo_path) else {
            self.state.error_message = Some("Failed to open git repository".to_string());
            return;
        };

        // Get existing git worktrees to detect untracked worktrees
        let git_worktree_branches: std::collections::HashSet<String> =
            match crate::git::worktree::list_worktrees(git.repository()) {
                Ok(worktrees) => worktrees.into_iter().filter_map(|wt| wt.branch).collect(),
                Err(e) => {
                    tracing::warn!("Failed to list git worktrees: {}", e);
                    std::collections::HashSet::new()
                }
            };

        // Try to fetch from remotes (may fail if offline)
        let _ = self.show_loading("Fetching branches from remotes...");
        if let Err(e) = git.fetch_all_remotes() {
            tracing::warn!("Failed to fetch remotes: {}", e);
            self.state.fetch_error = Some(format!("Fetch failed: {}", e));
        }
        self.clear_loading();

        // Get all branch refs
        let default_base = default_base_branch.as_deref();
        match git.list_all_branch_refs(default_base) {
            Ok(refs) => {
                self.state.worktree_wizard.all_branches = refs
                    .into_iter()
                    .map(|r| {
                        let ref_type = match r.ref_type {
                            crate::git::BranchRefInfoType::Local => BranchRefType::Local,
                            crate::git::BranchRefInfoType::Remote => BranchRefType::Remote,
                        };
                        let is_tracked = tracked_branches.contains(&r.name);
                        // Branch has untracked git worktree if git knows about it but Panoptes doesn't track it
                        let has_git_worktree =
                            !is_tracked && git_worktree_branches.contains(&r.name);
                        BranchRef {
                            ref_type,
                            name: r.name.clone(),
                            display_name: r.name.clone(),
                            is_default_base: r.is_default_base,
                            is_already_tracked: is_tracked,
                            has_git_worktree,
                        }
                    })
                    .collect();
                self.state.worktree_wizard.filtered_branches =
                    self.state.worktree_wizard.all_branches.clone();
            }
            Err(e) => {
                tracing::error!("Failed to list branches: {}", e);
                self.state.error_message = Some(format!("Failed to list branches: {}", e));
                return;
            }
        }

        // Transition to step 1
        self.state.input_mode = InputMode::WorktreeSelectBranch;
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

    /// Fetch remotes and populate branch refs for a project
    fn fetch_and_populate_branch_refs(&mut self, project_id: ProjectId) {
        // Get project info and clone what we need
        let (repo_path, default_base_branch) = {
            let Some(project) = self.project_store.get_project(project_id) else {
                self.state.available_branch_refs.clear();
                self.state.filtered_branch_refs.clear();
                return;
            };
            (
                project.repo_path.clone(),
                project.default_base_branch.clone(),
            )
        };

        let Ok(git) = crate::git::GitOps::open(&repo_path) else {
            self.state.available_branch_refs.clear();
            self.state.filtered_branch_refs.clear();
            return;
        };

        // Try to fetch from remotes (may fail if offline, continue anyway)
        let _ = self.show_loading("Fetching branches from remotes...");
        if let Err(e) = git.fetch_all_remotes() {
            tracing::warn!("Failed to fetch remotes: {}", e);
            self.state.fetch_error = Some(format!("Fetch failed: {}", e));
        }
        self.clear_loading();

        // Get all branch refs
        let default_base = default_base_branch.as_deref();
        match git.list_all_branch_refs(default_base) {
            Ok(refs) => {
                // Convert git::BranchRefInfo to app::BranchRef
                self.state.available_branch_refs = refs
                    .into_iter()
                    .map(|r| {
                        let ref_type = match r.ref_type {
                            crate::git::BranchRefInfoType::Local => BranchRefType::Local,
                            crate::git::BranchRefInfoType::Remote => BranchRefType::Remote,
                        };
                        BranchRef {
                            ref_type,
                            name: r.name.clone(),
                            display_name: r.name,
                            is_default_base: r.is_default_base,
                            is_already_tracked: false, // Deprecated code path
                            has_git_worktree: false,   // Deprecated code path
                        }
                    })
                    .collect();
                self.state.filtered_branch_refs = self.state.available_branch_refs.clone();
                // Select the default base branch
                self.select_default_base_branch();
            }
            Err(e) => {
                tracing::error!("Failed to list branches: {}", e);
                self.state.available_branch_refs.clear();
                self.state.filtered_branch_refs.clear();
            }
        }
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
        let (repo_path, project_name, session_subdir) = {
            let Some(project) = self.project_store.get_project(project_id) else {
                anyhow::bail!("Project not found");
            };
            (
                project.repo_path.clone(),
                project.name.clone(),
                project.session_subdir.clone(),
            )
        };

        let git = crate::git::GitOps::open(&repo_path)?;
        let worktree_path = crate::git::worktree::worktree_path_for_branch(
            &self.config.worktrees_dir,
            &project_name,
            branch_name,
        );

        // Show loading indicator for worktree creation
        let _ = self.show_loading(&format!("Creating worktree for '{}'...", branch_name));

        crate::git::worktree::create_worktree(
            git.repository(),
            branch_name,
            &worktree_path,
            create_branch,
            base_ref,
        )?;

        self.clear_loading();

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
        self.project_store.save()?;
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
        let (repo_path, session_subdir) = {
            let Some(project) = self.project_store.get_project(project_id) else {
                anyhow::bail!("Project not found");
            };
            (project.repo_path.clone(), project.session_subdir.clone())
        };

        let git = crate::git::GitOps::open(&repo_path)?;

        // Find the worktree path for this branch from git
        let worktrees = crate::git::worktree::list_worktrees(git.repository())?;
        let worktree = worktrees
            .into_iter()
            .find(|wt| wt.branch.as_deref() == Some(branch_name))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No git worktree found for branch '{}'. It may have been removed.",
                    branch_name
                )
            })?;

        let worktree_path = worktree.path;

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
        self.project_store.save()?;
        tracing::info!("Imported existing worktree for branch: {}", branch_name);

        Ok(branch_id)
    }

    /// Jump to the next session needing attention (oldest first)
    pub(crate) fn jump_to_next_attention(&mut self) -> Result<()> {
        let attention_sessions = self
            .sessions
            .sessions_needing_attention(self.config.idle_threshold_secs);

        if let Some(session) = attention_sessions.first() {
            let session_id = session.info.id;
            self.state.navigate_to_session(session_id);
            self.tui.enable_mouse_capture();
            // Auto-enter session mode so user is immediately active in the session
            self.sessions.acknowledge_attention(session_id);
            if self.config.notification_method == "title" {
                SessionManager::reset_terminal_title();
            }
            self.resize_active_session_pty()?;
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
            SessionManager::send_notification(&self.config.notification_method, session_name);
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
    /// to the user that something is happening.
    pub(crate) fn show_loading(&mut self, message: &str) -> Result<()> {
        self.state.loading_message = Some(message.to_string());
        self.render()?;
        Ok(())
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
                View::ActivityTimeline => {
                    render_timeline(
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
                    let attention_count =
                        sessions.total_attention_count(config.idle_threshold_secs);
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
                    let attention_count =
                        sessions.total_attention_count(config.idle_threshold_secs);
                    render_claude_configs(
                        frame,
                        area,
                        claude_config_store,
                        state.claude_configs_selected_index,
                        &state.header_notifications,
                        attention_count,
                    );
                }
                View::CodexConfigs => {
                    let attention_count =
                        sessions.total_attention_count(config.idle_threshold_secs);
                    render_codex_configs(
                        frame,
                        area,
                        codex_config_store,
                        state.codex_configs_selected_index,
                        &state.header_notifications,
                        attention_count,
                    );
                }
            }

            // Render Claude config name input dialog
            if state.input_mode == InputMode::AddingClaudeConfigName {
                render_config_name_input_dialog(frame, area, &state.new_claude_config_name);
            }

            // Render Claude config path input dialog
            if state.input_mode == InputMode::AddingClaudeConfigPath {
                render_config_path_input_dialog(
                    frame,
                    area,
                    &state.new_claude_config_name,
                    &state.new_claude_config_path,
                    &state.path_completions,
                    state.path_completion_index,
                    state.show_path_completions,
                );
            }

            // Render Claude config delete confirmation dialog
            if state.input_mode == InputMode::ConfirmingClaudeConfigDelete {
                if let Some(config_id) = state.pending_delete_claude_config {
                    if let Some(config) = claude_config_store.get(config_id) {
                        // Find projects using this config
                        let affected_projects: Vec<String> = project_store
                            .projects()
                            .filter(|p| p.default_claude_config == Some(config_id))
                            .map(|p| p.name.clone())
                            .collect();
                        render_config_delete_dialog(frame, area, config, &affected_projects);
                    }
                }
            }

            // Render Claude config selector overlay
            if state.input_mode == InputMode::SelectingClaudeConfig {
                render_config_selector(
                    frame,
                    area,
                    &state.available_claude_configs,
                    state.claude_config_selector_index,
                    claude_config_store.get_default_id(),
                );
            }

            // Render Claude settings copy dialog
            if state.input_mode == InputMode::ConfirmingClaudeSettingsCopy {
                if let Some(ref copy_state) = state.pending_claude_settings_copy {
                    render_claude_settings_copy_dialog(frame, area, copy_state);
                }
            }

            // Render Claude settings migrate dialog
            if state.input_mode == InputMode::ConfirmingClaudeSettingsMigrate {
                if let Some(ref migrate_state) = state.pending_claude_settings_migrate {
                    render_claude_settings_migrate_dialog(frame, area, migrate_state);
                }
            }

            // Render agent type selector dialog
            if state.input_mode == InputMode::SelectingAgentType {
                render_agent_type_selector(frame, area, state.agent_type_selector_index);
            }

            // Render Codex config name input dialog
            if state.input_mode == InputMode::AddingCodexConfigName {
                render_codex_config_name_input_dialog(frame, area, &state.new_codex_config_name);
            }

            // Render Codex config path input dialog
            if state.input_mode == InputMode::AddingCodexConfigPath {
                render_codex_config_path_input_dialog(
                    frame,
                    area,
                    &state.new_codex_config_name,
                    &state.new_codex_config_path,
                    &state.path_completions,
                    state.path_completion_index,
                    state.show_path_completions,
                );
            }

            // Render Codex config delete confirmation dialog
            if state.input_mode == InputMode::ConfirmingCodexConfigDelete {
                if let Some(config_id) = state.pending_delete_codex_config {
                    if let Some(config) = codex_config_store.get(config_id) {
                        // Find projects using this config
                        let affected_projects: Vec<String> = project_store
                            .projects()
                            .filter(|p| p.default_codex_config == Some(config_id))
                            .map(|p| p.name.clone())
                            .collect();
                        render_codex_config_delete_dialog(frame, area, config, &affected_projects);
                    }
                }
            }

            // Render Codex config selector overlay
            if state.input_mode == InputMode::SelectingCodexConfig {
                render_codex_config_selector(
                    frame,
                    area,
                    &state.available_codex_configs,
                    state.codex_config_selector_index,
                    codex_config_store.get_default_id(),
                );
            }

            // Render custom shortcut dialogs
            if matches!(
                state.input_mode,
                InputMode::ManagingCustomShortcuts
                    | InputMode::AddingCustomShortcutKey
                    | InputMode::AddingCustomShortcutName
                    | InputMode::AddingCustomShortcutCommand
                    | InputMode::AddingCustomShortcutAutoClose
                    | InputMode::ConfirmingCustomShortcutDelete
            ) {
                render_custom_shortcut_dialogs(frame, area, state, config);
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
    fn test_view_default() {
        assert_eq!(View::default(), View::ProjectsOverview);
    }

    #[test]
    fn test_app_state_default() {
        let state = AppState::default();
        assert_eq!(state.view, View::ProjectsOverview);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert_eq!(state.selected_project_index, 0);
        assert_eq!(state.selected_branch_index, 0);
        assert_eq!(state.selected_session_index, 0);
        assert!(state.active_session.is_none());
        assert!(!state.should_quit);
    }

    #[test]
    fn test_app_state_select_next() {
        let mut state = AppState::default();
        // In ProjectsOverview, selection uses selected_project_index
        state.select_next(3);
        assert_eq!(state.selected_project_index, 1);
        state.select_next(3);
        assert_eq!(state.selected_project_index, 2);
        state.select_next(3);
        assert_eq!(state.selected_project_index, 0); // Wraps around
    }

    #[test]
    fn test_app_state_select_prev() {
        let mut state = AppState::default();
        state.select_prev(3);
        assert_eq!(state.selected_project_index, 2); // Wraps to end
        state.select_prev(3);
        assert_eq!(state.selected_project_index, 1);
    }

    #[test]
    fn test_app_state_select_by_number() {
        let mut state = AppState::default();
        state.select_by_number(2, 5);
        assert_eq!(state.selected_project_index, 1); // 1-indexed to 0-indexed
        state.select_by_number(0, 5); // Invalid, should not change
        assert_eq!(state.selected_project_index, 1);
        state.select_by_number(6, 5); // Out of range, should not change
        assert_eq!(state.selected_project_index, 1);
    }

    #[test]
    fn test_app_state_select_empty() {
        let mut state = AppState::default();
        state.select_next(0); // Should not panic
        assert_eq!(state.selected_project_index, 0);
        state.select_prev(0); // Should not panic
        assert_eq!(state.selected_project_index, 0);
    }

    #[test]
    fn test_view_helpers() {
        assert!(View::ProjectsOverview.is_projects_overview());
        assert!(!View::ProjectsOverview.is_session_view());

        let project_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();

        let project_view = View::ProjectDetail(project_id);
        assert!(project_view.is_project_detail());
        assert_eq!(project_view.project_id(), Some(project_id));
        assert_eq!(project_view.parent(), Some(View::ProjectsOverview));

        let branch_view = View::BranchDetail(project_id, branch_id);
        assert!(branch_view.is_branch_detail());
        assert_eq!(branch_view.project_id(), Some(project_id));
        assert_eq!(branch_view.branch_id(), Some(branch_id));
        assert_eq!(branch_view.parent(), Some(View::ProjectDetail(project_id)));

        assert!(View::ActivityTimeline.is_activity_timeline());
        assert_eq!(
            View::ActivityTimeline.parent(),
            Some(View::ProjectsOverview)
        );
    }

    #[test]
    fn test_navigation_helpers() {
        let mut state = AppState::default();
        let project_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();

        // Create a SessionManager for testing (session won't exist, tests fallback path).
        // Backed by a temp store so the test never touches the real
        // ~/.panoptes/sessions.json.
        let temp_dir = tempfile::TempDir::new().unwrap();
        let sessions = SessionManager::with_store(
            Config::default(),
            crate::session::SessionStore::with_path(temp_dir.path().join("sessions.json")),
        );

        // Navigate to project
        state.navigate_to_project(project_id);
        assert_eq!(state.view, View::ProjectDetail(project_id));
        assert_eq!(state.selected_branch_index, 0);

        // Navigate to branch
        state.navigate_to_branch(project_id, branch_id);
        assert_eq!(state.view, View::BranchDetail(project_id, branch_id));
        assert_eq!(state.selected_session_index, 0);

        // Navigate to session
        state.navigate_to_session(session_id);
        assert_eq!(state.view, View::SessionView);
        assert_eq!(state.active_session, Some(session_id));
        assert_eq!(
            state.session_return_view,
            Some(View::BranchDetail(project_id, branch_id))
        );

        // Return from session (session not in manager, uses fallback to stored return view)
        state.return_from_session(&sessions);
        assert_eq!(state.view, View::BranchDetail(project_id, branch_id));
        assert!(state.active_session.is_none());

        // Navigate back
        state.navigate_back();
        assert_eq!(state.view, View::ProjectDetail(project_id));

        state.navigate_back();
        assert_eq!(state.view, View::ProjectsOverview);
    }

    #[test]
    fn test_timeline_navigation() {
        let mut state = AppState::default();

        state.navigate_to_timeline();
        assert_eq!(state.view, View::ActivityTimeline);
        assert_eq!(state.selected_timeline_index, 0);

        state.navigate_back();
        assert_eq!(state.view, View::ProjectsOverview);
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
            view: View::ActivityTimeline,
            ..Default::default()
        };

        // Start from timeline, navigate to session A
        state.navigate_to_session(_session_a);
        assert_eq!(state.session_return_view, Some(View::ActivityTimeline));

        // Jump to session B (simulates Space key)
        state.active_session = Some(session_b);
        state.session_return_view = Some(View::SessionView); // This is what causes the bug

        // Return from session - should go to session B's branch detail, NOT ActivityTimeline
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
