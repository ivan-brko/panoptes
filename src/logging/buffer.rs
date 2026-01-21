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

    /// Check if this level is a warning or error (for alerts)
    pub fn is_alert(&self) -> bool {
        matches!(self, LogLevel::Warn | LogLevel::Error)
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
    /// Warnings and errors only (for future alert display)
    alerts: RwLock<VecDeque<LogEntry>>,
    /// Maximum entries to keep in the main buffer
    max_entries: usize,
    /// Maximum alerts to keep
    max_alerts: usize,
}

impl LogBuffer {
    /// Create a new log buffer with specified capacities
    pub fn new(max_entries: usize, max_alerts: usize) -> Self {
        Self {
            entries: RwLock::new(VecDeque::with_capacity(max_entries)),
            alerts: RwLock::new(VecDeque::with_capacity(max_alerts)),
            max_entries,
            max_alerts,
        }
    }

    /// Push a new log entry to the buffer
    pub fn push(&self, entry: LogEntry) {
        // Add to alerts buffer if warning or error
        if entry.level.is_alert() {
            if let Ok(mut alerts) = self.alerts.write() {
                if alerts.len() >= self.max_alerts {
                    alerts.pop_front();
                }
                alerts.push_back(entry.clone());
            }
        }

        // Add to main entries buffer
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

    /// Get pending alerts (warnings and errors)
    pub fn pending_alerts(&self) -> Vec<LogEntry> {
        self.alerts
            .read()
            .map(|a| a.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get the number of alerts
    pub fn alert_count(&self) -> usize {
        self.alerts.read().map(|a| a.len()).unwrap_or(0)
    }

    /// Clear all alerts (after they've been acknowledged)
    pub fn clear_alerts(&self) {
        if let Ok(mut alerts) = self.alerts.write() {
            alerts.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_buffer_push_and_retrieve() {
        let buffer = LogBuffer::new(100, 10);

        buffer.push(LogEntry::new(LogLevel::Info, "test", "message 1"));
        buffer.push(LogEntry::new(LogLevel::Warn, "test", "warning 1"));
        buffer.push(LogEntry::new(LogLevel::Error, "test", "error 1"));

        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.alert_count(), 2);

        let entries = buffer.all_entries();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].message, "message 1");
        assert_eq!(entries[1].message, "warning 1");
        assert_eq!(entries[2].message, "error 1");

        let alerts = buffer.pending_alerts();
        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[0].message, "warning 1");
        assert_eq!(alerts[1].message, "error 1");
    }

    #[test]
    fn test_log_buffer_capacity() {
        let buffer = LogBuffer::new(3, 2);

        for i in 0..5 {
            buffer.push(LogEntry::new(LogLevel::Info, "test", format!("msg {}", i)));
        }

        assert_eq!(buffer.len(), 3);
        let entries = buffer.all_entries();
        assert_eq!(entries[0].message, "msg 2");
        assert_eq!(entries[1].message, "msg 3");
        assert_eq!(entries[2].message, "msg 4");
    }

    #[test]
    fn test_log_level_is_alert() {
        assert!(!LogLevel::Trace.is_alert());
        assert!(!LogLevel::Debug.is_alert());
        assert!(!LogLevel::Info.is_alert());
        assert!(LogLevel::Warn.is_alert());
        assert!(LogLevel::Error.is_alert());
    }
}
