//! Platform-specific utilities for Diamond.
//!
//! This module contains platform-specific functionality, primarily for handling
//! cross-platform path display differences between Windows and Unix systems.

use std::fmt::{Display, Formatter, Result as FmtResult};
use std::path::Path;

/// A wrapper around `&Path` that displays paths in a user-friendly format.
///
/// On Windows, canonicalized paths include a `\\?\` prefix for verbatim path handling.
/// This wrapper strips that prefix when displaying paths to users, making error messages
/// and output more readable.
///
/// On Unix systems, this is equivalent to calling `.display()` on the path.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use diamond_cli::platform::DisplayPath;
///
/// let path = Path::new(r"\\?\C:\Users\test\file.txt");
/// println!("Path: {}", DisplayPath(path));
/// // Output on Windows: "Path: C:\Users\test\file.txt"
/// // The \\?\ prefix is stripped for readability
/// ```
///
/// This type integrates seamlessly with all formatting macros:
///
/// ```
/// # use std::path::Path;
/// # use diamond_cli::platform::DisplayPath;
/// let path = Path::new("/tmp/test.txt");
///
/// // Works with println!, format!, write!, etc.
/// println!("File: {}", DisplayPath(path));
/// let message = format!("Processing {}", DisplayPath(path));
/// ```
pub struct DisplayPath<'a>(pub &'a Path);

impl<'a> Display for DisplayPath<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let path_str = self.0.to_string_lossy();

        // Strip Windows UNC verbatim prefix if present
        // On Windows, canonicalize() adds \\?\ prefix for extended-length path support
        // This is correct for filesystem operations but not user-friendly in output
        if cfg!(windows) && path_str.starts_with(r"\\?\") {
            write!(f, "{}", &path_str[4..])
        } else {
            write!(f, "{}", path_str)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_display_path_strips_windows_unc_prefix() {
        // On Windows, canonicalized paths have \\?\ prefix which should be stripped
        // On Unix, paths should pass through unchanged

        if cfg!(windows) {
            // Simulate a Windows UNC path
            let path = Path::new(r"\\?\C:\Users\test\worktree");
            let displayed = format!("{}", DisplayPath(path));
            assert_eq!(displayed, r"C:\Users\test\worktree");

            // Regular Windows path without prefix should pass through
            let path = Path::new(r"C:\Users\test\worktree");
            let displayed = format!("{}", DisplayPath(path));
            assert_eq!(displayed, r"C:\Users\test\worktree");
        } else {
            // Unix paths should pass through unchanged
            let path = Path::new("/tmp/worktree");
            let displayed = format!("{}", DisplayPath(path));
            assert_eq!(displayed, "/tmp/worktree");
        }
    }

    #[test]
    fn test_display_path_edge_cases() {
        if cfg!(windows) {
            // Network UNC path (not verbatim) should pass through
            let path = Path::new(r"\\server\share\worktree");
            let displayed = format!("{}", DisplayPath(path));
            assert_eq!(displayed, r"\\server\share\worktree");

            // Root path with UNC prefix
            let path = Path::new(r"\\?\C:\");
            let displayed = format!("{}", DisplayPath(path));
            assert_eq!(displayed, r"C:\");

            // Deep nested path with UNC prefix
            let path = Path::new(r"\\?\C:\very\deep\nested\path\to\worktree");
            let displayed = format!("{}", DisplayPath(path));
            assert_eq!(displayed, r"C:\very\deep\nested\path\to\worktree");
        } else {
            // Root path on Unix
            let path = Path::new("/");
            let displayed = format!("{}", DisplayPath(path));
            assert_eq!(displayed, "/");

            // Deep nested Unix path
            let path = Path::new("/very/deep/nested/path/to/worktree");
            let displayed = format!("{}", DisplayPath(path));
            assert_eq!(displayed, "/very/deep/nested/path/to/worktree");
        }
    }

    #[test]
    fn test_display_path_works_in_format_macro() {
        let path = Path::new("/test/path");

        // Should work seamlessly in format!
        let result = format!("Processing: {}", DisplayPath(path));
        assert!(result.contains("/test/path") || result.contains(r"\test\path"));

        // Should work in multiple format parameters
        let result = format!("From {} to {}", DisplayPath(path), DisplayPath(path));
        assert!(result.contains("From") && result.contains("to"));
    }

    #[test]
    fn test_display_path_works_with_pathbuf() {
        use std::path::PathBuf;

        // Should work with PathBuf references
        let path_buf = PathBuf::from("/test/path");
        let result = format!("{}", DisplayPath(&path_buf));
        assert!(result.contains("/test/path") || result.contains(r"\test\path"));
    }

    #[test]
    fn test_display_path_with_nonexistent_path() {
        // DisplayPath should work even if the path doesn't exist
        let path = Path::new("/nonexistent/path/to/nowhere");
        let result = format!("{}", DisplayPath(path));
        assert!(result.contains("nonexistent"));
    }

    #[test]
    fn test_display_path_with_anyhow_error() {
        use anyhow::bail;

        // Should work with anyhow::bail! (common usage pattern)
        let path = Path::new("/some/path");
        let result: Result<(), anyhow::Error> = (|| {
            bail!("Failed to process {}", DisplayPath(path));
        })();

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to process"));
        assert!(err_msg.contains("some") && err_msg.contains("path"));
    }

    #[test]
    fn test_display_path_with_unicode() {
        // Should handle unicode paths correctly
        let path = Path::new("/tmp/файл/文件/ファイル");
        let result = format!("{}", DisplayPath(path));
        // Just verify it doesn't panic - exact output depends on platform
        assert!(result.contains("tmp") || result.contains(r"\tmp"));
    }
}
