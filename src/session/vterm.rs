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

/// Number of scrollback rows to retain
const SCROLLBACK_ROWS: usize = 10000;

/// Cached styled lines for rendering optimization
struct StyledLinesCache {
    lines: Rc<Vec<Line<'static>>>,
    viewport_height: usize,
    scroll_offset: usize,
}

/// Convert vt100 color to ratatui color
fn convert_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(idx) => Color::Indexed(idx),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Virtual terminal with screen buffer and scrollback support
pub struct VirtualTerminal {
    /// VT100 parser that handles all escape sequences
    parser: Parser,
    /// Cached styled lines (invalidated on process/resize/scroll)
    styled_cache: RefCell<Option<StyledLinesCache>>,
    /// Scroll offset from bottom (0 = live view, >0 = scrolled back into history)
    scroll_offset: usize,
}

impl VirtualTerminal {
    /// Create a new virtual terminal with the given dimensions and scrollback buffer
    pub fn new(rows: usize, cols: usize) -> Self {
        Self {
            parser: Parser::new(rows as u16, cols as u16, SCROLLBACK_ROWS),
            styled_cache: RefCell::new(None),
            scroll_offset: 0,
        }
    }

    /// Resize the terminal
    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.parser.screen_mut().set_size(rows as u16, cols as u16);
        // Invalidate cache on resize
        self.styled_cache.borrow_mut().take();
    }

    /// Process input bytes through the terminal emulator
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        // Invalidate cache when new data arrives
        self.styled_cache.borrow_mut().take();
    }

    /// Get the screen content as lines of text
    pub fn get_lines(&self) -> Vec<String> {
        let screen = self.parser.screen();
        let (rows, cols) = (screen.size().0 as usize, screen.size().1 as usize);

        (0..rows)
            .map(|row| {
                (0..cols)
                    .map(|col| {
                        let contents = screen.cell(row as u16, col as u16).unwrap().contents();
                        // Empty contents means unset cell - use space to preserve column positions
                        if contents.is_empty() {
                            " "
                        } else {
                            contents
                        }
                    })
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// Get visible lines for a viewport (plain text, no styling)
    pub fn visible_lines(&self, viewport_height: usize) -> Vec<String> {
        self.visible_styled_lines(viewport_height)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect()
    }

    /// Get visible styled lines for a viewport
    /// For full-screen terminal apps, we return the exact screen buffer content with colors
    /// Results are cached until process(), resize(), or scroll is called
    /// Returns Rc to avoid cloning the entire vector on each render
    ///
    /// When scrollback is active (via set_scrollback), vt100's cell() method
    /// automatically returns content from the scrolled position.
    pub fn visible_styled_lines(&self, viewport_height: usize) -> Rc<Vec<Line<'static>>> {
        // Check cache first (including scroll_offset in key)
        {
            let cache = self.styled_cache.borrow();
            if let Some(ref cached) = *cache {
                if cached.viewport_height == viewport_height
                    && cached.scroll_offset == self.scroll_offset
                {
                    return Rc::clone(&cached.lines);
                }
            }
        }

        let screen = self.parser.screen();
        let screen_rows = screen.size().0 as usize;
        let cols = screen.size().1 as usize;
        let rows_to_show = screen_rows.min(viewport_height);

        let lines: Vec<Line<'static>> = (0..rows_to_show)
            .map(|row| {
                let mut spans: Vec<Span<'static>> = Vec::new();
                let mut current_text = String::new();
                let mut current_style = Style::default();

                for col in 0..cols {
                    // cell() can return None when scrollback offset exceeds available content
                    let (text, style) = if let Some(cell) = screen.cell(row as u16, col as u16) {
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

                        (text, style)
                    } else {
                        // No cell available (scrolled past available content)
                        (" ", Style::default())
                    };

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
            scroll_offset: self.scroll_offset,
        });

        lines
    }

    /// Get dimensions
    pub fn size(&self) -> (usize, usize) {
        let size = self.parser.screen().size();
        (size.0 as usize, size.1 as usize)
    }

    /// Check if the terminal application has enabled bracketed paste mode
    pub fn bracketed_paste_enabled(&self) -> bool {
        self.parser.screen().bracketed_paste()
    }

    /// Set the scrollback view offset (0 = live view, >0 = scrolled back)
    ///
    /// The offset represents how many rows back from the live view to display.
    /// This uses vt100's set_scrollback() which shifts what cell() returns.
    pub fn set_scrollback(&mut self, offset: usize) {
        self.parser.screen_mut().set_scrollback(offset);
        self.scroll_offset = self.parser.screen().scrollback();
        // Invalidate cache when scrollback changes
        self.styled_cache.borrow_mut().take();
    }

    /// Scroll up (toward older content) by the given number of lines
    pub fn scroll_up(&mut self, lines: usize) {
        let current = self.parser.screen().scrollback();
        let max = self.max_scrollback();
        let new_offset = (current + lines).min(max);
        self.set_scrollback(new_offset);
    }

    /// Scroll down (toward newer content) by the given number of lines
    pub fn scroll_down(&mut self, lines: usize) {
        let current = self.parser.screen().scrollback();
        let new_offset = current.saturating_sub(lines);
        self.set_scrollback(new_offset);
    }

    /// Scroll to the bottom (live view)
    pub fn scroll_to_bottom(&mut self) {
        if self.scroll_offset != 0 {
            self.set_scrollback(0);
        }
    }

    /// Get current scroll offset (0 = at bottom/live view)
    pub fn scrollback_offset(&self) -> usize {
        self.parser.screen().scrollback()
    }

    /// Get maximum scrollback available (number of lines in history)
    pub fn max_scrollback(&self) -> usize {
        SCROLLBACK_ROWS
    }

    /// Check if at the bottom (live view)
    pub fn is_at_bottom(&self) -> bool {
        self.parser.screen().scrollback() == 0
    }

    /// Get cursor position (row, col)
    pub fn cursor_position(&self) -> (u16, u16) {
        let screen = self.parser.screen();
        screen.cursor_position()
    }

    /// Check if cursor should be visible
    pub fn cursor_visible(&self) -> bool {
        !self.parser.screen().hide_cursor()
    }

    /// Check if the terminal application has enabled mouse reporting
    ///
    /// Returns the mouse protocol mode that the application has requested.
    /// Mouse events should be forwarded to the PTY when this is not `None`.
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
        let lines = vt.get_lines();
        assert!(lines[0].starts_with("Hello, World!"));
    }

    #[test]
    fn test_vterm_newline() {
        let mut vt = VirtualTerminal::new(24, 80);
        // Use CR+LF for proper newline behavior (CR resets column, LF moves down)
        vt.process(b"Line 1\r\nLine 2");
        let lines = vt.get_lines();
        assert!(lines[0].starts_with("Line 1"));
        assert!(lines[1].starts_with("Line 2"));
    }

    #[test]
    fn test_vterm_cursor_movement() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"\x1b[5;10HX"); // Move to row 5, col 10 and print X
        let lines = vt.get_lines();
        assert_eq!(lines[4].chars().nth(9), Some('X'));
    }

    #[test]
    fn test_vterm_clear_screen() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"Text here");
        vt.process(b"\x1b[2J"); // Clear screen
        let lines = vt.get_lines();
        assert!(lines[0].is_empty());
    }

    #[test]
    fn test_vterm_resize() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"Hello");
        vt.resize(10, 40);
        assert_eq!(vt.size(), (10, 40));
        let lines = vt.get_lines();
        assert!(lines[0].starts_with("Hello"));
    }

    #[test]
    fn test_bracketed_paste_mode_default() {
        let vt = VirtualTerminal::new(24, 80);
        assert!(!vt.bracketed_paste_enabled());
    }

    #[test]
    fn test_bracketed_paste_mode_enabled() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"\x1b[?2004h");
        assert!(vt.bracketed_paste_enabled());
    }

    #[test]
    fn test_bracketed_paste_mode_disabled() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"\x1b[?2004h");
        vt.process(b"\x1b[?2004l");
        assert!(!vt.bracketed_paste_enabled());
    }
}
