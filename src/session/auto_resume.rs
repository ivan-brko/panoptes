//! Resuming a session automatically once its usage limit resets
//!
//! A turn refused on a usage limit says when the limit lifts (Claude's
//! `quotaLimits`, Codex's "Try again at 3:37 PM."). Armed, the session is sent
//! [`CONTINUE_PROMPT`] a minute after that, so a long job picks up where it
//! stopped without anyone having to be at the keyboard at 03:10.
//!
//! Typing into a PTY nobody is watching is the whole risk, so this module is
//! mostly guards. It is pure - every decision takes the clock as an argument -
//! and [`step`] re-checks every one of them on every tick, not just once when
//! the session is armed:
//!
//! - only a genuine usage limit with a known reset arms ([`armable`]): never
//!   "rate limited", which passes in moments, nor an auth or server failure,
//!   which a prompt does not fix, nor a reset that is not after the failure;
//! - the session must still be sitting on *that* failure, idle at its prompt
//!   (`Waiting`), in the same conversation, run by the same process - a
//!   kill, a wake or a restart is a different process and disarms;
//! - any input the user sends it after the failure - keys, a paste, the
//!   mouse - disarms it, both at once (see
//!   [`crate::session::Session::note_user_input`]) and as a guard here, and
//!   input sent before arming refuses the arm: it may still be sitting
//!   half-typed in the prompt;
//! - a reset more than [`MAX_OVERDUE`] in the past - a laptop that slept
//!   through it - gives up rather than continuing a day-old task by surprise.
//!
//! A suspended session is never woken to be resumed. An armed session is kept
//! out of the idle-suspend sweep instead (`SessionManager::may_suspend`), so
//! the case does not arise on its own; should one be suspended anyway, it is
//! no longer idle at its prompt and simply disarms.
//!
//! The prompt and its `Enter` are written [`SUBMIT_DELAY`] apart. Both agents
//! treat a burst of characters ending in `\r` as a paste, and a pasted `\r` is
//! a newline in the prompt rather than a submission.

use chrono::{DateTime, Duration, Utc};

use super::{SessionInfo, SessionState, SessionType, TurnFailure};

/// What an armed session is sent when its limit lifts
pub const CONTINUE_PROMPT: &str = "Continue where you left off.";

/// The reason label every usage-limit failure carries, Claude's and Codex's
pub const USAGE_LIMIT: &str = "usage limit";

/// How long after the reset the prompt is sent
///
/// The reset is the provider's clock, not ours; a minute of slack keeps the
/// prompt from landing just before the window has actually reopened.
pub fn grace() -> Duration {
    Duration::minutes(1)
}

/// How far past its firing time an arm gives up instead of firing
pub const MAX_OVERDUE: Duration = Duration::hours(24);

/// The gap between typing the prompt and pressing `Enter`
pub const SUBMIT_DELAY: Duration = Duration::seconds(1);

/// A session armed to resume once its usage limit resets
///
/// Everything needed to tell, at firing time, that the session is still the
/// one that was armed: the failure (by its reset and when it happened), the
/// conversation and the process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeArm {
    /// When the limit lifts
    pub resets_at: DateTime<Utc>,
    /// When the failure armed against was recorded
    pub failed_at: DateTime<Utc>,
    /// The agent's conversation ID when armed
    pub conversation: Option<String>,
    /// The agent process's ID when armed
    pub pid: Option<u32>,
    /// When the arm was set
    pub armed_at: DateTime<Utc>,
    /// When the prompt was typed, once it has been - `Enter` is still to come
    pub typed_at: Option<DateTime<Utc>>,
}

impl ResumeArm {
    /// When the prompt is due
    pub fn fires_at(&self) -> DateTime<Utc> {
        self.resets_at + grace()
    }
}

/// Why a session cannot be armed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Not a Claude or Codex session
    WrongAgent,
    /// Not sitting on a failed turn
    NotFailed,
    /// Failed on something other than a usage limit
    NotUsageLimit,
    /// A usage limit, but the agent did not say when it resets
    NoReset,
    /// The reset reported is not after the failure - a stale reading
    ResetBeforeFailure,
    /// The limit has already lifted
    AlreadyReset,
    /// The user has sent the session input since it failed, which may still
    /// be sitting half-typed in its prompt
    UserTyped,
}

impl Refusal {
    /// Short text for the header notification
    pub fn describe(self) -> &'static str {
        match self {
            Refusal::WrongAgent => "Only Claude and Codex sessions can resume at reset",
            Refusal::NotFailed => "Nothing to resume: the session's turn did not fail",
            Refusal::NotUsageLimit => "Only a usage limit can be resumed at its reset",
            Refusal::NoReset => "The agent did not say when its limit resets",
            Refusal::ResetBeforeFailure => "The reported reset is stale",
            Refusal::AlreadyReset => "The limit has already reset - just continue",
            Refusal::UserTyped => "You have typed into it since it failed - resume it yourself",
        }
    }
}

/// The failure this session could be armed against: its reset and when it
/// happened
///
/// Antigravity is not offered: its failed turns carry no reset to wait for.
pub fn armable(info: &SessionInfo) -> Result<(DateTime<Utc>, DateTime<Utc>), Refusal> {
    if !matches!(
        info.session_type,
        SessionType::ClaudeCode | SessionType::OpenAICodex
    ) {
        return Err(Refusal::WrongAgent);
    }
    let failure = info.failed_turn().ok_or(Refusal::NotFailed)?;
    if failure.reason != USAGE_LIMIT {
        return Err(Refusal::NotUsageLimit);
    }
    let resets_at = failure
        .limit
        .as_ref()
        .and_then(|limit| limit.resets_at)
        .ok_or(Refusal::NoReset)?;
    let failed_at = failure.failed_at.ok_or(Refusal::NoReset)?;
    if resets_at <= failed_at {
        return Err(Refusal::ResetBeforeFailure);
    }
    if typed_since(info, failed_at) {
        return Err(Refusal::UserTyped);
    }
    Ok((resets_at, failed_at))
}

/// Arm a session against the failure it is sitting on
pub fn arm(info: &SessionInfo, pid: Option<u32>, now: DateTime<Utc>) -> Result<ResumeArm, Refusal> {
    let (resets_at, failed_at) = armable(info)?;
    let arm = ResumeArm {
        resets_at,
        failed_at,
        conversation: info.agent_session_id.clone(),
        pid,
        armed_at: now,
        typed_at: None,
    };
    // Arming after the fact would fire at once, which is a prompt the user
    // could simply have typed
    if arm.fires_at() <= now {
        return Err(Refusal::AlreadyReset);
    }
    Ok(arm)
}

/// What an armed session should do this tick
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Not yet
    Wait,
    /// Type [`CONTINUE_PROMPT`]
    Type,
    /// Press `Enter` on the prompt typed earlier
    Submit,
    /// Give up, without writing anything
    Disarm(Abandon),
}

/// Why an armed session was given up on
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Abandon {
    /// Its process is not the one armed: killed, woken, restarted
    Restarted,
    /// No longer idle at its prompt
    NotIdle,
    /// No longer sitting on the failure armed against
    NoLongerFailed,
    /// Now in a different conversation
    ConversationChanged,
    /// The user sent it input after it failed
    UserTyped,
    /// Its time passed long ago
    Overdue,
}

/// Decide what an armed session should do now, re-checking every guard
pub fn step(arm: &ResumeArm, info: &SessionInfo, pid: Option<u32>, now: DateTime<Utc>) -> Step {
    if pid.is_none() || pid != arm.pid {
        return Step::Disarm(Abandon::Restarted);
    }
    if info.state != SessionState::Waiting {
        return Step::Disarm(Abandon::NotIdle);
    }
    if !same_failure(arm, info.turn_failure.as_ref()) {
        return Step::Disarm(Abandon::NoLongerFailed);
    }
    if info.agent_session_id != arm.conversation {
        return Step::Disarm(Abandon::ConversationChanged);
    }
    if typed_since(info, arm.failed_at) {
        return Step::Disarm(Abandon::UserTyped);
    }
    match arm.typed_at {
        // Once typed, the `Enter` is owed whatever the clock says: leaving the
        // prompt typed and unsent would be the worse outcome
        Some(typed) if now >= typed + SUBMIT_DELAY => Step::Submit,
        Some(_) => Step::Wait,
        None if now > arm.fires_at() + MAX_OVERDUE => Step::Disarm(Abandon::Overdue),
        None if now >= arm.fires_at() => Step::Type,
        None => Step::Wait,
    }
}

/// Whether the user has sent the session input since `since`
///
/// Measured from the failure rather than from arming: the user may have
/// typed half a prompt, looked away and armed it from the pane, and the
/// continue prompt would be appended to whatever they left there. Nothing
/// says the prompt box is empty again short of a new turn, which is a new
/// failure to arm against.
fn typed_since(info: &SessionInfo, since: DateTime<Utc>) -> bool {
    info.last_user_input.is_some_and(|at| at > since)
}

/// Whether the session's failure is still the one the arm was set against
fn same_failure(arm: &ResumeArm, failure: Option<&TurnFailure>) -> bool {
    failure.is_some_and(|failure| {
        failure.reason == USAGE_LIMIT
            && failure.failed_at == Some(arm.failed_at)
            && failure.limit.as_ref().and_then(|limit| limit.resets_at) == Some(arm.resets_at)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::events::UsageLimit;
    use uuid::Uuid;

    const PID: Option<u32> = Some(4242);

    fn at(mins: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap() + Duration::minutes(mins)
    }

    /// A Claude session that failed at minute 0 on a limit resetting at 300
    fn failed_session(reason: &str, resets_at: Option<DateTime<Utc>>) -> SessionInfo {
        let mut info = SessionInfo::new(
            "s".to_string(),
            "/tmp".into(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        info.session_type = SessionType::ClaudeCode;
        info.state = SessionState::Waiting;
        info.agent_session_id = Some("conv-1".to_string());
        let limit = Some(UsageLimit {
            window: "five_hour".to_string(),
            resets_at,
            using_overage: false,
        });
        let mut failure = TurnFailure::new(Some(reason.to_string()), limit);
        failure.failed_at = Some(at(0));
        info.turn_failure = Some(failure);
        info
    }

    fn armed() -> (SessionInfo, ResumeArm) {
        let info = failed_session(USAGE_LIMIT, Some(at(300)));
        let arm = arm(&info, PID, at(1)).unwrap();
        (info, arm)
    }

    #[test]
    fn test_arms_on_a_usage_limit_with_a_reset() {
        let (_, arm) = armed();
        assert_eq!(arm.resets_at, at(300));
        assert_eq!(arm.fires_at(), at(301));
        assert_eq!(arm.conversation.as_deref(), Some("conv-1"));
    }

    #[test]
    fn test_codex_arms_too_but_not_antigravity_or_shells() {
        let mut info = failed_session(USAGE_LIMIT, Some(at(300)));
        info.session_type = SessionType::OpenAICodex;
        assert!(arm(&info, PID, at(1)).is_ok());
        for other in [SessionType::Antigravity, SessionType::Shell] {
            info.session_type = other;
            assert_eq!(arm(&info, PID, at(1)), Err(Refusal::WrongAgent));
        }
    }

    #[test]
    fn test_never_arms_on_anything_but_a_usage_limit() {
        for reason in ["rate limited", "auth failed", "server error"] {
            let info = failed_session(reason, Some(at(300)));
            assert_eq!(arm(&info, PID, at(1)), Err(Refusal::NotUsageLimit));
        }
    }

    #[test]
    fn test_never_arms_without_a_reset() {
        let info = failed_session(USAGE_LIMIT, None);
        assert_eq!(arm(&info, PID, at(1)), Err(Refusal::NoReset));
    }

    #[test]
    fn test_never_arms_when_the_reset_is_not_after_the_failure() {
        for reset in [at(0), at(-5)] {
            let info = failed_session(USAGE_LIMIT, Some(reset));
            assert_eq!(arm(&info, PID, at(1)), Err(Refusal::ResetBeforeFailure));
        }
    }

    #[test]
    fn test_never_arms_once_the_limit_has_lifted() {
        let info = failed_session(USAGE_LIMIT, Some(at(300)));
        assert_eq!(arm(&info, PID, at(301)), Err(Refusal::AlreadyReset));
    }

    #[test]
    fn test_never_arms_a_session_that_is_not_failed() {
        let mut info = failed_session(USAGE_LIMIT, Some(at(300)));
        info.state = SessionState::Thinking;
        assert_eq!(arm(&info, PID, at(1)), Err(Refusal::NotFailed));
    }

    #[test]
    fn test_waits_until_a_minute_after_the_reset_then_types_then_submits() {
        let (info, mut arm) = armed();
        assert_eq!(step(&arm, &info, PID, at(299)), Step::Wait);
        assert_eq!(step(&arm, &info, PID, at(300)), Step::Wait);
        assert_eq!(step(&arm, &info, PID, at(301)), Step::Type);

        arm.typed_at = Some(at(301));
        assert_eq!(step(&arm, &info, PID, at(301)), Step::Wait);
        assert_eq!(step(&arm, &info, PID, at(301) + SUBMIT_DELAY), Step::Submit);
    }

    #[test]
    fn test_gives_up_after_a_day_overdue() {
        let (info, arm) = armed();
        let deadline = arm.fires_at() + MAX_OVERDUE;
        assert_eq!(step(&arm, &info, PID, deadline), Step::Type);
        assert_eq!(
            step(&arm, &info, PID, deadline + Duration::seconds(1)),
            Step::Disarm(Abandon::Overdue)
        );
    }

    #[test]
    fn test_skips_a_session_no_longer_idle() {
        let (mut info, arm) = armed();
        for state in [
            SessionState::Thinking,
            SessionState::Executing,
            SessionState::AwaitingApproval,
            SessionState::Suspended,
            SessionState::Exited,
        ] {
            info.state = state;
            assert_eq!(
                step(&arm, &info, PID, at(301)),
                Step::Disarm(Abandon::NotIdle),
                "{state:?}"
            );
        }
    }

    #[test]
    fn test_skips_a_session_no_longer_on_that_failure() {
        let (mut info, arm) = armed();
        info.turn_failure = None;
        assert_eq!(
            step(&arm, &info, PID, at(301)),
            Step::Disarm(Abandon::NoLongerFailed)
        );

        // A later failure is a different one, even with the same reset
        let mut later = failed_session(USAGE_LIMIT, Some(at(300))).turn_failure;
        later.as_mut().unwrap().failed_at = Some(at(10));
        info.turn_failure = later;
        assert_eq!(
            step(&arm, &info, PID, at(301)),
            Step::Disarm(Abandon::NoLongerFailed)
        );
    }

    #[test]
    fn test_skips_a_session_the_user_typed_into() {
        let (mut info, arm) = armed();
        info.last_user_input = Some(at(-1));
        assert_eq!(step(&arm, &info, PID, at(301)), Step::Type);
        info.last_user_input = Some(at(2));
        assert_eq!(
            step(&arm, &info, PID, at(301)),
            Step::Disarm(Abandon::UserTyped)
        );
    }

    #[test]
    fn test_never_arms_a_session_typed_into_since_it_failed() {
        let mut info = failed_session(USAGE_LIMIT, Some(at(300)));
        // Input before the failure is the prompt that failed
        info.last_user_input = Some(at(-1));
        assert!(arm(&info, PID, at(1)).is_ok());
        // Input after it may still be sitting half-typed in the prompt
        info.last_user_input = Some(at(0) + Duration::seconds(5));
        assert_eq!(arm(&info, PID, at(1)), Err(Refusal::UserTyped));
    }

    #[test]
    fn test_skips_a_killed_or_restarted_session() {
        let (info, arm) = armed();
        assert_eq!(
            step(&arm, &info, Some(9999), at(301)),
            Step::Disarm(Abandon::Restarted)
        );
        assert_eq!(
            step(&arm, &info, None, at(301)),
            Step::Disarm(Abandon::Restarted)
        );
    }

    #[test]
    fn test_skips_a_session_now_in_another_conversation() {
        let (mut info, arm) = armed();
        info.agent_session_id = Some("conv-2".to_string());
        assert_eq!(
            step(&arm, &info, PID, at(301)),
            Step::Disarm(Abandon::ConversationChanged)
        );
    }

    #[test]
    fn test_a_typed_prompt_is_submitted_even_past_the_deadline() {
        let (info, mut arm) = armed();
        let late = arm.fires_at() + MAX_OVERDUE;
        arm.typed_at = Some(late);
        assert_eq!(
            step(&arm, &info, PID, late + Duration::hours(1)),
            Step::Submit
        );
    }
}
