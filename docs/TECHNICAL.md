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
│  │ - Logs       │    │                 │   │                 │ │
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
- **Automatic retention** - Old log files automatically cleaned up after 7 days
- **Structured logging** - Uses tracing framework with timestamps and log levels
- **No in-memory buffer** - Nothing is held in memory for the TUI to display.
  Settings → About / paths names the current log file; reading it is `tail`'s
  job, not the dashboard's.

## Theme System

The TUI uses a centralized theme system (`tui/theme.rs`) for consistent styling:

- Semantic tokens for everything a view draws - raw `Color::` literals in view
  code are a bug, because they are invisible to a theme change
- Tiered tokens where hierarchy matters: `text` / `text_dim` / `text_faint`,
  `border_focus` / `border` / `border_dim`, `bg_base` / `bg_surface`
- State-specific colors for session states (thinking, executing, waiting,
  awaiting approval, suspended), plus `attention_color` for badge reasons
- Three capability tiers - `truecolor()`, `ansi256()`, `ansi16()` - detected
  from `COLORTERM`/`TERM` at startup and forceable with the `theme` config
  key. Within a palette the tiers agree on every chromatic token (the user's
  terminal palette keeps deciding what "green" means) and differ only in the
  structural greys and surfaces, so the 16-colour baseline is exactly the
  classic appearance
- Four palettes - `Peacock` (default), `Io`, `Hera`, `Argus` - chosen with the
  `palette` config key or live from **Settings → Theme**. A theme is one
  palette at one tier: `Theme::new(palette, support)`. A palette owns only the
  `Chrome` tokens (accent, focused border, the two surfaces, input prompt,
  default marker); states, outcomes, banners and the text ramp are identical
  in all four, so a session list reads the same whichever is on. Peacock is
  byte-for-byte the pre-palette theme, pinned by
  `test_peacock_is_byte_for_byte_the_old_palette`
- The global is a swappable `RwLock`, not a `OnceLock`: the picker previews
  live, so a palette change has to reach the next render. `theme()` returns a
  `Copy` snapshot rather than a borrow, so no render holds the lock

Focus is signalled by four channels at once, so it survives a colourblind
user, a low-contrast theme, and a screenshot: border brightness
(`border_focus` vs `border_dim`), border weight (thick vs rounded), an
explicit title style, and `Modifier::DIM` over the unfocused pane's text
ramp. Dimming carves out the signals - session state colours and attention
badges hold full strength in every pane, focused or not, pinned by
`test_signals_survive_an_unfocused_pane`.

## Communication Flow

### User Input
1. Crossterm captures keyboard events
2. In normal mode the global keys are handled first (`Tab`, `←`/`→`, `q`, `?`,
   `Space`); every other input mode owns those keys itself, which is why `Tab`
   completes a path in the add-project prompt and types a tab in Session mode,
   and why `←`/`→` still toggle Yes/No in a dialog. `←`/`→` are exact synonyms
   for `Shift+Tab`/`Tab`: because globals run *before* the mode handler, no pane
   may claim them, which is why nothing in normal mode does
3. Otherwise the key routes on `Focus` — one of the three panes, or a
   full-screen session — and then on that pane's own drill-down level
   (`ProjectsNav` / `SettingsNav`)
4. In Session mode, keystrokes are written to the PTY

### Mouse Input and Text Selection
Mouse capture stays on for the whole time a session is on screen — the wheel
cannot drive local scrollback otherwise — and mouse reporting is a single
terminal-wide switch, so the terminal's own drag-selection is off while it is.
`App::handle_mouse_event` therefore routes every mouse event itself, in this
order:

1. Wheel notches over a Codex session become local scrollback
2. **Shift claims the event for Panoptes.** Held shift skips step 3 entirely,
   which is what shift means in every terminal: this one is the terminal's
   business, not the application's. It is the only way to select out of an
   agent that took the mouse, and it costs the child nothing — the terminal
   sees the report first, so a child could never have had shift-drag anyway
3. If the child enabled a mouse protocol (Claude Code's TUI, `vim` with
   `mouse=a`, `htop`), the event is encoded and forwarded to the PTY, and the
   child does its own selection — the same as in a real terminal tab
4. Alternate-screen apps that did *not* enable a mouse protocol get wheel
   notches translated to arrow keys, as iTerm2 does
5. Wheel notches over anything else become local scrollback
6. What is left — left-button press, drag and release — becomes a Panoptes
   selection, a rectangle if control was held at button-down and the usual
   stream if not

Selection (`app/selection.rs`, painted by `tui/views/session.rs`) is modelled
on tmux:

- **Press** anchors the selection and *freezes that session's PTY reads*
  (`tick_output_polling` already excludes one session; the drag adds itself to
  that condition). The screen is a snapshot for as long as the button is down,
  so nothing can shift under the pointer. Output queues in the PTY and resumes
  on release. The freeze is read from the live selection state, never a flag
  of its own, so a drag that ends abnormally cannot leak it.
- **Release** extracts the text via `VirtualTerminal::contents_between` and
  copies it (`clipboard.rs`: a helper such as `pbcopy` first, OSC 52 as the
  fallback — iTerm2 ships with OSC 52 clipboard access disabled). A press and
  release on one cell is a plain click and never touches the clipboard.
- **Double- and triple-click** select a word and a logical line. crossterm
  reports no click count, so `ClickTracker` derives one from presses landing
  within 400 ms and one cell of each other. Both follow soft-wraps.
- **The highlight is ephemeral**: it survives release (the copy already
  happened) but is dropped by the session's next output, the next click, a
  wheel notch, a resize, a session switch, or leaving the view.

Coordinates are *absolute rows* — row 0 is the oldest line of scrollback and
`history_rows()` is the top of the live screen — rather than rows of the
current view. That is what lets a drag held past the top or bottom edge scroll
the view (driven from the event-loop tick, because drag events stop arriving
when the pointer stops moving) and keep extending across more than a
screenful. Extraction over such a range needs `contents_between_absolute`, one
of the `PANOPTES PATCH` additions in the [`panoptes-vt100`](https://github.com/ivan-brko/panoptes-vt100)
fork this crate depends on in place of upstream `vt100`.

Selection works on suspended sessions too: it only ever reads the terminal
emulator, and never writes to or wakes a PTY.

### Pane Layout
The three panes are sized by `tui/panes.rs`. `pane_widths(total, focused)` is a
pure function of the terminal width and the focused pane; `PaneLayout` eases
between two width sets over ~140ms, driven from the existing 16ms tick.

Two invariants hold at every frame, including mid-transition:
- only the two boundaries *between* panes are interpolated, and the three widths
  are derived from them, so they always sum to exactly the terminal width;
- a pane's render density (`SideMode`: full / compact / strip / hidden) comes
  from its *current* width, so a pane can cross strip → compact part-way through
  a transition.

A focus change retargets from wherever the panes currently are rather than
queueing, so holding `Tab` or `→` never overshoots or builds up a backlog. Once a
transition lands, `PaneLayout::tick` stops asking for frames — idle Panoptes
renders exactly as often as it did before the accordion existed.

PTY dimensions are still computed from the *full* terminal via `FrameLayout`:
the session view is full-screen, so the pane split must never reach the PTY.

### Session Output
1. Each PTY has a reader thread (`pty-reader-<pid>`, `session/pty_reader.rs`)
   that drains it continuously into a queue
2. Each pass of the event loop takes queued output, up to a byte budget per
   session, and feeds it through the virtual terminal
3. Scrollback is kept by the emulator (10K lines by default)
4. TUI renders visible portion with ANSI color support

**Why a thread.** A PTY's kernel buffer holds about 1 KB on macOS, and a child
that fills it waits for a read. When the UI thread read the PTY itself, once per
loop pass with up to 16 ms of sleep between passes, a chatty child was held to
roughly 64 KB/s. Claude Code's fullscreen renderer writes 300-700 KB per
trackpad flick and does not block - it queues frames and delivers them late -
so scrolling kept coasting for seconds after the wheel stopped.

**The reader.** It owns a `dup` of the master fd, and closes it on exit. The fd
is `O_NONBLOCK` (the open file description is shared with the writer, whose
retry and timeout logic depends on it), so the thread waits in `poll` with a
50 ms timeout and then reads in 64 KB chunks until `WouldBlock`. Every read is
queued as its own chunk: read boundaries matter, because query replies use the
cursor as of the read that carried the query, and a drag hold replays reads one
at a time.

**Backpressure.** The queue is capped at 1 MB per session, counted in bytes.
When it is full the thread stops reading, the kernel buffer fills and the child
blocks, exactly as before, but with room for a whole scroll burst. A Codex
session the user has scrolled up in is not polled at all, and this is the
backpressure it relies on.

**End of stream.** A read error (`EIO` after the child dies on Linux) is queued
after the reads before it; `Session::poll_output` turns it into `Exited` with a
`PTY read error` reason once they are taken. End of file (how a dead child's
master reads on macOS) ends the thread quietly, and reaping is left to
`check_alive`. Dropping the `PtyHandle` stops the thread: it notices at its next
poll timeout, or at once if it was waiting for room in the queue. It is never
joined, so dropping a session does not stall the UI.

**Byte budget.** `SessionManager::poll_outputs_except` takes at most about the
queue's cap (1 MB) per pass from the *watched* session - the active one, while
it fills the screen - so a scroll burst lands in one pass, while a runaway
child (`yes`), whose thread refills the queue as fast as it empties, cannot
starve the loop. When a pass stops on that budget with the watched session's
output still queued, the next `event::poll` does not sleep; otherwise the loop
waits its usual 16 ms tick, so an idle Panoptes still sleeps.

Every other session gets one read's worth (64 KB) per pass, and its backlog
never shortens the sleep. Nobody is looking at a background session, so a flood
there is paced by the tick - its queue fills and the child blocks - and costs
about what it did when the UI thread read the PTY itself, instead of a core
spent parsing output no one sees. Since no read is bigger than the budget, a
background session is slowed, never stalled.

### Background Git Work
Git operations that can take seconds (`git fetch --all`, creating or removing a
worktree) never run on the event-loop thread:

1. The flow that needs one calls `App::spawn_git_job` with a `GitTask` (the git
   work, self-contained enough to move to a worker thread) and a `JobFollowUp`
   (what the app does with the result)
2. A worker thread runs the task and sends the result back over a channel
3. Meanwhile the event loop keeps rendering, hooks keep arriving, and the
   "Working" overlay animates a spinner. Keys are swallowed while a job runs,
   so nothing acts on state the job is about to replace
4. `App::tick_background_job` polls the channel each pass and applies the
   follow-up: opening the worktree wizard, opening the default-base selector,
   registering a created worktree, or finishing a worktree delete

Fetches are cancellable: `Esc` kills the `git fetch` child process and the flow
continues with the refs already on disk (the same fallback as a failed fetch).
Worktree create/remove are not - interrupting one halfway would leave the repo
in a worse state than it started.

### State Updates (Hooks)
1. Agent (Claude Code or Codex) executes hook scripts on events
2. Hook script reads the agent's JSON payload from stdin
3. Hook script POSTs an envelope to localhost:9999
4. Axum server forwards it to `SessionManager::handle_hook_event`, which
   translates the hook into an `AgentEvent`
5. The pure state machine (`session/state_machine.rs`) applies it to the
   session: state, the in-flight tool set, and attention. It operates on a bare
   `SessionInfo`, which is what makes every transition unit-testable
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
events. Codex spawns the `notify` argv directly, with no shell, and appends
the event JSON as its final argument; stdin is `/dev/null` and output is
discarded. Panoptes' hook ignores the event, and must never block on stdin.
Codex state comes from its transcript instead - see Reading Agent
Transcripts below.

If `config.toml` already has a `notify` hook, Panoptes chains in front of it
rather than replacing it (backing the file up first):

```toml
notify = ["bash", "-c", "'<panoptes hook>' \"$@\"; '<user argv>'... \"$@\"", "panoptes-notify"]
```

`panoptes-notify` fills `$0`, so the event lands in `"$@"` and both hooks
receive it unchanged. The hooks are joined with `;`, so the user's runs even
when Panoptes' fails. Chains written by older versions (`bash -lc`, no `$0`)
dropped the event; they are rewritten to this shape when the user's argv can
be recovered exactly, and reported for a manual merge when it cannot.

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

`Stalled` is the one reason with a liveness test in front of it
(`SessionManager::check_state_timeouts`). Passing `state_timeout_secs` decides
only that an in-flight tool report is too old to believe, so the tool is evicted
and the state repaired regardless. The flag is raised only if the session has
also written nothing to its PTY for `STALL_SILENCE_SECS` (30s) and is not the
session on screen: both agents redraw a spinner while a tool runs, so continuing
output means a long tool, not a hung one. Without that test every long `Bash`
call in a row re-raised the badge moments after the user cleared the last one.

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

All state files are written through a shared persistence layer
(`src/persistence.rs`): saves are atomic (written to a sibling temp file, then
renamed), and a corrupted file is backed up to a timestamped
`<name>.corrupt.<timestamp>` sibling before starting fresh with defaults and a
visible warning.

## Multi-Account Support

Panoptes supports multiple accounts for both Claude Code and Codex CLI. Both
account stores are aliases of the same generic `ProfileStore`
(`src/agent_profiles.rs`), and the add/select/delete dialogs share one set of
input handlers and views (`input/agent_configs.rs`, `tui/views/agent_configs.rs`),
parameterized by agent kind:

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

Every key has a default and the file is optional; unknown keys are ignored.
See [CONFIG_GUIDE.md](CONFIG_GUIDE.md) for the full reference.

| Setting | Default | Description |
|---------|---------|-------------|
| `hook_port` | 9999 | Port for the HTTP hook server |
| `worktrees_dir` | `~/.panoptes/worktrees` | Where branch worktrees are created |
| `hooks_dir` | `~/.panoptes/hooks` | Where generated hook scripts are written |
| `scrollback_lines` | 10,000 | Lines of history retained per session |
| `state_timeout_secs` | 300 | Seconds before an in-flight tool report stops being believed |
| `suspend_after_secs` | 7200 (2h) | Seconds a session may sit inactive before its agent process is suspended; 0 disables |
| `log_agent_events` | false | Log raw agent transcript lines for debugging |
| `notify_on` | approval, turn_complete, crashed | Which attention reasons ring the bell |
| `attention_on_idle` | false | Whether Claude's idle reminder raises attention |
| `theme` | `auto` | Colour-capability tier: `auto` / `truecolor` / `ansi256` / `ansi16` |
| `palette` | `peacock` | Colour preset: `peacock` / `io` / `hera` / `argus` |
| `custom_shortcuts` | `[]` | Array of custom shell shortcuts |

Several config keys from earlier versions — an output-line cap, an Escape-hold
threshold, and `theme_preset` — have been removed from the `Config` struct. Like
any unknown key, each is simply ignored if left in an older config file, so those
files keep loading. `notification_method` is validated on load: `bell`, `title`,
or `none`, with unknown values logging a warning and falling back to `bell`.

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
- Managed in Settings → Shortcuts (pane 3)
- Triggered in session view (normal mode) by pressing the shortcut key
- Creates shell session using `SessionManager::create_shell_session_with_command()`

**Key validation:**
- Reserved keys are rejected (q, n, s, d, 0-9)
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
- **Shell**: has no conversation, and is therefore **not persisted at all**. Its
  state is the scrollback, the environment, and the processes it is running,
  none of which survive the PTY; respawning `$SHELL` in the recorded directory
  yields a blank prompt, which is what creating a new shell session on that
  branch already gives you. A record would promise a restoration it cannot
  perform, so none is written. Records left by earlier versions are dropped, and
  the file rewritten without them, on the next load.

Records are written on membership change (create, close), not on state change -
live state describes a process that no longer exists and is discarded at load.
Quitting Panoptes keeps records so sessions can be resumed; explicitly closing a
session discards its record. Because shells are the one thing quitting destroys
rather than sets aside, the quit prompt counts the live ones and says so.

At startup every record is reconciled to `SessionState::Resumable` and listed
inertly - nothing is spawned until the user opens it. A record whose working
directory has been deleted, or which never recorded a conversation ID, is still
listed but shows why it cannot be brought back.

### Reading Agent Transcripts

Both agents write a complete record of every conversation to disk as it
happens, and Panoptes knows where because it stores each session's conversation
ID. Reading those files needs no cooperation from the agent, and for Codex it is
the only channel there is.

| | Claude Code | Codex CLI |
|---|---|---|
| File | `$CLAUDE_CONFIG_DIR/projects/<cwd-slug>/<uuid>.jsonl` | `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl` |
| Path is | derived from cwd and ID | searched for, since the name embeds a timestamp |
| Drives state | no - hooks own it | **yes** |
| Contributes | context usage, model | state, context usage, model, rate limits |
| Measured flush latency | immediate | under 50ms |

The two tailers have deliberately different jobs. Codex's rollout drives its
state, which is what brings it to parity with Claude - until this, a Codex
session could only ever report "my turn ended". Claude's transcript only
supplements: hooks already report state and arrive sooner, and two producers
writing the same field would fight over it.

Everything converges on `AgentEvent`, a vocabulary neither agent speaks.
`SessionManager::apply_agent_event` is the single ingest path; hooks and both
tailers translate into it, so what an event *means* is decided in exactly one
place.

**Codex record mapping.** `event_msg` records narrate the session;
`response_item` records are what the model emitted. Tool starts exist only in
the second - `event_msg` contains no `*_begin` events at all - so
`function_call` / `function_call_output` is the begin/end pair, matched on
`call_id`. Verified across real rollouts: those two appear 1183/1183 times, and
`task_started` (42) equals `task_complete` (41) plus `turn_aborted` (1).
`exec_command_end` and `mcp_tool_call_end` describe completions already seen as
`function_call_output` and are deliberately ignored, or every tool would be
retired twice.

**Where reading starts.** A session that created its own transcript is read
from the beginning: everything in the file describes what it has just been
doing, including the opening seconds during which a Codex conversation is still
being located. A session reattaching to a conversation that predates it seeks to
EOF instead, so an old conversation does not replay as if it were happening now,
and scans backwards once for the most recent usage figures so the display is not
blank until the next turn. A trailing partial line is held back until its newline arrives,
as are trailing bytes that stop mid-character; transcripts are routinely read
mid-write, and decoding a chunk in isolation would corrupt a split character
permanently. A file that shrinks is re-attached at its new end rather than read
from a stale offset.

**Threading.** The watcher runs on its own OS thread and is drained with
`try_recv` each tick, the same shape as hook events. The reads are incremental,
but a burst of tool output can append a lot at once and parsing that on the
render thread would show as a stutter.

**Subagents.** Codex subagents write their own separate rollout files, so a
parent looks completely idle while its children work - the mirror image of
Claude, whose subagents share the parent's session ID and show up in
`in_flight`. A child names its parent in `forked_from_id`, so discovery is
exact. Liveness is not: there is no reliable "this subagent exited" signal, so
it is inferred from recent writes. That is why the display is a count and not a
claim, and why a session with subagents is never suspended.

The same `forked_from_id` marker keeps `discover_session_id` from claiming a
subagent rollout as a session's own conversation - they sit in the same working
directory with their own fresh timestamps and otherwise match every criterion.
Note that on a subagent rollout `payload.id` is the subagent's own ID while
`payload.session_id` is its parent's.

Setting `log_agent_events = true` writes every raw transcript line to
`~/.panoptes/logs/agent-events/<session>.ndjson`, so the reader's interpretation
can be checked against its input. Best effort throughout: it must never disturb
the session it describes.

### Suspending Inactive Sessions

An idle Claude Code process measures around 565 MB - roughly 25x the whole
Panoptes process, which sits at about 23 MB. A handful of forgotten sessions
therefore dominate memory use while doing nothing at all.

After `suspend_after_secs` of no engagement, the child process is killed and the
session moves to `Suspended`. The `Session` and its `vt100::Parser` buffer are
kept, so the scrollback stays readable and scrollable - the buffer lives in
Panoptes' memory, not the child's. Reading a suspended session is free; only
interacting with one pays.

Two clocks must agree. `last_engagement` moves when the agent changes state or
the user types, and deliberately *not* on raw PTY output - a redrawn status line
is rendering, not engagement, and Claude's once-a-minute idle notification is
excluded for the same reason. `last_activity` does move on PTY output, and is
the safety net: Codex reports nothing between the start of a turn and its end,
so a working Codex session can sit in `Waiting` with a stale `last_engagement`
while producing output the whole time. Requiring silence too means the worst
case is a session that fails to suspend rather than one whose work is
destroyed.

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
missed: `poll_outputs` (a dead PTY's reader reports a read error, at least on
Linux, which `poll_output` turns into `Exited`), `check_alive` (reaping our own kill would notify the user of a
crash), and `cleanup_exited_sessions` (which calls `forget_session` and deletes
the record from `sessions.json`, making the session permanently unrecoverable).
`SessionState::has_process()` is the single predicate they all use.

## Testing

The project has 650+ unit tests covering:
- Configuration loading/saving
- Session state transitions
- Output buffer management
- Hook event parsing and envelope compatibility
- Hook script behaviour (executed against a sealed PATH)
- Session state precedence and legacy-record migration
- Transcript parsing, tailing, and mid-write robustness
- Session suspension exclusions
- PTY operations
- VTerm ANSI parsing
- Project/branch management
- Navigation state machine
- Logging system
- Path completion

Run tests with: `cargo test`

### End-to-end mouse selection

`tests/selection_e2e.rs` drives the **real binary**: it spawns Panoptes in a
PTY, plays iTerm2 at the other end (answering the startup queries, sending SGR
mouse reports exactly as a terminal does once mouse capture is on), and asserts
on the *system clipboard*. A passing run exercises the whole chain, from an
escape sequence arriving on stdin to text landing in `pbpaste`.

These are `#[ignore]`d — they spawn processes and touch the developer's
clipboard (which they save and restore), so they do not belong in a plain
`cargo test`:

```bash
cargo test --test selection_e2e -- --ignored          # every scenario
cargo test --test selection_e2e -- --ignored shell    # just one
```

Being a Cargo integration test rather than a loose script is what guarantees
the binary under test is fresh: `CARGO_BIN_EXE_panoptes` is built before the
test runs, so it is impossible to validate a stale build. The scenarios live in
`tests/e2e/drive_selection.py` and need `python3` with
[`pyte`](https://pypi.org/project/pyte/), which renders Panoptes' output so the
harness knows where on screen to click. The `codex` scenario additionally needs
an authenticated `~/.codex`.
