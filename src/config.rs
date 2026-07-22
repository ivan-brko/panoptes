//! Configuration management for Panoptes

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

/// Reserved keys that cannot be used for custom shortcuts in session view
///
/// These keys are already bound to functionality in session view normal mode:
/// - q: quit/back
/// - i: enter session mode (if used)
/// - g, G: scroll to top/bottom
/// - k: manage shortcuts (this feature)
/// - 0-9: jump to session by number
/// - Space: jump to attention
/// - Esc, Enter, Tab: navigation
const RESERVED_KEYS: &[char] = &['q', 'i', 'g', 'G', 'k', 'x'];
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

/// Categories of disk errors for user-friendly messages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskErrorKind {
    /// Disk is full or quota exceeded
    DiskFull,
    /// Permission denied (read or write)
    PermissionDenied,
    /// File or directory not found
    NotFound,
    /// Other IO error
    Other,
}

impl DiskErrorKind {
    /// Get a user-friendly message for this error kind
    pub fn user_message(&self) -> &'static str {
        match self {
            DiskErrorKind::DiskFull => "Disk full - free space needed to save",
            DiskErrorKind::PermissionDenied => "Permission denied writing to ~/.panoptes/",
            DiskErrorKind::NotFound => "File or directory not found",
            DiskErrorKind::Other => "Failed to save data",
        }
    }
}

/// Categorize an IO error into a user-friendly category
pub fn categorize_io_error(e: &std::io::Error) -> DiskErrorKind {
    use std::io::ErrorKind;

    match e.kind() {
        // Disk full errors
        ErrorKind::StorageFull => DiskErrorKind::DiskFull,
        // On some systems, disk full might appear as WriteZero or Other
        ErrorKind::WriteZero => DiskErrorKind::DiskFull,

        // Permission errors
        ErrorKind::PermissionDenied => DiskErrorKind::PermissionDenied,

        // Not found
        ErrorKind::NotFound => DiskErrorKind::NotFound,

        // Check raw OS error for disk full on Unix
        _ => {
            #[cfg(unix)]
            {
                if let Some(os_error) = e.raw_os_error() {
                    // ENOSPC (No space left on device) = 28 on Linux, 28 on macOS
                    // EDQUOT (Disk quota exceeded) = 122 on Linux, 69 on macOS
                    if os_error == 28 || os_error == 122 || os_error == 69 {
                        return DiskErrorKind::DiskFull;
                    }
                    // EACCES = 13 on both
                    if os_error == 13 {
                        return DiskErrorKind::PermissionDenied;
                    }
                }
            }
            DiskErrorKind::Other
        }
    }
}

/// Create a user-friendly error message from an IO error
pub fn friendly_io_error_message(e: &std::io::Error, context: &str) -> String {
    let kind = categorize_io_error(e);
    match kind {
        DiskErrorKind::DiskFull => format!("{}: {}", context, kind.user_message()),
        DiskErrorKind::PermissionDenied => format!("{}: {}", context, kind.user_message()),
        DiskErrorKind::NotFound => format!("{}: file or directory not found", context),
        DiskErrorKind::Other => format!("{}: {}", context, e),
    }
}

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Port for the hook HTTP server
    pub hook_port: u16,

    /// Directory for git worktrees
    pub worktrees_dir: PathBuf,

    /// Directory for hook scripts
    pub hooks_dir: PathBuf,

    /// Maximum lines to keep in output buffer per session
    pub max_output_lines: usize,

    /// Idle threshold in seconds before session is flagged as needing attention (default: 300 = 5 min)
    #[serde(default = "default_idle_threshold")]
    pub idle_threshold_secs: u64,

    /// State timeout in seconds - Executing states auto-transition to Idle after this (default: 300 = 5 min)
    #[serde(default = "default_state_timeout")]
    pub state_timeout_secs: u64,

    /// Exited session retention in seconds - sessions are cleaned up after this duration (default: 300 = 5 min)
    #[serde(default = "default_exited_retention")]
    pub exited_retention_secs: u64,

    /// Theme preset: "dark" (default), "light", or "high-contrast"
    #[serde(default = "default_theme_preset")]
    pub theme_preset: String,

    /// Notification method: "bell" (terminal bell), "title" (update terminal title), "none"
    #[serde(default = "default_notification_method")]
    pub notification_method: String,

    /// Esc hold threshold in milliseconds for exiting session mode (default: 400ms)
    #[serde(default = "default_esc_hold_threshold_ms")]
    pub esc_hold_threshold_ms: u64,

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

fn default_idle_threshold() -> u64 {
    300
}

fn default_state_timeout() -> u64 {
    300 // 5 minutes
}

fn default_exited_retention() -> u64 {
    300 // 5 minutes
}

fn default_theme_preset() -> String {
    "dark".to_string()
}

fn default_notification_method() -> String {
    "bell".to_string()
}

fn default_esc_hold_threshold_ms() -> u64 {
    400
}

fn default_scrollback_lines() -> usize {
    10_000
}

fn default_suspend_after() -> u64 {
    7200 // 2 hours
}

impl Default for Config {
    fn default() -> Self {
        let base = config_dir();
        Self {
            hook_port: 9999,
            worktrees_dir: base.join("worktrees"),
            hooks_dir: base.join("hooks"),
            max_output_lines: 10_000,
            idle_threshold_secs: default_idle_threshold(),
            state_timeout_secs: default_state_timeout(),
            exited_retention_secs: default_exited_retention(),
            theme_preset: default_theme_preset(),
            notification_method: default_notification_method(),
            esc_hold_threshold_ms: default_esc_hold_threshold_ms(),
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
    /// Load configuration from file, or return default if not found
    pub fn load() -> Result<Self> {
        let path = config_file_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path).context("Failed to read config file")?;
            toml::from_str(&content).context("Failed to parse config file")
        } else {
            Ok(Self::default())
        }
    }

    /// Save configuration to file
    pub fn save(&self) -> Result<()> {
        let path = config_file_path();
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;
        std::fs::write(&path, content).context("Failed to write config file")?;
        Ok(())
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
        assert_eq!(parsed.esc_hold_threshold_ms, original.esc_hold_threshold_ms);
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

        assert_eq!(parsed.max_output_lines, 500);
        assert_eq!(parsed.notification_method, "title");
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
        assert_eq!(config.max_output_lines, 10_000);
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
        assert!(is_reserved_key('q'));
        assert!(is_reserved_key('g'));
        assert!(is_reserved_key('G'));
        assert!(is_reserved_key('k'));

        // Reserved digits
        assert!(is_reserved_key('0'));
        assert!(is_reserved_key('5'));
        assert!(is_reserved_key('9'));

        // Codex configs key
        assert!(is_reserved_key('x'));

        // Non-reserved keys
        assert!(!is_reserved_key('v'));
        assert!(!is_reserved_key('e'));

        // Freed when the focus timer was removed
        assert!(!is_reserved_key('t'));
        assert!(!is_reserved_key('T'));
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
        let shortcut = CustomShortcut::new('q', "Quit".to_string(), "exit".to_string(), false);

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
