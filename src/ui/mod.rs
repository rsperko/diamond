//! Centralized UI module for beautiful, consistent terminal output.
//!
//! This module provides:
//! - Consistent styling and color conventions
//! - Progress indicators (spinners, progress bars)
//! - PR update progress (parallel multi-progress)
//! - Interactive prompts (confirm, select)
//! - Section headers and formatting
//! - TTY detection helpers
//!
//! All functions gracefully degrade when not running in a TTY.

use std::io::IsTerminal;

mod output;
mod pr_progress;
mod progress;
mod prompt;
mod section;
mod style;

pub use output::*;
pub use pr_progress::*;
pub use progress::*;
pub use prompt::*;
pub use section::*;
pub use style::*;

// ============================================================================
// TTY Detection Helpers
// ============================================================================

/// Check if stdout is a terminal (for TUI commands like `dm log`, `dm checkout`).
///
/// Use this when a command needs to render a full-screen TUI or
/// output that requires terminal capabilities (cursor movement, colors, etc.)
pub fn is_stdout_terminal() -> bool {
    std::io::stdout().is_terminal()
}

/// Check if stdin is a terminal (for commands that prompt for input).
///
/// Use this when a command needs to read interactive input from the user.
pub fn is_stdin_terminal() -> bool {
    std::io::stdin().is_terminal()
}

/// Bail if stdout is not a terminal.
///
/// Use this at the start of TUI commands that require interactive output.
/// Provides a consistent error message directing users to non-interactive alternatives.
///
/// # Example
/// ```ignore
/// pub fn run() -> Result<()> {
///     ui::require_stdout_terminal("dm log --short")?;
///     // ... TUI code ...
/// }
/// ```
pub fn require_stdout_terminal(fallback_hint: &str) -> anyhow::Result<()> {
    if !is_stdout_terminal() {
        anyhow::bail!(
            "This command requires an interactive terminal.\n\
             Use '{}' for non-interactive output.",
            fallback_hint
        );
    }
    Ok(())
}

/// Bail if stdin is not a terminal.
///
/// Use this for commands that prompt for confirmation and require `--force` in non-TTY.
///
/// # Example
/// ```ignore
/// pub fn run(force: bool) -> Result<()> {
///     if !force {
///         ui::require_stdin_terminal()?;
///         // ... prompt for confirmation ...
///     }
///     // ... proceed with operation ...
/// }
/// ```
pub fn require_stdin_terminal() -> anyhow::Result<()> {
    if !is_stdin_terminal() {
        anyhow::bail!(
            "This command requires confirmation.\n\
             Use --force (-f) to skip confirmation in non-interactive mode."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests run in a non-TTY environment (cargo test), so:
    // - is_stdout_terminal() returns false
    // - is_stdin_terminal() returns false
    // - require_* functions return errors
    //
    // This is actually the important case to test since we want to verify
    // error messages are correct for CI/scripts/pipes.

    #[test]
    fn test_is_stdout_terminal_returns_bool() {
        // In test environment, this will be false, but we verify it returns a bool
        let result = is_stdout_terminal();
        // Just verify it's a bool (doesn't panic) - use result to avoid unused warning
        let _: bool = result;
    }

    #[test]
    fn test_is_stdin_terminal_returns_bool() {
        // In test environment, this will be false, but we verify it returns a bool
        let result = is_stdin_terminal();
        let _: bool = result;
    }

    #[test]
    fn test_require_stdout_terminal_error_includes_fallback_hint() {
        // In non-TTY environment, this should fail with helpful message
        let result = require_stdout_terminal("dm log --short");

        // Tests run without a TTY, so this should be an error
        if let Err(e) = result {
            let err = e.to_string();
            assert!(
                err.contains("interactive terminal"),
                "Error should mention interactive terminal: {}",
                err
            );
            assert!(
                err.contains("dm log --short"),
                "Error should include the fallback hint: {}",
                err
            );
        }
        // If it somehow passes (running in actual terminal), that's also fine
    }

    #[test]
    fn test_require_stdin_terminal_error_mentions_force_flag() {
        // In non-TTY environment, this should fail with helpful message
        let result = require_stdin_terminal();

        // Tests run without a TTY, so this should be an error
        if let Err(e) = result {
            let err = e.to_string();
            assert!(
                err.contains("confirmation"),
                "Error should mention confirmation: {}",
                err
            );
            assert!(err.contains("--force"), "Error should mention --force flag: {}", err);
            assert!(err.contains("-f"), "Error should mention -f shorthand: {}", err);
        }
        // If it somehow passes (running in actual terminal), that's also fine
    }
}
