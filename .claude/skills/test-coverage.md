# Test Coverage Skill

Analyze test coverage and identify modules needing tests.

## Steps

1. **Run all tests and collect results**
   ```bash
   cargo test 2>&1
   ```
   Note total test count and any failures.

2. **Identify modules without tests**
   Search for Rust source files and check for `#[cfg(test)]` blocks:
   ```bash
   # Find files without test modules
   for f in src/**/*.rs; do
     if ! grep -q '#\[cfg(test)\]' "$f" 2>/dev/null; then
       echo "$f"
     fi
   done
   ```

3. **Analyze untested modules**
   For each untested module, identify:
   - Public functions that should have unit tests
   - Complex logic that would benefit from testing
   - Edge cases in the implementation

4. **Generate coverage report**
   Report modules by priority:
   - **Critical**: Core logic (session management, state transitions)
   - **High**: Input handling, navigation logic
   - **Medium**: Utility functions, helpers
   - **Low**: Simple structs, trivial implementations

## Coverage Categories

### Modules that SHOULD have tests
- State management (`app/state.rs`, `session/mod.rs`)
- Business logic (filters, calculations, validations)
- Type conversions and parsing
- Navigation and routing logic

### Modules that MAY skip tests
- Pure TUI rendering (requires integration testing)
- PTY interaction (requires real terminal)
- Simple re-exports and type aliases

## Example Report

```
Test Coverage Report:

Total tests: 261
Modules with tests: 26/42 (62%)

High Priority Gaps:
- src/input/dispatcher.rs - No direct tests (routes to handlers)
- src/input/session_mode.rs - No tests (PTY interaction)

Medium Priority Gaps:
- src/tui/views/*.rs - Rendering modules (integration tests recommended)

Coverage by Area:
- Core (app, session, hooks): 80%
- Input handling: 40%
- TUI rendering: 20%
- Utilities: 90%
```

## Suggestions

After running this skill, consider:
1. Adding unit tests to high-priority gaps
2. Creating integration test scaffolding for TUI tests
3. Adding property-based tests for parsing/filtering logic
