//! Interactive prompts using dialoguer.
//!
//! All functions gracefully handle non-TTY environments.

use anyhow::{bail, Result};
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, MultiSelect, Select};
use std::io::IsTerminal;

use crate::program_name::program_name;

// ──────────────────────────────────────────────────────────────
// Confirmation prompts
// ──────────────────────────────────────────────────────────────

/// Confirmation prompt with default value.
///
/// In non-TTY mode, returns an error asking for --force flag.
///
/// # Example
/// ```ignore
/// if !ui::confirm("Delete these branches?", false)? {
///     ui::warning("Cancelled");
///     return Ok(());
/// }
/// ```
pub fn confirm(message: &str, default: bool) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("This operation requires confirmation. Use --force to skip in non-interactive mode.");
    }

    let result = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(message)
        .default(default)
        .interact()?;

    Ok(result)
}

/// Confirmation prompt that defaults to the safe option in non-TTY.
///
/// Unlike `confirm()`, this doesn't error in non-TTY - it returns `false`.
/// Use for optional confirmations where skipping is acceptable.
pub fn confirm_optional(message: &str, default: bool) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        return Ok(false);
    }

    let result = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(message)
        .default(default)
        .interact()?;

    Ok(result)
}

// ──────────────────────────────────────────────────────────────
// Selection prompts
// ──────────────────────────────────────────────────────────────

/// Single selection from a list.
///
/// Returns the index of the selected item.
/// Errors in non-TTY mode.
///
/// # Example
/// ```ignore
/// let branches = vec!["feature-a", "feature-b", "feature-c"];
/// let idx = ui::select("Choose a branch", &branches)?;
/// let chosen = branches[idx];
/// ```
pub fn select<T: std::fmt::Display>(message: &str, items: &[T]) -> Result<usize> {
    if !std::io::stdin().is_terminal() {
        bail!(
            "Interactive selection required. Specify the value directly or use {} in a terminal.",
            program_name()
        );
    }

    if items.is_empty() {
        bail!("No items to select from");
    }

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(message)
        .items(items)
        .default(0)
        .interact()?;

    Ok(selection)
}

/// Multi-selection from a list.
///
/// Returns indices of selected items.
/// Errors in non-TTY mode.
pub fn multi_select<T: std::fmt::Display>(message: &str, items: &[T]) -> Result<Vec<usize>> {
    if !std::io::stdin().is_terminal() {
        bail!("Interactive selection required. Use {} in a terminal.", program_name());
    }

    if items.is_empty() {
        bail!("No items to select from");
    }

    let selections = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt(message)
        .items(items)
        .interact()?;

    Ok(selections)
}

// ──────────────────────────────────────────────────────────────
// Text input
// ──────────────────────────────────────────────────────────────

/// Text input with optional default value.
///
/// Errors in non-TTY mode.
pub fn input(message: &str, default: Option<&str>) -> Result<String> {
    if !std::io::stdin().is_terminal() {
        bail!("Interactive input required. Use {} in a terminal.", program_name());
    }

    let theme = ColorfulTheme::default();
    let mut builder = Input::<String>::with_theme(&theme).with_prompt(message);

    if let Some(def) = default {
        builder = builder.default(def.to_string());
    }

    let result = builder.interact_text()?;
    Ok(result)
}

/// Text input that allows empty values.
pub fn input_optional(message: &str) -> Result<Option<String>> {
    if !std::io::stdin().is_terminal() {
        return Ok(None);
    }

    let result: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt(message)
        .allow_empty(true)
        .interact_text()?;

    if result.is_empty() {
        Ok(None)
    } else {
        Ok(Some(result))
    }
}

// ──────────────────────────────────────────────────────────────
// Legacy-style prompts (for gradual migration)
// ──────────────────────────────────────────────────────────────

/// Simple yes/no prompt matching existing Diamond style.
///
/// Shows: "message [y/N]: " and reads a single line.
/// Returns true if user enters 'y' or 'Y'.
pub fn yes_no(message: &str, default_yes: bool) -> Result<bool> {
    use std::io::{self, Write};

    if !std::io::stdin().is_terminal() {
        bail!("This command requires confirmation. Use --force to skip.");
    }

    let prompt = if default_yes {
        format!("{} {} ", message, "[Y/n]:".bright_black())
    } else {
        format!("{} {} ", message, "[y/N]:".bright_black())
    };

    print!("{}", prompt);
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_lowercase();

    let result = if input.is_empty() {
        default_yes
    } else {
        input == "y" || input == "yes"
    };

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confirm_errors_in_non_tty() {
        // In test environment, stdin is not a TTY
        let result = confirm("Test?", false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("--force"));
    }

    #[test]
    fn test_confirm_optional_returns_false_in_non_tty() {
        let result = confirm_optional("Test?", true);
        assert!(result.is_ok());
        assert!(!result.unwrap()); // Returns false in non-TTY
    }

    #[test]
    fn test_select_errors_in_non_tty() {
        let items = vec!["a", "b", "c"];
        let result = select("Choose:", &items);
        assert!(result.is_err());
    }

    #[test]
    fn test_select_errors_on_empty() {
        let items: Vec<&str> = vec![];
        // This would error even in TTY mode
        let result = select("Choose:", &items);
        assert!(result.is_err());
    }

    #[test]
    fn test_input_errors_in_non_tty() {
        let result = input("Enter value:", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_input_optional_returns_none_in_non_tty() {
        let result = input_optional("Enter value:");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_yes_no_errors_in_non_tty() {
        let result = yes_no("Continue?", false);
        assert!(result.is_err());
    }
}
