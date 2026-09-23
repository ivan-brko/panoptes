//! Copying an account's old Codex history into the shared home
//!
//! Turning on `codex_shared_history` leaves every account's own home alone,
//! history included, so conversations from before are not in the shared tree.
//! This is the explicit, one-off way to bring them over: `panoptes
//! merge-codex-history [ACCOUNT]`. It only ever *copies*. The originals stay
//! where they were, which is what keeps turning the flag off a full undo.
//!
//! Only rollouts and thread names are copied. Codex lists and resumes a
//! rollout that has no row in its state database, rebuilding what it needs
//! from the file, so nothing is done at the database level.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::homes::{append_lines, CodexHomes};
use super::CodexConfigStore;
use crate::config::Config;

/// The directories rollouts are filed under, relative to a Codex home
const ROLLOUT_DIRS: &[&str] = &["sessions", "archived_sessions"];

/// How deep to look below a rollout directory (`YYYY/MM/DD/file`)
const MAX_DEPTH: usize = 4;

/// The command-line name of the action
pub const COMMAND: &str = "merge-codex-history";

/// What a merge did
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MergeReport {
    /// Rollouts copied into the shared home
    pub copied: usize,
    /// Rollouts the shared home already had, left as they were
    pub already_there: usize,
    /// Thread-name lines added to the shared `session_index.jsonl`
    pub index_lines: usize,
    /// Files that could not be copied, with why
    pub failures: Vec<String>,
}

/// Copy `source`'s rollouts and thread names into `shared`
///
/// Never overwrites: a rollout already in the shared tree is left alone.
/// Rollout names carry the conversation's UUIDv7, so a clash means the same
/// conversation, not a different one. A copy is written under a temporary name
/// and renamed into place, so Codex never lists a half-written rollout.
pub fn merge_history(source: &Path, shared: &Path) -> Result<MergeReport> {
    let mut report = MergeReport::default();
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    if canonical(source) == canonical(shared) {
        return Ok(report);
    }

    for dir in ROLLOUT_DIRS {
        let from_root = source.join(dir);
        let into_root = shared.join(dir);
        for file in rollout_files_below(&from_root) {
            let Ok(relative) = file.strip_prefix(&from_root) else {
                continue;
            };
            let dest = into_root.join(relative);
            if std::fs::symlink_metadata(&dest).is_ok() {
                report.already_there += 1;
                continue;
            }
            match copy_into_place(&file, &dest) {
                Ok(()) => report.copied += 1,
                Err(e) => report.failures.push(format!("{}: {:#}", file.display(), e)),
            }
        }
    }

    report.index_lines = merge_session_index(
        &source.join("session_index.jsonl"),
        &shared.join("session_index.jsonl"),
    )?;

    Ok(report)
}

/// Every `rollout-*.jsonl` below `root`, not following symlinks
///
/// A symlinked directory is skipped rather than followed: in a shadow home
/// `sessions` is a link to the shared tree, and following it would "merge"
/// the shared tree into itself.
fn rollout_files_below(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if depth > MAX_DEPTH {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                walk(&path, depth + 1, out);
            } else if kind.is_file() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("rollout-") && name.ends_with(".jsonl") {
                    out.push(path);
                }
            }
        }
    }
    let is_real_dir = std::fs::symlink_metadata(root).is_ok_and(|m| m.is_dir());
    let mut out = Vec::new();
    if is_real_dir {
        walk(root, 0, &mut out);
    }
    out.sort();
    out
}

/// Copy one rollout to `dest` via a temporary name, keeping its mtime
///
/// The mtime is kept because it is a rollout's "last updated" - Codex sorts
/// its resume list by it, and the transcript watcher counts a recently written
/// rollout as a live subagent - and a fresh copy would look brand new.
fn copy_into_place(from: &Path, dest: &Path) -> Result<()> {
    let parent = dest.parent().context("rollout destination has no parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let name = dest
        .file_name()
        .context("rollout destination has no file name")?
        .to_string_lossy();
    // Not `rollout-*`, so Codex never picks up the partial copy
    let tmp = parent.join(format!(".{}.panoptes-merge", name));

    let result = (|| -> Result<()> {
        std::fs::copy(from, &tmp).context("copying")?;
        keep_mtime(from, &tmp);
        std::fs::rename(&tmp, dest).context("moving the copy into place")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Give `to` the modification time of `from`, best effort
///
/// Through `touch -r` because the std call for it (`File::set_modified`) is
/// newer than the crate's minimum Rust version. A copy that keeps a fresh
/// mtime is still a correct copy, so failure is only logged.
fn keep_mtime(from: &Path, to: &Path) {
    let status = std::process::Command::new("touch")
        .arg("-r")
        .arg(from)
        .arg(to)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if !status.is_ok_and(|s| s.success()) {
        tracing::debug!(path = %to.display(), "Could not keep the merged rollout's mtime");
    }
}

/// Add `from`'s thread names for threads the shared index does not know
///
/// A thread already named in the shared index keeps that name: the shared
/// file is where it has been renamed since, if anywhere.
fn merge_session_index(from: &Path, into: &Path) -> Result<usize> {
    let Ok(incoming) = std::fs::read_to_string(from) else {
        return Ok(0);
    };
    let thread_id = |line: &str| -> Option<String> {
        serde_json::from_str::<serde_json::Value>(line)
            .ok()?
            .get("id")?
            .as_str()
            .map(str::to_string)
    };
    let known: HashSet<String> = std::fs::read_to_string(into)
        .unwrap_or_default()
        .lines()
        .filter_map(thread_id)
        .collect();

    append_lines(
        into,
        incoming
            .lines()
            .filter(|line| thread_id(line).is_some_and(|id| !known.contains(&id))),
    )
}

/// Run `panoptes merge-codex-history [ACCOUNT]`, printing what it did
///
/// With no account named, merges every account that is not already the shared
/// home. Returns whether every copy succeeded.
pub fn run_command(account: Option<&str>) -> Result<bool> {
    let (config, config_warning) = Config::load_with_status();
    if let Some(warning) = config_warning {
        eprintln!("{}", warning);
    }
    let (store, store_warning) = CodexConfigStore::load_with_status();
    if let Some(warning) = store_warning {
        eprintln!("{}", warning);
    }
    let homes = CodexHomes::from_config(&config);
    let shared = homes.shared_home().to_path_buf();

    let accounts: Vec<_> = store
        .configs_sorted()
        .into_iter()
        .filter(|c| account.map_or(true, |name| c.name == name))
        .collect();
    if let Some(name) = account {
        if accounts.is_empty() {
            anyhow::bail!("no Codex account named '{}'", name);
        }
    }

    println!("Shared Codex home: {}", shared.display());
    if !homes.enabled() {
        println!("Note: codex_shared_history is off, so sessions do not read the shared home yet.");
    }

    let mut all_ok = true;
    let mut merged_any = false;
    for config in accounts {
        let home = config
            .codex_home
            .clone()
            .unwrap_or_else(crate::transcript::default_codex_home);
        if homes.is_direct(config.codex_home.as_deref()) {
            continue;
        }
        merged_any = true;
        let report = merge_history(&home, &shared)
            .with_context(|| format!("merging history of '{}'", config.name))?;
        println!(
            "{} ({}): copied {} conversation(s), {} already there, {} thread name(s)",
            config.name,
            home.display(),
            report.copied,
            report.already_there,
            report.index_lines
        );
        for failure in &report.failures {
            all_ok = false;
            eprintln!("  failed: {}", failure);
        }
    }
    if !merged_any {
        println!("Nothing to merge: no account keeps its history outside the shared home.");
    }
    Ok(all_ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn tree(root: &Path) -> Vec<(PathBuf, String)> {
        let mut out: Vec<_> = walkdir(root)
            .into_iter()
            .map(|p| {
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                (p.strip_prefix(root).unwrap().to_path_buf(), text)
            })
            .collect();
        out.sort();
        out
    }

    fn walkdir(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                out.extend(walkdir(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    #[test]
    fn test_merge_copies_rollouts_and_never_touches_the_source() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("account");
        let shared = tmp.path().join("shared");
        write(
            &source.join("sessions/2026/01/02/rollout-2026-01-02T00-00-00-aaa.jsonl"),
            "a\n",
        );
        write(
            &source.join("archived_sessions/rollout-2025-12-01T00-00-00-bbb.jsonl"),
            "b\n",
        );
        write(
            &source.join("sessions/2026/01/02/notes.txt"),
            "not a rollout",
        );
        write(&source.join("auth.json"), "fake");
        let before = tree(&source);

        let report = merge_history(&source, &shared).unwrap();

        assert_eq!(report.copied, 2);
        assert_eq!(
            std::fs::read_to_string(
                shared.join("sessions/2026/01/02/rollout-2026-01-02T00-00-00-aaa.jsonl")
            )
            .unwrap(),
            "a\n"
        );
        assert!(shared
            .join("archived_sessions/rollout-2025-12-01T00-00-00-bbb.jsonl")
            .exists());
        assert!(!shared.join("auth.json").exists());
        assert!(!shared.join("sessions/2026/01/02/notes.txt").exists());
        assert_eq!(tree(&source), before);
    }

    #[test]
    fn test_merge_never_overwrites_and_is_repeatable() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("account");
        let shared = tmp.path().join("shared");
        let rel = "sessions/2026/01/02/rollout-2026-01-02T00-00-00-aaa.jsonl";
        write(&source.join(rel), "old\n");
        write(&shared.join(rel), "newer, with more turns\n");

        let report = merge_history(&source, &shared).unwrap();
        assert_eq!((report.copied, report.already_there), (0, 1));
        assert_eq!(
            std::fs::read_to_string(shared.join(rel)).unwrap(),
            "newer, with more turns\n"
        );
        // No temporary copies left behind
        assert_eq!(walkdir(&shared.join("sessions")).len(), 1);
    }

    #[test]
    fn test_merge_keeps_the_rollout_mtime() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("account");
        let shared = tmp.path().join("shared");
        let rel = "sessions/2025/01/01/rollout-2025-01-01T00-00-00-aaa.jsonl";
        write(&source.join(rel), "a\n");
        let status = std::process::Command::new("touch")
            .args(["-t", "202311141213.20"])
            .arg(source.join(rel))
            .status()
            .unwrap();
        assert!(status.success());
        let old = std::fs::metadata(source.join(rel))
            .unwrap()
            .modified()
            .unwrap();
        assert!(old < std::time::SystemTime::now() - std::time::Duration::from_secs(86_400));

        merge_history(&source, &shared).unwrap();

        let copied = std::fs::metadata(shared.join(rel))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(copied, old);
    }

    #[test]
    fn test_merge_adds_names_only_for_threads_the_shared_index_lacks() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("account");
        let shared = tmp.path().join("shared");
        let line = |id: &str, name: &str| format!(r#"{{"id":"{id}","thread_name":"{name}"}}"#);
        write(
            &source.join("session_index.jsonl"),
            &format!("{}\n{}\n", line("t1", "old name"), line("t2", "two")),
        );
        write(
            &shared.join("session_index.jsonl"),
            &format!("{}\n", line("t1", "renamed since")),
        );

        let report = merge_history(&source, &shared).unwrap();

        assert_eq!(report.index_lines, 1);
        assert_eq!(
            std::fs::read_to_string(shared.join("session_index.jsonl")).unwrap(),
            format!("{}\n{}\n", line("t1", "renamed since"), line("t2", "two"))
        );
    }

    #[test]
    fn test_merging_the_shared_home_into_itself_does_nothing() {
        let tmp = TempDir::new().unwrap();
        let shared = tmp.path().join("shared");
        write(&shared.join("sessions/2026/01/01/rollout-x.jsonl"), "x\n");

        assert_eq!(
            merge_history(&shared, &shared).unwrap(),
            MergeReport::default()
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_merge_does_not_follow_a_linked_sessions_dir() {
        // A shadow home's `sessions` is a link to the shared tree
        let tmp = TempDir::new().unwrap();
        let shared = tmp.path().join("shared");
        let shadow = tmp.path().join("shadow");
        write(&shared.join("sessions/2026/01/01/rollout-x.jsonl"), "x\n");
        std::fs::create_dir_all(&shadow).unwrap();
        std::os::unix::fs::symlink(shared.join("sessions"), shadow.join("sessions")).unwrap();

        let report = merge_history(&shadow, &shared).unwrap();

        assert_eq!((report.copied, report.already_there), (0, 0));
    }
}
