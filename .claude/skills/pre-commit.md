# Pre-Commit Skill

Run all quality checks before committing code changes.

## Checks (in order)

### 1. Format Check
```bash
cargo fmt --check
```
Verify code is properly formatted. If this fails, run `cargo fmt` to fix.

### 2. Clippy Lint
```bash
cargo clippy -- -D warnings
```
Ensure no clippy warnings. Fix any issues before committing.

### 3. Run Tests
```bash
cargo test
```
Verify all tests pass. Do not commit if tests fail.

### 4. Check for TODOs/FIXMEs
```bash
git diff --cached | grep -E '^\+.*TODO|^\+.*FIXME' || true
```
Review any new TODO/FIXME comments. These should have associated issues.

### 5. Check for Secrets
```bash
# Check for potentially sensitive files
git diff --cached --name-only | grep -E '\.env|credentials|secret|key\.json' || true

# Check for API key patterns
git diff --cached | grep -E 'api[_-]?key|password|secret' || true
```
Never commit secrets. Use environment variables instead.

### 6. Check Staged Files
```bash
git status --short
```
Review what's being committed. Avoid committing:
- `.env` files
- Large binary files
- Generated files (build artifacts)
- Personal IDE settings

## Quick Pre-Commit Script

Run all checks in sequence:
```bash
cargo fmt --check && \
cargo clippy -- -D warnings && \
cargo test && \
echo "All checks passed!"
```

## Common Issues

| Check | Fix |
|-------|-----|
| Format failed | Run `cargo fmt` |
| Clippy warning | Fix the warning or `#[allow(...)]` with justification |
| Test failed | Fix the test or the code |
| Secret detected | Remove and use env var |

## Example Output

```
Pre-Commit Checks:

[1/5] Format check... OK
[2/5] Clippy... OK (0 warnings)
[3/5] Tests... OK (261 passed)
[4/5] TODO/FIXME check... 0 new comments
[5/5] Secrets check... OK

All checks passed! Ready to commit.
```

## If Checks Fail

1. **Do not** use `--no-verify` to skip checks
2. Fix the issues
3. Re-run checks
4. Only commit when all checks pass

## Commit Message Guidelines

After passing all checks, create a descriptive commit:
- Start with verb: Add, Fix, Update, Remove, Refactor
- Keep first line under 72 characters
- Add body for complex changes
- Reference issues if applicable
