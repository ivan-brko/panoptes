//! Claude configuration management module
//!
//! This module handles multiple Claude Code configurations (accounts) that can be
//! used with different projects. Each configuration points to a different
//! CLAUDE_CONFIG_DIR, allowing users to switch between different Claude accounts.

pub mod store;

pub use store::ClaudeConfigStore;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Unique identifier for a Claude configuration
pub type ClaudeConfigId = Uuid;

/// A Claude Code configuration representing an account/profile
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClaudeConfig {
    /// Unique identifier
    pub id: ClaudeConfigId,
    /// Display name (e.g., "Work", "Personal")
    pub name: String,
    /// Config directory path. None = default Claude (no env var set)
    pub config_dir: Option<PathBuf>,
}

impl ClaudeConfig {
    /// Create a new Claude configuration
    pub fn new(name: String, config_dir: Option<PathBuf>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            config_dir,
        }
    }

    /// Create the default Claude configuration (uses default ~/.claude)
    pub fn default_config() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "Default".to_string(),
            config_dir: None,
        }
    }

    /// Check if this is the default configuration (no custom config_dir)
    pub fn is_default_dir(&self) -> bool {
        self.config_dir.is_none()
    }

    /// Get display text for the config directory
    pub fn config_dir_display(&self) -> String {
        match &self.config_dir {
            Some(path) => {
                // Try to shorten path with ~ if it's in home directory
                if let Some(home) = dirs::home_dir() {
                    if let Ok(relative) = path.strip_prefix(&home) {
                        return format!("~/{}", relative.display());
                    }
                }
                path.display().to_string()
            }
            None => "~/.claude (default)".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_config_creation() {
        let config = ClaudeConfig::new(
            "Work".to_string(),
            Some(PathBuf::from("/home/user/.claude-work")),
        );
        assert_eq!(config.name, "Work");
        assert_eq!(
            config.config_dir,
            Some(PathBuf::from("/home/user/.claude-work"))
        );
        assert!(!config.is_default_dir());
    }

    #[test]
    fn test_default_config() {
        let config = ClaudeConfig::default_config();
        assert_eq!(config.name, "Default");
        assert!(config.is_default_dir());
        assert!(config.config_dir.is_none());
    }

    #[test]
    fn test_config_dir_display_default() {
        let config = ClaudeConfig::default_config();
        assert_eq!(config.config_dir_display(), "~/.claude (default)");
    }

    #[test]
    fn test_config_dir_display_custom() {
        let config = ClaudeConfig::new("Work".to_string(), Some(PathBuf::from("/tmp/claude-work")));
        assert_eq!(config.config_dir_display(), "/tmp/claude-work");
    }

    #[test]
    fn test_config_serialization() {
        let config = ClaudeConfig::new("Test".to_string(), Some(PathBuf::from("/test/path")));
        let json = serde_json::to_string(&config).unwrap();
        let parsed: ClaudeConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.id, parsed.id);
        assert_eq!(config.name, parsed.name);
        assert_eq!(config.config_dir, parsed.config_dir);
    }
}
