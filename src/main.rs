use anyhow::Result;

use panoptes::app::App;
use panoptes::config;
use panoptes::logging;

#[tokio::main]
async fn main() -> Result<()> {
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
