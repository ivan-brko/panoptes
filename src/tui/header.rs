//! Unified header component
//!
//! Provides a consistent header across all views with:
//! - The wordmark, top-left, at whichever size the caller asked for
//! - Breadcrumb navigation, beside the wordmark
//! - Header notifications (shown under the breadcrumb)
//! - Attention indicator (blinking when sessions need attention)
//!
//! The wordmark is the app's name, so no view repeats it in text. Everything
//! else is laid out around it: the header reserves [`logo::WORDMARK_WIDTH`]
//! columns on the left and fills the rest by arithmetic.
//!
//! A header that cannot afford the art - too narrow for the wordmark, or on a
//! terminal short enough that four rows of branding would crowd out the
//! content - collapses to the one-line form the app used before, so the
//! smallest terminals lose the logo rather than the view.

use std::time::Instant;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::tui::header_notifications::HeaderNotificationManager;
use crate::tui::logo;
use crate::tui::theme::theme;
use crate::tui::views::{truncate_string, Breadcrumb};

/// Columns between the wordmark and whatever is drawn beside it
const LOGO_GAP: u16 = 2;

/// How much of the mark a header wears
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoKind {
    /// No art - the one-line header
    None,
    /// The wordmark alone, three rows
    Wordmark,
    /// The wordmark over the tagline and version, four rows
    Full,
}

impl LogoKind {
    /// Rows of art this kind draws, before the bottom border
    fn art_rows(self) -> u16 {
        match self {
            LogoKind::None => 0,
            LogoKind::Wordmark => 3,
            LogoKind::Full => 4,
        }
    }

    /// Shortest terminal this kind is worth drawing on
    ///
    /// Below this the header would take a quarter of the screen to say
    /// something the user already knows, so the art is dropped instead.
    fn min_terminal_height(self) -> u16 {
        match self {
            LogoKind::None => 0,
            LogoKind::Wordmark => 12,
            LogoKind::Full => 14,
        }
    }
}

/// Start time for blinking calculation
static START_TIME: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn get_start_time() -> Instant {
    *START_TIME.get_or_init(Instant::now)
}

/// Unified header component for all views
pub struct Header<'a> {
    /// Breadcrumb navigation path
    breadcrumb: Breadcrumb,
    /// Optional suffix text (e.g., "(3 active, 2 need attention)")
    suffix: Option<String>,
    /// Header notifications manager
    notifications: Option<&'a HeaderNotificationManager>,
    /// Number of sessions needing attention
    attention_count: usize,
    /// Optional custom style (for session view state-based coloring)
    custom_style: Option<Style>,
    /// How much of the wordmark this header draws
    logo: LogoKind,
}

impl<'a> Header<'a> {
    /// Create a new header with the given breadcrumb
    pub fn new(breadcrumb: Breadcrumb) -> Self {
        Self {
            breadcrumb,
            suffix: None,
            notifications: None,
            attention_count: 0,
            custom_style: None,
            logo: LogoKind::None,
        }
    }

    /// Choose how much of the wordmark this header draws
    pub fn with_logo(mut self, logo: LogoKind) -> Self {
        self.logo = logo;
        self
    }

    /// Rows this header needs in `area`, bottom border included
    ///
    /// The caller lays the screen out before rendering, so this has to answer
    /// the same question `render` will: the two disagreeing would leave a
    /// stripe of the header drawn over the content, or a gap above it.
    pub fn height(&self, terminal: Rect) -> u16 {
        match self.affordable_logo(terminal) {
            LogoKind::None => HEADER_HEIGHT,
            kind => kind.art_rows() + 1,
        }
    }

    /// The logo kind a terminal of this size can afford
    ///
    /// Takes the whole terminal, not the header's slice of it: the question is
    /// how much branding the screen has room for.
    fn affordable_logo(&self, terminal: Rect) -> LogoKind {
        if terminal.width < logo::WORDMARK_WIDTH
            || terminal.height < self.logo.min_terminal_height()
        {
            LogoKind::None
        } else {
            self.logo
        }
    }

    /// The logo kind that fits the rows this header was actually handed
    ///
    /// `height` and `render` are called with different rects - the terminal
    /// and the header's own slice - so this re-checks rather than trusting
    /// that the caller sized the slice from the same answer.
    fn rendered_logo(&self, area: Rect) -> LogoKind {
        if area.width < logo::WORDMARK_WIDTH || area.height < self.logo.art_rows() + 1 {
            LogoKind::None
        } else {
            self.logo
        }
    }

    /// Add a suffix to the header (e.g., status counts)
    pub fn with_suffix(mut self, suffix: impl Into<String>) -> Self {
        let s = suffix.into();
        if !s.is_empty() {
            self.suffix = Some(s);
        }
        self
    }

    /// Add header notifications
    pub fn with_notifications(
        mut self,
        notifications: Option<&'a HeaderNotificationManager>,
    ) -> Self {
        self.notifications = notifications;
        self
    }

    /// Set the attention count for blinking indicator
    pub fn with_attention_count(mut self, count: usize) -> Self {
        self.attention_count = count;
        self
    }

    /// Set a custom style (overrides default header style)
    pub fn with_custom_style(mut self, style: Style) -> Self {
        self.custom_style = Some(style);
        self
    }

    /// Render the header to the given area
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let t = theme();
        let kind = self.rendered_logo(area);
        let width = area.width.saturating_sub(2) as usize; // Account for borders

        let lines = match kind {
            LogoKind::None => vec![self.plain_line(width)],
            _ => self.logo_lines(kind, width),
        };

        // The custom style tints the breadcrumb, not the wordmark: a session's
        // state colour is a signal, and a mark that changes colour with it
        // stops reading as the app's name
        let style = if kind == LogoKind::None {
            self.custom_style.unwrap_or_else(|| t.header_style())
        } else {
            t.header_style()
        };

        let paragraph = Paragraph::new(lines)
            .style(style)
            .block(Block::default().borders(Borders::BOTTOM));

        frame.render_widget(paragraph, area);
    }

    /// The one-line header, for terminals with no room for the art
    ///
    /// The wordmark is the only thing naming the app, so when it is dropped
    /// its spelled-out form takes over - unless there is a breadcrumb, which
    /// says something the user cannot get anywhere else on the screen.
    fn plain_line(&self, width: usize) -> Line<'static> {
        let breadcrumb = self.breadcrumb_text();
        let left = if breadcrumb.is_empty() {
            logo::PLAIN.to_string()
        } else {
            breadcrumb
        };

        let mut left_spans = vec![Span::raw(left)];
        if let Some(notification) = self.notification_span() {
            left_spans.push(Span::raw(" | "));
            left_spans.push(notification);
        }
        let left_len: usize = left_spans.iter().map(|s| s.content.chars().count()).sum();

        let badge = self.badge_spans();
        let badge_len: usize = badge.iter().map(|s| s.content.chars().count()).sum();

        let mut spans = left_spans;
        spans.push(Span::raw(
            " ".repeat(width.saturating_sub(left_len + badge_len)),
        ));
        spans.extend(badge);
        Line::from(spans)
    }

    /// The rows of a header wearing the wordmark
    ///
    /// The art owns a fixed column band on the left and everything else is
    /// right-aligned, so each row is a pairing across the header: the badge on
    /// the first, the breadcrumb on the second, a notification on the third,
    /// and - on the full mark - the tagline and version on a fourth row.
    fn logo_lines(&self, kind: LogoKind, width: usize) -> Vec<Line<'static>> {
        let t = theme();
        let art = Style::default().fg(t.accent).bold();
        let muted = Style::default().fg(t.text_muted);
        let breadcrumb_style = self.custom_style.unwrap_or_else(|| t.header_style());

        let breadcrumb = self.breadcrumb_text();
        let breadcrumb =
            (!breadcrumb.is_empty()).then(|| Span::styled(breadcrumb, breadcrumb_style));

        let mut lines = vec![
            Self::row(
                width,
                Span::styled(logo::WORDMARK[0], art),
                None,
                self.badge_spans(),
            ),
            Self::row(
                width,
                Span::styled(logo::WORDMARK[1], art),
                breadcrumb,
                vec![],
            ),
            Self::row(
                width,
                Span::styled(logo::WORDMARK[2], art),
                self.notification_span(),
                vec![],
            ),
        ];

        if kind == LogoKind::Full {
            lines.push(Self::row(
                width,
                Span::styled(logo::TAGLINE, muted),
                None,
                vec![Span::styled(logo::version(), muted)],
            ));
        }

        lines
    }

    /// One header row: `left` in the wordmark's column band, then `mid` and
    /// `right` both flush to the far edge, the whole thing clipped to `width`
    ///
    /// `left` is padded out to the wordmark's width even when it is shorter
    /// than that - the tagline is - so every row's text starts in the same
    /// column. Everything else is right-aligned: the header is the art on one
    /// side and the state of the app on the other, not a caption trailing the
    /// wordmark.
    fn row(
        width: usize,
        left: Span<'static>,
        mid: Option<Span<'static>>,
        right: Vec<Span<'static>>,
    ) -> Line<'static> {
        let band = logo::WORDMARK_WIDTH as usize;
        let left_len = left.content.chars().count();
        let right_len: usize = right.iter().map(|s| s.content.chars().count()).sum();

        let mut spans = vec![left];

        // Everything after the band is optional, and a header narrow enough
        // that the band fills it keeps the art and drops the rest
        let after_band = width.saturating_sub(band + LOGO_GAP as usize);
        if after_band == 0 {
            return Line::from(spans);
        }
        spans.push(Span::raw(
            " ".repeat(band.saturating_sub(left_len) + LOGO_GAP as usize),
        ));

        // The mid span keeps the style it arrived with; only its text is cut
        let mid_room = after_band.saturating_sub(right_len);
        let mid_text = mid.and_then(|span| {
            let content = truncate_string(&span.content, mid_room);
            (!content.is_empty()).then(|| Span::styled(content, span.style))
        });

        let mid_len = mid_text.as_ref().map_or(0, |s| s.content.chars().count());

        // The pad goes ahead of the text, not after it, so the row ends flush
        // with the pane borders below rather than starting flush with the art
        if right_len <= after_band {
            spans.push(Span::raw(" ".repeat(after_band - mid_len - right_len)));
            if let Some(span) = mid_text {
                spans.push(span);
            }
            spans.extend(right);
        }

        Line::from(spans)
    }

    /// The breadcrumb, with its suffix appended when there is one
    fn breadcrumb_text(&self) -> String {
        match &self.suffix {
            Some(suffix) => self.breadcrumb.display_with_suffix(suffix),
            None => self.breadcrumb.display(),
        }
    }

    /// The current notification, red when it is an error
    ///
    /// A persistent notification is an error the user has to act on, so it
    /// outranks whatever transient message happens to be showing.
    fn notification_span(&self) -> Option<Span<'static>> {
        let t = theme();
        let (msg, is_error) = self.notifications.and_then(|n| match n.persistent() {
            Some(msg) => Some((msg.to_string(), true)),
            None => n.current_message().map(|msg| (msg.to_string(), false)),
        })?;

        let style = if is_error {
            Style::default().fg(t.state_exited).bold()
        } else {
            Style::default()
        };
        Some(Span::styled(msg, style))
    }

    /// The blinking attention badge, empty when nothing needs attention
    fn badge_spans(&self) -> Vec<Span<'static>> {
        if self.attention_count == 0 {
            return Vec::new();
        }
        let t = theme();
        // The count stays put through the blink-off phase so it does not jitter
        let (text, style) = if Self::should_show_blink() {
            (
                format!("[{}\u{25CF}]", self.attention_count), // [N●]
                Style::default().fg(t.attention_badge).bold(),
            )
        } else {
            (
                format!("[{}]", self.attention_count),
                Style::default().fg(t.attention_badge),
            )
        };
        vec![Span::styled(text, style), Span::raw(" ")]
    }

    /// Check if the blink indicator should be visible (500ms on/off cycle)
    fn should_show_blink() -> bool {
        let elapsed = get_start_time().elapsed();
        (elapsed.as_millis() % 1000) < 500
    }
}

/// Height of the header without the wordmark (including bottom border)
pub const HEADER_HEIGHT: u16 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_creation() {
        let breadcrumb = Breadcrumb::new().push("Projects");
        let header = Header::new(breadcrumb);
        assert!(header.suffix.is_none());
        assert_eq!(header.attention_count, 0);
    }

    #[test]
    fn test_header_with_suffix() {
        let breadcrumb = Breadcrumb::new().push("Projects");
        let header = Header::new(breadcrumb).with_suffix("(3 active)");
        assert_eq!(header.suffix, Some("(3 active)".to_string()));
    }

    #[test]
    fn test_header_with_attention() {
        let breadcrumb = Breadcrumb::new().push("Projects");
        let header = Header::new(breadcrumb).with_attention_count(5);
        assert_eq!(header.attention_count, 5);
    }

    fn area(width: u16, height: u16) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    #[test]
    fn test_height_covers_the_art_and_its_border() {
        let full = Header::new(Breadcrumb::new()).with_logo(LogoKind::Full);
        assert_eq!(full.height(area(120, 30)), 5);

        let wordmark = Header::new(Breadcrumb::new()).with_logo(LogoKind::Wordmark);
        assert_eq!(wordmark.height(area(120, 30)), 4);

        let none = Header::new(Breadcrumb::new());
        assert_eq!(none.height(area(120, 30)), HEADER_HEIGHT);
    }

    /// A header too narrow or too short for the art falls back to the
    /// one-line form, which is the height the rest of the app grew up with
    #[test]
    fn test_height_falls_back_when_the_art_does_not_fit() {
        let full = Header::new(Breadcrumb::new()).with_logo(LogoKind::Full);
        assert_eq!(
            full.height(area(logo::WORDMARK_WIDTH - 1, 40)),
            HEADER_HEIGHT
        );
        assert_eq!(full.height(area(200, 13)), HEADER_HEIGHT);

        // The wordmark alone is worth drawing on a terminal too short for the
        // full mark, so the two thresholds are not the same
        let wordmark = Header::new(Breadcrumb::new()).with_logo(LogoKind::Wordmark);
        assert_eq!(wordmark.height(area(200, 13)), 4);
    }

    /// `height` sizes the header from the terminal and `render` from the slice
    /// it was handed. A slice smaller than the art has to fall back too, or
    /// the wordmark would be drawn over the content below it.
    #[test]
    fn test_a_slice_too_small_for_the_art_falls_back() {
        let full = Header::new(Breadcrumb::new()).with_logo(LogoKind::Full);
        assert_eq!(full.rendered_logo(area(120, 5)), LogoKind::Full);
        assert_eq!(full.rendered_logo(area(120, 4)), LogoKind::None);
        assert_eq!(
            full.rendered_logo(area(logo::WORDMARK_WIDTH - 1, 5)),
            LogoKind::None
        );
    }

    /// With the root segment gone the breadcrumb can be empty, and a header
    /// showing neither art nor a name would say nothing at all
    #[test]
    fn test_the_plain_line_names_the_app_when_there_is_no_breadcrumb() {
        let header = Header::new(Breadcrumb::new());
        let line = header.plain_line(40);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with(logo::PLAIN), "{text:?}");

        // A breadcrumb says where you are, which the wordmark cannot
        let header = Header::new(Breadcrumb::new().push("proj").push("main"));
        let line = header.plain_line(40);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("proj > main"), "{text:?}");
    }

    /// The wordmark owns the left, everything else is flush right - and that
    /// holds whatever the left span is, since the tagline is shorter than the art
    #[test]
    fn test_rows_right_align_the_text_beside_the_wordmark() {
        let art = Header::row(
            80,
            Span::raw(logo::WORDMARK[1]),
            Some(Span::raw("beside")),
            vec![],
        );
        let tagline = Header::row(
            80,
            Span::raw(logo::TAGLINE),
            Some(Span::raw("beside")),
            vec![],
        );

        for line in [art, tagline] {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert_eq!(text.chars().count(), 80, "{text:?}");
            assert!(text.ends_with("beside"), "{text:?}");
        }
    }

    /// A row carrying both keeps the pinned span outermost, with the message
    /// tucked just inside it - neither drifts back to the wordmark
    #[test]
    fn test_a_pinned_span_stays_outside_the_message() {
        let line = Header::row(
            80,
            Span::raw(logo::WORDMARK[0]),
            Some(Span::raw("breadcrumb")),
            vec![Span::raw("[2\u{25CF}]")],
        );
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text.chars().count(), 80, "{text:?}");
        assert!(text.ends_with("breadcrumb[2\u{25CF}]"), "{text:?}");
    }

    /// The band is fixed, so a header only just wider than it has nothing left
    /// for text - and must not paper over that with a negative-width pad
    #[test]
    fn test_a_row_with_no_room_past_the_band_keeps_only_the_art() {
        let line = Header::row(
            logo::WORDMARK_WIDTH as usize,
            Span::raw(logo::WORDMARK[0]),
            Some(Span::raw("dropped")),
            vec![Span::raw("[1]")],
        );
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, logo::WORDMARK[0]);
    }

    /// A message longer than the room beside the wordmark is cut, not wrapped
    /// onto the row below - which holds the next row of the art
    #[test]
    fn test_a_long_message_is_cut_to_the_room_beside_the_wordmark() {
        let width = logo::WORDMARK_WIDTH as usize + LOGO_GAP as usize + 10;
        let line = Header::row(
            width,
            Span::raw(logo::WORDMARK[2]),
            Some(Span::raw("a message far longer than ten columns")),
            vec![],
        );
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text.chars().count(), width, "{text:?}");
    }

    #[test]
    fn test_blink_cycle() {
        // Verify should_show_blink returns a deterministic value based on time
        // The function uses a 1-second cycle with 500ms on and 500ms off
        let result1 = Header::should_show_blink();
        let result2 = Header::should_show_blink();
        // Two immediate calls should return the same value (time hasn't changed significantly)
        assert_eq!(result1, result2);
    }
}
