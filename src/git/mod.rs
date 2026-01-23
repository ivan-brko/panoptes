//! Git operations wrapper module
//!
//! Provides a safe wrapper around git2 for common repository operations.

pub mod worktree;

use anyhow::{Context, Result};
use git2::{BranchType, Repository};
use std::path::{Path, PathBuf};

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
        let repo = Repository::discover(path)
            .with_context(|| format!("Failed to discover git repository from {:?}", path))?;
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

    /// List all local branches
    pub fn list_local_branches(&self) -> Result<Vec<String>> {
        let branches = self
            .repo
            .branches(Some(BranchType::Local))
            .context("Failed to list branches")?;

        let mut result = Vec::new();
        for branch_result in branches {
            let (branch, _) = branch_result.context("Failed to get branch")?;
            if let Some(name) = branch.name().context("Failed to get branch name")? {
                result.push(name.to_string());
            }
        }

        result.sort();
        Ok(result)
    }

    /// List all remote branches
    pub fn list_remote_branches(&self) -> Result<Vec<String>> {
        let branches = self
            .repo
            .branches(Some(BranchType::Remote))
            .context("Failed to list remote branches")?;

        let mut result = Vec::new();
        for branch_result in branches {
            let (branch, _) = branch_result.context("Failed to get branch")?;
            if let Some(name) = branch.name().context("Failed to get branch name")? {
                result.push(name.to_string());
            }
        }

        result.sort();
        Ok(result)
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

    /// Get a mutable reference to the underlying repository
    pub fn repository_mut(&mut self) -> &mut Repository {
        &mut self.repo
    }

    /// Fetch from all remotes using the git CLI
    ///
    /// This is a potentially slow operation that should be run in a background task.
    /// Returns Ok(()) on success, or an error if fetch fails.
    ///
    /// Uses the system git command which handles SSH authentication natively
    /// via the user's SSH agent and configuration.
    pub fn fetch_all_remotes(&self) -> Result<()> {
        let workdir = self
            .workdir()
            .context("Repository has no working directory")?;

        tracing::debug!("Fetching all remotes via git CLI in {:?}", workdir);

        let output = std::process::Command::new("git")
            .args(["fetch", "--all"])
            .current_dir(workdir)
            .output()
            .context("Failed to execute git fetch")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git fetch failed: {}", stderr.trim());
        }

        Ok(())
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
    const INVALID_CHARS: &[char] = &[' ', '~', '^', ':', '?', '*', '[', '\\'];
    for c in name.chars() {
        if INVALID_CHARS.contains(&c) {
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
    fn test_git_ops_list_branches() {
        let (temp_dir, repo) = create_test_repo();

        // Create another branch
        let head = repo.head().unwrap();
        let commit = head.peel_to_commit().unwrap();
        repo.branch("feature-test", &commit, false).unwrap();

        let git_ops = GitOps::open(temp_dir.path()).unwrap();
        let branches = git_ops.list_local_branches().unwrap();

        assert!(branches.len() >= 1);
        assert!(branches.contains(&"feature-test".to_string()));
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
