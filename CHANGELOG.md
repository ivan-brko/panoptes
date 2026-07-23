# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Projects can be grouped into folders in the projects overview, nested up to 3 levels deep (`m` to move, `r` to rename, `d` to ungroup, `Enter`/`←`/`→` to fold).

### Changed
- **Git operations no longer freeze the interface.** Fetching remotes and creating or removing a worktree ran on the event-loop thread behind a static "Please Wait" box, so the whole TUI stopped rendering — no session output, no hook updates, no way out — for as long as git took. They now run on a worker thread while the UI keeps rendering, under a "Working" overlay with an animated spinner. A slow `git fetch` can be called off with `Esc`: the fetch is killed and the flow continues with the refs already on disk, the same fallback used when a fetch fails. Worktree create and remove are deliberately not cancellable — interrupting one halfway leaves the repository worse off than not starting.
- Terminology is now consistent across footers, help, and dialogs: the Claude/Codex account configs are "configs" everywhere (were "Configs", "Configurations", and "accounts" in different places), and entering or leaving Session mode is "session mode" everywhere (the footer said "activate"/"deactivate" while the help said "Enter/Exit session mode").
- The worktree delete dialog says plainly that the git branch itself is never deleted, and its toggle is explicit about deleting the *directory* from disk.
- **All state files are now written atomically.** `projects.json`, `sessions.json`, both agent-config files, and `config.toml` are saved via a sibling temp file and rename, so a crash mid-write can never truncate a store. Corrupted files are uniformly backed up to a timestamped `<file>.corrupt.<timestamp>` sibling before starting fresh.
- `notification_method` is validated on load: `bell`, `title`, or `none`. Unknown values log a warning and fall back to `bell` instead of being silently misread.
- A stalled PTY write blocks the UI for at most ~50ms (was up to 1s), and runaway child output is drained with a per-tick budget, so a single misbehaving session can no longer starve the interface.
- Paste now works in every text-input mode — config names and paths, folder move/rename, and custom shortcut fields — not just session creation.
- UI consistency pass: a single `▶` selection glyph everywhere (the worktree wizard used `▸`), standard green/red Yes/No buttons in all confirmations, selector overlays styled like the other menus, and dialogs that clamp inside tiny terminals instead of overflowing.
- Spawn failures and worktree-wizard errors now include the underlying cause instead of a bare summary.

### Fixed
- **A partial `config.toml` no longer refuses to start.** `hook_port`, `worktrees_dir`, `hooks_dir`, and `max_output_lines` had no defaults when deserializing, so a config file omitting any of them failed with `missing field ...` — including the example the configuration guide told you to create. All four now fall back to the same values `Config::default()` already used.
- Documentation now matches the code: the help overlay and keyboard reference listed `Shift+Tab`, `i`, `g`/`G`, and an `f` auto-follow toggle that were never implemented, and `1-9` in views that do not support it. `max_output_lines` and `theme_preset` are documented as parsed-but-unused, which is what they are.
- **Every Claude `Notification` hook rang the bell and flagged the session.** The `notification_type` values Panoptes matched (`idle`, `permission_request`, `task_completed`, `elicitation`) were not the ones Claude Code sends (`idle_prompt`, `permission_prompt`, `agent_completed`, `elicitation_dialog`, …), so all of them fell through to the unknown case, which assumes the agent wants you. The worst offender was `idle_prompt`, which fires repeatedly while you have *not* replied — so a session you had already read kept announcing itself with nothing new to show. `auth_success`, `elicitation_complete`, and `elicitation_response` are now classified as informational and stay silent.
- The session open on screen no longer flags itself as needing attention. An event arriving while you are looking at a session used to leave its badge set until you navigated away and back.
- Panics on non-ASCII text: both the settings-view path truncation and the paste-limit truncation sliced strings mid-codepoint and crashed on multi-byte characters.
- Codex subagent counts were computed from the default `CODEX_HOME` even for sessions running under a different Codex account, so their counts were wrong or missing.
- Multi-byte characters split across PTY reads no longer render as `�` in Codex fallback scrollback; trailing partial bytes are held until the rest of the character arrives.
- A failed paste or keystroke no longer leaves a session stuck showing "Thinking".
- Session cleanup leaked navigation-order entries, so number-key jumps could hit holes after sessions aged out.
- **A corrupt `config.toml` no longer aborts startup.** The unparseable file is backed up with a timestamp, defaults are used, and a visible warning explains what happened.
- A corrupt `~/.claude/.claude.json` no longer silently disables permission comparison — it is warned about and treated as empty, and the file itself is never touched.
- A stale default account pointing at a deleted profile now recovers deterministically (alphabetically first remaining profile) for Claude configs too, matching Codex.
- Removing a git worktree without force no longer deletes the working tree on disk, so local modifications survive.
- Malformed hook payloads are now logged instead of vanishing silently.
- Session-create failures always surface an on-screen error; previously only Codex sessions reported them.
- New sessions start with correct PTY dimensions, removing a brief mis-sized flash on open.
- Custom-shortcut sessions set auto-close at creation time, closing a race where a command that finished instantly missed the flag and never closed.
- **A session you had already read kept demanding attention forever.** Alongside the flag set by real events, `session_needs_attention` had a second, time-based rule: any session in `Waiting` whose last activity was older than `idle_threshold_secs` was flagged too. Acknowledging clears the flag, not the clock, so opening the session did nothing and the badge reappeared the instant you looked away. The rule was a leftover from when the state was called `Idle` and meant "this session has gone abnormally quiet" — a job now done by the `Stalled` attention reason. Renaming it to `Waiting`, the normal resting state of every healthy session, quietly turned it into "you finished a turn five minutes ago". It bit Codex hardest: a Codex session parked at its prompt writes nothing to its PTY or its rollout, so its activity clock never moved. Attention is now raised only by events that actually mean something.

### Removed
- The `idle_threshold_secs` config key, which only fed the time-based attention rule described above. Leaving it in `config.toml` is harmless — unknown keys are ignored.
- **Activity Timeline view.** The `a` shortcut, the view, and its documentation are gone. It listed every session sorted by recency, but selecting a row and opening it used two different orderings — the list sorted by last activity while `Enter` indexed creation order — so it opened the wrong session as soon as the two diverged. The homepage Sessions panel covers the same ground without that flaw. `a` is now free for a custom shortcut.
- **Vim-style `j`/`k` navigation.** It only ever worked in the log viewer, the two config views, and four selector dialogs, and `k` never reached a view at all — it is globally bound to the custom shortcuts manager. Navigation is now consistently by arrow key.
- **Focus timer and focus statistics.** The `t`, `T`, and `Ctrl+t` shortcuts, the Focus Statistics view, and all focus-interval tracking are gone.
  - The `focus_timer_minutes` and `focus_stats_retention_days` config keys are no longer read. Leaving them in `config.toml` is harmless — unknown keys are ignored.
  - `~/.panoptes/focus_sessions.json` is no longer read or written. Existing files are left on disk and can be deleted by hand.
  - `t` and `T` are no longer reserved and can now be bound as custom shortcuts.
- **Overlay notification system.** The focus timer was its only producer, so `NotificationManager`, `NotificationType`, and the notification overlay are gone. Transient messages still appear in the header, and session attention still rings the bell / sets the terminal title per `notification_method`.
- **Terminal focus tracking.** Focus-change reporting is no longer requested from the terminal. A session you are currently viewing no longer rings when you switch away from the terminal window — attention notifications now fire only for sessions you are *not* looking at.
- Ordinal numbering in the projects list; digit keys now select only in the Sessions list.

### Technical
- Major internal refactor with no intended behavior change beyond the entries above: a shared persistence layer (`persistence.rs`), a generic agent-profile store (`agent_profiles.rs`) backing both Claude and Codex configs, a pure session state machine (`session/state_machine.rs`), unified Claude/Codex config input handlers and views, and a decomposed event loop. Unit test count grew from 520 to about 660.

## [0.3.1] - 2026-02-11

### Changed
- Refactored session scrolling handlers to share keyboard scroll logic across Session mode and normal Session view.
- Isolated Codex fallback scroll state to Codex sessions only.

### Fixed
- Restored reliable mouse-wheel scrolling in active Codex sessions.
- Fixed Codex upward-scroll edge case where history could get stuck at `Output [↑1]`.
- Fixed top-of-history over-scroll behavior that could visually remove bottom lines while scrolling up.

## [0.3.0] - 2026-02-11

### Added
- OpenAI Codex CLI support — run Codex sessions alongside Claude Code with the same attention tracking and session management
- Multi-account support for Codex CLI (CODEX_HOME-based configuration)
- Session type indicators ([CC], [CX], [SH]) in all session lists
- DSR (Device Status Report) query handling for PTY sessions

### Changed
- Updated shortcut documentation to be agent-agnostic
- Updated Ctrl+C warning to mention Esc as an alternative quit key

### Fixed
- Scroll not working in Codex sessions
- Codex session creation dialog not rendering
- Codex character dropping caused by blocking stdin read in notify hook
- Codex hook setup hardened to surface configuration failures
- Codex config selector bugs

### Removed
- codex-harness diagnostic binary

## [0.2.2] - 2025-01-29

### Added
- Custom shell session shortcuts feature - define custom keyboard shortcuts that execute shell commands
- Custom shortcuts support in branch detail view
- Custom shortcuts documentation in UI footers
- Shell session attention/notification support
- Comprehensive FAQ documentation
- Session mode troubleshooting questions to FAQ

### Changed
- Skip notifications and attention flag for active session (no notifications while you're viewing that session)

### Fixed
- Deletion dialog now shows correct warning for session type
- Keyboard shortcut documentation discrepancies

## [0.2.1] - 2025-01-27

### Added
- Hook event coalescing to prevent UI lag during rapid state changes
- Branch refresh feature (`R` key) to check for stale worktrees
- Stale worktree indicators (red highlighting for missing worktrees)
- Improved disk error handling with user-friendly messages for disk full and permission errors
- Comprehensive documentation:
  - [Keyboard Reference](docs/KEYBOARD_REFERENCE.md)
  - [Configuration Guide](docs/CONFIG_GUIDE.md)
  - [Troubleshooting Guide](docs/TROUBLESHOOTING.md)
  - [Installation Guide](docs/INSTALLATION.md)

### Changed
- README.md overhauled with improved structure and documentation links

## [0.1.0] - 2025-01-23

### Added

#### Core Features
- Multi-session management for Claude Code
- Project and branch organization with git repository support
- Real-time session state tracking (Starting, Thinking, Executing, Waiting, Idle, Exited)
- Session naming for easy identification
- Automatic session cleanup on quit

#### Git Integration
- Git worktree support for branch isolation
- Worktree creation wizard with branch selection
- Default base branch configuration per project
- Remote branch fetching via git CLI

#### Navigation
- Hierarchical navigation (Projects -> Branches -> Sessions)
- Activity timeline view for all sessions sorted by recent activity
- Keyboard-driven interface with vim-style navigation
- Number shortcuts for quick selection (1-9)

#### Attention System
- Terminal bell notifications when sessions need input
- Visual attention badges (green for new, yellow for idle)
- Attention count indicators in header
- Space key to jump to next session needing attention

#### Focus Timer
- Pomodoro-style focus timer with configurable duration
- Per-project and per-branch time tracking
- Focus statistics view with session history
- Terminal focus detection for accurate time tracking

#### User Interface
- Unified header component with notifications
- Transient header notifications for feedback
- Loading indicators for blocking operations
- Confirmation dialogs for destructive actions
- Log viewer for application debugging

#### Configuration
- TOML configuration file support
- Configurable hook server port
- Configurable notification method (bell, title, none)
- Theme presets (dark, light, high-contrast)

### Technical
- Built with Rust using async/await (Tokio runtime)
- Ratatui for terminal UI
- Axum for HTTP hook server
- vt100 crate for terminal emulation
- Portable-pty for PTY management
- git2 for git operations

### Fixed
- Paste handling with retry logic for non-blocking PTY writes
- Git fetch operations using git CLI for proper SSH authentication
- Branch name validation in worktree wizard
- Focus timer countdown accuracy with Alt+Tab detection
- Escape key behavior (Shift+Escape forwards to PTY)

[Unreleased]: https://github.com/ivan-brko/panoptes/compare/v0.3.1...HEAD
[0.3.1]: https://github.com/ivan-brko/panoptes/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/ivan-brko/panoptes/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/ivan-brko/panoptes/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/ivan-brko/panoptes/compare/v0.1.0...v0.2.1
[0.1.0]: https://github.com/ivan-brko/panoptes/releases/tag/v0.1.0
