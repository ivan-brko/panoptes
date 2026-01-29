# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.2] - 2025-01-29

### Added
- Custom shell session shortcuts feature - define custom keyboard shortcuts that execute shell commands
- Custom shortcuts support in branch detail view
- Custom shortcuts documentation in UI footers
- Shell session attention/notification support
- Comprehensive FAQ documentation
- Session mode troubleshooting questions to FAQ

### Changed
- Skip notifications and attention flag for active session (no notifications while you're viewing that session)

### Fixed
- Deletion dialog now shows correct warning for session type
- Keyboard shortcut documentation discrepancies

## [0.2.1] - 2025-01-27

### Added
- Hook event coalescing to prevent UI lag during rapid state changes
- Branch refresh feature (`R` key) to check for stale worktrees
- Stale worktree indicators (red highlighting for missing worktrees)
- Improved disk error handling with user-friendly messages for disk full and permission errors
- Comprehensive documentation:
  - [Keyboard Reference](docs/KEYBOARD_REFERENCE.md)
  - [Configuration Guide](docs/CONFIG_GUIDE.md)
  - [Troubleshooting Guide](docs/TROUBLESHOOTING.md)
  - [Installation Guide](docs/INSTALLATION.md)

### Changed
- README.md overhauled with improved structure and documentation links

## [0.1.0] - 2025-01-23

### Added

#### Core Features
- Multi-session management for Claude Code
- Project and branch organization with git repository support
- Real-time session state tracking (Starting, Thinking, Executing, Waiting, Idle, Exited)
- Session naming for easy identification
- Automatic session cleanup on quit

#### Git Integration
- Git worktree support for branch isolation
- Worktree creation wizard with branch selection
- Default base branch configuration per project
- Remote branch fetching via git CLI

#### Navigation
- Hierarchical navigation (Projects -> Branches -> Sessions)
- Activity timeline view for all sessions sorted by recent activity
- Keyboard-driven interface with vim-style navigation
- Number shortcuts for quick selection (1-9)

#### Attention System
- Terminal bell notifications when sessions need input
- Visual attention badges (green for new, yellow for idle)
- Attention count indicators in header
- Space key to jump to next session needing attention

#### Focus Timer
- Pomodoro-style focus timer with configurable duration
- Per-project and per-branch time tracking
- Focus statistics view with session history
- Terminal focus detection for accurate time tracking

#### User Interface
- Unified header component with notifications
- Transient header notifications for feedback
- Loading indicators for blocking operations
- Confirmation dialogs for destructive actions
- Log viewer for application debugging

#### Configuration
- TOML configuration file support
- Configurable hook server port
- Configurable notification method (bell, title, none)
- Theme presets (dark, light, high-contrast)

### Technical
- Built with Rust using async/await (Tokio runtime)
- Ratatui for terminal UI
- Axum for HTTP hook server
- vt100 crate for terminal emulation
- Portable-pty for PTY management
- git2 for git operations

### Fixed
- Paste handling with retry logic for non-blocking PTY writes
- Git fetch operations using git CLI for proper SSH authentication
- Branch name validation in worktree wizard
- Focus timer countdown accuracy with Alt+Tab detection
- Escape key behavior (Shift+Escape forwards to PTY)

[Unreleased]: https://github.com/ivan-brko/panoptes/compare/v0.2.2...HEAD
[0.2.2]: https://github.com/ivan-brko/panoptes/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/ivan-brko/panoptes/compare/v0.1.0...v0.2.1
[0.1.0]: https://github.com/ivan-brko/panoptes/releases/tag/v0.1.0
