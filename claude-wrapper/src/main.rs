//! Claude Code Wrapper - Standalone Demo
//!
//! This binary demonstrates the claude-wrapper library by running Claude Code
//! in a framed PTY with customizable header and footer.

use std::path::PathBuf;

use anyhow::Result;
use claude_wrapper::{ClaudeWrapper, FrameConfig, WrapperCallbacks, WrapperConfig};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use tracing::Level;
use tracing_subscriber::fmt::format::FmtSpan;

/// Demo callbacks for the wrapper
struct DemoCallbacks {
    session_name: String,
    exit_code: Option<u32>,
}

impl DemoCallbacks {
    fn new(session_name: impl Into<String>) -> Self {
        Self {
            session_name: session_name.into(),
            exit_code: None,
        }
    }
}

impl WrapperCallbacks for DemoCallbacks {
    fn on_esc(&mut self) -> bool {
        // Return true to exit when ESC is pressed
        true
    }

    fn render_header(&self, area: Rect, frame: &mut Frame) {
        let header = Paragraph::new(format!(
            " Session: {} | Press ESC to exit",
            self.session_name
        ))
        .style(Style::default().fg(Color::White).bg(Color::DarkGray));
        frame.render_widget(header, area);
    }

    fn render_footer(&self, area: Rect, frame: &mut Frame) {
        let status = if let Some(code) = self.exit_code {
            format!(" Process exited with code: {}", code)
        } else {
            " Process running...".to_string()
        };
        let footer =
            Paragraph::new(status).style(Style::default().fg(Color::White).bg(Color::DarkGray));
        frame.render_widget(footer, area);
    }

    fn border_color(&self) -> Color {
        if self.exit_code.is_some() {
            Color::Red
        } else {
            Color::Blue
        }
    }

    fn on_exit(&mut self, exit_code: Option<u32>) {
        self.exit_code = exit_code;
    }
}

fn main() -> Result<()> {
    // Setup logging to file
    let log_file = std::fs::File::create("/tmp/claude-wrapper.log")?;
    tracing_subscriber::fmt()
        .with_writer(log_file)
        .with_max_level(Level::DEBUG)
        .with_span_events(FmtSpan::CLOSE)
        .init();

    // Parse command line args
    let args: Vec<String> = std::env::args().collect();

    // Default to running 'claude' with no args, or use provided command
    let (command, cmd_args) = if args.len() > 1 {
        (args[1].clone(), args[2..].to_vec())
    } else {
        ("claude".to_string(), vec![])
    };

    // Get working directory
    let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

    // Create config
    let config = WrapperConfig::new(&command)
        .with_args(cmd_args)
        .with_working_dir(working_dir)
        .with_frame(FrameConfig::new(1, 1));

    // Create callbacks
    let mut callbacks = DemoCallbacks::new(&command);

    // Create and run wrapper
    let mut wrapper = ClaudeWrapper::new(config);
    wrapper.run(&mut callbacks)?;

    // Print exit code after wrapper exits
    if let Some(code) = callbacks.exit_code {
        println!("Process exited with code: {}", code);
    } else {
        println!("Wrapper exited (ESC pressed)");
    }

    Ok(())
}
