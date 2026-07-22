//! Session manager module
//!
//! This module provides centralized management of Claude Code sessions,
//! handling creation, destruction, state updates, and I/O polling.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};

use crate::agent::adapter::SpawnConfig;
use crate::agent::AgentType;
use crate::claude_config::ClaudeConfigId;
use crate::config::Config;
use crate::hooks::{HookEvent, HookEventType};
use crate::project::{BranchId, ProjectId};

use crate::codex_config::CodexConfigId;

use super::{Session, SessionId, SessionInfo, SessionState, SessionStore, SessionType};

/// Manages multiple Claude Code sessions
pub struct SessionManager {
    /// All active sessions, keyed by session ID
    sessions: HashMap<SessionId, Session>,
    /// Session order (for navigation)
    session_order: Vec<SessionId>,
    /// Application configuration
    config: Config,
    /// Durable index of sessions, so they can be recovered after a restart
    store: SessionStore,
    /// Sessions inherited from a previous Panoptes run, not yet brought back
    ///
    /// These have no PTY and therefore cannot be `Session` values. They stay
    /// inert until the user opens one, at which point the entry moves from here
    /// into `sessions`.
    recovered: HashMap<SessionId, SessionInfo>,
}

/// A session as it appears in a list
///
/// Lists mix sessions running right now with ones recoverable from a previous
/// run, and both render from `SessionInfo`. Keeping them in one ordered
/// sequence means selection indices stay meaningful without callers having to
/// merge two collections themselves.
#[derive(Debug, Clone, Copy)]
pub struct SessionEntry<'a> {
    /// Metadata for the session
    pub info: &'a SessionInfo,
    /// Whether a process is currently attached
    pub live: bool,
}

impl SessionManager {
    /// Create a new session manager
    ///
    /// A corrupted or unreadable session store is not fatal: it degrades to the
    /// pre-persistence behaviour of starting with nothing to recover.
    pub fn new(config: Config) -> Self {
        let (store, warning) = SessionStore::load_with_status();
        if let Some(warning) = warning {
            tracing::warn!("{}", warning);
        }
        Self::with_store(config, store)
    }

    /// Create a session manager backed by a specific store (for testing)
    ///
    /// Tests must use this rather than `new`, which reads and writes the real
    /// `~/.panoptes/sessions.json` owned by any running Panoptes instance.
    pub fn with_store(config: Config, store: SessionStore) -> Self {
        let recovered = Self::reconcile(&store);
        Self {
            sessions: HashMap::new(),
            session_order: Vec::new(),
            config,
            store,
            recovered,
        }
    }

    /// Turn stored records into the recovery list
    ///
    /// Every record in the store is by definition dead at startup: its PTY died
    /// with the Panoptes process that owned it. So the live state captured in
    /// the record (Thinking, Waiting, an attention flag) describes a process
    /// that no longer exists and is discarded rather than shown as if current.
    fn reconcile(store: &SessionStore) -> HashMap<SessionId, SessionInfo> {
        store
            .sessions()
            .map(|stored| {
                let mut info = stored.clone();
                info.state = SessionState::Resumable;
                info.state_entered_at = Utc::now();
                // Belonged to a process that is gone; re-derived once resumed
                info.needs_attention = false;
                info.exit_reason = None;
                info.exited_at = None;
                (info.id, info)
            })
            .collect()
    }

    /// Access the durable session store
    pub fn store(&self) -> &SessionStore {
        &self.store
    }

    /// Sessions inherited from a previous run that have not been brought back
    pub fn recovered(&self) -> impl Iterator<Item = &SessionInfo> {
        self.recovered.values()
    }

    /// Number of sessions awaiting recovery
    pub fn recovered_count(&self) -> usize {
        self.recovered.len()
    }

    /// Look up a recovered session by ID
    pub fn get_recovered(&self, session_id: SessionId) -> Option<&SessionInfo> {
        self.recovered.get(&session_id)
    }

    /// Discard a recovered session without ever bringing it back
    pub fn discard_recovered(&mut self, session_id: SessionId) -> bool {
        if self.recovered.remove(&session_id).is_none() {
            return false;
        }
        self.forget_session(session_id);
        true
    }

    /// All sessions in navigation order, live first then recoverable
    ///
    /// Live sessions keep their existing order so navigation is unchanged for
    /// anyone not using recovery; recovered ones follow, most recent first.
    pub fn entries_in_order(&self) -> Vec<SessionEntry<'_>> {
        let mut entries: Vec<SessionEntry<'_>> = self
            .session_order
            .iter()
            .filter_map(|id| self.sessions.get(id))
            .map(|session| SessionEntry {
                info: &session.info,
                live: true,
            })
            .collect();

        // A session that has been brought back lives in `sessions`; skip its
        // stale recovery entry rather than listing it twice
        let mut recovered: Vec<&SessionInfo> = self
            .recovered
            .values()
            .filter(|info| !self.sessions.contains_key(&info.id))
            .collect();
        recovered.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
        entries.extend(
            recovered
                .into_iter()
                .map(|info| SessionEntry { info, live: false }),
        );

        entries
    }

    /// All sessions for a branch, live first then recoverable
    pub fn entries_for_branch(&self, branch_id: BranchId) -> Vec<SessionEntry<'_>> {
        self.entries_in_order()
            .into_iter()
            .filter(|entry| entry.info.branch_id == branch_id)
            .collect()
    }

    /// All sessions for a project, live first then recoverable
    pub fn entries_for_project(&self, project_id: ProjectId) -> Vec<SessionEntry<'_>> {
        self.entries_in_order()
            .into_iter()
            .filter(|entry| entry.info.project_id == project_id)
            .collect()
    }

    /// Bring a recovered session back to life
    ///
    /// Relaunches the agent in the session's original working directory,
    /// reattaching to its conversation via the recorded ID. The Panoptes session
    /// ID is preserved, which keeps hook routing working: hooks key off
    /// `PANOPTES_SESSION_ID`, so a resumed session reports state exactly as it
    /// did before the restart.
    ///
    /// Config directories are passed in rather than looked up here, mirroring
    /// `create_session_with_config` - the manager does not own the account
    /// stores.
    ///
    /// On failure the recovery entry is left untouched so the user can retry or
    /// discard it; a failed resume must not silently consume the record.
    pub fn resume_session(
        &mut self,
        session_id: SessionId,
        rows: usize,
        cols: usize,
        claude_config_dir: Option<PathBuf>,
        codex_home: Option<PathBuf>,
    ) -> Result<SessionId> {
        let info = self
            .recovered
            .get(&session_id)
            .ok_or_else(|| anyhow!("No recovered session with ID {}", session_id))?
            .clone();

        if let Some(reason) = info.resume_blocker() {
            return Err(anyhow!("Cannot resume '{}': {}", info.name, reason));
        }

        let agent_type = match info.session_type {
            SessionType::ClaudeCode => AgentType::ClaudeCode,
            SessionType::OpenAICodex => AgentType::OpenAICodex,
            SessionType::Shell => AgentType::Shell,
        };

        // A shell has no conversation to reattach to - it is restored by
        // respawning in the same directory, so it must not carry a resume cursor
        let resume = match info.session_type {
            SessionType::Shell => None,
            _ => info.agent_session_id.clone(),
        };

        let spawn_config = SpawnConfig {
            session_id,
            session_name: info.name.clone(),
            working_dir: info.working_dir.clone(),
            initial_prompt: None,
            rows: rows as u16,
            cols: cols as u16,
            claude_config_dir,
            codex_home,
            resume,
        };

        let adapter = agent_type.create_adapter();
        let spawn_result = adapter.spawn(&self.config, &spawn_config)?;

        let mut info = info;
        info.state = SessionState::Starting;
        info.state_entered_at = Utc::now();
        info.last_activity = Utc::now();
        if spawn_result.agent_session_id.is_some() {
            info.agent_session_id = spawn_result.agent_session_id;
        }

        let session = Session::with_scrollback(
            info,
            spawn_result.pty,
            rows,
            cols,
            self.config.scrollback_lines,
        );

        // Only now that the process exists does the session stop being "recovered"
        self.recovered.remove(&session_id);
        self.sessions.insert(session_id, session);
        self.session_order.push(session_id);
        self.persist_session(session_id);

        tracing::info!(session_id = %session_id, "Resumed session from a previous run");

        Ok(session_id)
    }

    /// Live Codex sessions whose conversation ID has not been resolved yet
    ///
    /// Returns the session ID, its working directory, and when it started -
    /// everything needed to match it against a rollout file. Empty once every
    /// Codex session has an ID, which is the steady state.
    /// Ordered oldest-first, so that when several sessions share a working
    /// directory each claims the rollout it actually created.
    pub fn sessions_pending_codex_id(&self) -> Vec<(SessionId, PathBuf, DateTime<Utc>)> {
        let mut pending: Vec<_> = self
            .sessions
            .values()
            .filter(|session| {
                session.info.session_type == SessionType::OpenAICodex
                    && session.info.agent_session_id.is_none()
                    && session.info.state != SessionState::Exited
            })
            .map(|session| {
                (
                    session.info.id,
                    session.info.working_dir.clone(),
                    session.info.created_at,
                )
            })
            .collect();
        pending.sort_by_key(|(_, _, created_at)| *created_at);
        pending
    }

    /// Every agent conversation ID already spoken for
    ///
    /// Includes recovered sessions, not just live ones: a conversation waiting
    /// to be resumed still belongs to that session and must not be handed to a
    /// different one that happens to share its working directory.
    pub fn claimed_agent_session_ids(&self) -> std::collections::HashSet<String> {
        self.sessions
            .values()
            .map(|session| &session.info)
            .chain(self.recovered.values())
            .filter_map(|info| info.agent_session_id.clone())
            .collect()
    }

    /// Record the agent conversation ID for a live session
    ///
    /// Returns whether the session was found and updated. Persists immediately:
    /// an ID that only exists in memory is exactly the pointer this whole
    /// mechanism is meant to stop losing.
    pub fn set_agent_session_id(
        &mut self,
        session_id: SessionId,
        agent_session_id: String,
    ) -> bool {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return false;
        };
        if session.info.agent_session_id.as_deref() == Some(agent_session_id.as_str()) {
            return false;
        }
        session.info.agent_session_id = Some(agent_session_id);
        self.persist_session(session_id);
        true
    }

    /// Write a session's record to the durable index
    ///
    /// Called on membership changes rather than state changes: identity and
    /// context are what survive a restart, whereas live state (Thinking,
    /// Executing) is meaningless once the process is gone. Hook events fire on
    /// every tool call, so persisting state would mean writing to disk
    /// constantly for data that gets discarded on load anyway.
    ///
    /// Failure to persist never fails the operation that triggered it - losing
    /// the ability to recover a session later is strictly better than refusing
    /// to create it now.
    fn persist_session(&mut self, session_id: SessionId) {
        let Some(session) = self.sessions.get(&session_id) else {
            return;
        };
        self.store.upsert(session.info.clone());
        if let Err(e) = self.store.save() {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "Failed to persist session record; this session will not be recoverable"
            );
        }
    }

    /// Drop a session's record from the durable index
    ///
    /// Used when the user explicitly discards a session, as opposed to quitting
    /// Panoptes - the latter must leave records intact so they can be resumed.
    fn forget_session(&mut self, session_id: SessionId) {
        if self.store.remove(session_id).is_none() {
            return;
        }
        if let Err(e) = self.store.save() {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "Failed to remove session record; it may reappear as recoverable"
            );
        }
    }

    /// Create a new session with the given name, working directory, project/branch, and terminal dimensions
    #[allow(clippy::too_many_arguments)]
    pub fn create_session(
        &mut self,
        name: String,
        working_dir: PathBuf,
        project_id: ProjectId,
        branch_id: BranchId,
        initial_prompt: Option<String>,
        rows: usize,
        cols: usize,
    ) -> Result<SessionId> {
        self.create_session_with_config(
            name,
            working_dir,
            project_id,
            branch_id,
            initial_prompt,
            rows,
            cols,
            None,
            None,
            None,
        )
    }

    /// Create a new session with a specific Claude configuration
    #[allow(clippy::too_many_arguments)]
    pub fn create_session_with_config(
        &mut self,
        name: String,
        working_dir: PathBuf,
        project_id: ProjectId,
        branch_id: BranchId,
        initial_prompt: Option<String>,
        rows: usize,
        cols: usize,
        claude_config_id: Option<ClaudeConfigId>,
        claude_config_dir: Option<PathBuf>,
        claude_config_name: Option<String>,
    ) -> Result<SessionId> {
        let mut info = SessionInfo::with_claude_config(
            name.clone(),
            working_dir.clone(),
            project_id,
            branch_id,
            claude_config_id,
            claude_config_name,
        );
        let session_id = info.id;

        // Create spawn config
        let spawn_config = SpawnConfig {
            session_id,
            session_name: name,
            working_dir,
            initial_prompt,
            rows: rows as u16,
            cols: cols as u16,
            claude_config_dir,
            codex_home: None,
            resume: None,
        };

        // Get adapter and spawn the process
        let agent_type = AgentType::ClaudeCode;
        let adapter = agent_type.create_adapter();
        let spawn_result = adapter.spawn(&self.config, &spawn_config)?;

        // Record the agent's conversation ID so this session can be resumed
        // after a restart
        info.agent_session_id = spawn_result.agent_session_id;

        // Create session with PTY and virtual terminal
        let session = Session::with_scrollback(
            info,
            spawn_result.pty,
            rows,
            cols,
            self.config.scrollback_lines,
        );

        // Store session
        self.sessions.insert(session_id, session);
        self.session_order.push(session_id);
        self.persist_session(session_id);

        Ok(session_id)
    }

    /// Create a new shell session (bash/zsh)
    ///
    /// Shell sessions don't use hooks - state tracking is done via foreground detection.
    pub fn create_shell_session(
        &mut self,
        name: String,
        working_dir: PathBuf,
        project_id: ProjectId,
        branch_id: BranchId,
        rows: usize,
        cols: usize,
    ) -> Result<SessionId> {
        let info = SessionInfo::shell(name.clone(), working_dir.clone(), project_id, branch_id);
        let session_id = info.id;

        // Create spawn config
        let spawn_config = SpawnConfig {
            session_id,
            session_name: name,
            working_dir,
            initial_prompt: None, // Shell doesn't use initial prompts
            rows: rows as u16,
            cols: cols as u16,
            claude_config_dir: None,
            codex_home: None,
            resume: None,
        };

        // Get adapter and spawn the process
        let agent_type = AgentType::Shell;
        let adapter = agent_type.create_adapter();
        let spawn_result = adapter.spawn(&self.config, &spawn_config)?;

        // Create session with PTY and virtual terminal
        let session = Session::with_scrollback(
            info,
            spawn_result.pty,
            rows,
            cols,
            self.config.scrollback_lines,
        );

        // Store session
        self.sessions.insert(session_id, session);
        self.session_order.push(session_id);
        self.persist_session(session_id);

        Ok(session_id)
    }

    /// Create a new shell session with an initial command to execute
    ///
    /// Shell sessions don't use hooks - state tracking is done via foreground detection.
    /// The command is written to the PTY after the shell starts.
    #[allow(clippy::too_many_arguments)]
    pub fn create_shell_session_with_command(
        &mut self,
        name: String,
        working_dir: PathBuf,
        project_id: ProjectId,
        branch_id: BranchId,
        initial_command: String,
        rows: usize,
        cols: usize,
    ) -> Result<SessionId> {
        // Create the shell session first
        let session_id =
            self.create_shell_session(name, working_dir, project_id, branch_id, rows, cols)?;

        // Write the command to the PTY
        // The shell should be ready immediately, but we can write right away since
        // the PTY will buffer the input until the shell reads it
        if let Some(session) = self.sessions.get_mut(&session_id) {
            // Write the command followed by newline to execute it
            let command_with_newline = format!("{}\n", initial_command);
            if let Err(e) = session.pty.write(command_with_newline.as_bytes()) {
                tracing::warn!(
                    session_id = %session_id,
                    command = %initial_command,
                    error = %e,
                    "Failed to write initial command to shell session"
                );
            }
        }

        Ok(session_id)
    }

    /// Create a new Codex session
    #[allow(clippy::too_many_arguments)]
    pub fn create_codex_session(
        &mut self,
        name: String,
        working_dir: PathBuf,
        project_id: ProjectId,
        branch_id: BranchId,
        initial_prompt: Option<String>,
        rows: usize,
        cols: usize,
    ) -> Result<SessionId> {
        self.create_codex_session_with_config(
            name,
            working_dir,
            project_id,
            branch_id,
            initial_prompt,
            rows,
            cols,
            None,
            None,
            None,
        )
    }

    /// Create a new Codex session with a specific Codex configuration
    #[allow(clippy::too_many_arguments)]
    pub fn create_codex_session_with_config(
        &mut self,
        name: String,
        working_dir: PathBuf,
        project_id: ProjectId,
        branch_id: BranchId,
        initial_prompt: Option<String>,
        rows: usize,
        cols: usize,
        codex_config_id: Option<CodexConfigId>,
        codex_home: Option<PathBuf>,
        codex_config_name: Option<String>,
    ) -> Result<SessionId> {
        let mut info = SessionInfo::with_codex_config(
            name.clone(),
            working_dir.clone(),
            project_id,
            branch_id,
            codex_config_id,
            codex_config_name,
        );
        let session_id = info.id;

        // Create spawn config
        let spawn_config = SpawnConfig {
            session_id,
            session_name: name,
            working_dir,
            initial_prompt,
            rows: rows as u16,
            cols: cols as u16,
            claude_config_dir: None,
            codex_home,
            resume: None,
        };

        // Get adapter and spawn the process
        let agent_type = AgentType::OpenAICodex;
        let adapter = agent_type.create_adapter();
        let spawn_result = adapter.spawn(&self.config, &spawn_config)?;

        // Codex has no flag to dictate its session ID, so this stays None until
        // the rollout file is resolved (see step 4)
        info.agent_session_id = spawn_result.agent_session_id;

        // Create session with PTY and virtual terminal
        let session = Session::with_scrollback(
            info,
            spawn_result.pty,
            rows,
            cols,
            self.config.scrollback_lines,
        );

        // Store session
        self.sessions.insert(session_id, session);
        self.session_order.push(session_id);
        self.persist_session(session_id);

        Ok(session_id)
    }

    /// Destroy a session by ID
    pub fn destroy_session(&mut self, session_id: SessionId) -> Result<()> {
        if let Some(mut session) = self.sessions.remove(&session_id) {
            // Kill the PTY process
            if session.is_alive() {
                session.kill()?;
            }

            // Remove from order list
            self.session_order.retain(|&id| id != session_id);

            // Closing a session is an explicit discard, so drop its durable
            // record too. Quitting Panoptes deliberately does not do this.
            self.forget_session(session_id);

            Ok(())
        } else {
            Err(anyhow!("Session not found: {}", session_id))
        }
    }

    /// Get a session by ID
    pub fn get(&self, session_id: SessionId) -> Option<&Session> {
        self.sessions.get(&session_id)
    }

    /// Get a mutable session by ID
    pub fn get_mut(&mut self, session_id: SessionId) -> Option<&mut Session> {
        self.sessions.get_mut(&session_id)
    }

    /// Get session by index in order list
    pub fn get_by_index(&self, index: usize) -> Option<&Session> {
        self.session_order
            .get(index)
            .and_then(|id| self.sessions.get(id))
    }

    /// Get mutable session by index in order list
    pub fn get_by_index_mut(&mut self, index: usize) -> Option<&mut Session> {
        if let Some(&id) = self.session_order.get(index) {
            self.sessions.get_mut(&id)
        } else {
            None
        }
    }

    /// Get the index of a session in the order list
    pub fn index_of(&self, session_id: SessionId) -> Option<usize> {
        self.session_order.iter().position(|&id| id == session_id)
    }

    /// Get all sessions in order
    pub fn sessions_in_order(&self) -> Vec<&Session> {
        self.session_order
            .iter()
            .filter_map(|id| self.sessions.get(id))
            .collect()
    }

    /// Get all session IDs in order
    pub fn session_ids(&self) -> &[SessionId] {
        &self.session_order
    }

    /// Get the number of sessions
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Check if there are no sessions
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Poll all sessions for new output
    /// Returns list of session IDs that had new output
    /// Drains ALL available PTY data before returning to prevent rendering lag
    pub fn poll_outputs(&mut self) -> Vec<SessionId> {
        self.poll_outputs_except(None)
    }

    /// Poll all sessions for new output, optionally excluding one session.
    ///
    /// This is useful when the active session is scrolled up in history and the
    /// UI should "freeze" that view while still polling other sessions.
    pub fn poll_outputs_except(&mut self, excluded: Option<SessionId>) -> Vec<SessionId> {
        let mut sessions_with_output = Vec::new();

        for (&session_id, session) in &mut self.sessions {
            if excluded == Some(session_id) {
                continue;
            }
            let mut had_output = false;
            // Drain ALL available PTY data before returning
            while session.poll_output() {
                had_output = true;
            }
            if had_output {
                sessions_with_output.push(session_id);
            }
        }

        sessions_with_output
    }

    /// Check all sessions for exited processes
    /// Updates state to Exited for any dead sessions
    /// Returns a list of (session_id, session_name, exit_reason) for sessions that crashed
    pub fn check_alive(&mut self) -> Vec<(SessionId, String, String)> {
        let mut crashed_sessions = Vec::new();
        for session in self.sessions.values_mut() {
            if session.info.state != SessionState::Exited {
                // Check exit status to detect crashes vs normal termination
                if let Some(exit_info) = session.exit_status() {
                    let reason = exit_info.format_reason();
                    if exit_info.success {
                        tracing::debug!(
                            session_id = %session.info.id,
                            session_name = %session.info.name,
                            "Session exited normally"
                        );
                        session.info.exit_reason = None;
                    } else {
                        tracing::warn!(
                            session_id = %session.info.id,
                            session_name = %session.info.name,
                            exit_code = exit_info.code,
                            signal = ?exit_info.signal,
                            exit_reason = %reason,
                            "Session exited abnormally"
                        );
                        session.info.exit_reason = Some(reason.clone());
                        // Collect crashed sessions for notification
                        crashed_sessions.push((session.info.id, session.info.name.clone(), reason));
                    }
                    session.set_state(SessionState::Exited);
                }
            }
        }
        crashed_sessions
    }

    /// Check for sessions stuck in Executing state too long
    /// Transitions them to Idle state if they've been Executing longer than timeout_secs
    /// Returns true if any session state changed
    pub fn check_state_timeouts(&mut self, timeout_secs: u64) -> bool {
        let now = Utc::now();
        let mut changed = false;

        for session in self.sessions.values_mut() {
            if let SessionState::Executing(_) = &session.info.state {
                let elapsed = now
                    .signed_duration_since(session.info.state_entered_at)
                    .num_seconds();
                if elapsed > timeout_secs as i64 {
                    tracing::warn!(
                        session_id = %session.info.id,
                        session_name = %session.info.name,
                        elapsed_secs = elapsed,
                        "Session stuck in Executing state, transitioning to Idle"
                    );
                    session.set_state(SessionState::Idle);
                    changed = true;
                }
            }
        }
        changed
    }

    /// Check shell session states by polling foreground process detection
    ///
    /// For shell sessions (SessionType::Shell), this checks whether a command
    /// is currently running in the foreground and updates state accordingly:
    /// - Foreground busy -> Executing("command")
    /// - Foreground idle -> Waiting
    ///
    /// Returns a list of session IDs that transitioned from Executing to Waiting
    /// (these sessions need notifications).
    ///
    /// The `active_session` parameter indicates which session the user is currently viewing.
    /// Sessions that are active will not have `needs_attention` set or be included in the
    /// notification list, since the user is already looking at them.
    pub fn check_shell_states(&mut self, active_session: Option<SessionId>) -> Vec<SessionId> {
        use super::SessionType;

        let mut needs_notification = Vec::new();

        for session in self.sessions.values_mut() {
            // Only check shell sessions that haven't exited
            if session.info.session_type != SessionType::Shell {
                continue;
            }
            if session.info.state == SessionState::Exited {
                continue;
            }

            let is_busy = session.pty.is_foreground_busy();
            let current_is_executing = matches!(session.info.state, SessionState::Executing(_));

            if is_busy && !current_is_executing {
                // Transition to Executing
                session.set_state(SessionState::Executing("command".to_string()));
            } else if !is_busy && current_is_executing {
                // Transition to Waiting - command finished
                session.set_state(SessionState::Waiting);
                // Only set needs_attention and add to notification list if not the active session
                let is_active = active_session == Some(session.info.id);
                if !is_active {
                    session.info.needs_attention = true;
                    needs_notification.push(session.info.id);
                }
            }
        }

        needs_notification
    }

    /// Clean up exited sessions that have been exited longer than retention_secs
    /// Returns the number of sessions cleaned up
    pub fn cleanup_exited_sessions(&mut self, retention_secs: u64) -> usize {
        let now = Utc::now();
        let mut to_remove: Vec<SessionId> = Vec::new();

        for (session_id, session) in &self.sessions {
            if session.info.state == SessionState::Exited {
                if let Some(exited_at) = session.info.exited_at {
                    let elapsed = now.signed_duration_since(exited_at).num_seconds();
                    if elapsed > retention_secs as i64 {
                        tracing::debug!(
                            session_id = %session_id,
                            session_name = %session.info.name,
                            elapsed_secs = elapsed,
                            "Cleaning up exited session"
                        );
                        to_remove.push(*session_id);
                    }
                }
            }
        }

        let count = to_remove.len();
        for session_id in to_remove {
            self.sessions.remove(&session_id);
            // These aged out under the retention policy, which is an explicit
            // statement that they are no longer wanted
            self.forget_session(session_id);
        }
        count
    }

    /// Handle a hook event and update session state accordingly
    /// Returns the session ID if notification should be sent (session entered Waiting state)
    ///
    /// The `active_session` parameter indicates which session the user is currently viewing.
    /// If the session entering Waiting state is the active session, `needs_attention` will
    /// not be set and `None` will be returned (no notification needed).
    pub fn handle_hook_event(&mut self, event: &HookEvent) -> Option<SessionId> {
        // Try to parse session_id as UUID
        let session_id = match event.session_id.parse::<SessionId>() {
            Ok(id) => id,
            Err(_) => {
                tracing::warn!("Invalid session ID in hook event: {}", event.session_id);
                return None;
            }
        };

        // Find the session
        let session = match self.sessions.get_mut(&session_id) {
            Some(s) => s,
            None => {
                tracing::debug!("Hook event for unknown session: {}", session_id);
                return None;
            }
        };

        // Capture old state to check for transition to Waiting
        let old_state = session.info.state.clone();

        // Update state based on event type
        // Note: Any hook event means Claude Code is running (no longer Starting)
        let new_state = match event.event_type() {
            HookEventType::SessionStart => {
                // SessionStart just confirms Claude is running, transition to Waiting
                SessionState::Waiting
            }
            HookEventType::PreToolUse => {
                let tool = event.tool.clone().unwrap_or_else(|| "unknown".to_string());
                SessionState::Executing(tool)
            }
            HookEventType::PostToolUse => SessionState::Thinking,
            HookEventType::Stop => SessionState::Waiting,
            HookEventType::Notification => {
                // Notification usually means waiting for user (e.g., permission prompt)
                SessionState::Waiting
            }
            HookEventType::PermissionRequest => {
                // Permission dialog is showing - session needs user input
                SessionState::Waiting
            }
            HookEventType::AgentTurnComplete => {
                // Codex CLI agent turn complete - agent is waiting for user input
                SessionState::Waiting
            }
            HookEventType::Unknown => {
                // For unknown events, just update last_activity
                session.info.last_activity = Utc::now();
                return None;
            }
        };

        session.set_state(new_state.clone());

        // PermissionRequest/Notification always need attention regardless of state transition.
        // Other events only set attention on non-Waiting → Waiting transition.
        // Bell only rings on false → true transition to avoid duplicate notifications.
        let always_needs_attention = matches!(
            event.event_type(),
            HookEventType::PermissionRequest | HookEventType::Notification
        );

        if always_needs_attention
            || (new_state == SessionState::Waiting && old_state != SessionState::Waiting)
        {
            if session.info.needs_attention {
                // Already flagged — don't re-notify
                None
            } else {
                session.info.needs_attention = true;
                Some(session_id)
            }
        } else {
            None
        }
    }

    /// Send a notification based on the configured method
    /// - "bell": Ring the terminal bell
    /// - "title": Update terminal title with attention message
    /// - "none": Do nothing
    pub fn send_notification(method: &str, session_name: &str) {
        match method {
            "bell" => {
                print!("\x07"); // ASCII bell character
                std::io::stdout().flush().ok();
            }
            "title" => {
                // Update terminal title using OSC escape sequence
                // Format: ESC ] 0 ; title BEL
                print!("\x1b]0;[!] {} needs attention\x07", session_name);
                std::io::stdout().flush().ok();
            }
            "none" => {
                // Do nothing
            }
            _ => {
                // Unknown method, default to bell
                print!("\x07");
                std::io::stdout().flush().ok();
            }
        }
    }

    /// Ring the terminal bell (convenience method for backward compatibility)
    pub fn ring_terminal_bell() {
        print!("\x07"); // ASCII bell character
        std::io::stdout().flush().ok();
    }

    /// Reset the terminal title to default (used after "title" notification mode)
    pub fn reset_terminal_title() {
        // Reset to "Panoptes" as the default title
        print!("\x1b]0;Panoptes\x07");
        std::io::stdout().flush().ok();
    }

    /// Clear the attention flag for a session (called when user views it)
    pub fn acknowledge_attention(&mut self, session_id: SessionId) {
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.info.needs_attention = false;
        }
    }

    /// Resize all session PTYs and virtual terminals
    pub fn resize_all(&mut self, cols: u16, rows: u16) {
        for session in self.sessions.values_mut() {
            if let Err(e) = session.resize(cols, rows) {
                tracing::warn!("Failed to resize session {}: {}", session.info.id, e);
            }
        }
    }

    /// Get iterator over sessions
    pub fn iter(&self) -> impl Iterator<Item = (&SessionId, &Session)> {
        self.sessions.iter()
    }

    /// Get mutable iterator over sessions
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&SessionId, &mut Session)> {
        self.sessions.iter_mut()
    }

    /// Shutdown all sessions, killing any that are still alive
    ///
    /// This should be called when the application is exiting to ensure
    /// no orphaned Claude Code processes are left running.
    pub fn shutdown_all(&mut self) {
        tracing::info!("Shutting down {} session(s)", self.sessions.len());

        // Refresh the durable records before tearing down, so the recovery list
        // reflects the final state of this run. Records are deliberately kept:
        // quitting Panoptes is not the same as discarding your sessions.
        for session in self.sessions.values() {
            self.store.upsert(session.info.clone());
        }
        if let Err(e) = self.store.save() {
            tracing::warn!(
                error = %e,
                "Failed to persist session records on shutdown; recovery list may be stale"
            );
        }

        for (id, session) in self.sessions.iter_mut() {
            if session.is_alive() {
                tracing::debug!("Killing session {}", id);
                if let Err(e) = session.kill() {
                    tracing::warn!("Failed to kill session {}: {}", id, e);
                }
            }
        }

        self.sessions.clear();
        self.session_order.clear();
    }

    /// Get all sessions for a specific project
    pub fn sessions_for_project(&self, project_id: ProjectId) -> Vec<&Session> {
        self.sessions
            .values()
            .filter(|s| s.info.project_id == project_id)
            .collect()
    }

    /// Get all sessions for a specific branch
    pub fn sessions_for_branch(&self, branch_id: BranchId) -> Vec<&Session> {
        self.sessions
            .values()
            .filter(|s| s.info.branch_id == branch_id)
            .collect()
    }

    /// Get count of sessions for a project
    pub fn session_count_for_project(&self, project_id: ProjectId) -> usize {
        self.sessions
            .values()
            .filter(|s| s.info.project_id == project_id)
            .count()
    }

    /// Get count of sessions for a branch
    pub fn session_count_for_branch(&self, branch_id: BranchId) -> usize {
        self.sessions
            .values()
            .filter(|s| s.info.branch_id == branch_id)
            .count()
    }

    /// Get count of active sessions for a project
    pub fn active_session_count_for_project(&self, project_id: ProjectId) -> usize {
        self.sessions
            .values()
            .filter(|s| s.info.project_id == project_id && s.info.state.is_active())
            .count()
    }

    /// Get count of active sessions for a branch
    pub fn active_session_count_for_branch(&self, branch_id: BranchId) -> usize {
        self.sessions
            .values()
            .filter(|s| s.info.branch_id == branch_id && s.info.state.is_active())
            .count()
    }

    /// Get total count of active sessions across all projects
    pub fn total_active_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|s| s.info.state.is_active())
            .count()
    }

    /// Check if a session needs attention (needs_attention flag set, or Idle beyond threshold)
    ///
    /// The `needs_attention` flag is decoupled from state because subagents share a
    /// session_id — one subagent can be waiting for input while another is actively
    /// working, so the session may be in Thinking/Executing state while still needing
    /// attention. The flag is set only by hook events (not PTY output transitions),
    /// since hook events like PermissionRequest and Notification explicitly signal
    /// that user interaction is required.
    /// Takes `SessionInfo` rather than `Session` so that list views can call it
    /// for recovered sessions too, which have no process attached.
    pub fn session_needs_attention(&self, info: &SessionInfo, idle_threshold_secs: u64) -> bool {
        // Sticky flag: set by hook events requiring user attention, cleared on acknowledgment
        if info.needs_attention {
            return true;
        }
        // Idle beyond threshold always needs attention (time-based)
        if info.state == SessionState::Idle {
            let idle_duration = Utc::now().signed_duration_since(info.last_activity);
            return idle_duration.num_seconds() > idle_threshold_secs as i64;
        }
        false
    }

    /// Get all sessions needing attention, sorted by urgency (Waiting first, then by idle duration)
    pub fn sessions_needing_attention(&self, idle_threshold_secs: u64) -> Vec<&Session> {
        let mut sessions: Vec<_> = self
            .sessions
            .values()
            .filter(|s| self.session_needs_attention(&s.info, idle_threshold_secs))
            .collect();

        // Sort: sessions with needs_attention flag first, then by last_activity (oldest first)
        sessions.sort_by(|a, b| {
            match (a.info.needs_attention, b.info.needs_attention) {
                (true, true) | (false, false) => {
                    // Same priority: sort by oldest activity first
                    a.info.last_activity.cmp(&b.info.last_activity)
                }
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
            }
        });

        sessions
    }

    /// Count sessions needing attention for a project
    pub fn attention_count_for_project(
        &self,
        project_id: ProjectId,
        idle_threshold_secs: u64,
    ) -> usize {
        self.sessions
            .values()
            .filter(|s| {
                s.info.project_id == project_id
                    && self.session_needs_attention(&s.info, idle_threshold_secs)
            })
            .count()
    }

    /// Count sessions needing attention for a branch
    pub fn attention_count_for_branch(
        &self,
        branch_id: BranchId,
        idle_threshold_secs: u64,
    ) -> usize {
        self.sessions
            .values()
            .filter(|s| {
                s.info.branch_id == branch_id
                    && self.session_needs_attention(&s.info, idle_threshold_secs)
            })
            .count()
    }

    /// Total sessions needing attention globally
    pub fn total_attention_count(&self, idle_threshold_secs: u64) -> usize {
        self.sessions
            .values()
            .filter(|s| self.session_needs_attention(&s.info, idle_threshold_secs))
            .count()
    }
}

#[cfg(test)]
impl SessionManager {
    /// Insert a stub session for testing
    ///
    /// Spawns a `sleep` process to create a valid Session with minimal overhead.
    /// The session will have a real PTY but a short-lived process.
    pub fn insert_test_session(
        &mut self,
        name: &str,
        project_id: ProjectId,
        branch_id: BranchId,
    ) -> Result<SessionId> {
        use super::pty::PtyHandle;
        use super::{Session, SessionInfo};
        use std::collections::HashMap;

        let info = SessionInfo::new(
            name.to_string(),
            std::path::PathBuf::from("/tmp"),
            project_id,
            branch_id,
        );
        let session_id = info.id;

        // Spawn a simple sleep process for the PTY
        let pty = PtyHandle::spawn(
            "sleep",
            &["1"],
            &std::path::PathBuf::from("/tmp"),
            HashMap::new(),
            24,
            80,
        )?;
        let session = Session::new(info, pty, 24, 80);

        self.sessions.insert(session_id, session);
        self.session_order.push(session_id);

        Ok(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionType;
    use tempfile::TempDir;
    use uuid::Uuid;

    /// Build a manager backed by a temp store.
    ///
    /// Never use `SessionManager::new` in tests: it reads and writes the real
    /// `~/.panoptes/sessions.json`, which a running Panoptes instance owns.
    fn test_manager(temp_dir: &TempDir, config: Config) -> SessionManager {
        SessionManager::with_store(
            config,
            SessionStore::with_path(temp_dir.path().join("sessions.json")),
        )
    }

    fn test_config(temp_dir: &TempDir) -> Config {
        Config {
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            ..Config::default()
        }
    }

    #[test]
    fn test_session_manager_new() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(&temp_dir);
        let manager = test_manager(&temp_dir, config);

        assert!(manager.is_empty());
        assert_eq!(manager.len(), 0);
    }

    #[test]
    fn test_handle_hook_event_pre_tool_use() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(&temp_dir);
        let mut manager = test_manager(&temp_dir, config);

        // We can't easily create a real session without spawning a process,
        // so we'll test the event parsing logic indirectly
        let event = HookEvent {
            session_id: "not-a-valid-uuid".to_string(),
            event: "PreToolUse".to_string(),
            tool: Some("Bash".to_string()),
            timestamp: 1234567890,
        };

        // Should not panic with invalid session ID
        manager.handle_hook_event(&event);
    }

    #[test]
    fn test_handle_hook_event_types() {
        // Test that event type parsing works correctly
        let event = HookEvent {
            session_id: "test".to_string(),
            event: "PreToolUse".to_string(),
            tool: Some("Read".to_string()),
            timestamp: 123,
        };
        assert_eq!(event.event_type(), HookEventType::PreToolUse);

        let event = HookEvent {
            session_id: "test".to_string(),
            event: "PostToolUse".to_string(),
            tool: None,
            timestamp: 123,
        };
        assert_eq!(event.event_type(), HookEventType::PostToolUse);

        let event = HookEvent {
            session_id: "test".to_string(),
            event: "Stop".to_string(),
            tool: None,
            timestamp: 123,
        };
        assert_eq!(event.event_type(), HookEventType::Stop);
    }

    #[test]
    fn test_session_order_tracking() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(&temp_dir);
        let manager = test_manager(&temp_dir, config);

        // Initially empty
        assert!(manager.session_ids().is_empty());
        assert!(manager.sessions_in_order().is_empty());
    }

    #[test]
    fn test_get_by_index_empty() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(&temp_dir);
        let manager = test_manager(&temp_dir, config);

        assert!(manager.get_by_index(0).is_none());
        assert!(manager.get_by_index(100).is_none());
    }

    /// Helper: create a real session in the manager by spawning a lightweight process.
    /// Returns the session ID (which is also the UUID string for hook events).
    fn insert_test_session(manager: &mut SessionManager) -> SessionId {
        use crate::session::pty::PtyHandle;
        use crate::session::{Session, SessionInfo};
        use std::collections::HashMap;

        let project_id = uuid::Uuid::new_v4();
        let branch_id = uuid::Uuid::new_v4();
        let info = SessionInfo::new(
            "test-session".to_string(),
            "/tmp".into(),
            project_id,
            branch_id,
        );
        let session_id = info.id;

        let pty = PtyHandle::spawn(
            "sleep",
            &["60"],
            std::path::Path::new("/tmp"),
            HashMap::new(),
            24,
            80,
        )
        .expect("Failed to spawn test process");
        let session = Session::new(info, pty, 24, 80);

        manager.sessions.insert(session_id, session);
        manager.session_order.push(session_id);
        session_id
    }

    #[test]
    fn test_permission_request_sets_needs_attention_while_waiting() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(&temp_dir);
        let mut manager = test_manager(&temp_dir, config);
        let session_id = insert_test_session(&mut manager);

        // Put session into Waiting state (simulating a Stop event already processed)
        manager
            .get_mut(session_id)
            .unwrap()
            .set_state(SessionState::Waiting);
        assert!(!manager.get(session_id).unwrap().info.needs_attention);

        // PermissionRequest while already Waiting should set needs_attention and ring bell
        let event = HookEvent {
            session_id: session_id.to_string(),
            event: "PermissionRequest".to_string(),
            tool: None,
            timestamp: 100,
        };
        let result = manager.handle_hook_event(&event);
        assert_eq!(
            result,
            Some(session_id),
            "First PermissionRequest should ring bell"
        );
        assert!(manager.get(session_id).unwrap().info.needs_attention);

        // Second PermissionRequest while already flagged should NOT ring bell
        let event2 = HookEvent {
            session_id: session_id.to_string(),
            event: "PermissionRequest".to_string(),
            tool: None,
            timestamp: 200,
        };
        let result2 = manager.handle_hook_event(&event2);
        assert_eq!(
            result2, None,
            "Duplicate PermissionRequest should not ring bell"
        );
        assert!(manager.get(session_id).unwrap().info.needs_attention);

        // After acknowledgment, a third PermissionRequest should ring bell again
        manager.acknowledge_attention(session_id);
        assert!(!manager.get(session_id).unwrap().info.needs_attention);

        let event3 = HookEvent {
            session_id: session_id.to_string(),
            event: "PermissionRequest".to_string(),
            tool: None,
            timestamp: 300,
        };
        let result3 = manager.handle_hook_event(&event3);
        assert_eq!(
            result3,
            Some(session_id),
            "PermissionRequest after ack should ring bell"
        );
        assert!(manager.get(session_id).unwrap().info.needs_attention);
    }

    #[test]
    fn test_notification_sets_needs_attention_while_waiting() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(&temp_dir);
        let mut manager = test_manager(&temp_dir, config);
        let session_id = insert_test_session(&mut manager);

        // Put session into Waiting state
        manager
            .get_mut(session_id)
            .unwrap()
            .set_state(SessionState::Waiting);

        // Notification while already Waiting should set needs_attention
        let event = HookEvent {
            session_id: session_id.to_string(),
            event: "Notification".to_string(),
            tool: None,
            timestamp: 100,
        };
        let result = manager.handle_hook_event(&event);
        assert_eq!(result, Some(session_id));
        assert!(manager.get(session_id).unwrap().info.needs_attention);
    }

    #[test]
    fn test_stop_while_already_waiting_does_not_set_attention() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(&temp_dir);
        let mut manager = test_manager(&temp_dir, config);
        let session_id = insert_test_session(&mut manager);

        // Put session into Waiting state
        manager
            .get_mut(session_id)
            .unwrap()
            .set_state(SessionState::Waiting);

        // Stop event while already Waiting should NOT set needs_attention
        // (Waiting → Waiting is not a non-Waiting → Waiting transition)
        let event = HookEvent {
            session_id: session_id.to_string(),
            event: "Stop".to_string(),
            tool: None,
            timestamp: 100,
        };
        let result = manager.handle_hook_event(&event);
        assert_eq!(result, None);
        assert!(!manager.get(session_id).unwrap().info.needs_attention);
    }

    #[test]
    fn test_destroy_nonexistent_session() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(&temp_dir);
        let mut manager = test_manager(&temp_dir, config);

        let fake_id = uuid::Uuid::new_v4();
        let result = manager.destroy_session(fake_id);
        assert!(result.is_err());
    }

    #[test]
    #[cfg(unix)]
    fn test_check_shell_states_sets_needs_attention() {
        use crate::session::pty::PtyHandle;
        use crate::session::{Session, SessionInfo, SessionType};
        use std::collections::HashMap;

        let temp_dir = TempDir::new().unwrap();
        let config = test_config(&temp_dir);
        let mut manager = test_manager(&temp_dir, config);

        let project_id = uuid::Uuid::new_v4();
        let branch_id = uuid::Uuid::new_v4();

        // Create a shell session info
        let info = SessionInfo::shell(
            "test-shell".to_string(),
            std::path::PathBuf::from("/tmp"),
            project_id,
            branch_id,
        );
        let session_id = info.id;

        // Spawn a very short-lived process so it exits quickly
        let pty = PtyHandle::spawn(
            "true",
            &[],
            &std::path::PathBuf::from("/tmp"),
            HashMap::new(),
            24,
            80,
        )
        .unwrap();

        let mut session = Session::new(info, pty, 24, 80);

        // Manually set the session to Executing state to simulate a command running
        session.set_state(SessionState::Executing("test-command".to_string()));
        assert_eq!(session.info.session_type, SessionType::Shell);
        assert!(!session.info.needs_attention);

        manager.sessions.insert(session_id, session);
        manager.session_order.push(session_id);

        // Wait for the process to exit so foreground becomes idle
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Call check_shell_states - should detect transition from Executing to Waiting
        // Pass None for active_session so the session gets flagged for attention
        let notified = manager.check_shell_states(None);

        // The session should have transitioned and be in the notification list
        assert!(
            notified.contains(&session_id),
            "Session should be in notification list after transitioning to Waiting"
        );

        // Verify needs_attention is set
        let session = manager.get(session_id).unwrap();
        assert!(
            session.info.needs_attention,
            "needs_attention should be true after command completion"
        );
        assert_eq!(
            session.info.state,
            SessionState::Waiting,
            "State should be Waiting after command completion"
        );
    }

    // Persistence lifecycle
    //
    // These use shell sessions because they spawn without needing a Claude or
    // Codex binary on PATH.

    #[test]
    fn test_creating_a_session_persists_its_record() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("sessions.json");
        let mut manager =
            SessionManager::with_store(test_config(&temp_dir), SessionStore::with_path(store_path));

        let session_id = manager
            .create_shell_session(
                "persisted".to_string(),
                PathBuf::from("/tmp"),
                Uuid::new_v4(),
                Uuid::new_v4(),
                24,
                80,
            )
            .unwrap();

        // Written immediately, not deferred to shutdown - a crash one second
        // after creation must still leave a usable record
        let record = manager
            .store()
            .get(session_id)
            .expect("record should exist");
        assert_eq!(record.name, "persisted");
        assert_eq!(record.working_dir, PathBuf::from("/tmp"));

        manager.shutdown_all();
    }

    #[test]
    fn test_record_survives_a_reload_from_disk() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("sessions.json");
        let mut manager = SessionManager::with_store(
            test_config(&temp_dir),
            SessionStore::with_path(store_path.clone()),
        );

        let session_id = manager
            .create_shell_session(
                "survivor".to_string(),
                PathBuf::from("/tmp"),
                Uuid::new_v4(),
                Uuid::new_v4(),
                24,
                80,
            )
            .unwrap();
        manager.shutdown_all();

        // Simulates the next Panoptes launch reading what the previous run left
        let reloaded = SessionStore::load_from(&store_path).unwrap();
        assert_eq!(
            reloaded.get(session_id).map(|s| s.name.as_str()),
            Some("survivor")
        );
    }

    #[test]
    fn test_quitting_panoptes_keeps_records() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("sessions.json");
        let mut manager = SessionManager::with_store(
            test_config(&temp_dir),
            SessionStore::with_path(store_path.clone()),
        );

        manager
            .create_shell_session(
                "kept".to_string(),
                PathBuf::from("/tmp"),
                Uuid::new_v4(),
                Uuid::new_v4(),
                24,
                80,
            )
            .unwrap();

        manager.shutdown_all();

        // Quitting is not the same as discarding: the session must still be
        // offered as recoverable on the next launch
        assert_eq!(manager.store().len(), 1);
        assert_eq!(SessionStore::load_from(&store_path).unwrap().len(), 1);
    }

    #[test]
    fn test_closing_a_session_discards_its_record() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("sessions.json");
        let mut manager = SessionManager::with_store(
            test_config(&temp_dir),
            SessionStore::with_path(store_path.clone()),
        );

        let session_id = manager
            .create_shell_session(
                "discarded".to_string(),
                PathBuf::from("/tmp"),
                Uuid::new_v4(),
                Uuid::new_v4(),
                24,
                80,
            )
            .unwrap();
        assert_eq!(manager.store().len(), 1);

        manager.destroy_session(session_id).unwrap();

        // Explicitly closing a session is intent to discard it
        assert!(manager.store().get(session_id).is_none());
        assert!(SessionStore::load_from(&store_path).unwrap().is_empty());
    }

    // Startup reconciliation

    /// A store pre-populated as if a previous Panoptes run had left it behind
    fn store_with_record(dir: &TempDir, mutate: impl FnOnce(&mut SessionInfo)) -> SessionStore {
        let mut store = SessionStore::with_path(dir.path().join("sessions.json"));
        let mut info = SessionInfo::new(
            "from-last-run".to_string(),
            dir.path().to_path_buf(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        info.agent_session_id = Some(Uuid::new_v4().to_string());
        mutate(&mut info);
        store.upsert(info);
        store
    }

    #[test]
    fn test_stored_sessions_come_back_as_resumable() {
        let temp_dir = TempDir::new().unwrap();
        let store = store_with_record(&temp_dir, |_| {});
        let manager = SessionManager::with_store(test_config(&temp_dir), store);

        assert_eq!(manager.recovered_count(), 1);
        let recovered = manager.recovered().next().unwrap();
        assert_eq!(recovered.state, SessionState::Resumable);
        assert_eq!(recovered.name, "from-last-run");
    }

    #[test]
    fn test_reconciliation_discards_state_from_the_dead_process() {
        let temp_dir = TempDir::new().unwrap();
        // The previous run crashed mid-turn with an unread notification
        let store = store_with_record(&temp_dir, |info| {
            info.state = SessionState::Thinking;
            info.needs_attention = true;
            info.exit_reason = Some("killed".to_string());
            info.exited_at = Some(Utc::now());
        });
        let manager = SessionManager::with_store(test_config(&temp_dir), store);

        let recovered = manager.recovered().next().unwrap();
        // None of this describes anything that still exists
        assert_eq!(recovered.state, SessionState::Resumable);
        assert!(!recovered.needs_attention);
        assert!(recovered.exit_reason.is_none());
        assert!(recovered.exited_at.is_none());
    }

    #[test]
    fn test_recovered_sessions_are_not_live_sessions() {
        let temp_dir = TempDir::new().unwrap();
        let store = store_with_record(&temp_dir, |_| {});
        let manager = SessionManager::with_store(test_config(&temp_dir), store);

        // Nothing is running: recovery is on demand, not automatic
        assert!(manager.is_empty());
        assert_eq!(manager.len(), 0);
        assert_eq!(manager.total_active_count(), 0);
    }

    #[test]
    fn test_entries_list_live_sessions_before_recovered_ones() {
        let temp_dir = TempDir::new().unwrap();
        let store = store_with_record(&temp_dir, |_| {});
        let mut manager = SessionManager::with_store(test_config(&temp_dir), store);

        manager
            .create_shell_session(
                "live".to_string(),
                PathBuf::from("/tmp"),
                Uuid::new_v4(),
                Uuid::new_v4(),
                24,
                80,
            )
            .unwrap();

        let entries = manager.entries_in_order();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].live);
        assert_eq!(entries[0].info.name, "live");
        assert!(!entries[1].live);
        assert_eq!(entries[1].info.name, "from-last-run");

        manager.shutdown_all();
    }

    #[test]
    fn test_discarding_a_recovered_session_removes_it_permanently() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("sessions.json");
        let store = store_with_record(&temp_dir, |_| {});
        store.save().unwrap();
        let mut manager = SessionManager::with_store(test_config(&temp_dir), store);

        let session_id = manager.recovered().next().unwrap().id;
        assert!(manager.discard_recovered(session_id));

        assert_eq!(manager.recovered_count(), 0);
        assert!(manager.entries_in_order().is_empty());
        // Must not reappear on the next launch
        assert!(SessionStore::load_from(&store_path).unwrap().is_empty());
        // Discarding something that is not recovered is a no-op, not a panic
        assert!(!manager.discard_recovered(session_id));
    }

    #[test]
    fn test_missing_working_directory_blocks_resume() {
        let temp_dir = TempDir::new().unwrap();
        let store = store_with_record(&temp_dir, |info| {
            info.working_dir = PathBuf::from("/nonexistent/worktree/deleted");
        });
        let manager = SessionManager::with_store(test_config(&temp_dir), store);

        let recovered = manager.recovered().next().unwrap();
        // Still listed - surfacing why it cannot come back beats hiding it
        assert_eq!(recovered.state, SessionState::Resumable);
        assert!(!recovered.is_resumable());
        assert_eq!(
            recovered.resume_blocker(),
            Some("working directory is missing")
        );
    }

    #[test]
    fn test_agent_session_without_a_conversation_id_blocks_resume() {
        let temp_dir = TempDir::new().unwrap();
        let store = store_with_record(&temp_dir, |info| {
            info.session_type = SessionType::OpenAICodex;
            info.agent_session_id = None;
        });
        let manager = SessionManager::with_store(test_config(&temp_dir), store);

        let recovered = manager.recovered().next().unwrap();
        assert_eq!(
            recovered.resume_blocker(),
            Some("no conversation was recorded")
        );
    }

    #[test]
    fn test_shell_needs_no_conversation_id_to_be_resumable() {
        let temp_dir = TempDir::new().unwrap();
        let store = store_with_record(&temp_dir, |info| {
            info.session_type = SessionType::Shell;
            info.agent_session_id = None;
        });
        let manager = SessionManager::with_store(test_config(&temp_dir), store);

        // Shells are restored by respawning in the same directory
        let recovered = manager.recovered().next().unwrap();
        assert!(recovered.is_resumable(), "shell should be restorable");
    }

    // Resume

    #[test]
    fn test_resuming_a_shell_brings_it_back_as_a_live_session() {
        let temp_dir = TempDir::new().unwrap();
        let store = store_with_record(&temp_dir, |info| {
            info.session_type = SessionType::Shell;
            info.agent_session_id = None;
        });
        let mut manager = SessionManager::with_store(test_config(&temp_dir), store);
        let session_id = manager.recovered().next().unwrap().id;

        let resumed = manager
            .resume_session(session_id, 24, 80, None, None)
            .unwrap();

        // The Panoptes session ID is preserved, which is what keeps hook
        // routing working across the restart
        assert_eq!(resumed, session_id);
        assert!(manager.get(session_id).is_some());
        assert_eq!(manager.recovered_count(), 0);
        assert_eq!(manager.len(), 1);

        manager.shutdown_all();
    }

    #[test]
    fn test_resumed_session_appears_once_and_as_live() {
        let temp_dir = TempDir::new().unwrap();
        let store = store_with_record(&temp_dir, |info| {
            info.session_type = SessionType::Shell;
            info.agent_session_id = None;
        });
        let mut manager = SessionManager::with_store(test_config(&temp_dir), store);
        let session_id = manager.recovered().next().unwrap().id;

        manager
            .resume_session(session_id, 24, 80, None, None)
            .unwrap();

        let entries = manager.entries_in_order();
        assert_eq!(entries.len(), 1, "resumed session must not be listed twice");
        assert!(entries[0].live);
        assert_eq!(entries[0].info.state, SessionState::Starting);

        manager.shutdown_all();
    }

    #[test]
    fn test_resume_refuses_when_blocked_and_keeps_the_record() {
        let temp_dir = TempDir::new().unwrap();
        let store = store_with_record(&temp_dir, |info| {
            info.working_dir = PathBuf::from("/nonexistent/worktree/deleted");
        });
        let mut manager = SessionManager::with_store(test_config(&temp_dir), store);
        let session_id = manager.recovered().next().unwrap().id;

        let err = manager
            .resume_session(session_id, 24, 80, None, None)
            .unwrap_err();

        assert!(
            err.to_string().contains("working directory is missing"),
            "error should explain why: {err}"
        );
        // A failed resume must not consume the record - the user can still
        // retry or discard it
        assert_eq!(manager.recovered_count(), 1);
        assert!(manager.is_empty());
    }

    #[test]
    fn test_resuming_an_unknown_session_is_an_error() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = SessionManager::with_store(
            test_config(&temp_dir),
            SessionStore::with_path(temp_dir.path().join("sessions.json")),
        );

        let err = manager
            .resume_session(Uuid::new_v4(), 24, 80, None, None)
            .unwrap_err();

        assert!(err.to_string().contains("No recovered session"));
    }

    #[test]
    fn test_resuming_twice_fails_the_second_time() {
        let temp_dir = TempDir::new().unwrap();
        let store = store_with_record(&temp_dir, |info| {
            info.session_type = SessionType::Shell;
            info.agent_session_id = None;
        });
        let mut manager = SessionManager::with_store(test_config(&temp_dir), store);
        let session_id = manager.recovered().next().unwrap().id;

        manager
            .resume_session(session_id, 24, 80, None, None)
            .unwrap();

        // Guards against spawning a second process for one conversation
        assert!(manager
            .resume_session(session_id, 24, 80, None, None)
            .is_err());
        assert_eq!(manager.len(), 1);

        manager.shutdown_all();
    }

    #[test]
    fn test_resumed_session_is_still_persisted() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("sessions.json");
        let store = store_with_record(&temp_dir, |info| {
            info.session_type = SessionType::Shell;
            info.agent_session_id = None;
        });
        let mut manager = SessionManager::with_store(test_config(&temp_dir), store);
        let session_id = manager.recovered().next().unwrap().id;

        manager
            .resume_session(session_id, 24, 80, None, None)
            .unwrap();

        // Resuming must not lose the record: a crash right after resuming has
        // to leave the session recoverable again
        assert!(SessionStore::load_from(&store_path)
            .unwrap()
            .get(session_id)
            .is_some());

        manager.shutdown_all();
    }

    // Codex conversation ID resolution

    #[test]
    fn test_shell_and_claude_sessions_are_never_pending_codex_resolution() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = SessionManager::with_store(
            test_config(&temp_dir),
            SessionStore::with_path(temp_dir.path().join("sessions.json")),
        );

        manager
            .create_shell_session(
                "shell".to_string(),
                PathBuf::from("/tmp"),
                Uuid::new_v4(),
                Uuid::new_v4(),
                24,
                80,
            )
            .unwrap();

        // Claude dictates its ID at spawn and shells have none to find, so
        // neither should ever trigger a filesystem scan
        assert!(manager.sessions_pending_codex_id().is_empty());

        manager.shutdown_all();
    }

    #[test]
    fn test_setting_the_conversation_id_persists_it() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("sessions.json");
        let mut manager = SessionManager::with_store(
            test_config(&temp_dir),
            SessionStore::with_path(store_path.clone()),
        );

        let session_id = manager
            .create_shell_session(
                "session".to_string(),
                PathBuf::from("/tmp"),
                Uuid::new_v4(),
                Uuid::new_v4(),
                24,
                80,
            )
            .unwrap();

        assert!(manager.set_agent_session_id(session_id, "codex-abc".to_string()));

        // An ID that only exists in memory is the pointer this whole mechanism
        // exists to stop losing
        let stored = SessionStore::load_from(&store_path).unwrap();
        assert_eq!(
            stored.get(session_id).unwrap().agent_session_id.as_deref(),
            Some("codex-abc")
        );

        manager.shutdown_all();
    }

    #[test]
    fn test_setting_the_same_conversation_id_twice_is_a_no_op() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = SessionManager::with_store(
            test_config(&temp_dir),
            SessionStore::with_path(temp_dir.path().join("sessions.json")),
        );

        let session_id = manager
            .create_shell_session(
                "session".to_string(),
                PathBuf::from("/tmp"),
                Uuid::new_v4(),
                Uuid::new_v4(),
                24,
                80,
            )
            .unwrap();

        assert!(manager.set_agent_session_id(session_id, "codex-abc".to_string()));
        // Avoids rewriting the store on every scan once resolved
        assert!(!manager.set_agent_session_id(session_id, "codex-abc".to_string()));

        manager.shutdown_all();
    }

    #[test]
    fn test_setting_the_conversation_id_of_an_unknown_session_is_a_no_op() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = SessionManager::with_store(
            test_config(&temp_dir),
            SessionStore::with_path(temp_dir.path().join("sessions.json")),
        );

        assert!(!manager.set_agent_session_id(Uuid::new_v4(), "codex-abc".to_string()));
    }

    #[test]
    fn test_shell_sessions_have_no_agent_conversation_to_resume() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("sessions.json");
        let mut manager =
            SessionManager::with_store(test_config(&temp_dir), SessionStore::with_path(store_path));

        let session_id = manager
            .create_shell_session(
                "shell".to_string(),
                PathBuf::from("/tmp"),
                Uuid::new_v4(),
                Uuid::new_v4(),
                24,
                80,
            )
            .unwrap();

        // Shells are restored by respawning in the same directory, not by
        // resuming a conversation - there is nothing to resume
        let record = manager.store().get(session_id).unwrap();
        assert!(record.agent_session_id.is_none());

        manager.shutdown_all();
    }
}
