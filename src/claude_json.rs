//! Claude Code JSON configuration file management
//!
//! Provides reading and writing of Claude Code's `.claude.json` configuration file.
//! This module is used for copying permissions when creating worktrees and migrating
//! permissions when deleting worktrees.
//!
//! Note: This module handles the `.claude.json` file inside a config directory,
//! not the management of multiple Claude accounts (which is in `claude_config/`).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Settings for a single project in Claude Code's configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeProjectSettings {
    /// List of allowed tools (e.g., "Bash(git init:*)", "Bash(npm install:*)")
    #[serde(default)]
    pub allowed_tools: Vec<String>,

    /// Whether the trust dialog has been accepted for this project
    #[serde(default)]
    pub has_trust_dialog_accepted: bool,

    /// MCP server configurations
    #[serde(default)]
    pub mcp_servers: HashMap<String, Value>,

    /// Preserve unknown fields for forward compatibility
    #[serde(flatten)]
    pub other: HashMap<String, Value>,
}

/// Claude Code's full configuration file structure (the .claude.json file)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClaudeJsonConfig {
    /// Project-specific settings keyed by absolute path
    #[serde(default)]
    pub projects: HashMap<String, ClaudeProjectSettings>,

    /// Preserve unknown top-level fields for forward compatibility
    #[serde(flatten)]
    pub other: HashMap<String, Value>,
}

/// Store for reading and writing Claude Code's .claude.json configuration file
///
/// This handles the per-project settings within a specific Claude config directory.
/// For managing multiple Claude accounts, see `claude_config::ClaudeConfigStore`.
pub struct ClaudeJsonStore {
    /// Path to the configuration file
    config_path: PathBuf,
}

impl ClaudeJsonStore {
    /// Create a new store for a specific Claude config directory
    ///
    /// If `config_dir` is None, uses the default `~/.claude` directory.
    /// Returns None if the home directory cannot be determined when needed.
    pub fn for_config_dir(config_dir: Option<&Path>) -> Option<Self> {
        let base_dir = match config_dir {
            Some(path) => path.to_path_buf(),
            None => dirs::home_dir()?.join(".claude"),
        };
        Some(Self {
            config_path: base_dir.join(".claude.json"),
        })
    }

    /// Create a new store pointing to the default ~/.claude/.claude.json
    ///
    /// Returns None if the home directory cannot be determined.
    ///
    /// Note: Prefer using `for_config_dir` when working with multi-account support.
    pub fn new() -> Option<Self> {
        Self::for_config_dir(None)
    }

    /// Load the configuration from disk
    ///
    /// Returns a default (empty) configuration if the file doesn't exist.
    pub fn load(&self) -> Result<ClaudeJsonConfig> {
        if !self.config_path.exists() {
            return Ok(ClaudeJsonConfig::default());
        }

        let contents = fs::read_to_string(&self.config_path)
            .with_context(|| format!("Failed to read {}", self.config_path.display()))?;

        let config: ClaudeJsonConfig = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse {}", self.config_path.display()))?;

        Ok(config)
    }

    /// Save the configuration to disk
    ///
    /// Creates a backup (.json.bak) before writing.
    pub fn save(&self, config: &ClaudeJsonConfig) -> Result<()> {
        // Create backup if file exists
        if self.config_path.exists() {
            let backup_path = self.config_path.with_extension("json.bak");
            fs::copy(&self.config_path, &backup_path)
                .with_context(|| format!("Failed to create backup at {}", backup_path.display()))?;
        }

        let contents = serde_json::to_string_pretty(config)
            .context("Failed to serialize Claude configuration")?;

        fs::write(&self.config_path, contents)
            .with_context(|| format!("Failed to write {}", self.config_path.display()))?;

        Ok(())
    }

    /// Check if a path has any Claude settings configured
    pub fn has_settings(&self, path: &str) -> Result<bool> {
        let config = self.load()?;
        if let Some(settings) = config.projects.get(path) {
            // Has settings if there are any tools, MCP servers, or trust accepted
            Ok(!settings.allowed_tools.is_empty()
                || !settings.mcp_servers.is_empty()
                || settings.has_trust_dialog_accepted
                || !settings.other.is_empty())
        } else {
            Ok(false)
        }
    }

    /// Get settings for a specific path
    pub fn get_settings(&self, path: &str) -> Result<Option<ClaudeProjectSettings>> {
        let config = self.load()?;
        Ok(config.projects.get(path).cloned())
    }

    /// Copy all settings from one path to another
    ///
    /// This copies the entire ClaudeProjectSettings including tools, MCP servers,
    /// trust acceptance, and any other fields.
    pub fn copy_settings(&self, from: &str, to: &str) -> Result<()> {
        let mut config = self.load()?;

        if let Some(source_settings) = config.projects.get(from).cloned() {
            config.projects.insert(to.to_string(), source_settings);
            self.save(&config)?;
        }

        Ok(())
    }

    /// Compare tool permissions between two paths
    ///
    /// Returns a tuple of (shared_tools, only_in_a, only_in_b)
    pub fn compare_tools(
        &self,
        path_a: &str,
        path_b: &str,
    ) -> Result<(Vec<String>, Vec<String>, Vec<String>)> {
        let config = self.load()?;

        let tools_a: std::collections::HashSet<String> = config
            .projects
            .get(path_a)
            .map(|s| s.allowed_tools.iter().cloned().collect())
            .unwrap_or_default();

        let tools_b: std::collections::HashSet<String> = config
            .projects
            .get(path_b)
            .map(|s| s.allowed_tools.iter().cloned().collect())
            .unwrap_or_default();

        let shared: Vec<String> = tools_a.intersection(&tools_b).cloned().collect();
        let only_a: Vec<String> = tools_a.difference(&tools_b).cloned().collect();
        let only_b: Vec<String> = tools_b.difference(&tools_a).cloned().collect();

        Ok((shared, only_a, only_b))
    }

    /// Merge unique tools from worktree into main repository
    ///
    /// This adds any tools that exist in the worktree but not in the main repo
    /// to the main repo's allowed tools list.
    ///
    /// Returns the list of tools that were added.
    pub fn merge_settings(&self, worktree_path: &str, main_path: &str) -> Result<Vec<String>> {
        let mut config = self.load()?;

        let worktree_tools: std::collections::HashSet<String> = config
            .projects
            .get(worktree_path)
            .map(|s| s.allowed_tools.iter().cloned().collect())
            .unwrap_or_default();

        let main_tools: std::collections::HashSet<String> = config
            .projects
            .get(main_path)
            .map(|s| s.allowed_tools.iter().cloned().collect())
            .unwrap_or_default();

        // Find tools unique to worktree
        let unique_tools: Vec<String> = worktree_tools.difference(&main_tools).cloned().collect();

        if !unique_tools.is_empty() {
            // Get or create main settings
            let main_settings = config.projects.entry(main_path.to_string()).or_default();

            // Add unique tools
            for tool in &unique_tools {
                if !main_settings.allowed_tools.contains(tool) {
                    main_settings.allowed_tools.push(tool.clone());
                }
            }

            self.save(&config)?;
        }

        Ok(unique_tools)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_store(content: &str) -> (ClaudeJsonStore, NamedTempFile) {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();

        let store = ClaudeJsonStore {
            config_path: file.path().to_path_buf(),
        };

        (store, file)
    }

    #[test]
    fn test_load_empty_config() {
        let (store, _file) = create_test_store("{}");
        let config = store.load().unwrap();
        assert!(config.projects.is_empty());
    }

    #[test]
    fn test_load_config_with_projects() {
        let json = r#"{
            "projects": {
                "/path/to/project": {
                    "allowedTools": ["Bash(git:*)", "Bash(npm:*)"],
                    "hasTrustDialogAccepted": true
                }
            }
        }"#;

        let (store, _file) = create_test_store(json);
        let config = store.load().unwrap();

        assert!(config.projects.contains_key("/path/to/project"));
        let settings = config.projects.get("/path/to/project").unwrap();
        assert_eq!(settings.allowed_tools.len(), 2);
        assert!(settings.has_trust_dialog_accepted);
    }

    #[test]
    fn test_has_settings() {
        let json = r#"{
            "projects": {
                "/with/tools": {
                    "allowedTools": ["Bash(git:*)"]
                },
                "/empty": {}
            }
        }"#;

        let (store, _file) = create_test_store(json);

        assert!(store.has_settings("/with/tools").unwrap());
        assert!(!store.has_settings("/empty").unwrap());
        assert!(!store.has_settings("/nonexistent").unwrap());
    }

    #[test]
    fn test_compare_tools() {
        let json = r#"{
            "projects": {
                "/path/a": {
                    "allowedTools": ["tool1", "tool2", "shared"]
                },
                "/path/b": {
                    "allowedTools": ["tool3", "shared"]
                }
            }
        }"#;

        let (store, _file) = create_test_store(json);
        let (shared, only_a, only_b) = store.compare_tools("/path/a", "/path/b").unwrap();

        assert_eq!(shared, vec!["shared"]);
        assert!(only_a.contains(&"tool1".to_string()));
        assert!(only_a.contains(&"tool2".to_string()));
        assert_eq!(only_b, vec!["tool3"]);
    }

    #[test]
    fn test_copy_settings() {
        let json = r#"{
            "projects": {
                "/source": {
                    "allowedTools": ["tool1"],
                    "hasTrustDialogAccepted": true
                }
            }
        }"#;

        let (store, _file) = create_test_store(json);
        store.copy_settings("/source", "/dest").unwrap();

        let config = store.load().unwrap();
        assert!(config.projects.contains_key("/dest"));
        let dest_settings = config.projects.get("/dest").unwrap();
        assert_eq!(dest_settings.allowed_tools, vec!["tool1"]);
        assert!(dest_settings.has_trust_dialog_accepted);
    }

    #[test]
    fn test_merge_settings() {
        let json = r#"{
            "projects": {
                "/worktree": {
                    "allowedTools": ["tool1", "shared"]
                },
                "/main": {
                    "allowedTools": ["shared", "existing"]
                }
            }
        }"#;

        let (store, _file) = create_test_store(json);
        let added = store.merge_settings("/worktree", "/main").unwrap();

        assert_eq!(added, vec!["tool1"]);

        let config = store.load().unwrap();
        let main_settings = config.projects.get("/main").unwrap();
        assert!(main_settings.allowed_tools.contains(&"tool1".to_string()));
        assert!(main_settings.allowed_tools.contains(&"shared".to_string()));
        assert!(main_settings
            .allowed_tools
            .contains(&"existing".to_string()));
    }

    #[test]
    fn test_preserves_unknown_fields() {
        let json = r#"{
            "someOtherField": "value",
            "projects": {
                "/path": {
                    "allowedTools": ["tool1"],
                    "unknownSetting": true
                }
            }
        }"#;

        let (store, _file) = create_test_store(json);
        let config = store.load().unwrap();

        // Check that unknown top-level field is preserved
        assert!(config.other.contains_key("someOtherField"));

        // Check that unknown project setting is preserved
        let settings = config.projects.get("/path").unwrap();
        assert!(settings.other.contains_key("unknownSetting"));
    }

    #[test]
    fn test_for_config_dir_custom() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let store = ClaudeJsonStore::for_config_dir(Some(temp_dir.path())).unwrap();
        assert_eq!(
            store.config_path,
            temp_dir.path().join(".claude.json")
        );
    }
}
