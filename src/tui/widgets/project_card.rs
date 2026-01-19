//! Project card widget
//!
//! A card component for displaying project information in a grid layout.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::project::Project;

/// A card widget for displaying a project in the projects overview grid
pub struct ProjectCard<'a> {
    project: &'a Project,
    branch_count: usize,
    active_session_count: usize,
    is_selected: bool,
}

impl<'a> ProjectCard<'a> {
    /// Create a new project card
    pub fn new(project: &'a Project) -> Self {
        Self {
            project,
            branch_count: 0,
            active_session_count: 0,
            is_selected: false,
        }
    }

    /// Set the number of branches
    pub fn branch_count(mut self, count: usize) -> Self {
        self.branch_count = count;
        self
    }

    /// Set the number of active sessions
    pub fn active_sessions(mut self, count: usize) -> Self {
        self.active_session_count = count;
        self
    }

    /// Set whether this card is selected
    pub fn selected(mut self, selected: bool) -> Self {
        self.is_selected = selected;
        self
    }
}

impl Widget for ProjectCard<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.is_selected {
            Style::default().fg(Color::Cyan).bold()
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(self.project.name.as_str());

        let inner = block.inner(area);
        block.render(area, buf);

        // Content lines
        let branches_text = format!("{} branches", self.branch_count);
        let sessions_text = if self.active_session_count > 0 {
            format!("{} active", self.active_session_count)
        } else {
            "no active sessions".to_string()
        };

        let content = format!("{}\n{}", branches_text, sessions_text);
        let text_style = if self.is_selected {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Gray)
        };

        Paragraph::new(content).style(text_style).render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::path::PathBuf;

    fn test_project() -> Project {
        Project {
            id: uuid::Uuid::new_v4(),
            name: "test-project".to_string(),
            repo_path: PathBuf::from("/tmp/test"),
            remote_url: None,
            default_branch: "main".to_string(),
            created_at: Utc::now(),
            last_activity: Utc::now(),
        }
    }

    #[test]
    fn test_project_card_creation() {
        let project = test_project();
        let card = ProjectCard::new(&project)
            .branch_count(3)
            .active_sessions(2)
            .selected(true);

        assert_eq!(card.branch_count, 3);
        assert_eq!(card.active_session_count, 2);
        assert!(card.is_selected);
    }
}
