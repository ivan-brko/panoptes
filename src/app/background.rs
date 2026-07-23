//! Git work that runs off the event-loop thread
//!
//! Fetching remotes and creating or removing a worktree can take seconds. Run
//! on the event loop they freeze the whole TUI; run here they leave it live,
//! rendering an animated overlay, and - for fetches - cancellable with Esc.
//!
//! A job is a [`GitTask`] (what the worker thread does, knowing nothing about
//! the app) plus a [`JobFollowUp`] (what the app does with the result once it
//! arrives). [`App::tick_background_job`](crate::app::App::tick_background_job)
//! polls for completion each pass of the event loop.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::git::{BranchRefInfo, FetchOutcome, GitOps};
use crate::project::{Branch, ProjectId};

/// The git work itself - self-contained, so it can run on a worker thread
pub(crate) enum GitTask {
    /// Fetch every remote, then list all branch refs
    ///
    /// The fetch is best-effort: failures and cancellation both fall through
    /// to the refs already on disk.
    FetchAndListBranches {
        repo_path: PathBuf,
        default_base_branch: Option<String>,
    },
    /// Create a git worktree for a branch
    CreateWorktree {
        repo_path: PathBuf,
        branch_name: String,
        worktree_path: PathBuf,
        create_branch: bool,
        base_ref: Option<String>,
    },
    /// Remove a branch's git worktree from disk
    RemoveWorktree {
        repo_path: PathBuf,
        branch_name: String,
    },
}

/// What the app does with a finished job
pub(crate) enum JobFollowUp {
    /// Open the worktree wizard on the branches that came back
    ///
    /// The wizard's tracking flags are stamped on here: they come from the
    /// project store and the git worktree list, which the git layer knows
    /// nothing about.
    OpenWorktreeWizard {
        tracked_branches: HashSet<String>,
        git_worktree_branches: HashSet<String>,
    },
    /// Open the default-base-branch selector on the branches that came back
    OpenDefaultBaseSelector,
    /// Register the created worktree as a branch and navigate to it
    RegisterWorktree {
        project_id: ProjectId,
        branch_name: String,
        worktree_path: PathBuf,
    },
    /// Finish deleting the branch whose worktree was just removed
    FinishBranchDelete { branch: Box<Branch> },
}

/// What a finished job produced
pub(crate) enum JobOutput {
    /// Branch refs, plus the fetch error to surface (if the fetch failed)
    Branches {
        refs: Result<Vec<BranchRefInfo>>,
        fetch_error: Option<String>,
    },
    /// An operation with nothing to return but success or failure
    Completed(Result<()>),
}

/// A finished job, as it comes back over the channel
pub(crate) struct JobResult {
    pub output: JobOutput,
    pub follow_up: JobFollowUp,
}

/// A job the event loop is waiting on
pub(crate) struct BackgroundJob {
    /// Result channel; the worker sends exactly one message and exits
    rx: Receiver<JobResult>,
    /// Flipped when the user cancels, polled by the worker
    cancel: Arc<AtomicBool>,
}

impl BackgroundJob {
    /// Spawn `task` on a worker thread, to be finished by `follow_up`
    pub(crate) fn spawn(task: GitTask, follow_up: JobFollowUp) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);

        std::thread::spawn(move || {
            let output = run_task(task, &worker_cancel);
            // A send error just means the app stopped caring (quit, or the
            // job was abandoned); there is nothing to do about it.
            let _ = tx.send(JobResult { output, follow_up });
        });

        Self { rx, cancel }
    }

    /// Ask the running job to stop
    ///
    /// Only fetches honour this; other tasks run to completion (interrupting a
    /// half-created worktree would leave the repo in a worse state).
    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Take the result if the job has finished
    ///
    /// A disconnected channel means the worker died without sending (it
    /// panicked); treated as "finished with nothing", so the app is not stuck
    /// behind the overlay forever.
    pub(crate) fn poll(&self) -> JobPoll {
        match self.rx.try_recv() {
            Ok(result) => JobPoll::Finished(Some(result)),
            Err(TryRecvError::Empty) => JobPoll::Running,
            Err(TryRecvError::Disconnected) => {
                tracing::error!("Background git job ended without a result");
                JobPoll::Finished(None)
            }
        }
    }
}

/// The state of a job as of the last [`BackgroundJob::poll`]
pub(crate) enum JobPoll {
    /// Still working
    Running,
    /// Done; `None` when the worker died without producing a result
    Finished(Option<JobResult>),
}

/// Run a task to completion on the calling (worker) thread
fn run_task(task: GitTask, cancel: &AtomicBool) -> JobOutput {
    match task {
        GitTask::FetchAndListBranches {
            repo_path,
            default_base_branch,
        } => {
            let (refs, fetch_error) =
                fetch_and_list_branches(&repo_path, default_base_branch.as_deref(), cancel);
            JobOutput::Branches { refs, fetch_error }
        }
        GitTask::CreateWorktree {
            repo_path,
            branch_name,
            worktree_path,
            create_branch,
            base_ref,
        } => JobOutput::Completed((|| {
            let git = GitOps::open(&repo_path).context("Failed to open git repository")?;
            crate::git::worktree::create_worktree(
                git.repository(),
                &branch_name,
                &worktree_path,
                create_branch,
                base_ref.as_deref(),
            )
            .map(|_| ())
            .with_context(|| format!("Failed to create worktree for '{}'", branch_name))
        })()),
        GitTask::RemoveWorktree {
            repo_path,
            branch_name,
        } => JobOutput::Completed((|| {
            let git = GitOps::open(&repo_path).context("Failed to open git repository")?;
            crate::git::worktree::remove_worktree(git.repository(), &branch_name, true)
                .context("Failed to remove worktree")
        })()),
    }
}

/// Fetch remotes (best-effort) and list the repository's branch refs
///
/// A failed fetch only produces a message for the caller to show - the refs
/// already on disk still list fine offline. Cancelling is not a failure at
/// all, so it produces no message.
fn fetch_and_list_branches(
    repo_path: &std::path::Path,
    default_base_branch: Option<&str>,
    cancel: &AtomicBool,
) -> (Result<Vec<BranchRefInfo>>, Option<String>) {
    let git = match GitOps::open(repo_path).context("Failed to open git repository") {
        Ok(git) => git,
        Err(e) => return (Err(e), None),
    };

    let fetch_error = match git.fetch_all_remotes_cancellable(cancel) {
        Ok(FetchOutcome::Completed) => None,
        Ok(FetchOutcome::Cancelled) => {
            tracing::info!("Fetch cancelled; listing local refs only");
            None
        }
        Err(e) => {
            tracing::warn!("Failed to fetch remotes: {}", e);
            Some(format!("Fetch failed: {}", e))
        }
    };

    let refs = git
        .list_all_branch_refs(default_base_branch)
        .context("Failed to list branch refs");

    (refs, fetch_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawned_job_reports_failure_without_blocking() {
        let job = BackgroundJob::spawn(
            GitTask::RemoveWorktree {
                repo_path: PathBuf::from("/nonexistent/panoptes-test-repo"),
                branch_name: "feature".to_string(),
            },
            JobFollowUp::OpenDefaultBaseSelector,
        );

        // Poll until the worker reports back, as the event loop does
        let result = loop {
            match job.poll() {
                JobPoll::Running => std::thread::sleep(std::time::Duration::from_millis(5)),
                JobPoll::Finished(result) => break result,
            }
        };

        let result = result.expect("worker should report a result");
        match result.output {
            JobOutput::Completed(outcome) => assert!(outcome.is_err()),
            _ => panic!("expected a Completed output"),
        }
    }

    #[test]
    fn cancelling_a_missing_repo_still_finishes() {
        let job = BackgroundJob::spawn(
            GitTask::FetchAndListBranches {
                repo_path: PathBuf::from("/nonexistent/panoptes-test-repo"),
                default_base_branch: None,
            },
            JobFollowUp::OpenDefaultBaseSelector,
        );
        job.cancel();

        let result = loop {
            match job.poll() {
                JobPoll::Running => std::thread::sleep(std::time::Duration::from_millis(5)),
                JobPoll::Finished(result) => break result,
            }
        };

        match result.expect("worker should report a result").output {
            JobOutput::Branches { refs, fetch_error } => {
                assert!(refs.is_err(), "opening a missing repo should fail");
                assert!(fetch_error.is_none());
            }
            _ => panic!("expected a Branches output"),
        }
    }
}
