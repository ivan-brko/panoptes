//! Persistence for focus session data
//!
//! Stores completed focus sessions to JSON file for historical tracking.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;

use crate::config::config_dir;

use super::stats::FocusSession;

const FOCUS_SESSIONS_FILE: &str = "focus_sessions.json";

/// Store for persisting focus session data
#[derive(Debug)]
pub struct FocusStore {
    store_path: PathBuf,
}

impl FocusStore {
    /// Create a new focus store
    pub fn new() -> Self {
        Self {
            store_path: config_dir().join(FOCUS_SESSIONS_FILE),
        }
    }

    /// Get the path to the store file
    pub fn path(&self) -> &PathBuf {
        &self.store_path
    }

    /// Load all sessions from disk
    pub fn load(&self) -> Result<Vec<FocusSession>> {
        if !self.store_path.exists() {
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(&self.store_path)
            .context("Failed to read focus sessions file")?;

        if content.trim().is_empty() {
            return Ok(Vec::new());
        }

        serde_json::from_str(&content).context("Failed to parse focus sessions")
    }

    /// Save all sessions to disk
    pub fn save(&self, sessions: &[FocusSession]) -> Result<()> {
        let content =
            serde_json::to_string_pretty(sessions).context("Failed to serialize focus sessions")?;

        std::fs::write(&self.store_path, content).context("Failed to write focus sessions file")?;

        Ok(())
    }

    /// Add a single session and save
    pub fn add_session(&self, session: FocusSession) -> Result<()> {
        let mut sessions = self.load()?;
        sessions.push(session);
        self.save(&sessions)
    }

    /// Prune sessions older than the retention period and save
    pub fn prune_old_sessions(&self, retention_days: u64) -> Result<usize> {
        let mut sessions = self.load()?;
        let cutoff = Utc::now() - chrono::Duration::days(retention_days as i64);
        let original_count = sessions.len();

        sessions.retain(|s| s.completed_at >= cutoff);

        let pruned = original_count - sessions.len();
        if pruned > 0 {
            self.save(&sessions)?;
        }

        Ok(pruned)
    }

    /// Get sessions within a time window
    pub fn sessions_in_window(&self, window: Duration) -> Result<Vec<FocusSession>> {
        let sessions = self.load()?;
        let cutoff = Utc::now() - chrono::Duration::seconds(window.as_secs() as i64);

        Ok(sessions
            .into_iter()
            .filter(|s| s.completed_at >= cutoff)
            .collect())
    }

    /// Get total session count
    pub fn session_count(&self) -> Result<usize> {
        Ok(self.load()?.len())
    }
}

impl Default for FocusStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_store(temp_dir: &TempDir) -> FocusStore {
        FocusStore {
            store_path: temp_dir.path().join(FOCUS_SESSIONS_FILE),
        }
    }

    #[test]
    fn test_load_empty() {
        let temp_dir = TempDir::new().unwrap();
        let store = test_store(&temp_dir);

        let sessions = store.load().unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn test_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let store = test_store(&temp_dir);

        let session = FocusSession::from_timer_result(
            Duration::from_secs(25 * 60),
            Duration::from_secs(20 * 60),
            Duration::from_secs(30 * 60),
            None,
            None,
        );

        store.save(&[session.clone()]).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, session.id);
    }

    #[test]
    fn test_add_session() {
        let temp_dir = TempDir::new().unwrap();
        let store = test_store(&temp_dir);

        let session1 = FocusSession::from_timer_result(
            Duration::from_secs(25 * 60),
            Duration::from_secs(25 * 60),
            Duration::from_secs(25 * 60),
            None,
            None,
        );
        let session2 = FocusSession::from_timer_result(
            Duration::from_secs(15 * 60),
            Duration::from_secs(15 * 60),
            Duration::from_secs(15 * 60),
            None,
            None,
        );

        store.add_session(session1).unwrap();
        store.add_session(session2).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), 2);
    }

    #[test]
    fn test_session_count() {
        let temp_dir = TempDir::new().unwrap();
        let store = test_store(&temp_dir);

        assert_eq!(store.session_count().unwrap(), 0);

        let session = FocusSession::from_timer_result(
            Duration::from_secs(25 * 60),
            Duration::from_secs(25 * 60),
            Duration::from_secs(25 * 60),
            None,
            None,
        );
        store.add_session(session).unwrap();

        assert_eq!(store.session_count().unwrap(), 1);
    }
}
