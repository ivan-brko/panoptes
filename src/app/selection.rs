//! Mouse text selection inside an active session
//!
//! Panoptes keeps mouse capture on while a session is on screen - it has to,
//! or the wheel cannot drive local scrollback - and a captured mouse means the
//! terminal's own drag-selection is off. So selection is done here instead,
//! the way tmux does it: take the drag events, draw the highlight in our own
//! render pass, and lift the text out of the vterm on release.
//!
//! Everything in this module is pure. The event routing lives beside the other
//! mouse handling in [`crate::app`], and the highlight is painted in
//! [`crate::tui::views::session`].
//!
//! # Coordinates
//!
//! A selection is anchored in *absolute* rows - row 0 is the oldest line of
//! scrollback, and [`crate::session::VirtualTerminal::history_rows`] is the
//! top of the live screen - rather than in rows of the current view. PTY reads
//! are frozen for the duration of a drag, so those addresses cannot move, and
//! the selection survives the view scrolling under it when a drag runs off the
//! edge of the screen.

use std::time::{Duration, Instant};

use ratatui::prelude::Rect;

use crate::session::SessionId;

/// A selected cell: absolute row, column
pub type Cell = (usize, u16);

/// How far a click may land from the previous one and still count as repeat
const MULTI_CLICK_TOLERANCE: u16 = 1;

/// The most rows one tick of edge auto-scroll may move
///
/// Pushing further past the edge scrolls faster, the way iTerm does, but not
/// without limit: past this the view moves faster than anyone can read it and
/// the extra distance buys nothing.
const MAX_SCROLL_STEP: usize = 12;

/// What shape a drag draws
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shape {
    /// The terminal's usual selection: the first row from the anchor to the
    /// end of the line, whole rows after it, the last row up to the head
    #[default]
    Stream,
    /// A rectangle: the same column range from every row
    ///
    /// What a stream selection cannot do - take one column out of `docker ps`
    /// or `ls -l` without dragging the rest of every line with it.
    Block,
}

/// How much of the text one click takes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Granularity {
    /// A single cell, extended cell by cell (one click)
    #[default]
    Cell,
    /// Whole words (double click)
    Word,
    /// Whole logical lines (triple click)
    Line,
}

impl Granularity {
    /// What a click count means: 1 cell, 2 word, 3 line, and round again
    pub fn for_click_count(clicks: u8) -> Self {
        match clicks {
            2 => Granularity::Word,
            3 => Granularity::Line,
            _ => Granularity::Cell,
        }
    }
}

/// Which way a drag is pushing past the edge, and how hard
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    /// Toward older output (above the top) or newer (below the bottom)
    pub direction: EdgeDirection,
    /// How many rows past the boundary the pointer is
    ///
    /// The further out, the faster the view should follow - a pointer parked
    /// just past the edge is nudging, one thrown to the top of the screen is
    /// asking to get somewhere.
    pub overshoot: u16,
}

impl Edge {
    /// How many rows to scroll for this overshoot, on one tick
    pub fn scroll_step(self) -> usize {
        (1 + usize::from(self.overshoot) / 2).min(MAX_SCROLL_STEP)
    }
}

/// Which way an edge-held drag wants the view to move
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeDirection {
    /// Above the top row - the view should scroll toward older output
    Above,
    /// Below the bottom row - the view should scroll toward newer output
    Below,
}

/// An in-progress or just-finished mouse selection
///
/// Lives only as long as the screen it was made against: the copy happens on
/// release, so the highlight is free to evaporate on the next output, click,
/// resize, or session switch without anything being lost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSelection {
    /// The session the selection belongs to - never rendered against another
    pub session_id: SessionId,
    /// The end that stays put
    pub anchor: Cell,
    /// The end that follows the pointer
    pub head: Cell,
    /// Whether the button is still down; only then is output held back
    pub dragging: bool,
    /// Last pointer position in content-area coordinates (row, col)
    ///
    /// Kept in view coordinates on purpose: when the view scrolls under a
    /// held button the head follows the pointer's place on *screen*, which is
    /// what makes edge auto-scroll extend the selection.
    pub pointer: (u16, u16),
    /// Which edge the pointer is pushing past, if any
    pub edge: Option<Edge>,
    /// Whether the drag moves by cells, words or lines
    pub granularity: Granularity,
    /// The whole word or line the drag began in
    ///
    /// A word-granularity drag pivots around this rather than around a single
    /// cell: dragging left from the middle of a word must keep all of that
    /// word, not just the half the pointer started on.
    pub anchor_span: (Cell, Cell),
    /// Stream or rectangle
    ///
    /// Decided by the modifier held when the button went down and fixed for
    /// the drag: changing shape halfway would redraw the highlight around
    /// text the user never dragged over.
    pub shape: Shape,
}

impl SessionSelection {
    /// Start a selection covering `span`, which one click makes a single cell
    pub fn started(
        session_id: SessionId,
        span: (Cell, Cell),
        granularity: Granularity,
        pointer: (u16, u16),
        shape: Shape,
    ) -> Self {
        Self {
            session_id,
            anchor: span.0,
            head: span.1,
            dragging: true,
            pointer,
            edge: None,
            granularity,
            anchor_span: span,
            shape,
        }
    }

    /// Move the head to cover `span`, keeping whichever side of the anchor's
    /// own span is further from it
    ///
    /// At cell granularity this is just "the head is where the pointer is".
    /// At word or line granularity it is what makes a drag grow by whole
    /// words in either direction.
    pub fn extend_to(&mut self, span: (Cell, Cell)) {
        if span.0 < self.anchor_span.0 {
            self.anchor = self.anchor_span.1;
            self.head = span.0;
        } else {
            self.anchor = self.anchor_span.0;
            self.head = span.1;
        }
    }

    /// The two ends in reading order, so backward and upward drags work
    pub fn ordered(&self) -> (Cell, Cell) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// The rectangle a block selection covers: `(top, bottom)` rows and
    /// `(left, right)` columns, all inclusive
    ///
    /// Rows and columns are ordered independently, which is the whole
    /// difference from [`Self::ordered`]: dragging up and to the left draws
    /// the same rectangle as dragging down and to the right, whereas a stream
    /// selection started at the later cell would run the other way.
    pub fn block_bounds(&self) -> ((usize, usize), (u16, u16)) {
        let (top, bottom) = if self.anchor.0 <= self.head.0 {
            (self.anchor.0, self.head.0)
        } else {
            (self.head.0, self.anchor.0)
        };
        let (left, right) = if self.anchor.1 <= self.head.1 {
            (self.anchor.1, self.head.1)
        } else {
            (self.head.1, self.anchor.1)
        };
        ((top, bottom), (left, right))
    }

    /// Whether this is a plain click that never went anywhere
    ///
    /// One click, one cell, no movement - which must not touch the clipboard.
    /// A double click covering a single-letter word is *not* this: the user
    /// asked for that letter.
    pub fn is_bare_click(&self) -> bool {
        self.granularity == Granularity::Cell && self.anchor == self.head
    }
}

/// Where a mouse event lands relative to a content area
///
/// The column is clamped into the rect and the row is clamped with the edge
/// it overshot reported separately: a drag that leaves the content area still
/// extends the selection to the edge (dropping the event would make the
/// selection stutter at the border) and, above or below, asks for the view to
/// scroll.
pub fn locate(rect: Rect, row: u16, column: u16) -> ((u16, u16), Option<Edge>) {
    if rect.width == 0 || rect.height == 0 {
        return ((0, 0), None);
    }
    let last_row = rect.height - 1;
    let last_col = rect.width - 1;

    let (view_row, edge) = if row < rect.y {
        (
            0,
            Some(Edge {
                direction: EdgeDirection::Above,
                overshoot: rect.y - row,
            }),
        )
    } else if row >= rect.y + rect.height {
        (
            last_row,
            Some(Edge {
                direction: EdgeDirection::Below,
                overshoot: row - (rect.y + last_row),
            }),
        )
    } else {
        (row - rect.y, None)
    };
    let view_col = column.saturating_sub(rect.x).min(last_col);

    ((view_row, view_col), edge)
}

/// Whether a mouse event landed inside a content area
pub fn contains(rect: Rect, row: u16, column: u16) -> bool {
    row >= rect.y && row < rect.y + rect.height && column >= rect.x && column < rect.x + rect.width
}

/// The absolute row a view row refers to, given the top of the viewport
pub fn absolute_row(viewport_top: usize, view_row: u16) -> usize {
    viewport_top + usize::from(view_row)
}

/// The view row an absolute row falls on, or `None` when it is off screen
pub fn view_row(viewport_top: usize, row: usize, height: u16) -> Option<u16> {
    let offset = row.checked_sub(viewport_top)?;
    if offset < usize::from(height) {
        Some(offset as u16)
    } else {
        None
    }
}

/// Counts clicks that land close together in time and space
///
/// crossterm reports presses, not click counts, so the 1 -> 2 -> 3 -> 1 cycle
/// behind double- and triple-click selection is tracked here.
#[derive(Debug, Default)]
pub struct ClickTracker {
    /// When and where the last press landed
    last: Option<(Instant, (u16, u16))>,
    /// How many clicks the current run is up to
    count: u8,
}

impl ClickTracker {
    /// Register a press at a terminal cell, returning 1, 2 or 3
    ///
    /// `window` is how long a second press may take and still count as a
    /// double click - the system's setting, if the user tells us what it is.
    pub fn press(&mut self, now: Instant, cell: (u16, u16), window: Duration) -> u8 {
        let continues = self.last.is_some_and(|(at, previous)| {
            now.saturating_duration_since(at) <= window
                && previous.0.abs_diff(cell.0) <= MULTI_CLICK_TOLERANCE
                && previous.1.abs_diff(cell.1) <= MULTI_CLICK_TOLERANCE
        });
        self.count = if continues { self.count % 3 + 1 } else { 1 };
        self.last = Some((now, cell));
        self.count
    }

    /// Forget the run, so the next press starts a fresh one
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// What kind of thing a cell holds, for deciding where a word ends
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellClass {
    /// Part of a word: alphanumeric, or one of the configured extra characters
    Word,
    /// Blank
    Space,
    /// Punctuation outside the word set, which stands alone
    Other,
}

/// Which class a cell's text falls into
fn class_of(text: &str, word_characters: &str) -> CellClass {
    match text.chars().next() {
        None => CellClass::Space,
        Some(c) if c.is_alphanumeric() || word_characters.contains(c) => CellClass::Word,
        Some(c) if c.is_whitespace() => CellClass::Space,
        Some(_) => CellClass::Other,
    }
}

/// The class of column `col` of `cells`
///
/// A wide character's second cell carries no text of its own, so it inherits
/// the class of the cell it continues.
fn column_class(cells: &[Option<String>], col: u16, word_characters: &str) -> CellClass {
    let mut index = usize::from(col);
    loop {
        match cells.get(index) {
            Some(Some(text)) => return class_of(text, word_characters),
            // A continuation cell: ask the character it belongs to
            Some(None) if index > 0 => index -= 1,
            _ => return CellClass::Space,
        }
    }
}

/// The column a wide character's cell starts on
fn word_start_column(cells: &[Option<String>], col: u16) -> u16 {
    let mut index = usize::from(col);
    while index > 0 && matches!(cells.get(index), Some(None)) {
        index -= 1;
    }
    index as u16
}

/// The last column a character covers, including a wide character's second cell
fn word_end_column(cells: &[Option<String>], col: u16) -> u16 {
    let mut index = usize::from(col);
    while matches!(cells.get(index + 1), Some(None)) {
        index += 1;
    }
    index as u16
}

/// Expand a clicked cell to the run of like characters around it (double-click)
///
/// A word takes the whole word, and whitespace takes the whole run of
/// whitespace - both follow soft-wraps, so a path broken across two screen
/// rows selects whole. Punctuation outside the word set stands alone, which
/// is what keeps a double click on a bracket from swallowing its neighbours.
///
/// `row_cells` yields the cells of an absolute row, `wrapped` says whether a
/// row continues into the next one, and `word_characters` is the configured
/// set of non-alphanumerics that still count as part of a word.
pub fn word_at(
    cell: Cell,
    row_cells: &dyn Fn(usize) -> Vec<Option<String>>,
    wrapped: &dyn Fn(usize) -> bool,
    word_characters: &str,
) -> (Cell, Cell) {
    let (row, col) = cell;
    let cells = row_cells(row);
    let single = ((row, col), (row, col));
    if cells.is_empty() {
        return single;
    }
    // Only runs of word characters or of whitespace expand; anything else is
    // its own selection
    let class = column_class(&cells, col, word_characters);
    if class == CellClass::Other {
        return single;
    }
    let same =
        |cells: &[Option<String>], col: u16| column_class(cells, col, word_characters) == class;

    // Walk left, crossing into the row above while it soft-wraps into this one
    let mut start = (row, word_start_column(&cells, col));
    let mut start_cells = cells.clone();
    loop {
        if start.1 > 0 {
            let next = word_start_column(&start_cells, start.1 - 1);
            if !same(&start_cells, next) {
                break;
            }
            start.1 = next;
        } else {
            let Some(above) = start.0.checked_sub(1) else {
                break;
            };
            if !wrapped(above) {
                break;
            }
            let above_cells = row_cells(above);
            let Some(last) = above_cells.len().checked_sub(1) else {
                break;
            };
            let last = word_start_column(&above_cells, last as u16);
            if !same(&above_cells, last) {
                break;
            }
            start = (above, last);
            start_cells = above_cells;
        }
    }

    // Walk right, crossing into the row below while this one soft-wraps
    let mut end = (row, word_end_column(&cells, col));
    let mut end_cells = cells;
    loop {
        let next_col = end.1 + 1;
        if usize::from(next_col) < end_cells.len() {
            if !same(&end_cells, next_col) {
                break;
            }
            end.1 = word_end_column(&end_cells, next_col);
        } else {
            if !wrapped(end.0) {
                break;
            }
            let below_cells = row_cells(end.0 + 1);
            if below_cells.is_empty() || !same(&below_cells, 0) {
                break;
            }
            end = (end.0 + 1, word_end_column(&below_cells, 0));
            end_cells = below_cells;
        }
    }

    (start, end)
}

/// Expand a clicked cell to its whole logical line (triple-click)
///
/// A logical line is every screen row joined by soft-wraps: walk up while the
/// row above continues into this one, and down while this one continues into
/// the row below. `last_row` bounds the walk at the bottom of the buffer.
pub fn logical_line_at(
    cell: Cell,
    wrapped: &dyn Fn(usize) -> bool,
    width: u16,
    last_row: usize,
) -> (Cell, Cell) {
    let mut first = cell.0;
    while first > 0 && wrapped(first - 1) {
        first -= 1;
    }
    let mut last = cell.0;
    while last < last_row && wrapped(last) {
        last += 1;
    }
    ((first, 0), (last, width.saturating_sub(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(text: &str) -> Vec<Option<String>> {
        text.chars().map(|c| Some(c.to_string())).collect()
    }

    fn selection(anchor: Cell, head: Cell) -> SessionSelection {
        SessionSelection {
            session_id: uuid::Uuid::nil(),
            anchor,
            head,
            dragging: true,
            pointer: (0, 0),
            edge: None,
            shape: Shape::Stream,
            granularity: Granularity::Cell,
            anchor_span: (anchor, anchor),
        }
    }

    const WINDOW: Duration = Duration::from_millis(400);
    const WORD_CHARS: &str = "/-+\\~_.";

    fn above(overshoot: u16) -> Edge {
        Edge {
            direction: EdgeDirection::Above,
            overshoot,
        }
    }

    fn below(overshoot: u16) -> Edge {
        Edge {
            direction: EdgeDirection::Below,
            overshoot,
        }
    }

    // Normalization

    #[test]
    fn test_a_forward_drag_is_already_in_reading_order() {
        let sel = selection((3, 2), (5, 7));
        assert_eq!(sel.ordered(), ((3, 2), (5, 7)));
        assert!(!sel.is_bare_click());
    }

    /// Dragging up, or right-to-left on one row, must select the same text as
    /// dragging the other way
    #[test]
    fn test_backward_and_upward_drags_normalize() {
        assert_eq!(selection((5, 7), (3, 2)).ordered(), ((3, 2), (5, 7)));
        assert_eq!(selection((4, 9), (4, 1)).ordered(), ((4, 1), (4, 9)));
        // Same row, same column: nothing to swap
        assert_eq!(selection((4, 1), (4, 9)).ordered(), ((4, 1), (4, 9)));
    }

    /// A row further down wins regardless of column: the lower row's column 0
    /// is still later in the text than the upper row's last column
    #[test]
    fn test_ordering_compares_rows_before_columns() {
        assert_eq!(selection((6, 0), (5, 79)).ordered(), ((5, 79), (6, 0)));
    }

    #[test]
    fn test_a_click_that_never_moved_is_a_single_cell() {
        assert!(selection((2, 4), (2, 4)).is_bare_click());
        assert!(!selection((2, 4), (2, 5)).is_bare_click());
    }

    // Coordinate translation

    #[test]
    fn test_locate_reports_content_relative_cells() {
        let rect = Rect::new(2, 5, 40, 10);
        assert_eq!(locate(rect, 5, 2), ((0, 0), None));
        assert_eq!(locate(rect, 9, 12), ((4, 10), None));
        assert_eq!(locate(rect, 14, 41), ((9, 39), None));
        assert!(contains(rect, 5, 2));
        assert!(!contains(rect, 4, 2));
        assert!(!contains(rect, 5, 42));
    }

    /// A drag that leaves the content area keeps extending to the edge -
    /// dropping the event would make the selection stutter at the border
    #[test]
    fn test_locate_clamps_and_reports_the_edge_it_overshot() {
        let rect = Rect::new(2, 5, 40, 10);

        assert_eq!(locate(rect, 0, 12), ((0, 10), Some(above(5))));
        assert_eq!(locate(rect, 30, 12), ((9, 10), Some(below(16))));
        // Sideways is a clamp only: there is nothing to scroll horizontally
        assert_eq!(locate(rect, 9, 0), ((4, 0), None));
        assert_eq!(locate(rect, 9, 200), ((4, 39), None));
        // Both at once
        assert_eq!(locate(rect, 99, 0), ((9, 0), Some(below(85))));
    }

    #[test]
    fn test_absolute_and_view_rows_round_trip() {
        assert_eq!(absolute_row(120, 3), 123);
        assert_eq!(view_row(120, 123, 10), Some(3));
        // Scrolled off the top and off the bottom
        assert_eq!(view_row(120, 119, 10), None);
        assert_eq!(view_row(120, 130, 10), None);
        assert_eq!(view_row(120, 129, 10), Some(9));
    }

    // Click counting

    #[test]
    fn test_clicks_cycle_one_two_three_and_back() {
        let mut tracker = ClickTracker::default();
        let start = Instant::now();

        assert_eq!(tracker.press(start, (4, 10), WINDOW), 1);
        assert_eq!(
            tracker.press(start + Duration::from_millis(100), (4, 10), WINDOW),
            2
        );
        assert_eq!(
            tracker.press(start + Duration::from_millis(200), (4, 10), WINDOW),
            3
        );
        // A fourth click starts a new selection rather than a fourth mode
        assert_eq!(
            tracker.press(start + Duration::from_millis(300), (4, 10), WINDOW),
            1
        );
    }

    #[test]
    fn test_a_slow_second_click_is_a_new_click() {
        let mut tracker = ClickTracker::default();
        let start = Instant::now();

        assert_eq!(tracker.press(start, (4, 10), WINDOW), 1);
        assert_eq!(
            tracker.press(start + Duration::from_millis(401), (4, 10), WINDOW),
            1
        );
    }

    /// A hand shakes: one cell of drift still counts as the same click
    #[test]
    fn test_click_counting_tolerates_one_cell_of_drift() {
        let mut tracker = ClickTracker::default();
        let start = Instant::now();

        assert_eq!(tracker.press(start, (4, 10), WINDOW), 1);
        assert_eq!(
            tracker.press(start + Duration::from_millis(50), (5, 11), WINDOW),
            2
        );
        // Two cells away is a different place on screen
        assert_eq!(
            tracker.press(start + Duration::from_millis(100), (5, 14), WINDOW),
            1
        );
    }

    #[test]
    fn test_reset_ends_the_run() {
        let mut tracker = ClickTracker::default();
        let start = Instant::now();

        assert_eq!(tracker.press(start, (4, 10), WINDOW), 1);
        tracker.reset();
        assert_eq!(
            tracker.press(start + Duration::from_millis(50), (4, 10), WINDOW),
            1
        );
    }

    // Word expansion

    fn word_in(text: &str, col: u16) -> (Cell, Cell) {
        let rows = [cells(text)];
        word_at(
            (0, col),
            &|row| rows.get(row).cloned().unwrap_or_default(),
            &|_| false,
            WORD_CHARS,
        )
    }

    #[test]
    fn test_double_click_takes_the_word_under_the_pointer() {
        // "hello world" - clicking anywhere in "world" takes all of it
        assert_eq!(word_in("hello world", 8), ((0, 6), (0, 10)));
        assert_eq!(word_in("hello world", 6), ((0, 6), (0, 10)));
        assert_eq!(word_in("hello world", 0), ((0, 0), (0, 4)));
    }

    /// Paths and flags are one word: stopping at every `/` or `-` would make
    /// double-click useless for the things a terminal is full of
    #[test]
    fn test_word_characters_include_the_punctuation_paths_use() {
        assert_eq!(word_in("cd /usr/local-bin/x.rs now", 10), ((0, 3), (0, 21)));
    }

    #[test]
    fn test_clicking_whitespace_takes_the_run_of_whitespace() {
        // One space between two words is a run of one
        assert_eq!(word_in("hello world", 5), ((0, 5), (0, 5)));
        // A wider gap comes whole, the way iTerm takes it
        assert_eq!(word_in("hello     world", 7), ((0, 5), (0, 9)));
        // Punctuation outside the word set stands alone, so a double click on
        // a bracket does not swallow what is beside it
        assert_eq!(word_in("a(b)c", 1), ((0, 1), (0, 1)));
        assert_eq!(word_in("((()))", 3), ((0, 3), (0, 3)));
    }

    /// The word set is the user's to change, so it has to be a parameter and
    /// not a constant baked into the walk
    #[test]
    fn test_the_word_character_set_decides_where_a_word_ends() {
        let rows = [cells("a-b_c")];
        let expand = |word_characters: &str| {
            word_at(
                (0, 2),
                &|row| rows.get(row).cloned().unwrap_or_default(),
                &|_| false,
                word_characters,
            )
        };

        // The default set holds the whole thing together
        assert_eq!(expand("/-+\\~_."), ((0, 0), (0, 4)));
        // Without the dash, the run stops at it
        assert_eq!(expand("_"), ((0, 2), (0, 4)));
        // With nothing extra, only the alphanumeric under the pointer
        assert_eq!(expand(""), ((0, 2), (0, 2)));
    }

    // Click granularity

    #[test]
    fn test_click_counts_map_to_what_they_take() {
        assert_eq!(Granularity::for_click_count(1), Granularity::Cell);
        assert_eq!(Granularity::for_click_count(2), Granularity::Word);
        assert_eq!(Granularity::for_click_count(3), Granularity::Line);
        // The cycle starts over rather than inventing a fourth mode
        assert_eq!(Granularity::for_click_count(4), Granularity::Cell);
    }

    /// A drag that began as a double click grows by whole words, in either
    /// direction, and never gives back the word it started on
    #[test]
    fn test_a_word_drag_pivots_around_the_whole_word_it_started_in() {
        // The drag began on the word spanning columns 6..=10
        let mut sel = SessionSelection::started(
            uuid::Uuid::nil(),
            ((0, 6), (0, 10)),
            Granularity::Word,
            (0, 8),
            Shape::Stream,
        );
        assert_eq!(sel.ordered(), ((0, 6), (0, 10)));

        // Dragging right onto a later word keeps the anchor word whole
        sel.extend_to(((0, 12), (0, 16)));
        assert_eq!(sel.ordered(), ((0, 6), (0, 16)));

        // Dragging back left past the start takes the earlier word whole, and
        // the anchor flips to the far side of the word it began in
        sel.extend_to(((0, 0), (0, 4)));
        assert_eq!(sel.ordered(), ((0, 0), (0, 10)));

        // And back again
        sel.extend_to(((0, 12), (0, 16)));
        assert_eq!(sel.ordered(), ((0, 6), (0, 16)));
    }

    #[test]
    fn test_a_bare_click_is_only_one_click_that_never_moved() {
        let click = SessionSelection::started(
            uuid::Uuid::nil(),
            ((2, 4), (2, 4)),
            Granularity::Cell,
            (0, 0),
            Shape::Stream,
        );
        assert!(click.is_bare_click());

        // A double click that landed on a one-letter word is not a bare
        // click: the user asked for that letter
        let letter = SessionSelection::started(
            uuid::Uuid::nil(),
            ((2, 4), (2, 4)),
            Granularity::Word,
            (0, 0),
            Shape::Stream,
        );
        assert!(!letter.is_bare_click());

        let mut dragged = click.clone();
        dragged.extend_to(((2, 5), (2, 5)));
        assert!(!dragged.is_bare_click());
    }

    // Edge auto-scroll

    /// Held just past the edge it nudges; thrown to the far side of the
    /// screen it gets somewhere - but never faster than a reader can follow
    #[test]
    fn test_auto_scroll_speeds_up_the_further_out_the_pointer_is() {
        assert_eq!(above(1).scroll_step(), 1);
        assert_eq!(above(4).scroll_step(), 3);
        assert_eq!(below(10).scroll_step(), 6);
        assert_eq!(below(22).scroll_step(), 12);
        // Capped, so throwing the pointer off the display does not make the
        // view uncontrollable
        assert_eq!(below(1000).scroll_step(), 12);
    }

    #[test]
    fn test_locate_reports_how_far_past_the_edge_the_pointer_is() {
        let rect = Rect::new(2, 5, 40, 10);
        assert_eq!(locate(rect, 4, 12).1, Some(above(1)));
        assert_eq!(locate(rect, 0, 12).1, Some(above(5)));
        assert_eq!(locate(rect, 15, 12).1, Some(below(1)));
        assert_eq!(locate(rect, 20, 12).1, Some(below(6)));
    }

    /// A word broken by a soft wrap is still one word
    #[test]
    fn test_word_expansion_crosses_a_soft_wrap() {
        let rows = [cells("abcdefghij"), cells("klmno     ")];
        let expanded = word_at(
            (0, 8),
            &|row| rows.get(row).cloned().unwrap_or_default(),
            // Row 0 continues into row 1
            &|row| row == 0,
            WORD_CHARS,
        );
        assert_eq!(expanded, ((0, 0), (1, 4)));

        // The same word, clicked from the far side
        let expanded = word_at(
            (1, 2),
            &|row| rows.get(row).cloned().unwrap_or_default(),
            &|row| row == 0,
            WORD_CHARS,
        );
        assert_eq!(expanded, ((0, 0), (1, 4)));
    }

    /// A hard newline is a boundary even though the rows are adjacent
    #[test]
    fn test_word_expansion_stops_at_a_hard_newline() {
        let rows = [cells("abcdefghij"), cells("klmno     ")];
        let expanded = word_at(
            (1, 2),
            &|row| rows.get(row).cloned().unwrap_or_default(),
            &|_| false,
            WORD_CHARS,
        );
        assert_eq!(expanded, ((1, 0), (1, 4)));
    }

    /// A wide character owns two columns; the word must cover both, or the
    /// highlight would stop halfway through the glyph
    #[test]
    fn test_word_expansion_covers_both_halves_of_a_wide_character() {
        let row = vec![
            Some("\u{65e5}".to_string()),
            None,
            Some("\u{672c}".to_string()),
            None,
            Some(" ".to_string()),
        ];
        let rows = [row];
        let expanded = word_at(
            (0, 2),
            &|r| rows.get(r).cloned().unwrap_or_default(),
            &|_| false,
            WORD_CHARS,
        );
        assert_eq!(expanded, ((0, 0), (0, 3)));

        // Clicking the continuation cell finds the same word
        let expanded = word_at(
            (0, 1),
            &|r| rows.get(r).cloned().unwrap_or_default(),
            &|_| false,
            WORD_CHARS,
        );
        assert_eq!(expanded, ((0, 0), (0, 3)));
    }

    // Logical lines

    #[test]
    fn test_triple_click_takes_one_unwrapped_row() {
        assert_eq!(
            logical_line_at((5, 3), &|_| false, 80, 100),
            ((5, 0), (5, 79))
        );
    }

    /// Three screen rows of one logical line select as one
    #[test]
    fn test_triple_click_walks_the_whole_wrapped_line() {
        // Rows 4 and 5 continue into the next; row 6 ends the line
        let wrapped = |row: usize| row == 4 || row == 5;
        assert_eq!(
            logical_line_at((5, 3), &wrapped, 80, 100),
            ((4, 0), (6, 79))
        );
        assert_eq!(
            logical_line_at((4, 0), &wrapped, 80, 100),
            ((4, 0), (6, 79))
        );
        assert_eq!(
            logical_line_at((6, 9), &wrapped, 80, 100),
            ((4, 0), (6, 79))
        );
    }

    /// The walk must stop at the end of the buffer, not run past it
    #[test]
    fn test_triple_click_stops_at_the_last_row() {
        assert_eq!(
            logical_line_at((9, 0), &|_| true, 80, 10),
            ((0, 0), (10, 79))
        );
    }
}
