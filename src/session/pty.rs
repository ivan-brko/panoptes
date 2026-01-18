//! PTY (Pseudo-Terminal) management for spawning and interacting with processes

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

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

    /// Write raw bytes to the PTY
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.writer
            .write_all(data)
            .context("Failed to write to PTY")?;
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
