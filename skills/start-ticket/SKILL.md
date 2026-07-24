---
name: start-ticket
description: >
  End-to-end workflow for implementing the Skope ticket that names the current
  branch: find it, move it to In Progress, implement it, have an independent
  agent review the code, rebase onto origin/main, move the ticket to Done, and
  open the PR. Use when asked to "work on this branch's ticket", "start the
  ticket", or any request that begins with finding a ticket for the branch.
---

# Start Ticket Skill

One pass, seven steps, in order. Do not skip a step and do not reorder them —
the review happens *before* the rebase, and the ticket only moves to Done once
the PR is open.

## 0. Locate the ticket

The branch name carries the ticket key: `pan-14-left-right-arrows-...` → `PAN-14`.
Uppercase the leading token before the first number.

```
git rev-parse --abbrev-ref HEAD
```

Skope coordinates for this repo (`Smaller Projects` → `Panoptes`):

| Thing | ID |
|---|---|
| Workspace | `sk://workspace/3e182c96-5fc7-44b8-94e9-c04cc260cffa` |
| Project | `sk://project/33da3221-c5e1-4566-8aac-3531b9ef11c4` (key `PAN`) |
| Board | `1offs` — `sk://board/626f096d-2e36-4006-a0da-5832b72fd731` |
| To Do | `sk://column/95ad9749-7b94-4e47-a1e8-b53f85039bdd` |
| In Progress | `sk://column/c4aac6af-1014-4a46-9809-1ed116b39fd3` |
| Done | `sk://column/ae155c39-59dd-4fb0-96d5-a55f0e7a4473` |

Fetch it with `get_entity` — a ticket key works directly as the `entityId`, no
search needed. The response echoes the board's columns, so re-read them from
there rather than trusting the table above if they ever disagree.

If the branch does not encode a key, fall back to `search` in the workspace using
words from the branch name, and confirm the match with the user before moving
anything.

## 1. Move it to In Progress

```
move_ticket(workspaceId, ticketId: "PAN-14", columnId: <In Progress>)
```

Do this *before* writing code, not after. It is what makes the work visible.

## 2. Read the ticket as the spec

Panoptes tickets are written as full specs: an explicit **Scope** list of files,
a **Tests** list, and an **Out of scope** list. Treat all three as binding.

- Implement everything under Scope, including the doc and footer edits — they
  are not optional trailing chores.
- Write the tests the ticket asks for, by name where it names them.
- Do not implement anything under Out of scope, even if it looks like an
  obvious adjacent improvement. Note it for the user instead.

If the ticket's file:line references have drifted since it was written, trust
the code and say so in the summary.

Remember the repo rule from `CLAUDE.md`: a new keyboard shortcut touches
**three** places — the footer in `src/tui/views/panes.rs`, the help overlay in
`src/tui/views/help.rs`, and `RESERVED_KEYS` in `src/config.rs`. A ticket that
changes key bindings almost always has doc updates in `docs/` on top of that.

## 3. Implement, then gate

Commit in logical chunks as you go. Before considering the work done:

```bash
cargo fmt
cargo lint      # clippy --all-targets -- -D warnings
cargo test
```

All three must be clean. `cargo lint` treats warnings as errors, so code the
ticket orphans (a function whose only caller you deleted) must be deleted too,
not left behind.

## 4. Independent review

Spawn **one** agent with the `feature-dev:code-reviewer` type to review the
branch diff against `origin/main`. It must be a fresh agent — the point is a
context that has not already convinced itself the code is right.

Give it: the ticket's Ask and Scope, the diff to review, and the instruction to
report only findings that are actually wrong or actually violate the ticket,
not stylistic preferences.

Then judge the findings yourself:

- **Fix** anything that is a real bug, a missed Scope item, or a convention
  violation.
- **Skip** anything that is a preference, a rewrite of working code, or work the
  ticket explicitly put out of scope — and say in the final summary that you
  skipped it and why.

Re-run `cargo fmt && cargo lint && cargo test` after any fix.

## 5. Rebase onto origin/main

```bash
git fetch origin
git rebase origin/main
```

Re-run the full gate after the rebase — a clean rebase can still break a test.
If there are conflicts, resolve them; if they are non-trivial or you would be
guessing at intent, stop and ask.

## 6. Move the ticket to Done

```
move_ticket(workspaceId, ticketId: "PAN-14", columnId: <Done>)
```

## 7. Open the PR

```bash
git push -u origin <branch>
gh pr create --title "PAN-14: <ticket title>" --body "<body>"
```

The body should state what changed and why, list the tests added, and link the
ticket. End it with:

```
🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

## Report back

Close with: the ticket key and title, what was implemented, what the reviewer
found and what you did about each finding, the test results, and the PR URL.
State plainly if anything was left undone.
