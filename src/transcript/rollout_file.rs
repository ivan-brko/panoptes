//! Codex rollout files, plain or zstd-compressed
//!
//! Codex writes every rollout as `rollout-<ts>-<uuid>.jsonl`, and later
//! zstd-compresses idle ones in place to `rollout-<ts>-<uuid>.jsonl.zst`. The
//! rule, from codex-rs at `rust-v0.156.1` (`rollout/src/compression.rs`): a
//! worker at startup compresses each rollout idle for at least seven days
//! (`MIN_ROLLOUT_AGE`) whose writer lock is not held, as a single zstd frame.
//! A live session always writes plain `.jsonl`. Resuming a compressed one
//! (`materialize_rollout_for_append`, `rollout/src/recorder.rs`) decompresses it
//! back to plain with the same mtime, deletes the `.zst`, and then appends.
//!
//! Every place that discovers rollouts goes through [`is_rollout_name`] so a
//! compressed conversation is never silently left out, and every place that
//! reads one from its start goes through [`open`], which hands back the
//! decompressed bytes either way.
//!
//! Byte offsets into a rollout always count *decompressed* bytes: a compressed
//! file is read as the stream it decodes to, never as the bytes on disk.
//!
//! Nothing ever appends to a compressed rollout, so the live tailer
//! (`transcript::Tailer`) never reads one: a resumed conversation is plain
//! again by the time it grows.

use std::io::Read;
use std::path::{Path, PathBuf};

/// The suffix of a rollout Codex is writing, or has left uncompressed
pub const PLAIN_SUFFIX: &str = ".jsonl";

/// The suffix of a rollout Codex has compressed
pub const COMPRESSED_SUFFIX: &str = ".jsonl.zst";

/// Whether a file name is a rollout's, plain or compressed
///
/// Only the suffix is checked; callers that also need the `rollout-` prefix
/// check it themselves.
pub fn is_rollout_name(name: &str) -> bool {
    name.ends_with(PLAIN_SUFFIX) || name.ends_with(COMPRESSED_SUFFIX)
}

/// Whether a path names a rollout file, plain or compressed
pub fn is_rollout_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(is_rollout_name)
}

/// Whether a path names a compressed rollout
pub fn is_compressed(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(COMPRESSED_SUFFIX))
}

/// A rollout's file name without its suffix - the same for both forms
///
/// `rollout-<ts>-<uuid>.jsonl` and `rollout-<ts>-<uuid>.jsonl.zst` both give
/// `rollout-<ts>-<uuid>`, which ends in the conversation ID.
pub fn stem(path: &Path) -> Option<&str> {
    let name = path.file_name()?.to_str()?;
    name.strip_suffix(COMPRESSED_SUFFIX)
        .or_else(|| name.strip_suffix(PLAIN_SUFFIX))
}

/// Drop every compressed rollout whose plain twin is also listed
///
/// The same conversation in both forms means Codex has the plain file to hand,
/// and that is the one that can still be growing. Order is otherwise kept.
pub fn prefer_plain(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let plain: std::collections::HashSet<PathBuf> = paths
        .iter()
        .filter(|path| !is_compressed(path))
        .cloned()
        .collect();
    paths
        .into_iter()
        .filter(|path| !is_compressed(path) || !plain.contains(&plain_twin(path)))
        .collect()
}

/// The plain path a compressed rollout would have had
///
/// For a path that is already plain, the path itself.
pub fn plain_twin(path: &Path) -> PathBuf {
    match path
        .to_str()
        .and_then(|p| p.strip_suffix(".zst"))
        .filter(|_| is_compressed(path))
    {
        Some(plain) => PathBuf::from(plain),
        None => path.to_path_buf(),
    }
}

/// The compressed path a plain rollout becomes once Codex compresses it
///
/// For a path that is already compressed, the path itself.
pub fn compressed_twin(path: &Path) -> PathBuf {
    if is_compressed(path) {
        return path.to_path_buf();
    }
    PathBuf::from(format!("{}.zst", path.display()))
}

/// Open a rollout for reading from its first byte, decompressing if needed
///
/// A compressed rollout is decoded as a stream: only as much is inflated as is
/// read. A corrupt one fails here if its frame header is bad, or on a later
/// read if the damage is further in.
pub fn open(path: &Path) -> std::io::Result<Box<dyn Read + Send>> {
    let file = std::fs::File::open(path)?;
    if !is_compressed(path) {
        return Ok(Box::new(file));
    }
    // Codex writes a single zstd frame per rollout
    let decoder = ruzstd::decoding::StreamingDecoder::new(std::io::BufReader::new(file))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(Box::new(decoder))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Compress some text as Codex does, for writing a `.jsonl.zst` fixture
    pub(crate) fn compress(text: &str) -> Vec<u8> {
        ruzstd::encoding::compress_to_vec(
            text.as_bytes(),
            ruzstd::encoding::CompressionLevel::Fastest,
        )
    }

    #[test]
    fn test_names_both_forms_and_nothing_else() {
        assert!(is_rollout_name("rollout-a.jsonl"));
        assert!(is_rollout_name("rollout-a.jsonl.zst"));
        assert!(!is_rollout_name("rollout-a.zst"));
        assert!(!is_rollout_name("rollout-a.json"));
        assert!(!is_rollout_name("rollout-a.jsonl.tmp"));
        assert!(is_compressed(Path::new("/d/rollout-a.jsonl.zst")));
        assert!(!is_compressed(Path::new("/d/rollout-a.jsonl")));
    }

    #[test]
    fn test_stem_is_the_same_for_both_forms() {
        assert_eq!(
            stem(Path::new("/d/rollout-t-id.jsonl")),
            Some("rollout-t-id")
        );
        assert_eq!(
            stem(Path::new("/d/rollout-t-id.jsonl.zst")),
            Some("rollout-t-id")
        );
        assert_eq!(stem(Path::new("/d/notes.txt")), None);
    }

    #[test]
    fn test_prefer_plain_drops_only_a_twinned_compressed_file() {
        let paths = vec![
            PathBuf::from("/d/rollout-a.jsonl.zst"),
            PathBuf::from("/d/rollout-b.jsonl.zst"),
            PathBuf::from("/d/rollout-a.jsonl"),
        ];
        assert_eq!(
            prefer_plain(paths),
            vec![
                PathBuf::from("/d/rollout-b.jsonl.zst"),
                PathBuf::from("/d/rollout-a.jsonl"),
            ]
        );
    }

    #[test]
    fn test_opens_plain_and_compressed_to_the_same_text() {
        let dir = tempfile::TempDir::new().unwrap();
        let text = "{\"type\":\"session_meta\"}\n{\"type\":\"event_msg\"}\n";
        let plain = dir.path().join("rollout-a.jsonl");
        let compressed = dir.path().join("rollout-b.jsonl.zst");
        std::fs::write(&plain, text).unwrap();
        std::fs::write(&compressed, compress(text)).unwrap();

        for path in [plain, compressed] {
            let mut read = String::new();
            open(&path).unwrap().read_to_string(&mut read).unwrap();
            assert_eq!(read, text, "for {}", path.display());
        }
    }

    #[test]
    fn test_a_corrupt_compressed_file_fails_to_read() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("rollout-bad.jsonl.zst");
        std::fs::write(&path, b"definitely not zstd").unwrap();
        let read = open(&path).and_then(|mut r| {
            let mut buf = Vec::new();
            r.read_to_end(&mut buf).map(|_| buf)
        });
        assert!(read.is_err());
    }
}
