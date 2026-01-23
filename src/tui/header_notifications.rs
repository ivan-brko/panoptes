//! Header notification management
//!
//! Provides a queue-based notification system for displaying transient messages
//! in the header area. These are distinct from overlay notifications and are
//! designed for quick, non-intrusive feedback.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Default duration for header notifications (5 seconds)
const DEFAULT_NOTIFICATION_DURATION: Duration = Duration::from_secs(5);

/// A notification to be displayed in the header
#[derive(Debug, Clone)]
pub struct HeaderNotification {
    /// The message to display
    pub message: String,
    /// When the notification was created
    pub created_at: Instant,
    /// How long the notification should be shown
    pub duration: Duration,
}

impl HeaderNotification {
    /// Create a new header notification with default duration (5 seconds)
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            created_at: Instant::now(),
            duration: DEFAULT_NOTIFICATION_DURATION,
        }
    }

    /// Create a new header notification with custom duration
    pub fn with_duration(message: impl Into<String>, duration: Duration) -> Self {
        Self {
            message: message.into(),
            created_at: Instant::now(),
            duration,
        }
    }

    /// Check if this notification has expired
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= self.duration
    }

    /// Get remaining time before expiry
    pub fn remaining(&self) -> Duration {
        let elapsed = self.created_at.elapsed();
        if elapsed >= self.duration {
            Duration::ZERO
        } else {
            self.duration - elapsed
        }
    }
}

/// Manages a FIFO queue of header notifications
///
/// Shows one notification at a time, removing expired ones automatically.
/// Also supports a persistent notification that is always displayed (e.g., for errors).
#[derive(Debug, Default)]
pub struct HeaderNotificationManager {
    /// Queue of notifications (oldest first)
    queue: VecDeque<HeaderNotification>,
    /// Maximum queue size to prevent unbounded growth
    max_queue_size: usize,
    /// Persistent notification that doesn't expire (for critical errors like server down)
    persistent: Option<String>,
}

impl HeaderNotificationManager {
    /// Create a new notification manager
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            max_queue_size: 10, // Reasonable limit
            persistent: None,
        }
    }

    /// Set a persistent notification (displayed until cleared)
    ///
    /// Use this for critical status messages like server errors.
    /// Persistent notifications take priority over transient ones.
    pub fn set_persistent(&mut self, message: String) {
        self.persistent = Some(message);
    }

    /// Clear the persistent notification
    pub fn clear_persistent(&mut self) {
        self.persistent = None;
    }

    /// Get the persistent notification, if any
    pub fn persistent(&self) -> Option<&str> {
        self.persistent.as_deref()
    }

    /// Check if there's a persistent notification
    pub fn has_persistent(&self) -> bool {
        self.persistent.is_some()
    }

    /// Push a new notification onto the queue
    pub fn push(&mut self, message: impl Into<String>) {
        let notification = HeaderNotification::new(message);
        self.queue.push_back(notification);

        // Trim excess notifications (oldest first)
        while self.queue.len() > self.max_queue_size {
            self.queue.pop_front();
        }
    }

    /// Push a notification with custom duration
    pub fn push_with_duration(&mut self, message: impl Into<String>, duration: Duration) {
        let notification = HeaderNotification::with_duration(message, duration);
        self.queue.push_back(notification);

        while self.queue.len() > self.max_queue_size {
            self.queue.pop_front();
        }
    }

    /// Tick - remove expired notifications from the front of the queue
    pub fn tick(&mut self) {
        while let Some(front) = self.queue.front() {
            if front.is_expired() {
                self.queue.pop_front();
            } else {
                break;
            }
        }
    }

    /// Get the current notification to display (oldest non-expired)
    pub fn current(&self) -> Option<&HeaderNotification> {
        self.queue.front().filter(|n| !n.is_expired())
    }

    /// Get the current notification message, if any
    pub fn current_message(&self) -> Option<&str> {
        self.current().map(|n| n.message.as_str())
    }

    /// Clear all notifications
    pub fn clear(&mut self) {
        self.queue.clear();
    }

    /// Check if there are any active notifications
    pub fn is_empty(&self) -> bool {
        self.current().is_none()
    }

    /// Get the number of queued notifications
    pub fn len(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_notification_creation() {
        let notif = HeaderNotification::new("Test message");
        assert_eq!(notif.message, "Test message");
        assert!(!notif.is_expired());
    }

    #[test]
    fn test_notification_expiry() {
        let notif = HeaderNotification::with_duration("Test", Duration::from_millis(10));
        assert!(!notif.is_expired());
        sleep(Duration::from_millis(20));
        assert!(notif.is_expired());
    }

    #[test]
    fn test_manager_push_and_current() {
        let mut manager = HeaderNotificationManager::new();
        assert!(manager.is_empty());

        manager.push("First message");
        assert!(!manager.is_empty());
        assert_eq!(manager.current_message(), Some("First message"));

        manager.push("Second message");
        // Should still show first (FIFO)
        assert_eq!(manager.current_message(), Some("First message"));
    }

    #[test]
    fn test_manager_tick() {
        let mut manager = HeaderNotificationManager::new();
        manager.push_with_duration("Short", Duration::from_millis(10));
        manager.push("Long");

        assert_eq!(manager.current_message(), Some("Short"));

        sleep(Duration::from_millis(20));
        manager.tick();

        // Should now show the second message
        assert_eq!(manager.current_message(), Some("Long"));
    }

    #[test]
    fn test_manager_clear() {
        let mut manager = HeaderNotificationManager::new();
        manager.push("Test 1");
        manager.push("Test 2");
        assert_eq!(manager.len(), 2);

        manager.clear();
        assert!(manager.is_empty());
        assert_eq!(manager.len(), 0);
    }

    #[test]
    fn test_manager_max_queue_size() {
        let mut manager = HeaderNotificationManager::new();
        for i in 0..15 {
            manager.push(format!("Message {}", i));
        }
        // Should be capped at max_queue_size (10)
        assert_eq!(manager.len(), 10);
        // Should have dropped oldest messages
        assert_eq!(manager.current_message(), Some("Message 5"));
    }
}
