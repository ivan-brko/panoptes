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
use crate::hooks::{self, HookEventReceiver, HookEventSender, ServerHandle};
use crate::project::{BranchId, ProjectId, ProjectStore};
use crate::session::{SessionId, SessionManager};
use crate::tui::views::{
    render_placeholder, render_project_detail, render_projects_overview, render_session_view,
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
    /// Buffer for new project path input
    pub new_project_path: String,
    /// Whether the application should quit
    pub should_quit: bool,
    /// Whether the UI needs to be re-rendered
    pub needs_render: bool,
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

    /// Navigate to session view
    pub fn navigate_to_session(&mut self, session_id: SessionId) {
        // Remember where we came from
        self.session_return_view = Some(self.view);
        self.view = View::SessionView;
        self.active_session = Some(session_id);
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
    }
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
    /// Hook server handle (kept alive)
    _hook_server: ServerHandle,
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

        // Create hook event channel
        let (hook_tx, hook_rx): (HookEventSender, HookEventReceiver) =
            hooks::server::create_channel(100);

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
            _hook_server: hook_server,
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
            self.sessions.handle_hook_event(&event);
            had_events = true;
        }
        had_events
    }

    /// Handle a key event
    fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
        // Global keys (work in any mode)
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.state.should_quit = true;
            return Ok(());
        }

        match self.state.input_mode {
            InputMode::Normal => self.handle_normal_mode_key(key),
            InputMode::Session => self.handle_session_mode_key(key),
            InputMode::CreatingSession => self.handle_creating_session_key(key),
        }
    }

    /// Handle key in normal mode
    fn handle_normal_mode_key(&mut self, key: KeyEvent) -> Result<()> {
        match self.state.view {
            View::ProjectsOverview => self.handle_projects_overview_key(key),
            View::ProjectDetail(_) => self.handle_project_detail_key(key),
            View::BranchDetail(_, _) => self.handle_branch_detail_key(key),
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
                // Create a new quick session
                self.state.input_mode = InputMode::CreatingSession;
                self.state.new_session_name.clear();
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
    /// TODO: Will be fully implemented in Ticket 26
    fn handle_branch_detail_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state.navigate_back();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                // Will navigate sessions when implemented
                self.state.select_next(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.state.select_prev(1);
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
            KeyCode::Char('i') | KeyCode::Enter => {
                // Enter session mode (send keys to PTY)
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
                        self.state.active_session = Some(session.info.id);
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
                            self.state.active_session = Some(session.info.id);
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
            }
            KeyCode::Enter => {
                // Create the session
                let name = if self.state.new_session_name.is_empty() {
                    format!("Session {}", self.sessions.len() + 1)
                } else {
                    std::mem::take(&mut self.state.new_session_name)
                };

                // Use current directory as working directory
                let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

                // Get terminal dimensions for the session
                let (rows, cols) = if let Ok(size) = self.tui.size() {
                    (
                        size.height.saturating_sub(8) as usize,
                        size.width.saturating_sub(2) as usize,
                    )
                } else {
                    (24, 80) // Fallback dimensions
                };

                // TODO: Replace with actual project_id and branch_id from selected project/branch
                // This will be implemented in Ticket 30 (Session in Branch)
                let project_id = Uuid::nil();
                let branch_id = Uuid::nil();

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
                        // Update the selected index for the current view
                        let new_index = self.sessions.len().saturating_sub(1);
                        self.state.selected_project_index = new_index;
                        self.state.selected_timeline_index = new_index;
                    }
                    Err(e) => {
                        tracing::error!("Failed to create session: {}", e);
                    }
                }

                self.state.input_mode = InputMode::Normal;
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

        self.tui.draw(|frame| {
            let area = frame.size();

            match state.view {
                View::ProjectsOverview => {
                    render_projects_overview(frame, area, state, project_store, sessions);
                }
                View::ProjectDetail(project_id) => {
                    render_project_detail(frame, area, state, project_id, project_store, sessions);
                }
                View::BranchDetail(_, _) => {
                    // TODO: Ticket 26 - Render branch detail
                    render_placeholder(frame, area, "Branch Detail (Coming Soon)");
                }
                View::SessionView => {
                    render_session_view(frame, area, state, sessions);
                }
                View::ActivityTimeline => {
                    render_timeline(frame, area, state, sessions);
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
