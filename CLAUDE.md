<!-- NOTE: Core project instructions are duplicated in AGENTS.md for Codex compatibility. -->
<!-- When updating shared rules (stack, commands, architecture, conventions), update BOTH files. -->

# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test

```bash
cargo build              # Build debug
cargo build --release    # Build release
cargo test               # Run all tests
cargo run                # Run the application
cargo lint               # Lint (clippy --all-targets -- -D warnings)
cargo fmt                # Format code
```

## Linting

Common clippy patterns to be aware of:
- Use `is_some_and()` instead of `map_or(false, ...)`
- Use `!container.is_empty()` instead of `container.len() >= 1`
- Use `std::slice::from_ref()` instead of creating single-element vec via clone
- Remove needless borrows (e.g., `&foo.to_string()` → `foo.to_string()`)

## Architecture

Panoptes is a terminal dashboard for managing multiple AI coding agent sessions (Claude Code and OpenAI Codex CLI). It spawns agents in PTYs and tracks their state via HTTP hooks.

### Data Flow

```
User Input → App → PTY write → Agent (Claude Code / Codex)
Agent → Hook (port 9999) → HookEvent → SessionState update → TUI render
PTY Output → Session buffer → TUI render
```

### Key Modules

- `app/` - Application orchestration, state, navigation, input modes
- `app/nav.rs` - `Focus` / `Tab` / `ProjectsNav` / `SettingsNav`: which pane owns
  the screen and how far each one is drilled in
- `app/background.rs` - Off-thread git jobs (fetch, worktree create/remove) with a cancellable loading overlay
- `agent/` - Agent adapters (Claude Code, Codex, Shell) with hook setup
- `session/` - Session lifecycle, PTY management, terminal emulation
- `session/state_machine.rs` - Pure agent-event state machine (hook event → state transition)
- `hooks/` - HTTP server for agent callbacks
- `transcript/` - Reads agent transcripts on disk (Codex state, usage for both)
- `input/` - Input handling by mode (normal, session, dialogs)
- `input/normal/{projects,sessions,settings}_pane.rs` - One handler per pane, each routing on its own drill-down level
- `input/agent_configs.rs` - Shared Claude/Codex config input handlers (parameterized by `AgentKind`)
- `tui/` - Terminal UI rendering with ratatui
- `tui/panes.rs` - Accordion sizing and its transition (`pane_widths`, `PaneLayout`, `SideMode`)
- `tui/header.rs` - The one global header: the wordmark, and everything laid out around it
- `tui/logo.rs` - The wordmark itself, and the column band the header reserves for it
- `tui/views/panes.rs` - The three-pane frame: one header, three bordered panes, one footer
- `tui/views/pane_{projects,sessions,settings}.rs` - Each pane's content, at every density
- `tui/views/prompts.rs` / `tui/views/worktree.rs` - The centred overlays (lists and paragraphs)
- `tui/views/agent_configs.rs` - Shared Claude/Codex config view rendering
- `tui/widgets/dialog.rs` - Shared dialog widget (Yes/No buttons, clamped centering)
- `project/` - Project/branch management, folder tree, and persistence
- `persistence.rs` - Shared atomic-save / load-with-backup for all state files
- `agent_profiles.rs` - Generic agent profile store (`ProfileStore<C>`)
- `claude_config/` - Claude Code multi-account configuration
- `codex_config/` - Codex CLI multi-account configuration (CODEX_HOME)
- `config.rs` - Configuration (~/.panoptes/)

## Conventions

- Return `anyhow::Result<T>` for fallible functions
- Add context with `.context("description")` for error propagation
- Module-level doc comments (`//!`) describing purpose
- Tests in `#[cfg(test)]` blocks within each module
- State enums with display/color helpers for TUI rendering
- Always run `cargo fmt` and `cargo lint` before committing
- When adding keyboard shortcuts, update **all three**:
  1. Footer help in `src/tui/views/panes.rs` (`footer_text` and its helpers)
  2. Help overlay in `src/tui/views/help.rs`
  3. Reserved keys in `src/config.rs` (`RESERVED_KEYS` constant) - prevents users from binding custom shortcuts to built-in keys
- The header carries the wordmark, not the app's name in text. `Breadcrumb::new()`
  has no root segment, so a view names only where the user *is*; the header drops
  to a spelled-out `PANOPTES` when the terminal is too small for the art
- Prompts split by content: **if it shows a list or a paragraph it is a centred
  overlay** (`tui/views/prompts.rs`, `worktree.rs`), anchored to the terminal so
  an animating pane cannot resize it mid-typing; **if it is one line you type
  into, it is inline** in the pane that owns it
- Pane rows drop fields whole as the pane narrows rather than truncating one
  long string, and are truncated against the pane's *current* width

## Error Handling

- Use `anyhow::Result<T>` for all fallible functions
- Add context with `.context("description")` for all error propagation
- For recoverable errors (e.g., corrupted config files):
  - Log at appropriate level (`tracing::warn!` or `tracing::error!`)
  - Provide a sensible fallback (e.g., empty config, fresh state)
  - Create backups of corrupted files when possible
- For unrecoverable errors, propagate with context so the caller can decide

## Documentation

- `docs/PRODUCT.md` - Product overview
- `docs/TECHNICAL.md` - Technical details
- `docs/CONFIG_GUIDE.md` - Configuration reference
