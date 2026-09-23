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
/// Custom shortcuts fire in the branch list and the session view, so a shortcut
/// sharing a key with something bound there could never run - the built-in arm
/// matches first. The list is deliberately a little wider than that, covering
/// keys bound in neighbouring levels too, so a shortcut does not mean one thing
/// on one screen and something else on the next:
/// - q: quit, handled globally in normal mode (and in session-view normal mode)
/// - n, s, d: new worktree/AI, shell, delete - bound in pane 1 and pane 2
/// - i: import an existing Claude/Codex conversation - bound at pane 1's
///   branch level, exactly where custom shortcuts fire
/// - 0-9: jump to session by number
///
/// `c`, `g`, `G`, `k` and `x` used to be here and are now free: configs,
/// shortcuts and the log viewer they belonged to have all moved into pane 3,
/// which is reached with `Tab` rather than a letter. `,` is free for the same
/// kind of reason: per-project settings are a row of the branch list now, so no
/// key opens them.
///
/// `m` and `r` stay unreserved: they are bound only in the projects overview,
/// where custom shortcuts do not fire.
///
/// `Space`, `Esc`, `Enter`, and `Tab` are not chars and cannot be bound at all.
const RESERVED_KEYS: &[char] = &['q', 'n', 's', 'd', 'i'];
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

    /// State timeout in seconds - a tool in flight this long stops being
    /// believed and is dropped from the session's in-flight set (default: 300 =
    /// 5 min). Whether that also flags the session is decided by liveness, not
    /// by this threshold; see `SessionManager::check_state_timeouts`.
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

    /// Characters a double-click treats as part of a word, beyond alphanumerics
    ///
    /// Defaults to iTerm2's own set, which is what makes double-clicking a
    /// path or a flag take the whole thing instead of stopping at the first
    /// slash or dash.
    #[serde(default = "default_selection_word_characters")]
    pub selection_word_characters: String,

    /// How long after a click a second one still counts as a double-click
    ///
    /// Milliseconds. Match it to the system's double-click speed if the
    /// default feels quick or slow.
    #[serde(default = "default_multi_click_ms")]
    pub multi_click_ms: u64,

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

    /// Whether Claude sessions route their status line through Panoptes
    ///
    /// Claude reports its plan rate limits, and the running session's real
    /// context window, only to a `statusLine` command. On (the default),
    /// Panoptes installs one in the working directory's
    /// `.claude/settings.local.json` that forwards the figures and then runs
    /// the user's own status line, so what Claude displays is unchanged. Off,
    /// Panoptes puts back whatever it wrapped and Claude sessions show no
    /// rate limits. Read when a session spawns.
    #[serde(default = "default_true")]
    pub claude_status_line: bool,

    /// Which colour-capability tier the UI palette uses
    ///
    /// `auto` (the default) detects it from `COLORTERM`/`TERM`; the other
    /// values force a tier for when detection is wrong - `ansi16` is the
    /// always-safe baseline.
    #[serde(default)]
    pub theme: ThemeMode,

    /// Which colour preset the UI wears
    ///
    /// Orthogonal to [`Self::theme`]: that picks how many colours the terminal
    /// can show, this picks which ones. `peacock` (the default) is the look
    /// Panoptes has always had.
    #[serde(default)]
    pub palette: Palette,

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

/// Which colour-capability tier the palette should use
///
/// Serialises as the lowercase strings `auto`, `truecolor`, `ansi256`,
/// `ansi16`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// Detect from `COLORTERM`/`TERM`
    #[default]
    Auto,
    /// Force the 24-bit RGB tier
    TrueColor,
    /// Force the 256-colour indexed tier
    Ansi256,
    /// Force the 16-colour baseline
    Ansi16,
}

/// Unknown values fall back to `Auto` rather than failing the whole load:
/// a typo in a hand-edited config should cost the typo, not the file.
impl<'de> Deserialize<'de> for ThemeMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "auto" => Self::Auto,
            "truecolor" => Self::TrueColor,
            "ansi256" | "256" => Self::Ansi256,
            "ansi16" | "16" => Self::Ansi16,
            other => {
                tracing::warn!(
                    value = %other,
                    "Unknown theme in config; defaulting to auto"
                );
                Self::Auto
            }
        })
    }
}

/// Which colour preset the UI wears
///
/// Named for the myth: Panoptes is Argus Panoptes, the all-seeing. A preset
/// restyles the chrome - the accent, the focused border, the selection
/// surface, the input prompt, the default markers - and never the semantics:
/// green still means waiting and red still means crashed in every one of
/// them, so a session list reads the same whichever you pick.
///
/// Serialises as the lowercase strings `peacock`, `io`, `hera`, `argus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Palette {
    /// Cyan and blue - the look Panoptes has always had
    #[default]
    Peacock,
    /// Warm amber and gold
    Io,
    /// Royal violet
    Hera,
    /// The watcher's own green
    Argus,
}

impl Palette {
    /// The presets in the order the picker offers them
    pub const ALL: [Palette; 4] = [Palette::Peacock, Palette::Io, Palette::Hera, Palette::Argus];

    /// Row label in the picker
    pub fn label(self) -> &'static str {
        match self {
            Palette::Peacock => "Peacock",
            Palette::Io => "Io",
            Palette::Hera => "Hera",
            Palette::Argus => "Argus",
        }
    }

    /// One-line description, shown in the global footer
    pub fn blurb(self) -> &'static str {
        match self {
            Palette::Peacock => "Cyan and blue - the hundred eyes on the tail",
            Palette::Io => "Warm amber and gold - the heifer he guarded",
            Palette::Hera => "Royal violet - the goddess he served",
            Palette::Argus => "Green - the watcher himself",
        }
    }

    /// Position in [`Palette::ALL`], for seeding the picker's highlight
    pub fn index(self) -> usize {
        Palette::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    /// The preset at `index` in the picker
    pub fn at(index: usize) -> Option<Palette> {
        Palette::ALL.get(index).copied()
    }
}

/// Unknown values fall back to `Peacock` rather than failing the whole load:
/// a typo in a hand-edited config should cost the typo, not the file.
impl<'de> Deserialize<'de> for Palette {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "peacock" => Self::Peacock,
            "io" => Self::Io,
            "hera" => Self::Hera,
            "argus" => Self::Argus,
            other => {
                tracing::warn!(
                    value = %other,
                    "Unknown palette in config; defaulting to peacock"
                );
                Self::Peacock
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

    /// A turn died on an API error (usage limit, expired login, overload)
    ///
    /// Its own key rather than folded into `crashed`: the process is alive and
    /// the fix is usually to wait or log in, not to restart anything, so
    /// someone may well want one and not the other.
    #[serde(default = "default_true")]
    pub failed: bool,
}

impl Default for NotifyOn {
    fn default() -> Self {
        Self {
            approval: true,
            turn_complete: true,
            stalled: false,
            crashed: true,
            failed: true,
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
            AttentionReason::TurnFailed { .. } => self.failed,
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

fn default_selection_word_characters() -> String {
    "/-+\\~_.".to_string()
}

fn default_multi_click_ms() -> u64 {
    400
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
            selection_word_characters: default_selection_word_characters(),
            multi_click_ms: default_multi_click_ms(),
            suspend_after_secs: default_suspend_after(),
            log_agent_events: false,
            attention_on_idle: false,
            claude_status_line: true,
            theme: ThemeMode::default(),
            palette: Palette::default(),
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

    /// Drop custom shortcuts bound to keys that have since become reserved
    ///
    /// `q` was a legal shortcut key before the three-pane layout gave it a
    /// meaning of its own. A shortcut on a key like that could never fire
    /// again - the built-in arm matches first - so it is dropped rather than
    /// silently shadowed. The reserved set only ever shrinks after that: `,`
    /// was reserved for per-project settings and is bindable again now that a
    /// row opens them, and a config binding it survives untouched.
    ///
    /// Returns a message naming what went, for the startup notice, or `None`
    /// when nothing had to be dropped.
    pub fn drop_reserved_shortcuts(&mut self) -> Option<String> {
        let dropped: Vec<String> = self
            .custom_shortcuts
            .iter()
            .filter(|s| is_reserved_key(s.key))
            .map(|s| format!("'{}' ({})", s.key, s.display_name()))
            .collect();
        if dropped.is_empty() {
            return None;
        }
        self.custom_shortcuts.retain(|s| !is_reserved_key(s.key));
        tracing::warn!(
            "Dropped {} custom shortcut(s) bound to now-reserved keys",
            dropped.len()
        );
        Some(format!(
            "Dropped {} custom shortcut{} bound to keys that are now reserved: {}.\n\
             Rebind {} from Settings > Shortcuts.",
            dropped.len(),
            if dropped.len() == 1 { "" } else { "s" },
            dropped.join(", "),
            if dropped.len() == 1 { "it" } else { "them" },
        ))
    }

    /// Save configuration to file (atomically, via a sibling temp file)
    ///
    /// The whole file is rewritten from the struct, so a hand-edited
    /// `config.toml` loses its comments and key order the first time anything
    /// writes it - which the Notifications section does on every keystroke.
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
        original.claude_status_line = false;
        original.custom_shortcuts.push(CustomShortcut::new(
            'v',
            "VSCode".to_string(),
            "code . &".to_string(),
            false,
        ));

        let text = toml::to_string_pretty(&original).expect("config must serialise");
        let parsed: Config = toml::from_str(&text).expect("config must round trip");

        assert!(parsed.attention_on_idle);
        assert!(!parsed.claude_status_line);
        assert!(!parsed.notify_on.turn_complete);
        assert!(parsed.notify_on.stalled);
        assert!(parsed.notify_on.approval);
        assert_eq!(parsed.custom_shortcuts.len(), 1);
        assert_eq!(parsed.scrollback_lines, original.scrollback_lines);
    }

    #[test]
    fn test_notify_on_failed_round_trips() {
        let mut original = Config::default();
        assert!(original.notify_on.failed, "failed turns ring by default");
        original.notify_on.failed = false;

        let text = toml::to_string_pretty(&original).expect("config must serialise");
        assert!(text.contains("failed = false"), "{text}");
        let parsed: Config = toml::from_str(&text).expect("config must round trip");
        assert!(!parsed.notify_on.failed);
        // Its neighbours are untouched
        assert!(parsed.notify_on.crashed);

        // A `[notify_on]` table written before the key existed keeps the default
        let older = "approval = true\ncrashed = false\n";
        let parsed: NotifyOn = toml::from_str(older).expect("older table must load");
        assert!(parsed.failed);
        assert!(!parsed.crashed);
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
        assert!(parsed.notify_on.failed);
        assert!(!parsed.attention_on_idle);
        // The status line is wrapped unless the user says otherwise
        assert!(parsed.claude_status_line);
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

    /// The reserved set after the three-pane layout: `q n s d` and digits
    #[test]
    fn test_is_reserved_key() {
        // Quit, from every pane and from session-view normal mode
        assert!(is_reserved_key('q'));

        // Built-in action keys shadowed by pane 1 and pane 2
        assert!(is_reserved_key('n'));
        assert!(is_reserved_key('s'));
        assert!(is_reserved_key('d'));

        // Freed again: per-project settings are the last row of the branch
        // list, so no key opens them
        assert!(!is_reserved_key(','), "',' should be bindable again");

        // Jump to session by number
        assert!(is_reserved_key('0'));
        assert!(is_reserved_key('5'));
        assert!(is_reserved_key('9'));

        // Freed by the three-pane layout: configs, shortcuts and the log
        // viewer all live in pane 3 now, reached with Tab rather than a letter
        for freed in ['c', 'g', 'G', 'k', 'x'] {
            assert!(!is_reserved_key(freed), "{freed} should be bindable now");
        }

        // Bound only in the projects overview, where shortcuts do not fire
        assert!(!is_reserved_key('m'));
        assert!(!is_reserved_key('r'));

        // Import a conversation, at the branch level where shortcuts fire
        assert!(is_reserved_key('i'));

        // Never bound at all
        assert!(!is_reserved_key('v'));
        assert!(!is_reserved_key('e'));
        assert!(!is_reserved_key('t'));
    }

    #[test]
    fn test_reserved_keys_display_lists_every_reserved_key() {
        let display = reserved_keys_display();
        for key in RESERVED_KEYS {
            assert!(display.contains(*key), "{key} missing from {display:?}");
        }
        assert!(display.contains("0-9"));
    }

    #[test]
    fn test_drop_reserved_shortcuts_removes_and_reports_them() {
        let mut config = Config::default();
        // Written by an older version, when q was still bindable
        config.custom_shortcuts.push(CustomShortcut::new(
            'q',
            "Quit".into(),
            "exit".into(),
            false,
        ));
        config.custom_shortcuts.push(CustomShortcut::new(
            'v',
            "VSCode".into(),
            "code . &".into(),
            false,
        ));

        let warning = config
            .drop_reserved_shortcuts()
            .expect("dropping must be reported, never silent");

        assert!(warning.contains("'q' (Quit)"), "{warning}");
        assert!(!warning.contains("VSCode"), "{warning}");
        assert_eq!(config.custom_shortcuts.len(), 1);
        assert_eq!(config.custom_shortcuts[0].key, 'v');
    }

    /// `,` was reserved while it opened per-project settings and is bindable
    /// again now a row does. The reverse of the migration that dropped it: a
    /// config that binds it is legal and must survive load untouched.
    #[test]
    fn test_a_shortcut_bound_to_the_freed_comma_key_survives() {
        let mut config = Config::default();
        let comma = CustomShortcut::new(',', "Notes".into(), "vim notes.md".into(), false);
        config.custom_shortcuts.push(comma);

        assert!(config.drop_reserved_shortcuts().is_none());
        assert_eq!(config.custom_shortcuts.len(), 1);
        assert_eq!(config.custom_shortcuts[0].key, ',');

        // ...and it can be added in the first place
        let mut fresh = Config::default();
        assert!(fresh
            .add_shortcut(CustomShortcut::new(
                ',',
                "Notes".into(),
                "vim notes.md".into(),
                false
            ))
            .is_ok());
    }

    #[test]
    fn test_drop_reserved_shortcuts_is_quiet_when_nothing_changes() {
        let mut config = Config::default();
        config.custom_shortcuts.push(CustomShortcut::new(
            'v',
            "VSCode".into(),
            "code . &".into(),
            false,
        ));

        assert!(config.drop_reserved_shortcuts().is_none());
        assert_eq!(config.custom_shortcuts.len(), 1);
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

    /// `palette` is a scalar, so it has to serialise *above* the first table
    /// header or TOML swallows it into that table. A round-trip through a
    /// config that also has tables is what proves it did.
    #[test]
    fn test_palette_survives_a_round_trip_past_the_table_header() {
        let mut config = Config {
            palette: Palette::Hera,
            ..Config::default()
        };
        config
            .add_shortcut(CustomShortcut::new(
                'v',
                String::new(),
                "code .".to_string(),
                false,
            ))
            .unwrap();

        let toml_str = toml::to_string(&config).unwrap();
        let palette_line = toml_str.find("palette").expect("palette not serialised");
        let first_table = toml_str.find('[').unwrap_or(toml_str.len());
        assert!(
            palette_line < first_table,
            "palette must sit above the first table header:\n{toml_str}"
        );

        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.palette, Palette::Hera);
    }

    #[test]
    fn test_unknown_palette_falls_back_to_peacock() {
        let parsed: Config = toml::from_str("palette = \"chartreuse\"").unwrap();
        assert_eq!(parsed.palette, Palette::Peacock);
        // And an absent key is the same as the default
        let parsed: Config = toml::from_str("").unwrap();
        assert_eq!(parsed.palette, Palette::Peacock);
    }

    #[test]
    fn test_every_palette_round_trips_through_its_config_string() {
        for palette in Palette::ALL {
            let text = toml::to_string(&Config {
                palette,
                ..Config::default()
            })
            .unwrap();
            let parsed: Config = toml::from_str(&text).unwrap();
            assert_eq!(parsed.palette, palette, "{palette:?}");
        }
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
