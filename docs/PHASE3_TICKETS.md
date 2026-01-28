# Phase 3: Polish & Robustness - Implementation Tickets

This document contains 21 self-contained tickets for Phase 3 implementation. Each ticket includes enough context for a developer to work on it independently.

**Progress Tracking:** Mark `[x]` when completed.

---

## Sprint 1: Critical Safety

---

### Ticket 1: Index Out of Bounds Race Conditions

- [x] **Completed**

**Priority:** P0 - Critical
**Category:** Input Handling Edge Cases
**Original ID:** 2.1

#### Problem Description

Multiple places use `selected_index` to access collections without re-validating after potential state changes. Sessions/projects can be destroyed between `.len()` check and `.get()` access, causing index out of bounds panics.

#### Affected Files

- `src/input/normal/branch_detail.rs:45, 65-74`
- `src/input/normal/projects_overview.rs:108-111, 156-193`
- `src/input/normal/session_view.rs:102-120`
- `src/input/normal/project_detail.rs:53-56`
- `src/input/normal/timeline.rs`

#### Fix Approach

Replace vulnerable pattern:
```rust
// BEFORE (vulnerable):
let count = app.sessions.len();
if count > 0 {
    let idx = (app.state.selected_index + 1) % count;
    if let Some(session) = app.sessions.get_by_index(idx) {
        // use session
    }
}

// AFTER (safe):
if let Some(session) = app.sessions.get_by_index(app.state.selected_index) {
    // use session directly, index already validated
} else {
    // Reset index to 0 or handle empty state
    app.state.selected_index = 0;
}
```

Specific changes:
1. **branch_detail.rs**: Validate session exists before showing delete dialog
2. **projects_overview.rs**: Re-fetch project list atomically with access
3. **session_view.rs**: Handle destroyed session during Tab navigation
4. **timeline.rs**: Bounds-check before timeline navigation

#### Acceptance Criteria

- [ ] All index-based accesses use checked methods (`.get()` not direct indexing)
- [ ] Index is reset when collection shrinks below current index
- [ ] No panic possible from index out of bounds

---

### Ticket 2: Worktree Wizard Index Synchronization

- [x] **Completed**

**Priority:** P0 - Critical
**Category:** Input Handling Edge Cases
**Original ID:** 2.2

#### Problem Description

If `filtered_branches` is recalculated while user navigates, `list_index` can be out of bounds for the new list, causing a panic when accessing the branch list.

#### Affected Files

- `src/wizards/worktree/handlers.rs:53-83, 188-210`

#### Fix Approach

1. Clamp `list_index` whenever `filtered_branches` changes
2. Add helper function `clamp_wizard_index()` called after any filter operation
3. Use checked indexing for all wizard list accesses

```rust
fn clamp_wizard_index(state: &mut AppState) {
    let max_index = state.filtered_branches.len().saturating_sub(1);
    state.list_index = state.list_index.min(max_index);
}
```

#### Acceptance Criteria

- [ ] `list_index` is always valid for current `filtered_branches`
- [ ] Typing to filter doesn't cause index panic
- [ ] Navigation wraps correctly at boundaries

---

### Ticket 3: Hook Server Crash Detection

- [x] **Completed**

**Priority:** P0 - Critical
**Category:** Critical Error Handling
**Original ID:** 1.1

#### Problem Description

If the hook server crashes, errors are silently discarded. The app appears to work but won't receive Claude Code state updates, leaving users unaware of the problem.

#### Affected Files

- `src/hooks/server.rs:134-141`

#### Current Code

```rust
tokio::spawn(async move {
    axum::serve(listener, app)
        .with_graceful_shutdown(...)
        .await
        .ok();  // Error is swallowed!
});
```

#### Fix Approach

1. Add a channel to report server health status back to the main app
2. Log any server errors before they're discarded
3. Update app state to show "Hook server disconnected" warning
4. Consider auto-restart mechanism for transient failures

#### Acceptance Criteria

- [ ] Server errors are logged at ERROR level
- [ ] App displays warning notification if hook server stops
- [ ] User can see hook server status in UI (e.g., header indicator)

---

### Ticket 4: Paste Input Length Limits

- [x] **Completed**

**Priority:** P1 - High
**Category:** Input Handling Edge Cases
**Original ID:** 2.3

#### Problem Description

No length limits on pasted text. User can paste massive content causing memory exhaustion or UI freezes.

#### Affected Files

- `src/input/text_input.rs:254`
- `src/app/mod.rs:341-345`

#### Fix Approach

1. Define maximum lengths per field:
   - Project path: 4096 characters
   - Session name: 256 characters
   - Branch name: 256 characters
2. Truncate paste input to limit with warning notification
3. Add length validation in character input handlers too

```rust
const MAX_PROJECT_PATH_LEN: usize = 4096;
const MAX_SESSION_NAME_LEN: usize = 256;
const MAX_BRANCH_NAME_LEN: usize = 256;

fn handle_paste(input: &str, max_len: usize) -> (String, bool) {
    if input.len() > max_len {
        (input[..max_len].to_string(), true) // truncated
    } else {
        (input.to_string(), false)
    }
}
```

#### Acceptance Criteria

- [ ] All text input fields have defined max lengths
- [ ] Paste operations truncate and warn user
- [ ] Memory usage bounded regardless of clipboard content

---

## Sprint 2: Error Visibility

---

### Ticket 5: File Logging Failure Visibility

- [x] **Completed**

**Priority:** P0 - Critical
**Category:** Critical Error Handling
**Original ID:** 1.2

#### Problem Description

Write failures to log files are completely silent. Users may lose debug logs without knowing, making troubleshooting impossible.

#### Affected Files

- `src/logging/file_writer.rs:42-43`

#### Current Code

```rust
let _ = file.write_all(buf);
let _ = file.flush();
```

#### Fix Approach

1. Add a failure counter to track consecutive write failures
2. After N consecutive failures (e.g., 5), emit a warning via tracing
3. Consider fallback to stderr if file logging consistently fails

```rust
static FAILURE_COUNT: AtomicUsize = AtomicUsize::new(0);
const FAILURE_THRESHOLD: usize = 5;

fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
    match self.file.write_all(buf).and_then(|_| self.file.flush()) {
        Ok(_) => {
            FAILURE_COUNT.store(0, Ordering::Relaxed);
            Ok(buf.len())
        }
        Err(e) => {
            let count = FAILURE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            if count == FAILURE_THRESHOLD {
                eprintln!("Warning: Log file writes failing repeatedly: {}", e);
            }
            Err(e)
        }
    }
}
```

#### Acceptance Criteria

- [ ] Consecutive write failures are tracked
- [ ] Warning emitted after threshold exceeded
- [ ] Fallback mechanism documented

---

### Ticket 6: TUI Teardown Error Logging

- [x] **Completed**

**Priority:** P1 - High
**Category:** Critical Error Handling
**Original ID:** 1.3

#### Problem Description

Terminal restoration errors are silently ignored. If restoration fails, terminal state may be corrupted with no diagnostic info for debugging.

#### Affected Files

- `src/tui/mod.rs:107-202`

#### Fix Approach

1. Log all terminal restoration attempts at DEBUG level
2. On failure, log at WARN level with specific operation that failed
3. Add final sanity check that terminal is in expected state

```rust
// Replace patterns like:
let _ = execute!(stdout, LeaveAlternateScreen);

// With:
if let Err(e) = execute!(stdout, LeaveAlternateScreen) {
    tracing::warn!("Failed to leave alternate screen: {}", e);
}
tracing::debug!("Left alternate screen successfully");
```

#### Acceptance Criteria

- [ ] All `let _ = ...` patterns in teardown have logging
- [ ] Terminal corruption issues can be diagnosed from logs

---

### Ticket 7: Focus Session Persistence Feedback

- [ ] **Completed**

**Priority:** P1 - High
**Category:** Critical Error Handling
**Original ID:** 1.4

#### Problem Description

When saving focus sessions fails, error is logged but user sees no feedback. Focus data silently lost without any indication.

#### Affected Files

- `src/app/mod.rs:620`

#### Fix Approach

1. Show transient notification on save failure
2. Keep failed session in memory with retry option
3. Log with full error context

```rust
match focus_store.save_session(&session) {
    Ok(_) => {
        app.notifications.add("Focus session saved");
    }
    Err(e) => {
        tracing::error!("Failed to save focus session: {:?}", e);
        app.notifications.add_error(format!(
            "Failed to save focus session: {}. Data kept in memory.",
            e
        ));
        // Keep session for potential retry
        app.unsaved_focus_sessions.push(session);
    }
}
```

#### Acceptance Criteria

- [ ] User sees notification when focus session save fails
- [ ] Error includes actionable information

---

### Ticket 8: Claude Code Process Crashes

- [x] **Completed**

**Priority:** P1 - High
**Category:** Edge Case Handling
**Original ID:** 4.1

#### Problem Description

When Claude Code crashes unexpectedly, users need clear feedback about what happened and proper cleanup of resources.

#### Affected Files

- `src/session/manager.rs`
- `src/session/mod.rs`
- `src/app/mod.rs`

#### Fix Approach

1. Detect abnormal process termination (non-zero exit, signal)
2. Show notification with exit reason
3. Offer to restart session or view last output
4. Clean up any orphaned resources

```rust
enum ExitReason {
    Normal,
    Error(i32),      // exit code
    Signal(i32),     // signal number
    Unknown,
}

fn detect_exit_reason(status: ExitStatus) -> ExitReason {
    if status.success() {
        ExitReason::Normal
    } else if let Some(code) = status.code() {
        ExitReason::Error(code)
    } else {
        #[cfg(unix)]
        if let Some(sig) = status.signal() {
            return ExitReason::Signal(sig);
        }
        ExitReason::Unknown
    }
}
```

#### Acceptance Criteria

- [ ] Crashed sessions show clear error state
- [ ] Exit reason displayed (signal, exit code)
- [ ] User can see last output before crash

---

## Sprint 3: Edge Cases

---

### Ticket 9: Active Session Reference Validation

- [ ] **Completed**

**Priority:** P1 - High
**Category:** Input Handling Edge Cases
**Original ID:** 2.4

#### Problem Description

`active_session` can reference a destroyed session. Operations on it silently fail or cause undefined behavior.

#### Affected Files

- `src/app/mod.rs:359-365` (paste handling)
- `src/input/normal/session_view.rs`
- `src/input/session_mode.rs`

#### Fix Approach

1. When session is destroyed, check if it's the active session and clear `active_session`
2. Add validation before any operation on `active_session`
3. Navigate back to parent view if active session is destroyed

```rust
fn destroy_session(&mut self, session_id: &str) {
    if self.active_session.as_deref() == Some(session_id) {
        self.active_session = None;
        self.set_view(View::BranchDetail);
        self.notifications.add("Active session ended");
    }
    // ... rest of destroy logic
}

fn validate_active_session(&self) -> Option<&Session> {
    self.active_session.as_ref()
        .and_then(|id| self.sessions.get(id))
}
```

#### Acceptance Criteria

- [ ] Destroying active session clears reference and navigates away
- [ ] No operations attempted on destroyed sessions
- [ ] User sees clear feedback when session exits

---

### Ticket 10: Hook Server Port Conflicts

- [ ] **Completed**

**Priority:** P1 - High
**Category:** Edge Case Handling
**Original ID:** 4.2

#### Problem Description

If port 9999 is in use, hook server fails silently. Users don't know why state updates aren't working.

#### Affected Files

- `src/hooks/server.rs`
- `src/config.rs`

#### Fix Approach

1. Check port availability before binding
2. Show clear error if port in use
3. Consider auto-selecting available port with fallback
4. Document port configuration option

```rust
fn check_port_available(port: u16) -> Result<()> {
    match TcpListener::bind(("127.0.0.1", port)) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            Err(anyhow!(
                "Port {} is already in use. Either:\n\
                 1. Stop the other process using this port\n\
                 2. Configure a different port in ~/.panoptes/config.toml\n\
                    Example: hook_port = 9998",
                port
            ))
        }
        Err(e) => Err(e.into()),
    }
}
```

#### Acceptance Criteria

- [ ] Port conflict shows clear error message
- [ ] User knows how to resolve (config or kill other process)

---

### Ticket 11: Git Operation Error Context

- [ ] **Completed**

**Priority:** P1 - High
**Category:** Critical Error Handling
**Original ID:** 1.5

#### Problem Description

When all git ref lookups fail in `resolve_ref_to_commit()`, returns generic error without context about what was tried, making debugging difficult.

#### Affected Files

- `src/git/worktree.rs:102-130`

#### Fix Approach

1. Collect all attempted ref lookup methods
2. Return combined error message listing all failed approaches
3. Include original ref name in error

```rust
fn resolve_ref_to_commit(&self, ref_name: &str) -> Result<Oid> {
    let mut attempts = Vec::new();

    // Try direct ref
    match self.repo.find_reference(ref_name) {
        Ok(r) => return Ok(r.peel_to_commit()?.id()),
        Err(e) => attempts.push(format!("direct ref: {}", e)),
    }

    // Try as branch
    match self.repo.find_branch(ref_name, BranchType::Local) {
        Ok(b) => return Ok(b.get().peel_to_commit()?.id()),
        Err(e) => attempts.push(format!("local branch: {}", e)),
    }

    // Try revparse
    match self.repo.revparse_single(ref_name) {
        Ok(obj) => return Ok(obj.peel_to_commit()?.id()),
        Err(e) => attempts.push(format!("revparse: {}", e)),
    }

    Err(anyhow!(
        "Could not resolve '{}' to a commit. Tried:\n  - {}",
        ref_name,
        attempts.join("\n  - ")
    ))
}
```

#### Acceptance Criteria

- [ ] Error message shows all lookup methods attempted
- [ ] User can understand why ref resolution failed

---

### Ticket 12: Project Store Corruption Handling

- [x] **Completed**

**Priority:** P2 - Medium
**Category:** Critical Error Handling
**Original ID:** 1.6

#### Problem Description

If project store file is corrupted, all projects are lost with only a warning log. No backup created, no user notification.

#### Affected Files

- `src/app/mod.rs:73-76`
- `src/project/store.rs`

#### Fix Approach

1. Before discarding corrupted store, backup to `projects.json.corrupt.{timestamp}`
2. Show prominent notification to user
3. Log the corruption error with full details

```rust
fn load_or_create() -> Result<ProjectStore> {
    match Self::load() {
        Ok(store) => Ok(store),
        Err(e) => {
            let store_path = config::projects_path();
            if store_path.exists() {
                // Backup corrupted file
                let backup_path = store_path.with_extension(
                    format!("json.corrupt.{}", chrono::Utc::now().timestamp())
                );
                if let Err(backup_err) = std::fs::copy(&store_path, &backup_path) {
                    tracing::error!("Failed to backup corrupted store: {}", backup_err);
                } else {
                    tracing::warn!(
                        "Corrupted project store backed up to: {:?}",
                        backup_path
                    );
                }
            }
            tracing::error!("Project store corruption: {:?}", e);
            Ok(ProjectStore::new())
        }
    }
}
```

#### Acceptance Criteria

- [ ] Corrupted store is backed up, not deleted
- [ ] User sees clear notification about data recovery

---

## Sprint 4: Polish & Performance

---

### Ticket 13: Benchmark with 20+ Sessions

- [ ] **Completed**

**Priority:** P1 - High
**Category:** Performance Optimization
**Original ID:** 3.1

#### Problem Description

Need to verify application remains responsive with many concurrent sessions. Unknown performance characteristics at scale.

#### Affected Files

- New file: `benches/session_scale.rs` or `tests/performance.rs`
- Documentation in `docs/PERFORMANCE.md`

#### Tasks

1. Create test script to spawn N sessions programmatically
2. Measure:
   - Input latency (key press to screen update)
   - Memory usage per session
   - CPU usage during active sessions
   - Hook event processing throughput
3. Profile with `cargo flamegraph` to identify bottlenecks
4. Document performance characteristics

```rust
#[test]
#[ignore] // Run manually with: cargo test performance --ignored
fn test_20_session_performance() {
    let app = App::new().unwrap();

    // Spawn 20 sessions
    for i in 0..20 {
        app.create_session(format!("test-session-{}", i)).unwrap();
    }

    // Measure input latency
    let start = Instant::now();
    app.handle_key(KeyCode::Down);
    let latency = start.elapsed();

    assert!(latency < Duration::from_millis(50),
        "Input latency too high: {:?}", latency);
}
```

#### Acceptance Criteria

- [ ] Input latency < 50ms with 20 sessions
- [ ] Memory usage documented and reasonable
- [ ] No UI freezes during heavy activity

---

### Ticket 14: Input Mode / View State Consistency

- [x] **Completed**

**Priority:** P2 - Medium
**Category:** Input Handling Edge Cases
**Original ID:** 2.5

#### Problem Description

Input mode can become inconsistent with current view if state changes during input handling. This can cause keystrokes to be misrouted.

#### Affected Files

- `src/input/dispatcher.rs`
- `src/app/state.rs`

#### Fix Approach

1. Add validation that input mode is appropriate for current view
2. If mismatch detected, reset to Normal mode and log warning
3. Consider making view transitions atomic with mode changes

```rust
fn validate_mode_view_consistency(state: &mut AppState) {
    let expected_mode = match state.view {
        View::SessionView => {
            if state.session_mode_active {
                InputMode::Session
            } else {
                InputMode::Normal
            }
        }
        View::ProjectsOverview | View::ProjectDetail | View::BranchDetail => {
            InputMode::Normal
        }
        // ... other views
    };

    if state.input_mode != expected_mode && !is_valid_mode_for_view(&state.input_mode, &state.view) {
        tracing::warn!(
            "Mode/view mismatch: {:?} in {:?}, resetting to {:?}",
            state.input_mode, state.view, expected_mode
        );
        state.input_mode = expected_mode;
    }
}
```

#### Acceptance Criteria

- [ ] Stray keystrokes in wrong mode don't cause issues
- [ ] Mode/view mismatches are detected and corrected

---

### Ticket 15: Hook Event Processing Optimization

- [ ] **Completed**

**Priority:** P2 - Medium
**Category:** Performance Optimization
**Original ID:** 3.2

#### Problem Description

Hook events are processed sequentially. High event rate could cause backlog, leading to delayed state updates.

#### Affected Files

- `src/app/mod.rs:221-262`
- `src/hooks/server.rs`

#### Tasks

1. Measure hook event processing time
2. Consider batch processing of queued events
3. Add dropped event monitoring and alerting
4. Implement event coalescing for rapid state changes

```rust
fn process_hook_events(&mut self) {
    let mut events: Vec<HookEvent> = Vec::new();

    // Drain all available events
    while let Ok(event) = self.hook_rx.try_recv() {
        events.push(event);
    }

    if events.len() > 100 {
        tracing::warn!("High hook event backlog: {} events", events.len());
    }

    // Coalesce: keep only latest state per session
    let mut latest_state: HashMap<String, SessionState> = HashMap::new();
    for event in events {
        latest_state.insert(event.session_id.clone(), event.new_state);
    }

    // Apply coalesced updates
    for (session_id, state) in latest_state {
        self.update_session_state(&session_id, state);
    }
}
```

#### Acceptance Criteria

- [ ] Hook processing doesn't block UI rendering
- [ ] Dropped events are tracked and reported
- [ ] Burst of events handled gracefully

---

### Ticket 16: Git Repository State Changes

- [ ] **Completed**

**Priority:** P2 - Medium
**Category:** Edge Case Handling
**Original ID:** 4.3

#### Problem Description

External git operations (branch deleted, worktree removed) can invalidate app state. The app doesn't detect or handle these changes.

#### Affected Files

- `src/project/store.rs`
- `src/git/mod.rs`
- `src/app/mod.rs`

#### Tasks

1. Validate git state before operations
2. Handle missing worktree directories gracefully
3. Refresh branch list on demand
4. Show warning for orphaned sessions

```rust
fn validate_branch(&self, branch: &Branch) -> BranchStatus {
    // Check if worktree directory exists
    if branch.is_worktree && !branch.working_dir.exists() {
        return BranchStatus::WorktreeMissing;
    }

    // Check if branch still exists in git
    if let Err(_) = self.git.find_branch(&branch.name) {
        return BranchStatus::BranchDeleted;
    }

    BranchStatus::Valid
}

// Add refresh keybinding (e.g., 'R')
fn refresh_git_state(&mut self) {
    for project in self.project_store.projects_mut() {
        self.sync_branches_with_git(project);
    }
    self.notifications.add("Git state refreshed");
}
```

#### Acceptance Criteria

- [ ] External branch deletion doesn't crash app
- [ ] Missing worktrees are detected and reported
- [ ] User can manually refresh git state

---

## Sprint 5: Remaining Items

---

### Ticket 17: Path Completion Index Bounds

- [ ] **Completed**

**Priority:** P2 - Medium
**Category:** Input Handling Edge Cases
**Original ID:** 2.6

#### Problem Description

`path_completion_index` can be out of bounds if completions list shrinks when user continues typing.

#### Affected Files

- `src/input/text_input.rs:135-155`
- `src/app/state.rs`

#### Fix Approach

1. Clamp `path_completion_index` whenever `path_completions` is recalculated
2. Reset index to 0 when completions change significantly

```rust
fn update_path_completions(&mut self, input: &str) {
    let old_len = self.path_completions.len();
    self.path_completions = complete_path(input);

    // Clamp index to valid range
    if self.path_completions.is_empty() {
        self.path_completion_index = 0;
    } else {
        self.path_completion_index = self.path_completion_index
            .min(self.path_completions.len() - 1);
    }

    // Reset if list changed significantly
    if self.path_completions.len() != old_len {
        self.path_completion_index = 0;
    }
}
```

#### Acceptance Criteria

- [ ] Tab completion never accesses out-of-bounds
- [ ] Index reset when completion list changes

---

### Ticket 18: Delete Confirmation with Stale References

- [ ] **Completed**

**Priority:** P2 - Medium
**Category:** Input Handling Edge Cases
**Original ID:** 2.7

#### Problem Description

Item could be deleted externally between showing confirmation dialog and user pressing 'y', causing operations on non-existent items.

#### Affected Files

- `src/input/dialogs.rs:54-87`
- `src/input/dialogs.rs:110-143`

#### Fix Approach

1. Re-validate that item still exists when confirmation received
2. Show "Item no longer exists" message if already deleted
3. Cancel dialog gracefully

```rust
fn handle_delete_confirmation(app: &mut App, confirmed: bool) {
    if !confirmed {
        app.state.input_mode = InputMode::Normal;
        return;
    }

    // Re-validate item exists
    match &app.state.pending_delete {
        PendingDelete::Session(id) => {
            if app.sessions.get(id).is_none() {
                app.notifications.add("Session no longer exists");
                app.state.input_mode = InputMode::Normal;
                app.state.pending_delete = None;
                return;
            }
            app.destroy_session(id);
        }
        PendingDelete::Branch(id) => {
            if app.project_store.get_branch(id).is_none() {
                app.notifications.add("Branch no longer exists");
                app.state.input_mode = InputMode::Normal;
                app.state.pending_delete = None;
                return;
            }
            app.delete_branch(id);
        }
        None => {}
    }

    app.state.input_mode = InputMode::Normal;
    app.state.pending_delete = None;
}
```

#### Acceptance Criteria

- [ ] Confirming delete of already-deleted item shows helpful message
- [ ] No panic or silent failure

---

### Ticket 19: Terminal Resize During Operations

- [x] **Completed**

**Priority:** P2 - Medium
**Category:** Edge Case Handling
**Original ID:** 4.4

#### Problem Description

Terminal resize during session operations could cause rendering issues or PTY size mismatches.

#### Affected Files

- `src/tui/mod.rs`
- `src/session/pty.rs`
- `src/app/mod.rs`

#### Tasks

1. Audit resize handling in all views
2. Ensure PTY resize is propagated correctly to all sessions
3. Test rapid resize scenarios

```rust
fn handle_resize(&mut self, width: u16, height: u16) {
    // Update TUI size
    self.tui.resize(width, height);

    // Propagate to all active PTYs
    for session in self.sessions.values_mut() {
        if let Err(e) = session.pty.resize(width, height) {
            tracing::warn!(
                "Failed to resize PTY for session {}: {}",
                session.id, e
            );
        }
    }

    // Force redraw
    self.needs_redraw = true;
}
```

#### Acceptance Criteria

- [ ] Resize during any operation doesn't crash
- [ ] Session PTY receives correct new size

---

### Ticket 20: Disk Full / Permission Errors

- [ ] **Completed**

**Priority:** P3 - Low
**Category:** Edge Case Handling
**Original ID:** 4.5

#### Problem Description

Disk full or permission errors during persistence operations are not handled gracefully.

#### Affected Files

- `src/project/store.rs`
- `src/config.rs`
- `src/focus_timing/store.rs`

#### Tasks

1. Handle disk full error when saving projects/config
2. Handle permission denied errors
3. Show actionable error messages

```rust
fn save(&self) -> Result<()> {
    let path = config::projects_path();
    let content = toml::to_string_pretty(&self)?;

    match std::fs::write(&path, &content) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            Err(anyhow!(
                "Permission denied writing to {:?}. \
                 Check file permissions or run with appropriate privileges.",
                path
            ))
        }
        Err(e) if e.raw_os_error() == Some(28) => { // ENOSPC
            Err(anyhow!(
                "Disk full - cannot save to {:?}. \
                 Free up disk space and try again.",
                path
            ))
        }
        Err(e) => Err(e.into()),
    }
}
```

#### Acceptance Criteria

- [ ] Disk full shows clear message
- [ ] Permission errors explained to user

---

### Ticket 21: Session Output Buffer Optimization

- [ ] **Completed**

**Priority:** P3 - Low
**Category:** Performance Optimization
**Original ID:** 3.3

#### Problem Description

Session output buffers may consume excessive memory. Large outputs may cause lag during rendering.

#### Affected Files

- `src/session/mod.rs` (OutputBuffer)
- `src/session/vterm.rs`

#### Tasks

1. Profile memory usage of output ring buffer
2. Consider lazy rendering (only parse visible portion)
3. Measure impact of VTerm processing on large outputs

```rust
impl OutputBuffer {
    const MAX_LINES: usize = 10_000;  // Configurable limit

    fn append(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);

        // Trim if exceeds limit
        if self.line_count() > Self::MAX_LINES {
            self.trim_oldest_lines(Self::MAX_LINES / 2);
        }
    }

    fn visible_range(&self, scroll_offset: usize, height: usize) -> Range<usize> {
        let start = scroll_offset;
        let end = (scroll_offset + height).min(self.line_count());
        start..end
    }

    // Only process VTerm for visible lines
    fn render_visible(&self, scroll_offset: usize, height: usize) -> Vec<Line> {
        let range = self.visible_range(scroll_offset, height);
        self.lines[range].iter()
            .map(|line| self.vterm.process_line(line))
            .collect()
    }
}
```

#### Acceptance Criteria

- [ ] Memory usage per session documented
- [ ] Large outputs don't cause lag

---

## Summary

| Sprint | Tickets | Focus |
|--------|---------|-------|
| 1 | 1-4 | Critical Safety |
| 2 | 5-8 | Error Visibility |
| 3 | 9-12 | Edge Cases |
| 4 | 13-16 | Polish & Performance |
| 5 | 17-21 | Remaining Items |

**Total Tickets:** 21

**Success Metrics:**
1. Zero panics from user input in any mode
2. All errors visible to user or logged
3. < 50ms input latency with 20 sessions
4. Graceful degradation when external resources fail
5. Clear feedback for all failure scenarios
