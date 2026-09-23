//! Claude Code's status-line payload
//!
//! Claude Code pipes a JSON document to its `statusLine` command whenever the
//! status line refreshes. It is the only place Claude reports its plan rate
//! limits, and the only place it states the running session's context window
//! rather than leaving it to be guessed from the model name. Panoptes wraps
//! that command (see `agent/claude.rs`), which forwards the document here as
//! a `StatusLine` hook envelope.
//!
//! The shape, as Claude Code 2.1.280 sends it (a redacted capture lives in
//! `tests/fixtures/claude_status_line.json`):
//!
//! - `model.id`, e.g. `claude-opus-5-5[1m]`
//! - `context_window.context_window_size`, and `current_usage`, which is
//!   `null` until the first turn has been answered
//! - `rate_limits.five_hour` / `rate_limits.seven_day`, each
//!   `{used_percentage, resets_at}`: a 0-100 figure with one decimal, and
//!   epoch seconds. `rate_limits` is absent until the first API response has
//!   delivered the headers it is read from, and for API-key sessions.
//!
//! Every field is read defensively: an absent or retyped field costs that
//! figure, never the rest of the snapshot.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::agent::events::{base_model_id, RateLimitWindow, UsageSnapshot, WindowSource};

/// Length of the `five_hour` window, in minutes
const FIVE_HOURS: u64 = 5 * 60;

/// Length of the `seven_day` window, in minutes
const SEVEN_DAYS: u64 = 7 * 24 * 60;

/// Read a status-line payload into usage figures
///
/// Returns `None` when the payload carries nothing usable, e.g. the empty
/// envelope a sender without a payload would post.
pub fn usage_from_payload(payload: &Value) -> Option<UsageSnapshot> {
    let context = payload.get("context_window");
    let rate_limits = payload.get("rate_limits");

    let snapshot = UsageSnapshot {
        total_tokens: context
            .and_then(|c| c.get("current_usage"))
            .and_then(context_tokens),
        context_window: context
            .and_then(|c| c.get("context_window_size"))
            .and_then(Value::as_u64)
            .filter(|size| *size > 0),
        // The session reports its own window: nothing inferred may replace it
        context_window_source: WindowSource::Observed,
        // Claude reports the id it was launched with, `[1m]` included, while
        // its transcript logs the bare id. Stored bare, the two sources agree
        // on the model and the header does not flicker between them; the
        // window this reports already carries what the suffix meant.
        model: payload
            .get("model")
            .and_then(|m| m.get("id"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(|id| base_model_id(id).to_string()),
        primary: rate_limits
            .and_then(|r| r.get("five_hour"))
            .and_then(|w| window(w, FIVE_HOURS)),
        secondary: rate_limits
            .and_then(|r| r.get("seven_day"))
            .and_then(|w| window(w, SEVEN_DAYS)),
        ..Default::default()
    };

    // `Observed` alone, with nothing it describes, is not worth reporting
    let has_figures = snapshot.total_tokens.is_some()
        || snapshot.context_window.is_some()
        || snapshot.model.is_some()
        || snapshot.primary.is_some()
        || snapshot.secondary.is_some();
    has_figures.then_some(snapshot)
}

/// Tokens in the context, counted the way the transcript reader counts them
///
/// Summed from the same four fields, so a status-line figure and a transcript
/// figure for the same turn agree rather than nudging the header back and
/// forth. `None` before the first turn, when Claude sends `null`.
fn context_tokens(usage: &Value) -> Option<u64> {
    let total: u64 = [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "output_tokens",
    ]
    .iter()
    .filter_map(|field| usage.get(*field).and_then(Value::as_u64))
    .sum();
    (total > 0).then_some(total)
}

/// One rate-limit window, or `None` if it carries no usage figure
fn window(value: &Value, minutes: u64) -> Option<RateLimitWindow> {
    let used_percent = value.get("used_percentage")?.as_f64()?;
    let resets_at = value
        .get("resets_at")
        .and_then(Value::as_i64)
        .and_then(|secs| DateTime::<Utc>::from_timestamp(secs, 0));
    Some(RateLimitWindow {
        used_percent,
        window_minutes: Some(minutes),
        resets_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real Claude Code 2.1.280 status-line payload, captured after one turn
    /// and redacted
    const FIXTURE: &str = include_str!("../../tests/fixtures/claude_status_line.json");

    fn fixture() -> Value {
        serde_json::from_str(FIXTURE).expect("fixture is valid JSON")
    }

    #[test]
    fn test_status_line_payload_to_usage() {
        let usage = usage_from_payload(&fixture()).expect("the fixture carries figures");

        assert_eq!(usage.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(usage.context_window, Some(1_000_000));
        assert_eq!(usage.context_window_source, WindowSource::Observed);
        // 2 + 16245 + 24649 + 4
        assert_eq!(usage.total_tokens, Some(40_900));

        let five_hour = usage.primary.as_ref().expect("five_hour");
        assert_eq!(five_hour.used_percent, 21.0);
        assert_eq!(five_hour.window_minutes, Some(300));
        assert_eq!(
            five_hour.resets_at,
            DateTime::<Utc>::from_timestamp(1_790_175_600, 0)
        );

        let week = usage.secondary.as_ref().expect("seven_day");
        assert_eq!(week.used_percent, 11.0);
        assert_eq!(week.window_minutes, Some(10_080));
        assert_eq!(
            week.resets_at,
            DateTime::<Utc>::from_timestamp(1_790_722_800, 0)
        );

        // Claude's limits read exactly as Codex's do: the binding window
        let now = DateTime::<Utc>::from_timestamp(1_790_170_061, 0).unwrap();
        assert_eq!(
            usage.summary_at(now).as_deref(),
            Some("opus-5-5 · ctx 4% · 5h 21%")
        );
    }

    #[test]
    fn test_status_line_before_the_first_turn() {
        // What Claude sends at startup: no usage yet, no rate limits
        let mut payload = fixture();
        payload["context_window"]["current_usage"] = Value::Null;
        payload.as_object_mut().unwrap().remove("rate_limits");

        let usage = usage_from_payload(&payload).unwrap();
        assert_eq!(usage.total_tokens, None);
        assert_eq!(usage.context_window, Some(1_000_000));
        assert_eq!(usage.primary, None);
        assert_eq!(usage.secondary, None);
    }

    #[test]
    fn test_status_line_week_binds_when_fuller() {
        let mut payload = fixture();
        payload["rate_limits"]["seven_day"]["used_percentage"] = serde_json::json!(62.5);

        let usage = usage_from_payload(&payload).unwrap();
        let now = DateTime::<Utc>::from_timestamp(1_790_170_061, 0).unwrap();
        assert!(usage.summary_at(now).unwrap().ends_with("wk 62%"));
    }

    #[test]
    fn test_status_line_tolerates_missing_and_retyped_fields() {
        assert_eq!(usage_from_payload(&Value::Null), None);
        assert_eq!(usage_from_payload(&serde_json::json!({})), None);

        // A window without a percentage is dropped; one without a reset time
        // is kept
        let usage = usage_from_payload(&serde_json::json!({
            "rate_limits": {
                "five_hour": {"resets_at": 1},
                "seven_day": {"used_percentage": 3}
            },
            "context_window": {"context_window_size": "big"}
        }))
        .unwrap();
        assert_eq!(usage.primary, None);
        assert_eq!(usage.secondary.as_ref().unwrap().resets_at, None);
        assert_eq!(usage.context_window, None);
    }
}
