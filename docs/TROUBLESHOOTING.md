# Troubleshooting Guide

This guide covers common issues and their solutions.

## Hook Server Issues

### Port 9999 Already in Use

**Symptoms:**
- Error message on startup: "Address already in use"
- Sessions don't update their state

**Solutions:**

1. Check what's using the port:
   ```bash
   lsof -i :9999
   ```

2. Either stop the conflicting service, or change Panoptes' port:
   ```bash
   # Edit config
   echo 'hook_port = 9998' >> ~/.panoptes/config.toml
   ```

3. If you changed the port, also update your Claude Code hooks to use the new port.

### Sessions Not Updating State

**Symptoms:**
- Sessions remain stuck in "Starting" state
- State changes don't reflect in UI

**Solutions:**

1. Check that the hook server is running (look for startup message in logs)

2. View logs for errors:
   - Press `l` in Panoptes to open log viewer
   - Or check `~/.panoptes/logs/`

3. Verify Claude Code hooks are configured correctly:
   - Hooks should POST to `http://localhost:9999/hook`

4. If the hook server crashed, restart Panoptes

### Hook Server Stopped

**Symptoms:**
- Header shows "Hook server stopped - session state updates unavailable"

**Solutions:**

1. Restart Panoptes
2. Check logs for the error that caused the shutdown
3. Ensure port 9999 (or your configured port) is available

---

## Git/Worktree Issues

### Failed to Create Worktree

**Symptoms:**
- "Failed to create worktree" error when using `n` in Project Detail

**Possible causes and solutions:**

1. **Branch already exists:**
   - Use a different branch name
   - Or select the existing branch from the list

2. **Worktree path already exists:**
   - Check `~/.panoptes/worktrees/`
   - Remove the existing directory if it's stale

3. **Git repository issues:**
   - Ensure you're in a valid git repository
   - Try running `git status` in the repo manually

### Worktree Marked as Missing

**Symptoms:**
- Branch shows red with "(missing)" indicator

**Explanation:**
The worktree directory was deleted outside of Panoptes.

**Solutions:**

1. Press `R` to refresh and confirm the status
2. To remove from Panoptes: select and press `d`
3. To recreate: delete first, then create new worktree

### Permission Errors

**Symptoms:**
- "Permission denied" errors during git operations

**Solutions:**

1. Check ownership of directories:
   ```bash
   ls -la ~/.panoptes/
   ```

2. Fix permissions if needed:
   ```bash
   chmod -R u+rw ~/.panoptes/
   ```

3. Ensure you have write access to the git repository

---

## Display Issues

### Terminal Corruption After Exit

**Symptoms:**
- Terminal displays garbage after quitting Panoptes
- Cursor is invisible
- Keyboard input doesn't show

**Solutions:**

1. Run the reset command:
   ```bash
   reset
   ```

2. Or use stty to restore settings:
   ```bash
   stty sane
   ```

### Colors Look Wrong

**Symptoms:**
- UI colors are hard to read
- Text is invisible on some backgrounds

**Solutions:**

1. Try a different theme:
   ```bash
   echo 'theme_preset = "light"' >> ~/.panoptes/config.toml
   # or
   echo 'theme_preset = "high-contrast"' >> ~/.panoptes/config.toml
   ```

2. Ensure your terminal supports 256 colors:
   ```bash
   echo $TERM
   # Should be something like xterm-256color
   ```

### Session Output Not Rendering Correctly

**Symptoms:**
- Garbled output in session view
- Missing colors or formatting

**Solutions:**

1. Ensure your terminal size matches expectations:
   - Resize your terminal window
   - Panoptes automatically syncs PTY size

2. Try scrolling to refresh: `PageUp` then `PageDown`

---

## Claude Code Integration

### Hook Not Firing

**Symptoms:**
- Claude Code starts but state never changes from "Starting"

**Solutions:**

1. Verify Claude Code hook configuration:
   - Check Claude Code settings for hook scripts
   - Ensure hooks are pointing to correct URL/port

2. Check if hook script exists:
   ```bash
   ls -la ~/.panoptes/hooks/
   ```

3. Test the hook endpoint manually:
   ```bash
   curl -X POST http://localhost:9999/hook \
     -H "Content-Type: application/json" \
     -d '{"session_id":"test","event":"Stop","timestamp":0}'
   ```

### Session Stuck in Executing

**Symptoms:**
- Session shows "Executing" but Claude has finished

**Explanation:**
The "Stop" or completion hook event wasn't received.

**Solutions:**

1. Wait for automatic timeout (default 5 minutes)
2. Or reduce the timeout:
   ```bash
   echo 'state_timeout_secs = 60' >> ~/.panoptes/config.toml
   ```

---

## Performance Issues

### Slow with Many Sessions

**Symptoms:**
- UI becomes sluggish with 10+ sessions
- Input latency increases

**Solutions:**

1. Close unused sessions (select and press `d`)

2. Reduce output buffer size:
   ```bash
   echo 'max_output_lines = 5000' >> ~/.panoptes/config.toml
   ```

3. Let exited sessions clean up faster:
   ```bash
   echo 'exited_retention_secs = 60' >> ~/.panoptes/config.toml
   ```

### High CPU Usage

**Symptoms:**
- Panoptes using more CPU than expected

**Possible causes:**

1. Many active sessions generating output
2. Rapid state changes (event coalescing should handle this)

**Solutions:**

1. Check for runaway Claude Code sessions
2. Close sessions you're not actively using

---

## Log Files

### Finding Logs

Logs are stored in `~/.panoptes/logs/` with 7-day retention.

```bash
# List log files
ls -la ~/.panoptes/logs/

# View latest log
cat ~/.panoptes/logs/$(ls -t ~/.panoptes/logs/ | head -1)

# Or use the in-app log viewer (press 'l')
```

### Reading Log Levels

- **ERROR** - Something went wrong
- **WARN** - Something unexpected but not critical
- **INFO** - Normal operational messages
- **DEBUG** - Detailed diagnostic information

### Enabling Verbose Logging

Set the `RUST_LOG` environment variable:

```bash
RUST_LOG=debug ./target/release/panoptes
# or
RUST_LOG=panoptes=debug ./target/release/panoptes
```

---

## Reset and Recovery

### Clear All Data

To completely reset Panoptes:

```bash
# Backup first if needed
cp -r ~/.panoptes ~/.panoptes.backup

# Remove all data
rm -rf ~/.panoptes/

# Restart Panoptes - it will recreate the directory
./target/release/panoptes
```

### Clear Only Session Data

To keep projects but remove session history:

```bash
# Remove focus sessions only
rm ~/.panoptes/focus_sessions.json

# Note: Active sessions are only in memory
# Restarting Panoptes clears them
```

### Recover from Corrupted Data

If `projects.json` is corrupted:

```bash
# Panoptes creates a backup automatically
ls ~/.panoptes/projects.json.backup

# Restore from backup if valid
cp ~/.panoptes/projects.json.backup ~/.panoptes/projects.json

# Or start fresh
rm ~/.panoptes/projects.json
```

---

## Getting More Help

If your issue isn't covered here:

1. Check the application logs (`l` key or `~/.panoptes/logs/`)
2. Look for error messages in the header notifications
3. File an issue at: https://github.com/ivan-brko/panoptes/issues
