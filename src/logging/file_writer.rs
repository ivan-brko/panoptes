//! File-based logging with tracing integration
//!
//! Sets up file logging with timestamped filenames and integrates with the LogBuffer
//! for real-time TUI display.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{Local, Utc};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use super::buffer::{LogBuffer, LogEntry, LogLevel};

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
        // Write to file
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(buf);
            let _ = file.flush();
        }

        // Parse the log line and add to buffer
        if let Ok(line) = std::str::from_utf8(buf) {
            let line = line.trim();
            if !line.is_empty() {
                // Parse format: "2026-01-21T14:30:45.123456Z LEVEL target: message"
                if let Some(entry) = parse_log_line(line) {
                    self.buffer.push(entry);
                }
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
