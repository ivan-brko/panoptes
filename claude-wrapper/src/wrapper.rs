//! Main wrapper module
//!
//! This module contains the ClaudeWrapper struct that orchestrates PTY spawning,
//! terminal emulation, input handling, and rendering.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{stdout, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;

// Debug logging to file
static DEBUG_LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

fn init_debug_log() {
    let mut log = DEBUG_LOG.lock().unwrap();
    if log.is_none() {
        if let Ok(file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open("/Users/ivan/Projects/panoptes/wrapper-debug.log")
        {
            *log = Some(file);
        }
    }
}

fn log_debug(msg: &str) {
    if let Ok(mut log) = DEBUG_LOG.lock() {
        if let Some(ref mut file) = *log {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let _ = writeln!(file, "[{}] {}", timestamp, msg);
            let _ = file.flush();
        }
    }
}
use crossterm::{
    cursor,
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEventKind, MouseEventKind,
    },
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::*;
use tracing::debug;

use crate::frame::{render_frame_border, render_pty_content, FrameConfig, FrameLayout};
use crate::input::{mouse_event_to_bytes, process_key, KeyAction};
use crate::pty::PtyHandle;
use crate::vterm::VirtualTerminal;

/// Configuration for the wrapper
#[derive(Debug, Clone)]
pub struct WrapperConfig {
    /// Command to spawn (e.g., "claude")
    pub command: String,
    /// Arguments to pass to the command
    pub args: Vec<String>,
    /// Working directory for the process
    pub working_dir: Option<PathBuf>,
    /// Additional environment variables
    pub env: HashMap<String, String>,
    /// Frame configuration
    pub frame: FrameConfig,
}

impl Default for WrapperConfig {
    fn default() -> Self {
        Self {
            command: "claude".to_string(),
            args: vec![],
            working_dir: None,
            env: HashMap::new(),
            frame: FrameConfig::default(),
        }
    }
}

impl WrapperConfig {
    /// Create a new config with the specified command
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            ..Default::default()
        }
    }

    /// Set command arguments
    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    /// Set working directory
    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.working_dir = Some(dir);
        self
    }

    /// Add an environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Set frame configuration
    pub fn with_frame(mut self, frame: FrameConfig) -> Self {
        self.frame = frame;
        self
    }
}

/// Callbacks for wrapper events
///
/// Implement this trait to customize the wrapper's behavior.
pub trait WrapperCallbacks {
    /// Called when ESC is pressed without modifiers
    ///
    /// Return `true` to exit the wrapper, `false` to continue.
    fn on_esc(&mut self) -> bool;

    /// Render custom content in the header area
    fn render_header(&self, area: Rect, frame: &mut Frame);

    /// Render custom content in the footer area
    fn render_footer(&self, area: Rect, frame: &mut Frame);

    /// Get the current border color
    fn border_color(&self) -> Color;

    /// Called when the wrapped process exits
    fn on_exit(&mut self, exit_code: Option<u32>);
}

/// Main wrapper struct
pub struct ClaudeWrapper {
    config: WrapperConfig,
    pty: Option<PtyHandle>,
    vterm: Option<VirtualTerminal>,
    /// Timestamp of last resize event for debouncing
    last_resize: Option<Instant>,
    /// Whether a render is needed
    needs_render: bool,
    /// Whether the wrapper should exit
    should_exit: bool,
    /// Scrollback offset (0 = live view, >0 = scrolled back by N rows)
    scroll_offset: usize,
}

impl ClaudeWrapper {
    /// Create a new wrapper with the given configuration
    pub fn new(config: WrapperConfig) -> Self {
        Self {
            config,
            pty: None,
            vterm: None,
            last_resize: None,
            needs_render: true,
            should_exit: false,
            scroll_offset: 0,
        }
    }

    /// Run the wrapper with the provided callbacks
    ///
    /// This enters raw mode, spawns the PTY, and runs the main event loop.
    /// Returns when the process exits or the user triggers exit via ESC.
    pub fn run<C: WrapperCallbacks>(&mut self, callbacks: &mut C) -> Result<()> {
        init_debug_log();
        log_debug("=== WRAPPER STARTING ===");

        // Setup terminal
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        stdout().execute(EnableBracketedPaste)?;
        stdout().execute(EnableMouseCapture)?;
        stdout().execute(cursor::Hide)?;

        let backend = CrosstermBackend::new(stdout());
        let mut terminal = Terminal::new(backend)?;

        // Get initial size and spawn PTY
        let size = terminal.size()?;
        let layout = FrameLayout::calculate(size, &self.config.frame);
        let (rows, cols) = layout.pty_size();

        let working_dir = self
            .config
            .working_dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")));

        // Force Claude Code to use consistent header mode (prevents flickering on resize)
        let mut env = self.config.env.clone();
        env.insert("CLAUDE_CODE_FORCE_FULL_LOGO".to_string(), "1".to_string());

        self.pty = Some(PtyHandle::spawn(
            &self.config.command,
            &self.config.args,
            &working_dir,
            env,
            rows,
            cols,
        )?);

        self.vterm = Some(VirtualTerminal::new(rows, cols));
        log_debug(&format!(
            "Initial PTY size: {}x{}, vterm created",
            cols, rows
        ));

        // Run event loop
        let result = self.event_loop(&mut terminal, callbacks);

        // Cleanup
        self.cleanup()?;

        result
    }

    /// Main event loop
    fn event_loop<C: WrapperCallbacks>(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
        callbacks: &mut C,
    ) -> Result<()> {
        const POLL_TIMEOUT: Duration = Duration::from_millis(16); // ~60fps
        const RESIZE_DEBOUNCE: Duration = Duration::from_millis(50);

        loop {
            // Check if process has exited
            if let Some(ref mut pty) = self.pty {
                if !pty.is_alive() {
                    let exit_code = pty.exit_code();
                    callbacks.on_exit(exit_code);
                    break;
                }
            }

            // Handle pending resize
            if let Some(last_resize) = self.last_resize {
                if last_resize.elapsed() >= RESIZE_DEBOUNCE {
                    log_debug(&format!(
                        "Resize debounce elapsed, scroll_offset={}, applying resize",
                        self.scroll_offset
                    ));
                    self.apply_resize(terminal)?;
                    self.last_resize = None;
                    self.needs_render = true;
                }
            }

            // Read PTY output
            if let Some(ref mut pty) = self.pty {
                while let Some(data) = pty.try_read()? {
                    log_debug(&format!("PTY READ: got {} bytes", data.len()));
                    if let Some(ref mut vterm) = self.vterm {
                        log_debug("PTY READ: calling vterm.process()");
                        vterm.process(&data);
                        log_debug("PTY READ: vterm.process() complete");
                        self.needs_render = true;
                    }
                }
            }

            // Get viewport height for scroll calculations
            let viewport_height = terminal
                .size()
                .map(|s| {
                    let layout = FrameLayout::calculate(s, &self.config.frame);
                    layout.content.height as usize
                })
                .unwrap_or(24);

            // Poll for events
            if event::poll(POLL_TIMEOUT)? {
                match event::read()? {
                    Event::Key(key) => {
                        // Only handle key press events (not release/repeat)
                        if key.kind != KeyEventKind::Press {
                            continue;
                        }

                        // Intercept Page Up/Down for scrollback navigation
                        match key.code {
                            KeyCode::PageUp => {
                                // Scroll up (increase offset) by viewport height
                                let max_scroll =
                                    self.vterm.as_ref().map(|v| v.max_scrollback()).unwrap_or(0);
                                let old_offset = self.scroll_offset;
                                self.scroll_offset = self
                                    .scroll_offset
                                    .saturating_add(viewport_height)
                                    .min(max_scroll);
                                log_debug(&format!(
                                    "PageUp: scroll_offset {} -> {}, viewport_height={}, max_scroll={}",
                                    old_offset, self.scroll_offset, viewport_height, max_scroll
                                ));
                                if let Some(ref mut vterm) = self.vterm {
                                    log_debug("PageUp: calling vterm.set_scrollback()");
                                    vterm.set_scrollback(self.scroll_offset);
                                    log_debug("PageUp: vterm.set_scrollback() done");
                                }
                                self.needs_render = true;
                                continue;
                            }
                            KeyCode::PageDown => {
                                // Scroll down (decrease offset) by viewport height
                                let old_offset = self.scroll_offset;
                                self.scroll_offset =
                                    self.scroll_offset.saturating_sub(viewport_height);
                                log_debug(&format!(
                                    "PageDown: scroll_offset {} -> {}, viewport_height={}",
                                    old_offset, self.scroll_offset, viewport_height
                                ));
                                if let Some(ref mut vterm) = self.vterm {
                                    log_debug("PageDown: calling vterm.set_scrollback()");
                                    vterm.set_scrollback(self.scroll_offset);
                                    log_debug("PageDown: vterm.set_scrollback() done");
                                }
                                self.needs_render = true;
                                continue;
                            }
                            _ => {}
                        }

                        match process_key(key) {
                            KeyAction::Exit => {
                                if callbacks.on_esc() {
                                    self.should_exit = true;
                                    break;
                                }
                            }
                            KeyAction::Forward(bytes) => {
                                // Reset scroll to live view when user types
                                if self.scroll_offset > 0 {
                                    self.scroll_offset = 0;
                                    if let Some(ref mut vterm) = self.vterm {
                                        vterm.set_scrollback(0);
                                    }
                                }
                                if let Some(ref mut pty) = self.pty {
                                    pty.write(&bytes)?;
                                    self.needs_render = true;
                                }
                            }
                            KeyAction::Ignore => {}
                        }
                    }
                    Event::Paste(text) => {
                        log_debug(&format!(
                            "PASTE EVENT: received text_len={}, lines={}",
                            text.len(),
                            text.lines().count()
                        ));

                        // Reset scroll to live view when pasting
                        if self.scroll_offset > 0 {
                            log_debug("PASTE: resetting scroll offset to 0");
                            self.scroll_offset = 0;
                            if let Some(ref mut vterm) = self.vterm {
                                vterm.set_scrollback(0);
                            }
                        }

                        if let Some(ref mut pty) = self.pty {
                            let use_bracketed = self
                                .vterm
                                .as_ref()
                                .is_some_and(|v| v.bracketed_paste_enabled());
                            log_debug(&format!(
                                "PASTE: calling pty.write_paste, use_bracketed={}",
                                use_bracketed
                            ));

                            match pty.write_paste(&text, use_bracketed) {
                                Ok(()) => log_debug("PASTE: write_paste succeeded"),
                                Err(ref e) => {
                                    log_debug(&format!("PASTE: write_paste FAILED: {}", e));
                                    return Err(anyhow::anyhow!("Paste failed: {}", e));
                                }
                            }

                            log_debug("PASTE: marking needs_render=true");
                            self.needs_render = true;
                        }
                        log_debug("PASTE EVENT: handling complete");
                    }
                    Event::Resize(w, h) => {
                        // Debounce resize events
                        log_debug(&format!(
                            "Resize event received: {}x{}, scroll_offset={}",
                            w, h, self.scroll_offset
                        ));
                        self.last_resize = Some(Instant::now());
                    }
                    Event::FocusGained | Event::FocusLost => {
                        // Ignore focus events
                    }
                    Event::Mouse(mouse) => {
                        // Check if the application has enabled mouse mode
                        let mouse_enabled = self.vterm.as_ref().is_some_and(|v| {
                            v.mouse_protocol_mode() != vt100::MouseProtocolMode::None
                        });

                        if mouse_enabled {
                            // Forward mouse events to PTY when mouse mode is enabled
                            let size = terminal.size().unwrap_or_default();
                            let layout = FrameLayout::calculate(size, &self.config.frame);
                            if let Some(bytes) = mouse_event_to_bytes(mouse, layout.content) {
                                if let Some(ref mut pty) = self.pty {
                                    let _ = pty.write(&bytes);
                                    self.needs_render = true;
                                }
                            }
                        } else {
                            // Handle scroll wheel for our own scrollback when mouse mode is disabled
                            match mouse.kind {
                                MouseEventKind::ScrollUp => {
                                    let max_scroll = self
                                        .vterm
                                        .as_ref()
                                        .map(|v| v.max_scrollback())
                                        .unwrap_or(0);
                                    let old_offset = self.scroll_offset;
                                    self.scroll_offset =
                                        self.scroll_offset.saturating_add(3).min(max_scroll);
                                    log_debug(&format!(
                                        "ScrollUp: scroll_offset {} -> {}",
                                        old_offset, self.scroll_offset
                                    ));
                                    if let Some(ref mut vterm) = self.vterm {
                                        vterm.set_scrollback(self.scroll_offset);
                                    }
                                    self.needs_render = true;
                                }
                                MouseEventKind::ScrollDown => {
                                    let old_offset = self.scroll_offset;
                                    self.scroll_offset = self.scroll_offset.saturating_sub(3);
                                    log_debug(&format!(
                                        "ScrollDown: scroll_offset {} -> {}",
                                        old_offset, self.scroll_offset
                                    ));
                                    if let Some(ref mut vterm) = self.vterm {
                                        vterm.set_scrollback(self.scroll_offset);
                                    }
                                    self.needs_render = true;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }

            // Render if needed
            if self.needs_render {
                self.render(terminal, callbacks)?;
                self.needs_render = false;
            }

            if self.should_exit {
                break;
            }
        }

        Ok(())
    }

    /// Apply a pending resize
    fn apply_resize(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        let size = terminal.size()?;
        log_debug(&format!(
            "apply_resize: terminal size = {}x{}, scroll_offset={}",
            size.width, size.height, self.scroll_offset
        ));

        let layout = FrameLayout::calculate(size, &self.config.frame);
        let (rows, cols) = layout.pty_size();

        log_debug(&format!(
            "apply_resize: PTY size will be {}x{}, content area = {:?}",
            cols, rows, layout.content
        ));

        debug!("Resizing to {}x{}", cols, rows);

        // Resize PTY
        if let Some(ref pty) = self.pty {
            log_debug("apply_resize: resizing PTY");
            pty.resize(rows, cols)?;
            log_debug("apply_resize: PTY resize done");
        }

        // Resize vterm
        if let Some(ref mut vterm) = self.vterm {
            log_debug(&format!(
                "apply_resize: resizing vterm, current scrollback={}",
                vterm.scrollback()
            ));
            vterm.resize(rows, cols);
            log_debug(&format!(
                "apply_resize: vterm resize done, scrollback now={}",
                vterm.scrollback()
            ));
        }

        log_debug("apply_resize: complete");
        Ok(())
    }

    /// Render the current state
    fn render<C: WrapperCallbacks>(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
        callbacks: &C,
    ) -> Result<()> {
        let scroll_offset = self.scroll_offset;
        log_debug(&format!("render: starting, scroll_offset={}", scroll_offset));

        terminal.draw(|frame| {
            let size = frame.size();
            log_debug(&format!(
                "render: frame size = {}x{}",
                size.width, size.height
            ));

            let layout = FrameLayout::calculate(size, &self.config.frame);
            log_debug(&format!(
                "render: layout content = {:?}, content height={}",
                layout.content, layout.content.height
            ));

            // Render header
            if layout.header.height > 0 {
                callbacks.render_header(layout.header, frame);
            }

            // Build frame title with scroll indicator
            let title = if scroll_offset > 0 {
                match &self.config.frame.title {
                    Some(t) => Some(format!("{} [SCROLLED +{}]", t, scroll_offset)),
                    None => Some(format!("[SCROLLED +{}]", scroll_offset)),
                }
            } else {
                self.config.frame.title.clone()
            };

            // Render frame border
            render_frame_border(
                frame,
                layout.frame,
                callbacks.border_color(),
                title.as_deref(),
            );

            // Render PTY content
            if let Some(ref vterm) = self.vterm {
                log_debug(&format!(
                    "render: vterm size={:?}, vterm scrollback={}, requesting {} lines",
                    vterm.size(),
                    vterm.scrollback(),
                    layout.content.height
                ));

                let lines = vterm.visible_styled_lines(layout.content.height);
                log_debug(&format!("render: got {} lines from vterm", lines.len()));

                let cursor_pos = vterm.cursor_position();
                // Hide cursor when scrolled back (showing historical content)
                let cursor_visible = vterm.cursor_visible() && scroll_offset == 0;

                render_pty_content(
                    frame,
                    layout.content,
                    &lines,
                    Some(cursor_pos),
                    cursor_visible,
                );
                log_debug("render: PTY content rendered");
            }

            // Render footer
            if layout.footer.height > 0 {
                callbacks.render_footer(layout.footer, frame);
            }
        })?;

        log_debug("render: complete");
        Ok(())
    }

    /// Cleanup terminal state
    fn cleanup(&mut self) -> Result<()> {
        // Kill PTY process if still running
        if let Some(ref mut pty) = self.pty {
            if pty.is_alive() {
                let _ = pty.kill();
            }
        }

        // Drain any pending events
        while event::poll(Duration::from_millis(10))? {
            let _ = event::read();
        }

        // Restore terminal
        stdout().execute(cursor::Show)?;
        stdout().execute(DisableMouseCapture)?;
        stdout().execute(DisableBracketedPaste)?;
        stdout().execute(LeaveAlternateScreen)?;
        disable_raw_mode()?;

        Ok(())
    }
}

impl Drop for ClaudeWrapper {
    fn drop(&mut self) {
        // Ensure cleanup happens even if run() wasn't called or panicked
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrapper_config_builder() {
        let config = WrapperConfig::new("test-cmd")
            .with_args(vec!["arg1".to_string(), "arg2".to_string()])
            .with_working_dir(PathBuf::from("/tmp"))
            .with_env("FOO", "bar");

        assert_eq!(config.command, "test-cmd");
        assert_eq!(config.args, vec!["arg1", "arg2"]);
        assert_eq!(config.working_dir, Some(PathBuf::from("/tmp")));
        assert_eq!(config.env.get("FOO"), Some(&"bar".to_string()));
    }
}
