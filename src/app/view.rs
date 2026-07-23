//! View enum and navigation helpers
//!
//! Defines the current view being displayed and provides navigation utilities.

use crate::project::{BranchId, ProjectId};

/// Current view being displayed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    /// Grid of projects (main landing page)
    #[default]
    ProjectsOverview,
    /// Branches for a specific project
    ProjectDetail(ProjectId),
    /// Sessions for a specific branch
    BranchDetail(ProjectId, BranchId),
    /// Single session fullscreen view
    SessionView,
    /// Log viewer showing application logs
    LogViewer,
    /// Claude configurations management
    ClaudeConfigs,
    /// Codex configurations management
    CodexConfigs,
}

impl View {
    /// Check if this view is the projects overview
    pub fn is_projects_overview(&self) -> bool {
        matches!(self, View::ProjectsOverview)
    }

    /// Get the parent view for navigation (Esc key)
    pub fn parent(&self) -> Option<View> {
        match self {
            View::ProjectsOverview => None,
            View::ProjectDetail(_) => Some(View::ProjectsOverview),
            View::BranchDetail(project_id, _) => Some(View::ProjectDetail(*project_id)),
            View::SessionView => None, // Handled specially based on context
            View::LogViewer => Some(View::ProjectsOverview),
            View::ClaudeConfigs => Some(View::ProjectsOverview),
            View::CodexConfigs => Some(View::ProjectsOverview),
        }
    }

    /// Get the project ID if this view is associated with a project
    pub fn project_id(&self) -> Option<ProjectId> {
        match self {
            View::ProjectDetail(id) => Some(*id),
            View::BranchDetail(id, _) => Some(*id),
            _ => None,
        }
    }

    /// Get the branch ID if this view is associated with a branch
    pub fn branch_id(&self) -> Option<BranchId> {
        match self {
            View::BranchDetail(_, id) => Some(*id),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_view_default() {
        let view = View::default();
        assert_eq!(view, View::ProjectsOverview);
    }

    #[test]
    fn test_view_is_methods() {
        assert!(View::ProjectsOverview.is_projects_overview());
        assert!(!View::SessionView.is_projects_overview());
    }

    #[test]
    fn test_view_parent() {
        let project_id = uuid::Uuid::new_v4();
        let branch_id = uuid::Uuid::new_v4();

        // ProjectsOverview is root
        assert_eq!(View::ProjectsOverview.parent(), None);

        // ProjectDetail -> ProjectsOverview
        assert_eq!(
            View::ProjectDetail(project_id).parent(),
            Some(View::ProjectsOverview)
        );

        // BranchDetail -> ProjectDetail
        assert_eq!(
            View::BranchDetail(project_id, branch_id).parent(),
            Some(View::ProjectDetail(project_id))
        );

        // SessionView returns None (handled specially)
        assert_eq!(View::SessionView.parent(), None);

        // Other views -> ProjectsOverview
        assert_eq!(View::LogViewer.parent(), Some(View::ProjectsOverview));
        assert_eq!(View::ClaudeConfigs.parent(), Some(View::ProjectsOverview));
    }

    #[test]
    fn test_view_project_id() {
        let project_id = uuid::Uuid::new_v4();
        let branch_id = uuid::Uuid::new_v4();

        assert_eq!(View::ProjectsOverview.project_id(), None);
        assert_eq!(
            View::ProjectDetail(project_id).project_id(),
            Some(project_id)
        );
        assert_eq!(
            View::BranchDetail(project_id, branch_id).project_id(),
            Some(project_id)
        );
        assert_eq!(View::SessionView.project_id(), None);
    }

    #[test]
    fn test_view_branch_id() {
        let project_id = uuid::Uuid::new_v4();
        let branch_id = uuid::Uuid::new_v4();

        assert_eq!(View::ProjectsOverview.branch_id(), None);
        assert_eq!(View::ProjectDetail(project_id).branch_id(), None);
        assert_eq!(
            View::BranchDetail(project_id, branch_id).branch_id(),
            Some(branch_id)
        );
        assert_eq!(View::SessionView.branch_id(), None);
    }
}
