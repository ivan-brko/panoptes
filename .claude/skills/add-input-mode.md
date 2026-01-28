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
| Multi-step wizards | `src/wizards/<wizard>/handlers.rs` |

### 3. Create Handler Function

In the chosen module, add the handler:

```rust
pub async fn handle_new_mode(app: &mut App, key: KeyEvent) -> AppResult<()> {
    match key.code {
        KeyCode::Char(c) => {
            // Handle character input
        }
        KeyCode::Enter => {
            // Confirm/submit
            app.state.input_mode = InputMode::Normal;
        }
        KeyCode::Esc => {
            // Cancel
            app.state.input_mode = InputMode::Normal;
        }
        KeyCode::Backspace => {
            // Handle backspace if text input
        }
        _ => {}
    }
    Ok(())
}
```

### 4. Add Dispatch Case

In `src/input/dispatcher.rs`, in `handle_key_event()`, add:

```rust
InputMode::NewMode => {
    text_input::handle_new_mode(app, key).await?;
    // or dialogs::handle_new_mode(app, key).await?;
    // depending on handler location
}
```

### 5. Add State Fields (if needed)

If the mode needs to track state (e.g., text buffer, cursor position), add fields to `AppState` in `src/app/state.rs`:

```rust
pub struct AppState {
    // ... existing fields
    pub new_mode_buffer: String,
    pub new_mode_cursor: usize,
}
```

### 6. Add Trigger Logic

Add code to enter the new mode from normal mode. Usually in the relevant view's input handler:

```rust
KeyCode::Char('n') => {
    app.state.input_mode = InputMode::NewMode;
    app.state.new_mode_buffer.clear();
}
```

### 7. Update Rendering (if needed)

If the mode has visual feedback (input box, cursor), update the view's render function to display it when in this mode.

## Common Patterns

### Text Input Mode

```rust
pub async fn handle_text_mode(app: &mut App, key: KeyEvent) -> AppResult<()> {
    match key.code {
        KeyCode::Char(c) => {
            app.state.text_buffer.insert(app.state.cursor_pos, c);
            app.state.cursor_pos += 1;
        }
        KeyCode::Backspace if app.state.cursor_pos > 0 => {
            app.state.cursor_pos -= 1;
            app.state.text_buffer.remove(app.state.cursor_pos);
        }
        KeyCode::Left if app.state.cursor_pos > 0 => {
            app.state.cursor_pos -= 1;
        }
        KeyCode::Right if app.state.cursor_pos < app.state.text_buffer.len() => {
            app.state.cursor_pos += 1;
        }
        KeyCode::Enter => {
            // Process the input
            let input = app.state.text_buffer.clone();
            // ... do something with input
            app.state.input_mode = InputMode::Normal;
        }
        KeyCode::Esc => {
            app.state.input_mode = InputMode::Normal;
        }
        _ => {}
    }
    Ok(())
}
```

### Confirmation Dialog Mode

```rust
pub async fn handle_confirm_mode(app: &mut App, key: KeyEvent) -> AppResult<()> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            // Perform confirmed action
            app.state.input_mode = InputMode::Normal;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            // Cancel
            app.state.input_mode = InputMode::Normal;
        }
        _ => {}
    }
    Ok(())
}
```

## Verification

1. Run `cargo build` to check for compile errors
2. Run `cargo clippy -- -D warnings`
3. Test the mode:
   - Enter the mode via the trigger
   - Verify input handling works
   - Verify Esc cancels properly
   - Verify Enter/confirmation works
