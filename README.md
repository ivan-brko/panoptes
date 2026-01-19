# Panoptes

A terminal dashboard for managing multiple Claude Code sessions simultaneously.

Named after the many-eyed giant of Greek mythology, Panoptes gives developers a bird's-eye view of all their AI coding assistant sessions across different projects.

## Features

- **Multi-Session Management** - Run as many Claude Code sessions as you need, each independent with its own conversation history
- **Project & Branch Organization** - Sessions organized by git repository and branch, with git worktree support for branch isolation
- **Real-Time State Tracking** - See what each session is doing at any moment (Thinking, Executing, Waiting, Idle)
- **Attention System** - Terminal bell notifications when sessions need your input, with visual badges showing attention state
- **Activity Timeline** - View all sessions sorted by recent activity across all projects
- **Keyboard-Driven Interface** - Fast navigation with vim-style keys and number shortcuts
- **Session Naming** - Name sessions for easy identification ("frontend-auth", "api-refactor")
- **Clean Exit** - Sessions are automatically cleaned up when you quit

## Installation

```bash
# Clone the repository
git clone https://github.com/yourusername/panoptes.git
cd panoptes

# Build release binary
cargo build --release

# Run
./target/release/panoptes
```

## Quick Start

1. Launch Panoptes: `cargo run` or `./target/release/panoptes`
2. Press `a` to add your first project (enter the path to a git repository)
3. Navigate to a project with `Enter`, then to a branch with `Enter`
4. Press `n` to create a new session on that branch
5. Enter a name for the session and press Enter
6. You're now in Session mode - type to interact with Claude Code
7. Press `Esc` to exit Session mode, `Esc` again to go back through the hierarchy
8. Press `q` to quit (sessions are cleaned up automatically)

## Views

Panoptes has a hierarchical navigation model:

1. **Projects Overview** - Grid of all your projects with session counts
2. **Project Detail** - Branches within a selected project
3. **Branch Detail** - Sessions for a specific branch
4. **Session View** - Fullscreen view of a single Claude Code session
5. **Activity Timeline** - All sessions sorted by recent activity (press `t`)

Navigate forward with `Enter`, backward with `Esc`.

## Keyboard Shortcuts

### Projects Overview

| Key | Action |
|-----|--------|
| `a` | Add a new project |
| `n` | Create session (in selected project's default branch) |
| `t` | Open activity timeline |
| `j` / `Down` | Move selection down |
| `k` / `Up` | Move selection up |
| `Enter` | Open selected project |
| `1-9` | Select project by number |
| `q` | Quit |

### Project Detail View

| Key | Action |
|-----|--------|
| `w` | Create new worktree (opens branch selector) |
| `j` / `Down` | Move selection down |
| `k` / `Up` | Move selection up |
| `Enter` | Open selected branch |
| `1-9` | Select branch by number |
| `Esc` | Return to projects overview |
| `q` | Quit |

### Branch Detail View

| Key | Action |
|-----|--------|
| `n` | Create new session on this branch |
| `d` | Delete selected session (prompts for confirmation) |
| `j` / `Down` | Move selection down |
| `k` / `Up` | Move selection up |
| `Enter` | Open selected session |
| `Esc` | Return to project detail |
| `q` | Return to project detail |

### Session View (Normal Mode)

| Key | Action |
|-----|--------|
| `Enter` | Enter Session mode (interact with PTY) |
| `Esc` | Return to previous view |
| `Tab` | Switch to next session |
| `1-9` | Jump to session by number |
| `q` | Return to previous view |

### Session View (Session Mode)

| Key | Action |
|-----|--------|
| `Esc` | Exit to Normal mode |
| All other keys | Sent directly to Claude Code |

### Activity Timeline

| Key | Action |
|-----|--------|
| `j` / `Down` | Move selection down |
| `k` / `Up` | Move selection up |
| `Enter` | Open selected session |
| `Esc` | Return to projects overview |
| `q` | Return to projects overview |

### Worktree Creation (Branch Selector)

| Key | Action |
|-----|--------|
| Type | Filter branches / enter new branch name |
| `j` / `Down` | Move selection down |
| `k` / `Up` | Move selection up |
| `Enter` | Create worktree (new or existing branch) |
| `Esc` | Cancel |

### Session Deletion Confirmation

| Key | Action |
|-----|--------|
| `y` | Confirm deletion |
| `n` / `Esc` | Cancel deletion |

## Session States

- **Starting** - Session is initializing
- **Thinking** - Claude is processing your request
- **Executing** - Claude is running a tool (editing files, running commands)
- **Waiting** - Claude is waiting for your input
- **Idle** - No recent activity
- **Exited** - Session has ended

## Attention System

Panoptes helps you know when sessions need your attention:

- **Terminal Bell** - When a session transitions to "Waiting" state (needs your input), a terminal bell sounds (unless you're viewing that session)
- **Attention Badges** - Sessions display colored indicators:
  - `●` (green) - Session just started needing attention
  - `●` (yellow) - Session has been waiting for a while (idle threshold exceeded)
- **Needs Attention Section** - The projects overview shows a dedicated section for sessions requiring attention
- **Auto-Acknowledge** - Opening a session automatically acknowledges its attention state

Configure the idle threshold in your config file (default: 300 seconds / 5 minutes).

## Configuration

Configuration is stored in `~/.panoptes/config.toml`. Settings:

| Setting | Default | Description |
|---------|---------|-------------|
| `hook_port` | 9999 | Port for the HTTP hook server |
| `max_output_lines` | 10,000 | Lines kept in output buffer per session |
| `idle_threshold_secs` | 300 | Seconds before waiting session shows yellow badge |

Example config file:

```toml
hook_port = 9999
max_output_lines = 10000
idle_threshold_secs = 300
```

## File Locations

| Path | Purpose |
|------|---------|
| `~/.panoptes/config.toml` | User configuration |
| `~/.panoptes/projects.toml` | Project and branch persistence |
| `~/.panoptes/hooks/` | Hook scripts for Claude Code |
| `~/.panoptes/worktrees/` | Git worktrees for branch isolation |

## Documentation

- [Product Overview](docs/PRODUCT.md) - Detailed feature descriptions
- [Technical Stack](docs/TECHNICAL.md) - Architecture and dependencies
- [Implementation Phases](docs/PHASES.md) - Development roadmap

## Development

```bash
cargo build              # Build debug
cargo build --release    # Build release
cargo test               # Run all tests (108 tests)
cargo clippy             # Lint
cargo fmt                # Format code
```

## License

MIT
