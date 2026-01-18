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

- **app.rs**: Main orchestrator. Holds Config, manages event loop, routes input and hook events, handles graceful shutdown with session cleanup
- **config.rs**: Configuration loading/saving. Defines paths (~/.panoptes/), hook port (9999), limits
- **session/**: Session lifecycle management
  - `mod.rs`: SessionState enum (Starting→Thinking→Executing→Waiting→Idle→Exited), Session struct, OutputBuffer
  - `pty.rs`: PtyHandle wrapping portable-pty for spawning and I/O
  - `manager.rs`: SessionManager for create/destroy, polling, state updates
  - `vterm.rs`: ANSI/VT100 terminal emulation with color support
- **agent/**: Agent abstraction
  - `adapter.rs`: AgentAdapter trait defining spawn interface
  - `claude.rs`: ClaudeCodeAdapter implementation with hook script generation
- **hooks/**: HTTP server (Axum on port 9999) receiving Claude Code callbacks, HookEvent parsing
- **tui/**: Ratatui terminal setup/teardown, renders session list and fullscreen session view

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

## Documentation

- `docs/PRODUCT.md`: Product overview and feature descriptions
- `docs/TECHNICAL.md`: Technical stack and architecture details
- `docs/PHASE1_IMPLEMENTATION.md`: Phase 1 implementation tickets (all complete)
