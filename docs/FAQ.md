# Frequently Asked Questions

Quick answers to common questions. For detailed information, see the linked documentation.

## Table of Contents

- [Getting Started](#getting-started)
- [Projects & Worktrees](#projects--worktrees)
- [Sessions](#sessions)
- [Multiple Accounts](#multiple-accounts)
- [Configuration](#configuration)
- [Permissions Sync](#permissions-sync)
- [Quick Reference](#quick-reference)

---

## Getting Started

### What is Panoptes in one sentence?

A terminal dashboard for managing multiple AI coding agent sessions (Claude Code and Codex CLI) across different projects and branches, with real-time state tracking and attention notifications.

### What are the prerequisites?

At minimum, [Claude Code CLI](https://claude.ai/code) installed and configured. Optionally, [OpenAI Codex CLI](https://github.com/openai/codex) for Codex session support. Panoptes handles everything else.

### How do I add my first project?

1. Press `n` in the Projects pane
2. Enter the path to a git repository (Tab for autocomplete)
3. Press Enter

### What's the typical workflow?

1. Add a project (path to git repo)
2. Navigate into the project with Enter
3. Create a worktree for your branch with `n`
4. Create a session with `n` from the branch view
5. Work with Claude Code in session mode
6. Press `Esc` to exit session mode, then `Esc` again to navigate back (`Shift+Esc` sends an Escape keypress to the agent instead)

---

## Projects & Worktrees

### How do I add a new project?

Press `n` in the Projects pane, enter the path, and press Enter.

### How do I set the default base branch for new worktrees?

Open the project, press `,` for its settings, choose "Default base branch", and select the branch (e.g. `main` or `develop`).

### How do I create a worktree for an existing branch vs. a new branch?

Open the project and press `n` to open the worktree wizard:
- **Existing branch**: Start typing to filter, select from the list, press Enter
- **New branch**: Type a name that doesn't match any existing branch, press Enter, then select a base branch

### How do I delete a worktree without deleting the branch?

Press `d` on a worktree in the project's branch list. A confirmation dialog appears with a checkbox that `w` toggles to control whether the worktree's directory is removed from disk. Either way, deletion only removes the worktree — it never deletes the git branch itself.

### How do I use Panoptes with monorepos (session subdirectory)?

When creating a session, you can specify a subdirectory path. The session will start in that subdirectory within the worktree.

### Where are worktrees stored on disk?

All worktrees are stored in `~/.panoptes/worktrees/`, organized by project and branch name.

### Can I move or rename the worktrees directory?

No. The worktree location is managed by Panoptes and git. Moving worktrees manually will break tracking.

---

### How do I group related projects together?

Select a project in the Projects pane, press `m`, and type a folder path such as
`Acme/Platform`. Folders are created as you name them - there is no separate
"new folder" step - and can nest up to 3 levels. `Tab` autocompletes against
folders you already have, and an empty path moves a project back to the top level.

### How do I rename or get rid of a folder?

Select the folder heading and press `r` to rename it, or `d` to remove it. Removing
a folder never deletes anything: its contents move up one level. Pressing `m` on a
folder heading moves the whole subtree somewhere else.

### Do I lose the folder layout when I restart?

No. Folders live in `~/.panoptes/projects.json` alongside the projects themselves,
and which folders you had collapsed is remembered too.

## Sessions

### Does Panoptes support Codex CLI?

Yes! Press `n` at a branch and select "Codex" from the agent type selector. Codex sessions work the same way as Claude Code sessions, with the same attention tracking and notification system.

### How do I create a Claude Code or Codex session?

At a branch, press `n`, select Claude Code or Codex from the agent type selector, enter a session name, and press Enter.

### How do I create a shell session?

At a branch, press `s`, enter a session name, and press Enter. Shell sessions run your default shell (bash/zsh) instead of Claude Code.

### How do I enter/exit session mode?

- **Enter**: Press Enter on a session, or when viewing a session in Normal mode
- **Exit**: Press `Esc` (`Shift+Esc` does not exit — it sends an Escape keypress to the session)

### How do I send Escape to the session (not exit session mode)?

Press `Shift+Esc`. Regular `Esc` exits session mode; `Shift+Esc` sends the Escape key to the active session (Claude Code, Codex, or shell).

### How do I switch between sessions quickly?

- **Tab**: Switch to next session (cycles through all sessions in the branch)
- **1-9**: Jump directly to session by number
- **Space**: Jump to next session needing attention (works from any view)

### How do I scroll through session history while in session mode?

- **PageUp/PageDown**: Scroll through history
- **Ctrl+Home/Ctrl+End**: Jump to top/bottom
- Typing any key (except scroll keys) automatically scrolls back to live view

### I can't copy text from the session - what's wrong?

You're in session mode. Press `Esc` to exit session mode first, then use your terminal's native text selection (mouse drag or shift+arrow keys). Session mode forwards all input to the active session, which prevents normal terminal selection.

### I can't scroll through the session output - what's wrong?

If you're in Normal mode (viewing but not interacting), press `Enter` to enter session mode first. Scrolling with PageUp/PageDown works in session mode. Alternatively, in Normal mode you can use PageUp/PageDown but only after entering the session view.

### What do the session states mean?

| State | Meaning |
|-------|---------|
| Starting | Session is initializing |
| Thinking | Agent is processing your request |
| Executing | Agent is running a tool (editing files, commands) |
| Needs approval | Agent is asking for permission to proceed |
| Waiting | Agent is waiting for your input |
| Suspended | Session process suspended after inactivity (wakes on interaction) |
| Exited | Session has ended |
| Resumable | Session recovered from a previous run and can be resumed |

Shell sessions show **Running** (command executing) or **Ready** (waiting for input).

### What do the coloured attention dots mean?

The colour says *why* the session wants you:

- **Green dot (●)**: The turn finished — the agent is waiting for your next prompt
- **Yellow dot (●)**: Blocked on you (a permission dialog) or a tool that stopped reporting
- **Red dot (●)**: The agent process died

A dot always means something happened. Opening the session clears it, and it does not come back on its own.

### How do I jump to the next session that needs my input?

Press `Space` from any view. This is the fastest way to context-switch between waiting sessions.

---

## Multiple Accounts

### How do I set up multiple Claude accounts?

1. Open Settings (`Tab` twice) → Claude configs
2. Press `n` to add a new configuration
3. Enter a name and the path to the config directory
   - For a **new account**: Choose any folder; Claude will prompt for login on first use
   - For an **existing account**: Point to your existing Claude config directory (e.g., `~/.claude-work`)

### How do I set up multiple Codex accounts?

1. Open Settings (`Tab` twice) → Codex configs
2. Press `n` to add a new configuration
3. Enter a name and the path to the `CODEX_HOME` directory
   - For a **new account**: Choose any folder; Codex will use it for config and auth
   - For an **existing account**: Point to your existing Codex home directory (default `~/.codex`)

### How do I set a default account for a project?

Open the project, press `,` for its settings, and choose "Default Claude config" or "Default Codex config".

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

You cannot yet. Only the dark theme is implemented. The old `theme_preset`
config key has been removed; like any unrecognised key, it is ignored if left
in the file.

### Do I need to restart Panoptes after config changes?

Yes. Press `q` to quit (confirm when prompted), then restart Panoptes.

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
| `→` / `←` | Switch pane (Projects → Sessions → Settings, wrapping) |
| `Tab` / `Shift+Tab` | Same thing — switch to the next / previous pane |
| `Enter` | Open / Enter session mode |
| `Esc` | Back one level; at a pane's root, back out to the Projects pane. Never quits |
| `q` | Quit (with confirmation) |
| `Shift+Esc` | Send Escape to active session |
| `Space` | Jump to next session needing attention |
| `n` | New (project/worktree/session depending on context) |
| `s` | New shell session (at a branch) |
| `d` | Delete selected item |
| `,` | Per-project settings (at a project) |

### Navigation Keys

| Key | Action |
|-----|--------|
| `Down` | Move down |
| `Up` | Move up |
| `1-9` | Select by number (not in the project tree) |
| `PageUp/Down` | Scroll history |
| `?` | Show the keys for wherever you are |

Navigation is by arrow key everywhere; there are no `j`/`k` bindings.

### Where the settings live

Claude configs, Codex configs, custom shortcuts, notification toggles, and the
paths of every file Panoptes writes are all in the **Settings** pane — press
`Tab` twice to reach it.

For the complete list, see [Keyboard Reference](KEYBOARD_REFERENCE.md).

---

## Still Have Questions?

1. Check the [Troubleshooting Guide](TROUBLESHOOTING.md)
2. Read the logs in `~/.panoptes/logs/` (Settings → About / paths names the current file)
3. File an issue: https://github.com/ivan-brko/panoptes/issues
