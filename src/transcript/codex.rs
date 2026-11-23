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
//! usage figures and failed turns only, the one turn ending no hook reports
//! (see `state_machine::admits_transcript_event`).
//!
//! Two shapes matter. `event_msg` records describe what the session is doing;
//! `response_item` records describe what the model emitted. Tool *starts* only
//! exist in the second: verified across 200 real rollouts, `event_msg` contains
//! no `*_begin` events at all, so `function_call` / `function_call_output` is
//! the begin/end pair.

use std::path::{Path, PathBuf};

use chrono::{DateTime, MappedLocalTime, NaiveDate, TimeZone, Utc};
use serde_json::Value;

use super::rollout_file;
use crate::agent::events::{AgentEvent, RateLimitWindow, UsageLimit, UsageSnapshot, WindowSource};

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
///
/// Compressed rollouts (`.jsonl.zst`) are listed too, except where the same
/// conversation's plain file sits beside one (see [`rollout_file`]).
pub fn rollout_files(sessions_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for year in subdirs(sessions_dir) {
        for month in subdirs(&year) {
            for day in subdirs(&month) {
                let Ok(entries) = std::fs::read_dir(&day) else {
                    continue;
                };
                let listed: Vec<PathBuf> = entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| rollout_file::is_rollout_path(path))
                    .collect();
                out.extend(rollout_file::prefer_plain(listed));
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
/// file observed before Codex finished writing its first line, and a
/// compressed rollout that does not decode.
///
/// Called on every rollout by each discovery poll, so a corrupt compressed
/// file is only noted at debug; the conversation scanner warns about it.
pub fn read_session_meta(path: &Path) -> Option<RolloutMeta> {
    use std::io::BufRead;

    let reader = match rollout_file::open(path) {
        Ok(reader) => reader,
        Err(e) => {
            tracing::debug!(path = %path.display(), error = %e, "Cannot open rollout");
            return None;
        }
    };
    let mut first_line = String::new();
    std::io::BufReader::new(reader)
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
        "event_msg" => parse_event_msg(payload, &record),
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
fn parse_event_msg(payload: &Value, record: &Value) -> Option<AgentEvent> {
    match payload.get("type")?.as_str()? {
        "task_started" => Some(AgentEvent::TurnStarted { title: None }),
        // A turn that died on an error ends with the same record, carrying the
        // error that killed it. Codex fires no `Stop` hook for such a turn
        // (`run_turn_stop_hooks` only runs once a sampling request succeeds,
        // `core/src/session/turn.rs` at `rust-v0.156.1`), and its `Error`
        // event is never persisted, so this is the only report there is.
        "task_complete" => Some(match payload.get("error").filter(|e| e.is_object()) {
            Some(error) => turn_failed(payload, error, record),
            None => AgentEvent::TurnCompleted {
                last_message: payload
                    .get("last_agent_message")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            },
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
    snapshot.individual = limits
        .get("individual_limit")
        .and_then(parse_individual_limit);
    // Plus and Pro accounts carry on past 100% on purchased credits
    snapshot.credits_available = limits.get("credits").filter(|c| c.is_object()).map(|c| {
        let flag = |key| c.get(key).and_then(Value::as_bool).unwrap_or(false);
        flag("has_credits") || flag("unlimited")
    });
    snapshot
}

/// Read a workspace member's credit allowance, which Codex writes as
/// `individual_limit: {limit, used, remaining_percent, resets_at}`
///
/// The snake_case spelling of what app-server reports as `individualLimit`.
/// Held as a window used to `100 - remaining_percent`, like the others.
fn parse_individual_limit(limit: &Value) -> Option<RateLimitWindow> {
    let remaining = limit.get("remaining_percent").and_then(Value::as_f64)?;
    Some(RateLimitWindow {
        used_percent: (100.0 - remaining).clamp(0.0, 100.0),
        window_minutes: None,
        resets_at: limit.get("resets_at").and_then(parse_resets_at),
    })
}

/// The label every usage-limit failure carries, whichever agent reports it
const USAGE_LIMIT: &str = "usage limit";

/// A `task_complete` that carries the error its turn died on
///
/// The reason comes from `codex_error_info`. When the error is a usage limit
/// the only reset this record has is the one in Codex's own message text
/// ("Try again at 3:37 PM."); the rate-limit figures written just before it
/// say which window it was, which [`RolloutLimits`] fills in.
fn turn_failed(payload: &Value, error: &Value, record: &Value) -> AgentEvent {
    let message = error.get("message").and_then(Value::as_str).unwrap_or("");
    let failed_at = payload
        .get("completed_at")
        .and_then(Value::as_i64)
        .and_then(|secs| DateTime::from_timestamp(secs, 0))
        .or_else(|| record_timestamp(record).map(|t| t.with_timezone(&Utc)))
        .unwrap_or_else(Utc::now);
    let retry_at = retry_at(message, failed_at);

    let info = error.get("codex_error_info");
    let mut reason = failure_reason(info);
    // Codex wraps some service failures as `other`; a retry time in the text is
    // only ever written for a usage limit
    if reason.is_none() && retry_at.is_some() {
        reason = Some(USAGE_LIMIT.to_string());
    }
    let limit = match (reason.as_deref(), retry_at) {
        (Some(USAGE_LIMIT), Some(at)) => Some(UsageLimit {
            window: String::new(),
            resets_at: Some(at),
            using_overage: false,
        }),
        _ => None,
    };
    AgentEvent::RolloutTurnFailed {
        open_to_usage_limit: reason.as_deref() != Some(USAGE_LIMIT) && could_be_usage_limit(info),
        reason,
        limit,
        turn_id: payload
            .get("turn_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        inferred: false,
    }
}

/// The label for a throughput limit, which passes in moments - unlike a usage
/// limit, which holds until its window resets
const RATE_LIMITED: &str = "rate limited";

/// Whether an error leaves open that the turn was refused on its usage
/// allowance: `other`, which Codex wraps service failures in, and retries given
/// up on a 429, which may be either kind of limit
fn could_be_usage_limit(info: Option<&Value>) -> bool {
    match info {
        Some(Value::String(code)) => code == "other",
        Some(Value::Object(map)) => {
            map.get("response_too_many_failed_attempts")
                .and_then(|d| d.get("http_status_code"))
                .and_then(Value::as_u64)
                == Some(429)
        }
        _ => false,
    }
}

/// Short label for why a Codex turn failed, from its `codex_error_info`
///
/// The codes are Codex's `CodexErrorInfo` as of `rust-v0.156.1`, written
/// snake_case: a bare string for most, an object keyed by the code for those
/// that carry an HTTP status. `rate_limit_exceeded` is a throughput limit,
/// which Codex retries and which passes in moments, so it is `rate limited`
/// rather than a usage limit; so are retries given up on a 429, unless the
/// figures show the allowance spent (see [`RolloutLimits`]). `other`, or no
/// code at all, says nothing and gives `None`; a code this table does not know
/// is kept as it is.
fn failure_reason(info: Option<&Value>) -> Option<String> {
    let (code, status) = match info? {
        Value::String(code) => (code.as_str(), None),
        Value::Object(map) => {
            let (code, detail) = map.iter().next()?;
            let status = detail.get("http_status_code").and_then(Value::as_u64);
            (code.as_str(), status)
        }
        _ => return None,
    };
    let label = match code {
        "usage_limit_exceeded" => USAGE_LIMIT,
        "rate_limit_exceeded" => RATE_LIMITED,
        "response_too_many_failed_attempts" if status == Some(429) => RATE_LIMITED,
        "context_window_exceeded" => "context full",
        "session_budget_exceeded" => "budget spent",
        "server_overloaded" | "internal_server_error" => "server error",
        "unauthorized" => "auth failed",
        "bad_request" => "invalid request",
        "http_connection_failed"
        | "response_stream_connection_failed"
        | "response_stream_disconnected" => "connection failed",
        "response_too_many_failed_attempts" => "api error",
        "cyber_policy" | "bio_policy" | "misalignment_policy_violation" => "policy refusal",
        "sandbox_error" => "sandbox error",
        "other" => return None,
        other => other,
    };
    Some(label.to_string())
}

/// When a usage limit lifts, read from Codex's error text
///
/// See [`retry_at_in`]; Codex prints the time on its own clock, which for a
/// process Panoptes spawned is Panoptes' own.
fn retry_at(message: &str, failed_at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    retry_at_in(message, failed_at, &chrono::Local)
}

/// When a usage limit lifts, read from Codex's error text in a given zone
///
/// Codex ends a usage-limit message with "Try again at 3:37 PM." when the
/// reset is today, and "...or try again at Nov 11th, 2026 3:37 PM." when it is
/// not (`format_retry_timestamp`, `protocol/src/error.rs`), on its local clock.
/// `failed_at` is when it said so, which fixes the date of the first form.
///
/// Codex drops the seconds, so the minute after is the first one certainly
/// past the reset. At a fall-back the repeated hour is taken at its later
/// instant, for the same reason: a time too early is the one mistake that
/// matters. `None` when there is no such text, or it names a time already past.
fn retry_at_in<Tz: TimeZone>(
    message: &str,
    failed_at: DateTime<Utc>,
    tz: &Tz,
) -> Option<DateTime<Utc>> {
    const MARKER: &str = "ry again at ";
    let (at, _) = message
        .match_indices(MARKER)
        .find(|(at, _)| message[..*at].ends_with(['T', 't']))?;
    let rest = &message[at + MARKER.len()..];
    let stamp = &rest[..rest.find('.')?];
    let tokens: Vec<&str> = stamp.split_whitespace().collect();

    let (date, time, meridiem) = match tokens.as_slice() {
        [time, meridiem] => (failed_at.with_timezone(tz).date_naive(), *time, *meridiem),
        [month, day, year, time, meridiem] => {
            let month = MONTHS.iter().position(|m| m == month)? as u32 + 1;
            let day = day.strip_suffix(',')?;
            let day: u32 = ["st", "nd", "rd", "th"]
                .iter()
                .find_map(|suffix| day.strip_suffix(suffix))?
                .parse()
                .ok()?;
            let date = NaiveDate::from_ymd_opt(year.parse().ok()?, month, day)?;
            (date, *time, *meridiem)
        }
        _ => return None,
    };
    let (hour, minute) = time.split_once(':')?;
    if minute.len() != 2 {
        return None;
    }
    let (hour, minute): (u32, u32) = (hour.parse().ok()?, minute.parse().ok()?);
    if !(1..=12).contains(&hour) {
        return None;
    }
    let hour = hour % 12
        + match meridiem {
            "AM" => 0,
            "PM" => 12,
            _ => return None,
        };
    let local = date.and_hms_opt(hour, minute, 0)?;
    let reset = match tz.from_local_datetime(&local) {
        MappedLocalTime::Single(t) => t,
        MappedLocalTime::Ambiguous(_, later) => later,
        MappedLocalTime::None => return None,
    };
    let reset = reset.with_timezone(&Utc) + chrono::TimeDelta::minutes(1);
    (reset > failed_at).then_some(reset)
}

/// Month names as Codex's `%b` writes them
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// What a rollout's recent rate-limit figures say about the turn under way
///
/// A Codex turn refused on a usage limit leaves two records: a `token_count`
/// carrying the figures the refusal came with, then the `task_complete`
/// carrying the error. Each line parses on its own, so the window and its
/// reset, which only the first has, are joined to the failure here by a tailer
/// that sees both in order.
///
/// This is also what catches a limit a Codex too old to write the error leaves
/// behind: a turn that ends with no message, after figures showing a limit used
/// up or a refusal named, failed on that limit rather than finished.
#[derive(Debug, Default)]
pub struct RolloutLimits {
    /// The rate-limit figures as of the latest record
    usage: UsageSnapshot,
    /// Whether a record since the turn began showed the allowance refusing
    /// turns (see [`UsageSnapshot::is_limited`])
    hit_this_turn: bool,
}

impl RolloutLimits {
    /// Start from figures already known, e.g. a tailer's seed
    pub fn seeded(usage: Option<UsageSnapshot>) -> Self {
        Self {
            usage: usage.unwrap_or_default(),
            hit_this_turn: false,
        }
    }

    /// Note what an event says about the limits, and complete a failure with
    /// the window it was refused on
    pub fn observe(&mut self, event: AgentEvent) -> AgentEvent {
        match event {
            AgentEvent::TurnStarted { .. } => {
                self.hit_this_turn = false;
                event
            }
            AgentEvent::Usage(ref usage) => {
                if usage.carries_limits() {
                    self.usage.merge(usage.clone());
                    self.hit_this_turn |= self.usage.is_limited();
                }
                event
            }
            AgentEvent::RolloutTurnFailed {
                reason,
                limit,
                turn_id,
                inferred,
                open_to_usage_limit,
            } => {
                let reason = if open_to_usage_limit && self.hit_this_turn {
                    Some(USAGE_LIMIT.to_string())
                } else {
                    reason
                };
                let limit = if reason.as_deref() == Some(USAGE_LIMIT) {
                    self.refused_on(limit)
                } else {
                    limit
                };
                AgentEvent::RolloutTurnFailed {
                    reason,
                    limit,
                    turn_id,
                    inferred,
                    // Settled now, one way or the other
                    open_to_usage_limit: false,
                }
            }
            // Only a Codex too old to write the error ends a refused turn like
            // this, and one that old has no hooks; `inferred` keeps it from
            // overruling the hooks of one that does
            AgentEvent::TurnCompleted { last_message: None } if self.hit_this_turn => {
                AgentEvent::RolloutTurnFailed {
                    reason: Some(USAGE_LIMIT.to_string()),
                    limit: self.refused_on(None),
                    turn_id: None,
                    inferred: true,
                    open_to_usage_limit: false,
                }
            }
            event => event,
        }
    }

    /// The limit a turn was refused on: the figures' window when they showed
    /// one hit this turn, with the text's reset if they had none
    fn refused_on(&self, from_text: Option<UsageLimit>) -> Option<UsageLimit> {
        let from_figures = self
            .hit_this_turn
            .then(|| self.usage.exhausted_limit())
            .flatten();
        match (from_figures, from_text) {
            (Some(mut limit), text) => {
                if limit.resets_at.is_none() {
                    limit.resets_at = text.and_then(|t| t.resets_at);
                }
                Some(limit)
            }
            (None, text) => text,
        }
    }
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
    fn test_token_count_reads_individual_limit() {
        // Codex's `SpendControlLimitSnapshot`, which app-server calls
        // `individualLimit`, is `individual_limit` in the rollout
        let line = r#"{"type":"event_msg","payload":{"type":"token_count","info":null,
            "rate_limits":{"limit_id":"codex",
              "primary":{"used_percent":12.0,"window_minutes":300,"resets_at":1794411420},
              "individual_limit":{"limit":"50","used":"35","remaining_percent":30,"resets_at":1794646800},
              "plan_type":"business","rate_limit_reached_type":null}}}"#;

        let Some(AgentEvent::Usage(usage)) = parse_line(line) else {
            panic!("expected a usage event");
        };
        assert_eq!(
            usage.individual,
            Some(RateLimitWindow {
                used_percent: 70.0,
                window_minutes: None,
                resets_at: DateTime::from_timestamp(1_794_646_800, 0),
            })
        );
        // The credit allowance is the binding constraint here
        assert_eq!(usage.summary().as_deref(), Some("credits 70%"));
    }

    /// Feed lines through a fresh tracker, as a tailer would
    fn observed_raw(lines: &[&str]) -> Vec<AgentEvent> {
        let mut limits = RolloutLimits::default();
        lines
            .iter()
            .filter_map(|line| parse_line(line))
            .map(|event| limits.observe(event))
            .collect()
    }

    /// [`observed_raw`], with each failure read as what it means once
    /// admitted, for tests about the reason and limit alone
    fn observed(lines: &[&str]) -> Vec<AgentEvent> {
        observed_raw(lines)
            .into_iter()
            .map(|event| match event {
                AgentEvent::RolloutTurnFailed {
                    reason,
                    limit,
                    open_to_usage_limit,
                    ..
                } => {
                    assert!(!open_to_usage_limit, "the tracker settles it");
                    AgentEvent::TurnFailed { reason, limit }
                }
                event => event,
            })
            .collect()
    }

    fn token_count(rate_limits: &str) -> String {
        format!(
            r#"{{"type":"event_msg","payload":{{"type":"token_count","info":null,"rate_limits":{rate_limits}}}}}"#
        )
    }

    fn failed_task(error: &str) -> String {
        format!(
            r#"{{"timestamp":"2026-11-11T14:00:00.000Z","type":"event_msg","payload":{{"type":"task_complete","last_agent_message":null,"error":{error},"completed_at":1794405600}}}}"#
        )
    }

    const TASK_STARTED: &str = r#"{"type":"event_msg","payload":{"type":"task_started"}}"#;

    #[test]
    fn test_usage_limit_failure_names_the_latest_exhausted_window() {
        // Both windows used up: the turn can only run once both have lifted,
        // so the week's later reset is the one that matters
        let fixture = include_str!("fixtures/usage_limit_rollout.jsonl");
        let events = observed(&fixture.lines().collect::<Vec<_>>());

        assert_eq!(
            events.last(),
            Some(&AgentEvent::TurnFailed {
                reason: Some("usage limit".to_string()),
                limit: Some(UsageLimit {
                    window: "wk".to_string(),
                    resets_at: DateTime::from_timestamp(1_794_646_800, 0),
                    using_overage: false,
                }),
            })
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AgentEvent::TurnCompleted { .. })),
            "a refused turn must not read as finished"
        );
    }

    #[test]
    fn test_usage_limit_failure_with_one_exhausted_window() {
        let events = observed(&[
            TASK_STARTED,
            &token_count(
                r#"{"limit_id":"codex",
                  "primary":{"used_percent":100.0,"window_minutes":300,"resets_at":1794411420},
                  "secondary":{"used_percent":64.0,"window_minutes":10080,"resets_at":1794646800},
                  "plan_type":"plus","rate_limit_reached_type":"rate_limit_reached"}"#,
            ),
            &failed_task(
                r#"{"message":"You’ve hit your usage limit. Try again at 3:37 PM.","codex_error_info":"usage_limit_exceeded"}"#,
            ),
        ]);
        let Some(AgentEvent::TurnFailed { reason, limit }) = events.last() else {
            panic!("expected a failed turn, got {events:?}");
        };
        assert_eq!(reason.as_deref(), Some("usage limit"));
        let limit = limit.as_ref().expect("the window it was refused on");
        // The figures' own reset, not the text's: they are exact to the second
        assert_eq!(limit.window, "5h");
        assert_eq!(limit.resets_at, DateTime::from_timestamp(1_794_411_420, 0));
    }

    #[test]
    fn test_usage_limit_without_figures_falls_back_to_the_text() {
        // A proxy can strip the rate-limit headers, leaving only the message.
        // The text is on the user's clock, so read back on the same clock.
        let failed_at = Utc.with_ymd_and_hms(2026, 11, 11, 14, 0, 0).unwrap();
        let failed_local = failed_at.with_timezone(&chrono::Local);
        let reset_local = failed_local + chrono::TimeDelta::hours(2);
        // As Codex writes it: the dated form once the reset is another day,
        // which it is in some zones
        let text = if reset_local.date_naive() == failed_local.date_naive() {
            reset_local.format("%-I:%M %p").to_string()
        } else {
            use chrono::Datelike;
            let suffix = match reset_local.day() {
                1 | 21 | 31 => "st",
                2 | 22 => "nd",
                3 | 23 => "rd",
                _ => "th",
            };
            reset_local
                .format(&format!("%b %-d{suffix}, %Y %-I:%M %p"))
                .to_string()
        };
        let events = observed(&[
            TASK_STARTED,
            &failed_task(&format!(
                r#"{{"message":"You’ve hit your usage limit. Try again at {text}.","codex_error_info":"usage_limit_exceeded"}}"#
            )),
        ]);
        let Some(AgentEvent::TurnFailed { reason, limit }) = events.last() else {
            panic!("expected a failed turn, got {events:?}");
        };
        assert_eq!(reason.as_deref(), Some("usage limit"));
        let expected = failed_at + chrono::TimeDelta::hours(2) + chrono::TimeDelta::minutes(1);
        // The text names no window, only when
        assert_eq!(
            limit.as_ref(),
            Some(&UsageLimit {
                window: String::new(),
                resets_at: Some(expected),
                using_overage: false,
            })
        );
    }

    #[test]
    fn test_retry_text_parses_both_forms() {
        // 14:00 UTC is 15:00 in Zagreb (UTC+1 in November)
        let zagreb = chrono::FixedOffset::east_opt(3_600).unwrap();
        let failed_at = Utc.with_ymd_and_hms(2026, 11, 11, 14, 0, 0).unwrap();
        let utc = |d, h, m| Utc.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap();

        // Today's form, plus the minute Codex's rounding may have dropped
        assert_eq!(
            retry_at_in(
                "You’ve hit your usage limit. Try again at 3:37 PM.",
                failed_at,
                &zagreb
            ),
            Some(utc(11, 14, 38))
        );
        // The dated form, lower-cased mid-sentence
        assert_eq!(
            retry_at_in(
                "You’ve hit your usage limit. Upgrade to Plus to continue using Codex (https://chatgpt.com/explore/plus), or try again at Nov 13th, 2026 9:05 AM.",
                failed_at,
                &zagreb
            ),
            Some(utc(13, 8, 6))
        );
        // Every ordinal suffix, and midnight/noon on a 12-hour clock
        assert_eq!(
            retry_at_in("Try again at Dec 1st, 2026 12:00 AM.", failed_at, &Utc),
            Some(Utc.with_ymd_and_hms(2026, 12, 1, 0, 1, 0).unwrap())
        );
        assert_eq!(
            retry_at_in("Try again at Nov 22nd, 2026 12:30 PM.", failed_at, &Utc),
            Some(utc(22, 12, 31))
        );
        assert_eq!(
            retry_at_in("Try again at Nov 23rd, 2026 1:00 AM.", failed_at, &Utc),
            Some(utc(23, 1, 1))
        );

        // A time already past says nothing, and neither does anything else
        assert_eq!(
            retry_at_in("Try again at 2:00 PM.", failed_at, &zagreb),
            None
        );
        for text in [
            "You’ve hit your usage limit. Try again later.",
            "Try again at 13:37 PM.",
            "Try again at 3:7 PM.",
            "Try again at 3:37 PM",
            "Try again at Foo 11th, 2026 3:37 PM.",
            "Try again at Nov 11, 2026 3:37 PM.",
            "Pray again at 3:37 PM.",
        ] {
            assert_eq!(retry_at_in(text, failed_at, &zagreb), None, "for {text:?}");
        }
    }

    /// US Eastern around the 2026 fall-back (Nov 1, 06:00 UTC), without a
    /// time zone database: 01:00-02:00 local happens twice
    #[derive(Debug, Clone, Copy)]
    struct FallBack;

    impl FallBack {
        const EDT: i32 = -4 * 3_600;
        const EST: i32 = -5 * 3_600;

        fn switch() -> chrono::NaiveDateTime {
            NaiveDate::from_ymd_opt(2026, 11, 1)
                .unwrap()
                .and_hms_opt(6, 0, 0)
                .unwrap()
        }
    }

    impl TimeZone for FallBack {
        type Offset = chrono::FixedOffset;

        fn from_offset(_: &chrono::FixedOffset) -> Self {
            FallBack
        }

        fn offset_from_local_date(&self, local: &NaiveDate) -> MappedLocalTime<Self::Offset> {
            self.offset_from_local_datetime(&local.and_hms_opt(0, 0, 0).unwrap())
        }

        fn offset_from_local_datetime(
            &self,
            local: &chrono::NaiveDateTime,
        ) -> MappedLocalTime<Self::Offset> {
            let edt = chrono::FixedOffset::east_opt(Self::EDT).unwrap();
            let est = chrono::FixedOffset::east_opt(Self::EST).unwrap();
            let as_edt = *local - chrono::TimeDelta::seconds(Self::EDT.into()) < Self::switch();
            let as_est = *local - chrono::TimeDelta::seconds(Self::EST.into()) >= Self::switch();
            match (as_edt, as_est) {
                (true, true) => MappedLocalTime::Ambiguous(edt, est),
                (true, false) => MappedLocalTime::Single(edt),
                (false, true) => MappedLocalTime::Single(est),
                (false, false) => MappedLocalTime::None,
            }
        }

        fn offset_from_utc_date(&self, utc: &NaiveDate) -> Self::Offset {
            self.offset_from_utc_datetime(&utc.and_hms_opt(0, 0, 0).unwrap())
        }

        fn offset_from_utc_datetime(&self, utc: &chrono::NaiveDateTime) -> Self::Offset {
            let secs = if *utc < Self::switch() {
                Self::EDT
            } else {
                Self::EST
            };
            chrono::FixedOffset::east_opt(secs).unwrap()
        }
    }

    #[test]
    fn test_retry_text_takes_the_later_of_a_repeated_hour() {
        // 00:30 EDT on the night the clocks go back
        let failed_at = Utc.with_ymd_and_hms(2026, 11, 1, 4, 30, 0).unwrap();
        // 1:30 AM happens at 05:30 UTC (EDT) and again at 06:30 UTC (EST);
        // the later one is never too early
        assert_eq!(
            retry_at_in("Try again at 1:30 AM.", failed_at, &FallBack),
            Some(Utc.with_ymd_and_hms(2026, 11, 1, 6, 31, 0).unwrap())
        );
        // Either side of the repeat there is only one answer
        assert_eq!(
            retry_at_in("Try again at 12:45 AM.", failed_at, &FallBack),
            Some(Utc.with_ymd_and_hms(2026, 11, 1, 4, 46, 0).unwrap())
        );
        assert_eq!(
            retry_at_in("Try again at 2:30 AM.", failed_at, &FallBack),
            Some(Utc.with_ymd_and_hms(2026, 11, 1, 7, 31, 0).unwrap())
        );
    }

    #[test]
    fn test_failure_reason_codes() {
        let reason = |info: &str| failure_reason(Some(&serde_json::from_str(info).unwrap()));
        assert_eq!(
            reason(r#""usage_limit_exceeded""#).as_deref(),
            Some("usage limit")
        );
        // A throughput limit, which Codex retries, is not a spent allowance
        assert_eq!(
            reason(r#""rate_limit_exceeded""#).as_deref(),
            Some("rate limited")
        );
        // Nor are retries given up on a 429, on the code alone; on anything
        // else they are a plain API error
        assert_eq!(
            reason(r#"{"response_too_many_failed_attempts":{"http_status_code":429}}"#).as_deref(),
            Some("rate limited")
        );
        assert_eq!(
            reason(r#"{"response_too_many_failed_attempts":{"http_status_code":502}}"#).as_deref(),
            Some("api error")
        );
        assert_eq!(
            reason(r#"{"http_connection_failed":{"http_status_code":null}}"#).as_deref(),
            Some("connection failed")
        );
        assert_eq!(reason(r#""unauthorized""#).as_deref(), Some("auth failed"));
        assert_eq!(
            reason(r#""context_window_exceeded""#).as_deref(),
            Some("context full")
        );
        // `other` says nothing; a code from a newer Codex is kept as it is
        assert_eq!(reason(r#""other""#), None);
        assert_eq!(
            reason(r#""brand_new_code""#).as_deref(),
            Some("brand_new_code")
        );
        assert_eq!(failure_reason(None), None);
    }

    #[test]
    fn test_other_failure_is_a_usage_limit_only_with_evidence() {
        let other = r#"{"message":"Something went wrong.","codex_error_info":"other"}"#;

        // Nothing points at a limit: a bare failure, which still ends the turn
        assert_eq!(
            observed(&[TASK_STARTED, &failed_task(other)]).last(),
            Some(&AgentEvent::TurnFailed {
                reason: None,
                limit: None
            })
        );

        // The figures just before it say the limit was hit
        let events = observed(&[
            TASK_STARTED,
            &token_count(
                r#"{"limit_id":"codex","primary":{"used_percent":100.0,"resets_at":1794411420},
                  "plan_type":"free","rate_limit_reached_type":null}"#,
            ),
            &failed_task(other),
        ]);
        assert_eq!(
            events.last(),
            Some(&AgentEvent::TurnFailed {
                reason: Some("usage limit".to_string()),
                // No `window_minutes`: on the Free plan the primary is monthly
                limit: Some(UsageLimit {
                    window: "mo".to_string(),
                    resets_at: DateTime::from_timestamp(1_794_411_420, 0),
                    using_overage: false,
                }),
            })
        );

        // A different failure is not relabelled by a limit that happens to be hit
        let events = observed(&[
            TASK_STARTED,
            &token_count(r#"{"limit_id":"codex","primary":{"used_percent":100.0}}"#),
            &failed_task(r#"{"message":"Unauthorized","codex_error_info":"unauthorized"}"#),
        ]);
        assert_eq!(
            events.last(),
            Some(&AgentEvent::TurnFailed {
                reason: Some("auth failed".to_string()),
                limit: None
            })
        );
    }

    #[test]
    fn test_limit_hit_turn_without_an_error_record_still_fails() {
        // A Codex too old to write the error on `task_complete`
        let limited = token_count(
            r#"{"limit_id":"codex",
              "primary":{"used_percent":40.0,"window_minutes":300,"resets_at":1794411420},
              "credits":{"has_credits":false,"unlimited":false,"balance":"0"},
              "individual_limit":{"limit":"50","used":"50","remaining_percent":0,"resets_at":1794646800},
              "plan_type":"business","rate_limit_reached_type":"workspace_member_credits_depleted"}"#,
        );
        let bare_end =
            r#"{"type":"event_msg","payload":{"type":"task_complete","last_agent_message":null}}"#;
        let events = observed_raw(&[TASK_STARTED, &limited, bare_end]);
        assert_eq!(
            events.last(),
            Some(&AgentEvent::RolloutTurnFailed {
                reason: Some("usage limit".to_string()),
                limit: Some(UsageLimit {
                    window: "credits".to_string(),
                    resets_at: DateTime::from_timestamp(1_794_646_800, 0),
                    using_overage: false,
                }),
                turn_id: None,
                // Marked, so live hooks never let it through
                inferred: true,
                open_to_usage_limit: false,
            })
        );

        // A turn that says something finished, whatever the figures say
        let spoke = r#"{"type":"event_msg","payload":{"type":"task_complete","last_agent_message":"Done."}}"#;
        assert!(matches!(
            observed(&[TASK_STARTED, &limited, spoke]).last(),
            Some(AgentEvent::TurnCompleted { .. })
        ));

        // A limit hit in an earlier turn is not this turn's
        let lifted = token_count(
            r#"{"limit_id":"codex","primary":{"used_percent":41.0},
              "individual_limit":{"limit":"100","used":"50","remaining_percent":50,"resets_at":1794646800},
              "rate_limit_reached_type":null}"#,
        );
        assert!(matches!(
            observed(&[TASK_STARTED, &limited, TASK_STARTED, &lifted, bare_end]).last(),
            Some(AgentEvent::TurnCompleted { .. })
        ));
    }

    #[test]
    fn test_full_window_with_credits_is_not_a_refusal() {
        // Plus and Pro carry on past 100% on purchased credits
        let on_credits = token_count(
            r#"{"limit_id":"codex",
              "primary":{"used_percent":100.0,"window_minutes":300,"resets_at":1794411420},
              "credits":{"has_credits":true,"unlimited":false,"balance":"12.50"},
              "plan_type":"pro","rate_limit_reached_type":null}"#,
        );
        // An `after_agent` hook's abort, say: `other`, and nothing to do with
        // the allowance
        let other = failed_task(
            r#"{"message":"after_agent hook 'x' failed and aborted turn completion","codex_error_info":"other"}"#,
        );
        assert_eq!(
            observed(&[TASK_STARTED, &on_credits, &other]).last(),
            Some(&AgentEvent::TurnFailed {
                reason: None,
                limit: None
            })
        );
        // ...and a quiet turn end after it is just an end
        let bare_end =
            r#"{"type":"event_msg","payload":{"type":"task_complete","last_agent_message":null}}"#;
        assert!(matches!(
            observed(&[TASK_STARTED, &on_credits, bare_end]).last(),
            Some(AgentEvent::TurnCompleted { .. })
        ));

        // Unlimited counts the same; no credits at all does not
        let unlimited = on_credits.replace(r#""unlimited":false"#, r#""unlimited":true"#);
        let unlimited = unlimited.replace(r#""has_credits":true"#, r#""has_credits":false"#);
        assert!(matches!(
            observed(&[TASK_STARTED, &unlimited, bare_end]).last(),
            Some(AgentEvent::TurnCompleted { .. })
        ));
        let spent = unlimited.replace(r#""unlimited":true"#, r#""unlimited":false"#);
        assert!(matches!(
            observed(&[TASK_STARTED, &spent, &other]).last(),
            Some(AgentEvent::TurnFailed { reason: Some(r), .. }) if r == "usage limit"
        ));
    }

    #[test]
    fn test_retries_on_a_429_are_a_usage_limit_only_when_the_figures_say_so() {
        let given_up = failed_task(
            r#"{"message":"exceeded retry limit, last status: 429 Too Many Requests","codex_error_info":{"response_too_many_failed_attempts":{"http_status_code":429}}}"#,
        );
        let roomy = token_count(
            r#"{"limit_id":"codex","primary":{"used_percent":30.0,"window_minutes":300}}"#,
        );
        assert_eq!(
            observed(&[TASK_STARTED, &roomy, &given_up]).last(),
            Some(&AgentEvent::TurnFailed {
                reason: Some("rate limited".to_string()),
                limit: None
            })
        );

        let spent = token_count(
            r#"{"limit_id":"codex","primary":{"used_percent":100.0,"window_minutes":300,"resets_at":1794411420}}"#,
        );
        assert_eq!(
            observed(&[TASK_STARTED, &spent, &given_up]).last(),
            Some(&AgentEvent::TurnFailed {
                reason: Some("usage limit".to_string()),
                limit: Some(UsageLimit {
                    window: "5h".to_string(),
                    resets_at: DateTime::from_timestamp(1_794_411_420, 0),
                    using_overage: false,
                }),
            })
        );

        // `rate_limit_exceeded` is a throughput limit whatever the figures say
        let throughput = failed_task(
            r#"{"message":"Rate limit reached","codex_error_info":"rate_limit_exceeded"}"#,
        );
        assert!(matches!(
            observed(&[TASK_STARTED, &spent, &throughput]).last(),
            Some(AgentEvent::TurnFailed { reason: Some(r), .. }) if r == "rate limited"
        ));
    }

    #[test]
    fn test_failure_carries_its_turn_id() {
        let fixture = include_str!("fixtures/usage_limit_rollout.jsonl");
        let events = observed_raw(&fixture.lines().collect::<Vec<_>>());
        assert!(matches!(
            events.last(),
            Some(AgentEvent::RolloutTurnFailed { turn_id: Some(id), inferred: false, .. })
                if id == "turn-1"
        ));
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

    /// The fixture as Codex compresses it (`zstd -19` of the `.jsonl` beside it)
    const COMPRESSED_FIXTURE: &[u8] = include_bytes!("fixtures/compressed_rollout.jsonl.zst");

    #[test]
    fn test_compressed_fixture_decodes_to_its_source() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("rollout-c.jsonl.zst");
        std::fs::write(&path, COMPRESSED_FIXTURE).unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut rollout_file::open(&path).unwrap(), &mut text).unwrap();
        assert_eq!(text, include_str!("fixtures/compressed_rollout.jsonl"));
    }

    #[test]
    fn test_rollout_files_lists_compressed_rollouts() {
        let dir = tempfile::TempDir::new().unwrap();
        let plain = write_rollout_file(dir.path(), "rollout-a.jsonl", "{}");
        let compressed = plain.with_file_name("rollout-b.jsonl.zst");
        std::fs::write(&compressed, COMPRESSED_FIXTURE).unwrap();

        let mut found = rollout_files(dir.path());
        found.sort();
        assert_eq!(found, vec![plain, compressed]);
    }

    #[test]
    fn test_rollout_files_prefers_the_plain_file_of_a_pair() {
        let dir = tempfile::TempDir::new().unwrap();
        let plain = write_rollout_file(dir.path(), "rollout-t-id.jsonl", "{}");
        std::fs::write(
            plain.with_file_name("rollout-t-id.jsonl.zst"),
            COMPRESSED_FIXTURE,
        )
        .unwrap();

        assert_eq!(rollout_files(dir.path()), vec![plain]);
    }

    #[test]
    fn test_read_session_meta_of_a_compressed_rollout() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("rollout-c.jsonl.zst");
        std::fs::write(&path, COMPRESSED_FIXTURE).unwrap();

        let meta = read_session_meta(&path).expect("decoded session_meta");
        assert_eq!(meta.id, "00000000-0000-7000-8000-00000000c0de");
        assert_eq!(meta.cwd, Some(PathBuf::from("/Users/redacted/work")));
        assert_eq!(meta.kind, RolloutKind::Session);
        assert!(meta.created_at.is_some());
    }

    #[test]
    fn test_read_session_meta_of_a_corrupt_compressed_rollout_is_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let garbage = dir.path().join("rollout-bad.jsonl.zst");
        std::fs::write(&garbage, b"not zstd at all\n").unwrap();
        assert!(read_session_meta(&garbage).is_none());

        // A real frame cut short: the header decodes, the rest does not
        let truncated = dir.path().join("rollout-cut.jsonl.zst");
        std::fs::write(
            &truncated,
            &COMPRESSED_FIXTURE[..COMPRESSED_FIXTURE.len() / 2],
        )
        .unwrap();
        assert!(read_session_meta(&truncated).is_none());
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
