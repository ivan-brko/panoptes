//! Configuration management for Panoptes

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

/// Ensure all required directories exist
pub fn ensure_directories() -> Result<()> {
    let config = Config::default();

    std::fs::create_dir_all(config_dir()).context("Failed to create config directory")?;

    std::fs::create_dir_all(&config.worktrees_dir)
        .context("Failed to create worktrees directory")?;

    std::fs::create_dir_all(&config.hooks_dir).context("Failed to create hooks directory")?;

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
}
