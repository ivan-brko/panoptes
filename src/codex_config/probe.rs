//! Asking an idle Codex account who it is logged in as, and how much quota it has
//!
//! A running Codex session reports its rate limits in its rollout, but an
//! account with no session reports nothing - so before this, the only way to
//! find the account with headroom was to spawn one and look. The probe spawns
//! `codex app-server` under the account's resolved `CODEX_HOME` and speaks its
//! JSON-RPC over stdio: `initialize`, `initialized`, `account/read` (login,
//! email, plan), then `account/rateLimits/read` (the five-hour and weekly
//! windows, and a spend-control limit when the workspace sets one).
//!
//! Lessons the probe lives by, from Codex 0.156.1 and from T3 Code's own probe:
//!
//! - **It is never killed early.** Codex starts git (ssh for ssh remotes) for
//!   its plugin marketplaces at launch, and a probe killed mid-fetch strands
//!   ssh. Once answered, stdin is closed and Codex exits on its own; only one
//!   that is still running after [`EXIT_GRACE`] is killed.
//! - **Quota is read only where it means something**: a ChatGPT login whose
//!   model traffic goes to OpenAI itself. An API key has no plan quota, and a
//!   custom `model_provider` spends someone else's.
//! - **Usage is optional.** A failed or slow `account/rateLimits/read` costs
//!   the usage figures, never the login status.
//! - **`CODEX_INTERNAL_ORIGINATOR_OVERRIDE` is left unset.** app-server
//!   refreshes `models_cache.json` before `initialize`, under the default
//!   originator - the plain CLI's, which is what Panoptes sessions run. With
//!   the override set, that refresh would file another client's model list in
//!   the home the sessions read.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{json, Value};

use crate::agent::events::RateLimitWindow;

/// How long the handshake and `account/read` may take together
///
/// Generous because a cold start runs Codex's own startup work first.
pub const ACCOUNT_TIMEOUT: Duration = Duration::from_secs(20);

/// How long `account/rateLimits/read` may take, after the account answered
pub const USAGE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a probe is left to exit by itself once its stdin is closed
pub const EXIT_GRACE: Duration = Duration::from_secs(60);

/// The client name Codex sees, and stamps on its user agent
const CLIENT_NAME: &str = "panoptes";

/// The five-hour window's length, for a reading that omits it
const SESSION_MINS: u64 = 5 * 60;
/// The weekly window's length
const WEEK_MINS: u64 = 7 * 24 * 60;
/// The monthly window Free and Go plans have instead of a five-hour one
const MONTH_MINS: u64 = 30 * 24 * 60;

/// The deadlines one probe runs against
#[derive(Debug, Clone, Copy)]
pub struct ProbeBudget {
    pub account: Duration,
    pub usage: Duration,
}

impl Default for ProbeBudget {
    fn default() -> Self {
        Self {
            account: ACCOUNT_TIMEOUT,
            usage: USAGE_TIMEOUT,
        }
    }
}

/// How an account is logged in
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexLogin {
    /// No credentials at all: `codex login` has not been run for this home
    LoggedOut,
    /// An OpenAI API key, which has no plan quota
    ApiKey,
    /// A ChatGPT login
    ChatGpt {
        email: Option<String>,
        /// Plan as a label (`Plus`, `Pro 200`, ...)
        plan: Option<String>,
    },
    /// A login kind this Panoptes does not know (`amazonBedrock`, or newer)
    Other(String),
}

/// What is known of an account's quota
#[derive(Debug, Clone, PartialEq)]
pub enum CodexUsage {
    /// Read from Codex
    Known(CodexQuota),
    /// Quota does not apply to this account; the reason is shown as-is
    NotApplicable(&'static str),
    /// It should apply, but could not be read this time
    Unavailable(String),
}

/// An account's quota, as `account/rateLimits/read` reported it
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodexQuota {
    /// The short window (five hours; a month on Free and Go)
    pub primary: Option<RateLimitWindow>,
    /// The weekly window
    pub secondary: Option<RateLimitWindow>,
    /// A workspace's spend-control limit, as a share used
    pub individual: Option<RateLimitWindow>,
    /// Why Codex has stopped accepting turns, when it has
    pub limit_reached: Option<String>,
    /// The plan the usage read names, as a label
    pub plan: Option<String>,
}

impl CodexQuota {
    /// The windows that are known, in display order, with their labels
    pub fn windows(&self) -> Vec<(String, &RateLimitWindow)> {
        let mut windows: Vec<(String, &RateLimitWindow)> = Vec::new();
        for window in [&self.primary, &self.secondary].into_iter().flatten() {
            windows.push((window.label(), window));
        }
        if let Some(window) = &self.individual {
            windows.push(("credit".to_string(), window));
        }
        windows
    }
}

/// Everything one probe learned about an account
#[derive(Debug, Clone, PartialEq)]
pub struct CodexAccountStatus {
    pub login: CodexLogin,
    pub usage: CodexUsage,
}

/// A probe's outcome: the status, or why even the login could not be read
pub type ProbeResult = Result<CodexAccountStatus, String>;

/// Why a JSON-RPC request got no result
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcError {
    /// No answer before the deadline
    Timeout,
    /// Codex closed its stdout (exited) before answering
    Closed,
    /// Codex answered with an error
    Server { code: i64, message: String },
    /// The request could not be written
    Io(String),
}

impl RpcError {
    /// One line for the user, naming the step that failed
    fn describe(&self, step: &str) -> String {
        match self {
            RpcError::Timeout => format!("Codex did not answer {} in time", step),
            RpcError::Closed => format!("Codex exited before answering {}", step),
            RpcError::Server { message, .. } => format!("Codex refused {}: {}", step, message),
            RpcError::Io(e) => format!("could not ask Codex for {}: {}", step, e),
        }
    }
}

/// A JSON-RPC client for `codex app-server`'s stdio transport
///
/// Messages are one JSON object per line, without the `jsonrpc` member.
/// Lines arrive on a channel, fed by a reader thread in the real probe and by
/// a fixture in tests, so a request can wait with a deadline. Notifications
/// and answers to other requests are skipped while waiting.
pub struct RpcClient<W: Write> {
    writer: W,
    lines: Receiver<String>,
    next_id: i64,
}

impl<W: Write> RpcClient<W> {
    pub fn new(writer: W, lines: Receiver<String>) -> Self {
        Self {
            writer,
            lines,
            next_id: 1,
        }
    }

    fn send(&mut self, message: &Value) -> Result<(), RpcError> {
        let mut line = message.to_string();
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .and_then(|()| self.writer.flush())
            .map_err(|e| RpcError::Io(e.to_string()))
    }

    /// Send a notification, which has no answer
    pub fn notify(&mut self, method: &str) -> Result<(), RpcError> {
        self.send(&json!({ "method": method }))
    }

    /// Send a request and wait, until `deadline`, for its result
    pub fn request(
        &mut self,
        method: &str,
        params: Option<Value>,
        deadline: Instant,
    ) -> Result<Value, RpcError> {
        let id = self.next_id;
        self.next_id += 1;
        let mut message = json!({ "id": id, "method": method });
        if let Some(params) = params {
            message["params"] = params;
        }
        self.send(&message)?;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RpcError::Timeout);
            }
            let line = match self.lines.recv_timeout(remaining) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => return Err(RpcError::Timeout),
                Err(RecvTimeoutError::Disconnected) => return Err(RpcError::Closed),
            };
            let Ok(reply) = serde_json::from_str::<Value>(&line) else {
                tracing::debug!(line = %line, "Ignoring a non-JSON line from codex app-server");
                continue;
            };
            if reply.get("id").and_then(Value::as_i64) != Some(id) {
                continue;
            }
            if let Some(error) = reply.get("error") {
                return Err(RpcError::Server {
                    code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                    message: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                        .to_string(),
                });
            }
            return Ok(reply.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

/// Run the probe's conversation over an already-connected client
///
/// `native_quota` is whether this home's model traffic goes to OpenAI itself
/// ([`routes_to_native_openai`]); without it, quota is not read.
pub fn run_probe<W: Write>(
    client: &mut RpcClient<W>,
    native_quota: bool,
    budget: ProbeBudget,
) -> ProbeResult {
    let deadline = Instant::now() + budget.account;
    client
        .request(
            "initialize",
            Some(json!({
                "clientInfo": {
                    "name": CLIENT_NAME,
                    "title": "Panoptes",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            })),
            deadline,
        )
        .map_err(|e| e.describe("initialize"))?;
    client
        .notify("initialized")
        .map_err(|e| e.describe("initialized"))?;
    let account = client
        .request("account/read", Some(json!({})), deadline)
        .map_err(|e| e.describe("account/read"))?;
    let login = parse_account(&account);

    let usage = match &login {
        CodexLogin::LoggedOut => CodexUsage::NotApplicable("not logged in"),
        CodexLogin::ApiKey => CodexUsage::NotApplicable("API key, no plan quota"),
        CodexLogin::Other(_) => CodexUsage::NotApplicable("no plan quota"),
        CodexLogin::ChatGpt { .. } if !native_quota => {
            CodexUsage::NotApplicable("custom provider, no plan quota")
        }
        CodexLogin::ChatGpt { .. } => {
            match client.request(
                "account/rateLimits/read",
                None,
                Instant::now() + budget.usage,
            ) {
                Ok(limits) => CodexUsage::Known(parse_rate_limits(&limits)),
                Err(e) => {
                    tracing::debug!(error = ?e, "Codex rate-limit read failed");
                    CodexUsage::Unavailable(e.describe("the usage request"))
                }
            }
        }
    };

    // A plan the usage read names is the backend's word for it now; the one
    // `account/read` gave comes from the cached login token, and can be stale
    let login = match (login, &usage) {
        (CodexLogin::ChatGpt { email, plan }, CodexUsage::Known(quota)) => CodexLogin::ChatGpt {
            email,
            plan: quota.plan.clone().or(plan),
        },
        (login, _) => login,
    };

    Ok(CodexAccountStatus { login, usage })
}

/// Read `account/read`'s result
pub fn parse_account(result: &Value) -> CodexLogin {
    let Some(account) = result.get("account").filter(|a| !a.is_null()) else {
        return CodexLogin::LoggedOut;
    };
    match account.get("type").and_then(Value::as_str) {
        Some("apiKey") => CodexLogin::ApiKey,
        Some("chatgpt") => CodexLogin::ChatGpt {
            email: account
                .get("email")
                .and_then(Value::as_str)
                .map(str::to_string),
            plan: account
                .get("planType")
                .and_then(Value::as_str)
                .and_then(plan_label),
        },
        Some(other) => CodexLogin::Other(other.to_string()),
        None => CodexLogin::Other("unknown".to_string()),
    }
}

/// Read `account/rateLimits/read`'s result
///
/// The `codex` bucket of `rateLimitsByLimitId` when there is one, else the
/// single-bucket view. A bucket for another limit (a model-specific one) is
/// not the account's allowance and yields no windows.
pub fn parse_rate_limits(result: &Value) -> CodexQuota {
    let snapshot = result
        .pointer("/rateLimitsByLimitId/codex")
        .filter(|s| !s.is_null())
        .or_else(|| result.get("rateLimits"))
        .cloned()
        .unwrap_or(Value::Null);
    if snapshot
        .get("limitId")
        .and_then(Value::as_str)
        .is_some_and(|id| id != "codex")
    {
        return CodexQuota::default();
    }
    let plan = snapshot.get("planType").and_then(Value::as_str);
    let monthly_plan = matches!(plan, Some("free") | Some("go"));
    let primary_fallback = if monthly_plan {
        MONTH_MINS
    } else {
        SESSION_MINS
    };

    CodexQuota {
        primary: snapshot
            .get("primary")
            .and_then(|w| parse_window(w, primary_fallback)),
        secondary: snapshot
            .get("secondary")
            .and_then(|w| parse_window(w, WEEK_MINS)),
        individual: snapshot.get("individualLimit").and_then(parse_individual),
        limit_reached: snapshot
            .get("rateLimitReachedType")
            .and_then(Value::as_str)
            .map(str::to_string),
        plan: plan.and_then(plan_label),
    }
}

/// One window; `fallback_mins` when the reading does not say how long it is
fn parse_window(window: &Value, fallback_mins: u64) -> Option<RateLimitWindow> {
    let used = window.get("usedPercent").and_then(Value::as_f64)?;
    Some(RateLimitWindow {
        used_percent: used.clamp(0.0, 100.0),
        window_minutes: Some(
            window
                .get("windowDurationMins")
                .and_then(Value::as_u64)
                .unwrap_or(fallback_mins),
        ),
        resets_at: window.get("resetsAt").and_then(epoch_seconds),
    })
}

/// A spend-control limit, which reports what is *left*
fn parse_individual(limit: &Value) -> Option<RateLimitWindow> {
    let remaining = limit.get("remainingPercent").and_then(Value::as_f64)?;
    Some(RateLimitWindow {
        used_percent: (100.0 - remaining).clamp(0.0, 100.0),
        window_minutes: None,
        resets_at: limit.get("resetsAt").and_then(epoch_seconds),
    })
}

fn epoch_seconds(value: &Value) -> Option<DateTime<Utc>> {
    let secs = value.as_i64().filter(|s| *s > 0)?;
    Utc.timestamp_opt(secs, 0).single()
}

/// The label for a Codex plan type, `None` for no plan worth naming
///
/// Pro tiers carry their price because Codex names three of them (`prolite`,
/// `pro`, `promax`). An unrecognised value is shown as Codex spelled it.
pub fn plan_label(plan_type: &str) -> Option<String> {
    let label = match plan_type {
        "free" => "Free",
        "go" => "Go",
        "plus" => "Plus",
        "pro" => "Pro 200",
        "prolite" => "Pro 100",
        "promax" => "Pro 500",
        "self_serve_business_prolite" => "Business Premium",
        "team" | "self_serve_business_usage_based" | "business" => "Business",
        "ent26" | "enterprise_cbp_automation" | "enterprise_cbp_usage_based" | "enterprise" => {
            "Enterprise"
        }
        "edu" | "edu_plus" | "edu_pro" => "Edu",
        "unknown" => return None,
        "" => return None,
        other => return Some(other.to_string()),
    };
    Some(label.to_string())
}

/// Whether a Codex home's model traffic goes to OpenAI itself
///
/// Only then does the account's plan quota pay for it. Reads the top level of
/// `config.toml`: a `model_provider` other than `openai`, or an
/// `openai_base_url` pointing anywhere but OpenAI, routes elsewhere. A file
/// that cannot be read is no file - Codex's default is OpenAI.
pub fn routes_to_native_openai(config_toml: Option<&str>) -> bool {
    let Some(text) = config_toml else {
        return true;
    };
    let Ok(config) = text.parse::<toml::Table>() else {
        return true;
    };
    let provider = config
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .unwrap_or("openai");
    if provider != "openai" {
        return false;
    }
    match config.get("openai_base_url").and_then(toml::Value::as_str) {
        None | Some("") => true,
        Some(url) => is_openai_url(url),
    }
}

fn is_openai_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', ':', '?']).next().unwrap_or("");
    host == "api.openai.com" || host == "chatgpt.com"
}

/// What one probe needs to know about its account
#[derive(Debug, Clone)]
pub struct ProbeRequest {
    /// The `CODEX_HOME` the account's sessions spawn with (a shadow, or its own)
    pub codex_home: PathBuf,
    /// `CODEX_SQLITE_HOME`, set exactly when sessions get it
    pub sqlite_home: Option<PathBuf>,
    /// The `codex` binary
    pub binary: String,
}

/// Probe one account: spawn `codex app-server`, ask, and let it exit
///
/// Blocks for at most the budget; the exit is waited for on a thread of its
/// own, so a slow Codex costs nothing once it has answered.
pub fn probe_account(request: &ProbeRequest, budget: ProbeBudget) -> ProbeResult {
    let config_toml = std::fs::read_to_string(request.codex_home.join("config.toml")).ok();
    let native_quota = routes_to_native_openai(config_toml.as_deref());

    let mut command = Command::new(&request.binary);
    command
        .arg("app-server")
        .env("CODEX_HOME", &request.codex_home)
        .env_remove("CODEX_INTERNAL_ORIGINATOR_OVERRIDE")
        .current_dir(dirs::home_dir().unwrap_or_else(std::env::temp_dir))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(sqlite_home) = &request.sqlite_home {
        command.env("CODEX_SQLITE_HOME", sqlite_home);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start `{} app-server`: {}", request.binary, e))?;

    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        reap(child);
        return Err("codex app-server has no stdio".to_string());
    };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                // Nobody is listening any more; keep draining so Codex
                // never blocks on a full pipe while it winds down
                continue;
            }
        }
    });

    let mut client = RpcClient::new(stdin, rx);
    let result = run_probe(&mut client, native_quota, budget);
    // Closing stdin is how app-server is told to finish
    drop(client);
    reap(child);
    result
}

/// Wait out a probe's exit on a thread of its own, killing it only if it hangs
fn reap(mut child: Child) {
    std::thread::spawn(move || {
        let deadline = Instant::now() + EXIT_GRACE;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(200))
                }
                Ok(None) => {
                    tracing::warn!(
                        pid = child.id(),
                        "codex app-server probe did not exit; killing it"
                    );
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to wait for codex app-server probe");
                    return;
                }
            }
        }
    });
}

/// `auth.json`'s modification time under an account's own home
///
/// `codex login` and `codex logout` write or remove it, so a change here is
/// the cue to probe again. `None` when there is no file (logged out, or a
/// keyring login).
pub fn auth_mtime(account_home: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(account_home.join("auth.json"))
        .and_then(|m| m.modified())
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::Sender;

    const CHATGPT_PLUS: &str = include_str!("fixtures/app_server/chatgpt_plus.jsonl");
    const API_KEY: &str = include_str!("fixtures/app_server/api_key.jsonl");
    const LOGGED_OUT: &str = include_str!("fixtures/app_server/logged_out.jsonl");

    fn short_budget() -> ProbeBudget {
        ProbeBudget {
            account: Duration::from_millis(200),
            usage: Duration::from_millis(100),
        }
    }

    /// A client whose Codex "says" `lines`, plus the sender keeping it open
    fn client_over(lines: &[&str]) -> (RpcClient<Vec<u8>>, Sender<String>) {
        let (tx, rx) = mpsc::channel();
        for line in lines {
            tx.send(line.to_string()).unwrap();
        }
        (RpcClient::new(Vec::new(), rx), tx)
    }

    fn fixture_lines(fixture: &str) -> Vec<&str> {
        fixture.lines().filter(|l| !l.trim().is_empty()).collect()
    }

    /// The fixture without its answer to request 3 (`account/rateLimits/read`)
    fn without_usage_answer(fixture: &str) -> Vec<&str> {
        fixture_lines(fixture)
            .into_iter()
            .filter(|l| !l.starts_with("{\"id\":3"))
            .collect()
    }

    fn sent(client: &RpcClient<Vec<u8>>) -> Vec<Value> {
        String::from_utf8(client.writer.clone())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn asked_for_usage(client: &RpcClient<Vec<u8>>) -> bool {
        sent(client)
            .iter()
            .any(|m| m["method"] == "account/rateLimits/read")
    }

    #[test]
    fn chatgpt_fixture_reads_login_plan_and_both_windows() {
        let (mut client, _tx) = client_over(&fixture_lines(CHATGPT_PLUS));

        let status = run_probe(&mut client, true, short_budget()).unwrap();

        assert_eq!(
            status.login,
            CodexLogin::ChatGpt {
                email: Some("user@example.com".to_string()),
                plan: Some("Plus".to_string()),
            }
        );
        let CodexUsage::Known(quota) = status.usage else {
            panic!("expected usage, got {:?}", status.usage);
        };
        let primary = quota.primary.unwrap();
        assert_eq!(primary.used_percent, 0.0);
        assert_eq!(primary.window_minutes, Some(300));
        assert_eq!(primary.resets_at.unwrap().timestamp(), 1_795_447_950);
        let secondary = quota.secondary.unwrap();
        assert_eq!(secondary.window_minutes, Some(10_080));
        assert_eq!(secondary.resets_at.unwrap().timestamp(), 1_796_034_750);
        assert!(quota.individual.is_none());
        assert!(quota.limit_reached.is_none());
    }

    #[test]
    fn handshake_is_initialize_then_initialized_then_reads() {
        let (mut client, _tx) = client_over(&fixture_lines(CHATGPT_PLUS));
        run_probe(&mut client, true, short_budget()).unwrap();

        let sent = sent(&client);
        let methods: Vec<&str> = sent.iter().map(|m| m["method"].as_str().unwrap()).collect();
        assert_eq!(
            methods,
            [
                "initialize",
                "initialized",
                "account/read",
                "account/rateLimits/read"
            ]
        );
        assert_eq!(sent[0]["id"], 1);
        assert_eq!(sent[0]["params"]["clientInfo"]["name"], "panoptes");
        // A notification carries no id, and nothing names a JSON-RPC version
        assert!(sent[1].get("id").is_none());
        assert!(sent.iter().all(|m| m.get("jsonrpc").is_none()));
        assert_eq!(sent[2]["id"], 2);
        assert_eq!(sent[3]["id"], 3);
    }

    #[test]
    fn logged_out_fixture_reports_logged_out_without_reading_usage() {
        let (mut client, _tx) = client_over(&fixture_lines(LOGGED_OUT));

        let status = run_probe(&mut client, true, short_budget()).unwrap();

        assert_eq!(status.login, CodexLogin::LoggedOut);
        assert_eq!(status.usage, CodexUsage::NotApplicable("not logged in"));
        assert!(!asked_for_usage(&client));
    }

    #[test]
    fn api_key_fixture_has_no_quota_and_skips_the_usage_read() {
        let (mut client, _tx) = client_over(&fixture_lines(API_KEY));

        let status = run_probe(&mut client, true, short_budget()).unwrap();

        assert_eq!(status.login, CodexLogin::ApiKey);
        assert!(matches!(status.usage, CodexUsage::NotApplicable(_)));
        assert!(!asked_for_usage(&client));
    }

    #[test]
    fn custom_provider_skips_the_usage_read() {
        let (mut client, _tx) = client_over(&fixture_lines(CHATGPT_PLUS));

        let status = run_probe(&mut client, false, short_budget()).unwrap();

        assert!(matches!(status.login, CodexLogin::ChatGpt { .. }));
        assert_eq!(
            status.usage,
            CodexUsage::NotApplicable("custom provider, no plan quota")
        );
        assert!(!asked_for_usage(&client));
    }

    #[test]
    fn silent_codex_times_out_the_probe() {
        let (mut client, _tx) = client_over(&[]);
        let started = Instant::now();

        let error = run_probe(&mut client, true, short_budget()).unwrap_err();

        assert!(error.contains("did not answer initialize"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn exited_codex_fails_the_probe() {
        let (mut client, tx) = client_over(&[]);
        drop(tx);

        let error = run_probe(&mut client, true, short_budget()).unwrap_err();

        assert!(error.contains("exited before answering"), "{error}");
    }

    #[test]
    fn slow_usage_read_degrades_to_no_usage_and_keeps_the_login() {
        // Codex answers the handshake and the account, then goes quiet
        let (mut client, _tx) = client_over(&without_usage_answer(CHATGPT_PLUS));

        let status = run_probe(&mut client, true, short_budget()).unwrap();

        assert!(matches!(
            status.login,
            CodexLogin::ChatGpt { ref email, .. } if email.as_deref() == Some("user@example.com")
        ));
        match status.usage {
            CodexUsage::Unavailable(reason) => assert!(reason.contains("in time"), "{reason}"),
            other => panic!("expected unavailable usage, got {other:?}"),
        }
    }

    #[test]
    fn refused_usage_read_degrades_to_no_usage() {
        let mut lines = without_usage_answer(CHATGPT_PLUS);
        lines.push(r#"{"error":{"code":-32600,"message":"backend unavailable"},"id":3}"#);
        let (mut client, _tx) = client_over(&lines);

        let status = run_probe(&mut client, true, short_budget()).unwrap();

        match status.usage {
            CodexUsage::Unavailable(reason) => {
                assert!(reason.contains("backend unavailable"), "{reason}")
            }
            other => panic!("expected unavailable usage, got {other:?}"),
        }
    }

    #[test]
    fn request_skips_notifications_and_other_ids() {
        let (mut client, _tx) = client_over(&[
            r#"{"method":"account/updated","params":{}}"#,
            "not json",
            r#"{"id":99,"result":{"wrong":true}}"#,
            r#"{"id":1,"result":{"right":true}}"#,
        ]);

        let result = client
            .request("x", None, Instant::now() + Duration::from_millis(200))
            .unwrap();

        assert_eq!(result, json!({ "right": true }));
    }

    #[test]
    fn rate_limits_prefer_the_codex_bucket() {
        let result = json!({
            "rateLimits": { "limitId": "codex", "primary": { "usedPercent": 1 } },
            "rateLimitsByLimitId": {
                "codex": { "limitId": "codex", "primary": { "usedPercent": 42, "windowDurationMins": 300 } },
                "gpt-x": { "limitId": "gpt-x", "primary": { "usedPercent": 99 } }
            }
        });

        let quota = parse_rate_limits(&result);

        assert_eq!(quota.primary.unwrap().used_percent, 42.0);
    }

    #[test]
    fn rate_limits_for_another_limit_yield_no_windows() {
        let result = json!({
            "rateLimits": { "limitId": "gpt-x", "primary": { "usedPercent": 99 } },
            "rateLimitsByLimitId": null
        });

        assert_eq!(parse_rate_limits(&result), CodexQuota::default());
    }

    #[test]
    fn missing_durations_fall_back_by_plan() {
        let paid = parse_rate_limits(&json!({ "rateLimits": {
            "planType": "pro",
            "primary": { "usedPercent": 10 },
            "secondary": { "usedPercent": 20 }
        }}));
        assert_eq!(paid.primary.unwrap().window_minutes, Some(SESSION_MINS));
        assert_eq!(paid.secondary.unwrap().window_minutes, Some(WEEK_MINS));
        assert_eq!(paid.plan.as_deref(), Some("Pro 200"));

        let free = parse_rate_limits(&json!({ "rateLimits": {
            "planType": "free",
            "primary": { "usedPercent": 10 }
        }}));
        assert_eq!(free.primary.unwrap().window_minutes, Some(MONTH_MINS));
    }

    #[test]
    fn individual_limit_reports_the_share_used_and_limit_reached() {
        let quota = parse_rate_limits(&json!({ "rateLimits": {
            "limitId": "codex",
            "individualLimit": {
                "limit": "100", "used": "75", "remainingPercent": 25, "resetsAt": 1_800_000_000
            },
            "rateLimitReachedType": "workspace_member_usage_limit_reached"
        }}));

        let individual = quota.individual.unwrap();
        assert_eq!(individual.used_percent, 75.0);
        assert_eq!(individual.resets_at.unwrap().timestamp(), 1_800_000_000);
        assert_eq!(
            quota.limit_reached.as_deref(),
            Some("workspace_member_usage_limit_reached")
        );
    }

    #[test]
    fn usage_plan_overrides_a_stale_login_plan_unless_unknown() {
        let lines = [
            r#"{"id":1,"result":{}}"#,
            r#"{"id":2,"result":{"account":{"type":"chatgpt","email":null,"planType":"plus"},"requiresOpenaiAuth":true}}"#,
            r#"{"id":3,"result":{"rateLimits":{"limitId":"codex","planType":"prolite"}}}"#,
        ];
        let (mut client, _tx) = client_over(&lines);
        let status = run_probe(&mut client, true, short_budget()).unwrap();
        assert_eq!(
            status.login,
            CodexLogin::ChatGpt {
                email: None,
                plan: Some("Pro 100".to_string())
            }
        );

        let lines = [
            lines[0],
            lines[1],
            r#"{"id":3,"result":{"rateLimits":{"limitId":"codex","planType":"unknown"}}}"#,
        ];
        let (mut client, _tx) = client_over(&lines);
        let status = run_probe(&mut client, true, short_budget()).unwrap();
        assert!(matches!(
            status.login,
            CodexLogin::ChatGpt { ref plan, .. } if plan.as_deref() == Some("Plus")
        ));
    }

    #[test]
    fn plan_labels() {
        assert_eq!(plan_label("plus").as_deref(), Some("Plus"));
        assert_eq!(plan_label("prolite").as_deref(), Some("Pro 100"));
        assert_eq!(plan_label("promax").as_deref(), Some("Pro 500"));
        assert_eq!(
            plan_label("self_serve_business_prolite").as_deref(),
            Some("Business Premium")
        );
        assert_eq!(plan_label("team").as_deref(), Some("Business"));
        assert_eq!(plan_label("edu_pro").as_deref(), Some("Edu"));
        assert_eq!(plan_label("unknown"), None);
        assert_eq!(plan_label("galaxy").as_deref(), Some("galaxy"));
    }

    #[test]
    fn native_routing_follows_provider_and_base_url() {
        assert!(routes_to_native_openai(None));
        assert!(routes_to_native_openai(Some("model = \"gpt-6\"\n")));
        assert!(routes_to_native_openai(Some(
            "model_provider = \"openai\"\n"
        )));
        assert!(!routes_to_native_openai(Some(
            "model_provider = \"ollama\"\n"
        )));
        assert!(routes_to_native_openai(Some(
            "openai_base_url = \"https://api.openai.com/v1\"\n"
        )));
        assert!(!routes_to_native_openai(Some(
            "openai_base_url = \"http://localhost:8317/v1\"\n"
        )));
        // A provider set only inside a profile is not the top-level route
        assert!(routes_to_native_openai(Some(
            "[profiles.local]\nmodel_provider = \"ollama\"\n"
        )));
        // An unreadable file is no file
        assert!(routes_to_native_openai(Some("this is = = not toml")));
    }

    #[test]
    fn missing_binary_fails_without_hanging() {
        let dir = tempfile::TempDir::new().unwrap();
        let request = ProbeRequest {
            codex_home: dir.path().to_path_buf(),
            sqlite_home: None,
            binary: "/nonexistent/panoptes-test-codex".to_string(),
        };

        let error = probe_account(&request, short_budget()).unwrap_err();

        assert!(error.contains("could not start"), "{error}");
    }

    /// Against the real `codex` on PATH, in an empty home: logged out
    ///
    /// Ignored because it needs Codex installed; run it with `--ignored`
    /// after a Codex upgrade to check the protocol still holds.
    #[test]
    #[ignore]
    fn real_codex_reports_an_empty_home_as_logged_out() {
        let dir = tempfile::TempDir::new().unwrap();
        let request = ProbeRequest {
            codex_home: dir.path().to_path_buf(),
            sqlite_home: None,
            binary: "codex".to_string(),
        };

        let status = probe_account(&request, ProbeBudget::default()).unwrap();

        assert_eq!(status.login, CodexLogin::LoggedOut);
    }

    #[test]
    fn auth_mtime_tracks_the_file() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(auth_mtime(dir.path()).is_none());
        std::fs::write(dir.path().join("auth.json"), "{}").unwrap();
        assert!(auth_mtime(dir.path()).is_some());
    }
}
