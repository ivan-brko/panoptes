//! Codex rollout parsing
//!
//! Codex writes every event of a conversation to
//! `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl` as it happens -
//! measured flush latency is under 50ms, comfortably fast enough to drive a
//! live display.
//!
//! For a Codex without lifecycle hooks (before 0.156.1) this is the only
//! usable channel for its state: `notify` fires once per turn and cannot be
//! extended. Where the hooks report, they own the state and this file supplies
//! usage figures only (see `state_machine::admits_transcript_event`).
//!
//! Two shapes matter. `event_msg` records describe what the session is doing;
//! `response_item` records describe what the model emitted. Tool *starts* only
//! exist in the second: verified across 200 real rollouts, `event_msg` contains
//! no `*_begin` events at all, so `function_call` / `function_call_output` is
//! the begin/end pair.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::agent::events::{AgentEvent, RateLimitWindow, UsageSnapshot, WindowSource};

/// The `session_meta` header of a Codex rollout file
///
/// Beware the field names on a subagent rollout: `payload.id` is the
/// subagent's own ID while `payload.session_id` is its *parent's*, and a
/// second `session_meta` record follows carrying the parent's metadata. This
/// struct always reports the rollout's own identity, with the parent (if any)
/// in [`RolloutMeta::parent_id`].
#[derive(Debug, Clone)]
pub struct RolloutMeta {
    /// The conversation's own ID
    pub id: String,
    /// Working directory the conversation started in
    pub cwd: Option<PathBuf>,
    /// When the conversation began - the rollout's own creation timestamp,
    /// deliberately independent of the file's mtime, which is bumped on every
    /// turn
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    /// What kind of thread wrote this rollout
    ///
    /// Only a [`RolloutKind::Session`] can be a Panoptes session's own
    /// conversation, and only a [`RolloutKind::Subagent`] counts as work its
    /// parent is waiting on.
    pub kind: RolloutKind,
    /// The conversation this rollout was forked from, when it is a subagent's
    pub parent_id: Option<String>,
    /// Where the copy of the parent's history at the top of this rollout ends
    pub copied_history: CopiedHistory,
}

/// What kind of thread wrote a rollout
///
/// Codex writes a rollout for more than the conversations a user is having,
/// and the `session_meta` header is the only place that says which is which.
/// The shapes below are from Codex's own `SessionSource` / `ThreadSource`
/// (`protocol/src/protocol.rs` at `rust-v0.156.1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RolloutKind {
    /// A conversation a user is having: `source` is `"cli"`, `"exec"`,
    /// `"vscode"` and so on
    Session,
    /// Work the model delegated, which its parent is waiting on
    ///
    /// Codex subagents get their own rollout files, which look enough like a
    /// session's own to be claimed by mistake - a real example on this machine
    /// is a subagent rollout whose `cwd` is a Panoptes worktree, with its own
    /// fresh start timestamp.
    Subagent,
    /// A background thread Codex runs for itself - memory consolidation, or
    /// the guardian that reviews approval requests
    ///
    /// These can carry subagent-shaped metadata, and the guardian names the
    /// session it reviews for, but nothing user-visible is running while they
    /// do. Counting one as a subagent would hold a Waiting session awake.
    System,
}

/// How the copy of a parent's history at the top of a rollout is delimited
///
/// A forked rollout - every spawned subagent that inherits context is one -
/// opens by replaying the parent's history: the parent's own `session_meta`,
/// then its turns, re-stamped to the instant of the fork and written in one
/// burst. None of it happened in this rollout, so a reader starting from the
/// top must skip it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopiedHistory {
    /// Nothing was copied: not a fork, or a fork whose history is referenced
    /// (`history_base`) rather than copied in
    None,
    /// Paginated rollouts number every record, and a subagent's header names
    /// the first ordinal that is its own (`subagent_history_start_ordinal`)
    BeforeOrdinal(u64),
    /// Legacy rollouts carry no boundary in the header. The burst ends at the
    /// fork's own `thread_settings_applied` (the one naming this rollout's
    /// thread, written in the same append as the copy - present in 0.156.1,
    /// absent in 0.145, whose checkpoints name no thread), or,
    /// on versions that predate it, at the first gap of a second or more
    /// between records - a heuristic, see [`COPIED_HISTORY_MAX_GAP`]
    Burst,
}

/// The longest pause between two records of one copied-history burst
///
/// A heuristic, and only the fallback for rollouts too old to mark where the
/// copy ends. The copy is written in one synchronous append (every record of a
/// real 0.142 fork shares the same few milliseconds), while the child's own
/// work only lands after a model round trip - seconds later. The cost of the
/// heuristic is the child's own `task_started`, written a few milliseconds
/// after the copy and so indistinguishable from it; the turn's later records
/// still arrive. T3 Code and `ccusage` use the same threshold.
pub const COPIED_HISTORY_MAX_GAP: chrono::TimeDelta = chrono::TimeDelta::seconds(1);

/// Every rollout file under a Codex sessions directory
///
/// Rollouts are filed under `sessions/YYYY/MM/DD`, so this walks exactly three
/// levels rather than recursing over an unbounded tree. A missing or
/// unreadable directory yields an empty list, which is the normal state before
/// Codex has written anything.
pub fn rollout_files(sessions_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for year in subdirs(sessions_dir) {
        for month in subdirs(&year) {
            for day in subdirs(&month) {
                let Ok(entries) = std::fs::read_dir(&day) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                        out.push(path);
                    }
                }
            }
        }
    }
    out
}

/// List immediate subdirectories, ignoring anything unreadable
fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect()
}

/// Read the `session_meta` header of a rollout file
///
/// Only the first line is read: the rest of a rollout is the conversation,
/// which can be large and is of no interest here. Returns `None` for anything
/// that is not a rollout with a parseable `session_meta` header - including a
/// file observed before Codex finished writing its first line.
pub fn read_session_meta(path: &Path) -> Option<RolloutMeta> {
    use std::io::BufRead;

    let file = std::fs::File::open(path).ok()?;
    let mut first_line = String::new();
    std::io::BufReader::new(file)
        .read_line(&mut first_line)
        .ok()?;

    meta_from_line(&first_line)
}

/// Interpret one line as a `session_meta` header, or `None` for anything else
///
/// For a caller that has already read the line itself - the conversation
/// scanner reads bounded prefixes, and must not reopen the file for this.
pub fn meta_from_line(line: &str) -> Option<RolloutMeta> {
    meta_from_record(&serde_json::from_str(line).ok()?)
}

/// Interpret a `session_meta` record, or `None` for any other record
fn meta_from_record(value: &Value) -> Option<RolloutMeta> {
    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    let payload = value.get("payload")?;
    let id = payload.get("id").and_then(Value::as_str)?.to_string();

    // Codex records the timestamp on the payload and again on the envelope;
    // either identifies when the conversation began
    let created_at = payload
        .get("timestamp")
        .or_else(|| value.get("timestamp"))
        .and_then(Value::as_str)
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&chrono::Utc));

    Some(RolloutMeta {
        id,
        cwd: payload
            .get("cwd")
            .and_then(Value::as_str)
            .map(PathBuf::from),
        created_at,
        kind: rollout_kind(payload),
        parent_id: parent_conversation_id(payload),
        copied_history: copied_history(payload),
    })
}

/// Translate one rollout line into a session event
///
/// Returns `None` for records that say nothing about session state. Never
/// fails: a rollout is written by another process and may be observed
/// mid-write, so anything unparseable is simply not an event.
pub fn parse_line(line: &str) -> Option<AgentEvent> {
    let record: Value = serde_json::from_str(line).ok()?;
    let payload = record.get("payload")?;

    match record.get("type")?.as_str()? {
        "event_msg" => parse_event_msg(payload),
        "response_item" => parse_response_item(payload),
        _ => None,
    }
}

/// Skips the copied parent history at the top of a rollout read from its start
///
/// Fed every complete line from the very first, in order. The first line is
/// the rollout's own `session_meta`, which decides whether there is anything to
/// skip (see [`CopiedHistory`]); for all but forks, the answer is no and the
/// skipper retires after one line.
#[derive(Debug, Default)]
pub struct CopiedHistorySkip {
    state: SkipState,
}

#[derive(Debug, Default)]
enum SkipState {
    /// The header has not been seen yet
    #[default]
    AwaitingMeta,
    /// Inside the copy of a paginated subagent's inherited records
    BeforeOrdinal(u64),
    /// Inside a legacy fork's copied burst
    Burst {
        own_id: String,
        /// The previous record's timestamp, for the gap heuristic
        last: Option<chrono::DateTime<chrono::FixedOffset>>,
    },
    /// Past the copy, or there was none: everything from here is the
    /// rollout's own
    Done,
}

impl CopiedHistorySkip {
    /// A skipper for a rollout about to be read from its first byte
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the copy has been left behind, so every later line is the
    /// rollout's own and the skipper can be dropped
    pub fn is_done(&self) -> bool {
        matches!(self.state, SkipState::Done)
    }

    /// Whether this line is copied history that must not be read as news
    pub fn skips(&mut self, line: &str) -> bool {
        if self.is_done() {
            return false;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            // Without a header first there is no copy to find. Inside one, an
            // unparseable line is never an event, so skipping it is harmless,
            // and it must not end the copy early.
            if matches!(self.state, SkipState::AwaitingMeta) {
                self.state = SkipState::Done;
                return false;
            }
            return true;
        };

        match &mut self.state {
            SkipState::AwaitingMeta => {
                // Anything but a header first means this is not a rollout
                // shape that copies history; read it all
                self.state = match meta_from_record(&record) {
                    Some(meta) => match meta.copied_history {
                        CopiedHistory::None => SkipState::Done,
                        CopiedHistory::BeforeOrdinal(start) => SkipState::BeforeOrdinal(start),
                        CopiedHistory::Burst => SkipState::Burst {
                            own_id: meta.id,
                            last: record_timestamp(&record),
                        },
                    },
                    None => SkipState::Done,
                };
                // The header is the rollout's own either way, and not an event
                false
            }
            SkipState::BeforeOrdinal(start) => {
                let inherited = record
                    .get("ordinal")
                    .and_then(Value::as_u64)
                    .is_some_and(|ordinal| ordinal < *start);
                if !inherited {
                    self.state = SkipState::Done;
                }
                inherited
            }
            SkipState::Burst { own_id, last } => {
                // The exact end: the fork's own settings checkpoint, appended
                // with the copy. The parent's copied checkpoints name the
                // parent, or (before 0.156) no thread at all.
                let payload = record.get("payload");
                let own_checkpoint = record.get("type").and_then(Value::as_str)
                    == Some("event_msg")
                    && payload.and_then(|p| p.get("type")).and_then(Value::as_str)
                        == Some("thread_settings_applied")
                    && payload
                        .and_then(|p| p.get("thread_id"))
                        .and_then(Value::as_str)
                        == Some(own_id.as_str());
                if own_checkpoint {
                    self.state = SkipState::Done;
                    return true;
                }

                // The fallback: a pause the copy could not contain
                let at = record_timestamp(&record);
                if let (Some(at), Some(prev)) = (at, *last) {
                    if at - prev >= COPIED_HISTORY_MAX_GAP {
                        self.state = SkipState::Done;
                        return false;
                    }
                }
                if at.is_some() {
                    *last = at;
                }
                true
            }
            SkipState::Done => false,
        }
    }
}

/// The envelope timestamp Codex writes on every record
fn record_timestamp(record: &Value) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    record
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
}

/// `event_msg` records: the session narrating itself
fn parse_event_msg(payload: &Value) -> Option<AgentEvent> {
    match payload.get("type")?.as_str()? {
        "task_started" => Some(AgentEvent::TurnStarted { title: None }),
        "task_complete" => Some(AgentEvent::TurnCompleted {
            last_message: payload
                .get("last_agent_message")
                .and_then(Value::as_str)
                .map(str::to_string),
        }),
        "turn_aborted" => Some(AgentEvent::TurnAborted),
        "context_compacted" => Some(AgentEvent::ContextCompacted),
        "token_count" => Some(AgentEvent::Usage(parse_token_count(payload))),

        // Command and MCP results also arrive here, but they are the *same*
        // completions already seen as `function_call_output`. Acting on both
        // would retire each tool twice; the response_item pair is the
        // authoritative one because it is the only place starts appear.
        _ => None,
    }
}

/// `response_item` records: what the model emitted
fn parse_response_item(payload: &Value) -> Option<AgentEvent> {
    match payload.get("type")?.as_str()? {
        // Written when the model asks for a tool, before it runs
        "function_call" | "custom_tool_call" | "local_shell_call" | "tool_search_call" => {
            let key = tool_key(payload)?;
            let name = payload
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            Some(AgentEvent::ToolStarted { key, name })
        }
        "function_call_output" | "custom_tool_call_output" | "tool_search_output" => {
            Some(AgentEvent::ToolFinished {
                key: tool_key(payload)?,
            })
        }
        // `web_search_call` has no matching output record - it is written once,
        // already finished - so there is no interval to show. Counted across
        // real rollouts: 9 calls, 0 outputs.
        _ => None,
    }
}

/// Identify a tool invocation so its start and end can be paired
///
/// Codex names this `call_id`; a few record shapes use `id` instead.
fn tool_key(payload: &Value) -> Option<String> {
    payload
        .get("call_id")
        .or_else(|| payload.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Pull token and rate-limit figures out of a `token_count` record
fn parse_token_count(payload: &Value) -> UsageSnapshot {
    let info = payload.get("info");
    let total = info
        .and_then(|i| i.get("total_token_usage"))
        .and_then(|u| u.get("total_tokens"))
        .and_then(Value::as_u64);
    let window = info
        .and_then(|i| i.get("model_context_window"))
        .and_then(Value::as_u64);

    let mut snapshot = UsageSnapshot {
        total_tokens: total,
        context_window: window,
        // Codex states its window itself rather than leaving it to be guessed.
        // Only claimed when a window is actually present, so a record without
        // one stays an empty snapshot
        context_window_source: if window.is_some() {
            WindowSource::Observed
        } else {
            WindowSource::default()
        },
        context_window_model: None,
        model: info
            .and_then(|i| i.get("model"))
            .and_then(Value::as_str)
            .map(str::to_string),
        ..Default::default()
    };

    let Some(limits) = payload.get("rate_limits").filter(|r| r.is_object()) else {
        return snapshot;
    };
    // Codex reports several allowances through the same record: `codex` is the
    // account's main one, while model-specific ids (the Spark model writes
    // `codex_bengalfox`) describe a separate pool, often at 0%. Taking those
    // would overwrite the real figure with an unrelated one. Older versions
    // write no id at all, and then the record can only be the main allowance.
    if limits
        .get("limit_id")
        .and_then(Value::as_str)
        .is_some_and(|id| id != "codex")
    {
        return snapshot;
    }

    let primary = limits.get("primary");
    snapshot.primary = primary.and_then(parse_rate_limit_window);
    snapshot.secondary = limits.get("secondary").and_then(parse_rate_limit_window);
    // Current versions name the plan beside the windows; older ones put it
    // inside the primary window
    snapshot.plan = limits
        .get("plan_type")
        .and_then(Value::as_str)
        .or_else(|| {
            primary
                .and_then(|p| p.get("plan_type"))
                .and_then(Value::as_str)
        })
        .map(str::to_string);
    snapshot.limit_reached = limits
        .get("rate_limit_reached_type")
        .and_then(Value::as_str)
        .map(str::to_string);
    snapshot
}

/// Read one window of a `rate_limits` block; `None` if it has no usage figure
fn parse_rate_limit_window(window: &Value) -> Option<RateLimitWindow> {
    Some(RateLimitWindow {
        used_percent: window.get("used_percent").and_then(Value::as_f64)?,
        window_minutes: window.get("window_minutes").and_then(Value::as_u64),
        resets_at: window.get("resets_at").and_then(parse_resets_at),
    })
}

/// A reset time as Codex writes it: epoch seconds now, RFC 3339 in older versions
fn parse_resets_at(value: &Value) -> Option<DateTime<Utc>> {
    match value {
        Value::Number(n) => DateTime::from_timestamp(n.as_i64()?, 0),
        Value::String(s) => DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|t| t.with_timezone(&Utc)),
        _ => None,
    }
}

/// Classify a `session_meta` payload
///
/// See [`RolloutKind`] for why this matters; consumers read it from the struct
/// rather than re-parsing raw payloads.
///
/// System threads are recognised first because they wear subagent clothing:
/// memory consolidation has been written as `source: {"subagent":
/// "memory_consolidation"}` as well as `source: {"internal": ...}`, and the
/// guardian is saved as `source: {"subagent": {"other": "guardian"}}`.
/// `thread_source`, which newer versions add, names both outright.
fn rollout_kind(payload: &Value) -> RolloutKind {
    let source = payload.get("source");
    let subagent = source.and_then(|s| s.get("subagent"));
    let thread_source = payload.get("thread_source").and_then(Value::as_str);

    let system = source.and_then(|s| s.get("internal")).is_some()
        || matches!(
            thread_source,
            Some("memory_consolidation" | "guardian_review")
        )
        || subagent.and_then(Value::as_str) == Some("memory_consolidation")
        || subagent
            .and_then(|s| s.get("other"))
            .and_then(Value::as_str)
            == Some("guardian");
    if system {
        return RolloutKind::System;
    }

    let subagent = subagent.is_some()
        || thread_source == Some("subagent")
        || payload.get("forked_from_id").is_some_and(|v| !v.is_null());
    if subagent {
        RolloutKind::Subagent
    } else {
        RolloutKind::Session
    }
}

/// Where a rollout's copy of its parent's history ends, from its header
///
/// Only a fork copies anything. Across every subagent rollout on this machine
/// (0.115 to 0.142), each one with a `forked_from_id` opens with a second,
/// copied `session_meta`, and each one without starts straight on its own
/// `task_started` - so a subagent that is not a fork must not be skipped into.
fn copied_history(payload: &Value) -> CopiedHistory {
    if !payload.get("forked_from_id").is_some_and(|v| !v.is_null()) {
        return CopiedHistory::None;
    }
    if let Some(start) = payload
        .get("subagent_history_start_ordinal")
        .and_then(Value::as_u64)
    {
        return CopiedHistory::BeforeOrdinal(start);
    }
    // A referenced fork points at the parent's file instead of copying it
    if payload.get("history_base").is_some_and(|v| !v.is_null()) {
        return CopiedHistory::None;
    }
    CopiedHistory::Burst
}

/// The conversation this rollout was forked from, if it is a subagent's
fn parent_conversation_id(payload: &Value) -> Option<String> {
    if let Some(id) = payload.get("forked_from_id").and_then(Value::as_str) {
        return Some(id.to_string());
    }
    payload
        .get("source")
        .and_then(|s| s.get("subagent"))
        .and_then(|s| s.get("thread_spawn"))
        .and_then(|s| s.get("parent_thread_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_turn_lifecycle() {
        assert_eq!(
            parse_line(r#"{"type":"event_msg","payload":{"type":"task_started"}}"#),
            Some(AgentEvent::TurnStarted { title: None })
        );
        assert_eq!(
            parse_line(
                r#"{"type":"event_msg","payload":{"type":"task_complete","last_agent_message":"done"}}"#
            ),
            Some(AgentEvent::TurnCompleted {
                last_message: Some("done".to_string())
            })
        );
        assert_eq!(
            parse_line(r#"{"type":"event_msg","payload":{"type":"turn_aborted"}}"#),
            Some(AgentEvent::TurnAborted)
        );
        assert_eq!(
            parse_line(r#"{"type":"event_msg","payload":{"type":"context_compacted"}}"#),
            Some(AgentEvent::ContextCompacted)
        );
    }

    #[test]
    fn test_tool_calls_pair_by_call_id() {
        // `event_msg` carries no *_begin events at all, so the start of a tool
        // only exists as a response_item
        let start = parse_line(
            r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","call_id":"call_1"}}"#,
        );
        assert_eq!(
            start,
            Some(AgentEvent::ToolStarted {
                key: "call_1".to_string(),
                name: "shell".to_string()
            })
        );

        let end = parse_line(
            r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"call_1"}}"#,
        );
        assert_eq!(
            end,
            Some(AgentEvent::ToolFinished {
                key: "call_1".to_string()
            })
        );
    }

    #[test]
    fn test_tool_search_is_a_pair_but_web_search_is_not() {
        // Counted across real rollouts: tool_search_call/tool_search_output
        // appear 20/20, while web_search_call appears 9 times with no matching
        // output record at all. Treating the latter as a start would leave a
        // tool in flight forever.
        assert!(matches!(
            parse_line(
                r#"{"type":"response_item","payload":{"type":"tool_search_call","call_id":"s1","name":"search"}}"#
            ),
            Some(AgentEvent::ToolStarted { .. })
        ));
        assert!(matches!(
            parse_line(
                r#"{"type":"response_item","payload":{"type":"tool_search_output","call_id":"s1"}}"#
            ),
            Some(AgentEvent::ToolFinished { .. })
        ));
        assert_eq!(
            parse_line(
                r#"{"type":"response_item","payload":{"type":"web_search_call","id":"w1"}}"#
            ),
            None
        );
    }

    #[test]
    fn test_command_end_events_are_ignored() {
        // These describe the same completion as function_call_output. Acting on
        // both would retire the tool twice.
        assert_eq!(
            parse_line(
                r#"{"type":"event_msg","payload":{"type":"exec_command_end","call_id":"c"}}"#
            ),
            None
        );
        assert_eq!(
            parse_line(
                r#"{"type":"event_msg","payload":{"type":"mcp_tool_call_end","call_id":"c"}}"#
            ),
            None
        );
    }

    #[test]
    fn test_token_count_accepts_legacy_string_resets_at() {
        // The shape Codex wrote before 0.156: a string reset time and the plan
        // inside the primary window. Formerly `test_token_count`, now also
        // checking the reset time it used to drop.
        let line = r#"{"type":"event_msg","payload":{"type":"token_count",
            "info":{"model":"gpt-5-codex","model_context_window":272000,
                    "total_token_usage":{"total_tokens":48000}},
            "rate_limits":{"primary":{"used_percent":12.5,"window_minutes":300,
                                      "resets_at":"2026-07-22T18:00:00Z","plan_type":"pro"}}}}"#;

        let Some(AgentEvent::Usage(usage)) = parse_line(line) else {
            panic!("expected a usage event");
        };
        assert_eq!(usage.total_tokens, Some(48_000));
        assert_eq!(usage.context_window, Some(272_000));
        assert_eq!(usage.model.as_deref(), Some("gpt-5-codex"));
        let primary = usage.primary.expect("primary window");
        assert_eq!(primary.used_percent, 12.5);
        assert_eq!(primary.window_minutes, Some(300));
        assert_eq!(
            primary.resets_at,
            Some(Utc.with_ymd_and_hms(2026, 7, 22, 18, 0, 0).unwrap())
        );
        assert_eq!(usage.secondary, None);
        assert_eq!(usage.plan.as_deref(), Some("pro"));
    }

    #[test]
    fn test_token_count_reads_current_rate_limit_shape() {
        // The shape a Codex 0.156.1 rollout writes, plan name filled in
        let line = r#"{"type":"event_msg","payload":{"type":"token_count",
            "info":{"total_token_usage":{"total_tokens":48000},"model_context_window":258400},
            "rate_limits":{
              "limit_id": "codex", "limit_name": null,
              "primary":   {"used_percent": 1.0,  "window_minutes": 300,   "resets_at": 1790167277},
              "secondary": {"used_percent": 40.0, "window_minutes": 10080, "resets_at": 1790602271},
              "credits": {"has_credits": false, "unlimited": false, "balance": "0"},
              "plan_type": "plus", "rate_limit_reached_type": null
            }}}"#;

        let Some(AgentEvent::Usage(usage)) = parse_line(line) else {
            panic!("expected a usage event");
        };
        assert_eq!(
            usage.primary,
            Some(RateLimitWindow {
                used_percent: 1.0,
                window_minutes: Some(300),
                resets_at: DateTime::from_timestamp(1_790_167_277, 0),
            })
        );
        assert_eq!(
            usage.secondary,
            Some(RateLimitWindow {
                used_percent: 40.0,
                window_minutes: Some(10_080),
                resets_at: DateTime::from_timestamp(1_790_602_271, 0),
            })
        );
        assert_eq!(usage.plan.as_deref(), Some("plus"));
        assert_eq!(usage.limit_reached, None);
    }

    #[test]
    fn test_token_count_ignores_non_codex_limit_id() {
        let line = r#"{"type":"event_msg","payload":{"type":"token_count",
            "info":{"model":"gpt-5.3-codex-spark","model_context_window":128000,
                    "total_token_usage":{"total_tokens":9000}},
            "rate_limits":{"limit_id":"codex_bengalfox","limit_name":"GPT-5.3-Codex-Spark",
              "primary":{"used_percent":0.0,"window_minutes":300,"resets_at":1790167277},
              "secondary":{"used_percent":0.0,"window_minutes":10080,"resets_at":1790602271},
              "credits":{"has_credits":false,"unlimited":false,"balance":null},
              "plan_type":"plus","rate_limit_reached_type":"rate_limit_exceeded"}}}"#;

        let Some(AgentEvent::Usage(usage)) = parse_line(line) else {
            panic!("expected a usage event");
        };
        // The conversation's own figures are still this conversation's
        assert_eq!(usage.total_tokens, Some(9_000));
        assert_eq!(usage.context_window, Some(128_000));
        assert_eq!(usage.model.as_deref(), Some("gpt-5.3-codex-spark"));
        // But the limits describe another allowance entirely
        assert_eq!(usage.primary, None);
        assert_eq!(usage.secondary, None);
        assert_eq!(usage.plan, None);
        assert_eq!(usage.limit_reached, None);
    }

    #[test]
    fn test_token_count_reads_limit_reached() {
        let line = r#"{"type":"event_msg","payload":{"type":"token_count","info":null,
            "rate_limits":{"limit_id":"codex",
              "primary":{"used_percent":100.0,"window_minutes":300,"resets_at":1790167277},
              "plan_type":"business",
              "rate_limit_reached_type":"workspace_member_credits_depleted"}}}"#;

        let Some(AgentEvent::Usage(usage)) = parse_line(line) else {
            panic!("expected a usage event");
        };
        assert_eq!(
            usage.limit_reached.as_deref(),
            Some("workspace_member_credits_depleted")
        );
        assert_eq!(usage.primary.map(|p| p.used_percent), Some(100.0));
    }

    #[test]
    fn test_malformed_input_is_never_an_error() {
        // Rollouts are written by another process and can be read mid-write
        for line in [
            "",
            "   ",
            "not json at all",
            r#"{"type":"event_msg","payload":{"type":"task_st"#,
            r#"{"type":"event_msg"}"#,
            r#"{"payload":{"type":"task_started"}}"#,
            r#"{"type":"event_msg","payload":null}"#,
            r#"{"type":"event_msg","payload":{}}"#,
            "null",
            "[]",
        ] {
            assert_eq!(parse_line(line), None, "for {line:?}");
        }
    }

    #[test]
    fn test_unknown_record_types_are_ignored_not_guessed() {
        assert_eq!(
            parse_line(r#"{"type":"world_state","payload":{"anything":1}}"#),
            None
        );
        assert_eq!(
            parse_line(r#"{"type":"event_msg","payload":{"type":"something_new"}}"#),
            None
        );
    }

    /// Write a rollout file with an arbitrary first line
    fn write_rollout_file(dir: &Path, name: &str, first_line: &str) -> PathBuf {
        let day = dir.join("2026").join("07").join("22");
        std::fs::create_dir_all(&day).unwrap();
        let path = day.join(name);
        std::fs::write(&path, format!("{}\n{{\"type\":\"message\"}}\n", first_line)).unwrap();
        path
    }

    #[test]
    fn test_rollout_files_walks_the_dated_tree_and_filters_jsonl() {
        let dir = tempfile::TempDir::new().unwrap();
        let a = write_rollout_file(dir.path(), "rollout-a.jsonl", "{}");
        let b = write_rollout_file(dir.path(), "rollout-b.jsonl", "{}");
        // Not a rollout, must be ignored
        write_rollout_file(dir.path(), "notes.txt", "irrelevant");
        // A file outside the YYYY/MM/DD depth must not be picked up
        std::fs::write(dir.path().join("stray.jsonl"), "{}\n").unwrap();

        let mut found = rollout_files(dir.path());
        found.sort();
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(found, expected);

        // A sessions dir that does not exist yet is the normal starting state
        assert!(rollout_files(&dir.path().join("nope")).is_empty());
    }

    #[test]
    fn test_read_session_meta_full_header() {
        let dir = tempfile::TempDir::new().unwrap();
        let line = serde_json::json!({
            "timestamp": "2026-07-22T10:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": "conv-1",
                "timestamp": "2026-07-22T10:00:05Z",
                "cwd": "/work/here",
                "originator": "codex_cli_rs"
            }
        });
        let path = write_rollout_file(dir.path(), "rollout-full.jsonl", &line.to_string());

        let meta = read_session_meta(&path).expect("valid session_meta");
        assert_eq!(meta.id, "conv-1");
        assert_eq!(meta.cwd.as_deref(), Some(Path::new("/work/here")));
        // The payload timestamp wins over the envelope's
        assert_eq!(
            meta.created_at.unwrap().to_rfc3339(),
            "2026-07-22T10:00:05+00:00"
        );
        assert_eq!(meta.kind, RolloutKind::Session);
        assert_eq!(meta.copied_history, CopiedHistory::None);
        assert_eq!(meta.parent_id, None);
    }

    #[test]
    fn test_read_session_meta_falls_back_to_envelope_timestamp() {
        // Some rollouts stamp only the envelope, not the payload. Discovery
        // matches on this timestamp, so losing it would make the rollout
        // undiscoverable rather than merely less precise.
        let dir = tempfile::TempDir::new().unwrap();
        let line = serde_json::json!({
            "timestamp": "2026-07-22T09:30:00Z",
            "type": "session_meta",
            "payload": {
                "id": "envelope-only",
                "cwd": "/work/here"
            }
        });
        let path = write_rollout_file(dir.path(), "rollout-envelope.jsonl", &line.to_string());

        let meta = read_session_meta(&path).expect("valid session_meta");
        assert_eq!(meta.id, "envelope-only");
        assert_eq!(
            meta.created_at.unwrap().to_rfc3339(),
            "2026-07-22T09:30:00+00:00"
        );
    }

    #[test]
    fn test_read_session_meta_reports_subagents() {
        let dir = tempfile::TempDir::new().unwrap();
        let line = serde_json::json!({
            "timestamp": "2026-07-22T10:00:00Z",
            "type": "session_meta",
            "payload": {
                // `id` is the subagent's own; `session_id` is the parent's
                "id": "child",
                "session_id": "parent",
                "forked_from_id": "parent",
                "cwd": "/work/here",
                "source": {"subagent": {"thread_spawn": {"parent_thread_id": "parent"}}}
            }
        });
        let path = write_rollout_file(dir.path(), "rollout-sub.jsonl", &line.to_string());

        let meta = read_session_meta(&path).expect("valid session_meta");
        assert_eq!(meta.id, "child", "must report the rollout's own identity");
        assert_eq!(meta.kind, RolloutKind::Subagent);
        assert_eq!(meta.parent_id.as_deref(), Some("parent"));
    }

    #[test]
    fn test_read_session_meta_rejects_non_rollouts() {
        let dir = tempfile::TempDir::new().unwrap();
        for (name, first_line) in [
            ("broken.jsonl", "not json at all"),
            ("empty.jsonl", ""),
            ("wrong-type.jsonl", r#"{"type":"event_msg","payload":{}}"#),
            (
                "no-id.jsonl",
                r#"{"type":"session_meta","payload":{"cwd":"/x"}}"#,
            ),
        ] {
            let path = write_rollout_file(dir.path(), name, first_line);
            assert!(read_session_meta(&path).is_none(), "for {name}");
        }
        assert!(read_session_meta(Path::new("/no/such/file.jsonl")).is_none());
    }

    #[test]
    fn test_subagent_detection() {
        let forked = serde_json::json!({"id": "child", "forked_from_id": "parent"});
        assert_eq!(rollout_kind(&forked), RolloutKind::Subagent);
        assert_eq!(parent_conversation_id(&forked).as_deref(), Some("parent"));

        let spawned = serde_json::json!({
            "id": "child",
            "source": {"subagent": {"thread_spawn": {"parent_thread_id": "parent"}}}
        });
        assert_eq!(rollout_kind(&spawned), RolloutKind::Subagent);
        assert_eq!(parent_conversation_id(&spawned).as_deref(), Some("parent"));

        // `/review` and compaction are still delegated work
        let review = serde_json::json!({"id": "r", "source": {"subagent": "review"}});
        assert_eq!(rollout_kind(&review), RolloutKind::Subagent);

        // A normal session, including one that has been resumed many times
        let plain = serde_json::json!({"id": "own", "cwd": "/tmp", "source": "cli",
            "thread_source": "user"});
        assert_eq!(rollout_kind(&plain), RolloutKind::Session);
        assert_eq!(parent_conversation_id(&plain), None);

        // An explicit null must not read as "forked"
        let null_fork = serde_json::json!({"id": "own", "forked_from_id": null});
        assert_eq!(rollout_kind(&null_fork), RolloutKind::Session);
    }

    /// A redacted fixture, parsed as `read_session_meta` would
    fn fixture_meta(fixture: &str) -> RolloutMeta {
        let first = fixture.lines().next().unwrap();
        meta_from_record(&serde_json::from_str(first).unwrap()).expect("fixture header")
    }

    #[test]
    fn test_memory_consolidation_rollout_is_system() {
        // 0.156.1 shape (`memories/write/src/runtime.rs`): an internal source
        // and a thread_source both naming it
        let meta = fixture_meta(include_str!("fixtures/memory_consolidation_rollout.jsonl"));
        assert_eq!(meta.kind, RolloutKind::System);

        // Older versions filed it as a subagent - the shape T3 Code filters
        // and Codex's own rollout migration still recognises
        let as_subagent = serde_json::json!({"id": "m",
            "source": {"subagent": "memory_consolidation"}});
        assert_eq!(rollout_kind(&as_subagent), RolloutKind::System);

        // `thread_source` alone is enough, whatever `source` says
        let by_thread_source = serde_json::json!({"id": "m", "source": "exec",
            "thread_source": "memory_consolidation"});
        assert_eq!(rollout_kind(&by_thread_source), RolloutKind::System);
    }

    #[test]
    fn test_guardian_rollout_is_system_even_when_it_names_a_parent() {
        // Saved as `SubAgent(Other("guardian"))` and forked from the session
        // it reviews for (`core/src/thread_manager.rs`), so it links to a
        // parent exactly like a subagent does
        let guardian = serde_json::json!({"id": "g", "forked_from_id": "parent",
            "source": {"subagent": {"other": "guardian"}},
            "thread_source": "guardian_review"});
        assert_eq!(rollout_kind(&guardian), RolloutKind::System);
        assert_eq!(parent_conversation_id(&guardian).as_deref(), Some("parent"));

        let internal = serde_json::json!({"id": "g", "source": {"internal": "guardian"}});
        assert_eq!(rollout_kind(&internal), RolloutKind::System);
    }

    #[test]
    fn test_copied_history_is_read_from_the_header() {
        let legacy = fixture_meta(include_str!("fixtures/forked_legacy_rollout.jsonl"));
        assert_eq!(legacy.copied_history, CopiedHistory::Burst);

        let paginated = fixture_meta(include_str!("fixtures/forked_paginated_rollout.jsonl"));
        assert_eq!(paginated.copied_history, CopiedHistory::BeforeOrdinal(4));

        // A referenced fork keeps its inheritance in the parent's file
        let referenced = serde_json::json!({"id": "c", "forked_from_id": "p",
            "history_base": {"thread_id": "p", "end_ordinal_exclusive": 9, "end_byte_offset": 1}});
        assert_eq!(copied_history(&referenced), CopiedHistory::None);

        // A subagent that is not a fork starts on its own first turn
        let unforked = serde_json::json!({"id": "c",
            "source": {"subagent": {"thread_spawn": {"parent_thread_id": "p"}}}});
        assert_eq!(copied_history(&unforked), CopiedHistory::None);
    }

    /// The lines of a fixture the skipper lets through
    fn kept(fixture: &str) -> Vec<&str> {
        let mut skip = CopiedHistorySkip::new();
        fixture.lines().filter(|line| !skip.skips(line)).collect()
    }

    #[test]
    fn test_skip_ends_at_the_forks_own_settings_checkpoint() {
        let fixture = include_str!("fixtures/forked_marked_rollout.jsonl");
        let events: Vec<_> = kept(fixture).into_iter().filter_map(parse_line).collect();
        // The parent's copied turn is gone - including its checkpoint, which
        // names the parent - and the child's own turn, written 6ms after the
        // copy, survives because the marker ends the copy exactly
        assert_eq!(
            events,
            vec![
                AgentEvent::TurnStarted { title: None },
                AgentEvent::ToolStarted {
                    key: "child-call-1".to_string(),
                    name: "shell".to_string()
                },
            ]
        );
    }

    #[test]
    fn test_skip_ends_at_the_subagents_first_own_ordinal() {
        let fixture = include_str!("fixtures/forked_paginated_rollout.jsonl");
        let events: Vec<_> = kept(fixture).into_iter().filter_map(parse_line).collect();
        assert_eq!(events, vec![AgentEvent::TurnStarted { title: None }]);
    }

    #[test]
    fn test_skip_leaves_ordinary_rollouts_alone() {
        let fixture = include_str!("fixtures/memory_consolidation_rollout.jsonl");
        let mut skip = CopiedHistorySkip::new();
        assert!(fixture.lines().all(|line| !skip.skips(line)));
        assert!(skip.is_done(), "the skipper retires after the header");

        // A file that does not start with a header is read in full
        let mut skip = CopiedHistorySkip::new();
        assert!(!skip.skips(r#"{"type":"event_msg","payload":{"type":"task_started"}}"#));
        assert!(skip.is_done());
    }
}
