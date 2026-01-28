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

    /// Remove settings for a path (used when deleting worktrees)
    ///
    /// This removes the project entry from the configuration file to avoid
    /// accumulating stale entries over time.
    ///
    /// Returns Ok(true) if an entry was removed, Ok(false) if no entry existed.
    pub fn remove_settings(&self, path: &str) -> Result<bool> {
        let mut config = self.load()?;

        if config.projects.remove(path).is_some() {
            self.save(&config)?;
            Ok(true)
        } else {
            Ok(false)
        }
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

/// Copy local settings.local.json from source project to destination project
///
/// This function copies the `.claude/settings.local.json` file from one project
/// directory to another, preserving Claude Code trust settings and permissions
/// when creating worktrees.
///
/// # Safety
/// - Only copies if destination doesn't already exist (won't overwrite)
/// - Creates parent directories if needed
///
/// # Returns
/// - `Ok(true)` if settings were copied
/// - `Ok(false)` if source doesn't exist or destination already exists
pub fn copy_local_settings(source_dir: &Path, dest_dir: &Path) -> Result<bool> {
    let source_settings = source_dir.join(".claude").join("settings.local.json");
    let dest_settings = dest_dir.join(".claude").join("settings.local.json");

    if !source_settings.exists() {
        tracing::debug!(
            "No source settings.local.json at {}",
            source_settings.display()
        );
        return Ok(false);
    }

    // Don't overwrite existing settings
    if dest_settings.exists() {
        tracing::debug!(
            "Destination settings already exist at {}, skipping copy",
            dest_settings.display()
        );
        return Ok(false);
    }

    // Create .claude directory in destination if needed
    if let Some(parent) = dest_settings.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("Failed to create .claude directory at {}", parent.display())
        })?;
    }

    fs::copy(&source_settings, &dest_settings).with_context(|| {
        format!(
            "Failed to copy settings from {} to {}",
            source_settings.display(),
            dest_settings.display()
        )
    })?;

    tracing::info!(
        "Copied local Claude settings from {} to {}",
        source_settings.display(),
        dest_settings.display()
    );

    Ok(true)
}

/// Merge local settings from worktree back to main repo (excluding hooks)
///
/// This function merges non-hook settings from a worktree's `.claude/settings.local.json`
/// back to the main repository, preserving any new permissions granted during the worktree's
/// lifetime when deleting it.
///
/// # What gets merged
/// - All top-level keys EXCEPT "hooks" (which are Panoptes-managed)
/// - Only keys that don't already exist in the main repo are added
///
/// # Safety
/// - Creates backup before modifying main repo settings
/// - Only adds new keys, never overwrites existing ones
///
/// # Returns
/// - List of keys that were added to main repo
pub fn merge_local_settings(worktree_dir: &Path, main_dir: &Path) -> Result<Vec<String>> {
    let worktree_settings = worktree_dir.join(".claude").join("settings.local.json");
    let main_settings = main_dir.join(".claude").join("settings.local.json");

    if !worktree_settings.exists() {
        tracing::debug!(
            "No worktree settings.local.json at {}",
            worktree_settings.display()
        );
        return Ok(vec![]);
    }

    // Load worktree settings
    let worktree_content = fs::read_to_string(&worktree_settings)
        .with_context(|| format!("Failed to read {}", worktree_settings.display()))?;

    let worktree_json: serde_json::Value = serde_json::from_str(&worktree_content)
        .with_context(|| format!("Failed to parse {}", worktree_settings.display()))?;

    // Load or create main settings
    let mut main_json: serde_json::Value = if main_settings.exists() {
        let content = fs::read_to_string(&main_settings)
            .with_context(|| format!("Failed to read {}", main_settings.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", main_settings.display()))?
    } else {
        serde_json::json!({})
    };

    let mut added = Vec::new();

    // Get worktree settings as object
    let Some(worktree_obj) = worktree_json.as_object() else {
        return Ok(vec![]);
    };

    // Ensure main_json is an object
    if !main_json.is_object() {
        main_json = serde_json::json!({});
    }

    // Merge non-hook keys that don't already exist in main
    for (key, value) in worktree_obj.iter() {
        // Skip hooks - they're Panoptes-managed
        if key == "hooks" {
            continue;
        }

        // Only add if main doesn't already have this key
        if !main_json.as_object().is_some_and(|o| o.contains_key(key)) {
            main_json[key] = value.clone();
            added.push(key.clone());
        }
    }

    if !added.is_empty() {
        // Create backup before writing
        if main_settings.exists() {
            let backup_path = main_settings.with_extension("json.bak");
            if let Err(e) = fs::copy(&main_settings, &backup_path) {
                tracing::warn!(
                    "Failed to create backup of {}: {}",
                    main_settings.display(),
                    e
                );
            }
        }

        // Create .claude directory if needed
        if let Some(parent) = main_settings.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&main_settings, serde_json::to_string_pretty(&main_json)?)
            .with_context(|| format!("Failed to write {}", main_settings.display()))?;

        tracing::info!(
            "Merged {} settings from worktree to main repo: {:?}",
            added.len(),
            added
        );
    }

    Ok(added)
}

/// Check if a worktree has unique local settings that should be migrated
///
/// Returns true if the worktree has settings.local.json with keys (other than hooks)
/// that don't exist in the main repo.
pub fn has_unique_local_settings(worktree_dir: &Path, main_dir: &Path) -> Result<bool> {
    let worktree_settings = worktree_dir.join(".claude").join("settings.local.json");
    let main_settings = main_dir.join(".claude").join("settings.local.json");

    if !worktree_settings.exists() {
        return Ok(false);
    }

    // Load worktree settings
    let worktree_content = fs::read_to_string(&worktree_settings)?;
    let worktree_json: serde_json::Value = serde_json::from_str(&worktree_content)?;

    // Load main settings (or empty if doesn't exist)
    let main_json: serde_json::Value = if main_settings.exists() {
        let content = fs::read_to_string(&main_settings)?;
        serde_json::from_str(&content)?
    } else {
        serde_json::json!({})
    };

    // Check for unique keys in worktree
    if let Some(worktree_obj) = worktree_json.as_object() {
        for key in worktree_obj.keys() {
            // Skip hooks
            if key == "hooks" {
                continue;
            }

            // If main doesn't have this key, worktree has unique settings
            if !main_json.as_object().is_some_and(|o| o.contains_key(key)) {
                return Ok(true);
            }
        }
    }

    Ok(false)
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
    fn test_remove_settings() {
        let json = r#"{
            "projects": {
                "/to-remove": {
                    "allowedTools": ["tool1"]
                },
                "/to-keep": {
                    "allowedTools": ["tool2"]
                }
            }
        }"#;

        let (store, _file) = create_test_store(json);

        // Remove existing entry
        let removed = store.remove_settings("/to-remove").unwrap();
        assert!(removed);

        // Verify entry is gone
        let config = store.load().unwrap();
        assert!(!config.projects.contains_key("/to-remove"));
        assert!(config.projects.contains_key("/to-keep"));

        // Remove non-existent entry
        let removed = store.remove_settings("/nonexistent").unwrap();
        assert!(!removed);
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
        assert_eq!(store.config_path, temp_dir.path().join(".claude.json"));
    }

    #[test]
    fn test_copy_local_settings_success() {
        let source_dir = tempfile::TempDir::new().unwrap();
        let dest_dir = tempfile::TempDir::new().unwrap();

        // Create source settings
        let source_claude_dir = source_dir.path().join(".claude");
        fs::create_dir_all(&source_claude_dir).unwrap();
        let source_settings = source_claude_dir.join("settings.local.json");
        fs::write(
            &source_settings,
            r#"{"enabledBetaFeatures": ["feature1"], "hasTrustDialogAccepted": true}"#,
        )
        .unwrap();

        // Copy settings
        let result = copy_local_settings(source_dir.path(), dest_dir.path()).unwrap();
        assert!(result);

        // Verify destination has the settings
        let dest_settings = dest_dir.path().join(".claude").join("settings.local.json");
        assert!(dest_settings.exists());

        let content = fs::read_to_string(&dest_settings).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(json["hasTrustDialogAccepted"].as_bool().unwrap());
    }

    #[test]
    fn test_copy_local_settings_no_source() {
        let source_dir = tempfile::TempDir::new().unwrap();
        let dest_dir = tempfile::TempDir::new().unwrap();

        // No source settings
        let result = copy_local_settings(source_dir.path(), dest_dir.path()).unwrap();
        assert!(!result);

        // Destination should not have settings
        let dest_settings = dest_dir.path().join(".claude").join("settings.local.json");
        assert!(!dest_settings.exists());
    }

    #[test]
    fn test_copy_local_settings_dest_exists() {
        let source_dir = tempfile::TempDir::new().unwrap();
        let dest_dir = tempfile::TempDir::new().unwrap();

        // Create source settings
        let source_claude_dir = source_dir.path().join(".claude");
        fs::create_dir_all(&source_claude_dir).unwrap();
        fs::write(
            source_claude_dir.join("settings.local.json"),
            r#"{"source": true}"#,
        )
        .unwrap();

        // Create existing destination settings
        let dest_claude_dir = dest_dir.path().join(".claude");
        fs::create_dir_all(&dest_claude_dir).unwrap();
        fs::write(
            dest_claude_dir.join("settings.local.json"),
            r#"{"dest": true}"#,
        )
        .unwrap();

        // Copy should not overwrite
        let result = copy_local_settings(source_dir.path(), dest_dir.path()).unwrap();
        assert!(!result);

        // Destination should still have original content
        let dest_settings = dest_dir.path().join(".claude").join("settings.local.json");
        let content = fs::read_to_string(&dest_settings).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(json["dest"].as_bool().unwrap());
        assert!(json.get("source").is_none());
    }

    #[test]
    fn test_merge_local_settings_success() {
        let worktree_dir = tempfile::TempDir::new().unwrap();
        let main_dir = tempfile::TempDir::new().unwrap();

        // Create worktree settings with some unique keys
        let worktree_claude_dir = worktree_dir.path().join(".claude");
        fs::create_dir_all(&worktree_claude_dir).unwrap();
        fs::write(
            worktree_claude_dir.join("settings.local.json"),
            r#"{"newSetting": true, "hooks": {"PreToolUse": []}, "shared": "worktree"}"#,
        )
        .unwrap();

        // Create main settings
        let main_claude_dir = main_dir.path().join(".claude");
        fs::create_dir_all(&main_claude_dir).unwrap();
        fs::write(
            main_claude_dir.join("settings.local.json"),
            r#"{"shared": "main", "existing": true}"#,
        )
        .unwrap();

        // Merge settings
        let added = merge_local_settings(worktree_dir.path(), main_dir.path()).unwrap();

        // Only newSetting should be added (hooks skipped, shared already exists)
        assert_eq!(added, vec!["newSetting"]);

        // Verify main has the new setting but not overwritten shared
        let main_settings = main_dir.path().join(".claude").join("settings.local.json");
        let content = fs::read_to_string(&main_settings).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(json["newSetting"].as_bool().unwrap());
        assert_eq!(json["shared"].as_str().unwrap(), "main"); // Not overwritten
        assert!(json["existing"].as_bool().unwrap());
        assert!(json.get("hooks").is_none()); // hooks not copied
    }

    #[test]
    fn test_merge_local_settings_no_worktree() {
        let worktree_dir = tempfile::TempDir::new().unwrap();
        let main_dir = tempfile::TempDir::new().unwrap();

        // No worktree settings
        let added = merge_local_settings(worktree_dir.path(), main_dir.path()).unwrap();
        assert!(added.is_empty());
    }

    #[test]
    fn test_merge_local_settings_creates_main() {
        let worktree_dir = tempfile::TempDir::new().unwrap();
        let main_dir = tempfile::TempDir::new().unwrap();

        // Create worktree settings
        let worktree_claude_dir = worktree_dir.path().join(".claude");
        fs::create_dir_all(&worktree_claude_dir).unwrap();
        fs::write(
            worktree_claude_dir.join("settings.local.json"),
            r#"{"newSetting": true}"#,
        )
        .unwrap();

        // No main settings yet
        let added = merge_local_settings(worktree_dir.path(), main_dir.path()).unwrap();
        assert_eq!(added, vec!["newSetting"]);

        // Main settings should now exist
        let main_settings = main_dir.path().join(".claude").join("settings.local.json");
        assert!(main_settings.exists());
    }

    #[test]
    fn test_has_unique_local_settings_true() {
        let worktree_dir = tempfile::TempDir::new().unwrap();
        let main_dir = tempfile::TempDir::new().unwrap();

        // Create worktree settings with unique key
        let worktree_claude_dir = worktree_dir.path().join(".claude");
        fs::create_dir_all(&worktree_claude_dir).unwrap();
        fs::write(
            worktree_claude_dir.join("settings.local.json"),
            r#"{"uniqueSetting": true}"#,
        )
        .unwrap();

        // No main settings
        let result = has_unique_local_settings(worktree_dir.path(), main_dir.path()).unwrap();
        assert!(result);
    }

    #[test]
    fn test_has_unique_local_settings_false_only_hooks() {
        let worktree_dir = tempfile::TempDir::new().unwrap();
        let main_dir = tempfile::TempDir::new().unwrap();

        // Create worktree settings with only hooks
        let worktree_claude_dir = worktree_dir.path().join(".claude");
        fs::create_dir_all(&worktree_claude_dir).unwrap();
        fs::write(
            worktree_claude_dir.join("settings.local.json"),
            r#"{"hooks": {"PreToolUse": []}}"#,
        )
        .unwrap();

        let result = has_unique_local_settings(worktree_dir.path(), main_dir.path()).unwrap();
        assert!(!result); // Hooks are ignored
    }

    #[test]
    fn test_has_unique_local_settings_false_all_exist() {
        let worktree_dir = tempfile::TempDir::new().unwrap();
        let main_dir = tempfile::TempDir::new().unwrap();

        // Create worktree settings
        let worktree_claude_dir = worktree_dir.path().join(".claude");
        fs::create_dir_all(&worktree_claude_dir).unwrap();
        fs::write(
            worktree_claude_dir.join("settings.local.json"),
            r#"{"setting": true}"#,
        )
        .unwrap();

        // Create main settings with same key
        let main_claude_dir = main_dir.path().join(".claude");
        fs::create_dir_all(&main_claude_dir).unwrap();
        fs::write(
            main_claude_dir.join("settings.local.json"),
            r#"{"setting": false}"#,
        )
        .unwrap();

        let result = has_unique_local_settings(worktree_dir.path(), main_dir.path()).unwrap();
        assert!(!result); // Key exists in main (even if value differs)
    }
}
