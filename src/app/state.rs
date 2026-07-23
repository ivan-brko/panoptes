//! Application state management
//!
//! Contains the main AppState struct and navigation helpers.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::claude_config::ClaudeConfig;
use crate::project::{BranchId, ProjectId};
use crate::session::{SessionId, SessionManager};
use crate::tui::HeaderNotificationManager;
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
    /// Whether local settings.local.json exists and will be copied
    pub has_local_settings: bool,
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
    /// Tools unique to worktree that will be migrated (legacy format)
    pub unique_tools: Vec<String>,
    /// Whether Yes is selected (default true)
    pub selected_yes: bool,
    /// Which Claude config directory to use (None = default ~/.claude)
    pub claude_config_dir: Option<PathBuf>,
    /// Whether worktree has unique local settings to migrate (modern format)
    pub has_local_settings: bool,
}

use super::input_mode::InputMode;
use super::nav::{Focus, ProjectsNav, SettingsNav, Tab};

/// Advance a wrap-around list selection, tolerating stale indices
///
/// The index is clamped into `0..count` before stepping, so a selection left
/// pointing past the end of a list that shrank wraps predictably instead of
/// jumping. Returns 0 when the list is empty.
pub fn cycle_next(index: usize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    (index.min(count - 1) + 1) % count
}

/// Step a wrap-around list selection backwards, tolerating stale indices
///
/// See [`cycle_next`] for the stale-index handling. Returns 0 when the list
/// is empty.
pub fn cycle_prev(index: usize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    index.min(count - 1).checked_sub(1).unwrap_or(count - 1)
}

/// Draft state for an agent config being created (name step, then path step)
///
/// Claude and Codex configs are created through the same two-step dialog and
/// never at the same time, so one draft serves both flows; the current
/// [`InputMode`] carries which agent it is for.
#[derive(Debug, Clone, Default)]
pub struct ConfigDraft {
    /// Config name being typed (step 1)
    pub name: String,
    /// Config directory path being typed (step 2; empty = agent default dir)
    pub path: String,
}

impl ConfigDraft {
    /// Clear the draft (config creation cancelled or completed)
    pub fn reset(&mut self) {
        self.name.clear();
        self.path.clear();
    }
}

/// Draft state for a session being created
///
/// Filled in by the view that starts session creation (name typed by the
/// user, project/branch/working-dir context from the selected branch), then
/// consumed by the create flow via [`SessionDraft::take`].
#[derive(Debug, Clone, Default)]
pub struct SessionDraft {
    /// Session name being typed (empty = auto-generate one)
    pub name: String,
    /// Project the session belongs to (None = unassociated)
    pub project_id: Option<ProjectId>,
    /// Branch the session belongs to (None = unassociated)
    pub branch_id: Option<BranchId>,
    /// Directory the session starts in (None = current directory)
    pub working_dir: Option<PathBuf>,
}

impl SessionDraft {
    /// Start a draft for a session under the given project/branch
    pub fn for_branch(project_id: ProjectId, branch_id: BranchId, working_dir: PathBuf) -> Self {
        Self {
            name: String::new(),
            project_id: Some(project_id),
            branch_id: Some(branch_id),
            working_dir: Some(working_dir),
        }
    }

    /// Clear the draft (session creation cancelled)
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Consume the draft, leaving an empty one behind
    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }
}

/// What a pending folder move applies to
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderMoveTarget {
    /// A single project is being filed
    Project(ProjectId),
    /// A folder and everything under it is being re-parented
    Folder(Vec<String>),
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
    /// Base branches matching `base_search_text` (mirrors `filtered_branches`
    /// for step 1); kept in step so handlers and render share one filter pass
    pub filtered_base_branches: Vec<BranchRef>,
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

/// Frames of the loading spinner, cycled while an operation is in flight
pub const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// How long each spinner frame is shown
const SPINNER_FRAME_INTERVAL: Duration = Duration::from_millis(80);

/// The "Working" overlay shown while an operation is in flight
///
/// Background jobs advance [`frame`](Self::frame) from the event loop so the
/// spinner animates; operations that still block the loop simply render it
/// once, with a static frame.
#[derive(Debug, Clone)]
pub struct LoadingOverlay {
    /// What the operation is doing, e.g. "Fetching branches from remotes..."
    pub message: String,
    /// When the overlay appeared (drives the spinner)
    pub started_at: Instant,
    /// Whether Esc aborts the operation (also shown as a hint)
    pub cancellable: bool,
    /// Set once the user asked to cancel, until the job actually ends
    pub cancelling: bool,
    /// Index into [`SPINNER_FRAMES`]
    pub frame: usize,
}

impl LoadingOverlay {
    /// A new overlay for `message`, starting at the first spinner frame
    pub fn new(message: impl Into<String>, cancellable: bool) -> Self {
        Self {
            message: message.into(),
            started_at: Instant::now(),
            cancellable,
            cancelling: false,
            frame: 0,
        }
    }

    /// The spinner character for the current frame
    pub fn spinner(&self) -> &'static str {
        SPINNER_FRAMES[self.frame % SPINNER_FRAMES.len()]
    }

    /// Advance the spinner to the frame `now` calls for
    ///
    /// Returns whether the frame changed, i.e. whether a re-render is worth it.
    pub fn tick(&mut self, now: Instant) -> bool {
        let frame = (now.saturating_duration_since(self.started_at).as_millis()
            / SPINNER_FRAME_INTERVAL.as_millis()) as usize
            % SPINNER_FRAMES.len();
        let changed = frame != self.frame;
        self.frame = frame;
        changed
    }
}

/// Application state
#[derive(Default)]
pub struct AppState {
    /// What owns the screen: one of the three panes, or a full-screen session
    pub focus: Focus,
    /// Drill-down level of pane 1
    pub projects_nav: ProjectsNav,
    /// Drill-down level of pane 3
    pub settings_nav: SettingsNav,
    /// Current input mode
    pub input_mode: InputMode,
    /// Selected index in the project tree, counted over *visible tree rows*
    /// (folder headings included), not over projects
    pub selected_project_index: usize,
    /// Selected index in pane 1's branch list
    pub selected_branch_index: usize,
    /// Selected index in pane 1's per-branch session list
    ///
    /// Separate from [`Self::sessions_pane_index`]: pane 1's branch drill-down
    /// and pane 2's flat list are on screen at the same time, so one index
    /// cannot serve both.
    pub branch_session_index: usize,
    /// Selected index in pane 2's flat session list
    pub sessions_pane_index: usize,
    /// Selected row in pane 3's sections list
    pub settings_section_index: usize,
    /// Selected row in the per-project settings list (pane 1, opened with `,`)
    pub project_settings_index: usize,
    /// Selected row in pane 3's notifications list
    pub notifications_index: usize,
    /// Which session the session view is on, for digit jumps
    ///
    /// Indexes `SessionManager::session_order`, not any on-screen list.
    pub session_cycle_index: usize,
    /// Session being viewed (in session view)
    pub active_session: Option<SessionId>,
    /// Pane the session view was opened from, restored when it is left
    pub session_return_focus: Option<Focus>,
    /// Draft for the session being created (name plus project/branch context)
    pub session_draft: SessionDraft,
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

    // --- Project folder organization ---
    /// Buffer for folder path input (move and rename dialogs)
    pub folder_input: String,
    /// Existing folder paths matching the current input, for autocomplete
    pub folder_completions: Vec<String>,
    /// Selected index in the folder completions list
    pub folder_completion_index: usize,
    /// Whether the folder completions list is showing
    pub show_folder_completions: bool,
    /// What the pending folder move applies to
    pub moving_to_folder: Option<FolderMoveTarget>,
    /// Folder being renamed
    pub renaming_folder: Option<Vec<String>>,
    /// Folder pending removal (for confirmation dialog)
    pub pending_remove_folder: Option<Vec<String>>,
    /// Validation error for the folder dialogs (displayed inline)
    pub folder_error: Option<String>,
    /// Whether the application should quit
    pub should_quit: bool,
    /// Whether the UI needs to be re-rendered
    pub needs_render: bool,
    /// Count of dropped hook events (for warning display)
    pub dropped_events_count: u64,
    /// Error message to display to the user (cleared on next keypress)
    pub error_message: Option<String>,
    /// Startup notice (e.g. corrupt-file backups) shown as a persistent,
    /// dismissable overlay until the user presses a key
    pub startup_notice: Option<String>,
    /// Timestamp of last resize event (for debouncing)
    pub last_resize: Option<Instant>,
    /// Whether a resize is pending (debounced)
    pub pending_resize: bool,
    /// Scroll offset for session view (0 = live view, >0 = scrolled back)
    pub session_scroll_offset: usize,

    /// Worktree creation wizard state (grouped together)
    pub worktree_wizard: WorktreeWizardState,

    /// Loading overlay shown while an operation is in flight
    pub loading: Option<LoadingOverlay>,

    /// Header notification manager for transient header messages
    pub header_notifications: HeaderNotificationManager,

    // --- Agent config state (shared between Claude and Codex flows) ---
    /// Draft for the config being created (name + path steps, either agent)
    pub config_draft: ConfigDraft,
    /// Config pending deletion; the confirming [`InputMode`] says which agent
    pub pending_delete_agent_config: Option<uuid::Uuid>,
    /// Selected index in the config selector (either agent)
    pub config_selector_index: usize,
    /// Project ID for setting project default config
    pub setting_project_default_config: Option<ProjectId>,

    // --- Claude config state ---
    /// Selected index in Claude configs view
    pub claude_configs_selected_index: usize,
    /// Available Claude configs for selection during session creation
    pub available_claude_configs: Vec<ClaudeConfig>,

    // --- Claude settings dialogs ---
    /// Pending Claude settings copy (after worktree creation)
    pub pending_claude_settings_copy: Option<ClaudeSettingsCopyState>,
    /// Pending Claude settings migration (before worktree deletion)
    pub pending_claude_settings_migrate: Option<ClaudeSettingsMigrateState>,

    // --- Codex config state ---
    /// Selected index in Codex configs view
    pub codex_configs_selected_index: usize,
    /// Available Codex configs for selection during session creation
    pub available_codex_configs: Vec<crate::codex_config::CodexConfig>,
    /// Agent type selector index (0 = Claude Code, 1 = Codex)
    pub agent_type_selector_index: usize,

    // --- Help overlay ---
    /// Whether to show the help overlay with keyboard shortcuts
    pub show_help_overlay: bool,

    // --- Custom shortcuts dialog state ---
    /// Selected index in the custom shortcuts list
    pub custom_shortcuts_selected: usize,
    /// Key being added for new shortcut
    pub new_shortcut_key: Option<char>,
    /// Name being entered for new shortcut
    pub new_shortcut_name: String,
    /// Command being entered for new shortcut
    pub new_shortcut_command: String,
    /// Index of shortcut pending deletion
    pub pending_delete_shortcut_index: Option<usize>,
    /// Validation error message for shortcut creation
    pub shortcut_error: Option<String>,
    /// Auto-close toggle for new shortcut being added
    pub new_shortcut_auto_close: bool,
}

impl AppState {
    /// The pane that currently has focus, or `None` inside a session
    pub fn focused_tab(&self) -> Option<Tab> {
        self.focus.tab()
    }

    /// Whether pane `tab` currently has focus
    pub fn is_focused(&self, tab: Tab) -> bool {
        self.focus == Focus::Panes(tab)
    }

    /// Move focus to the next or previous pane, wrapping around
    ///
    /// A no-op while a session fills the screen: there are no panes to cycle.
    pub fn cycle_pane(&mut self, forward: bool) -> bool {
        let Focus::Panes(tab) = self.focus else {
            return false;
        };
        self.focus = Focus::Panes(if forward { tab.next() } else { tab.prev() });
        true
    }

    /// Go back one level in the focused pane
    ///
    /// At the root of a pane this does nothing at all - `Esc` never quits and
    /// never changes which pane is focused. Returns whether anything moved.
    pub fn navigate_back(&mut self) -> bool {
        match self.focus {
            Focus::Panes(Tab::Projects) => match self.projects_nav.parent() {
                Some(parent) => {
                    self.projects_nav = parent;
                    true
                }
                None => false,
            },
            Focus::Panes(Tab::Settings) => match self.settings_nav.parent() {
                Some(parent) => {
                    self.settings_nav = parent;
                    true
                }
                None => false,
            },
            // Pane 2 is a flat list with nothing to pop, and the session view
            // has its own two-step Esc in `handle_session_view_normal_key`
            Focus::Panes(Tab::Sessions) | Focus::Session => false,
        }
    }

    /// Drill pane 1 into a project's branch list
    pub fn navigate_to_project(&mut self, project_id: ProjectId) {
        self.projects_nav = ProjectsNav::Project(project_id);
        self.selected_branch_index = 0;
    }

    /// Drill pane 1 into a branch's session list
    pub fn navigate_to_branch(&mut self, project_id: ProjectId, branch_id: BranchId) {
        self.projects_nav = ProjectsNav::Branch(project_id, branch_id);
        self.branch_session_index = 0;
    }

    /// Open the per-project settings level of pane 1
    pub fn navigate_to_project_settings(&mut self, project_id: ProjectId) {
        self.projects_nav = ProjectsNav::ProjectSettings(project_id);
        self.project_settings_index = 0;
    }

    /// Open a session full-screen (auto-activates session mode)
    ///
    /// Records the pane it was opened from so `Esc` can put it back, including
    /// after a `Space` attention-jump from a pane the session has nothing to do
    /// with.
    pub fn navigate_to_session(&mut self, session_id: SessionId) {
        // Only remember a pane; jumping between sessions must not overwrite
        // the pane the first jump came from with "the session view"
        if let Focus::Panes(_) = self.focus {
            self.session_return_focus = Some(self.focus);
        }
        self.focus = Focus::Session;
        self.active_session = Some(session_id);
        // Reset scroll offset when entering session view
        self.session_scroll_offset = 0;
        // Auto-activate session mode so keys go directly to PTY
        self.input_mode = InputMode::Session;
    }

    /// Leave the session view, restoring the pane it was opened from
    ///
    /// When that pane was pane 1, its drill-down follows the session to its own
    /// branch, so `Esc` lands on the list the session belongs to rather than
    /// wherever pane 1 happened to be.
    pub fn return_from_session(&mut self, sessions: &SessionManager) {
        let restored = self.session_return_focus.take().unwrap_or_default();

        if restored == Focus::Panes(Tab::Projects) {
            if let Some(session) = self.active_session.and_then(|id| sessions.get(id)) {
                self.navigate_to_branch(session.info.project_id, session.info.branch_id);
            }
        }

        self.focus = restored;
        self.active_session = None;
        self.input_mode = InputMode::Normal;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cycle helper tests
    #[test]
    fn test_cycle_next_wraps() {
        assert_eq!(cycle_next(0, 3), 1);
        assert_eq!(cycle_next(1, 3), 2);
        assert_eq!(cycle_next(2, 3), 0);
    }

    #[test]
    fn test_cycle_prev_wraps() {
        assert_eq!(cycle_prev(2, 3), 1);
        assert_eq!(cycle_prev(1, 3), 0);
        assert_eq!(cycle_prev(0, 3), 2);
    }

    #[test]
    fn test_cycle_empty_list() {
        assert_eq!(cycle_next(5, 0), 0);
        assert_eq!(cycle_prev(5, 0), 0);
    }

    #[test]
    fn test_cycle_stale_index_is_clamped_first() {
        // Index 10 in a 3-item list: clamp to 2, then step
        assert_eq!(cycle_next(10, 3), 0);
        assert_eq!(cycle_prev(10, 3), 1);
        // Single-item list always lands on 0
        assert_eq!(cycle_next(7, 1), 0);
        assert_eq!(cycle_prev(7, 1), 0);
    }

    // SessionDraft tests
    #[test]
    fn test_session_draft_reset_and_take() {
        let mut draft = SessionDraft::for_branch(
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            PathBuf::from("/tmp"),
        );
        draft.name = "my session".to_string();

        let taken = draft.take();
        assert_eq!(taken.name, "my session");
        assert!(taken.project_id.is_some());
        assert!(draft.name.is_empty());
        assert!(draft.project_id.is_none());

        let mut draft2 = SessionDraft::for_branch(
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            PathBuf::from("/tmp"),
        );
        draft2.reset();
        assert!(draft2.working_dir.is_none());
        assert!(draft2.branch_id.is_none());
    }

    // Pane focus tests
    #[test]
    fn test_cycle_pane_wraps_in_both_directions() {
        let mut state = AppState::default();
        assert!(state.is_focused(Tab::Projects));

        assert!(state.cycle_pane(true));
        assert!(state.is_focused(Tab::Sessions));
        state.cycle_pane(true);
        assert!(state.is_focused(Tab::Settings));
        state.cycle_pane(true);
        assert!(state.is_focused(Tab::Projects));

        state.cycle_pane(false);
        assert!(state.is_focused(Tab::Settings));
    }

    #[test]
    fn test_cycle_pane_does_nothing_inside_a_session() {
        let mut state = AppState {
            focus: Focus::Session,
            ..Default::default()
        };
        assert!(!state.cycle_pane(true));
        assert_eq!(state.focus, Focus::Session);
        assert!(state.focused_tab().is_none());
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
        let mut state = WorktreeWizardState {
            list_index: 10,
            ..Default::default()
        };
        state.clamp_list_index();
        assert_eq!(state.list_index, 0);
    }

    #[test]
    fn test_worktree_wizard_clamp_list_index_with_branches() {
        use crate::wizards::worktree::{BranchRef, BranchRefType};

        let mut state = WorktreeWizardState {
            filtered_branches: vec![
                BranchRef::new(BranchRefType::Local, "main".to_string()),
                BranchRef::new(BranchRefType::Local, "develop".to_string()),
            ],
            list_index: 10,
            ..Default::default()
        };
        state.clamp_list_index();
        // Without search text, max index is filtered_count - 1 = 1
        assert_eq!(state.list_index, 1);
    }

    #[test]
    fn test_worktree_wizard_clamp_list_index_with_create_option() {
        use crate::wizards::worktree::{BranchRef, BranchRefType};

        let mut state = WorktreeWizardState {
            search_text: "new-branch".to_string(), // Non-empty enables "create new" option
            filtered_branches: vec![BranchRef::new(BranchRefType::Local, "main".to_string())],
            list_index: 10,
            ..Default::default()
        };
        state.clamp_list_index();
        // With search text, max index is filtered_count (1) to allow "create new" option
        assert_eq!(state.list_index, 1);
    }

    #[test]
    fn test_worktree_wizard_clamp_base_list_index() {
        let mut state = WorktreeWizardState {
            base_list_index: 10,
            ..Default::default()
        };
        state.clamp_base_list_index(3);
        assert_eq!(state.base_list_index, 2);

        state.clamp_base_list_index(0);
        assert_eq!(state.base_list_index, 0);
    }

    // AppState navigation tests
    #[test]
    fn test_app_state_default() {
        let state = AppState::default();
        assert_eq!(state.focus, Focus::Panes(Tab::Projects));
        assert_eq!(state.projects_nav, ProjectsNav::Overview);
        assert_eq!(state.settings_nav, SettingsNav::Sections);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert_eq!(state.selected_project_index, 0);
        assert!(!state.should_quit);
    }

    #[test]
    fn test_navigate_to_project() {
        let mut state = AppState::default();
        let project_id = uuid::Uuid::new_v4();

        state.selected_branch_index = 4;
        state.navigate_to_project(project_id);
        assert_eq!(state.projects_nav, ProjectsNav::Project(project_id));
        assert_eq!(state.selected_branch_index, 0);
    }

    #[test]
    fn test_navigate_to_branch() {
        let mut state = AppState::default();
        let project_id = uuid::Uuid::new_v4();
        let branch_id = uuid::Uuid::new_v4();

        state.branch_session_index = 3;
        state.navigate_to_branch(project_id, branch_id);
        assert_eq!(
            state.projects_nav,
            ProjectsNav::Branch(project_id, branch_id)
        );
        assert_eq!(state.branch_session_index, 0);
    }

    /// Pane 1's own drill-down must not disturb the other two panes
    #[test]
    fn test_drilling_pane_one_leaves_the_other_panes_alone() {
        let mut state = AppState {
            settings_nav: SettingsNav::Notifications,
            sessions_pane_index: 3,
            ..Default::default()
        };

        state.navigate_to_project(uuid::Uuid::new_v4());
        state.navigate_to_branch(uuid::Uuid::new_v4(), uuid::Uuid::new_v4());

        assert_eq!(state.settings_nav, SettingsNav::Notifications);
        assert_eq!(state.sessions_pane_index, 3);
    }

    #[test]
    fn test_navigate_to_session_records_the_pane_it_came_from() {
        let mut state = AppState {
            focus: Focus::Panes(Tab::Settings),
            ..Default::default()
        };
        let session_id = uuid::Uuid::new_v4();

        state.navigate_to_session(session_id);

        assert_eq!(state.focus, Focus::Session);
        assert_eq!(state.active_session, Some(session_id));
        assert_eq!(state.input_mode, InputMode::Session);
        assert_eq!(
            state.session_return_focus,
            Some(Focus::Panes(Tab::Settings))
        );
        assert_eq!(state.session_scroll_offset, 0);
    }

    /// Jumping session-to-session must not overwrite the origin pane with
    /// "the session view", which is what used to strand `Esc`
    #[test]
    fn test_jumping_between_sessions_keeps_the_original_pane() {
        let mut state = AppState {
            focus: Focus::Panes(Tab::Settings),
            ..Default::default()
        };
        state.navigate_to_session(uuid::Uuid::new_v4());
        state.navigate_to_session(uuid::Uuid::new_v4());

        assert_eq!(
            state.session_return_focus,
            Some(Focus::Panes(Tab::Settings))
        );
    }

    #[test]
    fn test_return_from_session_restores_the_pane_it_was_opened_from() {
        // Backed by a temp store so the test never touches the real
        // ~/.panoptes/sessions.json.
        let temp_dir = tempfile::TempDir::new().unwrap();
        let sessions = SessionManager::with_store(
            crate::config::Config::default(),
            crate::session::SessionStore::with_path(temp_dir.path().join("sessions.json")),
        );

        // Attention-jump out of Settings: Esc must land back in Settings
        let mut state = AppState {
            focus: Focus::Panes(Tab::Settings),
            settings_nav: SettingsNav::Notifications,
            ..Default::default()
        };
        state.navigate_to_session(uuid::Uuid::new_v4());
        state.return_from_session(&sessions);

        assert_eq!(state.focus, Focus::Panes(Tab::Settings));
        assert_eq!(state.settings_nav, SettingsNav::Notifications);
        assert!(state.active_session.is_none());
        assert!(state.session_return_focus.is_none());
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_return_to_pane_one_follows_the_session_to_its_branch() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config = crate::config::Config {
            worktrees_dir: temp_dir.path().join("worktrees"),
            hooks_dir: temp_dir.path().join("hooks"),
            ..crate::config::Config::default()
        };
        let mut sessions = SessionManager::with_store(
            config,
            crate::session::SessionStore::with_path(temp_dir.path().join("sessions.json")),
        );

        let project_id = uuid::Uuid::new_v4();
        let branch_id = uuid::Uuid::new_v4();
        let session_id = sessions
            .insert_test_session("elsewhere", project_id, branch_id)
            .unwrap();

        // Pane 1 is sitting on the overview when the session is opened
        let mut state = AppState::default();
        state.navigate_to_session(session_id);
        state.return_from_session(&sessions);

        assert_eq!(state.focus, Focus::Panes(Tab::Projects));
        assert_eq!(
            state.projects_nav,
            ProjectsNav::Branch(project_id, branch_id)
        );
    }

    #[test]
    fn test_esc_pops_one_level_and_is_a_no_op_at_a_root() {
        let project_id = uuid::Uuid::new_v4();
        let branch_id = uuid::Uuid::new_v4();

        let mut state = AppState {
            projects_nav: ProjectsNav::Branch(project_id, branch_id),
            ..Default::default()
        };
        assert!(state.navigate_back());
        assert_eq!(state.projects_nav, ProjectsNav::Project(project_id));
        assert!(state.navigate_back());
        assert_eq!(state.projects_nav, ProjectsNav::Overview);
        // At the root: nothing happens, and the focused pane does not change
        assert!(!state.navigate_back());
        assert_eq!(state.projects_nav, ProjectsNav::Overview);
        assert_eq!(state.focus, Focus::Panes(Tab::Projects));

        // Pane 2 is flat, so Esc never has anywhere to go
        state.focus = Focus::Panes(Tab::Sessions);
        assert!(!state.navigate_back());

        // Pane 3 pops back to its sections list, then stops
        state.focus = Focus::Panes(Tab::Settings);
        state.settings_nav = SettingsNav::About;
        assert!(state.navigate_back());
        assert_eq!(state.settings_nav, SettingsNav::Sections);
        assert!(!state.navigate_back());
    }

    #[test]
    fn test_resize_debounce_state() {
        let mut state = AppState::default();

        // Initially no resize pending
        assert!(state.last_resize.is_none());
        assert!(!state.pending_resize);

        // Simulate resize event
        state.last_resize = Some(Instant::now());
        state.pending_resize = true;

        assert!(state.pending_resize);
        assert!(state.last_resize.is_some());

        // After processing, pending should be cleared
        state.pending_resize = false;
        assert!(!state.pending_resize);
    }
}
