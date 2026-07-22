//! Project and branch management module
//!
//! This module handles project and branch data structures for organizing
//! sessions by git repository and branch.

pub mod store;
pub mod tree;

pub use store::ProjectStore;
pub use tree::{
    all_folder_paths, parent_row_index, row_at, row_count, row_index_of_folder,
    row_index_of_project, visible_rows, FolderRow, ProjectRow, RowRef, TreeRow,
};

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::claude_config::ClaudeConfigId;
use crate::codex_config::CodexConfigId;

/// Unique identifier for a project
pub type ProjectId = Uuid;

/// Unique identifier for a branch
pub type BranchId = Uuid;

/// Maximum nesting depth for project folders
pub const MAX_FOLDER_DEPTH: usize = 3;

/// Maximum length of a single folder name segment
pub const MAX_FOLDER_SEGMENT_LEN: usize = 40;

/// Parse a user-entered folder path (e.g. "Acme/Platform") into segments
///
/// Empty or whitespace-only input yields an empty vec, meaning "root level".
/// Segments are trimmed and empty ones are skipped, so "Acme//Platform/" is
/// equivalent to "Acme/Platform".
pub fn parse_folder_path(input: &str) -> Result<Vec<String>> {
    let segments: Vec<String> = input
        .split('/')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    if segments.len() > MAX_FOLDER_DEPTH {
        bail!(
            "Folders can nest at most {} levels deep (got {})",
            MAX_FOLDER_DEPTH,
            segments.len()
        );
    }
    for segment in &segments {
        if segment.chars().count() > MAX_FOLDER_SEGMENT_LEN {
            bail!(
                "Folder name '{}' is too long (max {} characters)",
                segment,
                MAX_FOLDER_SEGMENT_LEN
            );
        }
        if segment.chars().any(|c| c.is_control()) {
            bail!("Folder names cannot contain control characters");
        }
    }

    Ok(segments)
}

/// Format a project count for display, e.g. "1 project" / "3 projects"
pub fn project_count_label(count: usize) -> String {
    if count == 1 {
        "1 project".to_string()
    } else {
        format!("{} projects", count)
    }
}

/// Render folder segments as a display/lookup path (e.g. "Acme/Platform")
///
/// Segments can never contain `/`, so this is a lossless key.
pub fn folder_path_key(segments: &[String]) -> String {
    segments.join("/")
}

/// A project representing a git repository
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Unique identifier
    pub id: ProjectId,
    /// Display name (usually the repository name)
    pub name: String,
    /// Path to the git repository root
    pub repo_path: PathBuf,
    /// Remote URL (if any)
    pub remote_url: Option<String>,
    /// Default branch name (e.g., "main" or "master")
    pub default_branch: String,
    /// Default base branch for creating new worktrees (e.g., "origin/develop")
    #[serde(default)]
    pub default_base_branch: Option<String>,
    /// Subfolder relative to repo_path for Claude Code sessions.
    /// If None, sessions start at repo root.
    #[serde(default)]
    pub session_subdir: Option<PathBuf>,
    /// Default Claude configuration for this project
    #[serde(default)]
    pub default_claude_config: Option<ClaudeConfigId>,
    /// Default Codex configuration for this project
    #[serde(default)]
    pub default_codex_config: Option<CodexConfigId>,
    /// Folder path segments this project is filed under, e.g. `["Acme", "Platform"]`.
    /// Empty means the project sits at the root of the project list.
    #[serde(default)]
    pub folder: Vec<String>,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last activity timestamp
    pub last_activity: DateTime<Utc>,
}

impl Project {
    /// Create a new project
    pub fn new(name: String, repo_path: PathBuf, default_branch: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            repo_path,
            remote_url: None,
            default_branch,
            default_base_branch: None,
            session_subdir: None,
            default_claude_config: None,
            default_codex_config: None,
            folder: Vec::new(),
            created_at: now,
            last_activity: now,
        }
    }

    /// Whether this project sits directly inside `folder`
    pub fn is_in_folder(&self, folder: &[String]) -> bool {
        self.folder == folder
    }

    /// Whether this project sits inside `folder` or any of its subfolders
    pub fn is_under_folder(&self, folder: &[String]) -> bool {
        self.folder.len() >= folder.len() && self.folder[..folder.len()] == *folder
    }

    /// Display path of the folder this project is filed under ("(root)" at top level)
    pub fn folder_display(&self) -> String {
        if self.folder.is_empty() {
            "(root)".to_string()
        } else {
            folder_path_key(&self.folder)
        }
    }

    /// Get effective working dir for a base path (repo or worktree)
    /// If session_subdir is set, returns base_path/session_subdir, otherwise returns base_path
    pub fn effective_working_dir(&self, base_path: &Path) -> PathBuf {
        match &self.session_subdir {
            Some(subdir) => base_path.join(subdir),
            None => base_path.to_path_buf(),
        }
    }

    /// Create a new project with remote URL
    pub fn with_remote(
        name: String,
        repo_path: PathBuf,
        default_branch: String,
        remote_url: String,
    ) -> Self {
        let mut project = Self::new(name, repo_path, default_branch);
        project.remote_url = Some(remote_url);
        project
    }

    /// Set the default base branch for creating worktrees
    pub fn set_default_base_branch(&mut self, base_branch: Option<String>) {
        self.default_base_branch = base_branch;
    }

    /// Update last activity timestamp
    pub fn touch(&mut self) {
        self.last_activity = Utc::now();
    }
}

/// A branch within a project
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    /// Unique identifier
    pub id: BranchId,
    /// Parent project identifier
    pub project_id: ProjectId,
    /// Git branch name
    pub name: String,
    /// Working directory (repo path for default, worktree path for others)
    pub working_dir: PathBuf,
    /// Whether this is the default branch
    pub is_default: bool,
    /// Whether this branch uses a git worktree
    pub is_worktree: bool,
    /// Whether this branch's working directory is missing (worktree deleted externally)
    /// This is a transient state that is updated by refresh_branches()
    #[serde(default)]
    pub stale: bool,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last activity timestamp
    pub last_activity: DateTime<Utc>,
}

impl Branch {
    /// Create a new branch entry
    pub fn new(
        project_id: ProjectId,
        name: String,
        working_dir: PathBuf,
        is_default: bool,
        is_worktree: bool,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            project_id,
            name,
            working_dir,
            is_default,
            is_worktree,
            stale: false,
            created_at: now,
            last_activity: now,
        }
    }

    /// Create the default branch for a project
    pub fn default_for_project(project_id: ProjectId, name: String, repo_path: PathBuf) -> Self {
        let mut branch = Self::new(project_id, name, repo_path, true, false);
        branch.stale = false;
        branch
    }

    /// Check if this branch's working directory exists
    pub fn working_dir_exists(&self) -> bool {
        self.working_dir.exists()
    }

    /// Mark this branch as stale (working directory missing)
    pub fn mark_stale(&mut self) {
        self.stale = true;
    }

    /// Clear stale status
    pub fn clear_stale(&mut self) {
        self.stale = false;
    }

    /// Update last activity timestamp
    pub fn touch(&mut self) {
        self.last_activity = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_creation() {
        let project = Project::new(
            "panoptes".to_string(),
            "/home/user/projects/panoptes".into(),
            "main".to_string(),
        );

        assert_eq!(project.name, "panoptes");
        assert_eq!(project.default_branch, "main");
        assert!(project.remote_url.is_none());
        assert!(project.created_at <= Utc::now());
        assert_eq!(project.created_at, project.last_activity);
    }

    #[test]
    fn test_project_with_remote() {
        let project = Project::with_remote(
            "panoptes".to_string(),
            "/home/user/projects/panoptes".into(),
            "main".to_string(),
            "https://github.com/user/panoptes.git".to_string(),
        );

        assert_eq!(
            project.remote_url,
            Some("https://github.com/user/panoptes.git".to_string())
        );
    }

    #[test]
    fn test_project_touch() {
        let mut project = Project::new("test".to_string(), "/tmp/test".into(), "main".to_string());
        let old_activity = project.last_activity;
        std::thread::sleep(std::time::Duration::from_millis(10));
        project.touch();
        assert!(project.last_activity > old_activity);
    }

    #[test]
    fn test_branch_creation() {
        let project_id = Uuid::new_v4();
        let branch = Branch::new(
            project_id,
            "feature/test".to_string(),
            "/home/user/worktrees/feature-test".into(),
            false,
            true,
        );

        assert_eq!(branch.project_id, project_id);
        assert_eq!(branch.name, "feature/test");
        assert!(!branch.is_default);
        assert!(branch.is_worktree);
    }

    #[test]
    fn test_branch_default_for_project() {
        let project_id = Uuid::new_v4();
        let branch = Branch::default_for_project(
            project_id,
            "main".to_string(),
            "/home/user/projects/repo".into(),
        );

        assert!(branch.is_default);
        assert!(!branch.is_worktree);
        assert_eq!(branch.name, "main");
    }

    #[test]
    fn test_branch_touch() {
        let mut branch = Branch::new(
            Uuid::new_v4(),
            "test".to_string(),
            "/tmp/test".into(),
            false,
            false,
        );
        let old_activity = branch.last_activity;
        std::thread::sleep(std::time::Duration::from_millis(10));
        branch.touch();
        assert!(branch.last_activity > old_activity);
    }

    #[test]
    fn test_parse_folder_path_splits_and_trims() {
        assert_eq!(
            parse_folder_path(" Acme / Platform ").unwrap(),
            vec!["Acme".to_string(), "Platform".to_string()]
        );
    }

    #[test]
    fn test_parse_folder_path_skips_empty_segments() {
        assert_eq!(
            parse_folder_path("Acme//Platform/").unwrap(),
            vec!["Acme".to_string(), "Platform".to_string()]
        );
    }

    #[test]
    fn test_parse_folder_path_empty_means_root() {
        assert!(parse_folder_path("").unwrap().is_empty());
        assert!(parse_folder_path("   ").unwrap().is_empty());
        assert!(parse_folder_path("/").unwrap().is_empty());
    }

    #[test]
    fn test_parse_folder_path_rejects_excess_depth() {
        assert!(parse_folder_path("a/b/c").is_ok());
        assert!(parse_folder_path("a/b/c/d").is_err());
    }

    #[test]
    fn test_parse_folder_path_rejects_overlong_segment() {
        let long = "x".repeat(MAX_FOLDER_SEGMENT_LEN + 1);
        assert!(parse_folder_path(&long).is_err());
    }

    #[test]
    fn test_folder_path_key_round_trips() {
        let segments = parse_folder_path("Acme/Platform").unwrap();
        assert_eq!(folder_path_key(&segments), "Acme/Platform");
        assert_eq!(folder_path_key(&[]), "");
    }

    #[test]
    fn test_project_defaults_to_root_folder() {
        let project = Project::new("test".to_string(), "/tmp/test".into(), "main".to_string());
        assert!(project.folder.is_empty());
        assert_eq!(project.folder_display(), "(root)");
    }

    #[test]
    fn test_project_serialization_preserves_folder() {
        let mut project = Project::new("test".to_string(), "/tmp/test".into(), "main".to_string());
        project.folder = vec!["Acme".to_string(), "Platform".to_string()];
        let json = serde_json::to_string(&project).unwrap();
        let parsed: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.folder, project.folder);
        assert_eq!(parsed.folder_display(), "Acme/Platform");
    }

    #[test]
    fn test_project_serialization() {
        let project = Project::new("test".to_string(), "/tmp/test".into(), "main".to_string());
        let json = serde_json::to_string(&project).unwrap();
        let parsed: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, project.id);
        assert_eq!(parsed.name, project.name);
    }

    #[test]
    fn test_branch_serialization() {
        let branch = Branch::new(
            Uuid::new_v4(),
            "test".to_string(),
            "/tmp/test".into(),
            true,
            false,
        );
        let json = serde_json::to_string(&branch).unwrap();
        let parsed: Branch = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, branch.id);
        assert_eq!(parsed.name, branch.name);
    }
}
