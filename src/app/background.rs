//! Slow work that runs off the event-loop thread
//!
//! Fetching remotes and creating or removing a worktree can take seconds, and
//! so can searching the agents' transcript directories for conversations to
//! import. Run on the event loop they freeze the whole TUI; run here they
//! leave it live, rendering an animated overlay, and - for fetches and
//! conversation scans - cancellable with Esc.
//!
//! A job is a [`GitTask`] (what the worker thread does, knowing nothing about
//! the app) plus a [`JobFollowUp`] (what the app does with the result once it
//! arrives). [`App::tick_background_job`](crate::app::App::tick_background_job)
//! polls for completion each pass of the event loop.
//!
//! Codex account probes ([`CodexProbes`]) also run here, but are not jobs:
//! they are not modal, several run at once, and nothing waits on them.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::codex_config::probe::{ProbeBudget, ProbeRequest, ProbeResult};
use crate::git::{BranchRefInfo, FetchOutcome, GitOps};
use crate::project::{Branch, BranchId, ProjectId};
use crate::transcript::scan::{ScanOutcome, ScanRequest};

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
    /// Search the agents' transcripts for conversations to import
    ///
    /// Not git work, but the same shape of problem: bounded, yet slow enough
    /// on a cold disk to be felt, and something a user may want to abandon.
    ScanConversations { request: Box<ScanRequest> },
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
    /// Offer the conversations found for this branch
    OpenConversationImport {
        project_id: ProjectId,
        branch_id: BranchId,
    },
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
    /// Conversations found on disk
    Conversations(ScanOutcome),
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
    /// Fetches and conversation scans honour this; other tasks run to
    /// completion (interrupting a half-created worktree would leave the repo
    /// in a worse state).
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
                tracing::error!("Background job ended without a result");
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
        GitTask::ScanConversations { request } => {
            JobOutput::Conversations(crate::transcript::scan::scan(&request, cancel))
        }
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

/// How soon an account is probed again just because its list was reopened
pub(crate) const PROBE_THROTTLE: Duration = Duration::from_secs(60);

/// How long `auth.json` must sit still after changing before it is probed
///
/// `codex login` writes the file more than once; probing on the first write
/// could catch it half-done.
pub(crate) const AUTH_DEBOUNCE: Duration = Duration::from_millis(250);

/// How often the accounts' `auth.json` files are looked at while on screen
const AUTH_POLL: Duration = Duration::from_millis(100);

/// What is remembered about one account between probes
#[derive(Debug, Clone)]
struct ProbeHistory {
    /// When its last probe started
    started: Instant,
    /// `auth.json`'s modification time as that probe saw it
    auth: Option<SystemTime>,
    /// A newer modification time, and when it was first seen
    pending_auth: Option<(Option<SystemTime>, Instant)>,
}

/// Codex account probes, running beside the event loop
///
/// Unlike a [`BackgroundJob`] these are not modal - no overlay, nothing to
/// cancel - and there may be one per account at once. Each sends one
/// [`ProbeResult`] and exits; [`Self::poll`] collects them.
///
/// When to probe is decided here, purely, so it can be tested on a clock:
/// - when the accounts become visible (the Settings list or the wizard's
///   account step), unless the account was probed within [`PROBE_THROTTLE`];
/// - when an account's `auth.json` changed since its last probe - at once if
///   that is noticed as the list opens, else once it has been still for
///   [`AUTH_DEBOUNCE`].
#[derive(Default)]
pub(crate) struct CodexProbes {
    running: HashMap<Uuid, Receiver<ProbeResult>>,
    history: HashMap<Uuid, ProbeHistory>,
    visible: bool,
    last_check: Option<Instant>,
}

impl CodexProbes {
    /// Record whether the accounts are on screen; true when they just appeared
    pub(crate) fn set_visible(&mut self, visible: bool) -> bool {
        let opened = visible && !self.visible;
        self.visible = visible;
        opened
    }

    /// Whether it is time to look at the accounts' `auth.json` again
    ///
    /// The event loop spins far faster than a login changes; a stat per
    /// account every [`AUTH_POLL`] is plenty to keep the debounce honest.
    pub(crate) fn check_due(&mut self, now: Instant) -> bool {
        let due = self
            .last_check
            .is_none_or(|at| now.duration_since(at) >= AUTH_POLL);
        if due {
            self.last_check = Some(now);
        }
        due
    }

    /// Whether the accounts were on screen at the last [`Self::set_visible`]
    pub(crate) fn visible(&self) -> bool {
        self.visible
    }

    /// Whether a probe for `id` is running
    pub(crate) fn is_running(&self, id: Uuid) -> bool {
        self.running.contains_key(&id)
    }

    /// Whether to probe `id` now, given its `auth.json` modification time
    ///
    /// `opened` is whether the accounts just came on screen. Records the
    /// debounce as a side effect, so call it once per account per check.
    pub(crate) fn should_probe(
        &mut self,
        id: Uuid,
        auth: Option<SystemTime>,
        opened: bool,
        now: Instant,
    ) -> bool {
        if self.is_running(id) {
            return false;
        }
        let Some(history) = self.history.get_mut(&id) else {
            return opened;
        };
        if history.auth == auth {
            history.pending_auth = None;
            return opened && now.duration_since(history.started) >= PROBE_THROTTLE;
        }
        // The login changed since the last probe
        if opened {
            return true;
        }
        match history.pending_auth {
            Some((seen, since)) if seen == auth => now.duration_since(since) >= AUTH_DEBOUNCE,
            _ => {
                history.pending_auth = Some((auth, now));
                false
            }
        }
    }

    /// Start probing `id` on a thread of its own
    ///
    /// `auth` is the `auth.json` modification time the decision was made on.
    pub(crate) fn start(
        &mut self,
        id: Uuid,
        auth: Option<SystemTime>,
        request: ProbeRequest,
        now: Instant,
    ) {
        self.start_with(id, auth, now, move || {
            crate::codex_config::probe::probe_account(&request, ProbeBudget::default())
        });
    }

    /// Record a probe that failed before it could start
    pub(crate) fn record_start(&mut self, id: Uuid, auth: Option<SystemTime>, now: Instant) {
        self.history.insert(
            id,
            ProbeHistory {
                started: now,
                auth,
                pending_auth: None,
            },
        );
    }

    /// [`Self::start`] with the probe itself supplied, which is what tests use
    fn start_with(
        &mut self,
        id: Uuid,
        auth: Option<SystemTime>,
        now: Instant,
        probe: impl FnOnce() -> ProbeResult + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(probe());
        });
        self.running.insert(id, rx);
        self.record_start(id, auth, now);
    }

    /// Collect the probes that have finished
    ///
    /// A probe whose thread died without a result reports that as a failure,
    /// so its row does not say "checking" forever.
    pub(crate) fn poll(&mut self) -> Vec<(Uuid, ProbeResult)> {
        let mut finished = Vec::new();
        self.running.retain(|id, rx| match rx.try_recv() {
            Ok(result) => {
                finished.push((*id, result));
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => {
                finished.push((*id, Err("the probe ended without a result".to_string())));
                false
            }
        });
        finished
    }
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
    fn conversation_scan_runs_off_thread_and_reports_back() {
        use crate::transcript::scan::{ScanBudget, ScanRequest};

        let dir = tempfile::TempDir::new().unwrap();
        let job = BackgroundJob::spawn(
            GitTask::ScanConversations {
                request: Box::new(ScanRequest {
                    working_dir: dir.path().to_path_buf(),
                    claude_accounts: Vec::new(),
                    codex_accounts: Vec::new(),
                    claimed: HashSet::new(),
                    budget: ScanBudget::default(),
                }),
            },
            JobFollowUp::OpenConversationImport {
                project_id: uuid::Uuid::new_v4(),
                branch_id: uuid::Uuid::new_v4(),
            },
        );

        let result = loop {
            match job.poll() {
                JobPoll::Running => std::thread::sleep(std::time::Duration::from_millis(5)),
                JobPoll::Finished(result) => break result,
            }
        };
        match result.expect("worker should report a result").output {
            JobOutput::Conversations(outcome) => {
                assert!(outcome.conversations.is_empty());
                assert!(!outcome.cancelled);
            }
            _ => panic!("expected a Conversations output"),
        }
    }

    fn mtime(secs: u64) -> Option<SystemTime> {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
    }

    /// Wait for every running probe to report back
    fn drain(probes: &mut CodexProbes) -> Vec<(Uuid, ProbeResult)> {
        let mut all = Vec::new();
        for _ in 0..400 {
            all.extend(probes.poll());
            if probes.running.is_empty() {
                return all;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("probes never finished");
    }

    #[test]
    fn opening_probes_a_new_account_then_throttles() {
        let mut probes = CodexProbes::default();
        let id = Uuid::new_v4();
        let t0 = Instant::now();

        assert!(
            !probes.should_probe(id, mtime(1), false, t0),
            "not yet visible"
        );
        assert!(probes.should_probe(id, mtime(1), true, t0));
        probes.record_start(id, mtime(1), t0);

        let soon = t0 + Duration::from_secs(5);
        assert!(!probes.should_probe(id, mtime(1), true, soon), "throttled");
        let later = t0 + PROBE_THROTTLE;
        assert!(probes.should_probe(id, mtime(1), true, later));
    }

    #[test]
    fn a_login_change_seen_on_open_skips_the_throttle() {
        let mut probes = CodexProbes::default();
        let id = Uuid::new_v4();
        let t0 = Instant::now();
        probes.record_start(id, mtime(1), t0);

        assert!(probes.should_probe(id, mtime(2), true, t0 + Duration::from_secs(1)));
        // Logging out removes the file: also a change
        assert!(probes.should_probe(id, None, true, t0 + Duration::from_secs(1)));
    }

    #[test]
    fn a_login_change_while_open_waits_out_the_debounce() {
        let mut probes = CodexProbes::default();
        let id = Uuid::new_v4();
        let t0 = Instant::now();
        probes.record_start(id, mtime(1), t0);

        let t1 = t0 + Duration::from_secs(1);
        assert!(
            !probes.should_probe(id, mtime(2), false, t1),
            "just changed"
        );
        assert!(!probes.should_probe(id, mtime(2), false, t1 + AUTH_DEBOUNCE / 2));
        // Written again mid-debounce: the wait starts over
        let t2 = t1 + AUTH_DEBOUNCE / 2;
        assert!(!probes.should_probe(id, mtime(3), false, t2));
        assert!(!probes.should_probe(id, mtime(3), false, t2 + AUTH_DEBOUNCE / 2));
        assert!(probes.should_probe(id, mtime(3), false, t2 + AUTH_DEBOUNCE));
    }

    #[test]
    fn an_unchanged_login_is_not_reprobed_while_open() {
        let mut probes = CodexProbes::default();
        let id = Uuid::new_v4();
        let t0 = Instant::now();
        probes.record_start(id, mtime(1), t0);

        let much_later = t0 + PROBE_THROTTLE * 10;
        assert!(!probes.should_probe(id, mtime(1), false, much_later));
    }

    #[test]
    fn set_visible_reports_only_the_rising_edge() {
        let mut probes = CodexProbes::default();
        assert!(!probes.set_visible(false));
        assert!(probes.set_visible(true));
        assert!(!probes.set_visible(true));
        assert!(!probes.set_visible(false));
        assert!(probes.set_visible(true));
    }

    #[test]
    fn running_probes_report_back_and_are_not_doubled() {
        use crate::codex_config::probe::{CodexAccountStatus, CodexLogin, CodexUsage};

        let mut probes = CodexProbes::default();
        let (ok, failed) = (Uuid::new_v4(), Uuid::new_v4());
        let now = Instant::now();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        probes.start_with(ok, mtime(1), now, move || {
            let _ = release_rx.recv();
            Ok(CodexAccountStatus {
                login: CodexLogin::LoggedOut,
                usage: CodexUsage::NotApplicable("not logged in"),
            })
        });
        probes.start_with(failed, None, now, || Err("timed out".to_string()));

        assert!(probes.is_running(ok));
        assert!(
            !probes.should_probe(ok, mtime(2), true, now + PROBE_THROTTLE),
            "a running probe is never doubled"
        );

        release_tx.send(()).unwrap();
        let mut finished = drain(&mut probes);
        finished.sort_by_key(|(id, _)| *id == ok);
        assert_eq!(finished.len(), 2);
        assert_eq!(finished[0], (failed, Err("timed out".to_string())));
        assert!(finished[1].1.is_ok());
        assert!(!probes.is_running(ok));
    }

    #[test]
    fn a_probe_that_panics_reports_a_failure() {
        let mut probes = CodexProbes::default();
        let id = Uuid::new_v4();
        probes.start_with(id, None, Instant::now(), || panic!("probe blew up"));

        let finished = drain(&mut probes);

        assert_eq!(finished.len(), 1);
        assert!(finished[0].1.is_err());
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
