//! Virtual terminal emulator
//!
//! This module provides a virtual terminal that parses ANSI escape sequences
//! and maintains a screen buffer, enabling proper display of full-screen
//! terminal applications like Claude Code.
//!
//! Uses the vt100 crate for complete terminal emulation.

use std::cell::RefCell;
use std::rc::Rc;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use vt100::Parser;

/// Cached styled lines for rendering optimization
struct StyledLinesCache {
    lines: Rc<Vec<Line<'static>>>,
    viewport_height: usize,
}

/// Convert vt100 color to ratatui color
fn convert_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(idx) => Color::Indexed(idx),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Number of lines to keep in scrollback buffer
const SCROLLBACK_ROWS: usize = 10000;

/// Virtual terminal with screen buffer
pub struct VirtualTerminal {
    /// VT100 parser that handles all escape sequences
    parser: Parser,
    /// Cached styled lines (invalidated on process/resize)
    styled_cache: RefCell<Option<StyledLinesCache>>,
}

impl VirtualTerminal {
    /// Create a new virtual terminal with the given dimensions
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: Parser::new(rows, cols, SCROLLBACK_ROWS),
            styled_cache: RefCell::new(None),
        }
    }

    /// Resize the terminal
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows, cols);
        // Invalidate cache on resize
        self.styled_cache.borrow_mut().take();
    }

    /// Set the scrollback view offset (0 = live view, >0 = scrolled back)
    ///
    /// The offset represents how many rows back from the live view to display.
    /// The visible_styled_lines() method will automatically read from the
    /// correct scrollback position.
    pub fn set_scrollback(&mut self, offset: usize) {
        self.parser.screen_mut().set_scrollback(offset);
        // Invalidate cache when scrollback changes
        self.styled_cache.borrow_mut().take();
    }

    /// Get current scrollback position (0 = live view)
    pub fn scrollback(&self) -> usize {
        self.parser.screen().scrollback()
    }

    /// Get the maximum scrollback buffer size
    pub fn max_scrollback(&self) -> usize {
        SCROLLBACK_ROWS
    }

    /// Process input bytes through the terminal emulator
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        // Invalidate cache when new data arrives
        self.styled_cache.borrow_mut().take();
    }

    /// Get visible styled lines for a viewport
    ///
    /// For full-screen terminal apps, we return the exact screen buffer content with colors.
    /// Results are cached until process() or resize() is called.
    /// Returns Rc to avoid cloning the entire vector on each render.
    pub fn visible_styled_lines(&self, viewport_height: u16) -> Rc<Vec<Line<'static>>> {
        let viewport_height = viewport_height as usize;

        // Check cache first
        {
            let cache = self.styled_cache.borrow();
            if let Some(ref cached) = *cache {
                if cached.viewport_height == viewport_height {
                    return Rc::clone(&cached.lines);
                }
            }
        }

        // Compute styled lines
        let screen = self.parser.screen();
        let rows = (screen.size().0 as usize).min(viewport_height);
        let cols = screen.size().1 as usize;

        let lines: Vec<Line<'static>> = (0..rows)
            .map(|row| {
                let mut spans: Vec<Span<'static>> = Vec::new();
                let mut current_text = String::new();
                let mut current_style = Style::default();

                for col in 0..cols {
                    let cell = screen.cell(row as u16, col as u16).unwrap();
                    let contents = cell.contents();
                    let text = if contents.is_empty() { " " } else { contents };

                    // Build style from cell attributes
                    let mut style = Style::default();
                    style = style.fg(convert_color(cell.fgcolor()));
                    style = style.bg(convert_color(cell.bgcolor()));

                    let mut modifiers = Modifier::empty();
                    if cell.bold() {
                        modifiers |= Modifier::BOLD;
                    }
                    if cell.italic() {
                        modifiers |= Modifier::ITALIC;
                    }
                    if cell.underline() {
                        modifiers |= Modifier::UNDERLINED;
                    }
                    if cell.inverse() {
                        modifiers |= Modifier::REVERSED;
                    }
                    if cell.dim() {
                        modifiers |= Modifier::DIM;
                    }
                    style = style.add_modifier(modifiers);

                    // If style changed, push current span and start new one
                    if style != current_style && !current_text.is_empty() {
                        spans.push(Span::styled(
                            std::mem::take(&mut current_text),
                            current_style,
                        ));
                    }
                    current_style = style;
                    current_text.push_str(text);
                }

                // Push final span (trim trailing spaces for the last span)
                let trimmed = current_text.trim_end();
                if !trimmed.is_empty() {
                    spans.push(Span::styled(trimmed.to_string(), current_style));
                } else if spans.is_empty() {
                    // Empty line - ensure we have at least one span
                    spans.push(Span::raw(""));
                }

                Line::from(spans)
            })
            .collect();

        let lines = Rc::new(lines);

        // Store in cache
        *self.styled_cache.borrow_mut() = Some(StyledLinesCache {
            lines: Rc::clone(&lines),
            viewport_height,
        });

        lines
    }

    /// Get dimensions (rows, cols)
    pub fn size(&self) -> (u16, u16) {
        self.parser.screen().size()
    }

    /// Check if the terminal application has enabled bracketed paste mode
    pub fn bracketed_paste_enabled(&self) -> bool {
        self.parser.screen().bracketed_paste()
    }

    /// Get cursor position (row, col) - 0-indexed
    pub fn cursor_position(&self) -> (u16, u16) {
        self.parser.screen().cursor_position()
    }

    /// Check if cursor is visible
    pub fn cursor_visible(&self) -> bool {
        !self.parser.screen().hide_cursor()
    }

    /// Check if the terminal application has enabled mouse reporting
    ///
    /// Returns true if the application is expecting mouse events (e.g., vim with mouse enabled).
    /// Mouse events should only be forwarded when this returns true.
    pub fn mouse_protocol_mode(&self) -> vt100::MouseProtocolMode {
        self.parser.screen().mouse_protocol_mode()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vterm_creation() {
        let vt = VirtualTerminal::new(24, 80);
        assert_eq!(vt.size(), (24, 80));
    }

    #[test]
    fn test_vterm_simple_text() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"Hello, World!");
        let lines = vt.visible_styled_lines(24);
        let first_line: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(first_line.starts_with("Hello, World!"));
    }

    #[test]
    fn test_vterm_newline() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"Line 1\r\nLine 2");
        let lines = vt.visible_styled_lines(24);
        let line0: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        let line1: String = lines[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(line0.starts_with("Line 1"));
        assert!(line1.starts_with("Line 2"));
    }

    #[test]
    fn test_vterm_resize() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"Hello");
        vt.resize(10, 40);
        assert_eq!(vt.size(), (10, 40));
    }

    #[test]
    fn test_bracketed_paste_mode() {
        let mut vt = VirtualTerminal::new(24, 80);
        assert!(!vt.bracketed_paste_enabled());
        vt.process(b"\x1b[?2004h");
        assert!(vt.bracketed_paste_enabled());
        vt.process(b"\x1b[?2004l");
        assert!(!vt.bracketed_paste_enabled());
    }

    #[test]
    fn test_cursor_position() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"\x1b[5;10H"); // Move to row 5, col 10 (1-indexed)
        let (row, col) = vt.cursor_position();
        assert_eq!(row, 4); // 0-indexed
        assert_eq!(col, 9); // 0-indexed
    }

    #[test]
    fn test_scrollback() {
        // Create a small terminal (5 rows) so we can easily generate scrollback
        let mut vt = VirtualTerminal::new(5, 80);

        // Initial scrollback should be 0
        assert_eq!(vt.scrollback(), 0);
        assert_eq!(vt.max_scrollback(), SCROLLBACK_ROWS);

        // Generate enough lines to create scrollback content
        // Print 10 lines (more than 5 row terminal height)
        for i in 0..10 {
            vt.process(format!("Line {}\r\n", i).as_bytes());
        }

        // Now set scrollback - this should work since we have scrollback content
        vt.set_scrollback(3);
        assert_eq!(vt.scrollback(), 3);

        // Reset scrollback
        vt.set_scrollback(0);
        assert_eq!(vt.scrollback(), 0);
    }
}
