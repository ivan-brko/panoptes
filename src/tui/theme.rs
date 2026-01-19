//! Theme module for centralized color and style definitions
//!
//! This module provides semantic color constants and styles used throughout the UI.
//! Centralizing colors here makes it easy to maintain visual consistency and
//! implement theme switching in the future.

use ratatui::style::{Color, Modifier, Style};

/// Application theme with all color definitions
#[derive(Debug, Clone)]
pub struct Theme {
    // === Session States ===
    /// Session is starting up
    pub state_starting: Color,
    /// Claude is thinking/processing
    pub state_thinking: Color,
    /// Claude is executing a tool
    pub state_executing: Color,
    /// Claude is waiting for user input
    pub state_waiting: Color,
    /// Session is idle (no recent activity)
    pub state_idle: Color,
    /// Session has exited
    pub state_exited: Color,

    // === UI Elements ===
    /// Primary accent color (headers, titles)
    pub accent: Color,
    /// Secondary accent color
    pub accent_secondary: Color,
    /// Text color for normal content
    pub text: Color,
    /// Text color for muted/secondary content
    pub text_muted: Color,
    /// Color for selected/focused items
    pub selected: Color,
    /// Color for active items (running processes)
    pub active: Color,

    // === Input Modes ===
    /// Color for input mode prompts
    pub input_prompt: Color,

    // === Notifications ===
    /// Color for items needing attention (waiting state with flag)
    pub attention_waiting: Color,
    /// Color for idle sessions needing attention
    pub attention_idle: Color,
    /// Attention badge color
    pub attention_badge: Color,

    // === Banners ===
    /// Error banner background
    pub error_bg: Color,
    /// Error banner foreground
    pub error_fg: Color,
    /// Warning banner background
    pub warning_bg: Color,
    /// Warning banner foreground
    pub warning_fg: Color,

    // === Borders ===
    /// Normal border color
    pub border: Color,
    /// Focused/active border color
    pub border_focused: Color,
    /// Warning border color
    pub border_warning: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// Dark theme (default)
    pub fn dark() -> Self {
        Self {
            // Session states
            state_starting: Color::Blue,
            state_thinking: Color::Yellow,
            state_executing: Color::Cyan,
            state_waiting: Color::Green,
            state_idle: Color::DarkGray,
            state_exited: Color::Red,

            // UI elements
            accent: Color::Cyan,
            accent_secondary: Color::Blue,
            text: Color::White,
            text_muted: Color::DarkGray,
            selected: Color::White,
            active: Color::Green,

            // Input modes - using Magenta to avoid conflict with Yellow (thinking/idle)
            input_prompt: Color::Magenta,

            // Notifications
            attention_waiting: Color::Green,
            attention_idle: Color::Yellow,
            attention_badge: Color::Yellow,

            // Banners
            error_bg: Color::Red,
            error_fg: Color::White,
            warning_bg: Color::Yellow,
            warning_fg: Color::Black,

            // Borders
            border: Color::White,
            border_focused: Color::Cyan,
            border_warning: Color::Yellow,
        }
    }

    /// Get the color for a session state
    pub fn session_state_color(&self, state: &crate::session::SessionState) -> Color {
        use crate::session::SessionState;
        match state {
            SessionState::Starting => self.state_starting,
            SessionState::Thinking => self.state_thinking,
            SessionState::Executing(_) => self.state_executing,
            SessionState::Waiting => self.state_waiting,
            SessionState::Idle => self.state_idle,
            SessionState::Exited => self.state_exited,
        }
    }

    // === Style Builders ===

    /// Style for headers/titles
    pub fn header_style(&self) -> Style {
        Style::default().fg(self.accent).add_modifier(Modifier::BOLD)
    }

    /// Style for muted text
    pub fn muted_style(&self) -> Style {
        Style::default().fg(self.text_muted)
    }

    /// Style for selected items
    pub fn selected_style(&self) -> Style {
        Style::default()
            .fg(self.selected)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for input prompts
    pub fn input_style(&self) -> Style {
        Style::default().fg(self.input_prompt)
    }

    /// Style for error banners
    pub fn error_banner_style(&self) -> Style {
        Style::default().fg(self.error_fg).bg(self.error_bg)
    }

    /// Style for warning banners
    pub fn warning_banner_style(&self) -> Style {
        Style::default().fg(self.warning_fg).bg(self.warning_bg)
    }

    /// Style for attention badge based on state
    pub fn attention_badge_style(&self, is_waiting: bool) -> Style {
        let color = if is_waiting {
            self.attention_waiting
        } else {
            self.attention_idle
        };
        Style::default().fg(color)
    }
}

/// Global theme instance
/// In the future, this could be made configurable
static THEME: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();

/// Get the current theme
pub fn theme() -> &'static Theme {
    THEME.get_or_init(Theme::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_default() {
        let theme = Theme::default();
        assert_eq!(theme.accent, Color::Cyan);
        assert_eq!(theme.state_waiting, Color::Green);
    }

    #[test]
    fn test_theme_dark() {
        let theme = Theme::dark();
        assert_eq!(theme.input_prompt, Color::Magenta);
    }

    #[test]
    fn test_session_state_color() {
        use crate::session::SessionState;
        let theme = Theme::dark();

        assert_eq!(theme.session_state_color(&SessionState::Starting), Color::Blue);
        assert_eq!(theme.session_state_color(&SessionState::Thinking), Color::Yellow);
        assert_eq!(
            theme.session_state_color(&SessionState::Executing("Bash".to_string())),
            Color::Cyan
        );
        assert_eq!(theme.session_state_color(&SessionState::Waiting), Color::Green);
        assert_eq!(theme.session_state_color(&SessionState::Idle), Color::DarkGray);
        assert_eq!(theme.session_state_color(&SessionState::Exited), Color::Red);
    }

    #[test]
    fn test_global_theme() {
        let t = theme();
        assert_eq!(t.accent, Color::Cyan);
    }
}
