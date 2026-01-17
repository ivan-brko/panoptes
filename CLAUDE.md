# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
cargo build              # Build debug
cargo build --release    # Build release (with LTO)
cargo test               # Run all tests
cargo test <test_name>   # Run single test by name
cargo test <module>::    # Run tests in module (e.g., cargo test config::)
cargo run                # Run the application
cargo clippy             # Lint
cargo fmt                # Format code
```

## Architecture

Panoptes is a terminal dashboard for managing multiple Claude Code sessions. It spawns Claude Code in PTYs and tracks their state via HTTP hooks.

### Data Flow

```
User Input → App → PTY write → Claude Code
Claude Code → Hook callback (port 9999) → HookEvent → SessionState update → TUI render
PTY Output → Session buffer → TUI render
```

### Module Responsibilities

- **app.rs**: Main orchestrator. Holds Config, manages event loop, routes input and hook events
- **config.rs**: Configuration loading/saving. Defines paths (~/.panoptes/), hook port (9999), limits
- **session/**: Session lifecycle. SessionState enum (Starting→Thinking→Executing→Waiting→Idle→Exited), SessionInfo struct, PTY handle (future: pty.rs, manager.rs)
- **agent/**: Agent abstraction. AgentType enum, spawn configuration. Future: trait for different agent backends
- **hooks/**: HTTP server receiving Claude Code callbacks. HookEvent parsing, state mapping
- **tui/**: Ratatui terminal setup/teardown, rendering session list and session view

### Key Types

- `SessionState`: Enum tracking Claude Code lifecycle, has `display_name()`, `color()`, `is_active()` helpers
- `SessionInfo`: Session metadata (id, name, state, working_dir, agent_type)
- `HookEvent`: Parsed JSON from Claude Code hooks (session_id, event_type, tool, timestamp)
- `AgentType`: Enum for supported agents (currently ClaudeCode, expandable)
- `Config`: App configuration with serde TOML serialization

## Conventions

- Return `anyhow::Result<T>` for fallible functions
- Module-level doc comments (`//!`) describing purpose
- Tests in `#[cfg(test)]` blocks within each module
- State enums with display/color helpers for TUI rendering
- Always run `cargo fmt` before committing

## Implementation Reference

See `docs/PHASE1_IMPLEMENTATION.md` for the ticket breakdown. Current status: Ticket 1 (Project Setup) complete, types defined for Tickets 4-6, implementation needed for PTY management, hook server, session manager, and TUI rendering.
