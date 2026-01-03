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
}
