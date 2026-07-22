//! Background transcript watching
//!
//! Runs on its own OS thread rather than the UI thread. The reads are small and
//! incremental, but a burst of tool output can append a lot at once, and
//! parsing that on the render thread would show up as a stutter.
//!
//! Communication is deliberately plain `std::sync::mpsc` in both directions:
//! the app already drains hook events with `try_recv` each tick, so transcript
//! events arrive through the same shape of code with no async machinery.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use crate::agent::events::AgentEvent;
use crate::session::SessionId;

use super::{Tailer, TranscriptKind};

/// How often to check followed files for new content
///
/// Agents flush in well under 50ms, so this is what actually bounds how stale
/// the display can be. Fast enough to feel live, slow enough that idle sessions
/// cost nothing but a `stat` a few times a second.
const POLL_INTERVAL: Duration = Duration::from_millis(150);

/// How often to look for newly spawned subagents
///
/// Much rarer than tailing, and it means listing directories, so it runs on its
/// own slower clock.
const SUBAGENT_SCAN_INTERVAL: Duration = Duration::from_secs(5);

/// How recently a subagent rollout must have been written to count as running
///
/// Best effort by construction - see [`WatcherState::count_subagents`].
const SUBAGENT_LIVE_WINDOW: Duration = Duration::from_secs(90);

/// A transcript to follow on behalf of a session
#[derive(Debug, Clone)]
pub struct WatchTarget {
    /// Session the events belong to
    pub session_id: SessionId,
    /// Which agent wrote the file
    pub kind: TranscriptKind,
    /// The transcript itself
    pub path: PathBuf,
    /// Where Codex keeps its sessions, for finding subagent rollouts
    pub codex_sessions_dir: Option<PathBuf>,
    /// The conversation ID subagent rollouts would name as their parent
    pub conversation_id: Option<String>,
    /// Whether to read the file from the beginning rather than from its end
    ///
    /// True for a transcript this session wrote itself, where every record
    /// describes what this session has just been doing. False when reattaching
    /// to a conversation that predates the session, whose history must not
    /// replay as if it were happening now.
    pub from_start: bool,
}

/// Instructions to the watcher thread
enum Command {
    Watch(Box<WatchTarget>),
    Forget(SessionId),
    /// Write every raw transcript line to `~/.panoptes/logs/agent-events/`
    SetDebugLog(Option<PathBuf>),
}

/// Handle to the watcher thread
pub struct TranscriptWatcher {
    commands: Sender<Command>,
    events: Receiver<(SessionId, AgentEvent)>,
}

impl TranscriptWatcher {
    /// Start the watcher thread
    pub fn spawn() -> Self {
        let (command_tx, command_rx) = std::sync::mpsc::channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();

        std::thread::Builder::new()
            .name("panoptes-transcripts".to_string())
            .spawn(move || run(command_rx, event_tx))
            .expect("failed to spawn transcript watcher thread");

        Self {
            commands: command_tx,
            events: event_rx,
        }
    }

    /// Follow a session's transcript
    ///
    /// Watching the same session again replaces the previous target, which is
    /// what a resumed or woken session needs: same session, new file offset.
    pub fn watch(&self, target: WatchTarget) {
        // A dead watcher thread costs live state updates, not correctness, so
        // it is reported rather than propagated
        if self
            .commands
            .send(Command::Watch(Box::new(target)))
            .is_err()
        {
            tracing::warn!("Transcript watcher is not running; session state may lag");
        }
    }

    /// Stop following a session
    pub fn forget(&self, session_id: SessionId) {
        let _ = self.commands.send(Command::Forget(session_id));
    }

    /// Turn raw-event logging on or off
    pub fn set_debug_log_dir(&self, dir: Option<PathBuf>) {
        let _ = self.commands.send(Command::SetDebugLog(dir));
    }

    /// Take everything observed since the last call
    pub fn drain(&self) -> Vec<(SessionId, AgentEvent)> {
        let mut out = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            out.push(event);
        }
        out
    }
}

/// One followed transcript
struct Watched {
    target: WatchTarget,
    tailer: Tailer,
    subagents: usize,
}

/// Everything the watcher thread owns
#[derive(Default)]
struct WatcherState {
    watched: HashMap<SessionId, Watched>,
    debug_log_dir: Option<PathBuf>,
}

impl WatcherState {
    fn watch(&mut self, target: WatchTarget, events: &Sender<(SessionId, AgentEvent)>) {
        let tailer = if target.from_start {
            // Everything already in the file belongs to this session, including
            // whatever it did while its conversation was still being discovered
            Tailer::from_start(target.kind, target.path.clone())
        } else {
            let (tailer, seed) = Tailer::attach(target.kind, target.path.clone());
            // Seed the usage display from history so it is not blank until the
            // next turn happens to mention token counts
            if let Some(usage) = seed {
                let _ = events.send((target.session_id, AgentEvent::Usage(usage)));
            }
            tailer
        };

        self.watched.insert(
            target.session_id,
            Watched {
                target,
                tailer,
                subagents: 0,
            },
        );
    }

    fn poll(&mut self, events: &Sender<(SessionId, AgentEvent)>) {
        for watched in self.watched.values_mut() {
            let (parsed, raw) = watched.tailer.poll();

            if !raw.is_empty() {
                if let Some(dir) = &self.debug_log_dir {
                    log_raw(dir, watched.target.session_id, &raw);
                }
            }

            for event in parsed {
                if events.send((watched.target.session_id, event)).is_err() {
                    return;
                }
            }
        }
    }

    /// Count each Codex session's live subagents
    ///
    /// Codex subagents write *separate* rollout files, so a parent looks idle
    /// while its children work - the mirror image of Claude, where subagents
    /// share one session ID.
    ///
    /// Discovery is exact: a child names its parent in `forked_from_id`.
    /// Liveness is not. There is no reliable "this subagent exited" signal, so
    /// this infers it from recent writes, which means a subagent that pauses
    /// for longer than the window stops being counted. That is why the display
    /// is a count and not a claim about what is running.
    fn scan_subagents(&mut self, events: &Sender<(SessionId, AgentEvent)>) {
        let targets: Vec<(SessionId, PathBuf, String)> = self
            .watched
            .values()
            .filter_map(|w| {
                Some((
                    w.target.session_id,
                    w.target.codex_sessions_dir.clone()?,
                    w.target.conversation_id.clone()?,
                ))
            })
            .collect();

        if targets.is_empty() {
            return;
        }

        // Only look at rollouts written recently. The full tree holds every
        // conversation ever - over a thousand files on a working machine - and
        // a subagent that has not been written to lately does not count anyway.
        let recent = recent_rollouts(&targets[0].1);

        for (session_id, _, conversation_id) in targets {
            let count = count_subagents(&recent, &conversation_id);
            let Some(watched) = self.watched.get_mut(&session_id) else {
                continue;
            };
            if watched.subagents != count {
                watched.subagents = count;
                let _ = events.send((session_id, AgentEvent::Subagents { active: count }));
            }
        }
    }
}

/// Rollout files written within the liveness window
fn recent_rollouts(sessions_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let now = std::time::SystemTime::now();

    // Filed under sessions/YYYY/MM/DD, so walk exactly three levels rather than
    // recursing over an unbounded tree
    for year in subdirs(sessions_dir) {
        for month in subdirs(&year) {
            for day in subdirs(&month) {
                let Ok(entries) = std::fs::read_dir(&day) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                        continue;
                    }
                    let fresh = entry
                        .metadata()
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|m| now.duration_since(m).ok())
                        .is_some_and(|age| age < SUBAGENT_LIVE_WINDOW);
                    if fresh {
                        out.push(path);
                    }
                }
            }
        }
    }
    out
}

/// How many of these rollouts are subagents of the given conversation
fn count_subagents(rollouts: &[PathBuf], conversation_id: &str) -> usize {
    rollouts
        .iter()
        .filter(|path| {
            read_meta_payload(path).is_some_and(|payload| {
                super::codex::is_subagent_meta(&payload)
                    && super::codex::parent_conversation_id(&payload).as_deref()
                        == Some(conversation_id)
            })
        })
        .count()
}

/// Read the `session_meta` payload from the first line of a rollout
fn read_meta_payload(path: &Path) -> Option<serde_json::Value> {
    use std::io::BufRead;

    let file = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(file).read_line(&mut line).ok()?;

    let record: serde_json::Value = serde_json::from_str(&line).ok()?;
    if record.get("type").and_then(|t| t.as_str()) != Some("session_meta") {
        return None;
    }
    record.get("payload").cloned()
}

/// List immediate subdirectories, ignoring anything unreadable
fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect()
}

/// Append raw transcript lines to a per-session debug log
///
/// Best effort throughout: this exists to make a misbehaving tailer
/// diagnosable, and must never be able to disturb the session it is describing.
fn log_raw(dir: &Path, session_id: SessionId, lines: &[String]) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::warn!(error = %e, "Could not create agent event log directory");
        return;
    }
    let path = dir.join(format!("{}.ndjson", session_id));
    let opened = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path);

    match opened {
        Ok(mut file) => {
            for line in lines {
                if let Err(e) = writeln!(file, "{}", line) {
                    tracing::warn!(error = %e, "Could not write agent event log");
                    return;
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "Could not open agent event log")
        }
    }
}

/// The watcher thread's main loop
fn run(commands: Receiver<Command>, events: Sender<(SessionId, AgentEvent)>) {
    let mut state = WatcherState::default();
    let mut last_subagent_scan = Instant::now() - SUBAGENT_SCAN_INTERVAL;

    loop {
        // Apply every pending instruction before reading files, so a session
        // that was just told to stop is not polled one last time
        loop {
            match commands.try_recv() {
                Ok(Command::Watch(target)) => state.watch(*target, &events),
                Ok(Command::Forget(session_id)) => {
                    state.watched.remove(&session_id);
                }
                Ok(Command::SetDebugLog(dir)) => state.debug_log_dir = dir,
                Err(TryRecvError::Empty) => break,
                // The app is gone; so is any reason to keep reading files
                Err(TryRecvError::Disconnected) => return,
            }
        }

        state.poll(&events);

        if last_subagent_scan.elapsed() >= SUBAGENT_SCAN_INTERVAL {
            state.scan_subagents(&events);
            last_subagent_scan = Instant::now();
        }

        std::thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_rollout(dir: &Path, name: &str, meta: serde_json::Value) -> PathBuf {
        let day = dir.join("2026").join("07").join("22");
        std::fs::create_dir_all(&day).unwrap();
        let path = day.join(name);
        let line = serde_json::json!({"type": "session_meta", "payload": meta});
        std::fs::write(&path, format!("{}\n", line)).unwrap();
        path
    }

    #[test]
    fn test_counts_only_this_conversations_subagents() {
        let dir = TempDir::new().unwrap();
        let sessions = dir.path();

        write_rollout(
            sessions,
            "rollout-parent.jsonl",
            serde_json::json!({"id": "parent", "cwd": "/tmp"}),
        );
        write_rollout(
            sessions,
            "rollout-child-1.jsonl",
            serde_json::json!({"id": "c1", "forked_from_id": "parent"}),
        );
        write_rollout(
            sessions,
            "rollout-child-2.jsonl",
            serde_json::json!({"id": "c2",
                "source": {"subagent": {"thread_spawn": {"parent_thread_id": "parent"}}}}),
        );
        write_rollout(
            sessions,
            "rollout-other.jsonl",
            serde_json::json!({"id": "c3", "forked_from_id": "someone-else"}),
        );

        let recent = recent_rollouts(sessions);
        assert_eq!(count_subagents(&recent, "parent"), 2);
        assert_eq!(count_subagents(&recent, "someone-else"), 1);
        assert_eq!(count_subagents(&recent, "nobody"), 0);
    }

    #[test]
    fn test_stale_rollouts_are_not_counted_as_running() {
        let dir = TempDir::new().unwrap();
        let path = write_rollout(
            dir.path(),
            "rollout-old.jsonl",
            serde_json::json!({"id": "c1", "forked_from_id": "parent"}),
        );

        // Backdate well past the liveness window
        let old = std::time::SystemTime::now() - Duration::from_secs(3600);
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();

        assert!(recent_rollouts(dir.path()).is_empty());
    }

    #[test]
    fn test_missing_sessions_directory_is_not_an_error() {
        let dir = TempDir::new().unwrap();
        assert!(recent_rollouts(&dir.path().join("nope")).is_empty());
    }

    #[test]
    fn test_debug_log_appends_raw_lines() {
        let dir = TempDir::new().unwrap();
        let session_id = uuid::Uuid::new_v4();
        let log_dir = dir.path().join("agent-events");

        log_raw(
            &log_dir,
            session_id,
            &["one".to_string(), "two".to_string()],
        );
        log_raw(&log_dir, session_id, &["three".to_string()]);

        let written =
            std::fs::read_to_string(log_dir.join(format!("{}.ndjson", session_id))).unwrap();
        assert_eq!(written, "one\ntwo\nthree\n");
    }

    #[test]
    fn test_debug_log_failure_is_swallowed() {
        // A log directory that cannot exist must not take anything down with it
        log_raw(
            Path::new("/proc/definitely/not/writable"),
            uuid::Uuid::new_v4(),
            &["x".to_string()],
        );
    }
}
