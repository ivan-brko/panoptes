//! Path completion for directory navigation
//!
//! Provides shell-like path autocompletion for the project addition dialog.

use std::path::{Path, PathBuf};

/// Get directory completions for a partial path
///
/// # Arguments
/// * `partial` - The partial path typed by the user
///
/// # Returns
/// A vector of matching directory paths, sorted alphabetically
pub fn get_completions(partial: &str) -> Vec<PathBuf> {
    if partial.is_empty() {
        return Vec::new();
    }

    // Expand tilde to home directory
    let expanded = shellexpand::tilde(partial);
    let expanded_path = Path::new(expanded.as_ref());

    // Determine the parent directory and prefix to match
    let (parent_dir, prefix) = if expanded_path.to_string_lossy().ends_with('/') {
        // User typed a complete directory path ending with /
        // List contents of that directory
        (expanded_path.to_path_buf(), String::new())
    } else if expanded_path.exists() && expanded_path.is_dir() {
        // User typed a directory name without trailing slash
        // Could be completing or could want to enter it
        // Return this directory plus its contents
        let mut results = Vec::new();
        // First, add the directory itself if it looks like a completion target
        results.push(expanded_path.to_path_buf());
        // Then add its contents
        if let Ok(entries) = std::fs::read_dir(expanded_path) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.is_dir() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    // Hide dotfiles unless we're looking for them
                    if !name_str.starts_with('.') {
                        results.push(path);
                    }
                }
            }
        }
        results.sort();
        return results;
    } else {
        // User is typing a partial name - get parent and prefix
        let parent = expanded_path.parent().map(|p| p.to_path_buf());
        let file_name = expanded_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        match parent {
            Some(p) if p.as_os_str().is_empty() => {
                // Relative path with no parent (e.g., "foo")
                (PathBuf::from("."), file_name)
            }
            Some(p) => (p, file_name),
            None => return Vec::new(),
        }
    };

    // Read the parent directory and filter
    let show_hidden = prefix.starts_with('.');
    let prefix_lower = prefix.to_lowercase();

    let mut results: Vec<PathBuf> = match std::fs::read_dir(&parent_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|entry| {
                let path = entry.path();
                if !path.is_dir() {
                    return false;
                }
                let name = entry.file_name();
                let name_str = name.to_string_lossy();

                // Hide dotfiles unless prefix starts with .
                if name_str.starts_with('.') && !show_hidden {
                    return false;
                }

                // Case-insensitive prefix matching
                name_str.to_lowercase().starts_with(&prefix_lower)
            })
            .map(|entry| entry.path())
            .collect(),
        Err(_) => Vec::new(),
    };

    results.sort();
    results
}

/// Convert a path to a display string with ~ for home directory
pub fn path_to_display(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(stripped) = path.strip_prefix(&home) {
            return format!("~/{}", stripped.display());
        }
    }
    path.display().to_string()
}

/// Convert a path back to a string suitable for the input field
/// Includes trailing slash for directories
pub fn path_to_input(path: &Path) -> String {
    let display = path_to_display(path);
    if path.is_dir() && !display.ends_with('/') {
        format!("{}/", display)
    } else {
        display
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_empty_partial() {
        let results = get_completions("");
        assert!(results.is_empty());
    }

    #[test]
    fn test_home_expansion() {
        // This test just verifies ~ expansion doesn't panic
        let results = get_completions("~/");
        // Should return directories in home, may be empty in test env
        assert!(results.iter().all(|p| p.is_dir() || !p.exists()));
    }

    #[test]
    fn test_completions_in_temp_dir() {
        let temp = tempdir().unwrap();
        let base = temp.path();

        // Create test directories
        fs::create_dir(base.join("alpha")).unwrap();
        fs::create_dir(base.join("beta")).unwrap();
        fs::create_dir(base.join("alphabet")).unwrap();
        fs::create_dir(base.join(".hidden")).unwrap();

        // Create a file (should not appear)
        fs::write(base.join("file.txt"), "test").unwrap();

        // Test listing all
        let partial = format!("{}/", base.display());
        let results = get_completions(&partial);
        assert_eq!(results.len(), 3); // alpha, alphabet, beta (not .hidden or file.txt)

        // Test prefix matching
        let partial = format!("{}/al", base.display());
        let results = get_completions(&partial);
        assert_eq!(results.len(), 2); // alpha, alphabet

        // Test case-insensitive matching
        let partial = format!("{}/AL", base.display());
        let results = get_completions(&partial);
        assert_eq!(results.len(), 2); // alpha, alphabet

        // Test hidden file matching
        let partial = format!("{}/.h", base.display());
        let results = get_completions(&partial);
        assert_eq!(results.len(), 1); // .hidden
    }

    #[test]
    fn test_path_to_display() {
        if let Some(home) = dirs::home_dir() {
            let test_path = home.join("test").join("path");
            assert_eq!(path_to_display(&test_path), "~/test/path");
        }

        let abs_path = PathBuf::from("/usr/local/bin");
        assert_eq!(path_to_display(&abs_path), "/usr/local/bin");
    }

    #[test]
    fn test_path_to_input_adds_slash() {
        let temp = tempdir().unwrap();
        let result = path_to_input(temp.path());
        assert!(result.ends_with('/'));
    }
}
