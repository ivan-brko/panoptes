//! Session management module
//!
//! This module handles Claude Code session lifecycle, PTY management,
//! and session state tracking.

pub mod manager;
pub mod pty;
pub mod state_machine;
pub mod store;
pub mod vterm;

pub use manager::{AgentAccount, NewSessionSpec, SessionManager};
pub use pty::{mouse_event_to_bytes, ExitInfo, PtyHandle, PtyWriteTimedOut};
pub use store::{sessions_file_path, SessionStore};
pub use vterm::{VirtualTerminal, DEFAULT_SCROLLBACK_ROWS};

use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::claude_config::ClaudeConfigId;
use crate::codex_config::CodexConfigId;
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
    /// OpenAI Codex CLI session - uses notify hook for state tracking
    OpenAICodex,
}

impl SessionType {
    /// Get the display name for this session type
    pub fn display_name(&self) -> &str {
        match self {
            SessionType::ClaudeCode => "Claude Code",
            SessionType::Shell => "Shell",
            SessionType::OpenAICodex => "Codex",
        }
    }

    /// Get a short tag for display in session lists (e.g. [CC], [CX], [SH])
    pub fn short_tag(&self) -> &str {
        match self {
            SessionType::ClaudeCode => "[CC]",
            SessionType::Shell => "[SH]",
            SessionType::OpenAICodex => "[CX]",
        }
    }

    /// Check if this session type uses hooks for state tracking
    pub fn uses_hooks(&self) -> bool {
        matches!(self, SessionType::ClaudeCode | SessionType::OpenAICodex)
    }

    /// Whether this agent tells Panoptes when the user submits a prompt
    ///
    /// Claude Code fires `UserPromptSubmit`, so the start of a turn is
    /// observed. Codex has no equivalent - its `notify` hook cannot be extended
    /// without stalling its output pipeline - so a Codex session falls back to
    /// guessing from the Enter keystroke until its rollout confirms the turn.
    pub fn reports_prompt_submission(&self) -> bool {
        matches!(self, SessionType::ClaudeCode)
    }

    /// Whether this agent reports individual tool starts and finishes
    ///
    /// Claude Code sends `PreToolUse`/`PostToolUse`; Codex writes
    /// `function_call` / `function_call_output` to its rollout. Both therefore
    /// populate `in_flight`, and both need the stall watchdog as a backstop for
    /// a completion that never arrives.
    ///
    /// Shell sessions reach `Executing` through foreground-process detection
    /// and never populate `in_flight`, so an empty set says nothing about them.
    pub fn reports_tool_use(&self) -> bool {
        matches!(self, SessionType::ClaudeCode | SessionType::OpenAICodex)
    }
}

/// State of a session
///
/// Every variant is a unit variant on purpose. In-flight tool names used to
/// ride along inside `Executing(String)`, which meant the state changed
/// identity every time a different tool started and could only ever name one
/// of several concurrent tools. Tools now live in [`SessionInfo::in_flight`];
/// this enum answers one question only, "what is this session doing".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub enum SessionState {
    /// Spawned, but the agent has not reported in yet
    #[default]
    Starting,
    /// Working, with no tool currently in flight
    Thinking,
    /// One or more tools are in flight (see `in_flight` for which)
    Executing,
    /// Blocked on a permission dialog, waiting for the user to approve or deny
    ///
    /// Distinct from `Waiting`: both want the user, but this one is holding a
    /// turn open and cannot make progress, while `Waiting` has finished.
    AwaitingApproval,
    /// The turn is over; the agent is waiting for the next prompt
    Waiting,
    /// Deliberately killed by Panoptes to reclaim memory, scrollback retained
    ///
    /// Written by the idle sweep (`SessionManager::suspend_idle_sessions`)
    /// once a resumable session has sat idle past the configured threshold.
    Suspended,
    /// The process died on its own - see `exit_reason` for why
    Exited,
    /// Session belongs to a previous Panoptes run and can be brought back
    ///
    /// Distinct from `Exited`: a session you deliberately closed and one
    /// orphaned by a crash should not look the same in the list. This state is
    /// only ever assigned at startup, when reconciling the durable store
    /// against the (empty) set of running processes.
    Resumable,
}

impl SessionState {
    /// Get the display name for this state
    pub fn display_name(&self) -> &str {
        match self {
            SessionState::Starting => "Starting",
            SessionState::Thinking => "Thinking",
            SessionState::Executing => "Executing",
            SessionState::AwaitingApproval => "Needs approval",
            SessionState::Waiting => "Waiting",
            SessionState::Suspended => "Suspended",
            SessionState::Exited => "Exited",
            SessionState::Resumable => "Resumable",
        }
    }

    /// Get the color for this state (for TUI rendering)
    pub fn color(&self) -> ratatui::style::Color {
        crate::tui::theme::theme().session_state_color(self)
    }

    /// Check if session is in an active state
    ///
    /// `AwaitingApproval` is deliberately not active: the process is alive but
    /// it is blocked on the user, which is the same thing `Waiting` means for
    /// the purpose of "how many sessions are actually working right now".
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            SessionState::Starting | SessionState::Thinking | SessionState::Executing
        )
    }

    /// Whether a live child process is expected to be attached
    ///
    /// Polling a session without one reads EOF from a dead PTY, which
    /// `poll_output` reports as an error and turns into `Exited` - so a
    /// deliberate teardown would come back as a crash, and a suspended session
    /// would age into the cleanup path that deletes its stored record.
    pub fn has_process(&self) -> bool {
        !matches!(
            self,
            SessionState::Suspended | SessionState::Exited | SessionState::Resumable
        )
    }

    /// Whether a state reported by an agent should displace this one
    ///
    /// Subagents share one `session_id`, so several things are genuinely true
    /// at once and the events describing them arrive interleaved. Precedence
    /// resolves the tie in favour of whatever the user can act on:
    /// `AwaitingApproval` > `Executing` > `Thinking` > `Waiting`.
    fn precedence(&self) -> u8 {
        match self {
            SessionState::AwaitingApproval => 3,
            SessionState::Executing => 2,
            SessionState::Thinking => 1,
            _ => 0,
        }
    }

    /// Resolve two concurrently-true states into the one worth showing
    pub fn most_urgent(self, other: SessionState) -> SessionState {
        if other.precedence() > self.precedence() {
            other
        } else {
            self
        }
    }

    /// Map a stored value onto the current model
    ///
    /// Session records outlive the code that wrote them, and a record that
    /// fails to parse takes the whole `sessions.json` with it into the
    /// corrupt-file backup path - losing the user's entire recovery index over
    /// a renamed enum variant. So unknown values degrade instead of failing.
    fn from_stored(raw: &serde_json::Value) -> Self {
        match raw {
            serde_json::Value::String(name) => match name.as_str() {
                "Starting" => SessionState::Starting,
                "Thinking" => SessionState::Thinking,
                "Executing" => SessionState::Executing,
                "AwaitingApproval" => SessionState::AwaitingApproval,
                "Waiting" => SessionState::Waiting,
                "Suspended" => SessionState::Suspended,
                "Exited" => SessionState::Exited,
                "Resumable" => SessionState::Resumable,
                // Legacy. `Idle` never meant "idle" - its only writer was the
                // stuck-in-Executing watchdog, so it meant "a tool hung". The
                // process it described is gone by the time anything reads this.
                "Idle" => SessionState::Waiting,
                other => {
                    tracing::warn!(state = %other, "Unknown stored session state, treating as Starting");
                    SessionState::Starting
                }
            },
            // Legacy: `Executing` used to carry its tool name, serialising as
            // `{"Executing": "Bash"}`.
            serde_json::Value::Object(map) if map.contains_key("Executing") => {
                SessionState::Executing
            }
            other => {
                tracing::warn!(state = %other, "Unrecognised stored session state, treating as Starting");
                SessionState::Starting
            }
        }
    }
}

impl<'de> Deserialize<'de> for SessionState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = serde_json::Value::deserialize(deserializer)?;
        Ok(SessionState::from_stored(&raw))
    }
}

/// A tool the agent has started but not yet finished
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InFlightTool {
    /// Tool name as reported by the agent (`Read`, `Bash`, ...)
    pub name: String,
    /// When `PreToolUse` announced it, used to detect stalls
    pub started_at: DateTime<Utc>,
}

/// Why a session is asking for the user
///
/// This replaces a bare `needs_attention: bool`, which could say that a session
/// wanted you but never why - so every reason had to be treated with the same
/// urgency, and Claude's periodic idle nag rang the same bell as a permission
/// prompt blocking real work.
///
/// Attention is deliberately *not* redundant with [`SessionState`]. State
/// describes the process; attention describes the user's queue. A session stays
/// `AwaitingApproval` after you glance at it and clear the flag, because the
/// dialog is still open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttentionReason {
    /// A permission dialog or inline question is blocking the turn
    Approval {
        /// The tool awaiting approval, when the agent named one
        tool: Option<String>,
    },
    /// The agent finished its turn and is waiting for the next prompt
    TurnComplete,
    /// A tool has been in flight far longer than expected
    ///
    /// This is what the old `Idle` state was really flagging.
    Stalled {
        /// The tool that stopped reporting
        tool: String,
        /// How long it had been running when the watchdog gave up on it
        secs: i64,
    },
    /// The process died unexpectedly
    Crashed {
        /// Exit code or signal description
        reason: String,
    },
}

impl AttentionReason {
    /// Short human-readable description for the session list
    pub fn summary(&self) -> String {
        match self {
            AttentionReason::Approval { tool: Some(tool) } => format!("approve {}", tool),
            AttentionReason::Approval { tool: None } => "needs approval".to_string(),
            AttentionReason::TurnComplete => "turn complete".to_string(),
            AttentionReason::Stalled { tool, secs } => {
                format!("{} stalled {}m", tool, secs / 60)
            }
            AttentionReason::Crashed { reason } => format!("crashed: {}", reason),
        }
    }

    /// Badge colour for this reason
    ///
    /// Green means "done, your turn"; yellow means "blocked on you"; red means
    /// something went wrong.
    pub fn badge_color(&self) -> ratatui::style::Color {
        use ratatui::style::Color;
        match self {
            AttentionReason::TurnComplete => Color::Green,
            AttentionReason::Approval { .. } | AttentionReason::Stalled { .. } => Color::Yellow,
            AttentionReason::Crashed { .. } => Color::Red,
        }
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
    /// When the agent last did something, or the user last touched this session
    ///
    /// Deliberately distinct from `last_activity`, which also moves on raw PTY
    /// output. A redrawn status line is rendering, not engagement, and using
    /// `last_activity` as the suspend clock would let a chatty idle prompt keep
    /// a half-gigabyte process alive indefinitely.
    #[serde(default = "Utc::now")]
    pub last_engagement: DateTime<Utc>,
    /// Why this session wants the user, if it does (cleared when viewed)
    #[serde(default)]
    pub attention: Option<AttentionReason>,
    /// Tools the agent has started but not yet finished, keyed by `tool_use_id`
    ///
    /// Not persisted: it describes work owned by a process that does not
    /// survive a restart. Keying by the agent's own invocation ID is what makes
    /// concurrent subagent tools tractable - and what stops an out-of-order
    /// `PostToolUse` from retiring the wrong tool, since hook deliveries are
    /// backgrounded and can arrive reversed.
    #[serde(skip)]
    pub in_flight: HashMap<String, InFlightTool>,
    /// The last thing the assistant said, from the most recent `Stop`
    #[serde(default)]
    pub last_message: Option<String>,
    /// Token and rate-limit figures read from the agent's own transcript
    ///
    /// Not persisted: it describes a live conversation, and stale numbers shown
    /// as current are worse than no numbers. Re-seeded when the tailer attaches.
    #[serde(skip)]
    pub usage: crate::agent::events::UsageSnapshot,
    /// Subagents this session appears to be running
    ///
    /// Only meaningful for Codex, whose subagents write their own separate
    /// rollout files and would otherwise leave the parent looking idle. Claude
    /// subagents share the parent's session and show up in `in_flight` instead.
    #[serde(skip)]
    pub subagents: usize,
    /// Whether this session reattached to a conversation that already existed
    ///
    /// Decides where transcript reading starts. A fresh session's transcript
    /// contains only what this session has just done, so it is read from the
    /// beginning - otherwise the opening seconds, during which Codex is
    /// discovered and first watched, would be lost outright. A reattached
    /// session's file holds a whole prior conversation that must not replay as
    /// if it were happening now.
    #[serde(skip)]
    pub resumed_conversation: bool,
    /// Whether `name` was generated by Panoptes rather than chosen by the user
    ///
    /// Only an auto-generated name may be replaced by the agent's own title;
    /// a name the user typed is theirs.
    #[serde(default)]
    pub auto_named: bool,
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
    /// Codex configuration ID used for this session
    #[serde(default)]
    pub codex_config_id: Option<CodexConfigId>,
    /// Codex configuration name (cached for display)
    #[serde(default)]
    pub codex_config_name: Option<String>,
    /// Whether to automatically close this session after its command finishes
    #[serde(default)]
    pub auto_close_after_command: bool,
    /// The agent's own conversation ID, used to resume this session after a restart.
    ///
    /// For Claude Code this equals `id` - Panoptes dictates the conversation UUID
    /// via `--session-id` rather than discovering it. For Codex it is resolved
    /// from the rollout file, since Codex has no equivalent flag. Always `None`
    /// for shell sessions, which have no conversation to resume.
    #[serde(default)]
    pub agent_session_id: Option<String>,
}

impl SessionInfo {
    /// Why this session cannot be brought back, if it cannot
    ///
    /// Returns `None` when the session is resumable. A recovered session can
    /// become unusable between runs - its worktree may have been deleted, or an
    /// agent session may have died before ever recording a conversation ID -
    /// and the reason is worth showing rather than silently hiding the entry.
    pub fn resume_blocker(&self) -> Option<&'static str> {
        if !self.working_dir.exists() {
            return Some("working directory is missing");
        }
        // Shells are restored by respawning in the same directory, so they have
        // no conversation to reattach to and need no ID
        if self.session_type != SessionType::Shell && self.agent_session_id.is_none() {
            return Some("no conversation was recorded");
        }
        None
    }

    /// Whether this session can be brought back
    pub fn is_resumable(&self) -> bool {
        self.resume_blocker().is_none()
    }

    /// The conversation cursor to hand a relaunched agent, if any
    ///
    /// A shell has no conversation to reattach to - it is restored by
    /// respawning in the same directory - so it never carries a resume cursor.
    /// Agents reattach via their recorded conversation ID.
    pub fn resume_cursor(&self) -> Option<String> {
        match self.session_type {
            SessionType::Shell => None,
            _ => self.agent_session_id.clone(),
        }
    }

    /// Check if this session needs attention
    ///
    /// Attention is decoupled from state because subagents share a session_id -
    /// one subagent can be waiting for input while another is actively working,
    /// so the session may be in Thinking/Executing state while still needing
    /// attention. It is set only by hook events and by the crash and stall
    /// watchdogs, never by PTY output, since only those explicitly signal that
    /// user interaction is required.
    ///
    /// Deliberately *only* the flag. There used to be a second, time-based rule
    /// here: a session in `Waiting` whose `last_activity` had gone stale was
    /// flagged too. That rule dated from when the state was called `Idle` and
    /// meant "this session has gone abnormally quiet" - a job now done properly
    /// by `AttentionReason::Stalled`. Renaming the state to `Waiting` silently
    /// changed what the rule said, because `Waiting` is the normal resting
    /// state of every healthy session: it made "you finished a turn five
    /// minutes ago" indistinguishable from "something wants you". Worse, it
    /// could not be dismissed - `acknowledge_attention` clears the flag, not
    /// the clock - so opening the session did nothing and the badge came back
    /// the moment you looked away. It bit Codex hardest, whose sessions write
    /// nothing to the PTY between turns and so never refresh `last_activity`.
    ///
    /// Lives on `SessionInfo` rather than `Session` so that list views can
    /// call it for recovered sessions too, which have no process attached.
    pub fn needs_attention(&self) -> bool {
        self.attention.is_some()
    }

    /// Update session state, stamping the transition clocks
    ///
    /// The pure counterpart of [`Session::set_state`], which delegates here.
    /// Taking `now` as a parameter keeps the state machine deterministic and
    /// testable without a live session.
    pub fn set_state_at(&mut self, state: SessionState, now: DateTime<Utc>) {
        // Track when session exited for cleanup
        if state == SessionState::Exited && self.exited_at.is_none() {
            self.exited_at = Some(now);
        }
        // A dead process owns no running tools
        if state == SessionState::Exited {
            self.in_flight.clear();
        }
        self.state = state;
        self.last_activity = now;
        self.state_entered_at = now;
        self.last_engagement = now;
    }

    /// The tools currently in flight, rendered for the session list
    ///
    /// Returns `None` when nothing is running. Repeated tools collapse into
    /// `Task ×3`, and the order is by start time rather than by `HashMap`
    /// iteration, which is arbitrary and would reshuffle on every render.
    pub fn in_flight_summary(&self) -> Option<String> {
        if self.in_flight.is_empty() {
            return None;
        }

        let mut grouped: Vec<(&str, DateTime<Utc>, usize)> = Vec::new();
        for tool in self.in_flight.values() {
            match grouped.iter_mut().find(|(name, _, _)| *name == tool.name) {
                Some(entry) => {
                    entry.2 += 1;
                    entry.1 = entry.1.min(tool.started_at);
                }
                None => grouped.push((tool.name.as_str(), tool.started_at, 1)),
            }
        }
        grouped.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));

        let rendered: Vec<String> = grouped
            .into_iter()
            .map(|(name, _, count)| {
                if count > 1 {
                    format!("{} ×{}", name, count)
                } else {
                    name.to_string()
                }
            })
            .collect();

        Some(rendered.join(", "))
    }

    /// Record a tool the agent has just started
    pub fn start_tool(&mut self, key: String, name: String, started_at: DateTime<Utc>) {
        self.in_flight
            .insert(key, InFlightTool { name, started_at });
    }

    /// Retire a tool the agent has finished, returning its name if it was tracked
    pub fn finish_tool(&mut self, key: &str) -> Option<String> {
        self.in_flight.remove(key).map(|tool| tool.name)
    }

    /// Adopt the agent's own title for this conversation
    ///
    /// Only replaces a name Panoptes generated. A name the user typed is a
    /// deliberate label and is never overwritten. Returns whether it changed.
    pub fn adopt_agent_title(&mut self, title: &str) -> bool {
        let title = title.trim();
        if title.is_empty() || !self.auto_named || self.name == title {
            return false;
        }
        self.name = title.to_string();
        true
    }

    /// Record the assistant's closing message, trimmed to something displayable
    pub fn set_last_message(&mut self, message: &str) {
        /// Enough for a session-list hint; the full text lives in the transcript
        const MAX_LEN: usize = 160;

        let collapsed = message.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            return;
        }

        // Truncate on a character boundary, not a byte one
        let trimmed = match collapsed.char_indices().nth(MAX_LEN) {
            Some((idx, _)) => format!("{}…", &collapsed[..idx]),
            None => collapsed,
        };
        self.last_message = Some(trimmed);
    }

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
            last_engagement: now,
            attention: None,
            in_flight: HashMap::new(),
            last_message: None,
            usage: crate::agent::events::UsageSnapshot::default(),
            subagents: 0,
            resumed_conversation: false,
            auto_named: false,
            exit_reason: None,
            exited_at: None,
            claude_config_id: None,
            claude_config_name: None,
            codex_config_id: None,
            codex_config_name: None,
            auto_close_after_command: false,
            agent_session_id: None,
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

    /// Create new session info for a Codex session
    pub fn codex(
        name: String,
        working_dir: std::path::PathBuf,
        project_id: ProjectId,
        branch_id: BranchId,
    ) -> Self {
        let mut info = Self::new(name, working_dir, project_id, branch_id);
        info.session_type = SessionType::OpenAICodex;
        info
    }

    /// Create new session info for a Codex session with configuration
    pub fn with_codex_config(
        name: String,
        working_dir: std::path::PathBuf,
        project_id: ProjectId,
        branch_id: BranchId,
        codex_config_id: Option<CodexConfigId>,
        codex_config_name: Option<String>,
    ) -> Self {
        let mut info = Self::codex(name, working_dir, project_id, branch_id);
        info.codex_config_id = codex_config_id;
        info.codex_config_name = codex_config_name;
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

    /// Check if this session should be auto-closed.
    ///
    /// Returns true when the session has `auto_close_after_command` enabled,
    /// is a shell session in `Waiting` state, and the grace period (seconds
    /// since `Waiting` was entered) has elapsed.
    pub fn should_auto_close(&self, grace_secs: i64) -> bool {
        self.auto_close_after_command
            && self.session_type == SessionType::Shell
            && self.state == SessionState::Waiting
            && Utc::now()
                .signed_duration_since(self.state_entered_at)
                .num_seconds()
                >= grace_secs
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

    /// Scroll up by n lines, clamped for the current viewport height.
    ///
    /// Stops at the highest full-page position (`total_lines -
    /// viewport_height`) so the visible page height stays stable at the top of
    /// history.
    pub fn scroll_up_with_viewport(&mut self, n: usize, viewport_height: usize) {
        let effective_height = viewport_height.max(1);
        let max_scroll = self.lines.len().saturating_sub(effective_height);
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

    /// Scroll to top for a given viewport height.
    ///
    /// This pins to the oldest full page rather than a single first line.
    pub fn scroll_to_top_with_viewport(&mut self, viewport_height: usize) {
        let effective_height = viewport_height.max(1);
        self.scroll_offset = self.lines.len().saturating_sub(effective_height);
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

/// Codex-specific fallback scrollback state.
#[derive(Debug)]
struct CodexFallback {
    /// Plain-text fallback history extracted from PTY output.
    output_buffer: OutputBuffer,
    /// Partial plain-text line carried between PTY reads.
    partial_output_line: String,
    /// Remainder bytes for incomplete ANSI escape sequences between reads.
    ansi_remainder: Vec<u8>,
    /// Trailing bytes of a multi-byte UTF-8 character split across PTY reads.
    ///
    /// Without this, a character whose bytes straddle two reads would be
    /// lossy-decoded per chunk and end up as U+FFFD in fallback history.
    utf8_remainder: Vec<u8>,
}

impl CodexFallback {
    fn new(scrollback_rows: usize) -> Self {
        Self {
            output_buffer: OutputBuffer::new(scrollback_rows),
            partial_output_line: String::new(),
            ansi_remainder: Vec::new(),
            utf8_remainder: Vec::new(),
        }
    }

    /// Feed raw PTY bytes into the fallback history: strip ANSI sequences,
    /// reassemble UTF-8 characters split across reads, and append the plain
    /// text to the output buffer.
    fn ingest(&mut self, bytes: &[u8]) {
        let plain = Session::extract_plain_text_bytes(&mut self.ansi_remainder, bytes);
        let mut data = std::mem::take(&mut self.utf8_remainder);
        data.extend_from_slice(&plain);

        if let Err(e) = std::str::from_utf8(&data) {
            // `error_len() == None` means the data merely ends mid-character
            // (a multi-byte char split across PTY reads): hold the tail back
            // until the rest arrives. A genuinely invalid byte (`Some`) is
            // left in place and becomes U+FFFD, as before. If the held-back
            // tail never completes, the next chunk fails validation at that
            // spot with `Some` and it degrades to U+FFFD then.
            if e.error_len().is_none() {
                self.utf8_remainder = data.split_off(e.valid_up_to());
            }
        }

        self.output_buffer
            .push_bytes(&data, &mut self.partial_output_line);
    }
}

/// Responds to DSR (Device Status Report) queries from the child process.
///
/// Some programs (e.g. Codex CLI) send `\x1b[6n` at startup to query cursor
/// position and crash if no CPR response arrives in time. A query can be
/// split across PTY reads, so a rolling tail of the previous read is kept to
/// match sequences that straddle a chunk boundary.
#[derive(Debug, Default)]
struct DsrResponder {
    /// Rolling tail (last 3 bytes) of previously seen output.
    tail: Vec<u8>,
}

impl DsrResponder {
    fn new() -> Self {
        Self::default()
    }

    /// Scan `bytes` (prefixed with the tail of the previous read) for DSR
    /// queries. Returns the CPR response bytes to write back to the child,
    /// one response per query, or `None` if no query was seen.
    ///
    /// `cursor` is the current (row, col) cursor position, 0-based; the CPR
    /// reply is 1-based.
    fn respond(&mut self, bytes: &[u8], cursor: (u16, u16)) -> Option<Vec<u8>> {
        let mut combined = Vec::with_capacity(self.tail.len() + bytes.len());
        combined.extend_from_slice(&self.tail);
        combined.extend_from_slice(bytes);

        let dsr_count = combined.windows(4).filter(|w| *w == b"\x1b[6n").count();

        // Keep the last 3 bytes: long enough to complete a split "\x1b[6n",
        // short enough that an already-matched query (4 bytes) can never be
        // counted twice.
        let keep_from = combined.len().saturating_sub(3);
        self.tail = combined[keep_from..].to_vec();

        if dsr_count == 0 {
            return None;
        }
        let (row, col) = cursor;
        let response = format!("\x1b[{};{}R", row + 1, col + 1);
        Some(response.repeat(dsr_count).into_bytes())
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
    /// Codex-only fallback state for cases where terminal-emulator scrollback
    /// is unavailable.
    codex_fallback: Option<CodexFallback>,
    /// Responder for DSR cursor-position queries from the child process.
    dsr: DsrResponder,
}

impl Session {
    /// Create a new session with the given info, PTY, and terminal dimensions
    pub fn new(info: SessionInfo, pty: PtyHandle, rows: usize, cols: usize) -> Self {
        let codex_fallback = (info.session_type == SessionType::OpenAICodex)
            .then(|| CodexFallback::new(DEFAULT_SCROLLBACK_ROWS));
        Self {
            info,
            pty,
            vterm: VirtualTerminal::new(rows, cols),
            codex_fallback,
            dsr: DsrResponder::new(),
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
        let codex_fallback = (info.session_type == SessionType::OpenAICodex)
            .then(|| CodexFallback::new(scrollback_rows));
        Self {
            info,
            pty,
            vterm: VirtualTerminal::with_scrollback(rows, cols, scrollback_rows),
            codex_fallback,
            dsr: DsrResponder::new(),
        }
    }

    /// Poll PTY for new output and process through virtual terminal
    /// Returns true if any output was read
    pub fn poll_output(&mut self) -> bool {
        match self.pty.try_read() {
            Ok(Some(bytes)) => {
                self.vterm.process(&bytes);

                if let Some(fallback) = self.codex_fallback.as_mut() {
                    fallback.ingest(&bytes);
                }

                // Respond to DSR (Device Status Report) queries from the child process.
                // Some programs (e.g. Codex CLI) send \x1b[6n at startup to query cursor
                // position and crash if no CPR response arrives in time.
                let cursor = self.vterm.cursor_position();
                if let Some(response) = self.dsr.respond(&bytes, cursor) {
                    if let Err(e) = self.pty.write(&response) {
                        tracing::debug!(error = %e, "Failed to write DSR response");
                    }
                }

                self.note_output_activity();

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

    /// Get visible styled lines for rendering (with colors)
    /// Returns Rc to avoid cloning the entire vector on each render
    pub fn visible_styled_lines(
        &self,
        viewport_height: usize,
    ) -> Rc<Vec<ratatui::text::Line<'static>>> {
        self.vterm.visible_styled_lines(viewport_height)
    }

    /// Get visible lines from the plain-text fallback history buffer.
    pub fn fallback_visible_lines(&self, viewport_height: usize) -> Vec<String> {
        self.codex_fallback
            .as_ref()
            .map(|fallback| {
                fallback
                    .output_buffer
                    .visible_lines(viewport_height)
                    .into_iter()
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Scroll the plain-text fallback history up, clamped for viewport height.
    pub fn fallback_scroll_up_with_viewport(&mut self, n: usize, viewport_height: usize) {
        if let Some(fallback) = self.codex_fallback.as_mut() {
            fallback
                .output_buffer
                .scroll_up_with_viewport(n, viewport_height);
        }
    }

    /// Scroll the plain-text fallback history down.
    pub fn fallback_scroll_down(&mut self, n: usize) {
        if let Some(fallback) = self.codex_fallback.as_mut() {
            fallback.output_buffer.scroll_down(n);
        }
    }

    /// Scroll plain-text fallback history to bottom.
    pub fn fallback_scroll_to_bottom(&mut self) {
        if let Some(fallback) = self.codex_fallback.as_mut() {
            fallback.output_buffer.scroll_to_bottom();
        }
    }

    /// Scroll plain-text fallback history to top for a viewport.
    pub fn fallback_scroll_to_top_with_viewport(&mut self, viewport_height: usize) {
        if let Some(fallback) = self.codex_fallback.as_mut() {
            fallback
                .output_buffer
                .scroll_to_top_with_viewport(viewport_height);
        }
    }

    /// Current fallback history scroll offset.
    pub fn fallback_scroll_offset(&self) -> usize {
        self.codex_fallback
            .as_ref()
            .map_or(0, |fallback| fallback.output_buffer.scroll_offset())
    }

    /// Degrade a full-buffer write timeout to a dropped write.
    ///
    /// `write` and `send_key` errors propagate to the event loop, where they
    /// would exit the app. A child that has wedged and stopped draining its
    /// PTY must not take the UI down with it: the bytes are dropped with a
    /// warning instead - a keystroke lost to a session that is not consuming
    /// input anyway. Any other error still propagates.
    fn tolerate_full_buffer(&self, result: anyhow::Result<()>) -> anyhow::Result<()> {
        match result {
            Err(e) if e.downcast_ref::<PtyWriteTimedOut>().is_some() => {
                tracing::warn!(
                    session_id = %self.info.id,
                    session_name = %self.info.name,
                    error = %e,
                    "PTY buffer stayed full; dropping write"
                );
                Ok(())
            }
            other => other,
        }
    }

    /// Write bytes to the PTY
    pub fn write(&mut self, data: &[u8]) -> anyhow::Result<()> {
        self.note_user_input();
        let result = self.pty.write(data);
        self.tolerate_full_buffer(result)
    }

    /// Write pasted text to the PTY
    /// Wraps in bracketed paste sequences only if the app has enabled it
    pub fn write_paste(&mut self, text: &str) -> anyhow::Result<()> {
        // Without bracketed paste the agent sees the text as typing, so an
        // embedded newline submits it. With it, the newline is inserted
        // literally and a separate Enter follows, which `send_key` handles.
        let submits = !self.vterm.bracketed_paste_enabled() && text.contains('\n');

        self.note_user_input();
        let result = if self.vterm.bracketed_paste_enabled() {
            self.pty.write_paste(text)
        } else {
            self.pty.write(text.as_bytes())
        };

        // Only guess a turn once the write actually succeeded; otherwise the
        // session would show Thinking with nothing submitted. Paste errors are
        // surfaced to the user by the caller, so they are not degraded here.
        if result.is_ok() && submits {
            self.guess_turn_started();
        }
        result
    }

    /// Send a key event to the PTY
    pub fn send_key(&mut self, key: crossterm::event::KeyEvent) -> anyhow::Result<()> {
        use crossterm::event::KeyCode;

        self.note_user_input();
        let result = self.pty.send_key(key);

        // Only guess a turn once the write actually succeeded; a dropped
        // Enter never reached the agent.
        if result.is_ok() && key.code == KeyCode::Enter {
            self.guess_turn_started();
        }
        self.tolerate_full_buffer(result)
    }

    /// Infer that the user just submitted a prompt
    ///
    /// Only for agents that do not announce it. This is a guess, and a poor
    /// one - it fires on an Enter that merely dismissed a menu - so it is used
    /// only where there is nothing better. Claude Code reports
    /// `UserPromptSubmit` and is excluded; Codex has no equivalent hook.
    fn guess_turn_started(&mut self) {
        let must_guess = self.info.session_type.uses_hooks()
            && !self.info.session_type.reports_prompt_submission();
        if must_guess && self.info.state == SessionState::Waiting {
            self.set_state(SessionState::Thinking);
        }
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
        self.info.set_state_at(state, Utc::now());
    }

    /// Bookkeeping for a successful PTY read: bump the activity clock and
    /// promote a `Starting` session that produced output.
    fn note_output_activity(&mut self) {
        self.info.last_activity = Utc::now();

        // If we're in Starting state and receiving output, Claude is running
        // Transition to Waiting (ready for user input)
        if self.info.state == SessionState::Starting {
            self.set_state(SessionState::Waiting);
        }
    }

    /// Note that the user has interacted with this session
    ///
    /// Keeps a session the user is typing into out of the suspend sweep -
    /// killing a process with half a prompt typed into it would lose real work,
    /// and the agent produces no events at all while that happens.
    pub fn note_user_input(&mut self) {
        let now = Utc::now();
        self.info.last_activity = now;
        self.info.last_engagement = now;
    }

    /// Strip ANSI escape/control sequences from PTY bytes while preserving
    /// printable UTF-8-compatible bytes and newlines for fallback history.
    ///
    /// A sequence split across PTY reads is stashed in `ansi_remainder` (from
    /// its ESC onward, together with any bytes after it in this chunk) and
    /// re-scanned when the next chunk arrives.
    fn extract_plain_text_bytes(ansi_remainder: &mut Vec<u8>, bytes: &[u8]) -> Vec<u8> {
        let mut data = Vec::with_capacity(ansi_remainder.len() + bytes.len());
        data.extend_from_slice(ansi_remainder);
        data.extend_from_slice(bytes);
        ansi_remainder.clear();

        let mut out = Vec::with_capacity(data.len());
        let mut i = 0;
        while i < data.len() {
            if data[i] == 0x1b {
                let esc_start = i;
                let result = if i + 1 >= data.len() {
                    // Bare ESC at the end of the chunk: introducer not seen yet
                    SkipResult::Incomplete
                } else {
                    match data[i + 1] {
                        b'[' => skip_csi(&data, i + 2),
                        b']' => skip_string_seq(&data, i + 2, true),
                        b'P' | b'X' | b'^' | b'_' => skip_string_seq(&data, i + 2, false),
                        // Two-byte escape (e.g. ESC 7, ESC =): skip both bytes
                        _ => SkipResult::Complete(i + 2),
                    }
                };
                match result {
                    SkipResult::Complete(next) => {
                        i = next;
                        continue;
                    }
                    SkipResult::Incomplete => {
                        ansi_remainder.extend_from_slice(&data[esc_start..]);
                        break;
                    }
                }
            }

            let b = data[i];
            if b == b'\n' || b == b'\r' || b == b'\t' || b >= 0x20 {
                out.push(b);
            }
            i += 1;
        }

        out
    }
}

/// Outcome of skipping one escape sequence in `extract_plain_text_bytes`.
enum SkipResult {
    /// Sequence fully consumed; scanning resumes at this index.
    Complete(usize),
    /// Sequence runs past the end of the chunk; the caller stashes everything
    /// from its ESC onward and waits for the next read.
    Incomplete,
}

/// Skip a CSI sequence body, starting at the byte after `ESC [`.
///
/// A CSI sequence ends at its first final byte (0x40..=0x7E).
fn skip_csi(data: &[u8], mut i: usize) -> SkipResult {
    while i < data.len() {
        let b = data[i];
        i += 1;
        if (0x40..=0x7e).contains(&b) {
            return SkipResult::Complete(i);
        }
    }
    SkipResult::Incomplete
}

/// Skip a string sequence body (OSC, DCS, SOS, PM, APC), starting at the byte
/// after the two-byte introducer.
///
/// All of these are terminated by ST (`ESC \`); OSC (`bel_terminates`) also
/// accepts BEL. An ESC not (yet) followed by `\` - split across reads, or a
/// malformed sequence - reports the whole sequence as incomplete.
fn skip_string_seq(data: &[u8], mut i: usize, bel_terminates: bool) -> SkipResult {
    while i < data.len() {
        if bel_terminates && data[i] == 0x07 {
            return SkipResult::Complete(i + 1);
        }
        if data[i] == 0x1b {
            if i + 1 < data.len() && data[i + 1] == b'\\' {
                return SkipResult::Complete(i + 2);
            }
            return SkipResult::Incomplete;
        }
        i += 1;
    }
    SkipResult::Incomplete
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
        assert_eq!(SessionState::Executing.display_name(), "Executing");
        assert_eq!(
            SessionState::AwaitingApproval.display_name(),
            "Needs approval"
        );
        assert_eq!(SessionState::Waiting.display_name(), "Waiting");
        assert_eq!(SessionState::Suspended.display_name(), "Suspended");
        assert_eq!(SessionState::Exited.display_name(), "Exited");
        assert_eq!(SessionState::Resumable.display_name(), "Resumable");
    }

    #[test]
    fn test_session_state_is_active() {
        assert!(SessionState::Starting.is_active());
        assert!(SessionState::Thinking.is_active());
        assert!(SessionState::Executing.is_active());
        assert!(!SessionState::Waiting.is_active());
        // Blocked on the user is not the same as working
        assert!(!SessionState::AwaitingApproval.is_active());
        assert!(!SessionState::Suspended.is_active());
        assert!(!SessionState::Exited.is_active());
    }

    #[test]
    fn test_state_precedence_favours_what_the_user_can_act_on() {
        use SessionState::*;

        // Several subagents are true at once; the actionable one wins
        assert_eq!(Executing.most_urgent(AwaitingApproval), AwaitingApproval);
        assert_eq!(AwaitingApproval.most_urgent(Executing), AwaitingApproval);
        assert_eq!(Thinking.most_urgent(Executing), Executing);
        assert_eq!(Executing.most_urgent(Thinking), Executing);
        // Waiting never wins by precedence; only an authoritative event may
        // demote a session to it
        assert_eq!(Executing.most_urgent(Waiting), Executing);
    }

    #[test]
    fn test_legacy_session_states_still_parse() {
        // A record that fails to parse takes the whole sessions.json with it
        // into the corrupt-file backup path, losing the recovery index.
        let cases = [
            (r#""Idle""#, SessionState::Waiting),
            (r#"{"Executing":"Bash"}"#, SessionState::Executing),
            (r#""Waiting""#, SessionState::Waiting),
            (r#""Resumable""#, SessionState::Resumable),
            // Something a future version might write
            (r#""SomethingNew""#, SessionState::Starting),
        ];

        for (json, expected) in cases {
            let parsed: SessionState =
                serde_json::from_str(json).unwrap_or_else(|e| panic!("{json} failed: {e}"));
            assert_eq!(parsed, expected, "for {json}");
        }
    }

    #[test]
    fn test_legacy_session_info_loads_intact() {
        // A record written by the pre-PAN-1 build, including the fields this
        // ticket removed
        let json = r#"{
            "id": "8f0b8e46-1f3a-4c9e-9a1e-2b1c3d4e5f60",
            "name": "old session",
            "state": {"Executing": "Bash"},
            "session_type": "ClaudeCode",
            "working_dir": "/tmp",
            "project_id": "8f0b8e46-1f3a-4c9e-9a1e-2b1c3d4e5f61",
            "branch_id": "8f0b8e46-1f3a-4c9e-9a1e-2b1c3d4e5f62",
            "created_at": "2026-01-01T00:00:00Z",
            "last_activity": "2026-01-01T00:00:00Z",
            "state_entered_at": "2026-01-01T00:00:00Z",
            "needs_attention": true,
            "agent_session_id": "8f0b8e46-1f3a-4c9e-9a1e-2b1c3d4e5f60"
        }"#;

        let info: SessionInfo = serde_json::from_str(json).expect("legacy record must still load");
        assert_eq!(info.name, "old session");
        assert_eq!(info.state, SessionState::Executing);
        // The removed flag is simply ignored; attention is re-derived
        assert!(info.attention.is_none());
        assert!(info.in_flight.is_empty());
        assert!(!info.auto_named);
    }

    #[test]
    fn test_in_flight_summary_is_stable_and_grouped() {
        let mut info = SessionInfo::new(
            "s".to_string(),
            "/tmp".into(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        assert_eq!(info.in_flight_summary(), None);

        let t0 = Utc::now();
        info.start_tool("a".into(), "Read".into(), t0);
        info.start_tool("b".into(), "Grep".into(), t0 + chrono::Duration::seconds(1));
        info.start_tool("c".into(), "Grep".into(), t0 + chrono::Duration::seconds(2));

        // Ordered by start time, duplicates collapsed - and identical across
        // repeated calls, which HashMap iteration order alone would not give
        let first = info.in_flight_summary();
        assert_eq!(first.as_deref(), Some("Read, Grep ×2"));
        for _ in 0..10 {
            assert_eq!(info.in_flight_summary(), first);
        }

        assert_eq!(info.finish_tool("a"), Some("Read".to_string()));
        assert_eq!(info.in_flight_summary().as_deref(), Some("Grep ×2"));
        assert_eq!(info.finish_tool("nonexistent"), None);
    }

    /// A live session wrapping a harmless long-running child
    fn test_session(session_type: SessionType) -> Session {
        let pty = crate::session::pty::PtyHandle::spawn(
            "sleep",
            &["60"],
            std::path::Path::new("/tmp"),
            std::collections::HashMap::new(),
            24,
            80,
        )
        .unwrap();
        let mut info = SessionInfo::new(
            "s".to_string(),
            "/tmp".into(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        info.session_type = session_type;
        info.state = SessionState::Waiting;
        Session::new(info, pty, 24, 80)
    }

    #[test]
    fn test_paste_that_submits_starts_a_turn_for_agents_that_cannot_report_it() {
        // Codex has no prompt-submission hook, so a submitted prompt has to be
        // inferred. Without bracketed paste the embedded newline submits, and
        // no Enter key event ever arrives to notice it - leaving the session
        // looking idle for the entire turn, and eligible for suspension.
        let mut session = test_session(SessionType::OpenAICodex);
        assert!(!session.vterm.bracketed_paste_enabled());

        session.write_paste("do the thing\n").unwrap();
        assert_eq!(session.info.state, SessionState::Thinking);
    }

    #[test]
    fn test_bracketed_paste_does_not_start_a_turn() {
        // With bracketed paste the newline is inserted literally; the user
        // still has to press Enter, which send_key sees.
        let mut session = test_session(SessionType::OpenAICodex);
        session.vterm.process(b"\x1b[?2004h");
        assert!(session.vterm.bracketed_paste_enabled());

        session.write_paste("do the thing\n").unwrap();
        assert_eq!(session.info.state, SessionState::Waiting);
    }

    #[test]
    fn test_paste_never_guesses_for_agents_that_report_prompts() {
        // Claude Code sends UserPromptSubmit; guessing on top of it would only
        // add wrong answers.
        let mut session = test_session(SessionType::ClaudeCode);
        session.write_paste("do the thing\n").unwrap();
        assert_eq!(session.info.state, SessionState::Waiting);
    }

    #[test]
    fn test_failed_paste_does_not_start_a_turn() {
        // If the PTY write fails, the session must not be left showing
        // Thinking when nothing was submitted; the error surfaces so the app
        // can tell the user. Writing into a dead child's PTY is the cheapest
        // deterministic write failure available.
        let mut session = test_session(SessionType::OpenAICodex);
        assert!(!session.vterm.bracketed_paste_enabled());

        session.kill().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while session.is_alive() {
            assert!(std::time::Instant::now() < deadline, "child never died");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let result = session.write_paste("do the thing\n");

        assert!(result.is_err(), "a write into a dead PTY must fail");
        assert_eq!(session.info.state, SessionState::Waiting);
    }

    #[test]
    fn test_full_buffer_write_timeout_is_dropped_not_fatal() {
        // `write` and `send_key` errors propagate to the event loop and would
        // exit the app; a child that stopped draining its PTY must degrade to
        // a dropped write instead. Any other error still propagates, and the
        // downcast must see through anyhow's context wrapping.
        let session = test_session(SessionType::ClaudeCode);

        let timeout = anyhow::Error::new(PtyWriteTimedOut {
            timeout: std::time::Duration::from_millis(50),
            written: 0,
            total: 10,
        })
        .context("Failed to write paste content");
        assert!(session.tolerate_full_buffer(Err(timeout)).is_ok());

        let other = anyhow::anyhow!("some real I/O failure");
        assert!(session.tolerate_full_buffer(Err(other)).is_err());

        assert!(session.tolerate_full_buffer(Ok(())).is_ok());
    }

    #[test]
    fn test_user_input_advances_the_engagement_clock() {
        let mut session = test_session(SessionType::ClaudeCode);
        let long_ago = Utc::now() - chrono::Duration::seconds(10_000);
        session.info.last_engagement = long_ago;
        session.info.last_activity = long_ago;

        session.write(b"x").unwrap();

        assert!(session.info.last_engagement > long_ago);
        assert!(session.info.last_activity > long_ago);
    }

    #[test]
    fn test_adopt_agent_title() {
        let mut info = SessionInfo::new(
            "My name".to_string(),
            "/tmp".into(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );

        // A name the user chose is never overwritten
        assert!(!info.adopt_agent_title("Agent title"));
        assert_eq!(info.name, "My name");

        info.auto_named = true;
        assert!(info.adopt_agent_title("Agent title"));
        assert_eq!(info.name, "Agent title");

        // No-ops
        assert!(!info.adopt_agent_title("Agent title"));
        assert!(!info.adopt_agent_title("   "));
        assert_eq!(info.name, "Agent title");
    }

    #[test]
    fn test_set_last_message_collapses_and_truncates_safely() {
        let mut info = SessionInfo::new(
            "s".to_string(),
            "/tmp".into(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );

        info.set_last_message("  Done\n  with   the thing.  ");
        assert_eq!(info.last_message.as_deref(), Some("Done with the thing."));

        // Whitespace-only input leaves the previous value alone
        info.set_last_message("   \n  ");
        assert_eq!(info.last_message.as_deref(), Some("Done with the thing."));

        // Multi-byte characters must not be split mid-character
        info.set_last_message(&"é".repeat(500));
        let stored = info.last_message.clone().unwrap();
        assert!(stored.ends_with('…'));
        assert_eq!(stored.chars().count(), 161);
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
        for state in [
            SessionState::Starting,
            SessionState::Thinking,
            SessionState::Executing,
            SessionState::AwaitingApproval,
            SessionState::Waiting,
            SessionState::Suspended,
            SessionState::Exited,
            SessionState::Resumable,
        ] {
            let json = serde_json::to_string(&state).unwrap();
            let parsed: SessionState = serde_json::from_str(&json).unwrap();
            assert_eq!(state, parsed, "round trip failed for {json}");
        }
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
    fn test_session_type_short_tag() {
        assert_eq!(SessionType::ClaudeCode.short_tag(), "[CC]");
        assert_eq!(SessionType::Shell.short_tag(), "[SH]");
        assert_eq!(SessionType::OpenAICodex.short_tag(), "[CX]");
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
    fn test_should_auto_close_all_conditions_met() {
        let project_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();
        let mut info = SessionInfo::shell("sh".to_string(), "/tmp".into(), project_id, branch_id);
        info.auto_close_after_command = true;
        // Simulate state_entered_at 5 seconds ago
        info.state_entered_at = Utc::now() - chrono::Duration::seconds(5);
        assert!(info.should_auto_close(3));
    }

    #[test]
    fn test_should_auto_close_within_grace_period() {
        let project_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();
        let mut info = SessionInfo::shell("sh".to_string(), "/tmp".into(), project_id, branch_id);
        info.auto_close_after_command = true;
        // state_entered_at is now (just created), so <3s elapsed
        assert!(!info.should_auto_close(3));
    }

    #[test]
    fn test_should_auto_close_disabled() {
        let project_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();
        let mut info = SessionInfo::shell("sh".to_string(), "/tmp".into(), project_id, branch_id);
        info.auto_close_after_command = false;
        info.state_entered_at = Utc::now() - chrono::Duration::seconds(10);
        assert!(!info.should_auto_close(3));
    }

    #[test]
    fn test_should_auto_close_not_waiting_state() {
        let project_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();
        let mut info = SessionInfo::shell("sh".to_string(), "/tmp".into(), project_id, branch_id);
        info.auto_close_after_command = true;
        info.state = SessionState::Executing;
        info.state_entered_at = Utc::now() - chrono::Duration::seconds(10);
        assert!(!info.should_auto_close(3));
    }

    #[test]
    fn test_should_auto_close_not_shell_type() {
        let project_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();
        let mut info = SessionInfo::new("cc".to_string(), "/tmp".into(), project_id, branch_id);
        info.auto_close_after_command = true;
        info.state = SessionState::Waiting;
        info.state_entered_at = Utc::now() - chrono::Duration::seconds(10);
        // ClaudeCode session type should not auto-close
        assert!(!info.should_auto_close(3));
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

        // Scroll up (1-line viewport: offset can reach the very first line)
        buf.scroll_up_with_viewport(5, 1);
        assert!(!buf.is_at_bottom());
        assert_eq!(buf.scroll_offset(), 5);

        // Scroll down
        buf.scroll_down(3);
        assert_eq!(buf.scroll_offset(), 2);

        // Scroll to bottom
        buf.scroll_to_bottom();
        assert!(buf.is_at_bottom());

        // Scroll to top
        buf.scroll_to_top_with_viewport(1);
        assert_eq!(buf.scroll_offset(), 19);

        // Can't scroll past max
        buf.scroll_up_with_viewport(100, 1);
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
        buf.scroll_up_with_viewport(3, 5);
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
    fn test_output_buffer_scroll_up_with_viewport_clamps_at_full_page_top() {
        let mut buf = OutputBuffer::new(100);
        for i in 0..10 {
            buf.push(format!("line {}", i));
        }

        // Viewport is 5 lines, so max useful offset is 10 - 5 = 5.
        buf.scroll_up_with_viewport(100, 5);
        assert_eq!(buf.scroll_offset(), 5);

        // Keep a full page visible at the top.
        let visible = buf.visible_lines(5);
        assert_eq!(visible.len(), 5);
        assert_eq!(visible[0], "line 0");
        assert_eq!(visible[4], "line 4");
    }

    #[test]
    fn test_output_buffer_scroll_to_top_with_viewport() {
        let mut buf = OutputBuffer::new(100);
        for i in 0..3 {
            buf.push(format!("line {}", i));
        }

        // When viewport is taller than content, top offset is 0.
        buf.scroll_to_top_with_viewport(10);
        assert_eq!(buf.scroll_offset(), 0);

        // With a 2-line viewport over 3 lines, top offset is 1.
        buf.scroll_to_top_with_viewport(2);
        assert_eq!(buf.scroll_offset(), 1);
    }

    #[test]
    fn test_output_buffer_clear() {
        let mut buf = OutputBuffer::new(100);
        buf.push("line 1".to_string());
        buf.push("line 2".to_string());
        buf.scroll_up_with_viewport(1, 1);

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

    #[test]
    fn test_extract_plain_text_bytes_strips_csi_sequences() {
        let mut ansi_remainder = Vec::new();
        let plain =
            Session::extract_plain_text_bytes(&mut ansi_remainder, b"hello\x1b[31m red\x1b[0m\r\n");
        assert_eq!(String::from_utf8_lossy(&plain), "hello red\r\n");
    }

    #[test]
    fn test_extract_plain_text_bytes_handles_split_escape_sequence() {
        let mut ansi_remainder = Vec::new();
        let first = Session::extract_plain_text_bytes(&mut ansi_remainder, b"abc\x1b[3");
        let second = Session::extract_plain_text_bytes(&mut ansi_remainder, b"1mdef\x1b[0m");
        assert_eq!(String::from_utf8_lossy(&first), "abc");
        assert_eq!(String::from_utf8_lossy(&second), "def");
    }

    #[test]
    fn test_extract_plain_text_bytes_strips_osc_terminated_by_bel() {
        let mut ansi_remainder = Vec::new();
        let plain =
            Session::extract_plain_text_bytes(&mut ansi_remainder, b"a\x1b]0;window title\x07b");
        assert_eq!(String::from_utf8_lossy(&plain), "ab");
        assert!(ansi_remainder.is_empty());
    }

    #[test]
    fn test_extract_plain_text_bytes_strips_osc_terminated_by_st() {
        let mut ansi_remainder = Vec::new();
        let plain =
            Session::extract_plain_text_bytes(&mut ansi_remainder, b"a\x1b]0;window title\x1b\\b");
        assert_eq!(String::from_utf8_lossy(&plain), "ab");
        assert!(ansi_remainder.is_empty());
    }

    #[test]
    fn test_extract_plain_text_bytes_strips_dcs_sequence() {
        let mut ansi_remainder = Vec::new();
        let plain = Session::extract_plain_text_bytes(&mut ansi_remainder, b"a\x1bP1$r0m\x1b\\b");
        assert_eq!(String::from_utf8_lossy(&plain), "ab");
        assert!(ansi_remainder.is_empty());
    }

    #[test]
    fn test_extract_plain_text_bytes_handles_split_osc_across_reads() {
        let mut ansi_remainder = Vec::new();
        let first = Session::extract_plain_text_bytes(&mut ansi_remainder, b"a\x1b]0;tit");
        assert_eq!(String::from_utf8_lossy(&first), "a");
        assert!(!ansi_remainder.is_empty());

        let second = Session::extract_plain_text_bytes(&mut ansi_remainder, b"le\x07b");
        assert_eq!(String::from_utf8_lossy(&second), "b");
        assert!(ansi_remainder.is_empty());
    }

    #[test]
    fn test_extract_plain_text_bytes_handles_st_split_across_reads() {
        // The ESC of the ESC-\ terminator lands at the end of one read and
        // the backslash at the start of the next.
        let mut ansi_remainder = Vec::new();
        let first = Session::extract_plain_text_bytes(&mut ansi_remainder, b"a\x1b]0;title\x1b");
        assert_eq!(String::from_utf8_lossy(&first), "a");

        let second = Session::extract_plain_text_bytes(&mut ansi_remainder, b"\\b");
        assert_eq!(String::from_utf8_lossy(&second), "b");
        assert!(ansi_remainder.is_empty());
    }

    #[test]
    fn test_dsr_responder_whole_sequence_in_one_read() {
        let mut dsr = DsrResponder::new();
        let response = dsr.respond(b"\x1b[6n", (4, 9));
        // CPR is 1-based
        assert_eq!(response.as_deref(), Some(b"\x1b[5;10R".as_ref()));
    }

    #[test]
    fn test_dsr_responder_sequence_split_across_reads() {
        let mut dsr = DsrResponder::new();
        assert_eq!(dsr.respond(b"startup noise\x1b[", (0, 0)), None);
        let response = dsr.respond(b"6n", (0, 0));
        assert_eq!(response.as_deref(), Some(b"\x1b[1;1R".as_ref()));
    }

    #[test]
    fn test_dsr_responder_ignores_unrelated_escape_sequences() {
        let mut dsr = DsrResponder::new();
        assert_eq!(dsr.respond(b"\x1b[31mred\x1b[0m\x1b[6m", (0, 0)), None);
        // The tail of the previous read must not conjure a match either
        assert_eq!(dsr.respond(b"more text", (0, 0)), None);
    }

    #[test]
    fn test_dsr_responder_does_not_double_count_across_reads() {
        let mut dsr = DsrResponder::new();
        // A fully matched query leaves only 3 tail bytes, so it can never be
        // re-matched by the next read.
        assert!(dsr.respond(b"\x1b[6n", (0, 0)).is_some());
        assert_eq!(dsr.respond(b"plain output", (0, 0)), None);
    }

    #[test]
    fn test_dsr_responder_multiple_queries_in_one_chunk() {
        let mut dsr = DsrResponder::new();
        let response = dsr.respond(b"\x1b[6nabc\x1b[6n", (0, 0));
        assert_eq!(response.as_deref(), Some(b"\x1b[1;1R\x1b[1;1R".as_ref()));
    }

    #[test]
    fn test_codex_fallback_reassembles_two_byte_char_split_across_reads() {
        // "é" = 0xC3 0xA9, split at its only interior boundary
        let mut fallback = CodexFallback::new(100);
        fallback.ingest(b"caf\xc3");
        assert_eq!(fallback.partial_output_line, "caf");
        fallback.ingest(b"\xa9!");
        assert_eq!(fallback.partial_output_line, "café!");
        assert!(!fallback.partial_output_line.contains('\u{FFFD}'));
    }

    #[test]
    fn test_codex_fallback_reassembles_four_byte_char_split_at_every_boundary() {
        // "😀" = F0 9F 98 80
        let emoji = "😀".as_bytes();
        for split in 1..emoji.len() {
            let mut fallback = CodexFallback::new(100);
            fallback.ingest(b"x");
            fallback.ingest(&emoji[..split]);
            fallback.ingest(&emoji[split..]);
            assert_eq!(
                fallback.partial_output_line, "x😀",
                "split at byte {split} corrupted the char"
            );
        }
    }

    #[test]
    fn test_codex_fallback_genuinely_invalid_bytes_become_replacement_char() {
        let mut fallback = CodexFallback::new(100);
        // 0xFF can never start a UTF-8 sequence; it must not be held back
        fallback.ingest(b"a\xffb");
        assert_eq!(fallback.partial_output_line, "a\u{FFFD}b");

        // A lead byte followed by a non-continuation byte degrades too
        let mut fallback = CodexFallback::new(100);
        fallback.ingest(b"a\xc3");
        fallback.ingest(b"b");
        assert_eq!(fallback.partial_output_line, "a\u{FFFD}b");
    }
}
