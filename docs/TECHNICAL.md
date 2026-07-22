# Panoptes Technical Stack

## Language

**Rust** - Chosen for its performance, safety guarantees, and excellent ecosystem for terminal applications. Rust's async capabilities with Tokio make it well-suited for managing multiple concurrent sessions.

## Core Dependencies

### Terminal UI

**Ratatui** (v0.26) - A Rust library for building rich terminal user interfaces. Provides widgets, layouts, and rendering primitives for creating the dashboard interface.

**Crossterm** (v0.27) - Cross-platform terminal manipulation library. Handles raw mode input, ANSI escape sequences, and terminal events. Works on macOS, Linux, and Windows.

### Async Runtime

**Tokio** (v1) - Async runtime for Rust. Powers the concurrent handling of multiple PTY sessions, the HTTP hook server, and event processing. Used with full features enabled.

### HTTP Server

**Axum** (v0.7) - Ergonomic web framework built on Tokio. Runs a local HTTP server (port 9999) that receives state updates from Claude Code's hook system.

### PTY Management

**portable-pty** (v0.8) - Cross-platform pseudo-terminal library. Spawns Claude Code processes in PTYs, enabling full terminal emulation with proper I/O handling and resize support.

### Git Integration

**git2** (v0.18) - Rust bindings to libgit2. Used for detecting git repositories, managing worktrees, and handling branch operations.

### Serialization

**Serde** (v1) with `derive` feature - Serialization framework for Rust. Used for JSON parsing of hook events and configuration files.

**serde_json** (v1) - JSON support for Serde. Parses hook payloads from Claude Code.

**TOML** (v0.8) - TOML format support. Used for configuration and project persistence files.

### Error Handling

**anyhow** (v1) - Flexible error handling for applications. Provides context-rich error propagation.

**thiserror** (v1) - Derive macro for custom error types. Used for domain-specific errors.

### Utilities

**dirs** (v5) - Platform-specific directory paths. Locates home directory for config storage.

**chrono** (v0.4) - Date and time library. Timestamps for session activity tracking.

**uuid** (v4) - UUID generation. Creates unique identifiers for sessions, projects, and branches.

**tracing** (v0.1) with **tracing-subscriber** - Structured logging framework. Debug and diagnostic output.

**shellexpand** (v3) - Shell-like tilde expansion for paths.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│ Panoptes Process                                                │
│                                                                 │
│  ┌──────────────┐    ┌─────────────────┐   ┌─────────────────┐ │
│  │ TUI Layer    │◄───│ Application     │◄──│ HTTP Hook       │ │
│  │ (Ratatui)    │    │ State           │   │ Server (Axum)   │ │
│  │              │    │                 │   │                 │ │
│  │ Views:       │    │ - AppState      │   │ Receives state  │ │
│  │ - Projects   │    │ - View enum     │   │ updates from    │ │
│  │ - Project    │    │ - InputMode     │   │ Claude Code     │ │
│  │ - Branch     │    │                 │   │ hooks           │ │
│  │ - Session    │    │                 │   │                 │ │
│  │ - Timeline   │    │                 │   │                 │ │
│  └──────────────┘    └────────┬────────┘   └────────┬────────┘ │
│         │                     │                     │          │
│         │            ┌────────┴────────┐            │          │
│         │            │ Session         │            │          │
│         │            │ Manager         │◄───────────┘          │
│         │            │                 │                       │
│         │            │ - Attention     │                       │
│         │            │   tracking      │                       │
│         │            │ - State updates │                       │
│         │            └────────┬────────┘                       │
│         │                     │                                │
│         │            ┌────────┴────────┐                       │
│         │            │ Project         │                       │
│         │            │ Store           │                       │
│         │            │                 │                       │
│         │            │ - Projects      │                       │
│         │            │ - Branches      │                       │
│         │            │ - Persistence   │                       │
│         │            └────────┬────────┘                       │
│         ▼                     │                                │
│  ┌──────────────┐    ┌────────┴────────┐                       │
│  │ VTerm        │    │ Agent           │                       │
│  │ (ANSI/color) │    │ Adapter         │                       │
│  │              │    │ - ClaudeAdapter │                       │
│  │              │    │ - CodexAdapter  │                       │
│  │              │    │ - ShellAdapter  │                       │
│  └──────┬───────┘    └────────┬────────┘                       │
│         │                     │                                │
│  ┌──────┴───────┐             │                                │
│  │ PTY Manager  │◄────────────┘                                │
│  │ (portable-   │                                              │
│  │  pty)        │                                              │
│  └──────┬───────┘                                              │
└─────────┼──────────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────────────────────┐
│ Agent Instances (Child Processes)                                │
│                                                                 │
│  Claude Code / Codex / Shell instances run in PTYs and send     │
│  hook events to the HTTP server when state changes occur        │
└─────────────────────────────────────────────────────────────────┘
```

## Logging System

Panoptes includes a comprehensive logging system for debugging and diagnostics:

- **File-based logging** - Logs written to `~/.panoptes/logs/` with daily rotation
- **In-memory buffer** - Real-time log buffer accessible via Log Viewer (`l` key)
- **Automatic retention** - Old log files automatically cleaned up after 7 days
- **Structured logging** - Uses tracing framework with timestamps and log levels

## Theme System

The TUI uses a centralized theme system (`tui/theme.rs`) for consistent styling:

- Semantic colors for UI elements (primary, secondary, success, warning, error)
- State-specific colors for session states (thinking, executing, waiting, awaiting approval, suspended)
- Reusable style definitions for borders, text, and highlights
- Easy customization point for future theming support

## Communication Flow

### User Input
1. Crossterm captures keyboard events
2. Application routes based on mode (Normal/Session) and view
3. In Session mode, keystrokes are written to the PTY

### Session Output
1. PTY reader captures output from Claude Code
2. Output is buffered (ring buffer, max 10K lines by default)
3. TUI renders visible portion with ANSI color support

### State Updates (Hooks)
1. Agent (Claude Code or Codex) executes hook scripts on events
2. Hook script reads the agent's JSON payload from stdin
3. Hook script POSTs an envelope to localhost:9999
4. Axum server forwards it to `SessionManager::handle_hook_event`
5. SessionManager updates state, the in-flight tool set, and attention
6. TUI reflects new state on next render

**Wire format.** The POST body is an envelope: Panoptes' own routing fields at
the top level, the agent's payload nested verbatim underneath.

```json
{
  "session_id": "<panoptes session uuid>",
  "event": "PreToolUse",
  "timestamp": 1784720912,
  "payload": { "tool_name": "Bash", "tool_use_id": "toolu_01", "...": "..." }
}
```

The payload is nested rather than merged because Claude's own payload carries a
`session_id` (its conversation UUID) that would otherwise collide with ours. It
is built by `jq`, never by shell interpolation - a quote or newline in any
forwarded field would produce malformed JSON and silently cost the event. If
`jq` is not on PATH the script degrades to the envelope alone and logs a
warning; state tracking still works, but tool names and notification types are
lost.

**Claude Code hooks:** `SessionStart`, `SessionEnd`, `UserPromptSubmit`,
`PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `Stop`, `Notification`,
`PermissionRequest`.

`SessionStart` does not only mean "a process came up". Its `source` is one of
`startup`, `resume`, `clear`, `compact`, `fork` - and `compact` fires on its own
whenever the context window fills, in the middle of a turn the agent is still
working on. Only `startup`, `resume`, `clear` and `fork` reset the session to
`Waiting`; anything else leaves the state alone.

**Codex hooks:** Limited to `notify` config firing `agent-turn-complete`
events. Maps to Waiting state. No granular tool-use tracking - the notify hook
must not read stdin or it stalls Codex's output pipeline, so it cannot be
extended. A Codex session therefore still infers the start of a turn from the
Enter keystroke.

### Session States

| State | Process | Meaning | Set by |
|-------|---------|---------|--------|
| `Starting` | alive | spawned, agent hasn't reported in | spawn |
| `Thinking` | alive | working, nothing in flight | `UserPromptSubmit`, last `PostToolUse` |
| `Executing` | alive | one or more tools in flight | `PreToolUse`, shell foreground poll |
| `AwaitingApproval` | alive | blocked on a permission dialog | `PermissionRequest` |
| `Waiting` | alive | turn over, awaiting a prompt | `Stop`, shell foreground idle |
| `Suspended` | killed by us | scrollback kept, wakes on interaction | idle sweep |
| `Exited` | died itself | see `exit_reason` | `check_alive` |
| `Resumable` | never spawned | loaded from `sessions.json` | `reconcile` at startup |

Shell sessions render `Executing` as "Running" and `Waiting` as "Ready".

Tool names do not live in the state. Subagents share one `session_id`, so
several tools run at once; they are tracked in `SessionInfo::in_flight`, keyed
by the agent's `tool_use_id`. Keying by invocation ID also means an out-of-order
`PostToolUse` retires its own tool rather than whichever ran most recently -
hook deliveries are backgrounded and can arrive reversed.

Because several states are genuinely true at once, events that announce new
concurrent work only ever *upgrade* the state, in the order
`AwaitingApproval > Executing > Thinking`. Events that report a turn is over are
authoritative and may demote, so a single dropped `PostToolUse` cannot pin a
session in `Executing`.

### Attention Flow

Attention is separate from state: state describes the process, attention
describes the user's queue. A session stays `AwaitingApproval` after you glance
at it and clear the flag, because the dialog is still open.

1. An event raises an `AttentionReason` - `Approval`, `TurnComplete`, `Stalled`, or `Crashed`
2. The badge appears in every session list, coloured by reason
3. If `notify_on` allows that reason, and the session is not the one you are looking at, `notification_method` fires
4. The bell rings only when the reason is new, not on every repeat
5. When the user opens or types into the session, attention is acknowledged

## File Locations

| Path | Purpose |
|------|---------|
| `~/.panoptes/config.toml` | User configuration |
| `~/.panoptes/projects.json` | Project and branch persistence |
| `~/.panoptes/sessions.json` | Session index for recovery after a restart |
| `~/.panoptes/claude_configs.json` | Claude account configurations |
| `~/.panoptes/codex_configs.json` | Codex account configurations |
| `~/.panoptes/hooks/` | Hook scripts for Claude Code and Codex |
| `~/.panoptes/worktrees/` | Git worktrees for branch isolation |
| `~/.panoptes/logs/` | Application logs (7-day retention) |

## Multi-Account Support

Panoptes supports multiple accounts for both Claude Code and Codex CLI:

### Claude Code Accounts

Via the `CLAUDE_CONFIG_DIR` environment variable:

1. **Define configurations** - Each configuration points to a Claude config directory (e.g., `~/.claude-work`, `~/.claude-personal`)
2. **Set project defaults** - Each project can have a default Claude configuration
3. **Session selection** - When creating a Claude session with multiple configs available, a selector appears
4. **Environment injection** - `CLAUDE_CONFIG_DIR` is set when spawning with a non-default configuration

### Codex Accounts

Via the `CODEX_HOME` environment variable:

1. **Define configurations** - Each configuration points to a Codex home directory (e.g., `~/.codex-work`, `~/.codex-personal`)
2. **Set project defaults** - Each project can have a default Codex configuration (independent of Claude config)
3. **Session selection** - When creating a Codex session with multiple configs available, a selector appears
4. **Environment injection** - `CODEX_HOME` is set when spawning with a non-default configuration (defaults to `~/.codex/`)

### Session Display

Sessions display their configuration name in the header (e.g., `[Work]`) when using a non-default configuration.

## Configuration

| Setting | Default | Description |
|---------|---------|-------------|
| `hook_port` | 9999 | Port for the HTTP hook server |
| `max_output_lines` | 10,000 | Lines kept in output buffer per session |
| `idle_threshold_secs` | 300 | Seconds before an unattended waiting session resurfaces |
| `state_timeout_secs` | 300 | Seconds before an in-flight tool is treated as stalled |
| `suspend_after_secs` | 7200 (2h) | Idle seconds before an agent process is suspended; 0 disables |
| `notify_on` | approval, turn_complete, crashed | Which attention reasons ring the bell |
| `attention_on_idle` | false | Whether Claude's idle reminder raises attention |
| `custom_shortcuts` | `[]` | Array of custom shell shortcuts |

### Custom Shortcuts

Custom shortcuts provide quick access to shell sessions with predefined commands:

```toml
[[custom_shortcuts]]
key = "v"
name = "VSCode"
command = "code . &"
```

**Architecture:**
- Stored in `~/.panoptes/config.toml` as a TOML array
- Managed via dialog UI (press `k` in any view)
- Triggered in session view (normal mode) by pressing the shortcut key
- Creates shell session using `SessionManager::create_shell_session_with_command()`

**Key validation:**
- Reserved keys are rejected (q, i, g, G, t, T, k, x, 0-9)
- Duplicate keys are rejected
- Validation occurs in `config::is_reserved_key()` and `Config::add_shortcut()`

**Session creation flow:**
1. User presses shortcut key in session view (normal mode)
2. `session_view.rs` looks up shortcut in config
3. Creates shell session with current project/branch context
4. Writes command to PTY immediately after spawn
5. Switches to session mode in the new session

## Platform Support

Primary target: **macOS** (development platform)

Secondary: **Linux** (should work with no changes)

Windows: Possible with portable-pty, but untested.

## Session Lifecycle

Sessions are cleaned up automatically when Panoptes exits:
- All child processes (Claude Code instances) are terminated
- PTY handles are closed
- No orphaned processes are left behind

### Session Recovery

Agent processes do not outlive Panoptes - the PTY closes and the child is
signalled. The conversation, however, is owned by the agent and already durable:
Claude Code writes `~/.claude/projects/<cwd-slug>/<session-uuid>.jsonl` and Codex
writes `~/.codex/sessions/<date>/rollout-<ts>-<uuid>.jsonl`. What Panoptes stores
in `sessions.json` is the *index* over that data - which conversation belongs to
which session, plus the working directory, project, branch, and account config
needed to relaunch it.

- **Claude Code**: Panoptes dictates the conversation UUID with `--session-id`
  rather than discovering it, so the Panoptes session ID and the Claude session
  ID are the same value. Resume passes `--resume <uuid>`; `--fork-session` is
  never used, since forking would mint a new ID and orphan the stored pointer.
- **Codex**: has no equivalent flag, so its ID is discovered instead. Codex
  writes a rollout file whose first line is a `session_meta` record carrying the
  session `id` and the `cwd` it started in; Panoptes matches on that `cwd` plus
  the session start time. A throttled scan runs only while some Codex session
  still lacks an ID, so it costs nothing in the steady state. The notify hook
  cannot be used for this - it must not read stdin, or it stalls Codex's output
  pipeline and drops keystrokes.
- **Shell**: has no conversation. Restoring one respawns a fresh shell in the
  same directory; its scrollback is not recovered.

Records are written on membership change (create, close), not on state change -
live state describes a process that no longer exists and is discarded at load.
Quitting Panoptes keeps records so sessions can be resumed; explicitly closing a
session discards its record.

At startup every record is reconciled to `SessionState::Resumable` and listed
inertly - nothing is spawned until the user opens it. A record whose working
directory has been deleted, or which never recorded a conversation ID, is still
listed but shows why it cannot be brought back.

### Suspending Idle Sessions

An idle Claude Code process measures around 565 MB - roughly 25x the whole
Panoptes process, which sits at about 23 MB. A handful of forgotten sessions
therefore dominate memory use while doing nothing at all.

After `suspend_after_secs` of no engagement, the child process is killed and the
session moves to `Suspended`. The `Session` and its `vt100::Parser` buffer are
kept, so the scrollback stays readable and scrollable - the buffer lives in
Panoptes' memory, not the child's. Reading a suspended session is free; only
interacting with one pays.

The clock is `last_engagement`, which moves when the agent changes state or the
user types, and deliberately *not* on raw PTY output. A redrawn status line is
rendering, not engagement. Claude's once-a-minute idle notification is excluded
for the same reason - counting it would reset the clock forever.

A session is only suspended when all of the following hold. Each clause
describes work a kill would destroy:

- it is not a shell (no conversation to reattach to, and killing one ends a
  build, a dev server, or an ssh session)
- it is in `Waiting` - never `AwaitingApproval`, `Executing` or `Thinking`
- it is not the session the user is currently viewing
- it has no `resume_blocker()`; suspending something with no way back is just
  closing it

Waking spawns a fresh agent through the same `--resume` path a recovered session
uses, and **discards the terminal buffer at that moment**. Reusing it would have
the new process draw into a vterm still holding the old cursor position,
alt-screen flags, modes and thousands of rows of output. Measured wake latency
is around 2 seconds and does not scale with conversation length: the agent
renders a compact resumed view rather than replaying the transcript. The
conversation itself is intact - only the on-screen history is not.

Three paths must exclude suspended sessions, and all three fail silently if
missed: `poll_outputs` (reading a dead PTY reports as an error and becomes
`Exited`), `check_alive` (reaping our own kill would notify the user of a
crash), and `cleanup_exited_sessions` (which calls `forget_session` and deletes
the record from `sessions.json`, making the session permanently unrecoverable).
`SessionState::has_process()` is the single predicate they all use.

## Testing

The project has 435+ unit tests covering:
- Configuration loading/saving
- Session state transitions
- Output buffer management
- Hook event parsing and envelope compatibility
- Hook script behaviour (executed against a sealed PATH)
- Session state precedence and legacy-record migration
- PTY operations
- VTerm ANSI parsing
- Project/branch management
- Navigation state machine
- Logging system
- Path completion

Run tests with: `cargo test`
