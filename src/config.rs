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
}

impl Default for Config {
    fn default() -> Self {
        let base = config_dir();
        Self {
            hook_port: 9999,
            worktrees_dir: base.join("worktrees"),
            hooks_dir: base.join("hooks"),
            max_output_lines: 10_000,
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
pub fn config_dir() -> PathBuf {
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".panoptes")
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
}
