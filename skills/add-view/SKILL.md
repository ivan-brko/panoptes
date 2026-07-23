---
name: add-view
description: >
  Step-by-step checklist for adding a new screen to Panoptes. Panoptes has
  three always-visible panes, so "a new screen" is a new drill-down level in
  one of them, a new Settings section, or a new overlay - never a new
  top-level view. Follow the checklist for whichever it is.
---

# Add View Skill

Panoptes shows **three panes at once** — Projects, Sessions, Settings — and one
of them holds focus. There is no `View` enum and no one-screen-at-a-time model:
a session filling the terminal is the only exception, and there is exactly one
of those.

So before writing anything, decide which of these you are actually adding:

| What you want | Where it goes | Checklist |
|---|---|---|
| A deeper level inside the project tree | `ProjectsNav` variant | [A](#a-a-new-drill-down-level-in-pane-1) |
| A new settings screen | `SettingsNav` variant | [B](#b-a-new-settings-section-pane-3) |
| A prompt, selector, or confirmation | An overlay or an inline input | [C](#c-a-new-prompt) |
| A fourth pane | Don't | — |

A fourth pane is out of scope by design: the accordion's width table is built
for three, and panes are not reorderable or user-resizable.

---

## A. A new drill-down level in pane 1

### 1. Add the `ProjectsNav` variant

In `src/app/nav.rs`:

```rust
pub enum ProjectsNav {
    // ... existing levels
    NewLevel(ProjectId),
}
```

Then add its arm to `parent()` (what `Esc` pops to) and, if it carries IDs, to
`project_id()` / `branch_id()`. Both matches are exhaustive, so the compiler
will find them for you.

### 2. Render it

Add an arm to the `match state.projects_nav` in
`src/tui/views/pane_projects.rs::render_projects_pane`, and one to
`projects_breadcrumb` for the pane's block title.

Your renderer receives the area **inside** the pane border — the pane owns its
border and title. Two rules apply to every row:

- **Truncate against `area.width`**, using `truncate_string` / `clamp_line`.
  Nothing may render past the pane border.
- **Drop fields whole as the pane narrows** rather than truncating one long
  string. Branch on the `SideMode` you were handed (`Full` / `Compact`), and
  add a `Strip` case to `render_strip` — ten columns, so a counter and nothing
  else.

The mode comes from the pane's *current* width, which is why a pane can change
density part-way through an accordion transition. Never cache it.

### 3. Handle its keys

Add an arm to the `match app.state.projects_nav` in
`src/input/normal/projects_pane.rs::handle_key`, and write the level's handler
beside the others. `Esc` calls `app.state.navigate_back()`.

Do **not** handle `Tab`, `q`, `?`, or `Space`: those are global, handled in
`src/input/dispatcher.rs` before your handler runs.

### 4. Pin any new input mode to it

If the level opens a text input or dialog, add the mode to
`validate_mode_focus_consistency` in `src/input/dispatcher.rs`. The match is
exhaustive: a new `InputMode` without a decision there is a compile error, and
`test_every_mode_has_a_place_it_is_valid` fails if it is rejected everywhere.

---

## B. A new settings section (pane 3)

### 1. Add the `SettingsNav` variant

In `src/app/nav.rs`, add the variant, then add it to `SECTIONS` (which is what
puts it in the list) and give it a `title()` and a one-line `description()` —
the description is what shows in the global footer when the row is highlighted.

### 2. Render it

Add an arm to the `match ctx.state.settings_nav` in
`src/tui/views/pane_settings.rs::render_settings_pane`. Same two rules as
above: truncate against the pane width, drop fields rather than cutting them.

### 3. Handle its keys

Add an arm to the `match app.state.settings_nav` in
`src/input/normal/settings_pane.rs::handle_key`, plus its handler. `Esc` calls
`navigate_back()`; give it a footer entry in `settings_footer`
(`src/tui/views/panes.rs`).

### 4. If it edits config

Only settings the runtime **re-reads on every event** belong here. Anything read
at startup or at spawn time goes read-only under About / paths instead — a
half-applied setting is worse than one that plainly needs a restart.

If you do add a live one: mutate `app.config`, then call `persist(app)`, which
pushes the change into `SessionManager::apply_runtime_config` (the state machine
reads the manager's own copy, not `App::config`) and saves the file.

---

## C. A new prompt

**The rule: if it shows a list or a paragraph it is a centred overlay; if it is
one line you type into, it is inline in the pane that owns it.**

Overlays are anchored to the terminal, so an animating pane can never resize a
prompt under the user mid-typing, and a list of paths is not truncated to one
pane's width.

### Inline (one line)

Add an arm to the `match state.input_mode` near the top of
`render_projects_pane` and call `render_inline_input`. It returns early, so the
prompt replaces the pane's content.

### Overlay (a list or a paragraph)

Write it in `src/tui/views/prompts.rs` (or `worktree.rs` for worktree flows),
using `centered_rect` / `render_dialog` from `tui/widgets/dialog.rs`, or
`render_confirm_dialog` with `overlay: Some(...)` for confirmations. Then add an
arm to the exhaustive `match state.input_mode` in `src/app/mod.rs::render`.

### Either way

- Add the mode's footer line to `prompt_footer` in `src/tui/views/panes.rs`.
- Add the mode to `validate_mode_focus_consistency`.
- If it consumes `Tab` (autocomplete) or `Space`, you get that for free: the
  global keys only fire in `InputMode::Normal`.

---

## Shared building blocks

Use these rather than hand-rolling:

- `tui/panes.rs` — `side_mode(width)` for density, `pane_widths` for the table
- `tui/widgets/selection.rs` — `selection_prefix`, `selection_style`,
  `activity_style` so lists highlight consistently
- `tui/widgets/dialog.rs` — `centered_rect`, `render_dialog`, `yes_no_line`
  (they clamp inside tiny terminals)
- `tui/views/mod.rs` — `session_state_display`, `footer_with_attention`,
  `status_parts`, `visible_window`, `truncate_string`, `truncate_path`
- `tui/views/pane_projects.rs` — `clamp_line`, `compact_state`

## Help and reserved keys

Adding a keyboard shortcut means updating **all three**:

1. Footer help — `footer_text` and its helpers in `src/tui/views/panes.rs`
2. Help overlay — `src/tui/views/help.rs` (structured per pane and per level)
3. `RESERVED_KEYS` in `src/config.rs`, if the key fires anywhere a custom
   shortcut could — otherwise a user's shortcut is silently shadowed

## Tests

Render functions are unit-tested with `src/tui/views/test_util.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::views::test_util::{contains_line, render_to_lines};

    #[test]
    fn test_new_level_renders_its_rows() {
        let lines = render_to_lines(60, 12, |frame| {
            render_projects_pane(frame, frame.size(), &state, &store, &sessions)
        });
        assert!(contains_line(&lines, "Expected row"));
    }

    /// Every pane renderer owes this one
    #[test]
    fn test_no_row_renders_past_the_pane_width() {
        for width in [10_u16, 22, 30, 44] {
            let lines = render_to_lines(width, 12, |frame| { /* ... */ });
            for line in &lines {
                assert!(line.chars().count() <= width as usize, "{line:?}");
            }
        }
    }
}
```

## Verification

1. `cargo build`
2. `cargo lint` (clippy with `-D warnings`)
3. `cargo test`
4. Run the app and check:
   - it renders at 80, 100, 140 and 200 columns, and while `Tab` is held
   - nothing spills past a pane border at any width
   - `Esc` pops one level and does nothing at the root
   - the footer and `?` overlay both show the new keys
