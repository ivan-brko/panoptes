//! Frame rendering module
//!
//! This module handles rendering the frame (border, header, footer) around
//! the PTY content area.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Configuration for the wrapper frame
#[derive(Debug, Clone)]
pub struct FrameConfig {
    /// Number of lines reserved for header area (above the frame)
    pub header_height: u16,
    /// Number of lines reserved for footer area (below the frame)
    pub footer_height: u16,
    /// Optional title for the frame border
    pub title: Option<String>,
}

impl Default for FrameConfig {
    fn default() -> Self {
        Self {
            header_height: 1,
            footer_height: 1,
            title: None,
        }
    }
}

impl FrameConfig {
    /// Create a new frame config with specified heights
    pub fn new(header_height: u16, footer_height: u16) -> Self {
        Self {
            header_height,
            footer_height,
            title: None,
        }
    }

    /// Set the frame title
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}

/// Layout areas calculated from the frame config and terminal size
#[derive(Debug, Clone, Copy)]
pub struct FrameLayout {
    /// Area for the header (above the frame)
    pub header: Rect,
    /// Area for the frame border and content
    pub frame: Rect,
    /// Area for the PTY content (inside the frame border)
    pub content: Rect,
    /// Area for the footer (below the frame)
    pub footer: Rect,
}

impl FrameLayout {
    /// Calculate layout areas from terminal size and frame config
    pub fn calculate(terminal_size: Rect, config: &FrameConfig) -> Self {
        let header_height = config.header_height;
        let footer_height = config.footer_height;

        // Calculate frame area (between header and footer)
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

    /// Get the dimensions for the PTY (rows, cols)
    pub fn pty_size(&self) -> (u16, u16) {
        (self.content.height, self.content.width)
    }
}

/// Render the frame border
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
    // Create a paragraph from the lines
    let content = Paragraph::new(lines.to_vec());
    frame.render_widget(content, area);

    // Optionally show cursor
    if cursor_visible {
        if let Some((row, col)) = cursor_pos {
            // Translate cursor position to screen coordinates
            let screen_x = area.x + col;
            let screen_y = area.y + row;

            // Only set cursor if within bounds
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
        let terminal = Rect::new(0, 0, 80, 24);
        let config = FrameConfig::new(1, 1);
        let layout = FrameLayout::calculate(terminal, &config);

        assert_eq!(layout.header, Rect::new(0, 0, 80, 1));
        assert_eq!(layout.frame, Rect::new(0, 1, 80, 22));
        assert_eq!(layout.content, Rect::new(1, 2, 78, 20));
        assert_eq!(layout.footer, Rect::new(0, 23, 80, 1));
    }

    #[test]
    fn test_pty_size() {
        let terminal = Rect::new(0, 0, 80, 24);
        let config = FrameConfig::new(1, 1);
        let layout = FrameLayout::calculate(terminal, &config);

        let (rows, cols) = layout.pty_size();
        assert_eq!(rows, 20);
        assert_eq!(cols, 78);
    }

    #[test]
    fn test_frame_layout_no_header_footer() {
        let terminal = Rect::new(0, 0, 80, 24);
        let config = FrameConfig::new(0, 0);
        let layout = FrameLayout::calculate(terminal, &config);

        assert_eq!(layout.header.height, 0);
        assert_eq!(layout.footer.height, 0);
        assert_eq!(layout.frame.height, 24);
        assert_eq!(layout.content.height, 22);
    }
}
