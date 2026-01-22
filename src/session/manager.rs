//! Session manager module
//!
//! This module provides centralized management of Claude Code sessions,
//! handling creation, destruction, state updates, and I/O polling.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use chrono::Utc;

use crate::agent::adapter::SpawnConfig;
use crate::agent::AgentType;
use crate::config::Config;
use crate::hooks::{HookEvent, HookEventType};
use crate::project::{BranchId, ProjectId};

use super::{Session, SessionId, SessionInfo, SessionState};

/// Manages multiple Claude Code sessions
pub struct SessionManager {
    /// All active sessions, keyed by session ID
    sessions: HashMap<SessionId, Session>,
    /// Session order (for navigation)
    session_order: Vec<SessionId>,
    /// Application configuration
    config: Config,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new(config: Config) -> Self {
        Self {
            sessions: HashMap::new(),
            session_order: Vec::new(),
            config,
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
        let info = SessionInfo::new(name.clone(), working_dir.clone(), project_id, branch_id);
        let session_id = info.id;

        // Create spawn config
        let spawn_config = SpawnConfig {
            session_id,
            session_name: name,
            working_dir,
            initial_prompt,
            rows: rows as u16,
            cols: cols as u16,
        };

        // Get adapter and spawn the process
        let agent_type = AgentType::ClaudeCode;
        let adapter = agent_type.create_adapter();
        let spawn_result = adapter.spawn(&self.config, &spawn_config)?;

        // Create session with PTY and virtual terminal
        let session = Session::new(info, spawn_result.pty, rows, cols);

        // Store session
        self.sessions.insert(session_id, session);
        self.session_order.push(session_id);

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
        let mut sessions_with_output = Vec::new();

        for (&session_id, session) in &mut self.sessions {
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
    /// Returns true if any session state changed
    pub fn check_alive(&mut self) -> bool {
        let mut changed = false;
        for session in self.sessions.values_mut() {
            if !session.is_alive() && session.info.state != SessionState::Exited {
                session.set_state(SessionState::Exited);
                changed = true;
            }
        }
        changed
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
        }
        count
    }

    /// Handle a hook event and update session state accordingly
    /// Returns the session ID if terminal bell should be rung (session entered Waiting state)
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
            HookEventType::Unknown => {
                // For unknown events, just update last_activity
                session.info.last_activity = Utc::now();
                return None;
            }
        };

        session.set_state(new_state.clone());

        // Return session ID if bell should ring (entered Waiting from non-Waiting)
        if new_state == SessionState::Waiting && old_state != SessionState::Waiting {
            session.info.needs_attention = true;
            Some(session_id)
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

    /// Check if a session needs attention (Waiting with flag set, or Idle beyond threshold)
    pub fn session_needs_attention(&self, session: &Session, idle_threshold_secs: u64) -> bool {
        match session.info.state {
            SessionState::Waiting => session.info.needs_attention, // Use flag (cleared when viewed)
            SessionState::Idle => {
                // Idle beyond threshold always needs attention (time-based)
                let idle_duration = Utc::now().signed_duration_since(session.info.last_activity);
                idle_duration.num_seconds() > idle_threshold_secs as i64
            }
            _ => false,
        }
    }

    /// Get all sessions needing attention, sorted by urgency (Waiting first, then by idle duration)
    pub fn sessions_needing_attention(&self, idle_threshold_secs: u64) -> Vec<&Session> {
        let mut sessions: Vec<_> = self
            .sessions
            .values()
            .filter(|s| self.session_needs_attention(s, idle_threshold_secs))
            .collect();

        // Sort: Waiting sessions first, then by last_activity (oldest first for Idle)
        sessions.sort_by(|a, b| {
            match (&a.info.state, &b.info.state) {
                (SessionState::Waiting, SessionState::Waiting) => {
                    // Both waiting: sort by oldest activity first
                    a.info.last_activity.cmp(&b.info.last_activity)
                }
                (SessionState::Waiting, _) => std::cmp::Ordering::Less,
                (_, SessionState::Waiting) => std::cmp::Ordering::Greater,
                _ => {
                    // Both idle: sort by oldest activity first
                    a.info.last_activity.cmp(&b.info.last_activity)
                }
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
                    && self.session_needs_attention(s, idle_threshold_secs)
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
                    && self.session_needs_attention(s, idle_threshold_secs)
            })
            .count()
    }

    /// Total sessions needing attention globally
    pub fn total_attention_count(&self, idle_threshold_secs: u64) -> usize {
        self.sessions
            .values()
            .filter(|s| self.session_needs_attention(s, idle_threshold_secs))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(temp_dir: &TempDir) -> Config {
        Config {
            hook_port: 9999,
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            max_output_lines: 1000,
            idle_threshold_secs: 300,
            state_timeout_secs: 300,
            exited_retention_secs: 300,
            theme_preset: "dark".to_string(),
            notification_method: "bell".to_string(),
            esc_hold_threshold_ms: 400,
            focus_timer_minutes: 25,
            focus_stats_retention_days: 30,
        }
    }

    #[test]
    fn test_session_manager_new() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(&temp_dir);
        let manager = SessionManager::new(config);

        assert!(manager.is_empty());
        assert_eq!(manager.len(), 0);
    }

    #[test]
    fn test_handle_hook_event_pre_tool_use() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(&temp_dir);
        let mut manager = SessionManager::new(config);

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
        let manager = SessionManager::new(config);

        // Initially empty
        assert!(manager.session_ids().is_empty());
        assert!(manager.sessions_in_order().is_empty());
    }

    #[test]
    fn test_get_by_index_empty() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(&temp_dir);
        let manager = SessionManager::new(config);

        assert!(manager.get_by_index(0).is_none());
        assert!(manager.get_by_index(100).is_none());
    }

    #[test]
    fn test_destroy_nonexistent_session() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(&temp_dir);
        let mut manager = SessionManager::new(config);

        let fake_id = uuid::Uuid::new_v4();
        let result = manager.destroy_session(fake_id);
        assert!(result.is_err());
    }
}
