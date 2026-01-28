# Frequently Asked Questions

Quick answers to common questions. For detailed information, see the linked documentation.

## Table of Contents

- [Getting Started](#getting-started)
- [Projects & Worktrees](#projects--worktrees)
- [Sessions](#sessions)
- [Multiple Claude Accounts](#multiple-claude-accounts)
- [Configuration](#configuration)
- [Focus Timer](#focus-timer)
- [Permissions Sync](#permissions-sync)
- [Quick Reference](#quick-reference)

---

## Getting Started

### What is Panoptes in one sentence?

A terminal dashboard for managing multiple Claude Code sessions across different projects and branches, with real-time state tracking and attention notifications.

### What are the prerequisites?

Just [Claude Code CLI](https://claude.ai/code) installed and configured. Panoptes handles everything else.

### How do I add my first project?

1. Press `a` from the Projects Overview
2. Enter the path to a git repository (Tab for autocomplete)
3. Press Enter

### What's the typical workflow?

1. Add a project (path to git repo)
2. Navigate into the project with Enter
3. Create a worktree for your branch with `n`
4. Create a session with `n` from the branch view
5. Work with Claude Code in session mode
6. Press `Shift+Esc` to exit session mode, `Esc` to navigate back

---

## Projects & Worktrees

### How do I add a new project?

Press `a` from Projects Overview, enter the path, and press Enter.

### How do I set the default base branch for new worktrees?

From Project Detail, press `b` and select the branch (e.g., `main` or `develop`).

### How do I create a worktree for an existing branch vs. a new branch?

Press `n` from Project Detail to open the worktree wizard:
- **Existing branch**: Start typing to filter, select from the list, press Enter
- **New branch**: Type a name that doesn't match any existing branch, press Enter, then select a base branch

### How do I delete a worktree without deleting the branch?

When deleting a branch (press `d`), a confirmation dialog appears with a checkbox. By default, the worktree is deleted but the branch is preserved. Press `w` to toggle whether to also delete the git branch.

### How do I use Panoptes with monorepos (session subdirectory)?

When creating a session, you can specify a subdirectory path. The session will start in that subdirectory within the worktree.

### Where are worktrees stored on disk?

All worktrees are stored in `~/.panoptes/worktrees/`, organized by project and branch name.

### Can I move or rename the worktrees directory?

No. The worktree location is managed by Panoptes and git. Moving worktrees manually will break tracking.

---

## Sessions

### How do I create a Claude Code session?

From Branch Detail, press `n`, enter a session name, and press Enter.

### How do I create a shell session?

From Branch Detail, press `s`, enter a session name, and press Enter. Shell sessions run your default shell (bash/zsh) instead of Claude Code.

### How do I enter/exit session mode?

- **Enter**: Press Enter on a session, or when viewing a session in Normal mode
- **Exit**: Press `Esc` or `Shift+Esc`

### How do I send Escape to Claude Code (not exit session mode)?

Press `Shift+Esc`. Regular `Esc` exits session mode; `Shift+Esc` sends the Escape key to Claude Code.

### How do I switch between sessions quickly?

- **Tab**: Switch to next session (cycles through all sessions in the branch)
- **1-9**: Jump directly to session by number
- **Space**: Jump to next session needing attention (works from any view)

### How do I scroll through session history while in session mode?

- **PageUp/PageDown**: Scroll through history
- **Ctrl+Home/Ctrl+End**: Jump to top/bottom
- Typing any key (except scroll keys) automatically scrolls back to live view

### I can't copy text from the session - what's wrong?

You're in session mode. Press `Esc` to exit session mode first, then use your terminal's native text selection (mouse drag or shift+arrow keys). Session mode forwards all input to Claude Code, which prevents normal terminal selection.

### I can't scroll through the session output - what's wrong?

If you're in Normal mode (viewing but not interacting), press `Enter` to enter session mode first. Scrolling with PageUp/PageDown works in session mode. Alternatively, in Normal mode you can use PageUp/PageDown but only after entering the session view.

### What do the session states mean?

| State | Meaning |
|-------|---------|
| Starting | Session is initializing |
| Thinking | Claude is processing your request |
| Executing | Claude is running a tool (editing files, commands) |
| Waiting | Claude is waiting for your input |
| Idle | No recent activity |
| Exited | Session has ended |

Shell sessions show **Running** (command executing) or **Ready** (waiting for input).

### What do the green/yellow attention dots mean?

- **Green dot (●)**: Session just started needing your attention
- **Yellow dot (●)**: Session has been waiting longer than the idle threshold (default 5 minutes)

### How do I jump to the next session that needs my input?

Press `Space` from any view. This is the fastest way to context-switch between waiting sessions.

---

## Multiple Claude Accounts

### How do I set up multiple Claude accounts?

1. Press `c` from Projects Overview to open Claude Configs
2. Press `n` to add a new configuration
3. Enter a name and the path to the config directory
   - For a **new account**: Choose any folder; Claude will prompt for login on first use
   - For an **existing account**: Point to your existing Claude config directory (e.g., `~/.claude-work`)

### How do I set a default account for a project?

From Project Detail, press `c` and select the configuration.

### How do I switch accounts for a single session?

When creating a new session, you'll be prompted to select which configuration to use if the project has a default set.

### What happens if I don't set up any configurations?

Panoptes uses Claude Code's default configuration (`~/.claude`). Multi-account support is entirely optional.

---

## Configuration

### Where is the config file?

`~/.panoptes/config.toml`

See [Configuration Guide](CONFIG_GUIDE.md) for all options.

### How do I change notifications from bell to title change?

Add to `~/.panoptes/config.toml`:
```toml
notification_method = "title"
```

### How do I disable notifications entirely?

```toml
notification_method = "none"
```

### How do I change how long sessions stay visible after exiting?

```toml
exited_retention_secs = 60  # 1 minute instead of default 5
```

### How do I change the theme?

```toml
theme_preset = "light"  # Options: "dark", "light", "high-contrast"
```

### Do I need to restart Panoptes after config changes?

Yes. Press `q` to quit, then restart Panoptes.

---

## Focus Timer

### How do I start a focus timer?

Press `t` from any view, enter duration in minutes (or press Enter for default), and the timer starts.

### How do I change the default timer duration?

Add to `~/.panoptes/config.toml`:
```toml
focus_timer_minutes = 50  # Default is 25
```

### How do I stop a running timer?

Press `Ctrl+t` from any view.

### How do I view my focus history?

Press `T` (Shift+t) to open Focus Statistics view.

---

## Permissions Sync

### What is permissions sync?

Panoptes helps manage Claude Code's per-project permissions (tool approvals, MCP servers) across worktrees:

- **Copy to new worktrees**: When creating a worktree, Panoptes offers to copy permissions from the main repository
- **Migrate before deletion**: When deleting a worktree with unique permissions, Panoptes offers to migrate them back

### When does it happen?

- **On worktree creation**: If the main repo has Claude permissions, you're prompted to copy them
- **On worktree deletion**: If the worktree has permissions not in the main repo, you're prompted to migrate them

### Can I disable it?

The prompts are optional—you can decline each time. There's no global setting to disable them.

---

## Quick Reference

### Essential Shortcuts

| Key | Action |
|-----|--------|
| `Enter` | Open/Enter session mode |
| `Esc` | Go back/Exit session mode |
| `Shift+Esc` | Send Escape to Claude Code |
| `Space` | Jump to next session needing attention |
| `Tab` | Switch to next session |
| `n` | New (project/worktree/session depending on context) |
| `s` | New shell session (from branch view) |
| `d` | Delete selected item |
| `q` | Quit |

### Navigation Keys

| Key | Action |
|-----|--------|
| `j` / `Down` | Move down |
| `k` / `Up` | Move up |
| `1-9` | Select by number |
| `PageUp/Down` | Scroll history |

### View Shortcuts

| Key | Action |
|-----|--------|
| `a` | Activity timeline |
| `c` | Claude configs |
| `l` | Log viewer |
| `t` | Start focus timer |
| `T` | Focus statistics |

For the complete list, see [Keyboard Reference](KEYBOARD_REFERENCE.md).

---

## Still Have Questions?

1. Check the [Troubleshooting Guide](TROUBLESHOOTING.md)
2. View logs with `l` or check `~/.panoptes/logs/`
3. File an issue: https://github.com/ivan-brko/panoptes/issues
