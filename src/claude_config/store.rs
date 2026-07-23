//! Claude configuration persistence
//!
//! Handles saving and loading Claude configurations to/from disk. The
//! mechanism lives in [`crate::agent_profiles::ProfileStore`]; this module
//! binds it to [`ClaudeConfig`] and keeps the Claude-flavoured method names.

use super::ClaudeConfig;
use crate::agent_profiles::{AgentProfile, ProfileStore};
use std::path::Path;
use uuid::Uuid;

impl AgentProfile for ClaudeConfig {
    const STORE_FILENAME: &'static str = "claude_configs.json";
    const LABEL: &'static str = "claude configs";

    fn id(&self) -> Uuid {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn home_dir(&self) -> Option<&Path> {
        self.config_dir.as_deref()
    }

    fn home_dir_display(&self) -> String {
        self.config_dir_display()
    }

    fn new_profile(name: String, home_dir: Option<std::path::PathBuf>) -> Self {
        Self::new(name, home_dir)
    }
}

/// Store for persisting Claude configurations
pub type ClaudeConfigStore = ProfileStore<ClaudeConfig>;

impl ClaudeConfigStore {
    /// Check if a config directory is already used by another config
    pub fn is_config_dir_used(&self, path: Option<&Path>) -> bool {
        self.is_home_dir_used(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_config::ClaudeConfigId;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn test_store_add_config() {
        let mut store = ClaudeConfigStore::new();
        let config = ClaudeConfig::new(
            "Work".to_string(),
            Some(PathBuf::from("/home/.claude-work")),
        );
        let config_id = config.id;

        store.add(config);
        assert_eq!(store.count(), 1);
        assert!(store.get(config_id).is_some());
        // First config should be default
        assert_eq!(store.get_default_id(), Some(config_id));
    }

    #[test]
    fn test_store_remove_config() {
        let mut store = ClaudeConfigStore::new();
        let config1 = ClaudeConfig::new(
            "Work".to_string(),
            Some(PathBuf::from("/home/.claude-work")),
        );
        let config2 = ClaudeConfig::new(
            "Personal".to_string(),
            Some(PathBuf::from("/home/.claude-personal")),
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
    fn test_store_remove_default_picks_alphabetically_first_replacement() {
        let mut store = ClaudeConfigStore::new();
        let zulu = ClaudeConfig::new("Zulu".to_string(), Some(PathBuf::from("/z")));
        let alpha = ClaudeConfig::new("Alpha".to_string(), Some(PathBuf::from("/a")));
        let mike = ClaudeConfig::new("Mike".to_string(), Some(PathBuf::from("/m")));
        let zulu_id = zulu.id;
        let alpha_id = alpha.id;

        store.add(zulu);
        store.add(alpha);
        store.add(mike);
        assert_eq!(store.get_default_id(), Some(zulu_id));

        store.remove(zulu_id);
        // Deterministic: alphabetically first remaining config, not HashMap order
        assert_eq!(store.get_default_id(), Some(alpha_id));
    }

    #[test]
    fn test_store_set_default() {
        let mut store = ClaudeConfigStore::new();
        let config1 = ClaudeConfig::new(
            "Work".to_string(),
            Some(PathBuf::from("/home/.claude-work")),
        );
        let config2 = ClaudeConfig::new(
            "Personal".to_string(),
            Some(PathBuf::from("/home/.claude-personal")),
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
        let mut store = ClaudeConfigStore::new();
        let config = ClaudeConfig::new(
            "Work".to_string(),
            Some(PathBuf::from("/home/.claude-work")),
        );
        let config_id = config.id;

        store.add(config);

        let found = store.find_by_name("Work");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, config_id);

        let not_found = store.find_by_name("Personal");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_store_is_config_dir_used() {
        let mut store = ClaudeConfigStore::new();
        let config = ClaudeConfig::new(
            "Work".to_string(),
            Some(PathBuf::from("/home/.claude-work")),
        );

        store.add(config);

        assert!(store.is_config_dir_used(Some(Path::new("/home/.claude-work"))));
        assert!(!store.is_config_dir_used(Some(Path::new("/home/.claude-other"))));
    }

    #[test]
    fn test_store_configs_sorted() {
        let mut store = ClaudeConfigStore::new();

        store.add(ClaudeConfig::new(
            "Zebra".to_string(),
            Some(PathBuf::from("/z")),
        ));
        store.add(ClaudeConfig::new(
            "Alpha".to_string(),
            Some(PathBuf::from("/a")),
        ));
        store.add(ClaudeConfig::new(
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
        let path = temp_dir.path().join("claude_configs.json");

        // Create and save store
        let mut store = ClaudeConfigStore::with_path(path.clone());
        let config = ClaudeConfig::new(
            "Work".to_string(),
            Some(PathBuf::from("/home/.claude-work")),
        );
        let config_id = config.id;
        store.add(config);

        store.save().unwrap();

        // Load into new store
        let (loaded, warning) = ClaudeConfigStore::load_from_with_status(&path);
        assert!(warning.is_none());
        assert_eq!(loaded.count(), 1);
        assert!(loaded.get(config_id).is_some());
        assert_eq!(loaded.get_default_id(), Some(config_id));
    }

    #[test]
    fn test_store_load_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.json");

        let (store, warning) = ClaudeConfigStore::load_from_with_status(&path);
        assert!(warning.is_none());
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn test_store_load_corrupted_backs_up_and_starts_fresh() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("claude_configs.json");
        std::fs::write(&path, "{ invalid").unwrap();

        let (store, warning) = ClaudeConfigStore::load_from_with_status(&path);

        assert_eq!(store.count(), 0);
        assert!(warning.is_some(), "corruption should surface a warning");
        assert!(!path.exists(), "corrupted file should be renamed away");

        let backups: Vec<_> = std::fs::read_dir(temp_dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("json.corrupt."))
            .collect();
        assert_eq!(backups.len(), 1, "expected exactly one timestamped backup");
    }

    #[test]
    fn test_store_load_with_stale_default_recovers_deterministically() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("claude_configs.json");

        let id_a = uuid::Uuid::new_v4();
        let id_b = uuid::Uuid::new_v4();
        let stale_default = uuid::Uuid::new_v4();

        let file = serde_json::json!({
            "configs": [
                { "id": id_b, "name": "Zulu", "config_dir": "/tmp/zulu" },
                { "id": id_a, "name": "Alpha", "config_dir": "/tmp/alpha" }
            ],
            "default_config_id": stale_default
        });
        std::fs::write(&path, serde_json::to_string_pretty(&file).unwrap()).unwrap();

        let (store, warning) = ClaudeConfigStore::load_from_with_status(&path);
        assert!(warning.is_none());
        assert_eq!(store.count(), 2);
        // Deterministic fallback picks alphabetically first config.
        assert_eq!(store.get_default_id(), Some(id_a));
    }

    #[test]
    fn test_store_load_without_default_sets_fallback_when_configs_exist() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("claude_configs.json");

        let id_a = uuid::Uuid::new_v4();
        let id_b = uuid::Uuid::new_v4();
        let file = serde_json::json!({
            "configs": [
                { "id": id_b, "name": "Zulu", "config_dir": "/tmp/zulu" },
                { "id": id_a, "name": "Alpha", "config_dir": "/tmp/alpha" }
            ]
        });
        std::fs::write(&path, serde_json::to_string_pretty(&file).unwrap()).unwrap();

        let (store, warning) = ClaudeConfigStore::load_from_with_status(&path);
        assert!(warning.is_none());
        assert_eq!(store.get_default_id(), Some(id_a));
    }

    /// The alias must keep accepting the ID type alias downstream code uses.
    #[test]
    fn test_id_type_alias_compatibility() {
        let mut store = ClaudeConfigStore::new();
        let config = ClaudeConfig::new("Work".to_string(), None);
        let id: ClaudeConfigId = config.id;
        store.add(config);
        assert!(store.get(id).is_some());
    }
}
