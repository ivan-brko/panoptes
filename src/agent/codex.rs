//! OpenAI Codex CLI adapter implementation
//!
//! This module implements the `AgentAdapter` trait for OpenAI Codex CLI.
//! It handles hook installation and process spawning.
//!
//! Codex 0.156.1 and later has Claude-style lifecycle hooks. Panoptes declares
//! them for each spawn on the command line (`-c hooks.<Event>=...`), together
//! with the trust Codex requires before it will run them, so nothing is
//! written into the user's `CODEX_HOME` - see [`lifecycle_hook_args`].
//! Older Codex versions get the `notify` hook in `config.toml` instead, which
//! fires only on `agent-turn-complete`; there the rollout file supplies the
//! rest of the session's state.

use crate::config::Config;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::adapter::{AgentAdapter, SpawnConfig, SpawnResult};
use super::claude::ClaudeCodeAdapter;
use crate::session::PtyHandle;
use crate::transcript::codex::{read_session_meta, rollout_files, RolloutKind};

/// Notify hook script filename
const CODEX_NOTIFY_SCRIPT_NAME: &str = "codex-notify.sh";
/// `$0` for the `bash -c` command that chains Panoptes' notify hook in front
/// of the user's, so the event Codex appends lands in `$1` rather than `$0`
const CHAIN_ARGV0: &str = "panoptes-notify";
/// What older Panoptes versions wrote for a `'` inside a single-quoted word
///
/// It does not round-trip through a shell - `it's` came out as `it"\"s` -
/// which is one reason those chains are repaired. Kept only to recognise and
/// decode them.
const LEGACY_QUOTE_ESCAPE: &str = r#"'\"'\"'"#;
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
        // rather than the conversation the user was having. Codex's own
        // background threads (memory consolidation, the guardian) are no
        // better a thing to reattach to, so only a real session qualifies.
        if meta.kind != RolloutKind::Session {
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
    candidates.sort_by_key(|a| a.0);
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

/// When a known Codex conversation's rollout was created
///
/// `None` while the file does not exist yet, or when it records no creation
/// time.
pub fn rollout_created_at(
    codex_home: &Path,
    conversation_id: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    read_session_meta(&rollout_path(codex_home, conversation_id)?)?.created_at
}

/// The first Codex whose lifecycle hooks Panoptes has verified end to end
///
/// What matters is not that hooks exist but that [`hook_trust_hash`] matches
/// Codex's own hash. A Codex that hashes differently would show every
/// Panoptes hook as "modified" and put its hook review screen in front of
/// every spawn, so an older Codex keeps `notify` rather than risk that.
const LIFECYCLE_HOOKS_MIN_VERSION: CodexVersion = CodexVersion(0, 156, 1);

/// Subdirectory of the hooks directory holding one symlink per Codex event
///
/// Separate from Claude's symlinks (which share the directory's top level)
/// so neither adapter's install can disturb the other's.
const CODEX_HOOKS_SUBDIR: &str = "codex";

/// Handler timeout Panoptes declares, in seconds
///
/// Declared rather than defaulted because it is part of the hash Codex trusts
/// (see [`hook_trust_hash`]), and 3 is the most Codex allows for `SessionEnd`
/// and `Interrupt`, so one value serves every event unchanged. The script
/// returns in milliseconds; this only bounds a pathological hang.
const LIFECYCLE_HOOK_TIMEOUT_SECS: u64 = 3;

/// How Codex names hooks declared through `-c` when it keys their trust state
///
/// A fixed, synthetic path - Codex's own placeholder for the `-c` layer - not
/// a file under `CODEX_HOME`. Neither the key nor the hash
/// ([`hook_trust_hash`]) involves `CODEX_HOME` at all, so the same overrides
/// stay trusted whichever home, real or a Panoptes-made shadow, Codex runs
/// against.
const SESSION_FLAGS_KEY_SOURCE: &str = "/<session-flags>/config.toml";

/// The Codex hook events Panoptes registers, with the label Codex uses for
/// each in trust keys and hashes
///
/// `Interrupt` has no Claude counterpart but is needed all the same: Codex
/// reports a turn the user cut short with nothing else, and once the hooks
/// own a session's state nothing else would end it.
const LIFECYCLE_EVENTS: &[(&str, &str)] = &[
    ("SessionStart", "session_start"),
    ("SessionEnd", "session_end"),
    ("UserPromptSubmit", "user_prompt_submit"),
    ("PreToolUse", "pre_tool_use"),
    ("PostToolUse", "post_tool_use"),
    ("PermissionRequest", "permission_request"),
    ("Stop", "stop"),
    ("SubagentStart", "subagent_start"),
    ("SubagentStop", "subagent_stop"),
    ("Interrupt", "interrupt"),
];

/// How long a `codex --version` probe is believed
///
/// Long enough that spawning sessions does not keep paying for a process
/// launch, short enough that upgrading Codex does not need a Panoptes restart.
const VERSION_PROBE_TTL: std::time::Duration = std::time::Duration::from_secs(600);

/// A Codex release number, `major.minor.patch`
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CodexVersion(u64, u64, u64);

impl CodexVersion {
    /// Read the version out of `codex --version` output (`codex-cli 0.156.1`)
    ///
    /// A pre-release suffix (`0.157.0-alpha.3`) is dropped: it is the release
    /// it leads up to that decides what the build supports.
    fn parse(output: &str) -> Option<Self> {
        let word = output
            .split_whitespace()
            .find(|word| word.starts_with(|c: char| c.is_ascii_digit()))?;
        let core = word.split(['-', '+']).next()?;
        let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
        let version = CodexVersion(parts.next()??, parts.next()??, parts.next()??);
        Some(version)
    }
}

/// Which mechanism a spawn reports its state through
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookInstall {
    /// Lifecycle hooks declared on the command line; nothing written to disk
    /// outside Panoptes' own hooks directory
    Lifecycle,
    /// The `notify` hook in `CODEX_HOME/config.toml`, plus the rollout
    Notify,
}

impl HookInstall {
    /// The mechanism a Codex of this version supports
    ///
    /// An unknown version gets `notify`: the worst it costs is the detail
    /// hooks would have added, where guessing wrong the other way puts a
    /// trust prompt in front of every spawn.
    fn for_version(version: Option<CodexVersion>) -> Self {
        match version {
            Some(version) if version >= LIFECYCLE_HOOKS_MIN_VERSION => HookInstall::Lifecycle,
            _ => HookInstall::Notify,
        }
    }
}

/// Run `<command> --version`
fn probe_codex_version(command: &str) -> Option<CodexVersion> {
    let output = std::process::Command::new(command)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    CodexVersion::parse(&String::from_utf8_lossy(&output.stdout))
}

/// The installed Codex's version, probed at most once per [`VERSION_PROBE_TTL`]
fn cached_codex_version(command: &str) -> Option<CodexVersion> {
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    type Probe = (Instant, Option<CodexVersion>);
    static CACHE: OnceLock<Mutex<Option<Probe>>> = OnceLock::new();

    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut cached = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((probed_at, version)) = *cached {
        if probed_at.elapsed() < VERSION_PROBE_TTL {
            return version;
        }
    }
    let version = probe_codex_version(command);
    match version {
        Some(version) => tracing::debug!(?version, "Probed Codex version"),
        None => tracing::warn!("Could not read the Codex version; falling back to the notify hook"),
    }
    *cached = Some((Instant::now(), version));
    version
}

/// The command line Codex runs for one Panoptes hook
///
/// Codex hands the whole string to the user's shell, so the path is quoted.
fn lifecycle_hook_command(hooks_dir: &Path, event: &str) -> String {
    let script = hooks_dir
        .join(CODEX_HOOKS_SUBDIR)
        .join(format!("{event}.sh"));
    CodexAdapter::shell_quote(&script.to_string_lossy())
}

/// The hash Codex computes for a hook, which it compares against the hash it
/// was told to trust
///
/// A reproduction of Codex's `hook_hash`: SHA-256 over the canonical
/// (sorted-key, compact) JSON of the normalized declaration,
/// `{"event_name", "hooks": [<handler>]}`. `matcher` is absent because
/// Panoptes declares none, and every optional handler field Panoptes leaves
/// unset is omitted, as Codex omits it. The command *string* is hashed, not
/// the script it names, so reinstalling the script never needs re-trusting.
///
/// Written out by hand rather than serialized, so the key order cannot
/// depend on how `serde_json` happens to be built.
fn hook_trust_hash(event_label: &str, command: &str) -> String {
    use sha2::{Digest, Sha256};

    let identity = format!(
        r#"{{"event_name":{},"hooks":[{{"async":false,"command":{},"timeout":{},"type":"command"}}]}}"#,
        serde_json::Value::from(event_label),
        serde_json::Value::from(command),
        LIFECYCLE_HOOK_TIMEOUT_SECS,
    );
    let digest = Sha256::digest(identity.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("sha256:{hex}")
}

/// A string as a TOML basic string, for a `-c` override value
fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_string()).to_string()
}

/// The `-c` overrides that declare Panoptes' Codex hooks and trust them
///
/// One override per event declares a single command handler, then one more
/// sets `hooks.state`, carrying the hash of each declaration as
/// `trusted_hash`. Codex runs a hook only once its trusted hash matches what
/// it computes itself, and otherwise stops at startup to ask the user to
/// review it - on every spawn, since nothing here is saved.
///
/// This grants no trust beyond Panoptes' own hooks. Codex reads `hooks.state`
/// only from the user's config and from these overrides (never from a
/// project or plugin), and each key names a hook declared by the same
/// overrides, by its exact command and timeout. Hooks in the user's or a
/// project's `hooks.json` keep whatever trust they already had, and a new
/// one still prompts. `--dangerously-bypass-hook-trust`, which would switch
/// the check off for all of them, is deliberately not used.
///
/// Codex and the user's own hooks coexist: handlers from every layer run.
pub(crate) fn lifecycle_hook_args(hooks_dir: &Path) -> Vec<String> {
    let mut args = Vec::with_capacity(LIFECYCLE_EVENTS.len() * 2 + 2);
    let mut trusted = Vec::with_capacity(LIFECYCLE_EVENTS.len());

    for (event, label) in LIFECYCLE_EVENTS {
        let command = lifecycle_hook_command(hooks_dir, event);
        args.push("-c".to_string());
        args.push(format!(
            "hooks.{event}=[{{hooks=[{{type=\"command\",command={},timeout={}}}]}}]",
            toml_string(&command),
            LIFECYCLE_HOOK_TIMEOUT_SECS,
        ));
        // Codex keys a hook's state by source, event, and its position within
        // that source: each event here has exactly one group of one handler
        let key = format!("{SESSION_FLAGS_KEY_SOURCE}:{label}:0:0");
        trusted.push(format!(
            "{}={{trusted_hash={}}}",
            toml_string(&key),
            toml_string(&hook_trust_hash(label, &command)),
        ));
    }

    // `hooks.state` is one inline table: the keys contain dots, which a
    // dotted `-c` path would split on
    args.push("-c".to_string());
    args.push(format!("hooks.state={{{}}}", trusted.join(", ")));
    args
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

    /// Install the scripts Codex's lifecycle hooks run
    ///
    /// Codex's hook payload is Claude's - JSON on stdin, `hook_event_name`,
    /// `tool_name`, `tool_use_id` - so the Claude hook script serves both.
    /// Each event gets a symlink named after it, since the script takes the
    /// event name from its own basename.
    ///
    /// Touches only Panoptes' own hooks directory. A symlink already pointing
    /// at the script is left alone.
    fn install_lifecycle_hooks(config: &Config) -> Result<()> {
        let script_path = ClaudeCodeAdapter::hook_script_path(config);
        super::install_executable_script(
            &script_path,
            &ClaudeCodeAdapter::generate_hook_script(config.hook_port),
        )
        .context("Failed to install hook script")?;

        let dir = config.hooks_dir.join(CODEX_HOOKS_SUBDIR);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create {}", dir.display()))?;

        for (event, _) in LIFECYCLE_EVENTS {
            let link = dir.join(format!("{event}.sh"));
            if std::fs::read_link(&link).is_ok_and(|target| target == script_path) {
                continue;
            }
            if link.exists() || link.is_symlink() {
                std::fs::remove_file(&link)
                    .with_context(|| format!("Failed to replace {}", link.display()))?;
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(&script_path, &link)
                .with_context(|| format!("Failed to create symlink for {event}"))?;
        }
        Ok(())
    }

    /// Install whichever hooks this Codex supports, returning the extra
    /// arguments the spawn needs
    ///
    /// Lifecycle hooks travel on the command line; `notify` lives in
    /// `config.toml` and needs none.
    fn install_hooks(&self, config: &Config, spawn_config: &SpawnConfig) -> Result<Vec<String>> {
        let install = HookInstall::for_version(cached_codex_version(self.command()));
        Self::install_hooks_as(install, config, spawn_config)
    }

    /// [`Self::install_hooks`], for a mechanism already chosen
    fn install_hooks_as(
        install: HookInstall,
        config: &Config,
        spawn_config: &SpawnConfig,
    ) -> Result<Vec<String>> {
        match install {
            HookInstall::Lifecycle => {
                Self::install_lifecycle_hooks(config)?;
                Ok(lifecycle_hook_args(&config.hooks_dir))
            }
            HookInstall::Notify => {
                let notify_script_path = Self::install_notify_script(config)?;
                let codex_home = Self::resolve_codex_home(spawn_config);
                Self::configure_codex_notify(&codex_home, &notify_script_path)?;
                Ok(Vec::new())
            }
        }
    }

    /// The full argument list: hook overrides first, as root options that
    /// must precede a `resume` subcommand, then [`AgentAdapter::build_args`]
    fn command_line(&self, hook_args: Vec<String>, spawn_config: &SpawnConfig) -> Vec<String> {
        let mut args = hook_args;
        args.extend(self.build_args(spawn_config));
        args
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
# Codex spawns this hook directly (no shell) and appends the event JSON as
# the final argument, so the event is "${{@: -1}}". stdin is /dev/null and
# stdout/stderr are discarded. This hook ignores the event on purpose.
#
# CRITICAL: Do NOT use blocking stdin reads (e.g. `read -r`) in this script.
# Codex writes nothing to stdin; the event is on argv. Whatever runs this
# hook - a Codex version, or a wrapper chained in front of it - must never
# be left waiting on a stdin that may not be /dev/null, because a stalled
# hook has been seen to drop typed characters while Codex streams.

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
                if !Self::notify_command_mentions_script(&existing_cmd, script) {
                    let chained_cmd = Self::build_chained_notify_command(script, &existing_cmd);
                    return NotifyPlan::Set(Self::notify_array_value(&chained_cmd));
                }
                if Self::is_legacy_chain(&existing_cmd) {
                    // An older Panoptes chained with a command that dropped
                    // the event on the way to the user's hook. Rewrite it,
                    // but only if the user's argv comes back out exactly -
                    // a repair that guessed would silently change their hook.
                    return match Self::recover_legacy_chain(&existing_cmd, script) {
                        Some(user_cmd) => NotifyPlan::Set(Self::notify_array_value(
                            &Self::build_chained_notify_command(script, &user_cmd),
                        )),
                        None => NotifyPlan::Unsupported,
                    };
                }
                // Already chained through Panoptes (or merged by hand), avoid
                // duplicate wrapping — and avoid rewriting a file that needs
                // no change.
                NotifyPlan::AlreadyConfigured
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
            // Codex itself only accepts an argv array (`notify` is a
            // `Vec<String>` in its config), so a string cannot be run as-is.
            // Treat it as the command line it reads as, with Codex's
            // arguments appended: the event reaches it as it would an argv
            // hook. Plain `-c`, not `-lc` - Codex runs nothing through a
            // login shell, and sourcing the user's profile on every turn is
            // slow and leaks whatever the profile prints.
            toml::Value::String(shell_cmd) => Some(vec![
                "bash".to_string(),
                "-c".to_string(),
                format!("{shell_cmd} \"$@\""),
                CHAIN_ARGV0.to_string(),
            ]),
            _ => None,
        }
    }

    fn notify_command_mentions_script(cmd: &[String], script_path: &Path) -> bool {
        let script = script_path.to_string_lossy();
        cmd.iter().any(|part| part.contains(script.as_ref()))
    }

    /// Quote one word for a POSIX shell, so it arrives as exactly one argument
    fn shell_quote(arg: &str) -> String {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }

    /// Chain Panoptes' hook in front of the user's own
    ///
    /// Codex spawns `notify` directly and appends the event JSON as one final
    /// argument. Under `bash -c`, the first argument after the script is `$0`,
    /// not `$1`, so the chain supplies its own `$0` ([`CHAIN_ARGV0`]) and the
    /// event lands in `"$@"`, which both hooks are then given unchanged.
    ///
    /// The hooks are joined with `;`, not `&&`: the user's hook runs whether
    /// or not ours succeeds, so Panoptes can never suppress it.
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

        let script = format!("{panoptes_hook} \"$@\"; {existing} \"$@\"");
        vec![
            "bash".to_string(),
            "-c".to_string(),
            script,
            CHAIN_ARGV0.to_string(),
        ]
    }

    /// Whether `cmd` has the shape older Panoptes versions chained with
    ///
    /// That was `["bash", "-lc", "'<ours>' \"$@\"; '<theirs>'..."]`: a login
    /// shell, and no `$0` placeholder, so the event Codex appended became
    /// `$0` and neither hook received it. Only called on a command that
    /// already mentions our script.
    fn is_legacy_chain(cmd: &[String]) -> bool {
        matches!(cmd, [bash, flag, _] if bash == "bash" && flag == "-lc")
    }

    /// Recover the user's own argv from a legacy chain, or `None` if it
    /// cannot be recovered exactly
    ///
    /// The legacy chain quoted every word with [`LEGACY_QUOTE_ESCAPE`] for
    /// embedded single quotes. The words are decoded, then quoted again the
    /// old way; only a result that reproduces the stored command byte for
    /// byte is trusted. Anything hand-edited, truncated, or otherwise off that
    /// exact shape is refused rather than guessed at.
    fn recover_legacy_chain(cmd: &[String], script: &Path) -> Option<Vec<String>> {
        let [_, _, chain] = cmd else {
            return None;
        };
        let prefix = format!(
            "{} \"$@\"; ",
            Self::legacy_shell_quote(&script.to_string_lossy())
        );
        let user_words = chain.strip_prefix(&prefix)?;
        let user_cmd = Self::decode_legacy_quoted_words(user_words)?;

        let reencoded = user_cmd
            .iter()
            .map(|word| Self::legacy_shell_quote(word))
            .collect::<Vec<_>>()
            .join(" ");
        if reencoded != user_words {
            return None;
        }
        // The legacy writer never chained over a command naming our script,
        // so one that does was not written by it
        if Self::notify_command_mentions_script(&user_cmd, script) {
            return None;
        }
        Some(user_cmd)
    }

    /// How the legacy chain quoted a word (see [`LEGACY_QUOTE_ESCAPE`])
    fn legacy_shell_quote(arg: &str) -> String {
        format!("'{}'", arg.replace('\'', LEGACY_QUOTE_ESCAPE))
    }

    /// Split space-separated words quoted by [`Self::legacy_shell_quote`]
    ///
    /// Inside such a word the only `'` characters are its closing quote and
    /// the start of [`LEGACY_QUOTE_ESCAPE`], so the decoding is unambiguous.
    fn decode_legacy_quoted_words(mut rest: &str) -> Option<Vec<String>> {
        let mut words = Vec::new();
        loop {
            rest = rest.strip_prefix('\'')?;
            let mut word = String::new();
            loop {
                let quote = rest.find('\'')?;
                word.push_str(&rest[..quote]);
                rest = &rest[quote..];
                match rest.strip_prefix(LEGACY_QUOTE_ESCAPE) {
                    Some(after) => {
                        word.push('\'');
                        rest = after;
                    }
                    None => {
                        rest = &rest[1..];
                        break;
                    }
                }
            }
            words.push(word);
            if rest.is_empty() {
                return Some(words);
            }
            rest = rest.strip_prefix(' ')?;
        }
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

    fn generate_env(&self, config: &Config, spawn_config: &SpawnConfig) -> HashMap<String, String> {
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
        // A shadow home (shared history) keeps its state databases in the
        // shared home. Pointed there rather than linked, because Codex creates
        // some of them lazily and would otherwise create them in the shadow.
        if let Some(sqlite_home) =
            crate::codex_config::CodexHomes::from_config(config).sqlite_home_for(&codex_home)
        {
            env.insert(
                "CODEX_SQLITE_HOME".to_string(),
                sqlite_home.to_string_lossy().to_string(),
            );
        }
        env
    }

    fn setup_hooks(&self, config: &Config, spawn_config: &SpawnConfig) -> Result<Vec<PathBuf>> {
        // Lifecycle hooks also need arguments, which only `spawn` can pass;
        // this installs what is on disk
        self.install_hooks(config, spawn_config)?;

        // TODO: Codex permission sharing
        // When Codex supports per-project permissions (similar to Claude's
        // .claude/settings.local.json), implement copying from root branch
        // to worktree here. See check_claude_settings_for_copy() in
        // src/wizards/worktree/handlers.rs for the Claude implementation.

        // Nothing to clean up: the notify hook is harmless for non-Panoptes
        // instances, and lifecycle hooks exist only on this spawn's command line
        Ok(vec![])
    }

    /// Spawn Codex, with its lifecycle hooks on the command line when it has them
    ///
    /// Differs from the default only in where the hook arguments go: they are
    /// root options, so they lead - ahead of a `resume` subcommand.
    fn spawn(&self, config: &Config, spawn_config: &SpawnConfig) -> Result<SpawnResult> {
        let hook_args = self.install_hooks(config, spawn_config)?;
        let args = self.command_line(hook_args, spawn_config);
        let env = self.generate_env(config, spawn_config);
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

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

    /// Codex mints its own conversation ID. Its `SessionStart` hook reports it;
    /// without hooks it is discovered from the rollout (see
    /// [`discover_session_id`])
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
        assert_eq!(notify.len(), 4);
        assert_eq!(notify[0].as_str().unwrap(), "bash");
        assert_eq!(notify[1].as_str().unwrap(), "-c");
        let script = notify[2].as_str().unwrap();
        assert!(script.contains("/test/codex-notify.sh"));
        assert!(script.contains("'echo' 'legacy-hook' \"$@\""));
        assert_eq!(notify[3].as_str(), Some(CHAIN_ARGV0));

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
        assert_eq!(arr.len(), 4);
        assert_eq!(arr[0].as_str(), Some("bash"));
        assert_eq!(arr[1].as_str(), Some("-c"));
        let script = arr[2].as_str().unwrap();
        assert!(script.contains("/test/codex-notify.sh"));
        assert!(script.contains("'echo' 'legacy-hook' \"$@\""));
        assert_eq!(arr[3].as_str(), Some(CHAIN_ARGV0));
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
        // Codex itself rejects a string, but one naming our script was put
        // there on purpose: leave it alone
        let existing = toml::Value::String("/test/codex-notify.sh \"$@\"; my-own-hook".to_string());
        assert_eq!(
            CodexAdapter::plan_notify(Some(&existing), &panoptes_script()),
            NotifyPlan::AlreadyConfigured
        );
    }

    // The chained notify command, executed
    //
    // A string comparison cannot catch the bug these guard against: the old
    // chain looked right and still delivered the event to neither hook.

    /// An event as Codex would send it, with everything a shell could mangle
    const EVENT_JSON: &str = r#"{"type":"agent-turn-complete","last-assistant-message":"it's \"done\" - cost $5, `id` $(id) ${HOME} \\n"}"#;

    /// A hook that records every argument it is given, NUL-terminated, to `out`
    #[cfg(unix)]
    fn recording_hook(path: &Path, out: &Path) -> PathBuf {
        let script = format!(
            "#!/bin/bash\nfor arg in \"$@\"; do printf '%s\\0' \"$arg\"; done > {}\n",
            CodexAdapter::shell_quote(&out.to_string_lossy())
        );
        super::super::install_executable_script(path, &script).unwrap();
        path.to_path_buf()
    }

    /// What a [`recording_hook`] was called with, or `None` if it never ran
    #[cfg(unix)]
    fn recorded_args(out: &Path) -> Option<Vec<String>> {
        let bytes = std::fs::read(out).ok()?;
        let text = String::from_utf8(bytes).unwrap();
        Some(
            text.split_terminator('\0')
                .map(str::to_string)
                .collect::<Vec<_>>(),
        )
    }

    /// Run a `notify` value exactly as Codex does: argv spawned directly, no
    /// shell, the event appended as one final argument, stdin at /dev/null
    #[cfg(unix)]
    fn run_as_codex(notify: &toml::Value, event: &str) {
        let argv = CodexAdapter::parse_notify_command(notify).expect("argv");
        // The exit status is the user hook's, and Codex ignores it; only what
        // the hooks recorded matters
        std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .arg(event)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("run notify command");
    }

    #[cfg(unix)]
    fn chained_value(panoptes_hook: &Path, user_cmd: &[String]) -> toml::Value {
        CodexAdapter::notify_array_value(&CodexAdapter::build_chained_notify_command(
            panoptes_hook,
            user_cmd,
        ))
    }

    #[test]
    #[cfg(unix)]
    fn test_chained_notify_passes_event_to_both_hooks() {
        let dir = TempDir::new().unwrap();
        let ours_out = dir.path().join("ours.out");
        let theirs_out = dir.path().join("theirs.out");
        let ours = recording_hook(&dir.path().join("ours.sh"), &ours_out);
        let theirs = recording_hook(&dir.path().join("theirs.sh"), &theirs_out);

        let notify = chained_value(&ours, &[theirs.to_string_lossy().to_string()]);
        run_as_codex(&notify, EVENT_JSON);

        assert_eq!(recorded_args(&ours_out), Some(vec![EVENT_JSON.to_string()]));
        assert_eq!(
            recorded_args(&theirs_out),
            Some(vec![EVENT_JSON.to_string()])
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_chained_notify_user_hook_runs_when_panoptes_hook_fails() {
        let dir = TempDir::new().unwrap();
        let theirs_out = dir.path().join("theirs.out");
        let theirs = recording_hook(&dir.path().join("theirs.sh"), &theirs_out);
        let ours = dir.path().join("ours.sh");
        super::super::install_executable_script(&ours, "#!/bin/bash\nexit 3\n").unwrap();

        run_as_codex(
            &chained_value(&ours, &[theirs.to_string_lossy().to_string()]),
            EVENT_JSON,
        );
        assert_eq!(
            recorded_args(&theirs_out),
            Some(vec![EVENT_JSON.to_string()]),
            "a failing Panoptes hook must not suppress the user's"
        );

        // Nor must one that is missing altogether
        std::fs::remove_file(&theirs_out).unwrap();
        run_as_codex(
            &chained_value(
                &dir.path().join("no-such-hook.sh"),
                &[theirs.to_string_lossy().to_string()],
            ),
            EVENT_JSON,
        );
        assert_eq!(
            recorded_args(&theirs_out),
            Some(vec![EVENT_JSON.to_string()])
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_chained_notify_quotes_user_argv_with_spaces_and_quotes() {
        let dir = TempDir::new().unwrap();
        let ours_out = dir.path().join("ours.out");
        let theirs_out = dir.path().join("theirs.out");
        let ours = recording_hook(&dir.path().join("ours.sh"), &ours_out);
        // The hook itself lives somewhere a shell would split and unquote
        let awkward_dir = dir.path().join("it's a \"dir\" $HOME");
        let theirs = recording_hook(&awkward_dir.join("hook.sh"), &theirs_out);

        let user_cmd = vec![
            "bash".to_string(),
            theirs.to_string_lossy().to_string(),
            "--title".to_string(),
            "it's \"quoted\"".to_string(),
            "$HOME `id` ;&|*".to_string(),
            String::new(),
        ];
        run_as_codex(&chained_value(&ours, &user_cmd), EVENT_JSON);

        let mut expected = user_cmd[2..].to_vec();
        expected.push(EVENT_JSON.to_string());
        assert_eq!(recorded_args(&theirs_out), Some(expected));
        assert_eq!(recorded_args(&ours_out), Some(vec![EVENT_JSON.to_string()]));
    }

    #[test]
    #[cfg(unix)]
    fn test_chained_string_form_notify_receives_the_event() {
        let dir = TempDir::new().unwrap();
        let ours_out = dir.path().join("ours.out");
        let theirs_out = dir.path().join("theirs.out");
        let ours = recording_hook(&dir.path().join("ours.sh"), &ours_out);
        let theirs = recording_hook(&dir.path().join("theirs.sh"), &theirs_out);

        let existing = toml::Value::String(format!(
            "{} --flag",
            CodexAdapter::shell_quote(&theirs.to_string_lossy())
        ));
        let NotifyPlan::Set(notify) = CodexAdapter::plan_notify(Some(&existing), &ours) else {
            panic!("a string-form notify should be chained");
        };
        assert_eq!(notify.as_array().unwrap()[1].as_str(), Some("-c"));
        run_as_codex(&notify, EVENT_JSON);

        assert_eq!(
            recorded_args(&theirs_out),
            Some(vec!["--flag".to_string(), EVENT_JSON.to_string()])
        );
        assert_eq!(recorded_args(&ours_out), Some(vec![EVENT_JSON.to_string()]));
    }

    // Repairing chains written by older Panoptes versions

    /// A chain exactly as older Panoptes versions wrote it
    fn legacy_chain(chain: &str) -> toml::Value {
        CodexAdapter::notify_array_value(&[
            "bash".to_string(),
            "-lc".to_string(),
            chain.to_string(),
        ])
    }

    #[test]
    fn test_plan_notify_repairs_legacy_chain() {
        let existing = legacy_chain(r#"'/test/codex-notify.sh' "$@"; 'echo' 'legacy-hook'"#);
        assert_eq!(
            CodexAdapter::plan_notify(Some(&existing), &panoptes_script()),
            NotifyPlan::Set(CodexAdapter::notify_array_value(
                &CodexAdapter::build_chained_notify_command(
                    &panoptes_script(),
                    &["echo".to_string(), "legacy-hook".to_string()],
                )
            ))
        );

        // A word with an embedded quote, in the legacy escaping
        let existing =
            legacy_chain(r#"'/test/codex-notify.sh' "$@"; '/hooks/my hook.sh' 'it'\"'\"'s' ''"#);
        assert_eq!(
            CodexAdapter::plan_notify(Some(&existing), &panoptes_script()),
            NotifyPlan::Set(CodexAdapter::notify_array_value(
                &CodexAdapter::build_chained_notify_command(
                    &panoptes_script(),
                    &[
                        "/hooks/my hook.sh".to_string(),
                        "it's".to_string(),
                        String::new(),
                    ],
                )
            ))
        );
    }

    #[test]
    fn test_plan_notify_new_chain_is_already_configured() {
        let user_cmd = ["notify-send".to_string(), "it's done".to_string()];
        let chained = CodexAdapter::notify_array_value(
            &CodexAdapter::build_chained_notify_command(&panoptes_script(), &user_cmd),
        );
        assert_eq!(
            CodexAdapter::plan_notify(Some(&chained), &panoptes_script()),
            NotifyPlan::AlreadyConfigured
        );

        // The value a repair writes is itself left alone next time
        let legacy = legacy_chain(r#"'/test/codex-notify.sh' "$@"; 'notify-send'"#);
        let NotifyPlan::Set(repaired) =
            CodexAdapter::plan_notify(Some(&legacy), &panoptes_script())
        else {
            panic!("legacy chain should be repaired");
        };
        assert_eq!(
            CodexAdapter::plan_notify(Some(&repaired), &panoptes_script()),
            NotifyPlan::AlreadyConfigured
        );
    }

    #[test]
    fn test_plan_notify_unparseable_legacy_chain_is_unsupported() {
        for chain in [
            // Hand-edited: an unquoted word
            r#"'/test/codex-notify.sh' "$@"; echo legacy-hook"#,
            // Truncated mid-word
            r#"'/test/codex-notify.sh' "$@"; 'echo' 'legacy"#,
            // Nothing chained after ours
            r#"'/test/codex-notify.sh' "$@"; "#,
            // Doubled separator
            r#"'/test/codex-notify.sh' "$@"; 'echo'  'legacy-hook'"#,
            // Our script, but not the prefix the legacy writer produced
            r#"/test/codex-notify.sh; 'echo' 'legacy-hook'"#,
            // A user argv that itself names our script
            r#"'/test/codex-notify.sh' "$@"; 'bash' '/test/codex-notify.sh'"#,
        ] {
            assert_eq!(
                CodexAdapter::plan_notify(Some(&legacy_chain(chain)), &panoptes_script()),
                NotifyPlan::Unsupported,
                "{chain:?} must not be guessed at"
            );
        }
    }

    #[test]
    fn test_configure_codex_notify_repairs_legacy_chain_with_backup() {
        let temp_dir = TempDir::new().unwrap();
        let codex_home = temp_dir.path().join("codex-home");
        std::fs::create_dir_all(&codex_home).unwrap();
        let legacy_config = r#"
model = "o3-mini"
notify = ["bash", "-lc", "'/test/codex-notify.sh' \"$@\"; 'echo' 'legacy-hook'"]
"#;
        std::fs::write(codex_home.join("config.toml"), legacy_config).unwrap();

        CodexAdapter::configure_codex_notify(&codex_home, &panoptes_script()).unwrap();

        let content = std::fs::read_to_string(codex_home.join("config.toml")).unwrap();
        let config: toml::Value = toml::from_str(&content).unwrap();
        assert_eq!(
            config.get("notify"),
            Some(&CodexAdapter::notify_array_value(
                &CodexAdapter::build_chained_notify_command(
                    &panoptes_script(),
                    &["echo".to_string(), "legacy-hook".to_string()],
                )
            ))
        );
        // A repair is a modification: the pre-repair file is kept
        let backup = std::fs::read_to_string(codex_home.join("config.toml.panoptes.bak")).unwrap();
        assert_eq!(backup, legacy_config);
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

        // A Codex without lifecycle hooks; which one is installed on this
        // machine must not decide what the test checks
        CodexAdapter::install_hooks_as(HookInstall::Notify, &config, &spawn_config).unwrap();

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
    fn test_discover_session_id_never_claims_system_rollout() {
        // A background thread writes its rollout with a fresh timestamp; if
        // its cwd happens to match, it passes every other criterion. The
        // redacted fixture's cwd is replaced by the session's own to prove
        // the classification alone rejects it.
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let started = an_hour_ago();
        let at = started + chrono::Duration::minutes(1);

        let fixture = include_str!("../transcript/fixtures/memory_consolidation_rollout.jsonl");
        let mut header: serde_json::Value =
            serde_json::from_str(fixture.lines().next().unwrap()).unwrap();
        header["payload"]["cwd"] = serde_json::json!(cwd.path().to_string_lossy());
        header["payload"]["timestamp"] = serde_json::json!(at.to_rfc3339());
        let dir = home.path().join("sessions/2026/09/23");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rollout-memory.jsonl"), format!("{header}\n")).unwrap();

        assert_eq!(
            discover_session_id(home.path(), cwd.path(), started, &nothing_claimed()),
            None,
            "a system thread must never be claimed as a session's conversation"
        );

        // The session's own rollout is still found beside it
        write_rollout(home.path(), "own-id", cwd.path(), at);
        assert_eq!(
            discover_session_id(home.path(), cwd.path(), started, &nothing_claimed()).as_deref(),
            Some("own-id")
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

    // Lifecycle hooks (Codex 0.156.1+)

    fn test_spawn_config() -> SpawnConfig {
        SpawnConfig {
            session_id: Uuid::new_v4(),
            session_name: "test".to_string(),
            working_dir: PathBuf::from("/tmp"),
            initial_prompt: None,
            rows: 24,
            cols: 80,
            claude_config_dir: None,
            codex_home: None,
            resume: None,
        }
    }

    /// The Codex hooks directory the tests declare hooks under
    fn hooks_dir() -> PathBuf {
        PathBuf::from("/home/someone/.panoptes/hooks")
    }

    /// Parse the value half of a `-c key=value` override as Codex does:
    /// as a TOML value
    fn override_value(arg: &str) -> (String, toml::Value) {
        let (key, value) = arg.split_once('=').expect("key=value");
        let table: toml::Table = toml::from_str(&format!("v = {value}")).expect("valid TOML");
        (key.to_string(), table["v"].clone())
    }

    #[test]
    fn test_codex_version_parsing() {
        assert_eq!(
            CodexVersion::parse("codex-cli 0.156.1\n"),
            Some(CodexVersion(0, 156, 1))
        );
        assert_eq!(
            CodexVersion::parse("codex-cli 0.157.0-alpha.3"),
            Some(CodexVersion(0, 157, 0))
        );
        assert_eq!(CodexVersion::parse("1.2.3"), Some(CodexVersion(1, 2, 3)));
        assert_eq!(CodexVersion::parse("codex-cli"), None);
        assert_eq!(CodexVersion::parse("codex-cli 0.156"), None);
        assert_eq!(CodexVersion::parse(""), None);
    }

    #[test]
    fn test_hook_install_is_gated_on_version() {
        assert_eq!(
            HookInstall::for_version(Some(CodexVersion(0, 156, 1))),
            HookInstall::Lifecycle
        );
        assert_eq!(
            HookInstall::for_version(Some(CodexVersion(0, 157, 0))),
            HookInstall::Lifecycle
        );
        assert_eq!(
            HookInstall::for_version(Some(CodexVersion(1, 0, 0))),
            HookInstall::Lifecycle
        );
        // Older than the hash Panoptes has verified: the notify hook
        assert_eq!(
            HookInstall::for_version(Some(CodexVersion(0, 156, 0))),
            HookInstall::Notify
        );
        assert_eq!(
            HookInstall::for_version(Some(CodexVersion(0, 99, 9))),
            HookInstall::Notify
        );
        // A Codex that would not say: never risk a trust prompt on every spawn
        assert_eq!(HookInstall::for_version(None), HookInstall::Notify);
    }

    /// The hash must be Codex's own, or every spawn stops at Codex's hook
    /// review screen. This value is the one Codex 0.156.1 accepted as
    /// trusted for this exact declaration, with no review screen, in the
    /// PAN-44 spike.
    #[test]
    fn test_hook_trust_hash_matches_codex() {
        let command = "'/private/tmp/claude-501/-Users-ivan-Projects-panoptes/\
                       77ac9f70-4050-4b87-9f15-e9aec9adb576/scratchpad/pan44/links/Stop.sh'";
        assert_eq!(
            hook_trust_hash("stop", command),
            "sha256:9d7dae08b2a78807a6f1254194f193c335e5c63902f86627a3caa0873177394b"
        );
    }

    #[test]
    fn test_lifecycle_hook_args_declare_and_trust_every_event() {
        let args = lifecycle_hook_args(&hooks_dir());

        // Every value is its own `-c`
        assert_eq!(args.len(), (LIFECYCLE_EVENTS.len() + 1) * 2);
        for pair in args.chunks(2) {
            assert_eq!(pair[0], "-c");
        }
        let overrides: Vec<(String, toml::Value)> = args
            .iter()
            .skip(1)
            .step_by(2)
            .map(|a| override_value(a))
            .collect();

        let (state_key, state) = overrides.last().unwrap();
        assert_eq!(state_key, "hooks.state");
        let state = state.as_table().unwrap();
        assert_eq!(state.len(), LIFECYCLE_EVENTS.len());

        for ((event, label), (key, value)) in LIFECYCLE_EVENTS.iter().zip(&overrides) {
            assert_eq!(key, &format!("hooks.{event}"));

            // One group, no matcher, one command handler
            let groups = value.as_array().unwrap();
            assert_eq!(groups.len(), 1);
            let group = groups[0].as_table().unwrap();
            assert!(group.get("matcher").is_none());
            let handlers = group["hooks"].as_array().unwrap();
            assert_eq!(handlers.len(), 1);
            let handler = handlers[0].as_table().unwrap();
            assert_eq!(handler["type"].as_str(), Some("command"));
            assert_eq!(
                handler["timeout"].as_integer(),
                Some(LIFECYCLE_HOOK_TIMEOUT_SECS as i64)
            );
            // Codex runs the command through a shell: the path is quoted
            let command = handler["command"].as_str().unwrap();
            assert_eq!(
                command,
                format!("'/home/someone/.panoptes/hooks/codex/{event}.sh'")
            );

            // ...and trusted by exactly that declaration's hash
            let trust_key = format!("/<session-flags>/config.toml:{label}:0:0");
            assert_eq!(
                state[&trust_key]["trusted_hash"].as_str(),
                Some(hook_trust_hash(label, command).as_str()),
                "{event} is not trusted by its own hash"
            );
        }
    }

    #[test]
    fn test_lifecycle_hook_args_are_stable_across_spawns() {
        // Trust is keyed on the exact declaration, so two spawns must
        // declare byte-identical hooks
        assert_eq!(
            lifecycle_hook_args(&hooks_dir()),
            lifecycle_hook_args(&hooks_dir())
        );
    }

    #[test]
    fn test_lifecycle_hook_command_quotes_awkward_paths() {
        let dir = PathBuf::from("/Users/Jane Doe/it's/hooks");
        let command = lifecycle_hook_command(&dir, "Stop");
        assert_eq!(command, r"'/Users/Jane Doe/it'\''s/hooks/codex/Stop.sh'");
        // Still a valid `-c` value
        let args = lifecycle_hook_args(&dir);
        let (_, value) = override_value(&args[1]);
        assert!(value.as_array().is_some());
    }

    #[test]
    fn test_hook_args_lead_even_a_resume() {
        let adapter = CodexAdapter::new();
        let mut spawn_config = test_spawn_config();
        spawn_config.resume = Some("0199-thread".to_string());
        let hook_args = lifecycle_hook_args(&hooks_dir());
        let hook_count = hook_args.len();

        let args = adapter.command_line(hook_args, &spawn_config);

        // `-c` is a root option; after `resume` it would belong to the
        // subcommand's own parser
        assert!(args[..hook_count].chunks(2).all(|pair| pair[0] == "-c"));
        assert_eq!(args[hook_count], "resume");
        assert_eq!(args.last().map(String::as_str), Some("0199-thread"));
    }

    #[test]
    fn test_lifecycle_install_writes_nothing_into_codex_home() {
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            ..Config::default()
        };
        let codex_home = temp_dir.path().join("codex-home");
        std::fs::create_dir_all(&codex_home).unwrap();
        let user_config = "model = \"o3\"\nnotify = [\"my-hook\"]\n";
        std::fs::write(codex_home.join("config.toml"), user_config).unwrap();
        let user_hooks = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"mine"}]}]}}"#;
        std::fs::write(codex_home.join("hooks.json"), user_hooks).unwrap();
        let mut spawn_config = test_spawn_config();
        spawn_config.codex_home = Some(codex_home.clone());

        let args =
            CodexAdapter::install_hooks_as(HookInstall::Lifecycle, &config, &spawn_config).unwrap();
        assert_eq!(args, lifecycle_hook_args(&config.hooks_dir));

        // The user's Codex files are exactly as they were, with no backups
        let mut entries: Vec<_> = std::fs::read_dir(&codex_home)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        entries.sort();
        assert_eq!(entries, ["config.toml", "hooks.json"]);
        assert_eq!(
            std::fs::read_to_string(codex_home.join("config.toml")).unwrap(),
            user_config
        );
        assert_eq!(
            std::fs::read_to_string(codex_home.join("hooks.json")).unwrap(),
            user_hooks
        );

        // Every declared command resolves to the shared hook script
        let script = ClaudeCodeAdapter::hook_script_path(&config);
        assert!(script.is_file());
        for (event, _) in LIFECYCLE_EVENTS {
            let link = config.hooks_dir.join("codex").join(format!("{event}.sh"));
            assert_eq!(std::fs::read_link(&link).unwrap(), script, "{event}");
        }

        // Reinstalling is a no-op that yields the same declaration
        let again =
            CodexAdapter::install_hooks_as(HookInstall::Lifecycle, &config, &spawn_config).unwrap();
        assert_eq!(again, args);
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

    // Shared history across accounts (`codex_shared_history`)

    /// A shared home, a shadows dir and two account homes with fake logins
    fn shared_layout(root: &Path) -> (crate::codex_config::CodexHomes, PathBuf, PathBuf) {
        let account_a = root.join("account-a");
        let account_b = root.join("account-b");
        for (home, key) in [(&account_a, "sk-fake-a"), (&account_b, "sk-fake-b")] {
            std::fs::create_dir_all(home).unwrap();
            std::fs::write(
                home.join("auth.json"),
                format!(r#"{{"OPENAI_API_KEY":"{key}"}}"#),
            )
            .unwrap();
        }
        let homes = crate::codex_config::CodexHomes::new(
            true,
            root.join("shared"),
            root.join("panoptes/codex-homes"),
            root.join("default-home"),
        );
        (homes, account_a, account_b)
    }

    #[test]
    fn test_a_conversation_started_under_one_account_resumes_under_another() {
        let tmp = TempDir::new().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let (homes, account_a, account_b) = shared_layout(&root);
        let cwd = TempDir::new().unwrap();
        let id = "019aa0c9-8dea-7611-89d1-0000000000aa";

        // Account A's Codex writes its rollout through A's shadow
        let shadow_a = homes
            .prepare_spawn(Some(Uuid::new_v4()), Some(&account_a))
            .unwrap()
            .unwrap();
        write_rollout(&shadow_a, id, cwd.path(), an_hour_ago());

        // Account B's resume check finds it, as does B's own shadow...
        assert!(rollout_path(&homes.data_home(Some(&account_b)), id).is_some());
        let shadow_b = homes
            .prepare_spawn(Some(Uuid::new_v4()), Some(&account_b))
            .unwrap()
            .unwrap();
        assert!(rollout_path(&shadow_b, id).is_some());

        // ...and B's relaunch resumes it, from B's home with B's credentials
        let mut spawn_config = resume_spawn_config(Some(id));
        spawn_config.codex_home = Some(shadow_b.clone());
        let adapter = CodexAdapter::new();
        let args = adapter.build_args(&spawn_config);
        assert_eq!(args.first().map(String::as_str), Some("resume"));
        assert!(args.contains(&id.to_string()));
        let env = adapter.generate_env(&Config::default(), &spawn_config);
        assert_eq!(
            env.get("CODEX_HOME"),
            Some(&shadow_b.to_string_lossy().to_string())
        );
        assert_eq!(
            std::fs::read_to_string(shadow_b.join("auth.json")).unwrap(),
            r#"{"OPENAI_API_KEY":"sk-fake-b"}"#
        );
    }

    #[test]
    fn test_two_accounts_starting_in_one_cwd_at_once_get_their_own_rollouts() {
        // In a shared tree nothing but the claimed set keeps two accounts'
        // sessions apart; the discovery sweep grows it as it resolves each
        let tmp = TempDir::new().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let (homes, account_a, account_b) = shared_layout(&root);
        let cwd = TempDir::new().unwrap();
        let started = an_hour_ago();
        let shadow_a = homes
            .prepare_spawn(Some(Uuid::new_v4()), Some(&account_a))
            .unwrap()
            .unwrap();
        let shadow_b = homes
            .prepare_spawn(Some(Uuid::new_v4()), Some(&account_b))
            .unwrap()
            .unwrap();
        write_rollout(&shadow_a, "rollout-of-a", cwd.path(), started);
        write_rollout(
            &shadow_b,
            "rollout-of-b",
            cwd.path(),
            started + chrono::Duration::seconds(1),
        );

        let mut claimed = nothing_claimed();
        let a = discover_session_id(
            &homes.data_home(Some(&account_a)),
            cwd.path(),
            started,
            &claimed,
        )
        .unwrap();
        claimed.insert(a.clone());
        let b = discover_session_id(
            &homes.data_home(Some(&account_b)),
            cwd.path(),
            started,
            &claimed,
        )
        .unwrap();

        assert_eq!(a, "rollout-of-a");
        assert_eq!(b, "rollout-of-b");
    }

    #[test]
    fn test_only_a_shadow_home_gets_the_shared_sqlite_home() {
        let tmp = TempDir::new().unwrap();
        let shared = tmp.path().join("shared-not-created");
        let adapter = CodexAdapter::new();
        let config = Config {
            codex_shared_history: true,
            codex_shared_home: Some(shared.clone()),
            ..Config::default()
        };
        let shadows =
            crate::config::config_dir().join(crate::codex_config::homes::SHADOWS_DIR_NAME);

        let mut spawn_config = resume_spawn_config(None);
        spawn_config.codex_home = Some(shadows.join(Uuid::new_v4().to_string()));
        let env = adapter.generate_env(&config, &spawn_config);
        assert_eq!(
            env.get("CODEX_SQLITE_HOME"),
            Some(&shared.to_string_lossy().to_string())
        );

        // An account spawned in its own home, or anything with the flag off,
        // is left exactly as before
        spawn_config.codex_home = Some(tmp.path().join("account"));
        assert!(!adapter
            .generate_env(&config, &spawn_config)
            .contains_key("CODEX_SQLITE_HOME"));
        spawn_config.codex_home = Some(shadows.join("x"));
        assert!(!adapter
            .generate_env(&Config::default(), &spawn_config)
            .contains_key("CODEX_SQLITE_HOME"));
    }
}
