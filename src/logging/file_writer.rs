//! File-based logging with tracing integration
//!
//! Sets up file logging with timestamped filenames. Nothing is buffered in
//! memory: the Settings pane points at the file, and reading it is `tail`'s
//! job rather than the TUI's.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Local;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Number of consecutive write failures before emitting a warning
const FAILURE_THRESHOLD: usize = 5;

/// Global counter for consecutive write failures
/// We use a static counter because the writer is recreated on each write operation
static FAILURE_COUNT: AtomicUsize = AtomicUsize::new(0);
/// Track if we've already emitted a warning (to avoid spamming)
static WARNING_EMITTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Information about the current log file
#[derive(Debug, Clone)]
pub struct LogFileInfo {
    /// Full path to the log file
    pub path: PathBuf,
}

/// Generate a timestamped log file path
pub fn create_log_file_path(logs_dir: &Path) -> PathBuf {
    let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
    logs_dir.join(format!("panoptes-{}.log", timestamp))
}

/// A writer that appends to the shared log file
///
/// Write failures are counted rather than propagated: a full disk must not
/// take the application down, and `Ok` keeps the tracing pipeline alive.
struct FileWriter {
    file: Arc<std::sync::Mutex<File>>,
}

impl Write for FileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut file) = self.file.lock() {
            match file.write_all(buf).and_then(|_| file.flush()) {
                Ok(()) => {
                    // Reset failure counter on success
                    FAILURE_COUNT.store(0, Ordering::Relaxed);
                    WARNING_EMITTED.store(false, Ordering::Relaxed);
                }
                Err(e) => {
                    let count = FAILURE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                    // Emit warning to stderr after threshold (only once until recovery)
                    if count == FAILURE_THRESHOLD && !WARNING_EMITTED.swap(true, Ordering::Relaxed)
                    {
                        eprintln!(
                            "WARNING: Log file writes failing repeatedly ({} failures): {}",
                            count, e
                        );
                    }
                }
            }
        } else {
            let count = FAILURE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            if count == FAILURE_THRESHOLD && !WARNING_EMITTED.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "WARNING: Log file mutex poisoned after {} failures. Logs not being written to file.",
                    count
                );
            }
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Ok(mut file) = self.file.lock() {
            file.flush()
        } else {
            Ok(())
        }
    }
}

/// Writer factory for tracing-subscriber
struct FileWriterMaker {
    file: Arc<std::sync::Mutex<File>>,
}

impl<'a> MakeWriter<'a> for FileWriterMaker {
    type Writer = FileWriter;

    fn make_writer(&'a self) -> Self::Writer {
        FileWriter {
            file: Arc::clone(&self.file),
        }
    }
}

/// Guard that keeps the logging system alive
pub struct LoggingGuard {
    _file: Arc<std::sync::Mutex<File>>,
}

/// Initialize file logging
///
/// Returns the log file info and a guard that must be kept alive for the duration of logging.
pub fn init_file_logging(logs_dir: PathBuf) -> Result<(LogFileInfo, LoggingGuard)> {
    // Ensure logs directory exists
    fs::create_dir_all(&logs_dir).context("Failed to create logs directory")?;

    // Create log file path
    let log_path = create_log_file_path(&logs_dir);

    // Open log file
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .context("Failed to open log file")?;

    let file = Arc::new(std::sync::Mutex::new(file));

    // Create writer maker
    let writer = FileWriterMaker {
        file: Arc::clone(&file),
    };

    // Create file logging layer
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true);

    // Create env filter
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "panoptes=info".into());

    // Initialize subscriber
    tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .init();

    let info = LogFileInfo {
        path: log_path.clone(),
    };

    let guard = LoggingGuard { _file: file };

    Ok((info, guard))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_log_file_path() {
        let logs_dir = PathBuf::from("/tmp/panoptes/logs");
        let path = create_log_file_path(&logs_dir);
        assert!(path.to_string_lossy().contains("panoptes-"));
        assert!(path.to_string_lossy().ends_with(".log"));
    }
}
