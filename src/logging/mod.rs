//! Logging system for Panoptes
//!
//! Provides file-based logging with retention, real-time log buffering for TUI display,
//! and utilities for log management.

mod buffer;
mod file_writer;
mod retention;

pub use buffer::{LogBuffer, LogEntry, LogLevel};
pub use file_writer::{init_file_logging, LogFileInfo};
pub use retention::cleanup_old_logs;
