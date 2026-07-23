//! OpenAI Codex CLI adapter implementation
//!
//! This module implements the `AgentAdapter` trait for OpenAI Codex CLI.
//! It handles notify hook configuration and process spawning.
//!
//! Codex CLI has limited hooks compared to Claude Code — only a `notify`
//! config that fires on `agent-turn-complete` events. This gives us the
//! critical "session needs attention" transition but no granular tool-use tracking.

use crate::config::Config;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::adapter::{AgentAdapter, SpawnConfig};
use crate::transcript::codex::{read_session_meta, rollout_files};

/// Notify hook script filename
const CODEX_NOTIFY_SCRIPT_NAME: &str = "codex-notify.sh";
/// Disable Codex alternate screen so Panoptes scrollback behaves like Claude sessions.
const NO_ALT_SCREEN_FLAG: &str = "--no-alt-screen";

/// Clock tolerance when matching a rollout against a session start time
///
/// Codex stamps the conversation a moment after Panoptes records the session as
/// created, so the rollout is normally the later of the two. This absorbs the
/// rounding and sub-second ordering that can invert them.
const ROLLOUT_TIME_TOLERANCE_SECS: i64 = 5;

/// Resolve a path for comparison, tolerating symlinks
///
/// Necessary on macOS, where `/tmp` is a symlink to `/private/tmp`: Codex
/// records the resolved path while Panoptes may hold the unresolved one, and a
/// textual comparison would never match.
fn canonical_or_original(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Find the Codex conversation ID for a session, by locating its rollout file
///
/// Unlike Claude Code, Codex offers no flag to dictate the session ID, so it has
/// to be discovered after the fact. Every rollout file begins with a
/// `session_meta` record carrying the session `id` and the `cwd` it started in,
/// which together are enough to match a rollout to a Panoptes session.
///
/// Returns `None` while Codex has not written the file yet, which is the normal
/// state for the first moments of a session - callers are expected to retry.
pub fn discover_session_id(
    codex_home: &Path,
    working_dir: &Path,
    started_at: chrono::DateTime<chrono::Utc>,
    claimed: &std::collections::HashSet<String>,
) -> Option<String> {
    let sessions_dir = codex_home.join("sessions");
    let target_cwd = canonical_or_original(working_dir);
    let cutoff = started_at - chrono::Duration::seconds(ROLLOUT_TIME_TOLERANCE_SECS);

    let mut candidates: Vec<(chrono::DateTime<chrono::Utc>, String)> = Vec::new();
    for path in rollout_files(&sessions_dir) {
        let Some(meta) = read_session_meta(&path) else {
            continue;
        };
        // A subagent gets its own rollout, in the same working directory and
        // with its own fresh timestamp, so it matches every other criterion
        // here and would be claimed as if it were the session's own
        // conversation. Resuming that pointer would reattach to a subagent
        // rather than the conversation the user was having.
        if meta.is_subagent {
            continue;
        }
        let same_cwd = meta
            .cwd
            .is_some_and(|cwd| canonical_or_original(&cwd) == target_cwd);
        if !same_cwd {
            continue;
        }
        // The rollout's own creation timestamp, not the file mtime: mtime is
        // bumped on every turn, so an older conversation being actively used
        // would otherwise look newer than the session we are trying to
        // identify. A rollout with no timestamp at all cannot be matched.
        let Some(created_at) = meta.created_at else {
            continue;
        };
        if created_at < cutoff {
            continue;
        }
        // Another Panoptes session already owns this conversation
        if claimed.contains(&meta.id) {
            continue;
        }
        candidates.push((created_at, meta.id));
    }

    // Oldest first. Callers resolve pending sessions in start order, so the
    // earliest unclaimed rollout created after this session started is its own.
    // Picking the newest would hand a session the rollout of a *later* session
    // started in the same directory.
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates.into_iter().next().map(|(_, id)| id)
}

/// Locate the rollout file holding a known Codex conversation
///
/// Needed to tail a conversation whose ID is already known, which is the
/// reverse of `discover_session_id`. Returns `None` before Codex has written
/// the file, which is normal for the first moments of a session.
pub fn rollout_path(codex_home: &Path, conversation_id: &str) -> Option<PathBuf> {
    rollout_files(&codex_home.join("sessions"))
        .into_iter()
        .find(|path| {
            // The filename embeds the conversation UUID, so most files can be
            // dismissed without opening them
            let names_it = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.contains(conversation_id));
            names_it && read_session_meta(path).is_some_and(|meta| meta.id == conversation_id)
        })
}

/// What installing the Panoptes notify hook into a Codex config requires
///
/// The pure outcome of [`CodexAdapter::plan_notify`], separated from the
/// filesystem work of acting on it.
#[derive(Debug, Clone, PartialEq)]
enum NotifyPlan {
    /// `notify` already routes through the Panoptes script — write nothing,
    /// back nothing up
    AlreadyConfigured,
    /// Write this as the new `notify` value (backing up the file first)
    Set(toml::Value),
    /// The existing `notify` value has a shape Panoptes cannot chain safely;
    /// the user has to merge by hand
    Unsupported,
}

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
        super::install_executable_script(
            &script_path,
            &Self::generate_notify_script(config.hook_port),
        )
        .context("Failed to install codex notify script")?;
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

    /// How to bring a Codex `notify` setting under Panoptes
    ///
    /// The pure policy behind [`Self::configure_codex_notify`]: given the
    /// existing `notify` value (if any), decide what — if anything — should be
    /// written, without touching the filesystem.
    fn plan_notify(existing: Option<&toml::Value>, script: &Path) -> NotifyPlan {
        let panoptes_notify_cmd = vec!["bash".to_string(), script.to_string_lossy().to_string()];
        let panoptes_notify_value = Self::notify_array_value(&panoptes_notify_cmd);

        match existing {
            None => NotifyPlan::Set(panoptes_notify_value),
            Some(existing) if *existing == panoptes_notify_value => {
                // Already configured exactly as expected.
                NotifyPlan::AlreadyConfigured
            }
            Some(existing) => {
                let Some(existing_cmd) = Self::parse_notify_command(existing) else {
                    return NotifyPlan::Unsupported;
                };
                if Self::notify_command_mentions_script(&existing_cmd, script) {
                    // Already chained through Panoptes (or equivalent), avoid
                    // duplicate wrapping — and avoid rewriting a file that
                    // needs no change.
                    NotifyPlan::AlreadyConfigured
                } else {
                    let chained_cmd = Self::build_chained_notify_command(script, &existing_cmd);
                    NotifyPlan::Set(Self::notify_array_value(&chained_cmd))
                }
            }
        }
    }

    /// Configure Codex's config.toml to use the notify hook
    ///
    /// Reads existing config.toml and configures the `notify` key.
    /// If user already has a notify hook, Panoptes chains to it instead of
    /// overwriting it. When the value is already correct, nothing is written
    /// and no backup is made.
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

        let existing = table.get("notify").cloned();
        match Self::plan_notify(existing.as_ref(), notify_script_path) {
            NotifyPlan::AlreadyConfigured => return Ok(()),
            NotifyPlan::Unsupported => {
                let existing =
                    existing.context("unsupported notify plan without an existing notify value")?;
                let merge_script =
                    Self::write_manual_merge_script(codex_home, &existing, notify_script_path)?;
                anyhow::bail!(
                    "Codex config.toml has an unsupported 'notify' value. \
                     Panoptes cannot install its hook safely. \
                     Merge Panoptes hook call into your existing notify hook using: {}",
                    merge_script.display()
                );
            }
            NotifyPlan::Set(value) => {
                if existing.is_some() {
                    tracing::warn!(
                        "Codex config.toml already has 'notify' set. Chaining existing hook through Panoptes."
                    );
                }
                table.insert("notify".to_string(), value);
            }
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
        super::install_executable_script(&merge_script_path, &content).with_context(|| {
            format!(
                "Failed to write merge helper {}",
                merge_script_path.display()
            )
        })?;

        Ok(merge_script_path)
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

    /// Build Codex CLI args with Panoptes defaults.
    ///
    /// Panoptes runs Codex in inline mode (no alternate screen) so PTY scrollback
    /// remains usable from the session view, matching Claude behavior.
    fn build_args(&self, spawn_config: &SpawnConfig) -> Vec<String> {
        let mut args = Vec::new();

        // Resuming is a subcommand, not a flag: `codex resume [OPTIONS]
        // [SESSION_ID] [PROMPT]`. It has to lead the argument list.
        if spawn_config.resume.is_some() {
            args.push("resume".to_string());
        }

        args.extend(self.default_args());

        if !args.iter().any(|arg| arg == NO_ALT_SCREEN_FLAG) {
            args.push(NO_ALT_SCREEN_FLAG.to_string());
        }

        // Positional arguments follow the options, session ID before prompt
        if let Some(ref resume) = spawn_config.resume {
            args.push(resume.clone());
        }

        if let Some(ref prompt) = spawn_config.initial_prompt {
            // Codex CLI takes initial prompt as a positional argument
            args.push(prompt.clone());
        }

        args
    }

    /// Codex mints its own conversation ID; it is discovered from the rollout
    /// after the fact (see [`discover_session_id`])
    fn agent_session_id(&self, _spawn_config: &SpawnConfig) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
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
            resume: None,
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
            resume: None,
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

    fn resume_spawn_config(resume: Option<&str>) -> SpawnConfig {
        SpawnConfig {
            session_id: Uuid::new_v4(),
            session_name: "test".to_string(),
            working_dir: PathBuf::from("/tmp"),
            initial_prompt: None,
            rows: 24,
            cols: 80,
            claude_config_dir: None,
            codex_home: None,
            resume: resume.map(|r| r.to_string()),
        }
    }

    #[test]
    fn test_resume_uses_the_resume_subcommand_with_the_session_id() {
        let adapter = CodexAdapter::new();
        let args = adapter.build_args(&resume_spawn_config(Some("019aa0c9-conversation")));

        // Without this, "resuming" silently starts a brand-new conversation
        assert_eq!(
            args.first().map(|s| s.as_str()),
            Some("resume"),
            "resume is a subcommand and must lead: {args:?}"
        );
        assert!(
            args.contains(&"019aa0c9-conversation".to_string()),
            "the conversation ID must actually be passed: {args:?}"
        );
        // `codex resume [OPTIONS] [SESSION_ID]` - options precede the positional
        let id_at = args
            .iter()
            .position(|a| a == "019aa0c9-conversation")
            .unwrap();
        let flag_at = args.iter().position(|a| a == NO_ALT_SCREEN_FLAG).unwrap();
        assert!(flag_at < id_at, "options must precede SESSION_ID: {args:?}");
    }

    #[test]
    fn test_fresh_spawn_does_not_use_the_resume_subcommand() {
        let adapter = CodexAdapter::new();
        let args = adapter.build_args(&resume_spawn_config(None));

        assert!(!args.contains(&"resume".to_string()), "{args:?}");
    }

    #[test]
    fn test_resume_still_passes_an_initial_prompt_last() {
        let adapter = CodexAdapter::new();
        let mut spawn_config = resume_spawn_config(Some("conv-id"));
        spawn_config.initial_prompt = Some("carry on".to_string());

        let args = adapter.build_args(&spawn_config);

        // `codex resume [OPTIONS] [SESSION_ID] [PROMPT]`
        let id_at = args.iter().position(|a| a == "conv-id").unwrap();
        let prompt_at = args.iter().position(|a| a == "carry on").unwrap();
        assert!(id_at < prompt_at, "prompt must follow SESSION_ID: {args:?}");
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
            resume: None,
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
            resume: None,
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
            resume: None,
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

    // plan_notify: the pure policy behind configure_codex_notify

    fn panoptes_script() -> PathBuf {
        PathBuf::from("/test/codex-notify.sh")
    }

    #[test]
    fn test_plan_notify_inserts_when_absent() {
        let plan = CodexAdapter::plan_notify(None, &panoptes_script());
        let NotifyPlan::Set(value) = plan else {
            panic!("expected Set, got {plan:?}");
        };
        let arr = value.as_array().unwrap();
        assert_eq!(arr[0].as_str(), Some("bash"));
        assert_eq!(arr[1].as_str(), Some("/test/codex-notify.sh"));
    }

    #[test]
    fn test_plan_notify_exact_match_needs_no_write() {
        let existing = CodexAdapter::notify_array_value(&[
            "bash".to_string(),
            "/test/codex-notify.sh".to_string(),
        ]);
        assert_eq!(
            CodexAdapter::plan_notify(Some(&existing), &panoptes_script()),
            NotifyPlan::AlreadyConfigured
        );
    }

    #[test]
    fn test_plan_notify_empty_array_is_unsupported() {
        // An empty argv can neither run nor be chained; guessing would
        // either drop the user's intent or invent one
        let existing = toml::Value::Array(vec![]);
        assert_eq!(
            CodexAdapter::plan_notify(Some(&existing), &panoptes_script()),
            NotifyPlan::Unsupported
        );
    }

    #[test]
    fn test_plan_notify_mixed_type_array_is_unsupported() {
        let existing = toml::Value::Array(vec![
            toml::Value::String("bash".to_string()),
            toml::Value::Integer(42),
        ]);
        assert_eq!(
            CodexAdapter::plan_notify(Some(&existing), &panoptes_script()),
            NotifyPlan::Unsupported
        );
    }

    #[test]
    fn test_plan_notify_chains_a_foreign_hook() {
        let existing =
            CodexAdapter::notify_array_value(&["echo".to_string(), "legacy-hook".to_string()]);
        let plan = CodexAdapter::plan_notify(Some(&existing), &panoptes_script());
        let NotifyPlan::Set(value) = plan else {
            panic!("expected Set, got {plan:?}");
        };
        let arr = value.as_array().unwrap();
        assert_eq!(arr[0].as_str(), Some("bash"));
        assert_eq!(arr[1].as_str(), Some("-lc"));
        let script = arr[2].as_str().unwrap();
        assert!(script.contains("/test/codex-notify.sh"));
        assert!(script.contains("'echo' 'legacy-hook'"));
    }

    #[test]
    fn test_plan_notify_is_idempotent_over_its_own_chaining() {
        // Planning again over the value a previous chain produced must not
        // wrap it a second time - the exact "no rewrite, no backup" guarantee
        let existing =
            CodexAdapter::notify_array_value(&["echo".to_string(), "legacy-hook".to_string()]);
        let NotifyPlan::Set(chained) =
            CodexAdapter::plan_notify(Some(&existing), &panoptes_script())
        else {
            panic!("first plan should chain");
        };

        assert_eq!(
            CodexAdapter::plan_notify(Some(&chained), &panoptes_script()),
            NotifyPlan::AlreadyConfigured,
            "a chained value must be recognised, not wrapped again"
        );
    }

    #[test]
    fn test_plan_notify_recognises_a_string_command_mentioning_the_script() {
        // Codex accepts a shell command string as well as an argv array
        let existing = toml::Value::String("/test/codex-notify.sh \"$@\"; my-own-hook".to_string());
        assert_eq!(
            CodexAdapter::plan_notify(Some(&existing), &panoptes_script()),
            NotifyPlan::AlreadyConfigured
        );
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
            resume: None,
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
            resume: None,
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
            resume: None,
        };

        let resolved = CodexAdapter::resolve_codex_home(&spawn_config);
        // Should resolve to ~/.codex
        assert!(resolved.ends_with(".codex"));
    }

    // Rollout discovery
    //
    // Codex has no flag to dictate its session ID, so this is the one place in
    // the recovery path that infers rather than dictates - it earns the tests.

    /// Write a rollout file the way Codex does: `session_meta` on line one,
    /// conversation after it.
    /// `created_at` is the conversation's own timestamp, which is what
    /// discovery matches on - deliberately independent of the file's mtime.
    fn write_rollout(
        codex_home: &Path,
        id: &str,
        cwd: &Path,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> PathBuf {
        let dir = codex_home
            .join("sessions")
            .join(created_at.format("%Y").to_string())
            .join(created_at.format("%m").to_string())
            .join(created_at.format("%d").to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-{}-{}.jsonl", created_at.timestamp(), id));
        let meta = serde_json::json!({
            "timestamp": created_at.to_rfc3339(),
            "type": "session_meta",
            "payload": {
                "id": id,
                "timestamp": created_at.to_rfc3339(),
                "cwd": cwd.to_string_lossy(),
                "originator": "codex_cli_rs"
            }
        });
        std::fs::write(&path, format!("{}\n{{\"type\":\"message\"}}\n", meta)).unwrap();
        path
    }

    /// Write a rollout that belongs to a subagent of `parent`
    fn write_subagent_rollout(
        codex_home: &Path,
        id: &str,
        parent: &str,
        cwd: &Path,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> PathBuf {
        let dir = codex_home
            .join("sessions")
            .join(created_at.format("%Y").to_string())
            .join(created_at.format("%m").to_string())
            .join(created_at.format("%d").to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-{}-{}.jsonl", created_at.timestamp(), id));
        let meta = serde_json::json!({
            "timestamp": created_at.to_rfc3339(),
            "type": "session_meta",
            "payload": {
                // On a subagent rollout `id` is the subagent's own, while
                // `session_id` is the parent's - a trap for anything reading
                // these files without being explicit about which it wants
                "id": id,
                "session_id": parent,
                "forked_from_id": parent,
                "timestamp": created_at.to_rfc3339(),
                "cwd": cwd.to_string_lossy(),
                "source": {"subagent": {"thread_spawn": {"parent_thread_id": parent}}}
            }
        });
        std::fs::write(&path, format!("{}\n", meta)).unwrap();
        path
    }

    #[test]
    fn test_never_claims_a_subagent_rollout() {
        // A real case from this machine: a subagent rollout sitting in a
        // Panoptes worktree, with its own fresh timestamp, matching every
        // criterion the discovery used. Claiming it would point the session at
        // a subagent instead of the conversation the user was having.
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let started = an_hour_ago();

        write_subagent_rollout(
            home.path(),
            "subagent-id",
            "parent-id",
            cwd.path(),
            started + chrono::Duration::minutes(1),
        );

        assert_eq!(
            discover_session_id(home.path(), cwd.path(), started, &nothing_claimed()),
            None,
            "a subagent rollout must never be claimed as a session's conversation"
        );

        // The parent's own rollout is still found, even though it is older -
        // the subagent must not shadow it
        write_rollout(
            home.path(),
            "parent-id",
            cwd.path(),
            started + chrono::Duration::seconds(30),
        );
        assert_eq!(
            discover_session_id(home.path(), cwd.path(), started, &nothing_claimed()).as_deref(),
            Some("parent-id")
        );
    }

    #[test]
    fn test_rollout_path_finds_a_known_conversation() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let written = write_rollout(home.path(), "wanted-id", cwd.path(), an_hour_ago());
        write_rollout(home.path(), "other-id", cwd.path(), an_hour_ago());

        assert_eq!(rollout_path(home.path(), "wanted-id"), Some(written));
        assert_eq!(rollout_path(home.path(), "no-such-id"), None);
    }

    #[test]
    fn test_rollout_path_tolerates_a_missing_sessions_directory() {
        let home = TempDir::new().unwrap();
        assert_eq!(rollout_path(home.path(), "anything"), None);
    }

    fn an_hour_ago() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now() - chrono::Duration::hours(1)
    }

    fn nothing_claimed() -> std::collections::HashSet<String> {
        std::collections::HashSet::new()
    }

    #[test]
    fn test_discovers_session_id_from_rollout() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        write_rollout(
            home.path(),
            "019aa0c9-8dea-7611-89d1-9d94731a6a6d",
            cwd.path(),
            an_hour_ago(),
        );

        let found = discover_session_id(home.path(), cwd.path(), an_hour_ago(), &nothing_claimed());

        assert_eq!(
            found.as_deref(),
            Some("019aa0c9-8dea-7611-89d1-9d94731a6a6d")
        );
    }

    #[test]
    fn test_ignores_rollouts_from_a_different_working_directory() {
        let home = TempDir::new().unwrap();
        let ours = TempDir::new().unwrap();
        let theirs = TempDir::new().unwrap();
        write_rollout(home.path(), "not-ours", theirs.path(), an_hour_ago());

        // Another Codex session running concurrently elsewhere must not be
        // mistaken for this one
        assert!(
            discover_session_id(home.path(), ours.path(), an_hour_ago(), &nothing_claimed())
                .is_none()
        );
    }

    #[test]
    fn test_ignores_rollouts_created_before_the_session_started() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        write_rollout(
            home.path(),
            "older-session",
            cwd.path(),
            chrono::Utc::now() - chrono::Duration::days(30),
        );

        // A previous session in the same directory would otherwise be adopted,
        // silently pointing this session at the wrong conversation
        assert!(
            discover_session_id(home.path(), cwd.path(), an_hour_ago(), &nothing_claimed())
                .is_none()
        );
    }

    #[test]
    fn test_never_returns_a_conversation_another_session_already_owns() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        write_rollout(home.path(), "already-taken", cwd.path(), an_hour_ago());

        // Two Codex sessions on the same branch share a working directory. The
        // first to resolve owns that conversation; the second must not be
        // handed the same one.
        let claimed = std::collections::HashSet::from(["already-taken".to_string()]);
        assert!(discover_session_id(home.path(), cwd.path(), an_hour_ago(), &claimed).is_none());
    }

    #[test]
    fn test_two_sessions_in_one_directory_get_their_own_conversations() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let first_started = chrono::Utc::now() - chrono::Duration::minutes(10);
        let second_started = chrono::Utc::now() - chrono::Duration::minutes(5);
        write_rollout(home.path(), "first", cwd.path(), first_started);
        write_rollout(home.path(), "second", cwd.path(), second_started);

        // Resolved oldest-first, accumulating claims as the caller does
        let mut claimed = nothing_claimed();
        let a =
            discover_session_id(home.path(), cwd.path(), first_started, &claimed).expect("first");
        claimed.insert(a.clone());
        let b =
            discover_session_id(home.path(), cwd.path(), second_started, &claimed).expect("second");

        assert_eq!(a, "first");
        assert_eq!(b, "second");
    }

    #[test]
    fn test_matches_on_conversation_time_not_file_mtime() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        // An old conversation still being actively used: Codex appends turns to
        // it, so its mtime is newer than a session that started a moment ago.
        // Matching on mtime would hand this session the wrong conversation.
        let old = write_rollout(
            home.path(),
            "old-but-recently-touched",
            cwd.path(),
            chrono::Utc::now() - chrono::Duration::days(3),
        );
        std::fs::write(
            &old,
            std::fs::read_to_string(&old).unwrap() + "{\"type\":\"message\"}\n",
        )
        .unwrap();

        assert!(
            discover_session_id(home.path(), cwd.path(), an_hour_ago(), &nothing_claimed())
                .is_none()
        );
    }

    #[test]
    fn test_returns_none_when_codex_has_not_written_a_rollout_yet() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join("sessions")).unwrap();

        // The normal state for the first moments of a session
        assert!(
            discover_session_id(home.path(), cwd.path(), an_hour_ago(), &nothing_claimed())
                .is_none()
        );
    }

    #[test]
    fn test_missing_sessions_directory_is_not_an_error() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();

        assert!(
            discover_session_id(home.path(), cwd.path(), an_hour_ago(), &nothing_claimed())
                .is_none()
        );
    }

    #[test]
    fn test_skips_unparseable_rollouts() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let dir = home.path().join("sessions/2026/01/01");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rollout-broken.jsonl"), "not json at all\n").unwrap();
        std::fs::write(dir.join("rollout-empty.jsonl"), "").unwrap();
        write_rollout(home.path(), "the-good-one", cwd.path(), an_hour_ago());

        // One corrupt file must not hide a valid one
        assert_eq!(
            discover_session_id(home.path(), cwd.path(), an_hour_ago(), &nothing_claimed())
                .as_deref(),
            Some("the-good-one")
        );
    }

    #[test]
    fn test_matches_through_symlinked_working_directory() {
        let home = TempDir::new().unwrap();
        let real = TempDir::new().unwrap();
        let link_parent = TempDir::new().unwrap();
        let link = link_parent.path().join("linked");
        std::os::unix::fs::symlink(real.path(), &link).unwrap();

        // Codex records the resolved path; Panoptes may hold the symlinked one.
        // This is the /tmp -> /private/tmp case on macOS.
        write_rollout(home.path(), "via-symlink", real.path(), an_hour_ago());

        assert_eq!(
            discover_session_id(home.path(), &link, an_hour_ago(), &nothing_claimed()).as_deref(),
            Some("via-symlink")
        );
    }

    #[test]
    fn test_ignores_non_jsonl_files() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let dir = home.path().join("sessions/2026/01/01");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.txt"), "irrelevant\n").unwrap();

        assert!(
            discover_session_id(home.path(), cwd.path(), an_hour_ago(), &nothing_claimed())
                .is_none()
        );
    }
}
