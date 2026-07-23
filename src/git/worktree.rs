//! Git worktree operations
//!
//! Provides functionality for creating, removing, and listing git worktrees.

use anyhow::{Context, Result};
use git2::{BranchType, Repository, WorktreeAddOptions, WorktreePruneOptions};
use std::path::{Path, PathBuf};

/// Information about a git worktree
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    /// Name of the worktree
    pub name: String,
    /// Path to the worktree directory
    pub path: PathBuf,
    /// Branch checked out in the worktree (if any)
    pub branch: Option<String>,
    /// Whether this is the main worktree
    pub is_main: bool,
}

/// Create a new worktree for a branch
///
/// # Arguments
/// * `repo` - The git repository
/// * `branch_name` - Name of the branch to check out (or create)
/// * `worktree_path` - Path where the worktree will be created
/// * `create_branch` - If true, create the branch if it doesn't exist
/// * `base_ref` - Optional base reference to branch from (e.g., "origin/develop")
///
/// # Returns
/// Path to the created worktree
pub fn create_worktree(
    repo: &Repository,
    branch_name: &str,
    worktree_path: &Path,
    create_branch: bool,
    base_ref: Option<&str>,
) -> Result<PathBuf> {
    // Check if worktree path already exists
    if worktree_path.exists() {
        anyhow::bail!(
            "Worktree path already exists: {}. Choose a different branch name or remove the existing directory.",
            worktree_path.display()
        );
    }

    // Ensure parent directory exists
    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent directory: {:?}", parent))?;
    }

    // Get the worktree name from the path
    let worktree_name = worktree_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid worktree path"))?;

    // Check if branch exists, create if needed
    let branch_exists = repo.find_branch(branch_name, BranchType::Local).is_ok();

    if !branch_exists {
        if create_branch {
            // Determine which commit to branch from
            let commit = if let Some(base) = base_ref {
                // Try to resolve the base reference
                resolve_ref_to_commit(repo, base)?
            } else {
                // Default to HEAD
                let head = repo.head().context("Failed to get HEAD")?;
                head.peel_to_commit().context("Failed to get HEAD commit")?
            };

            repo.branch(branch_name, &commit, false)
                .with_context(|| format!("Failed to create branch '{}'", branch_name))?;
        } else {
            anyhow::bail!("Branch '{}' does not exist", branch_name);
        }
    }

    // Find the branch reference
    let branch = repo
        .find_branch(branch_name, BranchType::Local)
        .with_context(|| format!("Failed to find branch '{}'", branch_name))?;

    let reference = branch.into_reference();

    // Create the worktree with options
    let mut opts = WorktreeAddOptions::new();
    opts.reference(Some(&reference));

    repo.worktree(worktree_name, worktree_path, Some(&opts))
        .with_context(|| format!("Failed to create worktree at {:?}", worktree_path))?;

    Ok(worktree_path.to_path_buf())
}

/// Resolve a reference (branch name, remote branch, or commit) to a commit
///
/// Tries multiple resolution strategies and provides detailed error messages
/// if none succeed.
fn resolve_ref_to_commit<'repo>(
    repo: &'repo Repository,
    ref_name: &str,
) -> Result<git2::Commit<'repo>> {
    let mut attempts = Vec::new();

    // Try as a local branch
    match repo.find_branch(ref_name, BranchType::Local) {
        Ok(branch) => {
            let reference = branch.into_reference();
            return reference
                .peel_to_commit()
                .context("Failed to peel local branch to commit");
        }
        Err(e) => attempts.push(format!("local branch: {}", e)),
    }

    // Try as a remote branch
    match repo.find_branch(ref_name, BranchType::Remote) {
        Ok(branch) => {
            let reference = branch.into_reference();
            return reference
                .peel_to_commit()
                .context("Failed to peel remote branch to commit");
        }
        Err(e) => attempts.push(format!("remote branch: {}", e)),
    }

    // Try as a direct reference
    match repo.find_reference(ref_name) {
        Ok(reference) => {
            return reference
                .peel_to_commit()
                .context("Failed to peel reference to commit");
        }
        Err(e) => attempts.push(format!("direct reference: {}", e)),
    }

    // Try as a revspec (e.g., "refs/remotes/origin/main")
    match repo.revparse_single(ref_name) {
        Ok(obj) => {
            return obj
                .peel_to_commit()
                .with_context(|| format!("Failed to peel revspec '{}' to commit", ref_name));
        }
        Err(e) => attempts.push(format!("revspec: {}", e)),
    }

    anyhow::bail!(
        "Could not resolve '{}' to a commit. Tried:\n  - {}",
        ref_name,
        attempts.join("\n  - ")
    )
}

/// Remove a worktree
///
/// # Arguments
/// * `repo` - The git repository
/// * `worktree_name` - Name of the worktree to remove
/// * `force` - If true, also delete the working tree on disk, discarding any
///   local modifications. If false, the worktree is only pruned from git's
///   bookkeeping and the directory is left untouched.
pub fn remove_worktree(repo: &Repository, worktree_name: &str, force: bool) -> Result<()> {
    // Find the worktree
    let worktree = repo
        .find_worktree(worktree_name)
        .with_context(|| format!("Worktree '{}' not found", worktree_name))?;

    // Get the worktree path before pruning
    let worktree_path = worktree.path().to_path_buf();

    // Prune (remove) the worktree
    let mut opts = WorktreePruneOptions::new();
    // valid(true) allows pruning of valid (still existing) worktrees
    opts.valid(true);
    if force {
        // working_tree(true) removes the actual working tree on disk
        opts.working_tree(true);
    }

    worktree
        .prune(Some(&mut opts))
        .with_context(|| format!("Failed to prune worktree '{}'", worktree_name))?;

    // Only delete the directory when the caller asked for it. Without force,
    // deleting here would silently destroy uncommitted local modifications.
    if force && worktree_path.exists() {
        std::fs::remove_dir_all(&worktree_path)
            .with_context(|| format!("Failed to remove worktree directory: {:?}", worktree_path))?;
    }

    Ok(())
}

/// Helper function to get branch name from a worktree path
fn get_branch_from_worktree(path: &Path) -> Option<String> {
    let repo = Repository::open(path).ok()?;
    let head = repo.head().ok()?;
    if head.is_branch() {
        head.shorthand().map(String::from)
    } else {
        None
    }
}

/// List all worktrees in a repository
pub fn list_worktrees(repo: &Repository) -> Result<Vec<WorktreeInfo>> {
    let mut worktrees = Vec::new();

    // Add the main worktree
    if let Some(workdir) = repo.workdir() {
        let branch = repo
            .head()
            .ok()
            .filter(|h| h.is_branch())
            .and_then(|h| h.shorthand().map(String::from));

        worktrees.push(WorktreeInfo {
            name: "main".to_string(),
            path: workdir.to_path_buf(),
            branch,
            is_main: true,
        });
    }

    // List additional worktrees
    let wt_names = repo.worktrees().context("Failed to list worktrees")?;

    for wt_name in wt_names.iter().flatten() {
        if let Ok(wt) = repo.find_worktree(wt_name) {
            let wt_path = wt.path().to_path_buf();

            // Try to open the worktree repo to get branch info
            let branch = get_branch_from_worktree(&wt_path);

            worktrees.push(WorktreeInfo {
                name: wt_name.to_string(),
                path: wt_path,
                branch,
                is_main: false,
            });
        }
    }

    Ok(worktrees)
}

/// Generate a worktree path for a branch within a project directory
///
/// Creates a sanitized directory structure: `{worktrees_dir}/{project_name}/{branch_name}`.
/// This allows worktrees from different projects to coexist without name collisions.
///
/// # Arguments
/// * `worktrees_dir` - Base directory for all worktrees (e.g., `~/.panoptes/worktrees`)
/// * `project_name` - Human-readable project name
/// * `branch_name` - Git branch name
///
/// # Example
/// ```ignore
/// worktree_path_for_branch(Path::new("~/.panoptes/worktrees"), "my-app", "feature/auth")
/// // Returns: ~/.panoptes/worktrees/my-app/feature-auth
/// ```
pub fn worktree_path_for_branch(
    worktrees_dir: &Path,
    project_name: &str,
    branch_name: &str,
) -> PathBuf {
    // Sanitize project name for use as directory name
    let safe_project =
        project_name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|', ' '], "-");
    // Sanitize branch name for use as directory name
    let safe_branch = branch_name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "-");

    worktrees_dir.join(safe_project).join(safe_branch)
}

/// Check if a worktree exists for a given branch
pub fn worktree_exists_for_branch(repo: &Repository, branch_name: &str) -> bool {
    if let Ok(worktrees) = list_worktrees(repo) {
        worktrees
            .iter()
            .any(|wt| wt.branch.as_deref() == Some(branch_name))
    } else {
        false
    }
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

    /// Add an empty commit on HEAD, returning its OID
    fn add_commit(repo: &Repository, message: &str) -> git2::Oid {
        let sig = repo.signature().unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
            .unwrap()
    }

    /// Create a "base" branch, then advance HEAD past it so tests can tell
    /// whether a new branch was cut from the base ref or from HEAD.
    ///
    /// Returns the OID the base branch points at.
    fn setup_diverged_base(repo: &Repository) -> git2::Oid {
        let base_oid = repo.head().unwrap().peel_to_commit().unwrap().id();
        let base_commit = repo.find_commit(base_oid).unwrap();
        repo.branch("base", &base_commit, false).unwrap();

        let head_oid = add_commit(repo, "Second commit");
        assert_ne!(base_oid, head_oid, "HEAD must diverge from base");
        base_oid
    }

    /// OID a local branch currently points at
    fn branch_tip(repo: &Repository, name: &str) -> git2::Oid {
        repo.find_branch(name, BranchType::Local)
            .unwrap()
            .into_reference()
            .peel_to_commit()
            .unwrap()
            .id()
    }

    #[test]
    fn test_list_worktrees_main_only() {
        let (temp_dir, repo) = create_test_repo();
        let worktrees = list_worktrees(&repo).unwrap();

        assert_eq!(worktrees.len(), 1);
        assert!(worktrees[0].is_main);
        // Compare canonicalized paths to handle macOS symlinks
        let expected = temp_dir.path().canonicalize().unwrap();
        let actual = worktrees[0].path.canonicalize().unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_create_worktree() {
        let (temp_dir, repo) = create_test_repo();
        let worktree_dir = temp_dir.path().join("worktrees").join("feature-test");

        // Create a worktree with a new branch (no base_ref)
        let path = create_worktree(&repo, "feature-test", &worktree_dir, true, None).unwrap();

        assert_eq!(path, worktree_dir);
        assert!(worktree_dir.exists());

        // Verify it appears in the list
        let worktrees = list_worktrees(&repo).unwrap();
        assert_eq!(worktrees.len(), 2);
    }

    #[test]
    fn test_create_worktree_existing_branch() {
        let (temp_dir, repo) = create_test_repo();

        // Create a branch first
        let head = repo.head().unwrap();
        let commit = head.peel_to_commit().unwrap();
        repo.branch("existing-branch", &commit, false).unwrap();

        let worktree_dir = temp_dir.path().join("worktrees").join("existing-branch");

        // Create worktree for existing branch
        let path = create_worktree(&repo, "existing-branch", &worktree_dir, false, None).unwrap();

        assert_eq!(path, worktree_dir);
        assert!(worktree_dir.exists());
    }

    #[test]
    fn test_create_worktree_base_ref_local_branch_name() {
        let (temp_dir, repo) = create_test_repo();
        let base_oid = setup_diverged_base(&repo);
        let worktree_dir = temp_dir.path().join("worktrees").join("from-local");

        // Resolved as a local branch (first strategy)
        create_worktree(&repo, "from-local", &worktree_dir, true, Some("base")).unwrap();

        assert_eq!(branch_tip(&repo, "from-local"), base_oid);
    }

    #[test]
    fn test_create_worktree_base_ref_remote_branch_name() {
        let (temp_dir, repo) = create_test_repo();
        let base_oid = setup_diverged_base(&repo);
        // Simulate a remote-tracking branch: a plain ref under refs/remotes/
        repo.reference("refs/remotes/origin/develop", base_oid, false, "test")
            .unwrap();
        let worktree_dir = temp_dir.path().join("worktrees").join("from-remote");

        // Resolved as a remote branch (second strategy)
        create_worktree(
            &repo,
            "from-remote",
            &worktree_dir,
            true,
            Some("origin/develop"),
        )
        .unwrap();

        assert_eq!(branch_tip(&repo, "from-remote"), base_oid);
    }

    #[test]
    fn test_create_worktree_base_ref_fully_qualified() {
        let (temp_dir, repo) = create_test_repo();
        let base_oid = setup_diverged_base(&repo);
        let worktree_dir = temp_dir.path().join("worktrees").join("from-ref");

        // "refs/heads/base" is not a branch shorthand, so it falls through to
        // the direct-reference strategy
        create_worktree(
            &repo,
            "from-ref",
            &worktree_dir,
            true,
            Some("refs/heads/base"),
        )
        .unwrap();

        assert_eq!(branch_tip(&repo, "from-ref"), base_oid);
    }

    #[test]
    fn test_create_worktree_base_ref_commit_sha() {
        let (temp_dir, repo) = create_test_repo();
        let base_oid = setup_diverged_base(&repo);
        let worktree_dir = temp_dir.path().join("worktrees").join("from-sha");

        // A raw SHA is not a branch or a reference; only the revspec strategy
        // can resolve it
        create_worktree(
            &repo,
            "from-sha",
            &worktree_dir,
            true,
            Some(&base_oid.to_string()),
        )
        .unwrap();

        assert_eq!(branch_tip(&repo, "from-sha"), base_oid);
    }

    #[test]
    fn test_create_worktree_base_ref_unresolvable() {
        let (temp_dir, repo) = create_test_repo();
        setup_diverged_base(&repo);
        let worktree_dir = temp_dir.path().join("worktrees").join("bad-base");

        let err = create_worktree(&repo, "bad-base", &worktree_dir, true, Some("no-such-ref"))
            .unwrap_err();

        let msg = format!("{err:#}");
        assert!(
            msg.contains("Could not resolve 'no-such-ref'"),
            "unexpected error: {msg}"
        );
        // Error lists every attempted strategy
        for strategy in [
            "local branch",
            "remote branch",
            "direct reference",
            "revspec",
        ] {
            assert!(msg.contains(strategy), "missing '{strategy}' in: {msg}");
        }
        // No branch or worktree should have been created
        assert!(repo.find_branch("bad-base", BranchType::Local).is_err());
        assert!(!worktree_dir.exists());
    }

    #[test]
    fn test_create_worktree_nonexistent_branch_no_create() {
        let (temp_dir, repo) = create_test_repo();
        let worktree_dir = temp_dir.path().join("worktrees").join("nonexistent");

        // Should fail when branch doesn't exist and create_branch is false
        let result = create_worktree(&repo, "nonexistent", &worktree_dir, false, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_worktree() {
        let (temp_dir, repo) = create_test_repo();
        let worktree_dir = temp_dir.path().join("worktrees").join("to-remove");

        // Create a worktree
        create_worktree(&repo, "to-remove", &worktree_dir, true, None).unwrap();
        assert!(worktree_dir.exists());

        // Remove it (force=true because the worktree is valid/active)
        remove_worktree(&repo, "to-remove", true).unwrap();

        // Verify it's gone
        let worktrees = list_worktrees(&repo).unwrap();
        assert_eq!(worktrees.len(), 1); // Only main remains
    }

    #[test]
    fn test_remove_worktree_without_force_preserves_dirty_tree() {
        let (temp_dir, repo) = create_test_repo();
        let worktree_dir = temp_dir.path().join("worktrees").join("keep-my-work");

        // Create a worktree and dirty it with an uncommitted local change
        create_worktree(&repo, "keep-my-work", &worktree_dir, true, None).unwrap();
        let precious = worktree_dir.join("uncommitted.txt");
        std::fs::write(&precious, "unsaved work").unwrap();

        // Without force, the tree on disk must survive
        remove_worktree(&repo, "keep-my-work", false).unwrap();

        assert!(worktree_dir.exists());
        assert_eq!(std::fs::read_to_string(&precious).unwrap(), "unsaved work");
    }

    #[test]
    fn test_worktree_path_for_branch() {
        let base = PathBuf::from("/home/user/worktrees");

        assert_eq!(
            worktree_path_for_branch(&base, "my-project", "main"),
            PathBuf::from("/home/user/worktrees/my-project/main")
        );

        assert_eq!(
            worktree_path_for_branch(&base, "my-project", "feature/add-auth"),
            PathBuf::from("/home/user/worktrees/my-project/feature-add-auth")
        );

        assert_eq!(
            worktree_path_for_branch(&base, "My Project", "fix/bug:123"),
            PathBuf::from("/home/user/worktrees/My-Project/fix-bug-123")
        );
    }

    #[test]
    fn test_worktree_exists_for_branch() {
        let (temp_dir, repo) = create_test_repo();

        // Main branch should be found
        let main_branch = repo
            .head()
            .unwrap()
            .shorthand()
            .map(String::from)
            .unwrap_or_default();

        assert!(worktree_exists_for_branch(&repo, &main_branch));

        // Create a worktree for a new branch
        let worktree_dir = temp_dir.path().join("worktrees").join("test-branch");
        create_worktree(&repo, "test-branch", &worktree_dir, true, None).unwrap();

        assert!(worktree_exists_for_branch(&repo, "test-branch"));
        assert!(!worktree_exists_for_branch(&repo, "nonexistent"));
    }
}
