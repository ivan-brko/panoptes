//! Configuration management for Panoptes

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Custom shell session shortcut
///
/// Defines a keyboard shortcut that spawns a shell session with a predefined command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomShortcut {
    /// Single character trigger key (e.g., 'v')
    pub key: char,
    /// Display name (optional - if empty, uses first 6 chars of command)
    #[serde(default)]
    pub name: String,
    /// Command to run in the shell (e.g., "code . &")
    pub command: String,
    /// Whether to automatically close the session after the command finishes
    #[serde(default)]
    pub auto_close: bool,
}

impl CustomShortcut {
    /// Create a new custom shortcut
    pub fn new(key: char, name: String, command: String, auto_close: bool) -> Self {
        Self {
            key,
            name,
            command,
            auto_close,
        }
    }

    /// Get the display name for this shortcut
    ///
    /// Returns the name if set, otherwise returns the command (caller should truncate if needed)
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.command
        } else {
            &self.name
        }
    }

    /// Get a truncated display name (max 6 chars) for footer display
    pub fn short_display_name(&self) -> String {
        let name = self.display_name();
        if name.chars().count() <= 6 {
            name.to_string()
        } else {
            name.chars().take(6).collect()
        }
    }
}

/// Keys that cannot be bound to a custom shortcut
///
/// Custom shortcuts fire in the session and branch views, so a shortcut sharing
/// a key with something bound there could never run - the view's own arm
/// matches first. The list is deliberately a little wider than that, covering
/// keys bound in neighbouring views too, so a shortcut does not mean one thing
/// on one screen and something else on the next:
/// - k: manage shortcuts (this feature), handled globally
/// - g, G: jump to top/bottom in the log viewer
/// - x: Codex configs, from the overview and project views
/// - n, s, d: new worktree/AI, shell, delete - bound in the branch/project views
/// - 0-9: jump to session by number
///
/// `Space`, `Esc`, `Enter`, and `Tab` are not chars and cannot be bound at all.
const RESERVED_KEYS: &[char] = &['g', 'G', 'k', 'x', 'n', 's', 'd'];
const RESERVED_DIGITS: bool = true;

/// Check if a key is reserved and cannot be used for custom shortcuts
pub fn is_reserved_key(key: char) -> bool {
    if RESERVED_DIGITS && key.is_ascii_digit() {
        return true;
    }
    RESERVED_KEYS.contains(&key)
}

/// Get a human-readable list of reserved keys
pub fn reserved_keys_display() -> String {
    let mut keys: Vec<String> = RESERVED_KEYS.iter().map(|c| c.to_string()).collect();
    if RESERVED_DIGITS {
        keys.push("0-9".to_string());
    }
    keys.join(", ")
}

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Port for the hook HTTP server
    #[serde(default = "default_hook_port")]
    pub hook_port: u16,

    /// Directory for git worktrees
    #[serde(default = "default_worktrees_dir")]
    pub worktrees_dir: PathBuf,

    /// Directory for hook scripts
    #[serde(default = "default_hooks_dir")]
    pub hooks_dir: PathBuf,

    /// State timeout in seconds - Executing states auto-transition to Idle after this (default: 300 = 5 min)
    #[serde(default = "default_state_timeout")]
    pub state_timeout_secs: u64,

    /// Exited session retention in seconds - sessions are cleaned up after this duration (default: 300 = 5 min)
    #[serde(default = "default_exited_retention")]
    pub exited_retention_secs: u64,

    /// How to get the user's attention when a session needs it
    #[serde(default)]
    pub notification_method: NotificationMethod,

    /// Maximum scrollback lines per session (default: 10000)
    /// Each 1000 lines uses approximately 10KB of memory
    #[serde(default = "default_scrollback_lines")]
    pub scrollback_lines: usize,

    /// Seconds a session may sit idle before its agent process is suspended
    ///
    /// Suspending kills the child process and keeps the scrollback; the session
    /// wakes on the next interaction. Set to 0 to disable.
    #[serde(default = "default_suspend_after")]
    pub suspend_after_secs: u64,

    /// Whether to log every raw agent transcript line to `~/.panoptes/logs/`
    ///
    /// Off by default. Turn it on to diagnose a session whose state looks
    /// wrong: the log holds exactly what the agent wrote, so the transcript
    /// reader's interpretation can be checked against its input.
    #[serde(default)]
    pub log_agent_events: bool,

    /// Whether Claude's periodic "idle" notification raises attention at all
    ///
    /// Claude nags after roughly a minute of an unattended prompt. That is the
    /// same event type it uses to say a permission dialog is open, which is why
    /// every notification used to ring. Off by default: a session you already
    /// know is waiting does not need to keep telling you.
    #[serde(default)]
    pub attention_on_idle: bool,

    // Everything below serialises as a TOML table or array-of-tables. TOML has
    // no way to express a bare key after a table header, so any scalar field
    // added later must go ABOVE this line or it will be silently swallowed into
    // whichever table precedes it.
    /// Which attention reasons ring the terminal bell
    #[serde(default)]
    pub notify_on: NotifyOn,

    /// Custom shell session shortcuts
    ///
    /// Each shortcut defines a key that spawns a shell session with a predefined command.
    #[serde(default)]
    pub custom_shortcuts: Vec<CustomShortcut>,
}

/// How Panoptes gets the user's attention when a session needs it
///
/// Serialises as the lowercase strings ("bell", "title", "none") this field
/// has always used in `config.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationMethod {
    /// Ring the terminal bell
    #[default]
    Bell,
    /// Rewrite the terminal title with the session that wants attention
    Title,
    /// Stay silent
    None,
}

/// Unknown values fall back to `Bell` rather than failing the whole load.
///
/// The field predates the enum, so arbitrary hand-typed strings exist in
/// config files; they always behaved as `bell` (the old string match's
/// catch-all) and must keep both loading and behaving that way.
impl<'de> Deserialize<'de> for NotificationMethod {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "bell" => Self::Bell,
            "title" => Self::Title,
            "none" => Self::None,
            other => {
                tracing::warn!(
                    value = %other,
                    "Unknown notification_method in config; defaulting to bell"
                );
                Self::Bell
            }
        })
    }
}

/// Which attention reasons are worth interrupting the user for
///
/// Every reason still raises the badge in the session list; these control only
/// the audible/terminal-title notification. The split is deliberate: a stalled
/// tool is worth showing in the list but is rarely worth a sound, since nothing
/// is blocked on you and the watchdog is guessing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyOn {
    /// A permission dialog or inline question is blocking a turn
    #[serde(default = "default_true")]
    pub approval: bool,

    /// An agent finished its turn
    #[serde(default = "default_true")]
    pub turn_complete: bool,

    /// A tool has been in flight far longer than expected
    #[serde(default)]
    pub stalled: bool,

    /// A session's process died unexpectedly
    #[serde(default = "default_true")]
    pub crashed: bool,
}

impl Default for NotifyOn {
    fn default() -> Self {
        Self {
            approval: true,
            turn_complete: true,
            stalled: false,
            crashed: true,
        }
    }
}

impl NotifyOn {
    /// Whether this reason should produce an audible notification
    pub fn rings(&self, reason: &crate::session::AttentionReason) -> bool {
        use crate::session::AttentionReason;
        match reason {
            AttentionReason::Approval { .. } => self.approval,
            AttentionReason::TurnComplete => self.turn_complete,
            AttentionReason::Stalled { .. } => self.stalled,
            AttentionReason::Crashed { .. } => self.crashed,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_hook_port() -> u16 {
    9999
}

fn default_worktrees_dir() -> PathBuf {
    config_dir().join("worktrees")
}

fn default_hooks_dir() -> PathBuf {
    config_dir().join("hooks")
}

fn default_state_timeout() -> u64 {
    300 // 5 minutes
}

fn default_exited_retention() -> u64 {
    300 // 5 minutes
}

fn default_scrollback_lines() -> usize {
    10_000
}

fn default_suspend_after() -> u64 {
    7200 // 2 hours
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hook_port: default_hook_port(),
            worktrees_dir: default_worktrees_dir(),
            hooks_dir: default_hooks_dir(),
            state_timeout_secs: default_state_timeout(),
            exited_retention_secs: default_exited_retention(),
            notification_method: NotificationMethod::default(),
            scrollback_lines: default_scrollback_lines(),
            suspend_after_secs: default_suspend_after(),
            log_agent_events: false,
            attention_on_idle: false,
            notify_on: NotifyOn::default(),
            custom_shortcuts: Vec::new(),
        }
    }
}

impl Config {
    /// Load configuration from file, returning a warning message if it was corrupted
    ///
    /// One bad line in `config.toml` must not abort startup: the broken file
    /// is backed up with a timestamp, the defaults take over, and the warning
    /// is surfaced to the user so the hand-edit is not silently discarded.
    pub fn load_with_status() -> (Self, Option<String>) {
        Self::load_from_with_status(&config_file_path())
    }

    /// Load configuration from a specific path, returning a warning message if corrupted
    fn load_from_with_status(path: &Path) -> (Self, Option<String>) {
        use crate::persistence::{backup_corrupted_file, load_text, LoadOutcome};

        let content = match load_text(path, "config") {
            LoadOutcome::Absent => return (Self::default(), None),
            LoadOutcome::Loaded(content) => content,
            LoadOutcome::Corrupted { fallback_warning } => {
                return (Self::default(), Some(fallback_warning))
            }
        };

        match toml::from_str(&content) {
            Ok(config) => (config, None),
            Err(e) => {
                tracing::error!("The config file {} is corrupted: {}", path.display(), e);
                let warning = match backup_corrupted_file(path) {
                    Some(backup_path) => format!(
                        "The config file was invalid. Backup saved to {}. Using defaults.",
                        backup_path.display()
                    ),
                    None => format!("The config file was invalid ({}). Using defaults.", e),
                };
                (Self::default(), Some(warning))
            }
        }
    }

    /// Save configuration to file (atomically, via a sibling temp file)
    pub fn save(&self) -> Result<()> {
        let path = config_file_path();
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;
        crate::persistence::save_text_atomic(&path, &content, "config")
    }

    /// Get a custom shortcut by key
    pub fn get_shortcut(&self, key: char) -> Option<&CustomShortcut> {
        self.custom_shortcuts.iter().find(|s| s.key == key)
    }

    /// Add a custom shortcut, returning error if key is reserved or duplicate
    pub fn add_shortcut(&mut self, shortcut: CustomShortcut) -> Result<()> {
        if is_reserved_key(shortcut.key) {
            anyhow::bail!("Key '{}' is reserved", shortcut.key);
        }
        if self.custom_shortcuts.iter().any(|s| s.key == shortcut.key) {
            anyhow::bail!("Key '{}' is already in use", shortcut.key);
        }
        self.custom_shortcuts.push(shortcut);
        Ok(())
    }

    /// Remove a custom shortcut by index
    pub fn remove_shortcut(&mut self, index: usize) -> Option<CustomShortcut> {
        if index < self.custom_shortcuts.len() {
            Some(self.custom_shortcuts.remove(index))
        } else {
            None
        }
    }

    /// Check if a key is available for a custom shortcut
    pub fn is_shortcut_key_available(&self, key: char) -> bool {
        !is_reserved_key(key) && !self.custom_shortcuts.iter().any(|s| s.key == key)
    }
}

/// Get the base configuration directory (~/.panoptes)
/// Falls back to ./.panoptes if home directory cannot be determined
pub fn config_dir() -> PathBuf {
    try_config_dir().unwrap_or_else(|| {
        tracing::warn!("Could not determine home directory, using current directory for config");
        PathBuf::from(".panoptes")
    })
}

/// Try to get the base configuration directory, returning None if home dir is unavailable
pub fn try_config_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".panoptes"))
}

/// Get the path to the config file
pub fn config_file_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Get the path to the logs directory
pub fn logs_dir() -> PathBuf {
    config_dir().join("logs")
}

/// Ensure all required directories exist
pub fn ensure_directories() -> Result<()> {
    let config = Config::default();

    std::fs::create_dir_all(config_dir()).context("Failed to create config directory")?;

    std::fs::create_dir_all(&config.worktrees_dir)
        .context("Failed to create worktrees directory")?;

    std::fs::create_dir_all(&config.hooks_dir).context("Failed to create hooks directory")?;

    std::fs::create_dir_all(logs_dir()).context("Failed to create logs directory")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_survives_a_toml_round_trip() {
        // TOML has no way to express a bare key after a table header, so a
        // scalar field declared below `notify_on` would either fail to
        // serialise or be silently swallowed into it on the way back in.
        let mut original = Config::default();
        original.notify_on.turn_complete = false;
        original.notify_on.stalled = true;
        original.attention_on_idle = true;
        original.custom_shortcuts.push(CustomShortcut::new(
            'v',
            "VSCode".to_string(),
            "code . &".to_string(),
            false,
        ));

        let text = toml::to_string_pretty(&original).expect("config must serialise");
        let parsed: Config = toml::from_str(&text).expect("config must round trip");

        assert!(parsed.attention_on_idle);
        assert!(!parsed.notify_on.turn_complete);
        assert!(parsed.notify_on.stalled);
        assert!(parsed.notify_on.approval);
        assert_eq!(parsed.custom_shortcuts.len(), 1);
        assert_eq!(parsed.scrollback_lines, original.scrollback_lines);
    }

    #[test]
    fn test_config_written_before_notify_settings_still_loads() {
        // A config file from before these options existed must keep working
        let legacy = r#"
hook_port = 9999
worktrees_dir = "/tmp/wt"
hooks_dir = "/tmp/hooks"
max_output_lines = 500
notification_method = "title"
"#;
        let parsed: Config = toml::from_str(legacy).expect("legacy config must load");

        // `max_output_lines` was removed; an old file that still sets it must
        // load fine (the now-unknown key is simply ignored, not an error)
        assert_eq!(parsed.notification_method, NotificationMethod::Title);
        // Absent sections fall back to the documented defaults
        assert!(parsed.notify_on.approval);
        assert!(parsed.notify_on.turn_complete);
        assert!(!parsed.notify_on.stalled);
        assert!(parsed.notify_on.crashed);
        assert!(!parsed.attention_on_idle);
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.hook_port, 9999);
        assert_eq!(config.notification_method, NotificationMethod::Bell);
    }

    /// The three documented values parse to their variants; anything else -
    /// this field predates the enum, so arbitrary hand-typed strings exist in
    /// config files - falls back to the bell, exactly as the old string match
    /// treated it. A typo must not fail the whole config load.
    #[test]
    fn test_notification_method_parses_known_and_unknown_values() {
        for (raw, expected) in [
            ("bell", NotificationMethod::Bell),
            ("title", NotificationMethod::Title),
            ("none", NotificationMethod::None),
            ("gong", NotificationMethod::Bell),
        ] {
            let parsed: Config = toml::from_str(&format!("notification_method = \"{raw}\""))
                .unwrap_or_else(|e| panic!("{raw:?} must load: {e}"));
            assert_eq!(parsed.notification_method, expected, "for input {raw:?}");
        }
    }

    /// The enum must serialise back to the same lowercase strings old config
    /// files use, so a round trip does not rewrite the field
    #[test]
    fn test_notification_method_round_trips() {
        for method in [
            NotificationMethod::Bell,
            NotificationMethod::Title,
            NotificationMethod::None,
        ] {
            let config = Config {
                notification_method: method,
                ..Default::default()
            };
            let text = toml::to_string(&config).unwrap();
            let parsed: Config = toml::from_str(&text).unwrap();
            assert_eq!(parsed.notification_method, method);
        }
    }

    #[test]
    fn test_load_corrupt_toml_backs_up_and_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "hook_port = not a number\n[[[").unwrap();

        let (config, warning) = Config::load_from_with_status(&path);

        // Falls back to defaults instead of aborting startup
        assert_eq!(config.hook_port, 9999);
        assert!(warning.is_some(), "corruption should surface a warning");
        assert!(!path.exists(), "corrupted file should be renamed away");

        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("toml.corrupt."))
            .collect();
        assert_eq!(backups.len(), 1, "expected exactly one timestamped backup");
    }

    #[test]
    fn test_load_missing_config_uses_defaults_without_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let (config, warning) = Config::load_from_with_status(&path);

        assert!(warning.is_none());
        assert_eq!(config.hook_port, 9999);
    }

    #[test]
    fn test_config_serialization() {
        let config = Config::default();
        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(config.hook_port, parsed.hook_port);
    }

    #[test]
    fn test_config_dir_does_not_panic() {
        // This test verifies that config_dir() does not panic
        // even if it falls back to a local directory
        let dir = config_dir();
        assert!(dir.ends_with(".panoptes"));
    }

    #[test]
    fn test_try_config_dir() {
        // try_config_dir should return Some on most systems with a home dir
        // but the important thing is it doesn't panic
        let result = try_config_dir();
        // We can't assert it's Some because CI might not have a home dir
        // But if it is Some, it should end with .panoptes
        if let Some(path) = result {
            assert!(path.ends_with(".panoptes"));
        }
    }

    // Config scrollback_lines tests
    #[test]
    fn test_default_config_scrollback() {
        let config = Config::default();
        assert_eq!(config.scrollback_lines, 10_000);
    }

    #[test]
    fn test_config_serialization_with_scrollback() {
        let config = Config {
            scrollback_lines: 5000,
            ..Default::default()
        };

        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.scrollback_lines, 5000);
    }

    // Custom shortcut tests
    #[test]
    fn test_custom_shortcut_display_name_with_name() {
        let shortcut =
            CustomShortcut::new('v', "VSCode".to_string(), "code . &".to_string(), false);
        assert_eq!(shortcut.display_name(), "VSCode");
    }

    #[test]
    fn test_custom_shortcut_display_name_without_name() {
        let shortcut = CustomShortcut::new('v', String::new(), "code . &".to_string(), false);
        assert_eq!(shortcut.display_name(), "code . &");
    }

    #[test]
    fn test_custom_shortcut_short_display_name() {
        let shortcut = CustomShortcut::new(
            'v',
            "VSCodeEditor".to_string(),
            "code . &".to_string(),
            false,
        );
        assert_eq!(shortcut.short_display_name(), "VSCode");
    }

    #[test]
    fn test_is_reserved_key() {
        // Reserved alphabetic keys
        assert!(is_reserved_key('g'));
        assert!(is_reserved_key('G'));
        assert!(is_reserved_key('k'));

        // Built-in action keys shadowed by branch/project views
        assert!(is_reserved_key('n'));
        assert!(is_reserved_key('s'));
        assert!(is_reserved_key('d'));

        // Reserved digits
        assert!(is_reserved_key('0'));
        assert!(is_reserved_key('5'));
        assert!(is_reserved_key('9'));

        // Codex configs key
        assert!(is_reserved_key('x'));

        // Freed when q stopped being a key at all (navigation is Esc-only)
        assert!(!is_reserved_key('q'));

        // Non-reserved keys
        assert!(!is_reserved_key('v'));
        assert!(!is_reserved_key('e'));

        // Freed when the focus timer was removed
        assert!(!is_reserved_key('t'));
        assert!(!is_reserved_key('T'));

        // Never actually bound: nothing enters session mode with 'i'
        assert!(!is_reserved_key('i'));
    }

    #[test]
    fn test_config_add_shortcut() {
        let mut config = Config::default();
        let shortcut =
            CustomShortcut::new('v', "VSCode".to_string(), "code . &".to_string(), false);

        assert!(config.add_shortcut(shortcut).is_ok());
        assert_eq!(config.custom_shortcuts.len(), 1);
    }

    #[test]
    fn test_config_add_shortcut_reserved_key() {
        let mut config = Config::default();
        let shortcut = CustomShortcut::new('n', "New".to_string(), "true".to_string(), false);

        assert!(config.add_shortcut(shortcut).is_err());
    }

    #[test]
    fn test_config_add_shortcut_duplicate_key() {
        let mut config = Config::default();
        let shortcut1 =
            CustomShortcut::new('v', "VSCode".to_string(), "code . &".to_string(), false);
        let shortcut2 = CustomShortcut::new('v', "Vim".to_string(), "vim .".to_string(), false);

        assert!(config.add_shortcut(shortcut1).is_ok());
        assert!(config.add_shortcut(shortcut2).is_err());
    }

    #[test]
    fn test_config_get_shortcut() {
        let mut config = Config::default();
        let shortcut =
            CustomShortcut::new('v', "VSCode".to_string(), "code . &".to_string(), false);
        config.add_shortcut(shortcut).unwrap();

        assert!(config.get_shortcut('v').is_some());
        assert!(config.get_shortcut('x').is_none());
    }

    #[test]
    fn test_config_remove_shortcut() {
        let mut config = Config::default();
        let shortcut =
            CustomShortcut::new('v', "VSCode".to_string(), "code . &".to_string(), false);
        config.add_shortcut(shortcut).unwrap();

        let removed = config.remove_shortcut(0);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().key, 'v');
        assert!(config.custom_shortcuts.is_empty());
    }

    #[test]
    fn test_config_serialization_with_custom_shortcuts() {
        let mut config = Config::default();
        config
            .add_shortcut(CustomShortcut::new(
                'v',
                "VSCode".to_string(),
                "code . &".to_string(),
                false,
            ))
            .unwrap();

        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.custom_shortcuts.len(), 1);
        assert_eq!(parsed.custom_shortcuts[0].key, 'v');
        assert_eq!(parsed.custom_shortcuts[0].name, "VSCode");
        assert_eq!(parsed.custom_shortcuts[0].command, "code . &");
    }

    #[test]
    fn test_custom_shortcut_auto_close_defaults_false() {
        // Old config without auto_close field should deserialize with auto_close = false
        let toml_str = r#"
[[custom_shortcuts]]
key = "v"
name = "VSCode"
command = "code . &"
"#;
        let parsed: Config = toml::from_str(&format!(
            "hook_port = 9999\nworktrees_dir = '/tmp/wt'\nhooks_dir = '/tmp/hooks'\nmax_output_lines = 100\n{}",
            toml_str
        ))
        .unwrap();
        assert_eq!(parsed.custom_shortcuts.len(), 1);
        assert!(!parsed.custom_shortcuts[0].auto_close);
    }

    #[test]
    fn test_custom_shortcut_auto_close_serialization() {
        let shortcut = CustomShortcut::new('v', "VSCode".to_string(), "code . &".to_string(), true);
        assert!(shortcut.auto_close);

        let mut config = Config::default();
        config.custom_shortcuts.push(shortcut);
        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert!(parsed.custom_shortcuts[0].auto_close);
    }
}
