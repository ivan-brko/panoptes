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

- **app.rs**: Main orchestrator. Holds Config, manages event loop, routes input and hook events, handles graceful shutdown with session cleanup. Defines `View` enum (ProjectsOverview, ProjectDetail, BranchDetail, SessionView, ActivityTimeline) and `InputMode` enum.
- **config.rs**: Configuration loading/saving. Defines paths (~/.panoptes/), hook port (9999), limits, `idle_threshold_secs`
- **session/**: Session lifecycle management
  - `mod.rs`: SessionState enum (Starting→Thinking→Executing→Waiting→Idle→Exited), Session struct, OutputBuffer, `needs_attention` tracking
  - `pty.rs`: PtyHandle wrapping portable-pty for spawning and I/O
  - `manager.rs`: SessionManager for create/destroy, polling, state updates, attention acknowledgment, terminal bell
  - `vterm.rs`: ANSI/VT100 terminal emulation with color support
- **project/**: Project and branch management
  - `mod.rs`: Project and Branch structs, ProjectId/BranchId type aliases
  - `store.rs`: ProjectStore for CRUD operations and persistence to `~/.panoptes/projects.json`
- **agent/**: Agent abstraction
  - `adapter.rs`: AgentAdapter trait defining spawn interface
  - `claude.rs`: ClaudeCodeAdapter implementation with hook script generation
- **git/**: Git operations
  - `mod.rs`: GitOps struct for repository operations
  - `worktree.rs`: Git worktree creation and management
- **hooks/**: HTTP server (Axum on port 9999) receiving Claude Code callbacks, HookEvent parsing
- **tui/**: Ratatui terminal setup/teardown
  - `mod.rs`: Tui struct for terminal management
  - `views/mod.rs`: View rendering exports
  - `views/projects.rs`: Projects overview (grid of projects, needs attention section)
  - `views/project_detail.rs`: Project detail (branches list, worktree creation UI)
  - `views/branch_detail.rs`: Branch detail (sessions list, delete confirmation)
  - `views/session.rs`: Fullscreen session view
  - `views/timeline.rs`: Activity timeline (all sessions sorted by activity)

### Key Types

- `SessionState`: Enum tracking Claude Code lifecycle, has `display_name()`, `color()`, `is_active()` helpers
- `SessionInfo`: Session metadata (id, name, state, working_dir, agent_type, project_id, branch_id, needs_attention)
- `HookEvent`: Parsed JSON from Claude Code hooks (session_id, event_type, tool, timestamp)
- `AgentType`: Enum for supported agents (currently ClaudeCode, expandable)
- `Config`: App configuration with serde TOML serialization
- `ProjectId`, `BranchId`: UUID type aliases for type-safe identifiers
- `Project`: Git repository metadata (name, repo_path, default_branch, timestamps)
- `Branch`: Branch within a project (name, working_dir, is_default, is_worktree)
- `ProjectStore`: In-memory store for projects and branches with TOML persistence
- `View`: Enum for current UI view (ProjectsOverview, ProjectDetail, BranchDetail, SessionView, ActivityTimeline)
- `InputMode`: Enum for input handling (Normal, Session, CreatingSession, AddingProject, CreatingWorktree, ConfirmingSessionDelete)

## Conventions

- Return `anyhow::Result<T>` for fallible functions
- Module-level doc comments (`//!`) describing purpose
- Tests in `#[cfg(test)]` blocks within each module
- State enums with display/color helpers for TUI rendering
- Always run `cargo fmt` before committing

## Documentation

- `docs/PRODUCT.md`: Product overview and feature descriptions
- `docs/TECHNICAL.md`: Technical stack and architecture details
- `docs/PHASES.md`: Implementation roadmap and progress
