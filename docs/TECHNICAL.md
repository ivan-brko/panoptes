# Panoptes Technical Stack

## Language

**Rust** - Chosen for its performance, safety guarantees, and excellent ecosystem for terminal applications. Rust's async capabilities with Tokio make it well-suited for managing multiple concurrent sessions.

## Core Dependencies

### Terminal UI

**Ratatui** (v0.26) - A Rust library for building rich terminal user interfaces. Provides widgets, layouts, and rendering primitives for creating the dashboard interface.

**Crossterm** (v0.27) - Cross-platform terminal manipulation library. Handles raw mode input, ANSI escape sequences, and terminal events. Works on macOS, Linux, and Windows.

### Async Runtime

**Tokio** (v1) - Async runtime for Rust. Powers the concurrent handling of multiple PTY sessions, the HTTP hook server, and event processing. Used with full features enabled.

### HTTP Server

**Axum** (v0.7) - Ergonomic web framework built on Tokio. Runs a local HTTP server (port 9999) that receives state updates from Claude Code's hook system.

### PTY Management

**portable-pty** (v0.8) - Cross-platform pseudo-terminal library. Spawns Claude Code processes in PTYs, enabling full terminal emulation with proper I/O handling and resize support.

### Git Integration

**git2** (v0.18) - Rust bindings to libgit2. Used for detecting git repositories, managing worktrees, and handling branch operations.

### Serialization

**Serde** (v1) with `derive` feature - Serialization framework for Rust. Used for JSON parsing of hook events and configuration files.

**serde_json** (v1) - JSON support for Serde. Parses hook payloads from Claude Code.

**TOML** (v0.8) - TOML format support. Used for configuration and project persistence files.

### Error Handling

**anyhow** (v1) - Flexible error handling for applications. Provides context-rich error propagation.

**thiserror** (v1) - Derive macro for custom error types. Used for domain-specific errors.

### Utilities

**dirs** (v5) - Platform-specific directory paths. Locates home directory for config storage.

**chrono** (v0.4) - Date and time library. Timestamps for session activity tracking.

**uuid** (v4) - UUID generation. Creates unique identifiers for sessions, projects, and branches.

**tracing** (v0.1) with **tracing-subscriber** - Structured logging framework. Debug and diagnostic output.

**shellexpand** (v3) - Shell-like tilde expansion for paths.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│ Panoptes Process                                                │
│                                                                 │
│  ┌──────────────┐    ┌─────────────────┐   ┌─────────────────┐ │
│  │ TUI Layer    │◄───│ Application     │◄──│ HTTP Hook       │ │
│  │ (Ratatui)    │    │ State           │   │ Server (Axum)   │ │
│  │              │    │                 │   │                 │ │
│  │ Views:       │    │ - AppState      │   │ Receives state  │ │
│  │ - Projects   │    │ - View enum     │   │ updates from    │ │
│  │ - Project    │    │ - InputMode     │   │ Claude Code     │ │
│  │ - Branch     │    │                 │   │ hooks           │ │
│  │ - Session    │    │                 │   │                 │ │
│  │ - Timeline   │    │                 │   │                 │ │
│  └──────────────┘    └────────┬────────┘   └────────┬────────┘ │
│         │                     │                     │          │
│         │            ┌────────┴────────┐            │          │
│         │            │ Session         │            │          │
│         │            │ Manager         │◄───────────┘          │
│         │            │                 │                       │
│         │            │ - Attention     │                       │
│         │            │   tracking      │                       │
│         │            │ - State updates │                       │
│         │            └────────┬────────┘                       │
│         │                     │                                │
│         │            ┌────────┴────────┐                       │
│         │            │ Project         │                       │
│         │            │ Store           │                       │
│         │            │                 │                       │
│         │            │ - Projects      │                       │
│         │            │ - Branches      │                       │
│         │            │ - Persistence   │                       │
│         │            └────────┬────────┘                       │
│         ▼                     │                                │
│  ┌──────────────┐    ┌────────┴────────┐                       │
│  │ VTerm        │    │ Agent           │                       │
│  │ (ANSI/color) │    │ Adapter         │                       │
│  └──────┬───────┘    └────────┬────────┘                       │
│         │                     │                                │
│  ┌──────┴───────┐             │                                │
│  │ PTY Manager  │◄────────────┘                                │
│  │ (portable-   │                                              │
│  │  pty)        │                                              │
│  └──────┬───────┘                                              │
└─────────┼──────────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────────────────────┐
│ Claude Code Instances (Child Processes)                         │
│                                                                 │
│  Each instance runs in its own PTY and sends hook events        │
│  to the HTTP server when state changes occur                    │
└─────────────────────────────────────────────────────────────────┘
```

## Communication Flow

### User Input
1. Crossterm captures keyboard events
2. Application routes based on mode (Normal/Session) and view
3. In Session mode, keystrokes are written to the PTY

### Session Output
1. PTY reader captures output from Claude Code
2. Output is buffered (ring buffer, max 10K lines by default)
3. TUI renders visible portion with ANSI color support

### State Updates (Hooks)
1. Claude Code executes hook scripts on events
2. Hook script extracts session_id from JSON stdin
3. Hook script POSTs event to localhost:9999
4. Axum server updates session state
5. SessionManager tracks attention flags
6. TUI reflects new state on next render

### Attention Flow
1. Session transitions to "Waiting" state
2. SessionManager sets `needs_attention` flag
3. If session is not currently active, terminal bell rings
4. Session displays attention badge (green or yellow based on idle time)
5. When user opens session, attention is acknowledged

## File Locations

| Path | Purpose |
|------|---------|
| `~/.panoptes/config.toml` | User configuration |
| `~/.panoptes/projects.json` | Project and branch persistence |
| `~/.panoptes/hooks/` | Hook scripts for Claude Code |
| `~/.panoptes/worktrees/` | Git worktrees for branch isolation |

## Configuration

| Setting | Default | Description |
|---------|---------|-------------|
| `hook_port` | 9999 | Port for the HTTP hook server |
| `max_output_lines` | 10,000 | Lines kept in output buffer per session |
| `idle_threshold_secs` | 300 | Seconds before waiting session shows yellow attention badge |

## Platform Support

Primary target: **macOS** (development platform)

Secondary: **Linux** (should work with no changes)

Windows: Possible with portable-pty, but untested.

## Session Lifecycle

Sessions are cleaned up automatically when Panoptes exits:
- All child processes (Claude Code instances) are terminated
- PTY handles are closed
- No orphaned processes are left behind

## Testing

The project has 108 unit tests covering:
- Configuration loading/saving
- Session state transitions
- Output buffer management
- Hook event parsing
- PTY operations
- VTerm ANSI parsing
- Project/branch management
- Navigation state machine

Run tests with: `cargo test`
