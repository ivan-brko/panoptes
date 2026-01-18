//! Application state and main event loop
//!
//! This module contains the central application state and the main event loop
//! that ties together session management, hook handling, and terminal UI.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::config::Config;
use crate::hooks::{self, HookEventReceiver, HookEventSender, ServerHandle};
use crate::session::{SessionId, SessionManager};
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
    /// Session list overview
    #[default]
    SessionList,
    /// Single session fullscreen view
    SessionView,
}

/// Application state
#[derive(Default)]
pub struct AppState {
    /// Current view
    pub view: View,
    /// Current input mode
    pub input_mode: InputMode,
    /// Currently selected session index (in session list)
    pub selected_index: usize,
    /// Session being viewed (in session view)
    pub active_session: Option<SessionId>,
    /// Buffer for new session name input
    pub new_session_name: String,
    /// Whether the application should quit
    pub should_quit: bool,
    /// Whether the UI needs to be re-rendered
    pub needs_render: bool,
}

impl AppState {
    /// Select the next session in the list
    pub fn select_next(&mut self, session_count: usize) {
        if session_count > 0 {
            self.selected_index = (self.selected_index + 1) % session_count;
        }
    }

    /// Select the previous session in the list
    pub fn select_prev(&mut self, session_count: usize) {
        if session_count > 0 {
            self.selected_index = self
                .selected_index
                .checked_sub(1)
                .unwrap_or(session_count - 1);
        }
    }

    /// Select a session by number (1-indexed)
    pub fn select_by_number(&mut self, num: usize, session_count: usize) {
        if num > 0 && num <= session_count {
            self.selected_index = num - 1;
        }
    }
}

/// Main application struct
pub struct App {
    /// Application configuration
    #[allow(dead_code)]
    config: Config,
    /// Application state
    state: AppState,
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
            View::SessionList => self.handle_session_list_key(key),
            View::SessionView => self.handle_session_view_normal_key(key),
        }
    }

    /// Handle key in session list view (normal mode)
    fn handle_session_list_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('q') => {
                self.state.should_quit = true;
            }
            KeyCode::Char('n') => {
                // Start creating a new session
                self.state.input_mode = InputMode::CreatingSession;
                self.state.new_session_name.clear();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.state.select_next(self.sessions.len());
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.state.select_prev(self.sessions.len());
            }
            KeyCode::Enter => {
                // Enter session view for selected session
                if let Some(session) = self.sessions.get_by_index(self.state.selected_index) {
                    self.state.active_session = Some(session.info.id);
                    self.state.view = View::SessionView;
                    // Resize PTY to match output viewport
                    self.resize_active_session_pty()?;
                }
            }
            KeyCode::Char('d') => {
                // Delete selected session
                if let Some(session) = self.sessions.get_by_index(self.state.selected_index) {
                    let id = session.info.id;
                    if let Err(e) = self.sessions.destroy_session(id) {
                        tracing::error!("Failed to destroy session: {}", e);
                    }
                    // Adjust selection if needed
                    if self.state.selected_index >= self.sessions.len()
                        && self.state.selected_index > 0
                    {
                        self.state.selected_index -= 1;
                    }
                }
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                // Jump to session by number
                if let Some(num) = c.to_digit(10) {
                    self.state
                        .select_by_number(num as usize, self.sessions.len());
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
                // Go back to session list
                self.state.view = View::SessionList;
                self.state.active_session = None;
            }
            KeyCode::Char('i') | KeyCode::Enter => {
                // Enter session mode (send keys to PTY)
                self.state.input_mode = InputMode::Session;
            }
            KeyCode::Tab => {
                // Switch to next session
                self.state.select_next(self.sessions.len());
                if let Some(session) = self.sessions.get_by_index(self.state.selected_index) {
                    self.state.active_session = Some(session.info.id);
                    self.resize_active_session_pty()?;
                }
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                // Jump to session by number
                if let Some(num) = c.to_digit(10) {
                    let count = self.sessions.len();
                    if num > 0 && (num as usize) <= count {
                        self.state.selected_index = (num as usize) - 1;
                        if let Some(session) = self.sessions.get_by_index(self.state.selected_index)
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

                match self
                    .sessions
                    .create_session(name.clone(), working_dir, None, rows, cols)
                {
                    Ok(session_id) => {
                        tracing::info!("Created session: {} ({})", name, session_id);
                        self.state.selected_index = self.sessions.len() - 1;
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
        let sessions = &self.sessions;

        self.tui.draw(|frame| {
            let area = frame.size();

            match state.view {
                View::SessionList => {
                    render_session_list(frame, area, state, sessions);
                }
                View::SessionView => {
                    render_session_view(frame, area, state, sessions);
                }
            }
        })?;

        Ok(())
    }
}

/// Render the session list view
fn render_session_list(frame: &mut Frame, area: Rect, state: &AppState, sessions: &SessionManager) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Session list
            Constraint::Length(3), // Footer/help
        ])
        .split(area);

    // Header
    let header = Paragraph::new("Panoptes - Claude Code Session Manager")
        .style(Style::default().fg(Color::Cyan).bold())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    // Session list or new session input
    if state.input_mode == InputMode::CreatingSession {
        let input = Paragraph::new(format!("New session name: {}_", state.new_session_name))
            .style(Style::default().fg(Color::Yellow))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Create Session"),
            );
        frame.render_widget(input, chunks[1]);
    } else if sessions.is_empty() {
        let empty = Paragraph::new("No sessions. Press 'n' to create one.")
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::ALL).title("Sessions"));
        frame.render_widget(empty, chunks[1]);
    } else {
        let items: Vec<ListItem> = sessions
            .sessions_in_order()
            .iter()
            .enumerate()
            .map(|(i, session)| {
                let state_color = session.info.state.color();
                let selected = i == state.selected_index;
                let prefix = if selected { "▶ " } else { "  " };
                let content = format!(
                    "{}{}: {} [{}]",
                    prefix,
                    i + 1,
                    session.info.name,
                    session.info.state.display_name()
                );
                let style = if selected {
                    Style::default().fg(state_color).bold()
                } else {
                    Style::default().fg(state_color)
                };
                ListItem::new(content).style(style)
            })
            .collect();

        let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Sessions"));
        frame.render_widget(list, chunks[1]);
    }

    // Footer with help
    let help_text = match state.input_mode {
        InputMode::CreatingSession => "Enter: create | Esc: cancel",
        _ => "n: new | j/k: navigate | Enter: open | d: delete | q: quit",
    };
    let footer = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
}

/// Render the session view
fn render_session_view(frame: &mut Frame, area: Rect, state: &AppState, sessions: &SessionManager) {
    let session = state.active_session.and_then(|id| sessions.get(id));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Output
            Constraint::Length(3), // Footer
        ])
        .split(area);

    // Header with session info
    let header_text = if let Some(session) = session {
        let mode_indicator = match state.input_mode {
            InputMode::Session => " [SESSION MODE]",
            _ => " [NORMAL]",
        };
        format!(
            "{} - {}{}",
            session.info.name,
            session.info.state.display_name(),
            mode_indicator
        )
    } else {
        "No session selected".to_string()
    };

    let header_color = session
        .map(|s| s.info.state.color())
        .unwrap_or(Color::DarkGray);

    let header = Paragraph::new(header_text)
        .style(Style::default().fg(header_color).bold())
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    // Output area
    if let Some(session) = session {
        let output_height = chunks[1].height.saturating_sub(2) as usize; // Account for borders
        let styled_lines = session.visible_styled_lines(output_height);
        let output = Paragraph::new(styled_lines)
            .block(Block::default().borders(Borders::ALL).title("Output"));
        frame.render_widget(output, chunks[1]);
    } else {
        let empty = Paragraph::new("Session not found")
            .style(Style::default().fg(Color::Red))
            .block(Block::default().borders(Borders::ALL).title("Output"));
        frame.render_widget(empty, chunks[1]);
    }

    // Footer with help
    let help_text = match state.input_mode {
        InputMode::Session => "Esc: exit session mode | Keys sent to session",
        _ => "i/Enter: session mode | Tab: next | 1-9: jump | Esc/q: back",
    };
    let footer = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, chunks[2]);
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
        assert_eq!(View::default(), View::SessionList);
    }

    #[test]
    fn test_app_state_default() {
        let state = AppState::default();
        assert_eq!(state.view, View::SessionList);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert_eq!(state.selected_index, 0);
        assert!(state.active_session.is_none());
        assert!(!state.should_quit);
    }

    #[test]
    fn test_app_state_select_next() {
        let mut state = AppState::default();
        state.select_next(3);
        assert_eq!(state.selected_index, 1);
        state.select_next(3);
        assert_eq!(state.selected_index, 2);
        state.select_next(3);
        assert_eq!(state.selected_index, 0); // Wraps around
    }

    #[test]
    fn test_app_state_select_prev() {
        let mut state = AppState::default();
        state.select_prev(3);
        assert_eq!(state.selected_index, 2); // Wraps to end
        state.select_prev(3);
        assert_eq!(state.selected_index, 1);
    }

    #[test]
    fn test_app_state_select_by_number() {
        let mut state = AppState::default();
        state.select_by_number(2, 5);
        assert_eq!(state.selected_index, 1); // 1-indexed to 0-indexed
        state.select_by_number(0, 5); // Invalid, should not change
        assert_eq!(state.selected_index, 1);
        state.select_by_number(6, 5); // Out of range, should not change
        assert_eq!(state.selected_index, 1);
    }

    #[test]
    fn test_app_state_select_empty() {
        let mut state = AppState::default();
        state.select_next(0); // Should not panic
        assert_eq!(state.selected_index, 0);
        state.select_prev(0); // Should not panic
        assert_eq!(state.selected_index, 0);
    }
}
