//! Canonical agent event vocabulary
//!
//! Agents report themselves in incompatible ways. Claude Code fires HTTP hooks
//! naming its own event types; Codex CLI fires exactly one hook and writes
//! everything else to a rollout file on disk. Before this existed, the session
//! state machine spoke Claude's vocabulary with a single Codex event bolted
//! onto the side, which meant Codex could only ever say "my turn ended".
//!
//! [`AgentEvent`] is what the state machine actually consumes. Each source
//! translates into it, so there is one place that decides what an event *means*
//! and several small places that decide what an event *is*.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::hooks::NotificationKind;

/// Something an agent did, expressed in terms the session model understands
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    /// A turn began - the user asked for something
    TurnStarted {
        /// The agent's own title for the conversation, when it offers one
        title: Option<String>,
    },

    /// A tool started running
    ToolStarted {
        /// Unique per invocation, so concurrent tools stay distinguishable
        key: String,
        /// Tool name as the agent reports it
        name: String,
    },

    /// A tool finished, successfully or not
    ToolFinished {
        /// Matches the key from [`AgentEvent::ToolStarted`]
        key: String,
    },

    /// The turn ended normally
    TurnCompleted {
        /// The assistant's closing message, when the agent reports one
        last_message: Option<String>,
    },

    /// The turn was interrupted before it finished
    TurnAborted,

    /// The turn died on an error before it finished - a usage limit, an
    /// expired login, an overloaded API
    ///
    /// Unlike [`AgentEvent::TurnAborted`] nobody chose this, so it is worth
    /// telling the user about. The agent is back at its prompt either way.
    TurnFailed {
        /// Short human label for why, e.g. "usage limit"; `None` when the
        /// agent gave no reason
        reason: Option<String>,
    },

    /// The agent is blocked waiting for the user to approve something
    ApprovalRequested {
        /// The tool awaiting approval, when the agent names one
        tool: Option<String>,
    },

    /// Something the agent was blocked on the user for went away without the
    /// turn ending - a tool call denied without a dialog, or an MCP question
    /// answered
    ///
    /// The turn carries on afterwards, so this demotes an open approval rather
    /// than finishing anything.
    ApprovalResolved {
        /// The tool whose approval resolved, when the agent names one; `None`
        /// resolves any approval
        tool: Option<String>,
    },

    /// A subagent began running inside this session
    ///
    /// Claude's subagents share the parent's process and session, so unlike
    /// Codex's they are reported by the agent itself, one at a time, rather
    /// than inferred from files on disk.
    SubagentStarted {
        /// Pairs this with its [`AgentEvent::SubagentFinished`]
        id: String,
    },

    /// A subagent finished
    SubagentFinished {
        /// Matches the id from [`AgentEvent::SubagentStarted`]
        id: String,
    },

    /// A snapshot of the work that outlives the turn
    ///
    /// Background shells, monitors, backgrounded subagents and scheduled
    /// prompts keep running - or will wake the session - after the turn has
    /// settled into `Waiting`. Each figure replaces the last rather than
    /// adding to it; `None` means the agent did not say, and leaves the known
    /// figure alone.
    BackgroundWork {
        /// In-flight background tasks of every kind
        tasks: Option<usize>,
        /// Session-scoped scheduled prompts (`/loop` and friends)
        crons: Option<usize>,
        /// How many of `tasks` are subagents - only reported at the end of a
        /// turn, when every subagent still running is a backgrounded one and
        /// so is guaranteed to be listed
        subagents: Option<usize>,
    },

    /// The agent is reminding the user that nothing has happened
    ///
    /// Deliberately distinct from every other event: it reports the *absence*
    /// of activity, so it must not be treated as activity.
    IdleReminder,

    /// A fresh conversation: process start, resume, `/clear`, or `/fork`
    SessionReset {
        /// The agent's own title for the conversation, when it offers one
        title: Option<String>,
    },

    /// The conversation was compacted mid-turn
    ///
    /// The agent carries on working, so this must not be mistaken for a reset.
    ContextCompacted,

    /// The agent's process is shutting down
    SessionEnding,

    /// Fresh token and rate-limit figures
    Usage(UsageSnapshot),

    /// How many subagents this session appears to be running
    ///
    /// Codex subagents write their own separate rollout files, so a parent
    /// session looks idle while its children work. Discovering them is exact;
    /// knowing whether one is still running is inference, so this is reported
    /// as a count rather than as a claim about what they are doing.
    Subagents {
        /// Subagent rollouts written recently enough to look alive
        active: usize,
    },

    /// The agent renamed the conversation
    ///
    /// Claude writes `ai-title` records into its transcript; Codex keeps its
    /// thread names in `$CODEX_HOME/session_index.jsonl`. Both revise the
    /// title as the conversation evolves, so the latest one wins. It names the
    /// conversation rather than reporting anything the agent did, so it is not
    /// activity.
    TitleChanged {
        /// The agent's title, as it wrote it
        title: String,
    },

    /// Recognised but deliberately not modelled
    Ignored,
}

/// Token and rate-limit figures scraped from an agent's own records
///
/// Every field is optional because the two agents report different subsets.
/// Codex publishes rate limits; Claude publishes none at all, so its sessions
/// show context usage and model only.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    /// Tokens consumed by the conversation so far
    #[serde(default)]
    pub total_tokens: Option<u64>,

    /// Size of the model's context window, if known
    #[serde(default)]
    pub context_window: Option<u64>,

    /// How `context_window` was arrived at, which decides whether a later
    /// figure may replace it
    #[serde(default)]
    pub context_window_source: WindowSource,

    /// The model `context_window` was established for
    ///
    /// Kept apart from `model`, which any record may rename: Claude writes
    /// `<synthetic>` records that name no real model, and one of those must not
    /// read as a model switch that discards an observed window.
    #[serde(default)]
    pub context_window_model: Option<String>,

    /// Model currently serving the conversation
    #[serde(default)]
    pub model: Option<String>,

    /// The short rate-limit window, five hours on current plans (Codex only)
    #[serde(default)]
    pub primary: Option<RateLimitWindow>,

    /// The long rate-limit window, a week on current plans (Codex only)
    ///
    /// Tracked separately because it is routinely the one that bites: a quiet
    /// morning can leave the five-hour window at 1% while the week sits at 40%.
    #[serde(default)]
    pub secondary: Option<RateLimitWindow>,

    /// Plan name backing the rate limit (Codex only)
    #[serde(default)]
    pub plan: Option<String>,

    /// Why the agent has stopped accepting turns, when it has (Codex only)
    ///
    /// The agent's own reason string, e.g. `workspace_member_credits_depleted`.
    /// Only its presence is shown; the wording is Codex's and may change.
    #[serde(default)]
    pub limit_reached: Option<String>,
}

/// One rate-limit window as the agent reported it
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RateLimitWindow {
    /// Share of the window's allowance consumed, 0-100
    #[serde(default)]
    pub used_percent: f64,

    /// How long the window is; decides its label (`5h`, `wk`)
    #[serde(default)]
    pub window_minutes: Option<u64>,

    /// When the window's allowance is restored
    #[serde(default)]
    pub resets_at: Option<DateTime<Utc>>,
}

impl RateLimitWindow {
    /// Fold a newer reading of the same window in, keeping what it omits
    fn merge(&mut self, newer: RateLimitWindow) {
        self.used_percent = newer.used_percent;
        if newer.window_minutes.is_some() {
            self.window_minutes = newer.window_minutes;
        }
        if newer.resets_at.is_some() {
            self.resets_at = newer.resets_at;
        }
    }

    /// Short name for the window's length: `5h`, `wk`, or `limit` when unknown
    fn label(&self) -> String {
        match self.window_minutes {
            None | Some(0) => "limit".to_string(),
            Some(10_080) => "wk".to_string(),
            Some(m) if m % 1_440 == 0 => format!("{}d", m / 1_440),
            Some(m) if m % 60 == 0 => format!("{}h", m / 60),
            Some(m) => format!("{}m", m),
        }
    }
}

/// Where a context window figure came from, weakest first
///
/// Claude's transcript names the model but never its window, and a 1M run of a
/// model logs the same bare id as a 200k run of it, so a window read off the
/// model name is a guess. A figure the agent reports itself is not, and must not
/// be overwritten by the next guess that happens to arrive after it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WindowSource {
    /// Looked up from the model name
    #[default]
    Inferred,
    /// Implied by how Panoptes launched the agent, e.g. `--model opus[1m]`
    Launch,
    /// Reported by the agent itself for the running session
    Observed,
}

impl UsageSnapshot {
    /// Whether this snapshot carries anything worth showing
    pub fn is_empty(&self) -> bool {
        *self == UsageSnapshot::default()
    }

    /// Fold newer figures in, keeping known values the update does not mention
    ///
    /// Sources are partial and interleaved: a Claude assistant record names the
    /// model and its token counts but no rate limit, while a Codex
    /// `token_count` carries limits but no model. Overwriting wholesale would
    /// make fields flicker between present and absent.
    ///
    /// The context window is the one field that is not simply last-writer-wins:
    /// see [`Self::accepts_window_from`].
    pub fn merge(&mut self, newer: UsageSnapshot) {
        let newer_has_limits =
            newer.primary.is_some() || newer.secondary.is_some() || newer.limit_reached.is_some();
        if newer.total_tokens.is_some() {
            self.total_tokens = newer.total_tokens;
        }
        // Decided before `model` is overwritten, since a model switch is one of
        // the reasons to let a weaker figure through
        if newer.context_window.is_some() {
            if self.accepts_window_from(&newer) {
                self.context_window = newer.context_window;
                self.context_window_source = newer.context_window_source;
                self.context_window_model = newer.model.clone();
            } else if self.context_window_model.is_none() {
                // A launch window names no model; it belongs to the first one
                // seen running under it, so a later switch away can be noticed
                self.context_window_model = newer.model.clone();
            }
        }
        if newer.model.is_some() {
            self.model = newer.model;
        }
        // Each window stands alone: an update that only mentions the five-hour
        // window says nothing about the week
        merge_window(&mut self.primary, newer.primary);
        merge_window(&mut self.secondary, newer.secondary);
        if newer.plan.is_some() {
            self.plan = newer.plan;
        }
        // A record carrying rate limits is authoritative about whether the limit
        // is hit, and Codex writes an explicit null once it is lifted. Merging
        // this like the other fields would leave "limit hit" up forever.
        if newer_has_limits {
            self.limit_reached = newer.limit_reached;
        }
    }

    /// Whether `newer`'s context window may replace the one already held
    ///
    /// A figure from an equal or stronger [`WindowSource`] always may. A weaker
    /// one - the transcript's guess arriving after the agent reported the real
    /// window - may only when the model has changed underneath it, because the
    /// stronger figure described the previous model and is stale now. `[1m]` is
    /// ignored in that comparison: an id reported with it and the transcript's
    /// bare id name the same model.
    fn accepts_window_from(&self, newer: &UsageSnapshot) -> bool {
        if self.context_window.is_none()
            || newer.context_window_source >= self.context_window_source
        {
            return true;
        }
        match (&self.context_window_model, &newer.model) {
            (Some(held), Some(incoming)) => base_model_id(held) != base_model_id(incoming),
            _ => false,
        }
    }

    /// How full the context window is, as a percentage
    pub fn context_percent(&self) -> Option<f64> {
        let used = self.total_tokens? as f64;
        let window = self.context_window? as f64;
        if window <= 0.0 {
            return None;
        }
        Some((used / window * 100.0).clamp(0.0, 100.0))
    }

    /// Compact description for the session header, or `None` if nothing is known
    ///
    /// Reads like `gpt-5.5 · ctx 34% · wk 40%`. Rate limit is omitted rather
    /// than shown as zero when the agent does not report one, because "we do not
    /// know" and "you have used none of it" are different claims.
    pub fn summary(&self) -> Option<String> {
        self.summary_at(Utc::now())
    }

    /// [`Self::summary`] against a given clock, so the reset countdown is testable
    pub fn summary_at(&self, now: DateTime<Utc>) -> Option<String> {
        let mut parts = Vec::new();

        if let Some(model) = &self.model {
            parts.push(short_model_name(model).to_string());
        }
        if let Some(pct) = self.context_percent() {
            parts.push(format!("ctx {:.0}%", pct));
        } else if let Some(total) = self.total_tokens {
            parts.push(format!("{} tok", format_thousands(total)));
        }
        let binding = self.binding_window();
        if self.limit_reached.is_some() {
            // Blocked outright - how full the window is no longer matters, only
            // when it lets the user back in
            match binding.and_then(|w| w.resets_at) {
                Some(at) => parts.push(format!("limit hit · resets {}", format_reset(at, now))),
                None => parts.push("limit hit".to_string()),
            }
        } else if let Some(window) = binding {
            parts.push(format!("{} {:.0}%", window.label(), window.used_percent));
        }

        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// The window closest to stopping the user, which is the one worth showing
    ///
    /// Higher usage wins; on a tie the longer window does, since it takes
    /// longer to recover from.
    fn binding_window(&self) -> Option<&RateLimitWindow> {
        match (&self.primary, &self.secondary) {
            (Some(a), Some(b)) => {
                let a_key = (a.used_percent, a.window_minutes.unwrap_or(0));
                let b_key = (b.used_percent, b.window_minutes.unwrap_or(0));
                Some(if b_key > a_key { b } else { a })
            }
            (a, b) => a.as_ref().or(b.as_ref()),
        }
    }
}

/// Fold a newer reading of one window into what is known of it
fn merge_window(current: &mut Option<RateLimitWindow>, newer: Option<RateLimitWindow>) {
    match (current.as_mut(), newer) {
        (Some(known), Some(newer)) => known.merge(newer),
        (None, Some(newer)) => *current = Some(newer),
        (_, None) => {}
    }
}

/// Render the time until `at` as `in 3h 20m` / `in 2d 4h` / `in 45m`
///
/// Minutes are rounded up so a reset under a minute away still reads as
/// pending rather than `in 0m`; a reset already past reads as `now`.
fn format_reset(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = (at - now).num_seconds();
    if seconds <= 0 {
        return "now".to_string();
    }
    let minutes = (seconds + 59) / 60;
    let (days, hours, mins) = (minutes / 1_440, minutes % 1_440 / 60, minutes % 60);
    match (days, hours) {
        (0, 0) => format!("in {}m", mins),
        (0, _) => format!("in {}h {}m", hours, mins),
        _ => format!("in {}d {}h", days, hours),
    }
}

/// Trim a model identifier down to something that fits in a header
///
/// `claude-opus-4-8-20260101` reads as `opus-4-8`; `gpt-5-codex` is left alone.
fn short_model_name(model: &str) -> &str {
    let trimmed = model.strip_prefix("claude-").unwrap_or(model);
    // Drop a trailing date stamp, which is never the interesting part
    match trimmed.rsplit_once('-') {
        Some((head, tail)) if tail.len() == 8 && tail.chars().all(|c| c.is_ascii_digit()) => head,
        _ => trimmed,
    }
}

/// A model id without its `[1m]` context suffix, for telling models apart
fn base_model_id(model: &str) -> &str {
    let stem_len = model.len().saturating_sub("[1m]".len());
    match model.get(stem_len..) {
        Some(suffix) if suffix.eq_ignore_ascii_case("[1m]") => &model[..stem_len],
        _ => model,
    }
}

/// Render a token count as `1.2M` / `34.5k` / `812`
fn format_thousands(n: u64) -> String {
    match n {
        0..=9_999 => n.to_string(),
        10_000..=999_999 => format!("{:.0}k", n as f64 / 1_000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

impl From<NotificationKind> for AgentEvent {
    fn from(kind: NotificationKind) -> Self {
        match kind {
            NotificationKind::Idle => AgentEvent::IdleReminder,
            NotificationKind::PermissionRequest | NotificationKind::Elicitation => {
                AgentEvent::ApprovalRequested { tool: None }
            }
            NotificationKind::TaskCompleted => AgentEvent::TurnCompleted { last_message: None },
            NotificationKind::Informational => AgentEvent::Ignored,
            // Either a notification type the agent added after this was written,
            // or the degraded path where no payload arrived at all. Both are
            // more likely to want the user than not.
            NotificationKind::Other => AgentEvent::ApprovalRequested { tool: None },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_keeps_fields_the_update_omits() {
        let mut usage = UsageSnapshot {
            model: Some("gpt-5-codex".to_string()),
            context_window: Some(200_000),
            total_tokens: Some(1_000),
            ..Default::default()
        };

        // A later record that only knows the token count must not blank the rest
        usage.merge(UsageSnapshot {
            total_tokens: Some(2_000),
            ..Default::default()
        });

        assert_eq!(usage.total_tokens, Some(2_000));
        assert_eq!(usage.context_window, Some(200_000));
        assert_eq!(usage.model.as_deref(), Some("gpt-5-codex"));
    }

    #[test]
    fn test_context_percent() {
        let usage = UsageSnapshot {
            total_tokens: Some(50_000),
            context_window: Some(200_000),
            ..Default::default()
        };
        assert_eq!(usage.context_percent(), Some(25.0));

        // Missing either half means we do not know
        assert_eq!(
            UsageSnapshot {
                total_tokens: Some(50_000),
                ..Default::default()
            }
            .context_percent(),
            None
        );

        // A nonsense window must not divide by zero
        assert_eq!(
            UsageSnapshot {
                total_tokens: Some(1),
                context_window: Some(0),
                ..Default::default()
            }
            .context_percent(),
            None
        );
    }

    #[test]
    fn test_summary() {
        let codex = UsageSnapshot {
            model: Some("gpt-5-codex".to_string()),
            total_tokens: Some(68_000),
            context_window: Some(200_000),
            primary: Some(window(12.4, Some(300), None)),
            ..Default::default()
        };
        assert_eq!(
            codex.summary().as_deref(),
            Some("gpt-5-codex · ctx 34% · 5h 12%")
        );

        // A window of unknown length still shows, under a generic label
        let unlabelled = UsageSnapshot {
            primary: Some(window(12.4, None, None)),
            ..Default::default()
        };
        assert_eq!(unlabelled.summary().as_deref(), Some("limit 12%"));

        // Claude publishes no rate limit at all, so none is shown - as opposed
        // to showing 0%, which would claim something we do not know
        let claude = UsageSnapshot {
            model: Some("claude-opus-4-8-20260101".to_string()),
            total_tokens: Some(20_000),
            context_window: Some(200_000),
            ..Default::default()
        };
        assert_eq!(claude.summary().as_deref(), Some("opus-4-8 · ctx 10%"));

        assert_eq!(UsageSnapshot::default().summary(), None);
    }

    fn window(
        used: f64,
        minutes: Option<u64>,
        resets_at: Option<DateTime<Utc>>,
    ) -> RateLimitWindow {
        RateLimitWindow {
            used_percent: used,
            window_minutes: minutes,
            resets_at,
        }
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("valid timestamp")
    }

    #[test]
    fn test_merge_keeps_secondary_when_update_has_only_primary() {
        let mut usage = UsageSnapshot {
            primary: Some(window(1.0, Some(300), Some(at(1_000)))),
            secondary: Some(window(40.0, Some(10_080), Some(at(9_000)))),
            ..Default::default()
        };

        usage.merge(UsageSnapshot {
            primary: Some(window(5.0, Some(300), None)),
            ..Default::default()
        });

        assert_eq!(
            usage.secondary,
            Some(window(40.0, Some(10_080), Some(at(9_000))))
        );
        // Within a window, too, a field the update omits is kept
        assert_eq!(usage.primary, Some(window(5.0, Some(300), Some(at(1_000)))));
    }

    #[test]
    fn test_merge_clears_limit_reached_once_lifted() {
        let mut usage = UsageSnapshot {
            primary: Some(window(100.0, Some(300), None)),
            limit_reached: Some("workspace_member_credits_depleted".to_string()),
            ..Default::default()
        };

        // A token-only update knows nothing about limits, so leaves it alone
        usage.merge(UsageSnapshot {
            total_tokens: Some(10),
            ..Default::default()
        });
        assert!(usage.limit_reached.is_some());

        // A rate-limit reading without a reason means the block has lifted
        usage.merge(UsageSnapshot {
            primary: Some(window(2.0, Some(300), None)),
            ..Default::default()
        });
        assert_eq!(usage.limit_reached, None);
    }

    #[test]
    fn test_summary_shows_most_constraining_window() {
        // The week is the binding constraint even though the 5h window is fresher
        let usage = UsageSnapshot {
            primary: Some(window(1.0, Some(300), None)),
            secondary: Some(window(40.0, Some(10_080), None)),
            ..Default::default()
        };
        assert_eq!(usage.summary().as_deref(), Some("wk 40%"));

        let usage = UsageSnapshot {
            primary: Some(window(85.0, Some(300), None)),
            secondary: Some(window(26.0, Some(10_080), None)),
            ..Default::default()
        };
        assert_eq!(usage.summary().as_deref(), Some("5h 85%"));

        // A tie goes to the longer window, which takes longer to recover
        let usage = UsageSnapshot {
            primary: Some(window(30.0, Some(300), None)),
            secondary: Some(window(30.0, Some(10_080), None)),
            ..Default::default()
        };
        assert_eq!(usage.summary().as_deref(), Some("wk 30%"));

        // Either window alone is shown
        let usage = UsageSnapshot {
            secondary: Some(window(7.0, Some(10_080), None)),
            ..Default::default()
        };
        assert_eq!(usage.summary().as_deref(), Some("wk 7%"));
    }

    #[test]
    fn test_summary_limit_reached_shows_reset_countdown() {
        let now = at(1_000_000);
        let usage = UsageSnapshot {
            model: Some("gpt-5.5".to_string()),
            primary: Some(window(
                100.0,
                Some(300),
                Some(now + chrono::Duration::minutes(200)),
            )),
            secondary: Some(window(
                60.0,
                Some(10_080),
                Some(now + chrono::Duration::days(3)),
            )),
            limit_reached: Some("workspace_member_credits_depleted".to_string()),
            ..Default::default()
        };
        assert_eq!(
            usage.summary_at(now).as_deref(),
            Some("gpt-5.5 · limit hit · resets in 3h 20m")
        );

        // Without a reset time the block is still shown, just without a countdown
        let usage = UsageSnapshot {
            primary: Some(window(100.0, Some(300), None)),
            limit_reached: Some("x".to_string()),
            ..Default::default()
        };
        assert_eq!(usage.summary_at(now).as_deref(), Some("limit hit"));
    }

    #[test]
    fn test_format_reset() {
        let now = at(1_000_000);
        let later = |secs| now + chrono::Duration::seconds(secs);
        assert_eq!(format_reset(later(200 * 60), now), "in 3h 20m");
        assert_eq!(format_reset(later(45 * 60), now), "in 45m");
        assert_eq!(format_reset(later(30), now), "in 1m");
        assert_eq!(format_reset(later((2 * 24 + 4) * 3_600), now), "in 2d 4h");
        assert_eq!(format_reset(later(0), now), "now");
        assert_eq!(format_reset(later(-60), now), "now");
    }

    #[test]
    fn test_window_label() {
        assert_eq!(window(0.0, Some(300), None).label(), "5h");
        assert_eq!(window(0.0, Some(10_080), None).label(), "wk");
        assert_eq!(window(0.0, Some(1_440), None).label(), "1d");
        assert_eq!(window(0.0, Some(90), None).label(), "90m");
        assert_eq!(window(0.0, None, None).label(), "limit");
    }

    #[test]
    fn test_summary_falls_back_to_raw_tokens_without_a_window() {
        let usage = UsageSnapshot {
            total_tokens: Some(34_500),
            ..Default::default()
        };
        assert_eq!(usage.summary().as_deref(), Some("34k tok"));
    }

    #[test]
    fn test_short_model_name() {
        assert_eq!(short_model_name("claude-opus-4-8-20260101"), "opus-4-8");
        assert_eq!(short_model_name("gpt-5-codex"), "gpt-5-codex");
        assert_eq!(short_model_name("o3"), "o3");
    }

    #[test]
    fn test_short_model_name_current_ids() {
        assert_eq!(short_model_name("claude-opus-5-5"), "opus-5-5");
        assert_eq!(short_model_name("claude-fable-5-1"), "fable-5-1");
        assert_eq!(short_model_name("claude-sonnet-5"), "sonnet-5");
        assert_eq!(short_model_name("claude-haiku-4-5-20251001"), "haiku-4-5");
    }

    /// A transcript record: model named, window guessed from it
    fn inferred(model: &str, window: u64) -> UsageSnapshot {
        UsageSnapshot {
            model: Some(model.to_string()),
            context_window: Some(window),
            context_window_source: WindowSource::Inferred,
            ..Default::default()
        }
    }

    /// The agent's own report of the running session's window
    fn observed(model: &str, window: u64) -> UsageSnapshot {
        UsageSnapshot {
            model: Some(model.to_string()),
            context_window: Some(window),
            context_window_source: WindowSource::Observed,
            ..Default::default()
        }
    }

    /// A session's usage after taking in `first`, as `merge` would leave it
    fn holding(first: UsageSnapshot) -> UsageSnapshot {
        let mut usage = UsageSnapshot::default();
        usage.merge(first);
        usage
    }

    #[test]
    fn test_observed_window_beats_inferred() {
        // Observed first: the next transcript guess must not replace it. This
        // is the real 200k-capped run of a model whose default is 1M.
        let mut usage = holding(observed("claude-opus-5-5", 200_000));
        usage.merge(inferred("claude-opus-5-5", 1_000_000));
        assert_eq!(usage.context_window, Some(200_000));
        assert_eq!(usage.context_window_source, WindowSource::Observed);

        // Inferred first: the observed figure replaces the guess
        let mut usage = holding(inferred("claude-opus-5-5", 1_000_000));
        usage.merge(observed("claude-opus-5-5", 200_000));
        assert_eq!(usage.context_window, Some(200_000));
        assert_eq!(usage.context_window_source, WindowSource::Observed);

        // A newer observation still replaces an older one
        usage.merge(observed("claude-opus-5-5", 1_000_000));
        assert_eq!(usage.context_window, Some(1_000_000));
    }

    #[test]
    fn test_launch_window_survives_the_bare_transcript_id() {
        // `--model claude-sonnet-4-6[1m]` seeds 1M with no model named; the
        // transcript then logs the bare id and guesses 200k
        let mut usage = holding(UsageSnapshot {
            context_window: Some(1_000_000),
            context_window_source: WindowSource::Launch,
            ..Default::default()
        });
        usage.merge(inferred("claude-sonnet-4-6", 200_000));
        assert_eq!(usage.context_window, Some(1_000_000));

        // The same model again, even spelled with its suffix, changes nothing
        usage.merge(inferred("claude-sonnet-4-6[1m]", 200_000));
        assert_eq!(usage.context_window, Some(1_000_000));

        // The launch window was taken as the first model's, so leaving that
        // model lets the new one's guess through
        usage.merge(inferred("claude-haiku-4-5-20251001", 200_000));
        assert_eq!(usage.context_window, Some(200_000));
        assert_eq!(usage.context_window_source, WindowSource::Inferred);
    }

    #[test]
    fn test_model_switch_lets_the_new_models_guess_through() {
        // `/model` mid-session: the observed window described the old model
        let mut usage = holding(observed("claude-opus-5-5", 1_000_000));
        usage.merge(inferred("claude-haiku-4-5-20251001", 200_000));
        assert_eq!(usage.context_window, Some(200_000));
        assert_eq!(usage.context_window_source, WindowSource::Inferred);
        assert_eq!(usage.model.as_deref(), Some("claude-haiku-4-5-20251001"));
    }

    #[test]
    fn test_synthetic_record_is_not_a_model_switch() {
        let mut usage = holding(observed("claude-opus-5-5", 200_000));
        // Claude's `<synthetic>` records name a model but carry no window
        usage.merge(UsageSnapshot {
            model: Some("<synthetic>".to_string()),
            ..Default::default()
        });
        usage.merge(inferred("claude-opus-5-5", 1_000_000));
        assert_eq!(usage.context_window, Some(200_000));
        assert_eq!(usage.context_window_source, WindowSource::Observed);
    }

    #[test]
    fn test_base_model_id() {
        assert_eq!(base_model_id("claude-opus-5-5[1m]"), "claude-opus-5-5");
        assert_eq!(base_model_id("claude-opus-5-5[1M]"), "claude-opus-5-5");
        assert_eq!(base_model_id("claude-opus-5-5"), "claude-opus-5-5");
        assert_eq!(base_model_id(""), "");
    }

    #[test]
    fn test_notification_kinds_map_to_events() {
        assert_eq!(
            AgentEvent::from(NotificationKind::Idle),
            AgentEvent::IdleReminder
        );
        assert_eq!(
            AgentEvent::from(NotificationKind::TaskCompleted),
            AgentEvent::TurnCompleted { last_message: None }
        );
        assert_eq!(
            AgentEvent::from(NotificationKind::Other),
            AgentEvent::ApprovalRequested { tool: None }
        );
    }
}
