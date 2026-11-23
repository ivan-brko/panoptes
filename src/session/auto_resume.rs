//! Resuming a session automatically once its usage limit resets
//!
//! A turn refused on a usage limit says when the limit lifts (Claude's
//! `quotaLimits`, Codex's "Try again at 3:37 PM."). Armed, the session is sent
//! [`CONTINUE_PROMPT`] a minute after that, so a long job picks up where it
//! stopped without anyone having to be at the keyboard at 03:10.
//!
//! Typing into a PTY nobody is watching is the whole risk, so this module is
//! mostly guards. It is pure - every decision takes the clock, and the screen,
//! as an argument - and [`step`] re-checks every guard on every tick, not just
//! once when the session is armed:
//!
//! - only a genuine usage limit with a known reset arms ([`armable`]): never
//!   "rate limited", which passes in moments, nor an auth or server failure,
//!   which a prompt does not fix, nor a reset that is not after the failure;
//!   and never one more than [`MAX_LEAD`] away - a weekly limit's reset days
//!   out is not a moment anyone expects their session to start talking;
//! - the session must still be sitting on *that* failure, idle at its prompt
//!   (`Waiting`), in the same conversation, run by the same process - a
//!   kill, a wake or a restart is a different process and disarms;
//! - no input from the user since the failed turn began: a draft typed while
//!   it ran, or after it failed, may still be sitting in the prompt. Input
//!   before arming refuses the arm, and input after it disarms at once (see
//!   [`crate::session::Session::note_user_input`]) as well as here;
//! - the screen has to agree ([`read_prompt`]): before typing, the cursor's
//!   line must be the agent's own prompt with nothing typed on it and no menu
//!   open beneath it - Claude's rate-limit options and Codex's model-switch
//!   popup both turn up on a usage limit, and typed text there would choose an
//!   option. Before `Enter`, the line must hold exactly the prompt typed.
//!   Anything not positively recognised disarms, with a notice;
//! - more than [`MAX_OVERDUE`] late - a laptop that slept through it - gives
//!   up rather than continuing by surprise; and coming back from sleep holds
//!   any resume for [`WAKE_HOLD`] (see [`slept`]), so the user who just opened
//!   the lid is not typed over.
//!
//! A suspended session is never woken to be resumed. An armed session is kept
//! out of the idle-suspend sweep instead (`SessionManager::may_suspend`), so
//! the case does not arise on its own; should one be suspended anyway, it is
//! no longer idle at its prompt and simply disarms.
//!
//! The prompt and its `Enter` are written [`SUBMIT_DELAY`] apart. Both agents
//! treat a burst of characters ending in `\r` as a paste, and a pasted `\r` is
//! a newline in the prompt rather than a submission. Sessions due in the same
//! tick are typed [`STAGGER`] apart, so a dozen sessions on one account do not
//! all hit the freshly reset limit in the same second.

use std::time::Instant;

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

/// How far out a reset may be and still arm
///
/// A five-hour window always fits; a weekly one days away does not. Coming
/// back to a session that started by itself days later is a surprise, not a
/// convenience.
pub const MAX_LEAD: Duration = Duration::hours(6);

/// How far past its firing time an arm gives up instead of firing
pub const MAX_OVERDUE: Duration = Duration::minutes(30);

/// The gap between typing the prompt and pressing `Enter`
pub const SUBMIT_DELAY: Duration = Duration::seconds(1);

/// The gap between sessions due in the same tick
pub const STAGGER: Duration = Duration::seconds(5);

/// How long a resume waits after the machine wakes from sleep
pub const WAKE_HOLD: Duration = Duration::seconds(60);

/// How far the wall clock may run ahead of the monotonic one between two ticks
/// before it counts as the machine having slept
pub const SLEEP_JUMP: Duration = Duration::seconds(30);

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
    /// Since when the user must not have sent input: the start of the failed
    /// turn, where it is known, and the failure otherwise
    pub quiet_since: DateTime<Utc>,
    /// The agent's conversation ID when armed
    pub conversation: Option<String>,
    /// The agent process's ID when armed
    pub pid: Option<u32>,
    /// When the arm was set
    pub armed_at: DateTime<Utc>,
    /// Not before this, even once due: staggered behind another session, or
    /// held after a wake from sleep
    pub hold_until: Option<DateTime<Utc>>,
    /// When the prompt was typed, once it has been - `Enter` is still to come
    pub typed_at: Option<DateTime<Utc>>,
}

impl ResumeArm {
    /// When the prompt is due
    pub fn fires_at(&self) -> DateTime<Utc> {
        self.resets_at + grace()
    }

    /// Push the arm back to at least `until`
    pub fn hold(&mut self, until: DateTime<Utc>) {
        self.hold_until = Some(self.hold_until.map_or(until, |held| held.max(until)));
    }
}

/// Why a session cannot be armed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// There is no such live session
    NoSuchSession,
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
    /// The reset is further out than [`MAX_LEAD`]
    TooFarOut(DateTime<Utc>),
    /// The user has sent the session input since the failed turn began, which
    /// may still be sitting half-typed in its prompt
    UserTyped,
}

impl Refusal {
    /// Short text for the header notification
    pub fn describe(self, now: DateTime<Utc>) -> String {
        match self {
            Refusal::NoSuchSession => "That session is not running".to_string(),
            Refusal::WrongAgent => "Only Claude and Codex sessions can resume at reset".to_string(),
            Refusal::NotFailed => "Nothing to resume: the session's turn did not fail".to_string(),
            Refusal::NotUsageLimit => "Only a usage limit can be resumed at its reset".to_string(),
            Refusal::NoReset => "The agent did not say when its limit resets".to_string(),
            Refusal::ResetBeforeFailure => "The reported reset is stale".to_string(),
            Refusal::AlreadyReset => "The limit has already reset - just continue".to_string(),
            Refusal::TooFarOut(resets_at) => format!(
                "Resets in {}; won't auto-resume that far out",
                how_far(resets_at - now)
            ),
            Refusal::UserTyped => {
                "You have typed into it since the turn began - resume it yourself".to_string()
            }
        }
    }
}

/// `3 days`, `9h`, rounded down
fn how_far(lead: Duration) -> String {
    match lead.num_hours() {
        hours if hours >= 48 => format!("{} days", hours / 24),
        hours => format!("{}h", hours),
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
    if typed_since(info, quiet_since(info, failed_at)) {
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
        quiet_since: quiet_since(info, failed_at),
        conversation: info.agent_session_id.clone(),
        pid,
        armed_at: now,
        hold_until: None,
        typed_at: None,
    };
    // Arming after the fact would fire at once, which is a prompt the user
    // could simply have typed
    if arm.fires_at() <= now {
        return Err(Refusal::AlreadyReset);
    }
    if resets_at - now > MAX_LEAD {
        return Err(Refusal::TooFarOut(resets_at));
    }
    Ok(arm)
}

/// Since when the session must have had no input: the start of the turn
/// that failed, so a draft typed while it ran counts, or the failure itself
/// when the turn's start was not seen
fn quiet_since(info: &SessionInfo, failed_at: DateTime<Utc>) -> DateTime<Utc> {
    info.turn_started_at
        .filter(|started| *started <= failed_at)
        .unwrap_or(failed_at)
}

/// What an armed session should do this tick
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Not yet
    Wait,
    /// Type [`CONTINUE_PROMPT`], once the screen agrees ([`confirm`])
    Type,
    /// Press `Enter` on the prompt typed earlier, once the screen agrees
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
    /// The user sent it input since the failed turn began
    UserTyped,
    /// Its time passed too long ago
    Overdue,
    /// The prompt already holds text the user typed
    PromptNotEmpty,
    /// The screen is not a prompt Panoptes can recognise: a menu, a popup, a
    /// hidden cursor
    PromptUnrecognised(&'static str),
    /// The line no longer holds exactly the prompt typed
    PromptChanged,
}

impl Abandon {
    /// What to tell the user, for the cases they did not cause themselves
    pub fn notice(self) -> Option<String> {
        Some(match self {
            Abandon::Overdue => "its reset passed too long ago".to_string(),
            Abandon::PromptNotEmpty => "its prompt already has text in it".to_string(),
            Abandon::PromptUnrecognised(why) => {
                format!("could not confirm an empty prompt ({why})")
            }
            Abandon::PromptChanged => {
                "the prompt changed before it was sent; it was left unsent".to_string()
            }
            Abandon::Restarted
            | Abandon::NotIdle
            | Abandon::NoLongerFailed
            | Abandon::ConversationChanged
            | Abandon::UserTyped => return None,
        })
    }
}

/// Decide what an armed session should do now, re-checking every guard
///
/// `Type` and `Submit` still have to pass [`confirm`] against the screen.
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
    if typed_since(info, arm.quiet_since) {
        return Step::Disarm(Abandon::UserTyped);
    }
    match arm.typed_at {
        // Once typed, the `Enter` is owed whatever the clock says - as long
        // as the screen still shows exactly what was typed
        Some(typed) if now >= typed + SUBMIT_DELAY => Step::Submit,
        Some(_) => Step::Wait,
        None if now > arm.fires_at() + MAX_OVERDUE => Step::Disarm(Abandon::Overdue),
        None if now < arm.fires_at() => Step::Wait,
        None if arm.hold_until.is_some_and(|held| now < held) => Step::Wait,
        None => Step::Type,
    }
}

/// Whether the screen lets a `Type` or `Submit` go ahead
pub fn confirm(step: Step, read: &PromptRead) -> Result<(), Abandon> {
    match (step, read) {
        (_, PromptRead::Unrecognised(why)) => Err(Abandon::PromptUnrecognised(why)),
        (Step::Type, PromptRead::Empty) => Ok(()),
        (Step::Type, PromptRead::Text(_)) => Err(Abandon::PromptNotEmpty),
        (Step::Submit, PromptRead::Text(text)) if text == CONTINUE_PROMPT => Ok(()),
        (Step::Submit, _) => Err(Abandon::PromptChanged),
        (Step::Wait | Step::Disarm(_), _) => Ok(()),
    }
}

/// Whether the machine slept between two ticks
///
/// The monotonic clock stops while the machine sleeps and the wall clock does
/// not, so a gap between them is the time spent asleep.
pub fn slept(previous: (Instant, DateTime<Utc>), mono: Instant, now: DateTime<Utc>) -> bool {
    let awake =
        Duration::from_std(mono.saturating_duration_since(previous.0)).unwrap_or(Duration::zero());
    now - previous.1 - awake > SLEEP_JUMP
}

/// Whether the user has sent the session input since `since`
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

/// One cell of the live screen
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScreenCell {
    /// The cell's text; empty for a blank cell
    pub text: String,
    /// Drawn faint - how both agents draw their placeholder text
    pub dim: bool,
    /// The second column of a wide character, which carries no text of its own
    pub wide_tail: bool,
}

/// The live screen, as [`read_prompt`] needs it
#[derive(Debug, Clone, Default)]
pub struct ScreenSnapshot {
    /// Every visible row, one cell per column
    pub rows: Vec<Vec<ScreenCell>>,
    /// `(row, col)`
    pub cursor: (u16, u16),
    pub cursor_visible: bool,
}

/// What the agent's prompt line holds
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptRead {
    /// The agent's prompt, with nothing but its placeholder on it
    Empty,
    /// The agent's prompt, with this typed on it
    Text(String),
    /// Not a prompt Panoptes recognises, and why
    Unrecognised(&'static str),
}

/// The glyphs the agent starts its prompt line with
///
/// Read from the agents themselves: Claude Code 2.1.330 draws `❯ ` between
/// two rules (older builds drew `> ` in a box), and Codex 0.156.1 draws a bold
/// `› `. Both put the real cursor on that line, at the end of what is typed,
/// and draw their placeholder (`Try "..."`, `Ask Codex to do anything`) dim.
fn prompt_glyphs(agent: SessionType) -> &'static [&'static str] {
    match agent {
        SessionType::ClaudeCode => &["\u{276f}", ">"],
        SessionType::OpenAICodex => &["\u{203a}"],
        _ => &[],
    }
}

/// Read the agent's prompt line off the screen
///
/// Deliberately narrow, because a wrong "empty" types into something that
/// is not a prompt: the cursor must be visible and on a line starting with
/// the agent's prompt glyph, the typed text (everything not dim) must end at
/// the cursor - or the cursor must sit right after the glyph when there is
/// none - and the rest of the input area must be blank (see
/// [`input_area_is_clear`]), so a multi-line draft whose first line is empty
/// is not mistaken for an empty prompt. No line below the prompt, and none
/// above it as far as the block it sits in, may look like a menu (a
/// selection marker or a numbered option). Anything else is
/// [`PromptRead::Unrecognised`].
pub fn read_prompt(screen: &ScreenSnapshot, agent: SessionType) -> PromptRead {
    let glyphs = prompt_glyphs(agent);
    if glyphs.is_empty() {
        return PromptRead::Unrecognised("not an agent with a known prompt");
    }
    if !screen.cursor_visible {
        return PromptRead::Unrecognised("the cursor is hidden");
    }
    let (cursor_row, cursor_col) = (screen.cursor.0 as usize, screen.cursor.1 as usize);
    let Some(row) = screen.rows.get(cursor_row) else {
        return PromptRead::Unrecognised("the cursor is off the screen");
    };
    let Some(glyph) = leading_glyph(row, glyphs) else {
        return PromptRead::Unrecognised("the cursor is not on the prompt");
    };
    if screen.rows[cursor_row + 1..]
        .iter()
        .any(|below| looks_like_menu(below))
    {
        return PromptRead::Unrecognised("a menu is open");
    }
    if let Err(why) = input_area_is_clear(&screen.rows, cursor_row, agent) {
        return PromptRead::Unrecognised(why);
    }

    let start = glyph + 2;
    let input = row.get(start..).unwrap_or(&[]);
    // The typed text is everything up to the last cell not drawn dim; past it
    // there may only be blanks, a placeholder, a ghost suggestion or the
    // prompt's frame
    let typed_end = input
        .iter()
        .rposition(|cell| !is_blank(cell) && !cell.dim && !is_frame(cell))
        .map_or(0, |last| last + 1);
    if input[..typed_end].iter().any(|cell| cell.dim) {
        return PromptRead::Unrecognised("the prompt line is ambiguous");
    }
    if cursor_col != start + typed_end {
        return PromptRead::Unrecognised("the cursor is not where the input ends");
    }
    if typed_end == 0 {
        return PromptRead::Empty;
    }
    let text: String = input[..typed_end]
        .iter()
        .filter(|cell| !cell.wide_tail)
        .map(|cell| {
            if cell.text.is_empty() {
                " "
            } else {
                &cell.text
            }
        })
        .collect();
    PromptRead::Text(text)
}

/// Whether everything around the prompt line that belongs to the input is
/// blank, and nothing abutting it is a menu
///
/// Claude draws its input between two rules, so the rows from the prompt
/// line down to the bottom rule must be blank - a draft's later lines sit
/// there - and so must those up to the top rule. Above that rule, the block
/// of rows touching it is checked for a menu too. No rule either way is not a
/// prompt box Panoptes knows, and fails.
///
/// Codex draws its composer with a blank row of padding above and below and
/// its status line underneath, so every row below the prompt but that last
/// status line must be blank, and the status line must be set off by a blank
/// row; above, the rows up to the padding must be empty of menus and text.
fn input_area_is_clear(
    rows: &[Vec<ScreenCell>],
    cursor_row: usize,
    agent: SessionType,
) -> Result<(), &'static str> {
    const MORE_INPUT: &str = "there is more input around the prompt line";
    const MENU: &str = "a menu is open";
    let boxed = agent == SessionType::ClaudeCode;

    // Up: inside the prompt box, then the block abutting it
    let mut row = cursor_row;
    loop {
        if row == 0 {
            if boxed {
                return Err("no prompt box above the prompt line");
            }
            break;
        }
        row -= 1;
        let above = &rows[row];
        if boxed && is_rule(above) {
            break;
        }
        if row_is_blank(above) {
            if boxed {
                continue;
            }
            break;
        }
        return Err(if looks_like_menu(above) {
            MENU
        } else {
            MORE_INPUT
        });
    }
    while row > 0 {
        row -= 1;
        if row_is_blank(&rows[row]) {
            break;
        }
        if looks_like_menu(&rows[row]) {
            return Err(MENU);
        }
    }

    // Down: to the bottom rule, or to the status line
    let below = &rows[cursor_row + 1..];
    if boxed {
        for row in below {
            if is_rule(row) {
                return Ok(());
            }
            if !row_is_blank(row) {
                return Err(MORE_INPUT);
            }
        }
        return Err("no prompt box below the prompt line");
    }
    let status = below.iter().rposition(|row| !row_is_blank(row));
    for (i, row) in below.iter().enumerate() {
        let set_off = i > 0 && row_is_blank(&below[i - 1]);
        if !row_is_blank(row) && !(Some(i) == status && set_off) {
            return Err(MORE_INPUT);
        }
    }
    Ok(())
}

/// Whether a row has nothing on it but blanks and a box's sides
fn row_is_blank(row: &[ScreenCell]) -> bool {
    row.iter().all(|cell| is_blank(cell) || is_frame(cell))
}

/// Whether a row is a horizontal rule or a box's top or bottom edge
fn is_rule(row: &[ScreenCell]) -> bool {
    let drawn: Vec<&ScreenCell> = row.iter().filter(|cell| !is_blank(cell)).collect();
    drawn.len() >= 3
        && drawn.iter().all(|cell| {
            matches!(
                cell.text.as_str(),
                "\u{2500}"
                    | "\u{2501}"
                    | "\u{2550}"
                    | "\u{256d}"
                    | "\u{256e}"
                    | "\u{2570}"
                    | "\u{256f}"
                    | "\u{250c}"
                    | "\u{2510}"
                    | "\u{2514}"
                    | "\u{2518}"
            )
        })
}

/// The column of the row's prompt glyph, when the row starts with one
/// followed by a blank
fn leading_glyph(row: &[ScreenCell], glyphs: &[&str]) -> Option<usize> {
    let first = row
        .iter()
        .position(|cell| !is_blank(cell) && !is_frame(cell))?;
    let is_glyph = glyphs.contains(&row[first].text.as_str());
    let spaced = row.get(first + 1).map_or(true, is_blank);
    (is_glyph && spaced).then_some(first)
}

/// Whether a row looks like an option in a selection menu
///
/// `❯ 1. Stop and wait`, `› Switch model`, or a bare `2. Upgrade`.
fn looks_like_menu(row: &[ScreenCell]) -> bool {
    if leading_glyph(row, &["\u{276f}", "\u{203a}", ">"]).is_some() {
        return true;
    }
    let text: String = row
        .iter()
        .filter(|cell| !cell.wide_tail && !is_frame(cell))
        .map(|cell| {
            if cell.text.is_empty() {
                " "
            } else {
                &cell.text
            }
        })
        .collect();
    let text = text.trim_start();
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && text[digits..].starts_with(". ")
}

fn is_blank(cell: &ScreenCell) -> bool {
    cell.text.trim().is_empty()
}

fn is_frame(cell: &ScreenCell) -> bool {
    matches!(cell.text.as_str(), "\u{2502}" | "\u{2503}" | "|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::events::UsageLimit;
    use crate::session::vterm::VirtualTerminal;
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
    fn test_never_arms_on_a_reset_more_than_six_hours_out() {
        let three_days = at(3 * 24 * 60);
        let info = failed_session(USAGE_LIMIT, Some(three_days));
        let refusal = arm(&info, PID, at(1)).unwrap_err();
        assert_eq!(refusal, Refusal::TooFarOut(three_days));
        assert_eq!(
            refusal.describe(at(1)),
            "Resets in 2 days; won't auto-resume that far out"
        );
        // Exactly six hours is still in
        let info = failed_session(USAGE_LIMIT, Some(at(361)));
        assert!(arm(&info, PID, at(1)).is_ok());
    }

    #[test]
    fn test_never_arms_a_session_that_is_not_failed() {
        let mut info = failed_session(USAGE_LIMIT, Some(at(300)));
        info.state = SessionState::Thinking;
        assert_eq!(arm(&info, PID, at(1)), Err(Refusal::NotFailed));
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
    fn test_never_arms_a_session_typed_into_during_the_failed_turn() {
        let mut info = failed_session(USAGE_LIMIT, Some(at(300)));
        info.turn_started_at = Some(at(-10));
        // The Enter that started the turn
        info.last_user_input = Some(at(-10));
        assert!(arm(&info, PID, at(1)).is_ok());
        // A draft typed while the turn ran
        info.last_user_input = Some(at(-4));
        assert_eq!(arm(&info, PID, at(1)), Err(Refusal::UserTyped));
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
    fn test_gives_up_half_an_hour_overdue() {
        let (info, arm) = armed();
        let deadline = arm.fires_at() + MAX_OVERDUE;
        assert_eq!(step(&arm, &info, PID, deadline), Step::Type);
        assert_eq!(
            step(&arm, &info, PID, deadline + Duration::seconds(1)),
            Step::Disarm(Abandon::Overdue)
        );
    }

    #[test]
    fn test_a_hold_delays_typing() {
        let (info, mut arm) = armed();
        arm.hold(at(302));
        assert_eq!(step(&arm, &info, PID, at(301)), Step::Wait);
        assert_eq!(step(&arm, &info, PID, at(302)), Step::Type);
        // A shorter hold does not shorten a longer one
        arm.hold(at(301));
        assert_eq!(arm.hold_until, Some(at(302)));
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

    #[test]
    fn test_a_jump_in_the_wall_clock_is_sleep() {
        let mono = Instant::now();
        let later = mono + std::time::Duration::from_secs(1);
        // One second on both clocks: awake
        assert!(!slept((mono, at(0)), later, at(0) + Duration::seconds(1)));
        // One second awake, an hour on the wall: asleep
        assert!(slept((mono, at(0)), later, at(60)));
    }

    // --- The screen, through the real emulator ---

    /// A 10x60 screen fed `bytes`
    fn screen(bytes: &[u8]) -> ScreenSnapshot {
        let mut vt = VirtualTerminal::new(10, 60);
        vt.process(bytes);
        vt.live_snapshot()
    }

    /// Claude Code 2.1.330's prompt, as it draws it: a rule, `❯ ` and a dim
    /// placeholder, a rule, a footer, and the cursor put back after `❯ `
    fn claude_prompt(typed: &str) -> Vec<u8> {
        let mut out = b"\x1b[1;1HThe last answer\r\n".to_vec();
        out.extend_from_slice("\x1b[4;1H\x1b[38;2;138;138;138m────────────\x1b[39m".as_bytes());
        out.extend_from_slice("\x1b[5;1H\u{276f} ".as_bytes());
        if typed.is_empty() {
            out.extend_from_slice(b"\x1b[2mTry \"edit manager.rs\"\x1b[22m");
        } else {
            out.extend_from_slice(typed.as_bytes());
        }
        out.extend_from_slice("\x1b[6;1H────────────".as_bytes());
        out.extend_from_slice("\x1b[7;3H\u{23f5}\u{23f5} auto mode on".as_bytes());
        let col = 3 + typed.chars().count();
        out.extend_from_slice(format!("\x1b[5;{col}H").as_bytes());
        out
    }

    /// Codex 0.156.1's composer: a bold `›`, a dim placeholder, the model
    /// line, and the cursor after `› `
    fn codex_prompt(typed: &str) -> Vec<u8> {
        let mut out = b"\x1b[1;1H\x1b[1m\xe2\x80\xba\x1b[22m ".to_vec();
        if typed.is_empty() {
            out.extend_from_slice(b"\x1b[2mAsk Codex to do anything\x1b[22m");
        } else {
            out.extend_from_slice(typed.as_bytes());
        }
        out.extend_from_slice(b"\x1b[3;3Hgpt-5.5 high");
        let col = 3 + typed.chars().count();
        out.extend_from_slice(format!("\x1b[1;{col}H").as_bytes());
        out
    }

    #[test]
    fn test_an_empty_prompt_is_recognised() {
        assert_eq!(
            read_prompt(&screen(&claude_prompt("")), SessionType::ClaudeCode),
            PromptRead::Empty
        );
        assert_eq!(
            read_prompt(&screen(&codex_prompt("")), SessionType::OpenAICodex),
            PromptRead::Empty
        );
    }

    #[test]
    fn test_a_draft_is_not_an_empty_prompt() {
        let read = read_prompt(
            &screen(&claude_prompt("half a thou")),
            SessionType::ClaudeCode,
        );
        assert_eq!(read, PromptRead::Text("half a thou".to_string()));
        assert_eq!(confirm(Step::Type, &read), Err(Abandon::PromptNotEmpty));

        let read = read_prompt(&screen(&codex_prompt("wip")), SessionType::OpenAICodex);
        assert_eq!(confirm(Step::Type, &read), Err(Abandon::PromptNotEmpty));
    }

    #[test]
    fn test_the_typed_prompt_is_what_submit_expects() {
        let read = read_prompt(
            &screen(&claude_prompt(CONTINUE_PROMPT)),
            SessionType::ClaudeCode,
        );
        assert_eq!(confirm(Step::Submit, &read), Ok(()));
        let read = read_prompt(
            &screen(&codex_prompt(CONTINUE_PROMPT)),
            SessionType::OpenAICodex,
        );
        assert_eq!(confirm(Step::Submit, &read), Ok(()));
    }

    #[test]
    fn test_a_line_that_does_not_match_is_not_submitted() {
        for typed in ["x", "Continue where you left off.x", "Continue where"] {
            let read = read_prompt(&screen(&claude_prompt(typed)), SessionType::ClaudeCode);
            assert_eq!(
                confirm(Step::Submit, &read),
                Err(Abandon::PromptChanged),
                "{typed:?}"
            );
        }
        // An empty line - the text went somewhere else - is not sent either
        let read = read_prompt(&screen(&claude_prompt("")), SessionType::ClaudeCode);
        assert_eq!(confirm(Step::Submit, &read), Err(Abandon::PromptChanged));
    }

    /// Claude's `/rate-limit-options` menu, which replaces the prompt
    #[test]
    fn test_claude_rate_limit_menu_is_not_a_prompt() {
        let menu = "\x1b[1;1HWhat do you want to do?\x1b[3;1H\u{276f} 1. Stop and wait for limit to reset\x1b[4;3H2. Upgrade your plan\x1b[3;3H";
        let read = read_prompt(&screen(menu.as_bytes()), SessionType::ClaudeCode);
        assert!(matches!(read, PromptRead::Unrecognised(_)), "{read:?}");
        assert!(confirm(Step::Type, &read).is_err());
    }

    /// Codex's model-switch popup, drawn under the composer
    #[test]
    fn test_codex_popup_under_the_prompt_is_a_menu() {
        let mut bytes = codex_prompt("");
        bytes.extend_from_slice(
            "\x1b[5;1H\x1b[7m\u{203a} 1. Switch to gpt-5-codex-mini\x1b[27m\x1b[6;3H2. Keep current model\x1b[1;3H"
                .as_bytes(),
        );
        assert_eq!(
            read_prompt(&screen(&bytes), SessionType::OpenAICodex),
            PromptRead::Unrecognised("a menu is open")
        );
    }

    /// An option list over a prompt that is still drawn, empty: inside the
    /// box above the prompt line, or in the block touching the box
    #[test]
    fn test_a_menu_above_an_empty_prompt_is_not_a_prompt() {
        let inside = format!(
            "\x1b[2;1H{rule}\x1b[3;3H1. Stop and wait\x1b[4;3H2. Upgrade\x1b[5;1H❯ \x1b[6;1H{rule}\x1b[5;3H",
            rule = "────────────"
        );
        assert_eq!(
            read_prompt(&screen(inside.as_bytes()), SessionType::ClaudeCode),
            PromptRead::Unrecognised("a menu is open")
        );

        let abutting = format!(
            "\x1b[1;1HWhat now?\x1b[2;1H❯ 1. Stop and wait\x1b[3;3H2. Upgrade\x1b[4;1H{rule}\x1b[5;1H❯ \x1b[6;1H{rule}\x1b[5;3H",
            rule = "────────────"
        );
        assert_eq!(
            read_prompt(&screen(abutting.as_bytes()), SessionType::ClaudeCode),
            PromptRead::Unrecognised("a menu is open")
        );

        // Codex: a popup drawn straight over the composer
        let codex = "\x1b[2;1H\x1b[7m› 1. Switch model\x1b[27m\x1b[3;1H› \x1b[2mAsk Codex\x1b[22m\x1b[5;3Hgpt-5.5 high\x1b[3;3H";
        assert_eq!(
            read_prompt(&screen(codex.as_bytes()), SessionType::OpenAICodex),
            PromptRead::Unrecognised("a menu is open")
        );
    }

    /// A multi-line draft whose first line is empty: the prompt line reads
    /// empty, but the input is not
    #[test]
    fn test_a_draft_below_an_empty_prompt_line_is_not_empty() {
        let rule = "────────────";
        for below in ["\x1b[6;3Hsecond line", "\x1b[7;3Hthird line"] {
            let bytes = format!("\x1b[4;1H{rule}\x1b[5;1H❯ {below}\x1b[8;1H{rule}\x1b[5;3H");
            let read = read_prompt(&screen(bytes.as_bytes()), SessionType::ClaudeCode);
            assert!(
                matches!(read, PromptRead::Unrecognised(_)),
                "{below:?}: {read:?}"
            );
            assert!(confirm(Step::Type, &read).is_err());
        }
        // Before `Enter`: the typed prompt on the first line, a draft below
        let bytes = format!(
            "\x1b[4;1H{rule}\x1b[5;1H❯ {CONTINUE_PROMPT}\x1b[6;3Hdraft\x1b[7;1H{rule}\x1b[5;31H"
        );
        let read = read_prompt(&screen(bytes.as_bytes()), SessionType::ClaudeCode);
        assert!(confirm(Step::Submit, &read).is_err(), "{read:?}");

        // Codex: continuation lines sit under the prompt, above the padding
        // and the status line
        for below in ["\x1b[2;3Hsecond line", "\x1b[3;3Hthird line"] {
            let bytes = format!("\x1b[1;1H› {below}\x1b[5;3Hgpt-5.5 high\x1b[1;3H");
            let read = read_prompt(&screen(bytes.as_bytes()), SessionType::OpenAICodex);
            assert!(
                matches!(read, PromptRead::Unrecognised(_)),
                "{below:?}: {read:?}"
            );
        }
    }

    /// Claude's input always sits between two rules; without them this is
    /// not a prompt box Panoptes knows
    #[test]
    fn test_a_claude_prompt_without_its_rules_is_unrecognised() {
        let bytes = "\x1b[5;1H\u{276f} \x1b[5;3H";
        assert!(matches!(
            read_prompt(&screen(bytes.as_bytes()), SessionType::ClaudeCode),
            PromptRead::Unrecognised(_)
        ));
    }

    #[test]
    fn test_a_hidden_cursor_or_one_off_the_prompt_is_unrecognised() {
        let mut hidden = claude_prompt("");
        hidden.extend_from_slice(b"\x1b[?25l");
        assert!(matches!(
            read_prompt(&screen(&hidden), SessionType::ClaudeCode),
            PromptRead::Unrecognised(_)
        ));

        let mut elsewhere = claude_prompt("");
        elsewhere.extend_from_slice(b"\x1b[1;5H");
        assert!(matches!(
            read_prompt(&screen(&elsewhere), SessionType::ClaudeCode),
            PromptRead::Unrecognised(_)
        ));
    }

    #[test]
    fn test_a_scrolled_back_view_reads_the_live_screen() {
        let mut vt = VirtualTerminal::new(10, 60);
        for i in 0..30 {
            vt.process(format!("line {i}\r\n").as_bytes());
        }
        vt.process(b"\x1b[2J");
        vt.process(&codex_prompt(""));
        vt.set_scrollback(5);
        assert_eq!(
            read_prompt(&vt.live_snapshot(), SessionType::OpenAICodex),
            PromptRead::Empty
        );
        assert_eq!(vt.scrollback_offset(), 5, "the user's scroll is put back");
    }
}
