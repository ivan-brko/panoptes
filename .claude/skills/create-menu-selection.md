# Create Menu Selection - Developer Guide

This guide walks you through creating a selectable list menu in Panoptes with consistent styling and behavior.

## Overview

A **selectable list menu** is a UI component that displays a list of items where users can navigate with arrow keys and select items with Enter. Examples in Panoptes include:

- **Projects Overview** (`src/tui/views/projects.rs`) - Grid of projects
- **Branch Detail** (`src/tui/views/branch_detail.rs`) - List of sessions for a branch
- **Activity Timeline** (`src/tui/views/timeline.rs`) - All sessions sorted by activity
- **Focus Stats** (`src/tui/views/focus_stats.rs`) - Focus session history
- **Claude Configs** (`src/tui/views/claude_configs.rs`) - Claude configuration accounts

All menus follow a consistent pattern: **arrow prefix (`▶`) + bold text + state/accent colors**, with **NO background highlighting**.

## When to Use This Pattern

Use this pattern when you need:
- A list of selectable items
- Keyboard navigation (Up/Down arrows)
- Visual indication of the selected item
- Consistent look and feel with the rest of the app

## Quick Start Checklist

Follow these steps to create a new selectable list menu:

### 1. State Management (`src/app/state.rs`)
- [ ] Add selection index field to `AppState` (e.g., `selected_my_menu_index: usize`)
- [ ] Initialize to `0` in `AppState::new()`
- [ ] Add navigation helper methods (see "State Management" section below)

### 2. View Enum (`src/app/view.rs`)
- [ ] Add new variant to `View` enum
- [ ] Implement `parent()` method for back navigation

### 3. Render Function (`src/tui/views/my_menu.rs`)
- [ ] Create new file in `src/tui/views/`
- [ ] Import selection helpers: `use crate::tui::widgets::selection::{selection_prefix, selection_style};`
- [ ] Build list items with selection styling (see "View Creation" section)
- [ ] Export from `src/tui/views/mod.rs`

### 4. App Render Dispatch (`src/app/mod.rs`)
- [ ] Add case to `render()` method to dispatch to your render function

### 5. Input Handler (`src/input/normal/my_menu.rs`)
- [ ] Create handler file in `src/input/normal/`
- [ ] Implement Up/Down navigation
- [ ] Implement Enter to select/open
- [ ] Implement other shortcuts (Delete, New, etc.)
- [ ] Export from `src/input/normal/mod.rs`

### 6. Input Routing (`src/input/dispatcher.rs`)
- [ ] Add case to route key events to your handler

## State Management

### Adding Selection Index

In `src/app/state.rs`, add a field to track the selected item:

```rust
pub struct AppState {
    // ... other fields ...

    /// Selected index in my menu
    pub selected_my_menu_index: usize,
}
```

Initialize in `new()`:

```rust
impl AppState {
    pub fn new() -> Self {
        Self {
            // ... other fields ...
            selected_my_menu_index: 0,
        }
    }
}
```

### Navigation Helper Methods

Add methods to handle selection changes safely:

```rust
impl AppState {
    /// Select next item in my menu
    pub fn select_next_my_menu_item(&mut self, count: usize) {
        if count > 0 {
            self.selected_my_menu_index = (self.selected_my_menu_index + 1) % count;
        }
    }

    /// Select previous item in my menu
    pub fn select_prev_my_menu_item(&mut self, count: usize) {
        if count > 0 {
            self.selected_my_menu_index =
                (self.selected_my_menu_index + count - 1) % count;
        }
    }

    /// Reset selection when entering the view
    pub fn reset_my_menu_selection(&mut self) {
        self.selected_my_menu_index = 0;
    }
}
```

## View Creation

### File Structure

Create a new file `src/tui/views/my_menu.rs`:

```rust
//! My menu view
//!
//! Brief description of what this menu displays.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::app::AppState;
use crate::tui::header::Header;
use crate::tui::header_notifications::HeaderNotificationManager;
use crate::tui::layout::ScreenLayout;
use crate::tui::theme::theme;
use crate::tui::views::Breadcrumb;
use crate::tui::widgets::selection::{selection_prefix, selection_style_with_accent};

/// Render my menu view
pub fn render_my_menu(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    items: &[MyItem], // Your data type
    header_notifications: &HeaderNotificationManager,
    attention_count: usize,
) {
    let t = theme();

    // Build header
    let breadcrumb = Breadcrumb::new().push("My Menu");
    let header = Header::new(breadcrumb)
        .with_suffix(format!("({} items)", items.len()))
        .with_notifications(Some(header_notifications))
        .with_attention_count(attention_count);

    // Create layout
    let areas = ScreenLayout::new(area).with_header(header).render(frame);

    // Build list items
    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == state.selected_my_menu_index;
            let prefix = selection_prefix(is_selected);

            // Build content spans
            let content = Line::from(vec![
                Span::raw(prefix),
                Span::styled(
                    &item.name,
                    selection_style_with_accent(is_selected, &t)
                ),
                // Add more spans as needed
            ]);

            ListItem::new(content)
        })
        .collect();

    let list = List::new(list_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("My Menu ({})", items.len()))
    );

    frame.render_widget(list, areas.content);
}
```

### Selection Helpers

Panoptes provides three helper functions in `src/tui/widgets/selection.rs`:

#### 1. `selection_prefix(is_selected: bool) -> &'static str`

Returns the arrow prefix for list items:
- Selected: `"▶ "`
- Unselected: `"  "` (two spaces for alignment)

```rust
let prefix = selection_prefix(is_selected);
```

#### 2. `selection_style_with_accent(is_selected: bool, theme: &Theme) -> Style`

Most common pattern - uses theme's accent color (cyan) for selected items:
- Selected: bold + accent color (cyan)
- Unselected: normal text color

```rust
let style = selection_style_with_accent(is_selected, &t);
```

#### 3. `selection_style(is_selected: bool, base_color: Color) -> Style`

For state-based coloring (e.g., session states, attention indicators):
- Selected: bold + base_color
- Unselected: base_color without bold

```rust
// Use session state color
let style = selection_style(is_selected, session.state.color());

// Use attention color
let style = if attention_count > 0 {
    selection_style(is_selected, t.attention_badge)
} else {
    selection_style_with_accent(is_selected, &t)
};
```

### Rendering Patterns

#### Simple List (One Color)

For simple menus where all items have the same color:

```rust
let content = Line::from(vec![
    Span::raw(selection_prefix(is_selected)),
    Span::styled(&item.name, selection_style_with_accent(is_selected, &t)),
]);
ListItem::new(content)
```

#### State-Based Colors

For items with state (sessions, projects with active counts):

```rust
let content = Line::from(vec![
    Span::raw(selection_prefix(is_selected)),
    Span::raw(format!("{}: ", i + 1)),
    Span::raw(&item.name),
]);

let style = selection_style(is_selected, item.state.color());
ListItem::new(content).style(style)
```

#### Complex Items with Badges

For items with attention indicators or status badges:

```rust
let content = Line::from(vec![
    Span::raw(selection_prefix(is_selected)),
    Span::styled("● ", Style::default().fg(badge_color)),  // Badge
    Span::styled(&item.name, selection_style_with_accent(is_selected, &t)),
    Span::styled(format!("  {}", item.status), Style::default().fg(t.text_muted)),
]);
ListItem::new(content)
```

### Important: NO Background Highlighting

❌ **DO NOT** use background colors for selection:

```rust
// WRONG - creates invisible white-on-white text
let style = if is_selected {
    Style::default().bg(t.selected)  // NO!
} else {
    Style::default()
};
```

✅ **DO** use arrow prefix + bold + color:

```rust
// CORRECT - visible and consistent
let prefix = selection_prefix(is_selected);
let style = selection_style_with_accent(is_selected, &t);
```

## Input Handling

### Creating the Handler

Create `src/input/normal/my_menu.rs`:

```rust
//! Input handlers for my menu view

use crossterm::event::{KeyCode, KeyEvent};

use crate::app::App;

/// Handle key events in my menu view
pub fn handle_my_menu_key(app: &mut App, key: KeyEvent) -> anyhow::Result<bool> {
    match key.code {
        // Navigation
        KeyCode::Up | KeyCode::Char('k') => {
            let count = get_item_count(app);  // Your logic to get count
            app.state.select_prev_my_menu_item(count);
            Ok(true)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let count = get_item_count(app);
            app.state.select_next_my_menu_item(count);
            Ok(true)
        }

        // Selection
        KeyCode::Enter => {
            let selected_index = app.state.selected_my_menu_index;
            // Handle selection - open detail view, perform action, etc.
            Ok(true)
        }

        // Back navigation
        KeyCode::Esc | KeyCode::Char('q') => {
            app.state.view = app.state.view.parent();
            Ok(true)
        }

        // Other actions
        KeyCode::Char('d') => {
            // Delete selected item
            Ok(true)
        }
        KeyCode::Char('n') => {
            // Create new item
            Ok(true)
        }

        _ => Ok(false),  // Key not handled
    }
}
```

### Routing in Dispatcher

In `src/input/dispatcher.rs`, add your view to the dispatch:

```rust
use crate::input::normal::my_menu::handle_my_menu_key;

pub fn handle_normal_mode_key(app: &mut App, key: KeyEvent) -> anyhow::Result<bool> {
    match app.state.view {
        // ... other views ...
        View::MyMenu => handle_my_menu_key(app, key),
        // ...
    }
}
```

### Navigation Patterns

#### Up/Down with Wrap-Around

```rust
KeyCode::Up => {
    let count = items.len();
    app.state.select_prev_my_menu_item(count);
    Ok(true)
}
KeyCode::Down => {
    let count = items.len();
    app.state.select_next_my_menu_item(count);
    Ok(true)
}
```

#### Enter to Select

```rust
KeyCode::Enter => {
    let selected_index = app.state.selected_my_menu_index;
    if let Some(item) = items.get(selected_index) {
        // Navigate to detail view
        app.state.view = View::MyMenuDetail(item.id);
        // Or perform action
        perform_action(item);
    }
    Ok(true)
}
```

#### Reset Selection When Entering View

```rust
// In the handler that navigates TO your view
app.state.view = View::MyMenu;
app.state.reset_my_menu_selection();
```

## Complete Example: Projects Overview

Here's an annotated excerpt from the Projects view (`src/tui/views/projects.rs`):

```rust
use crate::tui::widgets::selection::{selection_prefix, selection_style, selection_style_with_accent};

fn render_project_list(
    frame: &mut Frame,
    area: Rect,
    projects: &[&Project],
    selected_index: usize,
    focused: bool,
    sessions: &SessionManager,
    idle_threshold_secs: u64,
) {
    let t = theme();

    let items: Vec<ListItem> = projects
        .iter()
        .enumerate()
        .map(|(i, project)| {
            let selected = i == selected_index && focused;
            let prefix = selection_prefix(selected);  // "▶ " or "  "

            // Count sessions for state-based coloring
            let active_count = sessions.active_session_count_for_project(project.id);
            let attention_count = sessions.attention_count_for_project(project.id, idle_threshold_secs);

            let content = format!("{}{}: {}", prefix, i + 1, project.name);

            // Color precedence: attention > active > accent
            let style = if selected {
                if attention_count > 0 {
                    selection_style(true, t.attention_badge)  // Bold yellow
                } else if active_count > 0 {
                    selection_style(true, t.active)  // Bold green
                } else {
                    selection_style_with_accent(true, &t)  // Bold cyan
                }
            } else if attention_count > 0 {
                Style::default().fg(t.attention_badge)  // Yellow
            } else if active_count > 0 {
                Style::default().fg(t.active)  // Green
            } else {
                Style::default().fg(t.text)  // White
            };

            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Projects ({})", projects.len()))
    );

    frame.render_widget(list, area);
}
```

Input handler (`src/input/normal/projects_overview.rs`):

```rust
pub fn handle_projects_overview_key(app: &mut App, key: KeyEvent) -> anyhow::Result<bool> {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            let count = app.project_store.project_count();
            app.state.select_prev_project(count);
            Ok(true)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let count = app.project_store.project_count();
            app.state.select_next_project(count);
            Ok(true)
        }
        KeyCode::Enter => {
            let projects = app.project_store.projects_sorted();
            if let Some(project) = projects.get(app.state.selected_project_index) {
                app.state.view = View::ProjectDetail(project.id);
                app.state.reset_branch_selection();
            }
            Ok(true)
        }
        KeyCode::Char('d') => {
            // Delete selected project
            app.state.input_mode = InputMode::ConfirmDeleteProject;
            Ok(true)
        }
        _ => Ok(false),
    }
}
```

## Common Patterns

### Pattern 1: Arrow Prefix
Always use for consistency:
```rust
let prefix = selection_prefix(is_selected);  // "▶ " or "  "
```

### Pattern 2: Bold + Color for Selected
```rust
// Simple: accent color
let style = selection_style_with_accent(is_selected, &t);

// State-based: session/project state color
let style = selection_style(is_selected, state_color);
```

### Pattern 3: NO Background Highlighting
Never use `.bg()` for selection - it creates white-on-white text.

### Pattern 4: State-Based Coloring
```rust
// Precedence: attention > active > default
let style = if selected {
    if has_attention {
        selection_style(true, t.attention_badge)
    } else if is_active {
        selection_style(true, t.active)
    } else {
        selection_style_with_accent(true, &t)
    }
} else {
    // Unselected: same color logic without bold
    if has_attention {
        Style::default().fg(t.attention_badge)
    } else if is_active {
        Style::default().fg(t.active)
    } else {
        Style::default().fg(t.text)
    }
};
```

### Pattern 5: Numbered Items
```rust
let content = format!("{}{}: {}", prefix, i + 1, item.name);
```

### Pattern 6: Status Indicators
```rust
let content = Line::from(vec![
    Span::raw(prefix),
    Span::styled("● ", Style::default().fg(badge_color)),  // Badge first
    Span::styled(&item.name, name_style),
    Span::styled(format!("  [{}]", status), Style::default().fg(t.text_muted)),
]);
```

## Testing Checklist

After implementing your menu, verify:

### Visual
- [ ] Arrow (`▶`) appears next to selected item
- [ ] Selected item is bold
- [ ] Selected item has appropriate color (accent or state color)
- [ ] Unselected items are normal weight
- [ ] NO white background highlighting
- [ ] Selection is clearly visible against background
- [ ] Consistent with other menus (Projects, Branches, etc.)

### Navigation
- [ ] Up arrow moves selection up
- [ ] Down arrow moves selection down
- [ ] Selection wraps from bottom to top
- [ ] Selection wraps from top to bottom
- [ ] Vim keys (`j`/`k`) work if implemented

### Behavior
- [ ] Enter selects/opens the item
- [ ] Selection persists when leaving and returning to view
- [ ] Selection resets to 0 when appropriate
- [ ] Empty list doesn't crash
- [ ] Single item list works correctly

### Integration
- [ ] Footer shows correct shortcuts for the view
- [ ] Back navigation works (Esc/q)
- [ ] View appears in breadcrumb correctly
- [ ] Attention count updates correctly

## Common Mistakes to Avoid

1. **Background Highlighting**: Never use `.bg(t.selected)` - creates invisible white-on-white
2. **Forgetting Bold**: Always make selected items bold for visibility
3. **Missing Wrap-Around**: Navigation should wrap at top/bottom
4. **Hardcoded Indices**: Always use modulo arithmetic for safe wrapping
5. **Inconsistent Shortcuts**: Match existing patterns (Up/Down, j/k, Enter)
6. **Missing Footer Update**: Document shortcuts in `src/tui/views/` footer section
7. **Not Using Helpers**: Always use `selection_prefix()` and `selection_style()` helpers

## Footer Help Text

Don't forget to add/update the footer help text in your render function:

```rust
// In your render function, after creating the layout:
let areas = ScreenLayout::new(area)
    .with_header(header)
    .with_footer("↑↓: navigate | Enter: select | d: delete | n: new | Esc: back")
    .render(frame);
```

**IMPORTANT**: Keep the implementation (in `src/input/`) and documentation (footer) in sync!

## Further Reading

- **Architecture**: See `docs/TECHNICAL.md` for overall architecture
- **View System**: See `src/app/view.rs` for view navigation
- **Input System**: See `src/input/dispatcher.rs` for input routing
- **Theme System**: See `src/tui/theme.rs` for color definitions
- **Examples**: Study existing menus in `src/tui/views/` and `src/input/normal/`

## Summary

Creating a new selectable list menu involves:
1. Adding selection state to `AppState`
2. Creating a render function with selection styling (arrow + bold + color)
3. Creating an input handler with Up/Down/Enter navigation
4. Routing input events in the dispatcher
5. Using selection helper functions for consistency
6. **Never using background highlighting**

Follow this guide and study the existing examples to create menus that feel native to Panoptes!
