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

Activity Timeline (accessible from Overview with 't')
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

Press `t` from the projects overview to see all sessions sorted by recent activity. This view cuts across project/branch boundaries, showing you what's been happening across your entire workspace.

### Keyboard-Driven Interface

Everything is accessible via keyboard shortcuts. Number keys (1-9) for quick selection, Tab to cycle through sessions, and intuitive vim-style navigation.

### Session Naming

You name each session when you create it, making it easy to remember what each one is working on ("frontend-auth", "api-refactor", "test-fixes").

### Worktree Creation

Create new git worktrees directly from Panoptes with a fuzzy branch selector. Type to filter existing branches or create a new one.

### Session Lifecycle

- Create sessions with a name and optional branch context
- Delete sessions with confirmation (`d` then `y/n`)
- Sessions are automatically cleaned up when Panoptes exits

## Who Is This For?

Panoptes is for developers who:
- Use Claude Code regularly
- Work on multiple tasks or features simultaneously
- Want better visibility into their AI assistant sessions
- Prefer terminal-based tools over GUI applications

## Current Scope

The current version includes:
- Managing multiple Claude Code sessions
- Git repository and branch organization with worktree support
- Real-time session state tracking
- Attention system with notifications
- Activity timeline view
- Keyboard-driven navigation
- Project and branch persistence

Sessions are ephemeral - they exist only while Panoptes is running. Projects and branches are persisted between sessions.
