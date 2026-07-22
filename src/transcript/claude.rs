//! Claude Code transcript parsing
//!
//! Claude writes `$CLAUDE_CONFIG_DIR/projects/<cwd-slug>/<session-uuid>.jsonl`
//! as the conversation happens.
//!
//! **This tailer contributes usage figures only, never state.** Claude's hooks
//! already report state, they arrive sooner, and two producers writing the same
//! field would fight over it. The transcript is read for the one thing hooks do
//! not carry: how full the context window is and which model is answering.
//!
//! There is no rate-limit data anywhere in a Claude transcript, so those fields
//! stay empty for Claude sessions rather than being guessed at.

use serde_json::Value;

use crate::agent::events::{AgentEvent, UsageSnapshot};

/// Translate one transcript line into a usage event
///
/// Returns `None` for everything that is not an assistant message carrying
/// usage - which is most of the file. Never fails: the transcript belongs to
/// another process and may be read mid-write.
pub fn parse_line(line: &str) -> Option<AgentEvent> {
    let record: Value = serde_json::from_str(line).ok()?;
    let message = record.get("message")?;

    // Only assistant messages carry usage. A user record has no counts, and a
    // summary or system record has no `message` at all.
    let usage = message.get("usage")?;

    // Claude reports the window in pieces. Everything the model can see next
    // turn is what has been read plus what has been written, so cache reads
    // count: they are context, they were merely cheap to send.
    let context_tokens: u64 = [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "output_tokens",
    ]
    .iter()
    .filter_map(|field| usage.get(*field).and_then(Value::as_u64))
    .sum();

    let snapshot = UsageSnapshot {
        total_tokens: (context_tokens > 0).then_some(context_tokens),
        // Claude never states its context window in the transcript, so it is
        // inferred from the model name rather than left unknown - a bare token
        // count is far less useful than a percentage.
        context_window: message
            .get("model")
            .and_then(Value::as_str)
            .and_then(context_window_for),
        model: message
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        ..Default::default()
    };

    if snapshot.is_empty() {
        return None;
    }
    Some(AgentEvent::Usage(snapshot))
}

/// Best-known context window for a Claude model
///
/// Claude does not publish this in the transcript. Returning `None` for an
/// unrecognised model is deliberate: the usage display then falls back to a raw
/// token count rather than showing a percentage of a number we invented.
fn context_window_for(model: &str) -> Option<u64> {
    if model.contains("[1m]") || model.contains("-1m") {
        return Some(1_000_000);
    }
    if model.contains("haiku") || model.contains("sonnet") || model.contains("opus") {
        return Some(200_000);
    }
    None
}

/// The directory name Claude derives from a working directory
///
/// Every character outside `[A-Za-z0-9]` becomes `-`, so
/// `/Users/ivan/Projects/panoptes` files under
/// `-Users-ivan-Projects-panoptes`.
pub fn project_slug(working_dir: &std::path::Path) -> String {
    working_dir
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Where Claude keeps a session's transcript
pub fn transcript_path(
    claude_config_dir: &std::path::Path,
    working_dir: &std::path::Path,
    conversation_id: &str,
) -> std::path::PathBuf {
    claude_config_dir
        .join("projects")
        .join(project_slug(working_dir))
        .join(format!("{}.jsonl", conversation_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_assistant_usage_is_summed_across_all_context_fields() {
        let line = r#"{"type":"assistant","message":{"model":"claude-opus-4-8",
            "usage":{"input_tokens":100,"cache_creation_input_tokens":2000,
                     "cache_read_input_tokens":48000,"output_tokens":400}}}"#;

        let Some(AgentEvent::Usage(usage)) = parse_line(line) else {
            panic!("expected usage");
        };
        // Cache reads are context: cheap to send, but still occupying the window
        assert_eq!(usage.total_tokens, Some(50_500));
        assert_eq!(usage.context_window, Some(200_000));
        assert_eq!(usage.model.as_deref(), Some("claude-opus-4-8"));
        // Claude publishes no rate limits anywhere
        assert_eq!(usage.rate_limit_used_percent, None);
        assert_eq!(usage.plan, None);
    }

    #[test]
    fn test_one_million_context_variant() {
        let line = r#"{"message":{"model":"claude-opus-4-8[1m]","usage":{"input_tokens":10}}}"#;
        let Some(AgentEvent::Usage(usage)) = parse_line(line) else {
            panic!("expected usage");
        };
        assert_eq!(usage.context_window, Some(1_000_000));
    }

    #[test]
    fn test_unknown_model_reports_tokens_without_inventing_a_window() {
        let line = r#"{"message":{"model":"some-future-model","usage":{"input_tokens":10}}}"#;
        let Some(AgentEvent::Usage(usage)) = parse_line(line) else {
            panic!("expected usage");
        };
        assert_eq!(usage.total_tokens, Some(10));
        assert_eq!(usage.context_window, None);
    }

    #[test]
    fn test_records_without_usage_are_ignored() {
        for line in [
            r#"{"type":"user","message":{"role":"user","content":"hi"}}"#,
            r#"{"type":"summary","summary":"a title"}"#,
            r#"{"type":"system","subtype":"turn_duration","durationMs":1200}"#,
            "",
            "not json",
            r#"{"message":{"usage":{}}}"#,
            r#"{"message":{"usage":{"input_tokens":0}}}"#,
        ] {
            assert_eq!(parse_line(line), None, "for {line:?}");
        }
    }

    #[test]
    fn test_project_slug_and_path() {
        assert_eq!(
            project_slug(Path::new("/Users/ivan/Projects/panoptes")),
            "-Users-ivan-Projects-panoptes"
        );
        // Dots and underscores are not alphanumeric either
        assert_eq!(project_slug(Path::new("/a/b_c.d")), "-a-b-c-d");

        assert_eq!(
            transcript_path(
                Path::new("/home/u/.claude"),
                Path::new("/Users/ivan/Projects/panoptes"),
                "abc-123"
            ),
            Path::new("/home/u/.claude/projects/-Users-ivan-Projects-panoptes/abc-123.jsonl")
        );
    }
}
