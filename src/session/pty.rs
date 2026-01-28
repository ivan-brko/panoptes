//! PTY (Pseudo-Terminal) management for spawning and interacting with processes

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use ratatui::prelude::Rect;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

/// Information about a process exit
#[derive(Debug, Clone)]
pub struct ExitInfo {
    /// Exit code (0 for success, non-zero for error)
    pub code: i32,
    /// Whether the process exited successfully
    pub success: bool,
    /// Signal number if the process was killed by a signal (Unix only)
    pub signal: Option<i32>,
}

impl ExitInfo {
    /// Format the exit reason as a human-readable string
    pub fn format_reason(&self) -> String {
        if self.success {
            "Exited normally".to_string()
        } else if let Some(sig) = self.signal {
            format!("Killed by signal {} ({})", sig, signal_name(sig))
        } else {
            format!("Exit code: {}", self.code)
        }
    }
}

/// Get a human-readable name for a signal number
fn signal_name(sig: i32) -> &'static str {
    match sig {
        1 => "SIGHUP",
        2 => "SIGINT",
        3 => "SIGQUIT",
        4 => "SIGILL",
        6 => "SIGABRT",
        8 => "SIGFPE",
        9 => "SIGKILL",
        11 => "SIGSEGV",
        13 => "SIGPIPE",
        14 => "SIGALRM",
        15 => "SIGTERM",
        _ => "unknown",
    }
}

/// Bracketed paste start sequence
const PASTE_START: &[u8] = b"\x1b[200~";
/// Bracketed paste end sequence
const PASTE_END: &[u8] = b"\x1b[201~";

/// Maximum time to retry writes before giving up
const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
/// Delay between retry attempts when buffer is full
const WRITE_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(1);

/// Handle to a PTY with spawned process
pub struct PtyHandle {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    reader: Box<dyn Read + Send>,
}

impl PtyHandle {
    /// Spawn a new process in a PTY
    ///
    /// # Arguments
    /// * `cmd` - Command to execute
    /// * `args` - Command arguments
    /// * `working_dir` - Working directory for the process
    /// * `env` - Additional environment variables
    /// * `rows` - Initial PTY rows
    /// * `cols` - Initial PTY columns
    pub fn spawn(
        cmd: &str,
        args: &[&str],
        working_dir: &Path,
        env: HashMap<String, String>,
        rows: u16,
        cols: u16,
    ) -> Result<Self> {
        let pty_system = native_pty_system();

        // Create PTY with specified size
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to open PTY")?;

        // Build command
        let mut cmd_builder = CommandBuilder::new(cmd);
        cmd_builder.args(args);
        cmd_builder.cwd(working_dir);

        for (key, value) in env {
            cmd_builder.env(key, value);
        }

        // Spawn the process
        let child = pair
            .slave
            .spawn_command(cmd_builder)
            .context("Failed to spawn command in PTY")?;

        // Get reader and writer from master
        let reader = pair
            .master
            .try_clone_reader()
            .context("Failed to clone PTY reader")?;

        let writer = pair
            .master
            .take_writer()
            .context("Failed to take PTY writer")?;

        // Set non-blocking mode on Unix
        #[cfg(unix)]
        if let Some(fd) = pair.master.as_raw_fd() {
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFL);
                if flags != -1 {
                    libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                }
            }
        }

        Ok(Self {
            master: pair.master,
            child,
            writer,
            reader,
        })
    }

    /// Write all bytes with retry logic for non-blocking PTY
    ///
    /// Handles WouldBlock (EAGAIN) errors by retrying with small delays.
    /// Times out after WRITE_TIMEOUT to avoid infinite loops.
    fn write_all_with_retry(&mut self, data: &[u8]) -> Result<()> {
        let mut written = 0;
        let start = std::time::Instant::now();

        while written < data.len() {
            if start.elapsed() > WRITE_TIMEOUT {
                anyhow::bail!(
                    "Timed out writing to PTY after {:?} ({} of {} bytes written)",
                    WRITE_TIMEOUT,
                    written,
                    data.len()
                );
            }

            match self.writer.write(&data[written..]) {
                Ok(0) => anyhow::bail!("Write returned 0 bytes"),
                Ok(n) => written += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(WRITE_RETRY_DELAY);
                }
                Err(e) => return Err(e).context("Failed to write to PTY"),
            }
        }
        Ok(())
    }

    /// Write raw bytes to the PTY
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.write_all_with_retry(data)?;
        self.writer.flush().context("Failed to flush PTY writer")?;
        Ok(())
    }

    /// Write pasted text to the PTY, wrapped in bracketed paste sequences
    pub fn write_paste(&mut self, text: &str) -> Result<()> {
        self.write_all_with_retry(PASTE_START)
            .context("Failed to write paste start sequence")?;
        self.write_all_with_retry(text.as_bytes())
            .context("Failed to write paste content")?;
        self.write_all_with_retry(PASTE_END)
            .context("Failed to write paste end sequence")?;
        self.writer.flush().context("Failed to flush PTY writer")?;
        Ok(())
    }

    /// Send a key event to the PTY, converting it to appropriate escape sequences
    pub fn send_key(&mut self, key: KeyEvent) -> Result<()> {
        let bytes = key_event_to_bytes(key);
        if !bytes.is_empty() {
            self.write(&bytes)?;
        }
        Ok(())
    }

    /// Try to read available data from the PTY without blocking
    ///
    /// Returns `Ok(None)` if no data is available, `Ok(Some(data))` if data was read
    pub fn try_read(&mut self) -> Result<Option<Vec<u8>>> {
        let mut buf = [0u8; 4096];

        match self.reader.read(&mut buf) {
            Ok(0) => Ok(None), // EOF
            Ok(n) => Ok(Some(buf[..n].to_vec())),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e).context("Failed to read from PTY"),
        }
    }

    /// Resize the PTY
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to resize PTY")
    }

    /// Check if the child process is still running
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Check if the child process has exited and return exit info
    ///
    /// Returns:
    /// - `None` if the process is still running
    /// - `Some(ExitInfo)` if the process has exited, containing:
    ///   - code: The exit code (or signal number + 128 for signal termination)
    ///   - success: Whether it was a successful exit (code 0)
    ///   - signal: The signal number if killed by a signal (Unix only)
    pub fn exit_status(&mut self) -> Option<ExitInfo> {
        match self.child.try_wait() {
            Ok(None) => None, // Still running
            Ok(Some(status)) => {
                let code = status.exit_code() as i32;
                let success = status.success();

                // On Unix, detect signal termination
                // Signals typically result in exit code = 128 + signal_number
                #[cfg(unix)]
                let signal = if !success && code > 128 && code <= 128 + 64 {
                    Some(code - 128)
                } else {
                    None
                };

                #[cfg(not(unix))]
                let signal = None;

                Some(ExitInfo {
                    code,
                    success,
                    signal,
                })
            }
            Err(_) => {
                // Error checking status - treat as exited abnormally
                Some(ExitInfo {
                    code: 255,
                    success: false,
                    signal: None,
                })
            }
        }
    }

    /// Kill the child process
    pub fn kill(&mut self) -> Result<()> {
        self.child
            .kill()
            .context("Failed to kill PTY child process")
    }
}

/// Convert a crossterm KeyEvent to terminal escape sequence bytes
fn key_event_to_bytes(key: KeyEvent) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        // Basic keys
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => vec![0x1b, b'[', b'Z'], // Shift+Tab (CSI Z)
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],

        // Arrow keys
        KeyCode::Up => vec![0x1b, b'[', b'A'],
        KeyCode::Down => vec![0x1b, b'[', b'B'],
        KeyCode::Right => vec![0x1b, b'[', b'C'],
        KeyCode::Left => vec![0x1b, b'[', b'D'],

        // Navigation keys
        KeyCode::Home => vec![0x1b, b'[', b'H'],
        KeyCode::End => vec![0x1b, b'[', b'F'],
        KeyCode::PageUp => vec![0x1b, b'[', b'5', b'~'],
        KeyCode::PageDown => vec![0x1b, b'[', b'6', b'~'],
        KeyCode::Insert => vec![0x1b, b'[', b'2', b'~'],
        KeyCode::Delete => vec![0x1b, b'[', b'3', b'~'],

        // Function keys
        KeyCode::F(1) => vec![0x1b, b'O', b'P'],
        KeyCode::F(2) => vec![0x1b, b'O', b'Q'],
        KeyCode::F(3) => vec![0x1b, b'O', b'R'],
        KeyCode::F(4) => vec![0x1b, b'O', b'S'],
        KeyCode::F(5) => vec![0x1b, b'[', b'1', b'5', b'~'],
        KeyCode::F(6) => vec![0x1b, b'[', b'1', b'7', b'~'],
        KeyCode::F(7) => vec![0x1b, b'[', b'1', b'8', b'~'],
        KeyCode::F(8) => vec![0x1b, b'[', b'1', b'9', b'~'],
        KeyCode::F(9) => vec![0x1b, b'[', b'2', b'0', b'~'],
        KeyCode::F(10) => vec![0x1b, b'[', b'2', b'1', b'~'],
        KeyCode::F(11) => vec![0x1b, b'[', b'2', b'3', b'~'],
        KeyCode::F(12) => vec![0x1b, b'[', b'2', b'4', b'~'],
        KeyCode::F(_) => vec![],

        // Character input
        KeyCode::Char(c) => {
            if ctrl {
                // Ctrl+A through Ctrl+Z -> 0x01 through 0x1A
                if c.is_ascii_alphabetic() {
                    vec![(c.to_ascii_lowercase() as u8) - b'a' + 1]
                } else {
                    vec![]
                }
            } else if alt {
                // Alt+char -> ESC followed by char
                let mut bytes = vec![0x1b];
                let mut buf = [0u8; 4];
                bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                bytes
            } else {
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf).as_bytes().to_vec()
            }
        }

        // Null and other special
        KeyCode::Null => vec![0],
        _ => vec![],
    }
}

/// Convert a mouse event to SGR mouse escape sequence bytes
///
/// # Arguments
/// * `mouse` - The mouse event from crossterm
/// * `content_area` - The content area rect where the PTY is rendered
///
/// # Returns
/// * `Some(bytes)` - SGR mouse escape sequence to send to PTY
/// * `None` - if the mouse event is outside the content area or not relevant
///
/// SGR format: `\x1b[<button;col;row{M|m}`
/// - M = press/motion, m = release
/// - Coordinates are 1-indexed
pub fn mouse_event_to_bytes(mouse: MouseEvent, content_area: Rect) -> Option<Vec<u8>> {
    // Check if mouse is within content area
    if mouse.column < content_area.x
        || mouse.column >= content_area.x + content_area.width
        || mouse.row < content_area.y
        || mouse.row >= content_area.y + content_area.height
    {
        return None;
    }

    // Translate screen coordinates to content-relative coordinates (1-indexed for SGR)
    let col = (mouse.column - content_area.x) + 1;
    let row = (mouse.row - content_area.y) + 1;

    // Determine button code and press/release
    let (button, is_release) = match mouse.kind {
        MouseEventKind::Down(btn) => {
            let code = match btn {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
            };
            (code, false)
        }
        MouseEventKind::Up(btn) => {
            let code = match btn {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
            };
            (code, true)
        }
        MouseEventKind::Drag(btn) => {
            // Drag is button + 32
            let code = match btn {
                MouseButton::Left => 32,
                MouseButton::Middle => 33,
                MouseButton::Right => 34,
            };
            (code, false)
        }
        MouseEventKind::ScrollUp => (64, false),
        MouseEventKind::ScrollDown => (65, false),
        MouseEventKind::ScrollLeft => (66, false),
        MouseEventKind::ScrollRight => (67, false),
        MouseEventKind::Moved => {
            // Mouse motion without button - button code 35
            (35, false)
        }
    };

    // Add modifier bits to button code
    let mut final_button = button;
    if mouse.modifiers.contains(KeyModifiers::SHIFT) {
        final_button += 4;
    }
    if mouse.modifiers.contains(KeyModifiers::ALT) {
        final_button += 8;
    }
    if mouse.modifiers.contains(KeyModifiers::CONTROL) {
        final_button += 16;
    }

    // Generate SGR escape sequence
    let suffix = if is_release { 'm' } else { 'M' };
    let sequence = format!("\x1b[<{};{};{}{}", final_button, col, row, suffix);

    Some(sequence.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_event_to_bytes_enter() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(key_event_to_bytes(key), vec![b'\r']);
    }

    #[test]
    fn test_key_event_to_bytes_char() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(key_event_to_bytes(key), vec![b'a']);
    }

    #[test]
    fn test_key_event_to_bytes_ctrl_c() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(key_event_to_bytes(key), vec![0x03]);
    }

    #[test]
    fn test_key_event_to_bytes_arrow_up() {
        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(key_event_to_bytes(key), vec![0x1b, b'[', b'A']);
    }

    #[test]
    fn test_key_event_to_bytes_backtab() {
        let key = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(key_event_to_bytes(key), vec![0x1b, b'[', b'Z']);
    }

    #[test]
    fn test_key_event_to_bytes_alt_char() {
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT);
        assert_eq!(key_event_to_bytes(key), vec![0x1b, b'x']);
    }

    #[test]
    fn test_key_event_to_bytes_unicode() {
        let key = KeyEvent::new(KeyCode::Char('é'), KeyModifiers::NONE);
        let bytes = key_event_to_bytes(key);
        assert_eq!(bytes, "é".as_bytes());
    }

    #[test]
    fn test_pty_spawn_and_read() {
        // Spawn `echo hello` and verify we can read the output
        let mut pty = PtyHandle::spawn(
            "echo",
            &["hello"],
            std::path::Path::new("/tmp"),
            HashMap::new(),
            24,
            80,
        )
        .expect("Failed to spawn PTY");

        // Wait for process to complete and output to be available
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Read output
        let mut output = Vec::new();
        for _ in 0..10 {
            match pty.try_read() {
                Ok(Some(data)) => output.extend(data),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        let output_str = String::from_utf8_lossy(&output);
        assert!(
            output_str.contains("hello"),
            "Expected 'hello' in output, got: {:?}",
            output_str
        );

        // Wait a bit more and check if process exited (echo should be fast)
        std::thread::sleep(std::time::Duration::from_millis(50));
        // Note: We don't strictly assert exit since timing can be flaky in CI
    }

    // ExitInfo tests
    #[test]
    fn test_exit_info_format_reason_success() {
        let info = ExitInfo {
            code: 0,
            success: true,
            signal: None,
        };
        assert_eq!(info.format_reason(), "Exited normally");
    }

    #[test]
    fn test_exit_info_format_reason_error_code() {
        let info = ExitInfo {
            code: 1,
            success: false,
            signal: None,
        };
        assert_eq!(info.format_reason(), "Exit code: 1");
    }

    #[test]
    fn test_exit_info_format_reason_signal() {
        let info = ExitInfo {
            code: 137,
            success: false,
            signal: Some(9),
        };
        assert_eq!(info.format_reason(), "Killed by signal 9 (SIGKILL)");
    }

    #[test]
    fn test_signal_name_known_signals() {
        assert_eq!(signal_name(1), "SIGHUP");
        assert_eq!(signal_name(2), "SIGINT");
        assert_eq!(signal_name(9), "SIGKILL");
        assert_eq!(signal_name(11), "SIGSEGV");
        assert_eq!(signal_name(15), "SIGTERM");
    }

    #[test]
    fn test_signal_name_unknown_signal() {
        assert_eq!(signal_name(99), "unknown");
    }

    #[test]
    fn test_pty_write_and_read() {
        // Spawn a simple cat process
        let mut pty = PtyHandle::spawn(
            "cat",
            &[],
            std::path::Path::new("/tmp"),
            HashMap::new(),
            24,
            80,
        )
        .expect("Failed to spawn PTY");

        // Write some data
        pty.write(b"hello pty\n").expect("Failed to write");

        // Wait for echo
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Read output (cat echoes back what we write)
        let mut output = Vec::new();
        loop {
            match pty.try_read() {
                Ok(Some(data)) => output.extend(data),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        let output_str = String::from_utf8_lossy(&output);
        assert!(
            output_str.contains("hello pty"),
            "Expected 'hello pty' in output, got: {:?}",
            output_str
        );

        // Process should still be alive (cat waits for more input)
        assert!(pty.is_alive(), "Process should still be alive");

        // Clean up
        pty.kill().expect("Failed to kill");
    }

    #[test]
    fn test_pty_is_alive_and_kill() {
        let mut pty = PtyHandle::spawn(
            "sleep",
            &["10"],
            std::path::Path::new("/tmp"),
            HashMap::new(),
            24,
            80,
        )
        .expect("Failed to spawn PTY");

        // Process should be alive
        assert!(pty.is_alive(), "Process should be alive");

        // Kill it
        pty.kill().expect("Failed to kill");

        // Wait a moment for process to terminate
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Process should be dead
        assert!(!pty.is_alive(), "Process should be dead after kill");
    }
}
