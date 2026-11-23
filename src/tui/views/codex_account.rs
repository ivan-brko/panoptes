//! A Codex account's login and quota, as one row under its name
//!
//! Drawn in the Settings Codex accounts list and in the account step of the
//! new-session wizard, from what the last probe found
//! ([`crate::codex_config::probe`]). The row reads like
//! `user@example.com · Plus · 5h ██░░░░░░ 21% resets in 3h 20m · wk █░░░░░░░ 9%`.
//!
//! Fields drop whole as the row narrows, least useful first: the other
//! window's reset, the email, the binding window's reset, the plan, the other
//! window. The binding window - the one closest to stopping the user, which
//! decides whether this account can take a turn - goes last.

use chrono::{DateTime, Utc};
use ratatui::prelude::*;

use crate::agent::events::{format_reset, RateLimitWindow};
use crate::codex_config::probe::{
    CodexAccountProbe, CodexAccountStatus, CodexLogin, CodexQuota, CodexUsage,
};
use crate::tui::theme::{theme, Theme};

/// Indent that puts the row under the account's name, past the selection prefix
pub const DETAIL_INDENT: &str = "    ";

/// Cells in a quota bar
const BAR_CELLS: usize = 8;

/// Separator between fields
const SEP: &str = " · ";

/// One piece of the row, and how long it holds on as the row narrows
struct Field {
    spans: Vec<Span<'static>>,
    /// Higher holds on longer
    keep: u8,
    /// Joined to the field before it by a space rather than a separator
    ///
    /// A window's reset time is part of that window, not a field of its own.
    /// It always gives way before its window does, so it is never left
    /// joined to the wrong one.
    joined: bool,
}

impl Field {
    fn text(text: impl Into<String>, style: Style, keep: u8) -> Self {
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

/// The status row for one account, fitted to `width` columns
pub fn codex_account_line(
    probe: Option<&CodexAccountProbe>,
    width: usize,
    now: DateTime<Utc>,
) -> Line<'static> {
    let t = theme();
    let mut fields = match probe.and_then(|p| p.result.as_ref()) {
        None => vec![Field::text("checking…", t.muted_style(), 9)],
        Some(Err(reason)) => vec![
            Field::text("status unavailable", Style::default().fg(t.warning), 9),
            Field::text(reason.clone(), t.muted_style(), 1),
        ],
        Some(Ok(status)) => status_fields(status, now, &t),
    };
    // A refresh keeps the last result up, marked as being checked again
    if probe.is_some_and(|p| p.checking && p.result.is_some()) {
        fields.push(Field::text("…", t.muted_style(), 0));
    }
    fit(fields, width)
}

/// Fields for a probe that answered
fn status_fields(status: &CodexAccountStatus, now: DateTime<Utc>, t: &Theme) -> Vec<Field> {
    let text = Style::default().fg(t.text);
    let mut fields = Vec::new();
    match &status.login {
        CodexLogin::LoggedOut => {
            fields.push(Field::text(
                "not logged in",
                Style::default().fg(t.warning),
                9,
            ));
            fields.push(Field::text(
                "run `codex login` with this CODEX_HOME",
                t.muted_style(),
                1,
            ));
            return fields;
        }
        CodexLogin::ApiKey => fields.push(Field::text("API key", text, 9)),
        CodexLogin::Other(kind) => fields.push(Field::text(kind.clone(), text, 9)),
        CodexLogin::ChatGpt { email, plan } => {
            if let Some(email) = email {
                fields.push(Field::text(email.clone(), t.muted_style(), 2));
            }
            fields.push(Field::text(
                plan.clone().unwrap_or_else(|| "ChatGPT".to_string()),
                text,
                5,
            ));
        }
    }
    match &status.usage {
        CodexUsage::Known(quota) => fields.extend(quota_fields(quota, now, t)),
        CodexUsage::NotApplicable(reason) => fields.push(Field::text(*reason, t.muted_style(), 3)),
        CodexUsage::Unavailable(_) => {
            fields.push(Field::text("usage unavailable", t.muted_style(), 3))
        }
    }
    fields
}

/// Each window's bar, percentage and reset; the binding window holds on longest
fn quota_fields(quota: &CodexQuota, now: DateTime<Utc>, t: &Theme) -> Vec<Field> {
    let windows = quota.windows();
    if windows.is_empty() {
        return vec![Field::text("no quota reported", t.muted_style(), 3)];
    }
    let binding = binding_index(&windows);
    let mut fields = Vec::new();
    if quota.limit_reached.is_some() {
        fields.push(Field::text("limit hit", Style::default().fg(t.danger), 10));
    }
    for (i, (label, window)) in windows.iter().enumerate() {
        let is_binding = i == binding;
        let colour = Style::default().fg(level_colour(window.used_percent, t));
        let (filled, empty) = bar_cells(window.used_percent);
        fields.push(Field {
            spans: vec![
                Span::styled(format!("{} ", label), Style::default().fg(t.text)),
                Span::styled("█".repeat(filled), colour),
                Span::styled("░".repeat(empty), Style::default().fg(t.text_faint)),
                Span::styled(format!(" {:.0}%", window.used_percent), colour),
            ],
            keep: if is_binding { 8 } else { 6 },
            joined: false,
        });
        if let Some(at) = window.resets_at {
            let mut reset = Field::text(
                format!("resets {}", format_reset(at, now)),
                t.muted_style(),
                if is_binding { 4 } else { 1 },
            );
            reset.joined = true;
            fields.push(reset);
        }
    }
    fields
}

/// The window closest to stopping the user: highest use, longer on a tie
fn binding_index(windows: &[(String, &RateLimitWindow)]) -> usize {
    let key = |w: &RateLimitWindow| (w.used_percent, w.window_minutes.unwrap_or(u64::MAX));
    let mut best = 0;
    for (i, (_, window)) in windows.iter().enumerate() {
        if key(window) > key(windows[best].1) {
            best = i;
        }
    }
    best
}

/// Filled and empty cells for a share used
fn bar_cells(used_percent: f64) -> (usize, usize) {
    let filled = ((used_percent.clamp(0.0, 100.0) / 100.0) * BAR_CELLS as f64).round() as usize;
    (filled, BAR_CELLS - filled)
}

/// The outcome colour for a share used: fine, careful, nearly out
fn level_colour(used_percent: f64, t: &Theme) -> Color {
    if used_percent >= 90.0 {
        t.danger
    } else if used_percent >= 70.0 {
        t.warning
    } else {
        t.success
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
fn fit(mut fields: Vec<Field>, width: usize) -> Line<'static> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000, 0).unwrap()
    }

    fn text_of(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn window(used: f64, minutes: u64, resets_in_mins: i64) -> RateLimitWindow {
        RateLimitWindow {
            used_percent: used,
            window_minutes: Some(minutes),
            resets_at: Some(now() + chrono::Duration::minutes(resets_in_mins)),
        }
    }

    fn chatgpt(primary: f64, secondary: f64) -> CodexAccountProbe {
        CodexAccountProbe {
            result: Some(Ok(CodexAccountStatus {
                login: CodexLogin::ChatGpt {
                    email: Some("user@example.com".to_string()),
                    plan: Some("Plus".to_string()),
                },
                usage: CodexUsage::Known(CodexQuota {
                    primary: Some(window(primary, 300, 200)),
                    secondary: Some(window(secondary, 10_080, (2 * 24 + 4) * 60)),
                    ..Default::default()
                }),
            })),
            checking: false,
        }
    }

    fn row(probe: &CodexAccountProbe, width: usize) -> String {
        text_of(&codex_account_line(Some(probe), width, now()))
    }

    #[test]
    fn wide_row_shows_everything() {
        let line = row(&chatgpt(25.0, 40.0), 200);
        assert_eq!(
            line,
            "    user@example.com · Plus · 5h ██░░░░░░ 25% resets in 3h 20m · wk ███░░░░░ 40% resets in 2d 4h"
        );
    }

    #[test]
    fn narrowing_drops_fields_whole_keeping_the_binding_window() {
        let probe = chatgpt(25.0, 40.0);

        let no_side_reset = row(&probe, 80);
        assert!(!no_side_reset.contains("in 3h 20m"), "{no_side_reset}");
        assert!(
            no_side_reset.contains("user@example.com"),
            "{no_side_reset}"
        );

        let no_email = row(&probe, 64);
        assert!(!no_email.contains("user@example.com"), "{no_email}");
        assert!(no_email.contains("Plus"), "{no_email}");
        assert!(no_email.contains("resets in 2d 4h"), "{no_email}");

        let two_bars = row(&probe, 40);
        assert_eq!(two_bars, "    5h ██░░░░░░ 25% · wk ███░░░░░ 40%");

        let narrow = row(&probe, 30);
        assert_eq!(narrow, "    wk ███░░░░░ 40%");

        let tiny = row(&probe, 20);
        assert!(tiny.chars().count() <= 20, "{tiny}");
        assert!(tiny.starts_with("    wk"), "{tiny}");
    }

    #[test]
    fn a_reset_never_outlives_its_window() {
        let probe = chatgpt(25.0, 40.0);
        for width in 10..120 {
            let line = row(&probe, width);
            if line.contains("in 3h 20m") {
                assert!(line.contains("5h ██"), "width {width}: {line}");
            }
            if line.contains("in 2d 4h") {
                assert!(line.contains("wk ███"), "width {width}: {line}");
            }
        }
    }

    #[test]
    fn limit_hit_holds_on_longest() {
        let mut probe = chatgpt(100.0, 40.0);
        if let Some(Ok(status)) = probe.result.as_mut() {
            if let CodexUsage::Known(quota) = &mut status.usage {
                quota.limit_reached = Some("rate_limit_reached".to_string());
            }
        }
        assert!(row(&probe, 200).contains("limit hit · "));
        assert!(row(&probe, 20).contains("limit hit"));
    }

    #[test]
    fn logged_out_says_so() {
        let probe = CodexAccountProbe {
            result: Some(Ok(CodexAccountStatus {
                login: CodexLogin::LoggedOut,
                usage: CodexUsage::NotApplicable("not logged in"),
            })),
            checking: false,
        };
        let wide = row(&probe, 120);
        assert!(wide.contains("not logged in · run `codex login`"), "{wide}");
        assert_eq!(row(&probe, 20).trim(), "not logged in");
    }

    #[test]
    fn api_key_shows_no_quota() {
        let probe = CodexAccountProbe {
            result: Some(Ok(CodexAccountStatus {
                login: CodexLogin::ApiKey,
                usage: CodexUsage::NotApplicable("API key, no plan quota"),
            })),
            checking: false,
        };
        assert_eq!(row(&probe, 120).trim(), "API key · API key, no plan quota");
    }

    #[test]
    fn failed_or_pending_probes() {
        assert_eq!(
            text_of(&codex_account_line(None, 60, now())).trim(),
            "checking…"
        );
        let failed = CodexAccountProbe {
            result: Some(Err("Codex did not answer initialize in time".to_string())),
            checking: false,
        };
        assert!(row(&failed, 120).contains("status unavailable · Codex did not answer"));
        assert_eq!(row(&failed, 24).trim(), "status unavailable");

        let unavailable = CodexAccountProbe {
            result: Some(Ok(CodexAccountStatus {
                login: CodexLogin::ChatGpt {
                    email: None,
                    plan: None,
                },
                usage: CodexUsage::Unavailable("timeout".to_string()),
            })),
            checking: true,
        };
        assert_eq!(
            row(&unavailable, 120).trim(),
            "ChatGPT · usage unavailable · …"
        );
    }

    #[test]
    fn bar_rounds_to_cells() {
        assert_eq!(bar_cells(0.0), (0, 8));
        assert_eq!(bar_cells(6.0), (0, 8));
        assert_eq!(bar_cells(7.0), (1, 7));
        assert_eq!(bar_cells(100.0), (8, 0));
        assert_eq!(bar_cells(150.0), (8, 0));
    }

    #[test]
    fn binding_window_is_the_fuller_one_and_the_longer_on_a_tie() {
        let a = window(10.0, 300, 0);
        let b = window(10.0, 10_080, 0);
        let windows = vec![("5h".to_string(), &a), ("wk".to_string(), &b)];
        assert_eq!(binding_index(&windows), 1);

        let c = window(50.0, 300, 0);
        let windows = vec![("5h".to_string(), &c), ("wk".to_string(), &b)];
        assert_eq!(binding_index(&windows), 0);
    }
}
