//! Helper functions for consistent selection rendering across all menus.
//!
//! This module provides utilities to render selected items in lists with a consistent
//! visual style: arrow prefix, bold text, and appropriate colors. No background highlighting
//! is used to maintain a clean, professional appearance.

use ratatui::style::{Color, Modifier, Style, Stylize};

use crate::tui::theme::Theme;

/// Returns the selection prefix for a list item.
///
/// Selected items get an arrow (`▶ `), unselected items get two spaces for alignment.
///
/// # Example
/// ```no_run
/// # use panoptes::tui::widgets::selection::selection_prefix;
/// let prefix = selection_prefix(true);
/// assert_eq!(prefix, "▶ ");
/// ```
pub fn selection_prefix(is_selected: bool) -> &'static str {
    if is_selected {
        "▶ "
    } else {
        "  "
    }
}

/// Returns a style for a selected/unselected item with a custom color.
///
/// Selected items are bold with the specified color. Unselected items use the same color
/// but without bold. This is useful when you want to use state-based colors (e.g., session
/// state colors, attention indicators).
///
/// # Example
/// ```no_run
/// # use panoptes::tui::widgets::selection::selection_style;
/// # use ratatui::style::Color;
/// let style = selection_style(true, Color::Green);
/// // Creates a bold green style for selected item
/// ```
pub fn selection_style(is_selected: bool, base_color: Color) -> Style {
    if is_selected {
        Style::default().fg(base_color).bold()
    } else {
        Style::default().fg(base_color)
    }
}

/// Returns a style for a selected/unselected item using theme colors.
///
/// Selected items are bold with the accent color. Unselected items use the default text color.
/// This is the most common pattern for simple menus where selection is the primary visual cue.
///
/// # Example
/// ```no_run
/// # use panoptes::tui::widgets::selection::selection_style_with_accent;
/// # use panoptes::tui::theme::theme;
/// let t = theme();
/// let style = selection_style_with_accent(true, &t);
/// // Creates a bold cyan (accent) style for selected item
/// ```
pub fn selection_style_with_accent(is_selected: bool, theme: &Theme) -> Style {
    if is_selected {
        Style::default().fg(theme.accent).bold()
    } else {
        Style::default().fg(theme.text)
    }
}

/// Returns a style for an item name with selection styling.
///
/// This is a convenience function that combines the color logic with modifiers.
/// It's commonly used for rendering item names in lists.
///
/// # Example
/// ```no_run
/// # use panoptes::tui::widgets::selection::selection_name_style;
/// # use panoptes::tui::theme::theme;
/// # use ratatui::text::Span;
/// # use ratatui::style::Stylize;
/// let t = theme();
/// let style = selection_name_style(true, &t);
/// let span = Span::styled("My Item", style);
/// // Creates a bold cyan "My Item" span
/// ```
pub fn selection_name_style(is_selected: bool, theme: &Theme) -> Style {
    Style::default()
        .fg(if is_selected {
            theme.accent
        } else {
            theme.text
        })
        .add_modifier(if is_selected {
            Modifier::BOLD
        } else {
            Modifier::empty()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selection_prefix() {
        assert_eq!(selection_prefix(true), "▶ ");
        assert_eq!(selection_prefix(false), "  ");
    }

    #[test]
    fn test_selection_style() {
        let selected = selection_style(true, Color::Green);
        let unselected = selection_style(false, Color::Green);

        // Both should have green foreground
        assert!(matches!(selected.fg, Some(Color::Green)));
        assert!(matches!(unselected.fg, Some(Color::Green)));

        // Only selected should be bold
        assert!(selected.add_modifier.contains(Modifier::BOLD));
        assert!(!unselected.add_modifier.contains(Modifier::BOLD));
    }
}
