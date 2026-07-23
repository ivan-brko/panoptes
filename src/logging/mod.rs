//! Logging system for Panoptes
//!
//! Provides file-based logging with retention. There is no in-memory log
//! buffer: logs are read from `~/.panoptes/logs/` with whatever the user
//! already uses for tailing files, and the Settings pane says where they are.

mod file_writer;
mod retention;

pub use file_writer::{init_file_logging, LogFileInfo};
pub use retention::cleanup_old_logs;
