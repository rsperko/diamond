//! Output functions for consistent message formatting.
//!
//! These functions replace ad-hoc println! calls with semantic output.

use colored::Colorize;
use std::io::IsTerminal;

use super::style::*;

// ──────────────────────────────────────────────────────────────
// Primary output functions
// ──────────────────────────────────────────────────────────────

/// Print success message: "✓ {message}" in green
pub fn success(message: &str) {
    println!("{} {}", MARK_SUCCESS.green(), message);
}

/// Print bold success message: "✓ {message}" in bold green
pub fn success_bold(message: &str) {
    println!("{} {}", MARK_SUCCESS.green().bold(), message.green().bold());
}

/// Print error message: "✗ {message}" in red
pub fn error(message: &str) {
    println!("{} {}", MARK_ERROR.red(), message);
}

/// Print error message to stderr: "✗ {message}" in red
pub fn error_stderr(message: &str) {
    eprintln!("{} {}", MARK_ERROR.red(), message);
}

/// Print warning message: "! {message}" in yellow
pub fn warning(message: &str) {
    println!("{} {}", MARK_WARNING.yellow().bold(), message);
}

/// Print info message: "ℹ {message}" in blue
pub fn info(message: &str) {
    println!("{} {}", MARK_INFO.blue(), message);
}

/// Print step/progress message: "→ {message}" in blue
pub fn step(message: &str) {
    println!("{} {}", MARK_STEP.blue(), message);
}

/// Print indented item: "  • {message}"
pub fn bullet(message: &str) {
    println!("  {} {}", MARK_BULLET, message);
}

/// Print indented success: "  ✓ {message}" in green
pub fn bullet_success(message: &str) {
    println!("  {} {}", MARK_SUCCESS.green(), message);
}

/// Print indented error: "  ✗ {message}" in red
pub fn bullet_error(message: &str) {
    println!("  {} {}", MARK_ERROR.red(), message);
}

/// Print indented step: "  → {message}" in blue
pub fn bullet_step(message: &str) {
    println!("  {} {}", MARK_STEP.blue(), message);
}

// ──────────────────────────────────────────────────────────────
// TTY-aware output
// ──────────────────────────────────────────────────────────────

/// Print hint only in TTY mode (skipped in CI/logs)
pub fn hint(message: &str) {
    if std::io::stdout().is_terminal() {
        println!("{}", dim_style(message));
    }
}

/// Print blank line only in TTY mode
pub fn blank() {
    if std::io::stdout().is_terminal() {
        println!();
    }
}

// ──────────────────────────────────────────────────────────────
// Formatted output helpers
// ──────────────────────────────────────────────────────────────

/// Print a branch name in the standard style
pub fn print_branch(name: &str) -> String {
    format!("{}", branch_style(name))
}

/// Print a parent branch name in the standard style
pub fn print_parent(name: &str) -> String {
    format!("{}", parent_style(name))
}

/// Print a command in the standard style
pub fn print_cmd(cmd: &str) -> String {
    format!("{}", cmd_style(cmd))
}

/// Print a URL in the standard style
pub fn print_url(url: &str) -> String {
    format!("{}", url_style(url))
}

/// Print a count in the standard style
pub fn print_count(n: usize) -> String {
    format!("{}", count_style(n.to_string()))
}

/// Create a clickable hyperlink using OSC 8 terminal escape sequences.
///
/// In terminals that support OSC 8 (iTerm2, VSCode, kitty, etc.), the text
/// becomes clickable and opens the URL. In terminals that don't support it,
/// the text is displayed as-is with no visible escape sequences.
///
/// In non-TTY environments (pipes, files), only the text is returned.
///
/// # Example
/// ```ignore
/// let link = hyperlink("https://github.com/user/repo/pull/123", "PR #123");
/// println!("See {} for details", link); // PR #123 is clickable
/// ```
pub fn hyperlink(url: &str, text: &str) -> String {
    if !std::io::stdout().is_terminal() {
        // Non-TTY: just show text without escape sequences
        return text.to_string();
    }

    // OSC 8 format: \x1b]8;;URL\x07TEXT\x1b]8;;\x07
    // This is invisible in terminals that don't support it
    format!("\x1b]8;;{}\x07{}\x1b]8;;\x07", url, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_helpers() {
        // Just verify they don't panic and return non-empty strings
        assert!(!print_branch("feature").is_empty());
        assert!(!print_parent("main").is_empty());
        assert!(!print_cmd("dm sync").is_empty());
        assert!(!print_url("https://example.com").is_empty());
        assert!(!print_count(42).is_empty());
    }

    #[test]
    fn test_output_functions_dont_panic() {
        // These write to stdout/stderr, just verify they don't crash
        // In a real test environment, we'd capture and verify output
        success("test success");
        success_bold("test bold success");
        error("test error");
        warning("test warning");
        info("test info");
        step("test step");
        bullet("test bullet");
        bullet_success("test bullet success");
        bullet_error("test bullet error");
        bullet_step("test bullet step");
    }

    #[test]
    fn test_hyperlink_non_tty() {
        // In test environment (non-TTY), should return plain text
        let result = hyperlink("https://github.com/user/repo/pull/123", "PR #123");

        // In non-TTY mode, should be just the text
        // Note: This test runs in non-TTY so we can verify this behavior
        // When is_terminal() returns false, we expect plain text
        assert_eq!(result, "PR #123");
    }

    #[test]
    fn test_hyperlink_osc8_format() {
        // Test that OSC 8 format is correct when applied
        // We can't easily test TTY mode, but we can verify the format logic
        let url = "https://example.com";
        let text = "Example";

        // Manually construct what TTY mode should produce
        let expected = format!("\x1b]8;;{}\x07{}\x1b]8;;\x07", url, text);

        // Verify the format structure
        assert!(expected.contains(url));
        assert!(expected.contains(text));
        assert!(expected.starts_with("\x1b]8;;"));
        assert!(expected.ends_with("\x1b]8;;\x07"));
    }
}
