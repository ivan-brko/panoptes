//! Session management module
//!
//! This module handles Claude Code session lifecycle, PTY management,
//! and session state tracking.

pub mod manager;
pub mod pty;
pub mod vterm;

pub use manager::SessionManager;
pub use pty::{mouse_event_to_bytes, ExitInfo, PtyHandle};
pub use vterm::{VirtualTerminal, DEFAULT_SCROLLBACK_ROWS};

use std::collections::VecDeque;
use std::rc::Rc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::claude_config::ClaudeConfigId;
use crate::project::{BranchId, ProjectId};

/// Unique identifier for a session
pub type SessionId = Uuid;

/// Type of session (determines state tracking behavior)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SessionType {
    /// Claude Code CLI session - uses hooks for state tracking
    #[default]
    ClaudeCode,
    /// Generic shell session (bash/zsh/etc) - uses foreground detection
    Shell,
}

impl SessionType {
    /// Get the display name for this session type
    pub fn display_name(&self) -> &str {
        match self {
            SessionType::ClaudeCode => "Claude Code",
            SessionType::Shell => "Shell",
        }
    }

    /// Check if this session type uses hooks for state tracking
    pub fn uses_hooks(&self) -> bool {
        matches!(self, SessionType::ClaudeCode)
    }
}

/// State of a session
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SessionState {
    /// Session is starting up
    #[default]
    Starting,
    /// Claude is thinking/processing
    Thinking,
    /// Claude is executing a tool
    Executing(String),
    /// Claude is waiting for user input
    Waiting,
    /// Session is idle (no recent activity)
    Idle,
    /// Session has exited
    Exited,
}

impl SessionState {
    /// Get the display name for this state
    pub fn display_name(&self) -> &str {
        match self {
            SessionState::Starting => "Starting",
            SessionState::Thinking => "Thinking",
            SessionState::Executing(_) => "Executing",
            SessionState::Waiting => "Waiting",
            SessionState::Idle => "Idle",
            SessionState::Exited => "Exited",
        }
    }

    /// Get the color for this state (for TUI rendering)
    pub fn color(&self) -> ratatui::style::Color {
        crate::tui::theme::theme().session_state_color(self)
    }

    /// Check if session is in an active state
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            SessionState::Starting | SessionState::Thinking | SessionState::Executing(_)
        )
    }
}

/// Metadata for a session (without PTY details)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Unique identifier
    pub id: SessionId,
    /// User-provided name
    pub name: String,
    /// Current state
    pub state: SessionState,
    /// Type of session (ClaudeCode or Shell)
    #[serde(default)]
    pub session_type: SessionType,
    /// Working directory
    pub working_dir: std::path::PathBuf,
    /// Parent project identifier
    pub project_id: ProjectId,
    /// Parent branch identifier
    pub branch_id: BranchId,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last activity timestamp
    pub last_activity: DateTime<Utc>,
    /// Timestamp when current state was entered (for timeout detection)
    #[serde(default = "Utc::now")]
    pub state_entered_at: DateTime<Utc>,
    /// Whether this session needs user attention (set on Waiting transition, cleared on view)
    #[serde(default)]
    pub needs_attention: bool,
    /// Exit reason (if exited due to error)
    #[serde(default)]
    pub exit_reason: Option<String>,
    /// Timestamp when session exited (for cleanup)
    #[serde(default)]
    pub exited_at: Option<DateTime<Utc>>,
    /// Claude configuration ID used for this session
    #[serde(default)]
    pub claude_config_id: Option<ClaudeConfigId>,
    /// Claude configuration name (cached for display)
    #[serde(default)]
    pub claude_config_name: Option<String>,
}

impl SessionInfo {
    /// Create new session info
    pub fn new(
        name: String,
        working_dir: std::path::PathBuf,
        project_id: ProjectId,
        branch_id: BranchId,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            state: SessionState::default(),
            session_type: SessionType::default(),
            working_dir,
            project_id,
            branch_id,
            created_at: now,
            last_activity: now,
            state_entered_at: now,
            needs_attention: false,
            exit_reason: None,
            exited_at: None,
            claude_config_id: None,
            claude_config_name: None,
        }
    }

    /// Create new session info with Claude configuration
    pub fn with_claude_config(
        name: String,
        working_dir: std::path::PathBuf,
        project_id: ProjectId,
        branch_id: BranchId,
        claude_config_id: Option<ClaudeConfigId>,
        claude_config_name: Option<String>,
    ) -> Self {
        let mut info = Self::new(name, working_dir, project_id, branch_id);
        info.claude_config_id = claude_config_id;
        info.claude_config_name = claude_config_name;
        info
    }

    /// Create new session info for a shell session
    pub fn shell(
        name: String,
        working_dir: std::path::PathBuf,
        project_id: ProjectId,
        branch_id: BranchId,
    ) -> Self {
        let mut info = Self::new(name, working_dir, project_id, branch_id);
        info.session_type = SessionType::Shell;
        // Shell sessions start in Waiting state (ready for input)
        info.state = SessionState::Waiting;
        info
    }
}

/// Bounded ring buffer for session output
#[derive(Debug)]
pub struct OutputBuffer {
    /// Lines of output (ring buffer)
    lines: VecDeque<String>,
    /// Maximum lines to retain
    max_lines: usize,
    /// Scroll offset from the bottom (0 = at bottom, showing most recent)
    scroll_offset: usize,
}

impl OutputBuffer {
    /// Create a new output buffer with the specified capacity
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(max_lines.min(1000)), // Pre-allocate reasonably
            max_lines,
            scroll_offset: 0,
        }
    }

    /// Append a line to the buffer, removing oldest if at capacity
    pub fn push(&mut self, line: String) {
        if self.lines.len() >= self.max_lines {
            self.lines.pop_front();
            // Adjust scroll offset if we're scrolled up
            if self.scroll_offset > 0 {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
        }
        self.lines.push_back(line);
    }

    /// Append raw bytes, splitting on newlines
    /// Returns the number of complete lines added
    pub fn push_bytes(&mut self, bytes: &[u8], partial_line: &mut String) -> usize {
        let text = String::from_utf8_lossy(bytes);
        let mut lines_added = 0;

        for ch in text.chars() {
            if ch == '\n' {
                self.push(std::mem::take(partial_line));
                lines_added += 1;
            } else if ch != '\r' {
                // Skip carriage returns, keep other characters
                partial_line.push(ch);
            }
        }
        lines_added
    }

    /// Get total number of lines in buffer
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Check if buffer is empty
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Clear all lines
    pub fn clear(&mut self) {
        self.lines.clear();
        self.scroll_offset = 0;
    }

    /// Get visible lines for a given viewport height
    /// Returns slice of lines to display based on scroll position
    pub fn visible_lines(&self, viewport_height: usize) -> Vec<&str> {
        if self.lines.is_empty() || viewport_height == 0 {
            return Vec::new();
        }

        let total = self.lines.len();
        // Calculate the end index (from bottom, accounting for scroll)
        let end = total.saturating_sub(self.scroll_offset);
        // Calculate start index
        let start = end.saturating_sub(viewport_height);

        self.lines
            .iter()
            .skip(start)
            .take(end - start)
            .map(|s| s.as_str())
            .collect()
    }

    /// Get all lines as an iterator
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.lines.iter()
    }

    /// Scroll up by n lines (toward older content)
    pub fn scroll_up(&mut self, n: usize) {
        let max_scroll = self.lines.len().saturating_sub(1);
        self.scroll_offset = (self.scroll_offset + n).min(max_scroll);
    }

    /// Scroll down by n lines (toward newer content)
    pub fn scroll_down(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    /// Scroll to bottom (most recent output)
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    /// Scroll to top (oldest output)
    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = self.lines.len().saturating_sub(1);
    }

    /// Check if scrolled to bottom
    pub fn is_at_bottom(&self) -> bool {
        self.scroll_offset == 0
    }

    /// Get current scroll offset
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }
}

/// A full session with PTY and virtual terminal
pub struct Session {
    /// Session metadata
    pub info: SessionInfo,
    /// PTY handle for I/O
    pub pty: PtyHandle,
    /// Virtual terminal emulator
    pub vterm: VirtualTerminal,
}

impl Session {
    /// Create a new session with the given info, PTY, and terminal dimensions
    pub fn new(info: SessionInfo, pty: PtyHandle, rows: usize, cols: usize) -> Self {
        Self {
            info,
            pty,
            vterm: VirtualTerminal::new(rows, cols),
        }
    }

    /// Create a new session with the given info, PTY, terminal dimensions, and scrollback
    pub fn with_scrollback(
        info: SessionInfo,
        pty: PtyHandle,
        rows: usize,
        cols: usize,
        scrollback_rows: usize,
    ) -> Self {
        Self {
            info,
            pty,
            vterm: VirtualTerminal::with_scrollback(rows, cols, scrollback_rows),
        }
    }

    /// Poll PTY for new output and process through virtual terminal
    /// Returns true if any output was read
    pub fn poll_output(&mut self) -> bool {
        match self.pty.try_read() {
            Ok(Some(bytes)) => {
                self.vterm.process(&bytes);
                self.info.last_activity = Utc::now();

                // If we're in Starting state and receiving output, Claude is running
                // Transition to Waiting (ready for user input)
                if self.info.state == SessionState::Starting {
                    self.set_state(SessionState::Waiting);
                }

                true
            }
            Ok(None) => false,
            Err(e) => {
                // PTY read error - log and store the reason
                let reason = format!("PTY read error: {}", e);
                tracing::warn!(
                    session_id = %self.info.id,
                    session_name = %self.info.name,
                    error = %e,
                    "PTY read error, transitioning to Exited state"
                );
                self.info.exit_reason = Some(reason);
                self.set_state(SessionState::Exited);
                false
            }
        }
    }

    /// Get visible lines for rendering (plain text)
    pub fn visible_lines(&self, viewport_height: usize) -> Vec<String> {
        self.vterm.visible_lines(viewport_height)
    }

    /// Get visible styled lines for rendering (with colors)
    /// Returns Rc to avoid cloning the entire vector on each render
    pub fn visible_styled_lines(
        &self,
        viewport_height: usize,
    ) -> Rc<Vec<ratatui::text::Line<'static>>> {
        self.vterm.visible_styled_lines(viewport_height)
    }

    /// Write bytes to the PTY
    pub fn write(&mut self, data: &[u8]) -> anyhow::Result<()> {
        self.pty.write(data)
    }

    /// Write pasted text to the PTY
    /// Wraps in bracketed paste sequences only if the app has enabled it
    pub fn write_paste(&mut self, text: &str) -> anyhow::Result<()> {
        if self.vterm.bracketed_paste_enabled() {
            self.pty.write_paste(text)
        } else {
            self.pty.write(text.as_bytes())
        }
    }

    /// Send a key event to the PTY
    /// If Enter is pressed while Waiting, transitions to Thinking state
    pub fn send_key(&mut self, key: crossterm::event::KeyEvent) -> anyhow::Result<()> {
        use crossterm::event::KeyCode;

        // When user presses Enter while waiting, Claude will start processing
        if key.code == KeyCode::Enter && self.info.state == SessionState::Waiting {
            self.set_state(SessionState::Thinking);
        }

        self.pty.send_key(key)
    }

    /// Check if the session's process is still running
    pub fn is_alive(&mut self) -> bool {
        self.pty.is_alive()
    }

    /// Get the exit status of the session's process (if exited)
    ///
    /// Returns:
    /// - `None` if the process is still running
    /// - `Some(ExitInfo)` if the process has exited
    pub fn exit_status(&mut self) -> Option<ExitInfo> {
        self.pty.exit_status()
    }

    /// Kill the session's process
    pub fn kill(&mut self) -> anyhow::Result<()> {
        self.pty.kill()
    }

    /// Resize the PTY and virtual terminal
    pub fn resize(&mut self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.pty.resize(rows, cols)?;
        self.vterm.resize(rows as usize, cols as usize);
        Ok(())
    }

    /// Update session state
    pub fn set_state(&mut self, state: SessionState) {
        let now = Utc::now();
        // Track when session exited for cleanup
        if state == SessionState::Exited && self.info.exited_at.is_none() {
            self.info.exited_at = Some(now);
        }
        self.info.state = state;
        self.info.last_activity = now;
        self.info.state_entered_at = now;
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Kill the PTY process if it's still alive to prevent orphaned processes
        if self.is_alive() {
            tracing::debug!("Killing session {} on drop", self.info.id);
            if let Err(e) = self.kill() {
                tracing::warn!("Failed to kill session {} on drop: {}", self.info.id, e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_state_display() {
        assert_eq!(SessionState::Starting.display_name(), "Starting");
        assert_eq!(SessionState::Thinking.display_name(), "Thinking");
        assert_eq!(
            SessionState::Executing("Read".to_string()).display_name(),
            "Executing"
        );
        assert_eq!(SessionState::Waiting.display_name(), "Waiting");
        assert_eq!(SessionState::Idle.display_name(), "Idle");
        assert_eq!(SessionState::Exited.display_name(), "Exited");
    }

    #[test]
    fn test_session_state_is_active() {
        assert!(SessionState::Starting.is_active());
        assert!(SessionState::Thinking.is_active());
        assert!(SessionState::Executing("test".to_string()).is_active());
        assert!(!SessionState::Waiting.is_active());
        assert!(!SessionState::Idle.is_active());
        assert!(!SessionState::Exited.is_active());
    }

    #[test]
    fn test_session_info_creation() {
        let project_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();
        let info = SessionInfo::new("test".to_string(), "/tmp".into(), project_id, branch_id);
        assert_eq!(info.name, "test");
        assert_eq!(info.state, SessionState::Starting);
        assert_eq!(info.project_id, project_id);
        assert_eq!(info.branch_id, branch_id);
        assert!(info.created_at <= Utc::now());
    }

    #[test]
    fn test_session_state_serialization() {
        let state = SessionState::Executing("Bash".to_string());
        let json = serde_json::to_string(&state).unwrap();
        let parsed: SessionState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, parsed);
    }

    #[test]
    fn test_session_type_default() {
        let session_type = SessionType::default();
        assert_eq!(session_type, SessionType::ClaudeCode);
    }

    #[test]
    fn test_session_type_display() {
        assert_eq!(SessionType::ClaudeCode.display_name(), "Claude Code");
        assert_eq!(SessionType::Shell.display_name(), "Shell");
    }

    #[test]
    fn test_session_type_uses_hooks() {
        assert!(SessionType::ClaudeCode.uses_hooks());
        assert!(!SessionType::Shell.uses_hooks());
    }

    #[test]
    fn test_session_info_shell() {
        let project_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();
        let info = SessionInfo::shell("shell".to_string(), "/tmp".into(), project_id, branch_id);
        assert_eq!(info.name, "shell");
        assert_eq!(info.session_type, SessionType::Shell);
        assert_eq!(info.state, SessionState::Waiting); // Shell starts in Waiting
    }

    #[test]
    fn test_output_buffer_push() {
        let mut buf = OutputBuffer::new(5);
        assert!(buf.is_empty());

        buf.push("line 1".to_string());
        buf.push("line 2".to_string());
        assert_eq!(buf.len(), 2);

        // Fill to capacity
        buf.push("line 3".to_string());
        buf.push("line 4".to_string());
        buf.push("line 5".to_string());
        assert_eq!(buf.len(), 5);

        // Push beyond capacity - oldest should be removed
        buf.push("line 6".to_string());
        assert_eq!(buf.len(), 5);

        let lines: Vec<&String> = buf.iter().collect();
        assert_eq!(lines[0], "line 2");
        assert_eq!(lines[4], "line 6");
    }

    #[test]
    fn test_output_buffer_push_bytes() {
        let mut buf = OutputBuffer::new(100);
        let mut partial = String::new();

        // Push bytes with newlines
        let lines_added = buf.push_bytes(b"hello\nworld\n", &mut partial);
        assert_eq!(lines_added, 2);
        assert_eq!(buf.len(), 2);
        assert!(partial.is_empty());

        // Push partial line
        let lines_added = buf.push_bytes(b"partial", &mut partial);
        assert_eq!(lines_added, 0);
        assert_eq!(partial, "partial");

        // Complete the partial line
        let lines_added = buf.push_bytes(b" line\n", &mut partial);
        assert_eq!(lines_added, 1);
        assert_eq!(buf.len(), 3);
        assert!(partial.is_empty());

        let lines: Vec<&String> = buf.iter().collect();
        assert_eq!(lines[2], "partial line");
    }

    #[test]
    fn test_output_buffer_scroll() {
        let mut buf = OutputBuffer::new(100);
        for i in 0..20 {
            buf.push(format!("line {}", i));
        }

        assert!(buf.is_at_bottom());
        assert_eq!(buf.scroll_offset(), 0);

        // Scroll up
        buf.scroll_up(5);
        assert!(!buf.is_at_bottom());
        assert_eq!(buf.scroll_offset(), 5);

        // Scroll down
        buf.scroll_down(3);
        assert_eq!(buf.scroll_offset(), 2);

        // Scroll to bottom
        buf.scroll_to_bottom();
        assert!(buf.is_at_bottom());

        // Scroll to top
        buf.scroll_to_top();
        assert_eq!(buf.scroll_offset(), 19);

        // Can't scroll past max
        buf.scroll_up(100);
        assert_eq!(buf.scroll_offset(), 19);
    }

    #[test]
    fn test_output_buffer_visible_lines() {
        let mut buf = OutputBuffer::new(100);
        for i in 0..10 {
            buf.push(format!("line {}", i));
        }

        // Viewport of 5 lines, at bottom
        let visible = buf.visible_lines(5);
        assert_eq!(visible.len(), 5);
        assert_eq!(visible[0], "line 5");
        assert_eq!(visible[4], "line 9");

        // Scroll up 3 lines
        buf.scroll_up(3);
        let visible = buf.visible_lines(5);
        assert_eq!(visible.len(), 5);
        assert_eq!(visible[0], "line 2");
        assert_eq!(visible[4], "line 6");

        // Viewport larger than content
        buf.scroll_to_bottom();
        let visible = buf.visible_lines(20);
        assert_eq!(visible.len(), 10);
    }

    #[test]
    fn test_output_buffer_clear() {
        let mut buf = OutputBuffer::new(100);
        buf.push("line 1".to_string());
        buf.push("line 2".to_string());
        buf.scroll_up(1);

        buf.clear();
        assert!(buf.is_empty());
        assert!(buf.is_at_bottom());
    }

    #[test]
    fn test_session_resize_updates_vterm() {
        // Create a session with spawn
        let pty = crate::session::pty::PtyHandle::spawn(
            "sleep",
            &["1"],
            std::path::Path::new("/tmp"),
            std::collections::HashMap::new(),
            24,
            80,
        )
        .unwrap();

        let project_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();
        let info = SessionInfo::new("test".to_string(), "/tmp".into(), project_id, branch_id);
        let mut session = Session::new(info, pty, 24, 80);

        // Initial size
        assert_eq!(session.vterm.size(), (24, 80));

        // Resize
        session.resize(100, 40).unwrap();

        // Verify vterm was resized (rows, cols)
        assert_eq!(session.vterm.size(), (40, 100));
    }
}
