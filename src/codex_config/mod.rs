//! Codex configuration management module
//!
//! This module handles multiple OpenAI Codex CLI configurations (accounts) that can be
//! used with different projects. Each configuration points to a different
//! CODEX_HOME, allowing users to switch between different Codex accounts.

pub mod store;

pub use store::CodexConfigStore;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Unique identifier for a Codex configuration
pub type CodexConfigId = Uuid;

/// A Codex CLI configuration representing an account/profile
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexConfig {
    /// Unique identifier
    pub id: CodexConfigId,
    /// Display name (e.g., "Work", "Personal")
    pub name: String,
    /// CODEX_HOME directory path. None = default (~/.codex)
    pub codex_home: Option<PathBuf>,
}

impl CodexConfig {
    /// Create a new Codex configuration
    pub fn new(name: String, codex_home: Option<PathBuf>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            codex_home,
        }
    }

    /// Create the default Codex configuration (uses default ~/.codex)
    pub fn default_config() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "Default".to_string(),
            codex_home: None,
        }
    }

    /// Check if this is the default configuration (no custom codex_home)
    pub fn is_default_dir(&self) -> bool {
        self.codex_home.is_none()
    }

    /// Get display text for the codex home directory
    pub fn codex_home_display(&self) -> String {
        match &self.codex_home {
            Some(path) => {
                // Try to shorten path with ~ if it's in home directory
                if let Some(home) = dirs::home_dir() {
                    if let Ok(relative) = path.strip_prefix(&home) {
                        return format!("~/{}", relative.display());
                    }
                }
                path.display().to_string()
            }
            None => "~/.codex (default)".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codex_config_creation() {
        let config = CodexConfig::new(
            "Work".to_string(),
            Some(PathBuf::from("/home/user/.codex-work")),
        );
        assert_eq!(config.name, "Work");
        assert_eq!(
            config.codex_home,
            Some(PathBuf::from("/home/user/.codex-work"))
        );
        assert!(!config.is_default_dir());
    }

    #[test]
    fn test_default_config() {
        let config = CodexConfig::default_config();
        assert_eq!(config.name, "Default");
        assert!(config.is_default_dir());
        assert!(config.codex_home.is_none());
    }

    #[test]
    fn test_codex_home_display_default() {
        let config = CodexConfig::default_config();
        assert_eq!(config.codex_home_display(), "~/.codex (default)");
    }

    #[test]
    fn test_codex_home_display_custom() {
        let config = CodexConfig::new("Work".to_string(), Some(PathBuf::from("/tmp/codex-work")));
        assert_eq!(config.codex_home_display(), "/tmp/codex-work");
    }

    #[test]
    fn test_config_serialization() {
        let config = CodexConfig::new("Test".to_string(), Some(PathBuf::from("/test/path")));
        let json = serde_json::to_string(&config).unwrap();
        let parsed: CodexConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.id, parsed.id);
        assert_eq!(config.name, parsed.name);
        assert_eq!(config.codex_home, parsed.codex_home);
    }
}
