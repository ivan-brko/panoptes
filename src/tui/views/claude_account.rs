//! A Claude profile's login, as one row under its name
//!
//! Drawn in the Settings Claude accounts list and in the account step of the
//! new-session wizard, from the profile's `.claude.json`
//! ([`crate::claude_config::account`]). The row reads like
//! `user@example.com · Max 20x · Example Org`.
//!
//! Fields drop whole as the row narrows, least useful first: the
//! organisation, the email, the plan. A warning that another profile is
//! signed into the same account outlasts them all but the login itself -
//! it is the thing most worth knowing before picking this profile.

use std::path::Path;

use ratatui::prelude::*;
use uuid::Uuid;

use crate::claude_config::account::{login_command, ClaudeAuth, ClaudeLogins};
use crate::claude_config::ClaudeConfigStore;
use crate::tui::theme::theme;
use crate::tui::views::account_row::{fit, Field};

/// The status row for profile `id` of `store`, from what was last read
pub fn claude_profile_line(
    store: &ClaudeConfigStore,
    logins: &ClaudeLogins,
    id: Uuid,
    width: usize,
) -> Line<'static> {
    let shared_with: Vec<&str> = logins
        .shared
        .get(&id)
        .into_iter()
        .flatten()
        .filter_map(|other| store.get(*other).map(|c| c.name.as_str()))
        .collect();
    let config_dir = store.get(id).and_then(|c| c.config_dir.as_deref());
    claude_account_line(logins.auth.get(&id), &shared_with, config_dir, width)
}

/// The status row for one profile, fitted to `width` columns
///
/// `shared_with` names the other profiles signed into the same account.
/// `config_dir` is the profile's `CLAUDE_CONFIG_DIR`, for the login command.
pub fn claude_account_line(
    auth: Option<&ClaudeAuth>,
    shared_with: &[&str],
    config_dir: Option<&Path>,
    width: usize,
) -> Line<'static> {
    let t = theme();
    let text = Style::default().fg(t.text);
    let warning = Style::default().fg(t.warning);
    let mut fields = match auth {
        None => vec![Field::text("checking…", t.muted_style(), 9)],
        Some(ClaudeAuth::SignedOut) => vec![
            Field::text("not signed in", warning, 9),
            Field::text(
                format!("run `{}`", login_command(config_dir)),
                t.muted_style(),
                1,
            ),
        ],
        Some(ClaudeAuth::Unreadable(reason)) => vec![
            Field::text("can't read .claude.json", warning, 9),
            Field::text(reason.clone(), t.muted_style(), 1),
        ],
        Some(ClaudeAuth::Other(kind)) => vec![Field::text(*kind, text, 9)],
        Some(ClaudeAuth::Subscription(account)) => {
            let mut fields = Vec::new();
            if let Some(email) = &account.email {
                fields.push(Field::text(email.clone(), t.muted_style(), 4));
            }
            fields.push(Field::text(
                account.plan.clone().unwrap_or_else(|| "Claude".to_string()),
                text,
                5,
            ));
            if let Some(org) = &account.organization {
                fields.push(Field::text(org.clone(), t.muted_style(), 1));
            }
            fields
        }
    };
    if !shared_with.is_empty() {
        fields.push(Field::text(
            format!("same account as {}", shared_with.join(", ")),
            warning,
            8,
        ));
    }
    fit(fields, width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_config::account::ClaudeAccount;

    fn text_of(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn max() -> ClaudeAuth {
        ClaudeAuth::Subscription(ClaudeAccount {
            email: Some("user@example.com".to_string()),
            plan: Some("Max 20x".to_string()),
            organization: Some("Example Org".to_string()),
            ..Default::default()
        })
    }

    #[test]
    fn test_full_row() {
        let line = claude_account_line(Some(&max()), &[], None, 80);
        assert_eq!(
            text_of(&line),
            "    user@example.com · Max 20x · Example Org"
        );
    }

    /// Organisation first, then email; the plan holds on
    #[test]
    fn test_fields_drop_whole_as_the_row_narrows() {
        let auth = max();
        let at = |width| text_of(&claude_account_line(Some(&auth), &[], None, width));
        assert_eq!(at(40), "    user@example.com · Max 20x");
        assert_eq!(at(20), "    Max 20x");
    }

    /// The duplicate warning outlasts the email and organisation
    #[test]
    fn test_shared_account_warning_holds_on() {
        let auth = max();
        let row = |width| text_of(&claude_account_line(Some(&auth), &["Work"], None, width));
        assert_eq!(
            row(100),
            "    user@example.com · Max 20x · Example Org · same account as Work"
        );
        assert_eq!(row(40), "    Max 20x · same account as Work");
    }

    #[test]
    fn test_signed_out_row_names_the_login_command() {
        let line = claude_account_line(
            Some(&ClaudeAuth::SignedOut),
            &[],
            Some(Path::new("/opt/claude-work")),
            100,
        );
        assert_eq!(
            text_of(&line),
            "    not signed in · run `CLAUDE_CONFIG_DIR=/opt/claude-work claude auth login`"
        );
        // Too narrow for the command, the state itself stays
        let narrow = claude_account_line(Some(&ClaudeAuth::SignedOut), &[], None, 30);
        assert_eq!(text_of(&narrow), "    not signed in");
    }

    #[test]
    fn test_other_auth_and_unreadable_rows() {
        let line = claude_account_line(Some(&ClaudeAuth::Other("Bedrock")), &[], None, 80);
        assert_eq!(text_of(&line), "    Bedrock");
        let line = claude_account_line(
            Some(&ClaudeAuth::Unreadable("not valid JSON".to_string())),
            &[],
            None,
            80,
        );
        assert_eq!(
            text_of(&line),
            "    can't read .claude.json · not valid JSON"
        );
        assert_eq!(
            text_of(&claude_account_line(None, &[], None, 80)),
            "    checking…"
        );
    }
}
