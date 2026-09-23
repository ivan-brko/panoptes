//! Draining a PTY on its own thread
//!
//! A PTY's kernel buffer is tiny - about 1 KB on macOS - and a child that
//! fills it stops until someone reads. When the UI thread was that someone,
//! reading once per event-loop pass with up to 16 ms of sleep between passes,
//! a chatty child was held to roughly 64 KB/s. Claude Code's fullscreen
//! renderer writes 300-700 KB per trackpad flick, and does not block: it
//! queues frames in userspace and delivers them seconds late, so the view
//! kept scrolling long after the wheel stopped.
//!
//! So every PTY gets a thread that does nothing but drain it, into a queue
//! the UI thread empties at its own pace. The queue is bounded in *bytes*:
//! when it is full the thread stops reading, the kernel buffer fills, and the
//! child blocks - the same backpressure as before, with a megabyte of
//! headroom for a scroll burst instead of a kilobyte.
//!
//! Each read is queued as its own chunk. Read boundaries are part of the
//! stream's meaning here: query replies are answered with the cursor as of
//! the read that carried the query, and a drag hold replays reads one by one.

use anyhow::{Context, Result};
use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;

/// Largest single read, and so the largest chunk the queue ever holds
///
/// Bigger than the old 4 KB UI-thread read: the thread reads until the
/// kernel buffer is empty anyway, and fewer, larger chunks mean fewer queue
/// round trips on both sides of a burst.
pub(crate) const READ_CHUNK: usize = 64 * 1024;

/// How many bytes may wait in the queue before the reader stops reading
///
/// Sized for a scroll burst: a whole trackpad flick of fullscreen repaints
/// at a large window fits, so the child never feels the UI's cadence, while
/// a runaway child (`yes`) is held to a megabyte per session. A read that
/// does not fit waits in the thread's hands, so what is queued plus what is
/// in flight never exceeds this plus one [`READ_CHUNK`].
pub(crate) const QUEUE_CAP: usize = 1024 * 1024;

/// How long the reader waits on a quiet PTY before checking for shutdown
///
/// The thread owns a duplicate of the master fd, so the handle being
/// dropped does not wake it; this timeout is what bounds how long a dropped
/// session's thread outlives it.
#[cfg(unix)]
const POLL_TIMEOUT_MS: i32 = 50;

/// How the stream ended, kept until every chunk before it has been taken
enum End {
    /// The PTY reported end of file
    Eof,
    /// A read failed - typically `EIO` once the child is gone
    Error(io::Error),
}

/// What the reader thread and the consumer share
#[derive(Default)]
struct Queue {
    chunks: VecDeque<Vec<u8>>,
    /// Sum of `chunks`' lengths
    bytes: usize,
    /// Set once, after the last chunk; reported only once the queue is empty
    end: Option<End>,
    /// The consumer is gone; the thread should exit at its next chance
    stop: bool,
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    /// Signalled when room is made, or when the thread is told to stop
    room: Condvar,
}

impl Shared {
    /// The queue, even if a panicking thread poisoned the lock
    ///
    /// Every critical section leaves the queue consistent, so a poison is
    /// no reason to lose the session's output.
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn stopped(&self) -> bool {
        self.lock().stop
    }

    /// Queue one read, waiting for room first
    ///
    /// Returns false if the consumer went away meanwhile, which is the
    /// thread's cue to exit. An empty queue always takes the chunk, so a
    /// read is never stuck behind a cap it could not fit under.
    fn push(&self, chunk: Vec<u8>) -> bool {
        let mut queue = self.lock();
        while !queue.stop && queue.bytes > 0 && queue.bytes + chunk.len() > QUEUE_CAP {
            queue = self.room.wait(queue).unwrap_or_else(|e| e.into_inner());
        }
        if queue.stop {
            return false;
        }
        queue.bytes += chunk.len();
        queue.chunks.push_back(chunk);
        true
    }

    fn finish(&self, end: End) {
        self.lock().end = Some(end);
    }
}

/// The consumer's side of a PTY's reader thread
///
/// Dropping it tells the thread to stop; the thread notices within
/// [`POLL_TIMEOUT_MS`] (or at once, if it is waiting for room) and closes
/// its fd on the way out. Nothing joins it: a drop on the UI thread must not
/// wait out a poll timeout per session.
pub(crate) struct PtyReader {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl PtyReader {
    /// Start draining `master_fd` on a thread called `name`
    ///
    /// The thread reads a `dup` of the fd, which it owns and closes when it
    /// exits: the handle's own fd is never touched from here, so neither
    /// side can close the other's. The duplicate shares the open file
    /// description, `O_NONBLOCK` included - which the writer's retry logic
    /// depends on, so the thread waits in `poll` rather than clearing it.
    #[cfg(unix)]
    pub(crate) fn spawn(master_fd: std::os::unix::io::RawFd, name: String) -> Result<Self> {
        use std::os::unix::io::FromRawFd;

        // SAFETY: dup has no memory-safety preconditions; a bad fd is an
        // error return, checked below
        let fd = unsafe { libc::dup(master_fd) };
        if fd < 0 {
            return Err(io::Error::last_os_error())
                .context("Failed to duplicate PTY master for its reader");
        }
        // SAFETY: `fd` was just returned by dup, is open, and is owned by
        // nothing else; the File is its only owner from here on
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        Self::start(name, move |shared| drain_nonblocking(file, &shared))
    }

    /// Start draining a blocking reader on a thread called `name`
    ///
    /// Without `poll`, the thread can only notice shutdown between reads;
    /// a dropped session's thread ends when its PTY does.
    #[cfg(not(unix))]
    pub(crate) fn spawn(reader: Box<dyn Read + Send>, name: String) -> Result<Self> {
        Self::start(name, move |shared| drain_blocking(reader, &shared))
    }

    fn start(name: String, drain: impl FnOnce(Arc<Shared>) + Send + 'static) -> Result<Self> {
        let shared = Arc::new(Shared::default());
        let thread_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name(name)
            .spawn(move || drain(thread_shared))
            .context("Failed to start PTY reader thread")?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    /// A reader that delivers `reads` and then fails with `error`
    ///
    /// How a PTY ends differs by platform - a dead child's master reads as
    /// `EIO` on Linux but as end of file on macOS - so the error path is
    /// exercised with a script rather than whatever the host kernel does.
    #[cfg(test)]
    pub(crate) fn scripted(reads: Vec<Vec<u8>>, error: io::Error) -> Result<Self> {
        Self::start("pty-reader-scripted".to_string(), move |shared| {
            for read in reads {
                if !shared.push(read) {
                    return;
                }
            }
            shared.finish(End::Error(error));
        })
    }

    /// Take the next read, if it fits in `budget` bytes
    ///
    /// `Ok(None)` means nothing to take: the queue is empty, the stream hit
    /// EOF, or the next read is bigger than `budget` (see
    /// [`PtyReader::has_pending`] to tell that apart). A read error is
    /// reported once the reads before it are taken, and on every call after,
    /// exactly as reading a dead PTY directly kept failing.
    pub(crate) fn try_recv_within(&self, budget: usize) -> Result<Option<Vec<u8>>> {
        let mut queue = self.shared.lock();
        if let Some(len) = queue.chunks.front().map(Vec::len) {
            if len > budget {
                return Ok(None);
            }
            let chunk = queue.chunks.pop_front();
            queue.bytes -= len;
            drop(queue);
            self.shared.room.notify_one();
            return Ok(chunk);
        }
        match &queue.end {
            None | Some(End::Eof) => Ok(None),
            Some(End::Error(e)) => Err(copy_error(e)).context("Failed to read from PTY"),
        }
    }

    /// Whether a read is waiting to be taken
    pub(crate) fn has_pending(&self) -> bool {
        !self.shared.lock().chunks.is_empty()
    }

    /// How many bytes are waiting to be taken
    #[cfg(test)]
    pub(crate) fn queued_bytes(&self) -> usize {
        self.shared.lock().bytes
    }

    /// The thread itself, so a test can see it end
    #[cfg(test)]
    pub(crate) fn take_thread(&mut self) -> Option<JoinHandle<()>> {
        self.thread.take()
    }
}

impl Drop for PtyReader {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        // Wakes a thread waiting for room; one in `poll` sees the flag at
        // its next timeout
        self.shared.room.notify_all();
        // Detached, not joined - see the type's doc comment
        drop(self.thread.take());
    }
}

/// `io::Error` is not `Clone`; rebuild one that displays the same
fn copy_error(e: &io::Error) -> io::Error {
    match e.raw_os_error() {
        Some(code) => io::Error::from_raw_os_error(code),
        None => io::Error::new(e.kind(), e.to_string()),
    }
}

/// The reader thread's loop: wait for the PTY, then read it dry
#[cfg(unix)]
fn drain_nonblocking(mut file: std::fs::File, shared: &Shared) {
    use std::os::unix::io::AsRawFd;

    let fd = file.as_raw_fd();
    let mut buf = vec![0u8; READ_CHUNK];
    loop {
        if shared.stopped() {
            return;
        }
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pfd` is a valid pollfd for the duration of the call
        let ready = unsafe { libc::poll(&mut pfd, 1, POLL_TIMEOUT_MS) };
        if ready < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            shared.finish(End::Error(err));
            return;
        }
        if ready == 0 {
            // Quiet: go round to check for shutdown
            continue;
        }

        let mut read_any = false;
        loop {
            match file.read(&mut buf) {
                Ok(0) => {
                    shared.finish(End::Eof);
                    return;
                }
                Ok(n) => {
                    read_any = true;
                    if !shared.push(buf[..n].to_vec()) {
                        return;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    shared.finish(End::Error(e));
                    return;
                }
            }
        }

        // A hangup that still reads as "nothing yet" would make every poll
        // return at once; wait out a timeout instead of spinning on it
        if !read_any && pfd.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            std::thread::sleep(std::time::Duration::from_millis(POLL_TIMEOUT_MS as u64));
        }
    }
}

/// The reader thread's loop where the PTY can only be read blocking
#[cfg(not(unix))]
fn drain_blocking(mut reader: Box<dyn Read + Send>, shared: &Shared) {
    let mut buf = vec![0u8; READ_CHUNK];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                shared.finish(End::Eof);
                return;
            }
            Ok(n) => {
                if !shared.push(buf[..n].to_vec()) {
                    return;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => {
                shared.finish(End::Error(e));
                return;
            }
        }
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use crate::session::pty::PtyHandle;
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    fn spawn_sh(script: &str) -> PtyHandle {
        PtyHandle::spawn(
            "sh",
            &["-c", script],
            std::path::Path::new("/tmp"),
            HashMap::new(),
            24,
            80,
        )
        .expect("Failed to spawn PTY")
    }

    /// Wait until `done` holds, failing the test after `limit`
    fn wait_for(limit: Duration, what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + limit;
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {}", what);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn test_reader_drains_without_consumer() {
        // Half a megabyte is some 500 kernel buffers' worth. With the PTY
        // read only when someone asks, the child would still be blocked on
        // its first kilobyte; the reader thread takes it all on its own.
        let mut pty = spawn_sh("head -c 524288 /dev/zero");

        wait_for(Duration::from_secs(10), "the writer to finish", || {
            !pty.is_alive()
        });
        assert_eq!(
            pty.queued_output_len(),
            524288,
            "everything the child wrote should be queued, untaken"
        );
    }

    #[test]
    fn test_reader_backpressure_is_bounded() {
        let mut pty = spawn_sh("yes");

        // Nothing is consumed, so the queue fills to its cap and stops there
        wait_for(Duration::from_secs(10), "the queue to fill", || {
            pty.queued_output_len() + READ_CHUNK > QUEUE_CAP
        });
        // Small reads can still squeeze into the last chunk's worth of room;
        // once one does not fit, the queue stops growing
        let mut full = pty.queued_output_len();
        wait_for(Duration::from_secs(5), "the queue to stop growing", || {
            std::thread::sleep(Duration::from_millis(100));
            let now = pty.queued_output_len();
            std::mem::replace(&mut full, now) == now
        });
        std::thread::sleep(Duration::from_millis(300));

        assert!(
            pty.queued_output_len() <= QUEUE_CAP,
            "queued {} bytes against a cap of {}",
            pty.queued_output_len(),
            QUEUE_CAP
        );
        assert_eq!(
            pty.queued_output_len(),
            full,
            "a full queue must stop the reader, not grow"
        );
        assert!(
            pty.is_alive(),
            "the child is blocked on a full PTY, not gone"
        );

        // Taking output makes room, and the reader carries on
        pty.try_recv()
            .unwrap()
            .expect("a full queue has a read to take");
        wait_for(Duration::from_secs(5), "the queue to refill", || {
            pty.queued_output_len() + READ_CHUNK > QUEUE_CAP
        });

        pty.kill().unwrap();
    }

    #[test]
    fn test_reader_preserves_order_and_boundaries() {
        // No newlines, so the line discipline passes it through unchanged:
        // what arrives must be exactly what was written
        let count = 60_000;
        let expected: String = (0..count).map(|i| format!("{},", i)).collect();
        let mut pty = spawn_sh(&format!(
            "i=0; while [ $i -lt {count} ]; do printf '%d,' $i; i=$((i+1)); done"
        ));

        let mut received = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(60);
        while received.len() < expected.len() {
            assert!(Instant::now() < deadline, "output never completed");
            match pty.try_recv() {
                Ok(Some(chunk)) => {
                    // One read, one message: never empty, never merged
                    // beyond what a single read can return
                    assert!(!chunk.is_empty(), "an empty chunk was queued");
                    assert!(chunk.len() <= READ_CHUNK, "chunk bigger than a read");
                    received.extend(chunk);
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(1)),
                Err(e) => panic!("read failed before the output completed: {e:#}"),
            }
        }
        assert_eq!(String::from_utf8_lossy(&received), expected);
    }

    #[test]
    fn test_reader_thread_exits_on_drop() {
        // A live child: the thread is in `poll` on a quiet PTY
        let mut live = PtyHandle::spawn(
            "cat",
            &[],
            std::path::Path::new("/tmp"),
            HashMap::new(),
            24,
            80,
        )
        .unwrap();
        let live_thread = live.take_reader_thread().unwrap();
        live.kill().unwrap();
        drop(live);

        // A child that is gone, with output left untaken
        let mut dead = spawn_sh("echo bye");
        wait_for(Duration::from_secs(5), "the child to exit", || {
            !dead.is_alive()
        });
        let dead_thread = dead.take_reader_thread().unwrap();
        drop(dead);

        // A child blocked on a full queue: the thread waits for room
        let mut blocked = spawn_sh("yes");
        wait_for(Duration::from_secs(10), "the queue to fill", || {
            blocked.queued_output_len() + READ_CHUNK > QUEUE_CAP
        });
        let blocked_thread = blocked.take_reader_thread().unwrap();
        blocked.kill().unwrap();
        drop(blocked);

        for (what, thread) in [
            ("live", live_thread),
            ("dead", dead_thread),
            ("blocked", blocked_thread),
        ] {
            wait_for(
                Duration::from_secs(2),
                &format!("the {what} child's reader thread to exit"),
                || thread.is_finished(),
            );
            thread.join().expect("reader thread panicked");
        }
    }

    #[test]
    fn test_reader_thread_exits_on_its_own_when_the_child_is_gone() {
        // Nobody drops the handle: the read error alone ends the thread,
        // which is what keeps a suspended session's reader from lingering
        let mut pty = spawn_sh("true");
        let thread = pty.take_reader_thread().unwrap();
        wait_for(Duration::from_secs(5), "the reader to see the end", || {
            thread.is_finished()
        });
        // What it read before the end is still there to take, then the end.
        // A dead child's master reads as EIO on Linux and as end of file on
        // macOS; either way the stream stays ended, as the fd itself did.
        while let Ok(Some(_)) = pty.try_recv() {}
        for _ in 0..2 {
            match pty.try_recv() {
                Err(e) => assert_eq!(e.to_string(), "Failed to read from PTY"),
                Ok(None) => assert!(!pty.has_pending_output()),
                Ok(Some(chunk)) => panic!("output after the end: {chunk:?}"),
            }
        }
    }
}
