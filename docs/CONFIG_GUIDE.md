# Configuration Guide

Panoptes stores its configuration in a TOML file at `~/.panoptes/config.toml`. If the file doesn't exist, Panoptes uses default values.

## File Location

```
~/.panoptes/config.toml
```

## Complete Example

```toml
# HTTP server port for Claude Code hooks
hook_port = 9999

# Maximum lines to keep in output buffer per session
max_output_lines = 10000

# Maximum scrollback lines per session (for terminal history)
scrollback_lines = 10000

# Seconds before a waiting session shows the yellow "idle" badge
idle_threshold_secs = 300

# Seconds before a tool still in flight is treated as stalled and evicted
# (handles cases where hook events are missed)
state_timeout_secs = 300

# Seconds to retain exited sessions before cleanup
exited_retention_secs = 300

# Seconds a session may sit idle before its agent process is suspended
# (scrollback is kept; the session wakes when you type into it). 0 disables.
suspend_after_secs = 7200

# Theme preset: "dark", "light", or "high-contrast"
theme_preset = "dark"

# Notification method when sessions need attention
# Options: "bell" (terminal bell), "title" (update terminal title), "none"
notification_method = "bell"

# Whether Claude's periodic "you have been idle" notification raises attention
attention_on_idle = false

# Which attention reasons produce a notification
[notify_on]
approval = true       # a permission dialog is blocking a turn
turn_complete = true  # an agent finished its turn
stalled = false       # a tool has been in flight far longer than expected
crashed = true        # a session's process died unexpectedly

# Milliseconds to hold Escape for exiting session mode (deprecated)
esc_hold_threshold_ms = 400

# Default focus timer duration in minutes
focus_timer_minutes = 25

# Days to retain focus session history
focus_stats_retention_days = 30

# Custom shortcuts for spawning shell sessions with predefined commands
[[custom_shortcuts]]
key = "v"
name = "VSCode"
command = "code . &"

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

### max_output_lines

| Property | Value |
|----------|-------|
| Default | `10000` |
| Type | Integer |

Maximum number of lines to keep in the output buffer for each session. Older lines are discarded when this limit is reached.

**When to change:** Increase if you need more scrollback history; decrease if you have memory constraints.

---

### scrollback_lines

| Property | Value |
|----------|-------|
| Default | `10000` |
| Type | Integer |

Maximum number of scrollback lines to retain in the terminal emulator for each session. This controls how far back you can scroll in session history using PageUp/PageDown.

Each 1000 lines uses approximately 10KB of memory per session.

**When to change:** Increase if you need to scroll back further in session history; decrease if you have many concurrent sessions and want to reduce memory usage.

---

### idle_threshold_secs

| Property | Value |
|----------|-------|
| Default | `300` (5 minutes) |
| Type | Integer (seconds) |

When a session has been in the "Waiting" state (waiting for your input) for longer than this threshold, its attention badge changes from green to yellow.

**When to change:** Decrease if you want to be notified sooner about idle sessions; increase if you prefer longer focus periods.

---

### state_timeout_secs

| Property | Value |
|----------|-------|
| Default | `300` (5 minutes) |
| Type | Integer (seconds) |

A tool that has been in flight this long without its completion event arriving is treated as stalled: it is dropped from the session's in-flight set and the session is flagged with a `Stalled` attention reason. If nothing else is running the session falls back to "Thinking".

This exists because a `PostToolUse` hook can go missing - dropped on channel overflow, or belonging to a subagent that died - and without it the session would sit in "Executing" forever.

**When to change:** Increase if you have long-running tool executions that should remain in "Executing" state longer.

---

### exited_retention_secs

| Property | Value |
|----------|-------|
| Default | `300` (5 minutes) |
| Type | Integer (seconds) |

How long to keep exited sessions before they're removed from the UI. This gives you time to review output from sessions that have ended.

**When to change:** Increase to keep exited sessions visible longer; decrease for cleaner session lists.

---

### theme_preset

| Property | Value |
|----------|-------|
| Default | `"dark"` |
| Type | String |
| Options | `"dark"`, `"light"`, `"high-contrast"` |

The color theme for the terminal UI.

- **dark** - Standard dark theme (light text on dark background)
- **light** - Light theme (dark text on light background)
- **high-contrast** - High contrast for accessibility

**When to change:** Based on your terminal's background color and personal preference.

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
that is working, blocked on a permission dialog, or currently on screen.

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
| Default | `approval = true`, `turn_complete = true`, `stalled = false`, `crashed = true` |
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
```

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

### esc_hold_threshold_ms

| Property | Value |
|----------|-------|
| Default | `400` |
| Type | Integer (milliseconds) |

**Deprecated:** This setting is no longer actively used. Escape key behavior now uses `Shift+Escape` to forward Escape to Claude Code.

---

### focus_timer_minutes

| Property | Value |
|----------|-------|
| Default | `25` |
| Type | Integer (minutes) |

The default duration for focus timer sessions when you press Enter without typing a number.

**When to change:** Adjust to match your preferred focus session length. Common values: 25 (Pomodoro), 50, 90.

---

### focus_stats_retention_days

| Property | Value |
|----------|-------|
| Default | `30` |
| Type | Integer (days) |

How long to keep focus session history. Sessions older than this are automatically pruned.

**When to change:** Increase to keep longer history; decrease to save disk space.

---

### custom_shortcuts

| Property | Value |
|----------|-------|
| Default | `[]` (empty array) |
| Type | Array of shortcut objects |

Defines custom keyboard shortcuts that spawn shell sessions with predefined commands. Each shortcut is an array entry with three fields:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `key` | character | Yes | Single character trigger (e.g., `'v'`, `'e'`) |
| `name` | string | No | Display name shown in footer (if empty, uses first 6 chars of command) |
| `command` | string | Yes | Command to run in the shell session |

**Reserved keys** (cannot be used for custom shortcuts):
- `q`, `i`, `g`, `G`, `t`, `T`, `k`, `x` - Already bound in session view
- `0-9` - Used for session number jumping

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
- Press `k` in any view to open the shortcuts management dialog
- In session view (normal mode), press the shortcut key to spawn a shell session with that command

**When to use:** Define shortcuts for commands you frequently run when working with Claude Code sessions, such as opening editors, starting dev servers, or running build tools.

---

## Data Directories

Panoptes stores data in the `~/.panoptes/` directory:

| Path | Purpose |
|------|---------|
| `~/.panoptes/config.toml` | User configuration file |
| `~/.panoptes/projects.json` | Project and branch data |
| `~/.panoptes/focus_sessions.json` | Focus timer history |
| `~/.panoptes/worktrees/` | Git worktrees created by Panoptes |
| `~/.panoptes/codex_configs.json` | Codex account configurations |
| `~/.panoptes/hooks/` | Hook scripts for agent integration |
| `~/.panoptes/logs/` | Application logs (7-day retention) |

## Creating Configuration

To create a config file with default values:

```bash
mkdir -p ~/.panoptes
cat > ~/.panoptes/config.toml << 'EOF'
hook_port = 9999
max_output_lines = 10000
idle_threshold_secs = 300
notification_method = "bell"
focus_timer_minutes = 25
EOF
```

## Reloading Configuration

Configuration changes require restarting Panoptes to take effect.

```bash
# After editing config.toml
# Press q to quit Panoptes
# Then restart it
./target/release/panoptes
```
