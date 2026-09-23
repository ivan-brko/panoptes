//! Where each Codex account's `CODEX_HOME` really is
//!
//! Every Codex account normally has a `CODEX_HOME` of its own, so everything
//! Codex keeps - rollouts, the state databases, thread names, skills, plugins -
//! is split per account, and a conversation started under one account cannot
//! be resumed under another. With `codex_shared_history` on, each account that
//! does not already live in the shared home is spawned from a *shadow* home,
//! `~/.panoptes/codex-homes/<account-id>/`, in which:
//!
//! - `auth.json` is a symlink to the account's *own* `auth.json`. Never a copy:
//!   a copy forks the ChatGPT refresh token, and whichever copy refreshes first
//!   logs the other out.
//! - a fixed allow-list of entries ([`SHARED_ENTRIES`]) are symlinks into the
//!   shared home. A list rather than "whatever the shared home holds", so an
//!   entry a newer Codex adds is private until someone decides otherwise.
//! - everything else Codex creates there stays private.
//!
//! The SQLite databases are shared through `CODEX_SQLITE_HOME` rather than by
//! symlink ([`CodexHomes::sqlite_home_for`]): Codex creates some of them lazily,
//! and a database that did not exist when the shadow was built would otherwise
//! be created, privately, in the shadow.
//!
//! Rules the layout lives by, all learned from Codex 0.156.1:
//!
//! - **Shadows are never deleted or moved.** Codex records each thread's
//!   rollout path *through the shadow* in the shared state database, and
//!   resuming trusts that path. Removing an account drops only its
//!   `auth.json` link ([`CodexHomes::forget_account`]).
//! - **Real data is never clobbered.** Healing replaces a missing or wrong
//!   link; a real directory where a link belongs is logged and left alone. The
//!   one real file that is folded back is `session_index.jsonl`, which
//!   `codex delete` rewrites by rename and so turns into a private copy.
//! - **The accounts' own homes are never written.** They are only read: for
//!   `auth.json`'s existence and for `config.toml`, to warn when an account's
//!   settings differ from the shared ones it will now run with.
//!
//! [`CodexHomes`] is also the one resolver for where an account's
//! conversations are *read* from - the transcript watcher, conversation-ID
//! discovery, the resume check and thread titles all ask it. With the flag off
//! it hands back exactly the home each caller used before.

use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::config::Config;

/// Directory under `~/.panoptes/` that holds the shadow homes
pub const SHADOWS_DIR_NAME: &str = "codex-homes";

/// Shadow directory name for a session with no account profile
const DEFAULT_SHADOW_NAME: &str = "default";

/// Whether a shared entry is a directory or a file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
}

/// Entries a shadow home links into the shared home
///
/// The approved allow-list, deliberately fixed. `thread-writer-locks` is the
/// one that *must* be here: it is Codex's cross-process "one writer per
/// thread" lock, and kept private it would let two accounts append to one
/// rollout at once. `config.toml` is shared so the `notify` chain and hooks
/// are written once, through the link (Codex resolves the link before its own
/// atomic writes, so it keeps it).
pub const SHARED_ENTRIES: &[(&str, EntryKind)] = &[
    ("sessions", EntryKind::Dir),
    ("archived_sessions", EntryKind::Dir),
    ("thread-writer-locks", EntryKind::Dir),
    ("session_index.jsonl", EntryKind::File),
    ("history.jsonl", EntryKind::File),
    ("skills", EntryKind::Dir),
    ("plugins", EntryKind::Dir),
    ("rules", EntryKind::Dir),
    ("worktrees", EntryKind::Dir),
    ("cache", EntryKind::Dir),
    ("mcp-oauth-locks", EntryKind::Dir),
    (".tmp", EntryKind::Dir),
    ("config.toml", EntryKind::File),
];

/// The account's credentials, linked to its own home
const AUTH_FILE: &str = "auth.json";
/// Shared settings, compared against each account's own
const CONFIG_FILE: &str = "config.toml";
/// The one shared file Codex can turn into a private copy
const SESSION_INDEX_FILE: &str = "session_index.jsonl";

/// Top-level `config.toml` keys not worth a divergence warning
///
/// `notify` is Panoptes' own chain, rewritten into whichever file is in use;
/// `notice` is Codex remembering which one-off notices it has shown.
const DIVERGENCE_IGNORED_KEYS: &[&str] = &["notify", "notice"];

/// Resolve a path for comparison, tolerating symlinks and missing paths
fn canonical_or_original(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Expand a leading `~` in a configured path
fn expand_tilde(path: &Path) -> PathBuf {
    PathBuf::from(shellexpand::tilde(&path.to_string_lossy()).into_owned())
}

/// The one resolver for Codex homes
///
/// Built once from the config: `codex_shared_history` is read at startup
/// only, since flipping it under a running session would split that session's
/// view of its own history.
#[derive(Debug, Clone)]
pub struct CodexHomes {
    enabled: bool,
    /// The home every account shares while enabled
    shared_home: PathBuf,
    /// Where shadow homes are kept (`~/.panoptes/codex-homes`)
    shadows_dir: PathBuf,
    /// The home of a session with no account directory (`~/.codex`)
    default_home: PathBuf,
}

/// What building or healing a shadow home did
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ShadowReport {
    /// Links made for the first time
    pub created: Vec<String>,
    /// Links that were missing their target or pointed elsewhere, now fixed
    pub repaired: Vec<String>,
    /// Things the user should know about, which healing did not fix
    pub warnings: Vec<String>,
}

impl ShadowReport {
    /// Log what happened to one account's shadow
    fn log(&self, account: &str, shadow: &Path) {
        if !self.created.is_empty() {
            tracing::info!(
                account,
                shadow = %shadow.display(),
                entries = ?self.created,
                "Linked Codex shadow home into the shared home"
            );
        }
        if !self.repaired.is_empty() {
            tracing::warn!(
                account,
                shadow = %shadow.display(),
                entries = ?self.repaired,
                "Repaired Codex shadow home links"
            );
        }
        for warning in &self.warnings {
            tracing::warn!(account, shadow = %shadow.display(), "{}", warning);
        }
    }
}

impl CodexHomes {
    /// The resolver the running app uses
    pub fn from_config(config: &Config) -> Self {
        let default_home = crate::transcript::default_codex_home();
        let shared_home = config
            .codex_shared_home
            .as_deref()
            .map(expand_tilde)
            .unwrap_or_else(|| default_home.clone());
        Self::new(
            config.codex_shared_history,
            shared_home,
            crate::config::config_dir().join(SHADOWS_DIR_NAME),
            default_home,
        )
    }

    /// A resolver over explicit directories, which is what tests use
    pub fn new(
        enabled: bool,
        shared_home: PathBuf,
        shadows_dir: PathBuf,
        default_home: PathBuf,
    ) -> Self {
        Self {
            enabled,
            shared_home,
            shadows_dir,
            default_home,
        }
    }

    /// Whether accounts share one history
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The home every account shares while enabled
    pub fn shared_home(&self) -> &Path {
        &self.shared_home
    }

    /// Where an account's conversations are read from, `None` meaning default
    ///
    /// Off: the account's own home, exactly as given - so every caller sees
    /// what it saw before this existed. On: the shared home, canonicalised,
    /// because two accounts reading one tree must key their caches (the
    /// watcher's subagent scan, its thread-name followers) by one path.
    pub fn data_home_for(&self, account_home: Option<&Path>) -> Option<PathBuf> {
        if !self.enabled {
            return account_home.map(Path::to_path_buf);
        }
        Some(canonical_or_original(&self.shared_home))
    }

    /// Where an account's conversations are read from
    pub fn data_home(&self, account_home: Option<&Path>) -> PathBuf {
        self.data_home_for(account_home)
            .unwrap_or_else(|| self.default_home.clone())
    }

    /// Where an account's rollouts are filed (`<data home>/sessions`)
    ///
    /// The home is canonicalised before `sessions` is appended, never after:
    /// in a shadow `sessions` is itself a link, and resolving it would lose
    /// the home that `session_index.jsonl` sits beside.
    pub fn sessions_dir(&self, account_home: Option<&Path>) -> PathBuf {
        self.data_home(account_home).join("sessions")
    }

    /// The shadow home of an account
    pub fn shadow_home(&self, account_id: Option<Uuid>) -> PathBuf {
        let name = account_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| DEFAULT_SHADOW_NAME.to_string());
        self.shadows_dir.join(name)
    }

    /// An account's own home, `None` meaning the default
    fn original_home(&self, account_home: Option<&Path>) -> PathBuf {
        account_home
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.default_home.clone())
    }

    /// Whether an account already lives in the shared home
    ///
    /// Such an account needs no shadow: it is spawned in the shared home
    /// directly, with its credentials where they always were.
    pub fn is_direct(&self, account_home: Option<&Path>) -> bool {
        canonical_or_original(&self.original_home(account_home))
            == canonical_or_original(&self.shared_home)
    }

    /// The `CODEX_HOME` to spawn an account's Codex with
    ///
    /// Off, or for an account already in the shared home, the account's own
    /// home unchanged. Otherwise its shadow, built or healed first - every
    /// spawn heals, so a link Codex or the user broke is fixed by the next
    /// session rather than by a restart.
    pub fn prepare_spawn(
        &self,
        account_id: Option<Uuid>,
        account_home: Option<&Path>,
    ) -> Result<Option<PathBuf>> {
        if !self.enabled || self.is_direct(account_home) {
            return Ok(account_home.map(Path::to_path_buf));
        }
        let shadow = self.shadow_home(account_id);
        let original = self.original_home(account_home);
        let report = ensure_shadow(&shadow, &self.shared_home, &original)
            .with_context(|| format!("preparing Codex shadow home {}", shadow.display()))?;
        report.log(
            &account_id.map_or_else(|| DEFAULT_SHADOW_NAME.to_string(), |id| id.to_string()),
            &shadow,
        );
        Ok(Some(shadow))
    }

    /// The `CODEX_SQLITE_HOME` a spawn under `codex_home` needs, if any
    ///
    /// Set for a shadow, and only a shadow: it is what puts the state
    /// databases - including the ones Codex creates lazily - in the shared
    /// home. Derived from the home rather than carried alongside it, because
    /// "spawned from a shadow" is exactly the condition.
    pub fn sqlite_home_for(&self, codex_home: &Path) -> Option<PathBuf> {
        (self.enabled && codex_home.starts_with(&self.shadows_dir))
            .then(|| canonical_or_original(&self.shared_home))
    }

    /// Build or heal every account's shadow, returning what to warn about
    ///
    /// Run once at startup when enabled, so a divergent `config.toml` or a
    /// real directory in the way is reported before a session trips on it.
    /// Each warning names its account.
    pub fn heal_all<'a>(
        &self,
        accounts: impl IntoIterator<Item = (Uuid, &'a str, Option<&'a Path>)>,
    ) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }
        let mut warnings = Vec::new();
        for (id, name, home) in accounts {
            if self.is_direct(home) {
                continue;
            }
            let shadow = self.shadow_home(Some(id));
            match ensure_shadow(&shadow, &self.shared_home, &self.original_home(home)) {
                Ok(report) => {
                    report.log(name, &shadow);
                    warnings.extend(
                        report
                            .warnings
                            .into_iter()
                            .map(|w| format!("Codex account '{}': {}", name, w)),
                    );
                }
                Err(e) => {
                    tracing::error!(account = name, error = %e, "Failed to prepare Codex shadow home");
                    warnings.push(format!(
                        "Codex account '{}': shadow home could not be prepared: {:#}",
                        name, e
                    ));
                }
            }
        }
        warnings
    }

    /// Drop a removed account's credentials from its shadow
    ///
    /// Only the `auth.json` *link* goes, and only if it is a link. The rest of
    /// the shadow stays: the shared state database refers to rollouts through
    /// it, and deleting it would make those conversations unresumable from
    /// every other account. Runs whether or not sharing is enabled now - a
    /// shadow left from an earlier enabled spell must not keep a login alive.
    pub fn forget_account(&self, account_id: Uuid) {
        let link = self.shadow_home(Some(account_id)).join(AUTH_FILE);
        let is_link = std::fs::symlink_metadata(&link)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        if !is_link {
            return;
        }
        match std::fs::remove_file(&link) {
            Ok(()) => tracing::info!(
                account = %account_id,
                "Removed the deleted Codex account's credentials link from its shadow home"
            ),
            Err(e) => tracing::warn!(
                account = %account_id,
                error = %e,
                "Failed to remove the deleted Codex account's credentials link"
            ),
        }
    }
}

/// Build or heal one shadow home
///
/// Idempotent: a second run over a correct shadow changes nothing. `original`
/// is only read.
pub fn ensure_shadow(shadow: &Path, shared: &Path, original: &Path) -> Result<ShadowReport> {
    let mut report = ShadowReport::default();

    std::fs::create_dir_all(shared)
        .with_context(|| format!("creating shared Codex home {}", shared.display()))?;
    // Links point at the resolved shared home, so a shared home reached
    // through a symlink yields the same links as one named directly
    let shared = canonical_or_original(shared);
    std::fs::create_dir_all(shadow)
        .with_context(|| format!("creating shadow home {}", shadow.display()))?;

    for &(name, kind) in SHARED_ENTRIES {
        let target = shared.join(name);
        ensure_target(&target, kind)?;
        link_shared_entry(&shadow.join(name), &target, name, &mut report)?;
    }

    link_auth(shadow, original, &mut report)?;

    let differing = config_divergence(&original.join(CONFIG_FILE), &shared.join(CONFIG_FILE));
    if !differing.is_empty() {
        report.warnings.push(format!(
            "its own config.toml differs from the shared one in [{}]; with shared history it \
             runs with the shared settings",
            differing.join(", ")
        ));
    }

    Ok(report)
}

/// Make sure a shared entry exists, so the link to it is never dangling
///
/// Pre-creating is what stops Codex creating a missing entry privately. A
/// file is created empty and never truncated.
fn ensure_target(target: &Path, kind: EntryKind) -> Result<()> {
    if std::fs::symlink_metadata(target).is_ok() {
        return Ok(());
    }
    match kind {
        EntryKind::Dir => std::fs::create_dir_all(target),
        EntryKind::File => std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(target)
            .map(|_| ()),
    }
    .with_context(|| format!("creating shared entry {}", target.display()))
}

/// Point `link` at `target`, repairing it if it points anywhere else
fn link_shared_entry(
    link: &Path,
    target: &Path,
    name: &str,
    report: &mut ShadowReport,
) -> Result<()> {
    let meta = match std::fs::symlink_metadata(link) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            make_symlink(target, link)?;
            report.created.push(name.to_string());
            return Ok(());
        }
        Err(e) => return Err(e).with_context(|| format!("inspecting {}", link.display())),
    };

    if meta.file_type().is_symlink() {
        let current =
            std::fs::read_link(link).with_context(|| format!("reading {}", link.display()))?;
        if current == target {
            return Ok(());
        }
        std::fs::remove_file(link).with_context(|| format!("removing {}", link.display()))?;
        make_symlink(target, link)?;
        report
            .repaired
            .push(format!("{} (pointed at {})", name, current.display()));
        return Ok(());
    }

    if meta.is_dir() {
        report.warnings.push(format!(
            "{} is a real directory in the shadow home, not a link to the shared home; left \
             alone, so it is not shared",
            link.display()
        ));
        return Ok(());
    }

    if name == SESSION_INDEX_FILE {
        // `codex delete` rewrites the index by rename, which replaces the
        // link with a private copy. Its new lines go back into the shared
        // file, then the link is restored.
        let merged = append_missing_lines(link, target)?;
        std::fs::remove_file(link).with_context(|| format!("removing {}", link.display()))?;
        make_symlink(target, link)?;
        report.repaired.push(name.to_string());
        report.warnings.push(format!(
            "{} had become a private copy (Codex rewrites it on delete); merged {} line(s) back \
             into the shared index and relinked it",
            link.display(),
            merged
        ));
        return Ok(());
    }

    report.warnings.push(format!(
        "{} is a real file in the shadow home, not a link to the shared home; left alone, so \
         it is not shared",
        link.display()
    ));
    Ok(())
}

/// Link the shadow's `auth.json` to the account's own
///
/// A link rather than a copy, because Codex refreshes tokens by rewriting
/// the file in place: through a link both homes see the refresh, while a copy
/// would fork the refresh token. A real `auth.json` in the shadow is a login
/// made under the shadow itself, and is kept.
fn link_auth(shadow: &Path, original: &Path, report: &mut ShadowReport) -> Result<()> {
    let link = shadow.join(AUTH_FILE);
    let target = original.join(AUTH_FILE);

    match std::fs::symlink_metadata(&link) {
        Ok(meta) if meta.file_type().is_symlink() => {
            let current =
                std::fs::read_link(&link).with_context(|| format!("reading {}", link.display()))?;
            if current != target {
                std::fs::remove_file(&link)
                    .with_context(|| format!("removing {}", link.display()))?;
                make_symlink(&target, &link)?;
                report
                    .repaired
                    .push(format!("{} (pointed at {})", AUTH_FILE, current.display()));
            }
        }
        // The user logged in under the shadow itself; that login is theirs
        Ok(meta) if meta.is_file() => return Ok(()),
        Ok(_) => {
            report.warnings.push(format!(
                "{} is not a file or a link; left alone",
                link.display()
            ));
            return Ok(());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if target.exists() {
                make_symlink(&target, &link)?;
                report.created.push(AUTH_FILE.to_string());
            }
        }
        Err(e) => return Err(e).with_context(|| format!("inspecting {}", link.display())),
    }

    if !target.exists() {
        // Keyring (or `auto`) credentials are keyed on the canonical
        // CODEX_HOME, so a shadow can never inherit them
        report.warnings.push(format!(
            "{} has no auth.json to share; if this account keeps its login in the OS keyring, \
             log in once with `CODEX_HOME={} codex login`",
            original.display(),
            shadow.display()
        ));
    }
    Ok(())
}

fn make_symlink(target: &Path, link: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
            .with_context(|| format!("linking {} -> {}", link.display(), target.display()))
    }
    #[cfg(not(unix))]
    {
        let _ = (target, link);
        anyhow::bail!("shared Codex history needs symlinks, which this platform lacks")
    }
}

/// Append to `into` every line of `from` it does not already hold
///
/// Returns how many lines were appended. Order is kept, which is what thread
/// names need: the later of two lines for a thread wins.
pub(crate) fn append_missing_lines(from: &Path, into: &Path) -> Result<usize> {
    let incoming =
        std::fs::read_to_string(from).with_context(|| format!("reading {}", from.display()))?;
    append_lines(into, incoming.lines())
}

/// Append to `into` each of `lines` it does not already hold, in order
pub(crate) fn append_lines<'a>(
    into: &Path,
    lines: impl IntoIterator<Item = &'a str>,
) -> Result<usize> {
    let existing = std::fs::read_to_string(into).unwrap_or_default();
    let held: std::collections::HashSet<&str> = existing.lines().collect();

    let missing: Vec<&str> = lines
        .into_iter()
        .filter(|line| !line.trim().is_empty() && !held.contains(line))
        .collect();
    if missing.is_empty() {
        return Ok(0);
    }

    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(into)
        .with_context(|| format!("opening {}", into.display()))?;
    // An existing last line with no newline would otherwise be glued to ours
    let mut text = String::new();
    if !existing.is_empty() && !existing.ends_with('\n') {
        text.push('\n');
    }
    for line in &missing {
        text.push_str(line);
        text.push('\n');
    }
    out.write_all(text.as_bytes())
        .with_context(|| format!("appending to {}", into.display()))?;
    Ok(missing.len())
}

/// Top-level `config.toml` keys whose values differ between two files
///
/// Empty when either file is missing or unreadable: an account with no config
/// of its own loses nothing by running with the shared one.
pub fn config_divergence(own: &Path, shared: &Path) -> Vec<String> {
    let read = |path: &Path| -> Option<toml::Table> {
        let text = std::fs::read_to_string(path).ok()?;
        match toml::from_str::<toml::Table>(&text) {
            Ok(table) => Some(table),
            Err(e) => {
                tracing::debug!(path = %path.display(), error = %e, "Unparseable Codex config.toml");
                None
            }
        }
    };
    let (Some(own), Some(shared)) = (read(own), read(shared)) else {
        return Vec::new();
    };

    let mut keys: Vec<String> = own
        .keys()
        .chain(shared.keys())
        .filter(|key| !DIVERGENCE_IGNORED_KEYS.contains(&key.as_str()))
        .filter(|key| own.get(*key) != shared.get(*key))
        .cloned()
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A shared home, a shadows dir and two account homes, all in a temp dir
    struct Layout {
        _tmp: TempDir,
        root: PathBuf,
        shared: PathBuf,
        shadows: PathBuf,
        account_a: PathBuf,
        account_b: PathBuf,
    }

    fn layout() -> Layout {
        let tmp = TempDir::new().unwrap();
        // Canonical, so paths compare equal to what the resolver hands back
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let shared = root.join("shared");
        let shadows = root.join("panoptes/codex-homes");
        let account_a = root.join("account-a");
        let account_b = root.join("account-b");
        for dir in [&shared, &account_a, &account_b] {
            std::fs::create_dir_all(dir).unwrap();
        }
        // Fake credentials only
        std::fs::write(
            account_a.join(AUTH_FILE),
            r#"{"OPENAI_API_KEY":"sk-fake-a"}"#,
        )
        .unwrap();
        std::fs::write(
            account_b.join(AUTH_FILE),
            r#"{"OPENAI_API_KEY":"sk-fake-b"}"#,
        )
        .unwrap();
        Layout {
            _tmp: tmp,
            root,
            shared,
            shadows,
            account_a,
            account_b,
        }
    }

    fn homes(l: &Layout, enabled: bool) -> CodexHomes {
        CodexHomes::new(
            enabled,
            l.shared.clone(),
            l.shadows.clone(),
            l.root.join("default-home"),
        )
    }

    /// Every file and directory under `dir`, with contents, for comparison
    fn snapshot(dir: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let meta = std::fs::symlink_metadata(&path).unwrap();
                if meta.is_dir() {
                    out.push((path.clone(), None));
                    walk(&path, out);
                } else {
                    out.push((path.clone(), std::fs::read(&path).ok()));
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, &mut out);
        out.sort();
        out
    }

    fn is_link_to(link: &Path, target: &Path) -> bool {
        std::fs::symlink_metadata(link).is_ok_and(|m| m.file_type().is_symlink())
            && std::fs::read_link(link).unwrap() == target
    }

    #[test]
    fn test_the_shared_allow_list_is_the_approved_one() {
        // Changing this list changes what accounts share. It is pinned so that
        // happens on purpose, after checking the Codex version in use.
        let names: Vec<&str> = SHARED_ENTRIES.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            [
                "sessions",
                "archived_sessions",
                "thread-writer-locks",
                "session_index.jsonl",
                "history.jsonl",
                "skills",
                "plugins",
                "rules",
                "worktrees",
                "cache",
                "mcp-oauth-locks",
                ".tmp",
                "config.toml",
            ]
        );
        // Credentials and per-process state must never be on it
        for private in [
            AUTH_FILE,
            "models_cache.json",
            "log",
            "tmp",
            "ipc",
            "shell_snapshots",
            "memories",
            "installation_id",
            "version.json",
        ] {
            assert!(!names.contains(&private), "{private} must stay private");
        }
        // No SQLite file is linked: CODEX_SQLITE_HOME shares those
        assert!(names.iter().all(|name| !name.ends_with(".sqlite")));
    }

    #[test]
    fn test_builder_creates_the_links_and_the_auth_link() {
        let l = layout();
        let shadow = l.shadows.join("a");

        let report = ensure_shadow(&shadow, &l.shared, &l.account_a).unwrap();

        for (name, kind) in SHARED_ENTRIES {
            let link = shadow.join(name);
            assert!(is_link_to(&link, &l.shared.join(name)), "{name} not linked");
            // Pre-created, so no link dangles and nothing goes private
            let target = std::fs::metadata(&link).unwrap();
            assert_eq!(target.is_dir(), *kind == EntryKind::Dir, "{name}");
        }
        assert!(is_link_to(
            &shadow.join(AUTH_FILE),
            &l.account_a.join(AUTH_FILE)
        ));
        assert_eq!(
            std::fs::read_to_string(shadow.join(AUTH_FILE)).unwrap(),
            r#"{"OPENAI_API_KEY":"sk-fake-a"}"#
        );
        assert_eq!(report.created.len(), SHARED_ENTRIES.len() + 1);
        assert!(report.repaired.is_empty());
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    #[test]
    fn test_builder_is_idempotent() {
        let l = layout();
        let shadow = l.shadows.join("a");
        ensure_shadow(&shadow, &l.shared, &l.account_a).unwrap();
        let before = snapshot(&l.root);

        let report = ensure_shadow(&shadow, &l.shared, &l.account_a).unwrap();

        assert_eq!(report, ShadowReport::default());
        assert_eq!(snapshot(&l.root), before);
    }

    #[test]
    fn test_builder_repairs_a_missing_or_wrong_link() {
        let l = layout();
        let shadow = l.shadows.join("a");
        ensure_shadow(&shadow, &l.shared, &l.account_a).unwrap();

        // One link gone, one pointing somewhere else, one dangling elsewhere
        std::fs::remove_file(shadow.join("skills")).unwrap();
        std::fs::remove_file(shadow.join("sessions")).unwrap();
        make_symlink(&l.account_b.join("sessions"), &shadow.join("sessions")).unwrap();
        std::fs::remove_file(shadow.join(AUTH_FILE)).unwrap();
        make_symlink(&l.account_b.join(AUTH_FILE), &shadow.join(AUTH_FILE)).unwrap();

        let report = ensure_shadow(&shadow, &l.shared, &l.account_a).unwrap();

        assert!(is_link_to(&shadow.join("skills"), &l.shared.join("skills")));
        assert!(is_link_to(
            &shadow.join("sessions"),
            &l.shared.join("sessions")
        ));
        assert!(is_link_to(
            &shadow.join(AUTH_FILE),
            &l.account_a.join(AUTH_FILE)
        ));
        assert_eq!(report.created, vec!["skills".to_string()]);
        assert_eq!(report.repaired.len(), 2, "{:?}", report.repaired);
    }

    #[test]
    fn test_builder_never_replaces_a_real_directory() {
        let l = layout();
        let shadow = l.shadows.join("a");
        let private = shadow.join("sessions/2026/09/23");
        std::fs::create_dir_all(&private).unwrap();
        std::fs::write(private.join("rollout-x.jsonl"), "precious\n").unwrap();

        let report = ensure_shadow(&shadow, &l.shared, &l.account_a).unwrap();

        let meta = std::fs::symlink_metadata(shadow.join("sessions")).unwrap();
        assert!(meta.is_dir() && !meta.file_type().is_symlink());
        assert_eq!(
            std::fs::read_to_string(private.join("rollout-x.jsonl")).unwrap(),
            "precious\n"
        );
        assert!(report.warnings.iter().any(|w| w.contains("real directory")));
        // The rest were linked regardless
        assert!(is_link_to(&shadow.join("skills"), &l.shared.join("skills")));
    }

    #[test]
    fn test_builder_never_touches_the_original_home() {
        let l = layout();
        std::fs::create_dir_all(l.account_a.join("sessions/2026/01/01")).unwrap();
        std::fs::write(
            l.account_a.join("sessions/2026/01/01/rollout-old.jsonl"),
            "old\n",
        )
        .unwrap();
        std::fs::write(l.account_a.join(CONFIG_FILE), "model = \"o3\"\n").unwrap();
        let before = snapshot(&l.account_a);

        let homes = homes(&l, true);
        let shadow = homes
            .prepare_spawn(Some(Uuid::new_v4()), Some(&l.account_a))
            .unwrap()
            .unwrap();
        // Healing again, after Codex has written through the links
        std::fs::write(shadow.join("config.toml"), "model = \"gpt-5\"\n").unwrap();
        let _ = homes.heal_all([(Uuid::new_v4(), "A", Some(l.account_a.as_path()))]);

        assert_eq!(snapshot(&l.account_a), before);
        // The write went to the shared file, through the link
        assert_eq!(
            std::fs::read_to_string(l.shared.join(CONFIG_FILE)).unwrap(),
            "model = \"gpt-5\"\n"
        );
    }

    #[test]
    fn test_a_private_session_index_is_merged_back_and_relinked() {
        let l = layout();
        let shadow = l.shadows.join("a");
        ensure_shadow(&shadow, &l.shared, &l.account_a).unwrap();
        let old = r#"{"id":"t1","thread_name":"one","updated_at":"2026-09-23T10:00:00Z"}"#;
        let new = r#"{"id":"t2","thread_name":"two","updated_at":"2026-09-23T11:00:00Z"}"#;
        std::fs::write(l.shared.join(SESSION_INDEX_FILE), format!("{old}\n")).unwrap();

        // What `codex delete` leaves behind: a rewritten private copy
        std::fs::remove_file(shadow.join(SESSION_INDEX_FILE)).unwrap();
        std::fs::write(shadow.join(SESSION_INDEX_FILE), format!("{old}\n{new}\n")).unwrap();

        let report = ensure_shadow(&shadow, &l.shared, &l.account_a).unwrap();

        assert!(is_link_to(
            &shadow.join(SESSION_INDEX_FILE),
            &l.shared.join(SESSION_INDEX_FILE)
        ));
        assert_eq!(
            std::fs::read_to_string(l.shared.join(SESSION_INDEX_FILE)).unwrap(),
            format!("{old}\n{new}\n")
        );
        assert!(report.warnings.iter().any(|w| w.contains("merged 1 line")));
    }

    #[test]
    fn test_another_real_file_in_a_link_slot_is_left_alone() {
        let l = layout();
        let shadow = l.shadows.join("a");
        std::fs::create_dir_all(&shadow).unwrap();
        std::fs::write(shadow.join("history.jsonl"), "mine\n").unwrap();

        let report = ensure_shadow(&shadow, &l.shared, &l.account_a).unwrap();

        assert_eq!(
            std::fs::read_to_string(shadow.join("history.jsonl")).unwrap(),
            "mine\n"
        );
        assert!(report.warnings.iter().any(|w| w.contains("real file")));
    }

    #[test]
    fn test_a_login_made_under_the_shadow_is_kept() {
        let l = layout();
        let shadow = l.shadows.join("a");
        std::fs::create_dir_all(&shadow).unwrap();
        std::fs::write(shadow.join(AUTH_FILE), "shadow login").unwrap();

        ensure_shadow(&shadow, &l.shared, &l.account_a).unwrap();

        assert_eq!(
            std::fs::read_to_string(shadow.join(AUTH_FILE)).unwrap(),
            "shadow login"
        );
    }

    #[test]
    fn test_an_account_with_no_auth_file_gets_no_dangling_link_and_a_warning() {
        let l = layout();
        std::fs::remove_file(l.account_b.join(AUTH_FILE)).unwrap();
        let shadow = l.shadows.join("b");

        let report = ensure_shadow(&shadow, &l.shared, &l.account_b).unwrap();

        assert!(std::fs::symlink_metadata(shadow.join(AUTH_FILE)).is_err());
        assert!(report.warnings.iter().any(|w| w.contains("codex login")));
    }

    #[test]
    fn test_a_divergent_config_is_reported_by_key() {
        let l = layout();
        std::fs::write(
            l.account_a.join(CONFIG_FILE),
            "model = \"o3\"\nforced_login_method = \"chatgpt\"\nnotify = [\"a\"]\n",
        )
        .unwrap();
        std::fs::write(
            l.shared.join(CONFIG_FILE),
            "model = \"o3\"\nnotify = [\"b\"]\n[notice]\nx = true\n",
        )
        .unwrap();

        let report = ensure_shadow(&l.shadows.join("a"), &l.shared, &l.account_a).unwrap();

        assert_eq!(
            config_divergence(&l.account_a.join(CONFIG_FILE), &l.shared.join(CONFIG_FILE)),
            vec!["forced_login_method".to_string()]
        );
        assert!(report
            .warnings
            .iter()
            .any(|w| w.contains("[forced_login_method]")));
    }

    #[test]
    fn test_off_resolves_every_home_exactly_as_before_and_writes_nothing() {
        let l = layout();
        let homes = homes(&l, false);
        let before = snapshot(&l.root);

        assert_eq!(homes.data_home_for(None), None);
        assert_eq!(
            homes.data_home_for(Some(&l.account_a)),
            Some(l.account_a.clone())
        );
        assert_eq!(
            homes.sessions_dir(Some(&l.account_a)),
            l.account_a.join("sessions")
        );
        assert_eq!(
            homes
                .prepare_spawn(Some(Uuid::new_v4()), Some(&l.account_a))
                .unwrap(),
            Some(l.account_a.clone())
        );
        assert_eq!(homes.prepare_spawn(None, None).unwrap(), None);
        assert_eq!(homes.sqlite_home_for(&l.account_a), None);
        assert!(homes
            .heal_all([(Uuid::new_v4(), "A", Some(l.account_a.as_path()))])
            .is_empty());

        assert_eq!(snapshot(&l.root), before);
    }

    #[test]
    fn test_on_every_account_reads_the_shared_tree() {
        let l = layout();
        let homes = homes(&l, true);

        for account in [
            Some(l.account_a.as_path()),
            Some(l.account_b.as_path()),
            None,
        ] {
            assert_eq!(homes.data_home(account), l.shared);
            assert_eq!(homes.sessions_dir(account), l.shared.join("sessions"));
        }
    }

    #[test]
    fn test_on_the_shared_home_is_canonicalised_but_its_sessions_link_is_not() {
        let l = layout();
        // The shared home reached through a symlink
        let alias = l.root.join("alias");
        make_symlink(&l.shared, &alias).unwrap();
        let homes = CodexHomes::new(true, alias, l.shadows.clone(), l.root.join("d"));

        assert_eq!(homes.sessions_dir(None), l.shared.join("sessions"));
    }

    #[test]
    fn test_on_a_shadow_spawns_with_the_shared_sqlite_home() {
        let l = layout();
        let homes = homes(&l, true);
        let id = Uuid::new_v4();

        let spawn_home = homes
            .prepare_spawn(Some(id), Some(&l.account_a))
            .unwrap()
            .unwrap();

        assert_eq!(spawn_home, l.shadows.join(id.to_string()));
        assert_eq!(homes.sqlite_home_for(&spawn_home), Some(l.shared.clone()));
    }

    #[test]
    fn test_on_an_account_already_in_the_shared_home_spawns_directly() {
        let l = layout();
        let homes = homes(&l, true);

        assert_eq!(
            homes
                .prepare_spawn(Some(Uuid::new_v4()), Some(&l.shared))
                .unwrap(),
            Some(l.shared.clone())
        );
        assert_eq!(homes.sqlite_home_for(&l.shared), None);
        assert!(!l.shadows.exists());
    }

    #[test]
    fn test_a_conversation_from_one_account_is_found_under_another() {
        // The cross-account resume: A's rollout, written through A's shadow,
        // is what B's resume check and B's `codex resume` both look for
        let l = layout();
        let homes = homes(&l, true);
        let shadow_a = homes
            .prepare_spawn(Some(Uuid::new_v4()), Some(&l.account_a))
            .unwrap()
            .unwrap();
        let shadow_b = homes
            .prepare_spawn(Some(Uuid::new_v4()), Some(&l.account_b))
            .unwrap()
            .unwrap();

        let id = "019a0000-0000-7000-8000-00000000000a";
        let day = shadow_a.join("sessions/2026/09/23");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(
            day.join(format!("rollout-2026-09-23T10-00-00-{id}.jsonl")),
            format!(
                r#"{{"timestamp":"2026-09-23T10:00:00Z","type":"session_meta","payload":{{"id":"{id}","timestamp":"2026-09-23T10:00:00Z","cwd":"/tmp"}}}}"#
            ) + "\n",
        )
        .unwrap();

        // Found from B's resolved home, and through B's own shadow
        assert!(
            crate::agent::codex::rollout_path(&homes.data_home(Some(&l.account_b)), id).is_some()
        );
        assert!(crate::agent::codex::rollout_path(&shadow_b, id).is_some());
    }

    #[test]
    fn test_forgetting_an_account_drops_only_its_auth_link() {
        let l = layout();
        let homes = homes(&l, true);
        let id = Uuid::new_v4();
        let shadow = homes
            .prepare_spawn(Some(id), Some(&l.account_a))
            .unwrap()
            .unwrap();

        homes.forget_account(id);

        assert!(std::fs::symlink_metadata(shadow.join(AUTH_FILE)).is_err());
        assert!(l.account_a.join(AUTH_FILE).exists());
        assert!(is_link_to(
            &shadow.join("sessions"),
            &l.shared.join("sessions")
        ));
    }

    #[test]
    fn test_forgetting_keeps_a_login_made_under_the_shadow() {
        let l = layout();
        let homes = homes(&l, false);
        let id = Uuid::new_v4();
        let shadow = homes.shadow_home(Some(id));
        std::fs::create_dir_all(&shadow).unwrap();
        std::fs::write(shadow.join(AUTH_FILE), "real").unwrap();

        homes.forget_account(id);

        assert!(shadow.join(AUTH_FILE).exists());
    }

    #[test]
    fn test_heal_all_names_the_account_in_each_warning() {
        let l = layout();
        std::fs::remove_file(l.account_b.join(AUTH_FILE)).unwrap();
        let homes = homes(&l, true);

        let warnings = homes.heal_all([
            (Uuid::new_v4(), "Work", Some(l.account_a.as_path())),
            (Uuid::new_v4(), "Personal", Some(l.account_b.as_path())),
            (Uuid::new_v4(), "Main", Some(l.shared.as_path())),
        ]);

        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].starts_with("Codex account 'Personal'"));
    }

    #[test]
    fn test_append_missing_lines_keeps_order_and_skips_what_is_there() {
        let tmp = TempDir::new().unwrap();
        let from = tmp.path().join("from");
        let into = tmp.path().join("into");
        std::fs::write(&from, "a\nb\nc\n").unwrap();
        std::fs::write(&into, "b").unwrap();

        assert_eq!(append_missing_lines(&from, &into).unwrap(), 2);
        assert_eq!(std::fs::read_to_string(&into).unwrap(), "b\na\nc\n");
        assert_eq!(append_missing_lines(&from, &into).unwrap(), 0);
    }
}
