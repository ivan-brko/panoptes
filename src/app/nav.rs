//! Navigation model for the three-pane layout
//!
//! The application shows three panes at once - Projects, Sessions, Settings -
//! and one of them holds focus. A session is the only thing that takes over
//! the whole terminal, which is what [`Focus::Session`] means.
//!
//! Each pane keeps its own drill-down state ([`ProjectsNav`], [`SettingsNav`]),
//! so opening a project in pane 1 leaves panes 2 and 3 exactly as they were.

use crate::project::{BranchId, ProjectId};

/// One of the three always-visible panes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Tab {
    /// Pane 1: the project tree and its drill-downs
    #[default]
    Projects,
    /// Pane 2: every session, flat and sorted
    Sessions,
    /// Pane 3: settings sections
    Settings,
}

impl Tab {
    /// The three panes in `Tab`/`Shift+Tab` order
    pub const ALL: [Tab; 3] = [Tab::Projects, Tab::Sessions, Tab::Settings];

    /// Position of this pane on screen, left to right
    pub fn index(self) -> usize {
        match self {
            Tab::Projects => 0,
            Tab::Sessions => 1,
            Tab::Settings => 2,
        }
    }

    /// The pane after this one, wrapping around
    pub fn next(self) -> Tab {
        Tab::ALL[(self.index() + 1) % Tab::ALL.len()]
    }

    /// The pane before this one, wrapping around
    pub fn prev(self) -> Tab {
        Tab::ALL[(self.index() + Tab::ALL.len() - 1) % Tab::ALL.len()]
    }

    /// Pane title, used as the first breadcrumb segment of its block
    pub fn title(self) -> &'static str {
        match self {
            Tab::Projects => "Projects",
            Tab::Sessions => "Sessions",
            Tab::Settings => "Settings",
        }
    }
}

/// What currently owns the screen and the keyboard
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The three panes are visible; `Tab` cycles between them
    Panes(Tab),
    /// A single session fills the terminal
    Session,
}

impl Focus {
    /// The focused pane, or `None` while a session fills the screen
    pub fn tab(self) -> Option<Tab> {
        match self {
            Focus::Panes(tab) => Some(tab),
            Focus::Session => None,
        }
    }

    /// Whether the three-pane layout is what is on screen
    pub fn is_panes(self) -> bool {
        matches!(self, Focus::Panes(_))
    }
}

// `#[default]` only accepts unit variants, and the default focus carries the
// pane it starts on, so this is written out.
impl Default for Focus {
    fn default() -> Self {
        Focus::Panes(Tab::Projects)
    }
}

/// Drill-down level of pane 1
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProjectsNav {
    /// The folder tree of projects
    #[default]
    Overview,
    /// Branches of one project
    Project(ProjectId),
    /// Sessions of one branch
    Branch(ProjectId, BranchId),
    /// Per-project settings, opened with `,`
    ProjectSettings(ProjectId),
}

impl ProjectsNav {
    /// The level `Esc` goes back to, or `None` at the root
    pub fn parent(self) -> Option<ProjectsNav> {
        match self {
            ProjectsNav::Overview => None,
            ProjectsNav::Project(_) => Some(ProjectsNav::Overview),
            ProjectsNav::Branch(project_id, _) => Some(ProjectsNav::Project(project_id)),
            ProjectsNav::ProjectSettings(project_id) => Some(ProjectsNav::Project(project_id)),
        }
    }

    /// The project this level belongs to, if any
    pub fn project_id(self) -> Option<ProjectId> {
        match self {
            ProjectsNav::Overview => None,
            ProjectsNav::Project(id)
            | ProjectsNav::Branch(id, _)
            | ProjectsNav::ProjectSettings(id) => Some(id),
        }
    }

    /// The branch this level belongs to, if any
    pub fn branch_id(self) -> Option<BranchId> {
        match self {
            ProjectsNav::Branch(_, id) => Some(id),
            _ => None,
        }
    }
}

/// Row 0 of every nested level of pane 1: the way back out
///
/// The file-manager `..` convention - a selectable row whose `Enter` is the
/// same action as `Esc`. It shifts every list selection in the pane by one, so
/// a selection index there is a *row*, and [`item_row`]/[`row_item`] convert
/// between a row and the list item under it.
pub const BACK_ROW: usize = 0;

/// The row a nested level lands on when it is drilled into
///
/// Not [`BACK_ROW`]: drilling in and immediately pressing `Enter` must not
/// bounce straight back out. [`clamp_row`] moves it onto the back row when the
/// level turns out to have no items at all.
pub const FIRST_ITEM_ROW: usize = 1;

/// The row that shows list item `item`
pub fn item_row(item: usize) -> usize {
    item + 1
}

/// The list item `row` points at, or `None` on the back row
pub fn row_item(row: usize) -> Option<usize> {
    row.checked_sub(1)
}

/// Fit a row selection to a list of `item_count` items
///
/// An empty level has only the back row, and deleting the last item leaves the
/// selection past the end; both land on the nearest row that still exists.
pub fn clamp_row(row: usize, item_count: usize) -> usize {
    row.min(item_count)
}

/// Number of selectable rows at a nested level holding `item_count` items
pub fn rows_with_back(item_count: usize) -> usize {
    item_count + 1
}

/// Drill-down level of pane 3
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsNav {
    /// The list of sections
    #[default]
    Sections,
    /// Claude Code account configs
    ClaudeConfigs,
    /// Codex account configs
    CodexConfigs,
    /// Custom shell shortcuts
    Shortcuts,
    /// Live notification toggles
    Notifications,
    /// Version, hook health, and where the files live
    About,
}

impl SettingsNav {
    /// The sections in list order (everything except [`SettingsNav::Sections`])
    pub const SECTIONS: [SettingsNav; 5] = [
        SettingsNav::ClaudeConfigs,
        SettingsNav::CodexConfigs,
        SettingsNav::Shortcuts,
        SettingsNav::Notifications,
        SettingsNav::About,
    ];

    /// Row label in the sections list
    pub fn title(self) -> &'static str {
        match self {
            SettingsNav::Sections => "Settings",
            SettingsNav::ClaudeConfigs => "Claude configs",
            SettingsNav::CodexConfigs => "Codex configs",
            SettingsNav::Shortcuts => "Shortcuts",
            SettingsNav::Notifications => "Notifications",
            SettingsNav::About => "About / paths",
        }
    }

    /// One-line description of the section, shown in the global footer
    pub fn description(self) -> &'static str {
        match self {
            SettingsNav::Sections => "Enter: open section",
            SettingsNav::ClaudeConfigs => "Claude Code accounts (CLAUDE_CONFIG_DIR)",
            SettingsNav::CodexConfigs => "Codex accounts (CODEX_HOME)",
            SettingsNav::Shortcuts => "Custom keys that launch a shell command",
            SettingsNav::Notifications => "What interrupts you, and how",
            SettingsNav::About => "Version, hook server, and where the files live",
        }
    }

    /// The section at `index` in the sections list
    pub fn at(index: usize) -> Option<SettingsNav> {
        SettingsNav::SECTIONS.get(index).copied()
    }

    /// The level `Esc` goes back to, or `None` at the root
    pub fn parent(self) -> Option<SettingsNav> {
        match self {
            SettingsNav::Sections => None,
            _ => Some(SettingsNav::Sections),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_focus_defaults_to_the_projects_pane() {
        assert_eq!(Focus::default(), Focus::Panes(Tab::Projects));
        assert_eq!(Focus::default().tab(), Some(Tab::Projects));
        assert!(Focus::Session.tab().is_none());
    }

    #[test]
    fn test_tab_cycles_with_wraparound() {
        assert_eq!(Tab::Projects.next(), Tab::Sessions);
        assert_eq!(Tab::Sessions.next(), Tab::Settings);
        assert_eq!(Tab::Settings.next(), Tab::Projects);

        assert_eq!(Tab::Projects.prev(), Tab::Settings);
        assert_eq!(Tab::Settings.prev(), Tab::Sessions);
        assert_eq!(Tab::Sessions.prev(), Tab::Projects);
    }

    #[test]
    fn test_tab_index_matches_screen_order() {
        for (i, tab) in Tab::ALL.iter().enumerate() {
            assert_eq!(tab.index(), i);
        }
    }

    #[test]
    fn test_projects_nav_parents() {
        let project = uuid::Uuid::new_v4();
        let branch = uuid::Uuid::new_v4();

        assert_eq!(ProjectsNav::Overview.parent(), None);
        assert_eq!(
            ProjectsNav::Project(project).parent(),
            Some(ProjectsNav::Overview)
        );
        assert_eq!(
            ProjectsNav::Branch(project, branch).parent(),
            Some(ProjectsNav::Project(project))
        );
        // Project settings is a sibling of the branch list, not a deeper level
        assert_eq!(
            ProjectsNav::ProjectSettings(project).parent(),
            Some(ProjectsNav::Project(project))
        );
    }

    #[test]
    fn test_projects_nav_ids() {
        let project = uuid::Uuid::new_v4();
        let branch = uuid::Uuid::new_v4();

        assert_eq!(ProjectsNav::Overview.project_id(), None);
        assert_eq!(ProjectsNav::Project(project).project_id(), Some(project));
        assert_eq!(
            ProjectsNav::ProjectSettings(project).project_id(),
            Some(project)
        );
        assert_eq!(
            ProjectsNav::Branch(project, branch).branch_id(),
            Some(branch)
        );
        assert_eq!(ProjectsNav::Project(project).branch_id(), None);
    }

    /// The back row occupies row 0, so a row and the item under it are one
    /// apart in every direction
    #[test]
    fn test_rows_and_items_round_trip_across_the_back_row() {
        assert_eq!(row_item(BACK_ROW), None);
        assert_eq!(row_item(FIRST_ITEM_ROW), Some(0));
        assert_eq!(item_row(0), FIRST_ITEM_ROW);
        for item in 0..5 {
            assert_eq!(row_item(item_row(item)), Some(item));
        }
        // A three-item level has four rows: the back row and the three items
        assert_eq!(rows_with_back(3), 4);
        assert_eq!(rows_with_back(0), 1);
    }

    /// Drilling into an empty level lands on the only row it has
    #[test]
    fn test_clamp_row_falls_back_to_the_back_row_when_there_is_nothing_else() {
        assert_eq!(clamp_row(FIRST_ITEM_ROW, 0), BACK_ROW);
        // The last item survives a list that shrank under the selection
        assert_eq!(clamp_row(9, 3), 3);
        // A row already in range is left alone
        assert_eq!(clamp_row(2, 3), 2);
        assert_eq!(clamp_row(BACK_ROW, 3), BACK_ROW);
    }

    #[test]
    fn test_settings_sections_list_is_ordered_and_addressable() {
        assert_eq!(SettingsNav::at(0), Some(SettingsNav::ClaudeConfigs));
        assert_eq!(SettingsNav::at(4), Some(SettingsNav::About));
        assert_eq!(SettingsNav::at(5), None);
        assert!(!SettingsNav::SECTIONS.contains(&SettingsNav::Sections));

        for section in SettingsNav::SECTIONS {
            assert_eq!(section.parent(), Some(SettingsNav::Sections));
            assert!(!section.description().is_empty());
        }
        assert_eq!(SettingsNav::Sections.parent(), None);
    }
}
