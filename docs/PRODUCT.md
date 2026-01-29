# Panoptes

## What is Panoptes?

Panoptes is a terminal-based dashboard for managing multiple Claude Code sessions simultaneously. Named after the many-eyed giant of Greek mythology, it gives developers a bird's-eye view of all their AI coding assistant sessions across different projects and branches.

## The Problem

When working with Claude Code on complex projects, developers often need to:

- Run multiple sessions for different tasks (one for frontend, one for backend, one for tests)
- Work across multiple git branches simultaneously
- Keep track of what each session is doing
- Switch context between sessions without losing state
- Know immediately when a session needs attention

Currently, this requires juggling multiple terminal windows or tabs, manually tracking which session is working on what, and constantly switching between them.

## The Solution

Panoptes provides a unified interface where you can:

- **See all sessions at a glance** - View every active Claude Code session with its current state (thinking, executing, waiting for input)
- **Organize by project and branch** - Sessions are grouped by git repository and branch, making it easy to find what you're looking for
- **Switch instantly** - Jump between sessions with a single keypress
- **Branch isolation** - Each branch gets its own working directory via git worktrees, so sessions on different branches never interfere with each other
- **Real-time status** - Know immediately when a session needs your attention or has finished a task
- **Get notified** - Terminal bell alerts when sessions need your input

## Navigation Model

Panoptes uses a hierarchical navigation structure:

```
Projects Overview
    └── Project Detail (branches)
            └── Branch Detail (sessions)
                    └── Session View (fullscreen)

Activity Timeline (accessible from Overview with 'a')
```

Navigate forward with `Enter`, backward with `Esc`. This mental model makes it easy to manage many sessions across multiple codebases.

## Session States

Sessions display their current state in real-time:

- **Starting** - Session is initializing
- **Thinking** - Claude is processing your request
- **Executing** - Claude is running a tool (like editing files or running commands)
- **Waiting** - Claude is waiting for your input
- **Idle** - No recent activity
- **Exited** - Session has ended

Shell sessions show simplified states:
- **Running** - A command is executing in the foreground
- **Ready** - Shell is idle and waiting for input

## Key Features

### Multi-Session Management

Run as many Claude Code sessions as you need. Each session is independent and maintains its own conversation history.

### Git-Aware Organization

Panoptes understands git. It organizes sessions by repository and branch, and uses git worktrees to give each branch its own isolated working directory.

### Real-Time State Tracking

Through Claude Code's hook system, Panoptes knows exactly what each session is doing at any moment. No more guessing if a session is still working or waiting for you.

### Attention System

Panoptes actively helps you manage your attention across sessions:

- **Terminal Bell** - When a session transitions to "Waiting" state, a terminal bell sounds to alert you (unless you're already viewing that session)
- **Visual Badges** - Sessions that need attention display colored indicators:
  - Green dot (`●`) - Session just started needing attention
  - Yellow dot (`●`) - Session has been waiting past the idle threshold (default 5 minutes)
- **Needs Attention Section** - The projects overview highlights sessions requiring your input
- **Auto-Acknowledge** - Opening a session automatically clears its attention flag

### Activity Timeline

Press `a` from the projects overview to see all sessions sorted by recent activity. This view cuts across project/branch boundaries, showing you what's been happening across your entire workspace.

### Log Viewer

Press `l` from the projects overview to access the application log viewer. Useful for debugging and understanding what's happening under the hood. Logs are stored in `~/.panoptes/logs/` with 7-day automatic retention.

### Session Scrollback

Session views support scrollback through output history with PgUp/PgDn keys. The terminal maintains a 10,000-line scrollback buffer per session, allowing you to review past output even after it scrolls off screen.

### Keyboard-Driven Interface

Everything is accessible via keyboard shortcuts. Number keys (1-9) for quick selection, Tab to cycle through sessions, and intuitive vim-style navigation.

### Session Naming

You name each session when you create it, making it easy to remember what each one is working on ("frontend-auth", "api-refactor", "test-fixes").

### Shell Sessions

In addition to Claude Code sessions, Panoptes can manage regular shell sessions (bash/zsh). This is useful when you need a terminal alongside your AI sessions - for running servers, watching logs, or manual testing.

- Press `s` from Branch Detail to create a shell session
- Shell sessions show "Running" when a command is executing, "Ready" when idle
- Same keyboard shortcuts work for both session types
- State is detected automatically via foreground process detection
- Shell sessions participate in the attention system - you'll be notified when commands complete

### Custom Shell Shortcuts

Define keyboard shortcuts that quickly spawn shell sessions with predefined commands. Perfect for frequently-used tasks like opening editors, starting dev servers, or running builds.

- Press `k` from any view to manage shortcuts
- In session view (normal mode), press your shortcut key to spawn a shell with that command
- Shortcuts are stored in `~/.panoptes/config.toml` and persist between sessions
- The footer shows your configured shortcuts (e.g., `v:VSCode e:vim`)

Example shortcuts:
- `v` → Open VS Code: `code . &`
- `e` → Open vim: `vim .`
- `w` → Start dev server: `npm run dev`

### Worktree Creation

Create new git worktrees directly from Panoptes with a fuzzy branch selector. Type to filter existing branches or create a new one.

### Multi-Account Support

Manage multiple Claude Code accounts (configurations) and switch between them:

- **Global Configurations** - Define named configurations pointing to different Claude config directories
- **Project Defaults** - Set a default configuration for each project
- **Session Selection** - Choose which configuration to use when creating a new session
- **Visual Indicator** - See which configuration a session is using in the header

Use the `c` key from the projects overview to manage configurations, or from project detail to set the project default.

### Claude Code Permissions Sync

Panoptes helps manage Claude Code's per-project permissions (tool approvals, MCP servers) across worktrees:

- **Copy to New Worktrees** - When creating a new worktree, if the main repository has Claude Code permissions configured, Panoptes offers to copy them to the new worktree. This saves you from re-approving the same tools.

- **Migrate Before Deletion** - When deleting a worktree that has unique permissions not present in the main repository, Panoptes offers to migrate those permissions back. This prevents losing tool approvals you granted while working in the worktree.

- **Multi-Account Aware** - Permissions are read from the correct Claude configuration directory based on the project's default account setting.

### Session Lifecycle

- Create sessions with a name and optional branch context
- Delete sessions with confirmation (`d` then `y/n`)
- Sessions are automatically cleaned up when Panoptes exits

### Project Management

- Add projects by path with automatic git repository detection
- Rename projects for better organization (`r` key)
- Delete projects with confirmation, including cascade deletion of branches and sessions (`d` key)
- Quick attention navigation with `Space` key to jump to next session needing input

## Who Is This For?

Panoptes is for developers who:
- Use Claude Code regularly
- Work on multiple tasks or features simultaneously
- Want better visibility into their AI assistant sessions
- Prefer terminal-based tools over GUI applications

## Current Scope

The current version includes:
- Managing multiple Claude Code sessions
- Shell sessions alongside Claude Code sessions
- Custom shell shortcuts for quick command execution
- Multi-account support for different Claude configurations
- Git repository and branch organization with worktree support
- Real-time session state tracking
- Attention system with notifications and quick navigation
- Activity timeline view
- Log viewer for debugging
- Session scrollback (10,000 lines)
- Project management (add, rename, delete)
- Keyboard-driven navigation
- Project and branch persistence
- Path autocomplete when adding projects

Sessions are ephemeral - they exist only while Panoptes is running. Projects and branches are persisted between sessions.
