//! Log file retention management
//!
//! Handles cleanup of old log files based on age.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use anyhow::Result;

/// Default retention period in days
pub const DEFAULT_RETENTION_DAYS: u64 = 7;

/// Clean up log files older than the retention period
///
/// Returns the number of files deleted.
pub fn cleanup_old_logs(logs_dir: &Path) -> Result<usize> {
    cleanup_old_logs_with_retention(logs_dir, DEFAULT_RETENTION_DAYS)
}

/// Clean up log files older than the specified number of days
///
/// Returns the number of files deleted.
pub fn cleanup_old_logs_with_retention(logs_dir: &Path, retention_days: u64) -> Result<usize> {
    if !logs_dir.exists() {
        return Ok(0);
    }

    let retention_duration = Duration::from_secs(retention_days * 24 * 60 * 60);
    let cutoff = SystemTime::now()
        .checked_sub(retention_duration)
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let mut deleted_count = 0;

    for entry in fs::read_dir(logs_dir)? {
        let entry = entry?;
        let path = entry.path();

        // Only process panoptes log files
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if !name.starts_with("panoptes-") || !name.ends_with(".log") {
                continue;
            }
        } else {
            continue;
        }

        // Check file modification time
        if let Ok(metadata) = entry.metadata() {
            if let Ok(modified) = metadata.modified() {
                if modified < cutoff && fs::remove_file(&path).is_ok() {
                    deleted_count += 1;
                }
            }
        }
    }

    Ok(deleted_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_cleanup_empty_dir() {
        let temp_dir = TempDir::new().unwrap();
        let count = cleanup_old_logs(temp_dir.path()).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_cleanup_nonexistent_dir() {
        let path = Path::new("/nonexistent/path/for/testing");
        let count = cleanup_old_logs(path).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_cleanup_ignores_non_log_files() {
        let temp_dir = TempDir::new().unwrap();

        // Create a non-log file
        let other_file = temp_dir.path().join("other.txt");
        File::create(&other_file)
            .unwrap()
            .write_all(b"test")
            .unwrap();

        // Create a log file with wrong prefix
        let wrong_prefix = temp_dir.path().join("other-2026-01-01_00-00-00.log");
        File::create(&wrong_prefix)
            .unwrap()
            .write_all(b"test")
            .unwrap();

        let count = cleanup_old_logs(temp_dir.path()).unwrap();
        assert_eq!(count, 0);

        // Files should still exist
        assert!(other_file.exists());
        assert!(wrong_prefix.exists());
    }

    #[test]
    fn test_cleanup_keeps_recent_files() {
        let temp_dir = TempDir::new().unwrap();

        // Create a recent log file
        let log_file = temp_dir.path().join("panoptes-2026-01-21_14-30-45.log");
        File::create(&log_file)
            .unwrap()
            .write_all(b"test log content")
            .unwrap();

        let count = cleanup_old_logs(temp_dir.path()).unwrap();
        assert_eq!(count, 0);

        // File should still exist
        assert!(log_file.exists());
    }
}
