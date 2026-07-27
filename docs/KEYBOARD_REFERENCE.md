# Keyboard Reference

Complete keyboard shortcut reference for Panoptes.

Panoptes shows three panes at once — **Projects**, **Sessions**, **Settings** —
and one of them holds focus. Opening a session is the only thing that takes over
the whole terminal.

## Global Shortcuts

These work from every pane, in normal mode. Any other input mode (a text field,
a confirmation dialog, an autocomplete) owns these keys itself.

| Key | Action |
|-----|--------|
| `Right` / `Left` | Switch to the next / previous pane (wraps around) |
| `Tab` / `Shift+Tab` | Same thing — switch to the next / previous pane |
| `q` | Quit (prompts for confirmation) |
| `?` | Show the shortcuts for wherever you are (`?` or `Esc` closes it) |
| `Space` | Jump to the next session needing attention |
| `Esc` | Go back one level in the focused pane; with nothing left to pop, back out to the Projects pane. Never quits |

`Right` / `Left` match the panes' left-to-right order, and are exact synonyms
for `Tab` / `Shift+Tab`. No pane claims them for anything else, which is why
they can be global without exceptions.

These switch panes **only** in normal mode. In a path input `Tab` completes a
path; in Session mode it types a tab into the agent, and the arrows go to the
agent too. In a confirmation dialog `Left` / `Right` toggle Yes / No. In the
Settings pane's Notifications section, `Space` toggles the highlighted option
instead of jumping.

Vertical navigation is by arrow key throughout: `Up` / `Down` move within a
pane, and `Enter` is the only key that acts on what is selected. There are no
`j`/`k` bindings.

## Pane 1 — Projects

The project tree, and three levels beneath it.

### Projects (the tree)

| Key | Action |
|-----|--------|
| `Up` / `Down` | Move selection |
| `Enter` | Open selected project; expand/collapse selected folder |
| `n` | Add new project (opens the path prompt) |
| `d` | Delete selected project — or ungroup a folder, which deletes nothing |
| `m` | Move selected project (or folder subtree) into a folder |
| `r` | Rename selected folder |
| `R` | Refresh git state for all projects |

Expanded folders are marked `▾`, collapsed ones `▸`. The footer changes to show
folder actions whenever a folder heading is selected. Collapsed folders show a
rollup of the sessions inside them, so you still see active and attention counts
without expanding.

Projects can be grouped into folders, nested up to 3 levels deep. Folders are
created by moving a project into a path that does not exist yet. In the move
prompt, type a path like `Acme/Platform`, use `Tab` to autocomplete against
existing folders, and leave the input empty to move back to the root level.

### Project (its branches)

Row 0 is the back row, `‹ Projects` — the way out, made visible. It takes the
selection like any other row, and `Enter` on it is the same action as `Esc`.
Every level below applies the same rule, naming its own destination.

| Key | Action |
|-----|--------|
| `Up` / `Down` | Select a branch (`Up` from the first reaches the back row) |
| `Enter` | Open selected branch; on the back row, go back to the tree |
| `n` | Create new worktree (opens the wizard) |
| `d` | Delete selected worktree (never deletes the git branch) |
| `R` | Refresh branches (check for stale worktrees) |
| `,` | Project settings |
| `Esc` | Back to the tree |

### Project settings (`,`)

Per-project defaults. Replaces the old `c`, `x`, `b` and `r` keys.

| Row | What it opens |
|-----|---------------|
| Default Claude config | Claude config selector |
| Default Codex config | Codex config selector |
| Default base branch | Branch-ref selector |
| Rename project | One-line input |

| Key | Action |
|-----|--------|
| `Up` / `Down` | Move selection (row 0 is `‹ <project name>`) |
| `Enter` | Open the selected setting; on the back row, back to the branch list |
| `Esc` | Back to the branch list |

### Branch (its sessions)

| Key | Action |
|-----|--------|
| `Up` / `Down` | Select a session (row 0 is `‹ <project name>`) |
| `Enter` | Open selected session (resumes it if `[Resumable]`); on the back row, back to the branch list |
| `n` | Create new AI session (Claude Code or Codex) |
| `s` | Create new shell session |
| `d` | Delete selected session (prompts for confirmation) |
| `Esc` | Back to the branch list |
| any other key | Run a matching custom shortcut, if one is bound |

## Pane 2 — Sessions

Every session, flat and sorted, with a pinned "Needs Attention" section on top.

| Key | Action |
|-----|--------|
| `Up` / `Down` / `1-9` | Select a session (`0` = 10) |
| `Enter` | Open the selected session full-screen |
| `d` | Delete the selected session (prompts for confirmation) |
| `Esc` | Back out to the Projects pane |

## Pane 3 — Settings

Five sections. The highlighted row carries its own description, muted, just
after the label; if the pane is too narrow for both, the description scrolls
past on its own while the label stays put.

| Key | Action |
|-----|--------|
| `Up` / `Down` | Move through the sections |
| `Enter` | Open the selected section |
| `Esc` | Back to the sections list from inside a section; from the sections list, out to the Projects pane |

### Claude configs / Codex configs

Manage multiple Claude Code accounts (`CLAUDE_CONFIG_DIR`) or Codex accounts
(`CODEX_HOME`). Both sections use the same keys.

| Key | Action |
|-----|--------|
| `Up` / `Down` | Move selection |
| `n` | Add new configuration |
| `s` | Set selected as global default |
| `d` | Delete selected configuration (prompts for confirmation) |

### Shortcuts

Custom keys that launch a shell command.

| Key | Action |
|-----|--------|
| `Up` / `Down` | Move selection |
| `n` | Bind a key to a command |
| `d` | Delete the selected shortcut (prompts for confirmation) |

### Notifications

Six live toggles. Each takes effect on the **next** event — no restart — and is
written to `config.toml` immediately.

| Key | Action |
|-----|--------|
| `Up` / `Down` | Move through the rows |
| `Space` / `Enter` | Toggle the highlighted option — on the first row, advance how you are notified (Bell → Title → Silent, wrapping) |

### About / paths

Read-only: version, hook server port and health, and where `config.toml`,
`logs/`, `projects.json`, `sessions.json`, `worktrees/` and `hooks/` live. The
settings that are only read at startup are shown here too. Edit `config.toml`
by hand to change them.

## Session View

The session fills the terminal and there is **one mode**: nearly every key goes
straight to the agent — including `q`, `Tab`, `Space`, the arrow keys, the
digits and `PageUp`/`PageDown`. Panoptes keeps three keys and no more.

| Key | Action |
|-----|--------|
| `Esc` | Leave the session, back to the pane it was opened from. One press |
| `Shift+Esc` | Send a literal Escape to the agent |
| `Ctrl+Home` | Jump to the oldest line Panoptes still holds |
| `Ctrl+End` | Back to live output |
| All other keys | Forwarded to the agent |

**The two jumps step aside for a program drawing its own screen** (Claude
Code's UI, `vim`, `less`). There is no Panoptes scrollback behind such a
program, so the keys go to it instead — which is what makes `vim`'s `gg` and
`G` keep working.

Ordinary scrolling is the mouse wheel's job. When the agent has asked for the
mouse, the wheel goes to the agent and you scroll with its own history, exactly
as in a plain terminal tab.

### Mouse

| Gesture | Action |
|-----|--------|
| Drag | Select and copy on release |
| `⇧`+Drag | The same, over an agent that owns the mouse |
| `Ctrl`+Drag | Select a rectangle rather than a stream of text |
| Double / triple click | Select a word / a whole logical line |

Typing anything while scrolled back returns you to live output.

**Switching sessions, custom shortcuts and `q` to quit live in the panes.**
Press `Esc` first.

## Worktree Creation Wizard

### Step 1: Select Branch

Letters typed here go into the filter, so only the arrow keys navigate.

| Key | Action |
|-----|--------|
| Type | Filter branches / enter new branch name |
| `Up` / `Down` | Move selection |
| `Enter` | Select branch (or create new) |
| `Esc` | Cancel |

### Step 2: Select Base (for new branches)

| Key | Action |
|-----|--------|
| Type | Filter base branches |
| `Up` / `Down` | Move selection |
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
| `w` | (Worktree delete) Toggle deleting the worktree directory from disk |

## Reserved Keys

Custom shortcuts cannot be bound to `q`, `n`, `s`, `d`, `,` or the digits `0-9`:
those are built-in where custom shortcuts fire, so a shortcut on one could never
run. `Space`, `Esc`, `Enter`, `Tab` and the arrow keys are not characters and
cannot be bound at all.

A shortcut bound to a key that has since become reserved is dropped when
Panoptes starts, and a startup notice says which ones went.

## Mouse Support

Panoptes supports mouse scrolling in Session View:

| Mouse Action | Effect |
|--------------|--------|
| Scroll Up | Scroll up through session history |
| Scroll Down | Scroll down through session history |

When the session has mouse mode enabled (e.g., in vim), mouse events are
forwarded to the PTY instead.
