# Panoptes

## What is Panoptes?

Panoptes is a terminal-based dashboard for managing multiple AI coding agent sessions simultaneously. It supports both Claude Code and OpenAI Codex CLI. Named after the many-eyed giant of Greek mythology, it gives developers a bird's-eye view of all their AI coding assistant sessions across different projects and branches.

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

Panoptes shows three panes at once. `→` and `←` cycle focus — `Tab` and
`Shift+Tab` do the same — and the focused pane widens while the other two
shrink, so whatever you are working on gets the room and the rest stay in view.

```
┌──────────────────────────┬─────────────────┬─────────────────────┐
│ Projects                 │ Sessions        │ Settings            │
│   folders, projects,     │   every session │   accounts, custom  │
│   branches, sessions     │   flat & sorted │   keys, notifications│
└──────────────────────────┴─────────────────┴─────────────────────┘
```

Pane 1 drills down; the other two stay put while it does:

```
Projects (tree)
    └── Folder (optional, up to 3 levels deep)
            └── Project (branches)  ── , ──> Project settings
                    └── Branch (sessions)
                            └── Session (fullscreen)
```

Navigate forward with `Enter`, back with `Esc`. `Esc` backs out: one level in
the focused pane, and once there is nothing left to pop it returns you to the
Projects pane — home. It never quits; `q` quits.

Opening a session is the only thing that fills the terminal; `Esc` puts you back
in the pane you opened it from.

## Session States

Sessions display their current state in real-time:

- **Starting** - Session is initializing
- **Thinking** - Agent is processing your request
- **Executing** - Agent is running a tool (like editing files or running commands)
- **Needs approval** - Agent is asking for permission to proceed
- **Waiting** - Agent is waiting for your input
- **Suspended** - Session process was suspended after inactivity (wakes on interaction)
- **Exited** - Session has ended
- **Resumable** - Session recovered from a previous run and can be resumed

Shell sessions show simplified states:
- **Running** - A command is executing in the foreground
- **Ready** - Shell is idle and waiting for input

## Key Features

### Multi-Session Management

Run as many Claude Code and Codex sessions as you need. Each session is independent and maintains its own conversation history. When creating a new session, an agent type selector lets you choose between Claude Code and Codex.

### Git-Aware Organization

Panoptes understands git. It organizes sessions by repository and branch, and uses git worktrees to give each branch its own isolated working directory.

### Real-Time State Tracking

Through agent hook systems, Panoptes knows exactly what each session is doing at any moment. Claude Code provides granular tool-use tracking (Thinking, Executing, Waiting), while Codex provides turn-complete notifications (Thinking, Waiting). No more guessing if a session is still working or waiting for you.

### Attention System

Panoptes actively helps you manage your attention across sessions:

- **Terminal Bell** - When a session transitions to "Waiting" state, a terminal bell sounds to alert you (unless you're already viewing that session)
- **Visual Badges** - Sessions that need attention display a coloured dot saying why:
  - Green dot (`●`) - The turn finished; the agent is waiting for your next prompt
  - Yellow dot (`●`) - Blocked on a permission dialog, or a tool that stopped reporting
  - Red dot (`●`) - The agent process died
- **Needs Attention Section** - Pinned to the top of the Sessions pane, with a blinking count in the header that is visible from every pane
- **Auto-Acknowledge** - Opening a session clears its attention flag, and nothing re-raises it until something new happens

### Logs

Logs are written to `~/.panoptes/logs/` with 7-day automatic retention.
Settings → About / paths shows the current log file's path; read it with
whatever you already use for tailing files.

### Session Scrollback

Session views support scrollback through output history with PgUp/PgDn keys. The terminal maintains a 10,000-line scrollback buffer per session, allowing you to review past output even after it scrolls off screen.

### Keyboard-Driven Interface

Everything is accessible via keyboard shortcuts. Number keys (1-9) for quick
selection, `→`/`←` (or `Tab`/`Shift+Tab`) to cycle panes, `↑`/`↓` to move within
one, and `Enter` as the single key that acts on whatever is selected.

### Session Naming

You name each session when you create it, making it easy to remember what each one is working on ("frontend-auth", "api-refactor", "test-fixes").

### Shell Sessions

In addition to Claude Code sessions, Panoptes can manage regular shell sessions (bash/zsh). This is useful when you need a terminal alongside your AI sessions - for running servers, watching logs, or manual testing.

- Press `s` at a branch to create a shell session
- Shell sessions show "Running" when a command is executing, "Ready" when idle
- Same keyboard shortcuts work for both session types
- State is detected automatically via foreground process detection
- Shell sessions participate in the attention system - you'll be notified when commands complete

### Custom Shell Shortcuts

Define keyboard shortcuts that quickly spawn shell sessions with predefined commands. Perfect for frequently-used tasks like opening editors, starting dev servers, or running builds.

- Manage them in Settings → Shortcuts
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

Manage multiple accounts for both Claude Code and Codex CLI:

**Claude Code Accounts:**
- Define named configurations pointing to different Claude config directories (`CLAUDE_CONFIG_DIR`)
- Use the `c` key from the projects overview to manage configurations

**Codex Accounts:**
- Define named configurations pointing to different Codex home directories (`CODEX_HOME`)
- Use the `x` key from the projects overview to manage configurations

**Shared features:**
- **Project Defaults** - Set a default configuration for each project (independent for Claude and Codex)
- **Session Selection** - Choose which configuration to use when creating a new session
- **Visual Indicator** - See which configuration a session is using in the header

### Claude Code Permissions Sync

> **Note:** Permissions sync is currently supported for Claude Code only. Codex permissions sync will be added when Codex CLI supports per-project permissions.

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
- Group projects into folders, nested up to 3 levels, so several repos belonging
  to the same client or product sit together. A collapsed folder still reports
  how many of its sessions are active or waiting on you
- Rename projects for better organization (`r` key)
- Delete projects with confirmation, including cascade deletion of branches and sessions (`d` key)
- Quick attention navigation with `Space` key to jump to next session needing input

## Who Is This For?

Panoptes is for developers who:
- Use Claude Code or Codex CLI regularly
- Work on multiple tasks or features simultaneously
- Want better visibility into their AI assistant sessions
- Prefer terminal-based tools over GUI applications

## Current Scope

The current version includes:
- Managing multiple Claude Code and Codex CLI sessions
- Shell sessions alongside AI agent sessions
- Custom shell shortcuts for quick command execution
- Multi-account support for both Claude and Codex configurations
- Git repository and branch organization with worktree support
- Real-time session state tracking
- Attention system with notifications and quick navigation
- Log viewer for debugging
- Session scrollback (10,000 lines)
- Project management (add, rename, delete)
- Keyboard-driven navigation
- Project and branch persistence
- Path autocomplete when adding projects

Agent sessions are persisted to `sessions.json` and recovered across restarts - a recovered session shows as "Resumable" and can be resumed. Shell sessions are not: a shell has no conversation to reattach to, and everything that made it worth keeping - its scrollback, its environment, whatever it was running - dies with the terminal. Quitting warns you when live shells are about to be killed. Projects and branches are persisted as well.
