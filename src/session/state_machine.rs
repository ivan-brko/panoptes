//! The session state machine
//!
//! Pure logic that decides what an agent event *means* for a session: how it
//! moves the state, which tools it starts or retires, and whether it raises a
//! new, bell-worthy reason to look at the session. Everything here operates on
//! a bare [`SessionInfo`] - no PTY, no manager - which is what makes the
//! machine testable without spawning a single process.

use chrono::{DateTime, Utc};

use crate::agent::events::AgentEvent;
use crate::config::Config;
use crate::hooks::{HookEvent, HookEventType, NotificationKind};

use super::{AttentionReason, SessionInfo, SessionState};

/// What applying an event did, beyond mutating the session
#[derive(Debug, Clone, Copy)]
pub struct Applied {
    /// Whether the event raised a new, bell-worthy reason to look at the
    /// session. Whether the bell actually sounds is the caller's decision - it
    /// also knows whether the user is already looking at the session and
    /// whether the terminal has focus.
    pub rang: bool,
}

/// Translate a Claude Code hook into the canonical vocabulary
///
/// Hooks are Claude's way of describing itself. Keeping that translation
/// separate from [`apply`], rather than baked into it, is what lets Codex's
/// transcript feed the same machine without either agent's vocabulary leaking
/// into it.
pub fn translate_hook(event: &HookEvent) -> AgentEvent {
    match event.event_type() {
        HookEventType::SessionStart => {
            // `SessionStart` does not only mean "a process came up". Claude
            // also fires it for `clear`, `fork` and - crucially - `compact`,
            // which happens on its own whenever the context window fills up,
            // in the middle of a turn the agent is still working on.
            match event.session_start_source() {
                Some(source) if source.is_fresh_conversation() => AgentEvent::SessionReset {
                    title: event.session_title().map(str::to_string),
                },
                // Mid-turn compaction, a source added after this was written,
                // or a payload-less degraded event. Leave the state alone
                // rather than guess wrong.
                _ => AgentEvent::ContextCompacted,
            }
        }
        HookEventType::SessionEnd => AgentEvent::SessionEnding,
        HookEventType::UserPromptSubmit => AgentEvent::TurnStarted {
            title: event.session_title().map(str::to_string),
        },
        HookEventType::PreToolUse => AgentEvent::ToolStarted {
            key: event.tool_key(),
            name: event.tool_name().unwrap_or("unknown").to_string(),
        },
        HookEventType::PostToolUse | HookEventType::PostToolUseFailure => {
            AgentEvent::ToolFinished {
                key: event.tool_key(),
            }
        }
        HookEventType::Stop => AgentEvent::TurnCompleted {
            last_message: event.last_assistant_message().map(str::to_string),
        },
        HookEventType::PermissionRequest => AgentEvent::ApprovalRequested {
            tool: event.tool_name().map(str::to_string),
        },
        HookEventType::Notification => match event.notification_kind() {
            // Carry the tool through where the payload named one; the
            // generic mapping cannot know it
            NotificationKind::PermissionRequest => AgentEvent::ApprovalRequested {
                tool: event.tool_name().map(str::to_string),
            },
            kind => AgentEvent::from(kind),
        },
        // Codex CLI's only hook: the agent is done and wants input
        HookEventType::AgentTurnComplete => AgentEvent::TurnCompleted { last_message: None },
        HookEventType::Unknown => AgentEvent::Ignored,
    }
}

/// Apply a canonical agent event to a session
///
/// The single place where what an event *means* is decided. Claude's hooks and
/// both transcript tailers all funnel through here (via
/// `SessionManager::apply_agent_event`), so the two agents cannot drift apart
/// in how the same event moves a session.
pub fn apply(
    info: &mut SessionInfo,
    event: AgentEvent,
    now: DateTime<Utc>,
    config: &Config,
) -> Applied {
    // Two events must not count as activity, or they would hold
    // `last_activity` permanently fresh and neither the idle badge nor the
    // suspend sweep would ever fire on the sessions they exist for:
    //
    // - `IdleReminder` is the agent reporting that nothing has happened.
    // - `Subagents` is Panoptes' own periodic observation, not the agent
    //   doing anything.
    let is_activity = !matches!(
        event,
        AgentEvent::IdleReminder | AgentEvent::Subagents { .. }
    );
    if is_activity {
        info.last_activity = now;
    }

    // How an event wants to move the session.
    //
    // `Authoritative` overwrites. `AtLeast` only upgrades, per
    // `SessionState::most_urgent` - subagents share one session, so several
    // states are genuinely true at once and the events describing them
    // interleave. An event announcing new concurrent work must not be able
    // to un-report a permission dialog someone else is blocked on, while an
    // event reporting the turn is over must be able to demote, or a single
    // dropped tool completion would pin the session in `Executing`.
    enum Move {
        Authoritative(SessionState),
        AtLeast(SessionState),
        Unchanged,
    }

    let mut attention: Option<AttentionReason> = None;
    let mut clear_attention = false;

    let movement = match event {
        AgentEvent::SessionReset { title } => {
            if let Some(title) = title {
                info.adopt_agent_title(&title);
            }
            info.in_flight.clear();
            clear_attention = true;
            // The agent is up but has not been asked anything yet
            Move::Authoritative(SessionState::Waiting)
        }

        AgentEvent::ContextCompacted => {
            tracing::debug!(
                session_id = %info.id,
                "Conversation compacted mid-turn; leaving state unchanged"
            );
            Move::Unchanged
        }

        AgentEvent::SessionEnding => {
            // The process is on its way out but has not gone yet. Leave the
            // Exited transition to `check_alive`, which is the only place
            // that can tell a clean exit from a crash; just stop claiming
            // that tools are still running.
            info.in_flight.clear();
            Move::Unchanged
        }

        AgentEvent::TurnStarted { title } => {
            if let Some(title) = title {
                info.adopt_agent_title(&title);
            }
            // A prompt is a clean turn boundary: anything still marked in
            // flight from the previous turn is stale, and the user is
            // demonstrably present so nothing needs flagging for them.
            info.in_flight.clear();
            clear_attention = true;
            Move::Authoritative(SessionState::Thinking)
        }

        AgentEvent::ToolStarted { key, name } => {
            info.start_tool(key, name, now);
            Move::AtLeast(SessionState::Executing)
        }

        AgentEvent::ToolFinished { key } => {
            info.finish_tool(&key);
            if info.in_flight.is_empty() {
                // Nothing left running. Demote out of Executing, but do not
                // stomp on a permission dialog raised meanwhile.
                match info.state {
                    SessionState::AwaitingApproval => Move::Unchanged,
                    _ => Move::Authoritative(SessionState::Thinking),
                }
            } else {
                Move::AtLeast(SessionState::Executing)
            }
        }

        AgentEvent::TurnCompleted { last_message } => {
            if let Some(message) = last_message {
                info.set_last_message(&message);
            }
            // End of turn: whatever was still marked in flight never
            // reported back and is not running any more.
            info.in_flight.clear();
            attention = Some(AttentionReason::TurnComplete);
            Move::Authoritative(SessionState::Waiting)
        }

        AgentEvent::TurnAborted => {
            // The user interrupted it themselves, so they already know -
            // flagging it for their attention would be telling them what
            // they just did.
            info.in_flight.clear();
            Move::Authoritative(SessionState::Waiting)
        }

        AgentEvent::ApprovalRequested { tool } => {
            attention = Some(AttentionReason::Approval { tool });
            Move::AtLeast(SessionState::AwaitingApproval)
        }

        AgentEvent::IdleReminder => {
            if config.attention_on_idle {
                attention = Some(AttentionReason::TurnComplete);
            }
            Move::Unchanged
        }

        AgentEvent::Usage(usage) => {
            info.usage.merge(usage);
            Move::Unchanged
        }

        AgentEvent::Subagents { active } => {
            info.subagents = active;
            Move::Unchanged
        }

        AgentEvent::Ignored => Move::Unchanged,
    };

    match movement {
        Move::Authoritative(state) => info.set_state_at(state, now),
        Move::AtLeast(state) => {
            let resolved = info.state.most_urgent(state);
            if resolved != info.state {
                info.set_state_at(resolved, now);
            }
        }
        Move::Unchanged => {}
    }

    if clear_attention {
        info.attention = None;
    }

    let Some(reason) = attention else {
        return Applied { rang: false };
    };

    // Ring only when the reason is new. Re-notifying for a flag the user
    // has already seen is what makes notifications easy to ignore.
    let is_new = info.attention.as_ref() != Some(&reason);
    let rings = is_new && config.notify_on.rings(&reason);
    info.attention = Some(reason);

    Applied { rang: rings }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{SessionId, SessionType};
    use std::path::PathBuf;
    use uuid::Uuid;

    fn test_info() -> SessionInfo {
        SessionInfo::new(
            "test-session".to_string(),
            PathBuf::from("/tmp"),
            Uuid::new_v4(),
            Uuid::new_v4(),
        )
    }

    /// Build a hook event addressed at a session, with an arbitrary payload
    fn hook(session_id: SessionId, event: &str, payload: serde_json::Value) -> HookEvent {
        HookEvent {
            session_id: session_id.to_string(),
            event: event.to_string(),
            timestamp: 100,
            payload,
        }
    }

    /// Feed a hook through translation and application, as the manager does.
    /// Returns whether the event rang.
    fn apply_hook(
        info: &mut SessionInfo,
        event: &str,
        payload: serde_json::Value,
        config: &Config,
    ) -> bool {
        let event = hook(info.id, event, payload);
        apply(info, translate_hook(&event), Utc::now(), config).rang
    }

    #[test]
    fn test_permission_request_raises_approval_and_rings_once() {
        let config = Config::default();
        let mut info = test_info();

        info.set_state_at(SessionState::Waiting, Utc::now());
        assert!(info.attention.is_none());

        let payload = serde_json::json!({"tool_name": "Bash"});
        assert!(
            apply_hook(&mut info, "PermissionRequest", payload.clone(), &config),
            "First PermissionRequest should ring bell"
        );
        assert_eq!(
            info.attention,
            Some(AttentionReason::Approval {
                tool: Some("Bash".to_string())
            })
        );
        assert_eq!(
            info.state,
            SessionState::AwaitingApproval,
            "a blocked dialog must not look like a finished turn"
        );

        // Same reason again: already flagged, so no second bell
        assert!(
            !apply_hook(&mut info, "PermissionRequest", payload.clone(), &config),
            "Duplicate PermissionRequest should not ring bell"
        );
        assert!(info.attention.is_some());

        // Acknowledging clears the queue entry but not the state - the dialog
        // is still open
        info.attention = None;
        assert_eq!(info.state, SessionState::AwaitingApproval);

        assert!(
            apply_hook(&mut info, "PermissionRequest", payload, &config),
            "PermissionRequest after ack should ring bell"
        );
    }

    /// Every `notification_type` Claude Code actually sends must classify to
    /// something deliberate.
    ///
    /// The values are the CLI's own. Panoptes previously matched invented
    /// spellings ("idle", "permission_request"), so every real notification
    /// fell through to `Other` - which is treated as "the agent wants you" and
    /// therefore rang the bell and flagged the session. The repeating
    /// `idle_prompt`, which fires precisely when the user has *not* replied,
    /// made a session look like it needed attention when nothing had changed.
    #[test]
    fn test_real_claude_notification_types_do_not_all_ring() {
        let config = Config::default();

        // Silent: informational, or the idle nag with attention_on_idle off
        for value in [
            "idle_prompt",
            "auth_success",
            "elicitation_complete",
            "elicitation_response",
        ] {
            let mut info = test_info();
            info.set_state_at(SessionState::Waiting, Utc::now());

            let payload = serde_json::json!({ "notification_type": value });
            assert!(
                !apply_hook(&mut info, "Notification", payload, &config),
                "{} must not ring",
                value
            );
            assert!(
                info.attention.is_none(),
                "{} must not flag the session",
                value
            );
        }

        // Actionable: the agent is blocked on the user
        for value in [
            "permission_prompt",
            "elicitation_dialog",
            "agent_needs_input",
        ] {
            let mut info = test_info();
            let payload = serde_json::json!({ "notification_type": value });
            assert!(
                apply_hook(&mut info, "Notification", payload, &config),
                "{} must ring",
                value
            );
        }
    }

    /// Re-notifying for a reason the user already cleared is what makes an
    /// idle session ring with nothing new to show.
    #[test]
    fn test_repeated_idle_prompts_stay_silent_after_the_user_looks() {
        let config = Config::default();
        let mut info = test_info();

        // The agent answers and waits: one ring, which the user acts on
        assert!(apply_hook(
            &mut info,
            "Stop",
            serde_json::json!({}),
            &config
        ));
        info.attention = None; // acknowledged

        // The user reads it and does not reply. Claude nags every minute.
        for _ in 0..5 {
            let payload = serde_json::json!({"notification_type": "idle_prompt"});
            assert!(
                !apply_hook(&mut info, "Notification", payload, &config),
                "an idle nag must never ring again"
            );
        }
        assert!(info.attention.is_none());
        assert_eq!(info.state, SessionState::Waiting);
    }

    #[test]
    fn test_idle_notification_does_not_ring_but_permission_does() {
        let config = Config::default();
        let mut info = test_info();

        info.set_state_at(SessionState::Waiting, Utc::now());

        // Claude's periodic idle nag: not actionable, must stay silent
        let idle =
            serde_json::json!({"notification_type": "idle_prompt", "message": "still there?"});
        assert!(!apply_hook(&mut info, "Notification", idle, &config));
        assert!(
            info.attention.is_none(),
            "idle nag must not raise attention"
        );
        assert_eq!(info.state, SessionState::Waiting);

        // The same event type carrying a permission request must ring
        let permission = serde_json::json!({"notification_type": "permission_prompt"});
        assert!(apply_hook(&mut info, "Notification", permission, &config));
        assert_eq!(info.state, SessionState::AwaitingApproval);
    }

    #[test]
    fn test_turn_with_no_tools_thinks_then_waits() {
        let config = Config::default();
        let mut info = test_info();

        // A prompt submitted any way at all - typed, pasted, or passed on the
        // command line - reports itself. No keystroke is involved.
        let submit = serde_json::json!({"prompt": "hello"});
        assert!(!apply_hook(&mut info, "UserPromptSubmit", submit, &config));
        assert_eq!(info.state, SessionState::Thinking);

        // The agent answers without calling a tool
        let stop = serde_json::json!({"last_assistant_message": "Hi there."});
        assert!(apply_hook(&mut info, "Stop", stop, &config));
        assert_eq!(info.state, SessionState::Waiting);
        assert_eq!(info.attention, Some(AttentionReason::TurnComplete));
        assert_eq!(info.last_message.as_deref(), Some("Hi there."));
    }

    #[test]
    fn test_concurrent_tools_render_as_a_set() {
        let config = Config::default();
        let mut info = test_info();

        // Subagents share one session_id, so several tools are genuinely in
        // flight at once and their events interleave.
        for (id, name) in [("t1", "Read"), ("t2", "Grep"), ("t3", "Grep")] {
            let payload = serde_json::json!({"tool_name": name, "tool_use_id": id});
            apply_hook(&mut info, "PreToolUse", payload, &config);
        }

        assert_eq!(info.state, SessionState::Executing);
        assert_eq!(info.in_flight.len(), 3);
        assert_eq!(info.in_flight_summary().as_deref(), Some("Read, Grep ×2"));

        // Retiring one leaves the others running rather than clearing the state
        let payload = serde_json::json!({"tool_name": "Read", "tool_use_id": "t1"});
        apply_hook(&mut info, "PostToolUse", payload, &config);
        assert_eq!(info.state, SessionState::Executing);
        assert_eq!(info.in_flight_summary().as_deref(), Some("Grep ×2"));

        // Only when the last one finishes does the session fall back
        for id in ["t2", "t3"] {
            let payload = serde_json::json!({"tool_name": "Grep", "tool_use_id": id});
            apply_hook(&mut info, "PostToolUse", payload, &config);
        }
        assert_eq!(info.state, SessionState::Thinking);
        assert!(info.in_flight.is_empty());
    }

    #[test]
    fn test_post_tool_use_retires_the_right_tool_when_reordered() {
        let config = Config::default();
        let mut info = test_info();

        let read = serde_json::json!({"tool_name": "Read", "tool_use_id": "t1"});
        apply_hook(&mut info, "PreToolUse", read, &config);
        let bash = serde_json::json!({"tool_name": "Bash", "tool_use_id": "t2"});
        apply_hook(&mut info, "PreToolUse", bash, &config);

        // Hook deliveries are backgrounded and timestamped to the second, so
        // the second tool's completion can land first. Keying by tool_use_id
        // means it retires its own entry rather than the most recent one.
        let done = serde_json::json!({"tool_name": "Bash", "tool_use_id": "t2"});
        apply_hook(&mut info, "PostToolUse", done, &config);

        assert_eq!(info.in_flight_summary().as_deref(), Some("Read"));
        assert_eq!(info.state, SessionState::Executing);
    }

    #[test]
    fn test_permission_request_survives_concurrent_tool_traffic() {
        let config = Config::default();
        let mut info = test_info();

        let read = serde_json::json!({"tool_name": "Read", "tool_use_id": "t1"});
        apply_hook(&mut info, "PreToolUse", read, &config);
        let permission = serde_json::json!({"tool_name": "Bash"});
        apply_hook(&mut info, "PermissionRequest", permission, &config);
        assert_eq!(info.state, SessionState::AwaitingApproval);

        // Another subagent's tool starting must not un-report the open dialog
        let grep = serde_json::json!({"tool_name": "Grep", "tool_use_id": "t2"});
        apply_hook(&mut info, "PreToolUse", grep, &config);
        assert_eq!(
            info.state,
            SessionState::AwaitingApproval,
            "AwaitingApproval outranks Executing"
        );

        // Nor must the last tool finishing
        for id in ["t1", "t2"] {
            let payload = serde_json::json!({"tool_use_id": id});
            apply_hook(&mut info, "PostToolUse", payload, &config);
        }
        assert_eq!(info.state, SessionState::AwaitingApproval);

        // Only the end of the turn resolves it
        apply_hook(&mut info, "Stop", serde_json::json!({}), &config);
        assert_eq!(info.state, SessionState::Waiting);
    }

    #[test]
    fn test_stop_clears_leaked_in_flight_tools() {
        let config = Config::default();
        let mut info = test_info();

        // A PreToolUse whose PostToolUse never arrives - dropped hook, dead
        // subagent - would otherwise pin the session in Executing forever
        let payload = serde_json::json!({"tool_name": "Bash", "tool_use_id": "t1"});
        apply_hook(&mut info, "PreToolUse", payload, &config);
        apply_hook(&mut info, "Stop", serde_json::json!({}), &config);

        assert!(info.in_flight.is_empty());
        assert_eq!(info.state, SessionState::Waiting);
    }

    #[test]
    fn test_agent_title_replaces_only_generated_names() {
        let config = Config::default();
        let mut info = test_info();

        let titled =
            serde_json::json!({"source": "startup", "session_title": "Fixing the login bug"});

        // A name the user typed is theirs
        apply_hook(&mut info, "SessionStart", titled.clone(), &config);
        assert_eq!(info.name, "test-session");

        // A name Panoptes generated is not
        info.auto_named = true;
        apply_hook(&mut info, "SessionStart", titled, &config);
        assert_eq!(info.name, "Fixing the login bug");
    }

    #[test]
    fn test_session_start_from_compaction_does_not_report_a_busy_session_as_idle() {
        let config = Config::default();
        let mut info = test_info();

        // A long agentic loop, mid-tool
        let payload = serde_json::json!({"tool_name": "Bash", "tool_use_id": "t1"});
        apply_hook(&mut info, "PreToolUse", payload, &config);
        assert_eq!(info.state, SessionState::Executing);

        // Claude auto-compacts when the context window fills. This fires
        // SessionStart with no user involvement at all, while the agent is
        // still working.
        let compact = serde_json::json!({"source": "compact", "model": "claude-opus-4-8"});
        apply_hook(&mut info, "SessionStart", compact, &config);

        assert_eq!(
            info.state,
            SessionState::Executing,
            "compaction must not report a working session as finished"
        );
        assert_eq!(
            info.in_flight_summary().as_deref(),
            Some("Bash"),
            "the tool is still running across a compaction"
        );
    }

    #[test]
    fn test_session_start_from_a_real_start_resets_the_session() {
        let config = Config::default();
        let mut info = test_info();

        let payload = serde_json::json!({"tool_name": "Bash", "tool_use_id": "t1"});
        apply_hook(&mut info, "PreToolUse", payload, &config);
        apply_hook(&mut info, "Stop", serde_json::json!({}), &config);
        assert!(info.attention.is_some());

        // /clear starts a fresh conversation: nothing is running and nothing
        // is owed to the user
        let clear = serde_json::json!({"source": "clear"});
        apply_hook(&mut info, "SessionStart", clear, &config);

        assert_eq!(info.state, SessionState::Waiting);
        assert!(info.in_flight.is_empty());
        assert!(info.attention.is_none());
    }

    #[test]
    fn test_user_prompt_clears_stale_attention() {
        let config = Config::default();
        let mut info = test_info();

        apply_hook(&mut info, "Stop", serde_json::json!({}), &config);
        assert!(info.attention.is_some());

        // The user is demonstrably present; nothing needs flagging for them
        apply_hook(
            &mut info,
            "UserPromptSubmit",
            serde_json::json!({}),
            &config,
        );
        assert!(info.attention.is_none());
        assert_eq!(info.state, SessionState::Thinking);
    }

    #[test]
    fn test_notification_without_payload_still_notifies() {
        let config = Config::default();
        let mut info = test_info();

        // The no-jq degraded path delivers an envelope with no payload. An
        // unclassifiable notification should fall back to notifying rather
        // than silently swallowing a possible permission prompt.
        assert!(apply_hook(
            &mut info,
            "Notification",
            serde_json::Value::Null,
            &config
        ));
        assert_eq!(
            info.attention,
            Some(AttentionReason::Approval { tool: None })
        );
    }

    #[test]
    fn test_interrupted_codex_turn_does_not_stay_stuck() {
        let config = Config::default();
        let mut info = test_info();
        let now = Utc::now();

        apply(
            &mut info,
            AgentEvent::TurnStarted { title: None },
            now,
            &config,
        );
        apply(
            &mut info,
            AgentEvent::ToolStarted {
                key: "c1".to_string(),
                name: "shell".to_string(),
            },
            now,
            &config,
        );
        assert_eq!(info.state, SessionState::Executing);

        // The user pressed Esc. No completion for the running tool will ever
        // arrive, so without handling this the session sits in Executing until
        // the stall watchdog notices.
        apply(&mut info, AgentEvent::TurnAborted, now, &config);

        assert_eq!(info.state, SessionState::Waiting);
        assert!(info.in_flight.is_empty());
        assert!(
            info.attention.is_none(),
            "the user interrupted it themselves; telling them about it is noise"
        );
    }

    #[test]
    fn test_claude_usage_events_never_disturb_state() {
        let config = Config::default();
        let mut info = test_info();
        let now = Utc::now();

        apply(
            &mut info,
            AgentEvent::TurnStarted { title: None },
            now,
            &config,
        );
        apply(
            &mut info,
            AgentEvent::ToolStarted {
                key: "t1".to_string(),
                name: "Read".to_string(),
            },
            now,
            &config,
        );

        // Hooks own Claude's state. The transcript arrives later and must only
        // ever add figures, or the two producers fight over the same field.
        apply(
            &mut info,
            AgentEvent::Usage(crate::agent::events::UsageSnapshot {
                model: Some("claude-opus-4-8".to_string()),
                total_tokens: Some(1234),
                ..Default::default()
            }),
            now,
            &config,
        );

        assert_eq!(info.state, SessionState::Executing);
        assert_eq!(info.in_flight.len(), 1);
        assert_eq!(info.usage.total_tokens, Some(1234));
    }

    #[test]
    fn test_codex_transcript_drives_a_full_turn() {
        use crate::transcript::{Tailer, TranscriptKind};

        let config = Config::default();
        let mut info = test_info();
        info.session_type = SessionType::OpenAICodex;
        info.set_state_at(SessionState::Waiting, Utc::now());

        // The exact record sequence a real Codex turn writes, in order
        let temp_dir = tempfile::tempdir().unwrap();
        let rollout = temp_dir.path().join("rollout.jsonl");
        std::fs::write(
            &rollout,
            [
                r#"{"type":"event_msg","payload":{"type":"task_started"}}"#,
                r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","call_id":"c1"}}"#,
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"model":"gpt-5-codex","model_context_window":272000,"total_token_usage":{"total_tokens":3000}}}}"#,
                r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"c1"}}"#,
                r#"{"type":"event_msg","payload":{"type":"agent_message"}}"#,
                r#"{"type":"event_msg","payload":{"type":"task_complete","last_agent_message":"pong"}}"#,
                "",
            ]
            .join("\n"),
        )
        .unwrap();

        let mut tailer = Tailer::from_start(TranscriptKind::Codex, rollout);
        let (events, _) = tailer.poll();

        // Feed them through one at a time, checking the state as it goes -
        // Codex used to be able to report only "my turn ended"
        let mut states = Vec::new();
        for event in events {
            apply(&mut info, event, Utc::now(), &config);
            states.push(info.state);
        }

        assert_eq!(
            states,
            vec![
                SessionState::Thinking,  // task_started
                SessionState::Executing, // function_call
                SessionState::Executing, // token_count changes nothing
                SessionState::Thinking,  // function_call_output, nothing left
                SessionState::Waiting,   // task_complete
            ]
        );

        assert_eq!(info.attention, Some(AttentionReason::TurnComplete));
        assert_eq!(info.last_message.as_deref(), Some("pong"));
        assert_eq!(info.usage.model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(
            info.usage.context_percent(),
            Some(3000.0 / 272_000.0 * 100.0)
        );
        assert!(info.in_flight.is_empty());
    }
}
