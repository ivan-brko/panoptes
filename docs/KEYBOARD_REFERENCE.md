# Keyboard Reference

Complete keyboard shortcut reference for Panoptes.

## Global Shortcuts

These shortcuts work in most views (except when in Session mode or text input dialogs).

| Key | Action |
|-----|--------|
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
| `1-9` | Select session by number (Sessions list only — the project tree is not numbered) |
| `Enter` | Open selected project or session; expand/collapse selected folder |
| `Right` | Expand selected folder |
| `Left` | Collapse selected folder, or jump to its parent folder |

Expanded folders are marked `▾`, collapsed ones `▸`. The footer changes to show
folder actions whenever a folder heading is selected.

### Folders

Projects can be grouped into folders, nested up to 3 levels deep. Folders are
created by moving a project into a path that does not exist yet.

| Key | Action |
|-----|--------|
| `m` | Move selected project (or folder subtree) into a folder |
| `r` | Rename selected folder |
| `d` | Remove selected folder — its contents move up one level, nothing is deleted |

In the move dialog, type a path like `Acme/Platform`, use `Tab` to autocomplete
against existing folders, and leave the input empty to move back to the root level.
Collapsed folders show a rollup of the sessions inside them, so you still see
active and attention counts without expanding.

### Actions

| Key | Action |
|-----|--------|
| `n` | Add new project (opens path input) |
| `a` | Open activity timeline |
| `c` | Open Claude configs management |
| `x` | Open Codex configs management |
| `l` | Open log viewer |
| `d` | Delete selected project or session (removes a folder when one is selected) |
| `R` | Refresh git state for all projects |
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
| `x` | Set default Codex config for project |
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
| `n` | Create new AI session (Claude Code or Codex) |
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
| `Enter` | Enter Session mode (interact with session) |

## Session View (Session Mode)

Interacting directly with the session (Claude Code, Codex, or shell). Most keys are forwarded to the PTY.

| Key | Action |
|-----|--------|
| `Esc` | Exit to Normal mode |
| `Shift+Esc` | Send Escape to the session |
| `PageUp` | Scroll up through history |
| `PageDown` | Scroll down through history |
| `Ctrl+Home` | Scroll to top |
| `Ctrl+End` | Scroll to bottom |
| All other keys | Forwarded to the session |

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

## Claude Configurations / Codex Configurations

Manage multiple Claude Code accounts (`c` from Projects Overview) or Codex accounts (`x` from Projects Overview). Both views use the same keyboard shortcuts.

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

## Mouse Support

Panoptes supports mouse scrolling in Session View:

| Mouse Action | Effect |
|--------------|--------|
| Scroll Up | Scroll up through session history |
| Scroll Down | Scroll down through session history |

When the session has mouse mode enabled (e.g., in vim), mouse events are forwarded to the PTY instead.
