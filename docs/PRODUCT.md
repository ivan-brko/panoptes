# Panoptes

## What is Panoptes?

Panoptes is a terminal-based dashboard for managing multiple Claude Code sessions simultaneously. Named after the many-eyed giant of Greek mythology, it gives developers a bird's-eye view of all their AI coding assistant sessions across different projects and branches.

## The Problem

When working with Claude Code on complex projects, developers often need to:

- Run multiple sessions for different tasks (one for frontend, one for backend, one for tests)
- Work across multiple git branches simultaneously
- Keep track of what each session is doing
- Switch context between sessions without losing state

Currently, this requires juggling multiple terminal windows or tabs, manually tracking which session is working on what, and constantly switching between them.

## The Solution

Panoptes provides a unified interface where you can:

- **See all sessions at a glance** - View every active Claude Code session with its current state (thinking, executing, waiting for input)
- **Organize by project and branch** - Sessions are grouped by git repository and branch, making it easy to find what you're looking for
- **Switch instantly** - Jump between sessions with a single keypress
- **Branch isolation** - Each branch gets its own working directory via git worktrees, so sessions on different branches never interfere with each other
- **Real-time status** - Know immediately when a session needs your attention or has finished a task

## How It Works

You launch Panoptes, and it becomes your command center. Create new Claude Code sessions with a name of your choice, and they appear in your dashboard. The interface shows you what each session is doing in real-time:

- **Starting** - Session is initializing
- **Thinking** - Claude is processing your request
- **Executing** - Claude is running a tool (like editing files or running commands)
- **Waiting** - Claude is waiting for your input
- **Idle** - No recent activity

When you want to interact with a session, press Enter to jump into it. All your keystrokes go directly to Claude Code. Press Escape to return to the dashboard view.

## Key Features

### Multi-Session Management
Run as many Claude Code sessions as you need. Each session is independent and maintains its own conversation history.

### Git-Aware Organization
Panoptes understands git. It organizes sessions by repository and branch, and uses git worktrees to give each branch its own isolated working directory.

### Real-Time State Tracking
Through Claude Code's hook system, Panoptes knows exactly what each session is doing at any moment. No more guessing if a session is still working or waiting for you.

### Keyboard-Driven Interface
Everything is accessible via keyboard shortcuts. Number keys (1-9) for quick session switching, Tab to cycle through sessions, and intuitive vim-style navigation.

### Session Naming
You name each session when you create it, making it easy to remember what each one is working on ("frontend-auth", "api-refactor", "test-fixes").

## Who Is This For?

Panoptes is for developers who:
- Use Claude Code regularly
- Work on multiple tasks or features simultaneously
- Want better visibility into their AI assistant sessions
- Prefer terminal-based tools over GUI applications

## Current Scope

Version 1.0 focuses on:
- Managing multiple Claude Code sessions
- Git repository and branch organization
- Real-time session state tracking
- Keyboard-driven navigation

Sessions are ephemeral - they exist only while Panoptes is running. Persistence and other advanced features are planned for future versions.
