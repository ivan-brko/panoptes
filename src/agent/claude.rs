//! Claude Code adapter implementation
//!
//! This module implements the `AgentAdapter` trait for Claude Code CLI.
//! It handles hook script installation, session settings configuration,
//! and process spawning.

use crate::config::Config;
use crate::session::PtyHandle;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::adapter::{AgentAdapter, SpawnConfig, SpawnResult};

/// Base hook script filename (shared across all sessions)
const HOOK_SCRIPT_NAME: &str = "panoptes-hook.sh";

/// Hook event types that Claude Code supports
const HOOK_EVENTS: &[&str] = &["PreToolUse", "PostToolUse", "Stop", "Notification"];

/// Claude Code adapter for spawning and managing Claude Code sessions
pub struct ClaudeCodeAdapter {
    /// Additional command-line arguments
    extra_args: Vec<String>,
}

impl ClaudeCodeAdapter {
    /// Create a new Claude Code adapter with default settings
    pub fn new() -> Self {
        Self {
            extra_args: Vec::new(),
        }
    }

    /// Create a new Claude Code adapter with additional arguments
    pub fn with_args(args: Vec<String>) -> Self {
        Self { extra_args: args }
    }

    /// Get the path to the shared hook script
    fn hook_script_path(config: &Config) -> PathBuf {
        config.hooks_dir.join(HOOK_SCRIPT_NAME)
    }

    /// Install the shared hook script and create symlinks for each event type
    fn install_hook_script(config: &Config) -> Result<HashMap<String, PathBuf>> {
        let script_path = Self::hook_script_path(config);

        // Ensure hooks directory exists
        std::fs::create_dir_all(&config.hooks_dir).context("Failed to create hooks directory")?;

        // Generate the hook script content
        let script_content = Self::generate_hook_script(config.hook_port);

        // Write the script
        std::fs::write(&script_path, &script_content).context("Failed to write hook script")?;

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

        // Create symlinks for each event type so basename $0 returns the event name
        let mut event_scripts = HashMap::new();
        for event in HOOK_EVENTS {
            let symlink_path = config.hooks_dir.join(format!("{}.sh", event));

            // Remove existing symlink if present
            if symlink_path.exists() || symlink_path.is_symlink() {
                let _ = std::fs::remove_file(&symlink_path);
            }

            // Create symlink
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&script_path, &symlink_path)
                    .with_context(|| format!("Failed to create symlink for {}", event))?;
            }

            event_scripts.insert(event.to_string(), symlink_path);
        }

        Ok(event_scripts)
    }

    /// Generate the hook script content
    fn generate_hook_script(port: u16) -> String {
        format!(
            r#"#!/bin/bash
# Panoptes hook script for Claude Code
# This script receives JSON from Claude Code on stdin and forwards relevant events to Panoptes

# Read session ID from environment
SESSION_ID="${{PANOPTES_SESSION_ID:-unknown}}"

# Read JSON from stdin
read -r json_input

# Extract event type (the script name indicates the hook type)
hook_name="$(basename "$0" .sh)"

# Extract tool_name if present (for tool-related hooks)
tool_name=""
if command -v jq &> /dev/null; then
    tool_name=$(echo "$json_input" | jq -r '.tool_name // .tool // empty' 2>/dev/null || echo "")
fi

# Get current timestamp
timestamp=$(date +%s)

# Build the payload
payload=$(cat <<EOF
{{"session_id": "$SESSION_ID", "event": "$hook_name", "tool": "$tool_name", "timestamp": $timestamp}}
EOF
)

# Send to Panoptes hook server (fire and forget, don't block Claude Code)
curl -s -X POST "http://127.0.0.1:{port}/hook" \
    -H "Content-Type: application/json" \
    -d "$payload" \
    --connect-timeout 1 \
    --max-time 2 \
    > /dev/null 2>&1 &

# Always exit successfully so we don't block Claude Code
exit 0
"#
        )
    }

    /// Create the session-specific settings file
    fn create_session_settings(
        working_dir: &Path,
        event_scripts: &HashMap<String, PathBuf>,
    ) -> Result<PathBuf> {
        // Create .claude directory in the working directory
        let claude_dir = working_dir.join(".claude");
        std::fs::create_dir_all(&claude_dir).context("Failed to create .claude directory")?;

        let settings_path = claude_dir.join("settings.local.json");

        // Build hooks config using the event-specific script paths
        let mut hooks = serde_json::Map::new();
        for event in HOOK_EVENTS {
            if let Some(script_path) = event_scripts.get(*event) {
                let script_path_str = script_path.to_string_lossy().to_string();
                hooks.insert(
                    event.to_string(),
                    serde_json::json!([
                        {
                            "matcher": ".*",
                            "hooks": [{"type": "command", "command": script_path_str}]
                        }
                    ]),
                );
            }
        }

        let settings = serde_json::json!({ "hooks": hooks });

        std::fs::write(
            &settings_path,
            serde_json::to_string_pretty(&settings).context("Failed to serialize settings")?,
        )
        .context("Failed to write settings file")?;

        Ok(settings_path)
    }
}

impl Default for ClaudeCodeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentAdapter for ClaudeCodeAdapter {
    fn name(&self) -> &str {
        "Claude Code"
    }

    fn command(&self) -> &str {
        "claude"
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
        env
    }

    fn setup_hooks(&self, config: &Config, spawn_config: &SpawnConfig) -> Result<Vec<PathBuf>> {
        let mut cleanup_paths = Vec::new();

        // Install shared hook script and create event-specific symlinks
        let event_scripts = Self::install_hook_script(config)?;
        // Note: We don't add the shared scripts to cleanup_paths since they're reused

        // Create session-specific settings file
        let settings_path =
            Self::create_session_settings(&spawn_config.working_dir, &event_scripts)?;
        cleanup_paths.push(settings_path);

        Ok(cleanup_paths)
    }

    fn spawn(&self, config: &Config, spawn_config: &SpawnConfig) -> Result<SpawnResult> {
        // Setup hooks first
        let _cleanup_paths = self.setup_hooks(config, spawn_config)?;

        // Generate environment
        let env = self.generate_env(config, spawn_config);

        // Build arguments
        let mut args = self.default_args();
        if let Some(ref prompt) = spawn_config.initial_prompt {
            args.push("--print".to_string());
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

    fn test_spawn_config(working_dir: PathBuf) -> SpawnConfig {
        SpawnConfig {
            session_id: Uuid::new_v4(),
            session_name: "test-session".to_string(),
            working_dir,
            initial_prompt: None,
            rows: 24,
            cols: 80,
        }
    }

    #[test]
    fn test_claude_adapter_name() {
        let adapter = ClaudeCodeAdapter::new();
        assert_eq!(adapter.name(), "Claude Code");
    }

    #[test]
    fn test_claude_adapter_command() {
        let adapter = ClaudeCodeAdapter::new();
        assert_eq!(adapter.command(), "claude");
    }

    #[test]
    fn test_claude_adapter_supports_hooks() {
        let adapter = ClaudeCodeAdapter::new();
        assert!(adapter.supports_hooks());
    }

    #[test]
    fn test_claude_adapter_default_args() {
        let adapter = ClaudeCodeAdapter::new();
        let args = adapter.default_args();
        assert!(args.is_empty());
    }

    #[test]
    fn test_claude_adapter_with_extra_args() {
        let adapter = ClaudeCodeAdapter::with_args(vec!["--verbose".to_string()]);
        let args = adapter.default_args();
        assert_eq!(args, vec!["--verbose".to_string()]);
    }

    #[test]
    fn test_generate_env_contains_session_id() {
        let adapter = ClaudeCodeAdapter::new();
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            hook_port: 9999,
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            max_output_lines: 1000,
            idle_threshold_secs: 300,
            state_timeout_secs: 300,
            exited_retention_secs: 300,
            theme_preset: "dark".to_string(),
            notification_method: "bell".to_string(),
        };
        let session_id = Uuid::new_v4();
        let spawn_config = SpawnConfig {
            session_id,
            session_name: "test".to_string(),
            working_dir: temp_dir.path().to_path_buf(),
            initial_prompt: None,
            rows: 24,
            cols: 80,
        };

        let env = adapter.generate_env(&config, &spawn_config);
        assert_eq!(
            env.get("PANOPTES_SESSION_ID"),
            Some(&session_id.to_string())
        );
    }

    #[test]
    fn test_generate_hook_script_content() {
        let script = ClaudeCodeAdapter::generate_hook_script(9999);
        assert!(script.contains("#!/bin/bash"));
        assert!(script.contains("PANOPTES_SESSION_ID"));
        assert!(script.contains("http://127.0.0.1:9999/hook"));
        assert!(script.contains("curl"));
    }

    #[test]
    fn test_install_hook_script() {
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            hook_port: 9999,
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            max_output_lines: 1000,
            idle_threshold_secs: 300,
            state_timeout_secs: 300,
            exited_retention_secs: 300,
            theme_preset: "dark".to_string(),
            notification_method: "bell".to_string(),
        };

        let event_scripts = ClaudeCodeAdapter::install_hook_script(&config).unwrap();

        // Verify base script was created
        let base_script = config.hooks_dir.join(HOOK_SCRIPT_NAME);
        assert!(base_script.exists());

        // Verify symlinks were created for each event type
        for event in HOOK_EVENTS {
            let symlink = event_scripts.get(*event).expect("Should have event script");
            assert!(symlink.exists() || symlink.is_symlink());
            assert!(symlink.ends_with(format!("{}.sh", event)));
        }

        // Verify base script is executable on Unix
        #[cfg(unix)]
        {
            let metadata = std::fs::metadata(&base_script).unwrap();
            let permissions = metadata.permissions();
            assert!(
                permissions.mode() & 0o111 != 0,
                "Script should be executable"
            );
        }
    }

    #[test]
    fn test_hooks_settings_json_structure() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path().to_path_buf();

        // Create mock event scripts HashMap
        let mut event_scripts = HashMap::new();
        for event in HOOK_EVENTS {
            event_scripts.insert(
                event.to_string(),
                PathBuf::from(format!("/test/{}.sh", event)),
            );
        }

        let settings_path =
            ClaudeCodeAdapter::create_session_settings(&working_dir, &event_scripts).unwrap();

        // Verify settings file was created
        assert!(settings_path.exists());

        // Read and parse the JSON
        let content = std::fs::read_to_string(&settings_path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Verify structure
        let hooks = settings.get("hooks").expect("Should have hooks key");
        assert!(hooks.get("PreToolUse").is_some());
        assert!(hooks.get("PostToolUse").is_some());
        assert!(hooks.get("Notification").is_some());
        assert!(hooks.get("Stop").is_some());
    }

    #[test]
    fn test_create_session_settings() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path().to_path_buf();

        // Create mock event scripts HashMap
        let mut event_scripts = HashMap::new();
        for event in HOOK_EVENTS {
            event_scripts.insert(
                event.to_string(),
                PathBuf::from(format!("/test/{}.sh", event)),
            );
        }

        let settings_path =
            ClaudeCodeAdapter::create_session_settings(&working_dir, &event_scripts).unwrap();

        // Verify file location
        assert_eq!(
            settings_path,
            working_dir.join(".claude/settings.local.json")
        );
        assert!(settings_path.exists());

        // Verify .claude directory was created
        assert!(working_dir.join(".claude").is_dir());
    }

    #[test]
    fn test_setup_hooks_returns_cleanup_paths() {
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            hook_port: 9999,
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            max_output_lines: 1000,
            idle_threshold_secs: 300,
            state_timeout_secs: 300,
            exited_retention_secs: 300,
            theme_preset: "dark".to_string(),
            notification_method: "bell".to_string(),
        };
        let spawn_config = test_spawn_config(temp_dir.path().to_path_buf());

        let adapter = ClaudeCodeAdapter::new();
        let cleanup_paths = adapter.setup_hooks(&config, &spawn_config).unwrap();

        // Should return the settings file path for cleanup
        assert!(!cleanup_paths.is_empty());
        assert!(cleanup_paths[0].ends_with("settings.local.json"));
        assert!(cleanup_paths[0].exists());
    }
}
