//! Application state and main event loop
//!
//! This module contains the central application state and the main event loop
//! that ties together session management, hook handling, and terminal UI.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use uuid::Uuid;

use crate::config::Config;
use crate::focus_timing::stats::FocusSession;
use crate::focus_timing::store::FocusStore;
use crate::focus_timing::tracker::FocusTracker;
use crate::focus_timing::FocusTimer;
use crate::hooks::{
    self, HookEventReceiver, HookEventSender, ServerHandle, DEFAULT_CHANNEL_BUFFER,
};
use crate::logging::{LogBuffer, LogFileInfo};
use crate::project::{BranchId, ProjectId, ProjectStore};
use crate::session::{mouse_event_to_bytes, SessionId, SessionManager};
use crate::tui::frame::{FrameConfig, FrameLayout};
use crate::tui::views::{
    render_branch_detail, render_focus_stats, render_loading_indicator, render_log_viewer,
    render_notifications, render_project_detail, render_projects_overview, render_session_view,
    render_timeline, render_timer_input_dialog,
};
use crate::tui::{NotificationManager, NotificationType, Tui};

/// Focus state for the homepage (projects overview)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HomepageFocus {
    /// Projects list is focused
    #[default]
    Projects,
    /// Sessions list is focused
    Sessions,
}

/// Type of branch reference (local or remote)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchRefType {
    /// Local branch (e.g., "main")
    Local,
    /// Remote tracking branch (e.g., "origin/main")
    Remote,
}

impl BranchRefType {
    /// Get display prefix for UI
    pub fn prefix(&self) -> &'static str {
        match self {
            BranchRefType::Local => "[L]",
            BranchRefType::Remote => "[R]",
        }
    }
}

/// A reference to a git branch (local or remote)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRef {
    /// Type of branch (local or remote)
    pub ref_type: BranchRefType,
    /// Full reference name (e.g., "main" or "origin/main")
    pub name: String,
    /// Display name for UI
    pub display_name: String,
    /// Whether this is the default base branch
    pub is_default_base: bool,
    /// Whether this branch already has a worktree tracked in Panoptes
    pub is_already_tracked: bool,
}

impl BranchRef {
    /// Create a new branch reference
    pub fn new(ref_type: BranchRefType, name: String) -> Self {
        let display_name = name.clone();
        Self {
            ref_type,
            name,
            display_name,
            is_default_base: false,
            is_already_tracked: false,
        }
    }

    /// Mark this branch as the default base
    pub fn with_default_base(mut self, is_default: bool) -> Self {
        self.is_default_base = is_default;
        self
    }
}

/// Input mode determines how keyboard input is handled
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    /// Normal mode - keys are handled as commands
    #[default]
    Normal,
    /// Session mode - keys are sent to the active session's PTY
    Session,
    /// Creating a new session - typing session name
    CreatingSession,
    /// Adding a new project - typing path
    AddingProject,
    /// Adding a new project - typing optional name (after path validation)
    AddingProjectName,
    /// Fetching branches from git remotes (shows spinner) - DEPRECATED
    FetchingBranches,
    /// Creating a new worktree - typing branch name - DEPRECATED (use WorktreeSelectBranch)
    CreatingWorktree,
    /// Selecting default base branch - DEPRECATED (use WorktreeSelectBase)
    SelectingDefaultBase,
    /// Confirming session deletion
    ConfirmingSessionDelete,
    /// Confirming branch/worktree deletion
    ConfirmingBranchDelete,
    /// Confirming project deletion
    ConfirmingProjectDelete,
    /// Confirming application quit
    ConfirmingQuit,
    /// Renaming a project
    RenamingProject,
    /// Worktree creation Step 1: Search/select existing branch or create new
    WorktreeSelectBranch,
    /// Worktree creation Step 2: Select base branch for new branch
    WorktreeSelectBase,
    /// Worktree creation Step 3: Confirmation before creation
    WorktreeConfirm,
    /// Starting a focus timer - entering duration
    StartingFocusTimer,
}

/// Type of worktree creation being performed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorktreeCreationType {
    /// Checkout existing local branch into a worktree
    #[default]
    ExistingLocal,
    /// Create local tracking branch from remote and checkout into worktree
    RemoteTracking,
    /// Create a new branch from a base and checkout into worktree
    NewBranch,
}

/// Current view being displayed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    /// Grid of projects (main landing page)
    #[default]
    ProjectsOverview,
    /// Branches for a specific project
    ProjectDetail(ProjectId),
    /// Sessions for a specific branch
    BranchDetail(ProjectId, BranchId),
    /// Single session fullscreen view
    SessionView,
    /// All sessions sorted by recent activity
    ActivityTimeline,
    /// Log viewer showing application logs
    LogViewer,
    /// Focus timing statistics view
    FocusStats,
}

impl View {
    /// Check if this view is the projects overview
    pub fn is_projects_overview(&self) -> bool {
        matches!(self, View::ProjectsOverview)
    }

    /// Check if this view is a project detail view
    pub fn is_project_detail(&self) -> bool {
        matches!(self, View::ProjectDetail(_))
    }

    /// Check if this view is a branch detail view
    pub fn is_branch_detail(&self) -> bool {
        matches!(self, View::BranchDetail(_, _))
    }

    /// Check if this view is the session view
    pub fn is_session_view(&self) -> bool {
        matches!(self, View::SessionView)
    }

    /// Check if this view is the activity timeline
    pub fn is_activity_timeline(&self) -> bool {
        matches!(self, View::ActivityTimeline)
    }

    /// Check if this view is the focus stats view
    pub fn is_focus_stats(&self) -> bool {
        matches!(self, View::FocusStats)
    }

    /// Get the parent view for navigation (Esc key)
    pub fn parent(&self) -> Option<View> {
        match self {
            View::ProjectsOverview => None,
            View::ProjectDetail(_) => Some(View::ProjectsOverview),
            View::BranchDetail(project_id, _) => Some(View::ProjectDetail(*project_id)),
            View::SessionView => None, // Handled specially based on context
            View::ActivityTimeline => Some(View::ProjectsOverview),
            View::LogViewer => Some(View::ProjectsOverview),
            View::FocusStats => Some(View::ProjectsOverview),
        }
    }

    /// Get the project ID if this view is associated with a project
    pub fn project_id(&self) -> Option<ProjectId> {
        match self {
            View::ProjectDetail(id) => Some(*id),
            View::BranchDetail(id, _) => Some(*id),
            _ => None,
        }
    }

    /// Get the branch ID if this view is associated with a branch
    pub fn branch_id(&self) -> Option<BranchId> {
        match self {
            View::BranchDetail(_, id) => Some(*id),
            _ => None,
        }
    }
}

/// Application state
#[derive(Default)]
pub struct AppState {
    /// Current view
    pub view: View,
    /// Current input mode
    pub input_mode: InputMode,
    /// Selected index in ProjectsOverview
    pub selected_project_index: usize,
    /// Selected index in ProjectDetail (branch selection)
    pub selected_branch_index: usize,
    /// Selected index in BranchDetail (session selection)
    pub selected_session_index: usize,
    /// Selected index in ActivityTimeline
    pub selected_timeline_index: usize,
    /// Session being viewed (in session view)
    pub active_session: Option<SessionId>,
    /// Context for returning from session view (which view to go back to)
    pub session_return_view: Option<View>,
    /// Buffer for new session name input
    pub new_session_name: String,
    /// Context: project ID for session being created (None = unassociated)
    pub creating_session_project_id: Option<ProjectId>,
    /// Context: branch ID for session being created (None = unassociated)
    pub creating_session_branch_id: Option<BranchId>,
    /// Context: working directory for session being created
    pub creating_session_working_dir: Option<PathBuf>,
    /// Buffer for new project path input
    pub new_project_path: String,
    /// Path completions for autocomplete
    pub path_completions: Vec<PathBuf>,
    /// Selected index in path completions list
    pub path_completion_index: usize,
    /// Whether to show path completions popup
    pub show_path_completions: bool,
    /// Buffer for new project name input (optional custom name)
    pub new_project_name: String,
    /// Pending project path (validated repo path) for two-step project addition
    pub pending_project_path: PathBuf,
    /// Pending session subdir (computed from user path vs repo root)
    pub pending_session_subdir: Option<PathBuf>,
    /// Pending default branch (computed during path validation)
    pub pending_default_branch: String,
    /// Buffer for new branch name input (worktree creation)
    pub new_branch_name: String,
    /// Cached branches from git (for branch selector) - legacy, keep for compatibility
    pub available_branches: Vec<String>,
    /// Branches matching current search filter - legacy, keep for compatibility
    pub filtered_branches: Vec<String>,
    /// Selected index in branch selector (0 = "Create new")
    pub branch_selector_index: usize,
    /// Available branch refs (local and remote) for worktree creation
    pub available_branch_refs: Vec<BranchRef>,
    /// Filtered branch refs matching search query
    pub filtered_branch_refs: Vec<BranchRef>,
    /// Selected index in base branch selector
    pub base_branch_selector_index: usize,
    /// The currently selected base branch (independent of filtering)
    pub selected_base_branch: Option<BranchRef>,
    /// Whether git fetch encountered an error (show warning)
    pub fetch_error: Option<String>,
    /// Session pending deletion (for confirmation dialog)
    pub pending_delete_session: Option<SessionId>,
    /// Project pending deletion (for confirmation dialog)
    pub pending_delete_project: Option<ProjectId>,
    /// Branch pending deletion (for confirmation dialog)
    pub pending_delete_branch: Option<BranchId>,
    /// Whether to also delete the git worktree on disk when deleting a branch
    pub delete_worktree_on_disk: bool,
    /// Project being renamed
    pub renaming_project: Option<ProjectId>,
    /// Whether the application should quit
    pub should_quit: bool,
    /// Whether the UI needs to be re-rendered
    pub needs_render: bool,
    /// Count of dropped hook events (for warning display)
    pub dropped_events_count: u64,
    /// Error message to display to the user (cleared on next keypress)
    pub error_message: Option<String>,
    /// Timestamp of last resize event (for debouncing)
    pub last_resize: Option<Instant>,
    /// Whether a resize is pending (debounced)
    pub pending_resize: bool,
    /// Scroll offset in log viewer
    pub log_viewer_scroll: usize,
    /// Whether log viewer auto-scrolls to new entries
    pub log_viewer_auto_scroll: bool,
    /// Scroll offset for session view (0 = live view, >0 = scrolled back)
    pub session_scroll_offset: usize,

    // --- New worktree creation wizard state ---
    /// Search text in WorktreeSelectBranch step
    pub worktree_search_text: String,
    /// All available branches (local + remote) for worktree creation
    pub worktree_all_branches: Vec<BranchRef>,
    /// Filtered branches matching search text
    pub worktree_filtered_branches: Vec<BranchRef>,
    /// Selected index in branch list (0..N = branches, N = "create new" option)
    pub worktree_list_index: usize,
    /// Final local branch name (for new branch or from remote)
    pub worktree_branch_name: String,
    /// Selected existing/remote branch (for ExistingLocal/RemoteTracking)
    pub worktree_source_branch: Option<BranchRef>,
    /// Base branch for creating new branches (step 2)
    pub worktree_base_branch: Option<BranchRef>,
    /// Search text in WorktreeSelectBase step
    pub worktree_base_search_text: String,
    /// Selected index in base branch list (step 2)
    pub worktree_base_list_index: usize,
    /// Type of worktree creation being performed
    pub worktree_creation_type: WorktreeCreationType,
    /// Project name for worktree path (cached during wizard)
    pub worktree_project_name: String,
    /// Loading message to display during blocking operations
    pub loading_message: Option<String>,
    /// Focus state for homepage (projects vs sessions list)
    pub homepage_focus: HomepageFocus,

    // --- Focus timing state ---
    /// Focus tracker for recording focus intervals
    pub focus_tracker: FocusTracker,
    /// Active focus timer (if any)
    pub focus_timer: Option<FocusTimer>,
    /// Notification manager for displaying alerts
    pub notifications: NotificationManager,
    /// Whether terminal currently has focus
    pub terminal_focused: bool,
    /// Whether focus events are supported by terminal
    pub focus_events_supported: bool,
    /// Input buffer for timer duration entry
    pub focus_timer_input: String,
    /// Selected index in focus stats view
    pub focus_stats_selected_index: usize,
    /// Cached focus sessions for stats view
    pub focus_sessions: Vec<FocusSession>,
}

impl AppState {
    /// Get the current selected index for the current view
    pub fn current_selected_index(&self) -> usize {
        match self.view {
            View::ProjectsOverview => self.selected_project_index,
            View::ProjectDetail(_) => self.selected_branch_index,
            View::BranchDetail(_, _) => self.selected_session_index,
            View::ActivityTimeline => self.selected_timeline_index,
            View::SessionView => 0,
            View::LogViewer => self.log_viewer_scroll,
            View::FocusStats => self.focus_stats_selected_index,
        }
    }

    /// Set the selected index for the current view
    pub fn set_current_selected_index(&mut self, index: usize) {
        match self.view {
            View::ProjectsOverview => self.selected_project_index = index,
            View::ProjectDetail(_) => self.selected_branch_index = index,
            View::BranchDetail(_, _) => self.selected_session_index = index,
            View::ActivityTimeline => self.selected_timeline_index = index,
            View::SessionView => {}
            View::LogViewer => self.log_viewer_scroll = index,
            View::FocusStats => self.focus_stats_selected_index = index,
        }
    }

    /// Select the next item in the current view
    pub fn select_next(&mut self, item_count: usize) {
        if item_count > 0 {
            let current = self.current_selected_index();
            let next = (current + 1) % item_count;
            self.set_current_selected_index(next);
        }
    }

    /// Select the previous item in the current view
    pub fn select_prev(&mut self, item_count: usize) {
        if item_count > 0 {
            let current = self.current_selected_index();
            let prev = current.checked_sub(1).unwrap_or(item_count - 1);
            self.set_current_selected_index(prev);
        }
    }

    /// Select by number (1-indexed) in the current view
    pub fn select_by_number(&mut self, num: usize, item_count: usize) {
        if num > 0 && num <= item_count {
            self.set_current_selected_index(num - 1);
        }
    }

    /// Navigate to the parent view
    pub fn navigate_back(&mut self) {
        if let Some(parent) = self.view.parent() {
            self.view = parent;
        }
    }

    /// Navigate to a project detail view
    pub fn navigate_to_project(&mut self, project_id: ProjectId) {
        self.view = View::ProjectDetail(project_id);
        self.selected_branch_index = 0;
    }

    /// Navigate to a branch detail view
    pub fn navigate_to_branch(&mut self, project_id: ProjectId, branch_id: BranchId) {
        self.view = View::BranchDetail(project_id, branch_id);
        self.selected_session_index = 0;
    }

    /// Navigate to session view (auto-activates session mode)
    pub fn navigate_to_session(&mut self, session_id: SessionId) {
        // Remember where we came from
        self.session_return_view = Some(self.view);
        self.view = View::SessionView;
        self.active_session = Some(session_id);
        // Reset scroll offset when entering session view
        self.session_scroll_offset = 0;
        // Auto-activate session mode so keys go directly to PTY
        self.input_mode = InputMode::Session;
    }

    /// Navigate to activity timeline
    pub fn navigate_to_timeline(&mut self) {
        self.view = View::ActivityTimeline;
        self.selected_timeline_index = 0;
    }

    /// Return from session view to the previous view
    pub fn return_from_session(&mut self) {
        if let Some(return_view) = self.session_return_view.take() {
            self.view = return_view;
        } else {
            self.view = View::ProjectsOverview;
        }
        self.active_session = None;
        self.input_mode = InputMode::Normal;
    }
}

/// Filter branch refs by fuzzy substring match
fn filter_branch_refs(branch_refs: &[BranchRef], query: &str) -> Vec<BranchRef> {
    if query.is_empty() {
        return branch_refs.to_vec();
    }
    let query_lower = query.to_lowercase();
    branch_refs
        .iter()
        .filter(|b| b.name.to_lowercase().contains(&query_lower))
        .cloned()
        .collect()
}

/// Main application struct
pub struct App {
    /// Application configuration (used for project flows)
    #[allow(dead_code)]
    config: Config,
    /// Application state
    state: AppState,
    /// Project store for project/branch persistence
    project_store: ProjectStore,
    /// Session manager
    sessions: SessionManager,
    /// Hook event receiver
    hook_rx: HookEventReceiver,
    /// Hook server handle (kept alive and used for dropped events tracking)
    hook_server: ServerHandle,
    /// Terminal UI
    tui: Tui,
    /// Log buffer for real-time log viewing
    log_buffer: Arc<LogBuffer>,
    /// Information about the current log file
    log_file_info: LogFileInfo,
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
                        self.handle_key_event(key)?;
                        self.state.needs_render = true;
                    }
                    Event::Paste(text) => {
                        self.handle_paste_event(&text)?;
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

    /// Handle a key event
    fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
        // Handle Ctrl+C specially
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            // In Session mode, fall through to forward Ctrl+C to PTY
            if self.state.input_mode != InputMode::Session {
                // Show warning in all other modes
                self.state.error_message = Some("Ctrl+C disabled. Press 'q' to quit.".to_string());
                return Ok(());
            }
        }

        // Global: Jump to next session needing attention (Space key)
        // Only works in Normal mode (not in text input modes or Session mode)
        if key.code == KeyCode::Char(' ') && self.state.input_mode == InputMode::Normal {
            return self.jump_to_next_attention();
        }

        match self.state.input_mode {
            InputMode::Normal => self.handle_normal_mode_key(key),
            InputMode::Session => self.handle_session_mode_key(key),
            InputMode::CreatingSession => self.handle_creating_session_key(key),
            InputMode::AddingProject => self.handle_adding_project_key(key),
            InputMode::AddingProjectName => self.handle_adding_project_name_key(key),
            InputMode::FetchingBranches => {
                // While fetching, only allow Esc to cancel
                if key.code == KeyCode::Esc {
                    self.state.input_mode = InputMode::Normal;
                }
                Ok(())
            }
            InputMode::CreatingWorktree => {
                // Need to get project_id from current view
                if let View::ProjectDetail(project_id) = self.state.view {
                    self.handle_creating_worktree_key(key, project_id)
                } else {
                    Ok(())
                }
            }
            InputMode::SelectingDefaultBase => {
                // Need to get project_id from current view
                if let View::ProjectDetail(project_id) = self.state.view {
                    self.handle_selecting_default_base_key(key, project_id)
                } else {
                    Ok(())
                }
            }
            InputMode::ConfirmingSessionDelete => self.handle_confirming_delete_key(key),
            InputMode::ConfirmingBranchDelete => self.handle_confirming_branch_delete_key(key),
            InputMode::ConfirmingProjectDelete => self.handle_confirming_project_delete_key(key),
            InputMode::ConfirmingQuit => self.handle_confirming_quit_key(key),
            InputMode::RenamingProject => self.handle_renaming_project_key(key),
            InputMode::WorktreeSelectBranch => {
                if let View::ProjectDetail(project_id) = self.state.view {
                    self.handle_worktree_select_branch_key(key, project_id)
                } else {
                    Ok(())
                }
            }
            InputMode::WorktreeSelectBase => {
                if let View::ProjectDetail(project_id) = self.state.view {
                    self.handle_worktree_select_base_key(key, project_id)
                } else {
                    Ok(())
                }
            }
            InputMode::WorktreeConfirm => {
                if let View::ProjectDetail(project_id) = self.state.view {
                    self.handle_worktree_confirm_key(key, project_id)
                } else {
                    Ok(())
                }
            }
            InputMode::StartingFocusTimer => self.handle_starting_focus_timer_key(key),
        }
    }

    /// Handle key in normal mode
    fn handle_normal_mode_key(&mut self, key: KeyEvent) -> Result<()> {
        match self.state.view {
            View::ProjectsOverview => self.handle_projects_overview_key(key),
            View::ProjectDetail(_) => self.handle_project_detail_key(key),
            View::BranchDetail(project_id, branch_id) => {
                self.handle_branch_detail_key(key, project_id, branch_id)
            }
            View::SessionView => self.handle_session_view_normal_key(key),
            View::ActivityTimeline => self.handle_timeline_key(key),
            View::LogViewer => self.handle_log_viewer_key(key),
            View::FocusStats => self.handle_focus_stats_key(key),
        }
    }

    /// Handle key in projects overview (normal mode)
    fn handle_projects_overview_key(&mut self, key: KeyEvent) -> Result<()> {
        // Only process key press events (not release/repeat)
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        // Handle focus timer shortcuts (t, T, Ctrl+t)
        if self.handle_focus_timer_shortcut(key) {
            return Ok(());
        }

        let project_count = self.project_store.project_count();
        let session_count = self.sessions.len();
        let both_exist = project_count > 0 && session_count > 0;

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state.input_mode = InputMode::ConfirmingQuit;
            }
            KeyCode::Char('a') => {
                // Activity timeline
                self.state.navigate_to_timeline();
            }
            KeyCode::Char('n') => {
                // Start adding a new project
                self.state.input_mode = InputMode::AddingProject;
                self.state.new_project_path.clear();
            }
            KeyCode::Tab => {
                // Toggle focus between projects and sessions (only when both exist)
                if both_exist {
                    self.state.homepage_focus = match self.state.homepage_focus {
                        HomepageFocus::Projects => HomepageFocus::Sessions,
                        HomepageFocus::Sessions => HomepageFocus::Projects,
                    };
                }
            }
            KeyCode::Down => {
                // Navigate based on current focus
                if both_exist {
                    match self.state.homepage_focus {
                        HomepageFocus::Projects => {
                            self.state.selected_project_index =
                                (self.state.selected_project_index + 1) % project_count;
                        }
                        HomepageFocus::Sessions => {
                            self.state.selected_session_index =
                                (self.state.selected_session_index + 1) % session_count;
                        }
                    }
                } else if project_count > 0 {
                    self.state.selected_project_index =
                        (self.state.selected_project_index + 1) % project_count;
                } else if session_count > 0 {
                    self.state.selected_session_index =
                        (self.state.selected_session_index + 1) % session_count;
                }
            }
            KeyCode::Up => {
                // Navigate based on current focus
                if both_exist {
                    match self.state.homepage_focus {
                        HomepageFocus::Projects => {
                            self.state.selected_project_index = self
                                .state
                                .selected_project_index
                                .checked_sub(1)
                                .unwrap_or(project_count - 1);
                        }
                        HomepageFocus::Sessions => {
                            self.state.selected_session_index = self
                                .state
                                .selected_session_index
                                .checked_sub(1)
                                .unwrap_or(session_count - 1);
                        }
                    }
                } else if project_count > 0 {
                    self.state.selected_project_index = self
                        .state
                        .selected_project_index
                        .checked_sub(1)
                        .unwrap_or(project_count - 1);
                } else if session_count > 0 {
                    self.state.selected_session_index = self
                        .state
                        .selected_session_index
                        .checked_sub(1)
                        .unwrap_or(session_count - 1);
                }
            }
            KeyCode::Enter => {
                // Open selected project or session based on focus
                if both_exist {
                    match self.state.homepage_focus {
                        HomepageFocus::Projects => {
                            let projects = self.project_store.projects_sorted();
                            if let Some(project) = projects.get(self.state.selected_project_index) {
                                self.state.navigate_to_project(project.id);
                            }
                        }
                        HomepageFocus::Sessions => {
                            if let Some(session) = self
                                .sessions
                                .get_by_index(self.state.selected_session_index)
                            {
                                let session_id = session.info.id;
                                self.state.navigate_to_session(session_id);
                                self.sessions.acknowledge_attention(session_id);
                                if self.config.notification_method == "title" {
                                    SessionManager::reset_terminal_title();
                                }
                                self.resize_active_session_pty()?;
                            }
                        }
                    }
                } else if project_count > 0 {
                    let projects = self.project_store.projects_sorted();
                    if let Some(project) = projects.get(self.state.selected_project_index) {
                        self.state.navigate_to_project(project.id);
                    }
                } else if session_count > 0 {
                    if let Some(session) = self
                        .sessions
                        .get_by_index(self.state.selected_session_index)
                    {
                        let session_id = session.info.id;
                        self.state.navigate_to_session(session_id);
                        self.tui.enable_mouse_capture();
                        self.sessions.acknowledge_attention(session_id);
                        if self.config.notification_method == "title" {
                            SessionManager::reset_terminal_title();
                        }
                        self.resize_active_session_pty()?;
                    }
                }
            }
            KeyCode::Char('d') => {
                // Delete from currently focused list
                if both_exist {
                    match self.state.homepage_focus {
                        HomepageFocus::Projects => {
                            let projects = self.project_store.projects_sorted();
                            if let Some(project) = projects.get(self.state.selected_project_index) {
                                self.state.pending_delete_project = Some(project.id);
                                self.state.input_mode = InputMode::ConfirmingProjectDelete;
                            }
                        }
                        HomepageFocus::Sessions => {
                            if let Some(session) = self
                                .sessions
                                .get_by_index(self.state.selected_session_index)
                            {
                                let id = session.info.id;
                                if let Err(e) = self.sessions.destroy_session(id) {
                                    tracing::error!("Failed to destroy session: {}", e);
                                }
                                let new_count = self.sessions.len();
                                if self.state.selected_session_index >= new_count
                                    && self.state.selected_session_index > 0
                                {
                                    self.state.selected_session_index -= 1;
                                }
                            }
                        }
                    }
                } else if project_count > 0 {
                    // Only projects - delete selected project
                    let projects = self.project_store.projects_sorted();
                    if let Some(project) = projects.get(self.state.selected_project_index) {
                        self.state.pending_delete_project = Some(project.id);
                        self.state.input_mode = InputMode::ConfirmingProjectDelete;
                    }
                } else if session_count > 0 {
                    // Only sessions - delete selected session
                    if let Some(session) = self
                        .sessions
                        .get_by_index(self.state.selected_session_index)
                    {
                        let id = session.info.id;
                        if let Err(e) = self.sessions.destroy_session(id) {
                            tracing::error!("Failed to destroy session: {}", e);
                        }
                        let new_count = self.sessions.len();
                        if self.state.selected_session_index >= new_count
                            && self.state.selected_session_index > 0
                        {
                            self.state.selected_session_index -= 1;
                        }
                    }
                }
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                if let Some(num) = c.to_digit(10) {
                    if both_exist {
                        match self.state.homepage_focus {
                            HomepageFocus::Projects => {
                                if num > 0 && (num as usize) <= project_count {
                                    self.state.selected_project_index = (num as usize) - 1;
                                }
                            }
                            HomepageFocus::Sessions => {
                                if num > 0 && (num as usize) <= session_count {
                                    self.state.selected_session_index = (num as usize) - 1;
                                }
                            }
                        }
                    } else if project_count > 0 && num > 0 && (num as usize) <= project_count {
                        self.state.selected_project_index = (num as usize) - 1;
                    } else if project_count == 0 && num > 0 && (num as usize) <= session_count {
                        self.state.selected_session_index = (num as usize) - 1;
                    }
                }
            }
            KeyCode::Char('l') => {
                // Open log viewer
                self.state.view = View::LogViewer;
                self.state.log_viewer_scroll = 0;
                self.state.log_viewer_auto_scroll = true;
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key in project detail view (normal mode)
    fn handle_project_detail_key(&mut self, key: KeyEvent) -> Result<()> {
        // Only process key press events (not release/repeat)
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        // Handle focus timer shortcuts (t, T, Ctrl+t)
        if self.handle_focus_timer_shortcut(key) {
            return Ok(());
        }

        let project_id = match self.state.view {
            View::ProjectDetail(id) => id,
            _ => return Ok(()),
        };

        let branch_count = self.project_store.branches_for_project(project_id).len();

        match key.code {
            KeyCode::Esc => {
                self.state.navigate_back();
            }
            KeyCode::Char('q') => {
                self.state.input_mode = InputMode::ConfirmingQuit;
            }
            KeyCode::Down => {
                if branch_count > 0 {
                    self.state.selected_branch_index =
                        (self.state.selected_branch_index + 1) % branch_count;
                }
            }
            KeyCode::Up => {
                if branch_count > 0 {
                    self.state.selected_branch_index = self
                        .state
                        .selected_branch_index
                        .checked_sub(1)
                        .unwrap_or(branch_count - 1);
                }
            }
            KeyCode::Enter => {
                // Open selected branch
                let branches = self.project_store.branches_for_project_sorted(project_id);
                if let Some(branch) = branches.get(self.state.selected_branch_index) {
                    self.state.navigate_to_branch(project_id, branch.id);
                }
            }
            KeyCode::Char('n') => {
                // Start creating a new worktree (new wizard flow)
                self.start_worktree_wizard(project_id);
            }
            KeyCode::Char('b') => {
                // Set default base branch
                self.start_default_base_selection(project_id);
            }
            KeyCode::Char('d') => {
                // Delete selected branch/worktree
                let branches = self.project_store.branches_for_project_sorted(project_id);
                if let Some(branch) = branches.get(self.state.selected_branch_index) {
                    self.state.pending_delete_branch = Some(branch.id);
                    self.state.delete_worktree_on_disk = branch.is_worktree; // Default to deleting if it's a worktree
                    self.state.input_mode = InputMode::ConfirmingBranchDelete;
                }
            }
            KeyCode::Char('r') => {
                // Start renaming project
                if let Some(project) = self.project_store.get_project(project_id) {
                    self.state.new_project_name = project.name.clone();
                    self.state.renaming_project = Some(project_id);
                    self.state.input_mode = InputMode::RenamingProject;
                }
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                if let Some(num) = c.to_digit(10) {
                    if num > 0 && (num as usize) <= branch_count {
                        self.state.selected_branch_index = (num as usize) - 1;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key in branch detail view (normal mode)
    fn handle_branch_detail_key(
        &mut self,
        key: KeyEvent,
        project_id: ProjectId,
        branch_id: BranchId,
    ) -> Result<()> {
        // Only process key press events (not release/repeat)
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        // Handle focus timer shortcuts (t, T, Ctrl+t)
        if self.handle_focus_timer_shortcut(key) {
            return Ok(());
        }

        let branch_sessions = self.sessions.sessions_for_branch(branch_id);
        let session_count = branch_sessions.len();

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state.navigate_back();
            }
            KeyCode::Down => {
                self.state.select_next(session_count);
            }
            KeyCode::Up => {
                self.state.select_prev(session_count);
            }
            KeyCode::Enter => {
                let index = self.state.selected_session_index;
                if index < session_count {
                    let session_id = branch_sessions[index].info.id;
                    self.state.navigate_to_session(session_id);
                    self.tui.enable_mouse_capture();
                    self.sessions.acknowledge_attention(session_id);
                    if self.config.notification_method == "title" {
                        SessionManager::reset_terminal_title();
                    }
                    self.resize_active_session_pty()?;
                }
            }
            KeyCode::Char('n') => {
                // Prompt for session name before creating
                if let Some(branch) = self.project_store.get_branch(branch_id) {
                    self.state.creating_session_project_id = Some(project_id);
                    self.state.creating_session_branch_id = Some(branch_id);
                    self.state.creating_session_working_dir = Some(branch.working_dir.clone());
                    self.state.new_session_name.clear();
                    self.state.input_mode = InputMode::CreatingSession;
                }
            }
            KeyCode::Char('d') => {
                // Prompt for confirmation before deleting session
                if session_count > 0 {
                    let index = self.state.selected_session_index;
                    if index < session_count {
                        let session_id = branch_sessions[index].info.id;
                        self.state.pending_delete_session = Some(session_id);
                        self.state.input_mode = InputMode::ConfirmingSessionDelete;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key in activity timeline (normal mode)
    fn handle_timeline_key(&mut self, key: KeyEvent) -> Result<()> {
        // Only process key press events (not release/repeat)
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        // Handle focus timer shortcuts (t, T, Ctrl+t)
        if self.handle_focus_timer_shortcut(key) {
            return Ok(());
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state.navigate_back();
            }
            KeyCode::Down => {
                self.state.select_next(self.sessions.len());
            }
            KeyCode::Up => {
                self.state.select_prev(self.sessions.len());
            }
            KeyCode::Enter => {
                let index = self.state.current_selected_index();
                if let Some(session) = self.sessions.get_by_index(index) {
                    let session_id = session.info.id;
                    self.state.navigate_to_session(session_id);
                    self.tui.enable_mouse_capture();
                    self.sessions.acknowledge_attention(session_id);
                    if self.config.notification_method == "title" {
                        SessionManager::reset_terminal_title();
                    }
                    self.resize_active_session_pty()?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key in log viewer (normal mode)
    fn handle_log_viewer_key(&mut self, key: KeyEvent) -> Result<()> {
        // Only process key press events (not release/repeat)
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        // Handle focus timer shortcuts (t, T, Ctrl+t)
        if self.handle_focus_timer_shortcut(key) {
            return Ok(());
        }

        let entry_count = self.log_buffer.len();

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                // Go back to projects overview
                self.state.view = View::ProjectsOverview;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                // Scroll down (disable auto-scroll)
                self.state.log_viewer_auto_scroll = false;
                if self.state.log_viewer_scroll < entry_count.saturating_sub(1) {
                    self.state.log_viewer_scroll += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                // Scroll up (disable auto-scroll)
                self.state.log_viewer_auto_scroll = false;
                self.state.log_viewer_scroll = self.state.log_viewer_scroll.saturating_sub(1);
            }
            KeyCode::Char('g') => {
                // Jump to top (disable auto-scroll)
                self.state.log_viewer_auto_scroll = false;
                self.state.log_viewer_scroll = 0;
            }
            KeyCode::Char('G') => {
                // Jump to bottom and enable auto-scroll
                self.state.log_viewer_auto_scroll = true;
                self.state.log_viewer_scroll = entry_count.saturating_sub(1);
            }
            KeyCode::PageDown => {
                // Page down (disable auto-scroll)
                self.state.log_viewer_auto_scroll = false;
                self.state.log_viewer_scroll =
                    (self.state.log_viewer_scroll + 20).min(entry_count.saturating_sub(1));
            }
            KeyCode::PageUp => {
                // Page up (disable auto-scroll)
                self.state.log_viewer_auto_scroll = false;
                self.state.log_viewer_scroll = self.state.log_viewer_scroll.saturating_sub(20);
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key in focus stats view (normal mode)
    fn handle_focus_stats_key(&mut self, key: KeyEvent) -> Result<()> {
        // Only process key press events (not release/repeat)
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        // Handle focus timer shortcuts (t, T, Ctrl+t)
        if self.handle_focus_timer_shortcut(key) {
            return Ok(());
        }

        let session_count = self.state.focus_sessions.len();

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state.navigate_back();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.select_next(session_count);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.select_prev(session_count);
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key when starting a focus timer (entering duration)
    fn handle_starting_focus_timer_key(&mut self, key: KeyEvent) -> Result<()> {
        // Only process key press events (not release/repeat)
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        match key.code {
            KeyCode::Esc => {
                // Cancel timer start
                self.state.input_mode = InputMode::Normal;
                self.state.focus_timer_input.clear();
            }
            KeyCode::Enter => {
                // Start the timer with entered duration (or default)
                let minutes = if self.state.focus_timer_input.is_empty() {
                    self.config.focus_timer_minutes
                } else {
                    self.state
                        .focus_timer_input
                        .parse()
                        .unwrap_or(self.config.focus_timer_minutes)
                };

                self.start_focus_timer(minutes);
                self.state.input_mode = InputMode::Normal;
                self.state.focus_timer_input.clear();
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                // Only allow digits
                if self.state.focus_timer_input.len() < 3 {
                    self.state.focus_timer_input.push(c);
                }
            }
            KeyCode::Backspace => {
                self.state.focus_timer_input.pop();
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle common focus timer shortcuts. Returns true if the key was handled.
    fn handle_focus_timer_shortcut(&mut self, key: KeyEvent) -> bool {
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
    fn start_focus_timer(&mut self, minutes: u64) {
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
                self.save_focus_session(
                    target,
                    focused,
                    elapsed,
                    timer.project_id,
                    timer.branch_id,
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
                self.save_focus_session(
                    target,
                    focused_duration,
                    elapsed,
                    timer.project_id,
                    timer.branch_id,
                );
            }
        }
    }

    /// Load focus sessions from disk into state
    fn load_focus_sessions(&mut self) {
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
    ) {
        // Save the session
        let session = FocusSession::from_timer_result(
            target_duration,
            focused_duration,
            elapsed_duration,
            project_id,
            branch_id,
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

    /// Handle key in session view (normal mode)
    fn handle_session_view_normal_key(&mut self, key: KeyEvent) -> Result<()> {
        // Only process key press events (not release/repeat)
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        // Handle focus timer shortcuts (t, T, Ctrl+t) - only in Normal mode
        if self.handle_focus_timer_shortcut(key) {
            return Ok(());
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                // Go back to the view we came from
                self.state.return_from_session();
                // Re-enable mouse capture when leaving session view
                self.tui.enable_mouse_capture();
            }
            KeyCode::Enter => {
                // Re-activate session mode (send keys to PTY)
                self.state.input_mode = InputMode::Session;
                // Re-enable mouse capture for scroll wheel
                self.tui.enable_mouse_capture();
            }
            KeyCode::PageUp => {
                // Scroll up in session output (toward older content)
                if let Some(session_id) = self.state.active_session {
                    if let Some(session) = self.sessions.get_mut(session_id) {
                        // Calculate viewport height for scroll amount
                        let terminal_size = self.tui.size().unwrap_or_default();
                        let frame_config = FrameConfig::default();
                        let layout = FrameLayout::calculate(terminal_size, &frame_config);
                        let viewport_height = layout.content.height as usize;
                        let max_scroll = session.vterm.max_scrollback();

                        // Update app-level scroll offset with constraints
                        self.state.session_scroll_offset = self
                            .state
                            .session_scroll_offset
                            .saturating_add(viewport_height)
                            .min(max_scroll);
                        session
                            .vterm
                            .set_scrollback(self.state.session_scroll_offset);
                    }
                }
            }
            KeyCode::PageDown => {
                // Scroll down in session output (toward newer content)
                if let Some(session_id) = self.state.active_session {
                    if let Some(session) = self.sessions.get_mut(session_id) {
                        // Calculate viewport height for scroll amount
                        let terminal_size = self.tui.size().unwrap_or_default();
                        let frame_config = FrameConfig::default();
                        let layout = FrameLayout::calculate(terminal_size, &frame_config);
                        let viewport_height = layout.content.height as usize;

                        // Update app-level scroll offset
                        self.state.session_scroll_offset = self
                            .state
                            .session_scroll_offset
                            .saturating_sub(viewport_height);
                        session
                            .vterm
                            .set_scrollback(self.state.session_scroll_offset);
                    }
                }
            }
            KeyCode::Home => {
                // Scroll to top (oldest content)
                if let Some(session_id) = self.state.active_session {
                    if let Some(session) = self.sessions.get_mut(session_id) {
                        let max_scroll = session.vterm.max_scrollback();
                        self.state.session_scroll_offset = max_scroll;
                        session
                            .vterm
                            .set_scrollback(self.state.session_scroll_offset);
                    }
                }
            }
            KeyCode::End => {
                // Scroll to bottom (live view)
                if let Some(session_id) = self.state.active_session {
                    if let Some(session) = self.sessions.get_mut(session_id) {
                        self.state.session_scroll_offset = 0;
                        session.vterm.scroll_to_bottom();
                    }
                }
            }
            KeyCode::Tab => {
                // Switch to next session
                let count = self.sessions.len();
                if count > 0 {
                    // Use the timeline index for cycling through all sessions
                    self.state.selected_timeline_index =
                        (self.state.selected_timeline_index + 1) % count;
                    if let Some(session) = self
                        .sessions
                        .get_by_index(self.state.selected_timeline_index)
                    {
                        let session_id = session.info.id;
                        self.state.active_session = Some(session_id);
                        // Reset scroll offset when switching sessions
                        self.state.session_scroll_offset = 0;
                        self.sessions.acknowledge_attention(session_id);
                        if self.config.notification_method == "title" {
                            SessionManager::reset_terminal_title();
                        }
                        self.resize_active_session_pty()?;
                    }
                }
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                // Jump to session by number
                if let Some(num) = c.to_digit(10) {
                    let count = self.sessions.len();
                    if num > 0 && (num as usize) <= count {
                        self.state.selected_timeline_index = (num as usize) - 1;
                        if let Some(session) = self
                            .sessions
                            .get_by_index(self.state.selected_timeline_index)
                        {
                            let session_id = session.info.id;
                            self.state.active_session = Some(session_id);
                            // Reset scroll offset when switching sessions
                            self.state.session_scroll_offset = 0;
                            self.sessions.acknowledge_attention(session_id);
                            if self.config.notification_method == "title" {
                                SessionManager::reset_terminal_title();
                            }
                            self.resize_active_session_pty()?;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key in session mode (keys go to PTY)
    fn handle_session_mode_key(&mut self, key: KeyEvent) -> Result<()> {
        // Handle Esc key
        if key.code == KeyCode::Esc {
            return self.handle_session_mode_esc(key);
        }

        // Only forward key press events (not release/repeat)
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        // Intercept scroll keys - don't forward to PTY
        match key.code {
            KeyCode::PageUp => {
                if let Some(session_id) = self.state.active_session {
                    if let Some(session) = self.sessions.get_mut(session_id) {
                        // Calculate viewport height for scroll amount
                        let terminal_size = self.tui.size().unwrap_or_default();
                        let frame_config = FrameConfig::default();
                        let layout = FrameLayout::calculate(terminal_size, &frame_config);
                        let viewport_height = layout.content.height as usize;
                        let max_scroll = session.vterm.max_scrollback();

                        // Update app-level scroll offset with constraints
                        self.state.session_scroll_offset = self
                            .state
                            .session_scroll_offset
                            .saturating_add(viewport_height)
                            .min(max_scroll);
                        session
                            .vterm
                            .set_scrollback(self.state.session_scroll_offset);
                    }
                }
                return Ok(());
            }
            KeyCode::PageDown => {
                if let Some(session_id) = self.state.active_session {
                    if let Some(session) = self.sessions.get_mut(session_id) {
                        // Calculate viewport height for scroll amount
                        let terminal_size = self.tui.size().unwrap_or_default();
                        let frame_config = FrameConfig::default();
                        let layout = FrameLayout::calculate(terminal_size, &frame_config);
                        let viewport_height = layout.content.height as usize;

                        // Update app-level scroll offset
                        self.state.session_scroll_offset = self
                            .state
                            .session_scroll_offset
                            .saturating_sub(viewport_height);
                        session
                            .vterm
                            .set_scrollback(self.state.session_scroll_offset);
                    }
                }
                return Ok(());
            }
            KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+Home: scroll to top
                if let Some(session_id) = self.state.active_session {
                    if let Some(session) = self.sessions.get_mut(session_id) {
                        let max_scroll = session.vterm.max_scrollback();
                        self.state.session_scroll_offset = max_scroll;
                        session
                            .vterm
                            .set_scrollback(self.state.session_scroll_offset);
                    }
                }
                return Ok(());
            }
            KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+End: scroll to bottom (live view)
                if let Some(session_id) = self.state.active_session {
                    if let Some(session) = self.sessions.get_mut(session_id) {
                        self.state.session_scroll_offset = 0;
                        session.vterm.scroll_to_bottom();
                    }
                }
                return Ok(());
            }
            _ => {}
        }

        // Reset scroll to live view when typing
        if self.state.session_scroll_offset > 0 {
            self.state.session_scroll_offset = 0;
            if let Some(session_id) = self.state.active_session {
                if let Some(session) = self.sessions.get_mut(session_id) {
                    session.vterm.scroll_to_bottom();
                }
            }
        }

        // Send key to active session
        if let Some(session_id) = self.state.active_session {
            if let Some(session) = self.sessions.get_mut(session_id) {
                session.send_key(key)?;
            }
        }
        Ok(())
    }

    /// Handle Esc key in session mode
    fn handle_session_mode_esc(&mut self, key: KeyEvent) -> Result<()> {
        // Only handle key press events
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        if key.modifiers.contains(KeyModifiers::ALT) {
            // Option/Alt+Escape: forward Escape to Claude Code
            self.forward_esc_to_pty()?;
        } else {
            // Plain Escape: deactivate session mode (switch to Normal), stay in SessionView
            self.state.input_mode = InputMode::Normal;
            // Disable mouse capture so user can select and copy text
            self.tui.disable_mouse_capture();
        }
        Ok(())
    }

    /// Forward an Esc key press to the active session's PTY
    fn forward_esc_to_pty(&mut self) -> Result<()> {
        if let Some(session_id) = self.state.active_session {
            if let Some(session) = self.sessions.get_mut(session_id) {
                let esc_key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
                session.send_key(esc_key)?;
            }
        }
        Ok(())
    }

    /// Handle key while creating a new session
    fn handle_creating_session_key(&mut self, key: KeyEvent) -> Result<()> {
        // Only process key press events (not release/repeat)
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        match key.code {
            KeyCode::Esc => {
                // Cancel session creation
                self.state.input_mode = InputMode::Normal;
                self.state.new_session_name.clear();
                self.state.creating_session_project_id = None;
                self.state.creating_session_branch_id = None;
                self.state.creating_session_working_dir = None;
            }
            KeyCode::Enter => {
                // Create the session
                let name = if self.state.new_session_name.is_empty() {
                    format!("Session {}", self.sessions.len() + 1)
                } else {
                    std::mem::take(&mut self.state.new_session_name)
                };

                // Use context working directory, or current directory as fallback
                let working_dir = self
                    .state
                    .creating_session_working_dir
                    .take()
                    .unwrap_or_else(|| {
                        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
                    });

                // Get project/branch context (nil if unassociated)
                let project_id = self
                    .state
                    .creating_session_project_id
                    .take()
                    .unwrap_or(Uuid::nil());
                let branch_id = self
                    .state
                    .creating_session_branch_id
                    .take()
                    .unwrap_or(Uuid::nil());

                // Get terminal dimensions for the session
                let (rows, cols) = if let Ok(size) = self.tui.size() {
                    (
                        size.height.saturating_sub(8) as usize,
                        size.width.saturating_sub(2) as usize,
                    )
                } else {
                    (24, 80) // Fallback dimensions
                };

                match self.sessions.create_session(
                    name.clone(),
                    working_dir,
                    project_id,
                    branch_id,
                    None,
                    rows,
                    cols,
                ) {
                    Ok(session_id) => {
                        tracing::info!("Created session: {} ({})", name, session_id);

                        // Update project/branch activity timestamps if associated
                        if !project_id.is_nil() {
                            if let Some(project) = self.project_store.get_project_mut(project_id) {
                                project.touch();
                            }
                        }
                        if !branch_id.is_nil() {
                            if let Some(branch) = self.project_store.get_branch_mut(branch_id) {
                                branch.touch();
                            }
                        }

                        // Navigate to the new session (auto-activates Session mode)
                        self.state.navigate_to_session(session_id);
                        self.tui.enable_mouse_capture();
                        self.sessions.acknowledge_attention(session_id);
                        if self.config.notification_method == "title" {
                            SessionManager::reset_terminal_title();
                        }
                        self.resize_active_session_pty()?;
                    }
                    Err(e) => {
                        tracing::error!("Failed to create session: {}", e);
                        // Only reset to Normal mode on failure
                        self.state.input_mode = InputMode::Normal;
                    }
                }
            }
            KeyCode::Backspace => {
                self.state.new_session_name.pop();
            }
            KeyCode::Char(c) => {
                self.state.new_session_name.push(c);
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key when adding a new project (path input step)
    fn handle_adding_project_key(&mut self, key: KeyEvent) -> Result<()> {
        // Only process key press events (not release/repeat)
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        match key.code {
            KeyCode::Esc => {
                if self.state.show_path_completions {
                    // First Esc hides completions
                    self.clear_path_completions();
                } else {
                    // Second Esc cancels input
                    self.state.input_mode = InputMode::Normal;
                    self.state.new_project_path.clear();
                    self.clear_path_completions();
                }
            }
            KeyCode::Tab => {
                if self.state.show_path_completions && !self.state.path_completions.is_empty() {
                    // Apply selected completion (standard shell behavior)
                    self.apply_path_completion();
                } else {
                    // Show completions
                    self.update_path_completions();
                }
            }
            KeyCode::BackTab => {
                // Cycle backward through completions
                if self.state.show_path_completions {
                    let count = self.state.path_completions.len();
                    if count > 0 {
                        self.state.path_completion_index = self
                            .state
                            .path_completion_index
                            .checked_sub(1)
                            .unwrap_or(count - 1);
                    }
                }
            }
            KeyCode::Up => {
                // Navigate up in completions
                if self.state.show_path_completions {
                    let count = self.state.path_completions.len();
                    if count > 0 {
                        self.state.path_completion_index = self
                            .state
                            .path_completion_index
                            .checked_sub(1)
                            .unwrap_or(count - 1);
                    }
                }
            }
            KeyCode::Down => {
                // Navigate down in completions
                if self.state.show_path_completions {
                    let count = self.state.path_completions.len();
                    if count > 0 {
                        self.state.path_completion_index =
                            (self.state.path_completion_index + 1) % count;
                    }
                }
            }
            KeyCode::Enter => {
                // Always validate path and transition to name input
                self.clear_path_completions();
                let path_str = std::mem::take(&mut self.state.new_project_path);
                let user_path = PathBuf::from(shellexpand::tilde(&path_str).into_owned());
                let user_path = user_path.canonicalize().unwrap_or(user_path);

                // Check if it's a git repository
                match crate::git::GitOps::discover(&user_path) {
                    Ok(git) => {
                        let repo_path = git.repo_path().to_path_buf();
                        let repo_path = repo_path.canonicalize().unwrap_or(repo_path);

                        // Calculate session_subdir if user_path is inside repo_path
                        let session_subdir = if user_path != repo_path {
                            user_path
                                .strip_prefix(&repo_path)
                                .ok()
                                .map(|p| p.to_path_buf())
                        } else {
                            None
                        };

                        // Check if already added (with same subdir)
                        if self
                            .project_store
                            .find_by_repo_and_subdir(&repo_path, session_subdir.as_deref())
                            .is_some()
                        {
                            let path_display = if let Some(ref subdir) = session_subdir {
                                format!("{}/{}", repo_path.display(), subdir.display())
                            } else {
                                repo_path.display().to_string()
                            };
                            self.state.error_message =
                                Some(format!("Project already exists: {}", path_display));
                            tracing::warn!("Project already exists: {}", path_display);
                            self.state.input_mode = InputMode::Normal;
                            return Ok(());
                        }

                        // Get default branch
                        let default_branch = git
                            .default_branch_name()
                            .unwrap_or_else(|_| "main".to_string());

                        // Compute default project name from subdir folder or repo folder
                        let default_name = session_subdir
                            .as_ref()
                            .and_then(|s| s.file_name())
                            .or_else(|| repo_path.file_name())
                            .and_then(|s| s.to_str())
                            .unwrap_or("unknown")
                            .to_string();

                        // Store pending values and transition to name input
                        self.state.pending_project_path = repo_path;
                        self.state.pending_session_subdir = session_subdir;
                        self.state.pending_default_branch = default_branch;
                        self.state.new_project_name = default_name;
                        self.state.input_mode = InputMode::AddingProjectName;
                    }
                    Err(e) => {
                        self.state.error_message =
                            Some(format!("Not a git repository: {}", user_path.display()));
                        tracing::error!("Not a git repository: {} ({})", user_path.display(), e);
                        self.state.input_mode = InputMode::Normal;
                    }
                }
            }
            KeyCode::Backspace => {
                self.state.new_project_path.pop();
                self.update_path_completions();
            }
            KeyCode::Char(c) => {
                self.state.new_project_path.push(c);
                self.update_path_completions();
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key when entering project name (second step of project addition)
    fn handle_adding_project_name_key(&mut self, key: KeyEvent) -> Result<()> {
        // Only process key press events (not release/repeat)
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        match key.code {
            KeyCode::Esc => {
                // Cancel project addition entirely
                self.state.input_mode = InputMode::Normal;
                self.state.new_project_name.clear();
                self.state.new_project_path.clear();
                self.state.pending_project_path = PathBuf::new();
                self.state.pending_session_subdir = None;
                self.state.pending_default_branch.clear();
            }
            KeyCode::Enter => {
                // Create project with custom (or default) name
                let name = if self.state.new_project_name.trim().is_empty() {
                    // Use folder name as fallback
                    self.state
                        .pending_project_path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string()
                } else {
                    std::mem::take(&mut self.state.new_project_name)
                        .trim()
                        .to_string()
                };

                let repo_path = std::mem::take(&mut self.state.pending_project_path);
                let session_subdir = self.state.pending_session_subdir.take();
                let default_branch = std::mem::take(&mut self.state.pending_default_branch);

                // Create project
                let mut project = crate::project::Project::new(
                    name.clone(),
                    repo_path.clone(),
                    default_branch.clone(),
                );
                project.session_subdir = session_subdir;
                let project_id = project.id;
                self.project_store.add_project(project);

                // Create default branch entry with effective working dir
                let effective_working_dir = self
                    .project_store
                    .get_project(project_id)
                    .map(|p| p.effective_working_dir(&repo_path))
                    .unwrap_or(repo_path);

                let branch = crate::project::Branch::default_for_project(
                    project_id,
                    default_branch,
                    effective_working_dir,
                );
                self.project_store.add_branch(branch);

                // Save to disk
                if let Err(e) = self.project_store.save() {
                    tracing::error!("Failed to save project store: {}", e);
                    self.state.error_message = Some(format!("Failed to save project: {}", e));
                }

                tracing::info!("Added project: {}", name);

                // Select the newly added project
                let project_count = self.project_store.project_count();
                self.state.selected_project_index = project_count.saturating_sub(1);

                self.state.input_mode = InputMode::Normal;
            }
            KeyCode::Backspace => {
                self.state.new_project_name.pop();
            }
            KeyCode::Char(c) => {
                self.state.new_project_name.push(c);
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key when creating a new worktree
    ///
    /// New flow:
    /// - Type branch name to create NEW branch (leave empty to checkout existing)
    /// - Navigate list to select base branch (for new) or target branch (for checkout)
    /// - Press 's' to set current selection as default base
    fn handle_creating_worktree_key(&mut self, key: KeyEvent, project_id: ProjectId) -> Result<()> {
        // Only process key press events (not release/repeat)
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        match key.code {
            KeyCode::Esc => {
                // Cancel worktree creation
                self.state.input_mode = InputMode::Normal;
                self.state.new_branch_name.clear();
                self.state.available_branch_refs.clear();
                self.state.filtered_branch_refs.clear();
                self.state.selected_base_branch = None;
                self.state.fetch_error = None;
            }
            KeyCode::Up => {
                // Navigate up (wrapping)
                let count = self.state.filtered_branch_refs.len();
                if count > 0 {
                    self.state.base_branch_selector_index = self
                        .state
                        .base_branch_selector_index
                        .checked_sub(1)
                        .unwrap_or(count - 1);
                    // Update selected_base_branch when navigating
                    self.state.selected_base_branch = self
                        .state
                        .filtered_branch_refs
                        .get(self.state.base_branch_selector_index)
                        .cloned();
                }
            }
            KeyCode::Down => {
                // Navigate down (wrapping)
                let count = self.state.filtered_branch_refs.len();
                if count > 0 {
                    self.state.base_branch_selector_index =
                        (self.state.base_branch_selector_index + 1) % count;
                    // Update selected_base_branch when navigating
                    self.state.selected_base_branch = self
                        .state
                        .filtered_branch_refs
                        .get(self.state.base_branch_selector_index)
                        .cloned();
                }
            }
            KeyCode::Char('s') if key.modifiers.is_empty() => {
                // Set current selection as default base branch
                if let Some(selected) = self
                    .state
                    .filtered_branch_refs
                    .get(self.state.base_branch_selector_index)
                {
                    let branch_name = selected.name.clone();
                    if let Some(project) = self.project_store.get_project_mut(project_id) {
                        project.set_default_base_branch(Some(branch_name.clone()));
                        if let Err(e) = self.project_store.save() {
                            tracing::error!("Failed to save default base branch: {}", e);
                            self.state.error_message =
                                Some(format!("Failed to save default: {}", e));
                        } else {
                            // Update the is_default_base flags in our list
                            for branch_ref in &mut self.state.available_branch_refs {
                                branch_ref.is_default_base = branch_ref.name == branch_name;
                            }
                            self.state.filtered_branch_refs = filter_branch_refs(
                                &self.state.available_branch_refs,
                                &self.state.new_branch_name,
                            );
                            tracing::debug!("Set default base branch to: {}", branch_name);
                        }
                    }
                }
            }
            KeyCode::Enter => {
                let branch_name_typed = std::mem::take(&mut self.state.new_branch_name);
                let selected_idx = self.state.base_branch_selector_index;
                let selected_branch = self.state.filtered_branch_refs.get(selected_idx).cloned();

                let result: Result<()> = if !branch_name_typed.is_empty() {
                    // Create NEW branch from selected base, then create worktree
                    // Use selected_base_branch which is preserved even when filtered out
                    let base_ref = self
                        .state
                        .selected_base_branch
                        .as_ref()
                        .map(|b| b.name.clone());
                    self.create_worktree(project_id, &branch_name_typed, true, base_ref.as_deref())
                        .map(|_| ())
                } else if let Some(selected) = selected_branch {
                    // Checkout existing branch as worktree (empty name = checkout selected)
                    // For local branches, just create worktree
                    // For remote branches, need to create tracking branch first
                    let branch_name = if selected.ref_type == BranchRefType::Remote {
                        // Extract branch name from remote ref (e.g., "origin/feature" -> "feature")
                        selected
                            .name
                            .split_once('/')
                            .map(|(_, b)| b.to_string())
                            .unwrap_or(selected.name.clone())
                    } else {
                        selected.name.clone()
                    };

                    // For remote branches, we create a new local branch tracking it
                    let create_branch = selected.ref_type == BranchRefType::Remote;
                    let base_ref = if create_branch {
                        Some(selected.name.as_str())
                    } else {
                        None
                    };

                    self.create_worktree(project_id, &branch_name, create_branch, base_ref)
                        .map(|_| ())
                } else {
                    Ok(())
                };

                if let Err(e) = result {
                    tracing::error!("Failed to create worktree: {}", e);
                    self.state.error_message = Some(format!("Failed to create worktree: {}", e));
                }
                self.state.input_mode = InputMode::Normal;
                self.state.available_branch_refs.clear();
                self.state.filtered_branch_refs.clear();
                self.state.selected_base_branch = None;
                self.state.fetch_error = None;
            }
            KeyCode::Backspace => {
                self.state.new_branch_name.pop();
                self.state.filtered_branch_refs = filter_branch_refs(
                    &self.state.available_branch_refs,
                    &self.state.new_branch_name,
                );
                // Find and select the default base branch if exists
                self.select_default_base_branch();
            }
            KeyCode::Char(c) => {
                self.state.new_branch_name.push(c);
                self.state.filtered_branch_refs = filter_branch_refs(
                    &self.state.available_branch_refs,
                    &self.state.new_branch_name,
                );
                // Find and select the default base branch if exists
                self.select_default_base_branch();
            }
            _ => {}
        }
        Ok(())
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

    /// Clear path completion state
    fn clear_path_completions(&mut self) {
        self.state.path_completions.clear();
        self.state.path_completion_index = 0;
        self.state.show_path_completions = false;
    }

    /// Apply the selected completion to the input field
    fn apply_path_completion(&mut self) {
        if let Some(path) = self
            .state
            .path_completions
            .get(self.state.path_completion_index)
        {
            self.state.new_project_path = crate::path_complete::path_to_input(path);
            // After applying, refresh completions for the new path
            self.update_path_completions();
        }
    }

    /// Handle key when selecting default base branch (via 'b' in project view)
    fn handle_selecting_default_base_key(
        &mut self,
        key: KeyEvent,
        project_id: ProjectId,
    ) -> Result<()> {
        // Only process key press events (not release/repeat)
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        match key.code {
            KeyCode::Esc => {
                // Cancel selection
                self.state.input_mode = InputMode::Normal;
                self.state.available_branch_refs.clear();
                self.state.filtered_branch_refs.clear();
                self.state.new_branch_name.clear();
                self.state.fetch_error = None;
            }
            KeyCode::Up => {
                let count = self.state.filtered_branch_refs.len();
                if count > 0 {
                    self.state.base_branch_selector_index = self
                        .state
                        .base_branch_selector_index
                        .checked_sub(1)
                        .unwrap_or(count - 1);
                }
            }
            KeyCode::Down => {
                let count = self.state.filtered_branch_refs.len();
                if count > 0 {
                    self.state.base_branch_selector_index =
                        (self.state.base_branch_selector_index + 1) % count;
                }
            }
            KeyCode::Enter => {
                // Set selected branch as default base
                if let Some(selected) = self
                    .state
                    .filtered_branch_refs
                    .get(self.state.base_branch_selector_index)
                {
                    let branch_name = selected.name.clone();
                    if let Some(project) = self.project_store.get_project_mut(project_id) {
                        project.set_default_base_branch(Some(branch_name.clone()));
                        if let Err(e) = self.project_store.save() {
                            tracing::error!("Failed to save default base branch: {}", e);
                            self.state.error_message =
                                Some(format!("Failed to save default: {}", e));
                        } else {
                            tracing::debug!("Set default base branch to: {}", branch_name);
                        }
                    }
                }
                self.state.input_mode = InputMode::Normal;
                self.state.available_branch_refs.clear();
                self.state.filtered_branch_refs.clear();
                self.state.new_branch_name.clear();
                self.state.fetch_error = None;
            }
            KeyCode::Backspace => {
                self.state.new_branch_name.pop();
                self.state.filtered_branch_refs = filter_branch_refs(
                    &self.state.available_branch_refs,
                    &self.state.new_branch_name,
                );
                self.select_default_base_branch();
            }
            KeyCode::Char(c) => {
                self.state.new_branch_name.push(c);
                self.state.filtered_branch_refs = filter_branch_refs(
                    &self.state.available_branch_refs,
                    &self.state.new_branch_name,
                );
                self.select_default_base_branch();
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key when confirming session deletion
    fn handle_confirming_delete_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                // Confirm deletion
                if let Some(session_id) = self.state.pending_delete_session.take() {
                    // Get branch_id before destroying (for selection adjustment)
                    let branch_id = self.state.view.branch_id();

                    if let Err(e) = self.sessions.destroy_session(session_id) {
                        tracing::error!("Failed to destroy session: {}", e);
                    }

                    // Adjust selection if needed
                    if let Some(branch_id) = branch_id {
                        let new_count = self.sessions.sessions_for_branch(branch_id).len();
                        if self.state.selected_session_index >= new_count && new_count > 0 {
                            self.state.selected_session_index = new_count - 1;
                        }
                    }
                }
                self.state.input_mode = InputMode::Normal;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                // Cancel deletion
                self.state.pending_delete_session = None;
                self.state.input_mode = InputMode::Normal;
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key when confirming branch/worktree deletion
    fn handle_confirming_branch_delete_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        match key.code {
            KeyCode::Char('w') => {
                // Toggle worktree deletion option
                self.state.delete_worktree_on_disk = !self.state.delete_worktree_on_disk;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                // Confirm deletion
                if let Some(branch_id) = self.state.pending_delete_branch.take() {
                    // Get branch info before removing
                    let branch_info = self.project_store.get_branch(branch_id).cloned();

                    // Destroy all sessions for this branch
                    let sessions_to_destroy: Vec<_> = self
                        .sessions
                        .sessions_for_branch(branch_id)
                        .iter()
                        .map(|s| s.info.id)
                        .collect();

                    for session_id in sessions_to_destroy {
                        if let Err(e) = self.sessions.destroy_session(session_id) {
                            tracing::error!("Failed to destroy session: {}", e);
                        }
                    }

                    // If user opted to delete worktree on disk
                    if self.state.delete_worktree_on_disk {
                        if let Some(branch) = &branch_info {
                            if branch.is_worktree {
                                // Get the project to access the repo
                                if let Some(project_id) = self.state.view.project_id() {
                                    // Clone the repo_path to avoid borrow conflicts
                                    let repo_path = self
                                        .project_store
                                        .get_project(project_id)
                                        .map(|p| p.repo_path.clone());

                                    if let Some(repo_path) = repo_path {
                                        // Show loading indicator
                                        let _ = self.show_loading(&format!(
                                            "Removing worktree '{}'...",
                                            branch.name
                                        ));

                                        match crate::git::GitOps::open(&repo_path) {
                                            Ok(git) => {
                                                if let Err(e) =
                                                    crate::git::worktree::remove_worktree(
                                                        git.repository(),
                                                        &branch.name,
                                                        true,
                                                    )
                                                {
                                                    tracing::error!(
                                                        "Failed to remove worktree: {}",
                                                        e
                                                    );
                                                    self.state.error_message = Some(format!(
                                                        "Failed to remove worktree: {}",
                                                        e
                                                    ));
                                                } else {
                                                    tracing::info!(
                                                        "Removed worktree for branch: {}",
                                                        branch.name
                                                    );
                                                }
                                            }
                                            Err(e) => {
                                                tracing::error!("Failed to open git repo: {}", e);
                                                self.state.error_message =
                                                    Some(format!("Failed to open git repo: {}", e));
                                            }
                                        }

                                        self.clear_loading();
                                    }
                                }
                            }
                        }
                    }

                    // Remove branch from the store
                    self.project_store.remove_branch(branch_id);

                    // Save to disk
                    if let Err(e) = self.project_store.save() {
                        tracing::error!("Failed to save project store: {}", e);
                        self.state.error_message =
                            Some(format!("Failed to save project store: {}", e));
                    }

                    tracing::info!("Deleted branch: {}", branch_id);

                    // Adjust selected index if needed
                    if let Some(project_id) = self.state.view.project_id() {
                        let new_count = self.project_store.branches_for_project(project_id).len();
                        if self.state.selected_branch_index >= new_count && new_count > 0 {
                            self.state.selected_branch_index = new_count - 1;
                        } else if new_count == 0 {
                            self.state.selected_branch_index = 0;
                        }
                    }
                }
                self.state.delete_worktree_on_disk = false;
                self.state.input_mode = InputMode::Normal;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                // Cancel deletion
                self.state.pending_delete_branch = None;
                self.state.delete_worktree_on_disk = false;
                self.state.input_mode = InputMode::Normal;
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key when confirming project deletion
    fn handle_confirming_project_delete_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                // Confirm deletion
                if let Some(project_id) = self.state.pending_delete_project.take() {
                    // Destroy all sessions for this project
                    let sessions_to_destroy: Vec<_> = self
                        .sessions
                        .sessions_for_project(project_id)
                        .iter()
                        .map(|s| s.info.id)
                        .collect();

                    for session_id in sessions_to_destroy {
                        if let Err(e) = self.sessions.destroy_session(session_id) {
                            tracing::error!("Failed to destroy session: {}", e);
                        }
                    }

                    // Remove project and its branches from the store
                    self.project_store.remove_project(project_id);

                    // Save to disk
                    if let Err(e) = self.project_store.save() {
                        tracing::error!("Failed to save project store: {}", e);
                        self.state.error_message =
                            Some(format!("Failed to save project store: {}", e));
                    }

                    tracing::info!("Deleted project: {}", project_id);

                    // Navigate back to projects overview
                    self.state.view = View::ProjectsOverview;

                    // Adjust selected index if needed
                    let new_project_count = self.project_store.project_count();
                    if self.state.selected_project_index >= new_project_count
                        && new_project_count > 0
                    {
                        self.state.selected_project_index = new_project_count - 1;
                    } else if new_project_count == 0 {
                        self.state.selected_project_index = 0;
                    }
                }
                self.state.input_mode = InputMode::Normal;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                // Cancel deletion
                self.state.pending_delete_project = None;
                self.state.input_mode = InputMode::Normal;
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key while confirming quit
    fn handle_confirming_quit_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                // Confirm quit
                self.state.should_quit = true;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                // Cancel quit
                self.state.input_mode = InputMode::Normal;
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key while renaming a project
    fn handle_renaming_project_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        match key.code {
            KeyCode::Esc => {
                self.state.input_mode = InputMode::Normal;
                self.state.new_project_name.clear();
                self.state.renaming_project = None;
            }
            KeyCode::Enter => {
                if let Some(project_id) = self.state.renaming_project {
                    let new_name = self.state.new_project_name.trim().to_string();
                    if !new_name.is_empty() {
                        if let Some(project) = self.project_store.get_project_mut(project_id) {
                            project.name = new_name;
                        }
                        if let Err(e) = self.project_store.save() {
                            self.state.error_message = Some(format!("Failed to save: {}", e));
                        }
                    }
                }
                self.state.input_mode = InputMode::Normal;
                self.state.new_project_name.clear();
                self.state.renaming_project = None;
            }
            KeyCode::Backspace => {
                self.state.new_project_name.pop();
            }
            KeyCode::Char(c) => {
                self.state.new_project_name.push(c);
            }
            _ => {}
        }
        Ok(())
    }

    // ========================================================================
    // New Worktree Creation Wizard Handlers
    // ========================================================================

    /// Handle key in WorktreeSelectBranch mode (Step 1)
    ///
    /// User can:
    /// - Type to filter existing branches
    /// - Arrow keys to navigate the list
    /// - Enter on existing local branch → WorktreeConfirm (ExistingLocal)
    /// - Enter on remote branch → WorktreeConfirm (RemoteTracking)
    /// - Enter on "Create new branch" → WorktreeSelectBase
    /// - Esc to cancel
    fn handle_worktree_select_branch_key(
        &mut self,
        key: KeyEvent,
        project_id: ProjectId,
    ) -> Result<()> {
        // project_id reserved for potential future use
        let _ = project_id;

        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        match key.code {
            KeyCode::Esc => {
                self.cancel_worktree_wizard();
            }
            KeyCode::Up => {
                self.worktree_navigate_branches(-1);
            }
            KeyCode::Down => {
                self.worktree_navigate_branches(1);
            }
            KeyCode::Enter => {
                let filtered_count = self.state.worktree_filtered_branches.len();
                let has_create_option = !self.state.worktree_search_text.is_empty();

                if self.state.worktree_list_index < filtered_count {
                    // Selected an existing branch
                    let selected = self.state.worktree_filtered_branches
                        [self.state.worktree_list_index]
                        .clone();

                    // Block selection of already-tracked branches
                    if selected.is_already_tracked {
                        return Ok(());
                    }

                    if selected.ref_type == BranchRefType::Local {
                        // Existing local branch → go directly to confirm
                        self.state.worktree_source_branch = Some(selected.clone());
                        self.state.worktree_branch_name = selected.name.clone();
                        self.state.worktree_creation_type = WorktreeCreationType::ExistingLocal;
                        self.state.input_mode = InputMode::WorktreeConfirm;
                    } else {
                        // Remote branch → will create tracking branch
                        // Extract local name from remote ref (e.g., "origin/feature" -> "feature")
                        let local_name = selected
                            .name
                            .split_once('/')
                            .map(|(_, b)| b.to_string())
                            .unwrap_or_else(|| selected.name.clone());

                        self.state.worktree_source_branch = Some(selected);
                        self.state.worktree_branch_name = local_name;
                        self.state.worktree_creation_type = WorktreeCreationType::RemoteTracking;
                        self.state.input_mode = InputMode::WorktreeConfirm;
                    }
                } else if has_create_option {
                    // Selected "Create new branch" option
                    self.state.worktree_branch_name = self.state.worktree_search_text.clone();
                    self.state.worktree_creation_type = WorktreeCreationType::NewBranch;

                    // Initialize base branch selection
                    self.state.worktree_base_search_text.clear();
                    self.state.worktree_base_list_index = 0;

                    // Find and select the default base branch
                    if let Some(idx) = self
                        .state
                        .worktree_all_branches
                        .iter()
                        .position(|b| b.is_default_base)
                    {
                        self.state.worktree_base_list_index = idx;
                        self.state.worktree_base_branch =
                            Some(self.state.worktree_all_branches[idx].clone());
                    } else if let Some(first) = self.state.worktree_all_branches.first() {
                        self.state.worktree_base_branch = Some(first.clone());
                    }

                    self.state.input_mode = InputMode::WorktreeSelectBase;
                }
            }
            KeyCode::Backspace => {
                self.state.worktree_search_text.pop();
                self.update_worktree_filtered_branches();
                // Reset selection to first selectable
                self.worktree_select_first_selectable();
            }
            KeyCode::Char(c) => {
                self.state.worktree_search_text.push(c);
                self.update_worktree_filtered_branches();
                // Reset selection to first selectable
                self.worktree_select_first_selectable();
            }
            _ => {}
        }
        Ok(())
    }

    /// Navigate up/down in the worktree branch list, skipping already-tracked branches
    fn worktree_navigate_branches(&mut self, direction: i32) {
        let filtered_count = self.state.worktree_filtered_branches.len();
        let has_create_option = !self.state.worktree_search_text.is_empty();
        let total_options = if has_create_option {
            filtered_count + 1
        } else {
            filtered_count
        };

        if total_options == 0 {
            return;
        }

        let current = self.state.worktree_list_index;
        let mut next = current;

        for _ in 0..total_options {
            if direction > 0 {
                next = (next + 1) % total_options;
            } else {
                next = next.checked_sub(1).unwrap_or(total_options - 1);
            }
            // The "Create new branch" option (at filtered_count) is always selectable
            if next >= filtered_count {
                self.state.worktree_list_index = next;
                return;
            }
            // Check if this branch is selectable (not already tracked)
            if !self.state.worktree_filtered_branches[next].is_already_tracked {
                self.state.worktree_list_index = next;
                return;
            }
        }
        // If all branches are tracked, stay at current position
    }

    /// Select the first non-tracked branch in the list
    fn worktree_select_first_selectable(&mut self) {
        let filtered_count = self.state.worktree_filtered_branches.len();
        let has_create_option = !self.state.worktree_search_text.is_empty();

        // First, try to find a non-tracked branch
        for (i, branch) in self.state.worktree_filtered_branches.iter().enumerate() {
            if !branch.is_already_tracked {
                self.state.worktree_list_index = i;
                return;
            }
        }
        // If all branches are tracked and there's a create option, select it
        if has_create_option {
            self.state.worktree_list_index = filtered_count;
            return;
        }
        // Otherwise default to 0
        self.state.worktree_list_index = 0;
    }

    /// Handle key in WorktreeSelectBase mode (Step 2)
    ///
    /// User can:
    /// - Type to filter base branches
    /// - Arrow keys to navigate the list
    /// - Enter to confirm and go to WorktreeConfirm
    /// - Esc to go back to WorktreeSelectBranch
    fn handle_worktree_select_base_key(
        &mut self,
        key: KeyEvent,
        project_id: ProjectId,
    ) -> Result<()> {
        // project_id reserved for potential future use (e.g., setting default base branch)
        let _ = project_id;

        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        // Filter branches based on base search text
        let filtered: Vec<BranchRef> = if self.state.worktree_base_search_text.is_empty() {
            self.state.worktree_all_branches.clone()
        } else {
            let query = self.state.worktree_base_search_text.to_lowercase();
            self.state
                .worktree_all_branches
                .iter()
                .filter(|b| b.name.to_lowercase().contains(&query))
                .cloned()
                .collect()
        };
        let filtered_count = filtered.len();

        match key.code {
            KeyCode::Esc => {
                // Go back to step 1
                self.state.input_mode = InputMode::WorktreeSelectBranch;
                self.state.worktree_base_search_text.clear();
            }
            KeyCode::Up => {
                if filtered_count > 0 {
                    self.state.worktree_base_list_index = self
                        .state
                        .worktree_base_list_index
                        .checked_sub(1)
                        .unwrap_or(filtered_count - 1);
                    // Update selected base branch
                    if let Some(branch) = filtered.get(self.state.worktree_base_list_index) {
                        self.state.worktree_base_branch = Some(branch.clone());
                    }
                }
            }
            KeyCode::Down => {
                if filtered_count > 0 {
                    self.state.worktree_base_list_index =
                        (self.state.worktree_base_list_index + 1) % filtered_count;
                    // Update selected base branch
                    if let Some(branch) = filtered.get(self.state.worktree_base_list_index) {
                        self.state.worktree_base_branch = Some(branch.clone());
                    }
                }
            }
            KeyCode::Enter => {
                // Confirm base branch selection, go to confirmation
                if let Some(branch) = filtered.get(self.state.worktree_base_list_index) {
                    self.state.worktree_base_branch = Some(branch.clone());
                }
                self.state.input_mode = InputMode::WorktreeConfirm;
            }
            KeyCode::Backspace => {
                self.state.worktree_base_search_text.pop();
                self.state.worktree_base_list_index = 0;
            }
            KeyCode::Char(c) => {
                self.state.worktree_base_search_text.push(c);
                self.state.worktree_base_list_index = 0;
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key in WorktreeConfirm mode (Step 3)
    ///
    /// User can:
    /// - Enter to create the worktree
    /// - Esc to go back to the previous step
    fn handle_worktree_confirm_key(&mut self, key: KeyEvent, project_id: ProjectId) -> Result<()> {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        match key.code {
            KeyCode::Esc => {
                // Go back to appropriate step
                match self.state.worktree_creation_type {
                    WorktreeCreationType::NewBranch => {
                        self.state.input_mode = InputMode::WorktreeSelectBase;
                    }
                    _ => {
                        self.state.input_mode = InputMode::WorktreeSelectBranch;
                    }
                }
            }
            KeyCode::Enter => {
                // Create the worktree
                let result = match self.state.worktree_creation_type {
                    WorktreeCreationType::ExistingLocal => {
                        // Create worktree from existing local branch (don't create branch)
                        self.create_worktree(
                            project_id,
                            &self.state.worktree_branch_name.clone(),
                            false,
                            None,
                        )
                    }
                    WorktreeCreationType::RemoteTracking => {
                        // Create local tracking branch from remote, then worktree
                        let base_ref = self
                            .state
                            .worktree_source_branch
                            .as_ref()
                            .map(|b| b.name.clone());
                        self.create_worktree(
                            project_id,
                            &self.state.worktree_branch_name.clone(),
                            true,
                            base_ref.as_deref(),
                        )
                    }
                    WorktreeCreationType::NewBranch => {
                        // Create new branch from base, then worktree
                        let base_ref = self
                            .state
                            .worktree_base_branch
                            .as_ref()
                            .map(|b| b.name.clone());
                        self.create_worktree(
                            project_id,
                            &self.state.worktree_branch_name.clone(),
                            true,
                            base_ref.as_deref(),
                        )
                    }
                };

                // Capture branch_id before canceling wizard (which clears state)
                let created_branch_id = match &result {
                    Ok(branch_id) => Some(*branch_id),
                    Err(e) => {
                        tracing::error!("Failed to create worktree: {}", e);
                        self.state.error_message =
                            Some(format!("Failed to create worktree: {}", e));
                        None
                    }
                };

                self.cancel_worktree_wizard();

                // Navigate to the newly created branch
                if let Some(branch_id) = created_branch_id {
                    self.state.navigate_to_branch(project_id, branch_id);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Cancel and clean up the worktree wizard state
    fn cancel_worktree_wizard(&mut self) {
        self.state.input_mode = InputMode::Normal;
        self.state.worktree_search_text.clear();
        self.state.worktree_all_branches.clear();
        self.state.worktree_filtered_branches.clear();
        self.state.worktree_list_index = 0;
        self.state.worktree_branch_name.clear();
        self.state.worktree_source_branch = None;
        self.state.worktree_base_branch = None;
        self.state.worktree_base_search_text.clear();
        self.state.worktree_base_list_index = 0;
        self.state.worktree_creation_type = WorktreeCreationType::ExistingLocal;
        self.state.worktree_project_name.clear();
        self.state.fetch_error = None;
    }

    /// Update filtered branches based on search text
    fn update_worktree_filtered_branches(&mut self) {
        if self.state.worktree_search_text.is_empty() {
            self.state.worktree_filtered_branches = self.state.worktree_all_branches.clone();
        } else {
            let query = self.state.worktree_search_text.to_lowercase();
            self.state.worktree_filtered_branches = self
                .state
                .worktree_all_branches
                .iter()
                .filter(|b| b.name.to_lowercase().contains(&query))
                .cloned()
                .collect();
        }
    }

    /// Start the new worktree creation wizard (Step 1)
    fn start_worktree_wizard(&mut self, project_id: ProjectId) {
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

    // ========================================================================
    // End New Worktree Creation Wizard Handlers
    // ========================================================================

    /// Start worktree creation flow: fetch branches, then show dialog
    /// DEPRECATED: Use start_worktree_wizard instead for new worktree creation
    #[allow(dead_code)]
    fn start_worktree_creation(&mut self, project_id: ProjectId) {
        self.state.new_branch_name.clear();
        self.state.base_branch_selector_index = 0;
        self.state.fetch_error = None;

        // Fetch branches (synchronous for now - could be made async)
        self.fetch_and_populate_branch_refs(project_id);

        // Transition to worktree creation mode
        self.state.input_mode = InputMode::CreatingWorktree;
    }

    /// Start default base branch selection flow
    fn start_default_base_selection(&mut self, project_id: ProjectId) {
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
    fn create_worktree(
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
    fn jump_to_next_attention(&mut self) -> Result<()> {
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
            self.state.error_message = Some("No sessions need attention".to_string());
        }
        Ok(())
    }

    /// Resize the active session's PTY to match the output viewport
    fn resize_active_session_pty(&mut self) -> Result<()> {
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
    fn show_loading(&mut self, message: &str) -> Result<()> {
        self.state.loading_message = Some(message.to_string());
        self.render()?;
        Ok(())
    }

    /// Clear the loading indicator
    fn clear_loading(&mut self) {
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
                    );
                }
                View::SessionView => {
                    render_session_view(frame, area, state, sessions, project_store, config);
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
                    );
                }
                View::LogViewer => {
                    render_log_viewer(
                        frame,
                        area,
                        log_buffer,
                        log_file_info,
                        state.log_viewer_scroll,
                        state.log_viewer_auto_scroll,
                        state.focus_timer.as_ref(),
                    );
                }
                View::FocusStats => {
                    render_focus_stats(
                        frame,
                        area,
                        &state.focus_sessions,
                        project_store,
                        state.focus_stats_selected_index,
                        state.focus_events_supported,
                        state.focus_timer.as_ref(),
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
