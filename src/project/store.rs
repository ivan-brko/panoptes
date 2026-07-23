//! Project and branch persistence
//!
//! Handles saving and loading projects and branches to/from disk.

use super::{folder_path_key, Branch, BranchId, Project, ProjectId, MAX_FOLDER_DEPTH};
use crate::config::config_dir;
use crate::persistence::{self, LoadOutcome};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Serializable format for the project store
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreData {
    projects: Vec<Project>,
    branches: Vec<Branch>,
    /// Display paths of folders the user has collapsed in the projects overview
    #[serde(default)]
    collapsed_folders: Vec<String>,
}

/// Store for persisting projects and branches
#[derive(Debug)]
pub struct ProjectStore {
    /// All projects indexed by ID
    projects: HashMap<ProjectId, Project>,
    /// All branches indexed by ID
    branches: HashMap<BranchId, Branch>,
    /// Display paths of folders collapsed in the projects overview
    collapsed_folders: HashSet<String>,
    /// Path to the projects.json file
    store_path: PathBuf,
}

impl Default for ProjectStore {
    fn default() -> Self {
        Self {
            projects: HashMap::new(),
            branches: HashMap::new(),
            collapsed_folders: HashSet::new(),
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
            collapsed_folders: HashSet::new(),
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

    /// Find a project by repository path AND session subdir
    /// Used for duplicate detection when adding projects from subfolders
    pub fn find_by_repo_and_subdir(
        &self,
        repo_path: &Path,
        session_subdir: Option<&Path>,
    ) -> Option<&Project> {
        self.projects
            .values()
            .find(|p| p.repo_path == repo_path && p.session_subdir.as_deref() == session_subdir)
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

    // ====================================================================
    // Folder organization
    // ====================================================================

    /// Move a project into `folder` (empty = root level)
    ///
    /// Returns an error if the folder would exceed [`MAX_FOLDER_DEPTH`].
    pub fn set_project_folder(&mut self, id: ProjectId, folder: Vec<String>) -> Result<()> {
        if folder.len() > MAX_FOLDER_DEPTH {
            bail!(
                "Folders can nest at most {} levels deep (got {})",
                MAX_FOLDER_DEPTH,
                folder.len()
            );
        }
        let project = self
            .projects
            .get_mut(&id)
            .context("Project not found while setting folder")?;
        project.folder = folder;
        Ok(())
    }

    /// Rename the last segment of `path`, keeping the subtree intact
    ///
    /// Returns the number of projects whose folder path changed.
    pub fn rename_folder(&mut self, path: &[String], new_name: &str) -> Result<usize> {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            bail!("Folder name cannot be empty");
        }
        if new_name.contains('/') {
            bail!("Folder names cannot contain '/' - move the folder instead");
        }
        let Some((_, parent)) = path.split_last() else {
            bail!("Cannot rename the root level");
        };

        let mut renamed = 0;
        for project in self.projects.values_mut() {
            if project.is_under_folder(path) {
                project.folder[path.len() - 1] = new_name.to_string();
                renamed += 1;
            }
        }

        let mut new_path = parent.to_vec();
        new_path.push(new_name.to_string());
        self.remap_collapsed(path, Some(&new_path));

        Ok(renamed)
    }

    /// Re-parent a folder subtree under `new_parent` (empty = root level)
    ///
    /// Returns the number of projects that moved.
    pub fn move_folder(&mut self, path: &[String], new_parent: &[String]) -> Result<usize> {
        let Some(name) = path.last() else {
            bail!("Cannot move the root level");
        };
        if new_parent.len() >= path.len() && new_parent[..path.len()] == *path {
            bail!("Cannot move a folder into itself");
        }

        let mut new_path = new_parent.to_vec();
        new_path.push(name.clone());
        if new_path.len() > MAX_FOLDER_DEPTH {
            bail!(
                "Moving here would nest deeper than {} levels",
                MAX_FOLDER_DEPTH
            );
        }

        // The deepest project in the subtree determines the resulting depth
        let deepest = self
            .projects
            .values()
            .filter(|p| p.is_under_folder(path))
            .map(|p| p.folder.len())
            .max()
            .unwrap_or(path.len());
        let resulting_depth = deepest - path.len() + new_path.len();
        if resulting_depth > MAX_FOLDER_DEPTH {
            bail!(
                "Moving here would nest its contents deeper than {} levels",
                MAX_FOLDER_DEPTH
            );
        }

        let mut moved = 0;
        for project in self.projects.values_mut() {
            if project.is_under_folder(path) {
                let mut folder = new_path.clone();
                folder.extend_from_slice(&project.folder[path.len()..]);
                project.folder = folder;
                moved += 1;
            }
        }
        self.remap_collapsed(path, Some(&new_path));

        Ok(moved)
    }

    /// Dissolve a folder, moving its direct contents up one level
    ///
    /// Projects are never deleted. Subfolders are lifted along with their
    /// contents. Returns the number of projects that moved.
    pub fn remove_folder(&mut self, path: &[String]) -> usize {
        if path.is_empty() {
            return 0;
        }
        let cut = path.len() - 1;

        let mut moved = 0;
        for project in self.projects.values_mut() {
            if project.is_under_folder(path) {
                // Drop just this folder's segment, keeping any nested structure
                project.folder.remove(cut);
                moved += 1;
            }
        }
        self.remap_collapsed(path, None);

        moved
    }

    /// Whether a folder is currently collapsed in the overview
    pub fn is_folder_collapsed(&self, path: &[String]) -> bool {
        self.collapsed_folders.contains(&folder_path_key(path))
    }

    /// Toggle a folder's collapsed state, returning the new state
    pub fn toggle_folder_collapsed(&mut self, path: &[String]) -> bool {
        let key = folder_path_key(path);
        if self.collapsed_folders.remove(&key) {
            false
        } else {
            self.collapsed_folders.insert(key);
            true
        }
    }

    /// Set a folder's collapsed state explicitly
    pub fn set_folder_collapsed(&mut self, path: &[String], collapsed: bool) {
        let key = folder_path_key(path);
        if collapsed {
            self.collapsed_folders.insert(key);
        } else {
            self.collapsed_folders.remove(&key);
        }
    }

    /// The set of collapsed folder keys, for tree flattening
    pub fn collapsed_folders(&self) -> &HashSet<String> {
        &self.collapsed_folders
    }

    /// Rewrite collapse keys for a subtree that moved (`Some`) or vanished (`None`)
    fn remap_collapsed(&mut self, old_path: &[String], new_path: Option<&[String]>) {
        let old_key = folder_path_key(old_path);
        let prefix = format!("{}/", old_key);

        let affected: Vec<String> = self
            .collapsed_folders
            .iter()
            .filter(|k| **k == old_key || k.starts_with(&prefix))
            .cloned()
            .collect();

        for key in affected {
            self.collapsed_folders.remove(&key);
            if let Some(new_path) = new_path {
                let suffix = &key[old_key.len()..];
                self.collapsed_folders
                    .insert(format!("{}{}", folder_path_key(new_path), suffix));
            }
        }
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

    /// Refresh branch status for a project
    ///
    /// This checks if each worktree's working directory still exists and
    /// marks branches as stale if their directories are missing.
    ///
    /// Returns the number of branches that were marked stale.
    pub fn refresh_branches(&mut self, project_id: ProjectId) -> usize {
        let mut stale_count = 0;

        // Get branch IDs for this project
        let branch_ids: Vec<BranchId> = self
            .branches
            .values()
            .filter(|b| b.project_id == project_id)
            .map(|b| b.id)
            .collect();

        // Check each branch
        for branch_id in branch_ids {
            if let Some(branch) = self.branches.get_mut(&branch_id) {
                let exists = branch.working_dir.exists();
                let was_stale = branch.stale;

                if exists {
                    branch.stale = false;
                } else if branch.is_worktree {
                    // Only mark worktrees as stale if their directory is missing
                    // The default branch (local checkout) might be in a state where
                    // the repo itself was moved, which is a different problem
                    branch.stale = true;
                    if !was_stale {
                        stale_count += 1;
                        tracing::warn!(
                            "Worktree '{}' marked stale: directory {:?} not found",
                            branch.name,
                            branch.working_dir
                        );
                    }
                }
            }
        }

        stale_count
    }

    /// Get stale branch count for a project
    pub fn stale_branch_count(&self, project_id: ProjectId) -> usize {
        self.branches
            .values()
            .filter(|b| b.project_id == project_id && b.stale)
            .count()
    }

    /// Load store from disk, returning a warning message if data was corrupted
    ///
    /// This is useful for showing a notification to the user on startup.
    pub fn load_with_status() -> (Self, Option<String>) {
        Self::load_from_with_status(&projects_file_path())
    }

    /// Load store from a specific path, returning a warning message if data was corrupted
    pub fn load_from_with_status(path: &Path) -> (Self, Option<String>) {
        match persistence::load_json::<StoreData>(path, "projects") {
            LoadOutcome::Absent => (Self::with_path(path.to_path_buf()), None),
            LoadOutcome::Loaded(data) => (Self::from_data(data, path), None),
            LoadOutcome::Corrupted { fallback_warning } => {
                (Self::with_path(path.to_path_buf()), Some(fallback_warning))
            }
        }
    }

    /// Build a store from parsed data
    fn from_data(data: StoreData, path: &Path) -> Self {
        Self {
            projects: data.projects.into_iter().map(|p| (p.id, p)).collect(),
            branches: data.branches.into_iter().map(|b| (b.id, b)).collect(),
            collapsed_folders: data.collapsed_folders.into_iter().collect(),
            store_path: path.to_path_buf(),
        }
    }

    /// Save store to disk
    pub fn save(&self) -> Result<()> {
        self.save_to(&self.store_path)
    }

    /// Save store to a specific path (atomically, via a sibling temp file)
    ///
    /// The projects file is saved during live use, so a crash mid-write must
    /// never be able to truncate it.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        // Drop collapse entries for folders that no longer hold any project,
        // so the file does not accumulate stale paths
        let mut collapsed_folders: Vec<String> = self
            .collapsed_folders
            .iter()
            .filter(|key| {
                let segments: Vec<String> = key.split('/').map(|s| s.to_string()).collect();
                self.projects.values().any(|p| p.is_under_folder(&segments))
            })
            .cloned()
            .collect();
        collapsed_folders.sort();

        let data = StoreData {
            projects: self.projects.values().cloned().collect(),
            branches: self.branches.values().cloned().collect(),
            collapsed_folders,
        };

        persistence::save_json_atomic(path, &data, "projects")
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
        let (loaded, warning) = ProjectStore::load_from_with_status(&path);
        assert!(warning.is_none());
        assert_eq!(loaded.project_count(), 1);
        assert_eq!(loaded.branch_count(), 1);
        assert!(loaded.get_project(project_id).is_some());
        assert!(loaded.get_branch(branch_id).is_some());
    }

    #[test]
    fn test_store_load_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.json");

        let (store, warning) = ProjectStore::load_from_with_status(&path);
        assert!(warning.is_none());
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

    // load_with_status corruption handling tests
    #[test]
    fn test_load_with_status_nonexistent_returns_empty() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.json");

        let (store, warning) = ProjectStore::load_from_with_status(&path);

        assert_eq!(store.project_count(), 0);
        assert!(warning.is_none());
    }

    #[test]
    fn test_load_with_status_valid_json() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("projects.json");

        // Create valid store and save
        let mut store = ProjectStore::with_path(path.clone());
        let project = Project::new("test".to_string(), "/tmp/test".into(), "main".to_string());
        store.add_project(project);
        store.save().unwrap();

        // Load with status
        let (loaded, warning) = ProjectStore::load_from_with_status(&path);

        assert_eq!(loaded.project_count(), 1);
        assert!(warning.is_none());
    }

    #[test]
    fn test_load_with_status_corrupted_json() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("projects.json");

        // Write invalid JSON
        std::fs::write(&path, "{ invalid json }").unwrap();

        // Load with status should return empty store with warning
        let (store, warning) = ProjectStore::load_from_with_status(&path);

        assert_eq!(store.project_count(), 0);
        assert!(warning.is_some());
        assert!(warning.unwrap().contains("corrupted"));
    }

    // ====================================================================
    // Folder organization tests
    // ====================================================================

    fn folder(segments: &[&str]) -> Vec<String> {
        segments.iter().map(|s| s.to_string()).collect()
    }

    /// Add a project filed under `segments`, returning its id
    fn add_filed_project(store: &mut ProjectStore, name: &str, segments: &[&str]) -> ProjectId {
        let mut project = Project::new(
            name.to_string(),
            PathBuf::from(format!("/tmp/{}", name)),
            "main".to_string(),
        );
        project.folder = folder(segments);
        let id = project.id;
        store.add_project(project);
        id
    }

    #[test]
    fn test_set_project_folder() {
        let mut store = ProjectStore::new();
        let id = add_filed_project(&mut store, "api", &[]);

        store
            .set_project_folder(id, folder(&["Acme", "Platform"]))
            .unwrap();

        assert_eq!(
            store.get_project(id).unwrap().folder,
            folder(&["Acme", "Platform"])
        );
    }

    #[test]
    fn test_set_project_folder_rejects_excess_depth() {
        let mut store = ProjectStore::new();
        let id = add_filed_project(&mut store, "api", &[]);

        let result = store.set_project_folder(id, folder(&["a", "b", "c", "d"]));

        assert!(result.is_err());
        assert!(store.get_project(id).unwrap().folder.is_empty());
    }

    #[test]
    fn test_rename_folder_rewrites_subtree() {
        let mut store = ProjectStore::new();
        let shallow = add_filed_project(&mut store, "api", &["Acme"]);
        let deep = add_filed_project(&mut store, "web", &["Acme", "Platform"]);
        let other = add_filed_project(&mut store, "blog", &["Personal"]);

        let renamed = store.rename_folder(&folder(&["Acme"]), "AcmeCorp").unwrap();

        assert_eq!(renamed, 2);
        assert_eq!(
            store.get_project(shallow).unwrap().folder,
            folder(&["AcmeCorp"])
        );
        assert_eq!(
            store.get_project(deep).unwrap().folder,
            folder(&["AcmeCorp", "Platform"])
        );
        assert_eq!(
            store.get_project(other).unwrap().folder,
            folder(&["Personal"])
        );
    }

    #[test]
    fn test_rename_folder_rejects_slash_and_empty() {
        let mut store = ProjectStore::new();
        add_filed_project(&mut store, "api", &["Acme"]);

        assert!(store.rename_folder(&folder(&["Acme"]), "A/B").is_err());
        assert!(store.rename_folder(&folder(&["Acme"]), "   ").is_err());
    }

    #[test]
    fn test_rename_folder_carries_collapse_state() {
        let mut store = ProjectStore::new();
        add_filed_project(&mut store, "web", &["Acme", "Platform"]);
        store.set_folder_collapsed(&folder(&["Acme", "Platform"]), true);

        store.rename_folder(&folder(&["Acme"]), "AcmeCorp").unwrap();

        assert!(store.is_folder_collapsed(&folder(&["AcmeCorp", "Platform"])));
        assert!(!store.is_folder_collapsed(&folder(&["Acme", "Platform"])));
    }

    #[test]
    fn test_move_folder_reparents_subtree() {
        let mut store = ProjectStore::new();
        let id = add_filed_project(&mut store, "web", &["Platform"]);

        let moved = store
            .move_folder(&folder(&["Platform"]), &folder(&["Acme"]))
            .unwrap();

        assert_eq!(moved, 1);
        assert_eq!(
            store.get_project(id).unwrap().folder,
            folder(&["Acme", "Platform"])
        );
    }

    #[test]
    fn test_move_folder_to_root() {
        let mut store = ProjectStore::new();
        let id = add_filed_project(&mut store, "web", &["Acme", "Platform"]);

        store
            .move_folder(&folder(&["Acme", "Platform"]), &[])
            .unwrap();

        assert_eq!(store.get_project(id).unwrap().folder, folder(&["Platform"]));
    }

    #[test]
    fn test_move_folder_rejects_move_into_itself() {
        let mut store = ProjectStore::new();
        add_filed_project(&mut store, "web", &["Acme", "Platform"]);

        let result = store.move_folder(&folder(&["Acme"]), &folder(&["Acme", "Platform"]));

        assert!(result.is_err());
    }

    #[test]
    fn test_move_folder_rejects_when_contents_would_exceed_depth() {
        let mut store = ProjectStore::new();
        // "Group" holds a two-level subtree; nesting it one deeper hits the limit
        add_filed_project(&mut store, "web", &["Group", "Mid", "Leaf"]);
        add_filed_project(&mut store, "api", &["Other"]);

        let result = store.move_folder(&folder(&["Group"]), &folder(&["Other"]));

        assert!(result.is_err());
        assert_eq!(
            store.projects().find(|p| p.name == "web").unwrap().folder,
            folder(&["Group", "Mid", "Leaf"])
        );
    }

    #[test]
    fn test_remove_folder_lifts_contents_without_deleting() {
        let mut store = ProjectStore::new();
        let direct = add_filed_project(&mut store, "api", &["Acme"]);
        let nested = add_filed_project(&mut store, "web", &["Acme", "Platform"]);

        let moved = store.remove_folder(&folder(&["Acme"]));

        assert_eq!(moved, 2);
        assert_eq!(store.project_count(), 2);
        assert!(store.get_project(direct).unwrap().folder.is_empty());
        assert_eq!(
            store.get_project(nested).unwrap().folder,
            folder(&["Platform"])
        );
    }

    #[test]
    fn test_remove_nested_folder_keeps_parent() {
        let mut store = ProjectStore::new();
        let id = add_filed_project(&mut store, "web", &["Acme", "Platform"]);

        store.remove_folder(&folder(&["Acme", "Platform"]));

        assert_eq!(store.get_project(id).unwrap().folder, folder(&["Acme"]));
    }

    #[test]
    fn test_toggle_folder_collapsed() {
        let mut store = ProjectStore::new();
        let path = folder(&["Acme"]);

        assert!(!store.is_folder_collapsed(&path));
        assert!(store.toggle_folder_collapsed(&path));
        assert!(store.is_folder_collapsed(&path));
        assert!(!store.toggle_folder_collapsed(&path));
        assert!(!store.is_folder_collapsed(&path));
    }

    #[test]
    fn test_folders_and_collapse_state_survive_round_trip() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("projects.json");

        let mut store = ProjectStore::with_path(path.clone());
        let id = add_filed_project(&mut store, "web", &["Acme", "Platform"]);
        store.set_folder_collapsed(&folder(&["Acme"]), true);
        store.save().unwrap();

        let (loaded, warning) = ProjectStore::load_from_with_status(&path);
        assert!(warning.is_none());

        assert_eq!(
            loaded.get_project(id).unwrap().folder,
            folder(&["Acme", "Platform"])
        );
        assert!(loaded.is_folder_collapsed(&folder(&["Acme"])));
    }

    #[test]
    fn test_save_prunes_collapse_state_for_vanished_folders() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("projects.json");

        let mut store = ProjectStore::with_path(path.clone());
        add_filed_project(&mut store, "web", &["Acme"]);
        store.set_folder_collapsed(&folder(&["Acme"]), true);
        store.set_folder_collapsed(&folder(&["Ghost"]), true);
        store.save().unwrap();

        let (loaded, warning) = ProjectStore::load_from_with_status(&path);
        assert!(warning.is_none());

        assert!(loaded.is_folder_collapsed(&folder(&["Acme"])));
        assert!(!loaded.is_folder_collapsed(&folder(&["Ghost"])));
    }

    #[test]
    fn test_legacy_store_without_folders_loads() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("projects.json");
        // A pre-folders projects.json has neither `folder` nor `collapsed_folders`
        let legacy = r#"{
            "projects": [{
                "id": "6f9619ff-8b86-d011-b42d-00cf4fc964ff",
                "name": "legacy",
                "repo_path": "/tmp/legacy",
                "remote_url": null,
                "default_branch": "main",
                "created_at": "2024-01-01T00:00:00Z",
                "last_activity": "2024-01-01T00:00:00Z"
            }],
            "branches": []
        }"#;
        std::fs::write(&path, legacy).unwrap();

        let (store, warning) = ProjectStore::load_from_with_status(&path);
        assert!(warning.is_none());

        assert_eq!(store.project_count(), 1);
        assert!(store.projects().next().unwrap().folder.is_empty());
        assert!(store.collapsed_folders().is_empty());
    }

    #[test]
    fn test_backup_corrupted_file_creates_timestamped_backup() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("projects.json");

        // Write invalid JSON
        std::fs::write(&path, "{ invalid json }").unwrap();

        // Load with status (triggers backup)
        let _ = ProjectStore::load_from_with_status(&path);

        // Check for backup file with timestamp pattern
        let entries: Vec<_> = std::fs::read_dir(temp_dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt."))
            .collect();

        assert_eq!(entries.len(), 1);
    }
}
