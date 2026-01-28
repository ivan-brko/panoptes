# Keyboard Reference

Complete keyboard shortcut reference for Panoptes.

## Global Shortcuts

These shortcuts work in most views (except when in Session mode or text input dialogs).

| Key | Action |
|-----|--------|
| `t` | Start focus timer (opens duration input) |
| `T` | Open focus statistics view |
| `Ctrl+t` | Stop focus timer (when running) |
| `Esc` | Go back / Cancel current action |
| `q` | Quit (with confirmation in Projects Overview) |

## Projects Overview

The main view showing all projects and sessions.

### Navigation

| Key | Action |
|-----|--------|
| `Down` / `j` | Move selection down |
| `Up` / `k` | Move selection up |
| `Tab` | Toggle focus between Projects and Sessions lists |
| `1-9` | Select item by number |
| `Enter` | Open selected project or session |

### Actions

| Key | Action |
|-----|--------|
| `n` | Add new project (opens path input) |
| `a` | Open activity timeline |
| `c` | Open Claude configs management |
| `l` | Open log viewer |
| `d` | Delete selected item (project or session) |
| `q` / `Esc` | Quit (prompts for confirmation) |

## Project Detail

Shows branches within a selected project.

### Navigation

| Key | Action |
|-----|--------|
| `Down` / `j` | Move selection down |
| `Up` / `k` | Move selection up |
| `1-9` | Select branch by number |
| `Enter` | Open selected branch |
| `Esc` | Return to Projects Overview |

### Actions

| Key | Action |
|-----|--------|
| `n` | Create new worktree (opens wizard) |
| `b` | Set default base branch |
| `c` | Set default Claude config for project |
| `r` | Rename project |
| `R` | Refresh branches (check for stale worktrees) |
| `d` | Delete selected branch/worktree |
| `q` | Quit |

## Branch Detail

Shows sessions for a specific branch.

### Navigation

| Key | Action |
|-----|--------|
| `Down` / `j` | Move selection down |
| `Up` / `k` | Move selection up |
| `Enter` | Open selected session |
| `Esc` / `q` | Return to Project Detail |

### Actions

| Key | Action |
|-----|--------|
| `n` | Create new Claude Code session |
| `s` | Create new shell session |
| `d` | Delete selected session (prompts for confirmation) |

## Session View (Normal Mode)

Viewing a session without interacting with it.

### Navigation

| Key | Action |
|-----|--------|
| `PageUp` | Scroll up through history |
| `PageDown` | Scroll down through history |
| `Home` | Scroll to top (oldest) |
| `End` | Scroll to bottom (live view) |
| `Tab` | Switch to next session |
| `1-9` | Switch to session by number |
| `Esc` / `q` | Return to previous view |

### Actions

| Key | Action |
|-----|--------|
| `Enter` | Enter Session mode (interact with Claude) |

## Session View (Session Mode)

Interacting directly with Claude Code. Most keys are forwarded to the PTY.

| Key | Action |
|-----|--------|
| `Esc` | Exit to Normal mode |
| `Shift+Esc` | Send Escape to Claude Code |
| `PageUp` | Scroll up through history |
| `PageDown` | Scroll down through history |
| `Ctrl+Home` | Scroll to top |
| `Ctrl+End` | Scroll to bottom |
| All other keys | Sent to Claude Code |

**Note:** When scrolled up in history, typing any key (except scroll keys) will automatically scroll back to the live view.

## Activity Timeline

Shows all sessions sorted by recent activity.

### Navigation

| Key | Action |
|-----|--------|
| `Down` / `j` | Move selection down |
| `Up` / `k` | Move selection up |
| `Enter` | Open selected session |
| `Esc` / `q` | Return to Projects Overview |

## Log Viewer

Shows application logs for debugging.

### Navigation

| Key | Action |
|-----|--------|
| `Down` / `j` | Scroll down |
| `Up` / `k` | Scroll up |
| `PageDown` | Page down |
| `PageUp` | Page up |
| `g` | Jump to top |
| `G` | Jump to bottom (enables auto-scroll) |
| `Esc` / `q` | Return to Projects Overview |

## Claude Configurations

Manage multiple Claude Code accounts.

### Navigation

| Key | Action |
|-----|--------|
| `Down` / `j` | Move selection down |
| `Up` / `k` | Move selection up |
| `Esc` / `q` | Return to Projects Overview |

### Actions

| Key | Action |
|-----|--------|
| `n` | Add new configuration |
| `s` | Set selected as global default |
| `d` | Delete selected configuration (prompts for confirmation) |

## Focus Statistics

Shows focus timer history and statistics.

### Navigation

| Key | Action |
|-----|--------|
| `Down` / `j` | Move selection down |
| `Up` / `k` | Move selection up |
| `Enter` | View session details |
| `Esc` / `q` | Return to previous view |

### Actions

| Key | Action |
|-----|--------|
| `d` | Delete selected focus session (prompts for confirmation) |

## Worktree Creation Wizard

### Step 1: Select Branch

| Key | Action |
|-----|--------|
| Type | Filter branches / enter new branch name |
| `Down` / `j` | Move selection down |
| `Up` / `k` | Move selection up |
| `Enter` | Select branch (or create new) |
| `Esc` | Cancel |

### Step 2: Select Base (for new branches)

| Key | Action |
|-----|--------|
| Type | Filter base branches |
| `Down` / `j` | Move selection down |
| `Up` / `k` | Move selection up |
| `Enter` | Confirm selection |
| `Esc` | Go back to step 1 |

### Step 3: Confirm

| Key | Action |
|-----|--------|
| `Enter` | Create worktree |
| `Esc` | Go back |

## Text Input Dialogs

When entering text (session names, project paths, branch names):

| Key | Action |
|-----|--------|
| Type | Enter text |
| `Backspace` | Delete last character |
| `Enter` | Confirm input |
| `Esc` | Cancel |
| `Tab` | (For path input) Autocomplete path |

## Confirmation Dialogs

When prompted to confirm an action:

| Key | Action |
|-----|--------|
| `y` | Confirm |
| `n` / `Esc` | Cancel |
| `w` | (Branch delete) Toggle worktree deletion checkbox |

## Focus Timer Dialog

When setting focus timer duration:

| Key | Action |
|-----|--------|
| `0-9` | Enter duration in minutes |
| `Enter` | Start timer (or use default if empty) |
| `Esc` | Cancel |

## Mouse Support

Panoptes supports mouse scrolling in Session View:

| Mouse Action | Effect |
|--------------|--------|
| Scroll Up | Scroll up through session history |
| Scroll Down | Scroll down through session history |

When Claude Code has mouse mode enabled (e.g., in vim), mouse events are forwarded to the PTY instead.
