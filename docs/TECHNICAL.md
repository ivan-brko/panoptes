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

**TOML** (v0.8) - TOML format support. Used for the configuration file (`~/.panoptes/config.toml`).

### Error Handling

**anyhow** (v1) - Flexible error handling for applications. Provides context-rich error propagation.

**thiserror** (v1) - Derive macro for custom error types. Used for domain-specific errors.

### Utilities

**dirs** (v5) - Platform-specific directory paths. Locates home directory for config storage.

**chrono** (v0.4) - Date and time library. Timestamps for session activity tracking.

**uuid** (v4) - UUID generation. Creates unique session identifiers.

**tracing** (v0.1) with **tracing-subscriber** - Structured logging framework. Debug and diagnostic output.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│ Panoptes Process                                            │
│                                                             │
│  ┌──────────────┐    ┌─────────────────┐   ┌─────────────┐ │
│  │ TUI Layer    │◄───│ Application     │◄──│ HTTP Hook   │ │
│  │ (Ratatui)    │    │ State           │   │ Server      │ │
│  └──────────────┘    └─────────────────┘   │ (Axum)      │ │
│         │                    │             └──────┬──────┘ │
│         │            ┌───────┴───────┐            │        │
│         │            │ Session       │            │        │
│         │            │ Manager       │            │        │
│         │            └───────┬───────┘            │        │
│         ▼                    │                    │        │
│  ┌──────────────┐    ┌───────┴───────┐            │        │
│  │ PTY Manager  │◄───│ Agent         │            │        │
│  │ (portable-   │    │ Adapter       │            │        │
│  │  pty)        │    └───────────────┘            │        │
│  └──────┬───────┘                                 │        │
└─────────┼─────────────────────────────────────────┼────────┘
          │                                         │
          ▼                                         │
┌─────────────────────────────────────────────────────────────┐
│ Claude Code Instances (Child Processes)                     │
│                                                             │
│  Each instance runs in its own PTY and sends hook events    │
│  to the HTTP server when state changes occur                │
└─────────────────────────────────────────────────────────────┘
```

## Communication Flow

### User Input
1. Crossterm captures keyboard events
2. Application routes based on mode (Normal/Session)
3. In Session mode, keystrokes are written to the PTY

### Session Output
1. PTY reader captures output from Claude Code
2. Output is buffered (ring buffer, max 10K lines)
3. TUI renders visible portion with ANSI color support

### State Updates (Hooks)
1. Claude Code executes hook scripts on events
2. Hook script extracts session_id from JSON stdin
3. Hook script POSTs event to localhost:9999
4. Axum server updates session state
5. TUI reflects new state on next render

## File Locations

| Path | Purpose |
|------|---------|
| `~/.panoptes/config.toml` | User configuration |
| `~/.panoptes/hooks/` | Hook scripts for Claude Code |
| `~/.panoptes/worktrees/` | Git worktrees for branch isolation |

## Platform Support

Primary target: **macOS** (development platform)

Secondary: **Linux** (should work with no changes)

Windows: Possible with portable-pty, but untested.
