//! Codex thread names
//!
//! Codex names each conversation ("thread") itself and keeps the names in
//! `$CODEX_HOME/session_index.jsonl`, appending one line per rename:
//!
//! ```text
//! {"id":"<thread id>","thread_name":"Reply with hi","updated_at":"2026-09-23T10:20:46.971421Z"}
//! ```
//!
//! The rollout never carries the name, so this file is the only place a Codex
//! session's title can come from. It is shared by every conversation under one
//! `CODEX_HOME`, which is why [`SessionIndex`] follows it once per home rather
//! than once per session, and why every lookup filters on the thread ID.
//!
//! A thread can appear many times. The line with the newest `updated_at` is
//! its current name; on a tie, or when a timestamp is missing, the later line
//! wins, since the file is append-only.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

/// The file's name inside `CODEX_HOME`
const FILE_NAME: &str = "session_index.jsonl";

/// Where the thread-name index lives for a Codex sessions directory
///
/// Watch targets carry `$CODEX_HOME/sessions`, so `CODEX_HOME` is its parent.
pub fn index_path(codex_sessions_dir: &Path) -> Option<PathBuf> {
    Some(codex_sessions_dir.parent()?.join(FILE_NAME))
}

/// One thread's name as of one line
#[derive(Debug, Clone)]
struct Named {
    name: String,
    updated_at: Option<DateTime<Utc>>,
}

/// The newest name of every thread mentioned in these lines
///
/// Lines that are not a well-formed entry, or that carry an empty name, are
/// skipped rather than failing the lot: the file belongs to Codex and may be
/// read mid-write.
pub fn latest_names<'a>(lines: impl IntoIterator<Item = &'a str>) -> HashMap<String, String> {
    let mut latest: HashMap<String, Named> = HashMap::new();

    for line in lines {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let (Some(id), Some(name)) = (
            record.get("id").and_then(Value::as_str),
            record.get("thread_name").and_then(Value::as_str),
        ) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let updated_at = record
            .get("updated_at")
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Utc));

        let entry = Named {
            name: name.to_string(),
            updated_at,
        };
        match latest.get(id) {
            // An older timestamp further down loses; anything else is newer
            Some(held)
                if matches!((held.updated_at, entry.updated_at),
                    (Some(held), Some(this)) if this < held) => {}
            _ => {
                latest.insert(id.to_string(), entry);
            }
        }
    }

    latest.into_iter().map(|(id, n)| (id, n.name)).collect()
}

/// The current name of one thread, read from the whole index
///
/// For seeding a session on attach: its name may have been written long before
/// Panoptes started following the file. `None` when the file is missing or
/// never names the thread.
pub fn latest_name(path: &Path, thread_id: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    // Parsing is the cost; most lines are other threads', and those can be
    // rejected without it
    let mut names = latest_names(text.lines().filter(|line| line.contains(thread_id)));
    names.remove(thread_id)
}

/// Follows one `session_index.jsonl`, yielding what was appended since last time
///
/// Holds a byte offset and any unterminated final line, like the transcript
/// [`super::Tailer`] - but yields names rather than events, because which
/// session a line belongs to is for the caller to decide.
#[derive(Debug)]
pub struct SessionIndex {
    path: PathBuf,
    offset: u64,
    /// Bytes after the last newline seen, held until it arrives
    partial: Vec<u8>,
}

impl SessionIndex {
    /// Start following from the file's current end
    ///
    /// Whatever is already there is read by [`latest_name`] when each session
    /// attaches, so starting at the end is what keeps a busy index from being
    /// re-parsed in full every time a session starts.
    pub fn at_end(path: PathBuf) -> Self {
        let offset = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Self {
            path,
            offset,
            partial: Vec::new(),
        }
    }

    /// The newest name of every thread renamed since the last call
    ///
    /// Never fails: a missing or unreadable file yields nothing. A file that
    /// shrank was rewritten, so it is read again from the start - a name
    /// already reported is harmless to report again, while missing a rename is
    /// not.
    pub fn read_new(&mut self) -> HashMap<String, String> {
        let len = match std::fs::metadata(&self.path) {
            Ok(metadata) => metadata.len(),
            Err(_) => {
                // Gone: a recreated file must be read from its beginning
                self.offset = 0;
                self.partial.clear();
                return HashMap::new();
            }
        };
        if len < self.offset {
            self.offset = 0;
            self.partial.clear();
        }
        if len == self.offset {
            return HashMap::new();
        }

        let Ok(mut file) = std::fs::File::open(&self.path) else {
            return HashMap::new();
        };
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return HashMap::new();
        }
        let mut buf = Vec::new();
        if file.take(len - self.offset).read_to_end(&mut buf).is_err() {
            return HashMap::new();
        }
        self.offset += buf.len() as u64;

        let mut bytes = std::mem::take(&mut self.partial);
        bytes.extend_from_slice(&buf);
        // Everything up to the last newline is whole lines, and therefore
        // whole characters; the rest waits for the writer to finish it
        let complete = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        self.partial = bytes.split_off(complete);

        let text = String::from_utf8_lossy(&bytes);
        latest_names(text.lines())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    /// Several threads, renamed repeatedly, one update out of order; the
    /// line shape is the real file's, names and IDs redacted
    const FIXTURE: &str = r#"{"id":"thread-a","thread_name":"Reply with hi","updated_at":"2026-09-23T10:20:46.971421Z"}
{"id":"thread-b","thread_name":"Fix flaky test","updated_at":"2026-09-23T10:21:00.000000Z"}
{"id":"thread-a","thread_name":"Say hello politely","updated_at":"2026-09-23T10:25:00.000000Z"}
{"id":"unrelated","thread_name":"Someone else's thread","updated_at":"2026-09-23T10:26:00.000000Z"}
{"id":"thread-b","thread_name":"Fix the flaky login test","updated_at":"2026-09-23T10:30:00.000000Z"}
{"id":"thread-a","thread_name":"A stale rename written late","updated_at":"2026-09-23T10:22:00.000000Z"}
not json at all
{"id":"thread-b","thread_name":"   ","updated_at":"2026-09-23T10:40:00.000000Z"}
"#;

    #[test]
    fn test_latest_names_picks_each_threads_newest_name() {
        let names = latest_names(FIXTURE.lines());
        assert_eq!(names["thread-a"], "Say hello politely");
        assert_eq!(names["thread-b"], "Fix the flaky login test");
        assert_eq!(names.len(), 3, "one entry per thread, junk skipped");
    }

    #[test]
    fn test_latest_name_matches_only_the_asked_thread() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, FIXTURE).unwrap();

        assert_eq!(
            latest_name(&path, "thread-a").as_deref(),
            Some("Say hello politely")
        );
        assert_eq!(
            latest_name(&path, "thread-b").as_deref(),
            Some("Fix the flaky login test")
        );
        assert_eq!(latest_name(&path, "thread-c"), None);
        assert_eq!(
            latest_name(&dir.path().join("missing.jsonl"), "thread-a"),
            None
        );
    }

    #[test]
    fn test_later_line_wins_without_timestamps() {
        let names = latest_names([
            r#"{"id":"t","thread_name":"First"}"#,
            r#"{"id":"t","thread_name":"Second"}"#,
        ]);
        assert_eq!(names["t"], "Second");
    }

    #[test]
    fn test_index_path_is_beside_the_sessions_dir() {
        assert_eq!(
            index_path(Path::new("/home/u/.codex/sessions")),
            Some(PathBuf::from("/home/u/.codex/session_index.jsonl"))
        );
    }

    fn append(path: &Path, text: &str) {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }

    #[test]
    fn test_follows_only_what_was_appended() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, FIXTURE).unwrap();

        let mut index = SessionIndex::at_end(path.clone());
        assert!(index.read_new().is_empty(), "history is not news");

        // A line arriving in two writes is held until it is whole
        append(&path, r#"{"id":"thread-a","thread_name":"Wave"#);
        assert!(index.read_new().is_empty());
        append(&path, "\",\"updated_at\":\"2026-09-23T11:00:00Z\"}\n");
        let names = index.read_new();
        assert_eq!(names.len(), 1);
        assert_eq!(names["thread-a"], "Wave");
        assert!(index.read_new().is_empty(), "and exactly once");
    }

    #[test]
    fn test_rewritten_file_is_read_from_the_start() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, FIXTURE).unwrap();
        let mut index = SessionIndex::at_end(path.clone());

        std::fs::write(
            &path,
            "{\"id\":\"thread-a\",\"thread_name\":\"Compacted\"}\n",
        )
        .unwrap();
        assert_eq!(index.read_new()["thread-a"], "Compacted");
    }

    #[test]
    fn test_missing_file_is_not_an_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut index = SessionIndex::at_end(path.clone());
        assert!(index.read_new().is_empty());

        // Codex creates it on the first rename
        append(&path, "{\"id\":\"t\",\"thread_name\":\"Born\"}\n");
        assert_eq!(index.read_new()["t"], "Born");
    }
}
