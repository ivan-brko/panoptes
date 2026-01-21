use std::sync::Arc;

use anyhow::Result;

use panoptes::app::App;
use panoptes::config;
use panoptes::logging::{self, LogBuffer};

#[tokio::main]
async fn main() -> Result<()> {
    // Ensure config directory exists (creates logs dir too)
    config::ensure_directories()?;

    // Create log buffer for real-time log viewing
    let log_buffer = Arc::new(LogBuffer::new(10_000, 100));

    // Initialize file logging BEFORE any tracing calls
    let (log_file_info, _guard) =
        logging::init_file_logging(config::logs_dir(), Arc::clone(&log_buffer))?;

    // Clean up old logs (7-day retention)
    if let Ok(count) = logging::cleanup_old_logs(&config::logs_dir()) {
        if count > 0 {
            tracing::info!("Cleaned up {} old log files", count);
        }
    }

    tracing::info!("Logging to: {}", log_file_info.path.display());

    // Run the application
    let mut app = App::new(log_buffer, log_file_info).await?;
    app.run().await
}
