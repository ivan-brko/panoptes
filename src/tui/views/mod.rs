//! View rendering modules
//!
//! Each view in the application has its own module for rendering logic.

mod branch_detail;
mod placeholder;
mod project_detail;
mod projects;
mod session;
mod timeline;

pub use branch_detail::render_branch_detail;
pub use placeholder::render_placeholder;
pub use project_detail::render_project_detail;
pub use projects::render_projects_overview;
pub use session::render_session_view;
pub use timeline::render_timeline;
