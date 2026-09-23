# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.1] - 2026-09-23

### Fixed
- **Scrolling a fullscreen Claude Code session stops when the wheel does.** With Claude Code's fullscreen renderer (`"tui": "fullscreen"`) the wheel goes to Claude, which animates each scroll by repainting every few milliseconds — 300–700 KB of output per trackpad flick in a large window. Panoptes read that output on the UI thread, once per event-loop pass, out of a PTY that buffers only about 1 KB on macOS, so Claude's frames reached the screen seconds late: the view kept coasting for up to several seconds after the wheel stopped, and reversing direction bounced it back and forth. Every session's PTY is now drained continuously by its own reader thread into a queue capped at 1 MB, and the session on screen takes a whole burst in one pass. The view now stops within Claude's own ~0.15 s of smoothing. Backpressure is unchanged — a full queue still blocks the child — and a flooding session in the background costs the same CPU as before.
- **A Codex `notify` hook of your own keeps receiving its event.** When Codex already had a `notify` hook, Panoptes chained its own in front of it with a command under which Codex's event JSON landed in `$0` and reached neither hook, so the user's hook still ran every turn but with no event. The chain now passes the event to both, runs the user's hook even if Panoptes' fails, and no longer starts a login shell on every turn. Chains written by earlier versions are repaired on the next Codex spawn when the original command can be recovered exactly, and otherwise fall back to the manual-merge helper. An apostrophe in the user's hook command is now quoted correctly too.
- **The open session notifies you when the terminal is unfocused.** Notifications were suppressed for whichever session was on screen, even with you away in another app. A session now counts as watched only while it is on screen *and* the terminal has focus; terminals that do not report focus behave as before.

## [0.4.0] - 2026-07-27

### Added
- **Sessions survive a restart.** The agent processes die with Panoptes, but the conversations do not — Claude Code and Codex each write their own transcript, and what was missing was the index over them. That index now lives in `~/.panoptes/sessions.json`, so the next launch offers the sessions back as `Resumable` and reattaches to the actual conversation on open. Claude's ID is dictated at spawn with `--session-id` rather than discovered; Codex has no such flag, so its ID is found from the rollout file, matched on working directory and session start time. Nothing spawns until you open one. A session that cannot come back — deleted worktree, no recorded conversation — is still listed and says why, and a failed resume leaves the record intact so it can be retried or discarded.
- **Idle agent sessions are suspended to reclaim memory.** An idle Claude Code process measures around 565 MB, roughly 25× the whole Panoptes process. After `suspend_after_secs` (default 2h) the child is killed and the session moves to `Suspended`, while the scrollback stays readable — the buffer lives in Panoptes' memory, not the child's. Reading a suspended session is free; only interacting pays, and waking spawns a fresh agent through the same `--resume` path recovered sessions use, at about 2s regardless of conversation length. Never suspended: shells, anything not in `Waiting`, the session on screen, anything with no way back, and anything with subagents running.
- **Codex state and usage are read from the transcript.** Codex's notify hook can emit exactly one event and cannot be extended, so Codex used to be a black box between turns. Panoptes now tails the rollout file it already knows the path to, bringing Codex to parity with Claude: `Thinking`, `Executing` with named tools, `Waiting`, and interrupted turns that no longer stick. Claude's transcript only supplements — hooks keep owning its state and arrive sooner — contributing context usage and model, which hooks do not carry. Codex subagents are counted and shown, since a parent otherwise looks idle while its children work.
- **Select and copy with the mouse while a session is running.** Mouse capture has to stay on so the wheel can drive scrollback, which turns the terminal's own drag-selection off, so Panoptes does what tmux does: take the drag, paint the highlight in its own render pass, and lift the text out of the terminal buffer on release. Selections are anchored in absolute rows, so a drag held past the edge of the screen scrolls the view and keeps extending, accelerating the further past the edge you hold. Double-click drags by whole words and triple-click by whole lines, pivoting around the word they started in. `Shift` claims a drag from an agent that owns the mouse — Claude Code's TUI, `vim` with `mouse=a`, `htop` — which is what shift means in every terminal. `Ctrl` draws a rectangle instead of a stream, so one column can be taken out of `docker ps` without dragging the rest of every line with it. A copy says what it took, because the highlight is gone by the next frame and a silent failure otherwise looks identical to success. `selection_word_characters` and `multi_click_ms` are new config keys.
- **Four theme presets — Peacock, Io, Hera, Argus — picked live from Settings → Theme.** A preset owns only the chrome: accent, focused border, selected-row and text-selection surfaces, input prompt, default marker. States, outcomes and banners are the same colour in all four, so a session list reads identically whichever is on. The highlight is a live preview of the whole dashboard rather than a swatch — arrows repaint, `Enter` keeps, `Esc` puts back what was saved. Peacock is byte-for-byte the previous theme, so upgrading changes nothing until you pick something else.
- **A tiered semantic palette.** Every colour a view draws now comes from a named token, and the theme renders at one of three capability tiers — truecolor, 256, 16-ANSI — detected from `COLORTERM`/`TERM` and forceable with a new `theme` key in `config.toml`. The tiers agree on every chromatic token and differ only in the structural greys and surfaces.
- **A wordmark header.** The header's top-left carried "Panoptes" plus counts already repeated a row below in the pane titles; it now carries the mark and no text. The three-pane screen wears the full mark over its tagline and version; the session view wears the mark alone, because every row it takes is a row of agent output. A terminal too narrow or too short falls back to the one-line header, spelling out `PANOPTES` or showing the breadcrumb.
- **Three always-visible panes: Projects, Sessions, Settings.** `→`/`Tab` and `←`/`Shift+Tab` cycle focus; the focused pane widens and the other two shrink rather than disappearing, so watching sessions no longer means leaving the project tree. How aggressively the accordion leans depends on the terminal — from a 60/10/10 split with the sides reduced to counters at 80 columns, out to near-equal thirds at 200. Unfocused panes degrade full → compact → strip by dropping whole fields, never by cutting one long string. Opening a session is still full-screen, and `Esc` returns you to the pane you opened it from.
- **A Settings pane**, replacing four scattered entry points (`c`, `x`, `k`, `l`) with one place to look: Claude configs, Codex configs, custom shortcuts, notifications, and an About/paths section naming the version, the hook server's port and health, and where every file Panoptes writes actually lives.
- **Notification settings are editable while Panoptes runs** — how you are notified, the four attention reasons that ring, and whether Claude's idle nudge counts. Each takes effect on the next event with no restart. Everything else stays read-only, shown under About/paths, because it is only read at startup or when a session spawns.
- **Per-project settings as the last row of a project's branch list** — `⚙ Project settings`, below a divider and muted so it does not read as another branch — gathering the project's default Claude config, default Codex config, default base branch, and rename into one list. It is there whether the project has branches or not, so the settings are always one `Enter` away rather than behind a shortcut you have to know about.
- Projects can be grouped into folders in the projects overview, nested up to 3 levels deep (`m` to move, `r` to rename, `d` to ungroup, `Enter` to fold).
- **Creating a session is one modal, asking in the right order**: agent, then config, then name — where before it bounced between an overlay, the Projects pane, and an overlay again, and asked for a name before saying which account the session would run under. The config step is skipped when the agent has nothing to choose between, each step titles itself with what has already been chosen, and `Esc` steps back one rather than throwing the flow away.
- **A back row at every nested level of the Projects pane.** Row 0 of the branch list, the session list and the per-project settings list is the file-manager `..`, adapted: muted so it reads as chrome, reachable by arrow, and `Enter` on it does exactly what `Esc` does. It names the destination rather than the action, since the pane title already says where you are.
- **`Ctrl+Home` and `Ctrl+End` in a session** jump to the oldest line Panoptes holds and back to live output. They are the only two keys the session view takes, chosen for colliding with nothing a session runs — `bash` and `zsh` leave both unbound, and the programs that do bind them draw on the alternate screen, where the guard hands them straight back because there is no scrollback to reach anyway.
- **The settings description now rides the row it describes**, muted, after the label, instead of being glued into the footer where it parsed as an explanation of `Esc`. Where the row cannot hold both, the description pans under a fixed label — a new `Marquee` holds at the start long enough to read the first words, pans a column at a time, holds, and loops, and settles rather than holding the event loop at 60fps.
- **The help overlay scrolls.** The session section runs 27 lines and the overlay tops out at 26, so the bottom of the very list that tells you which keys survive a session was itself unreachable, and `↓` — reached for exactly then — was swallowed. `↑↓`/`PgUp`/`PgDn` now scroll it and the bottom border says how many rows are still below.
- A state summary strip atop the Sessions pane — `1 approval · 2 waiting · 1 exec`, each count in its state's colour — so the screen looks different when things are on fire. It only earns its row when the pane has height to spare.
- Attention badges carry their reason as a glyph as well as a colour: `●` turn complete, `◐` blocked on you, `✗` crashed, so the state survives a monochrome screenshot.
- Custom shortcuts can be set to close their session once the command finishes, for shortcuts that launch an external program and leave nothing worth keeping behind.
- Claude Code sessions are spawned with `--enable-auto-mode`.

### Changed
- **The session view has one mode, and every key but `Esc` belongs to the agent.** It used to have two — attached, where keys reached the PTY, and detached, where they were Panoptes' — and detached existed mainly so leaving it would drop mouse capture and let the terminal select. A drag copies while the session runs now, so what was left was arrow scrolling and a two-step `Esc`: a whole mode, with its header tag, footer, help section and capture dance, for two keys, and a mode you have to know you are in before you can predict what a keystroke does. `Esc` now leaves the session view outright, one press, and everything else reaches the agent. `Shift+Esc` is unchanged and is the only way to send a literal `Esc`.
- **`Esc` means exactly one thing everywhere: back one level.** "Esc at the overview quits" is gone; `q` quits instead, from every pane. At a pane's root `Esc` no longer dies silently — it returns focus to the Projects pane, and the accordion follows. The Projects overview stays a no-op, because it is home. In a session `q` types a `q`, like every other key.
- **`←`/`→` cycle panes, and `Enter` is the only action key.** Right is exactly `Tab` and left exactly `Shift+Tab`, unconditionally, with no guard and no drift between the dispatcher and the pane handlers. Two of the twelve pane states gave up a horizontal-arrow binding for it: the Projects overview's jump-to-parent, which up/down cover, and cycling notification methods backwards, where there are three methods and next wraps. Session mode and dialogs are untouched, so arrows still reach the PTY and still toggle Yes/No.
- **`Tab` is unambiguous app-wide.** It switches panes in normal mode, and nothing else: every other input mode owns it completely, so it still completes a path in the add-project prompt and still types a tab into an agent. It no longer silently means "next session" in the session view.
- **Panoptes now behaves like a real terminal to the agent it hosts, not a silent one.** It sits between each agent and your terminal, and it now answers what agents actually ask — cursor position, device attributes, `XTVERSION`, the kitty keyboard flags, synchronised-output support, and the foreground/background colours, the last two proxied from the real terminal so Codex's theme detection sees its true colours. Output between synchronised-update markers is held so the buffer only ever ingests whole frames, ending half-drawn renders. Clipboard writes from the agent (OSC 52) are forwarded on instead of being swallowed. Input encoding follows what the child negotiated, so `Shift+Enter` inserts a newline in both agents rather than submitting, and modified arrows keep their modifiers. Alternate-screen scroll semantics match a real terminal: an alt-screen app owns `PgUp`/`PgDn` and gets wheel notches as arrows when it has no mouse protocol. Verified against Claude Code 2.1.218/219 and Codex 0.145.0 by driving the real binaries in a PTY harness.
- **The focused pane is obvious at a glance.** Focus used to be a hue change at equal brightness, with unfocused panes wearing the brightest colour in the palette. It now rides four signals: border brightness, border weight (thick when focused, rounded when not), an explicitly declared title, and a dimmed text ramp in the body. The dimming carves out the signals — session state colours, attention badges and warning borders stay at full strength in every pane.
- **The session view drops its border, and the scroll indicator moves to the header.** The border's colour was the mode indicator, and with one mode it had nothing left to say while charging two rows and two columns of the agent's screen to say it. The header draws its own rule underneath and the footer its own above, so what is left is header, the agent edge to edge, footer. `Output [↑8]` — the only sign that the rows on screen are history rather than what the agent is doing now — moves into the header suffix and goes first, ahead of the agent tag, because it changes what everything below it means.
- **The session header stops narrating what the terminal already shows.** `Executing`, `Thinking`, `Waiting`, `Starting` and `Needs approval` all spent a row on the one thing the screen cannot fail to convey. `Exited` and `Suspended` stay, because both leave the output frozen mid-page, which looks exactly like a session sitting at its prompt. The agent and its account now share one bracket — `[CC · dot-lambda]` — which turned up a gap: only the Claude account name was ever read, so a Codex session showed none at all.
- The accordion gives the focused pane half the terminal from 200 columns, rather than backing off to 40% — which is what left a 250-column terminal 19 columns short of a full session row. The sides keep the full row format at 25% each.
- **Prompts split by content.** Anything showing a list or a paragraph — the add-project path and its completions, the folder move, all three worktree wizard steps, the base-branch selector, and every delete confirmation — is now a centred overlay anchored to the terminal, so an animating pane cannot resize a prompt under you and a list of paths is never truncated to one pane's width. One-line inputs stay inline in the pane that owns them.
- Reserved keys are now `q`, `n`, `s`, `d` and the digits. `c`, `g`, `G`, `k`, `x` and `,` are freed — a net gain of four bindable keys. A custom shortcut bound to a key that has since become reserved is dropped on load and reported in a startup notice, rather than being left in place to be silently shadowed.
- The "Needs Attention" list moved into the Sessions pane as a pinned top section. The blinking indicator stays in the global header, so it is visible from every pane including deep inside Settings.
- **Git operations no longer freeze the interface.** Fetching remotes and creating or removing a worktree ran on the event-loop thread behind a static "Please Wait" box, so the whole TUI stopped rendering — no session output, no hook updates, no way out — for as long as git took. They now run on a worker thread while the UI keeps rendering, under a "Working" overlay with an animated spinner. A slow `git fetch` can be called off with `Esc`: the fetch is killed and the flow continues with the refs already on disk, the same fallback used when a fetch fails. Worktree create and remove are deliberately not cancellable — interrupting one halfway leaves the repository worse off than not starting.
- Terminology is now consistent across footers, help, and dialogs: the Claude/Codex account configs are "configs" everywhere (were "Configs", "Configurations", and "accounts" in different places), and entering or leaving Session mode is "session mode" everywhere (the footer said "activate"/"deactivate" while the help said "Enter/Exit session mode").
- The worktree delete dialog says plainly that the git branch itself is never deleted, and its toggle is explicit about deleting the *directory* from disk.
- **All state files are now written atomically.** `projects.json`, `sessions.json`, both agent-config files, and `config.toml` are saved via a sibling temp file and rename, so a crash mid-write can never truncate a store. Corrupted files are uniformly backed up to a timestamped `<file>.corrupt.<timestamp>` sibling before starting fresh.
- `notification_method` is validated on load: `bell`, `title`, or `none`. Unknown values log a warning and fall back to `bell` instead of being silently misread.
- A stalled PTY write blocks the UI for at most ~50ms (was up to 1s), and runaway child output is drained with a per-tick budget, so a single misbehaving session can no longer starve the interface.
- Paste now works in every text-input mode — config names and paths, folder move/rename, and custom shortcut fields — not just session creation.
- UI consistency pass: a single `▶` selection glyph everywhere (the worktree wizard used `▸`), standard green/red Yes/No buttons in all confirmations, selector overlays styled like the other menus, and dialogs that clamp inside tiny terminals instead of overflowing.
- Spawn failures and worktree-wizard errors now include the underlying cause instead of a bare summary.
- **A long branch name no longer collapses a session row to an unidentifiable stub.** A branch slugified from a ticket title used to knock the row down to `name [state]` — no index, no project, no branch — leaving two sessions byte-identical on screen. The row's identity is now kept whole and the shortfall charged to project and branch, longer field first, middle-elided down to a 12-column floor.
- **Attention now means something happened, not that time passed.** `Stalled` used to be raised on every in-flight tool that outlived `state_timeout_secs`, so a session making a series of long Bash calls raised the badge, the header count and the footer nag once per call. The threshold still decides eviction — a tool report that old is not to be believed — but whether you are told is now a liveness question, because no threshold can tell a ten-minute build from a hang. Both agents redraw a spinner while a tool runs, so a session still talking is long-running rather than stalled; `Stalled` needs the tool overdue *and* the session silent for 30s *and* the session not to be the one on screen.
- Deleting a session from the overview asks for confirmation first.
- The branch-delete dialog puts its worktree toggle after the confirmation prompt, where it is visible rather than buried above it.
- `Esc` is requested from the terminal in an unambiguous encoding again, so it carries its modifiers and cannot be confused with the introducer of a mouse report. Terminals that do not support the protocol are unaffected.
- Every list now follows its own selection. Four views drew a list and then drew the selection wherever it happened to land — the worktree wizard worst of all, where on a repo with more branches than the overlay is tall, `↓` moved the selection below the visible rows and left you navigating blind. The About section gets a cursor too, not to act on a row but so a clipped list has something to follow.
- The footers say which key goes which way, one hint per direction, and stopped keeping quiet about `Esc` at the project level and `n`/`R` on folder rows.

### Fixed
- **A partial `config.toml` no longer refuses to start.** `hook_port`, `worktrees_dir`, `hooks_dir`, and `max_output_lines` had no defaults when deserializing, so a config file omitting any of them failed with `missing field ...` — including the example the configuration guide told you to create. All four now fall back to the same values `Config::default()` already used.
- Documentation now matches the code: the help overlay and keyboard reference listed `Shift+Tab`, `i`, `g`/`G`, and an `f` auto-follow toggle that were never implemented, and `1-9` in views that do not support it. `max_output_lines` and `theme_preset` are documented as parsed-but-unused, which is what they are.
- **Every Claude `Notification` hook rang the bell and flagged the session.** The `notification_type` values Panoptes matched (`idle`, `permission_request`, `task_completed`, `elicitation`) were not the ones Claude Code sends (`idle_prompt`, `permission_prompt`, `agent_completed`, `elicitation_dialog`, …), so all of them fell through to the unknown case, which assumes the agent wants you. The worst offender was `idle_prompt`, which fires repeatedly while you have *not* replied — so a session you had already read kept announcing itself with nothing new to show. `auth_success`, `elicitation_complete`, and `elicitation_response` are now classified as informational and stay silent.
- The session open on screen no longer flags itself as needing attention. An event arriving while you are looking at a session used to leave its badge set until you navigated away and back.
- Panics on non-ASCII text: both the settings-view path truncation and the paste-limit truncation sliced strings mid-codepoint and crashed on multi-byte characters.
- Codex subagent counts were computed from the default `CODEX_HOME` even for sessions running under a different Codex account, so their counts were wrong or missing.
- Multi-byte characters split across PTY reads no longer render as `�` in Codex fallback scrollback; trailing partial bytes are held until the rest of the character arrives.
- A failed paste or keystroke no longer leaves a session stuck showing "Thinking".
- Session cleanup leaked navigation-order entries, so number-key jumps could hit holes after sessions aged out.
- **A corrupt `config.toml` no longer aborts startup.** The unparseable file is backed up with a timestamp, defaults are used, and a visible warning explains what happened.
- A corrupt `~/.claude/.claude.json` no longer silently disables permission comparison — it is warned about and treated as empty, and the file itself is never touched.
- A stale default account pointing at a deleted profile now recovers deterministically (alphabetically first remaining profile) for Claude configs too, matching Codex.
- Removing a git worktree without force no longer deletes the working tree on disk, so local modifications survive.
- Malformed hook payloads are now logged instead of vanishing silently.
- Session-create failures always surface an on-screen error; previously only Codex sessions reported them.
- New sessions start with correct PTY dimensions, removing a brief mis-sized flash on open.
- Custom-shortcut sessions set auto-close at creation time, closing a race where a command that finished instantly missed the flag and never closed.
- **A session you had already read kept demanding attention forever.** Alongside the flag set by real events, `session_needs_attention` had a second, time-based rule: any session in `Waiting` whose last activity was older than `idle_threshold_secs` was flagged too. Acknowledging clears the flag, not the clock, so opening the session did nothing and the badge reappeared the instant you looked away. The rule was a leftover from when the state was called `Idle` and meant "this session has gone abnormally quiet" — a job now done by the `Stalled` attention reason. Renaming it to `Waiting`, the normal resting state of every healthy session, quietly turned it into "you finished a turn five minutes ago". It bit Codex hardest: a Codex session parked at its prompt writes nothing to its PTY or its rollout, so its activity clock never moved. Attention is now raised only by events that actually mean something.

- **Typing at a session whose process had died took Panoptes down with it.** The write to a closed PTY propagated out of the key handler and through the event loop's `?`. It is a no-op with a message now, and a write that fails anyway is logged rather than fatal. The exit itself was always detected promptly — what was broken is that nobody redrew, because the crash tick reported "did anything crash?" when asked "did anything happen?", and a shell you typed `exit` into crashes nothing. So the header went on claiming the session was live, in exactly the case it exists to report.
- **Resuming a Codex session started a brand-new conversation.** The conversation ID was passed into the spawn config but never read when building the argument list, so recovery appeared to work while silently discarding the conversation it was meant to bring back — worse than refusing outright.
- **Rollout discovery could claim the wrong Codex conversation.** It filtered on file mtime, which Codex bumps every turn, so an older conversation being actively used in the same directory looked newer than the session being identified — reachable in normal use, since two Codex sessions on one branch share a working directory. Discovery now matches on the rollout's own unchanging timestamp, skips conversations already claimed, and takes the oldest unclaimed match while callers resolve oldest-first. It also no longer claims a subagent's rollout as its parent's conversation.
- **A recovered session that could not come back could not be discarded either.** The delete confirmation bailed out early when the session was not live, which every recovered session is, so the branch that discards the record was unreachable — leaving no way to clear it short of deleting the whole branch or project.
- **A mid-turn `SessionStart` was read as a finished turn.** Claude fires it for five sources, and `compact` fires on its own whenever the context window fills, in the middle of a turn the agent is still working on. Forcing `Waiting` there reported a busy session as finished. Only `startup`, `resume`, `clear` and `fork` reset the session now; anything else leaves the state untouched rather than guess.
- **A `PermissionRequest` or `Notification` arriving for a session already in `Waiting` was dropped.** Subagents share their parent's session ID, so these events routinely arrive without a state transition, and the guard only set the flag on a transition into `Waiting`. Attention is also no longer cleared by a state change alone, and is cleared when you actually type at the session.
- **Reaching the top of a Codex session's history dropped you back to live output.** A vterm that cannot advance because it is at the top of its scrollback and one that cannot advance because it has no scrollback at all read identically, so one `PgUp` too many snapped the view to the bottom and into the shallower fallback buffer — scrolling up moved the view down. The two are now told apart by asking whether the vterm holds any history.
- **Scrolling a non-Codex session clamped against capacity rather than against history.** Fifty lines of output and a determined wheel left the counter at 10000 and the screen on row 47, and every notch back then moved the counter without moving the view, so reaching live output again took thousands of notches.
- A drag held against the oldest line kept asking to scroll one line further every tick, so the offset climbed away from the history that exists and left the wrong figure in the scroll indicator on release.
- **Mouse clicks landed one row below the pointer, and the PTY was sized one row taller than the visible area**, clipping the agent's bottom row: everything reasoning about the session content area off-screen assumed a 3-row header while the view drew a 4-row one. `PgUp` made the same assumption and moved one row further than the screen did, dropping a line of output through the seam on every press. There is one answer for the header's height now, and no default to guess with.
- **Key-release events made `Shift+Tab` cycle two panes, killed auto-repeat outside session mode, and dismissed error toasts as they appeared.** They were invited by a flag pushed for a hold-to-exit `Esc` feature that no longer exists. The flag is gone, and every branch that could see one guards against a terminal reporting them unasked.
- Leaving a session mid-drag left its output held indefinitely: dropping mouse capture means the button release never arrives, and a selection stuck in the dragging state is what holds the reads back.
- A drag no longer stops a build. Holding a selection used to stop reading the session's PTY, and once the kernel buffer filled the child blocked on its next write. The bytes are drained and held instead, then replayed read by read on release, so the child experiences what it would have minus the stall. Past 8MB held the selection is dropped rather than the buffer grown.
- **The stall watchdog skipped every non-Claude session**, which was right when only Claude reported in-flight tools. Codex reports them now, so a tool whose completion never arrived would pin the session in `Executing` forever — and since the suspend sweep only considers `Waiting` sessions, that also kept the process permanently alive.
- Transcript tailers attached at the end of the file even for a session that had just written it, so everything a new Codex session did while its rollout was being discovered was skipped rather than merely delayed. Characters split across a read boundary are also held back now instead of being replaced with `U+FFFD` before the rest arrived.
- `Shift+Esc` was the one keystroke that would not wake a suspended session — it forwarded straight to an orphaned PTY without the wake guard every other input path has. A Codex prompt pasted without bracketed paste also left the session showing `Waiting` for the whole turn, since Codex has no prompt hook and the submission is inferred from `Enter`.
- The agent-config and custom-shortcut lists showed their selection regardless of pane focus, so a bright bold row sat inside a dimmed pane and read as a second focus.
- Folder headings in the projects view used a blue that was nearly illegible on a dark background, and the same value as the "starting" state — putting a structural label in the channel you scan for status. They are set apart by weight now, leaving the row's colour free to carry status. `1 branches` is also grammatical.

### Removed
- **The log viewer, and the in-memory log buffer that fed it.** Panoptes is not a better `cat`: file logging to `~/.panoptes/logs/` is untouched, with its 7-day retention, and Settings → About/paths tells you which file is current. The 10,000-entry ring buffer that mirrored every log line into memory is gone with it.
- The global `k` shortcut overlay. Custom shortcuts are managed from Settings → Shortcuts, one place and one path.
- **Detached mode in the session view**, and with it `Enter` to attach, the two-step `Esc`, and arrow/`PgUp`/`PgDn` scrolling of session history. Those keys are the agent's now — `PgUp` typing into Claude Code is worse than not having it — and the wheel, `Ctrl+Home` and `Ctrl+End` are what scrolling is. `q` in the session view goes the same way and types a `q`.
- **Shell sessions are no longer persisted across restarts.** They were written to the durable index and brought back labelled `Resumable`, the same word used for the agent sessions beside them. For an agent it is honest; a shell has no transcript, and its state is the scrollback, the environment and whatever it is running, none of which survive the PTY — the respawn was a blank prompt in the recorded directory, which is what creating a new shell session on that branch already gives you. Records left by earlier versions are dropped on load. The quit prompt now counts the live shells and says they will be killed along with anything running in them, while agent sessions return on the next start.
- The `,` shortcut for per-project settings, which is now the last row of the branch list. `,` leaves the reserved set and is bindable as a custom shortcut again.
- Option-drag is no longer offered as the way to copy out of an agent that owns the mouse — `Shift`-drag does it properly, with a highlight, word selection and a message saying what was taken. It survives in one place: Codex's fallback history is plain text recovered from the byte stream with no terminal cells behind it, so Panoptes refuses to select it and the terminal's own selection is all there is.
- The `idle_threshold_secs` config key, which only fed the time-based attention rule described above. Leaving it in `config.toml` is harmless — unknown keys are ignored.
- **Activity Timeline view.** The `a` shortcut, the view, and its documentation are gone. It listed every session sorted by recency, but selecting a row and opening it used two different orderings — the list sorted by last activity while `Enter` indexed creation order — so it opened the wrong session as soon as the two diverged. The homepage Sessions panel covers the same ground without that flaw. `a` is now free for a custom shortcut.
- **Vim-style `j`/`k` navigation.** It only ever worked in the log viewer, the two config views, and four selector dialogs, and `k` never reached a view at all — it is globally bound to the custom shortcuts manager. Navigation is now consistently by arrow key.
- **Focus timer and focus statistics.** The `t`, `T`, and `Ctrl+t` shortcuts, the Focus Statistics view, and all focus-interval tracking are gone.
  - The `focus_timer_minutes` and `focus_stats_retention_days` config keys are no longer read. Leaving them in `config.toml` is harmless — unknown keys are ignored.
  - `~/.panoptes/focus_sessions.json` is no longer read or written. Existing files are left on disk and can be deleted by hand.
  - `t` and `T` are no longer reserved and can now be bound as custom shortcuts.
- **Overlay notification system.** The focus timer was its only producer, so `NotificationManager`, `NotificationType`, and the notification overlay are gone. Transient messages still appear in the header, and session attention still rings the bell / sets the terminal title per `notification_method`.
- **Terminal focus tracking.** Focus-change reporting is no longer requested from the terminal. A session you are currently viewing no longer rings when you switch away from the terminal window — attention notifications now fire only for sessions you are *not* looking at.
- Ordinal numbering in the projects list; digit keys now select only in the Sessions list.

### Technical
- Major internal refactor with no intended behavior change beyond the entries above: a shared persistence layer (`persistence.rs`), a generic agent-profile store (`agent_profiles.rs`) backing both Claude and Codex configs, a pure session state machine (`session/state_machine.rs`), unified Claude/Codex config input handlers and views, and a decomposed event loop. Unit test count grew from 520 to about 660.
- The Claude hook script forwards the agent's whole payload through `jq` rather than building JSON by shell interpolation, which meant the first quote or newline in any field produced malformed JSON and lost the event silently. Without `jq` it degrades to the envelope alone and warns. Four more hooks are registered: `UserPromptSubmit` makes `Thinking` an observation rather than a guess about keystrokes, and `SessionStart`/`SessionEnd` bracket the process.
- **Terminal emulation moves to a fork of `vt100`, published as [`panoptes-vt100`](https://crates.io/crates/panoptes-vt100) 0.16.2** and depended on under the name `vt100`, so imports are unchanged. It carries one behaviour fix — lines scrolled out of a scroll region whose top is row 1 now reach scrollback, matching xterm and iTerm, which was the root cause of Codex's missing history since its inline UI pins a footer with scroll regions — and five additive accessors that address rows absolutely, which is what a selection surviving a scrolling view requires. The fork lives at [ivan-brko/panoptes-vt100](https://github.com/ivan-brko/panoptes-vt100) and exists only until the changes can go upstream.
- An end-to-end harness (`tests/e2e/`) drives the real `claude`, `codex` and `zsh` binaries in a PTY playing the terminal's role, covering mouse selection, forwarded click coordinates, the OSC 52 clipboard fallback, the Codex scroll collapse, and the alternate-screen guard on `Ctrl+Home`/`Ctrl+End`. It copies Codex credentials into a scratch home rather than symlinking the real `~/.codex`, which Panoptes rewrites to install its notify hook.

## [0.3.1] - 2026-02-11

### Changed
- Refactored session scrolling handlers to share keyboard scroll logic across Session mode and normal Session view.
- Isolated Codex fallback scroll state to Codex sessions only.

### Fixed
- Restored reliable mouse-wheel scrolling in active Codex sessions.
- Fixed Codex upward-scroll edge case where history could get stuck at `Output [↑1]`.
- Fixed top-of-history over-scroll behavior that could visually remove bottom lines while scrolling up.

## [0.3.0] - 2026-02-11

### Added
- OpenAI Codex CLI support — run Codex sessions alongside Claude Code with the same attention tracking and session management
- Multi-account support for Codex CLI (CODEX_HOME-based configuration)
- Session type indicators ([CC], [CX], [SH]) in all session lists
- DSR (Device Status Report) query handling for PTY sessions

### Changed
- Updated shortcut documentation to be agent-agnostic
- Updated Ctrl+C warning to mention Esc as an alternative quit key

### Fixed
- Scroll not working in Codex sessions
- Codex session creation dialog not rendering
- Codex character dropping caused by blocking stdin read in notify hook
- Codex hook setup hardened to surface configuration failures
- Codex config selector bugs

### Removed
- codex-harness diagnostic binary

## [0.2.2] - 2025-01-29

### Added
- Custom shell session shortcuts feature - define custom keyboard shortcuts that execute shell commands
- Custom shortcuts support in branch detail view
- Custom shortcuts documentation in UI footers
- Shell session attention/notification support
- Comprehensive FAQ documentation
- Session mode troubleshooting questions to FAQ

### Changed
- Skip notifications and attention flag for active session (no notifications while you're viewing that session)

### Fixed
- Deletion dialog now shows correct warning for session type
- Keyboard shortcut documentation discrepancies

## [0.2.1] - 2025-01-27

### Added
- Hook event coalescing to prevent UI lag during rapid state changes
- Branch refresh feature (`R` key) to check for stale worktrees
- Stale worktree indicators (red highlighting for missing worktrees)
- Improved disk error handling with user-friendly messages for disk full and permission errors
- Comprehensive documentation:
  - [Keyboard Reference](docs/KEYBOARD_REFERENCE.md)
  - [Configuration Guide](docs/CONFIG_GUIDE.md)
  - [Troubleshooting Guide](docs/TROUBLESHOOTING.md)
  - [Installation Guide](docs/INSTALLATION.md)

### Changed
- README.md overhauled with improved structure and documentation links

## [0.1.0] - 2025-01-23

### Added

#### Core Features
- Multi-session management for Claude Code
- Project and branch organization with git repository support
- Real-time session state tracking (Starting, Thinking, Executing, Waiting, Idle, Exited)
- Session naming for easy identification
- Automatic session cleanup on quit

#### Git Integration
- Git worktree support for branch isolation
- Worktree creation wizard with branch selection
- Default base branch configuration per project
- Remote branch fetching via git CLI

#### Navigation
- Hierarchical navigation (Projects -> Branches -> Sessions)
- Activity timeline view for all sessions sorted by recent activity
- Keyboard-driven interface with vim-style navigation
- Number shortcuts for quick selection (1-9)

#### Attention System
- Terminal bell notifications when sessions need input
- Visual attention badges (green for new, yellow for idle)
- Attention count indicators in header
- Space key to jump to next session needing attention

#### Focus Timer
- Pomodoro-style focus timer with configurable duration
- Per-project and per-branch time tracking
- Focus statistics view with session history
- Terminal focus detection for accurate time tracking

#### User Interface
- Unified header component with notifications
- Transient header notifications for feedback
- Loading indicators for blocking operations
- Confirmation dialogs for destructive actions
- Log viewer for application debugging

#### Configuration
- TOML configuration file support
- Configurable hook server port
- Configurable notification method (bell, title, none)
- Theme presets (dark, light, high-contrast)

### Technical
- Built with Rust using async/await (Tokio runtime)
- Ratatui for terminal UI
- Axum for HTTP hook server
- vt100 crate for terminal emulation
- Portable-pty for PTY management
- git2 for git operations

### Fixed
- Paste handling with retry logic for non-blocking PTY writes
- Git fetch operations using git CLI for proper SSH authentication
- Branch name validation in worktree wizard
- Focus timer countdown accuracy with Alt+Tab detection
- Escape key behavior (Shift+Escape forwards to PTY)

[Unreleased]: https://github.com/ivan-brko/panoptes/compare/v0.4.1...HEAD
[0.4.1]: https://github.com/ivan-brko/panoptes/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/ivan-brko/panoptes/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/ivan-brko/panoptes/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/ivan-brko/panoptes/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/ivan-brko/panoptes/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/ivan-brko/panoptes/compare/v0.1.0...v0.2.1
[0.1.0]: https://github.com/ivan-brko/panoptes/releases/tag/v0.1.0
