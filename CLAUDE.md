# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test

```bash
cargo build              # Build debug
cargo build --release    # Build release
cargo test               # Run all tests
cargo run                # Run the application
cargo clippy -- -D warnings  # Lint (run before committing)
cargo fmt                # Format code
```

## Linting

Common clippy patterns to be aware of:
- Use `is_some_and()` instead of `map_or(false, ...)`
- Use `!container.is_empty()` instead of `container.len() >= 1`
- Use `std::slice::from_ref()` instead of creating single-element vec via clone
- Remove needless borrows (e.g., `&foo.to_string()` → `foo.to_string()`)

## Architecture

Panoptes is a terminal dashboard for managing multiple Claude Code sessions. It spawns Claude Code in PTYs and tracks their state via HTTP hooks.

### Data Flow

```
User Input → App → PTY write → Claude Code
Claude Code → Hook (port 9999) → HookEvent → SessionState update → TUI render
PTY Output → Session buffer → TUI render
```

### Key Modules

- `app/` - Application orchestration, state, views, input modes
- `session/` - Session lifecycle, PTY management, terminal emulation
- `hooks/` - HTTP server for Claude Code callbacks
- `input/` - Input handling by mode (normal, session, dialogs)
- `tui/` - Terminal UI rendering with ratatui
- `project/` - Project/branch management and persistence
- `config.rs` - Configuration (~/.panoptes/)

## Conventions

- Return `anyhow::Result<T>` for fallible functions
- Add context with `.context("description")` for error propagation
- Module-level doc comments (`//!`) describing purpose
- Tests in `#[cfg(test)]` blocks within each module
- State enums with display/color helpers for TUI rendering
- Always run `cargo fmt` and `cargo clippy` before committing
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

## Documentation

- `docs/PRODUCT.md` - Product overview
- `docs/TECHNICAL.md` - Technical details
- `docs/CONFIG_GUIDE.md` - Configuration reference
