//! Which account a Claude profile is signed into
//!
//! Claude Code keeps its login in `.claude.json` - `$CLAUDE_CONFIG_DIR/.claude.json`
//! for a profile, `~/.claude.json` (beside `~/.claude`, not inside it) for one
//! without a `CLAUDE_CONFIG_DIR`. Its `oauthAccount` object names the account:
//! email, organisation and plan. The token itself lives elsewhere (the
//! keychain or `.credentials.json`) and is never read here.
//!
//! A profile can also not be using a subscription at all. Bedrock, Vertex and
//! Foundry are switched on by environment variables, from Panoptes' own
//! environment (which sessions inherit) or the profile's `settings.json`
//! `env`; an API key comes from `ANTHROPIC_API_KEY` (only once Claude has
//! been told to use it), `ANTHROPIC_AUTH_TOKEN`, an `apiKeyHelper`, or a key
//! stored by `/login` as `primaryApiKey`. Those are shown as what they are.
//!
//! Reading is synchronous and cheap - one small JSON file and a stat - and
//! only happens while the profiles are on screen, when a file changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

/// How a profile's sessions will authenticate
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeAuth {
    /// A claude.ai subscription login
    Subscription(ClaudeAccount),
    /// Not through Anthropic's login at all: "API key", "Bedrock", ...
    Other(&'static str),
    /// Nothing to sign in with
    SignedOut,
    /// `.claude.json` is there but could not be read
    Unreadable(String),
}

/// The account behind a subscription login
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClaudeAccount {
    pub email: Option<String>,
    /// Plan as the user knows it: "Pro", "Max 20x", "Team"
    pub plan: Option<String>,
    pub organization: Option<String>,
    pub account_uuid: Option<String>,
    pub organization_uuid: Option<String>,
}

impl ClaudeAccount {
    /// What makes two logins the same: the account, in the same organisation
    ///
    /// One person in two organisations is two seats with two quotas, so that
    /// is not a duplicate. `None` when the file names neither id nor email.
    pub fn identity(&self) -> Option<(String, Option<String>)> {
        let who = self
            .account_uuid
            .clone()
            .or_else(|| self.email.as_ref().map(|e| e.to_lowercase()))?;
        Some((who, self.organization_uuid.clone()))
    }
}

/// The `.claude.json` a profile's sessions read their login from
///
/// `config_dir` is the profile's `CLAUDE_CONFIG_DIR`. Without one a session
/// inherits Panoptes' own, and only without that too is it `~/.claude.json`.
pub fn claude_json_path(config_dir: Option<&Path>) -> Option<PathBuf> {
    Some(effective_dir(config_dir)?.join(".claude.json"))
}

/// The directory `.claude.json` lives in, or `None` with no home directory
fn effective_dir(config_dir: Option<&Path>) -> Option<PathBuf> {
    match config_dir {
        Some(dir) => Some(dir.to_path_buf()),
        None => match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
            Some(dir) => Some(PathBuf::from(dir)),
            None => dirs::home_dir(),
        },
    }
}

/// The profile's user-level `settings.json`, whose `env` its sessions get
fn settings_path(config_dir: Option<&Path>) -> Option<PathBuf> {
    let dir = match config_dir {
        Some(dir) => dir.to_path_buf(),
        None => match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
            Some(dir) => PathBuf::from(dir),
            None => dirs::home_dir()?.join(".claude"),
        },
    };
    Some(dir.join("settings.json"))
}

/// Modification times of the files a profile's login is read from
///
/// Compared between looks to tell when to read again. A missing file is
/// `None`, so creating or deleting one is a change too.
pub fn login_files_mtime(config_dir: Option<&Path>) -> (Option<SystemTime>, Option<SystemTime>) {
    let mtime = |path: Option<PathBuf>| {
        path.and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok())
    };
    (
        mtime(claude_json_path(config_dir)),
        mtime(settings_path(config_dir)),
    )
}

/// Read how a profile is signed in, from disk and Panoptes' environment
pub fn read_auth(config_dir: Option<&Path>) -> ClaudeAuth {
    let read = |path: Option<PathBuf>| -> std::io::Result<Option<String>> {
        let Some(path) = path else { return Ok(None) };
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    };
    let claude_json = match read(claude_json_path(config_dir)) {
        Ok(text) => text,
        Err(e) => return ClaudeAuth::Unreadable(e.to_string()),
    };
    // A settings file that cannot be read only loses its `env`
    let settings = read(settings_path(config_dir)).ok().flatten();
    parse_auth(claude_json.as_deref(), settings.as_deref(), &|name| {
        std::env::var(name).ok()
    })
}

/// [`read_auth`] on file contents already read, with the environment supplied
///
/// `claude_json` and `settings` are `None` for a missing file. `env` looks up
/// Panoptes' own environment; the settings file's `env` overrides it, as it
/// does for the session.
pub fn parse_auth(
    claude_json: Option<&str>,
    settings: Option<&str>,
    env: &dyn Fn(&str) -> Option<String>,
) -> ClaudeAuth {
    let settings: Option<Value> = settings.and_then(|s| serde_json::from_str(s).ok());
    let settings_env = settings.as_ref().and_then(|s| s.get("env"));
    let var = |name: &str| -> Option<String> {
        settings_env
            .and_then(|e| e.get(name))
            .and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Bool(b) => Some(b.to_string()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .or_else(|| env(name))
            .filter(|v| !v.is_empty())
    };
    let truthy = |name: &str| {
        var(name).is_some_and(|v| !matches!(v.to_ascii_lowercase().as_str(), "0" | "false" | "no"))
    };

    // A cloud provider replaces Anthropic's login outright
    if truthy("CLAUDE_CODE_USE_BEDROCK") {
        return ClaudeAuth::Other("Bedrock");
    }
    if truthy("CLAUDE_CODE_USE_VERTEX") {
        return ClaudeAuth::Other("Vertex AI");
    }
    if truthy("CLAUDE_CODE_USE_FOUNDRY") {
        return ClaudeAuth::Other("Foundry");
    }

    let config: Option<Value> = match claude_json {
        None => None,
        Some(text) if text.trim().is_empty() => None,
        Some(text) => match serde_json::from_str(text) {
            Ok(value) => Some(value),
            Err(e) => return ClaudeAuth::Unreadable(format!("not valid JSON: {}", e)),
        },
    };

    if var("ANTHROPIC_AUTH_TOKEN").is_some() {
        return ClaudeAuth::Other("auth token");
    }
    if settings
        .as_ref()
        .and_then(|s| s.get("apiKeyHelper"))
        .and_then(Value::as_str)
        .is_some_and(|h| !h.trim().is_empty())
    {
        return ClaudeAuth::Other("API key helper");
    }
    // Claude only uses an environment key the user has said yes to, and
    // remembers the answer by the key's last 20 characters
    if let Some(key) = var("ANTHROPIC_API_KEY") {
        let tail: String = {
            let chars: Vec<char> = key.chars().collect();
            chars[chars.len().saturating_sub(20)..].iter().collect()
        };
        let approved = config
            .as_ref()
            .and_then(|c| c.pointer("/customApiKeyResponses/approved"))
            .and_then(Value::as_array)
            .is_some_and(|keys| keys.iter().any(|k| k.as_str() == Some(tail.as_str())));
        if approved {
            return ClaudeAuth::Other("API key");
        }
    }

    let Some(config) = config else {
        return ClaudeAuth::SignedOut;
    };
    if let Some(account) = config.get("oauthAccount").and_then(parse_account) {
        return ClaudeAuth::Subscription(account);
    }
    if config
        .get("primaryApiKey")
        .and_then(Value::as_str)
        .is_some_and(|k| !k.is_empty())
    {
        return ClaudeAuth::Other("API key");
    }
    ClaudeAuth::SignedOut
}

/// The account in an `oauthAccount` object, or `None` for an empty one
fn parse_account(value: &Value) -> Option<ClaudeAccount> {
    let field = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let account = ClaudeAccount {
        email: field("emailAddress"),
        plan: plan_label(value),
        organization: field("organizationName"),
        account_uuid: field("accountUuid"),
        organization_uuid: field("organizationUuid"),
    };
    (account.email.is_some() || account.account_uuid.is_some()).then_some(account)
}

/// The plan as the user knows it
///
/// Claude's own display name when it recorded one; otherwise from the
/// organisation type, with Max's usage multiple read off its rate-limit tier
/// (`default_claude_max_20x`).
fn plan_label(value: &Value) -> Option<String> {
    let field = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
    };
    if let Some(name) = field("planDisplayName") {
        return Some(name.trim().to_string());
    }
    let label = match field("organizationType")? {
        "claude_pro" => "Pro",
        "claude_max" => {
            let tier = [
                field("userRateLimitTier"),
                field("organizationRateLimitTier"),
            ]
            .into_iter()
            .flatten()
            .find_map(|tier| {
                ["20x", "5x"]
                    .into_iter()
                    .find(|m| tier.ends_with(&format!("_{}", m)))
            });
            return Some(match tier {
                Some(multiple) => format!("Max {}", multiple),
                None => "Max".to_string(),
            });
        }
        "claude_team" => "Team",
        "claude_enterprise" => "Enterprise",
        _ => return None,
    };
    Some(label.to_string())
}

/// For each profile signed into the same account as another, those others
///
/// Two profiles on one login share one quota, which defeats the point of
/// having two; Settings warns about it. Profiles not signed into a
/// subscription, or whose account cannot be identified, share with nobody.
pub fn shared_accounts<'a>(
    profiles: impl IntoIterator<Item = (uuid::Uuid, &'a ClaudeAuth)>,
) -> HashMap<uuid::Uuid, Vec<uuid::Uuid>> {
    let mut by_identity: HashMap<(String, Option<String>), Vec<uuid::Uuid>> = HashMap::new();
    for (id, auth) in profiles {
        if let ClaudeAuth::Subscription(account) = auth {
            if let Some(identity) = account.identity() {
                by_identity.entry(identity).or_default().push(id);
            }
        }
    }
    let mut shared = HashMap::new();
    for ids in by_identity.into_values().filter(|ids| ids.len() > 1) {
        for &id in &ids {
            shared.insert(id, ids.iter().copied().filter(|&o| o != id).collect());
        }
    }
    shared
}

/// The command that signs a profile in, ready to paste into a shell
///
/// `config_dir` is the profile's `CLAUDE_CONFIG_DIR`, or `None` for the
/// default profile - which needs none unless Panoptes itself was started with
/// one, since its sessions inherit that.
pub fn login_command(config_dir: Option<&Path>) -> String {
    let dir = config_dir.map(Path::to_path_buf).or_else(|| {
        std::env::var_os("CLAUDE_CONFIG_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    });
    match dir {
        Some(dir) => format!(
            "CLAUDE_CONFIG_DIR={} claude auth login",
            shell_path(&dir, dirs::home_dir().as_deref())
        ),
        None => "claude auth login".to_string(),
    }
}

/// A path as a shell word: `~/...` when it is under home and needs no
/// quoting, else single-quoted when it has anything a shell would mangle
fn shell_path(path: &Path, home: Option<&Path>) -> String {
    let plain = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+,@:%".contains(c))
    };
    let full = path.to_string_lossy();
    if let Some(rest) = home.and_then(|h| path.strip_prefix(h).ok()) {
        let rest = rest.to_string_lossy();
        if !rest.is_empty() && plain(&rest) {
            return format!("~/{}", rest);
        }
    }
    if plain(&full) {
        full.into_owned()
    } else {
        format!("'{}'", full.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: &str = include_str!("fixtures/claude_json/max.json");
    const TEAM: &str = include_str!("fixtures/claude_json/team.json");
    const SIGNED_OUT: &str = include_str!("fixtures/claude_json/signed_out.json");

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn account(auth: ClaudeAuth) -> ClaudeAccount {
        match auth {
            ClaudeAuth::Subscription(account) => account,
            other => panic!("expected a subscription, got {other:?}"),
        }
    }

    #[test]
    fn test_parses_a_max_login() {
        let account = account(parse_auth(Some(MAX), None, &no_env));
        assert_eq!(account.email.as_deref(), Some("user@example.com"));
        assert_eq!(account.plan.as_deref(), Some("Max 20x"));
        assert_eq!(account.organization.as_deref(), Some("Example Org"));
        assert_eq!(
            account.account_uuid.as_deref(),
            Some("00000000-0000-4000-8000-000000000001")
        );
    }

    #[test]
    fn test_parses_a_team_login() {
        let account = account(parse_auth(Some(TEAM), None, &no_env));
        assert_eq!(account.email.as_deref(), Some("dev@example.com"));
        assert_eq!(account.plan.as_deref(), Some("Team"));
    }

    #[test]
    fn test_prefers_claudes_own_plan_name() {
        let json = r#"{"oauthAccount":{"emailAddress":"a@example.com","organizationType":"claude_max","planDisplayName":"Max (20x)"}}"#;
        assert_eq!(
            account(parse_auth(Some(json), None, &no_env))
                .plan
                .as_deref(),
            Some("Max (20x)")
        );
        let pro =
            r#"{"oauthAccount":{"emailAddress":"a@example.com","organizationType":"claude_pro"}}"#;
        assert_eq!(
            account(parse_auth(Some(pro), None, &no_env))
                .plan
                .as_deref(),
            Some("Pro")
        );
        let unknown = r#"{"oauthAccount":{"emailAddress":"a@example.com","organizationType":"something_new"}}"#;
        assert_eq!(account(parse_auth(Some(unknown), None, &no_env)).plan, None);
    }

    #[test]
    fn test_missing_file_or_account_is_signed_out() {
        assert_eq!(parse_auth(None, None, &no_env), ClaudeAuth::SignedOut);
        assert_eq!(parse_auth(Some(""), None, &no_env), ClaudeAuth::SignedOut);
        assert_eq!(
            parse_auth(Some(SIGNED_OUT), None, &no_env),
            ClaudeAuth::SignedOut
        );
        // An account object with nothing in it is no login either
        assert_eq!(
            parse_auth(Some(r#"{"oauthAccount":{}}"#), None, &no_env),
            ClaudeAuth::SignedOut
        );
    }

    #[test]
    fn test_garbled_file_is_unreadable() {
        assert!(matches!(
            parse_auth(Some(r#"{"oauthAccount": {"emailAdd"#), None, &no_env),
            ClaudeAuth::Unreadable(_)
        ));
    }

    #[test]
    fn test_api_key_and_cloud_setups_name_their_auth() {
        let stored_key = r#"{"primaryApiKey":"sk-ant-synthetic"}"#;
        assert_eq!(
            parse_auth(Some(stored_key), None, &no_env),
            ClaudeAuth::Other("API key")
        );

        // A cloud provider in the profile's settings wins over the login
        let bedrock = r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1"}}"#;
        assert_eq!(
            parse_auth(Some(MAX), Some(bedrock), &no_env),
            ClaudeAuth::Other("Bedrock")
        );
        // ...and in Panoptes' own environment
        let vertex = |name: &str| (name == "CLAUDE_CODE_USE_VERTEX").then(|| "true".to_string());
        assert_eq!(
            parse_auth(Some(MAX), None, &vertex),
            ClaudeAuth::Other("Vertex AI")
        );
        // Switched off is off
        let off = r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"0"}}"#;
        assert!(matches!(
            parse_auth(Some(MAX), Some(off), &no_env),
            ClaudeAuth::Subscription(_)
        ));

        let helper = r#"{"apiKeyHelper":"~/bin/key.sh"}"#;
        assert_eq!(
            parse_auth(Some(MAX), Some(helper), &no_env),
            ClaudeAuth::Other("API key helper")
        );
    }

    /// An `ANTHROPIC_API_KEY` in the environment is only used once approved
    #[test]
    fn test_env_api_key_counts_only_when_approved() {
        let key = |name: &str| {
            (name == "ANTHROPIC_API_KEY")
                .then(|| "sk-ant-api03-abcdefghijklmnopqrstuvwxyz".to_string())
        };
        assert!(matches!(
            parse_auth(Some(MAX), None, &key),
            ClaudeAuth::Subscription(_)
        ));
        let approved =
            r#"{"customApiKeyResponses":{"approved":["ghijklmnopqrstuvwxyz"],"rejected":[]}}"#;
        assert_eq!(
            parse_auth(Some(approved), None, &key),
            ClaudeAuth::Other("API key")
        );
    }

    #[test]
    fn test_identity_is_the_account_in_its_organisation() {
        let a = account(parse_auth(Some(MAX), None, &no_env));
        let mut same = a.clone();
        same.email = Some("USER@example.com".to_string());
        assert_eq!(a.identity(), same.identity());

        let mut other_org = a.clone();
        other_org.organization_uuid = Some("00000000-0000-4000-8000-0000000000ff".to_string());
        assert_ne!(a.identity(), other_org.identity());

        // Without an account id, the email stands in, case-blind
        let by_email = |email: &str| ClaudeAccount {
            email: Some(email.to_string()),
            ..Default::default()
        };
        assert_eq!(
            by_email("A@example.com").identity(),
            by_email("a@example.com").identity()
        );
        assert_eq!(ClaudeAccount::default().identity(), None);
    }

    #[test]
    fn test_reads_a_profiles_own_claude_json() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(read_auth(Some(dir.path())), ClaudeAuth::SignedOut);
        assert_eq!(login_files_mtime(Some(dir.path())), (None, None));

        std::fs::write(dir.path().join(".claude.json"), MAX).unwrap();
        assert!(matches!(
            read_auth(Some(dir.path())),
            ClaudeAuth::Subscription(_)
        ));
        assert!(login_files_mtime(Some(dir.path())).0.is_some());
    }

    #[test]
    fn test_shared_accounts_finds_duplicate_logins() {
        let max = parse_auth(Some(MAX), None, &no_env);
        let team = parse_auth(Some(TEAM), None, &no_env);
        let [a, b, c, d, e] = [(); 5].map(|_| uuid::Uuid::new_v4());
        let profiles = [
            (a, &max),
            (b, &team),
            (c, &max),
            (d, &ClaudeAuth::SignedOut),
            (e, &ClaudeAuth::SignedOut),
        ];

        let shared = shared_accounts(profiles);
        assert_eq!(shared.get(&a), Some(&vec![c]));
        assert_eq!(shared.get(&c), Some(&vec![a]));
        // A distinct login, and two signed-out profiles, share nothing
        assert!(!shared.contains_key(&b));
        assert!(!shared.contains_key(&d));
        assert!(!shared.contains_key(&e));
    }

    #[test]
    fn test_login_command_names_the_profiles_dir() {
        let home = Path::new("/Users/someone");
        assert_eq!(
            shell_path(Path::new("/Users/someone/.claude-work"), Some(home)),
            "~/.claude-work"
        );
        assert_eq!(
            shell_path(Path::new("/opt/claude"), Some(home)),
            "/opt/claude"
        );
        assert_eq!(
            shell_path(Path::new("/Users/someone/My Claude"), Some(home)),
            "'/Users/someone/My Claude'"
        );
        assert_eq!(shell_path(Path::new("/tmp/it's"), None), r"'/tmp/it'\''s'");
        assert_eq!(
            login_command(Some(Path::new("/opt/claude"))),
            "CLAUDE_CONFIG_DIR=/opt/claude claude auth login"
        );
    }
}
