//! Codex input debug harness
//!
//! Minimal standalone binary that replicates the PTY mediation path from Panoptes
//! (no TUI dashboard, no hooks, no session management) to isolate and reproduce
//! the character-dropping bug during Codex streaming output.
//!
//! Usage:
//!   codex-harness [OPTIONS] [-- CODEX_ARGS...]
//!
//! Options:
//!   --cmd <CMD>              Command to run (default: "codex")
//!   --dir <PATH>             Working directory (default: cwd)
//!   --loop-delay-ms <MS>     Extra delay per loop iteration (for timing diagnosis)
//!   --enable-mouse           Enable mouse capture (matches Panoptes EnableMouseCapture)
//!   --enable-focus           Enable focus change reporting (matches Panoptes EnableFocusChange)
//!   --env KEY=VALUE          Add env var to PTY spawn (repeatable)
//!   --row-offset N           Row deduction for PTY size (default: 2, Panoptes uses ~8)
//!   --drain-events           Drain all pending crossterm events per iteration
//!
//! Press Esc to exit. All other keys (including Ctrl+C) are forwarded to the PTY.

use std::collections::HashMap;
use std::io::{self, stdout, Write};
use std::time::Duration;

use crossterm::event::{
    self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyboardEnhancementFlags,
    MouseEvent, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use panoptes::session::{mouse_event_to_bytes, PtyHandle, VirtualTerminal};

/// Parsed CLI options for the harness.
struct HarnessOptions {
    cmd: String,
    dir: std::path::PathBuf,
    loop_delay_ms: Option<u64>,
    enable_mouse: bool,
    enable_focus: bool,
    extra_env: HashMap<String, String>,
    row_offset: u16,
    drain_events: bool,
    extra_args: Vec<String>,
}

fn main() {
    // Install panic hook that restores the terminal before printing the panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = teardown_terminal(true, true, true);
        original_hook(info);
    }));

    if let Err(e) = run() {
        let _ = teardown_terminal(false, false, false);
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

/// Parse CLI arguments.
fn parse_args() -> HarnessOptions {
    let args: Vec<String> = std::env::args().collect();
    let mut opts = HarnessOptions {
        cmd: "codex".to_string(),
        dir: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        loop_delay_ms: None,
        enable_mouse: false,
        enable_focus: false,
        extra_env: HashMap::new(),
        row_offset: 2,
        drain_events: false,
        extra_args: Vec::new(),
    };

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--cmd" => {
                i += 1;
                if i < args.len() {
                    opts.cmd = args[i].clone();
                }
            }
            "--dir" => {
                i += 1;
                if i < args.len() {
                    opts.dir = std::path::PathBuf::from(&args[i]);
                }
            }
            "--loop-delay-ms" => {
                i += 1;
                if i < args.len() {
                    opts.loop_delay_ms = Some(args[i].parse().unwrap_or_else(|_| {
                        eprintln!("Invalid value for --loop-delay-ms: {}", args[i]);
                        std::process::exit(1);
                    }));
                }
            }
            "--enable-mouse" => {
                opts.enable_mouse = true;
            }
            "--enable-focus" => {
                opts.enable_focus = true;
            }
            "--env" => {
                i += 1;
                if i < args.len() {
                    if let Some((key, value)) = args[i].split_once('=') {
                        opts.extra_env.insert(key.to_string(), value.to_string());
                    } else {
                        eprintln!("Invalid --env format, expected KEY=VALUE: {}", args[i]);
                        std::process::exit(1);
                    }
                }
            }
            "--row-offset" => {
                i += 1;
                if i < args.len() {
                    opts.row_offset = args[i].parse().unwrap_or_else(|_| {
                        eprintln!("Invalid value for --row-offset: {}", args[i]);
                        std::process::exit(1);
                    });
                }
            }
            "--drain-events" => {
                opts.drain_events = true;
            }
            "--" => {
                opts.extra_args = args[i + 1..].to_vec();
                break;
            }
            other => {
                eprintln!("Unknown option: {other}");
                eprintln!(
                    "Usage: codex-harness [--cmd CMD] [--dir PATH] [--loop-delay-ms MS] \
                     [--enable-mouse] [--enable-focus] [--env KEY=VALUE]... \
                     [--row-offset N] [--drain-events] [-- ARGS...]"
                );
                std::process::exit(1);
            }
        }
        i += 1;
    }

    opts
}

/// Teardown terminal state. Safe to call multiple times.
fn teardown_terminal(keyboard_enhancement: bool, mouse: bool, focus: bool) -> io::Result<()> {
    if keyboard_enhancement {
        let _ = stdout().execute(PopKeyboardEnhancementFlags);
        let _ = stdout().flush();
        // Drain pending terminal responses
        while event::poll(Duration::from_millis(10)).unwrap_or(false) {
            let _ = event::read();
        }
    }
    if mouse {
        let _ = stdout().execute(DisableMouseCapture);
    }
    if focus {
        let _ = stdout().execute(DisableFocusChange);
    }
    let _ = stdout().execute(DisableBracketedPaste);
    let _ = stdout().execute(LeaveAlternateScreen);
    let _ = disable_raw_mode();
    Ok(())
}

/// Forward a mouse event to the PTY if the vterm has mouse protocol enabled.
///
/// Mirrors `handle_mouse_event` in `src/app/mod.rs:673-702`.
fn forward_mouse_event(
    mouse: MouseEvent,
    pty: &mut PtyHandle,
    vterm: &VirtualTerminal,
    content_area: Rect,
) -> anyhow::Result<bool> {
    if vterm.mouse_protocol_mode() == vt100::MouseProtocolMode::None {
        return Ok(false);
    }
    if let Some(bytes) = mouse_event_to_bytes(mouse, content_area) {
        pty.write(&bytes)?;
        return Ok(true);
    }
    Ok(false)
}

/// Process a single crossterm event. Returns `true` if the harness should exit.
fn process_event(
    ev: Event,
    pty: &mut PtyHandle,
    vterm: &mut VirtualTerminal,
    needs_render: &mut bool,
    content_area: Rect,
    row_offset: u16,
) -> anyhow::Result<bool> {
    match ev {
        Event::Key(key) => {
            // Only handle Press and Repeat (skip Release events)
            if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
                return Ok(false);
            }

            // Esc exits the harness
            if key.code == KeyCode::Esc {
                return Ok(true);
            }

            // Forward all other keys to PTY
            pty.send_key(key)?;
            *needs_render = true;
        }
        Event::Paste(text) => {
            pty.write_paste(&text)?;
            *needs_render = true;
        }
        Event::Resize(w, h) => {
            let new_rows = h.saturating_sub(row_offset);
            let new_cols = w.saturating_sub(2);
            pty.resize(new_rows, new_cols)?;
            vterm.resize(new_rows as usize, new_cols as usize);
            *needs_render = true;
        }
        Event::Mouse(mouse) => {
            if forward_mouse_event(mouse, pty, vterm, content_area)? {
                *needs_render = true;
            }
        }
        _ => {}
    }
    Ok(false)
}

fn run() -> anyhow::Result<()> {
    let opts = parse_args();

    // --- Terminal setup (mirrors src/tui/mod.rs:130-163) ---
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;

    let keyboard_enhancement_enabled = supports_keyboard_enhancement().unwrap_or(false)
        && stdout()
            .execute(PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::REPORT_EVENT_TYPES,
            ))
            .is_ok();

    let _ = stdout().execute(EnableBracketedPaste);

    // Enable mouse capture if requested (matches Panoptes EnableMouseCapture)
    let mouse_enabled = opts.enable_mouse && stdout().execute(EnableMouseCapture).is_ok();

    // Enable focus change reporting if requested (matches Panoptes EnableFocusChange)
    let focus_enabled = opts.enable_focus && stdout().execute(EnableFocusChange).is_ok();

    // Create ratatui terminal
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;
    terminal.clear()?;

    // Get initial terminal size for PTY
    let size = terminal.size()?;
    let pty_rows = size.height.saturating_sub(opts.row_offset);
    let pty_cols = size.width.saturating_sub(2);

    // Spawn PTY with extra env vars
    let extra_refs: Vec<&str> = opts.extra_args.iter().map(|s| s.as_str()).collect();
    let mut pty = PtyHandle::spawn(
        &opts.cmd,
        &extra_refs,
        &opts.dir,
        opts.extra_env,
        pty_rows,
        pty_cols,
    )?;

    // Create virtual terminal (for DSR detection + rendering)
    let mut vterm = VirtualTerminal::new(pty_rows as usize, pty_cols as usize);

    // DSR rolling buffer (for detecting \x1b[6n across read boundaries)
    let mut dsr_buffer: Vec<u8> = Vec::new();

    let mut needs_render = true;
    let tick = Duration::from_millis(16);

    // Track content area for mouse forwarding
    let mut last_content_area = Rect::default();

    // Print active flags to title
    let mut flags: Vec<&str> = Vec::new();
    if mouse_enabled {
        flags.push("mouse");
    }
    if focus_enabled {
        flags.push("focus");
    }
    if opts.drain_events {
        flags.push("drain");
    }
    if opts.row_offset != 2 {
        flags.push("row-offset");
    }
    if opts.loop_delay_ms.is_some() {
        flags.push("delay");
    }
    let title = if flags.is_empty() {
        "codex-harness".to_string()
    } else {
        format!("codex-harness [{}]", flags.join("+"))
    };

    // --- Event loop ---
    loop {
        // 1. Render if needed
        if needs_render {
            let title_ref = &title;
            terminal.draw(|frame| {
                let area = frame.size();
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Green))
                    .title(title_ref.as_str());
                let inner = block.inner(area);
                last_content_area = inner;

                // Get styled lines from vterm
                let styled_lines = vterm.visible_styled_lines(inner.height as usize);
                let lines: Vec<Line<'_>> = styled_lines.iter().cloned().collect();
                let paragraph = Paragraph::new(lines);

                frame.render_widget(block, area);
                frame.render_widget(paragraph, inner);
            })?;
            needs_render = false;
        }

        // 2. Poll crossterm events
        if event::poll(tick)? {
            let should_exit = process_event(
                event::read()?,
                &mut pty,
                &mut vterm,
                &mut needs_render,
                last_content_area,
                opts.row_offset,
            )?;
            if should_exit {
                break;
            }

            // Drain all pending events if --drain-events is set (matches Panoptes behavior)
            if opts.drain_events {
                while event::poll(Duration::ZERO)? {
                    let should_exit = process_event(
                        event::read()?,
                        &mut pty,
                        &mut vterm,
                        &mut needs_render,
                        last_content_area,
                        opts.row_offset,
                    )?;
                    if should_exit {
                        break;
                    }
                }
            }
        }

        // 3. Poll PTY output
        while let Ok(Some(bytes)) = pty.try_read() {
            vterm.process(&bytes);

            // DSR detection (inline from src/session/mod.rs:407-428)
            let mut combined = Vec::with_capacity(dsr_buffer.len() + bytes.len());
            combined.extend_from_slice(&dsr_buffer);
            combined.extend_from_slice(&bytes);
            let dsr_count = combined.windows(4).filter(|w| *w == b"\x1b[6n").count();
            if dsr_count > 0 {
                let (row, col) = vterm.cursor_position();
                let response = format!("\x1b[{};{}R", row + 1, col + 1);
                for _ in 0..dsr_count {
                    if pty.write(response.as_bytes()).is_err() {
                        break;
                    }
                }
            }
            let keep_from = combined.len().saturating_sub(3);
            dsr_buffer = combined[keep_from..].to_vec();

            needs_render = true;
        }

        // 3.5. Simulate extra loop overhead (for timing diagnosis)
        if let Some(delay) = opts.loop_delay_ms {
            std::thread::sleep(Duration::from_millis(delay));
        }

        // 4. Check if process is still alive
        if !pty.is_alive() {
            // Final render to show last output
            terminal.draw(|frame| {
                let area = frame.size();
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Red))
                    .title("codex-harness [exited]");
                let inner = block.inner(area);

                let styled_lines = vterm.visible_styled_lines(inner.height as usize);
                let lines: Vec<Line<'_>> = styled_lines.iter().cloned().collect();
                let paragraph = Paragraph::new(lines);

                frame.render_widget(block, area);
                frame.render_widget(paragraph, inner);
            })?;

            // Wait for user to press a key before exiting
            loop {
                if event::poll(Duration::from_millis(100))? {
                    if let Event::Key(key) = event::read()? {
                        if key.kind == KeyEventKind::Press {
                            break;
                        }
                    }
                }
            }
            break;
        }
    }

    // --- Terminal teardown ---
    if keyboard_enhancement_enabled {
        let _ = stdout().execute(PopKeyboardEnhancementFlags);
        let _ = stdout().flush();
        while event::poll(Duration::from_millis(10)).unwrap_or(false) {
            let _ = event::read();
        }
    }
    if mouse_enabled {
        let _ = stdout().execute(DisableMouseCapture);
    }
    if focus_enabled {
        let _ = stdout().execute(DisableFocusChange);
    }
    let _ = stdout().execute(DisableBracketedPaste);
    terminal.show_cursor()?;
    stdout().execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;

    Ok(())
}
