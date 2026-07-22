//! Reading agent transcripts
//!
//! Both agents already write a complete, live record of every conversation to
//! disk, and Panoptes knows where because it stores each session's conversation
//! ID. Reading those files is strictly better than asking the agents to report:
//! it needs no cooperation from them, and it is the *only* channel Codex has,
//! whose single `notify` hook cannot be extended without stalling its output.
//!
//! The two tailers have deliberately different jobs:
//!
//! - **Codex**: the rollout drives state. This is what brings Codex to parity
//!   with Claude, which until now could only ever report "my turn ended".
//! - **Claude**: the transcript only supplements. Hooks keep owning state -
//!   they are lower latency, and two producers writing the same field fight.
//!
//! Measured flush latency is under 50ms for Codex and effectively immediate for
//! Claude, so both are fast enough to drive a live display.

pub mod claude;
pub mod codex;
pub mod watcher;

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::agent::events::{AgentEvent, UsageSnapshot};

pub use watcher::{TranscriptWatcher, WatchTarget};

/// How much of the tail to read when seeding usage from an existing file
///
/// Enough to reach back past a long stretch of tool output to the last record
/// carrying token counts, without reading a multi-megabyte transcript in full.
const SEED_SCAN_BYTES: u64 = 512 * 1024;

/// Largest chunk read in a single poll
///
/// A burst of tool output can append a lot at once; this bounds how much is
/// parsed in one pass so a single poll cannot stall on a huge read. Whatever
/// is left over is picked up on the next poll.
const MAX_READ_BYTES: usize = 4 * 1024 * 1024;

/// Which agent wrote a transcript, and therefore how to read it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptKind {
    /// Codex rollout - drives session state
    Codex,
    /// Claude Code transcript - contributes usage only
    Claude,
}

impl TranscriptKind {
    fn parse_line(&self, line: &str) -> Option<AgentEvent> {
        match self {
            TranscriptKind::Codex => codex::parse_line(line),
            TranscriptKind::Claude => claude::parse_line(line),
        }
    }
}

/// Follows one transcript file, yielding events as they are appended
///
/// Holds a byte offset rather than re-reading, and keeps any trailing partial
/// line back until its newline arrives - transcripts are written by another
/// process and are routinely observed mid-write.
#[derive(Debug)]
pub struct Tailer {
    kind: TranscriptKind,
    path: PathBuf,
    /// Bytes already consumed, including anything held back below
    offset: u64,
    /// A final line seen without its terminating newline
    partial: String,
    /// Trailing bytes that stop mid-character and cannot be decoded yet
    pending_bytes: Vec<u8>,
}

impl Tailer {
    /// Start following a file from its current end
    ///
    /// Seeking to EOF is what stops a *reattached* session replaying its
    /// entire history as if it were happening now. The cost is that the usage
    /// display would start blank, so the tail is scanned backwards once for the
    /// most recent figures, which are returned to seed it.
    ///
    /// Only correct for a conversation that predates this session. A session
    /// that created its own transcript must use [`Tailer::from_start`], or the
    /// opening seconds - during which a Codex conversation is still being
    /// discovered - are lost outright rather than merely delayed.
    pub fn attach(kind: TranscriptKind, path: PathBuf) -> (Self, Option<UsageSnapshot>) {
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let seed = seed_usage(kind, &path, len);

        (
            Self {
                kind,
                path,
                offset: len,
                partial: String::new(),
                pending_bytes: Vec::new(),
            },
            seed,
        )
    }

    /// Follow a file from the beginning
    ///
    /// For a transcript this session wrote itself, where everything in the file
    /// describes what this session has just been doing.
    pub fn from_start(kind: TranscriptKind, path: PathBuf) -> Self {
        Self {
            kind,
            path,
            offset: 0,
            partial: String::new(),
            pending_bytes: Vec::new(),
        }
    }

    /// The file being followed
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read whatever has been appended since the last call
    ///
    /// Returns the parsed events, and separately the raw lines so a caller can
    /// log them. Never fails: an unreadable or vanished transcript simply
    /// produces nothing, because the alternative is taking down a session over
    /// a file Panoptes does not own.
    pub fn poll(&mut self) -> (Vec<AgentEvent>, Vec<String>) {
        let Ok(metadata) = std::fs::metadata(&self.path) else {
            return (Vec::new(), Vec::new());
        };
        let len = metadata.len();

        // Shorter than what we have read means the file was replaced or
        // truncated. Restart from the new end rather than reinterpreting
        // unrelated bytes at our old offset as a continuation.
        if len < self.offset {
            tracing::debug!(path = %self.path.display(), "Transcript shrank; re-attaching at its end");
            self.offset = len;
            self.partial.clear();
            self.pending_bytes.clear();
            return (Vec::new(), Vec::new());
        }
        if len == self.offset {
            return (Vec::new(), Vec::new());
        }

        let Ok(mut file) = std::fs::File::open(&self.path) else {
            return (Vec::new(), Vec::new());
        };
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return (Vec::new(), Vec::new());
        }

        let wanted = ((len - self.offset) as usize).min(MAX_READ_BYTES);
        let mut buf = vec![0u8; wanted];
        let Ok(read) = read_fully(&mut file, &mut buf) else {
            return (Vec::new(), Vec::new());
        };
        buf.truncate(read);
        self.offset += read as u64;

        // A character can straddle the read boundary, either because the file
        // was appended to mid-character or because `MAX_READ_BYTES` cut the
        // chunk there. Decoding each chunk in isolation would replace those
        // bytes with U+FFFD before the rest of the character ever arrived,
        // which is silent and permanent - the offset has already moved past
        // them. So undecodable trailing bytes are held, exactly like an
        // unterminated line.
        let mut bytes = std::mem::take(&mut self.pending_bytes);
        bytes.extend_from_slice(&buf);
        let (decoded, keep_from) = decode_prefix(&bytes);
        self.pending_bytes = bytes[keep_from..].to_vec();

        let mut text = std::mem::take(&mut self.partial);
        text.push_str(&decoded);

        let ends_complete = text.ends_with('\n');
        let mut lines: Vec<&str> = text.split('\n').collect();
        if !ends_complete {
            // Hold the unterminated remainder until its newline arrives
            self.partial = lines.pop().unwrap_or_default().to_string();
        }

        let mut events = Vec::new();
        let mut raw = Vec::new();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            raw.push(line.to_string());
            if let Some(event) = self.kind.parse_line(line) {
                events.push(event);
            }
        }

        (events, raw)
    }
}

/// Decode as much of a byte run as is valid UTF-8
///
/// Returns the decoded text and the offset from which bytes must be kept for
/// the next read. Genuinely invalid bytes are replaced and stepped over rather
/// than held, or a single corrupt byte would wedge the tailer forever; only a
/// truncated final character is held back.
fn decode_prefix(bytes: &[u8]) -> (String, usize) {
    let mut text = String::new();
    let mut start = 0;

    loop {
        match std::str::from_utf8(&bytes[start..]) {
            Ok(valid) => {
                text.push_str(valid);
                return (text, bytes.len());
            }
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                text.push_str(
                    std::str::from_utf8(&bytes[start..start + valid_up_to]).unwrap_or_default(),
                );
                match error.error_len() {
                    // Stops mid-character: the rest is still on its way
                    None => return (text, start + valid_up_to),
                    // Actually invalid: note it and keep going
                    Some(bad) => {
                        text.push('\u{FFFD}');
                        start += valid_up_to + bad;
                    }
                }
            }
        }
    }
}

/// Read until the buffer is full or the file ends
fn read_fully(file: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match file.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}

/// Scan backwards through the end of a file for the most recent usage figures
fn seed_usage(kind: TranscriptKind, path: &Path, len: u64) -> Option<UsageSnapshot> {
    if len == 0 {
        return None;
    }

    let mut file = std::fs::File::open(path).ok()?;
    let start = len.saturating_sub(SEED_SCAN_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;

    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);

    // Skip the first line when the window started mid-record
    let mut lines: Vec<&str> = text.split('\n').collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }

    let mut merged: Option<UsageSnapshot> = None;
    for line in lines.iter().rev() {
        if let Some(AgentEvent::Usage(usage)) = kind.parse_line(line) {
            match &mut merged {
                // Walking backwards, so anything already collected is newer and
                // must win; only fields it left unknown get filled in.
                Some(existing) => {
                    let mut older = usage;
                    older.merge(existing.clone());
                    *existing = older;
                }
                None => merged = Some(usage),
            }
            let complete = merged
                .as_ref()
                .is_some_and(|u| u.model.is_some() && u.total_tokens.is_some());
            if complete {
                break;
            }
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn append_bytes(path: &Path, bytes: &[u8]) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(bytes).unwrap();
    }

    fn append(path: &Path, text: &str) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(text.as_bytes()).unwrap();
    }

    #[test]
    fn test_tail_yields_only_new_lines() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rollout.jsonl");
        append(
            &path,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n",
        );

        let mut tailer = Tailer::from_start(TranscriptKind::Codex, path.clone());
        let (events, raw) = tailer.poll();
        assert_eq!(events, vec![AgentEvent::TurnStarted { title: None }]);
        assert_eq!(raw.len(), 1);

        // Nothing new
        assert!(tailer.poll().0.is_empty());

        append(
            &path,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\"}}\n",
        );
        assert_eq!(
            tailer.poll().0,
            vec![AgentEvent::TurnCompleted { last_message: None }]
        );
    }

    #[test]
    fn test_partial_line_is_held_until_its_newline_arrives() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rollout.jsonl");
        let mut tailer = Tailer::from_start(TranscriptKind::Codex, path.clone());

        // Observed mid-write: the record is not valid JSON yet, and parsing it
        // now would silently drop the event for good
        append(&path, "{\"type\":\"event_msg\",\"payload\":{\"type\":\"tas");
        assert!(tailer.poll().0.is_empty());

        append(&path, "k_started\"}}\n");
        assert_eq!(
            tailer.poll().0,
            vec![AgentEvent::TurnStarted { title: None }]
        );
    }

    #[test]
    fn test_attach_at_end_does_not_replay_history() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rollout.jsonl");
        for _ in 0..10 {
            append(
                &path,
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n",
            );
        }

        // A resumed session's file is full of history that already happened
        let (mut tailer, _) = Tailer::attach(TranscriptKind::Codex, path.clone());
        assert!(
            tailer.poll().0.is_empty(),
            "attaching must not replay the existing conversation as live events"
        );

        append(
            &path,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\"}}\n",
        );
        assert_eq!(tailer.poll().0.len(), 1);
    }

    #[test]
    fn test_attach_seeds_usage_from_the_tail() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rollout.jsonl");
        append(&path, "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"model\":\"gpt-5-codex\",\"model_context_window\":272000,\"total_token_usage\":{\"total_tokens\":10}}}}\n");
        // Plenty of noise after it
        for _ in 0..50 {
            append(
                &path,
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\"}}\n",
            );
        }
        append(&path, "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"model\":\"gpt-5-codex\",\"model_context_window\":272000,\"total_token_usage\":{\"total_tokens\":48000}}}}\n");

        let (_, seed) = Tailer::attach(TranscriptKind::Codex, path);
        let seed = seed.expect("usage should be seeded so the display is not blank");
        assert_eq!(
            seed.total_tokens,
            Some(48_000),
            "the most recent figures win"
        );
        assert_eq!(seed.context_window, Some(272_000));
    }

    #[test]
    fn test_missing_file_is_not_an_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("never-created.jsonl");

        let (mut tailer, seed) = Tailer::attach(TranscriptKind::Codex, path.clone());
        assert!(seed.is_none());
        assert!(tailer.poll().0.is_empty());

        // And it starts working when the agent finally creates it
        append(
            &path,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n",
        );
        assert_eq!(tailer.poll().0.len(), 1);
    }

    #[test]
    fn test_truncated_file_re_attaches_instead_of_misreading() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rollout.jsonl");
        append(
            &path,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n",
        );
        let mut tailer = Tailer::from_start(TranscriptKind::Codex, path.clone());
        assert_eq!(tailer.poll().0.len(), 1);

        // Replaced by something shorter: reading from the old offset would
        // interpret the middle of an unrelated record as a continuation
        std::fs::write(&path, "{}\n").unwrap();
        assert!(tailer.poll().0.is_empty());
        assert_eq!(tailer.offset, 3);
    }

    #[test]
    fn test_garbage_lines_do_not_wedge_the_tailer() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rollout.jsonl");
        let mut tailer = Tailer::from_start(TranscriptKind::Codex, path.clone());

        append(&path, "not json\n\n{}\n[]\nnull\n");
        assert!(tailer.poll().0.is_empty());

        // Still reading afterwards
        append(
            &path,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n",
        );
        assert_eq!(tailer.poll().0.len(), 1);
    }

    #[test]
    fn test_a_character_split_across_the_read_boundary_survives() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rollout.jsonl");
        let mut tailer = Tailer::from_start(TranscriptKind::Codex, path.clone());

        // Append a record whose message contains a multi-byte character, in
        // two writes that cut through the middle of it. Decoding each chunk in
        // isolation would replace the split bytes with U+FFFD permanently -
        // the offset has already moved past them.
        let record =
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\",\"last_agent_message\":\"café ☕\"}}\n";
        let bytes = record.as_bytes();
        let split = record.find('é').unwrap() + 1; // mid-character

        append_bytes(&path, &bytes[..split]);
        assert!(tailer.poll().0.is_empty());

        append_bytes(&path, &bytes[split..]);
        assert_eq!(
            tailer.poll().0,
            vec![AgentEvent::TurnCompleted {
                last_message: Some("café ☕".to_string())
            }]
        );
    }

    #[test]
    fn test_decode_prefix_holds_truncation_but_steps_over_corruption() {
        // A character cut short is still on its way, so its bytes are held
        let truncated = "aé".as_bytes();
        let (text, keep_from) = decode_prefix(&truncated[..truncated.len() - 1]);
        assert_eq!(text, "a");
        assert_eq!(keep_from, 1, "the incomplete character must be held back");

        // Genuinely invalid bytes must be stepped over, or one corrupt byte
        // would wedge the tailer forever
        let (text, keep_from) = decode_prefix(&[b'a', 0xff, b'b']);
        assert_eq!(text, "a\u{FFFD}b");
        assert_eq!(keep_from, 3);

        let (text, keep_from) = decode_prefix("plain".as_bytes());
        assert_eq!(text, "plain");
        assert_eq!(keep_from, 5);
    }

    #[test]
    fn test_from_start_reads_what_the_session_already_wrote() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rollout.jsonl");

        // Codex starts working before Panoptes has finished discovering its
        // conversation, so by the time watching begins the turn is under way
        append(
            &path,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n",
        );
        append(&path, "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"shell\",\"call_id\":\"c1\"}}\n");

        let mut tailer = Tailer::from_start(TranscriptKind::Codex, path);
        let (events, _) = tailer.poll();
        assert_eq!(
            events.len(),
            2,
            "a session's own opening events must not be skipped"
        );
    }

    #[test]
    fn test_claude_tailer_never_produces_state_events() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("transcript.jsonl");
        let mut tailer = Tailer::from_start(TranscriptKind::Claude, path.clone());

        append(
            &path,
            "{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n",
        );
        append(&path, "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-opus-4-8\",\"usage\":{\"input_tokens\":10}}}\n");

        let (events, _) = tailer.poll();
        assert!(
            events.iter().all(|e| matches!(e, AgentEvent::Usage(_))),
            "hooks own Claude's state; the transcript must only supplement, got {events:?}"
        );
    }
}
