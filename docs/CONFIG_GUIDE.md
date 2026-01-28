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

# Seconds before an Executing state auto-transitions to Idle
# (handles cases where hook events are missed)
state_timeout_secs = 300

# Seconds to retain exited sessions before cleanup
exited_retention_secs = 300

# Theme preset: "dark", "light", or "high-contrast"
theme_preset = "dark"

# Notification method when sessions need attention
# Options: "bell" (terminal bell), "title" (update terminal title), "none"
notification_method = "bell"

# Milliseconds to hold Escape for exiting session mode (deprecated)
esc_hold_threshold_ms = 400

# Default focus timer duration in minutes
focus_timer_minutes = 25

# Days to retain focus session history
focus_stats_retention_days = 30
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

Sessions in the "Executing" state that don't receive updates for this long automatically transition to "Idle". This handles cases where hook events might be missed.

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

## Data Directories

Panoptes stores data in the `~/.panoptes/` directory:

| Path | Purpose |
|------|---------|
| `~/.panoptes/config.toml` | User configuration file |
| `~/.panoptes/projects.json` | Project and branch data |
| `~/.panoptes/focus_sessions.json` | Focus timer history |
| `~/.panoptes/worktrees/` | Git worktrees created by Panoptes |
| `~/.panoptes/hooks/` | Hook scripts for Claude Code integration |
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
