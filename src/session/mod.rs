//! Session management module
//!
//! This module handles Claude Code session lifecycle, PTY management,
//! and session state tracking.

pub mod pty;

// Submodules will be added in later tickets:
// pub mod manager;

pub use pty::PtyHandle;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a session
pub type SessionId = Uuid;

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
        use ratatui::style::Color;
        match self {
            SessionState::Starting => Color::Blue,
            SessionState::Thinking => Color::Yellow,
            SessionState::Executing(_) => Color::Cyan,
            SessionState::Waiting => Color::Green,
            SessionState::Idle => Color::DarkGray,
            SessionState::Exited => Color::Red,
        }
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
    /// Working directory
    pub working_dir: std::path::PathBuf,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last activity timestamp
    pub last_activity: DateTime<Utc>,
}

impl SessionInfo {
    /// Create new session info
    pub fn new(name: String, working_dir: std::path::PathBuf) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            state: SessionState::default(),
            working_dir,
            created_at: now,
            last_activity: now,
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
        let info = SessionInfo::new("test".to_string(), "/tmp".into());
        assert_eq!(info.name, "test");
        assert_eq!(info.state, SessionState::Starting);
        assert!(info.created_at <= Utc::now());
    }

    #[test]
    fn test_session_state_serialization() {
        let state = SessionState::Executing("Bash".to_string());
        let json = serde_json::to_string(&state).unwrap();
        let parsed: SessionState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, parsed);
    }
}
