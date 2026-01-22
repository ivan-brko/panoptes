//! Focus timing module for tracking focused work sessions
//!
//! This module provides functionality for:
//! - Configurable focus timers (Pomodoro-style)
//! - Activity tracking based on terminal focus state
//! - Statistics per project/branch

pub mod stats;
pub mod store;
pub mod tracker;

use std::time::{Duration, Instant};

use crate::project::{BranchId, ProjectId};

/// State of a focus timer
#[derive(Debug, Clone, Default)]
pub enum TimerState {
    /// Timer is not running
    #[default]
    Stopped,
    /// Timer is actively running (wall-clock time)
    Running { started_at: Instant },
    /// Timer has completed
    Completed { elapsed: Duration },
}

/// Result from completing a focus timer
#[derive(Debug, Clone)]
pub struct FocusTimerResult {
    /// Total wall-clock time from start to completion
    pub elapsed_duration: Duration,
    /// Target duration that was set
    pub target_duration: Duration,
    /// Project context if set
    pub project_id: Option<ProjectId>,
    /// Branch context if set
    pub branch_id: Option<BranchId>,
}

/// A focus timing session
///
/// The timer runs on wall-clock time and does NOT pause when the terminal
/// loses focus. Focus tracking is handled separately by `FocusTracker`.
#[derive(Debug, Clone)]
pub struct FocusTimer {
    /// Target duration for the timer
    pub target_duration: Duration,
    /// Current state of the timer
    pub state: TimerState,
    /// Project context for statistics
    pub project_id: Option<ProjectId>,
    /// Branch context for statistics
    pub branch_id: Option<BranchId>,
}

impl FocusTimer {
    /// Create a new focus timer with the given target duration in minutes
    pub fn new(target_minutes: u64) -> Self {
        Self {
            target_duration: Duration::from_secs(target_minutes * 60),
            state: TimerState::Stopped,
            project_id: None,
            branch_id: None,
        }
    }

    /// Set the project context for this timer
    pub fn with_project(mut self, project_id: ProjectId) -> Self {
        self.project_id = Some(project_id);
        self
    }

    /// Set the branch context for this timer
    pub fn with_branch(mut self, branch_id: BranchId) -> Self {
        self.branch_id = Some(branch_id);
        self
    }

    /// Start the timer
    pub fn start(&mut self) {
        self.state = TimerState::Running {
            started_at: Instant::now(),
        };
    }

    /// Stop the timer and return the elapsed wall-clock time
    pub fn stop(&mut self) -> Option<Duration> {
        let elapsed = self.elapsed();

        self.state = TimerState::Stopped;

        if elapsed > Duration::ZERO {
            Some(elapsed)
        } else {
            None
        }
    }

    /// Get the remaining time until timer completion (wall-clock)
    pub fn remaining(&self) -> Option<Duration> {
        let elapsed = self.elapsed();
        if elapsed >= self.target_duration {
            Some(Duration::ZERO)
        } else {
            Some(self.target_duration - elapsed)
        }
    }

    /// Get the total wall-clock time elapsed since start
    pub fn elapsed(&self) -> Duration {
        match &self.state {
            TimerState::Stopped => Duration::ZERO,
            TimerState::Running { started_at } => started_at.elapsed(),
            TimerState::Completed { elapsed } => *elapsed,
        }
    }

    /// Check if the timer has reached its target duration (wall-clock)
    pub fn is_complete(&self) -> bool {
        self.elapsed() >= self.target_duration
    }

    /// Check if the timer is currently running
    pub fn is_running(&self) -> bool {
        matches!(self.state, TimerState::Running { .. })
    }

    /// Check if the timer is active (running)
    pub fn is_active(&self) -> bool {
        matches!(self.state, TimerState::Running { .. })
    }

    /// Format remaining time as MM:SS
    pub fn format_remaining(&self) -> String {
        if let Some(remaining) = self.remaining() {
            let total_secs = remaining.as_secs();
            let mins = total_secs / 60;
            let secs = total_secs % 60;
            format!("{:02}:{:02}", mins, secs)
        } else {
            "00:00".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_timer_creation() {
        let timer = FocusTimer::new(25);
        assert_eq!(timer.target_duration, Duration::from_secs(25 * 60));
        assert!(matches!(timer.state, TimerState::Stopped));
    }

    #[test]
    fn test_timer_start_stop() {
        let mut timer = FocusTimer::new(1); // 1 minute
        timer.start();
        assert!(timer.is_running());
        assert!(timer.is_active());

        // Sleep a tiny bit to accumulate some time
        sleep(Duration::from_millis(10));

        let result = timer.stop();
        assert!(result.is_some());
        assert!(!timer.is_active());
    }

    #[test]
    fn test_timer_elapsed_is_wall_clock() {
        let mut timer = FocusTimer::new(1);
        timer.start();

        // Sleep for a known duration
        sleep(Duration::from_millis(50));

        let elapsed = timer.elapsed();
        // Timer should report wall-clock time elapsed
        assert!(elapsed >= Duration::from_millis(50));
    }

    #[test]
    fn test_timer_with_context() {
        let project_id = uuid::Uuid::new_v4();
        let branch_id = uuid::Uuid::new_v4();

        let timer = FocusTimer::new(25)
            .with_project(project_id)
            .with_branch(branch_id);

        assert_eq!(timer.project_id, Some(project_id));
        assert_eq!(timer.branch_id, Some(branch_id));
    }

    #[test]
    fn test_format_remaining() {
        let timer = FocusTimer::new(25);
        assert_eq!(timer.format_remaining(), "25:00");
    }
}
