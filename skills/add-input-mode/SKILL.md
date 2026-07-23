---
name: add-input-mode
description: >
  Step-by-step checklist for adding a new input mode to Panoptes.
  Follow the checklist to add the enum variant, create the handler
  in the appropriate module, and update the dispatcher.
---

# Add Input Mode Skill

Step-by-step checklist for adding a new input mode to Panoptes.

## Steps

### 1. Add InputMode Enum Variant

In `src/app/input_mode.rs`, add the new variant:

```rust
pub enum InputMode {
    // ... existing variants
    NewMode,  // or NewMode { field: Type } if it needs state
}
```

### 2. Determine Handler Location

Choose the appropriate module based on the mode type:

| Mode Type | Handler Location |
|-----------|-----------------|
| Text input (naming, paths) | `src/input/text_input.rs` |
| Confirmation dialogs | `src/input/dialogs.rs` |
| Session interaction | `src/input/session_mode.rs` |
| Claude/Codex config flows | `src/input/agent_configs.rs` (one handler, parameterized by `AgentKind`) |
| Multi-step wizards | `src/wizards/<wizard>/` |

If the mode exists for both Claude and Codex, do not write it twice: add it to
`input/agent_configs.rs` and extend `AgentKind` with whatever differs (the
`InputMode` variants it moves between, wording, which store it touches).

### 3. Create Handler Function

Handlers are synchronous and return `anyhow::Result<()>`. Prefer a parts-based
handler that takes only what it needs (`&mut AppState`, a store) so it can be
unit tested without a terminal, plus a thin `handle_*` wrapper that
destructures `App` for the dispatcher — see `input/agent_configs.rs` for the
pattern:

```rust
pub fn handle_new_mode_key(app: &mut App, key: KeyEvent) -> Result<()> {
    new_mode_key(&mut app.state, key)
}

fn new_mode_key(state: &mut AppState, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Char(c) => { /* handle character input */ }
        KeyCode::Enter => {
            // Confirm/submit
            state.input_mode = InputMode::Normal;
        }
        KeyCode::Esc => {
            // Cancel
            state.input_mode = InputMode::Normal;
        }
        _ => {}
    }
    Ok(())
}
```

### 4. Add Dispatch Case

In `src/input/dispatcher.rs`, in `handle_key_event()`, add an arm to the
`match app.state.input_mode` block:

```rust
InputMode::NewMode => super::text_input::handle_new_mode_key(app, key),
```

Then update the routing-table mirror in the same file's test module: the
`handler_family()` function is deliberately exhaustive over `InputMode`, so
adding a variant without deciding its routing is a compile error. Add the new
mode to it and `test_every_mode_reaches_intended_handler_family` verifies the
routing.

### 5. Add Overlay Rendering

Overlay dialogs are dispatched by the exhaustive `match state.input_mode`
block in `App::render()` (`src/app/mod.rs`) — a new variant will not compile
until you decide what it renders (possibly nothing). Build the dialog itself
with the shared widgets in `tui/widgets/dialog.rs` (`centered_rect`,
`render_dialog`, `yes_no_line`) so it matches the other dialogs and clamps
inside tiny terminals.

### 6. Add State Fields (if needed)

If the mode needs to track state (e.g., a text buffer or a selection index),
add fields to `AppState` in `src/app/state.rs`. Reuse existing draft structs
where they fit (e.g., `session_draft`, `config_draft`).

If the mode has a text field, also wire it into `App::paste_into_mode_field`
(`src/app/mod.rs`) so paste works in it like in every other text-input mode.

### 7. Add Trigger Logic

Add code to enter the new mode from normal mode, usually in the relevant
view's input handler:

```rust
KeyCode::Char('n') => {
    app.state.input_mode = InputMode::NewMode;
}
```

### 8. Check Mode/View Consistency

`validate_mode_view_consistency()` in `src/input/dispatcher.rs` resets modes
that only make sense in a particular view (e.g., Session mode outside
SessionView). If the new mode is view-bound, add it there.

## Verification

1. Run `cargo build` — the exhaustive matches in `App::render()` and the
   dispatcher test's `handler_family()` will flag anything you missed
2. Run `cargo lint`
3. Run `cargo test`
4. Test the mode manually:
   - Enter the mode via the trigger
   - Verify input handling, paste, Esc-cancel, and Enter-confirm
