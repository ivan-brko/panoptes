# Panoptes Implementation Phases

## Phase 1: Core Foundation

**Goal**: Build a working prototype that can spawn and interact with multiple Claude Code sessions.

### What We're Building

- **Project scaffolding** - Cargo workspace, module structure, dependencies
- **PTY management** - Spawn Claude Code in pseudo-terminals, handle input/output
- **Hook integration** - HTTP server to receive state updates from Claude Code
- **Basic TUI** - Single-pane session view, status bar, session switching
- **Session lifecycle** - Create, destroy, and switch between sessions

### Why This Phase First

This phase proves the core concept works. The two riskiest technical components are:
1. PTY handling - Can we properly spawn and communicate with Claude Code?
2. Hook integration - Can we receive real-time state updates?

By tackling these first, we validate the architecture before building the full UI.

### Deliverable

A working application where you can:
- Create 2-3 Claude Code sessions
- Switch between them with keyboard shortcuts
- See real-time state updates (Starting → Thinking → Executing → Waiting)

---

## Phase 2: Multi-Project & Git Integration

**Goal**: Full project and branch hierarchy with git worktree support.

### What We're Building

- **Project model** - Data structures for projects, branches, and their relationships
- **Git worktree support** - Create isolated working directories for each branch
- **Projects overview screen** - Grid view of all projects with session counts
- **Project detail view** - Branches and sessions within a project
- **Activity timeline** - All sessions sorted by recent activity

### Why This Phase Second

Once we have working sessions, users need a way to organize them. The three-screen navigation (Overview → Project → Session) provides the mental model for managing many sessions across multiple codebases.

Git worktrees are essential for branch isolation - without them, sessions on different branches would conflict when modifying files.

### Deliverable

A fully navigable application where you can:
- Add git repositories as projects
- Create sessions on specific branches
- Navigate between overview, project, and session views
- Have automatic worktree creation for branch isolation

---

## Phase 3: Polish & Robustness

**Goal**: Production-quality user experience and error handling.

### What We're Building

- **Visual polish** - Color scheme, borders, visual hierarchy
- **Notifications** - Alerts when sessions need attention
- **Configuration** - TOML config file, customizable settings
- **Error handling** - Graceful recovery, helpful error messages
- **Performance tuning** - Handle 20+ sessions smoothly

### Why This Phase Third

Functionality before polish. Phase 1 and 2 build the features; Phase 3 makes them pleasant to use. This includes handling edge cases (what if Claude Code crashes?), giving visual feedback, and ensuring performance at scale.

### Deliverable

A release candidate that:
- Looks professional and consistent
- Handles errors gracefully
- Stays responsive with many sessions
- Can be configured to user preferences

---

## Phase 4: Documentation & Distribution

**Goal**: Make Panoptes easy to install and use.

### What We're Building

- **README** - Quick start guide, feature overview
- **Keyboard reference** - Complete shortcut documentation
- **Configuration guide** - All available settings explained
- **Troubleshooting** - Common issues and solutions
- **Distribution** - cargo install, Homebrew formula

### Why This Phase Last

Documentation and packaging come after the product is stable. Writing docs for a moving target creates maintenance burden. Once Phase 3 is complete, we document what exists.

### Deliverable

Version 1.0 release with:
- Comprehensive documentation
- Easy installation via cargo or Homebrew
- Clear upgrade path for future versions

---

## Summary

| Phase | Focus | Key Risk |
|-------|-------|----------|
| 1 | Core prototype | PTY and hook integration |
| 2 | Organization | Git worktree edge cases |
| 3 | Quality | Performance at scale |
| 4 | Release | User onboarding |

Each phase builds on the previous one. We validate risky technical assumptions early, then layer on organization, polish, and finally documentation.
