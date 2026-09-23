use anyhow::Result;

use panoptes::app::App;
use panoptes::config;
use panoptes::logging;

#[tokio::main]
async fn main() -> Result<()> {
    // Claude's status line runs this on every refresh: answer before touching
    // config, logs or anything else
    if std::env::args().nth(1).as_deref() == Some(panoptes::hooks::status_line::SUBCOMMAND) {
        panoptes::hooks::status_line::run_subcommand();
        return Ok(());
    }

    // A one-off, explicit copy of old Codex history into the shared home, run
    // outside the TUI so a large copy cannot stall it
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some(panoptes::codex_config::merge::COMMAND) {
        let account = args.next();
        let ok = panoptes::codex_config::merge::run_command(account.as_deref())?;
        std::process::exit(if ok { 0 } else { 1 });
    }

    // Ensure config directory exists (creates logs dir too)
    config::ensure_directories()?;

    // Initialize file logging BEFORE any tracing calls
    let (log_file_info, _guard) = logging::init_file_logging(config::logs_dir())?;

    // Clean up old logs (7-day retention)
    if let Ok(count) = logging::cleanup_old_logs(&config::logs_dir()) {
        if count > 0 {
            tracing::debug!("Cleaned up {} old log files", count);
        }
    }

    tracing::debug!("Logging to: {}", log_file_info.path.display());

    // Run the application
    let mut app = App::new(log_file_info).await?;
    app.run().await
}
