//! A Codex account's login and quota, as one row under its name
//!
//! Drawn in the Settings Codex accounts list and in the account step of the
//! new-session wizard, from what the last probe found
//! ([`crate::codex_config::probe`]). The row reads like
//! `user@example.com · Plus · 5h ██░░░░░░ 21% resets 20:10 · wk █░░░░░░░ 9%`.
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
use crate::tui::views::account_row::{fit, Field};

/// Cells in a quota bar
const BAR_CELLS: usize = 8;

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
        // A reset already past means nothing; it is left out, as everywhere
        if let Some(when) = window.resets_at.and_then(|at| format_reset(at, now)) {
            let mut reset = Field::text(
                format!("resets {}", when),
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

    /// The reset text for a window resetting `mins` from now, as drawn
    ///
    /// Computed rather than spelled out: resets are clock times on the
    /// user's own clock, so the text depends on the machine's time zone.
    fn reset_text(mins: i64) -> String {
        let at = now() + chrono::Duration::minutes(mins);
        format!("resets {}", format_reset(at, now()).unwrap())
    }

    #[test]
    fn wide_row_shows_everything() {
        let line = row(&chatgpt(25.0, 40.0), 200);
        assert_eq!(
            line,
            format!(
                "    user@example.com · Plus · 5h ██░░░░░░ 25% {} · wk ███░░░░░ 40% {}",
                reset_text(200),
                reset_text((2 * 24 + 4) * 60)
            )
        );
    }

    #[test]
    fn narrowing_drops_fields_whole_keeping_the_binding_window() {
        let probe = chatgpt(25.0, 40.0);
        let short_reset = reset_text(200);
        let week_reset = reset_text((2 * 24 + 4) * 60);
        let fields = [
            short_reset.as_str(),
            "user@example.com",
            week_reset.as_str(),
            "Plus",
            "5h ██░░░░░░ 25%",
        ];

        // Narrow a column at a time and note the order fields disappear in
        let mut gone: Vec<&str> = Vec::new();
        for width in (10..=200).rev() {
            let line = row(&probe, width);
            assert!(line.chars().count() <= width, "width {width}: {line}");
            for field in fields {
                if !line.contains(field) && !gone.contains(&field) {
                    gone.push(field);
                }
            }
            if width >= 20 {
                assert!(line.contains("wk ███░░░░░ 40%"), "width {width}: {line}");
            }
        }
        assert_eq!(gone, fields);
        assert_eq!(row(&probe, 30), "    wk ███░░░░░ 40%");
    }

    #[test]
    fn a_reset_never_outlives_its_window() {
        let probe = chatgpt(25.0, 40.0);
        let (short_reset, week_reset) = (reset_text(200), reset_text((2 * 24 + 4) * 60));
        for width in 10..120 {
            let line = row(&probe, width);
            if line.contains(&short_reset) {
                assert!(line.contains("5h ██"), "width {width}: {line}");
            }
            if line.contains(&week_reset) {
                assert!(line.contains("wk ███"), "width {width}: {line}");
            }
        }
    }

    #[test]
    fn a_past_reset_is_left_out() {
        let mut probe = chatgpt(25.0, 40.0);
        if let Some(Ok(status)) = probe.result.as_mut() {
            if let CodexUsage::Known(quota) = &mut status.usage {
                quota.primary = Some(window(25.0, 300, -5));
            }
        }
        let line = row(&probe, 200);
        assert!(line.contains("5h ██░░░░░░ 25% · wk"), "{line}");
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
