//! The Panoptes wordmark
//!
//! Drawn from half-block glyphs so it costs three rows rather than the five a
//! figlet font would. Every row is exactly [`WORDMARK_WIDTH`] single-width
//! columns, which is what lets the header lay text out beside it by arithmetic
//! instead of by measuring.

/// The wordmark, one string per row - `PANOPTES` in half blocks
pub const WORDMARK: [&str; 3] = [
    "\u{2584}\u{2584}\u{2584} \u{2584}\u{2584}\u{2584} \u{2584}\u{2584}\u{2584} \u{2584}\u{2584}\u{2584} \u{2584}\u{2584}\u{2584} \u{2584}\u{2588}\u{2584} \u{2584}\u{2584}\u{2584} \u{2584}\u{2584}\u{2584}",
    "\u{2588} \u{2588} \u{2588}\u{2584}\u{2588} \u{2588} \u{2588} \u{2588} \u{2588} \u{2588} \u{2588}  \u{2588}  \u{2588}\u{2580}\u{2580} \u{2580} \u{2584}",
    "\u{2588}\u{2580}\u{2580} \u{2580}\u{2580}\u{2580} \u{2580} \u{2580} \u{2580}\u{2580}\u{2580} \u{2588}\u{2580}\u{2580}  \u{2580}\u{2580} \u{2580}\u{2580}\u{2580} \u{2580}\u{2580}\u{2580}",
];

/// Columns every wordmark row occupies
pub const WORDMARK_WIDTH: u16 = 31;

/// The tagline drawn under the wordmark
pub const TAGLINE: &str = "the-all-seeing";

/// The wordmark spelled out, for headers too narrow or too short for the art
pub const PLAIN: &str = "PANOPTES";

/// The running version, rendered as `v1.2.3`
pub fn version() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header pads text beside the wordmark by subtracting a constant, so
    /// a row that is not exactly `WORDMARK_WIDTH` wide would skew that text
    #[test]
    fn test_every_wordmark_row_is_the_declared_width() {
        for row in WORDMARK {
            assert_eq!(
                row.chars().count(),
                WORDMARK_WIDTH as usize,
                "row {row:?} is not {WORDMARK_WIDTH} columns"
            );
        }
    }

    /// Half blocks are single-width, so the header can pad by counting chars.
    /// A double-width glyph would draw one column past what was reserved.
    #[test]
    fn test_the_wordmark_holds_no_double_width_glyphs() {
        for row in WORDMARK {
            assert_eq!(
                ratatui::text::Span::raw(row).width(),
                row.chars().count(),
                "row {row:?} renders wider than its character count"
            );
        }
    }

    #[test]
    fn test_version_tracks_the_crate() {
        assert_eq!(version(), format!("v{}", env!("CARGO_PKG_VERSION")));
    }
}
