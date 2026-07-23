//! Agent adapter trait and associated types
//!
//! This module defines the extensible abstraction for AI coding agents.
//! Each agent type implements the `AgentAdapter` trait to provide
//! consistent spawning and hook configuration.

use crate::config::Config;
use crate::session::{PtyHandle, SessionId};
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

/// Configuration for spawning an agent session
#[derive(Debug, Clone)]
pub struct SpawnConfig {
    /// Unique session identifier
    pub session_id: SessionId,
    /// User-friendly session name
    pub session_name: String,
    /// Working directory for the agent
    pub working_dir: PathBuf,
    /// Optional initial prompt to send to the agent
    pub initial_prompt: Option<String>,
    /// Terminal rows
    pub rows: u16,
    /// Terminal columns
    pub cols: u16,
    /// Claude config directory (CLAUDE_CONFIG_DIR). None = use default ~/.claude
    pub claude_config_dir: Option<PathBuf>,
    /// Codex home directory (CODEX_HOME). None = use default ~/.codex
    pub codex_home: Option<PathBuf>,
    /// Agent conversation ID to resume, when relaunching a session recovered
    /// from a previous Panoptes run. `None` starts a fresh conversation.
    pub resume: Option<String>,
}

/// Result of spawning an agent
pub struct SpawnResult {
    /// PTY handle for I/O with the agent
    pub pty: PtyHandle,
    /// Agent-specific session ID (if the agent provides one)
    pub agent_session_id: Option<String>,
}

/// Trait for agent adapters
///
/// This trait provides an abstraction layer for different AI coding agents,
/// allowing Panoptes to manage multiple agent types uniformly. Each implementation
/// handles agent-specific details like hook configuration and environment setup.
///
/// # Object Safety
/// This trait is object-safe to allow `Box<dyn AgentAdapter>` usage.
pub trait AgentAdapter: Send + Sync {
    /// Get the display name of this agent
    fn name(&self) -> &str;

    /// Get the command used to invoke this agent
    fn command(&self) -> &str;

    /// Get default command-line arguments
    fn default_args(&self) -> Vec<String>;

    /// Check if this agent supports hooks for state tracking
    fn supports_hooks(&self) -> bool;

    /// Generate environment variables for the agent process
    fn generate_env(&self, config: &Config, spawn_config: &SpawnConfig) -> HashMap<String, String>;

    /// Set up hooks for state tracking
    ///
    /// This method creates any necessary files (hook scripts, settings files)
    /// needed for the agent to report its state back to Panoptes.
    ///
    /// # Returns
    /// A list of paths that were created and should be cleaned up when the session ends.
    fn setup_hooks(&self, config: &Config, spawn_config: &SpawnConfig) -> Result<Vec<PathBuf>>;

    /// Build the complete argument list for a spawn
    ///
    /// The one genuinely agent-specific part of spawning: how a resume, an
    /// initial prompt and the default arguments combine into a command line.
    fn build_args(&self, spawn_config: &SpawnConfig) -> Vec<String>;

    /// The agent-native conversation ID this spawn will use, if known upfront
    ///
    /// `Some` only when the agent lets Panoptes dictate the ID (Claude, via
    /// `--session-id`). Agents that mint their own (Codex) return `None` and
    /// have it discovered later.
    fn agent_session_id(&self, spawn_config: &SpawnConfig) -> Option<String>;

    /// Spawn the agent in a PTY
    ///
    /// The lifecycle is the same for every agent — set up hooks, build the
    /// environment and arguments, hand them to the PTY — so it lives here
    /// once; adapters supply only the varying parts above.
    fn spawn(&self, config: &Config, spawn_config: &SpawnConfig) -> Result<SpawnResult> {
        // Setup hooks first
        let _cleanup_paths = self.setup_hooks(config, spawn_config)?;

        let env = self.generate_env(config, spawn_config);
        let args = self.build_args(spawn_config);
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        // Spawn the process with correct terminal dimensions
        let pty = PtyHandle::spawn(
            self.command(),
            &args_refs,
            &spawn_config.working_dir,
            env,
            spawn_config.rows,
            spawn_config.cols,
        )?;

        Ok(SpawnResult {
            pty,
            agent_session_id: self.agent_session_id(spawn_config),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper struct for testing trait bounds
    struct MockAdapter;

    impl AgentAdapter for MockAdapter {
        fn name(&self) -> &str {
            "Mock"
        }

        fn command(&self) -> &str {
            "mock"
        }

        fn default_args(&self) -> Vec<String> {
            vec!["--test".to_string()]
        }

        fn supports_hooks(&self) -> bool {
            false
        }

        fn generate_env(
            &self,
            _config: &Config,
            _spawn_config: &SpawnConfig,
        ) -> HashMap<String, String> {
            HashMap::new()
        }

        fn setup_hooks(
            &self,
            _config: &Config,
            _spawn_config: &SpawnConfig,
        ) -> Result<Vec<PathBuf>> {
            Ok(vec![])
        }

        fn build_args(&self, _spawn_config: &SpawnConfig) -> Vec<String> {
            self.default_args()
        }

        fn agent_session_id(&self, _spawn_config: &SpawnConfig) -> Option<String> {
            None
        }

        fn spawn(&self, _config: &Config, _spawn_config: &SpawnConfig) -> Result<SpawnResult> {
            anyhow::bail!("Mock adapter does not spawn")
        }
    }

    #[test]
    fn test_adapter_is_object_safe() {
        // Verify the trait is object-safe by creating a Box<dyn AgentAdapter>
        let adapter: Box<dyn AgentAdapter> = Box::new(MockAdapter);
        assert_eq!(adapter.name(), "Mock");
        assert_eq!(adapter.command(), "mock");
        assert!(!adapter.supports_hooks());
    }

    #[test]
    fn test_spawn_config_creation() {
        let config = SpawnConfig {
            session_id: uuid::Uuid::new_v4(),
            session_name: "test-session".to_string(),
            working_dir: PathBuf::from("/tmp"),
            initial_prompt: Some("hello".to_string()),
            rows: 24,
            cols: 80,
            claude_config_dir: None,
            codex_home: None,
            resume: None,
        };
        assert_eq!(config.session_name, "test-session");
        assert_eq!(config.initial_prompt, Some("hello".to_string()));
    }

    #[test]
    fn test_spawn_config_with_claude_config_dir() {
        let config = SpawnConfig {
            session_id: uuid::Uuid::new_v4(),
            session_name: "work-session".to_string(),
            working_dir: PathBuf::from("/tmp"),
            initial_prompt: None,
            rows: 24,
            cols: 80,
            claude_config_dir: Some(PathBuf::from("/home/user/.claude-work")),
            codex_home: None,
            resume: None,
        };
        assert_eq!(
            config.claude_config_dir,
            Some(PathBuf::from("/home/user/.claude-work"))
        );
    }
}
