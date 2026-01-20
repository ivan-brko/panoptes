//! Project and branch management module
//!
//! This module handles project and branch data structures for organizing
//! sessions by git repository and branch.

pub mod store;

pub use store::ProjectStore;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Unique identifier for a project
pub type ProjectId = Uuid;

/// Unique identifier for a branch
pub type BranchId = Uuid;

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
            created_at: now,
            last_activity: now,
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
            created_at: now,
            last_activity: now,
        }
    }

    /// Create the default branch for a project
    pub fn default_for_project(project_id: ProjectId, name: String, repo_path: PathBuf) -> Self {
        Self::new(project_id, name, repo_path, true, false)
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
