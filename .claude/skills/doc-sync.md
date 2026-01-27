# Doc Sync Skill

Verify that documentation matches the actual code implementation.

## Areas to Check

### 1. Keyboard Shortcuts

**Files to compare:**
- `README.md` - Quick reference section
- `docs/KEYBOARD_REFERENCE.md` - Complete reference
- `src/input/normal/*.rs` - Actual implementations

**Verification:**
1. Extract shortcuts from README.md keyboard tables
2. Extract shortcuts from KEYBOARD_REFERENCE.md
3. Search for `KeyCode::Char('x')` patterns in input handlers
4. Report any discrepancies

**Common issues:**
- README shows `'a'` for adding projects, but code uses `'n'`
- Missing shortcuts in documentation
- Deprecated shortcuts still documented

### 2. Configuration Options

**Files to compare:**
- `docs/CONFIG_GUIDE.md` - Config documentation
- `src/config.rs` - Config struct and defaults

**Verification:**
1. Extract config fields from `Config` struct
2. Check each field is documented in CONFIG_GUIDE.md
3. Verify default values match
4. Check for undocumented fields

### 3. Module Descriptions

**Files to compare:**
- `CLAUDE.md` - Module responsibilities section
- `src/*/mod.rs` - Module doc comments

**Verification:**
1. List all modules in CLAUDE.md
2. Check each module exists
3. Compare descriptions with module `//!` comments
4. Report missing or outdated entries

### 4. Key Types

**Files to compare:**
- `CLAUDE.md` - Key Types section
- Source files with type definitions

**Verification:**
1. List types in CLAUDE.md
2. Verify each type exists in the codebase
3. Check descriptions are accurate
4. Report missing important types

## Example Report

```
Documentation Sync Report:

README.md vs KEYBOARD_REFERENCE.md:
  [MISMATCH] README: 'a' = Add project
             REFERENCE: 'n' = Add project
  [OK] All other shortcuts match

CONFIG_GUIDE.md vs config.rs:
  [OK] All 12 config options documented
  [OK] Default values match

CLAUDE.md vs codebase:
  [MISSING] hooks/mod.rs HookEventType not documented in Key Types
  [OK] Module responsibilities up to date
```

## Quick Checks

Run these to spot-check documentation:

```bash
# Find all KeyCode patterns in input handlers
grep -r "KeyCode::Char" src/input/ | grep -v test

# List config fields
grep "pub " src/config.rs | head -20

# Check module doc comments
head -5 src/*/mod.rs
```

## Fixes Applied

After running this skill, fix discrepancies by:
1. Updating README.md for quick fixes
2. Updating KEYBOARD_REFERENCE.md for complete changes
3. Updating CLAUDE.md for architecture documentation
4. Adding missing types to Key Types section
