//! Hook server module
//!
//! HTTP server that receives Claude Code hook callbacks and forwards them
//! through a channel to the main application.

use anyhow::Result;
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use std::net::SocketAddr;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info};

use super::HookEvent;

/// Sender half of the hook event channel
pub type HookEventSender = mpsc::Sender<HookEvent>;

/// Receiver half of the hook event channel
pub type HookEventReceiver = mpsc::Receiver<HookEvent>;

/// Handle to control the running server
pub struct ServerHandle {
    shutdown_tx: Option<oneshot::Sender<()>>,
    addr: SocketAddr,
}

impl ServerHandle {
    /// Get the address the server is listening on
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Shutdown the server gracefully
    pub fn shutdown(mut self) -> Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            // Ignore error if receiver is already dropped
            let _ = tx.send(());
        }
        Ok(())
    }
}

/// Create a bounded channel for hook events
///
/// # Arguments
/// * `buffer` - Maximum number of events to buffer before backpressure
///
/// # Returns
/// A tuple of (sender, receiver) for hook events
pub fn create_channel(buffer: usize) -> (HookEventSender, HookEventReceiver) {
    mpsc::channel(buffer)
}

/// Start the hook server
///
/// # Arguments
/// * `port` - Port to listen on
/// * `sender` - Channel sender for forwarding events
///
/// # Returns
/// A `ServerHandle` that can be used to shut down the server
pub async fn start(port: u16, sender: HookEventSender) -> Result<ServerHandle> {
    let app = Router::new()
        .route("/hook", post(hook_handler))
        .with_state(sender);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;

    info!("Hook server listening on {}", bound_addr);

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                shutdown_rx.await.ok();
                info!("Hook server shutting down");
            })
            .await
            .ok();
    });

    Ok(ServerHandle {
        shutdown_tx: Some(shutdown_tx),
        addr: bound_addr,
    })
}

/// POST /hook handler
///
/// Receives hook events from Claude Code and forwards them to the event channel.
async fn hook_handler(
    State(sender): State<HookEventSender>,
    Json(event): Json<HookEvent>,
) -> StatusCode {
    debug!(
        session_id = %event.session_id,
        event = %event.event,
        tool = ?event.tool,
        "Received hook event"
    );

    match sender.try_send(event) {
        Ok(()) => StatusCode::OK,
        Err(mpsc::error::TrySendError::Full(_)) => {
            error!("Hook event channel full, dropping event");
            StatusCode::OK // Still return OK to not block Claude Code
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            error!("Hook event channel closed");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    fn create_test_event() -> HookEvent {
        HookEvent {
            session_id: "test-session".to_string(),
            event: "PreToolUse".to_string(),
            tool: Some("Bash".to_string()),
            timestamp: 1704067200,
        }
    }

    #[test]
    fn test_create_channel() {
        let (tx, mut rx) = create_channel(10);

        // Should be able to send
        tx.try_send(create_test_event()).unwrap();

        // Should be able to receive
        let event = rx.try_recv().unwrap();
        assert_eq!(event.session_id, "test-session");
    }

    #[tokio::test]
    async fn test_hook_handler_valid_event() {
        let (sender, _receiver) = create_channel(10);

        let app = Router::new()
            .route("/hook", post(hook_handler))
            .with_state(sender);

        let request = Request::builder()
            .method("POST")
            .uri("/hook")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"session_id":"test","event":"PreToolUse","tool":"Bash","timestamp":123}"#,
            ))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_hook_handler_invalid_json() {
        let (sender, _receiver) = create_channel(10);

        let app = Router::new()
            .route("/hook", post(hook_handler))
            .with_state(sender);

        let request = Request::builder()
            .method("POST")
            .uri("/hook")
            .header("content-type", "application/json")
            .body(Body::from("not valid json"))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        // Axum returns 400 Bad Request for JSON syntax errors
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_hook_handler_missing_fields() {
        let (sender, _receiver) = create_channel(10);

        let app = Router::new()
            .route("/hook", post(hook_handler))
            .with_state(sender);

        // Missing required field 'timestamp'
        let request = Request::builder()
            .method("POST")
            .uri("/hook")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"session_id":"test","event":"Stop"}"#))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn test_server_starts_on_port() {
        let (sender, _receiver) = create_channel(10);

        // Use port 0 to let OS assign an available port
        let handle = start(0, sender).await.unwrap();

        assert!(handle.addr().port() > 0);
        handle.shutdown().unwrap();
    }

    #[tokio::test]
    async fn test_event_sent_through_channel() {
        let (sender, mut receiver) = create_channel(10);

        let app = Router::new()
            .route("/hook", post(hook_handler))
            .with_state(sender);

        let request = Request::builder()
            .method("POST")
            .uri("/hook")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"session_id":"my-session","event":"Stop","timestamp":999}"#,
            ))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Event should be in the channel
        let event = receiver.try_recv().unwrap();
        assert_eq!(event.session_id, "my-session");
        assert_eq!(event.event, "Stop");
        assert_eq!(event.timestamp, 999);
    }

    #[tokio::test]
    async fn test_server_shutdown() {
        let (sender, _receiver) = create_channel(10);

        let handle = start(0, sender).await.unwrap();
        let addr = handle.addr();

        // Server should be running
        assert!(tokio::net::TcpStream::connect(addr).await.is_ok());

        // Shutdown
        handle.shutdown().unwrap();

        // Give server time to shut down
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Server should be stopped (connection refused)
        assert!(tokio::net::TcpStream::connect(addr).await.is_err());
    }
}
