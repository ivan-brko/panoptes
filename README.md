# Panoptes

A terminal dashboard for managing multiple Claude Code sessions simultaneously.

Named after the many-eyed giant of Greek mythology, Panoptes gives developers a bird's-eye view of all their AI coding assistant sessions across different projects.

## Features

- **Multi-Session Management** - Run as many Claude Code sessions as you need, each independent with its own conversation history
- **Real-Time State Tracking** - See what each session is doing at any moment (Thinking, Executing, Waiting, Idle)
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
2. Press `n` to create a new session
3. Enter a name for the session and press Enter
4. Press Enter again to open the session
5. Press `i` or Enter to enter Session mode (interact with Claude Code)
6. Press `Esc` to return to Normal mode
7. Press `Esc` again to return to session list
8. Press `q` to quit (sessions are cleaned up automatically)

## Keyboard Shortcuts

### Session List View

| Key | Action |
|-----|--------|
| `n` | Create new session |
| `j` / `Down` | Move selection down |
| `k` / `Up` | Move selection up |
| `Enter` | Open selected session |
| `d` | Delete selected session |
| `q` | Quit |

### Session View (Normal Mode)

| Key | Action |
|-----|--------|
| `i` / `Enter` | Enter Session mode (interact with PTY) |
| `Esc` | Return to session list |
| `Tab` | Switch to next session |
| `1-9` | Jump to session by number |

### Session View (Session Mode)

| Key | Action |
|-----|--------|
| `Esc` | Exit to Normal mode |
| All other keys | Sent directly to Claude Code |

## Session States

- **Starting** - Session is initializing
- **Thinking** - Claude is processing your request
- **Executing** - Claude is running a tool (editing files, running commands)
- **Waiting** - Claude is waiting for your input
- **Idle** - No recent activity
- **Exited** - Session has ended

## Configuration

Configuration is stored in `~/.panoptes/config.toml`. Default settings:

- Hook server port: 9999
- Max sessions: 10
- Output buffer: 10,000 lines

## Documentation

- [Product Overview](docs/PRODUCT.md) - Detailed feature descriptions
- [Technical Stack](docs/TECHNICAL.md) - Architecture and dependencies
- [Implementation Tracker](docs/PHASE1_IMPLEMENTATION.md) - Development progress

## Development

```bash
cargo build              # Build debug
cargo build --release    # Build release
cargo test               # Run all tests (69 tests)
cargo clippy             # Lint
cargo fmt                # Format code
```

## License

MIT
