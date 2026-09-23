//! Finding conversations Panoptes did not start
//!
//! A user who has been running `claude` or `codex` by hand in a project's
//! directory has conversations on disk that Panoptes has no record of. This
//! finds the ones belonging to one working directory, so they can be adopted
//! as resumable sessions (see `SessionManager::adopt_external_conversation`).
//!
//! The two agents file their transcripts very differently, and that decides
//! how each is searched:
//!
//! - **Claude** files by directory: `<config_dir>/projects/<slug(cwd)>/`. The
//!   slug names the one directory to list, so nothing else is looked at.
//! - **Codex** files by date: `$CODEX_HOME/sessions/YYYY/MM/DD/`, every working
//!   directory mixed together. The only way to know a rollout's `cwd` is to
//!   read its header, so the tree is walked newest first and read until a
//!   budget runs out.
//!
//! Everything here is bounded, because a heavy user has thousands of rollouts
//! and transcripts of tens of megabytes. [`ScanBudget`] caps the files opened,
//! the bytes read and the results kept, and every read is a bounded prefix:
//! no transcript is ever read whole. The scan runs on a worker thread
//! (`app/background.rs`), and checks its cancel flag between files.
//!
//! Nothing read here is trusted. Files are written by other processes and may
//! be caught mid-write; a partial line, an unknown record type or an
//! unreadable file is logged at debug and skipped, never an error.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use super::codex::{self, RolloutKind};
use super::TranscriptKind;

/// How much of a Claude transcript is read for its title and first prompt
///
/// The header region: the first prompt is among the first few records, and
/// Claude writes its first `ai-title` within the first couple of turns.
const CLAUDE_HEAD_BYTES: u64 = 64 * 1024;

/// How much of a Codex rollout is read to decide whether it is ours
///
/// Enough for the `session_meta` fields that identify it (`id`, `cwd`,
/// `source`), which Codex writes ahead of the ~20 KB of base instructions
/// that make the header line long.
const CODEX_PROBE_BYTES: u64 = 8 * 1024;

/// How much of a matching Codex rollout is read, in all
///
/// The header line runs to ~22 KB, and the first user prompt follows the
/// injected developer and AGENTS.md messages - measured at 40-100 KB in.
const CODEX_HEAD_BYTES: u64 = 256 * 1024;

/// How much of the end of a `session_index.jsonl` is read for thread names
///
/// Renames are appended, so the newest names are at the end.
const SESSION_INDEX_TAIL_BYTES: u64 = 1024 * 1024;

/// Longest title kept; the overlay truncates further to fit its width
const MAX_TITLE_CHARS: usize = 120;

/// Claude caps a project directory's name at this many characters, then
/// appends `-<hash>`; a longer working directory can only be found by prefix
const CLAUDE_SLUG_MAX: usize = 200;

/// How much a scan may do before it stops
///
/// Whichever runs out first ends the scan, and the outcome says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanBudget {
    /// Files opened for reading
    pub max_files: usize,
    /// Bytes read across every file
    pub max_bytes: u64,
    /// Conversations found
    pub max_results: usize,
}

impl Default for ScanBudget {
    fn default() -> Self {
        Self {
            max_files: 500,
            max_bytes: 8 * 1024 * 1024,
            max_results: 50,
        }
    }
}

/// What a scan actually did, against its [`ScanBudget`]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanCounters {
    /// Files opened for reading
    pub files_examined: usize,
    /// Bytes read across every file
    pub bytes_read: u64,
    /// Conversations found
    pub results: usize,
}

/// One account whose transcripts are searched
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanAccount {
    /// `CLAUDE_CONFIG_DIR` or `CODEX_HOME`
    pub dir: PathBuf,
    /// The Panoptes profile for this directory; `None` for the default account
    pub config_id: Option<Uuid>,
    /// The profile's name, for display
    pub name: Option<String>,
}

/// Everything a scan needs, as plain data so it can cross to a worker thread
#[derive(Debug, Clone)]
pub struct ScanRequest {
    /// The branch's working directory: only conversations started here count
    pub working_dir: PathBuf,
    /// Claude accounts to search
    pub claude_accounts: Vec<ScanAccount>,
    /// Codex accounts to search
    pub codex_accounts: Vec<ScanAccount>,
    /// Conversation IDs already owned by a Panoptes session, never offered
    pub claimed: HashSet<String>,
    /// Limits on the work done
    pub budget: ScanBudget,
}

/// A conversation found on disk that Panoptes does not know about
#[derive(Debug, Clone, PartialEq)]
pub struct FoundConversation {
    /// Which agent wrote it
    pub kind: TranscriptKind,
    /// The agent's conversation ID - what `--resume` takes
    pub id: String,
    /// The agent's own title, else the first prompt; `None` when it has neither
    pub title: Option<String>,
    /// When the transcript was last written to
    pub last_active: DateTime<Utc>,
    /// The Panoptes profile of the account it belongs to
    pub config_id: Option<Uuid>,
    /// That profile's name
    pub account_name: Option<String>,
    /// The transcript file
    pub path: PathBuf,
}

/// The result of a scan
#[derive(Debug, Clone, Default)]
pub struct ScanOutcome {
    /// Newest first
    pub conversations: Vec<FoundConversation>,
    /// The work done
    pub counters: ScanCounters,
    /// A budget ran out before every candidate was looked at, so older
    /// conversations may exist that are not listed
    pub truncated: bool,
    /// The user cancelled; whatever was found so far is still returned
    pub cancelled: bool,
}

/// Find the conversations started in `request.working_dir`
///
/// Both agents' candidates are examined together, newest first, so a budget
/// that runs out costs the oldest conversations rather than one agent's
/// entirely. Never fails: what cannot be read is skipped.
pub fn scan(request: &ScanRequest, cancel: &AtomicBool) -> ScanOutcome {
    let target = canonical(&request.working_dir);
    let mut meter = Meter::new(request.budget);
    let mut found = Vec::new();
    let mut cancelled = false;

    let claude_accounts = dedupe_accounts(&request.claude_accounts);
    let codex_accounts = dedupe_accounts(&request.codex_accounts);

    let mut streams: Vec<Stream> = Vec::new();
    streams.push(Stream::List(claude_candidates(
        &claude_accounts,
        &request.working_dir,
        &request.claimed,
    )));
    for (index, account) in codex_accounts.iter().enumerate() {
        streams.push(Stream::Codex(CodexRollouts::new(index, &account.dir)));
    }
    // Loaded once per Codex account, and only when it turns out to hold a
    // conversation of ours
    let mut codex_names: HashMap<usize, HashMap<String, String>> = HashMap::new();

    let truncated = loop {
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break false;
        }
        // The newest candidate across every stream
        let next = streams
            .iter_mut()
            .enumerate()
            .filter_map(|(i, stream)| stream.peek_key().map(|key| (i, key)))
            .max_by_key(|(_, key)| *key)
            .map(|(i, _)| i);
        let Some(stream) = next else {
            break false;
        };
        if meter.exhausted() {
            break true;
        }
        let Some(candidate) = streams[stream].pop() else {
            continue;
        };

        let conversation = match candidate.kind {
            TranscriptKind::Claude => {
                let account = &claude_accounts[candidate.account];
                examine_claude(&mut meter, &candidate, &target, account)
            }
            TranscriptKind::Codex => {
                let account = &codex_accounts[candidate.account];
                examine_codex(&mut meter, &candidate, &target, &request.claimed).map(
                    |(id, head)| {
                        let names = codex_names
                            .entry(candidate.account)
                            .or_insert_with(|| read_thread_names(&mut meter, &account.dir));
                        let title = match names.get(&id) {
                            Some(name) => Some(clean_title(name)),
                            None => codex_first_prompt(&mut meter, &candidate.path, head),
                        };
                        found_conversation(&candidate, id, title, account)
                    },
                )
            }
        };
        if let Some(conversation) = conversation {
            meter.counters.results += 1;
            found.push(conversation);
        }
    };

    found.sort_by_key(|c| std::cmp::Reverse(c.last_active));
    ScanOutcome {
        conversations: found,
        counters: meter.counters,
        truncated,
        cancelled,
    }
}

/// Drop accounts that point at a directory already listed
///
/// The default account and a profile set up for the same directory are the
/// same transcripts; the first listed wins, which is why callers put profiles
/// (which carry a name) ahead of the default.
fn dedupe_accounts(accounts: &[ScanAccount]) -> Vec<ScanAccount> {
    let mut seen = HashSet::new();
    accounts
        .iter()
        .filter(|account| seen.insert(canonical(&account.dir)))
        .cloned()
        .collect()
}

/// Resolve a path for comparison, tolerating symlinks
///
/// On macOS `/tmp` is `/private/tmp`, and the agents record the resolved path
/// while Panoptes may hold the other one.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Tracks the work done against the budget
struct Meter {
    budget: ScanBudget,
    counters: ScanCounters,
}

impl Meter {
    fn new(budget: ScanBudget) -> Self {
        Self {
            budget,
            counters: ScanCounters::default(),
        }
    }

    fn exhausted(&self) -> bool {
        self.counters.files_examined >= self.budget.max_files
            || self.counters.bytes_read >= self.budget.max_bytes
            || self.counters.results >= self.budget.max_results
    }

    /// Count a file as opened, refusing when the file budget is spent
    fn open_file(&mut self) -> bool {
        if self.counters.files_examined >= self.budget.max_files {
            return false;
        }
        self.counters.files_examined += 1;
        true
    }

    /// Read up to `max` bytes of `path` from `offset`, charged to the budget
    ///
    /// Shortened to whatever byte budget is left. `None` when the file cannot
    /// be read, which callers treat as "skip it".
    fn read(&mut self, path: &Path, offset: u64, max: u64) -> Option<Vec<u8>> {
        let allowed = max.min(
            self.budget
                .max_bytes
                .saturating_sub(self.counters.bytes_read),
        );
        if allowed == 0 {
            return None;
        }
        let read = (|| {
            let mut file = std::fs::File::open(path)?;
            file.seek(SeekFrom::Start(offset))?;
            let mut buf = Vec::new();
            file.take(allowed).read_to_end(&mut buf)?;
            Ok::<_, std::io::Error>(buf)
        })();
        match read {
            Ok(buf) => {
                self.counters.bytes_read += buf.len() as u64;
                Some(buf)
            }
            Err(e) => {
                tracing::debug!(path = %path.display(), error = %e, "Skipping unreadable transcript");
                None
            }
        }
    }
}

/// The complete lines of a prefix read
///
/// A read that stopped short of the file's end may have cut its last line in
/// half, and half a JSON record is not a record, so it is dropped. A read that
/// reached the end keeps its last line even without a newline: the agent may
/// simply not have written one yet.
fn complete_lines(buf: &[u8], reached_end: bool) -> Vec<String> {
    let text = String::from_utf8_lossy(buf);
    let mut lines: Vec<&str> = text.split('\n').collect();
    if !reached_end && !text.ends_with('\n') {
        lines.pop();
    }
    lines
        .into_iter()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// One file waiting to be examined
#[derive(Debug, Clone)]
struct Candidate {
    kind: TranscriptKind,
    path: PathBuf,
    /// Its mtime: when it was last written, the order candidates are taken in
    modified: DateTime<Utc>,
    /// Index into the scan's deduped account list for its agent
    account: usize,
}

/// A source of candidates, newest first
enum Stream {
    /// Everything known up front, already sorted
    List(VecDeque<Candidate>),
    /// A Codex sessions tree, listed a day at a time
    Codex(CodexRollouts),
}

impl Stream {
    fn peek_key(&mut self) -> Option<DateTime<Utc>> {
        match self {
            Stream::List(list) => list.front().map(|c| c.modified),
            Stream::Codex(rollouts) => rollouts.peek().map(|c| c.modified),
        }
    }

    fn pop(&mut self) -> Option<Candidate> {
        match self {
            Stream::List(list) => list.pop_front(),
            Stream::Codex(rollouts) => {
                rollouts.peek();
                rollouts.pending.pop_front()
            }
        }
    }
}

/// When a file was last written, as a UTC time
fn modified(path: &Path) -> Option<DateTime<Utc>> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(DateTime::<Utc>::from(modified))
}

// ============================================================================
// Claude
// ============================================================================

/// Every Claude transcript filed under `working_dir`, newest first
///
/// Listing and stat'ing are not charged to the budget: the slug narrows this
/// to one project's directory, and nothing is read yet.
fn claude_candidates(
    accounts: &[ScanAccount],
    working_dir: &Path,
    claimed: &HashSet<String>,
) -> VecDeque<Candidate> {
    // Claude names the directory after the path it resolved, so a working
    // directory reached through a symlink files under the resolved one
    let mut spellings = vec![working_dir.to_path_buf()];
    let resolved = canonical(working_dir);
    if resolved != working_dir {
        spellings.push(resolved);
    }

    let mut candidates = Vec::new();
    for (index, account) in accounts.iter().enumerate() {
        let mut dirs: Vec<PathBuf> = spellings
            .iter()
            .flat_map(|spelling| claude_project_dirs(&account.dir, spelling))
            .collect();
        dirs.dedup();
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                // The file is named for the conversation, so a claimed one is
                // skipped without being opened
                let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                if claimed.contains(id) {
                    continue;
                }
                let Some(modified) = modified(&path) else {
                    continue;
                };
                candidates.push(Candidate {
                    kind: TranscriptKind::Claude,
                    path,
                    modified,
                    account: index,
                });
            }
        }
    }
    candidates.sort_by_key(|c| std::cmp::Reverse(c.modified));
    candidates.into()
}

/// The project directories a working directory's transcripts can be under
///
/// Normally exactly one, named by [`super::claude::project_slug`]. Claude
/// shortens a name over [`CLAUDE_SLUG_MAX`] characters and appends a hash we
/// cannot reproduce, so for a long path every directory sharing the prefix is
/// a candidate - the `cwd` inside each transcript then decides.
fn claude_project_dirs(config_dir: &Path, working_dir: &Path) -> Vec<PathBuf> {
    let projects = config_dir.join("projects");
    let slug = super::claude::project_slug(working_dir);

    let exact = projects.join(&slug);
    if exact.is_dir() || slug.len() <= CLAUDE_SLUG_MAX {
        return vec![exact];
    }
    let prefix = &slug[..CLAUDE_SLUG_MAX];
    let Ok(entries) = std::fs::read_dir(&projects) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(prefix))
        .map(|entry| entry.path())
        .collect()
}

/// Read a Claude transcript's header region and describe the conversation
///
/// `None` when it is not a conversation started in `target`: another `cwd`,
/// or a file with no exchange in it at all (Claude writes one for a session
/// that was opened and closed without a prompt, and resuming it fails).
fn examine_claude(
    meter: &mut Meter,
    candidate: &Candidate,
    target: &Path,
    account: &ScanAccount,
) -> Option<FoundConversation> {
    if !meter.open_file() {
        return None;
    }
    let len = std::fs::metadata(&candidate.path).ok()?.len();
    let buf = meter.read(&candidate.path, 0, CLAUDE_HEAD_BYTES)?;
    let reached_end = buf.len() as u64 >= len;

    let mut header = ClaudeHeader::default();
    for line in complete_lines(&buf, reached_end) {
        header.read(&line);
    }

    // A transcript names its directory in every record. The slug that filed
    // it is lossy (`/a-b` and `/a/b` share one), so this is the real check.
    if let Some(cwd) = &header.cwd {
        if canonical(cwd) != target {
            tracing::debug!(path = %candidate.path.display(), "Claude transcript is for another directory");
            return None;
        }
    }
    if !header.has_exchange {
        tracing::debug!(path = %candidate.path.display(), "Claude transcript holds no conversation");
        return None;
    }

    let id = candidate.path.file_stem()?.to_str()?.to_string();
    let title = header.title.or(header.summary).or(header.first_prompt);
    Some(found_conversation(candidate, id, title, account))
}

/// What the start of a Claude transcript says about it
#[derive(Debug, Default)]
struct ClaudeHeader {
    cwd: Option<PathBuf>,
    /// The latest `ai-title` seen
    title: Option<String>,
    /// An older transcript's `summary` record, the title's predecessor
    summary: Option<String>,
    first_prompt: Option<String>,
    /// Whether any user or assistant message is in the conversation proper
    has_exchange: bool,
}

impl ClaudeHeader {
    fn read(&mut self, line: &str) {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            return;
        };
        // A subagent's records describe its own task, not the conversation
        if flag(&record, "isSidechain") {
            return;
        }
        if self.cwd.is_none() {
            if let Some(cwd) = record.get("cwd").and_then(Value::as_str) {
                self.cwd = Some(PathBuf::from(cwd));
            }
        }
        match record.get("type").and_then(Value::as_str) {
            Some("ai-title") => {
                if let Some(title) = record.get("aiTitle").and_then(Value::as_str) {
                    self.title = non_empty_title(title).or(self.title.take());
                }
            }
            Some("summary") => {
                if let Some(summary) = record.get("summary").and_then(Value::as_str) {
                    self.summary = self.summary.take().or(non_empty_title(summary));
                }
            }
            Some("assistant") => self.has_exchange = true,
            Some("user") if !flag(&record, "isMeta") => {
                self.has_exchange = true;
                if self.first_prompt.is_none() {
                    self.first_prompt = record
                        .get("message")
                        .and_then(|m| m.get("content"))
                        .and_then(prompt_text)
                        .and_then(non_empty_title);
                }
            }
            _ => {}
        }
    }
}

/// Whether a top-level boolean flag is set on a record
fn flag(record: &Value, name: &str) -> bool {
    record.get(name).and_then(Value::as_bool) == Some(true)
}

/// The text a user typed, from a Claude `message.content`
///
/// `None` for what Claude injects rather than what was typed: tool results,
/// and the tag-wrapped records of slash commands and their output
/// (`<command-name>`, `<local-command-stdout>`, ...). None of those reads as
/// the subject of the conversation.
fn prompt_text(content: &Value) -> Option<&str> {
    let text = match content {
        Value::String(text) => text.as_str(),
        Value::Array(blocks) => blocks.iter().find_map(|block| {
            (block.get("type").and_then(Value::as_str) == Some("text"))
                .then(|| block.get("text").and_then(Value::as_str))
                .flatten()
        })?,
        _ => return None,
    };
    (!text.trim_start().starts_with('<')).then_some(text)
}

// ============================================================================
// Codex
// ============================================================================

/// A Codex sessions tree, newest day first, listed one day at a time
///
/// Listing a day is cheap (one directory read, a stat per file), but a heavy
/// user has years of them; loading them lazily means a scan that finds what it
/// needs in the last week never lists the rest.
struct CodexRollouts {
    account: usize,
    /// Day directories not yet listed, newest last so `pop` takes it
    days: Vec<PathBuf>,
    /// The listed day's rollouts, newest first
    pending: VecDeque<Candidate>,
}

impl CodexRollouts {
    fn new(account: usize, codex_home: &Path) -> Self {
        let sessions = codex_home.join("sessions");
        let mut days = Vec::new();
        for year in sorted_subdirs(&sessions) {
            for month in sorted_subdirs(&year) {
                days.extend(sorted_subdirs(&month));
            }
        }
        // Ascending by name, which for `YYYY/MM/DD` is by date
        Self {
            account,
            days,
            pending: VecDeque::new(),
        }
    }

    /// The next rollout, listing older days until one has any
    fn peek(&mut self) -> Option<&Candidate> {
        while self.pending.is_empty() {
            let day = self.days.pop()?;
            let Ok(entries) = std::fs::read_dir(&day) else {
                continue;
            };
            let mut listed: Vec<Candidate> = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("jsonl"))
                .filter_map(|path| {
                    Some(Candidate {
                        kind: TranscriptKind::Codex,
                        modified: modified(&path)?,
                        path,
                        account: self.account,
                    })
                })
                .collect();
            listed.sort_by_key(|c| std::cmp::Reverse(c.modified));
            self.pending = listed.into();
        }
        self.pending.front()
    }
}

/// Immediate subdirectories, sorted by name
fn sorted_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// Decide whether a rollout is a conversation of ours
///
/// Reads only a probe of the header first: most rollouts belong to other
/// directories, and the `cwd` near the top of the header line says so without
/// reading the ~20 KB of instructions that follow it. Only a match is read on
/// to the end of that line, for the fields that classify it.
///
/// Returns the conversation ID and the bytes read so far, so looking further
/// for the first prompt carries on from there rather than paying for them
/// twice.
fn examine_codex(
    meter: &mut Meter,
    candidate: &Candidate,
    target: &Path,
    claimed: &HashSet<String>,
) -> Option<(String, Vec<u8>)> {
    // The file name ends in the conversation ID, so a claimed one is skipped
    // without being opened
    let stem = candidate.path.file_stem()?.to_str()?;
    if claimed.iter().any(|id| stem.ends_with(id.as_str())) {
        return None;
    }
    if !meter.open_file() {
        return None;
    }

    let mut head = meter.read(&candidate.path, 0, CODEX_PROBE_BYTES)?;
    if !head.contains(&b'\n') {
        // The header runs past the probe. Rule the file out on its `cwd`
        // alone if it can be; otherwise read on for the whole line.
        if let Some(cwd) = probe_cwd(&head) {
            if canonical(Path::new(&cwd)) != target {
                return None;
            }
        }
        let rest = meter.read(
            &candidate.path,
            head.len() as u64,
            CODEX_HEAD_BYTES.saturating_sub(head.len() as u64),
        )?;
        head.extend_from_slice(&rest);
    }
    let Some(end) = head.iter().position(|&b| b == b'\n') else {
        tracing::debug!(path = %candidate.path.display(), "Codex header line is incomplete or too long; skipping");
        return None;
    };
    let header_line = String::from_utf8_lossy(&head[..end]).into_owned();

    let Some(meta) = codex::meta_from_line(&header_line) else {
        tracing::debug!(path = %candidate.path.display(), "Not a Codex rollout header; skipping");
        return None;
    };
    // Subagents and Codex's own background threads get rollouts too; resuming
    // one would reattach to that work, not to a conversation the user had
    if meta.kind != RolloutKind::Session {
        return None;
    }
    let cwd = meta.cwd?;
    if canonical(&cwd) != target || claimed.contains(&meta.id) {
        return None;
    }
    Some((meta.id, head))
}

/// The `cwd` value in a header line cut short by the probe
///
/// A textual look, since the truncated line is not valid JSON: the string
/// after the first `"cwd":` is decoded on its own. `None` when the probe does
/// not contain it whole, which only means "cannot rule this file out yet".
fn probe_cwd(probe: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(probe);
    let at = text.find("\"cwd\":")? + "\"cwd\":".len();
    let mut values =
        serde_json::Deserializer::from_str(text[at..].trim_start()).into_iter::<String>();
    values.next()?.ok()
}

/// The first prompt of a Codex rollout, for one Codex has not named
///
/// Codex records what the user typed as a `user_message` event; the
/// `response_item` user messages before it are injected context (AGENTS.md,
/// the environment), not the subject.
fn codex_first_prompt(meter: &mut Meter, path: &Path, head: Vec<u8>) -> Option<String> {
    let len = std::fs::metadata(path).ok()?.len();
    let mut buf = head;
    let wanted = CODEX_HEAD_BYTES.saturating_sub(buf.len() as u64);
    if wanted > 0 && (buf.len() as u64) < len {
        if let Some(rest) = meter.read(path, buf.len() as u64, wanted) {
            buf.extend_from_slice(&rest);
        }
    }
    let reached_end = buf.len() as u64 >= len;
    complete_lines(&buf, reached_end).iter().find_map(|line| {
        let record: Value = serde_json::from_str(line).ok()?;
        if record.get("type").and_then(Value::as_str) != Some("event_msg") {
            return None;
        }
        let payload = record.get("payload")?;
        if payload.get("type").and_then(Value::as_str) != Some("user_message") {
            return None;
        }
        non_empty_title(payload.get("message")?.as_str()?)
    })
}

/// The thread names Codex keeps for one `CODEX_HOME`
///
/// Only the tail of the index is read (see [`SESSION_INDEX_TAIL_BYTES`]); a
/// conversation named before that falls back to its first prompt.
fn read_thread_names(meter: &mut Meter, codex_home: &Path) -> HashMap<String, String> {
    let path = codex_home.join("session_index.jsonl");
    let Ok(metadata) = std::fs::metadata(&path) else {
        return HashMap::new();
    };
    let start = metadata.len().saturating_sub(SESSION_INDEX_TAIL_BYTES);
    let Some(buf) = meter.read(&path, start, SESSION_INDEX_TAIL_BYTES) else {
        return HashMap::new();
    };
    let mut lines = complete_lines(&buf, true);
    // A window starting mid-file starts mid-line
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    super::session_index::latest_names(lines.iter().map(String::as_str))
}

// ============================================================================
// Shared
// ============================================================================

fn found_conversation(
    candidate: &Candidate,
    id: String,
    title: Option<String>,
    account: &ScanAccount,
) -> FoundConversation {
    FoundConversation {
        kind: candidate.kind,
        id,
        title,
        last_active: candidate.modified,
        config_id: account.config_id,
        account_name: account.name.clone(),
        path: candidate.path.clone(),
    }
}

/// A title as it should be shown: one line, bounded, or `None` if blank
fn non_empty_title(text: &str) -> Option<String> {
    let title = clean_title(text);
    (!title.is_empty()).then_some(title)
}

/// Collapse whitespace to single spaces and cap the length
fn clean_title(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match collapsed.char_indices().nth(MAX_TITLE_CHARS) {
        Some((at, _)) => format!("{}…", &collapsed[..at]),
        None => collapsed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};
    use tempfile::TempDir;

    /// A home with a project checkout, a Claude config dir and a `CODEX_HOME`
    struct Fixture {
        _dir: TempDir,
        root: PathBuf,
        working_dir: PathBuf,
        claude: PathBuf,
        codex: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            // Resolved, as the agents record it: on macOS the temp dir is
            // reached through the `/var` -> `/private/var` symlink
            let root = std::fs::canonicalize(dir.path()).unwrap();
            let working_dir = root.join("work").join("panoptes");
            std::fs::create_dir_all(&working_dir).unwrap();
            Self {
                _dir: dir,
                claude: root.join("claude"),
                codex: root.join("codex"),
                working_dir,
                root,
            }
        }

        fn other_dir(&self) -> PathBuf {
            let other = self.root.join("work").join("elsewhere");
            std::fs::create_dir_all(&other).unwrap();
            other
        }

        fn request(&self) -> ScanRequest {
            ScanRequest {
                working_dir: self.working_dir.clone(),
                claude_accounts: vec![ScanAccount {
                    dir: self.claude.clone(),
                    config_id: None,
                    name: None,
                }],
                codex_accounts: vec![ScanAccount {
                    dir: self.codex.clone(),
                    config_id: None,
                    name: None,
                }],
                claimed: HashSet::new(),
                budget: ScanBudget::default(),
            }
        }

        /// A Claude transcript filed under `filed_under`'s slug, recording `cwd`
        fn claude(
            &self,
            filed_under: &Path,
            cwd: &Path,
            id: &str,
            age_secs: u64,
            title: Option<&str>,
        ) {
            let dir = self
                .claude
                .join("projects")
                .join(super::super::claude::project_slug(filed_under));
            let mut lines = vec![
                format!(
                    r#"{{"type":"permission-mode","permissionMode":"default","sessionId":"{id}"}}"#
                ),
                claude_user(cwd, id, "<command-name>/model</command-name>"),
                claude_user(cwd, id, &format!("Prompt of {id}")),
                format!(
                    r#"{{"type":"assistant","cwd":"{}","sessionId":"{id}","message":{{"model":"claude-opus-4-8","content":[{{"type":"text","text":"ok"}}]}}}}"#,
                    cwd.display()
                ),
            ];
            if let Some(title) = title {
                lines.push(format!(
                    r#"{{"type":"ai-title","aiTitle":"{title}","sessionId":"{id}"}}"#
                ));
            }
            write(
                &dir.join(format!("{id}.jsonl")),
                &lines.join("\n"),
                age_secs,
            );
        }

        /// A Codex rollout started in `cwd`, with an optional first prompt
        fn codex(&self, cwd: &Path, id: &str, age_secs: u64, source: &str, prompt: Option<&str>) {
            let day = self
                .codex
                .join("sessions")
                .join("2026")
                .join("09")
                .join("23");
            let mut lines = vec![codex_meta(cwd, id, source, 0)];
            lines.push(r#"{"type":"event_msg","payload":{"type":"task_started"}}"#.to_string());
            if let Some(prompt) = prompt {
                lines.push(format!(
                    r#"{{"type":"event_msg","payload":{{"type":"user_message","message":"{prompt}"}}}}"#
                ));
            }
            write(
                &day.join(format!("rollout-2026-09-23T10-00-00-{id}.jsonl")),
                &lines.join("\n"),
                age_secs,
            );
        }

        fn codex_index(&self, lines: &[(&str, &str)]) {
            let text: Vec<String> = lines
                .iter()
                .map(|(id, name)| {
                    format!(
                        r#"{{"id":"{id}","thread_name":"{name}","updated_at":"2026-09-23T10:20:46Z"}}"#
                    )
                })
                .collect();
            std::fs::create_dir_all(&self.codex).unwrap();
            std::fs::write(
                self.codex.join("session_index.jsonl"),
                text.join("\n") + "\n",
            )
            .unwrap();
        }
    }

    fn claude_user(cwd: &Path, id: &str, text: &str) -> String {
        format!(
            r#"{{"type":"user","cwd":"{}","sessionId":"{id}","message":{{"role":"user","content":"{text}"}}}}"#,
            cwd.display()
        )
    }

    /// A `session_meta` header, padded like the real ~20 KB instructions
    fn codex_meta(cwd: &Path, id: &str, source: &str, padding: usize) -> String {
        format!(
            r#"{{"timestamp":"2026-09-23T10:00:00.000Z","type":"session_meta","payload":{{"id":"{id}","timestamp":"2026-09-23T10:00:00.000Z","cwd":"{}","originator":"codex-tui","source":{source},"base_instructions":{{"text":"{}"}}}}}}"#,
            cwd.display(),
            "x".repeat(padding)
        )
    }

    /// Write a file whose mtime is `age_secs` in the past
    fn write(path: &Path, text: &str, age_secs: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("{text}\n")).unwrap();
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(age_secs))
            .unwrap();
    }

    fn run(request: &ScanRequest) -> ScanOutcome {
        scan(request, &AtomicBool::new(false))
    }

    fn ids(outcome: &ScanOutcome) -> Vec<&str> {
        outcome
            .conversations
            .iter()
            .map(|c| c.id.as_str())
            .collect()
    }

    #[test]
    fn test_finds_conversations_for_the_cwd() {
        let f = Fixture::new();
        f.claude(
            &f.working_dir,
            &f.working_dir,
            "claude-titled",
            10,
            Some("Header fix"),
        );
        f.claude(&f.working_dir, &f.working_dir, "claude-untitled", 20, None);
        f.codex(
            &f.working_dir,
            "codex-named",
            30,
            r#""cli""#,
            Some("Why is it slow"),
        );
        f.codex(
            &f.working_dir,
            "codex-unnamed",
            40,
            r#""cli""#,
            Some("Why is it slow"),
        );
        f.codex_index(&[("codex-named", "Profile the scanner")]);

        let outcome = run(&f.request());

        let found: Vec<(&str, TranscriptKind, Option<&str>)> = outcome
            .conversations
            .iter()
            .map(|c| (c.id.as_str(), c.kind, c.title.as_deref()))
            .collect();
        assert_eq!(
            found,
            vec![
                ("claude-titled", TranscriptKind::Claude, Some("Header fix")),
                // The slash command before it is not what the conversation is about
                (
                    "claude-untitled",
                    TranscriptKind::Claude,
                    Some("Prompt of claude-untitled")
                ),
                (
                    "codex-named",
                    TranscriptKind::Codex,
                    Some("Profile the scanner")
                ),
                (
                    "codex-unnamed",
                    TranscriptKind::Codex,
                    Some("Why is it slow")
                ),
            ]
        );
        assert!(!outcome.truncated);
        assert!(!outcome.cancelled);
    }

    #[test]
    fn test_ignores_other_cwds() {
        let f = Fixture::new();
        let other = f.other_dir();
        // Filed under our slug but recording another directory: the slug is
        // lossy, so the record's own `cwd` decides
        f.claude(&f.working_dir, &other, "claude-slug-collision", 10, None);
        f.claude(&other, &other, "claude-elsewhere", 10, None);
        f.codex(&other, "codex-elsewhere", 10, r#""cli""#, None);
        f.claude(&f.working_dir, &f.working_dir, "claude-ours", 20, None);

        let outcome = run(&f.request());
        assert_eq!(ids(&outcome), vec!["claude-ours"]);
    }

    #[test]
    fn test_a_long_codex_header_is_ruled_out_from_its_probe() {
        let f = Fixture::new();
        let other = f.other_dir();
        let day = f.codex.join("sessions/2026/09/23");
        // Headers longer than the probe, as real ones are (~22 KB)
        write(
            &day.join("rollout-2026-09-23T10-00-00-long-elsewhere.jsonl"),
            &codex_meta(&other, "long-elsewhere", r#""cli""#, 40_000),
            10,
        );
        write(
            &day.join("rollout-2026-09-23T10-00-01-long-ours.jsonl"),
            &codex_meta(&f.working_dir, "long-ours", r#""cli""#, 40_000),
            20,
        );

        let outcome = run(&f.request());
        assert_eq!(ids(&outcome), vec!["long-ours"]);
        // The other directory's rollout cost one probe, not its whole header
        assert!(
            outcome.counters.bytes_read < 40_000 + 2 * CODEX_PROBE_BYTES + 1_000,
            "read {} bytes",
            outcome.counters.bytes_read
        );
    }

    #[test]
    fn test_newest_first() {
        let f = Fixture::new();
        f.codex(&f.working_dir, "codex-2", 200, r#""cli""#, None);
        f.claude(&f.working_dir, &f.working_dir, "claude-3", 300, None);
        f.claude(&f.working_dir, &f.working_dir, "claude-1", 100, None);
        f.codex(&f.working_dir, "codex-0", 5, r#""cli""#, None);

        let outcome = run(&f.request());
        assert_eq!(
            ids(&outcome),
            vec!["codex-0", "claude-1", "codex-2", "claude-3"]
        );
    }

    #[test]
    fn test_honours_the_file_budget() {
        let f = Fixture::new();
        let other = f.other_dir();
        for i in 0..10 {
            f.codex(&other, &format!("elsewhere-{i}"), 10 + i, r#""cli""#, None);
        }
        let mut request = f.request();
        request.budget.max_files = 3;

        let outcome = run(&request);
        assert_eq!(outcome.counters.files_examined, 3);
        assert!(outcome.truncated, "seven rollouts were never looked at");
    }

    #[test]
    fn test_honours_the_byte_budget() {
        let f = Fixture::new();
        for i in 0..10 {
            f.claude(
                &f.working_dir,
                &f.working_dir,
                &format!("c-{i}"),
                10 + i,
                None,
            );
        }
        let one_file = std::fs::metadata(
            f.claude
                .join("projects")
                .join(super::super::claude::project_slug(&f.working_dir))
                .join("c-0.jsonl"),
        )
        .unwrap()
        .len();
        let mut request = f.request();
        request.budget.max_bytes = one_file * 3;

        let outcome = run(&request);
        assert_eq!(outcome.counters.bytes_read, one_file * 3);
        assert_eq!(outcome.counters.files_examined, 3);
        assert!(outcome.truncated);
        assert_eq!(
            ids(&outcome),
            vec!["c-0", "c-1", "c-2"],
            "the newest survive"
        );
    }

    #[test]
    fn test_honours_the_result_budget() {
        let f = Fixture::new();
        for i in 0..5 {
            f.claude(
                &f.working_dir,
                &f.working_dir,
                &format!("c-{i}"),
                10 + i,
                None,
            );
        }
        let mut request = f.request();
        request.budget.max_results = 2;

        let outcome = run(&request);
        assert_eq!(outcome.counters.results, 2);
        assert_eq!(outcome.counters.files_examined, 2);
        assert_eq!(ids(&outcome), vec!["c-0", "c-1"]);
        assert!(outcome.truncated);
    }

    #[test]
    fn test_skips_claimed_ids_without_opening_them() {
        let f = Fixture::new();
        f.claude(&f.working_dir, &f.working_dir, "claude-owned", 10, None);
        f.codex(&f.working_dir, "codex-owned", 20, r#""cli""#, None);
        f.claude(&f.working_dir, &f.working_dir, "claude-free", 30, None);
        let mut request = f.request();
        request.claimed = ["claude-owned", "codex-owned"]
            .into_iter()
            .map(str::to_string)
            .collect();

        let outcome = run(&request);
        assert_eq!(ids(&outcome), vec!["claude-free"]);
        assert_eq!(outcome.counters.files_examined, 1);
    }

    #[test]
    fn test_skips_subagent_and_system_rollouts() {
        let f = Fixture::new();
        f.codex(
            &f.working_dir,
            "subagent",
            10,
            r#"{"subagent":{"thread_spawn":{"parent_thread_id":"parent","depth":1}}}"#,
            Some("Review the diff"),
        );
        f.codex(
            &f.working_dir,
            "memory",
            20,
            r#"{"internal":"memory_consolidation"}"#,
            None,
        );
        f.codex(&f.working_dir, "session", 30, r#""cli""#, None);

        let outcome = run(&f.request());
        assert_eq!(ids(&outcome), vec!["session"]);
    }

    #[test]
    fn test_tolerates_garbage_partial_lines_and_empty_files() {
        let f = Fixture::new();
        let slug_dir = f
            .claude
            .join("projects")
            .join(super::super::claude::project_slug(&f.working_dir));
        // Opened and closed without a prompt: nothing to resume
        write(
            &slug_dir.join("no-exchange.jsonl"),
            r#"{"type":"permission-mode","permissionMode":"default"}"#,
            10,
        );
        write(&slug_dir.join("garbage.jsonl"), "not json\n{\"type\":", 20);
        write(&slug_dir.join("empty.jsonl"), "", 30);
        // A real one whose last line is still being written
        write(
            &slug_dir.join("mid-write.jsonl"),
            &format!(
                "{}\n{{\"type\":\"assistant\",\"mess",
                claude_user(&f.working_dir, "mid-write", "Still typing")
            ),
            40,
        );
        let day = f.codex.join("sessions/2026/09/23");
        write(
            &day.join("rollout-2026-09-23T10-00-00-broken.jsonl"),
            "{\"type\":\"session_me",
            50,
        );
        // Unreadable: a directory where a file should be
        std::fs::create_dir_all(day.join("rollout-2026-09-23T10-00-00-dir.jsonl")).unwrap();

        let outcome = run(&f.request());
        assert_eq!(ids(&outcome), vec!["mid-write"]);
        assert_eq!(
            outcome.conversations[0].title.as_deref(),
            Some("Still typing")
        );
    }

    #[test]
    fn test_a_cancelled_scan_stops_before_reading() {
        let f = Fixture::new();
        f.claude(&f.working_dir, &f.working_dir, "c", 10, None);

        let outcome = scan(&f.request(), &AtomicBool::new(true));
        assert!(outcome.cancelled);
        assert_eq!(outcome.counters, ScanCounters::default());
    }

    #[test]
    fn test_labels_each_conversation_with_its_account() {
        let f = Fixture::new();
        let work_id = Uuid::new_v4();
        f.claude(&f.working_dir, &f.working_dir, "c", 10, None);
        let mut request = f.request();
        // The same directory as a named profile and as the default: the
        // profile, listed first, is the one that sticks
        request.claude_accounts.insert(
            0,
            ScanAccount {
                dir: f.claude.clone(),
                config_id: Some(work_id),
                name: Some("work".to_string()),
            },
        );

        let outcome = run(&request);
        assert_eq!(
            outcome.conversations.len(),
            1,
            "listed once, not per spelling"
        );
        assert_eq!(outcome.conversations[0].config_id, Some(work_id));
        assert_eq!(
            outcome.conversations[0].account_name.as_deref(),
            Some("work")
        );
    }

    #[test]
    fn test_finds_a_working_dir_whose_slug_claude_shortened() {
        let f = Fixture::new();
        let mut long = f.working_dir.clone();
        while super::super::claude::project_slug(&long).len() <= CLAUDE_SLUG_MAX {
            long = long.join("a-very-long-directory-name");
        }
        std::fs::create_dir_all(&long).unwrap();
        let slug = super::super::claude::project_slug(&long);
        let dir = f
            .claude
            .join("projects")
            .join(format!("{}-1x2y3z", &slug[..CLAUDE_SLUG_MAX]));
        write(
            &dir.join("long.jsonl"),
            &claude_user(&long, "long", "Deep in the tree"),
            10,
        );

        let mut request = f.request();
        request.working_dir = long;
        let outcome = run(&request);
        assert_eq!(ids(&outcome), vec!["long"]);
    }
}
