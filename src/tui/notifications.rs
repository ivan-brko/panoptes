//! Notification management for TUI
//!
//! Provides a system for displaying transient notifications to users.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use uuid::Uuid;

/// Types of notifications that can be displayed
#[derive(Debug, Clone)]
pub enum NotificationType {
    /// Focus timer has completed
    TimerComplete { focused: Duration, target: Duration },
    /// Informational message
    Info { message: String },
    /// Warning message
    Warning { message: String },
}

impl NotificationType {
    /// Get a title for this notification type
    pub fn title(&self) -> &str {
        match self {
            NotificationType::TimerComplete { .. } => "Timer Complete",
            NotificationType::Info { .. } => "Info",
            NotificationType::Warning { .. } => "Warning",
        }
    }

    /// Get the message content
    pub fn message(&self) -> String {
        match self {
            NotificationType::TimerComplete { focused, target } => {
                let focus_pct = if target.as_secs() > 0 {
                    (focused.as_secs_f64() / target.as_secs_f64()) * 100.0
                } else {
                    0.0
                };
                format!(
                    "In the last {}, you were focused for {} ({:.0}%)",
                    format_duration(*target),
                    format_duration(*focused),
                    focus_pct
                )
            }
            NotificationType::Info { message } => message.clone(),
            NotificationType::Warning { message } => message.clone(),
        }
    }
}

/// A notification to be displayed
#[derive(Debug, Clone)]
pub struct Notification {
    /// Unique identifier
    pub id: Uuid,
    /// Type and content of notification
    pub notification_type: NotificationType,
    /// When the notification was created
    pub created_at: Instant,
    /// How long before auto-dismiss (None = manual dismiss only)
    pub auto_dismiss: Option<Duration>,
}

impl Notification {
    /// Create a new notification
    pub fn new(notification_type: NotificationType, auto_dismiss: Option<Duration>) -> Self {
        Self {
            id: Uuid::new_v4(),
            notification_type,
            created_at: Instant::now(),
            auto_dismiss,
        }
    }

    /// Check if this notification should be dismissed
    pub fn should_dismiss(&self) -> bool {
        if let Some(duration) = self.auto_dismiss {
            self.created_at.elapsed() >= duration
        } else {
            false
        }
    }

    /// Get remaining time before auto-dismiss
    pub fn remaining_time(&self) -> Option<Duration> {
        self.auto_dismiss.map(|duration| {
            let elapsed = self.created_at.elapsed();
            if elapsed >= duration {
                Duration::ZERO
            } else {
                duration - elapsed
            }
        })
    }
}

/// Manages a queue of notifications
#[derive(Debug)]
pub struct NotificationManager {
    /// Active notifications
    notifications: VecDeque<Notification>,
    /// Maximum number of visible notifications
    max_visible: usize,
}

impl NotificationManager {
    /// Create a new notification manager
    pub fn new(max_visible: usize) -> Self {
        Self {
            notifications: VecDeque::new(),
            max_visible,
        }
    }

    /// Push a new notification
    pub fn push(&mut self, notification_type: NotificationType, auto_dismiss: Option<Duration>) {
        let notification = Notification::new(notification_type, auto_dismiss);
        self.notifications.push_back(notification);

        // Trim excess notifications (oldest first)
        while self.notifications.len() > self.max_visible * 2 {
            self.notifications.pop_front();
        }
    }

    /// Dismiss a notification by ID
    pub fn dismiss(&mut self, id: Uuid) {
        self.notifications.retain(|n| n.id != id);
    }

    /// Dismiss all notifications
    pub fn dismiss_all(&mut self) {
        self.notifications.clear();
    }

    /// Tick - remove expired notifications
    pub fn tick(&mut self) {
        self.notifications.retain(|n| !n.should_dismiss());
    }

    /// Get visible notifications (most recent up to max_visible)
    pub fn visible(&self) -> Vec<&Notification> {
        self.notifications
            .iter()
            .rev()
            .take(self.max_visible)
            .collect()
    }

    /// Check if there are any notifications
    pub fn is_empty(&self) -> bool {
        self.notifications.is_empty()
    }

    /// Get the count of notifications
    pub fn len(&self) -> usize {
        self.notifications.len()
    }

    /// Dismiss the most recent notification (for keyboard shortcut)
    pub fn dismiss_latest(&mut self) {
        self.notifications.pop_back();
    }
}

impl Default for NotificationManager {
    fn default() -> Self {
        Self::new(3) // Default to showing up to 3 notifications
    }
}

/// Format a duration as MM:SS
fn format_duration(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{:02}:{:02}", mins, secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_notification_creation() {
        let notif = Notification::new(
            NotificationType::Info {
                message: "Test".to_string(),
            },
            Some(Duration::from_secs(30)),
        );

        assert!(!notif.should_dismiss());
    }

    #[test]
    fn test_notification_auto_dismiss() {
        let notif = Notification::new(
            NotificationType::Info {
                message: "Test".to_string(),
            },
            Some(Duration::from_millis(10)),
        );

        sleep(Duration::from_millis(20));
        assert!(notif.should_dismiss());
    }

    #[test]
    fn test_notification_manager() {
        let mut manager = NotificationManager::new(3);

        manager.push(
            NotificationType::Info {
                message: "Test 1".to_string(),
            },
            None,
        );
        manager.push(
            NotificationType::Info {
                message: "Test 2".to_string(),
            },
            None,
        );

        assert_eq!(manager.len(), 2);
        assert_eq!(manager.visible().len(), 2);
    }

    #[test]
    fn test_notification_manager_dismiss() {
        let mut manager = NotificationManager::new(3);

        manager.push(
            NotificationType::Info {
                message: "Test".to_string(),
            },
            None,
        );

        let id = manager.visible()[0].id;
        manager.dismiss(id);

        assert!(manager.is_empty());
    }

    #[test]
    fn test_timer_complete_message() {
        let notif_type = NotificationType::TimerComplete {
            focused: Duration::from_secs(20 * 60),
            target: Duration::from_secs(25 * 60),
        };

        let msg = notif_type.message();
        // Format: "In the last 25:00, you were focused for 20:00 (80%)"
        assert!(msg.contains("In the last 25:00"));
        assert!(msg.contains("focused for 20:00"));
        assert!(msg.contains("80%"));
    }
}
