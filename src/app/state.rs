//! Application state management
//!
//! Contains the main AppState struct and navigation helpers.

use std::path::PathBuf;
use std::time::Instant;

use crate::claude_config::{ClaudeConfig, ClaudeConfigId};
use crate::focus_timing::stats::FocusSession;
use crate::focus_timing::tracker::FocusTracker;
use crate::focus_timing::FocusTimer;
use crate::project::{BranchId, ProjectId};
use crate::session::{SessionId, SessionManager};
use crate::tui::{HeaderNotificationManager, NotificationManager};
use crate::wizards::worktree::{BranchRef, WorktreeCreationType};

/// State for Claude settings copy dialog (copies ALL project settings)
#[derive(Debug, Clone)]
pub struct ClaudeSettingsCopyState {
    /// Main repository path (source of settings)
    pub source_path: PathBuf,
    /// New worktree path (destination for settings)
    pub target_path: PathBuf,
    /// Project ID for navigation after dialog
    pub project_id: ProjectId,
    /// Branch ID for navigation after dialog
    pub branch_id: BranchId,
    /// Preview of tools to show user (subset of full settings)
    pub tools_preview: Vec<String>,
    /// Whether MCP servers will be copied (for display)
    pub has_mcp_servers: bool,
    /// Whether Yes is selected (default true)
    pub selected_yes: bool,
    /// Which Claude config directory to use (None = default ~/.claude)
    pub claude_config_dir: Option<PathBuf>,
}

/// State for Claude settings migrate dialog
#[derive(Debug, Clone)]
pub struct ClaudeSettingsMigrateState {
    /// Worktree path being deleted
    pub worktree_path: PathBuf,
    /// Main repository path (destination for migration)
    pub main_path: PathBuf,
    /// Branch ID being deleted
    pub branch_id: BranchId,
    /// Tools unique to worktree that will be migrated
    pub unique_tools: Vec<String>,
    /// Whether Yes is selected (default true)
    pub selected_yes: bool,
    /// Which Claude config directory to use (None = default ~/.claude)
    pub claude_config_dir: Option<PathBuf>,
}

use super::input_mode::InputMode;
use super::view::View;

/// Focus state for the homepage (projects overview)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HomepageFocus {
    /// Projects list is focused
    #[default]
    Projects,
    /// Sessions list is focused
    Sessions,
}

/// Worktree creation wizard state
///
/// Groups all fields related to the multi-step worktree creation wizard.
/// This state is initialized in `start_worktree_wizard()` and cleared in
/// `cancel_worktree_wizard()`.
#[derive(Debug, Clone, Default)]
pub struct WorktreeWizardState {
    /// Search text in WorktreeSelectBranch step
    pub search_text: String,
    /// All available branches (local + remote) for worktree creation
    pub all_branches: Vec<BranchRef>,
    /// Filtered branches matching search text
    pub filtered_branches: Vec<BranchRef>,
    /// Selected index in branch list (0..N = branches, N = "create new" option)
    pub list_index: usize,
    /// Final local branch name (for new branch or from remote)
    pub branch_name: String,
    /// Selected existing/remote branch (for ExistingLocal/RemoteTracking)
    pub source_branch: Option<BranchRef>,
    /// Base branch for creating new branches (step 2)
    pub base_branch: Option<BranchRef>,
    /// Search text in WorktreeSelectBase step
    pub base_search_text: String,
    /// Selected index in base branch list (step 2)
    pub base_list_index: usize,
    /// Type of worktree creation being performed
    pub creation_type: WorktreeCreationType,
    /// Project name for worktree path (cached during wizard)
    pub project_name: String,
    /// Validation error for branch name input (displayed in UI)
    pub branch_validation_error: Option<String>,
}

impl WorktreeWizardState {
    /// Clamp list_index to valid range for filtered_branches
    ///
    /// Call this after updating filtered_branches to ensure index is valid.
    /// Accounts for the "create new" option if search text is not empty.
    pub fn clamp_list_index(&mut self) {
        let filtered_count = self.filtered_branches.len();
        let has_create_option = !self.search_text.is_empty();
        let max_index = if has_create_option {
            filtered_count // "create new" is at index filtered_count
        } else {
            filtered_count.saturating_sub(1)
        };
        self.list_index = self.list_index.min(max_index);
    }

    /// Clamp base_list_index to valid range for the given filtered count
    pub fn clamp_base_list_index(&mut self, filtered_count: usize) {
        if filtered_count == 0 {
            self.base_list_index = 0;
        } else {
            self.base_list_index = self.base_list_index.min(filtered_count - 1);
        }
    }
}

/// Application state
#[derive(Default)]
pub struct AppState {
    /// Current view
    pub view: View,
    /// Current input mode
    pub input_mode: InputMode,
    /// Selected index in ProjectsOverview
    pub selected_project_index: usize,
    /// Selected index in ProjectDetail (branch selection)
    pub selected_branch_index: usize,
    /// Selected index in BranchDetail (session selection)
    pub selected_session_index: usize,
    /// Selected index in ActivityTimeline
    pub selected_timeline_index: usize,
    /// Session being viewed (in session view)
    pub active_session: Option<SessionId>,
    /// Context for returning from session view (which view to go back to)
    pub session_return_view: Option<View>,
    /// Buffer for new session name input
    pub new_session_name: String,
    /// Context: project ID for session being created (None = unassociated)
    pub creating_session_project_id: Option<ProjectId>,
    /// Context: branch ID for session being created (None = unassociated)
    pub creating_session_branch_id: Option<BranchId>,
    /// Context: working directory for session being created
    pub creating_session_working_dir: Option<PathBuf>,
    /// Buffer for new project path input
    pub new_project_path: String,
    /// Path completions for autocomplete
    pub path_completions: Vec<PathBuf>,
    /// Selected index in path completions list
    pub path_completion_index: usize,
    /// Whether to show path completions popup
    pub show_path_completions: bool,
    /// Buffer for new project name input (optional custom name)
    pub new_project_name: String,
    /// Pending project path (validated repo path) for two-step project addition
    pub pending_project_path: PathBuf,
    /// Pending session subdir (computed from user path vs repo root)
    pub pending_session_subdir: Option<PathBuf>,
    /// Pending default branch (computed during path validation)
    pub pending_default_branch: String,
    /// Buffer for new branch name input (worktree creation)
    pub new_branch_name: String,
    /// Selected index in branch selector (0 = "Create new")
    pub branch_selector_index: usize,
    /// Available branch refs (local and remote) for worktree creation
    pub available_branch_refs: Vec<BranchRef>,
    /// Filtered branch refs matching search query
    pub filtered_branch_refs: Vec<BranchRef>,
    /// Selected index in base branch selector
    pub base_branch_selector_index: usize,
    /// The currently selected base branch (independent of filtering)
    pub selected_base_branch: Option<BranchRef>,
    /// Whether git fetch encountered an error (show warning)
    pub fetch_error: Option<String>,
    /// Session pending deletion (for confirmation dialog)
    pub pending_delete_session: Option<SessionId>,
    /// Project pending deletion (for confirmation dialog)
    pub pending_delete_project: Option<ProjectId>,
    /// Branch pending deletion (for confirmation dialog)
    pub pending_delete_branch: Option<BranchId>,
    /// Whether to also delete the git worktree on disk when deleting a branch
    pub delete_worktree_on_disk: bool,
    /// Project being renamed
    pub renaming_project: Option<ProjectId>,
    /// Whether the application should quit
    pub should_quit: bool,
    /// Whether the UI needs to be re-rendered
    pub needs_render: bool,
    /// Count of dropped hook events (for warning display)
    pub dropped_events_count: u64,
    /// Error message to display to the user (cleared on next keypress)
    pub error_message: Option<String>,
    /// Timestamp of last resize event (for debouncing)
    pub last_resize: Option<Instant>,
    /// Whether a resize is pending (debounced)
    pub pending_resize: bool,
    /// Scroll offset in log viewer
    pub log_viewer_scroll: usize,
    /// Whether log viewer auto-scrolls to new entries
    pub log_viewer_auto_scroll: bool,
    /// Scroll offset for session view (0 = live view, >0 = scrolled back)
    pub session_scroll_offset: usize,

    /// Worktree creation wizard state (grouped together)
    pub worktree_wizard: WorktreeWizardState,

    /// Loading message to display during blocking operations
    pub loading_message: Option<String>,
    /// Focus state for homepage (projects vs sessions list)
    pub homepage_focus: HomepageFocus,

    // --- Focus timing state ---
    /// Focus tracker for recording focus intervals
    pub focus_tracker: FocusTracker,
    /// Active focus timer (if any)
    pub focus_timer: Option<FocusTimer>,
    /// Notification manager for displaying alerts (overlay notifications)
    pub notifications: NotificationManager,
    /// Header notification manager for transient header messages
    pub header_notifications: HeaderNotificationManager,
    /// Whether terminal currently has focus
    pub terminal_focused: bool,
    /// Whether focus events are supported by terminal
    pub focus_events_supported: bool,
    /// Input buffer for timer duration entry
    pub focus_timer_input: String,
    /// Selected index in focus stats view
    pub focus_stats_selected_index: usize,
    /// Cached focus sessions for stats view
    pub focus_sessions: Vec<FocusSession>,
    /// Focus session pending deletion (for confirmation dialog)
    pub pending_delete_focus_session: Option<uuid::Uuid>,
    /// Focus session being viewed in detail dialog
    pub viewing_focus_session: Option<FocusSession>,

    // --- Claude config state ---
    /// Selected index in Claude configs view
    pub claude_configs_selected_index: usize,
    /// Claude config pending deletion (for confirmation dialog)
    pub pending_delete_claude_config: Option<ClaudeConfigId>,
    /// Buffer for new config name input
    pub new_claude_config_name: String,
    /// Buffer for new config path input
    pub new_claude_config_path: String,
    /// Claude config being selected for session creation
    pub creating_session_claude_config: Option<ClaudeConfigId>,
    /// Available Claude configs for selection during session creation
    pub available_claude_configs: Vec<ClaudeConfig>,
    /// Selected index in Claude config selector
    pub claude_config_selector_index: usize,
    /// Whether Claude config selector is showing (overlay during session creation)
    pub show_claude_config_selector: bool,
    /// Project ID for setting project default config
    pub setting_project_default_config: Option<ProjectId>,

    // --- Claude settings dialogs ---
    /// Pending Claude settings copy (after worktree creation)
    pub pending_claude_settings_copy: Option<ClaudeSettingsCopyState>,
    /// Pending Claude settings migration (before worktree deletion)
    pub pending_claude_settings_migrate: Option<ClaudeSettingsMigrateState>,
}

impl AppState {
    /// Get the current selected index for the current view
    pub fn current_selected_index(&self) -> usize {
        match self.view {
            View::ProjectsOverview => self.selected_project_index,
            View::ProjectDetail(_) => self.selected_branch_index,
            View::BranchDetail(_, _) => self.selected_session_index,
            View::ActivityTimeline => self.selected_timeline_index,
            View::SessionView => 0,
            View::LogViewer => self.log_viewer_scroll,
            View::FocusStats => self.focus_stats_selected_index,
            View::ClaudeConfigs => self.claude_configs_selected_index,
        }
    }

    /// Set the selected index for the current view
    pub fn set_current_selected_index(&mut self, index: usize) {
        match self.view {
            View::ProjectsOverview => self.selected_project_index = index,
            View::ProjectDetail(_) => self.selected_branch_index = index,
            View::BranchDetail(_, _) => self.selected_session_index = index,
            View::ActivityTimeline => self.selected_timeline_index = index,
            View::SessionView => {}
            View::LogViewer => self.log_viewer_scroll = index,
            View::FocusStats => self.focus_stats_selected_index = index,
            View::ClaudeConfigs => self.claude_configs_selected_index = index,
        }
    }

    /// Select the next item in the current view
    pub fn select_next(&mut self, item_count: usize) {
        if item_count > 0 {
            // Clamp current index to valid range first to handle stale indices
            let current = self
                .current_selected_index()
                .min(item_count.saturating_sub(1));
            let next = (current + 1) % item_count;
            self.set_current_selected_index(next);
        } else {
            // Reset index when collection is empty
            self.set_current_selected_index(0);
        }
    }

    /// Select the previous item in the current view
    pub fn select_prev(&mut self, item_count: usize) {
        if item_count > 0 {
            // Clamp current index to valid range first to handle stale indices
            let current = self
                .current_selected_index()
                .min(item_count.saturating_sub(1));
            let prev = current.checked_sub(1).unwrap_or(item_count - 1);
            self.set_current_selected_index(prev);
        } else {
            // Reset index when collection is empty
            self.set_current_selected_index(0);
        }
    }

    /// Select by number (1-indexed) in the current view
    pub fn select_by_number(&mut self, num: usize, item_count: usize) {
        if num > 0 && num <= item_count {
            self.set_current_selected_index(num - 1);
        }
    }

    /// Navigate to the parent view
    pub fn navigate_back(&mut self) {
        if let Some(parent) = self.view.parent() {
            // If leaving session view, reset session mode state
            if self.view == View::SessionView {
                self.active_session = None;
                self.input_mode = InputMode::Normal;
            }

            // Update focus context based on the parent view
            match parent {
                View::ProjectsOverview
                | View::ActivityTimeline
                | View::LogViewer
                | View::FocusStats
                | View::ClaudeConfigs => {
                    // Leaving project context - clear the context
                    self.focus_tracker.set_context(None, None);
                }
                View::ProjectDetail(project_id) => {
                    // Going back from BranchDetail to ProjectDetail - keep project, clear branch
                    self.focus_tracker.set_context(Some(project_id), None);
                }
                View::BranchDetail(project_id, branch_id) => {
                    // Going back to branch detail - set both
                    self.focus_tracker
                        .set_context(Some(project_id), Some(branch_id));
                }
                View::SessionView => {
                    // Unlikely to navigate back TO session view, but handle it
                }
            }
            self.view = parent;

            // Reset input mode if it's not Normal (handles cases where user was in a text input mode)
            if self.input_mode != InputMode::Normal && self.input_mode != InputMode::Session {
                // Only reset text input modes - keep Session mode if going to SessionView
                if self.view != View::SessionView {
                    self.input_mode = InputMode::Normal;
                }
            }
        }
    }

    /// Navigate to a project detail view
    pub fn navigate_to_project(&mut self, project_id: ProjectId) {
        self.view = View::ProjectDetail(project_id);
        self.selected_branch_index = 0;
        // Update focus tracker context to attribute time to this project
        self.focus_tracker.set_context(Some(project_id), None);
    }

    /// Navigate to a branch detail view
    pub fn navigate_to_branch(&mut self, project_id: ProjectId, branch_id: BranchId) {
        self.view = View::BranchDetail(project_id, branch_id);
        self.selected_session_index = 0;
        // Update focus tracker context to attribute time to this project and branch
        self.focus_tracker
            .set_context(Some(project_id), Some(branch_id));
    }

    /// Navigate to session view (auto-activates session mode)
    pub fn navigate_to_session(&mut self, session_id: SessionId) {
        // Remember where we came from
        self.session_return_view = Some(self.view);
        self.view = View::SessionView;
        self.active_session = Some(session_id);
        // Reset scroll offset when entering session view
        self.session_scroll_offset = 0;
        // Auto-activate session mode so keys go directly to PTY
        self.input_mode = InputMode::Session;
    }

    /// Navigate to activity timeline
    pub fn navigate_to_timeline(&mut self) {
        self.view = View::ActivityTimeline;
        self.selected_timeline_index = 0;
    }

    /// Navigate to Claude configs view
    pub fn navigate_to_claude_configs(&mut self) {
        self.view = View::ClaudeConfigs;
        self.claude_configs_selected_index = 0;
    }

    /// Return from session view based on the current session's context
    ///
    /// Navigates to the branch detail view for the current session's project/branch,
    /// rather than returning to where the user originally came from. This ensures
    /// consistent navigation when jumping between sessions with Space.
    pub fn return_from_session(&mut self, sessions: &SessionManager) {
        // Navigate based on the current session's context (not where we came from)
        if let Some(session_id) = self.active_session {
            if let Some(session) = sessions.get(session_id) {
                // Go to the session's branch detail view
                self.view = View::BranchDetail(session.info.project_id, session.info.branch_id);
            } else {
                // Session was deleted - fall back to stored return view or projects overview
                self.view = self
                    .session_return_view
                    .take()
                    .unwrap_or(View::ProjectsOverview);
            }
        } else {
            // No active session - fall back
            self.view = self
                .session_return_view
                .take()
                .unwrap_or(View::ProjectsOverview);
        }
        self.active_session = None;
        self.input_mode = InputMode::Normal;
        self.session_return_view = None; // Clear it
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // HomepageFocus tests
    #[test]
    fn test_homepage_focus_default() {
        let focus = HomepageFocus::default();
        assert_eq!(focus, HomepageFocus::Projects);
    }

    // WorktreeWizardState tests
    #[test]
    fn test_worktree_wizard_state_default() {
        let state = WorktreeWizardState::default();
        assert!(state.search_text.is_empty());
        assert!(state.all_branches.is_empty());
        assert!(state.filtered_branches.is_empty());
        assert_eq!(state.list_index, 0);
        assert_eq!(state.creation_type, WorktreeCreationType::ExistingLocal);
    }

    #[test]
    fn test_worktree_wizard_clamp_list_index_empty() {
        let mut state = WorktreeWizardState::default();
        state.list_index = 10;
        state.clamp_list_index();
        assert_eq!(state.list_index, 0);
    }

    #[test]
    fn test_worktree_wizard_clamp_list_index_with_branches() {
        use crate::wizards::worktree::{BranchRef, BranchRefType};

        let mut state = WorktreeWizardState::default();
        state.filtered_branches = vec![
            BranchRef::new(BranchRefType::Local, "main".to_string()),
            BranchRef::new(BranchRefType::Local, "develop".to_string()),
        ];
        state.list_index = 10;
        state.clamp_list_index();
        // Without search text, max index is filtered_count - 1 = 1
        assert_eq!(state.list_index, 1);
    }

    #[test]
    fn test_worktree_wizard_clamp_list_index_with_create_option() {
        use crate::wizards::worktree::{BranchRef, BranchRefType};

        let mut state = WorktreeWizardState::default();
        state.search_text = "new-branch".to_string(); // Non-empty enables "create new" option
        state.filtered_branches = vec![BranchRef::new(BranchRefType::Local, "main".to_string())];
        state.list_index = 10;
        state.clamp_list_index();
        // With search text, max index is filtered_count (1) to allow "create new" option
        assert_eq!(state.list_index, 1);
    }

    #[test]
    fn test_worktree_wizard_clamp_base_list_index() {
        let mut state = WorktreeWizardState::default();
        state.base_list_index = 10;
        state.clamp_base_list_index(3);
        assert_eq!(state.base_list_index, 2);

        state.clamp_base_list_index(0);
        assert_eq!(state.base_list_index, 0);
    }

    // AppState navigation tests
    #[test]
    fn test_app_state_default() {
        let state = AppState::default();
        assert_eq!(state.view, View::ProjectsOverview);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert_eq!(state.selected_project_index, 0);
        assert!(!state.should_quit);
    }

    #[test]
    fn test_select_next() {
        let mut state = AppState::default();
        state.view = View::ProjectsOverview;
        state.selected_project_index = 0;

        state.select_next(3);
        assert_eq!(state.selected_project_index, 1);

        state.select_next(3);
        assert_eq!(state.selected_project_index, 2);

        // Wrap around
        state.select_next(3);
        assert_eq!(state.selected_project_index, 0);
    }

    #[test]
    fn test_select_prev() {
        let mut state = AppState::default();
        state.view = View::ProjectsOverview;
        state.selected_project_index = 2;

        state.select_prev(3);
        assert_eq!(state.selected_project_index, 1);

        state.select_prev(3);
        assert_eq!(state.selected_project_index, 0);

        // Wrap around
        state.select_prev(3);
        assert_eq!(state.selected_project_index, 2);
    }

    #[test]
    fn test_select_by_number() {
        let mut state = AppState::default();
        state.view = View::ProjectsOverview;

        state.select_by_number(2, 5);
        assert_eq!(state.selected_project_index, 1); // 1-indexed, so 2 -> index 1

        // Out of range - no change
        state.select_by_number(10, 5);
        assert_eq!(state.selected_project_index, 1);

        // Zero - no change
        state.select_by_number(0, 5);
        assert_eq!(state.selected_project_index, 1);
    }

    #[test]
    fn test_select_with_empty_list() {
        let mut state = AppState::default();
        state.selected_project_index = 5;

        state.select_next(0);
        assert_eq!(state.selected_project_index, 0);

        state.select_prev(0);
        assert_eq!(state.selected_project_index, 0);
    }

    #[test]
    fn test_navigate_to_project() {
        let mut state = AppState::default();
        let project_id = uuid::Uuid::new_v4();

        state.navigate_to_project(project_id);
        assert_eq!(state.view, View::ProjectDetail(project_id));
        assert_eq!(state.selected_branch_index, 0);
    }

    #[test]
    fn test_navigate_to_branch() {
        let mut state = AppState::default();
        let project_id = uuid::Uuid::new_v4();
        let branch_id = uuid::Uuid::new_v4();

        state.navigate_to_branch(project_id, branch_id);
        assert_eq!(state.view, View::BranchDetail(project_id, branch_id));
        assert_eq!(state.selected_session_index, 0);
    }

    #[test]
    fn test_navigate_to_session() {
        let mut state = AppState::default();
        let session_id = uuid::Uuid::new_v4();

        state.view = View::ProjectsOverview;
        state.navigate_to_session(session_id);

        assert_eq!(state.view, View::SessionView);
        assert_eq!(state.active_session, Some(session_id));
        assert_eq!(state.input_mode, InputMode::Session);
        assert_eq!(state.session_return_view, Some(View::ProjectsOverview));
        assert_eq!(state.session_scroll_offset, 0);
    }

    #[test]
    fn test_navigate_to_timeline() {
        let mut state = AppState::default();
        state.navigate_to_timeline();
        assert_eq!(state.view, View::ActivityTimeline);
        assert_eq!(state.selected_timeline_index, 0);
    }

    #[test]
    fn test_navigate_to_claude_configs() {
        let mut state = AppState::default();
        state.navigate_to_claude_configs();
        assert_eq!(state.view, View::ClaudeConfigs);
        assert_eq!(state.claude_configs_selected_index, 0);
    }

    #[test]
    fn test_navigate_back_from_project_detail() {
        let mut state = AppState::default();
        let project_id = uuid::Uuid::new_v4();

        state.view = View::ProjectDetail(project_id);
        state.navigate_back();
        assert_eq!(state.view, View::ProjectsOverview);
    }

    #[test]
    fn test_navigate_back_from_branch_detail() {
        let mut state = AppState::default();
        let project_id = uuid::Uuid::new_v4();
        let branch_id = uuid::Uuid::new_v4();

        state.view = View::BranchDetail(project_id, branch_id);
        state.navigate_back();
        assert_eq!(state.view, View::ProjectDetail(project_id));
    }

    #[test]
    fn test_navigate_back_resets_input_mode() {
        let mut state = AppState::default();
        let project_id = uuid::Uuid::new_v4();

        state.view = View::ProjectDetail(project_id);
        state.input_mode = InputMode::RenamingProject;
        state.navigate_back();

        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_current_selected_index_for_different_views() {
        let mut state = AppState::default();
        let project_id = uuid::Uuid::new_v4();
        let branch_id = uuid::Uuid::new_v4();

        // ProjectsOverview
        state.view = View::ProjectsOverview;
        state.selected_project_index = 2;
        assert_eq!(state.current_selected_index(), 2);

        // ProjectDetail
        state.view = View::ProjectDetail(project_id);
        state.selected_branch_index = 3;
        assert_eq!(state.current_selected_index(), 3);

        // BranchDetail
        state.view = View::BranchDetail(project_id, branch_id);
        state.selected_session_index = 1;
        assert_eq!(state.current_selected_index(), 1);

        // ActivityTimeline
        state.view = View::ActivityTimeline;
        state.selected_timeline_index = 4;
        assert_eq!(state.current_selected_index(), 4);
    }
}
