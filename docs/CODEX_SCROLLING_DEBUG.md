# Codex Session Mouse Scrolling Debug Log

Last updated: 2026-02-11

## Goal

In active Session mode, mouse wheel should scroll session output up/down in Panoptes for Codex sessions (same expectation as Claude sessions).

## Context

- Codex standalone can rely on terminal/native scroll behavior in inline mode.
- In Panoptes, mouse events are handled in `src/app/mod.rs` and may be forwarded to PTY or handled as Panoptes-local scrollback.

## Changes Tried

1. Force Codex inline mode (`--no-alt-screen`)

- Files:
  - `src/agent/codex.rs`
- What changed:
  - Added default Codex arg `--no-alt-screen` via `build_args()`.
  - Switched spawn path to use `build_args()`.
  - Added tests for arg injection and de-duplication.
- Why:
  - Inline mode preserves scrollback and avoids alternate-screen behavior.

2. Prioritize local wheel handling over PTY forwarding

- Files:
  - `src/app/mod.rs`
- What changed:
  - Mouse wheel (`ScrollUp`, `ScrollDown`) is handled first as Panoptes-local scrollback.
  - Non-wheel mouse events are still forwarded to PTY when session mouse mode is active.
- Why:
  - Forwarded wheel packets can be ignored by Codex, making scrolling appear broken.

3. Fix missing mouse-capture enable on one session-entry path

- Files:
  - `src/input/normal/projects_overview.rs`
- What changed:
  - Added `app.tui.enable_mouse_capture()` in `HomepageFocus::Sessions` Enter path.
- Why:
  - Without mouse capture, no mouse events are received.

4. Add session-mode mouse-capture safety net

- Files:
  - `src/app/mod.rs`
- What changed:
  - In event loop, when `View::SessionView` + `InputMode::Session`, force-enable mouse capture.
- Why:
  - Prevent missing capture from any unhandled navigation path/regression.

5. Add debug-level tracing for wheel handling

- Files:
  - `src/app/mod.rs`
- What changed:
  - Added opt-in mouse diagnostics controlled by `PANOPTES_MOUSE_DEBUG`.
  - Logs wheel-event receipt in the event loop with view/input mode/capture state.
  - Logs local wheel-scroll handling and resulting scroll offset.
- Why:
  - Confirms whether wheel events are arriving and whether scroll offset updates.

6. Freeze active-session PTY polling while scrolled up

- Files:
  - `src/app/mod.rs`
  - `src/session/manager.rs`
- What changed:
  - Added `SessionManager::poll_outputs_except(...)`.
  - Event loop now skips PTY reads for the active session when `session_scroll_offset > 0`.
- Why:
  - Prevents live output from shifting/overwriting the scrolled history view.

7. Sync UI scroll offset to vt100 clamped offset

- Files:
  - `src/app/mod.rs`
  - `src/input/session_mode.rs`
  - `src/input/normal/session_view.rs`
- What changed:
  - Scroll handlers now apply a requested offset, then read back `vterm.scrollback_offset()`.
  - App-level `session_scroll_offset` is always synchronized to the parser’s actual value.
- Why:
  - Avoids app/parser drift when requested offsets exceed available vt100 history.

8. Add plain-text fallback history when vt100 scrollback is unavailable

- Files:
  - `src/session/mod.rs`
  - `src/app/mod.rs`
  - `src/input/session_mode.rs`
  - `src/input/normal/session_view.rs`
  - `src/tui/views/session.rs`
- What changed:
  - Session now keeps a plain-text fallback buffer from PTY output.
  - Added ANSI-stripping logic (with split-sequence handling) before writing to fallback buffer.
  - Mouse wheel and PgUp/PgDn now:
    - use vt100 scrollback when available
    - fall back to plain-text history when vt100 offset stays at `0`
  - Session view now renders fallback history when `session_scroll_offset > 0` but vt100 offset is `0`.
  - Reset fallback scroll when returning to live view / sending normal input.
- Why:
  - Some Codex rendering paths can leave vt100 scrollback empty even though output exists.
  - This keeps mouse-wheel scrolling functional in Panoptes session mode.

9. Clamp fallback "scroll up" at viewport top

- Files:
  - `src/session/mod.rs`
  - `src/app/mod.rs`
  - `src/input/session_mode.rs`
  - `src/input/normal/session_view.rs`
- What changed:
  - Added viewport-aware fallback scrolling:
    - `scroll_up_with_viewport()`
    - `scroll_to_top_with_viewport()`
  - Mouse wheel/PageUp now clamp fallback offset to `total_lines - viewport_height`.
  - This prevents over-scrolling past the top, which previously caused bottom lines to disappear one-by-one.
- Why:
  - Makes fallback scrolling stop cleanly at top and match expected terminal behavior.

10. Scope scrolling fixes to Codex sessions only

- Files:
  - `src/app/mod.rs`
  - `src/input/session_mode.rs`
  - `src/input/normal/session_view.rs`
  - `src/tui/views/session.rs`
  - `src/input/normal/projects_overview.rs`
  - `src/session/mod.rs`
- What changed:
  - Codex-only wheel interception and fallback history remain enabled.
  - Non-Codex (Claude/Shell) wheel and keyboard scroll paths keep original behavior.
  - PTY-output polling freeze while scrolled is applied only to active Codex sessions.
  - Fallback history collection in `Session::poll_output()` is only active for Codex.
  - Session scroll indicator/footer use app-level fallback offset only for Codex.
  - Session-entry mouse-capture safety path in Projects view is now Codex-only.
- Why:
  - Prevent regressions in Claude/Shell sessions while preserving Codex scrolling fixes.

11. Refactor scrolling code paths for maintainability

- Files:
  - `src/input/session_scroll.rs`
  - `src/input/session_mode.rs`
  - `src/input/normal/session_view.rs`
  - `src/app/mod.rs`
  - `src/session/mod.rs`
- What changed:
  - Added shared keyboard scroll helpers in `input/session_scroll.rs`.
  - Switched both session key handlers (`Session` mode and normal Session view) to reuse shared helpers.
  - Split `App::handle_mouse_event` into focused helper methods:
    - Codex wheel handling
    - non-Codex wheel handling
    - PTY-forwarding path
    - shared layout/logging helpers
  - Wrapped fallback history internals behind a Codex-only `Option<CodexFallback>` in `Session`.
    - Non-Codex sessions no longer allocate/track fallback state.
    - Existing fallback APIs remain no-op/empty for non-Codex.
- Why:
  - Reduce duplication and make Codex-specific behavior explicit and isolated.
  - Lower risk of behavior drift between keyboard and mouse scroll implementations.

12. Fix Codex "stuck at ↑1" upward-scroll edge case

- Files:
  - `src/app/mod.rs`
  - `src/input/session_scroll.rs`
- What changed:
  - In Codex upward scroll paths, after requesting more vt100 scrollback we now check
    whether the vterm offset actually advanced.
  - If vterm offset does not advance (even if non-zero), we:
    - force vterm back to bottom (`scroll_to_bottom()`)
    - continue scroll-up using fallback history
  - This ensures render switches to fallback path (`vterm_offset == 0`) and scrolling can continue past the shallow vt100 cap.
- Why:
  - Some Codex screens report only a tiny vterm scrollback (commonly offset `1`) and never increase further.
  - Without this guard, mouse/PageUp appears "stuck" at `Output [↑1]`.

## External Verification Notes

Checked local Codex source tree at `/tmp/codex`:

- Codex `--no-alt-screen` is intended for inline/scrollback use.
- Codex event mapper drops mouse events in its own TUI stream.
- Codex alternate-scroll mode (`?1007h`) is only enabled in alternate screen path.

Implication:

- Forwarding wheel events from Panoptes to Codex PTY is not a reliable scroll strategy.
- Panoptes-local wheel scrollback handling is the safer default.

## Validation Run

- `cargo fmt`
- `cargo test --quiet`
- `cargo lint`

## Next Debug Steps If Still Reproducible

1. Verify fallback path in logs:
   - `vterm_offset=0` with `fallback_offset>0` during wheel up.
2. Compare behavior between:
   - newly created Codex session after restart
   - existing pre-change Codex session
3. Capture terminal/multiplexer info (`TERM`, tmux/zellij, terminal app).

## How To Enable Mouse Diagnostics

Run Panoptes with:

```bash
PANOPTES_MOUSE_DEBUG=1 cargo run
```

Then inspect logs in:

- Panoptes log viewer (`l` from Projects view), or
- log file under `~/.panoptes/logs/`.
