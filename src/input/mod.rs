//! Input handling module
//!
//! Handles keyboard and mouse input dispatching based on current mode and view.

pub mod dialogs;
pub mod dispatcher;
pub mod normal;
pub mod session_mode;
pub mod text_input;

// Re-export commonly used items
pub use dispatcher::handle_key_event;
