//! Normal mode input handlers
//!
//! One handler per pane, plus the full-screen session view. Each pane handler
//! routes on its own drill-down level; the Claude and Codex config sections
//! share one handler in [`crate::input::agent_configs`].

pub mod projects_pane;
pub mod session_view;
pub mod sessions_pane;
pub mod settings_pane;

use std::path::PathBuf;

use crate::app::App;
use crate::config::CustomShortcut;
use crate::project::{BranchId, ProjectId};
use crate::session::{NewSessionSpec, SessionId};
use crate::tui::frame::{FrameConfig, FrameLayout};

/// Launch a shell session running a custom shortcut's command
///
/// The shared body of the custom-shortcut key in the branch-detail and
/// session views: sizes the PTY like the session view renders it, creates a
/// shell session with the shortcut's command as initial input, and applies
/// the shortcut's auto-close setting at creation time (so the flag can never
/// miss a command that finishes instantly).
///
/// Returns the new session's ID; on failure the error has been surfaced to
/// the user already and `None` is returned. Navigation into the session is
/// left to the caller, since the two views enter it differently.
pub(crate) fn launch_shortcut_session(
    app: &mut App,
    shortcut: &CustomShortcut,
    project_id: ProjectId,
    branch_id: BranchId,
    working_dir: PathBuf,
) -> Option<SessionId> {
    let session_name = shortcut.short_display_name();

    // Get terminal size
    let terminal_size = app.tui.size().unwrap_or_default();
    let frame_config = FrameConfig::default();
    let layout = FrameLayout::calculate(terminal_size, &frame_config);
    let rows = layout.content.height as usize;
    let cols = layout.content.width as usize;

    // Create shell session with command
    match app.sessions.create_session(
        crate::agent::AgentType::Shell,
        NewSessionSpec {
            name: session_name,
            working_dir,
            project_id,
            branch_id,
            initial_prompt: Some(shortcut.command.clone()),
            account: None,
            auto_close: shortcut.auto_close,
        },
        rows,
        cols,
    ) {
        Ok(new_session_id) => Some(new_session_id),
        Err(e) => {
            tracing::error!("Failed to create shell session: {}", e);
            app.state.error_message = Some(format!("Failed to create session: {}", e));
            None
        }
    }
}
