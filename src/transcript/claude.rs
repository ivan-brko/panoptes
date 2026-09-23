//! Claude Code transcript parsing
//!
//! Claude writes `$CLAUDE_CONFIG_DIR/projects/<cwd-slug>/<session-uuid>.jsonl`
//! as the conversation happens.
//!
//! **This tailer contributes usage figures and the conversation's title, plus
//! one state change: a failed turn.** Claude's hooks report everything else,
//! they arrive sooner, and two producers writing the same field would fight
//! over it. The transcript is read for what hooks do not carry: how full the
//! context window is, which model is answering, what Claude has named the
//! conversation (its `ai-title` records, revised as it goes) - and whether the
//! turn died on an API error.
//!
//! That last one breaks the rule deliberately. A turn that dies on an API
//! error (usage limit, expired login, overload) fires Claude's `StopFailure`
//! hook *instead of* `Stop`, and that hook is the primary report. The
//! transcript's record is the backstop for a hook that never arrived - a
//! dropped delivery, a Claude too old to send it - which would leave the
//! session in `Thinking` until the stall watchdog flagged it, unexplained. Both
//! read the code through [`failure_reason`], and the state machine ignores a
//! repeat, so the two cannot double-fire.
//!
//! Not every record describes the live conversation. Subagent (sidechain)
//! messages carry the subagent's model and context; meta records and
//! compaction summaries are injected, not exchanged; and a `<synthetic>`
//! assistant record is a placeholder Claude writes locally, with zeroed usage.
//! All of them are skipped, or the header would flash a subagent's model or a
//! near-empty context over the real session.
//!
//! There is no rate-limit data anywhere in a Claude transcript, so those fields
//! stay empty for Claude sessions rather than being guessed at.

use serde_json::Value;

use crate::agent::events::{AgentEvent, UsageSnapshot};

/// The model name Claude stamps on records it wrote itself, not the API
const SYNTHETIC_MODEL: &str = "<synthetic>";

/// Translate one transcript line into an event
///
/// Returns `None` for everything that is not an assistant message carrying
/// usage, a title, or a failed turn - which is most of the file. Never fails: the
/// transcript belongs to another process and may be read mid-write.
pub fn parse_line(line: &str) -> Option<AgentEvent> {
    let record: Value = serde_json::from_str(line).ok()?;

    // A subagent's records share this file but describe its own conversation:
    // its model, its context window, its failures. None of that is the
    // session's, so the check comes before anything is read out of it.
    if flag(&record, "isSidechain") {
        return None;
    }

    // Checked before the synthetic-model filter below, because a failed turn
    // is itself a synthetic record: Claude writes the error locally, with
    // `"model": "<synthetic>"` and zeroed usage.
    if flag(&record, "isApiErrorMessage") {
        let code = record.get("error").and_then(Value::as_str);
        tracing::debug!(code = ?code, "Claude transcript reports a failed turn");
        return Some(AgentEvent::TurnFailed {
            reason: failure_reason(code, error_text(&record)),
        });
    }

    if flag(&record, "isMeta") || flag(&record, "isCompactSummary") {
        return None;
    }

    // Claude's own name for the conversation, rewritten as it evolves - so
    // every one is passed on and the latest wins. After the sidechain check:
    // a subagent's title would name its task, not the session.
    if record.get("type").and_then(Value::as_str) == Some("ai-title") {
        let title = record.get("aiTitle").and_then(Value::as_str)?.trim();
        return (!title.is_empty()).then(|| AgentEvent::TitleChanged {
            title: title.to_string(),
        });
    }

    let message = record.get("message")?;
    let model = message.get("model").and_then(Value::as_str);
    if model == Some(SYNTHETIC_MODEL) {
        return None;
    }

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
        context_window: model.and_then(context_window_for),
        model: model.map(str::to_string),
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

/// Whether a top-level boolean flag is set on a record
fn flag(record: &Value, name: &str) -> bool {
    record.get(name).and_then(Value::as_bool) == Some(true)
}

/// The text Claude showed the user for a failed turn
fn error_text(record: &Value) -> Option<&str> {
    record
        .get("message")?
        .get("content")?
        .as_array()?
        .iter()
        .find_map(|block| block.get("text").and_then(Value::as_str))
}

/// A short human label for why a Claude turn failed
///
/// `code` is the record's `error` field - the same enum Claude's `StopFailure`
/// hook reports as its `error`, so the hook can share this. The codes, as of
/// Claude Code 2.1.280 (the `error` enum in the binary, which is also the
/// `StopFailure` matcher list; the ones marked * also appear in local
/// transcripts):
///
/// - `authentication_failed`*, `oauth_org_not_allowed`, `account_on_hold`,
///   `verification_required`, `cloud_credential_error`: the account cannot
///   make the request
/// - `rate_limit`*: a usage limit - the five-hour or weekly session limit
///   ("You've hit your session limit · resets 8:10pm"), or a 429
/// - `billing_error`: out of credits
/// - `overloaded`, `server_error`*: the API failed, including "Connection to
///   the API was lost" and "Your computer went to sleep mid-response"
/// - `invalid_request`*: the request itself was refused. Prompt-too-long is
///   one of these, told apart only by its text ("Prompt is too long"); there
///   is no `prompt_too_long` code
/// - `model_not_found`, `max_output_tokens`
/// - `unknown`*: everything else, including "API Error: Overloaded" from older
///   versions
///
/// An unrecognised code is kept as it is rather than dropped: a raw code says
/// more than a generic "turn failed". Returns `None` only when the record
/// carries no code at all.
pub fn failure_reason(code: Option<&str>, text: Option<&str>) -> Option<String> {
    let code = code?;
    let label = match code {
        "authentication_failed"
        | "oauth_org_not_allowed"
        | "verification_required"
        | "cloud_credential_error" => "auth failed",
        "account_on_hold" => "account on hold",
        "rate_limit" => "usage limit",
        "billing_error" => "out of credits",
        "overloaded" | "server_error" => "server error",
        "invalid_request" if text.is_some_and(|t| t.starts_with("Prompt is too long")) => {
            "prompt too long"
        }
        "invalid_request" => "invalid request",
        "model_not_found" => "model not found",
        "max_output_tokens" => "output limit",
        "unknown" => "api error",
        other => return Some(other.to_string()),
    };
    Some(label.to_string())
}

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

/// Whether a conversation's transcript exists under a config directory
///
/// `claude --resume` only finds a conversation in the config directory it runs
/// under, so this is exactly the question of whether a resume can succeed.
///
/// Looks where [`transcript_path`] says first, which is one stat. Failing
/// that, every project directory is tried: Claude derives the directory name
/// itself, and a mismatch with ours must not be reported as a lost
/// conversation, since the user would then be refused a resume that works.
/// Still one level of directory listing, never a recursive walk.
pub fn transcript_exists(
    claude_config_dir: &std::path::Path,
    working_dir: &std::path::Path,
    conversation_id: &str,
) -> bool {
    if transcript_path(claude_config_dir, working_dir, conversation_id).is_file() {
        return true;
    }
    let file_name = format!("{}.jsonl", conversation_id);
    let Ok(projects) = std::fs::read_dir(claude_config_dir.join("projects")) else {
        return false;
    };
    projects
        .flatten()
        .any(|project| project.path().join(&file_name).is_file())
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

    /// A real failed turn, copied from a local Claude Code 2.1.x transcript
    /// with its ids, paths and branch redacted. Note the `<synthetic>` model
    /// and zeroed usage: it must be read as a failure, not skipped as noise.
    const RATE_LIMIT_RECORD: &str = r#"{"parentUuid":"00000000-0000-0000-0000-000000000001","isSidechain":false,"type":"assistant","uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2026-08-28T17:59:57.050Z","message":{"diagnostics":null,"id":"00000000-0000-0000-0000-000000000003","container":null,"model":"<synthetic>","role":"assistant","stop_details":null,"stop_reason":"stop_sequence","stop_sequence":"","type":"message","usage":{"output_tokens_details":null,"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":null,"cache_creation":{"ephemeral_1h_input_tokens":0,"ephemeral_5m_input_tokens":0},"inference_geo":null,"iterations":null,"speed":null},"content":[{"type":"text","text":"You've hit your session limit · resets 8:10pm (Europe/Zagreb)"}],"context_management":null},"requestId":"req_redacted","quotaLimits":{"status":"rejected","resetsAt":1787940600,"rateLimitType":"five_hour","overageStatus":"rejected","isUsingOverage":false},"error":"rate_limit","isApiErrorMessage":true,"apiErrorStatus":429,"userType":"external","entrypoint":"cli","cwd":"/redacted","sessionId":"00000000-0000-0000-0000-000000000004","version":"2.1.267","gitBranch":"redacted"}"#;

    #[test]
    fn test_skips_sidechain_meta_compact_and_synthetic() {
        let usage = r#""usage":{"input_tokens":100,"output_tokens":50}"#;
        for line in [
            // A subagent's turn: its model and context, not the session's
            format!(
                r#"{{"type":"assistant","isSidechain":true,"message":{{"model":"claude-haiku-4-5",{usage}}}}}"#
            ),
            format!(
                r#"{{"type":"user","isMeta":true,"message":{{"model":"claude-opus-4-8",{usage}}}}}"#
            ),
            format!(
                r#"{{"type":"user","isCompactSummary":true,"message":{{"model":"claude-opus-4-8",{usage}}}}}"#
            ),
            // A local placeholder: there was no model, so there is no context
            format!(r#"{{"type":"assistant","message":{{"model":"<synthetic>",{usage}}}}}"#),
        ] {
            assert_eq!(parse_line(&line), None, "for {line}");
        }

        // The flags are only honoured when set: an explicit `false` is the
        // normal main-conversation record
        let normal = format!(
            r#"{{"type":"assistant","isSidechain":false,"isMeta":false,"message":{{"model":"claude-opus-4-8",{usage}}}}}"#
        );
        let Some(AgentEvent::Usage(usage)) = parse_line(&normal) else {
            panic!("a normal assistant record must still report usage");
        };
        assert_eq!(usage.total_tokens, Some(150));
        assert_eq!(usage.model.as_deref(), Some("claude-opus-4-8"));
    }

    #[test]
    fn test_api_error_record_is_turn_failed() {
        assert_eq!(
            parse_line(RATE_LIMIT_RECORD),
            Some(AgentEvent::TurnFailed {
                reason: Some("usage limit".to_string())
            })
        );

        // The other shapes seen on disk, trimmed to the fields that matter
        let auth = r#"{"isSidechain":false,"type":"assistant","message":{"model":"<synthetic>","content":[{"type":"text","text":"Not logged in · Please run /login"}]},"error":"authentication_failed","isApiErrorMessage":true}"#;
        assert_eq!(
            parse_line(auth),
            Some(AgentEvent::TurnFailed {
                reason: Some("auth failed".to_string())
            })
        );
        let too_long = r#"{"type":"assistant","message":{"model":"<synthetic>","content":[{"type":"text","text":"Prompt is too long"}]},"error":"invalid_request","isApiErrorMessage":true}"#;
        assert_eq!(
            parse_line(too_long),
            Some(AgentEvent::TurnFailed {
                reason: Some("prompt too long".to_string())
            })
        );

        // A code this was written before still names itself; no code at all
        // is still a failure, just an unexplained one
        let novel = r#"{"type":"assistant","error":"some_new_code","isApiErrorMessage":true}"#;
        assert_eq!(
            parse_line(novel),
            Some(AgentEvent::TurnFailed {
                reason: Some("some_new_code".to_string())
            })
        );
        let bare = r#"{"type":"assistant","isApiErrorMessage":true}"#;
        assert_eq!(
            parse_line(bare),
            Some(AgentEvent::TurnFailed { reason: None })
        );
    }

    #[test]
    fn test_sidechain_api_error_is_ignored() {
        // A subagent hitting a limit is the subagent's failure; the parent
        // turn carries on (or fails with its own top-level record)
        let sidechain =
            RATE_LIMIT_RECORD.replace(r#""isSidechain":false"#, r#""isSidechain":true"#);
        assert_ne!(sidechain, RATE_LIMIT_RECORD);
        assert_eq!(parse_line(&sidechain), None);
    }

    /// Shape of a real record (Claude Code 2.1.280), title redacted
    const AI_TITLE_RECORD: &str = r#"{"type":"ai-title","aiTitle":"Fix the login redirect","sessionId":"00000000-0000-4000-8000-000000000001"}"#;

    #[test]
    fn test_ai_title_record_emits_title_changed() {
        assert_eq!(
            parse_line(AI_TITLE_RECORD),
            Some(AgentEvent::TitleChanged {
                title: "Fix the login redirect".to_string()
            })
        );

        // Nothing to adopt: no title, or only whitespace
        assert_eq!(parse_line(r#"{"type":"ai-title","sessionId":"x"}"#), None);
        assert_eq!(
            parse_line(r#"{"type":"ai-title","aiTitle":"   ","sessionId":"x"}"#),
            None
        );
    }

    #[test]
    fn test_sidechain_ai_title_ignored() {
        // No real transcript carries one today, but the rule is the same as for
        // every other record: a subagent's title names its task, not the session
        let sidechain = AI_TITLE_RECORD.replace(
            r#""type":"ai-title","#,
            r#""type":"ai-title","isSidechain":true,"#,
        );
        assert_ne!(sidechain, AI_TITLE_RECORD);
        assert_eq!(parse_line(&sidechain), None);
    }

    #[test]
    fn test_failure_reason_labels() {
        for (code, label) in [
            ("authentication_failed", "auth failed"),
            ("rate_limit", "usage limit"),
            ("server_error", "server error"),
            ("overloaded", "server error"),
            ("billing_error", "out of credits"),
            ("invalid_request", "invalid request"),
            ("unknown", "api error"),
        ] {
            assert_eq!(
                failure_reason(Some(code), None).as_deref(),
                Some(label),
                "for {code}"
            );
        }
        assert_eq!(failure_reason(None, Some("anything")), None);
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

    #[test]
    fn test_transcript_exists_finds_the_file_even_under_a_different_slug() {
        let config = tempfile::TempDir::new().unwrap();
        let working_dir = Path::new("/w/project");
        assert!(!transcript_exists(config.path(), working_dir, "abc"));

        // Where we would derive it
        let derived = transcript_path(config.path(), working_dir, "abc");
        std::fs::create_dir_all(derived.parent().unwrap()).unwrap();
        std::fs::write(&derived, "{}\n").unwrap();
        assert!(transcript_exists(config.path(), working_dir, "abc"));

        // Filed by Claude under a directory name we did not predict
        let elsewhere = config.path().join("projects").join("-private-w-project");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("def.jsonl"), "{}\n").unwrap();
        assert!(transcript_exists(config.path(), working_dir, "def"));

        // Another config directory is another account: not found there
        let other = tempfile::TempDir::new().unwrap();
        assert!(!transcript_exists(other.path(), working_dir, "abc"));
    }
}
