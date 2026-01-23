//! File-based logging with tracing integration
//!
//! Sets up file logging with timestamped filenames and integrates with the LogBuffer
//! for real-time TUI display.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{Local, Utc};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use super::buffer::{LogBuffer, LogEntry, LogLevel};

/// Number of consecutive write failures before emitting a warning
const FAILURE_THRESHOLD: usize = 5;

/// Global counter for consecutive write failures
/// We use a static counter because the DualWriter is recreated on each write operation
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

/// A writer that writes to both a file and the LogBuffer
struct DualWriter {
    file: Arc<std::sync::Mutex<File>>,
    buffer: Arc<LogBuffer>,
}

impl Write for DualWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Write to file with failure tracking
        let mut write_failed = false;
        if let Ok(mut file) = self.file.lock() {
            let write_result = file.write_all(buf).and_then(|_| file.flush());
            match write_result {
                Ok(()) => {
                    // Reset failure counter on success
                    FAILURE_COUNT.store(0, Ordering::Relaxed);
                    WARNING_EMITTED.store(false, Ordering::Relaxed);
                }
                Err(e) => {
                    write_failed = true;
                    let count = FAILURE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                    // Emit warning to stderr after threshold (only once until recovery)
                    if count == FAILURE_THRESHOLD && !WARNING_EMITTED.swap(true, Ordering::Relaxed)
                    {
                        eprintln!(
                            "WARNING: Log file writes failing repeatedly ({} failures): {}",
                            count, e
                        );
                        eprintln!("         Log entries are being stored in memory but may not be persisted.");
                    }
                }
            }
        } else {
            write_failed = true;
            let count = FAILURE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            if count == FAILURE_THRESHOLD && !WARNING_EMITTED.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "WARNING: Log file mutex poisoned after {} failures. Logs not being written to file.",
                    count
                );
            }
        }

        // Parse the log line and add to buffer (always, even if file write fails)
        // This ensures logs are still visible in the TUI even if file writes fail
        if let Ok(line) = std::str::from_utf8(buf) {
            let line = line.trim();
            if !line.is_empty() {
                // Parse format: "2026-01-21T14:30:45.123456Z LEVEL target: message"
                if let Some(entry) = parse_log_line(line) {
                    self.buffer.push(entry);
                }
            }
        }

        // Return success to avoid breaking the logging pipeline
        // The log entries are stored in the buffer even if file write fails
        if write_failed {
            // Return Ok anyway to not break tracing - logs are still in buffer
            Ok(buf.len())
        } else {
            Ok(buf.len())
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Ok(mut file) = self.file.lock() {
            file.flush()
        } else {
            Ok(())
        }
    }
}

/// Parse a log line into a LogEntry
fn parse_log_line(line: &str) -> Option<LogEntry> {
    // Format: "2026-01-21T14:30:45.123456Z LEVEL message"
    // The tracing_subscriber fmt layer produces this format

    // Skip empty lines
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Try to find the level indicator
    let level = if line.contains(" TRACE ") {
        LogLevel::Trace
    } else if line.contains(" DEBUG ") {
        LogLevel::Debug
    } else if line.contains(" INFO ") {
        LogLevel::Info
    } else if line.contains(" WARN ") {
        LogLevel::Warn
    } else if line.contains(" ERROR ") {
        LogLevel::Error
    } else {
        // If we can't parse, treat as info
        LogLevel::Info
    };

    // Extract message (everything after the level)
    let level_str = format!(" {} ", level.as_str());
    let message = if let Some(pos) = line.find(&level_str) {
        line[pos + level_str.len()..].trim().to_string()
    } else {
        line.to_string()
    };

    // Extract target from message if present (format: "target: message")
    let (target, final_message) = if let Some(colon_pos) = message.find(": ") {
        let potential_target = &message[..colon_pos];
        // Check if it looks like a module path (contains :: or is a simple word)
        if potential_target.contains("::") || !potential_target.contains(' ') {
            (
                potential_target.to_string(),
                message[colon_pos + 2..].to_string(),
            )
        } else {
            ("panoptes".to_string(), message)
        }
    } else {
        ("panoptes".to_string(), message)
    };

    Some(LogEntry {
        timestamp: Utc::now(),
        level,
        target,
        message: final_message,
    })
}

/// Writer factory for tracing-subscriber
struct DualWriterMaker {
    file: Arc<std::sync::Mutex<File>>,
    buffer: Arc<LogBuffer>,
}

impl<'a> MakeWriter<'a> for DualWriterMaker {
    type Writer = DualWriter;

    fn make_writer(&'a self) -> Self::Writer {
        DualWriter {
            file: Arc::clone(&self.file),
            buffer: Arc::clone(&self.buffer),
        }
    }
}

/// Guard that keeps the logging system alive
pub struct LoggingGuard {
    _file: Arc<std::sync::Mutex<File>>,
}

/// Initialize file logging with buffer integration
///
/// Returns the log file info and a guard that must be kept alive for the duration of logging.
pub fn init_file_logging(
    logs_dir: PathBuf,
    buffer: Arc<LogBuffer>,
) -> Result<(LogFileInfo, LoggingGuard)> {
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
    let writer = DualWriterMaker {
        file: Arc::clone(&file),
        buffer,
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
    fn test_parse_log_line_info() {
        let line = "2026-01-21T14:30:45.123456Z  INFO panoptes: Starting application";
        let entry = parse_log_line(line).unwrap();
        assert_eq!(entry.level, LogLevel::Info);
        assert_eq!(entry.target, "panoptes");
        assert_eq!(entry.message, "Starting application");
    }

    #[test]
    fn test_parse_log_line_warn() {
        let line = "2026-01-21T14:30:45.123456Z  WARN panoptes::config: Config not found";
        let entry = parse_log_line(line).unwrap();
        assert_eq!(entry.level, LogLevel::Warn);
        assert_eq!(entry.target, "panoptes::config");
        assert_eq!(entry.message, "Config not found");
    }

    #[test]
    fn test_parse_log_line_error() {
        let line = "2026-01-21T14:30:45.123456Z ERROR panoptes::session: Failed to start";
        let entry = parse_log_line(line).unwrap();
        assert_eq!(entry.level, LogLevel::Error);
    }

    #[test]
    fn test_create_log_file_path() {
        let logs_dir = PathBuf::from("/tmp/panoptes/logs");
        let path = create_log_file_path(&logs_dir);
        assert!(path.to_string_lossy().contains("panoptes-"));
        assert!(path.to_string_lossy().ends_with(".log"));
    }
}
