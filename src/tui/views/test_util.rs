//! Shared helpers for view render tests
//!
//! Every view test follows the same shape: draw into a `TestBackend`
//! buffer, flatten it to text lines, and assert on content or styles.
//! These helpers implement that shape once; view test modules wrap
//! [`render_to_buffer`] with their own render call.

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::prelude::*;
use ratatui::Terminal;

/// Render a frame closure into a test buffer of the given size
pub(crate) fn render_to_buffer(width: u16, height: u16, draw: impl FnOnce(&mut Frame)) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| draw(frame)).unwrap();
    terminal.backend().buffer().clone()
}

/// The buffer's rows as strings, trimmed of trailing padding
pub(crate) fn buffer_lines(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.get(x, y).symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// Render a frame closure and return its lines, trimmed of padding
pub(crate) fn render_to_lines(
    width: u16,
    height: u16,
    draw: impl FnOnce(&mut Frame),
) -> Vec<String> {
    buffer_lines(&render_to_buffer(width, height, draw))
}

/// Whether any rendered line contains `needle`
pub(crate) fn contains_line(lines: &[String], needle: &str) -> bool {
    lines.iter().any(|line| line.contains(needle))
}

/// Column at which `needle` starts on the row that contains it
///
/// Counted in characters, not bytes: border and marker glyphs are
/// multi-byte, so `str::find` alone would not give a screen column.
pub(crate) fn column_of(lines: &[String], needle: &str) -> usize {
    lines
        .iter()
        .find_map(|line| line.find(needle).map(|b| line[..b].chars().count()))
        .unwrap_or_else(|| panic!("no rendered row contains {:?}", needle))
}

/// Style of the first cell of `needle` on the row containing it
pub(crate) fn style_of_row_with(buffer: &Buffer, needle: &str) -> Style {
    for y in 0..buffer.area.height {
        let line: String = (0..buffer.area.width)
            .map(|x| buffer.get(x, y).symbol())
            .collect();
        if let Some(byte_idx) = line.find(needle) {
            // Screen column is a character count, not a byte offset
            let col = line[..byte_idx].chars().count() as u16;
            return buffer.get(col, y).style();
        }
    }
    panic!("no rendered row contains {:?}", needle);
}
