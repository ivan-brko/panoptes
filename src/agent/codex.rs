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
/// Disable Codex alternate screen so Panoptes scrollback behaves like Claude sessions.
const NO_ALT_SCREEN_FLAG: &str = "--no-alt-screen";

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

    /// Create a new Codex adapter with additional arguments
    pub fn with_args(args: Vec<String>) -> Self {
        Self { extra_args: args }
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
#
# CRITICAL: Do NOT use blocking stdin reads (e.g. `read -r`) in this script.
# Codex executes notify hooks synchronously and pipes event JSON to stdin.
# A blocking read stalls Codex's output pipeline, causing typed characters
# to be dropped during streaming. If stdin data is needed in the future,
# it MUST be consumed non-blockingly (e.g. `cat > /dev/null &` to drain,
# or read in a backgrounded subshell).

SESSION_ID="${{PANOPTES_SESSION_ID:-}}"
if [ -z "$SESSION_ID" ]; then exit 0; fi

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
    /// Reads existing config.toml and configures the `notify` key.
    /// If user already has a notify hook, Panoptes chains to it instead of
    /// overwriting it.
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

        // Get or create the table
        let table = config
            .as_table_mut()
            .context("Codex config.toml root is not a table")?;

        let panoptes_notify_cmd = vec![
            "bash".to_string(),
            notify_script_path.to_string_lossy().to_string(),
        ];
        let panoptes_notify_value = Self::notify_array_value(&panoptes_notify_cmd);

        let mut did_modify = false;
        match table.get("notify") {
            None => {
                table.insert("notify".to_string(), panoptes_notify_value);
                did_modify = true;
            }
            Some(existing_notify) if *existing_notify == panoptes_notify_value => {
                // Already configured exactly as expected.
            }
            Some(existing_notify) => {
                if let Some(existing_cmd) = Self::parse_notify_command(existing_notify) {
                    if Self::notify_command_mentions_script(&existing_cmd, notify_script_path) {
                        // Already chained through Panoptes (or equivalent), avoid duplicate wrapping.
                    } else {
                        tracing::warn!(
                            "Codex config.toml already has 'notify' set. Chaining existing hook through Panoptes."
                        );
                        let chained_cmd =
                            Self::build_chained_notify_command(notify_script_path, &existing_cmd);
                        table.insert("notify".to_string(), Self::notify_array_value(&chained_cmd));
                        did_modify = true;
                    }
                } else {
                    let merge_script = Self::write_manual_merge_script(
                        codex_home,
                        existing_notify,
                        notify_script_path,
                    )?;
                    anyhow::bail!(
                        "Codex config.toml has an unsupported 'notify' value. \
                         Panoptes cannot install its hook safely. \
                         Merge Panoptes hook call into your existing notify hook using: {}",
                        merge_script.display()
                    );
                }
            }
        }

        if !did_modify {
            return Ok(());
        }

        // Create backup before modifying if file exists
        if config_path.exists() {
            let backup_path = config_path.with_extension("toml.panoptes.bak");
            if let Err(e) = std::fs::copy(&config_path, &backup_path) {
                tracing::warn!("Failed to create backup of codex config.toml: {}", e);
            }
        }

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

    fn notify_array_value(cmd: &[String]) -> toml::Value {
        toml::Value::Array(cmd.iter().cloned().map(toml::Value::String).collect())
    }

    fn parse_notify_command(value: &toml::Value) -> Option<Vec<String>> {
        match value {
            toml::Value::Array(parts) => {
                let mut cmd = Vec::with_capacity(parts.len());
                for part in parts {
                    cmd.push(part.as_str()?.to_string());
                }
                if cmd.is_empty() {
                    None
                } else {
                    Some(cmd)
                }
            }
            // Codex supports command strings in addition to argv arrays.
            toml::Value::String(shell_cmd) => Some(vec![
                "bash".to_string(),
                "-lc".to_string(),
                shell_cmd.clone(),
            ]),
            _ => None,
        }
    }

    fn notify_command_mentions_script(cmd: &[String], script_path: &Path) -> bool {
        let script = script_path.to_string_lossy();
        cmd.iter().any(|part| part.contains(script.as_ref()))
    }

    fn shell_quote(arg: &str) -> String {
        format!("'{}'", arg.replace('\'', r#"'\"'\"'"#))
    }

    fn build_chained_notify_command(
        notify_script_path: &Path,
        existing_notify_cmd: &[String],
    ) -> Vec<String> {
        let panoptes_hook = Self::shell_quote(&notify_script_path.to_string_lossy());
        let existing = existing_notify_cmd
            .iter()
            .map(|part| Self::shell_quote(part))
            .collect::<Vec<_>>()
            .join(" ");

        let script = format!("{panoptes_hook} \"$@\"; {existing}");
        vec!["bash".to_string(), "-lc".to_string(), script]
    }

    fn detect_existing_notify_hook_path(
        codex_home: &Path,
        notify_value: &toml::Value,
    ) -> Option<PathBuf> {
        let arr = notify_value.as_array()?;
        let strings: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
        if strings.len() < 2 {
            return None;
        }

        // Common form: ["bash", "/path/to/hook.sh", ...]
        let second = PathBuf::from(strings[1]);
        if second.is_absolute() {
            return Some(second);
        }

        // Treat relative paths as CODEX_HOME-relative for guidance output.
        Some(codex_home.join(second))
    }

    fn write_manual_merge_script(
        codex_home: &Path,
        notify_value: &toml::Value,
        notify_script_path: &Path,
    ) -> Result<PathBuf> {
        let existing_hook_path = Self::detect_existing_notify_hook_path(codex_home, notify_value);
        let target_dir = existing_hook_path
            .as_ref()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| codex_home.to_path_buf());
        std::fs::create_dir_all(&target_dir).with_context(|| {
            format!(
                "Failed to create merge script directory {}",
                target_dir.display()
            )
        })?;

        let merge_script_path = target_dir.join("panoptes-notify-merge.sh");
        let existing_display = existing_hook_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<your existing notify hook>".to_string());
        let panoptes_hook = notify_script_path.to_string_lossy();
        let content = format!(
            r#"#!/bin/bash
# Panoptes merge helper for Codex notify hooks.
# Merge the line below into your existing notify hook ({existing_display}):
#   "{panoptes_hook}" "$@"
#
# This helper is informational only and is not executed automatically.
"{panoptes_hook}" "$@" >/dev/null 2>&1 || true
"#
        );
        std::fs::write(&merge_script_path, content).with_context(|| {
            format!(
                "Failed to write merge helper {}",
                merge_script_path.display()
            )
        })?;

        #[cfg(unix)]
        {
            let mut perms = std::fs::metadata(&merge_script_path)
                .context("Failed to read merge helper metadata")?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&merge_script_path, perms)
                .context("Failed to set merge helper permissions")?;
        }

        Ok(merge_script_path)
    }

    /// Build Codex CLI args with Panoptes defaults.
    ///
    /// Panoptes runs Codex in inline mode (no alternate screen) so PTY scrollback
    /// remains usable from the session view, matching Claude behavior.
    fn build_args(&self, spawn_config: &SpawnConfig) -> Vec<String> {
        let mut args = self.default_args();

        if !args.iter().any(|arg| arg == NO_ALT_SCREEN_FLAG) {
            args.push(NO_ALT_SCREEN_FLAG.to_string());
        }

        if let Some(ref prompt) = spawn_config.initial_prompt {
            // Codex CLI takes initial prompt as a positional argument
            args.push(prompt.clone());
        }

        args
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
        // Set TERM for proper terminal emulation
        env.insert("TERM".to_string(), "xterm-256color".to_string());
        // Always set CODEX_HOME explicitly to keep runtime home and hook configuration aligned.
        let codex_home = Self::resolve_codex_home(spawn_config);
        env.insert(
            "CODEX_HOME".to_string(),
            codex_home.to_string_lossy().to_string(),
        );
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
        let args = self.build_args(spawn_config);

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
        // CODEX_HOME should always be set, even when no custom config is provided.
        assert_eq!(
            env.get("CODEX_HOME"),
            Some(
                &CodexAdapter::resolve_codex_home(&spawn_config)
                    .display()
                    .to_string()
            )
        );
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
    fn test_build_args_includes_no_alt_screen_by_default() {
        let adapter = CodexAdapter::new();
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

        let args = adapter.build_args(&spawn_config);
        assert!(args.iter().any(|arg| arg == NO_ALT_SCREEN_FLAG));
    }

    #[test]
    fn test_build_args_appends_prompt_after_flags() {
        let adapter = CodexAdapter::new();
        let prompt = "hello codex".to_string();
        let spawn_config = SpawnConfig {
            session_id: Uuid::new_v4(),
            session_name: "test".to_string(),
            working_dir: PathBuf::from("/tmp"),
            initial_prompt: Some(prompt.clone()),
            rows: 24,
            cols: 80,
            claude_config_dir: None,
            codex_home: None,
        };

        let args = adapter.build_args(&spawn_config);
        assert!(args.iter().any(|arg| arg == NO_ALT_SCREEN_FLAG));
        assert_eq!(args.last(), Some(&prompt));
    }

    #[test]
    fn test_build_args_preserves_existing_no_alt_screen_flag() {
        let adapter = CodexAdapter::with_args(vec![NO_ALT_SCREEN_FLAG.to_string()]);
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

        let args = adapter.build_args(&spawn_config);
        let count = args.iter().filter(|arg| *arg == NO_ALT_SCREEN_FLAG).count();
        assert_eq!(count, 1);
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
    fn test_configure_codex_notify_chains_existing_notify() {
        let temp_dir = TempDir::new().unwrap();
        let codex_home = temp_dir.path().join("codex-home");
        std::fs::create_dir_all(&codex_home).unwrap();

        let existing_config = r#"
model = "o3-mini"
notify = ["echo", "legacy-hook"]
"#;
        std::fs::write(codex_home.join("config.toml"), existing_config).unwrap();

        let notify_script = PathBuf::from("/test/codex-notify.sh");
        CodexAdapter::configure_codex_notify(&codex_home, &notify_script).unwrap();

        let content = std::fs::read_to_string(codex_home.join("config.toml")).unwrap();
        let config: toml::Value = toml::from_str(&content).unwrap();

        let notify = config.get("notify").unwrap().as_array().unwrap();
        assert_eq!(notify[0].as_str().unwrap(), "bash");
        assert_eq!(notify[1].as_str().unwrap(), "-lc");
        let script = notify[2].as_str().unwrap();
        assert!(script.contains("/test/codex-notify.sh"));
        assert!(script.contains("'echo' 'legacy-hook'"));

        // Existing settings should still be present.
        assert_eq!(config.get("model").unwrap().as_str().unwrap(), "o3-mini");
    }

    #[test]
    fn test_configure_codex_notify_rejects_unsupported_notify_shape() {
        let temp_dir = TempDir::new().unwrap();
        let codex_home = temp_dir.path().join("codex-home");
        std::fs::create_dir_all(&codex_home).unwrap();
        let legacy_hook_dir = temp_dir.path().join("legacy-hooks");
        std::fs::create_dir_all(&legacy_hook_dir).unwrap();
        let legacy_hook = legacy_hook_dir.join("notify.sh");
        std::fs::write(&legacy_hook, "#!/bin/bash\n").unwrap();

        let existing_config = r#"
model = "o3-mini"
notify = ["bash", "__LEGACY__", 42]
"#;
        let existing_config =
            existing_config.replace("__LEGACY__", legacy_hook.to_string_lossy().as_ref());
        std::fs::write(codex_home.join("config.toml"), existing_config).unwrap();

        let notify_script = PathBuf::from("/test/codex-notify.sh");
        let err = CodexAdapter::configure_codex_notify(&codex_home, &notify_script)
            .expect_err("unsupported notify shape should fail");
        assert!(err.to_string().contains("unsupported 'notify' value"));
        assert!(err.to_string().contains("panoptes-notify-merge.sh"));

        let content = std::fs::read_to_string(codex_home.join("config.toml")).unwrap();
        let config: toml::Value = toml::from_str(&content).unwrap();
        let notify = config.get("notify").unwrap().as_array().unwrap();
        assert_eq!(notify[0].as_str(), Some("bash"));
        assert_eq!(
            notify[1].as_str(),
            Some(legacy_hook.to_string_lossy().as_ref())
        );
        assert!(!codex_home.join("config.toml.panoptes.bak").exists());

        let merge_helper = legacy_hook_dir.join("panoptes-notify-merge.sh");
        assert!(merge_helper.exists());
        let helper_content = std::fs::read_to_string(merge_helper).unwrap();
        assert!(helper_content.contains("/test/codex-notify.sh"));
        assert!(helper_content.contains("notify.sh"));
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
