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

/// Default number of scrollback rows to retain
pub const DEFAULT_SCROLLBACK_ROWS: usize = 10000;

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
    /// Maximum scrollback rows this terminal was configured with
    scrollback_rows: usize,
}

impl VirtualTerminal {
    /// Create a new virtual terminal with the given dimensions
    ///
    /// Uses the default scrollback rows (10000 lines).
    pub fn new(rows: usize, cols: usize) -> Self {
        Self::with_scrollback(rows, cols, DEFAULT_SCROLLBACK_ROWS)
    }

    /// Create a new virtual terminal with the given dimensions and scrollback buffer
    pub fn with_scrollback(rows: usize, cols: usize, scrollback_rows: usize) -> Self {
        Self {
            parser: Parser::new(rows as u16, cols as u16, scrollback_rows),
            styled_cache: RefCell::new(None),
            scroll_offset: 0,
            scrollback_rows,
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
        let max = self.scrollback_capacity();
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

    /// Get the configured scrollback capacity (maximum rows of history kept)
    ///
    /// This is the capacity the terminal was created with, not how many rows
    /// of history currently exist.
    pub fn scrollback_capacity(&self) -> usize {
        self.scrollback_rows
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
        let lines = vt.visible_lines(vt.size().0);
        assert!(lines[0].starts_with("Hello, World!"));
    }

    #[test]
    fn test_vterm_newline() {
        let mut vt = VirtualTerminal::new(24, 80);
        // Use CR+LF for proper newline behavior (CR resets column, LF moves down)
        vt.process(b"Line 1\r\nLine 2");
        let lines = vt.visible_lines(vt.size().0);
        assert!(lines[0].starts_with("Line 1"));
        assert!(lines[1].starts_with("Line 2"));
    }

    #[test]
    fn test_vterm_cursor_movement() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"\x1b[5;10HX"); // Move to row 5, col 10 and print X
        let lines = vt.visible_lines(vt.size().0);
        assert_eq!(lines[4].chars().nth(9), Some('X'));
    }

    #[test]
    fn test_vterm_clear_screen() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"Text here");
        vt.process(b"\x1b[2J"); // Clear screen
        let lines = vt.visible_lines(vt.size().0);
        assert!(lines[0].is_empty());
    }

    #[test]
    fn test_vterm_resize() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"Hello");
        vt.resize(10, 40);
        assert_eq!(vt.size(), (10, 40));
        let lines = vt.visible_lines(vt.size().0);
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

    // VirtualTerminal scrollback tests
    #[test]
    fn test_vterm_with_scrollback() {
        let vt = VirtualTerminal::with_scrollback(24, 80, 5000);
        assert_eq!(vt.size(), (24, 80));
        assert_eq!(vt.scrollback_capacity(), 5000);
    }

    #[test]
    fn test_vterm_default_scrollback() {
        let vt = VirtualTerminal::new(24, 80);
        assert_eq!(vt.scrollback_capacity(), DEFAULT_SCROLLBACK_ROWS);
    }

    #[test]
    fn test_vterm_custom_scrollback_zero() {
        let vt = VirtualTerminal::with_scrollback(24, 80, 0);
        assert_eq!(vt.scrollback_capacity(), 0);
    }

    // visible_styled_lines: span merging and cache invalidation

    #[test]
    fn test_styled_lines_merge_adjacent_same_style_cells() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"\x1b[31mred\x1b[0m plain");

        let lines = vt.visible_styled_lines(24);
        let spans = &lines[0].spans;

        // "red" is three cells of identical style: one span, not three.
        // The unstyled tail becomes a second span.
        assert_eq!(spans.len(), 2, "spans: {:?}", spans);
        assert_eq!(spans[0].content.as_ref(), "red");
        assert_eq!(spans[0].style.fg, Some(Color::Indexed(1)));
        assert_eq!(spans[1].content.as_ref(), " plain");
        assert_eq!(spans[1].style.fg, Some(Color::Reset));
    }

    #[test]
    fn test_styled_lines_cache_returns_same_rc_until_invalidated() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"hello");

        let first = vt.visible_styled_lines(24);
        let second = vt.visible_styled_lines(24);
        assert!(
            Rc::ptr_eq(&first, &second),
            "repeated renders must reuse the cached allocation"
        );
    }

    #[test]
    fn test_styled_lines_cache_invalidated_by_process() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"hello");
        let before = vt.visible_styled_lines(24);

        vt.process(b" world");
        let after = vt.visible_styled_lines(24);

        assert!(!Rc::ptr_eq(&before, &after));
        assert!(after[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
            .starts_with("hello world"));
    }

    #[test]
    fn test_styled_lines_cache_invalidated_by_resize() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"hello");
        let before = vt.visible_styled_lines(24);

        vt.resize(10, 40);
        let after = vt.visible_styled_lines(24);

        assert!(!Rc::ptr_eq(&before, &after));
        // Only 10 rows exist after the resize
        assert_eq!(after.len(), 10);
    }

    #[test]
    fn test_styled_lines_scroll_offset_changes_what_is_returned() {
        let mut vt = VirtualTerminal::with_scrollback(4, 20, 100);
        for i in 0..10 {
            vt.process(format!("line {}\r\n", i).as_bytes());
        }

        let live = vt.visible_styled_lines(4);
        let live_text: Vec<String> = live
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();

        vt.scroll_up(3);
        let scrolled = vt.visible_styled_lines(4);
        let scrolled_text: Vec<String> = scrolled
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();

        assert!(
            !Rc::ptr_eq(&live, &scrolled),
            "scroll must invalidate cache"
        );
        // Scrolled back 3 rows: the whole viewport shows history 3 rows older
        assert_ne!(live_text, scrolled_text);
        assert_eq!(scrolled_text[0], "line 4");

        // Scrolling back to the bottom restores the live view content
        vt.scroll_to_bottom();
        let back = vt.visible_styled_lines(4);
        let back_text: Vec<String> = back
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        assert_eq!(back_text, live_text);
    }
}
