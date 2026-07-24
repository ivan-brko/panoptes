//! Terminal UI module
//!
//! This module handles all terminal rendering and UI components using Ratatui.

pub mod frame;
pub mod header;
pub mod header_notifications;
pub mod layout;
pub mod logo;
pub mod panes;
pub mod theme;
pub mod views;
pub mod widgets;

pub use header::Header;
pub use header_notifications::HeaderNotificationManager;
pub use layout::ScreenLayout;
pub use theme::{theme, Theme};

use anyhow::Result;
use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::*;
use std::io::{self, stdout};

/// Terminal UI wrapper
///
/// Handles terminal setup, teardown, and provides the rendering surface.
pub struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    /// Whether bracketed paste mode is enabled
    bracketed_paste_enabled: bool,
    /// Whether mouse capture is enabled
    mouse_capture_enabled: bool,
}

/// Error handler for terminal cleanup operations
/// Used during both normal exit and panic/drop scenarios
enum ErrorHandler {
    /// Log errors via tracing (normal exit)
    Tracing,
    /// Print errors to stderr (panic/drop, tracing may be unavailable)
    Stderr,
}

impl ErrorHandler {
    fn handle(&self, context: &str, error: impl std::fmt::Display) {
        match self {
            ErrorHandler::Tracing => tracing::warn!("{}: {}", context, error),
            ErrorHandler::Stderr => eprintln!("TUI teardown: {}: {}", context, error),
        }
    }

    fn debug(&self, message: &str) {
        if let ErrorHandler::Tracing = self {
            tracing::debug!("{}", message);
        }
    }
}

/// Disable bracketed paste mode
fn disable_bracketed_paste(handler: &ErrorHandler) {
    if let Err(e) = stdout().execute(DisableBracketedPaste) {
        handler.handle("failed to disable bracketed paste", e);
    }
    handler.debug("Bracketed paste disabled");
}

/// Disable mouse capture
fn disable_mouse_capture_internal(handler: &ErrorHandler) {
    if let Err(e) = stdout().execute(DisableMouseCapture) {
        handler.handle("failed to disable mouse capture", e);
    }
    handler.debug("Mouse capture disabled");
}

impl Tui {
    /// Create a new TUI instance
    pub fn new() -> Result<Self> {
        let backend = CrosstermBackend::new(stdout());
        let terminal = Terminal::new(backend)?;
        Ok(Self {
            terminal,
            bracketed_paste_enabled: false,
            mouse_capture_enabled: false,
        })
    }

    /// Enter TUI mode (raw mode + alternate screen)
    pub fn enter(&mut self) -> Result<()> {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;

        // Enable bracketed paste mode so paste events are detected
        if stdout().execute(EnableBracketedPaste).is_ok() {
            self.bracketed_paste_enabled = true;
        }

        // Enable mouse capture for scroll wheel support
        if stdout().execute(EnableMouseCapture).is_ok() {
            self.mouse_capture_enabled = true;
        }

        self.terminal.hide_cursor()?;
        self.terminal.clear()?;
        Ok(())
    }

    /// Exit TUI mode (restore terminal)
    pub fn exit(&mut self) -> Result<()> {
        tracing::debug!("Starting TUI exit sequence");
        let handler = ErrorHandler::Tracing;

        // Disable bracketed paste mode
        if self.bracketed_paste_enabled {
            disable_bracketed_paste(&handler);
            self.bracketed_paste_enabled = false;
        }

        // Disable mouse capture
        if self.mouse_capture_enabled {
            disable_mouse_capture_internal(&handler);
            self.mouse_capture_enabled = false;
        }

        // Now restore the terminal
        self.terminal.show_cursor()?;
        stdout().execute(LeaveAlternateScreen)?;
        disable_raw_mode()?;

        tracing::debug!("TUI exit sequence completed");
        Ok(())
    }

    /// Enable mouse capture (for scroll wheel support)
    pub fn enable_mouse_capture(&mut self) {
        if !self.mouse_capture_enabled && stdout().execute(EnableMouseCapture).is_ok() {
            self.mouse_capture_enabled = true;
        }
    }

    /// Disable mouse capture (allows native text selection)
    pub fn disable_mouse_capture(&mut self) {
        if self.mouse_capture_enabled {
            let _ = stdout().execute(DisableMouseCapture);
            self.mouse_capture_enabled = false;
        }
    }

    /// Check whether mouse capture is currently enabled.
    pub fn is_mouse_capture_enabled(&self) -> bool {
        self.mouse_capture_enabled
    }

    /// Get terminal size
    pub fn size(&self) -> Result<Rect> {
        Ok(self.terminal.size()?)
    }

    /// Draw a frame
    pub fn draw<F>(&mut self, f: F) -> Result<()>
    where
        F: FnOnce(&mut Frame),
    {
        self.terminal.draw(f)?;
        Ok(())
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        // Note: During drop, tracing may not be available, so errors go to stderr
        // for emergency diagnostics
        let handler = ErrorHandler::Stderr;

        // Disable bracketed paste mode
        if self.bracketed_paste_enabled {
            disable_bracketed_paste(&handler);
        }

        // Disable mouse capture
        if self.mouse_capture_enabled {
            disable_mouse_capture_internal(&handler);
        }

        // Now restore the terminal
        if let Err(e) = self.terminal.show_cursor() {
            handler.handle("failed to show cursor", e);
        }
        if let Err(e) = stdout().execute(LeaveAlternateScreen) {
            handler.handle("failed to leave alternate screen", e);
        }
        if let Err(e) = disable_raw_mode() {
            handler.handle("failed to disable raw mode", e);
        }
    }
}

#[cfg(test)]
mod tests {
    // TUI tests require a real terminal, so we keep them minimal here
    // Integration tests would use a mock or headless approach

    #[test]
    fn test_module_compiles() {
        // Basic compilation check
        let _ = std::mem::size_of::<super::Tui>();
    }
}
