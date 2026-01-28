//! Shell adapter implementation
//!
//! This module implements the `AgentAdapter` trait for generic shell sessions.
//! Unlike Claude Code, shell sessions don't use hooks - instead they use
//! foreground process detection to track execution state.

use crate::config::Config;
use crate::session::PtyHandle;
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

use super::adapter::{AgentAdapter, SpawnConfig, SpawnResult};

/// Shell adapter for spawning generic bash/zsh sessions
pub struct ShellAdapter {
    /// Shell command to use (detected from $SHELL or fallback)
    shell_command: String,
}

impl ShellAdapter {
    /// Create a new shell adapter, detecting the user's shell
    pub fn new() -> Self {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        Self {
            shell_command: shell,
        }
    }

    /// Create a shell adapter with a specific shell command
    pub fn with_shell(shell: String) -> Self {
        Self {
            shell_command: shell,
        }
    }
}

impl Default for ShellAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentAdapter for ShellAdapter {
    fn name(&self) -> &str {
        "Shell"
    }

    fn command(&self) -> &str {
        &self.shell_command
    }

    fn default_args(&self) -> Vec<String> {
        // Start an interactive login shell
        vec!["-l".to_string()]
    }

    fn supports_hooks(&self) -> bool {
        false
    }

    fn generate_env(
        &self,
        _config: &Config,
        spawn_config: &SpawnConfig,
    ) -> HashMap<String, String> {
        let mut env = HashMap::new();

        // Set TERM for proper terminal emulation
        env.insert("TERM".to_string(), "xterm-256color".to_string());

        // Pass through session ID for identification (even though we don't use hooks)
        env.insert(
            "PANOPTES_SESSION_ID".to_string(),
            spawn_config.session_id.to_string(),
        );

        // Mark as Panoptes session for potential shell integration
        env.insert("PANOPTES_SESSION".to_string(), "1".to_string());

        env
    }

    fn setup_hooks(&self, _config: &Config, _spawn_config: &SpawnConfig) -> Result<Vec<PathBuf>> {
        // Shell sessions don't use hooks - state tracking is done via foreground detection
        Ok(vec![])
    }

    fn spawn(&self, config: &Config, spawn_config: &SpawnConfig) -> Result<SpawnResult> {
        // Generate environment
        let env = self.generate_env(config, spawn_config);

        // Build arguments
        let args = self.default_args();
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        // Spawn the shell process
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
            agent_session_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn test_spawn_config() -> SpawnConfig {
        SpawnConfig {
            session_id: Uuid::new_v4(),
            session_name: "test-shell".to_string(),
            working_dir: PathBuf::from("/tmp"),
            initial_prompt: None,
            rows: 24,
            cols: 80,
            claude_config_dir: None,
        }
    }

    #[test]
    fn test_shell_adapter_name() {
        let adapter = ShellAdapter::new();
        assert_eq!(adapter.name(), "Shell");
    }

    #[test]
    fn test_shell_adapter_command_from_env() {
        // The command should be whatever $SHELL is, or fallback
        let adapter = ShellAdapter::new();
        let command = adapter.command();
        assert!(!command.is_empty());
    }

    #[test]
    fn test_shell_adapter_with_explicit_shell() {
        let adapter = ShellAdapter::with_shell("/bin/zsh".to_string());
        assert_eq!(adapter.command(), "/bin/zsh");
    }

    #[test]
    fn test_shell_adapter_does_not_support_hooks() {
        let adapter = ShellAdapter::new();
        assert!(!adapter.supports_hooks());
    }

    #[test]
    fn test_shell_adapter_default_args() {
        let adapter = ShellAdapter::new();
        let args = adapter.default_args();
        assert!(args.contains(&"-l".to_string()));
    }

    #[test]
    fn test_shell_adapter_generate_env() {
        let adapter = ShellAdapter::new();
        let config = Config::default();
        let spawn_config = test_spawn_config();

        let env = adapter.generate_env(&config, &spawn_config);

        assert_eq!(env.get("TERM"), Some(&"xterm-256color".to_string()));
        assert!(env.get("PANOPTES_SESSION_ID").is_some());
        assert_eq!(env.get("PANOPTES_SESSION"), Some(&"1".to_string()));
    }

    #[test]
    fn test_shell_adapter_setup_hooks_returns_empty() {
        let adapter = ShellAdapter::new();
        let config = Config::default();
        let spawn_config = test_spawn_config();

        let paths = adapter.setup_hooks(&config, &spawn_config).unwrap();
        assert!(paths.is_empty());
    }
}
