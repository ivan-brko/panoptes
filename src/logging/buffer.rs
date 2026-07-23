//! In-memory log buffer for real-time viewing
//!
//! Provides a thread-safe ring buffer that stores log entries for display in the TUI.

use std::collections::VecDeque;
use std::sync::RwLock;

use chrono::{DateTime, Utc};

/// Log level for display purposes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// Get the display name for this level
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }
}

impl From<tracing::Level> for LogLevel {
    fn from(level: tracing::Level) -> Self {
        match level {
            tracing::Level::TRACE => LogLevel::Trace,
            tracing::Level::DEBUG => LogLevel::Debug,
            tracing::Level::INFO => LogLevel::Info,
            tracing::Level::WARN => LogLevel::Warn,
            tracing::Level::ERROR => LogLevel::Error,
        }
    }
}

/// A single log entry
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Timestamp when the log was recorded
    pub timestamp: DateTime<Utc>,
    /// Log level
    pub level: LogLevel,
    /// Target/module that produced the log
    pub target: String,
    /// Log message
    pub message: String,
}

impl LogEntry {
    /// Create a new log entry
    pub fn new(level: LogLevel, target: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            timestamp: Utc::now(),
            level,
            target: target.into(),
            message: message.into(),
        }
    }
}

/// Thread-safe ring buffer for storing log entries
pub struct LogBuffer {
    /// All log entries (capped at max_entries)
    entries: RwLock<VecDeque<LogEntry>>,
    /// Maximum entries to keep in the main buffer
    max_entries: usize,
}

impl LogBuffer {
    /// Create a new log buffer with the specified capacity
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: RwLock::new(VecDeque::with_capacity(max_entries)),
            max_entries,
        }
    }

    /// Push a new log entry to the buffer
    pub fn push(&self, entry: LogEntry) {
        if let Ok(mut entries) = self.entries.write() {
            if entries.len() >= self.max_entries {
                entries.pop_front();
            }
            entries.push_back(entry);
        }
    }

    /// Get all entries as a vector (for rendering)
    pub fn all_entries(&self) -> Vec<LogEntry> {
        self.entries
            .read()
            .map(|e| e.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get the number of entries in the buffer
    pub fn len(&self) -> usize {
        self.entries.read().map(|e| e.len()).unwrap_or(0)
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_buffer_push_and_retrieve() {
        let buffer = LogBuffer::new(100);

        buffer.push(LogEntry::new(LogLevel::Info, "test", "message 1"));
        buffer.push(LogEntry::new(LogLevel::Warn, "test", "warning 1"));
        buffer.push(LogEntry::new(LogLevel::Error, "test", "error 1"));

        assert_eq!(buffer.len(), 3);

        let entries = buffer.all_entries();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].message, "message 1");
        assert_eq!(entries[1].message, "warning 1");
        assert_eq!(entries[2].message, "error 1");
    }

    #[test]
    fn test_log_buffer_capacity() {
        let buffer = LogBuffer::new(3);

        for i in 0..5 {
            buffer.push(LogEntry::new(LogLevel::Info, "test", format!("msg {}", i)));
        }

        assert_eq!(buffer.len(), 3);
        let entries = buffer.all_entries();
        assert_eq!(entries[0].message, "msg 2");
        assert_eq!(entries[1].message, "msg 3");
        assert_eq!(entries[2].message, "msg 4");
    }
}
