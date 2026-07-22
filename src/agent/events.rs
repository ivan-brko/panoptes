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

    /// The agent is blocked waiting for the user to approve something
    ApprovalRequested {
        /// The tool awaiting approval, when the agent names one
        tool: Option<String>,
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

    /// Model currently serving the conversation
    #[serde(default)]
    pub model: Option<String>,

    /// Percentage of the plan's rate limit consumed (Codex only)
    #[serde(default)]
    pub rate_limit_used_percent: Option<f64>,

    /// When the rate-limit window resets, as the agent reported it (Codex only)
    #[serde(default)]
    pub rate_limit_resets_at: Option<String>,

    /// Plan name backing the rate limit (Codex only)
    #[serde(default)]
    pub plan: Option<String>,
}

impl UsageSnapshot {
    /// Whether this snapshot carries anything worth showing
    pub fn is_empty(&self) -> bool {
        *self == UsageSnapshot::default()
    }

    /// Fold newer figures in, keeping known values the update does not mention
    ///
    /// Sources are partial and interleaved: a Claude assistant record names the
    /// model and its token counts but never a context window, while a Codex
    /// `token_count` carries limits but no model. Overwriting wholesale would
    /// make fields flicker between present and absent.
    pub fn merge(&mut self, newer: UsageSnapshot) {
        if newer.total_tokens.is_some() {
            self.total_tokens = newer.total_tokens;
        }
        if newer.context_window.is_some() {
            self.context_window = newer.context_window;
        }
        if newer.model.is_some() {
            self.model = newer.model;
        }
        if newer.rate_limit_used_percent.is_some() {
            self.rate_limit_used_percent = newer.rate_limit_used_percent;
        }
        if newer.rate_limit_resets_at.is_some() {
            self.rate_limit_resets_at = newer.rate_limit_resets_at;
        }
        if newer.plan.is_some() {
            self.plan = newer.plan;
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
    /// Reads like `opus-4.8 · ctx 34% · limit 12%`. Rate limit is omitted rather
    /// than shown as zero when the agent does not report one, because "we do not
    /// know" and "you have used none of it" are different claims.
    pub fn summary(&self) -> Option<String> {
        let mut parts = Vec::new();

        if let Some(model) = &self.model {
            parts.push(short_model_name(model).to_string());
        }
        if let Some(pct) = self.context_percent() {
            parts.push(format!("ctx {:.0}%", pct));
        } else if let Some(total) = self.total_tokens {
            parts.push(format!("{} tok", format_thousands(total)));
        }
        if let Some(pct) = self.rate_limit_used_percent {
            parts.push(format!("limit {:.0}%", pct));
        }

        (!parts.is_empty()).then(|| parts.join(" · "))
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
            rate_limit_used_percent: Some(12.4),
            ..Default::default()
        };
        assert_eq!(
            codex.summary().as_deref(),
            Some("gpt-5-codex · ctx 34% · limit 12%")
        );

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
