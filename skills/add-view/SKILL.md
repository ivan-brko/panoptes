---
name: add-view
description: >
  Step-by-step checklist for adding a new view to Panoptes.
  Follow the checklist to create the view enum variant, render function,
  input handler, and update all dispatch points.
---

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

Create `src/tui/views/new_view.rs`. Views receive the frame, the full area,
and whatever state they need; they build their header/content/footer split
with `ScreenLayout` and use the shared footer helpers:

```rust
use ratatui::prelude::*;

use crate::tui::header::Header;
use crate::tui::layout::ScreenLayout;
use crate::tui::theme::theme;
use crate::tui::views::{render_footer, Breadcrumb};

pub fn render_new_view(frame: &mut Frame, area: Rect, state: &AppState /* , ... */) {
    let header = Header::new(/* breadcrumb, notifications, attention count */);
    let areas = ScreenLayout::new(area).with_header(header).render(frame);

    // Render view content into areas.content

    // Footer with keyboard shortcuts
    render_footer(frame, areas.footer, "[↑↓] Navigate  [Enter] Select  [q] Back");
}
```

Use the shared building blocks rather than hand-rolling them:
- `tui/widgets/selection.rs` - selection glyph/styles (`selection_prefix`,
  `selection_style`) so lists highlight consistently
- `tui/widgets/dialog.rs` - `centered_rect` / `render_dialog` / `yes_no_line`
  for overlay dialogs (they clamp inside tiny terminals)
- `tui/views/mod.rs` helpers - `session_state_display`, `footer_with_attention`,
  `status_parts`, `visible_window`

### 4. Export from views/mod.rs

In `src/tui/views/mod.rs`, modules are private and render functions are
re-exported:

```rust
mod new_view;
// ...
pub use new_view::render_new_view;
```

### 5. Add Render Dispatch

In `src/app/mod.rs`, in the `render()` method, add an arm to the
`match state.view` block. The match is exhaustive on purpose - adding a `View`
variant without deciding what it renders is a compile error:

```rust
match state.view {
    // ... existing cases
    View::NewView => {
        render_new_view(frame, area, state /* , ... */);
    }
}
```

Overlays (dialogs shown over the view) are dispatched separately in the same
method by the exhaustive `match state.input_mode` block.

### 6. Create Input Handler

Create `src/input/normal/new_view.rs`. Handlers are synchronous and return
`anyhow::Result<()>`:

```rust
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use crate::app::App;

pub fn handle_new_view_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Up => { /* navigate up */ }
        KeyCode::Down => { /* navigate down */ }
        KeyCode::Enter => { /* select */ }
        KeyCode::Char('q') | KeyCode::Esc => {
            app.state.navigate_back();
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

In `src/app/mod.rs`, in `handle_normal_mode_key()`, add an arm to the
`match self.state.view` block:

```rust
View::NewView => normal::new_view::handle_new_view_key(self, key),
```

### 9. Update Help Overlay

In `src/tui/views/help.rs`, add the shortcuts for the new view.

### 10. Add View Tests

View render functions are unit-tested with the helpers in
`src/tui/views/test_util.rs` (`render_to_lines`, `contains_line`, ...):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::views::test_util::{contains_line, render_to_lines};

    #[test]
    fn test_new_view_renders_title() {
        let lines = render_to_lines(80, 24, |frame| {
            render_new_view(frame, frame.size(), &state /* , ... */);
        });
        assert!(contains_line(&lines, "Expected title"));
    }
}
```

## Verification

1. Run `cargo build` to check for compile errors
2. Run `cargo lint` (clippy with `-D warnings`)
3. Run `cargo test`
4. Navigate to the new view in the app and verify:
   - Rendering works correctly
   - All keyboard shortcuts function
   - Navigation back works
   - Help overlay shows correct shortcuts
