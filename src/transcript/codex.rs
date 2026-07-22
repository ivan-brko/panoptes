//! Codex rollout parsing
//!
//! Codex writes every event of a conversation to
//! `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl` as it happens -
//! measured flush latency is under 50ms, comfortably fast enough to drive a
//! live display.
//!
//! This is the only usable channel for Codex state. Its `notify` hook fires
//! once per turn and cannot be extended: the script is forbidden from reading
//! stdin, because a blocking read stalls Codex's output pipeline and drops
//! typed characters (see `agent/codex.rs`).
//!
//! Two shapes matter. `event_msg` records describe what the session is doing;
//! `response_item` records describe what the model emitted. Tool *starts* only
//! exist in the second: verified across 200 real rollouts, `event_msg` contains
//! no `*_begin` events at all, so `function_call` / `function_call_output` is
//! the begin/end pair.

use serde_json::Value;

use crate::agent::events::{AgentEvent, UsageSnapshot};

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

    let primary = payload.get("rate_limits").and_then(|r| r.get("primary"));

    UsageSnapshot {
        total_tokens: total,
        context_window: window,
        model: info
            .and_then(|i| i.get("model"))
            .and_then(Value::as_str)
            .map(str::to_string),
        rate_limit_used_percent: primary
            .and_then(|p| p.get("used_percent"))
            .and_then(Value::as_f64),
        rate_limit_resets_at: primary
            .and_then(|p| p.get("resets_at"))
            .and_then(Value::as_str)
            .map(str::to_string),
        plan: primary
            .and_then(|p| p.get("plan_type"))
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// Whether a rollout file belongs to a subagent rather than a real session
///
/// Codex subagents get their own rollout files, which look enough like a
/// session's own to be claimed by mistake - a real example on this machine is a
/// subagent rollout whose `cwd` is a Panoptes worktree, with its own fresh
/// start timestamp.
///
/// Beware the field names: on a subagent rollout, `payload.id` is the
/// subagent's own ID while `payload.session_id` is its *parent's*, and a second
/// `session_meta` record follows carrying the parent's metadata.
pub fn is_subagent_meta(payload: &Value) -> bool {
    payload.get("forked_from_id").is_some_and(|v| !v.is_null())
        || payload
            .get("source")
            .and_then(|s| s.get("subagent"))
            .is_some()
}

/// The conversation this rollout was forked from, if it is a subagent's
pub fn parent_conversation_id(payload: &Value) -> Option<String> {
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
    fn test_token_count() {
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
        assert_eq!(usage.rate_limit_used_percent, Some(12.5));
        assert_eq!(usage.plan.as_deref(), Some("pro"));
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

    #[test]
    fn test_subagent_detection() {
        let forked = serde_json::json!({"id": "child", "forked_from_id": "parent"});
        assert!(is_subagent_meta(&forked));
        assert_eq!(parent_conversation_id(&forked).as_deref(), Some("parent"));

        let spawned = serde_json::json!({
            "id": "child",
            "source": {"subagent": {"thread_spawn": {"parent_thread_id": "parent"}}}
        });
        assert!(is_subagent_meta(&spawned));
        assert_eq!(parent_conversation_id(&spawned).as_deref(), Some("parent"));

        // A normal session, including one that has been resumed many times
        let plain = serde_json::json!({"id": "own", "cwd": "/tmp"});
        assert!(!is_subagent_meta(&plain));
        assert_eq!(parent_conversation_id(&plain), None);

        // An explicit null must not read as "forked"
        let null_fork = serde_json::json!({"id": "own", "forked_from_id": null});
        assert!(!is_subagent_meta(&null_fork));
    }
}
