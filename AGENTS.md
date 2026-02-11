# Panoptes

<!-- SHARED INSTRUCTIONS — keep in sync with CLAUDE.md -->
<!-- Both Claude Code and OpenAI Codex read this file. -->
<!-- Claude-specific config (hooks, subagents) stays in .claude/ -->
<!-- Codex-specific config stays in .codex/ -->

## Tech Stack

- **Language**: Rust
- **UI Framework**: ratatui (terminal UI)
- **Async Runtime**: tokio
- **Error Handling**: anyhow
- **Serialization**: serde + serde_json
- **Terminal**: crossterm

## Commands

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

- `app/` - Application orchestration, state, views, input modes
- `agent/` - Agent adapters (Claude Code, Codex, Shell) with hook setup
- `session/` - Session lifecycle, PTY management, terminal emulation
- `hooks/` - HTTP server for agent callbacks
- `input/` - Input handling by mode (normal, session, dialogs)
- `tui/` - Terminal UI rendering with ratatui
- `project/` - Project/branch management and persistence
- `claude_config/` - Claude Code multi-account configuration
- `codex_config/` - Codex CLI multi-account configuration (CODEX_HOME)
- `config.rs` - Configuration (~/.panoptes/)

## Coding Conventions

- Return `anyhow::Result<T>` for fallible functions
- Add context with `.context("description")` for error propagation
- Module-level doc comments (`//!`) describing purpose
- Tests in `#[cfg(test)]` blocks within each module
- State enums with display/color helpers for TUI rendering
- Always run `cargo fmt` and `cargo lint` before committing
- When adding keyboard shortcuts, update **all three**:
  1. Footer help in `src/tui/views/<view>.rs`
  2. Help overlay in `src/tui/views/help.rs`
  3. Reserved keys in `src/config.rs` (`RESERVED_KEYS` constant) - prevents users from binding custom shortcuts to built-in keys

## Error Handling

- Use `anyhow::Result<T>` for all fallible functions
- Add context with `.context("description")` for all error propagation
- For recoverable errors (e.g., corrupted config files):
  - Log at appropriate level (`tracing::warn!` or `tracing::error!`)
  - Provide a sensible fallback (e.g., empty config, fresh state)
  - Create backups of corrupted files when possible
- For unrecoverable errors, propagate with context so the caller can decide

## Git Conventions

- Commit messages start with a verb: Add, Fix, Update, Remove, Refactor
- Keep first line under 72 characters
- Add body for complex changes
- Reference issues if applicable

## Testing Requirements

- Tests in `#[cfg(test)]` blocks within each module
- Use Arrange/Act/Assert pattern
- Cover happy path, edge cases, and error conditions
- Common test dependencies: `tempfile`, `uuid`

## Documentation

- `docs/PRODUCT.md` - Product overview
- `docs/TECHNICAL.md` - Technical details
- `docs/CONFIG_GUIDE.md` - Configuration reference
