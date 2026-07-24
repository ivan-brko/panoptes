//! Frame layout and rendering utilities
//!
//! This module provides pre-calculated frame layouts and rendering functions
//! for the session view. Based on the proven claude-wrapper architecture.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::tui::header::{Header, LogoKind};
use crate::tui::views::Breadcrumb;

/// Configuration for frame layout
#[derive(Debug, Clone)]
pub struct FrameConfig {
    pub header_height: u16,
    pub footer_height: u16,
    pub title: Option<String>,
}

impl Default for FrameConfig {
    fn default() -> Self {
        Self {
            header_height: 3, // Match existing Panoptes header
            footer_height: 3, // Match existing Panoptes footer
            title: None,
        }
    }
}

impl FrameConfig {
    /// The config the session view actually renders with on this terminal
    ///
    /// The session header is the wordmark, whose height depends on whether the
    /// terminal can afford the art. Everything that reasons about the session
    /// content area off-screen — PTY sizing, mouse coordinate translation —
    /// must use this, not `default()`: a header row the layout math doesn't
    /// know about shifts every forwarded mouse click one row down and clips
    /// the PTY's bottom row.
    pub fn for_terminal(terminal: Rect) -> Self {
        Self {
            header_height: Header::new(Breadcrumb::new())
                .with_logo(LogoKind::Wordmark)
                .height(terminal),
            footer_height: 3,
            title: None,
        }
    }
}

/// Pre-calculated layout areas
#[derive(Debug, Clone, Copy)]
pub struct FrameLayout {
    pub header: Rect,
    pub frame: Rect,
    pub content: Rect, // Inside frame border
    pub footer: Rect,
}

impl FrameLayout {
    pub fn calculate(terminal_size: Rect, config: &FrameConfig) -> Self {
        let header_height = config.header_height;
        let footer_height = config.footer_height;

        let frame_height = terminal_size
            .height
            .saturating_sub(header_height)
            .saturating_sub(footer_height);

        let header = Rect {
            x: terminal_size.x,
            y: terminal_size.y,
            width: terminal_size.width,
            height: header_height,
        };

        let frame = Rect {
            x: terminal_size.x,
            y: terminal_size.y + header_height,
            width: terminal_size.width,
            height: frame_height,
        };

        // Content is inside the frame border (1 char on each side)
        let content = Rect {
            x: frame.x + 1,
            y: frame.y + 1,
            width: frame.width.saturating_sub(2),
            height: frame_height.saturating_sub(2),
        };

        let footer = Rect {
            x: terminal_size.x,
            y: terminal_size.y + header_height + frame_height,
            width: terminal_size.width,
            height: footer_height,
        };

        Self {
            header,
            frame,
            content,
            footer,
        }
    }

    /// Get PTY dimensions (rows, cols)
    pub fn pty_size(&self) -> (u16, u16) {
        (self.content.height, self.content.width)
    }
}

/// Render frame border separately from content
pub fn render_frame_border(
    frame: &mut Frame,
    area: Rect,
    border_color: Color,
    title: Option<&str>,
) {
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    if let Some(title) = title {
        block = block.title(title);
    }

    frame.render_widget(block, area);
}

/// Render PTY content inside the frame
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

    #[test]
    fn test_frame_layout_calculation() {
        let terminal_size = Rect::new(0, 0, 80, 24);
        let config = FrameConfig::default();
        let layout = FrameLayout::calculate(terminal_size, &config);

        // Header: y=0, height=3
        assert_eq!(layout.header.y, 0);
        assert_eq!(layout.header.height, 3);

        // Frame: y=3, height=18 (24-3-3)
        assert_eq!(layout.frame.y, 3);
        assert_eq!(layout.frame.height, 18);

        // Content: y=4 (frame.y + 1), height=16 (18-2)
        assert_eq!(layout.content.y, 4);
        assert_eq!(layout.content.height, 16);
        assert_eq!(layout.content.x, 1);
        assert_eq!(layout.content.width, 78);

        // Footer: y=21, height=3
        assert_eq!(layout.footer.y, 21);
        assert_eq!(layout.footer.height, 3);
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
        let config = FrameConfig::default();
        let layout = FrameLayout::calculate(terminal_size, &config);
        let (rows, cols) = layout.pty_size();

        // Content area dimensions
        assert_eq!(rows, 16);
        assert_eq!(cols, 78);
    }
}
