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

- **app/**: Application orchestration and state management
  - `mod.rs`: Main `App` struct and event loop. Routes input and hook events, handles graceful shutdown with session cleanup, rendering
  - `view.rs`: `View` enum (ProjectsOverview, ProjectDetail, BranchDetail, SessionView, ActivityTimeline, LogViewer, FocusStats) with navigation helpers
  - `input_mode.rs`: `InputMode` enum for input handling modes
  - `state.rs`: `AppState` struct containing all application state, `HomepageFocus` enum, and navigation helper methods
- **input/**: Input handling organized by mode
  - `mod.rs`: Module exports
  - `dispatcher.rs`: Main input dispatch logic - routes key events to mode-specific handlers
  - `session_mode.rs`: Session mode handlers - PTY forwarding, mouse events, paste handling
  - `text_input.rs`: Text input handlers - session naming, project path input, project renaming
  - `dialogs.rs`: Confirmation dialog handlers - delete confirmations, focus timer dialogs
  - `normal/`: Normal mode handlers by view
    - `projects_overview.rs`: Projects grid navigation, project selection
    - `project_detail.rs`: Branch list navigation, worktree creation
    - `branch_detail.rs`: Session list navigation, branch deletion
    - `session_view.rs`: Session scrolling, session switching
    - `timeline.rs`: Activity timeline navigation
    - `log_viewer.rs`: Log scrolling and navigation
    - `focus_stats.rs`: Focus stats navigation
- **wizards/**: Multi-step wizard workflows
  - `worktree/`: Worktree creation wizard
    - `mod.rs`: Module exports
    - `types.rs`: `BranchRef`, `BranchRefType`, `WorktreeCreationType`, `filter_branch_refs()`
    - `handlers.rs`: Wizard step handlers - branch selection, base selection, confirmation
  - `project/`: (Placeholder) Project addition wizard
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
- **logging/**: Application logging system
  - `mod.rs`: Logging initialization and exports
  - `buffer.rs`: LogBuffer for real-time log display in TUI
  - `file_writer.rs`: File-based logging with timestamps
  - `retention.rs`: Automatic cleanup of old log files (7-day retention)
- **focus_timing/**: Focus timing and statistics
  - `mod.rs`: FocusTimer struct for tracking focus sessions
  - `tracker.rs`: FocusTracker for recording focus intervals with terminal focus awareness
  - `stats.rs`: FocusSession and FocusContextBreakdown types for statistics
  - `store.rs`: FocusStore for persisting focus sessions to disk
- **path_complete.rs**: Path completion/autocomplete for directory input with tilde expansion
- **tui/**: Ratatui terminal setup/teardown
  - `mod.rs`: Tui struct for terminal management
  - `theme.rs`: Centralized color and style definitions for UI
  - `header.rs`: Unified header component with notifications and attention indicator
  - `header_notifications.rs`: Transient header notification manager
  - `notifications.rs`: Overlay notification manager
  - `frame.rs`: FrameConfig and FrameLayout for consistent UI layout
  - `layout.rs`: Screen layout calculations
  - `widgets/mod.rs`: Custom widget exports
  - `widgets/project_card.rs`: Project card widget for grid display
  - `views/mod.rs`: View rendering exports
  - `views/projects.rs`: Projects overview (grid of projects, needs attention section)
  - `views/project_detail.rs`: Project detail (branches list, worktree creation UI)
  - `views/branch_detail.rs`: Branch detail (sessions list, delete confirmation)
  - `views/session.rs`: Fullscreen session view with scrollback support
  - `views/timeline.rs`: Activity timeline (all sessions sorted by activity)
  - `views/logs.rs`: Log viewer for application logs
  - `views/focus_stats.rs`: Focus timing statistics view
  - `views/confirm.rs`: Reusable confirmation dialog component
  - `views/notifications.rs`: Notification overlay rendering

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
- `View`: Enum for current UI view (in `app/view.rs`)
- `InputMode`: Enum for input handling modes (in `app/input_mode.rs`)
- `AppState`: All application state with navigation helpers (in `app/state.rs`)
- `HomepageFocus`: Focus state for homepage (Projects or Sessions)
- `BranchRef`, `BranchRefType`, `WorktreeCreationType`: Worktree wizard types (in `wizards/worktree/types.rs`)
- `FocusTimer`, `FocusTracker`, `FocusSession`: Focus timing types

## Development Skills

### Adding a New View

1. Add variant to `View` enum in `src/app/view.rs`
2. Implement `parent()` method for navigation
3. Create render function in `src/tui/views/new_view.rs`
4. Export from `src/tui/views/mod.rs`
5. Add case to render dispatch in `src/app/mod.rs` (in `render()` method)
6. Create input handler in `src/input/normal/new_view.rs`
7. Export handler from `src/input/normal/mod.rs`
8. Add case to `handle_normal_mode_key()` in `src/app/mod.rs`

### Adding a New Input Mode

1. Add variant to `InputMode` enum in `src/app/input_mode.rs`
2. Create handler function in appropriate module:
   - Text input modes: `src/input/text_input.rs`
   - Dialog modes: `src/input/dialogs.rs`
   - Session modes: `src/input/session_mode.rs`
   - Wizard modes: `src/wizards/<wizard>/handlers.rs`
3. Add case to `handle_key_event()` in `src/input/dispatcher.rs`

### Adding a New Agent Type

1. Add variant to `AgentType` enum in `src/agent/mod.rs`
2. Create adapter in `src/agent/new_agent.rs`
3. Implement `AgentAdapter` trait
4. Update factory method in `AgentType::create_adapter()`

### Adding Worktree Wizard Types

1. Add types to `src/wizards/worktree/types.rs`
2. Export from `src/wizards/worktree/mod.rs`
3. Re-export from `src/app/mod.rs` for backwards compatibility if needed

## Conventions

- Return `anyhow::Result<T>` for fallible functions
- Module-level doc comments (`//!`) describing purpose
- Tests in `#[cfg(test)]` blocks within each module
- State enums with display/color helpers for TUI rendering
- Always run `cargo fmt` before committing
- Types are extracted into submodules but re-exported from parent modules for ergonomic imports
- When adding/changing/removing keyboard shortcuts, always update the corresponding footer help text in `src/tui/views/`. The implementation (in `src/input/`) and documentation (in `src/tui/views/`) must stay in sync.

## Documentation

- `docs/PRODUCT.md`: Product overview and feature descriptions
- `docs/TECHNICAL.md`: Technical stack and architecture details
- `docs/PHASES.md`: Implementation roadmap and progress
