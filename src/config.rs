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
}

impl CustomShortcut {
    /// Create a new custom shortcut
    pub fn new(key: char, name: String, command: String) -> Self {
        Self { key, name, command }
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
/// - t, T: focus timer
/// - k: manage shortcuts (this feature)
/// - 0-9: jump to session by number
/// - Space: jump to attention
/// - Esc, Enter, Tab: navigation
const RESERVED_KEYS: &[char] = &['q', 'i', 'g', 'G', 't', 'T', 'k'];
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

    /// Default focus timer duration in minutes (default: 25)
    #[serde(default = "default_focus_timer_minutes")]
    pub focus_timer_minutes: u64,

    /// Focus stats retention in days (default: 30)
    #[serde(default = "default_focus_stats_retention_days")]
    pub focus_stats_retention_days: u64,

    /// Maximum scrollback lines per session (default: 10000)
    /// Each 1000 lines uses approximately 10KB of memory
    #[serde(default = "default_scrollback_lines")]
    pub scrollback_lines: usize,

    /// Custom shell session shortcuts
    ///
    /// Each shortcut defines a key that spawns a shell session with a predefined command.
    #[serde(default)]
    pub custom_shortcuts: Vec<CustomShortcut>,
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

fn default_focus_timer_minutes() -> u64 {
    25 // Pomodoro-style default
}

fn default_focus_stats_retention_days() -> u64 {
    30
}

fn default_scrollback_lines() -> usize {
    10_000
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
            focus_timer_minutes: default_focus_timer_minutes(),
            focus_stats_retention_days: default_focus_stats_retention_days(),
            scrollback_lines: default_scrollback_lines(),
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
        let mut config = Config::default();
        config.scrollback_lines = 5000;

        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.scrollback_lines, 5000);
    }

    // Custom shortcut tests
    #[test]
    fn test_custom_shortcut_display_name_with_name() {
        let shortcut = CustomShortcut::new('v', "VSCode".to_string(), "code . &".to_string());
        assert_eq!(shortcut.display_name(), "VSCode");
    }

    #[test]
    fn test_custom_shortcut_display_name_without_name() {
        let shortcut = CustomShortcut::new('v', String::new(), "code . &".to_string());
        assert_eq!(shortcut.display_name(), "code . &");
    }

    #[test]
    fn test_custom_shortcut_short_display_name() {
        let shortcut = CustomShortcut::new('v', "VSCodeEditor".to_string(), "code . &".to_string());
        assert_eq!(shortcut.short_display_name(), "VSCode");
    }

    #[test]
    fn test_is_reserved_key() {
        // Reserved alphabetic keys
        assert!(is_reserved_key('q'));
        assert!(is_reserved_key('g'));
        assert!(is_reserved_key('G'));
        assert!(is_reserved_key('t'));
        assert!(is_reserved_key('T'));
        assert!(is_reserved_key('k'));

        // Reserved digits
        assert!(is_reserved_key('0'));
        assert!(is_reserved_key('5'));
        assert!(is_reserved_key('9'));

        // Non-reserved keys
        assert!(!is_reserved_key('v'));
        assert!(!is_reserved_key('e'));
        assert!(!is_reserved_key('x'));
    }

    #[test]
    fn test_config_add_shortcut() {
        let mut config = Config::default();
        let shortcut = CustomShortcut::new('v', "VSCode".to_string(), "code . &".to_string());

        assert!(config.add_shortcut(shortcut).is_ok());
        assert_eq!(config.custom_shortcuts.len(), 1);
    }

    #[test]
    fn test_config_add_shortcut_reserved_key() {
        let mut config = Config::default();
        let shortcut = CustomShortcut::new('q', "Quit".to_string(), "exit".to_string());

        assert!(config.add_shortcut(shortcut).is_err());
    }

    #[test]
    fn test_config_add_shortcut_duplicate_key() {
        let mut config = Config::default();
        let shortcut1 = CustomShortcut::new('v', "VSCode".to_string(), "code . &".to_string());
        let shortcut2 = CustomShortcut::new('v', "Vim".to_string(), "vim .".to_string());

        assert!(config.add_shortcut(shortcut1).is_ok());
        assert!(config.add_shortcut(shortcut2).is_err());
    }

    #[test]
    fn test_config_get_shortcut() {
        let mut config = Config::default();
        let shortcut = CustomShortcut::new('v', "VSCode".to_string(), "code . &".to_string());
        config.add_shortcut(shortcut).unwrap();

        assert!(config.get_shortcut('v').is_some());
        assert!(config.get_shortcut('x').is_none());
    }

    #[test]
    fn test_config_remove_shortcut() {
        let mut config = Config::default();
        let shortcut = CustomShortcut::new('v', "VSCode".to_string(), "code . &".to_string());
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
            ))
            .unwrap();

        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.custom_shortcuts.len(), 1);
        assert_eq!(parsed.custom_shortcuts[0].key, 'v');
        assert_eq!(parsed.custom_shortcuts[0].name, "VSCode");
        assert_eq!(parsed.custom_shortcuts[0].command, "code . &");
    }
}
