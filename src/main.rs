use anyhow::Result;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use panoptes::app::App;
use panoptes::config;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "panoptes=info".into()),
        )
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();

    // Ensure config directory exists
    config::ensure_directories()?;

    // Run the application
    let mut app = App::new().await?;
    app.run().await
}
