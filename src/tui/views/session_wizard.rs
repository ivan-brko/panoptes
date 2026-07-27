//! The new-session wizard: one centred modal, three steps
//!
//! `n` at a branch asks three things in a row - which agent, which account,
//! what to call it - and all three are drawn in the same centred frame so the
//! eye never moves between an overlay and a pane mid-flow. Steps 1 and 3 live
//! here; step 2 is the config selector in [`super::agent_configs`], which is
//! dual-use and so keeps its own home.
//!
//! Every step after the first names what has already been chosen in its
//! title, so the modal reads as one dialog advancing rather than three
//! dialogs in a queue.

use ratatui::prelude::*;

use crate::tui::theme::theme;
use crate::tui::widgets::dialog::{render_dialog, DialogSize, DialogSpec};
use crate::tui::widgets::selection::{selection_name_style, selection_prefix};

/// The agents offered by step 1, in list order
pub const WIZARD_AGENTS: [&str; 2] = ["Claude Code", "Codex"];

/// Title for a wizard step, naming what has been chosen so far
///
/// `agent` is the agent's own label ("Claude", "Codex", "shell"); `account` is
/// the config picked in step 2, when there was one to pick.
pub fn wizard_title(agent: &str, account: Option<&str>) -> String {
    match account {
        Some(account) => format!(" New {} session · {} ", agent, account),
        None => format!(" New {} session ", agent),
    }
}

/// Render step 1: which agent runs the session
pub fn render_agent_type_selector(frame: &mut Frame, area: Rect, selected_index: usize) {
    let t = theme();

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Select agent type:",
            Style::default().fg(t.text),
        )),
        Line::from(""),
    ];

    for (i, agent) in WIZARD_AGENTS.iter().enumerate() {
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
            width: DialogSize::Fixed(50),
            height: DialogSize::Fixed(9),
        },
        lines,
    );
}

/// Render the wizard's last step: what the session is called
///
/// `esc_backs_up` is false only for a shell session, whose one-step flow has
/// nothing behind it to go back to.
pub fn render_session_name_input(
    frame: &mut Frame,
    area: Rect,
    agent: &str,
    account: Option<&str>,
    name: &str,
    esc_backs_up: bool,
) {
    let t = theme();

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled("Session name:", Style::default().fg(t.text))),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{}_", name), Style::default().fg(t.accent)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            if esc_backs_up {
                "[Enter] Create  [Esc] Back"
            } else {
                "[Enter] Create  [Esc] Cancel"
            },
            Style::default().fg(t.text_dim),
        )),
    ];

    render_dialog(
        frame,
        area,
        DialogSpec {
            title: &wizard_title(agent, account),
            border_color: t.accent,
            alignment: Alignment::Left,
            width: DialogSize::Fixed(50),
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

    /// The name step carries the choices already made, so the modal reads as
    /// one dialog advancing
    #[test]
    fn test_name_step_titles_itself_with_agent_and_account() {
        let lines = render_to_lines(80, 24, |frame| {
            render_session_name_input(
                frame,
                frame.size(),
                "Claude",
                Some("dot-lambda"),
                "revi",
                true,
            )
        });

        assert!(
            contains_line(&lines, "New Claude session · dot-lambda"),
            "{:?}",
            lines
        );
        assert!(contains_line(&lines, "Session name:"), "{:?}", lines);
        assert!(contains_line(&lines, "revi_"), "{:?}", lines);
    }

    /// With no account chosen the title stops at the agent, and a step that
    /// has nothing behind it says "Cancel" rather than "Back"
    #[test]
    fn test_name_step_without_an_account_or_a_step_back() {
        let lines = render_to_lines(80, 24, |frame| {
            render_session_name_input(frame, frame.size(), "shell", None, "", false)
        });

        assert!(contains_line(&lines, "New shell session"), "{:?}", lines);
        assert!(
            !contains_line(&lines, "·"),
            "no account, so no separator: {:?}",
            lines
        );
        assert!(contains_line(&lines, "[Esc] Cancel"), "{:?}", lines);

        let backs_up = render_to_lines(80, 24, |frame| {
            render_session_name_input(frame, frame.size(), "Codex", None, "", true)
        });
        assert!(contains_line(&backs_up, "[Esc] Back"), "{:?}", backs_up);
    }
}
