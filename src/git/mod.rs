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
}

/// Check if a path is inside a git repository
pub fn is_git_repository(path: &Path) -> bool {
    Repository::discover(path).is_ok()
}

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
}
