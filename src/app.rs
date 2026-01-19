//! Application state and main event loop
//!
//! This module contains the central application state and the main event loop
//! that ties together session management, hook handling, and terminal UI.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use uuid::Uuid;

use crate::config::Config;
use crate::hooks::{
    self, HookEventReceiver, HookEventSender, ServerHandle, DEFAULT_CHANNEL_BUFFER,
};
use crate::project::{BranchId, ProjectId, ProjectStore};
use crate::session::{SessionId, SessionManager};
use crate::tui::views::{
    render_branch_detail, render_project_detail, render_projects_overview, render_session_view,
    render_timeline,
};
use crate::tui::Tui;

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
    /// Creating a new worktree - typing branch name
    CreatingWorktree,
    /// Confirming session deletion
    ConfirmingSessionDelete,
    /// Confirming project deletion
    ConfirmingProjectDelete,
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

    /// Get the parent view for navigation (Esc key)
    pub fn parent(&self) -> Option<View> {
        match self {
            View::ProjectsOverview => None,
            View::ProjectDetail(_) => Some(View::ProjectsOverview),
            View::BranchDetail(project_id, _) => Some(View::ProjectDetail(*project_id)),
            View::SessionView => None, // Handled specially based on context
            View::ActivityTimeline => Some(View::ProjectsOverview),
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
    /// Buffer for new branch name input (worktree creation)
    pub new_branch_name: String,
    /// Cached branches from git (for branch selector)
    pub available_branches: Vec<String>,
    /// Branches matching current search filter
    pub filtered_branches: Vec<String>,
    /// Selected index in branch selector (0 = "Create new")
    pub branch_selector_index: usize,
    /// Session pending deletion (for confirmation dialog)
    pub pending_delete_session: Option<SessionId>,
    /// Project pending deletion (for confirmation dialog)
    pub pending_delete_project: Option<ProjectId>,
    /// Whether the application should quit
    pub should_quit: bool,
    /// Whether the UI needs to be re-rendered
    pub needs_render: bool,
    /// Count of dropped hook events (for warning display)
    pub dropped_events_count: u64,
    /// Error message to display to the user (cleared on next keypress)
    pub error_message: Option<String>,
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

/// Filter branches by fuzzy substring match
fn filter_branches(branches: &[String], query: &str) -> Vec<String> {
    if query.is_empty() {
        return branches.to_vec();
    }
    let query_lower = query.to_lowercase();
    branches
        .iter()
        .filter(|b| b.to_lowercase().contains(&query_lower))
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
}

impl App {
    /// Create a new application instance
    pub async fn new() -> Result<Self> {
        let config = Config::load()?;

        // Load project store (or create empty if doesn't exist)
        let project_store = ProjectStore::load().unwrap_or_else(|e| {
            tracing::warn!("Failed to load project store: {}, starting fresh", e);
            ProjectStore::new()
        });
        tracing::info!(
            "Loaded {} projects, {} branches",
            project_store.project_count(),
            project_store.branch_count()
        );

        // Create hook event channel with large buffer to avoid dropping events
        let (hook_tx, hook_rx): (HookEventSender, HookEventReceiver) =
            hooks::server::create_channel(DEFAULT_CHANNEL_BUFFER);

        // Start hook server
        let hook_server = hooks::server::start(config.hook_port, hook_tx).await?;
        tracing::info!("Hook server started on port {}", hook_server.addr().port());

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
        let tick_rate = Duration::from_millis(50);

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
                    Event::Resize(_, _) => {
                        // Resize active session PTY to match output area
                        self.resize_active_session_pty()?;
                        self.state.needs_render = true;
                    }
                    _ => {}
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
        // Works in all modes except Session mode (where keys go to PTY)
        if key.code == KeyCode::Char(' ') && self.state.input_mode != InputMode::Session {
            return self.jump_to_next_attention();
        }

        match self.state.input_mode {
            InputMode::Normal => self.handle_normal_mode_key(key),
            InputMode::Session => self.handle_session_mode_key(key),
            InputMode::CreatingSession => self.handle_creating_session_key(key),
            InputMode::AddingProject => self.handle_adding_project_key(key),
            InputMode::CreatingWorktree => {
                // Need to get project_id from current view
                if let View::ProjectDetail(project_id) = self.state.view {
                    self.handle_creating_worktree_key(key, project_id)
                } else {
                    Ok(())
                }
            }
            InputMode::ConfirmingSessionDelete => self.handle_confirming_delete_key(key),
            InputMode::ConfirmingProjectDelete => self.handle_confirming_project_delete_key(key),
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
        }
    }

    /// Handle key in projects overview (normal mode)
    fn handle_projects_overview_key(&mut self, key: KeyEvent) -> Result<()> {
        let project_count = self.project_store.project_count();
        let session_count = self.sessions.len();

        match key.code {
            KeyCode::Char('q') => {
                self.state.should_quit = true;
            }
            KeyCode::Char('t') => {
                self.state.navigate_to_timeline();
            }
            KeyCode::Char('n') => {
                if project_count > 0 {
                    // Projects exist - navigate to selected project's default branch
                    let projects = self.project_store.projects_sorted();
                    if let Some(project) = projects.get(self.state.selected_project_index) {
                        let project_id = project.id;
                        // Find default branch for this project
                        if let Some(branch) = self
                            .project_store
                            .branches_for_project(project_id)
                            .into_iter()
                            .find(|b| b.is_default)
                        {
                            // Navigate directly to branch detail
                            self.state.navigate_to_branch(project_id, branch.id);
                        } else {
                            // No default branch - navigate to project detail
                            self.state.navigate_to_project(project_id);
                        }
                    }
                } else {
                    // No projects - allow creating an unassociated quick session
                    self.state.creating_session_project_id = None;
                    self.state.creating_session_branch_id = None;
                    self.state.creating_session_working_dir = None;
                    self.state.new_session_name.clear();
                    self.state.input_mode = InputMode::CreatingSession;
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                // Navigate projects if any, otherwise sessions
                if project_count > 0 {
                    self.state.selected_project_index =
                        (self.state.selected_project_index + 1) % project_count;
                } else if session_count > 0 {
                    self.state.selected_session_index =
                        (self.state.selected_session_index + 1) % session_count;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if project_count > 0 {
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
                // Open selected project or session
                if project_count > 0 {
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
                        self.sessions.acknowledge_attention(session_id);
                        if self.config.notification_method == "title" {
                            SessionManager::reset_terminal_title();
                        }
                        self.resize_active_session_pty()?;
                    }
                }
            }
            KeyCode::Char('d') => {
                // Delete selected session (only if no projects, sessions are in focus)
                if project_count == 0 && session_count > 0 {
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
            KeyCode::Char('a') => {
                // Start adding a new project
                self.state.input_mode = InputMode::AddingProject;
                self.state.new_project_path.clear();
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                if let Some(num) = c.to_digit(10) {
                    if project_count > 0 && num > 0 && (num as usize) <= project_count {
                        self.state.selected_project_index = (num as usize) - 1;
                    } else if project_count == 0 && num > 0 && (num as usize) <= session_count {
                        self.state.selected_session_index = (num as usize) - 1;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key in project detail view (normal mode)
    fn handle_project_detail_key(&mut self, key: KeyEvent) -> Result<()> {
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
                self.state.should_quit = true;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if branch_count > 0 {
                    self.state.selected_branch_index =
                        (self.state.selected_branch_index + 1) % branch_count;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
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
            KeyCode::Char('w') => {
                // Start creating a new worktree
                self.state.input_mode = InputMode::CreatingWorktree;
                self.state.new_branch_name.clear();
                self.state.branch_selector_index = 0;

                // Fetch available branches from git
                if let Some(project) = self.project_store.get_project(project_id) {
                    if let Ok(git) = crate::git::GitOps::open(&project.repo_path) {
                        self.state.available_branches =
                            git.list_local_branches().unwrap_or_default();
                    } else {
                        self.state.available_branches.clear();
                    }
                } else {
                    self.state.available_branches.clear();
                }
                self.state.filtered_branches = self.state.available_branches.clone();
            }
            KeyCode::Char('d') => {
                // Prompt for confirmation before deleting project
                self.state.pending_delete_project = Some(project_id);
                self.state.input_mode = InputMode::ConfirmingProjectDelete;
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
        let branch_sessions = self.sessions.sessions_for_branch(branch_id);
        let session_count = branch_sessions.len();

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state.navigate_back();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.state.select_next(session_count);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.state.select_prev(session_count);
            }
            KeyCode::Enter => {
                let index = self.state.selected_session_index;
                if index < session_count {
                    let session_id = branch_sessions[index].info.id;
                    self.state.navigate_to_session(session_id);
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
    /// TODO: Will be fully implemented in Ticket 27
    fn handle_timeline_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state.navigate_back();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.state.select_next(self.sessions.len());
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.state.select_prev(self.sessions.len());
            }
            KeyCode::Enter => {
                let index = self.state.current_selected_index();
                if let Some(session) = self.sessions.get_by_index(index) {
                    let session_id = session.info.id;
                    self.state.navigate_to_session(session_id);
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

    /// Handle key in session view (normal mode)
    fn handle_session_view_normal_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                // Go back to the view we came from
                self.state.return_from_session();
            }
            KeyCode::Enter => {
                // Re-activate session mode (send keys to PTY)
                self.state.input_mode = InputMode::Session;
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
        // Escape exits session mode
        if key.code == KeyCode::Esc {
            self.state.input_mode = InputMode::Normal;
            return Ok(());
        }

        // Send key to active session
        if let Some(session_id) = self.state.active_session {
            if let Some(session) = self.sessions.get_mut(session_id) {
                session.send_key(key)?;
            }
        }
        Ok(())
    }

    /// Handle key while creating a new session
    fn handle_creating_session_key(&mut self, key: KeyEvent) -> Result<()> {
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

    /// Handle key when adding a new project
    fn handle_adding_project_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                // Cancel project addition
                self.state.input_mode = InputMode::Normal;
                self.state.new_project_path.clear();
            }
            KeyCode::Enter => {
                // Try to add the project
                let path_str = std::mem::take(&mut self.state.new_project_path);
                let path = PathBuf::from(shellexpand::tilde(&path_str).into_owned());

                // Check if it's a git repository
                match crate::git::GitOps::discover(&path) {
                    Ok(git) => {
                        let repo_path = git.repo_path().to_path_buf();

                        // Check if already added
                        if self.project_store.find_by_repo_path(&repo_path).is_some() {
                            tracing::warn!("Project already exists: {}", repo_path.display());
                            self.state.input_mode = InputMode::Normal;
                            return Ok(());
                        }

                        // Get project name from directory
                        let name = repo_path
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("unknown")
                            .to_string();

                        // Get default branch
                        let default_branch = git
                            .default_branch_name()
                            .unwrap_or_else(|_| "main".to_string());

                        // Create project
                        let project = crate::project::Project::new(
                            name.clone(),
                            repo_path.clone(),
                            default_branch.clone(),
                        );
                        let project_id = project.id;
                        self.project_store.add_project(project);

                        // Create default branch entry
                        let branch = crate::project::Branch::default_for_project(
                            project_id,
                            default_branch,
                            repo_path,
                        );
                        self.project_store.add_branch(branch);

                        // Save to disk
                        if let Err(e) = self.project_store.save() {
                            tracing::error!("Failed to save project store: {}", e);
                            self.state.error_message =
                                Some(format!("Failed to save project: {}", e));
                        }

                        tracing::info!("Added project: {}", name);

                        // Select the newly added project
                        let project_count = self.project_store.project_count();
                        self.state.selected_project_index = project_count.saturating_sub(1);
                    }
                    Err(e) => {
                        tracing::error!("Not a git repository: {} ({})", path.display(), e);
                    }
                }

                self.state.input_mode = InputMode::Normal;
            }
            KeyCode::Backspace => {
                self.state.new_project_path.pop();
            }
            KeyCode::Char(c) => {
                self.state.new_project_path.push(c);
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key when creating a new worktree
    fn handle_creating_worktree_key(&mut self, key: KeyEvent, project_id: ProjectId) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                // Cancel worktree creation
                self.state.input_mode = InputMode::Normal;
                self.state.new_branch_name.clear();
                self.state.available_branches.clear();
                self.state.filtered_branches.clear();
            }
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                // Navigate up (wrapping)
                let count = self.state.filtered_branches.len() + 1; // +1 for "Create new"
                self.state.branch_selector_index = self
                    .state
                    .branch_selector_index
                    .checked_sub(1)
                    .unwrap_or(count - 1);
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                // Navigate down (wrapping)
                let count = self.state.filtered_branches.len() + 1;
                self.state.branch_selector_index = (self.state.branch_selector_index + 1) % count;
            }
            KeyCode::Enter => {
                let result = if self.state.branch_selector_index == 0 {
                    // "Create new branch" selected - use typed name
                    let branch_name = std::mem::take(&mut self.state.new_branch_name);
                    if !branch_name.is_empty() {
                        self.create_worktree(project_id, &branch_name, true)
                    } else {
                        Ok(())
                    }
                } else {
                    // Existing branch selected
                    let idx = self.state.branch_selector_index - 1;
                    if let Some(branch_name) = self.state.filtered_branches.get(idx).cloned() {
                        self.create_worktree(project_id, &branch_name, false)
                    } else {
                        Ok(())
                    }
                };
                if let Err(e) = result {
                    tracing::error!("Failed to create worktree: {}", e);
                    self.state.error_message = Some(format!("Failed to create worktree: {}", e));
                }
                self.state.input_mode = InputMode::Normal;
                self.state.available_branches.clear();
                self.state.filtered_branches.clear();
            }
            KeyCode::Backspace => {
                self.state.new_branch_name.pop();
                self.state.filtered_branches =
                    filter_branches(&self.state.available_branches, &self.state.new_branch_name);
                self.state.branch_selector_index = 0; // Reset to "Create new"
            }
            KeyCode::Char(c) => {
                self.state.new_branch_name.push(c);
                self.state.filtered_branches =
                    filter_branches(&self.state.available_branches, &self.state.new_branch_name);
                self.state.branch_selector_index = 0; // Reset to "Create new"
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle key when confirming session deletion
    fn handle_confirming_delete_key(&mut self, key: KeyEvent) -> Result<()> {
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

    /// Handle key when confirming project deletion
    fn handle_confirming_project_delete_key(&mut self, key: KeyEvent) -> Result<()> {
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
                    self.state.selected_project_index = 0;
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

    /// Create a worktree for a branch
    fn create_worktree(
        &mut self,
        project_id: ProjectId,
        branch_name: &str,
        create_branch: bool,
    ) -> Result<()> {
        if let Some(project) = self.project_store.get_project(project_id) {
            let git = crate::git::GitOps::open(&project.repo_path)?;
            let worktree_path = crate::git::worktree::worktree_path_for_branch(
                &self.config.worktrees_dir,
                branch_name,
            );

            crate::git::worktree::create_worktree(
                git.repository(),
                branch_name,
                &worktree_path,
                create_branch,
            )?;

            let branch = crate::project::Branch::new(
                project_id,
                branch_name.to_string(),
                worktree_path,
                false, // is_default
                true,  // is_worktree
            );
            self.project_store.add_branch(branch);
            self.project_store.save()?;
            tracing::info!("Created worktree for branch: {}", branch_name);
        }
        Ok(())
    }

    /// Jump to the next session needing attention (oldest first)
    fn jump_to_next_attention(&mut self) -> Result<()> {
        let attention_sessions = self
            .sessions
            .sessions_needing_attention(self.config.idle_threshold_secs);

        if let Some(session) = attention_sessions.first() {
            let session_id = session.info.id;
            self.state.navigate_to_session(session_id);
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

    /// Render the current state
    fn render(&mut self) -> Result<()> {
        let state = &self.state;
        let project_store = &self.project_store;
        let sessions = &self.sessions;
        let config = &self.config;

        self.tui.draw(|frame| {
            let area = frame.size();

            match state.view {
                View::ProjectsOverview => {
                    render_projects_overview(frame, area, state, project_store, sessions, config);
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
                    );
                }
                View::SessionView => {
                    render_session_view(frame, area, state, sessions, config);
                }
                View::ActivityTimeline => {
                    render_timeline(frame, area, state, sessions, project_store, config);
                }
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
