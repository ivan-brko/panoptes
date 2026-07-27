//! Frame layout and rendering utilities
//!
//! The session view is a header, the agent's screen, and a footer. There is
//! no border around the middle: the header draws its own rule underneath and
//! the footer its own above, which is every line the screen needs, and two
//! rows and two columns that a box would have taken belong to the agent
//! instead.

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::tui::header::{Header, LogoKind};
use crate::tui::views::Breadcrumb;

/// Configuration for frame layout
///
/// Deliberately has no `Default`. A default would have to guess a header
/// height, and every caller that guessed got a different content area than the
/// one on screen — which is the whole bug class this type exists to prevent.
/// Use [`FrameConfig::for_terminal`], or state the heights outright.
#[derive(Debug, Clone)]
pub struct FrameConfig {
    pub header_height: u16,
    pub footer_height: u16,
}

impl FrameConfig {
    /// The config the session view actually renders with on this terminal
    ///
    /// The session header is the wordmark, whose height depends on whether the
    /// terminal can afford the art. Everything that reasons about the session
    /// content area off-screen — PTY sizing, mouse coordinate translation,
    /// how far a page scroll moves — must use this: a header row the layout
    /// math doesn't know about shifts every forwarded mouse click one row
    /// down, clips the PTY's bottom row, and makes a page scroll overshoot.
    pub fn for_terminal(terminal: Rect) -> Self {
        Self {
            header_height: Header::new(Breadcrumb::new())
                .with_logo(LogoKind::Wordmark)
                .height(terminal),
            footer_height: 3,
        }
    }
}

/// Pre-calculated layout areas
///
/// Three, and they tile the terminal exactly: the content runs edge to edge
/// between the header and the footer. There is no fourth "frame" rect, because
/// there is no border to inset it from.
#[derive(Debug, Clone, Copy)]
pub struct FrameLayout {
    pub header: Rect,
    pub content: Rect,
    pub footer: Rect,
}

impl FrameLayout {
    pub fn calculate(terminal_size: Rect, config: &FrameConfig) -> Self {
        let header_height = config.header_height;
        let footer_height = config.footer_height;

        let content_height = terminal_size
            .height
            .saturating_sub(header_height)
            .saturating_sub(footer_height);

        let header = Rect {
            x: terminal_size.x,
            y: terminal_size.y,
            width: terminal_size.width,
            height: header_height,
        };

        // Everything between the header and the footer is the agent's, to the
        // edges. Whatever is inset here is taken off the PTY and shifts every
        // forwarded mouse click, so the two must be the same rectangle - the
        // bug class that put clicks a row off when they disagreed.
        let content = Rect {
            x: terminal_size.x,
            y: terminal_size.y + header_height,
            width: terminal_size.width,
            height: content_height,
        };

        let footer = Rect {
            x: terminal_size.x,
            y: terminal_size.y + header_height + content_height,
            width: terminal_size.width,
            height: footer_height,
        };

        Self {
            header,
            content,
            footer,
        }
    }

    /// Get PTY dimensions (rows, cols)
    pub fn pty_size(&self) -> (u16, u16) {
        (self.content.height, self.content.width)
    }
}

/// Render PTY content
pub fn render_pty_content(
    frame: &mut Frame,
    area: Rect,
    lines: &[Line<'static>],
    cursor_pos: Option<(u16, u16)>,
    cursor_visible: bool,
) {
    let content = Paragraph::new(lines.to_vec());
    frame.render_widget(content, area);

    if cursor_visible {
        if let Some((row, col)) = cursor_pos {
            let screen_x = area.x + col;
            let screen_y = area.y + row;

            if screen_x < area.x + area.width && screen_y < area.y + area.height {
                frame.set_cursor(screen_x, screen_y);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config with the heights stated outright, for the arithmetic tests
    ///
    /// These exercise `FrameLayout::calculate`, not the session view, so they
    /// say what they mean rather than borrowing a real screen's numbers.
    fn config(header_height: u16, footer_height: u16) -> FrameConfig {
        FrameConfig {
            header_height,
            footer_height,
        }
    }

    #[test]
    fn test_frame_layout_calculation() {
        let terminal_size = Rect::new(0, 0, 80, 24);
        let config = config(3, 3);
        let layout = FrameLayout::calculate(terminal_size, &config);

        // Header: y=0, height=3
        assert_eq!(layout.header.y, 0);
        assert_eq!(layout.header.height, 3);

        // Content: straight under the header, edge to edge, 24-3-3 rows
        assert_eq!(layout.content.y, 3);
        assert_eq!(layout.content.height, 18);
        assert_eq!(layout.content.x, 0);
        assert_eq!(layout.content.width, 80);

        // Footer: y=21, height=3
        assert_eq!(layout.footer.y, 21);
        assert_eq!(layout.footer.height, 3);
    }

    /// The three areas tile the terminal with nothing left over
    ///
    /// A gap between them would be a row nobody draws, and an overlap would be
    /// a row drawn twice. Either way the PTY and the screen would disagree
    /// about where a cell is, which is what puts a forwarded click on the
    /// wrong line.
    #[test]
    fn test_the_areas_tile_the_terminal_exactly() {
        for (w, h) in [(80, 24), (120, 40), (40, 12)] {
            let terminal = Rect::new(0, 0, w, h);
            let layout = FrameLayout::calculate(terminal, &FrameConfig::for_terminal(terminal));

            assert_eq!(layout.header.y, terminal.y, "{w}x{h}");
            assert_eq!(
                layout.header.y + layout.header.height,
                layout.content.y,
                "gap or overlap above the content at {w}x{h}"
            );
            assert_eq!(
                layout.content.y + layout.content.height,
                layout.footer.y,
                "gap or overlap below the content at {w}x{h}"
            );
            assert_eq!(
                layout.footer.y + layout.footer.height,
                terminal.y + terminal.height,
                "{w}x{h}"
            );
            // Edge to edge: no border to inset from
            assert_eq!(layout.content.x, terminal.x, "{w}x{h}");
            assert_eq!(layout.content.width, terminal.width, "{w}x{h}");
        }
    }

    /// The regression behind this constructor: mouse translation used
    /// `default()` (3-row header) while the session view rendered a 4-row
    /// wordmark header, so every click forwarded to the agent landed one row
    /// below the pointer.
    #[test]
    fn test_for_terminal_matches_the_wordmark_header() {
        // Room for the wordmark: header is its 3 art rows plus the border row
        let wide = Rect::new(0, 0, 120, 40);
        assert_eq!(FrameConfig::for_terminal(wide).header_height, 4);

        // Too small for the art: falls back to the one-line header's 3 rows
        let tiny = Rect::new(0, 0, 20, 10);
        assert_eq!(FrameConfig::for_terminal(tiny).header_height, 3);
    }

    #[test]
    fn test_pty_size() {
        let terminal_size = Rect::new(0, 0, 80, 24);
        let config = config(3, 3);
        let layout = FrameLayout::calculate(terminal_size, &config);
        let (rows, cols) = layout.pty_size();

        // The PTY gets the whole content area, borders being gone
        assert_eq!(rows, 18);
        assert_eq!(cols, 80);
    }

    /// What a page scroll, a new PTY and a mouse click all have to agree on
    ///
    /// The guessed 3-row header was one row short of the wordmark the session
    /// view actually draws, so anything sized from it was one row too tall -
    /// a page scroll by that height stepped past a line of output every time.
    #[test]
    fn test_a_page_is_the_content_the_session_view_renders() {
        let terminal = Rect::new(0, 0, 120, 40);
        let rendered = FrameLayout::calculate(terminal, &FrameConfig::for_terminal(terminal));

        // 4-row wordmark header and 3-row footer; everything else is the agent's
        assert_eq!(rendered.content.height, 40 - 4 - 3);

        // The height the old guess produced, kept here to name the gap rather
        // than leave it to arithmetic in a reader's head
        let guessed = FrameLayout::calculate(terminal, &config(3, 3));
        assert_eq!(
            guessed.content.height,
            rendered.content.height + 1,
            "a guessed header height must not be mistaken for the real one"
        );
    }
}
