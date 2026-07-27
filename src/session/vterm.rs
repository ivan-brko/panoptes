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

    /// Whether the child is currently in the alternate screen
    ///
    /// Decides scroll semantics: an alternate-screen app owns its own
    /// scrolling (there is no terminal scrollback to show), so scroll input
    /// belongs to the app, exactly as in a real terminal.
    pub fn alternate_screen_active(&self) -> bool {
        self.parser.screen().alternate_screen()
    }

    /// Whether the child enabled application cursor keys (DECCKM)
    ///
    /// Decides the encoding of synthesized arrow keys (`ESC O A` vs
    /// `ESC [ A`), e.g. when a wheel notch is translated to arrows for an
    /// alternate-screen app.
    pub fn application_cursor_keys(&self) -> bool {
        self.parser.screen().application_cursor()
    }

    /// Check if the terminal application has enabled mouse reporting
    ///
    /// Returns the mouse protocol mode that the application has requested.
    /// Mouse events should be forwarded to the PTY when this is not `None`.
    pub fn mouse_protocol_mode(&self) -> vt100::MouseProtocolMode {
        self.parser.screen().mouse_protocol_mode()
    }

    /// How many rows of history the terminal is holding right now
    ///
    /// The origin for the absolute row coordinates the other methods here
    /// take: visible row `r` is absolute row `history_rows() - offset + r`,
    /// where `offset` is [`Self::scrollback_offset`]. Anchoring a selection
    /// in absolute rows is what lets it survive the view scrolling under it.
    pub fn history_rows(&self) -> usize {
        self.parser.screen().history_rows()
    }

    /// The absolute row of the top of the current viewport
    ///
    /// Convenience for the conversion above; `None` would mean a scrollback
    /// offset deeper than the history, which the vterm clamps away.
    pub fn viewport_top_row(&self) -> usize {
        self.history_rows().saturating_sub(self.scrollback_offset())
    }

    /// The text selected between two cells, both ends inclusive
    ///
    /// Rows are absolute (see [`Self::history_rows`]), so a selection that
    /// spans more than the viewport reads correctly. `end_col` is inclusive
    /// here, unlike vt100's exclusive `contents_between`: a selection names
    /// the last cell the pointer covered, not the one after it.
    ///
    /// Soft-wrapped rows are joined into one line, wide characters are not
    /// split, and trailing blanks are dropped - all of which vt100 already
    /// does for us.
    pub fn contents_between(
        &self,
        start_row: usize,
        start_col: u16,
        end_row: usize,
        end_col: u16,
    ) -> String {
        self.parser.screen().contents_between_absolute(
            start_row,
            start_col,
            end_row,
            end_col.saturating_add(1),
        )
    }

    /// The text of a rectangle of cells, `end_col` inclusive
    ///
    /// The block counterpart of [`Self::contents_between`]: the same column
    /// range from every row, so one column of a table comes out without the
    /// rest of each line. Rows are never joined - a rectangle's rows line up
    /// with the rows on screen, wrapped or not.
    pub fn contents_in_columns(
        &self,
        start_row: usize,
        end_row: usize,
        start_col: u16,
        end_col: u16,
    ) -> String {
        self.parser.screen().contents_in_columns_absolute(
            start_row,
            end_row,
            start_col,
            end_col.saturating_add(1),
        )
    }

    /// Whether an absolute row soft-wraps into the next one
    ///
    /// `true` means this row *continues* into the row below, which is how a
    /// logical line is walked for word and line selection.
    pub fn row_wrapped(&self, row: usize) -> bool {
        self.parser.screen().row_wrapped_absolute(row)
    }

    /// The cells of an absolute row: text per column, `None` for the second
    /// half of a wide character
    pub fn row_cells(&self, row: usize) -> Vec<Option<String>> {
        self.parser.screen().row_cells_absolute(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rectangle takes the same columns from every row
    ///
    /// The thing a stream selection cannot do: one column of a table, without
    /// the rest of each line coming with it.
    #[test]
    fn test_block_selection_takes_one_column_of_a_table() {
        let mut vt = VirtualTerminal::new(6, 40);
        vt.process(b"NAME      STATUS    PORT\r\n");
        vt.process(b"alpha     running   8080\r\n");
        vt.process(b"beta      exited    9090\r\n");

        // Columns 10..=17 are the STATUS column
        let block = vt.contents_in_columns(0, 2, 10, 17);
        assert_eq!(block, "STATUS  \nrunning \nexited  ");

        // The same rows as a stream selection drag in whole lines instead
        let stream = vt.contents_between(0, 10, 2, 17);
        assert_eq!(
            stream,
            "STATUS    PORT\nalpha     running   8080\nbeta      exited  "
        );
    }

    /// Every row of a rectangle contributes a line, including a blank one, so
    /// the result lines up with the rows on screen
    #[test]
    fn test_block_selection_keeps_blank_rows() {
        let mut vt = VirtualTerminal::new(6, 20);
        vt.process(b"aaa\r\n\r\nccc\r\n");

        assert_eq!(vt.contents_in_columns(0, 2, 0, 2), "aaa\n\nccc");
    }

    /// A rectangle ignores soft wrapping
    ///
    /// A stream selection joins a wrapped row to the next, because they are
    /// one logical line. A rectangle must not: joining would slide every later
    /// cell out of the column the user drew.
    #[test]
    fn test_block_selection_does_not_join_wrapped_rows() {
        let mut vt = VirtualTerminal::new(6, 10);
        // 14 characters into a 10-column terminal: wraps after "0123456789"
        vt.process(b"0123456789abcd");

        assert_eq!(vt.contents_in_columns(0, 1, 0, 1), "01\nab");
        // The stream selection over the same cells treats it as one line
        assert_eq!(vt.contents_between(0, 0, 1, 1), "0123456789ab");
    }

    /// A wide character straddling the edge of the rectangle
    ///
    /// vt100 keeps a wide character in its *first* cell and leaves the second
    /// empty, which gives one rule that holds at both edges: the character
    /// belongs to the rectangle exactly when its first cell does. A right edge
    /// landing on that first cell keeps the whole character; a left edge
    /// landing on the second half yields a blank in its place, never half a
    /// character - and the blank is what keeps every row the width the user
    /// drew, so a pasted rectangle still lines up.
    #[test]
    fn test_block_selection_handles_wide_characters_at_both_edges() {
        let mut vt = VirtualTerminal::new(4, 20);
        // Each CJK character occupies two columns: 0-1, 2-3, 4-5
        vt.process("一二三".as_bytes());

        // Both halves inside: kept, obviously
        assert_eq!(vt.contents_in_columns(0, 0, 0, 1), "一");

        // Right edge on the second character's *first* cell: kept whole, so
        // the text is never cut mid-character
        assert_eq!(vt.contents_in_columns(0, 0, 0, 2), "一二");

        // Left edge on the second character's *second* cell: that orphaned
        // half comes out as a blank rather than as half a character, which is
        // what keeps the rectangle rectangular - every row is the width the
        // user drew, so the columns still line up when pasted.
        assert_eq!(vt.contents_in_columns(0, 0, 3, 5), " 三");
    }

    /// An empty or inverted rectangle is empty, never a panic
    #[test]
    fn test_block_selection_rejects_an_empty_rectangle() {
        let mut vt = VirtualTerminal::new(4, 20);
        vt.process(b"hello");

        // Columns beyond the terminal clamp rather than panic
        assert_eq!(vt.contents_in_columns(0, 0, 100, 200), "");
        // A row range running backwards selects nothing
        assert_eq!(vt.contents_in_columns(2, 0, 0, 4), "");
    }

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

    /// The Codex CLI pattern: a DECSTBM region pinning a footer at the bottom
    /// while history scrolls out of the top. Real terminals feed those lines
    /// into scrollback; upstream vt100 discarded them (our vendored patch
    /// restores the real behavior). Without this, Codex sessions had no
    /// scrollback at all and needed a plain-text fallback buffer.
    #[test]
    fn test_lines_scrolled_out_of_a_top_anchored_region_reach_scrollback() {
        let mut vt = VirtualTerminal::with_scrollback(6, 40, 100);
        // Region rows 1..=4, footer pinned on rows 5-6
        vt.process(b"\x1b[1;4r");
        vt.process(b"\x1b[6;1HFOOTER");
        // Print into the region: 10 lines through a 4-row window
        vt.process(b"\x1b[1;1H");
        for i in 0..10 {
            vt.process(format!("line-{}\r\n", i).as_bytes());
        }

        // The footer must not have moved
        let live = vt.visible_lines(6);
        assert_eq!(live[5], "FOOTER");

        // Lines pushed out of the region's top are in scrollback
        vt.scroll_up(3);
        let scrolled = vt.visible_lines(6);
        assert!(
            scrolled[0].starts_with("line-"),
            "history must survive region scrolling: {:?}",
            scrolled
        );

        // A region that does NOT start at the top row still discards, exactly
        // like a real terminal
        let mut vt = VirtualTerminal::with_scrollback(6, 40, 100);
        vt.process(b"\x1b[2;5r\x1b[2;1H");
        for i in 0..10 {
            vt.process(format!("mid-{}\r\n", i).as_bytes());
        }
        vt.scroll_up(5);
        assert_eq!(vt.scrollback_offset(), 0, "no scrollback should exist");
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

    // contents_between: the clipboard extraction behind drag-to-copy

    /// The release cell is part of the selection, so the wrapper's `end_col`
    /// is inclusive where vt100's own is exclusive
    #[test]
    fn test_contents_between_includes_the_cell_the_drag_ended_on() {
        let mut vt = VirtualTerminal::new(24, 80);
        vt.process(b"FIDELITY-42 and more");

        let top = vt.viewport_top_row();
        assert_eq!(vt.contents_between(top, 0, top, 10), "FIDELITY-42");
        // A single cell is a one-character selection, not an empty one
        assert_eq!(vt.contents_between(top, 0, top, 0), "F");
    }

    /// A soft-wrapped logical line is one line in the clipboard: the wrap is
    /// the terminal's doing, not the text's
    #[test]
    fn test_contents_between_joins_a_soft_wrapped_line() {
        let mut vt = VirtualTerminal::new(6, 10);
        vt.process(b"abcdefghijKLMNO\r\nnext");

        let top = vt.viewport_top_row();
        assert_eq!(vt.contents_between(top, 0, top + 1, 4), "abcdefghijKLMNO");
        // A hard newline is preserved
        assert_eq!(
            vt.contents_between(top, 0, top + 2, 3),
            "abcdefghijKLMNO\nnext"
        );
    }

    /// A wide character occupies two cells; releasing on either must copy the
    /// whole character rather than half of it
    #[test]
    fn test_contents_between_does_not_split_wide_characters() {
        let mut vt = VirtualTerminal::new(6, 20);
        vt.process("日本語".as_bytes());

        let top = vt.viewport_top_row();
        assert_eq!(vt.contents_between(top, 0, top, 1), "日");
        assert_eq!(vt.contents_between(top, 0, top, 5), "日本語");
        assert_eq!(vt.contents_between(top, 2, top, 3), "本");
    }

    #[test]
    fn test_contents_between_trims_trailing_blanks() {
        let mut vt = VirtualTerminal::new(6, 20);
        vt.process(b"hi\r\nthere");

        let top = vt.viewport_top_row();
        // The selection runs to the end of a mostly empty row: the blanks the
        // user dragged over are not text
        assert_eq!(vt.contents_between(top, 0, top + 1, 19), "hi\nthere");
    }

    /// Scrolled-back rows keep their absolute addresses, which is what makes
    /// a selection survive the view moving under it
    #[test]
    fn test_contents_between_reads_scrollback_by_absolute_row() {
        let mut vt = VirtualTerminal::with_scrollback(4, 20, 100);
        for i in 0..10 {
            vt.process(format!("line {}\r\n", i).as_bytes());
        }

        // Absolute row 4 is "line 4" whether or not the view is scrolled
        assert_eq!(vt.contents_between(4, 0, 4, 5), "line 4");
        vt.scroll_up(3);
        assert_eq!(vt.contents_between(4, 0, 4, 5), "line 4");
        // And a selection made while scrolled up extracts what is on screen
        let top = vt.viewport_top_row();
        assert_eq!(vt.contents_between(top, 0, top, 5), "line 4");

        // A range taller than the viewport still resolves - the case that
        // only exists because the drag scrolled the view along with it
        assert_eq!(
            vt.contents_between(0, 0, 5, 5),
            "line 0\nline 1\nline 2\nline 3\nline 4\nline 5"
        );
    }

    /// Backwards ranges never reach the wrapper (the selection is normalized
    /// first), but must not panic or invent text if one ever does
    #[test]
    fn test_contents_between_rejects_a_backwards_range() {
        let mut vt = VirtualTerminal::new(6, 20);
        vt.process(b"hello");
        let top = vt.viewport_top_row();

        assert_eq!(vt.contents_between(top + 1, 0, top, 0), "");
        assert_eq!(vt.contents_between(1000, 0, 1001, 5), "");
    }

    #[test]
    fn test_row_cells_and_wrapping_report_the_row_shape() {
        let mut vt = VirtualTerminal::new(4, 6);
        vt.process("ab日x\r\n".as_bytes());
        vt.process(b"cdefghIJ");

        let top = vt.viewport_top_row();
        let cells = vt.row_cells(top);
        assert_eq!(cells.len(), 6);
        assert_eq!(cells[0].as_deref(), Some("a"));
        assert_eq!(cells[2].as_deref(), Some("\u{65e5}"));
        // The wide character's second cell is not a character of its own
        assert_eq!(cells[3], None);
        assert_eq!(cells[4].as_deref(), Some("x"));

        assert!(!vt.row_wrapped(top), "a hard newline does not wrap");
        assert!(
            vt.row_wrapped(top + 1),
            "a row filled to the edge continues into the next"
        );
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
