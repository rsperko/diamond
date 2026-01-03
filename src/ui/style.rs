//! Style constants and color helpers.
//!
//! Centralizes all styling decisions for consistent output.

use colored::{ColoredString, Colorize};

// ──────────────────────────────────────────────────────────────
// Emoji markers (preserve existing conventions)
// ──────────────────────────────────────────────────────────────

/// Success marker: ✓
pub const MARK_SUCCESS: &str = "✓";
/// Error/failure marker: ✗
pub const MARK_ERROR: &str = "✗";
/// Warning marker: !
pub const MARK_WARNING: &str = "!";
/// Info marker: ℹ
pub const MARK_INFO: &str = "ℹ";
/// Progress/step marker: →
pub const MARK_STEP: &str = "→";
/// Skip marker: ⏭
pub const MARK_SKIP: &str = "⏭";
/// Bullet marker: •
pub const MARK_BULLET: &str = "•";

// ──────────────────────────────────────────────────────────────
// Spinner styles
// ──────────────────────────────────────────────────────────────

/// Braille spinner frames (dense, techy)
pub const SPINNER_FRAMES: &str = "⡀⡄⡆⡇⠇⠏⠋⠉";

// ──────────────────────────────────────────────────────────────
// Color helper functions
// ──────────────────────────────────────────────────────────────

/// Format text as success (green)
pub fn success_style<S: AsRef<str>>(s: S) -> ColoredString {
    s.as_ref().green()
}

/// Format text as error (red)
pub fn error_style<S: AsRef<str>>(s: S) -> ColoredString {
    s.as_ref().red()
}

/// Format text as warning (yellow)
pub fn warning_style<S: AsRef<str>>(s: S) -> ColoredString {
    s.as_ref().yellow()
}

/// Format text as info (blue)
pub fn info_style<S: AsRef<str>>(s: S) -> ColoredString {
    s.as_ref().blue()
}

/// Format branch name (green)
pub fn branch_style<S: AsRef<str>>(s: S) -> ColoredString {
    s.as_ref().green()
}

/// Format parent branch name (blue)
pub fn parent_style<S: AsRef<str>>(s: S) -> ColoredString {
    s.as_ref().blue()
}

/// Format command text (cyan)
pub fn cmd_style<S: AsRef<str>>(s: S) -> ColoredString {
    s.as_ref().cyan()
}

/// Format URL (cyan)
pub fn url_style<S: AsRef<str>>(s: S) -> ColoredString {
    s.as_ref().cyan()
}

/// Format count/number (yellow)
pub fn count_style<S: AsRef<str>>(s: S) -> ColoredString {
    s.as_ref().yellow()
}

/// Format subdued/secondary text (bright black/gray)
pub fn dim_style<S: AsRef<str>>(s: S) -> ColoredString {
    s.as_ref().bright_black()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_markers_are_single_chars() {
        // Markers should be single visible characters
        assert_eq!(MARK_SUCCESS.chars().count(), 1);
        assert_eq!(MARK_ERROR.chars().count(), 1);
        assert_eq!(MARK_WARNING.chars().count(), 1);
        assert_eq!(MARK_INFO.chars().count(), 1);
        assert_eq!(MARK_STEP.chars().count(), 1);
        assert_eq!(MARK_SKIP.chars().count(), 1);
        assert_eq!(MARK_BULLET.chars().count(), 1);
    }

    #[test]
    fn test_spinner_frames_not_empty() {
        assert!(!SPINNER_FRAMES.is_empty());
        // Should have multiple frames for animation
        assert!(SPINNER_FRAMES.chars().count() >= 4);
    }

    #[test]
    fn test_style_functions_work() {
        // Just verify they don't panic
        let _ = success_style("test");
        let _ = error_style("test");
        let _ = warning_style("test");
        let _ = info_style("test");
        let _ = branch_style("feature");
        let _ = parent_style("main");
        let _ = cmd_style("dm sync");
        let _ = url_style("https://example.com");
        let _ = count_style("42");
        let _ = dim_style("secondary");
    }
}
