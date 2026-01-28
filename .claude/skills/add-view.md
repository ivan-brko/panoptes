# Add View Skill

Step-by-step checklist for adding a new view to Panoptes.

## Steps

### 1. Add View Enum Variant

In `src/app/view.rs`, add the new variant to the `View` enum:

```rust
pub enum View {
    // ... existing variants
    NewView,  // or NewView(SomeId) if it needs parameters
}
```

### 2. Implement parent() Method

In the same file, add the parent navigation case:

```rust
impl View {
    pub fn parent(&self) -> Option<View> {
        match self {
            // ... existing cases
            View::NewView => Some(View::ParentView),
        }
    }
}
```

### 3. Create Render Function

Create `src/tui/views/new_view.rs`:

```rust
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};
use crate::app::AppState;
use crate::tui::{frame::FrameLayout, theme};

pub fn render(frame: &mut Frame, state: &AppState, layout: &FrameLayout) {
    let content = layout.content;

    // Render your view content here

    // Footer with keyboard shortcuts
    let footer_text = "[↑↓] Navigate  [Enter] Select  [Esc] Back";
    // ...
}
```

### 4. Export from views/mod.rs

In `src/tui/views/mod.rs`, add:

```rust
pub mod new_view;
```

### 5. Add Render Dispatch

In `src/app/mod.rs`, in the `render()` method, add the case:

```rust
match &self.state.view {
    // ... existing cases
    View::NewView => views::new_view::render(frame, &self.state, &layout),
}
```

### 6. Create Input Handler

Create `src/input/normal/new_view.rs`:

```rust
use crossterm::event::{KeyCode, KeyEvent};
use crate::app::{App, AppResult};

pub async fn handle_key(app: &mut App, key: KeyEvent) -> AppResult<()> {
    match key.code {
        KeyCode::Up => {
            // Handle up navigation
        }
        KeyCode::Down => {
            // Handle down navigation
        }
        KeyCode::Enter => {
            // Handle selection
        }
        KeyCode::Esc => {
            app.navigate_back();
        }
        _ => {}
    }
    Ok(())
}
```

### 7. Export from normal/mod.rs

In `src/input/normal/mod.rs`, add:

```rust
pub mod new_view;
```

### 8. Add Input Dispatch

In `src/app/mod.rs`, in `handle_normal_mode_key()`, add:

```rust
View::NewView => normal::new_view::handle_key(self, key).await?,
```

### 9. Update Help Overlay

In `src/tui/views/help.rs`, add the shortcuts for the new view:

```rust
View::NewView => vec![
    ("↑/↓", "Navigate"),
    ("Enter", "Select"),
    ("Esc", "Back"),
],
```

## Verification

1. Run `cargo build` to check for compile errors
2. Run `cargo clippy -- -D warnings` to check for linting issues
3. Navigate to the new view in the app and verify:
   - Rendering works correctly
   - All keyboard shortcuts function
   - Navigation back works
   - Help overlay shows correct shortcuts
