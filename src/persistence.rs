//! Shared persistence helpers
//!
//! Every on-disk store in Panoptes (projects, sessions, agent profiles, the
//! TOML config) follows the same lifecycle: load with a graceful fallback when
//! the file is corrupted (backing the bad file up first), and save atomically
//! via a sibling temp file so a crash mid-write can never truncate the store.
//! This module holds that mechanism once so the stores stay thin.

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Categories of disk errors for user-friendly messages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskErrorKind {
    /// Disk is full or quota exceeded
    DiskFull,
    /// Permission denied (read or write)
    PermissionDenied,
    /// File or directory not found
    NotFound,
    /// Other IO error
    Other,
}

/// Categorize an IO error into a user-friendly category
pub fn categorize_io_error(e: &std::io::Error) -> DiskErrorKind {
    use std::io::ErrorKind;

    match e.kind() {
        // Disk full errors
        ErrorKind::StorageFull => DiskErrorKind::DiskFull,
        // On some systems, disk full might appear as WriteZero or Other
        ErrorKind::WriteZero => DiskErrorKind::DiskFull,

        // Permission errors
        ErrorKind::PermissionDenied => DiskErrorKind::PermissionDenied,

        // Not found
        ErrorKind::NotFound => DiskErrorKind::NotFound,

        // Check raw OS error for disk full on Unix
        _ => {
            #[cfg(unix)]
            {
                if let Some(os_error) = e.raw_os_error() {
                    // ENOSPC (No space left on device) = 28 on Linux, 28 on macOS
                    // EDQUOT (Disk quota exceeded) = 122 on Linux, 69 on macOS
                    if os_error == 28 || os_error == 122 || os_error == 69 {
                        return DiskErrorKind::DiskFull;
                    }
                    // EACCES = 13 on both
                    if os_error == 13 {
                        return DiskErrorKind::PermissionDenied;
                    }
                }
            }
            DiskErrorKind::Other
        }
    }
}

/// Result of loading a store file from disk
#[derive(Debug)]
pub enum LoadOutcome<T> {
    /// The file does not exist; start fresh with defaults
    Absent,
    /// The file was read and parsed successfully
    Loaded(T),
    /// The file was unreadable or unparseable; it has been backed up (when
    /// possible) and the caller should fall back to defaults, surfacing the
    /// warning to the user
    Corrupted { fallback_warning: String },
}

/// Backup a corrupted file by renaming it with a timestamped extension
///
/// Extension-agnostic: `foo.json` becomes `foo.json.corrupt.<ts>` and
/// `config.toml` becomes `config.toml.corrupt.<ts>`. The timestamp keeps a
/// later corruption from overwriting the evidence of an earlier one.
///
/// Returns the backup path on success.
pub fn backup_corrupted_file(path: &Path) -> Option<PathBuf> {
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let suffix = match path.extension() {
        Some(ext) => format!("{}.corrupt.{}", ext.to_string_lossy(), timestamp),
        None => format!("corrupt.{}", timestamp),
    };
    let backup_path = path.with_extension(suffix);
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
            "Corrupted file {} backed up to {}",
            path.display(),
            backup_path.display()
        );
        Some(backup_path)
    }
}

/// Read a file for a load-with-fallback flow, backing it up when unreadable
///
/// Shared front half of [`load_json`] and the TOML config load: existence
/// check, read, and backup-plus-warning on failure. `what` names the store in
/// user-facing messages (e.g. "projects", "sessions").
pub fn load_text(path: &Path, what: &str) -> LoadOutcome<String> {
    if !path.exists() {
        return LoadOutcome::Absent;
    }

    match std::fs::read_to_string(path) {
        Ok(content) => LoadOutcome::Loaded(content),
        Err(e) => {
            tracing::error!("Failed to read {} file {}: {}", what, path.display(), e);
            let fallback_warning = match backup_corrupted_file(path) {
                Some(backup_path) => format!(
                    "The {} file was unreadable. Backup saved to {}. Starting fresh.",
                    what,
                    backup_path.display()
                ),
                None => format!("The {} file was unreadable ({}). Starting fresh.", what, e),
            };
            LoadOutcome::Corrupted { fallback_warning }
        }
    }
}

/// Load a JSON file, backing it up and falling back when corrupted
///
/// On parse failure the file is renamed to a timestamped `.corrupt.<ts>`
/// backup and a human-readable warning naming the backup path is returned for
/// the caller to surface. A missing file is [`LoadOutcome::Absent`], never an
/// error.
pub fn load_json<T: DeserializeOwned>(path: &Path, what: &str) -> LoadOutcome<T> {
    let content = match load_text(path, what) {
        LoadOutcome::Absent => return LoadOutcome::Absent,
        LoadOutcome::Loaded(content) => content,
        LoadOutcome::Corrupted { fallback_warning } => {
            return LoadOutcome::Corrupted { fallback_warning }
        }
    };

    match serde_json::from_str(&content) {
        Ok(value) => LoadOutcome::Loaded(value),
        Err(e) => {
            tracing::error!("The {} file {} is corrupted: {}", what, path.display(), e);
            let fallback_warning = match backup_corrupted_file(path) {
                Some(backup_path) => format!(
                    "The {} file was corrupted. Backup saved to {}. Starting fresh.",
                    what,
                    backup_path.display()
                ),
                None => format!("The {} file was corrupted ({}). Starting fresh.", what, e),
            };
            LoadOutcome::Corrupted { fallback_warning }
        }
    }
}

/// Serialize a value to pretty JSON and write it atomically
///
/// See [`save_text_atomic`] for the write discipline.
pub fn save_json_atomic<T: Serialize>(path: &Path, value: &T, what: &str) -> Result<()> {
    let content = serde_json::to_string_pretty(value)
        .with_context(|| format!("Failed to serialize {}", what))?;
    save_text_atomic(path, &content, what)
}

/// Write a file atomically: parent dirs, sibling temp file, rename
///
/// Stores are written while sessions are running, so a crash part-way through
/// a write is a realistic scenario - and truncating a store is exactly the
/// failure the persistence layer exists to prevent. The rename makes the
/// switch from old content to new atomic on the same filesystem.
///
/// Disk errors are classified once here so every store reports full disks and
/// permission problems in the same user-facing words.
pub fn save_text_atomic(path: &Path, content: &str, what: &str) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            match categorize_io_error(&e) {
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
                    return Err(e)
                        .with_context(|| format!("Failed to create directory for {} file", what));
                }
            }
        }
    }

    let suffix = match path.extension() {
        Some(ext) => format!("{}.tmp", ext.to_string_lossy()),
        None => "tmp".to_string(),
    };
    let tmp_path = path.with_extension(suffix);
    if let Err(e) = std::fs::write(&tmp_path, content) {
        match categorize_io_error(&e) {
            DiskErrorKind::DiskFull => {
                anyhow::bail!(
                    "Disk full - free space needed to save {}. Your changes may not be saved.",
                    what
                );
            }
            DiskErrorKind::PermissionDenied => {
                anyhow::bail!(
                    "Permission denied writing to {:?}. Check file permissions.",
                    tmp_path
                );
            }
            _ => {
                return Err(e).with_context(|| format!("Failed to write {} file", what));
            }
        }
    }

    std::fs::rename(&tmp_path, path).with_context(|| format!("Failed to commit {} file", what))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Sample {
        name: String,
        value: u32,
    }

    fn sample() -> Sample {
        Sample {
            name: "alpha".to_string(),
            value: 42,
        }
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.json");

        save_json_atomic(&path, &sample(), "sample").unwrap();

        match load_json::<Sample>(&path, "sample") {
            LoadOutcome::Loaded(loaded) => assert_eq!(loaded, sample()),
            other => panic!("expected Loaded, got {:?}", other),
        }
    }

    #[test]
    fn test_load_absent_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.json");

        assert!(matches!(
            load_json::<Sample>(&path, "sample"),
            LoadOutcome::Absent
        ));
    }

    #[test]
    fn test_load_corrupted_file_backs_up_and_warns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.json");
        std::fs::write(&path, "{ this is not json").unwrap();

        let outcome = load_json::<Sample>(&path, "sample");
        let warning = match outcome {
            LoadOutcome::Corrupted { fallback_warning } => fallback_warning,
            other => panic!("expected Corrupted, got {:?}", other),
        };

        assert!(!path.exists(), "corrupted file should be renamed away");

        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("json.corrupt."))
            .collect();
        assert_eq!(backups.len(), 1, "expected exactly one timestamped backup");
        assert!(
            warning.contains(&backups[0].path().display().to_string()),
            "warning should name the backup path: {}",
            warning
        );
    }

    #[test]
    fn test_save_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.json");

        save_json_atomic(&path, &sample(), "sample").unwrap();

        assert!(path.exists());
        assert!(
            !path.with_extension("json.tmp").exists(),
            "temp file should be renamed, not left behind"
        );
    }

    #[test]
    fn test_save_creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deeper").join("sample.json");

        save_json_atomic(&path, &sample(), "sample").unwrap();

        assert!(matches!(
            load_json::<Sample>(&path, "sample"),
            LoadOutcome::Loaded(_)
        ));
    }

    #[test]
    fn test_backup_is_extension_agnostic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not valid").unwrap();

        let backup = backup_corrupted_file(&path).expect("backup should succeed");

        assert!(!path.exists());
        assert!(backup
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("config.toml.corrupt."));
    }

    #[test]
    fn test_save_text_atomic_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        save_text_atomic(&path, "hook_port = 9999\n", "config").unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "hook_port = 9999\n"
        );
        assert!(
            !path.with_extension("toml.tmp").exists(),
            "temp file should be renamed, not left behind"
        );
    }
}
