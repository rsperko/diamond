//! Program name detection for argv[0] support
//!
//! This module provides a function to get the program name as invoked,
//! allowing Diamond to respect symlinks. For example, if `sc` is symlinked
//! to `dm`, running `sc --help` will show `sc` in the help text.

#[cfg(not(test))]
use std::sync::OnceLock;

#[cfg(not(test))]
static PROGRAM_NAME: OnceLock<String> = OnceLock::new();

/// Get the program name as invoked (respects symlinks)
///
/// Returns the basename of argv[0], falling back to "dm" if unavailable.
/// The value is memoized on first call. In test mode, always returns "dm".
///
/// # Examples
///
/// - Invoked as `dm` → returns `"dm"`
/// - Invoked as `/usr/local/bin/dm` → returns `"dm"`
/// - Invoked via symlink `sc` → returns `"sc"`
pub fn program_name() -> &'static str {
    #[cfg(test)]
    {
        "dm"
    }

    #[cfg(not(test))]
    {
        PROGRAM_NAME.get_or_init(|| {
            std::env::args()
                .next()
                .and_then(|s| {
                    std::path::Path::new(&s)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(String::from)
                })
                .unwrap_or_else(|| "dm".to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_program_name_returns_dm_in_tests() {
        let name = program_name();
        assert_eq!(name, "dm");
    }

    #[test]
    fn test_program_name_is_consistent() {
        // Multiple calls should return the same value
        let name1 = program_name();
        let name2 = program_name();
        assert_eq!(name1, name2);
    }
}
