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

/// Context window Claude Code gives each model by default, by id prefix
///
/// Source: the model catalogue compiled into Claude Code 2.1.280 (each entry's
/// `context:{window:..}`), read 2026-09-23. A new model is one row here.
///
/// The first matching row wins, so a version must come before any shorter
/// prefix it extends - `claude-opus-4-8` before `claude-opus-4`. Rows for the
/// 1M models are what Claude Code runs on Anthropic's own API; it drops them to
/// 200k on most third-party providers, with `CLAUDE_CODE_DISABLE_1M_CONTEXT`,
/// or when the account cannot pay for long context. The transcript shows none
/// of that, which is why a reported window outranks this table - see
/// [`crate::agent::events::WindowSource`].
const CONTEXT_WINDOWS: &[(&str, u64)] = &[
    ("claude-fable-5", 1_000_000),
    ("claude-mythos-5", 1_000_000),
    ("claude-opus-5", 1_000_000),
    ("claude-opus-4-8", 1_000_000),
    ("claude-opus-4-7", 1_000_000),
    ("claude-opus-4", 200_000),
    ("claude-sonnet-5", 1_000_000),
    ("claude-sonnet-4", 200_000),
    ("claude-haiku-4", 200_000),
    ("claude-3", 200_000),
];

/// Best-known context window for a Claude model
///
/// Claude does not publish this in the transcript, so it is looked up in
/// [`CONTEXT_WINDOWS`]. Returning `None` for an unrecognised model is
/// deliberate: the usage display then falls back to a raw token count rather
/// than showing a percentage of a number we invented.
fn context_window_for(model: &str) -> Option<u64> {
    let model = model.to_ascii_lowercase();
    if model.contains("[1m]") || model.contains("-1m") {
        return Some(1_000_000);
    }
    // Provider spellings wrap the same id: `us.anthropic.claude-opus-4-8-v1:0`
    let id = model
        .find("claude-")
        .map_or(model.as_str(), |at| &model[at..]);
    CONTEXT_WINDOWS
        .iter()
        .find(|(prefix, _)| {
            // A whole id segment, so `claude-opus-4` does not claim a future
            // `claude-opus-40`
            id.strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '@', '[']))
        })
        .map(|(_, window)| *window)
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
        assert_eq!(usage.context_window, Some(1_000_000));
        assert_eq!(usage.model.as_deref(), Some("claude-opus-4-8"));
        // Claude publishes no rate limits anywhere
        assert_eq!(usage.primary, None);
        assert_eq!(usage.secondary, None);
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
    fn test_context_window_for_current_models() {
        for (model, window) in [
            // Current models run at 1M natively, and the transcript logs them
            // without any suffix
            ("claude-opus-5-5", 1_000_000),
            ("claude-opus-5", 1_000_000),
            ("claude-opus-4-8", 1_000_000),
            ("claude-opus-4-7", 1_000_000),
            ("claude-sonnet-5", 1_000_000),
            ("claude-fable-5-1", 1_000_000),
            ("claude-fable-5", 1_000_000),
            ("claude-mythos-5-1", 1_000_000),
            // Older ones are 200k unless launched with `[1m]`
            ("claude-opus-4-6", 200_000),
            ("claude-opus-4-1-20250805", 200_000),
            ("claude-opus-4-20250514", 200_000),
            ("claude-sonnet-4-6", 200_000),
            ("claude-sonnet-4-5-20250929", 200_000),
            ("claude-haiku-4-5-20251001", 200_000),
            ("claude-3-7-sonnet-20250219", 200_000),
            ("claude-sonnet-4-6[1m]", 1_000_000),
            ("claude-opus-4-6[1M]", 1_000_000),
            // Provider spellings of the same ids
            ("us.anthropic.claude-opus-4-8-v1:0", 1_000_000),
            ("claude-opus-4-5@20251101", 200_000),
        ] {
            assert_eq!(context_window_for(model), Some(window), "for {model:?}");
        }
    }

    #[test]
    fn test_context_window_unknown_family_is_none() {
        for model in [
            "some-future-model",
            "<synthetic>",
            "gpt-5-codex",
            // A version the table does not list is not guessed at either
            "claude-opus-40",
            "claude-opus-6",
            "",
        ] {
            assert_eq!(context_window_for(model), None, "for {model:?}");
        }
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
