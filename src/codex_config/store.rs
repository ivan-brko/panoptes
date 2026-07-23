//! Codex configuration persistence
//!
//! Handles saving and loading Codex configurations to/from disk. The
//! mechanism lives in [`crate::agent_profiles::ProfileStore`]; this module
//! binds it to [`CodexConfig`] and keeps the Codex-flavoured method names.

use super::CodexConfig;
use crate::agent_profiles::{AgentProfile, ProfileStore};
use std::path::Path;
use uuid::Uuid;

impl AgentProfile for CodexConfig {
    const STORE_FILENAME: &'static str = "codex_configs.json";
    const LABEL: &'static str = "codex configs";

    fn id(&self) -> Uuid {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn home_dir(&self) -> Option<&Path> {
        self.codex_home.as_deref()
    }

    fn home_dir_display(&self) -> String {
        self.codex_home_display()
    }

    fn new_profile(name: String, home_dir: Option<std::path::PathBuf>) -> Self {
        Self::new(name, home_dir)
    }
}

/// Store for persisting Codex configurations
pub type CodexConfigStore = ProfileStore<CodexConfig>;

impl CodexConfigStore {
    /// Check if a codex home directory is already used by another config
    pub fn is_codex_home_used(&self, path: Option<&Path>) -> bool {
        self.is_home_dir_used(path)
    }
}

#[cfg(test)]
mod tests {
    //! Codex-specific store tests
    //!
    //! `CodexConfigStore` is `ProfileStore<CodexConfig>`; the generic add /
    //! remove / default-selection / sorting / corruption-recovery behavior is
    //! covered once, in `claude_config::store` and `persistence`. The tests
    //! here cover only what is Codex-flavoured: the `codex_home` wire format,
    //! the renamed wrapper method, and the ID type alias.

    use super::*;
    use crate::codex_config::CodexConfigId;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn test_store_is_codex_home_used() {
        let mut store = CodexConfigStore::new();
        let config = CodexConfig::new("Work".to_string(), Some(PathBuf::from("/home/.codex-work")));

        store.add(config);

        assert!(store.is_codex_home_used(Some(Path::new("/home/.codex-work"))));
        assert!(!store.is_codex_home_used(Some(Path::new("/home/.codex-other"))));
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
        let (loaded, warning) = CodexConfigStore::load_from_with_status(&path);
        assert!(warning.is_none());
        assert_eq!(loaded.count(), 1);
        assert!(loaded.get(config_id).is_some());
        assert_eq!(loaded.get_default_id(), Some(config_id));
    }

    #[test]
    fn test_store_load_with_stale_default_recovers_deterministically() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("codex_configs.json");

        let id_a = uuid::Uuid::new_v4();
        let id_b = uuid::Uuid::new_v4();
        let stale_default = uuid::Uuid::new_v4();

        let file = serde_json::json!({
            "configs": [
                { "id": id_b, "name": "Zulu", "codex_home": "/tmp/zulu" },
                { "id": id_a, "name": "Alpha", "codex_home": "/tmp/alpha" }
            ],
            "default_config_id": stale_default
        });
        std::fs::write(&path, serde_json::to_string_pretty(&file).unwrap()).unwrap();

        let (store, warning) = CodexConfigStore::load_from_with_status(&path);
        assert!(warning.is_none());
        assert_eq!(store.count(), 2);
        // Deterministic fallback picks alphabetically first config.
        assert_eq!(store.get_default_id(), Some(id_a));
    }

    /// The alias must keep accepting the ID type alias downstream code uses.
    #[test]
    fn test_id_type_alias_compatibility() {
        let mut store = CodexConfigStore::new();
        let config = CodexConfig::new("Work".to_string(), None);
        let id: CodexConfigId = config.id;
        store.add(config);
        assert!(store.get(id).is_some());
    }
}
