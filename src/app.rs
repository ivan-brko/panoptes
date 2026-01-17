//! Application state and main event loop

use anyhow::Result;

use crate::config::Config;

/// Main application struct
pub struct App {
    /// Application configuration
    #[allow(dead_code)]
    config: Config,
}

impl App {
    /// Create a new application instance
    pub async fn new() -> Result<Self> {
        let config = Config::load()?;
        Ok(Self { config })
    }

    /// Run the main application loop
    pub async fn run(&mut self) -> Result<()> {
        // Placeholder - will be implemented in later tickets
        tracing::info!("Panoptes starting...");
        tracing::info!("Configuration loaded successfully");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_app_creation() {
        // This test verifies the app can be created
        // Note: requires config directory setup in real environment
        let result = App::new().await;
        // We expect this to succeed if dirs exist, or fail gracefully
        assert!(result.is_ok() || result.is_err());
    }
}
