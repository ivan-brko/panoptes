//! Statistics calculations for focus timing
//!
//! Provides types and functions for calculating and aggregating focus statistics.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::project::{BranchId, ProjectId};

/// Time spent in a specific project/branch context during a focus session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusContextBreakdown {
    /// Associated project if any
    pub project_id: Option<ProjectId>,
    /// Associated branch if any
    pub branch_id: Option<BranchId>,
    /// Duration spent in this context
    #[serde(with = "duration_serde")]
    pub duration: Duration,
}

/// A completed focus timer session record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusSession {
    /// Unique identifier for this session
    pub id: Uuid,
    /// Target duration that was set (e.g., 25 minutes)
    #[serde(with = "duration_serde")]
    pub target_duration: Duration,
    /// Time the app was actually focused
    #[serde(with = "duration_serde")]
    pub focused_duration: Duration,
    /// Total wall clock time from start to end
    #[serde(with = "duration_serde")]
    pub total_elapsed: Duration,
    /// Focus percentage (focused / target * 100)
    pub focus_percentage: f64,
    /// Associated project if any
    pub project_id: Option<ProjectId>,
    /// Associated branch if any
    pub branch_id: Option<BranchId>,
    /// When the session was completed
    pub completed_at: DateTime<Utc>,
    /// Per-project/branch time breakdown during the session
    #[serde(default)]
    pub context_breakdown: Vec<FocusContextBreakdown>,
}

impl FocusSession {
    /// Create a new focus session from timer result
    pub fn from_timer_result(
        target_duration: Duration,
        focused_duration: Duration,
        total_elapsed: Duration,
        project_id: Option<ProjectId>,
        branch_id: Option<BranchId>,
        context_breakdown: Vec<FocusContextBreakdown>,
    ) -> Self {
        let focus_percentage = if target_duration.as_secs() > 0 {
            (focused_duration.as_secs_f64() / target_duration.as_secs_f64()) * 100.0
        } else {
            0.0
        };

        Self {
            id: Uuid::new_v4(),
            target_duration,
            focused_duration,
            total_elapsed,
            focus_percentage,
            project_id,
            branch_id,
            completed_at: Utc::now(),
            context_breakdown,
        }
    }

    /// Format the focus percentage as a display string
    pub fn format_percentage(&self) -> String {
        format!("{:.0}%", self.focus_percentage)
    }

    /// Format the target duration as MM:SS or HH:MM:SS
    pub fn format_target(&self) -> String {
        format_duration(self.target_duration)
    }

    /// Format the focused duration as MM:SS or HH:MM:SS
    pub fn format_focused(&self) -> String {
        format_duration(self.focused_duration)
    }
}

/// Aggregated statistics for a project or branch
#[derive(Debug, Clone, Default)]
pub struct AggregatedStats {
    /// Number of completed sessions
    pub session_count: u32,
    /// Total target time across all sessions
    pub total_target: Duration,
    /// Total focused time across all sessions
    pub total_focused: Duration,
    /// Average focus percentage
    pub average_focus_percentage: f64,
}

impl AggregatedStats {
    /// Add a session to the aggregation
    pub fn add_session(&mut self, session: &FocusSession) {
        self.session_count += 1;
        self.total_target += session.target_duration;
        self.total_focused += session.focused_duration;

        // Recalculate average
        if self.total_target.as_secs() > 0 {
            self.average_focus_percentage =
                (self.total_focused.as_secs_f64() / self.total_target.as_secs_f64()) * 100.0;
        }
    }

    /// Format the total target as a human-readable string
    pub fn format_total_target(&self) -> String {
        format_duration(self.total_target)
    }

    /// Format the total focused as a human-readable string
    pub fn format_total_focused(&self) -> String {
        format_duration(self.total_focused)
    }

    /// Format average percentage
    pub fn format_average(&self) -> String {
        format!("{:.0}%", self.average_focus_percentage)
    }
}

/// Aggregate sessions by project
pub fn aggregate_by_project(sessions: &[FocusSession]) -> HashMap<ProjectId, AggregatedStats> {
    let mut stats: HashMap<ProjectId, AggregatedStats> = HashMap::new();

    for session in sessions {
        if let Some(project_id) = session.project_id {
            stats.entry(project_id).or_default().add_session(session);
        }
    }

    stats
}

/// Aggregate sessions by branch within a specific project
pub fn aggregate_by_branch(
    sessions: &[FocusSession],
    project_id: ProjectId,
) -> HashMap<BranchId, AggregatedStats> {
    let mut stats: HashMap<BranchId, AggregatedStats> = HashMap::new();

    for session in sessions {
        if session.project_id == Some(project_id) {
            if let Some(branch_id) = session.branch_id {
                stats.entry(branch_id).or_default().add_session(session);
            }
        }
    }

    stats
}

/// Calculate overall statistics from all sessions
pub fn calculate_overall_stats(sessions: &[FocusSession]) -> AggregatedStats {
    let mut stats = AggregatedStats::default();
    for session in sessions {
        stats.add_session(session);
    }
    stats
}

/// Format a duration as MM:SS or HH:MM:SS
pub fn format_duration(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, mins, secs)
    } else {
        format!("{:02}:{:02}", mins, secs)
    }
}

/// Custom serde module for Duration (stored as seconds)
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_secs())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_focus_session_creation() {
        let session = FocusSession::from_timer_result(
            Duration::from_secs(25 * 60), // 25 min target
            Duration::from_secs(20 * 60), // 20 min focused
            Duration::from_secs(30 * 60), // 30 min elapsed
            None,
            None,
            vec![],
        );

        assert_eq!(session.focus_percentage, 80.0);
        assert_eq!(session.format_percentage(), "80%");
    }

    #[test]
    fn test_aggregated_stats() {
        let mut stats = AggregatedStats::default();

        let session1 = FocusSession::from_timer_result(
            Duration::from_secs(25 * 60),
            Duration::from_secs(25 * 60),
            Duration::from_secs(25 * 60),
            None,
            None,
            vec![],
        );
        let session2 = FocusSession::from_timer_result(
            Duration::from_secs(25 * 60),
            Duration::from_secs(12 * 60 + 30), // 50%
            Duration::from_secs(30 * 60),
            None,
            None,
            vec![],
        );

        stats.add_session(&session1);
        stats.add_session(&session2);

        assert_eq!(stats.session_count, 2);
        assert_eq!(stats.total_target, Duration::from_secs(50 * 60));
    }

    #[test]
    fn test_aggregate_by_project() {
        let project1 = Uuid::new_v4();
        let project2 = Uuid::new_v4();

        let sessions = vec![
            FocusSession::from_timer_result(
                Duration::from_secs(25 * 60),
                Duration::from_secs(25 * 60),
                Duration::from_secs(25 * 60),
                Some(project1),
                None,
                vec![],
            ),
            FocusSession::from_timer_result(
                Duration::from_secs(25 * 60),
                Duration::from_secs(20 * 60),
                Duration::from_secs(30 * 60),
                Some(project1),
                None,
                vec![],
            ),
            FocusSession::from_timer_result(
                Duration::from_secs(15 * 60),
                Duration::from_secs(15 * 60),
                Duration::from_secs(15 * 60),
                Some(project2),
                None,
                vec![],
            ),
        ];

        let aggregated = aggregate_by_project(&sessions);

        assert_eq!(aggregated.get(&project1).unwrap().session_count, 2);
        assert_eq!(aggregated.get(&project2).unwrap().session_count, 1);
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(65)), "01:05");
        assert_eq!(format_duration(Duration::from_secs(3665)), "01:01:05");
        assert_eq!(format_duration(Duration::from_secs(0)), "00:00");
    }
}
