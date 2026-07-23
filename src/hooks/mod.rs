//! Hooks module
//!
//! This module handles receiving state updates from agents via HTTP callbacks.
//! Claude Code's hook system sends POST requests when state changes occur;
//! Codex CLI sends a single `AgentTurnComplete` through its `notify` hook.

pub mod server;

pub use server::{
    DroppedEventsCounter, HookEventReceiver, HookEventSender, ServerHandle, ServerStatus,
    DEFAULT_CHANNEL_BUFFER,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Event received from an agent hook
///
/// The wire format is an *envelope*: Panoptes' own routing fields sit at the
/// top level and the agent's payload is nested untouched under `payload`.
///
/// Nesting rather than merging is deliberate. Claude's payload carries its own
/// `session_id` (the conversation UUID) and merging would have it fight with
/// ours for the same key, with the winner depending on argument order in a
/// shell script. Nested, the two can never collide.
///
/// `payload` is `#[serde(default)]` so the older flat shape still parses. The
/// Codex `notify` script (see `agent/codex.rs`) still emits it, and it must
/// keep working: Codex hooks cannot be extended without stalling its output
/// pipeline, so PAN-3 replaces that channel rather than this ticket widening it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookEvent {
    /// The Panoptes session ID this event belongs to (from `PANOPTES_SESSION_ID`)
    pub session_id: String,

    /// Event type (e.g., "PreToolUse", "PostToolUse", "Stop")
    pub event: String,

    /// Unix timestamp when the event occurred
    pub timestamp: i64,

    /// The agent's own hook payload, forwarded verbatim
    ///
    /// Empty (`Null`) when the sender could not build it — see the no-`jq`
    /// degraded path in `agent/claude.rs`.
    #[serde(default)]
    pub payload: serde_json::Value,
}

impl HookEvent {
    /// Get the event timestamp as a DateTime
    pub fn datetime(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(self.timestamp, 0).unwrap_or_else(Utc::now)
    }

    /// Get the typed event
    pub fn event_type(&self) -> HookEventType {
        HookEventType::from(self.event.as_str())
    }

    /// Read a string field out of the agent payload
    fn str_field(&self, key: &str) -> Option<&str> {
        self.payload.get(key)?.as_str().filter(|s| !s.is_empty())
    }

    /// Tool name, for tool-related events
    pub fn tool_name(&self) -> Option<&str> {
        self.str_field("tool_name")
    }

    /// The agent's own identifier for this specific tool invocation
    ///
    /// Present on `PreToolUse` and `PostToolUse`, which is what lets a late
    /// `PostToolUse` retire the right entry instead of whichever ran last.
    pub fn tool_use_id(&self) -> Option<&str> {
        self.str_field("tool_use_id")
    }

    /// Stable key for tracking a tool across its Pre/Post pair
    ///
    /// Prefers `tool_use_id`, which is unique per invocation. Falls back to the
    /// tool name when the payload is unavailable (the no-`jq` path), which is
    /// correct for serial tool use and merely imprecise for concurrent use.
    pub fn tool_key(&self) -> String {
        self.tool_use_id()
            .or_else(|| self.tool_name())
            .unwrap_or("unknown")
            .to_string()
    }

    /// Classified `notification_type` from a `Notification` event
    pub fn notification_kind(&self) -> NotificationKind {
        NotificationKind::from(self.str_field("notification_type").unwrap_or_default())
    }

    /// The agent's suggested title for this conversation
    pub fn session_title(&self) -> Option<&str> {
        self.str_field("session_title")
    }

    /// What caused a `SessionStart`
    ///
    /// `None` when the payload carried no `source` at all — the no-`jq`
    /// degraded path — which callers should treat as cautiously as
    /// [`SessionStartSource::Other`].
    pub fn session_start_source(&self) -> Option<SessionStartSource> {
        self.str_field("source").map(SessionStartSource::from)
    }

    /// The last thing the assistant said, from a `Stop` event
    pub fn last_assistant_message(&self) -> Option<&str> {
        self.str_field("last_assistant_message")
    }
}

/// What a Claude `Notification` event is actually about
///
/// Claude fires `Notification` both for "I need your approval right now" and
/// for a periodic "you have been idle" nag. Treating them alike is why every
/// notification used to ring the bell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    /// Periodic reminder that the session has been sitting unattended
    Idle,
    /// A permission dialog is open and blocking the turn
    PermissionRequest,
    /// The agent finished what it was asked to do
    TaskCompleted,
    /// The agent is asking the user a question inline
    Elicitation,
    /// Something happened that the user does not have to act on
    ///
    /// Reported so the event is recognised rather than falling through to
    /// [`NotificationKind::Other`], which assumes the user is wanted.
    Informational,
    /// Anything Claude adds later that we do not recognise
    Other,
}

impl From<&str> for NotificationKind {
    /// Classify the `notification_type` field of a Claude `Notification` hook
    ///
    /// The values are Claude Code's own, taken from the matcher metadata it
    /// ships (`fieldToMatch: "notification_type"`). Getting one wrong is not a
    /// cosmetic mistake: an unrecognised value falls through to `Other`, which
    /// is treated as "the agent wants you", so it both rings the bell and
    /// flags the session.
    fn from(s: &str) -> Self {
        match s {
            "idle_prompt" => NotificationKind::Idle,
            "permission_prompt" => NotificationKind::PermissionRequest,
            "agent_completed" => NotificationKind::TaskCompleted,
            // The agent is blocked on the user either way
            "elicitation_dialog" | "agent_needs_input" => NotificationKind::Elicitation,
            // The dialog resolved, or a background event the user need not act on
            "elicitation_complete" | "elicitation_response" | "auth_success" => {
                NotificationKind::Informational
            }
            _ => NotificationKind::Other,
        }
    }
}

/// What caused a Claude `SessionStart` event
///
/// `SessionStart` does not only mean "a process came up". `startup` and
/// `resume` do, but `clear` and `fork` fire inside a live process — and all
/// four begin a fresh conversation with nothing running and nothing owed to
/// the user. `compact` is the odd one out: it fires in the *middle* of a turn
/// the agent is still working on, without the user doing anything at all,
/// whenever the context window fills up — so it must never be treated as a
/// conversation boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStartSource {
    /// A fresh process came up
    Startup,
    /// A process came up reattached to an existing conversation
    Resume,
    /// The user ran `/clear`, starting a fresh conversation in place
    Clear,
    /// The conversation was forked into a new one
    Fork,
    /// Automatic context compaction, mid-turn, with no user involvement
    Compact,
    /// A source added after this was written
    Other,
}

impl SessionStartSource {
    /// Whether this source begins a fresh conversation
    ///
    /// True for everything except `compact`, which interrupts a turn the
    /// agent is still working on, and unrecognised sources, where guessing
    /// "fresh" could wrongly report a busy session as idle.
    pub fn is_fresh_conversation(&self) -> bool {
        matches!(
            self,
            SessionStartSource::Startup
                | SessionStartSource::Resume
                | SessionStartSource::Clear
                | SessionStartSource::Fork
        )
    }
}

impl From<&str> for SessionStartSource {
    /// Classify the `source` field of a Claude `SessionStart` hook
    ///
    /// The values are Claude Code's own schema:
    /// `"startup" | "resume" | "clear" | "compact" | "fork"`.
    fn from(s: &str) -> Self {
        match s {
            "startup" => SessionStartSource::Startup,
            "resume" => SessionStartSource::Resume,
            "clear" => SessionStartSource::Clear,
            "fork" => SessionStartSource::Fork,
            "compact" => SessionStartSource::Compact,
            _ => SessionStartSource::Other,
        }
    }
}

/// Known hook event types from Claude Code
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEventType {
    /// Session has started
    SessionStart,
    /// Session has ended (carries a `reason`)
    SessionEnd,
    /// The user submitted a prompt, so a turn is beginning
    ///
    /// This is what makes `Thinking` an observation rather than a guess: it
    /// fires however the prompt arrived, including paste and initial prompts.
    UserPromptSubmit,
    /// Session has stopped
    Stop,
    /// About to use a tool
    PreToolUse,
    /// Finished using a tool
    PostToolUse,
    /// A tool failed or was interrupted
    PostToolUseFailure,
    /// Notification from Claude (e.g., waiting for input)
    Notification,
    /// Permission request (Claude is waiting for user to approve/deny)
    PermissionRequest,
    /// Agent turn complete (from Codex CLI notify hook)
    AgentTurnComplete,
    /// Unknown event type
    Unknown,
}

impl HookEventType {
    /// Get the string name of this event type as used by Claude Code
    pub fn as_str(&self) -> &'static str {
        match self {
            HookEventType::SessionStart => "SessionStart",
            HookEventType::SessionEnd => "SessionEnd",
            HookEventType::UserPromptSubmit => "UserPromptSubmit",
            HookEventType::Stop => "Stop",
            HookEventType::PreToolUse => "PreToolUse",
            HookEventType::PostToolUse => "PostToolUse",
            HookEventType::PostToolUseFailure => "PostToolUseFailure",
            HookEventType::Notification => "Notification",
            HookEventType::PermissionRequest => "PermissionRequest",
            HookEventType::AgentTurnComplete => "AgentTurnComplete",
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
            "SessionEnd" => HookEventType::SessionEnd,
            "UserPromptSubmit" => HookEventType::UserPromptSubmit,
            "Stop" => HookEventType::Stop,
            "PreToolUse" => HookEventType::PreToolUse,
            "PostToolUse" => HookEventType::PostToolUse,
            "PostToolUseFailure" => HookEventType::PostToolUseFailure,
            "Notification" => HookEventType::Notification,
            "PermissionRequest" => HookEventType::PermissionRequest,
            "AgentTurnComplete" => HookEventType::AgentTurnComplete,
            _ => HookEventType::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(json: &str) -> HookEvent {
        serde_json::from_str(json).expect("valid hook event")
    }

    #[test]
    fn test_hook_event_parsing() {
        let e = event(
            r#"{
                "session_id": "abc123",
                "event": "PreToolUse",
                "timestamp": 1704067200,
                "payload": {"tool_name": "Bash", "tool_use_id": "toolu_01"}
            }"#,
        );

        assert_eq!(e.session_id, "abc123");
        assert_eq!(e.event, "PreToolUse");
        assert_eq!(e.tool_name(), Some("Bash"));
        assert_eq!(e.tool_use_id(), Some("toolu_01"));
        assert_eq!(e.event_type(), HookEventType::PreToolUse);
    }

    #[test]
    fn test_hook_event_without_payload() {
        let e = event(r#"{"session_id": "abc123", "event": "Stop", "timestamp": 1704067200}"#);

        assert!(e.payload.is_null());
        assert_eq!(e.tool_name(), None);
        assert_eq!(e.event_type(), HookEventType::Stop);
    }

    #[test]
    fn test_legacy_codex_shape_still_parses() {
        // The Codex notify script still emits the pre-envelope flat shape with a
        // `tool` field. It must keep working untouched.
        let e =
            event(r#"{"session_id":"abc","event":"AgentTurnComplete","tool":"","timestamp":123}"#);

        assert_eq!(e.event_type(), HookEventType::AgentTurnComplete);
        assert!(e.payload.is_null());
        assert_eq!(e.tool_name(), None);
    }

    #[test]
    fn test_payload_survives_quotes_and_newlines() {
        // The whole point of forwarding through jq: a tool input containing a
        // quote or a newline used to produce invalid JSON and lose the event.
        let e = event(
            r#"{
                "session_id":"abc","event":"PreToolUse","timestamp":1,
                "payload":{"tool_name":"Bash","tool_input":{"command":"echo \"hi\"\nls"}}
            }"#,
        );

        assert_eq!(e.tool_name(), Some("Bash"));
        assert_eq!(
            e.payload["tool_input"]["command"].as_str(),
            Some("echo \"hi\"\nls")
        );
    }

    #[test]
    fn test_tool_key_prefers_tool_use_id() {
        let with_id = event(
            r#"{"session_id":"a","event":"PreToolUse","timestamp":1,
                "payload":{"tool_name":"Read","tool_use_id":"toolu_99"}}"#,
        );
        assert_eq!(with_id.tool_key(), "toolu_99");

        // Degraded path: no tool_use_id, fall back to the name
        let without_id = event(
            r#"{"session_id":"a","event":"PreToolUse","timestamp":1,
                "payload":{"tool_name":"Read"}}"#,
        );
        assert_eq!(without_id.tool_key(), "Read");

        // Fully degraded: no payload at all
        let bare = event(r#"{"session_id":"a","event":"PreToolUse","timestamp":1}"#);
        assert_eq!(bare.tool_key(), "unknown");
    }

    #[test]
    fn test_notification_kind_classification() {
        // These strings are Claude Code's own, not ours. They come from the
        // matcher metadata the CLI ships for the Notification event.
        let idle = event(
            r#"{"session_id":"a","event":"Notification","timestamp":1,
                "payload":{"notification_type":"idle_prompt","message":"still there?"}}"#,
        );
        assert_eq!(idle.notification_kind(), NotificationKind::Idle);

        let perm = event(
            r#"{"session_id":"a","event":"Notification","timestamp":1,
                "payload":{"notification_type":"permission_prompt"}}"#,
        );
        assert_eq!(
            perm.notification_kind(),
            NotificationKind::PermissionRequest
        );

        for (value, expected) in [
            ("agent_completed", NotificationKind::TaskCompleted),
            ("elicitation_dialog", NotificationKind::Elicitation),
            ("agent_needs_input", NotificationKind::Elicitation),
            ("elicitation_complete", NotificationKind::Informational),
            ("elicitation_response", NotificationKind::Informational),
            ("auth_success", NotificationKind::Informational),
        ] {
            let e = event(&format!(
                r#"{{"session_id":"a","event":"Notification","timestamp":1,
                    "payload":{{"notification_type":"{}"}}}}"#,
                value
            ));
            assert_eq!(e.notification_kind(), expected, "for {}", value);
        }

        // An unrecognised or absent type must not masquerade as idle
        let unknown =
            event(r#"{"session_id":"a","event":"Notification","timestamp":1,"payload":{}}"#);
        assert_eq!(unknown.notification_kind(), NotificationKind::Other);
    }

    #[test]
    fn test_session_start_source_classification() {
        // The values are Claude Code's own schema for SessionStart.source
        for (value, expected, fresh) in [
            ("startup", SessionStartSource::Startup, true),
            ("resume", SessionStartSource::Resume, true),
            ("clear", SessionStartSource::Clear, true),
            ("fork", SessionStartSource::Fork, true),
            // Compaction fires mid-turn; treating it as a fresh conversation
            // would report a busy session as idle
            ("compact", SessionStartSource::Compact, false),
            ("something_new", SessionStartSource::Other, false),
        ] {
            let e = event(&format!(
                r#"{{"session_id":"a","event":"SessionStart","timestamp":1,
                    "payload":{{"source":"{}"}}}}"#,
                value
            ));
            assert_eq!(e.session_start_source(), Some(expected), "for {}", value);
            assert_eq!(
                expected.is_fresh_conversation(),
                fresh,
                "is_fresh_conversation for {}",
                value
            );
        }

        // The no-jq degraded path delivers no payload at all
        let bare = event(r#"{"session_id":"a","event":"SessionStart","timestamp":1}"#);
        assert_eq!(bare.session_start_source(), None);
    }

    #[test]
    fn test_stop_payload_accessors() {
        let e = event(
            r#"{"session_id":"a","event":"Stop","timestamp":1,
                "payload":{"stop_hook_active":false,"last_assistant_message":"Done."}}"#,
        );
        assert_eq!(e.last_assistant_message(), Some("Done."));
    }

    #[test]
    fn test_empty_strings_read_as_absent() {
        // The no-jq path and Claude both emit "" rather than omitting keys in
        // places; an empty tool name is not a tool name.
        let e = event(
            r#"{"session_id":"a","event":"PreToolUse","timestamp":1,
                "payload":{"tool_name":"","session_title":""}}"#,
        );
        assert_eq!(e.tool_name(), None);
        assert_eq!(e.session_title(), None);
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
            HookEventType::from("UserPromptSubmit"),
            HookEventType::UserPromptSubmit
        );
        assert_eq!(HookEventType::from("SomethingElse"), HookEventType::Unknown);
    }

    #[test]
    fn test_hook_event_datetime() {
        let e = HookEvent {
            session_id: "test".to_string(),
            event: "Stop".to_string(),
            timestamp: 1704067200,
            payload: serde_json::Value::Null,
        };

        assert_eq!(e.datetime().timestamp(), 1704067200);
    }

    #[test]
    fn test_hook_event_type_roundtrip() {
        // Verify that as_str() and From<&str> are consistent
        for event_type in [
            HookEventType::SessionStart,
            HookEventType::SessionEnd,
            HookEventType::UserPromptSubmit,
            HookEventType::Stop,
            HookEventType::PreToolUse,
            HookEventType::PostToolUse,
            HookEventType::PostToolUseFailure,
            HookEventType::Notification,
            HookEventType::PermissionRequest,
            HookEventType::AgentTurnComplete,
        ] {
            let str_repr = event_type.as_str();
            let parsed: HookEventType = str_repr.into();
            assert_eq!(parsed, event_type);
        }
    }
}
