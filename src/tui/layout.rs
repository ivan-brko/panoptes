//! Screen layout helper
//!
//! Provides a convenient way to create consistent screen layouts with
//! header and optional footer, returning the content area for view-specific rendering.

use ratatui::prelude::*;

use crate::tui::header::{Header, HEADER_HEIGHT};

/// Default footer height (including top border)
pub const DEFAULT_FOOTER_HEIGHT: u16 = 3;

/// Screen layout builder
///
/// Simplifies creating consistent layouts across views by handling
/// header rendering and calculating content areas automatically.
pub struct ScreenLayout<'a> {
    /// Total area for the screen
    area: Rect,
    /// Header to render (optional, but typically present)
    header: Option<Header<'a>>,
    /// Footer height (set to 0 for no footer space)
    footer_height: u16,
}

impl<'a> ScreenLayout<'a> {
    /// Create a new screen layout for the given area
    pub fn new(area: Rect) -> Self {
        Self {
            area,
            header: None,
            footer_height: DEFAULT_FOOTER_HEIGHT,
        }
    }

    /// Add a header to the layout
    pub fn with_header(mut self, header: Header<'a>) -> Self {
        self.header = Some(header);
        self
    }

    /// Set the footer height (default is 3)
    pub fn with_footer_height(mut self, height: u16) -> Self {
        self.footer_height = height;
        self
    }

    /// Render the header (if present) and return layout areas
    ///
    /// Returns a `LayoutAreas` struct with the content and footer areas.
    /// The header is rendered automatically if present.
    pub fn render(self, frame: &mut Frame) -> LayoutAreas {
        let header_height = if self.header.is_some() {
            HEADER_HEIGHT
        } else {
            0
        };

        // Build constraints
        let mut constraints = Vec::new();
        if header_height > 0 {
            constraints.push(Constraint::Length(header_height));
        }
        constraints.push(Constraint::Min(0)); // Content
        if self.footer_height > 0 {
            constraints.push(Constraint::Length(self.footer_height));
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(self.area);

        let mut chunk_idx = 0;

        // Render header if present
        let header_area = if let Some(header) = self.header {
            let area = chunks[chunk_idx];
            header.render(frame, area);
            chunk_idx += 1;
            Some(area)
        } else {
            None
        };

        // Content area
        let content = chunks[chunk_idx];
        chunk_idx += 1;

        // Footer area
        let footer = if self.footer_height > 0 {
            Some(chunks[chunk_idx])
        } else {
            None
        };

        LayoutAreas {
            header: header_area,
            content,
            footer,
        }
    }
}

/// Areas calculated by ScreenLayout
#[derive(Debug, Clone, Copy)]
pub struct LayoutAreas {
    /// Header area (if header was present)
    pub header: Option<Rect>,
    /// Main content area
    pub content: Rect,
    /// Footer area (if footer height > 0)
    pub footer: Option<Rect>,
}

impl LayoutAreas {
    /// Get the content area (main rendering area for the view)
    pub fn content(&self) -> Rect {
        self.content
    }

    /// Get the footer area, panicking if not present
    pub fn footer(&self) -> Rect {
        self.footer.expect("Footer not configured in layout")
    }

    /// Get the footer area if present
    pub fn footer_opt(&self) -> Option<Rect> {
        self.footer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::views::Breadcrumb;

    #[test]
    fn test_layout_areas() {
        let areas = LayoutAreas {
            header: Some(Rect::new(0, 0, 100, 3)),
            content: Rect::new(0, 3, 100, 20),
            footer: Some(Rect::new(0, 23, 100, 3)),
        };

        assert_eq!(areas.content().height, 20);
        assert_eq!(areas.footer().height, 3);
    }

    #[test]
    fn test_layout_no_footer() {
        let areas = LayoutAreas {
            header: Some(Rect::new(0, 0, 100, 3)),
            content: Rect::new(0, 3, 100, 23),
            footer: None,
        };

        assert_eq!(areas.content().height, 23);
        assert!(areas.footer_opt().is_none());
    }

    #[test]
    fn test_screen_layout_creation() {
        let area = Rect::new(0, 0, 100, 30);
        let layout = ScreenLayout::new(area);
        assert!(layout.header.is_none());
        assert_eq!(layout.footer_height, DEFAULT_FOOTER_HEIGHT);
    }

    #[test]
    fn test_screen_layout_with_header() {
        let area = Rect::new(0, 0, 100, 30);
        let breadcrumb = Breadcrumb::new().push("Test");
        let header = Header::new(breadcrumb);
        let layout = ScreenLayout::new(area).with_header(header);
        assert!(layout.header.is_some());
    }

    #[test]
    fn test_screen_layout_custom_footer() {
        let area = Rect::new(0, 0, 100, 30);
        let layout = ScreenLayout::new(area).with_footer_height(5);
        assert_eq!(layout.footer_height, 5);
    }
}
