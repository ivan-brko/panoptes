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

use chrono::{DateTime, Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::agent::prompt_cache::PromptCache;
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
        /// Which usage limit was hit and when it lifts, when the failure was
        /// a usage limit and the agent said which
        limit: Option<UsageLimit>,
    },

    /// A failed turn as Codex's rollout reports it
    ///
    /// Once admitted it means exactly [`AgentEvent::TurnFailed`]. It is kept
    /// apart because no hook reports a failed Codex turn, so the rollout's
    /// report gets past live hooks - and the rollout can be read after the
    /// hooks have moved on to the next turn. These fields are what decides
    /// whether it is still news (`state_machine::admits_transcript_event`).
    RolloutTurnFailed {
        /// As for [`AgentEvent::TurnFailed`]
        reason: Option<String>,
        /// As for [`AgentEvent::TurnFailed`]
        limit: Option<UsageLimit>,
        /// The turn that failed, Codex's `turn_id`, which its hooks also carry
        turn_id: Option<String>,
        /// Concluded from the rate-limit figures rather than written as an
        /// error, for a Codex too old to write one - and so never true of a
        /// Codex that has hooks
        inferred: bool,
        /// Whether the error leaves open that the turn was refused on its
        /// usage allowance (`other`, or retries given up on a 429), so the
        /// rate-limit figures decide
        open_to_usage_limit: bool,
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

    /// The same figures, re-reported on the agent's own schedule
    ///
    /// Claude's status line reruns at startup, after turns, on a mode or
    /// model switch, and on timers - when a rate-limit window resets, when the
    /// prompt cache expires, every `refreshInterval` seconds. Most of those
    /// fire while nobody is doing anything, so unlike [`AgentEvent::Usage`]
    /// this must not count as activity or idle suspension would never fire.
    UsageRefresh(UsageSnapshot),

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
/// Every field is optional because sources report different subsets. Codex's
/// rollout carries rate limits but no model; a Claude transcript record names
/// the model but carries no rate limits, which only its status line reports.
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

    /// The short rate-limit window, five hours on current plans
    #[serde(default)]
    pub primary: Option<RateLimitWindow>,

    /// The long rate-limit window, a week on current plans
    ///
    /// Tracked separately because it is routinely the one that bites: a quiet
    /// morning can leave the five-hour window at 1% while the week sits at 40%.
    #[serde(default)]
    pub secondary: Option<RateLimitWindow>,

    /// A workspace member's own credit allowance (Codex only)
    ///
    /// Codex writes it as `individual_limit` beside the two windows, as a
    /// remaining share rather than a used one; it is held here converted, with
    /// no length, since it is a spend cap rather than a rolling window.
    #[serde(default)]
    pub individual: Option<RateLimitWindow>,

    /// Whether the account can keep working past a used-up window, on
    /// purchased or unlimited credits (Codex only: `credits.has_credits` or
    /// `credits.unlimited`)
    #[serde(default)]
    pub credits_available: Option<bool>,

    /// Plan name backing the rate limit (Codex only)
    #[serde(default)]
    pub plan: Option<String>,

    /// Why the agent has stopped accepting turns, when it has (Codex only)
    ///
    /// The agent's own reason string, e.g. `workspace_member_credits_depleted`.
    /// Only its presence is shown; the wording is Codex's and may change.
    #[serde(default)]
    pub limit_reached: Option<String>,

    /// The last request that used the prompt cache, and when it expires
    /// (Claude only, from the transcript's `usage.cache_creation` split)
    #[serde(default)]
    pub prompt_cache: Option<PromptCache>,
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
    pub(crate) fn label(&self) -> String {
        self.label_or(None)
    }

    /// [`Self::label`], with a length to assume when the agent wrote none
    fn label_or(&self, fallback_minutes: Option<u64>) -> String {
        match self.window_minutes.filter(|&m| m > 0).or(fallback_minutes) {
            None | Some(0) => "limit".to_string(),
            Some(WEEK_MINUTES) => "wk".to_string(),
            Some(MONTH_MINUTES) => "mo".to_string(),
            Some(m) if m % 1_440 == 0 => format!("{}d", m / 1_440),
            Some(m) if m % 60 == 0 => format!("{}h", m / 60),
            Some(m) => format!("{}m", m),
        }
    }
}

/// A five-hour window, the short window on paid Codex plans
const SESSION_MINUTES: u64 = 5 * 60;
/// A week, the long window
const WEEK_MINUTES: u64 = 7 * 24 * 60;
/// Thirty days, the only window on Codex's Free and Go plans
const MONTH_MINUTES: u64 = 30 * 24 * 60;

/// Which of a snapshot's rate limits a window is
///
/// Codex's `primary` and `secondary` are positions, not lengths: a record that
/// leaves out `window_minutes` still means the same windows, and which ones
/// depends on the plan (see [`UsageSnapshot::assumed_minutes`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Primary,
    Secondary,
    Individual,
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
        let newer_has_limits = newer.carries_limits();
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
        merge_window(&mut self.individual, newer.individual);
        if newer.plan.is_some() {
            self.plan = newer.plan;
        }
        // A record carrying rate limits is authoritative about whether the limit
        // is hit, and Codex writes an explicit null once it is lifted. Merging
        // this like the other fields would leave "limit hit" up forever.
        if newer_has_limits {
            self.limit_reached = newer.limit_reached;
        }
        if newer.credits_available.is_some() {
            self.credits_available = newer.credits_available;
        }
        match (self.prompt_cache.as_mut(), newer.prompt_cache) {
            (Some(known), Some(newer)) => known.merge(newer),
            (None, Some(newer)) => self.prompt_cache = Some(newer),
            (_, None) => {}
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

    /// [`Self::summary`] against a given clock, so the reset time is testable
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
        let binding = self.binding_slot();
        if self.limit_reached.is_some() {
            // Blocked outright - how full the window is no longer matters, only
            // when it lets the user back in
            match binding
                .and_then(|(_, w)| w.resets_at)
                .and_then(|at| format_reset(at, now))
            {
                Some(reset) => parts.push(format!("limit hit · resets {}", reset)),
                None => parts.push("limit hit".to_string()),
            }
        } else if let Some((slot, window)) = binding {
            parts.push(format!(
                "{} {:.0}%",
                self.slot_label(slot, window),
                window.used_percent
            ));
        }

        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// The window closest to stopping the user, which is the one worth showing
    ///
    /// Higher usage wins; on a tie the longer window does, since it takes
    /// longer to recover from.
    pub(crate) fn binding_window(&self) -> Option<&RateLimitWindow> {
        self.binding_slot().map(|(_, window)| window)
    }

    /// [`Self::binding_window`], with which of the limits it is
    fn binding_slot(&self) -> Option<(Slot, &RateLimitWindow)> {
        let mut best: Option<(Slot, &RateLimitWindow)> = None;
        for (slot, window) in self.slots() {
            let wins = match best {
                None => true,
                Some((held_slot, held)) => {
                    (window.used_percent, self.minutes_of(slot, window))
                        > (held.used_percent, self.minutes_of(held_slot, held))
                }
            };
            if wins {
                best = Some((slot, window));
            }
        }
        best
    }

    /// Whether this snapshot reads the rate limits at all
    ///
    /// A record that does is authoritative about every limit field, including
    /// the ones it leaves empty.
    pub(crate) fn carries_limits(&self) -> bool {
        self.primary.is_some()
            || self.secondary.is_some()
            || self.individual.is_some()
            || self.limit_reached.is_some()
    }

    /// Whether these figures say the agent is refusing turns: it says so
    /// outright, or a limit is used up with nothing to carry on with
    ///
    /// A used-up window alone is not a refusal while credits are available:
    /// Plus and Pro accounts keep working past 100% on purchased credits. The
    /// credit allowance itself has nothing behind it, so it always counts.
    pub fn is_limited(&self) -> bool {
        self.limit_reached.is_some()
            || self.slots().any(|(slot, w)| {
                w.used_percent >= 100.0
                    && (slot == Slot::Individual || self.credits_available != Some(true))
            })
    }

    /// The limit standing between the user and their next turn, if any
    ///
    /// Every exhausted limit has to lift before a turn can run, so this is the
    /// one that lifts *last*. An exhausted limit whose reset is unknown could
    /// lift later than any known one, so it wins and the reset stays unknown,
    /// rather than promising a time that may be too early. With nothing used
    /// up, a refusal the agent names anyway falls on the credit allowance when
    /// it is a workspace's refusal (`workspace_*`: out of credits, or a spend
    /// cap hit) and there is one, and on the binding window otherwise - a plain
    /// `rate_limit_reached` at 99.6% is the window's, however little of the
    /// allowance is used.
    ///
    /// The window is this snapshot's own label (`5h`, `wk`, `credits`), which
    /// [`UsageLimit::window_label`] passes through as it is.
    pub fn exhausted_limit(&self) -> Option<UsageLimit> {
        let exhausted = self
            .slots()
            .filter(|(_, w)| w.used_percent >= 100.0)
            .max_by_key(|(_, w)| w.resets_at.unwrap_or(DateTime::<Utc>::MAX_UTC));
        let (slot, window) = match exhausted {
            Some(found) => found,
            None => {
                let workspace = self.limit_reached.as_ref()?.starts_with("workspace_");
                match &self.individual {
                    Some(credits) if workspace => (Slot::Individual, credits),
                    _ => self.binding_slot()?,
                }
            }
        };
        Some(UsageLimit {
            window: self.slot_label(slot, window),
            resets_at: window.resets_at,
            using_overage: false,
        })
    }

    /// Every limit the snapshot holds, in slot order
    fn slots(&self) -> impl Iterator<Item = (Slot, &RateLimitWindow)> {
        [
            (Slot::Primary, &self.primary),
            (Slot::Secondary, &self.secondary),
            (Slot::Individual, &self.individual),
        ]
        .into_iter()
        .filter_map(|(slot, window)| window.as_ref().map(|w| (slot, w)))
    }

    /// The length a slot's window is taken to have when the agent wrote none
    ///
    /// Codex's primary window is a month on the Free and Go plans and five
    /// hours on every paid one; the secondary is always the week. With no plan
    /// known there is nothing to go on for the primary, and it stays
    /// unlabelled rather than guessed at.
    fn assumed_minutes(&self, slot: Slot) -> Option<u64> {
        match slot {
            Slot::Primary => self.plan.as_deref().map(|plan| match plan {
                "free" | "go" => MONTH_MINUTES,
                _ => SESSION_MINUTES,
            }),
            Slot::Secondary => Some(WEEK_MINUTES),
            Slot::Individual => None,
        }
    }

    /// How long a window is, for ranking: its own figure, else the assumed one
    ///
    /// The credit allowance ranks as monthly, which is how Codex bills it.
    fn minutes_of(&self, slot: Slot, window: &RateLimitWindow) -> u64 {
        match slot {
            Slot::Individual => MONTH_MINUTES,
            _ => window
                .window_minutes
                .filter(|&m| m > 0)
                .or(self.assumed_minutes(slot))
                .unwrap_or(0),
        }
    }

    /// A window's label in this snapshot: `5h`, `wk`, `credits`
    fn slot_label(&self, slot: Slot, window: &RateLimitWindow) -> String {
        match slot {
            Slot::Individual => "credits".to_string(),
            _ => window.label_or(self.assumed_minutes(slot)),
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

/// Render when a limit lifts, on the user's own clock: `20:10` today,
/// `Aug 29 01:30` on any other day
///
/// The one reset formatter: the header's `limit hit`, a failed turn's
/// `5h resets 20:10` and Panoptes' default status line all use it, so none of
/// them disagree about the same reset. A clock time rather than a countdown
/// because it does not go stale: "resets in 2h" read an hour later is wrong,
/// where "resets 20:10" is not - and Claude redraws its status line on
/// events, not on a clock.
///
/// `None` once the reset has passed. Every caller then leaves the reset out:
/// a time that no longer means anything is not worth the room.
pub fn format_reset(at: DateTime<Utc>, now: DateTime<Utc>) -> Option<String> {
    format_reset_in(at, now, &Local)
}

/// [`format_reset`] in a given time zone, so midnight is testable
pub(crate) fn format_reset_in<Tz: TimeZone>(
    at: DateTime<Utc>,
    now: DateTime<Utc>,
    tz: &Tz,
) -> Option<String>
where
    Tz::Offset: std::fmt::Display,
{
    if at <= now {
        return None;
    }
    let (at, now) = (at.with_timezone(tz), now.with_timezone(tz));
    Some(if at.date_naive() == now.date_naive() {
        at.format("%H:%M").to_string()
    } else {
        at.format("%b %-d %H:%M").to_string()
    })
}

/// A usage limit the agent refused a turn on: which window, and when it lifts
///
/// Claude writes this as `quotaLimits` on the failed turn's transcript record
/// (`rateLimitType`, `resetsAt` in epoch seconds, `isUsingOverage`). Codex
/// names no window on a refused turn, so its limits are read off the rate-limit
/// figures instead (see [`UsageSnapshot::exhausted_limit`]) and carry those
/// figures' own labels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageLimit {
    /// The window as the agent names it, e.g. `five_hour`, `seven_day_opus`,
    /// or Codex's `5h` / `credits`
    ///
    /// Empty when only the reset is known - Codex's error text says when, not
    /// which. A limit is never built with neither.
    pub window: String,
    /// When the window's allowance is restored, if the agent said
    pub resets_at: Option<DateTime<Utc>>,
    /// Whether the turn was already drawing on paid overage when refused
    ///
    /// Carried for completeness; nothing renders it yet.
    pub using_overage: bool,
}

impl UsageLimit {
    /// Short label for the window, in the header's vocabulary (`5h`, `wk`)
    ///
    /// The names are Claude's `rateLimitType` values as of Claude Code
    /// 2.1.330 (read from the binary). A window this table does not know is
    /// shown by its raw name: one nobody has prettied still tells the user
    /// more than a generic "limit".
    pub fn window_label(&self) -> &str {
        match self.window.as_str() {
            "five_hour" => "5h",
            "seven_day" => "wk",
            "seven_day_opus" => "wk opus",
            "seven_day_sonnet" => "wk sonnet",
            "seven_day_overage_included" => "wk overage",
            "overage" => "overage",
            other => other,
        }
    }

    /// `5h resets 20:10`, or just `5h` when the reset is unknown or past
    ///
    /// A limit with no window reads `resets 20:10`, and once that has passed,
    /// the generic `limit`.
    pub fn describe_at(&self, now: DateTime<Utc>) -> String {
        let label = match self.window_label() {
            "" => None,
            label => Some(label),
        };
        match (label, self.resets_at.and_then(|at| format_reset(at, now))) {
            (Some(label), Some(reset)) => format!("{} resets {}", label, reset),
            (None, Some(reset)) => format!("resets {}", reset),
            (Some(label), None) => label.to_string(),
            (None, None) => "limit".to_string(),
        }
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
pub(crate) fn base_model_id(model: &str) -> &str {
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
    fn test_summary_limit_reached_shows_reset_time() {
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
        // On the user's clock, through the same formatter a failed turn uses
        let reset = format_reset(now + chrono::Duration::minutes(200), now).unwrap();
        assert_eq!(
            usage.summary_at(now),
            Some(format!("gpt-5.5 · limit hit · resets {reset}"))
        );

        // Without a reset time the block is still shown, just without one
        let usage = UsageSnapshot {
            primary: Some(window(100.0, Some(300), None)),
            limit_reached: Some("x".to_string()),
            ..Default::default()
        };
        assert_eq!(usage.summary_at(now).as_deref(), Some("limit hit"));

        // Likewise once the reset has passed: no time that means nothing
        let usage = UsageSnapshot {
            primary: Some(window(
                100.0,
                Some(300),
                Some(now - chrono::Duration::minutes(1)),
            )),
            limit_reached: Some("x".to_string()),
            ..Default::default()
        };
        assert_eq!(usage.summary_at(now).as_deref(), Some("limit hit"));
    }

    #[test]
    fn test_format_reset() {
        // 2026-08-28 17:59:57 UTC, which is 19:59:57 in Zagreb (UTC+2)
        let now = at(1_787_939_997);
        let zagreb = chrono::FixedOffset::east_opt(2 * 3_600).unwrap();
        let later = |secs| now + chrono::Duration::seconds(secs);

        // Today: just the clock time, in the zone's own hours
        assert_eq!(
            format_reset_in(later(603), now, &zagreb),
            Some("20:10".to_string())
        );
        assert_eq!(
            format_reset_in(later(603), now, &Utc),
            Some("18:10".to_string())
        );
        assert_eq!(
            format_reset_in(later(30), now, &zagreb),
            Some("20:00".to_string())
        );

        // Past midnight it is another day, and says which
        assert_eq!(
            format_reset_in(later(4 * 3_600 + 603), now, &zagreb),
            Some("Aug 29 00:10".to_string())
        );
        // ...but only where midnight has actually passed: the same instant is
        // still the 28th in UTC
        assert_eq!(
            format_reset_in(later(4 * 3_600 + 603), now, &Utc),
            Some("22:10".to_string())
        );
        assert_eq!(
            format_reset_in(later(6 * 24 * 3_600), now, &zagreb),
            Some("Sep 3 19:59".to_string())
        );

        // A reset already past has nothing left to say
        assert_eq!(format_reset_in(later(0), now, &zagreb), None);
        assert_eq!(format_reset_in(later(-60), now, &zagreb), None);
    }

    #[test]
    fn test_usage_limit_window_labels() {
        let limit = |window: &str| UsageLimit {
            window: window.to_string(),
            resets_at: None,
            using_overage: false,
        };
        for (window, label) in [
            ("five_hour", "5h"),
            ("seven_day", "wk"),
            ("seven_day_opus", "wk opus"),
            ("seven_day_sonnet", "wk sonnet"),
            ("seven_day_overage_included", "wk overage"),
            ("overage", "overage"),
            // A window this was written before keeps its own name
            ("seven_day_cowork", "seven_day_cowork"),
        ] {
            assert_eq!(limit(window).window_label(), label, "for {window}");
        }

        // No reset time: the window alone
        assert_eq!(limit("five_hour").describe_at(at(0)), "5h");
        let now = at(1_787_939_997);
        let with_reset = UsageLimit {
            resets_at: Some(at(1_787_940_600)),
            ..limit("five_hour")
        };
        assert_eq!(
            with_reset.describe_at(now),
            format!(
                "5h resets {}",
                format_reset(at(1_787_940_600), now).unwrap()
            )
        );
        // Past its reset the window still names itself, and only that
        assert_eq!(with_reset.describe_at(at(1_787_940_600)), "5h");
    }

    #[test]
    fn test_window_label() {
        assert_eq!(window(0.0, Some(300), None).label(), "5h");
        assert_eq!(window(0.0, Some(10_080), None).label(), "wk");
        assert_eq!(window(0.0, Some(1_440), None).label(), "1d");
        assert_eq!(window(0.0, Some(90), None).label(), "90m");
        assert_eq!(window(0.0, None, None).label(), "limit");
        assert_eq!(window(0.0, Some(43_200), None).label(), "mo");
    }

    #[test]
    fn test_window_without_minutes_is_labelled_by_plan() {
        let exhausted = |plan: Option<&str>| UsageSnapshot {
            primary: Some(window(100.0, None, Some(at(5_000)))),
            plan: plan.map(str::to_string),
            ..Default::default()
        };
        // Codex's primary is a month on Free and Go, five hours on paid plans
        for (plan, label) in [
            (Some("free"), "mo"),
            (Some("go"), "mo"),
            (Some("plus"), "5h"),
            (Some("business"), "5h"),
            // With no plan there is nothing to go on
            (None, "limit"),
        ] {
            let usage = exhausted(plan);
            assert_eq!(
                usage.exhausted_limit().map(|l| l.window).as_deref(),
                Some(label),
                "for {plan:?}"
            );
            assert_eq!(usage.summary_at(at(0)).unwrap(), format!("{label} 100%"));
        }

        // The secondary is the week whatever the plan
        let usage = UsageSnapshot {
            secondary: Some(window(100.0, None, None)),
            ..Default::default()
        };
        assert_eq!(usage.exhausted_limit().unwrap().window, "wk");
    }

    #[test]
    fn test_exhausted_limit_is_the_last_to_lift() {
        let both = UsageSnapshot {
            primary: Some(window(100.0, Some(300), Some(at(1_000)))),
            secondary: Some(window(100.0, Some(10_080), Some(at(9_000)))),
            ..Default::default()
        };
        let limit = both.exhausted_limit().unwrap();
        assert_eq!(
            (limit.window.as_str(), limit.resets_at),
            ("wk", Some(at(9_000)))
        );

        // Order does not matter, only the reset
        let reversed = UsageSnapshot {
            primary: Some(window(100.0, Some(300), Some(at(9_000)))),
            secondary: Some(window(100.0, Some(10_080), Some(at(1_000)))),
            ..Default::default()
        };
        assert_eq!(reversed.exhausted_limit().unwrap().window, "5h");

        // A used-up window with no known reset could lift later than any
        // known one, so no time is promised
        let unknown = UsageSnapshot {
            primary: Some(window(100.0, Some(300), None)),
            secondary: Some(window(100.0, Some(10_080), Some(at(9_000)))),
            ..Default::default()
        };
        let limit = unknown.exhausted_limit().unwrap();
        assert_eq!((limit.window.as_str(), limit.resets_at), ("5h", None));

        // A window short of full is not what blocks the turn
        let one = UsageSnapshot {
            primary: Some(window(99.5, Some(300), Some(at(1_000)))),
            secondary: Some(window(100.0, Some(10_080), Some(at(9_000)))),
            ..Default::default()
        };
        assert_eq!(one.exhausted_limit().unwrap().window, "wk");

        // Nothing used up and no refusal named: nothing is in the way
        let fine = UsageSnapshot {
            primary: Some(window(60.0, Some(300), Some(at(1_000)))),
            ..Default::default()
        };
        assert_eq!(fine.exhausted_limit(), None);
        assert!(!fine.is_limited());

        // A named refusal with nothing used up falls on the binding window
        let named = UsageSnapshot {
            limit_reached: Some("rate_limit_reached".to_string()),
            ..fine
        };
        assert!(named.is_limited());
        assert_eq!(named.exhausted_limit().unwrap().window, "5h");
    }

    #[test]
    fn test_named_refusal_falls_on_credits_only_for_a_workspace() {
        let snapshot = |reached: &str| UsageSnapshot {
            primary: Some(window(99.6, Some(300), Some(at(1_000)))),
            individual: Some(window(20.0, None, Some(at(9_000)))),
            limit_reached: Some(reached.to_string()),
            ..Default::default()
        };
        // A plain rate limit just short of a full window is the window's,
        // however little of the credit allowance is used
        assert_eq!(
            snapshot("rate_limit_reached")
                .exhausted_limit()
                .unwrap()
                .window,
            "5h"
        );
        // A workspace's refusal is about its credits or spend cap
        for reached in [
            "workspace_member_credits_depleted",
            "workspace_owner_usage_limit_reached",
        ] {
            let limit = snapshot(reached).exhausted_limit().unwrap();
            assert_eq!(
                (limit.window.as_str(), limit.resets_at),
                ("credits", Some(at(9_000))),
                "for {reached}"
            );
        }
    }

    #[test]
    fn test_full_window_on_credits_is_not_limited() {
        let full = |credits: Option<bool>| UsageSnapshot {
            primary: Some(window(100.0, Some(300), None)),
            credits_available: credits,
            ..Default::default()
        };
        assert!(full(None).is_limited());
        assert!(full(Some(false)).is_limited());
        // Plus and Pro keep working on purchased credits
        assert!(!full(Some(true)).is_limited());
        // ...unless Codex says it is refusing anyway
        let refused = UsageSnapshot {
            limit_reached: Some("rate_limit_reached".to_string()),
            ..full(Some(true))
        };
        assert!(refused.is_limited());
        // The credit allowance has nothing behind it
        let spent = UsageSnapshot {
            individual: Some(window(100.0, None, None)),
            credits_available: Some(true),
            ..Default::default()
        };
        assert!(spent.is_limited());
    }

    #[test]
    fn test_usage_limit_without_a_window_names_only_its_reset() {
        let now = at(1_787_939_997);
        let reset = at(1_787_940_600);
        let limit = UsageLimit {
            window: String::new(),
            resets_at: Some(reset),
            using_overage: false,
        };
        assert_eq!(
            limit.describe_at(now),
            format!("resets {}", format_reset(reset, now).unwrap())
        );
        // Once past, there is nothing specific left to say
        assert_eq!(limit.describe_at(reset), "limit");
    }

    #[test]
    fn test_merge_keeps_the_credit_allowance_alongside_the_windows() {
        let mut usage = UsageSnapshot {
            individual: Some(window(70.0, None, Some(at(9_000)))),
            ..Default::default()
        };
        usage.merge(UsageSnapshot {
            primary: Some(window(5.0, Some(300), None)),
            ..Default::default()
        });
        assert_eq!(usage.individual, Some(window(70.0, None, Some(at(9_000)))));
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
