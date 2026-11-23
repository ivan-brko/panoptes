//! The detail row under an account, fitted to its width by dropping fields
//!
//! Shared by the Codex accounts ([`super::codex_account`]) and the Claude
//! profiles ([`super::claude_account`]): both draw one row of fields under
//! each account's name, and both drop fields whole as the row narrows rather
//! than truncating one long string.

use ratatui::prelude::*;

use crate::tui::theme::theme;

/// Indent that puts the row under the account's name, past the selection prefix
pub const DETAIL_INDENT: &str = "    ";

/// Separator between fields
const SEP: &str = " · ";

/// One piece of the row, and how long it holds on as the row narrows
pub(crate) struct Field {
    pub spans: Vec<Span<'static>>,
    /// Higher holds on longer
    pub keep: u8,
    /// Joined to the field before it by a space rather than a separator
    ///
    /// A window's reset time is part of that window, not a field of its own.
    /// It always gives way before its window does, so it is never left
    /// joined to the wrong one.
    pub joined: bool,
}

impl Field {
    pub fn text(text: impl Into<String>, style: Style, keep: u8) -> Self {
        Self {
            spans: vec![Span::styled(text.into(), style)],
            keep,
            joined: false,
        }
    }

    fn width(&self) -> usize {
        self.spans.iter().map(|s| s.content.chars().count()).sum()
    }
}

/// Columns a row of `fields` takes, indent and joins included
fn row_width(fields: &[Field]) -> usize {
    let joins: usize = fields
        .iter()
        .skip(1)
        .map(|f| if f.joined { 1 } else { SEP.chars().count() })
        .sum();
    DETAIL_INDENT.len() + fields.iter().map(Field::width).sum::<usize>() + joins
}

/// Drop fields, least useful first, until the row fits; then lay it out
pub(crate) fn fit(mut fields: Vec<Field>, width: usize) -> Line<'static> {
    while fields.len() > 1 && row_width(&fields) > width {
        // The weakest field; the later one of equals, so the row keeps its start
        let weakest = fields
            .iter()
            .enumerate()
            .min_by_key(|(i, f)| (f.keep, std::cmp::Reverse(*i)))
            .map(|(i, _)| i)
            .unwrap_or(0);
        fields.remove(weakest);
    }

    let muted = theme().muted_style();
    let mut spans = vec![Span::raw(DETAIL_INDENT)];
    for (i, field) in fields.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(if field.joined { " " } else { SEP }, muted));
        }
        spans.extend(field.spans);
    }
    crate::tui::views::pane_projects::clamp_line(Line::from(spans), width)
}
