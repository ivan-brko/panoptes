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
    /// All sessions sorted by recent activity
    ActivityTimeline,
    /// Log viewer showing application logs
    LogViewer,
    /// Focus timing statistics view
    FocusStats,
    /// Claude configurations management
    ClaudeConfigs,
}

impl View {
    /// Check if this view is the projects overview
    pub fn is_projects_overview(&self) -> bool {
        matches!(self, View::ProjectsOverview)
    }

    /// Check if this view is a project detail view
    pub fn is_project_detail(&self) -> bool {
        matches!(self, View::ProjectDetail(_))
    }

    /// Check if this view is a branch detail view
    pub fn is_branch_detail(&self) -> bool {
        matches!(self, View::BranchDetail(_, _))
    }

    /// Check if this view is the session view
    pub fn is_session_view(&self) -> bool {
        matches!(self, View::SessionView)
    }

    /// Check if this view is the activity timeline
    pub fn is_activity_timeline(&self) -> bool {
        matches!(self, View::ActivityTimeline)
    }

    /// Check if this view is the focus stats view
    pub fn is_focus_stats(&self) -> bool {
        matches!(self, View::FocusStats)
    }

    /// Check if this view is the Claude configs view
    pub fn is_claude_configs(&self) -> bool {
        matches!(self, View::ClaudeConfigs)
    }

    /// Get the parent view for navigation (Esc key)
    pub fn parent(&self) -> Option<View> {
        match self {
            View::ProjectsOverview => None,
            View::ProjectDetail(_) => Some(View::ProjectsOverview),
            View::BranchDetail(project_id, _) => Some(View::ProjectDetail(*project_id)),
            View::SessionView => None, // Handled specially based on context
            View::ActivityTimeline => Some(View::ProjectsOverview),
            View::LogViewer => Some(View::ProjectsOverview),
            View::FocusStats => Some(View::ProjectsOverview),
            View::ClaudeConfigs => Some(View::ProjectsOverview),
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
