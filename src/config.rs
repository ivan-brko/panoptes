//! Configuration management for Panoptes

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

impl DiskErrorKind {
    /// Get a user-friendly message for this error kind
    pub fn user_message(&self) -> &'static str {
        match self {
            DiskErrorKind::DiskFull => "Disk full - free space needed to save",
            DiskErrorKind::PermissionDenied => "Permission denied writing to ~/.panoptes/",
            DiskErrorKind::NotFound => "File or directory not found",
            DiskErrorKind::Other => "Failed to save data",
        }
    }
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

/// Create a user-friendly error message from an IO error
pub fn friendly_io_error_message(e: &std::io::Error, context: &str) -> String {
    let kind = categorize_io_error(e);
    match kind {
        DiskErrorKind::DiskFull => format!("{}: {}", context, kind.user_message()),
        DiskErrorKind::PermissionDenied => format!("{}: {}", context, kind.user_message()),
        DiskErrorKind::NotFound => format!("{}: file or directory not found", context),
        DiskErrorKind::Other => format!("{}: {}", context, e),
    }
}

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Port for the hook HTTP server
    pub hook_port: u16,

    /// Directory for git worktrees
    pub worktrees_dir: PathBuf,

    /// Directory for hook scripts
    pub hooks_dir: PathBuf,

    /// Maximum lines to keep in output buffer per session
    pub max_output_lines: usize,

    /// Idle threshold in seconds before session is flagged as needing attention (default: 300 = 5 min)
    #[serde(default = "default_idle_threshold")]
    pub idle_threshold_secs: u64,

    /// State timeout in seconds - Executing states auto-transition to Idle after this (default: 300 = 5 min)
    #[serde(default = "default_state_timeout")]
    pub state_timeout_secs: u64,

    /// Exited session retention in seconds - sessions are cleaned up after this duration (default: 300 = 5 min)
    #[serde(default = "default_exited_retention")]
    pub exited_retention_secs: u64,

    /// Theme preset: "dark" (default), "light", or "high-contrast"
    #[serde(default = "default_theme_preset")]
    pub theme_preset: String,

    /// Notification method: "bell" (terminal bell), "title" (update terminal title), "none"
    #[serde(default = "default_notification_method")]
    pub notification_method: String,

    /// Esc hold threshold in milliseconds for exiting session mode (default: 400ms)
    #[serde(default = "default_esc_hold_threshold_ms")]
    pub esc_hold_threshold_ms: u64,

    /// Default focus timer duration in minutes (default: 25)
    #[serde(default = "default_focus_timer_minutes")]
    pub focus_timer_minutes: u64,

    /// Focus stats retention in days (default: 30)
    #[serde(default = "default_focus_stats_retention_days")]
    pub focus_stats_retention_days: u64,

    /// Maximum scrollback lines per session (default: 10000)
    /// Each 1000 lines uses approximately 10KB of memory
    #[serde(default = "default_scrollback_lines")]
    pub scrollback_lines: usize,
}

fn default_idle_threshold() -> u64 {
    300
}

fn default_state_timeout() -> u64 {
    300 // 5 minutes
}

fn default_exited_retention() -> u64 {
    300 // 5 minutes
}

fn default_theme_preset() -> String {
    "dark".to_string()
}

fn default_notification_method() -> String {
    "bell".to_string()
}

fn default_esc_hold_threshold_ms() -> u64 {
    400
}

fn default_focus_timer_minutes() -> u64 {
    25 // Pomodoro-style default
}

fn default_focus_stats_retention_days() -> u64 {
    30
}

fn default_scrollback_lines() -> usize {
    10_000
}

impl Default for Config {
    fn default() -> Self {
        let base = config_dir();
        Self {
            hook_port: 9999,
            worktrees_dir: base.join("worktrees"),
            hooks_dir: base.join("hooks"),
            max_output_lines: 10_000,
            idle_threshold_secs: default_idle_threshold(),
            state_timeout_secs: default_state_timeout(),
            exited_retention_secs: default_exited_retention(),
            theme_preset: default_theme_preset(),
            notification_method: default_notification_method(),
            esc_hold_threshold_ms: default_esc_hold_threshold_ms(),
            focus_timer_minutes: default_focus_timer_minutes(),
            focus_stats_retention_days: default_focus_stats_retention_days(),
            scrollback_lines: default_scrollback_lines(),
        }
    }
}

impl Config {
    /// Load configuration from file, or return default if not found
    pub fn load() -> Result<Self> {
        let path = config_file_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path).context("Failed to read config file")?;
            toml::from_str(&content).context("Failed to parse config file")
        } else {
            Ok(Self::default())
        }
    }

    /// Save configuration to file
    pub fn save(&self) -> Result<()> {
        let path = config_file_path();
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;
        std::fs::write(&path, content).context("Failed to write config file")?;
        Ok(())
    }
}

/// Get the base configuration directory (~/.panoptes)
/// Falls back to ./.panoptes if home directory cannot be determined
pub fn config_dir() -> PathBuf {
    try_config_dir().unwrap_or_else(|| {
        tracing::warn!("Could not determine home directory, using current directory for config");
        PathBuf::from(".panoptes")
    })
}

/// Try to get the base configuration directory, returning None if home dir is unavailable
pub fn try_config_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".panoptes"))
}

/// Get the path to the config file
pub fn config_file_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Get the path to the logs directory
pub fn logs_dir() -> PathBuf {
    config_dir().join("logs")
}

/// Ensure all required directories exist
pub fn ensure_directories() -> Result<()> {
    let config = Config::default();

    std::fs::create_dir_all(config_dir()).context("Failed to create config directory")?;

    std::fs::create_dir_all(&config.worktrees_dir)
        .context("Failed to create worktrees directory")?;

    std::fs::create_dir_all(&config.hooks_dir).context("Failed to create hooks directory")?;

    std::fs::create_dir_all(logs_dir()).context("Failed to create logs directory")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.hook_port, 9999);
        assert_eq!(config.max_output_lines, 10_000);
    }

    #[test]
    fn test_config_serialization() {
        let config = Config::default();
        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(config.hook_port, parsed.hook_port);
    }

    #[test]
    fn test_config_dir_does_not_panic() {
        // This test verifies that config_dir() does not panic
        // even if it falls back to a local directory
        let dir = config_dir();
        assert!(dir.ends_with(".panoptes"));
    }

    #[test]
    fn test_try_config_dir() {
        // try_config_dir should return Some on most systems with a home dir
        // but the important thing is it doesn't panic
        let result = try_config_dir();
        // We can't assert it's Some because CI might not have a home dir
        // But if it is Some, it should end with .panoptes
        if let Some(path) = result {
            assert!(path.ends_with(".panoptes"));
        }
    }

    // Config scrollback_lines tests
    #[test]
    fn test_default_config_scrollback() {
        let config = Config::default();
        assert_eq!(config.scrollback_lines, 10_000);
    }

    #[test]
    fn test_config_serialization_with_scrollback() {
        let mut config = Config::default();
        config.scrollback_lines = 5000;

        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.scrollback_lines, 5000);
    }
}
