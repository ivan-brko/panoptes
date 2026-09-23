//! Claude Code adapter implementation
//!
//! This module implements the `AgentAdapter` trait for Claude Code CLI.
//! It handles hook script installation, session settings configuration,
//! and process spawning.
//!
//! # The status line
//!
//! Claude reports its plan rate limits, and the running session's real
//! context window, only to a `statusLine` command, as JSON on stdin. Panoptes
//! puts its own command in `<working dir>/.claude/settings.local.json`, next
//! to the hooks. A `statusLine` there replaces whatever the user configured at
//! any lower layer, so the command *wraps* the user's own: it forwards the
//! document to the hook server, then pipes the same document to the user's
//! command and prints what that prints. With no user status line it prints
//! nothing, which is as close as a command can get to Claude's default of no
//! status line (Claude still reserves the row, so it shows as one blank line).
//!
//! The user's command travels as an argument of ours, so a later spawn can
//! tell its own command from the user's and never wraps itself. A command
//! that came from `settings.local.json` itself - the one layer Panoptes
//! overwrites - is marked `--local`, so it can be put back when the feature
//! is turned off (`claude_status_line = false`). One found at a lower layer is
//! re-read from that layer at every spawn instead, so an edit to it is
//! picked up.
//!
//! Like the hooks, the command is never removed when a session ends: the
//! file is shared by every session in the working directory, including ones
//! still running, and Claude re-reads it live. Left behind, it is inert
//! outside Panoptes - without `PANOPTES_SESSION_ID` it posts nothing and
//! only runs the user's command.

use crate::config::Config;
use crate::hooks::HookEventType;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::adapter::{AgentAdapter, SpawnConfig};
use super::events::{UsageSnapshot, WindowSource};

/// Base hook script filename (shared across all sessions)
const HOOK_SCRIPT_NAME: &str = "panoptes-hook.sh";

/// Status-line wrapper filename (shared across all sessions)
///
/// Also how a `statusLine` command is recognised as Panoptes' own, whatever
/// directory it was installed to.
const STATUS_LINE_SCRIPT_NAME: &str = "panoptes-statusline.sh";

/// Marks a wrapped command that came from `settings.local.json` itself
const STATUS_LINE_LOCAL_FLAG: &str = "--local";

/// A user's own `statusLine`, found where Claude would find it
#[derive(Debug, Clone, PartialEq)]
struct UserStatusLine {
    /// The whole setting - `padding`, `refreshInterval` and the rest are kept
    setting: serde_json::Value,
    /// The command it runs
    command: String,
    /// Whether it came from `settings.local.json`, the file Panoptes rewrites
    local: bool,
}

/// What a spawn does to the working directory's `statusLine`
#[derive(Debug, Clone, PartialEq)]
enum StatusLinePlan {
    /// Wrap the user's status line with the script at this path
    Install {
        script: PathBuf,
        /// The user-level settings file, the lowest layer
        user_settings: Option<PathBuf>,
    },
    /// Put back whatever Panoptes wrapped, if it wrapped anything
    Restore,
}

/// Hook event types Panoptes registers with Claude Code
///
/// `UserPromptSubmit` is what makes `Thinking` an observation instead of a
/// guess about keystrokes, and `SessionStart`/`SessionEnd` bracket the process
/// so its lifecycle does not have to be inferred from PTY output alone.
///
/// The newer events each close a gap the older set left:
///
/// - `StopFailure` fires *instead of* `Stop` when a turn dies on an API error
///   or a usage limit. Without it the session sat in `Thinking` until the
///   stall watchdog guessed, with no reason to show.
/// - `PermissionDenied` fires when auto mode's classifier refuses a tool
///   call. The turn carries on, so any approval still showing is stale.
/// - `SubagentStart`/`SubagentStop` count the subagents running inside the
///   session, and `Stop`'s own payload lists the background work outliving
///   the turn. Both keep the suspend sweep off a session that only looks idle.
/// - `Elicitation`/`ElicitationResult` bracket an MCP server's question to
///   the user, which otherwise only arrives as a `Notification` whose
///   `notification_type` is not guaranteed.
///
/// Claude offers more hooks than this, deliberately left unregistered, since
/// an event nothing consumes is just a process spawn per firing:
///
/// - `PreCompact`/`PostCompact`: `SessionStart` with `source: compact`
///   already reports compaction, and it changes no state.
/// - `PostToolBatch`: the per-tool events already say everything it does.
/// - `UserPromptExpansion`: `UserPromptSubmit` already starts the turn.
/// - `PreModelSwitch`/`PostModelSwitch`: the model comes from the transcript.
/// - `Setup`: one-off repository setup, before any session state exists.
/// - `TeammateIdle`: agent teams are not modelled.
/// - `TaskCreated`/`TaskCompleted`: despite the names, these are the agent's
///   to-do list (`TaskCreate`/`TaskUpdate`), not background work - and a
///   deleted item never fires `TaskCompleted`, so counting them would leak.
///   Background work comes from `Stop`'s `background_tasks` instead.
/// - `ConfigChange`: settings edits change nothing Panoptes shows.
/// - `WorktreeCreate`/`WorktreeRemove`: Panoptes manages its own worktrees.
/// - `InstructionsLoaded`, `FileChanged`, `DirectoryAdded`, `MessageDisplay`:
///   nothing in the session model depends on them.
/// - `CwdChanged`: worth a follow-up, since Panoptes shows a working directory,
///   but not handled yet.
const HOOK_EVENTS: &[HookEventType] = &[
    HookEventType::SessionStart,
    HookEventType::SessionEnd,
    HookEventType::UserPromptSubmit,
    HookEventType::PreToolUse,
    HookEventType::PostToolUse,
    HookEventType::PostToolUseFailure,
    HookEventType::Stop,
    HookEventType::StopFailure,
    HookEventType::Notification,
    HookEventType::PermissionRequest,
    HookEventType::PermissionDenied,
    HookEventType::SubagentStart,
    HookEventType::SubagentStop,
    HookEventType::Elicitation,
    HookEventType::ElicitationResult,
];

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

    /// The context window a `--model` argument settles, if it settles one
    ///
    /// Only the `[1m]` suffix does: Claude Code then runs that model at 1M, but
    /// its transcript logs the bare id, from which a 1M-capable older model
    /// (`claude-sonnet-4-6`) would be read as 200k. Without the suffix the
    /// transcript's own guess is as good as anything the argument could add.
    fn launch_context_window(args: &[String]) -> Option<u64> {
        // The last `--model` is the one Claude Code honours
        let mut model = None;
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            if let Some(value) = arg.strip_prefix("--model=") {
                model = Some(value);
            } else if arg == "--model" {
                model = args.next().map(String::as_str);
            }
        }
        model?
            .to_ascii_lowercase()
            .ends_with("[1m]")
            .then_some(1_000_000)
    }

    /// Get the path to the shared hook script
    ///
    /// Shared with Codex, whose lifecycle hooks take the same payload on stdin
    /// (see `agent/codex.rs`).
    pub(crate) fn hook_script_path(config: &Config) -> PathBuf {
        config.hooks_dir.join(HOOK_SCRIPT_NAME)
    }

    /// Install the shared hook script and create symlinks for each event type
    fn install_hook_script(config: &Config) -> Result<Vec<(HookEventType, PathBuf)>> {
        Self::warn_if_jq_missing();

        let script_path = Self::hook_script_path(config);
        super::install_executable_script(
            &script_path,
            &Self::generate_hook_script(config.hook_port),
        )
        .context("Failed to install hook script")?;

        // Create symlinks for each event type so basename $0 returns the event name
        let mut event_scripts = Vec::with_capacity(HOOK_EVENTS.len());
        for event in HOOK_EVENTS {
            let event_name = event.as_str();
            let symlink_path = config.hooks_dir.join(format!("{}.sh", event_name));

            // Remove existing symlink if present
            if symlink_path.exists() || symlink_path.is_symlink() {
                let _ = std::fs::remove_file(&symlink_path);
            }

            // Create symlink
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&script_path, &symlink_path)
                    .with_context(|| format!("Failed to create symlink for {}", event_name))?;
            }

            event_scripts.push((*event, symlink_path));
        }

        Ok(event_scripts)
    }

    /// Generate the hook script content
    ///
    /// Claude pipes the hook payload as JSON on stdin. The script wraps it in
    /// an envelope carrying the Panoptes session ID and the event name, then
    /// POSTs the result.
    ///
    /// The payload is forwarded verbatim rather than picked apart here. Shell
    /// string interpolation cannot build JSON safely - the previous version
    /// substituted an extracted tool name straight into a here-doc, so the
    /// first quote or newline in any field produced malformed JSON and the
    /// event was silently lost. `jq` already does this correctly, so the script
    /// hands it the whole document and lets Panoptes decide what it needs.
    pub(crate) fn generate_hook_script(port: u16) -> String {
        format!(
            r#"#!/bin/bash
# Panoptes hook script for Claude Code
# Receives the hook payload as JSON on stdin and forwards it to Panoptes,
# wrapped in an envelope identifying the session and the event.

# Read session ID from environment
SESSION_ID="${{PANOPTES_SESSION_ID:-unknown}}"

# The script name says which hook fired: every event is a symlink to this file
hook_name="$(basename "$0" .sh)"

# Read the whole document. `read -r` would stop at the first newline, which is
# fine for compact JSON but silently truncates anything pretty-printed.
json_input="$(cat)"

# Anything non-numeric here - an unset PATH, a date that is not on it - would
# produce `"timestamp":` and cost us the whole event, which is the exact class
# of failure this script exists to avoid.
timestamp="$(date +%s 2>/dev/null)"
case "$timestamp" in
    '' | *[!0-9]*) timestamp=0 ;;
esac

payload=""
if command -v jq > /dev/null 2>&1 && [ -n "$json_input" ]; then
    payload="$(printf '%s' "$json_input" | jq -c \
        --arg sid "$SESSION_ID" \
        --arg ev "$hook_name" \
        --argjson ts "$timestamp" \
        '{{session_id: $sid, event: $ev, timestamp: $ts, payload: .}}' 2>/dev/null)"
fi

# Degraded path: without jq the agent payload cannot be embedded safely, so
# send the envelope alone. State still tracks correctly from the event name;
# only payload-derived detail (tool names, notification_type) goes missing.
# Interpolation is safe here because both values are ours: a UUID from the
# environment and the basename of a symlink we created.
if [ -z "$payload" ]; then
    payload="{{\"session_id\":\"$SESSION_ID\",\"event\":\"$hook_name\",\"timestamp\":$timestamp}}"
fi

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

    /// Warn once if `jq` is missing
    ///
    /// `jq` is not a hard requirement - the hook script degrades to an
    /// envelope-only payload without it - but the degradation is invisible from
    /// the UI, so it is worth saying out loud rather than leaving the user to
    /// wonder why tool names never appear.
    fn warn_if_jq_missing() {
        use std::sync::Once;
        static WARNED: Once = Once::new();

        WARNED.call_once(|| {
            let found = std::env::var_os("PATH")
                .map(|path| std::env::split_paths(&path).any(|dir| dir.join("jq").is_file()))
                .unwrap_or(false);
            if !found {
                tracing::warn!(
                    "jq was not found on PATH; Claude hook events will carry no tool names \
                     or notification detail. Install jq for full session state tracking."
                );
            }
        });
    }

    /// Create the session-specific settings file, hooks only
    ///
    /// `Restore` leaves `statusLine` alone unless Panoptes wrapped one.
    #[cfg(test)]
    fn create_session_settings(
        working_dir: &Path,
        event_scripts: &[(HookEventType, PathBuf)],
    ) -> Result<PathBuf> {
        Self::write_session_settings(working_dir, event_scripts, &StatusLinePlan::Restore)
    }

    /// Create the session-specific settings file
    ///
    /// This function MERGES hooks into existing settings rather than overwriting,
    /// preserving Claude Code trust settings and other user configurations.
    /// `statusLine` is the one other key it may change, per `status_line`.
    fn write_session_settings(
        working_dir: &Path,
        event_scripts: &[(HookEventType, PathBuf)],
        status_line: &StatusLinePlan,
    ) -> Result<PathBuf> {
        // Create .claude directory in the working directory
        let claude_dir = working_dir.join(".claude");
        std::fs::create_dir_all(&claude_dir).context("Failed to create .claude directory")?;

        let settings_path = claude_dir.join("settings.local.json");

        // Load existing settings if present, otherwise start fresh
        let mut settings: serde_json::Value = if settings_path.exists() {
            let content = std::fs::read_to_string(&settings_path)
                .context("Failed to read existing settings")?;
            serde_json::from_str(&content).unwrap_or_else(|e| {
                tracing::warn!(
                    "Failed to parse existing settings.local.json: {}, starting fresh",
                    e
                );
                serde_json::json!({})
            })
        } else {
            serde_json::json!({})
        };

        // Build hooks config using the event-specific script paths
        let mut hooks = serde_json::Map::new();
        for (event, script_path) in event_scripts {
            let script_path_str = script_path.to_string_lossy().to_string();
            hooks.insert(
                event.as_str().to_string(),
                serde_json::json!([
                    {
                        "matcher": ".*",
                        "hooks": [{"type": "command", "command": script_path_str}]
                    }
                ]),
            );
        }

        // Merge hooks into settings (only overwrite the hooks key, preserve everything else)
        settings["hooks"] = serde_json::Value::Object(hooks);

        Self::plan_status_line(
            &mut settings,
            status_line,
            &claude_dir.join("settings.json"),
        );

        // Create backup before writing if file exists (safeguard)
        if settings_path.exists() {
            let backup_path = settings_path.with_extension("json.bak");
            if let Err(e) = std::fs::copy(&settings_path, &backup_path) {
                tracing::warn!("Failed to create backup of settings.local.json: {}", e);
            }
        }

        std::fs::write(
            &settings_path,
            serde_json::to_string_pretty(&settings).context("Failed to serialize settings")?,
        )
        .context("Failed to write settings file")?;

        Ok(settings_path)
    }

    /// Get the path to the shared status-line wrapper
    fn status_line_script_path(config: &Config) -> PathBuf {
        config.hooks_dir.join(STATUS_LINE_SCRIPT_NAME)
    }

    /// Install the shared status-line wrapper
    fn install_status_line_script(config: &Config) -> Result<PathBuf> {
        let script_path = Self::status_line_script_path(config);
        super::install_executable_script(
            &script_path,
            &Self::generate_status_line_script(&format!(
                "http://127.0.0.1:{}/hook",
                config.hook_port
            )),
        )
        .context("Failed to install status line script")?;
        Ok(script_path)
    }

    /// Generate the status-line wrapper, posting to `hook_url`
    ///
    /// Claude runs it on every status-line refresh, so it is kept to the
    /// shell's own builtins plus `date` and a backgrounded `curl`. The envelope
    /// is built by splicing Claude's document in whole rather than with `jq`:
    /// that is safe because the document is already JSON and nothing is
    /// picked out of it, and it saves a process on the hot path. The session
    /// ID is ours, and is checked to look like one before it is spliced in.
    ///
    /// Arguments: `[--local] [COMMAND]`, where `COMMAND` is the user's own
    /// status line, run by `bash -c` as Claude would run it.
    fn generate_status_line_script(hook_url: &str) -> String {
        let url = super::shell_quote(hook_url);
        format!(
            r#"#!/bin/sh
# Panoptes status line for Claude Code
# Forwards the status-line JSON on stdin to Panoptes, then runs the user's own
# status line on the same input and prints what it prints.

input="$(cat)"

# Outside Panoptes there is nobody to tell: only the user's command runs
sid="${{PANOPTES_SESSION_ID:-}}"
case "$sid" in
    '' | *[!0-9A-Fa-f-]*) sid="" ;;
esac

if [ -n "$sid" ] && [ -n "$input" ]; then
    timestamp="$(date +%s 2>/dev/null)"
    case "$timestamp" in
        '' | *[!0-9]*) timestamp=0 ;;
    esac
    # Fire and forget, detached from stdout so Claude is not kept waiting
    curl -s -X POST {url} \
        -H "Content-Type: application/json" \
        -d "{{\"session_id\":\"$sid\",\"event\":\"StatusLine\",\"timestamp\":$timestamp,\"payload\":$input}}" \
        --connect-timeout 1 \
        --max-time 2 \
        > /dev/null 2>&1 &
fi

if [ "${{1:-}}" = "{local_flag}" ]; then
    shift
fi

if [ -n "${{1:-}}" ]; then
    printf '%s' "$input" | bash -c "$1"
    exit $?
fi
exit 0
"#,
            local_flag = STATUS_LINE_LOCAL_FLAG,
        )
    }

    /// The `statusLine` command that runs `script` in front of `user`'s
    fn status_line_command(script: &Path, user: Option<&UserStatusLine>) -> String {
        let mut words = vec![super::shell_quote(&script.to_string_lossy())];
        if let Some(user) = user {
            if user.local {
                words.push(super::shell_quote(STATUS_LINE_LOCAL_FLAG));
            }
            words.push(super::shell_quote(&user.command));
        }
        words.join(" ")
    }

    /// Read a `statusLine` command back as Panoptes' own
    ///
    /// Returns whether it was marked local and the user command it wraps, or
    /// `None` if it is not one of ours. Only the exact shape
    /// [`Self::status_line_command`] writes is accepted.
    fn parse_own_status_line(command: &str) -> Option<(bool, Option<String>)> {
        let words = super::split_shell_quoted(command)?;
        let (script, rest) = words.split_first()?;
        if Path::new(script).file_name()? != STATUS_LINE_SCRIPT_NAME {
            return None;
        }
        match rest {
            [] => Some((false, None)),
            [flag, user] if flag == STATUS_LINE_LOCAL_FLAG => Some((true, Some(user.clone()))),
            [user] if user != STATUS_LINE_LOCAL_FLAG => Some((false, Some(user.clone()))),
            _ => None,
        }
    }

    /// The command a `statusLine` setting runs, if it is one Claude would run
    fn setting_command(setting: &serde_json::Value) -> Option<&str> {
        let object = setting.as_object()?;
        if object.get("type").and_then(|t| t.as_str()) != Some("command") {
            return None;
        }
        object
            .get("command")?
            .as_str()
            .filter(|c| !c.trim().is_empty())
    }

    /// The user-level `settings.json` for a spawn's `CLAUDE_CONFIG_DIR`
    ///
    /// A profile's directory when it names one; otherwise whatever Panoptes'
    /// own environment says, since the spawned process inherits it; otherwise
    /// `~/.claude`.
    fn user_settings_path(claude_config_dir: Option<&Path>) -> Option<PathBuf> {
        let dir = match claude_config_dir {
            Some(dir) => dir.to_path_buf(),
            None => match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
                Some(dir) => PathBuf::from(dir),
                None => dirs::home_dir()?.join(".claude"),
            },
        };
        Some(dir.join("settings.json"))
    }

    /// Read one settings layer's `statusLine`, tolerating a missing or broken file
    fn read_status_line_setting(path: &Path) -> Option<serde_json::Value> {
        let content = std::fs::read_to_string(path).ok()?;
        let settings: serde_json::Value = serde_json::from_str(&content).ok()?;
        settings.get("statusLine").cloned()
    }

    /// The status line the user would see without Panoptes
    ///
    /// Claude takes `statusLine` from the highest layer that sets it: the
    /// project's `settings.local.json`, then its `settings.json`, then the
    /// user's `$CLAUDE_CONFIG_DIR/settings.json`. Panoptes' own command is
    /// seen through, to the command it wraps; an unmarked one in the local
    /// file wraps a lower layer's, which is read afresh from that layer.
    fn resolve_user_status_line(
        local_settings: &serde_json::Value,
        project_settings: &Path,
        user_settings: Option<&Path>,
    ) -> Option<UserStatusLine> {
        let layers = [
            (local_settings.get("statusLine").cloned(), true),
            (Self::read_status_line_setting(project_settings), false),
            (
                user_settings.and_then(Self::read_status_line_setting),
                false,
            ),
        ];
        for (setting, local) in layers {
            let Some(setting) = setting else { continue };
            let Some(command) = Self::setting_command(&setting) else {
                continue;
            };
            let (command, local) = match Self::parse_own_status_line(command) {
                // Ours: see through it. Only a local mark says the wrapped
                // command lives nowhere else.
                Some((marked, Some(wrapped))) if marked || !local => (wrapped, local),
                Some(_) => continue,
                None => (command.to_string(), local),
            };
            let mut setting = setting;
            setting["command"] = serde_json::Value::String(command.clone());
            return Some(UserStatusLine {
                setting,
                command,
                local,
            });
        }
        None
    }

    /// Apply a [`StatusLinePlan`] to the local settings about to be written
    fn plan_status_line(
        settings: &mut serde_json::Value,
        plan: &StatusLinePlan,
        project_settings: &Path,
    ) {
        match plan {
            StatusLinePlan::Install {
                script,
                user_settings,
            } => {
                let user = Self::resolve_user_status_line(
                    settings,
                    project_settings,
                    user_settings.as_deref(),
                );
                // The user's own options - padding, refresh interval - carry
                // over; only the command is ours
                let mut setting = user
                    .as_ref()
                    .map(|u| u.setting.clone())
                    .unwrap_or_else(|| serde_json::json!({}));
                setting["type"] = serde_json::Value::String("command".to_string());
                setting["command"] =
                    serde_json::Value::String(Self::status_line_command(script, user.as_ref()));
                settings["statusLine"] = setting;
            }
            StatusLinePlan::Restore => {
                let Some(setting) = settings.get("statusLine") else {
                    return;
                };
                let Some(own) =
                    Self::setting_command(setting).and_then(Self::parse_own_status_line)
                else {
                    return;
                };
                match own {
                    (true, Some(original)) => {
                        settings["statusLine"]["command"] = serde_json::Value::String(original);
                    }
                    _ => {
                        if let Some(object) = settings.as_object_mut() {
                            object.remove("statusLine");
                        }
                    }
                }
            }
        }
    }

    /// The Claude conversation ID this spawn will use
    ///
    /// Panoptes dictates the conversation UUID rather than discovering it, so
    /// the Panoptes session ID and the Claude session ID are the same value.
    /// That removes the whole class of races where a session dies before it
    /// ever reports an ID back, leaving an unreachable transcript on disk.
    fn conversation_id(spawn_config: &SpawnConfig) -> String {
        spawn_config
            .resume
            .clone()
            .unwrap_or_else(|| spawn_config.session_id.to_string())
    }

    /// Arguments that bind this spawn to a specific Claude conversation
    ///
    /// A fresh session claims its UUID with `--session-id`; a recovered one
    /// reattaches with `--resume`. `--fork-session` is deliberately omitted so
    /// that resuming preserves the original ID, keeping the stored pointer
    /// valid across any number of restarts.
    ///
    /// Note that `--session-id` fails if the UUID is already in use, so callers
    /// must pass `resume` for any session that has been started before.
    fn conversation_args(spawn_config: &SpawnConfig) -> Vec<String> {
        match &spawn_config.resume {
            Some(id) => vec!["--resume".to_string(), id.clone()],
            None => vec![
                "--session-id".to_string(),
                spawn_config.session_id.to_string(),
            ],
        }
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
        let mut args = vec!["--enable-auto-mode".to_string()];
        args.extend(self.extra_args.clone());
        args
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
        // Force Claude Code to use consistent header mode (prevents flickering on resize)
        env.insert("CLAUDE_CODE_FORCE_FULL_LOGO".to_string(), "1".to_string());
        // Set custom Claude config directory if specified
        if let Some(ref config_dir) = spawn_config.claude_config_dir {
            env.insert(
                "CLAUDE_CONFIG_DIR".to_string(),
                config_dir.to_string_lossy().to_string(),
            );
        }
        env
    }

    fn setup_hooks(&self, config: &Config, spawn_config: &SpawnConfig) -> Result<Vec<PathBuf>> {
        let mut cleanup_paths = Vec::new();

        // Install shared hook script and create event-specific symlinks
        let event_scripts = Self::install_hook_script(config)?;
        // Note: We don't add the shared scripts to cleanup_paths since they're reused

        let status_line = if config.claude_status_line {
            StatusLinePlan::Install {
                script: Self::install_status_line_script(config)?,
                user_settings: Self::user_settings_path(spawn_config.claude_config_dir.as_deref()),
            }
        } else {
            StatusLinePlan::Restore
        };

        // Create session-specific settings file
        let settings_path =
            Self::write_session_settings(&spawn_config.working_dir, &event_scripts, &status_line)?;
        cleanup_paths.push(settings_path);

        Ok(cleanup_paths)
    }

    fn build_args(&self, spawn_config: &SpawnConfig) -> Vec<String> {
        let mut args = self.default_args();
        args.extend(Self::conversation_args(spawn_config));
        if let Some(ref prompt) = spawn_config.initial_prompt {
            args.push("--print".to_string());
            args.push(prompt.clone());
        }
        args
    }

    fn agent_session_id(&self, spawn_config: &SpawnConfig) -> Option<String> {
        Some(Self::conversation_id(spawn_config))
    }

    fn launch_usage(&self, spawn_config: &SpawnConfig) -> Option<UsageSnapshot> {
        let window = Self::launch_context_window(&self.build_args(spawn_config))?;
        Some(UsageSnapshot {
            context_window: Some(window),
            context_window_source: WindowSource::Launch,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;
    use uuid::Uuid;

    /// Event scripts as `install_hook_script` would report them, without
    /// touching the filesystem
    fn mock_event_scripts() -> Vec<(HookEventType, PathBuf)> {
        HOOK_EVENTS
            .iter()
            .map(|event| {
                (
                    *event,
                    PathBuf::from(format!("/test/{}.sh", event.as_str())),
                )
            })
            .collect()
    }

    fn test_spawn_config(working_dir: PathBuf) -> SpawnConfig {
        SpawnConfig {
            session_id: Uuid::new_v4(),
            session_name: "test-session".to_string(),
            working_dir,
            initial_prompt: None,
            rows: 24,
            cols: 80,
            claude_config_dir: None,
            codex_home: None,
            resume: None,
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
        assert_eq!(args, vec!["--enable-auto-mode".to_string()]);
    }

    #[test]
    fn test_launch_context_window() {
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        let window = |list: &[&str]| ClaudeCodeAdapter::launch_context_window(&args(list));

        assert_eq!(
            window(&["--model", "claude-sonnet-4-6[1m]"]),
            Some(1_000_000)
        );
        assert_eq!(window(&["--model=opus[1M]"]), Some(1_000_000));
        // The last one wins, as it does for Claude Code
        assert_eq!(window(&["--model", "sonnet[1m]", "--model", "haiku"]), None);
        // No suffix, or no model at all: the transcript's guess stands
        assert_eq!(window(&["--model", "claude-opus-5-5"]), None);
        assert_eq!(window(&["--enable-auto-mode"]), None);
        assert_eq!(window(&["--model"]), None);
    }

    #[test]
    fn test_launch_usage_seeds_a_launch_window() {
        let dir = TempDir::new().unwrap();
        let spawn = test_spawn_config(dir.path().to_path_buf());

        // What production spawns today: no `--model`, nothing to seed
        assert_eq!(ClaudeCodeAdapter::new().launch_usage(&spawn), None);

        let adapter = ClaudeCodeAdapter::with_args(vec![
            "--model".to_string(),
            "claude-sonnet-4-6[1m]".to_string(),
        ]);
        let usage = adapter.launch_usage(&spawn).expect("a launch window");
        assert_eq!(usage.context_window, Some(1_000_000));
        assert_eq!(usage.context_window_source, WindowSource::Launch);
        // Nothing else is known yet, so the header still shows nothing
        assert_eq!(usage.summary(), None);
    }

    #[test]
    fn test_claude_adapter_with_extra_args() {
        let adapter = ClaudeCodeAdapter::with_args(vec!["--verbose".to_string()]);
        let args = adapter.default_args();
        assert_eq!(
            args,
            vec!["--enable-auto-mode".to_string(), "--verbose".to_string()]
        );
    }

    #[test]
    fn test_fresh_spawn_claims_the_panoptes_session_id() {
        let spawn_config = test_spawn_config(PathBuf::from("/tmp"));
        let args = ClaudeCodeAdapter::conversation_args(&spawn_config);

        assert_eq!(
            args,
            vec![
                "--session-id".to_string(),
                spawn_config.session_id.to_string()
            ]
        );
    }

    #[test]
    fn test_resume_spawn_reattaches_without_forking() {
        let mut spawn_config = test_spawn_config(PathBuf::from("/tmp"));
        let prior = Uuid::new_v4().to_string();
        spawn_config.resume = Some(prior.clone());

        let args = ClaudeCodeAdapter::conversation_args(&spawn_config);

        assert_eq!(args, vec!["--resume".to_string(), prior]);
        // Forking would mint a new conversation ID and orphan the stored pointer
        assert!(!args.contains(&"--fork-session".to_string()));
        // --session-id would be rejected for an ID that is already in use
        assert!(!args.contains(&"--session-id".to_string()));
    }

    #[test]
    fn test_conversation_id_matches_session_id_for_fresh_spawn() {
        let spawn_config = test_spawn_config(PathBuf::from("/tmp"));

        assert_eq!(
            ClaudeCodeAdapter::conversation_id(&spawn_config),
            spawn_config.session_id.to_string(),
            "Panoptes and Claude must agree on the conversation UUID"
        );
    }

    #[test]
    fn test_conversation_id_is_stable_across_resume() {
        let mut spawn_config = test_spawn_config(PathBuf::from("/tmp"));
        let original = ClaudeCodeAdapter::conversation_id(&spawn_config);

        // Resuming feeds the stored ID back in; it must round-trip unchanged
        spawn_config.resume = Some(original.clone());

        assert_eq!(ClaudeCodeAdapter::conversation_id(&spawn_config), original);
    }

    #[test]
    fn test_session_id_arg_is_a_valid_uuid() {
        // Claude rejects --session-id values that do not parse as UUIDs
        let spawn_config = test_spawn_config(PathBuf::from("/tmp"));
        let args = ClaudeCodeAdapter::conversation_args(&spawn_config);

        let value = &args[1];
        assert!(
            Uuid::parse_str(value).is_ok(),
            "expected a UUID, got {value:?}"
        );
    }

    #[test]
    fn test_generate_env_contains_session_id() {
        let adapter = ClaudeCodeAdapter::new();
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
        // No CLAUDE_CONFIG_DIR when not specified
        assert!(!env.contains_key("CLAUDE_CONFIG_DIR"));
    }

    #[test]
    fn test_generate_env_with_claude_config_dir() {
        let adapter = ClaudeCodeAdapter::new();
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            ..Config::default()
        };
        let session_id = Uuid::new_v4();
        let claude_config_path = PathBuf::from("/home/user/.claude-work");
        let spawn_config = SpawnConfig {
            session_id,
            session_name: "test".to_string(),
            working_dir: temp_dir.path().to_path_buf(),
            initial_prompt: None,
            rows: 24,
            cols: 80,
            claude_config_dir: Some(claude_config_path.clone()),
            codex_home: None,
            resume: None,
        };

        let env = adapter.generate_env(&config, &spawn_config);
        assert_eq!(
            env.get("CLAUDE_CONFIG_DIR"),
            Some(&claude_config_path.to_string_lossy().to_string())
        );
    }

    #[test]
    fn test_generate_hook_script_content() {
        let script = ClaudeCodeAdapter::generate_hook_script(9999);
        assert!(script.contains("#!/bin/bash"));
        assert!(script.contains("PANOPTES_SESSION_ID"));
        assert!(script.contains("http://127.0.0.1:9999/hook"));
        assert!(script.contains("curl"));
        // The payload must be built by jq, never by shell interpolation
        assert!(script.contains("jq -c"));
        assert!(!script.contains("<<EOF"));
    }

    /// Locate a binary the hook script shells out to
    #[cfg(unix)]
    fn which(name: &str) -> PathBuf {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
            .unwrap_or_else(|| panic!("{} must exist to test the hook script", name))
    }

    /// Run the generated hook script and capture what it would have POSTed.
    ///
    /// The script runs against a sealed PATH containing only a fake `curl`
    /// (which writes its `-d` argument to a file) and the handful of binaries
    /// the script genuinely needs. `with_jq` decides whether `jq` is among
    /// them, which is how the degraded path gets exercised deterministically
    /// rather than depending on where jq happens to live on the host.
    #[cfg(unix)]
    fn run_hook_script(event: &str, stdin: &str, with_jq: bool) -> String {
        use std::process::{Command, Stdio};

        let temp = TempDir::new().unwrap();
        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let capture = temp.path().join("captured.json");

        let mut needed = vec!["basename", "date", "cat"];
        if with_jq {
            needed.push("jq");
        }
        for tool in needed {
            std::os::unix::fs::symlink(which(tool), bin.join(tool)).unwrap();
        }

        let fake_curl = format!(
            "#!/bin/bash\nwhile [ $# -gt 0 ]; do\n  if [ \"$1\" = \"-d\" ]; then printf '%s' \"$2\" > {}; fi\n  shift\ndone\nexit 0\n",
            capture.display()
        );
        let curl_path = bin.join("curl");
        std::fs::write(&curl_path, fake_curl).unwrap();
        std::fs::set_permissions(&curl_path, std::fs::Permissions::from_mode(0o755)).unwrap();

        // The script derives the event name from its own filename
        let script_path = temp.path().join(format!("{}.sh", event));
        std::fs::write(&script_path, ClaudeCodeAdapter::generate_hook_script(9999)).unwrap();
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut child = Command::new(&script_path)
            .env("PATH", bin.display().to_string())
            .env(
                "PANOPTES_SESSION_ID",
                "11111111-2222-3333-4444-555555555555",
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("hook script should be executable");
        std::io::Write::write_all(child.stdin.as_mut().unwrap(), stdin.as_bytes()).unwrap();
        drop(child.stdin.take());
        child.wait().unwrap();

        // curl is backgrounded, so the script exits before it has written
        for _ in 0..200 {
            if let Ok(body) = std::fs::read_to_string(&capture) {
                if !body.is_empty() {
                    return body;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!(
            "hook script never posted a payload (capture={}, exists={})",
            capture.display(),
            capture.exists()
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_hook_script_survives_quotes_and_newlines_in_payload() {
        // The exact defect this replaced: the old script interpolated an
        // extracted tool name into a here-doc, so any quote or newline in a
        // field produced malformed JSON and the event was silently dropped.
        let hostile = serde_json::json!({
            "session_id": "claude-own-id",
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_use_id": "toolu_01",
            "tool_input": {
                "command": "echo \"hi\" && printf 'a\nb'",
                "description": "quotes \" backslashes \\ and {braces}"
            }
        })
        .to_string();

        let posted = run_hook_script("PreToolUse", &hostile, true);
        let parsed: serde_json::Value =
            serde_json::from_str(&posted).expect("posted body must be valid JSON");

        // Panoptes' own routing fields, not Claude's
        assert_eq!(parsed["session_id"], "11111111-2222-3333-4444-555555555555");
        assert_eq!(parsed["event"], "PreToolUse");
        assert!(parsed["timestamp"].is_i64());

        // Claude's payload arrives intact, nested so its own session_id cannot
        // collide with ours
        assert_eq!(parsed["payload"]["session_id"], "claude-own-id");
        assert_eq!(parsed["payload"]["tool_name"], "Bash");
        assert_eq!(parsed["payload"]["tool_use_id"], "toolu_01");
        assert_eq!(
            parsed["payload"]["tool_input"]["command"],
            "echo \"hi\" && printf 'a\nb'"
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_hook_script_reads_multiline_json() {
        // `read -r` would have stopped at the first newline
        let pretty = "{\n  \"tool_name\": \"Read\",\n  \"tool_use_id\": \"toolu_02\"\n}";
        let posted = run_hook_script("PostToolUse", pretty, true);
        let parsed: serde_json::Value = serde_json::from_str(&posted).unwrap();

        assert_eq!(parsed["payload"]["tool_use_id"], "toolu_02");
    }

    #[test]
    #[cfg(unix)]
    fn test_hook_script_degrades_without_jq() {
        // With no jq the agent payload cannot be embedded safely. The envelope
        // must still be valid JSON so state tracking keeps working.
        let posted = run_hook_script("Stop", r#"{"last_assistant_message":"do\"ne"}"#, false);
        let parsed: serde_json::Value =
            serde_json::from_str(&posted).expect("degraded body must still be valid JSON");

        assert_eq!(parsed["session_id"], "11111111-2222-3333-4444-555555555555");
        assert_eq!(parsed["event"], "Stop");
        assert!(
            parsed.get("payload").is_none(),
            "no payload is better than a corrupt one"
        );
    }

    #[test]
    fn test_registered_hook_events_cover_the_state_model() {
        // Each of these feeds a transition the state model depends on; losing
        // one silently degrades state tracking rather than failing loudly.
        for required in [
            HookEventType::SessionStart,
            HookEventType::SessionEnd,
            HookEventType::UserPromptSubmit,
            HookEventType::PreToolUse,
            HookEventType::PostToolUse,
            HookEventType::PostToolUseFailure,
            HookEventType::Stop,
            HookEventType::StopFailure,
            HookEventType::Notification,
            HookEventType::PermissionRequest,
            HookEventType::PermissionDenied,
            HookEventType::SubagentStart,
            HookEventType::SubagentStop,
            HookEventType::Elicitation,
            HookEventType::ElicitationResult,
        ] {
            assert!(
                HOOK_EVENTS.contains(&required),
                "{} must be registered",
                required
            );
        }
    }

    #[test]
    fn test_install_hook_script() {
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            ..Config::default()
        };

        let event_scripts = ClaudeCodeAdapter::install_hook_script(&config).unwrap();

        // Verify base script was created
        let base_script = config.hooks_dir.join(HOOK_SCRIPT_NAME);
        assert!(base_script.exists());

        // Verify symlinks were created for each event type
        for event in HOOK_EVENTS {
            let event_name = event.as_str();
            let symlink = event_scripts
                .iter()
                .find(|(e, _)| e == event)
                .map(|(_, path)| path)
                .expect("Should have event script");
            assert!(symlink.exists() || symlink.is_symlink());
            assert!(symlink.ends_with(format!("{}.sh", event_name)));
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

        // An install from before the newer events were registered has no
        // symlink for them. Installing runs on every spawn, so the next spawn
        // must fill them in.
        for event in ["StopFailure", "SubagentStart", "ElicitationResult"] {
            std::fs::remove_file(config.hooks_dir.join(format!("{}.sh", event))).unwrap();
        }
        ClaudeCodeAdapter::install_hook_script(&config).unwrap();
        for event in HOOK_EVENTS {
            let symlink = config.hooks_dir.join(format!("{}.sh", event.as_str()));
            assert!(symlink.is_symlink(), "{} must be re-created", event);
        }
    }

    #[test]
    fn test_hooks_settings_json_structure() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path().to_path_buf();

        let event_scripts = mock_event_scripts();

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
        assert!(hooks.get("PermissionRequest").is_some());
        assert!(hooks.get("Stop").is_some());

        // Every registered event gets its own entry, pointing at its own
        // symlink - the newer events included
        for event in HOOK_EVENTS {
            let command = hooks[event.as_str()][0]["hooks"][0]["command"]
                .as_str()
                .unwrap_or_else(|| panic!("{} must be registered", event));
            assert!(command.ends_with(&format!("/{}.sh", event.as_str())));
        }
        for event in [
            "StopFailure",
            "PermissionDenied",
            "SubagentStart",
            "SubagentStop",
            "Elicitation",
            "ElicitationResult",
        ] {
            assert!(hooks.get(event).is_some(), "{} must be registered", event);
        }
        // To-do list events are not background work, and stay unregistered
        assert!(hooks.get("TaskCreated").is_none());
        assert!(hooks.get("TaskCompleted").is_none());
    }

    #[test]
    fn test_create_session_settings() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path().to_path_buf();

        let event_scripts = mock_event_scripts();

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
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            ..Config::default()
        };
        let spawn_config = test_spawn_config(temp_dir.path().to_path_buf());

        let adapter = ClaudeCodeAdapter::new();
        let cleanup_paths = adapter.setup_hooks(&config, &spawn_config).unwrap();

        // Should return the settings file path for cleanup
        assert!(!cleanup_paths.is_empty());
        assert!(cleanup_paths[0].ends_with("settings.local.json"));
        assert!(cleanup_paths[0].exists());
    }

    #[test]
    fn test_create_session_settings_preserves_existing() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path().to_path_buf();

        // Create existing settings with trust and other fields
        let claude_dir = working_dir.join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let existing_settings = r#"{
            "enabledBetaFeatures": ["feature1", "feature2"],
            "hasTrustDialogAccepted": true,
            "customSetting": 42
        }"#;
        std::fs::write(claude_dir.join("settings.local.json"), existing_settings).unwrap();

        let event_scripts = mock_event_scripts();

        // Create session settings (should merge hooks, not overwrite)
        let settings_path =
            ClaudeCodeAdapter::create_session_settings(&working_dir, &event_scripts).unwrap();

        // Read the resulting settings
        let content = std::fs::read_to_string(&settings_path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Verify existing settings are preserved
        assert!(settings["hasTrustDialogAccepted"].as_bool().unwrap());
        assert_eq!(settings["customSetting"].as_i64().unwrap(), 42);
        let features = settings["enabledBetaFeatures"].as_array().unwrap();
        assert_eq!(features.len(), 2);
        assert!(features.iter().any(|v| v.as_str() == Some("feature1")));
        assert!(features.iter().any(|v| v.as_str() == Some("feature2")));

        // Verify hooks were added
        assert!(settings.get("hooks").is_some());
        let hooks = settings["hooks"].as_object().unwrap();
        assert!(hooks.contains_key("PreToolUse"));
        assert!(hooks.contains_key("PostToolUse"));
        assert!(hooks.contains_key("Stop"));
        assert!(hooks.contains_key("Notification"));
    }

    #[test]
    fn test_create_session_settings_creates_backup() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path().to_path_buf();

        // Create existing settings
        let claude_dir = working_dir.join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let original_content = r#"{"original": true}"#;
        std::fs::write(claude_dir.join("settings.local.json"), original_content).unwrap();

        let event_scripts = mock_event_scripts();

        // Create session settings
        ClaudeCodeAdapter::create_session_settings(&working_dir, &event_scripts).unwrap();

        // Verify backup was created
        let backup_path = claude_dir.join("settings.local.json.bak");
        assert!(backup_path.exists());

        // Verify backup has original content
        let backup_content = std::fs::read_to_string(&backup_path).unwrap();
        let backup_json: serde_json::Value = serde_json::from_str(&backup_content).unwrap();
        assert!(backup_json["original"].as_bool().unwrap());
    }

    #[test]
    fn test_create_session_settings_handles_invalid_json() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path().to_path_buf();

        // Create existing settings with invalid JSON
        let claude_dir = working_dir.join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(claude_dir.join("settings.local.json"), "not valid json").unwrap();

        let event_scripts = mock_event_scripts();

        // Create session settings (should start fresh when JSON is invalid)
        let settings_path =
            ClaudeCodeAdapter::create_session_settings(&working_dir, &event_scripts).unwrap();

        // Read the resulting settings
        let content = std::fs::read_to_string(&settings_path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Verify hooks were added (fresh settings object)
        assert!(settings.get("hooks").is_some());
    }

    // The status line: which one the user has, and the wrapper around it

    /// Write a settings file with the given `statusLine`, or none
    fn settings_file(path: &Path, status_line: Option<serde_json::Value>) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut settings = serde_json::json!({"model": "opus"});
        if let Some(status_line) = status_line {
            settings["statusLine"] = status_line;
        }
        std::fs::write(path, settings.to_string()).unwrap();
    }

    fn command_setting(command: &str) -> serde_json::Value {
        serde_json::json!({"type": "command", "command": command})
    }

    /// The command a resolved status line runs, and whether it is local
    fn resolved(
        local: &serde_json::Value,
        project: &Path,
        user: Option<&Path>,
    ) -> Option<(String, bool)> {
        ClaudeCodeAdapter::resolve_user_status_line(local, project, user)
            .map(|found| (found.command, found.local))
    }

    #[test]
    fn test_resolves_effective_user_status_line() {
        let dir = TempDir::new().unwrap();
        let project = dir.path().join("repo/.claude/settings.json");
        // A profile's CLAUDE_CONFIG_DIR is where the user layer lives
        let config_dir = dir.path().join("claude-work");
        let user = ClaudeCodeAdapter::user_settings_path(Some(&config_dir)).expect("a user layer");
        assert_eq!(user, config_dir.join("settings.json"));
        let user = Some(user.as_path());
        let no_local = serde_json::json!({});
        let ours = Path::new("/elsewhere/hooks/panoptes-statusline.sh");

        // Nothing anywhere, and missing files are not an error
        assert_eq!(resolved(&no_local, &project, user), None);

        // User layer only
        settings_file(
            &config_dir.join("settings.json"),
            Some(command_setting("user-sl")),
        );
        assert_eq!(
            resolved(&no_local, &project, user),
            Some(("user-sl".to_string(), false))
        );

        // The project beats the user
        settings_file(&project, Some(command_setting("project-sl")));
        assert_eq!(
            resolved(&no_local, &project, user),
            Some(("project-sl".to_string(), false))
        );

        // And the local file beats both
        let local = serde_json::json!({"statusLine": command_setting("local-sl")});
        assert_eq!(
            resolved(&local, &project, user),
            Some(("local-sl".to_string(), true))
        );

        // Panoptes' own command wrapping a lower layer's is seen through, and
        // that layer is read afresh - here it has changed since
        let stale = UserStatusLine {
            setting: command_setting("old-project-sl"),
            command: "old-project-sl".to_string(),
            local: false,
        };
        let local = serde_json::json!({"statusLine": command_setting(
            &ClaudeCodeAdapter::status_line_command(ours, Some(&stale))
        )});
        assert_eq!(
            resolved(&local, &project, user),
            Some(("project-sl".to_string(), false))
        );

        // One wrapping the local file's own command gives it back
        let original = UserStatusLine {
            setting: command_setting("local-sl"),
            command: "local-sl".to_string(),
            local: true,
        };
        let local = serde_json::json!({"statusLine": command_setting(
            &ClaudeCodeAdapter::status_line_command(ours, Some(&original))
        )});
        assert_eq!(
            resolved(&local, &project, user),
            Some(("local-sl".to_string(), true))
        );

        // A bare wrapper of ours, in any layer, is not a user status line
        let bare = command_setting(&ClaudeCodeAdapter::status_line_command(ours, None));
        settings_file(&project, Some(bare.clone()));
        let local = serde_json::json!({ "statusLine": bare });
        assert_eq!(
            resolved(&local, &project, user),
            Some(("user-sl".to_string(), false))
        );

        // Something Claude would not run is skipped, not wrapped
        settings_file(
            &project,
            Some(serde_json::json!({"type": "static", "text": "x"})),
        );
        let local = serde_json::json!({"statusLine": {"type": "command", "command": "  "}});
        assert_eq!(
            resolved(&local, &project, user),
            Some(("user-sl".to_string(), false))
        );

        // The user's other options travel with the command
        settings_file(
            &config_dir.join("settings.json"),
            Some(
                serde_json::json!({"type": "command", "command": "user-sl", "padding": 2, "refreshInterval": 10}),
            ),
        );
        let found = ClaudeCodeAdapter::resolve_user_status_line(&no_local, &project, user).unwrap();
        assert_eq!(found.setting["padding"], 2);
        assert_eq!(found.setting["refreshInterval"], 10);
    }

    #[test]
    fn test_own_status_line_command_round_trips() {
        let script = Path::new("/it's a \"dir\" $HOME/panoptes-statusline.sh");
        let user = |command: &str, local: bool| UserStatusLine {
            setting: command_setting(command),
            command: command.to_string(),
            local,
        };
        for (found, expected) in [
            (None, (false, None)),
            (
                Some(user("echo \"it's\" $(id) `x` --local", false)),
                (false, Some("echo \"it's\" $(id) `x` --local".to_string())),
            ),
            (
                Some(user("--local", true)),
                (true, Some("--local".to_string())),
            ),
        ] {
            let command = ClaudeCodeAdapter::status_line_command(script, found.as_ref());
            assert_eq!(
                ClaudeCodeAdapter::parse_own_status_line(&command),
                Some(expected),
                "for {command:?}"
            );
        }

        // Anybody else's command is not ours, even one naming the script
        for other in [
            "~/.claude/statusline.sh",
            "/x/panoptes-statusline.sh",
            "'/x/panoptes-statusline.sh' 'a' 'b'",
            "'/x/not-panoptes-statusline.sh'",
        ] {
            assert_eq!(
                ClaudeCodeAdapter::parse_own_status_line(other),
                None,
                "{other}"
            );
        }
    }

    #[test]
    fn test_status_line_install_and_restore() {
        let dir = TempDir::new().unwrap();
        let working_dir = dir.path().join("repo");
        let local_path = working_dir.join(".claude/settings.local.json");
        let script = dir.path().join("hooks/panoptes-statusline.sh");
        let user_settings = dir.path().join("claude/settings.json");
        let install = StatusLinePlan::Install {
            script: script.clone(),
            user_settings: Some(user_settings.clone()),
        };
        let written = || -> serde_json::Value {
            serde_json::from_str(&std::fs::read_to_string(&local_path).unwrap()).unwrap()
        };
        let write = |plan: &StatusLinePlan| {
            ClaudeCodeAdapter::write_session_settings(&working_dir, &mock_event_scripts(), plan)
                .unwrap();
        };

        // The user's own local status line, with an option of its own
        settings_file(
            &local_path,
            Some(serde_json::json!({"type": "command", "command": "my-sl", "padding": 1})),
        );
        settings_file(&user_settings, Some(command_setting("user-sl")));

        write(&install);
        let first = written();
        assert_eq!(first["statusLine"]["padding"], 1);
        assert_eq!(first["statusLine"]["type"], "command");
        let command = first["statusLine"]["command"].as_str().unwrap();
        assert_eq!(
            ClaudeCodeAdapter::parse_own_status_line(command),
            Some((true, Some("my-sl".to_string())))
        );
        assert!(first["hooks"].get("Stop").is_some());
        assert_eq!(first["model"], "opus");

        // A second session in the same directory writes the same thing rather
        // than wrapping the wrapper
        write(&install);
        assert_eq!(written(), first);

        // Turned off, the user's own local status line comes back
        write(&StatusLinePlan::Restore);
        assert_eq!(
            written()["statusLine"],
            serde_json::json!({"type": "command", "command": "my-sl", "padding": 1})
        );

        // Wrapping the user layer's instead, restore removes the key: the user
        // layer shows through again on its own
        settings_file(&local_path, None);
        write(&install);
        assert_eq!(
            ClaudeCodeAdapter::parse_own_status_line(
                written()["statusLine"]["command"].as_str().unwrap()
            ),
            Some((false, Some("user-sl".to_string())))
        );
        write(&StatusLinePlan::Restore);
        assert!(written().get("statusLine").is_none());

        // And restoring when Panoptes never wrapped anything touches nothing
        settings_file(&local_path, Some(command_setting("mine")));
        write(&StatusLinePlan::Restore);
        assert_eq!(written()["statusLine"], command_setting("mine"));
    }

    #[test]
    fn test_setup_hooks_honours_the_status_line_switch() {
        let temp_dir = TempDir::new().unwrap();
        let mut config = Config {
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            ..Config::default()
        };
        let mut spawn_config = test_spawn_config(temp_dir.path().join("repo"));
        // Keep the real user layer out of it
        spawn_config.claude_config_dir = Some(temp_dir.path().join("claude"));
        let adapter = ClaudeCodeAdapter::new();
        let status_line = |paths: &[PathBuf]| -> Option<serde_json::Value> {
            let settings: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&paths[0]).unwrap()).unwrap();
            settings.get("statusLine").cloned()
        };

        assert!(config.claude_status_line, "on by default");
        let paths = adapter.setup_hooks(&config, &spawn_config).unwrap();
        let installed = status_line(&paths).expect("a status line");
        let script = config.hooks_dir.join(STATUS_LINE_SCRIPT_NAME);
        assert!(script.is_file());
        assert_eq!(
            installed["command"].as_str(),
            Some(super::super::shell_quote(&script.to_string_lossy()).as_str())
        );

        config.claude_status_line = false;
        let paths = adapter.setup_hooks(&config, &spawn_config).unwrap();
        assert_eq!(status_line(&paths), None);
    }

    // The status-line wrapper, executed
    //
    // Run the way Claude runs it: the settings' command string handed to
    // `bash -c`, the payload on stdin, stdout read to EOF.

    /// What one run of the wrapper did
    #[cfg(unix)]
    struct StatusLineRun {
        /// Everything the wrapper printed
        stdout: String,
        /// Whether it exited successfully
        success: bool,
        /// Whether stdout closed while `curl` was still held (see
        /// [`run_status_line`]), i.e. Claude was not kept waiting on it
        finished_before_post: bool,
        /// `curl`'s arguments, if it was called
        posted: Option<Vec<String>>,
    }

    /// Run the wrapper at `script` in front of `user`'s command, in `dir`
    ///
    /// The PATH is sealed: a fake `curl` records its arguments to a file, and
    /// only the binaries the wrapper and stubs genuinely need are there. With
    /// `hold_curl` the fake stands in for a server that has not answered: it
    /// records nothing until the wrapper's stdout has closed and the test
    /// releases it. That is a sequence rather than a stopwatch, so a loaded
    /// machine cannot flake it.
    #[cfg(unix)]
    fn run_status_line(
        dir: &Path,
        script: &Path,
        user: Option<&UserStatusLine>,
        session_id: Option<&str>,
        hold_curl: bool,
    ) -> StatusLineRun {
        use std::process::{Command, Stdio};

        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for tool in ["bash", "cat", "date", "sleep", "seq"] {
            let link = bin.join(tool);
            if !link.exists() {
                std::os::unix::fs::symlink(which(tool), link).unwrap();
            }
        }
        let capture = dir.join("curl.args");
        let release = dir.join("curl.release");
        let _ = std::fs::remove_file(&capture);
        let _ = std::fs::remove_file(&release);
        if !hold_curl {
            std::fs::write(&release, "").unwrap();
        }
        // Held, it gives up after a minute so a failed test leaves nothing
        let fake_curl = format!(
            "#!/bin/bash\nfor _ in $(seq 1200); do [ -e {release} ] && break; sleep 0.05; done\nfor arg in \"$@\"; do printf '%s\\0' \"$arg\"; done > {capture}\n",
            release = super::super::shell_quote(&release.to_string_lossy()),
            capture = super::super::shell_quote(&capture.to_string_lossy()),
        );
        super::super::install_executable_script(&bin.join("curl"), &fake_curl).unwrap();

        super::super::install_executable_script(
            script,
            &ClaudeCodeAdapter::generate_status_line_script("http://127.0.0.1:1/hook"),
        )
        .unwrap();

        let command = ClaudeCodeAdapter::status_line_command(script, user);
        let payload = dir.join("payload.json");
        std::fs::write(
            &payload,
            include_str!("../../tests/fixtures/claude_status_line.json"),
        )
        .unwrap();
        let stdout = dir.join("stdout");

        // The pipe the wrapper writes to is made by this driver, not by the
        // test process: `cat` reads it to EOF, as Claude does, and a pipe
        // opened in a single-threaded shell cannot leak into another test's
        // child and be held open by it
        let mut cmd = Command::new(which("bash"));
        cmd.arg("-c")
            .arg(r#"set -o pipefail; bash -c "$1" < "$2" | cat > "$3""#)
            .arg("driver")
            .arg(&command)
            .arg(&payload)
            .arg(&stdout)
            .env_clear()
            .env("PATH", bin.display().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(id) = session_id {
            cmd.env("PANOPTES_SESSION_ID", id);
        }

        let status = cmd.status().expect("run status line");
        let finished_before_post = !capture.exists();
        std::fs::write(&release, "").unwrap();

        // curl is backgrounded, so it may still be on its way. Generous,
        // because a loaded machine can take seconds to start a process;
        // a run that should post nothing gets a shorter look.
        let started = std::time::Instant::now();
        let mut posted = None;
        for _ in 0..2_000 {
            if let Ok(bytes) = std::fs::read(&capture) {
                if !bytes.is_empty() {
                    posted = Some(
                        String::from_utf8(bytes)
                            .unwrap()
                            .split_terminator('\0')
                            .map(str::to_string)
                            .collect(),
                    );
                    break;
                }
            }
            let should_post = session_id.is_some_and(|id| Uuid::parse_str(id).is_ok());
            if !should_post && started.elapsed().as_millis() > 500 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        StatusLineRun {
            stdout: std::fs::read_to_string(&stdout).unwrap(),
            success: status.success(),
            finished_before_post,
            posted,
        }
    }

    /// A stub user status line that echoes a marker and records its stdin
    #[cfg(unix)]
    fn stub_user_status_line(path: &Path, seen: &Path) -> UserStatusLine {
        let stub = format!(
            "#!/bin/bash\ncat > {}\nprintf 'MARKER \"%s\" line\\nsecond line' \"$1\"\n",
            super::super::shell_quote(&seen.to_string_lossy())
        );
        super::super::install_executable_script(path, &stub).unwrap();
        // A shell command line, as a user would write it, with an argument
        // that needs its quotes
        let command = format!(
            "{} \"it's \\$HOME\"",
            super::super::shell_quote(&path.to_string_lossy())
        );
        UserStatusLine {
            setting: command_setting(&command),
            command,
            local: false,
        }
    }

    const SESSION: &str = "11111111-2222-3333-4444-555555555555";

    #[test]
    #[cfg(unix)]
    fn test_status_line_wrapper_prints_the_users_output_and_posts() {
        let dir = TempDir::new().unwrap();
        // Both scripts live where a shell would split, expand and unquote
        let hostile = dir.path().join("it's a \"dir\" $HOME `id`");
        let script = hostile.join("hooks").join(STATUS_LINE_SCRIPT_NAME);
        let seen = dir.path().join("seen.json");
        let user = stub_user_status_line(&hostile.join("my status.sh"), &seen);

        for local in [false, true] {
            let user = UserStatusLine {
                local,
                ..user.clone()
            };
            let run = run_status_line(dir.path(), &script, Some(&user), Some(SESSION), false);

            assert!(run.success);
            assert_eq!(run.stdout, "MARKER \"it's $HOME\" line\nsecond line");
            // The user's command saw the very document Claude sent
            let seen: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&seen).unwrap()).unwrap();
            assert_eq!(seen["rate_limits"]["five_hour"]["used_percentage"], 21);

            let posted = run.posted.expect("the payload was posted");
            assert!(posted.contains(&"http://127.0.0.1:1/hook".to_string()));
            assert!(posted.contains(&"--max-time".to_string()));
            let body = &posted[posted.iter().position(|a| a == "-d").unwrap() + 1];
            let envelope: crate::hooks::HookEvent =
                serde_json::from_str(body).expect("the envelope is valid JSON");
            assert_eq!(envelope.session_id, SESSION);
            assert_eq!(envelope.event_type(), HookEventType::StatusLine);
            assert!(envelope.timestamp > 0);
            let usage = crate::hooks::status_line::usage_from_payload(&envelope.payload)
                .expect("the payload arrives intact");
            assert_eq!(usage.primary.unwrap().used_percent, 21.0);
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_status_line_wrapper_without_a_user_command_prints_nothing() {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("hooks").join(STATUS_LINE_SCRIPT_NAME);

        let run = run_status_line(dir.path(), &script, None, Some(SESSION), false);

        // Claude's own default is no status line at all
        assert!(run.success);
        assert_eq!(run.stdout, "");
        assert!(run.posted.is_some(), "the figures are still forwarded");
    }

    #[test]
    #[cfg(unix)]
    fn test_status_line_wrapper_does_not_wait_for_the_post() {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("hooks").join(STATUS_LINE_SCRIPT_NAME);
        let seen = dir.path().join("seen.json");
        let user = stub_user_status_line(&dir.path().join("sl.sh"), &seen);

        // A server that has not answered must not hold up the status line
        let run = run_status_line(dir.path(), &script, Some(&user), Some(SESSION), true);

        assert!(run.stdout.starts_with("MARKER"));
        assert!(
            run.finished_before_post,
            "the status line waited for the POST"
        );
        assert!(run.posted.is_some(), "and the POST still went out");
    }

    #[test]
    #[cfg(unix)]
    fn test_status_line_wrapper_outside_panoptes_only_runs_the_user_command() {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("hooks").join(STATUS_LINE_SCRIPT_NAME);
        let seen = dir.path().join("seen.json");
        let user = stub_user_status_line(&dir.path().join("sl.sh"), &seen);

        // Left behind in settings.local.json, then run by a plain `claude`
        let run = run_status_line(dir.path(), &script, Some(&user), None, false);
        assert!(run.stdout.starts_with("MARKER"));
        assert_eq!(run.posted, None);

        // A session ID that is not one of ours is not spliced into JSON
        let run = run_status_line(dir.path(), &script, Some(&user), Some("x\",\"y"), false);
        assert!(run.stdout.starts_with("MARKER"));
        assert_eq!(run.posted, None);
    }

    #[test]
    #[cfg(unix)]
    fn test_status_line_wrapper_passes_the_users_exit_status() {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("hooks").join(STATUS_LINE_SCRIPT_NAME);
        let failing = UserStatusLine {
            setting: command_setting("echo partial; exit 3"),
            command: "echo partial; exit 3".to_string(),
            local: false,
        };

        let run = run_status_line(dir.path(), &script, Some(&failing), Some(SESSION), false);
        assert!(!run.success);
        assert_eq!(run.stdout, "partial\n");
    }
}
