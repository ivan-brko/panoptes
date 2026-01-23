//! Application state and main event loop
//!
//! This module contains the central application state and the main event loop
//! that ties together session management, hook handling, and terminal UI.

// Submodules
mod event_loop;
mod input_mode;
mod state;
mod view;

// Re-exports from submodules
pub use input_mode::InputMode;
pub use state::{AppState, HomepageFocus};
pub use view::View;

// Re-exports from wizards (for backwards compatibility)
pub use crate::wizards::worktree::{BranchRef, BranchRefType, WorktreeCreationType};

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
};

use crate::config::Config;
use crate::focus_timing::stats::{FocusContextBreakdown, FocusSession};
use crate::focus_timing::store::FocusStore;
use crate::focus_timing::FocusTimer;
use crate::hooks::{
    self, HookEventReceiver, HookEventSender, ServerHandle, DEFAULT_CHANNEL_BUFFER,
};
use crate::logging::{LogBuffer, LogFileInfo};
use crate::project::{BranchId, ProjectId, ProjectStore};
use crate::session::{mouse_event_to_bytes, SessionManager};
use crate::tui::frame::{FrameConfig, FrameLayout};
use crate::tui::views::{
    render_branch_detail, render_focus_session_delete_dialog, render_focus_session_detail_dialog,
    render_focus_stats, render_loading_indicator, render_log_viewer, render_notifications,
    render_project_detail, render_projects_overview, render_session_view, render_timeline,
    render_timer_input_dialog,
};
use crate::tui::{NotificationType, Tui};
use crate::wizards::worktree::filter_branch_refs;

/// Main application struct
pub struct App {
    /// Application configuration (used for project flows)
    pub(crate) config: Config,
    /// Application state
    pub(crate) state: AppState,
    /// Project store for project/branch persistence
    pub(crate) project_store: ProjectStore,
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
}

impl App {
    /// Create a new application instance
    pub async fn new(log_buffer: Arc<LogBuffer>, log_file_info: LogFileInfo) -> Result<Self> {
        let config = Config::load()?;

        // Load project store (or create empty if doesn't exist)
        let project_store = ProjectStore::load().unwrap_or_else(|e| {
            tracing::warn!("Failed to load project store: {}, starting fresh", e);
            ProjectStore::new()
        });
        tracing::debug!(
            "Loaded {} projects, {} branches",
            project_store.project_count(),
            project_store.branch_count()
        );

        // Create hook event channel with large buffer to avoid dropping events
        let (hook_tx, hook_rx): (HookEventSender, HookEventReceiver) =
            hooks::server::create_channel(DEFAULT_CHANNEL_BUFFER);

        // Start hook server
        let hook_server = hooks::server::start(config.hook_port, hook_tx).await?;
        tracing::debug!("Hook server started on port {}", hook_server.addr().port());

        // Create session manager
        let sessions = SessionManager::new(config.clone());

        // Create TUI
        let tui = Tui::new()?;

        Ok(Self {
            config,
            state: AppState::default(),
            project_store,
            sessions,
            hook_rx,
            hook_server,
            tui,
            log_buffer,
            log_file_info,
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
                        if self.handle_mouse_event(mouse)? {
                            self.state.needs_render = true;
                        }
                    }
                    Event::FocusGained => {
                        self.state.terminal_focused = true;
                        self.state.focus_events_supported = true;
                        self.state.focus_tracker.handle_focus_gained();
                        // Timer runs on wall-clock time, no pause/resume needed
                        self.state.needs_render = true;
                    }
                    Event::FocusLost => {
                        self.state.terminal_focused = false;
                        self.state.focus_events_supported = true;
                        self.state.focus_tracker.handle_focus_lost();
                        // Timer runs on wall-clock time, no pause/resume needed
                        self.state.needs_render = true;
                    }
                }
            }

            // Force render when focus timer is running to update countdown display
            if self
                .state
                .focus_timer
                .as_ref()
                .is_some_and(|t| t.is_running())
            {
                self.state.needs_render = true;
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

            // Poll session outputs - mark dirty if any session has new output
            if !self.sessions.poll_outputs().is_empty() {
                self.state.needs_render = true;
            }

            // Check for dead sessions
            let had_state_changes = self.sessions.check_alive();
            if had_state_changes {
                self.state.needs_render = true;
            }

            // Check for sessions stuck in Executing state too long
            let had_timeout_changes = self
                .sessions
                .check_state_timeouts(self.config.state_timeout_secs);
            if had_timeout_changes {
                self.state.needs_render = true;
            }

            // Clean up old exited sessions to prevent memory growth
            let cleaned_up = self
                .sessions
                .cleanup_exited_sessions(self.config.exited_retention_secs);
            if cleaned_up > 0 {
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

            // Check focus timer completion
            let timer_complete = self
                .state
                .focus_timer
                .as_ref()
                .map(|t| (t.is_complete(), t.target_duration));

            if let Some((true, target)) = timer_complete {
                // Query focus tracker for the focused time during this timer window
                let focused = self.state.focus_tracker.focused_time_in_last(target);
                // Complete and save the timer
                self.complete_focus_timer(focused);
                // Show notification
                self.state.notifications.push(
                    NotificationType::TimerComplete { focused, target },
                    Some(Duration::from_secs(30)),
                );
                // Terminal bell
                if self.config.notification_method == "bell" {
                    print!("\x07");
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                }
                self.state.needs_render = true;
            }

            // Tick notifications (remove expired)
            self.state.notifications.tick();
            self.state.header_notifications.tick();

            // Check if we should quit
            if self.state.should_quit {
                break;
            }
        }

        Ok(())
    }

    /// Process pending hook events from the channel
    /// Returns true if any events were processed
    fn process_hook_events(&mut self) -> bool {
        let mut had_events = false;
        while let Ok(event) = self.hook_rx.try_recv() {
            tracing::debug!(
                "Hook event: session={}, event={}, tool={:?}",
                event.session_id,
                event.event,
                event.tool
            );
            // handle_hook_event returns Some(session_id) if notification should be sent
            if let Some(session_id) = self.sessions.handle_hook_event(&event) {
                // Only notify if this session is NOT the one we're currently viewing
                let is_active_session = self.state.active_session == Some(session_id);
                if !is_active_session {
                    let session_name = self
                        .sessions
                        .get(session_id)
                        .map(|s| s.info.name.as_str())
                        .unwrap_or("Session");
                    SessionManager::send_notification(
                        &self.config.notification_method,
                        session_name,
                    );
                }
            }
            had_events = true;
        }
        had_events
    }

    /// Handle paste event (for clipboard paste support)
    fn handle_paste_event(&mut self, text: &str) -> Result<()> {
        // Clean the pasted text (take first line, trim whitespace)
        let cleaned = text.lines().next().unwrap_or("").trim();

        match self.state.input_mode {
            InputMode::AddingProject => {
                self.state.new_project_path.push_str(cleaned);
                self.update_path_completions();
            }
            InputMode::AddingProjectName | InputMode::RenamingProject => {
                self.state.new_project_name.push_str(cleaned);
            }
            InputMode::CreatingSession => {
                self.state.new_session_name.push_str(cleaned);
            }
            InputMode::CreatingWorktree | InputMode::SelectingDefaultBase => {
                self.state.new_branch_name.push_str(cleaned);
                // Update filtered branches
                self.state.filtered_branch_refs = filter_branch_refs(
                    &self.state.available_branch_refs,
                    &self.state.new_branch_name,
                );
                self.select_default_base_branch();
            }
            InputMode::Session => {
                // Send pasted text to PTY (wrapped in brackets if app enabled it)
                if let Some(session_id) = self.state.active_session {
                    if let Some(session) = self.sessions.get_mut(session_id) {
                        session.write_paste(text)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
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

        let Some(session) = self.sessions.get_mut(session_id) else {
            return Ok(false);
        };

        // Check if Claude Code has mouse mode enabled
        let mouse_enabled = session.vterm.mouse_protocol_mode() != vt100::MouseProtocolMode::None;

        if mouse_enabled {
            // Forward mouse events to PTY when Claude Code wants them
            // (for vim, etc. with mouse support)
            let terminal_size = self.tui.size()?;
            let frame_config = FrameConfig::default();
            let layout = FrameLayout::calculate(
                ratatui::prelude::Rect::new(0, 0, terminal_size.width, terminal_size.height),
                &frame_config,
            );
            if let Some(bytes) = mouse_event_to_bytes(mouse, layout.content) {
                session.write(&bytes)?;
                return Ok(true);
            }
        } else {
            // Handle scroll wheel for our own scrollback when mouse mode is disabled
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    let max_scroll = session.vterm.max_scrollback();
                    // Scroll up by 3 lines (same as claude-wrapper)
                    self.state.session_scroll_offset = self
                        .state
                        .session_scroll_offset
                        .saturating_add(3)
                        .min(max_scroll);
                    session
                        .vterm
                        .set_scrollback(self.state.session_scroll_offset);
                    return Ok(true);
                }
                MouseEventKind::ScrollDown => {
                    // Scroll down by 3 lines (same as claude-wrapper)
                    self.state.session_scroll_offset =
                        self.state.session_scroll_offset.saturating_sub(3);
                    session
                        .vterm
                        .set_scrollback(self.state.session_scroll_offset);
                    return Ok(true);
                }
                _ => {}
            }
        }

        Ok(false)
    }

    // ========================================================================
    // Input Handlers (called by input::dispatcher)
    // ========================================================================

    /// Handle key in normal mode
    pub(crate) fn handle_normal_mode_key(&mut self, key: KeyEvent) -> Result<()> {
        use crate::input::normal;
        match self.state.view {
            View::ProjectsOverview => normal::projects_overview::handle_projects_overview_key(self, key),
            View::ProjectDetail(_) => normal::project_detail::handle_project_detail_key(self, key),
            View::BranchDetail(project_id, branch_id) => {
                normal::branch_detail::handle_branch_detail_key(self, key, project_id, branch_id)
            }
            View::SessionView => normal::session_view::handle_session_view_normal_key(self, key),
            View::ActivityTimeline => normal::timeline::handle_timeline_key(self, key),
            View::LogViewer => normal::log_viewer::handle_log_viewer_key(self, key),
            View::FocusStats => normal::focus_stats::handle_focus_stats_key(self, key),
        }
    }

    /// Handle common focus timer shortcuts. Returns true if the key was handled.
    pub(crate) fn handle_focus_timer_shortcut(&mut self, key: KeyEvent) -> bool {
        // Ctrl+t: stop timer (if running)
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('t')
            && self.state.focus_timer.is_some()
        {
            self.stop_focus_timer();
            return true;
        }

        match key.code {
            KeyCode::Char('t') => {
                self.start_focus_timer_dialog();
                true
            }
            KeyCode::Char('T') => {
                self.load_focus_sessions();
                self.state.view = View::FocusStats;
                self.state.focus_stats_selected_index = 0;
                true
            }
            _ => false,
        }
    }

    /// Start the focus timer dialog
    fn start_focus_timer_dialog(&mut self) {
        self.state.input_mode = InputMode::StartingFocusTimer;
        self.state.focus_timer_input.clear();
    }

    /// Start a focus timer with the given duration
    pub(crate) fn start_focus_timer(&mut self, minutes: u64) {
        let mut timer = FocusTimer::new(minutes);

        // Set context based on current view
        if let Some(project_id) = self.state.view.project_id() {
            timer = timer.with_project(project_id);
        }
        if let Some(branch_id) = self.state.view.branch_id() {
            timer = timer.with_branch(branch_id);
        }

        // Set focus tracker context
        self.state
            .focus_tracker
            .set_context(self.state.view.project_id(), self.state.view.branch_id());

        timer.start();
        self.state.focus_timer = Some(timer);

        tracing::info!("Started focus timer for {} minutes", minutes);
    }

    /// Stop the active focus timer (user-initiated)
    fn stop_focus_timer(&mut self) {
        if let Some(mut timer) = self.state.focus_timer.take() {
            let target = timer.target_duration;
            if let Some(elapsed) = timer.stop() {
                // Query focus tracker for the focused time during this timer window
                let focused = self.state.focus_tracker.focused_time_in_last(elapsed);
                // Get per-project/branch breakdown
                let breakdown_map = self
                    .state
                    .focus_tracker
                    .focused_time_breakdown_in_last(elapsed);
                let breakdown: Vec<FocusContextBreakdown> = breakdown_map
                    .into_iter()
                    .map(
                        |((project_id, branch_id), duration)| FocusContextBreakdown {
                            project_id,
                            branch_id,
                            duration,
                        },
                    )
                    .collect();
                self.save_focus_session(
                    target,
                    focused,
                    elapsed,
                    timer.project_id,
                    timer.branch_id,
                    breakdown,
                );
            }
        }
    }

    /// Complete the focus timer (called when timer reaches target)
    /// Takes the focused_duration which was already calculated from focus_tracker
    fn complete_focus_timer(&mut self, focused_duration: Duration) {
        if let Some(mut timer) = self.state.focus_timer.take() {
            let target = timer.target_duration;
            if let Some(elapsed) = timer.stop() {
                // Get per-project/branch breakdown
                let breakdown_map = self
                    .state
                    .focus_tracker
                    .focused_time_breakdown_in_last(elapsed);
                let breakdown: Vec<FocusContextBreakdown> = breakdown_map
                    .into_iter()
                    .map(
                        |((project_id, branch_id), duration)| FocusContextBreakdown {
                            project_id,
                            branch_id,
                            duration,
                        },
                    )
                    .collect();
                self.save_focus_session(
                    target,
                    focused_duration,
                    elapsed,
                    timer.project_id,
                    timer.branch_id,
                    breakdown,
                );
            }
        }
    }

    /// Load focus sessions from disk into state
    pub(crate) fn load_focus_sessions(&mut self) {
        let store = FocusStore::new();
        match store.load() {
            Ok(sessions) => {
                self.state.focus_sessions = sessions;
            }
            Err(e) => {
                tracing::error!("Failed to load focus sessions: {}", e);
                self.state.focus_sessions = Vec::new();
            }
        }
    }

    /// Save a focus session result
    fn save_focus_session(
        &mut self,
        target_duration: Duration,
        focused_duration: Duration,
        elapsed_duration: Duration,
        project_id: Option<uuid::Uuid>,
        branch_id: Option<uuid::Uuid>,
        context_breakdown: Vec<FocusContextBreakdown>,
    ) {
        // Save the session
        let session = FocusSession::from_timer_result(
            target_duration,
            focused_duration,
            elapsed_duration,
            project_id,
            branch_id,
            context_breakdown,
        );

        // Add to cached sessions
        self.state.focus_sessions.push(session.clone());

        // Persist to disk
        let store = FocusStore::new();
        if let Err(e) = store.add_session(session) {
            tracing::error!("Failed to save focus session: {}", e);
        }

        tracing::info!(
            "Focus session complete: {:.0}% focus",
            (focused_duration.as_secs_f64() / target_duration.as_secs_f64()) * 100.0
        );
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
        self.state.worktree_search_text.clear();
        self.state.worktree_list_index = 0;
        self.state.worktree_branch_name.clear();
        self.state.worktree_source_branch = None;
        self.state.worktree_base_branch = None;
        self.state.worktree_base_search_text.clear();
        self.state.worktree_base_list_index = 0;
        self.state.worktree_creation_type = WorktreeCreationType::ExistingLocal;
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
        self.state.worktree_project_name = project_name;

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
                self.state.worktree_all_branches = refs
                    .into_iter()
                    .map(|r| {
                        let ref_type = match r.ref_type {
                            crate::git::BranchRefInfoType::Local => BranchRefType::Local,
                            crate::git::BranchRefInfoType::Remote => BranchRefType::Remote,
                        };
                        BranchRef {
                            ref_type,
                            name: r.name.clone(),
                            display_name: r.name.clone(),
                            is_default_base: r.is_default_base,
                            is_already_tracked: tracked_branches.contains(&r.name),
                        }
                    })
                    .collect();
                self.state.worktree_filtered_branches = self.state.worktree_all_branches.clone();
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
    pub(crate) fn resize_active_session_pty(&mut self) -> Result<()> {
        if let Some(session_id) = self.state.active_session {
            let size = self.tui.size()?;
            // Output area is: total height - header (3) - footer (3) - borders (2)
            let output_rows = size.height.saturating_sub(8);
            // Output width is: total width - borders (2)
            let output_cols = size.width.saturating_sub(2);

            if let Some(session) = self.sessions.get_mut(session_id) {
                session.resize(output_cols, output_rows)?;
            }
        }
        Ok(())
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
                        state.focus_timer.as_ref(),
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
                        state.focus_timer.as_ref(),
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
                        state.focus_timer.as_ref(),
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
                        state.focus_timer.as_ref(),
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
                        state.focus_timer.as_ref(),
                        &state.header_notifications,
                        attention_count,
                    );
                }
                View::FocusStats => {
                    let attention_count =
                        sessions.total_attention_count(config.idle_threshold_secs);
                    render_focus_stats(
                        frame,
                        area,
                        &state.focus_sessions,
                        project_store,
                        state.focus_stats_selected_index,
                        state.focus_events_supported,
                        state.focus_timer.as_ref(),
                        &state.header_notifications,
                        attention_count,
                    );
                }
            }

            // Render timer input dialog if entering timer duration
            if state.input_mode == InputMode::StartingFocusTimer {
                render_timer_input_dialog(
                    frame,
                    area,
                    &state.focus_timer_input,
                    config.focus_timer_minutes,
                );
            }

            // Render focus session delete confirmation dialog
            if state.input_mode == InputMode::ConfirmingFocusSessionDelete {
                if let Some(session_id) = state.pending_delete_focus_session {
                    // Find the session in the list
                    if let Some(session) = state.focus_sessions.iter().find(|s| s.id == session_id)
                    {
                        render_focus_session_delete_dialog(frame, area, session, project_store);
                    }
                }
            }

            // Render focus session detail dialog
            if state.input_mode == InputMode::ViewingFocusSessionDetail {
                if let Some(ref session) = state.viewing_focus_session {
                    render_focus_session_detail_dialog(frame, area, session, project_store);
                }
            }

            // Render notifications overlay
            let visible_notifications = state.notifications.visible();
            if !visible_notifications.is_empty() {
                render_notifications(frame, area, &visible_notifications);
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

        // Return from session
        state.return_from_session();
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
}
