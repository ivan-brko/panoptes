//! Agent type selector dialog
//!
//! Shown when creating a new AI session to choose which agent runs it.

use ratatui::prelude::*;

use crate::tui::theme::theme;
use crate::tui::widgets::dialog::{render_dialog, DialogSize, DialogSpec};
use crate::tui::widgets::selection::{selection_name_style, selection_prefix};

/// Render the agent type selector dialog
pub fn render_agent_type_selector(frame: &mut Frame, area: Rect, selected_index: usize) {
    let t = theme();

    let agents = ["Claude Code", "Codex"];

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Select agent type:",
            Style::default().fg(t.text),
        )),
        Line::from(""),
    ];

    for (i, agent) in agents.iter().enumerate() {
        let is_selected = i == selected_index;
        lines.push(Line::from(vec![
            Span::raw(selection_prefix(is_selected)),
            Span::styled(*agent, selection_name_style(is_selected, t)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "[Enter] Select  [Esc] Cancel",
        Style::default().fg(t.text_dim),
    )));

    render_dialog(
        frame,
        area,
        DialogSpec {
            title: " New Session ",
            border_color: t.accent,
            alignment: Alignment::Left,
            width: DialogSize::Fixed(40),
            height: DialogSize::Fixed(9),
        },
        lines,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::views::test_util::{contains_line, render_to_lines};

    #[test]
    fn test_selector_lists_both_agents() {
        let lines = render_to_lines(80, 24, |frame| {
            render_agent_type_selector(frame, frame.size(), 1)
        });

        assert!(contains_line(&lines, "New Session"), "{:?}", lines);
        assert!(contains_line(&lines, "Select agent type:"), "{:?}", lines);
        assert!(contains_line(&lines, "Claude Code"), "{:?}", lines);
        assert!(contains_line(&lines, "▶ Codex"), "{:?}", lines);
    }
}
