//! Centered dialog helpers
//!
//! Every overlay dialog in the application computes the same centered
//! rectangle, clears it, and draws a bordered paragraph into it. This module
//! implements that once: [`centered_rect`] for the geometry (two sizing
//! conventions, see [`DialogSize`]) and [`render_dialog`] for the common
//! clear-then-paragraph case. Dialogs with richer content (lists, split
//! layouts) use [`centered_rect`] directly.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::tui::theme::theme;

/// How a dialog dimension is sized within the available area
#[derive(Debug, Clone, Copy)]
pub enum DialogSize {
    /// Fixed size, capped so the dialog keeps a margin inside the area
    /// (4 columns horizontally, 2 rows vertically)
    Fixed(u16),
    /// Percentage of the area dimension, clamped to `min..=max`
    Percent { pct: u16, min: u16, max: u16 },
}

impl DialogSize {
    /// Resolve to a concrete dimension that fits inside `available`
    ///
    /// The result never exceeds the area, whichever variant is used and
    /// whatever `min` asks for. A `Percent` whose `min` is wider than the
    /// terminal used to win, and the resulting rect reached past the buffer -
    /// which `Clear` does not clip, so it panicked rather than overflowing
    /// quietly. `min` is a preference; the terminal is not.
    fn resolve(self, available: u16, margin: u16) -> u16 {
        let room = available.saturating_sub(margin);
        match self {
            DialogSize::Fixed(size) => size.min(room),
            DialogSize::Percent { pct, min, max } => ((available as u32 * pct as u32 / 100) as u16)
                .clamp(min, max)
                .min(room),
        }
    }
}

/// Compute a centered dialog rectangle inside `area`
pub fn centered_rect(area: Rect, width: DialogSize, height: DialogSize) -> Rect {
    let w = width.resolve(area.width, 4);
    let h = height.resolve(area.height, 2);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

/// Frame and styling for a standard overlay dialog
pub struct DialogSpec<'a> {
    /// Title shown in the border
    pub title: &'a str,
    /// Border color
    pub border_color: Color,
    /// Text alignment for the content
    pub alignment: Alignment,
    /// Dialog width
    pub width: DialogSize,
    /// Dialog height
    pub height: DialogSize,
}

/// Render a centered overlay dialog: clear the area, draw border and lines
pub fn render_dialog(frame: &mut Frame, area: Rect, spec: DialogSpec, lines: Vec<Line>) {
    let dialog_area = centered_rect(area, spec.width, spec.height);
    frame.render_widget(Clear, dialog_area);

    let paragraph = Paragraph::new(lines).alignment(spec.alignment).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(spec.border_color))
            .title(spec.title.to_string()),
    );
    frame.render_widget(paragraph, dialog_area);
}

/// A `Yes` / `No` button pair, highlighting the selected side
///
/// `lead` is prepended raw, letting left-aligned dialogs indent the pair.
pub fn yes_no_line(selected_yes: bool, lead: &'static str) -> Line<'static> {
    let t = theme();

    let yes_style = if selected_yes {
        Style::default()
            .fg(Color::Black)
            .bg(t.confirm_key)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.confirm_key)
    };

    let no_style = if !selected_yes {
        Style::default()
            .fg(Color::Black)
            .bg(t.cancel_key)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.cancel_key)
    };

    Line::from(vec![
        Span::raw(lead),
        Span::styled(" Yes ", yes_style),
        Span::raw("    "),
        Span::styled(" No ", no_style),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixed_rect_is_centered() {
        let area = Rect::new(0, 0, 80, 24);
        let rect = centered_rect(area, DialogSize::Fixed(40), DialogSize::Fixed(10));

        assert_eq!(rect, Rect::new(20, 7, 40, 10));
    }

    #[test]
    fn test_fixed_rect_caps_to_area_with_margin() {
        let area = Rect::new(0, 0, 30, 8);
        let rect = centered_rect(area, DialogSize::Fixed(50), DialogSize::Fixed(20));

        // Width keeps a 4-column margin, height a 2-row margin
        assert_eq!(rect.width, 26);
        assert_eq!(rect.height, 6);
    }

    #[test]
    fn test_fixed_rect_respects_area_offset() {
        let area = Rect::new(10, 5, 40, 20);
        let rect = centered_rect(area, DialogSize::Fixed(20), DialogSize::Fixed(10));

        assert_eq!(rect, Rect::new(20, 10, 20, 10));
    }

    #[test]
    fn test_percent_rect_clamps_to_min_and_max() {
        let area = Rect::new(0, 0, 200, 100);
        let size = DialogSize::Percent {
            pct: 60,
            min: 40,
            max: 60,
        };
        // 60% of 200 = 120, clamped to max 60
        assert_eq!(centered_rect(area, size, size).width, 60);

        let small = Rect::new(0, 0, 50, 100);
        // 60% of 50 = 30, clamped to min 40
        assert_eq!(centered_rect(small, size, size).width, 40);
    }

    /// A dialog that does not fit is shrunk, never drawn past the buffer:
    /// `Clear` does not clip, so an oversized rect is a panic, not a smudge
    #[test]
    fn test_rect_never_escapes_a_tiny_area() {
        let percent = DialogSize::Percent {
            pct: 70,
            min: 40,
            max: 80,
        };
        for width in 0..=90u16 {
            for height in [0u16, 1, 3, 8, 24] {
                let area = Rect::new(0, 0, width, height);
                for size in [percent, DialogSize::Fixed(60)] {
                    let rect = centered_rect(area, size, size);
                    assert!(
                        rect.x + rect.width <= area.x + area.width,
                        "{rect:?} escapes {area:?}"
                    );
                    assert!(
                        rect.y + rect.height <= area.y + area.height,
                        "{rect:?} escapes {area:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_yes_no_line_highlights_selected_side() {
        let t = theme();

        let yes = yes_no_line(true, "");
        assert_eq!(yes.spans[1].style.bg, Some(t.confirm_key));
        assert_eq!(yes.spans[3].style.bg, None);

        let no = yes_no_line(false, "");
        assert_eq!(no.spans[1].style.bg, None);
        assert_eq!(no.spans[3].style.bg, Some(t.cancel_key));
    }
}
