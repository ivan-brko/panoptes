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
- [x] Define `SessionId` type (UUID)
- [x] Define `SessionState` enum (Starting, Thinking, Executing, Waiting, Idle, Exited)
- [x] Implement state display helpers (name, color)
- [x] Define `Session` struct with all fields
- [x] Implement output buffer management (bounded ring buffer)
- [x] Implement scroll handling for output display

**Dependencies**: Ticket 3

**Files to create/modify**:
- `src/session/mod.rs`

---

### Ticket 5: Agent Adapter Layer
**Description**: Define the agent abstraction trait and implement Claude Code adapter.

**Tasks**:
- [x] Define `AgentAdapter` trait with core methods
- [x] Implement `ClaudeCodeAdapter` struct
- [x] Implement spawn logic for Claude Code
- [x] Implement hooks configuration generation

**Dependencies**: Ticket 3

**Files to create/modify**:
- `src/agent/mod.rs`
- `src/agent/adapter.rs`
- `src/agent/claude.rs`

---

### Ticket 6: Hook Server
**Description**: Implement Axum HTTP server to receive Claude Code hook callbacks.

**Tasks**:
- [x] Define `HookEvent` struct for incoming events
- [x] Create Axum router with `/hook` POST endpoint
- [x] Parse JSON payload (session_id, event, tool, timestamp)
- [x] Send events through channel to main app
- [x] Handle server startup on configurable port

**Dependencies**: Ticket 1, Ticket 2

**Files to create/modify**:
- `src/hooks/mod.rs`
- `src/hooks/server.rs`

---

### Ticket 7: Hook Scripts
**Description**: Create shell scripts that Claude Code will execute on hook events.

**Tasks**:
- [x] Create `state-update.sh` script template
- [x] Parse JSON from stdin using jq
- [x] Extract session_id, event, tool fields
- [x] POST to localhost hook server
- [x] Implement script installation to `~/.panoptes/hooks/`
- [x] Make scripts executable

**Dependencies**: Ticket 6

**Files to create/modify**:
- `src/session/manager.rs` (script installation logic)
- Script template in code

---

### Ticket 8: Session Manager
**Description**: Implement session lifecycle management.

**Tasks**:
- [x] Create `SessionManager` struct
- [x] Implement `create_session()` - spawn Claude Code with hooks
- [x] Implement `destroy_session()` - kill PTY, cleanup
- [x] Implement `poll_outputs()` - read from all session PTYs
- [x] Handle session state updates from hooks
- [x] Track session order for navigation

**Dependencies**: Ticket 4, Ticket 5, Ticket 7

**Files to create/modify**:
- `src/session/manager.rs`
- `src/session/mod.rs`

---

### Ticket 9: Application State
**Description**: Implement central application state and event handling.

**Tasks**:
- [x] Define `InputMode` enum (Normal, Session)
- [x] Define `View` enum (SessionList, SessionView)
- [x] Define `AppState` struct with all state fields
- [x] Implement session selection helpers (next, prev, by number)
- [x] Define `App` struct tying everything together
- [x] Implement main event loop (keyboard + hooks + PTY output)

**Dependencies**: Ticket 6, Ticket 8

**Files to create/modify**:
- `src/app.rs`

---

### Ticket 10: Basic TUI - Framework
**Description**: Set up Ratatui terminal handling and basic rendering infrastructure.

**Tasks**:
- [x] Create `Tui` struct wrapping terminal setup/teardown
- [x] Implement `enter()` - enable raw mode, alternate screen
- [x] Implement `exit()` - restore terminal state
- [x] Implement basic `draw()` method signature
- [x] Handle panic cleanup (restore terminal on crash)

**Dependencies**: Ticket 1

**Files to create/modify**:
- `src/tui/mod.rs`

---

### Ticket 11: Basic TUI - Session List View
**Description**: Render the session list/overview screen.

**Tasks**:
- [x] Layout: header, session list, footer/help
- [x] Render session items with name, state, state color
- [x] Highlight selected session
- [x] Show session creation prompt when active
- [x] Display keyboard shortcuts in footer

**Dependencies**: Ticket 10, Ticket 9

**Files to create/modify**:
- `src/tui/mod.rs`
- `src/tui/render.rs`

---

### Ticket 12: Basic TUI - Session View
**Description**: Render fullscreen session view with PTY output.

**Tasks**:
- [x] Layout: header (breadcrumb, state), output area, input mode indicator
- [x] Render PTY output lines with basic ANSI color support
- [x] Show scroll position if not at bottom
- [x] Display input mode (Normal vs Session) clearly
- [x] Handle output area sizing based on terminal dimensions

**Dependencies**: Ticket 11

**Files to create/modify**:
- `src/tui/mod.rs`
- `src/tui/render.rs`

---

### Ticket 13: Input Handling
**Description**: Implement keyboard input routing and handling.

**Tasks**:
- [x] Handle global keys (Ctrl+C to quit)
- [x] Session List view: n (new), j/k (nav), Enter (activate), d (delete), q (quit)
- [x] Session View (Normal mode): Esc (back), Tab (next), 1-9 (jump), i/Enter (session mode)
- [x] Session View (Session mode): Esc (exit to normal), all other keys → PTY
- [x] Session name input: character input, backspace, enter, escape

**Dependencies**: Ticket 9, Ticket 11, Ticket 12

**Files to create/modify**:
- `src/app.rs` (key handling in event loop)
- `src/tui/input.rs` (optional, or inline in app.rs)

---

### Ticket 14: Integration & Testing
**Description**: Wire everything together and test the complete flow.

**Tasks**:
- [x] Verify `cargo build --release` succeeds
- [x] Test: Launch panoptes
- [x] Test: Create a session (press n, enter name)
- [x] Test: Verify Claude Code spawns in PTY
- [x] Test: Send a prompt, verify state changes visible
- [x] Test: Create second session, verify switching works
- [x] Test: Verify hook callbacks update state correctly
- [x] Fix any bugs discovered during testing

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
| 1. Project Setup | ✅ Complete | Cargo.toml, modules |
| 2. Configuration | ✅ Complete | Config struct, dirs, TOML load/save |
| 3. PTY Management | ✅ Complete | PtyHandle with spawn, write, send_key, try_read, resize, is_alive, kill |
| 4. Session Data | ✅ Complete | Session, OutputBuffer with scroll |
| 5. Agent Adapter | ✅ Complete | AgentAdapter trait, ClaudeCodeAdapter with spawn and hooks |
| 6. Hook Server | ✅ Complete | Axum server, POST /hook endpoint, mpsc channel, graceful shutdown |
| 7. Hook Scripts | ✅ Complete | Implemented in ClaudeCodeAdapter (generate_hook_script, install, settings) |
| 8. Session Manager | ✅ Complete | SessionManager with create/destroy, poll_outputs, hook handling |
| 9. Application State | ✅ Complete | InputMode, View, AppState, App with full event loop and TUI rendering |
| 10. TUI Framework | ✅ Complete | Tui struct with enter/exit, panic cleanup, alternate screen |
| 11. TUI Session List | ✅ Complete | Session list with state colors, selection highlighting, help footer |
| 12. TUI Session View | ✅ Complete | Full PTY output rendering with vterm color support, scroll indicator |
| 13. Input Handling | ✅ Complete | All keyboard shortcuts, Normal/Session mode switching |
| 14. Integration | ✅ Complete | 69 tests passing, session cleanup on exit |

**Phase 1 Complete** - All tickets implemented and tested.
