# Release Skill

Prepare a new release by analyzing changes, suggesting a semantic version bump, and updating version files.

## Semantic Versioning Rules

### MAJOR (x.0.0) - Breaking Changes
Bump major version when changes are NOT backwards compatible:
- Removed or renamed public APIs
- Changed configuration format that breaks existing configs
- Removed features that users depend on
- Incompatible CLI changes (renamed flags, changed behavior)
- Changed default behavior in breaking ways

### MINOR (0.x.0) - New Features
Bump minor version for backwards-compatible additions:
- New features or functionality
- New configuration options (with defaults)
- New keyboard shortcuts
- New views or modes
- New CLI flags (existing behavior unchanged)
- Deprecation notices (not removals)

### PATCH (0.0.x) - Bug Fixes
Bump patch version for backwards-compatible fixes:
- Bug fixes
- Documentation updates
- Performance improvements
- Internal refactoring (no user-facing changes)
- Dependency updates (non-breaking)

## Analysis Steps

### 1. Find the Last Release Tag
```bash
git tag --list 'v*' | sort -V | tail -1
```

### 2. Get Current Version
Read from `Cargo.toml` line 7:
```toml
version = "x.y.z"
```

### 3. List Commits Since Last Release
```bash
git log v0.x.x..HEAD --oneline
```

### 4. Categorize Changes
Look for keywords in commit messages:

| Category | Keywords |
|----------|----------|
| Breaking | "BREAKING", "remove", "rename API", "incompatible", "breaking change" |
| Feature | "Add", "new feature", "implement", "support" |
| Fix | "Fix", "bug", "patch", "correct", "resolve" |
| Docs | "docs", "documentation", "README", "CHANGELOG" |
| Refactor | "refactor", "cleanup", "reorganize" |

### 5. Determine Version Bump
- If ANY breaking change: **MAJOR**
- Else if ANY new feature: **MINOR**
- Else: **PATCH**

## Files to Update

| File | Location | What to Change |
|------|----------|----------------|
| `Cargo.toml` | Line 7 | `version = "x.y.z"` |
| `CHANGELOG.md` | After `## [Unreleased]` | Add new version section |
| `CHANGELOG.md` | Bottom links | Update comparison URLs |

## CHANGELOG Format

The changelog follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format.

### New Version Section Template
Insert after `## [Unreleased]`:

```markdown
## [x.y.z] - YYYY-MM-DD

### Added
- New feature descriptions

### Changed
- Behavior changes

### Fixed
- Bug fix descriptions

### Removed
- Removed features (indicates breaking change)
```

Only include sections that have entries. Common sections in order:
1. Added - New features
2. Changed - Changes in existing functionality
3. Deprecated - Soon-to-be removed features
4. Removed - Removed features
5. Fixed - Bug fixes
6. Security - Security vulnerability fixes

### Link Format
At the bottom of CHANGELOG.md, update the links:

```markdown
[Unreleased]: https://github.com/ivan-brko/panoptes/compare/vX.Y.Z...HEAD
[X.Y.Z]: https://github.com/ivan-brko/panoptes/compare/vPREV...vX.Y.Z
```

Where:
- `X.Y.Z` is the NEW version
- `PREV` is the PREVIOUS version

## Example Workflow

### Before (v0.2.2 is current)
```markdown
## [Unreleased]

## [0.2.2] - 2025-01-29
...

[Unreleased]: https://github.com/ivan-brko/panoptes/compare/v0.2.2...HEAD
[0.2.2]: https://github.com/ivan-brko/panoptes/compare/v0.2.1...v0.2.2
```

### After (releasing v0.2.3)
```markdown
## [Unreleased]

## [0.2.3] - 2025-01-30

### Fixed
- Fixed session disconnect on rapid input

## [0.2.2] - 2025-01-29
...

[Unreleased]: https://github.com/ivan-brko/panoptes/compare/v0.2.3...HEAD
[0.2.3]: https://github.com/ivan-brko/panoptes/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/ivan-brko/panoptes/compare/v0.2.1...v0.2.2
```

## Verification Checklist

After making changes, verify:

### 1. Code Quality
```bash
cargo fmt --check && cargo clippy -- -D warnings && cargo test
```

### 2. Version Consistency
- [ ] Version in `Cargo.toml` matches version in `CHANGELOG.md`
- [ ] New version section date is correct (YYYY-MM-DD format)
- [ ] CHANGELOG entries match actual commits

### 3. Links
- [ ] `[Unreleased]` link points to `compare/vNEW...HEAD`
- [ ] New version link points to `compare/vPREV...vNEW`
- [ ] Previous version links unchanged

### 4. Content
- [ ] All significant changes documented
- [ ] Breaking changes clearly marked
- [ ] No duplicate entries
- [ ] Entries are user-facing (not internal implementation details)

## Quick Reference

```bash
# Get last tag
git tag --list 'v*' | sort -V | tail -1

# See commits since last release
git log $(git tag --list 'v*' | sort -V | tail -1)..HEAD --oneline

# See detailed changes
git log $(git tag --list 'v*' | sort -V | tail -1)..HEAD --pretty=format:"- %s"
```

## After Release Preparation

Once files are updated:
1. Commit changes: `git commit -am "Release vX.Y.Z"`
2. Create tag: `git tag vX.Y.Z`
3. Push: `git push && git push --tags`
4. Create GitHub release (optional)
