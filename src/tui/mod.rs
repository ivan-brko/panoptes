//! Terminal UI module
//!
//! This module handles all terminal rendering and UI components using Ratatui.

pub mod theme;
pub mod views;
pub mod widgets;

pub use theme::{theme, Theme};

use anyhow::Result;
use crossterm::{
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
}

impl Tui {
    /// Create a new TUI instance
    pub fn new() -> Result<Self> {
        let backend = CrosstermBackend::new(stdout());
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    /// Enter TUI mode (raw mode + alternate screen)
    pub fn enter(&mut self) -> Result<()> {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        self.terminal.hide_cursor()?;
        self.terminal.clear()?;
        Ok(())
    }

    /// Exit TUI mode (restore terminal)
    pub fn exit(&mut self) -> Result<()> {
        self.terminal.show_cursor()?;
        stdout().execute(LeaveAlternateScreen)?;
        disable_raw_mode()?;
        Ok(())
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
        // Attempt to restore terminal state on drop
        let _ = self.terminal.show_cursor();
        let _ = stdout().execute(LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    // TUI tests require a real terminal, so we keep them minimal here
    // Integration tests would use a mock or headless approach

    #[test]
    fn test_module_compiles() {
        // Basic compilation check
        assert!(true);
    }
}
