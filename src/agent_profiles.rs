//! Generic store for agent account profiles
//!
//! Claude Code and Codex both support multiple accounts, each pointing at its
//! own home directory (`CLAUDE_CONFIG_DIR` / `CODEX_HOME`). The bookkeeping is
//! identical - a set of named profiles, one of which is the default -
//! so [`ProfileStore`] implements it once, generically. `ClaudeConfigStore`
//! and `CodexConfigStore` are aliases of it; the [`AgentProfile`] trait
//! supplies the per-agent constants (store filename, display label) and field
//! accessors, keeping the JSON on disk byte-compatible with what each agent
//! module always wrote.

use crate::config::config_dir;
use crate::persistence::{self, LoadOutcome};
use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// An agent account profile that a [`ProfileStore`] can persist
pub trait AgentProfile: Clone + std::fmt::Debug + Serialize + DeserializeOwned {
    /// File name of the store inside `~/.panoptes/` (e.g. `claude_configs.json`)
    const STORE_FILENAME: &'static str;
    /// Human-readable label used in log and error messages (e.g. "claude configs")
    const LABEL: &'static str;

    /// Unique identifier of this profile
    fn id(&self) -> Uuid;
    /// Display name of this profile
    fn name(&self) -> &str;
    /// The agent home directory this profile points at; `None` = agent default
    fn home_dir(&self) -> Option<&Path>;
    /// Display text for the home directory (e.g. "~/.claude (default)")
    fn home_dir_display(&self) -> String;
    /// Create a new profile with a fresh ID
    fn new_profile(name: String, home_dir: Option<PathBuf>) -> Self;
}

/// Serializable format for the profile store
///
/// Field names are part of the on-disk JSON schema shared by
/// `claude_configs.json` and `codex_configs.json` - do not rename.
#[derive(Debug, Serialize, Deserialize)]
#[serde(bound(serialize = "C: Serialize", deserialize = "C: DeserializeOwned"))]
struct StoreData<C> {
    configs: Vec<C>,
    #[serde(default)]
    default_config_id: Option<Uuid>,
}

/// Store for persisting agent account profiles
#[derive(Debug)]
pub struct ProfileStore<C: AgentProfile> {
    /// All configurations indexed by ID
    configs: HashMap<Uuid, C>,
    /// ID of the default configuration
    default_config_id: Option<Uuid>,
    /// Path to the store file
    store_path: PathBuf,
}

impl<C: AgentProfile> Default for ProfileStore<C> {
    fn default() -> Self {
        Self {
            configs: HashMap::new(),
            default_config_id: None,
            store_path: store_file_path::<C>(),
        }
    }
}

/// Get the path to the store file for a profile type
pub fn store_file_path<C: AgentProfile>() -> PathBuf {
    config_dir().join(C::STORE_FILENAME)
}

impl<C: AgentProfile> ProfileStore<C> {
    /// Pick a replacement default deterministically: alphabetically first name
    ///
    /// `HashMap` iteration order would make the choice differ from run to run.
    fn deterministic_default_id(configs: &HashMap<Uuid, C>) -> Option<Uuid> {
        configs
            .values()
            .min_by(|a, b| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
            .map(|c| c.id())
    }

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
    pub fn configs(&self) -> impl Iterator<Item = &C> {
        self.configs.values()
    }

    /// Get all configurations as a sorted vector (by name)
    pub fn configs_sorted(&self) -> Vec<&C> {
        let mut configs: Vec<_> = self.configs.values().collect();
        configs.sort_by_key(|a| a.name().to_lowercase());
        configs
    }

    /// Get a configuration by ID
    pub fn get(&self, id: Uuid) -> Option<&C> {
        self.configs.get(&id)
    }

    /// Get a mutable reference to a configuration by ID
    pub fn get_mut(&mut self, id: Uuid) -> Option<&mut C> {
        self.configs.get_mut(&id)
    }

    /// Find a configuration by name
    pub fn find_by_name(&self, name: &str) -> Option<&C> {
        self.configs.values().find(|c| c.name() == name)
    }

    /// Check if a home directory is already used by another config
    ///
    /// `None` means the agent's default directory, so `None` vs `None` counts
    /// as a clash too.
    pub fn is_home_dir_used(&self, path: Option<&Path>) -> bool {
        self.configs.values().any(|c| c.home_dir() == path)
    }

    /// Add a configuration to the store
    pub fn add(&mut self, config: C) {
        let id = config.id();
        self.configs.insert(id, config);

        // If this is the first config, make it the default
        if self.configs.len() == 1 {
            self.default_config_id = Some(id);
        }
    }

    /// Remove a configuration
    ///
    /// If the removed configuration was the default, the alphabetically first
    /// remaining configuration becomes the new default.
    pub fn remove(&mut self, id: Uuid) -> Option<C> {
        let config = self.configs.remove(&id);

        // If we removed the default, pick a new default deterministically
        if self.default_config_id == Some(id) {
            self.default_config_id = Self::deterministic_default_id(&self.configs);
        }

        config
    }

    /// Get the default configuration
    pub fn get_default(&self) -> Option<&C> {
        self.default_config_id.and_then(|id| self.configs.get(&id))
    }

    /// Get the default configuration ID
    pub fn get_default_id(&self) -> Option<Uuid> {
        self.default_config_id
    }

    /// Set the default configuration
    pub fn set_default(&mut self, id: Uuid) -> bool {
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

    /// Load store from disk, returning a warning message if data was corrupted
    ///
    /// A corrupted profile store must never prevent startup - the corrupt file
    /// is backed up and the store starts empty.
    pub fn load_with_status() -> (Self, Option<String>) {
        Self::load_from_with_status(&store_file_path::<C>())
    }

    /// Load store from a specific path, returning a warning message if data was corrupted
    pub fn load_from_with_status(path: &Path) -> (Self, Option<String>) {
        match persistence::load_json::<StoreData<C>>(path, C::LABEL) {
            LoadOutcome::Absent => (Self::with_path(path.to_path_buf()), None),
            LoadOutcome::Loaded(data) => (Self::from_data(data, path), None),
            LoadOutcome::Corrupted { fallback_warning } => {
                (Self::with_path(path.to_path_buf()), Some(fallback_warning))
            }
        }
    }

    /// Build a store from parsed data, validating the default pointer
    ///
    /// A `default_config_id` naming a deleted config (a stale hand-edit or a
    /// partial write from an old version) falls back deterministically instead
    /// of silently leaving no default.
    fn from_data(data: StoreData<C>, path: &Path) -> Self {
        let configs: HashMap<_, _> = data.configs.into_iter().map(|c| (c.id(), c)).collect();
        let mut default_config_id = data.default_config_id.filter(|id| configs.contains_key(id));

        if data.default_config_id.is_some() && default_config_id.is_none() {
            tracing::warn!(
                "The {} store default_config_id points to a missing config; selecting a fallback default",
                C::LABEL
            );
        }

        if default_config_id.is_none() && !configs.is_empty() {
            default_config_id = Self::deterministic_default_id(&configs);
        }

        Self {
            configs,
            default_config_id,
            store_path: path.to_path_buf(),
        }
    }

    /// Save store to disk
    pub fn save(&self) -> Result<()> {
        self.save_to(&self.store_path)
    }

    /// Save store to a specific path (atomically, via a sibling temp file)
    pub fn save_to(&self, path: &Path) -> Result<()> {
        let data = StoreData {
            configs: self.configs.values().cloned().collect::<Vec<_>>(),
            default_config_id: self.default_config_id,
        };
        persistence::save_json_atomic(path, &data, C::LABEL)
    }
}
