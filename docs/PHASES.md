# Panoptes Implementation Phases

## Phase 1: Core Foundation ✅ Complete

**Goal**: Build a working prototype that can spawn and interact with multiple Claude Code sessions.

### What We Built

- **Project scaffolding** - Cargo workspace, module structure, dependencies
- **PTY management** - Spawn Claude Code in pseudo-terminals, handle input/output
- **Hook integration** - HTTP server to receive state updates from Claude Code
- **Basic TUI** - Single-pane session view, status bar, session switching
- **Session lifecycle** - Create, destroy, and switch between sessions

### Deliverable

A working application where you can:
- Create multiple Claude Code sessions
- Switch between them with keyboard shortcuts
- See real-time state updates (Starting → Thinking → Executing → Waiting)

---

## Phase 2: Multi-Project & Git Integration ✅ Complete

**Goal**: Full project and branch hierarchy with git worktree support.

### What We Built

- ✅ **Project model** - Data structures for projects, branches, and their relationships
- ✅ **Project persistence** - Projects and branches saved to `~/.panoptes/projects.json`
- ✅ **Git worktree support** - Create isolated working directories for each branch
- ✅ **Projects overview screen** - Grid view of all projects with session counts
- ✅ **Project detail view** - Branches and sessions within a project
- ✅ **Branch detail view** - Sessions for a specific branch with create/delete actions
- ✅ **Activity timeline** - All sessions sorted by recent activity
- ✅ **Fuzzy branch selector** - Type to filter existing branches or create new ones
- ✅ **Add project flow** - Add git repositories as projects with `a` key
- ✅ **Session deletion** - Delete sessions with confirmation dialog (`d` then `y/n`)
- ✅ **Attention system** - Terminal bell notifications, attention badges, idle threshold (bonus feature)

### Deliverable

A fully navigable application where you can:
- Add git repositories as projects
- Create sessions on specific branches
- Navigate between overview, project, branch, and session views
- Have automatic worktree creation for branch isolation
- Get notified when sessions need attention

---

## Phase 3: Polish & Robustness

**Goal**: Production-quality user experience and error handling. Make Panoptes production-ready through improved error handling, edge case coverage, and performance optimization.

**Status**: In Progress (10/21 tickets complete - Sprint 1 & 2 Critical Safety done)

### What We've Built

- ✅ Index bounds safety - All input handlers use `.get()` for checked access
- ✅ Worktree wizard index clamping - `clamp_list_index()` and `clamp_base_list_index()` helpers
- ✅ Hook server crash detection - Errors logged, status channel, dropped event tracking
- ✅ Paste input length limits - `MAX_*_LEN` constants with truncation
- ✅ File logging failure handling - Failure counters in `file_writer.rs`
- ✅ TUI teardown error logging - Terminal restoration logged properly

### What We're Building

- **Part 1: Critical Error Handling Fixes** - Hook server crash detection, file logging failures, TUI teardown errors, persistence feedback, git operation context, corruption handling
- **Part 2: Input Handling Edge Cases** - Race condition fixes, wizard index synchronization, paste limits, active session validation, mode/view consistency
- **Part 3: Performance Optimization** - Benchmark with 20+ sessions, hook event processing, output buffer optimization
- **Part 4: Edge Case Handling** - Process crashes, port conflicts, git state changes, terminal resize, disk/permission errors

---

### Part 1: Critical Error Handling Fixes

#### 1.1 Hook Server Crash Detection (P0 - Critical)

**Problem:** If the hook server crashes, errors are silently discarded. The app appears to work but won't receive Claude Code state updates.

**File:** `src/hooks/server.rs:134-141`

**Current Code:**
```rust
tokio::spawn(async move {
    axum::serve(listener, app)
        .with_graceful_shutdown(...)
        .await
        .ok();  // Error is swallowed!
});
```

**Fix:**
1. Add a channel to report server health status back to the main app
2. Log any server errors before they're discarded
3. Update app state to show "Hook server disconnected" warning
4. Consider auto-restart mechanism for transient failures

**Acceptance Criteria:**
- [ ] Server errors are logged at ERROR level
- [ ] App displays warning notification if hook server stops
- [ ] User can see hook server status in UI (e.g., header indicator)

---

#### 1.2 File Logging Failure Visibility (P0 - Critical)

**Problem:** Write failures to log files are completely silent. Users may lose debug logs without knowing.

**File:** `src/logging/file_writer.rs:42-43`

**Current Code:**
```rust
let _ = file.write_all(buf);
let _ = file.flush();
```

**Fix:**
1. Add a failure counter
2. After N consecutive failures (e.g., 5), emit a warning via tracing
3. Consider fallback to stderr if file logging consistently fails

**Acceptance Criteria:**
- [ ] Consecutive write failures are tracked
- [ ] Warning emitted after threshold exceeded
- [ ] Fallback mechanism documented

---

#### 1.3 TUI Teardown Error Logging (P1 - High)

**Problem:** Terminal restoration errors are silently ignored. If restoration fails, terminal state may be corrupted with no diagnostic info.

**File:** `src/tui/mod.rs:107-202`

**Fix:**
1. Log all terminal restoration attempts at DEBUG level
2. On failure, log at WARN level with specific operation that failed
3. Add final sanity check that terminal is in expected state

**Acceptance Criteria:**
- [ ] All `let _ = ...` patterns in teardown have logging
- [ ] Terminal corruption issues can be diagnosed from logs

---

#### 1.4 Focus Session Persistence Feedback (P1 - High)

**Problem:** When saving focus sessions fails, error is logged but user sees no feedback. Focus data silently lost.

**File:** `src/app/mod.rs:620`

**Fix:**
1. Show transient notification on save failure
2. Keep failed session in memory with retry option
3. Log with full error context

**Acceptance Criteria:**
- [ ] User sees notification when focus session save fails
- [ ] Error includes actionable information

---

#### 1.5 Git Operation Error Context (P1 - High)

**Problem:** When all git ref lookups fail in `resolve_ref_to_commit()`, returns generic error without context about what was tried.

**File:** `src/git/worktree.rs:102-130`

**Fix:**
1. Collect all attempted ref lookup methods
2. Return combined error message listing all failed approaches
3. Include original ref name in error

**Acceptance Criteria:**
- [ ] Error message shows all lookup methods attempted
- [ ] User can understand why ref resolution failed

---

#### 1.6 Project Store Corruption Handling (P2 - Medium)

**Problem:** If project store file is corrupted, all projects are lost with only a warning log.

**File:** `src/app/mod.rs:73-76`

**Fix:**
1. Before discarding corrupted store, backup to `projects.json.corrupt.{timestamp}`
2. Show prominent notification to user
3. Log the corruption error with full details

**Acceptance Criteria:**
- [ ] Corrupted store is backed up, not deleted
- [ ] User sees clear notification about data recovery

---

### Part 2: Input Handling Edge Cases

#### 2.1 Race Condition: Index Out of Bounds After Collection Changes (P0 - Critical)

**Problem:** Multiple places use `selected_index` to access collections without re-validating after potential state changes. Sessions/projects can be destroyed between `.len()` check and `.get()` access.

**Affected Files:**
- `src/input/normal/branch_detail.rs:45, 65-74`
- `src/input/normal/projects_overview.rs:108-111, 156-193`
- `src/input/normal/session_view.rs:102-120`
- `src/input/normal/project_detail.rs:53-56`
- `src/input/normal/timeline.rs`

**Fix Pattern:**
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

**Specific Fixes:**
1. **branch_detail.rs**: Validate session exists before showing delete dialog
2. **projects_overview.rs**: Re-fetch project list atomically with access
3. **session_view.rs**: Handle destroyed session during Tab navigation
4. **timeline.rs**: Bounds-check before timeline navigation

**Acceptance Criteria:**
- [ ] All index-based accesses use checked methods (.get() not direct indexing)
- [ ] Index is reset when collection shrinks below current index
- [ ] No panic possible from index out of bounds

---

#### 2.2 Worktree Wizard Index Synchronization (P0 - Critical)

**Problem:** If `filtered_branches` is recalculated while user navigates, `list_index` can be out of bounds for new list.

**File:** `src/wizards/worktree/handlers.rs:53-83, 188-210`

**Fix:**
1. Clamp `list_index` whenever `filtered_branches` changes
2. Add helper function `clamp_wizard_index()` called after any filter operation
3. Use checked indexing for all wizard list accesses

**Acceptance Criteria:**
- [ ] `list_index` is always valid for current `filtered_branches`
- [ ] Typing to filter doesn't cause index panic
- [ ] Navigation wraps correctly at boundaries

---

#### 2.3 Paste Input Length Limits (P1 - High)

**Problem:** No length limits on pasted text. User can paste massive content causing memory exhaustion.

**Files:**
- `src/input/text_input.rs:254`
- `src/app/mod.rs:341-345`

**Fix:**
1. Define maximum lengths per field:
   - Project path: 4096 characters
   - Session name: 256 characters
   - Branch name: 256 characters
2. Truncate paste input to limit with warning notification
3. Add length validation in character input handlers too

**Acceptance Criteria:**
- [ ] All text input fields have defined max lengths
- [ ] Paste operations truncate and warn
- [ ] Memory usage bounded regardless of clipboard content

---

#### 2.4 Active Session Reference Validation (P1 - High)

**Problem:** `active_session` can reference a destroyed session. Operations on it silently fail or cause undefined behavior.

**Files:**
- `src/app/mod.rs:359-365` (paste handling)
- `src/input/normal/session_view.rs`
- `src/input/session_mode.rs`

**Fix:**
1. When session is destroyed, check if it's the active session and clear `active_session`
2. Add validation before any operation on `active_session`
3. Navigate back to parent view if active session is destroyed

**Acceptance Criteria:**
- [ ] Destroying active session clears reference and navigates away
- [ ] No operations attempted on destroyed sessions
- [ ] User sees clear feedback when session exits

---

#### 2.5 Input Mode / View State Consistency (P2 - Medium)

**Problem:** Input mode can become inconsistent with current view if state changes during input handling.

**File:** `src/input/dispatcher.rs`

**Fix:**
1. Add validation that input mode is appropriate for current view
2. If mismatch detected, reset to Normal mode and log warning
3. Consider making view transitions atomic with mode changes

**Acceptance Criteria:**
- [ ] Stray keystrokes in wrong mode don't cause issues
- [ ] Mode/view mismatches are detected and corrected

---

#### 2.6 Path Completion Index Bounds (P2 - Medium)

**Problem:** `path_completion_index` can be out of bounds if completions list shrinks.

**File:** `src/input/text_input.rs:135-155`

**Fix:**
1. Clamp `path_completion_index` whenever `path_completions` is recalculated
2. Reset index to 0 when completions change significantly

**Acceptance Criteria:**
- [ ] Tab completion never accesses out-of-bounds
- [ ] Index reset when completion list changes

---

#### 2.7 Delete Confirmation with Stale References (P2 - Medium)

**Problem:** Item could be deleted between showing confirmation dialog and user pressing 'y'.

**Files:**
- `src/input/dialogs.rs:54-87`
- `src/input/dialogs.rs:110-143`

**Fix:**
1. Re-validate that item still exists when confirmation received
2. Show "Item no longer exists" message if already deleted
3. Cancel dialog gracefully

**Acceptance Criteria:**
- [ ] Confirming delete of already-deleted item shows helpful message
- [ ] No panic or silent failure

---

### Part 3: Performance Optimization

#### 3.1 Benchmark with 20+ Sessions (P1 - High)

**Goal:** Verify application remains responsive with many concurrent sessions.

**Tasks:**
1. Create test script to spawn N sessions programmatically
2. Measure:
   - Input latency (key press to screen update)
   - Memory usage per session
   - CPU usage during active sessions
   - Hook event processing throughput
3. Profile with `cargo flamegraph` to identify bottlenecks
4. Document performance characteristics

**Acceptance Criteria:**
- [ ] Input latency < 50ms with 20 sessions
- [ ] Memory usage documented and reasonable
- [ ] No UI freezes during heavy activity

---

#### 3.2 Hook Event Processing Optimization (P2 - Medium)

**Problem:** Hook events are processed sequentially. High event rate could cause backlog.

**File:** `src/app/mod.rs:221-262`

**Tasks:**
1. Measure hook event processing time
2. Consider batch processing of queued events
3. Add dropped event monitoring and alerting
4. Implement event coalescing for rapid state changes

**Acceptance Criteria:**
- [ ] Hook processing doesn't block UI rendering
- [ ] Dropped events are tracked and reported
- [ ] Burst of events handled gracefully

---

#### 3.3 Session Output Buffer Optimization (P3 - Low)

**File:** `src/session/mod.rs` (OutputBuffer)

**Tasks:**
1. Profile memory usage of output ring buffer
2. Consider lazy rendering (only parse visible portion)
3. Measure impact of VTerm processing on large outputs

**Acceptance Criteria:**
- [ ] Memory usage per session documented
- [ ] Large outputs don't cause lag

---

### Part 4: Edge Case Handling

#### 4.1 Claude Code Process Crashes (P1 - High)

**Problem:** When Claude Code crashes unexpectedly, need clear user feedback and cleanup.

**Tasks:**
1. Detect abnormal process termination (non-zero exit, signal)
2. Show notification with exit reason
3. Offer to restart session or view last output
4. Clean up any orphaned resources

**Acceptance Criteria:**
- [ ] Crashed sessions show clear error state
- [ ] Exit reason displayed (signal, exit code)
- [ ] User can see last output before crash

---

#### 4.2 Hook Server Port Conflicts (P1 - High)

**Problem:** If port 9999 is in use, hook server fails silently.

**File:** `src/hooks/server.rs`

**Tasks:**
1. Check port availability before binding
2. Show clear error if port in use
3. Consider auto-selecting available port with fallback
4. Document port configuration option

**Acceptance Criteria:**
- [ ] Port conflict shows clear error message
- [ ] User knows how to resolve (config or kill other process)

---

#### 4.3 Git Repository State Changes (P2 - Medium)

**Problem:** External git operations (branch deleted, worktree removed) can invalidate app state.

**Tasks:**
1. Validate git state before operations
2. Handle missing worktree directories gracefully
3. Refresh branch list on demand
4. Show warning for orphaned sessions

**Acceptance Criteria:**
- [ ] External branch deletion doesn't crash app
- [ ] Missing worktrees are detected and reported
- [ ] User can manually refresh git state

---

#### 4.4 Terminal Resize During Operations (P2 - Medium)

**Problem:** Terminal resize during session operations could cause rendering issues.

**Tasks:**
1. Audit resize handling in all views
2. Ensure PTY resize is propagated correctly
3. Test rapid resize scenarios

**Acceptance Criteria:**
- [ ] Resize during any operation doesn't crash
- [ ] Session PTY receives correct new size

---

#### 4.5 Disk Full / Permission Errors (P3 - Low)

**Problem:** Disk full or permission errors during persistence operations.

**Tasks:**
1. Handle disk full error when saving projects/config
2. Handle permission denied errors
3. Show actionable error messages

**Acceptance Criteria:**
- [ ] Disk full shows clear message
- [ ] Permission errors explained to user

---

### Implementation Priority Order

#### Sprint 1: Critical Safety (Recommended First)
1. 2.1 - Index out of bounds race conditions
2. 2.2 - Worktree wizard index synchronization
3. 1.1 - Hook server crash detection
4. 2.3 - Paste input length limits

#### Sprint 2: Error Visibility
5. 1.2 - File logging failure visibility
6. 1.3 - TUI teardown error logging
7. 1.4 - Focus session persistence feedback
8. 4.1 - Claude Code process crashes

#### Sprint 3: Edge Cases
9. 2.4 - Active session reference validation
10. 4.2 - Hook server port conflicts
11. 1.5 - Git operation error context
12. 1.6 - Project store corruption handling

#### Sprint 4: Polish & Performance
13. 3.1 - Benchmark with 20+ sessions
14. 2.5 - Input mode / view state consistency
15. 3.2 - Hook event processing optimization
16. 4.3 - Git repository state changes

#### Sprint 5: Remaining Items
17. 2.6 - Path completion index bounds
18. 2.7 - Delete confirmation with stale references
19. 4.4 - Terminal resize during operations
20. 4.5 - Disk full / permission errors
21. 3.3 - Session output buffer optimization

---

### Testing Strategy

#### Unit Tests
- Add tests for bounds checking in all input handlers
- Test error paths in persistence operations
- Test index clamping helpers

#### Integration Tests
- Spawn/destroy sessions rapidly
- Simulate hook server failure
- Test with corrupted config/project files

#### Manual Testing Checklist
- [ ] Type unexpected characters in all input modes
- [ ] Delete items while dialogs are open
- [ ] Paste very large content
- [ ] Kill Claude Code process externally
- [ ] Run with port 9999 already in use
- [ ] Remove worktree directory externally
- [ ] Resize terminal rapidly during operations
- [ ] Run with 20+ concurrent sessions

---

### Success Metrics

1. **Zero panics** from user input in any mode
2. **All errors** visible to user or logged
3. **< 50ms** input latency with 20 sessions
4. **Graceful degradation** when external resources fail
5. **Clear feedback** for all failure scenarios

---

### Why This Phase Third

Functionality before polish. Phase 1 and 2 build the features; Phase 3 makes them pleasant to use. This includes handling edge cases (what if Claude Code crashes?), giving visual feedback, and ensuring performance at scale.

### Deliverable

A release candidate that:
- Handles all known edge cases gracefully
- Has zero panics from user input
- Stays responsive with 20+ sessions (< 50ms input latency)
- Provides clear feedback for all error scenarios
- Has comprehensive error logging for debugging

---

## Phase 4: Documentation & Distribution

**Goal**: Make Panoptes easy to install and use.

### What We're Building

- **README** - Quick start guide, feature overview
- **Keyboard reference** - Complete shortcut documentation
- **Configuration guide** - All available settings explained
- **Troubleshooting** - Common issues and solutions
- **Distribution** - cargo install, Homebrew formula

### Why This Phase Last

Documentation and packaging come after the product is stable. Writing docs for a moving target creates maintenance burden. Once Phase 3 is complete, we document what exists.

### Deliverable

Version 1.0 release with:
- Comprehensive documentation
- Easy installation via cargo or Homebrew
- Clear upgrade path for future versions

---

## Summary

| Phase | Focus | Status |
|-------|-------|--------|
| 1 | Core prototype | ✅ Complete |
| 2 | Organization & Git | ✅ Complete |
| 3 | Quality & Polish | In Progress |
| 4 | Release | Planned |

Each phase builds on the previous one. We validate risky technical assumptions early, then layer on organization, polish, and finally documentation.
