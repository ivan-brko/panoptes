//! OpenAI Codex CLI adapter implementation
//!
//! This module implements the `AgentAdapter` trait for OpenAI Codex CLI.
//! It handles notify hook configuration and process spawning.
//!
//! Codex CLI has limited hooks compared to Claude Code — only a `notify`
//! config that fires on `agent-turn-complete` events. This gives us the
//! critical "session needs attention" transition but no granular tool-use tracking.

use crate::config::Config;
use crate::session::PtyHandle;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::adapter::{AgentAdapter, SpawnConfig, SpawnResult};

/// Notify hook script filename
const CODEX_NOTIFY_SCRIPT_NAME: &str = "codex-notify.sh";

/// OpenAI Codex CLI adapter for spawning and managing Codex sessions
pub struct CodexAdapter {
    /// Additional command-line arguments
    extra_args: Vec<String>,
}

impl CodexAdapter {
    /// Create a new Codex adapter with default settings
    pub fn new() -> Self {
        Self {
            extra_args: Vec::new(),
        }
    }

    /// Get the path to the notify hook script
    fn notify_script_path(config: &Config) -> PathBuf {
        config.hooks_dir.join(CODEX_NOTIFY_SCRIPT_NAME)
    }

    /// Install the notify hook script
    fn install_notify_script(config: &Config) -> Result<PathBuf> {
        let script_path = Self::notify_script_path(config);

        // Ensure hooks directory exists
        std::fs::create_dir_all(&config.hooks_dir).context("Failed to create hooks directory")?;

        // Generate the hook script content
        let script_content = Self::generate_notify_script(config.hook_port);

        // Write the script
        std::fs::write(&script_path, &script_content)
            .context("Failed to write codex notify script")?;

        // Make executable (Unix only)
        #[cfg(unix)]
        {
            let mut perms = std::fs::metadata(&script_path)
                .context("Failed to get script metadata")?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms)
                .context("Failed to set script permissions")?;
        }

        Ok(script_path)
    }

    /// Generate the notify hook script content
    fn generate_notify_script(port: u16) -> String {
        format!(
            r#"#!/bin/bash
# Panoptes notify hook for OpenAI Codex CLI
# Silently exits for non-Panoptes Codex instances

SESSION_ID="${{PANOPTES_SESSION_ID:-}}"
if [ -z "$SESSION_ID" ]; then exit 0; fi

# Read JSON from stdin (Codex sends event data)
read -r json_input

timestamp=$(date +%s)

payload=$(cat <<EOF
{{"session_id": "$SESSION_ID", "event": "AgentTurnComplete", "tool": "", "timestamp": $timestamp}}
EOF
)

# Send to Panoptes hook server (fire and forget)
curl -s -X POST "http://127.0.0.1:{port}/hook" \
    -H "Content-Type: application/json" \
    -d "$payload" \
    --connect-timeout 1 \
    --max-time 2 \
    > /dev/null 2>&1 &

exit 0
"#
        )
    }

    /// Configure Codex's config.toml to use the notify hook
    ///
    /// Reads existing config.toml, merges the `notify` key, and writes back.
    /// Creates a backup before modifying.
    fn configure_codex_notify(codex_home: &Path, notify_script_path: &Path) -> Result<()> {
        // Ensure codex home directory exists
        std::fs::create_dir_all(codex_home).context("Failed to create CODEX_HOME directory")?;

        let config_path = codex_home.join("config.toml");

        // Read existing config or start fresh
        let mut config: toml::Value = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .context("Failed to read codex config.toml")?;
            toml::from_str(&content).unwrap_or_else(|e| {
                tracing::warn!(
                    "Failed to parse existing codex config.toml: {}, starting fresh",
                    e
                );
                toml::Value::Table(toml::map::Map::new())
            })
        } else {
            toml::Value::Table(toml::map::Map::new())
        };

        // Create backup before modifying if file exists
        if config_path.exists() {
            let backup_path = config_path.with_extension("toml.panoptes.bak");
            if let Err(e) = std::fs::copy(&config_path, &backup_path) {
                tracing::warn!("Failed to create backup of codex config.toml: {}", e);
            }
        }

        // Get or create the table
        let table = config
            .as_table_mut()
            .context("Codex config.toml root is not a table")?;

        // Check if notify is already set by user
        if table.contains_key("notify") {
            tracing::warn!(
                "Codex config.toml already has 'notify' set. Overwriting with Panoptes hook."
            );
        }

        // Set notify = ["bash", "/path/to/codex-notify.sh"]
        let notify_value = toml::Value::Array(vec![
            toml::Value::String("bash".to_string()),
            toml::Value::String(notify_script_path.to_string_lossy().to_string()),
        ]);
        table.insert("notify".to_string(), notify_value);

        // Write back
        let content =
            toml::to_string_pretty(&config).context("Failed to serialize codex config.toml")?;
        std::fs::write(&config_path, &content).context("Failed to write codex config.toml")?;

        Ok(())
    }

    /// Determine the CODEX_HOME directory
    fn resolve_codex_home(spawn_config: &SpawnConfig) -> PathBuf {
        spawn_config.codex_home.clone().unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".codex")
        })
    }
}

impl Default for CodexAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentAdapter for CodexAdapter {
    fn name(&self) -> &str {
        "Codex"
    }

    fn command(&self) -> &str {
        "codex"
    }

    fn default_args(&self) -> Vec<String> {
        self.extra_args.clone()
    }

    fn supports_hooks(&self) -> bool {
        true
    }

    fn generate_env(
        &self,
        _config: &Config,
        spawn_config: &SpawnConfig,
    ) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert(
            "PANOPTES_SESSION_ID".to_string(),
            spawn_config.session_id.to_string(),
        );
        // Set custom CODEX_HOME if specified
        if let Some(ref codex_home) = spawn_config.codex_home {
            env.insert(
                "CODEX_HOME".to_string(),
                codex_home.to_string_lossy().to_string(),
            );
        }
        env
    }

    fn setup_hooks(&self, config: &Config, spawn_config: &SpawnConfig) -> Result<Vec<PathBuf>> {
        // Install the notify hook script
        let notify_script_path = Self::install_notify_script(config)?;

        // Determine CODEX_HOME and configure notify in config.toml
        let codex_home = Self::resolve_codex_home(spawn_config);
        Self::configure_codex_notify(&codex_home, &notify_script_path)?;

        // TODO: Codex permission sharing
        // When Codex supports per-project permissions (similar to Claude's
        // .claude/settings.local.json), implement copying from root branch
        // to worktree here. See check_claude_settings_for_copy() in
        // src/wizards/worktree/handlers.rs for the Claude implementation.

        // Return the config.toml path for reference (we don't clean it up since
        // the notify hook is harmless for non-Panoptes instances)
        Ok(vec![])
    }

    fn spawn(&self, config: &Config, spawn_config: &SpawnConfig) -> Result<SpawnResult> {
        // Setup hooks first
        let _cleanup_paths = self.setup_hooks(config, spawn_config)?;

        // Generate environment
        let env = self.generate_env(config, spawn_config);

        // Build arguments
        let mut args = self.default_args();
        if let Some(ref prompt) = spawn_config.initial_prompt {
            // Codex CLI takes initial prompt as a positional argument
            args.push(prompt.clone());
        }

        // Convert args to &str for PtyHandle
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
            agent_session_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use uuid::Uuid;

    #[test]
    fn test_codex_adapter_name() {
        let adapter = CodexAdapter::new();
        assert_eq!(adapter.name(), "Codex");
    }

    #[test]
    fn test_codex_adapter_command() {
        let adapter = CodexAdapter::new();
        assert_eq!(adapter.command(), "codex");
    }

    #[test]
    fn test_codex_adapter_supports_hooks() {
        let adapter = CodexAdapter::new();
        assert!(adapter.supports_hooks());
    }

    #[test]
    fn test_codex_adapter_default_args() {
        let adapter = CodexAdapter::new();
        let args = adapter.default_args();
        assert!(args.is_empty());
    }

    #[test]
    fn test_generate_env_contains_session_id() {
        let adapter = CodexAdapter::new();
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            ..Config::default()
        };
        let session_id = Uuid::new_v4();
        let spawn_config = SpawnConfig {
            session_id,
            session_name: "test".to_string(),
            working_dir: temp_dir.path().to_path_buf(),
            initial_prompt: None,
            rows: 24,
            cols: 80,
            claude_config_dir: None,
            codex_home: None,
        };

        let env = adapter.generate_env(&config, &spawn_config);
        assert_eq!(
            env.get("PANOPTES_SESSION_ID"),
            Some(&session_id.to_string())
        );
        // No CODEX_HOME when not specified
        assert!(env.get("CODEX_HOME").is_none());
    }

    #[test]
    fn test_generate_env_with_codex_home() {
        let adapter = CodexAdapter::new();
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            ..Config::default()
        };
        let session_id = Uuid::new_v4();
        let codex_home_path = PathBuf::from("/home/user/.codex-work");
        let spawn_config = SpawnConfig {
            session_id,
            session_name: "test".to_string(),
            working_dir: temp_dir.path().to_path_buf(),
            initial_prompt: None,
            rows: 24,
            cols: 80,
            claude_config_dir: None,
            codex_home: Some(codex_home_path.clone()),
        };

        let env = adapter.generate_env(&config, &spawn_config);
        assert_eq!(
            env.get("CODEX_HOME"),
            Some(&codex_home_path.to_string_lossy().to_string())
        );
    }

    #[test]
    fn test_generate_notify_script_content() {
        let script = CodexAdapter::generate_notify_script(9999);
        assert!(script.contains("#!/bin/bash"));
        assert!(script.contains("PANOPTES_SESSION_ID"));
        assert!(script.contains("http://127.0.0.1:9999/hook"));
        assert!(script.contains("AgentTurnComplete"));
        assert!(script.contains("curl"));
        // Should silently exit for non-Panoptes instances
        assert!(script.contains("if [ -z \"$SESSION_ID\" ]; then exit 0; fi"));
    }

    #[test]
    fn test_install_notify_script() {
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            ..Config::default()
        };

        let script_path = CodexAdapter::install_notify_script(&config).unwrap();

        // Verify script was created
        assert!(script_path.exists());
        assert!(script_path.ends_with(CODEX_NOTIFY_SCRIPT_NAME));

        // Verify script is executable on Unix
        #[cfg(unix)]
        {
            let metadata = std::fs::metadata(&script_path).unwrap();
            let permissions = metadata.permissions();
            assert!(
                permissions.mode() & 0o111 != 0,
                "Script should be executable"
            );
        }
    }

    #[test]
    fn test_configure_codex_notify_fresh() {
        let temp_dir = TempDir::new().unwrap();
        let codex_home = temp_dir.path().join("codex-home");
        let notify_script = PathBuf::from("/test/codex-notify.sh");

        CodexAdapter::configure_codex_notify(&codex_home, &notify_script).unwrap();

        // Verify config.toml was created
        let config_path = codex_home.join("config.toml");
        assert!(config_path.exists());

        // Verify content
        let content = std::fs::read_to_string(&config_path).unwrap();
        let config: toml::Value = toml::from_str(&content).unwrap();
        let notify = config.get("notify").expect("Should have notify key");
        let notify_arr = notify.as_array().unwrap();
        assert_eq!(notify_arr[0].as_str().unwrap(), "bash");
        assert_eq!(notify_arr[1].as_str().unwrap(), "/test/codex-notify.sh");
    }

    #[test]
    fn test_configure_codex_notify_preserves_existing() {
        let temp_dir = TempDir::new().unwrap();
        let codex_home = temp_dir.path().join("codex-home");
        std::fs::create_dir_all(&codex_home).unwrap();

        // Create existing config
        let existing_config = r#"
model = "o3-mini"
approval_policy = "suggest"
"#;
        std::fs::write(codex_home.join("config.toml"), existing_config).unwrap();

        let notify_script = PathBuf::from("/test/codex-notify.sh");
        CodexAdapter::configure_codex_notify(&codex_home, &notify_script).unwrap();

        // Verify existing settings preserved
        let content = std::fs::read_to_string(codex_home.join("config.toml")).unwrap();
        let config: toml::Value = toml::from_str(&content).unwrap();
        assert_eq!(config.get("model").unwrap().as_str().unwrap(), "o3-mini");
        assert_eq!(
            config.get("approval_policy").unwrap().as_str().unwrap(),
            "suggest"
        );
        // Notify should also be present
        assert!(config.get("notify").is_some());

        // Verify backup was created
        assert!(codex_home.join("config.toml.panoptes.bak").exists());
    }

    #[test]
    fn test_setup_hooks() {
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            ..Config::default()
        };
        let codex_home = temp_dir.path().join("codex-home");
        let spawn_config = SpawnConfig {
            session_id: Uuid::new_v4(),
            session_name: "test".to_string(),
            working_dir: temp_dir.path().to_path_buf(),
            initial_prompt: None,
            rows: 24,
            cols: 80,
            claude_config_dir: None,
            codex_home: Some(codex_home.clone()),
        };

        let adapter = CodexAdapter::new();
        adapter.setup_hooks(&config, &spawn_config).unwrap();

        // Verify notify script exists
        let notify_script = config.hooks_dir.join(CODEX_NOTIFY_SCRIPT_NAME);
        assert!(notify_script.exists());

        // Verify config.toml was updated
        let config_toml_path = codex_home.join("config.toml");
        assert!(config_toml_path.exists());
    }

    #[test]
    fn test_resolve_codex_home_with_config() {
        let spawn_config = SpawnConfig {
            session_id: Uuid::new_v4(),
            session_name: "test".to_string(),
            working_dir: PathBuf::from("/tmp"),
            initial_prompt: None,
            rows: 24,
            cols: 80,
            claude_config_dir: None,
            codex_home: Some(PathBuf::from("/custom/codex")),
        };

        let resolved = CodexAdapter::resolve_codex_home(&spawn_config);
        assert_eq!(resolved, PathBuf::from("/custom/codex"));
    }

    #[test]
    fn test_resolve_codex_home_default() {
        let spawn_config = SpawnConfig {
            session_id: Uuid::new_v4(),
            session_name: "test".to_string(),
            working_dir: PathBuf::from("/tmp"),
            initial_prompt: None,
            rows: 24,
            cols: 80,
            claude_config_dir: None,
            codex_home: None,
        };

        let resolved = CodexAdapter::resolve_codex_home(&spawn_config);
        // Should resolve to ~/.codex
        assert!(resolved.ends_with(".codex"));
    }
}
