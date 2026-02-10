//! Codex configuration persistence
//!
//! Handles saving and loading Codex configurations to/from disk.

use super::{CodexConfig, CodexConfigId};
use crate::config::config_dir;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Serializable format for the configuration store
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreData {
    configs: Vec<CodexConfig>,
    #[serde(default)]
    default_config_id: Option<CodexConfigId>,
}

/// Backup a corrupted file by renaming it with a .backup extension
fn backup_corrupted_file(path: &Path) {
    let backup_path = path.with_extension("json.backup");
    if let Err(e) = std::fs::rename(path, &backup_path) {
        tracing::warn!(
            "Failed to backup corrupted file {} to {}: {}",
            path.display(),
            backup_path.display(),
            e
        );
    } else {
        tracing::info!(
            "Corrupted codex configs file backed up to {}",
            backup_path.display()
        );
    }
}

/// Store for persisting Codex configurations
#[derive(Debug)]
pub struct CodexConfigStore {
    /// All configurations indexed by ID
    configs: HashMap<CodexConfigId, CodexConfig>,
    /// ID of the default configuration
    default_config_id: Option<CodexConfigId>,
    /// Path to the codex_configs.json file
    store_path: PathBuf,
}

impl Default for CodexConfigStore {
    fn default() -> Self {
        Self {
            configs: HashMap::new(),
            default_config_id: None,
            store_path: codex_configs_file_path(),
        }
    }
}

impl CodexConfigStore {
    /// Create a new empty store
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a store with a custom path (for testing)
    pub fn with_path(path: PathBuf) -> Self {
        Self {
            configs: HashMap::new(),
            default_config_id: None,
            store_path: path,
        }
    }

    /// Get all configurations
    pub fn configs(&self) -> impl Iterator<Item = &CodexConfig> {
        self.configs.values()
    }

    /// Get all configurations as a sorted vector (by name)
    pub fn configs_sorted(&self) -> Vec<&CodexConfig> {
        let mut configs: Vec<_> = self.configs.values().collect();
        configs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        configs
    }

    /// Get a configuration by ID
    pub fn get(&self, id: CodexConfigId) -> Option<&CodexConfig> {
        self.configs.get(&id)
    }

    /// Get a mutable reference to a configuration by ID
    pub fn get_mut(&mut self, id: CodexConfigId) -> Option<&mut CodexConfig> {
        self.configs.get_mut(&id)
    }

    /// Find a configuration by name
    pub fn find_by_name(&self, name: &str) -> Option<&CodexConfig> {
        self.configs.values().find(|c| c.name == name)
    }

    /// Find a configuration by codex home path
    pub fn find_by_codex_home(&self, path: &Path) -> Option<&CodexConfig> {
        self.configs.values().find(|c| match &c.codex_home {
            Some(dir) => dir == path,
            None => false,
        })
    }

    /// Check if a codex home directory is already used by another config
    pub fn is_codex_home_used(&self, path: Option<&Path>) -> bool {
        self.configs.values().any(|c| {
            match (&c.codex_home, path) {
                (Some(existing), Some(new)) => existing == new,
                (None, None) => true, // Default config already exists
                _ => false,
            }
        })
    }

    /// Add a configuration to the store
    pub fn add(&mut self, config: CodexConfig) {
        let id = config.id;
        self.configs.insert(id, config);

        // If this is the first config, make it the default
        if self.configs.len() == 1 {
            self.default_config_id = Some(id);
        }
    }

    /// Remove a configuration
    pub fn remove(&mut self, id: CodexConfigId) -> Option<CodexConfig> {
        let config = self.configs.remove(&id);

        // If we removed the default, pick a new default
        if self.default_config_id == Some(id) {
            self.default_config_id = self.configs.keys().next().copied();
        }

        config
    }

    /// Get the default configuration
    pub fn get_default(&self) -> Option<&CodexConfig> {
        self.default_config_id.and_then(|id| self.configs.get(&id))
    }

    /// Get the default configuration ID
    pub fn get_default_id(&self) -> Option<CodexConfigId> {
        self.default_config_id
    }

    /// Set the default configuration
    pub fn set_default(&mut self, id: CodexConfigId) -> bool {
        if self.configs.contains_key(&id) {
            self.default_config_id = Some(id);
            true
        } else {
            false
        }
    }

    /// Get the number of configurations
    pub fn count(&self) -> usize {
        self.configs.len()
    }

    /// Check if there are any configurations
    pub fn is_empty(&self) -> bool {
        self.configs.is_empty()
    }

    /// Load store from disk
    pub fn load() -> Result<Self> {
        Self::load_from(&codex_configs_file_path())
    }

    /// Load store from a specific path
    ///
    /// If the file is corrupted, this will create a backup and return an empty store.
    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::with_path(path.to_path_buf()));
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to read codex configs file: {}", e);
                backup_corrupted_file(path);
                return Err(anyhow::anyhow!(
                    "Could not read codex configs file ({}). Starting with empty config list.",
                    e
                ));
            }
        };

        let data: StoreData = match serde_json::from_str(&content) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("Codex configs file is corrupted: {}", e);
                backup_corrupted_file(path);
                return Err(anyhow::anyhow!(
                    "Codex configs file is corrupted and could not be parsed ({}). \
                     A backup has been created at {}.backup. \
                     Starting with empty config list.",
                    e,
                    path.display()
                ));
            }
        };

        let configs = data.configs.into_iter().map(|c| (c.id, c)).collect();

        Ok(Self {
            configs,
            default_config_id: data.default_config_id,
            store_path: path.to_path_buf(),
        })
    }

    /// Save store to disk
    pub fn save(&self) -> Result<()> {
        self.save_to(&self.store_path)
    }

    /// Save store to a specific path
    pub fn save_to(&self, path: &Path) -> Result<()> {
        use crate::config::{categorize_io_error, DiskErrorKind};

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                let kind = categorize_io_error(&e);
                match kind {
                    DiskErrorKind::PermissionDenied => {
                        anyhow::bail!(
                            "Permission denied creating directory {:?}. Check file permissions.",
                            parent
                        );
                    }
                    DiskErrorKind::DiskFull => {
                        anyhow::bail!("Disk full - cannot create directory {:?}", parent);
                    }
                    _ => {
                        return Err(e).context("Failed to create directory for codex configs file");
                    }
                }
            }
        }

        let data = StoreData {
            configs: self.configs.values().cloned().collect(),
            default_config_id: self.default_config_id,
        };

        let content =
            serde_json::to_string_pretty(&data).context("Failed to serialize codex configs")?;

        if let Err(e) = std::fs::write(path, &content) {
            let kind = categorize_io_error(&e);
            match kind {
                DiskErrorKind::DiskFull => {
                    anyhow::bail!(
                        "Disk full - free space needed to save codex configs. Your changes may not be saved."
                    );
                }
                DiskErrorKind::PermissionDenied => {
                    anyhow::bail!(
                        "Permission denied writing to {:?}. Check file permissions.",
                        path
                    );
                }
                _ => {
                    return Err(e).context("Failed to write codex configs file");
                }
            }
        }

        Ok(())
    }
}

/// Get the path to the codex configs file
pub fn codex_configs_file_path() -> PathBuf {
    config_dir().join("codex_configs.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_store_add_config() {
        let mut store = CodexConfigStore::new();
        let config = CodexConfig::new("Work".to_string(), Some(PathBuf::from("/home/.codex-work")));
        let config_id = config.id;

        store.add(config);
        assert_eq!(store.count(), 1);
        assert!(store.get(config_id).is_some());
        // First config should be default
        assert_eq!(store.get_default_id(), Some(config_id));
    }

    #[test]
    fn test_store_remove_config() {
        let mut store = CodexConfigStore::new();
        let config1 =
            CodexConfig::new("Work".to_string(), Some(PathBuf::from("/home/.codex-work")));
        let config2 = CodexConfig::new(
            "Personal".to_string(),
            Some(PathBuf::from("/home/.codex-personal")),
        );
        let id1 = config1.id;
        let id2 = config2.id;

        store.add(config1);
        store.add(config2);
        assert_eq!(store.count(), 2);
        assert_eq!(store.get_default_id(), Some(id1));

        // Remove the default
        store.remove(id1);
        assert_eq!(store.count(), 1);
        // Default should switch to the other one
        assert_eq!(store.get_default_id(), Some(id2));
    }

    #[test]
    fn test_store_set_default() {
        let mut store = CodexConfigStore::new();
        let config1 =
            CodexConfig::new("Work".to_string(), Some(PathBuf::from("/home/.codex-work")));
        let config2 = CodexConfig::new(
            "Personal".to_string(),
            Some(PathBuf::from("/home/.codex-personal")),
        );
        let _id1 = config1.id;
        let id2 = config2.id;

        store.add(config1);
        store.add(config2);

        assert!(store.set_default(id2));
        assert_eq!(store.get_default_id(), Some(id2));

        // Setting non-existent ID should fail
        assert!(!store.set_default(uuid::Uuid::new_v4()));
    }

    #[test]
    fn test_store_find_by_name() {
        let mut store = CodexConfigStore::new();
        let config = CodexConfig::new("Work".to_string(), Some(PathBuf::from("/home/.codex-work")));
        let config_id = config.id;

        store.add(config);

        let found = store.find_by_name("Work");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, config_id);

        let not_found = store.find_by_name("Personal");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_store_is_codex_home_used() {
        let mut store = CodexConfigStore::new();
        let config = CodexConfig::new("Work".to_string(), Some(PathBuf::from("/home/.codex-work")));

        store.add(config);

        assert!(store.is_codex_home_used(Some(Path::new("/home/.codex-work"))));
        assert!(!store.is_codex_home_used(Some(Path::new("/home/.codex-other"))));
    }

    #[test]
    fn test_store_configs_sorted() {
        let mut store = CodexConfigStore::new();

        store.add(CodexConfig::new(
            "Zebra".to_string(),
            Some(PathBuf::from("/z")),
        ));
        store.add(CodexConfig::new(
            "Alpha".to_string(),
            Some(PathBuf::from("/a")),
        ));
        store.add(CodexConfig::new(
            "beta".to_string(),
            Some(PathBuf::from("/b")),
        ));

        let sorted = store.configs_sorted();
        assert_eq!(sorted[0].name, "Alpha");
        assert_eq!(sorted[1].name, "beta");
        assert_eq!(sorted[2].name, "Zebra");
    }

    #[test]
    fn test_store_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("codex_configs.json");

        // Create and save store
        let mut store = CodexConfigStore::with_path(path.clone());
        let config = CodexConfig::new("Work".to_string(), Some(PathBuf::from("/home/.codex-work")));
        let config_id = config.id;
        store.add(config);

        store.save().unwrap();

        // Load into new store
        let loaded = CodexConfigStore::load_from(&path).unwrap();
        assert_eq!(loaded.count(), 1);
        assert!(loaded.get(config_id).is_some());
        assert_eq!(loaded.get_default_id(), Some(config_id));
    }

    #[test]
    fn test_store_load_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.json");

        let store = CodexConfigStore::load_from(&path).unwrap();
        assert_eq!(store.count(), 0);
    }
}
