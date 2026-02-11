---
name: sync-agent-docs
description: >
  Verify CLAUDE.md and AGENTS.md are aligned on shared sections.
  Compare both files, determine which is correct by checking the
  codebase, and update the stale file to match.
---

# Sync Agent Docs

Ensure CLAUDE.md and AGENTS.md stay in sync. These files share project
instructions that both Claude Code and OpenAI Codex need. When one gets
updated, the other may fall behind.

## When to Run

Run this skill after:
- Completing a feature that changed project conventions
- Adding/removing modules
- Changing build, test, or lint commands
- Modifying error handling patterns or coding conventions
- Any change that touched CLAUDE.md or AGENTS.md directly

## Section Mapping

These sections contain shared content and must stay aligned. The header
names may differ slightly between files:

| CLAUDE.md | AGENTS.md | Content |
|-----------|-----------|---------|
| `## Build & Test` | `## Commands` | Build/test/lint/run commands |
| `## Linting` | `## Linting` | Clippy patterns |
| `## Architecture` | `## Architecture` | Project description, data flow, key modules |
| `## Conventions` | `## Coding Conventions` | Code style rules |
| `## Error Handling` | `## Error Handling` | Error handling patterns |
| `## Documentation` | `## Documentation` | Doc file references |

### Sections that exist only in one file (no sync needed)

- **AGENTS.md only**: `## Tech Stack`, `## Git Conventions`, `## Testing Requirements`
- **CLAUDE.md only**: Claude-specific intro paragraph, sync note comment

These are intentionally different and should NOT be merged.

## Steps

### 1. Read Both Files

Read `CLAUDE.md` and `AGENTS.md` from the project root.

### 2. Extract and Compare Shared Sections

For each mapped section pair, extract the content (everything between
the section header and the next `##` header at the same level) and
compare. Ignore the header line itself since they intentionally differ.

Report each section as one of:
- **IN SYNC** — content matches
- **DRIFTED** — content differs (show the diff)
- **MISSING** — section exists in one file but not the other

### 3. Determine Which Is Correct

For each drifted section, verify against the actual codebase:

**Build commands** (`## Build & Test` / `## Commands`):
- Check that listed commands actually work (e.g., `cargo build`, `cargo test`)
- Check Cargo.toml exists and is valid

**Linting**:
- Verify listed clippy patterns are still relevant
- Check if any new common patterns should be added

**Architecture — Key Modules**:
- For each listed module, verify the directory/file actually exists:
  ```bash
  ls -d src/app/ src/agent/ src/session/ src/hooks/ src/input/ src/tui/ src/project/ src/claude_config/ src/codex_config/ src/config.rs 2>&1
  ```
- Check for new top-level modules not yet documented:
  ```bash
  ls -d src/*/ src/*.rs | grep -v mod.rs | grep -v main.rs | grep -v lib.rs
  ```

**Conventions / Coding Conventions**:
- Check that listed patterns exist in code (e.g., `anyhow::Result` usage)
- Verify keyboard shortcut update rules still apply

**Error Handling**:
- Spot-check that error handling patterns match actual usage

**Documentation**:
- Verify listed doc files actually exist:
  ```bash
  ls docs/PRODUCT.md docs/TECHNICAL.md docs/CONFIG_GUIDE.md 2>&1
  ```

### 4. Apply Fixes

For each drifted section:

1. If **CLAUDE.md is correct** (matches codebase): update AGENTS.md
2. If **AGENTS.md is correct** (matches codebase): update CLAUDE.md
3. If **neither is fully correct**: update BOTH to match the codebase
4. If a **new module or convention** was found: add it to BOTH files

When updating, preserve each file's own formatting:
- CLAUDE.md uses `## Build & Test`, `## Conventions`
- AGENTS.md uses `## Commands`, `## Coding Conventions`
- AGENTS.md has the `## Tech Stack` section that CLAUDE.md does not — leave it alone
- AGENTS.md has `## Git Conventions` and `## Testing Requirements` — leave them alone
  unless a relevant convention was added to CLAUDE.md's `## Conventions` section

### 5. Verify Sync Note

Confirm CLAUDE.md still has the sync reminder comment at the top:
```markdown
<!-- NOTE: Core project instructions are duplicated in AGENTS.md for Codex compatibility. -->
<!-- When updating shared rules (stack, commands, architecture, conventions), update BOTH files. -->
```

Confirm AGENTS.md still has the sync reminder comment at the top:
```markdown
<!-- SHARED INSTRUCTIONS — keep in sync with CLAUDE.md -->
```

## Example Report

```
Agent Docs Sync Report:

  Build commands:     IN SYNC
  Linting:            IN SYNC
  Architecture:       DRIFTED
    - CLAUDE.md lists `codex_config/` module
    - AGENTS.md is missing `codex_config/` module
    → Fixed: updated AGENTS.md to include codex_config/
  Conventions:        IN SYNC
  Error Handling:     IN SYNC
  Documentation:      DRIFTED
    - AGENTS.md lists docs/API.md which does not exist
    → Fixed: removed docs/API.md from AGENTS.md
  Codebase check:
    - Found new module `src/themes/` not listed in either file
    → Fixed: added to both CLAUDE.md and AGENTS.md

Result: 2 sections fixed, 4 in sync, 1 new module added.
```

## Important

- **Never remove AGENTS.md-only sections** (Tech Stack, Git Conventions,
  Testing Requirements) — these provide extra context for Codex.
- **Never remove CLAUDE.md-only content** (the intro paragraph, Claude-specific
  phrasing) — that's intentionally Claude-specific.
- **The codebase is the source of truth** — when both files disagree,
  check what the code actually does.
