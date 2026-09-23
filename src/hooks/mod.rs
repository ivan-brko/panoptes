//! Hooks module
//!
//! This module handles receiving state updates from agents via HTTP callbacks.
//! Claude Code's hook system sends POST requests when state changes occur.
//! Codex sends the same lifecycle events through its own hooks (0.156.1 and
//! later), and older Codex versions a single `AgentTurnComplete` through
//! `notify`.

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
/// Codex `notify` script (see `agent/codex.rs`) still emits it for Codex
/// versions without lifecycle hooks, and it must keep working there.
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

    /// The agent's own conversation ID, from the payload
    ///
    /// This is the payload's `session_id`, *not* [`HookEvent::session_id`]:
    /// that envelope field is the Panoptes session the event is routed to, and
    /// never changes. This one is Claude's conversation UUID, which does change
    /// inside a live process - `/clear`, an in-TUI `/resume` and `/branch` each
    /// move the process onto a different conversation and announce it through
    /// `SessionStart`.
    pub fn agent_conversation_id(&self) -> Option<&str> {
        self.str_field("session_id")
    }

    /// Where the agent is writing this conversation's transcript
    ///
    /// Claude reports the path it actually uses, which is worth preferring to
    /// one derived from the working directory: Claude resolves and slugs the
    /// directory itself, and the two need not agree.
    pub fn transcript_path(&self) -> Option<&std::path::Path> {
        self.str_field("transcript_path").map(std::path::Path::new)
    }

    /// The error code of a `StopFailure`, e.g. `rate_limit`
    ///
    /// Claude's own enum, the same one it writes to the transcript's `error`
    /// field - which is what lets both reports share
    /// [`crate::transcript::claude::failure_reason`].
    pub fn failure_code(&self) -> Option<&str> {
        self.str_field("error")
    }

    /// Free-text detail accompanying a `StopFailure`'s error code
    pub fn error_details(&self) -> Option<&str> {
        self.str_field("error_details")
    }

    /// Why a `PermissionDenied` was denied, in the agent's words
    pub fn denial_reason(&self) -> Option<&str> {
        self.str_field("reason")
    }

    /// The subagent a `SubagentStart` / `SubagentStop` is about
    ///
    /// The same ID on both, which is what pairs them: subagents run
    /// concurrently, so their ends arrive in any order. Codex also sets it on
    /// a subagent's own `UserPromptSubmit`, tool and permission events, which
    /// it sends under the *parent's* session - it is the only thing telling
    /// them apart from the parent's own.
    pub fn agent_id(&self) -> Option<&str> {
        self.str_field("agent_id")
    }

    /// The MCP server behind an `Elicitation` / `ElicitationResult`
    pub fn mcp_server_name(&self) -> Option<&str> {
        self.str_field("mcp_server_name")
    }

    /// The question an MCP server is asking, from an `Elicitation`
    pub fn message(&self) -> Option<&str> {
        self.str_field("message")
    }

    /// Background work still in flight, from a `Stop` / `SubagentStop`
    ///
    /// A snapshot, not a delta: Claude lists every running or pending
    /// backgrounded shell, monitor, subagent and workflow each time. `None`
    /// when the payload carries no list at all - an older Claude, or the
    /// no-`jq` degraded path - which must read as "unknown", not "none".
    pub fn background_tasks(&self) -> Option<&[serde_json::Value]> {
        self.array_field("background_tasks")
    }

    /// How many of [`Self::background_tasks`] are subagents
    pub fn background_subagents(&self) -> Option<usize> {
        let tasks = self.background_tasks()?;
        Some(
            tasks
                .iter()
                .filter(|task| task.get("type").and_then(|t| t.as_str()) == Some("subagent"))
                .count(),
        )
    }

    /// Session-scoped scheduled prompts (`/loop`, `CronCreate`,
    /// `ScheduleWakeup`) that will wake the session later, from a `Stop` /
    /// `SubagentStop`
    ///
    /// Snapshot semantics, and `None` when absent, exactly as
    /// [`Self::background_tasks`].
    pub fn session_crons(&self) -> Option<&[serde_json::Value]> {
        self.array_field("session_crons")
    }

    /// Read an array field out of the agent payload
    fn array_field(&self, key: &str) -> Option<&[serde_json::Value]> {
        self.payload.get(key)?.as_array().map(Vec::as_slice)
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

/// Known hook event types from Claude Code and Codex
///
/// The two agents share one vocabulary: Codex's lifecycle hooks deliberately
/// mirror Claude's names and payloads.
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
    /// A turn died on an API error instead of finishing (fires in place of
    /// `Stop`)
    StopFailure,
    /// A tool call was denied without a dialog - today, by auto mode's
    /// classifier
    PermissionDenied,
    /// A subagent began running
    SubagentStart,
    /// A subagent finished
    SubagentStop,
    /// An MCP server is asking the user a structured question
    Elicitation,
    /// The user answered, declined or cancelled an MCP elicitation
    ElicitationResult,
    /// The user interrupted the turn (Codex)
    ///
    /// Codex fires neither `Stop` nor `PostToolUse` for a turn cut short, so
    /// without this an aborted turn would never be reported as over.
    Interrupt,
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
            HookEventType::StopFailure => "StopFailure",
            HookEventType::PermissionDenied => "PermissionDenied",
            HookEventType::SubagentStart => "SubagentStart",
            HookEventType::SubagentStop => "SubagentStop",
            HookEventType::Elicitation => "Elicitation",
            HookEventType::ElicitationResult => "ElicitationResult",
            HookEventType::Interrupt => "Interrupt",
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
            "StopFailure" => HookEventType::StopFailure,
            "PermissionDenied" => HookEventType::PermissionDenied,
            "SubagentStart" => HookEventType::SubagentStart,
            "SubagentStop" => HookEventType::SubagentStop,
            "Elicitation" => HookEventType::Elicitation,
            "ElicitationResult" => HookEventType::ElicitationResult,
            "Interrupt" => HookEventType::Interrupt,
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
    fn test_conversation_accessors_read_the_payload_not_the_envelope() {
        // Shape captured from Claude Code 2.1.280 after `/clear`
        let e = event(
            r#"{"session_id":"panoptes-session","event":"SessionStart","timestamp":1,
                "payload":{"session_id":"b8be72f6-b45c-41de-83d6-c1f76e30d1dd",
                    "transcript_path":"/home/u/.claude/projects/-w/b8be72f6-b45c-41de-83d6-c1f76e30d1dd.jsonl",
                    "cwd":"/w","hook_event_name":"SessionStart","source":"clear"}}"#,
        );
        assert_eq!(e.session_id, "panoptes-session");
        assert_eq!(
            e.agent_conversation_id(),
            Some("b8be72f6-b45c-41de-83d6-c1f76e30d1dd")
        );
        assert_eq!(
            e.transcript_path(),
            Some(std::path::Path::new(
                "/home/u/.claude/projects/-w/b8be72f6-b45c-41de-83d6-c1f76e30d1dd.jsonl"
            ))
        );

        // The legacy flat shape has no payload, so no conversation ID - the
        // envelope's Panoptes ID must never be mistaken for one
        let flat = event(r#"{"session_id":"abc","event":"SessionStart","timestamp":1}"#);
        assert_eq!(flat.agent_conversation_id(), None);
        assert_eq!(flat.transcript_path(), None);
    }

    #[test]
    fn test_stop_payload_accessors() {
        let e = event(
            r#"{"session_id":"a","event":"Stop","timestamp":1,
                "payload":{"stop_hook_active":false,"last_assistant_message":"Done."}}"#,
        );
        assert_eq!(e.last_assistant_message(), Some("Done."));
    }

    // The payloads below follow the hook input schemas in Claude Code 2.1.280
    // (the zod definitions in the shipped binary), base fields included.

    #[test]
    fn test_stop_failure_accessors() {
        let e = event(
            r#"{"session_id":"a","event":"StopFailure","timestamp":1,
                "payload":{"session_id":"c","transcript_path":"/t.jsonl","cwd":"/w",
                    "hook_event_name":"StopFailure","error":"rate_limit",
                    "error_details":"429 Too Many Requests",
                    "last_assistant_message":"You've hit your limit · resets 3pm"}}"#,
        );
        assert_eq!(e.event_type(), HookEventType::StopFailure);
        assert_eq!(e.failure_code(), Some("rate_limit"));
        assert_eq!(e.error_details(), Some("429 Too Many Requests"));
        assert_eq!(
            e.last_assistant_message(),
            Some("You've hit your limit · resets 3pm")
        );
    }

    #[test]
    fn test_permission_denied_accessors() {
        let e = event(
            r#"{"session_id":"a","event":"PermissionDenied","timestamp":1,
                "payload":{"session_id":"c","transcript_path":"/t.jsonl","cwd":"/w",
                    "hook_event_name":"PermissionDenied","tool_name":"Bash",
                    "tool_input":{"command":"rm -rf build"},"tool_use_id":"toolu_07",
                    "reason":"Deleting files outside the task"}}"#,
        );
        assert_eq!(e.event_type(), HookEventType::PermissionDenied);
        assert_eq!(e.tool_name(), Some("Bash"));
        assert_eq!(e.tool_use_id(), Some("toolu_07"));
        assert_eq!(e.denial_reason(), Some("Deleting files outside the task"));
    }

    #[test]
    fn test_subagent_accessors_pair_start_and_stop() {
        let start = event(
            r#"{"session_id":"a","event":"SubagentStart","timestamp":1,
                "payload":{"session_id":"c","transcript_path":"/t.jsonl","cwd":"/w",
                    "hook_event_name":"SubagentStart","agent_id":"a1b2c3",
                    "agent_type":"Explore"}}"#,
        );
        let stop = event(
            r#"{"session_id":"a","event":"SubagentStop","timestamp":2,
                "payload":{"session_id":"c","transcript_path":"/t.jsonl","cwd":"/w",
                    "hook_event_name":"SubagentStop","stop_hook_active":false,
                    "agent_id":"a1b2c3","agent_type":"Explore",
                    "agent_transcript_path":"/t/subagents/agent-a1b2c3.jsonl",
                    "last_assistant_message":"Found it.",
                    "background_tasks":[],"session_crons":[]}}"#,
        );
        assert_eq!(start.event_type(), HookEventType::SubagentStart);
        assert_eq!(stop.event_type(), HookEventType::SubagentStop);
        assert_eq!(start.agent_id(), Some("a1b2c3"));
        assert_eq!(stop.agent_id(), start.agent_id());
        assert_eq!(stop.background_tasks().map(<[_]>::len), Some(0));
    }

    #[test]
    fn test_stop_background_work_accessors() {
        let e = event(
            r#"{"session_id":"a","event":"Stop","timestamp":1,
                "payload":{"session_id":"c","transcript_path":"/t.jsonl","cwd":"/w",
                    "hook_event_name":"Stop","stop_hook_active":false,
                    "last_assistant_message":"Started the dev server.",
                    "background_tasks":[
                        {"id":"b1","type":"shell","status":"running",
                         "description":"npm run dev","command":"npm run dev"},
                        {"id":"b2","type":"subagent","status":"running",
                         "description":"Audit deps","agent_type":"general-purpose"},
                        {"id":"b3","type":"monitor","status":"running",
                         "description":"CI","server":"github","tool":"watch_run"}],
                    "session_crons":[
                        {"id":"k1","schedule":"*/5 * * * *","recurring":true,
                         "prompt":"check the deploy"}]}}"#,
        );
        assert_eq!(e.background_tasks().map(<[_]>::len), Some(3));
        assert_eq!(e.background_subagents(), Some(1));
        assert_eq!(e.session_crons().map(<[_]>::len), Some(1));

        // An older Claude, or the no-jq path, says nothing - which is not the
        // same as saying "nothing is running"
        let bare = event(
            r#"{"session_id":"a","event":"Stop","timestamp":1,
                "payload":{"stop_hook_active":false}}"#,
        );
        assert_eq!(bare.background_tasks(), None);
        assert_eq!(bare.background_subagents(), None);
        assert_eq!(bare.session_crons(), None);
    }

    #[test]
    fn test_elicitation_accessors() {
        let ask = event(
            r#"{"session_id":"a","event":"Elicitation","timestamp":1,
                "payload":{"session_id":"c","transcript_path":"/t.jsonl","cwd":"/w",
                    "hook_event_name":"Elicitation","mcp_server_name":"linear",
                    "message":"Which team should own this issue?","mode":"form",
                    "elicitation_id":"el-1","requested_schema":{"type":"object"}}}"#,
        );
        let answer = event(
            r#"{"session_id":"a","event":"ElicitationResult","timestamp":2,
                "payload":{"session_id":"c","transcript_path":"/t.jsonl","cwd":"/w",
                    "hook_event_name":"ElicitationResult","mcp_server_name":"linear",
                    "elicitation_id":"el-1","mode":"form","action":"accept",
                    "content":{"team":"Platform"}}}"#,
        );
        assert_eq!(ask.event_type(), HookEventType::Elicitation);
        assert_eq!(answer.event_type(), HookEventType::ElicitationResult);
        assert_eq!(ask.mcp_server_name(), Some("linear"));
        assert_eq!(ask.message(), Some("Which team should own this issue?"));
        assert_eq!(answer.mcp_server_name(), ask.mcp_server_name());
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
        // Real Claude events Panoptes deliberately does not model stay unknown
        for unmodelled in [
            "TaskCreated",
            "TaskCompleted",
            "CwdChanged",
            "PostToolBatch",
        ] {
            assert_eq!(HookEventType::from(unmodelled), HookEventType::Unknown);
        }
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
            HookEventType::StopFailure,
            HookEventType::PermissionDenied,
            HookEventType::SubagentStart,
            HookEventType::SubagentStop,
            HookEventType::Elicitation,
            HookEventType::ElicitationResult,
            HookEventType::AgentTurnComplete,
        ] {
            let str_repr = event_type.as_str();
            let parsed: HookEventType = str_repr.into();
            assert_eq!(parsed, event_type);
        }
    }
}
