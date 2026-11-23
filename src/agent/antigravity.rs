//! Antigravity CLI (`agy`) adapter implementation
//!
//! `agy` runs interactively in a PTY like the other agents. What it reports
//! about itself, it reports only to a `statusLine` command: the agent's state,
//! whether a tool waits for approval, the conversation ID, context, quota and
//! the signed-in account (see `hooks::antigravity_status_line`). The PAN-59
//! spike settled how Panoptes gets in front of it:
//!
//! - **Per session, never persisted.** `agy --settings <json>` merges a JSON
//!   object of `settings.json` keys over the user's own, one top-level key at a
//!   time, and never writes it back. The overlay carries one key, `statusLine`,
//!   so everything else the user configured still applies. Nothing is written
//!   into the user's settings or repository, and nothing is left to clean up.
//! - **Chained.** The overlay replaces the user's own `statusLine`, so its
//!   command wraps theirs: the wrapper Claude sessions use (`agent/claude.rs`)
//!   forwards the payload to the hook server, then pipes it to the user's
//!   command, read from `~/.gemini/antigravity-cli/settings.json`, and prints
//!   what that prints. Without one, it prints Panoptes' compact quota line.
//! - **No hooks.** `agy` loads hooks only from `~/.gemini/config/hooks.json`
//!   and the workspace's `.agents/hooks.json`, so they cannot be injected per
//!   session; they also miss an Esc-cancel, which the status line does not.
//!   (Should they ever be added: a `PreToolUse` hook that prints `{}` *denies*
//!   the tool.)
//! - **Not `GEMINI_HOME`.** `agy` 1.5 ignores it. The hidden `--gemini_dir`
//!   does move the home, but would hide the user's conversations and replay
//!   onboarding, and is not needed for a single account.
//! - **Conversation ID.** `agy` mints its own and cannot be told one, so it is
//!   discovered from the first status-line payload that names it
//!   (`SessionManager`), and a relaunch resumes with `--conversation=<id>`.
//! - **Rendering.** Nothing to pin: Panoptes already answers the queries `agy`
//!   sends at startup (DECRQM 2026, OSC 11, kitty keyboard `?u`).

use crate::config::Config;
use crate::session::PtyHandle;
use anyhow::{bail, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::adapter::{AgentAdapter, SpawnConfig, SpawnResult};
use super::claude::{ClaudeCodeAdapter, STATUS_LINE_SCRIPT_NAME};

/// The oldest `agy` whose status line this adapter was built against
///
/// 1.5 is the line the PAN-59 spike recorded (1.5.15). Older releases are not
/// known to send `tool_confirmation_pending` or `conversation_id`, without
/// which a session would show no state at all.
const MIN_VERSION: AgyVersion = AgyVersion(1, 5, 0);

/// How long an `agy --version` probe is believed
///
/// Long enough that spawning sessions does not keep paying for a process
/// launch, short enough that `agy update` does not need a Panoptes restart.
const VERSION_PROBE_TTL: std::time::Duration = std::time::Duration::from_secs(600);

/// An `agy` release number, `major.minor.patch`
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct AgyVersion(u64, u64, u64);

impl AgyVersion {
    /// Read `agy --version` output, a bare `1.5.15`
    ///
    /// The first word that starts with a digit is taken, so a future
    /// `agy 1.6.0` reads too; a pre-release suffix is dropped.
    fn parse(output: &str) -> Option<Self> {
        let word = output
            .split_whitespace()
            .find(|word| word.starts_with(|c: char| c.is_ascii_digit()))?;
        let core = word.split(['-', '+']).next()?;
        let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
        Some(AgyVersion(parts.next()??, parts.next()??, parts.next()??))
    }
}

impl std::fmt::Display for AgyVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// Refuse an `agy` known to be older than [`MIN_VERSION`]
///
/// An unreadable version is let through: if `agy` is missing altogether the
/// spawn fails on its own, with the clearer error.
fn check_version(version: Option<AgyVersion>) -> Result<()> {
    match version {
        Some(version) if version < MIN_VERSION => bail!(
            "Antigravity CLI {version} is too old: Panoptes needs {MIN_VERSION} or later \
             to track its state. Run `agy update`."
        ),
        Some(_) => Ok(()),
        None => {
            tracing::warn!("Could not read the Antigravity CLI version; assuming it is recent");
            Ok(())
        }
    }
}

/// Run `<command> --version`
fn probe_version(command: &str) -> Option<AgyVersion> {
    let output = std::process::Command::new(command)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    AgyVersion::parse(&String::from_utf8_lossy(&output.stdout))
}

/// The installed `agy`'s version, probed at most once per [`VERSION_PROBE_TTL`]
fn cached_version(command: &str) -> Option<AgyVersion> {
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    type Probe = (Instant, Option<AgyVersion>);
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
    let version = probe_version(command);
    tracing::debug!(?version, "Probed Antigravity CLI version");
    *cached = Some((Instant::now(), version));
    version
}

/// Where `agy` keeps its conversations and settings, `~/.gemini/antigravity-cli`
pub fn data_dir() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".gemini").join("antigravity-cli"))
}

/// Whether `agy` has the conversation `id` on disk, for `--conversation`
///
/// Each conversation is one SQLite database, `conversations/<id>.db`.
pub fn conversation_exists(data_dir: &Path, id: &str) -> bool {
    // An ID is a UUID; anything path-like is not one of agy's
    if id.is_empty() || id.contains(['/', '\\']) || id.contains("..") {
        return false;
    }
    data_dir
        .join("conversations")
        .join(format!("{id}.db"))
        .is_file()
}

/// Adapter for the Antigravity CLI
pub struct AntigravityAdapter;

impl AntigravityAdapter {
    /// The user's own `statusLine` setting, if it runs a command
    ///
    /// Read from `settings.json` in `data_dir`, tolerating a missing or broken
    /// file. Panoptes' own wrapper is seen through to the command it wraps,
    /// should one ever end up there, so a session never wraps itself.
    fn user_status_line(data_dir: Option<&Path>) -> Option<(serde_json::Value, String)> {
        let path = data_dir?.join("settings.json");
        let content = std::fs::read_to_string(path).ok()?;
        let settings: serde_json::Value = serde_json::from_str(&content).ok()?;
        let setting = settings.get("statusLine")?.clone();
        let command = setting
            .get("command")?
            .as_str()
            .filter(|c| !c.trim().is_empty())?
            .to_string();
        if setting.get("type").and_then(|t| t.as_str()) != Some("command") {
            return None;
        }
        let command = match super::split_shell_quoted(&command) {
            Some(words)
                if words
                    .first()
                    .and_then(|w| Path::new(w).file_name())
                    .is_some_and(|name| name == STATUS_LINE_SCRIPT_NAME) =>
            {
                words.get(1).cloned()?
            }
            _ => command,
        };
        Some((setting, command))
    }

    /// The `--settings` overlay: a `statusLine` running `script` before `user`'s
    ///
    /// The user's own options carry over; only the command is Panoptes'.
    fn settings_overlay(
        script: &Path,
        user: Option<(serde_json::Value, String)>,
    ) -> serde_json::Value {
        let mut words = vec![super::shell_quote(&script.to_string_lossy())];
        let mut setting = match user {
            Some((setting, command)) => {
                words.push(super::shell_quote(&command));
                setting
            }
            None => serde_json::json!({}),
        };
        setting["type"] = serde_json::Value::String("command".to_string());
        setting["command"] = serde_json::Value::String(words.join(" "));
        serde_json::json!({ "statusLine": setting })
    }

    /// The arguments for a spawn, given the overlay to pass
    fn args_with_overlay(spawn_config: &SpawnConfig, overlay: Option<&str>) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(overlay) = overlay {
            args.push("--settings".to_string());
            args.push(overlay.to_string());
        }
        if let Some(id) = &spawn_config.resume {
            args.push(format!("--conversation={id}"));
        }
        if let Some(prompt) = &spawn_config.initial_prompt {
            // Interactive, unlike `--print`: the session carries on after it
            args.push("--prompt-interactive".to_string());
            args.push(prompt.clone());
        }
        args
    }

    /// The overlay for a spawn under `config`, as the JSON `--settings` takes
    fn overlay_json(config: &Config) -> String {
        let script = config.hooks_dir.join(STATUS_LINE_SCRIPT_NAME);
        let user = Self::user_status_line(data_dir().as_deref());
        Self::settings_overlay(&script, user).to_string()
    }
}

impl AgentAdapter for AntigravityAdapter {
    fn name(&self) -> &str {
        "Antigravity"
    }

    fn command(&self) -> &str {
        "agy"
    }

    fn default_args(&self) -> Vec<String> {
        vec![]
    }

    /// The status line reaches the hook server as a `StatusLine` envelope
    fn supports_hooks(&self) -> bool {
        true
    }

    fn generate_env(
        &self,
        _config: &Config,
        spawn_config: &SpawnConfig,
    ) -> HashMap<String, String> {
        let mut env = HashMap::new();
        // The status-line process inherits it; without it the wrapper posts
        // nothing and only runs the user's own command
        env.insert(
            "PANOPTES_SESSION_ID".to_string(),
            spawn_config.session_id.to_string(),
        );
        // An update mid-session would replace the binary under a running
        // session; the user updates on their own terms with `agy update`
        env.insert("AGY_CLI_DISABLE_AUTO_UPDATE".to_string(), "1".to_string());
        env
    }

    /// Check the version and install the shared status-line wrapper
    ///
    /// Nothing per session is written: the overlay travels on the command line.
    fn setup_hooks(&self, config: &Config, _spawn_config: &SpawnConfig) -> Result<Vec<PathBuf>> {
        check_version(cached_version(self.command()))?;
        ClaudeCodeAdapter::install_status_line_script(config)?;
        Ok(vec![])
    }

    /// The arguments without the `--settings` overlay
    ///
    /// The overlay names the status-line wrapper in `config.hooks_dir`, and
    /// this has no `Config` to find it in, so [`Self::spawn`] adds it.
    fn build_args(&self, spawn_config: &SpawnConfig) -> Vec<String> {
        Self::args_with_overlay(spawn_config, None)
    }

    /// Known upfront only on a resume; a fresh conversation names itself in
    /// its first status-line payload
    fn agent_session_id(&self, spawn_config: &SpawnConfig) -> Option<String> {
        spawn_config.resume.clone()
    }

    /// The default lifecycle, with the overlay - which needs `config` - in
    /// front of the arguments
    fn spawn(&self, config: &Config, spawn_config: &SpawnConfig) -> Result<SpawnResult> {
        self.setup_hooks(config, spawn_config)?;
        let env = self.generate_env(config, spawn_config);
        let args = Self::args_with_overlay(spawn_config, Some(&Self::overlay_json(config)));
        let args_refs: Vec<&str> = args.iter().map(String::as_str).collect();
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
    use tempfile::TempDir;

    fn spawn_config() -> SpawnConfig {
        SpawnConfig {
            session_id: uuid::Uuid::new_v4(),
            session_name: "agy".to_string(),
            working_dir: PathBuf::from("/tmp"),
            initial_prompt: None,
            rows: 24,
            cols: 80,
            claude_config_dir: None,
            codex_home: None,
            resume: None,
        }
    }

    #[test]
    fn test_version_parsing_and_minimum() {
        assert_eq!(AgyVersion::parse("1.5.15\n"), Some(AgyVersion(1, 5, 15)));
        assert_eq!(AgyVersion::parse("agy 1.6.0"), Some(AgyVersion(1, 6, 0)));
        assert_eq!(AgyVersion::parse("2.0.0-beta.1"), Some(AgyVersion(2, 0, 0)));
        assert_eq!(AgyVersion::parse("dev"), None);
        assert_eq!(AgyVersion::parse("1.5"), None);

        assert!(check_version(Some(AgyVersion(1, 5, 15))).is_ok());
        assert!(check_version(Some(AgyVersion(1, 5, 0))).is_ok());
        assert!(check_version(None).is_ok());
        let err = check_version(Some(AgyVersion(1, 4, 9))).unwrap_err();
        assert!(err.to_string().contains("1.4.9 is too old"), "{err}");
        assert!(err.to_string().contains("agy update"), "{err}");
    }

    #[test]
    fn test_fresh_spawn_args() {
        let config = spawn_config();
        assert_eq!(
            AntigravityAdapter::args_with_overlay(&config, Some("{}")),
            vec!["--settings", "{}"]
        );
        // The ID is minted by agy and found later
        assert_eq!(AntigravityAdapter.agent_session_id(&config), None);
    }

    #[test]
    fn test_resume_and_prompt_args() {
        let mut config = spawn_config();
        config.resume = Some("11111111-1111-4111-8111-111111111111".to_string());
        config.initial_prompt = Some("hello there".to_string());
        assert_eq!(
            AntigravityAdapter::args_with_overlay(&config, None),
            vec![
                "--conversation=11111111-1111-4111-8111-111111111111",
                "--prompt-interactive",
                "hello there",
            ]
        );
        assert_eq!(
            AntigravityAdapter.agent_session_id(&config).as_deref(),
            Some("11111111-1111-4111-8111-111111111111")
        );
    }

    #[test]
    fn test_env_carries_the_guard_and_disables_updates() {
        let config = spawn_config();
        let env = AntigravityAdapter.generate_env(&Config::default(), &config);
        assert_eq!(
            env.get("PANOPTES_SESSION_ID"),
            Some(&config.session_id.to_string())
        );
        assert_eq!(
            env.get("AGY_CLI_DISABLE_AUTO_UPDATE").map(String::as_str),
            Some("1")
        );
        // Not relocated: GEMINI_HOME is ignored by agy 1.5
        assert!(!env.contains_key("GEMINI_HOME"));
    }

    fn write_settings(dir: &Path, settings: serde_json::Value) {
        std::fs::write(dir.join("settings.json"), settings.to_string()).unwrap();
    }

    #[test]
    fn test_overlay_without_a_user_status_line() {
        let script = Path::new("/srv/panoptes hooks/panoptes-statusline.sh");
        let overlay = AntigravityAdapter::settings_overlay(script, None);
        assert_eq!(
            overlay,
            serde_json::json!({"statusLine": {
                "type": "command",
                "command": "'/srv/panoptes hooks/panoptes-statusline.sh'",
            }})
        );
        // Only the one key, so the user's other settings still apply
        assert_eq!(overlay.as_object().unwrap().len(), 1);
    }

    #[test]
    fn test_overlay_chains_the_users_status_line() {
        let dir = TempDir::new().unwrap();
        let script = Path::new("/hooks/panoptes-statusline.sh");

        // None configured, a broken file, and a non-command one
        assert_eq!(AntigravityAdapter::user_status_line(Some(dir.path())), None);
        std::fs::write(dir.path().join("settings.json"), "{not json").unwrap();
        assert_eq!(AntigravityAdapter::user_status_line(Some(dir.path())), None);
        write_settings(
            dir.path(),
            serde_json::json!({"statusLine": {"type": "static", "command": "x"}}),
        );
        assert_eq!(AntigravityAdapter::user_status_line(Some(dir.path())), None);
        assert_eq!(AntigravityAdapter::user_status_line(None), None);

        // A real one: its options carry over, its command is wrapped
        write_settings(
            dir.path(),
            serde_json::json!({
                "trustedWorkspaces": ["/x"],
                "statusLine": {"type": "command", "command": "echo \"it's $USER\"", "padding": 1}
            }),
        );
        let user = AntigravityAdapter::user_status_line(Some(dir.path()));
        let overlay = AntigravityAdapter::settings_overlay(script, user);
        let setting = &overlay["statusLine"];
        assert_eq!(setting["padding"], 1);
        assert_eq!(setting["type"], "command");
        let command = setting["command"].as_str().unwrap();
        assert_eq!(
            super::super::split_shell_quoted(command),
            Some(vec![
                "/hooks/panoptes-statusline.sh".to_string(),
                "echo \"it's $USER\"".to_string(),
            ])
        );

        // Our own wrapper left in the file is seen through, never wrapped twice
        write_settings(
            dir.path(),
            serde_json::json!({"statusLine": {"type": "command", "command": command}}),
        );
        let (_, found) = AntigravityAdapter::user_status_line(Some(dir.path())).unwrap();
        assert_eq!(found, "echo \"it's $USER\"");
    }

    #[test]
    fn test_conversation_exists() {
        let dir = TempDir::new().unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        assert!(!conversation_exists(dir.path(), id));
        std::fs::create_dir_all(dir.path().join("conversations")).unwrap();
        std::fs::write(
            dir.path().join("conversations").join(format!("{id}.db")),
            "",
        )
        .unwrap();
        assert!(conversation_exists(dir.path(), id));
        assert!(!conversation_exists(dir.path(), ""));
        assert!(!conversation_exists(dir.path(), "../conversations/x"));
    }
}
