//! Which agent binary a session launched, and whether it has changed since
//!
//! Claude Code, Codex and Antigravity all update themselves while a session
//! keeps running the old build. Two install shapes have to be caught:
//!
//! - **Symlink swap.** The native Claude installer keeps every build under
//!   `~/.local/share/claude/versions/<version>` and repoints the
//!   `~/.local/bin/claude` symlink at the new one. The canonical path changes.
//! - **In-place overwrite.** npm and Homebrew (`codex`, an npm-installed
//!   `claude`) and Antigravity's self-updater replace the file the command
//!   resolves to, at the same path. The canonical path stays put; the file's
//!   inode, modification time or size is what moves.
//!
//! A [`BinaryFingerprint`] records all of them at spawn, and re-checking it is
//! one `canonicalize` plus one `stat` of the path the command resolved to -
//! cheap enough for the UI thread when throttled.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The agent executable a session was launched from, as it was at launch
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryFingerprint {
    /// The path the command resolved to on `PATH` - what gets re-checked
    ///
    /// Kept rather than re-searching `PATH` each time, so a re-check stays a
    /// single `canonicalize` and `stat`.
    pub resolved: PathBuf,
    /// Where `resolved` leads once every symlink is followed
    pub canonical: PathBuf,
    /// Identity and contents of the file at `canonical`
    file: FileStamp,
}

/// The parts of a file's metadata that change when it is replaced
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    /// Inode number; a replace-by-rename gets a new one. Zero off Unix.
    inode: u64,
    modified: Option<SystemTime>,
    len: u64,
}

impl FileStamp {
    fn of(meta: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        let inode = {
            use std::os::unix::fs::MetadataExt;
            meta.ino()
        };
        #[cfg(not(unix))]
        let inode = 0;
        Self {
            inode,
            modified: meta.modified().ok(),
            len: meta.len(),
        }
    }
}

impl BinaryFingerprint {
    /// Fingerprint the executable `command` resolves to on the current `PATH`
    pub fn locate(command: &str) -> Option<Self> {
        let path = std::env::var_os("PATH");
        Self::of(&find_on_path(command, path.as_deref())?)
    }

    /// Fingerprint the file at `resolved`, following symlinks
    ///
    /// `None` when it cannot be read - including mid-update, when the old file
    /// is gone and the new one not yet in place.
    pub fn of(resolved: &Path) -> Option<Self> {
        let canonical = std::fs::canonicalize(resolved).ok()?;
        let meta = std::fs::metadata(&canonical).ok()?;
        Some(Self {
            resolved: resolved.to_path_buf(),
            canonical,
            file: FileStamp::of(&meta),
        })
    }

    /// The same path, looked at again now
    pub fn refresh(&self) -> Option<Self> {
        Self::of(&self.resolved)
    }

    /// Whether `now` describes a different executable than this launch did
    ///
    /// Any change counts: a new symlink target, or a file at the same path
    /// that was replaced or rewritten.
    pub fn is_superseded_by(&self, now: &Self) -> bool {
        self.canonical != now.canonical || self.file != now.file
    }
}

/// Find `command` the way a shell would: as given if it names a path, else
/// the first executable file of that name in `path`'s directories
pub fn find_on_path(command: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    if command.contains(std::path::MAIN_SEPARATOR) {
        let candidate = PathBuf::from(command);
        return is_executable(&candidate).then_some(candidate);
    }
    std::env::split_paths(path?)
        .map(|dir| dir.join(command))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_executable(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// The native Claude installer's shape: a stable symlink repointed at a
    /// new versioned build
    #[cfg(unix)]
    #[test]
    fn test_a_symlink_swap_is_detected() {
        let temp = TempDir::new().unwrap();
        let versions = temp.path().join("versions");
        let old = versions.join("2.1.0");
        let new = versions.join("2.1.1");
        // Same content and size on purpose: only the target moves
        write_executable(&old, "#!/bin/sh\n");
        write_executable(&new, "#!/bin/sh\n");

        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let link = bin.join("claude");
        std::os::unix::fs::symlink(&old, &link).unwrap();

        let launched = BinaryFingerprint::of(&link).unwrap();
        assert_eq!(launched.canonical, std::fs::canonicalize(&old).unwrap());
        assert!(!launched.is_superseded_by(&launched.refresh().unwrap()));

        // Repoint the link the way an installer does: a new link renamed
        // over the old one
        let staged = bin.join(".claude.tmp");
        std::os::unix::fs::symlink(&new, &staged).unwrap();
        std::fs::rename(&staged, &link).unwrap();

        let now = launched.refresh().unwrap();
        assert_eq!(now.canonical, std::fs::canonicalize(&new).unwrap());
        assert!(launched.is_superseded_by(&now));
    }

    /// npm and Homebrew's shape: the file at the same path is replaced
    #[test]
    fn test_an_in_place_replacement_is_detected() {
        let temp = TempDir::new().unwrap();
        let bin = temp.path().join("codex");
        write_executable(&bin, "#!/bin/sh\necho old\n");
        let launched = BinaryFingerprint::of(&bin).unwrap();

        let staged = temp.path().join("codex.new");
        write_executable(&staged, "#!/bin/sh\necho new build\n");
        std::fs::rename(&staged, &bin).unwrap();

        let now = launched.refresh().unwrap();
        assert_eq!(now.canonical, launched.canonical);
        assert!(launched.is_superseded_by(&now));
    }

    #[test]
    fn test_a_binary_missing_mid_update_is_not_a_change() {
        let temp = TempDir::new().unwrap();
        let bin = temp.path().join("agy");
        write_executable(&bin, "#!/bin/sh\n");
        let launched = BinaryFingerprint::of(&bin).unwrap();

        std::fs::remove_file(&bin).unwrap();
        assert_eq!(launched.refresh(), None);
    }

    #[test]
    fn test_find_on_path_takes_the_first_executable_match() {
        let temp = TempDir::new().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        // Not executable, so it is skipped the way a shell would skip it
        std::fs::write(first.join("claude"), "").unwrap();
        write_executable(&second.join("claude"), "#!/bin/sh\n");

        let path = std::env::join_paths([&first, &second]).unwrap();
        #[cfg(unix)]
        assert_eq!(
            find_on_path("claude", Some(&path)),
            Some(second.join("claude"))
        );
        assert_eq!(find_on_path("missing", Some(&path)), None);
        assert_eq!(find_on_path("claude", None), None);
    }

    #[test]
    fn test_find_on_path_takes_a_path_as_given() {
        let temp = TempDir::new().unwrap();
        let bin = temp.path().join("claude");
        write_executable(&bin, "#!/bin/sh\n");
        let command = bin.to_str().unwrap();
        assert_eq!(find_on_path(command, None), Some(bin.clone()));
    }
}
