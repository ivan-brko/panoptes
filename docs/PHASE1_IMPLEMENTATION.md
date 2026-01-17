# Phase 1 Implementation Tracker

## Goal
Build a working prototype that can spawn and interact with multiple Claude Code sessions with real-time state tracking via hooks.

## Success Criteria
- Can create 2-3 Claude Code sessions
- Can switch between sessions with keyboard shortcuts
- See real-time state updates (Starting → Thinking → Executing → Waiting)
- Basic TUI displays session output and status

---

## Tickets

### Ticket 1: Project Setup
**Description**: Initialize the Rust project with all required dependencies and module structure.

**Tasks**:
- [x] Create `Cargo.toml` with dependencies
- [x] Create directory structure (`src/`, subdirectories)
- [x] Create module files with placeholder `mod.rs` declarations
- [x] Verify project compiles with `cargo build`

**Dependencies**: None

**Files to create**:
```
Cargo.toml
src/
├── main.rs
├── app.rs
├── config.rs
├── tui/
│   └── mod.rs
├── session/
│   └── mod.rs
├── agent/
│   └── mod.rs
└── hooks/
    └── mod.rs
```

---

### Ticket 2: Configuration Module
**Description**: Implement configuration loading and directory management.

**Tasks**:
- [x] Define `Config` struct with settings (hook port, paths, limits)
- [x] Implement config directory creation (`~/.panoptes/`)
- [x] Implement subdirectory creation (hooks/, worktrees/)
- [x] Add config file loading/saving (TOML)

**Dependencies**: Ticket 1

**Files to create/modify**:
- `src/config.rs`

---

### Ticket 3: PTY Management
**Description**: Implement PTY spawning and I/O handling using portable-pty.

**Tasks**:
- [x] Create `PtyHandle` struct wrapping portable-pty
- [x] Implement `spawn()` - create PTY with command, args, working dir, env
- [x] Implement `write()` - send raw bytes to PTY
- [x] Implement `send_key()` - convert KeyEvent to terminal escape sequences
- [x] Implement `try_read()` - non-blocking read from PTY
- [x] Implement `resize()` - handle terminal resize
- [x] Implement `is_alive()` - check if child process running
- [x] Implement `kill()` - terminate child process

**Dependencies**: Ticket 1

**Files to create/modify**:
- `src/session/pty.rs`
- `src/session/mod.rs`

---

### Ticket 4: Session Data Structures
**Description**: Define core session types and state management.

**Tasks**:
- [ ] Define `SessionId` type (UUID)
- [ ] Define `SessionState` enum (Starting, Thinking, Executing, Waiting, Idle, Exited)
- [ ] Implement state display helpers (name, color)
- [ ] Define `Session` struct with all fields
- [ ] Implement output buffer management (bounded ring buffer)
- [ ] Implement scroll handling for output display

**Dependencies**: Ticket 3

**Files to create/modify**:
- `src/session/mod.rs`

---

### Ticket 5: Agent Adapter Layer
**Description**: Define the agent abstraction trait and implement Claude Code adapter.

**Tasks**:
- [ ] Define `AgentAdapter` trait with core methods
- [ ] Implement `ClaudeCodeAdapter` struct
- [ ] Implement spawn logic for Claude Code
- [ ] Implement hooks configuration generation

**Dependencies**: Ticket 3

**Files to create/modify**:
- `src/agent/mod.rs`
- `src/agent/adapter.rs`
- `src/agent/claude.rs`

---

### Ticket 6: Hook Server
**Description**: Implement Axum HTTP server to receive Claude Code hook callbacks.

**Tasks**:
- [ ] Define `HookEvent` struct for incoming events
- [ ] Create Axum router with `/hook` POST endpoint
- [ ] Parse JSON payload (session_id, event, tool, timestamp)
- [ ] Send events through channel to main app
- [ ] Handle server startup on configurable port

**Dependencies**: Ticket 1, Ticket 2

**Files to create/modify**:
- `src/hooks/mod.rs`
- `src/hooks/server.rs`

---

### Ticket 7: Hook Scripts
**Description**: Create shell scripts that Claude Code will execute on hook events.

**Tasks**:
- [ ] Create `state-update.sh` script template
- [ ] Parse JSON from stdin using jq
- [ ] Extract session_id, event, tool fields
- [ ] POST to localhost hook server
- [ ] Implement script installation to `~/.panoptes/hooks/`
- [ ] Make scripts executable

**Dependencies**: Ticket 6

**Files to create/modify**:
- `src/session/manager.rs` (script installation logic)
- Script template in code

---

### Ticket 8: Session Manager
**Description**: Implement session lifecycle management.

**Tasks**:
- [ ] Create `SessionManager` struct
- [ ] Implement `create_session()` - spawn Claude Code with hooks
- [ ] Implement `destroy_session()` - kill PTY, cleanup
- [ ] Implement `poll_outputs()` - read from all session PTYs
- [ ] Handle session state updates from hooks
- [ ] Track session order for navigation

**Dependencies**: Ticket 4, Ticket 5, Ticket 7

**Files to create/modify**:
- `src/session/manager.rs`
- `src/session/mod.rs`

---

### Ticket 9: Application State
**Description**: Implement central application state and event handling.

**Tasks**:
- [ ] Define `InputMode` enum (Normal, Session)
- [ ] Define `View` enum (SessionList, SessionView)
- [ ] Define `AppState` struct with all state fields
- [ ] Implement session selection helpers (next, prev, by number)
- [ ] Define `App` struct tying everything together
- [ ] Implement main event loop (keyboard + hooks + PTY output)

**Dependencies**: Ticket 6, Ticket 8

**Files to create/modify**:
- `src/app.rs`

---

### Ticket 10: Basic TUI - Framework
**Description**: Set up Ratatui terminal handling and basic rendering infrastructure.

**Tasks**:
- [ ] Create `Tui` struct wrapping terminal setup/teardown
- [ ] Implement `enter()` - enable raw mode, alternate screen
- [ ] Implement `exit()` - restore terminal state
- [ ] Implement basic `draw()` method signature
- [ ] Handle panic cleanup (restore terminal on crash)

**Dependencies**: Ticket 1

**Files to create/modify**:
- `src/tui/mod.rs`

---

### Ticket 11: Basic TUI - Session List View
**Description**: Render the session list/overview screen.

**Tasks**:
- [ ] Layout: header, session list, footer/help
- [ ] Render session items with name, state, state color
- [ ] Highlight selected session
- [ ] Show session creation prompt when active
- [ ] Display keyboard shortcuts in footer

**Dependencies**: Ticket 10, Ticket 9

**Files to create/modify**:
- `src/tui/mod.rs`
- `src/tui/render.rs`

---

### Ticket 12: Basic TUI - Session View
**Description**: Render fullscreen session view with PTY output.

**Tasks**:
- [ ] Layout: header (breadcrumb, state), output area, input mode indicator
- [ ] Render PTY output lines with basic ANSI color support
- [ ] Show scroll position if not at bottom
- [ ] Display input mode (Normal vs Session) clearly
- [ ] Handle output area sizing based on terminal dimensions

**Dependencies**: Ticket 11

**Files to create/modify**:
- `src/tui/mod.rs`
- `src/tui/render.rs`

---

### Ticket 13: Input Handling
**Description**: Implement keyboard input routing and handling.

**Tasks**:
- [ ] Handle global keys (Ctrl+C to quit)
- [ ] Session List view: n (new), j/k (nav), Enter (activate), d (delete), q (quit)
- [ ] Session View (Normal mode): Esc (back), Tab (next), 1-9 (jump), i/Enter (session mode)
- [ ] Session View (Session mode): Esc (exit to normal), all other keys → PTY
- [ ] Session name input: character input, backspace, enter, escape

**Dependencies**: Ticket 9, Ticket 11, Ticket 12

**Files to create/modify**:
- `src/app.rs` (key handling in event loop)
- `src/tui/input.rs` (optional, or inline in app.rs)

---

### Ticket 14: Integration & Testing
**Description**: Wire everything together and test the complete flow.

**Tasks**:
- [ ] Verify `cargo build --release` succeeds
- [ ] Test: Launch panoptes
- [ ] Test: Create a session (press n, enter name)
- [ ] Test: Verify Claude Code spawns in PTY
- [ ] Test: Send a prompt, verify state changes visible
- [ ] Test: Create second session, verify switching works
- [ ] Test: Verify hook callbacks update state correctly
- [ ] Fix any bugs discovered during testing

**Dependencies**: All previous tickets

**Files to modify**: As needed based on testing

---

## Implementation Order

```
Ticket 1 (Project Setup)
    │
    ├──► Ticket 2 (Config)
    │
    ├──► Ticket 3 (PTY) ──► Ticket 4 (Session Data) ──► Ticket 8 (Session Manager)
    │                                                          │
    ├──► Ticket 5 (Agent Adapter) ─────────────────────────────┤
    │                                                          │
    ├──► Ticket 6 (Hook Server) ──► Ticket 7 (Hook Scripts) ───┤
    │                                                          │
    └──► Ticket 10 (TUI Framework) ──► Ticket 11 (List View) ──┼──► Ticket 9 (App State)
                                              │                │           │
                                              └──► Ticket 12 (Session View)│
                                                         │                 │
                                                         └──► Ticket 13 (Input)
                                                                    │
                                                                    ▼
                                                         Ticket 14 (Integration)
```

## Progress

| Ticket | Status | Notes |
|--------|--------|-------|
| 1. Project Setup | ✅ Complete | Cargo.toml, modules, 16 tests passing |
| 2. Configuration | ✅ Complete | Config struct, dirs, TOML load/save |
| 3. PTY Management | ✅ Complete | PtyHandle with spawn, write, send_key, try_read, resize, is_alive, kill |
| 4. Session Data | Not Started | |
| 5. Agent Adapter | Not Started | |
| 6. Hook Server | Not Started | |
| 7. Hook Scripts | Not Started | |
| 8. Session Manager | Not Started | |
| 9. Application State | Not Started | |
| 10. TUI Framework | Not Started | |
| 11. TUI Session List | Not Started | |
| 12. TUI Session View | Not Started | |
| 13. Input Handling | Not Started | |
| 14. Integration | Not Started | |
