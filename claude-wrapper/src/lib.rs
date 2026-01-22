//! Claude Code Wrapper Library
//!
//! A standalone library for wrapping Claude Code in a PTY with customizable frame,
//! header/footer areas, and input interception.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │ Outer Terminal (user's terminal)                            │
//! │  ┌──────────────────────────────────────────────────────┐  │
//! │  │ Header Area (customizable via callback)              │  │
//! │  ├──────────────────────────────────────────────────────┤  │
//! │  │ ┌──────────────────────────────────────────────────┐ │  │
//! │  │ │           Claude Code PTY Output                │ │  │
//! │  │ │       (parsed through vt100 emulator)          │ │  │
//! │  │ └──────────────────────────────────────────────────┘ │  │
//! │  │ Frame (colored border, dynamic)                      │  │
//! │  ├──────────────────────────────────────────────────────┤  │
//! │  │ Footer Area (customizable via callback)              │  │
//! │  └──────────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```rust,no_run
//! use claude_wrapper::{ClaudeWrapper, WrapperConfig, WrapperCallbacks};
//! use ratatui::prelude::*;
//!
//! struct MyCallbacks;
//!
//! impl WrapperCallbacks for MyCallbacks {
//!     fn on_esc(&mut self) -> bool {
//!         true // Return true to exit
//!     }
//!     fn render_header(&self, area: Rect, frame: &mut Frame) {
//!         // Custom header rendering
//!     }
//!     fn render_footer(&self, area: Rect, frame: &mut Frame) {
//!         // Custom footer rendering
//!     }
//!     fn border_color(&self) -> Color {
//!         Color::Blue
//!     }
//!     fn on_exit(&mut self, exit_code: Option<u32>) {
//!         // Handle process exit
//!     }
//! }
//! ```

pub mod frame;
pub mod input;
pub mod pty;
pub mod vterm;
pub mod wrapper;

pub use frame::FrameConfig;
pub use wrapper::{ClaudeWrapper, WrapperCallbacks, WrapperConfig};
