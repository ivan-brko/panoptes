# Clippy Fix Skill

Run clippy with automatic fixes and report code quality improvements.

## Steps

1. **Run clippy with auto-fix**
   ```bash
   cargo clippy --fix --allow-dirty 2>&1
   ```
   This will automatically fix issues that clippy knows how to fix safely.

2. **Check remaining warnings**
   ```bash
   cargo clippy 2>&1
   ```
   Report any warnings that couldn't be auto-fixed.

3. **Verify tests still pass**
   ```bash
   cargo test 2>&1
   ```
   Ensure auto-fixes didn't break anything.

4. **Report summary**
   - Number of files modified by auto-fix
   - Remaining warnings requiring manual attention
   - Test results after fixes

## Common Clippy Fixes

The following patterns are commonly flagged and auto-fixed:

| Pattern | Better Alternative |
|---------|-------------------|
| `x.map_or(false, \|v\| ...)` | `x.is_some_and(\|v\| ...)` |
| `container.len() >= 1` | `!container.is_empty()` |
| `!env.contains_key(x).is_none()` | `!env.contains_key(x)` |
| `&foo.to_string()` | `foo.to_string()` (remove needless borrow) |
| Single-element vec via clone | `std::slice::from_ref()` |

## Manual Fixes Often Required

These patterns need human judgment:

- **Cognitive complexity**: Break down large functions
- **Too many arguments**: Consider a struct parameter
- **Missing docs**: Add documentation for public items
- **Panic in library code**: Use Result instead of unwrap/expect

## Example Output

```
Clippy Fix Summary:
- Auto-fixed: 9 issues in 4 files
- Remaining warnings: 0
- Tests: 223 passed

Files modified:
- src/claude_json.rs (2 fixes)
- src/tui/views/projects.rs (2 fixes)
- src/tui/views/project_detail.rs (4 fixes)
- src/tui/views/claude_configs.rs (1 fix)
```
