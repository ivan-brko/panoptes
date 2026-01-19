//! Project and branch persistence
//!
//! Handles saving and loading projects and branches to/from disk.

use super::{Branch, BranchId, Project, ProjectId};
use crate::config::config_dir;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Serializable format for the project store
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreData {
    projects: Vec<Project>,
    branches: Vec<Branch>,
}

/// Store for persisting projects and branches
#[derive(Debug)]
pub struct ProjectStore {
    /// All projects indexed by ID
    projects: HashMap<ProjectId, Project>,
    /// All branches indexed by ID
    branches: HashMap<BranchId, Branch>,
    /// Path to the projects.json file
    store_path: PathBuf,
}

impl Default for ProjectStore {
    fn default() -> Self {
        Self {
            projects: HashMap::new(),
            branches: HashMap::new(),
            store_path: projects_file_path(),
        }
    }
}

impl ProjectStore {
    /// Create a new empty store
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a store with a custom path (for testing)
    pub fn with_path(path: PathBuf) -> Self {
        Self {
            projects: HashMap::new(),
            branches: HashMap::new(),
            store_path: path,
        }
    }

    /// Get all projects
    pub fn projects(&self) -> impl Iterator<Item = &Project> {
        self.projects.values()
    }

    /// Get all projects as a sorted vector (by name)
    pub fn projects_sorted(&self) -> Vec<&Project> {
        let mut projects: Vec<_> = self.projects.values().collect();
        projects.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        projects
    }

    /// Get a project by ID
    pub fn get_project(&self, id: ProjectId) -> Option<&Project> {
        self.projects.get(&id)
    }

    /// Get a mutable reference to a project by ID
    pub fn get_project_mut(&mut self, id: ProjectId) -> Option<&mut Project> {
        self.projects.get_mut(&id)
    }

    /// Find a project by repository path
    pub fn find_by_repo_path(&self, path: &Path) -> Option<&Project> {
        self.projects.values().find(|p| p.repo_path == path)
    }

    /// Add a project to the store
    pub fn add_project(&mut self, project: Project) {
        self.projects.insert(project.id, project);
    }

    /// Remove a project and all its branches
    pub fn remove_project(&mut self, id: ProjectId) -> Option<Project> {
        // Remove all branches for this project
        let branch_ids: Vec<_> = self
            .branches
            .values()
            .filter(|b| b.project_id == id)
            .map(|b| b.id)
            .collect();
        for branch_id in branch_ids {
            self.branches.remove(&branch_id);
        }
        self.projects.remove(&id)
    }

    /// Get all branches
    pub fn branches(&self) -> impl Iterator<Item = &Branch> {
        self.branches.values()
    }

    /// Get a branch by ID
    pub fn get_branch(&self, id: BranchId) -> Option<&Branch> {
        self.branches.get(&id)
    }

    /// Get a mutable reference to a branch by ID
    pub fn get_branch_mut(&mut self, id: BranchId) -> Option<&mut Branch> {
        self.branches.get_mut(&id)
    }

    /// Get all branches for a project
    pub fn branches_for_project(&self, project_id: ProjectId) -> Vec<&Branch> {
        self.branches
            .values()
            .filter(|b| b.project_id == project_id)
            .collect()
    }

    /// Get all branches for a project sorted (default first, then by name)
    pub fn branches_for_project_sorted(&self, project_id: ProjectId) -> Vec<&Branch> {
        let mut branches = self.branches_for_project(project_id);
        branches.sort_by(|a, b| {
            // Default branch comes first
            match (a.is_default, b.is_default) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });
        branches
    }

    /// Find a branch by project and name
    pub fn find_branch(&self, project_id: ProjectId, name: &str) -> Option<&Branch> {
        self.branches
            .values()
            .find(|b| b.project_id == project_id && b.name == name)
    }

    /// Add a branch to the store
    pub fn add_branch(&mut self, branch: Branch) {
        self.branches.insert(branch.id, branch);
    }

    /// Remove a branch
    pub fn remove_branch(&mut self, id: BranchId) -> Option<Branch> {
        self.branches.remove(&id)
    }

    /// Get the number of projects
    pub fn project_count(&self) -> usize {
        self.projects.len()
    }

    /// Get the number of branches
    pub fn branch_count(&self) -> usize {
        self.branches.len()
    }

    /// Get the number of branches for a specific project
    pub fn branch_count_for_project(&self, project_id: ProjectId) -> usize {
        self.branches
            .values()
            .filter(|b| b.project_id == project_id)
            .count()
    }

    /// Load store from disk
    pub fn load() -> Result<Self> {
        Self::load_from(&projects_file_path())
    }

    /// Load store from a specific path
    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::with_path(path.to_path_buf()));
        }

        let content = std::fs::read_to_string(path).context("Failed to read projects file")?;

        let data: StoreData =
            serde_json::from_str(&content).context("Failed to parse projects file")?;

        let projects = data.projects.into_iter().map(|p| (p.id, p)).collect();
        let branches = data.branches.into_iter().map(|b| (b.id, b)).collect();

        Ok(Self {
            projects,
            branches,
            store_path: path.to_path_buf(),
        })
    }

    /// Save store to disk
    pub fn save(&self) -> Result<()> {
        self.save_to(&self.store_path)
    }

    /// Save store to a specific path
    pub fn save_to(&self, path: &Path) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .context("Failed to create directory for projects file")?;
        }

        let data = StoreData {
            projects: self.projects.values().cloned().collect(),
            branches: self.branches.values().cloned().collect(),
        };

        let content =
            serde_json::to_string_pretty(&data).context("Failed to serialize projects")?;

        std::fs::write(path, content).context("Failed to write projects file")?;

        Ok(())
    }
}

/// Get the path to the projects file
pub fn projects_file_path() -> PathBuf {
    config_dir().join("projects.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_store_add_project() {
        let mut store = ProjectStore::new();
        let project = Project::new("test".to_string(), "/tmp/test".into(), "main".to_string());
        let project_id = project.id;

        store.add_project(project);
        assert_eq!(store.project_count(), 1);
        assert!(store.get_project(project_id).is_some());
    }

    #[test]
    fn test_store_add_branch() {
        let mut store = ProjectStore::new();
        let project = Project::new("test".to_string(), "/tmp/test".into(), "main".to_string());
        let project_id = project.id;
        store.add_project(project);

        let branch =
            Branch::default_for_project(project_id, "main".to_string(), "/tmp/test".into());
        let branch_id = branch.id;

        store.add_branch(branch);
        assert_eq!(store.branch_count(), 1);
        assert!(store.get_branch(branch_id).is_some());
    }

    #[test]
    fn test_store_branches_for_project() {
        let mut store = ProjectStore::new();
        let project = Project::new("test".to_string(), "/tmp/test".into(), "main".to_string());
        let project_id = project.id;
        store.add_project(project);

        let branch1 =
            Branch::default_for_project(project_id, "main".to_string(), "/tmp/test".into());
        let branch2 = Branch::new(
            project_id,
            "feature".to_string(),
            "/tmp/worktree".into(),
            false,
            true,
        );

        store.add_branch(branch1);
        store.add_branch(branch2);

        let branches = store.branches_for_project(project_id);
        assert_eq!(branches.len(), 2);
    }

    #[test]
    fn test_store_remove_project_cascades() {
        let mut store = ProjectStore::new();
        let project = Project::new("test".to_string(), "/tmp/test".into(), "main".to_string());
        let project_id = project.id;
        store.add_project(project);

        let branch =
            Branch::default_for_project(project_id, "main".to_string(), "/tmp/test".into());
        store.add_branch(branch);

        assert_eq!(store.project_count(), 1);
        assert_eq!(store.branch_count(), 1);

        store.remove_project(project_id);
        assert_eq!(store.project_count(), 0);
        assert_eq!(store.branch_count(), 0);
    }

    #[test]
    fn test_store_find_by_repo_path() {
        let mut store = ProjectStore::new();
        let project = Project::new("test".to_string(), "/tmp/test".into(), "main".to_string());
        let project_id = project.id;
        store.add_project(project);

        let found = store.find_by_repo_path(Path::new("/tmp/test"));
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, project_id);

        let not_found = store.find_by_repo_path(Path::new("/tmp/other"));
        assert!(not_found.is_none());
    }

    #[test]
    fn test_store_find_branch() {
        let mut store = ProjectStore::new();
        let project = Project::new("test".to_string(), "/tmp/test".into(), "main".to_string());
        let project_id = project.id;
        store.add_project(project);

        let branch =
            Branch::default_for_project(project_id, "main".to_string(), "/tmp/test".into());
        store.add_branch(branch);

        let found = store.find_branch(project_id, "main");
        assert!(found.is_some());

        let not_found = store.find_branch(project_id, "nonexistent");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_store_branches_sorted() {
        let mut store = ProjectStore::new();
        let project = Project::new("test".to_string(), "/tmp/test".into(), "main".to_string());
        let project_id = project.id;
        store.add_project(project);

        // Add branches in non-sorted order
        let branch_z = Branch::new(
            project_id,
            "z-feature".to_string(),
            "/tmp/worktree-z".into(),
            false,
            true,
        );
        let branch_a = Branch::new(
            project_id,
            "a-feature".to_string(),
            "/tmp/worktree-a".into(),
            false,
            true,
        );
        let branch_main =
            Branch::default_for_project(project_id, "main".to_string(), "/tmp/test".into());

        store.add_branch(branch_z);
        store.add_branch(branch_a);
        store.add_branch(branch_main);

        let sorted = store.branches_for_project_sorted(project_id);
        assert_eq!(sorted.len(), 3);
        // Default (main) should be first
        assert!(sorted[0].is_default);
        assert_eq!(sorted[0].name, "main");
        // Then alphabetical
        assert_eq!(sorted[1].name, "a-feature");
        assert_eq!(sorted[2].name, "z-feature");
    }

    #[test]
    fn test_store_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("projects.json");

        // Create and save store
        let mut store = ProjectStore::with_path(path.clone());
        let project = Project::new("test".to_string(), "/tmp/test".into(), "main".to_string());
        let project_id = project.id;
        store.add_project(project);

        let branch =
            Branch::default_for_project(project_id, "main".to_string(), "/tmp/test".into());
        let branch_id = branch.id;
        store.add_branch(branch);

        store.save().unwrap();

        // Load into new store
        let loaded = ProjectStore::load_from(&path).unwrap();
        assert_eq!(loaded.project_count(), 1);
        assert_eq!(loaded.branch_count(), 1);
        assert!(loaded.get_project(project_id).is_some());
        assert!(loaded.get_branch(branch_id).is_some());
    }

    #[test]
    fn test_store_load_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.json");

        let store = ProjectStore::load_from(&path).unwrap();
        assert_eq!(store.project_count(), 0);
        assert_eq!(store.branch_count(), 0);
    }

    #[test]
    fn test_projects_sorted() {
        let mut store = ProjectStore::new();

        store.add_project(Project::new(
            "zebra".to_string(),
            "/tmp/z".into(),
            "main".to_string(),
        ));
        store.add_project(Project::new(
            "alpha".to_string(),
            "/tmp/a".into(),
            "main".to_string(),
        ));
        store.add_project(Project::new(
            "Beta".to_string(),
            "/tmp/b".into(),
            "main".to_string(),
        ));

        let sorted = store.projects_sorted();
        assert_eq!(sorted[0].name, "alpha");
        assert_eq!(sorted[1].name, "Beta");
        assert_eq!(sorted[2].name, "zebra");
    }
}
