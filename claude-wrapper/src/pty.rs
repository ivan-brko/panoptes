//! PTY (Pseudo-Terminal) management for spawning and interacting with processes
//!
//! This module handles spawning processes in a PTY, reading/writing data,
//! and managing the process lifecycle.

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

/// Bracketed paste start sequence
const PASTE_START: &[u8] = b"\x1b[200~";
/// Bracketed paste end sequence
const PASTE_END: &[u8] = b"\x1b[201~";

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
        args: &[String],
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

    /// Write pasted text to the PTY, optionally wrapped in bracketed paste sequences
    pub fn write_paste(&mut self, text: &str, use_bracketed_paste: bool) -> Result<()> {
        if use_bracketed_paste {
            self.writer
                .write_all(PASTE_START)
                .context("Failed to write paste start sequence")?;
        }
        self.writer
            .write_all(text.as_bytes())
            .context("Failed to write paste content")?;
        if use_bracketed_paste {
            self.writer
                .write_all(PASTE_END)
                .context("Failed to write paste end sequence")?;
        }
        self.writer.flush().context("Failed to flush PTY writer")?;
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

    /// Get exit code if process has exited
    pub fn exit_code(&mut self) -> Option<u32> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.exit_code()),
            _ => None,
        }
    }

    /// Kill the child process
    pub fn kill(&mut self) -> Result<()> {
        self.child
            .kill()
            .context("Failed to kill PTY child process")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pty_spawn_and_read() {
        // Spawn `echo hello` and verify we can read the output
        let mut pty = PtyHandle::spawn(
            "echo",
            &["hello".to_string()],
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

        // Clean up
        pty.kill().expect("Failed to kill");
    }

    #[test]
    fn test_pty_is_alive_and_kill() {
        let mut pty = PtyHandle::spawn(
            "sleep",
            &["10".to_string()],
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
