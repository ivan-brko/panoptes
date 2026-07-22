//! Session persistence
//!
//! Persists session metadata to disk so that sessions can be recovered after
//! Panoptes exits or crashes.
//!
//! Panoptes does not store conversation content. The agents already do that:
//! Claude Code writes `~/.claude/projects/<cwd-slug>/<session-uuid>.jsonl` and
//! Codex writes `~/.codex/sessions/<date>/rollout-<ts>-<uuid>.jsonl`. Those
//! transcripts survive a crash on their own, but they are anonymous - nothing
//! on disk connects a UUID to "the auth refactor on the feature/oauth
//! worktree". This store is that index: it records which agent conversation
//! belongs to which session, along with the context needed to relaunch it
//! (working directory, project, branch, and account config).
//!
//! What is deliberately *not* recovered is the PTY scrollback. It is a buffer
//! of rendered terminal cells with no semantic structure, and agents replay
//! their own history into the TUI when resumed.

use super::{SessionId, SessionInfo};
use crate::config::config_dir;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Serializable format for the session store
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreData {
    sessions: Vec<SessionInfo>,
}

/// Backup a corrupted file by renaming it with a timestamped extension
///
/// Returns the backup path on success
fn backup_corrupted_file(path: &Path) -> Option<PathBuf> {
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let backup_path = path.with_extension(format!("json.corrupt.{}", timestamp));
    if let Err(e) = std::fs::rename(path, &backup_path) {
        tracing::warn!(
            "Failed to backup corrupted file {} to {}: {}",
            path.display(),
            backup_path.display(),
            e
        );
        None
    } else {
        tracing::info!(
            "Corrupted sessions file backed up to {}",
            backup_path.display()
        );
        Some(backup_path)
    }
}

/// Store for persisting session metadata
#[derive(Debug)]
pub struct SessionStore {
    /// All persisted sessions indexed by ID
    sessions: HashMap<SessionId, SessionInfo>,
    /// Path to the sessions.json file
    store_path: PathBuf,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self {
            sessions: HashMap::new(),
            store_path: sessions_file_path(),
        }
    }
}

impl SessionStore {
    /// Create a new empty store
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a store with a custom path (for testing)
    pub fn with_path(path: PathBuf) -> Self {
        Self {
            sessions: HashMap::new(),
            store_path: path,
        }
    }

    /// Get all persisted sessions
    pub fn sessions(&self) -> impl Iterator<Item = &SessionInfo> {
        self.sessions.values()
    }

    /// Get all persisted sessions sorted by last activity (most recent first)
    pub fn sessions_sorted(&self) -> Vec<&SessionInfo> {
        let mut sessions: Vec<_> = self.sessions.values().collect();
        sessions.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
        sessions
    }

    /// Get a persisted session by ID
    pub fn get(&self, id: SessionId) -> Option<&SessionInfo> {
        self.sessions.get(&id)
    }

    /// Insert or replace a session record
    pub fn upsert(&mut self, info: SessionInfo) {
        self.sessions.insert(info.id, info);
    }

    /// Remove a session record
    pub fn remove(&mut self, id: SessionId) -> Option<SessionInfo> {
        self.sessions.remove(&id)
    }

    /// Get the number of persisted sessions
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Check whether the store is empty
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Load store from disk
    pub fn load() -> Result<Self> {
        Self::load_from(&sessions_file_path())
    }

    /// Load store from disk, returning a warning message if data was corrupted
    ///
    /// A corrupted session store must never prevent startup - the worst case is
    /// that recoverable sessions are not offered, which is the behaviour
    /// Panoptes had before persistence existed.
    pub fn load_with_status() -> (Self, Option<String>) {
        Self::load_from_with_status(&sessions_file_path())
    }

    /// Load store from a specific path, returning a warning message if data was corrupted
    pub fn load_from_with_status(path: &Path) -> (Self, Option<String>) {
        if !path.exists() {
            return (Self::with_path(path.to_path_buf()), None);
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to read sessions file: {}", e);
                if let Some(backup_path) = backup_corrupted_file(path) {
                    let warning = format!(
                        "Sessions file was unreadable. Backup saved to {}. Starting fresh.",
                        backup_path.display()
                    );
                    return (Self::with_path(path.to_path_buf()), Some(warning));
                }
                let warning = format!("Sessions file was unreadable ({}). Starting fresh.", e);
                return (Self::with_path(path.to_path_buf()), Some(warning));
            }
        };

        let data: StoreData = match serde_json::from_str(&content) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("Sessions file is corrupted: {}", e);
                if let Some(backup_path) = backup_corrupted_file(path) {
                    let warning = format!(
                        "Sessions file was corrupted. Backup saved to {}. Starting fresh.",
                        backup_path.display()
                    );
                    return (Self::with_path(path.to_path_buf()), Some(warning));
                }
                let warning = format!("Sessions file was corrupted ({}). Starting fresh.", e);
                return (Self::with_path(path.to_path_buf()), Some(warning));
            }
        };

        let sessions = data.sessions.into_iter().map(|s| (s.id, s)).collect();

        (
            Self {
                sessions,
                store_path: path.to_path_buf(),
            },
            None,
        )
    }

    /// Load store from a specific path
    ///
    /// If the file is corrupted, this will create a backup and return an error.
    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::with_path(path.to_path_buf()));
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to read sessions file: {}", e);
                backup_corrupted_file(path);
                return Err(anyhow::anyhow!(
                    "Could not read sessions file ({}). Starting with no recoverable sessions.",
                    e
                ));
            }
        };

        let data: StoreData = match serde_json::from_str(&content) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("Sessions file is corrupted: {}", e);
                backup_corrupted_file(path);
                return Err(anyhow::anyhow!(
                    "Sessions file is corrupted and could not be parsed ({}). \
                     A backup has been created. \
                     Starting with no recoverable sessions.",
                    e
                ));
            }
        };

        let sessions = data.sessions.into_iter().map(|s| (s.id, s)).collect();

        Ok(Self {
            sessions,
            store_path: path.to_path_buf(),
        })
    }

    /// Save store to disk
    pub fn save(&self) -> Result<()> {
        self.save_to(&self.store_path)
    }

    /// Save store to a specific path
    ///
    /// Writes to a temporary file and renames it into place. Unlike the project
    /// store, this file is written while sessions are running, so a crash
    /// part-way through a write is a realistic scenario - and truncating the
    /// index is exactly the failure this module exists to prevent.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        use crate::config::{categorize_io_error, DiskErrorKind};

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                let kind = categorize_io_error(&e);
                match kind {
                    DiskErrorKind::PermissionDenied => {
                        anyhow::bail!(
                            "Permission denied creating directory {:?}. Check file permissions.",
                            parent
                        );
                    }
                    DiskErrorKind::DiskFull => {
                        anyhow::bail!("Disk full - cannot create directory {:?}", parent);
                    }
                    _ => {
                        return Err(e).context("Failed to create directory for sessions file");
                    }
                }
            }
        }

        let data = StoreData {
            sessions: self.sessions.values().cloned().collect(),
        };

        let content =
            serde_json::to_string_pretty(&data).context("Failed to serialize sessions")?;

        let tmp_path = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp_path, &content) {
            let kind = categorize_io_error(&e);
            match kind {
                DiskErrorKind::DiskFull => {
                    anyhow::bail!(
                        "Disk full - free space needed to save sessions. Recoverable sessions may be lost."
                    );
                }
                DiskErrorKind::PermissionDenied => {
                    anyhow::bail!(
                        "Permission denied writing to {:?}. Check file permissions.",
                        tmp_path
                    );
                }
                _ => {
                    return Err(e).context("Failed to write sessions file");
                }
            }
        }

        std::fs::rename(&tmp_path, path).context("Failed to commit sessions file")?;

        Ok(())
    }
}

/// Get the path to the sessions file
pub fn sessions_file_path() -> PathBuf {
    config_dir().join("sessions.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionType;
    use uuid::Uuid;

    /// Build a store backed by a temp dir, plus the dir guard that owns it.
    ///
    /// Tests must never touch the real `~/.panoptes/sessions.json` - a running
    /// Panoptes instance owns that file.
    fn temp_store() -> (SessionStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("sessions.json");
        (SessionStore::with_path(path), dir)
    }

    fn sample_session(name: &str) -> SessionInfo {
        SessionInfo::new(
            name.to_string(),
            PathBuf::from("/tmp"),
            Uuid::new_v4(),
            Uuid::new_v4(),
        )
    }

    #[test]
    fn test_new_store_is_empty() {
        let (store, _dir) = temp_store();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_upsert_and_get() {
        let (mut store, _dir) = temp_store();
        let info = sample_session("alpha");
        let id = info.id;
        store.upsert(info);

        assert_eq!(store.len(), 1);
        assert_eq!(store.get(id).map(|s| s.name.as_str()), Some("alpha"));
    }

    #[test]
    fn test_upsert_replaces_existing_record() {
        let (mut store, _dir) = temp_store();
        let mut info = sample_session("alpha");
        let id = info.id;
        store.upsert(info.clone());

        info.name = "renamed".to_string();
        store.upsert(info);

        assert_eq!(store.len(), 1);
        assert_eq!(store.get(id).map(|s| s.name.as_str()), Some("renamed"));
    }

    #[test]
    fn test_remove() {
        let (mut store, _dir) = temp_store();
        let info = sample_session("alpha");
        let id = info.id;
        store.upsert(info);

        assert!(store.remove(id).is_some());
        assert!(store.is_empty());
        assert!(store.remove(id).is_none());
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");

        let mut store = SessionStore::with_path(path.clone());
        let mut info = sample_session("resumable");
        info.session_type = SessionType::ClaudeCode;
        info.agent_session_id = Some("11111111-2222-3333-4444-555555555555".to_string());
        let id = info.id;
        store.upsert(info);
        store.save().unwrap();

        let loaded = SessionStore::load_from(&path).unwrap();
        let restored = loaded.get(id).expect("session should survive roundtrip");
        assert_eq!(restored.name, "resumable");
        assert_eq!(
            restored.agent_session_id.as_deref(),
            Some("11111111-2222-3333-4444-555555555555")
        );
    }

    #[test]
    fn test_load_missing_file_is_empty_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");

        let store = SessionStore::load_from(&path).unwrap();
        assert!(store.is_empty());

        let (store, warning) = SessionStore::load_from_with_status(&path);
        assert!(store.is_empty());
        assert!(warning.is_none());
    }

    #[test]
    fn test_load_corrupted_file_backs_up_and_warns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        std::fs::write(&path, "{ this is not json").unwrap();

        let (store, warning) = SessionStore::load_from_with_status(&path);

        assert!(store.is_empty());
        assert!(warning.is_some(), "corruption should surface a warning");
        assert!(!path.exists(), "corrupted file should be renamed away");

        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("corrupt"))
            .collect();
        assert_eq!(backups.len(), 1, "expected exactly one backup file");
    }

    #[test]
    fn test_save_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");

        let mut store = SessionStore::with_path(path.clone());
        store.upsert(sample_session("alpha"));
        store.save().unwrap();

        assert!(path.exists());
        assert!(
            !path.with_extension("json.tmp").exists(),
            "temp file should be renamed, not left behind"
        );
    }

    #[test]
    fn test_save_overwrites_previous_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");

        let mut store = SessionStore::with_path(path.clone());
        let first = sample_session("first");
        let first_id = first.id;
        store.upsert(first);
        store.save().unwrap();

        store.remove(first_id);
        store.upsert(sample_session("second"));
        store.save().unwrap();

        let loaded = SessionStore::load_from(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(loaded.get(first_id).is_none());
    }

    #[test]
    fn test_sessions_sorted_by_last_activity_descending() {
        let (mut store, _dir) = temp_store();

        let mut older = sample_session("older");
        older.last_activity = chrono::Utc::now() - chrono::Duration::hours(2);
        let mut newer = sample_session("newer");
        newer.last_activity = chrono::Utc::now();

        store.upsert(older);
        store.upsert(newer);

        let sorted = store.sessions_sorted();
        assert_eq!(sorted[0].name, "newer");
        assert_eq!(sorted[1].name, "older");
    }
}
