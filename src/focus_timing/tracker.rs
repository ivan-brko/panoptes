//! Focus tracking for recording focus/blur intervals
//!
//! Tracks when the terminal has focus and records intervals for statistics.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use crate::project::{BranchId, ProjectId};

/// A recorded focus interval
#[derive(Debug, Clone)]
pub struct FocusInterval {
    /// When the focus period started
    pub started_at: DateTime<Utc>,
    /// When the focus period ended
    pub ended_at: DateTime<Utc>,
    /// Associated project if any
    pub project_id: Option<ProjectId>,
    /// Associated branch if any
    pub branch_id: Option<BranchId>,
}

impl FocusInterval {
    /// Get the duration of this interval
    pub fn duration(&self) -> Duration {
        let diff = self.ended_at - self.started_at;
        Duration::from_secs(diff.num_seconds().max(0) as u64)
    }
}

/// Tracks focus/blur state and accumulates intervals
#[derive(Debug)]
pub struct FocusTracker {
    /// Whether the terminal currently has focus
    is_focused: bool,
    /// When the current focus period started (if focused)
    focus_started_at: Option<Instant>,
    /// DateTime when current focus period started (for recording)
    focus_started_datetime: Option<DateTime<Utc>>,
    /// Recorded focus intervals
    intervals: VecDeque<FocusInterval>,
    /// How long to retain intervals
    retention_duration: Duration,
    /// Current project context
    current_project_id: Option<ProjectId>,
    /// Current branch context
    current_branch_id: Option<BranchId>,
}

impl FocusTracker {
    /// Create a new focus tracker with the given retention period in days
    pub fn new(retention_days: u64) -> Self {
        Self {
            is_focused: true, // Assume focused at start
            focus_started_at: Some(Instant::now()),
            focus_started_datetime: Some(Utc::now()),
            intervals: VecDeque::new(),
            retention_duration: Duration::from_secs(retention_days * 24 * 60 * 60),
            current_project_id: None,
            current_branch_id: None,
        }
    }

    /// Handle terminal gaining focus
    pub fn handle_focus_gained(&mut self) {
        if !self.is_focused {
            self.is_focused = true;
            self.focus_started_at = Some(Instant::now());
            self.focus_started_datetime = Some(Utc::now());
        }
    }

    /// Handle terminal losing focus
    pub fn handle_focus_lost(&mut self) {
        if self.is_focused {
            if let Some(started_datetime) = self.focus_started_datetime.take() {
                // Record the completed interval
                self.intervals.push_back(FocusInterval {
                    started_at: started_datetime,
                    ended_at: Utc::now(),
                    project_id: self.current_project_id,
                    branch_id: self.current_branch_id,
                });

                // Prune old intervals
                self.prune_old_intervals();
            }
            self.is_focused = false;
            self.focus_started_at = None;
        }
    }

    /// Set the current project/branch context for new intervals
    pub fn set_context(&mut self, project_id: Option<ProjectId>, branch_id: Option<BranchId>) {
        // If we're currently focused and the context is changing,
        // close out the current interval and start a new one
        if self.is_focused
            && (self.current_project_id != project_id || self.current_branch_id != branch_id)
        {
            if let Some(started_datetime) = self.focus_started_datetime.take() {
                // Record the interval with old context
                self.intervals.push_back(FocusInterval {
                    started_at: started_datetime,
                    ended_at: Utc::now(),
                    project_id: self.current_project_id,
                    branch_id: self.current_branch_id,
                });
            }
            // Start new interval with new context
            self.focus_started_at = Some(Instant::now());
            self.focus_started_datetime = Some(Utc::now());
        }

        self.current_project_id = project_id;
        self.current_branch_id = branch_id;
    }

    /// Check if terminal currently has focus
    pub fn is_focused(&self) -> bool {
        self.is_focused
    }

    /// Get the current project context
    pub fn current_project_id(&self) -> Option<ProjectId> {
        self.current_project_id
    }

    /// Get the current branch context
    pub fn current_branch_id(&self) -> Option<BranchId> {
        self.current_branch_id
    }

    /// Calculate total focused time in a time window
    pub fn focused_time_in_last(&self, window: Duration) -> Duration {
        let cutoff = Utc::now() - chrono::Duration::seconds(window.as_secs() as i64);
        let mut total = Duration::ZERO;

        for interval in &self.intervals {
            if interval.ended_at > cutoff {
                let start = if interval.started_at < cutoff {
                    cutoff
                } else {
                    interval.started_at
                };
                let diff = interval.ended_at - start;
                total += Duration::from_secs(diff.num_seconds().max(0) as u64);
            }
        }

        // Add current focus period if still focused
        if self.is_focused {
            if let Some(started_at) = self.focus_started_at {
                total += started_at.elapsed();
            }
        }

        total
    }

    /// Calculate focused time for a specific project in a time window
    pub fn focused_time_for_project(&self, id: ProjectId, window: Duration) -> Duration {
        let cutoff = Utc::now() - chrono::Duration::seconds(window.as_secs() as i64);
        let mut total = Duration::ZERO;

        for interval in &self.intervals {
            if interval.project_id == Some(id) && interval.ended_at > cutoff {
                let start = if interval.started_at < cutoff {
                    cutoff
                } else {
                    interval.started_at
                };
                let diff = interval.ended_at - start;
                total += Duration::from_secs(diff.num_seconds().max(0) as u64);
            }
        }

        // Add current focus period if still focused and same project
        if self.is_focused && self.current_project_id == Some(id) {
            if let Some(started_at) = self.focus_started_at {
                total += started_at.elapsed();
            }
        }

        total
    }

    /// Calculate focused time for a specific branch in a time window
    pub fn focused_time_for_branch(&self, id: BranchId, window: Duration) -> Duration {
        let cutoff = Utc::now() - chrono::Duration::seconds(window.as_secs() as i64);
        let mut total = Duration::ZERO;

        for interval in &self.intervals {
            if interval.branch_id == Some(id) && interval.ended_at > cutoff {
                let start = if interval.started_at < cutoff {
                    cutoff
                } else {
                    interval.started_at
                };
                let diff = interval.ended_at - start;
                total += Duration::from_secs(diff.num_seconds().max(0) as u64);
            }
        }

        // Add current focus period if still focused and same branch
        if self.is_focused && self.current_branch_id == Some(id) {
            if let Some(started_at) = self.focus_started_at {
                total += started_at.elapsed();
            }
        }

        total
    }

    /// Get all recorded intervals (for debugging/export)
    pub fn intervals(&self) -> &VecDeque<FocusInterval> {
        &self.intervals
    }

    /// Prune intervals older than retention duration
    fn prune_old_intervals(&mut self) {
        let cutoff =
            Utc::now() - chrono::Duration::seconds(self.retention_duration.as_secs() as i64);
        while let Some(front) = self.intervals.front() {
            if front.ended_at < cutoff {
                self.intervals.pop_front();
            } else {
                break;
            }
        }
    }
}

impl Default for FocusTracker {
    fn default() -> Self {
        Self::new(30) // 30 days default retention
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_tracker_creation() {
        let tracker = FocusTracker::new(7);
        assert!(tracker.is_focused());
        assert_eq!(
            tracker.retention_duration,
            Duration::from_secs(7 * 24 * 60 * 60)
        );
    }

    #[test]
    fn test_focus_gained_lost() {
        let mut tracker = FocusTracker::new(7);
        assert!(tracker.is_focused());

        tracker.handle_focus_lost();
        assert!(!tracker.is_focused());

        tracker.handle_focus_gained();
        assert!(tracker.is_focused());
    }

    #[test]
    fn test_interval_recording() {
        let mut tracker = FocusTracker::new(7);

        // Lose focus to create an interval
        sleep(Duration::from_millis(10));
        tracker.handle_focus_lost();

        assert_eq!(tracker.intervals.len(), 1);
    }

    #[test]
    fn test_context_switching() {
        let mut tracker = FocusTracker::new(7);
        let project1 = uuid::Uuid::new_v4();
        let project2 = uuid::Uuid::new_v4();

        // First context change: None -> project1 (creates interval for None context)
        tracker.set_context(Some(project1), None);
        assert_eq!(tracker.current_project_id(), Some(project1));
        // Should have 1 interval from the initial None context
        assert_eq!(tracker.intervals.len(), 1);
        assert_eq!(tracker.intervals[0].project_id, None);

        // Second context change while focused should create another interval
        sleep(Duration::from_millis(10));
        tracker.set_context(Some(project2), None);

        // Now we have 2 intervals: one for None context, one for project1
        assert_eq!(tracker.intervals.len(), 2);
        assert_eq!(tracker.intervals[1].project_id, Some(project1));
        assert_eq!(tracker.current_project_id(), Some(project2));
    }

    #[test]
    fn test_focused_time_calculation() {
        let tracker = FocusTracker::new(7);

        // Create some focus time
        sleep(Duration::from_millis(50));

        let focused = tracker.focused_time_in_last(Duration::from_secs(60));
        assert!(focused >= Duration::from_millis(50));
    }
}
