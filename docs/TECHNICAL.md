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
worktree) never run on the event-loop thread - nor does the one non-git job
that shares the machinery, the conversation import scan (see *Importing
conversations* under Session Recovery):

1. The flow that needs one calls `App::spawn_git_job` with a `GitTask` (the git
   work, self-contained enough to move to a worker thread) and a `JobFollowUp`
   (what the app does with the result)
2. A worker thread runs the task and sends the result back over a channel
3. Meanwhile the event loop keeps rendering, hooks keep arriving, and the
   "Working" overlay animates a spinner. Keys are swallowed while a job runs,
   so nothing acts on state the job is about to replace
4. `App::tick_background_job` polls the channel each pass and applies the
   follow-up: opening the worktree wizard, opening the default-base selector,
   registering a created worktree, finishing a worktree delete, or opening the
   conversation import picker

Fetches are cancellable: `Esc` kills the `git fetch` child process and the flow
continues with the refs already on disk (the same fallback as a failed fetch).
So is the conversation scan, which checks the flag between files and opens
nothing once cancelled.
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
`PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `Stop`, `StopFailure`,
`Notification`, `PermissionRequest`, `PermissionDenied`, `SubagentStart`,
`SubagentStop`, `Elicitation`, `ElicitationResult`. Every one is a symlink to
the same script, re-created on each spawn, so an existing install picks up a
newly registered event the next time a session starts. The rest of Claude's
hooks are deliberately unregistered; `HOOK_EVENTS` in `agent/claude.rs` says
why, one line each.

| Hook | Payload fields used | `AgentEvent` | Effect |
|---|---|---|---|
| `StopFailure` | `error`, `last_assistant_message`, `error_details` | `TurnFailed` | fires *instead of* `Stop`: tools cleared, `Waiting`, `TurnFailed` attention, reason on the row |
| `PermissionDenied` | `tool_name` (the `reason` is available but unused) | `ApprovalResolved` | `AwaitingApproval` demotes to `Thinking` and a matching approval flag clears; otherwise nothing |
| `SubagentStart` / `SubagentStop` | `agent_id` | `SubagentStarted` / `SubagentFinished` | a set of running subagent IDs; its size is `SessionInfo::subagents` |
| `Stop` / `SubagentStop` | `background_tasks`, `session_crons` | `BackgroundWork` | snapshot counts of background tasks and scheduled prompts |
| `Elicitation` | `mcp_server_name`, `message` | `ApprovalRequested` | as an `elicitation_dialog` notification: `AwaitingApproval`, approval attention |
| `ElicitationResult` | `mcp_server_name` | `ApprovalResolved` | clears it; the flag does not record the server, so any result does |

The field names are Claude Code 2.1.280's own, from the hook-input schemas in
its binary. Three findings shaped the design:

- `PermissionDenied` only fires when auto mode's classifier refuses a call -
  no dialog is shown, so there is usually no approval to resolve.
- `TaskCreated` / `TaskCompleted` are the agent's to-do list (`TaskCreate`,
  `TaskUpdate`), not background work, and a deleted item never fires
  `TaskCompleted`. They are not registered.
- Background work is instead listed on every `Stop` and `SubagentStop`, as
  `background_tasks` (`{id, type, status, description, ...}`, `type` one of
  `shell`, `subagent`, `monitor`, `workflow`, ...; running or pending, and
  backgrounded) and `session_crons` (`{id, schedule, prompt}` for `/loop`,
  `CronCreate`, `ScheduleWakeup`). Each list replaces the last count; a
  payload without one - an older Claude, or the no-`jq` path - leaves the
  count alone. The snapshot travels as a second event,
  `state_machine::background_snapshot`, applied after the hook's own.

Subagents are paired by `agent_id`, so a stop overtaking its start, or a stop
for a subagent never seen starting, cannot drive the count below zero. The one
leak - a subagent interrupted before its `SubagentStop` - is closed at the next
`Stop`: a turn has ended, so every subagent still running is backgrounded and
listed, and a list with no `subagent` entry retires every tracked ID. Background
work outlives turns, so `UserPromptSubmit` keeps all of it; a fresh
`SessionStart` and `SessionEnd` clear it. Codex's subagent count comes from its
transcript watcher instead, which never watches a Claude session, so the two
never feed one session.

`SessionStart` does not only mean "a process came up". Its `source` is one of
`startup`, `resume`, `clear`, `compact`, `fork` - and `compact` fires on its own
whenever the context window fills, in the middle of a turn the agent is still
working on. Only `startup`, `resume`, `clear` and `fork` reset the session to
`Waiting`; anything else leaves the state alone.

**Claude's status line.** Claude Code reports its plan rate limits, and the
running session's real context window, in one place only: the JSON it pipes to
a `statusLine` command on every status-line refresh. Panoptes installs its own
`statusLine` in the same `settings.local.json` as the hooks, pointing at
`~/.panoptes/hooks/panoptes-statusline.sh`. It POSTs the document as a
`StatusLine` envelope (backgrounded, `curl --max-time 2`, detached from stdout
so Claude never waits on it), and translates to `AgentEvent::UsageRefresh`.

A `statusLine` in `settings.local.json` replaces the user's own from any lower
layer, so Panoptes wraps rather than replaces it:

- At spawn it resolves the effective user status line the way Claude would:
  the project's `settings.local.json`, then `.claude/settings.json`, then
  `$CLAUDE_CONFIG_DIR/settings.json` (the profile's directory, else the
  environment's, else `~/.claude`). Only `type: "command"` settings count.
- The installed command is `'<wrapper>' ['--local'] '<user command>'`. The
  wrapper reads stdin once, forwards it, then pipes the same bytes to
  `bash -c '<user command>'` and exits with its status, so the user's output
  reaches Claude untouched. The user's other options (`padding`,
  `refreshInterval`) are copied onto Panoptes' setting.
- `--local` marks a command that came from `settings.local.json` itself, the
  one layer Panoptes overwrites; that is what lets `claude_status_line = false`
  put it back. A command from a lower layer is re-read from that layer at every
  spawn, so editing it takes effect. The wrapper recognises its own command by
  the script's file name and sees through it, so it never wraps itself.
- With no user status line it prints Panoptes' compact line instead, by piping
  the same input to `<panoptes> status-line`: the absolute path of the
  executable that wrote the settings (`std::env::current_exe()`), baked into
  the wrapper. Claude reserves the row for any status line, so the
  alternative was a blank one. The subcommand answers before Panoptes touches
  config or logs (about 25 ms a run, process start included), prints nothing
  and exits 0 on input it cannot read, and a missing binary only costs the
  line. It is drawn by `hooks::status_line::compact_line` from the same
  `usage_from_payload` the header uses, so the binding window is picked by the
  same rule - e.g. `5h 12% · wk 40% (resets Thu 18:00) · $2.14`. The reset is
  local wall-clock time (`18:00` today, `Thu 18:00` on another day), not a
  countdown: Claude redraws its status line on events, not on a clock, so a
  countdown would go stale. A reset already past, or any missing field, is
  left out.

The envelope is built by splicing Claude's document in whole, not with `jq`:
the document is already JSON and nothing is picked out of it, and the status
line runs often enough that a process saved matters. The session ID spliced
beside it is checked to look like a UUID first. Without `PANOPTES_SESSION_ID`
(a plain `claude` run in the same directory) nothing is posted and only the
user's command runs.

Like the hooks, the status line is never removed when a session ends - Panoptes
does not clean `settings.local.json` up at all. The file is shared by every
session in the working directory, running ones included, and Claude re-reads it
live, so removing the key on one session's exit would strip it from its
neighbours. Every spawn in a directory writes the same setting, so concurrent
sessions agree on it; each wrapper run reads its own session from the
environment.

Captured from Claude Code 2.1.280 (`tests/fixtures/claude_status_line.json`):
`rate_limits.five_hour` / `rate_limits.seven_day`, each `{used_percentage,
resets_at}` with a 0-100 percentage and `resets_at` in epoch seconds;
`context_window.context_window_size` and `current_usage`; `model.id` with any
`[1m]` suffix. `rate_limits` is absent until the first API response of the
process, and `current_usage` is `null` until the first turn. The command runs at
startup and after state changes (token usage, permission mode, model, effort,
vim mode), debounced - one turn produced a single refresh - and on timers:
when a rate-limit window resets, when the prompt cache expires, and every
`refreshInterval` seconds if the user set one. Many of those fire while nobody
is doing anything, which is why `UsageRefresh` is not activity.

**Codex lifecycle hooks (Codex 0.156.1 and later):** `SessionStart`,
`SessionEnd`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`,
`PermissionRequest`, `Stop`, `SubagentStart`, `SubagentStop`, `Interrupt`.
Codex's hooks are Claude-compatible - the same `{"hooks": {"<Event>": [{"hooks":
[{"type": "command", ...}]}]}}` declarations, the same JSON payload on stdin -
so they run the same script, through per-event symlinks in
`~/.panoptes/hooks/codex/`.

Nothing is written into `CODEX_HOME`. The hooks are declared for each spawn
as `-c hooks.<Event>=[...]` root options, ahead of any `resume` subcommand
(`agent/codex.rs::lifecycle_hook_args`). Codex runs a hook only once it is
*trusted*: an untrusted one stops startup at a "Hooks need review" screen,
and hooks declared through `-c` are no exception. So one more override,
`-c hooks.state={...}`, sets a `trusted_hash` for each Panoptes hook, keyed
`/<session-flags>/config.toml:<event>:0:0`. This trusts nothing but Panoptes'
own declarations:

- Codex reads `hooks.state` only from the user's `config.toml` and from `-c`
  overrides - never from a project or plugin - and each key names one hook
  declared by the same overrides.
- The hash is Codex's own `hook_hash`, reproduced in `hook_trust_hash`:
  SHA-256 over the canonical JSON of the normalized declaration (event, the
  command *string*, timeout, async). The script body is not hashed, so
  reinstalling it never needs re-trusting.
- The user's and the project's hooks keep whatever trust they had, and a new
  one still prompts.
- Nothing in the key or the hash involves `CODEX_HOME`: the key's path is
  Codex's fixed placeholder for the `-c` layer, and the command names a
  script in Panoptes' own hooks directory. The same overrides are trusted
  whichever `CODEX_HOME` a session runs against. `--dangerously-bypass-hook-trust`, which would switch the
  check off for all of them, is not used.

A Codex that hashed differently would show every Panoptes hook as modified
and prompt on every spawn, so lifecycle hooks are gated on `codex --version`
(probed at most every ten minutes) being at least 0.156.1, the version the
hash was verified against; a unit test pins the hash Codex accepted.

Codex waits for each hook, including `PermissionRequest` before it shows the
approval dialog, and treats exit 0 with empty stdout as "no opinion". Plain
stdout from `SessionStart` or `UserPromptSubmit` would be fed to the model as
context. The script prints nothing, backgrounds its `curl`, and exits 0 at once,
so it never influences a decision. Codex runs hooks through `$SHELL -lc` in the
session's working directory, with the session's environment, so
`PANOPTES_SESSION_ID` reaches them. Hooks from every layer run, so a user's own
Codex hooks coexist with Panoptes'.

What the Codex hooks mean differs from Claude's in three places
(`state_machine::translate_codex_hook`, `SessionManager::handle_hook_event`):

- **Subagents.** Codex reports a subagent's own `UserPromptSubmit`,
  `PreToolUse` and `PostToolUse` under the *parent's* `session_id`, marked only
  by `agent_id`. A subagent routinely outlives the turn that spawned it - the
  parent's `Stop` fires while the child works - so those events do not move
  the parent's state. `SubagentStart`/`SubagentStop` keep the subagent count
  instead, exactly as Claude's do (`subagent_ids`, paired on `agent_id`), so
  a Codex session with a live subagent counts as background work and is not
  suspended. A subagent's `PermissionRequest` still
  raises `AwaitingApproval`, since the user has to answer it either way.
- **`Interrupt`** ends a turn the user cut short, which Codex reports with no
  `PostToolUse` or `Stop`.
- **`SessionStart`** fires as the first turn begins, not when the process
  starts, and carries the conversation (thread) ID, which Panoptes records at
  once. Every `SessionStart` is adopted, since `/new` and forks move the
  session to a new conversation.

The first lifecycle hook marks the session `hooks_live`, and from then on the
hooks own its state: the rollout contributes usage figures and the
conversation title only (`state_machine::admits_transcript_event`), its
recency-based subagent count
is ignored, and a `notify` event left installed by an older Panoptes is
dropped as a duplicate of `Stop`. One known gap: Codex fires no hook for a
turn that fails on an API error, so with hooks live such a turn is not
reported as over, and the session reads `Thinking` until the next prompt.

**Codex `notify` (Codex before 0.156.1):** Limited to `notify` config firing
`agent-turn-complete` events. Codex spawns the `notify` argv directly, with no
shell, and appends the event JSON as its final argument; stdin is `/dev/null`
and output is discarded. Panoptes' hook ignores the event, and must never block
on stdin. The rest of such a session's state comes from its rollout - see
Reading Agent Transcripts below.

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
| `AwaitingApproval` | alive | blocked on a permission dialog or an MCP question | `PermissionRequest`, `Elicitation` |
| `Waiting` | alive | turn over, awaiting a prompt | `Stop`, `StopFailure`, `Interrupt` (Codex), shell foreground idle |
| `Suspended` | killed by us | scrollback kept, wakes on interaction | idle sweep |
| `Exited` | died itself | see `exit_reason` | `check_alive` |
| `Resumable` | never spawned | loaded from `sessions.json` | `reconcile` at startup |

Shell sessions render `Executing` as "Running" and `Waiting` as "Ready".

A turn can end with work still running: a background shell, a monitor, a
backgrounded subagent, or a `/loop` waiting to fire. A `Waiting` row says so -
`Waiting · 1 subagent · 2 in background · 1 scheduled` - which is also why the
suspend sweep leaves that session alone. Backgrounded subagents appear in both
of Claude's reports, and are counted once, as subagents.

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

1. An event raises an `AttentionReason` - `Approval`, `TurnComplete`, `Stalled`, `Crashed`, or `TurnFailed`
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
| `~/.panoptes/codex-homes/` | Per-account Codex shadow homes (only with `codex_shared_history`) |
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

#### Shared Codex history (`codex_shared_history`)

Off by default. When on, `codex_config::homes::CodexHomes` - built once from
the config and owned by the `SessionManager` - is the single resolver for
every Codex home:

- **Reading** (`data_home` / `sessions_dir`): the resume-blocker transcript
  check, conversation-ID discovery, the transcript watcher (rollout, subagent
  scan) and thread titles (`session_index.jsonl`) all read the shared home,
  canonicalised so two accounts key the watcher's per-directory caches by one
  path. Off, it returns each account's own home unchanged. The conversation
  importer (`import_scan_accounts`) asks it too: with shared history it
  searches the shared tree once, crediting finds to the account that lives
  in the shared home, else the default Codex profile.
- **Spawning** (`prepare_spawn`, called in `spawn_and_register`, the tail of
  create, resume and wake): an account whose home is the shared home runs
  there directly; any other runs from its shadow,
  `~/.panoptes/codex-homes/<account-id>/`, which is built or healed first.
  The Codex adapter adds `CODEX_SQLITE_HOME=<shared home>` for any
  `CODEX_HOME` under the shadows directory (`sqlite_home_for`).

A shadow holds a symlink to the account's own `auth.json` (a copy would fork
the refresh token) and symlinks for a fixed allow-list (`SHARED_ENTRIES`:
`sessions`, `archived_sessions`, `thread-writer-locks`, `session_index.jsonl`,
`history.jsonl`, `skills`, `plugins`, `rules`, `worktrees`, `cache`,
`mcp-oauth-locks`, `.tmp`, `config.toml`); the targets are pre-created so
nothing Codex writes goes private by accident. Everything else stays private.
`thread-writer-locks` must be shared: it is Codex's one-writer-per-thread lock
across processes. The SQLite files are shared through `CODEX_SQLITE_HOME`, not
links, because some are created lazily.

Invariants, from Codex 0.156.1:

- **Shadows are never deleted or moved**, and the shared home never changes
  under them: Codex's state DB stores `threads.rollout_path` *through the
  shadow*, and resume trusts it. Deleting an account removes only its shadow's
  `auth.json` link (`forget_account`).
- **Healing never clobbers**: a missing or wrong link is replaced; a real
  directory in a link's place is logged and left. The exception is
  `session_index.jsonl`, which `codex delete` rewrites by rename - its new
  lines are appended to the shared file and the link restored.
- **The accounts' own homes are only read** (`auth.json`'s existence, and
  `config.toml` for the startup divergence warning shown under About).
- `config.toml` is shared, so the `notify` chain (and any hook config) is
  written once, through the link; Codex resolves the link before its own
  atomic writes.

Discovery in a shared tree has no per-account directory to tell two accounts'
sessions apart, only the claimed-ID set - which was already global (live and
recovered sessions, every account), and grows as each pending session is
resolved in start order. Subagent counting matches on the parent's
conversation ID, which is globally unique, so a shared tree cannot confuse it.

`panoptes merge-codex-history [ACCOUNT]` (`codex_config::merge`) copies an
account's old rollouts and thread names into the shared home - copy, never
move, never overwrite, via a temporary name so Codex never lists a partial
file. Copied rollouts need no database rows; Codex lists and resumes them from
the file.

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
| `notify_on` | approval, turn_complete, crashed, failed | Which attention reasons ring the bell |
| `attention_on_idle` | false | Whether Claude's idle reminder raises attention |
| `claude_status_line` | true | Wrap Claude's status line to read rate limits and the real context window |
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
- Reserved keys are rejected (q, n, s, d, i, 0-9)
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
  ID start out as the same value. Resume passes `--resume <uuid>`, which keeps
  the ID (its `SessionStart` reports `source: resume` with the same
  `session_id`); `--fork-session` is never used, since forking mints a new ID.
  The two IDs do not stay equal, though: see *Following Claude across
  conversations* below.
- **Codex**: has no equivalent flag. With lifecycle hooks, its `SessionStart`
  hook reports the ID as the first turn begins, and Panoptes records it at once
  (well under a second after the first prompt, measured). Without hooks the ID
  is discovered instead. Codex writes a rollout file whose first line is a
  `session_meta` record carrying the session `id` and the `cwd` it started in;
  Panoptes matches on that `cwd` plus the session start time. A throttled scan
  runs only while some Codex session still lacks an ID, so it costs nothing in
  the steady state. It leaves a rollout younger than five seconds alone
  (`CODEX_HOOK_GRACE`), so a hook can claim it first: guessing is what goes
  wrong when several sessions share a directory. The notify hook cannot be used
  for this - it must not read stdin, or it stalls Codex's output pipeline and
  drops keystrokes.
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
directory has been deleted, which never recorded a conversation ID, or whose
conversation transcript is missing, is still listed but shows why it cannot be
brought back.

#### Importing conversations

A conversation started by running `claude` or `codex` by hand has a transcript
but no record, so Panoptes cannot resume it - until it is imported. `i` at a
branch searches for conversations started in that branch's working directory
(`transcript::scan`) and lists them in a centred overlay; `Enter` adopts one
through `SessionManager::adopt_external_conversation`, which builds a
`SessionInfo` exactly as a recovered one looks - `Resumable`, with the session
type, working directory, project and branch, account config id and name, and
the agent's conversation ID - and puts it in `recovered` and the store. From
there every existing resume path takes it unchanged: nothing spawns until it is
opened, and opening it runs `claude --resume <id>` / `codex resume <id>` under
the account it was found in. Its name is the agent's title, else the first
prompt, and is marked `auto_named`, so later agent retitles keep replacing it.

What is searched is every Claude and Codex profile, then the default account
of each (`$CLAUDE_CONFIG_DIR` or `~/.claude`, and `~/.codex`); a directory
listed twice is searched once, credited to the profile.

- **Claude** transcripts are filed by directory, so the slug of the working
  directory names the one folder to list (both the path as stored and its
  resolved form, since Claude files under the resolved one). Claude shortens a
  slug over 200 characters and appends a hash, so a longer path is matched by
  prefix. Each file's first 64 KB is read for the `cwd` its records carry (the
  slug is lossy, so this is the real check), the latest `ai-title`, and the
  first prompt a user typed - skipping injected records such as slash-command
  wrappers. A file with no exchange at all is skipped: resuming it fails.
- **Codex** rollouts are filed by date, every directory mixed together, so
  `sessions/YYYY/MM/DD` is walked newest day first, one day listed at a time.
  Each rollout gets an 8 KB probe; its header line is ~22 KB, but the `cwd`
  comes before the long base instructions, so most rollouts are ruled out on
  the probe alone. A match is read on to the end of the header line (up to
  256 KB) and classified: only `RolloutKind::Session` qualifies - subagent and
  system rollouts are not conversations the user had. The title is the
  thread's `session_index.jsonl` name (the index's last 1 MB, read once per
  `CODEX_HOME` that has a match), else the first `user_message`.

Candidates from every account are examined in one stream ordered by mtime, so
a budget that runs out costs the oldest conversations rather than one agent's
entirely. The budget (`ScanBudget`) is 500 files opened, 8 MB read, and 50
results; whichever runs out first stops the scan, and the picker says so.
Every read goes through one meter that charges its bytes and never reads past
what is left. Conversation IDs already owned by a session
(`claimed_agent_session_ids`) are never offered, and are ruled out from the
file name before anything is opened. Partial lines, unknown record types and
unreadable files are logged at debug and skipped.

The scan runs on a worker thread as a background job, behind the cancellable
loading overlay, so a cold disk cannot freeze the UI.

Only conversations whose working directory is a branch Panoptes knows are
reachable this way; a global browser, and offering to add a project for an
unknown directory, are out of scope.

#### Following Claude across conversations

Claude Code changes conversation inside a live process, and each change mints
or selects a different conversation ID. It announces every one through a
`SessionStart` hook whose payload carries the new `session_id` and
`transcript_path` (observed with Claude Code 2.1.280; the `compact` row is read
from its code, which reuses the session's own ID):

| Trigger | `source` | Payload `session_id` |
|---|---|---|
| process start with `--session-id` | `startup` | the dictated ID |
| process start with `--resume <id>` | `resume` | the same `<id>` |
| `/clear` | `clear` | a new ID |
| `/resume` inside the TUI | `resume` | the chosen conversation's ID |
| `/branch`, or `--resume <id> --fork-session` | `fork` | a new ID; the original is left intact |
| context compaction | `compact` | unchanged |

On any `SessionStart` except `compact` whose payload ID differs from the stored
one, `SessionManager::follow_agent_conversation` moves `agent_session_id` to it,
persists the record, and logs both IDs. The Panoptes session ID never moves -
hooks route on it, and it is the session's identity, not the conversation's.
The payload's `transcript_path` is kept alongside the new ID, and preferred over
the path derived from the working directory when choosing what to tail. Without
this, a `/clear` froze the usage display on the abandoned transcript, and a
suspension or a restart silently resumed the conversation from before it.

The transcript watcher follows on its own: `sync_transcript_watchers` recomputes
each session's target every couple of seconds and re-watches when the path
changes. A cleared or forked conversation is read from its first line, since
every record in it is this session's; an in-TUI `/resume` lands in an older
conversation and attaches at its end, as a relaunch with `--resume` does.

Codex's in-TUI `/new` and `/resume` are not followed: Codex's only hook channel
today is the single `notify` event, which carries no conversation ID. A Codex
hook that reported session starts would let the same path follow it.

#### Missing transcripts

`--resume` fails at launch if the conversation's transcript is not where the
agent will look: deleted, or left under a different `CLAUDE_CONFIG_DIR` or
`CODEX_HOME` than the account the session resumes under (with shared Codex
history, the shared home stands in for every account's). Such a session is
listed as unavailable - *conversation transcript is missing* - instead of being
offered and then failing. For Claude, "where it will look" is
`<config dir>/projects/<slug>/<id>.jsonl` (with every project directory tried
if our slug disagrees with Claude's); for Codex, a rollout under
`<CODEX_HOME>/sessions` whose `session_meta.id` is the ID.

`resume_blocker` is called every tick (the suspension sweep) and every frame
(the session list), so it does no I/O: it reports a cached
`SessionInfo::transcript_missing`. The actual look,
`SessionInfo::conversation_transcript_exists`, happens only when the recovery
list is built at startup, again at resume and at wake, and for a live session
once at the moment it would otherwise be suspended - after every cheap reason
not to has already been ruled out. A live session is not kept awake by this
unless its transcript is missing at that moment, which is exactly when
suspending it would be closing it: a Claude session that has not yet been sent
a message, or has just been `/clear`ed, has no transcript on disk yet. The next
agent event clears the cached answer, since that is what writes a transcript.

A default-account Claude session inherits Panoptes' own `CLAUDE_CONFIG_DIR`, if
set, so that - not `~/.claude` - is where its transcripts are looked for. Codex
does not inherit: its adapter always sets `CODEX_HOME` explicitly.

### Reading Agent Transcripts

Both agents write a complete record of every conversation to disk as it
happens, and Panoptes knows where because it stores each session's conversation
ID. Reading those files needs no cooperation from the agent, and for a Codex
without lifecycle hooks it is the only channel there is.

| | Claude Code | Codex CLI |
|---|---|---|
| File | `$CLAUDE_CONFIG_DIR/projects/<cwd-slug>/<uuid>.jsonl` | `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl` |
| Path is | as reported by `SessionStart` after a conversation change, else derived from cwd and ID | searched for, since the name embeds a timestamp |
| Drives state | only for a failed turn - hooks own the rest | only until lifecycle hooks report (never, on 0.156.1+) |
| Contributes | context usage, model, title, failed turns | context usage, model, rate limits, title (from `session_index.jsonl`); state and subagents without hooks |
| Measured flush latency | immediate | under 50ms |

Claude's rate limits and observed context window come from its status line
instead (see State Updates above); the transcript has neither.

The two tailers have deliberately different jobs. For a Codex without
lifecycle hooks, the rollout drives its state, since `notify` can only ever
report "my turn ended". Where hooks report, the transcript only supplements:
hooks report state sooner, and two producers writing the same field would
fight over it. Which applies is decided per session, by whether a lifecycle
hook has arrived (`SessionInfo::hooks_live`), and enforced by
`SessionManager::apply_transcript_event`.

One exception. A Claude turn that dies on an API error fires the `StopFailure`
hook *instead of* `Stop`. Claude also writes the failure to the
transcript - an assistant record with `"isApiErrorMessage": true` and an
`error` code such as `rate_limit` or `authentication_failed` - and the tailer
turns that into `AgentEvent::TurnFailed`, the same event `StopFailure`
translates to - both read the code through `transcript::claude::failure_reason`,
so they agree on the label. The state machine moves the session
to `Waiting` with a `TurnFailed` attention reason (a red `✗`, gated by
`notify_on.failed`), and ignores a second `TurnFailed` while the session is
still sitting on the first, so the hook and the transcript cannot double-fire
once both report it.

The Claude tailer skips records that are not the live conversation's own:
subagent turns (`isSidechain`), injected records (`isMeta`), compaction
summaries (`isCompactSummary`), and placeholders Claude wrote locally
(`"model": "<synthetic>"`, zeroed usage). Counting them would flash a
subagent's model, or a near-empty context, over the real session. A
subagent's API error is skipped with the rest - it is the subagent's failure.

**Agent titles.** Claude's `ai-title` records and the newest `thread_name` for
the session's thread in Codex's shared `$CODEX_HOME/session_index.jsonl`
(followed once per poll per `CODEX_HOME`, and searched in full on attach)
become `AgentEvent::TitleChanged`, which renames only an auto-generated session
name, persists it, and is not activity.

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

**Codex rate limits.** Each `token_count` record carries a `rate_limits` block
with two windows, `primary` (five hours, `window_minutes: 300`) and `secondary`
(a week, `10080`), each with `used_percent` and a `resets_at` in epoch seconds;
`plan_type` and `rate_limit_reached_type` sit beside them. Versions before
0.156 wrote `resets_at` as an RFC 3339 string and `plan_type` inside `primary`,
and both shapes are read. The block is only taken when `limit_id` is `codex`,
or absent as in older versions: model-specific allowances such as the Spark
model's `codex_bengalfox` report through the same record, and would otherwise
overwrite the account's figures with an unrelated pool's (in real rollouts,
about one `token_count` in fifteen). The session header shows whichever window
is closer to stopping the user - higher `used_percent` first, the longer window
on a tie - labelled by its length (`5h 12%`, `wk 40%`), or
`limit hit · resets in 3h 20m` while `rate_limit_reached_type` is set. Each
window merges independently, so an update naming only one never blanks the
other.

**Claude rate limits.** The status line's `five_hour` and `seven_day` windows
map onto the same `primary` / `secondary` windows (300 and 10080 minutes), so
the header shows Claude's limits exactly as it shows Codex's. Claude reports no
"limit reached" flag there; a turn refused for it still arrives as a failed turn
from the transcript.

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

**Claude's context window.** Claude's transcript names the model but never
its window, so the window is looked up in `CONTEXT_WINDOWS`
(`transcript/claude.rs`), a prefix table taken from the model catalogue
compiled into Claude Code (2.1.280, read 2026-09-23). Opus 4.7 and later,
Sonnet 5, Fable 5 and Mythos 5 run at 1M by default; older Opus and Sonnet,
Haiku 4.5 and the 3.x models run at 200k unless launched with a `[1m]` suffix.
An id the table does not know gets no window, and the header shows a raw token
count rather than a percentage of an invented number.

The table can only guess, because the transcript logs the bare id whatever the
window: `claude-opus-5-5[1m]` is logged as `claude-opus-5-5`, and so is a
native-1M model that Claude Code has capped at 200k (on most third-party
providers, under `CLAUDE_CODE_DISABLE_1M_CONTEXT`, or when the account cannot
pay for long context). So every window carries a `WindowSource`, weakest first:
`Inferred` from the table, `Launch` from a `--model …[1m]` argument Panoptes
spawned with, and `Observed` from the agent itself (Codex's
`model_context_window`, or Claude's status line's `context_window_size`).
`UsageSnapshot::merge` lets a window replace one from an equal or stronger
source, and a weaker one only when the model has changed, since the stronger
figure described the previous model.

**Copied parent history.** A forked Codex rollout - every subagent that inherits
its parent's context is one - opens with a copy of the parent's history: the
parent's own `session_meta`, then its turns, re-stamped to the instant of the
fork and written in one burst. Read from the start, that copy would replay the
parent's `task_started` / `task_complete` / `token_count` as if they were
happening now. `codex::CopiedHistorySkip` drops it, using the most exact
boundary the header allows:

| Rollout | Boundary |
|---|---|
| Not a fork (no `forked_from_id`), or a referenced fork (`history_base`) | nothing was copied |
| Paginated subagent | records with `ordinal` below the header's `subagent_history_start_ordinal` |
| Legacy fork, 0.156.1 (not 0.145) | up to and including the fork's own `thread_settings_applied` (the one whose `thread_id` is this rollout's) |
| Older legacy fork | the first gap of a second or more between record timestamps - a **heuristic** |

The heuristic is the fallback only for rollouts too old to mark the boundary:
the copy lands within milliseconds, the child's own work after a model round
trip. Its known cost is the child's opening `task_started`, written a few
milliseconds after the copy and indistinguishable from it. Attaching at EOF
finds the same boundary first, so the backwards usage scan never reaches into
the copy and a fresh fork does not show its parent's token count.

**Threading.** The watcher runs on its own OS thread and is drained with
`try_recv` each tick, the same shape as hook events. The reads are incremental,
but a burst of tool output can append a lot at once and parsing that on the
render thread would show as a stutter.

**Subagents.** Codex subagents run as their own threads, so a parent looks
completely idle while its children work - the mirror image of Claude, whose
subagents share the parent's session ID and show up in `in_flight`. With
lifecycle hooks the count is exact: `SubagentStart` and `SubagentStop` name
each subagent by `agent_id`. Without them it comes from the rollouts. A child
writes its own rollout and names its parent in `forked_from_id`, so discovery
is exact, but liveness is not. There is no "this subagent exited" record, so
it is inferred from recent writes. Either way the display is a count and not a
claim, and a session with subagents is never suspended.

Not every rollout with subagent-shaped metadata is a subagent. Each header is
classified as a `RolloutKind`, and only `Subagent` is counted:

| Kind | `session_meta.payload` |
|---|---|
| `System` | `source.internal` (any), `thread_source` of `memory_consolidation` or `guardian_review`, `source.subagent` of `"memory_consolidation"`, or `source.subagent.other` of `"guardian"` |
| `Subagent` | otherwise, any `source.subagent` (`thread_spawn`, `review`, `compact`, ...), `thread_source: "subagent"`, or a non-null `forked_from_id` |
| `Session` | everything else (`source: "cli"`, `"exec"`, `"vscode"`, ...) |

System threads are Codex working for itself: memory consolidation, and the
guardian that reviews approval requests - which is forked from, and so names,
the session it reviews for. Counting one would show subagents on a session that
is only waiting for the user, and would keep it from ever being suspended.
(Codex 0.156.1 runs memory consolidation ephemerally, so it writes no rollout;
older versions did, filed as `source.subagent`.)

The same classification keeps `discover_session_id` from claiming anything but a
`Session` as a session's own conversation - subagent rollouts sit in the same
working directory with their own fresh timestamps and otherwise match every
criterion. Note that on a subagent rollout `payload.id` is the subagent's own ID
while `payload.session_id` is its parent's.

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
excluded for the same reason. Neither moves on a status-line refresh, which
Claude also runs on timers. `last_activity` does move on PTY output, and is
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
- it has no subagents, background tasks or scheduled prompts
  (`SessionInfo::has_background_work`), whatever the agent. They all live in
  the agent's process and die with it, and they are exactly what a `Waiting`
  session that looks idle can still be running - Codex subagents in their own
  rollouts, Claude's backgrounded shells, monitors and agents after its turn has
  ended, and a `/loop` that is idle only until it fires
- its conversation transcript is on disk - looked for only once every clause
  above has passed, so the per-tick sweep does no I/O (see *Missing
  transcripts*)

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
