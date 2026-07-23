//! Unified confirmation dialog component
//!
//! Provides a consistent look and feel for all confirmation dialogs in the application.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{AppState, LoadingOverlay};
use crate::session::{SessionManager, SessionType};
use crate::tui::theme::theme;
use crate::tui::widgets::dialog::{centered_rect, render_dialog, DialogSize, DialogSpec};

/// The standard confirmation prompt: "Press y to confirm, n or Esc to cancel"
///
/// Shared by every confirmation dialog so the key styling (y green, n/Esc
/// red) cannot drift between them.
pub(crate) fn confirm_prompt_line() -> Line<'static> {
    let t = theme();
    Line::from(vec![
        Span::styled("Press ", Style::default().fg(t.text)),
        Span::styled(
            "y",
            Style::default()
                .fg(t.confirm_key)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" to confirm, ", Style::default().fg(t.text)),
        Span::styled(
            "n",
            Style::default()
                .fg(t.cancel_key)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" or ", Style::default().fg(t.text)),
        Span::styled(
            "Esc",
            Style::default()
                .fg(t.cancel_key)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" to cancel", Style::default().fg(t.text)),
    ])
}

/// Configuration for a confirmation dialog
pub struct ConfirmDialogConfig<'a> {
    /// Dialog title (e.g., "Confirm Delete")
    pub title: &'a str,
    /// Label for the item type (e.g., "project" or "session")
    pub item_label: &'a str,
    /// Name of the item being acted on
    pub item_name: &'a str,
    /// Warning lines (displayed in yellow with ⚠ prefix)
    pub warnings: Vec<String>,
    /// Note lines (displayed in muted gray)
    pub notes: Vec<String>,
    /// Pre-styled lines rendered after warnings/notes, before the prompt
    pub body_lines: Vec<Line<'a>>,
    /// Pre-styled lines rendered after the prompt (e.g. toggles)
    pub extra_lines: Vec<Line<'a>>,
    /// When set, render as a centered overlay of this size (cleared
    /// background) instead of filling the given area
    pub overlay: Option<(DialogSize, DialogSize)>,
}

impl<'a> ConfirmDialogConfig<'a> {
    /// A dialog with the standard header and prompt and nothing else
    pub fn new(title: &'a str, item_label: &'a str, item_name: &'a str) -> Self {
        Self {
            title,
            item_label,
            item_name,
            warnings: Vec::new(),
            notes: Vec::new(),
            body_lines: Vec::new(),
            extra_lines: Vec::new(),
            overlay: None,
        }
    }
}

/// Render a unified confirmation dialog
///
/// Unified style:
/// - Border: Yellow (warning color, indicates destructive action)
/// - Item name: Cyan + Bold
/// - Warnings: Yellow + Bold with ⚠ prefix
/// - Notes: Muted gray
/// - Prompt: "Press y to confirm, n or Esc to cancel" (y=green, n/Esc=red)
/// - Alignment: Center
pub fn render_confirm_dialog(frame: &mut Frame, area: Rect, config: ConfirmDialogConfig) {
    let t = theme();

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                format!("Delete {}: ", config.item_label),
                Style::default().fg(t.text),
            ),
            Span::styled(
                config.item_name,
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("?", Style::default().fg(t.text)),
        ]),
        Line::from(""),
    ];

    // Add warnings (yellow with ⚠ prefix)
    for warning in &config.warnings {
        lines.push(Line::from(vec![Span::styled(
            format!("⚠  {}", warning),
            Style::default()
                .fg(t.border_warning)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));
    }

    // Add notes (muted gray)
    for note in &config.notes {
        lines.push(Line::from(vec![Span::styled(
            note.as_str(),
            Style::default().fg(t.text_muted),
        )]));
        lines.push(Line::from(""));
    }

    // Pre-styled body content (before the prompt)
    lines.extend(config.body_lines);

    // Confirmation prompt with styled keys
    lines.push(confirm_prompt_line());

    // Pre-styled trailing content (after the prompt)
    lines.extend(config.extra_lines);

    let dialog_area = match config.overlay {
        Some((width, height)) => {
            let dialog_area = centered_rect(area, width, height);
            frame.render_widget(Clear, dialog_area);
            dialog_area
        }
        None => area,
    };

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border_warning))
            .title(config.title),
    );

    frame.render_widget(paragraph, dialog_area);
}

/// Render the "Confirm Delete" dialog for a session, as a centred overlay
///
/// Shared by every session list - pane 1's branch drill-down, pane 2, and the
/// session view - so deleting a session always asks first, reads the same
/// wherever it happens, and is anchored to the terminal rather than to a pane
/// that may be mid-transition.
pub fn render_session_delete_confirmation(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    sessions: &SessionManager,
) {
    let session = state.pending_delete_session.and_then(|id| sessions.get(id));

    let session_name = session
        .map(|s| s.info.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());

    let warning = session
        .map(|s| match s.info.session_type {
            SessionType::ClaudeCode => "This will kill the Claude Code process.",
            SessionType::OpenAICodex => "This will kill the Codex process.",
            SessionType::Shell => "This will kill the shell process.",
        })
        .unwrap_or("This will kill the process.")
        .to_string();

    let config = ConfirmDialogConfig {
        warnings: vec![warning],
        overlay: Some((DialogSize::Fixed(60), DialogSize::Fixed(9))),
        ..ConfirmDialogConfig::new("Confirm Delete", "session", &session_name)
    };
    render_confirm_dialog(frame, area, config);
}

/// Render a quit confirmation dialog
///
/// Centered dialog asking user to confirm quitting the application.
/// Uses the same styling as other confirmation dialogs.
pub fn render_quit_confirm_dialog(frame: &mut Frame, area: Rect) {
    let t = theme();

    let lines = vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            "Quit Panoptes?",
            Style::default().fg(t.text).add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        confirm_prompt_line(),
    ];

    render_dialog(
        frame,
        area,
        DialogSpec {
            title: "Confirm Quit",
            border_color: t.border_warning,
            alignment: Alignment::Center,
            width: DialogSize::Fixed(40),
            height: DialogSize::Fixed(7),
        },
        lines,
    );
}

/// Render a dismissable message overlay (errors and startup notices)
///
/// A centered box that renders over whatever view is active, so a message set
/// from any view or dialog is actually seen. The message may span several lines
/// (e.g. multiple startup warnings joined with newlines); the box grows to fit.
/// Dismissed by any keypress, handled in the main event loop.
fn render_message_overlay(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    border_color: Color,
    message: &str,
) {
    let t = theme();

    let mut lines = vec![Line::from("")];
    for msg_line in message.lines() {
        lines.push(Line::from(vec![Span::styled(
            msg_line.to_string(),
            Style::default().fg(t.text),
        )]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "Press any key to dismiss",
        Style::default().fg(t.text_muted),
    )]));

    // Height: borders (2) + blank + body + blank + hint
    let body = message.lines().count().max(1) as u16;
    let height = body + 5;

    render_dialog(
        frame,
        area,
        DialogSpec {
            title,
            border_color,
            alignment: Alignment::Center,
            width: DialogSize::Percent {
                pct: 70,
                min: 30,
                max: 70,
            },
            height: DialogSize::Fixed(height),
        },
        lines,
    );
}

/// Render an error overlay, visible from any view
pub fn render_error_overlay(frame: &mut Frame, area: Rect, message: &str) {
    let t = theme();
    render_message_overlay(frame, area, " Error ", t.border_warning, message);
}

/// Render a startup notice overlay (corrupt-file backups, etc.)
pub fn render_startup_notice_overlay(frame: &mut Frame, area: Rect, message: &str) {
    let t = theme();
    render_message_overlay(frame, area, " Startup Notice ", t.border_warning, message);
}

/// Render a loading indicator overlay
///
/// Displays a centered dialog with a spinner and the operation's message.
/// Uses cyan border to indicate informational/non-destructive status. Work
/// running in the background advances the spinner and adds a cancel hint;
/// work that blocks the event loop shows a still frame and no hint.
pub fn render_loading_indicator(frame: &mut Frame, area: Rect, loading: &LoadingOverlay) {
    let t = theme();

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                format!("{} ", loading.spinner()),
                Style::default().fg(t.accent),
            ),
            Span::styled(
                loading.message.as_str(),
                Style::default().fg(t.text).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    let cancellable = loading.cancellable && !loading.cancelling;
    if cancellable {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                "Esc",
                Style::default()
                    .fg(t.cancel_key)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to cancel", Style::default().fg(t.text_muted)),
        ]));
    }

    render_dialog(
        frame,
        area,
        DialogSpec {
            title: "Working",
            border_color: t.accent,
            alignment: Alignment::Center,
            width: DialogSize::Fixed(50),
            height: DialogSize::Fixed(if cancellable { 7 } else { 5 }),
        },
        lines,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::views::test_util::{contains_line, render_to_lines};

    #[test]
    fn test_session_delete_confirmation_asks_before_deleting() {
        use crate::config::Config;
        use crate::session::store::SessionStore;

        let config = Config::default();
        let sessions = SessionManager::with_store(config, SessionStore::new());
        let state = AppState {
            input_mode: crate::app::InputMode::ConfirmingSessionDelete,
            pending_delete_session: Some(uuid::Uuid::new_v4()),
            ..Default::default()
        };

        let lines = render_to_lines(70, 20, |frame| {
            render_session_delete_confirmation(frame, frame.size(), &state, &sessions)
        });

        assert!(contains_line(&lines, "Confirm Delete"), "{:?}", lines);
        assert!(
            contains_line(&lines, "Press y to confirm, n or Esc to cancel"),
            "{:?}",
            lines
        );
    }

    #[test]
    fn test_error_overlay_shows_message_and_dismiss_hint() {
        let lines = render_to_lines(70, 20, |frame| {
            render_error_overlay(frame, frame.size(), "Could not resume: worktree missing")
        });

        assert!(contains_line(&lines, "Error"), "{:?}", lines);
        assert!(
            contains_line(&lines, "Could not resume: worktree missing"),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "Press any key to dismiss"),
            "{:?}",
            lines
        );
    }

    #[test]
    fn test_startup_notice_overlay_renders_each_warning_line() {
        let lines = render_to_lines(70, 20, |frame| {
            render_startup_notice_overlay(
                frame,
                frame.size(),
                "config.toml was corrupt\nbacked up to config.toml.bak",
            )
        });

        assert!(contains_line(&lines, "Startup Notice"), "{:?}", lines);
        assert!(
            contains_line(&lines, "config.toml was corrupt"),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "backed up to config.toml.bak"),
            "{:?}",
            lines
        );
    }

    #[test]
    fn test_confirm_dialog_shows_warnings_notes_and_prompt() {
        let lines = render_to_lines(70, 20, |frame| {
            render_confirm_dialog(
                frame,
                frame.size(),
                ConfirmDialogConfig {
                    warnings: vec!["2 active sessions will be terminated".to_string()],
                    notes: vec!["Git worktrees on disk will NOT be deleted.".to_string()],
                    ..ConfirmDialogConfig::new("Confirm Delete", "project", "panoptes")
                },
            )
        });

        assert!(contains_line(&lines, "Confirm Delete"), "{:?}", lines);
        assert!(
            contains_line(&lines, "Delete project: panoptes?"),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "⚠  2 active sessions will be terminated"),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "Git worktrees on disk will NOT be deleted."),
            "{:?}",
            lines
        );
        assert!(
            contains_line(&lines, "Press y to confirm, n or Esc to cancel"),
            "{:?}",
            lines
        );
    }

    #[test]
    fn test_confirm_dialog_renders_body_and_extra_lines_around_prompt() {
        let lines = render_to_lines(70, 20, |frame| {
            render_confirm_dialog(
                frame,
                frame.size(),
                ConfirmDialogConfig {
                    body_lines: vec![Line::from("body detail"), Line::from("")],
                    extra_lines: vec![Line::from(""), Line::from("[ ] a toggle")],
                    ..ConfirmDialogConfig::new("Confirm Delete", "branch", "feature-x")
                },
            )
        });

        assert!(contains_line(&lines, "body detail"), "{:?}", lines);
        assert!(contains_line(&lines, "[ ] a toggle"), "{:?}", lines);

        // Body renders above the prompt, extras below it
        let body_row = lines
            .iter()
            .position(|l| l.contains("body detail"))
            .unwrap();
        let prompt_row = lines
            .iter()
            .position(|l| l.contains("Press y to confirm"))
            .unwrap();
        let toggle_row = lines.iter().position(|l| l.contains("a toggle")).unwrap();
        assert!(body_row < prompt_row && prompt_row < toggle_row);
    }

    #[test]
    fn test_confirm_dialog_overlay_is_centered_and_cleared() {
        let lines = render_to_lines(70, 21, |frame| {
            render_confirm_dialog(
                frame,
                frame.size(),
                ConfirmDialogConfig {
                    overlay: Some((DialogSize::Fixed(50), DialogSize::Fixed(7))),
                    ..ConfirmDialogConfig::new("Confirm Delete", "config", "Work")
                },
            )
        });

        assert!(contains_line(&lines, "Delete config: Work?"), "{:?}", lines);
        // Overlay leaves the rows above the centered dialog empty
        assert!(lines[0].is_empty(), "{:?}", lines);
    }

    #[test]
    fn test_quit_dialog_prompts_for_confirmation() {
        let lines = render_to_lines(70, 20, |frame| {
            render_quit_confirm_dialog(frame, frame.size())
        });

        assert!(contains_line(&lines, "Confirm Quit"), "{:?}", lines);
        assert!(contains_line(&lines, "Quit Panoptes?"), "{:?}", lines);
        assert!(
            contains_line(&lines, "Press y to confirm, n or Esc to cancel"),
            "{:?}",
            lines
        );
    }

    #[test]
    fn test_loading_indicator_shows_message_and_spinner() {
        let loading = LoadingOverlay::new("Creating worktree...", false);
        let lines = render_to_lines(70, 20, |frame| {
            render_loading_indicator(frame, frame.size(), &loading)
        });

        assert!(contains_line(&lines, "Working"), "{:?}", lines);
        assert!(
            contains_line(
                &lines,
                &format!("{} Creating worktree...", loading.spinner())
            ),
            "{:?}",
            lines
        );
        // A blocking operation cannot be called off
        assert!(!contains_line(&lines, "to cancel"), "{:?}", lines);
    }

    #[test]
    fn test_loading_indicator_offers_cancel_until_cancelling() {
        let mut loading = LoadingOverlay::new("Fetching branches from remotes...", true);
        let lines = render_to_lines(70, 20, |frame| {
            render_loading_indicator(frame, frame.size(), &loading)
        });
        assert!(contains_line(&lines, "Esc to cancel"), "{:?}", lines);

        // Once asked to cancel, there is nothing left to press
        loading.cancelling = true;
        let lines = render_to_lines(70, 20, |frame| {
            render_loading_indicator(frame, frame.size(), &loading)
        });
        assert!(!contains_line(&lines, "to cancel"), "{:?}", lines);
    }

    #[test]
    fn test_loading_indicator_spinner_advances_over_time() {
        use std::time::Duration;

        let mut loading = LoadingOverlay::new("Fetching branches from remotes...", true);
        let first = loading.spinner();

        assert!(
            loading.tick(loading.started_at + Duration::from_millis(240)),
            "the spinner should have advanced by 240ms"
        );
        assert_ne!(first, loading.spinner());

        // Ticking again within the same frame is not worth a re-render
        assert!(!loading.tick(loading.started_at + Duration::from_millis(250)));
    }
}
