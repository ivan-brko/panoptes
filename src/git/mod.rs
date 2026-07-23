//! Git operations wrapper module
//!
//! Provides a safe wrapper around git2 for common repository operations.

pub mod worktree;

use anyhow::{Context, Result};
use git2::{BranchType, Repository};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How often a running `git fetch` is checked for exit and cancellation
const FETCH_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How a fetch ended
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchOutcome {
    /// `git fetch --all` ran to completion
    Completed,
    /// The caller cancelled the fetch before git finished
    Cancelled,
}

/// Wrapper for git repository operations
pub struct GitOps {
    repo: Repository,
}

impl GitOps {
    /// Open a git repository at the given path
    pub fn open(path: &Path) -> Result<Self> {
        let repo = Repository::open(path)
            .with_context(|| format!("Failed to open git repository at {:?}", path))?;
        Ok(Self { repo })
    }

    /// Discover a git repository from any path within it
    ///
    /// Starting from `path`, walks up the directory tree to find a .git directory.
    pub fn discover(path: &Path) -> Result<Self> {
        let repo = Repository::discover(path).map_err(|e| {
            // Provide user-friendly error messages for common cases
            let path_display = path.display();
            if e.code() == git2::ErrorCode::NotFound {
                anyhow::anyhow!(
                    "Not a git repository: {}. Initialize with 'git init' or clone an existing repository.",
                    path_display
                )
            } else {
                anyhow::anyhow!("Failed to open git repository at {}: {}", path_display, e)
            }
        })?;
        Ok(Self { repo })
    }

    /// Get the repository root path (workdir)
    pub fn workdir(&self) -> Option<&Path> {
        self.repo.workdir()
    }

    /// Get the repository path as a PathBuf
    pub fn repo_path(&self) -> PathBuf {
        self.repo
            .workdir()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| self.repo.path().to_path_buf())
    }

    /// Check if this is a bare repository
    pub fn is_bare(&self) -> bool {
        self.repo.is_bare()
    }

    /// Get the name of the default branch (HEAD reference)
    pub fn default_branch_name(&self) -> Result<String> {
        // Try to get the current HEAD
        let head = self.repo.head().context("Failed to get HEAD reference")?;

        if head.is_branch() {
            // Extract branch name from reference
            let name = head
                .shorthand()
                .ok_or_else(|| anyhow::anyhow!("Invalid branch name"))?;
            Ok(name.to_string())
        } else {
            // HEAD is detached, try to find default branch from remote
            self.find_default_remote_branch()
        }
    }

    /// Find the default branch from the remote (origin)
    fn find_default_remote_branch(&self) -> Result<String> {
        // Try common default branch names
        for name in &["main", "master"] {
            if self.repo.find_branch(name, BranchType::Local).is_ok() {
                return Ok(name.to_string());
            }
        }

        // Fall back to "main" if nothing found
        Ok("main".to_string())
    }

    /// Get the current branch name (if on a branch)
    pub fn current_branch(&self) -> Result<Option<String>> {
        let head = self.repo.head();
        match head {
            Ok(reference) => {
                if reference.is_branch() {
                    Ok(reference.shorthand().map(String::from))
                } else {
                    Ok(None) // Detached HEAD
                }
            }
            Err(_) => Ok(None), // No commits yet
        }
    }

    /// Get the remote URL for origin (if exists)
    pub fn remote_url(&self) -> Option<String> {
        self.repo
            .find_remote("origin")
            .ok()
            .and_then(|remote| remote.url().map(String::from))
    }

    /// Check if a branch exists locally
    pub fn branch_exists(&self, name: &str) -> bool {
        self.repo.find_branch(name, BranchType::Local).is_ok()
    }

    /// Get access to the underlying repository
    pub fn repository(&self) -> &Repository {
        &self.repo
    }

    /// Fetch from all remotes using the git CLI
    ///
    /// This is a potentially slow operation that should be run in a background task.
    /// Returns Ok(()) on success, or an error if fetch fails.
    ///
    /// Uses the system git command which handles SSH authentication natively
    /// via the user's SSH agent and configuration.
    pub fn fetch_all_remotes(&self) -> Result<()> {
        let never = AtomicBool::new(false);
        self.fetch_all_remotes_cancellable(&never).map(|_| ())
    }

    /// Fetch from all remotes, aborting as soon as `cancel` is set
    ///
    /// Same as [`fetch_all_remotes`](Self::fetch_all_remotes), but polls
    /// `cancel` while `git fetch` runs and kills the child process when it
    /// flips. Cancelling is not an error: the caller falls back to the refs
    /// already on disk, exactly as it does when the fetch fails.
    pub fn fetch_all_remotes_cancellable(&self, cancel: &AtomicBool) -> Result<FetchOutcome> {
        let workdir = self
            .workdir()
            .context("Repository has no working directory")?;

        tracing::debug!("Fetching all remotes via git CLI in {:?}", workdir);

        let mut child = Command::new("git")
            .args(["fetch", "--all"])
            .current_dir(workdir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to execute git fetch. Is git installed and in PATH?")?;

        // Drain stderr on its own thread: git writes progress there, and a
        // full pipe buffer would deadlock the child while we poll for exit.
        let stderr_pipe = child.stderr.take();
        let drain = std::thread::spawn(move || {
            let mut buf = String::new();
            if let Some(mut pipe) = stderr_pipe {
                let _ = pipe.read_to_string(&mut buf);
            }
            buf
        });

        let status = loop {
            if let Some(status) = child.try_wait().context("Failed to wait for git fetch")? {
                break status;
            }
            if cancel.load(Ordering::Relaxed) {
                tracing::info!("Cancelling git fetch in {:?}", workdir);
                let _ = child.kill();
                let _ = child.wait();
                // Deliberately not joining the drain thread: a lingering
                // credential helper or ssh child can hold the pipe open past
                // git's own exit, and nothing needs the output any more.
                return Ok(FetchOutcome::Cancelled);
            }
            std::thread::sleep(FETCH_POLL_INTERVAL);
        };

        let stderr = drain.join().unwrap_or_default();

        if !status.success() {
            return Err(classify_fetch_error(stderr.trim()));
        }

        Ok(FetchOutcome::Completed)
    }

    /// List all branch refs (local and remote) sorted for UI display
    ///
    /// Returns branches sorted with default base first, then local, then remote.
    /// Within each group, branches are sorted alphabetically.
    pub fn list_all_branch_refs(&self, default_base: Option<&str>) -> Result<Vec<BranchRefInfo>> {
        let mut branch_refs = Vec::new();

        // Collect local branches
        let local_branches = self
            .repo
            .branches(Some(BranchType::Local))
            .context("Failed to list local branches")?;

        for branch_result in local_branches {
            let (branch, _) = branch_result.context("Failed to get local branch")?;
            if let Some(name) = branch.name().context("Failed to get branch name")? {
                let is_default = default_base.is_some_and(|d| d == name);
                branch_refs.push(BranchRefInfo {
                    ref_type: BranchRefInfoType::Local,
                    name: name.to_string(),
                    is_default_base: is_default,
                });
            }
        }

        // Collect remote branches (excluding HEAD refs)
        let remote_branches = self
            .repo
            .branches(Some(BranchType::Remote))
            .context("Failed to list remote branches")?;

        for branch_result in remote_branches {
            let (branch, _) = branch_result.context("Failed to get remote branch")?;
            if let Some(name) = branch.name().context("Failed to get branch name")? {
                // Skip HEAD references like "origin/HEAD"
                if name.ends_with("/HEAD") {
                    continue;
                }
                let is_default = default_base.is_some_and(|d| d == name);
                branch_refs.push(BranchRefInfo {
                    ref_type: BranchRefInfoType::Remote,
                    name: name.to_string(),
                    is_default_base: is_default,
                });
            }
        }

        // Sort: default base first, then local, then remote (alphabetically within groups)
        branch_refs.sort_by(|a, b| {
            // Default base always comes first
            match (a.is_default_base, b.is_default_base) {
                (true, false) => return std::cmp::Ordering::Less,
                (false, true) => return std::cmp::Ordering::Greater,
                _ => {}
            }

            // Then sort by type (local before remote)
            match (&a.ref_type, &b.ref_type) {
                (BranchRefInfoType::Local, BranchRefInfoType::Remote) => std::cmp::Ordering::Less,
                (BranchRefInfoType::Remote, BranchRefInfoType::Local) => {
                    std::cmp::Ordering::Greater
                }
                _ => a.name.cmp(&b.name),
            }
        });

        Ok(branch_refs)
    }
}

/// Classify a failed `git fetch`'s stderr into a user-facing error
///
/// Provides more helpful messages for common failures (network problems,
/// authentication) and falls back to the raw stderr otherwise.
fn classify_fetch_error(stderr: &str) -> anyhow::Error {
    if stderr.contains("Could not resolve hostname") || stderr.contains("unable to access") {
        anyhow::anyhow!(
            "Network error during git fetch: {}. Check your internet connection.",
            stderr
        )
    } else if stderr.contains("Permission denied") || stderr.contains("authentication failed") {
        anyhow::anyhow!(
            "Authentication failed during git fetch: {}. Check your SSH keys or credentials.",
            stderr
        )
    } else {
        anyhow::anyhow!("git fetch failed: {}", stderr)
    }
}

/// Type of branch reference
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchRefInfoType {
    /// Local branch
    Local,
    /// Remote tracking branch
    Remote,
}

/// Information about a branch reference
#[derive(Debug, Clone)]
pub struct BranchRefInfo {
    /// Type of branch (local or remote)
    pub ref_type: BranchRefInfoType,
    /// Full branch name (e.g., "main" or "origin/main")
    pub name: String,
    /// Whether this is the default base branch
    pub is_default_base: bool,
}

/// Check if a path is inside a git repository
pub fn is_git_repository(path: &Path) -> bool {
    Repository::discover(path).is_ok()
}

/// Validates a branch name according to git rules (git check-ref-format).
///
/// Returns `Ok(())` if the name is valid, or `Err(message)` with a specific error.
///
/// Git branch names cannot:
/// - Be empty
/// - Contain spaces, `~`, `^`, `:`, `?`, `*`, `[`, `\`, or control characters
/// - Begin or end with `/` or `.`
/// - Contain `..`, `@{`, or consecutive slashes `//`
/// - End with `.lock`
pub fn validate_branch_name(name: &str) -> Result<(), String> {
    // Check for empty name
    if name.is_empty() {
        return Err("Branch name cannot be empty".to_string());
    }

    // Check for invalid characters
    for c in name.chars() {
        if INVALID_BRANCH_CHARS.contains(&c) {
            return Err(format!("Branch name cannot contain '{}'", c));
        }
        // Check for control characters (ASCII 0-31 and 127)
        if c.is_ascii_control() {
            return Err("Branch name cannot contain control characters".to_string());
        }
    }

    // Check for starting/ending with / or .
    if name.starts_with('/') || name.ends_with('/') {
        return Err("Branch name cannot start or end with '/'".to_string());
    }
    if name.starts_with('.') || name.ends_with('.') {
        return Err("Branch name cannot start or end with '.'".to_string());
    }

    // Check for consecutive slashes
    if name.contains("//") {
        return Err("Branch name cannot contain consecutive slashes '//'".to_string());
    }

    // Check for ..
    if name.contains("..") {
        return Err("Branch name cannot contain '..'".to_string());
    }

    // Check for @{
    if name.contains("@{") {
        return Err("Branch name cannot contain '@{'".to_string());
    }

    // Check for .lock suffix
    if name.ends_with(".lock") {
        return Err("Branch name cannot end with '.lock'".to_string());
    }

    Ok(())
}

/// Characters that are invalid in git branch names and should be filtered during input
pub const INVALID_BRANCH_CHARS: &[char] = &[' ', '~', '^', ':', '?', '*', '[', '\\'];

/// Get the repository root from any path within it
pub fn find_repo_root(path: &Path) -> Result<PathBuf> {
    let repo = Repository::discover(path)
        .with_context(|| format!("Failed to discover git repository from {:?}", path))?;
    repo.workdir()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("Repository has no working directory (bare repo)"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_repo() -> (TempDir, Repository) {
        let temp_dir = TempDir::new().unwrap();
        let repo = Repository::init(temp_dir.path()).unwrap();

        // Configure user for commits
        {
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "Test User").unwrap();
            config.set_str("user.email", "test@example.com").unwrap();
        }

        // Create an initial commit so HEAD is valid
        {
            let sig = repo.signature().unwrap();
            let tree_id = {
                let mut index = repo.index().unwrap();
                index.write_tree().unwrap()
            };
            let tree = repo.find_tree(tree_id).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
                .unwrap();
        }

        (temp_dir, repo)
    }

    #[test]
    fn test_fetch_completes_on_a_repo_without_remotes() {
        let (temp_dir, _repo) = create_test_repo();
        let git_ops = GitOps::open(temp_dir.path()).unwrap();

        let never = AtomicBool::new(false);
        assert_eq!(
            git_ops.fetch_all_remotes_cancellable(&never).unwrap(),
            FetchOutcome::Completed
        );
    }

    #[test]
    fn test_cancelled_fetch_reports_cancellation_rather_than_failing() {
        let (temp_dir, repo) = create_test_repo();
        // A non-routable address, so the fetch hangs long enough to cancel
        repo.remote("origin", "https://192.0.2.1/panoptes-test.git")
            .unwrap();
        let git_ops = GitOps::open(temp_dir.path()).unwrap();

        let cancel = AtomicBool::new(true);
        let started = std::time::Instant::now();
        let outcome = git_ops.fetch_all_remotes_cancellable(&cancel).unwrap();

        assert_eq!(outcome, FetchOutcome::Cancelled);
        // Cancelling has to be prompt: the point is not making the user wait
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn test_git_ops_open() {
        let (temp_dir, _repo) = create_test_repo();
        let git_ops = GitOps::open(temp_dir.path()).unwrap();
        assert!(!git_ops.is_bare());
    }

    #[test]
    fn test_git_ops_discover() {
        let (temp_dir, _repo) = create_test_repo();
        // Create a subdirectory and discover from there
        let subdir = temp_dir.path().join("subdir");
        std::fs::create_dir(&subdir).unwrap();

        let git_ops = GitOps::discover(&subdir).unwrap();
        // Compare canonicalized paths to handle macOS symlinks
        let expected = temp_dir.path().canonicalize().unwrap();
        let actual = git_ops.workdir().unwrap().canonicalize().unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_git_ops_default_branch() {
        let (temp_dir, _repo) = create_test_repo();
        let git_ops = GitOps::open(temp_dir.path()).unwrap();

        // Default branch should be "master" (git init default) or our HEAD
        let default = git_ops.default_branch_name().unwrap();
        assert!(!default.is_empty());
    }

    #[test]
    fn test_git_ops_branch_exists() {
        let (temp_dir, repo) = create_test_repo();

        // Create a branch
        let head = repo.head().unwrap();
        let commit = head.peel_to_commit().unwrap();
        repo.branch("test-branch", &commit, false).unwrap();

        let git_ops = GitOps::open(temp_dir.path()).unwrap();
        assert!(git_ops.branch_exists("test-branch"));
        assert!(!git_ops.branch_exists("nonexistent"));
    }

    #[test]
    fn test_is_git_repository() {
        let (temp_dir, _repo) = create_test_repo();
        assert!(is_git_repository(temp_dir.path()));

        let non_git = TempDir::new().unwrap();
        assert!(!is_git_repository(non_git.path()));
    }

    #[test]
    fn test_find_repo_root() {
        let (temp_dir, _repo) = create_test_repo();
        let subdir = temp_dir.path().join("deep").join("nested");
        std::fs::create_dir_all(&subdir).unwrap();

        let root = find_repo_root(&subdir).unwrap();
        // Compare canonicalized paths to handle macOS symlinks
        let expected = temp_dir.path().canonicalize().unwrap();
        let actual = root.canonicalize().unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_git_ops_repo_path() {
        let (temp_dir, _repo) = create_test_repo();
        let git_ops = GitOps::open(temp_dir.path()).unwrap();

        let repo_path = git_ops.repo_path();
        // Compare canonicalized paths to handle macOS symlinks
        let expected = temp_dir.path().canonicalize().unwrap();
        let actual = repo_path.canonicalize().unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_git_ops_current_branch() {
        let (temp_dir, _repo) = create_test_repo();
        let git_ops = GitOps::open(temp_dir.path()).unwrap();

        let current = git_ops.current_branch().unwrap();
        assert!(current.is_some());
    }

    #[test]
    fn test_classify_fetch_error_host_resolution() {
        for stderr in [
            "ssh: Could not resolve hostname github.com",
            "fatal: unable to access 'https://example.com/repo.git/'",
        ] {
            let msg = classify_fetch_error(stderr).to_string();
            assert!(
                msg.starts_with("Network error during git fetch:"),
                "unexpected classification for {stderr:?}: {msg}"
            );
            assert!(msg.contains(stderr), "original stderr lost: {msg}");
        }
    }

    #[test]
    fn test_classify_fetch_error_authentication() {
        for stderr in [
            "git@github.com: Permission denied (publickey).",
            "fatal: authentication failed for 'https://example.com/repo.git/'",
        ] {
            let msg = classify_fetch_error(stderr).to_string();
            assert!(
                msg.starts_with("Authentication failed during git fetch:"),
                "unexpected classification for {stderr:?}: {msg}"
            );
            assert!(msg.contains(stderr), "original stderr lost: {msg}");
        }
    }

    #[test]
    fn test_classify_fetch_error_generic() {
        let msg = classify_fetch_error("fatal: something else went wrong").to_string();
        assert_eq!(msg, "git fetch failed: fatal: something else went wrong");
    }

    /// Set up local branches and simulated remote-tracking refs for sort tests
    ///
    /// Remote-tracking branches are plain refs under `refs/remotes/`, so they
    /// can be created locally without any network remote.
    fn setup_branch_refs(repo: &Repository) {
        let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("zeta", &head_commit, false).unwrap();
        repo.branch("alpha", &head_commit, false).unwrap();

        let oid = head_commit.id();
        repo.reference("refs/remotes/origin/beta", oid, false, "test")
            .unwrap();
        repo.reference("refs/remotes/origin/main", oid, false, "test")
            .unwrap();
        repo.reference_symbolic(
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
            false,
            "test",
        )
        .unwrap();
    }

    #[test]
    fn test_list_all_branch_refs_sorts_default_then_local_then_remote() {
        let (temp_dir, repo) = create_test_repo();
        setup_branch_refs(&repo);
        let head_branch = repo.head().unwrap().shorthand().unwrap().to_string();

        let git_ops = GitOps::open(temp_dir.path()).unwrap();
        let refs = git_ops.list_all_branch_refs(Some("zeta")).unwrap();

        let names: Vec<&str> = refs.iter().map(|r| r.name.as_str()).collect();
        // Default base first, then locals alphabetically, then remotes
        // alphabetically ("alpha" < head branch "main"/"master" < "zeta").
        assert_eq!(
            names,
            vec![
                "zeta",
                "alpha",
                head_branch.as_str(),
                "origin/beta",
                "origin/main"
            ]
        );

        assert!(refs[0].is_default_base);
        assert!(refs[1..].iter().all(|r| !r.is_default_base));
        assert_eq!(refs[0].ref_type, BranchRefInfoType::Local);
        assert_eq!(refs[3].ref_type, BranchRefInfoType::Remote);
        assert_eq!(refs[4].ref_type, BranchRefInfoType::Remote);
    }

    #[test]
    fn test_list_all_branch_refs_remote_default_floats_first_and_head_skipped() {
        let (temp_dir, repo) = create_test_repo();
        setup_branch_refs(&repo);

        let git_ops = GitOps::open(temp_dir.path()).unwrap();
        let refs = git_ops.list_all_branch_refs(Some("origin/main")).unwrap();

        // A remote default base still sorts first, ahead of all locals.
        assert_eq!(refs[0].name, "origin/main");
        assert!(refs[0].is_default_base);
        assert_eq!(refs[0].ref_type, BranchRefInfoType::Remote);

        // "origin/HEAD" is never listed.
        assert!(refs.iter().all(|r| r.name != "origin/HEAD"));

        // No default at all: plain local-then-remote ordering.
        let refs = git_ops.list_all_branch_refs(None).unwrap();
        assert!(refs.iter().all(|r| !r.is_default_base));
        let first_remote = refs
            .iter()
            .position(|r| r.ref_type == BranchRefInfoType::Remote)
            .unwrap();
        assert!(refs[..first_remote]
            .iter()
            .all(|r| r.ref_type == BranchRefInfoType::Local));
    }

    #[test]
    fn test_validate_branch_name_valid() {
        // Valid branch names
        assert!(validate_branch_name("feature/new-thing").is_ok());
        assert!(validate_branch_name("fix-bug-123").is_ok());
        assert!(validate_branch_name("my-branch").is_ok());
        assert!(validate_branch_name("feature/deeply/nested/branch").is_ok());
        assert!(validate_branch_name("UPPERCASE").is_ok());
        assert!(validate_branch_name("123-numeric").is_ok());
    }

    #[test]
    fn test_validate_branch_name_empty() {
        assert!(validate_branch_name("").is_err());
    }

    #[test]
    fn test_validate_branch_name_invalid_chars() {
        assert!(validate_branch_name("feature branch").is_err()); // space
        assert!(validate_branch_name("branch~name").is_err()); // tilde
        assert!(validate_branch_name("branch^name").is_err()); // caret
        assert!(validate_branch_name("branch:name").is_err()); // colon
        assert!(validate_branch_name("branch?name").is_err()); // question mark
        assert!(validate_branch_name("branch*name").is_err()); // asterisk
        assert!(validate_branch_name("branch[name").is_err()); // bracket
        assert!(validate_branch_name("branch\\name").is_err()); // backslash
    }

    #[test]
    fn test_validate_branch_name_slash_dot_edges() {
        assert!(validate_branch_name("/branch").is_err()); // starts with /
        assert!(validate_branch_name("branch/").is_err()); // ends with /
        assert!(validate_branch_name(".branch").is_err()); // starts with .
        assert!(validate_branch_name("branch.").is_err()); // ends with .
    }

    #[test]
    fn test_validate_branch_name_consecutive_slashes() {
        assert!(validate_branch_name("feature//branch").is_err());
    }

    #[test]
    fn test_validate_branch_name_double_dot() {
        assert!(validate_branch_name("branch..name").is_err());
        assert!(validate_branch_name("feature/..").is_err());
    }

    #[test]
    fn test_validate_branch_name_at_brace() {
        assert!(validate_branch_name("branch@{name").is_err());
    }

    #[test]
    fn test_validate_branch_name_lock_suffix() {
        assert!(validate_branch_name("branch.lock").is_err());
        assert!(validate_branch_name("feature/branch.lock").is_err());
        // But this should be fine:
        assert!(validate_branch_name("branch.locked").is_ok());
    }
}
