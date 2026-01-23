//! Terminal UI module
//!
//! This module handles all terminal rendering and UI components using Ratatui.

pub mod frame;
pub mod notifications;
pub mod theme;
pub mod views;
pub mod widgets;

pub use notifications::{Notification, NotificationManager, NotificationType};
pub use theme::{theme, Theme};

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    terminal::{
        disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
    ExecutableCommand,
};
use ratatui::prelude::*;
use std::io::{self, stdout, Write};
use std::time::Duration;

/// Terminal UI wrapper
///
/// Handles terminal setup, teardown, and provides the rendering surface.
pub struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    /// Whether keyboard enhancement (key release detection) is enabled
    keyboard_enhancement_enabled: bool,
    /// Whether bracketed paste mode is enabled
    bracketed_paste_enabled: bool,
    /// Whether mouse capture is enabled
    mouse_capture_enabled: bool,
    /// Whether focus change reporting is enabled
    focus_change_enabled: bool,
}

impl Tui {
    /// Create a new TUI instance
    pub fn new() -> Result<Self> {
        let backend = CrosstermBackend::new(stdout());
        let terminal = Terminal::new(backend)?;
        Ok(Self {
            terminal,
            keyboard_enhancement_enabled: false,
            bracketed_paste_enabled: false,
            mouse_capture_enabled: false,
            focus_change_enabled: false,
        })
    }

    /// Enter TUI mode (raw mode + alternate screen)
    pub fn enter(&mut self) -> Result<()> {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;

        // Enable keyboard enhancement for key release detection (if supported)
        if supports_keyboard_enhancement().unwrap_or(false)
            && stdout()
                .execute(PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::REPORT_EVENT_TYPES,
                ))
                .is_ok()
        {
            self.keyboard_enhancement_enabled = true;
        }

        // Enable bracketed paste mode so paste events are detected
        if stdout().execute(EnableBracketedPaste).is_ok() {
            self.bracketed_paste_enabled = true;
        }

        // Enable mouse capture for scroll wheel support
        if stdout().execute(EnableMouseCapture).is_ok() {
            self.mouse_capture_enabled = true;
        }

        // Enable focus change reporting so we can track window focus
        if stdout().execute(EnableFocusChange).is_ok() {
            self.focus_change_enabled = true;
        }

        self.terminal.hide_cursor()?;
        self.terminal.clear()?;
        Ok(())
    }

    /// Exit TUI mode (restore terminal)
    pub fn exit(&mut self) -> Result<()> {
        // Pop keyboard enhancement FIRST (while still in raw mode)
        // The terminal may send a response sequence, which we need to consume
        if self.keyboard_enhancement_enabled {
            let _ = stdout().execute(PopKeyboardEnhancementFlags);
            let _ = stdout().flush();
            // Drain any pending terminal responses (CSI u sequences)
            while event::poll(Duration::from_millis(10)).unwrap_or(false) {
                let _ = event::read();
            }
            self.keyboard_enhancement_enabled = false;
        }

        // Disable bracketed paste mode
        if self.bracketed_paste_enabled {
            let _ = stdout().execute(DisableBracketedPaste);
            self.bracketed_paste_enabled = false;
        }

        // Disable mouse capture
        if self.mouse_capture_enabled {
            let _ = stdout().execute(DisableMouseCapture);
            self.mouse_capture_enabled = false;
        }

        // Disable focus change reporting
        if self.focus_change_enabled {
            let _ = stdout().execute(DisableFocusChange);
            self.focus_change_enabled = false;
        }

        // Now restore the terminal
        self.terminal.show_cursor()?;
        stdout().execute(LeaveAlternateScreen)?;
        disable_raw_mode()?;

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
        // Pop keyboard enhancement FIRST (while still in raw mode)
        // The terminal may send a response sequence, which we need to consume
        if self.keyboard_enhancement_enabled {
            let _ = stdout().execute(PopKeyboardEnhancementFlags);
            let _ = stdout().flush();
            // Drain any pending terminal responses (CSI u sequences)
            while event::poll(Duration::from_millis(10)).unwrap_or(false) {
                let _ = event::read();
            }
        }

        // Disable bracketed paste mode
        if self.bracketed_paste_enabled {
            let _ = stdout().execute(DisableBracketedPaste);
        }

        // Disable mouse capture
        if self.mouse_capture_enabled {
            let _ = stdout().execute(DisableMouseCapture);
        }

        // Disable focus change reporting
        if self.focus_change_enabled {
            let _ = stdout().execute(DisableFocusChange);
        }

        // Now restore the terminal
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
