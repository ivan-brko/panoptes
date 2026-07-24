//! The terminal-proxy layer: what makes standing in the middle invisible
//!
//! Panoptes sits between each agent and the real terminal. The agent
//! negotiates with *us* as if we were iTerm: it queries capabilities, pushes
//! keyboard protocols, asks for the color scheme, and expects a terminal
//! that answers. Every query left unanswered is a feature the agent silently
//! degrades — Codex's theme detection, Claude's terminal identification,
//! kitty keyboard handling.
//!
//! This module holds the pieces that close that gap:
//!
//! - [`HostTerminal`]: the real terminal's identity and colors, probed once
//!   at startup, so children can be answered with the truth instead of a
//!   guess.
//! - [`ModeTracker`]: the terminal modes a child believes are active that
//!   vt100 does not model (kitty keyboard flag stack, focus reporting).
//!   Drives kitty-aware input encoding and focus-event forwarding.
//! - [`QueryResponder`]: answers the queries agents actually send (verified
//!   by probing Claude Code 2.1.218 and Codex 0.145.0): DSR 6n, DA1,
//!   XTVERSION, kitty `CSI ? u`, DECRQM 2026, OSC 10/11.
//! - [`SyncFrameGate`]: holds output between `CSI ?2026h` and `CSI ?2026l`
//!   so the virtual terminal only ever ingests whole frames — the renderer
//!   then never paints a half-drawn screen.
//! - [`extract_osc52`]: pulls clipboard writes out of the stream so the app
//!   can forward them to the real terminal.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// How long a synchronized-output frame may stay open before being flushed
/// anyway. Protects against a child that crashes mid-frame; iTerm uses a
/// similar internal timeout.
const FRAME_TIMEOUT: Duration = Duration::from_millis(150);
/// Cap on a buffered frame; a frame larger than this is not a frame
const FRAME_MAX_BYTES: usize = 2 * 1024 * 1024;

// ============================================================================
// Host terminal identity
// ============================================================================

/// What the real terminal told us about itself at startup
#[derive(Debug, Default, Clone)]
pub struct HostTerminal {
    /// Foreground color payload exactly as the terminal reported it
    /// (e.g. `rgb:c7c7/c7c7/c7c7`), if it answered
    pub fg: Option<String>,
    /// Background color payload, if it answered
    pub bg: Option<String>,
    /// The raw XTVERSION reply (`DCS > | name version ST`), if it answered
    pub xtversion_reply: Option<Vec<u8>>,
}

static HOST_TERMINAL: OnceLock<HostTerminal> = OnceLock::new();

/// Record the probed host terminal (called once at startup)
pub fn set_host_terminal(host: HostTerminal) {
    let _ = HOST_TERMINAL.set(host);
}

/// The probed host terminal, or defaults if the probe never ran
pub fn host_terminal() -> &'static HostTerminal {
    HOST_TERMINAL.get_or_init(HostTerminal::default)
}

/// Probe the real terminal for its identity and colors
///
/// Must run in raw mode, before crossterm's event machinery starts reading
/// stdin — the replies arrive as raw bytes on stdin and would otherwise be
/// misparsed as key presses. A DSR 5 ("report status") is sent last as a
/// fence: terminals answer queries in order, so its `CSI 0n` reply means
/// every earlier reply that is coming has arrived.
pub fn probe_host_terminal() -> HostTerminal {
    use std::io::Write;

    let mut host = HostTerminal::default();

    let query = b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b[>0q\x1b[5n";
    if std::io::stdout()
        .write_all(query)
        .and_then(|()| std::io::stdout().flush())
        .is_err()
    {
        return host;
    }

    let mut buf = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(300);
    let mut chunk = [0u8; 1024];
    while Instant::now() < deadline {
        let mut pollfd = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut pollfd, 1, 50) };
        if ready <= 0 {
            continue;
        }
        let n = unsafe { libc::read(libc::STDIN_FILENO, chunk.as_mut_ptr().cast(), chunk.len()) };
        if n <= 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n as usize]);
        // The fence: everything the terminal will say has been said
        if find_subsequence(&buf, b"\x1b[0n").is_some() {
            break;
        }
    }

    host.fg = parse_osc_color_reply(&buf, 10);
    host.bg = parse_osc_color_reply(&buf, 11);
    host.xtversion_reply = parse_xtversion_reply(&buf);

    tracing::info!(
        fg = ?host.fg,
        bg = ?host.bg,
        xtversion = host.xtversion_reply.as_ref().map(|r| String::from_utf8_lossy(r).into_owned()),
        "Probed host terminal"
    );
    host
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Parse `OSC <code> ; <payload> (BEL | ST)` out of a reply buffer
fn parse_osc_color_reply(buf: &[u8], code: u8) -> Option<String> {
    let prefix = format!("\x1b]{};", code).into_bytes();
    let start = find_subsequence(buf, &prefix)? + prefix.len();
    let rest = &buf[start..];
    let end = rest
        .iter()
        .position(|&b| b == 0x07 || b == 0x1b)
        .unwrap_or(rest.len());
    let payload = String::from_utf8_lossy(&rest[..end]).into_owned();
    (!payload.is_empty() && payload != "?").then_some(payload)
}

/// Parse the raw `DCS > | ... ST` XTVERSION reply out of a reply buffer
fn parse_xtversion_reply(buf: &[u8]) -> Option<Vec<u8>> {
    let start = find_subsequence(buf, b"\x1bP>|")?;
    let rest = &buf[start..];
    let end = find_subsequence(rest, b"\x1b\\")? + 2;
    Some(rest[..end].to_vec())
}

// ============================================================================
// Mode tracking (kitty keyboard, focus reporting)
// ============================================================================

/// Tracks terminal modes the child enables that vt100 does not model
///
/// vt100 predates the kitty keyboard protocol and never tracked focus
/// reporting, but both Claude Code and Codex negotiate them. What the child
/// believes is active decides how Panoptes must encode its input (kitty
/// CSI-u vs legacy) and whether focus events should be forwarded.
///
/// Sequences can be split across PTY reads, so an incomplete escape at the
/// end of a chunk is carried into the next scan.
#[derive(Debug, Default)]
pub struct ModeTracker {
    /// Kitty keyboard flag stack, as the child believes it to be
    /// (`CSI > flags u` pushes, `CSI < n u` pops, `CSI = flags ; mode u` sets)
    kitty_stack: Vec<u16>,
    /// Whether the child enabled focus reporting (`CSI ? 1004 h`)
    focus_reporting: bool,
    /// Incomplete escape sequence carried over from the previous chunk
    carry: Vec<u8>,
}

impl ModeTracker {
    /// Scan a chunk of child output for mode-changing sequences
    pub fn scan(&mut self, bytes: &[u8]) {
        let mut data = std::mem::take(&mut self.carry);
        data.extend_from_slice(bytes);

        let mut i = 0;
        while i < data.len() {
            if data[i] != 0x1b {
                i += 1;
                continue;
            }
            match self.scan_sequence(&data[i..]) {
                SequenceScan::Complete(len) => i += len,
                SequenceScan::NotOfInterest => i += 1,
                SequenceScan::Incomplete => {
                    // Only a bounded tail can be a split sequence of interest;
                    // anything longer is some other (uninteresting) sequence
                    let tail = &data[i..];
                    if tail.len() <= 16 {
                        self.carry = tail.to_vec();
                    }
                    return;
                }
            }
        }
    }

    /// Try to parse one escape sequence of interest at the start of `data`
    /// (which begins with ESC). Applies its effect when recognised.
    fn scan_sequence(&mut self, data: &[u8]) -> SequenceScan {
        if data.len() < 2 {
            return SequenceScan::Incomplete;
        }
        if data[1] != b'[' {
            return SequenceScan::NotOfInterest;
        }
        // Find the CSI final byte
        let mut end = 2;
        loop {
            let Some(&b) = data.get(end) else {
                return SequenceScan::Incomplete;
            };
            if (0x40..=0x7e).contains(&b) {
                break;
            }
            end += 1;
        }
        let body = &data[2..end];
        let final_byte = data[end];
        let len = end + 1;

        match final_byte {
            b'u' if !body.is_empty() => {
                let params = &body[1..];
                match body[0] {
                    b'>' => {
                        let flags = parse_u16(params).unwrap_or(0);
                        self.kitty_stack.push(flags);
                    }
                    b'<' => {
                        let n = parse_u16(params).unwrap_or(1).max(1) as usize;
                        let keep = self.kitty_stack.len().saturating_sub(n);
                        self.kitty_stack.truncate(keep);
                    }
                    b'=' => {
                        // "Set" semantics: replace the current entry
                        let flags = parse_u16(params.split(|&b| b == b';').next().unwrap_or(b""))
                            .unwrap_or(0);
                        match self.kitty_stack.last_mut() {
                            Some(top) => *top = flags,
                            None => self.kitty_stack.push(flags),
                        }
                    }
                    _ => {}
                }
                SequenceScan::Complete(len)
            }
            b'h' | b'l' if body == b"?1004" => {
                self.focus_reporting = final_byte == b'h';
                SequenceScan::Complete(len)
            }
            _ => SequenceScan::Complete(len),
        }
    }

    /// The kitty flags the child believes are active, if any
    pub fn kitty_flags(&self) -> Option<u16> {
        self.kitty_stack.last().copied()
    }

    /// Whether the child believes focus reporting is on
    pub fn focus_reporting(&self) -> bool {
        self.focus_reporting
    }
}

/// Outcome of scanning one escape sequence in [`ModeTracker`]
enum SequenceScan {
    /// Recognised or skipped whole; length consumed
    Complete(usize),
    /// Not a CSI sequence; advance past the ESC only
    NotOfInterest,
    /// Runs past the end of the chunk; carry it into the next scan
    Incomplete,
}

/// Parse an ASCII decimal, ignoring anything after the digits
fn parse_u16(bytes: &[u8]) -> Option<u16> {
    let digits: Vec<u8> = bytes
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .copied()
        .collect();
    if digits.is_empty() {
        return None;
    }
    std::str::from_utf8(&digits).ok()?.parse().ok()
}

// ============================================================================
// Query responder
// ============================================================================

/// Answers the terminal queries agents send, as the real terminal would
///
/// A query can be split across PTY reads, so a rolling tail of the previous
/// read is kept to match sequences that straddle a chunk boundary.
#[derive(Debug, Default)]
pub struct QueryResponder {
    /// Rolling tail of previously seen output: always the longest suffix
    /// that is a proper prefix of some query pattern, so an answered query
    /// can never be re-matched and unrelated bytes are never retained
    tail: Vec<u8>,
}

/// Every query pattern the responder recognises (used to size the tail)
const QUERY_PATTERNS: &[&[u8]] = &[
    b"\x1b[6n",
    b"\x1b[c",
    b"\x1b[>0q",
    b"\x1b[?u",
    b"\x1b[?2026$p",
    b"\x1b]10;?",
    b"\x1b]11;?",
];

/// The longest suffix of `data` that is a proper prefix of any pattern in
/// `patterns` — i.e. the only bytes worth carrying to the next scan
fn prefix_holdback_len(data: &[u8], patterns: &[&[u8]]) -> usize {
    let max = patterns
        .iter()
        .map(|p| p.len() - 1)
        .max()
        .unwrap_or(0)
        .min(data.len());
    for take in (1..=max).rev() {
        let suffix = &data[data.len() - take..];
        // Strictly shorter than the pattern: a complete match was already
        // acted on this scan and must not be seen again
        if patterns
            .iter()
            .any(|p| p.len() > suffix.len() && p.starts_with(suffix))
        {
            return take;
        }
    }
    0
}

impl QueryResponder {
    /// Scan child output for queries; returns the bytes to write back to the
    /// child, if any query was seen.
    ///
    /// `cursor` is the current 0-based (row, col) for CPR; `kitty_flags` is
    /// what the child believes its kitty stack top is.
    pub fn respond(
        &mut self,
        bytes: &[u8],
        cursor: (u16, u16),
        kitty_flags: Option<u16>,
    ) -> Option<Vec<u8>> {
        let mut combined = Vec::with_capacity(self.tail.len() + bytes.len());
        combined.extend_from_slice(&self.tail);
        combined.extend_from_slice(bytes);

        let mut reply: Vec<u8> = Vec::new();

        // DSR 6n -> CPR. Codex sends this at startup and hangs without an
        // answer.
        for _ in 0..count_occurrences(&combined, b"\x1b[6n") {
            let (row, col) = cursor;
            reply.extend_from_slice(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
        }

        // DA1 -> VT220-class with ANSI color. Both agents send `CSI c`.
        // (`CSI 0 c` is the same query; agents observed send the short form.)
        for _ in 0..count_occurrences(&combined, b"\x1b[c") {
            reply.extend_from_slice(b"\x1b[?62;22c");
        }

        // XTVERSION -> relay the real terminal's identity; Claude parses this
        // to decide which terminal it is running in. Fall back to naming
        // ourselves rather than staying silent: Claude has an explicit
        // "terminal ignored query" degraded path.
        for _ in 0..count_occurrences(&combined, b"\x1b[>0q") {
            match &host_terminal().xtversion_reply {
                Some(raw) => reply.extend_from_slice(raw),
                None => reply.extend_from_slice(
                    format!("\x1bP>|panoptes {}\x1b\\", env!("CARGO_PKG_VERSION")).as_bytes(),
                ),
            }
        }

        // Kitty keyboard query -> report the flags the child itself pushed
        // (0 if none). Any reply means "supported": Codex then uses CSI-u
        // encodings, which our input path speaks.
        for _ in 0..count_occurrences(&combined, b"\x1b[?u") {
            reply.extend_from_slice(format!("\x1b[?{}u", kitty_flags.unwrap_or(0)).as_bytes());
        }

        // DECRQM for synchronized output -> "recognised, currently reset".
        // Honest since SyncFrameGate actually implements it.
        for _ in 0..count_occurrences(&combined, b"\x1b[?2026$p") {
            reply.extend_from_slice(b"\x1b[?2026;2$y");
        }

        // OSC 10/11 -> the real terminal's colors, captured at startup. This
        // is how Codex detects light/dark themes. Stay silent when the host
        // never answered: a wrong color is worse than the agent's fallback.
        for _ in 0..count_occurrences(&combined, b"\x1b]10;?") {
            if let Some(fg) = &host_terminal().fg {
                reply.extend_from_slice(format!("\x1b]10;{}\x1b\\", fg).as_bytes());
            }
        }
        for _ in 0..count_occurrences(&combined, b"\x1b]11;?") {
            if let Some(bg) = &host_terminal().bg {
                reply.extend_from_slice(format!("\x1b]11;{}\x1b\\", bg).as_bytes());
            }
        }

        let keep = prefix_holdback_len(&combined, QUERY_PATTERNS);
        self.tail = combined[combined.len() - keep..].to_vec();

        (!reply.is_empty()).then_some(reply)
    }
}

/// Count non-overlapping occurrences of `needle` in `haystack`
///
/// For patterns that are prefixes of longer non-query sequences (`\x1b[c`
/// never is: `c` is a final byte; `\x1b[6n` likewise) counting complete
/// matches is exact.
fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }
    let mut count = 0;
    let mut i = 0;
    while i + needle.len() <= haystack.len() {
        if &haystack[i..i + needle.len()] == needle {
            count += 1;
            i += needle.len();
        } else {
            i += 1;
        }
    }
    count
}

// ============================================================================
// Synchronized-output frame gate
// ============================================================================

/// Holds child output between `CSI ?2026h` and `CSI ?2026l` so downstream
/// consumers (the virtual terminal, the fallback history) only ever see
/// whole frames — the same guarantee a supporting terminal gives the app.
///
/// A frame that stays open past [`FRAME_TIMEOUT`] or grows past
/// [`FRAME_MAX_BYTES`] is flushed as-is: correctness degrades to what we had
/// without the gate, never to a stall.
#[derive(Debug, Default)]
pub struct SyncFrameGate {
    /// Buffered bytes of the currently open frame (empty when no frame open)
    frame: Vec<u8>,
    /// When the open frame started
    frame_opened: Option<Instant>,
    /// Split-sequence carry: a chunk may end in the middle of the begin/end
    /// marker itself
    carry: Vec<u8>,
}

const SYNC_BEGIN: &[u8] = b"\x1b[?2026h";
const SYNC_END: &[u8] = b"\x1b[?2026l";

impl SyncFrameGate {
    /// Feed a chunk; returns the bytes that are ready for downstream
    pub fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut data = std::mem::take(&mut self.carry);
        data.extend_from_slice(bytes);

        let mut out = Vec::with_capacity(data.len());
        let mut rest: &[u8] = &data;

        loop {
            if self.frame_opened.is_some() {
                // Inside a frame: look for the end marker
                match find_subsequence(rest, SYNC_END) {
                    Some(pos) => {
                        self.frame.extend_from_slice(&rest[..pos + SYNC_END.len()]);
                        out.extend_from_slice(&self.frame);
                        self.frame.clear();
                        self.frame_opened = None;
                        rest = &rest[pos + SYNC_END.len()..];
                    }
                    None => {
                        // Hold back only a tail that is actually a prefix of
                        // the end marker; anything else joins the frame now
                        let keep = prefix_holdback_len(rest, &[SYNC_END]);
                        let (body, tail) = rest.split_at(rest.len() - keep);
                        self.frame.extend_from_slice(body);
                        self.carry = tail.to_vec();
                        break;
                    }
                }
            } else {
                // Outside a frame: look for the begin marker
                match find_subsequence(rest, SYNC_BEGIN) {
                    Some(pos) => {
                        out.extend_from_slice(&rest[..pos]);
                        self.frame_opened = Some(Instant::now());
                        self.frame.extend_from_slice(SYNC_BEGIN);
                        rest = &rest[pos + SYNC_BEGIN.len()..];
                    }
                    None => {
                        // Hold back only a tail that is actually a prefix of
                        // the begin marker; ordinary output passes through
                        // with no added latency
                        let keep = prefix_holdback_len(rest, &[SYNC_BEGIN]);
                        let (body, tail) = rest.split_at(rest.len() - keep);
                        out.extend_from_slice(body);
                        self.carry = tail.to_vec();
                        break;
                    }
                }
            }
        }

        // Cap runaway frames
        if self.frame.len() > FRAME_MAX_BYTES {
            out.extend_from_slice(&self.frame);
            self.frame.clear();
            self.frame_opened = None;
        }

        out
    }

    /// Flush anything held past its welcome (stale carry, timed-out frame)
    ///
    /// Called when no new output arrived this tick, so a child that stopped
    /// mid-sequence cannot leave its last bytes invisible.
    pub fn flush_stale(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let frame_expired = self
            .frame_opened
            .is_some_and(|opened| opened.elapsed() >= FRAME_TIMEOUT);
        if frame_expired {
            out.extend_from_slice(&self.frame);
            self.frame.clear();
            self.frame_opened = None;
        }
        // A carry only means anything while more bytes may complete it; when
        // the stream has gone quiet, release it. Without a frame open the
        // carry is at most 7 bytes of ordinary output.
        if self.frame_opened.is_none() && !self.carry.is_empty() {
            out.extend_from_slice(&self.carry);
            self.carry.clear();
        }
        out
    }
}

// ============================================================================
// Side-channel extraction (clipboard)
// ============================================================================

/// Find OSC 52 clipboard sequences in a chunk of child output
///
/// Returns each complete `OSC 52 ; ... (BEL | ST)` sequence found, for
/// forwarding to the real terminal. In a real tab this is how agents write
/// to the system clipboard; swallowing it silently breaks their copy
/// features. Sequences split across reads are handled by the caller's
/// pipeline feeding us committed frames, plus a bounded internal carry.
#[derive(Debug, Default)]
pub struct Osc52Extractor {
    carry: Vec<u8>,
}

/// An OSC 52 payload bigger than this is not a clipboard write we relay
/// (matches common terminal limits; keeps a hostile stream from ballooning)
const OSC52_MAX: usize = 1024 * 1024;

impl Osc52Extractor {
    /// Scan a chunk; returns complete OSC 52 sequences to forward
    pub fn extract(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut data = std::mem::take(&mut self.carry);
        data.extend_from_slice(bytes);

        let mut found = Vec::new();
        let mut i = 0;
        while let Some(rel) = find_subsequence(&data[i..], b"\x1b]52;") {
            let start = i + rel;
            let rest = &data[start..];
            // Find the terminator: BEL or ST
            let bel = rest.iter().position(|&b| b == 0x07);
            let st = find_subsequence(rest, b"\x1b\\");
            let end = match (bel, st) {
                (Some(b), Some(s)) => Some(if b < s { b + 1 } else { s + 2 }),
                (Some(b), None) => Some(b + 1),
                (None, Some(s)) => Some(s + 2),
                (None, None) => None,
            };
            match end {
                Some(len) => {
                    found.push(rest[..len].to_vec());
                    i = start + len;
                }
                None => {
                    // Incomplete: carry it, bounded
                    if rest.len() <= OSC52_MAX {
                        self.carry = rest.to_vec();
                    }
                    return found;
                }
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // ModeTracker
    // ------------------------------------------------------------------

    #[test]
    fn tracker_follows_kitty_push_pop() {
        let mut t = ModeTracker::default();
        assert_eq!(t.kitty_flags(), None);
        t.scan(b"\x1b[>1u");
        assert_eq!(t.kitty_flags(), Some(1));
        t.scan(b"\x1b[>7u");
        assert_eq!(t.kitty_flags(), Some(7));
        t.scan(b"\x1b[<u");
        assert_eq!(t.kitty_flags(), Some(1));
        t.scan(b"\x1b[<1u");
        assert_eq!(t.kitty_flags(), None);
        // Popping an empty stack is a no-op
        t.scan(b"\x1b[<5u");
        assert_eq!(t.kitty_flags(), None);
    }

    #[test]
    fn tracker_follows_focus_reporting() {
        let mut t = ModeTracker::default();
        assert!(!t.focus_reporting());
        t.scan(b"text\x1b[?1004hmore");
        assert!(t.focus_reporting());
        t.scan(b"\x1b[?1004l");
        assert!(!t.focus_reporting());
    }

    #[test]
    fn tracker_handles_sequences_split_across_chunks() {
        let mut t = ModeTracker::default();
        t.scan(b"\x1b[>");
        t.scan(b"1u");
        assert_eq!(t.kitty_flags(), Some(1));
    }

    #[test]
    fn tracker_ignores_unrelated_sequences() {
        let mut t = ModeTracker::default();
        t.scan(b"\x1b[38;5;100mhello\x1b[0m\x1b[?25l\x1b[2J");
        assert_eq!(t.kitty_flags(), None);
        assert!(!t.focus_reporting());
    }

    // ------------------------------------------------------------------
    // QueryResponder
    // ------------------------------------------------------------------

    #[test]
    fn responder_answers_dsr_with_cursor_position() {
        let mut q = QueryResponder::default();
        let reply = q.respond(b"\x1b[6n", (4, 9), None).unwrap();
        assert_eq!(reply, b"\x1b[5;10R");
    }

    #[test]
    fn responder_answers_da1() {
        let mut q = QueryResponder::default();
        let reply = q.respond(b"\x1b[c", (0, 0), None).unwrap();
        assert_eq!(reply, b"\x1b[?62;22c");
    }

    #[test]
    fn responder_answers_kitty_query_from_tracked_flags() {
        let mut q = QueryResponder::default();
        let reply = q.respond(b"\x1b[?u", (0, 0), Some(7)).unwrap();
        assert_eq!(reply, b"\x1b[?7u");

        let mut q = QueryResponder::default();
        let reply = q.respond(b"\x1b[?u", (0, 0), None).unwrap();
        assert_eq!(reply, b"\x1b[?0u");
    }

    #[test]
    fn responder_answers_decrqm_2026() {
        let mut q = QueryResponder::default();
        let reply = q.respond(b"\x1b[?2026$p", (0, 0), None).unwrap();
        assert_eq!(reply, b"\x1b[?2026;2$y");
    }

    #[test]
    fn responder_handles_query_split_across_reads() {
        let mut q = QueryResponder::default();
        assert!(q.respond(b"\x1b[6", (0, 0), None).is_none());
        let reply = q.respond(b"n", (0, 0), None).unwrap();
        assert_eq!(reply, b"\x1b[1;1R");
    }

    #[test]
    fn responder_does_not_double_answer_from_its_tail() {
        let mut q = QueryResponder::default();
        assert!(q.respond(b"\x1b[6n", (0, 0), None).is_some());
        // The tail must not re-match the query already answered
        assert!(q.respond(b"hello", (0, 0), None).is_none());
    }

    #[test]
    fn responder_stays_silent_on_color_queries_without_host_colors() {
        // HOST_TERMINAL is process-global; in tests it is unset (defaults),
        // so color queries must produce no reply rather than a lie.
        let mut q = QueryResponder::default();
        assert!(q.respond(b"\x1b]10;?\x1b]11;?", (0, 0), None).is_none());
    }

    // ------------------------------------------------------------------
    // SyncFrameGate
    // ------------------------------------------------------------------

    #[test]
    fn gate_passes_plain_output_straight_through() {
        let mut g = SyncFrameGate::default();
        let out = g.push(b"hello world, no frames here....");
        let stale = g.flush_stale();
        let mut all = out;
        all.extend(stale);
        assert_eq!(all, b"hello world, no frames here....");
    }

    #[test]
    fn gate_holds_a_frame_until_it_closes() {
        let mut g = SyncFrameGate::default();
        let out = g.push(b"pre\x1b[?2026hFRAME-PART-1");
        assert_eq!(out, b"pre");

        let out = g.push(b"FRAME-PART-2\x1b[?2026lpost");
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("FRAME-PART-1FRAME-PART-2"), "{:?}", text);
        assert!(text.ends_with("post"), "{:?}", text);
    }

    #[test]
    fn gate_handles_markers_split_across_chunks() {
        let mut g = SyncFrameGate::default();
        let mut all = Vec::new();
        all.extend(g.push(b"\x1b[?20"));
        all.extend(g.push(b"26hbody\x1b[?2026"));
        all.extend(g.push(b"l"));
        let text = String::from_utf8_lossy(&all);
        assert!(text.contains("body"), "{:?}", text);
    }

    #[test]
    fn gate_flushes_a_timed_out_frame() {
        let mut g = SyncFrameGate::default();
        let out = g.push(b"\x1b[?2026hstuck frame");
        assert!(out.is_empty());

        // Not yet stale
        assert!(g.flush_stale().is_empty());

        // Force the timeout
        g.frame_opened = Some(Instant::now() - FRAME_TIMEOUT);
        let flushed = g.flush_stale();
        assert!(String::from_utf8_lossy(&flushed).contains("stuck frame"));
    }

    // ------------------------------------------------------------------
    // Osc52Extractor
    // ------------------------------------------------------------------

    #[test]
    fn osc52_sequences_are_found_with_both_terminators() {
        let mut e = Osc52Extractor::default();
        let found = e.extract(b"a\x1b]52;c;aGVsbG8=\x07b\x1b]52;c;d29ybGQ=\x1b\\c");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0], b"\x1b]52;c;aGVsbG8=\x07");
        assert_eq!(found[1], b"\x1b]52;c;d29ybGQ=\x1b\\");
    }

    #[test]
    fn osc52_split_across_reads_is_reassembled() {
        let mut e = Osc52Extractor::default();
        assert!(e.extract(b"\x1b]52;c;aGVs").is_empty());
        let found = e.extract(b"bG8=\x07");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0], b"\x1b]52;c;aGVsbG8=\x07");
    }

    // ------------------------------------------------------------------
    // Host reply parsing
    // ------------------------------------------------------------------

    #[test]
    fn host_probe_replies_parse() {
        let buf = b"\x1b]10;rgb:c7c7/c7c7/c7c7\x1b\\\x1b]11;rgb:1e1e/2020/2222\x07\x1bP>|iTerm2 3.5.14\x1b\\\x1b[0n";
        assert_eq!(
            parse_osc_color_reply(buf, 10).as_deref(),
            Some("rgb:c7c7/c7c7/c7c7")
        );
        assert_eq!(
            parse_osc_color_reply(buf, 11).as_deref(),
            Some("rgb:1e1e/2020/2222")
        );
        assert_eq!(
            parse_xtversion_reply(buf).as_deref(),
            Some(b"\x1bP>|iTerm2 3.5.14\x1b\\".as_ref())
        );
    }

    #[test]
    fn host_probe_ignores_missing_replies() {
        assert_eq!(parse_osc_color_reply(b"\x1b[0n", 11), None);
        assert_eq!(parse_xtversion_reply(b"\x1b[0n"), None);
    }
}
