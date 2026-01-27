# Add Tests Skill

Generate test scaffolding for untested modules in Panoptes.

## Steps

1. **Analyze the target module**
   - Read the source file
   - Identify public functions and their signatures
   - Note any complex logic or edge cases

2. **Determine testable components**
   - Pure functions (no side effects)
   - State transitions
   - Type conversions
   - Validation logic
   - Filter/search functions

3. **Generate test scaffolding**
   Add a `#[cfg(test)]` block at the end of the module:
   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;

       // Helper functions for creating test fixtures
       fn create_test_item() -> ItemType {
           // ...
       }

       #[test]
       fn test_function_name_basic() {
           // Arrange
           let input = create_test_item();

           // Act
           let result = function_name(input);

           // Assert
           assert_eq!(result, expected);
       }

       #[test]
       fn test_function_name_edge_case() {
           // Test edge cases
       }
   }
   ```

4. **Test categories to include**
   - **Happy path**: Normal expected usage
   - **Edge cases**: Empty inputs, boundaries, special values
   - **Error cases**: Invalid inputs, error conditions
   - **State transitions**: Before/after state changes

## Panoptes-Specific Patterns

### Testing State Navigation
```rust
#[test]
fn test_navigate_to_project() {
    let mut state = AppState::default();
    let project_id = uuid::Uuid::new_v4();

    state.navigate_to_project(project_id);

    assert_eq!(state.view, View::ProjectDetail(project_id));
    assert_eq!(state.selected_branch_index, 0);
}
```

### Testing Filters
```rust
#[test]
fn test_filter_empty_query() {
    let items = create_test_items();
    let filtered = filter_items(&items, "");
    assert_eq!(filtered.len(), items.len());
}

#[test]
fn test_filter_case_insensitive() {
    let items = create_test_items();
    let filtered = filter_items(&items, "MAIN");
    assert!(filtered.iter().any(|i| i.name == "main"));
}
```

### Testing Enums
```rust
#[test]
fn test_event_type_roundtrip() {
    for event_type in [EventType::A, EventType::B, EventType::C] {
        let str_repr = event_type.as_str();
        let parsed: EventType = str_repr.into();
        assert_eq!(parsed, event_type);
    }
}
```

## Common Dependencies

Tests often need:
```rust
use tempfile::TempDir;  // For filesystem tests
use uuid::Uuid;          // For ID generation
```

## Output

After running this skill, you should have:
1. A `#[cfg(test)]` module with tests
2. Tests for all public functions
3. Edge case coverage
4. Verifiable assertions (not just `assert!(true)`)
