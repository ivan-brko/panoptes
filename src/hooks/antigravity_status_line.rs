//! Antigravity CLI's status-line payload
//!
//! `agy` pipes a JSON document to its `statusLine` command whenever anything
//! on it changes - about every 300 ms while a turn runs, and on quota and
//! token updates while idle. For Panoptes it is the *only* report `agy` makes:
//! its hooks cannot be injected per session and miss an Esc-cancel, while the
//! status line covers every edge, including a turn the agent starts on its
//! own when a background command finishes (see the PAN-59 spike, whose
//! captures live in `src/agent/fixtures/antigravity/`).
//!
//! The shape, as `agy` 1.5.15 sends it:
//!
//! - `agent_state`: `authenticating`, `initializing`, `idle`, `working`, or
//!   `tool_use`. `error` and `reviewing` exist in the binary but were never
//!   produced.
//! - `tool_confirmation_pending: true` while a tool waits for approval, only
//!   ever together with `tool_use`. Absent, not `false`, otherwise.
//! - `conversation_id`, which is `""` until the first prompt of a new
//!   conversation.
//! - `task_count`, present only while background commands run.
//! - `context_window.{total_input_tokens,context_window_size,used_percentage}`
//! - `quota.<family>-weekly.{remaining_fraction,reset_time}`: `gemini-weekly`
//!   and `3p-weekly` on the consumer tier. There is no `cost` and no five-hour
//!   window.
//! - `model.display_name`, `plan_tier` and `email`.
//!
//! Its `transcript_path` is wrong in 1.5.15 (it names `antigravity/brain/`, the
//! real file is under `antigravity-cli/brain/`), so it is not read.
//!
//! Every field is read defensively: an absent or retyped field costs that
//! figure, never the rest.

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

use crate::agent::events::{format_reset_in, RateLimitWindow, UsageSnapshot, WindowSource};

/// The `product` value that marks a payload as Antigravity's
const PRODUCT: &str = "antigravity";

/// Length of a `*-weekly` quota window, in minutes
const WEEK: u64 = 7 * 24 * 60;

/// Length of a `*-daily` quota window, in minutes
const DAY: u64 = 24 * 60;

/// What the agent says it is doing, in Panoptes' terms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    /// Signing in or loading: not ready for a prompt yet
    Starting,
    /// Running a turn, auto-approved tools included
    Working,
    /// A tool call is waiting for the user to approve it
    AwaitingApproval,
    /// Back at the prompt. Background commands may still run, and their
    /// finishing starts a new turn by itself.
    Idle,
    /// The agent reports an error
    Error,
}

/// Whether a status-line payload came from Antigravity rather than Claude
pub fn is_antigravity(payload: &Value) -> bool {
    payload.get("product").and_then(Value::as_str) == Some(PRODUCT)
}

/// The agent's state, or `None` if the payload names none
///
/// A pending confirmation wins over whatever `agent_state` says. A state this
/// was not written for is a busy one: claiming the agent is idle when it is
/// not would ring for a turn that has not finished.
pub fn agent_state(payload: &Value) -> Option<AgentState> {
    if payload
        .get("tool_confirmation_pending")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Some(AgentState::AwaitingApproval);
    }
    let state = match payload.get("agent_state")?.as_str()? {
        "authenticating" | "initializing" => AgentState::Starting,
        "idle" => AgentState::Idle,
        "error" => AgentState::Error,
        // `working`, `tool_use` with nothing pending, `reviewing`, and
        // anything newer
        _ => AgentState::Working,
    };
    Some(state)
}

/// The conversation the agent is on, once it has one
///
/// `None` before the first prompt of a new conversation, when `agy` sends `""`.
pub fn conversation_id(payload: &Value) -> Option<&str> {
    non_empty_str(payload, "conversation_id")
}

/// The signed-in account, as `agy` names it
pub fn email(payload: &Value) -> Option<&str> {
    non_empty_str(payload, "email")
}

/// Background commands still running; `agy` leaves the key out at zero
pub fn task_count(payload: &Value) -> usize {
    payload
        .get("task_count")
        .and_then(Value::as_u64)
        .map_or(0, |n| n as usize)
}

/// A string field, or `None` when absent, retyped or empty
fn non_empty_str<'a>(payload: &'a Value, key: &str) -> Option<&'a str> {
    payload.get(key)?.as_str().filter(|s| !s.is_empty())
}

/// Read a status-line payload into usage figures
///
/// Returns `None` when the payload carries nothing usable - the startup
/// payloads, which have no model and a zero-sized window.
pub fn usage_from_payload(payload: &Value) -> Option<UsageSnapshot> {
    let context = payload.get("context_window");
    let context_window = context
        .and_then(|c| c.get("context_window_size"))
        .and_then(Value::as_u64)
        .filter(|size| *size > 0);
    // The figure `agy` computes its own `used_percentage` from, so the header
    // and the agent agree on how full the context is
    let total_tokens = context_window.and_then(|_| {
        context?
            .get("total_input_tokens")
            .and_then(Value::as_u64)
            .filter(|n| *n > 0)
    });
    let (primary, secondary) = quota_windows(payload.get("quota"));

    let snapshot = UsageSnapshot {
        total_tokens,
        context_window,
        context_window_source: WindowSource::Observed,
        model: payload
            .get("model")
            .and_then(|m| m.get("display_name").or_else(|| m.get("id")))
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .map(str::to_string),
        primary,
        secondary,
        plan: non_empty_str(payload, "plan_tier").map(str::to_string),
        ..Default::default()
    };

    let has_figures = snapshot.total_tokens.is_some()
        || snapshot.context_window.is_some()
        || snapshot.model.is_some()
        || snapshot.primary.is_some()
        || snapshot.secondary.is_some();
    has_figures.then_some(snapshot)
}

/// The quota windows, as `(primary, secondary)`
///
/// The Gemini family's window is the primary one; the most used of the rest
/// (`3p-weekly`, the third-party models' allowance, on the consumer tier) is
/// the secondary. The header shows whichever binds.
fn quota_windows(quota: Option<&Value>) -> (Option<RateLimitWindow>, Option<RateLimitWindow>) {
    let Some(quota) = quota.and_then(Value::as_object) else {
        return (None, None);
    };
    let mut primary = None;
    let mut others: Vec<RateLimitWindow> = Vec::new();
    for (name, value) in quota {
        let Some(window) = quota_window(name, value) else {
            continue;
        };
        if primary.is_none() && name.starts_with("gemini") {
            primary = Some(window);
        } else {
            others.push(window);
        }
    }
    others.sort_by(|a, b| b.used_percent.total_cmp(&a.used_percent));
    let mut others = others.into_iter();
    match primary {
        Some(primary) => (Some(primary), others.next()),
        None => (others.next(), others.next()),
    }
}

/// One quota window, or `None` if it carries no remaining fraction
fn quota_window(name: &str, value: &Value) -> Option<RateLimitWindow> {
    let remaining = value.get("remaining_fraction")?.as_f64()?;
    if !remaining.is_finite() {
        return None;
    }
    let window_minutes = if name.ends_with("-weekly") {
        Some(WEEK)
    } else if name.ends_with("-daily") {
        Some(DAY)
    } else {
        None
    };
    let resets_at = value
        .get("reset_time")
        .and_then(Value::as_str)
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&Utc));
    Some(RateLimitWindow {
        used_percent: ((1.0 - remaining) * 100.0).clamp(0.0, 100.0),
        window_minutes,
        resets_at,
    })
}

/// Panoptes' default status line for `agy`, e.g. `quota 1% (resets Nov 30 10:18)`
///
/// Drawn for users without a status line of their own. Only the binding quota
/// window, with its reset: both of the consumer tier's windows are weekly, so
/// listing both would read `wk 1% · wk 0%`. `agy` already shows the model and
/// the context itself. `None` before the first quota report.
pub fn compact_line<Tz>(payload: &Value, now: DateTime<Tz>) -> Option<String>
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    let usage = usage_from_payload(payload)?;
    let window = usage.binding_window()?;
    let mut line = format!("quota {:.0}%", window.used_percent);
    // The same reset format as the session header and Claude's line
    if let Some(reset) = window
        .resets_at
        .and_then(|at| format_reset_in(at, now.to_utc(), &now.timezone()))
    {
        line.push_str(&format!(" (resets {})", reset));
    }
    Some(line)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// One of the spike's recorded status-line payloads
    pub(crate) fn fixture(name: &str) -> Value {
        let path = format!(
            "{}/src/agent/fixtures/antigravity/status-line/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let content = std::fs::read_to_string(&path).expect("fixture exists");
        serde_json::from_str(&content).expect("fixture is valid JSON")
    }

    #[test]
    fn test_every_recorded_payload_maps_to_a_state() {
        let cases = [
            ("authenticating", AgentState::Starting),
            ("initializing", AgentState::Starting),
            ("idle-before-first-prompt", AgentState::Idle),
            ("working", AgentState::Working),
            ("tool-confirmation-pending", AgentState::AwaitingApproval),
            ("idle", AgentState::Idle),
            ("working-background-task-running", AgentState::Working),
            ("idle-background-task-running", AgentState::Idle),
            ("working-self-wake", AgentState::Working),
            ("idle-after-self-wake", AgentState::Idle),
            ("idle-auto-mode", AgentState::Idle),
            ("idle-plan-mode", AgentState::Idle),
            ("idle-accept-edits-mode", AgentState::Idle),
        ];
        for (name, expected) in cases {
            let payload = fixture(name);
            assert!(is_antigravity(&payload), "{name}");
            assert_eq!(agent_state(&payload), Some(expected), "{name}");
        }
    }

    #[test]
    fn test_states_never_recorded_still_map() {
        let state = |s: &str| agent_state(&serde_json::json!({ "agent_state": s }));
        assert_eq!(state("error"), Some(AgentState::Error));
        // Busy, whatever it is: never claim a turn is over when it may not be
        assert_eq!(state("reviewing"), Some(AgentState::Working));
        assert_eq!(state("something-new"), Some(AgentState::Working));
        // `tool_use` without a pending confirmation is an auto-approved tool
        assert_eq!(state("tool_use"), Some(AgentState::Working));
        assert_eq!(agent_state(&serde_json::json!({})), None);
        assert_eq!(agent_state(&Value::Null), None);
    }

    #[test]
    fn test_conversation_id_is_empty_until_the_first_prompt() {
        assert_eq!(conversation_id(&fixture("authenticating")), None);
        assert_eq!(conversation_id(&fixture("idle-before-first-prompt")), None);
        assert_eq!(
            conversation_id(&fixture("working")),
            Some("11111111-1111-4111-8111-111111111111")
        );
    }

    #[test]
    fn test_email_and_background_tasks() {
        assert_eq!(email(&fixture("authenticating")), None);
        assert_eq!(email(&fixture("idle")), Some("user@example.com"));
        assert_eq!(task_count(&fixture("idle")), 0);
        assert_eq!(task_count(&fixture("idle-background-task-running")), 1);
        assert_eq!(task_count(&fixture("working-background-task-running")), 1);
        assert_eq!(task_count(&fixture("idle-after-self-wake")), 0);
    }

    #[test]
    fn test_status_line_payload_to_usage() {
        let usage = usage_from_payload(&fixture("tool-confirmation-pending")).unwrap();

        assert_eq!(usage.model.as_deref(), Some("Gemini 3.8 Flash (High)"));
        assert_eq!(usage.context_window, Some(1_048_576));
        assert_eq!(usage.context_window_source, WindowSource::Observed);
        assert_eq!(usage.total_tokens, Some(14_019));
        assert_eq!(usage.plan.as_deref(), Some("Antigravity Starter Quota"));

        let gemini = usage.primary.as_ref().expect("gemini-weekly");
        assert!((gemini.used_percent - 0.56792).abs() < 1e-3);
        assert_eq!(gemini.window_minutes, Some(WEEK));
        assert_eq!(
            gemini.resets_at,
            DateTime::parse_from_rfc3339("2026-11-30T10:18:23Z")
                .ok()
                .map(|at| at.with_timezone(&Utc))
        );
        let third_party = usage.secondary.as_ref().expect("3p-weekly");
        assert_eq!(third_party.used_percent, 0.0);

        let now = DateTime::parse_from_rfc3339("2026-11-23T10:20:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            usage.summary_at(now).as_deref(),
            Some("Gemini 3.8 Flash (High) · ctx 1% · wk 1%")
        );
    }

    #[test]
    fn test_startup_payloads_carry_no_usage() {
        // No model, a zero window, no quota yet
        assert_eq!(usage_from_payload(&fixture("authenticating")), None);
        assert_eq!(usage_from_payload(&fixture("initializing")), None);
    }

    #[test]
    fn test_quota_tolerates_other_tiers_and_nonsense() {
        let (primary, secondary) = quota_windows(Some(&serde_json::json!({
            "pro-daily": {"remaining_fraction": 0.25},
            "3p-weekly": {"remaining_fraction": 0.5, "reset_time": "not a time"},
            "odd": {"remaining_fraction": "lots"},
        })));
        // No Gemini family: the two most used of the rest
        let primary = primary.unwrap();
        assert_eq!(primary.used_percent, 75.0);
        assert_eq!(primary.window_minutes, Some(DAY));
        let secondary = secondary.unwrap();
        assert_eq!(secondary.used_percent, 50.0);
        assert_eq!(secondary.resets_at, None);

        assert_eq!(quota_windows(Some(&serde_json::json!([1]))), (None, None));
        assert_eq!(quota_windows(None), (None, None));
    }

    #[test]
    fn test_compact_line() {
        let now = DateTime::parse_from_rfc3339("2026-11-23T10:20:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // A week out
        assert_eq!(
            compact_line(&fixture("idle"), now).as_deref(),
            Some("quota 1% (resets Nov 30 10:18)")
        );
        // A reset already past is left out
        let later = DateTime::parse_from_rfc3339("2026-12-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            compact_line(&fixture("idle"), later).as_deref(),
            Some("quota 1%")
        );
        assert_eq!(compact_line(&fixture("authenticating"), now), None);
    }
}
