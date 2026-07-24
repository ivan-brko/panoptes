//! Folder tree flattening for the projects overview
//!
//! Folders are implicit: they exist because projects reference them via
//! [`Project::folder`]. This module turns that flat list into the ordered rows
//! the projects overview renders and navigates, so selection stays a simple
//! index into a `Vec`.

use std::collections::HashSet;

use super::{folder_path_key, Project, ProjectId, ProjectStore};

/// A folder row in the flattened tree
#[derive(Debug, Clone)]
pub struct FolderRow {
    /// Full path segments of this folder, e.g. `["Acme", "Platform"]`
    pub path: Vec<String>,
    /// Indentation level (0 for a top-level folder)
    pub depth: usize,
    /// Whether this folder's contents are shown
    pub expanded: bool,
    /// Projects in this folder and all of its subfolders
    pub descendants: Vec<ProjectId>,
}

impl FolderRow {
    /// Name of this folder (its last path segment)
    pub fn name(&self) -> &str {
        self.path.last().map(|s| s.as_str()).unwrap_or("")
    }

    /// Lookup key for collapse state
    pub fn key(&self) -> String {
        folder_path_key(&self.path)
    }
}

/// A project row in the flattened tree
#[derive(Debug, Clone)]
pub struct ProjectRow<'a> {
    /// The project this row displays
    pub project: &'a Project,
    /// Indentation level (0 when the project sits at the root)
    pub depth: usize,
}

/// One visible line of the projects overview
#[derive(Debug, Clone)]
pub enum TreeRow<'a> {
    /// A folder heading
    Folder(FolderRow),
    /// A project entry
    Project(ProjectRow<'a>),
}

impl TreeRow<'_> {
    /// Indentation level of this row
    pub fn depth(&self) -> usize {
        match self {
            TreeRow::Folder(f) => f.depth,
            TreeRow::Project(p) => p.depth,
        }
    }

    /// The project this row points at, if it is a project row
    pub fn project_id(&self) -> Option<ProjectId> {
        match self {
            TreeRow::Project(p) => Some(p.project.id),
            TreeRow::Folder(_) => None,
        }
    }

    /// The folder path this row points at, if it is a folder row
    pub fn folder_path(&self) -> Option<&[String]> {
        match self {
            TreeRow::Folder(f) => Some(&f.path),
            TreeRow::Project(_) => None,
        }
    }

    /// Projects affected by an action on this row (one for a project row,
    /// the whole subtree for a folder row)
    pub fn affected_projects(&self) -> Vec<ProjectId> {
        match self {
            TreeRow::Folder(f) => f.descendants.clone(),
            TreeRow::Project(p) => vec![p.project.id],
        }
    }
}

/// Flatten the store into the rows currently visible in the projects overview
///
/// Folders come before projects at each level; both are sorted case-insensitively
/// by name. Collapsed folders contribute their heading row but none of their
/// contents.
pub fn visible_rows<'a>(store: &'a ProjectStore, collapsed: &HashSet<String>) -> Vec<TreeRow<'a>> {
    let projects = store.projects_sorted();
    let mut rows = Vec::new();
    let mut prefix = Vec::new();
    walk(&mut prefix, &projects, collapsed, &mut rows);
    rows
}

/// Recursively emit folder and project rows for one level of the tree
fn walk<'a>(
    prefix: &mut Vec<String>,
    projects: &[&'a Project],
    collapsed: &HashSet<String>,
    rows: &mut Vec<TreeRow<'a>>,
) {
    let depth = prefix.len();

    // Distinct child folder names at this level, in case-insensitive order
    let mut child_names: Vec<&str> = projects
        .iter()
        .filter(|p| p.folder.len() > depth && p.is_under_folder(prefix))
        .map(|p| p.folder[depth].as_str())
        .collect();
    child_names.sort_by_key(|n| n.to_lowercase());
    child_names.dedup();

    for name in child_names {
        prefix.push(name.to_string());

        let expanded = !collapsed.contains(&folder_path_key(prefix));
        let descendants: Vec<ProjectId> = projects
            .iter()
            .filter(|p| p.is_under_folder(prefix))
            .map(|p| p.id)
            .collect();

        rows.push(TreeRow::Folder(FolderRow {
            path: prefix.clone(),
            depth,
            expanded,
            descendants,
        }));

        if expanded {
            walk(prefix, projects, collapsed, rows);
        }
        prefix.pop();
    }

    // Then the projects that live directly at this level
    for project in projects.iter().filter(|p| p.is_in_folder(prefix)) {
        rows.push(TreeRow::Project(ProjectRow { project, depth }));
    }
}

/// An owned snapshot of a visible row
///
/// Input handlers need to mutate the store while acting on the selected row,
/// which a borrowed [`TreeRow`] would prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowRef {
    /// A project row
    Project(ProjectId),
    /// A folder row
    Folder {
        /// Full path segments of the folder
        path: Vec<String>,
        /// Whether the folder is currently expanded
        expanded: bool,
        /// Projects in this folder and its subfolders
        descendants: Vec<ProjectId>,
    },
}

impl RowRef {
    /// The project this row points at, if it is a project row
    pub fn project_id(&self) -> Option<ProjectId> {
        match self {
            RowRef::Project(id) => Some(*id),
            RowRef::Folder { .. } => None,
        }
    }
}

/// Number of rows currently visible in the projects overview
pub fn row_count(store: &ProjectStore) -> usize {
    visible_rows(store, store.collapsed_folders()).len()
}

/// Snapshot of the visible row at `index`
pub fn row_at(store: &ProjectStore, index: usize) -> Option<RowRef> {
    visible_rows(store, store.collapsed_folders())
        .get(index)
        .map(|row| match row {
            TreeRow::Project(p) => RowRef::Project(p.project.id),
            TreeRow::Folder(f) => RowRef::Folder {
                path: f.path.clone(),
                expanded: f.expanded,
                descendants: f.descendants.clone(),
            },
        })
}

/// Index of the visible row for a project, if the project is not hidden
/// inside a collapsed folder
pub fn row_index_of_project(store: &ProjectStore, id: ProjectId) -> Option<usize> {
    visible_rows(store, store.collapsed_folders())
        .iter()
        .position(|row| row.project_id() == Some(id))
}

/// Index of the visible row for a folder
pub fn row_index_of_folder(store: &ProjectStore, path: &[String]) -> Option<usize> {
    visible_rows(store, store.collapsed_folders())
        .iter()
        .position(|row| row.folder_path() == Some(path))
}

/// All distinct folder paths in the store, including intermediate folders
///
/// Sorted case-insensitively by display path; used for move-target autocomplete.
pub fn all_folder_paths(store: &ProjectStore) -> Vec<Vec<String>> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut paths: Vec<Vec<String>> = Vec::new();

    for project in store.projects() {
        for depth in 1..=project.folder.len() {
            let path = project.folder[..depth].to_vec();
            if seen.insert(folder_path_key(&path)) {
                paths.push(path);
            }
        }
    }

    paths.sort_by_key(|p| folder_path_key(p).to_lowercase());
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn project_in(name: &str, folder: &[&str]) -> Project {
        let mut project = Project::new(
            name.to_string(),
            PathBuf::from(format!("/tmp/{}", name)),
            "main".to_string(),
        );
        project.folder = folder.iter().map(|s| s.to_string()).collect();
        project
    }

    fn store_with(projects: Vec<Project>) -> ProjectStore {
        let mut store = ProjectStore::new();
        for project in projects {
            store.add_project(project);
        }
        store
    }

    /// Render rows as "indent:kind:name" strings for compact assertions
    fn render(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                TreeRow::Folder(f) => format!("{}d:{}", f.depth, f.name()),
                TreeRow::Project(p) => format!("{}p:{}", p.depth, p.project.name),
            })
            .collect()
    }

    #[test]
    fn test_flat_store_has_no_folder_rows() {
        let store = store_with(vec![project_in("beta", &[]), project_in("alpha", &[])]);
        let rows = visible_rows(&store, &HashSet::new());
        assert_eq!(render(&rows), vec!["0p:alpha", "0p:beta"]);
    }

    #[test]
    fn test_folders_precede_projects_at_each_level() {
        let store = store_with(vec![
            project_in("zulu", &[]),
            project_in("api", &["Acme"]),
            project_in("web", &["Acme", "Platform"]),
        ]);
        let rows = visible_rows(&store, &HashSet::new());
        assert_eq!(
            render(&rows),
            vec!["0d:Acme", "1d:Platform", "2p:web", "1p:api", "0p:zulu"]
        );
    }

    #[test]
    fn test_collapsed_folder_hides_descendants_but_keeps_heading() {
        let store = store_with(vec![
            project_in("api", &["Acme"]),
            project_in("web", &["Acme", "Platform"]),
            project_in("solo", &[]),
        ]);
        let collapsed: HashSet<String> = ["Acme".to_string()].into_iter().collect();
        let rows = visible_rows(&store, &collapsed);
        assert_eq!(render(&rows), vec!["0d:Acme", "0p:solo"]);
    }

    #[test]
    fn test_collapsed_folder_still_reports_all_descendants() {
        let store = store_with(vec![
            project_in("api", &["Acme"]),
            project_in("web", &["Acme", "Platform"]),
        ]);
        let collapsed: HashSet<String> = ["Acme".to_string()].into_iter().collect();
        let rows = visible_rows(&store, &collapsed);
        match &rows[0] {
            TreeRow::Folder(f) => assert_eq!(f.descendants.len(), 2),
            _ => panic!("expected folder row"),
        }
    }

    #[test]
    fn test_intermediate_folders_are_synthesized() {
        // Nothing lives directly in "Acme" - it exists only as a parent
        let store = store_with(vec![project_in("web", &["Acme", "Platform"])]);
        let rows = visible_rows(&store, &HashSet::new());
        assert_eq!(render(&rows), vec!["0d:Acme", "1d:Platform", "2p:web"]);
    }

    #[test]
    fn test_all_folder_paths_includes_intermediates_and_dedupes() {
        let store = store_with(vec![
            project_in("web", &["Acme", "Platform"]),
            project_in("api", &["Acme", "Platform"]),
            project_in("blog", &["Personal"]),
        ]);
        let paths = all_folder_paths(&store);
        let keys: Vec<String> = paths.iter().map(|p| folder_path_key(p)).collect();
        assert_eq!(keys, vec!["Acme", "Acme/Platform", "Personal"]);
    }

    #[test]
    fn test_row_at_distinguishes_folders_from_projects() {
        let store = store_with(vec![project_in("api", &["Acme"])]);

        match row_at(&store, 0) {
            Some(RowRef::Folder { path, expanded, .. }) => {
                assert_eq!(path, vec!["Acme".to_string()]);
                assert!(expanded);
            }
            other => panic!("expected folder row, got {:?}", other),
        }
        assert!(matches!(row_at(&store, 1), Some(RowRef::Project(_))));
        assert!(row_at(&store, 2).is_none());
    }

    #[test]
    fn test_row_count_matches_visible_rows() {
        let mut store = store_with(vec![
            project_in("api", &["Acme"]),
            project_in("web", &["Acme", "Platform"]),
        ]);
        assert_eq!(row_count(&store), 4); // Acme, Platform, web, api

        store.set_folder_collapsed(&["Acme".to_string()], true);
        assert_eq!(row_count(&store), 1);
    }

    #[test]
    fn test_row_index_of_project_is_none_when_hidden() {
        let mut store = store_with(vec![project_in("api", &["Acme"])]);
        let id = store.projects().next().unwrap().id;

        assert_eq!(row_index_of_project(&store, id), Some(1));

        store.set_folder_collapsed(&["Acme".to_string()], true);
        assert_eq!(row_index_of_project(&store, id), None);
    }

    #[test]
    fn test_is_under_folder_matches_prefix_not_substring() {
        let project = project_in("web", &["Acme", "Platform"]);
        assert!(project.is_under_folder(&[]));
        assert!(project.is_under_folder(&["Acme".to_string()]));
        assert!(!project.is_under_folder(&["Platform".to_string()]));
        assert!(!project.is_in_folder(&["Acme".to_string()]));
        assert!(project.is_in_folder(&["Acme".to_string(), "Platform".to_string()]));
    }
}
