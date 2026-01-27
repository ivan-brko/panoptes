//! Hooks module
//!
//! This module handles receiving state updates from Claude Code via HTTP callbacks.
//! Claude Code's hook system sends POST requests when state changes occur.

pub mod server;

pub use server::{
    DroppedEventsCounter, HookEventReceiver, HookEventSender, ServerHandle, ServerStatus,
    DEFAULT_CHANNEL_BUFFER,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Event received from a Claude Code hook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookEvent {
    /// Claude Code's session ID
    pub session_id: String,

    /// Event type (e.g., "PreToolUse", "PostToolUse", "Stop")
    pub event: String,

    /// Tool name (for tool-related events)
    #[serde(default)]
    pub tool: Option<String>,

    /// Unix timestamp when the event occurred
    pub timestamp: i64,
}

impl HookEvent {
    /// Get the event timestamp as a DateTime
    pub fn datetime(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(self.timestamp, 0).unwrap_or_else(Utc::now)
    }
}

/// Known hook event types from Claude Code
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEventType {
    /// Session has started
    SessionStart,
    /// Session has stopped
    Stop,
    /// About to use a tool
    PreToolUse,
    /// Finished using a tool
    PostToolUse,
    /// Notification from Claude (e.g., waiting for input)
    Notification,
    /// Unknown event type
    Unknown,
}

impl HookEventType {
    /// Get the string name of this event type as used by Claude Code
    pub fn as_str(&self) -> &'static str {
        match self {
            HookEventType::SessionStart => "SessionStart",
            HookEventType::Stop => "Stop",
            HookEventType::PreToolUse => "PreToolUse",
            HookEventType::PostToolUse => "PostToolUse",
            HookEventType::Notification => "Notification",
            HookEventType::Unknown => "Unknown",
        }
    }
}

impl std::fmt::Display for HookEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<&str> for HookEventType {
    fn from(s: &str) -> Self {
        match s {
            "SessionStart" => HookEventType::SessionStart,
            "Stop" => HookEventType::Stop,
            "PreToolUse" => HookEventType::PreToolUse,
            "PostToolUse" => HookEventType::PostToolUse,
            "Notification" => HookEventType::Notification,
            _ => HookEventType::Unknown,
        }
    }
}

impl HookEvent {
    /// Get the typed event
    pub fn event_type(&self) -> HookEventType {
        HookEventType::from(self.event.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hook_event_parsing() {
        let json = r#"{
            "session_id": "abc123",
            "event": "PreToolUse",
            "tool": "Bash",
            "timestamp": 1704067200
        }"#;

        let event: HookEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.session_id, "abc123");
        assert_eq!(event.event, "PreToolUse");
        assert_eq!(event.tool, Some("Bash".to_string()));
        assert_eq!(event.event_type(), HookEventType::PreToolUse);
    }

    #[test]
    fn test_hook_event_without_tool() {
        let json = r#"{
            "session_id": "abc123",
            "event": "Stop",
            "timestamp": 1704067200
        }"#;

        let event: HookEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.tool, None);
        assert_eq!(event.event_type(), HookEventType::Stop);
    }

    #[test]
    fn test_hook_event_type_conversion() {
        assert_eq!(
            HookEventType::from("SessionStart"),
            HookEventType::SessionStart
        );
        assert_eq!(HookEventType::from("Stop"), HookEventType::Stop);
        assert_eq!(HookEventType::from("PreToolUse"), HookEventType::PreToolUse);
        assert_eq!(
            HookEventType::from("PostToolUse"),
            HookEventType::PostToolUse
        );
        assert_eq!(HookEventType::from("SomethingElse"), HookEventType::Unknown);
    }

    #[test]
    fn test_hook_event_datetime() {
        let event = HookEvent {
            session_id: "test".to_string(),
            event: "Stop".to_string(),
            tool: None,
            timestamp: 1704067200,
        };

        let dt = event.datetime();
        assert_eq!(dt.timestamp(), 1704067200);
    }

    #[test]
    fn test_hook_event_type_as_str() {
        assert_eq!(HookEventType::SessionStart.as_str(), "SessionStart");
        assert_eq!(HookEventType::Stop.as_str(), "Stop");
        assert_eq!(HookEventType::PreToolUse.as_str(), "PreToolUse");
        assert_eq!(HookEventType::PostToolUse.as_str(), "PostToolUse");
        assert_eq!(HookEventType::Notification.as_str(), "Notification");
        assert_eq!(HookEventType::Unknown.as_str(), "Unknown");
    }

    #[test]
    fn test_hook_event_type_display() {
        assert_eq!(format!("{}", HookEventType::PreToolUse), "PreToolUse");
        assert_eq!(format!("{}", HookEventType::Stop), "Stop");
    }

    #[test]
    fn test_hook_event_type_roundtrip() {
        // Verify that as_str() and From<&str> are consistent
        for event_type in [
            HookEventType::SessionStart,
            HookEventType::Stop,
            HookEventType::PreToolUse,
            HookEventType::PostToolUse,
            HookEventType::Notification,
        ] {
            let str_repr = event_type.as_str();
            let parsed: HookEventType = str_repr.into();
            assert_eq!(parsed, event_type);
        }
    }
}
