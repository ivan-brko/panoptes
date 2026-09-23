# Configuration Guide

Panoptes stores its configuration in a TOML file at `~/.panoptes/config.toml`.

Every setting has a default, and the file itself is optional: if it is missing, or
only sets some of the keys, Panoptes fills in the rest. Keys it does not
recognise are ignored, so a config left over from an older version still loads.

A file that cannot be parsed at all does not prevent startup either: Panoptes
backs it up to a timestamped `config.toml.corrupt.<timestamp>` sibling, starts
with defaults, and shows a warning so you can recover your settings from the
backup.

> **Hand-written comments are not preserved.** Panoptes rewrites the whole file
> from its own state whenever something in the app changes a setting — which the
> Settings → Notifications toggles do on every keystroke, and adding or deleting
> a custom shortcut does too. The first such write drops your comments and
> reorders the keys. Everything below stays hand-edited only, so if you keep
> notes in `config.toml`, keep a copy of them somewhere else.

## File Location

```
~/.panoptes/config.toml
```

## Complete Example

```toml
# HTTP server port for Claude Code hooks
hook_port = 9999

# Where Panoptes creates git worktrees and writes agent hook scripts.
# Both default to subdirectories of ~/.panoptes/ - set them only to relocate.
worktrees_dir = "/Users/you/.panoptes/worktrees"
hooks_dir = "/Users/you/.panoptes/hooks"

# Maximum scrollback lines per session (for terminal history)
scrollback_lines = 10000

# Mouse selection: what a double-click treats as part of a word (beyond
# alphanumerics), and how long a second click may take to still count as one
selection_word_characters = "/-+\\~_."
multi_click_ms = 400

# Seconds before a tool still in flight is treated as stalled and evicted
# (handles cases where hook events are missed)
state_timeout_secs = 300

# Seconds to retain exited sessions before cleanup
exited_retention_secs = 300

# Seconds a session may sit idle before its agent process is suspended
# (scrollback is kept; the session wakes when you type into it). 0 disables.
suspend_after_secs = 7200

# Notification method when sessions need attention
# Options: "bell" (terminal bell), "title" (update terminal title), "none"
notification_method = "bell"

# Whether Claude's periodic "you have been idle" notification raises attention
attention_on_idle = false

# Route Claude's status line through Panoptes, for rate limits and the real
# context window. Your own status line still shows, unchanged; without one,
# Panoptes shows a compact line of rate limits and cost.
claude_status_line = true

# Colour-capability tier for the UI palette
# Options: "auto" (detect from COLORTERM/TERM), "truecolor", "ansi256", "ansi16"
theme = "auto"

# Which colour preset the UI wears (pick it live from Settings > Theme)
# Options: "peacock" (default), "io", "hera", "argus"
palette = "peacock"

# Which attention reasons produce a notification
[notify_on]
approval = true       # a permission dialog is blocking a turn
turn_complete = true  # an agent finished its turn
stalled = false       # a tool has been in flight far longer than expected
crashed = true        # a session's process died unexpectedly
failed = true         # a turn died on an API error (usage limit, login, overload)

# Custom shortcuts for spawning shell sessions with predefined commands
[[custom_shortcuts]]
key = "v"
name = "VSCode"
command = "code . &"
auto_close = false    # close the shell session automatically when the command finishes

[[custom_shortcuts]]
key = "e"
name = "vim"
command = "vim ."
```

## Options Reference

### hook_port

| Property | Value |
|----------|-------|
| Default | `9999` |
| Type | Integer (1-65535) |

The port number for the HTTP server that receives Claude Code hook callbacks.

**When to change:** If port 9999 is already in use by another application on your system.

**Note:** You'll need to update your Claude Code configuration to match if you change this port.

---

### worktrees_dir

| Property | Value |
|----------|-------|
| Default | `~/.panoptes/worktrees` |
| Type | Absolute path |

Where Panoptes creates git worktrees when you make a branch. Each worktree is a
full checkout, so this directory grows with the number of branches you keep.

**When to change:** To put worktrees on a different disk, or somewhere your
editor indexes more happily. Moving it does not relocate existing worktrees.

---

### hooks_dir

| Property | Value |
|----------|-------|
| Default | `~/.panoptes/hooks` |
| Type | Absolute path |

Where Panoptes writes the hook scripts it registers with Claude Code and Codex.
It rewrites them on startup, so treat this directory as generated. Codex's
per-event hooks live in its `codex/` subdirectory, and Codex is given their
paths on each session's command line rather than in `CODEX_HOME`.

**When to change:** Rarely. Mainly if `~/.panoptes/` is on a filesystem that
cannot hold executable scripts.

---

### scrollback_lines

| Property | Value |
|----------|-------|
| Default | `10000` |
| Type | Integer |

Maximum number of scrollback lines to retain in the terminal emulator for each session. This controls how far back you can scroll in session history with the mouse wheel, and how far `Ctrl+Home` jumps.

Each 1000 lines uses approximately 10KB of memory per session.

**When to change:** Increase if you need to scroll back further in session history; decrease if you have many concurrent sessions and want to reduce memory usage.

---

### selection_word_characters

| Property | Value |
|----------|-------|
| Default | `"/-+\\~_."` |
| Type | String |

Which characters a double-click treats as part of a word, on top of letters and digits. The default is iTerm2's own set, which is what makes double-clicking `src/app/mod.rs` or `--no-verify` take the whole thing instead of stopping at the first slash or dash.

Note that this is a TOML string, so a backslash has to be written `\\`.

**When to change:** Add characters your work is full of — `@` for email addresses or scoped npm packages, `:` for `host:port` pairs — or empty it (`""`) to make a double-click take only the alphanumeric run.

---

### multi_click_ms

| Property | Value |
|----------|-------|
| Default | `400` |
| Type | Integer (milliseconds) |

How long after a click a second one still counts as a double-click, and a third as a triple-click. A repeat also has to land within one cell of the previous click.

**When to change:** Match it to your system's double-click speed if double-clicking to select a word feels like it needs hurrying, or if ordinary separate clicks keep being read as one gesture.

---

### state_timeout_secs

| Property | Value |
|----------|-------|
| Default | `300` (5 minutes) |
| Type | Integer (seconds) |

A tool that has been in flight this long without its completion event arriving stops being believed: it is dropped from the session's in-flight set, and if nothing else is running the session falls back to "Thinking".

This exists because a `PostToolUse` hook can go missing - dropped on channel overflow, or belonging to a subagent that died - and without it the session would sit in "Executing" forever.

Being overdue is not by itself a reason to interrupt you. No threshold can tell a ten-minute build from a hang, so the `Stalled` badge is raised only when the session has *also* stopped producing output for half a minute - and never for the session you are currently looking at. A session still drawing its spinner is long-running, not stalled, and is retired quietly.

**When to change:** Increase if you want tools to stay visible in the in-flight set (and the session in "Executing") for longer before Panoptes stops believing the report.

---

### exited_retention_secs

| Property | Value |
|----------|-------|
| Default | `300` (5 minutes) |
| Type | Integer (seconds) |

How long to keep exited sessions before they're removed from the UI. This gives you time to review output from sessions that have ended.

**When to change:** Increase to keep exited sessions visible longer; decrease for cleaner session lists.

---

### notification_method

| Property | Value |
|----------|-------|
| Default | `"bell"` |
| Type | String |
| Options | `"bell"`, `"title"`, `"none"` |

How Panoptes notifies you when a session needs attention.

- **bell** - Send terminal bell character (produces a sound or visual indicator depending on your terminal)
- **title** - Update the terminal title to indicate attention needed
- **none** - No notifications

Any other value logs a warning and falls back to `"bell"`.

**When to change:** Use `"title"` if you find the bell annoying; use `"none"` if you don't want interruptions.

---

### suspend_after_secs

| Property | Value |
|----------|-------|
| Default | `7200` (2 hours) |
| Type | Integer (seconds) |
| Disable with | `0` |

An idle Claude Code process uses roughly 565 MB - about 25x the entire Panoptes
process. After this long without engagement, the agent's process is killed and
the session shows as `Suspended`.

The scrollback is kept and stays scrollable, so reading a suspended session
costs nothing. Typing into one wakes it: the agent is relaunched against the
same conversation, which takes around two seconds. The on-screen history is not
restored at that point, though the conversation itself is fully intact.

Shell sessions are never suspended - they have no conversation to come back to,
and killing one would end a running build or dev server. Neither is a session
that is working, blocked on a permission dialog, or currently on screen - nor
one whose finished turn left work behind: subagents, background shells or
monitors, or a scheduled prompt such as `/loop`, all of which the kill would end.

**When to change:** Lower it if you keep many sessions open and are short on
memory; raise it, or set `0`, if you would rather never wait for a wake.

---

### log_agent_events

| Property | Value |
|----------|-------|
| Default | `false` |
| Type | Boolean |

Writes every raw line Panoptes reads from an agent's transcript to
`~/.panoptes/logs/agent-events/<session-id>.ndjson`.

Turn this on when a session's state looks wrong. The log holds exactly what the
agent wrote, so what Panoptes concluded can be checked against what it was
given. Leave it off otherwise - it grows with every tool call.

---

### notify_on

| Property | Value |
|----------|-------|
| Default | `approval = true`, `turn_complete = true`, `stalled = false`, `crashed = true`, `failed = true` |
| Type | Table of booleans |

Which reasons for wanting your attention are worth interrupting you for.

Every reason still raises the badge in the session list; these control only the
notification configured by `notification_method`. The split is deliberate - a
stalled tool is worth showing in the list but rarely worth a sound, since
nothing is blocked on you and the watchdog is only guessing.

```toml
[notify_on]
approval = true
turn_complete = true
stalled = false
crashed = true
failed = true
```

`failed` covers a turn that died on an API error - a usage limit, an expired
login, an overloaded API - rather than finishing. The agent is still running and
back at its prompt, so it is kept apart from `crashed`: the fix is usually to
wait or log in, not to restart anything. Claude Code only, for now.

**When to change:** Set `turn_complete = false` if you run many agents at once
and only want to hear about the ones that are actually blocked on you.

---

### attention_on_idle

| Property | Value |
|----------|-------|
| Default | `false` |
| Type | Boolean |

Claude sends a `Notification` after roughly a minute of an unattended prompt.
It uses the same event type it uses to say a permission dialog is open, which
is why Panoptes once treated the two alike and rang for both.

With this off, the idle reminder is ignored entirely: a session you already
know is waiting does not need to keep telling you. Turn it on if you want the
reminder back.

---

### claude_status_line

| Property | Value |
|----------|-------|
| Default | `true` |
| Type | Boolean |

Claude Code reports its plan rate limits (the five-hour and weekly windows),
and the context window the session is really running with, only to a
`statusLine` command. With this on, Panoptes installs one in the working
directory's `.claude/settings.local.json`, next to its hooks. That setting
outranks yours, so Panoptes' command *wraps* your status line: it forwards the
figures to Panoptes, then runs your own command on the same input and prints
exactly what it prints. Your status line looks the same as without Panoptes.

- Your status line is found where Claude looks: `.claude/settings.local.json`,
  then `.claude/settings.json`, then `settings.json` in the session's Claude
  config directory (`CLAUDE_CONFIG_DIR`, else `~/.claude`). Its `padding` and
  `refreshInterval` are kept.
- With no status line of your own, Panoptes fills the row with a compact line
  of its own, for example:

  ```
  5h 12% · wk 40% (resets Thu 18:00) · $2.14
  ```

  That is how much of Claude's five-hour and weekly allowances you have used,
  when the one closer to running out resets (local time: `18:00` for later
  today, `Thu 18:00` for another day), and what the session has cost so far.
  Anything Claude has not reported yet is left out - before the first reply of
  a session there are no rate limits, so it shows just the cost.
- Configure a status line of your own and it replaces the compact line
  completely; Panoptes only draws one when you have none.
- A status line you set in `.claude/settings.local.json` is remembered inside
  Panoptes' command and put back when you turn this off. One set anywhere else
  is never touched, and is re-read at every spawn.
- Like the hooks, the setting stays after the session ends. Outside Panoptes it
  just runs your own command.

Set it to `false` to opt out: the next Claude session spawned in a directory
puts back whatever Panoptes wrapped there (or removes the compact line), and
Claude sessions show model and context only, with the window inferred from the
model name. Read when a session
spawns; shown under **Settings → About / paths**.

---

### theme

| Property | Value |
|----------|-------|
| Default | `"auto"` |
| Type | String: `"auto"`, `"truecolor"`, `"ansi256"`, `"ansi16"` |

Which colour-capability tier the UI palette uses. The tiers agree on every
colour that carries meaning - session states, attention badges, the accent -
and differ only in the structural greys: the richer tiers can dim unfocused
pane chrome and tint the selected row, where 16 colours cannot.

- `auto` detects the tier from `COLORTERM` (`truecolor` / `24bit`) with
  `TERM` as the backstop (`*-256color`, `*-direct`)
- `truecolor` / `ansi256` / `ansi16` force a tier, for when detection is wrong

`ansi16` is the always-safe baseline and exactly the classic appearance; use
it if the UI looks off over SSH or in an unusual terminal.

**When to change:** Only if auto-detection picks the wrong tier - for example
a terminal that supports truecolor but does not advertise it.

---

### palette

| Property | Value |
|----------|-------|
| Default | `"peacock"` |
| Type | String: `"peacock"`, `"io"`, `"hera"`, `"argus"` |

Which colour preset the UI wears. Orthogonal to [`theme`](#theme): that picks
how *many* colours the terminal can show, this picks *which* ones.

| Preset | Look |
|--------|------|
| `peacock` | Cyan and blue - the hundred eyes on the tail, and the look Panoptes has always had |
| `io` | Warm amber and gold - the heifer he guarded |
| `hera` | Royal violet - the goddess he served |
| `argus` | Green - the watcher himself |

A preset restyles the **chrome** and never the **semantics**: the accent, the
focused pane border, the selected-row surface, the input prompt and the
`★` default markers all change, while green still means waiting, yellow still
means thinking and red still means crashed. A session list reads identically
in all four, which is the point - the preset is a skin, not a second language.

Each preset lands hardest on truecolor, where its selected-row and text
selection surfaces are tinted toward its hue. On `ansi16` a preset can only
reassign named colours, so the four are distinguishable but modest there.

**Changing it:** open **Settings > Theme** in pane 3. Moving through the list
with `Up`/`Down` applies the preset to the whole dashboard immediately - the
UI is the preview - `Enter` keeps it and writes it here, and `Esc` puts back
whatever was saved. So does leaving the section any other way: the preview
lasts exactly as long as you are looking at the picker. Editing this key by hand works too and takes effect on the
next start.

---

### custom_shortcuts

| Property | Value |
|----------|-------|
| Default | `[]` (empty array) |
| Type | Array of shortcut objects |

Defines custom keyboard shortcuts that spawn shell sessions with predefined commands. Each shortcut is an array entry with these fields:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `key` | character | Yes | Single character trigger (e.g., `'v'`, `'e'`) |
| `name` | string | No | Display name shown in footer (if empty, uses first 6 chars of command) |
| `command` | string | Yes | Command to run in the shell session |
| `auto_close` | bool | No | Default `false`. When `true`, the shortcut's shell session closes automatically once its command finishes |

**Reserved keys** (cannot be used for custom shortcuts):
- `q` - Quit, handled globally in normal mode
- `n`, `s`, `d` - New / shell / delete, bound in panes 1 and 2
- `i` - Import a conversation, bound at a branch in pane 1
- `0-9` - Used for session number jumping

`c`, `g`, `G`, `k` and `x` used to be reserved and are now free: the configs,
shortcuts and log viewer they belonged to have moved into the Settings pane,
which is reached with `Tab` rather than a letter. `,` is free for the same kind
of reason: per-project settings are the last row of a project's branch list, so
no key opens them.

A shortcut bound to a key that has since become reserved is **dropped** when
Panoptes starts, and a startup notice says which ones went — it is never left in
place to be silently shadowed by the built-in binding.

**Example:**

```toml
[[custom_shortcuts]]
key = "v"
name = "VSCode"
command = "code . &"

[[custom_shortcuts]]
key = "e"
name = ""  # Will show "vim ." in footer
command = "vim ."

[[custom_shortcuts]]
key = "w"
name = "Watch"
command = "npm run dev"
```

**Managing shortcuts:**
- Settings pane → Shortcuts: `n` adds one, `d` deletes the selected one
- At a branch, press the shortcut key to spawn
  a shell session with that command

**When to use:** Define shortcuts for commands you frequently run when working with Claude Code sessions, such as opening editors, starting dev servers, or running build tools.

---

## Data Directories

Panoptes stores data in the `~/.panoptes/` directory:

| Path | Purpose |
|------|---------|
| `~/.panoptes/config.toml` | User configuration file |
| `~/.panoptes/projects.json` | Project and branch data |
| `~/.panoptes/sessions.json` | Persisted sessions (recovered across restarts) |
| `~/.panoptes/claude_configs.json` | Claude Code account configurations |
| `~/.panoptes/codex_configs.json` | Codex account configurations |
| `~/.panoptes/worktrees/` | Git worktrees created by Panoptes |
| `~/.panoptes/hooks/` | Hook scripts for agent integration |
| `~/.panoptes/logs/` | Application logs (7-day retention) |

## Project Folders

Projects in the overview can be grouped into folders, nested up to 3 levels deep.
Folders are not separate records — each project stores the folder path it belongs
to, and a folder exists as long as at least one project references it.

Normally you manage this from the UI (`m` to move, `r` to rename, `d` to remove a
folder — see the [Keyboard Reference](KEYBOARD_REFERENCE.md)), but the underlying
fields in `~/.panoptes/projects.json` are editable by hand:

```json
{
  "projects": [
    {
      "name": "auth-service",
      "folder": ["Acme", "Platform"]
    },
    {
      "name": "personal-blog",
      "folder": []
    }
  ],
  "collapsed_folders": ["Acme/Platform"]
}
```

| Field | Purpose |
|-------|---------|
| `folder` | Folder path segments for a project. `[]` (or omitted) puts it at the root level. Max 3 segments. |
| `collapsed_folders` | Display paths of folders currently collapsed in the overview. Entries for folders that no longer hold projects are pruned on save. |

Files written before this feature existed load unchanged: a missing `folder` is
treated as the root level.

## Creating Configuration

To create a config file with default values:

```bash
mkdir -p ~/.panoptes
cat > ~/.panoptes/config.toml << 'EOF'
hook_port = 9999
scrollback_lines = 10000
notification_method = "bell"
EOF
```

## Reloading Configuration

Seven settings can be changed while Panoptes is running, from **Settings →
Notifications**. They take effect on the next event, with no restart:

| Row | Field |
|-----|-------|
| Notify me by | `notification_method` |
| …on approval needed | `notify_on.approval` |
| …on turn finished | `notify_on.turn_complete` |
| …on tool stalled | `notify_on.stalled` |
| …on session crashed | `notify_on.crashed` |
| …on turn failed | `notify_on.failed` |
| Idle nudge counts as attention | `attention_on_idle` |

The colour preset is live too, from **Settings → Theme**: `Up`/`Down` repaints
the whole UI on the spot, `Enter` writes `palette`, `Esc` puts the saved one
back, as does leaving the section any other way. The colour *tier* (`theme`) is not - it is a property of the terminal,
settled once at startup.

Everything else is read at startup or when a session is spawned, and needs a
restart. Those settings are shown read-only under **Settings → About / paths**,
alongside where each file lives.

```bash
# After hand-editing config.toml
# Press q to quit Panoptes (confirm when prompted), then restart it
./target/release/panoptes
```
